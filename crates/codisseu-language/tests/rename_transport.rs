use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, seconds: u64, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(seconds);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{:?} {:?} {:?}",
            s.statuses,
            s.last_error,
            s.rename_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn records(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
fn fixture(mode: &str) -> (tempfile::TempDir, LanguageService, PathBuf, PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("workspace");
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let source = root.join("main.ts");
    let target = root.join("target.ts");
    let outside = d.path().join("outside.ts");
    let log = d.path().join("wire.jsonl");
    for (p, t) in [
        (&source, "😀rainfall e\u{301}"),
        (&target, "rainfall"),
        (&outside, "rainfall"),
    ] {
        fs::write(p, t).unwrap();
    }
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs: vec![ProviderConfig {
            language: Language::TypeScript,
            program: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/rename_server.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
                log.display().to_string(),
                source.display().to_string(),
                target.display().to_string(),
                outside.display().to_string(),
            ],
        }],
    })
    .unwrap();
    wait(&mut s, 5, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    s.dispatch(LanguageCommand::Sync {
        path: source.clone(),
        version: 2,
        text: "😀rainfall e\u{301}".into(),
    })
    .unwrap();
    wait(&mut s, 5, |s| !s.is_busy());
    (d, s, source, log)
}
fn command(path: &std::path::Path, prepare: bool, name: &str) -> LanguageCommand {
    if prepare {
        LanguageCommand::PrepareRename {
            path: path.into(),
            version: 2,
            line: 0,
            character: 4,
        }
    } else {
        LanguageCommand::Rename {
            path: path.into(),
            version: 2,
            line: 0,
            character: 4,
            new_name: name.into(),
        }
    }
}
#[test]
fn prepare_capabilities_range_placeholder_default_refusal_and_static_fallback() {
    for mode in [
        "safe",
        "range",
        "default",
        "fallback",
        "fallback_options",
        "prepare_null",
        "prepare_surrogate",
        "missing",
        "error",
    ] {
        let (_d, mut s, p, log) = fixture(mode);
        s.dispatch(command(&p, true, "forecast")).unwrap();
        wait(&mut s, 5, |s| !s.rename_pending());
        let wire = records(&log);
        let init = &wire[0];
        assert_eq!(
            init.pointer("/params/capabilities/textDocument/rename/prepareSupport"),
            Some(&Value::Bool(true))
        );
        assert!(
            init.pointer("/params/capabilities/textDocument/rename/prepareSupportDefaultBehavior")
                .is_none()
        );
        assert_eq!(
            init.pointer("/params/capabilities/workspace/applyEdit"),
            Some(&Value::Bool(false))
        );
        if ["safe", "range", "default", "fallback", "fallback_options"].contains(&mode) {
            let r = s.prepare_rename.as_ref().unwrap();
            assert_eq!(r.used_fallback, mode.starts_with("fallback"));
            if mode == "safe" {
                assert_eq!(r.placeholder.as_deref(), Some("rainfall"));
                assert_eq!(r.range.as_ref().unwrap().start_character, 2);
            }
            if mode == "default" || mode.starts_with("fallback") {
                assert!(r.range.is_none());
            }
        } else {
            assert!(s.rename_error.is_some(), "{mode}");
            assert!(s.prepare_rename.is_none());
        }
        if mode.starts_with("fallback") || mode == "missing" {
            assert!(
                !wire
                    .iter()
                    .any(|v| v["method"] == "textDocument/prepareRename")
            );
        }
    }
}
#[test]
fn rename_workspace_forms_versions_and_invalid_edits_fail_all_or_nothing() {
    for mode in [
        "safe",
        "versioned",
        "mixed",
        "resource",
        "annotations",
        "annotated",
        "outside",
        "remote",
        "invalid_range",
        "surrogate",
        "overlap",
        "nul",
        "null",
        "empty",
        "stale",
        "error",
        "missing",
        "crash",
        "apply",
    ] {
        let (d, mut s, p, log) = fixture(mode);
        s.dispatch(command(&p, false, "forecast")).unwrap();
        wait(&mut s, 5, |s| !s.rename_pending());
        if ["safe", "versioned", "apply"].contains(&mode) {
            let r = s.rename.as_ref().unwrap();
            assert_eq!(r.new_name, "forecast");
            assert_eq!(r.changes.len(), 2);
            assert_eq!(r.changes[0].edits[0].start_character, 2);
            if mode == "versioned" {
                assert_eq!(r.changes[0].version, Some(2));
                assert_eq!(r.changes[1].version, Some(0));
            }
        } else {
            assert!(s.rename_error.is_some(), "{mode}");
            assert!(s.rename.is_none(), "{mode}");
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), "😀rainfall e\u{301}");
        assert_eq!(
            fs::read_to_string(d.path().join("workspace/target.ts")).unwrap(),
            "rainfall"
        );
        if mode == "apply" {
            wait(&mut s, 5, |_| {
                records(&log)
                    .iter()
                    .any(|v| v["id"] == "forbidden-apply" && v.get("error").is_some())
            });
        }
    }
}
#[test]
fn rename_and_prepare_reordered_ids_and_any_buffer_change_discard_late_responses() {
    for prepare in [true, false] {
        let (_d, mut s, p, log) = fixture("reorder");
        s.dispatch(command(&p, prepare, "first")).unwrap();
        let method = if prepare {
            "textDocument/prepareRename"
        } else {
            "textDocument/rename"
        };
        wait(&mut s, 5, |_| {
            records(&log).iter().any(|v| v["method"] == method)
        });
        let mut second = command(&p, prepare, "second");
        match &mut second {
            LanguageCommand::PrepareRename { character, .. }
            | LanguageCommand::Rename { character, .. } => *character = 5,
            _ => unreachable!(),
        }
        s.dispatch(second).unwrap();
        wait(&mut s, 5, |s| !s.rename_pending());
        if !prepare {
            assert_eq!(s.rename.as_ref().unwrap().character, 5);
            assert_eq!(s.rename.as_ref().unwrap().new_name, "second");
            assert_eq!(
                s.rename.as_ref().unwrap().changes[0].edits[0].new_text,
                "second"
            );
        } else {
            assert_eq!(s.prepare_rename.as_ref().unwrap().character, 5);
        }
        assert!(
            records(&log)
                .iter()
                .any(|v| v["method"] == "$/cancelRequest")
        );
        let (_d, mut s, p, log) = fixture("on_change");
        s.dispatch(command(&p, prepare, "forecast")).unwrap();
        wait(&mut s, 5, |_| {
            records(&log).iter().any(|v| v["method"] == method)
        });
        s.dispatch(LanguageCommand::Sync {
            path: p.clone(),
            version: 3,
            text: "// changed\n😀rainfall e\u{301}".into(),
        })
        .unwrap();
        wait(&mut s, 5, |_| {
            records(&log).iter().any(|v| v.get("replySent").is_some())
        });
        wait(&mut s, 5, |s| !s.is_busy());
        assert!(s.rename.is_none() && s.prepare_rename.is_none() && s.rename_error.is_none());
    }
}
#[test]
fn rename_and_prepare_actual_deadline_cancel_and_stop_never_publish_late() {
    for prepare in [true, false] {
        let (_d, mut s, p, log) = fixture("silent");
        s.dispatch(command(&p, prepare, "forecast")).unwrap();
        let start = Instant::now();
        wait(&mut s, 15, |s| !s.rename_pending());
        assert!(start.elapsed() >= Duration::from_secs(9));
        assert!(
            s.rename_error
                .as_ref()
                .unwrap()
                .contains("Timeout de rename")
        );
        wait(&mut s, 5, |_| {
            records(&log)
                .iter()
                .any(|v| v["method"] == "$/cancelRequest")
        });
        let wire = records(&log);
        let id = wire
            .iter()
            .find(|v| v["method"] == "$/cancelRequest")
            .unwrap()["params"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(id.starts_with(if prepare {
            "prepare-rename-"
        } else {
            "rename-"
        }));
        s.dispatch(LanguageCommand::Stop).unwrap();
        wait(&mut s, 5, |s| !s.is_busy());
        assert!(s.rename.is_none() && s.prepare_rename.is_none());
    }
}

#[test]
fn explicit_cancel_close_and_stop_invalidate_pending_prepare_and_rename() {
    for prepare in [true, false] {
        for kind in ["cancel", "close", "stop"] {
            let (_d, mut s, p, log) = fixture("on_change_error");
            s.dispatch(command(&p, prepare, "forecast")).unwrap();
            let method = if prepare {
                "textDocument/prepareRename"
            } else {
                "textDocument/rename"
            };
            wait(&mut s, 5, |_| {
                records(&log).iter().any(|v| v["method"] == method)
            });
            match kind {
                "cancel" => {
                    s.dispatch(LanguageCommand::CancelRename).unwrap();
                    s.dispatch(LanguageCommand::Sync {
                        path: p.clone(),
                        version: 3,
                        text: "😀rainfall changed".into(),
                    })
                    .unwrap();
                }
                "close" => {
                    s.dispatch(LanguageCommand::Close { path: p.clone() })
                        .unwrap();
                }
                _ => {
                    s.dispatch(LanguageCommand::Stop).unwrap();
                }
            }
            if kind != "stop" {
                wait(&mut s, 5, |_| {
                    records(&log).iter().any(|v| v.get("replySent").is_some())
                });
            }
            wait(&mut s, 5, |s| !s.is_busy());
            assert!(!s.rename_pending());
            assert!(s.rename.is_none() && s.prepare_rename.is_none() && s.rename_error.is_none());
        }
    }
}
