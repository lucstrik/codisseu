//! Real profile storage, real saves, restart and versioned editor restoration.
use codisseu_core::{AppService, Command};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(Instant::now() < end, "{:?}", app.last_error);
        thread::sleep(Duration::from_millis(2));
    }
}
fn open(app: &mut AppService, path: &Path) -> u64 {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    settle(app);
    app.active.unwrap()
}
fn edit(app: &mut AppService, id: u64, text: &str) {
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: text.into(),
    })
    .unwrap();
}
fn save(app: &mut AppService, id: u64) {
    app.dispatch(Command::Save { id }).unwrap();
    settle(app);
}
fn show(app: &mut AppService, id: u64) {
    app.dispatch(Command::ShowLocalHistory { id }).unwrap();
    settle(app);
    assert!(
        app.local_history.error.is_none(),
        "{:?}",
        app.local_history.error
    );
}
fn select(app: &mut AppService, id: u64) {
    app.dispatch(Command::SelectLocalRevision { revision: id })
        .unwrap();
    settle(app);
}

#[test]
fn history_persists_open_save_and_dirty_label_restores_with_undo_and_explicit_save() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("ação.php");
    let session = temp.path().join("profile/session.json");
    let a = "<?php\r\n$value = 'ação 😀';\r\n";
    let b = "<?php\r\n$value = 'salvo B';\r\n";
    let c = "<?php\r\n$value = 'checkpoint C';";
    fs::write(&source, a).unwrap();
    let mut app = AppService::new(session.clone());
    let id = open(&mut app, &source);
    edit(&mut app, id, b);
    save(&mut app, id);
    edit(&mut app, id, c);
    app.dispatch(Command::LabelLocalHistory {
        id,
        expected_version: app.active_document().unwrap().version,
        label: "Before experiment".into(),
    })
    .unwrap();
    settle(&mut app);
    assert_eq!(fs::read_to_string(&source).unwrap(), b);
    drop(app);
    let mut app = AppService::new(session);
    let id = open(&mut app, &source);
    show(&mut app, id);
    let items = &app.local_history.view.as_ref().unwrap().revisions;
    let oldest = items.last().unwrap().id;
    let label = items
        .iter()
        .find(|r| r.label.as_deref() == Some("Before experiment"))
        .unwrap()
        .id;
    assert!(!items.last().unwrap().date.is_empty());
    select(&mut app, oldest);
    assert_eq!(
        &*app
            .local_history
            .view
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .text,
        a
    );
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::RestoreLocalRevision {
        id,
        expected_version: version,
        revision: oldest,
    })
    .unwrap();
    assert_eq!(app.active_document().unwrap().text(), a);
    assert!(app.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(&source).unwrap(), b);
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), b);
    assert!(!app.active_document().unwrap().is_dirty());
    edit(&mut app, id, "unsaved must survive Undo");
    select(&mut app, label);
    app.dispatch(Command::RestoreLocalRevision {
        id,
        expected_version: app.active_document().unwrap().version,
        revision: label,
    })
    .unwrap();
    assert_eq!(app.active_document().unwrap().text(), c);
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(
        app.active_document().unwrap().text(),
        "unsaved must survive Undo"
    );
    app.dispatch(Command::Redo { id }).unwrap();
    save(&mut app, id);
    assert_eq!(fs::read_to_string(&source).unwrap(), c);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join("profile/session.history/history.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
    }
}
#[test]
fn history_rejects_other_paths_stale_versions_and_superseded_preview() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a.ts");
    let b = temp.path().join("b.php");
    fs::write(&a, "export const a=1;").unwrap();
    fs::write(&b, "<?php echo 2;").unwrap();
    let mut app = AppService::new(temp.path().join("profile.json"));
    let aid = open(&mut app, &a);
    edit(&mut app, aid, "export const a=3;");
    save(&mut app, aid);
    show(&mut app, aid);
    let items = app.local_history.view.as_ref().unwrap().revisions.clone();
    let first = items[0].id;
    let last = items.last().unwrap().id;
    app.dispatch(Command::SelectLocalRevision { revision: last })
        .unwrap();
    app.dispatch(Command::SelectLocalRevision { revision: first })
        .unwrap();
    settle(&mut app);
    assert_eq!(
        app.local_history
            .view
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .id,
        first
    );
    let version = app.active_document().unwrap().version;
    edit(&mut app, aid, "dirty");
    assert!(
        app.dispatch(Command::RestoreLocalRevision {
            id: aid,
            expected_version: version,
            revision: first
        })
        .is_err()
    );
    assert_eq!(app.active_document().unwrap().text(), "dirty");
    assert!(
        app.dispatch(Command::LabelLocalHistory {
            id: aid,
            expected_version: version,
            label: "stale".into()
        })
        .is_err()
    );
    let current = app.active_document().unwrap().version;
    assert!(
        app.dispatch(Command::LabelLocalHistory {
            id: aid,
            expected_version: current,
            label: "x".repeat(129)
        })
        .is_err()
    );
    assert!(
        app.dispatch(Command::LabelLocalHistory {
            id: aid,
            expected_version: current,
            label: "bad\nlabel".into()
        })
        .is_err()
    );
    let bid = open(&mut app, &b);
    let version = app.active_document().unwrap().version;
    assert!(
        app.dispatch(Command::RestoreLocalRevision {
            id: bid,
            expected_version: version,
            revision: first
        })
        .is_err()
    );
    show(&mut app, bid);
    assert!(
        app.dispatch(Command::SelectLocalRevision { revision: first })
            .is_err()
    );
    let r = app.local_history.view.as_ref().unwrap().revisions[0].id;
    app.dispatch(Command::SelectLocalRevision { revision: r })
        .unwrap();
    app.local_history.close();
    settle(&mut app);
    assert!(app.local_history.view.is_none());
    assert_eq!(fs::read_to_string(a).unwrap(), "export const a=3;");
    assert_eq!(fs::read_to_string(b).unwrap(), "<?php echo 2;");
}
#[test]
fn failed_save_never_claims_a_saved_revision() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("test.php");
    fs::write(&source, "original").unwrap();
    let mut app = AppService::new(temp.path().join("profile.json"));
    let id = open(&mut app, &source);
    edit(&mut app, id, "proposal");
    fs::write(&source, "external").unwrap();
    save(&mut app, id);
    assert!(app.last_error.is_some());
    assert_eq!(fs::read_to_string(&source).unwrap(), "external");
    show(&mut app, id);
    assert!(
        app.local_history
            .view
            .as_ref()
            .unwrap()
            .revisions
            .iter()
            .all(|r| r.reason != "Saved")
    );
    assert!(app.active_document().unwrap().is_dirty());
}
#[test]
fn unknown_history_store_is_preserved_and_does_not_prevent_save() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("profile.history");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("history.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TABLE sentinel(x); INSERT INTO sentinel VALUES(42); PRAGMA user_version=99;",
    )
    .unwrap();
    drop(db);
    let before = fs::read(&path).unwrap();
    let source = temp.path().join("test.php");
    fs::write(&source, "a").unwrap();
    let mut app = AppService::new(temp.path().join("profile.json"));
    let id = open(&mut app, &source);
    assert!(app.local_history.error.is_some());
    edit(&mut app, id, "b");
    save(&mut app, id);
    assert_eq!(fs::read_to_string(source).unwrap(), "b");
    assert_eq!(fs::read(path).unwrap(), before);
}
#[test]
fn consecutive_duplicates_are_deduplicated_and_file_retention_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("file.php");
    fs::write(&source, "0").unwrap();
    let mut app = AppService::new(temp.path().join("profile.json"));
    let id = open(&mut app, &source);
    for _ in 0..3 {
        save(&mut app, id);
    }
    show(&mut app, id);
    assert_eq!(app.local_history.view.as_ref().unwrap().revisions.len(), 1);
    app.local_history.close();
    for value in 1..39 {
        edit(&mut app, id, &value.to_string());
        save(&mut app, id);
    }
    show(&mut app, id);
    let view = app.local_history.view.as_ref().unwrap();
    assert_eq!(view.revisions.len(), 32);
    assert_eq!(&*view.preview.as_ref().unwrap().text, "38");
    let mut other = AppService::new(temp.path().join("other.json"));
    let id = open(&mut other, &source);
    show(&mut other, id);
    assert_eq!(
        other.local_history.view.as_ref().unwrap().revisions.len(),
        1
    );
}
#[test]
fn two_instances_share_a_profile_without_losing_manual_labels() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("file.php");
    fs::write(&source, "base").unwrap();
    let session = temp.path().join("profile.json");
    let mut a = AppService::new(session.clone());
    let mut b = AppService::new(session);
    let aid = open(&mut a, &source);
    let bid = open(&mut b, &source);
    a.dispatch(Command::LabelLocalHistory {
        id: aid,
        expected_version: a.active_document().unwrap().version,
        label: "instance A".into(),
    })
    .unwrap();
    b.dispatch(Command::LabelLocalHistory {
        id: bid,
        expected_version: b.active_document().unwrap().version,
        label: "instance B".into(),
    })
    .unwrap();
    settle(&mut a);
    settle(&mut b);
    show(&mut a, aid);
    let items = &a.local_history.view.as_ref().unwrap().revisions;
    assert!(
        items
            .iter()
            .any(|r| r.label.as_deref() == Some("instance A"))
    );
    assert!(
        items
            .iter()
            .any(|r| r.label.as_deref() == Some("instance B"))
    );
}
