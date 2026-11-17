//! Provider proposals are the only source of edits, including local symbols.
use codisseu_core::{AppService, Command, Coverage, ProviderConfig};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            app.status,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn configs() -> Vec<ProviderConfig> {
    serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap()
}
fn fixture(root: &Path, path: &str, text: &str) {
    fs::write(
        root.join("composer.json"),
        r#"{"name":"codisseu/rename-symbols-fixture"}"#,
    )
    .unwrap();
    fs::write(root.join("tsconfig.json"), r#"{"compilerOptions":{"strict":true,"target":"ES2022","module":"commonjs"},"include":["*.ts"]}"#).unwrap();
    fs::write(root.join(path), text).unwrap();
}
fn start(
    root: &Path,
    path: &str,
    text: &str,
    providers: &[ProviderConfig],
) -> (AppService, u64, String) {
    fixture(root, path, text);
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.to_path_buf()))
        .unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(root.join(path))).unwrap();
    idle(&mut app);
    let document = app.active_document().unwrap();
    let id = document.id;
    let dirty = if path.ends_with("php") {
        text.replacen("<?php", "<?php /* unsaved 😀 é */", 1)
    } else {
        format!("/* unsaved 😀 é */ {text}")
    };
    app.dispatch(Command::Edit {
        id,
        expected_version: document.version,
        text: dirty.clone(),
    })
    .unwrap();
    app.dispatch(Command::ConfigureProviders(providers.to_vec()))
        .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    (app, id, dirty)
}
fn propose(app: &mut AppService, id: u64, old: &str, new_name: &str) {
    app.dispatch(Command::Activate { id }).unwrap();
    let d = app.active_document().unwrap();
    let position = d.text()[..d.text().find(old).unwrap()].chars().count() + 1;
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: d.version,
        character_index: position,
    })
    .unwrap();
    idle(app);
    assert!(
        app.rename.error.is_none(),
        "prepare {old}: {:?}",
        app.rename.error
    );
    app.dispatch(Command::PreviewRename {
        new_name: new_name.into(),
    })
    .unwrap();
    idle(app);
    assert!(
        app.rename.error.is_none(),
        "preview {old}->{new_name}: {:?}",
        app.rename.error
    );
}
fn check_types(root: &Path, path: &str, providers: &[ProviderConfig]) {
    let output = if path.ends_with("php") {
        std::process::Command::new("php")
            .arg(root.join(path))
            .output()
            .unwrap()
    } else {
        let ts = providers
            .iter()
            .find(|c| c.language == codisseu_core::Language::TypeScript)
            .unwrap();
        let compiler = Path::new(&ts.args[0])
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("typescript/bin/tsc");
        std::process::Command::new(&ts.program)
            .arg(compiler)
            .args(["--noEmit", "--project", "tsconfig.json"])
            .current_dir(root)
            .output()
            .unwrap()
    };
    assert!(
        output.status.success(),
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
#[test]
#[ignore = "real PHP/TS local/parameter rename, dirty buffers, Undo/Redo/Save; CODISSEU_PROVIDER_CONFIG"]
fn real_local_and_parameter_rename_preserves_homonyms_dirty_history_and_explicit_save() {
    let providers = configs();
    let cases = [
        (
            "main.php",
            "<?php\nfunction weather(int $rain): int { $subtotal = $rain + 1; return $subtotal + $rain; }\nfunction other(int $rain): int { return $rain; }\n// rain and subtotal are comments\n$literal = 'rain subtotal';\nif (weather(3) !== 7) { throw new Exception('bad result'); }\n",
        ),
        (
            "main.ts",
            "export function weather(rain: number): number { const subtotal = rain + 1; return subtotal + rain; }\nexport function other(rain: number): number { return rain; }\n// rain and subtotal are comments\nconst literal = 'rain subtotal';\n",
        ),
    ];
    for (path, text) in cases {
        for (old, new, count) in [("rain", "precipitação", 3), ("subtotal", "soma", 2)] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let (mut app, id, dirty) = start(&root, path, text, &providers);
            let version = app.active_document().unwrap().version;
            propose(&mut app, id, old, new);
            let validation = app.rename.validation.as_ref().unwrap();
            eprintln!(
                "local {path} {old}->{new} {}",
                serde_json::to_string(validation).unwrap()
            );
            assert_eq!(validation.coverage, Coverage::Verified, "{validation:?}");
            assert!(!app.rename.choosing_methods());
            assert_eq!(app.rename.files.len(), 1);
            assert_eq!(app.rename.files[0].edits.len(), count);
            let expected = dirty.replacen(old, new, count);
            assert_eq!(app.rename.files[0].after.as_ref(), expected);
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), text);
            app.dispatch(Command::CancelRename).unwrap();
            idle(&mut app);
            assert_eq!(app.active_document().unwrap().version, version);
            assert_eq!(app.active_document().unwrap().text(), dirty);
            propose(&mut app, id, old, new);
            app.dispatch(Command::ApplyRename).unwrap();
            idle(&mut app);
            assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
            assert_eq!(app.active_document().unwrap().text(), expected);
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), text);
            app.dispatch(Command::Undo { id }).unwrap();
            assert_eq!(app.active_document().unwrap().text(), dirty);
            app.dispatch(Command::Redo { id }).unwrap();
            assert_eq!(app.active_document().unwrap().text(), expected);
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), text);
            app.dispatch(Command::Save { id }).unwrap();
            idle(&mut app);
            assert_eq!(fs::read_to_string(root.join(path)).unwrap(), expected);
            assert!(expected.contains("'rain subtotal'"));
            assert!(expected.contains("// rain and subtotal are comments"));
            assert!(expected.contains(if path.ends_with("php") {
                "other(int $rain)"
            } else {
                "other(rain: number)"
            }));
            check_types(&root, path, &providers);
            app.dispatch(Command::StopProviders).unwrap();
            idle(&mut app);
        }
    }
}
#[test]
#[ignore = "real PHP/TS lexical collision and capture rejection preserves dirty/disk state; CODISSEU_PROVIDER_CONFIG"]
fn real_lexical_collisions_and_captures_block_apply_without_losing_dirty_text() {
    let providers = configs();
    for (path, text) in [
        (
            "main.php",
            "<?php\nfunction weather(int $rain, int $forecast): int { return $rain + $forecast; }\n",
        ),
        (
            "main.ts",
            "export function weather(rain: number, forecast: number): number { return rain + forecast; }\n",
        ),
        (
            "main.ts",
            "export function weather(rain: number): number { { const forecast = 2; return rain + forecast; } }\n",
        ),
        (
            "main.ts",
            "declare const forecast: number; export function weather(rain: number): number { return rain + forecast; }\n",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (mut app, id, dirty) = start(&root, path, text, &providers);
        propose(&mut app, id, "rain", "forecast");
        let validation = app.rename.validation.as_ref().unwrap();
        eprintln!(
            "blocked {path}: {}",
            serde_json::to_string(validation).unwrap()
        );
        assert_ne!(validation.coverage, Coverage::Verified, "{validation:?}");
        let token = app.rename.context_token();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.rename.context_token(), token);
        assert_eq!(app.active_document().unwrap().text(), dirty);
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), text);
        app.dispatch(Command::CancelRename).unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), text);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::StopProviders).unwrap();
        idle(&mut app);
    }
}
