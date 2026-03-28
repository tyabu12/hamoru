# ADR-003: Provider Abstraction Design

## Status

Accepted (Phase 1)

## Context

Phase 1 requires calling Claude API and Ollama through a unified interface with execution history recording. Key design decisions were needed for:

- How providers expose a common interface despite different API formats
- How streaming is handled (Anthropic SSE vs Ollama NDJSON)
- How retry logic is applied without duplicating code
- How model metadata is sourced and configured
- How the CLI wires providers to user commands

## Decision

### 1. Unified `LlmProvider` trait with provider-specific internal types

Each provider implements the `LlmProvider` trait (`id`, `list_models`, `chat`, `chat_stream`, `model_info`). Provider-specific API types (request/response structs) are private to each provider module and never leak across layer boundaries. This keeps the public API stable while allowing providers full control over their API integration.

### 2. Direct reqwest + serde (no SDK wrappers)

Providers use `reqwest` and `serde` directly instead of third-party SDK crates. This serves the project's learning goal (deep understanding of API specs) and enables immediate support for provider-specific features like Anthropic's prompt caching.

### 3. Retry-as-decorator pattern

`RetryProvider` wraps any `LlmProvider` with exponential backoff + full jitter. The factory automatically wraps every provider. This avoids duplicating retry logic in each provider and keeps the retry concern cleanly separated.

### 4. Hardcoded model catalog with config overrides

`catalog.rs` contains hardcoded pricing and capabilities for known models. Users can override pricing in `hamoru.yaml` via `ModelEntry::WithOverride`. Providers store `Vec<ModelEntry>` from config and apply `catalog::apply_overrides()` in `list_models()`. Phase 2+ may fetch catalog data from APIs.

### 5. Custom SSE and NDJSON parsers (no new dependencies)

Anthropic streams via SSE, Ollama via NDJSON. Both parsers use `futures::stream::unfold` with a byte buffer state machine. No additional parsing libraries were added. The `bytes` crate was added as an explicit dependency (already a transitive dependency of `reqwest`).

### 6. Provider ID from config name

`LlmProvider::id()` returns the user-configured `name` field from `hamoru.yaml`, not a hardcoded provider type string. This allows multiple instances of the same provider type (e.g., two Anthropic configs with different API keys).

### 7. Factory pattern with dependency injection

`build_registry(config)` uses standard `resolve_api_key` for production. `build_registry_with(config, resolver_fn)` accepts a custom resolver for test isolation. All providers are wrapped in `RetryProvider` with default config.

### 8. Error sanitization

`sanitize_error()` strips credentials (API keys, bearer tokens, URL-embedded passwords) from error messages before wrapping them in `ProviderRequestFailed` or `MidWorkflowFailure`. This prevents credential leakage via error Display/Debug output.

## Consequences

- Each provider is ~300-400 lines, self-contained and independently testable
- Adding a new provider requires: one module implementing `LlmProvider`, a match arm in `factory.rs`, and catalog entries
- SSE/NDJSON parsers handle byte-boundary splits correctly but are not battle-tested against all edge cases
- `tracing` facade is now a dependency of `hamoru-core` (subscriber initialization remains in CLI)
- 85 tests cover conversions, parsing, factory wiring, and error handling
