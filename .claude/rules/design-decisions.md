# Key Design Decisions

- **Condition evaluation default: Tool Calling (v2)** — Workflow step transitions use `report_status` tool call by default. STATUS line parsing (v1) is kept as fallback for models without tool support. See design-plan.md Section 9.1.2.
- **ContextPolicy on workflow steps** — Steps can declare `context_policy: keep_last_n` to control message history. `SummarizeOnOverflow` is handled by Layer 5 inserting summary steps into the DAG. See design-plan.md Section 6.4.1.
- **Logging levels**: Default (step summary), `--verbose` (policy reasons, tokens), `--debug` (HTTP headers, raw SSE). See design-plan.md Section 11.2.
- **Failure UX**: Every error message must tell the user what happened AND what to do next. See design-plan.md Section 11.3 for the full scenario table.
- **YAML schema changes**: No breaking changes to YAML schema fields without bumping `version`. Additive fields are `Option` with defaults. See design-plan.md Section 7.1. Do NOT rename or remove existing YAML fields without user confirmation.
- **MessageContent enum (deviation from design doc)** — Design doc specifies `Message.content: Vec<ContentPart>`, but implementation uses `MessageContent::Text(String) | MessageContent::Parts(Vec<ContentPart>)` enum to avoid heap-allocating a `Vec` for the 95%+ plain-text case. Approved during Phase 0 planning.

## Error Message Pattern

Error messages must tell the user what happened AND what to do next:
- Good: `"Failed to reach Anthropic API: connection refused. Check HAMORU_ANTHROPIC_API_KEY and network connectivity."`
- Bad: `"API error"`
