//! Explicit real-provider Composer integration without Laravel or application
//! execution. Fixture code is original, disposable, and has no remote packages.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState, Selection};
use std::{
    fs,
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, ready: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        if ready(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}; providers={:?}; completion={:?}",
            app.status,
            app.language.statuses,
            app.completion.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "real PHP Composer auto-import; requires CODISSEU_PROVIDER_CONFIG, Composer and PHP"]
fn composer_class_completion_import_is_one_undo_and_only_explicit_save_writes_disk() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required; no silent skip"),
        )
        .unwrap(),
    )
    .unwrap();
    let configs = configs
        .into_iter()
        .filter(|config| config.language == Language::Php)
        .collect::<Vec<_>>();
    assert_eq!(configs.len(), 1, "one explicit PHP provider required");

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let source = root.join("main.php");
    let class = root.join("src/RainService.php");
    fs::create_dir(root.join("src")).unwrap();
    let manifest = serde_json::json!({
        "name": "codisseu/plain-php-composer-completion",
        "license": "MIT",
        "autoload": {"psr-4": {"Fixture\\": "src/"}},
        "scripts": {"post-autoload-dump": "@php -r \"file_put_contents('HOOK_EXECUTED', 'forbidden');\""}
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest).unwrap();
    fs::write(root.join("composer.json"), &manifest_bytes).unwrap();
    let class_text = "<?php\r\nnamespace Fixture;\r\nclass RainService {}\r\n";
    fs::write(&class, class_text).unwrap();
    let original = "<?php\r\nnamespace Demo;\r\n// 🌧 previsão e\u{301}\r\nnew RainSer;\r\n";
    fs::write(&source, original).unwrap();
    let composer = Process::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .current_dir(&root)
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .env("COMPOSER_HOME", root.join("composer-home"))
        .env("COMPOSER_CACHE_DIR", root.join("composer-cache"))
        .output()
        .expect("Composer required for the explicit integration fixture");
    assert!(
        composer.status.success(),
        "Composer: {}",
        String::from_utf8_lossy(&composer.stderr)
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
    assert!(!root.join("composer.lock").exists());

    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |app| !app.is_busy());
    app.dispatch(Command::OpenFile(source.clone())).unwrap();
    wait(&mut app, |app| !app.is_busy());
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |app| {
        app.language.statuses.len() == 1
            && app.language.statuses[0].state == ProviderState::Ready
            && !app.is_busy()
    });
    let doc = app.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    let caret = original[..original.find("RainSer;").unwrap() + "RainSer".len()]
        .chars()
        .count();
    app.dispatch(Command::SetSelection {
        id,
        expected_version,
        selection: Selection::caret(caret),
    })
    .unwrap();
    app.dispatch(Command::Completion {
        id,
        expected_version,
        character_index: caret,
    })
    .unwrap();
    wait(&mut app, |app| !app.completion.is_pending());
    assert!(app.completion.error.is_none(), "{:?}", app.completion.error);
    let index = app
        .completion
        .items
        .iter()
        .position(|item| item.label.starts_with("RainService"))
        .unwrap_or_else(|| panic!("real Composer class missing: {:?}", app.completion.items));
    let chosen = &app.completion.items[index];
    assert!(chosen.unsupported_reason.is_none(), "{chosen:?}");
    assert!(
        !chosen.additional_edits.is_empty(),
        "real provider must propose the import: {chosen:?}"
    );
    eprintln!("real PHP provider class completion: {chosen:?}");
    app.dispatch(Command::AcceptCompletion {
        index,
        replace: false,
    })
    .unwrap();
    wait(&mut app, |app| !app.completion.is_applying());
    assert!(app.completion.error.is_none(), "{:?}", app.completion.error);
    let completed = app.active_document().unwrap().text().to_string();
    assert!(
        completed.contains("use Fixture\\RainService;"),
        "{completed:?}"
    );
    assert_eq!(completed.matches("use Fixture\\RainService;").count(), 1);
    assert!(completed.contains("new RainService;"), "{completed:?}");
    assert!(completed.contains("// 🌧 previsão e\u{301}\r\n"));
    assert!(!completed.contains("new RainSer;"));
    assert!(app.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    assert_eq!(
        app.active_document().unwrap().selection(),
        Selection::caret(caret)
    );
    assert!(!app.active_document().unwrap().is_dirty());
    assert_eq!(fs::read_to_string(&source).unwrap(), original);
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), completed);
    assert_eq!(fs::read_to_string(&source).unwrap(), original);

    app.dispatch(Command::Save { id }).unwrap();
    wait(&mut app, |app| !app.is_busy());
    assert_eq!(fs::read_to_string(&source).unwrap(), completed);
    assert!(!app.active_document().unwrap().is_dirty());
    let lint = Process::new("php")
        .args(["-d", "xdebug.mode=off", "-l"])
        .arg(&source)
        .current_dir(&root)
        .output()
        .expect("PHP required for syntax validation only");
    assert!(
        lint.status.success(),
        "{}",
        String::from_utf8_lossy(&lint.stderr)
    );
    assert_eq!(fs::read_to_string(&class).unwrap(), class_text);
    assert_eq!(
        fs::read(root.join("composer.json")).unwrap(),
        manifest_bytes
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |app| !app.is_busy());
    assert!(
        app.language
            .statuses
            .iter()
            .all(|p| p.state == ProviderState::Stopped)
    );
}
