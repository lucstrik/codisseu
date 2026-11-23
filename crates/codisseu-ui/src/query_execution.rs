//! Query actions share the active view's caret/selection and datasource binding.
use super::*;
impl CodisseuApp {
    pub(super) fn query_execution_error(&self, ui: &mut egui::Ui, error: &str) {
        crate::status_messages::status_message(ui, error);
        egui::ScrollArea::vertical()
            .id_salt("database.query.error")
            .max_height(72.)
            .codisseu()
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(error).color(color(self.tokens.error)))
                        .wrap()
                        .selectable(true),
                );
            });
    }
    pub(super) fn is_query_document(&self) -> bool {
        self.service.active_document().is_some_and(|d| {
            self.service.remote_db.is_console_path(&d.path)
                || d.path.extension().is_some_and(|e| e == "sql")
                || (d.path.extension().is_some_and(|e| e == "json")
                    && self
                        .service
                        .remote_db
                        .active
                        .as_ref()
                        .is_some_and(|c| c.engine == codisseu_core::remote::Engine::Mongo))
        })
    }
    pub(super) fn can_execute_query(&self) -> bool {
        let bound = self.service.active_document().is_some_and(|d| {
            self.service
                .remote_db
                .console_bindings
                .contains_key(&d.path)
        });
        self.is_query_document()
            && (bound || self.service.remote_db.active.is_some() || self.service.db.connected)
            && !self.service.remote_db.is_busy()
            && !self.service.db.is_busy()
            && self.database_ui.staged.is_empty()
            && self.database_ui.edit.is_none()
    }
    pub(super) fn query_execution_toolbar(&mut self, ui: &mut egui::Ui) {
        if !self.is_query_document() {
            return;
        }
        let enabled = self.can_execute_query();
        let mut execute = false;
        ui.horizontal(|ui| {
            ui.set_min_height(self.tokens.database_toolbar_height);
            ui.spacing_mut().item_spacing.x = 0.;
            ui.add_enabled_ui(enabled, |ui| {
                execute = icon_button(
                    ui,
                    Icon::Play,
                    "Executar consulta sob o cursor",
                    false,
                    self.tokens.database_toolbar_height,
                )
                .on_hover_text("Ctrl+Enter · executa a seleção ou o bloco da consulta sob o cursor")
                .clicked();
            });
            ui.label(
                RichText::new("Ctrl+Enter")
                    .small()
                    .color(color(self.tokens.muted)),
            );
        });
        if execute {
            self.execute_sql();
            self.focus_editor = true;
        }
    }
}
