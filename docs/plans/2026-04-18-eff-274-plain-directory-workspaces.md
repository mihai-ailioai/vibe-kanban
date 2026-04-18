# EFF-274 Plain Directory Workspaces Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Support `in_place_directory` workspaces for one non-Git directory source by validating the source path, provisioning a synthetic managed workspace root with a symlink into the real directory, and starting the agent inside that linked directory.

**Architecture:** Keep the existing route-owned `workspace_mode` plus canonical `workspace_sources` model. Validate the request shape in the create/start route, but perform filesystem truth checks in local deployment where the host path is actually used. Provision `in_place_directory` the same way `in_place_git` already treats in-place state: create a synthetic workspace root under the managed base dir, add one symlink entry pointing at the real directory, and clean up only that synthetic root. Make sessions resolve their default working directory to that symlink entry so attachments and executor actions continue to work without teaching the rest of the system about real host paths.

**Tech Stack:** Rust, Axum, SQLx, local deployment container service, workspace manager, session model logic, filesystem symlinks.

---

### Task 1: Tighten the create/start contract for exactly one directory source

**Files:**
- Modify: `crates/server/src/routes/workspaces/create.rs`
- Test: `crates/server/src/routes/workspaces/create.rs`

**Step 1: Write the failing route tests**

Add tests beside the existing `normalize_workspace_sources(...)` and `prepare_create_and_start_workspace(...)` coverage:

```rust
#[test]
fn normalize_workspace_sources_rejects_multiple_directory_sources_for_in_place_directory() {
    let err = normalize_workspace_sources(
        WorkspaceMode::InPlaceDirectory,
        vec![
            WorkspaceSourceInput::Directory {
                path: "/tmp/project-a".to_string(),
                display_name: Some("project-a".to_string()),
            },
            WorkspaceSourceInput::Directory {
                path: "/tmp/project-b".to_string(),
                display_name: Some("project-b".to_string()),
            },
        ],
        vec![],
    )
    .unwrap_err();

    assert!(matches!(
        err,
        ApiError::BadRequest(message)
            if message.contains("in_place_directory")
                && message.contains("exactly one")
                && message.contains("directory")
    ));
}

#[test]
fn prepare_create_and_start_workspace_rejects_multiple_directory_sources() {
    run_async_test(async {
        let pool = test_pool().await;
        let db = DBService { pool: pool.clone() };
        let workspace_manager = WorkspaceManager::new(db.clone());
        let git = GitService::new();

        let err = prepare_create_and_start_workspace(
            &pool,
            &workspace_manager,
            &git,
            CreateAndStartWorkspaceRequest {
                name: Some("Directory workspace".to_string()),
                workspace_mode: WorkspaceMode::InPlaceDirectory,
                sources: vec![
                    WorkspaceSourceInput::Directory {
                        path: "/tmp/project-a".to_string(),
                        display_name: Some("project-a".to_string()),
                    },
                    WorkspaceSourceInput::Directory {
                        path: "/tmp/project-b".to_string(),
                        display_name: Some("project-b".to_string()),
                    },
                ],
                repos: vec![],
                linked_issue: None,
                executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                prompt: "Create the workspace".to_string(),
                attachment_ids: None,
            },
            |_name, _mode| async { unreachable!("validation should fail before create") },
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ApiError::BadRequest(_)));
    });
}
```

**Step 2: Run the route tests to confirm failure**

Run: `cargo test -p server create::tests --lib`

Expected: FAIL because `validate_sources_for_mode(...)` currently only checks source kind and still allows more than one directory source.

**Step 3: Add the single-directory guard**

In `create.rs`, extend the `WorkspaceMode::InPlaceDirectory` branch in `validate_sources_for_mode(...)` with an explicit count check:

```rust
WorkspaceMode::InPlaceDirectory => {
    if sources
        .iter()
        .any(|source| !matches!(source, WorkspaceSourceInput::Directory { .. }))
    {
        return Err(ApiError::BadRequest(
            "Workspace mode `in_place_directory` only supports `directory` sources and does not support `git_repo` sources.".to_string(),
        ));
    }

    if sources.len() != 1 {
        return Err(ApiError::BadRequest(
            "Workspace mode `in_place_directory` requires exactly one `directory` source.".to_string(),
        ));
    }
}
```

Keep persistence unchanged: one canonical directory source and zero legacy `workspace_repos`.

**Step 4: Re-run the route tests**

Run: `cargo test -p server create::tests --lib`

Expected: PASS for the new single-directory validation and the existing directory-source acceptance coverage.

**Step 5: Commit**

```bash
git add crates/server/src/routes/workspaces/create.rs
git commit -m "fix: require a single directory workspace source"
```

### Task 2: Implement real `in_place_directory` provisioning in local deployment

**Files:**
- Modify: `crates/local-deployment/src/container.rs`
- Test: `crates/local-deployment/src/container.rs`

**Step 1: Write the failing provisioning tests**

Replace the placeholder not-implemented test with real coverage and add failure-path tests:

```rust
#[cfg(unix)]
#[test]
fn provision_workspace_for_mode_creates_in_place_directory_workspace_root() {
    run_async_test(async {
        let source_dir = std::env::temp_dir().join(format!(
            "local-deployment-directory-source-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&source_dir).unwrap();
        fs::write(source_dir.join("README.md"), "hello\n").unwrap();

        let workspace_dir = std::env::temp_dir().join(format!(
            "local-deployment-directory-workspace-{}",
            Uuid::new_v4()
        ));

        let provisioned_path = provision_workspace_for_mode(
            WorkspaceProvisioningAction::Create,
            WorkspaceMode::InPlaceDirectory,
            &[WorkspaceSourceInput::Directory {
                path: source_dir.to_string_lossy().to_string(),
                display_name: Some("non-git-project".to_string()),
            }],
            &workspace_dir,
            &[],
            "unused-branch",
        )
        .await
        .unwrap();

        assert_eq!(provisioned_path, workspace_dir);
        let entry = workspace_dir.join("non-git-project");
        assert!(entry.exists());
        assert_eq!(fs::read_link(&entry).unwrap(), source_dir);
    });
}

#[test]
fn provision_workspace_for_mode_rejects_missing_directory_source() {
    run_async_test(async {
        let err = provision_workspace_for_mode(
            WorkspaceProvisioningAction::Create,
            WorkspaceMode::InPlaceDirectory,
            &[WorkspaceSourceInput::Directory {
                path: "/tmp/definitely-missing-directory-workspace".to_string(),
                display_name: Some("missing".to_string()),
            }],
            Path::new("/tmp/unused"),
            &[],
            "unused-branch",
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("does not exist"));
    });
}

#[test]
fn provision_workspace_for_mode_rejects_file_source_for_in_place_directory() {
    run_async_test(async {
        let source_file = std::env::temp_dir().join(format!(
            "local-deployment-directory-file-{}",
            Uuid::new_v4()
        ));
        fs::write(&source_file, "not a directory\n").unwrap();

        let err = provision_workspace_for_mode(
            WorkspaceProvisioningAction::Create,
            WorkspaceMode::InPlaceDirectory,
            &[WorkspaceSourceInput::Directory {
                path: source_file.to_string_lossy().to_string(),
                display_name: Some("file-source".to_string()),
            }],
            Path::new("/tmp/unused"),
            &[],
            "unused-branch",
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("must be a directory"));
    });
}
```

Also add one `EnsureExists` test that deletes the synthetic root after a successful create, then asserts `provision_workspace_for_mode(EnsureExists, ...)` recreates the root and symlink from the persisted source.

**Step 2: Run the local deployment tests to confirm failure**

Run: `cargo test -p local-deployment --lib`

Expected: FAIL because the `WorkspaceMode::InPlaceDirectory` branch still returns `UnsupportedWorkspaceMode`.

**Step 3: Add directory-source validation and entry naming helpers**

In `container.rs`, introduce a small helper for the single directory source:

```rust
struct DirectoryWorkspaceInput {
    source_path: PathBuf,
    entry_name: String,
}

fn validate_in_place_directory_workspace_source(
    sources: &[WorkspaceSourceInput],
) -> Result<DirectoryWorkspaceInput, ContainerError> {
    let [WorkspaceSourceInput::Directory { path, display_name }] = sources else {
        return Err(ContainerError::Other(anyhow!(
            "Workspace mode `in_place_directory` requires exactly one `directory` source"
        )));
    };

    let source_path = PathBuf::from(path);
    if !source_path.exists() {
        return Err(ContainerError::Other(anyhow!(
            "Directory workspace source {} does not exist",
            source_path.display()
        )));
    }
    if !source_path.is_dir() {
        return Err(ContainerError::Other(anyhow!(
            "Directory workspace source {} must be a directory",
            source_path.display()
        )));
    }

    let entry_name = display_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            source_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .ok_or_else(|| ContainerError::Other(anyhow!(
            "Directory workspace source {} must have a usable name",
            source_path.display()
        )))?;

    if entry_name.contains(std::path::MAIN_SEPARATOR) || entry_name == "." || entry_name == ".." {
        return Err(ContainerError::Other(anyhow!(
            "Directory workspace entry name '{}' must be a single path component",
            entry_name
        )));
    }

    Ok(DirectoryWorkspaceInput {
        source_path,
        entry_name,
    })
}
```

If you prefer, split the entry-name validation into a second helper so the same rules can be mirrored in `session.rs` later.

**Step 4: Reuse the symlink-root pattern for directory workspaces**

Generalise the current repo-only helpers so both in-place modes can use them:

```rust
fn ensure_workspace_entry_symlink(source_path: &Path, entry_path: &Path) -> Result<(), ContainerError>

fn ensure_single_directory_workspace_root(
    workspace_dir: &Path,
    input: &DirectoryWorkspaceInput,
) -> Result<(), ContainerError> {
    std::fs::create_dir_all(workspace_dir)?;
    let entry_path = workspace_dir.join(&input.entry_name);
    ensure_workspace_entry_symlink(&input.source_path, &entry_path)
}
```

Then replace the `WorkspaceMode::InPlaceDirectory` match arm in `provision_workspace_for_mode(...)` with real provisioning:

```rust
(_, WorkspaceMode::InPlaceDirectory) => {
    if !expects_directory_sources() {
        return Err(ContainerError::Other(anyhow!(
            "Workspace mode `in_place_directory` requires `directory` sources"
        )));
    }

    let input = validate_in_place_directory_workspace_source(sources)?;
    ensure_single_directory_workspace_root(workspace_dir, &input)?;
    Ok(workspace_dir.to_path_buf())
}
```

Do **not** add `WorkspaceRepo` rows for this mode.

**Step 5: Re-run the local deployment tests**

Run: `cargo test -p local-deployment --lib`

Expected: PASS, with real in-place directory provisioning and clear validation failures for bad source paths.

**Step 6: Commit**

```bash
git add crates/local-deployment/src/container.rs
git commit -m "feat: provision in-place directory workspaces"
```

### Task 3: Start sessions inside the linked directory and make failed-start cleanup match in-place semantics

**Files:**
- Modify: `crates/db/src/models/session.rs`
- Modify: `crates/server/src/routes/workspaces/create.rs`
- Test: `crates/db/src/models/session.rs`
- Test: `crates/server/src/routes/workspaces/create.rs`

**Step 1: Write the failing session and cleanup tests**

Add a session model test that creates an `in_place_directory` workspace with one persisted directory source and asserts `Session::create(...)` stores the directory entry name as `agent_working_dir`:

```rust
#[test]
fn create_sets_agent_working_dir_for_single_directory_workspace_source() {
    run_async_test(async {
        let pool = test_pool().await;
        let workspace_id = Uuid::new_v4();

        Workspace::create(
            &pool,
            &CreateWorkspace {
                branch: format!("branch-{workspace_id}"),
                workspace_mode: WorkspaceMode::InPlaceDirectory,
                name: Some("Directory workspace".to_string()),
            },
            workspace_id,
        )
        .await
        .unwrap();

        WorkspaceSource::create_many(
            &pool,
            workspace_id,
            &[CreateWorkspaceSource::Directory {
                path: "/tmp/non-git-project".to_string(),
                display_name: Some("non-git-project".to_string()),
            }],
        )
        .await
        .unwrap();

        let session = Session::create(
            &pool,
            &CreateSession {
                executor: Some("CODEX".to_string()),
                name: None,
            },
            Uuid::new_v4(),
            workspace_id,
        )
        .await
        .unwrap();

        assert_eq!(session.agent_working_dir.as_deref(), Some("non-git-project"));
    });
}
```

Add a server create-route test mirroring the existing in-place Git cleanup coverage, but for `WorkspaceMode::InPlaceDirectory`: create a real temp directory, create a synthetic workspace root under `WorkspaceManager::get_workspace_base_dir()`, symlink the directory into it, call `cleanup_failed_create_and_start_workspace(...)`, and assert the synthetic root is removed while the real directory still exists.

**Step 2: Run the targeted tests to confirm failure**

Run: `cargo test -p db session --lib && cargo test -p server create::tests --lib`

Expected: FAIL because `Session::resolve_agent_working_dir(...)` only knows how to derive paths from exactly one attached repo, and failed create/start cleanup still sends `InPlaceDirectory` through the worktree cleanup branch.

**Step 3: Teach sessions to use the synthetic directory entry**

In `session.rs`, keep the current repo-based behaviour first, then add an `in_place_directory` fallback:

```rust
async fn resolve_agent_working_dir(
    pool: &SqlitePool,
    workspace_id: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let repos = WorkspaceRepo::find_repos_for_workspace(pool, workspace_id).await?;
    if repos.len() == 1 {
        let repo = &repos[0];
        let path = match repo.default_working_dir.as_deref() {
            Some(subdir) if !subdir.is_empty() => PathBuf::from(&repo.name).join(subdir),
            _ => PathBuf::from(&repo.name),
        };
        return Ok(Some(path.to_string_lossy().to_string()));
    }

    let Some(workspace) = Workspace::find_by_id(pool, workspace_id).await? else {
        return Ok(None);
    };
    if workspace.workspace_mode != WorkspaceMode::InPlaceDirectory {
        return Ok(None);
    }

    let sources = WorkspaceSource::find_by_workspace_id(pool, workspace_id).await?;
    let [source] = sources.as_slice() else {
        return Ok(None);
    };

    Ok(directory_source_entry_name(source))
}
```

Implement `directory_source_entry_name(...)` locally or as a tiny shared helper in `crates/db/src/models/workspace_source.rs`; it should prefer non-empty `display_name`, otherwise fall back to the source path basename.

**Step 4: Make failed create/start cleanup treat directory workspaces like other in-place modes**

In `create.rs`, update the cleanup match:

```rust
let cleanup_result = match workspace.workspace_mode {
    WorkspaceMode::InPlaceGit | WorkspaceMode::InPlaceDirectory => {
        cleanup_in_place_workspace_root(&workspace_dir)
            .await
            .map_err(|e| e.to_string())
    }
    WorkspaceMode::GitWorktree => WorkspaceManager::cleanup_workspace(&workspace_dir, &repositories)
        .await
        .map_err(|e| e.to_string()),
};
```

Rename `cleanup_in_place_git_workspace_root(...)` in `crates/local-deployment/src/container.rs` to something mode-neutral like `cleanup_in_place_workspace_root(...)`, then update the create route import and any local deployment callers.

**Step 5: Re-run the targeted tests**

Run: `cargo test -p db session --lib && cargo test -p server create::tests --lib`

Expected: PASS, with sessions starting inside the linked directory entry and failed starts cleaning up only the synthetic root.

**Step 6: Commit**

```bash
git add crates/db/src/models/session.rs crates/server/src/routes/workspaces/create.rs crates/local-deployment/src/container.rs
git commit -m "fix: run directory workspaces from linked source"
```

### Task 4: Verify the end-to-end slice and update the issue journal

**Files:**
- Modify: `docs/plans/2026-04-18-eff-274-plain-directory-workspaces.md` only if verification exposes gaps
- Update: Vibe Kanban issue `EFF-274` (`16f8aa27-dc8a-4393-bb95-326c383c6565`)

**Step 1: Run format**

Run: `pnpm run format`

Expected: PASS.

**Step 2: Run focused automated verification**

Run: `cargo test -p db session --lib && cargo test -p local-deployment --lib && cargo test -p server create::tests --lib`

Expected: PASS for the new route, provisioning, session, and cleanup coverage.

**Step 3: Run broader regression coverage**

Run: `cargo test -p server --lib && cargo test -p local-deployment --lib && pnpm run check`

Expected: PASS, confirming the new mode does not break existing Git-backed workspace flows.

**Step 4: Append issue journal entries**

Add entries like:

```text
- Added real `in_place_directory` provisioning using a synthetic workspace root with one symlinked source directory - keeps cleanup non-destructive while still running the agent inside the linked project
- Updated session working-dir resolution for directory workspaces - attachments and agent actions now resolve relative to the linked directory instead of the synthetic root
```

**Step 5: Move the issue to Done when all verification passes**

Use `get_issue(issue_id="16f8aa27-dc8a-4393-bb95-326c383c6565")`, append the final entry, then:

```text
- Done: plain directory workspaces now validate one source directory, create a safe synthetic root, and start agents inside the linked project path
```

**Step 6: Commit the verification pass if code changed during fixes**

```bash
git add crates/db/src/models/session.rs crates/server/src/routes/workspaces/create.rs crates/local-deployment/src/container.rs
git commit -m "test: verify plain directory workspace flow"
```
