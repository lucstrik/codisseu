//! Real PHP/JS/TS edits, with private preview, Unicode and explicit Save.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < until,
            "{} {:?} actions={:?}",
            app.status,
            app.last_error,
            app.code_actions.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn setup(ext: &str) -> (tempfile::TempDir, AppService, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("projeto ação");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("composer.json"),
        "{\"autoload\":{\"psr-4\":{\"App\\\\\":\"src/\"}}}",
    )
    .unwrap();
    fs::write(root.join("tsconfig.json"),"{\"compilerOptions\":{\"allowJs\":true,\"checkJs\":true,\"noEmit\":true,\"target\":\"ES2022\",\"jsx\":\"preserve\"},\"include\":[\"*\"]}").unwrap();
    let source = if ext == "php" {
        "<?php\r\nfunction other(): int { $value = 5; return $vlaue; }\r\nfunction answer(): int { $number = 42; /* ação 😀 */ return $numbre; }\r\n"
    } else {
        "export const emoji = \"ação 😀\"; export const answer = Math.rond(4.2);\r\n"
    }
    .to_owned();
    fs::write(
        root.join("helpers.ts"),
        "export function double(value: number): number { return value * 2; }\n",
    )
    .unwrap();
    let path = root.join(format!("exemplo.{ext}"));
    fs::write(&path, &source).unwrap();
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required"))
            .unwrap(),
    )
    .unwrap();
    let language = Language::for_path(&path).unwrap();
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::ConfigureProviders(
        configs
            .into_iter()
            .filter(|c| c.language == language)
            .collect(),
    ))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        a.language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.is_busy()
            && !a.language.diagnostics.is_empty()
    });
    (dir, app, path, source)
}
fn preview(app: &mut AppService, php: bool) {
    let doc = app.active_document().unwrap();
    let needle = if php { "$numbre" } else { "rond" };
    let caret = doc.text()[..doc.text().find(needle).unwrap() + 2]
        .chars()
        .count();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::CodeActions {
        id,
        expected_version: version,
        character_index: caret,
    })
    .unwrap();
    wait(app, |a| !a.is_busy());
    eprintln!("actions: {:?}", app.code_actions.items);
    if php {
        assert!(
            app.code_actions
                .items
                .iter()
                .all(|i| !i.title.contains("vlaue")),
            "PHP fixes must match the caret context"
        );
    }
    let token = app
        .code_actions
        .items
        .iter()
        .find(|i| {
            i.disabled_reason.is_none()
                && if php {
                    i.title.contains("Correct undefined variable")
                } else {
                    i.title.contains("round")
                }
        })
        .expect("expected a real enabled correction")
        .token;
    app.dispatch(Command::PreviewCodeAction { token }).unwrap();
    wait(app, |a| !a.is_busy());
    assert!(
        app.code_actions.has_preview(),
        "{:?}",
        app.code_actions.error
    );
}
#[test]
#[ignore = "requires real Phpactor and TypeScript server, CODISSEU_PROVIDER_CONFIG"]
fn php_js_jsx_ts_tsx_preview_apply_undo_redo_save_real() {
    for ext in ["php", "js", "jsx", "ts", "tsx"] {
        let (_dir, mut app, path, source) = setup(ext);
        preview(&mut app, ext == "php");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(app.active_document().unwrap().text(), source);
        app.code_actions.files[0].after = "forged public presentation".into();
        app.dispatch(Command::ApplyCodeAction).unwrap();
        wait(&mut app, |a| !a.is_busy());
        let doc = app.active_document().unwrap();
        let id = doc.id;
        let fixed = doc.text().to_owned();
        assert!(
            if ext == "php" {
                fixed.contains("return $number;")
            } else {
                fixed.contains("Math.round(4.2)")
            },
            "{ext}: {fixed}"
        );
        assert_eq!(
            fixed,
            if ext == "php" {
                source.replace("$numbre", "$number")
            } else {
                source.replace("Math.rond", "Math.round")
            },
            "only the exact proposed spelling changes"
        );
        assert!(doc.is_dirty());
        assert!(fixed.contains("ação 😀"));
        assert!(!fixed.replace("\r\n", "").contains('\n'));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), source);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), fixed);
        app.dispatch(Command::Save { id }).unwrap();
        wait(&mut app, |a| !a.is_busy());
        assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
        app.dispatch(Command::StopProviders).unwrap();
        wait(&mut app, |a| !a.is_busy());
    }
}

#[test]
#[ignore = "real TypeScript server; CODISSEU_PROVIDER_CONFIG required"]
fn typescript_import_dirty_buffer_and_stale_preview_are_real() {
    let (_dir, mut app, path, original) = setup("ts");
    let d = app.active_document().unwrap();
    let (id, version) = (d.id, d.version);
    let dirty = "export const emoji = \"ação 😀\"; export const answer = double(21);\r\n";
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: dirty.into(),
    })
    .unwrap();
    wait(&mut app, |a| {
        a.language.analyzed_versions.get(&path) == Some(&a.active_document().unwrap().version)
            && !a.language.diagnostics.is_empty()
    });
    let d = app.active_document().unwrap();
    let version = d.version;
    let caret = d.text()[..d.text().find("double").unwrap() + 2]
        .chars()
        .count();
    app.dispatch(Command::CodeActions {
        id,
        expected_version: version,
        character_index: caret,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    eprintln!("import actions: {:?}", app.code_actions.items);
    let token = app
        .code_actions
        .items
        .iter()
        .find(|i| {
            i.title.contains("Add import")
                && i.title.contains("helpers")
                && i.disabled_reason.is_none()
        })
        .expect("real import fix")
        .token;
    app.dispatch(Command::PreviewCodeAction { token }).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(
        app.code_actions.has_preview(),
        "{:?}",
        app.code_actions.error
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    app.dispatch(Command::ApplyCodeAction).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let fixed = app.active_document().unwrap().text().to_owned();
    assert!(fixed.contains("import { double }"), "{fixed}");
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), dirty);
    // Re-query, then alter the authoritative buffer: Apply must not reuse it.
    let d = app.active_document().unwrap();
    let version = d.version;
    wait(&mut app, |a| {
        a.language.analyzed_versions.get(&path) == Some(&version)
    });
    app.dispatch(Command::CodeActions {
        id,
        expected_version: version,
        character_index: caret,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    let token = app
        .code_actions
        .items
        .iter()
        .find(|i| {
            i.title.contains("Add import")
                && i.title.contains("helpers")
                && i.disabled_reason.is_none()
        })
        .unwrap()
        .token;
    app.dispatch(Command::PreviewCodeAction { token }).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let edited = format!("{dirty}// changed\r\n");
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: edited.clone(),
    })
    .unwrap();
    app.poll();
    assert!(app.dispatch(Command::ApplyCodeAction).is_err());
    assert_eq!(app.active_document().unwrap().text(), edited);
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
}
