---
description: Perform a completion review for a specific Phase against design-plan.md deliverables.
argument-hint: <phase-number>
allowed-tools: Read, Grep, Glob, Bash, Agent
---

# /review-phase <N>

Perform a completion review for Phase $ARGUMENTS.

## Instructions

1. Read the Phase $ARGUMENTS section from `docs/design-plan.md` to get the deliverables checklist.
2. For each deliverable, verify:
   - The file/feature exists
   - It matches the specification in the design doc
   - It compiles and passes clippy
3. Run the evaluator agent's 11 checkpoints against all code created in this Phase.
4. Check for any Phase-specific ADRs that should have been created.

## Review Loop

After completing the deliverable checklist and evaluator run:

1. Launch 2 parallel subagents to cross-review (read-only — subagents must not modify any files):
   - Agent 1 (Spec fidelity): Re-read the Phase section of design-plan.md independently and verify each DONE item truly matches the spec — not just "exists" but "correct."
   - Agent 2 (Gaps): Look for implicit requirements not listed as explicit deliverables — error handling, edge cases, doc comments, ADRs.
2. If new MISSING or PARTIAL items are found, update the checklist and re-verify.
3. Repeat until no new issues. Hard limit: 3 iterations. Stop after 3 even if issues remain and report them as unresolved.
4. Report the final checklist with iteration count.

## Output

A checklist with DONE/MISSING/PARTIAL for each deliverable, followed by the evaluator's summary.
