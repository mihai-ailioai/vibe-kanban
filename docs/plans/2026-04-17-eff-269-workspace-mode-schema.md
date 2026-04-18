# EFF-269 Workspace Mode Schema Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add mode-aware workspace persistence and API contracts for optional workspace modes, while keeping the rest of the repo working and migrating current web and MCP callers to the new request shape.

**Architecture:** Add a new `WorkspaceMode` enum to `workspaces` and a canonical `workspace_sources` table/model that can represent Git-backed and plain-directory sources. Keep current workspace creation behaviour functionally equivalent by defaulting existing callers to `git_worktree`, persisting Git repo attachments in both the legacy `workspace_repos` table and the new `workspace_sources` table, and returning the new source data from the create/start API so EFF-270 can switch provisioning over without another contract break.

**Tech Stack:** Rust, SQLx/SQLite migrations, ts-rs generated shared types, React/TypeScript, MCP JSON schema.

---

### Task 1: Lock down request normalisation in workspace creation

**Files:**
- Modify: `crates/server/src/routes/workspaces/create.rs`

**Step 1: Write a failing test for the default Git worktree path**

Add a unit test beside the existing `#[cfg(test)]` module that feeds the new request shape into a helper such as `normalize_workspace_sources(...)` and asserts the result is:

```rust
vec![WorkspaceSourceInput::GitRepo {
    repo_id,
    target_branch: "main".to_string(),
}]
```

with `WorkspaceMode::GitWorktree` when the caller sends the current repo selection flow.

**Step 2: Write a failing test for an invalid mode/source combination**

Add a second test that sends `WorkspaceMode::InPlaceDirectory` with a Git repo source and assert it returns `ApiError::BadRequest` with a mode mismatch message.

**Step 3: Write a failing test for legacy repo compatibility**

Add a third test that passes legacy `repos` input and asserts the helper synthesises:

```rust
WorkspaceMode::GitWorktree
```

plus one Git source per repo.

**Step 4: Run the server tests to confirm failure**

Run: `cargo test -p server create::tests --lib`

Expected: FAIL because the normalisation helper and new types do not exist yet.

### Task 2: Add the database schema for workspace modes and sources

**Files:**
- Create: `crates/db/migrations/20260417000000_add_workspace_mode_and_sources.sql`
- Modify: `crates/db/src/models/mod.rs`
- Modify: `crates/db/src/models/workspace.rs`
- Create: `crates/db/src/models/workspace_source.rs`

**Step 1: Add the failing migration and SQLx-backed model changes**

Create a migration that:

```sql
ALTER TABLE workspaces ADD COLUMN workspace_mode TEXT NOT NULL DEFAULT 'git_worktree';

CREATE TABLE workspace_sources (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  source_type TEXT NOT NULL,
  repo_id TEXT,
  path TEXT,
  display_name TEXT,
  target_branch TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  updated_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE,
  FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
);
```

and add `CHECK` constraints so Git sources require `repo_id` + `target_branch`, while directory sources require `path`.

**Step 2: Extend the workspace DB model**

Add:

```rust
pub enum WorkspaceMode {
    GitWorktree,
    InPlaceGit,
    InPlaceDirectory,
}
```

to `crates/db/src/models/workspace.rs`, add `workspace_mode: WorkspaceMode` to `Workspace`, and update `CreateWorkspace` so it stores the mode at insert time.

**Step 3: Add the workspace source model**

Create `crates/db/src/models/workspace_source.rs` with structs along these lines:

```rust
pub enum WorkspaceSourceKind { GitRepo, Directory }

pub struct WorkspaceSource {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub source_type: WorkspaceSourceKind,
    pub repo_id: Option<Uuid>,
    pub path: Option<String>,
    pub display_name: Option<String>,
    pub target_branch: Option<String>,
}
```

plus `create_many(...)` and `find_by_workspace_id(...)` helpers.

**Step 4: Refresh SQLx metadata**

Run: `pnpm run prepare-db`

Expected: SQLx offline data updates successfully for the new migration and queries.

### Task 3: Extend Rust request, response, and scratch contracts

**Files:**
- Modify: `crates/db/src/models/requests.rs`
- Modify: `crates/db/src/models/scratch.rs`
- Modify: `crates/server/src/bin/generate_types.rs`

**Step 1: Add the new request-side types**

In `crates/db/src/models/requests.rs`, add:

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceSourceInput {
    GitRepo { repo_id: Uuid, target_branch: String },
    Directory { path: String, display_name: Option<String> },
}
```

and update `CreateAndStartWorkspaceRequest` to include:

```rust
pub workspace_mode: WorkspaceMode,
pub sources: Vec<WorkspaceSourceInput>,
#[serde(default)]
pub repos: Vec<WorkspaceRepoInput>,
```

so existing repo-only callers can still be normalised during the transition.

**Step 2: Add the new response-side contract**

Update `CreateAndStartWorkspaceResponse` to return the canonical persisted source list:

```rust
pub struct CreateAndStartWorkspaceResponse {
    pub workspace: Workspace,
    pub sources: Vec<WorkspaceSource>,
    pub execution_process: ExecutionProcess,
}
```

**Step 3: Update scratch payloads for the web draft flow**

Replace the repo-only draft shape in `crates/db/src/models/scratch.rs` with mode-aware fields, for example:

```rust
pub struct DraftWorkspaceData {
    pub message: String,
    pub workspace_mode: WorkspaceMode,
    pub sources: Vec<WorkspaceSourceInput>,
    // existing executor, linked issue, attachments...
}
```

Keep `repos` as a serde alias only if TypeScript migration becomes easier with a temporary fallback.

**Step 4: Regenerate shared TypeScript types**

Run: `pnpm run generate-types`

Expected: `shared/types.ts` now contains `WorkspaceMode`, `WorkspaceSourceInput`, `WorkspaceSource`, and the updated create/scratch contracts.

### Task 4: Persist canonical sources in the create/start route

**Files:**
- Modify: `crates/server/src/routes/workspaces/create.rs`
- Modify: `crates/db/src/models/workspace.rs`
- Modify: `crates/db/src/models/workspace_source.rs`

**Step 1: Implement the normalisation helper**

Add a helper in `create.rs` that accepts the new request, validates mode/source combinations, and returns a canonical pair:

```rust
struct NormalizedWorkspaceRequest {
    workspace_mode: WorkspaceMode,
    sources: Vec<WorkspaceSourceInput>,
    legacy_git_repos: Vec<WorkspaceRepoInput>,
}
```

**Step 2: Store the workspace mode at record creation time**

Update `create_workspace_record(...)` to accept `workspace_mode: WorkspaceMode` and persist it through `CreateWorkspace`.

**Step 3: Persist the new `workspace_sources` rows**

After the workspace record is created, write the canonical source list via `WorkspaceSource::create_many(...)`.

For Git-backed modes, continue calling `managed_workspace.add_repository(...)` so the existing `workspace_repos`-based flows remain functional before EFF-270.

**Step 4: Return the new source list in the API response**

Change the success body to:

```rust
CreateAndStartWorkspaceResponse {
    workspace,
    sources,
    execution_process,
}
```

and re-run:

Run: `cargo test -p server create::tests --lib`

Expected: PASS for the new request normalisation coverage.

### Task 5: Migrate the MCP workspace starter to the new contract

**Files:**
- Modify: `crates/mcp/src/task_server/tools/task_attempts.rs`

**Step 1: Update the MCP request schema**

Keep the public MCP UX repo-based for now, but build the new API payload under the hood by mapping:

```rust
repositories -> WorkspaceMode::GitWorktree + Vec<WorkspaceSourceInput::GitRepo>
```

before calling `/api/workspaces/start`.

**Step 2: Update the API payload creation**

Replace the repo-only body with:

```rust
CreateAndStartWorkspaceRequest {
    name: Some(name.clone()),
    workspace_mode: WorkspaceMode::GitWorktree,
    sources,
    repos: workspace_repos,
    linked_issue,
    executor_config,
    prompt: workspace_prompt,
    attachment_ids: None,
}
```

**Step 3: Verify MCP compiles cleanly**

Run: `cargo check -p mcp`

Expected: PASS with no schema or payload type errors.

### Task 6: Migrate the current web create-workspace caller to the new contract

**Files:**
- Modify: `packages/web-core/src/shared/types/createMode.ts`
- Modify: `packages/web-core/src/shared/lib/workspaceCreateState.ts`
- Modify: `packages/web-core/src/features/create-mode/model/useCreateModeState.ts`
- Modify: `packages/web-core/src/shared/hooks/useCreateWorkspace.ts`

**Step 1: Add a default create-mode workspace mode**

Extend `CreateModeInitialState` so the current UI has a stable default:

```ts
workspaceMode?: 'git_worktree' | 'in_place_git' | 'in_place_directory' | null;
```

and seed existing callers with `'git_worktree'`.

**Step 2: Convert draft persistence to sources**

Update `toDraftWorkspaceData(...)` and the debounced scratch save in `useCreateModeState.ts` so selected repos become:

```ts
sources: state.repos.map((r) => ({
  type: 'git_repo',
  repo_id: r.repo.id,
  target_branch: r.targetBranch ?? '',
}))
```

with `workspace_mode: 'git_worktree'`.

**Step 3: Send the new create/start payload**

Update the data passed through `useCreateWorkspace()` so the web app posts `workspace_mode` + `sources` (and only keeps `repos` if the server transition still needs it).

**Step 4: Run the web typecheck**

Run: `pnpm run check`

Expected: PASS for TypeScript after the shared types and web callers are aligned.

### Task 7: Verify the repo stays healthy after the schema change

**Files:**
- Modify: `docs/plans/2026-04-17-eff-269-workspace-mode-schema.md` (optional notes only if execution reveals adjustments)

**Step 1: Format the repo**

Run: `pnpm run format`

Expected: PASS with Rust and web formatting applied.

**Step 2: Run the targeted backend checks**

Run: `cargo test -p server && cargo check -p mcp`

Expected: PASS for the route tests and MCP boundary.

**Step 3: Run the repo-wide verification gate**

Run: `pnpm run check`

Expected: PASS so the non-workspace parts of the repo remain fully functional.

**Step 4: Update the issue journal before handing off**

Append entries to:

- `EFF-269` describing the schema and caller migration that landed
- `EFF-268` noting that the first blocking contract slice is complete and EFF-270 can consume the new model next
