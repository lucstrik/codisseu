//! Disposable replica set, synthetic credentials, TLS and socket lifecycle.
use super::*;
use crate::remote::{MongoOptions, RemoteCommand, RemoteService};
use futures_util::TryStreamExt;
use serde_json::json;

#[test]
fn mongo_driver_options_are_typed_bounded_and_never_retry_writes() {
    let config = ConnectionConfig {
        engine: Engine::Mongo,
        database: "fixture".into(),
        username: "fixture".into(),
        auth_database: "admin".into(),
        mongo: MongoOptions {
            replica_set: "fixture-rs".into(),
            authentication: MongoAuthentication::ScramSha1,
            read_preference: MongoReadPreference::SecondaryPreferred,
            direct_connection: false,
        },
        ..Default::default()
    };
    let options = mongo_options(&config, "synthetic").unwrap();
    assert_eq!(
        options.credential.as_ref().unwrap().mechanism,
        Some(AuthMechanism::ScramSha1)
    );
    assert_eq!(options.credential.unwrap().source.as_deref(), Some("admin"));
    assert_eq!(options.repl_set_name.as_deref(), Some("fixture-rs"));
    assert_eq!(options.direct_connection, Some(false));
    assert_eq!(options.retry_writes, Some(false));
    assert_eq!(options.retry_reads, Some(false));
    assert_eq!(options.max_pool_size, Some(1));
    assert_eq!(options.min_pool_size, Some(0));
    assert_eq!(options.max_connecting, Some(1));
    assert!(matches!(
        options.selection_criteria,
        Some(mongodb::options::SelectionCriteria::ReadPreference(
            ReadPreference::SecondaryPreferred { .. }
        ))
    ));
}
fn config() -> ConnectionConfig {
    ConnectionConfig {
        id: "mongo-options-fixture".into(),
        name: "Local TLS replica set".into(),
        engine: Engine::Mongo,
        host: "127.0.0.1".into(),
        port: std::env::var("CODISSEU_MONGO_FIXTURE_PORT")
            .unwrap()
            .parse()
            .unwrap(),
        database: "codisseu_fixture".into(),
        auth_database: "admin".into(),
        username: "fixture_reader".into(),
        ca_file: std::env::var("CODISSEU_MONGO_FIXTURE_CA").unwrap(),
        timeout_secs: 4,
        save_credentials: false,
        mongo: MongoOptions {
            replica_set: "codisseu-rs".into(),
            authentication: MongoAuthentication::ScramSha1,
            read_preference: MongoReadPreference::SecondaryPreferred,
            direct_connection: false,
        },
        ..Default::default()
    }
}
async fn sockets(observers: &[Client]) -> usize {
    let mut count = 0;
    for observer in observers {
        let mut cursor = observer
            .database("admin")
            .aggregate(vec![
                doc! {"$currentOp":{"allUsers":true,"idleConnections":true}},
                doc! {"$match":{"clientMetadata.application.name":"Codisseu"}},
                doc! {"$count":"connections"},
            ])
            .await
            .unwrap();
        if let Some(document) = cursor.try_next().await.unwrap() {
            count += document.get_i32("connections").unwrap_or(0) as usize;
        }
    }
    count
}
async fn empty(observers: &[Client]) {
    let until = Instant::now() + Duration::from_secs(8);
    while sockets(observers).await != 0 {
        assert!(Instant::now() < until, "fixture sockets not closed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
#[tokio::test]
#[ignore = "owned TLS authenticated two-node Mongo replica set; CODISSEU_MONGO_FIXTURE_PORT required"]
async fn real_mongo_tls_scram_replica_secondary_and_client_shutdown() {
    let config = config();
    let mut observers = vec![];
    for key in [
        "CODISSEU_MONGO_FIXTURE_PORT",
        "CODISSEU_MONGO_FIXTURE_SECONDARY_PORT",
    ] {
        let port = std::env::var(key).unwrap().parse().unwrap();
        let mut options = mongo_options(&config, "fixture-password").unwrap();
        options.hosts = vec![ServerAddress::Tcp {
            host: "127.0.0.1".into(),
            port: Some(port),
        }];
        options.app_name = Some("fixture-observer".into());
        options.direct_connection = Some(true);
        let credential = options.credential.as_mut().unwrap();
        credential.username = Some("fixture_admin".into());
        observers.push(Client::with_options(options).unwrap());
    }
    eprintln!("fixture: initial sockets");
    empty(&observers).await;
    eprintln!("fixture: connect/read");
    let mut session = NoSqlSession::connect(&config, "fixture-password")
        .await
        .unwrap();
    let temporary_ca = match &session {
        NoSqlSession::Mongo(mongo) => mongo._ca_file.as_ref().unwrap().path().to_path_buf(),
        _ => unreachable!(),
    };
    assert!(temporary_ca.is_file());
    let mut peak = 0;
    for _ in 0..20 {
        let result = session
            .execute(
                r#"{"collection":"users","filter":{"_id":1},"limit":1}"#,
                &config,
            )
            .await
            .unwrap();
        assert_eq!(result.rows.len(), 1);
        peak = peak.max(sockets(&observers).await);
    }
    eprintln!("fixture: sampled peak {peak}");
    assert!(
        peak > 0 && peak <= 6,
        "unexpected fixture pool/monitor sockets: {peak}"
    );
    // Require a secondary, rather than allowing fallback to the primary.
    session.close().await;
    assert!(!temporary_ca.exists());
    empty(&observers).await;
    let mut secondary = config.clone();
    secondary.mongo.read_preference = MongoReadPreference::Secondary;
    let mut session = NoSqlSession::connect(&secondary, "fixture-password")
        .await
        .unwrap();
    assert_eq!(
        session
            .execute(r#"{"collection":"users","limit":1}"#, &secondary)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );
    session.close().await;
    empty(&observers).await;
    for failure in ["password", "mechanism", "set", "ca"] {
        eprintln!("fixture: rejection {failure}");
        let mut invalid = config.clone();
        let mut password = "fixture-password";
        match failure {
            "password" => password = "wrong-password",
            "mechanism" => invalid.mongo.authentication = MongoAuthentication::ScramSha256,
            "set" => invalid.mongo.replica_set = "wrong-set".into(),
            "ca" => invalid.ca_file = std::env::var("CODISSEU_MONGO_FIXTURE_WRONG_CA").unwrap(),
            _ => unreachable!(),
        }
        assert!(
            NoSqlSession::connect(&invalid, password).await.is_err(),
            "fixture {failure} was accepted"
        );
        empty(&observers).await;
    }
    eprintln!("fixture: service test connection");
    // Test Connection uses the real service and must release its candidate.
    let temp = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(temp.path().join("profiles.json"));
    loop {
        service.poll();
        if !service.is_busy() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    service
        .dispatch(RemoteCommand::Connect {
            config: config.clone(),
            password: "fixture-password".into(),
            test: true,
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(30);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < until);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(service.error.is_none(), "{:?}", service.error);
    assert!(service.active.is_none());
    empty(&observers).await;
    drop(service);
    if let Ok(out) = std::env::var("CODISSEU_MONGO_FIXTURE_REPORT") {
        std::fs::write(out,serde_json::to_vec_pretty(&json!({"nodes":2,"tls_verified":true,"scram_sha1":true,"secondary_read":true,"repeated_reads":20,"sampled_peak_codisseu_sockets":peak,"includes_monitors":true,"application_pool_limit_per_server":1,"after_close":0,"after_test_connection":0,"rejected":["wrong password","unsupported user mechanism","wrong replica set","untrusted CA"],"synthetic_credentials_only":true})).unwrap()).unwrap();
    }
    for observer in observers {
        observer.shutdown().await;
    }
}

// Explicitly opted-in diagnosis only. Never runs the service's catalog path.
#[tokio::test]
#[ignore = "explicit human authorization and personal source paths required; authentication and ping only"]
async fn authorized_example_company_mongo_authentication_ping_only() {
    assert_eq!(
        std::env::var("CODISSEU_MONGO_REMOTE_PING_AUTHORIZED").as_deref(),
        Ok("1")
    );
    let ca = std::env::var("CODISSEU_MONGO_REMOTE_CA").unwrap();
    assert!(
        std::path::Path::new(&ca).is_file(),
        "CA público verificado ausente"
    );
    let mut reports = Vec::new();
    for environment in ["prod", "qa"] {
        let loaded = load_remote_ping_config(environment, &ca).await;
        let start = Instant::now();
        let outcome = match loaded {
            Ok((config, password)) => {
                assert!(
                    config.tls
                        && !config.host.contains('@')
                        && config.host.ends_with(".docdb.amazonaws.com")
                );
                match NoSqlSession::connect(&config, &password).await {
                    Ok(session) => {
                        session.close().await;
                        "connected"
                    }
                    Err(error) => {
                        let text = error.to_ascii_lowercase();
                        if text.contains("authentication") || text.contains("authenticate") {
                            "authentication_failed"
                        } else if text.contains("certificate")
                            || text.contains("tls")
                            || text.contains("ssl")
                        {
                            "tls_failed"
                        } else if text.contains("dns") || text.contains("resolve") {
                            "dns_failed"
                        } else if text.contains("timeout")
                            || text.contains("tempo limite")
                            || text.contains("server selection")
                        {
                            "network_or_selection_timeout"
                        } else {
                            "connection_failed"
                        }
                    }
                }
            }
            Err(reason) => reason,
        };
        reports.push(json!({"environment":environment,"outcome":outcome,"elapsed_ms":start.elapsed().as_millis(),"operation":"authentication and ping only","collections_queried":0,"document_commands":0,"writes":0,"tls_hostname_verified":true,"ca":"audited official AWS global bundle already in repository","credentials_saved":false}));
        eprintln!("Mongo {environment}: {outcome}");
    }
    let all_connected = reports.iter().all(|r| r["outcome"] == "connected");
    let out = std::env::var("CODISSEU_MONGO_REMOTE_REPORT").unwrap();
    std::fs::write(out, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    assert!(
        all_connected,
        "Um ou mais ambientes não conectaram; relatório contém apenas a classe do erro, sem segredos"
    );
}
async fn load_remote_ping_config(
    environment: &str,
    ca: &str,
) -> Result<(ConnectionConfig, String), &'static str> {
    let (mut config, password) = if environment == "prod" {
        let file =
            std::env::var("CODISSEU_MONGO_REMOTE_PROD_SOURCE").map_err(|_| "source_missing")?;
        let text = std::fs::read_to_string(file).map_err(|_| "source_unreadable")?;
        // The supplied URI wraps in the middle of secondaryPreferred.
        let joined = text.lines().map(str::trim).collect::<String>();
        let start = joined.find("mongodb://").ok_or("uri_missing")?;
        let uri = joined[start..]
            .split(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '`' | '<' | '>'))
            .next()
            .ok_or("uri_missing")?;
        let parsed = ClientOptions::parse(uri).await.map_err(|_| "uri_invalid")?;
        let [ServerAddress::Tcp { host, port }] = parsed.hosts.as_slice() else {
            return Err("unsupported_seeds");
        };
        let credential = parsed.credential.ok_or("credentials_missing")?;
        let authentication = match credential.mechanism {
            Some(AuthMechanism::ScramSha1) => MongoAuthentication::ScramSha1,
            Some(AuthMechanism::ScramSha256) => MongoAuthentication::ScramSha256,
            None => MongoAuthentication::Automatic,
            _ => return Err("unsupported_authentication"),
        };
        let preference = match parsed.selection_criteria {
            Some(mongodb::options::SelectionCriteria::ReadPreference(
                ReadPreference::SecondaryPreferred { .. },
            )) => MongoReadPreference::SecondaryPreferred,
            _ => return Err("unexpected_read_preference"),
        };
        let password = credential.password.ok_or("password_missing")?;
        (
            ConnectionConfig {
                id: "remote-ping-prod-ephemeral".into(),
                name: "Authorized production ping".into(),
                engine: Engine::Mongo,
                host: host.clone(),
                port: port.unwrap_or(27017),
                database: parsed.default_database.unwrap_or_else(|| "admin".into()),
                username: credential.username.ok_or("username_missing")?,
                auth_database: credential.source.unwrap_or_else(|| "admin".into()),
                tls: matches!(parsed.tls, Some(Tls::Enabled(_))),
                mongo: MongoOptions {
                    replica_set: parsed.repl_set_name.unwrap_or_default(),
                    authentication,
                    read_preference: preference,
                    direct_connection: false,
                },
                ..Default::default()
            },
            password,
        )
    } else {
        let file =
            std::env::var("CODISSEU_MONGO_REMOTE_QA_SOURCE").map_err(|_| "source_missing")?;
        let text = std::fs::read_to_string(file).map_err(|_| "source_unreadable")?;
        let values: std::collections::HashMap<_, _> = text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with('#') {
                    return None;
                }
                let (key, value) = line.split_once('=')?;
                let value = value.trim();
                let value = if (value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };
                Some((key.trim(), value))
            })
            .collect();
        let get = |key| values.get(key).copied().ok_or("env_field_missing");
        if get("MONGO_AUTH_MECHANISM")? != "SCRAM-SHA-1"
            || get("MONGO_READ_PREFERENCE")? != "secondaryPreferred"
            || get("MONGO_RETRY_WRITES")? != "false"
        {
            return Err("unexpected_env_options");
        }
        (
            ConnectionConfig {
                id: "remote-ping-qa-ephemeral".into(),
                name: "Authorized QA ping".into(),
                engine: Engine::Mongo,
                host: get("MONGO_HOST")?.into(),
                port: get("MONGO_PORT")?.parse().map_err(|_| "port_invalid")?,
                database: get("MONGO_DATABASE")?.into(),
                username: get("MONGO_USERNAME")?.into(),
                auth_database: get("MONGO_AUTH_SOURCE")?.into(),
                tls: get("MONGO_TLS")? == "true",
                mongo: MongoOptions {
                    replica_set: get("MONGO_REPLICA_SET")?.into(),
                    authentication: MongoAuthentication::ScramSha1,
                    read_preference: MongoReadPreference::SecondaryPreferred,
                    direct_connection: false,
                },
                ..Default::default()
            },
            get("MONGO_PASSWORD")?.to_owned(),
        )
    };
    config.ca_file = ca.into();
    config.timeout_secs = 10;
    config.save_credentials = false;
    config.validate().map_err(|_| "configuration_invalid")?;
    Ok((config, password))
}

#[tokio::test]
#[ignore = "explicit user request to register QA and production profiles; saves credentials in OS vault, never connects"]
async fn authorized_example_company_register_profiles_without_connecting() {
    assert_eq!(
        std::env::var("CODISSEU_MONGO_REGISTER_AUTHORIZED").as_deref(),
        Ok("1")
    );
    let ca = std::env::var("CODISSEU_MONGO_REMOTE_CA").unwrap();
    let path = std::path::PathBuf::from(std::env::var("CODISSEU_MONGO_REGISTER_PROFILE").unwrap());
    assert_eq!(path.file_name().unwrap(), "session.datasources.json");
    let mut service = RemoteService::new(path.clone());
    async fn settle(service: &mut RemoteService) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            service.poll();
            if !service.is_busy() {
                break;
            }
            assert!(
                Instant::now() < until,
                "Cadastro/cofre não respondeu no prazo"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            service.error.is_none(),
            "Falha ao salvar cadastro/cofre; detalhes sanitizados na aplicação"
        );
    }
    settle(&mut service).await;
    let previous = service.profiles.clone();
    let mut registered = Vec::new();
    for environment in ["qa", "prod"] {
        let (mut config, password) = load_remote_ping_config(environment, &ca)
            .await
            .unwrap_or_else(|reason| panic!("Fonte não carregada: {reason}"));
        config.id = format!("example_company-mongo-{environment}");
        config.name = format!(
            "Empresa Exemplo · MongoDB {}",
            environment.to_ascii_uppercase()
        );
        config.save_credentials = true;
        // Known AWS endpoints use the embedded global CA, not a build-tree path.
        config.ca_file.clear();
        config.ca_pem = Default::default();
        assert!(
            config.automatic_rds_ca(),
            "CA automática deve se limitar ao endpoint AWS validado"
        );
        service
            .dispatch(RemoteCommand::SaveRemembered {
                config: config.clone(),
                password: password.clone(),
            })
            .expect("Cadastro aceito");
        settle(&mut service).await;
        assert!(service.active.is_none(), "Salvar não deve conectar");
        let store = crate::remote_credentials::CredentialStore::new();
        let saved = store
            .load(&config)
            .await
            .expect("Ler somente a entrada recém-salva no cofre")
            .expect("Entrada persistida no cofre");
        assert!(
            saved.expose() == password,
            "Senha do cadastro não persistida corretamente"
        );
        registered.push(config);
    }
    for old in &previous {
        if !registered.iter().any(|c| c.id == old.id) {
            assert!(
                service.profiles.contains(old),
                "Cadastro existente alterado"
            );
        }
    }
    drop(service);
    let mut restored = RemoteService::new(path);
    settle(&mut restored).await;
    for config in &registered {
        assert!(restored.profiles.contains(config));
    }
    assert!(restored.active.is_none());
    let report = json!({"saved_profiles":["Empresa Exemplo · MongoDB QA","Empresa Exemplo · MongoDB PROD"],"credential_storage":"OS vault; read back exact entries verified","existing_profiles_preserved":true,"restore_verified":true,"auto_connect":false,"certificate_mode":"automatic official AWS CA embedded in the app","connections_opened":0,"writes_to_corporate_database":0,"production_database":"admin; original URI did not specify a database"});
    std::fs::write(
        std::env::var("CODISSEU_MONGO_REGISTER_REPORT").unwrap(),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
#[ignore = "explicit user request: migrate saved public CA files in normal profile; no connections or credential access"]
async fn authorized_normal_profile_import_existing_ca_files_without_connecting() {
    assert_eq!(
        std::env::var("CODISSEU_CA_MIGRATION_AUTHORIZED").as_deref(),
        Ok("1")
    );
    let path = std::path::PathBuf::from(std::env::var("CODISSEU_MONGO_REGISTER_PROFILE").unwrap());
    assert_eq!(path.file_name().unwrap(), "session.datasources.json");
    let mut service = RemoteService::new(path.clone());
    async fn idle(service: &mut RemoteService) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            service.poll();
            if !service.is_busy() {
                break;
            }
            assert!(Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            service.error.is_none(),
            "Falha ao importar certificado público existente"
        );
    }
    idle(&mut service).await;
    let before = service.profiles.clone();
    let mut migrated = 0;
    for config in before.iter().filter(|c| !c.ca_file.is_empty()) {
        let mut normalized = config.clone();
        crate::tls_roots::prepare_saved_certificate(&mut normalized).unwrap();
        assert!(!normalized.ca_pem.is_empty() && normalized.ca_file.is_empty());
        service.dispatch(RemoteCommand::Save(normalized)).unwrap();
        idle(&mut service).await;
        migrated += 1;
    }
    assert!(service.active.is_none());
    drop(service);
    let mut restored = RemoteService::new(path);
    idle(&mut restored).await;
    assert!(restored.profiles.len() == before.len());
    for old in before {
        let new = restored.profiles.iter().find(|c| c.id == old.id).unwrap();
        let mut expected = old.clone();
        if !old.ca_file.is_empty() {
            expected.ca_file.clear();
            expected.ca_pem = new.ca_pem.clone();
        }
        assert!(
            *new == expected,
            "A migração não pode alterar outras opções do cadastro"
        );
    }
    std::fs::write(std::env::var("CODISSEU_CA_MIGRATION_REPORT").unwrap(),serde_json::to_vec_pretty(&json!({"migrated_public_ca_files":migrated,"profiles_preserved":true,"restore_verified":true,"connections_opened":0,"credential_access":false})).unwrap()).unwrap();
}

#[test]
fn literal_shell_preserves_user_pipeline_order_database_and_int64() {
    let query = include_str!("../../../../fixtures/databases/mongo/aggregate-cte.js");
    let read = mongo_request(query, 100).unwrap();
    assert_eq!(read.database.as_deref(), Some("docdb-ics-faturamento"));
    assert_eq!(read.collection, "documento-cobranca-cte");
    let stages = read.pipeline.unwrap();
    assert_eq!(stages.len(), 5);
    let sort = stages[1].get_document("$sort").unwrap();
    assert_eq!(
        sort.keys().map(String::as_str).collect::<Vec<_>>(),
        ["dataCriacao", "_id"]
    );
    let ids = stages[0]
        .get_document("$match")
        .unwrap()
        .get_document("processor.cte.fatCteId")
        .unwrap()
        .get_array("$in")
        .unwrap();
    assert_eq!(ids.len(), 56);
    assert_eq!(
        stages[2]
            .get_document("$group")
            .unwrap()
            .get_document("maisRecente")
            .unwrap()
            .get_str("$first")
            .unwrap(),
        "$$ROOT"
    );
    let read = mongo_request(r#"/* 😀 */ db.getSiblingDB('á-base').getCollection('a-b').find({n:9007199254740993, text:'á \' 😀 "'}, {n:1,}).sort({z:-1,a:1}).limit(2); // fim"#, 10).unwrap();
    assert_eq!(read.filter.get_i64("n").unwrap(), 9007199254740993);
    assert_eq!(read.filter.get_str("text").unwrap(), "á ' 😀 \"");
    assert_eq!(read.limit, Some(2));
    assert_eq!(
        read.sort
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["z", "a"]
    );
}

#[test]
fn shell_and_json_reject_writes_code_ambiguous_or_unbounded_input() {
    for query in [
        "db.x.aggregate([{$out:'copy'}])",
        "db.x.aggregate([{$merge:{into:'copy'}}])",
        "db.x.aggregate([{$facet:{x:[{$out:'copy'}]}}])",
        "db.x.aggregate([{$project:{x:{$function:{body:'return 1',args:[],lang:'js'}}}}])",
        "db.x.find({$where:'true'})",
        "db.x.find({$expr:{$accumulator:{}}})",
        "db.x.deleteMany({})",
        "db.x.insertOne({x:1})",
        "db.x.aggregate([{$match:{}}], {allowDiskUse:true})",
        "db.x.aggregate([{}])",
        "db.x.aggregate([{$match:{},$sort:{x:1}}])",
        "db.x.aggregate([1])",
        "db.x.aggregate({})",
        "db.x.find({x: function(){return 1}})",
        "db.x.find({x:process.env})",
        "db.x.find({x:ObjectId('123')})",
        "db.x.find({x:1+2})",
        "db.x.find({x:9223372036854775808})",
        "db.x.find({x:1,x:2})",
        "db.x.find({}).limit(0)",
        "db.x.find({}).limit(101)",
        "db.x.find({}).limit(-1)",
        "db.x.find({}).sort({a:1}).sort({b:1})",
        "db.x.find({}); db.x.find({})",
        "db.getSiblingDB('../bad').x.find()",
        "db.x.aggregate([]).limit(1)",
        "db.system.users.find({})",
        r#"{"collection":"x","pipeline":[{"$out":"copy"}]}"#,
        r#"{"collection":"x","pipeline":[],"filter":{"x":1}}"#,
    ] {
        assert!(mongo_request(query, 100).is_err(), "{query}");
    }
    assert!(mongo_request(&format!("db.x.find({{x:{}}})", "[".repeat(130)), 100).is_err());
}
