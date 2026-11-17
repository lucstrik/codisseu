//! Explicit, task-owned loopback MySQL fixture. Never targets saved user profiles.
use codisseu_db::{
    remote::{Cell, ConnectionConfig, Engine},
    remote_sql::SqlSession,
};
use sqlx_core::{connection::Connection, query::query};
use sqlx_mysql::{MySqlConnectOptions, MySqlConnection};

async fn connect_empty_password(port: u16) -> Result<(), String> {
    let config = ConnectionConfig {
        id: "empty-password-fixture".into(),
        name: "Disposable empty password fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        username: "codisseu_empty".into(),
        database: "codisseu".into(),
        tls: false,
        read_only: true,
        save_credentials: false,
        timeout_secs: 3,
        ..Default::default()
    };
    let mut session = SqlSession::connect(&config, "").await?;
    let result = session.execute("SELECT 1 AS value").await;
    let close = session.close().await;
    let result = result?;
    close?;
    if result.rows != vec![vec![Cell::Number("1".into())]] {
        return Err("SELECT 1 did not return the expected typed value".into());
    }
    Ok(())
}

#[test]
#[ignore = "requires task-owned MySQL 8.0 loopback fixture; CODISSEU_SQL_MY_PORT"]
fn mysql_connects_without_password_with_caching_sha2_authentication() {
    let port = std::env::var("CODISSEU_SQL_MY_PORT")
        .expect("explicit task-owned loopback MySQL port")
        .parse()
        .unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let options = MySqlConnectOptions::new()
            .host("127.0.0.1")
            .port(port)
            .username("root")
            .password("codisseu_fixture")
            .database("codisseu");
        let mut admin = MySqlConnection::connect_with(&options).await.unwrap();
        // CREATE deliberately fails if a previous account exists: never remove
        // a pre-existing user as fixture preparation.
        query("CREATE USER 'codisseu_empty'@'%' IDENTIFIED WITH caching_sha2_password BY ''")
            .execute(&mut admin)
            .await
            .unwrap();
        let outcome = async {
            query("GRANT SELECT ON codisseu.* TO 'codisseu_empty'@'%'")
                .execute(&mut admin)
                .await
                .map_err(|error| error.to_string())?;
            connect_empty_password(port).await
        }
        .await;
        // Cleanup runs before checking the result, including connection failure.
        let cleanup = query("DROP USER 'codisseu_empty'@'%'")
            .execute(&mut admin)
            .await;
        cleanup.expect("remove only the account created by this test");
        outcome.unwrap();
        admin.close().await.unwrap();
    });
}
