use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

use super::{
    workspace::{Workspace, WorkspaceMode},
    workspace_repo::WorkspaceRepo,
    workspace_source::{WorkspaceSource, directory_workspace_entry_name},
};

#[derive(Debug, Error)]
pub enum SessionError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("Session not found")]
    NotFound,
    #[error("Workspace not found")]
    WorkspaceNotFound,
    #[error("Executor mismatch: session uses {expected} but request specified {actual}")]
    ExecutorMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct Session {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: Option<String>,
    pub executor: Option<String>,
    pub agent_working_dir: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateSession {
    pub executor: Option<String>,
    pub name: Option<String>,
}

impl Session {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            Session,
            r#"SELECT id AS "id!: Uuid",
                      workspace_id AS "workspace_id!: Uuid",
                      name,
                      executor,
                      agent_working_dir,
                      created_at AS "created_at!: DateTime<Utc>",
                      updated_at AS "updated_at!: DateTime<Utc>"
               FROM sessions
               WHERE id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    /// Find all sessions for a workspace, ordered by most recently used.
    /// "Most recently used" is defined as the most recent non-dev server execution process.
    /// Sessions with no executions fall back to created_at for ordering.
    pub async fn find_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            Session,
            r#"SELECT s.id AS "id!: Uuid",
                      s.workspace_id AS "workspace_id!: Uuid",
                      s.name,
                      s.executor,
                      s.agent_working_dir,
                      s.created_at AS "created_at!: DateTime<Utc>",
                      s.updated_at AS "updated_at!: DateTime<Utc>"
               FROM sessions s
               LEFT JOIN (
                   SELECT ep.session_id, MAX(ep.created_at) as last_used
                   FROM execution_processes ep
                   WHERE ep.run_reason != 'devserver' AND ep.dropped = FALSE
                   GROUP BY ep.session_id
               ) latest_ep ON s.id = latest_ep.session_id
               WHERE s.workspace_id = $1
               ORDER BY COALESCE(latest_ep.last_used, s.created_at) DESC"#,
            workspace_id
        )
        .fetch_all(pool)
        .await
    }

    /// Find the most recently used session for a workspace.
    /// "Most recently used" is defined as the most recent non-dev server execution process.
    /// Sessions with no executions fall back to created_at for ordering.
    pub async fn find_latest_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as!(
            Session,
            r#"SELECT s.id AS "id!: Uuid",
                      s.workspace_id AS "workspace_id!: Uuid",
                      s.name,
                      s.executor,
                      s.agent_working_dir,
                      s.created_at AS "created_at!: DateTime<Utc>",
                      s.updated_at AS "updated_at!: DateTime<Utc>"
               FROM sessions s
               LEFT JOIN (
                   SELECT ep.session_id, MAX(ep.created_at) as last_used
                   FROM execution_processes ep
                   WHERE ep.run_reason != 'devserver' AND ep.dropped = FALSE
                   GROUP BY ep.session_id
               ) latest_ep ON s.id = latest_ep.session_id
               WHERE s.workspace_id = $1
               ORDER BY COALESCE(latest_ep.last_used, s.created_at) DESC
               LIMIT 1"#,
            workspace_id
        )
        .fetch_optional(pool)
        .await
    }

    /// Find the first-created session for a workspace.
    /// This is a temporary policy for orchestrator MCP session discovery.
    pub async fn find_first_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Session>(
            r#"SELECT id,
                      workspace_id,
                      name,
                      executor,
                      agent_working_dir,
                      created_at,
                      updated_at
               FROM sessions
               WHERE workspace_id = ?
               ORDER BY created_at ASC, id ASC
               LIMIT 1"#,
        )
        .bind(workspace_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateSession,
        id: Uuid,
        workspace_id: Uuid,
    ) -> Result<Self, SessionError> {
        let agent_working_dir = Self::resolve_agent_working_dir(pool, workspace_id).await?;
        let name = data.name.as_deref().filter(|s| !s.is_empty());

        Ok(sqlx::query_as!(
            Session,
            r#"INSERT INTO sessions (id, workspace_id, name, executor, agent_working_dir)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING id AS "id!: Uuid",
                         workspace_id AS "workspace_id!: Uuid",
                         name,
                         executor,
                         agent_working_dir,
                         created_at AS "created_at!: DateTime<Utc>",
                         updated_at AS "updated_at!: DateTime<Utc>""#,
            id,
            workspace_id,
            name,
            data.executor,
            agent_working_dir
        )
        .fetch_one(pool)
        .await?)
    }

    async fn resolve_agent_working_dir(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Option<String>, sqlx::Error> {
        let repos = WorkspaceRepo::find_repos_for_workspace(pool, workspace_id).await?;
        if repos.len() == 1 {
            let repo = &repos[0];
            let path = match repo.default_working_dir.as_deref() {
                Some(subdir) if !subdir.is_empty() => {
                    std::path::PathBuf::from(&repo.name).join(subdir)
                }
                _ => std::path::PathBuf::from(&repo.name),
            };

            return Ok(Some(path.to_string_lossy().to_string()));
        }

        let Some(workspace) = Workspace::find_by_id(pool, workspace_id).await? else {
            return Ok(None);
        };
        if workspace.workspace_mode != WorkspaceMode::InPlaceDirectory {
            return Ok(None);
        }

        let sources = WorkspaceSource::find_by_workspace_id(pool, workspace_id).await?;
        let [source] = sources.as_slice() else {
            return Ok(None);
        };

        Ok(directory_source_entry_name(source))
    }

    pub async fn update(
        pool: &SqlitePool,
        id: Uuid,
        name: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let name_value = name.filter(|s| !s.is_empty());
        let name_provided = name.is_some();

        sqlx::query!(
            r#"UPDATE sessions SET
                name = CASE WHEN $1 THEN $2 ELSE name END,
                updated_at = datetime('now', 'subsec')
            WHERE id = $3"#,
            name_provided,
            name_value,
            id
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn update_executor(
        pool: &SqlitePool,
        id: Uuid,
        executor: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query!(
            r#"UPDATE sessions SET executor = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2"#,
            executor,
            id
        )
        .execute(pool)
        .await?;
        Ok(())
    }
}

fn directory_source_entry_name(source: &WorkspaceSource) -> Option<String> {
    source.path.as_deref().and_then(|path| {
        let raw_path = std::path::Path::new(path);
        let effective_path = raw_path
            .canonicalize()
            .unwrap_or_else(|_| raw_path.to_path_buf());

        directory_workspace_entry_name(source.display_name.as_deref(), &effective_path)
    })
}

#[cfg(test)]
mod tests {
    use std::{fs, str::FromStr};

    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use tokio::runtime::Builder;
    use uuid::Uuid;

    use super::{CreateSession, Session};
    use crate::models::{
        workspace::{CreateWorkspace, Workspace, WorkspaceMode},
        workspace_source::{CreateWorkspaceSource, WorkspaceSource},
    };

    async fn test_pool() -> SqlitePool {
        let db_path =
            std::env::temp_dir().join(format!("db-session-tests-{}.sqlite", Uuid::new_v4()));
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
    fn create_sets_agent_working_dir_for_single_directory_workspace_source() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{workspace_id}"),
                    workspace_mode: WorkspaceMode::InPlaceDirectory,
                    name: Some("Directory workspace".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            WorkspaceSource::create_many(
                &pool,
                workspace_id,
                &[CreateWorkspaceSource::Directory {
                    path: "/tmp/non-git-project".to_string(),
                    display_name: Some("non-git-project".to_string()),
                }],
            )
            .await
            .unwrap();

            let session = Session::create(
                &pool,
                &CreateSession {
                    executor: Some("CODEX".to_string()),
                    name: None,
                },
                Uuid::new_v4(),
                workspace_id,
            )
            .await
            .unwrap();

            assert_eq!(
                session.agent_working_dir.as_deref(),
                Some("non-git-project")
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn create_sets_agent_working_dir_for_directory_source_without_display_name_from_canonical_symlink_target()
     {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_id = Uuid::new_v4();
            let current_dir = std::env::current_dir().unwrap();
            let source_dir = current_dir
                .join("target")
                .join(format!("db-session-real-dir-{}", Uuid::new_v4()));
            fs::create_dir_all(&source_dir).unwrap();
            let linked_dir = current_dir
                .join("target")
                .join(format!("db-session-linked-dir-{}", Uuid::new_v4()));
            std::os::unix::fs::symlink(&source_dir, &linked_dir).unwrap();

            let persisted_path = linked_dir
                .strip_prefix(&current_dir)
                .unwrap()
                .to_string_lossy()
                .to_string();

            Workspace::create(
                &pool,
                &CreateWorkspace {
                    branch: format!("branch-{workspace_id}"),
                    workspace_mode: WorkspaceMode::InPlaceDirectory,
                    name: Some("Directory workspace".to_string()),
                },
                workspace_id,
            )
            .await
            .unwrap();

            WorkspaceSource::create_many(
                &pool,
                workspace_id,
                &[CreateWorkspaceSource::Directory {
                    path: persisted_path,
                    display_name: None,
                }],
            )
            .await
            .unwrap();

            let session = Session::create(
                &pool,
                &CreateSession {
                    executor: Some("CODEX".to_string()),
                    name: None,
                },
                Uuid::new_v4(),
                workspace_id,
            )
            .await
            .unwrap();

            assert_eq!(
                session.agent_working_dir.as_deref(),
                source_dir.file_name().and_then(|name| name.to_str())
            );

            let _ = fs::remove_file(&linked_dir);
            let _ = fs::remove_dir_all(&source_dir);
        });
    }
}
