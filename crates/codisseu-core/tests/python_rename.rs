//! Real Basedpyright bindings -> independent Python AST -> shared transaction.
use codisseu_core::{AppService, Command, Coverage, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn idle(a: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        a.poll();
        if !a.is_busy() {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?} {:?}",
            a.status,
            a.last_error,
            a.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn open(a: &mut AppService, p: &Path) {
    a.dispatch(Command::OpenFile(p.into())).unwrap();
    idle(a);
}
fn text(a: &AppService, p: &Path) -> String {
    a.documents
        .iter()
        .find(|d| d.path == p)
        .map(|d| d.text().to_owned())
        .unwrap_or_else(|| fs::read_to_string(p).unwrap())
}
fn prepare(a: &mut AppService, p: &Path, needle: &str, new: &str) {
    open(a, p);
    let source = text(a, p);
    let at = source[..source.find(needle).unwrap()].chars().count() + 1;
    let d = a.active_document().unwrap();
    let (id, version) = (d.id, d.version);
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        a.dispatch(Command::PrepareRename {
            id,
            expected_version: version,
            character_index: at,
        })
        .unwrap();
        idle(a);
        if a.rename.prepared {
            break;
        }
        assert!(Instant::now() < end, "{:?}", a.rename.error);
        thread::sleep(Duration::from_millis(50));
    }
    a.dispatch(Command::PreviewRename {
        new_name: new.into(),
    })
    .unwrap();
    idle(a);
}
fn setup() -> (tempfile::TempDir, PathBuf, AppService) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Python ação");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("metrics.py"),"# rainfall 😀\r\ndef rainfall(amount: int) -> int:\r\n    result = amount + 1\r\n    return result\r\nlabel = 'rainfall'\r\n").unwrap();
    fs::write(
        root.join("consumer.py"),
        "from metrics import rainfall\r\ndef consume() -> int:\r\n    return rainfall(41)\r\n",
    )
    .unwrap();
    fs::write(
        root.join("unrelated.py"),
        "def rainfall() -> int:\n    return 3\n",
    )
    .unwrap();
    fs::write(
        root.join("pyrightconfig.json"),
        "{\"typeCheckingMode\":\"standard\"}",
    )
    .unwrap();
    let root = root.canonicalize().unwrap();
    let mut a = AppService::new(dir.path().join("profile/session.json"));
    a.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut a);
    open(&mut a, &root.join("metrics.py"));
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools");
    a.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
        language: Language::Python,
        program: tools
            .join("node-v24.22.0-linux-x64/bin/node")
            .canonicalize()
            .unwrap(),
        args: vec![
            tools
                .join("python/node_modules/basedpyright/langserver.index.js")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            "--stdio".into(),
        ],
    }]))
    .unwrap();
    a.language
        .dispatch(codisseu_language::LanguageCommand::PythonEnvironment {
            python: Some(codisseu_runtime::python().unwrap()),
        })
        .unwrap();
    a.dispatch(Command::StartProviders).unwrap();
    idle(&mut a);
    assert!(
        a.language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
    );
    (dir, root, a)
}
#[test]
#[ignore = "real Basedpyright and private Python"]
fn python_multifile_unicode_dirty_rename_parameter_local_undo_redo_save_and_collision() {
    let (_dir, root, mut a) = setup();
    let lib = root.join("metrics.py");
    let consumer = root.join("consumer.py");
    let unrelated = root.join("unrelated.py");
    let unrelated_before = fs::read(&unrelated).unwrap();
    open(&mut a, &consumer);
    let d = a.active_document().unwrap();
    let dirty = format!("# dirty 😀\r\n{}", d.text());
    a.dispatch(Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: dirty.clone(),
    })
    .unwrap();
    idle(&mut a);
    for (needle, new) in [
        ("rainfall(amount", "precipitação"),
        ("amount: int", "volume"),
        ("result =", "total"),
    ] {
        let before_lib = text(&a, &lib);
        let before_consumer = text(&a, &consumer);
        prepare(&mut a, &lib, needle, new);
        let validation = a.rename.validation.as_ref().expect("AST validation");
        assert_eq!(
            validation.coverage,
            Coverage::Verified,
            "{validation:?} {:?}",
            a.rename.error
        );
        assert_eq!(
            validation.scope,
            codisseu_syntax::ValidationScope::PythonAnalyzerBindings
        );
        assert!(a.rename.can_apply(), "{:?}", a.rename.error);
        let proposal = a.rename.files.clone();
        a.dispatch(Command::ApplyRename).unwrap();
        idle(&mut a);
        assert!(a.rename.error.is_none(), "{:?}", a.rename.error);
        for f in proposal {
            assert_eq!(text(&a, &f.path), f.after.as_ref());
        }
        let after_lib = text(&a, &lib);
        let after_consumer = text(&a, &consumer);
        assert!(after_lib.contains("# rainfall 😀\r\n"));
        assert!(after_lib.contains("'rainfall'"));
        assert!(after_consumer.starts_with("# dirty 😀\r\n"));
        assert_eq!(fs::read(&unrelated).unwrap(), unrelated_before);
        let id = a.active.unwrap();
        a.dispatch(Command::Undo { id }).unwrap();
        idle(&mut a);
        assert_eq!(text(&a, &lib), before_lib);
        assert_eq!(text(&a, &consumer), before_consumer);
        a.dispatch(Command::Redo { id }).unwrap();
        idle(&mut a);
        assert_eq!(text(&a, &lib), after_lib);
        assert_eq!(text(&a, &consumer), after_consumer);
        for id in a
            .documents
            .iter()
            .filter(|d| d.is_dirty())
            .map(|d| d.id)
            .collect::<Vec<_>>()
        {
            a.dispatch(Command::Save { id }).unwrap();
            idle(&mut a);
        }
    }
    prepare(&mut a, &lib, "precipitação(volume", "consume");
    assert!(!a.rename.can_apply());
    assert_eq!(
        a.rename.validation.as_ref().unwrap().coverage,
        Coverage::Rejected
    );
    a.dispatch(Command::CancelRename).unwrap();
    // A closed file in the certified context must remain byte-for-byte unchanged.
    prepare(&mut a, &lib, "precipitação(volume", "forecast");
    assert!(a.rename.can_apply());
    let before = text(&a, &lib);
    fs::write(&unrelated, "def rainfall():\n    return 99\n").unwrap();
    a.dispatch(Command::ApplyRename).unwrap();
    idle(&mut a);
    assert_eq!(text(&a, &lib), before);
    assert!(a.rename.error.is_some() || a.last_error.is_some());
    a.dispatch(Command::StopProviders).unwrap();
    idle(&mut a);
}
