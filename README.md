<div align="center">

# 🎶 hamoru

[![CI](https://github.com/tyabu12/hamoru/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/tyabu12/hamoru/actions/workflows/ci.yml)
[![Security Audit](https://github.com/tyabu12/hamoru/actions/workflows/security.yml/badge.svg?branch=main)](https://github.com/tyabu12/hamoru/actions/workflows/security.yml)
[![coverage](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/tyabu12/8c8891a593f77b776e5d672b8dd8ab2c/raw/hamoru-coverage.json)](https://gist.github.com/tyabu12/8c8891a593f77b776e5d672b8dd8ab2c)
[![dependency status](https://deps.rs/repo/github/tyabu12/hamoru/status.svg)](https://deps.rs/repo/github/tyabu12/hamoru)

**"Terraform for LLMs."**

Declaratively orchestrate multiple LLM providers in harmony,\
with policy-based model selection and cost impact prediction.

*Named after Japanese ハモる (to harmonize)*\
*— because your LLMs should sing together, not solo.*

</div>

## Architecture

```
Layer 5: Agent Collaboration Engine  — Declarative agent coordination
Layer 4: Orchestration Engine        — Workflow DAG execution
Layer 3: Policy Engine               — Task intent → automatic model selection
Layer 2: Provider Abstraction        — Unified trait: LlmProvider
Layer 1: Configuration & Telemetry   — YAML config + execution history
API:     OpenAI-Compatible Server    — POST /v1/chat/completions
```

## Key Differentiators

1. **Declarative Agent Collaboration** — Define LLM collaboration patterns (Generator/Evaluator, Pipeline, Debate) in YAML
2. **Policy as Code** — Intent-based model selection: `tags: [review] → quality-first policy → Opus auto-selected`
3. **Cost Impact Prediction** — `hamoru plan` simulates cost changes before applying policy updates

## Prerequisites

- [Rust](https://rustup.rs/) stable toolchain (`clippy` and `rustfmt` components)

## Quick Start

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

## Project Structure

```
hamoru/
├── crates/
│   ├── hamoru-core/    # Core library (traits, types, errors)
│   └── hamoru-cli/     # CLI entry point
├── docs/
│   ├── design-plan.md  # Detailed design document
│   └── decisions/      # Architecture Decision Records
└── CLAUDE.md           # Project context for Claude Code
```

## Status

> **This project is under active development and is not production-ready.**
> Use at your own risk. APIs and configuration formats may change without notice.

## Current Phase

**Phase 1: Provider Abstraction + Basic Telemetry**

See [design-plan.md](docs/design-plan.md) for the full roadmap.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, coding rules, and testing policy.

## License

[MIT](LICENSE)
