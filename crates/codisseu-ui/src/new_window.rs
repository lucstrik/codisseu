use super::*;

impl CodisseuApp {
    /// The native host owns process creation; widget tests can supply an adapter.
    pub fn configure_new_window_launcher(
        &mut self,
        launcher: impl FnMut() -> Result<(), String> + 'static,
    ) {
        self.new_window_launcher = Some(Box::new(launcher));
    }

    pub(super) fn open_new_window(&mut self) {
        if self.exit_when_saved {
            return;
        }
        let result = self
            .new_window_launcher
            .as_mut()
            .ok_or_else(|| "Nova janela indisponível neste host".to_owned())
            .and_then(|launcher| launcher());
        if let Err(error) = result {
            self.service.last_error = Some(format!("Não foi possível abrir nova janela: {error}"));
        }
    }

    pub(super) fn new_window_shortcut(&mut self, ctx: &egui::Context) -> bool {
        // Global even with terminal, browser or text-field focus. Consume repeats
        // too, without creating a process for each auto-repeat while N is held.
        let mut pressed = false;
        let mut consumed = false;
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key: Key::N,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } = event
                    && modifiers.matches_exact(Modifiers::COMMAND.plus(Modifiers::SHIFT))
                {
                    consumed = true;
                    pressed |= !repeat;
                    return false;
                }
                true
            });
        });
        if pressed {
            self.open_new_window();
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn shortcut_menu_repeats_and_failure_preserve_the_current_editor() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.send(Command::NewUntitled);
        let active = app.service.active;
        let launches = Rc::new(Cell::new(0));
        let count = launches.clone();
        app.configure_new_window_launcher(move || {
            count.set(count.get() + 1);
            Ok(())
        });
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        h.run();
        let modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
        h.event(egui::Event::Key {
            key: Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        h.run();
        assert_eq!(launches.get(), 1);
        assert_eq!(h.state().service.active, active);
        assert_eq!(h.state().service.documents.len(), 1);
        assert!(!h.state().project_search.open);
        h.event(egui::Event::Key {
            key: Key::N,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers,
        });
        h.run();
        assert_eq!(launches.get(), 1);
        h.event(egui::Event::Key {
            key: Key::N,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        });
        h.run();
        h.get_by_label("Menu principal").click();
        h.run();
        h.get_by_label("Arquivo").click();
        h.run();
        if let Some(path) = std::env::var_os("CODISSEU_NEW_WINDOW_CAPTURE") {
            h.render().unwrap().save(path).unwrap();
        }
        h.get_by_label("Nova janela      Ctrl+Shift+N").click();
        h.run();
        assert_eq!(launches.get(), 2);
        h.state_mut()
            .configure_new_window_launcher(|| Err("falha de processo de teste".into()));
        h.key_press_modifiers(modifiers, Key::N);
        h.run();
        assert!(
            h.state()
                .service
                .last_error
                .as_deref()
                .unwrap()
                .contains("falha de processo")
        );
        assert_eq!(h.state().service.active, active);
        // Signature help keeps Ctrl+P; double Shift remains the file-search gesture.
        h.state_mut().service.last_error = None;
        h.key_press_modifiers(Modifiers::COMMAND, Key::N);
        h.run();
        assert_eq!(h.state().service.documents.len(), 2);
        assert_eq!(launches.get(), 2);
    }
}
