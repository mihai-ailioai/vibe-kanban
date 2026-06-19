use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IssueTimeTotal {
    pub project_id: Uuid,
    pub issue_id: Uuid,
    #[ts(type = "number")]
    pub opencode_active_ms: i64,
    #[ts(type = "number")]
    pub manual_adjustment_ms: i64,
    #[ts(type = "number")]
    pub total_ms: i64,
    #[ts(type = "number")]
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
    #[ts(type = "number")]
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
    #[ts(type = "number")]
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeEntriesResponse {
    #[ts(type = "number")]
    pub txid: i64,
    pub results: Vec<OpenCodeTimeEntryResult>,
    pub updated_totals: Vec<IssueTimeTotal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
/// Reserved for follow-up read endpoints; Task 4 only exposes totals through
/// the project-scoped Electric shape and fallback route.
pub struct GetIssueTimeTrackingResponse {
    pub total: Option<IssueTimeTotal>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
/// Reserved for follow-up manual adjustment endpoints; OpenCode entry creation
/// remains the only write route added in Task 4.
pub struct CreateIssueTimeAdjustmentRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entry_id: Option<Uuid>,
    #[ts(type = "number")]
    pub duration_ms: i64,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CreateOpenCodeTimeTrackingTokenRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
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
