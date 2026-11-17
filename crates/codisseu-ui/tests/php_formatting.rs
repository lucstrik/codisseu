use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(h: &mut Harness<'_, CodisseuApp>) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if !h.state().service.is_busy() {
            h.step();
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
#[test]
fn shortcut_formats_whole_active_php_even_with_selection_and_preserves_other_tab() {
    let temp = tempfile::tempdir().unwrap();
    let other = temp.path().join("untouched.php");
    let path = temp.path().join("active.php");
    let source = "<?php class Greeting{public function greet(string $name):string{return 'Hello '.$name;}}\n";
    fs::write(&other, "<?php $x=1;").unwrap();
    fs::write(&path, source).unwrap();
    let mut s = AppService::new(temp.path().join("session.json"));
    s.dispatch(Command::OpenFile(other.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_PHP_FORMAT_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
    idle(&mut h);
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.clone()))
        .unwrap();
    idle(&mut h);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    let d = h.state().service.active_document().unwrap();
    let id = d.id;
    let cmd = Command::SetSelection {
        id,
        expected_version: d.version,
        selection: Selection {
            anchor: 12,
            head: 8,
        },
    };
    h.state_mut().service.dispatch(cmd).unwrap();
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
    idle(&mut h);
    let formatted = h
        .state()
        .service
        .active_document()
        .unwrap()
        .text()
        .to_owned();
    assert_ne!(formatted, source);
    assert!(formatted.contains("    public function"));
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert_eq!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.path == other)
            .unwrap()
            .text(),
        "<?php $x=1;"
    );
    if let Some(out) = std::env::var_os("CODISSEU_PHP_FORMAT_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(std::path::PathBuf::from(out).join("formatted-php.png"))
            .unwrap();
    }
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    idle(&mut h);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    idle(&mut h);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        formatted
    );
}
