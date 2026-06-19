use std::collections::HashSet;

use api_types::{CreateOpenCodeTimeEntriesRequest, CreateOpenCodeTimeEntriesResponse};
use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    routing::post,
};
use uuid::Uuid;

use crate::{
    AppState,
    auth::RequestContext,
    db::{
        identity_errors::IdentityError,
        issue_time_tracking::{IssueTimeTrackingError, IssueTimeTrackingRepository},
        organization_members,
    },
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
) -> Result<Json<CreateOpenCodeTimeEntriesResponse>, ErrorResponse> {
    authorize_time_entry_projects(state.pool(), ctx.user.id, &payload).await?;

    let response =
        IssueTimeTrackingRepository::create_opencode_entries(state.pool(), ctx.user.id, payload)
            .await
            .map_err(map_time_tracking_error)?;

    Ok(Json(response))
}

async fn authorize_time_entry_projects(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    payload: &CreateOpenCodeTimeEntriesRequest,
) -> Result<(), ErrorResponse> {
    let mut project_ids = HashSet::<Uuid>::new();

    for entry in &payload.entries {
        if !project_ids.insert(entry.project_id) {
            continue;
        }

        organization_members::assert_project_access(pool, entry.project_id, user_id)
            .await
            .map_err(|error| map_project_access_error(error, entry.project_id, user_id))?;
    }

    Ok(())
}

fn map_project_access_error(
    error: IdentityError,
    project_id: Uuid,
    user_id: Uuid,
) -> ErrorResponse {
    match error {
        IdentityError::Database(error) => {
            tracing::error!(
                ?error,
                %project_id,
                %user_id,
                "failed to authorize time entry project"
            );
            ErrorResponse::new(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
        }
        IdentityError::NotFound | IdentityError::PermissionDenied => {
            tracing::warn!(
                %project_id,
                %user_id,
                "time entry project access denied"
            );
            ErrorResponse::new(StatusCode::FORBIDDEN, "project not accessible")
        }
        other => {
            tracing::warn!(
                ?other,
                %project_id,
                %user_id,
                "unexpected time entry project authorization error"
            );
            ErrorResponse::new(StatusCode::FORBIDDEN, "project not accessible")
        }
    }
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
            ErrorResponse::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to record time entry",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_opencode_entries_authorizes_projects_before_write() {
        let source = include_str!("time_tracking.rs");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);

        let auth_index = production_source
            .find("authorize_time_entry_projects(state.pool(), ctx.user.id, &payload)")
            .expect("route must authorize submitted projects before writing time entries");
        let write_index = production_source
            .find("IssueTimeTrackingRepository::create_opencode_entries")
            .expect("route must call repository write helper");

        assert!(
            auth_index < write_index,
            "project authorization must happen before repository write"
        );
    }

    #[test]
    fn project_authorization_is_distinct_by_project_id() {
        let source = include_str!("time_tracking.rs");
        let production_source = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(production_source.contains("HashSet::<Uuid>::new()"));
        assert!(production_source.contains("project_ids.insert(entry.project_id)"));
    }
}
