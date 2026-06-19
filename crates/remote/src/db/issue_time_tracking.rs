use std::collections::{HashMap, HashSet, hash_map::Entry};

use api_types::{
    CreateOpenCodeTimeEntriesRequest, CreateOpenCodeTimeEntriesResponse, IssueTimeTotal,
    OpenCodeTimeEntryInput, OpenCodeTimeEntryResult, TimeEntryStatus,
};
use chrono::{DateTime, SecondsFormat, Utc};
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

const OPENCODE_SCHEMA_VERSION: i32 = 1;
const MAX_BATCH_SIZE: usize = 100;
const DURATION_TOLERANCE_MS: i64 = 1_000;
const MAX_OPENCODE_INTERVAL_MS: i64 = 6 * 60 * 60 * 1_000;
const MAX_METADATA_BYTES: usize = 8 * 1_024;

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

impl IssueTimeTrackingRepository {
    pub async fn create_opencode_entries(
        pool: &sqlx::PgPool,
        user_id: Uuid,
        request: CreateOpenCodeTimeEntriesRequest,
    ) -> Result<CreateOpenCodeTimeEntriesResponse, IssueTimeTrackingError> {
        if request.schema_version != OPENCODE_SCHEMA_VERSION {
            return Err(IssueTimeTrackingError::InvalidRequest(
                "unsupported schema_version",
            ));
        }

        if request.entries.is_empty() {
            return Err(IssueTimeTrackingError::InvalidRequest(
                "entries must not be empty",
            ));
        }

        if request.entries.len() > MAX_BATCH_SIZE {
            return Err(IssueTimeTrackingError::InvalidRequest(
                "entries batch too large",
            ));
        }

        for entry in &request.entries {
            validate_opencode_entry(entry)?;
        }

        let mut tx = super::begin_tx(pool).await?;

        verify_issue_project_pairs(&mut tx, &request.entries).await?;
        let existing_entries = fetch_existing_entries(&mut tx, &request.entries).await?;

        let mut results = Vec::with_capacity(request.entries.len());
        let mut seen_in_batch = HashMap::<Uuid, String>::new();
        let mut created_entries = Vec::new();

        for entry in &request.entries {
            let payload_hash = canonical_payload_hash(entry);

            if let Some(existing_hash) = existing_entries.get(&entry.entry_id) {
                if existing_hash == &payload_hash {
                    results.push(entry_result(entry, TimeEntryStatus::Duplicate));
                    continue;
                }

                return Err(IssueTimeTrackingError::IdempotencyConflict);
            }

            if let Some(seen_hash) = seen_in_batch.get(&entry.entry_id) {
                if seen_hash == &payload_hash {
                    results.push(entry_result(entry, TimeEntryStatus::Duplicate));
                    continue;
                }

                return Err(IssueTimeTrackingError::IdempotencyConflict);
            }

            if insert_opencode_entry(&mut tx, user_id, entry, &payload_hash).await? {
                seen_in_batch.insert(entry.entry_id, payload_hash);
                created_entries.push(CreatedEntry {
                    project_id: entry.project_id,
                    issue_id: entry.issue_id,
                    duration_ms: entry.duration_ms,
                    ended_at: entry.ended_at,
                });
                results.push(entry_result(entry, TimeEntryStatus::Created));
                continue;
            }

            let existing_hash = fetch_existing_payload_hash(&mut tx, entry.entry_id).await?;
            if existing_hash == payload_hash {
                results.push(entry_result(entry, TimeEntryStatus::Duplicate));
                continue;
            }

            return Err(IssueTimeTrackingError::IdempotencyConflict);
        }

        let updated_totals = upsert_totals_for_created_entries(&mut tx, &created_entries).await?;
        let txid = super::get_txid(&mut *tx).await?;
        tx.commit().await?;

        Ok(CreateOpenCodeTimeEntriesResponse {
            txid,
            results,
            updated_totals,
        })
    }

    pub async fn list_totals_by_project(
        pool: &sqlx::PgPool,
        project_id: Uuid,
    ) -> Result<Vec<IssueTimeTotal>, sqlx::Error> {
        let rows = sqlx::query(
            r#"
            SELECT
                project_id,
                issue_id,
                opencode_active_ms,
                manual_adjustment_ms,
                total_ms,
                entry_count,
                last_entry_at,
                updated_at
            FROM issue_time_totals
            WHERE project_id = $1
            ORDER BY issue_id
            "#,
        )
        .bind(project_id)
        .fetch_all(pool)
        .await?;

        rows.iter().map(issue_time_total_from_row).collect()
    }
}

fn canonical_payload_hash(input: &OpenCodeTimeEntryInput) -> String {
    let mut hasher = Sha256::new();

    // Hash only immutable idempotency fields. `source` and `kind` are implied by
    // this OpenCode endpoint, and metadata is intentionally excluded because it
    // is mutable/non-semantic retry context rather than part of entry identity.
    hash_field(&mut hasher, "schema_version", "1");
    hash_field(&mut hasher, "source", "opencode");
    hash_field(&mut hasher, "kind", "active_interval");
    hash_field(&mut hasher, "project_id", &input.project_id.to_string());
    hash_field(&mut hasher, "issue_id", &input.issue_id.to_string());
    hash_field(
        &mut hasher,
        "started_at",
        &input.started_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
    );
    hash_field(
        &mut hasher,
        "ended_at",
        &input.ended_at.to_rfc3339_opts(SecondsFormat::Nanos, true),
    );
    hash_field(&mut hasher, "duration_ms", &input.duration_ms.to_string());
    match &input.source_session_id {
        Some(source_session_id) => hash_field(&mut hasher, "source_session_id", source_session_id),
        None => hash_field(&mut hasher, "source_session_id", "<null>"),
    }

    hex::encode(hasher.finalize())
}

fn validate_opencode_entry(input: &OpenCodeTimeEntryInput) -> Result<(), IssueTimeTrackingError> {
    if input.started_at >= input.ended_at {
        return Err(IssueTimeTrackingError::InvalidRequest(
            "started_at must be before ended_at",
        ));
    }

    if input.duration_ms <= 0 {
        return Err(IssueTimeTrackingError::InvalidRequest(
            "duration_ms must be positive",
        ));
    }

    let wall_clock_ms = (input.ended_at - input.started_at).num_milliseconds();
    if input.duration_ms > wall_clock_ms + DURATION_TOLERANCE_MS {
        return Err(IssueTimeTrackingError::InvalidRequest(
            "duration_ms exceeds interval",
        ));
    }

    if input.duration_ms > MAX_OPENCODE_INTERVAL_MS {
        return Err(IssueTimeTrackingError::InvalidRequest(
            "duration_ms exceeds maximum interval",
        ));
    }

    let metadata_size = serde_json::to_vec(&input.metadata)
        .map_err(|_| IssueTimeTrackingError::InvalidRequest("metadata is invalid"))?
        .len();
    if metadata_size > MAX_METADATA_BYTES {
        return Err(IssueTimeTrackingError::InvalidRequest(
            "metadata is too large",
        ));
    }

    Ok(())
}

fn hash_field(hasher: &mut Sha256, name: &str, value: &str) {
    hasher.update(name.as_bytes());
    hasher.update(b"=");
    hasher.update(value.len().to_string().as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    hasher.update(b"\n");
}

async fn verify_issue_project_pairs(
    tx: &mut super::Tx<'_>,
    entries: &[OpenCodeTimeEntryInput],
) -> Result<(), IssueTimeTrackingError> {
    let mut pairs = HashSet::new();
    for entry in entries {
        pairs.insert((entry.project_id, entry.issue_id));
    }

    for (project_id, issue_id) in pairs {
        let row: (bool,) = sqlx::query_as(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM issues
                WHERE id = $1
                  AND project_id = $2
            )
            "#,
        )
        .bind(issue_id)
        .bind(project_id)
        .fetch_one(&mut **tx)
        .await?;

        if !row.0 {
            return Err(IssueTimeTrackingError::InvalidRequest(
                "issue_id must belong to project_id",
            ));
        }
    }

    Ok(())
}

async fn fetch_existing_entries(
    tx: &mut super::Tx<'_>,
    entries: &[OpenCodeTimeEntryInput],
) -> Result<HashMap<Uuid, String>, IssueTimeTrackingError> {
    let entry_ids = entries
        .iter()
        .map(|entry| entry.entry_id)
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        r#"
        SELECT entry_id, payload_hash
        FROM issue_time_entries
        WHERE entry_id = ANY($1)
        "#,
    )
    .bind(&entry_ids)
    .fetch_all(&mut **tx)
    .await?;

    let mut existing_entries = HashMap::with_capacity(rows.len());
    for row in rows {
        existing_entries.insert(row.try_get("entry_id")?, row.try_get("payload_hash")?);
    }

    Ok(existing_entries)
}

async fn insert_opencode_entry(
    tx: &mut super::Tx<'_>,
    user_id: Uuid,
    entry: &OpenCodeTimeEntryInput,
    payload_hash: &str,
) -> Result<bool, IssueTimeTrackingError> {
    let inserted = sqlx::query(
        r#"
        INSERT INTO issue_time_entries (
            entry_id,
            project_id,
            issue_id,
            user_id,
            source,
            kind,
            started_at,
            ended_at,
            duration_ms,
            source_session_id,
            metadata,
            payload_hash,
            created_at
        )
        VALUES ($1, $2, $3, $4, 'opencode', 'active_interval', $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (entry_id) DO NOTHING
        RETURNING entry_id
        "#,
    )
    .bind(entry.entry_id)
    .bind(entry.project_id)
    .bind(entry.issue_id)
    .bind(user_id)
    .bind(entry.started_at)
    .bind(entry.ended_at)
    .bind(entry.duration_ms)
    .bind(&entry.source_session_id)
    .bind(&entry.metadata)
    .bind(payload_hash)
    .bind(entry.started_at)
    .fetch_optional(&mut **tx)
    .await?;

    Ok(inserted.is_some())
}

async fn fetch_existing_payload_hash(
    tx: &mut super::Tx<'_>,
    entry_id: Uuid,
) -> Result<String, IssueTimeTrackingError> {
    let row = sqlx::query(
        r#"
        SELECT payload_hash
        FROM issue_time_entries
        WHERE entry_id = $1
        "#,
    )
    .bind(entry_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(sqlx::Error::RowNotFound)?;

    Ok(row.try_get("payload_hash")?)
}

async fn upsert_totals_for_created_entries(
    tx: &mut super::Tx<'_>,
    created_entries: &[CreatedEntry],
) -> Result<Vec<IssueTimeTotal>, IssueTimeTrackingError> {
    let mut totals = HashMap::<(Uuid, Uuid), TotalAccumulator>::new();
    let mut total_order = Vec::new();
    for entry in created_entries {
        let key = (entry.project_id, entry.issue_id);
        let total = match totals.entry(key) {
            Entry::Vacant(vacant) => {
                total_order.push(key);
                vacant.insert(TotalAccumulator {
                    project_id: entry.project_id,
                    issue_id: entry.issue_id,
                    opencode_active_ms: 0,
                    entry_count: 0,
                    last_entry_at: entry.ended_at,
                })
            }
            Entry::Occupied(occupied) => occupied.into_mut(),
        };

        total.opencode_active_ms += entry.duration_ms;
        total.entry_count += 1;
        if entry.ended_at > total.last_entry_at {
            total.last_entry_at = entry.ended_at;
        }
    }

    let mut updated_totals = Vec::with_capacity(totals.len());
    for key in total_order {
        let total = totals.remove(&key).unwrap_or_else(|| {
            unreachable!(
                "time tracking total accumulator must exist for ordered key {:?}",
                key
            )
        });
        let row = sqlx::query(
            r#"
            INSERT INTO issue_time_totals (
                project_id,
                issue_id,
                opencode_active_ms,
                manual_adjustment_ms,
                total_ms,
                entry_count,
                last_entry_at
            )
            VALUES ($1, $2, $3, 0, $3, $4, $5)
            ON CONFLICT (project_id, issue_id) DO UPDATE SET
                opencode_active_ms = issue_time_totals.opencode_active_ms + EXCLUDED.opencode_active_ms,
                total_ms = issue_time_totals.total_ms + EXCLUDED.opencode_active_ms,
                entry_count = issue_time_totals.entry_count + EXCLUDED.entry_count,
                last_entry_at = CASE
                    WHEN issue_time_totals.last_entry_at IS NULL
                      OR EXCLUDED.last_entry_at > issue_time_totals.last_entry_at
                    THEN EXCLUDED.last_entry_at
                    ELSE issue_time_totals.last_entry_at
                END,
                updated_at = now()
            RETURNING
                project_id,
                issue_id,
                opencode_active_ms,
                manual_adjustment_ms,
                total_ms,
                entry_count,
                last_entry_at,
                updated_at
            "#,
        )
        .bind(total.project_id)
        .bind(total.issue_id)
        .bind(total.opencode_active_ms)
        .bind(total.entry_count)
        .bind(total.last_entry_at)
        .fetch_one(&mut **tx)
        .await?;

        updated_totals.push(issue_time_total_from_row(&row)?);
    }

    Ok(updated_totals)
}

fn issue_time_total_from_row(row: &PgRow) -> Result<IssueTimeTotal, sqlx::Error> {
    Ok(IssueTimeTotal {
        project_id: row.try_get("project_id")?,
        issue_id: row.try_get("issue_id")?,
        opencode_active_ms: row.try_get("opencode_active_ms")?,
        manual_adjustment_ms: row.try_get("manual_adjustment_ms")?,
        total_ms: row.try_get("total_ms")?,
        entry_count: row.try_get("entry_count")?,
        last_entry_at: row.try_get("last_entry_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn entry_result(
    entry: &OpenCodeTimeEntryInput,
    status: TimeEntryStatus,
) -> OpenCodeTimeEntryResult {
    OpenCodeTimeEntryResult {
        entry_id: entry.entry_id,
        status,
        project_id: entry.project_id,
        issue_id: entry.issue_id,
        duration_ms: entry.duration_ms,
    }
}

#[derive(Debug)]
struct CreatedEntry {
    project_id: Uuid,
    issue_id: Uuid,
    duration_ms: i64,
    ended_at: DateTime<Utc>,
}

#[derive(Debug)]
struct TotalAccumulator {
    project_id: Uuid,
    issue_id: Uuid,
    opencode_active_ms: i64,
    entry_count: i64,
    last_entry_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    use super::*;

    fn sample_entry() -> OpenCodeTimeEntryInput {
        let started_at = Utc.with_ymd_and_hms(2026, 6, 18, 10, 0, 0).unwrap();
        OpenCodeTimeEntryInput {
            entry_id: Uuid::from_u128(1),
            project_id: Uuid::from_u128(2),
            issue_id: Uuid::from_u128(3),
            source_session_id: Some("opencode-session".to_string()),
            started_at,
            ended_at: started_at + chrono::Duration::minutes(3),
            duration_ms: 180_000,
            metadata: serde_json::json!({"plugin_version": "test"}),
        }
    }

    #[test]
    fn canonical_payload_hash_is_stable_for_same_entry() {
        let input = sample_entry();

        let first = canonical_payload_hash(&input);
        let second = canonical_payload_hash(&input);

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    #[test]
    fn insert_sql_is_conflict_aware_for_concurrent_idempotency() {
        let source = include_str!("issue_time_tracking.rs");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(production_source.contains("ON CONFLICT (entry_id) DO NOTHING"));
    }

    #[test]
    fn list_totals_by_project_reads_only_project_totals() {
        let source = include_str!("issue_time_tracking.rs");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(production_source.contains("pub async fn list_totals_by_project"));
        assert!(production_source.contains("FROM issue_time_totals"));
        assert!(production_source.contains("WHERE project_id = $1"));
        assert!(production_source.contains("ORDER BY issue_id"));
    }

    #[test]
    fn canonical_payload_hash_changes_when_immutable_fields_change() {
        let input = sample_entry();
        let mut changed = input.clone();
        changed.duration_ms += 1;

        assert_ne!(
            canonical_payload_hash(&input),
            canonical_payload_hash(&changed)
        );
    }

    #[test]
    fn canonical_payload_hash_ignores_metadata_by_design() {
        let input = sample_entry();
        let mut changed = input.clone();
        changed.metadata = serde_json::json!({
            "plugin_version": "changed",
            "non_semantic_retry_note": "metadata is mutable"
        });

        assert_eq!(
            canonical_payload_hash(&input),
            canonical_payload_hash(&changed)
        );
    }

    #[test]
    fn opencode_duration_must_be_positive() {
        let mut input = sample_entry();
        input.duration_ms = 0;

        assert!(matches!(
            validate_opencode_entry(&input),
            Err(IssueTimeTrackingError::InvalidRequest(_))
        ));
    }

    #[test]
    fn opencode_started_at_must_be_before_ended_at() {
        let mut input = sample_entry();
        input.ended_at = input.started_at;

        assert!(matches!(
            validate_opencode_entry(&input),
            Err(IssueTimeTrackingError::InvalidRequest(_))
        ));
    }

    #[test]
    fn opencode_duration_cannot_exceed_interval_plus_tolerance() {
        let mut input = sample_entry();
        input.ended_at = input.started_at + chrono::Duration::seconds(10);
        input.duration_ms = 20_000;

        assert!(matches!(
            validate_opencode_entry(&input),
            Err(IssueTimeTrackingError::InvalidRequest(_))
        ));
    }

    #[test]
    fn opencode_duration_cannot_exceed_interval_cap() {
        let mut input = sample_entry();
        input.ended_at = input.started_at + chrono::Duration::hours(7);
        input.duration_ms = MAX_OPENCODE_INTERVAL_MS + 1;

        assert!(matches!(
            validate_opencode_entry(&input),
            Err(IssueTimeTrackingError::InvalidRequest(_))
        ));
    }

    #[test]
    fn opencode_metadata_size_is_bounded() {
        let mut input = sample_entry();
        input.metadata = serde_json::json!({"oversized": "x".repeat(MAX_METADATA_BYTES)});

        assert!(matches!(
            validate_opencode_entry(&input),
            Err(IssueTimeTrackingError::InvalidRequest(_))
        ));
    }
}
