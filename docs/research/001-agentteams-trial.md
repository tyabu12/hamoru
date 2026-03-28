# Research Note 001: AgentTeams Trial for Phase 6 Design

## Purpose

Trial Claude Code's experimental AgentTeams feature during Phase 3 development
to gather empirical input for Phase 6 (Agent Collaboration Engine) design.

Primary design question: **Should Layer 5 compile collaboration patterns
entirely into Layer 4 Workflows, or does it need an independent execution
path?** (See design-plan.md Section 9.1.3, bullet 2; provisional `compile()`
trait in Section 6.5.4)

## Setup

- Claude Code version: <!-- fill after trial -->
- AgentTeams mode: <!-- in-process / split-pane -->
- Number of Teammates: 2
- Task type: <!-- parallel-research / generator-evaluator -->
- Task description: <!-- fill after trial -->
- Trial date: <!-- YYYY-MM-DD -->
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

- **Error recovery**: How did the Team Lead handle Teammate failures or
  unsatisfactory output? Did it retry, redirect, or abort?
  <!-- Maps to HarnessConstraints and QualityGate design (Section 6.5.3) -->

- **Cost awareness**: Did the Team Lead show awareness of token/cost
  accumulation? Did coordination overhead grow with iterations?
  <!-- Maps to HarnessConstraints cost_limit design (Section 6.5.3) -->

## Implications for Phase 6

<!-- Map observations to the three Phase 6 re-evaluation items (Section 9.1.3):
1. Delegation method to OrchestrationEngine: trait parameter vs internal field
2. Internal representation: compile to Layer 4 Workflow vs independent execution path
3. Relationship between CollaborationResult and ExecutionResult

Additional design areas to address:
- QualityGate design: convergence mechanism for generator-evaluator (Section 6.5.3)
- ContextManagement: keep_last_n vs summarize_on_overflow (Section 6.4.1 / 6.5.3)
- Communication topology: DAG-only vs direct agent messaging
-->

## Decision

<!-- continue / read-only / stop (re-evaluate at GA) -->

## Open Questions Remaining

<!-- What this trial did NOT answer -->
