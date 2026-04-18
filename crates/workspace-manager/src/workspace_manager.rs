use std::path::{Path, PathBuf};

use db::{
    DBService,
    models::{
        file::WorkspaceAttachment,
        repo::{Repo, RepoError},
        requests::WorkspaceRepoInput,
        session::Session,
        workspace::{Workspace as DbWorkspace, WorkspaceMode},
        workspace_repo::{CreateWorkspaceRepo, RepoWithTargetBranch, WorkspaceRepo},
    },
};
use git::{GitService, GitServiceError};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use worktree_manager::{WorktreeCleanup, WorktreeError, WorktreeManager};

#[derive(Debug, Clone)]
pub struct RepoWorkspaceInput {
    pub repo: Repo,
    pub target_branch: String,
}

impl RepoWorkspaceInput {
    pub fn new(repo: Repo, target_branch: String) -> Self {
        Self {
            repo,
            target_branch,
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Repo(#[from] RepoError),
    #[error(transparent)]
    Worktree(#[from] WorktreeError),
    #[error(transparent)]
    GitService(#[from] GitServiceError),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Workspace not found")]
    WorkspaceNotFound,
    #[error("Repository already attached to workspace")]
    RepoAlreadyAttached,
    #[error("Branch '{branch}' does not exist in repository '{repo_name}'")]
    BranchNotFound { repo_name: String, branch: String },
    #[error("No repositories provided")]
    NoRepositories,
    #[error("Partial workspace creation failed: {0}")]
    PartialCreation(String),
}

/// Info about a single repo's worktree within a workspace
#[derive(Debug, Clone)]
pub struct RepoWorktree {
    pub repo_id: Uuid,
    pub repo_name: String,
    pub source_repo_path: PathBuf,
    pub worktree_path: PathBuf,
}

/// A container directory holding worktrees for all project repos
#[derive(Debug, Clone)]
pub struct WorktreeContainer {
    pub workspace_dir: PathBuf,
    pub worktrees: Vec<RepoWorktree>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceDeletionContext {
    pub workspace_id: Uuid,
    pub branch_name: String,
    pub workspace_mode: WorkspaceMode,
    pub workspace_dir: Option<PathBuf>,
    pub repositories: Vec<Repo>,
    pub repo_paths: Vec<PathBuf>,
    pub session_ids: Vec<Uuid>,
}

#[derive(Clone)]
pub struct ManagedWorkspace {
    pub workspace: DbWorkspace,
    pub repos: Vec<RepoWithTargetBranch>,
    db: DBService,
}

impl ManagedWorkspace {
    fn new(db: DBService, workspace: DbWorkspace, repos: Vec<RepoWithTargetBranch>) -> Self {
        Self {
            workspace,
            repos,
            db,
        }
    }

    async fn attach_repository(&self, repo: &WorkspaceRepoInput) -> Result<(), sqlx::Error> {
        let create_repo = CreateWorkspaceRepo {
            repo_id: repo.repo_id,
            target_branch: repo.target_branch.clone(),
        };

        WorkspaceRepo::create_many(
            &self.db.pool,
            self.workspace.id,
            std::slice::from_ref(&create_repo),
        )
        .await
        .map(|_| ())
    }

    async fn refresh(&mut self) -> Result<(), WorkspaceError> {
        self.workspace = DbWorkspace::find_by_id(&self.db.pool, self.workspace.id)
            .await?
            .ok_or(WorkspaceError::WorkspaceNotFound)?;
        self.repos = WorkspaceRepo::find_repos_with_target_branch_for_workspace(
            &self.db.pool,
            self.workspace.id,
        )
        .await?;
        Ok(())
    }

    pub async fn add_repository(
        &mut self,
        repo_ref: &WorkspaceRepoInput,
        git: &GitService,
    ) -> Result<(), WorkspaceError> {
        let repo = Repo::find_by_id(&self.db.pool, repo_ref.repo_id)
            .await?
            .ok_or(RepoError::NotFound)?;

        if !git.check_branch_exists(&repo.path, &repo_ref.target_branch)? {
            return Err(WorkspaceError::BranchNotFound {
                repo_name: repo.name,
                branch: repo_ref.target_branch.clone(),
            });
        }

        if WorkspaceRepo::find_by_workspace_and_repo_id(
            &self.db.pool,
            self.workspace.id,
            repo_ref.repo_id,
        )
        .await?
        .is_some()
        {
            return Err(WorkspaceError::RepoAlreadyAttached);
        }

        self.attach_repository(repo_ref).await?;
        self.refresh().await?;
        Ok(())
    }

    pub async fn associate_attachments(&self, attachment_ids: &[Uuid]) -> Result<(), sqlx::Error> {
        if attachment_ids.is_empty() {
            return Ok(());
        }

        WorkspaceAttachment::associate_many_dedup(&self.db.pool, self.workspace.id, attachment_ids)
            .await
    }

    pub async fn prepare_deletion_context(&self) -> Result<WorkspaceDeletionContext, sqlx::Error> {
        let repositories =
            WorkspaceRepo::find_repos_for_workspace(&self.db.pool, self.workspace.id).await?;
        let session_ids = Session::find_by_workspace_id(&self.db.pool, self.workspace.id)
            .await?
            .into_iter()
            .map(|session| session.id)
            .collect::<Vec<_>>();
        let repo_paths = repositories
            .iter()
            .map(|repo| repo.path.clone())
            .collect::<Vec<_>>();

        Ok(WorkspaceDeletionContext {
            workspace_id: self.workspace.id,
            branch_name: self.workspace.branch.clone(),
            workspace_mode: self.workspace.workspace_mode,
            workspace_dir: self.workspace.container_ref.clone().map(PathBuf::from),
            repositories,
            repo_paths,
            session_ids,
        })
    }

    pub async fn delete_record(&self) -> Result<u64, sqlx::Error> {
        DbWorkspace::delete(&self.db.pool, self.workspace.id).await
    }
}

#[derive(Clone)]
pub struct WorkspaceManager {
    db: DBService,
}

impl WorkspaceManager {
    pub fn new(db: DBService) -> Self {
        Self { db }
    }

    pub async fn load_managed_workspace(
        &self,
        workspace: DbWorkspace,
    ) -> Result<ManagedWorkspace, sqlx::Error> {
        let repos =
            WorkspaceRepo::find_repos_with_target_branch_for_workspace(&self.db.pool, workspace.id)
                .await?;
        Ok(ManagedWorkspace::new(self.db.clone(), workspace, repos))
    }

    pub fn spawn_workspace_deletion_cleanup(
        context: WorkspaceDeletionContext,
        delete_branches: bool,
    ) {
        tokio::spawn(async move {
            Self::run_workspace_deletion_cleanup(context, delete_branches).await;
        });
    }

    async fn run_workspace_deletion_cleanup(
        context: WorkspaceDeletionContext,
        delete_branches: bool,
    ) {
        let WorkspaceDeletionContext {
            workspace_id,
            branch_name,
            workspace_mode,
            workspace_dir,
            repositories,
            repo_paths,
            session_ids,
        } = context;

        for session_id in session_ids {
            if let Err(e) = Self::remove_session_process_logs(session_id).await {
                warn!(
                    "Failed to remove filesystem process logs for session {}: {}",
                    session_id, e
                );
            }
        }

        if let Some(workspace_dir) = workspace_dir {
            info!(
                "Starting background cleanup for workspace {} at {}",
                workspace_id,
                workspace_dir.display()
            );

            let cleanup_result = match workspace_mode {
                WorkspaceMode::GitWorktree => {
                    Self::cleanup_workspace(&workspace_dir, &repositories).await
                }
                WorkspaceMode::InPlaceGit | WorkspaceMode::InPlaceDirectory => {
                    Self::cleanup_workspace_root_in_base_dir(&workspace_dir).await
                }
            };

            if let Err(e) = cleanup_result {
                error!(
                    "Background workspace cleanup failed for {} at {}: {}",
                    workspace_id,
                    workspace_dir.display(),
                    e
                );
            } else {
                info!(
                    "Background cleanup completed for workspace {}",
                    workspace_id
                );
            }
        }

        if delete_branches && workspace_mode == WorkspaceMode::GitWorktree {
            let git_service = GitService::new();
            for repo_path in repo_paths {
                match git_service.delete_branch(&repo_path, &branch_name) {
                    Ok(()) => {
                        info!("Deleted branch '{}' from repo {:?}", branch_name, repo_path);
                    }
                    Err(e) => {
                        warn!(
                            "Failed to delete branch '{}' from repo {:?}: {}",
                            branch_name, repo_path, e
                        );
                    }
                }
            }
        }
    }

    async fn remove_session_process_logs(session_id: Uuid) -> Result<(), std::io::Error> {
        let dir = utils::execution_logs::process_logs_session_dir(session_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Create a workspace with worktrees for all repositories.
    /// On failure, rolls back any already-created worktrees.
    pub async fn create_workspace(
        workspace_dir: &Path,
        repos: &[RepoWorkspaceInput],
        branch_name: &str,
    ) -> Result<WorktreeContainer, WorkspaceError> {
        if repos.is_empty() {
            return Err(WorkspaceError::NoRepositories);
        }

        info!(
            "Creating workspace at {} with {} repositories",
            workspace_dir.display(),
            repos.len()
        );

        tokio::fs::create_dir_all(workspace_dir).await?;

        let mut created_worktrees: Vec<RepoWorktree> = Vec::new();

        for input in repos {
            let worktree_path = workspace_dir.join(&input.repo.name);

            debug!(
                "Creating worktree for repo '{}' at {}",
                input.repo.name,
                worktree_path.display()
            );

            match WorktreeManager::create_worktree(
                &input.repo.path,
                branch_name,
                &worktree_path,
                &input.target_branch,
                true,
            )
            .await
            {
                Ok(()) => {
                    created_worktrees.push(RepoWorktree {
                        repo_id: input.repo.id,
                        repo_name: input.repo.name.clone(),
                        source_repo_path: input.repo.path.clone(),
                        worktree_path,
                    });
                }
                Err(e) => {
                    error!(
                        "Failed to create worktree for repo '{}': {}. Rolling back...",
                        input.repo.name, e
                    );

                    // Rollback: cleanup all worktrees we've created so far
                    Self::cleanup_created_worktrees(&created_worktrees).await;

                    // Also remove the workspace directory if it's empty
                    if let Err(cleanup_err) = tokio::fs::remove_dir(workspace_dir).await {
                        debug!(
                            "Could not remove workspace dir during rollback: {}",
                            cleanup_err
                        );
                    }

                    return Err(WorkspaceError::PartialCreation(format!(
                        "Failed to create worktree for repo '{}': {}",
                        input.repo.name, e
                    )));
                }
            }
        }

        info!(
            "Successfully created workspace with {} worktrees",
            created_worktrees.len()
        );

        Ok(WorktreeContainer {
            workspace_dir: workspace_dir.to_path_buf(),
            worktrees: created_worktrees,
        })
    }

    /// Ensure all worktrees in a workspace exist (for cold restart scenarios)
    pub async fn ensure_workspace_exists(
        workspace_dir: &Path,
        repos: &[RepoWorkspaceInput],
        branch_name: &str,
    ) -> Result<(), WorkspaceError> {
        if repos.is_empty() {
            return Err(WorkspaceError::NoRepositories);
        }

        // Try legacy migration first (single repo projects only)
        // Old layout had worktree directly at workspace_dir; new layout has it at workspace_dir/{repo_name}
        if repos.len() == 1 && Self::migrate_legacy_worktree(workspace_dir, &repos[0].repo).await? {
            return Ok(());
        }

        if !workspace_dir.exists() {
            tokio::fs::create_dir_all(workspace_dir).await?;
        }

        let git = GitService::new();

        for input in repos {
            let repo = &input.repo;
            let worktree_path = workspace_dir.join(&repo.name);

            debug!(
                "Ensuring worktree exists for repo '{}' at {}",
                repo.name,
                worktree_path.display()
            );

            if git.check_branch_exists(&repo.path, branch_name)? {
                WorktreeManager::ensure_worktree_exists(&repo.path, branch_name, &worktree_path)
                    .await?;
            } else {
                info!(
                    "Workspace branch '{}' missing in repo '{}'; creating from target branch '{}'",
                    branch_name, repo.name, input.target_branch
                );
                WorktreeManager::create_worktree(
                    &repo.path,
                    branch_name,
                    &worktree_path,
                    &input.target_branch,
                    true,
                )
                .await?;
            }
        }

        Ok(())
    }

    /// Clean up all worktrees in a workspace
    pub async fn cleanup_workspace(
        workspace_dir: &Path,
        repos: &[Repo],
    ) -> Result<(), WorkspaceError> {
        info!("Cleaning up workspace at {}", workspace_dir.display());

        let cleanup_data: Vec<WorktreeCleanup> = repos
            .iter()
            .map(|repo| {
                let worktree_path = workspace_dir.join(&repo.name);
                WorktreeCleanup::new(worktree_path, Some(repo.path.clone()))
            })
            .collect();

        WorktreeManager::batch_cleanup_worktrees(&cleanup_data).await?;

        // Remove the workspace directory itself
        if workspace_dir.exists()
            && let Err(e) = tokio::fs::remove_dir_all(workspace_dir).await
        {
            debug!(
                "Could not remove workspace directory {}: {}",
                workspace_dir.display(),
                e
            );
        }

        Ok(())
    }

    /// Get the base directory for workspaces (same as worktree base dir)
    pub fn get_workspace_base_dir() -> PathBuf {
        WorktreeManager::get_worktree_base_dir()
    }

    pub async fn cleanup_workspace_root_in_base_dir(
        workspace_dir: &Path,
    ) -> Result<(), WorkspaceError> {
        let workspace_base_dir = tokio::fs::canonicalize(Self::get_workspace_base_dir()).await?;
        let resolved_workspace_dir =
            Self::resolve_workspace_root_for_cleanup(workspace_dir).await?;

        if resolved_workspace_dir == workspace_base_dir
            || !resolved_workspace_dir.starts_with(&workspace_base_dir)
        {
            return Err(WorkspaceError::Io(std::io::Error::other(format!(
                "Refusing to clean up workspace outside the workspace base directory: {}",
                workspace_dir.display()
            ))));
        }

        if tokio::fs::try_exists(&resolved_workspace_dir).await? {
            tokio::fs::remove_dir_all(&resolved_workspace_dir).await?;
        }

        Ok(())
    }

    async fn resolve_workspace_root_for_cleanup(
        workspace_dir: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        if tokio::fs::try_exists(workspace_dir).await? {
            return Ok(tokio::fs::canonicalize(workspace_dir).await?);
        }

        Ok(normalize_path(workspace_dir))
    }

    /// Migrate a legacy single-worktree layout to the new workspace layout.
    /// Old layout: workspace_dir IS the worktree
    /// New layout: workspace_dir contains worktrees at workspace_dir/{repo_name}
    ///
    /// Returns Ok(true) if migration was performed, Ok(false) if no migration needed.
    async fn migrate_legacy_worktree(
        workspace_dir: &Path,
        repo: &Repo,
    ) -> Result<bool, WorkspaceError> {
        let expected_worktree_path = workspace_dir.join(&repo.name);

        // Detect old-style: workspace_dir exists AND has .git file (worktree marker)
        // AND expected new location doesn't exist
        let git_file = workspace_dir.join(".git");
        let is_old_style = workspace_dir.exists()
            && git_file.exists()
            && git_file.is_file() // .git file = worktree, .git dir = main repo
            && !expected_worktree_path.exists();

        if !is_old_style {
            return Ok(false);
        }

        info!(
            "Detected legacy worktree at {}, migrating to new layout",
            workspace_dir.display()
        );

        // Move old worktree to temp location (can't move into subdirectory of itself)
        let temp_name = format!(
            "{}-migrating",
            workspace_dir
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        );
        let temp_path = workspace_dir.with_file_name(temp_name);

        WorktreeManager::move_worktree(&repo.path, workspace_dir, &temp_path).await?;

        // Create new workspace directory
        tokio::fs::create_dir_all(workspace_dir).await?;

        // Move worktree to final location using git worktree move
        WorktreeManager::move_worktree(&repo.path, &temp_path, &expected_worktree_path).await?;

        if temp_path.exists() {
            let _ = tokio::fs::remove_dir_all(&temp_path).await;
        }

        info!(
            "Successfully migrated legacy worktree to {}",
            expected_worktree_path.display()
        );

        Ok(true)
    }

    /// Helper to cleanup worktrees during rollback
    async fn cleanup_created_worktrees(worktrees: &[RepoWorktree]) {
        for worktree in worktrees {
            let cleanup = WorktreeCleanup::new(
                worktree.worktree_path.clone(),
                Some(worktree.source_repo_path.clone()),
            );

            if let Err(e) = WorktreeManager::cleanup_worktree(&cleanup).await {
                error!(
                    "Failed to cleanup worktree '{}' during rollback: {}",
                    worktree.repo_name, e
                );
            }
        }
    }

    pub async fn cleanup_orphan_workspaces(&self) {
        if std::env::var("DISABLE_WORKTREE_CLEANUP").is_ok() {
            info!(
                "Orphan workspace cleanup is disabled via DISABLE_WORKTREE_CLEANUP environment variable"
            );
            return;
        }

        // Always clean up the default directory
        let default_dir = WorktreeManager::get_default_worktree_base_dir();
        self.cleanup_orphans_in_directory(&default_dir).await;

        // Also clean up custom directory if it's different from the default
        let current_dir = Self::get_workspace_base_dir();
        if current_dir != default_dir {
            self.cleanup_orphans_in_directory(&current_dir).await;
        }
    }

    async fn cleanup_orphans_in_directory(&self, workspace_base_dir: &Path) {
        if !workspace_base_dir.exists() {
            debug!(
                "Workspace base directory {} does not exist, skipping orphan cleanup",
                workspace_base_dir.display()
            );
            return;
        }

        let entries = match std::fs::read_dir(workspace_base_dir) {
            Ok(entries) => entries,
            Err(e) => {
                error!(
                    "Failed to read workspace base directory {}: {}",
                    workspace_base_dir.display(),
                    e
                );
                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let workspace_path_str = path.to_string_lossy().to_string();
            if let Ok(false) =
                DbWorkspace::container_ref_exists(&self.db.pool, &workspace_path_str).await
            {
                info!("Found orphaned workspace: {}", workspace_path_str);
                if let Err(e) = Self::cleanup_workspace_without_repos(&path).await {
                    error!(
                        "Failed to remove orphaned workspace {}: {}",
                        workspace_path_str, e
                    );
                } else {
                    info!(
                        "Successfully removed orphaned workspace: {}",
                        workspace_path_str
                    );
                }
            }
        }
    }

    async fn cleanup_workspace_without_repos(workspace_dir: &Path) -> Result<(), WorkspaceError> {
        info!(
            "Cleaning up orphaned workspace at {}",
            workspace_dir.display()
        );

        let entries = match std::fs::read_dir(workspace_dir) {
            Ok(entries) => entries,
            Err(e) => {
                debug!(
                    "Cannot read workspace directory {}, attempting direct removal: {}",
                    workspace_dir.display(),
                    e
                );
                return tokio::fs::remove_dir_all(workspace_dir)
                    .await
                    .map_err(WorkspaceError::Io);
            }
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir()
                && let Err(e) = WorktreeManager::cleanup_suspected_worktree(&path).await
            {
                warn!("Failed to cleanup suspected worktree: {}", e);
            }
        }

        if workspace_dir.exists()
            && let Err(e) = tokio::fs::remove_dir_all(workspace_dir).await
        {
            debug!(
                "Could not remove workspace directory {}: {}",
                workspace_dir.display(),
                e
            );
        }

        Ok(())
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }

    normalized
}

#[cfg(test)]
mod tests {
    use std::{path::Path, str::FromStr, sync::OnceLock};

    use db::{
        DBService,
        models::{
            repo::Repo,
            workspace::{CreateWorkspace, Workspace as DbWorkspace, WorkspaceMode},
            workspace_repo::{CreateWorkspaceRepo, WorkspaceRepo},
        },
    };
    use git::GitService;
    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use uuid::Uuid;
    use worktree_manager::WorktreeManager;

    use super::WorkspaceManager;

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "workspace-manager-delete-tests-{}.sqlite",
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

    async fn create_workspace(pool: &SqlitePool, label: &str, branch: &str) -> DbWorkspace {
        DbWorkspace::create(
            pool,
            &CreateWorkspace {
                branch: branch.to_string(),
                workspace_mode: WorkspaceMode::InPlaceGit,
                name: Some(format!("Workspace {label}")),
            },
            Uuid::new_v4(),
        )
        .await
        .unwrap()
    }

    async fn attach_repo(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repo_path: &Path,
        name: &str,
    ) -> Repo {
        let repo = Repo::find_or_create(pool, repo_path, name).await.unwrap();
        WorkspaceRepo::create_many(
            pool,
            workspace_id,
            &[CreateWorkspaceRepo {
                repo_id: repo.id,
                target_branch: "main".to_string(),
            }],
        )
        .await
        .unwrap();
        repo
    }

    fn test_workspace_base_dir() -> &'static Path {
        static TEST_WORKSPACE_BASE_DIR: OnceLock<std::path::PathBuf> = OnceLock::new();

        TEST_WORKSPACE_BASE_DIR.get_or_init(|| {
            let override_root = std::env::temp_dir().join(format!(
                "workspace-manager-test-base-dir-{}",
                Uuid::new_v4()
            ));
            std::fs::create_dir_all(&override_root).unwrap();
            WorktreeManager::set_workspace_dir_override(override_root);

            let base_dir = WorkspaceManager::get_workspace_base_dir();
            std::fs::create_dir_all(&base_dir).unwrap();
            base_dir
        })
    }

    async fn wait_for_workspace_cleanup(workspace_dir: &Path) {
        for _ in 0..40 {
            if !workspace_dir.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_workspace_deletion_cleanup_for_in_place_git_removes_only_synthetic_root() {
        let pool = test_pool().await;
        let git = GitService::new();
        let branch = format!("workspace-branch-{}", Uuid::new_v4());
        let workspace = create_workspace(&pool, "cleanup-root", &branch).await;
        let repo_path =
            std::env::temp_dir().join(format!("workspace-manager-real-repo-{}", Uuid::new_v4()));
        git.initialize_repo_with_main_branch(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "repo\n").unwrap();

        let repo = attach_repo(&pool, workspace.id, &repo_path, "repo").await;
        let workspace_dir = test_workspace_base_dir().join(format!(
            "workspace-manager-synthetic-root-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        std::os::unix::fs::symlink(&repo.path, workspace_dir.join(&repo.name)).unwrap();
        DbWorkspace::update_container_ref(&pool, workspace.id, &workspace_dir.to_string_lossy())
            .await
            .unwrap();

        let workspace = DbWorkspace::find_by_id(&pool, workspace.id)
            .await
            .unwrap()
            .unwrap();
        let manager = WorkspaceManager::new(DBService { pool: pool.clone() });
        let managed_workspace = manager.load_managed_workspace(workspace).await.unwrap();
        let deletion_context = managed_workspace.prepare_deletion_context().await.unwrap();

        WorkspaceManager::spawn_workspace_deletion_cleanup(deletion_context, false);
        wait_for_workspace_cleanup(&workspace_dir).await;

        assert!(!workspace_dir.exists());
        assert!(repo_path.exists());
        assert!(repo_path.join("README.md").exists());

        let _ = tokio::fs::remove_dir_all(&repo_path).await;
    }

    #[tokio::test]
    async fn spawn_workspace_deletion_cleanup_for_in_place_git_never_deletes_repo_outside_workspace_base_dir()
     {
        let _ = test_workspace_base_dir();
        let pool = test_pool().await;
        let git = GitService::new();
        let branch = format!("workspace-branch-{}", Uuid::new_v4());
        let workspace = create_workspace(&pool, "outside-base-guard", &branch).await;
        let repo_path = std::env::temp_dir().join(format!(
            "workspace-manager-outside-base-repo-{}",
            Uuid::new_v4()
        ));
        git.initialize_repo_with_main_branch(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "repo\n").unwrap();

        attach_repo(&pool, workspace.id, &repo_path, "repo").await;
        DbWorkspace::update_container_ref(&pool, workspace.id, &repo_path.to_string_lossy())
            .await
            .unwrap();

        let workspace = DbWorkspace::find_by_id(&pool, workspace.id)
            .await
            .unwrap()
            .unwrap();
        let manager = WorkspaceManager::new(DBService { pool: pool.clone() });
        let managed_workspace = manager.load_managed_workspace(workspace).await.unwrap();
        let deletion_context = managed_workspace.prepare_deletion_context().await.unwrap();

        WorkspaceManager::spawn_workspace_deletion_cleanup(deletion_context, false);
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(repo_path.exists());
        assert!(repo_path.join("README.md").exists());

        let _ = tokio::fs::remove_dir_all(&repo_path).await;
    }

    #[tokio::test]
    async fn spawn_workspace_deletion_cleanup_for_in_place_git_skips_branch_deletion() {
        let pool = test_pool().await;
        let git = GitService::new();
        let branch = format!("workspace-branch-{}", Uuid::new_v4());
        let workspace = create_workspace(&pool, "skip-branch-delete", &branch).await;
        let repo_path =
            std::env::temp_dir().join(format!("workspace-manager-branch-repo-{}", Uuid::new_v4()));
        git.initialize_repo_with_main_branch(&repo_path).unwrap();
        git.create_branch(&repo_path, &branch, "main").unwrap();

        attach_repo(&pool, workspace.id, &repo_path, "repo").await;
        let workspace_dir = test_workspace_base_dir().join(format!(
            "workspace-manager-delete-branch-root-{}",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&workspace_dir).unwrap();
        DbWorkspace::update_container_ref(&pool, workspace.id, &workspace_dir.to_string_lossy())
            .await
            .unwrap();

        let workspace = DbWorkspace::find_by_id(&pool, workspace.id)
            .await
            .unwrap()
            .unwrap();
        let manager = WorkspaceManager::new(DBService { pool: pool.clone() });
        let managed_workspace = manager.load_managed_workspace(workspace).await.unwrap();
        let deletion_context = managed_workspace.prepare_deletion_context().await.unwrap();

        WorkspaceManager::spawn_workspace_deletion_cleanup(deletion_context, true);
        wait_for_workspace_cleanup(&workspace_dir).await;

        assert!(git.check_branch_exists(&repo_path, &branch).unwrap());

        let _ = tokio::fs::remove_dir_all(&repo_path).await;
    }

    #[tokio::test]
    async fn cleanup_workspace_root_in_base_dir_removes_valid_workspace_root_under_base_dir() {
        let workspace_root = test_workspace_base_dir()
            .join(format!("workspace-manager-valid-root-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&workspace_root).unwrap();
        std::fs::write(workspace_root.join("README.md"), "workspace\n").unwrap();

        WorkspaceManager::cleanup_workspace_root_in_base_dir(&workspace_root)
            .await
            .unwrap();

        assert!(!workspace_root.exists());
    }

    #[tokio::test]
    async fn cleanup_workspace_root_in_base_dir_rejects_workspace_base_dir_itself() {
        let workspace_base_dir = test_workspace_base_dir();
        std::fs::write(workspace_base_dir.join("base-marker.txt"), "keep\n").unwrap();

        let err = WorkspaceManager::cleanup_workspace_root_in_base_dir(workspace_base_dir)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("workspace base directory"));
        assert!(workspace_base_dir.exists());
        assert!(workspace_base_dir.join("base-marker.txt").exists());
    }

    #[tokio::test]
    async fn cleanup_workspace_root_in_base_dir_rejects_normalized_outside_path() {
        let workspace_base_dir = test_workspace_base_dir();
        let outside_dir_name = format!("workspace-manager-outside-root-{}", Uuid::new_v4());
        let outside_dir = workspace_base_dir.parent().unwrap().join(&outside_dir_name);
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("outside.txt"), "keep\n").unwrap();

        let escaped_path = workspace_base_dir.join("..").join(&outside_dir_name);
        let err = WorkspaceManager::cleanup_workspace_root_in_base_dir(&escaped_path)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("workspace base directory"));
        assert!(outside_dir.exists());
        assert!(outside_dir.join("outside.txt").exists());

        let _ = tokio::fs::remove_dir_all(&outside_dir).await;
    }
}
