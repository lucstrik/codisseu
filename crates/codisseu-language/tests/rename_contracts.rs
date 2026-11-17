//! Real-provider contracts, including cases where a proposal is not semantically complete.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, utf16_byte_offset,
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(service: &mut LanguageService, ready: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if ready(service) {
            return;
        }
        assert!(service.last_error.is_none(), "{:?}", service.last_error);
        assert!(
            Instant::now() < deadline,
            "{:?}; {:?}",
            service.statuses,
            service.rename_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn wire(path: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn closed_snapshot_lease_closes_after_success_error_invalid_version_and_late_cancel() {
    for mode in ["safe", "error", "version", "hold"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("closed.ts");
        let log = root.join("wire.jsonl");
        let source = "class Weather { rainfall(): string { return 'rain'; } }\n";
        fs::write(&path, source).unwrap();
        let mut service = LanguageService::new();
        service
            .dispatch(LanguageCommand::Start {
                workspace: root.clone(),
                configs: vec![ProviderConfig {
                    language: Language::TypeScript,
                    program: "python3".into(),
                    args: vec![
                        format!(
                            "{}/tests/fixtures/rename_contracts_server.py",
                            env!("CARGO_MANIFEST_DIR")
                        ),
                        mode.into(),
                        log.display().to_string(),
                    ],
                }],
            })
            .unwrap();
        wait(&mut service, |s| {
            s.statuses.iter().any(|s| s.state == ProviderState::Ready) && !s.is_busy()
        });
        service
            .dispatch(LanguageCommand::RenameClosed {
                path: path.clone(),
                text: source.into(),
                line: 0,
                character: 18,
                new_name: "precipitation".into(),
            })
            .unwrap();
        if mode == "hold" {
            wait(&mut service, |_| {
                wire(&log)
                    .iter()
                    .any(|m| m["method"] == "textDocument/rename")
            });
            service.dispatch(LanguageCommand::CancelRename).unwrap();
        }
        // No diagnostic or public synchronized document is created by the lease.
        wait(&mut service, |s| {
            !s.rename_pending()
                && !s.is_busy()
                && wire(&log)
                    .iter()
                    .any(|m| m["method"] == "textDocument/didClose")
        });
        assert!(service.diagnostics.is_empty());
        assert!(
            service
                .dispatch(LanguageCommand::Rename {
                    path: path.clone(),
                    version: 1,
                    line: 0,
                    character: 18,
                    new_name: "precipitation".into()
                })
                .is_err()
        );
        let records = wire(&log);
        assert_eq!(
            records
                .iter()
                .filter(|m| m["method"] == "textDocument/didOpen")
                .count(),
            1
        );
        assert_eq!(
            records
                .iter()
                .filter(|m| m["method"] == "textDocument/didClose")
                .count(),
            1
        );
        match mode {
            "safe" => assert_eq!(
                service.rename.as_ref().unwrap().transient_document_version,
                Some(1)
            ),
            "hold" => assert!(service.rename.is_none() && service.rename_error.is_none()),
            _ => assert!(service.rename.is_none() && service.rename_error.is_some()),
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        // A real editor document may then open on the same path without inheriting
        // the lease or discarding the caller's unsaved bytes/version.
        service
            .dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 2,
                text: format!("{source}// dirty source\n"),
            })
            .unwrap();
        // Sync ACK confirms queueing, not that the provider consumed its pipe.
        // Observe the second didOpen before inspecting the provider's wire log.
        wait(&mut service, |s| {
            !s.is_busy()
                && wire(&log)
                    .iter()
                    .filter(|m| m["method"] == "textDocument/didOpen")
                    .count()
                    == 2
        });
        assert!(
            service
                .dispatch(LanguageCommand::RenameClosed {
                    path: path.clone(),
                    text: source.into(),
                    line: 0,
                    character: 18,
                    new_name: "precipitation".into()
                })
                .is_err()
        );
        let records = wire(&log);
        let opens: Vec<_> = records
            .iter()
            .filter(|m| m["method"] == "textDocument/didOpen")
            .collect();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[1]["params"]["textDocument"]["version"], 2);
        assert_eq!(
            opens[1]["params"]["textDocument"]["text"],
            format!("{source}// dirty source\n")
        );
        service.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut service, |s| !s.is_busy());
    }
}

fn fixture(root: &Path) -> BTreeMap<String, String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-contracts");
    let mut files = BTreeMap::new();
    for entry in fs::read_dir(&source).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fs::create_dir(root.join(path.file_name().unwrap())).unwrap();
            for entry in fs::read_dir(&path).unwrap() {
                let path = entry.unwrap().path();
                let name = path
                    .strip_prefix(&source)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                files.insert(name, fs::read_to_string(path).unwrap());
            }
        } else {
            files.insert(
                path.file_name().unwrap().to_str().unwrap().to_owned(),
                fs::read_to_string(path).unwrap(),
            );
        }
    }
    for (name, text) in &files {
        fs::write(root.join(name), text).unwrap();
    }
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .current_dir(root)
        .output()
        .expect("Composer required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    files
}

#[test]
#[ignore = "real PHP/TS interface and trait proposals; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn provider_contract_proposals_preserve_unrelated_methods_and_utf16_names() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let files = fixture(&root);
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs,
        })
        .unwrap();
    for (name, text) in &files {
        if name.ends_with(".php") || name.ends_with(".ts") {
            service
                .dispatch(LanguageCommand::Sync {
                    path: root.join(name),
                    version: 1,
                    text: text.clone(),
                })
                .unwrap();
        }
    }
    wait(&mut service, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for origin in [
        "main.php",
        "src/Weather.php",
        "src/WeatherPort.php",
        "trait.php",
        "src/RainProvider.php",
        "main.ts",
        "src/weather.ts",
        "src/weatherPort.ts",
    ] {
        let source = &files[origin];
        let prefix = &source[..source.find("rainfall").unwrap() + 2];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        for new_name in ["precipitation", "precipitação"] {
            service
                .dispatch(LanguageCommand::Rename {
                    path: root.join(origin),
                    version: 1,
                    line,
                    character,
                    new_name: new_name.into(),
                })
                .unwrap();
            wait(&mut service, |s| !s.rename_pending());
            eprintln!(
                "contract-origin={origin} new_name={new_name} result={} error={:?}",
                serde_json::to_string(&service.rename).unwrap(),
                service.rename_error
            );
            assert!(
                service.rename_error.is_none(),
                "{origin}: {:?}",
                service.rename_error
            );
            let proposal = service.rename.as_ref().expect("real provider proposal");
            let expected_count = if origin == "trait.php" || origin == "src/RainProvider.php" {
                2
            } else if origin == "main.php" || origin == "src/Weather.php" {
                3
            } else {
                4
            };
            assert_eq!(
                proposal
                    .changes
                    .iter()
                    .map(|f| f.edits.len())
                    .sum::<usize>(),
                expected_count,
                "Raw Phpactor concrete-origin proposals omit the interface-typed call; the core must expand them"
            );
            for change in &proposal.changes {
                let relative = change.path.strip_prefix(&root).unwrap().to_str().unwrap();
                assert!(
                    !relative.starts_with("other")
                        && !relative.contains("OtherWeather")
                        && !relative.contains("otherWeather"),
                    "unrelated class edited: {relative}"
                );
                let before = &files[relative];
                for edit in &change.edits {
                    let a =
                        utf16_byte_offset(before, edit.start_line, edit.start_character).unwrap();
                    let b = utf16_byte_offset(before, edit.end_line, edit.end_character).unwrap();
                    assert_eq!(&before[a..b], "rainfall");
                    assert_eq!(edit.new_text, new_name);
                }
            }
        }
    }
    for origin in ["src/WeatherPort.php", "src/weatherPort.ts"] {
        let source = &files[origin];
        let prefix = &source[..source.find("rainfall").unwrap() + 2];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        service
            .dispatch(LanguageCommand::Close {
                path: root.join(origin),
            })
            .unwrap();
        wait(&mut service, |s| !s.is_busy());
        service
            .dispatch(LanguageCommand::RenameClosed {
                path: root.join(origin),
                text: source.clone(),
                line,
                character,
                new_name: "precipitation".into(),
            })
            .unwrap();
        wait(&mut service, |s| !s.rename_pending());
        assert!(service.rename_error.is_none(), "{:?}", service.rename_error);
        let proposal = service.rename.as_ref().unwrap();
        assert_eq!(
            proposal.version, 0,
            "request is an exact closed snapshot, not an editor revision"
        );
        assert_eq!(
            proposal.transient_document_version,
            Some(1),
            "actual didOpen lease version"
        );
        assert_eq!(
            proposal
                .changes
                .iter()
                .map(|f| f.edits.len())
                .sum::<usize>(),
            4
        );
        assert!(
            service
                .diagnostics
                .iter()
                .all(|d| d.path != root.join(origin))
        );
        assert!(
            service
                .dispatch(LanguageCommand::Rename {
                    path: root.join(origin),
                    version: 1,
                    line,
                    character,
                    new_name: "precipitation".into()
                })
                .is_err(),
            "lease must not become public document state"
        );
        service
            .dispatch(LanguageCommand::Sync {
                path: root.join(origin),
                version: 2,
                text: format!("{source}// dirty real buffer\n"),
            })
            .unwrap();
        assert!(
            service
                .dispatch(LanguageCommand::RenameClosed {
                    path: root.join(origin),
                    text: source.clone(),
                    line,
                    character,
                    new_name: "precipitation".into()
                })
                .is_err(),
            "temporary snapshots cannot replace unsaved editor documents"
        );
        wait(&mut service, |s| !s.is_busy());
    }
    for (name, text) in &files {
        assert_eq!(
            fs::read_to_string(root.join(name)).unwrap(),
            *text,
            "queries never save"
        );
    }
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |s| !s.is_busy());
}

#[test]
#[ignore = "real TS interface with two implementations; CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_typescript_interface_family_proposals_are_characterized_from_each_origin() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config")).unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let files = fixture(&root);
    let mut config: serde_json::Value = serde_json::from_str(&files["tsconfig.json"]).unwrap();
    config["include"] = serde_json::json!(["*.ts", "src/*.ts"]);
    let excluded_config = serde_json::to_string(&config).unwrap();
    fs::write(root.join("tsconfig.json"), &excluded_config).unwrap();
    let mut files = files;
    files.insert("tsconfig.json".into(), excluded_config);
    let mut service = LanguageService::new();
    service
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs,
        })
        .unwrap();
    let contract = "multiple/forecast.ts";
    service
        .dispatch(LanguageCommand::Sync {
            path: root.join(contract),
            version: 1,
            text: files[contract].clone(),
        })
        .unwrap();
    wait(&mut service, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|s| s.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for origin in [
        contract,
        "multiple/weather.ts",
        "multiple/wetWeather.ts",
        "multiple/interface-main.ts",
    ] {
        let source = &files[origin];
        let prefix = &source[..source.find("rainfall").unwrap() + 2];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let character = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        let command = if origin == contract {
            LanguageCommand::Rename {
                path: root.join(origin),
                version: 1,
                line,
                character,
                new_name: "precipitation".into(),
            }
        } else {
            LanguageCommand::RenameClosed {
                path: root.join(origin),
                text: source.clone(),
                line,
                character,
                new_name: "precipitation".into(),
            }
        };
        service.dispatch(command).unwrap();
        wait(&mut service, |s| !s.rename_pending());
        assert!(service.rename_error.is_none(), "{:?}", service.rename_error);
        let proposal = service.rename.as_ref().unwrap();
        eprintln!(
            "family-origin={origin} proposal={}",
            serde_json::to_string(proposal).unwrap()
        );
        assert_eq!(
            proposal
                .changes
                .iter()
                .map(|f| f.edits.len())
                .sum::<usize>(),
            match origin {
                "multiple/forecast.ts" => 1,
                "multiple/interface-main.ts" => 4,
                _ => 2,
            },
            "Excluded project files are not a complete semantic family merely because declaration queries succeed"
        );
        for file in &proposal.changes {
            let name = file.path.strip_prefix(&root).unwrap().to_str().unwrap();
            assert!(name.starts_with("multiple/"));
            let text = &files[name];
            for edit in &file.edits {
                let a = utf16_byte_offset(text, edit.start_line, edit.start_character).unwrap();
                let b = utf16_byte_offset(text, edit.end_line, edit.end_character).unwrap();
                assert_eq!(&text[a..b], "rainfall");
                assert_eq!(edit.new_text, "precipitation");
            }
        }
    }
    service.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut service, |s| !s.is_busy());
    for (name, text) in &files {
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), *text);
    }
}
