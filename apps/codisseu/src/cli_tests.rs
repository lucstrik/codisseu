use super::*;
use std::{
    ffi::OsString,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "codisseu-cli-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn parse(args: &[&str], cwd: &Path) -> Options {
    parse_options(args.iter().map(OsString::from), cwd)
        .unwrap()
        .unwrap()
}

#[test]
fn dot_relative_and_absolute_paths_use_the_terminal_cwd_with_spaces_and_unicode() {
    let f = Fixture::new();
    let project = f.0.join("Projeto ação com espaços");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("Main.php"), "<?php").unwrap();
    assert_eq!(parse(&["."], &project).workspace, Some(project.clone()));
    assert_eq!(
        parse(&["Projeto ação com espaços"], &f.0).workspace,
        Some(project.clone())
    );
    let opts = parse(
        &[
            project.to_str().unwrap(),
            "Projeto ação com espaços/Main.php",
        ],
        &f.0,
    );
    assert_eq!(opts.workspace, Some(project.clone()));
    assert_eq!(opts.files, [project.join("Main.php")]);
}

#[test]
fn no_arguments_allow_restoration_in_both_desktop_and_terminal_modes() {
    let desktop = parse(&[], Path::new("/"));
    assert!(
        !desktop.cli_mode
            && !desktop.no_restore
            && desktop.workspace.is_none()
            && desktop.files.is_empty()
    );
    let cli = parse(&["--cli", "--wait"], Path::new("/"));
    assert!(cli.cli_mode && cli.wait && cli.workspace.is_none() && cli.files.is_empty());
    assert!(parse(&["-n"], Path::new("/")).new_window);
}

#[test]
fn normal_launches_share_the_last_session_and_new_windows_keep_a_stable_profile() {
    let f = Fixture::new();
    let default = f.0.join("state/session.json");
    for args in [vec![], vec!["--cli", "--wait"], vec!["--cli"]] {
        let mut options = parse(&args, &f.0);
        cli::prepare_profile(&mut options, &default).unwrap();
        assert_eq!(options.session.as_ref(), Some(&default));
        assert_eq!(options.profile.as_ref(), Some(&default));
        assert!(!options.no_restore);
    }
    let mut fresh = parse(&["-n"], &f.0);
    cli::prepare_profile(&mut fresh, &default).unwrap();
    assert_ne!(fresh.session.as_ref(), Some(&default));
    assert_eq!(fresh.profile.as_ref(), Some(&default));
    assert!(fresh.no_restore);
    let mut child = parse_options(cli::gui_arguments(&fresh), &f.0)
        .unwrap()
        .unwrap();
    cli::prepare_profile(&mut child, &default).unwrap();
    assert_eq!(child.profile, fresh.profile);
    assert_eq!(child.session, fresh.session);
    assert!(child.no_restore);
    let mut isolated_new = parse(&["-n", "--session", "private.json"], &f.0);
    cli::prepare_profile(&mut isolated_new, &default).unwrap();
    assert_ne!(isolated_new.session, Some("private.json".into()));
    assert_eq!(isolated_new.profile, Some("private.json".into()));
}

#[test]
fn explicit_profiles_are_isolated_and_explicit_projects_override_restoration() {
    let f = Fixture::new();
    std::fs::create_dir(f.0.join("project")).unwrap();
    let default = f.0.join("normal/session.json");
    let mut options = parse(&["--session", "private.json", "project"], &f.0);
    cli::prepare_profile(&mut options, &default).unwrap();
    assert_eq!(options.profile, Some("private.json".into()));
    assert_eq!(options.session, Some("private.json".into()));
    assert_eq!(options.workspace, Some(f.0.join("project")));
    // Startup only restores in the absence of an explicit workspace or file.
    assert!(options.workspace.is_some());
    let mut empty = parse(&["--cli", "--no-restore"], &f.0);
    cli::prepare_profile(&mut empty, &default).unwrap();
    assert!(empty.no_restore);
}

#[test]
fn separator_preserves_option_shaped_names_and_never_evaluates_shell_text() {
    let f = Fixture::new();
    std::fs::create_dir(f.0.join("--open")).unwrap();
    let filename = "$(touch marker) `command` ' ação.md";
    std::fs::write(f.0.join(filename), "own fixture").unwrap();
    let options = parse(&["--", "--open", filename], &f.0);
    assert_eq!(options.workspace, Some(f.0.join("--open")));
    assert_eq!(options.files, [f.0.join(filename)]);
    assert!(!f.0.join("marker").exists());
}

#[test]
fn missing_paths_unknown_flags_multiple_folders_and_invalid_capture_are_errors() {
    let f = Fixture::new();
    for args in [
        vec!["missing"],
        vec!["--misspelled"],
        vec!["--capture"],
        vec!["--close-after-capture"],
        vec!["--wait-diagnostics"],
    ] {
        assert!(parse_options(args.into_iter().map(OsString::from), &f.0).is_err());
    }
    std::fs::create_dir(f.0.join("one")).unwrap();
    std::fs::create_dir(f.0.join("two")).unwrap();
    assert!(parse_options(["one", "two"].map(OsString::from), &f.0).is_err());
}

#[test]
fn workspace_file_and_existing_explicit_flags_keep_their_meaning() {
    let f = Fixture::new();
    std::fs::write(f.0.join("Mixed.code-workspace"), "{\"folders\":[]}").unwrap();
    assert_eq!(
        parse(&["Mixed.code-workspace"], &f.0).workspace,
        Some(f.0.join("Mixed.code-workspace"))
    );
    let options = parse(
        &[
            "--workspace",
            "one",
            "--open",
            "two",
            "--session",
            "profile.json",
            "--no-auto-providers",
            "--no-restore",
        ],
        &f.0,
    );
    assert_eq!(options.workspace, Some("one".into()));
    assert_eq!(options.files, [PathBuf::from("two")]);
    assert!(options.no_restore && options.no_auto_providers);
}

#[test]
fn detached_arguments_round_trip_literals_without_recursive_cli_or_inherited_actions() {
    let f = Fixture::new();
    let options = parse(
        &[
            "--cli",
            "--wait",
            "--session",
            "profile 'quoted'.json",
            "--open",
            "file with spaces.php",
            "--browser",
            "https://example.invalid/a?b=c&d=e",
            "--no-restore",
            "--no-auto-providers",
            "--capture",
            "capture image.png",
        ],
        &f.0,
    );
    let args = cli::gui_arguments(&options);
    assert!(!args.contains(&OsString::from("--cli")) && !args.contains(&OsString::from("--wait")));
    let child = parse_options(args, &f.0).unwrap().unwrap();
    assert_eq!(child.session, options.session);
    assert_eq!(child.files, options.files);
    assert_eq!(child.browsers, options.browsers);
    assert_eq!(child.capture, options.capture);
    assert!(
        child.no_restore && child.no_auto_providers && !child.start_providers && !child.execute_sql
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_and_symlink_paths_are_supported_without_lossy_argv_conversion() {
    use std::os::unix::{ffi::OsStringExt, fs::symlink};
    let f = Fixture::new();
    let filename = OsString::from_vec(vec![b'v', 0xff, b'.', b't', b's']);
    std::fs::write(f.0.join(&filename), "const value = 1;").unwrap();
    symlink(f.0.join(&filename), f.0.join("alias.ts")).unwrap();
    let opts = parse_options([filename.clone()], &f.0).unwrap().unwrap();
    assert_eq!(opts.files, [f.0.join(&filename)]);
    assert_eq!(parse(&["alias.ts"], &f.0).files, opts.files);
}

#[cfg(unix)]
#[test]
fn immediate_spawn_failure_reports_private_log_and_does_not_claim_success() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let f = Fixture::new();
    let program = f.0.join("owned launcher");
    std::fs::write(
        &program,
        "#!/bin/sh\nprintf 'own startup failure\\n' >&2\nexit 7\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let options = Options {
        session: Some(f.0.join("profile.json")),
        no_restore: true,
        ..Default::default()
    };
    let error = cli::launch(&program, &options).unwrap_err();
    assert!(error.contains("não conseguiu iniciar") && error.contains("cli-logs"));
    let log = std::fs::read_dir(f.0.join("cli-logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "own startup failure\n"
    );
    assert_eq!(log.metadata().unwrap().mode() & 0o777, 0o600);
    assert!(cli::launch(&f.0.join("missing binary"), &options).is_err());
}
