//! Real DAP adapters and disposable PHP/Node programs; no project code is run.
use codisseu_debug::*;
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(service: &mut DebugService, predicate: impl Fn(&DebugSession) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        service.poll();
        let s = &service.sessions[0];
        assert_ne!(s.state, DebugState::Failed, "{s:?}");
        if predicate(s) {
            return;
        }
        assert!(Instant::now() < end, "{s:?}");
        thread::sleep(Duration::from_millis(5));
    }
}
fn fixture(
    extension: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    u32,
    u32,
    &'static str,
) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(format!("ação with spaces.{extension}"));
    let (source, log_line, stop_line, variable) = if extension == "php" {
        (
            "<?php\nfor ($i=0;$i<6;$i++) {\n    $value=$i*2;\n    $seen=$value;\n}\necho 'finished';\n",
            3,
            4,
            "$i",
        )
    } else {
        (
            "for (let i=0;i<6;i++) {\n    const value=i*2;\n    const seen=value;\n}\nconsole.log('finished');\n",
            2,
            3,
            "i",
        )
    };
    let source = if extension == "ts" {
        source.replace("let i=0", "let i: number=0")
    } else {
        source.to_string()
    };
    fs::write(&path, source).unwrap();
    if extension == "ts" {
        let compiler = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.tools/node_modules/typescript/bin/tsc");
        let output = std::process::Command::new("node")
            .arg(compiler)
            .args([
                "--ignoreConfig",
                "--sourceMap",
                "--target",
                "es2020",
                "--outDir",
            ])
            .arg(temp.path().join("compiled"))
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    (temp, path, log_line, stop_line, variable)
}
fn evaluate(service: &mut DebugService, id: SessionId, expression: &str) -> String {
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: expression.into(),
        })
        .unwrap();
    wait(service, |s| {
        s.evaluations.iter().any(|e| e.expression == expression)
    });
    let value = service.sessions[0]
        .evaluations
        .iter()
        .find(|e| e.expression == expression)
        .unwrap();
    assert!(value.error.is_none(), "{value:?}");
    value.result.clone()
}
#[test]
fn real_php_and_node_conditions_live_edits_logpoints_and_individual_disable() {
    for extension in ["php", "js", "ts"] {
        let (temp, path, log_line, stop_line, variable) = fixture(extension);
        let source = fs::read(&path).unwrap();
        let mut service = DebugService::new();
        let mut point = Breakpoint::new(path.clone(), stop_line);
        point.options.condition = format!("{variable} === 3");
        service
            .execute(DebugCommand::SetBreakpoint(point.clone()))
            .unwrap();
        let mut log = Breakpoint::new(path.clone(), log_line);
        log.options.log_message = format!("loop={{{variable}}}");
        service
            .execute(DebugCommand::SetBreakpoint(log.clone()))
            .unwrap();
        assert_eq!(
            service
                .breakpoints
                .iter()
                .find(|b| b.line == stop_line)
                .unwrap()
                .options,
            point.options
        );
        let mut config = DebugConfiguration::for_file(&path, temp.path());
        config.trusted = true;
        if extension == "ts" {
            config.program = Some(
                temp.path()
                    .join("compiled")
                    .join(path.file_name().unwrap())
                    .with_extension("js"),
            );
        }
        let id = service
            .execute(DebugCommand::Start(config))
            .unwrap()
            .unwrap();
        wait(&mut service, |s| {
            s.state == DebugState::Stopped && s.pending == 0 && !s.variables.is_empty()
        });
        assert_eq!(service.sessions[0].frames[0].line, stop_line);
        assert_eq!(evaluate(&mut service, id, variable), "3");
        let output = service.sessions[0]
            .output
            .iter()
            .map(|o| o.text.as_str())
            .collect::<String>();
        for i in 0..=3 {
            assert!(
                output.contains(&format!("loop={i}")),
                "{extension}: {output}"
            );
        }
        point.options.condition = format!("{variable} === 5");
        service
            .execute(DebugCommand::SetBreakpoint(point.clone()))
            .unwrap();
        wait(&mut service, |s| {
            s.pending == 0
                && s.breakpoints
                    .iter()
                    .any(|b| b.line == stop_line && b.options == point.options && b.verified)
        });
        let previous_stop = service.sessions[0].stop_epoch;
        service.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut service, |s| {
            s.stop_epoch > previous_stop
                && s.state == DebugState::Stopped
                && s.pending == 0
                && !s.variables.is_empty()
        });
        assert_eq!(evaluate(&mut service, id, variable), "5");
        point.enabled = false;
        service
            .execute(DebugCommand::SetBreakpoint(point.clone()))
            .unwrap();
        service
            .execute(DebugCommand::SetBreakpointsEnabled(false))
            .unwrap();
        service
            .execute(DebugCommand::SetBreakpointsEnabled(true))
            .unwrap();
        assert!(
            !service
                .breakpoints
                .iter()
                .find(|b| b.line == stop_line)
                .unwrap()
                .enabled
        );
        service.execute(DebugCommand::Continue(id)).unwrap();
        wait(&mut service, |s| !s.state.is_active());
        let output = service.sessions[0]
            .output
            .iter()
            .map(|o| o.text.as_str())
            .collect::<String>();
        assert!(
            output.contains("loop=5") && output.contains("finished"),
            "{extension}: {output}"
        );
        assert_eq!(fs::read(&path).unwrap(), source);
    }
}
#[test]
fn real_php_and_node_hit_count_ignores_earlier_iterations() {
    for extension in ["php", "js", "ts"] {
        let (temp, path, _, line, variable) = fixture(extension);
        let mut service = DebugService::new();
        let mut point = Breakpoint::new(path.clone(), line);
        point.options.hit_condition = "3".into();
        service.execute(DebugCommand::SetBreakpoint(point)).unwrap();
        let mut config = DebugConfiguration::for_file(&path, temp.path());
        config.trusted = true;
        if extension == "ts" {
            config.program = Some(
                temp.path()
                    .join("compiled")
                    .join(path.file_name().unwrap())
                    .with_extension("js"),
            );
        }
        let id = service
            .execute(DebugCommand::Start(config))
            .unwrap()
            .unwrap();
        wait(&mut service, |s| {
            s.state == DebugState::Stopped && s.pending == 0 && !s.variables.is_empty()
        });
        assert_eq!(evaluate(&mut service, id, variable), "2");
        service.execute(DebugCommand::Stop(id)).unwrap();
        wait(&mut service, |s| !s.state.is_active());
    }
}
