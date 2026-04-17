# EFF-271 In-Place Git Workspace Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement real `in_place_git` workspaces that use the actual repo checkouts behind a synthetic workspace root, enforce exclusive runtime repo ownership, reject dirty repos, and keep cleanup non-destructive.

**Architecture:** Build on the EFF-270 mode-dispatch seam in `crates/local-deployment/src/container.rs`. Add DB-backed runtime repo claims plus Git-backed provisioning that validates all repos up front, creates or checks out the workspace branch inside the real repos, and exposes those repos under the workspace root via symlinks. Keep existing downstream git-backed flows working by syncing `workspace_repos` as a compatibility bridge for `in_place_git` after provisioning succeeds.

**Tech Stack:** Rust, SQLx/SQLite migrations, local deployment container service, GitService/GitCli, workspace DB models, Axum/server error handling.

---

### Task 1: Add DB-backed runtime repo claims for in-place Git ownership

**Files:**
- Create: `crates/db/migrations/20260417010000_add_workspace_repo_claims.sql`
- Create: `crates/db/src/models/workspace_repo_claim.rs`
- Modify: `crates/db/src/models/mod.rs`

**Step 1: Write the failing DB model tests**

In `crates/db/src/models/workspace_repo_claim.rs`, add a `#[cfg(test)]` module with tests that prove:

```rust
#[test]
fn create_many_rejects_duplicate_repo_claims() { /* unique repo ownership */ }

#[test]
fn release_for_workspace_removes_all_claims() { /* stop/delete cleanup */ }

#[test]
fn find_conflicting_repo_ids_excludes_same_workspace() { /* restart/reuse safety */ }
```

Use temporary SQLite DBs and two workspaces plus one or more repos so the tests assert the claim contract directly.

**Step 2: Run the DB test target to confirm failure**

Run: `cargo test -p db workspace_repo_claim --lib`

Expected: FAIL because the claim model and migration do not exist yet.

**Step 3: Add the migration**

Create `crates/db/migrations/20260417010000_add_workspace_repo_claims.sql` with a table like:

```sql
CREATE TABLE workspace_repo_claims (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
  FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE,
  UNIQUE (repo_id)
);

CREATE INDEX idx_workspace_repo_claims_workspace_id ON workspace_repo_claims(workspace_id);
```

The uniqueness guarantee should enforce repo-exclusive ownership.

**Step 4: Implement the claim model**

Add `crates/db/src/models/workspace_repo_claim.rs` with structs along these lines:

```rust
pub struct WorkspaceRepoClaim {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub repo_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct CreateWorkspaceRepoClaim {
    pub repo_id: Uuid,
}
```

and helpers:

```rust
create_many(pool, workspace_id, claims)
find_by_workspace_id(pool, workspace_id)
find_conflicting_repo_ids(pool, workspace_id, repo_ids)
release_for_workspace(pool, workspace_id)
```

Keep conflict lookup workspace-aware so the same workspace can reopen its own repos without false positives.

**Step 5: Refresh SQLx metadata**

Run: `pnpm run prepare-db`

Expected: PASS with the new migration and query metadata recorded.

**Step 6: Re-run the DB tests**

Run: `cargo test -p db workspace_repo_claim --lib`

Expected: PASS for the new claim model tests.

**Step 7: Commit the schema slice**

```bash
git add crates/db/migrations/20260417010000_add_workspace_repo_claims.sql crates/db/src/models/workspace_repo_claim.rs crates/db/src/models/mod.rs .sqlx
git commit -m "feat: add workspace repo claims"
```

### Task 2: Add Git and compatibility helpers for in-place Git workspaces

**Files:**
- Modify: `crates/git/src/lib.rs`
- Modify: `crates/git/src/cli.rs`
- Modify: `crates/db/src/models/workspace_repo.rs`

**Step 1: Write the failing Git helper tests**

Add tests in `crates/git/tests/git_workflow.rs` (or the most relevant existing Git test file) that prove:

```rust
#[test]
fn ensure_branch_checked_out_reuses_existing_workspace_branch() { /* checkout existing */ }

#[test]
fn ensure_branch_checked_out_creates_workspace_branch_from_target_branch() { /* create from base */ }

#[test]
fn strict_dirty_check_counts_untracked_files_as_dirty() { /* modified + staged + untracked */ }
```

The dirty test must explicitly create an untracked file and assert the helper rejects the repo.

**Step 2: Run the Git test target to confirm failure**

Run: `cargo test -p git git_workflow --test git_workflow`

Expected: FAIL because the in-place Git checkout/dirty helpers do not exist yet.

**Step 3: Implement minimal Git helpers**

In `crates/git/src/lib.rs` and/or `crates/git/src/cli.rs`, add helpers such as:

```rust
pub fn ensure_local_branch_checked_out(
    &self,
    repo_path: &Path,
    workspace_branch: &str,
    target_branch: &str,
) -> Result<(), GitServiceError>

pub fn ensure_repo_clean_including_untracked(
    &self,
    repo_path: &Path,
) -> Result<(), GitServiceError>
```

Behavior:

- fail if `target_branch` is missing locally
- if `workspace_branch` exists locally, checkout it
- otherwise create `workspace_branch` from `target_branch` and checkout it
- reject any modified, staged, or untracked file

Use Git CLI for checkout operations if that is safer with the current repository semantics.

**Step 4: Add `workspace_repos` compatibility sync helpers**

In `crates/db/src/models/workspace_repo.rs`, add helpers to keep git-backed downstream flows working for `in_place_git`, for example:

```rust
sync_for_workspace(pool, workspace_id, repos: &[CreateWorkspaceRepo])
delete_by_workspace_id(pool, workspace_id)
```

`sync_for_workspace(...)` should upsert repo/target-branch rows for the workspace after in-place Git preflight succeeds.

**Step 5: Re-run the Git tests**

Run: `cargo test -p git git_workflow --test git_workflow`

Expected: PASS for the new checkout and strict-dirty tests.

**Step 6: Commit the helper layer**

```bash
git add crates/git/src/lib.rs crates/git/src/cli.rs crates/git/tests/git_workflow.rs crates/db/src/models/workspace_repo.rs
git commit -m "feat: add in-place git repo helpers"
```

### Task 3: Implement in-place Git provisioning with synthetic-root symlinks

**Files:**
- Modify: `crates/local-deployment/src/container.rs`

**Step 1: Write the failing local-deployment tests**

Extend the `#[cfg(test)]` module in `crates/local-deployment/src/container.rs` with focused tests that prove:

```rust
#[test]
fn provision_workspace_for_mode_rejects_dirty_in_place_git_repo() { /* includes untracked */ }

#[test]
fn provision_workspace_for_mode_rejects_missing_local_target_branch() { /* fail fast */ }

#[test]
fn provision_workspace_for_mode_creates_symlinked_in_place_git_workspace_root() { /* root/repo symlink */ }

#[test]
fn provision_workspace_for_mode_fails_when_repo_entry_path_is_real_directory() { /* fail fast */ }

#[test]
fn provision_workspace_for_mode_is_all_or_nothing_for_multiple_repos() { /* one bad repo blocks all */ }

#[test]
fn ensure_container_exists_recreates_missing_repo_symlink() { /* repair missing link */ }
```

Use real temporary repos and temporary workspace roots. The symlink tests should assert `std::fs::symlink_metadata(...).file_type().is_symlink()`.

**Step 2: Run the local-deployment tests to confirm failure**

Run: `cargo test -p local-deployment --lib`

Expected: FAIL because the `WorkspaceMode::InPlaceGit` branch still returns `UnsupportedWorkspaceMode`.

**Step 3: Add in-place Git preflight and provisioning helpers**

In `crates/local-deployment/src/container.rs`, add helpers along these lines:

```rust
async fn in_place_git_context(...)
async fn validate_in_place_git_repos(...)
async fn acquire_in_place_git_claims(...)
async fn ensure_workspace_root_symlinks(...)
```

The preflight must:

- resolve only `WorkspaceSourceInput::GitRepo` sources
- resolve backing repos and target branches
- validate target branches locally
- validate strict cleanliness including untracked files
- validate claim conflicts as one batch

**Step 4: Implement the real `WorkspaceMode::InPlaceGit` branch**

Replace the placeholder arm in `provision_workspace_for_mode(...)` with logic that:

- validates all repos first
- acquires claims as a batch
- syncs `workspace_repos` for compatibility
- creates the synthetic workspace root directory
- creates/repairs repo symlinks under `<workspace_root>/<repo.name>`
- checks out or creates the workspace branch in each real repo
- returns the synthetic workspace root path as `container_ref`

If `<workspace_root>/<repo.name>` exists and is not a symlink, fail with a clear error.

**Step 5: Handle rollback on provisioning failure**

If any mutation step fails after claims are acquired, release the new claims before returning the error. The rollback does **not** need to restore prior branches.

**Step 6: Re-run the local-deployment tests**

Run: `cargo test -p local-deployment --lib`

Expected: PASS for the new in-place Git provisioning coverage while preserving existing git-worktree tests.

**Step 7: Commit the provisioning slice**

```bash
git add crates/local-deployment/src/container.rs
git commit -m "feat: provision in-place git workspaces"
```

### Task 4: Release claims on stop, failure, and teardown

**Files:**
- Modify: `crates/services/src/services/container.rs`
- Modify: `crates/local-deployment/src/container.rs`
- Modify: `crates/server/src/routes/workspaces/create.rs`

**Step 1: Write the failing lifecycle tests**

Add targeted tests that prove:

```rust
#[test]
fn in_place_git_claims_release_when_workspace_stop_runs() { /* active claim freed */ }

#[test]
fn in_place_git_claims_release_when_create_start_fails_after_claim_acquisition() { /* no stale claims */ }

#[test]
fn cleanup_workspace_for_in_place_git_removes_symlinks_but_not_real_repo_dirs() { /* non-destructive */ }
```

Put the stop/cleanup tests in the narrowest existing module that can exercise them without adding broad integration scaffolding.

**Step 2: Run the targeted tests to confirm failure**

Run: `cargo test -p local-deployment --lib && cargo test -p server create::tests --lib`

Expected: FAIL because claims are not yet released on stop/failure and cleanup still assumes worktree directories.

**Step 3: Add explicit release hooks**

Update the stop/cleanup flow so `in_place_git` claims are released when the workspace stops or teardown runs. If needed, add a small hook in `crates/services/src/services/container.rs` that local deployment can implement after `try_stop(...)` finishes, for example:

```rust
async fn after_workspace_stopped(&self, workspace: &Workspace) -> Result<(), ContainerError> {
    Ok(())
}
```

and have local deployment release claims there for `WorkspaceMode::InPlaceGit`.

**Step 4: Make cleanup non-destructive for symlinked repos**

Update `cleanup_workspace(...)` in `crates/local-deployment/src/container.rs` so the `in_place_git` branch removes only the synthetic workspace root and symlinks, never the real repo directories. Preserve `git_worktree` cleanup behavior as-is.

**Step 5: Ensure failed create/start paths release claims**

Use the existing failure handling in `crates/server/src/routes/workspaces/create.rs` plus local deployment rollback helpers so a failed start cannot leave stale claims behind.

**Step 6: Re-run the lifecycle verification**

Run: `cargo test -p local-deployment --lib && cargo test -p server create::tests --lib`

Expected: PASS with claim release and non-destructive cleanup locked down.

**Step 7: Commit the lifecycle slice**

```bash
git add crates/services/src/services/container.rs crates/local-deployment/src/container.rs crates/server/src/routes/workspaces/create.rs
git commit -m "fix: release in-place git repo claims on stop"
```

### Task 5: Final verification and issue updates

**Files:**
- Modify: `docs/plans/2026-04-17-eff-271-in-place-git-design.md` (only if implementation requires design note updates)

**Step 1: Format the code**

Run: `pnpm run format`

Expected: PASS with Rust and web formatting complete.

**Step 2: Run targeted verification**

Run: `cargo test -p db workspace_repo_claim --lib && cargo test -p git git_workflow --test git_workflow && cargo test -p local-deployment --lib && cargo test -p server create::tests --lib`

Expected: PASS for the new schema, Git helpers, provisioning path, and create/start cleanup coverage.

**Step 3: Run the repo-wide gate**

Run: `pnpm run check`

Expected: PASS so the new in-place Git path does not break the rest of the repo.

**Step 4: Update issue tracking before handoff**

Append notes to:

- `EFF-271` describing the claim model, symlinked workspace root, strict-dirty checks, and stop/release behavior
- `EFF-268` noting that `in_place_git` is now implemented and remaining parent work is capability gating (`EFF-273`), plain directory mode (`EFF-274`), UI (`EFF-275`), and docs/verification (`EFF-272`)

**Step 5: Optional integration commit**

```bash
git status
git log --oneline -5
```

If the history still reads cleanly, finish with:

```bash
git add crates/db/migrations/20260417010000_add_workspace_repo_claims.sql crates/db/src/models/workspace_repo_claim.rs crates/db/src/models/mod.rs crates/db/src/models/workspace_repo.rs crates/git/src/lib.rs crates/git/src/cli.rs crates/git/tests/git_workflow.rs crates/local-deployment/src/container.rs crates/services/src/services/container.rs crates/server/src/routes/workspaces/create.rs docs/plans/2026-04-17-eff-271-in-place-git-design.md
git commit -m "feat: implement in-place git workspaces"
```
