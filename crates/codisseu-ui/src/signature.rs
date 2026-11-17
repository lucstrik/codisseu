//! Parameter info is an LSP result. This module only coordinates focus and painting.
use super::*;
use codisseu_core::{Language, SignatureHelpTrigger, ViewId};

pub(super) struct Popup {
    document: DocumentId,
    view: ViewId,
    version: u64,
    caret: usize,
    explicit: bool,
    pub(super) anchor: egui::Pos2,
    rect: Option<egui::Rect>,
    refresh_at: Option<f64>,
    error: Option<String>,
}

impl CodisseuApp {
    pub(super) fn request_signature(&mut self, trigger: SignatureHelpTrigger, retrigger: bool) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let Some(view) = self.service.active_view() else {
            return;
        };
        let (id, version) = (doc.id, doc.version);
        let caret = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, i)| i);
        let explicit = matches!(trigger, SignatureHelpTrigger::Invoked);
        let anchor = self
            .signature_popup
            .as_ref()
            .filter(|p| p.document == id && p.view == view)
            .map_or(self.caret_screen.unwrap_or(egui::pos2(353., 104.)), |p| {
                p.anchor
            });
        self.documentation = None;
        if !retrigger {
            self.completion_popup = None;
        }
        self.signature_popup = Some(Popup {
            document: id,
            view,
            version,
            caret,
            explicit,
            anchor,
            rect: None,
            refresh_at: None,
            error: None,
        });
        let error = self
            .service
            .dispatch(Command::SignatureHelp {
                id,
                expected_version: version,
                character_index: caret,
                trigger,
                is_retrigger: retrigger,
            })
            .err();
        if let Some(popup) = self.signature_popup.as_mut() {
            popup.error = error;
        }
        self.focus_editor = true;
    }

    fn close_signature(&mut self) {
        if self.signature_popup.take().is_some() {
            self.send(Command::CancelSignatureHelp);
        }
    }

    pub(super) fn signature_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.signature_popup.is_some()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.close_signature();
            self.focus_editor = true;
            return true;
        }
        false
    }

    pub(super) fn capture_signature_input(&mut self, ctx: &egui::Context) {
        self.signature_input = None;
        if self.editor_focused(ctx)
            && self.path_dialog.is_none()
            && !self.provider_dialog
            && !self.preferences_dialog
            && self.confirm.is_none()
            && !self.search_input_focused(ctx)
        {
            self.signature_input = ctx
                .input(|i| {
                    i.events.iter().rev().find_map(|e| match e {
                        egui::Event::Text(text) if text.chars().count() == 1 => Some(text.clone()),
                        _ => None,
                    })
                })
                .and_then(|text| {
                    self.service
                        .active_document()
                        .map(|d| (text, d.id, d.version))
                });
        }
    }

    pub(super) fn update_signature(&mut self, ctx: &egui::Context) {
        let input = self.signature_input.take();
        if self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.exit_when_saved
            || (self.documentation.is_some() && !self.documentation_hover.is_automatic())
            || self.service.rename.is_open()
        {
            self.close_signature();
            return;
        }
        let Some(doc) = self.service.active_document() else {
            self.close_signature();
            return;
        };
        let (id, version) = (doc.id, doc.version);
        let caret = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, i)| i);
        let capabilities = Language::for_path(&doc.path).and_then(|language| {
            self.service
                .language
                .signature_help_capabilities
                .get(&language)
        });
        let trigger = input
            .filter(|(_, owner, old_version)| *owner == id && *old_version != version)
            .map(|(text, _, _)| text)
            .filter(|text| {
                capabilities.is_some_and(|caps| {
                    caps.trigger_characters.contains(text)
                        || (self.signature_popup.is_some()
                            && caps.retrigger_characters.contains(text))
                })
            });
        if !self.editor_focused(ctx) || self.editor_metrics().composing {
            self.close_signature();
            return;
        }
        if self
            .signature_popup
            .as_ref()
            .is_some_and(|p| p.document != id || Some(p.view) != self.service.active_view())
        {
            self.close_signature();
        }
        if let Some(text) = trigger {
            let retrigger = self.signature_popup.is_some();
            self.request_signature(SignatureHelpTrigger::Character(text), retrigger);
            return;
        }
        let Some(popup) = self.signature_popup.as_mut() else {
            return;
        };
        if version != popup.version || caret != popup.caret {
            popup.version = version;
            popup.caret = caret;
            popup.error = None;
            popup.refresh_at = Some(ctx.input(|i| i.time) + 0.08);
            self.service.dispatch(Command::CancelSignatureHelp).ok();
        }
        if let Some(at) = popup.refresh_at {
            if ctx.input(|i| i.time) >= at {
                self.request_signature(SignatureHelpTrigger::ContentChange, true);
            } else {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        } else if popup.error.is_none()
            && self.service.signature_help.error.is_none()
            && !self.service.signature_help.is_open()
        {
            self.close_signature();
        }
    }

    pub(super) fn signature_window(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.signature_popup.as_ref() else {
            return;
        };
        if popup.refresh_at.is_some() {
            return;
        }
        let state = &self.service.signature_help;
        let error = popup.error.as_deref().or(state.error.as_deref());
        if state.is_pending() && error.is_none() {
            return;
        }
        let result = state.result.as_ref();
        if !popup.explicit && error.is_none() && result.is_none_or(|r| r.signatures.is_empty()) {
            self.close_signature();
            return;
        }
        let php = self
            .service
            .active_document()
            .is_some_and(|d| Language::for_path(&d.path) == Some(Language::Php));
        let font = FontId::proportional(self.tokens.signature_font_size);
        let mut rows = Vec::new();
        if let Some(result) = result {
            for (index, signature) in result.signatures.iter().enumerate() {
                let active = (result.active_signature == Some(index))
                    .then_some(result.active_parameter)
                    .flatten();
                let active_label = active
                    .and_then(|i| signature.parameters.get(i))
                    .and_then(|p| signature.label.get(p.label_range.clone()))
                    .map(str::to_owned);
                let runs = signature_content::runs(signature, active, php, &self.tokens);
                let mut job = egui::text::LayoutJob::default();
                for run in &runs {
                    job.append(
                        &run.text,
                        run.padding[0],
                        egui::TextFormat {
                            font_id: font.clone(),
                            color: run.color,
                            underline: if run.bold {
                                egui::Stroke::new(0.5, run.color)
                            } else {
                                egui::Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                }
                let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
                let width = self
                    .signature_paint
                    .measure(
                        &runs,
                        self.tokens.signature_font_size,
                        ctx.pixels_per_point(),
                    )
                    .unwrap_or(galley.size().x);
                rows.push((signature.label.clone(), active_label, runs, galley, width));
            }
        }
        if rows.is_empty() {
            let message = error.unwrap_or("Nenhum parâmetro nesta posição").to_owned();
            let galley = ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(
                    message.clone(),
                    font.clone(),
                    color(self.tokens.signature_foreground),
                )
            });
            let width = galley.size().x;
            rows.push((
                message.clone(),
                None,
                vec![signature_paint::Run {
                    text: message,
                    color: color(self.tokens.signature_foreground),
                    bold: false,
                    clip: false,
                    padding: [0., 0.],
                }],
                galley,
                width,
            ));
        }
        if result.is_some_and(|r| r.truncated) {
            let message = "Mais assinaturas: limite de 32 resultados atingido".to_owned();
            let galley = ctx.fonts_mut(|f| {
                f.layout_no_wrap(
                    message.clone(),
                    font,
                    color(self.tokens.signature_foreground),
                )
            });
            let width = galley.size().x;
            rows.push((message, None, vec![], galley, width));
        }
        let available = ctx.content_rect().shrink(4.);
        let padding = egui::vec2(
            if php {
                self.tokens.signature_php_padding
            } else {
                self.tokens.signature_padding[0]
            },
            self.tokens.signature_padding[1],
        );
        let content_width = rows.iter().map(|(_, _, _, _, w)| *w).fold(0f32, f32::max);
        let width = (content_width + padding.x * 2.).min(available.width());
        let height = (rows.len() as f32 * self.tokens.signature_row_height + padding.y * 2.)
            .min(available.height() * 0.6);
        let x = (popup.anchor.x + self.tokens.editor_caret_width / 2. - padding.x)
            .round()
            .min(available.right() - width)
            .max(available.left());
        let above = popup.anchor.y
            - self.tokens.editor_line_height
            - height
            - self.tokens.signature_anchor_gap;
        let below = above < available.top();
        let y = if below { popup.anchor.y + 8. } else { above };
        let y = y
            .round()
            .min(available.bottom() - height - 8.)
            .max(available.top());
        let tip_margin = (f32::from(self.tokens.signature_radius) + 12.).min(width / 2.);
        let tip_x = (popup.anchor.x + self.tokens.editor_caret_width / 2.)
            .round()
            .clamp(x + tip_margin, x + width - tip_margin);
        let tokens = self.tokens.clone();
        let active_overload = result
            .filter(|r| r.signatures.len() > 1)
            .and_then(|r| r.active_signature);
        let renderer = &mut self.signature_paint;
        let response = egui::Area::new(egui::Id::unique("codisseu.signature"))
            .order(egui::Order::Foreground)
            .movable(false)
            .sense(egui::Sense::hover())
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::GenericContainer,
                        true,
                        "Assinatura da chamada",
                    )
                });
                let shadow = egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 12,
                    spread: 0,
                    color: Color32::from_black_alpha(72),
                };
                ui.painter()
                    .add(shadow.as_shape(rect, tokens.signature_radius));
                signature_frame::paint(
                    ui,
                    rect,
                    tip_x,
                    below,
                    color(tokens.signature_background),
                    color(tokens.signature_border),
                    f32::from(tokens.signature_radius),
                );
                let inner = rect.shrink2(padding);
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_clip_rect(inner.intersect(ui.clip_rect()));
                child.spacing_mut().item_spacing = egui::Vec2::ZERO;
                egui::ScrollArea::both()
                    .id_salt("codisseu.signature.scroll")
                    .max_height(inner.height())
                    .max_width(inner.width())
                    .codisseu()
                    .show(&mut child, |ui| {
                        ui.set_min_width(content_width);
                        for (index, (label, active, runs, galley, _)) in rows.iter().enumerate() {
                            let (row, response) = ui.allocate_exact_size(
                                egui::vec2(content_width, tokens.signature_row_height),
                                egui::Sense::hover(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, label)
                            });
                            if let Some(active) = active {
                                let response = ui.interact(
                                    row,
                                    ui.scope_id().with(("signature-active", index)),
                                    egui::Sense::hover(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Label,
                                        true,
                                        format!("Parâmetro ativo: {active}"),
                                    )
                                });
                            }
                            let row_background = if active_overload == Some(index) {
                                color(tokens.signature_overload_background)
                            } else {
                                color(tokens.signature_background)
                            };
                            if active_overload.is_some() {
                                ui.painter().rect_filled(
                                    row.expand2(egui::vec2(4., 0.)),
                                    0.,
                                    row_background,
                                );
                                if index > 0 {
                                    ui.painter().hline(
                                        row.x_range(),
                                        row.top(),
                                        egui::Stroke::new(1., color(tokens.signature_border)),
                                    );
                                }
                            }
                            if !renderer.paint(
                                ui,
                                runs,
                                row.min + egui::vec2(0., tokens.signature_baseline - padding.y),
                                content_width + 2.,
                                row_background,
                                tokens.signature_font_size,
                            ) {
                                ui.painter().galley(
                                    row.min + egui::vec2(0., 4.),
                                    galley.clone(),
                                    color(tokens.signature_foreground),
                                );
                            }
                        }
                    });
            });
        if let Some(popup) = self.signature_popup.as_mut() {
            popup.rect = Some(response.response.rect);
        }
    }
}
