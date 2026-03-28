---
description: Generate an Architecture Decision Record and save it to docs/decisions/.
argument-hint: <title>
allowed-tools: Read, Grep, Glob, Write, Edit, Agent
---

# /write-adr <title>

Generate an Architecture Decision Record for: $ARGUMENTS

## Instructions

1. Check `CLAUDE.md` for the next available ADR number and filename format.
2. Read any referenced sections from `docs/design-plan.md`.
3. Write the ADR in `docs/decisions/NNN-<short-slug>.md` using this structure:

```markdown
# ADR-NNN: <Title>

## Status
Accepted

## Context
What is the issue that we're seeing that is motivating this decision?

## Decision
What is the change that we're proposing and/or doing?

## Consequences
What becomes easier or more difficult because of this change?

## Alternatives Considered
What other options were evaluated? Why were they rejected?
```

4. Keep it concise and LLM-friendly — clear sections, explicit rationale.
5. Update `CLAUDE.md` if the next available ADR number needs incrementing.

## Review Loop

After writing the ADR:

1. Launch 2 parallel subagents to review the document (read-only — subagents must not modify any files):
   - Agent 1 (Accuracy): Verify the ADR's Context and Decision sections accurately reflect the actual codebase and design-plan.md. Check that Alternatives Considered includes all options that were discussed — even rejected ones — with clear reasons for rejection. Verify source references (design-plan.md sections, external URLs) are accurate and properly cited. Confirm filename follows the `NNN-<short-slug>.md` convention.
   - Agent 2 (Clarity): Review as a future reader — is the rationale self-contained? Could someone unfamiliar with the discussion understand the "why"? Are Consequences complete (both benefits and costs)?
2. If issues are found, revise the ADR and re-verify.
3. Repeat until no new issues. Hard limit: 3 iterations. Stop after 3 even if issues remain and report them as unresolved.
4. Report the final ADR with iteration count.
