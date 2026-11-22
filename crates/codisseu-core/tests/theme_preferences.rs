use codisseu_core::{AppService, Command, EditKind, Selection, TextDelta, ThemePreference};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(service: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() || service.ai.profile_pending() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
}

#[test]
fn themes_persist_and_restore_without_changing_source_views_or_history() {
    let dir = tempfile::tempdir().unwrap();
    let source = "<?php // ação 🦀 e\u{301}\r\nreturn 42;";
    let file = dir.path().join("sample.php");
    let session = dir.path().join("session.json");
    fs::write(&file, source).unwrap();
    let mut service = AppService::new(session.clone());
    assert_eq!(
        service.editor_preferences().theme,
        ThemePreference::Midnight
    );
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut service);
    let id = service.active.unwrap();
    let version = service.active_document().unwrap().version;
    service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection {
                anchor: 18,
                head: 7,
            },
        })
        .unwrap();
    service.dispatch(Command::CreateView).unwrap();
    let views: Vec<_> = service
        .document_views()
        .iter()
        .map(|view| (view.id, view.selection(), view.scroll()))
        .collect();
    for (theme, serialized) in [
        (ThemePreference::Classic, "classic"),
        (ThemePreference::Midnight, "midnight"),
    ] {
        service.dispatch(Command::SetTheme(theme)).unwrap();
        let doc = service.active_document().unwrap();
        assert_eq!(doc.text(), source);
        assert_eq!(doc.version, version);
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo() && !doc.can_redo());
        assert_eq!(
            views,
            service
                .document_views()
                .iter()
                .map(|view| (view.id, view.selection(), view.scroll()))
                .collect::<Vec<_>>()
        );
        service.dispatch(Command::PersistSession).unwrap();
        settle(&mut service);
        let saved: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&session).unwrap()).unwrap();
        assert_eq!(saved["editor_preferences"]["theme"], serialized);
        let mut restored = AppService::new(session.clone());
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert_eq!(restored.editor_preferences().theme, theme);
        assert_eq!(restored.active_document().unwrap().text(), source);
    }
    assert_eq!(fs::read_to_string(file).unwrap(), source);
}

#[test]
fn legacy_profiles_use_midnight_and_invalid_theme_preserves_original_profile() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("session.json");
    for version in [1, 2] {
        fs::write(&file, format!(r#"{{"schema_version":{version},"workspace":null,"tabs":[],"active":null,"editor_preferences":{{"ligatures":false}}}}"#)).unwrap();
        let mut service = AppService::new(file.clone());
        service.dispatch(Command::RestoreSession).unwrap();
        settle(&mut service);
        assert_eq!(
            service.editor_preferences().theme,
            ThemePreference::Midnight
        );
        assert!(!service.editor_preferences().ligatures);
    }
    let invalid = r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"editor_preferences":{"theme":"unsupported"}}"#;
    fs::write(&file, invalid).unwrap();
    let mut service = AppService::new(file.clone());
    service.dispatch(Command::RestoreSession).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.last_error.is_some());
    assert!(service.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(file).unwrap(), invalid);
}

#[test]
fn switching_theme_between_keystrokes_preserves_grouped_undo_and_redo() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("typing.php");
    fs::write(&file, "").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut service);
    let id = service.active.unwrap();
    for (position, text) in ["a", "b"].into_iter().enumerate() {
        if position == 1 {
            service
                .dispatch(Command::SetTheme(ThemePreference::Classic))
                .unwrap();
        }
        let doc = service.active_document().unwrap();
        service
            .dispatch(Command::EditTransaction {
                id,
                expected_version: doc.version,
                before: doc.selection(),
                after: Selection::caret(position + 1),
                delta: TextDelta {
                    range: position..position,
                    insert: text.into(),
                },
                kind: EditKind::Typing,
            })
            .unwrap();
    }
    service.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(service.active_document().unwrap().text(), "");
    service
        .dispatch(Command::SetTheme(ThemePreference::Midnight))
        .unwrap();
    service.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(service.active_document().unwrap().text(), "ab");
    assert_eq!(fs::read_to_string(file).unwrap(), "");
}
