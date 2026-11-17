use codisseu_core::{AppService, Command};
use codisseu_editor::{MAX_DOCUMENT_BYTES, search::MAX_SEARCH_MATCHES};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline, "worker timeout: {}", app.status);
        thread::sleep(Duration::from_millis(1));
    }
}

fn fixture(text: &str) -> (tempfile::TempDir, AppService, u64) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.ts");
    fs::write(&path, text).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    app.dispatch(Command::OpenFile(path)).unwrap();
    settle(&mut app);
    let id = app.active.unwrap();
    (dir, app, id)
}

fn find(app: &mut AppService, id: u64, query: &str) {
    let version = app.documents.iter().find(|d| d.id == id).unwrap().version;
    app.dispatch(Command::Find {
        id,
        expected_version: version,
        query: query.into(),
    })
    .unwrap();
}

fn replace(
    app: &mut AppService,
    id: u64,
    query: &str,
    replacement: &str,
    selection: Option<std::ops::Range<usize>>,
) {
    let version = app.documents.iter().find(|d| d.id == id).unwrap().version;
    app.dispatch(Command::ReplaceMatches {
        id,
        expected_version: version,
        query: query.into(),
        replacement: replacement.into(),
        selection,
    })
    .unwrap();
}

#[test]
fn unicode_find_replace_one_undo_redo_and_explicit_save() {
    let original = "\u{feff}// 😀 e\u{301}\r\nconst ação = 'ação';";
    let (dir, mut app, id) = fixture(original);
    find(&mut app, id, "ação");
    assert!(app.search_pending());
    settle(&mut app);
    let result = app.search.as_ref().unwrap();
    assert_eq!(result.document_id, id);
    assert_eq!(result.version, 1);
    assert_eq!(result.query, "ação");
    assert_eq!(result.ranges.len(), 2);
    assert!(!result.truncated);
    let selected = result.ranges[1].clone();
    let expected_start = original[..original.rfind("ação").unwrap()].chars().count();
    assert_eq!(selected, expected_start..expected_start + 4);
    replace(&mut app, id, "ação", "🌩️", Some(selected));
    settle(&mut app);
    let changed = "\u{feff}// 😀 e\u{301}\r\nconst ação = '🌩️';";
    assert_eq!(app.active_document().unwrap().text(), changed);
    assert_eq!(app.active_document().unwrap().version, 2);
    assert!(app.active_document().unwrap().is_dirty());
    assert_eq!(
        fs::read_to_string(dir.path().join("source.ts")).unwrap(),
        original
    );
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    assert!(!app.active_document().unwrap().can_undo());
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), changed);
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert_eq!(
        fs::read_to_string(dir.path().join("source.ts")).unwrap(),
        changed
    );
    assert!(!app.active_document().unwrap().is_dirty());
}

#[test]
fn replacement_all_exceeds_search_cap_but_has_one_undo_step() {
    let original = "a".repeat(MAX_SEARCH_MATCHES + 23);
    let (_dir, mut app, id) = fixture(&original);
    find(&mut app, id, "a");
    settle(&mut app);
    assert!(app.search.as_ref().unwrap().truncated);
    assert_eq!(
        app.search.as_ref().unwrap().ranges.len(),
        MAX_SEARCH_MATCHES
    );
    replace(&mut app, id, "a", "😀", None);
    settle(&mut app);
    assert_eq!(
        app.active_document().unwrap().text(),
        "😀".repeat(MAX_SEARCH_MATCHES + 23)
    );
    assert!(app.search.is_none());
    assert!(app.search_error.is_none());
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    assert!(!app.active_document().unwrap().can_undo());
}

#[test]
fn stale_find_and_replace_cannot_change_newer_edits() {
    let (_dir, mut app, id) = fixture("old old");
    find(&mut app, id, "old");
    replace(&mut app, id, "old", "wrong", None);
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "new 😀".into(),
    })
    .unwrap();
    settle(&mut app);
    assert!(app.search.is_none());
    assert!(app.search_error.as_ref().unwrap().contains("descartada"));
    assert_eq!(app.active_document().unwrap().text(), "new 😀");
    app.dispatch(Command::CancelSearch).unwrap();
    find(&mut app, id, "new");
    settle(&mut app);
    assert!(app.search_error.is_none());
    assert!(app.last_error.as_ref().unwrap().contains("descartada"));
    assert!(app.status.contains("descartada"));
    assert!(
        app.dispatch(Command::Find {
            id,
            expected_version: 1,
            query: "old".into()
        })
        .is_err()
    );
    assert!(
        app.dispatch(Command::ReplaceMatches {
            id,
            expected_version: 1,
            query: "old".into(),
            replacement: "wrong".into(),
            selection: None
        })
        .is_err()
    );
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "old old");
    assert!(!app.active_document().unwrap().can_undo());
}

#[test]
fn newest_search_wins_and_cancel_search_preserves_io_and_replacement() {
    let (dir, mut app, id) = fixture("first second first");
    find(&mut app, id, "first");
    find(&mut app, id, "second");
    settle(&mut app);
    assert_eq!(app.search.as_ref().unwrap().query, "second");
    assert_eq!(app.search.as_ref().unwrap().ranges, vec![6..12]);
    let other = dir.path().join("other.txt");
    fs::write(&other, "other").unwrap();
    find(&mut app, id, "first");
    replace(&mut app, id, "first", "done", None);
    app.dispatch(Command::OpenFile(other)).unwrap();
    app.dispatch(Command::CancelSearch).unwrap();
    assert!(
        app.search_pending(),
        "the accepted replacement is still pending"
    );
    settle(&mut app);
    assert!(app.search.is_none());
    assert!(app.search_error.is_none());
    assert_eq!(app.documents.len(), 2);
    assert_eq!(
        app.documents.iter().find(|d| d.id == id).unwrap().text(),
        "done second done"
    );
    assert!(!app.search_pending());
}

#[test]
fn cancel_all_and_closed_documents_never_apply_queued_replacements() {
    let (_dir, mut app, id) = fixture("original");
    replace(&mut app, id, "original", "wrong", None);
    app.dispatch(Command::CancelAll).unwrap();
    settle(&mut app);
    assert_eq!(app.active_document().unwrap().text(), "original");
    replace(&mut app, id, "original", "wrong", None);
    app.dispatch(Command::Close { id, discard: false }).unwrap();
    settle(&mut app);
    assert!(app.documents.is_empty());
    assert!(app.search_error.as_ref().unwrap().contains("fechado"));
}

#[test]
fn invalid_range_oversized_output_empty_query_and_noop_preserve_history() {
    let (_dir, mut app, id) = fixture("😀xxxxxxxxxxxxxxxxx");
    replace(&mut app, id, "xx", "z", Some(0..2));
    settle(&mut app);
    assert!(app.search_error.as_ref().unwrap().contains("coincide"));
    replace(&mut app, id, "x", &"z".repeat(MAX_DOCUMENT_BYTES), None);
    settle(&mut app);
    assert!(app.search_error.as_ref().unwrap().contains("16 MiB"));
    assert_eq!(app.active_document().unwrap().text(), "😀xxxxxxxxxxxxxxxxx");
    assert_eq!(app.active_document().unwrap().version, 1);
    assert!(!app.active_document().unwrap().can_undo());
    assert!(
        app.dispatch(Command::ReplaceMatches {
            id,
            expected_version: 1,
            query: String::new(),
            replacement: "z".into(),
            selection: None
        })
        .is_err()
    );
    find(&mut app, id, "");
    settle(&mut app);
    assert!(app.search.as_ref().unwrap().ranges.is_empty());
    assert!(app.search_error.is_none());
    find(&mut app, id, "x");
    settle(&mut app);
    replace(&mut app, id, "x", "x", None);
    settle(&mut app);
    assert_eq!(
        app.search.as_ref().unwrap().ranges,
        (1..18).map(|n| n..n + 1).collect::<Vec<_>>()
    );
    assert_eq!(app.active_document().unwrap().version, 1);
    assert!(!app.active_document().unwrap().can_undo());
}

#[test]
fn bounded_search_queue_rejection_preserves_latest_accepted_request() {
    let (_dir, mut app, id) = fixture("abcdef");
    for query in ["a", "b", "c", "d", "e", "f", "abc", "def"] {
        find(&mut app, id, query);
    }
    assert!(
        app.dispatch(Command::Find {
            id,
            expected_version: 1,
            query: "rejected".into()
        })
        .is_err()
    );
    settle(&mut app);
    assert_eq!(app.search.as_ref().unwrap().query, "def");
    assert_eq!(app.search.as_ref().unwrap().ranges, vec![3..6]);
}

#[test]
fn cancel_find_immediately_hides_results_and_waits_for_ack_without_repopulation() {
    let (_dir, mut app, id) = fixture(&"x".repeat(MAX_DOCUMENT_BYTES));
    find(&mut app, id, "never found");
    app.dispatch(Command::CancelSearch).unwrap();
    assert!(!app.search_pending());
    assert!(
        app.is_busy(),
        "cancelled snapshots still count until acknowledged"
    );
    settle(&mut app);
    assert!(app.search.is_none());
    assert!(app.search_error.is_none());
}
