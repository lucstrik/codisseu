use super::*;
use std::{fs, thread};
fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(Instant::now() < deadline, "{}", app.status);
        thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, AppService, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("Weather.php");
    let target = root.join("Forecast.php");
    fs::write(&source, "<?php\nclass Weather {}\n// Weather\n").unwrap();
    fs::write(
        root.join("main.php"),
        "<?php\nrequire_once __DIR__ . '/Weather.php';\nnew Weather();\n",
    )
    .unwrap();
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(source.clone())).unwrap();
    idle(&mut app);
    (dir, app, source, target)
}
fn preview(app: &mut AppService, options: RenameOptions) {
    let doc = app.active_document().unwrap();
    let id = doc.id;
    let version = doc.version;
    let character_index = doc.text().find("Weather").unwrap();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: version,
        character_index,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.prepared, "{:?} {}", app.rename.error, app.status);
    app.dispatch(Command::SetRenameOptions { options }).unwrap();
    app.dispatch(Command::PreviewRename {
        new_name: "Forecast".into(),
    })
    .unwrap();
    idle(app);
    if app.rename.choosing_inheritors() {
        app.dispatch(Command::ConfirmRenameInheritors {
            token: app.rename.context_token().unwrap(),
        })
        .unwrap();
        idle(app);
    }
}
#[test]
fn dirty_class_file_imports_paths_and_saved_bytes_roundtrip_with_save() {
    let (_dir, mut app, source, target) = fixture();
    let original = fs::read(&source).unwrap();
    let id = app.active_document().unwrap().id;
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "<?php\nclass Weather {}\n// Weather dirty 😀\n".into(),
    })
    .unwrap();
    preview(&mut app, RenameOptions::default());
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    assert_eq!(
        app.rename.resource_moves(),
        &[resources::ResourceLocation {
            source: source.clone(),
            target: target.clone()
        }]
    );
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.path.ends_with("main.php") && f.after.contains("'/Forecast.php'"))
    );
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(app.active_document().unwrap().id, id);
    assert_eq!(app.active_document().unwrap().path, target);
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Forecast")
    );
    assert!(app.active_document().unwrap().text().contains("dirty 😀"));
    assert_eq!(fs::read(&target).unwrap(), original);
    assert!(!source.exists());
    assert!(app.active_document().unwrap().is_dirty());
    assert!(app.documents.iter().any(|d| d.path.ends_with("main.php")
        && d.text().contains("new Forecast()")
        && d.is_dirty()));
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert_eq!(app.active_document().unwrap().path, source);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Weather")
    );
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    app.dispatch(Command::Save { id }).unwrap();
    idle(&mut app);
    let saved = fs::read(&target).unwrap();
    assert_ne!(saved, original);
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert_eq!(fs::read(&source).unwrap(), saved);
    assert!(!target.exists());
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    assert_eq!(fs::read(&target).unwrap(), saved);
    assert!(!app.active_document().unwrap().is_dirty());
}
#[test]
fn preview_cancel_and_late_destination_leave_buffers_and_disk_untouched() {
    let (_dir, mut app, source, target) = fixture();
    let before = app.active_document().unwrap().text().to_owned();
    preview(&mut app, RenameOptions::default());
    app.dispatch(Command::CancelRename).unwrap();
    idle(&mut app);
    assert!(source.exists());
    assert!(!target.exists());
    assert_eq!(app.active_document().unwrap().text(), before);
    preview(&mut app, RenameOptions::default());
    fs::write(&target, "external writer").unwrap();
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_some());
    assert_eq!(fs::read_to_string(&target).unwrap(), "external writer");
    assert_eq!(app.active_document().unwrap().path, source);
    assert_eq!(app.active_document().unwrap().text(), before);
}
#[test]
fn resource_commit_freezes_cancel_edit_close_and_keeps_ack_after_context_cancel() {
    let (_dir, mut app, source, target) = fixture();
    preview(&mut app, RenameOptions::default());
    let c = app.rename.context.as_ref().unwrap();
    let stage = app.stage_rename(&c.proposal, &c.resources).unwrap();
    let moves = c.resources.clone();
    app.queue_resource_stage(stage, moves).unwrap();
    let id = app.active_document().unwrap().id;
    assert!(app.dispatch(Command::CancelAll).is_err());
    assert!(app.dispatch(Command::CancelRename).is_err());
    assert!(
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "oops".into()
        })
        .is_err()
    );
    assert!(app.dispatch(Command::Close { id, discard: true }).is_err());
    app.cancel_rename(); // e.g. independent provider failure must not discard ACK.
    idle(&mut app);
    assert!(!source.exists());
    assert!(target.exists());
    assert_eq!(app.active_document().unwrap().path, target);
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Forecast")
    );
}
#[test]
fn options_include_dirty_text_and_keep_semantic_scope_complete() {
    let (dir, mut app, source, _) = fixture();
    let text = dir.path().join("notes.md");
    fs::write(&text, "Weather disk").unwrap();
    let id = app.active_document().unwrap().id;
    app.dispatch(Command::OpenFile(text.clone())).unwrap();
    idle(&mut app);
    let text_id = app.active_document().unwrap().id;
    app.dispatch(Command::Edit {
        id: text_id,
        expected_version: 1,
        text: "Weather dirty".into(),
    })
    .unwrap();
    app.dispatch(Command::Activate { id }).unwrap();
    preview(
        &mut app,
        RenameOptions {
            search_comments_strings: true,
            search_text: true,
            scope: RenameSearchScope::Project,
            ..RenameOptions::default()
        },
    );
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.path == source && f.after.contains("// Forecast"))
    );
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.path == text && f.after.as_ref() == "Forecast dirty")
    );
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(
        app.documents
            .iter()
            .find(|d| d.id == text_id)
            .unwrap()
            .text(),
        "Forecast dirty"
    );
    assert_eq!(fs::read_to_string(text).unwrap(), "Weather disk");
}
#[test]
fn undo_resource_collision_does_not_partially_undo_buffers() {
    let (_dir, mut app, source, target) = fixture();
    preview(&mut app, RenameOptions::default());
    let id = app.active_document().unwrap().id;
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    let after = app.active_document().unwrap().text().to_owned();
    fs::write(&source, "occupied").unwrap();
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert_eq!(app.active_document().unwrap().text(), after);
    assert_eq!(app.active_document().unwrap().path, target);
    assert_eq!(fs::read_to_string(source).unwrap(), "occupied");
    assert!(app.rename.error.is_some());
}

#[test]
fn externally_changed_saved_resource_and_text_manifest_abort_before_commit() {
    for mode in 0..3 {
        let (dir, mut app, source, target) = fixture();
        let notes = dir.path().join("notes.md");
        fs::write(&notes, "Weather notes").unwrap();
        preview(
            &mut app,
            RenameOptions {
                search_text: true,
                ..RenameOptions::default()
            },
        );
        assert!(app.rename.can_apply(), "{:?}", app.rename.error);
        let before = app.active_document().unwrap().text().to_owned();
        match mode {
            0 => fs::write(&source, "<?php class Weather {} // external").unwrap(),
            1 => fs::write(&notes, "Weather external").unwrap(),
            _ => fs::write(dir.path().join("new.md"), "Weather added").unwrap(),
        }
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(
            !app.rename.can_apply()
                && (app.rename.error.is_some() || app.rename.semantic_blocked()),
            "case {mode}"
        );
        assert_eq!(app.active_document().unwrap().text(), before);
        assert_eq!(app.active_document().unwrap().path, source);
        assert!(!target.exists());
    }
}

#[test]
fn search_scope_limits_optional_occurrences_but_includes_all_semantic_references() {
    let (dir, mut app, source, _) = fixture();
    fs::write(dir.path().join("notes.md"), "Weather notes").unwrap();
    let main = dir.path().join("main.php");
    fs::write(
        &main,
        "<?php\nrequire_once __DIR__ . '/Weather.php';\nnew Weather(); // Weather\n",
    )
    .unwrap();
    preview(
        &mut app,
        RenameOptions {
            search_comments_strings: true,
            search_text: true,
            scope: RenameSearchScope::CurrentFile,
            ..RenameOptions::default()
        },
    );
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    assert_eq!(app.rename.files.len(), 2);
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.path == source && f.after.contains("// Forecast"))
    );
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.path == main && f.after.contains("new Forecast(); // Weather"))
    );
    // Editing options invalidates the previously applicable private proposal.
    app.dispatch(Command::SetRenameOptions {
        options: RenameOptions::default(),
    })
    .unwrap();
    assert!(!app.rename.can_apply());
    assert!(app.dispatch(Command::ApplyRename).is_err());
}

#[test]
fn closed_class_resource_from_reference_keeps_stable_reference_id_and_undo() {
    let (dir, mut app, source, target) = fixture();
    let source_id = app.active_document().unwrap().id;
    app.dispatch(Command::Close {
        id: source_id,
        discard: false,
    })
    .unwrap();
    let main = dir.path().join("main.php");
    app.dispatch(Command::OpenFile(main.clone())).unwrap();
    idle(&mut app);
    let doc = app.active_document().unwrap();
    let id = doc.id;
    let version = doc.version;
    let character_index = doc.text().rfind("Weather").unwrap();
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: version,
        character_index,
    })
    .unwrap();
    idle(&mut app);
    assert!(app.rename.prepared, "{}", app.status);
    app.dispatch(Command::PreviewRename {
        new_name: "Forecast".into(),
    })
    .unwrap();
    idle(&mut app);
    if app.rename.choosing_inheritors() {
        app.dispatch(Command::ConfirmRenameInheritors {
            token: app.rename.context_token().unwrap(),
        })
        .unwrap();
        idle(&mut app);
    }
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(app.active_document().unwrap().id, id);
    assert_eq!(app.active_document().unwrap().path, main);
    assert!(
        app.documents
            .iter()
            .any(|d| d.path == target && d.text().contains("class Forecast"))
    );
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert!(source.exists());
    assert!(!target.exists());
    assert!(
        app.documents
            .iter()
            .any(|d| d.path == source && d.text().contains("class Weather"))
    );
}

#[test]
fn already_accepted_search_replacement_finishes_before_any_resource_stage() {
    let (_dir, mut app, source, target) = fixture();
    preview(&mut app, RenameOptions::default());
    let doc = app.active_document().unwrap();
    let id = doc.id;
    let expected_version = doc.version;
    app.dispatch(Command::ReplaceMatches {
        id,
        expected_version,
        query: "Weather".into(),
        replacement: "Other".into(),
        selection: None,
    })
    .unwrap();
    let c = app.rename.context.as_ref().unwrap();
    let moves = c.resources.clone();
    let stage = app.stage_rename(&c.proposal, &moves).unwrap();
    assert!(app.queue_resource_stage(stage, moves).is_err());
    assert!(app.rename_stage.is_none());
    idle(&mut app);
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Other")
    );
    assert_eq!(app.active_document().unwrap().path, source);
    assert!(!target.exists());
}

#[test]
fn accessor_conflicts_keep_preview_context_dirty_buffer_and_resist_public_report_spoof() {
    let (_dir, mut app, source, _) = fixture();
    let id = app.active_document().unwrap().id;
    let text = "<?php\nclass Weather { public int $weather=0; public function getWeather(): int { return $this->weather; } public function getForecast(): int { return 5; } }\n";
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: text.into(),
    })
    .unwrap();
    let changes = || {
        vec![codisseu_language::RenameFileEdits {
            path: source.clone(),
            version: Some(2),
            edits: text
                .match_indices("weather")
                .map(|(start, _)| {
                    let prefix = &text[..start];
                    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
                    let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
                    RenameTextEdit {
                        start_line: line,
                        start_character: col,
                        end_line: line,
                        end_character: col + 7,
                        new_text: "forecast".into(),
                    }
                })
                .collect(),
        }]
    };
    super::tests::incoming(&mut app, changes());
    idle(&mut app);
    assert!(app.rename.context.is_some());
    assert!(!app.rename.can_apply());
    assert!(app.rename.error.is_none());
    assert!(!app.rename.files.is_empty());
    let report = app.rename.validation.as_ref().unwrap();
    assert_eq!(report.coverage, Coverage::Rejected);
    assert!(
        report
            .conflicts
            .iter()
            .any(|c| c.message.contains("Accessor") && c.path == source)
    );
    assert!(
        app.rename
            .conflict_previews
            .iter()
            .any(|p| p.path == source && p.before.as_ref() == text)
    );
    assert_eq!(app.active_document().unwrap().text(), text);
    assert!(app.active_document().unwrap().is_dirty());
    app.rename.validation.as_mut().unwrap().coverage = Coverage::Verified;
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Rejected
    );
    assert!(!app.rename.can_apply());
    assert_eq!(app.active_document().unwrap().text(), text);
    assert!(app.rename.context.is_some());
    // The same Prepared/Preview transition used by Alterar nome remains usable.
    app.dispatch(Command::SetRenameOptions {
        options: RenameOptions {
            rename_accessors: false,
            ..RenameOptions::default()
        },
    })
    .unwrap();
    assert!(app.rename.context.as_ref().unwrap().phase == Phase::Prepared);
    super::tests::incoming(&mut app, changes());
    idle(&mut app);
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert!(
        app.rename
            .files
            .iter()
            .any(|f| f.after.contains("function getWeather()") && f.after.contains("$forecast"))
    );
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "<?php\nclass Weather {}\n// Weather\n"
    );
}
