//! Real SQLite workers and egui input; no synthetic database acknowledgements.
use super::*;
use codisseu_core::{
    DbCommand,
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand},
};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{thread, time::Instant};

fn settle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().service.is_busy() && !h.state().database_ui.refresh_after_edit {
            h.run_steps(3);
            break;
        }
        assert!(
            Instant::now() < end,
            "{} / {:?} / {:?}",
            h.state().service.db.status,
            h.state().service.db.last_error,
            h.state().service.remote_db.error
        );
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
}
fn sqlite(scale: f32, writable: bool) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("grid.sqlite");
    let c = rusqlite::Connection::open(&db).unwrap();
    c.execute_batch("CREATE TABLE users(id INTEGER PRIMARY KEY,name TEXT NOT NULL,quantity BIGINT,enabled BOOLEAN NOT NULL DEFAULT 0,note TEXT); INSERT INTO users VALUES(1,'Ada',NULL,1,'original'),(2,'Bruno',9,0,'untouched');").unwrap();
    drop(c);
    let mut a = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
    a.show_project = false;
    a.show_activity = true;
    a.tool_tab = ToolTab::Results;
    a.database_ui.sqlite = true;
    a.service
        .dispatch(Command::Database(if writable {
            DbCommand::ConnectWritable(db)
        } else {
            DbCommand::Connect(db)
        }))
        .unwrap();
    let mut h = Harness::builder()
        .with_step_dt(1. / 60.)
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(300)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), a);
    settle(&mut h);
    execute(&mut h, "SELECT * FROM users ORDER BY id");
    (dir, h)
}
fn execute(h: &mut Harness<'_, CodisseuApp>, sql: &str) {
    h.state_mut().database_ui.sqlite_pending_sql = Some(sql.into());
    h.state_mut()
        .service
        .dispatch(Command::Database(DbCommand::Execute { sql: sql.into() }))
        .unwrap();
    settle(h);
}
fn double_click(h: &mut Harness<'_, CodisseuApp>, label: &str) {
    let pos = h.get_by_label(label).rect().center();
    h.run_steps(30);
    for _ in 0..2 {
        h.event(egui::Event::PointerMoved(pos));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
    }
    h.run_steps(3);
}
fn fill(h: &mut Harness<'_, CodisseuApp>, column: &str, value: &str) {
    h.get_by_label(&format!("Editar {column}")).focus();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.step();
    h.event(egui::Event::Text(value.into()));
    h.run_steps(3);
}
fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_GRID_EDIT_CAPTURES") {
        std::fs::create_dir_all(&out).unwrap();
        h.step();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join(format!("{name}.png")))
            .unwrap();
    }
}
#[test]
fn grid_double_click_inline_commit_and_only_target_row_changes() {
    for scale in [1., 1.25] {
        let (dir, mut h) = sqlite(scale, true);
        double_click(&mut h, "Ada");
        assert!(h.state().database_ui.edit.is_some());
        fill(&mut h, "name", "Ada ação 😀");
        capture(&mut h, &format!("inline-{scale}"));
        h.key_press(Key::Enter);
        settle(&mut h);
        assert_eq!(
            h.state().database_ui.staged.get(&(0, 1)),
            Some(&Cell::Text("Ada ação 😀".into()))
        );
        let c = rusqlite::Connection::open(dir.path().join("grid.sqlite")).unwrap();
        assert_eq!(
            c.query_row("SELECT name FROM users WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Ada"
        );
        capture(&mut h, &format!("pending-{scale}"));
        h.get_by_label("Commit").click();
        settle(&mut h);
        assert!(
            h.state().service.db.last_error.is_none(),
            "{:?}",
            h.state().service.db.last_error
        );
        assert!(h.state().database_ui.staged.is_empty());
        assert!(!h.state().service.db.transaction);
        assert_eq!(
            c.query_row("SELECT name FROM users WHERE id=1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Ada ação 😀"
        );
        assert_eq!(
            c.query_row("SELECT name FROM users WHERE id=2", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Bruno"
        );
        assert_eq!(
            h.state().service.db.result.as_ref().unwrap().rows[0][1],
            rusqlite::types::Value::Text("Ada ação 😀".into())
        );
        capture(&mut h, &format!("committed-{scale}"));
    }
}
#[test]
fn grid_null_integer_rejects_letters_paste_and_bounds_and_keeps_exact_i64() {
    let (dir, mut h) = sqlite(1., true);
    h.state_mut().start_grid_edit(0, 2);
    h.run_steps(3);
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "NULL")
        .click();
    h.run_steps(3);
    fill(&mut h, "quantity", "letters");
    assert!(h.state().database_ui.edit.as_ref().unwrap().2.is_empty());
    fill(&mut h, "quantity", "9223372036854775808");
    h.key_press(Key::Enter);
    settle(&mut h);
    assert!(h.state().database_ui.edit_error.is_some());
    assert!(h.state().database_ui.staged.is_empty());
    fill(&mut h, "quantity", "9223372036854775807");
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(
        h.state().database_ui.staged.get(&(0, 2)),
        Some(&Cell::Number("9223372036854775807".into()))
    );
    h.get_by_label("Commit").click();
    settle(&mut h);
    let c = rusqlite::Connection::open(dir.path().join("grid.sqlite")).unwrap();
    assert_eq!(
        c.query_row("SELECT quantity FROM users WHERE id=1", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        i64::MAX
    );
}
#[test]
fn grid_submit_is_manual_rollback_and_escape_preserve_original() {
    let (dir, mut h) = sqlite(1., true);
    double_click(&mut h, "Ada");
    fill(&mut h, "name", "Discarded");
    h.key_press(Key::Escape);
    settle(&mut h);
    assert!(h.state().database_ui.staged.is_empty());
    assert!(h.state().database_ui.edit.is_none());
    double_click(&mut h, "Ada");
    fill(&mut h, "name", "Temporary");
    h.key_press(Key::Enter);
    settle(&mut h);
    h.get_by_label("Submit").click();
    settle(&mut h);
    assert!(h.state().service.db.transaction);
    let c = rusqlite::Connection::open(dir.path().join("grid.sqlite")).unwrap();
    assert_eq!(
        c.query_row("SELECT name FROM users WHERE id=1", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "Ada"
    );
    h.get_by_label("Rollback").click();
    settle(&mut h);
    assert!(!h.state().service.db.transaction);
    assert_eq!(
        h.state().service.db.result.as_ref().unwrap().rows[0][1],
        rusqlite::types::Value::Text("Ada".into())
    );
}
#[test]
fn grid_sqlite_explicit_write_enable_reruns_exact_query() {
    let (_dir, mut h) = sqlite(1., false);
    double_click(&mut h, "Ada");
    assert!(h.state().database_ui.edit.is_none());
    h.get_by_label("Habilitar edição").click();
    settle(&mut h);
    assert!(h.state().service.db.writable);
    assert!(h.state().service.db.origin.is_some());
    double_click(&mut h, "Ada");
    assert!(h.state().database_ui.edit.is_some());
}
#[test]
fn grid_boolean_uses_picker_and_not_null_column_refuses_null() {
    let (_dir, mut h) = sqlite(1., true);
    h.state_mut().start_grid_edit(0, 3);
    h.run_steps(3);
    let null = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "NULL");
    assert!(null.accesskit_node().is_disabled());
    h.get_by_label("Editar enabled").click();
    h.run_steps(3);
    h.get_by_label("false").click();
    h.run_steps(3);
    h.key_press(Key::Enter);
    settle(&mut h);
    assert_eq!(
        h.state().database_ui.staged.get(&(0, 3)),
        Some(&Cell::Bool(false))
    );
    h.get_by_label("Commit").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.db.result.as_ref().unwrap().rows[0][3],
        rusqlite::types::Value::Integer(0)
    );
}
#[test]
fn grid_readonly_join_and_primary_key_cannot_be_edited() {
    let (_dir, mut h) = sqlite(1., true);
    h.state_mut().start_grid_edit(0, 0);
    assert!(h.state().database_ui.edit.is_none());
    execute(
        &mut h,
        "SELECT a.id,a.name FROM users a JOIN users b ON a.id=b.id",
    );
    double_click(&mut h, "Ada");
    assert!(h.state().database_ui.edit.is_none());
}
#[test]
fn grid_proposal_cannot_follow_a_different_query_row_or_disconnect() {
    let (dir, mut h) = sqlite(1., true);
    double_click(&mut h, "Ada");
    fill(&mut h, "name", "Must stay on Ada");
    h.key_press(Key::Enter);
    settle(&mut h);
    h.state_mut().send(Command::Database(DbCommand::Execute {
        sql: "SELECT * FROM users ORDER BY id DESC".into(),
    }));
    assert!(
        !h.state().service.db.is_busy(),
        "UI must block result replacement with proposed cells"
    );
    h.state_mut().send(Command::Database(DbCommand::Disconnect));
    assert!(h.state().service.db.connected);
    // An independent API caller can replace a result. Its revision must invalidate
    // the GUI proposal instead of reconstructing its original row from new data.
    h.state_mut()
        .service
        .dispatch(Command::Database(DbCommand::Execute {
            sql: "SELECT * FROM users ORDER BY id DESC".into(),
        }))
        .unwrap();
    settle(&mut h);
    h.state_mut().submit_grid_edits(true);
    settle(&mut h);
    assert!(h.state().database_ui.edit_error.is_some());
    let c = rusqlite::Connection::open(dir.path().join("grid.sqlite")).unwrap();
    let names = c
        .prepare("SELECT name FROM users ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(names, ["Ada", "Bruno"]);
}
#[test]
#[ignore = "requires disposable CODISSEU_SQL_PG_PORT / MY_PORT / MARIA_PORT loopback containers"]
fn grid_remote_real_commit_inline_and_nullable_numeric_on_each_engine() {
    for (engine, key) in [
        (Engine::Postgres, "CODISSEU_SQL_PG_PORT"),
        (Engine::Mysql, "CODISSEU_SQL_MY_PORT"),
        (Engine::Mysql, "CODISSEU_SQL_MARIA_PORT"),
    ] {
        let port: u16 = std::env::var(key)
            .expect("explicit disposable port")
            .parse()
            .unwrap();
        // The fixture is seeded by grid_edit_remote integration tests before this UI replay.
        let dir = tempfile::tempdir().unwrap();
        let config = ConnectionConfig {
            id: "grid-ui".into(),
            name: "Disposable grid UI".into(),
            engine,
            host: "127.0.0.1".into(),
            port,
            database: "codisseu".into(),
            username: if engine == Engine::Postgres {
                "postgres"
            } else {
                "root"
            }
            .into(),
            read_only: false,
            tls: false,
            save_credentials: false,
            ..Default::default()
        };
        let mut a = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        a.show_activity = true;
        a.show_project = false;
        a.tool_tab = ToolTab::Results;
        a.database_ui.remote_results = true;
        let mut h = Harness::builder()
            .with_step_dt(1. / 60.)
            .with_size([1280., 800.])
            .with_max_steps(300)
            .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), a);
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Connect {
                config,
                password: "codisseu_fixture".into(),
                test: false,
            }))
            .unwrap();
        settle(&mut h);
        h.state_mut().database_ui.pending_sql =
            Some("SELECT * FROM grid_edit_ui ORDER BY id".into());
        h.state_mut().database_ui.pending_result_source = Some(None);
        h.state_mut().remote_send(RemoteCommand::Execute(
            "SELECT * FROM grid_edit_ui ORDER BY id".into(),
        ));
        settle(&mut h);
        assert!(
            h.state().service.remote_db.origin.is_some(),
            "{:?}",
            h.state().service.remote_db.error
        );
        double_click(&mut h, "Grid original");
        fill(&mut h, "name", "Grid committed");
        h.key_press(Key::Enter);
        settle(&mut h);
        h.get_by_label("Commit").click();
        settle(&mut h);
        assert!(
            h.state().service.remote_db.error.is_none(),
            "{:?}",
            h.state().service.remote_db.error
        );
        assert!(!h.state().service.remote_db.transaction);
        assert!(h.state().database_ui.staged.is_empty());
        assert_eq!(
            h.state().service.remote_db.result.as_ref().unwrap().rows[0][1],
            Cell::Text("Grid committed".into())
        );
        capture(&mut h, &format!("remote-{key}"));
    }
}
