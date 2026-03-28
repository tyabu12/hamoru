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
