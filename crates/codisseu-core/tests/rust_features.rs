//! Real Rust actions/hints through the shared application service, using disposable sources.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState, Selection};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?} fix={:?} hints={:?}",
            app.status,
            app.last_error,
            app.code_actions.error,
            app.inlay_hints.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn setup(source: &str) -> (tempfile::TempDir, AppService, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_features\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    let path = root.join("src/main.rs");
    fs::write(&path, source).unwrap();
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
        language: Language::Rust,
        program: std::env::var_os("CODISSEU_RUST_ANALYZER")
            .unwrap_or_else(|| "rust-analyzer".into())
            .into(),
        args: vec![],
    }]))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        !a.is_busy()
            && a.language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
    });
    (dir, app, path)
}
fn import_preview(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let d = app.active_document().unwrap();
        let at = d.text().find("HashMap").unwrap();
        let caret = d.text()[..at + 2].chars().count();
        let (id, version) = (d.id, d.version);
        app.dispatch(Command::CodeActions {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        wait(app, |a| !a.is_busy());
        if let Some(token) = app
            .code_actions
            .items
            .iter()
            .find(|i| {
                i.title.contains("Import")
                    && i.title.contains("HashMap")
                    && i.disabled_reason.is_none()
            })
            .map(|i| i.token)
        {
            app.dispatch(Command::PreviewCodeAction { token }).unwrap();
            wait(app, |a| !a.is_busy());
            assert!(
                app.code_actions.has_preview(),
                "{:?}",
                app.code_actions.error
            );
            return;
        }
        app.dispatch(Command::CancelCodeActions).unwrap();
        assert!(
            Instant::now() < deadline,
            "no import action {:?}",
            app.code_actions.items
        );
        thread::sleep(Duration::from_millis(50));
    }
}
#[test]
#[ignore = "real rust-analyzer/Cargo integration; requires Rust components"]
fn rust_quick_fix_private_preview_apply_undo_redo_save_and_stale_guard() {
    let source = "fn main() { let _emoji = \"😀\"; let mut map = HashMap::new(); map.insert(\"answer\", 42); println!(\"{}\", map[\"answer\"]); }\r\n";
    let (_dir, mut app, path) = setup(source);
    import_preview(&mut app);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert!(
        app.code_actions
            .files
            .iter()
            .any(|f| f.after.contains("use std::collections::HashMap"))
    );
    // Public presentation is never authority to change the worker-validated edit.
    app.code_actions.files[0].after = "forged".into();
    app.dispatch(Command::ApplyCodeAction).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let d = app.active_document().unwrap();
    let id = d.id;
    let fixed = d.text().to_owned();
    assert!(fixed.contains("use std::collections::HashMap"));
    assert!(!fixed.contains("forged"));
    assert!(!fixed.replace("\r\n", "").contains('\n'));
    assert!(d.is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), source);
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), fixed);
    app.dispatch(Command::Save { id }).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
    let output = std::process::Command::new(codisseu_core::rust_project::cargo_program())
        .args(["check", "--offline"])
        .current_dir(path.parent().unwrap().parent().unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    app.dispatch(Command::Undo { id }).unwrap();
    import_preview(&mut app);
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: source.replace("42", "43"),
    })
    .unwrap();
    assert!(app.dispatch(Command::ApplyCodeAction).is_err());
    assert!(app.active_document().unwrap().text().contains("43"));
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
}
#[test]
#[ignore = "real rust-analyzer inlay integration; requires Rust components"]
fn rust_inlay_hints_are_inert_versioned_background_and_preference_persists() {
    let source = "fn add(first:i32,second:i32)->i32 { first+second }\nfn main(){ let _emoji=\"😀\"; let answer=add(40,2); println!(\"{answer}\"); }\n";
    let (dir, mut app, path) = setup(source);
    let (id, version, count) = {
        let d = app.active_document().unwrap();
        (d.id, d.version, d.line_count())
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.dispatch(Command::InlayHints {
            id,
            expected_version: version,
            lines: 0..count,
        })
        .unwrap();
        wait(&mut app, |a| !a.inlay_hints.is_pending());
        if app
            .inlay_hints
            .items
            .iter()
            .any(|h| h.label.contains("i32"))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(50));
    }
    assert!(
        app.inlay_hints
            .items
            .iter()
            .any(|h| h.label.contains("first"))
    );
    assert!(
        app.inlay_hints
            .items
            .iter()
            .any(|h| h.label.contains("second"))
    );
    assert_eq!(app.active_document().unwrap().text(), source);
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::InlayHints {
        id,
        expected_version: version,
        lines: 0..count,
    })
    .unwrap();
    app.dispatch(Command::SetSelection {
        id,
        expected_version: version,
        selection: Selection::caret(0),
    })
    .unwrap();
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: source.replace("40,2", "39,3"),
    })
    .unwrap();
    app.poll();
    assert!(app.inlay_hints.items.is_empty());
    app.dispatch(Command::SetRustInlayHints(false)).unwrap();
    assert!(!app.editor_preferences().rust_inlay_hints);
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::PersistSession).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let session = dir.path().join("profile/session.json");
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    drop(app);
    let mut restored = AppService::new(session);
    restored.dispatch(Command::RestoreSession).unwrap();
    wait(&mut restored, |a| !a.is_busy());
    assert!(!restored.editor_preferences().rust_inlay_hints);
}
