//! Split composition around the shared document view service.
use super::*;
use codisseu_core::ViewId;
impl CodisseuApp {
    pub(super) fn tab_group_menu(&mut self, ui: &mut egui::Ui, view: ViewId, row: egui::Rect) {
        if self.service.view(view).is_none() || row.width() < 36. {
            return;
        }
        let rect = egui::Rect::from_min_size(
            egui::pos2(row.right() - 32., row.top() + 9.),
            egui::vec2(26., 24.),
        );
        let response = ui.interact(
            rect,
            egui::Id::unique(("codisseu.tabs.more", view)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                ui.is_enabled(),
                "Mais ações do grupo",
            )
        });
        let tokens = self.tokens.clone();
        let popup = egui::Popup::menu(&response)
            .width(234.)
            .style(move |style: &mut egui::Style| tokens.menu_style(style, false));
        let was_open = popup.is_open();
        let hovered = response.hovered() || response.has_focus() || was_open;
        if hovered {
            self.tab_paint.feedback(
                ui,
                rect.shrink2(egui::vec2(2., 1.)),
                [
                    color(self.tokens.tab_more_hover_background),
                    color(self.tokens.editor),
                    Color32::TRANSPARENT,
                ],
                self.tokens.tab_more_hover_radius,
            );
        }
        let foreground = color(if hovered {
            self.tokens.tab_more_hover_foreground
        } else {
            self.tokens.tab_more_foreground
        });
        for dy in [6., 11., 16.] {
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min + egui::vec2(12., dy), egui::Vec2::splat(2.)),
                0.,
                foreground,
            );
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                4.,
                egui::Stroke::new(1., color(self.tokens.accent)),
                egui::StrokeKind::Inside,
            );
        }
        let mut action = None;
        let split = self.service.document_views().len() == 2;
        popup.show(|ui| {
            for (id, label, enabled) in [
                (0, "Dividir à direita", !split),
                (1, "Dividir abaixo", !split),
                (2, "Remover divisão", split),
                (3, "Alterar orientação da divisão", split),
            ] {
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(label).min_size(egui::vec2(232., 24.)),
                    )
                    .clicked()
                {
                    action = Some(id);
                    ui.close();
                }
            }
        });
        match action {
            Some(0) => self.split_editor(false),
            Some(1) => self.split_editor(true),
            Some(2) => {
                self.send(Command::CloseView { view });
                self.focus_editor = true;
            }
            Some(3) => {
                self.send(Command::SetEditorSplit(codisseu_core::EditorSplit {
                    horizontal: !self.service.editor_split().horizontal,
                    ..self.service.editor_split()
                }));
                self.focus_editor = true;
            }
            _ => {
                if was_open
                    && !egui::Popup::is_id_open(
                        ui.ctx(),
                        egui::Popup::default_response_id(&response),
                    )
                {
                    self.focus_editor = true;
                }
            }
        }
        response.on_hover_text("Mais ações do grupo");
    }
    pub fn editor_view_metrics(
        &self,
    ) -> &std::collections::BTreeMap<ViewId, editor::EditorMetrics> {
        &self.editor_bridge.view_metrics
    }
    pub(super) fn split_editor(&mut self, horizontal: bool) {
        if self.service.document_views().len() < 2 {
            self.send(Command::CreateView);
        }
        self.send(Command::SetEditorSplit(codisseu_core::EditorSplit {
            horizontal,
            ..self.service.editor_split()
        }));
        self.focus_editor = true;
    }
    pub(super) fn next_editor_group(&mut self) {
        let views = self.service.document_views();
        if views.len() == 2 {
            let next = if self.service.active_view() == Some(views[0].id) {
                views[1].id
            } else {
                views[0].id
            };
            self.send(Command::ActivateView { view: next });
            self.focus_editor = true;
        }
    }
    pub(super) fn close_editor_group(&mut self) {
        if let Some(view) = self.service.active_view() {
            self.send(Command::CloseView { view });
        }
        self.focus_editor = true;
    }
    pub(super) fn activate_clicked_editor(&mut self, ctx: &egui::Context) {
        if self.path_dialog.is_some()
            || self.provider_dialog
            || self.database_ui.dialog
            || self.database_ui.exit_confirm
            || self.database_ui.preview.is_some()
            || self.database_ui.edit.is_some()
            || self.preferences_dialog
            || self.confirm.is_some()
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }
        let clicked = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } = e
                {
                    self.editor_bridge
                        .regions
                        .iter()
                        .find(|(id, rect)| {
                            rect.contains(*pos)
                                && !self.api_ui.active_in_view(**id)
                                && !self.browser_ui.active_in_view(**id)
                                && !self.jira_active_in_view(**id)
                        })
                        .map(|(&id, _)| id)
                } else {
                    None
                }
            })
        });
        if let Some(view) = clicked
            && self.service.active_view() != Some(view)
            && self.service.view(view).is_some()
        {
            self.send(Command::ActivateView { view });
            self.focus_editor = true;
        }
    }
    fn editor_group(&mut self, ui: &mut egui::Ui, id: ViewId, rect: egui::Rect) {
        let mut group = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("codisseu.editor.group", id))
                .max_rect(rect),
        );
        group.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.tabs(&mut group, id);
        if self.jira_active_in_view(id) {
            self.editor_bridge.regions.remove(&id);
            self.jira_resource_content(&mut group, id);
            return;
        }
        if self.browser_ui.active_in_view(id) {
            self.editor_bridge.regions.remove(&id);
            self.browser_panel(&mut group, id);
            return;
        }
        if self.api_ui.active_in_view(id) {
            self.editor_bridge.regions.remove(&id);
            self.api_editor(&mut group);
            return;
        }
        if self.service.active_view() == Some(id) {
            self.query_execution_toolbar(&mut group);
            self.search_bar(&mut group);
        }
        self.editor_bridge
            .regions
            .insert(id, group.available_rect_before_wrap());
        self.editor_document_content(&mut group, id);
    }
    pub(super) fn editors(&mut self, ui: &mut egui::Ui) {
        if self.service.documents.is_empty() {
            if self.jira_ui.tabs.has(0) {
                if self.jira_ui.tabs.selected(0).is_none()
                    && !self.browser_ui.active_in_view(0)
                    && !self.api_ui.active_in_view(0)
                {
                    let kind = if self.jira_ui.tabs.contains(crate::jira_tabs::Kind::Hours) {
                        crate::jira_tabs::Kind::Hours
                    } else {
                        crate::jira_tabs::Kind::Issue
                    };
                    self.jira_open_tab(kind);
                }
                self.tabs(ui, 0);
                if self.jira_resource_content(ui, 0) {
                    return;
                }
                if self.browser_ui.active_in_view(0) {
                    self.browser_panel(ui, 0);
                    return;
                }
                if self.api_ui.active_in_view(0) {
                    self.api_editor(ui);
                    return;
                }
            }
            if self.browser_ui.active_in_view(0) {
                self.tabs(ui, 0);
                self.browser_panel(ui, 0);
                return;
            }
            if !self.api_ui.tabs.is_empty() {
                self.tabs(ui, 0);
                self.api_editor(ui);
                return;
            }
            self.welcome(ui);
            return;
        }
        let ids: Vec<_> = self.service.document_views().iter().map(|v| v.id).collect();
        self.editor_bridge.regions.retain(|id, _| ids.contains(id));
        self.editor_bridge
            .view_metrics
            .retain(|id, _| ids.contains(id));
        if ids.len() < 2 {
            let id = ids.first().copied().unwrap_or(0);
            self.tabs(ui, id);
            if self.jira_active_in_view(id) {
                self.editor_bridge.regions.remove(&id);
                self.jira_resource_content(ui, id);
                return;
            }
            if self.browser_ui.active_in_view(id) {
                self.editor_bridge.regions.remove(&id);
                self.browser_panel(ui, id);
                return;
            }
            if self.api_ui.active_in_view(id) {
                self.editor_bridge.regions.remove(&id);
                self.api_editor(ui);
                return;
            }
            self.query_execution_toolbar(ui);
            self.search_bar(ui);
            self.editor_bridge
                .regions
                .insert(id, ui.available_rect_before_wrap());
            self.editor_document_content(ui, id);
            return;
        }
        let (all, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let horizontal = self.service.editor_split().horizontal;
        let ratio = self.service.editor_split().ratio;
        let coordinate = if horizontal {
            all.top() + all.height() * ratio
        } else {
            all.left() + all.width() * ratio
        };
        let line = if horizontal {
            egui::Rect::from_min_max(
                egui::pos2(all.left(), coordinate),
                egui::pos2(all.right(), coordinate + 1.),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(coordinate, all.top()),
                egui::pos2(coordinate + 1., all.bottom()),
            )
        };
        let handle = line.expand2(if horizontal {
            egui::vec2(0., 3.)
        } else {
            egui::vec2(3., 0.)
        });
        let response = ui.interact(
            handle,
            egui::Id::unique("codisseu.editor.divider"),
            egui::Sense::drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Splitter,
                true,
                "Redimensionar grupos do editor",
            )
        });
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let ratio = if horizontal {
                (pos.y - all.top()) / all.height()
            } else {
                (pos.x - all.left()) / all.width()
            }
            .clamp(0.2, 0.8);
            self.send(Command::SetEditorSplit(codisseu_core::EditorSplit {
                horizontal,
                ratio,
            }));
        }
        response.on_hover_cursor(if horizontal {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        });
        let mut painted_line = line;
        if horizontal {
            // The native divider starts after the first editor boundary pixel.
            painted_line.min.x += 1.;
        }
        ui.painter()
            .rect_filled(painted_line, 0., color(self.tokens.border));
        let (first, second) = if horizontal {
            (
                egui::Rect::from_min_max(all.min, line.right_top()),
                egui::Rect::from_min_max(line.left_bottom(), all.max),
            )
        } else {
            (
                egui::Rect::from_min_max(all.min, line.left_bottom()),
                egui::Rect::from_min_max(line.right_top(), all.max),
            )
        };
        self.editor_group(ui, ids[0], first);
        self.editor_group(ui, ids[1], second);
    }
}
