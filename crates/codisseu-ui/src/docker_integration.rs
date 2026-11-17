//! Shell/tool-window integration; no Docker I/O runs on the render thread.
use super::*;
impl CodisseuApp {
    pub(super) fn docker_tick(&mut self, ctx: &egui::Context) {
        // A shell request waits for cancellation/ACK of an automatic read; never
        // race its captured selection with a worker replacing the whole state.
        if self.docker_ui.pending_shell.is_some() {
            self.service.docker.interrupt_background_refresh();
        }
        if !self.service.docker.is_background_refresh()
            && let Some((id, name, shell, context)) = self.docker_ui.pending_shell.take()
        {
            let result = if self.service.docker.state.context.as_deref() != Some(context.as_str()) {
                Err("O contexto Docker mudou; selecione o container novamente".into())
            } else if self.exit_when_saved || self.confirm.is_some() {
                Err("O aplicativo está encerrando ou aguardando confirmação".into())
            } else {
                self.service.docker.exec_spec(&id, &shell)
            };
            match result {
                Ok(spec) => {
                    let cwd = self
                        .service
                        .workspace
                        .clone()
                        .unwrap_or_else(|| PathBuf::from("."));
                    let key = format!("{:?}:{}:{}:{}", spec.program, spec.context, spec.id, shell);
                    let label = format!("Docker · {name} · {} · {shell}", spec.context);
                    if self
                        .docker_terminal
                        .open_command(&cwd, spec.program, spec.args, label, key)
                    {
                        self.docker_terminal_active = true;
                        self.tool_tab = ToolTab::Terminal;
                        self.show_activity = true;
                        self.focus_editor = false;
                    } else {
                        self.service.docker.state.error = Some("Limite de sessões do terminal Docker atingido; feche uma sessão antes de abrir outra".into());
                    }
                }
                Err(error) => self.service.docker.state.error = Some(error),
            }
        }
        if ((self.docker_ui.open && self.show_project)
            || (self.show_activity && self.tool_tab == ToolTab::Docker))
            && !self.exit_when_saved
        {
            ctx.request_repaint_after(Duration::from_secs(1));
            if self.service.docker.is_busy() {
                self.docker_last_refresh = std::time::Instant::now();
            } else if self.service.docker.state.connected
                && self.docker_last_refresh.elapsed() >= Duration::from_secs(5)
            {
                self.docker_last_refresh = std::time::Instant::now();
                if let Err(error) = self.service.docker.refresh_in_background() {
                    self.service.docker.state.error = Some(error);
                }
            }
        }
    }
    fn close_docker_terminals(&mut self) {
        self.docker_terminal.close();
        self.docker_terminal_active = false;
        if self.terminal.has_sessions() {
            self.terminal.request_focus();
        } else {
            self.show_activity = false;
            self.focus_editor = true;
        }
    }
    pub(super) fn show_terminal_sessions(&mut self, ui: &mut egui::Ui) {
        if self.docker_terminal.has_sessions() {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!self.docker_terminal_active, "Local")
                    .clicked()
                {
                    self.open_local_terminal();
                }
                if ui
                    .selectable_label(self.docker_terminal_active, "Docker")
                    .clicked()
                {
                    self.docker_terminal_active = true;
                    self.docker_terminal.request_focus();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.docker_terminal_active
                        && icon_button(ui, Icon::Close, "Encerrar terminais Docker", false, 24.)
                            .clicked()
                    {
                        self.close_docker_terminals();
                    }
                });
            });
            if self.docker_terminal_active {
                ui.add(
                    egui::Label::new(
                        RichText::new(self.docker_terminal.session_label())
                            .weak()
                            .size(11.),
                    )
                    .truncate(),
                );
                self.docker_terminal.show(ui);
                return;
            }
        }
        self.terminal.show_content(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_cannot_start_during_exit_or_its_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.exit_when_saved = true;
        app.open_terminal();
        app.open_local_terminal();
        assert!(!app.terminal.has_sessions());
        app.exit_when_saved = false;
        app.confirm = Some(Confirm::ExitWithoutSession);
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: Key::F12,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::ALT,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.shortcuts(ui.ctx()));
        output.textures_delta.clear();
        app.open_terminal();
        app.open_local_terminal();
        assert!(!app.terminal.has_sessions());
    }
    #[cfg(unix)]
    #[test]
    fn returning_to_terminal_preserves_the_container_session_without_launching_local() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.docker_terminal.open_command(
            dir.path(),
            "/bin/sh".into(),
            vec!["-c".into(), "read hold".into()],
            "Docker · fixture".into(),
            "own-session".into(),
        );
        app.docker_terminal_active = true;
        app.show_activity = false;
        app.open_terminal();
        assert!(app.docker_terminal_active);
        assert!(app.show_activity);
        assert!(app.tool_tab == ToolTab::Terminal);
        assert!(!app.terminal.has_sessions());
        assert_eq!(app.docker_terminal.session_label(), "Docker · fixture");
        app.docker_terminal.close();
    }
    #[test]
    fn closing_container_terminals_never_launches_an_unrequested_host_shell() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.docker_terminal_active = true;
        app.show_activity = true;
        app.close_docker_terminals();
        assert!(!app.terminal.has_sessions());
        assert!(!app.docker_terminal.has_sessions());
        assert!(!app.show_activity);
        assert!(app.focus_editor);
    }
    #[test]
    fn shell_intent_cannot_cross_contexts_or_replace_the_local_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.service.docker.state.context = Some("new-context".into());
        app.docker_ui.pending_shell = Some((
            "a".repeat(64),
            "own".into(),
            "sh".into(),
            "old-context".into(),
        ));
        app.docker_tick(&egui::Context::default());
        assert!(
            app.service
                .docker
                .state
                .error
                .as_ref()
                .unwrap()
                .contains("contexto")
        );
        assert!(!app.docker_terminal.has_sessions());
        assert!(!app.terminal.has_sessions());
        assert!(!app.docker_terminal_active);
    }
    #[test]
    fn exit_refuses_new_container_sessions_without_starting_a_client() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        app.service.docker.state.context = Some("own".into());
        app.exit_when_saved = true;
        app.docker_ui.pending_shell =
            Some(("a".repeat(64), "own".into(), "sh".into(), "own".into()));
        app.docker_tick(&egui::Context::default());
        assert!(
            app.service
                .docker
                .state
                .error
                .as_ref()
                .unwrap()
                .contains("encerrando")
        );
        assert!(!app.docker_terminal.has_sessions());
        app.exit_when_saved = false;
        app.confirm = Some(Confirm::Exit);
        app.docker_ui.pending_shell =
            Some(("a".repeat(64), "own".into(), "sh".into(), "own".into()));
        app.docker_tick(&egui::Context::default());
        assert!(
            app.service
                .docker
                .state
                .error
                .as_ref()
                .unwrap()
                .contains("confirmação")
        );
        assert!(!app.docker_terminal.has_sessions());
    }
}
