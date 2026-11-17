//! Real LSP + egui/WGPU quick-fix acceptance; no normal workspace/profile.
use super::*;
use codisseu_core::{ProviderConfig, ProviderState, Selection};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Instant};
fn wait(h: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < until,
            "status={} err={:?} fixes={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.code_actions.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn capture(h: &mut Harness<'_, CodisseuApp>, ext: &str, scale: f32, name: &str) {
    let Some(out) = std::env::var_os("CODISSEU_QUICK_FIX_CAPTURES") else {
        return;
    };
    let dir = PathBuf::from(out).join(format!("{ext}-{scale}"));
    fs::create_dir_all(&dir).unwrap();
    h.ctx.request_repaint();
    h.step();
    h.step();
    h.render()
        .unwrap()
        .save(dir.join(format!("{name}.png")))
        .unwrap();
}
#[test]
#[ignore = "real Phpactor/TS servers and WGPU; CODISSEU_PROVIDER_CONFIG required"]
fn php_js_ts_alt_enter_preview_apply_undo_save_keyboard_and_mouse() {
    for ext in ["php", "js", "ts"] {
        for scale in [1., 1.25] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("Projeto ação");
            fs::create_dir(&root).unwrap();
            fs::write(root.join("tsconfig.json"),"{\"compilerOptions\":{\"allowJs\":true,\"checkJs\":true,\"noEmit\":true,\"target\":\"ES2022\"},\"include\":[\"*\"]}").unwrap();
            fs::write(root.join("composer.json"), "{}").unwrap();
            let source = if ext == "php" {
                "<?php\r\nfunction answer(): int { $number = 42; /* ação 😀 */ return $numbre; }\r\n"
            } else {
                "export const emoji = \"ação 😀\"; export const answer = Math.rond(4.2);\r\n"
            };
            let path = root.join(format!("example.{ext}"));
            fs::write(&path, source).unwrap();
            let service = AppService::new(dir.path().join("profile/session.json"));
            let mut h = Harness::builder()
                .with_size([1280., 800.])
                .with_pixels_per_point(scale)
                .with_max_steps(200)
                .build_ui_state(
                    |ui, a: &mut CodisseuApp| a.show(ui),
                    CodisseuApp::new(service),
                );
            h.state_mut()
                .service
                .dispatch(Command::OpenWorkspace(root))
                .unwrap();
            wait(&mut h, |a| !a.service.is_busy());
            h.state_mut()
                .service
                .dispatch(Command::OpenFile(path.clone()))
                .unwrap();
            wait(&mut h, |a| !a.service.is_busy());
            let config: Vec<ProviderConfig> = serde_json::from_slice(
                &fs::read(
                    std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required"),
                )
                .unwrap(),
            )
            .unwrap();
            h.state_mut()
                .service
                .dispatch(Command::ConfigureProviders(
                    config
                        .into_iter()
                        .filter(|c| c.language == codisseu_core::Language::for_path(&path).unwrap())
                        .collect(),
                ))
                .unwrap();
            h.state_mut()
                .service
                .dispatch(Command::StartProviders)
                .unwrap();
            wait(&mut h, |a| {
                !a.service.is_busy()
                    && a.service
                        .language
                        .statuses
                        .iter()
                        .any(|s| s.state == ProviderState::Ready)
                    && !a.service.language.diagnostics.is_empty()
            });
            let needle = if ext == "php" { "$numbre" } else { "rond" };
            let at = source[..source.find(needle).unwrap() + 2].chars().count();
            let d = h.state().service.active_document().unwrap();
            let (id, version) = (d.id, d.version);
            h.state_mut()
                .service
                .dispatch(Command::SetSelection {
                    id,
                    expected_version: version,
                    selection: Selection::caret(at),
                })
                .unwrap();
            h.get_by_role(Role::MultilineTextInput).focus();
            h.step();
            h.step();
            capture(&mut h, ext, scale, "before");
            h.key_press_modifiers(Modifiers::ALT, Key::Enter);
            wait(&mut h, |a| a.service.code_actions.has_preview());
            assert!(h.query_by_label("Prévia da correção").is_some());
            assert_eq!(h.state().service.active_document().unwrap().text(), source);
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            let title = h.get_by_label("Prévia da correção").rect();
            assert!(title.left() >= 0. && title.right() <= 1280. && title.bottom() < 800.);
            if ext == "js" {
                let original_title = h.state().service.code_actions.title.clone();
                h.key_press(Key::ArrowDown);
                wait(&mut h, |a| {
                    a.service.code_actions.has_preview()
                        && a.service.code_actions.title != original_title
                });
                assert!(
                    h.state().service.code_actions.files[0]
                        .after
                        .contains("@ts-ignore")
                );
                h.key_press(Key::ArrowUp);
                wait(&mut h, |a| {
                    a.service.code_actions.has_preview()
                        && a.service.code_actions.title == original_title
                });
            }
            capture(&mut h, ext, scale, "preview");
            // Escape returns editor focus without any edit, then query again.
            h.key_press(Key::Escape);
            wait(&mut h, |a| !a.service.code_actions.is_open());
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), source);
            h.key_press_modifiers(Modifiers::ALT, Key::Enter);
            wait(&mut h, |a| a.service.code_actions.has_preview());
            if scale == 1. {
                h.key_press(Key::Enter);
            } else {
                h.get_by_label("Aplicar correção").click();
            }
            wait(&mut h, |a| {
                !a.service.code_actions.is_open() && !a.service.is_busy()
            });
            let fixed = h
                .state()
                .service
                .active_document()
                .unwrap()
                .text()
                .to_owned();
            assert!(
                if ext == "php" {
                    fixed.contains("return $number;")
                } else {
                    fixed.contains("Math.round(4.2)")
                },
                "{ext}: {fixed}"
            );
            assert!(fixed.contains("ação 😀"));
            assert!(!fixed.replace("\r\n", "").contains('\n'));
            assert_eq!(fs::read_to_string(&path).unwrap(), source);
            capture(&mut h, ext, scale, "applied");
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.step();
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), source);
            h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
            h.step();
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), fixed);
            h.key_press_modifiers(Modifiers::COMMAND, Key::S);
            wait(&mut h, |a| !a.service.is_busy());
            assert_eq!(fs::read_to_string(&path).unwrap(), fixed);
            h.state_mut()
                .service
                .dispatch(Command::StopProviders)
                .unwrap();
            wait(&mut h, |a| !a.service.is_busy());
        }
    }
}
