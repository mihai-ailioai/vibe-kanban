# EFF-270 Mode-Driven Workspace Provisioning Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Refactor workspace create/start so provisioning is selected by `workspace_mode`, while keeping the current `git_worktree` path working and returning clear temporary errors for `in_place_git` and `in_place_directory` until EFF-271 and EFF-274 land.

**Architecture:** Move the remaining worktree assumptions out of shared request preparation and into explicit mode-dispatch seams. The server route should persist canonical sources for every mode, only attach legacy `workspace_repos` for the `git_worktree` branch, and let the container layer decide how to provision the workspace. The local container service should load canonical `workspace_sources`, branch on `workspace.workspace_mode`, keep the existing worktree path as one match arm, and return explicit unsupported-mode errors from placeholder arms so later tickets can fill them in without another refactor.

**Tech Stack:** Rust, Axum, SQLx, local deployment container service, workspace manager, git/worktree helpers.

---

### Task 1: Make route preparation mode-aware instead of worktree-only

**Files:**
- Modify: `crates/server/src/routes/workspaces/create.rs`

**Step 1: Add a failing test for `in_place_git` request preparation**

Add a route test beside the existing `prepare_create_and_start_workspace(...)` coverage that sends:

```rust
CreateAndStartWorkspaceRequest {
    name: Some("In-place git workspace".to_string()),
    workspace_mode: WorkspaceMode::InPlaceGit,
    sources: vec![WorkspaceSourceInput::GitRepo {
        repo_id: repo.id,
        target_branch: "main".to_string(),
    }],
    repos: vec![],
    linked_issue: None,
    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
    prompt: "Start the workspace".to_string(),
    attachment_ids: None,
}
```

and assert that `prepare_create_and_start_workspace(...)` succeeds, persists one canonical source, preserves `WorkspaceMode::InPlaceGit`, and leaves `prepared.managed_workspace.repos` empty because this branch must no longer auto-attach legacy `workspace_repos`.

**Step 2: Add a failing test for `in_place_directory` preparation**

Add a second test that sends:

```rust
CreateAndStartWorkspaceRequest {
    name: Some("Directory workspace".to_string()),
    workspace_mode: WorkspaceMode::InPlaceDirectory,
    sources: vec![WorkspaceSourceInput::Directory {
        path: "/tmp/non-git-project".to_string(),
        display_name: Some("non-git-project".to_string()),
    }],
    // ...same executor/prompt fields...
}
```

and assert the route no longer returns the current "only `git_repo` workspace sources are currently supported" error.

**Step 3: Run the route tests to confirm failure**

Run: `cargo test -p server create::tests --lib`

Expected: FAIL because `normalize_workspace_sources(...)` still rejects directory-backed requests and `prepare_create_and_start_workspace(...)` still requires/attaches legacy repos for every mode.

**Step 4: Implement minimal mode-aware normalization and preparation**

In `create.rs`, keep the existing compatibility rules for mixed `sources` + `repos`, but change the mode handling so it looks like this:

```rust
let legacy_git_repos = match workspace_mode {
    WorkspaceMode::GitWorktree => sources
        .iter()
        .map(git_source_to_workspace_repo)
        .collect::<Result<Vec<_>, _>>()?,
    WorkspaceMode::InPlaceGit | WorkspaceMode::InPlaceDirectory => Vec::new(),
};
```

and validate source kinds by mode instead of by "currently supported":

```rust
match workspace_mode {
    WorkspaceMode::GitWorktree | WorkspaceMode::InPlaceGit => {
        // require git_repo sources only
    }
    WorkspaceMode::InPlaceDirectory => {
        // require directory sources only
    }
}
```

Then update `prepare_create_and_start_workspace(...)` so the blanket repo check and `managed_workspace.add_repository(...)` loop only run inside the explicit `WorkspaceMode::GitWorktree` branch.

**Step 5: Re-run the route tests**

Run: `cargo test -p server create::tests --lib`

Expected: PASS for the new non-worktree preparation tests and the existing git-worktree regression coverage.

**Step 6: Commit the route refactor**

```bash
git add crates/server/src/routes/workspaces/create.rs
git commit -m "refactor: make workspace preparation mode aware"
```

### Task 2: Add a container-side provisioning dispatcher keyed by `workspace_mode`

**Files:**
- Modify: `crates/local-deployment/src/container.rs`

**Step 1: Add failing tests for the new dispatcher seam**

Create a `#[cfg(test)]` module in `container.rs` and add focused tests for a new helper such as `provision_workspace_for_mode(...)` or `load_workspace_provisioning_context(...)`:

- one test that seeds a normal repo-backed workspace and asserts `WorkspaceMode::GitWorktree` still reaches `WorkspaceManager::create_workspace(...)`
- one test that calls the helper with `WorkspaceMode::InPlaceGit` and asserts the error message contains both `in_place_git` and `not implemented`
- one test that calls the helper with `WorkspaceMode::InPlaceDirectory` and asserts the error message contains both `in_place_directory` and `not implemented`

Keep the non-git tests lightweight by constructing the mode directly and asserting the helper returns the expected placeholder error before any worktree operation runs.

**Step 2: Run the local deployment tests to confirm failure**

Run: `cargo test -p local-deployment --lib`

Expected: FAIL because the container still calls `workspace_repo_inputs(...)` unconditionally and has no mode-driven provisioning helper yet.

**Step 3: Introduce a provisioning context that starts from canonical sources**

Refactor `container.rs` so it loads canonical sources first:

```rust
struct WorkspaceProvisioningContext {
    sources: Vec<WorkspaceSource>,
    repositories: Vec<Repo>,
    workspace_inputs: Vec<RepoWorkspaceInput>,
}
```

or an equivalent enum with explicit match arms. The key rule is:

- always load `WorkspaceSource::find_by_workspace_id(...)`
- only call `workspace_repo_inputs(...)` inside the `WorkspaceMode::GitWorktree` branch
- leave `InPlaceGit` and `InPlaceDirectory` with enough context to be implemented later without reshaping the call sites again

**Step 4: Route both `create(...)` and `ensure_container_exists(...)` through the dispatcher**

Replace the current unconditional flow:

```rust
let (repositories, workspace_inputs) = self.workspace_repo_inputs(workspace.id).await?;
WorkspaceManager::create_workspace(&workspace_dir, &workspace_inputs, &workspace.branch).await?;
```

with:

```rust
match workspace.workspace_mode {
    WorkspaceMode::GitWorktree => { /* current worktree provisioning path */ }
    WorkspaceMode::InPlaceGit => Err(ContainerError::unsupported_workspace_mode(...)),
    WorkspaceMode::InPlaceDirectory => Err(ContainerError::unsupported_workspace_mode(...)),
}
```

Apply the same pattern to `ensure_container_exists(...)` so restarts do not keep assuming worktrees.

**Step 5: Re-run the local deployment tests**

Run: `cargo test -p local-deployment --lib`

Expected: PASS, with `git_worktree` still provisioning worktrees and the two placeholder modes failing with clear temporary errors.

**Step 6: Commit the dispatcher seam**

```bash
git add crates/local-deployment/src/container.rs
git commit -m "refactor: dispatch workspace provisioning by mode"
```

### Task 3: Surface unsupported workspace modes as explicit client errors

**Files:**
- Modify: `crates/services/src/services/container.rs`
- Modify: `crates/server/src/error.rs`

**Step 1: Add a failing mapping test in `server::error`**

Create a small `#[cfg(test)]` module in `crates/server/src/error.rs` with a test like:

```rust
#[test]
fn unsupported_workspace_mode_maps_to_bad_request() {
    let api_error = ApiError::from(ContainerError::UnsupportedWorkspaceMode {
        mode: WorkspaceMode::InPlaceDirectory,
    });

    assert!(matches!(
        api_error,
        ApiError::BadRequest(message)
            if message.contains("in_place_directory") && message.contains("not implemented")
    ));
}
```

**Step 2: Run the targeted server test to confirm failure**

Run: `cargo test -p server error --lib`

Expected: FAIL because `ContainerError` does not yet have an unsupported-mode variant and `ApiError::from(ContainerError)` currently treats container fallthroughs as internal errors.

**Step 3: Add the explicit container error variant**

In `crates/services/src/services/container.rs`, add a variant along these lines:

```rust
#[error("Workspace mode `{mode}` is not implemented yet")]
UnsupportedWorkspaceMode { mode: WorkspaceMode },
```

and add a tiny helper if that makes the call sites cleaner:

```rust
impl ContainerError {
    pub fn unsupported_workspace_mode(mode: WorkspaceMode) -> Self {
        Self::UnsupportedWorkspaceMode { mode }
    }
}
```

**Step 4: Map that variant to a user-facing `BadRequest`**

Update `impl From<ContainerError> for ApiError` so this new variant becomes:

```rust
ApiError::BadRequest(format!(
    "Workspace mode `{mode}` is not implemented yet"
))
```

Leave the rest of the container error mapping unchanged.

**Step 5: Re-run the targeted mapping test**

Run: `cargo test -p server error --lib`

Expected: PASS, proving that the placeholder mode branches will now reach the client as clear temporary errors instead of 500s.

**Step 6: Commit the error-surface fix**

```bash
git add crates/services/src/services/container.rs crates/server/src/error.rs
git commit -m "fix: return unsupported workspace mode errors"
```

### Task 4: Verify the seam and update tracking

**Files:**
- No code changes expected unless verification exposes regressions

**Step 1: Format the touched code**

Run: `pnpm run format`

Expected: PASS with Rust formatting applied across the touched crates.

**Step 2: Re-run targeted backend checks**

Run: `cargo test -p server create::tests --lib && cargo test -p server error --lib && cargo test -p local-deployment --lib`

Expected: PASS for route preparation, error mapping, and local provisioning dispatcher coverage.

**Step 3: Run the repo-wide verification gate**

Run: `pnpm run check`

Expected: PASS so the refactor does not break unrelated backend or frontend packages.

**Step 4: Update issue tracking before handoff**

Append implementation notes to:

- `EFF-270` describing the new mode-driven provisioning seam and placeholder mode branches
- `EFF-268` noting that `git_worktree` still works through the explicit dispatcher and EFF-271 / EFF-274 can now implement their branches without another contract refactor

**Step 5: Optional integration commit**

```bash
git status
git log --oneline -5
```

If the branch history still reads cleanly, create a final integration commit such as:

```bash
git add crates/server/src/routes/workspaces/create.rs crates/local-deployment/src/container.rs crates/services/src/services/container.rs crates/server/src/error.rs
git commit -m "refactor: add mode-driven workspace provisioning seam"
```
