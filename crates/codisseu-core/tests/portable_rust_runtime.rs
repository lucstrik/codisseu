//! Run explicitly against a relocated bundle, with neither host Rust nor a C
//! linker. All projects, generated binaries and debugger processes are owned.
use codisseu_core::{AppService, Command as AppCommand, Coverage};
use codisseu_debug::{DebugCommand, DebugConfiguration, DebugRuntime, DebugService, DebugState};
use codisseu_language::{Language, LanguageCommand, LanguageService, ProviderState};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};
fn language_wait(service: &mut LanguageService, predicate: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(45);
    loop {
        service.poll();
        if predicate(service) {
            return;
        }
        assert!(
            service.last_error.is_none(),
            "{:?} {:?}",
            service.last_error,
            service.statuses
        );
        assert!(
            Instant::now() < end,
            "analyzer timeout: {:?} hover={:?}",
            service.statuses,
            service.hover_error
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn debug_wait(service: &mut DebugService, predicate: impl Fn(&DebugService) -> bool) {
    let end = Instant::now() + Duration::from_secs(35);
    loop {
        service.poll();
        if predicate(service) {
            return;
        }
        assert!(
            !service
                .sessions
                .iter()
                .any(|s| s.state == DebugState::Failed),
            "{:?}",
            service.sessions
        );
        assert!(
            Instant::now() < end,
            "debugger timeout: {:?}",
            service.sessions
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn cargo(root: &Path, subcommand: &str, args: &[&str]) -> String {
    let executable = codisseu_runtime::cargo().unwrap();
    let output = Command::new(executable)
        .arg(subcommand)
        .arg("--offline")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo {subcommand}: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn explicit_toolchain_configuration() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::create_dir(root.join(".cargo")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"portable_config\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        root.join("rust-toolchain.toml"),
        "[toolchain]\nchannel=\"codisseu-unavailable-version-fixture\"\n",
    )
    .unwrap();
    fs::write(
        root.join(".cargo/config.toml"),
        "[build]\nrustflags=[\"--cfg\",\"codisseu_config\"]\n",
    )
    .unwrap();
    fs::write(root.join("src/main.rs"), "#[cfg(not(codisseu_config))]\ncompile_error!(\"explicit Cargo config ignored\");\nfn main() {}\n").unwrap();
    cargo(root, "check", &[]);
    fs::write(root.join("src/main.rs"), "#[cfg(not(codisseu_flags))]\ncompile_error!(\"explicit RUSTFLAGS ignored\");\nfn main() {}\n").unwrap();
    let status = Command::new(codisseu_runtime::cargo().unwrap())
        .args(["check", "--offline"])
        .env("RUSTFLAGS", "--cfg codisseu_flags")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let rustc = Command::new(codisseu_runtime::cargo().unwrap())
        .args(["check", "--offline"])
        .env("RUSTC", "/bin/false")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!rustc.status.success());
    assert!(String::from_utf8_lossy(&rustc.stderr).contains("/bin/false"));
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(
        root.join(".cargo/config.toml"),
        "[target.x86_64-unknown-linux-gnu]\nlinker=\"/bin/false\"\n",
    )
    .unwrap();
    let linker = Command::new(codisseu_runtime::cargo().unwrap())
        .args(["build", "--offline"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!linker.status.success());
    assert!(String::from_utf8_lossy(&linker.stderr).contains("/bin/false"));
}
fn app_wait(app: &mut AppService, predicate: impl Fn(&AppService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if predicate(app) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "core portable Rust timeout: {} {:?} fixes={:?} rename={:?}",
            app.status,
            app.last_error,
            app.code_actions.error,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn app_idle(app: &mut AppService) {
    app_wait(app, |a| !a.is_busy());
}
fn rename_preview(app: &mut AppService, id: u64, caret: usize, new_name: &str) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        let version = app.active_document().unwrap().version;
        app.dispatch(AppCommand::PrepareRename {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        app_idle(app);
        if app.rename.prepared {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "rename prepare: {:?}",
            app.rename.error
        );
        thread::sleep(Duration::from_millis(50));
    }
    app.dispatch(AppCommand::PreviewRename {
        new_name: new_name.into(),
    })
    .unwrap();
    app_idle(app);
}
fn app_features(
    root: &Path,
    source: &Path,
    original: &str,
    resources: &codisseu_runtime::Resources,
) {
    let mut app = AppService::new(root.join(".codisseu/session.json"));
    app.dispatch(AppCommand::OpenWorkspace(root.into()))
        .unwrap();
    app_idle(&mut app);
    app.dispatch(AppCommand::OpenFile(source.into())).unwrap();
    app_idle(&mut app);
    let provider = resources
        .providers()
        .unwrap()
        .into_iter()
        .find(|c| c.language == "Rust")
        .unwrap();
    app.dispatch(AppCommand::ConfigureProviders(vec![
        codisseu_language::ProviderConfig {
            language: Language::Rust,
            program: provider.program,
            args: provider.args,
        },
    ]))
    .unwrap();
    app.dispatch(AppCommand::StartProviders).unwrap();
    app_wait(&mut app, |a| {
        !a.is_busy()
            && a.language
                .statuses
                .iter()
                .any(|p| p.state == ProviderState::Ready)
    });
    let unresolved = format!(
        "{original}\npub fn import_fix() -> i32 {{\n    let mut values = HashMap::new();\n    values.insert(\"answer\", 42);\n    let _emoji = \"😀\"; let result = doubled(21);\n    result + values[\"answer\"]\n}}\n"
    );
    let doc = app.active_document().unwrap();
    let id = doc.id;
    let edit = AppCommand::Edit {
        id,
        expected_version: doc.version,
        text: unresolved.clone(),
    };
    app.dispatch(edit).unwrap();
    app_idle(&mut app);
    let caret = unresolved[..unresolved.find("HashMap").unwrap()]
        .chars()
        .count()
        + 2;
    let deadline = Instant::now() + Duration::from_secs(45);
    let token = loop {
        let version = app.active_document().unwrap().version;
        app.dispatch(AppCommand::CodeActions {
            id,
            expected_version: version,
            character_index: caret,
        })
        .unwrap();
        app_idle(&mut app);
        if let Some(item) = app
            .code_actions
            .items
            .iter()
            .find(|i| i.title.contains("HashMap") && i.disabled_reason.is_none())
        {
            break item.token;
        }
        assert!(
            Instant::now() < deadline,
            "HashMap quick fix: {:?} {:?}",
            app.code_actions.items,
            app.code_actions.error
        );
        thread::sleep(Duration::from_millis(50));
    };
    app.dispatch(AppCommand::PreviewCodeAction { token })
        .unwrap();
    app_idle(&mut app);
    assert!(
        app.code_actions.has_preview(),
        "{:?}",
        app.code_actions.error
    );
    assert!(
        app.code_actions
            .files
            .iter()
            .any(|f| f.after.contains("use std::collections::HashMap;"))
    );
    app.dispatch(AppCommand::ApplyCodeAction).unwrap();
    app_idle(&mut app);
    let fixed = app.active_document().unwrap().text().to_owned();
    assert!(fixed.contains("use std::collections::HashMap;"));
    assert_eq!(fs::read_to_string(source).unwrap(), original);
    app.dispatch(AppCommand::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), unresolved);
    app.dispatch(AppCommand::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), fixed);
    app.dispatch(AppCommand::Save { id }).unwrap();
    app_idle(&mut app);
    cargo(root, "check", &[]);
    // Hints come from the current version and never alter text, disk or Undo.
    let doc = app.active_document().unwrap();
    let version = doc.version;
    let lines = doc.line_count();
    app.dispatch(AppCommand::InlayHints {
        id,
        expected_version: version,
        lines: 0..lines,
    })
    .unwrap();
    app_wait(&mut app, |a| !a.inlay_hints.is_pending());
    assert!(
        app.inlay_hints.error.is_none(),
        "{:?}",
        app.inlay_hints.error
    );
    assert!(
        app.inlay_hints
            .items
            .iter()
            .any(|h| h.kind == Some(1) && h.label.contains("i32")),
        "{:?}",
        app.inlay_hints.items
    );
    assert!(
        app.inlay_hints
            .items
            .iter()
            .any(|h| h.kind == Some(2) && h.label == "k:"),
        "{:?}",
        app.inlay_hints.items
    );
    assert_eq!(app.active_document().unwrap().text(), fixed);
    for (label, expected) in [("k:", "\"answer\""), ("v:", "42")] {
        let hint = app
            .inlay_hints
            .items
            .iter()
            .find(|h| h.kind == Some(2) && h.label == label)
            .unwrap();
        let at = codisseu_language::utf16_byte_offset(&fixed, hint.line, hint.character).unwrap();
        assert!(
            fixed[at..].starts_with(expected),
            "{hint:?} must label the real argument"
        );
    }
    assert_eq!(app.active_document().unwrap().version, version);
    app.dispatch(AppCommand::Undo { id }).unwrap();
    app.dispatch(AppCommand::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    app.dispatch(AppCommand::Save { id }).unwrap();
    app_idle(&mut app);
    let caret = original[..original.find("doubled").unwrap()]
        .chars()
        .count()
        + 2;
    let macro_source = original.replace(
        "let answer = doubled(21); assert_eq!(answer, 42);",
        "assert_eq!(doubled(21), 42);",
    );
    assert_ne!(macro_source, original);
    let version = app.active_document().unwrap().version;
    app.dispatch(AppCommand::Edit {
        id,
        expected_version: version,
        text: macro_source.clone(),
    })
    .unwrap();
    app_idle(&mut app);
    rename_preview(&mut app, id, caret, "twice");
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Unsupported
    );
    assert!(app.dispatch(AppCommand::ApplyRename).is_err());
    assert_eq!(app.active_document().unwrap().text(), macro_source);
    assert_eq!(fs::read_to_string(source).unwrap(), original);
    app.dispatch(AppCommand::CancelRename).unwrap();
    app.dispatch(AppCommand::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    rename_preview(&mut app, id, caret, "twice");
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Verified,
        "{:?}",
        app.rename.validation
    );
    app.dispatch(AppCommand::ApplyRename).unwrap();
    app_idle(&mut app);
    let renamed = app.active_document().unwrap().text().to_owned();
    assert!(renamed.contains("fn twice"));
    assert!(renamed.contains("twice(answer)"));
    app.dispatch(AppCommand::Undo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), original);
    app.dispatch(AppCommand::Redo { id }).unwrap();
    assert_eq!(app.active_document().unwrap().text(), renamed);
    app.dispatch(AppCommand::Save { id }).unwrap();
    app_idle(&mut app);
    cargo(root, "check", &[]);
    app.dispatch(AppCommand::Undo { id }).unwrap();
    app.dispatch(AppCommand::Save { id }).unwrap();
    app_idle(&mut app);
    assert_eq!(fs::read_to_string(source).unwrap(), original);
    app.dispatch(AppCommand::StopProviders).unwrap();
    app_idle(&mut app);
    assert!(!app.providers_running());
}
#[test]
#[ignore = "relocated private Rust acceptance; requires CODISSEU_RESOURCE_DIR; executes own std fixture"]
fn private_rust_build_run_test_fmt_clippy_analyzer_and_native_debug() {
    let resources = codisseu_runtime::discover()
        .unwrap()
        .expect("relocated bundle required");
    let sysroot = resources.rust_sysroot().unwrap().unwrap();
    explicit_toolchain_configuration();
    assert!(sysroot.starts_with(resources.root()));
    assert!(
        sysroot
            .join("lib/rustlib/src/rust/library/core/src/lib.rs")
            .is_file()
    );
    let temp = tempfile::Builder::new()
        .prefix("codisseu-portable-Rust ação ")
        .tempdir()
        .unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_portable_rust\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    let source = root.join("src/main.rs");
    let text = "// ação 😀\nfn doubled(value: i32) -> i32 {\n    value * 2\n}\nfn main() {\n    let answer = 42;\n    let result = doubled(answer);\n    let _readings = Vec::<i32>::new();\n    println!(\"Codisseu result={result}\");\n}\n#[test]\nfn calculation() { let answer = doubled(21); assert_eq!(answer, 42); }\n";
    fs::write(&source, text).unwrap();
    cargo(&root, "check", &[]);
    cargo(&root, "build", &[]);
    let run = cargo(&root, "run", &[]);
    assert!(run.contains("Codisseu result=84"));
    let tests = cargo(&root, "test", &[]);
    assert!(tests.contains("1 passed"), "{tests}");
    cargo(&root, "clippy", &["--", "-D", "warnings"]);
    cargo(&root, "doc", &["--no-deps"]);
    let executable = root.join("target/debug/codisseu_portable_rust");
    assert!(executable.is_file());
    let fmt = root.join("format.rs");
    fs::write(
        &fmt,
        "fn format( ){let answer=40+2;println!(\"{}\",answer);}\n",
    )
    .unwrap();
    let formatted = Command::new(resources.tool("rustfmt").unwrap())
        .args(["--edition", "2024", "--emit", "stdout"])
        .arg(&fmt)
        .output()
        .unwrap();
    assert!(formatted.status.success());
    assert!(String::from_utf8_lossy(&formatted.stdout).contains("40 + 2"));
    assert!(fs::read_to_string(&fmt).unwrap().contains("40+2"));
    let mut analyzer = LanguageService::new();
    let config = resources
        .providers()
        .unwrap()
        .into_iter()
        .find(|c| c.language == "Rust")
        .unwrap();
    analyzer
        .dispatch(LanguageCommand::Start {
            workspace: root.clone(),
            configs: vec![codisseu_language::ProviderConfig {
                language: Language::Rust,
                program: config.program,
                args: config.args,
            }],
        })
        .unwrap();
    analyzer
        .dispatch(LanguageCommand::Sync {
            path: source.clone(),
            version: 1,
            text: text.into(),
        })
        .unwrap();
    language_wait(&mut analyzer, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready) && !s.is_busy()
    });
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        analyzer
            .dispatch(LanguageCommand::Hover {
                path: source.clone(),
                version: 1,
                line: 6,
                character: 18,
            })
            .unwrap();
        language_wait(&mut analyzer, |s| !s.hover_pending());
        if analyzer
            .hover
            .as_ref()
            .is_some_and(|h| h.text.contains("doubled"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hover never ready {:?} {:?}",
            analyzer.hover,
            analyzer.hover_error
        );
        thread::sleep(Duration::from_millis(50));
    }
    let definition_deadline = Instant::now() + Duration::from_secs(45);
    loop {
        analyzer
            .dispatch(LanguageCommand::Definition {
                path: source.clone(),
                version: 1,
                line: 7,
                character: 22,
            })
            .unwrap();
        language_wait(&mut analyzer, |s| !s.definition_pending());
        if analyzer
            .definition
            .as_ref()
            .is_some_and(|d| d.targets.iter().any(|t| t.path.starts_with(&sysroot)))
        {
            break;
        }
        assert!(
            Instant::now() < definition_deadline,
            "Vec std definition never indexed: {:?} {:?}",
            analyzer.definition,
            analyzer.definition_error
        );
        thread::sleep(Duration::from_millis(50));
    }
    analyzer.dispatch(LanguageCommand::Stop).unwrap();
    language_wait(&mut analyzer, |s| !s.is_busy());
    app_features(&root, &source, text, &resources);
    let mut debugger = DebugService::new();
    debugger
        .execute(DebugCommand::SetBreakpoints {
            path: source.clone(),
            lines: vec![7],
        })
        .unwrap();
    let id = debugger
        .execute(DebugCommand::Start(DebugConfiguration {
            name: "Portable Rust".into(),
            runtime: DebugRuntime::Rust,
            program: Some(executable.clone()),
            executable: executable.clone(),
            cwd: root.clone(),
            trusted: true,
            ..Default::default()
        }))
        .unwrap()
        .unwrap();
    debug_wait(&mut debugger, |s| {
        s.sessions[0].state == DebugState::Stopped
            && s.sessions[0]
                .variables
                .values()
                .flatten()
                .any(|v| v.name == "answer" && v.value == "42")
    });
    assert_eq!(debugger.sessions[0].frames[0].path.as_ref(), Some(&source));
    assert_eq!(debugger.sessions[0].frames[0].line, 7);
    debugger
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: "answer".into(),
        })
        .unwrap();
    debug_wait(&mut debugger, |s| !s.sessions[0].evaluations.is_empty());
    assert_eq!(debugger.sessions[0].evaluations[0].result, "42");
    debugger.execute(DebugCommand::Continue(id)).unwrap();
    debug_wait(&mut debugger, |s| !s.has_active_sessions());
    assert_eq!(debugger.sessions[0].exit_code, Some(0));
    assert!(
        debugger.sessions[0]
            .output
            .iter()
            .any(|o| o.text.contains("result=84"))
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), text);
    if let Some(path) = std::env::var_os("CODISSEU_PORTABLE_RUST_TEST_REPORT") {
        fs::write(path,serde_json::to_vec_pretty(&json!({"resources":resources.root(),"sysroot":sysroot,"explicit_toolchain_configuration":true,"cargo_check":true,"cargo_build":true,"cargo_run":run,"cargo_test":tests,"clippy":true,"rustdoc":true,"rustfmt":true,"analyzer_hover":true,"stdlib_definition":true,"quick_fix_preview_apply_undo":true,"real_inlay_type_parameter":true,"rust_rename_preview_apply_undo":true,"unsupported_macro_rename_rejected":true,"rust_native_breakpoint":true,"rust_evaluate":true,"rust_continue_exit0":true,"sources_unchanged":true,"external_network_required":false})).unwrap()).unwrap();
    }
}
