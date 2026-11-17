//! Real Cargo acceptance on own, dependency-free sources copied outside this repository.
use codisseu_core::rust_project::{
    CargoAction, CargoOptions, CargoPlan, cargo_program, discover_manifest, plan,
};
use codisseu_core::{AppService, Command as AppCommand};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};

const FIXTURE_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "README.md",
    "library/Cargo.toml",
    "library/src/lib.rs",
    "application/Cargo.toml",
    "application/src/main.rs",
    "application/tests/cli.rs",
];

fn fixture_source() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rust-workspace")
        .canonicalize()
        .unwrap()
}

fn copy_fixture(base: &Path) -> PathBuf {
    let source = fixture_source();
    let root = base.join("Cargo ação ; literal workspace");
    for name in FIXTURE_FILES {
        let destination = root.join(name);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(source.join(name), destination).unwrap();
    }
    root.canonicalize().unwrap()
}

fn execute(task: &CargoPlan, target_dir: &Path, records: &mut Vec<Value>) -> Output {
    let mut process = Command::new(&task.program);
    process
        .args(&task.args)
        .current_dir(&task.cwd)
        .env_clear()
        .env("CARGO_TARGET_DIR", target_dir)
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "HOME",
        "USERPROFILE",
        "PATH",
        "RUSTUP_HOME",
        "CARGO_HOME",
        "SYSTEMROOT",
        "TMPDIR",
        "TEMP",
    ] {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    if let Some(toolchain) = option_env!("RUSTUP_TOOLCHAIN") {
        process.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    let started = Instant::now();
    let mut child = process
        .spawn()
        .expect("installed Cargo is required for the explicit acceptance suite");
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(90) {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "Cargo exceeded 90s: {:?}\n{}\n{}",
                task.args,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    records.push(json!({
        "program": task.program,
        "cwd": task.cwd,
        "args": task.args,
        "target_dir": target_dir,
        "exit_code": output.status.code(),
        "success": output.status.success(),
        "elapsed_ms": started.elapsed().as_millis(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }));
    output
}

fn success(task: &CargoPlan, target: &Path, records: &mut Vec<Value>) -> Output {
    let output = execute(task, target, records);
    assert!(
        output.status.success(),
        "Cargo {:?} failed: {}\n{}",
        task.args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

#[test]
fn workspace_and_member_manifests_are_discovered_without_running_cargo() {
    let temp = tempfile::tempdir().unwrap();
    let root = copy_fixture(temp.path());
    assert_eq!(
        discover_manifest(&root, None).unwrap(),
        root.join("Cargo.toml")
    );
    for member in ["library", "application"] {
        let file = root.join(member).join("src").join(if member == "library" {
            "lib.rs"
        } else {
            "main.rs"
        });
        let expected = root.join(member).join("Cargo.toml");
        assert_eq!(discover_manifest(&root, Some(&file)).unwrap(), expected);
        assert_eq!(
            discover_manifest(&root.join(member), Some(&file)).unwrap(),
            expected
        );
    }
    assert!(
        discover_manifest(
            &root.join("application/src"),
            Some(&root.join("application/src/main.rs"))
        )
        .is_err(),
        "search must not escape the selected project root"
    );
    assert!(discover_manifest(&root, Some(temp.path())).is_err());
    assert!(!root.join("target").exists());
}

fn settle_application(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "application did not settle: {}",
            app.status
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
}

fn check_common_language_root(app: &mut AppService, root: &Path) {
    for relative in ["library/src/lib.rs", "application/src/main.rs"] {
        let path = root.join(relative).canonicalize().unwrap();
        app.dispatch(AppCommand::OpenFile(path.clone())).unwrap();
        settle_application(app);
        assert_eq!(app.active_document().unwrap().path, path);
        assert_eq!(app.active_document().unwrap().project_root(), root);
        assert_eq!(
            app.language_project_root().as_deref(),
            Some(root),
            "both Cargo members must share one semantic root"
        );
        assert_eq!(app.project_root_for_path(&path), root);
        assert!(
            !app.any_project_providers_running(),
            "opening sources must not execute an analyzer without trust"
        );
    }
    assert_eq!(app.documents.len(), 2);
    assert!(
        app.documents
            .iter()
            .all(|document| document.project_root() == root)
    );
}

#[test]
fn appservice_cargo_members_share_workspace_language_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = copy_fixture(temp.path());
    let mut app = AppService::new(temp.path().join("workspace-session.json"));
    settle_application(&mut app);
    app.dispatch(AppCommand::OpenWorkspace(root.clone()))
        .unwrap();
    settle_application(&mut app);
    assert_eq!(app.workspace_path(), Some(root.as_path()));
    check_common_language_root(&mut app, &root);
}

#[test]
fn appservice_open_file_only_discovers_common_cargo_workspace_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = copy_fixture(temp.path());
    let mut app = AppService::new(temp.path().join("file-only-session.json"));
    settle_application(&mut app);
    assert!(app.workspace_path().is_none());
    check_common_language_root(&mut app, &root);
}

#[test]
#[ignore = "explicit real Cargo/Clippy/rustdoc acceptance; executes own temporary Rust sources"]
fn real_cargo_workspace_check_build_run_test_clippy_doc_and_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = copy_fixture(temp.path());
    let target = temp.path().join("target output ; ação");
    let canonical_source = fixture_source();
    let before: Vec<_> = FIXTURE_FILES
        .iter()
        .map(|name| fs::read(canonical_source.join(name)).unwrap())
        .collect();
    let root_manifest = discover_manifest(&root, None).unwrap();
    let member_manifest =
        discover_manifest(&root, Some(&root.join("application/src/main.rs"))).unwrap();
    let cargo = cargo_program();
    let mut records = Vec::new();
    let mut options = CargoOptions {
        action: CargoAction::Check,
        offline: true,
        locked: true,
        ..Default::default()
    };
    success(
        &plan(&root, &root_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    options.action = CargoAction::Build;
    options.package = "codisseu-fixture-app".into();
    options.target = "codisseu-fixture-app".into();
    options.all_features = true;
    success(
        &plan(&root, &member_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    let executable = target.join("debug").join(if cfg!(windows) {
        "codisseu-fixture-app.exe"
    } else {
        "codisseu-fixture-app"
    });
    assert!(
        executable.is_file(),
        "Cargo must produce the real binary under CARGO_TARGET_DIR"
    );
    let marker = temp.path().join("shell-was-executed");
    let marker2 = temp.path().join("substitution-was-executed");
    let literal = format!(
        "ação 🦀 ; touch '{}'; $(touch '{}')",
        marker.display(),
        marker2.display()
    );
    options.action = CargoAction::Run;
    options.arguments = vec![
        literal.clone(),
        "spaces ' \" and\nnewline".into(),
        "--help".into(),
        String::new(),
    ];
    let output = success(
        &plan(&root, &member_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Olá, Codisseu 🦀! total=42\n"));
    assert!(stdout.contains(&format!("arg[0]={literal}\n")));
    assert!(stdout.contains("arg[1]=spaces ' \" and\nnewline\n"));
    assert!(stdout.contains("arg[2]=--help\n"));
    assert!(stdout.contains("arg[3]=\n"));
    assert!(
        !marker.exists() && !marker2.exists(),
        "program arguments must never be reinterpreted by a shell"
    );
    options.package.clear();
    options.target.clear();
    options.arguments.clear();
    options.action = CargoAction::Test;
    let output = success(
        &plan(&root, &root_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("executable_prints_unicode_and_literal_arguments ... ok"));
    assert!(stdout.contains("unicode_greeting_and_total ... ok"));
    assert!(
        stdout.contains("Doc-tests")
            || String::from_utf8_lossy(&output.stderr).contains("Doc-tests")
    );
    options.arguments = vec!["tests::unicode_greeting_and_total".into(), "--exact".into()];
    let output = success(
        &plan(&root, &root_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("tests::unicode_greeting_and_total ... ok")
    );
    options.arguments.clear();
    options.action = CargoAction::Clippy;
    success(
        &plan(&root, &root_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    options.action = CargoAction::Doc;
    success(
        &plan(&root, &root_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    assert!(
        target
            .join("doc/codisseu_fixture_library/index.html")
            .is_file()
    );
    assert!(target.join("doc/codisseu_fixture_app/index.html").is_file());
    options.action = CargoAction::Build;
    options.package = "codisseu-fixture-app".into();
    options.target = "codisseu-fixture-app".into();
    options.release = true;
    success(
        &plan(&root, &member_manifest, cargo.clone(), &options).unwrap(),
        &target,
        &mut records,
    );
    assert!(
        target
            .join("release")
            .join(executable.file_name().unwrap())
            .is_file()
    );
    options.package.clear();
    options.target.clear();
    options.release = false;
    // Failure is introduced only in the disposable copy; the fixture stays good.
    let lib = root.join("library/src/lib.rs");
    let mut text = fs::read_to_string(&lib).unwrap();
    text.push_str("\n#[cfg(test)]\n#[test]\nfn intentional_failure_for_cargo_exit_status() { assert_eq!(19 + 23, 43); }\n");
    fs::write(lib, text).unwrap();
    options.action = CargoAction::Test;
    let output = execute(
        &plan(&root, &root_manifest, cargo, &options).unwrap(),
        &target,
        &mut records,
    );
    assert!(
        !output.status.success(),
        "failed test must keep a failure exit status"
    );
    assert!(output.status.code().is_some());
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("intentional_failure_for_cargo_exit_status ... FAILED")
    );
    assert!(!root.join("target").exists());
    for (index, name) in FIXTURE_FILES.iter().enumerate() {
        assert_eq!(
            fs::read(canonical_source.join(name)).unwrap(),
            before[index],
            "canonical source changed: {name}"
        );
    }
    temp.close().unwrap();
    assert!(
        !root.exists() && !target.exists(),
        "temporary source and build output must be removed"
    );
    if let Some(directory) = std::env::var_os("CODISSEU_RUST_CARGO_ARTIFACT_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("cargo-results.json"), serde_json::to_vec_pretty(&json!({"scenario":"own two-member Cargo workspace","commands":records,"canonical_sources_unchanged":true,"temporary_copy_and_target_removed":true})).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "managed-profile migration acceptance; requires CODISSEU_PROVIDER_CONFIG and installed rust-analyzer"]
fn restored_managed_profile_adds_rust_without_transferring_old_execution_approval() {
    use codisseu_core::{Language, ProviderConfig};
    let temp = tempfile::tempdir().unwrap();
    let root = copy_fixture(temp.path());
    let session = temp.path().join("profile/session.json");
    let policy = temp.path().join("profile/providers.json");
    let source = std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("set managed provider config");
    let configs: Vec<ProviderConfig> = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
    assert_eq!(
        configs.len(),
        2,
        "this scenario requires the previous PHP/TS managed configuration"
    );
    let mut before = AppService::new(session.clone());
    before
        .dispatch(AppCommand::OpenWorkspace(root.clone()))
        .unwrap();
    settle_application(&mut before);
    before
        .dispatch(AppCommand::ConfigureProviders(configs.clone()))
        .unwrap();
    before.dispatch(AppCommand::PersistSession).unwrap();
    settle_application(&mut before);
    drop(before);
    fs::write(
        &policy,
        serde_json::to_vec(&json!({
            "schema_version":1, "automatic":true, "configs":configs,
            "approvals":[{"workspace":root,"configs":configs}]
        }))
        .unwrap(),
    )
    .unwrap();
    let mut after = AppService::new(session);
    after.dispatch(AppCommand::RestoreSession).unwrap();
    settle_application(&mut after);
    assert_eq!(after.provider_configs.len(), 2);
    after
        .dispatch(AppCommand::EnableProviderAutostart {
            profile: policy.clone(),
            fallback_config: Some(source.into()),
        })
        .unwrap();
    settle_application(&mut after);
    assert_eq!(after.provider_configs.len(), 3);
    assert!(
        after
            .provider_configs
            .iter()
            .any(|p| p.language == Language::Rust)
    );
    assert!(after.provider_startup().needs_trust);
    assert!(!after.provider_startup().trusted);
    assert!(
        after.language.statuses.is_empty(),
        "a new executable must never inherit the old approval"
    );
    let rust_configs = after.provider_configs.clone();
    after
        .dispatch(AppCommand::ConfigureProviders(rust_configs.clone()))
        .unwrap();
    assert_eq!(after.provider_configs, rust_configs);
    let persisted: Value = serde_json::from_slice(&fs::read(&policy).unwrap()).unwrap();
    assert_eq!(
        persisted["approvals"][0]["configs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}
