use codisseu_core::{AppService, Command, Selection};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn settle(s: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
}
#[test]
fn editor_ligatures_default_on_persist_off_and_do_not_edit_views_or_history() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.ts");
    let session = dir.path().join("session.json");
    fs::write(&file, "// ffi -> e\u{301} 中文\r\n").unwrap();
    let mut s = AppService::new(session.clone());
    assert!(s.editor_preferences().ligatures);
    assert!(s.project_preferences().follow_active_file);
    s.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut s);
    let id = s.active.unwrap();
    let initial_version = s.active_document().unwrap().version;
    s.dispatch(Command::SetSelection {
        id,
        expected_version: initial_version,
        selection: Selection { anchor: 9, head: 3 },
    })
    .unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let before: Vec<_> = s
        .document_views()
        .iter()
        .map(|v| (v.id, v.selection()))
        .collect();
    let version = s.active_document().unwrap().version;
    let text = s.active_document().unwrap().text().to_owned();
    for enabled in [false, true, false] {
        s.dispatch(Command::SetEditorLigatures(enabled)).unwrap();
    }
    assert_eq!(
        before,
        s.document_views()
            .iter()
            .map(|v| (v.id, v.selection()))
            .collect::<Vec<_>>()
    );
    s.dispatch(Command::SetGitInsights {
        inline: false,
        hover: false,
        gutter: true,
    })
    .unwrap();
    s.dispatch(Command::SetProjectFollowActiveFile(false))
        .unwrap();
    let d = s.active_document().unwrap();
    assert_eq!(d.text(), text);
    assert_eq!(d.version, version);
    assert!(!d.is_dirty());
    assert!(!d.can_undo());
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    drop(s);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert!(!restored.editor_preferences().ligatures);
    assert!(!restored.project_preferences().follow_active_file);
    assert!(!restored.editor_preferences().git_current_line);
    assert!(!restored.editor_preferences().git_hover);
    assert!(restored.editor_preferences().git_file_blame);
    assert_eq!(restored.active_document().unwrap().text(), text);
    assert_eq!(fs::read_to_string(file).unwrap(), text);
}
#[test]
fn older_sessions_default_on_and_invalid_preferences_preserve_previous_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("session.json");
    for version in [1, 2] {
        fs::write(
            &p,
            format!(r#"{{"schema_version":{version},"workspace":null,"tabs":[],"active":null}}"#),
        )
        .unwrap();
        let mut s = AppService::new(p.clone());
        s.dispatch(Command::RestoreSession).unwrap();
        settle(&mut s);
        assert!(s.editor_preferences().ligatures);
        assert!(s.project_preferences().follow_active_file);
        assert!(s.editor_preferences().git_current_line);
        assert!(s.editor_preferences().git_hover);
        assert!(!s.editor_preferences().git_file_blame);
    }
    let invalid = r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"editor_preferences":{"ligatures":"off"}}"#;
    fs::write(&p, invalid).unwrap();
    let mut s = AppService::new(p.clone());
    s.dispatch(Command::RestoreSession).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(s.last_error.is_some());
    assert!(s.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(p).unwrap(), invalid);
}

#[test]
fn project_reveal_loading_and_preference_do_not_split_typing_undo_group() {
    use codisseu_core::{EditKind, TextDelta};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir_all(root.join("src/deep")).unwrap();
    let file = root.join("src/deep/a.php");
    fs::write(&file, "").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    settle(&mut s);
    s.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut s);
    let id = s.active.unwrap();
    for (offset, insert) in ["a", "b"].into_iter().enumerate() {
        if offset == 1 {
            s.dispatch(Command::SetCommitToolWindowVisible(true))
                .unwrap();
            s.dispatch(Command::SetProjectFollowActiveFile(false))
                .unwrap();
            s.dispatch(Command::LoadDirectory(root.join("src")))
                .unwrap();
            settle(&mut s);
        }
        let d = s.active_document().unwrap();
        s.dispatch(Command::EditTransaction {
            id,
            expected_version: d.version,
            before: d.selection(),
            after: Selection::caret(offset + 1),
            delta: TextDelta {
                range: offset..offset,
                insert: insert.into(),
            },
            kind: EditKind::Typing,
        })
        .unwrap();
    }
    assert_eq!(s.active_document().unwrap().text(), "ab");
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), "");
    assert_eq!(fs::read_to_string(file).unwrap(), "");
}

#[test]
fn commit_tool_window_visibility_persists_and_old_profiles_default_to_source_control() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    for old in [
        r#"{"schema_version":1,"workspace":null,"tabs":[],"active":null}"#,
        r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null}"#,
    ] {
        fs::write(&path, old).unwrap();
        let mut app = AppService::new(path.clone());
        app.dispatch(Command::RestoreSession).unwrap();
        settle(&mut app);
        assert!(!app.tool_window_preferences().commit);
    }
    for visible in [true, false] {
        let mut app = AppService::new(path.clone());
        app.dispatch(Command::RestoreSession).unwrap();
        settle(&mut app);
        app.dispatch(Command::SetCommitToolWindowVisible(visible))
            .unwrap();
        app.dispatch(Command::PersistSession).unwrap();
        settle(&mut app);
        drop(app);
        let mut restored = AppService::new(path.clone());
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert_eq!(restored.tool_window_preferences().commit, visible);
    }
    let invalid = r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"tool_window_preferences":{"commit":"false"}}"#;
    fs::write(&path, invalid).unwrap();
    let mut app = AppService::new(path.clone());
    app.dispatch(Command::RestoreSession).unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(app.last_error.is_some());
    assert!(app.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), invalid);
}

#[test]
fn git_action_confirmation_defaults_off_and_persists_in_both_directions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    fs::write(&path,r#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"tool_window_preferences":{"commit":true}}"#).unwrap();
    for enabled in [true, false] {
        let mut app = AppService::new(path.clone());
        app.dispatch(Command::RestoreSession).unwrap();
        settle(&mut app);
        assert!(!app.tool_window_preferences().git_confirm_actions || !enabled);
        app.dispatch(Command::SetGitConfirmActions(enabled))
            .unwrap();
        app.dispatch(Command::PersistSession).unwrap();
        settle(&mut app);
        drop(app);
        let mut restored = AppService::new(path.clone());
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert_eq!(
            restored.tool_window_preferences().git_confirm_actions,
            enabled
        );
        assert!(restored.tool_window_preferences().commit);
    }
}

#[test]
fn copilot_preferences_default_off_persist_without_editing_and_reject_bad_programs() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.json");
    let mut s = AppService::new(session.clone());
    settle(&mut s);
    assert!(!s.editor_preferences().copilot_enabled);
    assert!(s.editor_preferences().copilot_program.is_empty());
    for p in ["relative/server", "/owned/\0bad"] {
        assert!(s.dispatch(Command::SetCopilotProgram(p.into())).is_err());
    }
    s.dispatch(Command::SetCopilotProgram(
        "/owned/copilot-language-server".into(),
    ))
    .unwrap();
    s.dispatch(Command::SetCopilotEnabled(true)).unwrap();
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    drop(s);
    let mut s = AppService::new(session);
    s.dispatch(Command::RestoreSession).unwrap();
    settle(&mut s);
    assert!(s.editor_preferences().copilot_enabled);
    assert_eq!(
        s.editor_preferences().copilot_program,
        "/owned/copilot-language-server"
    );
    assert!(s.documents.is_empty());
}
