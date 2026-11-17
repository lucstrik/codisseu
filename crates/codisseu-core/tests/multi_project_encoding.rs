use codisseu_core::{AppService, Command, FileEncoding, ProjectFileCommand};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn settle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    while app.is_busy() {
        app.poll();
        assert!(Instant::now() < end, "{} {:?}", app.status, app.last_error);
        std::thread::sleep(Duration::from_millis(2));
    }
    app.poll();
}
fn run(app: &mut AppService, command: Command) {
    app.dispatch(command).unwrap();
    settle(app);
    assert!(app.last_error.is_none(), "{:?}", app.last_error);
}
fn project(root: &Path, name: &str, charset: &str, bytes: &[u8]) -> PathBuf {
    let dir = root.join(name);
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("composer.json"), "{}").unwrap();
    fs::write(dir.join(".editorconfig"),format!("root = true\n[*]\nindent_style = space\n[*.php]\ncharset = {charset}\n[*.json]\ncharset = utf-8\n")).unwrap();
    let path = dir.join("src/a.php");
    fs::write(&path, bytes).unwrap();
    path
}
#[test]
fn mixed_services_keep_bytes_ownership_and_scoped_encoding_with_dirty_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("example_company");
    fs::create_dir(&root).unwrap();
    let a = project(&root, "utf8", "utf-8", "<?php // ação 😀\r\n".as_bytes());
    let b = project(&root, "legacy", "latin1", b"<?php // a\xe7\xe3o\r\n");
    let c = project(
        &root,
        "windows",
        "windows-1252",
        b"<?php // \x80 \x93a\x94\r\n",
    );
    let originals = [&a, &b, &c].map(|p| fs::read(p).unwrap());
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    run(&mut app, Command::OpenWorkspace(root.clone()));
    // A parent default must not override declarations of child services.
    run(
        &mut app,
        Command::SetFolderEncoding {
            root: root.clone(),
            encoding: FileEncoding::Latin1,
        },
    );
    for path in [&a, &b, &c] {
        run(&mut app, Command::OpenFile(path.clone()));
    }
    assert_eq!(
        app.documents
            .iter()
            .map(|d| d.encoding())
            .collect::<Vec<_>>(),
        vec![
            FileEncoding::Utf8,
            FileEncoding::Latin1,
            FileEncoding::Windows1252
        ]
    );
    for d in &app.documents {
        assert_eq!(d.project_root(), d.path.parent().unwrap().parent().unwrap());
    }
    assert_eq!(app.documents[1].text(), "<?php // ação\r\n");
    assert_eq!(app.documents[2].text(), "<?php // € “a”\r\n");
    let id = app.documents[0].id;
    let version = app.documents[0].version;
    run(
        &mut app,
        Command::Edit {
            id,
            expected_version: version,
            text: "<?php // edição pendente 😀\r\n".into(),
        },
    );
    let sibling_version = app.documents[0].version;
    let legacy_id = app.documents[1].id;
    run(&mut app, Command::Activate { id: legacy_id });
    run(
        &mut app,
        Command::SetProjectEncoding(FileEncoding::Windows1252),
    );
    assert_eq!(app.documents[0].version, sibling_version);
    assert!(app.documents[0].is_dirty());
    assert_eq!(app.documents[1].encoding(), FileEncoding::Windows1252);
    assert_eq!(app.documents[2].encoding(), FileEncoding::Windows1252);
    for (path, bytes) in [&a, &b, &c].into_iter().zip(originals) {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    run(&mut app, Command::Undo { id });
    assert!(!app.documents[0].is_dirty());
    run(&mut app, Command::PersistSession);
    drop(app);
    let mut restored = AppService::new(dir.path().join("profile/session.json"));
    run(&mut restored, Command::RestoreSession);
    assert_eq!(restored.documents[0].encoding(), FileEncoding::Utf8);
    assert_eq!(restored.documents[1].encoding(), FileEncoding::Windows1252);
    assert_eq!(
        restored.documents[2].project_root(),
        c.parent().unwrap().parent().unwrap()
    );
}

#[test]
fn nested_project_and_own_dirty_guard_do_not_reload_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let a = project(&root, "parent", "latin1", b"<?php // \xe9");
    let b = project(
        &root.join("parent"),
        "child",
        "utf-8",
        "<?php // café".as_bytes(),
    );
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root));
    run(&mut app, Command::OpenFile(a.clone()));
    run(&mut app, Command::OpenFile(b.clone()));
    let child_version = app.documents[1].version;
    run(
        &mut app,
        Command::SetFolderEncoding {
            root: a.parent().unwrap().parent().unwrap().into(),
            encoding: FileEncoding::Windows1252,
        },
    );
    assert_eq!(app.documents[1].version, child_version);
    assert_eq!(app.documents[1].encoding(), FileEncoding::Utf8);
    let id = app.documents[0].id;
    let version = app.documents[0].version;
    run(
        &mut app,
        Command::Edit {
            id,
            expected_version: version,
            text: "dirty".into(),
        },
    );
    assert!(
        app.dispatch(Command::SetFolderEncoding {
            root: a.parent().unwrap().parent().unwrap().into(),
            encoding: FileEncoding::Utf8
        })
        .is_err()
    );
    assert_eq!(app.documents[0].text(), "dirty");
    assert_eq!(fs::read(a).unwrap(), b"<?php // \xe9");
}

#[test]
fn new_files_follow_filename_editorconfig_via_both_creation_commands() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let a = project(&root, "legacy", "latin1", b"<?php // \xe9");
    let parent = a.parent().unwrap().to_path_buf();
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root));
    run(&mut app, Command::CreateFile(parent.join("direct.php")));
    assert_eq!(
        app.active_document().unwrap().encoding(),
        FileEncoding::Latin1
    );
    run(
        &mut app,
        Command::ProjectFile(ProjectFileCommand::CreateFile {
            parent: parent.clone(),
            name: "explorer.php".into(),
        }),
    );
    let d = app.active_document().unwrap();
    assert_eq!(d.encoding(), FileEncoding::Latin1);
    assert_eq!(d.project_root(), parent.parent().unwrap());
    let id = d.id;
    let version = d.version;
    run(
        &mut app,
        Command::Edit {
            id,
            expected_version: version,
            text: "<?php // ação".into(),
        },
    );
    run(&mut app, Command::Save { id });
    assert_eq!(
        fs::read(parent.join("explorer.php")).unwrap(),
        b"<?php // a\xe7\xe3o"
    );
    run(
        &mut app,
        Command::ProjectFile(ProjectFileCommand::CreateFile {
            parent,
            name: "utf8.json".into(),
        }),
    );
    assert_eq!(
        app.active_document().unwrap().encoding(),
        FileEncoding::Utf8
    );
}

#[test]
fn editorconfig_sections_inheritance_root_and_unsupported_charset_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir_all(root.join("legacy/sub")).unwrap();
    fs::write(
        root.join(".editorconfig"),
        "root = true\n[*]\ncharset = utf-8\n[legacy/**.{php,inc}]\ncharset = latin1\n",
    )
    .unwrap();
    let path = root.join("legacy/sub/ação.php");
    fs::write(&path, b"<?php // \xe9").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root.clone()));
    run(&mut app, Command::OpenFile(path.clone()));
    assert_eq!(app.active_document().unwrap().text(), "<?php // é");
    fs::write(
        root.join("legacy/sub/.editorconfig"),
        "root = true\n[*.php]\ncharset = utf-16\n",
    )
    .unwrap();
    let unsupported = root.join("legacy/sub/b.php");
    fs::write(&unsupported, b"unchanged").unwrap();
    app.dispatch(Command::OpenFile(unsupported.clone()))
        .unwrap();
    settle(&mut app);
    assert!(app.last_error.as_ref().unwrap().contains("utf-16"));
    assert_eq!(app.documents.len(), 1);
    assert_eq!(fs::read(unsupported).unwrap(), b"unchanged");
}

#[test]
fn folder_override_without_manifest_is_an_independent_persistent_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    let folder = root.join("legacy");
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join("a.php");
    fs::write(&path, b"<?php // \xe9").unwrap();
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root));
    run(
        &mut app,
        Command::SetFolderEncoding {
            root: folder.clone(),
            encoding: FileEncoding::Latin1,
        },
    );
    run(&mut app, Command::OpenFile(path));
    assert_eq!(app.active_document().unwrap().project_root(), folder);
    assert_eq!(app.active_document().unwrap().text(), "<?php // é");
}

#[test]
fn moving_a_document_between_services_updates_owner_without_converting_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let a = project(&root, "latin", "latin1", b"<?php // \xe9");
    let b = project(&root, "utf", "utf-8", b"<?php // UTF8");
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root));
    run(&mut app, Command::OpenFile(a.clone()));
    let parent = b.parent().unwrap().parent().unwrap().to_path_buf();
    run(
        &mut app,
        Command::ProjectFile(ProjectFileCommand::Transfer {
            source: a,
            parent: parent.clone(),
            move_item: true,
        }),
    );
    let doc = app.active_document().unwrap();
    assert_eq!(doc.path, parent.join("a.php"));
    assert_eq!(doc.project_root(), parent);
    assert_eq!(doc.encoding(), FileEncoding::Latin1);
    assert_eq!(fs::read(&doc.path).unwrap(), b"<?php // \xe9");
}

#[test]
fn external_refresh_keeps_explicit_project_encoding_and_decodes_watched_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let a = project(&root, "legacy", "latin1", b"<?php // \xe9");
    let project = a.parent().unwrap().parent().unwrap().to_path_buf();
    let mut app = AppService::new(dir.path().join("session.json"));
    run(&mut app, Command::OpenWorkspace(root.clone()));
    run(&mut app, Command::OpenFile(a.clone()));
    run(
        &mut app,
        Command::SetProjectEncoding(FileEncoding::Windows1252),
    );
    // The explicit codec wins even if a subsequent unsupported declaration appears.
    fs::write(
        project.join(".editorconfig"),
        "root=true\n[*]\ncharset=utf-16\n",
    )
    .unwrap();
    fs::write(&a, b"<?php // \x80 externo").unwrap();
    fs::write(root.join("new.txt"), "structure").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while app.active_document().unwrap().text() != "<?php // € externo"
        || !app.entries.iter().any(|e| e.path == root.join("new.txt"))
    {
        app.poll();
        assert!(
            Instant::now() < end,
            "{} {:?}",
            app.active_document().unwrap().text(),
            app.last_error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.project_encoding(), FileEncoding::Windows1252);
    assert_eq!(app.active_document().unwrap().project_root(), project);
    assert_eq!(fs::read(a).unwrap(), b"<?php // \x80 externo");
}
