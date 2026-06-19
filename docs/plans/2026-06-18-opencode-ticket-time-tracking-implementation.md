# OpenCode Ticket Time Tracking Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add conservative per-ticket OpenCode active-time tracking for standalone OpenCode sessions that reference vibe-kanban issue URLs.

**Architecture:** A global OpenCode plugin detects vibe-kanban issue URLs, binds the current OpenCode session to one issue at a time, records conservative active intervals, persists pending entries locally, and posts idempotent batches to local vibe-kanban. The local backend validates a narrow plugin token and forwards entries to the remote/self-hosted issue API, where Postgres stores immutable entries and Electric syncs project-scoped totals to the frontend.

**Tech Stack:** Rust, Axum, SQLx/Postgres, SQLx/SQLite, ts-rs generated TypeScript contracts, ElectricSQL shapes, React/TypeScript, pnpm workspace package for the OpenCode plugin.

---

## Reference material

- Design doc: `docs/plans/2026-06-18-opencode-ticket-time-tracking-design.md`
- Remote crate guide: `crates/remote/AGENTS.md`
- Shared remote types generator: `crates/remote/src/bin/generate_types.rs`
- Local shared types generator: `crates/server/src/bin/generate_types.rs`
- Generated files: do not edit `shared/remote-types.ts` or `shared/types.ts` manually.
- Commit checkpoints below are optional. Do not run `git commit` unless the user explicitly requests commits.

---

### Task 1: Add shared time-tracking API contracts

**Files:**
- Create: `crates/api-types/src/time_tracking.rs`
- Modify: `crates/api-types/src/lib.rs`
- Modify: `crates/remote/src/bin/generate_types.rs`

**Step 1: Create the shared contract module**

Create `crates/api-types/src/time_tracking.rs` with the first version of the typed contract:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IssueTimeTotal {
    pub project_id: Uuid,
    pub issue_id: Uuid,
    pub opencode_active_ms: i64,
    pub manual_adjustment_ms: i64,
    pub total_ms: i64,
    pub entry_count: i64,
    pub last_entry_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeEntriesRequest {
    pub schema_version: i32,
    pub entries: Vec<OpenCodeTimeEntryInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenCodeTimeEntryInput {
    pub entry_id: Uuid,
    pub project_id: Uuid,
    pub issue_id: Uuid,
    pub source_session_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_ms: i64,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TimeEntryStatus {
    Created,
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenCodeTimeEntryResult {
    pub entry_id: Uuid,
    pub status: TimeEntryStatus,
    pub project_id: Uuid,
    pub issue_id: Uuid,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeEntriesResponse {
    pub txid: i64,
    pub results: Vec<OpenCodeTimeEntryResult>,
    pub updated_totals: Vec<IssueTimeTotal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GetIssueTimeTrackingResponse {
    pub total: Option<IssueTimeTotal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateIssueTimeAdjustmentRequest {
    pub entry_id: Option<Uuid>,
    pub duration_ms: i64,
    pub note: String,
}
```

**Step 2: Export the module**

In `crates/api-types/src/lib.rs`, add:

```rust
pub mod time_tracking;
pub use time_tracking::*;
```

Keep this near the other issue-domain exports.

**Step 3: Add the remote TypeScript declarations**

In `crates/remote/src/bin/generate_types.rs`:

1. Import the new types from `api_types`.
2. Add these `::decl()` calls to `type_decls`:

```rust
IssueTimeTotal::decl(),
CreateOpenCodeTimeEntriesRequest::decl(),
OpenCodeTimeEntryInput::decl(),
TimeEntryStatus::decl(),
OpenCodeTimeEntryResult::decl(),
CreateOpenCodeTimeEntriesResponse::decl(),
GetIssueTimeTrackingResponse::decl(),
CreateIssueTimeAdjustmentRequest::decl(),
```

**Step 4: Generate remote types**

Run:

```bash
pnpm run remote:generate-types
```

Expected: `shared/remote-types.ts` is regenerated with the new time-tracking types. No manual edits.

**Step 5: Checkpoint**

Run:

```bash
pnpm run remote:generate-types:check
```

Expected: PASS.

---

### Task 2: Add the remote Postgres schema

**Files:**
- Create: `crates/remote/migrations/20260618000000_issue_time_tracking.sql`

**Step 1: Create the migration**

Add:

```sql
CREATE TABLE issue_time_entries (
  entry_id UUID PRIMARY KEY,
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  issue_id UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  source TEXT NOT NULL CHECK (source IN ('opencode', 'manual')),
  kind TEXT NOT NULL CHECK (kind IN ('active_interval', 'manual_adjustment')),
  started_at TIMESTAMPTZ,
  ended_at TIMESTAMPTZ,
  duration_ms BIGINT NOT NULL CHECK (duration_ms <> 0),
  source_session_id TEXT,
  note TEXT,
  metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
  payload_hash TEXT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL,
  received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_issue_time_entries_project_issue
  ON issue_time_entries(project_id, issue_id);

CREATE INDEX idx_issue_time_entries_user_received
  ON issue_time_entries(user_id, received_at DESC);

CREATE INDEX idx_issue_time_entries_source_kind
  ON issue_time_entries(source, kind);

CREATE TABLE issue_time_totals (
  project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  issue_id UUID NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
  opencode_active_ms BIGINT NOT NULL DEFAULT 0,
  manual_adjustment_ms BIGINT NOT NULL DEFAULT 0,
  total_ms BIGINT NOT NULL DEFAULT 0,
  entry_count BIGINT NOT NULL DEFAULT 0,
  last_entry_at TIMESTAMPTZ,
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  PRIMARY KEY (project_id, issue_id)
);

CREATE INDEX idx_issue_time_totals_project_id
  ON issue_time_totals(project_id);

ALTER TABLE issue_time_totals REPLICA IDENTITY FULL;
SELECT electric_sync_table('public', 'issue_time_totals');
```

Do not electrify `issue_time_entries` for the MVP.

**Step 2: Prepare remote DB metadata**

Run:

```bash
pnpm run remote:prepare-db
```

Expected: SQLx remote metadata is updated for the new migration.

---

### Task 3: Implement remote time-entry repository logic

**Files:**
- Create: `crates/remote/src/db/issue_time_tracking.rs`
- Modify: `crates/remote/src/db/mod.rs`

**Step 1: Add failing unit tests for pure helpers**

In `crates/remote/src/db/issue_time_tracking.rs`, start with tests for helper behaviour:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_payload_hash_is_stable_for_same_entry() {
        // Build an OpenCodeTimeEntryInput and assert two hash calls match.
    }

    #[test]
    fn opencode_duration_must_be_positive() {
        // Validate duration_ms = 0 returns a validation error.
    }

    #[test]
    fn opencode_duration_cannot_exceed_interval_plus_tolerance() {
        // Validate duration_ms greater than ended_at - started_at + tolerance fails.
    }
}
```

**Step 2: Add repository types and helper functions**

Implement:

```rust
pub struct IssueTimeTrackingRepository;

#[derive(Debug, thiserror::Error)]
pub enum IssueTimeTrackingError {
    #[error("invalid_request: {0}")]
    InvalidRequest(&'static str),
    #[error("idempotency_conflict")]
    IdempotencyConflict,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}
```

Add pure helpers:

```rust
fn canonical_payload_hash(input: &OpenCodeTimeEntryInput) -> String;
fn validate_opencode_entry(input: &OpenCodeTimeEntryInput) -> Result<(), IssueTimeTrackingError>;
```

Use `sha2` and `hex`, already present in `crates/remote/Cargo.toml`.

**Step 3: Implement `create_opencode_entries`**

Add:

```rust
impl IssueTimeTrackingRepository {
    pub async fn create_opencode_entries(
        pool: &sqlx::PgPool,
        user_id: uuid::Uuid,
        request: api_types::CreateOpenCodeTimeEntriesRequest,
    ) -> Result<api_types::CreateOpenCodeTimeEntriesResponse, IssueTimeTrackingError> {
        // validate schema_version == 1
        // validate batch size 1..=100
        // validate every entry
        // begin one transaction
        // verify issue_id belongs to project_id
        // select existing rows by entry_id
        // duplicate same hash => duplicate result
        // duplicate different hash => rollback with IdempotencyConflict
        // insert created rows
        // upsert issue_time_totals for created rows only
        // return txid and updated totals
    }
}
```

**Step 4: Register the module**

In `crates/remote/src/db/mod.rs`, add:

```rust
pub mod issue_time_tracking;
```

**Step 5: Run targeted tests**

Run:

```bash
cargo test --manifest-path crates/remote/Cargo.toml issue_time_tracking
```

Expected: helper tests pass. DB-backed tests may require `pnpm run remote:prepare-db` first.

---

### Task 4: Add remote routes and Electric shape for totals

**Files:**
- Create: `crates/remote/src/routes/time_tracking.rs`
- Modify: `crates/remote/src/routes/mod.rs`
- Modify: `crates/remote/src/shapes.rs`
- Modify: `crates/remote/src/shape_routes.rs`
- Modify: `crates/remote/src/bin/generate_types.rs`

**Step 1: Add the protected remote route**

Create `crates/remote/src/routes/time_tracking.rs`:

```rust
use api_types::CreateOpenCodeTimeEntriesRequest;
use axum::{Json, Router, extract::{Extension, State}, http::StatusCode, routing::post};

use crate::{
    AppState,
    auth::RequestContext,
    db::issue_time_tracking::{IssueTimeTrackingError, IssueTimeTrackingRepository},
    routes::error::ErrorResponse,
};

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/time-tracking/opencode/entries",
        post(create_opencode_entries),
    )
}

async fn create_opencode_entries(
    State(state): State<AppState>,
    Extension(ctx): Extension<RequestContext>,
    Json(payload): Json<CreateOpenCodeTimeEntriesRequest>,
) -> Result<Json<api_types::CreateOpenCodeTimeEntriesResponse>, ErrorResponse> {
    let response = IssueTimeTrackingRepository::create_opencode_entries(
        state.pool(),
        ctx.user.id,
        payload,
    )
    .await
    .map_err(map_time_tracking_error)?;

    Ok(Json(response))
}

fn map_time_tracking_error(error: IssueTimeTrackingError) -> ErrorResponse {
    match error {
        IssueTimeTrackingError::IdempotencyConflict => {
            ErrorResponse::new(StatusCode::CONFLICT, "idempotency_conflict")
        }
        IssueTimeTrackingError::InvalidRequest(message) => {
            ErrorResponse::new(StatusCode::BAD_REQUEST, message)
        }
        IssueTimeTrackingError::Sqlx(error) => {
            tracing::error!(?error, "issue time tracking DB error");
            ErrorResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "failed to record time entry")
        }
    }
}
```

**Step 2: Merge the route**

In `crates/remote/src/routes/mod.rs`:

```rust
mod time_tracking;
```

Add to `v1_protected`:

```rust
.merge(time_tracking::router())
```

Do not add the route to `v1_public`.

**Step 3: Add the Electric shape**

In `crates/remote/src/shapes.rs`, import `IssueTimeTotal` and add:

```rust
pub const PROJECT_ISSUE_TIME_TOTALS_SHAPE: ShapeDefinition<IssueTimeTotal> = crate::define_shape!(
    name: "PROJECT_ISSUE_TIME_TOTALS_SHAPE",
    table: "issue_time_totals",
    where_clause: r#""project_id" = $1"#,
    url: "/shape/project/{project_id}/issue_time_totals",
    params: ["project_id"],
);
```

**Step 4: Add the fallback route**

In `crates/remote/src/shape_routes.rs`:

1. Import `IssueTimeTotal` and `IssueTimeTrackingRepository`.
2. Add a response type:

```rust
#[derive(Debug, Serialize)]
struct ListIssueTimeTotalsResponse {
    issue_time_totals: Vec<IssueTimeTotal>,
}
```

3. Add a `ShapeRoute::new(...)` entry with `ShapeScope::Project` and fallback URL `/fallback/issue_time_totals`.
4. Add `fallback_list_issue_time_totals` that calls `ensure_project_access` and repository `list_totals_by_project`.

**Step 5: Regenerate remote types**

Run:

```bash
pnpm run remote:generate-types
pnpm run remote:generate-types:check
```

Expected: `shared/remote-types.ts` includes `IssueTimeTotal` and `PROJECT_ISSUE_TIME_TOTALS_SHAPE`.

---

### Task 5: Add local plugin token persistence

**Files:**
- Create: `crates/db/migrations/20260618000000_opencode_time_tracking_tokens.sql`
- Create: `crates/db/src/models/time_tracking_token.rs`
- Modify: `crates/db/src/models/mod.rs`
- Modify: `crates/db/Cargo.toml`

**Step 1: Create the SQLite migration**

Add:

```sql
CREATE TABLE opencode_time_tracking_tokens (
  id BLOB PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  scope TEXT NOT NULL CHECK (scope = 'time_tracking:write'),
  label TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
  last_used_at TEXT,
  revoked_at TEXT
);

CREATE INDEX idx_opencode_time_tracking_tokens_hash
  ON opencode_time_tracking_tokens(token_hash)
  WHERE revoked_at IS NULL;
```

**Step 2: Add dependencies if needed**

In `crates/db/Cargo.toml`, add direct dependencies if token generation/hash helpers live in the DB model:

```toml
base64 = "0.22"
rand = { version = "0.8", features = ["std"] }
sha2 = "0.10"
hex = "0.4"
```

If the generation helpers live in `server`, keep random generation in `server` and only store hashes in `db`.

**Step 3: Create the model**

Create `crates/db/src/models/time_tracking_token.rs` with:

```rust
pub const OPENCODE_TIME_TRACKING_SCOPE: &str = "time_tracking:write";
pub const OPENCODE_TIME_TRACKING_TOKEN_PREFIX: &str = "vktt_";

pub struct OpencodeTimeTrackingToken {
    pub id: uuid::Uuid,
    pub token_hash: String,
    pub scope: String,
    pub label: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

Add methods:

```rust
pub async fn create(pool: &sqlx::SqlitePool, token_hash: &str, label: Option<&str>) -> Result<Self, sqlx::Error>;
pub async fn find_active_by_hash(pool: &sqlx::SqlitePool, token_hash: &str) -> Result<Option<Self>, sqlx::Error>;
pub async fn list(pool: &sqlx::SqlitePool) -> Result<Vec<Self>, sqlx::Error>;
pub async fn mark_used(pool: &sqlx::SqlitePool, id: uuid::Uuid) -> Result<(), sqlx::Error>;
pub async fn revoke(pool: &sqlx::SqlitePool, id: uuid::Uuid) -> Result<(), sqlx::Error>;
```

**Step 4: Register the model**

In `crates/db/src/models/mod.rs`, add:

```rust
pub mod time_tracking_token;
```

**Step 5: Add model tests**

Add tests for:

- create then find active by hash
- revoked tokens are not returned by `find_active_by_hash`
- `mark_used` updates `last_used_at`

Run:

```bash
cargo test -p db time_tracking_token
pnpm run prepare-db
```

Expected: DB model tests pass and SQLx local metadata updates.

---

### Task 6: Add local time-tracking routes and remote forwarding

**Files:**
- Create: `crates/server/src/routes/time_tracking.rs`
- Modify: `crates/server/src/routes/mod.rs`
- Modify: `crates/services/src/services/remote_client.rs`
- Modify: `crates/server/src/bin/generate_types.rs`
- Optionally create local response/request types in `crates/api-types/src/time_tracking.rs`

**Step 1: Add remote-client forwarding**

In `crates/services/src/services/remote_client.rs`, import the request/response types and add:

```rust
pub async fn create_opencode_time_entries(
    &self,
    request: &CreateOpenCodeTimeEntriesRequest,
) -> Result<CreateOpenCodeTimeEntriesResponse, RemoteClientError> {
    self.post_json("/v1/time-tracking/opencode/entries", request).await
}
```

Use the existing helper style in `remote_client.rs`; if there is no generic `post_json` helper, follow nearby create/update methods and keep token refresh inside `require_token()`.

**Step 2: Define local token-management contracts**

Add shared types for UI setup:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeTrackingTokenRequest {
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeTrackingTokenResponse {
    pub id: Uuid,
    pub token: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OpenCodeTimeTrackingTokenSummary {
    pub id: Uuid,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}
```

Regenerate local shared types with `pnpm run generate-types` after wiring `crates/server/src/bin/generate_types.rs`.

**Step 3: Create the local routes**

Create `crates/server/src/routes/time_tracking.rs` with routes:

```text
POST   /time-tracking/opencode/entries
POST   /time-tracking/opencode/tokens
GET    /time-tracking/opencode/tokens
DELETE /time-tracking/opencode/tokens/{token_id}
```

Handler flow for `POST /time-tracking/opencode/entries`:

1. Extract `Authorization: Bearer vktt_...`.
2. Reject missing, malformed, or wrong-prefix tokens with `401`.
3. Hash the raw token and find a non-revoked token with `time_tracking:write`.
4. Forward the request with `deployment.remote_client()?`.
5. Mark `last_used_at` only after the remote accepts the batch.
6. Return the remote response body.

Token creation flow:

1. Generate 32+ random bytes.
2. Encode as base64url or hex.
3. Prefix with `vktt_`.
4. Store only `sha256(token)`.
5. Return the token once.

**Step 4: Mount the local route**

In `crates/server/src/routes/mod.rs`:

```rust
pub mod time_tracking;
```

Merge inside `relay_signed_routes` so it remains under `/api` and benefits from existing origin handling:

```rust
.merge(time_tracking::router(&deployment))
```

A standalone OpenCode plugin normally sends no `Origin` header; existing `validate_origin` should allow that. Do not require relay request signing for plugin-token auth.

**Step 5: Test local auth and forwarding**

Add route tests for:

- missing bearer token returns `401`
- wrong prefix returns `401`
- revoked token returns `401`
- valid token calls remote client and updates `last_used_at`

Run:

```bash
cargo test -p server time_tracking
pnpm run generate-types
pnpm run generate-types:check
```

Expected: route tests pass and `shared/types.ts` is up to date.

---

### Task 7: Add the OpenCode plugin package

**Files:**
- Create: `packages/opencode-time-tracker/package.json`
- Create: `packages/opencode-time-tracker/tsconfig.json`
- Create: `packages/opencode-time-tracker/src/index.ts`
- Create: `packages/opencode-time-tracker/src/url.ts`
- Create: `packages/opencode-time-tracker/src/state.ts`
- Create: `packages/opencode-time-tracker/src/time.ts`
- Create: `packages/opencode-time-tracker/src/client.ts`
- Create: `packages/opencode-time-tracker/src/*.test.ts` as needed
- Modify: root `package.json` only if adding aggregate check/build scripts is desired

**Step 1: Create the package manifest**

Use the workspace's existing `packages/*` pattern. Create:

```json
{
  "name": "@vibe/opencode-time-tracker",
  "version": "0.1.0",
  "type": "module",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "files": ["dist"],
  "scripts": {
    "build": "tsc -p tsconfig.json",
    "check": "tsc --noEmit -p tsconfig.json",
    "test": "vitest run"
  },
  "dependencies": {
    "@opencode-ai/plugin": "latest"
  },
  "devDependencies": {
    "typescript": "^5.7.0",
    "vitest": "^3.0.0",
    "@types/node": "^20.0.0"
  }
}
```

Pin `@opencode-ai/plugin` before publishing if the current registry package exposes a stable version.

**Step 2: Add URL parsing tests first**

In `src/url.test.ts`, test:

- parses `http://127.0.0.1:9000/projects/<uuid>/issues/<uuid>`
- parses `https://host/projects/<uuid>/issues/<uuid>`
- ignores non-vibe URLs
- returns all valid URLs in a message so the latest one can win

**Step 3: Implement URL parsing**

In `src/url.ts`, export:

```ts
export interface VibeIssueUrl {
  origin: string;
  projectId: string;
  issueId: string;
  url: string;
}

export function findVibeIssueUrls(text: string): VibeIssueUrl[];
```

Use `new URL(candidate)` instead of only regex slicing, and validate path segments are exactly `projects/<uuid>/issues/<uuid>`.

**Step 4: Add state persistence tests**

In `src/state.test.ts`, verify:

- session state is keyed by OpenCode session ID
- persisted binding reloads after process restart
- pending entries survive reload
- there is no global fallback binding

**Step 5: Implement state persistence**

In `src/state.ts`, export helpers that read/write JSON files under a plugin state directory. Keep writes atomic by writing `*.tmp` then renaming.

**Step 6: Add active-time state-machine tests**

In `src/time.test.ts`, test:

- unbound sessions create no entries
- bound active interval closes on idle
- waiting pauses or closes the interval
- suspicious intervals are capped or dropped according to the chosen policy
- ticket switch affects future entries only

**Step 7: Implement active-time state machine**

In `src/time.ts`, implement:

```ts
type TrackingState = 'unbound' | 'bound_idle' | 'bound_active' | 'bound_waiting';
```

Export operations such as `bindIssue`, `startActive`, `enterWaiting`, `resumeActive`, and `closeActiveInterval`.

**Step 8: Implement the local vibe-kanban client**

In `src/client.ts`, implement:

```ts
export async function postEntries(origin: string, token: string, entries: PendingEntry[]): Promise<PostResult>;
```

POST to:

```text
{origin}/api/time-tracking/opencode/entries
```

Use `Authorization: Bearer ${token}`.

**Step 9: Implement the plugin entrypoint**

In `src/index.ts`, export an OpenCode plugin function. Hook into:

- `chat.message` or the equivalent event containing user messages for URL detection
- `event` for `session.status`, `session.idle`, and `session.deleted`
- `permission.ask` / permission events if available
- `tool.execute.before` and `tool.execute.after` if needed to keep active state fresh

The plugin options should support per-origin tokens:

```ts
interface VibeKanbanTimeTrackerOptions {
  servers: Record<string, { token: string }>;
  maxRecoveredIntervalMs?: number;
}
```

**Step 10: Run plugin checks**

Run:

```bash
pnpm --filter @vibe/opencode-time-tracker run test
pnpm --filter @vibe/opencode-time-tracker run check
pnpm --filter @vibe/opencode-time-tracker run build
```

Expected: parser, state, timing, client unit tests pass and package builds.

---

### Task 8: Wire issue time totals into project context

**Files:**
- Modify: `packages/web-core/src/shared/providers/remote/ProjectProvider.tsx`
- Modify: `packages/web-core/src/shared/hooks/useProjectContext.ts`
- Optional modify: `packages/web-core/src/shared/integrations/electric/hooks.ts`

**Step 1: Extend context types**

In `useProjectContext.ts`, import `IssueTimeTotal` and add:

```ts
issueTimeTotals: IssueTimeTotal[];
issueTimeTotalsByIssueId: Map<string, IssueTimeTotal>;
getIssueTimeTotal: (issueId: string) => IssueTimeTotal | undefined;
```

**Step 2: Subscribe to the new shape**

In `ProjectProvider.tsx`, import `PROJECT_ISSUE_TIME_TOTALS_SHAPE` and `IssueTimeTotal`.

Add:

```ts
const issueTimeTotalsResult = useShape(PROJECT_ISSUE_TIME_TOTALS_SHAPE, params, {
  enabled,
});
```

Do not add `issueTimeTotalsResult.isLoading` to board readiness.

**Step 3: Keep rollout non-fatal**

Do not include `issueTimeTotalsResult.error` in the provider's fatal `error` value. If `useShape` registers global sync errors for this new shape during mixed-version rollout, add an option to `useShape` such as:

```ts
suppressErrorRegistration?: boolean;
```

Then use it for `PROJECT_ISSUE_TIME_TOTALS_SHAPE`.

**Step 4: Build the lookup map**

Add:

```ts
const issueTimeTotalsByIssueId = useMemo(() => {
  const map = new Map<string, IssueTimeTotal>();
  for (const total of issueTimeTotalsResult.data) {
    map.set(total.issue_id, total);
  }
  return map;
}, [issueTimeTotalsResult.data]);
```

Expose it in the context value.

**Step 5: Type-check web core**

Run:

```bash
pnpm --filter @vibe/web-core run check
```

Expected: PASS.

---

### Task 9: Add formatting helper and time badge component

**Files:**
- Create: `packages/web-core/src/shared/lib/issueTime.ts`
- Create: `packages/web-core/src/shared/lib/issueTime.test.ts`
- Create: `packages/ui/src/components/IssueTimeBadge.tsx`

**Step 1: Write formatter tests**

Create `issueTime.test.ts` with:

```ts
import { describe, expect, it } from 'vitest';
import { formatIssueActiveTime } from './issueTime';

describe('formatIssueActiveTime', () => {
  it('hides zero totals', () => {
    expect(formatIssueActiveTime(0)).toBeNull();
  });

  it('formats sub-minute totals', () => {
    expect(formatIssueActiveTime(1)).toBe('<1m');
  });

  it('formats minutes', () => {
    expect(formatIssueActiveTime(12 * 60_000)).toBe('12m');
  });

  it('formats hours and minutes', () => {
    expect(formatIssueActiveTime(80 * 60_000)).toBe('1h 20m');
  });

  it('formats days and hours', () => {
    expect(formatIssueActiveTime((2 * 24 + 3) * 60 * 60_000)).toBe('2d 3h');
  });
});
```

**Step 2: Implement formatter helpers**

Create `issueTime.ts`:

```ts
export function getIssueTotalMs(
  total?: { total_ms: bigint | number | string } | null
): number {
  if (!total) return 0;
  return Number(total.total_ms);
}

export function formatIssueActiveTime(totalMs: number): string | null {
  if (totalMs <= 0) return null;
  if (totalMs < 60_000) return '<1m';

  const totalMinutes = Math.floor(totalMs / 60_000);
  const minutes = totalMinutes % 60;
  const totalHours = Math.floor(totalMinutes / 60);
  const hours = totalHours % 24;
  const days = Math.floor(totalHours / 24);

  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (totalHours > 0) return minutes > 0 ? `${totalHours}h ${minutes}m` : `${totalHours}h`;
  return `${minutes}m`;
}
```

**Step 3: Add the UI badge**

Create `IssueTimeBadge.tsx`:

```tsx
import { ClockIcon } from '@phosphor-icons/react';
import { cn } from '../lib/cn';

export interface IssueTimeBadgeProps {
  label: string;
  tooltip?: string;
  className?: string;
}

export function IssueTimeBadge({
  label,
  tooltip = 'Tracked OpenCode active time',
  className,
}: IssueTimeBadgeProps) {
  return (
    <span
      title={tooltip}
      className={cn(
        'inline-flex items-center gap-half rounded-sm bg-secondary px-half py-px text-xs text-low',
        className
      )}
    >
      <ClockIcon className="size-icon-xs" weight="bold" />
      {label}
    </span>
  );
}
```

**Step 4: Run tests and type checks**

Run:

```bash
pnpm --filter @vibe/web-core run test -- src/shared/lib/issueTime.test.ts
pnpm --filter @vibe/web-core run check
pnpm --filter @vibe/ui run check
```

Expected: PASS.

---

### Task 10: Render time on cards, lists, and detail panel

**Files:**
- Modify: `packages/ui/src/components/KanbanCardContent.tsx`
- Modify: `packages/ui/src/components/IssueListView.tsx`
- Modify: `packages/ui/src/components/IssueListSection.tsx`
- Modify: `packages/ui/src/components/IssueListRow.tsx`
- Modify: `packages/ui/src/components/KanbanIssuePanel.tsx`
- Modify: `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx`
- Modify: `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx`

**Step 1: Add card prop and render badge**

In `KanbanCardContent.tsx`, import `IssueTimeBadge` and add prop:

```ts
issueTimeLabel?: string | null;
```

Render it in the badge row with tags, PRs, and relationships. Include `issueTimeLabel` in the row visibility condition.

**Step 2: Thread list props**

Add through `IssueListView.tsx` and `IssueListSection.tsx`:

```ts
getIssueTimeLabel?: (issueId: string) => string | null;
```

In `IssueListRow.tsx`, add prop:

```ts
issueTimeLabel?: string | null;
```

Render before assignees/age:

```tsx
{issueTimeLabel && <IssueTimeBadge label={issueTimeLabel} />}
```

**Step 3: Compute labels in `KanbanContainer`**

In `KanbanContainer.tsx`, import `getIssueTotalMs` and `formatIssueActiveTime`.

Read `issueTimeTotalsByIssueId` from `useProjectContext()` and add:

```ts
const getIssueTimeLabel = useCallback(
  (issueId: string) =>
    formatIssueActiveTime(
      getIssueTotalMs(issueTimeTotalsByIssueId.get(issueId))
    ),
  [issueTimeTotalsByIssueId]
);
```

Pass `issueTimeLabel={getIssueTimeLabel(issue.id)}` to cards and `getIssueTimeLabel={getIssueTimeLabel}` to list view.

**Step 4: Add detail-panel props**

In `KanbanIssuePanel.tsx`, import `IssueTimeBadge` and add props:

```ts
issueTimeLabel?: string | null;
issueTimeExplanation?: string;
```

Render near issue metadata in edit mode:

```tsx
{!isCreateMode && issueTimeLabel && (
  <div className="px-base py-base border-b">
    <div className="flex items-center justify-between gap-base">
      <span className="text-sm text-low">OpenCode active time</span>
      <IssueTimeBadge label={issueTimeLabel} />
    </div>
    <p className="mt-half text-xs text-low">
      {issueTimeExplanation ??
        'Time when OpenCode was actively working while this session was bound to the ticket. Idle and approval-wait time are excluded.'}
    </p>
  </div>
)}
```

**Step 5: Pass detail-panel labels from container**

In `KanbanIssuePanelContainer.tsx`, compute the selected issue's label from `issueTimeTotalsByIssueId` and pass it to `KanbanIssuePanel`.

**Step 6: Type-check UI packages**

Run:

```bash
pnpm --filter @vibe/ui run check
pnpm --filter @vibe/web-core run check
```

Expected: PASS.

---

### Task 11: Add settings UI for plugin tokens

**Files:**
- Create: `packages/web-core/src/shared/dialogs/settings/settings/OpenCodeTimeTrackingSettingsSection.tsx`
- Modify: `packages/web-core/src/shared/dialogs/settings/settings/settingsRegistry.tsx`
- Modify: `packages/web-core/src/shared/lib/api.ts`
- Modify: settings locale files under `packages/web-core/src/i18n/locales/*/settings.json` or use fallback strings consistently

**Step 1: Add API helpers**

In `packages/web-core/src/shared/lib/api.ts`, add methods for:

```text
GET    /api/time-tracking/opencode/tokens
POST   /api/time-tracking/opencode/tokens
DELETE /api/time-tracking/opencode/tokens/{token_id}
```

Use generated local types from `shared/types.ts`.

**Step 2: Add the settings section**

Create `OpenCodeTimeTrackingSettingsSection.tsx` that:

- lists existing token metadata
- creates a new token with an optional label
- shows the raw token only once after creation
- renders an OpenCode config snippet:

```json
{
  "plugin": [
    ["@vibe/opencode-time-tracker", {
      "servers": {
        "http://127.0.0.1:9000": {
          "token": "vktt_..."
        }
      }
    }]
  ]
}
```

- revokes existing tokens

**Step 3: Register the section**

In `settingsRegistry.tsx`:

1. Add an icon import, for example `ClockIcon`.
2. Add `'opencode-time-tracking'` to `SettingsSectionType`.
3. Add it to `SettingsSectionInitialState`.
4. Add a host-specific section definition.
5. Render `OpenCodeTimeTrackingSettingsSection` in `renderSettingsSection`.

**Step 4: Add strings**

Add strings for:

- navigation label
- create token button
- revoke token button
- token shown once warning
- install instructions
- config restart reminder: OpenCode must be restarted after plugin/config changes

If updating every locale is too large for the first slice, use existing fallback-string conventions and create a follow-up localisation issue.

**Step 5: Type-check**

Run:

```bash
pnpm --filter @vibe/web-core run check
```

Expected: PASS.

---

### Task 12: Add optional manual adjustment backend and UI

Manual adjustments are useful but not required for the first tracked-time slice. If included, keep them entry-based.

**Files:**
- Modify: `crates/api-types/src/time_tracking.rs`
- Modify: `crates/remote/src/db/issue_time_tracking.rs`
- Modify: `crates/remote/src/routes/time_tracking.rs`
- Modify: `crates/services/src/services/remote_client.rs`
- Modify: `crates/server/src/routes/time_tracking.rs`
- Modify: `packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx`
- Modify: `packages/ui/src/components/KanbanIssuePanel.tsx`

**Step 1: Add route contracts**

Use the design doc's `CreateIssueTimeAdjustmentRequest` and add response type:

```rust
pub struct CreateIssueTimeAdjustmentResponse {
    pub txid: i64,
    pub entry_id: Uuid,
    pub total: IssueTimeTotal,
}
```

**Step 2: Implement remote route**

Add:

```text
POST /v1/time-tracking/issues/:issue_id/adjustments
```

Validation:

- note is required and trimmed non-empty
- `duration_ms != 0`
- issue exists and user has access
- adjustment creates an `issue_time_entries` row with `source = manual` and `kind = manual_adjustment`
- aggregate updates through the same transaction path

**Step 3: Add local proxy route and frontend dialog**

Only add UI after the backend route exists. The detail panel can show `+15m`, `-5m`, and custom minutes with a required note.

**Step 4: Verify**

Run:

```bash
cargo test --manifest-path crates/remote/Cargo.toml issue_time_tracking
cargo test -p server time_tracking
pnpm --filter @vibe/web-core run check
```

Expected: PASS.

---

### Task 13: End-to-end smoke test

**Files:**
- No new files required unless adding a documented manual QA checklist.

**Step 1: Run backend and frontend checks**

Run:

```bash
pnpm run remote:generate-types:check
pnpm run generate-types:check
pnpm run remote:prepare-db
pnpm run prepare-db
pnpm run backend:check
pnpm run check
pnpm run lint
```

Expected: all checks pass.

**Step 2: Manual local smoke test**

Run vibe-kanban locally:

```bash
pnpm run dev
```

Then:

1. Open settings and create an OpenCode time-tracking token.
2. Configure OpenCode with the plugin and token.
3. Start a new standalone OpenCode session with a vibe-kanban issue URL.
4. Let OpenCode perform a short action and become idle.
5. Confirm the plugin posts an entry.
6. Confirm the issue card/list/detail show a compact active-time badge after Electric sync.
7. Restart OpenCode and resume the same session.
8. Confirm the session binding survives restart.
9. Start a different OpenCode session with no URL.
10. Confirm no time is tracked.
11. Send a new valid issue URL in the original session.
12. Confirm future time goes to the new issue only.

**Step 3: Security smoke test**

Try posting an entry with:

- no token
- malformed token
- revoked token
- mismatched `project_id`/`issue_id`
- duplicate `entry_id` with changed payload

Expected: requests fail with `401`, `400`, or `409` as appropriate and totals do not change.

---

## Suggested implementation slices

1. Remote contracts/schema/repository/routes/shape.
2. Local token storage, token management, and ingestion proxy.
3. OpenCode plugin package with unit tests.
4. Frontend totals shape, formatting helper, and read-only badges.
5. Settings UI for token setup.
6. Manual adjustments as a follow-up if not included in the first release.

Keep every slice shippable and avoid mixing plugin timing heuristics with remote storage changes in the same review.
