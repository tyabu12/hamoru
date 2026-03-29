# ADR-007: Agent Framework Integration Design

**Status**: Accepted
**Date**: 2026-03-29
**Phase**: Cross-cutting (accepted during Phase 4a; affects Phase 5 API Server, informs type design from Phase 4b onward)

## Context

hamoru positions itself as an LLM orchestration infrastructure layer — "Terraform for LLMs." The primary external consumers are agent frameworks (Claude Code, LangChain, AutoGen, custom agents) that connect via the OpenAI-compatible API (`hamoru serve`). Three design gaps were identified that would prevent hamoru from functioning as a practical agent infrastructure:

1. **Tool calling passthrough**: Agent frameworks need LLMs to call tools (file read/write, code execution, web search). hamoru must transparently relay `tool_calls` between the LLM and the framework without executing the tools itself. This extends ADR-002 (Tool Execution Boundary), which established that hamoru does not execute external tools. This ADR defines the passthrough mechanism — how tool_calls are represented internally and relayed to clients via the API.
2. **Streaming with tool_calls**: When the LLM responds with tool_calls during a streaming request, hamoru must handle the translation between provider-specific streaming formats.
3. **Collaboration intermediate results**: Server-side Collaboration execution (Generator-Evaluator, Pipeline) can take tens of seconds to minutes. Clients need progress signals to avoid timeouts and monitor cost.

These decisions are recorded together because they form a coherent design for "how agent frameworks interact with hamoru's API."

## Decision: Content Block Model for Internal Types

Extend `ContentPart` with `ToolUse` and `ToolResult` variants, adopting the content block model for hamoru's internal message representation.

```rust
pub enum ContentPart {
    Text { text: String },
    ImageUrl { url: String },
    ImageBase64 { media_type: String, data: String },
    ToolUse { id: String, name: String, input: serde_json::Value },  // new
    ToolResult { tool_use_id: String, content: String },             // new
}
```

`Message` struct remains unchanged. The `MessageContent::Text(String)` shorthand continues to serve the 95%+ plain-text case without allocating a `Vec` (this optimization, approved in design-decisions.md as a deviation from the design doc, is deliberately preserved). Tool-bearing messages use `MessageContent::Parts(Vec<ContentPart>)`.

### Tool result message representation

A tool result is represented as:

```rust
Message {
    role: Role::Tool,
    content: MessageContent::Parts(vec![
        ContentPart::ToolResult { tool_use_id: "call_123".into(), content: "result...".into() },
    ]),
}
```

The `tool_use_id` inside `ContentPart::ToolResult` replaces the need for a separate `tool_call_id` field on `Message`. Provider adapters handle the mapping:

| Provider | Wire format | Internal mapping |
|----------|------------|-----------------|
| Anthropic | `role: "user"` + `tool_result` content block | `Role::Tool` + `ContentPart::ToolResult` |
| OpenAI-compat | `role: "tool"` + `tool_call_id` top-level field | `Role::Tool` + `ContentPart::ToolResult` |

### ChatResponse is NOT changed

`ChatResponse` retains its current structure: `content: String` + `tool_calls: Option<Vec<ToolCall>>` as separate fields. This is intentional — `ChatResponse` is a provider adapter output type (the immediate result of a single LLM call), not a conversation history type. It is consumed by the orchestration engine and condition evaluator, which read `content` and `tool_calls` independently. Converting `ChatResponse` to content blocks would force every consumer to pattern-match on `ContentPart` variants for no benefit, since the provider adapter has already parsed the response.

`Message` (with content blocks) represents conversation history that flows across turns and layers. `ChatResponse` represents a single-call result that is decomposed by its consumers. The asymmetry is intentional and mirrors TensorZero's design, which similarly uses a flat response type internally while supporting content blocks in its API types.

### Rationale

**Industry convergence on content block model.** Anthropic's Messages API uses content blocks natively. OpenAI acknowledged the limitations of separate fields in Chat Completions and shipped the Responses API (March 2025) with heterogeneous Item arrays — effectively the same model. Google Gemini also uses a `parts` array.

**Extensibility.** Adding future content types (thinking blocks, citations, audio) requires only a new `ContentPart` variant. The alternative — adding top-level fields to `Message` for each new type — is the pattern OpenAI moved away from.

**Blast radius is small.** `MessageContent` pattern matching exists in 9 locations: `anthropic.rs` (2), `ollama.rs` (1), `context.rs` tests (6). Additionally, ~26 `Message` construction sites exist across the codebase (mostly in tests). The layered architecture isolates the change — condition evaluation works with `ChatResponse` (not `Message`), Policy Engine does not touch messages, and CLI constructs but does not pattern-match.

**Anthropic provider adapter simplifies.** Anthropic's `tool_use` content blocks map directly to `ContentPart::ToolUse` without field decomposition. LiteLLM's approach (decomposing content blocks into separate `content`/`tool_calls` fields) has been a persistent source of bugs.

### Alternatives Considered

**(A) OpenAI Chat Completions style — separate fields on Message.** `Message { role, content, tool_calls: Option<Vec<ToolCall>>, tool_call_id: Option<String> }`. Simplest to implement and matches LiteLLM/TensorZero's approach. Rejected because OpenAI itself deprecated this pattern, and it creates extensibility problems for future content types. The Anthropic adapter would need complex content block → separate fields decomposition.

### Migration plan

The type changes will be implemented before the next Phase begins. Estimated scope: ~6 files with real changes (types.rs, anthropic.rs, ollama.rs, context.rs, engine.rs, main.rs), ~26 Message constructions to update. No trait signature changes required.

## Decision: Tool Call Buffering for Streaming

Buffer tool_calls in provider adapters while streaming text immediately. `ChatChunk` gains a `tool_calls` field carrying **complete** `ToolCall` objects (not incremental fragments):

```rust
pub struct ChatChunk {
    pub delta: String,
    pub finish_reason: Option<FinishReason>,
    pub usage: Option<TokenUsage>,
    pub tool_calls: Option<Vec<ToolCall>>,  // new — complete, not incremental
}
```

Provider adapters accumulate tool_call chunks internally. Text deltas stream through immediately. When the provider signals message completion (`message_delta` with `stop_reason` for Anthropic, `finish_reason: "tool_calls"` for OpenAI-compat — mapped to internal `FinishReason::ToolUse`), the complete tool_calls are attached to the final `ChatChunk`.

### Final chunk specification

On the final chunk carrying tool_calls: `delta` is an empty string (all text content has already been streamed in prior chunks). When the LLM response contains only tool_calls and no text, a single chunk is emitted with `delta: ""`, `finish_reason: Some(FinishReason::ToolUse)`, and `tool_calls: Some(...)`.

### Shared buffering utility

To avoid duplicating buffering logic across providers (per CLAUDE.md DRY principle), a shared `ToolCallAccumulator` utility will handle the common accumulation logic — accumulating chunks by index, assembling complete `ToolCall` objects. Provider adapters handle provider-specific parsing (Anthropic's content block events vs OpenAI's indexed delta format) and feed parsed fragments into the shared accumulator.

### Rationale

**Agent frameworks cannot act on partial tool_call arguments.** Tool execution requires the complete function name and valid JSON arguments. Streaming argument fragments (`{"query":` → `"weather"}`) provides zero functional benefit to agent framework consumers — they must wait for the complete call regardless.

**Official SDKs handle single-chunk tool_calls correctly.** OpenAI Python SDK and Node.js SDK use append-based accumulation (`arguments += delta`). A single chunk with complete arguments produces identical results to incremental chunks. Tested against SDK source code. LangChain's chunk merging actually has known bugs with incremental delivery (langchainjs#8394) that single-chunk delivery avoids.

**Incremental streaming is a bug factory.** LiteLLM — the most mature proxy with incremental tool_call streaming — has documented issues: parallel tool_calls arguments concatenated into invalid JSON (#7621), delta chunks dropped when `id` is falsy (#20711), missing `id`/`name` fields from Anthropic (#15884). hamoru is written in Rust where these bugs are costlier to diagnose and fix.

**Progressive display loss is acceptable.** The only feature lost is UI progressive display of tool arguments (showing `"query": "wea..."` as the model types). This affects human-facing chat UIs, not agent frameworks. No major agent framework depends on this. The single documented breakage (OpenClaw's streaming JSON parser) was a client-side bug, now fixed.

### Alternatives Considered

**(A) Full incremental streaming translation.** Real-time translation of provider-specific tool_call chunk formats to OpenAI `delta.tool_calls[index]` format. Provides perfect OpenAI streaming fidelity. Rejected due to implementation complexity (per-provider state machines), high bug surface area (proven by LiteLLM experience), and negligible benefit for the target use case (agent frameworks).

**(C) Auto-switch to non-streaming on tool_calls detection.** Disable streaming when `tools` is present in the request. Simplest implementation but loses text streaming entirely. Rejected because agent frameworks always include `tools`, so this would make all requests non-streaming, degrading UX for text-heavy responses.

### Future migration path

If human-facing UI use cases demand incremental tool_call streaming, `ChatChunk.tool_calls` can be changed from `Option<Vec<ToolCall>>` to `Option<Vec<ToolCallChunk>>` (with `index`, optional `id`/`name`, incremental `arguments`). This is additive — existing consumers that wait for complete tool_calls continue to work by accumulating chunks.

## Decision: Staged SSE Progress Events for Collaboration

Implement L0 heartbeat + L1 metadata extension field for Phase 5, with typed events (L3) planned for future.

### What changes (Phase 5 implementation)

**L0: SSE Comment Heartbeat** — axum's `sse::KeepAlive` emits SSE comment lines (`: \n\n`) at a configurable interval (default to be determined during Phase 5 implementation based on proxy compatibility testing; typical values range from 10-30 seconds). Note: axum is not yet a project dependency — the crate placement decision (hamoru-cli vs dedicated crate) is deferred to a separate ADR at Phase 5 start, per CLAUDE.md.

**L1: Empty Choices + `hamoru` Extension Field** — Progress metadata in OpenAI-compatible chunks:

```json
data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"hamoru:agents:code-review","choices":[],"hamoru":{"type":"step_start","step":"reviewer","iteration":2,"model":"claude:claude-sonnet-4-6"}}

data: {"id":"chatcmpl-xxx","object":"chat.completion.chunk","created":1234567890,"model":"hamoru:agents:code-review","choices":[],"hamoru":{"type":"step_complete","step":"reviewer","iteration":2,"cost_so_far":0.031,"tokens_so_far":2847}}
```

`choices: []` is safe — OpenAI SDKs skip empty choices (verified: OpenAI itself emits `choices: []` for usage-only final chunks when `stream_options.include_usage` is set). Progress chunks include all standard OpenAI envelope fields (`id`, `object`, `created`, `model`) to ensure SDK validation passes. The `hamoru` field is ignored by unaware clients. hamoru-aware clients parse it for progress display and cost monitoring.

### Non-streaming Collaboration requests

For non-streaming requests (`stream: false`), the server returns the complete result when all steps finish. Timeout mitigation relies on HTTP-level mechanisms (client-side timeout configuration). If the Collaboration exceeds typical HTTP timeouts, clients should use `stream: true` to benefit from L0/L1 keepalive. A future enhancement could add `Prefer: respond-async` header support for long-running non-streaming requests.

### Error handling for malformed tool_calls

Provider adapters validate tool_call completeness (non-empty `id`, non-empty `name`). Malformed tool_calls are reported as `HamoruError::ProviderError` with actionable details, not silently dropped. Invalid `arguments` JSON is preserved as-is in the `arguments: String` field — parsing is the consumer's responsibility, consistent with the OpenAI API convention where `arguments` is always a raw JSON string.

### Rationale

**hamoru's Collaboration is server-side multi-step execution — unique among competitors.** TensorZero and LiteLLM have no server-side orchestration and therefore no progress reporting need. This is hamoru's differentiator, and L1 makes it observable without breaking OpenAI compatibility.

**Four user needs, prioritized by urgency:**

| Need | L0 | L1 | Planned L2 | Planned L3 |
|------|----|----|-----------|-----------|
| U1: Timeout avoidance | ✓ | ✓ | ✓ | ✓ |
| U2: Cost monitoring | — | ✓ | ✓ | ✓ |
| U3: Progress display | — | ✓ | ✓ | ✓ |
| U4: Step output debugging | — | — | ✓ | ✓ |

L0+L1 covers the three highest-urgency needs. U4 (step output debugging) is available post-hoc through the response body (`ExecutionResult.steps_executed`) and in real-time through the CLI (which uses hamoru-core directly, not SSE).

**CLI has a separate channel.** `hamoru run -a <collaboration>` uses the internal Rust API for rich progress display. SSE progress events are only needed for the API server, where the primary consumers are agent frameworks that typically want final results, not intermediate outputs.

**Hard Rule 8 compliance.** L0/L1 emit only metadata (step names, iteration counts, cost, token counts) — no prompt content. L2 (step output) would include prompt content and requires opt-in + security design, justifying its deferral.

### Alternatives Considered

**(A) SSE Comment Heartbeat only.** axum's `KeepAlive` solves timeout avoidance but provides no cost monitoring or progress display. Insufficient for hamoru's differentiation.

**(B) Extension field only (without heartbeat).** Progress events serve as implicit heartbeats, but events fire only on step transitions. Long-running individual steps (30+ seconds) could still trigger proxy timeouts between events.

**(C) Full typed events from Phase 5.** OpenAI Responses API-style semantic events (`hamoru.step.start`, `hamoru.step.complete`). The richest option but incompatible with standard OpenAI SDKs, requiring a hamoru-specific client library. Phase 5 scope would expand significantly. Rejected for initial release — planned as L3 when demand is validated. The primary consumers (agent frameworks) prioritize final results over real-time step visibility, and the CLI already provides rich progress through the internal API.

## Consequences

- `ContentPart`: Add `ToolUse` and `ToolResult` variants
- `ChatChunk`: Add `tool_calls: Option<Vec<ToolCall>>` field
- `ChatResponse`: No changes — retains `content: String` + `tool_calls: Option<Vec<ToolCall>>` separate fields
- Provider adapters: Update `MessageContent` pattern matches (~9 locations), add tool_call buffering via shared `ToolCallAccumulator`
- Tests: Update ~26 `Message` constructions
- Phase 5 API layer: Translate between OpenAI wire format and internal content blocks
- `OrchestrationEngine`: Expose progress callback hook for both CLI and API server use
- ADR-002 (Tool Execution Boundary) remains in effect — hamoru relays tool_calls but does not execute external tools

### Phase 5 API translation mapping

**Inbound (client → hamoru):** OpenAI wire format in request messages is translated to internal content blocks.

| OpenAI wire format | Internal representation | Note |
|---|---|---|
| `assistant.tool_calls[{id, function.name, function.arguments}]` | `ContentPart::ToolUse { id, name, input }` in `MessageContent::Parts` | `arguments` (JSON string) is parsed to `input` (`serde_json::Value`) |
| `{role: "tool", tool_call_id, content}` | `Role::Tool` + `ContentPart::ToolResult { tool_use_id, content }` | |
| `assistant.content` (string) | `MessageContent::Text(String)` | |

**Outbound (hamoru → client):** Responses use `ChatResponse` directly (not `Message` content blocks). `ChatResponse.content: String` and `ChatResponse.tool_calls: Option<Vec<ToolCall>>` map 1:1 to OpenAI response fields. The `ToolCall.arguments` field is already a JSON string, matching the OpenAI wire format with no conversion needed.

## Deferred to Future Phases

- **axum crate placement** (Phase 5 start): Whether the HTTP server lives in hamoru-cli, a dedicated `hamoru-server` crate, or elsewhere. Requires a separate ADR per CLAUDE.md.
- **L2: Step output in extension field** (post-Phase 5): Adds `"step_output"` to L1 events. Opt-in via request header. Requires Hard Rule 8 security review.
- **L3: Typed SSE events** (post-v1.0): Full semantic event types (`event: hamoru.step.start`). Opt-in mode for hamoru-native UIs/dashboards. Design informed by OpenAI Responses API event types and Anthropic content block lifecycle events. L1 remains the default.
- **Incremental tool_call streaming** (if demand validated): Upgrade `ChatChunk.tool_calls` from `Option<Vec<ToolCall>>` to `Option<Vec<ToolCallChunk>>` for human-facing UI use cases.
- **Non-streaming async Collaboration** (post-v1.0): `Prefer: respond-async` header returning 202 + polling URL for long-running non-streaming requests.

### Design constraints for future compatibility

1. **OrchestrationEngine progress callback hook.** The engine will expose a callback point fired on step start/complete. CLI uses this for terminal display; API server uses it for SSE event emission; L3 typed events will use the same hook.
2. **`hamoru` extension field naming convention.** Field names (`type`, `step`, `iteration`, `cost_so_far`, `tokens_so_far`) are designed to align with a future L3 typed event schema, minimizing migration effort.
3. **L3 does not replace L1.** L1 (OpenAI-compat mode) remains the default even after L3 is available. L3 is opt-in for hamoru-native clients.

## References

- [OpenAI Responses API — Why we built it](https://developers.openai.com/blog/responses-api) — OpenAI's acknowledgment that separate-field message design does not scale to agentic workflows
- [Anthropic Messages API — Tool Use](https://platform.claude.com/docs/en/docs/build-with-claude/tool-use) — Content block model for tool_use
- [Anthropic Streaming Messages](https://platform.claude.com/docs/en/api/messages-streaming) — Content block lifecycle events + ping heartbeat
- [LiteLLM tool_calls issues](https://github.com/BerriAI/litellm/issues/7621) — Incremental streaming bug examples (#7621, #20711, #15884)
- [TensorZero](https://github.com/tensorzero/tensorzero) — Three-tier type system, id-based chunk aggregation
- [axum SSE KeepAlive](https://docs.rs/axum/latest/axum/response/sse/struct.KeepAlive.html) — Built-in SSE heartbeat
- ADR-002: Tool Execution Boundary — Establishes hamoru does not execute external tools; this ADR defines the relay mechanism
- ADR-003: Provider Abstraction Design — Provider-specific types remain private to `provider/` module
