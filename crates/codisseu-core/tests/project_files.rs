//! Real project-tree file operations in disposable roots; Trash uses a private child profile.
use codisseu_core::{AppService, Command, ProjectFileCommand as F, Selection};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn settle(service: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(20);
    while service.is_busy() || service.project_files.pending {
        service.poll();
        assert!(
            Instant::now() < until,
            "worker timeout: {} / {:?}",
            service.status,
            service.project_files.error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn open(root: &Path) -> AppService {
    let mut app = AppService::new(root.parent().unwrap().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.to_owned()))
        .unwrap();
    settle(&mut app);
    assert_eq!(app.workspace.as_deref(), Some(root));
    app
}
fn run(app: &mut AppService, operation: F) -> Result<(), String> {
    app.last_error = None;
    app.dispatch(Command::ProjectFile(operation))?;
    settle(app);
    match app
        .project_files
        .error
        .take()
        .or_else(|| app.last_error.take())
    {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
fn root() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    (dir, root)
}

#[test]
fn creates_nested_files_and_folders_and_opens_a_shared_editable_document() {
    let (_dir, root) = root();
    let mut app = open(&root);
    run(
        &mut app,
        F::CreateFolder {
            parent: root.clone(),
            name: "app/Http/Controllers".into(),
        },
    )
    .unwrap();
    assert!(root.join("app/Http/Controllers").is_dir());
    run(
        &mut app,
        F::CreateFile {
            parent: root.clone(),
            name: "app/Http/Controllers/Ação.php".into(),
        },
    )
    .unwrap();
    let path = root.join("app/Http/Controllers/Ação.php");
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    assert_eq!(doc.path, path);
    assert_eq!(doc.text(), "");
    assert!(!doc.is_dirty());
    assert!(app.entries.iter().any(|entry| entry.path == path));
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "<?php\n// ação 😀\n".into(),
    })
    .unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert_eq!(fs::read_to_string(path).unwrap(), "<?php\n// ação 😀\n");
    assert!(!app.active_document().unwrap().is_dirty());
}

#[test]
fn creation_rejects_collisions_absolute_paths_parent_traversal_and_git_internals() {
    let (_dir, root) = root();
    fs::write(root.join("existing.txt"), "keep").unwrap();
    let mut app = open(&root);
    for name in [
        "existing.txt",
        "../outside.txt",
        "/tmp/codisseu-must-not-write",
        ".git/config",
        "bad\nname",
    ] {
        assert!(
            run(
                &mut app,
                F::CreateFile {
                    parent: root.clone(),
                    name: name.into()
                }
            )
            .is_err(),
            "{name}"
        );
    }
    assert_eq!(
        fs::read_to_string(root.join("existing.txt")).unwrap(),
        "keep"
    );
    assert!(!root.parent().unwrap().join("outside.txt").exists());
    assert!(!root.join(".git").exists());
    fs::create_dir(root.join(".git")).unwrap();
    assert!(
        run(
            &mut app,
            F::CreateFile {
                parent: root.join(".git"),
                name: "config".into()
            }
        )
        .is_err()
    );
    assert!(!root.join(".git/config").exists());
}

#[cfg(unix)]
#[test]
fn symlink_parent_cannot_create_outside_the_workspace() {
    let (dir, root) = root();
    let outside = dir.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    let mut app = open(&root);
    assert!(
        run(
            &mut app,
            F::CreateFile {
                parent: root.join("escape"),
                name: "secret.txt".into()
            }
        )
        .is_err()
    );
    assert!(!outside.join("secret.txt").exists());
}

#[test]
fn rename_preserves_dirty_buffer_selection_identity_and_history_until_explicit_save() {
    let (_dir, root) = root();
    let old = root.join("notes.txt");
    fs::write(&old, "original 😀").unwrap();
    let mut app = open(&root);
    app.dispatch(Command::OpenFile(old.clone())).unwrap();
    settle(&mut app);
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "edited 😀\n".into(),
    })
    .unwrap();
    let version = app.active_document().unwrap().version;
    app.dispatch(Command::SetSelection {
        id,
        expected_version: version,
        selection: Selection { anchor: 1, head: 4 },
    })
    .unwrap();
    run(
        &mut app,
        F::RenameFile {
            path: old.clone(),
            name: "renamed.ts".into(),
        },
    )
    .unwrap();
    let doc = app.active_document().unwrap();
    let new = root.join("renamed.ts");
    assert_eq!(doc.id, id);
    assert_eq!(doc.path, new);
    assert_eq!(doc.text(), "edited 😀\n");
    assert!(doc.is_dirty());
    assert_eq!(doc.selection(), Selection { anchor: 1, head: 4 });
    assert!(!old.exists());
    assert_eq!(fs::read_to_string(&new).unwrap(), "original 😀");
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "original 😀");
    app.dispatch(Command::Redo { id }).unwrap();
    app.dispatch(Command::Save { id }).unwrap();
    settle(&mut app);
    assert_eq!(fs::read_to_string(new).unwrap(), "edited 😀\n");
}

#[test]
fn rename_refuses_existing_target_and_folder_moves_without_changing_sources() {
    let (_dir, root) = root();
    let old = root.join("a.php");
    fs::write(&old, "<?php echo 1;").unwrap();
    fs::write(root.join("b.php"), "<?php echo 2;").unwrap();
    let mut app = open(&root);
    for name in ["b.php", "../outside.php", "new/c.php"] {
        assert!(
            run(
                &mut app,
                F::RenameFile {
                    path: old.clone(),
                    name: name.into()
                }
            )
            .is_err()
        );
    }
    assert_eq!(fs::read_to_string(&old).unwrap(), "<?php echo 1;");
    assert_eq!(
        fs::read_to_string(root.join("b.php")).unwrap(),
        "<?php echo 2;"
    );
}

#[test]
fn trash_rejects_root_and_dirty_documents_and_refresh_discovers_external_files() {
    let (_dir, root) = root();
    let path = root.join("draft.php");
    fs::write(&path, "<?php").unwrap();
    let mut app = open(&root);
    app.dispatch(Command::OpenFile(path.clone())).unwrap();
    settle(&mut app);
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "<?php // dirty".into(),
    })
    .unwrap();
    assert!(
        app.dispatch(Command::ProjectFile(F::Trash { path: path.clone() }))
            .is_err()
    );
    assert!(
        app.dispatch(Command::ProjectFile(F::Trash { path: root.clone() }))
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "<?php");
    assert_eq!(app.active_document().unwrap().text(), "<?php // dirty");
    let other = root.join("external.txt");
    fs::write(&other, "external").unwrap();
    run(&mut app, F::Refresh).unwrap();
    assert!(app.entries.iter().any(|entry| entry.path == other));
    assert_eq!(app.active_document().unwrap().id, id);
}

#[cfg(target_os = "linux")]
#[test]
fn recoverable_trash_uses_an_isolated_child_profile_and_closes_clean_documents() {
    if std::env::var_os("CODISSEU_PRIVATE_TRASH_CHILD").is_none() {
        // /tmp may be an internal mount that GIO deliberately refuses to Trash.
        // Keep both disposable source and private Trash on the workspace filesystem.
        let owned = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../artifacts");
        fs::create_dir_all(&owned).unwrap();
        let owned = fs::canonicalize(owned).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("codisseu-trash-")
            .tempdir_in(owned)
            .unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let data = dir.path().join("xdg-data");
        fs::create_dir(&data).unwrap();
        fs::write(root.join("trash-me.txt"), "recover 😀\n").unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/keep.txt"), "nested content").unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "recoverable_trash_uses_an_isolated_child_profile_and_closes_clean_documents",
                "--nocapture",
            ])
            .env("CODISSEU_PRIVATE_TRASH_CHILD", &root)
            .env("XDG_DATA_HOME", &data)
            .env("XDG_CONFIG_HOME", dir.path().join("xdg-config"))
            .env("XDG_CACHE_HOME", dir.path().join("xdg-cache"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(!root.join("trash-me.txt").exists());
        assert!(!root.join("nested").exists());
        assert_eq!(
            fs::read_to_string(data.join("Trash/files/trash-me.txt")).unwrap(),
            "recover 😀\n"
        );
        assert_eq!(
            fs::read_to_string(data.join("Trash/files/nested/keep.txt")).unwrap(),
            "nested content"
        );
        assert!(
            fs::read_to_string(data.join("Trash/info/trash-me.txt.trashinfo"))
                .unwrap()
                .contains("Path=")
        );
        return;
    }
    let root = std::path::PathBuf::from(std::env::var_os("CODISSEU_PRIVATE_TRASH_CHILD").unwrap());
    let mut app = open(&root);
    let file = root.join("trash-me.txt");
    app.dispatch(Command::OpenFile(file.clone())).unwrap();
    settle(&mut app);
    let id = app.active_document().unwrap().id;
    run(&mut app, F::Trash { path: file }).unwrap();
    assert!(app.documents.iter().all(|doc| doc.id != id));
    run(
        &mut app,
        F::Trash {
            path: root.join("nested"),
        },
    )
    .unwrap();
    assert!(
        !app.entries
            .iter()
            .any(|entry| entry.path.starts_with(root.join("nested")))
    );
}

#[test]
fn cancellation_after_a_file_is_created_still_acknowledges_the_real_mutation() {
    let (_dir, root) = root();
    let mut app = open(&root);
    let path = root.join("acknowledged.txt");
    app.dispatch(Command::ProjectFile(F::CreateFile {
        parent: root.clone(),
        name: "acknowledged.txt".into(),
    }))
    .unwrap();
    // Observe the actual mutation while deliberately withholding the completion pump.
    let until = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(app.project_files.pending);
    app.dispatch(Command::CancelAll).unwrap();
    settle(&mut app);
    assert!(!app.project_files.pending);
    assert!(path.is_file());
    assert_eq!(app.project_files.created.as_ref(), Some(&path));
    assert_eq!(app.active_document().unwrap().path, path);
    assert_eq!(app.active_document().unwrap().text(), "");
    assert!(app.project_files.revision > 0);
}

#[test]
fn refresh_preserves_project_trust_policy_without_starting_any_provider() {
    let (dir, root) = root();
    let mut app = open(&root);
    let fallback = dir.path().join("managed-providers.json");
    let configs = vec![codisseu_core::ProviderConfig {
        language: codisseu_core::Language::Php,
        program: std::path::PathBuf::from("/usr/bin/false"),
        args: vec![],
    }];
    fs::write(&fallback, serde_json::to_vec(&configs).unwrap()).unwrap();
    let profile = dir.path().join("profile/providers.json");
    app.dispatch(Command::EnableProviderAutostart {
        profile: profile.clone(),
        fallback_config: Some(fallback),
    })
    .unwrap();
    settle(&mut app);
    assert!(app.provider_startup().needs_trust);
    assert!(!app.provider_startup().trusted);
    assert!(app.language.statuses.is_empty());
    let before = app.provider_startup().clone();
    fs::write(root.join("new-file.php"), "<?php\n").unwrap();
    run(&mut app, F::Refresh).unwrap();
    assert_eq!(app.provider_startup(), &before);
    assert_eq!(app.provider_configs, configs);
    assert!(app.language.statuses.is_empty());
    assert!(
        !profile.exists(),
        "refresh must not grant trust or persist an approval"
    );
}

#[test]
fn folder_rename_and_cut_preserve_dirty_document_identity_and_saved_bytes() {
    let (_dir, root) = root();
    fs::create_dir(root.join("old")).unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("old/a.txt"), "saved").unwrap();
    let mut app = open(&root);
    app.dispatch(Command::OpenFile(root.join("old/a.txt")))
        .unwrap();
    settle(&mut app);
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    app.dispatch(Command::Edit {
        id,
        expected_version: version,
        text: "dirty".into(),
    })
    .unwrap();
    run(
        &mut app,
        F::RenameFile {
            path: root.join("old"),
            name: "renamed".into(),
        },
    )
    .unwrap();
    assert_eq!(
        app.active_document().unwrap().path,
        root.join("renamed/a.txt")
    );
    run(
        &mut app,
        F::Transfer {
            source: root.join("renamed"),
            parent: root.join("destination"),
            move_item: false,
        },
    )
    .unwrap_err();
    run(
        &mut app,
        F::Transfer {
            source: root.join("renamed"),
            parent: root.join("destination"),
            move_item: true,
        },
    )
    .unwrap();
    let doc = app.active_document().unwrap();
    assert_eq!(doc.id, id);
    assert_eq!(doc.text(), "dirty");
    assert_eq!(doc.path, root.join("destination/renamed/a.txt"));
    assert_eq!(fs::read_to_string(&doc.path).unwrap(), "saved");
    app.dispatch(Command::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), "saved");
    assert!(!root.join("renamed").exists());
}

#[test]
fn copy_folder_is_bounded_non_overwriting_and_preserves_sources() {
    let (_dir, root) = root();
    fs::create_dir_all(root.join("source/ação")).unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("source/ação/a.txt"), "Olá 😀").unwrap();
    let mut app = open(&root);
    run(
        &mut app,
        F::Transfer {
            source: root.join("source"),
            parent: root.join("destination"),
            move_item: false,
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(root.join("source/ação/a.txt")).unwrap(),
        fs::read(root.join("destination/source/ação/a.txt")).unwrap()
    );
    for (source, parent) in [
        (root.join("source"), root.join("destination")),
        (root.join("source"), root.join("source/ação")),
        (root.clone(), root.join("destination")),
    ] {
        assert!(
            run(
                &mut app,
                F::Transfer {
                    source,
                    parent,
                    move_item: true
                }
            )
            .is_err()
        );
    }
    assert!(root.join("source/ação/a.txt").is_file());
    assert!(
        !fs::read_dir(root.join("destination"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".codisseu-copy-"))
    );
}

#[cfg(unix)]
#[test]
fn copy_rejects_symlinks_nested_git_and_large_files_without_partial_target() {
    let (_dir, root) = root();
    fs::create_dir_all(root.join("source")).unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("source/a.txt"), "keep").unwrap();
    std::os::unix::fs::symlink("a.txt", root.join("source/link")).unwrap();
    let mut app = open(&root);
    let copy = || F::Transfer {
        source: root.join("source"),
        parent: root.join("destination"),
        move_item: false,
    };
    assert!(run(&mut app, copy()).is_err());
    fs::remove_file(root.join("source/link")).unwrap();
    fs::create_dir(root.join("source/.git")).unwrap();
    assert!(run(&mut app, copy()).is_err());
    fs::remove_dir(root.join("source/.git")).unwrap();
    fs::File::create(root.join("source/huge"))
        .unwrap()
        .set_len(257 * 1024 * 1024)
        .unwrap();
    assert!(run(&mut app, copy()).is_err());
    assert!(!root.join("destination/source").exists());
    assert_eq!(
        fs::read_to_string(root.join("source/a.txt")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_dir(root.join("destination")).unwrap().count(), 0);
}

#[test]
fn duplicate_in_same_folder_chooses_a_free_name_and_never_replaces_previous_copy() {
    let (_dir, root) = root();
    fs::write(root.join("notes.txt"), "original").unwrap();
    fs::write(root.join("notes copy.txt"), "keep").unwrap();
    let mut app = open(&root);
    run(
        &mut app,
        F::Transfer {
            source: root.join("notes.txt"),
            parent: root.clone(),
            move_item: false,
        },
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join("notes copy.txt")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::read_to_string(root.join("notes copy 2.txt")).unwrap(),
        "original"
    );
    fs::create_dir(root.join("folder.ext")).unwrap();
    run(
        &mut app,
        F::Transfer {
            source: root.join("folder.ext"),
            parent: root.clone(),
            move_item: false,
        },
    )
    .unwrap();
    assert!(root.join("folder.ext copy").is_dir());
}

#[test]
fn destination_open_document_is_not_aliased_when_its_disk_file_was_removed() {
    let (_dir, root) = root();
    fs::write(root.join("a.txt"), "a").unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    fs::create_dir(root.join("destination")).unwrap();
    fs::write(root.join("destination/a.txt"), "other").unwrap();
    let mut app = open(&root);
    for name in ["a.txt", "b.txt", "destination/a.txt"] {
        app.dispatch(Command::OpenFile(root.join(name))).unwrap();
        settle(&mut app);
    }
    fs::remove_file(root.join("b.txt")).unwrap();
    fs::remove_file(root.join("destination/a.txt")).unwrap();
    assert!(
        run(
            &mut app,
            F::RenameFile {
                path: root.join("a.txt"),
                name: "b.txt".into()
            }
        )
        .is_err()
    );
    for move_item in [false, true] {
        assert!(
            run(
                &mut app,
                F::Transfer {
                    source: root.join("a.txt"),
                    parent: root.join("destination"),
                    move_item
                }
            )
            .is_err()
        );
    }
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "a");
    assert!(!root.join("b.txt").exists());
    assert!(!root.join("destination/a.txt").exists());
    assert_eq!(app.documents.len(), 3);
}
