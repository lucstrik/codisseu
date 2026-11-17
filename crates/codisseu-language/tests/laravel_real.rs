//! Explicit Laravel Composer integration; no application boot, helpers, hooks or database.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use std::{
    fs,
    path::{Path, PathBuf},
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
        thread::sleep(Duration::from_millis(10));
    }
}
fn position(text: &str, offset: usize) -> (u32, u32) {
    let prefix = &text[..offset];
    (
        prefix.bytes().filter(|c| *c == b'\n').count() as u32,
        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}
fn sync(service: &mut LanguageService, path: &Path, version: u64, text: &str) {
    service
        .dispatch(LanguageCommand::Sync {
            path: path.into(),
            version,
            text: text.into(),
        })
        .unwrap();
}
#[test]
#[ignore = "real Laravel Composer fixture and Phpactor; requires CODISSEU_PROVIDER_CONFIG"]
fn laravel_composer_classes_inherited_methods_facades_and_navigation_are_real() {
    let root = std::env::var_os("CODISSEU_LARAVEL_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/laravel"))
        .canonicalize()
        .unwrap();
    assert!(
        root.join("vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php")
            .is_file(),
        "Install only the isolated fixture with Composer --no-scripts --no-plugins first"
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
    let source = root.join("app/Http/Controllers/WeatherController.php");
    let original = fs::read(&source).unwrap();
    let manifest_before = fs::read(root.join("composer.json")).unwrap();
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap();
    let configs = configs
        .into_iter()
        .filter(|p| p.language == Language::Php)
        .collect();
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs,
        })
        .unwrap();
    wait(&mut service, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready) && !s.is_busy()
    });
    // The source remains an unsaved buffer. Unicode before the query exercises
    // actual UTF-16 routing; PHP/Laravel never executes this incomplete text.
    let head = "<?php\nuse App\\Models\\User;\nuse App\\Services\\WeatherService;\nuse Illuminate\\Http\\Request;\nuse Illuminate\\Support\\Facades\\Route;\n// 🌧 previsão\n";
    let mut version = 1;
    for (tail, expected) in [
        ("$s = new WeatherService(); $s->fore", "forecast"),
        ("$s = new User(); $s->display", "displayName"),
        ("$s = new User(); $s->getTab", "getTable"),
        ("$s = new Request(); $s->getCli", "getClientIp"),
        ("Route::ge", "get"),
        ("User::whe", "where"),
        ("User::query()->whe", "where"),
    ] {
        version += 1;
        let text = format!("{head}{tail}");
        sync(&mut service, &source, version, &text);
        let (line, character) = position(&text, text.len());
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
        assert_eq!(result.version, version);
        assert!(
            result
                .items
                .iter()
                .any(|i| i.label == expected && i.unsupported_reason.is_none()),
            "Expected {expected}: {:?}",
            result.items
        );
        eprintln!(
            "real Laravel completion {expected}: {} items",
            result.items.len()
        );
    }
    for (tail, word, expected_file) in [
        (
            "$s = new WeatherService(); $s->forecast('Rio');",
            "forecast",
            "app/Services/WeatherService.php",
        ),
        (
            "$s = new User(); $s->displayName();",
            "displayName",
            "app/Models/User.php",
        ),
        (
            "$s = new User(); $s->getTable();",
            "getTable",
            "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php",
        ),
        (
            "User::where('active', true);",
            "where",
            "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Builder.php",
        ),
        (
            "User::query()->where('active', true);",
            "where",
            "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Builder.php",
        ),
        (
            "Route::get('/', fn()=>42);",
            "get",
            "vendor/laravel/framework/src/Illuminate/Support/Facades/Route.php",
        ),
        (
            "$s = new WeatherService();",
            "WeatherService",
            "app/Services/WeatherService.php",
        ),
    ] {
        version += 1;
        let text = format!("{head}{tail}");
        sync(&mut service, &source, version, &text);
        let (line, character) = position(&text, head.len() + tail.find(word).unwrap() + 1);
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
        assert_eq!(result.version, version);
        assert!(
            result
                .targets
                .iter()
                .any(|t| t.path == root.join(expected_file)),
            "{word}: {:?}",
            result.targets
        );
        for target in &result.targets {
            let bytes = fs::read_to_string(&target.path).unwrap();
            let start = codisseu_language::utf16_byte_offset(
                &bytes,
                target.start_line,
                target.start_character,
            )
            .unwrap();
            let end =
                codisseu_language::utf16_byte_offset(&bytes, target.end_line, target.end_character)
                    .unwrap();
            assert!(
                bytes[start..end].contains(word),
                "Provider target must contain real declaration/docblock {word}"
            );
        }
        eprintln!("real Laravel definition {word}: {:?}", result.targets);
    }

    // Restore the controller declaration before querying it from a separate
    // buffer: source-derived lookups must honor dirty target snapshots.
    version += 1;
    sync(
        &mut service,
        &source,
        version,
        &String::from_utf8(original.clone()).unwrap(),
    );
    let probe = root.join("editor_probe.php");
    let blade_path = root.join("resources/views/weather.blade.php");
    let blade_original = fs::read(&blade_path).unwrap();
    for (path, marked, expected) in [
        (&probe, "<?php // 🌧\nview('wea|');", "weather"),
        (
            &probe,
            "<?php config('weather.forecast.c|');",
            "weather.forecast.city",
        ),
        (&probe, "<?php route('admin.wea|');", "admin.weather.index"),
        (&probe, "<?php __('weather.fore|');", "weather.forecast"),
        (&probe, "<?php __('Weather f|');", "Weather forecast"),
        (&probe, "<?php asset('css/wea|');", "css/weather.css"),
        (
            &probe,
            "<?php use App\\Http\\Controllers\\WeatherController; use Illuminate\\Support\\Facades\\Route; Route::get('/', [WeatherController::class, 'ind|']);",
            "index",
        ),
        (&blade_path, "@extends('layouts.a|')", "layouts.app"),
        (
            &blade_path,
            "{{ config('weather.forecast.c|') }}",
            "weather.forecast.city",
        ),
        (&blade_path, "@fore|", "@foreach"),
    ] {
        version += 1;
        let at = marked.find('|').unwrap();
        let text = marked.replace('|', "");
        sync(&mut service, path, version, &text);
        let (line, character) = position(&text, at);
        service
            .dispatch(LanguageCommand::Completion {
                path: path.clone(),
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
            .find(|i| i.label == expected)
            .unwrap_or_else(|| panic!("{marked}: {:?}", result.items));
        assert!(item.unsupported_reason.is_none());
        assert!(item.text_edit.is_some());
        eprintln!("real PHP transport + Laravel resource completion {expected}");
    }
    for (path, marked, expected_file, count) in [
        (
            &probe,
            "<?php config('weather.forecast.c|ity');",
            "config/weather.php",
            1,
        ),
        (
            &probe,
            "<?php route('admin.weather.in|dex');",
            "routes/web.php",
            1,
        ),
        (
            &probe,
            "<?php __('weather.fore|cast');",
            "lang/en/weather.php",
            2,
        ),
        (
            &probe,
            "<?php __('Weather fore|cast');",
            "lang/pt_BR.json",
            1,
        ),
        (
            &probe,
            "<?php use App\\Http\\Controllers\\WeatherController; use Illuminate\\Support\\Facades\\Route; Route::get('/', [WeatherController::class, 'in|dex']);",
            "app/Http/Controllers/WeatherController.php",
            1,
        ),
        (
            &blade_path,
            "@extends('layouts.a|pp')",
            "resources/views/layouts/app.blade.php",
            1,
        ),
        (
            &blade_path,
            "{{ asset('css/wea|ther.css') }}",
            "public/css/weather.css",
            1,
        ),
    ] {
        version += 1;
        let at = marked.find('|').unwrap();
        let text = marked.replace('|', "");
        sync(&mut service, path, version, &text);
        let (line, character) = position(&text, at);
        service
            .dispatch(LanguageCommand::Definition {
                path: path.clone(),
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
        assert_eq!(
            result.targets.len(),
            count,
            "{marked}: {:?}",
            result.targets
        );
        assert!(
            result
                .targets
                .iter()
                .any(|t| t.path == root.join(expected_file)),
            "{marked}"
        );
        eprintln!(
            "real PHP transport + Laravel resource definition {expected_file}: {count} targets"
        );
    }
    assert_eq!(fs::read(&blade_path).unwrap(), blade_original);
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |s| !s.is_busy());
    assert_eq!(fs::read(source).unwrap(), original);
    assert_eq!(
        fs::read(root.join("composer.json")).unwrap(),
        manifest_before
    );
    assert!(!root.join("HOOK_EXECUTED").exists());
}
