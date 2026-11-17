//! Compose Markdown views around the one shared source editor and document service.
use super::*;
use crate::markdown::{MarkdownAction, MarkdownMode};
use codisseu_core::{EditKind, Selection, TextDelta, ViewId};

impl CodisseuApp {
    pub(super) fn markdown_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self
            .service
            .active_document()
            .is_none_or(|d| !markdown::is_markdown(&d.path))
        {
            return false;
        }
        if self.path_dialog.is_some()
            || self.confirm.is_some()
            || self.project_search.open
            || self.preferences_dialog
            || ctx.memory(|m| m.top_modal_layer().is_some())
        {
            return false;
        }
        let request = ctx
            .data_mut(|d| {
                d.remove_temp::<bool>(egui::Id::unique("codisseu.markdown.split.request"))
            })
            .unwrap_or(false);
        let active = self.editor_focused(ctx)
            || (!text_field_focused(ctx)
                && self.service.active_view().is_some_and(|view| {
                    self.service
                        .view_document(view)
                        .is_some_and(|d| self.markdown.mode(view, &d.path) != MarkdownMode::Source)
                }));
        if request || (active && consume_exact(ctx, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::V))
        {
            let view = self.service.active_view().unwrap();
            let path = self.service.active_document().unwrap().path.clone();
            let mode = if request {
                MarkdownMode::Split
            } else if self.markdown.mode(view, &path) == MarkdownMode::Preview {
                MarkdownMode::Source
            } else {
                MarkdownMode::Preview
            };
            self.markdown.set_mode(view, &path, mode);
            self.focus_editor = mode != MarkdownMode::Preview;
            return true;
        }
        false
    }
    pub(super) fn editor_document_content(&mut self, ui: &mut egui::Ui, view: ViewId) {
        let Some(doc) = self.service.view_document(view) else {
            return;
        };
        let path = doc.path.clone();
        if !markdown::is_markdown(&path) {
            self.editor(ui, view);
            return;
        }
        if self
            .markdown_navigation
            .as_ref()
            .is_some_and(|(target, _)| target == &path)
        {
            let (_, anchor) = self.markdown_navigation.take().unwrap();
            self.markdown.set_mode(view, &path, MarkdownMode::Split);
            self.markdown.navigate_anchor(view, anchor);
        }
        let mode = self.markdown.toolbar(ui, &self.tokens, view, &path);
        if mode == MarkdownMode::Source {
            self.editor(ui, view);
            return;
        }
        let all = ui.available_rect_before_wrap();
        let split = mode == MarkdownMode::Split;
        let mut preview = all;
        if split {
            let middle = all.left() + all.width() * 0.5;
            let source = egui::Rect::from_min_max(all.min, egui::pos2(middle - 2., all.bottom()));
            preview.min.x = middle + 3.;
            ui.painter().vline(
                middle,
                all.y_range(),
                egui::Stroke::new(1., color(self.tokens.control_border)),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("markdown.source", view))
                    .max_rect(source),
            );
            child.set_clip_rect(ui.clip_rect().intersect(source));
            self.editor_bridge.regions.insert(view, source);
            self.editor(&mut child, view);
            if let Some(metrics) = self.editor_bridge.view_metrics.get(&view) {
                self.markdown.set_source_line(view, metrics.visible_start);
            }
        } else {
            self.editor_bridge.regions.remove(&view);
            self.focus_editor = false;
        }
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("markdown.preview", view))
                .max_rect(preview),
        );
        child.set_clip_rect(ui.clip_rect().intersect(preview));
        let actions = if let Some(doc) = self.service.view_document(view) {
            self.markdown.preview(
                &mut child,
                &self.tokens,
                view,
                doc,
                self.service.workspace.as_deref(),
            )
        } else {
            Vec::new()
        };
        for action in actions {
            self.markdown_action(ui.ctx(), view, &path, action);
        }
        ui.advance_cursor_after_rect(all);
    }
    fn markdown_action(
        &mut self,
        ctx: &egui::Context,
        view: ViewId,
        path: &std::path::Path,
        action: MarkdownAction,
    ) {
        if !matches!(action, MarkdownAction::ScrollSource { .. })
            && self.service.active_view() != Some(view)
        {
            self.send(Command::ActivateView { view });
            if self.service.active_view() != Some(view) {
                return;
            }
        }
        match action {
            MarkdownAction::ToggleTask {
                id,
                version,
                byte_offset,
                checked,
            } => {
                let Some(doc) = self.service.documents.iter().find(|d| d.id == id) else {
                    return;
                };
                let snapshot = doc.snapshot();
                if doc.version != version || byte_offset >= snapshot.len_bytes() {
                    self.service.last_error =
                        Some("Markdown mudou; aguarde a prévia atualizar".into());
                    return;
                }
                let at = snapshot.byte_to_char(byte_offset);
                if snapshot.char_to_byte(at) != byte_offset
                    || !matches!(snapshot.char(at), ' ' | 'x' | 'X')
                {
                    self.service.last_error =
                        Some("Marcador Markdown não corresponde mais à prévia".into());
                    return;
                }
                let before = doc.selection();
                let delta = TextDelta {
                    range: at..at + 1,
                    insert: if checked { "x" } else { " " }.into(),
                };
                let after = before.mapped(&delta);
                self.send(Command::EditTransaction {
                    id,
                    expected_version: version,
                    delta,
                    kind: EditKind::Atomic,
                    before,
                    after,
                });
            }
            MarkdownAction::OpenLocal {
                path: target,
                anchor,
            } => {
                let path = match markdown::validate_local_path(
                    &target,
                    path,
                    self.service.workspace.as_deref(),
                ) {
                    Ok(path) => path,
                    Err(error) => {
                        self.service.last_error = Some(error);
                        return;
                    }
                };
                if let Some(anchor) = anchor {
                    self.markdown_navigation = Some((path.clone(), anchor));
                }
                self.send(Command::OpenFile(path));
                self.focus_editor = true;
            }
            MarkdownAction::OpenExternal { url } => {
                ctx.open_url(egui::OpenUrl::new_tab(url));
            }
            MarkdownAction::ScrollSource { line } => {
                if let Some(state) = self.service.view(view) {
                    let scroll = [
                        state.scroll()[0],
                        line as f32 * self.tokens.editor_line_height,
                    ];
                    self.send(Command::SetViewScroll { view, scroll });
                }
            }
            MarkdownAction::RevealSource { range, line } => {
                let Some(doc) = self.service.view_document(view) else {
                    return;
                };
                let snapshot = doc.snapshot();
                if range.end > snapshot.len_bytes() {
                    return;
                }
                let selection = Selection::caret(snapshot.byte_to_char(range.start));
                let id = doc.id;
                let version = doc.version;
                self.send(Command::ActivateView { view });
                self.send(Command::SetSelection {
                    id,
                    expected_version: version,
                    selection,
                });
                self.markdown.set_mode(view, path, MarkdownMode::Split);
                self.markdown.set_source_line(view, line);
                self.focus_editor = true;
            }
        }
    }
}
