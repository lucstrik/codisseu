//! Real Basedpyright definitions to venv packages and private typeshed, readonly.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::Path,
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};
fn wait(a: &mut AppService, p: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        a.poll();
        if p(a) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?} {:?}",
            a.status,
            a.last_error,
            a.navigation.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(a: &mut AppService) {
    wait(a, |a| !a.is_busy());
}
#[test]
#[ignore = "real Basedpyright, private Python and disposable venv"]
fn venv_and_stdlib_definitions_open_readonly_and_reject_edits_rename_save() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Python project");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let venv = root.join(".venv");
    let result = Process::new(codisseu_runtime::python().unwrap())
        .args(["-m", "venv", "--without-pip"])
        .arg(&venv)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let interpreter = venv.join("bin/python");
    let package = venv.join("lib/python3.13/site-packages/fakeweather");
    fs::create_dir_all(&package).unwrap();
    let dep = package.join("__init__.py");
    fs::write(&dep, "def temperature() -> int:\n    return 42\n").unwrap();
    let path = root.join("main.py");
    let source = "from fakeweather import temperature\nfrom pathlib import Path\nvalue = temperature()\npath = Path('example')\n";
    fs::write(&path, source).unwrap();
    let session = temp.path().join("profile/session.json");
    let mut a = AppService::new(session.clone());
    a.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut a);
    a.dispatch(Command::OpenFile(path.clone())).unwrap();
    idle(&mut a);
    // Exact saved per-project interpreter selection; UI selection itself is covered
    // by the project service tests. Preserve the declared venv symlink path.
    a.python.config = Some(codisseu_core::python_project::Config::new(root.clone()));
    a.python.interpreter = Some(interpreter.clone());
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
            python: Some(interpreter.clone()),
        })
        .unwrap();
    a.dispatch(Command::StartProviders).unwrap();
    wait(&mut a, |a| {
        !a.is_busy()
            && a.language
                .statuses
                .iter()
                .any(|p| p.state == ProviderState::Ready)
    });
    let origin = a.active.unwrap();
    for (needle, inside) in [("temperature()", true), ("Path('example'", false)] {
        let caret = source[..source.rfind(needle).unwrap()].chars().count() + 1;
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            a.dispatch(Command::Definition {
                id: origin,
                expected_version: 1,
                character_index: caret,
            })
            .unwrap();
            idle(&mut a);
            if a.active == Some(origin) && !a.navigation.choices.is_empty() {
                a.dispatch(Command::ChooseDefinition { index: 0 }).unwrap();
                idle(&mut a);
            }
            if a.active != Some(origin) {
                break;
            }
            assert!(
                Instant::now() < end,
                "{} {:?} {:?}",
                a.status,
                a.navigation.error,
                a.navigation.choices
            );
            thread::sleep(Duration::from_millis(50));
        }
        let d = a.active_document().unwrap();
        let (id, version, library_path, before) =
            (d.id, d.version, d.path.clone(), d.text().to_owned());
        assert!(d.is_read_only());
        assert_eq!(library_path.starts_with(&root), inside);
        assert!(
            codisseu_language::python_library_root(&library_path, Some(&interpreter)).is_some()
        );
        let disk = fs::read(&library_path).unwrap();
        eprintln!("readonly Python definition {}", library_path.display());
        for cmd in [
            Command::Edit {
                id,
                expected_version: version,
                text: "changed".into(),
            },
            Command::Save { id },
            Command::Undo { id },
            Command::Redo { id },
            Command::PrepareRename {
                id,
                expected_version: version,
                character_index: 1,
            },
            Command::FormatDocument {
                id,
                expected_version: version,
                range: None,
            },
        ] {
            assert!(a.dispatch(cmd).unwrap_err().contains("somente leitura"));
            assert_eq!(a.active_document().unwrap().text(), before);
        }
        assert!(!a.language.analyzed_versions.contains_key(&library_path));
        assert_eq!(fs::read(&library_path).unwrap(), disk);
        a.dispatch(Command::NavigateBack {
            id,
            expected_version: version,
            character_index: 0,
        })
        .unwrap();
        assert_eq!(a.active, Some(origin));
        assert_eq!(a.active_document().unwrap().text(), source);
    }
    a.dispatch(Command::StopProviders).unwrap();
    idle(&mut a);
    a.dispatch(Command::PersistSession).unwrap();
    idle(&mut a);
    let stored: serde_json::Value = serde_json::from_slice(&fs::read(session).unwrap()).unwrap();
    assert_eq!(stored["tabs"], serde_json::json!([path]));
}
