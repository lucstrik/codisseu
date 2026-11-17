use codisseu_core::{AppService, Command};
use std::time::{Duration, Instant};
fn settle(s: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(10);
    while s.is_busy() {
        s.poll();
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn untitled_save_cancel_collision_retry_and_history_preserve_contents() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = AppService::new(dir.path().join("session.json"));
    s.dispatch(Command::NewUntitled).unwrap();
    let id = s.active.unwrap();
    assert_eq!(
        s.active_document().unwrap().path.to_str(),
        Some("Untitled-1")
    );
    assert!(s.active_document().unwrap().is_untitled());
    s.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "<?php\r\n// ação 😀\n".into(),
    })
    .unwrap();
    assert!(s.active_document().unwrap().is_dirty());
    assert!(!dir.path().join("Untitled-1").exists());
    let target = dir.path().join("new.php");
    std::fs::write(&target, "existing").unwrap();
    s.dispatch(Command::SaveUntitled {
        id,
        path: target.clone(),
    })
    .unwrap();
    settle(&mut s);
    assert!(s.last_error.is_some());
    assert!(s.active_document().unwrap().is_untitled());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "existing");
    std::fs::remove_file(&target).unwrap();
    s.dispatch(Command::SaveUntitled {
        id,
        path: target.clone(),
    })
    .unwrap();
    settle(&mut s);
    let d = s.active_document().unwrap();
    assert!(!d.is_untitled());
    assert!(!d.is_dirty());
    assert_eq!(d.path, target);
    assert_eq!(d.id, id);
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "<?php\r\n// ação 😀\n"
    );
    s.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(s.active_document().unwrap().text(), "");
    assert!(s.active_document().unwrap().is_dirty());
    s.dispatch(Command::Redo { id }).unwrap();
    assert!(!s.active_document().unwrap().is_dirty());
    s.dispatch(Command::NewUntitled).unwrap();
    s.dispatch(Command::PersistSession).unwrap();
    settle(&mut s);
    let session: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("session.json")).unwrap()).unwrap();
    assert_eq!(session["tabs"].as_array().unwrap().len(), 1);
    assert!(
        session["editor_state"]["groups"][0]["document"]
            .as_str()
            .unwrap()
            .ends_with("new.php")
    );
}
