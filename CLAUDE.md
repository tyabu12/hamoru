# hamoru — LLM Orchestration Infrastructure as Code

> Detailed design: `docs/design-plan.md`

## Current Phase

**Phase 0: Scaffold & Interface Design**
- See: `docs/design-plan.md` Section 9 (Phase 0)

> Update this section manually when moving to the next Phase.

## Project Overview

hamoru is an orchestration infrastructure tool aiming to be "Terraform for LLMs." It declaratively manages multiple LLM providers, automatically selects optimal models based on cost/quality/latency policies, and executes multi-step workflows. The final form serves as an OpenAI-compatible API.

**Primary purpose is learning.** However, production-grade design decisions are embedded throughout. Each Phase delivers standalone value even if the project stops there.

## Language Rules

- Conversation with user: **Japanese**
- **Thinking (internal reasoning) MUST be in English**
- Code, commit messages, comments, documentation: **English**

## Architecture (5 Layers + API)

```
Layer 5: Agent Collaboration Engine  — Declarative agent coordination (core differentiator)
Layer 4: Orchestration Engine        — Workflow DAG execution
Layer 3: Policy Engine               — Task intent (tags) → automatic model selection
Layer 2: Provider Abstraction        — Unified trait: LlmProvider
Layer 1: Configuration & Telemetry   — YAML config + execution history
API:     OpenAI-Compatible Server    — POST /v1/chat/completions
```

Each layer is independently testable. Upper layers depend only on lower-layer abstractions.

## Competitive Differentiation

While TensorZero focuses on "statistical optimization of single inferences (POMDP)," hamoru differentiates on "declarative coordination control of multiple LLMs." The core moat is not any single feature but the integration of collaboration patterns × Policy Engine × cost impact prediction (plan).

## Crate Structure

```
hamoru/
├── Cargo.toml          # workspace root
├── crates/
│   ├── hamoru-core/    # All layer traits, types, errors, modules
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── provider/      # Layer 2
│   │       ├── telemetry/     # Layer 1
│   │       ├── policy/        # Layer 3
│   │       ├── orchestrator/  # Layer 4
│   │       ├── agents/        # Layer 5
│   │       ├── server/        # API Layer
│   │       └── error.rs
│   └── hamoru-cli/     # CLI entry point
│       └── src/
│           └── main.rs
├── docs/
│   ├── design-plan.md  # Detailed design document
│   └── decisions/      # ADRs
```

## Technology Stack

| Component | Crate | Purpose |
|-----------|-------|---------|
| Async runtime | `tokio` | Parallel LLM API calls, REST server |
| HTTP client | `reqwest` | Provider adapter HTTP communication |
| HTTP server | `axum` | OpenAI-compatible REST API |
| Serialization | `serde`, `serde_yaml`, `serde_json` | Config files, API communication |
| CLI | `clap` | Subcommands, argument parsing |
| Logging | `tracing` | Structured logs, OpenTelemetry-compatible |
| Error handling | `thiserror` | Unified `HamoruError` type |
| Local DB | `rusqlite` | Telemetry persistence (Phase 2+) |

Do NOT add dependencies beyond this list without explicit justification and user confirmation. If a new dependency seems needed, discuss it in an ADR first.

## Hard Rules

These rules are non-negotiable. Violations must be caught before commit.

1. **No `unwrap()`** — Use `?` operator or explicit error handling. `expect()` is allowed only in test code.
2. **No API keys in code, logs, or commits** — Credentials come from environment variables only.
3. **No provider-specific types outside `provider/`** — All cross-layer communication uses shared types (`ChatRequest`, `ChatResponse`, etc.).
4. **No `{previous_output}` in System messages** — Always inject as a separate User Role message (injection mitigation).
5. **No new dependencies without user confirmation** — See Technology Stack above.
6. **Doc comments on public functions** — `#[warn(missing_docs)]` from Phase 0 (skeletons/provisional traits are exempt). Escalate to `#[deny(missing_docs)]` from Phase 1 onward when real implementations begin.
7. **No code without tests** — TDD is mandatory from Phase 1 onward. Phase 0 (skeletons, empty trait impls, error type definitions) is exempt — completion criteria is compile + clippy clean. See Quality & Engineering Principles.

## Quality & Engineering Principles

- **TDD (Test-Driven Development)**: Write tests first, then implement to make them pass. The workflow for each feature is: trait definition → tests against trait (using mock/stub) → implementation to pass tests. No implementation code is committed without corresponding tests. **Exception**: Phase 0 skeletons (empty trait impls, type definitions) are exempt — compile + clippy clean is the completion criteria.
- **DRY (Don't Repeat Yourself)**: Extract shared logic into common functions or traits. Duplication across providers or layers should be refactored immediately.
- **SOLID Principles**: Applied through Rust idioms — single-responsibility modules, trait-based extension, focused trait interfaces (`LlmProvider` vs `TelemetryStore`), dependency inversion via `&dyn Trait`.
- **Maintainability first**: Favor clarity over cleverness. Write code that is readable and easy to change.
- **Performance and cost awareness**: Be mindful of token usage, API call count, and latency overhead. Avoid unnecessary LLM calls in orchestration logic. When there is a trade-off between performance and correctness, ask the user.

## Confirmation Policy

- Confirm with user before installing new packages or making major architectural decisions
- When uncertain about direction or trade-offs, always ask before proceeding
- Major refactors that change public trait signatures require user approval
- When the current task reveals a need for significant design changes or scope shifts beyond the current Phase, stop and report to the user before proceeding

## Layer Boundary Rules

- Provider-specific API types (e.g., Anthropic request/response structs) must NOT leak outside the `provider/` module
- Each Provider implements the `LlmProvider` trait and exposes only shared types externally
- Layer 5 compiles collaboration patterns into Layer 4 `Workflow` types and delegates execution. It must NOT have its own execution loop

## Provider Implementation Policy

Providers are implemented directly with reqwest + serde. No third-party abstraction libraries. Reasons:
- Immediate support for provider-specific features (Claude's Prompt Caching, OpenAI's Structured Outputs, etc.)
- Each adapter is ~200-400 lines
- Deep understanding of API specs directly serves the learning goal

## Key Design Decisions

- **Condition evaluation default: Tool Calling (v2)** — Workflow step transitions use `report_status` tool call by default. STATUS line parsing (v1) is kept as fallback for models without tool support. See design-plan.md Section 9.1.2.
- **ContextPolicy on workflow steps** — Steps can declare `context_policy: keep_last_n` to control message history. `SummarizeOnOverflow` is handled by Layer 5 inserting summary steps into the DAG. See design-plan.md Section 6.4.1.
- **Logging levels**: Default (step summary), `--verbose` (policy reasons, tokens), `--debug` (HTTP headers, raw SSE). See design-plan.md Section 11.2.
- **Failure UX**: Every error message must tell the user what happened AND what to do next. See design-plan.md Section 11.3 for the full scenario table.
- **YAML schema changes**: No breaking changes to YAML schema fields without bumping `version`. Additive fields are `Option` with defaults. See design-plan.md Section 7.1. Do NOT rename or remove existing YAML fields without user confirmation.

## Rust Coding Conventions

- Error types are unified under `HamoruError` using `thiserror` (details: design-plan.md Section 9.1.1)
- `async` functions return `Result<T, HamoruError>` by default
- `clippy -- -D warnings` is enforced in CI
- Formatting follows `cargo fmt`
- Non-obvious implementation choices must have a comment explaining **why**, not just what. Future readers (including LLMs) rely on these to understand intent

## Testing Policy

- **TDD workflow**: trait → test → impl. Tests are written against trait interfaces using mock implementations before the real implementation exists.
- Providers: unit tests with mock trait implementations. Integration tests hitting real APIs are marked `#[ignore]`
- Layers 3-5: unit tests with mock Provider + mock Telemetry
- E2E: spin up `hamoru serve` inside `tokio::test` and verify with reqwest
- Coverage target: 80%+

## Security Rules

- Credentials are injected via environment variables (`HAMORU_ANTHROPIC_API_KEY`, etc.)
- `hamoru serve` binds to `127.0.0.1` (localhost only) by default

## Commit Messages

Follow Conventional Commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`

Write subject lines that are vivid and concise — prefer active, expressive verbs over bland ones (e.g., "redesign" over "update", "wire up" over "add connection"). A dash of personality is welcome: puns, metaphors, or a wink of humor when it fits naturally (e.g., `feat: teach Policy Engine to play favorites`, `fix: stop workflows from ghosting mid-step`). Clarity always wins. Keep the subject line under 72 characters; add a body for context when the "why" isn't obvious.

Prefix the subject line with a single emoji that captures the spirit of the change (e.g., `✨ feat: teach Policy Engine to play favorites`, `🐛 fix: stop workflows from ghosting mid-step`, `♻️ refactor: untangle provider spaghetti`). One emoji only — this is seasoning, not the main course.

## Decision Records (ADR)

- Record architectural decisions in `docs/decisions/` as ADR
- Filename format: `NNN-<short-slug>.md` (e.g., `000-why-hamoru.md`, `001-architecture-overview.md`)
- ADRs are for Claude Code consumption — write in a structured, LLM-friendly format (clear sections, explicit rationale, concise)
- Each Phase completion produces at least one ADR

**Planned ADRs (do NOT reuse these numbers):**

| Number | Title | Source |
|--------|-------|--------|
| 000 | Why hamoru — Competitive analysis and differentiation | design-plan.md Section 1.1 |
| 001 | Architecture Overview | design-plan.md Section 3 |
| 002 | Tool Execution boundary — internal-only tools, external deferred to MCP | design-plan.md Phase 0 |

Next available number: **003**. Increment sequentially from here.

## Agent Configuration

- Evaluator subagent: `.claude/agents/evaluator.md`

## Implementation Phases

| Phase | Goal | Details |
|-------|------|---------|
| 0 | Scaffold & Interface Design | design-plan.md Section 9 (Phase 0) |
| 1 | Provider Abstraction + Basic Telemetry | design-plan.md Section 9 (Phase 1) |
| 2 | Telemetry + plan | design-plan.md Section 9 (Phase 2) |
| 3 | Policy Engine | design-plan.md Section 9 (Phase 3) |
| 4a | Orchestration Engine — Sequential | design-plan.md Section 9 (Phase 4a) |
| 4b | Orchestration Engine — Parallel | design-plan.md Section 9 (Phase 4b) |
| 5 | API Server (serve) | design-plan.md Section 9 (Phase 5) |
| 6 | Agent Collaboration Engine | design-plan.md Section 9 (Phase 6) |

**Before starting each Phase**: Read ONLY the corresponding Phase section in `docs/design-plan.md` (typically 30-50 lines). Do NOT read the entire document — it is ~1500 lines and will waste context.

**On Phase completion**: Record an ADR in `docs/decisions/`.
