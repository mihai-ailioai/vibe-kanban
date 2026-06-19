use api_types::{
    CreateOpenCodeTimeEntriesRequest, CreateOpenCodeTimeEntriesResponse,
    CreateOpenCodeTimeTrackingTokenRequest, CreateOpenCodeTimeTrackingTokenResponse,
    OpenCodeTimeTrackingTokenSummary,
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    response::Json as ResponseJson,
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use db::models::time_tracking_token::{
    OPENCODE_TIME_TRACKING_SCOPE, OPENCODE_TIME_TRACKING_TOKEN_PREFIX, OpencodeTimeTrackingToken,
};
use deployment::Deployment;
use rand::{RngCore, rngs::OsRng};
use sha2::{Digest, Sha256};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub fn router(_deployment: &DeploymentImpl) -> Router<DeploymentImpl> {
    Router::new()
        .route(
            "/time-tracking/opencode/entries",
            axum::routing::post(create_entries),
        )
        .route(
            "/time-tracking/opencode/tokens",
            get(list_tokens).post(create_token),
        )
        .route(
            "/time-tracking/opencode/tokens/{token_id}",
            axum::routing::delete(revoke_token),
        )
}

async fn create_entries(
    State(deployment): State<DeploymentImpl>,
    headers: HeaderMap,
    Json(request): Json<CreateOpenCodeTimeEntriesRequest>,
) -> Result<ResponseJson<ApiResponse<CreateOpenCodeTimeEntriesResponse>>, ApiError> {
    let raw_token = extract_bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    let token_hash = hash_token(raw_token);
    let token = OpencodeTimeTrackingToken::find_active_by_hash(&deployment.db().pool, &token_hash)
        .await?
        .filter(|token| token.scope == OPENCODE_TIME_TRACKING_SCOPE)
        .ok_or(ApiError::Unauthorized)?;

    let client = deployment.remote_client()?;
    let response = client.create_opencode_time_entries(&request).await?;

    OpencodeTimeTrackingToken::mark_used(&deployment.db().pool, token.id).await?;

    Ok(ResponseJson(ApiResponse::success(response)))
}

async fn create_token(
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<CreateOpenCodeTimeTrackingTokenRequest>,
) -> Result<ResponseJson<ApiResponse<CreateOpenCodeTimeTrackingTokenResponse>>, ApiError> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let created = OpencodeTimeTrackingToken::create(
        &deployment.db().pool,
        &token_hash,
        request.label.as_deref(),
    )
    .await?;

    Ok(ResponseJson(ApiResponse::success(
        CreateOpenCodeTimeTrackingTokenResponse {
            id: created.id,
            token,
            created_at: created.created_at,
        },
    )))
}

async fn list_tokens(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<OpenCodeTimeTrackingTokenSummary>>>, ApiError> {
    let tokens = OpencodeTimeTrackingToken::list(&deployment.db().pool).await?;
    let summaries = tokens
        .into_iter()
        .map(|token| OpenCodeTimeTrackingTokenSummary {
            id: token.id,
            label: token.label,
            created_at: token.created_at,
            last_used_at: token.last_used_at,
            revoked_at: token.revoked_at,
        })
        .collect();

    Ok(ResponseJson(ApiResponse::success(summaries)))
}

async fn revoke_token(
    State(deployment): State<DeploymentImpl>,
    Path(token_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    OpencodeTimeTrackingToken::revoke(&deployment.db().pool, token_id).await?;
    Ok(ResponseJson(ApiResponse::success(())))
}

fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?;
    let token = header.strip_prefix("Bearer ")?;

    if token.starts_with(OPENCODE_TIME_TRACKING_TOKEN_PREFIX) {
        Some(token)
    } else {
        None
    }
}

fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!(
        "{}{}",
        OPENCODE_TIME_TRACKING_TOKEN_PREFIX,
        URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn hash_token(raw_token: &str) -> String {
    format!("{:x}", Sha256::digest(raw_token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use std::{sync::LazyLock, time::Duration as StdDuration};

    use api_types::{
        CreateOpenCodeTimeEntriesRequest, CreateOpenCodeTimeEntriesResponse,
        OpenCodeTimeEntryInput, OpenCodeTimeEntryResult, TimeEntryStatus,
    };
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use chrono::{Duration, Utc};
    use db::models::time_tracking_token::OpencodeTimeTrackingToken;
    use deployment::Deployment;
    use reqwest::header::AUTHORIZATION;
    use services::services::oauth_credentials::Credentials;
    use sha2::{Digest, Sha256};
    use tokio::{net::TcpListener, sync::Mutex};
    use utils::response::ApiResponse;
    use uuid::Uuid;

    use crate::{DeploymentImpl, test_support::TestAssetDirGuard};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct TestApp {
        _env_guard: tokio::sync::MutexGuard<'static, ()>,
        _asset_guard: TestAssetDirGuard,
        deployment: DeploymentImpl,
        base_url: String,
        server: tokio::task::JoinHandle<()>,
        remote_server: tokio::task::JoinHandle<()>,
        remote_requests: std::sync::Arc<Mutex<Vec<RemoteRequest>>>,
    }

    struct RemoteState {
        requests: std::sync::Arc<Mutex<Vec<RemoteRequest>>>,
        fail_entries: bool,
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("VK_SHARED_API_BASE");
            }
            self.server.abort();
            self.remote_server.abort();
        }
    }

    #[derive(Debug, Clone)]
    struct RemoteRequest {
        authorization: Option<String>,
        body: CreateOpenCodeTimeEntriesRequest,
    }

    async fn remote_create_entries(
        State(state): State<std::sync::Arc<RemoteState>>,
        headers: HeaderMap,
        Json(body): Json<CreateOpenCodeTimeEntriesRequest>,
    ) -> axum::response::Response {
        state.requests.lock().await.push(RemoteRequest {
            authorization: headers
                .get(AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned),
            body: body.clone(),
        });

        if state.fail_entries {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "remote_failed" })),
            )
                .into_response();
        }

        let results = body
            .entries
            .iter()
            .map(|entry| OpenCodeTimeEntryResult {
                entry_id: entry.entry_id,
                status: TimeEntryStatus::Created,
                project_id: entry.project_id,
                issue_id: entry.issue_id,
                duration_ms: entry.duration_ms,
            })
            .collect();

        Json(CreateOpenCodeTimeEntriesResponse {
            txid: 42,
            results,
            updated_totals: vec![],
        })
        .into_response()
    }

    async fn start_test_app() -> TestApp {
        start_test_app_with_remote_failure(false).await
    }

    async fn start_test_app_with_remote_failure(fail_entries: bool) -> TestApp {
        let env_guard = ENV_LOCK.lock().await;
        let remote_requests = std::sync::Arc::new(Mutex::new(Vec::new()));
        let remote_state = std::sync::Arc::new(RemoteState {
            requests: remote_requests.clone(),
            fail_entries,
        });

        let remote_app = Router::new()
            .route(
                "/v1/time-tracking/opencode/entries",
                post(remote_create_entries),
            )
            .with_state(remote_state);
        let remote_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote_address = remote_listener.local_addr().unwrap();
        let remote_server = tokio::spawn(async move {
            axum::serve(remote_listener, remote_app).await.unwrap();
        });

        unsafe {
            std::env::set_var("VK_SHARED_API_BASE", format!("http://{remote_address}"));
        }
        let (asset_guard, deployment) = crate::test_support::new_test_deployment().await;
        deployment
            .auth_context()
            .save_credentials(&Credentials {
                access_token: Some("remote-access-token".to_string()),
                refresh_token: "remote-refresh-token".to_string(),
                expires_at: Some(Utc::now() + Duration::hours(1)),
            })
            .await
            .unwrap();

        let app = Router::new()
            .nest("/api", super::router(&deployment))
            .with_state(deployment.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        TestApp {
            _env_guard: env_guard,
            _asset_guard: asset_guard,
            deployment,
            base_url: format!("http://{address}"),
            server,
            remote_server,
            remote_requests,
        }
    }

    fn sample_entries_request() -> CreateOpenCodeTimeEntriesRequest {
        let now = Utc::now();
        CreateOpenCodeTimeEntriesRequest {
            schema_version: 1,
            entries: vec![OpenCodeTimeEntryInput {
                entry_id: Uuid::new_v4(),
                project_id: Uuid::new_v4(),
                issue_id: Uuid::new_v4(),
                source_session_id: Some("session-1".to_string()),
                started_at: now - Duration::minutes(5),
                ended_at: now,
                duration_ms: 300_000,
                metadata: serde_json::json!({"source": "test"}),
            }],
        }
    }

    fn token_hash(raw_token: &str) -> String {
        format!("{:x}", Sha256::digest(raw_token.as_bytes()))
    }

    async fn create_local_token(deployment: &DeploymentImpl, raw_token: &str) -> Uuid {
        let hash = token_hash(raw_token);
        OpencodeTimeTrackingToken::create(&deployment.db().pool, &hash, Some("test token"))
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn create_entries_rejects_missing_bearer_token() {
        let app = start_test_app().await;

        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/entries",
                app.base_url
            ))
            .json(&sample_entries_request())
            .send()
            .await
            .unwrap();

        let status = response.status();
        let payload: ApiResponse<serde_json::Value> = response.json().await.unwrap();

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!payload.is_success());
        assert!(app.remote_requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn create_entries_rejects_wrong_token_prefix() {
        let app = start_test_app().await;

        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/entries",
                app.base_url
            ))
            .bearer_auth("wrong_prefix_token")
            .json(&sample_entries_request())
            .send()
            .await
            .unwrap();

        let status = response.status();
        let payload: ApiResponse<serde_json::Value> = response.json().await.unwrap();

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!payload.is_success());
        assert!(app.remote_requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn create_entries_rejects_revoked_token() {
        let app = start_test_app().await;
        let raw_token = "vktt_revoked";
        let token_id = create_local_token(&app.deployment, raw_token).await;
        OpencodeTimeTrackingToken::revoke(&app.deployment.db().pool, token_id)
            .await
            .unwrap();

        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/entries",
                app.base_url
            ))
            .bearer_auth(raw_token)
            .json(&sample_entries_request())
            .send()
            .await
            .unwrap();

        let status = response.status();
        let payload: ApiResponse<serde_json::Value> = response.json().await.unwrap();

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(!payload.is_success());
        assert!(app.remote_requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn create_entries_with_valid_token_forwards_to_remote_and_marks_used() {
        let app = start_test_app().await;
        let raw_token = "vktt_valid_token";
        let token_id = create_local_token(&app.deployment, raw_token).await;
        let request = sample_entries_request();

        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/entries",
                app.base_url
            ))
            .bearer_auth(raw_token)
            .json(&request)
            .timeout(StdDuration::from_secs(5))
            .send()
            .await
            .unwrap();

        let status = response.status();
        let payload: ApiResponse<CreateOpenCodeTimeEntriesResponse> =
            response.json().await.unwrap();

        assert_eq!(status, StatusCode::OK);
        let response_data = payload.into_data().unwrap();
        assert_eq!(response_data.txid, 42);

        let remote_requests = app.remote_requests.lock().await;
        assert_eq!(remote_requests.len(), 1);
        assert_eq!(
            remote_requests[0].authorization.as_deref(),
            Some("Bearer remote-access-token")
        );
        assert_eq!(
            remote_requests[0].body.entries[0].entry_id,
            request.entries[0].entry_id
        );
        drop(remote_requests);

        let token = OpencodeTimeTrackingToken::list(&app.deployment.db().pool)
            .await
            .unwrap()
            .into_iter()
            .find(|token| token.id == token_id)
            .expect("token should still exist");
        assert!(token.last_used_at.is_some());
    }

    #[tokio::test]
    async fn remote_failure_does_not_mark_token_used() {
        let app = start_test_app_with_remote_failure(true).await;
        let raw_token = "vktt_remote_failure";
        let token_id = create_local_token(&app.deployment, raw_token).await;

        let response = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/entries",
                app.base_url
            ))
            .bearer_auth(raw_token)
            .json(&sample_entries_request())
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!app.remote_requests.lock().await.is_empty());

        let token = OpencodeTimeTrackingToken::list(&app.deployment.db().pool)
            .await
            .unwrap()
            .into_iter()
            .find(|token| token.id == token_id)
            .expect("token should still exist");
        assert!(token.last_used_at.is_none());
    }

    #[tokio::test]
    async fn token_create_and_list_responses_do_not_expose_hash_or_raw_token_in_list() {
        let app = start_test_app().await;

        let create_body = reqwest::Client::new()
            .post(format!(
                "{}/api/time-tracking/opencode/tokens",
                app.base_url
            ))
            .json(&serde_json::json!({ "label": "local plugin" }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let create_json: serde_json::Value = serde_json::from_str(&create_body).unwrap();
        let raw_token = create_json
            .pointer("/data/token")
            .and_then(|value| value.as_str())
            .expect("creation response should return plaintext token once");

        assert!(raw_token.starts_with("vktt_"));
        assert!(!create_body.contains("token_hash"));

        let list_body = reqwest::Client::new()
            .get(format!(
                "{}/api/time-tracking/opencode/tokens",
                app.base_url
            ))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

        assert!(!list_body.contains("token_hash"));
        assert!(!list_body.contains(raw_token));
        assert!(!list_body.contains("\"token\""));
    }

    #[tokio::test]
    async fn token_auth_failures_return_indistinguishable_generic_401_payloads() {
        let app = start_test_app().await;
        let revoked_token = "vktt_revoked_generic";
        let token_id = create_local_token(&app.deployment, revoked_token).await;
        OpencodeTimeTrackingToken::revoke(&app.deployment.db().pool, token_id)
            .await
            .unwrap();

        let client = reqwest::Client::new();
        let request = sample_entries_request();
        let url = format!("{}/api/time-tracking/opencode/entries", app.base_url);

        let malformed = client
            .post(&url)
            .header(AUTHORIZATION, "Token not-a-bearer-token")
            .json(&request)
            .send()
            .await
            .unwrap();
        let malformed_status = malformed.status();
        let malformed_body: serde_json::Value = malformed.json().await.unwrap();

        let unknown = client
            .post(&url)
            .bearer_auth("vktt_unknown_token")
            .json(&request)
            .send()
            .await
            .unwrap();
        let unknown_status = unknown.status();
        let unknown_body: serde_json::Value = unknown.json().await.unwrap();

        let revoked = client
            .post(&url)
            .bearer_auth(revoked_token)
            .json(&request)
            .send()
            .await
            .unwrap();
        let revoked_status = revoked.status();
        let revoked_body: serde_json::Value = revoked.json().await.unwrap();

        assert_eq!(malformed_status, StatusCode::UNAUTHORIZED);
        assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
        assert_eq!(revoked_status, StatusCode::UNAUTHORIZED);
        assert_eq!(unknown_body, malformed_body);
        assert_eq!(revoked_body, malformed_body);
    }
}
