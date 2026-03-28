# hamoru — LLM Orchestration Infrastructure as Code

> hamoru: from Japanese ハモる (to harmonize)
> "Multiple LLMs, one harmonious interface"

## 1. Vision

hamoru is an orchestration infrastructure tool aiming to be "Terraform for LLMs."

Just as Terraform enables declarative management of cloud infrastructure, hamoru declaratively manages multiple LLM providers, automatically selects optimal models based on cost/quality/latency policies, and executes them as workflows.

The final form serves as an OpenAI-compatible API, appearing as a single LLM from the outside while internally running multi-model orchestration — realizing "an LLM that orchestrates LLMs."

### 1.1 Why hamoru? — Differentiation from Existing Tools (ADR-000)

#### Competitive Analysis

The LLM orchestration space can be broadly divided into three categories:

| Category | Representative Tools | Approach | Relationship to hamoru |
|----------|---------------------|----------|----------------------|
| **LLM Gateways** | LiteLLM, TensorZero, OpenRouter, Portkey | Unified API + Routing + Observability | Closest competitors, especially TensorZero |
| **Intelligent Routers** | Martian, Not Diamond, RouteLLM | AI-driven prompt analysis → automatic model selection | Complementary. hamoru's Policy Engine uses declarative rules |
| **Orchestration FWs** | LangGraph, Haystack, AutoGen (Microsoft) | Code-based workflow construction | Different approach. hamoru uses declarative definitions |

**Notable Competitor: TensorZero**

TensorZero is philosophically closest to hamoru. Rust-based, GitOps configuration, high-performance gateway (P99 < 1ms), multi-provider support. Has raised $7.3M in seed funding and processes approximately 1% of global LLM API spend.

TensorZero's core approach is modeling LLM applications as a **POMDP (Partially Observable Markov Decision Process)** and continuously improving through a **statistical optimization data flywheel**. It integrates optimization techniques including Best-of-N sampling, multi-armed bandit A/B testing, and fine-tuning.

#### hamoru's Structural Differentiation

While TensorZero focuses on "how to optimize a single inference (POMDP + statistical optimization)," hamoru differentiates on an orthogonal axis: **"declaratively controlling how multiple LLMs collaborate."**

**1. Declarative Agent Collaboration Engine (Primary Differentiator)**

The need for "LLM collaboration patterns" is evident in Claude Code's AgentTeams, AWS Bedrock's Evaluator Reflect-Refine pattern, and OpenAI Swarm's Handoff pattern. However, no tool currently exists that can **declaratively define these patterns in configuration files and execute them with integrated policies**.

- TensorZero's episodes are "logical groupings for observation and tracking," not mechanisms for workflow execution control (conditional branching, loops, inter-role collaboration)
- LangGraph / AutoGen (Microsoft) have workflow execution but are code-based (Python/TypeScript SDKs), requiring programming. They are not designed for declarative YAML definitions executed via CLI
- hamoru declares role-based agent definitions + collaboration patterns (Generator/Evaluator, Pipeline, Debate, etc.) in YAML, with the Policy Engine automatically assigning models to each agent (details: Section 6.4)

**2. Policy as Code — Intent-Based Automatic Model Selection**

TensorZero's model selection is done through variant weight distribution and A/B testing (statistical optimization from historical data). hamoru uses an approach like `tags: [review] → quality-first policy applied → Opus auto-selected`, **declaring task intent and routing via policies**. This is fundamentally different from statistical optimization, with advantages in configuration readability, auditability, and immediate reflection of changes.

**3. plan — Telemetry-Based Cost Impact Prediction**

When policies or workflows change, hamoru can simulate "how costs will change" based on historical Telemetry data. Unlike TensorZero's post-hoc analysis approach (optimizing via bandits from production data), hamoru visualizes impact before applying changes.

#### Reasons TensorZero Cannot Easily Backport

| hamoru Differentiator | Why TensorZero Cannot Easily Backport |
|----------------------|--------------------------------------|
| Declarative agent collaboration | Requires fundamental TOML schema extension + new workflow runtime implementation. Directionally different from POMDP/statistical optimization design philosophy |
| Policy as Code | Tag-based routing is a different model from TensorZero's variant weight/bandit selection. Coexistence is possible but would compromise design consistency |
| plan (cost impact prediction) | TensorZero's GitOps applies configuration; it lacks a pre-application simulation layer |
| **Integration of all 3** (collaboration × Policy × plan) | Individual features can be imitated, but reproducing the integrated experience of "Policy automatically assigning models to agents in collaboration patterns while predicting cost impact of configuration changes" requires a design-philosophy-level pivot. The essence of differentiation lies in this integration, not individual features (detailed analysis to be documented separately) |

#### What Is NOT a Differentiator (competitors already have or can easily implement)

Local LLM support, OpenAI-compatible API, cost tracking, rate limiting, fallback — we do not compete on these.

#### Structural Reasons LLM Vendors Won't Enter

LLM vendors like Google / Anthropic / OpenAI have no incentive to build vendor-neutral orchestration tools. Lock-in to their own models is the revenue source, and a routing infrastructure that includes competitor models conflicts with their interests. This space is the domain of third parties, a market for players aiming to be "the HashiCorp of LLMs."

**The primary purpose of this project is learning, with emphasis on what was learned in each Phase. The design ensures each Phase delivers standalone value even if stopped midway. However, production-oriented design decisions are embedded throughout, so that learning outcomes directly become sources of competitive advantage.**

## 2. Design Principles

| # | Principle | Description |
|---|-----------|-------------|
| 1 | **Declarative First** | Configuration is declared in code. Define the desired state, not procedural scripts |
| 2 | **Layered Abstraction** | Each layer is independently testable. Upper layers depend on lower-layer abstractions |
| 3 | **Plan before Apply** | Configuration changes can go through plan → confirm → apply (※Hot Reload also supported in parallel. Usefulness validated in Phase 2) |
| 4 | **Provider Agnostic** | No dependency on specific LLM vendors. Local LLMs are first-class citizens |
| 5 | **Observable** | All requests, costs, and quality metrics are recorded and visualizable |
| 6 | **Secure by Default** | Safe credential management, API rate limiting, prompt injection mitigation |
| 7 | **Intent-Driven Routing** | Instead of directly specifying models, declare task intent (tags) and let policies select the model |

### 2.1 Scope and Limits of the Terraform Metaphor

hamoru is inspired by Terraform, but there are fundamental differences between LLM orchestration and cloud infrastructure management:

- **Absence of drift**: Terraform's tfstate manages the diff between "declared state" and "actual state of cloud resources." LLM routing has no "remote state" for hamoru to track
- **Iteration speed**: Cloud infrastructure changes operate on minute-to-hour timescales, while LLM routing iterates in seconds

Given these differences, the Terraform metaphor is applied "selectively" rather than "as-is":
- **Adopted**: Declarative configuration management, Provider Abstraction (trait = Provider Plugin), Policy as Code
- **Adopted with validation**: plan/apply flow (comparative validation with Hot Reload in Phase 2)
- **Not adopted**: tfstate-like single State concept → separated into Configuration and Telemetry (Section 6.1)

## 3. Architecture Overview

```
┌──────────────────────────────────────────────────────────┐
│                    User Interface                         │
│  CLI: hamoru plan / apply / serve / status                │
│  REST: OpenAI-compatible API (POST /v1/chat/completions)  │
├──────────────────────────────────────────────────────────┤
│  Layer 5: Agent Collaboration Engine          [NEW]       │
│  Declarative agent definitions (YAML)                     │
│  Patterns: Generator/Evaluator, Pipeline, Debate          │
│  Harness: cost caps, timeouts, quality gates              │
├──────────────────────────────────────────────────────────┤
│  Layer 4: Orchestration Engine                            │
│  Workflow definitions (YAML) → step DAG execution         │
│  Branching (Tool Calling / STATUS line), loops, parallel  │
├──────────────────────────────────────────────────────────┤
│  Layer 3: Policy Engine                                   │
│  Declarative policies: cost caps / quality / latency      │
│  Task intent (tags) → policy matching → model selection   │
├──────────────────────────────────────────────────────────┤
│  Layer 2: Provider Abstraction                            │
│  Unified trait: LlmProvider (direct impl w/ reqwest+serde)│
│  Adapters: Claude API / Ollama → later: OpenAI / Gemini   │
├──────────────────────────────────────────────────────────┤
│  Layer 1: Configuration & Telemetry                       │
│  Configuration: YAML (Git-managed, Hot Reload)            │
│  Telemetry: execution history / cost (InMemory→SQLite→S3) │
│  plan (Telemetry-based cost impact prediction)            │
└──────────────────────────────────────────────────────────┘
```

## 4. Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | **Rust** | Memory safety, type-driven inter-layer contracts, Wasm support, CLI performance |
| Build | cargo | Unified toolchain for testing, docs, publishing |
| Async Runtime | tokio | Parallel LLM API calls, REST API server |
| HTTP Client | reqwest | Provider Adapter HTTP communication |
| HTTP Server | axum | OpenAI-compatible REST API serving |
| Serialization | serde + serde_yaml / serde_json | Type-safe serialization for config files and API communication |
| Local DB | SQLite (rusqlite) | Local Telemetry persistence (introduced in Phase 2) |
| Remote Storage | S3-compatible (aws-sdk-s3 or rust-s3) | Remote Telemetry sync (introduced in Phase 2) |
| CLI Framework | clap | Subcommand and argument parsing |
| Logging | tracing | Structured logging, OpenTelemetry-compatible |
| License | **MIT** | |

### Reference Libraries (design reference only, not adopted as dependencies)

- [graniet/llm](https://github.com/graniet/llm) — Rust multi-provider LLM library. Reference for trait design and provider abstraction
- [takt](https://github.com/nrslib/takt) — Workflow enforcement philosophy. Design reference for Orchestration layer
- [TensorZero](https://github.com/tensorzero/tensorzero) — Rust LLM gateway. Reference for performance design, GitOps config, Observability

**Providers are implemented directly with reqwest + serde.** Depending on third-party abstraction libraries risks waiting for library updates to support provider-specific features (Claude's Prompt Caching, OpenAI's Structured Outputs, etc.). Each adapter is implementable in ~200-400 lines and deepens understanding of API specifications.

## 5. Security Design

### 5.1 Credential Management

Credentials such as API keys are resolved in the following priority order (same approach as Terraform):

1. **Environment variables** (highest priority): `HAMORU_ANTHROPIC_API_KEY`, `HAMORU_OPENAI_API_KEY`, etc.
2. **OS keychain**: macOS Keychain / Linux Secret Service (future)
3. **Config file**: `~/.hamoru/credentials.yaml` (file permission 600 enforced)

API keys must **never** be included in `hamoru.yaml` or workflow definitions. Configuration files contain only provider type and endpoint.

### 5.2 API Server Security (`hamoru serve`)

- **API key authentication**: `Authorization: Bearer hamoru-xxx` header required
- **Rate limiting**: Token bucket. Default 60 req/min, configurable override
- **Cost guardrails**: In addition to `max_cost_per_request`, global caps for `max_cost_per_minute` / `max_cost_per_day`
- **Bind address**: Default `127.0.0.1` (localhost only). External exposure requires explicit `--bind 0.0.0.0`

### 5.3 Prompt Injection Mitigation

The workflow `{previous_output}` template injects LLM output directly into the next prompt, creating injection risk.

**Fundamental mitigation: Role-based message separation**

Instead of embedding previous step output in instruction text, the Orchestration Engine **always adds it as a separate User Role message** in the messages array. This separates System Instructions and previous step output at the role level, structurally preventing XML tag closing attacks and similar exploits.

```rust
// Messages constructed by Orchestration Engine
vec![
    Message::system(step.instruction),          // Step instruction (system)
    Message::user(previous_step_output),         // Previous step output (separated via user role)
]
```

Additional measures:
- Warning logs when inter-step output contains system-instruction-like patterns
- Future consideration for sandboxed evaluation steps

## 6. Layer Design Detail

### 6.1 Layer 1: Configuration & Telemetry

**Responsibility**: Configuration management and execution history recording for hamoru.

Terraform's `tfstate` manages the diff between "declared state" and "actual state of remote resources," but LLM routing has no "remote state" that drifts. Therefore, hamoru separates **Configuration** and **Telemetry** explicitly instead of using a single `tfstate`-like concept.

#### Configuration (Static, Git-managed)

```
hamoru.yaml              → providers, defaults
hamoru.policy.yaml       → policies, routing_rules, cost_limits
hamoru.workflow.yaml     → workflows (multiple allowed)
hamoru.agents.yaml       → agent definitions and collaboration patterns (multiple allowed)
```

- Version-controlled as YAML files in Git
- Changes via **Hot Reload**: edit YAML while `hamoru serve` is running → auto-detected and applied
- `hamoru plan` does not preview Configuration changes themselves, but functions as **cost impact prediction based on Telemetry (historical data)**
- **Hot Reload and MetricsCache consistency**: When Policy YAML changes, the Hot Reload handler processes in order: (1) validate new Policy → (2) recalculate MetricsCache (rebuild aggregations based on new routing rules) → (3) atomically swap new Policy + new Cache. Requests during swap are processed with the old Policy (consistency-first)

#### Telemetry (Dynamic, SQLite/S3)

```
Telemetry
├── history     // Request history (input_hash/output_hash/model/latency/tokens/cost)
│               // Default: hashes only. --verbose-history for raw data (chmod 600)
├── metrics     // Aggregated metrics (daily cost, per-model success rate, etc.)
│               // → Cached in memory at startup, periodically updated (Policy Engine perf)
└── sessions    // In-progress workflow/agent collaboration session state
```

**Progressive Storage Implementation:**

| Phase | Backend | Purpose |
|-------|---------|---------|
| With Phase 1 | InMemory + JSON file | Minimal execution history |
| Phase 2 | SQLite | Local persistence, metrics aggregation |
| Phase 2 (later) | S3/R2 | Remote sync, team sharing |

**Telemetry Store trait:**

```rust
#[async_trait]
trait TelemetryStore {
    async fn record(&self, entry: &HistoryEntry) -> Result<()>;
    async fn query_metrics(&self, period: Duration) -> Result<Metrics>;
    async fn load_cache(&self) -> Result<MetricsCache>;
}
```

### 6.2 Layer 2: Provider Abstraction

**Responsibility**: Handle different LLM providers through a unified interface. Equivalent to Terraform's Provider Plugin.

**Core trait:**

```rust
#[async_trait]
trait LlmProvider: Send + Sync {
    fn id(&self) -> &str;
    async fn list_models(&self) -> Result<Vec<ModelInfo>>;
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>>;
    async fn model_info(&self, model: &str) -> Result<ModelInfo>;
}

struct ModelInfo {
    id: String,
    provider: String,
    context_window: u64,
    cost_per_input_token: f64,              // USD — Source: hardcoded + config override
    cost_per_output_token: f64,             // USD
    cost_per_cached_input_token: Option<f64>, // Input token cost with Prompt Caching (~1/10 of normal)
    capabilities: Vec<Capability>,          // Chat, Vision, FunctionCalling, Reasoning, etc.
    max_output_tokens: Option<u64>,
}

// Capabilities are enumerated for future extensibility.
// Reasoning models (o1, o3-mini, DeepSeek-R1, etc.) don't support System Prompt,
// don't support Temperature, and have different streaming behavior.
// When Layer 4 (Orchestration Engine) detects Capability::Reasoning,
// it applies fallback processing such as merging System Messages into User Messages.
enum Capability {
    Chat,
    Vision,
    FunctionCalling,
    Reasoning,       // Reasoning models (o1, o3-mini, DeepSeek-R1, etc.)
    PromptCaching,   // Prompt Caching support
}

// Message follows OpenAI's content_parts model,
// designed to handle multimodal (images, audio, etc.) from the start.
struct Message {
    role: Role,                  // System, User, Assistant, Tool
    content: Vec<ContentPart>,   // Can store non-text content
}

enum ContentPart {
    Text(String),
    ImageUrl { url: String },
    ImageBase64 { media_type: String, data: String },
    // Audio etc. added in the future
}

struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: Option<f64>,
    max_tokens: Option<u64>,
    tools: Option<Vec<Tool>>,
    stream: bool,
}

struct ChatResponse {
    content: String,
    model: String,
    usage: TokenUsage,
    latency_ms: u64,
    finish_reason: FinishReason,
    tool_calls: Option<Vec<ToolCall>>,
}

struct TokenUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: Option<u64>,  // Prompt Caching: cache creation tokens
    cache_read_input_tokens: Option<u64>,       // Prompt Caching: cache read tokens
    // On cache hit, cache_read_input_tokens are calculated at discounted rates.
    // This significantly improves PolicyEngine cost calculation and hamoru plan prediction accuracy.
}
```

**Cost Information Strategy:**

1. **Default values**: Major model pricing hardcoded (periodically updated)
2. **Config override**: Per-model override in `hamoru.yaml` providers section
3. **Actuals-based**: Compute actual costs from Telemetry execution history to improve estimation accuracy

```yaml
providers:
  - name: claude
    type: anthropic
    models:
      - id: claude-sonnet-4-6
        cost_override:
          input_per_1m: 3.00
          output_per_1m: 15.00
          cached_input_per_1m: 0.30   # Input cost with Prompt Caching
```

**Providers Implemented in Phase 1:**

1. **Claude API** — Direct reqwest implementation of Anthropic Messages API (including SSE streaming)
2. **Ollama** — Direct reqwest implementation of Ollama HTTP API (localhost:11434)

**Provider Implementation Structure:**

```rust
// crates/hamoru-core/src/provider/anthropic.rs
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn id(&self) -> &str { "claude" }
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        // Convert ChatRequest → Anthropic Messages API request
        // POST /v1/messages
        // Convert response → ChatResponse (extract usage.input_tokens, output_tokens)
    }
    async fn chat_stream(&self, request: ChatRequest) -> Result<...> {
        // Parse SSE (Server-Sent Events)
        // Generate chunks from content_block_delta events
        // Finalize TokenUsage on message_stop event
    }
}
```

```rust
// crates/hamoru-core/src/provider/ollama.rs
pub struct OllamaProvider {
    client: reqwest::Client,
    base_url: String,
}
```

### 6.3 Layer 3: Policy Engine

**Responsibility**: Automatically select models based on task characteristics and policies. Equivalent to Terraform's Sentinel/OPA.

**Differentiator**: While TensorZero learns "which model is statistically better" via variant weights/bandits, hamoru explicitly routes using task intent declaration via `tags` + policies ("which model should be used for this task"). Superior in readability, auditability, and immediate reflection.

**Policy Definition (YAML):**

```yaml
# hamoru.policy.yaml
policies:
  - name: cost-optimized
    description: Cost-focused routing
    constraints:
      max_cost_per_request: 0.01  # USD
      max_latency_ms: 5000
    preferences:
      priority: cost  # cost | quality | latency | balanced

  - name: quality-first
    description: Quality-focused (design reviews, etc.)
    constraints:
      min_quality_tier: high  # low | medium | high
    preferences:
      priority: quality

  - name: vision-capable
    description: For tasks involving image input
    constraints:
      required_capabilities: [Vision]  # Models selected by this policy must support Vision
    preferences:
      priority: quality

routing_rules:
  - match:
      tags: [review, architecture]
    policy: quality-first
  - match:
      tags: [generation, boilerplate]
    policy: cost-optimized
  - default:
      policy: cost-optimized

# Global cost guardrails
cost_limits:
  max_cost_per_workflow: 1.00
  max_cost_per_collaboration: 2.00  # Cap for entire agent collaboration session
  max_cost_per_day: 10.00
  alert_threshold: 0.8
```

**Core trait:**

```rust
trait PolicyEngine {
    /// Internal model selection flow:
    /// 1. Match tags → routing_rules to identify policy
    /// 2. Filter models by policy's required_capabilities
    ///    (e.g., Vision tasks → only models with Capability::Vision)
    /// 3. Narrow candidates by constraints (max_cost, max_latency, min_quality_tier)
    /// 4. Score by preferences.priority → return optimal model
    fn select_model(
        &self,
        request: &RoutingRequest,
        available_providers: &[&dyn LlmProvider],
        metrics_cache: &MetricsCache,
    ) -> Result<ModelSelection>;

    fn select_fallback_model(
        &self,
        original: &ModelSelection,
        error: &HamoruError,
        available_providers: &[&dyn LlmProvider],
    ) -> Result<Option<ModelSelection>>;

    fn check_cost_limits(
        &self,
        estimated_cost: f64,
        metrics_cache: &MetricsCache,
    ) -> Result<CostCheckResult>;

    /// Telemetry-based cost impact prediction (for hamoru plan)
    fn simulate_cost_impact(
        &self,
        current_config: &PolicyConfig,
        proposed_config: &PolicyConfig,
        metrics_cache: &MetricsCache,
    ) -> Result<CostImpactReport>;
}

struct CostImpactReport {
    estimated_daily_change: f64,     // USD
    model_shift: Vec<ModelShift>,    // Which models traffic shifts from/to
    confidence: f64,                 // Prediction confidence (based on data volume)
    period_used: Duration,           // Telemetry period used for prediction
}

struct ModelShift {
    from_model: String,
    to_model: String,
    estimated_percentage: f64,
    cost_delta: f64,
}
```

### 6.4 Layer 4: Orchestration Engine

**Responsibility**: Define and execute multi-step workflows. The layer that leverages insights from takt.

**Workflow Definition (YAML — finalized):**

```yaml
# hamoru.workflow.yaml
name: generate-and-review
description: Code generation → review → revision loop
max_iterations: 10
max_cost: 1.00  # USD

steps:
  - name: generate
    tags: [generation]
    instruction: |
      {task}
    transitions:
      - condition: done
        next: review

  - name: review
    tags: [review, architecture]
    instruction: |
      Please review the following output:
      {previous_output}
    # Note: {previous_output} is automatically separated as User Role message (Section 5.3)
    transitions:
      - condition: approved
        next: COMPLETE
      - condition: improve
        next: generate
```

**Key difference from takt**: Model selection for each step is delegated to the Policy Engine. takt specifies `model: opus` directly, while hamoru declares intent with `tags: [review]` and lets the Policy Engine auto-select based on cost/quality constraints.

**Streaming policy**: Intermediate workflow steps are buffered; only the final step streams output. An extension point for `step_index` field in API responses is preserved.

**TTFB (Time to First Byte) mitigation**: Workflows and agent collaborations may take tens of seconds before final output. When served as an OpenAI-compatible API, this risks client-side timeouts. Mitigation: inject progress events into the SSE stream:

```
data: {"object":"chat.completion.chunk","choices":[],"hamoru":{"type":"progress","step":"review","iteration":2,"cost_so_far":0.023}}

```

SSE events with empty `choices` are ignored by OpenAI SDKs, preserving compatibility. hamoru-aware clients can display progress from the `hamoru` field. The validity of this design will be verified against actual SDK behavior in Phase 5 (API Server) and recorded in an ADR.

**Condition evaluation**:
- **v1: STATUS line parsing** — Extracts and normalizes `STATUS: approved` etc. from output tail. Simple to implement but fragile against LLM output variation. Kept as fallback
- **v2: Tool Calling (default)** — Defines `report_status(status: "approved" | "improve" | "done", reason: string)` tool and forces LLM to call it. Robust via structured evaluation. Both v1/v2 are implemented in Phase 4a, with `condition_mode` selectable in workflow YAML

**Core trait:**

```rust
#[async_trait]
trait OrchestrationEngine {
    fn load_workflow(&self, path: &Path) -> Result<Workflow>;

    async fn execute(
        &self,
        workflow: &Workflow,
        task: &str,
        policy_engine: &dyn PolicyEngine,
        providers: &ProviderRegistry,
        telemetry: &dyn TelemetryStore,
    ) -> Result<ExecutionResult>;
}

struct ExecutionResult {
    steps_executed: Vec<StepResult>,
    total_cost: f64,
    total_tokens: TokenUsage,
    total_latency_ms: u64,
    final_output: String,
}
```

#### 6.4.1 Step-Level Context Management (ContextPolicy)

Natively supports context control such as `keep_last_n` from Layer 5 (Agent Collaboration) at Layer 4. This allows Layer 5 to simply set ContextPolicy at compile time while keeping execution logic within Layer 4.

```rust
/// Declarative control over how the messages array is processed before step execution.
/// Applied by Layer 4 before each step execution.
enum ContextPolicy {
    /// Keep all history (default)
    KeepAll,
    /// Keep only the last N iteration outputs
    KeepLastN { n: u32 },
}
// Note: SummarizeOnOverflow requires LLM calls for summarization,
// so it is NOT handled as a Layer 4 ContextPolicy. Instead, Layer 5
// explicitly inserts summarization steps into the DAG at compile time.
```

Also directly specifiable in workflow YAML:

```yaml
steps:
  - name: generate
    tags: [generation]
    context_policy: keep_last_n
    keep_last_n: 2
    instruction: |
      {task}
```

### 6.5 Layer 5: Agent Collaboration Engine (Core Differentiator)

**Responsibility**: Declaratively define multiple LLM agents and execute them based on collaboration patterns.

**Relationship to Layer 4 — Layer 5 is a "compiler"**: Layer 5 functions as a **macro layer that dynamically transpiles declared collaboration patterns (YAML) into Layer 4 Workflow DAGs**. Layer 5 itself has no execution loop or state transitions; it is implemented as a pure transpiler.

- **Unified execution**: Everything ultimately runs through the Layer 4 engine, so logging, Telemetry recording, and error handling are completed in a single mechanism
- **Separation of concerns**: Layer 5 handles only "collaboration pattern → Workflow DAG conversion," delegating execution control entirely to Layer 4
- **Testability**: Conversion logic (YAML → Workflow) can be tested as pure functions

#### 6.5.1 Agent Definitions

```yaml
# hamoru.agents.yaml
agents:
  - name: coder
    role: "Agent responsible for code generation"
    tags: [generation, coding]       # → Policy Engine auto-selects model
    system_prompt: |
      You are an experienced software engineer.
      Generate high-quality code based on requirements.

  - name: reviewer
    role: "Agent responsible for code review"
    tags: [review, architecture]     # → Different policy selects model
    system_prompt: |
      You are a rigorous code reviewer.
      Evaluate code quality, security, and performance from multiple angles.

  - name: security-auditor
    role: "Agent responsible for security auditing"
    tags: [review, security]
    system_prompt: |
      You are a security expert.
      Focus on checking for vulnerabilities, injection, and authentication issues.
```

#### 6.5.2 Collaboration Patterns

hamoru provides the following collaboration patterns as built-ins. Each pattern is internally converted to a Layer 4 workflow for execution.

```yaml
# hamoru.agents.yaml (continued)
collaborations:
  - name: code-gen-review
    description: Code generation → review → revision loop
    pattern: generator-evaluator
    config:
      generator: coder
      evaluator: reviewer
      max_iterations: 5
      harness:
        cost_limit: 1.00         # USD — Cost cap for entire session
        timeout: 120s            # Timeout for entire session
        quality_gate:
          evaluator_must_approve: true   # Loop until Evaluator returns approved
        context_management:
          strategy: keep_last_n  # keep_last_n | summarize_on_overflow
          keep_last_n: 2         # Keep only last 2 iterations

  - name: secure-code-review
    description: Code generation → parallel review (functional + security) → merged judgment
    pattern: pipeline-with-parallel-review
    config:
      stages:
        - agent: coder
          output_key: code
        - parallel:
            - agent: reviewer
              output_key: functional_review
            - agent: security-auditor
              output_key: security_review
        - merge:
            strategy: all-must-approve  # all-must-approve | majority | any
      harness:
        cost_limit: 2.00
        timeout: 180s
```

**Built-in Collaboration Patterns:**

| Pattern | Description | Use Cases |
|---------|-------------|-----------|
| `generator-evaluator` | Generate → evaluate → revision loop | Code gen + review, writing + proofreading |
| `pipeline` | Serial pipeline (A → B → C) | Translation → proofreading → formatting |
| `pipeline-with-parallel-review` | Serial + parallel review | Code gen → functional review + security review |
| `debate` | Multiple agents discuss and form consensus | Design decisions, risk analysis |
| `consensus` | Independent generation → majority vote or best selection | Quality improvement for critical decisions |

#### 6.5.3 Harness Constraints

Safety constraints applied to agent collaboration sessions. Subsumes per-workflow `max_cost` / `max_iterations` with richer constraint expression.

```rust
struct HarnessConstraints {
    cost_limit: Option<f64>,          // USD — Cost cap for entire session
    timeout: Option<Duration>,        // Timeout for entire session
    max_iterations: Option<u32>,      // Max iterations for Generator/Evaluator loop
    quality_gate: Option<QualityGate>,
    context_management: Option<ContextManagement>,  // Context bloat mitigation
}

/// In Generator/Evaluator loops, {previous_output} and conversation history
/// grow snowball-style with each iteration, risking context length and cost explosion.
/// ContextManagement provides declarative control over this problem.
///
/// Implementation approach (separation of concerns with Layer 4):
/// - KeepLastN: Layer 5 sets ContextPolicy::KeepLastN on steps at compile time.
///   Layer 4 processes messages before step execution (Section 6.4.1)
/// - SummarizeOnOverflow: Layer 5 inserts "summarization steps" into the DAG at compile time.
///   Layer 4 executes them as regular steps (keeping LLM call responsibility out of Layer 4)
enum ContextManagement {
    /// Keep only the last N iteration outputs
    /// → Mapped to Layer 4's ContextPolicy::KeepLastN at compile time
    KeepLastN { n: u32 },
    /// Insert a summarization model when token count exceeds threshold
    /// → Summarization steps inserted into DAG at compile time
    SummarizeOnOverflow {
        max_context_tokens: u64,
        summary_tags: Vec<String>,  // → Policy Engine auto-selects summarization model
    },
}

enum QualityGate {
    EvaluatorMustApprove,             // Until Evaluator returns approved
    AllMustApprove,                   // All parallel reviewers must approve
    Majority,                         // Majority must approve
    ScoreThreshold { min_score: f64 }, // Evaluation score meets threshold
}
// Quality Gate evaluation depends on the condition evaluation method (Section 9.1.2).
// v2 (Tool Calling) enables structured evaluation by forcing the Evaluator
// to call approve_or_reject(status, reason) tool.
```

**Cost Control for debate / consensus Patterns (to be evaluated at implementation time)**:

In debate / consensus patterns, costs grow rapidly with `agent count × round count`. Existing `cost_limit` + `timeout` will ultimately stop execution, but the need for dedicated guardrails to structurally prevent explosion will be evaluated at implementation time:

| Option | Constraint | Characteristics |
|--------|-----------|----------------|
| A: Existing constraints only | `cost_limit` + `timeout` | Simple. But tends to "hit the limit before noticing" |
| B: Add dedicated constraints | `max_rounds`, `max_agents_per_round` | Structurally prevents explosion. More config items |
| C: Hybrid | Existing constraints + `max_rounds` only | Balanced. Round count is the main driver of explosion, so controlling just this may suffice |

**Reason for deferral**: debate / consensus are in the Future Roadmap (post Phase 6). It is more accurate to decide after estimating actual cost profiles from Generator/Evaluator historical data. Evaluate in the ADR at Phase 6 completion.

#### 6.5.4 Core trait (Provisional — to be redesigned at Phase 6 start)

> **Note**: The following trait definitions are provisional as of Phase 0. They will be redesigned via ADR at Phase 6 start based on Layer 4 (Orchestration Engine) implementation experience. The delegation method to `OrchestrationEngine`, internal representation of collaboration patterns, and relationship between Result types are likely to change. (Section 9.1.3)

```rust
#[async_trait]
trait AgentCollaborationEngine {
    fn load_agents(&self, path: &Path) -> Result<AgentConfig>;

    /// Compile collaboration pattern into a Layer 4 Workflow.
    /// This conversion logic is the core of Layer 5; execution is fully delegated to Layer 4.
    fn compile(
        &self,
        collaboration: &Collaboration,
        task: &str,
    ) -> Result<Workflow>;  // Returns Layer 4's Workflow type

    /// Helper that runs compile → Layer 4 execute in sequence.
    /// Internally just passes compile() result to OrchestrationEngine::execute().
    async fn execute_collaboration(
        &self,
        collaboration: &Collaboration,
        task: &str,
        policy_engine: &dyn PolicyEngine,
        orchestration_engine: &dyn OrchestrationEngine,
        providers: &ProviderRegistry,
        telemetry: &dyn TelemetryStore,
    ) -> Result<CollaborationResult>;
}

struct CollaborationResult {
    agents_used: Vec<AgentExecution>,   // Execution record for each agent
    iterations: u32,                     // Generator/Evaluator loop count
    total_cost: f64,
    total_tokens: TokenUsage,
    total_latency_ms: u64,
    final_output: String,
    quality_gate_passed: bool,
    harness_report: HarnessReport,       // Constraint fulfillment status
}

struct AgentExecution {
    agent_name: String,
    model_used: String,                  // Model selected by Policy Engine
    policy_applied: String,              // Applied policy name
    cost: f64,
    tokens: TokenUsage,
    latency_ms: u64,
}

struct HarnessReport {
    cost_used: f64,
    cost_limit: Option<f64>,
    time_elapsed: Duration,
    timeout: Option<Duration>,
    iterations_used: u32,
    max_iterations: Option<u32>,
}
```

#### 6.5.5 Differentiation from Existing Orchestration Frameworks

| Feature | hamoru Agent Collaboration | LangGraph | AutoGen (Microsoft) | TensorZero |
|---------|--------------------------|-----------|-------------------|------------|
| Definition | YAML (declarative) | Python (code) | Python (code) | TOML (function/variant only) |
| Model selection | Policy Engine auto-selects from tags | Direct in code | Direct in code | variant weight / bandit |
| Collaboration patterns | Built-in (Generator/Evaluator, etc.) | Build manually | Built-in (Sequential, etc.) | None (episode tracking only) |
| Harness constraints | Cost cap, timeout, quality gate declared in YAML | Implement in code | Implement in code | None |
| Cost impact prediction | Pre-simulation via `hamoru plan` | None | None | None (post-hoc statistical analysis) |
| Execution | CLI or OpenAI-compatible API | Python process | Python process | REST API |

### 6.6 API Layer: OpenAI-Compatible Server

**Responsibility**: Expose hamoru as an OpenAI-compatible REST API.

```
POST /v1/chat/completions
{
  "model": "hamoru:cost-optimized",               // Policy name
  "messages": [{"role": "user", "content": "..."}],
  "stream": true
}

POST /v1/chat/completions
{
  "model": "hamoru:workflow:gen-review",           // Workflow name
  "messages": [{"role": "user", "content": "..."}]
}

POST /v1/chat/completions
{
  "model": "hamoru:agents:code-gen-review",        // Agent collaboration name
  "messages": [{"role": "user", "content": "..."}]
}
```

**model field namespace:**

| Pattern | Resolves to |
|---------|------------|
| `hamoru:<policy>` | Auto model selection via policy |
| `hamoru:workflow:<n>` | Workflow execution |
| `hamoru:agents:<n>` | Agent collaboration execution |
| `claude:claude-sonnet-4-6` | Direct provider passthrough |

Internal flow:
1. Receive request + API key auth + rate limit
2. Resolve namespace from `model` field
3. Delegate to Policy Engine / Orchestration Engine / Agent Collaboration Engine
4. Call LLM via Provider Abstraction
5. Return response in OpenAI format (SSE for streaming)
6. Record execution history in Telemetry

## 7. Configuration File Structure

```
project-root/
├── hamoru.yaml              # Main config file
├── hamoru.policy.yaml       # Policy definitions
├── hamoru.workflow.yaml     # Workflow definitions (multiple allowed)
├── hamoru.agents.yaml       # Agent definitions (multiple allowed)
├── .hamoru/
│   ├── state.json           # Phase 1: Basic Telemetry
│   ├── state.db             # Phase 2: SQLite Telemetry
│   ├── state.lock           # Exclusive lock file
│   └── logs/                # Execution logs
└── ~/.hamoru/
    ├── config.yaml           # Global settings
    └── credentials.yaml      # Provider credentials (chmod 600)
                              # ※ Environment variables recommended
```

**hamoru.yaml (Main config file):**

```yaml
version: "1"

providers:
  - name: claude
    type: anthropic
    models:
      - claude-sonnet-4-6
      - claude-haiku-4-5

  - name: local
    type: ollama
    endpoint: http://localhost:11434
    models:
      - llama3.3:70b
      - qwen2.5-coder:14b

telemetry:
  local:
    path: .hamoru/state.db
  remote:
    backend: s3
    bucket: hamoru-telemetry
    region: auto
    endpoint: https://xxx.r2.cloudflarestorage.com

serve:
  bind: 127.0.0.1
  port: 8080
  rate_limit:
    requests_per_minute: 60
  cost_limits:
    per_minute: 0.50
    per_day: 10.00

defaults:
  policy: cost-optimized
  workflow: default
```

### 7.1 YAML Schema Versioning Strategy

Schema versions are managed via the `version` field in each YAML file.

**Principles:**
- **Patch changes (field additions, default value changes)**: Handled backward-compatibly within the same version. New fields are added as `Option` types with defaults applied when unspecified
- **Breaking changes (field removal, type changes, semantics changes)**: Bump major version (`version: "1"` → `version: "2"`)
- **Migration**: `hamoru migrate` command converts old-version YAML to new version. Conversion logic is implemented as functions like `v1_to_v2()`, with pre-conversion files backed up as `.bak`
- **Support range**: Runtime supports current version + 1 generation back. 2+ generations old require `hamoru migrate` conversion

**Pre-v1.0 policy:** Breaking changes are allowed within `version: "1"` until v1.0 release (prioritizing agility during learning phase). YAML schema is frozen at v1.0; the above rules apply thereafter.

## 8. CLI Commands

```
hamoru init                        # Initialize project (create .hamoru/)
hamoru plan                        # Telemetry-based cost impact prediction
hamoru status                      # Current configuration overview

hamoru run "prompt"                # Single request execution
hamoru run -w <workflow> "task"    # Workflow execution
hamoru run -p <policy> "prompt"   # Execute with specific policy
hamoru run -m <provider:model>    # Direct provider specification
hamoru run -a <collaboration>     # Agent collaboration execution

hamoru serve                       # Start OpenAI-compatible API server
hamoru serve --port 8080 --bind 0.0.0.0

hamoru providers list              # List available providers
hamoru providers test              # Connectivity check for all providers

hamoru agents list                 # List defined agents
hamoru agents test <collaboration> # Dry run collaboration pattern (mock responses)

hamoru metrics                     # Cost/performance report
hamoru metrics --period 7d         # Report for last 7 days

hamoru telemetry show              # Telemetry detail view
hamoru telemetry pull              # Sync from remote
hamoru telemetry push              # Sync to remote
```

## 9. Implementation Phases

### Phase 0: Scaffold & Interface Design

**Goal**: Project skeleton and trait definitions for all layers

**Deliverables:**
- [x] `cargo init` + workspace setup (start with 2 crates)
- [x] Core trait definitions for Layers 1-4 (compiling state)
- [x] Layer 5 (Agent Collaboration) traits as **provisional definitions** only (to be redesigned at Phase 6 start. Details: 9.1.3)
- [x] Provider skeletons (empty implementations of AnthropicProvider, OllamaProvider)
- [x] Unified error type design (including variant enumeration — details: 9.1.1)
- [x] Workflow condition evaluation method finalized (details: 9.1.2)
- [x] CI setup (GitHub Actions: `cargo test`, `cargo clippy`, `cargo fmt`)
- [x] Claude Code tooling setup (CLAUDE.md, evaluator.md, commands/)
- [x] README.md / CONTRIBUTING.md
- [x] ADR-000: Why hamoru — Competitive analysis and differentiation strategy (this document Section 1.1)
- [x] ADR-001: Architecture Overview
- [x] ADR-002: Tool Execution boundary — hamoru supports only internal control tools (e.g., `report_status` for state transitions) and considers external tool execution (web search, DB queries, code execution, etc.) out of scope. External tool integration deferred to future MCP integration

#### 9.1.1 Error Type Design

```rust
#[derive(Debug, thiserror::Error)]
pub enum HamoruError {
    // Provider errors (Phase 1)
    #[error("Provider '{provider}' is unavailable: {reason}")]
    ProviderUnavailable { provider: String, reason: String },

    #[error("Model '{model}' not found in provider '{provider}'")]
    ModelNotFound { provider: String, model: String },

    #[error("Provider request failed after {attempts} retries: {source}")]
    ProviderRequestFailed { attempts: u32, source: Box<dyn std::error::Error + Send + Sync> },

    // Telemetry errors (Phase 2)
    #[error("Telemetry store error: {reason}")]
    TelemetryError { reason: String },

    #[error("Telemetry sync failed: {source}")]
    TelemetrySyncFailed { source: Box<dyn std::error::Error + Send + Sync> },

    // Policy errors (Phase 3)
    #[error("No model satisfies policy '{policy}': {reason}")]
    NoModelSatisfiesPolicy { policy: String, reason: String },

    #[error("Cost limit exceeded: {limit} (current: ${current:.4}, max: ${max:.4})")]
    CostLimitExceeded { limit: String, current: f64, max: f64 },

    // Orchestration errors (Phase 4)
    #[error("Workflow '{workflow}' reached max iterations ({max})")]
    MaxIterationsReached { workflow: String, max: u32 },

    #[error("Workflow '{workflow}' exceeded cost limit (${spent:.4} / ${limit:.4})")]
    WorkflowCostExceeded { workflow: String, spent: f64, limit: f64 },

    #[error("Provider failed mid-workflow at step '{step}'")]
    MidWorkflowFailure {
        step: String,
        partial_results: Vec<StepResult>,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    // Agent Collaboration errors (Phase 6)
    #[error("Collaboration '{name}' harness constraint violated: {constraint}")]
    HarnessViolation { name: String, constraint: String },

    #[error("Quality gate not passed after {iterations} iterations in '{name}'")]
    QualityGateNotPassed { name: String, iterations: u32 },

    // Config errors
    #[error("Invalid configuration: {reason}")]
    ConfigError { reason: String },

    #[error("Credential not found for provider '{provider}'")]
    CredentialNotFound { provider: String },
}
```

**Fallback responsibilities by layer:**

| Layer | Responsibility | Example |
|-------|---------------|---------|
| Layer 2 (Provider) | Retry/backoff within same provider | API transient error → exponential backoff retry |
| Layer 3 (Policy) | Fallback selection to another provider | `select_fallback_model()` returns alternative model with same tags |
| Layer 4 (Orchestration) | Workflow abort/continue decision | Fallback success → continue, failure → `MidWorkflowFailure` |
| Layer 5 (Agent Collab.) | Abort/continue based on harness constraints | Cost limit reached → `HarnessViolation` |

#### 9.1.2 Workflow Condition Evaluation Method

**v1: STATUS Line Parsing (fallback)**

The parser scans the last N lines of output in reverse order, adopting the first match. Normalization rules: case-insensitive, trim whitespace, strip trailing periods/punctuation. Kept as fallback for models that don't support Tool Calling (some local LLMs, etc.).

```rust
struct StepOutput {
    full_content: String,
    status: String,
    content: String,    // Body excluding STATUS line
}
```

**v2: Tool Calling (default)**

Defines a `report_status` tool for the LLM and forces it to call. Far more robust than text parsing. Implemented alongside v1 in Phase 4a, with `condition_mode: tool_calling` as default.

```rust
// report_status tool definition (passed to LLM)
Tool {
    name: "report_status",
    description: "Report the status of your evaluation",
    parameters: {
        "status": { type: "string", enum: ["approved", "improve", "done"] },
        "reason": { type: "string" },
    },
}
```

Workflow YAML specification:
```yaml
steps:
  - name: review
    condition_mode: tool_calling  # Default. status_line also selectable
```

#### 9.1.3 Layer 5 (Agent Collaboration) Trait Design Policy

The Layer 5 `AgentCollaborationEngine` trait depends on `OrchestrationEngine` (Layer 4). Finalizing the Layer 5 trait signature while Layer 4 is unimplemented at Phase 0 is high-risk — aspects like "what abstraction to build on top" only become clear after actually building Layer 4.

**Phase 0 approach:**
- Place the trait definitions from Section 6.5 as **provisional definitions** in `agents/mod.rs`
- Ensure compilation passes, but do not guarantee signature stability
- Add `// TODO: Redesign at Phase 6 start (ADR-00X)` comments

**Phase 6 start approach:**
- Redesign trait signatures based on Layer 4 implementation experience
- Specifically re-evaluate:
  - Delegation method to `OrchestrationEngine` (trait parameter vs internal field)
  - Internal representation of collaboration patterns (convert to Layer 4 `Workflow` or have independent execution path)
  - Relationship between `CollaborationResult` and `ExecutionResult`
- Record redesign results in ADR

**Crate structure (start with 2 crates, split as needed):**
```
hamoru/
├── Cargo.toml          # workspace root
├── crates/
│   ├── hamoru-core/    # All layer traits, types, errors, modules
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── provider/      # Layer 2 module
│   │       ├── telemetry/     # Layer 1 module
│   │       ├── policy/        # Layer 3 module
│   │       ├── orchestrator/  # Layer 4 module
│   │       ├── agents/        # Layer 5 module
│   │       ├── server/        # API Layer module
│   │       └── error.rs
│   └── hamoru-cli/     # CLI entry point
│       └── src/
│           └── main.rs
├── docs/
│   └── decisions/      # ADR
```

### Phase 1: Provider Abstraction + Basic Telemetry (Layer 2 + Layer 1 minimum)

**Goal**: Call Claude API and Ollama through a unified interface + execution history recorded

**Deliverables:**
- [ ] `LlmProvider` trait implementation: Claude adapter (reqwest + SSE)
- [ ] `LlmProvider` trait implementation: Ollama adapter (reqwest)
- [ ] Streaming support
- [ ] Retry, backoff, timeout
- [ ] `ModelInfo` retrieval (hardcoded + config override)
- [ ] InMemory + JSON file basic TelemetryStore
- [ ] Per-request execution history recording (model, tokens, cost, latency)
- [ ] Provider tests (mock + integration)
- [ ] `hamoru providers list` / `hamoru providers test` CLI
- [ ] `hamoru run -m <provider:model>` CLI

**Completion criteria:**
```bash
hamoru providers test
# ✓ claude: claude-sonnet-4-6 (200ms, healthy)
# ✓ local: llama3.3:70b (50ms, healthy)

hamoru run -m claude:claude-sonnet-4-6 "Hello"
hamoru run -m local:llama3.3:70b "Hello"
# → Results returned in same output format
# → Execution history recorded in .hamoru/state.json
```

**Learning points**: API differences between providers (Anthropic SSE vs Ollama NDJSON), Rust async streaming (`Pin<Box<dyn Stream>>`), trait object handling.

### Phase 2: Telemetry + plan (Layer 1)

**Goal**: SQLite/S3 persistence and Telemetry-based cost impact prediction

**Deliverables:**
- [ ] SQLite TelemetryStore implementation (replacing Phase 1 JSON Store)
- [ ] S3/R2 Remote Store implementation
- [ ] CompositeStore (Local → Remote sync)
- [ ] Metrics memory cache (loaded at startup, periodically updated)
- [ ] `hamoru plan` — Telemetry-based cost impact prediction
- [ ] `hamoru metrics` basic report

**Completion criteria:**
```bash
hamoru plan
# Policy change detected: routing_rules updated
#   review,architecture → quality-first (unchanged)
#   generation,boilerplate → cost-optimized (unchanged)
#   NEW: security → quality-first
#
# Estimated cost impact (based on last 7d telemetry):
#   Current: $1.42/day (142 requests)
#   Projected: $1.58/day (+$0.16/day, +11.3%)
#   Reason: ~12 security-tagged requests/day will shift
#           from local:llama3.3:70b ($0.001/req) to
#           claude:claude-sonnet-4-6 ($0.014/req)
#   Confidence: 82% (7d of data)

hamoru metrics --period 1d
# Total requests: 142
# Total cost: $0.23
# Avg latency: 1.2s
# Model breakdown:
#   claude-sonnet-4-6: 85 requests ($0.21)
#   llama3.3:70b: 57 requests ($0.02)
```

**Retrospective ADR**: "Was the Telemetry-based cost prediction in plan useful?"

**Learning points**: SQLite integration with Rust, S3-compatible API handling, metrics aggregation and caching design.

### Phase 3: Policy Engine (Layer 3)

**Goal**: Automatic model selection based on policy definitions

**Deliverables:**
- [ ] Policy YAML parser
- [ ] `PolicyEngine` trait implementation
- [ ] Tag-based routing rules
- [ ] Cost-actuals-based model selection (MetricsCache reference)
- [ ] Cost guardrails (per_request, per_workflow, per_collaboration, per_day)
- [ ] `simulate_cost_impact()` — Cost impact prediction for plan
- [ ] `select_fallback_model()` — Fallback selection on failure
- [ ] `hamoru run -p <policy>` / `hamoru run --tags` implementation

**Completion criteria:**
```bash
hamoru run -p cost-optimized "Summarize this briefly"
# → llama3.3:70b auto-selected (cost constraint)
# Selected: local:llama3.3:70b (reason: cost-optimized, est. $0.001)

hamoru run -p quality-first "Review this architecture"
# → claude-sonnet-4-6 auto-selected (quality constraint)

hamoru run --tags review,security "Check for vulnerabilities"
# → quality-first policy applied via routing_rules
```

**Learning points**: Policy as Code design, routing algorithms, metrics-based dynamic optimization.

### Phase 4a: Orchestration Engine — Sequential Execution (Layer 4)

**Goal**: Multi-step workflow sequential execution

**Deliverables:**
- [ ] Workflow YAML parser
- [ ] Sequential step execution engine
- [ ] Condition evaluation v1 (STATUS line parsing), loops, max iterations — get it working first
- [ ] Condition evaluation v2 (Tool Calling) — Define `report_status(status, reason)` tool and force Evaluator to call it. Resolves v1 instability with structured evaluation
- [ ] Workflow YAML `condition_mode: status_line | tool_calling` selectable (default: `tool_calling`)
- [ ] Per-workflow cost cap (`max_cost`)
- [ ] Inter-step context passing (`{previous_output}`, `{task}`)
- [ ] `{previous_output}` User Role message separation (injection mitigation)
- [ ] Step-level ContextPolicy support (`context_policy: keep_all | keep_last_n` — anticipating Layer 5 requirements. Section 6.4.1)
- [ ] Policy Engine-based auto model selection per step
- [ ] Workflow execution report generation
- [ ] `hamoru run -w <workflow>` implementation

**Completion criteria:**
```bash
hamoru run -w generate-and-review "Implement an auth API"
# Step 1: generate (local:llama3.3:70b, cost-optimized)
#   → Code generation complete (2.1s, $0.003)
# Step 2: review (claude:claude-sonnet-4-6, quality-first)
#   → 2 improvement suggestions → improve
# Step 3: generate (local:llama3.3:70b, cost-optimized)
#   → Revision complete (1.8s, $0.002)
# Step 4: review (claude:claude-sonnet-4-6, quality-first)
#   → approved
#
# Workflow complete: 4 steps, $0.047, 12.3s
```

**Learning points**: Workflow engine state transitions, structured evaluation via Tool Calling implementation, inter-step context management.

**Retrospective ADR**: "How much reliability difference was there between v1 (STATUS line parsing) and v2 (Tool Calling)? Is v1 worth keeping as fallback?" "Does ContextPolicy sufficiently cover Layer 5 requirements?"

### Phase 4b: Orchestration Engine — Parallel Execution (Layer 4)

**Goal**: Parallel execution of independent steps

**Deliverables:**
- [ ] Step DAG construction (dependency analysis)
- [ ] Parallel execution engine (`tokio::JoinSet`-based)
- [ ] Parallel step result merge strategy
- [ ] Cost cap apportioning/checking during parallel execution

**Completion criteria:**
```bash
hamoru run -w parallel-review "Review this code"
# Step 1: generate (sequential)
#   → Code generation complete (2.1s)
# Step 2a: review (parallel) + Step 2b: security-check (parallel)
#   → Parallel completion (max 3.2s)
# → Would take 5.3s sequential, completed in 3.2s
```

**Learning points**: Parallel task management in Rust (`tokio::JoinSet`, `select!`), DAG execution engine design.

### Phase 5: API Server — serve

**Goal**: OpenAI-compatible API server

**Deliverables:**
- [ ] `POST /v1/chat/completions` (normal + SSE streaming)
- [ ] `GET /v1/models` (expose policies/workflows/agent collaborations as models)
- [ ] model field namespace resolution
- [ ] API key authentication
- [ ] Rate limiting (token bucket, scope: per_key | global)
- [ ] Cost guardrails
- [ ] `hamoru serve` CLI command
- [ ] Connection test from existing OpenAI SDK

**`GET /v1/models` ID format** — colon-delimited:
- `hamoru:cost-optimized` (policy)
- `hamoru:workflow:generate-and-review` (workflow)
- `hamoru:agents:code-gen-review` (agent collaboration)
- `claude:claude-sonnet-4-6` (direct provider)

**Completion criteria:**
```python
from openai import OpenAI
client = OpenAI(base_url="http://localhost:8080/v1", api_key="hamoru-xxx")

response = client.chat.completions.create(
    model="hamoru:cost-optimized",
    messages=[{"role": "user", "content": "Hello"}]
)
# → All results returned in OpenAI format
```

**Learning points**: Understanding OpenAI API spec, SSE implementation with axum, API gateway design.

### Phase 6: Agent Collaboration Engine (Layer 5 — Core Differentiator)

**Goal**: Declarative agent collaboration execution

**Deliverables:**
- [ ] **ADR: Layer 5 trait redesign** — Redesign Phase 0 provisional definitions based on Layer 4 implementation experience (Section 9.1.3)
- [ ] Agent definition YAML parser
- [ ] Collaboration pattern implementation: `generator-evaluator`
- [ ] Collaboration pattern implementation: `pipeline`
- [ ] Collaboration pattern implementation: `pipeline-with-parallel-review`
- [ ] Harness constraint implementation (cost_limit, timeout, max_iterations, quality_gate)
- [ ] Internal conversion/delegation to Layer 4 (Orchestration Engine)
- [ ] Policy Engine-based auto model selection per agent
- [ ] CollaborationResult / HarnessReport output
- [ ] `hamoru run -a <collaboration>` CLI
- [ ] Execution via OpenAI-compatible API as `hamoru:agents:<n>`
- [ ] `hamoru agents list` / `hamoru agents test` CLI

**Completion criteria:**
```bash
hamoru run -a code-gen-review "Implement an auth API"
# Collaboration: code-gen-review (pattern: generator-evaluator)
#
# Iteration 1:
#   Agent: coder (local:llama3.3:70b, cost-optimized)
#     → Code generation complete (2.1s, $0.003)
#   Agent: reviewer (claude:claude-sonnet-4-6, quality-first)
#     → 3 improvement suggestions → improve
#
# Iteration 2:
#   Agent: coder (local:llama3.3:70b, cost-optimized)
#     → Revision complete (1.8s, $0.002)
#   Agent: reviewer (claude:claude-sonnet-4-6, quality-first)
#     → approved ✓
#
# Collaboration complete:
#   Iterations: 2/5 (max)
#   Cost: $0.047 / $1.00 (limit)
#   Time: 12.3s / 120s (timeout)
#   Quality gate: passed (evaluator approved)

hamoru run -a secure-code-review "Implement a payment API"
# Collaboration: secure-code-review (pattern: pipeline-with-parallel-review)
#
# Stage 1: coder (local:llama3.3:70b, cost-optimized)
#   → Code generation complete (3.2s, $0.005)
#
# Stage 2 (parallel):
#   Agent: reviewer (claude:claude-sonnet-4-6, quality-first)
#     → approved ✓ (functional review)
#   Agent: security-auditor (claude:claude-sonnet-4-6, quality-first)
#     → 1 vulnerability found → rejected ✗
#
# Merge: all-must-approve → NOT PASSED
#   Security audit failed. Review the findings.
```

```python
# Call agent collaboration directly from OpenAI SDK
response = client.chat.completions.create(
    model="hamoru:agents:code-gen-review",
    messages=[{"role": "user", "content": "Implement an auth API"}]
)
# → Generator/Evaluator loop runs internally, final result returned in OpenAI format
```

**Learning points**: Multi-agent collaboration pattern design, harness constraint implementation, building abstraction layers on top of existing workflow engines.

**Retrospective ADR**: "Was the separation of Layer 5 and Layer 4 appropriate? Should they have been merged into Layer 4?"

## 10. Claude Code Development Environment Design

hamoru implementation leverages Claude Code's AgentTeams. Agents are introduced progressively to minimize overhead.

### 10.1 Directory Structure

```
.claude/
├── CLAUDE.md                    # Project-wide context
├── agents/
│   └── evaluator.md             # Evaluator subagent (from Phase 0)
│   # rust-reviewer.md           # Rust-specialized reviewer (differentiate as needed)
│   # planner.md                 # Planner subagent (differentiate as needed)
└── commands/
    ├── phase-plan.md            # /phase-plan <N>
    ├── review-phase.md          # /review-phase <N>
    ├── write-adr.md             # /write-adr <title>
    └── hamoru-eval.md           # /hamoru-eval (hamoru-specific checks)
```

### 10.2 CLAUDE.md — Project Context

Includes:
- hamoru architecture overview (5 layers + API layer)
- **Competitive differentiation**: Design philosophy differences with TensorZero (POMDP optimization vs declarative agent collaboration)
- **Layer boundary rules**: Provider-specific API types must not leak outside `provider/` module
- **Provider direct implementation policy**: Reasons for implementing each API directly with reqwest + serde
- Rust coding conventions: `unwrap()` forbidden, error types defined with `thiserror`
- Testing policy: Providers use mock trait for unit tests, integration tests marked `#[ignore]`
- Security rules: No API keys in code or logs, User Role separation
- Commit messages: Conventional Commits

### 10.3 Agent Definitions

#### evaluator.md — Evaluator Subagent

**Checkpoints:**
1. **Trait contract**: Does it follow all 5 layers' traits?
2. **Layer boundary**: Are provider-specific API types leaking outside `provider/`?
3. **Error handling**: `unwrap()` forbidden. Are `HamoruError` variants appropriate?
4. **Tests**: Are there corresponding tests for new code?
5. **Security**: No hardcoded credentials. User Role separation
6. **Rust quality**: Ownership optimization, `async`/`Send` boundaries
7. **Build verification**: `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`

**Progressive differentiation:**
- Phase 0-2: Evaluator alone
- Frequent Rust-specific issues → Differentiate **rust-reviewer.md**
- Task decomposition precision needed → Differentiate **planner.md**

### 10.4 Slash Commands

| Command | Description |
|---------|-------------|
| `/phase-plan <N>` | Generate detailed implementation plan for Phase N |
| `/review-phase <N>` | Completion review for Phase N |
| `/write-adr <title>` | Generate ADR template |
| `/hamoru-eval` | hamoru-specific architecture check |

### 10.5 AgentTeams Execution Flow

```
/phase-plan <N>
    │  Generate task list
    ▼
┌─────────────────────────────────────┐
│  Generator/Evaluator per task        │
│                                     │
│  Generator (Sonnet)                 │
│    → Code generation                │
│                                     │
│  Evaluator (Opus)                   │
│    → Trait contract, layer boundary, │
│      Rust quality                   │
│                                     │
│  approved → next task               │
│  improve → return to Generator      │
└─────────────────────────────────────┘
    │
    ▼
/review-phase <N>
    │  Phase-wide retrospective
    ▼
/write-adr "Phase N: ..."
    │  Record learnings
    ▼
Next Phase
```

## 11. Development Workflow Summary

**Record ADR at each Phase completion:**
- Rationale for design decisions
- What was learned
- Implications for next Phase
- (Phase 2) Usefulness of plan's cost impact prediction
- (Phase 6) Appropriateness of Layer 5 / Layer 4 separation, practicality of collaboration patterns

**ADRs are saved in `docs/decisions/`.**

## 11.1 Testing Strategy

| Layer | Test Type | Approach |
|-------|-----------|----------|
| Layer 2 (Provider) | Unit | Verify response parsing and error handling with mock `LlmProvider` trait implementation |
| Layer 2 (Provider) | Integration | Marked `#[ignore]`. Connectivity check against real API / real Ollama. Skipped in CI |
| Layer 3 (Policy) | Unit | Verify model selection logic with mock Provider + fixed MetricsCache |
| Layer 4 (Orchestration) | Unit | Verify sequential/parallel/loop state transitions with mock Provider + mock Policy |
| Layer 5 (Agent Collab.) | Unit | Verify `compile()` I/O (YAML → Workflow conversion) as pure functions |
| API Layer | E2E | Spin up `hamoru serve` inside `tokio::test`, send OpenAI-format requests via `reqwest`. Verify response format, SSE chunks, error responses with mock Provider |

**CI Environment (GitHub Actions):**
- `cargo test` (unit + non-`#[ignore]`)
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Integration tests: manual or dedicated workflow (API keys in secrets, `cargo test -- --ignored`)
- Ollama integration tests: evaluate running Ollama in Docker in CI at Phase 1 completion (connectivity only with small model `tinyllama`)

## 11.2 Logging and Debug Experience

In addition to structured logging via `tracing`, design debug experience specialized for workflow/agent collaboration execution.

**Execution trace output:**

```bash
hamoru run -w generate-and-review "Implement an auth API" --verbose
# [TRACE] workflow=generate-and-review step=generate model=local:llama3.3:70b
# [TRACE]   policy=cost-optimized reason="tag:generation matched cost-optimized"
# [TRACE]   input_tokens=156 output_tokens=842 cost=$0.003 latency=2.1s
# [TRACE]   status_parsed="improve" raw_last_line="STATUS: improve"
# [TRACE] workflow=generate-and-review step=review model=claude:claude-sonnet-4-6
# [TRACE]   policy=quality-first reason="tag:review matched quality-first"
# ...
```

**Design approach:**
- Default: Step summary only (model name, cost, status)
- `--verbose`: Add Policy Engine selection reason, raw condition evaluation data, token counts
- `--debug`: Provider HTTP request/response headers, raw SSE events (※API keys masked)
- Each level maps to `tracing` `Level::INFO` / `DEBUG` / `TRACE`
- During workflow execution, `tracing::Span` includes step name and iteration number in scope for clear log context

## 11.3 Failure Scenario UX Design

On error, clearly communicate "what happened" and "what the user should do."

| Scenario | User-facing Message | Internal Behavior |
|----------|-------------------|-------------------|
| All providers down | `Error: All providers are unavailable. Run 'hamoru providers test' to diagnose.` | Per-provider error details shown via `--verbose` |
| Cost limit hit mid-workflow | `Error: Cost limit exceeded at step 'review' ($0.52 / $0.50). Partial results saved to .hamoru/partial/<run-id>.json` | Save partial results as JSON. `hamoru run --resume <run-id>` for resumption (future) |
| SQLite corruption | `Warning: Telemetry database corrupted. Falling back to in-memory store. Run 'hamoru telemetry repair' to attempt recovery.` | Fallback to InMemory Store. Repair command runs `VACUUM` + integrity check |
| YAML validation error | `Error: Invalid policy 'quality-first': unknown field 'min_quality_teir' (did you mean 'min_quality_tier'?)\n  → hamoru.policy.yaml:8:5` | Show file path + line number + typo suggestion |
| API key not set | `Error: Credential not found for provider 'claude'. Set HAMORU_ANTHROPIC_API_KEY or add to ~/.hamoru/credentials.yaml` | Provide specific resolution steps |
| Workflow max_iterations reached | `Warning: Workflow 'generate-and-review' reached max iterations (10). Last output returned as final result.\n  Tip: Increase max_iterations or review the evaluator's criteria.` | Return last iteration output as result (Warning, not Error) |

## 12. Future Roadmap (Post v1.0)

- OpenAI / Gemini Provider additions
- MCP (Model Context Protocol) integration
- Additional collaboration patterns: `debate`, `consensus`
- Workflow / agent collaboration visual editor (Web UI)
- Wasm build → Cloudflare Workers / edge execution
- Plugin system (external distribution of Providers / Policies / Patterns)
- Multi-tenant support (team use)
- ML-based cost prediction/optimization improvements
- OS keychain integration (macOS Keychain / Linux Secret Service)
- Streaming output for workflow intermediate steps
- TensorZero-compatible feedback API (complementing statistical optimization)

## 13. Success Metrics

| Metric | Target |
|--------|--------|
| Test coverage | 80%+ |
| `hamoru serve` latency overhead | < 50ms |
| Code required to add a Provider | Trait implementation only (< 400 lines) |
| Code required to add a collaboration pattern | Trait implementation only (< 500 lines) |
| Connection from existing OpenAI SDK | Zero code changes |
| Agent collaboration YAML definition | Generator/Evaluator pattern writable in ≤ 20 lines |

※ Timeline is flexible based on learning pace. Emphasis on "what was learned in each Phase" over deadlines.
