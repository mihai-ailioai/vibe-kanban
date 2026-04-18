use axum::{Extension, response::Json as ResponseJson};
use db::models::workspace::{Workspace, WorkspaceMode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use utils::response::ApiResponse;

use crate::error::ApiError;

type WorkspaceCapabilityResult = Result<(), String>;

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
pub struct WorkspaceCapabilities {
    pub supports_git_read: bool,
    pub supports_git_write: bool,
    pub supports_pull_requests: bool,
    pub supports_repo_attach: bool,
    pub supports_delete_branches: bool,
}

impl WorkspaceCapabilities {
    pub fn for_mode(mode: WorkspaceMode) -> Self {
        match mode {
            WorkspaceMode::GitWorktree => Self {
                supports_git_read: true,
                supports_git_write: true,
                supports_pull_requests: true,
                supports_repo_attach: true,
                supports_delete_branches: true,
            },
            WorkspaceMode::InPlaceGit => Self {
                supports_git_read: true,
                supports_git_write: true,
                supports_pull_requests: true,
                supports_repo_attach: false,
                supports_delete_branches: false,
            },
            WorkspaceMode::InPlaceDirectory => Self {
                supports_git_read: false,
                supports_git_write: false,
                supports_pull_requests: false,
                supports_repo_attach: false,
                supports_delete_branches: false,
            },
        }
    }
}

pub fn workspace_capabilities(workspace: &Workspace) -> WorkspaceCapabilities {
    WorkspaceCapabilities::for_mode(workspace.workspace_mode)
}

pub fn require_git_read(workspace: &Workspace) -> WorkspaceCapabilityResult {
    require_capability(
        workspace,
        workspace_capabilities(workspace).supports_git_read,
        "git read operations",
    )
}

pub fn require_git_write(workspace: &Workspace) -> WorkspaceCapabilityResult {
    require_capability(
        workspace,
        workspace_capabilities(workspace).supports_git_write,
        "git write operations",
    )
}

pub fn require_pull_requests(workspace: &Workspace) -> WorkspaceCapabilityResult {
    require_capability(
        workspace,
        workspace_capabilities(workspace).supports_pull_requests,
        "pull request operations",
    )
}

pub fn require_repo_attach(workspace: &Workspace) -> WorkspaceCapabilityResult {
    require_capability(
        workspace,
        workspace_capabilities(workspace).supports_repo_attach,
        "repository attach operations",
    )
}

fn require_capability(
    workspace: &Workspace,
    is_supported: bool,
    operation: &str,
) -> WorkspaceCapabilityResult {
    if is_supported {
        Ok(())
    } else {
        Err(format!(
            "Workspace mode `{}` does not support {}.",
            workspace.workspace_mode, operation
        ))
    }
}

pub async fn get_workspace_capabilities(
    Extension(workspace): Extension<Workspace>,
) -> Result<ResponseJson<ApiResponse<WorkspaceCapabilities>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(workspace_capabilities(
        &workspace,
    ))))
}

#[cfg(test)]
mod tests {
    use axum::{Router, http::StatusCode};
    use chrono::Utc;
    use db::models::workspace::{CreateWorkspace, Workspace, WorkspaceMode};
    use deployment::Deployment;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;
    use utils::response::ApiResponse;
    use uuid::Uuid;

    use super::*;
    use crate::DeploymentImpl;

    fn sample_workspace(workspace_mode: WorkspaceMode) -> Workspace {
        Workspace {
            id: Uuid::new_v4(),
            task_id: None,
            container_ref: None,
            branch: format!("branch-{}", Uuid::new_v4()),
            workspace_mode,
            setup_completed_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived: false,
            pinned: false,
            name: Some("Capability test workspace".to_string()),
            worktree_deleted: false,
        }
    }

    #[test]
    fn workspace_capabilities_match_workspace_mode_matrix() {
        assert_eq!(
            WorkspaceCapabilities::for_mode(WorkspaceMode::GitWorktree),
            WorkspaceCapabilities {
                supports_git_read: true,
                supports_git_write: true,
                supports_pull_requests: true,
                supports_repo_attach: true,
                supports_delete_branches: true,
            }
        );

        assert_eq!(
            WorkspaceCapabilities::for_mode(WorkspaceMode::InPlaceGit),
            WorkspaceCapabilities {
                supports_git_read: true,
                supports_git_write: true,
                supports_pull_requests: true,
                supports_repo_attach: false,
                supports_delete_branches: false,
            }
        );

        assert_eq!(
            WorkspaceCapabilities::for_mode(WorkspaceMode::InPlaceDirectory),
            WorkspaceCapabilities {
                supports_git_read: false,
                supports_git_write: false,
                supports_pull_requests: false,
                supports_repo_attach: false,
                supports_delete_branches: false,
            }
        );
    }

    #[test]
    fn capability_helpers_allow_supported_modes_and_reject_unsupported_modes() {
        let in_place_git = sample_workspace(WorkspaceMode::InPlaceGit);
        assert_eq!(
            workspace_capabilities(&in_place_git),
            WorkspaceCapabilities::for_mode(WorkspaceMode::InPlaceGit)
        );
        assert!(require_git_read(&in_place_git).is_ok());
        assert!(require_git_write(&in_place_git).is_ok());
        assert!(require_pull_requests(&in_place_git).is_ok());

        let err = require_repo_attach(&in_place_git).unwrap_err();
        assert!(err.contains("in_place_git") && err.contains("repository attach"));

        let in_place_directory = sample_workspace(WorkspaceMode::InPlaceDirectory);
        let git_read_err = require_git_read(&in_place_directory).unwrap_err();
        assert!(git_read_err.contains("in_place_directory") && git_read_err.contains("git read"));

        let git_write_err = require_git_write(&in_place_directory).unwrap_err();
        assert!(
            git_write_err.contains("in_place_directory") && git_write_err.contains("git write")
        );

        let pr_err = require_pull_requests(&in_place_directory).unwrap_err();
        assert!(pr_err.contains("in_place_directory") && pr_err.contains("pull request"));
    }

    #[tokio::test]
    async fn get_workspace_capabilities_returns_in_place_git_matrix() {
        let deployment = <DeploymentImpl as Deployment>::new(CancellationToken::new())
            .await
            .unwrap();
        let workspace = Workspace::create(
            &deployment.db().pool,
            &CreateWorkspace {
                branch: format!("capabilities-test-{}", Uuid::new_v4()),
                workspace_mode: WorkspaceMode::InPlaceGit,
                name: Some("Capabilities route test".to_string()),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();

        let app = Router::new()
            .nest("/api", super::super::router(&deployment))
            .with_state(deployment.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let response = reqwest::get(format!(
            "http://{address}/api/workspaces/{}/capabilities",
            workspace.id
        ))
        .await
        .unwrap();

        let status = response.status();
        let body = response.bytes().await.unwrap();

        server.abort();
        let _ = server.await;

        let _ = Workspace::delete(&deployment.db().pool, workspace.id).await;

        assert_eq!(status, StatusCode::OK);

        let payload: ApiResponse<WorkspaceCapabilities> = serde_json::from_slice(&body).unwrap();
        assert!(payload.is_success());
        assert_eq!(
            payload.into_data(),
            Some(WorkspaceCapabilities::for_mode(WorkspaceMode::InPlaceGit))
        );
    }
}
