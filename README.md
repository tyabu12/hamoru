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
# Build
cargo build

# Run CLI
cargo run -p hamoru-cli -- --help

# Run tests
cargo test --all-targets

# Check code quality
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
```

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

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, coding rules, and testing policy.

## 📄 License

[MIT](LICENSE)
