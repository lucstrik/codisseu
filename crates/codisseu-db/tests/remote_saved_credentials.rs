//! Explicit integration with the desktop's real OS vault and disposable MySQL.
//! Touches only fresh unique Codisseu profile identities, never user entries.
use codisseu_db::{
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService},
    remote_credentials::CredentialStore,
};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

const PASSWORD: &str = "codisseu_fixture";

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
struct Cleanup(Vec<ConnectionConfig>);
impl Drop for Cleanup {
    fn drop(&mut self) {
        runtime().block_on(async {
            let vault = CredentialStore::new();
            for config in &self.0 {
                if let Err(error) = vault.delete(config).await {
                    eprintln!(
                        "Owned fixture credential cleanup failed for {}: {error}",
                        config.id
                    );
                }
            }
        });
    }
}
fn settle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "Remote fixture operation timed out"
        );
        thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    settle(service);
}
fn success(service: &RemoteService) {
    assert!(
        service.error.is_none(),
        "{}",
        service.error.as_deref().unwrap_or("")
    );
}
fn stored(vault: &CredentialStore, config: &ConnectionConfig) {
    runtime().block_on(async {
        let secret = vault
            .load(config)
            .await
            .unwrap()
            .expect("owned fixture secret missing");
        assert!(
            secret.expose() == PASSWORD,
            "owned fixture password differs"
        );
    });
}
fn absent(vault: &CredentialStore, config: &ConnectionConfig) {
    runtime().block_on(async {
        assert!(
            vault.load(config).await.unwrap().is_none(),
            "owned fixture entry was not removed"
        );
    });
}
fn no_file_secret(path: &Path) {
    let json = std::fs::read_to_string(path).unwrap();
    assert!(
        !json.contains(PASSWORD),
        "profile JSON contains fixture password"
    );
    assert!(
        !json.contains("\"password\""),
        "profile JSON contains a password field"
    );
}
fn connect_and_query(service: &mut RemoteService, config: &ConnectionConfig) {
    send(
        service,
        RemoteCommand::ConnectRemembered {
            config: config.clone(),
            password: String::new(),
            test: false,
        },
    );
    success(service);
    assert!(service.active.is_some());
    send(
        service,
        RemoteCommand::Execute("SELECT 1 AS remembered_connection;".into()),
    );
    success(service);
    assert_eq!(
        service.result.as_ref().unwrap().rows,
        vec![vec![Cell::Number("1".into())]]
    );
    send(service, RemoteCommand::Disconnect);
    success(service);
    assert!(service.active.is_none());
}

#[test]
#[ignore = "requires owned MySQL and OS vault: CODISSEU_TEST_SYSTEM_KEYRING=1 CODISSEU_SQL_WORKFLOW_MY_PORT"]
fn saved_console_execution_connects_its_exact_source_after_restart_and_preserves_pending_edits() {
    use codisseu_db::remote::{CellEdit, EditBatch, TableRef};
    use sqlx_core::{connection::Connection, query::query};
    assert_eq!(std::env::var("CODISSEU_TEST_SYSTEM_KEYRING").unwrap(), "1");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sources.json");
    let id = format!(
        "owned-console-{}-{}",
        std::process::id(),
        directory.path().file_name().unwrap().to_string_lossy()
    );
    let dev = ConnectionConfig {
        id: format!("{id}-dev"),
        name: "Same display name".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: std::env::var("CODISSEU_SQL_WORKFLOW_MY_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        username: "root".into(),
        database: "codisseu".into(),
        tls: false,
        read_only: false,
        save_credentials: true,
        timeout_secs: 3,
        ..Default::default()
    };
    let qa = ConnectionConfig {
        id: format!("{id}-qa"),
        database: "information_schema".into(),
        read_only: true,
        ..dev.clone()
    };
    let bad = ConnectionConfig {
        id: format!("{id}-bad"),
        ..qa.clone()
    };
    let cleanup = Cleanup(vec![dev.clone(), qa.clone(), bad.clone()]);
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    let mut files = Vec::new();
    for source in [&dev, &qa, &bad] {
        send(
            &mut service,
            RemoteCommand::SaveRemembered {
                config: source.clone(),
                password: if source.id == bad.id {
                    "wrong-fixture-secret".into()
                } else {
                    PASSWORD.into()
                },
            },
        );
        success(&service);
        send(
            &mut service,
            RemoteCommand::CreateConsoleFor(source.id.clone()),
        );
        success(&service);
        let file = service.console_path.take().unwrap();
        std::fs::write(&file, "SELECT DATABASE() AS source").unwrap();
        files.push(file);
    }
    assert!(service.active.is_none(), "save/create never connects");
    drop(service);
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    assert!(service.active.is_none(), "restart never connects");
    let execute = |service: &mut RemoteService, file: &std::path::Path| {
        let datasource_id = service.console_target(file).unwrap().unwrap().to_owned();
        send(
            service,
            RemoteCommand::ExecuteConsole {
                sql: std::fs::read_to_string(file).unwrap(),
                datasource_id,
            },
        );
    };
    execute(&mut service, &files[1]);
    success(&service);
    assert_eq!(service.active.as_ref().unwrap().id, qa.id);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Text(qa.database.clone())
    );
    execute(&mut service, &files[0]);
    success(&service);
    assert_eq!(service.active.as_ref().unwrap().id, dev.id);
    assert_eq!(
        service.result.as_ref().unwrap().rows[0][0],
        Cell::Text(dev.database.clone())
    );
    assert!(!service.transaction);
    // A pending edit must survive an attempt to run a query from another source.
    let rt = runtime();
    let mut admin = rt
        .block_on(sqlx_mysql::MySqlConnection::connect_with(
            &sqlx_mysql::MySqlConnectOptions::new()
                .host("127.0.0.1")
                .port(dev.port)
                .username("root")
                .password(PASSWORD)
                .database("codisseu"),
        ))
        .unwrap();
    let table = TableRef {
        namespace: "codisseu".into(),
        name: format!("console_auto_{}", std::process::id()),
    };
    for sql in [
        format!(
            "CREATE TABLE `{}` (id BIGINT PRIMARY KEY, value VARCHAR(30)) ENGINE=InnoDB",
            table.name
        ),
        format!("INSERT INTO `{}` VALUES (1,'original')", table.name),
    ] {
        rt.block_on(query(&sql).execute(&mut admin)).unwrap();
    }
    send(&mut service, RemoteCommand::LoadTable(table.clone()));
    success(&service);
    let columns = service.origin.as_ref().unwrap().1.clone();
    let row = service.result.as_ref().unwrap().rows[0].clone();
    send(
        &mut service,
        RemoteCommand::Apply(EditBatch {
            table: table.clone(),
            columns,
            edits: vec![CellEdit {
                row,
                column: "value".into(),
                value: Cell::Text("pending".into()),
            }],
        }),
    );
    success(&service);
    assert!(service.transaction);
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteConsole {
                sql: "SELECT DATABASE()".into(),
                datasource_id: qa.id.clone()
            })
            .is_err()
    );
    assert!(service.transaction);
    assert_eq!(service.active.as_ref().unwrap().id, dev.id);
    send(&mut service, RemoteCommand::Rollback);
    success(&service);
    execute(&mut service, &files[1]);
    success(&service);
    execute(&mut service, &files[2]);
    assert!(service.error.is_some(), "wrong target credential must fail");
    assert!(service.active.is_none());
    assert!(service.result.is_none());
    assert!(
        !service
            .error
            .as_ref()
            .unwrap()
            .contains("wrong-fixture-secret")
    );
    send(
        &mut service,
        RemoteCommand::SaveRemembered {
            config: bad.clone(),
            password: PASSWORD.into(),
        },
    );
    success(&service);
    execute(&mut service, &files[2]);
    success(&service);
    assert_eq!(service.active.as_ref().unwrap().id, bad.id);
    send(&mut service, RemoteCommand::DeleteRemembered(qa.id.clone()));
    success(&service);
    assert!(
        service
            .dispatch(RemoteCommand::ExecuteConsole {
                sql: "SELECT DATABASE()".into(),
                datasource_id: qa.id.clone()
            })
            .is_err()
    );
    assert_eq!(service.active.as_ref().unwrap().id, bad.id);
    assert!(files[1].exists(), "removing source preserves saved query");
    send(&mut service, RemoteCommand::Disconnect);
    no_file_secret(&path);
    rt.block_on(query(&format!("DROP TABLE `{}`", table.name)).execute(&mut admin))
        .unwrap();
    rt.block_on(admin.close()).unwrap();
    drop(service);
    drop(cleanup);
    let vault = CredentialStore::new();
    for source in [&dev, &qa, &bad] {
        absent(&vault, source);
    }
}

#[test]
#[ignore = "requires normal desktop OS vault and disposable MySQL; CODISSEU_TEST_SYSTEM_KEYRING=1 CODISSEU_SQL_WORKFLOW_MY_PORT"]
fn remembered_password_survives_worker_restart_without_plaintext_and_is_deleted_explicitly() {
    assert_eq!(std::env::var("CODISSEU_TEST_SYSTEM_KEYRING").unwrap(), "1");
    assert!(
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
        "run in the normal Linux desktop session"
    );
    let port = std::env::var("CODISSEU_SQL_WORKFLOW_MY_PORT")
        .expect("disposable MySQL port required")
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let id = format!(
        "owned-keyring-workflow-{}-{}",
        std::process::id(),
        directory.path().file_name().unwrap().to_string_lossy()
    );
    let config = ConnectionConfig {
        id: id.clone(),
        name: "Owned keyring workflow fixture".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port,
        username: "root".into(),
        database: String::new(),
        tls: false,
        read_only: true,
        save_credentials: true,
        timeout_secs: 3,
        idle_secs: 60,
        ..Default::default()
    };
    let mut changed = config.clone();
    changed.database = "corrier".into();
    let mut host_case = config.clone();
    host_case.id.push_str("-host-case");
    host_case.host = "fixture.example.invalid".into();
    let _cleanup = Cleanup(vec![config.clone(), changed.clone(), host_case.clone()]);
    let path = directory.path().join("profiles.json");
    let vault = CredentialStore::new();
    absent(&vault, &config);
    let mut service = RemoteService::new(path.clone());
    settle(&mut service);
    send(
        &mut service,
        RemoteCommand::SaveRemembered {
            config: config.clone(),
            password: PASSWORD.into(),
        },
    );
    success(&service);
    assert!(service.active.is_none(), "saving does not auto-connect");
    assert_eq!(service.profiles.len(), 1);
    assert!(service.profiles[0].save_credentials);
    stored(&vault, &config);
    no_file_secret(&path);
    drop(service);

    let mut reopened = RemoteService::new(path.clone());
    settle(&mut reopened);
    assert!(reopened.active.is_none(), "restore must stay disconnected");
    connect_and_query(&mut reopened, &config);
    drop(reopened);
    let mut reopened = RemoteService::new(path.clone());
    settle(&mut reopened);
    connect_and_query(&mut reopened, &config);

    send(
        &mut reopened,
        RemoteCommand::ConnectRemembered {
            config: config.clone(),
            password: String::new(),
            test: true,
        },
    );
    success(&reopened);
    assert!(
        reopened.active.is_none(),
        "Test Connection must close its only socket"
    );
    let bad_password = format!("wrong-owned-fixture-{id}");
    send(
        &mut reopened,
        RemoteCommand::ConnectRemembered {
            config: config.clone(),
            password: bad_password.clone(),
            test: true,
        },
    );
    let error = reopened
        .error
        .as_deref()
        .expect("bad typed password must fail");
    assert!(!error.contains(&bad_password));
    assert!(!error.contains(PASSWORD));
    assert!(reopened.active.is_none());
    stored(&vault, &config);
    no_file_secret(&path);

    absent(&vault, &changed);
    send(
        &mut reopened,
        RemoteCommand::ConnectRemembered {
            config: changed.clone(),
            password: String::new(),
            test: false,
        },
    );
    assert!(
        reopened.error.is_some(),
        "changed credential identity must not reuse the old password"
    );
    assert!(reopened.active.is_none());
    absent(&vault, &changed);
    stored(&vault, &config);

    send(
        &mut reopened,
        RemoteCommand::SaveRemembered {
            config: host_case.clone(),
            password: PASSWORD.into(),
        },
    );
    success(&reopened);
    host_case.host = host_case.host.to_ascii_uppercase();
    send(
        &mut reopened,
        RemoteCommand::SaveRemembered {
            config: host_case.clone(),
            password: PASSWORD.into(),
        },
    );
    success(&reopened);
    stored(&vault, &host_case);
    send(
        &mut reopened,
        RemoteCommand::DeleteRemembered(host_case.id.clone()),
    );
    success(&reopened);
    absent(&vault, &host_case);

    let mut forget = config.clone();
    forget.save_credentials = false;
    send(
        &mut reopened,
        RemoteCommand::SaveRemembered {
            config: forget,
            password: String::new(),
        },
    );
    success(&reopened);
    assert!(!reopened.profiles[0].save_credentials);
    absent(&vault, &config);
    no_file_secret(&path);
    send(
        &mut reopened,
        RemoteCommand::SaveRemembered {
            config: config.clone(),
            password: PASSWORD.into(),
        },
    );
    success(&reopened);
    stored(&vault, &config);
    send(
        &mut reopened,
        RemoteCommand::DeleteRemembered(config.id.clone()),
    );
    success(&reopened);
    assert!(reopened.profiles.is_empty());
    absent(&vault, &config);
    no_file_secret(&path);
}
