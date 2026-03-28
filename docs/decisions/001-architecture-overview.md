# ADR-001: Architecture Overview

## Status

Accepted

## Context

hamoru needs a layered architecture that supports incremental development (each Phase delivers standalone value), clean separation of concerns, and independent testability of each layer.

## Decision

A 5-layer architecture plus an API layer, with strictly downward dependencies:

```
Layer 5: Agent Collaboration Engine  — Declarative agent coordination
Layer 4: Orchestration Engine        — Workflow DAG execution
Layer 3: Policy Engine               — Task intent → automatic model selection
Layer 2: Provider Abstraction        — Unified trait: LlmProvider
Layer 1: Configuration & Telemetry   — YAML config + execution history
API:     OpenAI-Compatible Server    — POST /v1/chat/completions
```

### Key Traits

| Layer | Trait | Async | Key Responsibility |
|-------|-------|-------|--------------------|
| 1 | `TelemetryStore` | Yes | Execution history recording and metrics |
| 2 | `LlmProvider` | Yes | Unified interface for all LLM providers |
| 3 | `PolicyEngine` | No | Intent-based model selection (synchronous: operates on cached config, no I/O) |
| 4 | `OrchestrationEngine` | Yes | Multi-step workflow execution |
| 5 | `AgentCollaborationEngine` | Yes | Compile patterns into workflows (provisional) |

### Crate Structure

Two crates in a Cargo workspace:
- `hamoru-core`: All layer traits, types, errors, and module implementations
- `hamoru-cli`: CLI entry point using `clap`, delegates to `hamoru-core`

### Layer Boundary Rules

- Provider-specific API types must NOT leak outside `provider/`
- Each provider implements `LlmProvider` and exposes only shared types
- Layer 5 compiles collaboration patterns into Layer 4 `Workflow` types and delegates execution — it has no execution loop of its own
- All traits are `Send + Sync` for safe use across async boundaries

## Consequences

- Each layer is independently testable with mock implementations of lower layers
- Adding a new provider requires only implementing `LlmProvider` — no changes to upper layers
- The strict layering prevents shortcuts but ensures maintainability
- Layer 5 trait is provisional (redesigned at Phase 6 based on Layer 4 experience)

## Alternatives Considered

- **Monolithic design**: Rejected — doesn't support incremental Phase delivery
- **Microservice architecture**: Rejected — over-engineering for a learning project; single-process is sufficient
- **Generic type parameters instead of trait objects**: Rejected — `dyn Trait` enables runtime provider selection and simpler test mocking, with negligible overhead for per-request dispatch
