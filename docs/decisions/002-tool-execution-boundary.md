# ADR-002: Tool Execution Boundary

## Status

Accepted

## Context

LLM orchestration involves tool calling — models can request actions like web searches, database queries, or code execution. We need to define what hamoru is responsible for executing and what it defers to external systems.

## Decision

hamoru supports **only internal control tools** for workflow state management. External tool execution is out of scope and deferred to future MCP (Model Context Protocol) integration.

### Internal Tools (hamoru implements)

- `report_status`: Used for workflow step transitions. The LLM calls this tool to report its evaluation result (e.g., "approved", "improve", "done"), which the Orchestration Engine uses to determine the next step.

### External Tools (out of scope)

Web search, database queries, code execution, file operations, API calls, etc. — all deferred to [MCP (Model Context Protocol)](https://modelcontextprotocol.io/) integration in future phases. MCP is an open standard for connecting AI models to external tools and data sources via a client-server protocol.

### Condition Evaluation Methods

Two methods for evaluating step transitions:

1. **Tool Calling (v2, default)**: The LLM is given a `report_status` tool and forced to call it. More robust and structured.
2. **STATUS Line Parsing (v1, fallback)**: The parser scans the last N lines of the LLM's text output in reverse order, adopting the first STATUS match. Normalization: case-insensitive, trim whitespace, strip trailing punctuation. Inherently less robust than tool calling but kept for models without tool support (some local LLMs).

`condition_mode: tool_calling` is the default in workflow YAML definitions. `status_line` is selectable as a fallback.

## Consequences

- hamoru stays focused on orchestration, not execution — clear scope boundary
- No need to implement sandboxing, security for arbitrary code execution, etc.
- MCP integration path is well-defined for when external tools are needed
- Both condition evaluation methods are supported, ensuring compatibility with all models

## Alternatives Considered

- **Built-in tool execution (web search, code runner)**: Rejected — would massively expand scope, introduce security concerns, and duplicate existing tool ecosystems
- **Plugin system for tools**: Rejected for now — MCP provides a standard protocol for this; building a custom plugin system would be reinventing the wheel
- **STATUS line parsing only**: Rejected as default — tool calling is more robust. STATUS parsing kept as fallback for compatibility
