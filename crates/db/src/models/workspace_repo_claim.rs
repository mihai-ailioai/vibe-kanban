use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct WorkspaceRepoClaim {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub repo_id: Uuid,
    #[ts(type = "Date")]
    pub created_at: DateTime<Utc>,
    #[ts(type = "Date")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, TS)]
pub struct CreateWorkspaceRepoClaim {
    pub repo_id: Uuid,
}

impl WorkspaceRepoClaim {
    pub async fn create_many(
        pool: &SqlitePool,
        workspace_id: Uuid,
        claims: &[CreateWorkspaceRepoClaim],
    ) -> Result<Vec<Self>, sqlx::Error> {
        if claims.is_empty() {
            return Ok(Vec::new());
        }

        let mut tx = pool.begin().await?;
        let mut results = Vec::with_capacity(claims.len());

        for claim in claims {
            let existing = sqlx::query_as!(
                WorkspaceRepoClaim,
                r#"SELECT id as "id!: Uuid",
                          workspace_id as "workspace_id!: Uuid",
                          repo_id as "repo_id!: Uuid",
                          created_at as "created_at!: DateTime<Utc>",
                          updated_at as "updated_at!: DateTime<Utc>"
                   FROM workspace_repo_claims
                   WHERE repo_id = $1"#,
                claim.repo_id,
            )
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(existing) = existing
                && existing.workspace_id == workspace_id
            {
                results.push(existing);
                continue;
            }

            let id = Uuid::new_v4();
            let workspace_repo_claim = sqlx::query_as!(
                WorkspaceRepoClaim,
                r#"INSERT INTO workspace_repo_claims (id, workspace_id, repo_id)
                   VALUES ($1, $2, $3)
                   RETURNING id as "id!: Uuid",
                             workspace_id as "workspace_id!: Uuid",
                             repo_id as "repo_id!: Uuid",
                             created_at as "created_at!: DateTime<Utc>",
                             updated_at as "updated_at!: DateTime<Utc>""#,
                id,
                workspace_id,
                claim.repo_id,
            )
            .fetch_one(&mut *tx)
            .await?;
            results.push(workspace_repo_claim);
        }

        tx.commit().await?;
        Ok(results)
    }

    pub async fn find_by_workspace_id(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceRepoClaim,
            r#"SELECT id as "id!: Uuid",
                      workspace_id as "workspace_id!: Uuid",
                      repo_id as "repo_id!: Uuid",
                      created_at as "created_at!: DateTime<Utc>",
                      updated_at as "updated_at!: DateTime<Utc>"
               FROM workspace_repo_claims
               WHERE workspace_id = $1
               ORDER BY repo_id ASC"#,
            workspace_id,
        )
        .fetch_all(pool)
        .await
    }

    pub async fn find_conflicting_repo_ids(
        pool: &SqlitePool,
        workspace_id: Uuid,
        repo_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        if repo_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut query_builder: QueryBuilder<Sqlite> =
            QueryBuilder::new("SELECT repo_id FROM workspace_repo_claims WHERE workspace_id != ");
        query_builder.push_bind(workspace_id);
        query_builder.push(" AND repo_id IN (");

        let mut separated = query_builder.separated(", ");
        for repo_id in repo_ids {
            separated.push_bind(repo_id);
        }
        separated.push_unseparated(") ORDER BY repo_id ASC");

        let conflicts = query_builder
            .build_query_scalar::<Uuid>()
            .fetch_all(pool)
            .await?;

        Ok(conflicts
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    pub async fn release_for_workspace(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query!(
            "DELETE FROM workspace_repo_claims WHERE workspace_id = $1",
            workspace_id
        )
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use std::{path::Path, str::FromStr};

    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use tokio::runtime::Builder;

    use super::{CreateWorkspaceRepoClaim, WorkspaceRepoClaim, *};
    use crate::models::{
        repo::Repo,
        workspace::{CreateWorkspace, Workspace, WorkspaceMode},
    };

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "db-workspace-repo-claim-tests-{}.sqlite",
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

    async fn create_workspace(pool: &SqlitePool, label: &str) -> Workspace {
        let workspace_id = Uuid::new_v4();
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
            "workspace-repo-claim-repo-{}-{}",
            name,
            Uuid::new_v4()
        ));
        Repo::find_or_create(pool, Path::new(&repo_path), name)
            .await
            .unwrap()
    }

    #[test]
    fn create_many_rejects_duplicate_repo_claims() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace_one = create_workspace(&pool, "one").await;
            let workspace_two = create_workspace(&pool, "two").await;
            let repo = create_repo(&pool, "shared-repo").await;

            WorkspaceRepoClaim::create_many(
                &pool,
                workspace_one.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap();

            let err = WorkspaceRepoClaim::create_many(
                &pool,
                workspace_two.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap_err();

            assert!(err.to_string().contains("UNIQUE") || err.to_string().contains("unique"));
        });
    }

    #[test]
    fn create_many_allows_reacquiring_claims_for_same_workspace() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace = create_workspace(&pool, "same-workspace").await;
            let repo = create_repo(&pool, "shared-repo").await;

            let created = WorkspaceRepoClaim::create_many(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap();

            let reacquired = WorkspaceRepoClaim::create_many(
                &pool,
                workspace.id,
                &[CreateWorkspaceRepoClaim { repo_id: repo.id }],
            )
            .await
            .unwrap();

            assert_eq!(reacquired.len(), 1);
            assert_eq!(reacquired[0].id, created[0].id);
            assert_eq!(reacquired[0].workspace_id, workspace.id);
            assert_eq!(reacquired[0].repo_id, repo.id);
        });
    }

    #[test]
    fn release_for_workspace_removes_all_claims() {
        run_async_test(async {
            let pool = test_pool().await;
            let workspace = create_workspace(&pool, "owned").await;
            let repo_one = create_repo(&pool, "repo-one").await;
            let repo_two = create_repo(&pool, "repo-two").await;

            WorkspaceRepoClaim::create_many(
                &pool,
                workspace.id,
                &[
                    CreateWorkspaceRepoClaim {
                        repo_id: repo_one.id,
                    },
                    CreateWorkspaceRepoClaim {
                        repo_id: repo_two.id,
                    },
                ],
            )
            .await
            .unwrap();

            let released = WorkspaceRepoClaim::release_for_workspace(&pool, workspace.id)
                .await
                .unwrap();
            assert_eq!(released, 2);

            let remaining = WorkspaceRepoClaim::find_by_workspace_id(&pool, workspace.id)
                .await
                .unwrap();
            assert!(remaining.is_empty());
        });
    }

    #[test]
    fn find_conflicting_repo_ids_excludes_same_workspace() {
        run_async_test(async {
            let pool = test_pool().await;
            let owning_workspace = create_workspace(&pool, "owner").await;
            let other_workspace = create_workspace(&pool, "other").await;
            let repo_one = create_repo(&pool, "repo-one").await;
            let repo_two = create_repo(&pool, "repo-two").await;

            WorkspaceRepoClaim::create_many(
                &pool,
                owning_workspace.id,
                &[
                    CreateWorkspaceRepoClaim {
                        repo_id: repo_one.id,
                    },
                    CreateWorkspaceRepoClaim {
                        repo_id: repo_two.id,
                    },
                ],
            )
            .await
            .unwrap();

            let same_workspace_conflicts = WorkspaceRepoClaim::find_conflicting_repo_ids(
                &pool,
                owning_workspace.id,
                &[repo_one.id, repo_two.id],
            )
            .await
            .unwrap();
            assert!(same_workspace_conflicts.is_empty());

            let other_workspace_conflicts = WorkspaceRepoClaim::find_conflicting_repo_ids(
                &pool,
                other_workspace.id,
                &[repo_one.id, repo_two.id],
            )
            .await
            .unwrap();
            assert_eq!(other_workspace_conflicts.len(), 2);
            assert!(other_workspace_conflicts.contains(&repo_one.id));
            assert!(other_workspace_conflicts.contains(&repo_two.id));
        });
    }
}
