//! Explicit decisions over versioned external snapshots; no I/O in the modal.
use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct ExternalChangesUi {
    pub active: Option<DocumentId>,
    pub dismissed: HashMap<DocumentId, u64>,
    saving_close: Option<Vec<DocumentId>>,
    close_error: Option<String>,
    untitled_saving: Option<DocumentId>,
    decision_pending: Option<DocumentId>,
    decision_error: Option<String>,
}
impl CodisseuApp {
    pub(super) fn external_changes_tick(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.external_changes.decision_pending
            && !self.service.document_save_pending(id)
        {
            self.external_changes.decision_pending = None;
            if self.service.document_external_change(id).is_some() {
                self.external_changes.decision_error = Some(self.service.last_error.clone().unwrap_or_else(|| "Não foi possível atualizar o arquivo. Suas alterações foram preservadas.".into()));
            }
        }
        self.external_changes
            .dismissed
            .retain(|id, _| self.service.documents.iter().any(|doc| doc.id == *id));
        if let Some(id) = self.external_changes.active
            && self.service.document_external_change(id).is_none()
        {
            self.external_changes.active = None;
            self.external_changes.decision_error = None;
            self.focus_editor = true;
        }
        if self.external_changes.active.is_none()
            && !self.exit_when_saved
            && self.confirm.is_none()
            && !self.workspace_switch_pending()
            && self.path_dialog.is_none()
            && !self.preferences_dialog
            && !self.provider_dialog
            && self.project_files_ui.trash.is_none()
            && !self.transfer_modal_open()
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
        {
            self.external_changes.active = self.service.documents.iter().find_map(|doc| {
                let change = self.service.document_external_change(doc.id)?;
                (self.external_changes.dismissed.get(&doc.id) != Some(&change.revision))
                    .then_some(doc.id)
            });
        }
    }

    pub(super) fn poll_close_saves(&mut self) {
        let Some(ids) = self.external_changes.saving_close.as_ref() else {
            return;
        };
        if self.path_dialog.is_some() {
            return;
        }
        if ids.iter().any(|id| self.service.document_save_pending(*id)) {
            return;
        }
        if let Some(id) = self.external_changes.untitled_saving.take()
            && self
                .service
                .documents
                .iter()
                .any(|doc| doc.id == id && doc.is_untitled())
        {
            self.external_changes.saving_close = None;
            self.external_changes.close_error = self.service.last_error.clone();
            return;
        }
        if let Some(id) = ids
            .iter()
            .find(|id| {
                self.service
                    .documents
                    .iter()
                    .any(|doc| doc.id == **id && doc.is_untitled() && doc.is_dirty())
            })
            .copied()
        {
            self.external_changes.untitled_saving = Some(id);
            self.open_path(PathAction::SaveUntitled(id));
            return;
        }
        if let Some(id) = ids
            .iter()
            .find(|id| self.service.document_external_change(**id).is_some())
        {
            self.external_changes.active = Some(*id);
            return;
        }
        let dirty = ids.iter().any(|id| {
            self.service
                .documents
                .iter()
                .any(|doc| doc.id == *id && doc.is_dirty())
        });
        self.external_changes.saving_close = None;
        if dirty {
            self.external_changes.close_error =
                Some(self.service.last_error.clone().unwrap_or_else(|| {
                    "O arquivo ainda tem alterações. Tente salvar novamente ou volte ao editor."
                        .into()
                }));
        } else {
            self.finish_close_decision(false);
        }
    }

    fn save_before_close(&mut self) {
        let ids = self.close_document_ids();
        self.external_changes.close_error = None;
        self.external_changes.dismissed.clear();
        for id in &ids {
            if self
                .service
                .documents
                .iter()
                .any(|doc| doc.id == *id && doc.is_untitled())
            {
                continue;
            }
            if let Err(error) = self.service.dispatch(Command::Save { id: *id }) {
                self.external_changes.close_error = Some(error);
                break;
            }
        }
        self.external_changes.saving_close = Some(ids);
    }

    pub(super) fn cancel_close_decision(&mut self) {
        self.confirm = None;
        self.external_changes.saving_close = None;
        self.external_changes.untitled_saving = None;
        self.external_changes.close_error = None;
        self.focus_editor = true;
    }

    fn close_document_ids(&self) -> Vec<DocumentId> {
        self.service
            .documents
            .iter()
            .filter(|doc| {
                doc.is_dirty()
                    && match self.confirm {
                        Some(Confirm::Tab { id, .. }) => doc.id == id,
                        Some(Confirm::Exit) => true,
                        _ => false,
                    }
            })
            .map(|doc| doc.id)
            .collect()
    }

    fn finish_close_decision(&mut self, discard: bool) {
        self.external_changes.saving_close = None;
        self.external_changes.untitled_saving = None;
        self.external_changes.close_error = None;
        match self.confirm.take() {
            Some(Confirm::Tab { view, id }) => {
                self.send(Command::CloseTab { view, id, discard });
                self.focus_editor = true;
            }
            Some(Confirm::Exit) => self.exit_when_saved = true,
            Some(Confirm::ExitWithoutSession) => {
                self.exit_without_session = true;
                self.exit_when_saved = true;
            }
            None => {}
        }
    }

    pub(super) fn unsaved_window(&mut self, ctx: &egui::Context) {
        if self.confirm.is_none()
            || self.external_changes.active.is_some()
            || self.path_dialog.is_some()
        {
            return;
        }
        let session_failure = matches!(self.confirm, Some(Confirm::ExitWithoutSession));
        let saving = self.external_changes.saving_close.is_some();
        let mut cancel = false;
        let mut discard = false;
        let mut save = false;
        let modal = egui::Modal::new(egui::Id::unique("codisseu.unsaved"))
            .frame(self.tokens.dialog_frame(ctx))
            .show(ctx, |ui| {
                self.tokens.dialog_header(ui, ctx, 520., if session_failure {
                    "Não foi possível salvar a sessão"
                } else {
                    "Alterações não salvas"
                });
                if session_failure {
                    ui.label("A sessão anterior será preservada. Você pode voltar ao editor ou sair sem atualizar as abas salvas.");
                } else {
                    ui.label("Os arquivos abaixo têm alterações no editor.");
                    egui::ScrollArea::vertical().max_height(150.).codisseu().show(ui, |ui| {
                        for id in self.close_document_ids() {
                            if let Some(doc) = self.service.documents.iter().find(|doc| doc.id == id) {
                                let name = doc.path.file_name().unwrap_or_default().to_string_lossy();
                                ui.add(egui::Label::new(name).truncate()).on_hover_text(doc.path.display().to_string());
                            }
                        }
                    });
                    if let Some(error) = &self.external_changes.close_error {
                        ui.add(egui::Label::new(error).wrap());
                    }
                    if saving {
                        ui.horizontal(|ui| { ui.add(egui::Spinner::new().size(16.).color(color(self.tokens.muted))); ui.label("Salvando arquivos…"); });
                    }
                }
                self.tokens.dialog_footer(ui, |ui| {
                    if !session_failure {
                        save = ui.add_enabled(!saving, self.tokens.dialog_button("Salvar e fechar", true)).clicked();
                    }
                    discard = ui.add_enabled(!saving, self.tokens.dialog_button(if session_failure {
                        "Sair sem salvar sessão"
                    } else { "Descartar e fechar" }, false)).clicked();
                    cancel = ui.add(self.tokens.dialog_button("Voltar ao editor", false)).clicked();
                });
            });
        cancel |= modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if cancel {
            self.cancel_close_decision();
        } else if discard {
            self.finish_close_decision(true);
        } else if save {
            self.save_before_close();
        }
        if cancel || discard || save {
            ctx.request_repaint();
        }
    }

    pub(super) fn external_changes_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.external_changes.active else {
            return;
        };
        let Some(change) = self.service.document_external_change(id).cloned() else {
            return;
        };
        let Some(doc) = self.service.documents.iter().find(|doc| doc.id == id) else {
            return;
        };
        let version = doc.version;
        let name = doc
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let pending = self.service.document_save_pending(id);
        let mut cancel = false;
        let mut reload = false;
        let mut overwrite = false;
        let modal = egui::Modal::new(egui::Id::unique("codisseu.external.change"))
            .frame(self.tokens.dialog_frame(ctx))
            .show(ctx, |ui| {
                self.tokens.dialog_header(ui, ctx, 560., "O arquivo mudou no disco");
                ui.strong(&name);
                ui.add(egui::Label::new(change.path.display().to_string()).wrap());
                ui.add_space(6.);
                ui.label("Há alterações no editor e uma versão diferente salva por outro programa.");
                ui.label("Usar a versão do disco substitui o texto aberto. Sobrescrever grava o texto do editor no lugar da versão do disco.");
                if let Some(error) = &self.external_changes.decision_error {
                    ui.add(egui::Label::new(RichText::new(error).color(color(self.tokens.error))).wrap());
                }
                if pending {
                    ui.horizontal(|ui| { ui.add(egui::Spinner::new().size(16.).color(color(self.tokens.muted))); ui.label("Atualizando arquivo…"); });
                }
                self.tokens.dialog_footer(ui, |ui| {
                    overwrite = ui.add_enabled(!pending, self.tokens.dialog_button("Sobrescrever", true)).clicked();
                    reload = ui.add_enabled(!pending, self.tokens.dialog_button("Usar versão do disco", false)).clicked();
                    cancel = ui.add(self.tokens.dialog_button("Cancelar", false)).clicked();
                });
            });
        cancel |= modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if cancel {
            self.external_changes.dismissed.insert(id, change.revision);
            self.external_changes.active = None;
            self.external_changes.decision_error = None;
            self.cancel_close_decision();
            self.service.cancel_workspace_open();
            self.workspace_switch = Default::default();
            self.focus_editor = true;
        } else if reload || overwrite {
            let command = if reload {
                Command::ReloadDocument {
                    id,
                    expected_version: version,
                    external_revision: change.revision,
                }
            } else {
                Command::OverwriteDocument {
                    id,
                    expected_version: version,
                    external_revision: change.revision,
                }
            };
            self.external_changes.decision_error = None;
            match self.service.dispatch(command) {
                Ok(()) => self.external_changes.decision_pending = Some(id),
                Err(error) => self.external_changes.decision_error = Some(error),
            }
        }
        if cancel || reload || overwrite {
            ctx.request_repaint();
        }
    }
}
