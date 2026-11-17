use codisseu_core::{AppService, Command, EditKind, Selection, TextDelta};
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
fn wrap_defaults_on_persists_off_and_preserves_source_views_and_undo() {
    let dir = tempfile::tempdir().unwrap();
    let source = "\ttexto ação 🦀 e\u{301} sem quebra visual salva\r\nsegunda linha";
    let file = dir.path().join("long.md");
    let session = dir.path().join("session.json");
    fs::write(&file, source).unwrap();
    let mut service = AppService::new(session.clone());
    assert!(service.editor_preferences().soft_wrap);
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut service);
    service.dispatch(Command::CreateView).unwrap();
    let before: Vec<_> = service
        .document_views()
        .iter()
        .map(|view| (view.id, view.selection(), view.scroll()))
        .collect();
    let version = service.active_document().unwrap().version;
    for enabled in [false, true, false] {
        service
            .dispatch(Command::SetEditorSoftWrap(enabled))
            .unwrap();
    }
    let doc = service.active_document().unwrap();
    assert_eq!(doc.text(), source);
    assert_eq!(doc.version, version);
    assert!(!doc.can_undo() && !doc.is_dirty());
    assert_eq!(
        before,
        service
            .document_views()
            .iter()
            .map(|view| (view.id, view.selection(), view.scroll()))
            .collect::<Vec<_>>()
    );
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    drop(service);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert!(!restored.editor_preferences().soft_wrap);
    assert_eq!(restored.active_document().unwrap().text(), source);
    assert_eq!(fs::read_to_string(file).unwrap(), source);
}

#[test]
fn legacy_profiles_default_on_and_invalid_value_preserves_original_profile() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("session.json");
    for version in [1, 2] {
        fs::write(&file, format!(r#"{{"schema_version":{version},"workspace":null,"tabs":[],"active":null,"editor_preferences":{{"ligatures":false}}}}"#)).unwrap();
        let mut service = AppService::new(file.clone());
        service.dispatch(Command::RestoreSession).unwrap();
        settle(&mut service);
        assert!(service.editor_preferences().soft_wrap);
        assert!(!service.editor_preferences().ligatures);
    }
    let invalid = r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"editor_preferences":{"soft_wrap":"yes"}}"#;
    fs::write(&file, invalid).unwrap();
    let mut service = AppService::new(file.clone());
    service.dispatch(Command::RestoreSession).unwrap();
    while service.is_busy() {
        service.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.last_error.is_some());
    assert!(service.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(file).unwrap(), invalid);
}

#[test]
fn toggling_wrap_between_keystrokes_does_not_add_or_split_undo_steps() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.php");
    fs::write(&file, "").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut service);
    let id = service.active.unwrap();
    for (position, text) in ["a", "b"].into_iter().enumerate() {
        if position == 1 {
            service.dispatch(Command::SetEditorSoftWrap(false)).unwrap();
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
    assert_eq!(service.active_document().unwrap().text(), "ab");
    service.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(service.active_document().unwrap().text(), "");
    assert_eq!(fs::read_to_string(file).unwrap(), "");
}
