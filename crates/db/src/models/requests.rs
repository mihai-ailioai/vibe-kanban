use executors::profile::ExecutorConfig;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::{
    execution_process::ExecutionProcess,
    workspace::{Workspace, WorkspaceMode},
    workspace_source::WorkspaceSource,
};

#[derive(Debug, Deserialize, Serialize)]
pub struct ContainerQuery {
    #[serde(rename = "ref")]
    pub container_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct WorkspaceRepoInput {
    pub repo_id: Uuid,
    pub target_branch: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct CreateWorkspaceApiRequest {
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct LinkedIssueInfo {
    pub remote_project_id: Uuid,
    pub issue_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum WorkspaceSourceInput {
    GitRepo {
        repo_id: Uuid,
        target_branch: String,
    },
    Directory {
        path: String,
        display_name: Option<String>,
    },
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct CreateAndStartWorkspaceRequest {
    pub name: Option<String>,
    #[serde(default)]
    pub workspace_mode: WorkspaceMode,
    #[serde(default)]
    pub sources: Vec<WorkspaceSourceInput>,
    #[serde(default)]
    pub repos: Vec<WorkspaceRepoInput>,
    pub linked_issue: Option<LinkedIssueInfo>,
    pub executor_config: ExecutorConfig,
    pub prompt: String,
    pub attachment_ids: Option<Vec<Uuid>>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(rename = "CreateAndStartWorkspaceRequest")]
pub enum CreateAndStartWorkspaceRequestTs {
    Legacy {
        name: Option<String>,
        repos: Vec<WorkspaceRepoInput>,
        linked_issue: Option<LinkedIssueInfo>,
        executor_config: ExecutorConfig,
        prompt: String,
        attachment_ids: Option<Vec<Uuid>>,
    },
    New {
        name: Option<String>,
        #[ts(optional)]
        workspace_mode: Option<WorkspaceMode>,
        #[ts(optional)]
        sources: Option<Vec<WorkspaceSourceInput>>,
        linked_issue: Option<LinkedIssueInfo>,
        executor_config: ExecutorConfig,
        prompt: String,
        attachment_ids: Option<Vec<Uuid>>,
    },
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct CreateAndStartWorkspaceResponse {
    pub workspace: Workspace,
    pub sources: Vec<WorkspaceSource>,
    pub execution_process: ExecutionProcess,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct UpdateWorkspace {
    pub archived: Option<bool>,
    pub pinned: Option<bool>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, TS)]
pub struct UpdateSession {
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use sqlx::types::Json;
    use ts_rs::TS;
    use uuid::Uuid;

    use super::{CreateAndStartWorkspaceResponse, WorkspaceRepoInput};
    use crate::models::{
        execution_process::{
            ExecutionProcess, ExecutionProcessRunReason, ExecutionProcessStatus,
            ExecutorActionField,
        },
        workspace::{Workspace, WorkspaceMode},
        workspace_source::{WorkspaceSource, WorkspaceSourceKind},
    };

    #[test]
    fn create_and_start_workspace_request_defaults_legacy_repo_payload() {
        let repo_id = Uuid::new_v4();
        let request: super::CreateAndStartWorkspaceRequest = serde_json::from_value(json!({
            "name": "legacy request",
            "repos": [{
                "repo_id": repo_id,
                "target_branch": "main"
            }],
            "executor_config": {
                "executor": "codex"
            },
            "prompt": "hello"
        }))
        .unwrap();

        assert_eq!(request.workspace_mode, WorkspaceMode::GitWorktree);
        assert!(request.sources.is_empty());
        assert_eq!(
            request.repos,
            vec![WorkspaceRepoInput {
                repo_id,
                target_branch: "main".to_string(),
            }]
        );
    }

    #[test]
    fn create_and_start_workspace_response_serializes_canonical_sources() {
        let workspace_id = Uuid::new_v4();
        let source_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let process_id = Uuid::new_v4();
        let timestamp = Utc::now();
        let repo_id = Uuid::new_v4();

        let response = CreateAndStartWorkspaceResponse {
            workspace: Workspace {
                id: workspace_id,
                task_id: None,
                container_ref: None,
                branch: "feature/test".to_string(),
                workspace_mode: WorkspaceMode::GitWorktree,
                setup_completed_at: None,
                created_at: timestamp,
                updated_at: timestamp,
                archived: false,
                pinned: false,
                name: Some("Test workspace".to_string()),
                worktree_deleted: false,
            },
            sources: vec![WorkspaceSource {
                id: source_id,
                workspace_id,
                source_type: WorkspaceSourceKind::GitRepo,
                repo_id: Some(repo_id),
                path: None,
                display_name: None,
                target_branch: Some("main".to_string()),
                position: 0,
                created_at: timestamp,
                updated_at: timestamp,
            }],
            execution_process: ExecutionProcess {
                id: process_id,
                session_id,
                run_reason: ExecutionProcessRunReason::CodingAgent,
                executor_action: Json(ExecutorActionField::Other(json!({"type": "noop"}))),
                status: ExecutionProcessStatus::Running,
                exit_code: None,
                dropped: false,
                started_at: timestamp,
                completed_at: None,
                created_at: timestamp,
                updated_at: timestamp,
            },
        };

        let value = serde_json::to_value(response).unwrap();

        assert_eq!(
            value["sources"],
            json!([{
                "id": source_id,
                "workspace_id": workspace_id,
                "source_type": "git_repo",
                "repo_id": repo_id,
                "path": null,
                "display_name": null,
                "target_branch": "main",
                "position": 0,
                "created_at": timestamp,
                "updated_at": timestamp,
            }])
        );
    }

    #[test]
    fn create_and_start_workspace_request_ts_decl_is_transition_union() {
        let decl = super::CreateAndStartWorkspaceRequestTs::decl();

        assert!(decl.contains("| {"), "{decl}");
        assert!(decl.contains("repos: Array<WorkspaceRepoInput>"), "{decl}");
        assert!(decl.contains("workspace_mode?: WorkspaceMode"), "{decl}");
        assert!(
            decl.contains("sources?: Array<WorkspaceSourceInput>"),
            "{decl}"
        );
        assert!(
            !decl.contains("repos?: Array<WorkspaceRepoInput>"),
            "{decl}"
        );
    }
}
