# ADR 004: Telemetry SQLite Migration

## Status

Accepted

## Context

Phase 1 used `JsonFileTelemetryStore` (backed by `.hamoru/state.json`) for execution history persistence. This approach has limitations:

- **Query performance**: Every metrics query reads the entire JSON file and filters in-memory.
- **No aggregation**: Cannot do GROUP BY, AVG, or SUM at the storage layer.
- **Concurrency**: Full-file writes on each record are not safe under concurrent access.

Phase 2 requires per-model/per-provider metrics breakdown, cost projections (`hamoru plan`), and a foundation for future Policy Engine cost analysis.

## Decision

### 1. SQLite via rusqlite with spawn_blocking

We use `rusqlite` (with `bundled` feature) for SQLite access. Since rusqlite is synchronous and our `TelemetryStore` trait is async, all database operations are wrapped in `tokio::task::spawn_blocking`.

**Why spawn_blocking over tokio-rusqlite**: No additional dependency needed. The CLI workload is low-volume (one command at a time). If `hamoru serve` (Phase 5) reveals thread pool pressure, we can upgrade to a dedicated connection thread then.

**Connection management**: `Arc<Mutex<Connection>>` with `tokio::sync::Mutex`. Single connection is sufficient for CLI workload. WAL journal mode enables concurrent reads.

### 2. MetricsCache expansion

`MetricsCache` was expanded from an empty struct to contain:
- `total: Metrics` (aggregate totals)
- `by_model: HashMap<String, ModelMetrics>` (per-model breakdown)
- `by_provider: HashMap<String, Metrics>` (per-provider breakdown)
- `period_days`, `entry_count` (metadata)

A new trait method `query_detailed_metrics(period)` was added to `TelemetryStore` to support period-parameterized detailed queries. The existing `load_cache()` delegates to this with a 7-day default.

### 3. JSON-to-SQLite migration

Auto-migration runs on first use: if `.hamoru/state.json` exists and `.hamoru/state.db` does not have those entries, they are migrated. The JSON file is renamed to `state.json.migrated` to prevent re-migration. Migration is idempotent (duplicate entries are skipped by timestamp+provider+model check).

### 4. `hamoru plan` scope

Phase 2's `plan` command delivers telemetry-based cost projections only (daily cost averages, per-model breakdown, confidence score based on data volume). Policy-aware cost impact prediction (`simulate_cost_impact()`) is deferred to Phase 3 when `PolicyEngine` exists.

### 5. HistoryEntry tags field

Added `tags: Vec<String>` with `#[serde(default)]` to future-proof the SQLite schema for Phase 3 tag-based routing. This avoids a schema migration later.

### 6. File permissions

`.hamoru/state.db` is set to `0o600` (owner read/write only) on unix systems, matching the security posture of config files.

## Consequences

- Phase 1 JSON store code is preserved but no longer used by CLI (available as fallback or for testing).
- SQLite adds ~1.5MB to binary size (bundled feature compiles SQLite from source).
- All future telemetry features (Policy Engine cost analysis, `hamoru serve` metrics endpoint) build on the SQLite foundation.
- S3/R2 remote sync and `CompositeStore` are deferred to a later sub-phase.

## Retrospective (to be completed after usage)

- Was the telemetry-based cost prediction in `hamoru plan` useful for real workflows?
- Did the MetricsCache structure serve Policy Engine needs in Phase 3?
- Was the migration from JSON seamless for existing users?
