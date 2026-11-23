use super::*;
use crate::components::{Icon, header, icon_button};
use codisseu_core::{DbCommand, ProviderState};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum ToolTab {
    PhpTests,
    JsTests,
    PythonTests,
    Terminal,
    Results,
    Problems,
    Activity,
    References,
    Rename,
    Git,
    Debug,
    Docker,
}

pub(super) struct SqlWidthCache {
    revision: u64,
    font_size: u32,
    pixels_per_point: u32,
    widths: Vec<f32>,
}

fn sql_widths<'a>(
    cache: &'a mut Option<SqlWidthCache>,
    database: &codisseu_core::DbService,
    ui: &mut egui::Ui,
    font_size: f32,
) -> &'a [f32] {
    let Some(revision) = database.result_revision() else {
        *cache = None;
        return &[];
    };
    let pixels_per_point = ui.ctx().pixels_per_point().to_bits();
    if cache.as_ref().is_none_or(|cached| {
        cached.revision != revision
            || cached.font_size != font_size.to_bits()
            || cached.pixels_per_point != pixels_per_point
    }) {
        let result = database.result.as_ref().expect("published result revision");
        let font = egui::FontId::monospace(font_size);
        // Measure the bounded result once per result/font/scale, never per repaint.
        let widths = ui.fonts_mut(|fonts| {
            result
                .columns
                .iter()
                .enumerate()
                .map(|(column, name)| {
                    let measure = |fonts: &mut egui::epaint::text::FontsView<'_>, text: &str| {
                        let mut width: f32 = 16.0;
                        for character in text.chars().take(120) {
                            let character = match character {
                                '\n' | '\r' | '\t' => ' ',
                                character => character,
                            };
                            width += fonts.glyph_width(&font, character);
                            if width >= 240.0 {
                                return 240.0;
                            }
                        }
                        width.ceil().clamp(163.0, 240.0)
                    };
                    let mut width = measure(fonts, name);
                    for row in &result.rows {
                        if width >= 240.0 {
                            break;
                        }
                        if let Some(value) = row.get(column) {
                            width = width.max(measure(fonts, &codisseu_core::display_value(value)));
                        }
                    }
                    width
                })
                .collect()
        });
        *cache = Some(SqlWidthCache {
            revision,
            font_size: font_size.to_bits(),
            pixels_per_point,
            widths,
        });
    }
    &cache.as_ref().expect("width cache refreshed").widths
}

impl CodisseuApp {
    /// One target for toolbar execution and Ctrl+Enter. The active editor view
    /// owns the directed scalar selection; the SQL lexer never rewrites text.
    pub(super) fn selected_execution_sql(&self) -> Result<String, String> {
        let doc = self
            .service
            .active_document()
            .ok_or_else(|| "Abra um console ou arquivo SQL".to_owned())?;
        let selection = self
            .service
            .active_view()
            .and_then(|id| self.service.view(id))
            .filter(|view| view.document == doc.id)
            .map_or_else(|| doc.selection(), |view| view.selection());
        let engine = self
            .service
            .remote_db
            .console_bindings
            .get(&doc.path)
            .and_then(|id| self.service.remote_db.profiles.iter().find(|c| c.id == *id))
            .or(self.service.remote_db.active.as_ref())
            .map(|config| config.engine);
        let range = if engine == Some(codisseu_core::remote::Engine::Mongo) {
            codisseu_core::mongo_text::execution_range(
                doc.text(),
                selection.head,
                (!selection.is_caret()).then(|| selection.range()),
            )?
        } else if engine.is_some_and(|engine| !engine.is_sql()) {
            if selection.is_caret() {
                0..doc.len_chars()
            } else {
                selection.range()
            }
        } else {
            use codisseu_core::{
                remote::Engine,
                sql_text::{self, SqlDialect},
            };
            let dialect = match engine {
                Some(Engine::Mysql | Engine::AuroraMysql) => SqlDialect::Mysql,
                Some(Engine::Postgres | Engine::AuroraPostgres) => SqlDialect::Postgres,
                _ => SqlDialect::Sqlite,
            };
            sql_text::execution_range(
                doc.text(),
                selection.head,
                (!selection.is_caret()).then(|| selection.range()),
                dialect,
            )?
        };
        Ok(doc
            .text()
            .chars()
            .skip(range.start)
            .take(range.len())
            .collect())
    }

    pub(super) fn execute_sql(&mut self) {
        if !self.database_ui.staged.is_empty() || self.database_ui.edit.is_some() {
            self.database_ui.edit_error = Some(
                "Envie ou descarte as alterações locais antes de executar outra consulta".into(),
            );
            return;
        }
        if self.service.remote_db.active.is_some()
            || self
                .service
                .active_document()
                .is_some_and(|d| self.service.remote_db.is_console_path(&d.path))
        {
            self.execute_remote();
            return;
        }
        if let Some(doc) = self.service.active_document() {
            // Keep the core command's document/version guard and scalar ranges.
            let selection = self
                .service
                .active_view()
                .and_then(|id| self.service.view(id))
                .map_or_else(|| doc.selection(), |view| view.selection());
            let range = match codisseu_core::sql_text::execution_range(
                doc.text(),
                selection.head,
                (!selection.is_caret()).then(|| selection.range()),
                codisseu_core::sql_text::SqlDialect::Sqlite,
            ) {
                Ok(range) => range,
                Err(error) => {
                    self.service.last_error = Some(error);
                    self.tool_tab = ToolTab::Results;
                    self.show_activity = true;
                    return;
                }
            };
            self.database_ui.sqlite_pending_source = Some(doc.path.clone());
            self.database_ui.sqlite_pending_sql = Some(
                doc.text()
                    .chars()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .collect(),
            );
            self.database_ui.output = false;
            self.send(Command::ExecuteSql {
                id: doc.id,
                expected_version: doc.version,
                selection: Some(range),
            });
            self.tool_tab = ToolTab::Results;
            self.show_activity = true;
        }
    }

    pub(super) fn database_panel(&mut self, ui: &mut egui::Ui) {
        self.remote_database_panel(ui);
    }
    pub(super) fn sqlite_database_panel(&mut self, ui: &mut egui::Ui) {
        if self.service.remote_db.active.is_some() || !self.database_ui.sqlite {
            return;
        }
        let connected = self.service.db.connected;
        let busy = self.service.db.is_busy();
        header(ui, self.tokens.database_toolbar_height, |ui| {
            ui.add_space(12.0);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(16., 16.), egui::Sense::hover());
            components::paint_database_engine(
                ui,
                rect,
                None,
                color(self.tokens.database_engine_mysql),
            );
            ui.label("SQLite");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.0);
                if connected {
                    ui.add_enabled_ui(!busy, |ui| {
                        if icon_button(
                            ui,
                            Icon::Disconnect,
                            "Desconectar",
                            false,
                            self.tokens.main_button_size,
                        )
                        .clicked()
                        {
                            self.send(Command::Database(DbCommand::Disconnect));
                        }
                        if icon_button(
                            ui,
                            Icon::Refresh,
                            "Atualizar schema",
                            false,
                            self.tokens.main_button_size,
                        )
                        .clicked()
                        {
                            self.send(Command::Database(DbCommand::RefreshSchema));
                        }
                    });
                    if icon_button(
                        ui,
                        Icon::File,
                        "Abrir SQL…",
                        false,
                        self.tokens.main_button_size,
                    )
                    .clicked()
                    {
                        self.open_path(PathAction::File);
                    }
                }
            });
        });
        if !connected || self.service.db.last_error.is_some() {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    self.tokens.form_style(ui);
                    if !connected {
                        let label = ui.label("Arquivo SQLite");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.database_input)
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id);
                        if ui
                            .add_enabled(
                                !busy && !self.database_input.trim().is_empty(),
                                self.tokens.dialog_button("Conectar SQLite", true),
                            )
                            .clicked()
                        {
                            self.send(Command::Database(DbCommand::Connect(PathBuf::from(
                                &self.database_input,
                            ))));
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new("SQLite · somente leitura")
                                    .color(color(self.tokens.muted)),
                            )
                            .wrap(),
                        );
                        ui.add(egui::Label::new(&self.service.db.status).wrap());
                    }
                    if let Some(error) = &self.service.db.last_error {
                        crate::status_messages::status_message(ui, error);
                    }
                });
        }
        if !connected {
            return;
        }
        use crate::components::TreeObjectRow;
        struct ObjectRow<'a> {
            id: egui::Id,
            parent: Option<egui::Id>,
            name: &'a str,
            depth: usize,
            expanded: Option<bool>,
            icon: Icon,
            detail: String,
            description: String,
        }
        let connection = self.service.db.config.as_ref();
        let name = connection
            .and_then(|p| p.file_name())
            .unwrap_or_default()
            .to_string_lossy();
        let state_id = egui::Id::unique(("codisseu.database.objects", connection));
        let connection_id = state_id.with("connection");
        let schema_id = connection_id.with(("schema", "main"));
        let expanded = |id: egui::Id| {
            ui.ctx()
                .data_mut(|data| data.get_temp::<bool>(id.with("expanded")).unwrap_or(true))
        };
        let connection_open = expanded(connection_id);
        let schema_open = expanded(schema_id);
        let mut rows = vec![ObjectRow {
            id: connection_id,
            parent: None,
            name: &name,
            depth: 0,
            expanded: Some(connection_open),
            icon: Icon::Database,
            detail: String::new(),
            description: format!(
                "SQLite · somente leitura\n{}",
                connection
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            ),
        }];
        if connection_open {
            // The adapter introspects main.sqlite_schema; this is its real namespace.
            rows.push(ObjectRow {
                id: schema_id,
                parent: Some(connection_id),
                name: "main",
                depth: 1,
                expanded: Some(schema_open),
                icon: Icon::Schema,
                detail: String::new(),
                description: "Schema main · catálogo SQLite introspectado".into(),
            });
            if schema_open {
                for table in &self.service.db.schema {
                    let table_id = schema_id.with(("table", &table.name));
                    let table_open = expanded(table_id);
                    rows.push(ObjectRow {
                        id: table_id,
                        parent: Some(schema_id),
                        name: &table.name,
                        depth: 2,
                        expanded: Some(table_open),
                        icon: Icon::Table,
                        detail: String::new(),
                        description: format!(
                            "main.{} · {} colunas",
                            table.name,
                            table.columns.len()
                        ),
                    });
                    if table_open {
                        for column in &table.columns {
                            let data_type = if column.data_type.is_empty() {
                                "tipo não declarado"
                            } else {
                                &column.data_type
                            };
                            rows.push(ObjectRow {
                                id: table_id.with(("column", &column.name)),
                                parent: Some(table_id),
                                name: &column.name,
                                depth: 3,
                                expanded: None,
                                icon: if column.primary_key {
                                    Icon::PrimaryKey
                                } else {
                                    Icon::Column
                                },
                                detail: format!(
                                    "{}{}",
                                    if column.primary_key { "PK · " } else { "" },
                                    data_type
                                ),
                                description: format!(
                                    "main.{}.{}\nTipo: {}\nChave primária: {}\n{}",
                                    table.name,
                                    column.name,
                                    data_type,
                                    if column.primary_key { "sim" } else { "não" },
                                    if column.nullable {
                                        "NULL permitido"
                                    } else {
                                        "NOT NULL"
                                    }
                                ),
                            });
                        }
                    }
                }
            }
        }
        let selection_id = state_id.with("selected");
        let mut selected = ui
            .ctx()
            .data_mut(|data| data.get_temp::<egui::Id>(selection_id));
        if !rows.iter().any(|row| Some(row.id) == selected) {
            selected = None;
        }
        let mut disconnect_sqlite = false;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(4.0);
            ui.ctx().accesskit_node_builder(ui.scope_id(), |node| {
                node.set_role(egui::accesskit::Role::Tree);
                node.set_label("Objetos de banco");
            });
            for (index, row) in rows.iter().enumerate() {
                let response = components::database_object_row(
                    ui,
                    row.id,
                    TreeObjectRow {
                        name: row.name,
                        detail: &row.detail,
                        description: &row.description,
                        depth: row.depth,
                        expanded: row.expanded,
                        selected: selected == Some(row.id),
                        icon: row.icon,
                    },
                    color(self.tokens.database_grid_foreground),
                    color(self.tokens.database_completion_key),
                )
                .on_hover_text(&row.description);
                if row.id == connection_id {
                    response.context_menu(|ui| {
                        if ui
                            .add_enabled(!busy, egui::Button::new("Desconectar"))
                            .clicked()
                        {
                            disconnect_sqlite = true;
                            ui.close();
                        }
                    });
                }
                let mut change_expansion = None;
                let mut focus = None;
                if response.clicked() {
                    selected = Some(row.id);
                    response.request_focus();
                    change_expansion = row.expanded.map(|open| !open);
                }
                if response.has_focus() {
                    let key = ui.input_mut(|input| {
                        [
                            Key::ArrowDown,
                            Key::ArrowUp,
                            Key::ArrowRight,
                            Key::ArrowLeft,
                            Key::Home,
                            Key::End,
                        ]
                        .into_iter()
                        .find(|key| input.consume_key(Modifiers::NONE, *key))
                    });
                    match key {
                        Some(Key::ArrowDown) => focus = rows.get(index + 1).map(|row| row.id),
                        Some(Key::ArrowUp) => {
                            focus = index.checked_sub(1).map(|index| rows[index].id)
                        }
                        Some(Key::Home) => focus = rows.first().map(|row| row.id),
                        Some(Key::End) => focus = rows.last().map(|row| row.id),
                        Some(Key::ArrowRight) => {
                            if row.expanded == Some(false) {
                                change_expansion = Some(true);
                            } else if row.expanded == Some(true) {
                                focus = rows
                                    .get(index + 1)
                                    .filter(|next| next.parent == Some(row.id))
                                    .map(|row| row.id);
                            }
                        }
                        Some(Key::ArrowLeft) => {
                            if row.expanded == Some(true) {
                                change_expansion = Some(false);
                            } else {
                                focus = row.parent;
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(open) = change_expansion {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(row.id.with("expanded"), open));
                    ui.ctx().request_repaint();
                }
                if let Some(id) = focus {
                    selected = Some(id);
                    ui.memory_mut(|memory| memory.request_focus(id));
                    ui.ctx().request_repaint();
                }
            }
            if let Some(selected) = selected {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(selection_id, selected));
            } else {
                ui.ctx()
                    .data_mut(|data| data.remove::<egui::Id>(selection_id));
            }
        });
        if disconnect_sqlite {
            self.send(Command::Database(DbCommand::Disconnect));
        }
    }

    pub(super) fn tool_panel(&mut self, ui: &mut egui::Ui) {
        if self.tool_tab == ToolTab::Terminal && !self.docker_terminal_active {
            self.terminal_reference_panel(ui);
            return;
        }
        ui.set_min_height(ui.available_height());
        self.tool_navigation(ui);
        egui::Frame::NONE
            .inner_margin(self.tokens.content_margin())
            .show(ui, |ui| {
                self.tool_panel_content(ui);
            });
    }

    pub(super) fn activate_tool_tab(&mut self, tab: ToolTab) {
        match tab {
            ToolTab::PhpTests => self.open_php_tests(false),
            ToolTab::JsTests => self.open_js_tests(false),
            ToolTab::PythonTests => self.open_python_tests(false),
            ToolTab::Terminal => self.open_terminal(),
            ToolTab::Git => {
                self.tool_tab = tab;
                if self.service.git.history.is_none() && !self.service.git.is_busy() {
                    self.git_request_history(true);
                }
            }
            _ => self.tool_tab = tab,
        }
    }
    fn tool_navigation(&mut self, ui: &mut egui::Ui) {
        self.tool_panel_header(ui);
    }

    fn tool_panel_content(&mut self, ui: &mut egui::Ui) {
        if matches!(
            self.tool_tab,
            ToolTab::PhpTests | ToolTab::JsTests | ToolTab::PythonTests
        ) {
            self.php_tests_panel(ui);
            return;
        }
        if self.tool_tab == ToolTab::Docker {
            self.docker_tool_panel(ui);
            return;
        }
        if self.tool_tab == ToolTab::Debug {
            self.debugger_panel(ui);
            return;
        }
        if self.tool_tab == ToolTab::Git {
            header(ui, self.tokens.tool_header_height, |ui| {
                ui.add_space(12.);
                ui.strong("Git");
                ui.add_space(12.);
                ui.selectable_value(&mut self.git_ui.tab, 0, "Log");
                ui.selectable_value(&mut self.git_ui.tab, 1, "Console");
                if components::database_icon_button(ui, Icon::Add, "New Log Tab", true).clicked() {
                    self.git_ui.log_branch.clear();
                    self.git_ui.filter.clear();
                    self.git_open_log();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if components::database_icon_button(ui, Icon::Remove, "Hide Git", true)
                        .clicked()
                    {
                        self.show_activity = false;
                    }
                    let busy = self.service.git.is_busy();
                    if busy {
                        if components::database_icon_button(
                            ui,
                            Icon::Stop,
                            "Cancel Git operation",
                            true,
                        )
                        .clicked()
                        {
                            self.git_ui.queue.clear();
                            self.send(Command::Git(codisseu_core::git::GitCommand::Cancel));
                        }
                    } else {
                        ui.allocate_exact_size(egui::vec2(26., 26.), egui::Sense::hover());
                    }
                    let feedback = if let Some(error) = &self.service.git.error {
                        Some((error.as_str(), self.tokens.error))
                    } else if !self.git_ui.notice.is_empty() {
                        Some((self.git_ui.notice.as_str(), self.tokens.muted))
                    } else if self.git_history_loading() {
                        Some(("Loading Git history…", self.tokens.muted))
                    } else if busy {
                        Some(("Updating Git…", self.tokens.muted))
                    } else {
                        None
                    };
                    if let Some((message, _tint)) = feedback {
                        crate::status_messages::status_message(ui, message).on_hover_text(message);
                    }
                });
            });
            self.git_log_panel(ui);
            return;
        }
        if self.tool_tab == ToolTab::Rename {
            self.rename_panel(ui);
            return;
        }
        if self.tool_tab == ToolTab::References {
            self.references_panel(ui);
            return;
        }
        if !matches!(
            self.tool_tab,
            ToolTab::Git
                | ToolTab::Debug
                | ToolTab::References
                | ToolTab::Terminal
                | ToolTab::Results
        ) && !self.service.language.statuses.is_empty()
        {
            ui.horizontal_wrapped(|ui| {
                for status in &self.service.language.statuses {
                    let ready = status.state == ProviderState::Ready;
                    let unversioned = self
                        .service
                        .language
                        .unversioned_paths
                        .iter()
                        .any(|p| codisseu_core::Language::for_path(p) == Some(status.language));
                    ui.label(
                        RichText::new(format!(
                            "{:?}: {}",
                            status.language,
                            if unversioned {
                                "publicação sem versão"
                            } else if ready {
                                "ativo"
                            } else {
                                status.detail.as_str()
                            }
                        ))
                        .color(if ready {
                            color(self.tokens.accent)
                        } else {
                            color(self.tokens.muted)
                        }),
                    );
                }
            });
        }
        match self.tool_tab {
            ToolTab::PhpTests | ToolTab::JsTests | ToolTab::PythonTests => self.php_tests_panel(ui),
            ToolTab::Docker => self.docker_tool_panel(ui),
            ToolTab::Debug => self.debugger_panel(ui),
            ToolTab::Git => self.git_log_panel(ui),
            ToolTab::Terminal => self.show_terminal_sessions(ui),
            ToolTab::References => self.references_panel(ui),
            ToolTab::Rename => self.rename_panel(ui),
            ToolTab::Results => self.database_services_panel(ui),
            ToolTab::Problems => {
                ui.label(format!(
                    "Problemas ({})",
                    self.service.language.diagnostics.len()
                ));
                ui.add_space(8.);
                if let Some(error) = &self.service.language.last_error {
                    crate::status_messages::status_message(ui, error);
                }
                if self.service.language.diagnostics.is_empty() {
                    ui.label(if self.service.providers_running() {
                        "Nenhum diagnóstico publicado para os documentos atuais."
                    } else {
                        "A análise inicia automaticamente em projetos confiáveis. Configure os providers ou autorize este projeto."
                    });
                }
                let mut jump = None;
                egui::ScrollArea::both()
                    .id_salt("codisseu.problems")
                    .codisseu()
                    .show(ui, |ui| {
                        for diagnostic in &self.service.language.diagnostics {
                            let name = diagnostic
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy();
                            let brief = diagnostic
                                .message
                                .lines()
                                .find(|line| !line.trim().is_empty())
                                .unwrap_or(&diagnostic.message);
                            let message = format!(
                                "{}  {}:{}  {}",
                                name,
                                diagnostic.start_line + 1,
                                diagnostic.start_character + 1,
                                brief
                            );
                            if ui
                                .selectable_label(
                                    false,
                                    RichText::new(message).color(if diagnostic.severity == 1 {
                                        color(self.tokens.error)
                                    } else {
                                        color(self.tokens.text)
                                    }),
                                )
                                .on_hover_text(&diagnostic.message)
                                .clicked()
                            {
                                jump = Some((
                                    diagnostic.path.clone(),
                                    diagnostic.start_line,
                                    diagnostic.start_character,
                                ));
                            }
                        }
                    });
                if let Some((path, line, character)) = jump {
                    self.send(Command::OpenFile(path.clone()));
                    self.jump_to = Some((path, line, character));
                    self.focus_editor = true;
                }
            }
            ToolTab::Activity => {
                crate::status_messages::status_message(ui, &self.service.status);
                ui.label("SQLite e diagnósticos usam workers. Nenhum script do projeto é iniciado ao abrir uma pasta.");
                ui.label(RichText::new("Use os painéis para acompanhar diagnósticos, consultas e sessões de depuração.").small().color(color(self.tokens.muted)));
            }
        }
    }

    fn database_result_grid(&mut self, ui: &mut egui::Ui) {
        if self.database_ui.remote_results {
            self.remote_results(ui);
            return;
        }
        ui.horizontal(|ui| {
            if self.service.db.result.is_none() {
                crate::status_messages::status_message(ui, &self.service.db.status);
            }
            if self.service.db.is_busy() && ui.button("Cancelar consulta").clicked() {
                self.send(Command::Database(DbCommand::Cancel));
            }
            if let Some(result) = &self.service.db.result {
                ui.label(RichText::new(format!(
                    "{} linhas · {} ms{}",
                    result.rows.len(),
                    result.elapsed_ms,
                    result
                        .limit_reason
                        .as_ref()
                        .map(|reason| format!(" · {reason}"))
                        .unwrap_or_default()
                )));
            }
        });
        ui.horizontal(|ui| {
            self.sql_productivity_toolbar(ui);
            self.query_navigation_toolbar(ui);
            self.result_transfer_toolbar(ui);
        });
        self.grid_edit_toolbar(ui);
        let edit_columns = self.grid_edit_columns();
        let binding_valid = self.grid_edit_binding_valid();
        let editable = self.can_edit_grid();
        let mut pending_edit = None;
        let mut sort_column = None;
        if let Some(error) = &self.service.db.last_error {
            crate::status_messages::status_message(ui, error);
        }
        if self.alternative_results(ui) {
            return;
        }
        self.grid_inspection_bar(ui);
        let columns = self.transfer_views.grid.visible_columns();
        let rows = self.transfer_views.grid.visible_rows();
        let sort_state = self.query_sort_state();
        let sort_available = !self.query_page_busy();
        let mut widths = sql_widths(
            &mut self.sql_width_cache,
            &self.service.db,
            ui,
            self.tokens.database_grid_font_size,
        )
        .to_vec();
        for (&column, &width) in &self.transfer_views.grid.widths {
            if let Some(w) = widths.get_mut(column) {
                *w = width;
            }
        }
        let total_width: f32 = columns.iter().filter_map(|c| widths.get(*c)).sum();
        let reveal = self.transfer_views.grid.take_scroll_target(
            &rows,
            &columns,
            &widths,
            self.tokens.database_grid_row_height,
        );
        if let Some(result) = &self.service.db.result {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let scroll = egui::ScrollArea::both()
                    .id_salt("codisseu.db.grid.scroll")
                    .auto_shrink([false, false]);
                let scroll = if let Some(reveal) = reveal {
                    scroll.scroll_offset(reveal)
                } else {
                    scroll
                };
                scroll.codisseu().show_rows(
                    ui,
                    self.tokens.database_grid_row_height,
                    rows.len() + 1,
                    |ui, visible| {
                        let viewport = ui.clip_rect();
                        // Keep the final column wholly visible after fractional-DPI scroll rounding.
                        ui.set_min_width(27. + total_width);
                        let row_height = self.tokens.database_grid_row_height;
                        let header_rect = egui::Rect::from_min_size(
                            egui::pos2(ui.cursor().left(), viewport.top()),
                            egui::vec2(27. + total_width, row_height),
                        );
                        // A reserved row keeps the scroll range equal
                        // to body height + fixed header. Data retain
                        // the one shared horizontal scroll offset.
                        if visible.start == 0 {
                            ui.allocate_space(header_rect.size());
                        }
                        ui.set_clip_rect(egui::Rect::from_min_max(
                            egui::pos2(
                                viewport.left(),
                                (viewport.top() + row_height).min(viewport.bottom()),
                            ),
                            viewport.max,
                        ));
                        egui::Grid::new("codisseu.db.grid")
                            .striped(false)
                            .start_row(visible.start)
                            .spacing(egui::Vec2::ZERO)
                            .min_col_width(25.0)
                            .min_row_height(self.tokens.database_grid_row_height)
                            .show(ui, |ui| {
                                for visual_index in visible.filter(|index| *index > 0) {
                                    let index = rows[visual_index - 1] + 1;
                                    let gutter = sql_cell(
                                        ui,
                                        &(self
                                            .database_ui
                                            .sqlite_page
                                            .as_ref()
                                            .map_or(0, |p| p.offset)
                                            + index as u64)
                                            .to_string(),
                                        25.0,
                                        false,
                                        [
                                            true,
                                            false,
                                            true,
                                            self.grid_selection.contains(index - 1, 0),
                                        ],
                                        &self.tokens,
                                    );
                                    if gutter.clicked() {
                                        self.grid_selection.row_clicked(
                                            index - 1,
                                            database_selection::click_modifiers(ui),
                                        );
                                        gutter.request_focus();
                                        self.grid_focus = Some(gutter.id);
                                    }
                                    for &col in &columns {
                                        let value = &result.rows[index - 1][col];
                                        let width = widths[col];
                                        let proposed = if binding_valid {
                                            self.database_ui.staged.get(&(index - 1, col))
                                        } else {
                                            None
                                        };
                                        let text = proposed
                                            .map(codisseu_core::remote::Cell::display)
                                            .unwrap_or_else(|| codisseu_core::display_value(value));
                                        let response = sql_cell(
                                            ui,
                                            &text,
                                            width,
                                            false,
                                            [
                                                matches!(
                                                    codisseu_core::value_type_name(value),
                                                    "INTEGER" | "REAL"
                                                ),
                                                codisseu_core::value_type_name(value) == "NULL",
                                                false,
                                                self.grid_selection.contains(index - 1, col),
                                            ],
                                            &self.tokens,
                                        )
                                        .on_hover_text(format!(
                                            "{}: {value:?}",
                                            codisseu_core::value_type_name(value)
                                        ));
                                        if proposed.is_some() {
                                            ui.painter().rect_stroke(
                                                response.rect.shrink(1.),
                                                0.,
                                                egui::Stroke::new(
                                                    1.,
                                                    color(self.tokens.database_dirty),
                                                ),
                                                egui::StrokeKind::Inside,
                                            );
                                        }
                                        database_cell_edit::inline_cell_editor(
                                            ui,
                                            &response,
                                            index - 1,
                                            col,
                                            edit_columns.get(col),
                                            &mut self.database_ui,
                                            &self.tokens,
                                        );
                                        if response.double_clicked()
                                            && editable
                                            && edit_columns
                                                .get(col)
                                                .is_some_and(database_cell_edit::editable_column)
                                        {
                                            pending_edit = Some((index - 1, col));
                                        }
                                        if self.transfer_views.grid.matches(index - 1, col) {
                                            ui.painter().rect_stroke(
                                                response.rect.shrink(1.),
                                                0.,
                                                egui::Stroke::new(1., color(self.tokens.accent)),
                                                egui::StrokeKind::Inside,
                                            );
                                        }
                                        if response.clicked() {
                                            self.database_ui.selected = Some((index - 1, col));
                                            self.grid_selection.clicked(
                                                index - 1,
                                                col,
                                                database_selection::click_modifiers(ui),
                                            );
                                            response.request_focus();
                                            self.grid_focus = Some(response.id);
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                        ui.set_clip_rect(viewport);
                        // Child painting does not advance the body
                        // cursor, and the header is never virtualized.
                        let mut heading = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("codisseu.sqlite.fixed-header")
                                .max_rect(header_rect),
                        );
                        heading.set_clip_rect(viewport.intersect(header_rect));
                        egui::Grid::new("codisseu.db.grid.header")
                            .spacing(egui::Vec2::ZERO)
                            .min_col_width(25.)
                            .min_row_height(row_height)
                            .show(&mut heading, |ui| {
                                sql_cell(
                                    ui,
                                    "#",
                                    25.,
                                    true,
                                    [false, false, true, false],
                                    &self.tokens,
                                );
                                for &col in &columns {
                                    let name = &result.columns[col];
                                    let width = widths[col];
                                    let response = sql_cell(
                                        ui,
                                        name,
                                        width,
                                        true,
                                        [false, false, false, false],
                                        &self.tokens,
                                    )
                                    .on_hover_text(name);
                                    let response = self.transfer_views.grid.header(
                                        ui,
                                        response,
                                        col,
                                        name,
                                        width,
                                        &self.tokens,
                                    );
                                    if database_navigation::sort_header(
                                        ui,
                                        response,
                                        col,
                                        name,
                                        sort_state,
                                        sort_available,
                                        &self.tokens,
                                    )
                                    .clicked()
                                    {
                                        sort_column = Some(col);
                                    }
                                }
                                ui.end_row();
                            });
                    },
                );
            });
        } else {
            ui.label(RichText::new("Conecte uma base e execute uma instrução em um arquivo SQL. Ctrl+Enter executa a seleção ou a instrução sob o cursor.").color(color(self.tokens.muted)));
        }
        if let Some((row, col)) = pending_edit {
            self.start_grid_edit(row, col);
        }
        if let Some(column) = sort_column {
            self.sort_query_column(column);
        }
    }

    fn database_services_panel(&mut self, ui: &mut egui::Ui) {
        let maximum = (ui.available_width() * 0.45).max(180.);
        let left = egui::Panel::left("database.services.tree")
            .default_size(360.)
            .size_range(180. ..=maximum)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| self.database_services_tree(ui));
        ui.painter().vline(
            left.response.rect.right(),
            left.response.rect.y_range(),
            egui::Stroke::new(1., color(self.tokens.border)),
        );
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let has_result = if self.database_ui.remote_results {
                    self.service.remote_db.result.is_some()
                } else {
                    self.service.db.result.is_some()
                };
                let output = self.database_ui.output;
                header(ui, self.tokens.database_toolbar_height, |ui| {
                    for (name, selected, show_output) in
                        [("Output", output, true), ("Result 1", !output, false)]
                    {
                        if name == "Result 1" && !has_result {
                            continue;
                        }
                        if database_result_tab(
                            ui,
                            name,
                            selected,
                            if show_output {
                                Icon::Terminal
                            } else {
                                Icon::Table
                            },
                            &self.tokens,
                        )
                        .clicked()
                        {
                            self.database_ui.output = show_output;
                        }
                    }
                    ui.add_space(8.);
                    // Auto-connect loads the catalog after the query completes.
                    // Its acknowledgement must not replace the query's result caption.
                    let query_status = self
                        .service
                        .remote_db
                        .result
                        .as_ref()
                        .filter(|r| {
                            self.database_ui.remote_results
                                && !output
                                && self.database_ui.result_sql.is_some()
                                && r.affected_rows == 0
                        })
                        .map(|r| {
                            format!(
                                "{} linhas · {} ms{}",
                                r.rows.len(),
                                r.elapsed_ms,
                                if r.truncated {
                                    " · resultado limitado"
                                } else {
                                    ""
                                }
                            )
                        });
                    let status = if self.database_ui.remote_results {
                        query_status
                            .as_deref()
                            .unwrap_or(&self.service.remote_db.status)
                    } else {
                        self.service.db.status.as_str()
                    };
                    if has_result {
                        ui.add(
                            egui::Label::new(
                                RichText::new(status)
                                    .small()
                                    .color(color(self.tokens.muted)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(status);
                    }
                });
                if !has_result && !output {
                    let error = if self.database_ui.remote_results {
                        &self.service.remote_db.error
                    } else {
                        &self.service.db.last_error
                    };
                    if let Some(error) = error {
                        if self.database_ui.remote_results {
                            self.query_execution_error(ui, error);
                        } else {
                            crate::status_messages::status_message(ui, error);
                        }
                    }
                    return;
                }
                if output {
                    if self.database_ui.remote_results && self.service.remote_db.is_busy() {
                        if components::database_icon_button(ui, Icon::Stop, "Stop", true).clicked()
                        {
                            self.send(Command::RemoteDatabase(
                                codisseu_core::remote::RemoteCommand::Cancel,
                            ));
                        }
                    } else if self.service.db.is_busy() && ui.button("Cancelar consulta").clicked()
                    {
                        self.send(Command::Database(DbCommand::Cancel));
                    }
                    egui::ScrollArea::both()
                        .id_salt("database.services.output")
                        .codisseu()
                        .show(ui, |ui| {
                            egui::Frame::NONE
                                .inner_margin(self.tokens.content_margin())
                                .show(ui, |ui| {
                                    if self.database_ui.remote_results {
                                        if let Some(c) = &self.service.remote_db.active {
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} · {}:{} · {}",
                                                    c.name,
                                                    c.host,
                                                    c.port,
                                                    if c.database.is_empty() {
                                                        "All databases"
                                                    } else {
                                                        &c.database
                                                    }
                                                ))
                                                .strong(),
                                            );
                                            ui.label(if c.read_only {
                                                "Read-only"
                                            } else if self.service.remote_db.transaction {
                                                "Tx: Manual · transação aberta"
                                            } else {
                                                "Tx: Manual"
                                            });
                                        }
                                        crate::status_messages::status_message(
                                            ui,
                                            &self.service.remote_db.status,
                                        );
                                        if let Some(e) = &self.service.remote_db.error {
                                            crate::status_messages::status_message(ui, e);
                                        }
                                        if let Some(r) = &self.service.remote_db.result {
                                            ui.label(format!(
                                                "{} rows · {} ms · {} affected{}",
                                                r.rows.len(),
                                                r.elapsed_ms,
                                                r.affected_rows,
                                                if r.truncated {
                                                    " · limite atingido"
                                                } else {
                                                    ""
                                                }
                                            ));
                                        }
                                    } else {
                                        if let Some(path) = &self.service.db.config {
                                            ui.label(path.display().to_string());
                                        }
                                        crate::status_messages::status_message(
                                            ui,
                                            &self.service.db.status,
                                        );
                                        if let Some(e) = &self.service.db.last_error {
                                            crate::status_messages::status_message(ui, e);
                                        }
                                    }
                                });
                        });
                } else {
                    self.database_result_grid(ui);
                }
            });
    }

    pub(super) fn providers_window(&mut self, ctx: &egui::Context) {
        self.workspace_trust_dialog(ctx);
        if !self.provider_dialog {
            return;
        }
        let mut open = true;
        let margin = self.tokens.providers_content_inset;
        let width =
            620_f32.min((ctx.content_rect().width() - 2. * f32::from(margin) - 32.).max(200.));
        let height = 600_f32.min((ctx.content_rect().height() - 160.).max(180.));
        egui::Window::new("Providers de linguagem")
            .id(egui::Id::unique("codisseu.providers.settings"))
            .open(&mut open)
            .collapsible(false)
            .default_width(width)
            .default_height(height)
            .default_pos(ctx.content_rect().center() - egui::vec2((width + 2. * f32::from(margin)) / 2., (height + 64.) / 2.))
            .max_width(width)
            .max_height((ctx.content_rect().height() - 80.).max(180.))
            .vscroll(true)
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(margin))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.spacing_mut().item_spacing.y = 12.;
                ui.add(egui::Label::new("PHP, TypeScript e Rust iniciam automaticamente nos projetos em que você confia.").wrap());
                if self.service.provider_startup().configured {
                    let mut automatic = self.service.provider_startup().enabled;
                    if ui.add_enabled(!self.service.is_busy(), egui::Checkbox::new(&mut automatic,
                        "Iniciar automaticamente em projetos confiáveis")).changed() {
                        self.send(Command::SetProvidersAutomatic(automatic));
                    }
                    ui.add(egui::Label::new(if self.service.provider_startup().trusted {
                        if self.service.provider_startup().suspended { "Projeto confiável; análise pausada até iniciar ou reabrir." }
                        else { "Projeto confiável; configuração autorizada neste perfil." }
                    } else { "Este projeto ou esta configuração ainda precisa de sua autorização." }).wrap());
                }
                ui.separator();
                ui.strong("Configuração dos providers");
                let project_root = self.service.language_project_root();
                if let Some(root) = &project_root { ui.add(egui::Label::new(format!("Projeto: {}", root.display())).wrap()); }
                ui.add(egui::Label::new("Cargo.toml, tsconfig.json, Composer e dependências são resolvidos na pasta deste projeto.").wrap());
                let label = ui.label("Arquivo de configuração");
                ui.add(egui::TextEdit::singleline(&mut self.provider_config_input)
                    .desired_width(f32::INFINITY)).labelled_by(label.id);
                if ui.add_enabled(!self.service.any_project_providers_running(),
                    self.tokens.dialog_button("Carregar configuração", false)).clicked() {
                    self.send(Command::LoadProviderConfig(PathBuf::from(&self.provider_config_input)));
                }
                if let Some(root) = &project_root
                    && ui.add_enabled(!self.service.is_busy() && !self.service.any_project_providers_running(), self.tokens.dialog_button("Carregar somente para este projeto", false)).clicked() {
                    self.send(Command::LoadProjectProviderConfig { root: root.clone(), path: PathBuf::from(&self.provider_config_input) });
                }
                let configs = project_root.as_ref().map_or_else(|| self.service.provider_configs.clone(), |root| self.service.project_provider_configs(root));
                for config in &configs {
                    egui::Frame::NONE
                        .inner_margin(self.tokens.content_margin())
                        .stroke(egui::Stroke::new(1., color(self.tokens.border)))
                        .corner_radius(self.tokens.control_corner_radius)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.strong(format!("{:?}", config.language));
                            ui.add(egui::Label::new(config.program.display().to_string()).wrap());
                            ui.add(egui::Label::new(RichText::new(config.args.join(" "))
                                .small().color(color(self.tokens.muted))).wrap());
                        });
                }
                ui.separator();
                ui.add(egui::Label::new("Iniciar autoriza estes executáveis para esta pasta. A autorização é lembrada fora do projeto; mudanças na configuração exigem nova autorização.").wrap());
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(!configs.is_empty() && !self.service.providers_running(),
                        self.tokens.dialog_button("Iniciar providers", true)).clicked() {
                        self.send(Command::StartProviders);
                        self.tool_tab = ToolTab::Problems;
                    }
                    if ui.add_enabled(self.service.any_project_providers_running(),
                        self.tokens.dialog_button("Parar providers", false)).clicked() {
                        self.send(Command::StopProviders);
                    }
                });
                for state in &self.service.language.statuses {
                    ui.add(egui::Label::new(format!("{:?}: {}", state.language, state.detail)).wrap());
                }
                for error in [&self.service.language.last_error, &self.service.provider_startup().error].into_iter().flatten() {
                    crate::status_messages::status_message(ui, error);
                }
            });
        self.provider_dialog = open;
    }
}

fn sql_cell(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    heading: bool,
    flags: [bool; 4],
    tokens: &Tokens,
) -> egui::Response {
    let [numeric, null, gutter, selected] = flags;
    let (id, rect) = ui.allocate_space(egui::vec2(width, tokens.database_grid_row_height));
    ui.painter().rect_filled(
        rect,
        0.,
        color(if selected {
            tokens.database_grid_selection
        } else if heading {
            tokens.database_grid_header
        } else {
            tokens.editor
        }),
    );
    let stroke = egui::Stroke::new(1., color(tokens.database_grid_separator));
    // Place the stroke wholly inside its cell. A stroke centered at bottom()
    // was covered by the following row's fill, erasing horizontal grid lines.
    let right = rect.right() - 0.5;
    let bottom = rect.bottom() - 0.5;
    ui.painter().line_segment(
        [
            egui::pos2(right, rect.top()),
            egui::pos2(right, rect.bottom()),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), bottom),
            egui::pos2(rect.right(), bottom),
        ],
        stroke,
    );
    let inset = if gutter {
        2.
    } else if heading {
        24.
    } else {
        8.
    };
    let right_inset = if gutter {
        2.
    } else if heading {
        40.
    } else {
        8.
    };
    if heading && !gutter {
        components::paint_database_icon(
            ui,
            egui::Rect::from_center_size(
                rect.left_center() + egui::vec2(11., 0.),
                egui::vec2(16., 16.),
            ),
            Icon::Column,
            color(tokens.database_grid_header_foreground),
        );
    }
    let label_rect = egui::Rect::from_min_max(
        rect.left_top() + egui::vec2(inset, 0.),
        rect.right_bottom() - egui::vec2(right_inset, 0.),
    );
    let response = ui.interact(rect, id, egui::Sense::click());
    let preview: String = text
        .chars()
        .take(120)
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let shown = if null {
        "<null>"
    } else if heading && gutter {
        ""
    } else {
        &preview
    };
    let (pos, align) = if numeric {
        (label_rect.right_center(), egui::Align2::RIGHT_CENTER)
    } else {
        (label_rect.left_center(), egui::Align2::LEFT_CENTER)
    };
    ui.painter().with_clip_rect(label_rect).text(
        pos,
        align,
        shown,
        egui::FontId::monospace(if gutter {
            tokens.database_row_header_font_size
        } else {
            tokens.database_grid_font_size
        }),
        color(if heading && !gutter {
            tokens.database_grid_header_foreground
        } else if null || gutter {
            tokens.database_grid_null
        } else {
            tokens.database_grid_foreground
        }),
    );
    // A screen reader keeps the typed value name, including NULL and full previews.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    response
}

fn database_result_tab(
    ui: &mut egui::Ui,
    name: &str,
    selected: bool,
    icon: Icon,
    tokens: &Tokens,
) -> egui::Response {
    let font = egui::FontId::proportional(12.);
    let galley = ui
        .painter()
        .layout_no_wrap(name.into(), font, color(tokens.text));
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(galley.size().x + 34., tokens.database_toolbar_height),
        egui::Sense::click(),
    );
    let tile = rect.shrink2(egui::vec2(2., 2.));
    if selected || response.hovered() {
        ui.painter().rect_filled(tile, 5., color(tokens.bar));
        if selected {
            ui.painter().rect_stroke(
                tile,
                5.,
                egui::Stroke::new(1., color(tokens.database_tab_border)),
                egui::StrokeKind::Inside,
            );
        }
    }
    components::paint_database_icon(
        ui,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(13., 0.),
            egui::vec2(14., 14.),
        ),
        icon,
        color(tokens.text),
    );
    ui.painter().galley(
        rect.left_center() + egui::vec2(25., -galley.size().y / 2.),
        galley,
        color(tokens.text),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, name));
    response
}
