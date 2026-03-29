# ADR-008: API Server Crate Placement

## Status

Accepted (Phase 5)

## Context

Phase 5 introduces an OpenAI-compatible API server (`hamoru serve`). The HTTP framework (axum) needs a home. Two options:

1. **hamoru-cli** — Place axum handlers alongside the existing CLI binary
2. **hamoru-server** — Create a dedicated third crate in the workspace

## Decision

**Place axum in hamoru-cli.** hamoru-core's `server/` module holds only framework-independent code: OpenAI wire format types, translation functions, and model namespace resolution.

### Module Ownership

| Module | Crate | Contents |
|--------|-------|----------|
| `hamoru-core::server::types` | hamoru-core | OpenAI request/response serde types |
| `hamoru-core::server::translate` | hamoru-core | Wire format <-> internal type translation (pure functions) |
| `hamoru-core::server::namespace` | hamoru-core | Model ID parsing (`hamoru:policy`, `provider:model`, etc.) |
| `hamoru-cli::server` | hamoru-cli | axum Router, handlers, middleware, AppState |

## Rationale

1. **Single consumer today.** hamoru-cli is the only binary. A third crate adds a workspace member, Cargo.toml, and inter-crate wiring for zero current benefit.
2. **Precedent.** hamoru-cli already hosts framework-specific code: clap argument parsing, tracing-subscriber initialization, and CLI presentation logic. axum fits the same pattern.
3. **Clean extraction path.** The boundary is a Rust module boundary (`hamoru-cli::server`). If a future consumer (Tauri desktop, standalone API binary, Wasm) needs the server, extracting `hamoru-cli::server` into `hamoru-server` is a mechanical refactor: move files, update `use` paths, add workspace member.
4. **hamoru-core stays framework-free.** hamoru-core has no dependency on axum, tower, or hyper. The `server/` module in hamoru-core contains only serde types and pure functions — testable without any HTTP runtime.

## Consequences

- axum, tower-http, and uuid are added as dependencies of hamoru-cli only
- hamoru-core gains no new dependencies for Phase 5
- If a second HTTP consumer appears, extraction to a dedicated crate is straightforward
- Server integration tests live in `hamoru-cli/tests/` (they need the full binary context)
