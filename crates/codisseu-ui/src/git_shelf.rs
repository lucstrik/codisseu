//! Persistent changelists and Shelf share the measured Commit tool window.
use super::git::{GitDialog, GitIntent};
use super::*;
use codisseu_core::git::{GitCommand, shelf::ShelfAction, workflows::WorkflowAction};
use std::collections::BTreeMap;
impl CodisseuApp {
    pub(super) fn git_request_shelves(&mut self) {
        self.git_send(GitCommand::Shelves);
        self.git_send(GitCommand::ShelfDeleted);
        self.git_send(GitCommand::Changelists);
    }
    pub(super) fn git_new_changelist(&mut self) {
        self.git_ui.dialog = Some(GitDialog::Changelist {
            id: None,
            name: String::new(),
            active: false,
        });
    }
    pub(super) fn git_open_shelve(&mut self, paths: Vec<PathBuf>) {
        self.git_ui.dialog = Some(GitDialog::Shelve {
            name: if self.git_ui.message.trim().is_empty() {
                "Uncommitted changes".into()
            } else {
                self.git_ui
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .into()
            },
            selected: paths.iter().cloned().collect(),
            paths,
        });
    }
    pub(super) fn git_prepare_shelf(&mut self, action: ShelfAction) {
        if !self.git_begin_intent(GitIntent::Other) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        self.git_send(GitCommand::ShelfPrepare(action));
    }
    pub(super) fn git_move_changelist(&mut self, id: String, paths: Vec<PathBuf>) {
        self.git_prepare_shelf(ShelfAction::MoveToChangelist { id, paths });
    }
    pub(super) fn git_changelist_menu(
        &mut self,
        ui: &mut egui::Ui,
        list: &codisseu_core::git::shelf::Changelist,
    ) {
        if ui.button("New Changelist…").clicked() {
            self.git_new_changelist();
            ui.close();
        }
        if ui.button("Rename Changelist…").clicked() {
            self.git_ui.dialog = Some(GitDialog::Changelist {
                id: Some(list.id.clone()),
                name: list.name.clone(),
                active: list.is_default,
            });
            ui.close();
        }
        if !list.is_default && ui.button("Set Active Changelist").clicked() {
            self.git_prepare_shelf(ShelfAction::SetDefaultChangelist {
                id: list.id.clone(),
            });
            ui.close();
        }
        if !list.is_default && ui.button("Delete Changelist…").clicked() {
            self.git_prepare_shelf(ShelfAction::DeleteChangelist {
                id: list.id.clone(),
            });
            ui.close();
        }
    }
    pub(super) fn git_stash_panel(&mut self, ui: &mut egui::Ui) {
        let top = (ui.available_height() * 0.50).max(70.);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("git.stash.list")
            .max_height(top)
            .min_scrolled_height(top)
            .codisseu()
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                ui.add_space(4.);
                for s in self.service.git.stashes.clone() {
                    let (message, branch) = stash_caption(&s.message);
                    let message = format!("{message}  {}", stash_timestamp(s.created_unix));
                    let selected = self.git_ui.stash.as_ref() == Some(&s.reference);
                    let row = self.git_saved_row(
                        ui,
                        &message,
                        branch.as_deref().map(|b| (b, true)),
                        selected,
                        14.,
                        &format!("Stash {} {}", s.reference, s.message),
                    );
                    if row.clicked() {
                        self.git_ui.stash = Some(s.reference.clone());
                        self.git_send(GitCommand::StashDetails {
                            stash: s.reference.clone(),
                        });
                    }
                    row.context_menu(|ui| {
                        if ui.button("Drop…").clicked() {
                            self.git_prepare_workflow(WorkflowAction::StashDrop {
                                stash: s.reference.clone(),
                            });
                            ui.close();
                        }
                    });
                }
                if self.service.git.stashes.is_empty() {
                    ui.add_space(8.);
                    ui.label(RichText::new("No stashed changes").color(color(self.tokens.muted)));
                }
            });
        let details = self
            .service
            .git
            .stash_details
            .as_ref()
            .filter(|d| Some(&d.reference) == self.git_ui.stash.as_ref());
        let patch = details.map(|d| d.patch.clone());
        let changes = details.map(|d| d.changes.clone()).unwrap_or_default();
        let paths = changes.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        let statuses = changes
            .iter()
            .map(|c| (c.path.clone(), c.status))
            .collect::<BTreeMap<_, _>>();
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique("git.saved.statuses"), statuses));
        self.git_stored_files(ui, &paths, patch.as_deref(), "git.stash.files");
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 58.), egui::Sense::hover());
        ui.painter().hline(
            rect.x_range(),
            rect.top(),
            egui::Stroke::new(1., color(self.tokens.border)),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(egui::vec2(14., 14.)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.;
                for (label, pop) in [("Apply", false), ("Pop", true)] {
                    let button = self
                        .tokens
                        .dialog_button(label, !pop)
                        .min_size(egui::vec2(72., 28.));
                    if ui
                        .add_enabled(self.git_ui.stash.is_some(), button)
                        .clicked()
                        && let Some(stash) = self.git_ui.stash.clone()
                    {
                        self.git_prepare_workflow(WorkflowAction::StashApply { stash, pop });
                    }
                }
            },
        );
        self.git_status_line(ui);
    }
    pub(super) fn git_shelf_panel(&mut self, ui: &mut egui::Ui) {
        let entries = self.service.git.shelves.clone();
        let detail = self
            .service
            .git
            .shelf_details
            .as_ref()
            .filter(|d| Some(&d.info.id) == self.git_ui.shelf.as_ref());
        let patch = detail.map(|d| d.patch.clone());
        header(ui, 35., |ui| {
            ui.add_space(7.);
            if components::database_icon_button(
                ui,
                Icon::Import,
                "Unshelve Changes…",
                self.git_ui
                    .shelf
                    .as_ref()
                    .is_some_and(|id| entries.iter().any(|s| &s.id == id)),
            )
            .clicked()
                && let Some(id) = self.git_ui.shelf.clone()
            {
                self.git_ui.dialog = Some(unshelve_dialog(id));
            }
            stored_patch_button(ui, patch.as_deref());
            if components::database_icon_button(
                ui,
                Icon::ExpandAll,
                "Expand All",
                !entries.is_empty(),
            )
            .clicked()
            {
                for s in &entries {
                    self.git_ui.groups.insert(format!("shelf:{}", s.id), true);
                }
            }
            if components::database_icon_button(
                ui,
                Icon::CollapseAll,
                "Collapse All",
                !entries.is_empty(),
            )
            .clicked()
            {
                for s in &entries {
                    self.git_ui.groups.insert(format!("shelf:{}", s.id), false);
                }
            }
        });
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("git.shelf.list")
            .codisseu()
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                ui.add_space(4.);
                for s in entries {
                    let key = format!("shelf:{}", s.id);
                    let expanded = *self.git_ui.groups.get(&key).unwrap_or(&false);
                    let selected = self.git_ui.shelf.as_ref() == Some(&s.id);
                    let count = format!(
                        "{} {}, {}",
                        s.files.len(),
                        if s.files.len() == 1 { "file" } else { "files" },
                        shelf_age(s.created_unix)
                    );
                    let row = self.git_saved_row(
                        ui,
                        s.name.lines().next().unwrap_or_default(),
                        Some((&count, false)),
                        selected,
                        32.,
                        &format!("Shelf {}", s.name),
                    );
                    paint_chevron(
                        ui,
                        row.rect.left() + 14.,
                        row.rect.center().y,
                        expanded,
                        color(self.tokens.muted),
                    );
                    if row.clicked() {
                        self.git_ui.shelf = Some(s.id.clone());
                        if row
                            .interact_pointer_pos()
                            .is_some_and(|p| p.x < row.rect.left() + 28.)
                            || row.double_clicked()
                        {
                            self.git_ui.groups.insert(key, !expanded);
                        }
                        self.git_send(GitCommand::ShelfDetails { id: s.id.clone() });
                    }
                    row.context_menu(|ui| {
                        if ui.button("Unshelve Changes…").clicked() {
                            self.git_ui.shelf = Some(s.id.clone());
                            self.git_ui.dialog = Some(unshelve_dialog(s.id.clone()));
                            ui.close();
                        }
                        if ui.button("Delete…").clicked() {
                            self.git_prepare_shelf(ShelfAction::Delete { id: s.id.clone() });
                            ui.close();
                        }
                    });
                    if expanded {
                        for file in &s.files {
                            self.git_saved_status_row(ui, &file.path, 52., file.status);
                        }
                    }
                }
                let deleted_key = "shelf:recently-deleted".to_string();
                let expanded = *self.git_ui.groups.get(&deleted_key).unwrap_or(&false);
                let row = self.git_saved_row(
                    ui,
                    "Recently Deleted",
                    None,
                    false,
                    32.,
                    "Recently Deleted",
                );
                paint_chevron(
                    ui,
                    row.rect.left() + 14.,
                    row.rect.center().y,
                    expanded,
                    color(self.tokens.muted),
                );
                if row.clicked() {
                    self.git_ui.groups.insert(deleted_key, !expanded);
                }
                if expanded {
                    for shelf in self.service.git.deleted_shelves.clone() {
                        let key = format!("shelf:deleted:{}", shelf.id);
                        let open = *self.git_ui.groups.get(&key).unwrap_or(&false);
                        let selected = self.git_ui.shelf.as_ref() == Some(&shelf.id);
                        let row = self.git_saved_row(
                            ui,
                            shelf.name.lines().next().unwrap_or_default(),
                            Some((&format!("{} files", shelf.files.len()), false)),
                            selected,
                            52.,
                            &format!("Deleted shelf {}", shelf.name),
                        );
                        paint_chevron(
                            ui,
                            row.rect.left() + 34.,
                            row.rect.center().y,
                            open,
                            color(self.tokens.muted),
                        );
                        if row.clicked() {
                            self.git_ui.shelf = Some(shelf.id.clone());
                            self.git_ui.groups.insert(key, !open);
                            self.git_send(GitCommand::ShelfDeletedDetails {
                                id: shelf.id.clone(),
                            });
                        }
                        row.context_menu(|ui| {
                            if ui.button("Restore").clicked() {
                                self.git_prepare_shelf(ShelfAction::RestoreDeleted {
                                    id: shelf.id.clone(),
                                });
                                ui.close();
                            }
                            if ui.button("Delete Permanently…").clicked() {
                                self.git_prepare_shelf(ShelfAction::DeletePermanently {
                                    id: shelf.id.clone(),
                                });
                                ui.close();
                            }
                        });
                        if open {
                            for file in &shelf.files {
                                self.git_saved_status_row(ui, &file.path, 72., file.status);
                            }
                        }
                    }
                }
                if self.service.git.shelves.is_empty() {
                    ui.add_space(8.);
                    ui.label(RichText::new("No shelved changes").color(color(self.tokens.muted)));
                }
            });
        self.git_status_line(ui);
    }
    fn git_saved_row(
        &mut self,
        ui: &mut egui::Ui,
        title: &str,
        suffix: Option<(&str, bool)>,
        selected: bool,
        inset: f32,
        accessible: &str,
    ) -> egui::Response {
        let (rect, row) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::click());
        row.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, accessible)
        });
        if row.clicked() {
            row.request_focus();
        }
        let background = if selected {
            color(if row.has_focus() {
                self.tokens.git_focus_selection
            } else {
                self.tokens.database_services_selection
            })
        } else {
            color(self.tokens.shell_panel_background)
        };
        if selected || row.hovered() {
            ui.painter().rect_filled(
                rect.shrink2(egui::vec2(12., 0.)),
                4.,
                if selected {
                    background
                } else {
                    color(self.tokens.border)
                },
            );
        }
        let start = rect.left() + inset;
        let suffix_width = suffix
            .map(|(s, _)| {
                ui.painter()
                    .layout_no_wrap(
                        s.into(),
                        egui::FontId::proportional(13.),
                        color(self.tokens.muted),
                    )
                    .size()
                    .x
                    + 14.
            })
            .unwrap_or(0.);
        let title_width = ui
            .painter()
            .layout_no_wrap(
                title.into(),
                egui::FontId::proportional(13.),
                color(self.tokens.text),
            )
            .size()
            .x;
        let available = (rect.width() - inset - 20.).max(0.);
        let used = title_width.min((available - suffix_width.min(available * 0.55)).max(0.));
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            egui::Rect::from_min_size(egui::pos2(start, rect.top()), egui::vec2(used + 1., 24.)),
            title,
            color(self.tokens.text),
            background,
        );
        if let Some((suffix, tag)) = suffix {
            let suffix_left = if tag {
                rect.right() - suffix_width - 20.
            } else {
                start + used + 9.
            };
            if tag {
                components::paint_icon(
                    ui,
                    egui::Rect::from_center_size(
                        egui::pos2(suffix_left - 8., rect.center().y),
                        egui::vec2(14., 14.),
                    ),
                    Icon::Tag,
                    color(self.tokens.git_reference),
                );
            }
            super::git_paint::text(
                ui,
                &mut self.git_ui.labels,
                egui::Rect::from_min_max(
                    egui::pos2(suffix_left, rect.top()),
                    egui::pos2(rect.right() - 16., rect.bottom()),
                ),
                suffix,
                if tag {
                    color(self.tokens.text)
                } else {
                    color(self.tokens.muted)
                },
                background,
            );
        }
        row
    }
    fn git_saved_file_row(&mut self, ui: &mut egui::Ui, path: &std::path::Path, inset: f32) {
        let status = ui
            .ctx()
            .data_mut(|d| {
                d.get_temp::<BTreeMap<PathBuf, char>>(egui::Id::unique("git.saved.statuses"))
            })
            .and_then(|s| s.get(path).copied())
            .unwrap_or('M');
        self.git_saved_status_row(ui, path, inset, status);
    }
    fn git_saved_status_row(
        &mut self,
        ui: &mut egui::Ui,
        path: &std::path::Path,
        inset: f32,
        status: char,
    ) {
        let foreground = saved_status_color(&self.tokens, status);
        let key = egui::Id::unique(("git.saved.file.selection", ui.scope_id()));
        let selected = ui.ctx().data_mut(|d| d.get_temp::<PathBuf>(key)).as_deref() == Some(path);
        let (rect, row) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::click());
        row.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                path.display().to_string(),
            )
        });
        if row.clicked() {
            row.request_focus();
        }
        let background = if selected {
            color(if row.has_focus() {
                self.tokens.git_focus_selection
            } else {
                self.tokens.database_services_selection
            })
        } else {
            color(self.tokens.shell_panel_background)
        };
        if selected {
            ui.painter()
                .rect_filled(rect.shrink2(egui::vec2(12., 0.)), 4., background);
        }
        if row.clicked() {
            ui.ctx()
                .data_mut(|d| d.insert_temp(key, path.to_path_buf()));
        }
        let icon = egui::Rect::from_min_size(
            egui::pos2(rect.left() + inset, rect.top() + 4.),
            egui::vec2(16., 16.),
        );
        components::paint_document_icon(
            ui,
            icon,
            path.to_str().unwrap_or_default(),
            color(self.tokens.accent),
        );
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let name_x = icon.right() + 5.;
        let name_width = ui
            .painter()
            .layout_no_wrap(
                name.to_string(),
                egui::FontId::proportional(13.),
                foreground,
            )
            .size()
            .x;
        let text_right = (name_x + name_width + 2.).min(rect.right());
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            egui::Rect::from_min_max(
                egui::pos2(name_x, rect.top()),
                egui::pos2(text_right, rect.bottom()),
            ),
            &name,
            foreground,
            background,
        );
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            super::git_paint::text(
                ui,
                &mut self.git_ui.labels,
                egui::Rect::from_min_max(
                    egui::pos2(text_right + 6., rect.top()),
                    rect.right_bottom(),
                ),
                &parent.display().to_string(),
                color(self.tokens.muted),
                background,
            );
        }
    }
    fn git_saved_path_tree(
        &mut self,
        ui: &mut egui::Ui,
        paths: &[PathBuf],
        inset: f32,
        salt: &str,
    ) {
        let mut parents = BTreeMap::<PathBuf, Vec<PathBuf>>::new();
        for path in paths {
            parents
                .entry(
                    path.parent()
                        .unwrap_or_else(|| std::path::Path::new(""))
                        .to_path_buf(),
                )
                .or_default()
                .push(path.clone());
        }
        for (parent, files) in parents {
            if parent.as_os_str().is_empty() {
                for path in files {
                    self.git_saved_file_row(ui, &path, inset);
                }
                continue;
            }
            let key = format!("{salt}:folder:{}", parent.display());
            let expanded = *self.git_ui.groups.get(&key).unwrap_or(&true);
            let row = self.git_saved_row(
                ui,
                &parent.display().to_string(),
                None,
                false,
                inset + 22.,
                &format!("Folder {}", parent.display()),
            );
            paint_chevron(
                ui,
                row.rect.left() + inset - 12.,
                row.rect.center().y,
                expanded,
                color(self.tokens.muted),
            );
            components::paint_icon(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(row.rect.left() + inset + 7., row.rect.center().y),
                    egui::vec2(16., 16.),
                ),
                Icon::Folder,
                color(self.tokens.muted),
            );
            if row.clicked() {
                self.git_ui.groups.insert(key, !expanded);
            }
            if expanded {
                for path in files {
                    self.git_saved_file_row(ui, &path, inset + 24.);
                }
            }
        }
    }
    fn git_stored_files(
        &mut self,
        ui: &mut egui::Ui,
        paths: &[PathBuf],
        patch: Option<&[u8]>,
        salt: &str,
    ) {
        let key = egui::Id::unique((salt, "expanded"));
        let mut expanded = ui
            .ctx()
            .data_mut(|d| d.get_temp::<bool>(key))
            .unwrap_or(true);
        header(ui, 35., |ui| {
            ui.add_space(7.);
            stored_patch_button(ui, patch);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 28.),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.add_space(7.);
                    if components::database_icon_button(
                        ui,
                        Icon::CollapseAll,
                        "Collapse All",
                        !paths.is_empty(),
                    )
                    .clicked()
                    {
                        expanded = false;
                    }
                    if components::database_icon_button(
                        ui,
                        Icon::ExpandAll,
                        "Expand All",
                        !paths.is_empty(),
                    )
                    .clicked()
                    {
                        expanded = true;
                    }
                },
            );
        });
        ui.ctx().data_mut(|d| d.insert_temp(key, expanded));
        let height = (ui.available_height() - 62.).max(24.);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt(salt)
            .max_height(height)
            .min_scrolled_height(height)
            .codisseu()
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.;
                let label = format!(
                    "{} {}",
                    paths.len(),
                    if paths.len() == 1 { "file" } else { "files" }
                );
                let row = self.git_saved_row(ui, &label, None, false, 54., &label);
                paint_chevron(
                    ui,
                    row.rect.left() + 18.,
                    row.rect.center().y,
                    expanded,
                    color(self.tokens.muted),
                );
                let icon = egui::Rect::from_center_size(
                    egui::pos2(row.rect.left() + 40., row.rect.center().y),
                    egui::vec2(16., 16.),
                );
                components::paint_icon(ui, icon, Icon::Folder, color(self.tokens.muted));
                if row.clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(key, !expanded));
                }
                if expanded {
                    self.git_saved_path_tree(ui, paths, 54., salt);
                }
            });
    }
    pub(super) fn git_shelf_window(&mut self, ctx: &egui::Context, mut dialog: GitDialog) {
        let preparing_before = self.git_ui.preview_requested;
        let mut close = false;
        let shelve = matches!(&dialog, GitDialog::Shelve { .. });
        let unshelve = matches!(&dialog, GitDialog::Unshelve { .. });
        let width = if shelve {
            self.tokens.git_shelve_width
        } else if unshelve {
            1278.
        } else {
            if matches!(&dialog, GitDialog::Stash { .. }) {
                763.
            } else {
                self.tokens.git_changelist_width
            }
        }
        .min(ctx.content_rect().width() - 48.);
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background))
            .inner_margin(12);
        egui::Modal::new(egui::Id::unique("git.shelf.dialog"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width - 24.);
                self.tokens.form_style(ui);
                match &mut dialog {
                    GitDialog::Stash {
                        message,
                        keep_index,
                        untracked,
                    } => {
                        ui.horizontal(|ui| {
                            ui.add_sized([106., 28.], egui::Label::new("Git Root:"));
                            ui.label(
                                self.service
                                    .git
                                    .repo
                                    .as_ref()
                                    .map(|r| r.root.display().to_string())
                                    .unwrap_or_default(),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.add_sized([106., 28.], egui::Label::new("Current Branch:"));
                            ui.label(
                                self.service
                                    .git
                                    .snapshot
                                    .as_ref()
                                    .and_then(|s| s.head_ref.as_deref())
                                    .unwrap_or("")
                                    .trim_start_matches("refs/heads/"),
                            );
                        });
                        let label = ui.label("Message:");
                        ui.add_sized(
                            [ui.available_width(), 62.],
                            egui::TextEdit::multiline(message),
                        )
                        .labelled_by(label.id);
                        ui.horizontal(|ui| {
                            ui.checkbox(keep_index, "Keep index");
                            ui.checkbox(untracked, "Include untracked files");
                        });
                        ui.add_space(8.);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 28.),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                close |=
                                    ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                                if ui
                                    .add_enabled(
                                        !self.service.git.is_busy(),
                                        self.tokens.dialog_button("Create Stash", true),
                                    )
                                    .clicked()
                                {
                                    self.git_prepare_workflow(WorkflowAction::StashCreateOptions {
                                        message: message.clone(),
                                        include_untracked: *untracked,
                                        keep_index: *keep_index,
                                    });
                                    close = true;
                                }
                            },
                        );
                    }
                    GitDialog::Shelve {
                        name,
                        paths,
                        selected,
                    } => {
                        ui.spacing_mut().item_spacing.y = 0.;
                        ui.ctx().accesskit_node_builder(ui.scope_id(), |node| {
                            node.set_label("Shelve Changes dialog")
                        });
                        let body_height = self
                            .tokens
                            .git_shelve_height
                            .min(ctx.content_rect().height() - 64.);
                        let tree_height = 125f32.min((body_height * 0.23).max(72.));
                        let focus_id = egui::Id::unique(("git.shelve.file", paths.clone()));
                        let mut file = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<PathBuf>(focus_id))
                            .or_else(|| paths.first().cloned());
                        let frame_id = focus_id.with("last_frame");
                        let frame = ui.ctx().cumulative_frame_nr();
                        ui.ctx().data_mut(|d| {
                            if d.get_temp::<u64>(frame_id)
                                .is_none_or(|last| last + 1 < frame)
                            {
                                d.remove::<(PathBuf, bool)>(focus_id.with("request"));
                            }
                            d.insert_temp(frame_id, frame);
                        });
                        let side_id = focus_id.with("index");
                        let mut index = ui
                            .ctx()
                            .data_mut(|d| d.get_temp::<bool>(side_id))
                            .unwrap_or(false);
                        let side = if index {
                            codisseu_core::git::DiffSide::Index
                        } else {
                            codisseu_core::git::DiffSide::Working
                        };
                        header(ui, 30., |ui| {
                            if components::database_icon_button(
                                ui,
                                Icon::Refresh,
                                "Refresh Diff",
                                file.is_some(),
                            )
                            .clicked()
                                && let Some(path) = &file
                            {
                                self.git_send(GitCommand::Diff {
                                    path: path.clone(),
                                    side,
                                });
                            }
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 28.),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(format!("{} files", paths.len()))
                                            .color(color(self.tokens.muted)),
                                    );
                                },
                            );
                        });
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("git.shelve.paths")
                                    .auto_shrink([false, false])
                                    .max_height(tree_height)
                                    .min_scrolled_height(tree_height)
                                    .codisseu()
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.;
                                        ui.horizontal(|ui| {
                                            ui.add_space(13.);
                                            let mut all = selected.len() == paths.len();
                                            if ui.checkbox(&mut all, "Changes").changed() {
                                                if all {
                                                    selected.extend(paths.iter().cloned());
                                                } else {
                                                    selected.clear();
                                                }
                                            }
                                        });
                                        for path in paths.iter() {
                                            let mut include = selected.contains(path);
                                            ui.horizontal(|ui| {
                                                ui.add_space(36.);
                                                if ui
                                                    .checkbox(&mut include, "")
                                                    .on_hover_text(format!(
                                                        "Include {}",
                                                        path.display()
                                                    ))
                                                    .changed()
                                                {
                                                    if include {
                                                        selected.insert(path.clone());
                                                    } else {
                                                        selected.remove(path);
                                                    }
                                                }
                                                let response = self.git_saved_row(
                                                    ui,
                                                    &path.display().to_string(),
                                                    None,
                                                    file.as_ref() == Some(path),
                                                    25.,
                                                    &path.display().to_string(),
                                                );
                                                let icon = egui::Rect::from_center_size(
                                                    egui::pos2(
                                                        response.rect.left() + 13.,
                                                        response.rect.center().y,
                                                    ),
                                                    egui::vec2(16., 16.),
                                                );
                                                components::paint_document_icon(
                                                    ui,
                                                    icon,
                                                    path.to_str().unwrap_or_default(),
                                                    color(self.tokens.accent),
                                                );
                                                if response.clicked() {
                                                    file = Some(path.clone());
                                                }
                                            });
                                        }
                                    });
                            });
                        if let Some(path) = &file {
                            let request_id = focus_id.with("request");
                            let previous = ui
                                .ctx()
                                .data_mut(|d| d.get_temp::<(PathBuf, bool)>(request_id));
                            if previous.as_ref() != Some(&(path.clone(), index)) {
                                self.git_send(GitCommand::Diff {
                                    path: path.clone(),
                                    side,
                                });
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(request_id, (path.clone(), index)));
                            }
                            ui.ctx().data_mut(|d| d.insert_temp(focus_id, path.clone()));
                        }
                        ui.add_space(3.);
                        ui.label(
                            RichText::new(format!("{} selected", selected.len()))
                                .color(color(self.tokens.git_modified)),
                        );
                        ui.add_space(8.);
                        let caption = ui.label("Commit Message");
                        ui.add_sized(
                            [ui.available_width(), (body_height * 0.22).clamp(64., 152.)],
                            egui::TextEdit::multiline(name).font(egui::TextStyle::Monospace),
                        )
                        .labelled_by(caption.id);
                        ui.add_space(3.);
                        ui.horizontal(|ui| {
                            ui.label("Diff");
                            if ui.selectable_label(!index, "Working Tree").clicked() {
                                index = false;
                            }
                            if ui.selectable_label(index, "Index").clicked() {
                                index = true;
                            }
                        });
                        ui.ctx().data_mut(|d| d.insert_temp(side_id, index));
                        let patch = self
                            .service
                            .git
                            .diff
                            .as_ref()
                            .filter(|d| Some(&d.path) == file.as_ref() && d.side == side)
                            .map(|d| d.bytes.clone());
                        stored_diff(
                            ui,
                            patch.as_deref(),
                            (body_height - tree_height - body_height * 0.22 - 205.).max(68.),
                            &self.tokens,
                        );
                        ui.add_space(8.);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 28.),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                close |=
                                    ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                                if ui
                                    .add_enabled(
                                        !selected.is_empty() && !name.trim().is_empty(),
                                        self.tokens.dialog_button("Shelve Changes", true),
                                    )
                                    .clicked()
                                {
                                    self.git_prepare_shelf(ShelfAction::Shelve {
                                        name: name.clone(),
                                        paths: selected.iter().cloned().collect(),
                                    });
                                    close = true;
                                }
                            },
                        );
                    }
                    GitDialog::Unshelve {
                        id,
                        requested,
                        initialized,
                        selected,
                        name,
                        comment,
                        active,
                        remove_applied,
                    } => {
                        ui.ctx().accesskit_node_builder(ui.scope_id(), |node| {
                            node.set_label("Unshelve Changes dialog")
                        });
                        ui.spacing_mut().item_spacing.y = 0.;
                        if !*requested {
                            self.git_send(GitCommand::ShelfDetails { id: id.clone() });
                            *requested = true;
                        }
                        let details = self
                            .service
                            .git
                            .shelf_details
                            .as_ref()
                            .filter(|d| &d.info.id == id && d.info.deleted_unix.is_none())
                            .cloned();
                        if let Some(details) = &details
                            && !*initialized
                        {
                            *selected = details.info.files.iter().map(|f| f.path.clone()).collect();
                            *name = details.info.name.lines().next().unwrap_or_default().into();
                            *comment = details.info.name.clone();
                            *initialized = true;
                        }
                        header(ui, 30., |ui| {
                            stored_patch_button(ui, details.as_ref().map(|d| d.patch.as_slice()));
                        });
                        egui::Frame::new()
                            .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
                            .show(ui, |ui| {
                                let height =
                                    281f32.min((ctx.content_rect().height() - 290.).max(80.));
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .id_salt("git.unshelve.paths")
                                    .max_height(height)
                                    .min_scrolled_height(height)
                                    .codisseu()
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.;
                                        if let Some(details) = &details {
                                            let mut all =
                                                selected.len() == details.info.files.len();
                                            if ui
                                                .add_sized(
                                                    [ui.available_width(), 24.],
                                                    egui::Checkbox::new(&mut all, "Changes"),
                                                )
                                                .changed()
                                            {
                                                if all {
                                                    *selected = details
                                                        .info
                                                        .files
                                                        .iter()
                                                        .map(|f| f.path.clone())
                                                        .collect();
                                                } else {
                                                    selected.clear();
                                                }
                                            }
                                            for file in &details.info.files {
                                                ui.horizontal(|ui| {
                                                    ui.add_space(32.);
                                                    let mut checked = selected.contains(&file.path);
                                                    let label = file
                                                        .original_path
                                                        .as_ref()
                                                        .map(|p| {
                                                            format!(
                                                                "{} → {}",
                                                                p.display(),
                                                                file.path.display()
                                                            )
                                                        })
                                                        .unwrap_or_else(|| {
                                                            file.path.display().to_string()
                                                        });
                                                    let response = ui.add_sized(
                                                        [ui.available_width(), 24.],
                                                        egui::Checkbox::new(
                                                            &mut checked,
                                                            RichText::new(label).color(
                                                                saved_status_color(
                                                                    &self.tokens,
                                                                    file.status,
                                                                ),
                                                            ),
                                                        ),
                                                    );
                                                    if response.changed() {
                                                        if checked {
                                                            selected.insert(file.path.clone());
                                                        } else {
                                                            selected.remove(&file.path);
                                                        }
                                                    }
                                                });
                                            }
                                        } else {
                                            ui.label("Loading shelved changes…");
                                        }
                                    });
                            });
                        ui.add_space(12.);
                        ui.horizontal(|ui| {
                            let label = ui.add_sized([62., 28.], egui::Label::new("Name:"));
                            let edit = ui
                                .add_sized(
                                    [ui.available_width() - 28., 28.],
                                    egui::TextEdit::singleline(name),
                                )
                                .labelled_by(label.id);
                            let choices = components::database_icon_button(
                                ui,
                                Icon::Down,
                                "Choose target changelist",
                                true,
                            );
                            egui::Popup::menu(&choices)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show(|ui| {
                                    for list in self.service.git.changelists.clone() {
                                        if ui
                                            .selectable_label(*name == list.name, &list.name)
                                            .clicked()
                                        {
                                            *name = list.name;
                                            *comment = list.comment;
                                            ui.close();
                                        }
                                    }
                                });
                            if edit.changed()
                                && let Some(list) = self
                                    .service
                                    .git
                                    .changelists
                                    .iter()
                                    .find(|l| l.name == *name)
                            {
                                *comment = list.comment.clone();
                            }
                        });
                        ui.add_space(16.);
                        ui.horizontal(|ui| {
                            let label = ui.add_sized([62., 41.], egui::Label::new("Comment:"));
                            ui.add_sized(
                                [ui.available_width(), 41.],
                                egui::TextEdit::multiline(comment),
                            )
                            .labelled_by(label.id);
                        });
                        ui.add_space(12.);
                        ui.checkbox(active, "Set active");
                        ui.add_space(10.);
                        ui.horizontal(|ui| {
                            ui.checkbox(
                                remove_applied,
                                "Remove successfully applied files from the shelf",
                            );
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 34.),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    close |= ui
                                        .add(
                                            self.tokens
                                                .dialog_button("Cancel", false)
                                                .min_size(egui::vec2(78., 28.)),
                                        )
                                        .clicked();
                                    if ui
                                        .add_enabled(
                                            details.is_some()
                                                && !selected.is_empty()
                                                && !name.trim().is_empty()
                                                && !self.service.git.is_busy(),
                                            self.tokens
                                                .dialog_button("Unshelve Changes", true)
                                                .min_size(egui::vec2(153., 28.)),
                                        )
                                        .clicked()
                                    {
                                        self.git_prepare_shelf(ShelfAction::UnshelveOptions {
                                            id: id.clone(),
                                            paths: Some(selected.iter().cloned().collect()),
                                            changelist_name: Some(name.clone()),
                                            comment: Some(comment.clone()),
                                            set_active: *active,
                                            remove_applied: *remove_applied,
                                        });
                                        close = true;
                                    }
                                },
                            );
                        });
                    }
                    GitDialog::Changelist { id, name, active } => {
                        ui.strong(if id.is_some() {
                            "Rename Changelist"
                        } else {
                            "New Changelist"
                        });
                        ui.add_space(12.);
                        let caption = ui.label("Name:");
                        ui.add_sized(
                            [ui.available_width(), 28.],
                            egui::TextEdit::singleline(name),
                        )
                        .labelled_by(caption.id);
                        ui.add_space(50.);
                        if id.is_none() {
                            ui.checkbox(active, "Set active");
                        }
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 28.),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                close |=
                                    ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                                if ui
                                    .add_enabled(
                                        !name.trim().is_empty(),
                                        self.tokens.dialog_button("OK", true),
                                    )
                                    .clicked()
                                {
                                    self.git_prepare_shelf(if let Some(id) = id {
                                        ShelfAction::RenameChangelist {
                                            id: id.clone(),
                                            name: name.clone(),
                                        }
                                    } else {
                                        ShelfAction::CreateChangelist {
                                            name: name.clone(),
                                            set_default: *active,
                                        }
                                    });
                                    close = true;
                                }
                            },
                        );
                    }
                    _ => unreachable!(),
                }
                self.git_status_line(ui);
            });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if close && !preparing_before && self.git_ui.preview_requested {
            self.git_ui.operation_draft = Some(Box::new(dialog));
        } else if !close {
            self.git_ui.dialog = Some(dialog);
        }
    }
}

/// `%gs` includes the branch in both standard stash subject forms. Keep unknown
/// subjects verbatim instead of guessing metadata not supplied by Git.
fn stash_caption(subject: &str) -> (String, Option<String>) {
    let rest = subject
        .strip_prefix("On ")
        .or_else(|| subject.strip_prefix("WIP on "));
    if let Some((branch, message)) = rest.and_then(|s| s.split_once(": ")) {
        (message.into(), Some(branch.into()))
    } else {
        (subject.into(), None)
    }
}
fn shelf_age(created: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|v| v.as_secs())
        .unwrap_or(created);
    let age = now.saturating_sub(created);
    match age {
        0..60 => "Moments ago".into(),
        60..3600 => format!("{} min ago", age / 60),
        3600..86400 => format!("{} h ago", age / 3600),
        _ => format!("{} d ago", age / 86400),
    }
}
fn paint_chevron(ui: &egui::Ui, x: f32, y: f32, expanded: bool, foreground: egui::Color32) {
    let points = if expanded {
        vec![
            egui::pos2(x - 3., y - 2.),
            egui::pos2(x, y + 1.),
            egui::pos2(x + 3., y - 2.),
        ]
    } else {
        vec![
            egui::pos2(x - 2., y - 3.),
            egui::pos2(x + 1., y),
            egui::pos2(x - 2., y + 3.),
        ]
    };
    ui.painter().add(egui::Shape::line(
        points,
        egui::Stroke::new(1.4, foreground),
    ));
}
fn stored_patch_button(ui: &mut egui::Ui, patch: Option<&[u8]>) {
    let diff =
        components::database_icon_button(ui, Icon::Compare, "Show stored patch", patch.is_some());
    egui::Popup::menu(&diff).width(700.).show(|ui| {
        ui.set_max_height(450.);
        egui::ScrollArea::both().codisseu().show(ui, |ui| {
            ui.monospace(String::from_utf8_lossy(patch.unwrap_or_default()));
        });
    });
}
/// Read-only comparison: Shelve always includes complete selected files, never
/// exposes the original IDE's unsupported per-hunk shelving checkboxes.
fn stored_diff(ui: &mut egui::Ui, patch: Option<&[u8]>, height: f32, tokens: &Tokens) {
    let rows = super::git_diff::diff_rows(&String::from_utf8_lossy(patch.unwrap_or_default()));
    let width = ui.available_width();
    egui::Frame::new()
        .stroke(egui::Stroke::new(1., color(tokens.control_border)))
        .show(ui, |ui| {
            ui.set_width((width - 2.).max(0.));
            egui::ScrollArea::both()
                .id_salt("git.shelve.diff")
                .auto_shrink([false, false])
                .max_height(height)
                .min_scrolled_height(height)
                .codisseu()
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.label(
                            RichText::new(if patch.is_some() {
                                "No working tree changes"
                            } else {
                                "Loading diff…"
                            })
                            .color(color(tokens.muted)),
                        );
                    }
                    ui.spacing_mut().item_spacing.y = 0.;
                    for row in rows {
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(width.max(320.) - 2., 21.),
                            egui::Sense::hover(),
                        );
                        let middle = rect.center().x;
                        ui.painter().vline(
                            middle,
                            rect.y_range(),
                            egui::Stroke::new(1., color(tokens.border)),
                        );
                        for (side, left) in [(row.left, true), (row.right, false)] {
                            if let Some((line, text, changed)) = side {
                                let side_rect = egui::Rect::from_min_max(
                                    egui::pos2(
                                        if left { rect.left() } else { middle + 1. },
                                        rect.top(),
                                    ),
                                    egui::pos2(
                                        if left { middle } else { rect.right() },
                                        rect.bottom(),
                                    ),
                                );
                                if changed {
                                    ui.painter().rect_filled(
                                        side_rect,
                                        0.,
                                        if left {
                                            egui::Color32::from_rgb(62, 46, 42)
                                        } else {
                                            egui::Color32::from_rgb(40, 62, 47)
                                        },
                                    );
                                }
                                let painter = ui
                                    .painter()
                                    .with_clip_rect(ui.clip_rect().intersect(side_rect));
                                if line > 0 {
                                    painter.text(
                                        egui::pos2(side_rect.left() + 6., rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        line.to_string(),
                                        egui::FontId::monospace(12.),
                                        color(tokens.muted),
                                    );
                                }
                                painter.text(
                                    egui::pos2(side_rect.left() + 40., rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    text,
                                    egui::FontId::monospace(13.),
                                    color(tokens.text),
                                );
                            }
                        }
                    }
                });
        });
}

pub(super) fn unshelve_dialog(id: String) -> GitDialog {
    GitDialog::Unshelve {
        id,
        requested: false,
        initialized: false,
        selected: Default::default(),
        name: String::new(),
        comment: String::new(),
        active: false,
        remove_applied: false,
    }
}
fn saved_status_color(tokens: &Tokens, status: char) -> egui::Color32 {
    color(match status {
        'A' | '?' => tokens.git_added,
        'D' => tokens.git_deleted,
        'U' => tokens.git_conflict,
        _ => tokens.git_modified,
    })
}
fn stash_timestamp(seconds: i64) -> String {
    #[cfg(unix)]
    {
        let epoch = seconds as nix::libc::time_t;
        let mut value = std::mem::MaybeUninit::<nix::libc::tm>::uninit();
        // localtime_r writes only this exclusive tm and does not return owned memory.
        if !unsafe { nix::libc::localtime_r(&epoch, value.as_mut_ptr()) }.is_null() {
            let tm = unsafe { value.assume_init() };
            return format!(
                "{:02}/{:02}/{:04} {:02}:{:02}",
                tm.tm_mday,
                tm.tm_mon + 1,
                tm.tm_year + 1900,
                tm.tm_hour,
                tm.tm_min
            );
        }
    }
    super::git_log::git_date(seconds)
}
