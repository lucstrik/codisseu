//! Stop producers before awaiting acknowledgements, then persist and close.
use super::*;
use std::time::Instant;

const PROGRESS_DELAY: Duration = Duration::from_millis(750);

fn trace_id() -> egui::Id {
    egui::Id::unique("codisseu.shutdown.trace")
}

impl CodisseuApp {
    pub(super) fn shutdown_tick(&mut self, ctx: &egui::Context) {
        if !self.exit_when_saved || self.allow_close {
            self.shutdown_started = None;
            ctx.data_mut(|data| data.remove::<(String, Instant)>(trace_id()));
            return;
        }
        self.shutdown_started.get_or_insert_with(Instant::now);
        self.startup.clear();
        self.service.suspend_provider_startup();
        self.ai_flush_draft();
        ctx.request_repaint_after(Duration::from_millis(20));
        self.terminal.close();
        self.docker_terminal.close();
        self.service.docker.cancel();
        self.service.python_tests.cancel();
        self.service.python.cancel();
        self.service.php_tests.cancel();
        self.service.js_tests.cancel();
        self.service.api.cancel();
        self.service.api.stop_mock();
        self.service.stop_debugger();
        self.service.cancel_git_line_changes();
        self.service.cancel_shutdown_reads();
        self.service.ai.cancel_all();

        // A pending initialize/hover is itself busy. Waiting for global idle before
        // Stop leaves that request in charge of whether the application may exit.
        // Critical file/Git guards can refuse Stop; retry only until it is accepted.
        if !self.services_stopped && self.service.dispatch(Command::StopProviders).is_ok() {
            self.services_stopped = true;
        }
        if self.service.db.connected && !self.service.db.is_busy() {
            let _ = self
                .service
                .dispatch(Command::Database(codisseu_core::DbCommand::Disconnect));
        }
        if self.service.remote_db.active.is_some() && !self.service.remote_db.is_busy() {
            let _ = self.service.dispatch(Command::RemoteDatabase(
                codisseu_core::remote::RemoteCommand::Disconnect,
            ));
        }
        if self.shutdown_blockers().is_empty() {
            if !self.exit_without_session && !self.session_pending {
                match self.service.dispatch(Command::PersistSession) {
                    Ok(()) => self.session_pending = true,
                    Err(_) => {
                        self.exit_when_saved = false;
                        self.confirm = Some(Confirm::ExitWithoutSession);
                    }
                }
            } else if self.exit_without_session || self.service.last_error.is_none() {
                self.shutdown_finalizing = true;
                self.service.shutdown_browser();
                self.service.docker.shutdown();
                self.service.api.shutdown();
                if !self.service.api.is_busy()
                    && !self.service.browser.is_busy()
                    && !self.service.docker.is_busy()
                {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            } else {
                self.exit_when_saved = false;
                self.session_pending = false;
                self.confirm = Some(Confirm::ExitWithoutSession);
            }
        }
        let mut blockers = self.shutdown_blockers();
        if self.shutdown_finalizing && self.service.browser.is_busy() {
            blockers.push("navegador");
        }
        let message = format!(
            "shutdown {} stop_accepted={} session_pending={} finalizing={} blockers=[{}]",
            if self.allow_close {
                "complete"
            } else {
                "waiting"
            },
            self.services_stopped,
            self.session_pending,
            self.shutdown_finalizing,
            blockers.join(", "),
        );
        let now = Instant::now();
        let should_record = ctx.data_mut(|data| {
            let old = data.get_temp::<(String, Instant)>(trace_id());
            let changed = old.as_ref().is_none_or(|(text, at)| {
                text != &message || now.duration_since(*at) >= Duration::from_secs(2)
            });
            if changed {
                data.insert_temp(trace_id(), (message.clone(), now));
            }
            changed
        });
        if should_record {
            super::diagnostics::record(ctx, &message);
        }
    }

    fn shutdown_blockers(&self) -> Vec<&'static str> {
        let mut pending = self.service.shutdown_blockers();
        macro_rules! busy {
            ($condition:expr, $label:literal) => {
                if $condition && !pending.contains(&$label) {
                    pending.push($label);
                }
            };
        }
        busy!(!self.services_stopped, "pedido de parada dos providers");
        busy!(
            self.service.any_project_providers_running(),
            "processos de linguagem"
        );
        busy!(self.service.db.connected, "conexão SQLite");
        busy!(
            self.service.remote_db.active.is_some(),
            "conexão de banco remoto"
        );
        busy!(self.service.debugger_has_sessions(), "depurador");
        busy!(self.service.ai.any_running(), "execuções AI");
        busy!(
            self.service.git_commit_messages.any_running(),
            "geração de mensagem de commit"
        );
        busy!(self.git_insights.is_busy(), "histórico Git do editor");
        busy!(
            self.service.git_line_changes_pending(),
            "marcas de alteração Git"
        );
        busy!(self.service.python_tests.is_busy(), "testes Python");
        busy!(self.service.js_tests.is_busy(), "testes JavaScript");
        busy!(self.service.php_tests.is_busy(), "testes PHP");
        busy!(self.service.docker.is_busy(), "Docker");
        busy!(self.transfer_work_pending(), "transferência de dados");
        busy!(self.terminal.shutdown_pending(), "terminal");
        busy!(self.docker_terminal.shutdown_pending(), "terminal Docker");
        pending
    }

    pub(super) fn shutdown_window(&mut self, ctx: &egui::Context) {
        if !self.exit_when_saved || self.allow_close {
            return;
        }
        let elapsed = self
            .shutdown_started
            .map(|at| at.elapsed())
            .unwrap_or_default();
        if elapsed < PROGRESS_DELAY {
            ctx.request_repaint_after(PROGRESS_DELAY - elapsed);
            return;
        }
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("codisseu.shutdown"))
            .frame(self.tokens.dialog_frame(ctx))
            .show(ctx, |ui| {
                self.tokens
                    .dialog_header(ui, ctx, 420., "Fechando o Codisseu");
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Spinner::new()
                            .size(16.)
                            .color(color(self.tokens.muted)),
                    );
                    ui.label(if self.session_pending {
                        "Salvando a sessão e concluindo o fechamento…"
                    } else {
                        "Encerrando os serviços e aguardando gravações…"
                    });
                });
                let mut pending = self.shutdown_blockers();
                if self.shutdown_finalizing && self.service.browser.is_busy() {
                    pending.push("navegador");
                }
                if !pending.is_empty() {
                    ui.add_space(8.);
                    ui.label(
                        egui::RichText::new(format!("Aguardando: {}", pending.join(", ")))
                            .small()
                            .weak(),
                    );
                }
                if !self.shutdown_finalizing {
                    self.tokens.dialog_footer(ui, |ui| {
                        cancel = ui
                            .add(self.tokens.dialog_button("Voltar ao editor", false))
                            .clicked();
                    });
                }
            });
        if cancel {
            super::diagnostics::record(ctx, "shutdown cancelled by user");
            ctx.data_mut(|data| data.remove::<(String, Instant)>(trace_id()));
            self.exit_when_saved = false;
            self.shutdown_started = None;
            self.services_stopped = false;
            self.session_pending = false;
            self.focus_editor = true;
            self.service.status = "Fechamento cancelado; serviços de linguagem pausados".into();
        }
    }
}
