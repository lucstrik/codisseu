use codisseu_core::{AppService, Command, Selection};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end, "{}", s.status);
        thread::sleep(Duration::from_millis(5));
    }
}
fn open(s: &mut AppService, path: &std::path::Path) {
    s.dispatch(Command::OpenFile(path.into())).unwrap();
    idle(s);
}
fn format(s: &mut AppService) {
    let d = s.active_document().unwrap();
    let cmd = Command::FormatDocument {
        id: d.id,
        expected_version: d.version,
        range: None,
    };
    s.dispatch(cmd).unwrap();
}
#[test]
fn real_php_only_active_dirty_buffer_no_execution_no_automatic_formatting_and_undo() {
    let temp = tempfile::tempdir().unwrap();
    let p = temp.path().join("active.php");
    let other = temp.path().join("other.php");
    let code = "<?php\r\n// ação 😀\r\nfile_put_contents(__DIR__.'/EXECUTED','bad'); class A{public function f(?string $x):string{return $x.'a  b';}}\r\n";
    let code = code.replace(
        "__DIR__.'/EXECUTED'",
        &format!("'{}'", temp.path().join("EXECUTED").display()),
    );
    fs::write(&p, &code).unwrap();
    fs::write(&other, "<?php $x=1;").unwrap();
    fs::write(
        temp.path().join(".php-cs-fixer.php"),
        "<?php file_put_contents(__DIR__.'/CONFIG_LOADED','bad');",
    )
    .unwrap();
    let mut s = AppService::new(temp.path().join("session.json"));
    open(&mut s, &other);
    let d = s.active_document().unwrap();
    let other_id = d.id;
    s.dispatch(Command::Edit {
        id: other_id,
        expected_version: d.version,
        text: "<?php $x=2;".into(),
    })
    .unwrap();
    open(&mut s, &p);
    assert_eq!(s.active_document().unwrap().text(), code);
    let d = s.active_document().unwrap();
    let id = d.id;
    let changed = code.replace("a  b", "dirty  literal");
    s.dispatch(Command::Edit {
        id,
        expected_version: d.version,
        text: changed.clone(),
    })
    .unwrap();
    s.dispatch(Command::Save { id }).unwrap();
    idle(&mut s);
    assert_eq!(s.active_document().unwrap().text(), changed);
    let before = Selection {
        anchor: 12,
        head: 8,
    };
    let d = s.active_document().unwrap();
    s.dispatch(Command::SetSelection {
        id,
        expected_version: d.version,
        selection: before,
    })
    .unwrap();
    format(&mut s);
    idle(&mut s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    let formatted = s.active_document().unwrap().text().to_owned();
    assert_ne!(formatted, changed);
    assert!(formatted.contains("dirty  literal"));
    assert!(formatted.contains("    public function"));
    assert!(!formatted.replace("\r\n", "").contains('\n'));
    assert_eq!(fs::read_to_string(&p).unwrap(), changed);
    assert_eq!(fs::read_to_string(&other).unwrap(), "<?php $x=1;");
    assert_eq!(
        s.documents
            .iter()
            .find(|d| d.id == other_id)
            .unwrap()
            .text(),
        "<?php $x=2;"
    );
    assert!(!temp.path().join("EXECUTED").exists());
    assert!(!temp.path().join("CONFIG_LOADED").exists());
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), changed);
    assert_eq!(s.active_document().unwrap().selection(), before);
    s.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), formatted);
    format(&mut s);
    idle(&mut s);
    assert_eq!(s.active_document().unwrap().text(), formatted);
}
#[test]
fn invalid_php_cancel_and_changed_version_do_not_apply_results() {
    for mode in 0..3 {
        let invalid = mode == 0;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("file.php");
        let source = if invalid {
            "<?php function broken( {"
        } else {
            "<?php class A{function f(){return 1;}}"
        };
        fs::write(&path, source).unwrap();
        let mut s = AppService::new(temp.path().join("session.json"));
        open(&mut s, &path);
        format(&mut s);
        if mode == 2 {
            s.dispatch(Command::CancelFormatting).unwrap();
        } else if !invalid {
            let d = s.active_document().unwrap();
            s.dispatch(Command::Edit {
                id: d.id,
                expected_version: d.version,
                text: "<?php $x=99;".into(),
            })
            .unwrap();
        }
        idle(&mut s);
        assert_eq!(
            s.active_document().unwrap().text(),
            if mode == 1 { "<?php $x=99;" } else { source }
        );
        if invalid {
            assert!(s.formatting.error.is_some());
        }
    }
}
