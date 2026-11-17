//! Real installed js-debug/Node coverage for live mute and exception policies.
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
#[test]
fn live_muted_breakpoint_changes_never_reactivate_target_breakpoints() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("interval.js");
    fs::write(
        &path,
        "let count=0;\nsetInterval(()=>{\n count+=1;\n},20);\n",
    )
    .unwrap();
    let mut service = DebugService::new();
    service
        .execute(DebugCommand::SetBreakpointsEnabled(false))
        .unwrap();
    let mut config = DebugConfiguration::for_file(&path, temp.path());
    config.trusted = true;
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    wait(&mut service, |s| {
        s.state == DebugState::Running && s.pending == 0 && !s.threads.is_empty()
    });
    service
        .execute(DebugCommand::ToggleBreakpoint {
            path: path.clone(),
            line: 3,
        })
        .unwrap();
    let end = Instant::now() + Duration::from_millis(300);
    while Instant::now() < end {
        service.poll();
        assert_eq!(service.sessions[0].state, DebugState::Running);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(service.breakpoints.iter().all(|b| b.enabled));
    assert!(!service.breakpoints_enabled());
    service
        .execute(DebugCommand::SetBreakpointsEnabled(true))
        .unwrap();
    wait(&mut service, |s| {
        s.state == DebugState::Stopped && s.frames.first().is_some_and(|f| f.line == 3)
    });
    wait(&mut service, |s| {
        s.breakpoints
            .iter()
            .any(|b| b.path == path && b.line == 3 && b.verified)
    });
    service.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut service, |s| !s.state.is_active());
}
#[test]
fn real_node_caught_exception_filter_ack_pause_and_disable() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("exceptions.js");
    fs::write(&path,"let counter=0;\nsetInterval(()=>{\n counter++;\n try { throw new Error('caught fixture'); } catch(error) { }\n},20);\n").unwrap();
    let mut service = DebugService::new();
    let mut config = DebugConfiguration::for_file(&path, temp.path());
    config.trusted = true;
    let id = service
        .execute(DebugCommand::Start(config))
        .unwrap()
        .unwrap();
    wait(&mut service, |s| {
        s.state == DebugState::Running && s.pending == 0 && !s.threads.is_empty()
    });
    assert!(
        service.sessions[0].capabilities["exceptionBreakpointFilters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["filter"] == "all")
    );
    service
        .execute(DebugCommand::SetExceptionBreakpoints {
            session: id,
            filters: vec!["all".into()],
        })
        .unwrap();
    wait(&mut service, |s| {
        s.state == DebugState::Stopped
            && s.stopped_reason.as_deref() == Some("exception")
            && !s.frames.is_empty()
    });
    wait(&mut service, |s| s.exception_filters == ["all"]);
    assert_eq!(service.sessions[0].frames[0].path.as_ref(), Some(&path));
    assert!(service.sessions[0].current_thread.is_some());
    service
        .execute(DebugCommand::Evaluate {
            session: id,
            expression: "counter".into(),
        })
        .unwrap();
    wait(&mut service, |s| {
        s.evaluations
            .iter()
            .any(|v| v.expression == "counter" && v.error.is_none())
    });
    assert!(
        service
            .execute(DebugCommand::SetExceptionBreakpoints {
                session: id,
                filters: vec!["unsupported-filter".into()]
            })
            .is_err()
    );
    service
        .execute(DebugCommand::SetExceptionBreakpoints {
            session: id,
            filters: vec![],
        })
        .unwrap();
    wait(&mut service, |s| s.exception_filters.is_empty());
    service.execute(DebugCommand::Continue(id)).unwrap();
    wait(&mut service, |s| {
        s.state == DebugState::Running && s.pending == 0
    });
    assert!(service.sessions[0].evaluations.is_empty());
    let end = Instant::now() + Duration::from_millis(150);
    while Instant::now() < end {
        service.poll();
        assert_eq!(service.sessions[0].state, DebugState::Running);
        thread::sleep(Duration::from_millis(5));
    }
    service.execute(DebugCommand::Stop(id)).unwrap();
    wait(&mut service, |s| !s.state.is_active());
}
