<div align="center">

# 🎶 hamoru

**"Terraform for LLMs."**

Declaratively orchestrate multiple LLM providers in harmony,\
with policy-based model selection and cost impact prediction.

*Named after Japanese ハモる (to harmonize)*\
*— because your LLMs should sing together, not solo.*

[![CI](https://github.com/tyabu12/hamoru/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/tyabu12/hamoru/actions/workflows/ci.yml)
[![Security Audit](https://github.com/tyabu12/hamoru/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/tyabu12/hamoru/actions/workflows/security.yml)
[![coverage](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/tyabu12/8c8891a593f77b776e5d672b8dd8ab2c/raw/hamoru-coverage.json)](https://gist.github.com/tyabu12/8c8891a593f77b776e5d672b8dd8ab2c)

</div>

> 🚧 **This project is under active development and is not production-ready.** 🚧
>
> Use at your own risk. APIs and configuration formats may change without notice.

## 🎯 Current Phase

**Phase 4a: Orchestration Engine — Sequential**

See [design-plan.md](docs/design-plan.md) for the full roadmap.

## ✨ Key Differentiators

1. **Declarative Agent Collaboration** — Define LLM collaboration patterns (Generator/Evaluator, Pipeline, Debate) in YAML
2. **Policy as Code** — Intent-based model selection: `tags: [review] → quality-first policy → Opus auto-selected`
3. **Cost Impact Prediction** — `hamoru plan` simulates cost changes before applying policy updates

## 🔌 Supported Providers

| Provider | Type | Models (built-in catalog) | Status |
|----------|------|---------------------------|--------|
| [Anthropic](https://www.anthropic.com/) | Cloud API | `claude-sonnet-4-6`, `claude-haiku-4-5` | ✅ Implemented |
| Google Gemini | Cloud API | — | 🔲 Planned |
| [Ollama](https://ollama.com) | Local | `llama3.3:70b`, `qwen2.5-coder:14b` | ✅ Implemented |
| OpenAI | Cloud API | — | 🔲 Planned |

> Models listed above are from the built-in catalog with default pricing. You can configure any model your provider supports via `hamoru.yaml` — including custom cost overrides.

## 🏗️ Architecture

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

## 📋 Prerequisites

- [Rust](https://rustup.rs/) stable toolchain (`clippy` and `rustfmt` components)

## 🚀 Quick Start

```bash
cargo build
cargo run -p hamoru-cli -- init
```

### Option A: Local LLM (no API key required)

Install [Ollama](https://ollama.com), then add it to your config (`.hamoru/hamoru.yaml`):

```yaml
providers:
  - name: local
    type: ollama
    endpoint: http://localhost:11434
    models:
      - llama3.2
```

```bash
ollama pull llama3.2
cargo run -p hamoru-cli -- providers test
cargo run -p hamoru-cli -- run -m local:llama3.2 "Hello, world!"
```

### Option B: Cloud LLM (Anthropic Claude)

Set your API key as an environment variable:

```bash
# Recommended: read without echoing to avoid shell history leakage
printf "Enter API key: " && read -rs HAMORU_ANTHROPIC_API_KEY && export HAMORU_ANTHROPIC_API_KEY
echo  # newline after silent input
```

> **Security note:** Avoid typing API keys directly in commands (e.g., `export KEY=sk-ant-...`) — they may be saved in your shell history file. Use `read -rs` as shown above, or load from a secrets manager.

```bash
cargo run -p hamoru-cli -- providers test
cargo run -p hamoru-cli -- run -m claude:claude-sonnet-4-6 "Hello, world!"
```

### More examples

```bash
# Policy-based model selection
cargo run -p hamoru-cli -- run -p cost-optimized "Summarize this text"

# Tag-based routing
cargo run -p hamoru-cli -- run --tags review "Review this code for security issues"

# Multi-step workflow
cargo run -p hamoru-cli -- run -w workflow.yaml "Implement an auth API"
```

## 📖 Commands

### Top-level commands

| Command | Description | Status |
|---------|-------------|--------|
| `hamoru init` | Initialize project (creates `.hamoru/` with config templates) | ✅ Implemented |
| `hamoru run <prompt>` | Execute a prompt, workflow, or collaboration | ✅ Implemented |
| `hamoru plan` | Telemetry-based cost impact prediction | ✅ Implemented |
| `hamoru metrics --period 7d` | View cost and performance metrics | ✅ Implemented |
| `hamoru providers list` | List configured providers and their models | ✅ Implemented |
| `hamoru providers test` | Test connectivity to all configured providers | ✅ Implemented |
| `hamoru telemetry show` | Show telemetry store details | ✅ Implemented |
| `hamoru telemetry pull` | Sync telemetry from remote storage | 🔲 Stub (requires remote config) |
| `hamoru telemetry push` | Sync telemetry to remote storage | 🔲 Stub (requires remote config) |
| `hamoru status` | Show current configuration overview | 🔲 Stub |
| `hamoru serve` | Start OpenAI-compatible API server | 🔲 Stub (Phase 5) |
| `hamoru agents list` | List agent definitions | 🔲 Stub (Phase 6) |
| `hamoru agents test <name>` | Dry-run a collaboration pattern | 🔲 Stub (Phase 6) |

### `hamoru run` options

| Flag | Description | Status |
|------|-------------|--------|
| `-m provider:model` | Direct model selection (e.g., `claude:claude-sonnet-4-6`) | ✅ |
| `-p policy-name` | Policy-based model selection (e.g., `cost-optimized`) | ✅ |
| `--tags tag1,tag2` | Tag-based routing (can combine with `-p`) | ✅ |
| `-w workflow.yaml` | Execute a multi-step workflow from YAML | ✅ |
| `-a collaboration` | Execute an agent collaboration pattern | 🔲 Phase 6 |
| `--no-stream` | Disable streaming (print full response at once) | ✅ |

## 🛠️ Development

```bash
# Run tests
cargo test --all-targets

# Check code quality
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

### E2E smoke test

```bash
# Offline only (no API key or Ollama needed)
bash scripts/smoke-test.sh --offline

# Auto-detect (runs Anthropic tests if API key is set, Ollama tests if server is running)
bash scripts/smoke-test.sh

# With Anthropic API tests
printf "API key: " && read -rs HAMORU_ANTHROPIC_API_KEY && export HAMORU_ANTHROPIC_API_KEY
bash scripts/smoke-test.sh

# With Ollama tests (start Ollama in a separate terminal first)
# Terminal 1: ollama serve
# Terminal 2:
ollama pull qwen2.5:0.5b  # recommended: lightweight (~400MB), responds in seconds
bash scripts/smoke-test.sh

# Verbose output (show stdout/stderr for all tests)
bash scripts/smoke-test.sh --verbose
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for full development setup and coding rules.

## 📁 Project Structure

```
hamoru/
├── crates/
│   ├── hamoru-core/          # Core library
│   │   └── src/
│   │       ├── provider/     # Layer 2: LLM provider adapters
│   │       ├── telemetry/    # Layer 1: Execution history & metrics
│   │       ├── config/       # Layer 1: YAML config loading
│   │       ├── policy/       # Layer 3: Policy engine
│   │       ├── orchestrator/ # Layer 4: Workflow execution
│   │       ├── agents/       # Layer 5: Agent collaboration (planned)
│   │       ├── server/       # API layer (planned)
│   │       └── error.rs      # Unified error types
│   └── hamoru-cli/           # CLI entry point
├── docs/
│   ├── design-plan.md        # Detailed design document
│   └── decisions/            # Architecture Decision Records
├── CLAUDE.md                 # Project context for Claude Code
├── CONTRIBUTING.md           # Development guidelines
└── SECURITY.md               # Security policy
```

## 📄 License

[MIT](LICENSE)
