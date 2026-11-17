//! Full application service routing, distinct process roots and real provider
//! settings. Controlled transport and real integration are reported separately.
use codisseu_core::{AppService, Command, Language, ProviderConfig};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[track_caller]
fn until(app: &mut AppService, condition: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        app.poll();
        if condition(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?} {:?} analyzed={:?} docs={:?}",
            app.status,
            app.last_error,
            app.language.statuses,
            app.language.analyzed_versions,
            app.documents
                .iter()
                .map(|d| (&d.path, d.version))
                .collect::<Vec<_>>()
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn idle(app: &mut AppService) {
    until(app, |a| !a.is_busy());
}
fn project(workspace: &Path, name: &str, strict: bool) -> (PathBuf, PathBuf) {
    let root = workspace.join(name);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("package.json"), r#"{"private":true}"#).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        format!(r#"{{"compilerOptions":{{"strictNullChecks":{strict}}},"include":["*.ts"]}}"#),
    )
    .unwrap();
    let path = root.join("same.ts");
    fs::write(&path, "export const value: string = null;\n").unwrap();
    (root, path)
}
fn open(app: &mut AppService, path: &Path) {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    idle(app);
}
fn activate(app: &mut AppService, path: &Path) {
    let id = app.documents.iter().find(|d| d.path == path).unwrap().id;
    app.dispatch(Command::Activate { id }).unwrap();
    idle(app);
}
fn controlled(log: &Path) -> Vec<ProviderConfig> {
    vec![ProviderConfig {
        language: Language::TypeScript,
        program: "python3".into(),
        args: vec![
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/project_lsp.py")
                .to_string_lossy()
                .into(),
            log.to_string_lossy().into(),
        ],
    }]
}
fn app(root: &Path, workspace: &Path, configs: Vec<ProviderConfig>) -> AppService {
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(workspace.into()))
        .unwrap();
    idle(&mut app);
    app.dispatch(Command::ConfigureProviders(configs)).unwrap();
    app
}

#[test]
fn project_sessions_use_distinct_roots_and_restore_cache_without_restarting() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (a, a_file) = project(&workspace, "a", true);
    let (b, b_file) = project(&workspace, "b", false);
    let log = fixture.path().join("lsp.jsonl");
    let mut app = app(fixture.path(), &workspace, controlled(&log));
    open(&mut app, &a_file);
    app.dispatch(Command::StartProviders).unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    assert_eq!(app.active_language_project(), Some(a.as_path()));
    open(&mut app, &b_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    assert_eq!(app.active_language_project(), Some(b.as_path()));
    assert!(!app.language.analyzed_versions.contains_key(&a_file));
    let id = app.documents.iter().find(|d| d.path == a_file).unwrap().id;
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "export const value: string = 'dirty 🌧';\n".into(),
    })
    .unwrap();
    activate(&mut app, &a_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.get(&a_file) == Some(&2)
    });
    assert_eq!(app.active_language_project(), Some(a.as_path()));
    assert!(!app.language.analyzed_versions.contains_key(&b_file));
    assert_eq!(
        fs::read_to_string(&a_file).unwrap(),
        "export const value: string = null;\n"
    );
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    let records: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for project in [&a, &b] {
        let calls: Vec<_> = records
            .iter()
            .filter(|r| r["cwd"].as_str() == project.to_str())
            .collect();
        assert_eq!(
            calls
                .iter()
                .filter(|r| r["message"]["method"] == "initialize")
                .count(),
            1,
            "one process per project; switch restores cache"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|r| r["message"]["method"] == "shutdown")
                .count(),
            1,
            "Stop reaches inactive process too"
        );
        for record in calls {
            if let Some(uri) = record["message"]["params"]["textDocument"]["uri"].as_str() {
                assert!(uri.contains(&format!(
                    "/{}/",
                    project.file_name().unwrap().to_string_lossy()
                )));
            }
        }
    }
}

#[test]
fn per_project_provider_configuration_persists_outside_sources_and_preserves_global_fallback() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (a, a_file) = project(&workspace, "a", true);
    let (b, b_file) = project(&workspace, "b", false);
    let default = controlled(&fixture.path().join("global.log"));
    let custom = controlled(&fixture.path().join("project.log"));
    let profile = fixture.path().join("policy.json");
    let mut app = app(fixture.path(), &workspace, default.clone());
    open(&mut app, &a_file);
    open(&mut app, &b_file);
    app.dispatch(Command::EnableProviderAutostart {
        profile: profile.clone(),
        fallback_config: None,
    })
    .unwrap();
    idle(&mut app);
    app.configure_project_providers(a.clone(), custom.clone())
        .unwrap();
    idle(&mut app);
    assert_eq!(app.project_provider_configs(&a), custom);
    assert_eq!(app.project_provider_configs(&b), default);
    assert!(!a.join(".codisseu").exists());
    drop(app);
    let mut restored = self::app(fixture.path(), &workspace, default.clone());
    restored
        .dispatch(Command::EnableProviderAutostart {
            profile,
            fallback_config: None,
        })
        .unwrap();
    idle(&mut restored);
    assert_eq!(restored.project_provider_configs(&a), custom);
    assert_eq!(restored.project_provider_configs(&b), default);
    assert!(
        restored.language.statuses.is_empty(),
        "project override grants no parent workspace trust"
    );
}

#[test]
#[ignore = "real TypeScript project roots/config isolation; requires CODISSEU_PROVIDER_CONFIG"]
fn real_typescript_projects_keep_distinct_tsconfig_and_dirty_buffers() {
    let config =
        std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("CODISSEU_PROVIDER_CONFIG required");
    let configs: Vec<ProviderConfig> = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let configs = configs
        .into_iter()
        .filter(|p| p.language == Language::TypeScript)
        .collect();
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (a, a_file) = project(&workspace, "strict-service", true);
    let (b, b_file) = project(&workspace, "legacy-service", false);
    fs::write(
        b.join(".editorconfig"),
        "root=true\n[*]\ncharset=windows-1252\n",
    )
    .unwrap();
    let b_bytes = b"import { LegacyThing } from './helper';\nexport const value: string = null; const item = new LegacyThing(); // pre\xe7o \x80 \x93cliente\x94\n";
    let helper = b.join("helper.ts");
    let helper_bytes = b"/* pre\xe7o \x80 \x93cliente\x94 */ export class LegacyThing {}\n";
    fs::write(&b_file, b_bytes).unwrap();
    fs::write(&helper, helper_bytes).unwrap();
    let mut app = app(fixture.path(), &workspace, configs);
    open(&mut app, &a_file);
    app.dispatch(Command::StartProviders).unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    assert!(
        app.language
            .diagnostics
            .iter()
            .any(|d| d.path == a_file && d.severity == 1)
    );
    open(&mut app, &b_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    assert_eq!(app.active_language_project(), Some(b.as_path()));
    assert!(
        !app.language.diagnostics.iter().any(|d| d.severity == 1),
        "legacy tsconfig disables strictNullChecks"
    );
    let doc = app.active_document().unwrap();
    assert_eq!(doc.encoding(), codisseu_core::FileEncoding::Windows1252);
    assert!(doc.text().contains("preço € “cliente”"));
    let character_index = doc.text()[..doc.text().rfind("LegacyThing").unwrap()]
        .chars()
        .count();
    app.dispatch(Command::Definition {
        id: doc.id,
        expected_version: doc.version,
        character_index,
    })
    .unwrap();
    until(&mut app, |s| {
        s.active_document().is_some_and(|d| d.path == helper) || s.navigation.error.is_some()
    });
    assert!(app.navigation.error.is_none(), "{:?}", app.navigation.error);
    let target = app.active_document().unwrap();
    assert_eq!(target.encoding(), codisseu_core::FileEncoding::Windows1252);
    assert!(target.text().contains("preço € “cliente”"));
    let selection = app.navigation.selection.as_ref().unwrap();
    assert_eq!(
        target
            .text()
            .chars()
            .skip(selection.range.start)
            .take(selection.range.len())
            .collect::<String>(),
        "LegacyThing"
    );
    assert_eq!(fs::read(&b_file).unwrap(), b_bytes);
    assert_eq!(fs::read(&helper).unwrap(), helper_bytes);
    activate(&mut app, &a_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    assert_eq!(app.active_language_project(), Some(a.as_path()));
    let id = app.documents.iter().find(|d| d.path == a_file).unwrap().id;
    app.dispatch(Command::Edit {
        id,
        expected_version: 1,
        text: "export const value: string = 'não salvo 🌧';\n".into(),
    })
    .unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.get(&a_file) == Some(&2)
    });
    assert!(app.language.diagnostics.is_empty());
    activate(&mut app, &b_file);
    activate(&mut app, &a_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.get(&a_file) == Some(&2)
    });
    assert!(app.language.diagnostics.is_empty());
    assert_eq!(
        fs::read_to_string(a_file).unwrap(),
        "export const value: string = null;\n"
    );
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
}

#[test]
fn cache_eviction_reopens_only_evicted_project_and_stop_drains_all_cached_services() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let log = fixture.path().join("lsp.jsonl");
    let projects: Vec<_> = (0..9)
        .map(|i| project(&workspace, &format!("service-{i}"), true))
        .collect();
    let mut app = app(fixture.path(), &workspace, controlled(&log));
    open(&mut app, &projects[0].1);
    app.dispatch(Command::StartProviders).unwrap();
    for (_, file) in &projects {
        open(&mut app, file);
        until(&mut app, |s| {
            s.language.analyzed_versions.contains_key(file)
        });
    }
    activate(&mut app, &projects[0].1);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&projects[0].1)
    });
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    let records: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let initialized = records
        .iter()
        .filter(|r| r["message"]["method"] == "initialize")
        .count();
    let shutdown = records
        .iter()
        .filter(|r| r["message"]["method"] == "shutdown")
        .count();
    assert_eq!(
        initialized, 10,
        "nine projects plus the evicted first project"
    );
    assert_eq!(
        shutdown, initialized,
        "all cached Stop acknowledgements are drained"
    );
    assert_eq!(
        records
            .iter()
            .filter(|r| r["cwd"].as_str() == projects[0].0.to_str()
                && r["message"]["method"] == "initialize")
            .count(),
        2
    );
}

#[test]
fn closing_an_inactive_project_document_closes_its_cached_lsp_document() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (_, a_file) = project(&workspace, "a", true);
    let (b, b_file) = project(&workspace, "b", false);
    let log = fixture.path().join("lsp.jsonl");
    let mut app = app(fixture.path(), &workspace, controlled(&log));
    open(&mut app, &a_file);
    app.dispatch(Command::StartProviders).unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    open(&mut app, &b_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    activate(&mut app, &a_file);
    let id = app.documents.iter().find(|d| d.path == b_file).unwrap().id;
    app.dispatch(Command::Close { id, discard: false }).unwrap();
    idle(&mut app);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    let records: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        records.iter().any(|r| r["cwd"].as_str() == b.to_str()
            && r["message"]["method"] == "textDocument/didClose")
    );
}

#[test]
#[ignore = "real Phpactor/Composer project isolation; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn real_php_projects_resolve_identical_class_names_with_their_own_composer_autoload() {
    let config =
        std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("CODISSEU_PROVIDER_CONFIG required");
    let configs: Vec<ProviderConfig> = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
    let configs = configs
        .into_iter()
        .filter(|p| p.language == Language::Php)
        .collect();
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let text = "<?php /* ação */ $service = new \\Shared\\Service(); $service->";
    let mut projects = Vec::new();
    for name in ["service-a", "service-b"] {
        let root = workspace.join(name);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("composer.json"), r#"{"name":"codisseu/workspace-fixture","autoload":{"psr-4":{"Shared\\":"src/"}},"scripts":{"post-autoload-dump":"@php -r \"file_put_contents('HOOK_EXECUTED','bad');\""}}"#).unwrap();
        let class = root.join("src/Service.php");
        let class_text = format!(
            "<?php namespace Shared; /* coração */ class Service {{ public function {}(): string {{ return 'ação'; }} }}",
            name.replace('-', "_")
        );
        let path = root.join("main.php");
        if name == "service-a" {
            fs::write(
                root.join(".editorconfig"),
                "root=true\n[*]\ncharset=latin1\n",
            )
            .unwrap();
            fs::write(
                &class,
                class_text.chars().map(|c| c as u8).collect::<Vec<_>>(),
            )
            .unwrap();
            fs::write(&path, text.chars().map(|c| c as u8).collect::<Vec<_>>()).unwrap();
        } else {
            fs::write(&class, class_text.as_bytes()).unwrap();
            fs::write(&path, text).unwrap();
        }
        let output = std::process::Command::new("composer")
            .args([
                "dump-autoload",
                "--no-scripts",
                "--no-plugins",
                "--no-interaction",
                "--optimize",
            ])
            .env("COMPOSER_DISABLE_NETWORK", "1")
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!root.join("HOOK_EXECUTED").exists());
        projects.push((root, path, class));
    }
    let mut app = app(fixture.path(), &workspace, configs);
    open(&mut app, &projects[0].1);
    app.dispatch(Command::StartProviders).unwrap();
    let mut saw_legacy_position_mismatch = false;
    for index in [0, 1, 0] {
        let (root, path, class) = &projects[index];
        open(&mut app, path);
        until(&mut app, |s| {
            s.language.analyzed_versions.contains_key(path)
        });
        assert_eq!(app.active_language_project(), Some(root.as_path()));
        let character = text[..text.find("Service()").unwrap()]
            .encode_utf16()
            .count() as u32
            + 2;
        app.language
            .dispatch(codisseu_core::LanguageCommand::Definition {
                path: path.clone(),
                version: 1,
                line: 0,
                character,
            })
            .unwrap();
        until(&mut app, |s| {
            s.language.definition.is_some() || s.language.definition_error.is_some()
        });
        assert!(
            app.language.definition_error.is_none(),
            "{:?}",
            app.language.definition_error
        );
        let result = app.language.definition.take().unwrap();
        assert!(
            result.targets.iter().any(|t| &t.path == class),
            "{:?}",
            result.targets
        );
        assert!(result.targets.iter().all(|t| t.path.starts_with(root)));
        if index == 0 {
            let raw = result.targets.iter().find(|t| &t.path == class).unwrap();
            let source: String = fs::read(class)
                .unwrap()
                .into_iter()
                .map(char::from)
                .collect();
            let actual = source[..source.find("class Service").unwrap()]
                .chars()
                .count();
            eprintln!(
                "Phpactor legacy saved source: reported_column={} decoded_class_column={actual}; precise selection intentionally withheld",
                raw.start_character
            );
            saw_legacy_position_mismatch |= raw.start_character as usize != actual;
        }
        let before = fs::read(class).unwrap();
        let doc = app.active_document().unwrap();
        let character_index = text[..text.find("Service()").unwrap()].chars().count() + 2;
        app.dispatch(Command::Definition {
            id: doc.id,
            expected_version: doc.version,
            character_index,
        })
        .unwrap();
        until(&mut app, |s| {
            s.active_document().is_some_and(|d| &d.path == class) || s.navigation.error.is_some()
        });
        assert!(app.navigation.error.is_none(), "{:?}", app.navigation.error);
        let target = app.active_document().unwrap();
        assert_eq!(
            target.encoding(),
            if index == 0 {
                codisseu_core::FileEncoding::Latin1
            } else {
                codisseu_core::FileEncoding::Utf8
            }
        );
        assert!(target.text().contains("coração"));
        if index == 0 {
            // A real provider limitation is exposed, never accepted as a precise
            // location: Latin1 byte offsets do not equal the Unicode positions.
            assert!(app.navigation.selection.is_none());
            assert!(
                app.navigation
                    .warning
                    .as_deref()
                    .is_some_and(|m| m.contains("posição do símbolo não certificada"))
            );
        } else {
            let selection = app.navigation.selection.as_ref().unwrap();
            assert_eq!(
                selection.range.start,
                target.text()[..target.text().find("class Service").unwrap()]
                    .chars()
                    .count()
            );
            assert!(
                target
                    .text()
                    .chars()
                    .skip(selection.range.start)
                    .take(selection.range.len())
                    .collect::<String>()
                    .starts_with("class Service")
            );
            assert!(app.navigation.warning.is_none());
        }
        assert_eq!(fs::read(class).unwrap(), before);
    }
    assert!(
        saw_legacy_position_mismatch,
        "fixture must reproduce a legacy position mismatch after the indexed file is opened as Unicode"
    );
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    for (root, _, _) in projects {
        assert!(!root.join("HOOK_EXECUTED").exists());
    }
}

#[test]
fn restored_project_config_without_matching_approval_cannot_execute() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (a, a_file) = project(&workspace, "a", true);
    let default = controlled(&fixture.path().join("global.log"));
    let custom = controlled(&fixture.path().join("custom.log"));
    let profile = fixture.path().join("policy.json");
    let mut app = app(fixture.path(), &workspace, default.clone());
    open(&mut app, &a_file);
    app.dispatch(Command::EnableProviderAutostart {
        profile: profile.clone(),
        fallback_config: None,
    })
    .unwrap();
    idle(&mut app);
    app.configure_project_providers(a.clone(), custom).unwrap();
    idle(&mut app);
    drop(app);
    let mut policy: Value = serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    policy["project_configs"][0]["configs"][0]["args"]
        .as_array_mut()
        .unwrap()
        .push(Value::String("unapproved-argument".into()));
    fs::write(&profile, serde_json::to_vec(&policy).unwrap()).unwrap();
    let mut restored = self::app(fixture.path(), &workspace, default);
    open(&mut restored, &a_file);
    restored
        .dispatch(Command::EnableProviderAutostart {
            profile,
            fallback_config: None,
        })
        .unwrap();
    idle(&mut restored);
    assert!(
        restored
            .dispatch(Command::StartProviders)
            .unwrap_err()
            .contains("Confie")
    );
    assert!(restored.language.statuses.is_empty());
    assert!(!fixture.path().join("custom.log").exists());
}

#[test]
fn aggregate_workspace_defers_providers_until_first_file_and_keeps_parent_trust_on_reopen() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("workspace");
    let (_, a_file) = project(&workspace, "a", true);
    let (_, b_file) = project(&workspace, "b", false);
    let log = fixture.path().join("lsp.jsonl");
    let profile = fixture.path().join("policy.json");
    let configs = controlled(&log);
    let mut app = app(fixture.path(), &workspace, configs.clone());
    app.dispatch(Command::EnableProviderAutostart {
        profile: profile.clone(),
        fallback_config: None,
    })
    .unwrap();
    idle(&mut app);
    app.dispatch(Command::StartProviders).unwrap();
    idle(&mut app);
    assert!(
        app.language.statuses.is_empty(),
        "opening aggregate folder must not index its entire tree"
    );
    assert!(!log.exists());
    open(&mut app, &a_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    open(&mut app, &b_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    drop(app);
    let mut restored = self::app(fixture.path(), &workspace, configs);
    restored
        .dispatch(Command::EnableProviderAutostart {
            profile,
            fallback_config: None,
        })
        .unwrap();
    idle(&mut restored);
    assert!(restored.provider_startup().trusted);
    assert!(!restored.provider_startup().needs_trust);
    assert!(restored.language.statuses.is_empty());
    open(&mut restored, &b_file);
    until(&mut restored, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    restored.dispatch(Command::StopProviders).unwrap();
    idle(&mut restored);
}

#[test]
fn changing_workspace_stops_cached_services_and_never_reuses_same_named_roots() {
    let fixture = tempfile::tempdir().unwrap();
    let workspace = fixture.path().join("first");
    let (_, a_file) = project(&workspace, "same-name", true);
    let (_, b_file) = project(&workspace, "other", false);
    let next_workspace = fixture.path().join("second");
    let (next_root, next_file) = project(&next_workspace, "same-name", true);
    let log = fixture.path().join("lsp.jsonl");
    let mut app = app(fixture.path(), &workspace, controlled(&log));
    open(&mut app, &a_file);
    app.dispatch(Command::StartProviders).unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&a_file)
    });
    open(&mut app, &b_file);
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&b_file)
    });
    app.dispatch(Command::OpenWorkspace(next_workspace.clone()))
        .unwrap();
    idle(&mut app);
    assert!(!app.any_project_providers_running());
    assert!(app.documents.is_empty());
    open(&mut app, &next_file);
    app.dispatch(Command::StartProviders).unwrap();
    until(&mut app, |s| {
        s.language.analyzed_versions.contains_key(&next_file)
    });
    assert_eq!(app.active_language_project(), Some(next_root.as_path()));
    assert_eq!(app.language.analyzed_versions.len(), 1);
    app.dispatch(Command::StopProviders).unwrap();
    idle(&mut app);
    let records: Vec<Value> = fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    for (workspace, count) in [(&workspace, 2), (&next_workspace, 1)] {
        assert_eq!(
            records
                .iter()
                .filter(|r| r["cwd"]
                    .as_str()
                    .is_some_and(|p| Path::new(p).starts_with(workspace))
                    && r["message"]["method"] == "initialize")
                .count(),
            count
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| r["cwd"]
                    .as_str()
                    .is_some_and(|p| Path::new(p).starts_with(workspace))
                    && r["message"]["method"] == "shutdown")
                .count(),
            count
        );
    }
}
