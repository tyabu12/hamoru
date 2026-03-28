---
description: Run hamoru-specific architecture and quality checks on recent changes. Use after implementation work.
allowed-tools: Read, Grep, Glob, Bash, Agent
---

# /hamoru-eval

Run hamoru-specific architecture checks on recent changes.

## Instructions

1. Identify files changed since the last commit (or all files if on initial commit).
2. Run the evaluator agent's checkpoints (defined in `.claude/agents/evaluator.md`) against the changed files.

## Review Loop

After the evaluator completes:

1. Launch 2 parallel subagents to cross-review the results (read-only — subagents must not modify any files):
   - Agent 1: Verify PASS results — are there false negatives? Re-check each PASS item independently.
   - Agent 2: Verify FAIL results — are there false positives? Confirm each failure is real.
2. If the cross-review finds new issues, incorporate them and re-run affected checks.
3. Repeat until no new issues are found. Hard limit: 3 iterations. Stop after 3 even if issues remain and report them as unresolved.
4. Report the final consolidated results with the iteration count.

## Output

Report each check as PASS/FAIL with details for any failures.
