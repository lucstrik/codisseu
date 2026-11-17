use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, f: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f(s) {
        s.poll();
        assert!(
            Instant::now() < end,
            "{:?} {:?}",
            s.last_error,
            s.definition_error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn setup(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(
        root.join("Child.php"),
        "<?php\nclass Child extends Base implements Port { function deliver(){ return 1; } }\n",
    )
    .unwrap();
    fs::write(
        root.join("Base.php"),
        "<?php\nclass Base { function deliver(){} }\n",
    )
    .unwrap();
    fs::write(
        root.join("Port.php"),
        "<?php\ninterface Port extends Contract {}\n",
    )
    .unwrap();
    fs::write(
        root.join("Contract.php"),
        "<?php\ninterface Contract { function deliver(); }\n",
    )
    .unwrap();
    fs::write(d.path().join("escape.php"), "<?php").unwrap();
    let log = d.path().join("wire.jsonl");
    let source = root.join("Child.php");
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root.clone(),
        configs: vec![ProviderConfig {
            language: Language::Php,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/php_super_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                root.display().to_string(),
                log.display().to_string(),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().any(|x| x.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: source.clone(),
        version: 7,
        text: fs::read_to_string(&source).unwrap(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    (d, s, source, log)
}
fn records(log: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect()
}
#[test]
fn nearest_methods_per_branch_lease_closed_interface_and_preserve_dirty_snapshot() {
    let (d, mut s, path, log) = setup("normal");
    let base = d.path().join("root/Base.php");
    let dirty = "<?php\nclass Base { /* 😀 unsaved */ function deliver(){} }\n";
    s.dispatch(LanguageCommand::Sync {
        path: base.clone(),
        version: 9,
        text: dirty.into(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    s.dispatch(LanguageCommand::SuperDefinition {
        path,
        version: 7,
        line: 1,
        character: 65,
    })
    .unwrap();
    wait(&mut s, |s| {
        s.definition.is_some() || s.definition_error.is_some()
    });
    assert!(s.definition_error.is_none(), "{:?}", s.definition_error);
    let result = s.definition.take().unwrap();
    assert_eq!(result.targets.len(), 2);
    let b = result.targets.iter().find(|t| t.path == base).unwrap();
    assert_eq!(b.start_character, 39);
    assert!(
        result
            .targets
            .iter()
            .any(|t| t.path.ends_with("Contract.php"))
    );
    let messages = records(&log);
    let opens = messages
        .iter()
        .filter(|m| {
            m["method"] == "textDocument/didOpen"
                && m["params"]["textDocument"]["uri"]
                    .as_str()
                    .unwrap()
                    .ends_with("Port.php")
        })
        .count();
    assert_eq!(opens, 1);
    assert!(messages.iter().any(|m| {
        m["method"] == "textDocument/didClose"
            && m["params"]["textDocument"]["uri"]
                .as_str()
                .unwrap()
                .ends_with("Port.php")
    }));
    assert!(!messages.iter().any(|m| {
        m["method"] == "textDocument/didClose"
            && m["params"]["textDocument"]["uri"]
                .as_str()
                .unwrap()
                .ends_with("Base.php")
    }));
    assert!(!fs::read_to_string(base).unwrap().contains("unsaved"));
}
#[test]
fn type_definition_negotiation_and_escape_fail_closed() {
    for mode in ["normal", "unsupported", "outside"] {
        let (_d, mut s, path, log) = setup(mode);
        let command = if mode == "outside" {
            LanguageCommand::SuperDefinition {
                path,
                version: 7,
                line: 1,
                character: 65,
            }
        } else {
            LanguageCommand::TypeDefinition {
                path,
                version: 7,
                line: 1,
                character: 65,
            }
        };
        s.dispatch(command).unwrap();
        wait(&mut s, |s| {
            s.definition.is_some() || s.definition_error.is_some()
        });
        if mode == "normal" {
            assert!(
                s.definition.as_ref().unwrap().targets[0]
                    .path
                    .ends_with("Port.php")
            );
        } else {
            assert!(s.definition_error.is_some());
            assert!(s.definition.is_none());
        }
        if mode == "unsupported" {
            assert!(
                !records(&log)
                    .iter()
                    .any(|m| m["method"] == "textDocument/typeDefinition")
            );
        }
        let messages = records(&log);
        let initialize = messages
            .iter()
            .find(|m| m["method"] == "initialize")
            .unwrap();
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["typeDefinition"]["linkSupport"],
            true
        );
        assert_eq!(
            initialize["params"]["capabilities"]["textDocument"]["typeDefinition"]["dynamicRegistration"],
            false
        );
    }
}
#[test]
fn supers_cancel_on_edit_and_request_wire_id_is_distinct() {
    let (_d, mut s, path, log) = setup("delay");
    s.dispatch(LanguageCommand::SuperDefinition {
        path: path.clone(),
        version: 7,
        line: 1,
        character: 65,
    })
    .unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        s.poll();
        if records(&log)
            .iter()
            .any(|m| m["method"] == "textDocument/definition")
        {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    s.dispatch(LanguageCommand::Sync {
        path,
        version: 8,
        text: "<?php class Changed {}".into(),
    })
    .unwrap();
    wait(&mut s, |s| !s.is_busy());
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if records(&log)
            .iter()
            .any(|m| m["method"] == "$/cancelRequest")
        {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(s.definition.is_none());
    let messages = records(&log);
    let request = messages
        .iter()
        .find(|m| m["method"] == "textDocument/definition")
        .unwrap();
    let cancel = messages
        .iter()
        .find(|m| m["method"] == "$/cancelRequest")
        .unwrap();
    assert_eq!(request["id"], cancel["params"]["id"]);
    assert!(request["id"].as_str().unwrap().starts_with("super-"));
}
