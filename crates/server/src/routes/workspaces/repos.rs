use axum::{Extension, Json, Router, extract::State, response::Json as ResponseJson, routing::get};
use db::models::{
    requests::WorkspaceRepoInput,
    workspace::{Workspace, WorkspaceError},
    workspace_repo::{RepoWithTargetBranch, WorkspaceRepo},
};
use deployment::Deployment;
use serde::{Deserialize, Serialize};
use services::services::container::ContainerService;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use super::capabilities::require_repo_attach;
use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize, Serialize, TS)]
pub struct AddWorkspaceRepoRequest {
    pub repo_id: Uuid,
    pub target_branch: String,
}

#[derive(Debug, Serialize, TS)]
pub struct AddWorkspaceRepoResponse {
    pub workspace: Workspace,
    pub repo: RepoWithTargetBranch,
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().route("/", get(get_workspace_repos).post(add_workspace_repo))
}

pub async fn get_workspace_repos(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<RepoWithTargetBranch>>>, ApiError> {
    let pool = &deployment.db().pool;
    let repos =
        WorkspaceRepo::find_repos_with_target_branch_for_workspace(pool, workspace.id).await?;
    Ok(ResponseJson(ApiResponse::success(repos)))
}

#[axum::debug_handler]
pub async fn add_workspace_repo(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<AddWorkspaceRepoRequest>,
) -> Result<ResponseJson<ApiResponse<AddWorkspaceRepoResponse>>, ApiError> {
    require_repo_attach(&workspace).map_err(ApiError::BadRequest)?;

    let mut managed_workspace = deployment
        .workspace_manager()
        .load_managed_workspace(workspace)
        .await?;

    let repo_input = WorkspaceRepoInput {
        repo_id: payload.repo_id,
        target_branch: payload.target_branch,
    };

    managed_workspace
        .add_repository(&repo_input, deployment.git())
        .await
        .map_err(ApiError::from)?;

    deployment
        .container()
        .ensure_container_exists(&managed_workspace.workspace)
        .await?;

    let workspace = Workspace::find_by_id(&deployment.db().pool, managed_workspace.workspace.id)
        .await?
        .ok_or(WorkspaceError::WorkspaceNotFound)?;
    let repo = managed_workspace
        .repos
        .iter()
        .find(|repo_with_target| repo_with_target.repo.id == repo_input.repo_id)
        .cloned()
        .ok_or_else(|| {
            ApiError::Conflict("Repository already attached to workspace".to_string())
        })?;

    deployment
        .track_if_analytics_allowed(
            "task_attempt_repo_added",
            serde_json::json!({
                "workspace_id": workspace.id.to_string(),
                "repo_id": repo.repo.id.to_string(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(
        AddWorkspaceRepoResponse { workspace, repo },
    )))
}

#[cfg(test)]
mod tests {
    use axum::{Router, http::StatusCode};
    use db::models::workspace::{CreateWorkspace, Workspace, WorkspaceMode};
    use deployment::Deployment;
    use tokio::net::TcpListener;
    use utils::response::ApiResponse;
    use uuid::Uuid;

    use crate::{DeploymentImpl, test_support::TestAssetDirGuard};

    async fn start_app() -> (
        TestAssetDirGuard,
        DeploymentImpl,
        String,
        tokio::task::JoinHandle<()>,
    ) {
        let (asset_guard, deployment) = crate::test_support::new_test_deployment().await;

        let app = Router::new()
            .nest("/api", super::super::router(&deployment))
            .with_state(deployment.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        (asset_guard, deployment, format!("http://{address}"), server)
    }

    async fn create_workspace(
        deployment: &DeploymentImpl,
        workspace_mode: WorkspaceMode,
    ) -> Workspace {
        Workspace::create(
            &deployment.db().pool,
            &CreateWorkspace {
                branch: format!("repos-route-test-{}", Uuid::new_v4()),
                workspace_mode,
                name: Some("Repos route capability test".to_string()),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn add_workspace_repo_rejects_in_place_git_mode() {
        let (_asset_guard, deployment, base_url, server) = start_app().await;
        let workspace = create_workspace(&deployment, WorkspaceMode::InPlaceGit).await;

        let response = reqwest::Client::new()
            .post(format!("{base_url}/api/workspaces/{}/repos", workspace.id))
            .json(&super::AddWorkspaceRepoRequest {
                repo_id: Uuid::new_v4(),
                target_branch: "main".to_string(),
            })
            .send()
            .await
            .unwrap();

        let status = response.status();
        let body = response.text().await.unwrap();

        server.abort();
        let _ = server.await;
        let _ = Workspace::delete(&deployment.db().pool, workspace.id).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
        let payload: ApiResponse<serde_json::Value> = serde_json::from_str(&body).unwrap();
        assert!(!payload.is_success());
        let message = payload.message().unwrap();
        assert!(message.contains("in_place_git"));
        assert!(message.contains("repository attach"));
    }
}
