use codisseu_language::{Language, LanguageCommand, LanguageService, ProviderConfig};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn until(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(8);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(Instant::now() < end, "{:?} {:?}", s.last_error, s.statuses);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn unnecessary_tags_are_negotiated_bounded_unicode_validated_and_invalidated() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("unused.ts");
    let text = "🌧 name\n";
    fs::write(&path, text).unwrap();
    let log = root.join("messages.jsonl");
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/diagnostics_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                log.to_str().unwrap().into(),
            ],
        }],
    })
    .unwrap();
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 1,
        text: text.into(),
    })
    .unwrap();
    until(&mut s, |s| s.analyzed_versions.get(&path) == Some(&1));
    assert_eq!(
        s.diagnostics.len(),
        9,
        "invalid UTF16 and inverted ranges rejected"
    );
    assert_eq!(
        s.diagnostics
            .iter()
            .map(|d| d.unnecessary)
            .collect::<Vec<_>>(),
        vec![true, false, true, false, false, false, false, false, false]
    );
    let init: serde_json::Value =
        serde_json::from_str(fs::read_to_string(&log).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!(
        init["params"]["capabilities"]["textDocument"]["publishDiagnostics"]["tagSupport"]["valueSet"],
        serde_json::json!([1])
    );
    let mut legacy = serde_json::to_value(&s.diagnostics[0]).unwrap();
    legacy.as_object_mut().unwrap().remove("unnecessary");
    assert!(
        !serde_json::from_value::<codisseu_language::Diagnostic>(legacy)
            .unwrap()
            .unnecessary
    );
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 2,
        text: "🌧 used\n".into(),
    })
    .unwrap();
    assert!(s.diagnostics.is_empty());
    until(&mut s, |s| s.analyzed_versions.get(&path) == Some(&2));
    assert!(
        s.diagnostics.is_empty(),
        "stale tagged publication must not resurrect decorations"
    );
    s.dispatch(LanguageCommand::Stop).unwrap();
    until(&mut s, |s| !s.is_busy());
    assert!(s.analyzed_versions.is_empty());
}
