use codisseu_core::{
    AppService, Command, ProjectFileCommand,
    git::GitCommand,
    search_everywhere::{self, Request},
    workspace_file::read_folders,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(Instant::now() < until, "{}", app.status);
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn send(app: &mut AppService, command: Command) {
    app.dispatch(command).unwrap();
    settle(app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
}
fn fixture(root: &Path) -> (PathBuf, Vec<PathBuf>) {
    let base = root.join("workspace");
    fs::create_dir(&base).unwrap();
    let folders = vec![
        base.join("service"),
        root.join("outside"),
        base.join("excluded"),
    ];
    for folder in &folders {
        fs::create_dir(folder).unwrap();
        fs::write(folder.join("find-me.txt"), "source bytes\n").unwrap();
        let status = std::process::Command::new("git")
            .args(["-c", "init.templateDir=", "init", "--quiet"])
            .arg(folder)
            .status()
            .unwrap();
        assert!(status.success());
    }
    let file = base.join("team.code-workspace");
    fs::write(&file, "{ // own fixture\n \"folders\": [ {\"name\":\"API ação\",\"path\":\"service\",}, {\"name\":\"Outside\",\"path\":\"../outside\"}, ], \"tasks\": {\"ignored\": true}, }").unwrap();
    (file, folders)
}

#[test]
fn declared_roots_tree_git_file_operations_and_restore_are_scoped() {
    let dir = tempfile::tempdir().unwrap();
    let (file, folders) = fixture(dir.path());
    let original = fs::read(&file).unwrap();
    let session = dir.path().join("session.json");
    let mut app = AppService::new(session.clone());
    settle(&mut app);
    send(&mut app, Command::OpenFile(file.clone()));
    assert_eq!(app.workspace_path(), Some(file.as_path()));
    assert_eq!(app.workspace_label(), "team");
    assert_eq!(app.workspace_roots(), folders[..2]);
    assert_eq!(
        app.entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
        folders[..2].iter().collect::<Vec<_>>()
    );
    for folder in &folders[..2] {
        send(&mut app, Command::LoadDirectory(folder.clone()));
        assert!(
            app.entries
                .iter()
                .any(|e| e.path == folder.join("find-me.txt") && e.depth == 1)
        );
    }
    send(&mut app, Command::OpenFile(folders[1].join("find-me.txt")));
    assert_eq!(app.active_document().unwrap().project_root(), folders[1]);
    send(
        &mut app,
        Command::ProjectFile(ProjectFileCommand::CreateFile {
            parent: folders[1].clone(),
            name: "new.txt".into(),
        }),
    );
    assert!(folders[1].join("new.txt").is_file());
    assert_eq!(app.workspace_roots(), folders[..2]);
    send(
        &mut app,
        Command::Git(GitCommand::DiscoverRepositories {
            workspace: file.clone(),
            max_depth: 6,
            max_repositories: 32,
            max_entries: 50000,
        }),
    );
    assert_eq!(app.git.repositories.len(), 2);
    assert!(!app.git.repositories.iter().any(|r| r.root == folders[2]));
    send(&mut app, Command::SelectGitRepository(folders[1].clone()));
    assert_eq!(app.git.repo.as_ref().unwrap().root, folders[1]);
    assert!(
        app.dispatch(Command::SelectGitRepository(folders[2].clone()))
            .is_err()
    );
    send(&mut app, Command::PersistSession);
    drop(app);
    let mut app = AppService::new(session);
    settle(&mut app);
    send(&mut app, Command::RestoreSession);
    assert_eq!(app.workspace_path(), Some(file.as_path()));
    assert_eq!(app.workspace_roots(), folders[..2]);
    assert_eq!(app.documents.len(), 2);
    assert!(!app.providers_running());
    assert!(app.remote_db.active.is_none());
    assert_eq!(fs::read(file).unwrap(), original);
}

#[test]
fn jsonc_errors_preserve_workspace_and_missing_folders_do_not_hide_available_roots() {
    let dir = tempfile::tempdir().unwrap();
    let (file, _) = fixture(dir.path());
    assert_eq!(read_folders(&file).unwrap()[0].name, "API ação");
    let mut app = AppService::new(dir.path().join("session.json"));
    settle(&mut app);
    send(&mut app, Command::OpenWorkspace(file.clone()));
    let bad = dir.path().join("bad.code-workspace");
    for text in [
        "{ /* unfinished",
        "{\"folders\":[{\"path\":3}]}",
        "{\"folders\":[]}",
    ] {
        fs::write(&bad, text).unwrap();
        assert!(read_folders(&bad).is_err());
        app.dispatch(Command::OpenWorkspace(bad.clone())).unwrap();
        settle(&mut app);
        assert!(app.last_error.is_some());
        assert_eq!(app.workspace_path(), Some(file.as_path()));
    }
    // Keep the existing roots and append an unavailable folder as in the user's workspace.
    let missing = file.parent().unwrap().join("missing");
    fs::write(
        &file,
        r#"{"folders":[{"path":"service"},{"name":"Planos","path":"missing"}]}"#,
    )
    .unwrap();
    send(&mut app, Command::OpenWorkspace(file.clone()));
    assert_eq!(app.workspace_folders.len(), 2);
    assert_eq!(app.workspace_roots().len(), 1);
    assert!(!app.workspace_folders[1].available);
    assert!(
        app.tree_directory_error(&missing)
            .unwrap()
            .contains("Pasta ausente")
    );
    assert!(app.workspace_root_for(&missing).is_none());
    assert!(!missing.exists());
}

#[test]
fn search_includes_external_declared_root_and_excludes_unlisted_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let (file, folders) = fixture(dir.path());
    let mut search = search_everywhere::SearchEverywhereService::new();
    search
        .submit(Request {
            encoding_profile: None,
            root: Some(file),
            external_roots: vec![],
            buffers: vec![],
            query: "find-me".into(),
            category: search_everywhere::Category::Files,
            filter: search_everywhere::FileFilter::All,
            include_non_project: false,
            only_open: false,
            refresh: 0,
            preview: false,
            contributors: search_everywhere::ALL_CONTRIBUTORS,
        })
        .unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    while search.pending() {
        search.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(search.error.is_none(), "{:?}", search.error);
    let hits = &search.result.as_ref().unwrap().hits;
    assert_eq!(hits.len(), 2);
    assert!(
        hits.iter()
            .any(|h| h.path == folders[1].join("find-me.txt"))
    );
    assert!(!hits.iter().any(|h| h.path.starts_with(&folders[2])));
}

#[test]
fn git_initial_binding_skips_non_git_and_missing_roots_and_never_enters_excluded_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let (file, folders) = fixture(dir.path());
    let docs = file.parent().unwrap().join("docs");
    fs::create_dir(&docs).unwrap();
    fs::write(
        &file,
        serde_json::to_vec(&serde_json::json!({"folders":[
            {"path":"docs"},{"path":"missing"},{"path":"service"},{"path":"../outside"}
        ]}))
        .unwrap(),
    )
    .unwrap();
    let original = fs::read(&file).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    settle(&mut app);
    send(&mut app, Command::OpenWorkspace(file.clone()));
    assert_eq!(app.git_selected_repository(), Some(folders[0].as_path()));
    assert!(app.git.error.is_none(), "{:?}", app.git.error);
    assert_eq!(
        app.git
            .repositories
            .iter()
            .map(|r| &r.root)
            .collect::<Vec<_>>(),
        folders[..2].iter().collect::<Vec<_>>()
    );
    assert_eq!(app.git.discovery_workspace.as_deref(), Some(file.as_path()));
    assert!(!app.git.trusted);
    assert_eq!(app.workspace_default_root(), Some(docs.as_path()));
    assert_eq!(app.encoding_project_root(), Some(docs.as_path()));
    assert_eq!(app.language_project_root(), Some(docs.clone()));
    send(&mut app, Command::SelectGitRepository(folders[1].clone()));
    send(&mut app, Command::Git(GitCommand::Trust(true)));
    let generation = app.git.repo_generation;
    // Even a caller supplying an unrelated discovery root cannot widen folders.
    send(
        &mut app,
        Command::Git(GitCommand::DiscoverRoots {
            workspace: folders[2].clone(),
            roots: vec![folders[2].clone()],
        }),
    );
    assert_eq!(app.git_selected_repository(), Some(folders[1].as_path()));
    assert_eq!(app.git.repo_generation, generation);
    assert!(app.git.trusted);
    assert!(!app.git.repositories.iter().any(|r| r.root == folders[2]));
    let revision = app.git.revision;
    for _ in 0..30 {
        app.poll();
    }
    assert_eq!(
        app.git.revision, revision,
        "idle polling must not keep discovering/rebinding"
    );
    send(
        &mut app,
        Command::Git(GitCommand::Prepare(
            codisseu_core::git::LocalAction::Stage {
                paths: vec!["find-me.txt".into()],
            },
        )),
    );
    let preview = app.git.preview.clone().unwrap();
    send(
        &mut app,
        Command::Git(GitCommand::Apply {
            preview_id: preview.id,
            repo_generation: preview.generation,
        }),
    );
    for (index, root) in folders.iter().enumerate() {
        let result = std::process::Command::new("git")
            .arg("--no-optional-locks")
            .arg("-C")
            .arg(root)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(
            String::from_utf8(result.stdout).unwrap(),
            if index == 1 { "find-me.txt\n" } else { "" },
            "index of {}",
            root.display()
        );
    }
    send(&mut app, Command::PersistSession);
    drop(app);
    let mut restored = AppService::new(dir.path().join("session.json"));
    settle(&mut restored);
    send(&mut restored, Command::RestoreSession);
    assert!(restored.git.error.is_none(), "{:?}", restored.git.error);
    assert_eq!(restored.git.repositories.len(), 2);
    assert_eq!(fs::read(file).unwrap(), original);
}

#[test]
fn different_workspace_files_in_same_directory_and_changed_folders_discard_stale_git_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let (file, folders) = fixture(dir.path());
    let other = file.parent().unwrap().join("other.code-workspace");
    fs::write(&other, r#"{"folders":[{"path":"excluded"}]}"#).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    settle(&mut app);
    send(&mut app, Command::OpenWorkspace(file.clone()));
    send(&mut app, Command::Git(GitCommand::Trust(true)));
    let generation = app.git.repo_generation;
    send(&mut app, Command::OpenWorkspace(other.clone()));
    assert_eq!(app.git_selected_repository(), Some(folders[2].as_path()));
    assert!(app.git.repo_generation > generation);
    assert!(!app.git.trusted);
    assert_eq!(app.git.repositories.len(), 1);
    assert_eq!(
        app.git.discovery_workspace.as_deref(),
        Some(other.as_path())
    );
    assert!(
        app.dispatch(Command::SelectGitRepository(folders[0].clone()))
            .is_err()
    );
    // Reloading the same workspace file must also honor its new folders list.
    fs::write(&other, r#"{"folders":[{"path":"../outside"}]}"#).unwrap();
    send(&mut app, Command::OpenWorkspace(other.clone()));
    assert_eq!(app.git_selected_repository(), Some(folders[1].as_path()));
    assert_eq!(app.git.repositories.len(), 1);
    fs::write(&other, r#"{"folders":[{"path":"absent"}]}"#).unwrap();
    send(&mut app, Command::OpenWorkspace(other.clone()));
    assert!(app.git.repo.is_none());
    assert!(app.git.repositories.is_empty());
    assert!(app.git.error.is_none());
    assert!(app.workspace_default_root().is_none());
    assert!(app.encoding_project_root().is_none());
    assert!(app.language_project_root().is_none());
    let revision = app.git.revision;
    for _ in 0..30 {
        app.poll();
    }
    assert_eq!(app.git.revision, revision);
    assert!(!app.git.is_busy());
}

#[test]
fn declared_subfolder_can_bind_its_actual_ancestor_repository_but_not_an_unlisted_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let (file, folders) = fixture(dir.path());
    let sub = folders[0].join("src");
    fs::create_dir(&sub).unwrap();
    fs::write(
        &file,
        r#"{"folders":[{"name":"Source only","path":"service/src"}]}"#,
    )
    .unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    settle(&mut app);
    send(&mut app, Command::OpenWorkspace(file));
    assert_eq!(app.git_selected_repository(), Some(folders[0].as_path()));
    assert!(app.git.error.is_none(), "{:?}", app.git.error);
    assert_eq!(app.git_workspace_for(&folders[0]), Some(sub.as_path()));
    assert!(
        app.dispatch(Command::SelectGitRepository(folders[2].clone()))
            .is_err()
    );
    app.last_error = None; // The preceding rejection is expected.
    send(&mut app, Command::SelectGitRepository(folders[0].clone()));
    assert_eq!(app.git_selected_repository(), Some(folders[0].as_path()));
}

#[test]
fn initializing_a_declared_folder_retains_workspace_file_and_excludes_parent() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    let project = dir.path().join("project");
    fs::create_dir(&config).unwrap();
    fs::create_dir(&project).unwrap();
    let file = config.join("team.code-workspace");
    fs::write(&file, r#"{"folders":[{"path":"../project"}]}"#).unwrap();
    let original = fs::read(&file).unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    settle(&mut app);
    send(&mut app, Command::OpenWorkspace(file.clone()));
    assert!(app.git.repo.is_none());
    send(
        &mut app,
        Command::Git(GitCommand::Init {
            path: project.clone(),
            initial_branch: "main".into(),
        }),
    );
    assert_eq!(app.workspace_path(), Some(file.as_path()));
    assert_eq!(app.git_selected_repository(), Some(project.as_path()));
    assert_eq!(app.git.repositories.len(), 1);
    assert!(!config.join(".git").exists());
    assert!(!dir.path().join(".git").exists());
    assert_eq!(fs::read(file).unwrap(), original);
}
