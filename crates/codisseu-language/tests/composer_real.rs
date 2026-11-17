//! Composer PHP without Laravel, scripts, plugins, network or application boot.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, utf16_byte_offset,
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn wait(service: &mut LanguageService, condition: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if condition(service) {
            return;
        }
        assert!(service.last_error.is_none(), "{:?}", service.last_error);
        assert!(
            Instant::now() < deadline,
            "{:?} {:?} {:?}",
            service.statuses,
            service.completion_error,
            service.definition_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn put(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn position(text: &str, byte: usize) -> (u32, u32) {
    let p = &text[..byte];
    (
        p.bytes().filter(|b| *b == b'\n').count() as u32,
        p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}
#[test]
#[ignore = "real generic Composer PHP; CODISSEU_PROVIDER_CONFIG, PHP and Composer required"]
fn composer_php_classes_methods_functions_attributes_and_imports_work_without_laravel() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    put(
        &root,
        "composer.json",
        r#"{"name":"codisseu/composer-fixture","autoload":{"psr-4":{"Fixture\\":"src/"},"classmap":["legacy/"],"files":["helpers.php"]},"autoload-dev":{"psr-4":{"Fixture\\Tests\\":"tests/"}},"scripts":{"post-autoload-dump":"@php -r \"file_put_contents('HOOK_EXECUTED','bad');\""}}"#,
    );
    put(
        &root,
        "src/RainService.php",
        "<?php namespace Fixture; class RainService { public function forecast(): string { return 'rain'; } }",
    );
    put(
        &root,
        "src/TraceSpan.php",
        "<?php namespace Fixture; #[\\Attribute] class TraceSpan { public function __construct(public string $name = '') {} }",
    );
    put(
        &root,
        "legacy/LegacyWeather.php",
        "<?php class LegacyWeather { public function city(): string { return 'Rio'; } }",
    );
    put(
        &root,
        "tests/WeatherCase.php",
        "<?php namespace Fixture\\Tests; class WeatherCase {}",
    );
    put(
        &root,
        "helpers.php",
        "<?php namespace Fixture; function weather_forecast(): string { return 'rain'; }",
    );
    put(&root, "main.php", "<?php // saved marker\n");
    let manifest = fs::read(root.join("composer.json")).unwrap();
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
            "--optimize",
        ])
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs: configs
                .into_iter()
                .filter(|c| c.language == Language::Php)
                .collect(),
        })
        .unwrap();
    wait(&mut service, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready) && !s.is_busy()
    });
    let source = root.join("main.php");
    let mut version = 0;
    for (tail, expected) in [
        ("new RainSer", "RainService"),
        ("$s = new \\Fixture\\RainService(); $s->fore", "forecast"),
        ("\\Fixture\\weather_fo", "weather_forecast"),
        ("weather_fo", "weather_forecast"),
        ("new LegacyWea", "LegacyWeather"),
        ("new \\Fixture\\Tests\\WeatherC", "WeatherCase"),
        ("#[Trac", "TraceSpan"),
    ] {
        version += 1;
        let text = format!("<?php\n// 🌧 e\u{301}\n{tail}");
        let (line, character) = position(&text, text.len());
        service
            .dispatch(LanguageCommand::Sync {
                path: source.clone(),
                version,
                text: text.clone(),
            })
            .unwrap();
        service
            .dispatch(LanguageCommand::Completion {
                path: source.clone(),
                version,
                line,
                character,
            })
            .unwrap();
        wait(&mut service, |s| !s.completion_pending());
        assert!(
            service.completion_error.is_none(),
            "{:?}",
            service.completion_error
        );
        let result = service.completion.as_ref().unwrap();
        let item = result
            .items
            .iter()
            .find(|i| i.label == expected || i.label.starts_with(&format!("{expected} (")))
            .unwrap_or_else(|| panic!("{expected}: {:?}", result.items));
        assert!(
            item.unsupported_reason.is_none(),
            "{expected}: {:?}",
            item.unsupported_reason
        );
        if tail == "weather_fo" {
            assert!(
                item.additional_edits.iter().any(|e| e
                    .new_text
                    .contains("use function Fixture\\weather_forecast;")),
                "real function autoimport: {item:?}"
            );
        }
        if expected == "RainService" {
            assert!(
                item.additional_edits
                    .iter()
                    .any(|e| e.new_text.contains("use Fixture\\RainService;")),
                "real autoimport: {item:?}"
            );
        }
        eprintln!(
            "generic Composer completion {expected}; imports={}",
            item.additional_edits.len()
        );
    }
    for (tail, symbol, file) in [
        (
            "use Fixture\\RainService as Rain; new Rain();",
            "Rain();",
            "src/RainService.php",
        ),
        (
            "$s = new \\Fixture\\RainService(); $s->forecast();",
            "forecast",
            "src/RainService.php",
        ),
        (
            "\\Fixture\\weather_forecast();",
            "weather_forecast",
            "helpers.php",
        ),
        (
            "new LegacyWeather();",
            "LegacyWeather",
            "legacy/LegacyWeather.php",
        ),
        (
            "new \\Fixture\\Tests\\WeatherCase();",
            "WeatherCase",
            "tests/WeatherCase.php",
        ),
        (
            "#[\\Fixture\\TraceSpan] class Example {}",
            "TraceSpan",
            "src/TraceSpan.php",
        ),
    ] {
        version += 1;
        let head = "<?php\n// 🌧 e\u{301}\n";
        let text = format!("{head}{tail}");
        let at = head.len() + tail.find(symbol).unwrap() + 1;
        let (line, character) = position(&text, at);
        service
            .dispatch(LanguageCommand::Sync {
                path: source.clone(),
                version,
                text,
            })
            .unwrap();
        service
            .dispatch(LanguageCommand::Definition {
                path: source.clone(),
                version,
                line,
                character,
            })
            .unwrap();
        wait(&mut service, |s| !s.definition_pending());
        assert!(
            service.definition_error.is_none(),
            "{:?}",
            service.definition_error
        );
        let result = service.definition.as_ref().unwrap();
        assert!(
            result.targets.iter().any(|t| t.path == root.join(file)),
            "{symbol}: {:?}",
            result.targets
        );
        for t in &result.targets {
            let bytes = fs::read_to_string(&t.path).unwrap();
            assert!(utf16_byte_offset(&bytes, t.start_line, t.start_character).is_some());
        }
        eprintln!("generic Composer definition {symbol}: {file}");
    }
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |s| !s.is_busy());
    assert_eq!(fs::read(root.join("composer.json")).unwrap(), manifest);
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "<?php // saved marker\n"
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
}
