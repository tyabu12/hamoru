---
description: Generate a detailed implementation plan for a specific Phase from design-plan.md.
argument-hint: <phase-number>
allowed-tools: Read, Grep, Glob, Agent
---

# /phase-plan <N>

Generate a detailed implementation plan for Phase $ARGUMENTS.

## Instructions

1. Read ONLY the Phase $ARGUMENTS section from `docs/design-plan.md` (do NOT read the entire document — it is ~1500 lines).
2. Read `CLAUDE.md` for project rules and conventions.
3. Identify all deliverables and their dependencies.
4. Generate a step-by-step implementation plan with:
   - Ordered tasks (each should be one commit)
   - Files to create or modify
   - Dependencies between tasks
   - Compile checkpoints after each task
5. Flag any ambiguities or design decisions that need user input.

## Review Loop

After generating the plan:

1. Launch 2 parallel subagents to review the plan (read-only — subagents must not modify any files):
   - Agent 1 (Feasibility): Verify each task is achievable — read the actual source files to confirm the plan's assumptions about existing code are correct. Are there missing intermediate steps that would break compilation?
   - Agent 2 (Completeness): Re-read the Phase section of design-plan.md and verify all deliverables are covered by at least one task. Check for missing ADRs, tests, or config changes.
2. If gaps or ordering issues are found, revise the plan and re-verify.
3. Repeat until no new issues. Hard limit: 3 iterations. Stop after 3 even if issues remain and report them as unresolved.
4. Present the final plan with iteration count.

## Output

A numbered task list with clear deliverables and verification steps.
