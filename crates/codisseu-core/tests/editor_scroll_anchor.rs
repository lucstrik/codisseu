use codisseu_core::{AppService, Command, EditKind, Selection, SourceScrollAnchor, TextDelta};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn settle(service: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while service.is_busy() || service.ai.profile_pending() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
}

#[test]
fn source_anchor_survives_restart_remaps_edits_and_explicit_scroll_invalidates_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("long.md");
    let session = dir.path().join("session.json");
    let source = "texto ação 🦀 ".repeat(100);
    fs::write(&path, &source).unwrap();
    let mut service = AppService::new(session.clone());
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    settle(&mut service);
    let document = service.active.unwrap();
    let view = service.active_view().unwrap();
    let anchor = SourceScrollAnchor {
        character: 321,
        offset: 4.5,
    };
    let pixels = [0., 1234.5];
    let version = service.active_document().unwrap().version;
    service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: pixels,
        })
        .unwrap();
    service
        .dispatch(Command::SetViewScrollAnchor {
            view,
            document,
            anchor: Some(anchor),
        })
        .unwrap();
    assert_eq!(
        service.view(view).unwrap().source_scroll_anchor(),
        Some(anchor)
    );
    assert_eq!(service.active_document().unwrap().version, version);
    assert!(!service.active_document().unwrap().can_undo());
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    drop(service);

    let mut service = AppService::new(session);
    service.dispatch(Command::RestoreSession).unwrap();
    settle(&mut service);
    let view = service.active_view().unwrap();
    let document = service.active.unwrap();
    assert_eq!(
        service.view(view).unwrap().source_scroll_anchor(),
        Some(anchor)
    );
    assert_eq!(service.view(view).unwrap().scroll(), pixels);
    let doc = service.active_document().unwrap();
    service
        .dispatch(Command::EditTransaction {
            id: document,
            expected_version: doc.version,
            before: doc.selection(),
            after: Selection::caret(1),
            delta: TextDelta {
                range: 0..0,
                insert: "X".into(),
            },
            kind: EditKind::Atomic,
        })
        .unwrap();
    assert_eq!(
        service
            .view(view)
            .unwrap()
            .source_scroll_anchor()
            .unwrap()
            .character,
        anchor.character + 1
    );
    service.dispatch(Command::Undo { id: document }).unwrap();
    assert_eq!(
        service.view(view).unwrap().source_scroll_anchor(),
        Some(anchor)
    );
    service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: pixels,
        })
        .unwrap();
    assert_eq!(
        service.view(view).unwrap().source_scroll_anchor(),
        Some(anchor)
    );
    service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: [0., 1256.5],
        })
        .unwrap();
    assert!(service.view(view).unwrap().source_scroll_anchor().is_none());
    assert_eq!(service.active_document().unwrap().text(), source);
    assert_eq!(fs::read_to_string(path).unwrap(), source);
}

#[test]
fn source_anchor_validates_owner_range_and_offset_without_mutating_text_or_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.ts");
    fs::write(&path, "const a = 1;").unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path)).unwrap();
    settle(&mut service);
    let view = service.active_view().unwrap();
    let document = service.active.unwrap();
    for (owner, doc, anchor) in [
        (
            u64::MAX,
            document,
            SourceScrollAnchor {
                character: 0,
                offset: 0.,
            },
        ),
        (
            view,
            u64::MAX,
            SourceScrollAnchor {
                character: 0,
                offset: 0.,
            },
        ),
        (
            view,
            document,
            SourceScrollAnchor {
                character: 1000,
                offset: 0.,
            },
        ),
        (
            view,
            document,
            SourceScrollAnchor {
                character: 0,
                offset: -1.,
            },
        ),
        (
            view,
            document,
            SourceScrollAnchor {
                character: 0,
                offset: f32::NAN,
            },
        ),
        (
            view,
            document,
            SourceScrollAnchor {
                character: 0,
                offset: 4097.,
            },
        ),
    ] {
        assert!(
            service
                .dispatch(Command::SetViewScrollAnchor {
                    view: owner,
                    document: doc,
                    anchor: Some(anchor)
                })
                .is_err()
        );
        assert!(service.view(view).unwrap().source_scroll_anchor().is_none());
    }
    assert_eq!(service.active_document().unwrap().text(), "const a = 1;");
    assert!(!service.active_document().unwrap().is_dirty());
    assert!(!service.active_document().unwrap().can_undo());
}

#[test]
fn legacy_positions_accept_missing_anchor_and_corrupt_anchor_preserves_session_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.php");
    let session = dir.path().join("session.json");
    fs::write(&path, "<?php\n$one = 1;\n").unwrap();
    let mut service = AppService::new(session.clone());
    service.dispatch(Command::OpenFile(path)).unwrap();
    settle(&mut service);
    service.dispatch(Command::PersistSession).unwrap();
    settle(&mut service);
    drop(service);
    let legacy = fs::read(&session).unwrap();
    let mut restored = AppService::new(session.clone());
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert!(
        restored
            .view(restored.active_view().unwrap())
            .unwrap()
            .source_scroll_anchor()
            .is_none()
    );
    drop(restored);
    let mut data: serde_json::Value = serde_json::from_slice(&legacy).unwrap();
    data["editor_state"]["groups"][0]["positions"][0]["source_anchor"] =
        serde_json::json!({"character": 0, "offset": -1.});
    let invalid = serde_json::to_vec(&data).unwrap();
    fs::write(&session, &invalid).unwrap();
    let mut restored = AppService::new(session.clone());
    restored.dispatch(Command::RestoreSession).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while restored.is_busy() {
        restored.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(restored.last_error.is_some());
    assert!(restored.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read(session).unwrap(), invalid);
}
