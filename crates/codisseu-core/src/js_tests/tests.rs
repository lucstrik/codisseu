use super::*;
#[test]
fn filters_are_literal_and_bounded() {
    assert_eq!(
        filter(&["ação 😀 [x].* (ok)".into()]).unwrap(),
        "^(?:ação 😀 \\[x\\]\\.\\* \\(ok\\))$"
    );
    assert!(filter(&["x".repeat(65536)]).is_err());
}
#[test]
fn parse_real_shapes_and_reject_unknown_or_external() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("a.ts");
    std::fs::write(&file, "test").unwrap();
    let json = serde_json::json!({"testResults":[{"name":file,"assertionResults":[{"fullName":"grupo ação 😀","status":"passed","duration":12.5,"location":{"line":2}},{"fullName":"skip","status":"pending"}]}]});
    let cases = report::parse(
        &serde_json::to_vec(&json).unwrap(),
        temp.path(),
        Framework::Jest,
    )
    .unwrap();
    assert_eq!(cases[0].line, Some(2));
    assert_eq!(cases[0].seconds, 0.0125);
    assert_eq!(cases[1].outcome, Outcome::Skipped);
    let mut bad = json.clone();
    bad["testResults"][0]["assertionResults"][0]["status"] = "invented".into();
    assert!(
        report::parse(
            &serde_json::to_vec(&bad).unwrap(),
            temp.path(),
            Framework::Jest
        )
        .is_err()
    );
    let mut external = json;
    external["testResults"][0]["name"] = "/etc/passwd".into();
    assert!(
        report::parse(
            &serde_json::to_vec(&external).unwrap(),
            temp.path(),
            Framework::Jest
        )
        .unwrap()[0]
            .path
            .is_none()
    );
    assert!(report::parse(b"{}", temp.path(), Framework::Jest).is_err());
    assert!(report::parse(b"not json", temp.path(), Framework::Jest).is_err());
}
#[test]
fn inspect_does_not_execute_and_isolates_roots() {
    let t = tempfile::tempdir().unwrap();
    let profile = t.path().join("profile.json");
    let c = inspect(&profile, t.path(), &[t.path().into()]).unwrap();
    assert_eq!(c.framework, Framework::Auto);
    assert!(!profile.exists());
    assert!(inspect(&profile, Path::new("/"), &[t.path().into()]).is_err());
}
#[test]
fn watch_is_transient_coalesced_and_stop_disarms() {
    let t = tempfile::tempdir().unwrap();
    let mut s = Service::new(t.path().join("profile"));
    assert!(s.dispatch(TestCommand::Watch(true), vec![]).is_err());
    s.last = Some(LastRun {
        config: Config::new(t.path().into()),
        invocations: vec![],
        cases: vec![],
    });
    s.dispatch(TestCommand::Watch(true), vec![]).unwrap();
    s.saved(Path::new("/external.ts"));
    assert!(!s.watch_pending);
    s.saved(&t.path().join("a.ts"));
    assert!(s.watch_pending);
    assert!(!s.watch_ready());
    s.cancel();
    assert!(!s.watching && !s.watch_pending);
}
#[test]
fn missing_runner_is_error_not_success() {
    let t = tempfile::tempdir().unwrap();
    let config = Config::new(t.path().into());
    let (tx, _) = mpsc::sync_channel(32);
    let (state, report) = execute(
        &t.path().join("config"),
        &config,
        &[Invocation {
            file: None,
            names: vec![],
        }],
        &[t.path().into()],
        &AtomicBool::new(false),
        &tx,
    );
    assert_eq!(state, State::Error);
    assert!(report.error.is_some());
    assert!(report.cases.is_empty());
}
#[cfg(unix)]
#[test]
fn runner_symlinks_cannot_escape_project() {
    let t = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/etc/passwd", t.path().join("runner.js")).unwrap();
    assert!(local_file(t.path(), Path::new("runner.js")).is_err());
}
