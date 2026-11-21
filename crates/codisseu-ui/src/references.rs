use super::*;

#[derive(Default)]
pub(super) struct ReferencesUi {
    selected: usize,
    token: Option<u64>,
    focus_results: bool,
    scroll_selection: bool,
    symbol: String,
    split_ratio: Option<f32>,
    collapsed: HashSet<PathBuf>,
}

fn row_id(token: Option<u64>, index: usize) -> egui::Id {
    egui::Id::unique(("codisseu.references.row", token, index))
}

impl CodisseuApp {
    pub(super) fn request_references(&mut self) {
        if self.documentation.is_some() {
            self.close_documentation();
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let expected_version = doc.version;
        let character_index = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, index)| index);
        // This is a display label only. References come exclusively from LSP.
        let chars = doc.snapshot();
        let mut start = character_index.min(chars.len_chars());
        let mut end = start;
        let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
        while start > 0 && end - start < 128 && word(chars.char(start - 1)) {
            start -= 1;
        }
        while end < chars.len_chars() && end - start < 128 && word(chars.char(end)) {
            end += 1;
        }
        let symbol = chars.slice(start..end).to_string();
        self.send(Command::References {
            id,
            expected_version,
            character_index,
        });
        self.references_ui = ReferencesUi {
            focus_results: true,
            scroll_selection: true,
            symbol,
            ..Default::default()
        };
        self.tool_tab = ToolTab::References;
        self.show_activity = true;
    }

    pub(super) fn show_navigation_targets(&mut self) {
        let symbol = self
            .service
            .navigation
            .presentation
            .as_ref()
            .map(|p| p.source_symbol.clone())
            .unwrap_or_default();
        self.send(Command::ShowDefinitionTargets);
        self.references_ui = ReferencesUi {
            symbol,
            focus_results: true,
            scroll_selection: true,
            ..Default::default()
        };
        self.tool_tab = ToolTab::References;
        self.show_activity = true;
    }

    fn choose_reference(&mut self, index: usize) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let expected_version = doc.version;
        let character_index = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, index)| index);
        self.send(Command::ChooseReference {
            index,
            id,
            expected_version,
            character_index,
        });
        self.references_ui.selected = index;
        self.references_ui.focus_results = false;
    }

    pub(super) fn references_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.tool_tab != ToolTab::References || !self.show_activity {
            return false;
        }
        let token = self.service.references.context_token();
        let count = self.service.references.items.len();
        let focused = ctx
            .memory(|m| m.focused())
            .is_some_and(|focus| (0..count).any(|index| focus == row_id(token, index)));
        let take = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        // egui releases keyboard focus during begin_pass for Escape. A visible
        // result list can therefore have no focused widget on the Escape frame.
        let released_focus = ctx.memory(|m| m.focused()).is_none()
            && self.service.references.is_open()
            && !self.search_panel.open;
        if (focused || self.service.references.is_pending() || released_focus) && take(Key::Escape)
        {
            if self.service.references.is_pending() {
                self.send(Command::CancelReferences);
            }
            self.references_ui.focus_results = false;
            self.focus_editor = true;
            return true;
        }
        if !focused || count == 0 {
            return false;
        }
        let selected = self.references_ui.selected.min(count - 1);
        let visible: Vec<usize> = self
            .service
            .references
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| !self.references_ui.collapsed.contains(&item.target.path))
            .map(|(index, _)| index)
            .collect();
        if visible.is_empty() {
            return false;
        }
        let position = visible
            .iter()
            .position(|index| *index == selected)
            .unwrap_or(0);
        let next = if take(Key::ArrowDown) {
            Some(visible[(position + 1).min(visible.len() - 1)])
        } else if take(Key::ArrowUp) {
            Some(visible[position.saturating_sub(1)])
        } else if take(Key::Home) {
            visible.first().copied()
        } else if take(Key::End) {
            visible.last().copied()
        } else {
            None
        };
        if let Some(next) = next {
            self.references_ui.selected = next;
            self.references_ui.scroll_selection = true;
            ctx.memory_mut(|m| {
                // begin_pass may have scheduled spatial focus movement before
                // our custom list consumed this key (especially just after click).
                m.move_focus(egui::FocusDirection::None);
                if !m.has_focus(row_id(token, next)) {
                    m.request_focus(row_id(token, next));
                }
            });
            return true;
        }
        if take(Key::Enter) {
            self.choose_reference(selected);
            return true;
        }
        false
    }

    pub(super) fn references_panel(&mut self, ui: &mut egui::Ui) {
        let token = self.service.references.context_token();
        if self.references_ui.token != token {
            self.references_ui.token = token;
            self.references_ui.selected = 0;
            self.references_ui.scroll_selection = true;
        }
        let pending = self.service.references.is_pending();
        header(ui, self.tokens.tool_header_height - 1., |ui| {
            ui.add_space(12.0);
            ui.label("Find");
            ui.add_space(20.0);
            let title = format!("{} em Arquivos do projeto", self.references_ui.symbol);
            let response = ui.label(title);
            let underline = egui::Rect::from_min_max(
                egui::pos2(response.rect.left() - 8., ui.max_rect().bottom() - 2.),
                egui::pos2(response.rect.right() + 8., ui.max_rect().bottom() + 1.),
            );
            ui.painter()
                .rect_filled(underline, 0., color(self.tokens.accent));
            if icon_button(ui, Icon::Close, "Fechar referências", false, 24.).clicked() {
                self.send(Command::CancelReferences);
                self.show_activity = false;
                self.focus_editor = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("...", |ui| {
                    for (tab, label) in [
                        (ToolTab::Results, "Resultados SQL"),
                        (ToolTab::Problems, "Problemas"),
                        (ToolTab::Activity, "Atividade"),
                    ] {
                        if ui.button(label).clicked() {
                            self.tool_tab = tab;
                            ui.close();
                        }
                    }
                });
            });
        });
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                if let Some(error) = &self.service.references.error {
                    crate::status_messages::status_message(ui, error);
                } else if pending {
                    ui.label("Buscando referências…");
                } else if self.service.references.items.is_empty() {
                    ui.label(if self.service.references.is_open() {
                        "Nenhuma referência encontrada nesta posição"
                    } else {
                        "Buscar referências no editor: Alt+Shift+7"
                    });
                }
                if self.service.references.truncated {
                    ui.label("Lista parcial: limite de 256 referências");
                }
            });
        let (body, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
        let ratio = self
            .references_ui
            .split_ratio
            .unwrap_or(self.tokens.references_preview_ratio);
        let split_x = body.left() + body.width() * ratio;
        let splitter = egui::Rect::from_min_max(
            egui::pos2(split_x - self.tokens.references_splitter_width, body.top()),
            egui::pos2(split_x, body.bottom()),
        );
        let response = ui.interact(
            splitter,
            egui::Id::unique("codisseu.references.splitter"),
            egui::Sense::drag(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Splitter,
                true,
                "Redimensionar preview de referências",
            )
        });
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            self.references_ui.split_ratio =
                Some(((pointer.x - body.left()) / body.width()).clamp(0.25, 0.8));
        }
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        ui.painter()
            .rect_filled(splitter, 0., color(self.tokens.border));
        let toolbar = egui::Rect::from_min_max(
            body.min,
            egui::pos2(body.left() + self.tokens.rail_width, body.bottom()),
        );
        let mut controls = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("codisseu.references.toolbar")
                .max_rect(toolbar.shrink2(egui::vec2(4., 4.)))
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        controls.set_clip_rect(toolbar.intersect(ui.clip_rect()));
        if icon_button(
            &mut controls,
            Icon::Refresh,
            "Atualizar referências",
            false,
            24.,
        )
        .clicked()
        {
            self.request_references();
        }
        for (label, offset, icon) in [
            ("Uso anterior", -1isize, Icon::Up),
            ("Próximo uso", 1isize, Icon::Down),
        ] {
            if controls
                .add_enabled_ui(
                    !pending && !self.service.references.items.is_empty(),
                    |ui| icon_button(ui, icon, label, false, 24.),
                )
                .inner
                .clicked()
            {
                let count = self.service.references.items.len();
                let index = self
                    .references_ui
                    .selected
                    .saturating_add_signed(offset)
                    .min(count - 1);
                self.choose_reference(index);
            }
        }
        let left = egui::Rect::from_min_max(
            egui::pos2(toolbar.right(), body.top()),
            egui::pos2(splitter.left(), body.bottom()),
        );
        let right = egui::Rect::from_min_max(egui::pos2(splitter.right(), body.top()), body.max);
        let mut list = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("codisseu.references.tree")
                .max_rect(left)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        list.set_clip_rect(left.intersect(ui.clip_rect()));
        self.references_list(&mut list, token, pending);
        let mut preview = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("codisseu.references.preview")
                .max_rect(right)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        preview.set_clip_rect(right.intersect(ui.clip_rect()));
        self.references_preview(&mut preview);
    }

    fn references_list(&mut self, ui: &mut egui::Ui, token: Option<u64>, pending: bool) {
        let mut chosen = None;
        let mut previous_path = None;
        let mut previous_folder = None;
        egui::ScrollArea::both()
            .id_salt("codisseu.references.scroll")
            .codisseu()
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.add_space(self.tokens.references_tree_padding);
                ui.horizontal(|ui| {
                    ui.add_space(self.tokens.references_indent);
                    ui.label(format!(
                        "Usos em Arquivos do projeto  {}{} resultados",
                        self.service.references.items.len(),
                        if self.service.references.truncated {
                            "+"
                        } else {
                            ""
                        }
                    ));
                });
                if !self.service.references.items.is_empty() {
                    let language = self
                        .service
                        .references
                        .items
                        .first()
                        .and_then(|item| codisseu_core::Language::for_path(&item.target.path));
                    reference_group(
                        ui,
                        self.tokens.references_indent * 2.,
                        match language {
                            Some(codisseu_core::Language::Php) => "PHP",
                            Some(codisseu_core::Language::TypeScript) => "TypeScript",
                            Some(codisseu_core::Language::Rust) => "Rust",
                            Some(codisseu_core::Language::Python) => "Python",
                            None => "Arquivos",
                        },
                        "",
                        None,
                        &self.tokens,
                    );
                    let name = self
                        .service
                        .workspace
                        .as_deref()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    reference_group(
                        ui,
                        self.tokens.references_indent * 3.,
                        &name,
                        "",
                        Some(Icon::Folder),
                        &self.tokens,
                    );
                }
                for (index, item) in self.service.references.items.iter().enumerate() {
                    let relative = item
                        .target
                        .path
                        .strip_prefix(
                            self.service
                                .workspace
                                .as_deref()
                                .unwrap_or(std::path::Path::new("")),
                        )
                        .unwrap_or(&item.target.path);
                    let folder = relative.parent().unwrap_or(std::path::Path::new(""));
                    if previous_folder != Some(folder) {
                        let folder_label = if folder.as_os_str().is_empty() {
                            ".".into()
                        } else {
                            folder.display().to_string()
                        };
                        reference_group(
                            ui,
                            self.tokens.references_indent * 4.,
                            &folder_label,
                            "",
                            Some(Icon::Folder),
                            &self.tokens,
                        );
                        previous_folder = Some(folder);
                    }
                    if previous_path != Some(&item.target.path) {
                        ui.horizontal(|ui| {
                            ui.add_space(self.tokens.references_indent * 5.0);
                            let collapsed =
                                self.references_ui.collapsed.contains(&item.target.path);
                            let response = ui
                                .small_button(if collapsed { ">" } else { "v" })
                                .on_hover_text("Expandir/recolher arquivo");
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    format!(
                                        "{} arquivo {}",
                                        if collapsed { "Expandir" } else { "Recolher" },
                                        relative.display()
                                    ),
                                )
                            });
                            if response.clicked() {
                                if collapsed {
                                    self.references_ui.collapsed.remove(&item.target.path);
                                } else {
                                    self.references_ui
                                        .collapsed
                                        .insert(item.target.path.clone());
                                }
                            }
                            let count = self
                                .service
                                .references
                                .items
                                .iter()
                                .filter(|i| i.target.path == item.target.path)
                                .count();
                            ui.label(relative.display().to_string());
                            ui.label(
                                RichText::new(format!("{count} usos"))
                                    .color(color(self.tokens.muted)),
                            );
                        });
                        previous_path = Some(&item.target.path);
                    }
                    if self.references_ui.collapsed.contains(&item.target.path) {
                        continue;
                    }
                    let label = format!(
                        "{}:{}:{}  {}",
                        relative.display(),
                        item.target.start_line + 1,
                        item.target.start_character + 1,
                        item.preview
                    );
                    let selected = self.references_ui.selected == index;
                    let text = format!("({})  {}", item.target.start_line + 1, item.preview);
                    let font = FontId::proportional(self.tokens.ui_font_size);
                    let galley = ui
                        .painter()
                        .layout_no_wrap(text, font, color(self.tokens.text));
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(
                            ui.available_width().max(
                                self.tokens.references_indent * 7. + 14. + galley.size().x + 12.,
                            ),
                            self.tokens.row_height,
                        ),
                        egui::Sense::hover(),
                    );
                    let response = ui.interact(rect, row_id(token, index), egui::Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::accesskit::Role::ListBoxOption,
                            ui.is_enabled(),
                            selected,
                            &label,
                        )
                    });
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_role(egui::accesskit::Role::ListBoxOption);
                        node.set_selected(selected);
                        node.set_label(&label);
                    });
                    if selected {
                        let fill = if response.has_focus() {
                            self.tokens.selection
                        } else {
                            self.tokens.references_inactive_selection
                        };
                        ui.painter().rect_filled(
                            rect.shrink2(egui::vec2(12., 0.)),
                            self.tokens.control_corner_radius,
                            color(fill),
                        );
                    }
                    if response.has_focus()
                        || response.clicked()
                        || (selected && self.references_ui.focus_results && !pending)
                    {
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
                    ui.painter()
                        .with_clip_rect(rect.intersect(ui.clip_rect()))
                        .galley(
                            rect.left_center()
                                + egui::vec2(
                                    self.tokens.references_indent * 7. + 14.,
                                    -galley.size().y / 2.,
                                ),
                            galley,
                            color(self.tokens.text),
                        );
                    if selected && self.references_ui.scroll_selection {
                        response.scroll_to_me(None);
                        self.references_ui.scroll_selection = false;
                    }
                    if selected && self.references_ui.focus_results && !pending {
                        if !response.has_focus() {
                            response.request_focus();
                        }
                        self.references_ui.focus_results = false;
                    }
                    if response.clicked() {
                        self.references_ui.selected = index;
                        if !response.has_focus() {
                            response.request_focus();
                        }
                    }
                    if response.double_clicked() {
                        chosen = Some(index);
                    }
                    response.on_hover_text(&label);
                }
            });
        if let Some(index) = chosen {
            self.choose_reference(index);
        }
    }

    fn references_preview(&self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0., color(self.tokens.editor));
        let footer = egui::Rect::from_min_max(
            egui::pos2(
                rect.left(),
                (rect.bottom() - (self.tokens.tool_header_height - 2.)).max(rect.top()),
            ),
            rect.max,
        );
        ui.painter()
            .rect_filled(footer, 0., color(self.tokens.panel));
        ui.painter().line_segment(
            [footer.left_top(), footer.left_top() + egui::vec2(72., 0.)],
            egui::Stroke::new(3., color(self.tokens.accent)),
        );
        ui.painter().text(
            footer.left_center() + egui::vec2(14., 0.),
            egui::Align2::LEFT_CENTER,
            "Preview",
            FontId::proportional(self.tokens.ui_font_size),
            color(self.tokens.text),
        );
        let Some(item) = self
            .service
            .references
            .items
            .get(self.references_ui.selected)
        else {
            return;
        };
        let clip = egui::Rect::from_min_max(rect.min, footer.right_top());
        let mut code = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(clip)
                .id_salt("codisseu.references.code")
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        code.set_clip_rect(clip.intersect(ui.clip_rect()));
        code.add_space(1.0);
        egui::ScrollArea::both()
            .id_salt("codisseu.references.preview.scroll")
            .codisseu()
            .show(&mut code, |ui| {
                let extension = item
                    .target
                    .path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                for (line, text) in item.preview_text.split('\n').enumerate() {
                    let number = item.preview_first_line + line as u32;
                    let mut job = highlight::layout_themed(
                        text,
                        extension,
                        FontId::monospace(self.tokens.editor_font_size),
                        self.tokens.theme,
                    );
                    job.wrap.max_width = f32::INFINITY;
                    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(
                            ui.available_width().max(37. + galley.size().x),
                            self.tokens.editor_line_height,
                        ),
                        egui::Sense::hover(),
                    );
                    if number == item.target.start_line {
                        ui.painter()
                            .rect_filled(row, 0., color(self.tokens.caret_row));
                    }
                    let painter = ui.painter().with_clip_rect(ui.clip_rect());
                    painter.text(
                        row.left_center() + egui::vec2(24., 0.),
                        egui::Align2::RIGHT_CENTER,
                        (number + 1).to_string(),
                        FontId::monospace(self.tokens.editor_font_size),
                        color(self.tokens.muted),
                    );
                    painter.galley(
                        row.left_top() + egui::vec2(37., 1.),
                        galley,
                        color(self.tokens.text),
                    );
                }
            });
    }
}

fn reference_group(
    ui: &mut egui::Ui,
    indent: f32,
    label: &str,
    detail: &str,
    icon: Option<Icon>,
    tokens: &Tokens,
) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(240.), tokens.row_height),
        egui::Sense::hover(),
    );
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    if let Some(icon) = icon {
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                rect.left_center() + egui::vec2(indent - 10., 0.),
                egui::vec2(16., 16.),
            ),
            icon,
            color(tokens.muted),
        );
    }
    let text = if detail.is_empty() {
        label.to_owned()
    } else {
        format!("{label}  {detail}")
    };
    painter.text(
        rect.left_center() + egui::vec2(indent, 0.),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(tokens.ui_font_size),
        color(tokens.text),
    );
}
