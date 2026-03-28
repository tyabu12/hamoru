# ADR-005: Policy Engine Design

## Status

Accepted (Phase 3)

## Context

Phase 3 introduces the Policy Engine (Layer 3), which automatically selects the optimal model based on declarative policies. This is hamoru's primary differentiator: explicit intent-based routing via tags + policies, rather than statistical optimization (TensorZero's approach).

Key design questions:
1. How should the PolicyEngine trait interact with provider data?
2. How should models be classified by quality?
3. How should policy YAML be structured for routing rules?
4. How should model scoring balance multiple priorities?

## Decisions

### D1: Trait Signature — `&[ModelInfo]` Instead of `&[&dyn LlmProvider]`

**Decision**: `select_model` and `select_fallback_model` accept `&[ModelInfo]` rather than `&[&dyn LlmProvider]`.

**Rationale**: The PolicyEngine is documented as synchronous (no async, no I/O), but `LlmProvider::list_models()` is async. Passing provider references would force the engine to call async methods, contradicting the sync design. The caller (CLI/orchestrator) pre-fetches model lists and passes flat `ModelInfo` slices.

**Trade-off**: The engine cannot dynamically query providers at selection time. This is acceptable because model catalogs change infrequently and can be cached.

### D2: RoutingRule YAML — Struct-with-Options

**Decision**: `RoutingRule` uses a flat struct with `Option` fields rather than an `#[serde(untagged)]` enum.

```rust
pub struct RoutingRule {
    pub match_rule: Option<MatchRule>,   // Form 1: tag-based
    pub default: Option<DefaultPolicy>,  // Form 2: fallback
    pub policy: Option<String>,          // Top-level for match rules
}
```

**Rationale**: Untagged enums produce poor error messages during deserialization. The struct approach allows explicit post-parse validation with actionable error messages ("Routing rule 3 references undefined policy 'foo'").

**Constraint**: Match rules carry `policy` at the top level; default rules carry it inside `DefaultPolicy`. Post-parse validation enforces this structural invariant.

### D3: Quality Tier Heuristic — Cost-Based Classification

**Decision**: Models are classified into `Low | Medium | High` quality tiers based on output token cost:
- **High**: `cost_per_output_token >= 10.0 / 1M` (Sonnet-class+)
- **Medium**: `cost_per_output_token >= 2.0 / 1M` (Haiku-class)
- **Low**: everything else (local/free)

**Rationale**: In current LLM markets, cost strongly correlates with capability. This avoids requiring users to manually classify every model. Thresholds are stored as named constants for easy adjustment.

**Limitation**: This heuristic may misclassify models with unusual pricing (e.g., a subsidized high-capability model). Future phases could allow per-model tier overrides in config.

### D4: Scoring Algorithm — Priority-Based Weighting

**Decision**: Four scoring modes with normalized 0-1 scores:
- **Cost**: cheapest = 1.0 (min-max normalization, inverted)
- **Quality**: highest tier = 1.0, tiebreak by cost within tier
- **Latency**: fastest = 1.0, missing data defaults to 0.5
- **Balanced**: 40% quality + 35% cost + 25% latency

**Rationale**: Balanced weights favor quality slightly because model capability differences have larger impact than marginal cost savings. Latency weight is lower because it's the most variable metric.

### D5: Cost Limits — Short-Circuit Evaluation

**Decision**: `check_cost_limits` evaluates limits in order (per_request → per_day → per_workflow → per_collaboration) and returns on first violation.

**Rationale**: Short-circuit is simpler and sufficient — the user needs to know which limit blocked them, not all limits that would block. Daily spend is calculated as `total_cost / period_days` with a guard for `period_days == 0`.

### D6: ModelSelection — No Default Derive

**Decision**: `ModelSelection` does not derive `Default`. Fields like `score: f64` and `provider: String` have no meaningful defaults.

**Rationale**: Phase 0 used a unit struct placeholder that derived Default. With real fields, Default would produce a misleading zero-score, empty-provider selection. Removing Default forces explicit construction.

## Consequences

- The Policy Engine is fully synchronous and testable without async runtime
- ~66 new tests bring the total from 117 to 183
- CLI supports `hamoru run -p policy` and `hamoru run --tags tag1,tag2`
- `hamoru init` generates a starter `hamoru.policy.yaml`
- Quality tier classification is automatic but may need refinement for unusual pricing models
- Scoring weights are hardcoded; future ADR may introduce user-configurable weights

## Deferred to Future Phases

- **Accumulated cost tracking for per_workflow / per_collaboration**: Phase 3 guards against single-request exceeding these limits. Accumulated tracking requires workflow/collaboration session state, which is the Orchestrator (Phase 4) and Agent Collaboration Engine (Phase 6) responsibility. They pass running totals to `check_cost_limits`.
- **Cost-actuals-based scoring**: `score_by_cost` uses static per-token pricing from ModelInfo. Using historical cost-per-request from MetricsCache is a future optimization when sufficient telemetry data volume justifies it.
- **simulate_cost_impact precision**: Current implementation uses a conservative heuristic. Full traffic simulation (routing each historical request through both configs) requires Phase 4's workflow execution context. The simplified version provides directional guidance for `hamoru plan`.
