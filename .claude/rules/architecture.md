# Architecture & Crate Structure

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
│   │       ├── config/        # Layer 1 (config loading)
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

## Layer Boundary Rules

- Provider-specific API types (e.g., Anthropic request/response structs) must NOT leak outside the `provider/` module
- Each Provider implements the `LlmProvider` trait and exposes only shared types externally
- Layer 5 compiles collaboration patterns into Layer 4 `Workflow` types and delegates execution. It must NOT have its own execution loop

## hamoru-core Library Constraints

hamoru-core is consumed by multiple frontends (CLI, API server, and potentially Tauri, Wasm). To maintain this flexibility:

- **No CLI-specific dependencies**: hamoru-core must not depend on `clap`, `tracing-subscriber` (the crate), or other CLI/runtime-specific crates. The `tracing` facade crate (for emitting events/spans) is allowed; subscriber initialization is the consumer's responsibility
- **No stdout/stderr output**: hamoru-core must not write to stdout/stderr. All output formatting is the consumer's responsibility
- **Public types derive `Serialize`**: All public return types should derive `serde::Serialize` to support future JSON output, API responses, and structured logging. Existing Phase 0 skeleton types will be brought into compliance during their respective implementation phases
- **Note** (Phase 5 decision pending): The `server/` module currently contains only trait and type definitions. The placement of axum HTTP framework implementation (in hamoru-cli or a dedicated crate) will be decided via ADR at Phase 5 start
