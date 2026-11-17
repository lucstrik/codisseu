use codisseu_db::{
    remote::{ConnectionConfig, Engine, RemoteCommand, RemoteService},
    transfer::{ImportApproval, NoSqlImportRequest, TransferRequest},
    transfer_nosql::*,
};

fn text(value: &str) -> TransferBytes {
    TransferBytes::Utf8(value.into())
}
fn redis(records: Vec<NoSqlRecord>) -> NoSqlTransfer {
    NoSqlTransfer {
        version: 1,
        engine: Engine::Redis,
        source_truncated: false,
        records,
    }
}
fn string(key: &str, value: TransferBytes, expiry: TransferExpiry) -> NoSqlRecord {
    NoSqlRecord::Redis {
        key: text(key),
        value: RedisValue::String(value),
        expiry,
    }
}

#[test]
fn binary_strings_and_all_redis_types_round_trip_without_numeric_coercion() {
    let data = redis(vec![
        string(
            "string",
            TransferBytes::Hex("00FF0D0A".into()),
            TransferExpiry::Never,
        ),
        NoSqlRecord::Redis {
            key: text("list"),
            value: RedisValue::List(vec![
                text("same"),
                text("same"),
                TransferBytes::Hex("FF".into()),
            ]),
            expiry: TransferExpiry::AtUnixMs(4_000_000_000_000),
        },
        NoSqlRecord::Redis {
            key: text("hash"),
            value: RedisValue::Hash(vec![(
                TransferBytes::Hex("FF00".into()),
                text("9223372036854775807"),
            )]),
            expiry: TransferExpiry::Never,
        },
        NoSqlRecord::Redis {
            key: text("set"),
            value: RedisValue::Set(vec![text("a"), text("b")]),
            expiry: TransferExpiry::Never,
        },
        NoSqlRecord::Redis {
            key: text("zset"),
            value: RedisValue::Zset(vec![(text("member"), "1.0000000000000002".into())]),
            expiry: TransferExpiry::Never,
        },
    ]);
    let json = data.to_json().unwrap();
    assert_eq!(NoSqlTransfer::from_json(&json).unwrap(), data);
    let plan = prepare_import(
        &json,
        Engine::Redis,
        NoSqlImportTarget::RedisKeys,
        UnknownTtlPolicy::Reject,
    )
    .unwrap();
    assert_eq!(plan.len(), 5);
    assert_eq!(
        TransferBytes::Hex("00FF0D0A".into()).decode().unwrap(),
        [0, 255, 13, 10]
    );
}

#[test]
fn unknown_ttl_requires_an_explicit_policy_and_cannot_become_never_silently() {
    let json = redis(vec![string("key", text("value"), TransferExpiry::Unknown)])
        .to_json()
        .unwrap();
    assert!(
        prepare_import(
            &json,
            Engine::Redis,
            NoSqlImportTarget::RedisKeys,
            UnknownTtlPolicy::Reject
        )
        .unwrap_err()
        .contains("TTL desconhecido")
    );
    for policy in [
        UnknownTtlPolicy::NeverExpire,
        UnknownTtlPolicy::AfterSeconds(60),
    ] {
        assert!(prepare_import(&json, Engine::Redis, NoSqlImportTarget::RedisKeys, policy).is_ok());
    }
    assert!(
        prepare_import(
            &json,
            Engine::Redis,
            NoSqlImportTarget::RedisKeys,
            UnknownTtlPolicy::AfterSeconds(0)
        )
        .is_err()
    );
}

#[test]
fn duplicate_identities_invalid_hex_types_scores_and_limits_are_rejected_before_writes() {
    for data in [
        redis(vec![
            string("same", text("a"), TransferExpiry::Never),
            string("same", text("b"), TransferExpiry::Never),
        ]),
        redis(vec![string(
            "key",
            TransferBytes::Hex("0G".into()),
            TransferExpiry::Never,
        )]),
        redis(vec![string(
            "key",
            TransferBytes::Hex("F".into()),
            TransferExpiry::Never,
        )]),
        redis(vec![NoSqlRecord::Redis {
            key: text("set"),
            value: RedisValue::Set(vec![text("a"), text("a")]),
            expiry: TransferExpiry::Never,
        }]),
        redis(vec![NoSqlRecord::Redis {
            key: text("zset"),
            value: RedisValue::Zset(vec![(text("member"), "NaN".into())]),
            expiry: TransferExpiry::Never,
        }]),
        redis(vec![string(
            "huge",
            text(&"x".repeat(RECORD_BYTES)),
            TransferExpiry::Never,
        )]),
        redis(
            (0..501)
                .map(|n| string(&format!("key:{n}"), text("a"), TransferExpiry::Never))
                .collect(),
        ),
    ] {
        assert!(data.to_json().is_err(), "{data:?}");
    }
}

#[test]
fn mongodb_extended_json_preserves_id_i64_binary_decimal_and_date() {
    let data = NoSqlTransfer {
        version: 1,
        engine: Engine::Mongo,
        source_truncated: false,
        records: vec![NoSqlRecord::Mongo {
            document: serde_json::json!({"_id":{"$oid":"0123456789abcdef01234567"},"big":{"$numberLong":"9223372036854775807"},"decimal":{"$numberDecimal":"12345678901234567890.0123456789"},"date":{"$date":{"$numberLong":"1700000000000"}},"binary":{"$binary":{"base64":"AP8=","subType":"00"}}}),
        }],
    };
    let json = data.to_json().unwrap();
    assert_eq!(NoSqlTransfer::from_json(&json).unwrap(), data);
    assert!(
        prepare_import(
            &json,
            Engine::Mongo,
            NoSqlImportTarget::MongoCollection("target".into()),
            UnknownTtlPolicy::Reject
        )
        .is_ok()
    );
    assert!(
        prepare_import(
            &json,
            Engine::Redis,
            NoSqlImportTarget::RedisKeys,
            UnknownTtlPolicy::Reject
        )
        .is_err()
    );
    assert!(
        prepare_import(
            &json,
            Engine::Mongo,
            NoSqlImportTarget::MongoCollection("system.users".into()),
            UnknownTtlPolicy::Reject
        )
        .is_err()
    );
}

#[test]
fn mongodb_requires_an_explicit_non_null_id_and_memcached_rejects_protocol_keys() {
    for document in [
        serde_json::json!({"name":"no id"}),
        serde_json::json!({"_id":null}),
        serde_json::json!({"_id":[1,2]}),
    ] {
        let data = NoSqlTransfer {
            version: 1,
            engine: Engine::Mongo,
            source_truncated: false,
            records: vec![NoSqlRecord::Mongo { document }],
        };
        assert!(data.to_json().is_err());
    }
    for key in ["", "bad key", "bad\r\nflush_all"] {
        let data = NoSqlTransfer {
            version: 1,
            engine: Engine::Memcached,
            source_truncated: false,
            records: vec![NoSqlRecord::Memcached {
                key: key.into(),
                value: text("x"),
                flags: u32::MAX,
                expiry: TransferExpiry::Never,
            }],
        };
        assert!(data.to_json().is_err());
    }
}

#[test]
fn unrecognized_envelopes_and_injected_commands_do_not_parse_as_transfer() {
    for json in [
        r#"{"version":1,"engine":"Redis","source_truncated":false,"records":[],"command":"FLUSHALL"}"#,
        r#"{"version":2,"engine":"Redis","source_truncated":false,"records":[]}"#,
    ] {
        assert!(NoSqlTransfer::from_json(json).is_err());
    }
    let json = redis(vec![]).to_json().unwrap();
    assert!(
        prepare_import(
            &json,
            Engine::Redis,
            NoSqlImportTarget::RedisKeys,
            UnknownTtlPolicy::Reject
        )
        .is_err()
    );
}

#[test]
fn mongo_arrays_and_ndjson_are_accepted_but_invalid_envelopes_never_fall_back() {
    for input in [
        r#"{"_id":1,"name":"single"}"#,
        r#"[{"_id":1},{"_id":2,"big":{"$numberLong":"9223372036854775807"}}]"#,
        "{\"_id\":1}\n{\"_id\":2}\n",
    ] {
        let plan = prepare_import(
            input,
            Engine::Mongo,
            NoSqlImportTarget::MongoCollection("target".into()),
            UnknownTtlPolicy::Reject,
        )
        .unwrap();
        assert!(!plan.is_empty());
    }
    for input in [
        r#"{"version":2,"engine":"Mongo","records":[],"_id":"not_a_document"}"#,
        r#"{"version":1,"engine":"Redis","source_truncated":false,"records":[]}"#,
        r#"[{"name":"missing id"}]"#,
        "{\"_id\":1}\n{\"version\":2,\"_id\":2}\n",
    ] {
        assert!(
            prepare_import(
                input,
                Engine::Mongo,
                NoSqlImportTarget::MongoCollection("target".into()),
                UnknownTtlPolicy::Reject
            )
            .is_err(),
            "{input}"
        );
    }
}

#[test]
#[ignore = "requires disposable Redis; CODISSEU_TEST_REDIS_PORT"]
fn real_worker_preview_approval_export_generation_and_new_readonly_guard() {
    use std::time::{Duration, Instant};
    fn settle(service: &mut RemoteService) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < deadline, "{}", service.status);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn bound<T>(service: &RemoteService, payload: T) -> TransferRequest<T> {
        TransferRequest {
            datasource_id: service.active.as_ref().unwrap().id.clone(),
            session_generation: service.session_generation,
            payload,
        }
    }
    let port: u16 = std::env::var("CODISSEU_TEST_REDIS_PORT")
        .expect("Use only a disposable local Redis fixture")
        .parse()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut service = RemoteService::new(directory.path().join("profiles.json"));
    settle(&mut service);
    let config = ConnectionConfig {
        id: "worker-transfer-fixture".into(),
        name: "Disposable transfer fixture".into(),
        engine: Engine::Redis,
        host: "127.0.0.1".into(),
        port,
        tls: false,
        read_only: false,
        save_credentials: false,
        timeout_secs: 10,
        ..Default::default()
    };
    service
        .dispatch(RemoteCommand::Connect {
            config: config.clone(),
            password: String::new(),
            test: false,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.active.is_some(), "{:?}", service.error);
    let name = format!(
        "codisseu:worker:{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let json = redis(vec![string(
        &name,
        TransferBytes::Hex("00FF".into()),
        TransferExpiry::Never,
    )])
    .to_json()
    .unwrap();
    let request = bound(
        &service,
        NoSqlImportRequest {
            json,
            target: NoSqlImportTarget::RedisKeys,
            ttl_policy: UnknownTtlPolicy::Reject,
        },
    );
    service
        .dispatch(RemoteCommand::PreviewNoSqlImport(request))
        .unwrap();
    settle(&mut service);
    assert!(service.error.is_none(), "{:?}", service.error);
    let preview = service.import_preview.as_ref().unwrap().clone();
    assert_eq!(preview.rows, 1);
    let client = redis::Client::open(format!("redis://127.0.0.1:{port}/0")).unwrap();
    let mut inspector = client
        .get_connection_with_timeout(Duration::from_secs(3))
        .unwrap();
    inspector
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    assert!(
        !redis::cmd("EXISTS")
            .arg(&name)
            .query::<bool>(&mut inspector)
            .unwrap(),
        "preview must not create keys"
    );
    let mut stale = bound(
        &service,
        ImportApproval {
            preview_id: preview.preview_id,
        },
    );
    stale.session_generation = stale.session_generation.saturating_sub(1);
    assert!(
        service
            .dispatch(RemoteCommand::ApplyNoSqlImport(stale))
            .is_err()
    );
    assert!(
        !redis::cmd("EXISTS")
            .arg(&name)
            .query::<bool>(&mut inspector)
            .unwrap()
    );
    let approval = bound(
        &service,
        ImportApproval {
            preview_id: preview.preview_id,
        },
    );
    service
        .dispatch(RemoteCommand::ApplyNoSqlImport(approval))
        .unwrap();
    settle(&mut service);
    let report = service.import_report.as_ref().unwrap();
    assert_eq!(report.applied, 1);
    assert!(!report.atomic);
    assert!(!report.transaction_pending);
    assert_eq!(report.unknown, 0);
    let request = bound(
        &service,
        NoSqlExportRequest::Keys {
            keys: vec![text(&name)],
        },
    );
    service
        .dispatch(RemoteCommand::ExportNoSql(request))
        .unwrap();
    settle(&mut service);
    let exported = NoSqlTransfer::from_json(service.native_export.as_deref().unwrap()).unwrap();
    assert_eq!(exported.records.len(), 1);
    let NoSqlRecord::Redis {
        value: RedisValue::String(value),
        expiry,
        ..
    } = &exported.records[0]
    else {
        panic!("type changed")
    };
    assert_eq!(value.decode().unwrap(), [0, 255]);
    assert_eq!(*expiry, TransferExpiry::Never);
    let second = format!("{name}:readonly");
    let request = bound(
        &service,
        NoSqlImportRequest {
            json: redis(vec![string(
                &second,
                text("must not exist"),
                TransferExpiry::Never,
            )])
            .to_json()
            .unwrap(),
            target: NoSqlImportTarget::RedisKeys,
            ttl_policy: UnknownTtlPolicy::Reject,
        },
    );
    service
        .dispatch(RemoteCommand::PreviewNoSqlImport(request))
        .unwrap();
    settle(&mut service);
    let approval = bound(
        &service,
        ImportApproval {
            preview_id: service.import_preview.as_ref().unwrap().preview_id,
        },
    );
    let mut read_only = config;
    read_only.read_only = true;
    service.dispatch(RemoteCommand::Save(read_only)).unwrap();
    settle(&mut service);
    assert!(
        service
            .dispatch(RemoteCommand::ApplyNoSqlImport(approval))
            .unwrap_err()
            .contains("somente leitura")
    );
    assert!(
        !redis::cmd("EXISTS")
            .arg(&second)
            .query::<bool>(&mut inspector)
            .unwrap()
    );
    service.dispatch(RemoteCommand::Disconnect).unwrap();
    settle(&mut service);
    assert!(service.active.is_none());
}
