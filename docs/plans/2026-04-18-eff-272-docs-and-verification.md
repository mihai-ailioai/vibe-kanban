# EFF-272 Docs and Verification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Update the user-facing docs and MCP docs so optional workspace modes are explained accurately, with `git_worktree` positioned as the recommended default, then run the final repository verification pass for the completed feature set.

**Architecture:** Keep documentation changes focused on the existing user journeys rather than adding a brand-new reference page. Update the core workspace overview, create, manage, repository, and git docs so they describe the three-mode model consistently and remove stale worktree-only assumptions. Update the MCP integration docs so `start_workspace` reflects the current API-backed request shape while preserving the repo-based MCP UX. Finish with a fresh verification pass across formatting, type generation checks, backend tests/checks, frontend checks, and lint.

**Tech Stack:** Mintlify MDX docs, root workspace scripts from `package.json`, Rust workspace tests/checks, TypeScript frontend checks, MCP integration docs.

---

### Task 1: Update the workspace overview and create-flow docs

**Files:**
- Modify: `docs/workspaces/index.mdx`
- Modify: `docs/workspaces/creating-workspaces.mdx`

**Step 1: Rewrite the overview page language so it no longer implies every workspace is a git worktree**

In `docs/workspaces/index.mdx`, replace the worktree-only explanation in:
- `## What is a Workspace?`
- the "What happens to my code when I create a workspace?" accordion

Document these points explicitly:
- `git_worktree` is the recommended default for most software work
- `in_place_git` is for advanced cases where you intentionally work inside existing repo checkouts
- `in_place_directory` is for non-Git folders such as Unity or other plain directories
- unsupported Git actions are hidden or unavailable in non-Git-capable modes

**Step 2: Update the create-workspace page to explain all three modes**

In `docs/workspaces/creating-workspaces.mdx`:
- replace the current "Git worktree is created" first-run explanation with a mode-aware summary
- add a new section after "What Happens When You Create a Workspace" that compares the three modes
- keep `git_worktree` as the recommended option
- explain what inputs differ by mode:
  - `git_worktree`: choose repo(s) and target branch(es)
  - `in_place_git`: choose existing repo(s), work in place, no repo copy
  - `in_place_directory`: choose one folder, no Git operations

**Step 3: Update the create steps and troubleshooting guidance**

In the same file, revise the numbered creation steps so they mention mode selection before repo/folder selection. Update troubleshooting text so failures are described in mode-appropriate terms instead of only "git worktree creation failed".

**Step 4: Verify no obvious worktree-only wording remains in these two files**

Run:

```bash
rg -n "git worktree|worktree" docs/workspaces/index.mdx docs/workspaces/creating-workspaces.mdx
```

Expected: only intentional mentions remain, not blanket statements that every workspace is a worktree.

---

### Task 2: Update management, repository, and Git-operation docs for mode-specific behaviour

**Files:**
- Modify: `docs/workspaces/managing-workspaces.mdx`
- Modify: `docs/workspaces/repositories.mdx`
- Modify: `docs/workspaces/git-operations.mdx`

**Step 1: Make deletion and storage docs mode-aware**

In `docs/workspaces/managing-workspaces.mdx`:
- update "What Gets Deleted" so it distinguishes between managed workspace roots, in-place repos, and plain directories
- state clearly that deleting `in_place_git` and `in_place_directory` workspaces does **not** delete the real repo/folder
- clarify that delete-branch is only relevant when the mode supports branch deletion
- revise disk-space language so it does not claim every workspace duplicates tracked files

**Step 2: Make the repositories page describe Git-backed modes only where appropriate**

In `docs/workspaces/repositories.mdx`:
- keep the repo-selection guidance for Git-backed workflows
- change the opening explanation so it says repositories are used by `git_worktree` and `in_place_git` modes
- add a note that `in_place_directory` uses a folder source instead of repository entries
- keep the worktree explanation, but label it as the `git_worktree` behaviour rather than the behaviour of every workspace

**Step 3: Scope Git actions to supported modes**

In `docs/workspaces/git-operations.mdx`:
- add a short note near the top that this page applies to Git-capable workspaces
- mention that plain-directory workspaces do not expose Git status, PR, push, merge, or rebase flows
- avoid implying every workspace has the Git panel/actions available

**Step 4: Verify the stale assumptions were removed from these pages**

Run:

```bash
rg -n "every workspace|Each workspace creates a \*\*git worktree\*\*|Deleting a workspace removes the worktree copy" docs/workspaces/managing-workspaces.mdx docs/workspaces/repositories.mdx docs/workspaces/git-operations.mdx
```

Expected: no remaining blanket statements that conflict with optional workspace modes.

---

### Task 3: Update MCP docs for the current workspace-start contract and examples

**Files:**
- Modify: `docs/integrations/vibe-kanban-mcp-server.mdx`
- Reference: `crates/mcp/src/task_server/tools/task_attempts.rs:95-119`

**Step 1: Document the public MCP UX accurately**

Read `build_create_and_start_workspace_payload(...)` in `crates/mcp/src/task_server/tools/task_attempts.rs:95-119` and keep the docs aligned with the real behaviour:
- `start_workspace` still accepts `repositories`
- under the hood, Vibe Kanban maps those repos into `workspace_mode: git_worktree` plus canonical Git sources before calling the local API

**Step 2: Update the Workspace Sessions section**

In `docs/integrations/vibe-kanban-mcp-server.mdx`:
- keep the tool table repo-based so the MCP user experience stays simple
- add a short explanatory note after the `start_workspace` parameter list saying the server converts repository inputs into the internal source-based workspace request
- avoid documenting stale `repos` payload wording if it appears anywhere in prose/examples

**Step 3: Update the example workflow text**

Revise the `start_workspace` examples so they remain user-facing and accurate:
- examples should still pass `repositories`
- prose should describe the result as creating a workspace in the default recommended `git_worktree` mode unless/until MCP grows explicit mode selection

**Step 4: Verify the MCP docs do not promise the wrong request shape**

Run:

```bash
rg -n "workspace_mode|sources|repos" docs/integrations/vibe-kanban-mcp-server.mdx
```

Expected: any mention of internal request-shape details is deliberate, accurate, and consistent with the current MCP implementation.

---

### Task 4: Clean up directly related stale references elsewhere in docs

**Files:**
- Modify as needed: `docs/workspaces/index.mdx`
- Modify as needed: `docs/workspaces/creating-workspaces.mdx`
- Modify as needed: `docs/workspaces/managing-workspaces.mdx`
- Modify as needed: `docs/workspaces/repositories.mdx`
- Modify as needed: `docs/workspaces/git-operations.mdx`
- Modify as needed: any additional docs file surfaced by the grep below that still makes a direct, user-facing worktree-only claim about all workspaces

**Step 1: Run a docs-only stale-reference search**

Run:

```bash
rg -n "git worktree|worktree|in-place Git|plain directory|workspace mode|workspace modes" docs --glob '!docs/plans/**'
```

**Step 2: Triage the results**

Only edit files that are both:
- user-facing documentation
- directly wrong or misleading after optional workspace modes landed

Do **not** churn unrelated docs just because they mention worktrees in a legitimate, scoped context.

**Step 3: Make the smallest fixes needed**

Examples of acceptable cleanups:
- adding "in `git_worktree` mode" where a statement is currently too broad
- adding a short note that `in_place_directory` does not expose Git actions
- fixing delete/disk-space language so it matches the real cleanup model

**Step 4: Re-run the search and confirm only intentional references remain**

Run the same command again and inspect the remaining hits.

Expected: remaining matches are either scoped correctly, belong to plan docs, or are legitimate feature explanations.

---

### Task 5: Run final verification for EFF-272

**Files:**
- No file edits expected in this task unless a verification failure exposes a missed docs/code mismatch

**Step 1: Format the repo**

Run:

```bash
pnpm run format
```

Expected: exit code 0.

**Step 2: Verify generated shared types are up to date**

Run:

```bash
pnpm run generate-types:check
```

Expected: exit code 0.

**Step 3: Run backend tests**

Run:

```bash
cargo test --workspace
```

Expected: workspace tests pass.

**Step 4: Run full project checks**

Run:

```bash
pnpm run check
```

Expected: local-web, remote-web, web-core, ui, and backend checks all pass.

**Step 5: Run lint**

Run:

```bash
pnpm run lint
```

Expected: frontend lint, backend clippy, and i18n-key checks all pass.

**Step 6: Update the ticket journal with the documentation and verification outcome**

Append a concise changelog entry to `EFF-272` covering:
- which docs were updated
- how the three modes are framed (`git_worktree` recommended default)
- whether any direct stale-reference cleanup was needed
- the exact verification commands that passed

---

### Task 6: Commit the documentation and verification pass

**Files:**
- Commit only the docs and any directly related generated-file changes if verification requires them

**Step 1: Inspect the final diff**

Run:

```bash
git status --short
git diff -- docs docs.json shared/types.ts crates/mcp/src/task_server/tools/task_attempts.rs
```

Expected: only the intended docs/verification-related changes are present.

**Step 2: Commit with a docs-focused message**

Suggested commit:

```bash
git commit -m "docs: document optional workspace modes"
```

**Step 3: Mark EFF-272 done only after verification and commit are complete**

Append a final issue note summarising the completed docs and verification pass, then move `EFF-272` to `Done`.
