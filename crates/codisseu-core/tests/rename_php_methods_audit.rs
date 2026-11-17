//! Real Phpactor rename audit over independently authored PHP projects.
use codisseu_core::{AppService, Command, Coverage, Language, ProviderConfig, ProviderState};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if !app.is_busy() && !app.rename.is_pending() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "status={} error={:?} validation={:?} providers={:?}",
            app.status,
            app.rename.error,
            app.rename.validation,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn configs() -> Vec<ProviderConfig> {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("explicit provider configuration required"),
        )
        .unwrap(),
    )
    .unwrap();
    let php = configs
        .into_iter()
        .filter(|config| config.language == Language::Php)
        .collect::<Vec<_>>();
    assert_eq!(php.len(), 1);
    php
}
fn open(app: &mut AppService, path: &Path, dirty: bool) -> u64 {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    wait(app);
    let doc = app.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    if dirty {
        let updated = format!("{}// unsaved 😀 e\u{301}\n", doc.text());
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: updated,
        })
        .unwrap();
    }
    id
}
fn propose(app: &mut AppService, id: u64, needle: &str, prefix: usize, new: &str) {
    app.dispatch(Command::Activate { id }).unwrap();
    let doc = app.active_document().unwrap();
    let caret = doc.text()[..doc.text().find(needle).expect("origin") + prefix]
        .chars()
        .count()
        + 1;
    app.dispatch(Command::PrepareRename {
        id,
        expected_version: doc.version,
        character_index: caret,
    })
    .unwrap();
    wait(app);
    assert!(
        app.rename.is_prepared(),
        "prepare: {:?}; {}",
        app.rename.error,
        app.status
    );
    app.dispatch(Command::PreviewRename {
        new_name: new.into(),
    })
    .unwrap();
    wait(app);
    if app.rename.choosing_methods() {
        assert!(app.rename.can_confirm_methods());
        let token = app.rename.context_token().unwrap();
        app.dispatch(Command::ConfirmRenameMethods { token })
            .unwrap();
        wait(app);
    }
    assert!(
        app.rename.can_apply(),
        "preview: {:?}; {:?}; {}",
        app.rename.error,
        app.rename.validation,
        app.status
    );
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified
    );
}
fn disk(root: &Path, files: &BTreeMap<&str, String>) {
    for (name, text) in files {
        assert_eq!(
            fs::read_to_string(root.join(name)).unwrap(),
            *text,
            "{name}"
        );
    }
}

#[test]
#[ignore = "real PHP method rename; requires CODISSEU_PROVIDER_CONFIG"]
fn php_method_declaration_and_call_origins_preserve_homonyms_and_dirty_group_history() {
    for declaration_origin in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(
            root.join("composer.json"),
            r#"{"name":"codisseu/php-method-audit","autoload":{"psr-4":{"Audit\\":"src/"}}}"#,
        )
        .unwrap();
        let files = BTreeMap::from([
            ("src/Gateway.php", "<?php\nnamespace Audit;\nclass Gateway {\n public function convert(string $value): string { return 'convert:' . $value; }\n public function run(): string { return $this->convert('inside'); }\n}\n".to_owned()),
            ("src/Other.php", "<?php\nnamespace Audit;\nclass Other {\n public function convert(string $value): string { return 'other:' . $value; }\n public function run(): string { return $this->convert('independent'); }\n}\n".to_owned()),
            ("main.php", "<?php\nuse Audit\\Gateway;\nuse Audit\\Other;\nfunction consume(Gateway $gateway): string { return $gateway->convert('typed'); }\n$gateway = new Gateway();\n/* 😀 e\u{301} */ echo $gateway->convert('outside');\n$other = new Other();\necho $other->convert('unchanged');\n// convert stays in comments\necho 'convert';\n".to_owned()),
            ("closed.php", "<?php\nuse Audit\\Gateway;\n$copy = new Gateway();\necho $copy->convert('closed');\n".to_owned()),
        ]);
        for (name, text) in &files {
            fs::write(root.join(name), text).unwrap();
        }
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        wait(&mut app);
        let declaration = open(&mut app, &root.join("src/Gateway.php"), true);
        let caller = open(&mut app, &root.join("main.php"), true);
        app.dispatch(Command::ConfigureProviders(configs()))
            .unwrap();
        app.dispatch(Command::StartProviders).unwrap();
        wait(&mut app);
        assert!(
            app.language
                .statuses
                .iter()
                .all(|status| status.state == ProviderState::Ready)
        );
        let origin = if declaration_origin {
            declaration
        } else {
            caller
        };
        propose(
            &mut app,
            origin,
            if declaration_origin {
                "function convert("
            } else {
                "echo $gateway->convert("
            },
            if declaration_origin {
                "function ".len()
            } else {
                "echo $gateway->".len()
            },
            "transform",
        );
        let preview = app.rename.files.clone();
        assert_eq!(
            preview.len(),
            3,
            "{:?}",
            preview.iter().map(|file| &file.path).collect::<Vec<_>>()
        );
        assert!(!preview.iter().any(|file| file.path.ends_with("Other.php")));
        assert_eq!(
            preview.iter().map(|file| file.edits.len()).sum::<usize>(),
            5
        );
        let before = preview
            .iter()
            .map(|file| (file.path.clone(), file.before.clone()))
            .collect::<BTreeMap<_, _>>();
        let after = preview
            .iter()
            .map(|file| (file.path.clone(), file.after.clone()))
            .collect::<BTreeMap<_, _>>();
        assert!(after[&root.join("main.php")].contains("$other->convert('unchanged')"));
        assert!(after[&root.join("main.php")].contains("// convert stays in comments"));
        assert!(after[&root.join("main.php")].contains("echo 'convert'"));
        disk(&root, &files);
        app.dispatch(Command::ApplyRename).unwrap();
        wait(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        for (path, expected) in &after {
            let doc = app.documents.iter().find(|doc| &doc.path == path).unwrap();
            assert_eq!(doc.text(), expected.as_ref());
            assert!(doc.is_dirty());
        }
        disk(&root, &files);
        app.dispatch(Command::Activate { id: origin }).unwrap();
        app.dispatch(Command::Undo { id: origin }).unwrap();
        wait(&mut app);
        for (path, expected) in &before {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|doc| &doc.path == path)
                    .unwrap()
                    .text(),
                expected.as_ref()
            );
        }
        app.dispatch(Command::Redo { id: origin }).unwrap();
        wait(&mut app);
        for (path, expected) in &after {
            assert_eq!(
                app.documents
                    .iter()
                    .find(|doc| &doc.path == path)
                    .unwrap()
                    .text(),
                expected.as_ref()
            );
        }
        let save_ids = app
            .documents
            .iter()
            .filter(|doc| after.contains_key(&doc.path))
            .map(|doc| doc.id)
            .collect::<Vec<_>>();
        for id in save_ids {
            app.dispatch(Command::Save { id }).unwrap();
            wait(&mut app);
        }
        for (path, expected) in &after {
            assert_eq!(fs::read_to_string(path).unwrap(), expected.as_ref());
        }
        assert_eq!(
            fs::read_to_string(root.join("src/Other.php")).unwrap(),
            files["src/Other.php"]
        );
        let output = std::process::Command::new("php")
            .args([
                "-d",
                "xdebug.mode=off",
                "-r",
                "require $argv[1]; require $argv[2]; require $argv[3]; require $argv[4];",
            ])
            .args([
                root.join("src/Gateway.php"),
                root.join("src/Other.php"),
                root.join("main.php"),
                root.join("closed.php"),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "convert:outsideother:unchangedconvertconvert:closed"
        );
        app.dispatch(Command::StopProviders).unwrap();
        wait(&mut app);
    }
}

#[test]
#[ignore = "real PHP static/inheritance/interface/property methods; CODISSEU_PROVIDER_CONFIG"]
fn php_method_static_inheritance_interface_and_typed_property_keep_complete_families() {
    for case in ["static", "inheritance", "interface", "property"] {
        for declaration_origin in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            fs::create_dir(root.join("src")).unwrap();
            fs::write(root.join("composer.json"), r#"{"name":"codisseu/php-method-family-audit","autoload":{"psr-4":{"Audit\\":"src/"}}}"#).unwrap();
            let mut files = BTreeMap::from([
                ("src/Gateway.php", "<?php\nnamespace Audit;\nclass Gateway { public function convert(string $value): string { return 'convert:' . $value; } public function run(): string { return $this->convert('inside'); } }\n".to_owned()),
                ("src/Other.php", "<?php\nnamespace Audit;\nclass Other { public function convert(string $value): string { return 'other:' . $value; } }\n".to_owned()),
                ("main.php", "<?php\nuse Audit\\Gateway;\nuse Audit\\Other;\nfunction consume(Gateway $gateway): string { return $gateway->convert('typed'); }\n$gateway = new Gateway();\n/* 😀 */ echo $gateway->convert('outside');\n$other = new Other(); echo $other->convert('independent');\n".to_owned()),
                ("closed.php", "<?php\nuse Audit\\Gateway as Client;\n$copy = new Client();\necho $copy->convert('closed');\n".to_owned()),
            ]);
            let (expected_files, expected_edits) = match case {
                "static" => {
                    files.insert("src/Gateway.php", "<?php\nnamespace Audit;\nclass Gateway { public static function convert(string $value): string { return 'convert:' . $value; } public static function run(): string { return self::convert('inside'); } }\n".into());
                    files.insert("src/Other.php", "<?php\nnamespace Audit;\nclass Other { public static function convert(string $value): string { return 'other:' . $value; } }\n".into());
                    files.insert("main.php", "<?php\nuse Audit\\Gateway;\nuse Audit\\Other;\n/* 😀 */ echo Gateway::convert('outside');\necho Other::convert('independent');\n".into());
                    files.insert(
                        "closed.php",
                        "<?php\nuse Audit\\Gateway as Client;\necho Client::convert('closed');\n"
                            .into(),
                    );
                    (3, 4)
                }
                "inheritance" => {
                    files.insert("src/BaseGateway.php", "<?php\nnamespace Audit;\nclass BaseGateway { public function convert(string $value): string { return 'base:' . $value; } }\n".into());
                    files.insert("src/Gateway.php", "<?php\nnamespace Audit;\nclass Gateway extends BaseGateway { public function convert(string $value): string { return parent::convert($value); } }\n".into());
                    files.insert(
                        "main.php",
                        files["main.php"]
                            .replace(
                                "use Audit\\Other;",
                                "use Audit\\Other;\nuse Audit\\BaseGateway;",
                            )
                            .replace("function consume(Gateway", "function consume(BaseGateway"),
                    );
                    (4, 6)
                }
                "interface" => {
                    files.insert("src/Port.php", "<?php\nnamespace Audit;\ninterface Port { public function convert(string $value): string; }\n".into());
                    files.insert("src/Gateway.php", "<?php\nnamespace Audit;\nclass Gateway implements Port { public function convert(string $value): string { return 'convert:' . $value; } }\n".into());
                    files.insert(
                        "main.php",
                        files["main.php"]
                            .replace("use Audit\\Other;", "use Audit\\Other;\nuse Audit\\Port;")
                            .replace("function consume(Gateway", "function consume(Port"),
                    );
                    (4, 5)
                }
                "property" => {
                    files.insert("src/Consumer.php", "<?php\nnamespace Audit;\nclass Consumer { public Gateway $gateway; public function run(): string { return $this->gateway->convert('property'); } }\n".into());
                    (4, 6)
                }
                _ => unreachable!(),
            };
            for (name, text) in &files {
                fs::write(root.join(name), text).unwrap();
            }
            let mut app = AppService::new(root.join("session.json"));
            app.dispatch(Command::SetAutoSave {
                enabled: false,
                delay_ms: 1000,
            })
            .unwrap();
            app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
            wait(&mut app);
            let declaration = open(&mut app, &root.join("src/Gateway.php"), true);
            let caller = open(&mut app, &root.join("main.php"), true);
            app.dispatch(Command::ConfigureProviders(configs()))
                .unwrap();
            app.dispatch(Command::StartProviders).unwrap();
            wait(&mut app);
            let origin = if declaration_origin {
                declaration
            } else {
                caller
            };
            let (needle, prefix) = if declaration_origin {
                ("function convert(", "function ".len())
            } else if case == "static" {
                ("Gateway::convert(", "Gateway::".len())
            } else {
                ("->convert(", 2)
            };
            eprintln!("PHP method audit case={case}, declaration_origin={declaration_origin}");
            propose(&mut app, origin, needle, prefix, "transform");
            let preview = app.rename.files.clone();
            assert_eq!(
                preview.len(),
                expected_files,
                "{case}: {:?}",
                preview.iter().map(|file| &file.path).collect::<Vec<_>>()
            );
            assert_eq!(
                preview.iter().map(|file| file.edits.len()).sum::<usize>(),
                expected_edits,
                "{case}"
            );
            assert!(
                !preview.iter().any(|file| file.path.ends_with("Other.php")),
                "{case}"
            );
            let before = preview
                .iter()
                .map(|file| (file.path.clone(), file.before.clone()))
                .collect::<BTreeMap<_, _>>();
            let after = preview
                .iter()
                .map(|file| (file.path.clone(), file.after.clone()))
                .collect::<BTreeMap<_, _>>();
            assert!(after[&root.join("main.php")].contains(if case == "static" {
                "Other::convert('independent')"
            } else {
                "$other->convert('independent')"
            }));
            disk(&root, &files);
            app.dispatch(Command::ApplyRename).unwrap();
            wait(&mut app);
            assert!(app.rename.error.is_none(), "{case}: {:?}", app.rename.error);
            for (path, expected) in &after {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|doc| &doc.path == path)
                        .unwrap()
                        .text(),
                    expected.as_ref(),
                    "{case}: {}",
                    path.display()
                );
            }
            app.dispatch(Command::Undo { id: origin }).unwrap();
            wait(&mut app);
            for (path, expected) in &before {
                assert_eq!(
                    app.documents
                        .iter()
                        .find(|doc| &doc.path == path)
                        .unwrap()
                        .text(),
                    expected.as_ref(),
                    "{case}"
                );
            }
            app.dispatch(Command::Redo { id: origin }).unwrap();
            wait(&mut app);
            let ids = app
                .documents
                .iter()
                .filter(|doc| after.contains_key(&doc.path))
                .map(|doc| doc.id)
                .collect::<Vec<_>>();
            for id in ids {
                app.dispatch(Command::Save { id }).unwrap();
                wait(&mut app);
            }
            for (path, expected) in &after {
                assert_eq!(
                    fs::read_to_string(path).unwrap(),
                    expected.as_ref(),
                    "{case}"
                );
            }
            assert_eq!(
                fs::read_to_string(root.join("src/Other.php")).unwrap(),
                files["src/Other.php"]
            );
            app.dispatch(Command::StopProviders).unwrap();
            wait(&mut app);
        }
    }
}
