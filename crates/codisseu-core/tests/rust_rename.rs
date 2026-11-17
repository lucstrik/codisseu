//! Real rust-analyzer rename, common preview/transaction and independent Cargo
//! checks. All sources are owned temporary fixtures; no project hooks are run.
use codisseu_core::{AppService, Command, Coverage, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn idle(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?} rename={:?}",
            app.status,
            app.last_error,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn open(app: &mut AppService, path: &Path) {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    idle(app);
}
fn text(app: &AppService, path: &Path) -> String {
    app.documents
        .iter()
        .find(|d| d.path == path)
        .map(|d| d.text().to_owned())
        .unwrap_or_else(|| fs::read_to_string(path).unwrap())
}
fn prepare(app: &mut AppService, path: &Path, needle: &str, new: &str) {
    open(app, path);
    let source = text(app, path);
    let caret = source[..source.find(needle).unwrap()].chars().count() + 1;
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        app.dispatch(Command::PrepareRename {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        idle(app);
        if app.rename.prepared {
            break;
        }
        assert!(
            Instant::now() < until,
            "prepare failed: {:?} {:?}",
            app.rename.error,
            app.last_error
        );
        thread::sleep(Duration::from_millis(50));
    }
    app.dispatch(Command::PreviewRename {
        new_name: new.into(),
    })
    .unwrap();
    idle(app);
}
fn cargo(root: &Path) {
    let output = std::process::Command::new(codisseu_core::rust_project::cargo_program())
        .args(["check", "--offline", "--quiet"])
        .current_dir(root)
        .env(
            "CARGO_TARGET_DIR",
            root.parent().unwrap().join("target-own"),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn setup() -> (tempfile::TempDir, PathBuf, AppService) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Rust ação workspace");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_rust_rename\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"),concat!(
        "// ação 😀 · rainfall stays in comments\r\n",
        "pub mod consumer;\r\n",
        "pub struct Weather { pub rainfall: i32 }\r\n",
        "impl Weather { pub fn calculate(&self, extra: i32) -> i32 { let total = self.rainfall + extra; total } }\r\n",
        "pub fn rainfall(input: i32) -> i32 { input + 1 }\r\n",
        "pub mod unrelated { pub fn rainfall() -> i32 { 3 } }\r\n",
        "pub fn run() -> i32 { let _literal = \"rainfall\"; rainfall(consumer::call()) + unrelated::rainfall() }\r\n"
    )).unwrap();
    fs::write(root.join("src/consumer.rs"),concat!(
        "use crate::{Weather, rainfall};\r\n",
        "pub fn call() -> i32 { let weather = Weather { rainfall: 40 }; rainfall(weather.calculate(2)) }\r\n"
    )).unwrap();
    let root = root.canonicalize().unwrap();
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    open(&mut app, &root.join("src/lib.rs"));
    app.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
        language: Language::Rust,
        program: std::env::var_os("CODISSEU_RUST_ANALYZER")
            .unwrap_or_else(|| "rust-analyzer".into())
            .into(),
        args: vec![],
    }]))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(
        app.language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
    );
    (dir, root, app)
}
#[test]
#[ignore = "real Rust rename; requires rust-analyzer/rust-src/Cargo"]
fn rust_real_multifile_function_type_field_parameter_local_undo_redo_save() {
    let (_dir, root, mut app) = setup();
    let lib = root.join("src/lib.rs");
    let consumer = root.join("src/consumer.rs");
    // Keep a dirty reference buffer while the declaration is queried.
    open(&mut app, &consumer);
    let d = app.active_document().unwrap();
    let dirty = format!("// dirty 😀\r\n{}", d.text());
    app.dispatch(Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: dirty.clone(),
    })
    .unwrap();
    idle(&mut app);
    for (needle, new, kind) in [
        (
            "rainfall(input",
            "precipitação",
            codisseu_core::RenameSymbolKind::Function,
        ),
        (
            "Weather { pub",
            "Station",
            codisseu_core::RenameSymbolKind::Class,
        ),
        (
            "calculate(&self",
            "compute",
            codisseu_core::RenameSymbolKind::Method,
        ),
        (
            "rainfall: i32",
            "degrees",
            codisseu_core::RenameSymbolKind::Property,
        ),
        (
            "extra: i32",
            "offset",
            codisseu_core::RenameSymbolKind::Parameter,
        ),
        (
            "total =",
            "r#type",
            codisseu_core::RenameSymbolKind::Variable,
        ),
    ] {
        let lib_before = text(&app, &lib);
        let consumer_before = text(&app, &consumer);
        let disk_lib = fs::read(&lib).unwrap();
        let disk_consumer = fs::read(&consumer).unwrap();
        prepare(&mut app, &lib, needle, new);
        assert_eq!(app.rename.symbol_kind(), kind);
        assert!(
            app.rename.can_apply(),
            "{needle}->{new}: {:?} {:?}",
            app.rename.error,
            app.rename.validation
        );
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Verified
        );
        let proposed = app.rename.files.clone();
        assert!(!proposed.is_empty());
        assert_eq!(fs::read(&lib).unwrap(), disk_lib);
        assert_eq!(fs::read(&consumer).unwrap(), disk_consumer);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        for file in &proposed {
            assert_eq!(text(&app, &file.path), file.after.as_ref());
        }
        let lib_after = text(&app, &lib);
        let consumer_after = text(&app, &consumer);
        assert!(lib_after.contains("// ação 😀 · rainfall stays in comments"));
        assert!(lib_after.contains("\"rainfall\""));
        assert!(lib_after.contains("pub mod unrelated { pub fn rainfall()"));
        assert!(lib_after.contains("unrelated::rainfall()"));
        assert!(consumer_after.starts_with("// dirty 😀\r\n"));
        let id = app.active.unwrap();
        app.dispatch(Command::Undo { id }).unwrap();
        idle(&mut app);
        assert_eq!(text(&app, &lib), lib_before);
        assert_eq!(text(&app, &consumer), consumer_before);
        app.dispatch(Command::Redo { id }).unwrap();
        idle(&mut app);
        assert_eq!(text(&app, &lib), lib_after);
        assert_eq!(text(&app, &consumer), consumer_after);
        for id in app
            .documents
            .iter()
            .filter(|d| d.is_dirty())
            .map(|d| d.id)
            .collect::<Vec<_>>()
        {
            app.dispatch(Command::Save { id }).unwrap();
            idle(&mut app);
        }
        assert_eq!(fs::read_to_string(&lib).unwrap(), lib_after);
        assert_eq!(fs::read_to_string(&consumer).unwrap(), consumer_after);
        cargo(&root);
    }
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    assert!(!app.providers_running());
}
#[test]
#[ignore = "real Rust rename; requires rust-analyzer/rust-src/Cargo"]
fn rust_real_collision_stale_edit_and_external_context_never_apply() {
    let (_dir, root, mut app) = setup();
    let lib = root.join("src/lib.rs");
    let consumer = root.join("src/consumer.rs");
    let original = fs::read_to_string(&lib).unwrap();
    prepare(&mut app, &lib, "rainfall(input", "run");
    assert!(!app.rename.can_apply());
    assert_eq!(text(&app, &lib), original);
    app.dispatch(Command::CancelRename).unwrap();
    idle(&mut app);
    prepare(&mut app, &lib, "rainfall(input", "forecast");
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    fs::write(
        &consumer,
        format!(
            "// external mutation\r\n{}",
            fs::read_to_string(&consumer).unwrap()
        ),
    )
    .unwrap();
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_some() || app.last_error.is_some());
    assert_eq!(text(&app, &lib), original);
    app.dispatch(Command::CancelRename).unwrap();
    idle(&mut app);
    prepare(&mut app, &lib, "rainfall(input", "forecast");
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    let d = app.active_document().unwrap();
    let id = d.id;
    let current_version = d.version;
    app.dispatch(Command::Edit {
        id,
        expected_version: current_version,
        text: format!("// newer buffer\r\n{}", d.text()),
    })
    .unwrap();
    idle(&mut app);
    assert!(app.dispatch(Command::ApplyRename).is_err());
    assert!(!text(&app, &lib).contains("fn forecast"));
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}
