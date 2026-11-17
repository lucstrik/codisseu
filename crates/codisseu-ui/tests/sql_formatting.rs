use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            h.step();
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
#[test]
fn shortcut_always_formats_active_file_with_selection_undo_and_no_save() {
    let original = "select id,name from users where active=1 order by name;\nselect 2;select 3;\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("query.sql");
    fs::write(&path, original).unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_SQL_FORMAT_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    wait(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
    wait(&mut h);
    let formatted = h
        .state()
        .service
        .active_document()
        .unwrap()
        .text()
        .to_owned();
    assert_ne!(formatted, original);
    assert!(formatted.lines().count() > 5);
    assert!(
        formatted.contains(";\n\nselect\n  2;\n\nselect\n  3;\n"),
        "{formatted}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    if let Some(out) = std::env::var_os("CODISSEU_SQL_FORMAT_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(std::path::PathBuf::from(out).join("formatted-sql.png"))
            .unwrap();
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    wait(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        original
    );
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    wait(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        formatted
    );
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    wait(&mut h);
    let d = h.state().service.active_document().unwrap();
    let id = d.id;
    let range = 0..original.find(';').unwrap() + 1;
    let cmd = Command::SetSelection {
        id,
        expected_version: d.version,
        selection: Selection {
            anchor: range.end,
            head: 0,
        },
    };
    h.state_mut().service.dispatch(cmd).unwrap();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
    wait(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        formatted
    );
}
