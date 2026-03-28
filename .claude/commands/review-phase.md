# /review-phase <N>

Perform a completion review for Phase $ARGUMENTS.

## Instructions

1. Read the Phase $ARGUMENTS section from `docs/design-plan.md` to get the deliverables checklist.
2. For each deliverable, verify:
   - The file/feature exists
   - It matches the specification in the design doc
   - It compiles and passes clippy
3. Run the evaluator agent's 7 checkpoints against all code created in this Phase.
4. Check for any Phase-specific ADRs that should have been created.

## Output

A checklist with DONE/MISSING/PARTIAL for each deliverable, followed by the evaluator's summary.
