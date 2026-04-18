use axum::{
    Extension, Json,
    extract::{Query, State},
    http::StatusCode,
    response::Json as ResponseJson,
};
use db::models::{
    coding_agent_turn::CodingAgentTurn,
    execution_process::ExecutionProcess,
    workspace::{Workspace, WorkspaceError},
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::{container::ContainerService, diff_stream, remote_sync};
use sqlx::Error as SqlxError;
use utils::response::ApiResponse;
use workspace_manager::WorkspaceManager;

use super::capabilities::workspace_capabilities;
use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize)]
pub struct DeleteWorkspaceQuery {
    #[serde(default)]
    pub delete_remote: bool,
    #[serde(default)]
    pub delete_branches: bool,
}

pub async fn get_workspaces(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<Workspace>>>, ApiError> {
    let pool = &deployment.db().pool;
    let workspaces = Workspace::fetch_all(pool).await?;
    Ok(ResponseJson(ApiResponse::success(workspaces)))
}

pub async fn get_workspace(
    Extension(workspace): Extension<Workspace>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(workspace)))
}

pub async fn update_workspace(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    Json(request): Json<db::models::requests::UpdateWorkspace>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    let pool = &deployment.db().pool;
    let is_archiving = request.archived == Some(true) && !workspace.archived;

    Workspace::update(
        pool,
        workspace.id,
        request.archived,
        request.pinned,
        request.name.as_deref(),
    )
    .await?;
    let updated = Workspace::find_by_id(pool, workspace.id)
        .await?
        .ok_or(WorkspaceError::WorkspaceNotFound)?;

    if (request.archived.is_some() || request.name.is_some())
        && let Ok(client) = deployment.remote_client()
    {
        let ws = updated.clone();
        let name = request.name.clone();
        let archived = request.archived;
        let stats = if workspace_capabilities(&ws).supports_git_read {
            diff_stream::compute_diff_stats(&deployment.db().pool, deployment.git(), &ws).await
        } else {
            None
        };
        tokio::spawn(async move {
            remote_sync::sync_workspace_to_remote(
                &client,
                ws.id,
                name.map(Some),
                archived,
                stats.as_ref(),
            )
            .await;
        });
    }

    if is_archiving && let Err(e) = deployment.container().archive_workspace(workspace.id).await {
        tracing::error!("Failed to archive workspace {}: {}", workspace.id, e);
    }

    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn get_first_user_message(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Option<String>>>, ApiError> {
    let pool = &deployment.db().pool;
    let message = Workspace::get_first_user_message(pool, workspace.id).await?;
    Ok(ResponseJson(ApiResponse::success(message)))
}

async fn stop_workspace_for_delete<F, Fut>(
    pool: &sqlx::SqlitePool,
    workspace: &Workspace,
    stop_workspace: F,
) -> Result<(), ApiError>
where
    F: FnOnce(Workspace, bool) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    if ExecutionProcess::has_running_non_dev_server_processes_for_workspace(pool, workspace.id)
        .await?
    {
        return Err(ApiError::Conflict(
            "Cannot delete workspace while processes are running. Stop all processes first."
                .to_string(),
        ));
    }

    stop_workspace(workspace.clone(), true).await;
    Ok(())
}

pub async fn delete_workspace(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<DeleteWorkspaceQuery>,
) -> Result<(StatusCode, ResponseJson<ApiResponse<()>>), ApiError> {
    let pool = deployment.db().pool.clone();
    let workspace_manager = deployment.workspace_manager();
    let workspace_id = workspace.id;
    let deployment_for_stop = deployment.clone();

    stop_workspace_for_delete(
        &pool,
        &workspace,
        |workspace, include_dev_server| async move {
            deployment_for_stop
                .container()
                .try_stop(&workspace, include_dev_server)
                .await;
        },
    )
    .await?;

    let managed_workspace = workspace_manager.load_managed_workspace(workspace).await?;
    let deletion_context = managed_workspace.prepare_deletion_context().await?;
    let rows_affected = managed_workspace.delete_record().await?;

    if rows_affected == 0 {
        return Err(ApiError::Database(SqlxError::RowNotFound));
    }

    deployment
        .track_if_analytics_allowed(
            "workspace_deleted",
            serde_json::json!({
                "workspace_id": workspace_id.to_string(),
            }),
        )
        .await;

    if query.delete_remote {
        if let Ok(client) = deployment.remote_client() {
            match client.delete_workspace(workspace_id).await {
                Ok(()) => {
                    tracing::info!("Deleted remote workspace for {}", workspace_id);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to delete remote workspace for {}: {}",
                        workspace_id,
                        e
                    );
                }
            }
        } else {
            tracing::debug!(
                "Remote client not available, skipping remote deletion for {}",
                workspace_id
            );
        }
    }

    WorkspaceManager::spawn_workspace_deletion_cleanup(deletion_context, query.delete_branches);

    Ok((StatusCode::ACCEPTED, ResponseJson(ApiResponse::success(()))))
}

#[axum::debug_handler]
pub async fn mark_seen(
    Extension(workspace): Extension<Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let pool = &deployment.db().pool;
    CodingAgentTurn::mark_seen_by_workspace_id(pool, workspace.id).await?;
    Ok(ResponseJson(ApiResponse::success(())))
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        str::FromStr,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use anyhow::anyhow;
    use db::models::{
        execution_process::{
            CreateExecutionProcess, ExecutionProcess, ExecutionProcessRunReason,
            ExecutionProcessStatus,
        },
        repo::Repo,
        session::{CreateSession, Session},
        workspace::WorkspaceMode,
        workspace_repo_claim::{CreateWorkspaceRepoClaim, WorkspaceRepoClaim},
    };
    use executors::actions::{
        ExecutorAction, ExecutorActionType,
        script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
    };
    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use uuid::Uuid;

    use super::*;

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "server-core-delete-tests-{}.sqlite",
            Uuid::new_v4()
        ));
        let database_url = format!("sqlite://{}", db_path.to_string_lossy());

        let options = SqliteConnectOptions::from_str(&database_url)
            .unwrap()
            .create_if_missing(true)
            .disable_statement_logging();

        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::migrate!("../db/migrations").run(&pool).await.unwrap();
        pool
    }

    fn script_action() -> ExecutorAction {
        ExecutorAction::new(
            ExecutorActionType::ScriptRequest(ScriptRequest {
                script: "sleep 60".to_string(),
                language: ScriptRequestLanguage::Bash,
                context: ScriptContext::DevServer,
                working_dir: None,
            }),
            None,
        )
    }

    #[tokio::test]
    async fn stop_workspace_for_delete_with_only_dev_servers_uses_try_stop_hook_to_release_claims()
    {
        let pool = test_pool().await;
        let workspace = Workspace::create(
            &pool,
            &db::models::workspace::CreateWorkspace {
                branch: format!("delete-dev-server-{}", Uuid::new_v4()),
                workspace_mode: WorkspaceMode::InPlaceGit,
                name: Some("Delete dev server workspace".to_string()),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap();

        let repo_path =
            std::env::temp_dir().join(format!("server-core-delete-claim-repo-{}", Uuid::new_v4()));
        let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "claim-repo")
            .await
            .unwrap();
        WorkspaceRepoClaim::create_many(
            &pool,
            workspace.id,
            &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
        )
        .await
        .unwrap();

        let session = Session::create(
            &pool,
            &CreateSession {
                executor: None,
                name: Some("Delete test session".to_string()),
            },
            Uuid::new_v4(),
            workspace.id,
        )
        .await
        .unwrap();

        let execution = ExecutionProcess::create(
            &pool,
            &CreateExecutionProcess {
                session_id: session.id,
                executor_action: script_action(),
                run_reason: ExecutionProcessRunReason::DevServer,
            },
            Uuid::new_v4(),
            &[],
        )
        .await
        .unwrap();

        let after_stop_called = Arc::new(AtomicBool::new(false));

        assert_eq!(
            WorkspaceRepoClaim::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap()
                .len(),
            1
        );

        let stop_pool = pool.clone();
        let stop_after_stop_called = after_stop_called.clone();

        stop_workspace_for_delete(&pool, &workspace, move |workspace, include_dev_server| {
            let pool = stop_pool.clone();
            let after_stop_called = stop_after_stop_called.clone();
            async move {
                assert!(include_dev_server);
                let running_dev_servers =
                    ExecutionProcess::find_running_dev_servers_by_workspace(&pool, workspace.id)
                        .await
                        .unwrap();

                for dev_server in running_dev_servers {
                    ExecutionProcess::update_completion(
                        &pool,
                        dev_server.id,
                        ExecutionProcessStatus::Killed,
                        None,
                    )
                    .await
                    .unwrap();
                }

                after_stop_called.store(true, Ordering::SeqCst);
                WorkspaceRepoClaim::release_for_workspace(&pool, workspace.id)
                    .await
                    .unwrap();
            }
        })
        .await
        .unwrap();

        assert!(after_stop_called.load(Ordering::SeqCst));
        assert!(
            WorkspaceRepoClaim::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap()
                .is_empty()
        );

        let execution = ExecutionProcess::find_by_id(&pool, execution.id)
            .await
            .unwrap()
            .ok_or_else(|| anyhow!("missing execution process after stop"))
            .unwrap();
        assert_eq!(execution.status, ExecutionProcessStatus::Killed);
    }
}
