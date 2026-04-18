use std::str::FromStr;

use db::{
    DBService,
    models::workspace::{CreateWorkspace, Workspace, WorkspaceMode},
};
use sqlx::{ConnectOptions, Row, SqlitePool, sqlite::SqliteConnectOptions};
use utils::assets::asset_dir;
use uuid::Uuid;

pub async fn isolated_test_db() -> DBService {
    let db_path = std::env::temp_dir().join(format!("services-tests-{}.sqlite", Uuid::new_v4()));
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

    DBService { pool }
}

#[tokio::test]
async fn isolated_test_db_does_not_write_to_shared_dev_db() {
    let db = isolated_test_db().await;
    let workspace_id = Uuid::new_v4();

    Workspace::create(
        &db.pool,
        &CreateWorkspace {
            branch: format!("isolated-test-{workspace_id}"),
            workspace_mode: WorkspaceMode::InPlaceDirectory,
            name: Some(format!("Isolated test workspace {workspace_id}")),
        },
        workspace_id,
    )
    .await
    .unwrap();

    let shared_database_url = format!(
        "sqlite://{}",
        asset_dir().join("db.v2.sqlite").to_string_lossy()
    );
    let shared_pool = sqlx::SqlitePool::connect(&shared_database_url)
        .await
        .unwrap();
    let leaked_count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM workspaces WHERE id = ?")
        .bind(workspace_id)
        .fetch_one(&shared_pool)
        .await
        .unwrap()
        .get("count");

    sqlx::query("DELETE FROM workspaces WHERE id = ?")
        .bind(workspace_id)
        .execute(&db.pool)
        .await
        .unwrap();

    assert_eq!(leaked_count, 0);
}
