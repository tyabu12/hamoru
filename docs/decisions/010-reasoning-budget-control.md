# ADR-010: Reasoning Budget Control

## Status

Proposed

**Staleness policy**: Update to Accepted when Phase A implementation begins. Re-evaluate if 6 months pass without starting.

## Context

LLMs increasingly offer "reasoning" or "thinking" modes that consume additional tokens for chain-of-thought before producing the final answer. These modes are controlled by provider-specific parameters:

| Provider | Parameter | Semantics |
|----------|-----------|-----------|
| Anthropic | `budget_tokens` in `thinking` block | Dedicated token budget for extended thinking |
| OpenAI | `reasoning_effort` (low/medium/high) | Abstract effort level |
| DeepSeek | (varies) | Provider-specific |

hamoru's Policy Engine currently routes based on cost, quality, latency, and capabilities — but has no mechanism to declaratively control *how much* reasoning a model should perform.

### Current State

- `Capability::Reasoning` exists as a simple enum variant in `provider/types.rs` (Phase 0) but is **unused**: no catalog model declares it, no policy code references it
- `TokenUsage` has no `reasoning_tokens` field — reasoning token consumption is invisible
- `HarnessConstraints` (Phase 6) has no reasoning budget parameter
- Provider adapters do not parse reasoning token counts from responses

### Motivation

- "Scale reasoning time, not just model size" is an emerging performance strategy
- Task difficulty varies — some steps need deep reasoning, others do not
- Declarative reasoning budget control enables cost/quality optimization at the workflow level
- Differentiator vs. TensorZero (statistical optimization of single inferences)

## Decisions

### D1: Phase A (0.x, post-Phase 5) — Reasoning Awareness

Phase A is the primary scope of this ADR. It establishes reasoning token tracking without introducing new control mechanisms.

**TokenUsage extension**:
- Add `reasoning_tokens: Option<u64>` to `TokenUsage`
- Semantics: separate from `output_tokens` (additive, no overlap). `output_tokens` holds final output tokens only; `reasoning_tokens` holds thinking/reasoning tokens only
- Type is `Option<u64>` (reasoning-incapable models return `None`; consistent with `cache_creation_input_tokens` pattern)
- Update `AddAssign` impl using existing `merge_option_u64` helper
- Update `calculate_cost()` to include reasoning tokens at the output token rate (all major providers — Anthropic, OpenAI, Google, DeepSeek — charge reasoning tokens at the same rate as output tokens, verified 2026-03)

**Provider-specific parsing** (asymmetric by design):
- Anthropic: API reports thinking tokens separately from `output_tokens` → map directly (documentation of existing API behavior)
- OpenAI: `completion_tokens` includes reasoning tokens → hamoru design decision to decompose: `output_tokens = completion_tokens.saturating_sub(reasoning_tokens)`. Use `saturating_sub` with `tracing::warn!()` on underflow. When `completion_tokens_details.reasoning_tokens` is absent, set `reasoning_tokens = None` and `output_tokens = completion_tokens`
- Providers that cannot separate reasoning tokens: `reasoning_tokens = None`

**Catalog activation**:
- Begin assigning `Capability::Reasoning` to supported models in the hardcoded catalog

**Telemetry extension**:
- SQLite schema migration: add `reasoning_tokens` column to `history` table
- Add `total_reasoning_tokens` to `Metrics` and `ModelMetrics` structs
- Update SQL aggregation queries

**Touch points** (files modified in Phase A):
- `crates/hamoru-core/src/provider/types.rs` — `TokenUsage`, `AddAssign`, `calculate_cost()`
- `crates/hamoru-core/src/provider/anthropic.rs` — response parser
- `crates/hamoru-core/src/provider/catalog.rs` — `Capability::Reasoning` activation
- `crates/hamoru-core/src/telemetry/sqlite.rs` — schema migration
- `crates/hamoru-core/src/telemetry/mod.rs` — `Metrics`/`ModelMetrics` aggregation

### D2: Security Constraints

**Thinking content is prompt-equivalent data** (Rule 8 extension):
- Thinking/reasoning text must never appear in telemetry, tracing spans, or log fields. Only token counts cross the provider boundary
- Provider adapter error handling must not capture thinking content in raw response bodies within error variants. Extend existing `sanitize_error()` pattern

**Prerequisite — custom `Debug` impls**:
- `ChatResponse` and `ChatRequest` currently use `#[derive(Debug)]`, exposing message content in debug output. Before Phase A begins, add custom `Debug` impls that redact `content` (following the `StepResult` pattern in `error.rs`). This is a pre-existing defense-in-depth gap independent of reasoning budget

**Phase B security requirements** (documented here for forward planning):
- `ReasoningBudget::Auto` must have a system default cap (unbounded Auto risks cost explosion)
- Budget values must be validated: provider-specific clamping, checked arithmetic to prevent overflow
- API boundary validation: reject abusive values (e.g., `TokenLimit(u64::MAX)`) with HTTP 400
- Concurrent reasoning request limits: `max_concurrent_reasoning_requests` or global reasoning token budget per time window via `tokio::sync::Semaphore`

### D3: Cost Calculation

Reasoning tokens are calculated at the output token rate. No `cost_per_reasoning_token` field is added to `ModelInfo`.

**Rationale**: As of 2026-03, all four major providers (Anthropic, OpenAI, Google, DeepSeek) charge reasoning tokens at the same rate as output tokens. Adding a separate field now would be premature complexity. When a provider introduces differential reasoning token pricing, add `cost_per_reasoning_token: Option<f64>` to `ModelInfo` with fallback to `cost_per_output_token` when `None`.

## Future Directions

The following phases are **illustrative, not prescriptive**. Each will be designed via a dedicated ADR when implementation begins, informed by Phase A experience.

### Phase B (Phase 6 / 1.0 scope) — Declarative Reasoning Budget

- Add `reasoning_budget: Option<ReasoningBudget>` to `ChatRequest` (Layer 2, per-request parameter)
- Add per-step reasoning budget to `StepConfig` (Layer 4, workflow-level control)
- Add session-level default to `HarnessConstraints` (Layer 5), compiled down to per-step budgets
- `ReasoningBudget` enum sketch:
  ```rust
  enum ReasoningBudget {
      /// Absolute token limit (maps to Anthropic budget_tokens)
      TokenLimit(u64),
      /// Abstract effort level (maps to OpenAI reasoning_effort)
      Effort(ReasoningEffort),  // Low | Medium | High
      /// Delegate to provider/model (with system default cap)
      Auto,
  }
  ```
- Provider adapters translate `ReasoningBudget` to provider-specific parameters internally (existing pattern)
- Lossy conversions (e.g., `TokenLimit` → OpenAI `reasoning_effort`) use heuristic mapping + log output
- When `Capability::Reasoning` model receives a reasoning budget, strip unsupported parameters (temperature, system prompt) with `tracing::info!()`

### Phase C (post-1.0) — Thinking Content & Provider Absorption

- Add `thinking_content: Option<String>` to `ChatResponse` as opt-in field
  - Provider adapter strips by default; returns content only on explicit opt-in
  - `#[serde(skip_serializing)]` to prevent accidental serialization to API responses
  - Custom `Debug` impl redacts the field
  - Enables Layer 5 Agent Collaboration patterns (evaluator reviews generator's reasoning trace)
- Provider Abstraction Layer absorbs provider-specific reasoning parameter differences
- Add `cost_per_reasoning_token` to `ModelInfo` if providers introduce differential rates

### Phase D (future) — Reasoning Performance Profiling

- Model-specific thinking budget vs. quality performance curves
- Telemetry-driven optimal reasoning budget recommendations
- Integration with `hamoru plan` for reasoning cost impact prediction

## Consequences

- Phase A is low-risk and immediately improves cost accuracy for reasoning models
- The `output_tokens` "final output only" definition creates an asymmetry: Anthropic API already separates them (documentation), while OpenAI requires hamoru-side decomposition (design decision). This may cause user confusion when comparing with OpenAI dashboard numbers — mitigate with clear doc comments and CLI output labels
- Phase B's `Effort` variant maps naturally to OpenAI but requires model-specific heuristic tables for Anthropic conversion
- Phase C's `thinking_content` opt-in enables collaboration patterns but increases leakage surface — mitigated by `#[serde(skip_serializing)]` + custom `Debug`

## Alternatives Considered

1. **Add `reasoning_capable: bool` to `ModelInfo`** — Rejected. `Capability::Reasoning` already provides equivalent functionality via `capabilities.contains(&Capability::Reasoning)`
2. **Immediate provider-specific parameter passthrough** — Rejected. Provider reasoning APIs are still evolving; premature abstraction risk
3. **`Proportional(f64)` variant in `ReasoningBudget`** — Rejected. The denominator is ambiguous (output tokens are unknown at request time). `Effort(Low/Medium/High)` maps naturally to provider APIs
4. **Add `cost_per_reasoning_token` from Phase A** — Rejected. All four major providers use the same rate as output tokens (verified 2026-03). YAGNI
5. **Always strip thinking content (no opt-in)** — Rejected. Closes off LLM collaboration patterns, hamoru's core differentiator
6. **Thinking content summary mode** — Rejected. Requires additional LLM call (cost), quality guarantee is difficult
7. **`reasoning_tokens: u64` (non-optional)** — Rejected. Reasoning-incapable models have no meaningful value. `Option<u64>` is consistent with cache token fields

## References

- [Anthropic Extended Thinking](https://docs.anthropic.com/en/docs/build-with-claude/extended-thinking)
- [OpenAI Reasoning Models](https://platform.openai.com/docs/models/o3)
- Pricing survey (2026-03): Anthropic, OpenAI, Google, DeepSeek all charge reasoning tokens at the output token rate
