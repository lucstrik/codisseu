//! Explicit, bounded language-server type choices. Server actions are never run.
use super::*;
impl CodisseuApp {
    pub(super) fn provider_choice_window(&mut self, ctx: &egui::Context) {
        let Some(choice) = self.service.language.server_choices.first() else {
            return;
        };
        // Ctrl-hover is a read-only preview and must never demand user input.
        if self.service.navigation.is_preview() {
            let _ = self
                .service
                .language
                .answer_server_choice(choice.token, None);
            return;
        }
        let mut selected = None;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique(("codisseu.provider.choice", choice.token)))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(12))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.).clamp(220., 560.));
                ui.label(egui::RichText::new("Escolher tipo para continuar").strong());
                egui::ScrollArea::vertical()
                    .max_height(320.)
                    .codisseu()
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(&choice.message).wrap());
                        ui.separator();
                        for (index, item) in choice.items.iter().enumerate() {
                            if ui
                                .add_sized([ui.available_width(), 24.], egui::Button::new(item))
                                .clicked()
                            {
                                selected = Some(index);
                            }
                        }
                    });
                cancel = ui.button("Cancelar navegação").clicked();
            });
        cancel |= consume_exact(ctx, Modifiers::NONE, Key::Escape);
        if cancel || selected.is_some() {
            if let Err(error) = self
                .service
                .language
                .answer_server_choice(choice.token, selected)
            {
                self.service.last_error = Some(error);
            }
            self.focus_editor = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::{Language, ProviderConfig, ProviderState};
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{fs, thread, time::Instant};
    fn wait(h: &mut Harness<CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            h.step();
            if check(h.state()) {
                h.step();
                return;
            }
            assert!(Instant::now() < end, "{:?}", h.state().service.last_error);
            thread::sleep(Duration::from_millis(3));
        }
    }
    fn setup() -> (
        tempfile::TempDir,
        Harness<'static, CodisseuApp>,
        PathBuf,
        PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let source = root.join("main.ts");
        let target = root.join("target.ts");
        for path in [&source, &target] {
            fs::write(path, "export const target = 42;\n").unwrap();
        }
        let mut service = AppService::new(dir.path().join("profile/session.json"));
        service
            .dispatch(Command::OpenWorkspace(root.clone()))
            .unwrap();
        let mut h = Harness::builder()
            .with_size([1100., 800.])
            .with_max_steps(100)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        wait(&mut h, |a| !a.service.is_busy());
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(source.clone()))
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
        h.state_mut()
            .service
            .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
                language: Language::TypeScript,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/../codisseu-language/tests/fixtures/server_choice.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    "normal".into(),
                    dir.path().join("protocol.jsonl").display().to_string(),
                    target.display().to_string(),
                ],
            }]))
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::StartProviders)
            .unwrap();
        wait(&mut h, |a| {
            a.service
                .language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
                && !a.service.is_busy()
        });
        (dir, h, source, target)
    }
    #[test]
    fn provider_type_choice_echoes_selection_and_preserves_source() {
        let (_dir, mut h, source, target) = setup();
        let d = h.state().service.active_document().unwrap();
        let (id, version) = (d.id, d.version);
        h.state_mut()
            .service
            .dispatch(Command::Definition {
                id,
                expected_version: version,
                character_index: 14,
            })
            .unwrap();
        wait(&mut h, |a| !a.service.language.server_choices.is_empty());
        assert!(h.query_by_label("Escolher tipo para continuar").is_some());
        h.key_press_modifiers(Modifiers::COMMAND, Key::N);
        h.step();
        assert_eq!(h.state().service.documents.len(), 1);
        h.get_by_label("Second").click();
        wait(&mut h, |a| {
            a.service
                .active_document()
                .is_some_and(|d| d.path == target)
                && !a.service.is_busy()
        });
        assert_eq!(
            fs::read_to_string(source).unwrap(),
            "export const target = 42;\n"
        );
        assert!(h.state().service.language.server_choices.is_empty());
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
    }
    #[test]
    fn hover_question_never_opens_modal_and_escape_cancels_explicit_question() {
        let (_dir, mut h, source, _target) = setup();
        let d = h.state().service.active_document().unwrap();
        let (id, version) = (d.id, d.version);
        h.state_mut()
            .service
            .dispatch(Command::PreviewDefinition {
                id,
                expected_version: version,
                character_index: 14,
            })
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
        assert!(h.query_by_label("Escolher tipo para continuar").is_none());
        assert_eq!(h.state().service.active_document().unwrap().path, source);
        h.state_mut()
            .service
            .dispatch(Command::Definition {
                id,
                expected_version: version,
                character_index: 14,
            })
            .unwrap();
        wait(&mut h, |a| !a.service.language.server_choices.is_empty());
        h.key_press(Key::Escape);
        wait(&mut h, |a| {
            a.service.language.server_choices.is_empty() && !a.service.is_busy()
        });
        assert_eq!(h.state().service.active_document().unwrap().path, source);
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
    }
}
