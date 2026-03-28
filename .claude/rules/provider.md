---
paths:
  - "crates/hamoru-core/src/provider/**"
---

# Provider Implementation Rules

## Implementation Policy

Providers are implemented directly with reqwest + serde. No third-party abstraction libraries. Reasons:
- Immediate support for provider-specific features (Claude's Prompt Caching, OpenAI's Structured Outputs, etc.)
- Each adapter is expected to be ~200-400 lines when fully implemented
- Deep understanding of API specs directly serves the learning goal

## Boundary Reminder

Provider boundary rules are defined in `architecture.md` (Layer Boundary Rules). In addition:
- All cross-layer communication uses shared types (`ChatRequest`, `ChatResponse`, etc.)

## Tracing Patterns

Enforces the "No prompt content in tracing" Hard Rule (CLAUDE.md Rule 8).

**Functions that accept `ChatRequest`, `ChatResponse`, or types containing prompt/message content:**
- Use `#[instrument(skip_all)]` or explicitly skip those parameters
- Safe span fields: `provider`, `model`, `latency_ms`, `tokens`, `cost` (metadata only)
- Unsafe span fields: `messages`, `content`, `prompt`, `request` (prompt content)

```rust
// Good: skip request, expose only metadata as span fields
#[instrument(skip_all, fields(provider = "anthropic", model = %request.model))]
async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> { ... }

// Bad: request is not skipped, prompt content leaks into spans
#[instrument(fields(provider = "anthropic"))]
async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> { ... }
```

**Functions that do NOT handle prompt content** (e.g., `list_models`, `model_info`): no special skip required.

**Streaming (`chat_stream`):** Do not create per-chunk spans. Use `trace!()` events within the existing method-level span instead.
