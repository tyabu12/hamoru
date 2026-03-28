# Research Note 001: AgentTeams Trial for Phase 6 Design

## Purpose

Trial Claude Code's experimental AgentTeams feature during Phase 3 development
to gather empirical input for Phase 6 (Agent Collaboration Engine) design.

Primary design question: **Is a static DAG compiler sufficient for Layer 5,
or does it need a runtime coordinator?** (See design-plan.md Section 9.1.3)

## Setup

- Claude Code version: <!-- fill after trial -->
- AgentTeams mode: <!-- in-process / split-pane -->
- Number of Teammates: 2
- Task type: <!-- parallel-research / generator-evaluator -->
- Task description: <!-- fill after trial -->
- Hook inheritance verified: <!-- yes / no — did Teammates inherit PreToolUse/PostToolUse hooks? -->

## Quantitative Observations

| Metric | AgentTeams | Subagent baseline (estimate) |
|--------|-----------|------------------------------|
| Token cost | | |
| Wall-clock time | | |
| Human interventions | | |

## Qualitative Observations

- **Team Lead dynamic decisions**: Did the Team Lead make decisions that a
  static DAG could not have expressed at compile time?
  <!-- This is the most important observation for Phase 6 -->

- **Context degradation**: Was context lost or degraded when passing work
  between Teammates?

- **Direct messaging (mailbox)**: Was Teammate-to-Teammate communication
  useful, or was all coordination through the Team Lead sufficient?

- **Parallel completion variance**: How much did Teammate finish times differ?

- **Rule compliance**: Did Teammates follow CLAUDE.md hard rules without
  explicit enforcement?

## Implications for Phase 6

<!-- Map observations to specific Phase 6 design decisions:
- Compiler vs runtime coordinator (design-plan.md Section 9.1.3)
- QualityGate design (generator-evaluator convergence)
- ContextManagement (keep_last_n vs summarize_on_overflow)
- DAG-only vs direct agent communication
-->

## Decision

<!-- continue / read-only only / stop (re-evaluate at GA) -->

## Open Questions Remaining

<!-- What this trial did NOT answer -->
