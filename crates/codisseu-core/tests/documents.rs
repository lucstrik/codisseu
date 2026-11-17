use codisseu_core::{AppService, Command};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline, "worker timeout: {}", app.status);
        thread::sleep(Duration::from_millis(1));
    }
}

fn open(app: &mut AppService, path: &Path) -> u64 {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    settle(app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
    app.active.unwrap()
}

fn edit(app: &mut AppService, id: u64, text: &str) {
    let version = app.documents.iter().find(|d| d.id == id).unwrap().version;
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: text.into(),
    })
    .unwrap();
}

#[test]
fn real_file_open_edit_undo_redo_save_preserves_unicode_and_line_endings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.php");
    let original = "\u{feff}<?php\r\n// ação 😀 e\u{301}\r\necho 'Codisseu';";
    fs::write(&path, original).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    assert_eq!(app.active_document().unwrap().text(), original);
    edit(&mut app, id, &format!("{original}\r\n// editado"));
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Redo { id }).unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        format!("{original}\r\n// editado")
    );
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(app.active_document().unwrap().is_dirty());
}

#[test]
fn stale_edits_and_dirty_close_are_rejected_without_losing_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.ts");
    fs::write(&path, "const n = 1;").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "const n = 2;");
    assert!(
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "wrong".into()
        })
        .is_err()
    );
    assert!(app.dispatch(Command::Close { id, discard: false }).is_err());
    assert_eq!(app.active_document().unwrap().text(), "const n = 2;");
    app.dispatch(Command::Close { id, discard: true }).unwrap();
    assert!(app.documents.is_empty());
    assert_eq!(fs::read_to_string(path).unwrap(), "const n = 1;");
}

#[test]
fn external_conflict_preserves_both_disk_and_unsaved_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "original").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "local");
    fs::write(&path, "externo").unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.as_ref().unwrap().contains("Conflito"));
    assert_eq!(fs::read_to_string(path).unwrap(), "externo");
    assert_eq!(app.active_document().unwrap().text(), "local");
    assert!(app.active_document().unwrap().is_dirty());
}

#[test]
fn newer_edit_during_save_is_not_marked_clean() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.ts");
    fs::write(&path, "v1").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "v2");
    app.dispatch(Command::Save { id }).unwrap();
    edit(&mut app, id, "v3");
    settle(&mut app);
    assert!(app.last_error.is_none());
    assert_eq!(fs::read_to_string(path).unwrap(), "v2");
    assert_eq!(app.active_document().unwrap().text(), "v3");
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(!app.active_document().unwrap().is_dirty());
}

#[test]
fn session_restores_workspace_tabs_and_active_file() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("project");
    fs::create_dir(&workspace).unwrap();
    let php = workspace.join("index.php");
    let ts = workspace.join("app.ts");
    fs::write(&php, "<?php echo 1;").unwrap();
    fs::write(&ts, "const n: number = 1;").unwrap();
    let session = dir.path().join("config/session.json");
    let mut app = AppService::new(session.clone());
    app.dispatch(Command::OpenWorkspace(workspace.clone()))
        .unwrap();
    settle(&mut app);
    let php_id = open(&mut app, &php);
    open(&mut app, &ts);
    app.dispatch(Command::Activate { id: php_id }).unwrap();
    app.dispatch(Command::PersistSession).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none());
    drop(app);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert!(restored.last_error.is_none());
    assert_eq!(
        restored.workspace,
        Some(fs::canonicalize(workspace).unwrap())
    );
    assert_eq!(restored.documents.len(), 2);
    assert_eq!(
        restored.active_document().unwrap().path,
        fs::canonicalize(php).unwrap()
    );
    assert_eq!(restored.entries.len(), 2);
}

#[test]
fn malformed_session_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.json");
    fs::write(&session, "{broken JSON").unwrap();
    let mut app = AppService::new(session.clone());
    app.dispatch(Command::RestoreSession).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_some());
    assert!(app.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(session).unwrap(), "{broken JSON");
}

#[test]
fn profile_loading_allows_restore_but_accepted_datasource_commands_block_it() {
    use codisseu_db::remote::{ConnectionConfig, RemoteCommand};
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.json");
    fs::write(
        &session,
        r#"{"schema_version":1,"workspace":null,"tabs":[],"active":null}"#,
    )
    .unwrap();
    let mut app = AppService::new(session);
    assert!(app.remote_db.is_busy());
    assert!(!app.remote_db.has_pending_operation());
    app.dispatch(Command::RestoreSession).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none());
    app.remote_db
        .dispatch(RemoteCommand::Save(ConnectionConfig {
            id: "restore-guard".into(),
            ..Default::default()
        }))
        .unwrap();
    assert!(app.remote_db.has_pending_operation());
    assert!(app.dispatch(Command::RestoreSession).is_err());
    settle(&mut app);
    assert_eq!(app.remote_db.profiles[0].id, "restore-guard");
    assert!(app.remote_db.active.is_none());
}

#[test]
fn cancelled_restore_cannot_overwrite_the_unvalidated_session() {
    let dir = tempfile::tempdir().unwrap();
    let session = dir.path().join("session.json");
    let original = r#"{"schema_version":1,"workspace":null,"tabs":[],"active":null}"#;
    fs::write(&session, original).unwrap();
    let mut app = AppService::new(session.clone());
    app.dispatch(Command::RestoreSession).unwrap();
    app.dispatch(Command::CancelAll).unwrap();
    settle(&mut app);
    assert!(app.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read_to_string(session).unwrap(), original);
}

#[test]
fn cancelled_open_never_repopulates_the_documents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "real content").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenFile(path)).unwrap();
    app.dispatch(Command::CancelAll).unwrap();
    settle(&mut app);
    assert!(app.documents.is_empty());
    assert!(app.status.contains("cancelada"));
}

#[test]
fn queue_rejection_does_not_cancel_previously_accepted_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    fs::write(&path, "content").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    for _ in 0..32 {
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
    }
    assert!(
        app.dispatch(Command::OpenWorkspace(dir.path().into()))
            .is_err()
    );
    settle(&mut app);
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.active_document().unwrap().text(), "content");
}

#[cfg(unix)]
#[test]
fn all_5001_symlinks_are_visible_without_recursive_discovery() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("target");
    fs::write(&target, "outside").unwrap();
    for n in 0..5001 {
        symlink(&target, dir.path().join(format!("link-{n}"))).unwrap();
    }
    let mut app = AppService::new(outside.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut app);
    assert_eq!(app.entries.len(), 5001);
    assert!(!app.tree_truncated);
    assert_eq!(app.status, "Pasta aberta; nenhum script foi executado");
}

#[test]
fn rejects_invalid_utf8_oversized_files_and_directories() {
    let dir = tempfile::tempdir().unwrap();
    let invalid = dir.path().join("invalid.bin");
    let large = dir.path().join("large.txt");
    fs::write(&invalid, [0xff, 0xfe]).unwrap();
    fs::write(
        &large,
        vec![b'a'; codisseu_editor::MAX_EDITOR_DOCUMENT_BYTES + 1],
    )
    .unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    for path in [invalid, large, dir.path().into()] {
        app.dispatch(Command::OpenFile(path)).unwrap();
        settle(&mut app);
        assert!(app.last_error.is_some());
        assert!(app.documents.is_empty());
    }
}

#[test]
fn readonly_save_fails_without_touching_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("readonly.txt");
    fs::write(&path, "original").unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "changed");
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.as_ref().unwrap().contains("somente leitura"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    assert!(app.active_document().unwrap().is_dirty());
}

#[test]
fn dirty_workspace_switch_is_rejected_and_same_tree_refresh_keeps_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let other = tempfile::tempdir().unwrap();
    fs::write(&path, "original").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut app);
    let id = open(&mut app, &path);
    edit(&mut app, id, "changed");
    assert!(
        app.dispatch(Command::OpenWorkspace(other.path().into()))
            .is_err()
    );
    app.dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    settle(&mut app);
    assert_eq!(app.active_document().unwrap().text(), "changed");
    assert_eq!(app.active, Some(id));
}

#[cfg(unix)]
#[test]
fn replaced_symlink_target_is_never_overwritten() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.php");
    let target = dir.path().join("other.php");
    fs::write(&path, "original").unwrap();
    fs::write(&target, "original").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "change");
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_some());
    assert_eq!(fs::read_to_string(target).unwrap(), "original");
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

#[cfg(unix)]
#[test]
fn atomic_save_preserves_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("script.php");
    fs::write(&path, "original").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    edit(&mut app, id, "saved");
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none());
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[test]
fn incremental_typing_groups_but_save_navigation_and_atomic_edit_split_undo() {
    use codisseu_core::{EditKind, TextDelta};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.ts");
    fs::write(&p, "").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &p);
    for (i, c) in "a😀e\u{301}".chars().enumerate() {
        let version = app.active_document().unwrap().version;
        app.dispatch(Command::EditDelta {
            id,
            expected_version: version,
            delta: TextDelta {
                range: i..i,
                insert: c.to_string(),
            },
            kind: EditKind::Typing,
        })
        .unwrap();
    }
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert_eq!(fs::read_to_string(&p).unwrap(), "a😀e\u{301}");
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::EditDelta {
        id,
        expected_version: version,
        delta: TextDelta {
            range: 4..4,
            insert: "x".into(),
        },
        kind: EditKind::Typing,
    })
    .unwrap();
    assert!(
        app.dispatch(Command::EditDelta {
            id,
            expected_version: version,
            delta: TextDelta {
                range: 0..0,
                insert: "stale".into()
            },
            kind: EditKind::Typing
        })
        .is_err()
    );
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "a😀e\u{301}");
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "");
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "a😀e\u{301}");
    app.dispatch(Command::EndEditGroup { id }).unwrap();
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::EditDelta {
        id,
        expected_version: version,
        delta: TextDelta {
            range: 0..1,
            insert: "雨".into(),
        },
        kind: EditKind::Typing,
    })
    .unwrap();
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "a😀e\u{301}");
}

#[test]
fn directed_history_selection_and_versions_are_shared_by_typed_commands() {
    use codisseu_core::{EditKind, Selection, TextDelta};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("view.ts");
    fs::write(&p, "😀e\u{301} weather\r\n").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &p);
    let selected = Selection {
        anchor: 11,
        head: 4,
    };
    app.dispatch(Command::SetSelection {
        id,
        expected_version: 1,
        selection: selected,
    })
    .unwrap();
    assert_eq!(app.active_document().unwrap().version, 1);
    assert!(!app.active_document().unwrap().can_undo());
    app.dispatch(Command::EditTransaction {
        id,
        expected_version: 1,
        delta: TextDelta {
            range: 4..11,
            insert: "rain".into(),
        },
        kind: EditKind::Atomic,
        before: selected,
        after: Selection::caret(8),
    })
    .unwrap();
    assert!(
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 1,
            selection: Selection::caret(0)
        })
        .is_err()
    );
    app.dispatch(Command::SetSelection {
        id,
        expected_version: 2,
        selection: Selection::caret(0),
    })
    .unwrap();
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(
        app.active_document().unwrap().text(),
        "😀e\u{301} weather\r\n"
    );
    assert_eq!(app.active_document().unwrap().selection(), selected);
    app.dispatch(Command::SetSelection {
        id,
        expected_version: 3,
        selection: Selection::caret(1),
    })
    .unwrap();
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "😀e\u{301} rain\r\n");
    assert_eq!(
        app.active_document().unwrap().selection(),
        Selection::caret(8)
    );
    assert_eq!(fs::read_to_string(&p).unwrap(), "😀e\u{301} weather\r\n");
    assert!(
        app.dispatch(Command::EditTransaction {
            id,
            expected_version: 4,
            delta: TextDelta {
                range: 0..0,
                insert: "!".into()
            },
            kind: EditKind::Atomic,
            before: Selection::caret(99),
            after: Selection::caret(1)
        })
        .is_err()
    );
    assert_eq!(app.active_document().unwrap().version, 4);
    assert_eq!(
        app.active_document().unwrap().selection(),
        Selection::caret(8)
    );
}

#[test]
fn dirty_identity_and_exact_comparison_track_manual_reversion_save_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dirty-unicode.txt");
    fs::write(&path, "a😀\r\nb").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    let initial = app.active_document().unwrap().snapshot();
    edit(&mut app, id, "a雨\r\nb");
    for _ in 0..50 {
        assert!(app.active_document().unwrap().is_dirty());
    }
    edit(&mut app, id, "a😀\r\nb");
    for _ in 0..50 {
        assert!(!app.active_document().unwrap().is_dirty());
    }
    assert_eq!(app.active_document().unwrap().snapshot(), initial);
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Redo { id }).unwrap();
    assert!(!app.active_document().unwrap().is_dirty());
}

#[test]
fn large_editor_files_open_edit_undo_save_and_reopen_with_semantics_explicitly_limited() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.ts");
    let size = codisseu_editor::MAX_EDITOR_DOCUMENT_BYTES;
    fs::write(&path, "a".repeat(size)).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &path);
    let doc = app.active_document().unwrap();
    assert_eq!(doc.len_bytes(), size);
    assert!(!doc.language_supported());
    assert!(!doc.is_dirty());
    for command in [
        Command::Hover {
            id,
            expected_version: 1,
            character_index: 0,
        },
        Command::Definition {
            id,
            expected_version: 1,
            character_index: 0,
        },
        Command::References {
            id,
            expected_version: 1,
            character_index: 0,
        },
        Command::Completion {
            id,
            expected_version: 1,
            character_index: 0,
        },
        Command::PrepareRename {
            id,
            expected_version: 1,
            character_index: 0,
        },
    ] {
        assert!(app.dispatch(command).unwrap_err().contains("1 MiB"));
    }
    app.last_error = None;
    app.dispatch(Command::EditDelta {
        id,
        expected_version: 1,
        delta: codisseu_core::TextDelta {
            range: size - 1..size,
            insert: "z".into(),
        },
        kind: codisseu_core::EditKind::Atomic,
    })
    .unwrap();
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Redo { id }).unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Close { id, discard: false }).unwrap();
    open(&mut app, &path);
    let doc = app.active_document().unwrap();
    assert_eq!(doc.len_bytes(), size);
    assert_eq!(doc.snapshot().char(size - 1), 'z');
}

#[test]
fn line_limit_open_and_paste_fail_atomically_while_exact_limit_remains_editable() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.txt");
    let oversized = dir.path().join("too-many-lines.txt");
    fs::write(&original, "saved").unwrap();
    fs::write(
        &oversized,
        "\n".repeat(codisseu_core::MAX_EDITOR_DOCUMENT_LINES),
    )
    .unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    let id = open(&mut app, &original);
    edit(&mut app, id, "unsaved");
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::OpenFile(oversized.clone())).unwrap();
    settle(&mut app);
    assert!(
        app.last_error
            .as_ref()
            .unwrap()
            .contains("1.000.000 linhas")
    );
    assert_eq!(app.active, Some(id));
    assert_eq!(app.documents.len(), 1);
    assert_eq!(app.active_document().unwrap().version, version);
    assert_eq!(app.active_document().unwrap().snapshot(), "unsaved");
    assert!(
        app.dispatch(Command::EditTransaction {
            id,
            expected_version: version,
            delta: codisseu_core::TextDelta {
                range: 0..7,
                insert: "\n".repeat(codisseu_core::MAX_EDITOR_DOCUMENT_LINES)
            },
            kind: codisseu_core::EditKind::Atomic,
            before: codisseu_core::Selection::caret(0),
            after: codisseu_core::Selection::caret(codisseu_core::MAX_EDITOR_DOCUMENT_LINES)
        })
        .unwrap_err()
        .contains("1.000.000 linhas")
    );
    assert_eq!(app.active_document().unwrap().version, version);
    assert_eq!(app.active_document().unwrap().snapshot(), "unsaved");
    app.dispatch(Command::Undo { id }).unwrap();
    assert!(!app.active_document().unwrap().is_dirty());
    assert_eq!(app.active_document().unwrap().snapshot(), "saved");
    fs::write(
        &oversized,
        "\r\n".repeat(codisseu_core::MAX_EDITOR_DOCUMENT_LINES - 1),
    )
    .unwrap();
    app.last_error = None;
    let exact = open(&mut app, &oversized);
    assert_ne!(exact, id);
    assert_eq!(
        app.active_document().unwrap().line_count(),
        codisseu_core::MAX_EDITOR_DOCUMENT_LINES
    );
    assert_eq!(fs::read_to_string(original).unwrap(), "saved");
}

#[test]
fn create_file_is_exclusive_handles_relative_workspace_paths_and_preserves_existing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    settle(&mut app);
    app.dispatch(Command::CreateFile("notes.md".into()))
        .unwrap();
    settle(&mut app);
    let path = root.join("notes.md");
    assert_eq!(app.active_document().unwrap().path, path);
    assert_eq!(app.active_document().unwrap().text(), "");
    assert_eq!(fs::read(&path).unwrap(), b"");
    assert!(app.entries.iter().any(|entry| entry.path == path));
    let id = app.active.unwrap();
    edit(&mut app, id, "# Ação 🌧\r\n");
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    app.dispatch(Command::CreateFile(path.clone())).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), "# Ação 🌧\r\n");
    assert_eq!(app.documents.len(), 1);
    app.last_error = None;
    app.dispatch(Command::CreateFile(root.join("missing/notes.md")))
        .unwrap();
    settle(&mut app);
    assert!(app.last_error.is_some());
    assert!(!root.join("missing").exists());
    #[cfg(unix)]
    {
        let link = root.join("link.md");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        app.last_error = None;
        app.dispatch(Command::CreateFile(link.clone())).unwrap();
        settle(&mut app);
        assert!(app.last_error.is_some());
        assert!(link.is_symlink());
        assert_eq!(fs::read_to_string(path).unwrap(), "# Ação 🌧\r\n");
    }
}
