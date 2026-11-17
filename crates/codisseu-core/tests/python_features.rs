//! Real Ruff/project configuration checks over disposable Python files and profiles.
use codisseu_core::{
    AppService, Command, Selection,
    python_project::{Action, Config, ProjectCommand, plan, resolve_interpreter},
};
use std::{
    fs,
    path::PathBuf,
    process::Command as Process,
    thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(Instant::now() < end, "{} {:?}", app.status, app.last_error);
        thread::sleep(Duration::from_millis(5));
    }
}
fn setup(source: &str) -> (tempfile::TempDir, AppService, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("projeto ação");
    fs::create_dir(&root).unwrap();
    let file = root.join("main.py");
    fs::write(&file, source).unwrap();
    let mut app = AppService::new(temp.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root)).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(file.clone())).unwrap();
    idle(&mut app);
    (temp, app, file)
}
#[test]
fn inspect_only_reads_declared_venv_and_profile_survives_without_execution() {
    let (temp, mut app, path) = setup("print('unchanged')\n");
    let root = path.parent().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(root.join(".venv/bin")).unwrap();
        let interpreter = root.join(".venv/bin/python");
        fs::write(&interpreter, "#!/bin/sh\ntouch SHOULD_NOT_EXECUTE\n").unwrap();
        fs::set_permissions(&interpreter, fs::Permissions::from_mode(0o755)).unwrap();
        app.dispatch(Command::Python(ProjectCommand::Inspect(root.into())))
            .unwrap();
        idle(&mut app);
        assert_eq!(app.python.interpreter.as_ref(), Some(&interpreter));
        assert!(!root.join("SHOULD_NOT_EXECUTE").exists());
        let mut cfg = app.python.config.clone().unwrap();
        cfg.arguments = vec!["literal 😀".into()];
        app.dispatch(Command::Python(ProjectCommand::Save(cfg.clone())))
            .unwrap();
        idle(&mut app);
        assert!(app.python.error.is_none());
        let profile = temp.path().join("profile/session.python.json");
        assert!(profile.exists());
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
        assert_eq!(stored[root.to_string_lossy().as_ref()]["interpreter"], "");
        let mut restored = codisseu_core::python_project::Service::new(profile);
        restored
            .dispatch(ProjectCommand::Inspect(root.into()), &[root.into()])
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while restored.is_busy() {
            restored.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(restored.config.unwrap(), cfg);
        assert_eq!(restored.interpreter, Some(interpreter));
        assert!(!root.join("SHOULD_NOT_EXECUTE").exists());
    }
    assert_eq!(fs::read_to_string(path).unwrap(), "print('unchanged')\n");
}
#[test]
#[ignore = "real bundled Python, venv and Ruff"]
fn python_task_plans_are_literal_venv_safe_and_dirty_guarded() {
    let (_temp, mut app, path) = setup(
        "import json,os,sys\nprint(json.dumps([sys.argv[1:],os.environ['CODISSEU_LITERAL']]))\n",
    );
    let root = path.parent().unwrap().to_path_buf();
    let mut cfg = Config::new(root.clone());
    cfg.arguments = vec![
        "ação 😀".into(),
        "$(touch SHOULD_NOT_EXECUTE)".into(),
        "quote'\"".into(),
    ];
    cfg.environment
        .insert("CODISSEU_LITERAL".into(), "literal;$HOME".into());
    let interpreter = resolve_interpreter(&cfg).unwrap();
    let planned = plan(&cfg, interpreter.clone(), Action::RunFile, Some(&path), &[]).unwrap();
    let mut process = Process::new(&planned.program);
    process.args(&planned.args).current_dir(&planned.cwd);
    for (k, v) in &planned.env {
        if let Some(v) = v {
            process.env(k, v);
        } else {
            process.env_remove(k);
        }
    }
    let out = process.output().unwrap();
    assert!(out.status.success());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value[0][0], "ação 😀");
    assert_eq!(value[0][1], "$(touch SHOULD_NOT_EXECUTE)");
    assert_eq!(value[1], "literal;$HOME");
    assert!(!root.join("SHOULD_NOT_EXECUTE").exists());
    let venv = plan(&cfg, interpreter, Action::CreateVenv, None, &[]).unwrap();
    assert!(
        Process::new(venv.program)
            .args(venv.args)
            .status()
            .unwrap()
            .success()
    );
    let selected = resolve_interpreter(&cfg).unwrap();
    assert_eq!(selected, root.join(".venv/bin/python"));
    assert!(plan(&cfg, selected.clone(), Action::CreateVenv, None, &[]).is_err());
    let pip = plan(
        &cfg,
        selected,
        Action::InstallPackages,
        None,
        &["pytest".into()],
    )
    .unwrap();
    assert!(pip.env.contains(&("PIP_TARGET".into(), None)));
    assert!(pip.args.ends_with(&["--".into(), "pytest".into()]));
    cfg.environment
        .insert("PIP_TARGET".into(), "/outside".into());
    assert!(
        plan(
            &cfg,
            resolve_interpreter(&cfg).unwrap(),
            Action::InstallPackages,
            None,
            &["pytest".into()]
        )
        .is_err()
    );
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "print('dirty')\n".into(),
    })
    .unwrap();
    assert!(
        app.dispatch(Command::Python(ProjectCommand::Prepare {
            config: cfg,
            action: Action::RunFile,
            path: Some(path.clone()),
            values: vec![]
        }))
        .is_err()
    );
    assert!(!fs::read_to_string(path).unwrap().contains("dirty"));
}
#[test]
#[ignore = "real Ruff formatter from the bundled tools"]
fn python_format_is_one_versioned_buffer_transaction_with_config_and_crlf() {
    let source = "# ação 😀\r\ndef combine(first:int,second:int)->int:\r\n return first+second\r\n";
    let (_temp, mut app, path) = setup(source);
    fs::write(
        path.parent().unwrap().join("pyproject.toml"),
        "[tool.ruff.format]\nquote-style = 'single'\n",
    )
    .unwrap();
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::SetSelection {
        id,
        expected_version: version,
        selection: Selection { anchor: 0, head: 4 },
    })
    .unwrap();
    app.dispatch(Command::FormatDocument {
        id,
        expected_version: version,
        range: None,
    })
    .unwrap();
    idle(&mut app);
    assert!(app.formatting.error.is_none(), "{:?}", app.formatting.error);
    let formatted = app.active_document().unwrap().text().to_owned();
    assert!(formatted.contains("first: int"));
    assert!(formatted.contains("    return first + second"));
    assert!(!formatted.replace("\r\n", "").contains('\n'));
    assert_eq!(fs::read_to_string(&path).unwrap(), source);
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), source);
    app.dispatch(Command::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), formatted);
    app.dispatch(Command::Save { id }).unwrap();
    idle(&mut app);
    assert_eq!(fs::read_to_string(path).unwrap(), formatted);
}
