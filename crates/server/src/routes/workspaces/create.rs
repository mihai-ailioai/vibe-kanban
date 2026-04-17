use std::{collections::HashMap, path::PathBuf};

use axum::{Json, extract::State, response::Json as ResponseJson};
use db::models::{
    execution_process::ExecutionProcess,
    requests::{
        CreateAndStartWorkspaceRequest, CreateAndStartWorkspaceResponse, CreateWorkspaceApiRequest,
        LinkedIssueInfo, WorkspaceRepoInput, WorkspaceSourceInput,
    },
    workspace::{CreateWorkspace, Workspace, WorkspaceMode},
    workspace_repo::WorkspaceRepo,
    workspace_source::{CreateWorkspaceSource, WorkspaceSource},
};
use deployment::Deployment;
use executors::profile::ExecutorConfig;
use git::GitService;
use local_deployment::container::cleanup_in_place_git_workspace_root;
use services::services::container::ContainerService;
use sqlx::SqlitePool;
use utils::response::ApiResponse;
use uuid::Uuid;
use workspace_manager::{ManagedWorkspace, WorkspaceManager};

use crate::{
    DeploymentImpl,
    error::ApiError,
    routes::workspaces::attachments::{
        ImportedIssueAttachment, import_issue_attachments_from_remote,
    },
};

pub(crate) async fn create_workspace_record(
    deployment: &DeploymentImpl,
    name: Option<String>,
    workspace_mode: WorkspaceMode,
) -> Result<Workspace, ApiError> {
    let workspace_id = Uuid::new_v4();
    let branch_label = name
        .as_deref()
        .filter(|branch_label| !branch_label.is_empty())
        .unwrap_or("workspace");
    let git_branch_name = deployment
        .container()
        .git_branch_from_workspace(&workspace_id, branch_label)
        .await;

    let workspace = Workspace::create(
        &deployment.db().pool,
        &CreateWorkspace {
            branch: git_branch_name,
            workspace_mode,
            name: name.filter(|workspace_name| !workspace_name.is_empty()),
        },
        workspace_id,
    )
    .await?;

    Ok(workspace)
}

pub async fn create_workspace(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateWorkspaceApiRequest>,
) -> Result<ResponseJson<ApiResponse<Workspace>>, ApiError> {
    let workspace =
        create_workspace_record(&deployment, payload.name, WorkspaceMode::GitWorktree).await?;

    deployment
        .track_if_analytics_allowed(
            "workspace_created",
            serde_json::json!({
                "workspace_id": workspace.id.to_string(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(workspace)))
}

fn normalize_prompt(prompt: &str) -> Option<String> {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NormalizedWorkspaceSources {
    workspace_mode: WorkspaceMode,
    sources: Vec<WorkspaceSourceInput>,
    legacy_git_repos: Vec<WorkspaceRepoInput>,
}

fn git_source_to_workspace_repo(
    source: &WorkspaceSourceInput,
) -> Result<WorkspaceRepoInput, ApiError> {
    match source {
        WorkspaceSourceInput::GitRepo {
            repo_id,
            target_branch,
        } => Ok(WorkspaceRepoInput {
            repo_id: *repo_id,
            target_branch: target_branch.clone(),
        }),
        WorkspaceSourceInput::Directory { .. } => Err(ApiError::BadRequest(
            "Workspace modes `git_worktree` and `in_place_git` only support `git_repo` sources."
                .to_string(),
        )),
    }
}

fn validate_sources_for_mode(
    workspace_mode: WorkspaceMode,
    sources: &[WorkspaceSourceInput],
) -> Result<(), ApiError> {
    match workspace_mode {
        WorkspaceMode::GitWorktree => {
            if sources
                .iter()
                .any(|source| !matches!(source, WorkspaceSourceInput::GitRepo { .. }))
            {
                return Err(ApiError::BadRequest(
                    "Workspace mode `git_worktree` only supports `git_repo` sources.".to_string(),
                ));
            }
        }
        WorkspaceMode::InPlaceGit => {
            if sources
                .iter()
                .any(|source| !matches!(source, WorkspaceSourceInput::GitRepo { .. }))
            {
                return Err(ApiError::BadRequest(
                    "Workspace mode `in_place_git` only supports `git_repo` sources.".to_string(),
                ));
            }
        }
        WorkspaceMode::InPlaceDirectory => {
            if sources
                .iter()
                .any(|source| !matches!(source, WorkspaceSourceInput::Directory { .. }))
            {
                return Err(ApiError::BadRequest(
                    "Workspace mode `in_place_directory` only supports `directory` sources and does not support `git_repo` sources."
                        .to_string(),
                ));
            }
        }
    }

    Ok(())
}

fn normalize_workspace_sources(
    workspace_mode: WorkspaceMode,
    sources: Vec<WorkspaceSourceInput>,
    repos: Vec<WorkspaceRepoInput>,
) -> Result<NormalizedWorkspaceSources, ApiError> {
    if !sources.is_empty() && !repos.is_empty() {
        return Err(ApiError::BadRequest(
            "Provide either `sources` or legacy `repos`, not both.".to_string(),
        ));
    }

    if sources.is_empty() && !repos.is_empty() && workspace_mode != WorkspaceMode::GitWorktree {
        return Err(ApiError::BadRequest(
            "Legacy `repos` input is only supported with workspace mode `git_worktree`."
                .to_string(),
        ));
    }

    let (workspace_mode, sources) = if sources.is_empty() && !repos.is_empty() {
        (
            WorkspaceMode::GitWorktree,
            repos
                .into_iter()
                .map(|repo| WorkspaceSourceInput::GitRepo {
                    repo_id: repo.repo_id,
                    target_branch: repo.target_branch,
                })
                .collect(),
        )
    } else {
        (workspace_mode, sources)
    };

    if sources.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one workspace source is required.".to_string(),
        ));
    }

    validate_sources_for_mode(workspace_mode, &sources)?;

    let legacy_git_repos = match workspace_mode {
        WorkspaceMode::GitWorktree => sources
            .iter()
            .map(git_source_to_workspace_repo)
            .collect::<Result<Vec<_>, _>>()?,
        WorkspaceMode::InPlaceGit | WorkspaceMode::InPlaceDirectory => Vec::new(),
    };

    Ok(NormalizedWorkspaceSources {
        workspace_mode,
        sources,
        legacy_git_repos,
    })
}

#[cfg(test)]
fn normalize_workspace_request(
    request: CreateAndStartWorkspaceRequest,
) -> Result<NormalizedWorkspaceSources, ApiError> {
    normalize_workspace_sources(request.workspace_mode, request.sources, request.repos)
}

fn canonical_workspace_sources(sources: &[WorkspaceSourceInput]) -> Vec<CreateWorkspaceSource> {
    sources
        .iter()
        .map(|source| match source {
            WorkspaceSourceInput::GitRepo {
                repo_id,
                target_branch,
            } => CreateWorkspaceSource::GitRepo {
                repo_id: *repo_id,
                target_branch: target_branch.clone(),
            },
            WorkspaceSourceInput::Directory { path, display_name } => {
                CreateWorkspaceSource::Directory {
                    path: path.clone(),
                    display_name: display_name.clone(),
                }
            }
        })
        .collect()
}

async fn persist_workspace_sources(
    pool: &SqlitePool,
    workspace_id: Uuid,
    sources: &[WorkspaceSourceInput],
) -> Result<Vec<WorkspaceSource>, sqlx::Error> {
    let create_sources = canonical_workspace_sources(sources);
    WorkspaceSource::create_many(pool, workspace_id, &create_sources).await
}

fn escape_markdown_label(label: &str) -> String {
    let mut escaped = String::with_capacity(label.len());
    for ch in label.chars() {
        if matches!(ch, '[' | ']' | '\\') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

fn build_workspace_attachment_markdown(
    file: &ImportedIssueAttachment,
    label: &str,
    uses_image_markdown: bool,
) -> String {
    let path = format!(".vibe-attachments/{}", file.file.file_path);
    let normalized_label = if label.trim().is_empty() {
        file.file.original_name.as_str()
    } else {
        label
    };
    let escaped_label = escape_markdown_label(normalized_label);

    if uses_image_markdown {
        format!("![{}]({})", escaped_label, path)
    } else {
        format!("[{}]({})", escaped_label, path)
    }
}

struct ParsedAttachmentMarkdown<'a> {
    attachment_id: Uuid,
    label: &'a str,
    uses_image_markdown: bool,
    end: usize,
}

fn find_unescaped_char(haystack: &str, target: char) -> Option<usize> {
    let mut escaped = false;

    for (index, ch) in haystack.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' {
            escaped = true;
            continue;
        }

        if ch == target {
            return Some(index);
        }
    }

    None
}

fn parse_attachment_markdown_at(
    prompt: &str,
    start: usize,
) -> Option<ParsedAttachmentMarkdown<'_>> {
    let rest = prompt.get(start..)?;
    let (uses_image_markdown, label_start_offset) = if rest.starts_with("![") {
        (true, 2)
    } else if rest.starts_with('[') {
        (false, 1)
    } else {
        return None;
    };

    let label_rest = rest.get(label_start_offset..)?;
    let label_end_offset = find_unescaped_char(label_rest, ']')?;
    let label = &label_rest[..label_end_offset];

    let after_label = label_rest.get(label_end_offset + 1..)?;
    let attachment_prefix = "(attachment://";
    if !after_label.starts_with(attachment_prefix) {
        return None;
    }

    let attachment_id_start =
        start + label_start_offset + label_end_offset + 1 + attachment_prefix.len();
    let attachment_id_rest = prompt.get(attachment_id_start..)?;
    let attachment_id_end_offset = attachment_id_rest.find(')')?;
    let attachment_id = Uuid::parse_str(&attachment_id_rest[..attachment_id_end_offset]).ok()?;

    Some(ParsedAttachmentMarkdown {
        attachment_id,
        label,
        uses_image_markdown,
        end: attachment_id_start + attachment_id_end_offset + 1,
    })
}

fn rewrite_imported_issue_attachments_markdown(
    prompt: &str,
    imported_attachments: &[ImportedIssueAttachment],
) -> String {
    if imported_attachments.is_empty() {
        return prompt.to_string();
    }

    let imported_by_attachment_id = imported_attachments
        .iter()
        .map(|attachment| (attachment.attachment_id, attachment))
        .collect::<HashMap<_, _>>();
    let mut rewritten = String::with_capacity(prompt.len());
    let mut index = 0;

    while index < prompt.len() {
        if let Some(parsed) = parse_attachment_markdown_at(prompt, index)
            && let Some(attachment) = imported_by_attachment_id.get(&parsed.attachment_id)
        {
            rewritten.push_str(&build_workspace_attachment_markdown(
                attachment,
                parsed.label,
                parsed.uses_image_markdown,
            ));
            index = parsed.end;
            continue;
        }

        let Some(ch) = prompt[index..].chars().next() else {
            break;
        };
        rewritten.push(ch);
        index += ch.len_utf8();
    }

    rewritten
}

fn inject_linked_issue_prompt_context(
    prompt: &str,
    linked_issue: Option<&LinkedIssueInfo>,
    organization_id: Option<Uuid>,
) -> String {
    let Some(linked_issue) = linked_issue else {
        return prompt.to_string();
    };

    let Some(organization_id) = organization_id else {
        return prompt.to_string();
    };

    format!(
        "organization_id: {organization_id}\nproject_id: {}\nissue_id: {}\n\n{prompt}",
        linked_issue.remote_project_id, linked_issue.issue_id,
    )
}

async fn cleanup_failed_create_and_start_workspace(pool: &SqlitePool, workspace_id: Uuid) {
    let workspace = match Workspace::find_by_id(pool, workspace_id).await {
        Ok(Some(workspace)) => workspace,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(
                "Failed to load workspace {} during create/start cleanup: {}",
                workspace_id,
                e
            );
            return;
        }
    };

    let workspace_dir = workspace.container_ref.clone().map(PathBuf::from);
    let repositories = match WorkspaceRepo::find_repos_for_workspace(pool, workspace_id).await {
        Ok(repos) => repos,
        Err(e) => {
            tracing::warn!(
                "Failed to find repos for workspace {} during create/start cleanup: {}",
                workspace_id,
                e
            );
            vec![]
        }
    };

    if let Err(e) = Workspace::delete(pool, workspace_id).await {
        tracing::warn!(
            "Failed to delete workspace {} during create/start cleanup: {}",
            workspace_id,
            e
        );
    }

    if let Some(workspace_dir) = workspace_dir {
        tokio::spawn(async move {
            let cleanup_result = match workspace.workspace_mode {
                WorkspaceMode::InPlaceGit => cleanup_in_place_git_workspace_root(&workspace_dir)
                    .await
                    .map_err(|e| e.to_string()),
                WorkspaceMode::GitWorktree | WorkspaceMode::InPlaceDirectory => {
                    WorkspaceManager::cleanup_workspace(&workspace_dir, &repositories)
                        .await
                        .map_err(|e| e.to_string())
                }
            };

            if let Err(e) = cleanup_result {
                tracing::error!(
                    "Background cleanup failed for workspace {} at {}: {}",
                    workspace_id,
                    workspace_dir.display(),
                    e
                );
            }
        });
    }
}

struct PreparedCreateAndStartWorkspace {
    pool: SqlitePool,
    managed_workspace: ManagedWorkspace,
    linked_issue: Option<LinkedIssueInfo>,
    executor_config: ExecutorConfig,
    workspace_prompt: String,
    persisted_sources: Vec<WorkspaceSource>,
}

async fn prepare_create_and_start_workspace<CreateWorkspaceRecord, CreateWorkspaceRecordFuture>(
    pool: &SqlitePool,
    workspace_manager: &WorkspaceManager,
    git: &GitService,
    payload: CreateAndStartWorkspaceRequest,
    create_workspace_record: CreateWorkspaceRecord,
) -> Result<PreparedCreateAndStartWorkspace, ApiError>
where
    CreateWorkspaceRecord: FnOnce(Option<String>, WorkspaceMode) -> CreateWorkspaceRecordFuture,
    CreateWorkspaceRecordFuture: std::future::Future<Output = Result<Workspace, ApiError>>,
{
    let CreateAndStartWorkspaceRequest {
        name,
        workspace_mode,
        sources,
        repos,
        linked_issue,
        executor_config,
        prompt,
        attachment_ids,
        ..
    } = payload;

    let normalized_request = normalize_workspace_sources(workspace_mode, sources, repos)?;
    let workspace_mode = normalized_request.workspace_mode;
    let sources = normalized_request.sources;
    let repos = normalized_request.legacy_git_repos;

    let workspace_prompt = normalize_prompt(&prompt).ok_or_else(|| {
        ApiError::BadRequest(
            "A workspace prompt is required. Provide a non-empty `prompt`.".to_string(),
        )
    })?;

    if workspace_mode == WorkspaceMode::GitWorktree && repos.is_empty() {
        return Err(ApiError::BadRequest(
            "At least one repository is required".to_string(),
        ));
    }

    let workspace = create_workspace_record(name, workspace_mode).await?;
    let workspace_id = workspace.id;

    let prepared = async {
        let persisted_sources = persist_workspace_sources(pool, workspace.id, &sources).await?;

        let mut managed_workspace = workspace_manager.load_managed_workspace(workspace).await?;

        if workspace_mode == WorkspaceMode::GitWorktree {
            for repo in &repos {
                managed_workspace
                    .add_repository(repo, git)
                    .await
                    .map_err(ApiError::from)?;
            }
        }

        if let Some(ids) = &attachment_ids {
            managed_workspace.associate_attachments(ids).await?;
        }

        Ok(PreparedCreateAndStartWorkspace {
            pool: pool.clone(),
            managed_workspace,
            linked_issue,
            executor_config,
            workspace_prompt,
            persisted_sources,
        })
    }
    .await;

    match prepared {
        Ok(prepared) => Ok(prepared),
        Err(err) => {
            cleanup_failed_create_and_start_workspace(pool, workspace_id).await;
            Err(err)
        }
    }
}

async fn start_prepared_workspace<StartWorkspace, StartWorkspaceFuture>(
    prepared: PreparedCreateAndStartWorkspace,
    linked_issue_organization_id: Option<Uuid>,
    start_workspace: StartWorkspace,
) -> Result<CreateAndStartWorkspaceResponse, ApiError>
where
    StartWorkspace: FnOnce(Workspace, ExecutorConfig, String) -> StartWorkspaceFuture,
    StartWorkspaceFuture: std::future::Future<Output = Result<ExecutionProcess, ApiError>>,
{
    let workspace_id = prepared.managed_workspace.workspace.id;

    let workspace_prompt = inject_linked_issue_prompt_context(
        &prepared.workspace_prompt,
        prepared.linked_issue.as_ref(),
        linked_issue_organization_id,
    );

    let workspace = prepared.managed_workspace.workspace.clone();
    tracing::info!("Created workspace {}", workspace.id);

    let execution_process = start_workspace(
        workspace.clone(),
        prepared.executor_config,
        workspace_prompt,
    )
    .await;

    match execution_process {
        Ok(execution_process) => Ok(CreateAndStartWorkspaceResponse {
            workspace,
            sources: prepared.persisted_sources,
            execution_process,
        }),
        Err(err) => {
            cleanup_failed_create_and_start_workspace(&prepared.pool, workspace_id).await;
            Err(err)
        }
    }
}

pub async fn create_and_start_workspace(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateAndStartWorkspaceRequest>,
) -> Result<ResponseJson<ApiResponse<CreateAndStartWorkspaceResponse>>, ApiError> {
    let analytics_executor = payload.executor_config.executor.clone();
    let analytics_variant = payload.executor_config.variant.clone();

    let mut prepared = prepare_create_and_start_workspace(
        &deployment.db().pool,
        deployment.workspace_manager(),
        deployment.git(),
        payload,
        |name, workspace_mode| create_workspace_record(&deployment, name, workspace_mode),
    )
    .await?;

    let mut linked_issue_organization_id = None;

    if let Some(linked_issue) = &prepared.linked_issue
        && let Ok(client) = deployment.remote_client()
    {
        match client
            .get_remote_project(linked_issue.remote_project_id)
            .await
        {
            Ok(project) => {
                linked_issue_organization_id = Some(project.organization_id);
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to fetch remote project {} for linked issue {}: {}",
                    linked_issue.remote_project_id,
                    linked_issue.issue_id,
                    e
                );
            }
        }

        match import_issue_attachments_from_remote(
            &client,
            deployment.file(),
            linked_issue.issue_id,
        )
        .await
        {
            Ok(imported_attachments) if !imported_attachments.is_empty() => {
                let imported_ids = imported_attachments
                    .iter()
                    .map(|imported| imported.file.id)
                    .collect::<Vec<_>>();

                if let Err(e) = prepared
                    .managed_workspace
                    .associate_attachments(&imported_ids)
                    .await
                {
                    tracing::warn!("Failed to associate imported files with workspace: {}", e);
                }

                prepared.workspace_prompt = rewrite_imported_issue_attachments_markdown(
                    &prepared.workspace_prompt,
                    &imported_attachments,
                );

                tracing::info!(
                    "Imported {} files from issue {}",
                    imported_ids.len(),
                    linked_issue.issue_id
                );
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    "Failed to import issue attachments for issue {}: {}",
                    linked_issue.issue_id,
                    e
                );
            }
        }
    }

    let deployment_for_start = deployment.clone();
    let response = start_prepared_workspace(
        prepared,
        linked_issue_organization_id,
        |workspace, executor_config, workspace_prompt| async move {
            deployment_for_start
                .container()
                .start_workspace(&workspace, executor_config, workspace_prompt)
                .await
                .map_err(ApiError::from)
        },
    )
    .await?;

    deployment
        .track_if_analytics_allowed(
            "workspace_created_and_started",
            serde_json::json!({
                "executor": &analytics_executor,
                "variant": &analytics_variant,
                "workspace_id": response.workspace.id.to_string(),
            }),
        )
        .await;

    Ok(ResponseJson(ApiResponse::success(response)))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        str::FromStr,
        sync::{Arc, Mutex},
    };

    use chrono::Utc;
    use db::{
        DBService,
        models::{
            execution_process::{
                ExecutionProcess, ExecutionProcessRunReason, ExecutionProcessStatus,
                ExecutorActionField,
            },
            file::File,
            repo::Repo,
            requests::{
                CreateAndStartWorkspaceRequest, LinkedIssueInfo, WorkspaceRepoInput,
                WorkspaceSourceInput,
            },
            session::{CreateSession, Session},
            workspace::{CreateWorkspace, Workspace, WorkspaceMode},
            workspace_repo::WorkspaceRepo,
            workspace_repo_claim::{CreateWorkspaceRepoClaim, WorkspaceRepoClaim},
            workspace_source::{WorkspaceSource, WorkspaceSourceKind},
        },
    };
    use executors::{executors::BaseCodingAgent, profile::ExecutorConfig};
    use git::GitService;
    use serde_json::json;
    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions, types::Json};
    use tokio::runtime::Builder;
    use uuid::Uuid;
    use workspace_manager::WorkspaceManager;

    use super::{
        ApiError, ImportedIssueAttachment, inject_linked_issue_prompt_context,
        normalize_workspace_request, normalize_workspace_sources, persist_workspace_sources,
        prepare_create_and_start_workspace, rewrite_imported_issue_attachments_markdown,
        start_prepared_workspace,
    };

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "server-create-route-tests-{}.sqlite",
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

    fn run_async_test<F>(future: F)
    where
        F: std::future::Future<Output = ()>,
    {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future);
    }

    fn linked_issue() -> LinkedIssueInfo {
        LinkedIssueInfo {
            remote_project_id: Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap(),
            issue_id: Uuid::parse_str("cccccccc-cccc-cccc-cccc-cccccccccccc").unwrap(),
        }
    }

    fn imported_file(
        attachment_id: Uuid,
        original_name: &str,
        file_path: &str,
        mime_type: Option<&str>,
    ) -> ImportedIssueAttachment {
        ImportedIssueAttachment {
            attachment_id,
            file: File {
                id: Uuid::new_v4(),
                file_path: file_path.to_string(),
                original_name: original_name.to_string(),
                mime_type: mime_type.map(str::to_string),
                size_bytes: 123,
                hash: "hash".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        }
    }

    async fn assert_workspace_state_deleted(pool: &SqlitePool, workspace_id: Uuid) {
        let workspace = Workspace::find_by_id(pool, workspace_id).await.unwrap();
        assert!(workspace.is_none());

        let sources = WorkspaceSource::find_by_workspace_id(pool, workspace_id)
            .await
            .unwrap();
        assert!(sources.is_empty());

        let repos = WorkspaceRepo::find_repos_with_target_branch_for_workspace(pool, workspace_id)
            .await
            .unwrap();
        assert!(repos.is_empty());

        let claims = WorkspaceRepoClaim::find_by_workspace_id(pool, workspace_id)
            .await
            .unwrap();
        assert!(claims.is_empty());
    }

    #[test]
    fn cleanup_failed_create_and_start_workspace_removes_in_place_git_claims() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();
            let workspace = Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("claim-cleanup-{workspace_id}"),
                    workspace_mode: WorkspaceMode::InPlaceGit,
                    name: Some("Claim cleanup workspace".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();
            let repo_path = std::env::temp_dir().join(format!(
                "create-route-claim-cleanup-repo-{}",
                Uuid::new_v4()
            ));
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Claim cleanup repo")
                .await
                .unwrap();

            WorkspaceRepoClaim::create_many(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap();

            super::cleanup_failed_create_and_start_workspace(&pool, workspace.id).await;

            assert_workspace_state_deleted(&pool, workspace.id).await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_failed_create_and_start_workspace_uses_in_place_git_root_cleanup() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();
            let workspace = Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("claim-cleanup-root-{workspace_id}"),
                    workspace_mode: WorkspaceMode::InPlaceGit,
                    name: Some("Claim cleanup root workspace".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            let real_repo_dir =
                std::env::temp_dir().join(format!("create-route-real-repo-{}", Uuid::new_v4()));
            fs::create_dir_all(&real_repo_dir).unwrap();
            fs::write(real_repo_dir.join("README.md"), "repo\n").unwrap();

            let workspace_root = workspace_manager::WorkspaceManager::get_workspace_base_dir()
                .join(format!("create-route-cleanup-root-{}", Uuid::new_v4()));
            fs::create_dir_all(&workspace_root).unwrap();
            std::os::unix::fs::symlink(&real_repo_dir, workspace_root.join("repo")).unwrap();

            Workspace::update_container_ref(&pool, workspace.id, &workspace_root.to_string_lossy())
                .await
                .unwrap();

            let repo = Repo::find_or_create(&pool, Path::new(&real_repo_dir), "Cleanup root repo")
                .await
                .unwrap();
            WorkspaceRepoClaim::create_many(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap();

            super::cleanup_failed_create_and_start_workspace(&pool, workspace.id).await;
            assert_workspace_state_deleted(&pool, workspace.id).await;

            for _ in 0..20 {
                if !workspace_root.exists() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }

            assert!(!workspace_root.exists());
            assert!(real_repo_dir.exists());
            assert!(real_repo_dir.join("README.md").exists());

            let _ = tokio::fs::remove_dir_all(&real_repo_dir).await;
        });
    }

    #[test]
    fn linked_issue_prompt_includes_all_ids_before_original_prompt() {
        let prompt = "Implement the linked ticket";
        let organization_id = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();

        let injected = inject_linked_issue_prompt_context(
            prompt,
            Some(&linked_issue()),
            Some(organization_id),
        );

        assert_eq!(
            injected,
            "organization_id: aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\nproject_id: bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\nissue_id: cccccccc-cccc-cccc-cccc-cccccccccccc\n\nImplement the linked ticket"
        );
    }

    #[test]
    fn linked_issue_prompt_without_metadata_leaves_prompt_unchanged() {
        let prompt = "Implement the linked ticket";

        let injected = inject_linked_issue_prompt_context(prompt, None, None);

        assert_eq!(injected, prompt);
    }

    #[test]
    fn linked_issue_prompt_preserves_multiline_prompt_body_after_context_block() {
        let prompt = "Line one\n\nLine two";
        let organization_id = Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap();

        let injected = inject_linked_issue_prompt_context(
            prompt,
            Some(&linked_issue()),
            Some(organization_id),
        );

        assert!(injected.ends_with("\n\nLine one\n\nLine two"));
    }

    #[test]
    fn rewrites_imported_non_image_attachment_links() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("[proposal.pdf](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "[proposal.pdf](.vibe-attachments/abc_proposal.pdf)"
        );
    }

    #[test]
    fn preserves_authored_image_markdown_for_imported_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("![diagram.png](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "diagram.png",
            "xyz_diagram.png",
            Some("image/png"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "![diagram.png](.vibe-attachments/xyz_diagram.png)"
        );
    }

    #[test]
    fn preserves_authored_link_markdown_for_imported_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("[diagram.png](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "diagram.png",
            "xyz_diagram.png",
            Some("image/png"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "[diagram.png](.vibe-attachments/xyz_diagram.png)"
        );
    }

    #[test]
    fn preserves_authored_image_markdown_for_imported_non_images() {
        let attachment_id = Uuid::new_v4();
        let prompt = format!("![proposal.pdf](attachment://{})", attachment_id);
        let imported = vec![imported_file(
            attachment_id,
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "![proposal.pdf](.vibe-attachments/abc_proposal.pdf)"
        );
    }

    #[test]
    fn leaves_unknown_attachment_references_unchanged() {
        let prompt = format!("[proposal.pdf](attachment://{})", Uuid::new_v4());
        let imported = vec![imported_file(
            Uuid::new_v4(),
            "proposal.pdf",
            "abc_proposal.pdf",
            Some("application/pdf"),
        )];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(rewritten, prompt);
    }

    #[test]
    fn rewrites_multiple_attachments_and_leaves_other_links_alone() {
        let image_attachment_id = Uuid::new_v4();
        let file_attachment_id = Uuid::new_v4();
        let prompt = format!(
            "See [doc.pdf](attachment://{}) and ![shot.png](attachment://{}). https://example.com",
            file_attachment_id, image_attachment_id
        );
        let imported = vec![
            imported_file(
                file_attachment_id,
                "doc.pdf",
                "doc_file.pdf",
                Some("application/pdf"),
            ),
            imported_file(
                image_attachment_id,
                "shot.png",
                "shot_file.png",
                Some("image/png"),
            ),
        ];

        let rewritten = rewrite_imported_issue_attachments_markdown(&prompt, &imported);

        assert_eq!(
            rewritten,
            "See [doc.pdf](.vibe-attachments/doc_file.pdf) and ![shot.png](.vibe-attachments/shot_file.png). https://example.com"
        );
    }

    #[test]
    fn normalize_workspace_request_defaults_new_git_repo_request_shape_to_git_worktree() {
        let repo_id = Uuid::new_v4();
        let request: CreateAndStartWorkspaceRequest = serde_json::from_value(json!({
            "sources": [
                {
                    "type": "git_repo",
                    "repo_id": repo_id,
                    "target_branch": "main"
                }
            ],
            "executor_config": {
                "executor": "CLAUDE_CODE"
            },
            "prompt": "Create a workspace"
        }))
        .unwrap();

        let normalized = normalize_workspace_request(request).unwrap();

        assert_eq!(normalized.workspace_mode, WorkspaceMode::GitWorktree);
        assert_eq!(
            normalized.sources,
            vec![WorkspaceSourceInput::GitRepo {
                repo_id,
                target_branch: "main".to_string(),
            }]
        );
        assert_eq!(
            normalized.legacy_git_repos,
            vec![WorkspaceRepoInput {
                repo_id,
                target_branch: "main".to_string(),
            }]
        );
    }

    #[test]
    fn normalize_workspace_request_rejects_mixed_sources_and_legacy_repos() {
        let repo_id = Uuid::new_v4();
        let request: CreateAndStartWorkspaceRequest = serde_json::from_value(json!({
            "sources": [
                {
                    "type": "git_repo",
                    "repo_id": repo_id,
                    "target_branch": "main"
                }
            ],
            "repos": [
                {
                    "repo_id": repo_id,
                    "target_branch": "main"
                }
            ],
            "executor_config": {
                "executor": "CLAUDE_CODE"
            },
            "prompt": "Create a workspace"
        }))
        .unwrap();

        let err = normalize_workspace_request(request).unwrap_err();

        assert!(matches!(
            err,
            ApiError::BadRequest(message)
                if message.contains("sources") && message.contains("repos")
        ));
    }

    #[test]
    fn normalize_workspace_sources_rejects_directory_mode_with_git_repo_source() {
        let err = normalize_workspace_sources(
            WorkspaceMode::InPlaceDirectory,
            vec![WorkspaceSourceInput::GitRepo {
                repo_id: Uuid::new_v4(),
                target_branch: "main".to_string(),
            }],
            vec![],
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ApiError::BadRequest(message)
                if message.contains("in_place_directory") && message.contains("git_repo")
        ));
    }

    #[test]
    fn normalize_workspace_sources_rejects_in_place_git_with_directory_source() {
        let err = normalize_workspace_sources(
            WorkspaceMode::InPlaceGit,
            vec![WorkspaceSourceInput::Directory {
                path: "/tmp/workspace".to_string(),
                display_name: Some("workspace".to_string()),
            }],
            vec![],
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ApiError::BadRequest(message)
                if message.contains("in_place_git") && message.contains("git_repo")
        ));
    }

    #[test]
    fn normalize_workspace_request_accepts_directory_sources_for_in_place_directory() {
        let request: CreateAndStartWorkspaceRequest = serde_json::from_value(json!({
            "workspace_mode": "in_place_directory",
            "sources": [
                {
                    "type": "directory",
                    "path": "/tmp/workspace",
                    "display_name": "workspace"
                }
            ],
            "executor_config": {
                "executor": "CLAUDE_CODE"
            },
            "prompt": "Create a workspace"
        }))
        .unwrap();

        let normalized = normalize_workspace_request(request).unwrap();

        assert_eq!(normalized.workspace_mode, WorkspaceMode::InPlaceDirectory);
        assert_eq!(
            normalized.sources,
            vec![WorkspaceSourceInput::Directory {
                path: "/tmp/workspace".to_string(),
                display_name: Some("workspace".to_string()),
            }]
        );
        assert!(normalized.legacy_git_repos.is_empty());
    }

    #[test]
    fn normalize_workspace_request_rejects_directory_sources_for_default_git_worktree() {
        let request: CreateAndStartWorkspaceRequest = serde_json::from_value(json!({
            "sources": [
                {
                    "type": "directory",
                    "path": "/tmp/workspace",
                    "display_name": "workspace"
                }
            ],
            "executor_config": {
                "executor": "CLAUDE_CODE"
            },
            "prompt": "Create a workspace"
        }))
        .unwrap();

        let err = normalize_workspace_request(request).unwrap_err();

        assert!(matches!(
            err,
            ApiError::BadRequest(message)
                if message.contains("git_worktree") && message.contains("git_repo")
        ));
    }

    #[test]
    fn normalize_workspace_sources_synthesizes_git_worktree_sources_from_legacy_repos() {
        let repo_id = Uuid::new_v4();

        let normalized = normalize_workspace_sources(
            WorkspaceMode::GitWorktree,
            vec![],
            vec![WorkspaceRepoInput {
                repo_id,
                target_branch: "main".to_string(),
            }],
        )
        .unwrap();

        assert_eq!(normalized.workspace_mode, WorkspaceMode::GitWorktree);
        assert_eq!(
            normalized.sources,
            vec![WorkspaceSourceInput::GitRepo {
                repo_id,
                target_branch: "main".to_string(),
            }]
        );
        assert_eq!(
            normalized.legacy_git_repos,
            vec![WorkspaceRepoInput {
                repo_id,
                target_branch: "main".to_string(),
            }]
        );
    }

    #[test]
    fn normalize_workspace_sources_rejects_legacy_repos_with_incompatible_mode() {
        let repo_id = Uuid::new_v4();

        let err = normalize_workspace_sources(
            WorkspaceMode::InPlaceGit,
            vec![],
            vec![WorkspaceRepoInput {
                repo_id,
                target_branch: "main".to_string(),
            }],
        )
        .unwrap_err();

        assert!(matches!(
            err,
            ApiError::BadRequest(message)
                if message.contains("repos") && message.contains("git_worktree")
        ));
    }

    #[test]
    fn persist_workspace_sources_creates_canonical_git_repo_rows() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{workspace_id}"),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    name: Some("Persist workspace sources test".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            let repo_path =
                std::env::temp_dir().join(format!("create-route-repo-{}", Uuid::new_v4()));
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Create route repo")
                .await
                .unwrap();

            let persisted = persist_workspace_sources(
                &pool,
                workspace_id,
                &[WorkspaceSourceInput::GitRepo {
                    repo_id: repo.id,
                    target_branch: "main".to_string(),
                }],
            )
            .await
            .unwrap();

            assert_eq!(persisted.len(), 1);
            assert_eq!(persisted[0].workspace_id, workspace_id);
            assert_eq!(persisted[0].source_type, WorkspaceSourceKind::GitRepo);
            assert_eq!(persisted[0].repo_id, Some(repo.id));
            assert_eq!(persisted[0].target_branch.as_deref(), Some("main"));
            assert_eq!(persisted[0].position, 0);

            let fetched = WorkspaceSource::find_by_workspace_id(&pool, workspace_id)
                .await
                .unwrap();
            assert_eq!(fetched, persisted);
        });
    }

    #[test]
    fn prepare_create_and_start_workspace_keeps_in_place_git_sources_canonical_without_attaching_legacy_repos()
     {
        run_async_test(async {
            let pool = test_pool().await;
            let db = DBService { pool: pool.clone() };
            let workspace_manager = WorkspaceManager::new(db.clone());
            let git = GitService::new();
            let repo_path =
                std::env::temp_dir().join(format!("prepare-in-place-git-repo-{}", Uuid::new_v4()));
            git.initialize_repo_with_main_branch(&repo_path).unwrap();
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "In-place git repo")
                .await
                .unwrap();
            let prepare_pool = pool.clone();

            let prepared = prepare_create_and_start_workspace(
                &pool,
                &workspace_manager,
                &git,
                CreateAndStartWorkspaceRequest {
                    name: Some("In-place git workspace".to_string()),
                    workspace_mode: WorkspaceMode::InPlaceGit,
                    sources: vec![WorkspaceSourceInput::GitRepo {
                        repo_id: repo.id,
                        target_branch: "main".to_string(),
                    }],
                    repos: vec![],
                    linked_issue: None,
                    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                    prompt: "Create the workspace".to_string(),
                    attachment_ids: None,
                },
                move |name, workspace_mode| {
                    let prepare_pool = prepare_pool.clone();

                    async move {
                        let workspace_id = Uuid::new_v4();
                        Workspace::create(
                            &prepare_pool,
                            &CreateWorkspace {
                                branch: format!("test-branch-{workspace_id}"),
                                workspace_mode,
                                name,
                            },
                            workspace_id,
                        )
                        .await
                        .map_err(ApiError::from)
                    }
                },
            )
            .await
            .unwrap();

            assert_eq!(
                prepared.managed_workspace.workspace.workspace_mode,
                WorkspaceMode::InPlaceGit
            );
            assert!(prepared.managed_workspace.repos.is_empty());
            assert_eq!(prepared.persisted_sources.len(), 1);
            assert_eq!(prepared.persisted_sources[0].repo_id, Some(repo.id));
            assert_eq!(
                prepared.persisted_sources[0].source_type,
                WorkspaceSourceKind::GitRepo
            );

            let attached_repos = WorkspaceRepo::find_repos_with_target_branch_for_workspace(
                &pool,
                prepared.managed_workspace.workspace.id,
            )
            .await
            .unwrap();
            assert!(attached_repos.is_empty());
        });
    }

    #[test]
    fn prepare_create_and_start_workspace_accepts_in_place_directory_sources_without_legacy_repos()
    {
        run_async_test(async {
            let pool = test_pool().await;
            let db = DBService { pool: pool.clone() };
            let workspace_manager = WorkspaceManager::new(db.clone());
            let git = GitService::new();
            let prepare_pool = pool.clone();

            let prepared = prepare_create_and_start_workspace(
                &pool,
                &workspace_manager,
                &git,
                CreateAndStartWorkspaceRequest {
                    name: Some("Directory workspace".to_string()),
                    workspace_mode: WorkspaceMode::InPlaceDirectory,
                    sources: vec![WorkspaceSourceInput::Directory {
                        path: "/tmp/non-git-project".to_string(),
                        display_name: Some("non-git-project".to_string()),
                    }],
                    repos: vec![],
                    linked_issue: None,
                    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                    prompt: "Create the workspace".to_string(),
                    attachment_ids: None,
                },
                move |name, workspace_mode| {
                    let prepare_pool = prepare_pool.clone();

                    async move {
                        let workspace_id = Uuid::new_v4();
                        Workspace::create(
                            &prepare_pool,
                            &CreateWorkspace {
                                branch: format!("test-branch-{workspace_id}"),
                                workspace_mode,
                                name,
                            },
                            workspace_id,
                        )
                        .await
                        .map_err(ApiError::from)
                    }
                },
            )
            .await
            .unwrap();

            assert_eq!(
                prepared.managed_workspace.workspace.workspace_mode,
                WorkspaceMode::InPlaceDirectory
            );
            assert!(prepared.managed_workspace.repos.is_empty());
            assert_eq!(prepared.persisted_sources.len(), 1);
            assert_eq!(
                prepared.persisted_sources[0].source_type,
                WorkspaceSourceKind::Directory
            );
            assert_eq!(
                prepared.persisted_sources[0].path.as_deref(),
                Some("/tmp/non-git-project")
            );

            let attached_repos = WorkspaceRepo::find_repos_with_target_branch_for_workspace(
                &pool,
                prepared.managed_workspace.workspace.id,
            )
            .await
            .unwrap();
            assert!(attached_repos.is_empty());
        });
    }

    #[test]
    fn create_and_start_workspace_core_returns_persisted_sources_and_attached_repo_for_git_input() {
        run_async_test(async {
            let pool = test_pool().await;
            let db = DBService { pool: pool.clone() };
            let workspace_manager = WorkspaceManager::new(db.clone());
            let git = GitService::new();
            let repo_path =
                std::env::temp_dir().join(format!("create-start-route-repo-{}", Uuid::new_v4()));
            git.initialize_repo_with_main_branch(&repo_path).unwrap();
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Route create repo")
                .await
                .unwrap();
            let prepare_pool = pool.clone();
            let start_pool = pool.clone();

            let prepared = prepare_create_and_start_workspace(
                &pool,
                &workspace_manager,
                &git,
                CreateAndStartWorkspaceRequest {
                    name: Some("Route-level workspace".to_string()),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    sources: vec![WorkspaceSourceInput::GitRepo {
                        repo_id: repo.id,
                        target_branch: "main".to_string(),
                    }],
                    repos: vec![],
                    linked_issue: None,
                    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                    prompt: "Create the workspace".to_string(),
                    attachment_ids: None,
                },
                move |name, workspace_mode| {
                    let prepare_pool = prepare_pool.clone();

                    async move {
                        let workspace_id = Uuid::new_v4();
                        let workspace = Workspace::create(
                            &prepare_pool,
                            &CreateWorkspace {
                                branch: format!("test-branch-{workspace_id}"),
                                workspace_mode,
                                name,
                            },
                            workspace_id,
                        )
                        .await?;

                        Ok(workspace)
                    }
                },
            )
            .await
            .unwrap();

            let attached_repos = WorkspaceRepo::find_repos_with_target_branch_for_workspace(
                &pool,
                prepared.managed_workspace.workspace.id,
            )
            .await
            .unwrap();

            assert_eq!(attached_repos.len(), 1);
            assert_eq!(attached_repos[0].repo.id, repo.id);
            assert_eq!(attached_repos[0].target_branch, "main");

            let response = start_prepared_workspace(
                prepared,
                None,
                move |workspace, _executor_config, _prompt| {
                    let start_pool = start_pool.clone();

                    async move {
                        let session = Session::create(
                            &start_pool,
                            &CreateSession {
                                executor: Some("CODEX".to_string()),
                                name: None,
                            },
                            Uuid::new_v4(),
                            workspace.id,
                        )
                        .await
                        .unwrap();

                        Ok(ExecutionProcess {
                            id: Uuid::new_v4(),
                            session_id: session.id,
                            run_reason: ExecutionProcessRunReason::CodingAgent,
                            executor_action: Json(ExecutorActionField::Other(
                                json!({ "type": "noop" }),
                            )),
                            status: ExecutionProcessStatus::Running,
                            exit_code: None,
                            dropped: false,
                            started_at: Utc::now(),
                            completed_at: None,
                            created_at: Utc::now(),
                            updated_at: Utc::now(),
                        })
                    }
                },
            )
            .await
            .unwrap();

            assert_eq!(response.sources.len(), 1);
            assert_eq!(response.sources[0].repo_id, Some(repo.id));
            assert_eq!(response.sources[0].target_branch.as_deref(), Some("main"));
            assert_eq!(
                response.sources[0].source_type,
                WorkspaceSourceKind::GitRepo
            );
        });
    }

    #[test]
    fn prepare_create_and_start_workspace_cleans_up_when_repo_attach_fails() {
        run_async_test(async {
            let pool = test_pool().await;
            let db = DBService { pool: pool.clone() };
            let workspace_manager = WorkspaceManager::new(db.clone());
            let git = GitService::new();
            let repo_path =
                std::env::temp_dir().join(format!("prepare-cleanup-route-repo-{}", Uuid::new_v4()));
            git.initialize_repo_with_main_branch(&repo_path).unwrap();
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Prepare cleanup repo")
                .await
                .unwrap();
            let created_workspace_id = Arc::new(Mutex::new(None));
            let created_workspace_id_for_creator = created_workspace_id.clone();
            let prepare_pool = pool.clone();

            let result = prepare_create_and_start_workspace(
                &pool,
                &workspace_manager,
                &git,
                CreateAndStartWorkspaceRequest {
                    name: Some("Cleanup workspace".to_string()),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    sources: vec![WorkspaceSourceInput::GitRepo {
                        repo_id: repo.id,
                        target_branch: "missing".to_string(),
                    }],
                    repos: vec![],
                    linked_issue: None,
                    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                    prompt: "Create the workspace".to_string(),
                    attachment_ids: None,
                },
                move |name, workspace_mode| {
                    let prepare_pool = prepare_pool.clone();
                    let created_workspace_id_for_creator = created_workspace_id_for_creator.clone();

                    async move {
                        let workspace_id = Uuid::new_v4();
                        *created_workspace_id_for_creator.lock().unwrap() = Some(workspace_id);
                        Workspace::create(
                            &prepare_pool,
                            &CreateWorkspace {
                                branch: format!("test-branch-{workspace_id}"),
                                workspace_mode,
                                name,
                            },
                            workspace_id,
                        )
                        .await
                        .map_err(ApiError::from)
                    }
                },
            )
            .await;

            let err = match result {
                Ok(_) => panic!("expected repo attach failure"),
                Err(err) => err,
            };

            assert!(matches!(err, ApiError::BadRequest(message) if message.contains("missing")));

            let workspace_id = created_workspace_id.lock().unwrap().unwrap();
            assert_workspace_state_deleted(&pool, workspace_id).await;
        });
    }

    #[test]
    fn start_prepared_workspace_cleans_up_when_start_fails() {
        run_async_test(async {
            let pool = test_pool().await;
            let db = DBService { pool: pool.clone() };
            let workspace_manager = WorkspaceManager::new(db.clone());
            let git = GitService::new();
            let repo_path =
                std::env::temp_dir().join(format!("start-cleanup-route-repo-{}", Uuid::new_v4()));
            git.initialize_repo_with_main_branch(&repo_path).unwrap();
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Start cleanup repo")
                .await
                .unwrap();
            let prepare_pool = pool.clone();

            let prepared = prepare_create_and_start_workspace(
                &pool,
                &workspace_manager,
                &git,
                CreateAndStartWorkspaceRequest {
                    name: Some("Cleanup start workspace".to_string()),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    sources: vec![WorkspaceSourceInput::GitRepo {
                        repo_id: repo.id,
                        target_branch: "main".to_string(),
                    }],
                    repos: vec![],
                    linked_issue: None,
                    executor_config: ExecutorConfig::new(BaseCodingAgent::Codex),
                    prompt: "Create the workspace".to_string(),
                    attachment_ids: None,
                },
                move |name, workspace_mode| {
                    let prepare_pool = prepare_pool.clone();

                    async move {
                        let workspace_id = Uuid::new_v4();
                        Workspace::create(
                            &prepare_pool,
                            &CreateWorkspace {
                                branch: format!("test-branch-{workspace_id}"),
                                workspace_mode,
                                name,
                            },
                            workspace_id,
                        )
                        .await
                        .map_err(ApiError::from)
                    }
                },
            )
            .await
            .unwrap();

            let workspace_id = prepared.managed_workspace.workspace.id;

            let err = start_prepared_workspace(
                prepared,
                None,
                |_workspace, _executor_config, _prompt| async {
                    Err(ApiError::Conflict("start failed".to_string()))
                },
            )
            .await
            .unwrap_err();

            assert!(matches!(err, ApiError::Conflict(message) if message.contains("start failed")));
            assert_workspace_state_deleted(&pool, workspace_id).await;
        });
    }
}
