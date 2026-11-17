//! Explicitly run against a relocated installation; no project-supplied tools.
use codisseu_core::{AppService, Command, ProviderState, Selection};
use codisseu_debug::{
    Breakpoint, DebugCommand, DebugConfiguration, DebugRequest, DebugService, DebugState,
};
use std::{
    fs,
    path::Path,
    process::Command as ProcessCommand,
    thread,
    time::{Duration, Instant},
};
fn debug_roundtrip(
    service: &mut DebugService,
    program: &Path,
    source: &Path,
    root: &Path,
    line: u32,
    expression: &str,
) {
    let mut point = Breakpoint::new(source.into(), line);
    point.options.condition = format!("{expression} == 42");
    point.options.hit_condition = "1".into();
    service.execute(DebugCommand::SetBreakpoint(point)).unwrap();
    let mut logpoint = Breakpoint::new(source.into(), line - 1);
    logpoint.options.log_message = "Codisseu portable logpoint".into();
    service
        .execute(DebugCommand::SetBreakpoint(logpoint))
        .unwrap();
    let mut config = DebugConfiguration::for_file(program, root);
    // Keep the literal default: the worker must resolve the relocated private runtime.
    assert_eq!(
        config.executable,
        Path::new(if program.extension().is_some_and(|x| x == "php") {
            "php"
        } else {
            "node"
        })
    );
    config.trusted = true;
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    debug_wait(service, |s| {
        s.sessions.iter().any(|x| {
            x.id == id
                && x.state == DebugState::Stopped
                && x.frames
                    .first()
                    .is_some_and(|f| f.path.as_deref() == Some(source) && f.line == line)
                && x.breakpoints
                    .iter()
                    .any(|b| b.path == source && b.line == line && b.verified)
        })
    });
    assert!(
        service
            .sessions
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .output
            .iter()
            .any(|o| o.text.contains("Codisseu portable logpoint")),
        "Private adapter must log without stopping before the conditional point"
    );
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: expression.into(),
        })
        .unwrap();
    debug_wait(service, |s| {
        s.sessions.iter().any(|x| {
            x.id == id
                && x.evaluations
                    .iter()
                    .any(|e| e.expression == expression && e.error.is_none() && e.result == "42")
        })
    });
    service.execute(DebugCommand::Stop(id)).unwrap();
    debug_wait(service, |s| {
        s.sessions
            .iter()
            .any(|x| x.id == id && x.state == DebugState::Terminated && x.pending == 0)
    });
}
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let until = Instant::now() + Duration::from_secs(75);
    loop {
        app.poll();
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < until,
            "{} {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn debug_wait(service: &mut DebugService, check: impl Fn(&DebugService) -> bool) {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        service.poll();
        if check(service) {
            return;
        }
        assert!(
            service
                .sessions
                .iter()
                .all(|s| s.state != DebugState::Failed),
            "{:?}",
            service.sessions
        );
        assert!(Instant::now() < until, "{:?}", service.sessions);
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "relocated bundle and CODISSEU_RESOURCE_DIR required; private PHP/Node/TS/DAP real corpus"]
fn relocated_private_engines_autostart_diagnostics_scripts_and_debug() {
    let resources = codisseu_runtime::discover().unwrap().unwrap();
    for provider in resources.providers().unwrap() {
        assert!(provider.program.starts_with(resources.root()));
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project with spaces");
    fs::create_dir(&root).unwrap();
    let php = root.join("weather.php");
    let ts = root.join("weather.ts");
    fs::write(&php, "<?php echo '🌧'; $broken = ;\n").unwrap();
    fs::write(
        &ts,
        "export const emoji = '🌧'; export const count: number = 'wrong';\n",
    )
    .unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022","noEmit":true},"include":["*.ts"]}"#,
    )
    .unwrap();
    let mut app = AppService::new(temp.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for path in [&php, &ts] {
        app.dispatch(Command::OpenFile(path.into())).unwrap();
    }
    wait(&mut app, |a| !a.is_busy());
    let policy = temp.path().join("profile/providers.json");
    let commands = resources.providers().unwrap();
    assert_eq!(
        commands.iter().filter(|c| c.language == "Rust").count(),
        usize::from(resources.rust_sysroot().unwrap().is_some())
    );
    assert_eq!(
        commands.iter().filter(|c| c.language == "Python").count(),
        usize::from(!resources.python_library_roots().unwrap().is_empty())
    );
    let php_extra = commands.iter().find(|p| p.language == "Php").unwrap().args[3].clone();
    let legacy = serde_json::json!([
        {"language":"Php","program":"/usr/bin/php","args":["-d","xdebug.mode=off","/missing-old-notebook/.tools/phpactor-2026.10.07.0/bin/phpactor",php_extra,"language-server"]},
        {"language":"TypeScript","program":"/missing-old-notebook/.tools/node-v24.22.0-linux-x64/bin/node","args":["/missing-old-notebook/.tools/node_modules/typescript-language-server/lib/cli.mjs","--stdio"]}
    ]);
    fs::create_dir_all(policy.parent().unwrap()).unwrap();
    fs::write(&policy,serde_json::to_vec(&serde_json::json!({"schema_version":1,"automatic":true,"configs":legacy,"approvals":[{"workspace":root,"configs":legacy}]})).unwrap()).unwrap();
    app.dispatch(Command::EnableProviderAutostart {
        profile: policy.clone(),
        fallback_config: None,
    })
    .unwrap();
    wait(&mut app, |a| {
        !a.is_busy() && a.provider_startup().needs_trust
    });
    assert!(app.language.statuses.is_empty());
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        a.language.statuses.len() == commands.len()
            && a.language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && a.language
                .diagnostics
                .iter()
                .any(|d| d.path == php && d.severity == 1)
            && a.language
                .diagnostics
                .iter()
                .any(|d| d.path == ts && d.severity == 1)
    });
    let scripts=codisseu_api::run_script("pm.environment.set('token', 'portable'); pm.test('private Node works',()=>pm.expect(2+2).to.eql(4));",&Default::default(),None).unwrap();
    assert_eq!(scripts.tests.len(), 1);
    assert!(scripts.tests[0].passed);
    let file = root.join("debug.js");
    fs::write(
        &file,
        "const answer = 42;\nconsole.log(answer);\nsetInterval(() => {}, 1000);\n",
    )
    .unwrap();
    let mut debug = DebugService::new();
    debug
        .execute(DebugCommand::ToggleBreakpoint {
            path: file.clone(),
            line: 2,
        })
        .unwrap();
    let mut config = DebugConfiguration::for_file(&file, &root);
    config.trusted = true;
    let id = debug.execute(DebugCommand::Start(config)).unwrap().unwrap();
    debug_wait(&mut debug, |s| {
        s.sessions
            .iter()
            .any(|x| x.id == id && x.state == DebugState::Stopped)
    });
    debug.execute(DebugCommand::Stop(id)).unwrap();
    debug_wait(&mut debug, |s| !s.has_active_sessions());
    // Use a separate valid source; weather.php must retain its diagnostic fixture.
    let php_debug = root.join("runtime-debug.php");
    fs::write(
        &php_debug,
        "<?php\n$answer = 42;\necho $answer;\nwhile (true) { usleep(100000); }\n",
    )
    .unwrap();
    debug_roundtrip(&mut debug, &php_debug, &php_debug, &root, 3, "$answer");
    let ts_debug = root.join("runtime-debug.ts");
    fs::write(
        &ts_debug,
        "const answer: number = 42;\nconsole.log(answer);\nsetInterval(() => {}, 1000);\n",
    )
    .unwrap();
    let node = resources.tool("node").unwrap();
    let compiler = resources
        .root()
        .join("runtime/node_modules/typescript/bin/tsc")
        .canonicalize()
        .unwrap();
    assert!(node.starts_with(resources.root()) && compiler.starts_with(resources.root()));
    let compilation = ProcessCommand::new(&node)
        .arg(&compiler)
        .args([
            "--ignoreConfig",
            "--sourceMap",
            "--inlineSources",
            "--target",
            "ES2022",
            "--module",
            "commonjs",
            "--skipLibCheck",
            "--noEmitOnError",
        ])
        .arg(&ts_debug)
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        compilation.status.success(),
        "private TypeScript compilation failed: {} {}",
        String::from_utf8_lossy(&compilation.stdout),
        String::from_utf8_lossy(&compilation.stderr)
    );
    let ts_program = ts_debug.with_extension("js");
    assert!(ts_program.is_file() && ts_program.with_extension("js.map").is_file());
    debug_roundtrip(&mut debug, &ts_program, &ts_debug, &root, 2, "answer");
    let mut config = DebugConfiguration::for_file(&php, &root);
    config.request = DebugRequest::Listen;
    config.program = None;
    config.trusted = true;
    let id = debug.execute(DebugCommand::Start(config)).unwrap().unwrap();
    debug_wait(&mut debug, |s| {
        s.sessions
            .iter()
            .any(|x| x.id == id && x.state == DebugState::Running && x.pending == 0)
    });
    debug.execute(DebugCommand::Stop(id)).unwrap();
    debug_wait(&mut debug, |s| !s.has_active_sessions());
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(!app.providers_running());
    app.dispatch(Command::PersistSession).unwrap();
    wait(&mut app, |a| !a.is_busy());
    drop(app);
    let mut app = AppService::new(temp.path().join("profile/session.json"));
    app.dispatch(Command::RestoreSession).unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::EnableProviderAutostart {
        profile: policy,
        fallback_config: None,
    })
    .unwrap();
    wait(&mut app, |a| {
        a.provider_startup().trusted
            && a.language.statuses.len() == commands.len()
            && a.language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
    });
    // All four formatters must work using only the relocated installation.
    // No host interpreter, project tools or network may be needed.
    let (_, fixer, helper) = codisseu_runtime::php_formatter().unwrap();
    assert!(fixer.starts_with(resources.root()) && helper.starts_with(resources.root()));
    let mut formatting = Vec::new();
    for (name, source, formatted_fragment) in [
        (
            "format.php",
            "<?php\r\n// ação 😀\r\nclass Greeting{function greet(string $name):string{return 'hello  '.$name;}}\r\n",
            "    function greet",
        ),
        (
            "format.sql",
            "-- ação 😀\r\nselect id,name from users where active=1;\r\n",
            "from\r\n",
        ),
        (
            "format.js",
            "// ação 😀\r\nexport function greet(name){\r\nreturn `hello  ${name}`;\r\n}\r\nexport const sum=(a,b)=>a+b;\r\n",
            "a + b",
        ),
        (
            "format.ts",
            "// ação 😀\r\nexport function greet(name:string):string{\r\nreturn `hello  ${name}`;\r\n}\r\nexport const sum=(a:number,b:number):number=>a+b;\r\n",
            "a + b",
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, source).unwrap();
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        wait(&mut app, |a| !a.is_busy());
        let doc = app.active_document().unwrap();
        let id = doc.id;
        let dirty = source
            .replace("hello  ", "dirty  ")
            .replace("active=1", "active=2");
        app.dispatch(Command::Edit {
            id,
            expected_version: doc.version,
            text: dirty.clone(),
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        let before = Selection { anchor: 1, head: 0 };
        let version = app.active_document().unwrap().version;
        app.dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: before,
        })
        .unwrap();
        app.dispatch(Command::FormatDocument {
            id,
            expected_version: version,
            range: None,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        let formatted = app.active_document().unwrap().text().to_owned();
        assert_ne!(formatted, dirty, "{name}");
        assert!(
            formatted.contains(formatted_fragment),
            "{name}: {formatted}"
        );
        assert!(formatted.contains("ação 😀"));
        assert!(!formatted.replace("\r\n", "").contains('\n'));
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), dirty);
        assert_eq!(app.active_document().unwrap().selection(), before);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), formatted);
        formatting.push(name);
    }
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert_eq!(
        fs::read_to_string(&php).unwrap(),
        "<?php echo '🌧'; $broken = ;\n"
    );
    assert!(app.remote_db.active.is_none());
    if let Ok(path) = std::env::var("CODISSEU_PORTABLE_TEST_REPORT") {
        fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"private_php":true,"private_node":true,"php_diagnostics":true,"ts_diagnostics":true,"trust_required":true,"legacy_paths_rebased_without_reusing_approval":true,"reopened_autostart":true,"api_scripts":true,"node_debug":true,"php_adapter_listen":true,"php_launch_default_private_runtime":true,"php_launch_verified_breakpoint":true,"php_launch_evaluate":true,"php_launch_stop":true,"conditional_logpoint_hit_count":true,"typescript_private_compiler":true,"typescript_source_map_breakpoint":true,"typescript_evaluate":true,"typescript_stop":true,"providers_stopped":true,"database_connections":0,"offline_formatters":formatting,"formatter_paths_confined":true})).unwrap()).unwrap();
    }
}
