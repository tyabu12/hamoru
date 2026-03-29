# ADR-006: Orchestration Engine Design

**Status**: Accepted
**Date**: 2026-03-29
**Phase**: 4a — Sequential Execution

## Context

Phase 4a implements Layer 4: the sequential workflow execution engine. This layer ties together providers, the Policy Engine, and telemetry into multi-step LLM workflows defined in YAML. Key design decisions were needed for module structure, condition evaluation, error handling, and context management.

## Decision: Module Structure

Split `orchestrator/` into focused files mirroring the `policy/` module pattern:

```
orchestrator/
  mod.rs          -- Trait + runtime types + re-exports
  config.rs       -- YAML config types + parser + validation
  condition.rs    -- Condition evaluation (v1 STATUS line + v2 Tool Calling)
  context.rs      -- Template substitution + ContextPolicy application
  engine.rs       -- DefaultOrchestrationEngine implementation
```

**Rationale**: Separation of concerns. Each file has a single responsibility and is independently testable. `condition.rs` and `context.rs` contain pure functions ideal for unit testing without mocks.

## Decision: Condition Evaluation — v1 vs v2

### v2: Tool Calling (default)

A `report_status(status, reason)` tool is injected into the LLM request with `ToolChoice::Tool` to force the model to call it. The `status` enum is **dynamically generated** from the step's transition conditions, constraining the LLM to valid values only.

### v1: STATUS Line Parsing (fallback)

Scans the last 10 lines of output in reverse for a `STATUS: <value>` line. Normalizes to lowercase and strips trailing punctuation.

### Fallback behavior

When `ConditionMode::ToolCalling` is set but the LLM does not call the tool, the evaluator **silently falls back to STATUS line parsing** with a `tracing::warn!()`. This handles models that ignore `tool_choice` constraints.

**Rationale**: Graceful degradation is better than hard failure. The warning lets operators diagnose the issue while the workflow continues.

### Retrospective TODO

After manual testing with real providers, measure empirical reliability of v1 vs v2. Consider whether v1 is worth keeping as a first-class option or should be demoted to internal fallback only.

## Decision: MaxIterationsReached — Warning, Not Error

Per design-plan.md §11.3, reaching `max_iterations` produces a **warning** and returns `Ok(ExecutionResult)` with `TerminationReason::MaxIterationsReached`. The last iteration's output is returned as `final_output`.

The `HamoruError::MaxIterationsReached` variant is retained in `error.rs` for potential future strict-mode use.

**Rationale**: Users expect partial progress to be preserved. A hard error would discard all accumulated work and force the user to restart from scratch.

## Decision: New Error Variants

Two error variants were added beyond the original design-plan.md error taxonomy:

- `WorkflowValidationError { workflow, reason }` — For YAML parse errors and semantic validation failures. Provides more context than generic `ConfigError`.
- `ConditionEvaluationFailed { step, reason }` — For condition evaluation failures (no status found, unmatched condition). Includes the step name and actionable guidance.

**Rationale**: Following the §11.3 principle that every error tells the user what happened AND what to do next. Generic error types lack the step-level context needed for actionable guidance.

## Decision: Workflow-Level `condition_mode`

The YAML schema extends the design doc by allowing `condition_mode` at the workflow level as a default for all steps. Per-step overrides take precedence. This is an ergonomic improvement to avoid repeating `condition_mode: status_line` on every step when using models without tool support.

## Decision: ContextPolicy Scope for Layer 5

`ContextPolicy` supports `KeepAll` and `KeepLastN { n }` at the step level. `SummarizeOnOverflow` is explicitly NOT included — it requires LLM calls for summarization and will be handled by Layer 5 inserting summary steps into the DAG at compile time.

This scope is sufficient for Phase 4a use cases (bounded loops) and Layer 5's generator-evaluator pattern. Layer 5 can set `KeepLastN` on generated steps without any Layer 4 changes.

## Decision: Streaming

All steps use buffered (non-streaming) execution in Phase 4a. The design doc specifies "only the final step streams," but detecting the final step in advance is not always possible in loop workflows. Final-step streaming is deferred to a future enhancement.

## Consequences

- 76+ tests covering config parsing, condition evaluation, context management, and the full execution loop
- Clean module boundaries enable independent testing and future extension
- `ToolChoice` added to shared `ChatRequest` type, wired into Anthropic provider, ignored by Ollama
- `StepResult` gained `policy_applied` field and custom `Debug` that redacts `output` content
