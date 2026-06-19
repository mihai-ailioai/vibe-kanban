# OpenCode Ticket Active Time Tracking Design

## Goal

Track conservative per-ticket OpenCode active time in vibe-kanban without requiring OpenCode to be launched from vibe-kanban.

The user usually starts standalone OpenCode sessions with a prompt such as:

```text
let's start working on this vibe kanban ticket http://127.0.0.1:9000/projects/<project_id>/issues/<issue_id>
```

The feature should accumulate time on the referenced issue while OpenCode is actively working. It should avoid major attribution and overcounting mistakes rather than provide payroll-grade precision.

## Product principles

- Prefer conservative, explainable tracking over fragile precision.
- Count nothing unless the current OpenCode session has an explicit vibe-kanban issue binding.
- Exclude idle time, user-wait time, and permission-wait time.
- Never use a global "last ticket" fallback.
- Never retroactively move time when a session switches tickets.
- Keep tracked time first-class and auditable; do not hide it in `issues.extension_metadata`.

## Current implementation constraints

vibe-kanban currently has a split data model:

- Remote issues are the authoritative ticket model in `crates/remote` and `crates/api-types`.
- Local `tasks` are legacy/local and should not be used for remote ticket time.
- Local workspaces, sessions, and execution processes live in SQLite and are not always involved because OpenCode usually runs separately.
- The frontend issue views receive issue data through project-scoped Electric shapes. They do not currently receive time data.
- Updating the main `issues` row for every timing interval would churn `issues.updated_at` and issue sync semantics.

Because standalone OpenCode is the normal workflow, the MVP must be based on a global OpenCode plugin rather than vibe-kanban's built-in OpenCode executor.

## Architecture

Use a standalone/global OpenCode plugin as the collector and vibe-kanban as the source of truth.

```text
OpenCode user message with issue URL
  -> global OpenCode plugin parses and validates the URL
  -> plugin binds the current OpenCode session to that issue
  -> plugin records conservative active-time intervals
  -> plugin persists pending entries locally before sync
  -> plugin posts idempotent entries to local vibe-kanban
  -> local vibe-kanban validates a narrow plugin token
  -> local vibe-kanban forwards writes to the remote/self-hosted issue API
  -> remote DB stores entries and updates issue totals
  -> frontend reads project-scoped totals through Electric
```

vibe-kanban's built-in OpenCode executor can later emit entries through the same backend path, but it is not the MVP source of truth.

## OpenCode session binding

The plugin detects vibe-kanban issue URLs in user messages:

```text
<origin>/projects/<project_id>/issues/<issue_id>
```

For example:

```text
http://127.0.0.1:9000/projects/b6eee4fd-8ea2-4945-ada9-0b17115ef642/issues/5e785426-ac88-44c4-8231-066ef6dd02bc
```

This yields:

```json
{
  "origin": "http://127.0.0.1:9000",
  "project_id": "b6eee4fd-8ea2-4945-ada9-0b17115ef642",
  "issue_id": "5e785426-ac88-44c4-8231-066ef6dd02bc"
}
```

Binding rules:

1. A new OpenCode session starts untracked.
2. The first valid vibe-kanban issue URL binds that OpenCode session to the issue.
3. The binding is persisted by OpenCode session ID so it survives OpenCode restarts.
4. The binding never applies to any other OpenCode session.
5. If a later user message in the same session contains a different valid issue URL, future time switches to the new issue.
6. Previously synced or pending entries stay assigned to the issue they were recorded for.
7. If URL validation fails, the plugin keeps the previous binding and logs or surfaces a warning.

The plugin should not infer tickets from titles, descriptions, or fuzzy matches in the MVP.

## Active-time tracking heuristic

The plugin should use a small state machine:

```text
unbound -> bound_idle -> bound_active -> bound_waiting -> bound_idle
```

State meanings:

- `unbound`: no valid issue URL has been seen in this OpenCode session; no time is tracked.
- `bound_idle`: the session has an issue binding, but OpenCode is not currently working.
- `bound_active`: OpenCode is generating, using tools, reading, editing, running commands, or otherwise doing assistant work.
- `bound_waiting`: OpenCode is blocked on user input, permission approval, or an interactive prompt.

MVP event rules:

- Bind or switch issue before timing the user turn that contains the URL.
- Start an active interval on the user message that triggers assistant work when the session is bound.
- Keep the interval active across model and tool activity.
- Close or pause the interval on permission/user-wait events.
- Resume after permission is granted or user-wait ends.
- Close the interval on `session.idle`, `session.error`, `session.deleted`, or OpenCode shutdown.

Overcounting protection:

- Persist `active_since` so a restart can recover a bounded interval.
- Cap recovered or suspicious intervals, for example at 30-60 minutes for MVP.
- Reject or warn on very large intervals rather than silently counting them.
- Prefer dropping uncertain time over assigning it to the wrong issue.

The UI should label the value as `OpenCode active time` or `Tracked active time`, not exact time spent.

## Plugin persistence and retry

The plugin should store state locally under its OpenCode plugin state directory, keyed by OpenCode session ID.

Session state should include:

```json
{
  "opencode_session_id": "...",
  "current_binding": {
    "origin": "http://127.0.0.1:9000",
    "project_id": "...",
    "issue_id": "..."
  },
  "active_since": null,
  "pending_entries": []
}
```

Every completed interval should be written to the pending queue before the plugin attempts a network request. The plugin retries pending entries on startup, on new user messages, and when sessions become idle or are deleted.

Each interval entry has a client-generated UUID `entry_id`. The backend uses this as the idempotency key so retry storms cannot double-count time.

## Authentication model

The OpenCode plugin is a separate local process making HTTP requests into vibe-kanban. It needs a narrow credential, but it should not receive the user's normal remote access token or browser session cookie.

vibe-kanban should generate a plugin-specific local token such as:

```text
vktt_<opaque-random-secret>
```

The plugin sends it as:

```http
Authorization: Bearer vktt_<opaque-random-secret>
```

Token rules:

- Store only a hash of the token in vibe-kanban.
- Scope the token to `time_tracking:write`.
- Authorise only the local plugin ingestion endpoint initially.
- Do not allow issue edits, issue deletion, project management, or normal user-data access.
- Attribute accepted writes to the currently signed-in remote user on the local vibe-kanban instance.
- If local vibe-kanban is logged out or cannot refresh remote credentials, return an auth/dependency error and let the plugin keep entries pending.

Setup should eventually be generated from a settings page:

1. Create an OpenCode time-tracking token.
2. Show the token once.
3. Show the local server URL.
4. Show an OpenCode plugin configuration snippet or install command.

## Storage design

Store time tracking in remote Postgres as first-class issue-domain data.

Do not store totals in `issues.extension_metadata`, and do not update the main `issues` row for every interval.

### `issue_time_entries`

This table is the immutable audit log and idempotency layer.

Recommended fields:

```text
entry_id UUID PRIMARY KEY
project_id UUID NOT NULL
issue_id UUID NOT NULL
user_id UUID NOT NULL
source TEXT NOT NULL              -- opencode | manual
kind TEXT NOT NULL                -- active_interval | manual_adjustment
started_at TIMESTAMPTZ NULL
ended_at TIMESTAMPTZ NULL
duration_ms BIGINT NOT NULL
source_session_id TEXT NULL       -- opaque OpenCode session ID
note TEXT NULL
metadata JSONB NOT NULL DEFAULT '{}'
payload_hash TEXT NOT NULL
created_at TIMESTAMPTZ NOT NULL
received_at TIMESTAMPTZ NOT NULL DEFAULT now()
```

For OpenCode entries, `duration_ms` is positive. Manual adjustments may be positive or negative.

### `issue_time_totals`

This table is the efficient UI aggregate.

Recommended fields:

```text
project_id UUID NOT NULL
issue_id UUID NOT NULL
opencode_active_ms BIGINT NOT NULL DEFAULT 0
manual_adjustment_ms BIGINT NOT NULL DEFAULT 0
total_ms BIGINT NOT NULL DEFAULT 0
entry_count BIGINT NOT NULL DEFAULT 0
last_entry_at TIMESTAMPTZ NULL
updated_at TIMESTAMPTZ NOT NULL
PRIMARY KEY (project_id, issue_id)
```

Only `issue_time_totals` should be Electric-synced for the MVP. The full entries table can be queried on demand later if the UI needs an audit view.

Frontend clients should treat a missing total row as zero.

## API contract

The plugin writes to local vibe-kanban. Local vibe-kanban validates the plugin token and forwards to the remote/self-hosted API using normal remote credentials that stay inside vibe-kanban.

### Local plugin endpoint

```http
POST /api/time-tracking/opencode/entries
Authorization: Bearer vktt_<token>
Content-Type: application/json
```

Request:

```json
{
  "schema_version": 1,
  "entries": [
    {
      "entry_id": "uuid",
      "project_id": "uuid",
      "issue_id": "uuid",
      "source_session_id": "opencode-session-id",
      "started_at": "2026-06-18T10:00:00Z",
      "ended_at": "2026-06-18T10:03:07Z",
      "duration_ms": 187000,
      "metadata": {
        "opencode_version": "...",
        "plugin_version": "...",
        "idle_policy": "conservative-v1"
      }
    }
  ]
}
```

Response:

```json
{
  "txid": 123,
  "results": [
    {
      "entry_id": "uuid",
      "status": "created",
      "project_id": "uuid",
      "issue_id": "uuid",
      "duration_ms": 187000
    }
  ],
  "updated_totals": []
}
```

Remote equivalents should live under `/v1/time-tracking/...`:

```http
POST /v1/time-tracking/opencode/entries
GET  /v1/time-tracking/issues/:issue_id
POST /v1/time-tracking/issues/:issue_id/adjustments
```

Local proxied UI routes can mirror them under `/api/time-tracking/...`.

### Validation

The server must validate:

- `schema_version` is `1`.
- Batch size is bounded, for example 1-100 entries.
- `entry_id` is a UUID.
- `project_id` and `issue_id` exist and match.
- The authenticated user has access to the project.
- `started_at < ended_at`.
- `duration_ms > 0` for OpenCode active intervals.
- `duration_ms` does not exceed the wall-clock interval plus a small tolerance.
- A single interval does not exceed the configured cap, for example six hours server-side.
- `metadata` size is bounded.

### Idempotency

Use `entry_id` as the idempotency key.

On insert, compute `payload_hash` from immutable fields such as:

- `project_id`
- `issue_id`
- `source`
- `kind`
- `started_at`
- `ended_at`
- `duration_ms`
- `source_session_id`

Duplicate handling:

- If `entry_id` already exists and the hash matches, return success with `status: "duplicate"` and do not update totals.
- If `entry_id` already exists and the hash differs, return `409 idempotency_conflict` and do not mutate anything.
- Insert entries and update totals in one transaction.
- Return the transaction ID so Electric clients can catch up consistently.

## Electric sync and frontend data flow

Add a project-scoped Electric shape for `issue_time_totals`:

```text
/shape/project/{project_id}/issue_time_totals
```

The frontend project provider should subscribe to this shape alongside issues, statuses, tags, assignees, relationships, pull requests, and workspaces.

The shape should be optional/non-fatal during rollout. If it fails or returns no row for an issue, the UI should display no badge or zero time rather than blocking the board.

Do not add time fields to the `Issue` type for the MVP. Keep issue mutation contracts stable.

## UI behaviour

Ticket time should be visible but secondary.

Kanban cards and list rows:

- Show a compact time badge when the total is greater than zero.
- Hide the badge for zero or missing totals.
- Use a tooltip such as `Tracked OpenCode active time`.
- Format compactly: `<1m`, `12m`, `1h 20m`, `2d 3h`.

Issue detail panel:

- Show a read-only total near issue metadata.
- Include a short explanation:
  > Time when OpenCode was actively working while this session was bound to the ticket. Idle and approval-wait time are excluded.
- Add manual adjustments either in the MVP if simple or as a follow-up.

Manual adjustments:

- Use entries with `source = manual` and `kind = manual_adjustment`.
- Allow positive and negative durations.
- Require a short note.
- Never mutate totals directly outside the normal entry aggregation path.

Settings/admin:

- Add an `OpenCode time tracking` section.
- Generate and revoke plugin tokens.
- Show plugin install/configuration instructions.
- Optionally show last successful sync time.

## Error handling

Plugin behaviour:

- If there is no binding, track nothing.
- If the backend is offline, keep pending entries locally.
- If auth fails, keep pending entries and surface a setup warning.
- If issue validation fails for a new URL, keep the previous binding and warn.
- If an interval is suspiciously long, cap or drop it with metadata explaining why.

Server behaviour:

- Treat duplicate matching entries as successful no-ops.
- Reject duplicate conflicting entries with `409`.
- Reject entries for invalid issue/project pairs.
- Reject entries that violate timing caps.
- Keep totals recomputable from entries for future repair jobs.

## Non-goals for the MVP

- Payroll-grade time tracking.
- Fuzzy issue-title inference.
- Global last-ticket fallback across OpenCode sessions.
- Full per-entry audit UI.
- Session breakdowns on the issue UI.
- Tracking non-OpenCode work.
- Updating `issues.updated_at` for time-only changes.

## Open implementation choices

- Exact OpenCode plugin package name and distribution path.
- Exact local plugin state directory path.
- Exact MVP interval cap values.
- Whether manual adjustments ship in the first implementation slice or immediately after card/detail display.
- Whether the built-in vibe-kanban OpenCode executor emits entries in the first implementation or a later follow-up.
