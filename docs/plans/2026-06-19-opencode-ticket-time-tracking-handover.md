# OpenCode Ticket Time Tracking Handover

**Date:** 2026-06-19
**Repo:** `/Users/mihai/Work/vibe-kanban`
**Issue:** `EFF-818` / `e250a7f0-a8b1-48b3-a5ac-278bc8cbc35a`
**Project:** `b6eee4fd-8ea2-4945-ada9-0b17115ef642`
**Status:** Implementation mostly complete; Task 13 manual local smoke and deployment research remain.

## User constraints and workflow

- User requested **subagent-driven, no worktrees**.
- All work has happened in the existing checkout: `/Users/mihai/Work/vibe-kanban`.
- Do **not** commit, stage, push, amend, or open a PR unless the user explicitly asks.
- Task 12 manual adjustments were explicitly skipped by the user: “let's skip task 12.”
- The user self-hosts vibe-kanban on a **Contabo machine** and expects remote-server update knowledge may exist from prior sessions. This has **not yet been researched in the repo from a correctly initialized vibe-kanban session**.

## Design and plan docs

Created/updated:

- `docs/plans/2026-06-18-opencode-ticket-time-tracking-design.md`
- `docs/plans/2026-06-18-opencode-ticket-time-tracking-implementation.md`

Important design decisions:

- Standalone/global OpenCode plugin is the primary collector because the user normally runs OpenCode outside vibe-kanban.
- Plugin binds an OpenCode session to a vibe-kanban issue URL found in user messages.
- Binding is per OpenCode session, persists across OpenCode restarts, never falls back to a global “last ticket.”
- A new issue URL in the same OpenCode session switches future time to the new ticket only.
- Active-time tracking is conservative: avoid major overcounts, exclude idle/waiting/approval time, cap recovered intervals.
- Auth uses a narrow local `vktt_` plugin token, not normal user/session credentials.
- Local backend forwards accepted entries to the remote API with normal remote credentials.
- Remote Postgres owns immutable entries and synced totals.

## Implemented tasks

### Task 1 — Shared contracts

Added shared time-tracking contracts:

- `crates/api-types/src/time_tracking.rs`
- `crates/api-types/src/lib.rs`
- `crates/remote/src/bin/generate_types.rs`
- `shared/remote-types.ts` regenerated

Key details:

- `IssueTimeTotal`
- `CreateOpenCodeTimeEntriesRequest`
- `OpenCodeTimeEntryInput`
- `CreateOpenCodeTimeEntriesResponse`
- token-management local contracts later added in the same API module
- JSON-facing `i64` fields are annotated to generate TypeScript `number`, not `bigint`.
- `CreateIssueTimeAdjustmentRequest.entry_id` is optional.

### Task 2 — Remote Postgres schema

Added:

- `crates/remote/migrations/20260618000000_issue_time_tracking.sql`

Schema:

- `issue_time_entries` immutable/idempotent audit table; **not** Electric-synced.
- `issue_time_totals` aggregate table; Electric-synced.
- `issue_time_totals` has `REPLICA IDENTITY FULL` and `electric_sync_table('public', 'issue_time_totals')`.
- Triggers enforce `issue_id` belongs to `project_id`.
- Added child indexes, source/kind constraints, active interval constraints, aggregate sanity checks.

Known blocker:

- `pnpm run remote:prepare-db` cannot run locally because `initdb` is missing.

### Task 3 — Remote repository logic

Added:

- `crates/remote/src/db/issue_time_tracking.rs`
- `crates/remote/src/db/mod.rs`

Implemented:

- validation for schema version, batch size, timing bounds, metadata size, issue/project relation
- canonical SHA-256 payload hash over immutable fields; metadata intentionally excluded and documented
- concurrency-safe idempotency with `INSERT ... ON CONFLICT (entry_id) DO NOTHING RETURNING entry_id`
- duplicate vs idempotency conflict behavior
- transactionally insert entries, upsert totals, return `txid` and results
- `list_totals_by_project` for shape fallback

Focused wrapper tests passed after fixes: remote `time_tracking --lib` had 12 tests passing by the end.

### Task 4 — Remote routes and Electric shape

Added/changed:

- `crates/remote/src/routes/time_tracking.rs`
- `crates/remote/src/routes/mod.rs`
- `crates/remote/src/shapes.rs`
- `crates/remote/src/shape_routes.rs`
- `crates/remote/src/bin/generate_types.rs`
- `shared/remote-types.ts` regenerated

Implemented:

- protected `POST /v1/time-tracking/opencode/entries`
- project authorization before repository write
- error mapping: idempotency conflict `409`, invalid request `400`, generic `500`
- `PROJECT_ISSUE_TIME_TOTALS_SHAPE`
- `/fallback/issue_time_totals` with project access check

Note:

- New shape uses a direct `ShapeDefinition` constant with a comment because SQLx macro validation requires DB metadata that is unavailable locally.

### Task 5 — Local plugin token persistence

Added:

- `crates/db/migrations/20260618000000_opencode_time_tracking_tokens.sql`
- `crates/db/src/models/time_tracking_token.rs`
- `crates/db/src/models/mod.rs`

Implemented:

- SQLite `opencode_time_tracking_tokens` table
- hash-only token storage
- constants `OPENCODE_TIME_TRACKING_SCOPE = "time_tracking:write"` and `OPENCODE_TIME_TRACKING_TOKEN_PREFIX = "vktt_"`
- create/find/list/mark_used/revoke methods
- revocation is idempotent after Task 6 fixes: preserves original `revoked_at`

### Task 6 — Local API and remote forwarding

Added/changed:

- `crates/server/src/routes/time_tracking.rs`
- `crates/server/src/routes/mod.rs`
- `crates/services/src/services/remote_client.rs`
- `crates/api-types/src/time_tracking.rs`
- `crates/server/src/bin/generate_types.rs`
- `shared/types.ts` regenerated

Implemented local routes:

- `POST /api/time-tracking/opencode/entries`
- `POST /api/time-tracking/opencode/tokens`
- `GET /api/time-tracking/opencode/tokens`
- `DELETE /api/time-tracking/opencode/tokens/{token_id}`

Implemented:

- `RemoteClient::create_opencode_time_entries`
- bearer `vktt_` token auth
- SHA-256 hash lookup of non-revoked token
- one-time plaintext token creation response
- metadata-only token list, no token hash/raw token exposure
- generic indistinguishable 401 failures for malformed/unknown/revoked token
- mark `last_used_at` only after successful remote forward

Task 13 lint fix changed `extract_bearer_token` to return `Option<&str>` and map to `ApiError::Unauthorized` at the call site to avoid Clippy `result_large_err`.

### Task 7 — OpenCode plugin package

Added:

- `packages/opencode-time-tracker/**`
- `pnpm-lock.yaml` updated

Package name:

- `@vibe/opencode-time-tracker`

Implemented:

- `src/url.ts` parses vibe-kanban issue URLs like `/projects/:projectId/issues/:issueId`
- `src/state.ts` file-backed per-session JSON state
- `src/time.ts` conservative active-time state machine
- `src/client.ts` posts local `ApiResponse<CreateOpenCodeTimeEntriesResponse>` to `/api/time-tracking/opencode/entries`
- `src/index.ts` OpenCode plugin entrypoint and hooks

Important fixes made during review:

- `entry_id` now uses `crypto.randomUUID()`.
- File names match plan: `time.ts` / `time.test.ts`, no stale `timer.ts` source.
- missing session IDs no-op; no `undefined.json` state writes.
- missing/malformed `servers` options retain pending entries and do not throw.
- strict local `ApiResponse` envelope validation; malformed success responses keep pending entries.
- per-session update queue serializes load/mutate/flush/save.
- ticket-switch interval close applies configured recovered interval cap.
- `sessionQueues` cleanup leak fixed by storing/comparing the same tracked promise.

Verification passed:

- `pnpm --filter @vibe/opencode-time-tracker run test` — 24 tests
- `pnpm --filter @vibe/opencode-time-tracker run check`
- `pnpm --filter @vibe/opencode-time-tracker run build`

Note:

- `@opencode-ai/plugin` pinned to `1.0.224` because latest pulled a Node engine mismatch in this environment.
- `packages/opencode-time-tracker/dist/` may exist from build output and should be reviewed/ignored/removed depending repo convention before commit.

### Task 8 — Project context totals

Changed:

- `packages/web-core/src/shared/providers/remote/ProjectProvider.tsx`
- `packages/web-core/src/shared/hooks/useProjectContext.ts`
- `packages/web-core/src/shared/integrations/electric/hooks.ts`

Implemented:

- subscribe to `PROJECT_ISSUE_TIME_TOTALS_SHAPE`
- expose `issueTimeTotals`, `issueTimeTotalsByIssueId`, `getIssueTimeTotal`
- totals loading/error are non-fatal and not part of board readiness
- added `suppressErrorRegistration?: boolean` to `useShape`, default false, used only for totals shape

### Task 9 — Formatter and badge

Added:

- `packages/web-core/src/shared/lib/issueTime.ts`
- `packages/web-core/src/shared/lib/issueTime.test.ts`
- `packages/ui/src/components/IssueTimeBadge.tsx`

Implemented:

- `getIssueTotalMs`
  - accepts number/string/bigint-ish `total_ms`
  - invalid, non-finite, unsafe values return `0`
  - safe negative values preserved so formatter hides them
- `formatIssueActiveTime`
  - `<= 0` => `null`
  - `< 1m`
  - minutes, hours+minutes, days+hours
- accessible `IssueTimeBadge` with decorative clock, sr-only context, native `title`

### Task 10 — Render time in UI

Changed:

- `packages/ui/src/components/KanbanCardContent.tsx`
- `packages/ui/src/components/IssueListView.tsx`
- `packages/ui/src/components/IssueListSection.tsx`
- `packages/ui/src/components/IssueListRow.tsx`
- `packages/ui/src/components/KanbanIssuePanel.tsx`
- `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx`
- `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx`

Implemented:

- card badge in existing badge row
- issue-list badge before assignees/age
- issue-detail section near metadata, only when not create mode and label exists
- `KanbanContainer` formats labels from `issueTimeTotalsByIssueId`

### Task 11 — Settings UI for plugin tokens

Changed:

- `packages/web-core/src/shared/lib/api.ts`
- `packages/web-core/src/shared/dialogs/settings/settings/OpenCodeTimeTrackingSettingsSection.tsx`
- `packages/web-core/src/shared/dialogs/settings/settings/SettingsComponents.tsx`
- `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx`
- settings locale JSON files for `en`, `es`, `fr`, `ja`, `ko`, `zh-Hans`, `zh-Hant`

Implemented:

- web API helpers for token list/create/revoke
- host settings section with token metadata list
- optional label creation
- one-time raw token display
- OpenCode config snippet for `@vibe/opencode-time-tracker`
- revoke flow
- install/restart guidance

Review fixes:

- token label input has accessible id/ARIA support
- revoke button retains text while spinner is decorative
- revoked tokens filtered from active list
- clipboard copy awaits/fails gracefully with manual-copy fallback
- `Never` localized through fallback

### Task 12 — Skipped

User explicitly skipped optional manual adjustment backend/UI.

### Task 13 — Automated QA and remaining manual smoke

Automated QA initially found lint blockers. Fixed:

- `crates/server/src/routes/time_tracking.rs`
  - changed bearer helper from `Result<&str, ApiError>` to `Option<&str>` to satisfy Clippy `result_large_err`
- `crates/remote/src/db/issue_time_tracking.rs`
  - replaced `contains_key` + `insert` with `HashMap::entry` to satisfy Clippy `map_entry`

Fresh automated checks passed after these fixes:

- `pnpm run generate-types:check`
- `node scripts/run-remote-without-billing.mjs -- pnpm run remote:generate-types:check`
- `pnpm run prepare-db`
- `pnpm run backend:check`
- `pnpm run check`
- `pnpm run lint`
- `cargo test -p server time_tracking`
- `node scripts/run-remote-without-billing.mjs -- cargo test --manifest-path crates/remote/Cargo.toml time_tracking --lib`
- `pnpm --filter @vibe/opencode-time-tracker run test`
- `pnpm --filter @vibe/opencode-time-tracker run check`
- `pnpm --filter @vibe/opencode-time-tracker run build`
- `pnpm run format`
- `git diff --check`
- `git diff -- crates/remote/Cargo.toml Cargo.toml crates/remote/Cargo.lock Cargo.lock` had no output after wrapper use

Environment blockers confirmed:

- Direct `pnpm run remote:generate-types:check` still fails because this machine cannot fetch private `billing` from `ssh://git@github.com/BloopAI/vibe-kanban-private`.
- Direct `pnpm run remote:prepare-db` still fails because `scripts/prepare-db.sh` cannot find `initdb` on `PATH`.

Remaining manual smoke not executed:

1. Start local app, likely `pnpm run dev`.
2. Create token in Settings → OpenCode time tracking.
3. Configure standalone OpenCode plugin with generated token.
4. Start OpenCode session with a vibe-kanban issue URL.
5. Let plugin post an idle entry.
6. Confirm badge appears after Electric sync.
7. Restart OpenCode and verify same-session binding persists.
8. Start no-URL session and verify no tracking.
9. Switch to another issue URL and verify only future time goes to new issue.

## Deployment question / next-session priority

User asked whether remote deployment is needed. Answer: yes, for end-to-end functionality.

Why remote deploy is needed:

- Plugin posts to local `/api/time-tracking/opencode/entries`.
- Local backend forwards to remote `/v1/time-tracking/opencode/entries`.
- UI totals come from remote/Electric `issue_time_totals`.
- Remote Postgres migration and remote server code must exist for acceptance, aggregation, and sync.

User then asked whether information exists about updating their self-hosted Contabo remote server, saying remote updates were done in prior sessions. A release/QA research task was about to inspect this, but the user requested this handover instead.

Next session should start in `/Users/mihai/Work/vibe-kanban` and first inspect repo/docs/scripts/session notes for Contabo/self-host deployment procedure before attempting any deploy. Search terms to use:

- `Contabo`
- `self-host`
- `remote server`
- `deploy`
- `production`
- `migrate`
- `Electric`
- `Postgres`
- `systemd`
- `docker`
- server hostname/IP if known from user/private env

Likely deployment checklist to confirm:

1. How remote server binary/container is built and deployed.
2. How remote Postgres migrations are applied on Contabo.
3. Whether Electric shape setup runs via migration or deploy hook.
4. Whether remote `shared/remote-types.ts` changes need frontend rebuild only or server deploy too.
5. Whether local app/runtime update is separate from remote server update.
6. How secrets/env for remote DB/Electric are configured.
7. How to roll back if migration or remote route fails.

Do **not** guess deploy commands without repo/server evidence.

## Current worktree notes

- The working tree is intentionally dirty with accumulated feature changes.
- Pre-existing unrelated untracked file remains: `docs/plans/2026-05-09-eff-491-release-local-runtime.md`.
- No commits were made.
- No files were staged.
- No worktree was created.
- Wrapper script `node scripts/run-remote-without-billing.mjs -- ...` is needed for remote Cargo/typegen checks on this machine due private billing dependency access.
- `remote:prepare-db` remains unavailable locally due missing `initdb`; verify on CI or a machine with Postgres tooling.

## Suggested opening prompt for the next session

```text
We are in /Users/mihai/Work/vibe-kanban continuing EFF-818 (OpenCode ticket time tracking).

Read docs/plans/2026-06-19-opencode-ticket-time-tracking-handover.md first.

Tasks 1-11 are implemented, Task 12 was skipped, Task 13 automated QA passes using the wrapper for remote commands. Remaining work:
1. Research repo/session docs for how we deploy/update the self-hosted Contabo remote server.
2. Confirm exact remote migration/deploy procedure for this feature.
3. Run manual local browser/OpenCode smoke if feasible.
4. Do not commit/stage/push unless explicitly asked.
```
