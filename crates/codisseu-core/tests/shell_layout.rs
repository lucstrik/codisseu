use codisseu_core::{AppService, Command, ShellLayout};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn settle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn dimensions_round_trip_and_legacy_sessions_get_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let layout = ShellLayout {
        bottom_height: Some(287.),
        left_width: Some(321.),
        right_width: Some(366.),
    };
    let mut app = AppService::new(path.clone());
    app.dispatch(Command::SetShellLayout(layout)).unwrap();
    settle(&mut app);
    app.dispatch(Command::PersistSession).unwrap();
    settle(&mut app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
    let mut restored = AppService::new(path.clone());
    restored.dispatch(Command::RestoreSession).unwrap();
    settle(&mut restored);
    assert_eq!(restored.shell_layout(), layout);
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("shell_layout");
    fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let mut legacy = AppService::new(path);
    legacy.dispatch(Command::RestoreSession).unwrap();
    settle(&mut legacy);
    assert!(legacy.last_error.is_none());
    assert_eq!(legacy.shell_layout(), ShellLayout::default());
}
#[test]
fn invalid_dimensions_reject_commands_and_preserve_bad_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.json");
    let mut app = AppService::new(path.clone());
    for value in [f32::NAN, f32::INFINITY, -1., 0., 501.] {
        assert!(
            app.dispatch(Command::SetShellLayout(ShellLayout {
                bottom_height: Some(value),
                ..Default::default()
            }))
            .is_err()
        );
        assert_eq!(app.shell_layout(), ShellLayout::default());
    }
    let original = br#"{"schema_version":2,"workspace":null,"tabs":[],"active":null,"shell_layout":{"bottom_height":999999,"left_width":null,"right_width":null}}"#;
    fs::write(&path, original).unwrap();
    app.dispatch(Command::RestoreSession).unwrap();
    settle(&mut app);
    assert!(
        app.last_error
            .as_deref()
            .unwrap()
            .contains("painéis inválidas")
    );
    assert!(app.dispatch(Command::PersistSession).is_err());
    assert_eq!(fs::read(path).unwrap(), original);
}
