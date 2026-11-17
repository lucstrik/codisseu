use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};
type H = Harness<'static, CodisseuApp>;
const BASE: &str = "<?php\n// base ação 😀\r\n";
const LOCAL: &str = "<?php\n// editor ação 😀\r\n";
const DISK: &str = "<?php\n// externo ação 😀\r\n";

fn wait(h: &mut H, predicate: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        h.step();
        if predicate(h.state()) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(3));
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf, DocumentId, H) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("código.php");
    fs::write(&path, BASE).unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1000., 720.])
        .with_max_steps(40)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |app| {
        !app.service.is_busy() && app.service.active.is_some()
    });
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version,
            text: LOCAL.into(),
        })
        .unwrap();
    (dir, path, id, h)
}
fn conflict(h: &mut H, path: &std::path::Path, id: DocumentId) {
    fs::write(path, DISK).unwrap();
    // Exercise manual Save before the watcher has necessarily observed the write.
    h.state_mut().send(Command::Save { id });
    wait(h, |app| {
        app.service.document_external_change(id).is_some() && !app.service.document_save_pending(id)
    });
    h.step();
    h.step();
    assert!(h.query_by_label("O arquivo mudou no disco").is_some());
}
#[test]
fn external_changes_cancel_silences_same_revision_and_save_reopens_without_writing() {
    let (_dir, path, id, mut h) = fixture();
    conflict(&mut h, &path, id);
    h.get_by_label("Cancelar").click();
    h.step();
    for _ in 0..5 {
        h.step();
    }
    assert!(h.query_by_label("O arquivo mudou no disco").is_none());
    assert_eq!(fs::read_to_string(&path).unwrap(), DISK);
    assert_eq!(h.state().service.active_document().unwrap().text(), LOCAL);
    h.state_mut().send(Command::Save { id });
    wait(&mut h, |app| app.external_changes.active == Some(id));
    h.step();
    assert!(h.query_by_label("Sobrescrever").is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), DISK);
}
#[test]
fn external_changes_reload_uses_disk_and_overwrite_uses_editor() {
    for reload in [true, false] {
        let (_dir, path, id, mut h) = fixture();
        conflict(&mut h, &path, id);
        h.get_by_label(if reload {
            "Usar versão do disco"
        } else {
            "Sobrescrever"
        })
        .click();
        h.step();
        wait(&mut h, |app| {
            !app.service.document_save_pending(id)
                && app.service.document_external_change(id).is_none()
        });
        let expected = if reload { DISK } else { LOCAL };
        assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            expected
        );
        assert!(!h.state().service.active_document().unwrap().is_dirty());
    }
}
#[test]
fn external_changes_save_and_close_waits_for_conflict_decision_then_closes() {
    let (_dir, path, id, mut h) = fixture();
    fs::write(&path, DISK).unwrap();
    h.state_mut().request_close(id);
    h.step();
    h.step();
    // A watcher may deliver the conflict before the close dialog renders.
    if h.query_by_label("Salvar e fechar").is_some() {
        h.get_by_label("Salvar e fechar").click();
        h.step();
    } else {
        h.state_mut().send(Command::Save { id });
    }
    wait(&mut h, |app| app.external_changes.active == Some(id));
    h.step();
    h.step();
    h.get_by_label("Usar versão do disco").click();
    h.step();
    wait(&mut h, |app| {
        !app.service.document_save_pending(id) && app.service.document_external_change(id).is_none()
    });
    // When the external decision preceded Save, finish the retained close intent.
    if h.state().confirm.is_some() && h.state().service.active_document().is_some() {
        h.step();
        h.get_by_label("Salvar e fechar").click();
        h.step();
    }
    wait(&mut h, |app| {
        !app.service.documents.iter().any(|doc| doc.id == id)
    });
    assert_eq!(fs::read_to_string(&path).unwrap(), DISK);
}
#[test]
fn external_changes_close_save_and_explicit_discard_preserve_correct_bytes() {
    for discard in [true, false] {
        let (_dir, path, id, mut h) = fixture();
        h.state_mut().request_close(id);
        h.step();
        h.step();
        h.get_by_label(if discard {
            "Descartar e fechar"
        } else {
            "Salvar e fechar"
        })
        .click();
        h.step();
        wait(&mut h, |app| {
            !app.service.documents.iter().any(|doc| doc.id == id)
        });
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            if discard { BASE } else { LOCAL }
        );
    }
}
#[test]
fn external_changes_escape_keeps_both_contents_and_cancels_close() {
    let (_dir, path, id, mut h) = fixture();
    conflict(&mut h, &path, id);
    h.state_mut().request_exit();
    h.step();
    h.key_press(Key::Escape);
    h.step();
    assert!(h.state().confirm.is_none());
    assert!(!h.state().exit_when_saved);
    assert_eq!(h.state().service.active_document().unwrap().text(), LOCAL);
    assert_eq!(fs::read_to_string(&path).unwrap(), DISK);
}

#[test]
fn external_changes_defer_prompt_until_existing_path_dialog_closes() {
    let (_dir, path, id, mut h) = fixture();
    h.state_mut().open_path(PathAction::File);
    h.step();
    fs::write(&path, DISK).unwrap();
    h.state_mut().send(Command::Save { id });
    wait(&mut h, |app| {
        app.service.document_external_change(id).is_some() && !app.service.document_save_pending(id)
    });
    assert!(h.state().external_changes.active.is_none());
    assert!(h.query_by_label("Abrir arquivo").is_some());
    h.get_by_label("Cancelar").click();
    h.step();
    wait(&mut h, |app| app.external_changes.active == Some(id));
    h.step();
    assert!(h.query_by_label("O arquivo mudou no disco").is_some());
    assert_eq!(fs::read_to_string(path).unwrap(), DISK);
}

#[cfg(unix)]
#[test]
fn external_changes_failed_resolution_shows_error_inside_modal_and_allows_cancel() {
    let (_dir, path, id, mut h) = fixture();
    conflict(&mut h, &path, id);
    use std::os::unix::fs::PermissionsExt;
    let permissions = fs::metadata(&path).unwrap().permissions();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    h.get_by_label("Sobrescrever").click();
    h.step();
    wait(&mut h, |app| !app.service.document_save_pending(id));
    h.step();
    let error = h.state().service.last_error.as_ref().unwrap();
    let heading = h.get_by_label("O arquivo mudou no disco").rect();
    let cancel = h.get_by_label("Cancelar").rect();
    let overwrite = h.get_by_label("Sobrescrever").rect();
    assert!(
        h.get_all_by_label(error).any(|node| {
            let rect = node.rect();
            rect.top() > heading.bottom()
                && rect.bottom() < cancel.top()
                && rect.left() >= heading.left() - 1.
                && rect.right() <= overwrite.right() + 1.
        }),
        "resolution error must be visible inside the modal, above its actions"
    );
    h.get_by_label("Cancelar").click();
    h.step();
    assert!(h.state().external_changes.active.is_none());
    assert_eq!(h.state().service.active_document().unwrap().text(), LOCAL);
    assert_eq!(fs::read_to_string(&path).unwrap(), DISK);
    fs::set_permissions(&path, permissions).unwrap();
}

#[test]
fn external_changes_untitled_save_as_retains_close_intent_and_cancel_preserves_text() {
    for cancel in [true, false] {
        let (dir, _path, _id, mut h) = fixture();
        h.state_mut().send(Command::NewUntitled);
        h.step();
        let doc = h.state().service.active_document().unwrap();
        let (id, expected_version) = (doc.id, doc.version);
        h.state_mut().send(Command::Edit {
            id,
            expected_version,
            text: "untitled ação 😀\n".into(),
        });
        h.state_mut().request_close(id);
        h.step();
        h.step();
        h.get_by_label("Salvar e fechar").click();
        h.step();
        wait(&mut h, |app| app.path_dialog.is_some());
        h.step();
        h.step();
        assert!(h.query_by_label("Salvar arquivo como").is_some());
        let target = dir.path().join("novo arquivo.ts");
        if cancel {
            h.get_by_label("Cancelar").click();
            h.step();
            assert!(h.state().confirm.is_none());
            assert_eq!(
                h.state().service.active_document().unwrap().text(),
                "untitled ação 😀\n"
            );
            assert!(!target.exists());
        } else {
            h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Caminho")
                .click();
            h.step();
            h.key_press_modifiers(Modifiers::COMMAND, Key::A);
            h.event(egui::Event::Text(target.display().to_string()));
            h.step();
            h.get_by_label("Salvar arquivo").click();
            h.step();
            wait(&mut h, |app| {
                !app.service.documents.iter().any(|doc| doc.id == id)
            });
            assert_eq!(fs::read_to_string(target).unwrap(), "untitled ação 😀\n");
        }
    }
}
