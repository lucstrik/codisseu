//! Full query CSV export is explicit and keeps the bounded grid intact.
use super::*;
use codisseu_core::{
    DbCommand,
    query_export::{CsvExportOptions, QueryExportRequest},
    remote::RemoteCommand,
    transfer::TransferRequest,
};

#[derive(Clone)]
enum Target {
    Sqlite(PathBuf),
    Remote { id: String, generation: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(app: &mut CodisseuApp) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.service.poll();
            app.sync_database_ui();
            if !app.service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn query_export_ui_uses_executed_sql_and_preserves_grid_and_editor() {
        let dir = tempfile::tempdir().unwrap();
        let query = dir.path().join("query.sql");
        let sql = "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<2000) SELECT x FROM n;";
        std::fs::write(&query, sql).unwrap();
        let db = dir.path().join("fixture.sqlite");
        rusqlite::Connection::open(&db).unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile.json")));
        settle(&mut app);
        for command in [
            Command::OpenFile(query.clone()),
            Command::Database(DbCommand::Connect(db)),
        ] {
            app.service.dispatch(command).unwrap();
            settle(&mut app);
        }
        app.execute_sql();
        settle(&mut app);
        assert_eq!(app.service.db.result.as_ref().unwrap().rows.len(), 500);
        let revision = app.service.db.result_revision();
        let doc = app.service.active_document().unwrap();
        app.service
            .dispatch(Command::EditTransaction {
                id: doc.id,
                expected_version: doc.version,
                delta: codisseu_core::TextDelta {
                    range: 0..doc.len_chars(),
                    insert: "SELECT 99;".into(),
                },
                kind: codisseu_core::EditKind::Atomic,
                before: doc.selection(),
                after: codisseu_core::Selection::caret(0),
            })
            .unwrap();
        app.begin_query_export();
        assert_eq!(app.query_export_ui.sql, sql);
        app.query_export_ui.path = dir.path().join("full.csv").to_string_lossy().into();
        app.start_query_export();
        settle(&mut app);
        let report = app.service.db.export_report.as_ref().unwrap();
        assert_eq!(report.rows, 2000);
        let content = std::fs::read_to_string(&report.path).unwrap();
        assert_eq!(content.lines().count(), 2001);
        assert!(content.ends_with("2000\r\n"));
        assert_eq!(app.service.db.result_revision(), revision);
        assert_eq!(app.service.db.result.as_ref().unwrap().rows.len(), 500);
        assert_eq!(app.service.active_document().unwrap().text(), "SELECT 99;");
        assert_eq!(std::fs::read_to_string(query).unwrap(), sql);
    }
    #[test]
    fn query_export_ui_rejects_changed_session_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile.json")));
        settle(&mut app);
        app.query_export_ui = QueryExportUi {
            open: true,
            target: Some(Target::Sqlite(dir.path().join("missing.sqlite"))),
            sql: "SELECT 1".into(),
            path: dir.path().join("never.csv").to_string_lossy().into(),
            timeout_secs: 10,
            ..Default::default()
        };
        app.start_query_export();
        assert!(
            app.query_export_ui
                .error
                .as_ref()
                .unwrap()
                .contains("conexão mudou")
        );
        assert!(!dir.path().join("never.csv").exists());
        assert!(!app.service.db.is_busy());
    }
}
#[derive(Default)]
pub(super) struct QueryExportUi {
    open: bool,
    target: Option<Target>,
    sql: String,
    path: String,
    options: CsvExportOptions,
    timeout_secs: u64,
    running: bool,
    finished: bool,
    error: Option<String>,
}
impl CodisseuApp {
    pub(super) fn query_export_modal_open(&self) -> bool {
        self.query_export_ui.open
    }
    pub(super) fn can_export_query(&self) -> bool {
        self.result_query().is_some()
            && if self.database_ui.remote_results {
                self.service
                    .remote_db
                    .active
                    .as_ref()
                    .is_some_and(|c| c.engine.is_sql())
            } else {
                self.service.db.connected
            }
    }
    pub(super) fn begin_query_export(&mut self) {
        if !self.can_export_query() {
            return;
        }
        let target = if self.database_ui.remote_results {
            let Some(config) = self.service.remote_db.active.as_ref() else {
                return;
            };
            Target::Remote {
                id: config.id.clone(),
                generation: self.service.remote_db.session_generation,
            }
        } else {
            let Some(path) = self.service.db.config.clone() else {
                return;
            };
            Target::Sqlite(path)
        };
        self.query_export_ui = QueryExportUi {
            open: true,
            target: Some(target),
            sql: self.result_query().unwrap_or_default().into(),
            path: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("query.csv")
                .to_string_lossy()
                .into(),
            timeout_secs: 3600,
            ..Default::default()
        };
    }
    fn query_export_target_valid(&self) -> bool {
        match &self.query_export_ui.target {
            Some(Target::Sqlite(path)) => {
                self.service.db.connected && self.service.db.config.as_ref() == Some(path)
            }
            Some(Target::Remote { id, generation }) => {
                self.service
                    .remote_db
                    .active
                    .as_ref()
                    .is_some_and(|c| &c.id == id)
                    && *generation == self.service.remote_db.session_generation
            }
            None => false,
        }
    }
    fn start_query_export(&mut self) {
        if !self.query_export_target_valid() {
            self.query_export_ui.error =
                Some("A conexão mudou; reabra a exportação no resultado desejado.".into());
            return;
        }
        let state = &self.query_export_ui;
        let request = QueryExportRequest {
            sql: state.sql.clone(),
            path: state.path.clone().into(),
            options: state.options.clone(),
            timeout_secs: state.timeout_secs,
        };
        let command = match state.target.as_ref() {
            Some(Target::Sqlite(_)) => Command::Database(DbCommand::ExportQuery(request)),
            Some(Target::Remote { id, generation }) => {
                Command::RemoteDatabase(RemoteCommand::ExportQuery(TransferRequest {
                    datasource_id: id.clone(),
                    session_generation: *generation,
                    payload: request,
                }))
            }
            None => return,
        };
        match self.service.dispatch(command) {
            Ok(()) => {
                self.query_export_ui.running = true;
                self.query_export_ui.finished = false;
                self.query_export_ui.error = None;
            }
            Err(error) => self.query_export_ui.error = Some(error),
        }
    }
    pub(super) fn query_export_window(&mut self, ctx: &egui::Context) {
        if !self.query_export_ui.open {
            return;
        }
        let remote = matches!(self.query_export_ui.target, Some(Target::Remote { .. }));
        let busy = if remote {
            self.service.remote_db.is_busy()
        } else {
            self.service.db.is_busy()
        };
        let report = if remote {
            self.service.remote_db.export_report.clone()
        } else {
            self.service.db.export_report.clone()
        };
        if self.query_export_ui.running && !busy {
            self.query_export_ui.running = false;
            self.query_export_ui.finished = report.is_some();
            self.query_export_ui.error = if remote {
                self.service.remote_db.error.clone()
            } else {
                self.service.db.last_error.clone()
            };
        }
        let valid = self.query_export_target_valid();
        let pending =
            remote && (self.service.remote_db.transaction || !self.database_ui.staged.is_empty());
        let progress = if remote {
            self.service.remote_db.export_progress.snapshot()
        } else {
            self.service.db.export_progress.snapshot()
        };
        let mut start = false;
        let mut close = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("database.query.export"))
            .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(12))
            .show(ctx, |ui| {
                ui.set_width(680_f32.min(ctx.content_rect().width()-64.));
                self.tokens.form_style(ui);
                ui.horizontal(|ui| {
                    ui.heading("Exportar consulta completa");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { close = components::database_icon_button(ui, Icon::Close, "Fechar exportação completa", !self.query_export_ui.running).clicked(); });
                });
                ui.label("CSV · todas as linhas retornadas pela consulta");
                ui.add_space(8.);
                ui.add_enabled_ui(!self.query_export_ui.running, |ui| {
                    let state = &mut self.query_export_ui;
                    ui.horizontal(|ui| { let label = ui.label("Arquivo:"); ui.add(egui::TextEdit::singleline(&mut state.path).id_salt("query.export.path").desired_width(f32::INFINITY)).labelled_by(label.id); });
                    ui.horizontal(|ui| {
                        ui.label("Separador:");
                        egui::ComboBox::from_id_salt("query.export.delimiter").selected_text(match state.options.delimiter { b';' => "Ponto e vírgula", b'\t' => "Tab", b'|' => "Pipe", _ => "Vírgula" }).show_ui(ui, |ui| {
                            for (delimiter, name) in [(b',', "Vírgula"), (b';', "Ponto e vírgula"), (b'\t', "Tab"), (b'|', "Pipe")] { ui.selectable_value(&mut state.options.delimiter, delimiter, name); }
                        });
                        ui.label("NULL:"); ui.add(egui::TextEdit::singleline(&mut state.options.null_marker).char_limit(128).desired_width(65.));
                    });
                    ui.checkbox(&mut state.options.header, "Incluir nomes das colunas");
                    ui.checkbox(&mut state.options.spreadsheet_safe, "Proteger fórmulas em planilhas");
                    ui.horizontal(|ui| { ui.label("Prazo máximo (segundos):"); ui.add(egui::DragValue::new(&mut state.timeout_secs).range(1..=3600)); });
                });
                ui.add_space(8.);
                egui::ScrollArea::both().id_salt("query.export.sql").max_height(130.).codisseu().show(ui, |ui| { ui.add(egui::Label::new(RichText::new(self.query_export_ui.sql.chars().take(8192).collect::<String>()).monospace()).wrap_mode(egui::TextWrapMode::Extend)); });
                ui.add(egui::Label::new(RichText::new("Reexecuta a consulta nos dados atuais. LIMIT explícito é preservado. Não inclui alterações locais da grade.").small().color(color(self.tokens.muted))).wrap());
                if pending { crate::status_messages::status_message(ui, "Confirme ou reverta a transação e as alterações da grade antes de exportar."); }
                if !valid { crate::status_messages::status_message(ui, "A conexão mudou. Feche e reabra a exportação."); }
                if self.query_export_ui.running {
                    ui.horizontal(|ui| { ui.spinner(); ui.label(format!("{} linhas · {:.1} MiB", progress.rows, progress.bytes as f64 / 1048576.)); });
                    ctx.request_repaint_after(Duration::from_millis(100));
                } else if self.query_export_ui.finished && let Some(report) = &report {
                    ui.label(format!("{} linhas exportadas · {:.1} MiB · {} ms", report.rows, report.bytes as f64 / 1048576., report.elapsed_ms));
                }
                if let Some(error) = &self.query_export_ui.error { crate::status_messages::status_message(ui, error); }
                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.query_export_ui.running { cancel = ui.add(self.tokens.dialog_button("Cancelar exportação", false)).clicked(); }
                    else {
                        start = ui.add_enabled(valid && !busy && !pending, self.tokens.dialog_button("Exportar CSV", true)).clicked();
                        close |= ui.add(self.tokens.dialog_button("Fechar", false)).clicked();
                    }
                });
                if !self.query_export_ui.running { close |= ui.input(|i| i.key_pressed(Key::Escape)); }
            });
        if start {
            self.start_query_export();
        }
        if cancel {
            self.send(if remote {
                Command::RemoteDatabase(RemoteCommand::Cancel)
            } else {
                Command::Database(DbCommand::Cancel)
            });
        }
        if close {
            self.query_export_ui.open = false;
        }
    }
}
