//! Real shells in sibling projects, independent from source-file encoding.
#![cfg(unix)]
use super::*;
use std::time::Instant;

#[test]
fn workspace_terminal_picker_opens_named_roots_with_real_cwd() {
    use crate::CodisseuApp;
    use codisseu_core::{AppService, Command};
    use egui_kittest::{Harness, kittest::Queryable};
    let fixture = tempfile::tempdir().unwrap();
    let first = fixture.path().join("api");
    let second = fixture.path().join("outside");
    let base = fixture.path().join("workspace");
    for path in [&first, &second, &base] {
        std::fs::create_dir(path).unwrap();
    }
    for path in [&first, &second] {
        assert!(
            std::process::Command::new("git")
                .args(["-c", "init.templateDir=", "init", "-q"])
                .arg(path)
                .status()
                .unwrap()
                .success()
        );
    }
    std::fs::write(first.join("first.php"), "<?php // fixture\n").unwrap();
    std::fs::write(second.join("second.ts"), "export const value = 1;\n").unwrap();
    let file = base.join("services.code-workspace");
    std::fs::write(
        &file,
        r#"{"folders":[{"name":"API","path":"../api"},{"name":"Frontend","path":"../outside"},{"name":"Planos","path":"missing"}]}"#,
    )
    .unwrap();
    let mut service = AppService::new(fixture.path().join("profile/session.json"));
    service.dispatch(Command::OpenWorkspace(file)).unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    while service.is_busy() {
        service.poll();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(2));
    }
    assert!(service.last_error.is_none(), "{:?}", service.last_error);
    let mut h = Harness::builder()
        .with_pixels_per_point(
            std::env::var("CODISSEU_WORKSPACE_SCALE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.),
        )
        .with_size([1280., 800.])
        .with_max_steps(100)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    h.run();
    assert!(h.query_by_label("API").is_some());
    assert!(h.query_by_label("Frontend").is_some());
    assert!(h.query_by_label("Planos (pasta ausente)").is_some());
    h.get_by_label("API").click();
    h.run();
    let until = Instant::now() + Duration::from_secs(15);
    while h.query_by_label("first.php").is_none() {
        h.step();
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(5));
    }
    h.get_by_label("first.php").click();
    h.run();
    h.state_mut().open_terminal();
    h.run();
    h.get_by_label("Escolher pasta do terminal").click();
    h.run();
    h.get_by_label("Novo terminal…").click();
    h.run();
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_CAPTURES") {
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("terminal-folder-picker.png"))
            .unwrap();
    }
    // Explorer and popup share names; choose the button in the menu.
    h.event(egui::Event::Text("Front".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    let pane = &mut h.state_mut().terminal;
    pane.send(b"printf 'ACTUAL_CWD='; pwd\r".to_vec());
    wait(pane, |p| {
        p.parser
            .screen()
            .contents()
            .contains(&format!("ACTUAL_CWD={}", second.display()))
    });
    assert_eq!(pane.parked.len(), 1);
    assert_eq!(pane.parked[0].cwd, first);
    h.run();
    h.get_by_label("Novo terminal").click();
    h.run();
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Terminal em API")
        .click();
    h.run();
    let pane = &mut h.state_mut().terminal;
    pane.send(b"printf 'SECOND_CWD='; pwd\r".to_vec());
    wait(pane, |p| {
        p.parser
            .screen()
            .contents()
            .contains(&format!("SECOND_CWD={}", first.display()))
    });
    assert_eq!(pane.parked.len(), 2);
    h.run();
    // The terminal surface must retain all existing neighboring tools and the
    // same PTYs/results when switching back and forth (user regression concern).
    let database = fixture.path().join("local-only.sqlite");
    drop(rusqlite::Connection::open(&database).unwrap());
    for command in [
        Command::Database(codisseu_core::DbCommand::Connect(database)),
        Command::Database(codisseu_core::DbCommand::Execute {
            sql: "SELECT 42 AS retained_value".into(),
        }),
    ] {
        h.state_mut().service.dispatch(command).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while h.state().service.is_busy() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < until);
        }
    }
    h.run();
    let rows = h.state().service.db.result.as_ref().unwrap().rows.clone();
    let session_identity = h.state().terminal.identity;
    for label in [
        "Git",
        "Debug",
        "Services",
        "Atividade",
        "Referências",
        "Docker",
    ] {
        h.get_by_role_and_label(egui::accesskit::Role::Tab, label)
            .click();
        let until = Instant::now() + Duration::from_secs(10);
        h.step();
        while h.state().service.is_busy() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < until);
        }
        h.run();
        assert_eq!(h.state().service.db.result.as_ref().unwrap().rows, rows);
        h.get_by_role_and_label(egui::accesskit::Role::Tab, "Terminal")
            .click();
        h.run();
        assert_eq!(h.state().terminal.identity, session_identity);
        assert_eq!(h.state().terminal.parked.len(), 2);
        assert!(
            h.state()
                .terminal
                .parser
                .screen()
                .contents()
                .contains("SECOND_CWD=")
        );
    }
    let plus = h.get_by_label("Novo terminal").rect();
    let terminal_tab = h
        .get_by_role_and_label(egui::accesskit::Role::Tab, "Terminal")
        .rect();
    assert!((plus.center().y - terminal_tab.center().y).abs() < 2.);
    assert!(plus.left() > terminal_tab.right());
    let frontend_id = h
        .state()
        .terminal
        .parked
        .iter()
        .find(|p| p.cwd == second)
        .unwrap()
        .identity;
    h.get_by_role_and_label(
        egui::accesskit::Role::Tab,
        &format!("Terminal {frontend_id} em Frontend"),
    )
    .click();
    h.run();
    assert_eq!(h.state().terminal.cwd, second);
    h.get_by_role_and_label(
        egui::accesskit::Role::Tab,
        &format!("Terminal {session_identity} em API"),
    )
    .click();
    h.run();
    assert_eq!(h.state().terminal.identity, session_identity);
    h.get_by_label("Novo terminal").click();
    h.run();
    h.key_press(egui::Key::Escape);
    h.run();
    assert_eq!(h.state().terminal.parked.len(), 2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::J);
    h.run();
    assert!(!h.state().show_activity);
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::F12);
    h.run();
    assert_eq!(h.state().terminal.identity, session_identity);
    h.set_size(egui::vec2(800., 600.));
    h.run();
    assert!(h.get_by_label("Novo terminal").rect().right() < 800.);
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_CAPTURES") {
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("terminal-narrow.png"))
            .unwrap();
    }
    h.set_size(egui::vec2(1280., 800.));
    h.run();
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_CAPTURES") {
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("workspace-terminal.png"))
            .unwrap();
    }
    h.get_by_label("Mais ações do terminal").click();
    h.run();
    h.get_by_label("Dividir terminal").click();
    h.run();
    assert_eq!(h.state().terminal.splits.len(), 1);
    h.event(egui::Event::Text("printf SPLIT_ONLY; pwd".into()));
    h.key_press(egui::Key::Enter);
    h.run();
    wait(&mut h.state_mut().terminal.splits[0], |p| {
        p.parser
            .screen()
            .contents()
            .contains(&format!("SPLIT_ONLY{}", first.display()))
    });
    assert!(
        !h.state()
            .terminal
            .parser
            .screen()
            .contents()
            .contains("SPLIT_ONLY")
    );
    if let Some(out) = std::env::var_os("CODISSEU_WORKSPACE_CAPTURES") {
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join("terminal-split.png"))
            .unwrap();
    }
    h.get_by_label("Maximizar painel").click();
    h.run();
    assert!(h.state().terminal_ui.maximized);
    h.get_by_label("Restaurar painel").click();
    h.run();
    assert!(!h.state().terminal_ui.maximized);
    h.get_by_label("Mais ações do terminal").click();
    h.run();
    h.get_by_label("Encerrar terminal atual").click();
    h.run();
    assert!(h.state().terminal.splits.is_empty());
    h.get_by_label("Mais ações do terminal").click();
    h.run();
    h.get_by_label("Encerrar terminal atual").click();
    h.run();
    assert_eq!(h.state().terminal.parked.len(), 1);
    h.state_mut().terminal.close();
    wait(&mut h.state_mut().terminal, |p| !p.shutdown_pending());
}

fn wait(pane: &mut TerminalPane, predicate: impl Fn(&TerminalPane) -> bool) {
    let start = Instant::now();
    while !predicate(pane) {
        pane.poll();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{:?}",
            pane.message
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
#[test]
fn sibling_project_ptys_keep_cwd_history_and_cleanup_independent_of_source_encoding() {
    let fixture = tempfile::tempdir().unwrap();
    let projects = ["utf8-service", "latin1-service", "windows1252-service"];
    let sources: [&[u8]; 3] = [
        "<?php // ação 界\n".as_bytes(),
        b"<?php // a\xe7\xe3o\n",
        b"<?php // \x80 \x93a\xe7\xe3o\x94\n",
    ];
    let mut pane = TerminalPane::default();
    for (index, project) in projects.iter().enumerate() {
        let path = fixture.path().join(project);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("index.php"), sources[index]).unwrap();
        // Explicit fixture shell avoids reading the user's interactive profile.
        assert!(pane.open_command(
            &path,
            "/bin/sh".into(),
            vec!["-i".into()],
            project.to_string(),
            project.to_string()
        ));
        pane.send(
            format!("printf 'MARKER_%s\\n' '{index}'; pwd; printf 'UTF8_%s\\n' 'ação界'\r")
                .into_bytes(),
        );
        wait(&mut pane, |p| {
            p.parser.screen().contents().contains("UTF8_ação界")
        });
        assert!(
            pane.parser
                .screen()
                .contents()
                .contains(path.to_str().unwrap())
        );
        assert_eq!(
            std::fs::read(path.join("index.php")).unwrap(),
            sources[index]
        );
    }
    assert_eq!(pane.parked.len(), 2);
    for (index, project) in projects.iter().enumerate() {
        let path = fixture.path().join(project);
        assert!(pane.open_command(
            &path,
            "/bin/sh".into(),
            vec!["-i".into()],
            project.to_string(),
            project.to_string()
        ));
        assert!(
            pane.parser
                .screen()
                .contents()
                .contains(&format!("MARKER_{index}"))
        );
        assert_eq!(
            pane.parked.len(),
            2,
            "reopening reuses the live project PTY"
        );
        pane.send(format!("printf 'CWD_{index}='; pwd\r").into_bytes());
        wait(&mut pane, |p| {
            p.parser
                .screen()
                .contents()
                .contains(&format!("CWD_{index}={}", path.display()))
        });
    }
    pane.close();
    wait(&mut pane, |p| !p.shutdown_pending());
    assert!(!pane.has_sessions());
}
