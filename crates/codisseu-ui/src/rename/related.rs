//! Review the exact declaration set certified by the worker, before application.
use super::*;
impl CodisseuApp {
    pub(crate) fn related_methods_dialog(&mut self, ctx: &egui::Context) {
        if !self.service.rename.choosing_methods() {
            return;
        }
        self.show_activity = false;
        let Some(token) = self.service.rename.context_token() else {
            return;
        };
        let methods = self.service.rename.related_methods().to_vec();
        if methods.is_empty() {
            return;
        }
        if self.rename_ui.related_token != Some(token) {
            self.rename_ui.related_token = Some(token);
            self.rename_ui.related_selected = 0;
            self.rename_ui.focus_related = true;
            ctx.request_repaint();
        }
        let row_id = |index| egui::Id::unique(("codisseu.rename.related.row", token, index));
        let direction = if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
            1
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
            -1
        } else {
            0
        };
        if direction != 0 {
            self.rename_ui.related_selected = self
                .rename_ui
                .related_selected
                .saturating_add_signed(direction)
                .min(methods.len() - 1);
            self.rename_ui.focus_related = true;
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        let mut toggles = vec![];
        let mut confirm = false;
        let mut cancel = false;
        let width = self
            .tokens
            .related_dialog_width
            .min(ctx.content_rect().width() - 32.);
        let height = self
            .tokens
            .related_dialog_height
            .min(ctx.content_rect().height() - 32.);
        egui::Modal::new(egui::Id::unique("codisseu.rename.related"))
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::new().fill(color(self.tokens.panel)))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.visuals_mut().override_text_color = Some(color(self.tokens.rename_text));
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                let inner = rect.shrink2(egui::vec2(12., 8.));
                let mut title = child(
                    ui,
                    egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), 32.)),
                    "related.title",
                );
                title.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.label(
                        "Métodos relacionados encontrados. Revise as declarações antes de renomear.",
                    );
                });
                let table = egui::Rect::from_min_size(
                    inner.min + egui::vec2(0., 32.),
                    egui::vec2(inner.width(), self.tokens.related_table_height),
                );
                ui.painter().rect_stroke(
                    table,
                    0.,
                    egui::Stroke::new(1., color(self.tokens.border)),
                    egui::StrokeKind::Inside,
                );
                let header = egui::Rect::from_min_size(
                    table.min + egui::vec2(1., 1.),
                    egui::vec2(table.width() - 2., self.tokens.related_header_height),
                );
                let mid = table.left() + 28. + (table.width() - 28.) / 2.;
                ui.painter().hline(
                    table.x_range(),
                    header.bottom(),
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                for (x, text) in [(table.left() + 37., "Método"), (mid + 8., "Renomear para")] {
                    ui.painter().text(
                        egui::pos2(x, header.center().y),
                        egui::Align2::LEFT_CENTER,
                        text,
                        FontId::proportional(self.tokens.ui_font_size),
                        color(self.tokens.rename_text),
                    );
                }
                let mut body = child(
                    ui,
                    egui::Rect::from_min_max(
                        header.left_bottom(),
                        table.right_bottom() - egui::vec2(1., 1.),
                    ),
                    "related.table",
                );
                body.spacing_mut().item_spacing.y = 0.;
                egui::ScrollArea::vertical()
                    .id_salt("codisseu.related.scroll")
                    .codisseu().show_rows(
                        &mut body,
                        self.tokens.related_row_height,
                        methods.len(),
                        |ui, visible| {
                            for index in visible {
                                let method = &methods[index];
                                let (row, _) = ui.allocate_exact_size(
                                    egui::vec2(
                                        ui.available_width(),
                                        self.tokens.related_row_height,
                                    ),
                                    egui::Sense::hover(),
                                );
                                let selected = self.rename_ui.related_selected == index;
                                let response =
                                    ui.interact(row, row_id(index), egui::Sense::click());
                                let label = format!("{}.{}()", method.container_name, method.name);
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::accesskit::Role::ListBoxOption,
                                        true,
                                        selected,
                                        &label,
                                    )
                                });
                                if selected {
                                    ui.painter().rect_filled(
                                        row,
                                        0.,
                                        color(self.tokens.rename_tree_selection),
                                    );
                                }
                                if response.clicked() {
                                    self.rename_ui.related_selected = index;
                                    response.request_focus();
                                }
                                if selected && self.rename_ui.focus_related {
                                    response.request_focus();
                                    response.scroll_to_me(None);
                                    self.rename_ui.focus_related = false;
                                }
                                let mut check = self.service.rename.method_selected(index);
                                let mut checkbox = child(
                                    ui,
                                    egui::Rect::from_min_size(
                                        row.min + egui::vec2(4., 0.),
                                        egui::vec2(23., row.height()),
                                    ),
                                    ("related.check", index),
                                );
                                let (_, check_response) = checkbox.allocate_exact_size(
                                    egui::vec2(23., row.height()),
                                    egui::Sense::click(),
                                );
                                if check_response.clicked() {
                                    check_response.request_focus();
                                    check = !check;
                                    toggles.push((index, check));
                                }
                                check_response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::accesskit::Role::CheckBox,
                                        true,
                                        check,
                                        format!("Incluir {label}"),
                                    )
                                });
                                let box_rect = egui::Rect::from_center_size(
                                    egui::pos2(row.left() + 13., row.top() + 10.),
                                    egui::vec2(16., 16.),
                                );
                                ui.painter().rect_filled(
                                    box_rect,
                                    3.,
                                    color(if check {
                                        self.tokens.accent
                                    } else {
                                        self.tokens.panel
                                    }),
                                );
                                if !check {
                                    ui.painter().rect_stroke(
                                    box_rect,
                                    3.,
                                    egui::Stroke::new(
                                        1.,
                                        color(if check_response.has_focus() {
                                            self.tokens.accent
                                        } else {
                                            self.tokens.control_border
                                        }),
                                    ),
                                    egui::StrokeKind::Inside,
                                );
                                }
                                if check {
                                    ui.painter().line(
                                        vec![
                                            box_rect.min + egui::vec2(3.5, 8.5),
                                            box_rect.min + egui::vec2(6.5, 11.),
                                            box_rect.min + egui::vec2(12.5, 4.5),
                                        ],
                                        egui::Stroke::new(2., Color32::WHITE),
                                    );
                                }
                                if check_response.has_focus() {
                                    ui.painter().rect_stroke(
                                        box_rect.expand(2.), 4.,
                                        egui::Stroke::new(1., color(self.tokens.accent)),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                                // The row itself also has a keyboard toggle, including immediately after a click.
                                if response.has_focus()
                                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
                                {
                                    toggles
                                        .push((index, !self.service.rename.method_selected(index)));
                                }
                                let background = color(if selected {
                                    self.tokens.rename_tree_selection
                                } else {
                                    self.tokens.panel
                                });
                                source::paint_label(
                                    &mut self.rename_ui.labels,
                                    ui,
                                    &label,
                                    row.left_center() + egui::vec2(30., -1.),
                                    egui::Rect::from_min_max(
                                        row.min + egui::vec2(28., 0.),
                                        egui::pos2(mid, row.bottom()),
                                    ),
                                    background,
                                    &self.tokens,
                                );
                                source::paint_label(
                                    &mut self.rename_ui.labels,
                                    ui,
                                    &self.service.rename.new_name,
                                    egui::pos2(mid + 4., row.center().y - 1.),
                                    egui::Rect::from_min_max(egui::pos2(mid, row.top()), row.max),
                                    background,
                                    &self.tokens,
                                );
                            }
                        },
                    );
                let mut bulk = child(
                    ui,
                    egui::Rect::from_min_size(
                        table.left_bottom() + egui::vec2(3., 3.),
                        egui::vec2(table.width() - 6., 28.),
                    ),
                    "related.bulk",
                );
                bulk.spacing_mut().item_spacing.x = 11.;
                bulk.spacing_mut().button_padding.x = 8.;
                bulk.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let none = ui.add(
                        self.tokens
                            .dialog_button("Nenhum", false)
                            .min_size(egui::vec2(100., 28.)),
                    );
                    none.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            true,
                            "Desmarcar todos",
                        )
                    });
                    if none.clicked() {
                        toggles.extend((0..methods.len()).map(|n| (n, false)));
                    }
                    let all = ui.add(
                        self.tokens
                            .dialog_button("Todos", false)
                            .min_size(egui::vec2(83., 28.)),
                    );
                    all.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            true,
                            "Selecionar todos",
                        )
                    });
                    if all.clicked() {
                        toggles.extend((0..methods.len()).map(|n| (n, true)));
                    }
                });
                let method = &methods[self.rename_ui.related_selected];
                let source_top = table.bottom() + 40.;
                let mut file_header = child(
                    ui,
                    egui::Rect::from_min_max(
                        egui::pos2(inner.left(), source_top),
                        egui::pos2(inner.right(), source_top + 17.),
                    ),
                    "related.file",
                );
                file_header.label(
                    method
                        .path
                        .strip_prefix(
                            self.service
                                .workspace
                                .as_deref()
                                .unwrap_or(std::path::Path::new("")),
                        )
                        .unwrap_or(&method.path)
                        .display()
                        .to_string(),
                );
                let code_rect = egui::Rect::from_min_max(
                    egui::pos2(inner.left(), source_top + 18.),
                    egui::pos2(inner.right(), inner.bottom() - 42.),
                );
                ui.painter()
                    .rect_filled(code_rect, 0., color(self.tokens.editor));
                if let Some(source) = self
                    .service
                    .rename
                    .conflict_previews
                    .iter()
                    .find(|s| s.path == method.path)
                {
                    let mut code = child(ui, code_rect, "related.code");
                    code.set_clip_rect(code_rect.intersect(ui.clip_rect()));
                    code.spacing_mut().item_spacing.y = 0.;
                    egui::ScrollArea::both()
                        .id_salt(("related.source", token, self.rename_ui.related_selected))
                        .codisseu().show(&mut code, |ui| {
                            let (lines, truncated) =
                                preview_lines(&source.before, method.range.start_line);
                            for (number, line) in lines {
                                let shaped = source::SourceLine::new(
                                    ui,
                                    line,
                                    method
                                        .path
                                        .extension()
                                        .and_then(|p| p.to_str())
                                        .unwrap_or(""),
                                    &self.tokens,
                                    self.service.editor_preferences().ligatures,
                                );
                                let galley = &shaped.galley;
                                let (row, _) = ui.allocate_exact_size(
                                    egui::vec2(
                                        ui.available_width().max(42. + galley.size().x),
                                        self.tokens.editor_line_height,
                                    ),
                                    egui::Sense::hover(),
                                );
                                if number == method.range.start_line as usize {
                                    ui.painter()
                                        .rect_filled(row, 0., color(self.tokens.caret_row));
                                }
                                if let Some(columns) = conflicts::conflict_symbol_columns(
                                    line,
                                    number,
                                    Some((
                                        method.range.start_line,
                                        method.range.start_character,
                                        method.range.end_line,
                                        method.range.end_character,
                                    )),
                                ) {
                                    let start = shaped.x(columns.start).unwrap_or(0.);
                                    let end = shaped.x(columns.end).unwrap_or(start);
                                    ui.painter().rect_filled(
                                        egui::Rect::from_min_size(
                                            row.min + egui::vec2(42. + start, 0.),
                                            egui::vec2(
                                                (end - start).max(0.),
                                                self.tokens.editor_line_height,
                                            ),
                                        ),
                                        0.,
                                        color(self.tokens.rename_conflict_name_background),
                                    );
                                }
                                ui.painter().text(
                                    row.left_center() + egui::vec2(30., 0.),
                                    egui::Align2::RIGHT_CENTER,
                                    (number + 1).to_string(),
                                    FontId::monospace(self.tokens.editor_font_size),
                                    color(self.tokens.muted),
                                );
                                ui.painter().galley(
                                    row.min + egui::vec2(42., 0.),
                                    galley.clone(),
                                    color(self.tokens.text),
                                );
                            }
                            if truncated {
                                ui.label("Preview limitado: até7linhas/8KiB");
                            }
                        });
                }
                let footer = egui::Rect::from_min_max(
                    egui::pos2(inner.left(), inner.bottom() - 42.),
                    inner.max,
                );
                let mut status = child(
                    ui,
                    egui::Rect::from_min_max(footer.min, footer.max - egui::vec2(180., 0.)),
                    "related.status",
                );
                status.label(if self.service.rename.can_confirm_methods() {
                    "A cadeia completa será renomeada; arquivos não serão salvos."
                } else {
                    "A operação exige todas as declarações. Selecione todos para continuar."
                });
                let mut buttons = child(
                    ui,
                    egui::Rect::from_min_max(
                        egui::pos2(inner.right() - 159., inner.bottom() - 31.),
                        inner.max - egui::vec2(3., 3.),
                    ),
                    "related.buttons",
                );
                buttons.spacing_mut().item_spacing.x = 12.;
                buttons.spacing_mut().button_padding.x = 8.;
                buttons.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    cancel = ui
                        .add(self.tokens.dialog_button("Cancelar", false))
                        .clicked();
                    confirm = ui
                        .add_enabled(
                            self.service.rename.can_confirm_methods(),
                            self.tokens
                                .dialog_button("OK", true)
                                .fill(color(self.tokens.rename_primary)),
                        )
                        .clicked();
                });
            });
        for (index, selected) in toggles {
            self.send(Command::SelectRenameMethod {
                token,
                index,
                selected,
            });
        }
        if cancel || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_rename();
        } else if confirm
            || (self.service.rename.can_confirm_methods()
                && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)))
        {
            self.send(Command::ConfirmRenameMethods { token });
            if !self.service.rename.choosing_methods() {
                self.tool_tab = ToolTab::Rename;
                self.show_activity = true;
                self.rename_ui.focus_results = true;
                ctx.request_repaint();
            }
        }
    }
}
fn child(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> egui::Ui {
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child
}
