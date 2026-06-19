use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

pub const OPENCODE_TIME_TRACKING_SCOPE: &str = "time_tracking:write";
pub const OPENCODE_TIME_TRACKING_TOKEN_PREFIX: &str = "vktt_";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpencodeTimeTrackingToken {
    pub id: Uuid,
    pub token_hash: String,
    pub scope: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl OpencodeTimeTrackingToken {
    pub async fn create(
        pool: &SqlitePool,
        token_hash: &str,
        label: Option<&str>,
    ) -> Result<Self, sqlx::Error> {
        let id = Uuid::new_v4();

        sqlx::query_as::<_, Self>(
            r#"
            INSERT INTO opencode_time_tracking_tokens (id, token_hash, scope, label)
            VALUES (?, ?, ?, ?)
            RETURNING id, token_hash, scope, label, created_at, last_used_at, revoked_at
            "#,
        )
        .bind(id)
        .bind(token_hash)
        .bind(OPENCODE_TIME_TRACKING_SCOPE)
        .bind(label)
        .fetch_one(pool)
        .await
    }

    pub async fn find_active_by_hash(
        pool: &SqlitePool,
        token_hash: &str,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(
            r#"
            SELECT id, token_hash, scope, label, created_at, last_used_at, revoked_at
            FROM opencode_time_tracking_tokens
            WHERE token_hash = ? AND revoked_at IS NULL
            "#,
        )
        .bind(token_hash)
        .fetch_optional(pool)
        .await
    }

    pub async fn list(pool: &SqlitePool) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(
            r#"
            SELECT id, token_hash, scope, label, created_at, last_used_at, revoked_at
            FROM opencode_time_tracking_tokens
            ORDER BY created_at DESC, id ASC
            "#,
        )
        .fetch_all(pool)
        .await
    }

    pub async fn mark_used(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE opencode_time_tracking_tokens
            SET last_used_at = datetime('now', 'subsec')
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn revoke(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE opencode_time_tracking_tokens
            SET revoked_at = COALESCE(revoked_at, datetime('now', 'subsec'))
            WHERE id = ?
            "#,
        )
        .bind(id)
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{str::FromStr, time::Duration};

    use sqlx::{ConnectOptions, SqlitePool, sqlite::SqliteConnectOptions};
    use tokio::runtime::Builder;
    use uuid::Uuid;

    use super::{OPENCODE_TIME_TRACKING_SCOPE, OpencodeTimeTrackingToken};

    async fn test_pool() -> SqlitePool {
        let db_path = std::env::temp_dir().join(format!(
            "db-opencode-time-tracking-token-tests-{}.sqlite",
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
    fn create_then_find_active_by_hash() {
        run_async_test(async {
            let pool = test_pool().await;

            let created = OpencodeTimeTrackingToken::create(
                &pool,
                "hash-create-find",
                Some("local opencode plugin"),
            )
            .await
            .unwrap();

            let found = OpencodeTimeTrackingToken::find_active_by_hash(&pool, "hash-create-find")
                .await
                .unwrap()
                .expect("created token should be active");

            assert_eq!(found.id, created.id);
            assert_eq!(found.token_hash, "hash-create-find");
            assert_eq!(found.scope, OPENCODE_TIME_TRACKING_SCOPE);
            assert_eq!(found.label.as_deref(), Some("local opencode plugin"));
            assert_eq!(found.created_at, created.created_at);
            assert!(found.last_used_at.is_none());
            assert!(found.revoked_at.is_none());
        });
    }

    #[test]
    fn revoked_tokens_are_not_returned_by_active_hash_lookup() {
        run_async_test(async {
            let pool = test_pool().await;
            let created = OpencodeTimeTrackingToken::create(&pool, "hash-revoked", None)
                .await
                .unwrap();

            OpencodeTimeTrackingToken::revoke(&pool, created.id)
                .await
                .unwrap();

            let found = OpencodeTimeTrackingToken::find_active_by_hash(&pool, "hash-revoked")
                .await
                .unwrap();

            assert!(found.is_none());

            let tokens = OpencodeTimeTrackingToken::list(&pool).await.unwrap();
            assert_eq!(tokens.len(), 1);
            assert!(tokens[0].revoked_at.is_some());
        });
    }

    #[test]
    fn mark_used_updates_last_used_at() {
        run_async_test(async {
            let pool = test_pool().await;
            let created = OpencodeTimeTrackingToken::create(&pool, "hash-mark-used", None)
                .await
                .unwrap();
            assert!(created.last_used_at.is_none());

            OpencodeTimeTrackingToken::mark_used(&pool, created.id)
                .await
                .unwrap();

            let found = OpencodeTimeTrackingToken::find_active_by_hash(&pool, "hash-mark-used")
                .await
                .unwrap()
                .expect("used token should remain active");

            assert_eq!(found.id, created.id);
            assert!(found.last_used_at.is_some());
            assert!(found.revoked_at.is_none());
        });
    }

    #[test]
    fn revoke_preserves_original_revoked_at_and_allows_unknown_ids() {
        run_async_test(async {
            let pool = test_pool().await;
            let created = OpencodeTimeTrackingToken::create(&pool, "hash-revoke-idempotent", None)
                .await
                .unwrap();

            OpencodeTimeTrackingToken::revoke(&pool, created.id)
                .await
                .unwrap();
            let first_revoked_at = OpencodeTimeTrackingToken::list(&pool)
                .await
                .unwrap()
                .into_iter()
                .find(|token| token.id == created.id)
                .unwrap()
                .revoked_at
                .expect("first revoke should set revoked_at");

            tokio::time::sleep(Duration::from_millis(5)).await;

            OpencodeTimeTrackingToken::revoke(&pool, created.id)
                .await
                .unwrap();
            OpencodeTimeTrackingToken::revoke(&pool, Uuid::new_v4())
                .await
                .unwrap();

            let second_revoked_at = OpencodeTimeTrackingToken::list(&pool)
                .await
                .unwrap()
                .into_iter()
                .find(|token| token.id == created.id)
                .unwrap()
                .revoked_at
                .expect("second revoke should keep revoked_at");

            assert_eq!(second_revoked_at, first_revoked_at);
        });
    }
}
