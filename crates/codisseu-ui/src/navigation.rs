use super::*;

impl CodisseuApp {
    fn navigation_cursor(&self) -> Option<(DocumentId, u64, usize)> {
        let doc = self.service.active_document()?;
        let index = self
            .caret
            .filter(|(id, _)| *id == doc.id)
            .map_or(0, |(_, index)| index);
        Some((doc.id, doc.version, index))
    }

    pub(super) fn go_to_definition(&mut self) {
        let Some((id, expected_version, character_index)) = self.navigation_cursor() else {
            return;
        };
        if self.documentation.is_some() {
            self.close_documentation();
        }
        self.definition_choice = 0;
        self.send(Command::Definition {
            id,
            expected_version,
            character_index,
        });
    }

    pub(super) fn go_to_implementation(&mut self) {
        let Some((id, expected_version, character_index)) = self.navigation_cursor() else {
            return;
        };
        if self.documentation.is_some() {
            self.close_documentation();
        }
        self.definition_choice = 0;
        self.send(Command::Implementation {
            id,
            expected_version,
            character_index,
        });
    }

    pub(super) fn go_to_super_definition(&mut self) {
        let Some((id, expected_version, character_index)) = self.navigation_cursor() else {
            return;
        };
        if self.documentation.is_some() {
            self.close_documentation();
        }
        self.definition_choice = 0;
        self.send(Command::SuperDefinition {
            id,
            expected_version,
            character_index,
        });
    }

    pub(super) fn go_to_type_definition(&mut self) {
        let Some((id, expected_version, character_index)) = self.navigation_cursor() else {
            return;
        };
        if self.documentation.is_some() {
            self.close_documentation();
        }
        self.definition_choice = 0;
        self.send(Command::TypeDefinition {
            id,
            expected_version,
            character_index,
        });
    }

    pub(super) fn navigate_history(&mut self, forward: bool) {
        let Some((id, expected_version, character_index)) = self.navigation_cursor() else {
            return;
        };
        if self.documentation.is_some() {
            self.close_documentation();
        }
        self.send(if forward {
            Command::NavigateForward {
                id,
                expected_version,
                character_index,
            }
        } else {
            Command::NavigateBack {
                id,
                expected_version,
                character_index,
            }
        });
        self.focus_editor = true;
    }

    pub(super) fn navigation_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let count = self.service.navigation.choices.len();
        if count > 1 {
            let key = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
            if key(Key::Escape) {
                self.send(Command::CancelDefinition);
                self.focus_editor = true;
            } else if key(Key::ArrowDown) {
                self.definition_choice = (self.definition_choice + 1) % count;
            } else if key(Key::ArrowUp) {
                self.definition_choice = (self.definition_choice + count - 1) % count;
            } else if key(Key::Enter) {
                self.send(Command::ChooseDefinition {
                    index: self.definition_choice.min(count - 1),
                });
            }
            return true;
        }
        if self.service.navigation.is_pending()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.send(Command::CancelDefinition);
            self.focus_editor = true;
            return true;
        }
        false
    }

    pub(super) fn definition_choices(&mut self, ctx: &egui::Context) {
        let count = self.service.navigation.choices.len();
        if count < 2 {
            return;
        }
        let implementation = self.service.navigation.is_implementation();
        let presentation = self.service.navigation.presentation.as_ref();
        let symbol = presentation
            .map(|p| p.source_symbol.as_str())
            .filter(|s| !s.is_empty());
        let super_definition = self.service.navigation.is_super_definition();
        let type_definition = self.service.navigation.is_type_definition();
        let title = if super_definition {
            format!("Escolher supermétodo/supertipo ({count})")
        } else if type_definition {
            format!("Escolher tipo ({count})")
        } else {
            match (implementation, symbol) {
                (true, Some(s)) => format!("Choose Implementation of {s} ({count} found)"),
                (true, None) => format!("Choose Implementation ({count} found)"),
                (false, Some(s)) => format!("Choose Declaration of {s} ({count} found)"),
                (false, None) => format!("Choose Declaration ({count} found)"),
            }
        };
        let accessible = if super_definition {
            "Escolher supermétodo/supertipo"
        } else if type_definition {
            "Escolher tipo"
        } else if implementation {
            "Escolher implementação"
        } else {
            "Escolher definição"
        };
        let font = FontId::new(
            self.tokens.ui_font_size,
            egui::FontFamily::Name("codisseu.inter.semibold".into()),
        );
        let mono = FontId::monospace(self.tokens.editor_font_size);
        let foreground = color(self.tokens.navigation_popup_text);
        let title_galley = ctx.fonts_mut(|f| f.layout_no_wrap(title.clone(), font, foreground));
        let padding = f32::from(self.tokens.navigation_popup_padding);
        let mut width = (title_galley.size().x + padding * 2. + 35.)
            .max(self.tokens.navigation_popup_min_width);
        let rows = self
            .service
            .navigation
            .choices
            .iter()
            .enumerate()
            .map(|(index, target)| {
                let detail = presentation
                    .and_then(|p| p.targets.get(index))
                    .and_then(Option::as_ref);
                let name = detail.map(|d| d.name.clone()).unwrap_or_else(|| {
                    target
                        .path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
                let namespace = detail.map_or_else(
                    || {
                        target
                            .path
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .strip_prefix(
                                self.service
                                    .workspace
                                    .as_deref()
                                    .unwrap_or(std::path::Path::new("")),
                            )
                            .unwrap_or(std::path::Path::new(""))
                            .to_string_lossy()
                            .into_owned()
                    },
                    |d| d.namespace.clone(),
                );
                let label = navigation_label(
                    ctx,
                    name,
                    mono.clone(),
                    foreground,
                    self.tokens.editor_letter_spacing,
                );
                let context = navigation_label(
                    ctx,
                    namespace,
                    mono.clone(),
                    foreground,
                    self.tokens.editor_letter_spacing,
                );
                width =
                    width.max(padding * 2. + 46. + label.size().x + 24. + context.size().x + 12.);
                let path = target
                    .path
                    .strip_prefix(
                        self.service
                            .workspace
                            .as_deref()
                            .unwrap_or(std::path::Path::new("")),
                    )
                    .unwrap_or(&target.path);
                (
                    label,
                    context,
                    detail.map_or('·', |d| d.kind),
                    detail.is_some_and(|d| d.public),
                    format!(
                        "{}:{}:{}",
                        path.display(),
                        target.start_line + 1,
                        target.start_character + 1
                    ),
                )
            })
            .collect::<Vec<_>>();
        let available = ctx.content_rect();
        width = width.min((available.width() - 16.).max(100.));
        let row_height = self.tokens.navigation_popup_row_height;
        let height = padding * 2.
            + self.tokens.navigation_popup_header_height
            + self.tokens.navigation_popup_header_gap
            + row_height * count.min(8) as f32
            + 2.;
        let anchor = self.caret_screen.unwrap_or(available.center());
        let x = anchor
            .x
            .min(available.right() - width - 8.)
            .max(available.left() + 8.);
        let y = if anchor.y + height <= available.bottom() - 8. {
            anchor.y
        } else {
            (anchor.y - height - self.tokens.editor_line_height).max(available.top() + 8.)
        };
        // Align one-pixel popup edges to the physical grid at both native scales.
        let scale = ctx.pixels_per_point();
        width = (width * scale).round() / scale;
        let position = egui::pos2((x * scale).round() / scale, (y * scale).round() / scale);
        let mut chosen = None;
        let mut tool_window = false;
        let area = egui::Area::new(egui::Id::unique("codisseu.definition.choices"))
            .order(egui::Order::Foreground)
            .movable(false)
            .sense(egui::Sense::hover())
            .fixed_pos(position)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(self.tokens.navigation_popup_background))
                    .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
                    .corner_radius(0)
                    .inner_margin(self.tokens.navigation_popup_padding)
                    .show(ui, |ui| {
                        ui.set_width(width - padding * 2. - 2.);
                        ui.spacing_mut().item_spacing.y = 0.;
                        let (header, _) = ui.allocate_exact_size(
                            egui::vec2(
                                ui.available_width(),
                                self.tokens.navigation_popup_header_height,
                            ),
                            egui::Sense::hover(),
                        );
                        let header_response = ui.interact(
                            header,
                            egui::Id::unique("codisseu.definition.title"),
                            egui::Sense::hover(),
                        );
                        header_response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Label,
                                true,
                                accessible,
                            )
                        });
                        let title_clip =
                            egui::Rect::from_min_max(header.min, header.max - egui::vec2(22., 0.));
                        ui.painter().with_clip_rect(title_clip).galley(
                            egui::pos2(
                                header.left() + 6.,
                                header.center().y - title_galley.size().y / 2.,
                            ),
                            title_galley.clone(),
                            foreground,
                        );
                        let pin = egui::Rect::from_center_size(
                            header.right_center() - egui::vec2(13., 0.),
                            egui::vec2(18., 18.),
                        );
                        let response = ui.interact(
                            pin,
                            egui::Id::unique("codisseu.definition.tool.window"),
                            egui::Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Button,
                                true,
                                "Abrir destinos na janela Referências",
                            )
                        });
                        if response.hovered() {
                            ui.painter().rect_filled(pin, 3., color(self.tokens.border));
                        }
                        let c = pin.center();
                        let stroke = egui::Stroke::new(1., foreground);
                        ui.painter().rect_stroke(
                            egui::Rect::from_center_size(c, egui::vec2(10., 10.)),
                            1.,
                            stroke,
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().line_segment(
                            [c + egui::vec2(2., -2.), c + egui::vec2(-2., 2.)],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [c + egui::vec2(-2., -1.), c + egui::vec2(-2., 2.)],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [c + egui::vec2(-2., 2.), c + egui::vec2(1., 2.)],
                            stroke,
                        );
                        tool_window = response
                            .on_hover_text("Abrir destinos na janela Referências")
                            .clicked();
                        ui.add_space(self.tokens.navigation_popup_header_gap);
                        egui::ScrollArea::vertical()
                            .id_salt("codisseu.definition.list")
                            .max_height(row_height * 8.)
                            .codisseu()
                            .show(ui, |ui| {
                                for (index, (label, namespace, kind, public, path)) in
                                    rows.iter().enumerate()
                                {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), row_height),
                                        egui::Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::accesskit::Role::ListBoxOption,
                                            true,
                                            index == self.definition_choice,
                                            path,
                                        )
                                    });
                                    if response.hovered()
                                        && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                                    {
                                        self.definition_choice = index;
                                    }
                                    if index == self.definition_choice {
                                        ui.painter().rect_filled(
                                            rect,
                                            4.,
                                            color(self.tokens.navigation_popup_selection),
                                        );
                                        if ui.input(|i| {
                                            i.key_pressed(Key::ArrowUp)
                                                || i.key_pressed(Key::ArrowDown)
                                        }) {
                                            response.scroll_to_me(None);
                                        }
                                    }
                                    let paint = ui.painter().with_clip_rect(rect);
                                    let center = rect.left_center() + egui::vec2(16., 0.);
                                    paint.circle_stroke(
                                        center,
                                        6.,
                                        egui::Stroke::new(
                                            1.,
                                            color(self.tokens.completion_icon_color),
                                        ),
                                    );
                                    paint.text(
                                        center,
                                        egui::Align2::CENTER_CENTER,
                                        kind,
                                        FontId::proportional(10.),
                                        color(self.tokens.completion_icon_color),
                                    );
                                    if *public {
                                        let lock = center + egui::vec2(17., 0.);
                                        let stroke = egui::Stroke::new(
                                            1.,
                                            color(self.tokens.navigation_popup_public_color),
                                        );
                                        paint.rect_stroke(
                                            egui::Rect::from_min_size(
                                                lock - egui::vec2(3., 1.),
                                                egui::vec2(5., 4.),
                                            ),
                                            0.,
                                            stroke,
                                            egui::StrokeKind::Inside,
                                        );
                                        paint.line_segment(
                                            [
                                                lock + egui::vec2(1., -1.),
                                                lock + egui::vec2(1., -4.),
                                            ],
                                            stroke,
                                        );
                                        paint.line_segment(
                                            [
                                                lock + egui::vec2(1., -4.),
                                                lock + egui::vec2(4., -4.),
                                            ],
                                            stroke,
                                        );
                                    }
                                    let at = egui::pos2(
                                        rect.left() + 46.,
                                        rect.center().y - label.size().y / 2.,
                                    );
                                    paint.galley(at, label.clone(), foreground);
                                    paint.galley(
                                        egui::pos2(
                                            at.x + label.size().x + 24.,
                                            rect.center().y - namespace.size().y / 2.,
                                        ),
                                        namespace.clone(),
                                        foreground,
                                    );
                                    if response.clicked() {
                                        chosen = Some(index);
                                    }
                                    response.on_hover_text(path);
                                }
                            });
                    });
            });
        let outside = ctx.input(|i| {
            !i.focused
                || (i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !area.response.rect.contains(p)))
        });
        if let Some(index) = chosen {
            self.send(Command::ChooseDefinition { index });
        } else if tool_window {
            self.show_navigation_targets();
        } else if outside {
            self.send(Command::CancelDefinition);
            self.focus_editor = true;
        }
    }
}

fn navigation_label(
    ctx: &egui::Context,
    text: String,
    font: FontId,
    foreground: Color32,
    spacing: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text, font, foreground);
    for section in &mut job.sections {
        section.format.extra_letter_spacing = spacing;
    }
    ctx.fonts_mut(|fonts| fonts.layout_job(job))
}
