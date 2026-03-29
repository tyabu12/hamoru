---
description: Orchestrate feature implementation from plan to PR — worktree isolation, TDD, review, and PR creation.
allowed-tools: Read, Grep, Glob, Bash, Agent, Write, Edit, EnterWorktree, ExitWorktree
argument-hint: "[description | issue-number | phase N]"
---

# /implement

Orchestrate the full development workflow: plan → issue → worktree → TDD implementation → review → PR.

## Constants

- `PLAN_MARKER`: `<!-- hamoru-plan -->` — machine-readable marker embedded in Issue plan comments for detection during resumption.
- `OWNER_REPO`: derived at runtime via `gh repo view --json nameWithOwner -q '.nameWithOwner'`.

## Step 0: Input Detection & Pre-flight

Interpret `$ARGUMENTS`:
- **`#N`** (digits after `#`): Fetch issue via `gh issue view N`, use title/body as task spec. Then check for an existing plan (see **Resumption Detection** below).
- **`phase N`** (e.g., `phase 3`, `phase 4a`): Read ONLY that Phase section from `docs/design-plan.md`.
- **(empty)**: Ask user what to implement.
- **Other text**: Use as inline task description. If user already ran `/phase-plan`, reuse that plan directly — do not re-derive from design-plan.md.

Derive from the task spec:
- `TASK_TYPE`: `feat` or `fix` (infer from content, default `feat`)
- `SLUG`: kebab-case, **must match `^[a-z0-9][a-z0-9-]{0,36}$`**. If not, sanitize or ask user.

### Resumption Detection (for `#N` input only)

After fetching the issue, check for an existing plan comment:
1. Fetch issue comments and search for `PLAN_MARKER`:
   ```bash
   gh api "repos/${OWNER_REPO}/issues/N/comments" --jq '.[] | select(.body | contains("<!-- hamoru-plan -->")) | {id, body}' | tail -1
   ```
   Use the **last** matching comment (handles multiple plan comments from retries).
2. If a plan comment is found:
   - Set `RESUMING=true` and capture `COMMENT_ID`.
   - Parse checkboxes: count `- [x]` (done) vs `- [ ]` (remaining). Identify `NEXT_ITEM` (first unchecked item number).
   - Extract `TASK_TYPE` and branch name from the `## Metadata` section in the comment.
   - Derive `SLUG` from the branch name.
   - Report to user: "Found existing plan on issue #N. {DONE}/{TOTAL} items complete. Resuming from item {NEXT_ITEM}."
   - **Skip Step 1 entirely** → proceed to Step 2.
3. If no plan comment found: proceed normally (Step 1 creates the plan, Step 2 attaches it).

**Pre-flight checks** (run in order):
1. `gh auth status` — warn and skip GitHub steps if unauthenticated.
2. `git status` — warn if uncommitted changes exist.
3. Verify on default branch (skip if `RESUMING=true` — resumption may start from a worktree or feature branch):
   - `DEFAULT_BRANCH=$(gh repo view --json defaultBranchRef -q '.defaultBranchRef.name')`
   - If current branch != `DEFAULT_BRANCH`, warn and offer `git checkout "$DEFAULT_BRANCH"`.
4. `git pull --ff-only origin "$DEFAULT_BRANCH"` — warn on failure, don't block. Skip if `RESUMING=true`.
5. If already in a worktree, warn and suggest `ExitWorktree` first (unless `RESUMING=true` and the worktree matches the expected branch).

## Step 1: Plan — Gate G1

1. Read `CLAUDE.md` for current phase and conventions.
2. If phase-related, read ONLY the relevant Phase section from `docs/design-plan.md`.
3. Format the plan as a numbered checkbox list (each item = one planned commit):
   ```
   - [ ] 1. <description> (`<primary-file-path>`)
   - [ ] 2. <description> (`<primary-file-path>`)
   ...
   ```
   Present this plan to the user. Store internally as `PLAN_BODY` for Issue attachment in Step 2.
4. **Ask: "Proceed with this plan?"** — For single-commit fixes, combine G1 and G2 into one confirmation.

## Step 2: Issue + Worktree — Gate G2

1. If from `#N`, skip issue creation. Otherwise ask: "Create a GitHub Issue, or proceed without?"
2. Display branch name: `<TASK_TYPE>/<SLUG>`.
3. **Ask: "Create worktree and start?"**
4. Call `EnterWorktree` with `name: "<TASK_TYPE>/<SLUG>"`.
   - On failure (name collision, etc.): suggest alternative name or cleanup. Check `git ls-remote --heads origin <branch>` for remote collisions too; append `-2` suffix if needed (re-validate SLUG).
5. Verify: `git branch --show-current`.

## Step 3: Implementation (TDD)

Follow the plan from Step 1. For each unit of work:
1. Write test first (TDD mandatory per CLAUDE.md Phase 1+).
2. `cargo test` — confirm failure.
3. Write implementation.
4. `cargo test` — confirm pass.
5. Commit (Conventional Commits + emoji per CLAUDE.md).

Note: `git commit` is NOT in the permissions allowlist — each commit triggers user approval (intentional security gate). Suggest batching into fewer commits if the user finds this disruptive.

PostToolUse hooks (`cargo fmt`, `cargo clippy`) fire automatically on every Write/Edit inside the worktree.

After all implementation, run full verification:
```bash
cargo test --all-targets && cargo clippy --all-targets -- -D warnings && cargo fmt --all --check
```
Fix any failures before proceeding.

## Step 4: Review — Gate G3

Run the evaluator agent (`.claude/agents/evaluator.md`) against changed files via `Agent` tool.

**If this is a Phase completion** (task was `phase N`): run `/review-phase N` before proceeding to PR. This catches doc updates (CLAUDE.md, README.md, design-plan.md checkboxes, ADR table) and spec fidelity issues.

**Cross-review loop:**
1. Launch 2 parallel read-only subagents:
   - Agent 1: Verify PASS results — check for false negatives.
   - Agent 2: Verify FAIL results — check for false positives.
2. If issues found, fix and re-verify. Hard limit: 3 iterations.

Show consolidated results. **Ask: "Create PR?"**
- If unresolved after 3 iterations, report issues and let user decide.

## Step 5: PR Creation — Gate G4

Derive base branch: `gh repo view --json defaultBranchRef -q '.defaultBranchRef.name'`

Determine label from the commit prefix (TASK_TYPE or dominant commit type):

| Commit prefix | Label |
|---------------|-------|
| `feat` | `enhancement` |
| `fix` | `bug` |
| `docs` | `documentation` |
| `refactor` | `refactor` |
| `test` | `testing` |
| `chore` | `chore` |
| `ci` | `ci` |
| `perf` | `performance` |

Additionally, if the changes are security-related (dependencies, auth, secrets, hooks, hardening), add the `security` label alongside the prefix-based label.

Present PR draft (title + body + label) for user review:
- Title: Emoji prefix + Conventional format, under 70 chars (same emoji convention as CLAUDE.md commits)
- Body: Summary bullets + test plan + `Closes #N` if applicable
- Label: from the table above
- Assignee: always `@me`

**Ask: "Create this PR?"**

Use HEREDOC with distinctive delimiter:
```bash
gh pr create --base "$BASE_BRANCH" --assignee "@me" --label "$LABEL" \
  --title "..." --body "$(cat <<'IMPLEMENT_PR_BODY'
## Summary
...
## Test plan
...
🤖 Generated with [Claude Code](https://claude.com/claude-code)
IMPLEMENT_PR_BODY
)"
```

Push the branch first: `git push -u origin <branch>`. Then create the PR.

After creation:
- Print the PR URL.
- "Wait for all required status checks to pass, then **merge manually**. Auto-merge is disabled."

## Step 6: Cleanup & Abandonment

**After merge** (guidance only — do NOT auto-execute):
1. `ExitWorktree` with action `"remove"`
2. `git checkout <default-branch> && git pull`
3. Remote branch: GitHub may auto-delete; if not, `git push origin --delete <branch>`

**To abandon** (no PR, or after PR created but want to cancel):
1. If PR exists: `gh pr close <number>`
2. `ExitWorktree` with action `"remove"`
3. If already pushed: `git push origin --delete <branch>`
