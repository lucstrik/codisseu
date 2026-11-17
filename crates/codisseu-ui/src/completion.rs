use super::*;

pub(super) struct Popup {
    document: DocumentId,
    version: u64,
    caret: usize,
    selected: usize,
    scroll_selection: bool,
    pub anchor: egui::Pos2,
    rect: Option<egui::Rect>,
    templates: Vec<crate::snippets::CompletionTemplate>,
}
#[cfg(test)]
impl Popup {
    pub(super) fn fixture(document: DocumentId, version: u64, caret: usize) -> Self {
        Self {
            document,
            version,
            caret,
            selected: 0,
            scroll_selection: false,
            anchor: egui::Pos2::ZERO,
            rect: None,
            templates: Vec::new(),
        }
    }
}
impl CodisseuApp {
    pub(super) fn completion_has_choices(&self) -> bool {
        self.completion_popup.as_ref().is_some_and(|popup| {
            !popup.templates.is_empty() || !self.service.completion.items.is_empty()
        })
    }

    fn accept_completion_choice(&mut self, index: usize, replace: bool) {
        let Some(popup) = self.completion_popup.as_ref() else {
            return;
        };
        if let Some(choice) = popup.templates.get(index).cloned() {
            let valid = self
                .service
                .active_document()
                .is_some_and(|doc| doc.id == popup.document && doc.version == popup.version)
                && self.caret == Some((popup.document, popup.caret))
                && self
                    .service
                    .active_view()
                    .and_then(|view| self.service.view(view))
                    .is_some_and(|view| {
                        view.selection() == codisseu_core::Selection::caret(popup.caret)
                    });
            if valid {
                self.completion_popup = None;
                self.send(Command::CancelCompletion);
                self.accept_completion_template(choice);
            }
        } else {
            self.send(Command::AcceptCompletion {
                index: index.saturating_sub(popup.templates.len()),
                replace,
            });
        }
        self.focus_editor = true;
    }

    /// Selected semantic item supplied as context to an independent inline provider.
    pub(super) fn copilot_completion_choice(&self) -> Option<crate::copilot::SemanticChoice> {
        let popup = self.completion_popup.as_ref()?;
        let doc = self
            .service
            .active_document()
            .filter(|d| d.id == popup.document && d.version == popup.version)?;
        let semantic_index = popup.selected.checked_sub(popup.templates.len())?;
        let item = self.service.completion.items.get(semantic_index)?;
        if item.unsupported_reason.is_some() || !item.additional_edits.is_empty() {
            return None;
        }
        let range = if let Some(edit) = item.text_edit.as_ref() {
            let start = codisseu_core::utf16_byte_offset(
                doc.text(),
                edit.start_line,
                edit.start_character,
            )?;
            let end =
                codisseu_core::utf16_byte_offset(doc.text(), edit.end_line, edit.end_character)?;
            doc.text()[..start].chars().count()..doc.text()[..end].chars().count()
        } else {
            let (id, version, start) = self.service.completion.anchor()?;
            if id != popup.document || version != popup.version {
                return None;
            }
            start..popup.caret
        };
        Some(crate::copilot::SemanticChoice {
            range,
            text: item.new_text.clone(),
        })
    }
    pub(super) fn editor_focused(&self, ctx: &egui::Context) -> bool {
        self.service.active_view().is_some_and(|id| {
            ctx.memory(|m| m.has_focus(egui::Id::unique(("codisseu.editor", id))))
        })
    }
    pub(super) fn request_completion(&mut self) {
        if self.request_sql_completion() {
            return;
        }
        if self.documentation.is_some() {
            self.close_documentation();
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let version = doc.version;
        let caret = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, i)| i);
        let templates = self.completion_templates(caret);
        let has_templates = !templates.is_empty();
        let provider_ready = codisseu_core::Language::for_path(&doc.path).is_some_and(|language| {
            self.service.language.statuses.iter().any(|status| {
                status.language == language && status.state == codisseu_core::ProviderState::Ready
            })
        });
        self.completion_popup = Some(Popup {
            document: id,
            version,
            caret,
            selected: 0,
            scroll_selection: true,
            anchor: self.caret_screen.unwrap_or(egui::pos2(353., 104.)),
            rect: None,
            templates,
        });
        if has_templates && !provider_ready {
            self.send(Command::CancelCompletion);
        } else {
            self.send(Command::Completion {
                id,
                expected_version: version,
                character_index: caret,
            });
        }
        self.focus_editor = true;
    }
    fn dismiss_completion(&mut self) {
        self.completion_popup = None;
        self.send(Command::CancelCompletion);
    }
    pub(super) fn completion_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.completion_popup.is_none() {
            return false;
        }
        let pointer_inside = self
            .completion_popup
            .as_ref()
            .and_then(|p| p.rect)
            .is_some_and(|r| {
                ctx.input(|i| {
                    i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| r.contains(p))
                })
            });
        if ctx.memory(|m| m.focused()).is_some()
            && !self.editor_focused(ctx)
            && !pointer_inside
            && !self.service.completion.is_applying()
        {
            self.dismiss_completion();
            return false;
        }
        let take = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        if take(Key::Escape) {
            self.dismiss_completion();
            self.focus_editor = true;
            return true;
        }
        let count = self.service.completion.items.len()
            + self
                .completion_popup
                .as_ref()
                .map_or(0, |popup| popup.templates.len());
        if count > 0 {
            if take(Key::ArrowDown) {
                let popup = self.completion_popup.as_mut().unwrap();
                popup.selected = (popup.selected + 1) % count;
                popup.scroll_selection = true;
                return true;
            }
            if take(Key::ArrowUp) {
                let popup = self.completion_popup.as_mut().unwrap();
                popup.selected = (popup.selected + count - 1) % count;
                popup.scroll_selection = true;
                return true;
            }
            let enter = take(Key::Enter);
            let tab = take(Key::Tab);
            if enter || tab {
                let index = self
                    .completion_popup
                    .as_ref()
                    .unwrap()
                    .selected
                    .min(count - 1);
                self.accept_completion_choice(index, tab);
                return true;
            }
        }
        // Other shortcuts/movement keep their normal editor behavior, dismissing stale choices.
        let dismiss=ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key {key,pressed:true,modifiers,..}
            if matches!(key,Key::ArrowLeft|Key::ArrowRight|Key::Home|Key::End|Key::PageUp|Key::PageDown)
                || ((modifiers.command||modifiers.ctrl||modifiers.alt) && *key!=Key::Space))));
        if dismiss {
            self.dismiss_completion();
        }
        false
    }
    pub(super) fn completion_window(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.completion_popup.as_ref() else {
            return;
        };
        let current = self
            .service
            .active_document()
            .is_some_and(|d| d.id == popup.document && d.version == popup.version);
        let moved = self
            .caret
            .is_some_and(|(id, index)| id == popup.document && index != popup.caret);
        let outside = popup.rect.is_some_and(|rect| {
            ctx.input(|i| {
                i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
            })
        });
        if !current
            || moved
            || outside
            || self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.exit_when_saved
        {
            self.dismiss_completion();
            return;
        }
        let anchor = popup.anchor
            + egui::vec2(
                self.tokens.completion_anchor_offset[0],
                self.tokens.completion_anchor_offset[1],
            );
        let selected = popup.selected;
        let scroll_selection = popup.scroll_selection;
        let templates = popup.templates.clone();
        let template_count = templates.len();
        let count = self.service.completion.items.len() + template_count;
        let prefix: String = self
            .service
            .completion
            .anchor()
            .and_then(|(_, _, start)| {
                self.service.active_document().map(|d| {
                    d.text()
                        .chars()
                        .skip(start)
                        .take(popup.caret - start)
                        .collect()
                })
            })
            .unwrap_or_default();
        let available = ctx.content_rect();
        let preferred_width = if self
            .service
            .active_document()
            .is_some_and(|d| d.path.extension().is_some_and(|s| s == "php"))
        {
            self.tokens.completion_php_width
        } else {
            self.tokens.completion_width
        };
        let width = preferred_width.min((available.width() - 8.).max(120.));
        let rows = count.min(self.tokens.completion_max_rows).max(1);
        let height = rows as f32 * self.tokens.completion_row_height
            + self.tokens.completion_footer_height
            + 2. * self.tokens.completion_padding as f32;
        let x = anchor
            .x
            .min(available.right() - width - 4.)
            .max(available.left() + 4.);
        let y = if anchor.y + height <= available.bottom() {
            anchor.y
        } else {
            (anchor.y - height - self.tokens.editor_line_height).max(available.top() + 4.)
        };
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("codisseu.completion"))
            .order(egui::Order::Foreground)
            .movable(false)
            .sense(egui::Sense::hover())
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(self.tokens.panel))
                    .stroke(egui::Stroke::new(1., color(self.tokens.border)))
                    .corner_radius(self.tokens.completion_frame_corner_radius)
                    .inner_margin(egui::Margin {
                        left: self.tokens.completion_padding,
                        right: self.tokens.completion_padding,
                        top: self.tokens.completion_padding,
                        bottom: self.tokens.completion_padding - 1,
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 2. * self.tokens.completion_padding as f32 - 2.);
                        ui.spacing_mut().item_spacing.y = 0.;
                        if count == 0 {
                            let message = if let Some(e) = &self.service.completion.error {
                                e.as_str()
                            } else if self.service.completion.is_pending() {
                                "Consultando sugestões…"
                            } else {
                                "Nenhuma sugestão nesta posição"
                            };
                            egui::ScrollArea::vertical()
                                .max_height(
                                    self.tokens.completion_max_rows as f32
                                        * self.tokens.completion_row_height,
                                )
                                .codisseu()
                                .show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(message).color(color(self.tokens.text)),
                                        )
                                        .wrap(),
                                    );
                                });
                        } else {
                            egui::ScrollArea::vertical()
                                .id_salt("codisseu.completion.list")
                                .max_height(
                                    self.tokens.completion_max_rows as f32
                                        * self.tokens.completion_row_height,
                                )
                                .codisseu()
                                .show(ui, |ui| {
                                    let rows = templates
                                        .iter()
                                        .map(|choice| {
                                            (
                                                choice.template.trigger,
                                                Some(15),
                                                Some(choice.template.description),
                                                None,
                                            )
                                        })
                                        .chain(self.service.completion.items.iter().map(|item| {
                                            (
                                                item.label.as_str(),
                                                item.kind,
                                                item.detail.as_deref(),
                                                item.unsupported_reason.as_deref(),
                                            )
                                        }));
                                    for (index, (label, kind, detail, unsupported_reason)) in
                                        rows.enumerate()
                                    {
                                        ui.push_id(("codisseu.completion.item", index), |ui| {
                                            let (rect, response) = ui.allocate_exact_size(
                                                egui::vec2(
                                                    ui.available_width(),
                                                    self.tokens.completion_row_height,
                                                ),
                                                egui::Sense::click(),
                                            );
                                            response.widget_info(|| {
                                                egui::WidgetInfo::selected(
                                                    egui::accesskit::Role::ListBoxOption,
                                                    true,
                                                    index == selected,
                                                    if index < template_count {
                                                        format!("{label} · Live Template")
                                                    } else {
                                                        label.into()
                                                    },
                                                )
                                            });
                                            if index == selected {
                                                ui.painter().rect_filled(
                                                    rect,
                                                    self.tokens.control_corner_radius,
                                                    color(self.tokens.selection),
                                                );
                                                if scroll_selection {
                                                    response.scroll_to_me(None);
                                                }
                                            } else if response.hovered() {
                                                ui.painter().rect_filled(
                                                    rect,
                                                    0.,
                                                    color(self.tokens.border),
                                                );
                                            }
                                            let painter = ui.painter().with_clip_rect(rect);
                                            let center = rect.left_center() + egui::vec2(14., 0.);
                                            painter.circle_stroke(
                                                center,
                                                6.,
                                                egui::Stroke::new(
                                                    1.,
                                                    color(self.tokens.completion_icon_color),
                                                ),
                                            );
                                            painter.text(
                                                center,
                                                egui::Align2::CENTER_CENTER,
                                                if kind == Some(15) {
                                                    "t"
                                                } else if matches!(kind, Some(2 | 3)) {
                                                    "m"
                                                } else {
                                                    "·"
                                                },
                                                FontId::proportional(10.),
                                                color(self.tokens.completion_icon_color),
                                            );
                                            let text_color =
                                                color(if unsupported_reason.is_some() {
                                                    self.tokens.muted
                                                } else {
                                                    self.tokens.text
                                                });
                                            let font =
                                                if self.service.active_document().is_some_and(|d| {
                                                    d.path.extension().is_some_and(|s| s == "php")
                                                }) {
                                                    FontId::new(
                                                        self.tokens.editor_font_size,
                                                        egui::FontFamily::Name(
                                                            "codisseu.mono.bold".into(),
                                                        ),
                                                    )
                                                } else {
                                                    FontId::monospace(self.tokens.editor_font_size)
                                                };
                                            let label_rect = egui::Rect::from_min_max(
                                                rect.min,
                                                egui::pos2(
                                                    if detail.is_some() {
                                                        rect.left() + rect.width() * 0.6 - 8.
                                                    } else {
                                                        rect.right() - 8.
                                                    },
                                                    rect.bottom(),
                                                ),
                                            );
                                            let mut job = egui::text::LayoutJob::default();
                                            let prefix_bytes = if label
                                                .to_lowercase()
                                                .starts_with(&prefix.to_lowercase())
                                            {
                                                label
                                                    .char_indices()
                                                    .nth(prefix.chars().count())
                                                    .map_or(label.len(), |(i, _)| i)
                                            } else {
                                                0
                                            };
                                            job.append(
                                                &label[..prefix_bytes],
                                                0.,
                                                egui::TextFormat {
                                                    font_id: font.clone(),
                                                    color: color(
                                                        self.tokens.completion_match_color,
                                                    ),
                                                    ..Default::default()
                                                },
                                            );
                                            job.append(
                                                &label[prefix_bytes..],
                                                0.,
                                                egui::TextFormat {
                                                    font_id: font.clone(),
                                                    color: text_color,
                                                    ..Default::default()
                                                },
                                            );
                                            let galley = ui.fonts_mut(|f| f.layout_job(job));
                                            painter.with_clip_rect(label_rect).galley(
                                                rect.left_center()
                                                    + egui::vec2(29., -galley.size().y / 2.),
                                                galley,
                                                text_color,
                                            );
                                            if let Some(detail) = detail {
                                                let detail_rect = egui::Rect::from_min_max(
                                                    egui::pos2(
                                                        rect.left() + width * 0.6,
                                                        rect.top(),
                                                    ),
                                                    rect.max,
                                                );
                                                painter.with_clip_rect(detail_rect).text(
                                                    rect.right_center() - egui::vec2(12., 0.),
                                                    egui::Align2::RIGHT_CENTER,
                                                    detail,
                                                    font,
                                                    color(self.tokens.muted),
                                                );
                                            }
                                            response.clone().on_hover_text(format!(
                                                "{}\n{}",
                                                label,
                                                detail.unwrap_or("")
                                            ));
                                            if let Some(reason) = unsupported_reason {
                                                response.clone().on_hover_text(reason);
                                            }
                                            if response.clicked() {
                                                chosen = Some(index)
                                            }
                                        });
                                    }
                                });
                        }
                        let (footer, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), self.tokens.completion_footer_height),
                            egui::Sense::hover(),
                        );
                        let footer =
                            footer.expand2(egui::vec2(self.tokens.completion_padding as f32, 0.));
                        ui.painter()
                            .rect_filled(footer, 0., color(self.tokens.border));
                        let hint = if selected < template_count {
                            "Live Template · Enter / Tab expandir"
                        } else if self.service.completion.truncated {
                            "Lista limitada · Ctrl+Space para atualizar"
                        } else if self
                            .service
                            .completion
                            .items
                            .get(selected.saturating_sub(template_count))
                            .is_some_and(|item| {
                                item.text_edit.is_some() && item.insert_edit.is_none()
                            })
                        {
                            "Enter / Tab aplicar intervalo LSP"
                        } else if self.service.completion.is_incomplete {
                            "Enter inserir · Tab substituir · parcial"
                        } else {
                            "Enter inserir · Tab substituir"
                        };
                        ui.painter().with_clip_rect(footer).text(
                            footer.left_center() + egui::vec2(8., 0.),
                            egui::Align2::LEFT_CENTER,
                            hint,
                            FontId::proportional(self.tokens.completion_footer_font_size),
                            color(self.tokens.completion_footer_text),
                        );
                    });
            });
        if let Some(popup) = self.completion_popup.as_mut() {
            popup.rect = Some(area.response.rect);
            if count > 0 {
                popup.scroll_selection = false;
            }
        }
        if let Some(index) = chosen {
            self.accept_completion_choice(index, false);
        }
    }
}
