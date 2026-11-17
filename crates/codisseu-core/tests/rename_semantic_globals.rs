//! The independent PHP semantic provider works without inventing LSP responses.
use codisseu_core::{AppService, Command, Coverage};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(5);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < end, "{}", app.status);
        thread::sleep(Duration::from_millis(2));
    }
}
fn preview(app: &mut AppService, old: &str, new: &str) {
    let doc = app.active_document().unwrap();
    app.dispatch(Command::PrepareRename {
        id: doc.id,
        expected_version: doc.version,
        character_index: doc.text()[..doc.text().find(old).unwrap()].chars().count() + 1,
    })
    .unwrap();
    idle(app);
    assert!(app.rename.is_prepared(), "{:?}", app.rename.error);
    app.dispatch(Command::PreviewRename {
        new_name: new.into(),
    })
    .unwrap();
    idle(app);
}
#[test]
fn own_php_provider_preserves_dirty_closed_aliases_and_rejects_stale_preview() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = root.join("a.php");
    let b = root.join("b.php");
    fs::write(
        &a,
        "<?php namespace A; function rainfall(): int { return 3; }\n",
    )
    .unwrap();
    fs::write(&b, "<?php use function A\\rainfall as rain; echo rain();\n").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(a.clone())).unwrap();
    idle(&mut app);
    let d = app.active_document().unwrap();
    let id = d.id;
    let dirty = format!("{}// unsaved 🌧 é", d.text());
    app.dispatch(Command::Edit {
        id,
        expected_version: d.version,
        text: dirty.clone(),
    })
    .unwrap();
    preview(&mut app, "rainfall", "precipitation");
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified
    );
    assert_eq!(app.rename.files.len(), 2);
    fs::write(
        &b,
        "<?php use function A\\rainfall as rain; echo rain(); // external\n",
    )
    .unwrap();
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_some());
    assert_eq!(app.active_document().unwrap().text(), dirty);
    preview(&mut app, "rainfall", "precipitation");
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    assert!(
        app.documents
            .iter()
            .any(|d| d.path == b && d.text().contains("A\\precipitation as rain"))
    );
    assert!(fs::read_to_string(&a).unwrap().contains("rainfall"));
    assert!(fs::read_to_string(&b).unwrap().contains("rainfall"));
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), dirty);
    app.dispatch(Command::Redo { id }).unwrap();
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("precipitation")
    );
}
#[test]
fn own_php_provider_blocks_unsaved_declaration_collision_and_public_report_spoofing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("a.php");
    let source = "<?php namespace A; class Weather {} class Forecast {}\n";
    fs::write(&path, source).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    idle(&mut app);
    preview(&mut app, "Weather", "Forecast");
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Rejected
    );
    assert!(app.dispatch(Command::ApplyRename).is_err());
    app.rename.validation.as_mut().unwrap().coverage = Coverage::Verified;
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Rejected
    );
    assert_eq!(app.active_document().unwrap().text(), source);
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}
