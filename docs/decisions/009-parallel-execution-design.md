# ADR-009: Parallel Execution Design

## Status

Accepted

## Context

Phase 4b adds parallel execution of independent workflow steps to the orchestration engine. The design must support fan-out/fan-in patterns while maintaining backward compatibility with Phase 4a sequential workflows.

## Decisions

### 1. Step Dependencies via `dependencies` Field

**Choice**: Add `dependencies: Option<Vec<String>>` to `WorkflowStep`.

**Alternatives considered**:
- `parallel_group` field: Cannot express general DAGs (e.g., diamond patterns)
- Array transition targets (`next: [a, b]`): Conflates scheduling with conditional routing; breaking change to `TransitionConfig.next` type

**Rationale**: Explicit dependency edges are the most general DAG primitive. Layer 5 (Phase 6) needs to compile arbitrary collaboration patterns into Layer 4 DAGs; `dependencies` gives maximum flexibility. Backward compatible: `None` infers sequential dependency on the previous step in list order.

### 2. `futures::future::join_all` over `tokio::JoinSet`

**Choice**: Use `futures::future::join_all` for concurrent step execution.

**Design-plan.md deviation**: The design document specifies `tokio::JoinSet`-based execution. We intentionally deviate.

**Rationale**: `JoinSet::spawn()` requires `'static` futures. The `OrchestrationEngine::execute()` trait method receives borrowed references (`&dyn PolicyEngine`, `&ProviderRegistry`, `&dyn TelemetryStore`). Wrapping these in `Arc` would require changing the trait signature, which is a constraint violation (trait stability). `join_all` has no `'static` requirement — futures borrow from the enclosing scope and are awaited within it. tokio interleaves them concurrently at I/O await points (LLM API calls are I/O-bound).

**Trade-off**: `join_all` cannot abort in-flight siblings on first failure (no `abort_all()`). We accept this: all parallel steps complete, then we inspect results. In-flight LLM calls would continue even with `JoinSet` abort since the HTTP request is already sent.

### 3. Wave-Based DAG Execution

**Choice**: Pre-compute execution waves via topological sort (Kahn's algorithm), execute waves sequentially with parallel steps within each wave.

**Algorithm**:
1. Build DAG from step dependencies
2. Topological sort into waves (groups at the same depth)
3. Linear DAGs (all waves size 1) use the sequential fast-path — supports transition-based loops from Phase 4a
4. Parallel DAGs iterate waves, running steps concurrently within each wave via `join_all`

### 4. Result Merge: Labeled Concatenation

**Choice**: Fan-in steps receive merged `{previous_output}` with labeled sections sorted alphabetically by step name.

```
=== [review] ===
Code looks good.

=== [security-check] ===
No vulnerabilities found.
```

Single predecessor: passthrough without labels. Alphabetical ordering ensures deterministic output regardless of parallel completion order.

### 5. Cost Cap: Per-Wave Checking (No Per-Step Budget Split)

**Choice**: Check cost after each wave completes. No per-step budget apportioning.

- Pre-wave guard: abort if remaining budget <= 0
- Post-wave check: sum wave costs, check against `max_cost`
- Overshoot tolerance: at most one wave's cost

**Rationale**: Cannot predict individual step costs before execution. Dividing budget by N penalizes cheap steps. Single-step overshoot is already tolerated in Phase 4a sequential execution.

### 6. Error Handling: Collect All Results

**Choice**: Use `join_all` (not `try_join_all`) — let all parallel steps complete, then check for errors.

On failure: return `MidWorkflowFailure` with all completed step results from this and previous waves. The first error's step name and source are preserved.

### 7. Transition Agreement for Parallel Steps

**Choice**: All parallel steps with transitions in the same wave must agree on the target.

Disagreement returns `ConditionEvaluationFailed`. Steps without transitions are ignored for agreement checking (they passthrough to successors).

This keeps Layer 4 simple. Layer 5 (Phase 6) can implement `QualityGate::AllMustApprove` / `Majority` by inserting a merge step that evaluates individual verdicts.

### 8. Latency Calculation

- Per-wave: wall-clock time (`Instant::now()` around `join_all`)
- Total: sum of wave latencies (reflects actual user-perceived time)
- Individual step `latency_ms` in `StepResult` remains per-step wall-clock

## Consequences

- Fan-out/fan-in workflows execute in parallel, reducing wall-clock latency
- Existing Phase 4a workflows (no `dependencies` field) use the sequential fast-path with zero overhead
- Loops with parallel branches are not supported in this phase (deferred)
- Layer 5 can compile collaboration patterns into Layer 4 DAGs via the `dependencies` field
