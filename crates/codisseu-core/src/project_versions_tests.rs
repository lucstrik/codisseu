use super::*;
use std::{fs, thread};

#[test]
fn declarations_preserve_platform_pin_and_requirement_meaning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("composer.json"),
        r#"{"require":{"php":"^7.4 || ^8.2"},"config":{"platform":{"php":"8.2.12"}}}"#,
    )
    .unwrap();
    fs::write(root.join(".php-version"), "8.3.4\n").unwrap();
    let version = php(root).unwrap();
    assert_eq!(version.value, "8.2.12");
    assert_eq!(version.field, "config.platform.php");
    assert!(!version.requirement);
    assert!(version.tooltip().contains("Não comprova"));
    fs::write(
        root.join("composer.json"),
        r#"{"require":{"php":"^7.4 || ^8.2"}}"#,
    )
    .unwrap();
    assert_eq!(php(root).unwrap().value, "8.3.4");
    fs::remove_file(root.join(".php-version")).unwrap();
    let version = php(root).unwrap();
    assert_eq!(version.value, "^7.4 || ^8.2");
    assert!(version.requirement);
    assert_eq!(version.source, root.join("composer.json"));
    fs::write(
        root.join("composer.json"),
        r#"{"require":{"php":">=7.4 <9.0"}}"#,
    )
    .unwrap();
    assert_eq!(php(root).unwrap().value, ">=7.4 <9.0");
}

#[test]
fn absent_invalid_and_oversized_sources_do_not_invent_a_version() {
    let dir = tempfile::tempdir().unwrap();
    assert!(php(dir.path()).is_none());
    for text in [
        "{",
        "{}",
        r#"{"config":{"platform":{"php":false}}}"#,
        r#"{"require":{"php":8.2}}"#,
        r#"{"require":{"php":"unknown"}}"#,
        r#"{"require":{"php":"8.2\nmalicious"}}"#,
    ] {
        fs::write(dir.path().join("composer.json"), text).unwrap();
        assert!(php(dir.path()).is_none(), "{text}");
    }
    fs::write(
        dir.path().join("composer.json"),
        " ".repeat(MAX_BYTES as usize + 1),
    )
    .unwrap();
    assert!(php(dir.path()).is_none());
    fs::write(dir.path().join(".php-version"), "8.2\n8.3").unwrap();
    assert!(php(dir.path()).is_none());
}

#[cfg(unix)]
#[test]
fn symlinks_and_special_files_are_not_opened() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let external = tempfile::NamedTempFile::new().unwrap();
    fs::write(external.path(), r#"{"require":{"php":"8.2"}}"#).unwrap();
    let path = dir.path().join("composer.json");
    symlink(external.path(), &path).unwrap();
    assert!(php(dir.path()).is_none());
    fs::remove_file(&path).unwrap();
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(php(dir.path()).is_none());
}

fn finish(service: &mut Service, root: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        service.tick(Some(root.into()));
        if !service.pending {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn project_switch_discards_old_replies_and_refresh_reads_changes_and_removal() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    fs::write(a.path().join(".php-version"), "7.4").unwrap();
    fs::write(b.path().join(".php-version"), "8.4").unwrap();
    let mut service = Service::default();
    service.tick(Some(a.path().into()));
    service.tick(Some(b.path().into()));
    assert!(service.version.versions.is_empty());
    finish(&mut service, b.path());
    assert_eq!(
        service
            .version
            .versions
            .get(&ProjectRuntime::Php)
            .unwrap()
            .value,
        "8.4"
    );
    fs::write(b.path().join(".php-version"), "8.3.10").unwrap();
    service.next = Instant::now();
    finish(&mut service, b.path());
    assert_eq!(
        service
            .version
            .versions
            .get(&ProjectRuntime::Php)
            .unwrap()
            .value,
        "8.3.10"
    );
    fs::remove_file(b.path().join(".php-version")).unwrap();
    service.next = Instant::now();
    finish(&mut service, b.path());
    assert!(service.version.versions.is_empty());
    service.tick(None);
    assert!(service.version.versions.is_empty());
}

#[test]
fn node_pins_and_engines_do_not_use_the_package_version_or_execute_scripts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let package = root.join("package.json");
    fs::write(&package, r#"{"version":"99.9.9","engines":{"node":">=20 <23"},"volta":{"node":"22.12.0"},"scripts":{"install":"touch sentinel"}}"#).unwrap();
    assert_eq!(node(root).unwrap().value, "22.12.0");
    fs::write(root.join(".node-version"), "22.11.0\n").unwrap();
    assert_eq!(node(root).unwrap().value, "22.11.0");
    fs::write(root.join(".nvmrc"), "lts/*\n").unwrap();
    assert_eq!(node(root).unwrap().value, "lts/*");
    fs::write(root.join(".nvmrc"), "$(touch sentinel)").unwrap();
    assert_eq!(node(root).unwrap().value, "22.11.0");
    fs::remove_file(root.join(".node-version")).unwrap();
    fs::write(
        &package,
        r#"{"engines":{"node":">=20 <23"},"version":"99.9.9"}"#,
    )
    .unwrap();
    let version = node(root).unwrap();
    assert_eq!(version.value, ">=20 <23");
    assert!(version.requirement);
    fs::write(&package, r#"{"version":"99.9.9"}"#).unwrap();
    assert!(node(root).is_none());
    assert!(detect(root).hints.contains(&ProjectRuntime::Node));
    assert!(!root.join("sentinel").exists());
}

#[test]
fn python_pin_requirement_and_invalid_toml_keep_the_project_hint() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let project = root.join("pyproject.toml");
    fs::write(
        &project,
        "[project]\nversion = '99.9.9'\nrequires-python = '>=3.10, <4'\n",
    )
    .unwrap();
    let version = python(root).unwrap();
    assert_eq!(version.value, ">=3.10, <4");
    assert_eq!(version.field, "project.requires-python");
    assert!(version.requirement);
    fs::write(root.join(".python-version"), "3.12.8\n").unwrap();
    assert_eq!(python(root).unwrap().value, "3.12.8");
    fs::write(root.join(".python-version"), "3.11\n3.12\n").unwrap();
    assert_eq!(python(root).unwrap().value, ">=3.10, <4");
    for invalid in ["not toml =", "[project]\nversion = '99.9.9'\n"] {
        fs::write(&project, invalid).unwrap();
        assert!(python(root).is_none());
        assert!(detect(root).hints.contains(&ProjectRuntime::Python));
    }
}

#[test]
fn rust_toolchain_channel_and_minimum_are_distinct_from_package_and_edition() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let cargo = root.join("Cargo.toml");
    fs::write(
        &cargo,
        "[package]\nname='sample'\nversion='99.9.9'\nedition='2024'\nrust-version='1.85'\n",
    )
    .unwrap();
    let version = rust(root).unwrap();
    assert_eq!(version.value, ">=1.85");
    assert!(version.requirement);
    fs::write(root.join("rust-toolchain"), "nightly-2026-11-01\n").unwrap();
    assert_eq!(rust(root).unwrap().value, "nightly-2026-11-01");
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel='1.90.0'\n",
    )
    .unwrap();
    assert_eq!(rust(root).unwrap().value, "1.90.0");
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel='stable'\n",
    )
    .unwrap();
    assert_eq!(rust(root).unwrap().value, "stable");
    fs::remove_file(root.join("rust-toolchain.toml")).unwrap();
    fs::write(root.join("rust-toolchain"), "[toolchain]\nchannel='beta'\n").unwrap();
    assert_eq!(rust(root).unwrap().value, "beta");
    fs::remove_file(root.join("rust-toolchain")).unwrap();
    fs::write(&cargo, "[workspace.package]\nrust-version='1.86'\n").unwrap();
    assert_eq!(rust(root).unwrap().value, ">=1.86");
    fs::write(&cargo, "[package]\nversion='99.9.9'\nedition='2024'\n").unwrap();
    assert!(rust(root).is_none());
}

#[test]
fn mixed_projects_keep_independent_versions_and_python_roots_are_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join(".php-version"), "8.2").unwrap();
    fs::write(root.join(".nvmrc"), "22").unwrap();
    fs::write(root.join(".python-version"), "3.13").unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel='stable'\n",
    )
    .unwrap();
    let versions = detect(&root);
    assert_eq!(versions.versions.len(), 4);
    for (runtime, value) in [
        (ProjectRuntime::Php, "8.2"),
        (ProjectRuntime::Node, "22"),
        (ProjectRuntime::Python, "3.13"),
        (ProjectRuntime::Rust, "stable"),
    ] {
        assert_eq!(versions.versions[&runtime].value, value);
        assert_eq!(versions.versions[&runtime].runtime, runtime);
    }
    let python = root.join("service");
    fs::create_dir_all(python.join("src")).unwrap();
    fs::write(
        python.join("pyproject.toml"),
        "[project]\nrequires-python='>=3.12'\n",
    )
    .unwrap();
    let file = python.join("src/main.py");
    fs::write(&file, "pass\n").unwrap();
    assert_eq!(crate::projects::resolve_root(Some(&root), &file), python);
    assert_eq!(
        detect(&python).versions[&ProjectRuntime::Python].value,
        ">=3.12"
    );
    assert!(!detect(&python).versions.contains_key(&ProjectRuntime::Php));
}

#[test]
fn disconnected_worker_clears_stale_data_and_recovers_without_pending_forever() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(".python-version"), "3.12.8").unwrap();
    let (sender, jobs) = mpsc::sync_channel::<Request>(1);
    drop(jobs);
    let (replies, receiver) = mpsc::sync_channel::<Response>(1);
    drop(replies);
    let mut service = Service {
        root: Some(dir.path().into()),
        epoch: 1,
        worker: Some((sender, receiver)),
        pending: true,
        next: Instant::now(),
        version: detect(dir.path()),
    };
    service.tick(Some(dir.path().into()));
    assert!(!service.pending);
    assert!(service.version.versions.is_empty());
    assert!(service.worker.is_none());
    service.next = Instant::now();
    finish(&mut service, dir.path());
    assert_eq!(
        service.version.versions[&ProjectRuntime::Python].value,
        "3.12.8"
    );
}
