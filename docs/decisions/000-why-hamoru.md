# ADR-000: Why hamoru — Competitive Analysis and Differentiation

## Status

Accepted

## Context

The LLM orchestration space has multiple existing tools. Before building hamoru, we needed to understand where it fits and why it should exist.

The space divides into three categories:

| Category | Examples | Approach |
|----------|----------|----------|
| LLM Gateways | LiteLLM, TensorZero, Portkey, OpenRouter | Unified API + Routing + Observability |
| Intelligent Routers | Martian, Not Diamond, RouteLLM | AI-driven prompt analysis → model selection |
| Orchestration Frameworks | LangGraph, Haystack, AutoGen (Microsoft) | Code-based workflow construction |

TensorZero is the closest competitor: Rust-based, GitOps configuration, high-performance gateway. Its core approach is POMDP-based statistical optimization of single inferences.

## Decision

hamoru differentiates on an orthogonal axis: **declaratively controlling how multiple LLMs collaborate.** Three structural differentiators:

### 1. Declarative Agent Collaboration Engine (Primary)

No existing tool can declaratively define LLM collaboration patterns (Generator/Evaluator, Pipeline, Debate) in YAML and execute them with integrated policy-based model selection. TensorZero's episodes are observational groupings, not execution control. LangGraph/AutoGen have workflow execution but require code (Python or TypeScript), not declarative YAML definitions.

### 2. Policy as Code — Intent-Based Model Selection

Instead of statistical optimization (TensorZero's bandit/A/B approach), hamoru uses `tags: [review] → quality-first policy → Opus auto-selected`. Declarative, auditable, immediately effective on config change.

### 3. plan — Telemetry-Based Cost Impact Prediction

Simulate "how costs will change" before applying policy/workflow changes, using historical telemetry data. TensorZero optimizes post-hoc; hamoru predicts pre-application.

### Why Integration Is the Moat

Individual features can be imitated. The moat is the integration: collaboration patterns × Policy Engine × cost impact prediction. Reproducing this requires a design-philosophy-level pivot from competitors.

### Why LLM Vendors Won't Easily Enter This Space

LLM vendors (OpenAI, Anthropic, Google) are unlikely to build multi-provider orchestration tools — it would mean routing traffic to competitors. This gives independent tools like hamoru a structural advantage in the multi-provider coordination space.

### What Is NOT a Differentiator

Local LLM support, OpenAI-compatible API, cost tracking, rate limiting, fallback — competitors already have these or can easily implement them.

## Consequences

- Primary purpose is learning, but design decisions are production-grade
- Each Phase delivers standalone value even if the project stops there
- We do not compete on features that are table-stakes for LLM gateways

## Alternatives Considered

- **Building on top of TensorZero/LiteLLM**: Rejected — the collaboration engine requires deep integration with routing and workflow execution that gateway APIs don't expose
- **Code-based framework (like LangGraph)**: Rejected — Rust gives performance guarantees needed for a gateway role, and the learning goal favors Rust. LangGraph also requires code rather than declarative definitions

## References

- [TensorZero](https://github.com/tensorzero/tensorzero) — Rust-based LLM gateway with POMDP optimization
- [LiteLLM](https://github.com/BerriAI/litellm) — Python LLM gateway with unified API
- [Portkey](https://github.com/Portkey-AI/gateway) — AI gateway with routing and observability
- [OpenRouter](https://openrouter.ai/) — Unified LLM API marketplace
- [LangGraph](https://github.com/langchain-ai/langgraph) — Code-based LLM workflow framework (Python/TypeScript)
- [AutoGen](https://github.com/microsoft/autogen) — Microsoft's multi-agent framework (Python)
- [Haystack](https://github.com/deepset-ai/haystack) — Python framework for LLM applications
- [MCP](https://modelcontextprotocol.io/) — Model Context Protocol for external tool integration
