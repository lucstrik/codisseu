use codisseu_core::{AppService, Command, EditKind, Selection, TextDelta};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn settle(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
fn open(s: &mut AppService, p: &Path) -> u64 {
    s.dispatch(Command::OpenFile(p.into())).unwrap();
    settle(s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    s.active.unwrap()
}
fn select(s: &mut AppService, selection: Selection) {
    let d = s.active_document().unwrap();
    s.dispatch(Command::SetSelection {
        id: d.id,
        expected_version: d.version,
        selection,
    })
    .unwrap();
}
fn edit(s: &mut AppService, range: std::ops::Range<usize>, text: &str) {
    let d = s.active_document().unwrap();
    s.dispatch(Command::EditTransaction {
        id: d.id,
        expected_version: d.version,
        before: d.selection(),
        after: Selection::caret(range.start + text.chars().count()),
        delta: TextDelta {
            range,
            insert: text.into(),
        },
        kind: EditKind::Atomic,
    })
    .unwrap();
}
#[test]
fn simultaneous_views_share_text_history_and_remap_passive_unicode_selections() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.ts");
    fs::write(&p, "α😀\r\nabcdef").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    let id = open(&mut s, &p);
    let left = s.active_view().unwrap();
    select(&mut s, Selection { anchor: 9, head: 5 });
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    assert_ne!(left, right);
    select(&mut s, Selection::caret(1));
    edit(&mut s, 1..1, "汉e\u{301}");
    assert_eq!(s.documents.len(), 1);
    assert_eq!(
        s.view(left).unwrap().selection(),
        Selection {
            anchor: 12,
            head: 8
        }
    );
    assert_eq!(s.view(right).unwrap().selection(), Selection::caret(4));
    assert_eq!(
        s.view_document(left).unwrap().text(),
        "α汉e\u{301}😀\r\nabcdef"
    );
    s.dispatch(Command::ActivateView { view: left }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection {
            anchor: 12,
            head: 8
        }
    );
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), "α😀\r\nabcdef");
    assert_eq!(s.view(right).unwrap().selection(), Selection::caret(1));
    s.dispatch(Command::Redo { id }).unwrap();
    s.dispatch(Command::Save { id }).unwrap();
    settle(&mut s);
    assert_eq!(
        fs::read_to_string(&p).unwrap(),
        s.view_document(right).unwrap().text()
    );
}
#[test]
fn tabs_restore_each_groups_position_and_closing_group_keeps_dirty_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "abcdefghij").unwrap();
    fs::write(&b, "0123456789").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    let aid = open(&mut s, &a);
    let left = s.active_view().unwrap();
    select(&mut s, Selection::caret(2));
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    select(&mut s, Selection::caret(7));
    let bid = open(&mut s, &b);
    select(&mut s, Selection::caret(5));
    s.dispatch(Command::Activate { id: aid }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(7)
    );
    s.dispatch(Command::ActivateView { view: left }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(2)
    );
    edit(&mut s, 0..0, "😀");
    s.dispatch(Command::ActivateView { view: right }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(8)
    );
    s.dispatch(Command::Activate { id: bid }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(5)
    );
    s.dispatch(Command::Activate { id: aid }).unwrap();
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(8)
    );
    s.dispatch(Command::CloseView { view: right }).unwrap();
    assert_eq!(s.documents.len(), 2);
    assert!(s.active_document().unwrap().is_dirty());
    assert!(
        s.dispatch(Command::Close {
            id: aid,
            discard: false
        })
        .is_err()
    );
    assert!(s.dispatch(Command::CloseView { view: left }).is_err());
    assert_eq!(fs::read_to_string(a).unwrap(), "abcdefghij");
}
#[test]
fn invalid_view_commands_are_atomic_and_limit_is_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    assert!(s.dispatch(Command::CreateView).is_err());
    let p = dir.path().join("a");
    fs::write(&p, "a").unwrap();
    open(&mut s, &p);
    s.dispatch(Command::CreateView).unwrap();
    let active = s.active_view();
    let text = s.active_document().unwrap().text().to_owned();
    assert!(s.dispatch(Command::CreateView).is_err());
    assert!(s.dispatch(Command::ActivateView { view: 999 }).is_err());
    assert!(s.dispatch(Command::CloseView { view: 999 }).is_err());
    assert_eq!(s.active_view(), active);
    assert_eq!(s.active_document().unwrap().text(), text);
    assert_eq!(s.document_views().len(), 2);
}

#[test]
fn showing_an_open_document_in_right_view_keeps_left_and_shared_dirty_history() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("main.ts");
    let b = dir.path().join("target.ts");
    fs::write(&a, "main source").unwrap();
    fs::write(&b, "target source").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    let aid = open(&mut s, &a);
    let left = s.active_view().unwrap();
    select(&mut s, Selection::caret(3));
    let bid = open(&mut s, &b);
    edit(&mut s, 0..0, "😀");
    select(&mut s, Selection::caret(5));
    let version = s.active_document().unwrap().version;
    s.dispatch(Command::Activate { id: aid }).unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    assert_ne!(left, right);
    assert_eq!(s.view(right).unwrap().document, aid);
    assert!(!s.view(right).unwrap().tabs().contains(&bid));

    s.dispatch(Command::ShowDocumentInView {
        view: right,
        id: bid,
    })
    .unwrap();
    assert_eq!(s.active_view(), Some(right));
    assert_eq!(s.active, Some(bid));
    assert_eq!(s.view(left).unwrap().document, aid);
    assert_eq!(s.view(left).unwrap().selection(), Selection::caret(3));
    assert_eq!(s.view(left).unwrap().tabs(), &[aid, bid]);
    assert_eq!(s.view(right).unwrap().document, bid);
    assert_eq!(s.view(right).unwrap().tabs(), &[aid, bid]);
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(5)
    );
    assert_eq!(s.active_document().unwrap().version, version);
    assert!(s.active_document().unwrap().is_dirty());
    assert!(s.active_document().unwrap().can_undo());
    assert_eq!(s.documents.len(), 2);
    assert_eq!(fs::read_to_string(&b).unwrap(), "target source");

    // Repeating the command preserves the target's own position and tab order.
    select(&mut s, Selection::caret(7));
    s.dispatch(Command::ShowDocumentInView {
        view: right,
        id: bid,
    })
    .unwrap();
    assert_eq!(s.view(right).unwrap().tabs(), &[aid, bid]);
    assert_eq!(
        s.active_document().unwrap().selection(),
        Selection::caret(7)
    );
    s.dispatch(Command::Undo { id: bid }).unwrap();
    assert_eq!(s.view_document(right).unwrap().text(), "target source");
    assert_eq!(
        s.documents.iter().find(|d| d.id == bid).unwrap().text(),
        "target source"
    );
    s.dispatch(Command::Redo { id: bid }).unwrap();
    assert_eq!(s.view_document(right).unwrap().text(), "😀target source");
    s.dispatch(Command::ShowDocumentInView {
        view: left,
        id: bid,
    })
    .unwrap();
    assert_eq!(s.active_view(), Some(left));
    assert_eq!(s.view_document(left).unwrap().id, bid);
    assert_eq!(s.view_document(right).unwrap().id, bid);
    assert_eq!(s.view_document(left).unwrap().text(), "😀target source");
    assert_eq!(s.documents.len(), 2);
    assert_eq!(fs::read_to_string(&a).unwrap(), "main source");
    assert_eq!(fs::read_to_string(&b).unwrap(), "target source");
}

#[test]
fn showing_document_with_invalid_ids_keeps_views_buffers_and_history_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.ts");
    fs::write(&path, "original").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    let id = open(&mut s, &path);
    edit(&mut s, 0..0, "dirty ");
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    let views = format!("{:?}", s.document_views());
    let active = (s.active_view(), s.active);
    let document = s.active_document().unwrap();
    let state = (
        document.version,
        document.text().to_owned(),
        document.selection(),
        document.is_dirty(),
        document.can_undo(),
        document.can_redo(),
    );
    for (view, document) in [(999, id), (right, 999), (999, 999)] {
        assert!(
            s.dispatch(Command::ShowDocumentInView { view, id: document })
                .is_err()
        );
        assert_eq!(format!("{:?}", s.document_views()), views);
        assert_eq!((s.active_view(), s.active), active);
        let document = s.active_document().unwrap();
        assert_eq!(
            (
                document.version,
                document.text().to_owned(),
                document.selection(),
                document.is_dirty(),
                document.can_undo(),
                document.can_redo(),
            ),
            state
        );
    }
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), "original");
    assert_eq!(s.documents.len(), 1);
    assert_eq!(fs::read_to_string(path).unwrap(), "original");
}
#[test]
fn default_session_migration_preserves_legacy_and_does_not_mask_corrupt_primary() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.ts");
    fs::write(&p, "const n = 1;").unwrap();
    let old = dir.path().join("storm/session.json");
    let new = dir.path().join("codisseu/session.json");
    let mut s = AppService::new(old.clone());
    open(&mut s, &p);
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    let bytes = fs::read(&old).unwrap();
    drop(s);
    let mut s = AppService::new(new.clone());
    s.dispatch(Command::RestoreSessionWithFallback {
        legacy: Some(old.clone()),
    })
    .unwrap();
    settle(&mut s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    assert_eq!(s.documents.len(), 1);
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    assert!(new.is_file());
    assert_eq!(fs::read(&old).unwrap(), bytes);
    drop(s);
    fs::write(&new, "corrupt").unwrap();
    let mut s = AppService::new(new);
    s.dispatch(Command::RestoreSessionWithFallback {
        legacy: Some(old.clone()),
    })
    .unwrap();
    settle(&mut s);
    assert!(s.documents.is_empty());
    assert!(s.last_error.is_some());
    assert_eq!(fs::read(old).unwrap(), bytes);
}

#[test]
fn session_restores_split_active_group_and_per_document_positions_without_text_or_history() {
    use codisseu_core::EditorSplit;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.ts");
    let b = dir.path().join("b.php");
    let session = dir.path().join("session.json");
    fs::write(&a, "α😀\r\nabcdefghij").unwrap();
    fs::write(&b, "<?php\n// abcdefghij\n").unwrap();
    let mut s = AppService::new(session.clone());
    let aid = open(&mut s, &a);
    let left = s.active_view().unwrap();
    select(&mut s, Selection { anchor: 9, head: 5 });
    s.dispatch(Command::SetViewScroll {
        view: left,
        scroll: [16., 88.],
    })
    .unwrap();
    let bid = open(&mut s, &b);
    select(&mut s, Selection::caret(7));
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    s.dispatch(Command::Activate { id: aid }).unwrap();
    select(&mut s, Selection::caret(2));
    s.dispatch(Command::SetViewScroll {
        view: right,
        scroll: [32., 220.],
    })
    .unwrap();
    let split = EditorSplit {
        horizontal: true,
        ratio: 0.63,
    };
    s.dispatch(Command::SetEditorSplit(split)).unwrap();
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    let json = fs::read_to_string(&session).unwrap();
    assert!(!json.contains("abcdefghij"));
    drop(s);
    let mut r = AppService::new(session);
    r.dispatch(Command::RestoreSession).unwrap();
    settle(&mut r);
    assert_eq!(r.editor_split(), split);
    assert_eq!(r.document_views().len(), 2);
    let l = r.document_views()[0].id;
    let rr = r.document_views()[1].id;
    assert_eq!(r.active_view(), Some(rr));
    assert_eq!(r.view(rr).unwrap().selection(), Selection::caret(2));
    assert_eq!(r.view(rr).unwrap().scroll(), [32., 220.]);
    r.dispatch(Command::ActivateView { view: l }).unwrap();
    assert_eq!(r.active_document().unwrap().path, b.canonicalize().unwrap());
    assert_eq!(
        r.active_document().unwrap().selection(),
        Selection::caret(7)
    );
    let ra = r
        .documents
        .iter()
        .find(|d| d.path == a.canonicalize().unwrap())
        .unwrap()
        .id;
    r.dispatch(Command::Activate { id: ra }).unwrap();
    assert_eq!(
        r.view(l).unwrap().selection(),
        Selection { anchor: 9, head: 5 }
    );
    assert_eq!(r.view(l).unwrap().scroll(), [16., 88.]);
    assert!(r.documents.iter().all(|d| !d.is_dirty() && !d.can_undo()));
    assert!(r.language.statuses.is_empty());
    assert!(!r.db.connected);
    assert_ne!(aid, bid);
}

#[test]
fn invalid_view_session_never_opens_extra_files_or_overwrites_previous_session() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.ts");
    let session = dir.path().join("session.json");
    fs::write(&file, "abc").unwrap();
    let mut s = AppService::new(session.clone());
    open(&mut s, &file);
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    drop(s);
    let good: serde_json::Value = serde_json::from_slice(&fs::read(&session).unwrap()).unwrap();
    for mode in 0..7 {
        let mut data = good.clone();
        let state = &mut data["editor_state"];
        match mode {
            0 => state["schema_version"] = 99.into(),
            1 => state["active_group"] = 2.into(),
            2 => state["split"]["ratio"] = 0.99.into(),
            3 => state["groups"][0]["positions"][0]["scroll"] = serde_json::json!([-1, 0]),
            4 => {
                state["groups"][0]["document"] =
                    dir.path().join("outside.php").display().to_string().into()
            }
            5 => {
                let p = state["groups"][0]["positions"][0].clone();
                state["groups"][0]["positions"]
                    .as_array_mut()
                    .unwrap()
                    .push(p);
            }
            _ => state["groups"] = serde_json::json!([]),
        }
        let bytes = serde_json::to_vec(&data).unwrap();
        fs::write(&session, &bytes).unwrap();
        let mut r = AppService::new(session.clone());
        r.dispatch(Command::RestoreSession).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while r.is_busy() {
            r.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(r.last_error.is_some(), "mode{mode}");
        assert!(r.documents.is_empty());
        assert!(r.dispatch(Command::PersistSession).is_err());
        assert_eq!(fs::read(&session).unwrap(), bytes);
    }
}

#[test]
fn view_session_clamps_selection_after_external_shortening_and_rejects_invalid_metadata_commands() {
    use codisseu_core::EditorSplit;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a");
    let path = dir.path().join("session.json");
    fs::write(&p, "abcdefghij").unwrap();
    let mut s = AppService::new(path.clone());
    open(&mut s, &p);
    select(&mut s, Selection { anchor: 9, head: 4 });
    let view = s.active_view().unwrap();
    let version = s.active_document().unwrap().version;
    assert!(
        s.dispatch(Command::SetViewScroll {
            view,
            scroll: [f32::NAN, 0.]
        })
        .is_err()
    );
    assert!(
        s.dispatch(Command::SetEditorSplit(EditorSplit {
            horizontal: false,
            ratio: f32::INFINITY
        }))
        .is_err()
    );
    assert_eq!(s.view(view).unwrap().scroll(), [0.; 2]);
    assert_eq!(s.active_document().unwrap().version, version);
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    drop(s);
    fs::write(&p, "😀").unwrap();
    let mut r = AppService::new(path);
    r.dispatch(Command::RestoreSession).unwrap();
    settle(&mut r);
    assert_eq!(
        r.active_document().unwrap().selection(),
        Selection::caret(1)
    );
    assert_eq!(r.active_document().unwrap().text(), "😀");
}

#[test]
fn viewport_metadata_does_not_split_typing_history_or_modify_text_version() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("a.ts");
    fs::write(&p, "").unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    let id = open(&mut s, &p);
    let view = s.active_view().unwrap();
    for (n, c) in ["a", "b"].into_iter().enumerate() {
        let d = s.active_document().unwrap();
        let version = d.version;
        s.dispatch(Command::EditTransaction {
            id,
            expected_version: version,
            before: Selection::caret(n),
            after: Selection::caret(n + 1),
            delta: TextDelta {
                range: n..n,
                insert: c.into(),
            },
            kind: EditKind::Typing,
        })
        .unwrap();
        let version = s.active_document().unwrap().version;
        s.dispatch(Command::SetViewScroll {
            view,
            scroll: [0., 22. * n as f32],
        })
        .unwrap();
        s.dispatch(Command::SetEditorSplit(codisseu_core::EditorSplit {
            horizontal: true,
            ratio: 0.6,
        }))
        .unwrap();
        assert_eq!(s.active_document().unwrap().version, version);
    }
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), "");
    assert!(!s.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(p).unwrap(), "");
}
