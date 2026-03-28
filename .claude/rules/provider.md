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
