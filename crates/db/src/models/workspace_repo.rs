use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

use super::repo::Repo;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct WorkspaceRepo {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub repo_id: Uuid,
    pub target_branch: String,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct CreateWorkspaceRepo {
    pub repo_id: Uuid,
    pub target_branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct RepoWithTargetBranch {
    #[serde(flatten)]
    pub repo: Repo,
    pub target_branch: String,
}

/// Repo info with copy_files configuration.
#[derive(Debug, Clone)]
pub struct RepoWithCopyFiles {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: String,
    pub copy_files: Option<String>,
}

impl WorkspaceRepo {
    pub async fn create_many(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repos: &[CreateWorkspaceRepo],
    ) -> Result<Vec<Self>, sqlx::Error> {
        if repos.is_empty() {
            return Ok(Vec::new());
        }

        // Build bulk insert query with VALUES for each repo
        // SQLite doesn't have great support for bulk inserts with RETURNING,
        // so we'll use a transaction to batch the inserts efficiently
        let mut tx = pool.begin().await?;
        let mut results = Vec::with_capacity(repos.len());

        for repo in repos {
            let id = Uuid::new_v4();
            let workspace_repo = sqlx::query_as!(
                WorkspaceRepo,
                r#"INSERT INTO workspace_repos (id, workspace_id, repo_id, target_branch)
                   VALUES ($1, $2, $3, $4)
                   RETURNING id as "id!: Uuid",
                             workspace_id as "workspace_id!: Uuid",
                             repo_id as "repo_id!: Uuid",
                             target_branch,
                             created_at as "created_at!: DateTime<Utc>",
                             updated_at as "updated_at!: DateTime<Utc>""#,
                id,
                workspace_id,
                repo.repo_id,
                repo.target_branch
            )
            .fetch_one(&mut *tx)
            .await?;
            results.push(workspace_repo);
        }

        tx.commit().await?;
        Ok(results)
    }

    pub async fn find_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceRepo,
            r#"SELECT id as "id!: Uuid",
                      workspace_id as "workspace_id!: Uuid",
                      repo_id as "repo_id!: Uuid",
                      target_branch,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM workspace_repos
               WHERE workspace_id = $1"#,
            workspace_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_repos_for_workspace(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<Repo>, sqlx::Error> {
        sqlx::query_as!(
            Repo,
            r#"SELECT r.id as "id!: Uuid",
                      r.path,
                      r.name,
                      r.display_name,
                      r.setup_script,
                      r.cleanup_script,
                      r.archive_script,
                      r.copy_files,
                      r.parallel_setup_script as "parallel_setup_script!: bool",
                      r.dev_server_script,
                      r.default_target_branch,
                      r.default_working_dir,
                      r.created_at as "created_at!: DateTime<Utc>",
                      r.updated_at as "updated_at!: DateTime<Utc>"
               FROM repos r
               JOIN workspace_repos wr ON r.id = wr.repo_id
               WHERE wr.workspace_id = $1
               ORDER BY r.display_name ASC"#,
            workspace_id
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_repos_with_target_branch_for_workspace(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<RepoWithTargetBranch>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT r.id as "id!: Uuid",
                      r.path,
                      r.name,
                      r.display_name,
                      r.setup_script,
                      r.cleanup_script,
                      r.archive_script,
                      r.copy_files,
                      r.parallel_setup_script as "parallel_setup_script!: bool",
                      r.dev_server_script,
                      r.default_target_branch,
                      r.default_working_dir,
                      r.created_at as "created_at!: DateTime<Utc>",
                      r.updated_at as "updated_at!: DateTime<Utc>",
                      wr.target_branch
               FROM repos r
               JOIN workspace_repos wr ON r.id = wr.repo_id
               WHERE wr.workspace_id = $1
               ORDER BY r.display_name ASC"#,
            workspace_id
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| RepoWithTargetBranch {
                repo: Repo {
                    id: row.id,
                    path: PathBuf::from(row.path),
                    name: row.name,
                    display_name: row.display_name,
                    setup_script: row.setup_script,
                    cleanup_script: row.cleanup_script,
                    archive_script: row.archive_script,
                    copy_files: row.copy_files,
                    parallel_setup_script: row.parallel_setup_script,
                    dev_server_script: row.dev_server_script,
                    default_target_branch: row.default_target_branch,
                    default_working_dir: row.default_working_dir,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                },
                target_branch: row.target_branch,
            })
            .collect())
    }

    pub async fn find_by_workspace_and_repo_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repo_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceRepo,
            r#"SELECT id as "id!: Uuid",
                      workspace_id as "workspace_id!: Uuid",
                      repo_id as "repo_id!: Uuid",
                      target_branch,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM workspace_repos
               WHERE workspace_id = $1 AND repo_id = $2"#,
            workspace_id,
            repo_id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn update_target_branch(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repo_id: Uuid,
        new_target_branch: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            "UPDATE workspace_repos SET target_branch = $1, updated_at = datetime('now') WHERE workspace_id = $2 AND repo_id = $3",
            new_target_branch,
            workspace_id,
            repo_id
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn delete_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM workspace_repos WHERE workspace_id = $1",
            workspace_id
        )
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn sync_for_workspace(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repos: &[CreateWorkspaceRepo],
    ) -> Result<Vec<Self>, sqlx::Error> {
        let mut tx = pool.begin().await?;

        if repos.is_empty() {
            sqlx::query!(
                "DELETE FROM workspace_repos WHERE workspace_id = $1",
                workspace_id
            )
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;
            return Ok(Vec::new());
        }

        let repo_ids = repos.iter().map(|repo| repo.repo_id).collect::<Vec<_>>();

        let mut delete_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "DELETE FROM workspace_repos WHERE workspace_id = ",
        );
        delete_query.push_bind(workspace_id);
        delete_query.push(" AND repo_id NOT IN (");
        let mut separated = delete_query.separated(", ");
        for repo_id in &repo_ids {
            separated.push_bind(repo_id);
        }
        separated.push_unseparated(")");

        delete_query.build().execute(&mut *tx).await?;

        let mut results = Vec::with_capacity(repos.len());
        for repo in repos {
            let id = Uuid::new_v4();
            let workspace_repo = sqlx::query_as!(
                WorkspaceRepo,
                r#"INSERT INTO workspace_repos (id, workspace_id, repo_id, target_branch)
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT(workspace_id, repo_id)
                   DO UPDATE SET target_branch = excluded.target_branch,
                                 updated_at = datetime('now', 'subsec')
                   RETURNING id as "id!: Uuid",
                             workspace_id as "workspace_id!: Uuid",
                             repo_id as "repo_id!: Uuid",
                             target_branch,
                             created_at as "created_at!: DateTime<Utc>",
                             updated_at as "updated_at!: DateTime<Utc>""#,
                id,
                workspace_id,
                repo.repo_id,
                repo.target_branch
            )
            .fetch_one(&mut *tx)
            .await?;
            results.push(workspace_repo);
        }

        tx.commit().await?;
        Ok(results)
    }

    pub async fn update_target_branch_for_children_of_workspace(
        pool: &SqlitePool,
        parent_workspace_id: Uuid,
        old_branch: &str,
        new_branch: &str,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            r#"UPDATE workspace_repos
               SET target_branch = $1, updated_at = datetime('now')
               WHERE target_branch = $2
                 AND workspace_id IN (
                     SELECT w.id FROM workspaces w
                     JOIN tasks t ON w.task_id = t.id
                     WHERE t.parent_workspace_id = $3
                 )"#,
            new_branch,
            old_branch,
            parent_workspace_id
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Find repos for a workspace with their copy_files configuration.
    pub async fn find_repos_with_copy_files(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<RepoWithCopyFiles>, sqlx::Error> {
        let rows = sqlx::query!(
            r#"SELECT r.id as "id!: Uuid", r.path, r.name, r.copy_files
               FROM repos r
               JOIN workspace_repos wr ON r.id = wr.repo_id
               WHERE wr.workspace_id = $1"#,
            workspace_id
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| RepoWithCopyFiles {
                id: row.id,
                path: PathBuf::from(row.path),
                name: row.name,
                copy_files: row.copy_files,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, str::FromStr};

    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use tokio::runtime::Builder;

    use super::{CreateWorkspaceRepo, WorkspaceRepo};
    use crate::models::{
        repo::Repo,
        workspace::{CreateWorkspace, Workspace, WorkspaceMode},
    };

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "db-workspace-repo-tests-{}.sqlite",
            uuid::Uuid::new_v4()
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
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
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

    async fn create_workspace(pool: &SqlitePool, label: &str) -> Workspace {
        let workspace_id = uuid::Uuid::new_v4();
        Workspace::create(
            pool,
            &CreateWorkspace {
                branch: format!("branch-{label}-{workspace_id}"),
                workspace_mode: WorkspaceMode::InPlaceGit,
                name: Some(format!("Workspace {label}")),
            },
            workspace_id,
        )
        .await
        .unwrap()
    }

    async fn create_repo(pool: &SqlitePool, name: &str) -> Repo {
        let repo_path = std::env::temp_dir().join(format!(
            "workspace-repo-model-repo-{}-{}",
            name,
            uuid::Uuid::new_v4()
        ));
        Repo::find_or_create(pool, Path::new(&repo_path), name)
            .await
            .unwrap()
    }

    #[test]
    fn sync_for_workspace_replaces_existing_workspace_repo_rows() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace = create_workspace(&pool, "sync").await;
            let repo_one = create_repo(&pool, "repo-one").await;
            let repo_two = create_repo(&pool, "repo-two").await;

            WorkspaceRepo::create_many(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepo {
                    repo_id: repo_one.id,
                    target_branch: "main".to_string(),
                }],
            )
            .await
            .unwrap();

            let synced = WorkspaceRepo::sync_for_workspace(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepo {
                    repo_id: repo_two.id,
                    target_branch: "develop".to_string(),
                }],
            )
            .await
            .unwrap();

            assert_eq!(synced.len(), 1);
            assert_eq!(synced[0].repo_id, repo_two.id);
            assert_eq!(synced[0].target_branch, "develop");

            let fetched = WorkspaceRepo::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap();
            assert_eq!(fetched.len(), 1);
            assert_eq!(fetched[0].repo_id, repo_two.id);
            assert_eq!(fetched[0].target_branch, "develop");
        });
    }

    #[test]
    fn sync_for_workspace_upserts_existing_rows_without_churning_ids() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace = create_workspace(&pool, "upsert").await;
            let repo_one = create_repo(&pool, "repo-one").await;
            let repo_two = create_repo(&pool, "repo-two").await;

            let created = WorkspaceRepo::create_many(
                &pool,
                workspace.id,
                &[
                    CreateWorkspaceRepo {
                        repo_id: repo_one.id,
                        target_branch: "main".to_string(),
                    },
                    CreateWorkspaceRepo {
                        repo_id: repo_two.id,
                        target_branch: "develop".to_string(),
                    },
                ],
            )
            .await
            .unwrap();

            let synced = WorkspaceRepo::sync_for_workspace(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepo {
                    repo_id: repo_one.id,
                    target_branch: "release".to_string(),
                }],
            )
            .await
            .unwrap();

            assert_eq!(synced.len(), 1);
            assert_eq!(synced[0].id, created[0].id);
            assert_eq!(synced[0].repo_id, repo_one.id);
            assert_eq!(synced[0].target_branch, "release");

            let fetched = WorkspaceRepo::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap();
            assert_eq!(fetched.len(), 1);
            assert_eq!(fetched[0].id, created[0].id);
            assert_eq!(fetched[0].repo_id, repo_one.id);
            assert_eq!(fetched[0].target_branch, "release");
        });
    }

    #[test]
    fn delete_by_workspace_id_removes_all_rows_for_workspace() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace = create_workspace(&pool, "delete").await;
            let repo_one = create_repo(&pool, "repo-one").await;
            let repo_two = create_repo(&pool, "repo-two").await;

            WorkspaceRepo::create_many(
                &pool,
                workspace.id,
                &[
                    CreateWorkspaceRepo {
                        repo_id: repo_one.id,
                        target_branch: "main".to_string(),
                    },
                    CreateWorkspaceRepo {
                        repo_id: repo_two.id,
                        target_branch: "develop".to_string(),
                    },
                ],
            )
            .await
            .unwrap();

            let deleted = WorkspaceRepo::delete_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap();
            assert_eq!(deleted, 2);

            let fetched = WorkspaceRepo::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap();
            assert!(fetched.is_empty());
        });
    }
}
