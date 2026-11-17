use codisseu_core::{
    AppService, Command,
    debugger::{DebugCommand, DebugConfiguration, DebugRequest, DebugRuntime},
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(service: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        service.poll();
        if !service.is_busy() {
            return;
        }
        assert!(Instant::now() < end, "{:?}", service.last_error);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn integrated_browser_profile_is_passive_redacted_and_cannot_become_external_chrome() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("app.js"), "console.log(42);\n").unwrap();
    let session = dir.path().join("profile/session.json");
    let mut app = AppService::new(session.clone());
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    let node = DebugConfiguration::for_file(root.join("app.js"), &root);
    app.dispatch(Command::SaveDebugConfiguration(node.clone()))
        .unwrap();
    idle(&mut app);
    let config = DebugConfiguration {
        id: "browser-profile".into(),
        name: "Browser profile".into(),
        runtime: DebugRuntime::Chrome,
        request: DebugRequest::Attach,
        integrated_browser: true,
        browser_tab: Some(123),
        url: "http://localhost:5173/app?token=must-not-persist#private".into(),
        port: 54321,
        cwd: root.clone(),
        trusted: true,
        ..DebugConfiguration::default()
    };
    app.dispatch(Command::SaveDebugConfiguration(config))
        .unwrap();
    idle(&mut app);
    let bytes = fs::read(session.with_extension("debugger.json")).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["schema_version"], 3);
    assert!(!text.contains("must-not-persist"));
    assert!(!text.contains("#private"));
    assert!(!text.contains("browser_tab"));
    assert!(!text.contains("browser_inspect_uri"));
    assert!(app.browser.engine_pid().is_none());
    assert!(app.debugger.sessions.is_empty());
    drop(app);
    let mut restored = AppService::new(session);
    restored
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    idle(&mut restored);
    assert!(
        restored
            .debug_configurations
            .iter()
            .any(|c| c.id == node.id && c.runtime == DebugRuntime::Node)
    );
    let mut saved = restored
        .debug_configurations
        .iter()
        .find(|c| c.id == "browser-profile")
        .cloned()
        .unwrap();
    assert!(saved.integrated_browser);
    assert_eq!(saved.port, 0);
    assert_eq!(saved.url, "http://localhost:5173/app");
    assert!(!saved.trusted);
    assert!(saved.browser_tab.is_none());
    saved.trusted = true;
    assert!(
        restored
            .dispatch(Command::Debug(DebugCommand::Start(saved)))
            .is_err()
    );
    assert!(restored.browser.engine_pid().is_none());
    assert!(restored.debugger.sessions.is_empty());
    assert_eq!(
        fs::read_to_string(root.join("app.js")).unwrap(),
        "console.log(42);\n"
    );
}
