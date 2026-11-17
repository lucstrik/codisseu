//! A private display snapshot and a real text input at the source identifier.
//! The document, Save, edit history and provider versions remain untouched.
use super::*;
use codisseu_core::TextSnapshot;
use std::ops::Range;

pub(super) struct InlineRename {
    document: DocumentId,
    version: u64,
    caret: usize,
    range: Option<Range<usize>>,
    located: bool,
    rect: Option<egui::Rect>,
    clip: egui::Rect,
    rendered: String,
    revision: u64,
    options_open: bool,
}

impl InlineRename {
    pub fn new(document: DocumentId, version: u64, caret: usize) -> Self {
        Self {
            document,
            version,
            caret,
            range: None,
            located: false,
            rect: None,
            clip: egui::Rect::NOTHING,
            rendered: String::new(),
            revision: 0,
            options_open: false,
        }
    }
}

fn identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$' || !c.is_ascii()
}
fn locate(
    snapshot: &TextSnapshot,
    caret: usize,
    placeholder: &str,
    php: bool,
) -> Option<Range<usize>> {
    let len = placeholder.chars().count();
    if len == 0 || len > 512 || caret > snapshot.len_chars() {
        return None;
    }
    (caret.saturating_sub(len)..=caret).find_map(|start| {
        let end = start.checked_add(len)?;
        if end > snapshot.len_chars() || end < caret {
            return None;
        }
        let preceding = start.checked_sub(1).map(|at| snapshot.char(at));
        let following = (end < snapshot.len_chars()).then(|| snapshot.char(end));
        if preceding.is_some_and(|c| identifier(c) && !(php && c == '$'))
            || following.is_some_and(identifier)
        {
            return None;
        }
        (snapshot.slice(start..end) == placeholder).then_some(start..end)
    })
}

impl CodisseuApp {
    pub(crate) fn rename_editor_input_blocked(&self) -> bool {
        self.rename_ui.dialog && self.rename_ui.inline.is_some()
    }

    /// Called only for the active document's display; the canonical Rope is cloned.
    pub(crate) fn rename_inline_snapshot(
        &mut self,
        id: DocumentId,
        version: u64,
        snapshot: &TextSnapshot,
    ) -> Option<(TextSnapshot, u64, Range<usize>)> {
        if !self.rename_ui.dialog || !self.rename_ui.initialized {
            return None;
        }
        if self.rename_ui.name.chars().take(513).count() > 512 {
            self.rename_ui.inline = None;
            self.rename_ui.focus_name = true;
            return None;
        }
        let inline = self.rename_ui.inline.as_mut()?;
        if inline.document != id || inline.version != version {
            return None;
        }
        if !inline.located {
            inline.located = true;
            let php = self
                .service
                .active_document()
                .is_some_and(|d| d.path.extension().is_some_and(|e| e == "php"));
            inline.range = locate(
                snapshot,
                inline.caret,
                self.service.rename.placeholder.as_deref().unwrap_or(""),
                php,
            );
        }
        let range = inline.range.clone()?;
        if inline.rendered != self.rename_ui.name {
            inline.rendered = self.rename_ui.name.clone();
            inline.revision = inline.revision.wrapping_add(1);
        }
        let mut display = snapshot.clone();
        display.remove(range.clone());
        // Five display-only spaces reserve the observed inline options control.
        display.insert(range.start, &format!("{}     ", inline.rendered));
        let end = range.start + inline.rendered.chars().count();
        Some((display, inline.revision, range.start..end))
    }

    pub(crate) fn rename_inline_geometry(
        &mut self,
        id: DocumentId,
        version: u64,
        rect: egui::Rect,
        clip: egui::Rect,
    ) {
        if let Some(inline) = self.rename_ui.inline.as_mut()
            && inline.document == id
            && inline.version == version
        {
            inline.rect = Some(rect);
            inline.clip = clip;
        }
    }

    pub(super) fn rename_inline(&mut self, ctx: &egui::Context) {
        let Some(inline) = self.rename_ui.inline.as_ref() else {
            return;
        };
        if self
            .service
            .active_document()
            .is_none_or(|d| d.id != inline.document || d.version != inline.version)
        {
            self.cancel_rename();
            return;
        }
        let Some(rect) = inline.rect else {
            if self.rename_ui.initialized && inline.located && inline.range.is_none() {
                // A provider may return a non-local placeholder. Keep the safe
                // complete dialog rather than overlaying an unrelated source span.
                self.rename_ui.inline = None;
            }
            ctx.request_repaint();
            return;
        };
        let clip = inline.clip;
        if !clip.intersects(rect) {
            self.cancel_rename();
            return;
        }
        let mut refactor = false;
        let mut more = false;
        let mut toggle_options =
            ctx.input_mut(|i| i.consume_key(Modifiers::ALT | Modifiers::SHIFT, Key::O));
        let mut input_changed = false;
        let enabled = self.service.rename.is_prepared() && !self.service.rename.is_pending();
        let width = rect
            .width()
            .min((clip.right() - rect.left()).max(7.8))
            .max(7.8);
        egui::Modal::new(egui::Id::unique("codisseu.rename.inline"))
            .area(
                egui::Modal::default_area(egui::Id::unique("codisseu.rename.inline"))
                    .fixed_pos(rect.min),
            )
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.set_clip_rect(clip);
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.visuals_mut().selection.bg_fill = color(self.tokens.editor_selection);
                // TextEdit uses selection.stroke.color for selected glyphs.
                // A zero-alpha NONE stroke makes the edited name disappear.
                let foreground = color(
                    if self.service.rename.symbol_kind()
                        == codisseu_core::RenameSymbolKind::Property
                    {
                        self.tokens.editor_php_variable
                    } else {
                        self.tokens.rename_input_text
                    },
                );
                ui.visuals_mut().selection.stroke = egui::Stroke::new(0., foreground);
                let input_rect =
                    egui::Rect::from_min_size(rect.min, egui::vec2(width, rect.height()));
                ui.painter()
                    .rect_filled(input_rect, 0., color(self.tokens.caret_row));
                let font = FontId::monospace(self.tokens.editor_font_size);
                let line_height = self.tokens.editor_line_height;
                let mut layout = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _: f32| {
                    let mut job = egui::text::LayoutJob::simple_singleline(
                        text.as_str().into(),
                        font.clone(),
                        foreground,
                    );
                    job.sections[0].format.line_height = Some(line_height);
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                let response = ui.put(
                    input_rect,
                    egui::TextEdit::singleline(&mut self.rename_ui.name)
                        .id(egui::Id::unique("codisseu.rename.name"))
                        .frame(egui::Frame::NONE)
                        .margin(egui::Vec2::ZERO)
                        .desired_width(width)
                        .min_size(egui::vec2(width, line_height))
                        .layouter(&mut layout),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        enabled,
                        "Renomear símbolo e seus usos para:",
                    )
                });
                ui.painter().rect_stroke(
                    input_rect,
                    0.,
                    egui::Stroke::new(1., color(self.tokens.accent)),
                    egui::StrokeKind::Inside,
                );
                if self.rename_ui.focus_name && enabled {
                    response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(self.rename_ui.name.chars().count()),
                            )));
                        state.store(ctx, response.id);
                    }
                    self.rename_ui.focus_name = false;
                }
                input_changed = response.changed();
                refactor =
                    response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && enabled;
                let options = ui.put(
                    egui::Rect::from_min_size(input_rect.right_top(), egui::vec2(39., line_height)),
                    egui::Button::new(
                        RichText::new("// ≡")
                            .monospace()
                            .color(color(self.tokens.muted)),
                    )
                    .frame(false),
                );
                options.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::Button,
                        true,
                        "Also rename in · Alt+Shift+O",
                    )
                });
                toggle_options |= options.clicked();
            });
        if toggle_options && let Some(inline) = self.rename_ui.inline.as_mut() {
            inline.options_open = !inline.options_open;
        }
        let open = self
            .rename_ui
            .inline
            .as_ref()
            .is_some_and(|i| i.options_open);
        if open {
            egui::Area::new(egui::Id::unique("codisseu.rename.inline.options"))
                .order(egui::Order::Tooltip)
                .fixed_pos(rect.right_bottom())
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(210.);
                        ui.label("Also rename in:");
                        ui.checkbox(
                            &mut self.rename_ui.options.search_comments_strings,
                            "Comments and strings",
                        );
                        ui.checkbox(&mut self.rename_ui.options.search_text, "Text occurrences");
                        more = ui.link("More options… Shift+F6").clicked();
                    });
                });
        }
        if !open
            && self.rename_ui.input_error.is_none()
            && self.service.rename.placeholder.as_deref() == Some(self.rename_ui.name.as_str())
        {
            egui::Area::new(egui::Id::unique("codisseu.rename.inline.suggestions"))
                .order(egui::Order::Tooltip)
                .fixed_pos(rect.left_bottom() - egui::vec2(34., 0.))
                .show(ctx, |ui| {
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let (popup, _) =
                        ui.allocate_exact_size(egui::vec2(344., 60.), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(popup, 0., color(self.tokens.rename_scope_fill));
                    let row = egui::Rect::from_min_size(
                        popup.min + egui::vec2(1., 1.),
                        egui::vec2(342., 30.),
                    );
                    ui.painter().rect_filled(row, 0., color(self.tokens.panel));
                    let choose =
                        ui.interact(row, ui.scope_id().with("suggestion"), egui::Sense::click());
                    choose.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            true,
                            &self.rename_ui.name,
                        )
                    });
                    ui.painter().text(
                        row.min + egui::vec2(32., 15.),
                        egui::Align2::LEFT_CENTER,
                        &self.rename_ui.name,
                        FontId::monospace(self.tokens.editor_font_size),
                        color(self.tokens.rename_input_text),
                    );
                    if choose.clicked() {
                        self.rename_ui.focus_name = true;
                    }
                    let footer = egui::Rect::from_min_max(
                        popup.min + egui::vec2(1., 31.),
                        popup.max - egui::vec2(1., 1.),
                    );
                    let help =
                        ui.interact(footer, ui.scope_id().with("options"), egui::Sense::click());
                    let label = "Press Alt+Shift+O to show options popup";
                    help.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label)
                    });
                    ui.painter().text(
                        footer.min + egui::vec2(12., 14.),
                        egui::Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(12.),
                        color(self.tokens.muted),
                    );
                    ui.painter().text(
                        footer.right_center() - egui::vec2(12., 0.),
                        egui::Align2::CENTER_CENTER,
                        "⋮",
                        FontId::proportional(16.),
                        color(self.tokens.muted),
                    );
                    if help.clicked()
                        && let Some(inline) = self.rename_ui.inline.as_mut()
                    {
                        inline.options_open = true;
                    }
                });
        }

        if input_changed {
            self.rename_ui.input_error = None;
            ctx.request_repaint();
        }
        if let Some(error) = self.rename_ui.input_error.as_ref().or(self
            .service
            .rename
            .error
            .as_ref())
        {
            egui::Area::new(egui::Id::unique("codisseu.rename.inline.error"))
                .order(egui::Order::Tooltip)
                .fixed_pos(rect.left_bottom())
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(400.);
                        crate::status_messages::status_message(ui, error);
                    });
                });
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            if open {
                if let Some(inline) = self.rename_ui.inline.as_mut() {
                    inline.options_open = false;
                }
                ctx.memory_mut(|memory| {
                    memory.request_focus(egui::Id::unique("codisseu.rename.name"))
                });
            } else {
                self.cancel_rename();
            }
        } else if more {
            self.rename_ui.inline = None;
            self.rename_ui.focus_name = true;
        } else if refactor {
            if self.service.rename.placeholder.as_deref() == Some(self.rename_ui.name.as_str()) {
                self.cancel_rename();
            } else {
                self.start_rename_preview(true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_placeholder_is_located_at_scalar_caret_without_matching_prefixes() {
        let source = TextSnapshot::from_str("/*😀*/ $rainfall; rainfallExtra; café_Δ;");
        assert_eq!(locate(&source, 9, "rainfall", true), Some(7..15));
        assert_eq!(locate(&source, 9, "rainfall", false), None);
        assert_eq!(locate(&source, 21, "rainfall", false), None);
        let text = source.to_string();
        let start = text[..text.find("café_Δ").unwrap()].chars().count();
        assert_eq!(
            locate(&source, start + 2, "café_Δ", false),
            Some(start..start + 6)
        );
        assert_eq!(
            locate(&source, source.len_chars() + 1, "rainfall", true),
            None
        );
    }
}
