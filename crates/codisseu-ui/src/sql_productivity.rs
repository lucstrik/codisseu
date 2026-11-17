//! Bounded, session-only query history and editor actions over shared commands.
use super::*;
use codisseu_core::{
    EditKind, Selection, TextDelta, remote,
    sql_text::{self, SqlDialect},
};

const HISTORY_LIMIT: usize = 50;
const HISTORY_BYTES: usize = 1024 * 1024;
#[derive(Clone)]
struct Entry {
    source: String,
    name: String,
    sql: String,
    elapsed_ms: u128,
    rows: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(app: &mut CodisseuApp) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.service.poll();
            app.sync_database_ui();
            app.sql_productivity_tick();
            if !app.service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture() -> (tempfile::TempDir, CodisseuApp) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("query.sql");
        std::fs::write(&file, "SELECT 'á;🙂' AS value;\nSELECT 2 AS second;").unwrap();
        let db = dir.path().join("fixture.sqlite");
        rusqlite::Connection::open(&db).unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile.json")));
        settle(&mut app);
        for command in [
            Command::OpenFile(file),
            Command::Database(codisseu_core::DbCommand::Connect(db)),
        ] {
            app.service.dispatch(command).unwrap();
            settle(&mut app);
        }
        (dir, app)
    }
    #[test]
    fn sql_history_records_success_once_and_restore_is_undoable_without_execution() {
        let (dir, mut app) = fixture();
        app.execute_sql();
        settle(&mut app);
        assert!(
            app.service.db.last_error.is_none(),
            "{:?}",
            app.service.db.last_error
        );
        assert_eq!(app.sql_productivity.entries.len(), 1);
        let sql = app.sql_productivity.entries[0].sql.clone();
        assert_eq!(sql, "SELECT 'á;🙂' AS value;");
        for _ in 0..4 {
            app.sql_productivity_tick();
        }
        assert_eq!(app.sql_productivity.entries.len(), 1);
        let doc = app.service.active_document().unwrap();
        let (id, original, version) = (doc.id, doc.text().to_owned(), doc.version);
        let revision = app.service.db.result_revision();
        app.service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version,
                selection: Selection::caret(doc.len_chars()),
            })
            .unwrap();
        app.insert_history_sql(sql.clone());
        assert_eq!(
            app.service.active_document().unwrap().text(),
            format!("{original}{sql}")
        );
        assert!(!app.service.db.is_busy());
        assert_eq!(app.service.db.result_revision(), revision);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("query.sql")).unwrap(),
            original
        );
        app.service.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.service.active_document().unwrap().text(), original);
    }
    #[test]
    fn sql_statement_selection_and_explain_use_actual_sqlite_without_mutation() {
        let (_dir, mut app) = fixture();
        let doc = app.service.active_document().unwrap();
        app.service
            .dispatch(Command::SetSelection {
                id: doc.id,
                expected_version: doc.version,
                selection: Selection::caret(doc.len_chars()),
            })
            .unwrap();
        app.select_sql_statement();
        assert_eq!(app.selected_execution_sql().unwrap(), "SELECT 2 AS second;");
        app.explain_sql();
        settle(&mut app);
        assert!(
            app.service.db.last_error.is_none(),
            "{:?}",
            app.service.db.last_error
        );
        let result = app.service.db.result.as_ref().unwrap();
        assert!(result.columns.iter().any(|name| name == "detail"));
        assert!(!result.rows.is_empty());
        assert!(app.database_ui.sqlite_result_sql.is_none());
        assert!(app.sql_productivity.entries.is_empty());
    }
    #[test]
    fn sql_history_limits_and_workspace_change_clear_session_data() {
        let mut history = SqlProductivity::default();
        for i in 0..70 {
            history.push(Entry {
                source: "source".into(),
                name: "fixture".into(),
                sql: format!("SELECT {i}"),
                elapsed_ms: 1,
                rows: 1,
            });
        }
        assert_eq!(history.entries.len(), HISTORY_LIMIT);
        assert_eq!(history.entries[0].sql, "SELECT 69");
        for _ in 0..8 {
            history.push(Entry {
                source: "source".into(),
                name: "fixture".into(),
                sql: "x".repeat(256 * 1024),
                elapsed_ms: 1,
                rows: 1,
            });
        }
        assert!(
            history
                .entries
                .iter()
                .map(|e| e.sql.len() + e.source.len() + e.name.len())
                .sum::<usize>()
                <= HISTORY_BYTES
        );
        let (_dir, mut app) = fixture();
        app.sql_productivity = history;
        app.service.workspace = Some(PathBuf::from("other-project"));
        app.sql_productivity_tick();
        assert!(app.sql_productivity.entries.is_empty());
    }
    #[test]
    fn sql_history_waits_for_sqlite_ack_and_does_not_reimport_previous_workspace() {
        let (_dir, mut app) = fixture();
        app.execute_sql();
        settle(&mut app);
        assert_eq!(app.sql_productivity.entries.len(), 1);
        app.execute_sql();
        // Dispatch clears the displayed result before the worker acknowledgement.
        // Sync that intermediate frame deliberately, without polling the worker.
        app.sync_database_ui();
        app.sql_productivity_tick();
        assert!(app.database_ui.sqlite_result_sql.is_none());
        assert_eq!(
            app.database_ui.sqlite_pending_sql.as_deref(),
            Some("SELECT 'á;🙂' AS value;")
        );
        settle(&mut app);
        assert_eq!(app.sql_productivity.entries.len(), 2);
        assert_eq!(
            app.database_ui.sqlite_result_sql.as_deref(),
            Some("SELECT 'á;🙂' AS value;")
        );
        app.service.workspace = Some(PathBuf::from("different-project"));
        app.sql_productivity_tick();
        app.sql_productivity_tick();
        assert!(app.sql_productivity.entries.is_empty());
    }
    #[test]
    fn sql_explain_rejects_other_datasource_and_never_falls_back_to_sqlite() {
        let (_dir, mut app) = fixture();
        let path = app.service.active_document().unwrap().path.clone();
        app.service
            .remote_db
            .console_bindings
            .insert(path, "source-a".into());
        let revision = app.service.db.result_revision();
        app.explain_sql();
        assert!(
            app.service
                .remote_db
                .error
                .as_ref()
                .unwrap()
                .contains("vinculado")
        );
        assert!(!app.service.db.is_busy());
        app.service.remote_db.active = Some(remote::ConnectionConfig {
            id: "source-b".into(),
            ..Default::default()
        });
        app.explain_sql();
        assert!(!app.service.remote_db.is_busy());
        assert!(!app.service.db.is_busy());
        assert_eq!(app.service.db.result_revision(), revision);
    }
    #[test]
    fn sql_history_records_query_ack_instead_of_grid_revision() {
        let (_dir, mut app) = fixture();
        // Presentation-only fixture; no remote connection or server is claimed.
        app.service.remote_db.active = Some(remote::ConnectionConfig {
            id: "source-a".into(),
            ..Default::default()
        });
        app.service.remote_db.revision = 42;
        app.database_ui.query_ack_revision = Some(42);
        app.database_ui.result_sql = Some("SELECT 1".into());
        app.service.remote_db.result = Some(remote::RemoteResult {
            columns: vec!["id".into()],
            rows: vec![vec![remote::Cell::Number("1".into())]],
            truncated: false,
            elapsed_ms: 1,
            affected_rows: 0,
        });
        app.sql_productivity_tick();
        assert_eq!(app.sql_productivity.entries.len(), 1);
        app.service.remote_db.revision = 43;
        app.sql_productivity_tick();
        assert_eq!(app.sql_productivity.entries.len(), 1);
        app.service.workspace = Some(PathBuf::from("other-remote-project"));
        app.sql_productivity_tick();
        app.sql_productivity_tick();
        assert!(app.sql_productivity.entries.is_empty());
    }

    #[test]
    fn sql_history_and_its_dialog_do_not_follow_neighboring_workspace_files() {
        let (_dir, mut app) = fixture();
        let carrier = PathBuf::from("/workspace-carrier");
        app.service.workspace = Some(carrier.clone());
        app.service.workspace_file = Some(carrier.join("shipping.code-workspace"));
        app.sql_productivity_tick();
        app.sql_productivity.push(Entry {
            source: "shipping-source".into(),
            name: "Shipping database".into(),
            sql: "SELECT 'Shipping private query'".into(),
            elapsed_ms: 1,
            rows: 1,
        });
        app.sql_productivity.open = true;
        app.sql_productivity.filter = "shipping private query".into();
        app.service.workspace_file = Some(carrier.join("game.code-workspace"));
        app.sql_productivity_tick();
        assert_eq!(app.service.workspace, Some(carrier));
        assert!(app.sql_productivity.entries.is_empty());
        assert!(!app.sql_productivity.open);
        assert!(app.sql_productivity.filter.is_empty());
    }
}
#[derive(Default)]
pub(super) struct SqlProductivity {
    workspace: Option<PathBuf>,
    remote_revision: Option<(u64, Option<u64>)>,
    sqlite_revision: Option<u64>,
    entries: VecDeque<Entry>,
    open: bool,
    filter: String,
    selected: usize,
}
impl SqlProductivity {
    fn push(&mut self, entry: Entry) {
        if entry.sql.len() > HISTORY_BYTES {
            return;
        }
        self.entries.push_front(entry);
        while self.entries.len() > HISTORY_LIMIT
            || self
                .entries
                .iter()
                .map(|e| e.sql.len() + e.source.len() + e.name.len())
                .sum::<usize>()
                > HISTORY_BYTES
        {
            self.entries.pop_back();
        }
        self.selected = 0;
    }
}
impl CodisseuApp {
    fn sql_history_source(&self) -> String {
        if let Some(config) = &self.service.remote_db.active {
            format!(
                "remote:{}:{:?}:{}:{}:{}",
                config.id, config.engine, config.host, config.port, config.database
            )
        } else {
            format!("sqlite:{:?}", self.service.db.config)
        }
    }
    fn sql_editor(&self) -> bool {
        self.service.active_document().is_some_and(|d| {
            d.path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
        })
    }
    fn sql_dialect(&self) -> SqlDialect {
        match self.service.remote_db.active.as_ref().map(|c| c.engine) {
            Some(remote::Engine::Mysql | remote::Engine::AuroraMysql) => SqlDialect::Mysql,
            Some(remote::Engine::Postgres | remote::Engine::AuroraPostgres) => SqlDialect::Postgres,
            _ => SqlDialect::Sqlite,
        }
    }
    pub(super) fn sql_productivity_tick(&mut self) {
        let remote_key = (
            self.service.remote_db.connection_generation,
            self.database_ui.query_ack_revision,
        );
        let workspace = self.service.workspace_path().map(PathBuf::from);
        if self.sql_productivity.workspace != workspace {
            self.sql_productivity = SqlProductivity {
                workspace,
                remote_revision: Some(remote_key),
                sqlite_revision: self.database_ui.sqlite_query_ack_revision,
                ..Default::default()
            };
            return;
        }
        if self.sql_productivity.remote_revision != Some(remote_key) {
            self.sql_productivity.remote_revision = Some(remote_key);
            if let (Some(config), Some(result), Some(sql)) = (
                &self.service.remote_db.active,
                &self.service.remote_db.result,
                &self.database_ui.result_sql,
            ) && config.engine.is_sql()
                && remote_key.1.is_some()
            {
                self.sql_productivity.push(Entry {
                    source: self.sql_history_source(),
                    name: config.name.clone(),
                    sql: sql.clone(),
                    elapsed_ms: result.elapsed_ms,
                    rows: result.rows.len(),
                });
            }
        }
        let revision = self.database_ui.sqlite_query_ack_revision;
        if self.sql_productivity.sqlite_revision != revision {
            self.sql_productivity.sqlite_revision = revision;
            if let (Some(_), Some(result), Some(sql)) = (
                revision,
                &self.service.db.result,
                &self.database_ui.sqlite_result_sql,
            ) {
                self.sql_productivity.push(Entry {
                    source: format!("sqlite:{:?}", self.service.db.config),
                    name: "SQLite".into(),
                    sql: sql.clone(),
                    elapsed_ms: result.elapsed_ms,
                    rows: result.rows.len(),
                });
            }
        }
    }
    pub(super) fn sql_productivity_modal_open(&self) -> bool {
        self.sql_productivity.open
    }
    pub(super) fn sql_history_shortcut(&mut self, ctx: &egui::Context) -> bool {
        if self.sql_editor()
            && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::ALT, Key::E))
        {
            self.sql_productivity.open = true;
            return true;
        }
        false
    }
    pub(super) fn sql_productivity_toolbar(&mut self, ui: &mut egui::Ui) {
        if components::database_icon_button(ui, Icon::GitHistory, "Histórico de consultas", true)
            .on_hover_text("Ctrl+Alt+E · consultas concluídas nesta sessão")
            .clicked()
        {
            self.sql_productivity.open = true;
        }
        if components::database_icon_button(
            ui,
            Icon::Code,
            "Selecionar instrução SQL",
            self.sql_editor(),
        )
        .clicked()
        {
            self.select_sql_statement();
        }
        let available = self.sql_editor()
            && if self.service.remote_db.active.is_some() {
                self.service
                    .remote_db
                    .active
                    .as_ref()
                    .is_some_and(|c| c.engine.is_sql())
                    && !self.service.remote_db.is_busy()
            } else {
                self.service.db.connected && !self.service.db.is_busy()
            };
        if components::database_icon_button(ui, Icon::SqlPreview, "Explain Plan", available)
            .on_hover_text("Plano de execução da seleção ou instrução atual · sem ANALYZE")
            .clicked()
        {
            self.explain_sql();
        }
    }
    fn explain_sql(&mut self) {
        if !self.sql_editor() {
            return;
        }
        if !self.database_ui.staged.is_empty() {
            self.service.last_error =
                Some("Envie ou descarte as alterações da grade antes de solicitar o plano".into());
            return;
        }
        let sql = match self.selected_execution_sql() {
            Ok(sql) => sql,
            Err(error) => {
                self.service.last_error = Some(error);
                return;
            }
        };
        let path = self.service.active_document().map(|d| d.path.clone());
        let binding = match path
            .as_ref()
            .map(|p| self.service.remote_db.console_target(p))
        {
            Some(Ok(target)) => target.map(str::to_owned),
            Some(Err(error)) => {
                self.service.remote_db.error = Some(error);
                return;
            }
            None => None,
        };
        if let Some(binding) = binding
            && self
                .service
                .remote_db
                .active
                .as_ref()
                .is_none_or(|c| c.id != binding)
        {
            self.service.remote_db.error = Some(
                "Conecte o datasource vinculado a este console antes de solicitar o plano".into(),
            );
            return;
        }
        if let Some(config) = self.service.remote_db.active.as_ref() {
            let request = codisseu_core::transfer::TransferRequest {
                datasource_id: config.id.clone(),
                session_generation: self.service.remote_db.session_generation,
                payload: sql,
            };
            match self
                .service
                .dispatch(Command::RemoteDatabase(remote::RemoteCommand::Explain(
                    request,
                ))) {
                Ok(()) => {
                    self.database_ui.remote_results = true;
                    self.database_ui.pending_result_source = Some(path);
                    self.database_ui.pending_sql = None;
                }
                Err(error) => self.service.remote_db.error = Some(error),
            }
        } else if self.service.db.connected {
            match self
                .service
                .dispatch(Command::Database(codisseu_core::DbCommand::Execute {
                    sql: format!("EXPLAIN QUERY PLAN {sql}"),
                })) {
                Ok(()) => {
                    self.database_ui.remote_results = false;
                    self.database_ui.sqlite_pending_source = path;
                    self.database_ui.sqlite_pending_sql = None;
                }
                Err(error) => self.service.db.last_error = Some(error),
            }
        }
        self.database_ui.output = false;
        self.tool_tab = ToolTab::Results;
        self.show_activity = true;
    }
    fn select_sql_statement(&mut self) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let caret = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
            .map_or_else(|| doc.selection(), |v| v.selection())
            .head;
        match sql_text::statement_range(doc.text(), caret, self.sql_dialect()) {
            Ok(range) => {
                let command = Command::SetSelection {
                    id: doc.id,
                    expected_version: doc.version,
                    selection: Selection {
                        anchor: range.start,
                        head: range.end,
                    },
                };
                self.send(command);
                self.focus_editor = true;
            }
            Err(e) => self.service.last_error = Some(e),
        }
    }
    fn insert_history_sql(&mut self, text: String) {
        if !self.sql_editor() {
            return;
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let before = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
            .map_or_else(|| doc.selection(), |v| v.selection());
        let after = Selection::caret(before.range().start + text.chars().count());
        self.send(Command::EditTransaction {
            id: doc.id,
            expected_version: doc.version,
            delta: TextDelta {
                range: before.range(),
                insert: text,
            },
            kind: EditKind::Atomic,
            before,
            after,
        });
        self.sql_productivity.open = false;
        self.focus_editor = true;
    }
    pub(super) fn sql_productivity_window(&mut self, ctx: &egui::Context) {
        if !self.sql_productivity.open {
            return;
        }
        let source = self.sql_history_source();
        let can_insert = self.sql_editor();
        let mut close = false;
        let mut insert = None;
        egui::Modal::new(egui::Id::unique("sql.query.history"))
            .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(12))
            .show(ctx, |ui| {
                ui.set_width(700_f32.min(ctx.content_rect().width() - 64.));
                self.tokens.form_style(ui);
                ui.horizontal(|ui| {
                    ui.heading("Histórico de consultas");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = components::database_icon_button(
                            ui,
                            Icon::Close,
                            "Fechar histórico SQL",
                            true,
                        )
                        .clicked();
                        if components::database_icon_button(
                            ui,
                            Icon::Delete,
                            "Limpar histórico SQL",
                            true,
                        )
                        .clicked()
                        {
                            self.sql_productivity.entries.clear();
                        }
                    });
                });
                let state = &mut self.sql_productivity;
                ui.add(
                    egui::TextEdit::singleline(&mut state.filter)
                        .hint_text("Buscar consulta…")
                        .char_limit(256)
                        .desired_width(f32::INFINITY),
                );
                let filter = state.filter.to_lowercase();
                let entries: Vec<_> = state
                    .entries
                    .iter()
                    .filter(|e| {
                        e.source == source
                            && (filter.is_empty() || e.sql.to_lowercase().contains(&filter))
                    })
                    .cloned()
                    .collect();
                state.selected = state.selected.min(entries.len().saturating_sub(1));
                egui::ScrollArea::vertical()
                    .id_salt("sql.history.list")
                    .max_height(140.)
                    .codisseu()
                    .show(ui, |ui| {
                        if entries.is_empty() {
                            ui.weak("Nenhuma consulta concluída nesta fonte.");
                        }
                        for (index, entry) in entries.iter().enumerate() {
                            let caption: String = entry
                                .sql
                                .chars()
                                .take(240)
                                .collect::<String>()
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                                .chars()
                                .take(110)
                                .collect();
                            let response = ui.selectable_label(
                                state.selected == index,
                                RichText::new(caption).monospace(),
                            );
                            if response.clicked() {
                                state.selected = index;
                            }
                        }
                    });
                ui.separator();
                if let Some(entry) = entries.get(state.selected) {
                    ui.weak(format!(
                        "{} · {} ms · {} linhas carregadas",
                        entry.name, entry.elapsed_ms, entry.rows
                    ));
                    egui::ScrollArea::both()
                        .id_salt("sql.history.preview")
                        .max_height(220.)
                        .codisseu()
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(entry.sql.chars().take(8192).collect::<String>())
                                        .monospace(),
                                )
                                .selectable(true)
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        });
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                can_insert,
                                self.tokens.dialog_button("Inserir no editor", true),
                            )
                            .clicked()
                        {
                            insert = Some(entry.sql.clone());
                        }
                        if ui
                            .add(self.tokens.dialog_button("Copiar SQL", false))
                            .clicked()
                        {
                            ctx.copy_text(entry.sql.clone());
                        }
                    });
                }
                ui.add_space(6.);
                ui.weak("Histórico desta sessão · inserir preserva Undo e não executa a consulta.");
                close |= ui.input(|i| i.key_pressed(Key::Escape));
            });
        if close {
            self.sql_productivity.open = false;
            self.focus_editor = true;
        }
        if let Some(text) = insert {
            self.insert_history_sql(text);
        }
    }
}
