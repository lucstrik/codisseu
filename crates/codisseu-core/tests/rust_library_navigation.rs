//! Explicit end-to-end Rust standard-library navigation; all project sources are synthetic.
use codisseu_core::{
    AppService, Command, EditKind, ProviderConfig, ProviderState, Selection, TextDelta,
};
use codisseu_language::Language;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, predicate: impl Fn(&AppService) -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < until,
            "{} {:?} {:?}",
            app.status,
            app.last_error,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "explicit real rust-analyzer/rust-src standard library navigation"]
fn vec_definition_opens_read_only_blocks_mutations_and_navigates_back() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_library_navigation\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    let source = root.join("src/lib.rs");
    let original = "pub fn make() -> Vec<i32> { Vec::new() }\n";
    fs::write(&source, original).unwrap();
    let session_path = root.join("profile/session.json");
    let mut app = AppService::new(session_path.clone());
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::OpenFile(source.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let origin = app.active_document().unwrap().id;
    let caret = original.find("new").unwrap() + 1;
    app.dispatch(Command::ConfigureProviders(vec![ProviderConfig {
        language: Language::Rust,
        program: std::env::var_os("CODISSEU_RUST_ANALYZER")
            .unwrap_or_else(|| "rust-analyzer".into())
            .into(),
        args: vec![],
    }]))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        !a.language.statuses.is_empty()
            && a.language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && !a.is_busy()
    });
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        app.dispatch(Command::Definition {
            id: origin,
            expected_version: 1,
            character_index: caret,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy() && !a.navigation.is_pending());
        if app.active != Some(origin) {
            break;
        }
        assert!(
            Instant::now() < until,
            "Vec::new definition failed: {} {:?}",
            app.status,
            app.navigation.error
        );
        thread::sleep(Duration::from_millis(100));
    }
    let library = app.active_document().unwrap();
    let id = library.id;
    let version = library.version;
    let path = library.path.clone();
    assert!(library.is_read_only());
    assert!(!path.starts_with(&root));
    assert!(codisseu_language::rust_library_root(&path).is_some());
    assert!(library.text().contains("pub const fn new()"));
    let text = library.text().to_string();
    let disk = fs::read(&path).unwrap();
    let semantic_root = app
        .active_language_project()
        .map(std::path::Path::to_path_buf);
    assert_eq!(semantic_root.as_ref(), Some(&root));
    assert!(
        !app.language.analyzed_versions.contains_key(&path),
        "library is not synced as a project buffer"
    );
    for command in [
        Command::Edit {
            id,
            expected_version: version,
            text: "changed".into(),
        },
        Command::EditDelta {
            id,
            expected_version: version,
            delta: TextDelta {
                range: 0..0,
                insert: "changed".into(),
            },
            kind: EditKind::Atomic,
        },
        Command::EditTransaction {
            id,
            expected_version: version,
            delta: TextDelta {
                range: 0..0,
                insert: "changed".into(),
            },
            kind: EditKind::Atomic,
            before: library.selection(),
            after: Selection::caret(7),
        },
        Command::EditBatchTransaction {
            id,
            expected_version: version,
            deltas: vec![TextDelta {
                range: 0..0,
                insert: "changed".into(),
            }],
            before: library.selection(),
            after: Selection::caret(7),
        },
        Command::FormatDocument {
            id,
            expected_version: version,
            range: None,
        },
        Command::Save { id },
        Command::Undo { id },
        Command::Redo { id },
        Command::ReplaceMatches {
            id,
            expected_version: version,
            query: "new".into(),
            replacement: "changed".into(),
            selection: None,
        },
        Command::PrepareRename {
            id,
            expected_version: version,
            character_index: 0,
        },
        Command::PrepareFileRename {
            id,
            expected_version: version,
        },
        Command::Completion {
            id,
            expected_version: version,
            character_index: 0,
        },
        Command::AcceptCompletion {
            index: 0,
            replace: false,
        },
        Command::RestoreLocalRevision {
            id,
            expected_version: version,
            revision: 1,
        },
    ] {
        assert!(
            app.dispatch(command)
                .unwrap_err()
                .contains("somente leitura")
        );
        assert_eq!(app.active_document().unwrap().text(), text);
        assert_eq!(app.active_document().unwrap().version, version);
        assert!(!app.active_document().unwrap().is_dirty());
    }
    app.dispatch(Command::SetSelection {
        id,
        expected_version: version,
        selection: Selection { anchor: 0, head: 3 },
    })
    .unwrap();
    app.dispatch(Command::EndEditGroup { id }).unwrap();
    app.dispatch(Command::Find {
        id,
        expected_version: version,
        query: "new".into(),
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::NavigateBack {
        id,
        expected_version: version,
        character_index: 3,
    })
    .unwrap();
    assert_eq!(app.active, Some(origin));
    assert_eq!(
        app.navigation.selection.as_ref().unwrap().range,
        caret..caret
    );
    assert_eq!(app.active_document().unwrap().text(), original);
    app.dispatch(Command::Hover {
        id: origin,
        expected_version: 1,
        character_index: original.find("make").unwrap() + 1,
    })
    .unwrap();
    wait(&mut app, |a| !a.language.hover_pending());
    assert!(
        app.language
            .hover
            .as_ref()
            .is_some_and(|h| h.text.contains("make")),
        "project semantics remain available with a library tab open: {:?}",
        app.language.hover_error
    );
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::PersistSession).unwrap();
    wait(&mut app, |a| !a.is_busy());
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_path).unwrap()).unwrap();
    assert_eq!(stored["tabs"], serde_json::json!([source]));
    assert!(
        !fs::read_to_string(&session_path)
            .unwrap()
            .contains(path.to_str().unwrap())
    );
    assert_eq!(fs::read(path).unwrap(), disk);
    assert_eq!(fs::read_to_string(source).unwrap(), original);
}
