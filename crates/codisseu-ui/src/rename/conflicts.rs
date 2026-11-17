//! A conflict is a blocking analysis result, never an invitation to apply anyway.
use super::*;
use std::sync::Arc;

#[derive(Clone)]
struct ConflictRow {
    path: PathBuf,
    range: Option<(u32, u32, u32, u32)>,
    container: Option<String>,
    message: String,
}
fn conflict_id(token: Option<u64>, index: usize) -> egui::Id {
    egui::Id::unique(("codisseu.rename.conflict", token, index))
}
impl CodisseuApp {
    pub(crate) fn sync_rename_conflicts(&mut self) {
        if !self.service.rename.semantic_blocked() {
            self.rename_ui.blocked_token = None;
            self.rename_ui.conflict_open = false;
        } else if self.rename_ui.blocked_token != self.service.rename.context_token() {
            self.rename_ui.blocked_token = self.service.rename.context_token();
            self.rename_ui.conflict_open = true;
            self.rename_ui.conflict_selected = 0;
            self.rename_ui.focus_conflict = true;
            self.rename_ui.dialog = false;
            self.show_activity = false;
        }
    }
    pub(crate) fn rename_conflicts(&mut self, ctx: &egui::Context) {
        let token = self.service.rename.context_token();
        if !self.rename_ui.conflict_open {
            return;
        }
        let Some(report) = self.service.rename.validation.as_ref() else {
            return;
        };
        let rejected = report.coverage == codisseu_core::RenameCoverage::Rejected;
        let conflict_count = report.conflicts.len();
        let hierarchy =
            report.scope == codisseu_core::ValidationScope::KnownHierarchyMemberNamespaces;
        let mut rows: Vec<_> = report
            .conflicts
            .iter()
            .map(|c| ConflictRow {
                path: c.path.clone(),
                range: Some((
                    c.range.start_line,
                    c.range.start_character,
                    c.range.end_line,
                    c.range.end_character,
                )),
                container: c.container_name.clone(),
                message: c.message.clone(),
            })
            .collect();
        rows.extend(report.issues.iter().map(|c| {
            ConflictRow {
                path: c.path.clone(),
                range: c
                    .range
                    .as_ref()
                    .map(|r| (r.start_line, r.start_character, r.end_line, r.end_character)),
                container: None,
                message: c.message.clone(),
            }
        }));
        if rows.is_empty() {
            return;
        }
        let count = rows.len();
        self.rename_ui.conflict_selected = self.rename_ui.conflict_selected.min(count - 1);
        let key = |k| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
        let click_before_arrow = click_before_vertical_arrow(ctx);
        let direction = if key(Key::ArrowDown) {
            1_isize
        } else if key(Key::ArrowUp) {
            -1
        } else {
            0
        };
        let deferred = direction != 0 && click_before_arrow;
        if direction != 0 && !deferred {
            self.rename_ui.conflict_selected = self
                .rename_ui
                .conflict_selected
                .saturating_add_signed(direction)
                .min(count - 1);
            self.rename_ui.focus_conflict = true;
            ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
        let mut cancel = key(Key::Escape);
        let mut change_name = false;
        let width = self
            .tokens
            .rename_conflict_width
            .min((ctx.content_rect().width() - 32.).max(240.));
        let height = self
            .tokens
            .rename_conflict_height
            .min((ctx.content_rect().height() - 32.).max(240.));
        egui::Modal::new(egui::Id::unique("codisseu.rename.conflicts"))
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::new().fill(color(self.tokens.panel)))
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(color(self.tokens.rename_text));
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                let toolbar = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(width, self.tokens.rename_conflict_toolbar_height),
                );
                let footer = egui::Rect::from_min_max(
                    egui::pos2(
                        rect.left(),
                        rect.bottom() - self.tokens.rename_conflict_footer_height,
                    ),
                    rect.max,
                );
                let tree_bottom = (toolbar.bottom() + self.tokens.rename_conflict_tree_height)
                    .min(footer.top() - self.tokens.rename_conflict_preview_header_height - 80.);
                let tree_rect = egui::Rect::from_min_max(
                    toolbar.left_bottom(),
                    egui::pos2(rect.right(), tree_bottom),
                );
                let preview_header = egui::Rect::from_min_size(
                    tree_rect.left_bottom(),
                    egui::vec2(width, self.tokens.rename_conflict_preview_header_height),
                );
                let code_rect =
                    egui::Rect::from_min_max(preview_header.left_bottom(), footer.right_top());
                let mut title = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(toolbar.shrink2(egui::vec2(12., 0.)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                title.label("Verificação de rename");
                title.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(if rejected {
                        format!(
                            "{} conflito{}",
                            conflict_count,
                            if conflict_count == 1 { "" } else { "s" }
                        )
                    } else {
                        "Verificação indisponível".into()
                    });
                });
                ui.painter().hline(
                    toolbar.x_range(),
                    toolbar.bottom(),
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                let mut tree = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(tree_rect)
                        .id_salt("codisseu.rename.conflict.tree")
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                tree.set_clip_rect(tree_rect.intersect(ui.clip_rect()));
                egui::ScrollArea::both()
                    .id_salt("codisseu.rename.conflict.scroll")
                    .codisseu()
                    .show(&mut tree, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add_space(4.);
                        rename_group(
                            ui,
                            34.,
                            if rejected && hierarchy {
                                "Conflitos na hierarquia"
                            } else if rejected {
                                "Conflitos na classe"
                            } else {
                                "Análise não suportada nesta estrutura"
                            },
                            &self.tokens,
                        );
                        let workspace = self
                            .service
                            .workspace
                            .as_deref()
                            .and_then(|p| p.file_name())
                            .map(|p| p.to_string_lossy())
                            .unwrap_or_default();
                        rename_group(ui, 58., &workspace, &self.tokens);
                        let mut previous_path = None;
                        for (index, item) in rows.iter().enumerate() {
                            let path = item
                                .path
                                .strip_prefix(
                                    self.service
                                        .workspace
                                        .as_deref()
                                        .unwrap_or(std::path::Path::new("")),
                                )
                                .unwrap_or(&item.path);
                            if previous_path != Some(&item.path) {
                                let folder = path.parent().unwrap_or(std::path::Path::new(""));
                                rename_group(
                                    ui,
                                    77.,
                                    if folder.as_os_str().is_empty() {
                                        "."
                                    } else {
                                        folder.to_str().unwrap_or("?")
                                    },
                                    &self.tokens,
                                );
                                rename_group(
                                    ui,
                                    96.,
                                    &path.file_name().unwrap_or_default().to_string_lossy(),
                                    &self.tokens,
                                );
                                if let Some(container) = &item.container {
                                    rename_group(ui, 115., container, &self.tokens);
                                }
                                previous_path = Some(&item.path);
                            }
                            let font = FontId::proportional(self.tokens.ui_font_size);
                            let galley = ui.painter().layout_no_wrap(
                                item.message.clone(),
                                font,
                                color(self.tokens.rename_text),
                            );
                            let (row, _) = ui.allocate_exact_size(
                                egui::vec2(
                                    ui.available_width().max(140. + galley.size().x),
                                    self.tokens.row_height,
                                ),
                                egui::Sense::hover(),
                            );
                            let response =
                                ui.interact(row, conflict_id(token, index), egui::Sense::click());
                            let selected = self.rename_ui.conflict_selected == index;
                            let label = format!(
                                "Conflito {}:{} — {}",
                                path.display(),
                                item.range.map_or(1, |r| r.0 + 1),
                                item.message
                            );
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
                                    row.shrink2(egui::vec2(12., 0.)),
                                    self.tokens.control_corner_radius,
                                    color(if response.has_focus() {
                                        self.tokens.rename_tree_selection
                                    } else {
                                        self.tokens.references_inactive_selection
                                    }),
                                );
                                if self.rename_ui.focus_conflict {
                                    if !response.has_focus() {
                                        response.request_focus();
                                    }
                                    response.scroll_to_me(None);
                                    self.rename_ui.focus_conflict = false;
                                }
                            }
                            if response.has_focus() || response.clicked() {
                                ui.memory_mut(|m| {
                                    m.set_focus_lock_filter(
                                        response.id,
                                        egui::EventFilter {
                                            vertical_arrows: true,
                                            horizontal_arrows: true,
                                            ..Default::default()
                                        },
                                    )
                                });
                            }
                            source::paint_label(
                                &mut self.rename_ui.labels,
                                ui,
                                &item.message,
                                row.left_center() + egui::vec2(128., 0.),
                                row,
                                color(if selected {
                                    self.tokens.rename_tree_selection
                                } else {
                                    self.tokens.panel
                                }),
                                &self.tokens,
                            );
                            if response.clicked() {
                                self.rename_ui.conflict_selected = index;
                                if !response.has_focus() {
                                    response.request_focus();
                                }
                            }
                            response.on_hover_text(label);
                        }
                    });
                if deferred {
                    self.rename_ui.conflict_selected = self
                        .rename_ui
                        .conflict_selected
                        .saturating_add_signed(direction)
                        .min(count - 1);
                    self.rename_ui.focus_conflict = true;
                    ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                    ctx.request_repaint();
                }
                let selected = &rows[self.rename_ui.conflict_selected];
                let relative = selected
                    .path
                    .strip_prefix(
                        self.service
                            .workspace
                            .as_deref()
                            .unwrap_or(std::path::Path::new("")),
                    )
                    .unwrap_or(&selected.path);
                ui.painter().hline(
                    preview_header.x_range(),
                    preview_header.top(),
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                let mut path_header = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(preview_header.shrink2(egui::vec2(23., 0.)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                path_header.label(
                    selected
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                );
                path_header.label(
                    RichText::new(relative.to_string_lossy()).color(color(self.tokens.muted)),
                );
                let source = self
                    .service
                    .rename
                    .conflict_previews
                    .iter()
                    .find(|f| f.path == selected.path)
                    .map(|f| Arc::clone(&f.before));
                self.conflict_code(ui, code_rect, selected, source.as_deref());
                let mut buttons = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_max(
                            egui::pos2(footer.right() - 250., footer.top() + 11.),
                            footer.right_bottom() - egui::vec2(15., 11.),
                        ))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                );
                self.tokens.form_style(&mut buttons);
                buttons.spacing_mut().item_spacing.x = 11.;
                buttons.spacing_mut().button_padding.x = 8.;
                cancel |= buttons
                    .add(
                        self.tokens
                            .dialog_button("Cancelar", false)
                            .min_size(egui::vec2(72., 28.))
                            .fill(color(self.tokens.rename_button_fill))
                            .stroke(egui::Stroke::new(
                                1.,
                                color(self.tokens.rename_button_border),
                            )),
                    )
                    .clicked();
                change_name = buttons
                    .add(
                        self.tokens
                            .dialog_button("Alterar nome", true)
                            .fill(color(self.tokens.rename_primary))
                            .min_size(egui::vec2(133., 28.)),
                    )
                    .clicked();
                if !tree::candidate_indices(&self.service.rename).is_empty()
                    && buttons.button("Mostrar candidatos").clicked()
                {
                    self.rename_ui.conflict_open = false;
                    self.rename_ui.focus_candidates = true;
                    self.rename_ui.candidate_selected =
                        tree::candidate_indices(&self.service.rename)
                            .first()
                            .copied();
                    self.tool_tab = ToolTab::Rename;
                    self.show_activity = true;
                }
                let mut status = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_max(
                            footer.min + egui::vec2(15., 0.),
                            egui::pos2(footer.left() + 300., footer.bottom()),
                        ))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                status.label(RichText::new("Aplicação bloqueada").color(color(self.tokens.muted)));
                if !self.service.rename.inheritor_choices().is_empty()
                    && status.button("Alterar seleção").clicked()
                {
                    self.edit_rename_inheritors();
                }
            });
        if cancel {
            self.rename_ui.conflict_open = false;
            self.cancel_rename();
        } else if change_name {
            self.rename_ui.conflict_open = false;
            self.rename_ui.dialog = true;
            self.rename_ui.initialized = true;
            self.rename_ui.focus_name = true;
        }
    }
    pub(super) fn rename_candidate_code(&self, ui: &mut egui::Ui, rect: egui::Rect, index: usize) {
        let Some(issue) = self
            .service
            .rename
            .validation
            .as_ref()
            .and_then(|r| r.issues.get(index))
        else {
            return;
        };
        let item = ConflictRow {
            path: issue.path.clone(),
            range: issue
                .range
                .as_ref()
                .map(|r| (r.start_line, r.start_character, r.end_line, r.end_character)),
            container: None,
            message: issue.message.clone(),
        };
        let text = self
            .service
            .rename
            .conflict_previews
            .iter()
            .find(|p| p.path == issue.path)
            .map(|p| p.before.as_ref());
        self.conflict_code(ui, rect, &item, text);
    }
    fn conflict_code(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        item: &ConflictRow,
        source: Option<&str>,
    ) {
        ui.painter()
            .rect_filled(rect, 0., color(self.tokens.editor));
        let Some(text) = source else {
            ui.painter().text(
                rect.left_top() + egui::vec2(16., 12.),
                egui::Align2::LEFT_TOP,
                "Texto do conflito indisponível",
                FontId::proportional(self.tokens.ui_font_size),
                color(self.tokens.muted),
            );
            return;
        };
        let selected_line = item.range.map_or(0, |r| r.0);
        let (lines, truncated) = conflict_lines(text, selected_line);
        let first = lines.first().map_or(0, |l| l.0);
        let mut code = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .id_salt("codisseu.rename.conflict.code")
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        code.set_clip_rect(rect.intersect(ui.clip_rect()));
        egui::ScrollArea::both()
            .id_salt((
                "codisseu.rename.conflict.code.scroll",
                &item.path,
                selected_line,
            ))
            .codisseu()
            .show(&mut code, |ui| {
                let extension = item.path.extension().and_then(|s| s.to_str()).unwrap_or("");
                for (number, line) in &lines {
                    let shaped = source::SourceLine::new(
                        ui,
                        line,
                        extension,
                        &self.tokens,
                        self.service.editor_preferences().ligatures,
                    );
                    let galley = &shaped.galley;
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(
                            ui.available_width()
                                .max(self.tokens.rename_conflict_code_offset + galley.size().x),
                            self.tokens.editor_line_height,
                        ),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                    if *number == selected_line as usize {
                        painter.rect_filled(
                            egui::Rect::from_min_max(row.left_top() + egui::vec2(12., 0.), row.max),
                            0.,
                            color(self.tokens.caret_row),
                        );
                    }
                    if let Some(columns) = conflict_symbol_columns(line, *number, item.range) {
                        let start = shaped.x(columns.start).unwrap_or(0.);
                        let end = shaped.x(columns.end).unwrap_or(start);
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                row.min
                                    + egui::vec2(
                                        self.tokens.rename_conflict_code_offset + start,
                                        0.,
                                    ),
                                egui::vec2((end - start).max(0.), self.tokens.editor_line_height),
                            ),
                            0.,
                            color(self.tokens.rename_conflict_name_background),
                        );
                    }
                    painter.text(
                        row.left_center() + egui::vec2(30., 1.),
                        egui::Align2::RIGHT_CENTER,
                        (number + 1).to_string(),
                        FontId::monospace(self.tokens.editor_font_size),
                        color(self.tokens.muted),
                    );
                    painter.galley(
                        row.left_top() + egui::vec2(self.tokens.rename_conflict_code_offset, 0.),
                        galley.clone(),
                        color(self.tokens.text),
                    );
                }
                if truncated {
                    ui.label("Preview limitado: até 13 linhas / 16 KiB");
                }
            });
        let anchor = egui::pos2(
            rect.left() + 26.,
            rect.top()
                + (selected_line as usize - first + 1) as f32 * self.tokens.editor_line_height
                + 10.,
        );
        let hint_width = 416_f32.min((rect.width() - 52.).max(80.));
        let galley = ui.painter().layout(
            item.message.clone(),
            FontId::proportional(self.tokens.ui_font_size),
            color(self.tokens.rename_conflict_hint_text),
            hint_width - 26.,
        );
        let min_height = if item.path.extension().is_some_and(|e| e == "ts") {
            self.tokens.rename_conflict_ts_hint_min_height
        } else {
            self.tokens.rename_conflict_hint_min_height
        };
        let hint = egui::Rect::from_min_size(
            anchor,
            egui::vec2(hint_width, (galley.size().y + 20.).max(min_height)),
        );
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .rect_filled(hint, 3., color(self.tokens.rename_conflict_hint_background));
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .add(egui::Shape::convex_polygon(
                vec![
                    hint.min + egui::vec2(18., 0.),
                    hint.min + egui::vec2(28., -10.),
                    hint.min + egui::vec2(38., 0.),
                ],
                color(self.tokens.rename_conflict_hint_background),
                egui::Stroke::NONE,
            ));
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .galley(
                hint.min + egui::vec2(13., 10.),
                galley,
                color(self.tokens.rename_conflict_hint_text),
            );
    }
}
fn conflict_lines(text: &str, line: u32) -> (Vec<(usize, &str)>, bool) {
    let mut remaining = 16 * 1024;
    let mut out = Vec::new();
    let mut truncated = false;
    for (number, text) in text
        .split('\n')
        .enumerate()
        .skip(line.saturating_sub(6) as usize)
        .take(13)
    {
        let mut end = text.len().min(remaining);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        truncated |= end != text.len();
        out.push((number, &text[..end]));
        remaining -= end;
        if remaining == 0 {
            break;
        }
    }
    (out, truncated)
}

// Use actual galley scalar columns after strict UTF-16 conversion; never multiply
// a protocol column by a guessed monospace width (emoji/fallback/combining text).
pub(super) fn conflict_symbol_columns(
    line: &str,
    number: usize,
    range: Option<(u32, u32, u32, u32)>,
) -> Option<std::ops::Range<usize>> {
    let (start_line, start, end_line, end) = range?;
    if number != start_line as usize || start_line != end_line || start >= end {
        return None;
    }
    let start = codisseu_core::utf16_byte_offset(line, 0, start)?;
    let end = codisseu_core::utf16_byte_offset(line, 0, end)?;
    Some(line[..start].chars().count()..line[..end].chars().count())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_conflict_name_uses_utf16_and_rejects_truncated_or_split_unicode_ranges() {
        let line = "🌧 e\u{301} rainfall()";
        assert_eq!(
            conflict_symbol_columns(line, 3, Some((3, 6, 3, 14))),
            Some(5..13)
        );
        assert_eq!(conflict_symbol_columns(line, 3, Some((3, 1, 3, 14))), None);
        assert_eq!(conflict_symbol_columns(line, 3, Some((3, 6, 4, 14))), None);
        assert_eq!(
            conflict_symbol_columns("🌧 e\u{301} rain", 3, Some((3, 6, 3, 14))),
            None
        );
        assert_eq!(conflict_symbol_columns(line, 2, Some((3, 6, 3, 14))), None);
    }
}
