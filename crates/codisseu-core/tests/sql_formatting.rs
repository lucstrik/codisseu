use codisseu_core::{AppService, Command, Selection};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end, "{}", s.status);
        thread::sleep(Duration::from_millis(2));
    }
}
fn fixture(text: &str) -> (tempfile::TempDir, AppService) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("query.sql");
    fs::write(&p, text).unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::OpenFile(p)).unwrap();
    wait(&mut s);
    (dir, s)
}
fn format(s: &mut AppService, range: Option<std::ops::Range<usize>>) {
    let d = s.active_document().unwrap();
    let cmd = Command::FormatDocument {
        id: d.id,
        expected_version: d.version,
        range,
    };
    s.dispatch(cmd).unwrap();
}
#[test]
fn offline_formatting_unicode_crlf_selection_undo_redo_and_disk() {
    let original = "-- ação 😀\r\nselect id,name from users where id=1;\r\nselect 2;\r\n";
    let (dir, mut s) = fixture(original);
    let d = s.active_document().unwrap();
    let id = d.id;
    let start = original[..original.find("select id").unwrap()]
        .chars()
        .count();
    let end = original[..original.find("\r\nselect 2").unwrap()]
        .chars()
        .count();
    let before = Selection {
        anchor: end,
        head: start,
    };
    s.dispatch(Command::SetSelection {
        id,
        expected_version: d.version,
        selection: before,
    })
    .unwrap();
    format(&mut s, Some(start..end));
    format(&mut s, Some(start..end));
    wait(&mut s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    let expected = s.active_document().unwrap().text().to_owned();
    assert_ne!(expected, original);
    assert!(expected.starts_with("-- ação 😀\r\n"));
    assert!(expected.ends_with(";\r\nselect 2;\r\n"));
    assert!(!expected.replace("\r\n", "").contains('\n'));
    assert_eq!(
        fs::read_to_string(dir.path().join("query.sql")).unwrap(),
        original
    );
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), original);
    assert_eq!(s.active_document().unwrap().selection(), before);
    s.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), expected);
}
#[test]
fn stale_edit_cancel_invalid_sql_and_version_are_safe() {
    for cancel in [true, false] {
        let original = "select a,b from t where id=1;";
        let (_dir, mut s) = fixture(original);
        format(&mut s, None);
        let d = s.active_document().unwrap();
        let id = d.id;
        let version = d.version;
        if cancel {
            s.dispatch(Command::CancelFormatting).unwrap();
        } else {
            s.dispatch(Command::Edit {
                id,
                expected_version: version,
                text: "select 99;".into(),
            })
            .unwrap();
        }
        wait(&mut s);
        assert_eq!(
            s.active_document().unwrap().text(),
            if cancel { original } else { "select 99;" }
        );
    }
    let (_dir, mut s) = fixture("select 'unfinished");
    format(&mut s, None);
    wait(&mut s);
    assert!(s.formatting.error.is_some());
    assert_eq!(s.active_document().unwrap().text(), "select 'unfinished");
    let d = s.active_document().unwrap();
    assert!(
        s.dispatch(Command::FormatDocument {
            id: d.id,
            expected_version: d.version + 1,
            range: None
        })
        .is_err()
    );
}

#[test]
fn multiple_statements_in_selection_keep_surroundings_and_single_undo() {
    let original = "-- ação 😀\r\nselect 0;\r\nselect 1;select 2;select 3;\r\nselect 4;\r\n";
    let (dir, mut s) = fixture(original);
    let start = original[..original.find("select 1").unwrap()]
        .chars()
        .count();
    let end = original[..original.find("\r\nselect 4").unwrap()]
        .chars()
        .count();
    let d = s.active_document().unwrap();
    let id = d.id;
    let before = Selection {
        anchor: end,
        head: start,
    };
    s.dispatch(Command::SetSelection {
        id,
        expected_version: d.version,
        selection: before,
    })
    .unwrap();
    format(&mut s, Some(start..end));
    wait(&mut s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    let expected = "-- ação 😀\r\nselect 0;\r\nselect\r\n  1;\r\n\r\nselect\r\n  2;\r\n\r\nselect\r\n  3;\r\nselect 4;\r\n";
    assert_eq!(s.active_document().unwrap().text(), expected);
    assert_eq!(
        fs::read_to_string(dir.path().join("query.sql")).unwrap(),
        original
    );
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), original);
    assert_eq!(s.active_document().unwrap().selection(), before);
    s.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), expected);
}
