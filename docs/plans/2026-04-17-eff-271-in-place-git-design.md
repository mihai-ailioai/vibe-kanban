# EFF-271 In-Place Git Workspace Design

**Ticket:** `EFF-271`  
**Parent:** `EFF-268`  
**Branch:** `feat/eff-268-optional-workspace-modes`

## Goal

Implement real `in_place_git` workspaces on top of the EFF-270 mode-driven provisioning seam so a workspace can use real repo checkouts instead of worktrees while enforcing exclusive runtime ownership and non-destructive cleanup.

## Agreed product decisions

- Ownership is **repo-exclusive**.
- A repo is considered dirty if it has **any modified, staged, or untracked files**.
- Cleanup is **non-destructive** and leaves repos on the workspace branch.
- A repo claim blocks reuse only while another `in_place_git` workspace is **actively running**.
- If the workspace branch already exists, provisioning should **checkout the existing branch**.
- If `target_branch` does not exist locally, provisioning should **fail fast**.
- Multi-repo `in_place_git` workspaces are allowed.
- Provisioning is **all-or-nothing** across repos.
- Stopping a workspace releases repo claims immediately.
- Multi-repo layout should keep the existing synthetic workspace root and expose each real repo as a symlink at `<workspace_root>/<repo.name>`.
- `ensure_container_exists(...)` should recreate missing or broken repo symlinks automatically.
- If `<workspace_root>/<repo.name>` already exists as a real file or directory instead of a symlink, provisioning should **fail fast** rather than deleting it automatically.

## Architecture

`in_place_git` should plug into the existing EFF-270 dispatch seam rather than introducing a parallel provisioning path. The request remains normalized in `crates/server/src/routes/workspaces/create.rs`, while actual provisioning happens in `crates/local-deployment/src/container.rs` under the `WorkspaceMode::InPlaceGit` branch.

That branch should stop being a placeholder and become a real provisioning flow built around two concepts:

1. **Atomic preflight validation** across all Git sources for the workspace
2. **Runtime repo claims** that represent temporary exclusive ownership while the workspace is actively running

Provisioning should first resolve canonical `WorkspaceSourceInput::GitRepo` entries to repos, validate all repos as one batch, acquire claims, and then perform branch setup. If any repo fails validation, no repo state should change anywhere.

The workspace root should remain the same synthetic container directory shape used by worktree-backed workspaces today. For `in_place_git`, the difference is that each repo entry under that root should be a symlink to the real repo checkout rather than a separate worktree directory.

## Repo claim model

Claims should be explicit and DB-backed rather than inferred from current Git checkout state. Each claim ties a workspace to a repo for `in_place_git` runtime ownership.

Claim lifecycle:

- acquire before checkout mutation begins
- hold while the workspace is actively running
- release on stop, failed start, or teardown
- do not require deletion or archival to free the repo

For multi-repo workspaces, claims should be treated as a batch. If any repo is unavailable, the whole start fails. If provisioning fails after claims are acquired, all newly acquired claims must be released.

## Validation order

Validation should run in a deterministic order:

1. Ensure all sources are Git repo sources valid for `in_place_git`
2. Resolve configured repos from the source list
3. Verify every `target_branch` exists locally
4. Verify every repo is clean, including untracked files
5. Verify no repo is claimed by another actively running `in_place_git` workspace

Only after all checks succeed should the system acquire claims and begin branch switching.

## Branch behavior

For each repo in the workspace:

- if the workspace branch already exists locally, checkout that branch
- otherwise create the workspace branch from the specified `target_branch`

The entire operation must stay all-or-nothing. Partial success across repos is not allowed.

## Failure handling

Two failure classes matter:

- **Preflight failures**: return a clear `BadRequest` or `Conflict` and make no repo changes
- **Mutation failures**: occur after claims are acquired and possibly after some repos have switched branches

For mutation failures, the minimum required rollback is reliable release of claims. The implementation does **not** need to restore previous branches because cleanup is intentionally non-destructive and should avoid hidden repo mutations.

## Testing focus

EFF-271 should add coverage for:

- dirty repo rejection, including untracked files
- missing local `target_branch` rejection
- claim conflict rejection when another active in-place workspace owns a repo
- multi-repo all-or-nothing preflight behavior
- checkout of existing workspace branches
- creation of workspace branches from `target_branch`
- claim release on stop
- claim release on failed start

## Out of scope

- plain directory workspaces (`EFF-274`)
- capability gating for Git APIs and cleanup (`EFF-273`)
- create-workspace UI changes (`EFF-275`)
- docs/final verification sweep (`EFF-272`)
