# ADR-011: API Server Hardening (Phase 5b)

## Status

Accepted (2026-03-30)

## Context

Phase 5a delivered the core API server: endpoints, namespace resolution, tool calls, SSE streaming, and telemetry recording. Phase 5b adds production-grade security and operational controls required by design-plan Section 5.2.

## Decisions

### D1: API Key Source — Environment Variables Only

Keys are resolved from `HAMORU_API_KEYS` (comma-separated, trimmed, deduplicated). No plaintext keys in hamoru.yaml (CLAUDE.md Rule 2). Empty variable = no auth (localhost dev mode).

**Rationale:** Consistent with provider credential pattern (HAMORU_ANTHROPIC_API_KEY). Prevents accidental key commits to version control.

### D2: Rate Limiting — DashMap Token Bucket

Added `dashmap` crate for sharded-lock concurrent hash map. Each API key gets an independent token bucket (60 req/min default). Global `__global__` key when auth is disabled.

Background eviction removes entries unused for >1 hour. Uses `tokio::sync::watch` channel for graceful shutdown signal.

**Rationale:** DashMap provides per-shard locking (lower contention than global Mutex). Token bucket is a well-understood algorithm. The `hashbrown 0.14` duplicate (transitive from dashmap) is allowed in deny.toml skip list.

**Trade-off:** Added 1 new dependency. Justified by ergonomic `entry()` API and future scalability if the server is exposed externally.

### D3: Tags for Policy Routing

Tags come from `X-Hamoru-Tags` HTTP header (comma-separated). Body-based tags via `serde(flatten)` extra fields are deferred.

When both sources provide tags, they are merged (union, deduplicated, header first).

### D4: Cost Enforcement

Pre-request estimation uses `byte_length / 3.0` for input tokens (conservative for CJK/multilingual) and `max_tokens` (or 2000) for output. Global `RwLock<(NaiveDate, f64)>` daily cost tracker with midnight boundary reset.

**Scopes:** per_request, per_minute (new field), per_day.

### D5: No UUID Dependency

`uuid` was dropped in commit eebb9ee due to cargo-deny ban (`getrandom 0.4` conflicts with `0.3` from `rand`). Request IDs use `rand::random::<u128>()` formatted as hex (32 chars). Cryptographically secure via ChaCha12, 128-bit collision resistance. See issue #51 for future UUID resolution.

### D6: Timing-Safe API Key Comparison

XOR-fold constant-time comparison: `a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0`. Length short-circuit is safe (length is not secret).

### D7: Error Sanitization

5xx errors return generic "Internal server error. Please try again later." to clients. Full error details logged via `tracing::error!`. 4xx errors expose the message (user-actionable).

### D8: Hybrid Streaming Timeout

Per-chunk stall timeout (30s between consecutive chunks) + total stream duration limit (300s). Client disconnect detected via `tx.closed()` in `tokio::select!`.

### D9: Request ID

`rand::random::<u128>()` hex per request. Propagated to: `HistoryEntry.request_id`, tracing spans (planned), `x-request-id` response header (planned).

### D10: ServerConfig in hamoru-core

YAML parsing types (`ServerConfig`, `RateLimitConfig`) live in `hamoru-core::config` alongside `TelemetryConfig`. Runtime construction of axum layers stays in hamoru-cli. No hamoru-core dependency on axum.

### D11: CORS Deferred

Server defaults to 127.0.0.1 (localhost-only). Browser clients not a target use case. CORS middleware can be added as future hardening.

### D12: Daily Cost Reset

Global cost tracker stores `(NaiveDate, f64)` tuple. On each pre-request check, if date changed, accumulator resets to 0.0.

### D13: serde(flatten) Safety

When `#[serde(flatten)]` and named fields coexist, serde gives named fields priority. Reserved keys like "model" in the flattened map are silently ignored. This is documented serde behavior.

## Consequences

- API server now has defense-in-depth: auth, rate limiting, cost guardrails, error sanitization, streaming timeouts
- DashMap dependency added (7 transitive crates including hashbrown 0.14)
- Cost guardrail integration into the request handler path is prepared but not yet wired (daily_cost tracker, estimate_request_cost, check_and_accumulate_cost exist but are called only at the infrastructure level)
- Policy routing works via `hamoru:<policy-name>` model targets with async model collection
