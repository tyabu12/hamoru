---
description: Monthly health check of all Claude Code configuration files. Read-only — reports findings without modifying anything.
allowed-tools: Read, Glob, Grep, WebSearch, Bash, Agent
---

# /review-config

Monthly health check for all Claude Code configuration files. Recommended frequency: once per month.

**This command is strictly read-only. Do NOT modify any files.**

## Before Starting

Read these files to understand current conventions:
- `CLAUDE.md` — project-wide conventions and development commands
- `.claude/settings.json` — permissions, hooks, and env config

## Procedure

Run the following reviews sequentially and collect findings as you go.

### 1. CLAUDE.md Review

Read `CLAUDE.md` and check:

- [ ] **Line count**: Report total lines. Flag if over 200 (getting large for context).
- [ ] **Path accuracy**: Every file path mentioned in CLAUDE.md must exist on disk. Verify each one with Glob or ls.
- [ ] **Tech Stack table**: Cross-reference the Technology Stack table against actual dependencies in `crates/hamoru-core/Cargo.toml` and `crates/hamoru-cli/Cargo.toml`. Flag crates listed in CLAUDE.md but missing from Cargo.toml, and non-dev dependencies in Cargo.toml but missing from CLAUDE.md.
- [ ] **Rules Reference table**: Verify the table matches actual files in `.claude/rules/`. Check that the "Loaded when" column matches each rule file's frontmatter (`paths:` field present or absent).
- [ ] **Current Phase**: Check that the "Current Phase" section is consistent with recent git activity (`git log --oneline -20`).
- [ ] **ADR table**: Verify "Next available number" matches the actual highest-numbered file in `docs/decisions/` plus one.
- [ ] **Staleness**: Flag any section that references features, files, functions, or conventions that no longer exist in the codebase.

### 2. Rules Review

For each file in `.claude/rules/`:

- [ ] **Frontmatter**: Verify YAML frontmatter is well-formed. If `paths:` is present, verify the glob pattern matches at least one existing file (Glob). If absent, confirm the rule is intended to be always-loaded.
- [ ] **Content accuracy**: Spot-check key claims against the actual codebase. Examples:
  - `architecture.md`: Does the crate structure diagram match the actual directory layout?
  - `provider.md`: Do the tracing pattern examples match actual `#[instrument]` usage in provider code?
  - `design-decisions.md`: Are the referenced design-plan.md sections still valid?
- [ ] **Consistency with CLAUDE.md**: Do the rules elaborate on (not contradict) what CLAUDE.md states?

### 3. Commands Review

For each file in `.claude/commands/`:

- [ ] **Tool permissions**: Verify `allowed-tools` lists only valid tool names. Valid tools: Read, Grep, Glob, Bash, Write, Edit, Agent, WebSearch, WebFetch. Tools prefixed with `mcp__` are also valid. Note: commands use `allowed-tools:` while agents use `tools:` — different field names.
- [ ] **`argument-hint`**: If present, verify it matches the command's expected arguments.
- [ ] **Procedure accuracy**: Verify that files and paths referenced in the procedure actually exist.
- [ ] **Review Loop pattern**: Check that commands using subagents include the standard pattern: read-only constraint on subagents, hard limit of 3 iterations.
- [ ] **Agent cross-references**: If a command references an agent (e.g., "evaluator agent's N checkpoints"), verify the count and name match the actual agent file.

### 4. Agents Review

For each file in `.claude/agents/`:

- [ ] **Frontmatter fields**: Verify `name`, `description`, `tools`, `model`, and `maxTurns` are present and reasonable.
- [ ] **Tool list**: Are all tools in the `tools:` field valid Claude Code tool names?
- [ ] **Evaluation criteria**: For the evaluator agent, verify that checkpoint references (types, traits, conventions) still exist in the codebase. Grep for referenced identifiers.
- [ ] **Model**: Report the model setting. Known valid values: `opus`, `sonnet`, `haiku`. Flag unrecognized values.

### 5. Hooks Review

For each hook configuration in `.claude/settings.json` and `.claude/settings.local.json`:

- [ ] **No orphan scripts**: Every `.sh` file in `.claude/hooks/` is referenced by a hook entry in `settings.json`. Flag scripts that exist but are not wired up.
- [ ] **Reverse check**: Every `command` path in settings.json hooks points to a script that exists on disk.
- [ ] **Matcher correctness**: Verify each hook's `matcher` field targets the appropriate tool names for its purpose (e.g., `Edit|Write` for secret protection, `Bash` for git safety, `compact` for SessionStart).
- [ ] **Script logic**: Read each hook script and verify:
  - Reads from stdin (the JSON input from Claude Code)
  - Parses with `jq` using the correct field (`tool_input.file_path` for Edit/Write, `tool_input.command` for Bash)
  - Exit code semantics: 0 = allow, 2 = block
  - Patterns are comprehensive for their stated purpose
- [ ] **PostToolUse hooks**: Verify `cargo fmt` and `cargo clippy` commands use correct flags matching the project's CI expectations.
- [ ] **SessionStart hooks**: Verify the reminder message accurately reflects current Hard Rules in CLAUDE.md.
- [ ] **settings.local.json**: Note its contents. Flag any conflicts or duplications with shared settings.
- [ ] **All hook types**: Check all hook types found in settings files (PreToolUse, PostToolUse, SessionStart, Notification, etc.) — do not assume a fixed list.

### 6. Cross-File Consistency

- [ ] **Permissions coverage**: Check that `settings.json` `permissions.allow` covers the Bash commands developers commonly need. Cross-reference with commands referenced in CLAUDE.md (e.g., `cargo test`, `cargo check`, `cargo clippy`, `cargo fmt`, `cargo build`, git commands).
- [ ] **Hook-permission alignment**: Verify that destructive commands blocked by hooks are NOT in the allow list. Check that hook regex patterns cover all safety-critical operations mentioned in project rules.
- [ ] **Agent tool access**: Verify that tools listed in agent `tools:` fields are valid Claude Code tools.
- [ ] **Command-to-agent references**: If any command references an agent, verify the referenced agent file exists and the details (name, checkpoint count) match.
- [ ] **Rule cross-references**: Verify that rules referencing other rules or CLAUDE.md sections point to content that exists.

### 7. Best Practices (Advisory)

Use WebSearch to check for recent Claude Code configuration best practices. **Restrict searches to these domains only:**
- `site:code.claude.com CLAUDE.md` — structure and content guidance
- `site:code.claude.com hooks` — hook configuration patterns
- `site:code.claude.com commands` — custom command features
- `site:anthropic.com/engineering Claude Code` — engineering blog posts

Compare the project's configuration against documented recommendations. Flag deviations or new features the project could adopt.

**This section is advisory only.** Label all findings as informational, never FAIL. Include source URLs so the user can verify independently.

## Review Loop

After completing all 7 sections:

1. Launch 2 parallel subagents to cross-review the findings (read-only — subagents must not modify any files):
   - Agent 1 (False positives): Re-examine each reported issue independently. Is it a real problem or a misunderstanding of intent? Are the severity ratings appropriate?
   - Agent 2 (Blind spots): Re-scan all config files from scratch, ignoring the existing report. Are there issues the main review missed?
2. If the cross-review finds new issues or reclassifies existing ones, update the report and re-verify.
3. Repeat until no new issues. Hard limit: 3 iterations. Stop after 3 even if issues remain and report them as unresolved.
4. Report the final health check with iteration count.

## Output

Produce a single structured report:

```markdown
# Claude Code Configuration Health Check

**Date**: YYYY-MM-DD
**Project**: hamoru
**Reviewer**: Claude Code /review-config
**Review iterations**: N

## Summary

| Section | Status | Issues |
|---------|--------|--------|
| 1. CLAUDE.md | PASS/WARN/FAIL | count |
| 2. Rules | PASS/WARN/FAIL | count |
| 3. Commands | PASS/WARN/FAIL | count |
| 4. Agents | PASS/WARN/FAIL | count |
| 5. Hooks | PASS/WARN/FAIL | count |
| 6. Cross-file Consistency | PASS/WARN/FAIL | count |
| 7. Best Practices | ADVISORY | count |

## 1. CLAUDE.md
**Line count**: N lines (PASS / WARN: approaching limit)
...
(Detail each sub-check with specific findings)

## 2. Rules
...

## 3. Commands
...

## 4. Agents
...

## 5. Hooks
...

## 6. Cross-file Consistency
...

## 7. Best Practices (Advisory)
...

## Recommended Actions
(Prioritized list. Each item references the section where it was found.)
```

**Severity definitions:**
- **PASS**: No issues found.
- **WARN**: Minor issues or suggestions. No functional impact.
- **FAIL**: Broken, inconsistent, or could cause incorrect behavior.
- **ADVISORY**: Informational only (Best Practices section).
