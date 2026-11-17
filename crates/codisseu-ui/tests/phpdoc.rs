use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Event, Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn fixture(source: &str, extension: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("main.{extension}"));
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_PHPDOC_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    let deadline = Instant::now() + Duration::from_secs(15);
    while h.state().service.active_document().is_none() || h.state().service.is_busy() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    h.step();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}
fn select(h: &mut Harness<'_, CodisseuApp>, selection: Selection) {
    let d = h.state().service.active_document().unwrap();
    let command = Command::SetSelection {
        id: d.id,
        expected_version: d.version,
        selection,
    };
    h.state_mut().service.dispatch(command).unwrap();
    h.step();
}
fn key(h: &mut Harness<'_, CodisseuApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.step();
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
fn source<'a>(h: &'a Harness<'_, CodisseuApp>) -> &'a str {
    h.state().service.active_document().unwrap().text()
}
fn selection(h: &Harness<'_, CodisseuApp>) -> Selection {
    h.state().service.active_document().unwrap().selection()
}

#[test]
fn typed_opener_generates_signature_in_one_undo_and_preserves_disk() {
    let original = "<?php\nclass GreetingService {\n    \n    public function greet(string $name, ?int $count = null): string {\n        return 'Hello';\n    }\n}\n";
    let (dir, mut h) = fixture(original, "php");
    let offset = original.find("    \n").unwrap() + 4;
    select(&mut h, Selection::caret(offset));
    for c in ["/", "*", "*"] {
        h.event(Event::Text(c.into()));
        h.step();
    }
    let before = source(&h).to_owned();
    let caret = selection(&h);
    key(&mut h, Modifiers::NONE, Key::Enter);
    let expanded = source(&h).to_owned();
    assert!(expanded.contains("    /**\n     * \n     * @param string $name\n     * @param int|null $count\n     * @return string\n     */\n    public function"), "{expanded}");
    assert_eq!(
        selection(&h),
        Selection::caret(caret.head + "\n     * ".len())
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("main.php")).unwrap(),
        original
    );
    if let Some(out) = std::env::var_os("CODISSEU_PHPDOC_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(std::path::PathBuf::from(out).join("generated-phpdoc.png"))
            .unwrap();
    }
    let after = selection(&h);
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(source(&h), before);
    assert_eq!(selection(&h), caret);
    key(&mut h, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
    assert_eq!(source(&h), expanded);
    assert_eq!(selection(&h), after);
    h.event(Event::Text("Description".into()));
    h.step();
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(source(&h).contains(" * Description\n     * \n     * @param"));
}

#[test]
fn crlf_inline_closer_and_unicode_caret_are_preserved() {
    let original =
        "<?php\r\n// ação 😀\r\n\t/** */\r\n\tfunction nome(string $nome): string {}\r\n";
    let (_dir, mut h) = fixture(original, "phtml");
    let caret = original[..original.find("/**").unwrap() + 3]
        .chars()
        .count();
    select(&mut h, Selection::caret(caret));
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert!(
        source(&h).contains(
            "\t/**\r\n\t * \r\n\t * @param string $nome\r\n\t * @return string\r\n\t */\r\n"
        ),
        "{}",
        source(&h)
    );
    assert_eq!(selection(&h).head, caret + 6);
    assert_eq!(source(&h).matches("*/").count(), 1);
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(source(&h), original);
}

#[test]
fn ordinary_comments_space_ctrl_enter_strings_and_other_languages_stay_literal() {
    for (original, ext, marker, modifiers) in [
        (
            "<?php\n/*\nfunction f(): string {}",
            "php",
            "/*",
            Modifiers::NONE,
        ),
        (
            "<?php\n/**\nfunction f(): string {}",
            "php",
            "/**",
            Modifiers::COMMAND,
        ),
        ("<?php\n$s = '\n/**\n';", "php", "/**", Modifiers::NONE),
        (
            "<?php\n$s = \"{$a[\"\n/**\n\"]}\";",
            "php",
            "/**",
            Modifiers::NONE,
        ),
        ("<?php\n/* comment\n/**\n*/", "php", "/**", Modifiers::NONE),
        (
            "<html>\n/**\nfunction f(): string {}",
            "phtml",
            "/**",
            Modifiers::NONE,
        ),
        ("/**\nfunction f() {}", "ts", "/**", Modifiers::NONE),
    ] {
        let (_dir, mut h) = fixture(original, ext);
        select(
            &mut h,
            Selection::caret(original.find(marker).unwrap() + marker.len()),
        );
        key(&mut h, modifiers, Key::Enter);
        assert!(!source(&h).contains("@return"), "{}", source(&h));
        assert!(!source(&h).contains("\n * "), "{}", source(&h));
    }
    let (_dir, mut h) = fixture("<?php\n/**\nfunction f(): string {}", "php");
    select(&mut h, Selection::caret(9));
    h.event(Event::Text(" ".into()));
    h.step();
    assert_eq!(source(&h), "<?php\n/** \nfunction f(): string {}");
}

#[test]
fn ime_and_selection_do_not_generate_docblocks() {
    let original = "<?php\n/**\nfunction f(): string {}";
    let (_dir, mut h) = fixture(original, "php");
    select(&mut h, Selection::caret(9));
    h.event(Event::Ime(egui::ImeEvent::Preedit {
        text: "文".into(),
        active_range_chars: None,
    }));
    h.step();
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(source(&h), original);
    key(&mut h, Modifiers::NONE, Key::Escape);
    select(&mut h, Selection { anchor: 6, head: 9 });
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(source(&h), "<?php\n\n\nfunction f(): string {}");
}

#[test]
fn multiple_carets_keep_plain_enter_and_atomic_undo() {
    let original = "<?php\n/**\n/**\nfunction f(): string {}";
    let (_dir, mut h) = fixture(original, "php");
    select(&mut h, Selection::caret(9));
    key(
        &mut h,
        Modifiers::SHIFT.plus(Modifiers::ALT),
        Key::ArrowDown,
    );
    assert_eq!(h.state().editor_metrics().selection_count, 2);
    key(&mut h, Modifiers::NONE, Key::Enter);
    assert_eq!(source(&h), "<?php\n/**\n\n/**\n\nfunction f(): string {}");
    key(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(source(&h), original);
}
