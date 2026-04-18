# EFF-273 Workspace Capability Gating Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Gate Git, diff, PR, repo-attach, and cleanup flows by workspace capabilities so `git_worktree` and `in_place_git` keep working where intended, `in_place_directory` fails explicitly, and in-place cleanup stays non-destructive.

**Architecture:** Add one small capability matrix derived from `workspace.workspace_mode`, then use it at the route and service boundaries instead of letting worktree assumptions leak everywhere. Keep `in_place_git` on the existing Git-backed downstream paths added in EFF-271, block only unsupported capabilities, and route deletion cleanup by mode so in-place modes never delete real repo contents or branches as part of normal cleanup. Expose a dedicated capabilities endpoint now so EFF-275 can consume a server-owned contract without reshaping the existing `Workspace` payload.

**Tech Stack:** Rust, Axum, SQLx, ts-rs type generation, local deployment container service, workspace manager, shared web client types.

---

### Task 1: Introduce a central workspace capability matrix and API contract

**Files:**
- Create: `crates/server/src/routes/workspaces/capabilities.rs`
- Modify: `crates/server/src/routes/workspaces/mod.rs`
- Modify: `crates/server/src/bin/generate_types.rs`

**Step 1: Write failing capability-matrix tests**

In `crates/server/src/routes/workspaces/capabilities.rs`, add focused unit tests for a new helper such as `WorkspaceCapabilities::for_mode(...)`.

Use a compact contract like:

```rust
#[derive(Debug, Clone, Serialize, TS, PartialEq, Eq)]
pub struct WorkspaceCapabilities {
    pub supports_git_read: bool,
    pub supports_git_write: bool,
    pub supports_pull_requests: bool,
    pub supports_repo_attach: bool,
    pub supports_delete_branches: bool,
}
```

Expected matrix:

```rust
WorkspaceMode::GitWorktree => WorkspaceCapabilities {
    supports_git_read: true,
    supports_git_write: true,
    supports_pull_requests: true,
    supports_repo_attach: true,
    supports_delete_branches: true,
}

WorkspaceMode::InPlaceGit => WorkspaceCapabilities {
    supports_git_read: true,
    supports_git_write: true,
    supports_pull_requests: true,
    supports_repo_attach: false,
    supports_delete_branches: false,
}

WorkspaceMode::InPlaceDirectory => WorkspaceCapabilities {
    supports_git_read: false,
    supports_git_write: false,
    supports_pull_requests: false,
    supports_repo_attach: false,
    supports_delete_branches: false,
}
```

Also add one failing handler test for `GET /api/workspaces/{id}/capabilities` that seeds an `in_place_git` workspace and asserts the response matches the matrix above.

**Step 2: Run the server tests to confirm failure**

Run: `cargo test -p server --lib`

Expected: FAIL because the capabilities module and endpoint do not exist yet.

**Step 3: Implement the capability helper and route guard helpers**

In `capabilities.rs`, add:

```rust
pub fn workspace_capabilities(workspace: &Workspace) -> WorkspaceCapabilities

pub fn require_git_read(workspace: &Workspace) -> Result<(), ApiError>
pub fn require_git_write(workspace: &Workspace) -> Result<(), ApiError>
pub fn require_pull_requests(workspace: &Workspace) -> Result<(), ApiError>
pub fn require_repo_attach(workspace: &Workspace) -> Result<(), ApiError>
```

Make the error messages explicit and mode-aware, for example:

```rust
ApiError::BadRequest(
    format!(
        "Workspace mode `{}` does not support pull request operations.",
        workspace.workspace_mode
    )
)
```

Do **not** add these fields to the persisted `Workspace` database model. Keep this as a route-owned API contract so EFF-275 can fetch it without changing every existing workspace payload in one shot.

**Step 4: Expose the new endpoint and generated type**

Add `GET /api/workspaces/{id}/capabilities` under the existing workspace-id router in `mod.rs`, and add `WorkspaceCapabilities::decl()` to `crates/server/src/bin/generate_types.rs`.

**Step 5: Re-run the server tests**

Run: `cargo test -p server --lib`

Expected: PASS for the new capability-matrix and endpoint coverage.

**Step 6: Commit the contract slice**

```bash
git add crates/server/src/routes/workspaces/capabilities.rs crates/server/src/routes/workspaces/mod.rs crates/server/src/bin/generate_types.rs
git commit -m "feat: add workspace capability contract"
```

### Task 2: Gate Git, PR, and repo-attach routes by capability instead of worktree assumptions

**Files:**
- Modify: `crates/server/src/routes/workspaces/git.rs`
- Modify: `crates/server/src/routes/workspaces/pr.rs`
- Modify: `crates/server/src/routes/workspaces/repos.rs`
- Modify: `crates/server/src/routes/workspaces/streams.rs`

**Step 1: Add failing route tests for unsupported modes**

Add route tests that create an `in_place_directory` workspace and assert:

- `GET /git/status` returns `ApiError::BadRequest` mentioning `in_place_directory` and `git`
- `POST /pull-requests` returns `ApiError::BadRequest` mentioning `pull request`

Add one test that creates an `in_place_git` workspace and asserts:

- `POST /repos` returns `ApiError::BadRequest` because repo attach must remain `git_worktree`-only

Add one test for the diff websocket route that checks the handler refuses unsupported modes **before** opening the websocket upgrade path.

**Step 2: Run the targeted server tests to confirm failure**

Run: `cargo test -p server --lib`

Expected: FAIL because the current handlers go straight into `WorkspaceRepo`, `ensure_container_exists(...)`, or PR/Git operations without any capability check.

**Step 3: Guard the Git route families up front**

Apply the new helpers before any repo lookup or container work:

- `git/status` and `git/diff/ws` → `require_git_read(...)`
- `git/merge`, `git/push`, `git/push/force`, `git/rebase`, `git/rebase/continue`, `git/conflicts/abort`, `git/target-branch`, `git/branch` → `require_git_write(...)`
- `pull-requests`, `pull-requests/attach`, `pull-requests/comments` → `require_pull_requests(...)`
- `repos` `POST` only → `require_repo_attach(...)`

Leave `GET /repos` alone so existing repo listings continue to work for `git_worktree` and `in_place_git`, and future `in_place_directory` work can safely return an empty list.

**Step 4: Make the diff websocket route reject unsupported modes before upgrade**

Refactor `stream_workspace_diff_ws(...)` and `stream_diff_ws(...)` to return an error response for unsupported modes before calling `ws.on_upgrade(...)`.

Use a signature like:

```rust
pub async fn stream_workspace_diff_ws(...) -> Result<impl IntoResponse, ApiError>
```

or return a concrete `axum::response::Response` if that is easier for the websocket path. The important rule is: do not accept the websocket and then fail deep inside the stream.

**Step 5: Re-run the server tests**

Run: `cargo test -p server --lib`

Expected: PASS, with explicit unsupported-mode responses instead of repo-not-found or generic container/git failures.

**Step 6: Commit the route gating slice**

```bash
git add crates/server/src/routes/workspaces/git.rs crates/server/src/routes/workspaces/pr.rs crates/server/src/routes/workspaces/repos.rs crates/server/src/routes/workspaces/streams.rs
git commit -m "fix: gate workspace git routes by capability"
```

### Task 3: Make diff stats and summary callers capability-safe

**Files:**
- Modify: `crates/services/src/services/diff_stream.rs`
- Modify: `crates/server/src/routes/workspaces/workspace_summary.rs`
- Modify: `crates/server/src/routes/workspaces/core.rs`

**Step 1: Add failing tests for non-Git summary behaviour**

Add tests that seed an `in_place_directory` workspace and verify:

- `compute_diff_stats(...)` returns `None` immediately
- `get_workspace_summaries(...)` leaves `files_changed`, `lines_added`, and `lines_removed` as `None` instead of trying to compute Git diffs

**Step 2: Run the relevant tests to confirm failure**

Run: `cargo test -p services --lib && cargo test -p server --lib`

Expected: FAIL because `compute_diff_stats(...)` still walks `WorkspaceRepo` and repo paths for every workspace with a `container_ref`.

**Step 3: Add a service-level early return for non-Git read modes**

In `diff_stream.rs`, short-circuit `compute_diff_stats(...)` for workspaces that do not support Git read operations:

```rust
if matches!(workspace.workspace_mode, WorkspaceMode::InPlaceDirectory) {
    return None;
}
```

If you want to keep the logic DRY, add a tiny shared helper in the server capability module and mirror the same boolean in services with a local helper rather than introducing a circular dependency.

**Step 4: Stop summary and remote-sync callers from assuming diff support**

In `workspace_summary.rs`, skip scheduling diff work for workspaces without Git read support.

In `core.rs`, when syncing an updated workspace to remote after archive/name changes, only compute diff stats for workspaces whose mode supports Git read.

This keeps EFF-274 from immediately inheriting noisy best-effort diff failures.

**Step 5: Re-run the tests**

Run: `cargo test -p services --lib && cargo test -p server --lib`

Expected: PASS, with non-Git modes returning `None`/empty diff metadata cleanly.

**Step 6: Commit the diff-safety slice**

```bash
git add crates/services/src/services/diff_stream.rs crates/server/src/routes/workspaces/workspace_summary.rs crates/server/src/routes/workspaces/core.rs
git commit -m "fix: make workspace diff stats mode aware"
```

### Task 4: Make workspace deletion cleanup mode-aware and non-destructive

**Files:**
- Modify: `crates/server/src/routes/workspaces/core.rs`
- Modify: `crates/workspace-manager/src/workspace_manager.rs`
- Modify: `crates/local-deployment/src/container.rs`

**Step 1: Add failing cleanup tests for in-place modes**

Add tests that prove two things:

1. deleting an `in_place_git` workspace removes only the synthetic workspace root and leaves the real repo directory intact
2. `delete_branches=true` does **not** delete the workspace branch for `in_place_git`

Add one server-side test showing that deleting a workspace with only a running dev server still releases claims by going through the container stop hook rather than bypassing it.

**Step 2: Run the workspace-manager, local-deployment, and server tests to confirm failure**

Run: `cargo test -p local-deployment --lib && cargo test -p server --lib`

Expected: FAIL because `delete_workspace(...)` still hands cleanup to `WorkspaceManager::spawn_workspace_deletion_cleanup(...)`, which is worktree-oriented and still honours `delete_branches` blindly.

**Step 3: Replace the worktree-only deletion assumptions with mode-aware cleanup**

Do all of the following:

- add `workspace_mode: WorkspaceMode` to `WorkspaceDeletionContext`
- replace the current unconditional cleanup branch in `spawn_workspace_deletion_cleanup(...)` with a mode match
- keep the existing `git_worktree` cleanup path as-is
- for `in_place_git`, remove only the synthetic workspace root and skip branch deletion entirely
- for `in_place_directory`, also remove only the synthetic workspace root and never touch the source directory

The easiest way to keep this DRY is to extract a generic safe helper out of `cleanup_in_place_git_workspace_root(...)` so both `local-deployment` and `workspace-manager` can use the same “workspace-base-dir only” deletion guard.

**Step 4: Route delete-time stopping through the container stop hook**

In `core.rs`, stop bypassing `after_workspace_stopped(...)` when only dev servers are running. After the existing guard that rejects running non-dev-server processes, call:

```rust
deployment.container().try_stop(&workspace, true).await;
```

instead of manually iterating dev servers with `stop_execution(...)`.

This ensures `in_place_git` claim release still happens through the EFF-271 stop hook before deletion proceeds.

**Step 5: Re-run the tests**

Run: `cargo test -p local-deployment --lib && cargo test -p server --lib`

Expected: PASS, with in-place cleanup staying non-destructive and delete-time stop flows releasing claims correctly.

**Step 6: Commit the cleanup slice**

```bash
git add crates/server/src/routes/workspaces/core.rs crates/workspace-manager/src/workspace_manager.rs crates/local-deployment/src/container.rs
git commit -m "fix: make workspace deletion cleanup mode aware"
```

### Task 5: Regenerate shared types and verify the whole EFF-273 slice

**Files:**
- Modify: `shared/types.ts` (generated)
- Modify: any generated schema files touched by `pnpm run generate-types`

**Step 1: Regenerate generated types**

Run: `pnpm run generate-types`

Expected: `shared/types.ts` includes the new `WorkspaceCapabilities` type and any endpoint-adjacent generated declarations.

**Step 2: Run formatting**

Run: `pnpm run format`

Expected: Rust and web formatting complete without changes left behind.

**Step 3: Run focused backend verification**

Run:

```bash
cargo test -p server --lib
cargo test -p services --lib
cargo test -p local-deployment --lib
```

Expected: PASS.

**Step 4: Run repo-wide verification required for completion**

Run:

```bash
pnpm run check
```

Expected: PASS.

**Step 5: Update the issue journal and commit**

Before implementation starts, append a short note to `EFF-273` describing the chosen approach: dedicated capability endpoint, explicit route gating, and non-destructive in-place cleanup.

After verification passes, append the completed verification commands and a summary of the landed capability matrix.

Then commit the generated types plus any final clean-up changes:

```bash
git add crates/server/src/routes/workspaces/capabilities.rs crates/server/src/routes/workspaces/mod.rs crates/server/src/routes/workspaces/git.rs crates/server/src/routes/workspaces/pr.rs crates/server/src/routes/workspaces/repos.rs crates/server/src/routes/workspaces/streams.rs crates/server/src/routes/workspaces/workspace_summary.rs crates/server/src/routes/workspaces/core.rs crates/workspace-manager/src/workspace_manager.rs crates/local-deployment/src/container.rs crates/services/src/services/diff_stream.rs crates/server/src/bin/generate_types.rs shared/types.ts
git commit -m "fix: gate workspace operations by mode capabilities"
```
