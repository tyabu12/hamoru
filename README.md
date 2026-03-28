# hamoru

> from Japanese ハモる (to harmonize) — "Multiple LLMs, one harmonious interface"

**hamoru** is an orchestration infrastructure tool aiming to be "Terraform for LLMs." It declaratively manages multiple LLM providers, automatically selects optimal models based on cost/quality/latency policies, and executes multi-step workflows. The final form serves as an OpenAI-compatible API.

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

## Current Phase

**Phase 0: Scaffold & Interface Design** — Project skeleton and trait definitions for all layers.

See [design-plan.md](docs/design-plan.md) for the full roadmap.

## License

TBD
