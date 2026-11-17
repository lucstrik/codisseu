//! Own generated PHP corpus; no application boot, Composer or network.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
#[test]
#[ignore = "large real Phpactor cold index; CODISSEU_PROVIDER_CONFIG required"]
fn large_php_index_completion_definition_and_stop() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("config")).unwrap(),
    )
    .unwrap();
    let configs: Vec<_> = configs
        .into_iter()
        .filter(|c| c.language == Language::Php)
        .collect();
    assert_eq!(configs.len(), 1);
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("composer.json"),
        r#"{"name":"codisseu/large-index","autoload":{"psr-4":{"Fixture\\":"src/"}}}"#,
    )
    .unwrap();
    for i in 0..32000 {
        fs::write(root.join(format!("src/Class{i}.php")), format!("<?php namespace Fixture; class Class{i} {{ public function forecast(): string {{ return 'rain'; }} }}\n")).unwrap();
    }
    let path = root.join("main.php");
    fs::write(&path, "<?php // saved baseline\n").unwrap();
    let source = "<?php\n$weather = new \\Fixture\\Class123();\n$weather->fore";
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root.clone(),
        configs,
    })
    .unwrap();
    s.dispatch(LanguageCommand::Sync {
        path: path.clone(),
        version: 2,
        text: source.into(),
    })
    .unwrap();
    let start = Instant::now();
    let mut details = std::collections::BTreeSet::new();
    loop {
        s.poll();
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        for status in &s.statuses {
            details.insert(status.detail.clone());
        }
        if s.statuses.iter().any(|p| p.state == ProviderState::Ready) {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(240),
            "{:?}",
            s.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
    let elapsed = start.elapsed();
    s.dispatch(LanguageCommand::Completion {
        path: path.clone(),
        version: 2,
        line: 2,
        character: 14,
    })
    .unwrap();
    wait(&mut s, |s| !s.completion_pending());
    assert!(s.completion_error.is_none(), "{:?}", s.completion_error);
    assert!(
        s.completion
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|i| i.label.starts_with("forecast"))
    );
    s.dispatch(LanguageCommand::Definition {
        path: path.clone(),
        version: 2,
        line: 1,
        character: 28,
    })
    .unwrap();
    wait(&mut s, |s| !s.definition_pending());
    assert!(s.definition_error.is_none(), "{:?}", s.definition_error);
    assert!(
        s.definition
            .as_ref()
            .unwrap()
            .targets
            .iter()
            .any(|t| t.path == root.join("src/Class123.php"))
    );
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().all(|p| p.state == ProviderState::Stopped)
    });
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "<?php // saved baseline\n"
    );
    println!(
        "{}",
        serde_json::json!({"files":32000,"index_seconds":elapsed.as_secs_f64(),"distinct_statuses":details.len(),"completion":true,"definition":true,"saved_bytes_preserved":true,"stop":true})
    );
}
fn wait(s: &mut LanguageService, condition: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        s.poll();
        if condition(s) {
            return;
        }
        assert!(Instant::now() < deadline, "{:?}", s.last_error);
        thread::sleep(Duration::from_millis(5));
    }
}
