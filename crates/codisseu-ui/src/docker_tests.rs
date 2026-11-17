//! Controlled CLI fixture, never the user's Docker daemon.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

type DockerHarness = Harness<'static, CodisseuApp>;

#[cfg(unix)]
fn fixture_app() -> (tempfile::TempDir, CodisseuApp) {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let cli = dir.path().join("fixture-docker");
    fs::write(&cli, r##"#!/usr/bin/python3
import json,sys,pathlib,time
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
with (root/'calls.jsonl').open('a') as f: f.write(json.dumps(args)+'\n')
if args[:2]==['context','ls']:
 (root/'entered').touch()
 while (root/'gate').exists(): time.sleep(.01)
 print(json.dumps({'Name':'fixture','Current':True,'Description':'Isolated UI fixture'}));print(json.dumps({'Name':'secondary','Current':False,'Description':'Empty own context'}));sys.exit(0)
if len(args)<2 or args[0]!='--context' or args[1] not in ['fixture','secondary']:sys.exit(3)
context=args[1];args=args[2:]
if context=='secondary' and args[:2]==['container','ls']:sys.exit(0)
statesfile=root/'states.json'
states=json.loads(statesfile.read_text()) if statesfile.exists() else {'a':'running','b':'exited','c':'exited'}
names={'a':'api-fixture','b':'worker-fixture','c':'zeta-fixture'}
if args[:2]==['container','ls']:
 for key,state in states.items():print(json.dumps({'ID':key*64,'Names':names[key],'Image':'fixture:local','State':state,'Status':state,'Ports':'127.0.0.1:8080->80/tcp'}))
elif args[:2]==['container','inspect']:
 key=args[-1][0];print(json.dumps({'ID':key*64,'Name':'/'+names[key],'Image':'fixture:local','State':states[key],'Created':'2026-10-26T12:00:00Z','Ports':{},'Mounts':[],'Networks':{'fixture':{'IPAddress':'172.20.0.2'}},'WorkingDir':'/app','User':'1000','RestartPolicy':'no'}))
elif args[:2]==['container','logs']:print('2026-10-26T12:00:00Z ação local 🌧')
elif args[0]=='container':
 key=args[-1][0];action=args[1]
 if action=='rm':del states[key]
 else:states[key]={'start':'running','stop':'exited','restart':'running','pause':'paused','unpause':'running'}[action]
 statesfile.write_text(json.dumps(states));print(key*64)
else:sys.exit(4)
"##).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.service
        .dispatch(Command::Docker(DockerCommand::Configure { program: cli }))
        .unwrap();
    app.docker_ui.open = true;
    (dir, app)
}

#[cfg(unix)]
fn fixture(size: [f32; 2]) -> (tempfile::TempDir, DockerHarness) {
    let (dir, app) = fixture_app();
    let mut harness = Harness::builder()
        .with_size(size)
        .with_max_steps(30)
        .build_ui_state(
            move |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.service.poll();
                if app.show_activity {
                    egui::Panel::bottom("fixture.docker.bottom")
                        .exact_size((size[1] * 0.5).min(350.))
                        .show(ui, |ui| app.tool_panel(ui));
                }
                app.docker_panel(ui);
            },
            app,
        );
    harness.run();
    (dir, harness)
}

fn settle(harness: &mut DockerHarness) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.step();
        if !harness.state().service.docker.is_busy() {
            break;
        }
        assert!(Instant::now() < deadline, "Docker fixture timeout");
        thread::sleep(Duration::from_millis(3));
    }
    harness.run();
    assert_eq!(harness.state().service.docker.state.error, None);
}

fn connect(h: &mut DockerHarness) {
    h.get_by_label("Conectar ao Docker").click();
    settle(h);
}

#[test]
#[cfg(unix)]
fn connect_select_logs_actions_and_shell_use_the_shared_worker() {
    let (dir, mut h) = fixture([340., 800.]);
    assert!(
        !dir.path().join("calls.jsonl").exists(),
        "Opening the panel must not run Docker"
    );
    connect(&mut h);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.docker.state.selected.as_deref(),
        Some("a".repeat(64).as_str())
    );
    assert!(
        h.get_by_label("Iniciar container")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !h.get_by_label("Excluir container")
            .accesskit_node()
            .is_disabled(),
        "Active containers offer explicit stop-and-delete confirmation"
    );
    h.get_by_label("Logs").click();
    settle(&mut h);
    assert!(
        h.query_by_label("2026-10-26T12:00:00Z ação local 🌧\n")
            .is_some()
    );
    h.get_by_label("Abrir shell do container").click();
    h.run();
    h.get_by_label("Abrir bash").click();
    h.run();
    assert_eq!(
        h.state().docker_ui.pending_shell,
        Some((
            "a".repeat(64),
            "api-fixture".into(),
            "bash".into(),
            "fixture".into()
        ))
    );
    h.get_by_label("Pausar container").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers[0].state, "paused");
    assert!(
        h.get_by_label("Abrir shell do container")
            .accesskit_node()
            .is_disabled()
    );
    h.get_by_label("Continuar container").click();
    settle(&mut h);
    h.get_by_label("Parar container").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers[0].state, "exited");
    h.get_by_label("Iniciar container").click();
    settle(&mut h);
    h.get_by_label("Reiniciar container").click();
    settle(&mut h);
    let calls = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(calls.contains("\"pause\""));
    assert!(!calls.contains("\"rm\""));
}

#[test]
#[cfg(unix)]
fn remove_requires_inline_confirmation_and_selection_change_cancels_it() {
    let (dir, mut h) = fixture([340., 800.]);
    connect(&mut h);
    h.get_by_label("worker-fixture · Parado").click();
    settle(&mut h);
    h.get_by_label("Excluir container").click();
    h.run();
    assert!(h.query_by_label("Confirmar exclusão").is_some());
    h.get_by_label("Cancelar exclusão").click();
    h.run();
    assert!(h.query_by_label("Confirmar exclusão").is_none());
    h.get_by_label("Excluir container").click();
    h.run();
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    assert!(h.state().docker_ui.confirm_remove.is_none());
    assert!(h.query_by_label("Confirmar exclusão").is_none());
    h.get_by_label("worker-fixture · Parado").click();
    settle(&mut h);
    h.get_by_label("Excluir container").click();
    h.run();
    h.get_by_label("Confirmar exclusão").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers.len(), 2);
    assert!(h.state().service.docker.state.selected.is_none());
    let calls = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert_eq!(
        calls.lines().filter(|line| line.contains("\"rm\"")).count(),
        1
    );
}

#[test]
#[cfg(unix)]
fn context_delete_and_keyboard_target_the_clicked_row_and_escape_never_removes() {
    let (dir, mut h) = fixture([700., 850.]);
    connect(&mut h);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    h.get_by_label("zeta-fixture · Parado").click_secondary();
    h.run();
    h.key_press(Key::Delete);
    h.run();
    assert!(
        h.state().docker_ui.confirm_remove.is_none(),
        "An open menu must own keyboard input"
    );
    h.get_by_label("Excluir container…").click();
    settle(&mut h);
    assert!(
        h.query_by_label("Excluir o container “zeta-fixture”?")
            .is_some()
    );
    assert_eq!(
        h.state().service.docker.state.selected,
        Some("c".repeat(64))
    );
    assert!(
        !fs::read_to_string(dir.path().join("calls.jsonl"))
            .unwrap()
            .contains("\"rm\"")
    );
    h.get_by_label("Cancelar exclusão").click();
    h.run();
    h.get_by_label("zeta-fixture · Parado").click();
    settle(&mut h);
    h.key_press(Key::Delete);
    h.run();
    assert!(h.query_by_label("Confirmar exclusão").is_some());
    h.key_press(Key::Escape);
    h.run();
    assert!(h.state().docker_ui.confirm_remove.is_none());
    assert!(
        !fs::read_to_string(dir.path().join("calls.jsonl"))
            .unwrap()
            .contains("\"rm\"")
    );
    h.key_press(Key::Delete);
    h.run();
    h.get_by_label("Confirmar exclusão").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers.len(), 2);
    assert!(
        h.state()
            .service
            .docker
            .state
            .containers
            .iter()
            .all(|c| c.id != "c".repeat(64))
    );
    assert!(
        h.state()
            .service
            .docker
            .state
            .containers
            .iter()
            .any(|c| c.name == "api-fixture" && c.state == "running")
    );
    let calls: Vec<Vec<String>> = fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let remove: Vec<_> = calls
        .iter()
        .filter(|args| args.get(3).map(String::as_str) == Some("rm"))
        .collect();
    assert_eq!(
        remove,
        [&vec![
            "--context".to_owned(),
            "fixture".into(),
            "container".into(),
            "rm".into(),
            "c".repeat(64)
        ]]
    );
}

#[test]
#[cfg(unix)]
fn active_and_paused_containers_require_explicit_stop_and_delete() {
    let (dir, mut h) = fixture([700., 850.]);
    connect(&mut h);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    h.get_by_label("Excluir container").click();
    h.run();
    assert!(h.query_by_label("Parar e excluir").is_some());
    assert!(h.query_by_label("Confirmar exclusão").is_none());
    h.get_by_label("Cancelar exclusão").click();
    h.run();
    let calls = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    assert!(!calls.contains("\"stop\"") && !calls.contains("\"rm\""));
    h.get_by_label("Pausar container").click();
    settle(&mut h);
    h.get_by_label("Excluir container").click();
    h.run();
    assert!(
        h.query_by_label_contains("Se estiver pausado, será retomado")
            .is_some()
    );
    h.get_by_label("Parar e excluir").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers.len(), 2);
    assert!(h.state().service.docker.state.selected.is_none());
    assert!(h.state().service.docker.state.details.is_none());
    assert!(h.state().service.docker.state.logs.is_empty());
    assert_eq!(
        h.state().service.docker.state.notice.as_deref(),
        Some("Container “api-fixture” excluído.")
    );
    let calls: Vec<Vec<String>> = fs::read_to_string(dir.path().join("calls.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let mutations: Vec<_> = calls
        .iter()
        .filter_map(|args| {
            args.get(3)
                .filter(|name| ["pause", "unpause", "stop", "rm"].contains(&name.as_str()))
                .map(String::as_str)
        })
        .collect();
    assert_eq!(mutations, ["pause", "unpause", "stop", "rm"]);
    assert!(
        !calls
            .iter()
            .flatten()
            .any(|arg| matches!(arg.as_str(), "--force" | "--volumes" | "prune"))
    );
}

#[test]
#[cfg(unix)]
fn context_removal_waits_for_background_ack_and_selects_its_target_before_confirming() {
    let (dir, mut h) = fixture([700., 850.]);
    connect(&mut h);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    fs::write(dir.path().join("gate"), "").unwrap();
    fs::remove_file(dir.path().join("entered")).unwrap();
    h.state_mut()
        .service
        .docker
        .refresh_in_background()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !dir.path().join("entered").exists() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    h.get_by_label("worker-fixture · Parado").click_secondary();
    h.run();
    h.get_by_label("Excluir container…").click();
    settle(&mut h);
    assert_eq!(
        h.state().service.docker.state.selected,
        Some("b".repeat(64))
    );
    assert!(
        h.query_by_label("Excluir o container “worker-fixture”?")
            .is_some()
    );
    assert!(
        !h.get_by_label("Confirmar exclusão")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !fs::read_to_string(dir.path().join("calls.jsonl"))
            .unwrap()
            .contains("\"rm\"")
    );
    h.get_by_label("Confirmar exclusão").click();
    settle(&mut h);
    assert!(
        h.state()
            .service
            .docker
            .state
            .containers
            .iter()
            .all(|c| c.id != "b".repeat(64))
    );
    assert!(
        h.state()
            .service
            .docker
            .state
            .containers
            .iter()
            .any(|c| c.name == "api-fixture" && c.state == "running")
    );
}

#[test]
#[cfg(unix)]
fn filter_keyboard_navigation_empty_state_and_insets_survive_narrow_panel() {
    for width in [250., 340.] {
        let (_dir, mut h) = fixture([width, 700.]);
        let filter_rect = h.get_by_label("Filtrar containers").rect();
        let context_rect = h.get_by_role(egui::accesskit::Role::ComboBox).rect();
        assert!((context_rect.left() - filter_rect.left()).abs() <= 1.);
        assert!(
            (context_rect.right() - filter_rect.right()).abs() <= 1.,
            "context {context_rect:?}, filter {filter_rect:?}"
        );
        assert!(
            filter_rect.left() >= 12. && filter_rect.right() <= width - 12.,
            "{filter_rect:?}"
        );
        connect(&mut h);
        h.get_by_label("api-fixture · Em execução").click();
        settle(&mut h);
        h.key_press(Key::ArrowDown);
        settle(&mut h);
        assert_eq!(
            h.state().service.docker.state.selected.as_deref(),
            Some("b".repeat(64).as_str())
        );
        h.key_press(Key::ArrowUp);
        settle(&mut h);
        assert_eq!(
            h.state().service.docker.state.selected.as_deref(),
            Some("a".repeat(64).as_str())
        );
        h.get_by_label("Filtrar containers").click();
        h.run();
        h.event(egui::Event::Text("absent-fixture".into()));
        h.run();
        assert!(
            h.query_by_label("Nenhum container corresponde ao filtro.")
                .is_some()
        );
        assert!(h.query_by_label("api-fixture · Em execução").is_none());
        assert!(h.state().service.docker.state.connected);
        for label in [
            "Filtrar containers",
            "Somente em execução",
            "Ocultar Docker",
        ] {
            let rect = h.get_by_label(label).rect();
            assert!(
                rect.left() >= 0.
                    && rect.right() <= width
                    && rect.top() >= 0.
                    && rect.bottom() <= 700.,
                "{label}: {rect:?}"
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn configure_cancel_does_not_replace_the_executable_and_hide_preserves_selection() {
    let (_dir, mut h) = fixture([340., 700.]);
    connect(&mut h);
    h.get_by_label("worker-fixture · Parado").click();
    settle(&mut h);
    let original = h.state().service.docker.program().to_owned();
    h.get_by_label("Configurar Docker").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Executável Docker")
        .click();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.event(egui::Event::Text("/fixture/unused".into()));
    h.get_by_label("Cancelar configuração").click();
    h.run();
    assert_eq!(h.state().service.docker.program(), original);
    h.get_by_label("Ocultar Docker").click();
    h.run();
    assert!(!h.state().docker_ui.open);
    assert_eq!(
        h.state().service.docker.state.selected.as_deref(),
        Some("b".repeat(64).as_str())
    );
}

#[test]
#[cfg(unix)]
fn unavailable_cli_routes_error_to_status_and_keeps_configuration_reachable() {
    let (dir, mut h) = fixture([280., 700.]);
    h.state_mut()
        .service
        .dispatch(Command::Docker(DockerCommand::Configure {
            program: dir.path().join("absent-docker"),
        }))
        .unwrap();
    h.run();
    h.get_by_label("Conectar ao Docker").click();
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().service.docker.is_busy() {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
    let error = h
        .state()
        .service
        .docker
        .state
        .error
        .clone()
        .expect("missing CLI error");
    assert!(h.query_by_label(&error).is_none());
    assert_eq!(
        crate::status_messages::current(&h.ctx).as_deref(),
        Some(error.as_str())
    );
    assert!(!h.state().service.docker.state.connected);
    h.get_by_label("Configurar Docker").click();
    h.run();
    assert!(
        h.query_by_role_and_label(egui::accesskit::Role::TextInput, "Executável Docker")
            .is_some()
    );
}

#[test]
#[cfg(unix)]
fn virtual_list_renders_only_the_viewport_and_reaches_the_final_record() {
    let (_dir, mut h) = fixture([340., 700.]);
    connect(&mut h);
    // Own model fixture isolates virtualization, rather than manufacturing daemon evidence.
    h.state_mut().service.docker.state.containers = (0..1000)
        .map(|index| Container {
            id: format!("{index:064x}"),
            name: format!("item-{index:04}"),
            image: "fixture:local".into(),
            state: "exited".into(),
            ..Container::default()
        })
        .collect();
    h.run();
    let count = h
        .query_all_by_role(egui::accesskit::Role::ListBoxOption)
        .count();
    assert!(count > 1 && count < 20, "painted {count} of 1000 rows");
    let row = h.get_by_label("item-0000 · Parado").rect();
    h.hover_at(row.center());
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0., -100_000.),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
    h.run();
    assert!(h.query_by_label("item-0999 · Parado").is_some());
    assert!(h.query_by_label("item-0000 · Parado").is_none());
    assert!(
        h.query_all_by_role(egui::accesskit::Role::ListBoxOption)
            .count()
            < 20
    );
}

#[cfg(unix)]
fn full_fixture() -> (tempfile::TempDir, DockerHarness) {
    let (dir, app) = fixture_app();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
    h.run();
    (dir, h)
}

#[test]
#[cfg(unix)]
fn selection_opens_bottom_docker_preserves_logs_and_panel_size_across_navigation() {
    use egui::accesskit::Role;
    let (dir, mut h) = full_fixture();
    connect(&mut h);
    let sidebar = h.get_by_label("Filtrar containers").rect();
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    assert!(h.state().show_activity && h.state().tool_tab == ToolTab::Docker);
    let tab = h.get_by_role_and_label(Role::Tab, "Docker").rect();
    let refs = h.get_by_role_and_label(Role::Tab, "Referências").rect();
    assert!(tab.left() >= refs.right());
    assert_eq!(h.get_by_label("Filtrar containers").rect(), sidebar);
    for label in [
        "Abrir shell do container",
        "Parar container",
        "Detalhes",
        "Logs",
        "ID",
    ] {
        assert!(
            h.get_by_label(label).rect().top() >= tab.bottom(),
            "{label} did not move below navigation"
        );
    }
    assert!(h.get_by_label("api-fixture · Em execução").rect().bottom() < tab.top());
    h.get_by_label("Logs").click();
    settle(&mut h);
    let calls = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
    let logs = h.state().service.docker.state.logs.clone();
    for label in ["Debug", "Referências", "Atividade", "Docker"] {
        h.get_by_role_and_label(Role::Tab, label).click();
        settle(&mut h);
        assert!((h.get_by_role_and_label(Role::Tab, "Docker").rect().top() - tab.top()).abs() < 1.);
    }
    assert_eq!(h.state().service.docker.state.logs, logs);
    assert!(h.state().docker_ui.logs);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    assert!(h.state().docker_ui.logs);
    assert_eq!(
        fs::read_to_string(dir.path().join("calls.jsonl")).unwrap(),
        calls
    );
    h.get_by_label("Ocultar painel inferior").click();
    settle(&mut h);
    assert!(!h.state().show_activity);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    assert!(h.state().docker_ui.logs);
    assert_eq!(h.state().service.docker.state.logs, logs);
    h.get_by_label("worker-fixture · Parado").click();
    settle(&mut h);
    assert!(!h.state().docker_ui.logs);
    assert!(h.state().service.docker.state.logs.is_empty());
    assert!(h.state().tool_tab == ToolTab::Docker);
    assert!(h.get_by_label("Iniciar container").rect().top() > tab.bottom());
}

#[test]
#[cfg(unix)]
fn bottom_remains_usable_without_sidebar_and_context_change_clears_old_content() {
    use egui::accesskit::Role;
    let (_dir, mut h) = full_fixture();
    connect(&mut h);
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h);
    h.get_by_label("Ocultar Docker").click();
    settle(&mut h);
    assert!(!h.state().docker_ui.open);
    h.get_by_label("Parar container").click();
    settle(&mut h);
    assert_eq!(h.state().service.docker.state.containers[0].state, "exited");
    h.get_by_label("Excluir container").click();
    settle(&mut h);
    assert!(h.state().docker_ui.confirm_remove.is_some());
    h.state_mut()
        .service
        .dispatch(Command::Docker(DockerCommand::SelectContext(
            "secondary".into(),
        )))
        .unwrap();
    settle(&mut h);
    assert!(h.state().docker_ui.confirm_remove.is_none());
    assert!(h.query_by_label("Confirmar exclusão").is_none());
    assert!(
        h.query_by_label("Selecione um container no painel Docker à esquerda.")
            .is_some()
    );
    assert!(h.query_by_label("Parar container").is_none());
    h.get_by_label("Configurar ferramenta Docker").click();
    settle(&mut h);
    assert!(h.state().docker_ui.open);
    assert!(
        h.query_by_role_and_label(Role::TextInput, "Executável Docker")
            .is_some()
    );
}

#[test]
#[cfg(unix)]
fn automatic_refresh_preserves_controls_geometry_and_accepts_selection() {
    let (dir, mut h) = fixture([340., 800.]);
    connect(&mut h);
    let before = [
        h.get_by_label("Filtrar containers").rect(),
        h.get_by_label("api-fixture · Em execução").rect(),
    ];
    fs::write(dir.path().join("gate"), "").unwrap();
    h.state_mut().show_project = true;
    h.state_mut().docker_last_refresh = Instant::now() - Duration::from_secs(6);
    h.state_mut().docker_tick(&egui::Context::default());
    assert!(h.state().service.docker.is_background_refresh());
    h.run();
    assert_eq!(h.get_by_label("Filtrar containers").rect(), before[0]);
    assert_eq!(
        h.get_by_label("api-fixture · Em execução").rect(),
        before[1]
    );
    assert!(h.query_by_label("Atualizando").is_none());
    assert!(h.query_by_label("Cancelar operação Docker").is_none());
    assert!(
        !h.get_by_label("Configurar Docker")
            .accesskit_node()
            .is_disabled()
    );
    assert!(h.query_by_label("Atualizar containers").is_some());
    h.get_by_label("api-fixture · Em execução").click();
    settle(&mut h); // selection cancels the still-gated automatic refresh.
    assert!(h.state().tool_tab == ToolTab::Docker);
    assert_eq!(
        h.state().service.docker.state.details.as_ref().unwrap().id,
        "a".repeat(64)
    );
    h.get_by_label("Atualizar containers").click();
    h.step();
    h.step(); // Bottom feedback precedes the sidebar's click dispatch in a frame.
    assert!(h.state().service.docker.is_foreground_busy());
    assert!(h.query_by_label("Atualizando").is_some());
    fs::remove_file(dir.path().join("gate")).unwrap();
    settle(&mut h);
}
