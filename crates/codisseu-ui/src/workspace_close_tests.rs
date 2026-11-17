//! Lifecycle regressions: real workers, actual widget actions, private files only.
use super::*;
use codisseu_core::{Language, LanguageCommand, ProviderConfig};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};

type H = Harness<'static, CodisseuApp>;
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, H) {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("Projeto ação A");
    let b = dir.path().join("Projeto ação B");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    fs::write(a.join("source.php"), "<?php\n$weather = 'saved';\n").unwrap();
    fs::write(b.join("other.ts"), "export const weather = 1;\n").unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    let h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    (dir, a, b, h)
}
fn wait(h: &mut H, predicate: impl Fn(&CodisseuApp) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        h.step();
        if predicate(h.state()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn open(h: &mut H, root: &std::path::Path) {
    h.state_mut()
        .send(Command::OpenWorkspace(root.to_path_buf()));
    wait(h, |a| !a.service.is_busy() && !a.workspace_switch_pending());
}
fn edit(h: &mut H, root: &std::path::Path) -> DocumentId {
    h.state_mut()
        .send(Command::OpenFile(root.join("source.php")));
    wait(h, |a| !a.service.is_busy());
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: "<?php\n$weather = 'dirty';\n".into(),
        })
        .unwrap();
    id
}
#[test]
fn workspace_close_welcome_and_header_retain_selection_while_language_ack_is_pending() {
    let (_dir, a, b, mut h) = fixture();
    let selected = a.clone();
    h.state_mut()
        .set_folder_picker(move |_| Some(selected.clone()))
        .unwrap();
    h.state_mut()
        .service
        .language
        .dispatch(LanguageCommand::CancelHover)
        .unwrap();
    h.get_by_label("Abrir projeto    Ctrl+Shift+O").click();
    h.step();
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.workspace_switch_pending() && !a.folder_picker_pending()
    });
    assert_eq!(h.state().service.workspace, Some(a.clone()));
    let selected = b.clone();
    h.state_mut()
        .set_folder_picker(move |_| Some(selected.clone()))
        .unwrap();
    h.state_mut()
        .service
        .language
        .dispatch(LanguageCommand::CancelHover)
        .unwrap();
    h.get_by_label("Projeto atual").click();
    h.step();
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.workspace_switch_pending() && !a.folder_picker_pending()
    });
    assert_eq!(h.state().service.workspace, Some(b));
    assert!(h.state().service.last_error.is_none());
    assert_eq!(
        fs::read_to_string(a.join("source.php")).unwrap(),
        "<?php\n$weather = 'saved';\n"
    );
}
#[test]
fn workspace_close_native_selection_is_queued_before_consuming_ready_ack() {
    let (_dir, a, _, mut h) = fixture();
    let selected = a.clone();
    h.state_mut()
        .set_folder_picker(move |_| Some(selected.clone()))
        .unwrap();
    h.state_mut().open_native_folder();
    thread::sleep(Duration::from_millis(20));
    h.state_mut()
        .service
        .language
        .dispatch(LanguageCommand::CancelHover)
        .unwrap();
    assert!(h.state().service.language.is_busy());
    let ctx = h.ctx.clone();
    h.state_mut().poll_native_folder(&ctx);
    assert!(
        h.state().workspace_switch_pending(),
        "selection must survive the busy guard"
    );
    assert!(h.state().service.last_error.is_none());
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.workspace_switch_pending()
    });
    assert_eq!(h.state().service.workspace, Some(a));
}
#[test]
fn workspace_close_dirty_cancel_then_save_and_open_preserves_bytes() {
    let (_dir, a, b, mut h) = fixture();
    open(&mut h, &a);
    let id = edit(&mut h, &a);
    h.state_mut().send(Command::OpenWorkspace(b.clone()));
    wait(&mut h, |a| !a.service.is_busy());
    h.step();
    h.get_by_label("Voltar ao editor").click();
    h.step();
    assert!(!h.state().workspace_switch_pending());
    assert!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(
        fs::read_to_string(a.join("source.php")).unwrap(),
        "<?php\n$weather = 'saved';\n"
    );
    h.state_mut().send(Command::OpenWorkspace(b.clone()));
    wait(&mut h, |a| !a.service.is_busy());
    h.step();
    h.get_by_label("Salvar e abrir").click();
    h.step();
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.workspace_switch_pending()
    });
    assert_eq!(h.state().service.workspace, Some(b));
    assert_eq!(
        fs::read_to_string(a.join("source.php")).unwrap(),
        "<?php\n$weather = 'dirty';\n"
    );
}
#[test]
fn workspace_close_pending_dirty_decision_pauses_auto_save_and_startup_capture() {
    let (dir, a, b, mut h) = fixture();
    open(&mut h, &a);
    let id = edit(&mut h, &a);
    h.state_mut()
        .service
        .dispatch(Command::SetAutoSave {
            enabled: true,
            delay_ms: 250,
        })
        .unwrap();
    h.step();
    h.state_mut().send(Command::OpenWorkspace(b));
    wait(&mut h, |a| !a.service.is_busy());
    h.state_mut()
        .capture_to(dir.path().join("must-not-capture.png"), false);
    thread::sleep(Duration::from_millis(350));
    for _ in 0..8 {
        h.step();
    }
    assert_eq!(h.state().service.workspace, Some(a.clone()));
    assert!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(
        fs::read_to_string(a.join("source.php")).unwrap(),
        "<?php\n$weather = 'saved';\n"
    );
    assert!(!h.state().capture_requested);
}

#[test]
fn workspace_close_switch_keeps_pending_database_grid_edits_in_the_original_project() {
    let (_dir, a, b, mut h) = fixture();
    open(&mut h, &a);
    h.state_mut()
        .database_ui
        .staged
        .insert((0, 0), codisseu_core::remote::Cell::Text("Keep A".into()));
    h.state_mut().send(Command::OpenWorkspace(b));
    h.step();
    h.step();
    assert_eq!(h.state().service.workspace_path(), Some(a.as_path()));
    assert_eq!(h.state().database_ui.staged.len(), 1);
    assert!(
        h.query_by_label("Não foi possível abrir o projeto")
            .is_some()
    );
    h.get_by_label("Voltar ao editor").click();
    h.step();
    assert!(!h.state().workspace_switch_pending());
    assert_eq!(h.state().database_ui.staged.len(), 1);
}
#[test]
fn workspace_close_failed_save_and_invalid_target_keep_project_and_buffer() {
    let (_dir, a, b, mut h) = fixture();
    open(&mut h, &a);
    let id = edit(&mut h, &a);
    fs::remove_file(a.join("source.php")).unwrap();
    fs::create_dir(a.join("source.php")).unwrap();
    h.state_mut().send(Command::OpenWorkspace(b));
    wait(&mut h, |a| !a.service.is_busy());
    h.step();
    h.get_by_label("Salvar e abrir").click();
    h.step();
    wait(&mut h, |a| {
        !a.service.is_busy() && a.service.last_error.is_some()
    });
    h.step();
    assert_eq!(h.state().service.workspace, Some(a.clone()));
    assert_eq!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .text(),
        "<?php\n$weather = 'dirty';\n"
    );
    assert!(
        h.query_by_label("Não foi possível abrir o projeto")
            .is_some()
    );
    h.get_by_label("Voltar ao editor").click();
    h.step();
    h.state_mut()
        .service
        .dispatch(Command::Close { id, discard: true })
        .unwrap();
    h.state_mut().send(Command::OpenWorkspace(a.join("absent")));
    wait(&mut h, |a| {
        !a.service.is_busy() && a.service.last_error.is_some()
    });
    h.step();
    assert_eq!(h.state().service.workspace, Some(a));
    assert!(
        h.query_by_label("Não foi possível abrir o projeto")
            .is_some()
    );
}
fn start_provider(h: &mut H, root: &std::path::Path, mode: &str, log: &std::path::Path) {
    open(h, root);
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lifecycle_server.py");
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::Php,
            program: "/usr/bin/python3".into(),
            args: vec![
                fixture.to_string_lossy().into(),
                mode.into(),
                log.to_string_lossy().into(),
            ],
        }]))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(h, |_| {
        fs::read_to_string(log).is_ok_and(|s| s.contains("initialize"))
    });
}
#[test]
fn workspace_close_does_not_wait_for_unresponsive_initialize_before_requesting_stop() {
    let (dir, a, _, mut h) = fixture();
    let log = dir.path().join("lsp.jsonl");
    start_provider(&mut h, &a, "initialize", &log);
    assert!(h.state().service.language.is_busy());
    h.state_mut().request_exit();
    let started = Instant::now();
    wait(&mut h, |a| a.allow_close);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "must stop before the 15s initialize timeout"
    );
    assert!(!h.state().service.any_project_providers_running());
    let records = fs::read_to_string(log).unwrap();
    assert!(records.contains("shutdown"));
    assert!(dir.path().join("profile/session.json").is_file());
}
#[test]
fn workspace_close_return_before_finalization_preserves_the_browser_service() {
    let (dir, a, _, mut h) = fixture();
    let log = dir.path().join("lsp.jsonl");
    start_provider(&mut h, &a, "initialize", &log);
    let browser_before = h.state().service.browser.state;
    h.state_mut().request_exit();
    h.state_mut().shutdown_started = Some(Instant::now() - Duration::from_secs(1));
    // Newly introduced egui modal areas need a sizing frame before AccessKit
    // exposes their visible controls. Keep the provider's real stop ACK pending.
    h.ctx.request_repaint();
    for _ in 0..4 {
        h.step();
    }
    h.get_by_label("Voltar ao editor").click();
    h.step();
    assert!(!h.state().exit_when_saved && !h.state().shutdown_finalizing);
    assert_eq!(h.state().service.browser.state, browser_before);
    wait(&mut h, |a| !a.service.is_busy());
    assert!(!h.state().allow_close);
}

#[test]
fn workspace_close_clean_exit_persists_metadata_without_flashing_a_saving_modal() {
    let (dir, a, _, mut h) = fixture();
    open(&mut h, &a);
    h.state_mut().request_exit();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !h.state().allow_close {
        h.step();
        assert!(h.query_by_label("Fechando o Codisseu").is_none());
        assert!(
            h.query_by_label("Salvando a sessão e concluindo o fechamento…")
                .is_none()
        );
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(2));
    }
    let session: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("profile/session.json")).unwrap())
            .unwrap();
    assert_eq!(session["workspace"].as_str(), a.to_str());
}

#[test]
fn workspace_close_slow_shutdown_shows_real_progress_only_after_delay() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
    app.exit_when_saved = true;
    app.shutdown_started = Some(Instant::now());
    // Render the real progress widget while withholding lifecycle ticks/ACKs.
    let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
        |ui, app: &mut CodisseuApp| app.shutdown_window(ui.ctx()),
        app,
    );
    assert!(h.query_by_label("Fechando o Codisseu").is_none());
    h.state_mut().shutdown_started = Some(Instant::now() - Duration::from_secs(1));
    h.ctx.request_repaint();
    for _ in 0..4 {
        h.step();
    }
    assert!(h.query_by_label("Fechando o Codisseu").is_some());
    assert!(h.query_by_label("Voltar ao editor").is_some());
    assert!(!h.state().allow_close);
    h.get_by_label("Voltar ao editor").click();
    h.step();
    assert!(!h.state().exit_when_saved);
    assert!(h.state().shutdown_started.is_none());
}
#[test]
fn workspace_close_cancels_unanswered_hover_and_reaps_provider_before_session_ack() {
    let (dir, a, _, mut h) = fixture();
    let log = dir.path().join("lsp.jsonl");
    start_provider(&mut h, &a, "hover", &log);
    wait(&mut h, |a| !a.service.is_busy());
    h.state_mut().send(Command::OpenFile(a.join("source.php")));
    wait(&mut h, |a| !a.service.is_busy());
    let d = h.state().service.active_document().unwrap();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::PreviewHover {
            id,
            expected_version,
            character_index: 7,
        })
        .unwrap();
    wait(&mut h, |_| {
        fs::read_to_string(&log)
            .unwrap()
            .contains("textDocument/hover")
    });
    h.state_mut().request_exit();
    let started = Instant::now();
    wait(&mut h, |a| a.allow_close);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "must not wait for the 10s hover timeout"
    );
    let first: serde_json::Value =
        serde_json::from_str(fs::read_to_string(log).unwrap().lines().next().unwrap()).unwrap();
    #[cfg(target_os = "linux")]
    assert!(!PathBuf::from(format!("/proc/{}", first["pid"].as_u64().unwrap())).exists());
}
#[test]
fn workspace_close_abandons_late_native_selection_and_closes_the_empty_welcome() {
    let (_dir, a, _, mut h) = fixture();
    let gate = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let opened = gate.clone();
    h.state_mut()
        .set_folder_picker(move |_| {
            while !opened.load(std::sync::atomic::Ordering::Acquire) {
                thread::sleep(Duration::from_millis(2));
            }
            Some(a.clone())
        })
        .unwrap();
    h.state_mut().open_native_folder();
    assert!(h.state().folder_picker_pending());
    h.state_mut().request_exit();
    gate.store(true, std::sync::atomic::Ordering::Release);
    wait(&mut h, |a| a.allow_close);
    assert!(h.state().service.workspace.is_none());
    assert!(!h.state().folder_picker_pending());
}
#[test]
fn workspace_close_dirty_exit_can_return_to_editor_without_saving_or_discarding() {
    let (_dir, a, _, mut h) = fixture();
    open(&mut h, &a);
    let id = edit(&mut h, &a);
    h.state_mut().request_exit();
    assert!(matches!(h.state().confirm, Some(Confirm::Exit)));
    h.step();
    h.step();
    assert!(!h.state().exit_when_saved);
    h.get_by_label("Voltar ao editor").click();
    h.step();
    assert!(!h.state().allow_close && h.state().confirm.is_none());
    assert!(
        h.state()
            .service
            .documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .is_dirty()
    );
    assert_eq!(
        fs::read_to_string(a.join("source.php")).unwrap(),
        "<?php\n$weather = 'saved';\n"
    );
}
#[test]
fn workspace_close_session_failure_requires_explicit_exit_without_overwriting_previous_path() {
    let (dir, a, _, mut h) = fixture();
    open(&mut h, &a);
    let session = dir.path().join("profile/session.json");
    fs::create_dir_all(&session).unwrap();
    h.state_mut().request_exit();
    wait(&mut h, |a| {
        matches!(a.confirm, Some(Confirm::ExitWithoutSession))
    });
    h.step();
    assert!(!h.state().allow_close);
    h.get_by_label("Sair sem salvar sessão").click();
    h.step();
    wait(&mut h, |a| a.allow_close);
    assert!(session.is_dir());
}

#[test]
fn workspace_close_reports_pending_categories_throttles_and_keeps_paths_private() {
    let (dir, a, _, mut h) = fixture();
    open(&mut h, &a);
    let messages = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let captured = messages.clone();
    let ctx = h.ctx.clone();
    set_diagnostic_sink(&ctx, move |message| {
        captured.lock().unwrap().push(message.to_owned())
    });
    h.state_mut()
        .service
        .language
        .dispatch(LanguageCommand::CancelHover)
        .unwrap();
    h.state_mut().request_exit();
    // Do not poll ACKs until the shutdown controller has observed the pending
    // work; this remains deterministic even if the worker has already replied.
    h.state_mut().shutdown_tick(&ctx);
    let first = messages.lock().unwrap().clone();
    assert_eq!(first.len(), 1);
    assert!(first[0].contains("providers de linguagem"));
    assert!(!first[0].contains(&dir.path().display().to_string()));
    assert!(!first[0].contains("source.php"));
    h.state_mut().shutdown_tick(&ctx);
    assert_eq!(
        messages.lock().unwrap().len(),
        1,
        "unchanged frame must not flood logs"
    );
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::unique("codisseu.shutdown.trace"),
            (first[0].clone(), Instant::now() - Duration::from_secs(3)),
        )
    });
    h.state_mut().shutdown_tick(&ctx);
    assert_eq!(
        messages.lock().unwrap().len(),
        2,
        "blocked shutdown repeats diagnostics periodically"
    );
    wait(&mut h, |app| app.allow_close);
    let captured = messages.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|line| line.contains("shutdown complete"))
    );
    assert!(
        captured
            .iter()
            .all(|line| !line.contains(&dir.path().display().to_string()))
    );
    assert!(dir.path().join("profile/session.json").is_file());
}
