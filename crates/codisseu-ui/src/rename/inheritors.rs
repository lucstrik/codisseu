//! Editable class/file suggestions; the worker proves the confirmed selection.
use super::*;
use codisseu_core::{RenameInheritorChoice, RenameInheritorKind};

#[derive(Default)]
pub(super) struct Chooser {
    token: Option<u64>,
    proposal_name: String,
    selected: usize,
    drafts: Vec<String>,
    focus_row: bool,
    full_paths: bool,
}

impl CodisseuApp {
    pub(super) fn edit_rename_inheritors(&mut self) {
        if let Some(token) = self.service.rename.context_token() {
            self.rename_ui.conflict_open = false;
            self.rename_ui.blocked_token = None;
            self.rename_ui.apply_intent = None;
            self.send(Command::EditRenameInheritors { token });
        }
    }
    pub(super) fn rename_inheritors_dialog(&mut self, ctx: &egui::Context) {
        let Some(token) = self.service.rename.context_token() else {
            return;
        };
        let choices = self.service.rename.inheritor_choices().to_vec();
        if choices.is_empty() {
            return;
        }
        self.show_activity = false;
        if self.rename_ui.inheritors.token != Some(token)
            || self.rename_ui.inheritors.proposal_name != self.service.rename.new_name
            || self.rename_ui.inheritors.drafts.len() != choices.len()
        {
            self.rename_ui.inheritors = Chooser {
                token: Some(token),
                proposal_name: self.service.rename.new_name.clone(),
                drafts: choices.iter().map(|c| c.new_name.clone()).collect(),
                focus_row: true,
                ..Default::default()
            };
        }
        let row_id = |index| egui::Id::unique(("codisseu.rename.inheritor.row", token, index));
        if ctx
            .memory(|m| m.focused())
            .is_some_and(|id| (0..choices.len()).any(|i| row_id(i) == id))
        {
            let direction = if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                1
            } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                -1
            } else {
                0
            };
            if direction != 0 {
                self.rename_ui.inheritors.selected = self
                    .rename_ui
                    .inheritors
                    .selected
                    .saturating_add_signed(direction)
                    .min(choices.len() - 1);
                self.rename_ui.inheritors.focus_row = true;
                ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
            }
        }
        let can_confirm = self.service.rename.can_confirm_inheritors()
            && choices
                .iter()
                .zip(&self.rename_ui.inheritors.drafts)
                .all(|(c, draft)| !c.selected || &c.new_name == draft);
        let mut changes = Vec::new();
        let mut confirm = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("codisseu.rename.inheritors"))
            .backdrop_color(Color32::TRANSPARENT)
            .frame(egui::Frame::new().fill(color(self.tokens.panel)))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(
                        self.tokens
                            .related_dialog_width
                            .min(ctx.content_rect().width() - 32.),
                        self.tokens
                            .related_dialog_height
                            .min(ctx.content_rect().height() - 32.),
                    ),
                    egui::Sense::hover(),
                );
                let inner = rect.shrink2(egui::vec2(12., 8.));
                source::accessible_label(
                    &mut self.rename_ui.labels,
                    ui,
                    "Rename inheritors and related files with the following names to:",
                    egui::Rect::from_min_size(
                        inner.min + egui::vec2(1., 8.),
                        egui::vec2(inner.width() - 32., 17.),
                    ),
                    &self.tokens,
                );
                let mut settings_ui = child(
                    ui,
                    egui::Rect::from_min_size(
                        egui::pos2(inner.right() - 30., inner.top() + 4.),
                        egui::vec2(24., 24.),
                    ),
                    "inheritors.settings",
                );
                let settings = icon_button(
                    &mut settings_ui,
                    Icon::Settings,
                    "Preview options",
                    false,
                    24.,
                );
                egui::Popup::menu(&settings).show(|ui| {
                    ui.checkbox(&mut self.rename_ui.inheritors.full_paths, "Show file paths");
                });
                let table = egui::Rect::from_min_size(
                    inner.min + egui::vec2(0., 32.),
                    egui::vec2(inner.width(), self.tokens.related_table_height),
                );
                let header = egui::Rect::from_min_size(
                    table.min,
                    egui::vec2(table.width(), self.tokens.related_header_height),
                );
                let mid = table.left() + 28. + (table.width() - 28.) / 2.;
                let stroke = egui::Stroke::new(1., color(self.tokens.border));
                ui.painter()
                    .rect_stroke(table, 0., stroke, egui::StrokeKind::Inside);
                ui.painter().hline(table.x_range(), header.bottom(), stroke);
                for (x, title) in [
                    (table.left() + 37., "Inheritor/File name"),
                    (mid + 8., "Rename to"),
                ] {
                    source::paint_label(
                        &mut self.rename_ui.labels,
                        ui,
                        title,
                        egui::pos2(x, header.center().y),
                        header,
                        color(self.tokens.panel),
                        &self.tokens,
                    );
                }
                let mut body = child(
                    ui,
                    egui::Rect::from_min_max(header.left_bottom(), table.right_bottom()),
                    "inheritors.table",
                );
                body.spacing_mut().item_spacing.y = 0.;
                egui::ScrollArea::vertical()
                    .id_salt(("inheritors.rows", token))
                    .codisseu()
                    .show_rows(
                        &mut body,
                        self.tokens.related_row_height,
                        choices.len(),
                        |ui, visible| {
                            for index in visible {
                                let choice = &choices[index];
                                let (row, _) = ui.allocate_exact_size(
                                    egui::vec2(
                                        ui.available_width(),
                                        self.tokens.related_row_height,
                                    ),
                                    egui::Sense::hover(),
                                );
                                let selected = self.rename_ui.inheritors.selected == index;
                                let background = color(if selected {
                                    self.tokens.rename_tree_selection
                                } else {
                                    self.tokens.panel
                                });
                                ui.painter().rect_filled(row, 0., background);
                                ui.painter().hline(row.x_range(), row.bottom(), stroke);
                                let left = egui::Rect::from_min_max(
                                    row.min + egui::vec2(28., 0.),
                                    egui::pos2(mid, row.bottom()),
                                );
                                let response =
                                    ui.interact(left, row_id(index), egui::Sense::click());
                                let prefix = if choice.kind == RenameInheritorKind::Class {
                                    "Class"
                                } else {
                                    "file"
                                };
                                let name = if self.rename_ui.inheritors.full_paths
                                    && choice.kind == RenameInheritorKind::File
                                {
                                    choice
                                        .path
                                        .strip_prefix(
                                            self.service
                                                .workspace
                                                .as_deref()
                                                .unwrap_or(std::path::Path::new("")),
                                        )
                                        .unwrap_or(&choice.path)
                                        .display()
                                        .to_string()
                                } else {
                                    choice.display_name.clone()
                                };
                                let label = format!("{prefix} {name}");
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::accesskit::Role::ListBoxOption,
                                        true,
                                        selected,
                                        &label,
                                    )
                                });
                                if response.clicked() {
                                    self.rename_ui.inheritors.selected = index;
                                    response.request_focus();
                                }
                                if selected && self.rename_ui.inheritors.focus_row {
                                    response.request_focus();
                                    response.scroll_to_me(None);
                                    self.rename_ui.inheritors.focus_row = false;
                                }
                                let mut checked = choice.selected;
                                let check = name_input::checkbox(
                                    ui,
                                    &mut self.rename_ui.labels,
                                    &self.tokens,
                                    "",
                                    egui::Rect::from_min_size(
                                        row.min + egui::vec2(5., -1.),
                                        egui::vec2(22., 24.),
                                    ),
                                    &mut checked,
                                );
                                check.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::accesskit::Role::CheckBox,
                                        true,
                                        checked,
                                        format!("Incluir {label}"),
                                    )
                                });
                                if response.has_focus()
                                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
                                {
                                    checked = !checked;
                                }
                                source::paint_label(
                                    &mut self.rename_ui.labels,
                                    ui,
                                    &label,
                                    row.left_center() + egui::vec2(32., -1.),
                                    left,
                                    background,
                                    &self.tokens,
                                );
                                let draft = &mut self.rename_ui.inheritors.drafts[index];
                                let target = ui.put(
                                    egui::Rect::from_min_max(
                                        egui::pos2(mid + 4., row.top()),
                                        row.max,
                                    ),
                                    egui::TextEdit::singleline(draft)
                                        .id(egui::Id::unique((
                                            "codisseu.rename.inheritor.target",
                                            token,
                                            index,
                                        )))
                                        .frame(
                                            egui::Frame::NONE
                                                .inner_margin(egui::Margin::symmetric(4, 1)),
                                        )
                                        .font(FontId::proportional(self.tokens.ui_font_size))
                                        .char_limit(256),
                                );
                                target.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::TextInput,
                                        true,
                                        format!("Rename to: {}", choice.display_name),
                                    )
                                });
                                if target.clicked() || target.has_focus() {
                                    self.rename_ui.inheritors.selected = index;
                                }
                                if checked != choice.selected || target.changed() {
                                    let name = if draft.len() <= 256 {
                                        draft.clone()
                                    } else {
                                        choice.new_name.clone()
                                    };
                                    changes.push((index, checked, name));
                                }
                            }
                        },
                    );
                ui.painter()
                    .vline(table.left() + 28., table.y_range(), stroke);
                ui.painter().vline(mid, table.y_range(), stroke);
                for (label, width, right, value) in [
                    ("Unselect all", 100., inner.right() - 3., false),
                    ("Select all", 83., inner.right() - 114., true),
                ] {
                    if name_input::button(
                        ui,
                        &mut self.rename_ui.name_input,
                        &self.tokens,
                        label,
                        egui::Rect::from_min_size(
                            egui::pos2(right - width, table.bottom() + 3.),
                            egui::vec2(width, 28.),
                        ),
                        false,
                        true,
                    )
                    .clicked()
                    {
                        changes.extend(choices.iter().enumerate().map(|(i, c)| {
                            (
                                i,
                                value,
                                if self.rename_ui.inheritors.drafts[i].len() <= 256 {
                                    self.rename_ui.inheritors.drafts[i].clone()
                                } else {
                                    c.new_name.clone()
                                },
                            )
                        }));
                    }
                }
                let choice = &choices[self.rename_ui.inheritors.selected];
                let source_top = table.bottom() + 40.;
                let filename = choice
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                source::paint_label(
                    &mut self.rename_ui.labels,
                    ui,
                    &filename,
                    egui::pos2(inner.left() + 1., source_top + 8.),
                    egui::Rect::from_min_size(
                        egui::pos2(inner.left(), source_top),
                        egui::vec2(inner.width(), 18.),
                    ),
                    color(self.tokens.panel),
                    &self.tokens,
                );
                let code = egui::Rect::from_min_max(
                    egui::pos2(inner.left(), source_top + 18.),
                    egui::pos2(inner.right(), inner.bottom() - 42.),
                );
                self.inheritor_source(ui, choice, code);
                let y = inner.bottom() - 31.;
                confirm = name_input::button(
                    ui,
                    &mut self.rename_ui.name_input,
                    &self.tokens,
                    "OK",
                    egui::Rect::from_min_size(
                        egui::pos2(inner.right() - 159., y),
                        egui::vec2(72., 28.),
                    ),
                    true,
                    can_confirm,
                )
                .clicked();
                cancel = name_input::button(
                    ui,
                    &mut self.rename_ui.name_input,
                    &self.tokens,
                    "Cancel",
                    egui::Rect::from_min_size(
                        egui::pos2(inner.right() - 75., y),
                        egui::vec2(72., 28.),
                    ),
                    false,
                    true,
                )
                .clicked();
                if !can_confirm {
                    crate::status_messages::status_message(
                        ui,
                        "Corrija os nomes selecionados antes de continuar.",
                    );
                }
            });
        for (index, selected, new_name) in changes {
            self.send(Command::UpdateRenameInheritor {
                token,
                index,
                selected,
                new_name,
            });
        }
        if cancel || ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.cancel_rename();
        } else if confirm {
            self.send(Command::ConfirmRenameInheritors { token });
            self.show_activity = true;
        }
    }

    fn inheritor_source(
        &mut self,
        ui: &mut egui::Ui,
        choice: &RenameInheritorChoice,
        rect: egui::Rect,
    ) {
        ui.painter()
            .rect_filled(rect, 0., color(self.tokens.editor));
        let Some(source) = self
            .service
            .rename
            .conflict_previews
            .iter()
            .find(|s| s.path == choice.path)
        else {
            return;
        };
        let mut code = child(ui, rect, "inheritors.source");
        code.set_clip_rect(rect.intersect(ui.clip_rect()));
        code.spacing_mut().item_spacing.y = 0.;
        egui::ScrollArea::both()
            .id_salt(("inheritors.source", self.rename_ui.inheritors.selected))
            .codisseu()
            .show(&mut code, |ui| {
                let line = choice.range.as_ref().map_or(0, |r| r.start_line);
                let (lines, truncated) = preview_lines(&source.before, line);
                for (number, text) in lines {
                    let shaped = source::SourceLine::new(
                        ui,
                        text,
                        choice
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or(""),
                        &self.tokens,
                        self.service.editor_preferences().ligatures,
                    );
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(
                            ui.available_width().max(42. + shaped.galley.size().x),
                            self.tokens.editor_line_height,
                        ),
                        egui::Sense::hover(),
                    );
                    if choice.kind == RenameInheritorKind::File {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_max(row.min + egui::vec2(40., 0.), row.max),
                            0.,
                            color(if text.is_empty() {
                                self.tokens.caret_row
                            } else {
                                self.tokens.rename_file_highlight
                            }),
                        );
                    } else if number == line as usize {
                        ui.painter()
                            .rect_filled(row, 0., color(self.tokens.caret_row));
                    }
                    if let Some(range) = choice.range.as_ref()
                        && let Some(columns) = conflicts::conflict_symbol_columns(
                            text,
                            number,
                            Some((
                                range.start_line,
                                range.start_character,
                                range.end_line,
                                range.end_character,
                            )),
                        )
                    {
                        let a = shaped.x(columns.start).unwrap_or(0.);
                        let b = shaped.x(columns.end).unwrap_or(a);
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                row.min + egui::vec2(42. + a, 0.),
                                egui::vec2(b - a, row.height()),
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
                        shaped.galley,
                        color(self.tokens.text),
                    );
                }
                if truncated {
                    ui.label("Preview limitado: 14 linhas / 8 KiB");
                }
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    fn wait(ui: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            ui.step();
            if predicate(ui.state()) {
                ui.step();
                return;
            }
            assert!(
                std::time::Instant::now() < end,
                "{} {:?}",
                ui.state().service.status,
                ui.state().service.rename.error
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn editable_inheritor_rows_are_independent_invalid_drafts_block_and_off_skips_chooser() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("Weather.php");
        let child = root.join("WetWeather.php");
        let original = "<?php namespace InheritorFixture; class Weather {}\n";
        let child_original =
            "<?php namespace InheritorFixture; class WetWeather extends Weather {}\n";
        std::fs::write(&path, original).unwrap();
        std::fs::write(&child, child_original).unwrap();
        let mut service = AppService::new(root.join("session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        let mut ui = Harness::builder().with_size([1280., 800.]).build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
        wait(&mut ui, |a| !a.service.is_busy());
        ui.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        wait(&mut ui, |a| !a.service.is_busy());
        let id = ui.state().service.active.unwrap();
        let prepare = |ui: &mut Harness<'_, CodisseuApp>| {
            ui.state_mut().caret = Some((id, original.find("Weather").unwrap() + 2));
            ui.state_mut().request_rename();
            wait(ui, |a| a.service.rename.is_prepared());
            ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
            ui.event(egui::Event::Text("Climate".into()));
            ui.step();
        };
        prepare(&mut ui);
        assert!(ui.state().rename_ui.options.rename_inheritors);
        ui.get_by_label("Preview").click();
        wait(&mut ui, |a| a.service.rename.choosing_inheritors());
        assert_eq!(ui.state().service.rename.inheritor_choices().len(), 3);
        ui.get_by_label("Incluir file Weather.php").click();
        ui.step();
        ui.get_by_label("Incluir file WetWeather.php").click();
        ui.step();
        ui.get_by_label("Rename to: InheritorFixture\\WetWeather")
            .click();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.key_press(Key::Backspace);
        ui.step();
        assert!(!ui.state().service.rename.can_confirm_inheritors());
        assert!(
            ui.state()
                .service
                .rename
                .inheritor_choices()
                .iter()
                .any(|c| c.kind == RenameInheritorKind::Class && c.new_name.is_empty())
        );
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.event(egui::Event::Text("Climate".into()));
        ui.step();
        assert!(ui.state().service.rename.can_confirm_inheritors());
        ui.get_by_label("OK").click();
        wait(&mut ui, |a| a.service.rename.semantic_blocked());
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.get_by_label("Alterar seleção").click();
        wait(&mut ui, |a| a.service.rename.choosing_inheritors());
        ui.get_by_label("Rename to: InheritorFixture\\WetWeather")
            .click();
        ui.key_press_modifiers(Modifiers::COMMAND, Key::A);
        ui.event(egui::Event::Text("Vapor".into()));
        ui.step();
        ui.get_by_label("OK").click();
        wait(&mut ui, |a| a.service.rename.can_apply());
        assert!(ui.state().service.rename.resource_moves().is_empty());
        assert!(
            ui.state()
                .service
                .rename
                .files
                .iter()
                .any(|f| f.path == child && f.after.contains("class Vapor extends Climate"))
        );
        assert_eq!(
            ui.state().service.active_document().unwrap().text(),
            original
        );
        ui.get_by_label("Cancelar").click();
        ui.step();
        prepare(&mut ui);
        ui.get_by_label("Rename inheritors").click();
        ui.step();
        assert!(!ui.state().rename_ui.options.rename_inheritors);
        ui.get_by_label("Preview").click();
        wait(&mut ui, |a| a.service.rename.can_apply());
        assert!(!ui.state().service.rename.choosing_inheritors());
        assert!(ui.state().service.rename.resource_moves().is_empty());
        assert!(
            ui.state()
                .service
                .rename
                .files
                .iter()
                .any(|f| f.path == child && f.after.contains("class WetWeather extends Climate"))
        );
        ui.get_by_label("Cancelar").click();
        ui.step();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&child).unwrap(), child_original);
    }
}
