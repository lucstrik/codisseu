//! Selection batches use the real worker; successful Trash runs use a private child profile.
use codisseu_core::{AppService, Command, ProjectFileCommand, Selection};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(25);
    while app.is_busy() || app.project_files.pending {
        app.poll();
        assert!(
            Instant::now() < until,
            "{} / {:?}",
            app.status,
            app.project_files.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn open(root: &Path) -> AppService {
    let mut app = AppService::new(root.parent().unwrap().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.to_owned()))
        .unwrap();
    settle(&mut app);
    app
}

fn batch(app: &mut AppService, paths: Vec<PathBuf>) -> Result<(), String> {
    app.dispatch(Command::ProjectFile(ProjectFileCommand::TrashMany {
        paths,
    }))?;
    settle(app);
    match &app.project_files.error {
        Some(error) => Err(error.clone()),
        None => Ok(()),
    }
}

#[test]
fn batch_rejects_empty_root_and_any_dirty_member_without_moving_other_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join("folder")).unwrap();
    let first = root.join("first.txt");
    let nested = root.join("folder/draft.txt");
    fs::write(&first, "first bytes").unwrap();
    fs::write(&nested, "saved bytes").unwrap();
    let mut app = open(&root);
    app.dispatch(Command::OpenFile(nested.clone())).unwrap();
    settle(&mut app);
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "dirty draft".into(),
    })
    .unwrap();
    for paths in [
        vec![],
        vec![first.clone(), root.clone()],
        vec![first.clone(), root.join("folder")],
    ] {
        assert!(batch(&mut app, paths).is_err());
        assert!(!app.project_files.pending);
        assert!(app.project_files.trashed.is_empty());
        assert_eq!(fs::read_to_string(&first).unwrap(), "first bytes");
        assert_eq!(fs::read_to_string(&nested).unwrap(), "saved bytes");
        assert_eq!(app.active_document().unwrap().text(), "dirty draft");
    }
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "saved bytes");
}

#[test]
fn late_invalid_member_prevents_every_mutation_and_does_not_close_clean_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join(".git")).unwrap();
    let first = root.join("first.txt");
    fs::write(&first, "first bytes").unwrap();
    fs::write(root.join(".git/config"), "git bytes").unwrap();
    let mut app = open(&root);
    app.dispatch(Command::OpenFile(first.clone())).unwrap();
    settle(&mut app);
    let id = app.active_document().unwrap().id;
    for invalid in [
        root.join(".git/config"),
        root.join("missing.txt"),
        root.join("folder/../first.txt"),
    ] {
        assert!(batch(&mut app, vec![first.clone(), invalid]).is_err());
        assert!(app.project_files.trashed.is_empty());
        assert_eq!(fs::read_to_string(&first).unwrap(), "first bytes");
        assert_eq!(app.active_document().unwrap().id, id);
    }
    assert_eq!(
        fs::read_to_string(root.join(".git/config")).unwrap(),
        "git bytes"
    );
}

#[cfg(unix)]
#[test]
fn any_selected_symlink_or_symlink_ancestor_rejects_the_whole_batch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let outside = dir.path().join("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("first.txt"), "inside").unwrap();
    fs::write(outside.join("secret.txt"), "outside").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
    let mut app = open(&root);
    for path in [root.join("link"), root.join("link/secret.txt")] {
        assert!(batch(&mut app, vec![root.join("first.txt"), path]).is_err());
        assert!(app.project_files.trashed.is_empty());
        assert_eq!(
            fs::read_to_string(root.join("first.txt")).unwrap(),
            "inside"
        );
        assert_eq!(
            fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "outside"
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn recoverable_batch_trash_deduplicates_folders_and_acknowledges_after_cancel() {
    if std::env::var_os("CODISSEU_PRIVATE_BATCH_TRASH_CHILD").is_none() {
        let owned = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts");
        fs::create_dir_all(&owned).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("codisseu-batch-trash-")
            .tempdir_in(fs::canonicalize(owned).unwrap())
            .unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("first.txt"), "recover first 😀\n").unwrap();
        fs::write(root.join("folder/nested.txt"), "recover nested\n").unwrap();
        fs::write(root.join("keep.txt"), "saved keep\n").unwrap();
        let data = dir.path().join("data");
        fs::create_dir(&data).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recoverable_batch_trash_deduplicates_folders_and_acknowledges_after_cancel",
                "--nocapture",
            ])
            .env("CODISSEU_PRIVATE_BATCH_TRASH_CHILD", &root)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!root.join("first.txt").exists());
        assert!(!root.join("folder").exists());
        assert_eq!(
            fs::read_to_string(data.join("Trash/files/first.txt")).unwrap(),
            "recover first 😀\n"
        );
        assert_eq!(
            fs::read_to_string(data.join("Trash/files/folder/nested.txt")).unwrap(),
            "recover nested\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("keep.txt")).unwrap(),
            "saved keep\n"
        );
        let infos = fs::read_dir(data.join("Trash/info")).unwrap().count();
        assert_eq!(
            infos, 2,
            "selected parent/child and duplicate file must trash only once"
        );
        return;
    }
    let root = PathBuf::from(std::env::var_os("CODISSEU_PRIVATE_BATCH_TRASH_CHILD").unwrap());
    let mut app = open(&root);
    let first = root.join("first.txt");
    let nested = root.join("folder/nested.txt");
    let keep = root.join("keep.txt");
    let mut ids = Vec::new();
    for path in [&first, &nested, &keep] {
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        settle(&mut app);
        ids.push(app.active_document().unwrap().id);
    }
    let keep_version = app.active_document().unwrap().version;
    let first_version = app
        .documents
        .iter()
        .find(|doc| doc.id == ids[0])
        .unwrap()
        .version;
    app.dispatch(Command::ProjectFile(ProjectFileCommand::TrashMany {
        paths: vec![nested, first.clone(), root.join("folder"), first.clone()],
    }))
    .unwrap();
    assert!(app.project_files.pending);
    let blocked = app
        .dispatch(Command::Edit {
            id: ids[0],
            expected_version: first_version,
            text: "must not race".into(),
        })
        .unwrap_err();
    assert!(blocked.contains("Aguarde"), "{blocked}");
    assert!(
        app.dispatch(Command::Close {
            id: ids[1],
            discard: false
        })
        .is_err()
    );
    let blocked_replace = app
        .dispatch(Command::ReplaceMatches {
            id: ids[0],
            expected_version: first_version,
            query: "recover".into(),
            replacement: "must not race".into(),
            selection: None,
        })
        .unwrap_err();
    assert!(blocked_replace.contains("Aguarde"), "{blocked_replace}");
    assert!(
        app.dispatch(Command::CreateFile(root.join("folder/new.txt")))
            .unwrap_err()
            .contains("Aguarde")
    );
    assert!(app.dispatch(Command::OpenWorkspace(root.clone())).is_err());
    app.dispatch(Command::Edit {
        id: ids[2],
        expected_version: keep_version,
        text: "keep dirty 😀\n".into(),
    })
    .unwrap();
    let keep_version = app.active_document().unwrap().version;
    app.dispatch(Command::SetSelection {
        id: ids[2],
        expected_version: keep_version,
        selection: Selection { anchor: 2, head: 7 },
    })
    .unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    while first.exists() || root.join("folder").exists() {
        assert!(
            Instant::now() < until,
            "real worker did not move both sources"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    // Cancellation must not throw away receipts after the two committed moves.
    app.dispatch(Command::CancelAll).unwrap();
    settle(&mut app);
    assert_eq!(app.project_files.trashed, vec![first, root.join("folder")]);
    assert_eq!(app.documents.len(), 1);
    let doc = app.active_document().unwrap();
    assert_eq!(doc.id, ids[2]);
    assert_eq!(doc.path, keep);
    assert_eq!(doc.text(), "keep dirty 😀\n");
    assert_eq!(doc.selection(), Selection { anchor: 2, head: 7 });
    assert!(doc.is_dirty());
    app.dispatch(Command::Undo { id: ids[2] }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "saved keep\n");
    assert!(app.entries.iter().all(|entry| {
        !app.project_files
            .trashed
            .iter()
            .any(|path| entry.path.starts_with(path))
    }));
}

#[cfg(target_os = "linux")]
#[test]
fn recoverable_batch_trash_spans_declared_roots_without_touching_carrier_siblings() {
    if std::env::var_os("CODISSEU_PRIVATE_MULTIROOT_TRASH_CHILD").is_none() {
        let owned = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts");
        fs::create_dir_all(&owned).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("codisseu-multiroot-trash-")
            .tempdir_in(fs::canonicalize(owned).unwrap())
            .unwrap();
        for name in ["a", "b", "excluded"] {
            fs::create_dir(dir.path().join(name)).unwrap();
            fs::write(dir.path().join(name).join("item.txt"), name).unwrap();
        }
        let file = dir.path().join("team.code-workspace");
        fs::write(&file, r#"{"folders":[{"path":"a"},{"path":"b"}]}"#).unwrap();
        let data = dir.path().join("data");
        fs::create_dir(&data).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recoverable_batch_trash_spans_declared_roots_without_touching_carrier_siblings",
                "--nocapture",
            ])
            .env("CODISSEU_PRIVATE_MULTIROOT_TRASH_CHILD", &file)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", dir.path().join("config"))
            .env("XDG_CACHE_HOME", dir.path().join("cache"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!dir.path().join("a/item.txt").exists());
        assert!(!dir.path().join("b/item.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.path().join("excluded/item.txt")).unwrap(),
            "excluded"
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            r#"{"folders":[{"path":"a"},{"path":"b"}]}"#
        );
        let mut contents = fs::read_dir(data.join("Trash/files"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<Vec<_>>();
        contents.sort();
        assert_eq!(contents, ["a", "b"]);
        return;
    }
    let file = PathBuf::from(std::env::var_os("CODISSEU_PRIVATE_MULTIROOT_TRASH_CHILD").unwrap());
    let base = file.parent().unwrap();
    let mut app = AppService::new(base.join("profile/session.json"));
    app.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut app);
    let selected = vec![base.join("a/item.txt"), base.join("b/item.txt")];
    assert!(
        batch(
            &mut app,
            vec![selected[0].clone(), base.join("excluded/item.txt")]
        )
        .is_err()
    );
    assert!(selected.iter().all(|path| path.exists()));
    assert!(batch(&mut app, vec![selected[0].clone(), base.join("b")]).is_err());
    assert!(selected.iter().all(|path| path.exists()));
    batch(&mut app, selected.clone()).unwrap();
    assert_eq!(app.project_files.trashed, selected);
    assert_eq!(app.workspace_path(), Some(file.as_path()));
    assert_eq!(app.workspace_roots(), [base.join("a"), base.join("b")]);
}
