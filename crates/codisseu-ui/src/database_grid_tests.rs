//! Real SQLite and opt-in owned remote fixtures; user profiles are never opened.
use super::*;
use codisseu_core::{
    DbCommand,
    remote::{Cell, ConnectionConfig, Engine, RemoteCommand, TableRef},
};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{fs, thread, time::Instant};

fn settle(h: &mut Harness<CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().service.is_busy() && !h.state().transfer_views.grid.pending() {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    h.run();
}
fn click(h: &mut Harness<CodisseuApp>, label: &str, button: egui::PointerButton) {
    let pos = h.get_by_label(label).rect().center();
    h.event(egui::Event::PointerMoved(pos));
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        });
        h.step();
    }
    h.run();
}
fn fill(h: &mut Harness<CodisseuApp>, label: &str, text: &str) {
    h.get_by_label(label).focus();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.step();
    h.event(egui::Event::Text(text.into()));
    settle(h);
}
fn capture(h: &mut Harness<CodisseuApp>, name: &str, scale: f32) {
    if let Some(out) = std::env::var_os("CODISSEU_GRID_CAPTURES") {
        let out = PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        // Capture a complete repaint after popup/layout transitions, rather
        // than the final event pass of a multi-pass offscreen frame.
        h.ctx.request_repaint();
        h.run_steps(2);
        h.run();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}-{scale}.png")))
            .unwrap();
    }
}
fn sqlite(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let temp = tempfile::tempdir().unwrap();
    let db = temp.path().join("own.sqlite");
    rusqlite::Connection::open(&db).unwrap().execute_batch("CREATE TABLE records(id INTEGER PRIMARY KEY,name TEXT,note TEXT,amount BIGINT); INSERT INTO records VALUES (1,'John','SECRET1',9223372036854775807),(2,'Jane','SECRET2',0),(3,'JOHNNY',NULL,-1),(4,'ação 😀','SECRET4',7);").unwrap();
    let source = temp.path().join("query.sql");
    fs::write(
        &source,
        "SELECT id,name,note,amount FROM records ORDER BY id;\n",
    )
    .unwrap();
    let mut app = CodisseuApp::new(AppService::new(temp.path().join("profile/session.json")));
    app.show_project = false;
    app.show_database = false;
    app.service.dispatch(Command::OpenFile(source)).unwrap();
    app.service
        .dispatch(Command::Database(DbCommand::Connect(db)))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1440., 900.])
        .with_pixels_per_point(scale)
        .with_max_steps(200)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    settle(&mut h);
    h.state_mut().execute_sql();
    settle(&mut h);
    (temp, h)
}
#[test]
fn grid_sqlite_real_resize_visibility_search_copy_reset_and_editor_shortcuts() {
    for scale in [1., 1.25] {
        let (temp, mut h) = sqlite(scale);
        let revision = h.state().service.db.result_revision();
        let resize = "Resize column name (2)";
        let original_width = h.get_by_label("Ordenar por note").rect().left()
            - h.get_by_label("Ordenar por name").rect().left();
        let start = h.get_by_label(resize).rect().center();
        h.event(egui::Event::PointerMoved(start));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        h.event(egui::Event::PointerMoved(start + egui::vec2(90., 0.)));
        h.step();
        h.event(egui::Event::PointerMoved(start + egui::vec2(120., 0.)));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: start + egui::vec2(120., 0.),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        settle(&mut h);
        assert!(
            (h.state().transfer_views.grid.widths[&1] - original_width - 120.).abs() < 1.,
            "{} vs {}",
            h.state().transfer_views.grid.widths[&1],
            original_width
        );
        assert_eq!(h.state().service.db.result_revision(), revision);
        click(&mut h, "Ordenar por note", egui::PointerButton::Secondary);
        h.get_by_label("Hide Column").click();
        settle(&mut h);
        assert_eq!(
            h.state().transfer_views.grid.visible_columns(),
            vec![0, 1, 3]
        );
        assert!(h.query_by_label("SECRET1").is_none());
        click(&mut h, "Ordenar por amount", egui::PointerButton::Secondary);
        h.get_by_label("Show Column List").click();
        settle(&mut h);
        fill(&mut h, "Search columns", "note");
        h.get_by_label("Column 3: note").click();
        h.run();
        h.key_press(Key::Space);
        settle(&mut h);
        assert!(!h.state().transfer_views.grid.hidden.contains(&2));
        h.key_press(Key::Space);
        settle(&mut h);
        assert!(h.state().transfer_views.grid.hidden.contains(&2));
        capture(&mut h, "columns-sqlite", scale);
        h.key_press(Key::Escape);
        settle(&mut h);
        h.get_by_label("John").click();
        h.run();
        h.key_press_modifiers(Modifiers::CTRL, Key::F);
        settle(&mut h);
        fill(&mut h, "Find on current page", "john");
        assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![0, 2]);
        assert!(h.query_by_label("Jane").is_none());
        assert_eq!(h.state().service.db.result_revision(), revision);
        capture(&mut h, "filtered-sqlite", scale);
        h.set_size(egui::vec2(980., 760.));
        h.run();
        assert!(h.get_by_label("Close Find on Current Page").rect().right() <= 980.);
        capture(&mut h, "narrow-filtered-sqlite", scale);
        h.set_size(egui::vec2(1440., 900.));
        h.run();
        h.get_by_label("Match case").click();
        settle(&mut h);
        assert!(h.state().transfer_views.grid.visible_rows().is_empty());
        h.get_by_label("Match case").click();
        settle(&mut h);
        h.get_by_label("Filter rows").click();
        settle(&mut h);
        assert_eq!(
            h.state().transfer_views.grid.visible_rows(),
            vec![0, 1, 2, 3]
        );
        assert!(h.state().transfer_views.grid.matches(2, 1));
        h.get_by_label("Filter rows").click();
        settle(&mut h);
        h.get_by_label("Next match").click();
        settle(&mut h);
        h.get_by_label("John").click();
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        h.event(egui::Event::Copy);
        let end = Instant::now() + Duration::from_secs(5);
        let copied = loop {
            h.step();
            if let Some(text) = h.output().platform_output.commands.iter().find_map(|c| {
                if let egui::OutputCommand::CopyText(t) = c {
                    Some(t.clone())
                } else {
                    None
                }
            }) {
                break text;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(2));
        };
        assert!(
            !copied.contains("SECRET") && !copied.contains("Jane"),
            "{copied}"
        );
        assert!(
            copied.contains("9223372036854775807") && copied.contains("JOHNNY"),
            "{copied}"
        );
        assert_eq!(copied.lines().count(), 2);
        h.get_by_label("Find on current page").focus();
        h.run();
        h.key_press(Key::Escape);
        settle(&mut h);
        assert_eq!(
            h.state().transfer_views.grid.visible_rows(),
            vec![0, 1, 2, 3]
        );
        h.get_by_label("Show Options Menu").click();
        h.run();
        h.get_by_label("Reset View").click();
        settle(&mut h);
        assert!(h.state().transfer_views.grid.widths.is_empty());
        assert!(h.state().transfer_views.grid.hidden.is_empty());
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.run();
        h.key_press_modifiers(Modifiers::CTRL, Key::F);
        h.run();
        assert!(
            !h.state().transfer_views.grid.find_open,
            "Editor search must stay in the editor"
        );
        assert_eq!(
            fs::read_to_string(temp.path().join("query.sql")).unwrap(),
            "SELECT id,name,note,amount FROM records ORDER BY id;\n"
        );
        assert_eq!(h.state().service.db.result_revision(), revision);
    }
}

#[test]
fn grid_sqlite_column_filters_focus_geometry_and_selection_are_real() {
    for scale in [1., 1.25] {
        let (temp, mut h) = sqlite(scale);
        let revision = h.state().service.db.result_revision();
        h.get_by_label("Enable Local Filter").click();
        settle(&mut h);
        h.get_by_label("Filter column 3: note").click();
        settle(&mut h);
        assert!(h.get_by_label("Search values").is_focused());
        h.event(egui::Event::Text("null".into()));
        settle(&mut h);
        h.get_by_label("Clear Values").click();
        settle(&mut h);
        assert!(h.state().transfer_views.grid.visible_rows().is_empty());
        h.get_by_label("Filter value 1: <null> (1)").click();
        settle(&mut h);
        assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![2]);
        capture(&mut h, "column-filter-null", scale);
        h.get_by_label("Close Column Filter").click();
        settle(&mut h);
        h.get_by_label("Filter column 4: amount").click();
        settle(&mut h);
        h.get_by_label("Clear Values").click();
        settle(&mut h);
        h.get_by_label("Filter value 1: -1 (1)").click();
        settle(&mut h);
        h.get_by_label("Close Column Filter").click();
        settle(&mut h);
        assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![2]);
        h.get_by_label("Enable Local Filter").click();
        settle(&mut h);
        assert_eq!(
            h.state().transfer_views.grid.visible_rows(),
            vec![0, 1, 2, 3]
        );
        h.get_by_label("Enable Local Filter").click();
        settle(&mut h);
        assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![2]);
        click(&mut h, "Ordenar por note", egui::PointerButton::Secondary);
        h.get_by_label("Hide Column").click();
        settle(&mut h);
        assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![2]);
        click(&mut h, "Ordenar por name", egui::PointerButton::Secondary);
        h.get_by_label("Select Column").click();
        settle(&mut h);
        assert!(h.state().grid_selection.contains(2, 1));
        h.event(egui::Event::Copy);
        let end = Instant::now() + Duration::from_secs(5);
        let copied = loop {
            h.step();
            if let Some(t) = h.output().platform_output.commands.iter().find_map(|c| {
                if let egui::OutputCommand::CopyText(t) = c {
                    Some(t.clone())
                } else {
                    None
                }
            }) {
                break t;
            }
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(copied, "JOHNNY");
        h.key_press_modifiers(Modifiers::CTRL, Key::F12);
        settle(&mut h);
        assert!(h.get_by_label("Search columns").is_focused());
        h.event(egui::Event::Text("note".into()));
        settle(&mut h);
        assert_eq!(
            h.get_by_label("Column 3: note")
                .accesskit_node()
                .is_selected(),
            Some(true)
        );
        let pos = egui::pos2(110., 180.);
        h.event(egui::Event::PointerMoved(pos));
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Modifiers::NONE,
            });
        }
        settle(&mut h);
        assert!(!h.state().transfer_views.columns_open);
        h.set_size(egui::vec2(980., 760.));
        h.run();
        h.get_by_label("Filter column 4: amount").click();
        settle(&mut h);
        assert!(h.get_by_label("Close Column Filter").rect().right() <= 980.);
        capture(&mut h, "column-filter-narrow", scale);
        h.key_press(Key::Escape);
        settle(&mut h);
        assert!(h.state().transfer_views.grid.local.open.is_none());
        click(&mut h, "Ordenar por name", egui::PointerButton::Secondary);
        h.get_by_label("Clear Local Filters").click();
        settle(&mut h);
        assert_eq!(
            h.state().transfer_views.grid.visible_rows(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(h.state().service.db.result_revision(), revision);
        assert_eq!(
            fs::read_to_string(temp.path().join("query.sql")).unwrap(),
            "SELECT id,name,note,amount FROM records ORDER BY id;\n"
        );
    }
}

#[test]
#[ignore = "requires owned local MySQL/PostgreSQL fixtures from tools/grid_inspection_fixtures.py"]
fn grid_remote_real_visibility_filter_preserves_edit_identity_and_rollback() {
    for (engine, key, namespace, user) in [
        (Engine::Mysql, "MY", "codisseu", "root"),
        (Engine::Postgres, "PG", "public", "postgres"),
    ] {
        for scale in [1., 1.25] {
            let temp = tempfile::tempdir().unwrap();
            let mut app = CodisseuApp::new(AppService::new(temp.path().join("session.json")));
            app.show_project = false;
            app.show_database = false;
            app.show_activity = true;
            app.tool_tab = ToolTab::Results;
            app.database_ui.remote_results = true;
            let config = ConnectionConfig {
                id: format!("owned-grid-{key}"),
                name: format!("Owned {key}"),
                engine,
                host: "127.0.0.1".into(),
                port: std::env::var(format!("CODISSEU_GRID_{key}_PORT"))
                    .unwrap()
                    .parse()
                    .unwrap(),
                database: "codisseu".into(),
                username: user.into(),
                tls: false,
                read_only: false,
                save_credentials: false,
                ..Default::default()
            };
            let mut h = Harness::builder()
                .with_size([1440., 900.])
                .with_pixels_per_point(scale)
                .with_step_dt(1. / 60.)
                .with_max_steps(200)
                .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
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
            let table = TableRef {
                namespace: namespace.into(),
                name: "grid_inspection".into(),
            };
            h.state_mut().remote_send(RemoteCommand::LoadTablePage {
                table: table.clone(),
                offset: 0,
            });
            settle(&mut h);
            assert!(
                h.state().service.remote_db.error.is_none(),
                "{:?}",
                h.state().service.remote_db.error
            );
            let before = h.state().service.remote_db.revision;
            h.get_by_label("Enable Local Filter").click();
            settle(&mut h);
            h.get_by_label("Filter column 4: amount").click();
            settle(&mut h);
            h.get_by_label("Clear Values").click();
            settle(&mut h);
            h.get_by_label("Filter value 2: 0 (1)").click();
            settle(&mut h);
            h.get_by_label("Close Column Filter").click();
            settle(&mut h);
            assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![1]);
            // These source-index toggles use the same state changed by the header menu.
            h.state_mut().transfer_views.grid.toggle_column(0);
            h.state_mut().transfer_views.grid.toggle_column(2);
            settle(&mut h);
            h.get_by_label("Find on Current Page").click();
            settle(&mut h);
            fill(&mut h, "Find on current page", "Jane");
            assert_eq!(h.state().transfer_views.grid.visible_rows(), vec![1]);
            assert_eq!(h.state().service.remote_db.revision, before);
            let pos = h.get_by_label("Jane").rect().center();
            capture(&mut h, &format!("before-edit-{key}"), scale);
            // Separate this gesture from the preceding toolbar click; otherwise
            // egui counts three rapid clicks even though the first was elsewhere.
            h.run_steps(24);
            for click_index in 0..2 {
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
                if click_index == 1 {
                    assert!(
                        h.ctx.input(|i| i
                            .pointer
                            .button_double_clicked(egui::PointerButton::Primary)),
                        "Not a double click; time {}",
                        h.ctx.input(|i| i.time)
                    );
                }
            }
            settle(&mut h);
            assert!(
                h.state().grid_selection.contains(1, 1),
                "Cell at {pos:?} did not select"
            );
            assert!(
                h.state().database_ui.edit.is_some(),
                "Double click at {pos:?} selected but did not open edit; profiles {:?}",
                h.state().service.remote_db.profiles
            );
            h.get_by_role_and_label(egui::accesskit::Role::MultilineTextInput, "Cell value")
                .focus();
            h.step();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.step();
            h.event(egui::Event::Text("Jane updated".into()));
            settle(&mut h);
            h.get_by_label("Stage Change").click();
            settle(&mut h);
            assert_eq!(
                h.state().database_ui.staged.get(&(1, 1)),
                Some(&Cell::Text("Jane updated".into()))
            );
            capture(&mut h, &format!("staged-{key}"), scale);
            h.get_by_label("Submit").click();
            settle(&mut h);
            assert!(
                h.state().service.remote_db.error.is_none(),
                "{:?}",
                h.state().service.remote_db.error
            );
            assert!(h.state().service.remote_db.transaction);
            assert!(h.state().database_ui.staged.is_empty());
            h.get_by_label("Rollback").click();
            settle(&mut h);
            assert!(!h.state().service.remote_db.transaction);
            h.state_mut()
                .remote_send(RemoteCommand::LoadTablePage { table, offset: 0 });
            settle(&mut h);
            assert_eq!(
                h.state().service.remote_db.result.as_ref().unwrap().rows[1][1],
                Cell::Text("Jane".into())
            );
            h.state_mut().remote_send(RemoteCommand::Disconnect);
            settle(&mut h);
            assert!(h.state().service.remote_db.active.is_none());
        }
    }
}
