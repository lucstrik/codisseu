//! Real TS language service proposals and compiler proof for JS/TS methods.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
    semantic::TypeScriptRuntime, utf16_byte_offset,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    thread,
    time::{Duration, Instant},
};

fn wait(service: &mut LanguageService, done: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if done(service) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{:?} {:?} {:?}",
            service.statuses,
            service.last_error,
            service.rename_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn position(text: &str, byte: usize) -> (u32, u32) {
    let prefix = &text[..byte];
    (
        prefix.bytes().filter(|byte| *byte == b'\n').count() as u32,
        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}
fn proof_snapshots(
    service: &LanguageService,
    texts: &BTreeMap<PathBuf, String>,
    new_name: &str,
) -> Vec<Value> {
    let proposal = service.rename.as_ref().unwrap();
    texts.iter().map(|(path, before)| {
        let mut after = before.clone();
        let mut edits = Vec::new();
        if let Some(change) = proposal.changes.iter().find(|change| &change.path == path) {
            for edit in change.edits.iter().rev() {
                let start = utf16_byte_offset(before, edit.start_line, edit.start_character).unwrap();
                let end = utf16_byte_offset(before, edit.end_line, edit.end_character).unwrap();
                assert_eq!(&before[start..end], "rainfall");
                assert_eq!(edit.new_text, new_name);
                after.replace_range(start..end, &edit.new_text);
            }
            edits = change.edits.iter().map(|edit| json!({"range":{"start_line":edit.start_line,"start_character":edit.start_character,"end_line":edit.end_line,"end_character":edit.end_character},"new_text":edit.new_text})).collect();
        }
        json!({"path":path,"before":before,"after":after,"edits":edits})
    }).collect()
}

#[test]
#[ignore = "actual JS/TS provider; requires explicit CODISSEU_PROVIDER_CONFIG"]
fn real_js_and_ts_method_rename_preserves_closed_callers_dirty_unicode_and_homonyms() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("explicit provider configuration required"),
        )
        .unwrap(),
    )
    .unwrap();
    let configs = configs
        .into_iter()
        .filter(|config| config.language == Language::TypeScript)
        .collect::<Vec<_>>();
    assert_eq!(configs.len(), 1);
    let runtime = TypeScriptRuntime::from_config(&configs[0]).unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/rename-web-audit");
    for (language, extension, config_name, inferred) in [
        ("javascript", "js", "jsconfig.json", false),
        ("typescript", "ts", "tsconfig.json", false),
        ("javascript", "js", "jsconfig.json", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        for entry in fs::read_dir(fixtures.join(language)).unwrap() {
            let entry = entry.unwrap();
            fs::copy(entry.path(), root.join(entry.file_name())).unwrap();
        }
        if inferred {
            fs::remove_file(root.join(config_name)).unwrap();
        }
        let disk = fs::read_dir(&root)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (path.clone(), fs::read_to_string(path).unwrap())
            })
            .collect::<BTreeMap<_, _>>();
        let caller = root.join(format!("main.{extension}"));
        let declaration = root.join(format!("weather.{extension}"));
        let closed = root.join(format!("closed.{extension}"));
        let other = root.join(format!("other.{extension}"));
        let mut texts = disk
            .iter()
            .filter(|(path, _)| path.extension().is_some_and(|ext| ext == extension))
            .map(|(path, text)| (path.clone(), text.clone()))
            .collect::<BTreeMap<_, _>>();
        let dirty = format!(
            "// unsaved 😀 e\u{301}\n{}\nweather.rainfall();\n",
            texts[&caller]
        );
        texts.insert(caller.clone(), dirty.clone());
        let mut service = LanguageService::new();
        service
            .dispatch(LanguageCommand::Start {
                workspace: root.clone(),
                configs: configs.clone(),
            })
            .unwrap();
        service
            .dispatch(LanguageCommand::Sync {
                path: caller.clone(),
                version: 1,
                text: disk[&caller].clone(),
            })
            .unwrap();
        service
            .dispatch(LanguageCommand::Sync {
                path: caller.clone(),
                version: 2,
                text: dirty.clone(),
            })
            .unwrap();
        wait(&mut service, |service| {
            service.statuses.len() == 1
                && service.statuses[0].state == ProviderState::Ready
                && !service.is_busy()
        });
        let origin = dirty.find("rainfall()").unwrap() + 2;
        let (line, character) = position(&dirty, origin);
        wait(&mut service, |service| {
            service.analyzed_versions.get(&caller) == Some(&2)
        });
        service
            .dispatch(LanguageCommand::Hover {
                path: caller.clone(),
                version: 2,
                line,
                character,
            })
            .unwrap();
        wait(&mut service, |service| !service.hover_pending());
        eprintln!(
            "{language} inferred={inferred} {line}:{character}: hover={:?}, diagnostics={:?}",
            service.hover, service.diagnostics
        );
        service
            .dispatch(LanguageCommand::PrepareRename {
                path: caller.clone(),
                version: 2,
                line,
                character,
            })
            .unwrap();
        wait(&mut service, |service| !service.rename_pending());
        assert!(
            service.prepare_rename.is_some(),
            "{language} inferred={inferred}: {:?}",
            service.rename_error
        );
        service
            .dispatch(LanguageCommand::Rename {
                path: caller.clone(),
                version: 2,
                line,
                character,
                new_name: "forecast".into(),
            })
            .unwrap();
        wait(&mut service, |service| !service.rename_pending());
        assert!(
            service.rename.is_some(),
            "{language} inferred={inferred}: {:?}",
            service.rename_error
        );
        if inferred {
            let partial = proof_snapshots(&service, &texts, "forecast");
            let result = runtime.verify(&root, json!({"root":root,"origin":{"path":caller,"line":line,"character":character},"newName":"forecast","configText":null,"snapshots":partial}), &AtomicBool::new(false)).unwrap();
            assert_eq!(
                result["coverage"], "Rejected",
                "inferred omitted caller must remain blocked: {result}"
            );
            assert!(
                result["conflicts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|finding| finding["message"]
                        .as_str()
                        .unwrap()
                        .contains("omite referência"))
            );
            assert_eq!(result["checkedDeclarations"], 1);
            let missing = result["missingReferences"]
                .as_array()
                .unwrap()
                .iter()
                .find(|missing| missing["path"] == json!(closed))
                .expect("exact missing closed caller origin");
            let closed_line =
                u32::try_from(missing["range"]["start_line"].as_u64().unwrap()).unwrap();
            let closed_character =
                u32::try_from(missing["range"]["start_character"].as_u64().unwrap()).unwrap();
            service
                .dispatch(LanguageCommand::RenameClosed {
                    path: closed.clone(),
                    text: texts[&closed].clone(),
                    line: closed_line,
                    character: closed_character,
                    new_name: "forecast".into(),
                })
                .unwrap();
            wait(&mut service, |service| !service.rename_pending());
            assert!(
                service.rename.is_some(),
                "inferred closed query: {:?}",
                service.rename_error
            );
            assert_eq!(
                service.rename.as_ref().unwrap().transient_document_version,
                Some(1)
            );
        }
        let proposal = service.rename.as_ref().unwrap();
        assert_eq!(
            proposal.changes.len(),
            3,
            "{language} inferred={inferred}: {proposal:?}"
        );
        for change in &proposal.changes {
            assert!(change.path == caller || change.path == declaration || change.path == closed);
            assert_eq!(
                change.edits.len(),
                if change.path == caller { 2 } else { 1 }
            );
        }
        let snapshots = proof_snapshots(&service, &texts, "forecast");
        let result = runtime.verify(&root, json!({"root":root,"origin":{"path":caller,"line":line,"character":character},"newName":"forecast","configText":if inferred {None} else {Some(disk[&root.join(config_name)].as_str())},"configName":if inferred {"tsconfig.json"} else {config_name},"snapshots":snapshots}), &AtomicBool::new(false)).unwrap();
        assert_eq!(
            result["coverage"], "Verified",
            "{language} inferred={inferred}: {result}"
        );
        let changed = snapshots
            .iter()
            .find(|snapshot| snapshot["path"] == json!(caller))
            .unwrap()["after"]
            .as_str()
            .unwrap();
        assert!(changed.contains("// rainfall is a comment"));
        assert!(changed.contains("\"rainfall\""));
        assert_eq!(
            snapshots
                .iter()
                .find(|snapshot| snapshot["path"] == json!(other))
                .unwrap()["before"],
            snapshots
                .iter()
                .find(|snapshot| snapshot["path"] == json!(other))
                .unwrap()["after"]
        );
        eprintln!(
            "{language} inferred={inferred}: provider returned 1 declaration/3 calls; compiler {} verified exact bindings",
            result["compilerVersion"]
        );
        if !inferred {
            let (declaration_line, declaration_character) = position(
                &texts[&declaration],
                texts[&declaration].find("rainfall").unwrap() + 2,
            );
            service
                .dispatch(LanguageCommand::RenameClosed {
                    path: declaration.clone(),
                    text: texts[&declaration].clone(),
                    line: declaration_line,
                    character: declaration_character,
                    new_name: "forecast".into(),
                })
                .unwrap();
            wait(&mut service, |service| !service.rename_pending());
            assert!(
                service.rename.is_some(),
                "closed declaration: {:?}",
                service.rename_error
            );
            assert_eq!(
                service.rename.as_ref().unwrap().transient_document_version,
                Some(1)
            );
            assert_eq!(service.rename.as_ref().unwrap().changes.len(), 3);
            let snapshots = proof_snapshots(&service, &texts, "forecast");
            let result = runtime.verify(&root, json!({"root":root,"origin":{"path":declaration,"line":declaration_line,"character":declaration_character},"newName":"forecast","configText":disk[&root.join(config_name)],"configName":config_name,"snapshots":snapshots}), &AtomicBool::new(false)).unwrap();
            assert_eq!(
                result["coverage"], "Verified",
                "closed declaration: {result}"
            );
        }
        if language == "javascript" && !inferred {
            let collision = texts[&declaration].replace(
                "    rainfall()",
                "    forecast() { return 'sun'; }\n    rainfall()",
            );
            assert_ne!(collision, texts[&declaration]);
            texts.insert(declaration.clone(), collision.clone());
            service
                .dispatch(LanguageCommand::Sync {
                    path: declaration.clone(),
                    version: 1,
                    text: collision,
                })
                .unwrap();
            wait(&mut service, |service| {
                service.analyzed_versions.get(&declaration) == Some(&1)
            });
            service
                .dispatch(LanguageCommand::Rename {
                    path: caller.clone(),
                    version: 2,
                    line,
                    character,
                    new_name: "forecast".into(),
                })
                .unwrap();
            wait(&mut service, |service| !service.rename_pending());
            assert!(
                service.rename.is_some(),
                "provider collision characterization: {:?}",
                service.rename_error
            );
            let snapshots = proof_snapshots(&service, &texts, "forecast");
            let result = runtime.verify(&root, json!({"root":root,"origin":{"path":caller,"line":line,"character":character},"newName":"forecast","configText":disk[&root.join(config_name)],"configName":config_name,"snapshots":snapshots}), &AtomicBool::new(false)).unwrap();
            assert_eq!(
                result["coverage"], "Rejected",
                "unchecked JS collision must be rejected despite provider proposal: {result}"
            );
        }
        for (path, text) in &disk {
            assert_eq!(
                fs::read_to_string(path).unwrap(),
                *text,
                "provider/proof must not save"
            );
        }
        service.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut service, |service| !service.is_busy());
    }
}
