//! One schema-driven cell editor for local and remote result grids.
use super::*;
use codisseu_core::{
    DbCommand,
    cell_edit::{self, CellEditorKind},
    remote::{Cell, CellEdit, EditBatch, RemoteColumn, RemoteCommand},
};

impl CodisseuApp {
    fn grid_edit_identity(&self) -> Option<String> {
        if self.database_ui.remote_results {
            let source = self.service.remote_db.active.as_ref()?;
            self.service.remote_db.result.as_ref()?;
            Some(format!(
                "remote:{}:{}:{}:{}:{}:{}:{}",
                self.service.remote_db.connection_generation,
                source.id,
                source.host,
                source.port,
                source.database,
                source.read_only,
                self.service.remote_db.revision
            ))
        } else {
            Some(format!(
                "sqlite:{}:{}",
                self.service.db.config.as_ref()?.display(),
                self.service.db.result_revision()?
            ))
        }
    }
    pub(super) fn grid_edit_binding_valid(&self) -> bool {
        self.database_ui
            .edit_binding
            .as_ref()
            .is_none_or(|binding| self.grid_edit_identity().as_ref() == Some(binding))
    }
    pub(super) fn grid_edit_budget_valid(&self) -> bool {
        let Some(data) = self.result_snapshot() else {
            return false;
        };
        if self.database_ui.staged.len() > 500 {
            return false;
        }
        let size = |cell: &Cell| match cell {
            Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => s.len(),
            Cell::Binary(bytes) => bytes.len(),
            _ => 8,
        };
        let mut bytes = 0usize;
        for (&(row, _), value) in &self.database_ui.staged {
            let Some(original) = data.rows.get(row) else {
                return false;
            };
            bytes = bytes
                .saturating_add(size(value))
                .saturating_add(original.iter().map(size).sum::<usize>());
            if bytes > 4 * 1024 * 1024 {
                return false;
            }
        }
        true
    }
    pub(super) fn grid_edit_columns(&self) -> Vec<RemoteColumn> {
        let origin = if self.database_ui.remote_results {
            &self.service.remote_db.origin
        } else {
            &self.service.db.origin
        };
        origin
            .as_ref()
            .map_or_else(Vec::new, |(_, cols)| cols.clone())
    }
    pub(super) fn can_edit_grid(&self) -> bool {
        if !self.grid_edit_binding_valid() {
            return false;
        }
        if self.database_ui.remote_results {
            self.can_edit_remote()
        } else {
            self.service.db.writable
                && self.service.db.origin.is_some()
                && !self.service.db.is_busy()
        }
    }
    pub(super) fn sqlite_edit_batch(&self) -> Option<EditBatch> {
        if !self.grid_edit_binding_valid() || !self.grid_edit_budget_valid() {
            return None;
        }
        let (table, columns) = self.service.db.origin.as_ref()?;
        let result = self.service.db.result.as_ref()?;
        Some(EditBatch {
            table: table.clone(),
            columns: columns.clone(),
            edits: self
                .database_ui
                .staged
                .iter()
                .filter_map(|(&(row, col), value)| {
                    Some(CellEdit {
                        row: result
                            .rows
                            .get(row)?
                            .iter()
                            .map(codisseu_core::sqlite_cell)
                            .collect(),
                        column: result.columns.get(col)?.clone(),
                        value: value.clone(),
                    })
                })
                .collect(),
        })
    }
    pub(super) fn start_grid_edit(&mut self, row: usize, col: usize) {
        if !self.can_edit_grid() || !self.finish_grid_edit() {
            return;
        }
        let Some(column) = self.grid_edit_columns().get(col).cloned() else {
            return;
        };
        if !editable_column(&column) {
            return;
        }
        let original = self.grid_original_cell(row, col);
        let Some(value) = self
            .database_ui
            .staged
            .get(&(row, col))
            .cloned()
            .or(original)
        else {
            return;
        };
        let text = match (&value, cell_edit::editor_kind(&column)) {
            (Cell::Null, _) => String::new(),
            (Cell::Bool(v), CellEditorKind::Integer) => u8::from(*v).to_string(),
            (Cell::Number(v), CellEditorKind::Boolean) if v == "0" || v == "1" => {
                (v == "1").to_string()
            }
            _ => value.clipboard_text(),
        };
        self.database_ui.edit = Some((row, col, text, value == Cell::Null));
        self.database_ui.editing_remote = Some(self.database_ui.remote_results);
        self.database_ui.edit_binding = self.grid_edit_identity();
        self.database_ui.edit_focus = true;
        self.database_ui.edit_error = None;
    }
    fn grid_original_cell(&self, row: usize, col: usize) -> Option<Cell> {
        if self.database_ui.remote_results {
            self.service
                .remote_db
                .result
                .as_ref()?
                .rows
                .get(row)?
                .get(col)
                .cloned()
        } else {
            self.service
                .db
                .result
                .as_ref()?
                .rows
                .get(row)?
                .get(col)
                .map(codisseu_core::sqlite_cell)
        }
    }
    pub(super) fn finish_grid_edit(&mut self) -> bool {
        if !self.grid_edit_binding_valid() {
            self.database_ui.edit_error=Some("O resultado ou a conexão mudou. Descarte as propostas antigas e execute novamente a consulta.".into());
            return false;
        }
        let Some((row, col, _, _)) = &self.database_ui.edit else {
            return true;
        };
        let (row, col) = (*row, *col);
        let Some(column) = self.grid_edit_columns().get(col).cloned() else {
            return false;
        };
        let Some(original) = self.grid_original_cell(row, col) else {
            return false;
        };
        accept_editor(&mut self.database_ui, &column, &original)
    }
    pub(super) fn grid_edit_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.database_ui.edit.is_some() {
            if egui::Popup::is_any_open(ctx) {
                return true;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.database_ui.edit = None;
                self.database_ui.edit_error = None;
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                self.finish_grid_edit();
            }
            return true;
        }
        if ctx
            .memory(|m| m.focused())
            .is_some_and(|id| Some(id) == self.grid_focus)
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2))
        {
            if let Some((row, col)) = self.database_ui.selected {
                self.start_grid_edit(row, col);
            }
            return true;
        }
        false
    }
    pub(super) fn submit_grid_edits(&mut self, commit: bool) {
        if !self.finish_grid_edit() {
            return;
        }
        if !self.database_ui.staged.is_empty() && !self.grid_edit_budget_valid() {
            self.database_ui.edit_error=Some("Lote excede 500 células ou 4 MiB de valores originais/propostos; envie um lote menor.".into());
            return;
        }
        let remote = self.database_ui.remote_results;
        let has_proposals = !self.database_ui.staged.is_empty();
        let command = if remote {
            let command = if has_proposals {
                let Some(batch) = self.edit_batch() else {
                    return;
                };
                if commit {
                    RemoteCommand::ApplyAndCommit(batch)
                } else {
                    RemoteCommand::Apply(batch)
                }
            } else if commit {
                RemoteCommand::Commit
            } else {
                return;
            };
            Command::RemoteDatabase(command)
        } else {
            let command = if has_proposals {
                let Some(batch) = self.sqlite_edit_batch() else {
                    return;
                };
                if commit {
                    DbCommand::ApplyAndCommit(batch)
                } else {
                    DbCommand::Apply(batch)
                }
            } else if commit {
                DbCommand::Commit
            } else {
                return;
            };
            Command::Database(command)
        };
        self.prepare_grid_refresh();
        match self.service.dispatch(command) {
            Ok(()) => self.database_ui.refresh_after_edit = true,
            Err(e) => self.database_ui.edit_error = Some(e),
        }
    }
    pub(super) fn grid_edit_toolbar(&mut self, ui: &mut egui::Ui) {
        let remote = self.database_ui.remote_results;
        let busy = if remote {
            self.service.remote_db.is_busy()
        } else {
            self.service.db.is_busy()
        };
        let transaction = if remote {
            self.service.remote_db.transaction
        } else {
            self.service.db.transaction
        };
        if !remote {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x=0.;
                let pending=!self.database_ui.staged.is_empty() || self.database_ui.edit.is_some();
                if components::database_icon_button(ui,Icon::Submit,"Submit",self.can_edit_grid()&&pending).clicked(){self.submit_grid_edits(false);}
                if components::database_icon_button(ui,Icon::Commit,"Commit",!busy&&(transaction||pending)).clicked(){self.submit_grid_edits(true);}
                if components::database_icon_button(ui,Icon::Rollback,"Rollback",!busy&&transaction).clicked(){self.rollback_grid_edits();}
                if components::database_icon_button(ui,Icon::Revert,"Revert",pending&&!busy).clicked(){self.discard_grid_edits();}
                if !self.service.db.writable && ui.add_enabled(!busy,egui::Button::new("Habilitar edição")).on_hover_text("Reabre este arquivo SQLite com permissão de escrita; somente UPDATE validado por chave").clicked()
                    && let Some(path)=self.service.db.config.clone() {
                    self.database_ui.sqlite_enable_query=self.database_ui.sqlite_result_sql.clone();
                    self.database_ui.sqlite_enable_source=self.database_ui.sqlite_result_source.clone();
                    self.send(Command::Database(DbCommand::ConnectWritable(path)));
                }
            });
        }
        if let Some((_, col, _, null)) = self.database_ui.edit.clone()
            && let Some(column) = self.grid_edit_columns().get(col).cloned()
        {
            ui.horizontal(|ui| {
                ui.add_space(8.);
                ui.label(format!("{} · {}", column.name, column.data_type));
                let mut is_null = null;
                if ui
                    .add_enabled(column.nullable, egui::Checkbox::new(&mut is_null, "NULL"))
                    .changed()
                    && let Some(edit) = &mut self.database_ui.edit
                {
                    edit.3 = is_null;
                    self.database_ui.edit_focus = !is_null;
                }
                if ui
                    .button("Aplicar célula")
                    .on_hover_text("Enter · mantém a alteração local até Submit ou Commit")
                    .clicked()
                {
                    self.finish_grid_edit();
                }
                if ui
                    .button("Cancelar célula")
                    .on_hover_text("Escape")
                    .clicked()
                {
                    self.database_ui.edit = None;
                    self.database_ui.edit_error = None;
                }
            });
        } else if !self.database_ui.staged.is_empty() {
            ui.label(
                RichText::new(format!(
                    "{} célula(s) pendente(s) · Commit grava no banco",
                    self.database_ui.staged.len()
                ))
                .color(color(self.tokens.accent)),
            );
        } else if !self.can_edit_grid() {
            ui.label(RichText::new("Somente leitura · edição exige tabela simples, chave primária completa e conexão com escrita habilitada").small().color(color(self.tokens.muted)));
        }
        if let Some(error) = &self.database_ui.edit_error {
            ui.label(RichText::new(error).color(color(self.tokens.error)));
        }
        if let Some(notice) = &self.database_ui.edit_notice {
            ui.label(
                RichText::new(notice)
                    .small()
                    .color(color(self.tokens.muted)),
            );
        }
    }
    pub(super) fn discard_grid_edits(&mut self) {
        self.database_ui.staged.clear();
        self.database_ui.edit = None;
        self.database_ui.edit_error = None;
        self.database_ui.editing_remote = None;
        self.database_ui.edit_binding = None;
    }
    pub(super) fn rollback_grid_edits(&mut self) {
        self.prepare_grid_refresh();
        self.discard_grid_edits();
        let command = if self.database_ui.remote_results {
            Command::RemoteDatabase(RemoteCommand::Rollback)
        } else {
            Command::Database(DbCommand::Rollback)
        };
        match self.service.dispatch(command) {
            Ok(()) => self.database_ui.refresh_after_edit = true,
            Err(e) => self.database_ui.edit_error = Some(e),
        }
    }
    fn prepare_grid_refresh(&mut self) {
        self.database_ui.edit_reload_remote = self.database_ui.remote_results;
        self.database_ui.edit_reload_sql = self.result_query().map(str::to_owned);
        self.database_ui.edit_reload_source = if self.database_ui.remote_results {
            self.database_ui.result_source.clone()
        } else {
            self.database_ui.sqlite_result_source.clone()
        };
        self.database_ui.edit_reload_page = if self.database_ui.remote_results {
            self.database_ui.remote_page.clone()
        } else {
            self.database_ui.sqlite_page.clone()
        };
    }
    pub(super) fn grid_edit_tick(&mut self) {
        if !self.service.db.is_busy()
            && let Some(sql) = self.database_ui.sqlite_enable_query.take()
            && self.service.db.last_error.is_none()
            && self.service.db.writable
        {
            self.database_ui.sqlite_pending_sql = Some(sql.clone());
            self.database_ui.sqlite_pending_source = self.database_ui.sqlite_enable_source.take();
            self.send(Command::Database(DbCommand::Execute { sql }));
        }
        if !self.database_ui.refresh_after_edit {
            if !self.grid_edit_binding_valid() {
                self.database_ui.edit_error = Some(
                    "O resultado mudou; descarte as propostas antigas antes de editar novamente."
                        .into(),
                );
            }
            return;
        }
        let remote = self.database_ui.edit_reload_remote;
        if if remote {
            self.service.remote_db.is_busy()
        } else {
            self.service.db.is_busy()
        } {
            return;
        }
        self.database_ui.refresh_after_edit = false;
        let error = if remote {
            self.service.remote_db.error.clone()
        } else {
            self.service.db.last_error.clone()
        };
        if let Some(error) = error {
            self.database_ui.edit_error = Some(error);
            return;
        }
        self.database_ui.edit_notice = Some(if remote {
            self.service.remote_db.status.clone()
        } else {
            self.service.db.status.clone()
        });
        self.discard_grid_edits();
        if remote {
            if let Some(table) = self.database_ui.table.clone() {
                self.remote_send(RemoteCommand::LoadTablePage {
                    table,
                    offset: self.database_ui.page_offset,
                });
            } else if let Some(page) = self.database_ui.remote_page.clone()
                && let Some(source) = self.service.remote_db.active.as_ref()
            {
                let bound = codisseu_core::query_page::BoundQueryPage {
                    datasource_id: source.id.clone(),
                    connection_generation: self.service.remote_db.connection_generation,
                    payload: page.clone(),
                };
                self.database_ui.remote_pending_page = Some(page);
                self.remote_send(RemoteCommand::ExecutePage(bound));
            } else if let Some(sql) = self.database_ui.result_sql.clone() {
                self.remote_send(RemoteCommand::Execute(sql));
            }
        } else if let Some(page) = self.database_ui.edit_reload_page.take() {
            self.database_ui.sqlite_result_sql = self.database_ui.edit_reload_sql.take();
            self.database_ui.sqlite_result_source = self.database_ui.edit_reload_source.take();
            self.database_ui.sqlite_pending_page = Some(page.clone());
            self.send(Command::Database(DbCommand::ExecutePage(page)));
        } else if let Some(sql) = self.database_ui.edit_reload_sql.take() {
            self.database_ui.sqlite_pending_sql = Some(sql.clone());
            self.database_ui.sqlite_pending_source = self.database_ui.edit_reload_source.take();
            self.send(Command::Database(DbCommand::Execute { sql }));
        }
    }
}

pub(super) fn editable_column(column: &RemoteColumn) -> bool {
    !column.primary_key && cell_edit::editor_kind(column) != CellEditorKind::Unsupported
}
fn accept_editor(state: &mut database::DatabaseUi, column: &RemoteColumn, original: &Cell) -> bool {
    let Some((row, col, text, null)) = state.edit.clone() else {
        return true;
    };
    match cell_edit::parse_cell_edit(column, &text, null) {
        Ok(value) => {
            if value == *original {
                state.staged.remove(&(row, col));
            } else {
                state.staged.insert((row, col), value);
            }
            state.edit = None;
            state.edit_error = None;
            true
        }
        Err(error) => {
            state.edit_error = Some(error);
            false
        }
    }
}

/// Overlay an actual egui input in the allocated cell, retaining source indices.
pub(super) fn inline_cell_editor(
    ui: &mut egui::Ui,
    response: &egui::Response,
    row: usize,
    col: usize,
    column: Option<&RemoteColumn>,
    state: &mut database::DatabaseUi,
    tokens: &Tokens,
) {
    let Some((edit_row, edit_col, mut text, null)) = state.edit.clone() else {
        return;
    };
    if (row, col) != (edit_row, edit_col) {
        return;
    }
    let Some(column) = column else {
        return;
    };
    let rect = response.rect.shrink2(egui::vec2(2., 1.));
    ui.painter().rect_filled(rect, 0., color(tokens.editor));
    let previous = text.clone();
    let kind = cell_edit::editor_kind(column);
    let label = format!("Editar {}", column.name);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(("db.inline", row, col))
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    let input = if kind == CellEditorKind::Boolean {
        let mut chosen = if null {
            "NULL".to_owned()
        } else {
            text.clone()
        };
        let output = egui::ComboBox::from_id_salt("boolean")
            .selected_text(&chosen)
            .width(rect.width() - 8.)
            .show_ui(&mut child, |ui| {
                for value in ["true", "false"] {
                    ui.selectable_value(&mut chosen, value.to_owned(), value);
                }
            });
        if chosen != "NULL" {
            text = chosen;
        }
        output.response
    } else if let Some(values) = cell_edit::enum_values(column) {
        egui::ComboBox::from_id_salt("enum")
            .selected_text(&text)
            .width(rect.width() - 8.)
            .show_ui(&mut child, |ui| {
                if cell_edit::base_type(column) == "set" {
                    let mut selected: std::collections::BTreeSet<_> =
                        text.split(',').map(str::to_owned).collect();
                    for value in &values {
                        let mut checked = selected.contains(value);
                        if ui.checkbox(&mut checked, value).changed() {
                            if checked {
                                selected.insert(value.clone());
                            } else {
                                selected.remove(value);
                            }
                        }
                    }
                    text = values
                        .into_iter()
                        .filter(|v| selected.contains(v))
                        .collect::<Vec<_>>()
                        .join(",");
                } else {
                    for value in values {
                        ui.selectable_value(&mut text, value.clone(), value);
                    }
                }
            })
            .response
    } else {
        child
            .add_enabled(
                !null,
                egui::TextEdit::singleline(&mut text)
                    .id_salt("value")
                    .hint_text(if null { "NULL" } else { "" })
                    .font(egui::FontId::monospace(tokens.database_grid_font_size))
                    .margin(egui::vec2(5., 3.))
                    .desired_width(rect.width() - 4.),
            )
            .on_hover_text(&label)
    };
    input.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, !null, &label));
    if state.edit_focus {
        input.request_focus();
        state.edit_focus = false;
    }
    if text != previous && !cell_edit::accepts_input(column, &text) {
        text = previous;
        state.edit_error = Some(format!(
            "{} exige um valor {}",
            column.name, column.data_type
        ));
    } else if text != previous {
        state.edit_error = None;
    }
    state.edit = Some((row, col, text, null));
    ui.painter().rect_stroke(
        rect,
        0.,
        egui::Stroke::new(
            1.,
            color(if state.edit_error.is_some() {
                tokens.error
            } else {
                tokens.accent
            }),
        ),
        egui::StrokeKind::Inside,
    );
    // Do not submit on blur: invalid input and pending changes stay inspectable.
}
