use super::*;
use std::time::{Duration, Instant};
fn id() -> String {
    "a".repeat(64)
}
fn connected() -> DockerService {
    let mut service = DockerService::default();
    service.state.context = Some("fixture".into());
    service.state.connected = true;
    service.state.contexts = vec![DockerContext {
        name: "fixture".into(),
        description: String::new(),
        current: true,
    }];
    service.state.containers = vec![Container {
        id: id(),
        name: "sample".into(),
        state: "running".into(),
        ..Container::default()
    }];
    service.state.selected = Some(id());
    service
}
fn settle(service: &mut DockerService) {
    let start = Instant::now();
    while service.is_busy() {
        service.poll();
        assert!(start.elapsed() < Duration::from_secs(45));
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn parses_structured_inventory_and_rejects_ambiguous_identity() {
    let source = format!(
        r#"{{"ID":"{}","Names":"á sample","Image":"image:1","State":"running","Status":"Up 1 minute","Ports":"8080/tcp","ComposeProject":"my project","ComposeService":"web"}}"#,
        id()
    );
    let containers = parse_containers(source.as_bytes()).unwrap();
    assert_eq!(containers[0].compose_project.as_deref(), Some("my project"));
    assert_eq!(containers[0].name, "á sample");
    assert!(parse_containers(format!("{source}\n{source}").as_bytes()).is_err());
    assert!(parse_containers(b"{\"ID\":\"--force\"}").is_err());
    assert!(parse_containers(b"invalid").is_err());
    assert!(json_lines(b"{}\n{}", 1).is_err());
}
#[test]
fn context_names_and_metadata_are_validated() {
    let contexts =
        parse_contexts(b"{\"Name\":\"fixture\",\"Description\":\"Local\",\"Current\":true}")
            .unwrap();
    assert!(contexts[0].current);
    assert!(parse_contexts(b"{\"Name\":\"-x\"}").is_err());
    let details = format!(
        r#"{{"ID":"{}","Name":"/sample","Image":"i","State":"running","Created":"today","Ports":{{"80/tcp":[{{"HostPort":"8080"}}]}},"Mounts":[{{"Type":"bind","Source":"/source","Destination":"/dest"}}],"Networks":{{"bridge":{{"IPAddress":"172.1.0.1"}}}},"WorkingDir":"/app","User":"1000","RestartPolicy":"no","Env":["SECRET=never surfaced"]}}"#,
        id()
    );
    let parsed = parse_details(details.as_bytes(), &id()).unwrap();
    assert_eq!(parsed.name, "sample");
    assert_eq!(parsed.mounts, ["/source → /dest (bind)"]);
    assert!(parsed.networks[0].contains("bridge"));
    assert!(!format!("{parsed:?}").contains("SECRET"));
    assert!(parse_details(details.as_bytes(), &"b".repeat(64)).is_err());
}
#[test]
fn metadata_overflow_is_reported_instead_of_silently_hiding_entries() {
    let value = serde_json::json!({"ID":id(),"Mounts":vec![serde_json::json!({});257]});
    assert!(
        parse_details(&serde_json::to_vec(&value).unwrap(), &id())
            .unwrap_err()
            .contains("256")
    );
    let mut networks = serde_json::Map::new();
    for n in 0..257 {
        networks.insert(n.to_string(), serde_json::json!({}));
    }
    let value = serde_json::json!({"ID":id(),"Networks":networks});
    assert!(
        parse_details(&serde_json::to_vec(&value).unwrap(), &id())
            .unwrap_err()
            .contains("256")
    );
}
#[test]
fn lifecycle_states_are_explicit_and_remove_never_forces() {
    for state in [
        "running",
        "paused",
        "restarting",
        "removing",
        "dead",
        "created",
        "exited",
        "unknown",
    ] {
        let c = Container {
            state: state.into(),
            ..Container::default()
        };
        assert_eq!(
            ContainerAction::Remove.can_apply(&c),
            matches!(state, "dead" | "created" | "exited")
        );
        assert_eq!(ContainerAction::Unpause.can_apply(&c), state == "paused");
        assert_eq!(ContainerAction::Pause.can_apply(&c), state == "running");
        assert_eq!(
            ContainerAction::StopAndRemove.can_apply(&c),
            matches!(
                state,
                "dead" | "created" | "exited" | "running" | "restarting" | "paused"
            )
        );
    }
    assert_eq!(ContainerAction::Remove.argument(), "rm");
}
#[test]
fn rejects_unlisted_ids_contexts_shells_and_stale_selection() {
    let mut service = connected();
    assert!(
        service
            .dispatch(DockerCommand::SelectContext("production".into()))
            .is_err()
    );
    assert!(
        service
            .dispatch(DockerCommand::Select("b".repeat(64)))
            .is_err()
    );
    assert!(
        service
            .dispatch(DockerCommand::Action {
                id: "b".repeat(64),
                action: ContainerAction::Stop
            })
            .is_err()
    );
    assert!(service.exec_spec(&id(), "sh -c evil").is_err());
    let spec = service.exec_spec(&id(), "bash").unwrap();
    assert_eq!(
        spec.args,
        vec!["--context", "fixture", "exec", "-it", &id(), "bash"]
    );
    assert_eq!(spec.context, "fixture");
    service.state.selected = None;
    assert!(service.shell_args(&id(), "sh").is_err());
}
#[test]
fn log_text_discards_osc_csi_clipboard_and_controls() {
    assert_eq!(
        inert("hello\u{1b}[31mred\u{1b}[0m\u{1b}]52;c;secret\u{7}\0\nOlá"),
        "hellored\nOlá"
    );
    assert_eq!(inert("\u{1b}]title\u{1b}\\ok"), "ok");
}
#[test]
fn missing_cli_is_reported_without_blocking_construction() {
    let mut service = DockerService::default();
    assert!(!service.state.connected);
    assert!(!service.is_busy());
    service
        .dispatch(DockerCommand::Configure {
            program: PathBuf::from("/codisseu/does-not-exist/docker"),
        })
        .unwrap();
    service.dispatch(DockerCommand::Refresh).unwrap();
    assert!(service.is_busy());
    assert!(service.dispatch(DockerCommand::Refresh).is_err());
    settle(&mut service);
    assert!(
        service
            .state
            .error
            .as_deref()
            .unwrap()
            .contains("Não foi possível executar Docker")
    );
    assert!(!service.state.connected);
}
#[cfg(unix)]
fn script(code: &str) -> (tempfile::TempDir, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("docker fixture");
    std::fs::write(&path, format!("#!/usr/bin/python3\n{code}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    (dir, path)
}
#[cfg(unix)]
#[test]
fn real_cli_double_proves_refresh_details_logs_action_and_context_capture() {
    let code = r#"import sys,json
args=sys.argv[1:]
ID='a'*64
if args[:2]==['context','ls']:
 print(json.dumps({'Name':'fixture','Current':True,'Description':'local'}))
elif args[:2]==['--context','fixture']:
 args=args[2:]
 if args[:2]==['container','ls']: print(json.dumps({'ID':ID,'Names':'fixture','Image':'fixture:1','State':'running','Status':'Up','Ports':''}))
 elif args[:2]==['container','inspect']:
  assert 'Env' not in args[3]
  print(json.dumps({'ID':ID,'Name':'/fixture','State':'running','Image':'fixture:1'}))
 elif args[:2]==['container','logs']:
  assert args[2:5]==['--tail','500','--timestamps']
  print('out');print('err',file=sys.stderr)
 elif args[:2]==['container','stop']: assert args[2:]==['--time','10',ID];print(ID)
 else: raise Exception(args)
else: raise Exception(args)
"#;
    let (_dir, path) = script(code);
    let mut service = DockerService::default();
    service
        .dispatch(DockerCommand::Configure { program: path })
        .unwrap();
    service.dispatch(DockerCommand::Refresh).unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    assert_eq!(service.state.context.as_deref(), Some("fixture"));
    assert_eq!(service.state.containers.len(), 1);
    service.dispatch(DockerCommand::Select(id())).unwrap();
    settle(&mut service);
    assert_eq!(service.state.details.as_ref().unwrap().id, id());
    service.dispatch(DockerCommand::Logs { id: id() }).unwrap();
    settle(&mut service);
    assert!(service.state.logs.contains("out"));
    assert!(service.state.logs.contains("err"));
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::Stop,
        })
        .unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    assert!(
        service
            .state
            .notice
            .as_deref()
            .unwrap()
            .contains("concluída")
    );
}
#[cfg(unix)]
#[test]
fn cancellation_reconciles_mutation_without_claiming_rollback() {
    let (_dir, path) = script("import time\ntime.sleep(30)");
    let mut service = connected();
    service.program = path;
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::Stop,
        })
        .unwrap();
    service.cancel();
    assert!(service.dispatch(DockerCommand::Refresh).is_err());
    settle(&mut service);
    assert!(!service.state.connected);
    assert!(
        service
            .state
            .notice
            .as_deref()
            .unwrap()
            .contains("não desfaz")
    );
    assert!(service.exec_spec(&id(), "sh").is_err());
}
#[cfg(unix)]
#[test]
fn external_state_change_prevents_destructive_action() {
    let code = r#"import json,sys
assert sys.argv[3:5]==['container','inspect'],sys.argv
print(json.dumps({'ID':'a'*64,'State':'running'}))"#;
    let (_dir, path) = script(code);
    let mut service = connected();
    service.program = path;
    service.state.containers[0].state = "exited".into();
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::Remove,
        })
        .unwrap();
    settle(&mut service);
    assert!(
        service
            .state
            .error
            .as_deref()
            .unwrap()
            .contains("estado do container mudou")
    );
    assert!(!service.state.connected);
}

#[cfg(unix)]
fn removal_fixture(state: &str, mode: &str) -> (tempfile::TempDir, DockerService) {
    let (dir, program) = script(
        r#"import json,sys,pathlib,time
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
assert args[:2]==['--context','fixture'],args
with (root/'calls.jsonl').open('a') as f:f.write(json.dumps(args)+'\n')
args=args[2:]
state=(root/'state').read_text();mode=(root/'mode').read_text()
assert args[0]=='container',args
action=args[1]
if action=='inspect':
 assert args[-1]=='a'*64,args
 countfile=root/'inspections';count=int(countfile.read_text())+1 if countfile.exists() else 1;countfile.write_text(str(count))
 if mode=='resurrect' and count>1:state='running'
 print(json.dumps({'ID':'a'*64,'Name':'/sample','Image':'own:image','State':state}))
elif action in ['unpause','stop','rm']:
 assert args[-1]=='a'*64,args
 if action=='unpause':
  assert state=='paused';(root/'state').write_text('running')
 elif action=='stop':
  assert args==['container','stop','--timeout','10','a'*64],args
  if mode=='stop-fails':print('owned stop failed',file=sys.stderr);sys.exit(7)
  (root/'state').write_text('exited');(root/'stopped').touch()
  while (root/'gate').exists():time.sleep(.01)
 else:
  assert args==['container','rm','a'*64],args
  assert state in ['exited','created','dead'],state
  if mode=='rm-fails':print('owned removal failed',file=sys.stderr);sys.exit(8)
  (root/'removed').touch()
 print('a'*64)
elif action=='ls':
 if not (root/'removed').exists():print(json.dumps({'ID':'a'*64,'Names':'sample','Image':'own:image','State':state}))
 print(json.dumps({'ID':'b'*64,'Names':'survivor','Image':'own:image','State':'running'}))
else:raise AssertionError(args)
"#,
    );
    std::fs::write(dir.path().join("state"), state).unwrap();
    std::fs::write(dir.path().join("mode"), mode).unwrap();
    let mut service = connected();
    service.program = program;
    service.state.containers[0].state = state.into();
    service.state.details = Some(ContainerDetails {
        id: id(),
        ..Default::default()
    });
    service.state.logs = "old container logs".into();
    (dir, service)
}

#[cfg(unix)]
fn removal_calls(dir: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(dir.join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[cfg(unix)]
#[test]
fn explicit_stop_and_remove_revalidates_each_stage_without_force_or_volume_flags() {
    for state in [
        "running",
        "paused",
        "restarting",
        "exited",
        "created",
        "dead",
    ] {
        let (dir, mut service) = removal_fixture(state, "success");
        service
            .dispatch(DockerCommand::Action {
                id: id(),
                action: ContainerAction::StopAndRemove,
            })
            .unwrap();
        settle(&mut service);
        assert!(
            service.state.error.is_none(),
            "{state}: {:?}",
            service.state.error
        );
        assert!(service.state.connected);
        assert_eq!(
            service.state.notice.as_deref(),
            Some("Container “sample” excluído.")
        );
        assert!(
            service.state.selected.is_none()
                && service.state.details.is_none()
                && service.state.logs.is_empty()
        );
        assert_eq!(service.state.containers.len(), 1);
        assert_eq!(service.state.containers[0].name, "survivor");
        let calls = removal_calls(dir.path());
        let actual: Vec<_> = calls.iter().map(|args| args[3].as_str()).collect();
        let expected = match state {
            "paused" => vec!["inspect", "unpause", "stop", "inspect", "rm", "ls"],
            "running" | "restarting" => vec!["inspect", "stop", "inspect", "rm", "ls"],
            _ => vec!["inspect", "inspect", "rm", "ls"],
        };
        assert_eq!(actual, expected, "{state}");
        assert!(
            calls
                .iter()
                .all(|args| args[..2] == ["--context", "fixture"])
        );
        assert!(!calls.iter().flatten().any(|arg| matches!(
            arg.as_str(),
            "--force" | "-f" | "--volumes" | "-v" | "prune"
        )));
        assert!(dir.path().join("removed").exists());
    }
}

#[cfg(unix)]
#[test]
fn stop_failure_or_live_state_after_stop_never_reaches_remove() {
    for mode in ["stop-fails", "resurrect"] {
        let (dir, mut service) = removal_fixture("running", mode);
        service
            .dispatch(DockerCommand::Action {
                id: id(),
                action: ContainerAction::StopAndRemove,
            })
            .unwrap();
        settle(&mut service);
        assert!(service.state.error.is_some());
        assert!(!service.state.connected);
        assert!(
            !service
                .state
                .notice
                .as_deref()
                .unwrap_or_default()
                .contains("excluído")
        );
        assert!(!dir.path().join("removed").exists());
        assert!(!removal_calls(dir.path()).iter().any(|args| args[3] == "rm"));
    }
}

#[cfg(unix)]
#[test]
fn failed_remove_preserves_inventory_and_reports_the_target_without_false_success() {
    let (dir, mut service) = removal_fixture("running", "rm-fails");
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::StopAndRemove,
        })
        .unwrap();
    settle(&mut service);
    let error = service.state.error.as_deref().unwrap();
    assert!(
        error.contains("Não foi possível excluir o container “sample”")
            && error.contains("owned removal failed")
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("state")).unwrap(),
        "exited"
    );
    assert!(service.state.containers.iter().any(|c| c.id == id()));
    assert!(!service.state.connected && !dir.path().join("removed").exists());
    assert!(
        service
            .state
            .notice
            .as_deref()
            .unwrap()
            .contains("Atualize")
    );
}

#[cfg(unix)]
#[test]
fn cancelling_the_combined_action_after_stop_does_not_start_remove() {
    let (dir, mut service) = removal_fixture("running", "success");
    std::fs::write(dir.path().join("gate"), "").unwrap();
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::StopAndRemove,
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("stopped").exists() {
        service.poll();
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    service.cancel();
    settle(&mut service);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("state")).unwrap(),
        "exited"
    );
    assert!(
        service
            .state
            .error
            .as_deref()
            .unwrap()
            .contains("interrompida")
    );
    assert!(!service.state.connected && !dir.path().join("removed").exists());
    assert!(!removal_calls(dir.path()).iter().any(|args| args[3] == "rm"));
}
#[test]
#[ignore = "Requires explicitly provided, disposable Codisseu-owned container and Docker CLI; mutates only that container"]
fn owned_container_real_lifecycle() {
    let owned = std::env::var("CODISSEU_DOCKER_TEST_CONTAINER").expect("provide owned full ID");
    assert!(valid_id(&owned));
    let mut service = DockerService::default();
    service.dispatch(DockerCommand::Refresh).unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    let container = service
        .state
        .containers
        .iter()
        .find(|c| c.id == owned)
        .expect("owned container listed");
    assert!(container.name.starts_with("codisseu-docker-test-"));
    service
        .dispatch(DockerCommand::Select(owned.clone()))
        .unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    if service
        .state
        .containers
        .iter()
        .find(|c| c.id == owned)
        .unwrap()
        .state
        == "running"
    {
        service
            .dispatch(DockerCommand::Action {
                id: owned.clone(),
                action: ContainerAction::Stop,
            })
            .unwrap();
        settle(&mut service);
        assert!(service.state.error.is_none(), "{:?}", service.state.error);
    }
    for action in [
        ContainerAction::Start,
        ContainerAction::Pause,
        ContainerAction::Unpause,
        ContainerAction::Restart,
        ContainerAction::Stop,
    ] {
        let c = service
            .state
            .containers
            .iter()
            .find(|c| c.id == owned)
            .unwrap();
        if action == ContainerAction::Start && c.state == "running" {
            continue;
        }
        service
            .dispatch(DockerCommand::Action {
                id: owned.clone(),
                action,
            })
            .unwrap();
        settle(&mut service);
        assert!(service.state.error.is_none(), "{:?}", service.state.error);
    }
    service.dispatch(DockerCommand::Logs { id: owned }).unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    // Cleanup/removal belongs to the fixture owner, even if this assertion fails.
}
#[cfg(unix)]
#[test]
fn cancelled_write_can_complete_in_daemon_and_refresh_reconciles_captured_context() {
    let fixture = tempfile::tempdir().unwrap();
    let marker = fixture.path().join("daemon_state");
    let header = format!(
        "STATE={}\n",
        serde_json::to_string(marker.to_str().unwrap()).unwrap()
    );
    let code = r#"import sys,json,time,os
args=sys.argv[1:]
if args[:2]==['context','ls']:
 for name in ['fixture','other']:print(json.dumps({'Name':name,'Current':name=='fixture'}))
else:
 assert args[0]=='--context'
 context=args[1];args=args[2:]
 ID=('a' if context=='fixture' else 'b')*64
 state=open(STATE).read() if os.path.exists(STATE) and context=='fixture' else 'running'
 if args[:2]==['container','ls']:print(json.dumps({'ID':ID,'Names':'sample','State':state}))
 elif args[:2]==['container','inspect']:assert args[-1]==ID;print(json.dumps({'ID':ID,'State':state}))
 elif args[:2]==['container','stop']:
  assert context=='fixture' and args[-1]==ID
  open(STATE,'w').write('exited')
  time.sleep(30)
 else:raise Exception(args)
"#;
    let (_dir, path) = script(&(header + code));
    let mut service = connected();
    service.program = path;
    service.state.contexts.push(DockerContext {
        name: "other".into(),
        ..DockerContext::default()
    });
    service
        .dispatch(DockerCommand::Action {
            id: id(),
            action: ContainerAction::Stop,
        })
        .unwrap();
    let began = Instant::now();
    while !marker.exists() {
        assert!(began.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        service
            .dispatch(DockerCommand::SelectContext("other".into()))
            .is_err()
    );
    service.cancel();
    settle(&mut service);
    assert_eq!(std::fs::read_to_string(&marker).unwrap(), "exited");
    assert!(!service.state.connected);
    service.dispatch(DockerCommand::Refresh).unwrap();
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    assert!(service.state.connected);
    assert_eq!(service.state.containers[0].state, "exited");
    assert_eq!(service.state.context.as_deref(), Some("fixture"));
    service
        .dispatch(DockerCommand::SelectContext("other".into()))
        .unwrap();
    assert!(service.state.selected.is_none());
    assert!(service.state.containers.is_empty());
    settle(&mut service);
    assert!(service.state.error.is_none(), "{:?}", service.state.error);
    assert_eq!(service.state.context.as_deref(), Some("other"));
    assert_eq!(service.state.containers[0].id, "b".repeat(64));
    assert!(
        service
            .dispatch(DockerCommand::Action {
                id: id(),
                action: ContainerAction::Stop
            })
            .is_err()
    );
}

#[test]
fn shutdown_is_nonblocking_and_keeps_busy_until_worker_acknowledges() {
    let (release, gate) = std::sync::mpsc::channel();
    let (returned, observe_return) = std::sync::mpsc::channel();
    let observer = std::thread::spawn(move || {
        // Release even on regression so an accidental join produces an assertion
        // failure instead of leaving the entire test process hung forever.
        let shutdown_returned = observe_return.recv_timeout(Duration::from_secs(1)).is_ok();
        release.send(()).unwrap();
        shutdown_returned
    });
    let mut service = connected();
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = std::thread::spawn(move || {
        gate.recv().unwrap();
        Ok(DockerState::default())
    });
    service.active = Some(Active {
        background: false,
        cancel: cancel.clone(),
        handle,
        mutation: false,
    });
    service.state.pending = Some("Fixture aguardando ACK".into());
    service.shutdown();
    assert!(cancel.load(Ordering::SeqCst));
    assert!(service.is_busy());
    assert_eq!(service.state.pending.as_deref(), Some("Cancelando"));
    returned.send(()).unwrap();
    assert!(
        observer.join().unwrap(),
        "shutdown bloqueou esperando o worker"
    );
    settle(&mut service);
    assert!(!service.is_busy());
    assert!(service.state.pending.is_none());
    service.shutdown();
    assert!(!service.is_busy());
}

#[cfg(unix)]
fn gated_refresh() -> (tempfile::TempDir, DockerService) {
    let (dir, program) = script(
        r#"import sys,json,pathlib,time
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
if args[:2]==['context','ls']:
 (root/'entered').touch()
 while (root/'gate').exists(): time.sleep(.01)
 if (root/'fail').exists(): sys.exit('own refresh failure')
 print(json.dumps({'Name':'fixture','Current':True}))
elif args[2:4]==['container','ls']:
 print(json.dumps({'ID':'a'*64,'Names':'updated','State':'running'}))
elif args[2:4]==['container','inspect']:
 print(json.dumps({'ID':'a'*64,'Name':'/updated','State':'running'}))
elif args[2:4]==['container','logs']: print('requested logs')
else: raise Exception(args)
"#,
    );
    std::fs::write(dir.path().join("gate"), "").unwrap();
    let mut service = connected();
    service.program = program;
    service.state.logs = "retained logs".into();
    service.state.notice = Some("retained notice".into());
    (dir, service)
}
#[cfg(unix)]
fn wait_for_read(dir: &Path) {
    let start = Instant::now();
    while !dir.join("entered").exists() {
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(unix)]
#[test]
fn automatic_refresh_keeps_complete_view_without_foreground_progress() {
    let (dir, mut service) = gated_refresh();
    service.refresh_in_background().unwrap();
    wait_for_read(dir.path());
    assert!(service.is_busy());
    assert!(service.is_background_refresh());
    assert!(!service.is_foreground_busy());
    assert_eq!(service.state.pending, None);
    assert_eq!(service.state.containers[0].name, "sample");
    assert_eq!(service.state.logs, "retained logs");
    assert_eq!(service.state.notice.as_deref(), Some("retained notice"));
    assert!(service.refresh_in_background().is_err());
    std::fs::remove_file(dir.path().join("gate")).unwrap();
    settle(&mut service);
    assert_eq!(service.state.containers[0].name, "updated");
    assert_eq!(service.state.logs, "retained logs");
    assert_eq!(service.state.notice.as_deref(), Some("retained notice"));
    assert_eq!(service.state.pending, None);
}
#[cfg(unix)]
#[test]
fn user_command_preempts_read_and_runs_only_after_its_ack() {
    let (dir, mut service) = gated_refresh();
    service.refresh_in_background().unwrap();
    wait_for_read(dir.path());
    service.dispatch(DockerCommand::Logs { id: id() }).unwrap();
    assert!(service.is_foreground_busy());
    assert!(service.dispatch(DockerCommand::Refresh).is_err());
    settle(&mut service); // gate remains closed: the cancelled read cannot win.
    assert_eq!(service.state.containers[0].name, "sample");
    assert_eq!(service.state.logs, "requested logs\n");
    assert_eq!(service.state.error, None);
    assert!(service.deferred.is_none());
    assert!(dir.path().join("gate").exists());
}
#[cfg(unix)]
#[test]
fn shutdown_discards_queued_intent_and_background_failures_keep_inventory() {
    let (dir, mut service) = gated_refresh();
    service.refresh_in_background().unwrap();
    wait_for_read(dir.path());
    service.dispatch(DockerCommand::Logs { id: id() }).unwrap();
    service.shutdown();
    settle(&mut service);
    assert_eq!(service.state.logs, "retained logs");
    assert_eq!(service.state.pending, None);
    assert_eq!(service.state.error, None);
    std::fs::write(dir.path().join("fail"), "").unwrap();
    std::fs::remove_file(dir.path().join("gate")).unwrap();
    service.refresh_in_background().unwrap();
    settle(&mut service);
    assert!(
        service
            .state
            .error
            .as_ref()
            .unwrap()
            .contains("own refresh failure")
    );
    assert_eq!(service.state.containers[0].name, "sample");
    assert_eq!(service.state.logs, "retained logs");
}
