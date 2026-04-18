use axum::{
    Extension,
    extract::{Query, State, ws::Message},
    response::IntoResponse,
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::container::ContainerService;

use super::capabilities::require_git_read;
use crate::{
    DeploymentImpl,
    error::ApiError,
    middleware::signed_ws::{MaybeSignedWebSocket, SignedWsUpgrade},
};

#[derive(Debug, Deserialize)]
pub struct DiffStreamQuery {
    #[serde(default)]
    pub stats_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceStreamQuery {
    pub archived: Option<bool>,
    pub limit: Option<i64>,
}

pub async fn stream_workspaces_ws(
    ws: SignedWsUpgrade,
    Query(query): Query<WorkspaceStreamQuery>,
    State(deployment): State<DeploymentImpl>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_workspaces_ws(socket, deployment, query.archived, query.limit).await
        {
            tracing::warn!("workspaces WS closed: {}", e);
        }
    })
}

pub async fn stream_workspace_diff_ws(
    ws: SignedWsUpgrade,
    Query(params): Query<DiffStreamQuery>,
    Extension(workspace): Extension<db::models::workspace::Workspace>,
    State(deployment): State<DeploymentImpl>,
) -> Result<impl IntoResponse, ApiError> {
    require_git_read(&workspace)?;

    let _ = deployment.container().touch(&workspace).await;
    let stats_only = params.stats_only;
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_workspace_diff_ws(socket, deployment, workspace, stats_only).await {
            tracing::warn!("diff WS closed: {}", e);
        }
    }))
}

async fn handle_workspace_diff_ws(
    mut socket: MaybeSignedWebSocket,
    deployment: DeploymentImpl,
    workspace: db::models::workspace::Workspace,
    stats_only: bool,
) -> anyhow::Result<()> {
    use futures_util::{StreamExt, TryStreamExt};
    use utils::log_msg::LogMsg;

    let stream = deployment
        .container()
        .stream_diff(&workspace, stats_only)
        .await?;

    let mut stream = stream.map_ok(|msg: LogMsg| msg.to_ws_message_unchecked());

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(Ok(msg)) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!("stream error: {}", e);
                        break;
                    }
                    None => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Ok(Some(Message::Close(_))) => break,
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

async fn handle_workspaces_ws(
    mut socket: MaybeSignedWebSocket,
    deployment: DeploymentImpl,
    archived: Option<bool>,
    limit: Option<i64>,
) -> anyhow::Result<()> {
    use futures_util::{StreamExt, TryStreamExt};

    let mut stream = deployment
        .events()
        .stream_workspaces_raw(archived, limit)
        .await?
        .map_ok(|msg| msg.to_ws_message_unchecked());

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(Ok(msg)) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!("stream error: {}", e);
                        break;
                    }
                    None => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Ok(Some(Message::Close(_))) => break,
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use axum::{
        Router,
        http::{HeaderValue, StatusCode, header},
    };
    use db::models::workspace::{CreateWorkspace, Workspace, WorkspaceMode};
    use deployment::Deployment;
    use tokio::net::TcpListener;
    use tokio_util::sync::CancellationToken;
    use utils::response::ApiResponse;
    use uuid::Uuid;

    use crate::DeploymentImpl;

    async fn start_app() -> (DeploymentImpl, String, tokio::task::JoinHandle<()>) {
        let deployment = <DeploymentImpl as Deployment>::new(CancellationToken::new())
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

        (deployment, format!("http://{address}"), server)
    }

    async fn create_workspace(
        deployment: &DeploymentImpl,
        workspace_mode: WorkspaceMode,
    ) -> Workspace {
        Workspace::create(
            &deployment.db().pool,
            &CreateWorkspace {
                branch: format!("diff-stream-test-{}", Uuid::new_v4()),
                workspace_mode,
                name: Some("Diff stream capability test".to_string()),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn stream_workspace_diff_ws_rejects_unsupported_mode_before_upgrade() {
        let (deployment, base_url, server) = start_app().await;
        let workspace = create_workspace(&deployment, WorkspaceMode::InPlaceDirectory).await;

        let response = reqwest::Client::new()
            .get(format!(
                "{base_url}/api/workspaces/{}/git/diff/ws",
                workspace.id
            ))
            .header(header::CONNECTION, HeaderValue::from_static("upgrade"))
            .header(header::UPGRADE, HeaderValue::from_static("websocket"))
            .header("Sec-WebSocket-Version", HeaderValue::from_static("13"))
            .header(
                "Sec-WebSocket-Key",
                HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
            )
            .send()
            .await
            .unwrap();

        let status = response.status();
        let body = response.text().await.unwrap();

        server.abort();
        let _ = server.await;
        let _ = Workspace::delete(&deployment.db().pool, workspace.id).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        let payload: ApiResponse<serde_json::Value> = serde_json::from_str(&body).unwrap();
        assert!(!payload.is_success());
        let message = payload.message().unwrap();
        assert!(message.contains("in_place_directory"));
        assert!(message.contains("git"));
    }
}
