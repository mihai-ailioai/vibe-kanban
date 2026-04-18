use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use strum_macros::{Display, EnumString};
use ts_rs::TS;
use uuid::Uuid;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS, EnumString, Display,
)]
#[sqlx(type_name = "workspace_source_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WorkspaceSourceKind {
    GitRepo,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum CreateWorkspaceSource {
    GitRepo {
        repo_id: Uuid,
        target_branch: String,
    },
    Directory {
        path: String,
        display_name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow, Serialize, Deserialize, TS)]
pub struct WorkspaceSource {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub source_type: WorkspaceSourceKind,
    pub repo_id: Option<Uuid>,
    pub path: Option<String>,
    pub display_name: Option<String>,
    pub target_branch: Option<String>,
    pub position: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn directory_workspace_entry_name(
    display_name: Option<&str>,
    source_path: &Path,
) -> Option<String> {
    display_name
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            source_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
}

pub fn validate_directory_workspace_entry_name(entry_name: &str) -> Result<(), &'static str> {
    let trimmed = entry_name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
    {
        return Err("must be a single path component");
    }

    Ok(())
}

impl WorkspaceSource {
    pub async fn create_many(
        pool: &SqlitePool,
        workspace_id: Uuid,
        sources: &[CreateWorkspaceSource],
    ) -> Result<Vec<Self>, sqlx::Error> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }

        let mut tx = pool.begin().await?;
        let mut results = Vec::with_capacity(sources.len());

        for (position, source) in sources.iter().enumerate() {
            let id = Uuid::new_v4();
            let position = position as i64;
            let (source_type, repo_id, path, display_name, target_branch) = match source {
                CreateWorkspaceSource::GitRepo {
                    repo_id,
                    target_branch,
                } => (
                    WorkspaceSourceKind::GitRepo,
                    Some(*repo_id),
                    None,
                    None,
                    Some(target_branch.clone()),
                ),
                CreateWorkspaceSource::Directory { path, display_name } => (
                    WorkspaceSourceKind::Directory,
                    None,
                    Some(path.clone()),
                    display_name.clone(),
                    None,
                ),
            };

            let workspace_source = sqlx::query_as!(
                WorkspaceSource,
                r#"INSERT INTO workspace_sources (
                       id,
                       workspace_id,
                       source_type,
                       repo_id,
                       path,
                       display_name,
                       target_branch,
                       position
                   )
                   VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                   RETURNING id as "id!: Uuid",
                             workspace_id as "workspace_id!: Uuid",
                             source_type as "source_type!: WorkspaceSourceKind",
                             repo_id as "repo_id: Uuid",
                             path,
                             display_name,
                             target_branch,
                             position,
                             created_at as "created_at!: DateTime<Utc>",
                             updated_at as "updated_at!: DateTime<Utc>""#,
                id,
                workspace_id,
                source_type,
                repo_id,
                path,
                display_name,
                target_branch,
                position,
            )
            .fetch_one(&mut *tx)
            .await?;

            results.push(workspace_source);
        }

        tx.commit().await?;
        Ok(results)
    }

    pub async fn find_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceSource,
            r#"SELECT id as "id!: Uuid",
                      workspace_id as "workspace_id!: Uuid",
                      source_type as "source_type!: WorkspaceSourceKind",
                      repo_id as "repo_id: Uuid",
                      path,
                      display_name,
                      target_branch,
                      position,
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM workspace_sources
               WHERE workspace_id = $1
               ORDER BY position ASC, id ASC"#,
            workspace_id,
        )
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, str::FromStr};

    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use tokio::runtime::Builder;
    use uuid::Uuid;

    use super::{CreateWorkspaceSource, WorkspaceSource, WorkspaceSourceKind};
    use crate::models::{
        repo::Repo,
        workspace::{CreateWorkspace, Workspace, WorkspaceMode},
    };

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "db-workspace-source-tests-{}.sqlite",
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

    #[test]
    fn create_many_and_find_by_workspace_id_round_trip_sources() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{}", workspace_id),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    name: Some("Workspace sources test".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            let repo_path =
                std::env::temp_dir().join(format!("workspace-source-repo-{}", Uuid::new_v4()));
            let repo = Repo::find_or_create(&pool, Path::new(&repo_path), "Workspace source repo")
                .await
                .unwrap();

            let created = WorkspaceSource::create_many(
                &pool,
                workspace_id,
                &[
                    CreateWorkspaceSource::GitRepo {
                        repo_id: repo.id,
                        target_branch: "main".to_string(),
                    },
                    CreateWorkspaceSource::Directory {
                        path: format!("/tmp/source-{}", Uuid::new_v4()),
                        display_name: Some("Design docs".to_string()),
                    },
                ],
            )
            .await
            .unwrap();

            assert_eq!(created.len(), 2);
            assert_eq!(created[0].position, 0);
            assert_eq!(created[0].source_type, WorkspaceSourceKind::GitRepo);
            assert_eq!(created[0].repo_id, Some(repo.id));
            assert_eq!(created[0].target_branch.as_deref(), Some("main"));
            assert_eq!(created[1].position, 1);
            assert_eq!(created[1].source_type, WorkspaceSourceKind::Directory);
            assert!(created[1].repo_id.is_none());
            assert_eq!(created[1].display_name.as_deref(), Some("Design docs"));

            let fetched = WorkspaceSource::find_by_workspace_id(&pool, workspace_id)
                .await
                .unwrap();

            assert_eq!(fetched, created);
        });
    }

    #[test]
    fn find_by_workspace_id_uses_persisted_position_order() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{}", workspace_id),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    name: Some("Workspace source order test".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            let directory_id = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
            let git_id = Uuid::parse_str("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap();
            let repo_path = std::env::temp_dir()
                .join(format!("workspace-source-order-repo-{}", Uuid::new_v4()));
            let repo =
                Repo::find_or_create(&pool, Path::new(&repo_path), "Workspace source order repo")
                    .await
                    .unwrap();

            sqlx::query(
                r#"INSERT INTO workspace_sources (
                       id, workspace_id, source_type, repo_id, path, display_name, target_branch, position, created_at, updated_at
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(directory_id)
            .bind(workspace_id)
            .bind("directory")
            .bind(Option::<Uuid>::None)
            .bind("/tmp/ordered-directory")
            .bind(Some("Ordered directory".to_string()))
            .bind(Option::<String>::None)
            .bind(1_i64)
            .bind("2026-04-17 00:00:00.000")
            .bind("2026-04-17 00:00:00.000")
            .execute(&pool)
            .await
            .unwrap();

            sqlx::query(
                r#"INSERT INTO workspace_sources (
                       id, workspace_id, source_type, repo_id, path, display_name, target_branch, position, created_at, updated_at
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
            )
            .bind(git_id)
            .bind(workspace_id)
            .bind("git_repo")
            .bind(Some(repo.id))
            .bind(Option::<String>::None)
            .bind(Option::<String>::None)
            .bind(Some("main".to_string()))
            .bind(0_i64)
            .bind("2026-04-17 00:00:00.000")
            .bind("2026-04-17 00:00:00.000")
            .execute(&pool)
            .await
            .unwrap();

            let fetched = WorkspaceSource::find_by_workspace_id(&pool, workspace_id)
                .await
                .unwrap();

            assert_eq!(fetched.len(), 2);
            assert_eq!(fetched[0].id, git_id);
            assert_eq!(fetched[0].position, 0);
            assert_eq!(fetched[1].id, directory_id);
            assert_eq!(fetched[1].position, 1);
        });
    }

    #[test]
    fn create_many_rejects_duplicate_git_sources_for_same_workspace_even_with_different_branches() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{}", workspace_id),
                    workspace_mode: WorkspaceMode::GitWorktree,
                    name: Some("Workspace duplicate source test".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            let repo_path = std::env::temp_dir().join(format!(
                "workspace-source-duplicate-repo-{}",
                Uuid::new_v4()
            ));
            let repo = Repo::find_or_create(
                &pool,
                Path::new(&repo_path),
                "Workspace duplicate source repo",
            )
            .await
            .unwrap();

            let error = WorkspaceSource::create_many(
                &pool,
                workspace_id,
                &[
                    CreateWorkspaceSource::GitRepo {
                        repo_id: repo.id,
                        target_branch: "main".to_string(),
                    },
                    CreateWorkspaceSource::GitRepo {
                        repo_id: repo.id,
                        target_branch: "release".to_string(),
                    },
                ],
            )
            .await
            .unwrap_err();

            assert!(
                error.to_string().contains("UNIQUE constraint failed"),
                "unexpected error: {error}"
            );
        });
    }
}
