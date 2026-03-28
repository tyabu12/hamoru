# hamoru — Launch & Growth Plan

> Referenced from `docs/design-plan.md` Section 12. This document defines hamoru's public-facing strategy: when and how to make the project visible, build trust, and grow adoption.

## Guiding Principles

- **Ship first, market second.** No public launch until Phase 4a is complete (working workflow execution).
- **Demo with zero trust required.** The first experience must not require API keys (Ollama-only demo).
- **Teach, don't pitch.** Articles and content lead with what was learned, not what the tool does.
- **Build in Public.** Share progress, design decisions, and revenue/star metrics openly.

## Phase-Gated Launch Milestones

Every action below is **blocked** until its corresponding Phase deliverables are complete and CI is green.

| Phase Complete | Action | Details | Precondition |
|---------------|--------|---------|--------------|
| Phase 1 | Zenn article #1 | "Rust で LLM プロバイダー抽象化を 1 から作った話" — Provider trait design, SSE streaming in Rust, API differences across providers | Phase 1 deliverables complete, CI green. **Ready to write now.** |
| Phase 2 | Zenn article #2 | "Telemetry 設計 — Terraform の tfstate を LLM に適用したらこうなった" — Configuration vs Telemetry split, `hamoru plan` cost prediction command | Phase 2 deliverables complete, CI green. **Ready to write now.** |
| Phase 3 | Zenn article #3 | "Policy as Code で LLM のコストを宣言的に制御する" — Tag-based routing, cost impact prediction | Phase 3 deliverables complete, CI green |
| Phase 4a | **Public launch prep** | README overhaul, terminal demo GIF, landing page, awesome-list registration | Phase 4a deliverables complete, CI green |
| Phase 4a | Zenn article #4 | "TensorZero と何が違うのか — LLM オーケストレーターの設計空間" — ADR-000 based competitive analysis | Phase 4a deliverables complete, CI green |
| Phase 4a | English article | Architecture overview + competitive analysis for international audience (dev.to; platform finalized at Phase 4a) | Phase 4a deliverables complete, CI green |
| Phase 4a | Show HN | "Show HN: hamoru — declarative LLM orchestration with policy-based model selection" | Phase 4a deliverables complete, CI green |
| Phase 4a | crates.io alpha | Publish `0.1.0-alpha` to reserve crate name and enable `cargo install` | Phase 4a deliverables complete, CI green |
| Phase 5 | Zenn article #5 | "OpenAI 互換 API の裏側で複数 LLM を協調させる" | Phase 5 deliverables complete, CI green |
| Phase 5 | crates.io stable | Publish `0.1.0` stable release | Phase 5 deliverables complete, CI green |
| Phase 5 | Cost Savings Calculator | Frontend-only widget on landing page (see section below) | Phase 5 deliverables complete, CI green |
| Phase 6 | Zenn article #6 | "LLM 同士の協調パターンを YAML で宣言する — Agent Collaboration Engine の設計" | Phase 6 deliverables complete, CI green |

## crates.io Publication Strategy

| Milestone | Version | Notes |
|-----------|---------|-------|
| Phase 4a | `0.1.0-alpha` | Reserve crate name. CLI commands (`providers`, `run -w`, `plan`, `metrics`) available. **API server (`hamoru serve`) not yet available** — this must be clearly stated in crates.io description and README. |
| Phase 5 | `0.1.0` | First stable release. `hamoru serve` available. Full CLI + API server. |
| Phase 6+ | `0.x.y` | Incremental releases as features stabilize. |
| 1.0 criteria | `1.0.0` | Phase 6 complete + stable YAML schema + 3 months without breaking changes. |

## README & Repository Presentation

**When**: Phase 4a completion (first working demo).

**Contents**:
- One-line description: "Declarative LLM orchestration. Policy-driven model selection. OpenAI-compatible API."
- Terminal demo GIF (asciinema recording — see below)
- Quick start with Ollama (no API key required)
- YAML configuration example (5-10 lines showing policy + workflow)
- Architecture diagram (5-layer ASCII from design-plan.md)
- Comparison table: hamoru vs TensorZero vs LiteLLM vs LangGraph
- Badges: CI status, `cargo audit`, MIT license, test coverage

**Terminal demo GIF** (created AFTER Phase 4a completion):

Must show these three steps in sequence:
1. `hamoru providers test` — both Ollama and Claude healthy
2. `hamoru run -w generate-and-review "Implement auth API"` — policy-based model selection in action, cost tracking per step
3. Total cost summary at the end

## Landing Page (Static Site)

**When**: Phase 4a completion, alongside README.

**Stack**: Static HTML/CSS on Cloudflare Pages or GitHub Pages. Zero hosting cost. External to the hamoru repository.

**Sections**:
1. Hero: tagline + terminal demo GIF
2. "How it works": YAML config → Policy Engine → auto model selection (3-step visual)
3. "Why not just use...": comparison table (TensorZero, LiteLLM, LangGraph)
4. Quick start (link to README)
5. Link to Zenn article series for deep dives

## Cost Savings Calculator (Frontend Widget)

**When**: Phase 5+ (deferred — requires Policy Engine real data from Phase 3+ to make credible estimates).

**Implementation**: Pure frontend (React or vanilla JS). No API calls. Embed on landing page.

**Inputs**:
- Monthly request volume
- Current model (e.g., "All requests go to GPT-4o")
- Task distribution estimate (% review, % generation, % boilerplate)

**Outputs**:
- Estimated monthly cost with single model
- Estimated monthly cost with hamoru policy routing (quality-first for review, cost-optimized for generation)
- Savings amount and percentage
- Which models hamoru would select for each task category

**Data source**: Hardcoded model pricing (same source as `ModelInfo` defaults in hamoru-core).

## Distribution Channels

### Awesome Lists (Phase 4a+)

Register on relevant curated lists once the tool is functional:
- [awesome-rust](https://github.com/rust-unofficial/awesome-rust)
- [awesome-llm](https://github.com/Hannibal046/Awesome-LLM)

Note: [awesome-self-hosted](https://github.com/awesome-selfhosted/awesome-selfhosted) is **not** a fit — hamoru is a CLI tool, not a self-hosted web service.

### Hacker News (Phase 4a)

**Format**: "Show HN: hamoru — declarative LLM orchestration with policy-based model selection"

**Post body must include**:
- What it does (1 sentence)
- Why existing tools don't solve this (TensorZero = statistical optimization, LangGraph = code-based, hamoru = declarative YAML + policies)
- Quick start command (Ollama, no API key needed)
- Link to repo

**HN survival tips**:
- Be ready to respond to comments for 24h
- "Why not just use LiteLLM?" — have a clear, non-defensive answer ready
- Acknowledge what hamoru does NOT do (not a replacement for TensorZero's statistical optimization)

### Zenn (Ongoing)

Article series following Phase milestones (see table above). Each article is standalone but links to previous/next. Written in Japanese, targeting the Japanese developer community.

**Article format**:
- Start with the problem/question that drove the design decision
- Show the ADR reasoning process
- Include code snippets and YAML examples
- End with "what I learned" and link to repo

### English Blog (Phase 4a)

One article timed with Show HN launch. Covers architecture overview and competitive analysis (based on ADR-000). Platform: dev.to (finalized at Phase 4a).

### X / Twitter (Ongoing from Phase 1)

- Share each Zenn article with relevant hashtags (#RustLang, #LLM, #AIEngineering)
- Share terminal screenshots of new features working
- Share interesting design decisions ("Today I learned that LLM API streaming formats are all different...")
- Engage with TensorZero, LiteLLM, and Rust communities
- Quote-tweet or reply to relevant threads in the LLM tooling space

## Trust Building

### Security Signals
- `cargo audit` in CI (badge in README) — already in place
- `SECURITY.md` in repo (created at Phase 4a) — credential handling policy, responsible disclosure process
- Minimal dependency footprint (highlight in README)
- Default `127.0.0.1` bind for `hamoru serve`

### Quality Signals
- Test coverage badge (80%+ target)
- CI badge (clippy + fmt + test) — already in place
- ADR directory visible in repo (shows thoughtful design process)
- Clean commit history (Conventional Commits)
- `CHANGELOG.md` (created at Phase 4a) — track user-facing changes

### Credibility Signals
- Zenn article series (demonstrates deep domain knowledge)
- ADR-000 competitive analysis (shows awareness of landscape)
- `design-plan.md` publicly visible (shows engineering rigor)

## Versioning Strategy

- **0.x**: Active development. Breaking changes are possible between minor versions.
- **1.0 criteria**: Phase 6 complete + stable YAML schema (`version` field unchanged) + 3 months without breaking changes to public APIs or YAML configuration.
- Follow [Semantic Versioning 2.0.0](https://semver.org/).

## What NOT to Do

- **Don't launch before Phase 4a.** A tool that can't run workflows has no demo story.
- **Don't build a web playground.** hamoru's value is in the CLI/YAML experience, not a chat UI. Terminal GIF > web demo.
- **Don't pay for marketing.** OSS adoption is earned through content and community, not ads.
- **Don't chase stars.** Stars follow utility. Focus on making the tool genuinely useful for 10 people before trying to reach 10,000.
- **Don't compare on features TensorZero already wins** (latency, provider count, statistical optimization). Compare on the axis hamoru owns (declarative collaboration, Policy as Code, cost prediction).
- **Don't register on awesome-self-hosted.** hamoru is a CLI tool, not a self-hosted web service. Stick to awesome-rust and awesome-llm.
