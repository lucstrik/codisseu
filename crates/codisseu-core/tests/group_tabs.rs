use codisseu_core::{AppService, Command, EditKind, Selection, TextDelta};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn idle(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(1));
    }
}
fn open(s: &mut AppService, path: &Path) -> u64 {
    s.dispatch(Command::OpenFile(path.into())).unwrap();
    idle(s);
    assert!(s.last_error.is_none(), "{:?}", s.last_error);
    s.active.unwrap()
}
fn fixture() -> (tempfile::TempDir, AppService, Vec<std::path::PathBuf>) {
    let d = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["a.ts", "b.php", "c.txt"]
        .iter()
        .map(|n| d.path().join(n))
        .collect();
    for p in &paths {
        fs::write(p, "α😀\r\nabcdef\n").unwrap();
    }
    let s = AppService::new(d.path().join("session.json"));
    (d, s, paths)
}
fn select(s: &mut AppService, at: usize) {
    let d = s.active_document().unwrap();
    s.dispatch(Command::SetSelection {
        id: d.id,
        expected_version: d.version,
        selection: Selection::caret(at),
    })
    .unwrap();
}
fn dirty(s: &mut AppService) {
    let d = s.active_document().unwrap();
    s.dispatch(Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: format!("{}// dirty\n", d.text()),
    })
    .unwrap();
}
#[test]
fn each_group_has_ordered_membership_and_reopening_prefers_existing_instance() {
    let (_dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    let b = open(&mut s, &p[1]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    let c = open(&mut s, &p[2]);
    assert_eq!(s.view(left).unwrap().tabs(), [a, b]);
    assert_eq!(s.view(right).unwrap().tabs(), [b, c]);
    assert_eq!(open(&mut s, &p[0]), a);
    assert_eq!(s.active_view(), Some(left));
    assert_eq!(s.view(right).unwrap().tabs(), [b, c]);
    s.dispatch(Command::ActivateTab { view: right, id: c })
        .unwrap();
    select(&mut s, 7);
    let before = (
        s.active_view(),
        s.active,
        s.documents.len(),
        s.view(right).unwrap().selection(),
    );
    assert!(
        s.dispatch(Command::ActivateTab { view: right, id: a })
            .is_err()
    );
    assert_eq!(
        (
            s.active_view(),
            s.active,
            s.documents.len(),
            s.view(right).unwrap().selection()
        ),
        before
    );
    s.dispatch(Command::Activate { id: a }).unwrap();
    assert_eq!(s.view(right).unwrap().tabs(), [b, c, a]);
    assert_eq!(s.documents.len(), 3);
}
#[test]
fn closing_dirty_copy_preserves_history_then_last_instance_requires_explicit_discard() {
    let (_dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    let b = open(&mut s, &p[1]);
    s.dispatch(Command::ActivateTab { view: right, id: a })
        .unwrap();
    dirty(&mut s);
    let text = s.active_document().unwrap().text().to_owned();
    let version = s.active_document().unwrap().version;
    assert!(!s.tab_close_needs_confirmation(left, a));
    s.dispatch(Command::CloseTab {
        view: left,
        id: a,
        discard: false,
    })
    .unwrap();
    assert_eq!(s.document_views().len(), 1);
    assert_eq!(s.active_view(), Some(right));
    assert_eq!(s.active_document().unwrap().text(), text);
    assert_eq!(s.active_document().unwrap().version, version);
    assert!(s.tab_close_needs_confirmation(right, a));
    assert!(
        s.dispatch(Command::CloseTab {
            view: right,
            id: a,
            discard: false
        })
        .is_err()
    );
    assert_eq!(s.view(right).unwrap().tabs(), [a, b]);
    assert_eq!(s.active_document().unwrap().text(), text);
    s.dispatch(Command::Undo { id: a }).unwrap();
    assert!(!s.active_document().unwrap().is_dirty());
    s.dispatch(Command::Redo { id: a }).unwrap();
    s.dispatch(Command::Save { id: a }).unwrap();
    idle(&mut s);
    assert_eq!(fs::read_to_string(&p[0]).unwrap(), text);
    dirty(&mut s);
    s.dispatch(Command::CloseTab {
        view: right,
        id: a,
        discard: true,
    })
    .unwrap();
    assert_eq!(s.view(right).unwrap().tabs(), [b]);
    assert_eq!(s.active, Some(b));
    assert_eq!(s.documents.len(), 1);
    assert_eq!(fs::read_to_string(&p[0]).unwrap(), text);
    s.dispatch(Command::CloseTab {
        view: right,
        id: b,
        discard: false,
    })
    .unwrap();
    assert!(s.documents.is_empty());
    assert!(s.document_views().is_empty());
    assert_eq!(s.active, None);
}
#[test]
fn passive_close_does_not_change_active_group_selection_or_canonical_version() {
    let (_dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    let b = open(&mut s, &p[1]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    select(&mut s, 7);
    let version = s.active_document().unwrap().version;
    s.dispatch(Command::CloseTab {
        view: left,
        id: a,
        discard: false,
    })
    .unwrap();
    assert_eq!(s.active_view(), Some(right));
    assert_eq!(s.active, Some(b));
    s.dispatch(Command::CloseTab {
        view: left,
        id: b,
        discard: false,
    })
    .unwrap();
    assert_eq!(s.active_view(), Some(right));
    assert_eq!(s.view(right).unwrap().selection(), Selection::caret(7));
    assert_eq!(s.active_document().unwrap().version, version);
    assert_eq!(s.documents.len(), 1);
    assert_eq!(s.document_views().len(), 1);
}
#[test]
fn closing_division_merges_unique_tabs_and_positions_without_losing_dirty_documents() {
    let (_dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    let b = open(&mut s, &p[1]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    let c = open(&mut s, &p[2]);
    select(&mut s, 8);
    dirty(&mut s);
    select(&mut s, 7);
    s.dispatch(Command::SetViewScroll {
        view: right,
        scroll: [12., 88.],
    })
    .unwrap();
    s.dispatch(Command::CloseView { view: right }).unwrap();
    assert_eq!(s.view(left).unwrap().tabs(), [a, b, c]);
    assert_eq!(s.active_view(), Some(left));
    assert_eq!(s.active, Some(b));
    s.dispatch(Command::ActivateTab { view: left, id: c })
        .unwrap();
    assert_eq!(s.view(left).unwrap().selection(), Selection::caret(7));
    assert_eq!(s.view(left).unwrap().scroll(), [12., 88.]);
    assert!(s.active_document().unwrap().is_dirty());
}
#[test]
fn membership_order_and_local_active_positions_round_trip_without_text_or_processes() {
    let (dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    let b = open(&mut s, &p[1]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    let c = open(&mut s, &p[2]);
    select(&mut s, 8);
    s.dispatch(Command::SetViewScroll {
        view: right,
        scroll: [4., 123.],
    })
    .unwrap();
    s.dispatch(Command::ActivateTab { view: left, id: a })
        .unwrap();
    select(&mut s, 2);
    s.dispatch(Command::PersistSession).unwrap();
    idle(&mut s);
    let bytes = fs::read(dir.path().join("session.json")).unwrap();
    let data: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(data["editor_state"]["schema_version"], 2);
    assert_eq!(
        data["editor_state"]["groups"][0]["tabs"],
        serde_json::json!([p[0], p[1]])
    );
    assert_eq!(
        data["editor_state"]["groups"][1]["tabs"],
        serde_json::json!([p[1], p[2]])
    );
    drop(s);
    let mut r = AppService::new(dir.path().join("session.json"));
    r.dispatch(Command::RestoreSession).unwrap();
    idle(&mut r);
    assert!(r.last_error.is_none(), "{:?}", r.last_error);
    let names = |v: &codisseu_core::DocumentView| {
        v.tabs()
            .iter()
            .map(|id| {
                r.documents
                    .iter()
                    .find(|d| d.id == *id)
                    .unwrap()
                    .path
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&r.document_views()[0]), [p[0].clone(), p[1].clone()]);
    assert_eq!(names(&r.document_views()[1]), [p[1].clone(), p[2].clone()]);
    assert_eq!(r.active_view(), Some(r.document_views()[0].id));
    assert_eq!(r.document_views()[0].selection(), Selection::caret(2));
    assert_eq!(r.document_views()[1].selection(), Selection::caret(8));
    assert_eq!(r.document_views()[1].scroll(), [4., 123.]);
    assert!(r.documents.iter().all(|d| !d.is_dirty() && !d.can_undo()));
    assert!(r.language.statuses.is_empty());
    assert!(!r.db.connected);
    assert_ne!(a, b);
    assert_ne!(b, c);
}
#[test]
fn legacy_global_tabs_migrate_and_invalid_local_metadata_is_rejected_atomically() {
    let (dir, mut s, p) = fixture();
    open(&mut s, &p[0]);
    open(&mut s, &p[1]);
    s.dispatch(Command::CreateView).unwrap();
    s.dispatch(Command::PersistSession).unwrap();
    idle(&mut s);
    drop(s);
    let path = dir.path().join("session.json");
    let good: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut legacy = good.clone();
    legacy["editor_state"]["schema_version"] = 1.into();
    for g in legacy["editor_state"]["groups"].as_array_mut().unwrap() {
        g.as_object_mut().unwrap().remove("tabs");
    }
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let mut r = AppService::new(path.clone());
    r.dispatch(Command::RestoreSession).unwrap();
    idle(&mut r);
    assert!(r.last_error.is_none(), "{:?}", r.last_error);
    assert!(r.document_views().iter().all(|v| v.tabs().len() == 2));
    drop(r);
    for mode in 0..8 {
        let mut v = good.clone();
        let state = &mut v["editor_state"];
        match mode {
            0 => state["groups"][0]["tabs"] = serde_json::json!([]),
            1 => state["groups"][0]["tabs"] = serde_json::json!([p[0], p[0]]),
            2 => state["groups"][0]["tabs"] = serde_json::json!([dir.path().join("outside.txt")]),
            3 => state["groups"][1]["tabs"] = serde_json::json!([p[0]]),
            4 => state["groups"][1]["positions"][0]["path"] = serde_json::json!(p[0]),
            5 => state["groups"][0]
                .as_object_mut()
                .unwrap()
                .remove("tabs")
                .map(|_| ())
                .unwrap(),
            6 => {
                for g in state["groups"].as_array_mut().unwrap() {
                    g["tabs"] = serde_json::json!([p[1]]);
                    g["positions"]
                        .as_array_mut()
                        .unwrap()
                        .retain(|q| q["path"] == serde_json::json!(p[1]));
                }
            }
            _ => state["groups"][0]["tabs"] = serde_json::json!(vec![p[0].clone(); 33]),
        }
        let bytes = serde_json::to_vec(&v).unwrap();
        fs::write(&path, &bytes).unwrap();
        let mut r = AppService::new(path.clone());
        r.dispatch(Command::RestoreSession).unwrap();
        idle(&mut r);
        assert!(r.last_error.is_some(), "mode{mode}");
        assert!(r.documents.is_empty());
        assert!(r.dispatch(Command::PersistSession).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn reorder_is_metadata_only_preserves_dirty_large_buffer_and_group_positions() {
    let (dir, mut s, p) = fixture();
    // A large document exercises the same path without making reorder depend
    // on its contents. Removing the source later rules out a hidden reload.
    let original = "α😀\r\nabcdef\n".repeat(150_000);
    fs::write(&p[0], &original).unwrap();
    let a = open(&mut s, &p[0]);
    let b = open(&mut s, &p[1]);
    let c = open(&mut s, &p[2]);
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    s.dispatch(Command::ActivateTab { view: left, id: a })
        .unwrap();
    select(&mut s, 7);
    s.dispatch(Command::SetViewScroll {
        view: left,
        scroll: [12., 888.],
    })
    .unwrap();
    dirty(&mut s);
    let before = s.active_document().unwrap();
    let ptr = before.text().as_ptr();
    let version = before.version;
    let selection = before.selection();
    let positions = s.view(left).unwrap().selection();
    let scroll = s.view(left).unwrap().scroll();
    fs::remove_file(&p[0]).unwrap();
    let started = Instant::now();
    for _ in 0..10_000 {
        s.dispatch(Command::ReorderTab {
            view: left,
            id: a,
            index: 2,
        })
        .unwrap();
        assert_eq!(s.view(left).unwrap().tabs(), [b, c, a]);
        s.dispatch(Command::ReorderTab {
            view: left,
            id: a,
            index: 0,
        })
        .unwrap();
    }
    eprintln!(
        "20,000 reorder commands / {} byte buffer: {:?}",
        original.len(),
        started.elapsed()
    );
    assert!(!s.is_busy());
    assert_eq!(s.active_view(), Some(left));
    assert_eq!(s.active, Some(a));
    assert_eq!(s.view(right).unwrap().tabs(), [c]);
    assert_eq!(s.view(left).unwrap().selection(), positions);
    assert_eq!(s.view(left).unwrap().scroll(), scroll);
    let doc = s.active_document().unwrap();
    assert_eq!(doc.text().as_ptr(), ptr, "no buffer/cache replacement");
    assert_eq!(doc.version, version);
    assert_eq!(doc.selection(), selection);
    assert!(doc.is_dirty() && doc.can_undo());
    assert_eq!(s.documents.len(), 3);
    assert!(s.language.statuses.is_empty());
    s.dispatch(Command::Undo { id: a }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), original);
    s.dispatch(Command::Redo { id: a }).unwrap();
    assert!(s.active_document().unwrap().text().ends_with("// dirty\n"));
    // Restore disk only for the explicit Save; reorder never needed it.
    fs::write(&p[0], &original).unwrap();
    s.dispatch(Command::Save { id: a }).unwrap();
    idle(&mut s);
    assert_eq!(
        fs::read_to_string(&p[0]).unwrap(),
        s.active_document().unwrap().text()
    );
    assert!(dir.path().exists());
}

#[test]
fn reorder_every_source_destination_validates_atomically_and_persists_passive_group() {
    let (dir, mut s, p) = fixture();
    let ids: Vec<_> = p.iter().map(|p| open(&mut s, p)).collect();
    let left = s.active_view().unwrap();
    s.dispatch(Command::CreateView).unwrap();
    let right = s.active_view().unwrap();
    for source in 0..3 {
        for destination in 0..3 {
            let before = s.view(left).unwrap().tabs().to_vec();
            let id = before[source];
            let mut expected = before;
            expected.remove(source);
            expected.insert(destination, id);
            s.dispatch(Command::ReorderTab {
                view: left,
                id,
                index: destination,
            })
            .unwrap();
            assert_eq!(s.view(left).unwrap().tabs(), expected);
            assert_eq!(s.active_view(), Some(right));
            assert_eq!(s.active, Some(ids[2]));
        }
    }
    let before = s.view(left).unwrap().tabs().to_vec();
    for (view, id, index) in [
        (u64::MAX, ids[0], 0),
        (left, u64::MAX, 0),
        (left, ids[0], 3),
        (right, ids[0], 0),
    ] {
        assert!(s.dispatch(Command::ReorderTab { view, id, index }).is_err());
        assert_eq!(s.view(left).unwrap().tabs(), before);
        assert_eq!(s.view(right).unwrap().tabs(), [ids[2]]);
    }
    s.dispatch(Command::ReorderTab {
        view: left,
        id: ids[2],
        index: 0,
    })
    .unwrap();
    let paths = s
        .view(left)
        .unwrap()
        .tabs()
        .iter()
        .map(|id| {
            s.documents
                .iter()
                .find(|d| d.id == *id)
                .unwrap()
                .path
                .clone()
        })
        .collect::<Vec<_>>();
    s.dispatch(Command::PersistSession).unwrap();
    idle(&mut s);
    drop(s);
    let mut restored = AppService::new(dir.path().join("session.json"));
    restored.dispatch(Command::RestoreSession).unwrap();
    idle(&mut restored);
    assert!(restored.last_error.is_none(), "{:?}", restored.last_error);
    let group = &restored.document_views()[0];
    let actual = group
        .tabs()
        .iter()
        .map(|id| {
            restored
                .documents
                .iter()
                .find(|d| d.id == *id)
                .unwrap()
                .path
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, paths);
    assert_eq!(
        restored.active_view(),
        Some(restored.document_views()[1].id)
    );
}

#[test]
fn metadata_reorder_does_not_split_an_in_progress_typing_undo_group() {
    let (_dir, mut s, p) = fixture();
    let a = open(&mut s, &p[0]);
    open(&mut s, &p[1]);
    let view = s.active_view().unwrap();
    s.dispatch(Command::ActivateTab { view, id: a }).unwrap();
    let original = s.active_document().unwrap().text().to_owned();
    for (n, c) in ["x", "y"].into_iter().enumerate() {
        let version = s.active_document().unwrap().version;
        s.dispatch(Command::EditTransaction {
            id: a,
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
        s.dispatch(Command::ReorderTab {
            view,
            id: a,
            index: 1 - n,
        })
        .unwrap();
    }
    assert!(s.active_document().unwrap().text().starts_with("xy"));
    s.dispatch(Command::Undo { id: a }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), original);
}
