use super::*;
use codisseu_core::{
    DbCommand, Selection,
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand},
};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{fs, thread, time::Instant};
fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            h.step();
            break;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            h.state().service.status,
            h.state().service.remote_db.error
        );
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
}
fn harness(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
    app.show_project = false;
    app.show_activity = false;
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(200)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    settle(&mut h);
    (dir, h)
}
fn place(h: &mut Harness<'_, CodisseuApp>, needle: &str) {
    let d = h.state().service.active_document().unwrap();
    let (id, version) = (d.id, d.version);
    let at = d.text()[..d.text().find(needle).unwrap()].chars().count() + 1;
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(at),
        })
        .unwrap();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.step();
}
fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_QUERY_EXEC_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.ctx.request_repaint();
        h.step();
        h.step();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join(format!("{name}.png")))
            .unwrap();
    }
}
#[test]
fn sqlite_play_and_keyboard_use_the_caret_block_without_source_edits() {
    for scale in [1., 1.25] {
        let (dir, mut h) = harness(scale);
        let path = dir.path().join("query.sql");
        let source = "SELECT 'ação 😀; first' AS marker;\r\n\r\nSELECT\r\n  22 AS answer;\r\n";
        fs::write(&path, source).unwrap();
        let db = dir.path().join("own.sqlite");
        rusqlite::Connection::open(&db).unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::Database(DbCommand::Connect(db)))
            .unwrap();
        h.state_mut().database_ui.sqlite = true;
        settle(&mut h);
        place(&mut h, "22 AS");
        h.get_by_label("Executar consulta sob o cursor").click();
        settle(&mut h);
        assert_eq!(
            h.state().service.db.result.as_ref().unwrap().rows,
            vec![vec![rusqlite::types::Value::Integer(22)]]
        );
        place(&mut h, "first");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        settle(&mut h);
        assert_eq!(
            h.state().service.db.result.as_ref().unwrap().rows,
            vec![vec![rusqlite::types::Value::Text("ação 😀; first".into())]]
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        assert_eq!(fs::read_to_string(path).unwrap(), source);
        capture(&mut h, &format!("sqlite-play-{scale}"));
        // Root context action uses the actual SQLite worker.
        let name = "own.sqlite";
        h.get_by_label(name).click_secondary();
        h.run();
        let choices = h.get_all_by_label("Desconectar").collect::<Vec<_>>();
        assert_eq!(
            choices.len(),
            2,
            "toolbar plus the new SQLite context action"
        );
        choices.last().unwrap().click();
        settle(&mut h);
        assert!(!h.state().service.db.connected);
    }
}
#[test]
fn disconnect_menu_is_scoped_and_available_for_every_remote_engine() {
    for engine in Engine::ALL {
        let (_dir, mut h) = harness(1.);
        let active = ConnectionConfig {
            id: "active".into(),
            name: format!("Active {}", engine.label()),
            engine,
            save_credentials: false,
            ..Default::default()
        };
        let other = ConnectionConfig {
            id: "other".into(),
            name: "Other".into(),
            ..active.clone()
        };
        // Presentation-only: this loop proves menu state/dispatch, not seven live services.
        h.state_mut().service.remote_db.profiles = vec![active.clone(), other.clone()];
        h.state_mut().service.remote_db.active = Some(active.clone());
        h.state_mut().service.remote_db.namespaces_loaded = true;
        h.run();
        h.get_by_label(&other.name).click_secondary();
        h.run();
        assert!(
            h.get_by_role_and_label(Role::Button, "Desconectar")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(Key::Escape);
        h.run();
        h.get_by_label(&active.name).click_secondary();
        h.run();
        assert!(
            !h.get_by_role_and_label(Role::Button, "Desconectar")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(Key::Escape);
        h.run();
        h.state_mut()
            .database_ui
            .staged
            .insert((0, 0), Cell::Text("pending".into()));
        h.get_by_label(&active.name).click_secondary();
        h.run();
        assert!(
            h.get_by_role_and_label(Role::Button, "Desconectar")
                .accesskit_node()
                .is_disabled()
        );
        h.key_press(Key::Escape);
        h.run();
        assert_eq!(
            h.state().service.remote_db.active.as_ref().unwrap().id,
            "active"
        );
    }
}
#[test]
fn plain_json_without_a_database_context_keeps_normal_editor_behavior() {
    let (dir, mut h) = harness(1.);
    let path = dir.path().join("ordinary.json");
    let source = "{\"name\":\"ordinary file\"}";
    fs::write(&path, source).unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path))
        .unwrap();
    settle(&mut h);
    assert!(h.query_by_label("Executar consulta sob o cursor").is_none());
    h.get_by_role(Role::MultilineTextInput).focus();
    h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    h.step();
    h.step();
    assert_ne!(h.state().service.active_document().unwrap().text(), source);
    assert!(h.state().service.remote_db.result.is_none());
}
#[test]
#[ignore = "requires own loopback MySQL/Mongo fixtures, CODISSEU_QUERY_*_PORT"]
fn mysql_and_mongo_real_ctrl_enter_play_and_context_disconnect() {
    for engine in [Engine::Mysql, Engine::Mongo] {
        for scale in [1., 1.25] {
            let (_dir, mut h) = harness(scale);
            let mongo = engine == Engine::Mongo;
            let config = ConnectionConfig {
                id: format!("own-{}", engine.label()),
                name: format!("Fixture {}", engine.label()),
                engine,
                host: "127.0.0.1".into(),
                port: std::env::var(if mongo {
                    "CODISSEU_QUERY_MONGO_PORT"
                } else {
                    "CODISSEU_QUERY_MY_PORT"
                })
                .expect("fixture port required")
                .parse()
                .unwrap(),
                database: "codisseu_fixture".into(),
                username: if mongo { String::new() } else { "root".into() },
                tls: false,
                save_credentials: false,
                timeout_secs: 5,
                idle_secs: 600,
                ..Default::default()
            };
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config.clone())))
                .unwrap();
            settle(&mut h);
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
                    config: config.clone(),
                    password: if mongo {
                        String::new()
                    } else {
                        "codisseu_fixture".into()
                    },
                    test: false,
                }))
                .unwrap();
            settle(&mut h);
            assert!(
                h.state().service.remote_db.error.is_none(),
                "{:?}",
                h.state().service.remote_db.error
            );
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(
                    config.id.clone(),
                )))
                .unwrap();
            settle(&mut h);
            let path = h.state().service.active_document().unwrap().path.clone();
            let disk = fs::read(&path).unwrap();
            let (first, second) = if mongo {
                (
                    "{\r\n \"collection\":\"users\",\r\n \"filter\":{\"_id\":1}\r\n}",
                    "{\r\n \"collection\":\"users\",\r\n \"filter\":{\"_id\":2}\r\n}",
                )
            } else {
                ("SELECT\r\n 11 AS answer;", "SELECT\r\n 22 AS answer;")
            };
            let source = if mongo {
                format!("// ação 😀\r\n{first};\r\n\r\n{second}\r\n")
            } else {
                format!("-- ação 😀\r\n{first}\r\n\r\n{second}\r\n")
            };
            let d = h.state().service.active_document().unwrap();
            let (id, version) = (d.id, d.version);
            h.state_mut()
                .service
                .dispatch(Command::Edit {
                    id,
                    expected_version: version,
                    text: source.clone(),
                })
                .unwrap();
            settle(&mut h);
            place(&mut h, if mongo { "\"_id\":2" } else { "22 AS" });
            h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
            settle(&mut h);
            assert!(
                h.state().service.remote_db.error.is_none(),
                "{:?}",
                h.state().service.remote_db.error
            );
            let result = h.state().service.remote_db.result.as_ref().unwrap();
            assert_eq!(result.rows.len(), 1);
            if mongo {
                let Cell::Json(value) = &result.rows[0][1] else {
                    panic!("real document required")
                };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(value).unwrap()["label"],
                    "second"
                );
            } else {
                assert_eq!(result.rows[0][0], Cell::Number("22".into()));
            }
            assert_eq!(h.state().service.active_document().unwrap().text(), source);
            assert_eq!(fs::read(&path).unwrap(), disk);
            capture(&mut h, &format!("{}-keyboard-{scale}", engine.label()));
            place(&mut h, if mongo { "\"_id\":1" } else { "11 AS" });
            h.get_by_label("Executar consulta sob o cursor").click();
            settle(&mut h);
            assert!(
                h.state().service.remote_db.error.is_none(),
                "{:?}",
                h.state().service.remote_db.error
            );
            let result = h.state().service.remote_db.result.as_ref().unwrap();
            if mongo {
                let Cell::Json(value) = &result.rows[0][1] else {
                    panic!()
                };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(value).unwrap()["label"],
                    "first"
                );
            } else {
                assert_eq!(result.rows[0][0], Cell::Number("11".into()));
            }
            // The existing main Play uses the same query target too (formerly SQLite-only).
            place(&mut h, if mongo { "\"_id\":2" } else { "22 AS" });
            h.get_by_label(if mongo {
                "Executar consulta"
            } else {
                "Executar SQL"
            })
            .click();
            settle(&mut h);
            let result = h.state().service.remote_db.result.as_ref().unwrap();
            if mongo {
                let Cell::Json(value) = &result.rows[0][1] else {
                    panic!()
                };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(value).unwrap()["label"],
                    "second"
                );
            } else {
                assert_eq!(result.rows[0][0], Cell::Number("22".into()));
            }
            h.get_by_role_and_label(Role::Button, &config.name)
                .click_secondary();
            h.run();
            capture(
                &mut h,
                &format!("{}-disconnect-menu-{scale}", engine.label()),
            );
            h.get_by_role_and_label(Role::Button, "Desconectar").click();
            settle(&mut h);
            assert!(h.state().service.remote_db.active.is_none());
            assert!(
                h.state()
                    .service
                    .remote_db
                    .profiles
                    .iter()
                    .any(|c| c.id == config.id)
            );
            assert_eq!(
                h.state().service.remote_db.console_bindings.get(&path),
                Some(&config.id)
            );
            assert_eq!(h.state().service.active_document().unwrap().text(), source);
            assert_eq!(fs::read(path).unwrap(), disk);
        }
    }
}

fn set_query(h: &mut Harness<'_, CodisseuApp>, text: &str) {
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: text.into(),
        })
        .unwrap();
    settle(h);
}
fn assert_aggregation(h: &Harness<'_, CodisseuApp>) {
    assert!(
        h.state().service.remote_db.error.is_none(),
        "{:?}",
        h.state().service.remote_db.error
    );
    let r = h.state().service.remote_db.result.as_ref().unwrap();
    assert_eq!(r.rows.len(), 56);
    assert!(!r.truncated);
    let mut previous = 0;
    for row in &r.rows {
        assert_eq!(row[0], Cell::Null, "$project removes _id");
        let Cell::Json(text) = &row[1] else {
            panic!("document expected");
        };
        let d: serde_json::Value = serde_json::from_str(text).unwrap();
        let id: i64 = d["fatCteId"]["$numberInt"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(id > previous, "final sort must be ascending");
        previous = id;
        assert_eq!(d["quantidadeDocumentos"]["$numberInt"], "3");
        assert_eq!(d["remetenteId"]["$numberInt"], "42");
        assert_eq!(d["awb"], format!("fixture-{id}-latest"));
        assert_eq!(d["statusEncomenda"], "latest");
        assert_eq!(d["documentoStatus"], "authorized");
        assert_eq!(d["dataCriacao"]["$date"]["$numberLong"], "1735776000000");
        assert_eq!(d["dataAutorizacao"], d["dataAutorizacaoCte"]);
    }
    assert_eq!(previous, 1056);
}

#[test]
#[ignore = "requires own Mongo fixture, CODISSEU_QUERY_MONGO_PORT"]
fn mongo_shell_aggregate_real_keyboard_play_errors_and_limits() {
    for scale in [1., 1.25] {
        let (_dir, mut h) = harness(scale);
        let mut config = ConnectionConfig {
            id: "aggregate-fixture".into(),
            name: "Mongo aggregate fixture".into(),
            engine: Engine::Mongo,
            host: "127.0.0.1".into(),
            port: std::env::var("CODISSEU_QUERY_MONGO_PORT")
                .unwrap()
                .parse()
                .unwrap(),
            database: "codisseu_fixture".into(),
            username: String::new(),
            tls: false,
            save_credentials: false,
            timeout_secs: 5,
            idle_secs: 600,
            row_limit: 100,
            ..Default::default()
        };
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config.clone())))
            .unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
                config: config.clone(),
                password: String::new(),
                test: false,
            }))
            .unwrap();
        settle(&mut h);
        assert!(
            h.state().service.remote_db.error.is_none(),
            "{:?}",
            h.state().service.remote_db.error
        );
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(
                config.id.clone(),
            )))
            .unwrap();
        settle(&mut h);
        let path = h.state().service.active_document().unwrap().path.clone();
        let disk = fs::read(&path).unwrap();
        let aggregate = include_str!("../../../fixtures/databases/mongo/aggregate-cte.js");
        let second = "db.getCollection('users').find({_id:2});";
        let source = format!("// ação 😀\r\n{aggregate}\r\n{second}");
        set_query(&mut h, &source);
        place(&mut h, "quantidadeDocumentos: { $sum");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        settle(&mut h);
        assert_aggregation(&h);
        assert_eq!(
            h.state()
                .service
                .remote_db
                .active
                .as_ref()
                .unwrap()
                .database,
            "codisseu_fixture"
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        capture(&mut h, &format!("aggregate-keyboard-{scale}"));
        place(&mut h, "maisRecente: { $first");
        h.get_by_label("Executar consulta sob o cursor").click();
        settle(&mut h);
        assert_aggregation(&h);
        place(&mut h, "fatCteId: 1");
        h.get_by_label("Executar consulta").click();
        settle(&mut h);
        assert_aggregation(&h);
        // Other call and selection use the connection DB, never the explicit sibling DB.
        let d = h.state().service.active_document().unwrap();
        let (id, expected_version) = (d.id, d.version);
        let start = source[..source.find(second).unwrap()].chars().count();
        h.state_mut()
            .service
            .dispatch(Command::SetSelection {
                id,
                expected_version,
                selection: Selection {
                    anchor: start,
                    head: start + second.chars().count(),
                },
            })
            .unwrap();
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        settle(&mut h);
        assert!(h.state().service.remote_db.error.is_none());
        assert_eq!(
            h.state()
                .service
                .remote_db
                .result
                .as_ref()
                .unwrap()
                .rows
                .len(),
            1
        );
        let Cell::Json(text) = &h.state().service.remote_db.result.as_ref().unwrap().rows[0][1]
        else {
            panic!()
        };
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text).unwrap()["label"],
            "second"
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        assert_eq!(fs::read(&path).unwrap(), disk);
        set_query(&mut h, "db.users.aggregate([{$match:{_id:-99}}]);");
        place(&mut h, "_id");
        h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
        settle(&mut h);
        assert!(h.state().service.remote_db.error.is_none());
        assert!(
            h.state()
                .service
                .remote_db
                .result
                .as_ref()
                .unwrap()
                .rows
                .is_empty()
        );
        for (name, query) in [
            ("syntax", "db.users.aggregate([{$match:{}}]"),
            ("unsupported", "db.users.deleteMany({})"),
            ("server", "db.users.aggregate([{$unknownStage:{}}])"),
        ] {
            set_query(&mut h, query);
            h.state_mut().database_ui.output = true;
            place(&mut h, "users");
            h.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
            settle(&mut h);
            let error = h
                .state()
                .service
                .remote_db
                .error
                .clone()
                .expect("explicit failure required");
            assert!(!h.state().database_ui.output);
            if name == "server" {
                assert!(
                    error.contains("40324") && error.contains("$unknownStage"),
                    "actual server error expected: {error}"
                );
                assert!(!error.contains("RawDocumentBuf"));
            }
            assert!(
                h.get_all_by_label(&error).count() >= 2,
                "error must be rendered: {error}"
            );
            assert_eq!(h.state().service.active_document().unwrap().text(), query);
            capture(&mut h, &format!("aggregate-error-{name}-{scale}"));
        }
        // The final-output limit must not cap the documents before $group.
        config.row_limit = 2;
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Disconnect))
            .unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Save(config.clone())))
            .unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
                config: config.clone(),
                password: String::new(),
                test: false,
            }))
            .unwrap();
        settle(&mut h);
        set_query(&mut h, aggregate);
        place(&mut h, "$group");
        h.get_by_label("Executar consulta sob o cursor").click();
        settle(&mut h);
        assert!(
            h.state().service.remote_db.error.is_none(),
            "{:?}",
            h.state().service.remote_db.error
        );
        let result = h.state().service.remote_db.result.as_ref().unwrap();
        assert_eq!(result.rows.len(), 2);
        assert!(result.truncated);
        for row in &result.rows {
            let Cell::Json(text) = &row[1] else { panic!() };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(text).unwrap()["quantidadeDocumentos"]["$numberInt"],
                "3"
            );
        }
        capture(&mut h, &format!("aggregate-truncated-{scale}"));
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Disconnect))
            .unwrap();
        settle(&mut h);
        assert!(h.state().service.remote_db.active.is_none());
    }
}
