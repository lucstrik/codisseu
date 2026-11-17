//! Worker/cache integration on the task-owned disposable MySQL fixture only.
use codisseu_db::remote::{Cell, ConnectionConfig, Engine, RemoteCommand, RemoteService, TableRef};
use std::time::{Duration, Instant};

fn idle(service: &mut RemoteService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(
            Instant::now() < deadline,
            "stuck: {} {:?}",
            service.status,
            service.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    service.poll();
}
fn send(service: &mut RemoteService, command: RemoteCommand) {
    service.dispatch(command).unwrap();
    idle(service);
    assert!(service.error.is_none(), "{}", service.status);
}
fn config() -> ConnectionConfig {
    ConnectionConfig {
        id: "catalog-worker-one".into(),
        name: "Disposable MySQL catalog worker".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: std::env::var("CODISSEU_SQL_WORKFLOW_MY_PORT")
            .expect("explicit disposable fixture port")
            .parse()
            .unwrap(),
        database: String::new(),
        username: "root".into(),
        tls: false,
        read_only: true,
        timeout_secs: 2,
        idle_secs: 60,
        row_limit: 2,
        ..ConnectionConfig::default()
    }
}
fn connect(service: &mut RemoteService, config: &ConnectionConfig) {
    send(
        service,
        RemoteCommand::Connect {
            config: config.clone(),
            password: "codisseu_fixture".into(),
            test: false,
        },
    );
}
fn execute(service: &mut RemoteService, id: &str, sql: &str) {
    send(
        service,
        RemoteCommand::ExecuteBound {
            sql: sql.into(),
            datasource_id: id.into(),
        },
    );
}
fn query_identity(service: &mut RemoteService, id: &str) -> Cell {
    execute(service, id, "SELECT CONNECTION_ID()");
    service.result.as_ref().unwrap().rows[0][0].clone()
}
#[test]
#[ignore = "requires seeded task-owned MySQL; explicit CODISSEU_SQL_WORKFLOW_MY_PORT"]
fn catalog_worker_keeps_safe_errors_scoped_and_clears_caches_on_session_change() {
    let config = config();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("datasources.json");
    let mut service = RemoteService::new(path.clone());
    idle(&mut service);
    assert!(service.active.is_none());
    assert!(!service.namespaces_loaded);
    connect(&mut service, &config);
    let identity = query_identity(&mut service, &config.id);
    let revision = service.revision;
    send(&mut service, RemoteCommand::ListNamespaces);
    assert!(service.namespaces_loaded);
    assert!(service.namespaces.iter().any(|name| name == "corrier"));
    assert!(service.namespaces.iter().any(|name| name == "corrier_fat"));
    assert!(
        service.namespace_objects.is_empty(),
        "listing schemas must not eagerly list tables"
    );
    assert!(service.table_columns.is_empty());
    assert_eq!(
        service.revision, revision,
        "metadata must not replace a query result"
    );
    assert_eq!(service.result.as_ref().unwrap().rows[0][0], identity);
    send(
        &mut service,
        RemoteCommand::InspectNamespace("corrier".into()),
    );
    let objects = service.namespace_objects.get("corrier").unwrap();
    assert!(objects.iter().any(|object| object.name == "remetentes"));
    assert!(
        objects
            .iter()
            .all(|object| object.namespace == "corrier" && object.columns.is_empty())
    );
    assert!(!service.namespace_objects.contains_key("corrier_fat"));
    assert!(
        service.table_columns.is_empty(),
        "table listing must not fetch columns"
    );
    let table = TableRef {
        namespace: "corrier".into(),
        name: "remetentes".into(),
    };
    send(&mut service, RemoteCommand::InspectColumns(table.clone()));
    let columns = service
        .table_columns
        .get(&(table.namespace.clone(), table.name.clone()))
        .unwrap();
    assert_eq!(
        columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "nome", "ativo"]
    );
    assert!(columns[0].primary_key);
    send(
        &mut service,
        RemoteCommand::InspectNamespace("corrier_fat".into()),
    );
    assert!(
        service
            .namespace_objects
            .get("corrier_fat")
            .unwrap()
            .iter()
            .any(|object| object.name == "fatcte")
    );
    execute(
        &mut service,
        &config.id,
        "SELECT * FROM corrier.remetentes ORDER BY id",
    );
    assert_eq!(service.result.as_ref().unwrap().rows.len(), 2);
    assert!(service.result.as_ref().unwrap().truncated);
    for sql in [
        "SELECT corrier.remetentes LIMIT 10",
        "SELECT * FROM corrier.table_does_not_exist",
    ] {
        service
            .dispatch(RemoteCommand::ExecuteBound {
                sql: sql.into(),
                datasource_id: config.id.clone(),
            })
            .unwrap();
        idle(&mut service);
        assert!(service.error.as_ref().unwrap().contains("42"));
        assert!(
            service
                .error
                .as_ref()
                .unwrap()
                .contains("sessão preservada")
        );
        assert_eq!(service.active.as_ref().unwrap().id, config.id);
        assert!(service.namespaces_loaded);
        assert!(service.namespace_objects.contains_key("corrier"));
        assert!(
            service
                .table_columns
                .contains_key(&(table.namespace.clone(), table.name.clone()))
        );
        assert!(!service.transaction);
        assert_eq!(query_identity(&mut service, &config.id), identity);
    }
    // A metadata failure must not unbind a healthy session either.
    service
        .dispatch(RemoteCommand::InspectColumns(TableRef {
            namespace: "corrier".into(),
            name: "missing".into(),
        }))
        .unwrap();
    idle(&mut service);
    assert!(
        service
            .error
            .as_ref()
            .unwrap()
            .contains("sessão preservada")
    );
    assert_eq!(query_identity(&mut service, &config.id), identity);
    let mut next = config.clone();
    next.id = "catalog-worker-two".into();
    connect(&mut service, &next);
    assert_eq!(service.active.as_ref().unwrap().id, next.id);
    assert!(!service.namespaces_loaded);
    assert!(service.namespaces.is_empty());
    assert!(service.namespace_objects.is_empty());
    assert!(service.table_columns.is_empty());
    assert_ne!(
        query_identity(&mut service, &next.id),
        identity,
        "explicit replacement must use new server session"
    );
    send(&mut service, RemoteCommand::ListNamespaces);
    send(
        &mut service,
        RemoteCommand::InspectNamespace("corrier".into()),
    );
    send(&mut service, RemoteCommand::InspectColumns(table));
    send(&mut service, RemoteCommand::Disconnect);
    assert!(service.active.is_none());
    assert!(service.namespace_objects.is_empty());
    assert!(service.table_columns.is_empty());
    assert!(service.namespaces.is_empty());
    assert!(!service.namespaces_loaded);
    let persisted = std::fs::read_to_string(path).unwrap();
    assert!(!persisted.contains("codisseu_fixture"));
    assert!(!persisted.contains("password"));
}

#[test]
#[ignore = "requires seeded task-owned MySQL; explicit CODISSEU_SQL_WORKFLOW_MY_PORT"]
fn cancel_during_query_cannot_be_misclassified_as_preserved_session() {
    let config = config();
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("datasources.json"));
    idle(&mut service);
    connect(&mut service, &config);
    send(&mut service, RemoteCommand::ListNamespaces);
    service.dispatch(RemoteCommand::ExecuteBound {
        sql: "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<999) SELECT SUM(a.x*b.x*c.x) FROM n a CROSS JOIN n b CROSS JOIN n c".into(),
        datasource_id: config.id,
    }).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    service.dispatch(RemoteCommand::Cancel).unwrap();
    idle(&mut service);
    assert!(
        service.active.is_none(),
        "dropping an in-flight query must close even if the adapter has not marked closed yet"
    );
    assert!(service.namespaces.is_empty());
    assert!(service.namespace_objects.is_empty());
    assert!(service.table_columns.is_empty());
    assert!(!service.namespaces_loaded);
    assert!(service.error.as_ref().unwrap().contains("conexão fechada"));
    assert!(
        !service
            .error
            .as_ref()
            .unwrap()
            .contains("sessão preservada")
    );
}
