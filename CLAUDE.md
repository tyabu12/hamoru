# hamoru — LLM Orchestration Infrastructure as Code

> Detailed design: `docs/design-plan.md`
> Launch plan: `docs/launch-plan.md`

## Current Phase

**Phase 2: Telemetry + plan**
- See: `docs/design-plan.md` Section 9 (Phase 2)

<!-- Update this section manually when moving to the next Phase. -->

## Project Overview

hamoru is an orchestration infrastructure tool aiming to be "Terraform for LLMs." It declaratively manages multiple LLM providers, automatically selects optimal models based on cost/quality/latency policies, and executes multi-step workflows. The final form serves as an OpenAI-compatible API.

**Primary purpose is learning.** However, production-grade design decisions are embedded throughout. Each Phase delivers standalone value even if the project stops there.

## Language Rules

- Conversation with user: **Japanese**
- **Extended thinking (internal reasoning): English**
- Code, commit messages, comments, documentation: **English**

## Architecture

5 Layers + API. Details in `.claude/rules/architecture.md`.

## Technology Stack

| Component | Crate | Purpose |
|-----------|-------|---------|
| Async runtime | `tokio` | Parallel LLM API calls, REST server |
| Async traits | `async-trait` | dyn-safe async trait methods (`LlmProvider`, etc.) |
| Streaming | `futures-core`, `futures` | `Stream` trait + stream combinators |
| Stream utilities | `tokio-stream` | Stream adapters for provider response parsing |
| Byte buffers | `bytes` | Streaming byte buffer handling in SSE/NDJSON parsing |
| HTTP client | `reqwest` | Provider adapter HTTP communication (features: `rustls-tls`, `stream`, `json`) |
| HTTP server | `axum` | OpenAI-compatible REST API |
| Serialization | `serde`, `serde_yaml`, `serde_json` | Config files, API communication |
| CLI | `clap` | Subcommands, argument parsing |
| Logging | `tracing` | Structured logs, OpenTelemetry-compatible |
| Logging subscriber | `tracing-subscriber` | Log formatting and env-filter (CLI layer only) |
| Error handling | `thiserror` | Unified `HamoruError` type |
| Timestamps | `chrono` | ISO 8601 timestamps in telemetry `HistoryEntry` |
| Randomness | `rand` | Jitter in `RetryProvider` exponential backoff |
| Local DB | `rusqlite` | Telemetry persistence (Phase 2+) |

Avoid adding dependencies beyond this list without explicit justification and user confirmation. If a new dependency seems needed, discuss it in an ADR first.

## Hard Rules

1. **No `unwrap()`** — Use `?` operator or explicit error handling. `expect()` is allowed only in test code.
2. **No API keys in code, logs, or commits** — Credentials come from environment variables only.
3. **No provider-specific types outside `provider/`** — All cross-layer communication uses shared types (`ChatRequest`, `ChatResponse`, etc.).
4. **No `{previous_output}` in System messages** — Always inject as a separate User Role message (injection mitigation).
5. **No new dependencies without user confirmation** — See Technology Stack above.
6. **Doc comments on public functions** — `#[warn(missing_docs)]` from Phase 0 (skeletons/provisional traits are exempt). Escalate to `#[deny(missing_docs)]` from Phase 1 onward when real implementations begin.
7. **No code without tests** — TDD is mandatory from Phase 1 onward. Phase 0 (skeletons, empty trait impls, error type definitions) is exempt — completion criteria is compile + clippy clean. See Quality & Engineering Principles.
8. **No prompt content in tracing** — `ChatRequest`, `ChatResponse`, and types containing prompt/message content must never appear as tracing span attributes or log fields. This prevents prompt leakage via OTLP export, log files, or `--debug` output. Complements Rule 2 (credentials); this rule protects prompt content. Concrete `#[instrument]` patterns are in `.claude/rules/provider.md`.
9. **No display logic in hamoru-core** — hamoru-core must not write directly to stdout/stderr or contain CLI-specific presentation logic. The core crate returns structured data; presentation is the CLI/UI layer's responsibility. `Display`/`Debug` trait implementations (including `thiserror` `#[error(...)]`) are permitted — these are data type contracts, not presentation logic. See `.claude/rules/architecture.md` Layer Boundary Rules for related constraints (dependencies, subscriber initialization, `Serialize` derive).

## Quality & Engineering Principles

- **TDD**: mandatory from Phase 1 onward. See Testing Policy for workflow details.
- **DRY**: Extract shared logic into common functions or traits. Refactor duplication across providers or layers immediately.
- Favor clarity over cleverness.
- Dependency inversion via `&dyn Trait`; focused trait interfaces.
- Minimize token usage and API calls in orchestration logic. Ask when trading performance vs correctness.

## Confirmation Policy

- Confirm with user before installing new packages or making major architectural decisions
- When uncertain about direction or trade-offs, always ask before proceeding
- Major refactors that change public trait signatures require user approval
- When the current task reveals a need for significant design changes or scope shifts beyond the current Phase, stop and report to the user before proceeding

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

**Planned ADRs (these numbers are reserved):**

| Number | Title | Source |
|--------|-------|--------|
| 000 | Why hamoru — Competitive analysis and differentiation | design-plan.md Section 1.1 |
| 001 | Architecture Overview | design-plan.md Section 3 |
| 002 | Tool Execution boundary — internal-only tools, external deferred to MCP | design-plan.md Phase 0 |
| 003 | Provider Abstraction Design — retry-as-decorator, custom SSE/NDJSON, factory DI | design-plan.md Phase 1 |
| 004 | Telemetry SQLite Migration — spawn_blocking, MetricsCache design, plan scope | design-plan.md Phase 2 |

Next available number: **005**. Increment sequentially from here.

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

**Before starting each Phase**: Read ONLY the corresponding Phase section in `docs/design-plan.md` (typically 30-50 lines). Avoid reading the entire document — it is ~1500 lines and will waste context.

**On Phase completion**: Record an ADR in `docs/decisions/`.

## Rules Reference

| File | Scope | Loaded when |
|------|-------|-------------|
| `.claude/rules/architecture.md` | Architecture, crate structure, layer boundaries, competitive positioning | Always |
| `.claude/rules/design-decisions.md` | Design decisions, error patterns | Always |
| `.claude/rules/provider.md` | Provider implementation rules | Editing `crates/hamoru-core/src/provider/**` |
