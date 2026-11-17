//! Real queries must resize the cached columns rather than reuse a prior result's geometry.
use codisseu_core::{AppService, Command, DbCommand};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use rusqlite::types::Value;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(ui: &mut Harness<'_, CodisseuApp>) {
    ui.step();
    let deadline = Instant::now() + Duration::from_secs(10);
    while ui.state().service.is_busy() {
        assert!(Instant::now() < deadline, "worker did not settle");
        thread::sleep(Duration::from_millis(1));
        ui.step();
    }
    ui.run();
}

fn execute(ui: &mut Harness<'_, CodisseuApp>, sql: &str) {
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
    ui.event(egui::Event::Text(sql.into()));
    settle(ui);
    assert_eq!(ui.state().service.active_document().unwrap().text(), sql);
    ui.get_by_label("Executar SQL").click();
    settle(ui);
    assert!(ui.state().service.db.last_error.is_none());
    assert_eq!(ui.state().service.db.result.as_ref().unwrap().rows.len(), 1);
}

fn first_column_width(ui: &Harness<'_, CodisseuApp>) -> f32 {
    header_rect(ui, "tail").left() - header_rect(ui, "value").left()
}

#[test]
fn new_query_replaces_cached_column_widths_and_empty_result_clears_revision() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("fixture.sqlite");
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE seed(id INTEGER);")
        .unwrap();
    let file = dir.path().join("query.sql");
    fs::write(&file, "SELECT 'x' AS value, 0 AS tail;").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(file)).unwrap();
    service
        .dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1440., 900.])
        .with_max_steps(30)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    assert_eq!(ui.state().service.db.result_revision(), None);

    execute(&mut ui, "SELECT 'x' AS value, 0 AS tail;");
    let narrow = first_column_width(&ui);
    let first_revision = ui.state().service.db.result_revision().unwrap();
    execute(
        &mut ui,
        "SELECT 'abcdefghijklmnopqrstuvwxyz0123456789' AS value, 0 AS tail;",
    );
    let wide = first_column_width(&ui);
    let second_revision = ui.state().service.db.result_revision().unwrap();
    assert_ne!(first_revision, second_revision);
    assert!(
        wide > narrow + 50.,
        "new result should widen first column: {narrow} -> {wide}"
    );
    for _ in 0..3 {
        ui.step();
    }
    assert_eq!(
        first_column_width(&ui),
        wide,
        "idle repaints retain geometry"
    );

    execute(&mut ui, "SELECT 'y' AS value, 0 AS tail;");
    let narrowed_again = first_column_width(&ui);
    assert!(
        (narrowed_again - narrow).abs() < 1.,
        "new short result should shrink again: {narrowed_again}"
    );
    assert_ne!(
        ui.state().service.db.result_revision().unwrap(),
        second_revision
    );
    // Programmatic setup does not choose the SQLite tool pane. Follow the real
    // Database menu before using its disconnect widget.
    ui.get_by_label("Adicionar datasource").click();
    settle(&mut ui);
    ui.get_by_label("SQLite (arquivo)").click();
    settle(&mut ui);
    ui.get_by_label("Desconectar").click();
    settle(&mut ui);
    assert_eq!(ui.state().service.db.result_revision(), None);
    assert!(
        ui.query_by_role_and_label(Role::Button, "Ordenar por value")
            .is_none()
    );
}

fn header_rect(ui: &Harness<'_, CodisseuApp>, name: &str) -> egui::Rect {
    // Query headers became sort buttons in M4.8; body values remain labels.
    ui.get_by_role_and_label(Role::Button, &format!("Ordenar por {name}"))
        .rect()
}

#[test]
fn vertical_scroll_keeps_sqlite_headers_visible_and_preserves_the_result() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("fixture.sqlite");
    rusqlite::Connection::open(&database).unwrap();
    let path = directory.path().join("query.sql");
    let sql = "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<100) SELECT n AS id, printf('row-%03d',n) AS label FROM numbers;";
    fs::write(&path, sql).unwrap();
    let mut service = AppService::new(directory.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    service
        .dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(50)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    ui.get_by_role(Role::MultilineTextInput).click();
    settle(&mut ui);
    ui.key_press_modifiers(Modifiers::COMMAND, Key::Enter);
    settle(&mut ui);
    assert!(ui.state().service.db.last_error.is_none());
    let expected = ui.state().service.db.result.as_ref().unwrap().rows.clone();
    assert_eq!(expected.len(), 100);
    let revision = ui.state().service.db.result_revision();
    let header = header_rect(&ui, "label");
    let first = ui.get_by_role_and_label(Role::Label, "row-001").rect();
    assert!(first.top() >= header.bottom());
    ui.event(egui::Event::PointerMoved(first.center()));
    ui.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., -500.),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    settle(&mut ui);
    let after = header_rect(&ui, "label");
    assert!(
        (after.top() - header.top()).abs() < 0.5,
        "header must remain pinned vertically"
    );
    assert!((after.left() - header.left()).abs() < 0.5);
    assert!(
        ui.query_by_role_and_label(Role::Label, "row-001").is_none(),
        "the first row must scroll out while its header remains"
    );
    assert!((2..=100).any(|row| {
        ui.query_by_role_and_label(Role::Label, &format!("row-{row:03}"))
            .is_some()
    }));
    assert_eq!(ui.state().service.db.result_revision(), revision);
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows,
        expected
    );
}

#[test]
fn viewport_resize_preserves_natural_result_widths_and_typed_values() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("fixture.sqlite");
    rusqlite::Connection::open(&database).unwrap();
    let file = dir.path().join("query.sql");
    let sql = "SELECT NULL AS absent, 9223372036854775807 AS exact, X'00FF80' AS payload;";
    fs::write(&file, sql).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(file)).unwrap();
    service
        .dispatch(Command::Database(DbCommand::Connect(database)))
        .unwrap();
    let mut ui = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(40)
        .build_eframe(|_| CodisseuApp::new(service));
    settle(&mut ui);
    execute(&mut ui, sql);
    let revision = ui.state().service.db.result_revision();
    let expected = vec![
        Value::Null,
        Value::Integer(i64::MAX),
        Value::Blob(vec![0, 255, 128]),
    ];
    let mut original_widths: Option<[f32; 3]> = None;
    for width in [1280., 900., 800., 1440., 1280.] {
        ui.set_size(egui::vec2(width, 900.));
        settle(&mut ui);
        let absent = header_rect(&ui, "absent");
        let exact = header_rect(&ui, "exact");
        let payload = header_rect(&ui, "payload");
        let grid_right = ui.get_by_label("Alternar painel Database").rect().left();
        let first_width = exact.left() - absent.left();
        let cell_widths = [
            first_width,
            payload.left() - exact.left(),
            payload.width() + 32., // 24px header icon/label inset + 8px right inset.
        ];
        assert!(cell_widths.iter().all(|width| *width >= 163.));
        let available = grid_right - (absent.left() - 24.);
        if cell_widths.iter().sum::<f32>() + 20. < available {
            assert!(
                payload.right() + 8. < grid_right - 20.,
                "fitting natural columns leave trailing space at {width}: {payload:?}, edge {grid_right}"
            );
        } else {
            assert!(
                payload.right() > grid_right,
                "narrow result pane must overflow rather than shrink natural columns"
            );
        }
        if let Some(original) = original_widths {
            assert!(
                cell_widths
                    .iter()
                    .zip(original)
                    .all(|(now, before)| (*now - before).abs() < 1.),
                "natural columns must keep their widths when the viewport changes"
            );
        } else {
            original_widths = Some(cell_widths);
        }
        assert_eq!(
            ui.state().service.db.result_revision(),
            revision,
            "resize must not re-execute query"
        );
        assert_eq!(
            ui.state().service.db.result.as_ref().unwrap().rows[0],
            expected
        );
        assert!(
            ui.query_by_role_and_label(Role::Label, "9223372036854775807")
                .is_some()
        );
        assert!(ui.query_by_role_and_label(Role::Label, "NULL").is_some());
        assert!(
            ui.query_by_role_and_label(Role::Label, "BLOB[3] X'00FF80'")
                .is_some()
        );
    }
    ui.set_pixels_per_point(1.25);
    settle(&mut ui);
    assert_eq!(ui.state().service.db.result_revision(), revision);
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows[0],
        expected
    );

    // Many naturally wide columns must overflow. Move the actual scroll area
    // to the end without reducing the intrinsic width of their values.
    let columns = (0..10)
        .map(|index| format!("'abcdefghijklmnopqrstuvwxyz0123456789' AS c{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    execute(&mut ui, &format!("SELECT {columns};"));
    let first = header_rect(&ui, "c0");
    let next = header_rect(&ui, "c1");
    assert!(next.left() - first.left() >= 200.);
    ui.event(egui::Event::PointerMoved(first.center()));
    ui.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(-3000., 0.),
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
        modifiers: Modifiers::NONE,
    });
    settle(&mut ui);
    let last = header_rect(&ui, "c9");
    let grid_right = ui.get_by_label("Alternar painel Database").rect().left();
    assert!(
        last.left() > 0. && last.right() <= grid_right,
        "horizontal scrolling must reveal final column: {last:?}, edge {grid_right}"
    );
    assert_eq!(
        ui.state().service.db.result.as_ref().unwrap().rows[0].len(),
        10
    );
}
