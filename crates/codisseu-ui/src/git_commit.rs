use super::git::GitActionKind;
use super::*;
use codisseu_core::git::{DiffSide, GitCommand, LocalAction};

impl CodisseuApp {
    pub(super) fn git_commit_panel(&mut self, ui: &mut egui::Ui) {
        if self.git_ui.source_control
            || (!self.service.tool_window_preferences().commit && self.git_ui.commit_tab == 0)
        {
            self.git_ui.source_control = true;
            self.git_source_control_panel(ui);
            return;
        }
        self.tokens.form_style(ui);
        ui.spacing_mut().interact_size.y = 24.;
        ui.spacing_mut().icon_width = 16.;
        ui.spacing_mut().icon_spacing = 8.;
        ui.spacing_mut().item_spacing.y = 0.;
        let has_repository = self.service.git.repo.is_some();
        if !has_repository {
            self.git_ui.commit_tab = 0;
        }
        let tabs = self.git_ui.commit_tab != 0
            || !self.service.git.stashes.is_empty()
            || !self.service.git.shelves.is_empty()
            || !self.service.git.deleted_shelves.is_empty();
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            for (n, label) in [(0, "Commit"), (2, "Shelf"), (1, "Stash")] {
                if (n == 0 && self.service.tool_window_preferences().commit)
                    || (n != 0 && self.git_ui.commit_tab == n)
                    || (n == 1 && !self.service.git.stashes.is_empty())
                    || (n == 2
                        && (!self.service.git.shelves.is_empty()
                            || !self.service.git.deleted_shelves.is_empty()))
                {
                    ui.spacing_mut().button_padding.x = 6.;
                    if ui
                        .add(
                            egui::Button::new(label)
                                .frame(tabs && self.git_ui.commit_tab == n)
                                .fill(if tabs && self.git_ui.commit_tab == n {
                                    color(self.tokens.database_tab_selection)
                                } else {
                                    egui::Color32::TRANSPARENT
                                })
                                .stroke(if tabs && self.git_ui.commit_tab == n {
                                    egui::Stroke::new(1., color(self.tokens.database_tab_border))
                                } else {
                                    egui::Stroke::NONE
                                })
                                .min_size(egui::vec2(0., 28.)),
                        )
                        .clicked()
                    {
                        self.git_ui.commit_tab = n;
                        if n == 1 {
                            self.git_request_stashes();
                        }
                        if n == 2 {
                            self.git_request_shelves();
                        }
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Remove, "Hide Commit", false, 26.).clicked() {
                    self.show_project = false;
                }
                if !has_repository {
                    if components::database_icon_button(
                        ui,
                        Icon::Refresh,
                        "Refresh Repositories",
                        self.service.workspace.is_some() && !self.service.git.is_busy(),
                    )
                    .clicked()
                    {
                        self.git_discover_repositories_with_progress(true);
                    }
                    return;
                }
                let options =
                    components::database_icon_button(ui, Icon::MoreDots, "Commit Options", true);
                egui::Popup::menu(&options).show(|ui| {
                    ui.checkbox(&mut self.git_ui.staging, "Enable staging area");
                    if ui.button("New Changelist…").clicked() {
                        self.git_new_changelist();
                        ui.close();
                    }
                    if ui.button("Git Log").clicked() {
                        self.git_open_log();
                        ui.close();
                    }
                    if ui.button("Stash Changes…").clicked() {
                        self.git_action(GitActionKind::Stash, String::new());
                        ui.close();
                    }
                    if ui.button("Unstash Changes…").clicked() {
                        self.git_action(GitActionKind::Stashes, String::new());
                        ui.close();
                    }
                });
            });
        });
        if self.git_ui.commit_tab == 1 {
            self.git_stash_panel(ui);
            return;
        }
        if self.git_ui.commit_tab == 2 {
            self.git_shelf_panel(ui);
            return;
        }
        if self.service.git.repo.is_none() {
            self.git_empty_repository_panel(ui);
            return;
        }
        header(ui, 35., |ui| {
            ui.add_space(7.);
            if components::database_icon_button(ui, Icon::Refresh, "Refresh Git", true).clicked() {
                self.git_send_foreground(GitCommand::Refresh);
            }
            let selected = self.git_ui.selected.iter().cloned().collect::<Vec<_>>();
            if components::database_icon_button(
                ui,
                Icon::Revert,
                "Rollback selected files",
                !self.git_ui.selected.is_empty(),
            )
            .clicked()
            {
                self.git_prepare(LocalAction::Discard {
                    paths: self.git_ui.selected.iter().cloned().collect(),
                });
            }
            if components::database_icon_button(ui, Icon::Compare, "Show Diff · Ctrl+D", true)
                .clicked()
                && let Some(path) = self
                    .git_ui
                    .file
                    .clone()
                    .or_else(|| selected.first().cloned())
            {
                self.git_open_diff(path, DiffSide::Working);
            }
            if components::database_icon_button(
                ui,
                Icon::Stash,
                "Shelve Changes…",
                !selected.is_empty(),
            )
            .clicked()
            {
                self.git_open_shelve(selected.clone());
            }
            let view = components::database_icon_button(ui, Icon::View, "View Options", true);
            egui::Popup::menu(&view).show(|ui| {
                ui.checkbox(&mut self.git_ui.staging, "Enable staging area");
                if ui.button("Select All").clicked() {
                    if let Some(s) = &self.service.git.snapshot {
                        self.git_ui.selected = s
                            .entries
                            .iter()
                            .filter(|e| !e.conflict)
                            .map(|e| e.path.clone())
                            .collect();
                    }
                    ui.close();
                }
                if ui.button("Deselect All").clicked() {
                    self.git_ui.selected.clear();
                    ui.close();
                }
            });
            if components::database_icon_button(ui, Icon::Locate, "Select Opened File", true)
                .clicked()
                && let Some(document) = self.service.active_document()
                && let Some(repo) = &self.service.git.repo
                && let Ok(path) = document.path.strip_prefix(&repo.root)
            {
                self.git_ui.file = Some(path.to_owned());
            }
            for (icon, label, open) in [
                (Icon::ExpandAll, "Expand All Changes", true),
                (Icon::CollapseAll, "Collapse All Changes", false),
            ] {
                if components::database_icon_button(ui, icon, label, true).clicked() {
                    for key in [
                        "Changes",
                        "Staged",
                        "Unstaged",
                        "Unversioned Files",
                        "Conflicts",
                    ] {
                        self.git_ui.groups.insert(key.into(), open);
                    }
                    for list in &self.service.git.changelists {
                        self.git_ui.groups.insert(list.name.clone(), open);
                    }
                }
            }
            if self.git_ui.staging {
                if components::database_icon_button(ui, Icon::Add, "Stage selected files", true)
                    .clicked()
                {
                    self.git_prepare(LocalAction::Stage {
                        paths: selected.clone(),
                    });
                }
                if components::database_icon_button(
                    ui,
                    Icon::Remove,
                    "Unstage selected files",
                    true,
                )
                .clicked()
                {
                    self.git_prepare(LocalAction::Unstage { paths: selected });
                }
            }
        });
        ui.add_space(4.);
        self.git_operation_banner(ui);
        self.git_status_line(ui);
        let bottom_height = (ui.available_height() * 0.41).clamp(155., 350.);
        let tree_height = (ui.available_height() - bottom_height - 8.).max(60.);
        let entries = self
            .service
            .git
            .snapshot
            .as_ref()
            .map(|s| s.entries.clone())
            .unwrap_or_default();
        let groups: Vec<(String, Vec<_>)> = if self.git_ui.staging {
            vec![
                (
                    "Conflicts".into(),
                    entries.iter().filter(|e| e.conflict).collect(),
                ),
                (
                    "Staged".into(),
                    entries
                        .iter()
                        .filter(|e| !e.conflict && !e.untracked && e.index != '.' && e.index != ' ')
                        .collect(),
                ),
                (
                    "Unstaged".into(),
                    entries
                        .iter()
                        .filter(|e| {
                            !e.conflict && !e.untracked && e.worktree != '.' && e.worktree != ' '
                        })
                        .collect(),
                ),
                (
                    "Unversioned Files".into(),
                    entries.iter().filter(|e| e.untracked).collect(),
                ),
            ]
        } else {
            let lists = self.service.git.changelists.clone();
            let assigned: std::collections::BTreeSet<_> = lists
                .iter()
                .filter(|l| !l.is_default)
                .flat_map(|l| l.paths.iter().cloned())
                .collect();
            let mut result = vec![(
                "Conflicts".into(),
                entries.iter().filter(|e| e.conflict).collect(),
            )];
            if lists.is_empty() {
                result.push((
                    "Changes".into(),
                    entries
                        .iter()
                        .filter(|e| !e.conflict && !e.untracked)
                        .collect(),
                ));
            } else {
                for list in lists {
                    let files = entries
                        .iter()
                        .filter(|e| {
                            !e.conflict
                                && !e.untracked
                                && if list.is_default {
                                    !assigned.contains(&e.path)
                                } else {
                                    list.paths.contains(&e.path)
                                }
                        })
                        .collect();
                    result.push((list.name, files));
                }
            }
            result.push((
                "Unversioned Files".into(),
                entries.iter().filter(|e| e.untracked).collect(),
            ));
            result
        };
        egui::ScrollArea::both()
            .id_salt("git.commit.changes")
            .auto_shrink([false, false])
            .max_height(tree_height)
            .min_scrolled_height(tree_height)
            .codisseu()
            .show(ui, |ui| {
                for (group, mut files) in groups {
                    files.sort_by_key(|e| {
                        e.path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase()
                    });
                    if files.is_empty()
                        && !self
                            .service
                            .git
                            .changelists
                            .iter()
                            .any(|l| l.name == group && !l.is_default)
                    {
                        continue;
                    }
                    let all = files.iter().all(|e| self.git_ui.selected.contains(&e.path));
                    let mut checked = all;
                    let mut open = self
                        .git_ui
                        .groups
                        .get(&group)
                        .copied()
                        .unwrap_or(group != "Unversioned Files");
                    let (row, group_response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 24.),
                        egui::Sense::click(),
                    );
                    if self.git_ui.focus_commit_tree {
                        group_response.request_focus();
                        self.git_ui.focus_commit_tree = false;
                        self.git_ui.commit_group = Some(group.clone());
                        self.git_ui.file = None;
                    }
                    let selected = self.git_ui.file.is_none()
                        && self.git_ui.commit_group.as_deref().unwrap_or("Changes") == group;
                    let bg = if selected {
                        color(if group_response.has_focus() {
                            self.tokens.git_focus_selection
                        } else {
                            self.tokens.database_services_selection
                        })
                    } else {
                        color(self.tokens.editor)
                    };
                    ui.painter()
                        .rect_filled(row.shrink2(egui::vec2(12., 0.)), 4., bg);
                    let arrow = ui.put(
                        egui::Rect::from_min_size(
                            row.min + egui::vec2(12., 0.),
                            egui::vec2(16., 24.),
                        ),
                        egui::Button::new("").frame(false),
                    );
                    ui.ctx().accesskit_node_builder(arrow.id, |n| {
                        n.set_label(format!("Toggle {group}"))
                    });
                    components::paint_icon(
                        ui,
                        arrow.rect.shrink(3.),
                        if open { Icon::Down } else { Icon::Next },
                        color(self.tokens.muted),
                    );
                    if arrow.clicked() {
                        open = !open;
                        self.git_ui.groups.insert(group.clone(), open);
                    }
                    let check = ui
                        .scope_builder(
                            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                                row.min + egui::vec2(35., 0.),
                                egui::vec2(20., 24.),
                            )),
                            |ui| {
                                ui.add_enabled(
                                    !files.iter().any(|e| e.conflict),
                                    egui::Checkbox::without_text(&mut checked),
                                )
                            },
                        )
                        .inner;
                    ui.ctx()
                        .accesskit_node_builder(check.id, |n| n.set_label(&group));
                    if check.changed() {
                        for entry in &files {
                            if checked && !entry.conflict {
                                self.git_ui.selected.insert(entry.path.clone());
                            } else {
                                self.git_ui.selected.remove(&entry.path);
                            }
                        }
                    }
                    let text = egui::Rect::from_min_max(
                        row.min + egui::vec2(58., 0.),
                        row.right_bottom() - egui::vec2(12., 0.),
                    );
                    let name_width = ui
                        .painter()
                        .layout_no_wrap(
                            group.clone(),
                            FontId::proportional(13.),
                            color(self.tokens.text),
                        )
                        .size()
                        .x;
                    super::git_paint::text(
                        ui,
                        &mut self.git_ui.labels,
                        egui::Rect::from_min_size(text.min, egui::vec2(name_width + 4., 24.)),
                        &group,
                        color(self.tokens.text),
                        bg,
                    );
                    let count = format!(
                        "{} {}",
                        files.len(),
                        if files.len() == 1 { "file" } else { "files" }
                    );
                    super::git_paint::text(
                        ui,
                        &mut self.git_ui.labels,
                        egui::Rect::from_min_max(
                            text.min + egui::vec2(name_width + 8., 0.),
                            text.max,
                        ),
                        &count,
                        color(self.tokens.muted),
                        bg,
                    );
                    if group_response.clicked() {
                        group_response.request_focus();
                        self.git_ui.file = None;
                        self.git_ui.commit_group = Some(group.clone());
                    }
                    if let Some(list) = self
                        .service
                        .git
                        .changelists
                        .iter()
                        .find(|l| l.name == group)
                        .cloned()
                    {
                        group_response.context_menu(|ui| self.git_changelist_menu(ui, &list));
                    }
                    if !open {
                        continue;
                    }
                    for entry in files {
                        ui.scope(|ui| {
                            let (row, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 24.),
                                egui::Sense::click(),
                            );
                            let selected = self.git_ui.file.as_ref() == Some(&entry.path);
                            let bg = if selected {
                                color(if response.has_focus() {
                                    self.tokens.git_focus_selection
                                } else {
                                    self.tokens.database_services_selection
                                })
                            } else {
                                color(self.tokens.editor)
                            };
                            if selected {
                                ui.painter()
                                    .rect_filled(row.shrink2(egui::vec2(12., 0.)), 4., bg);
                            }
                            let mut checked = self.git_ui.selected.contains(&entry.path);
                            let checkbox = ui
                                .scope_builder(
                                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                                        row.min + egui::vec2(54., 0.),
                                        egui::vec2(20., 24.),
                                    )),
                                    |ui| {
                                        ui.add_enabled(
                                            !entry.conflict,
                                            egui::Checkbox::without_text(&mut checked),
                                        )
                                    },
                                )
                                .inner;
                            ui.ctx().accesskit_node_builder(checkbox.id, |node| {
                                node.set_label(format!(
                                    "Include {} in commit",
                                    entry.path.display()
                                ))
                            });
                            if checkbox.changed() {
                                if checked {
                                    self.git_ui.selected.insert(entry.path.clone());
                                } else {
                                    self.git_ui.selected.remove(&entry.path);
                                }
                            }
                            components::paint_document_icon(
                                ui,
                                egui::Rect::from_min_size(
                                    row.min + egui::vec2(76., 4.),
                                    egui::vec2(16., 16.),
                                ),
                                entry.path.to_str().unwrap_or(""),
                                color(self.tokens.accent),
                            );
                            let tint = git_file_color(entry, &self.tokens);
                            let label = entry
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let label_width = ui
                                .painter()
                                .layout_no_wrap(label.clone(), FontId::proportional(13.), tint)
                                .size()
                                .x;
                            super::git_paint::text(
                                ui,
                                &mut self.git_ui.labels,
                                egui::Rect::from_min_max(
                                    row.min + egui::vec2(94., 0.),
                                    egui::pos2(
                                        (row.left() + 94. + label_width + 3.)
                                            .min(row.right() - 12.),
                                        row.bottom(),
                                    ),
                                ),
                                &label,
                                tint,
                                bg,
                            );
                            let mut detail = entry
                                .original_path
                                .as_ref()
                                .map(|p| {
                                    format!(
                                        " - renamed from {}",
                                        p.file_name().unwrap_or_default().to_string_lossy()
                                    )
                                })
                                .unwrap_or_default();
                            if let Some(parent) =
                                entry.path.parent().filter(|p| !p.as_os_str().is_empty())
                            {
                                detail.push(' ');
                                detail.push_str(&parent.to_string_lossy());
                            }
                            super::git_paint::text(
                                ui,
                                &mut self.git_ui.labels,
                                egui::Rect::from_min_max(
                                    row.min + egui::vec2(94. + label_width + 5., 0.),
                                    row.right_bottom() - egui::vec2(12., 0.),
                                ),
                                &detail,
                                color(self.tokens.muted),
                                bg,
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    format!("Git file {}", entry.path.display()),
                                )
                            });
                            if response.clicked() {
                                response.request_focus();
                                self.git_ui.file = Some(entry.path.clone());
                            }
                            if response.double_clicked() {
                                if entry.conflict {
                                    self.git_open_conflict(entry.path.clone());
                                } else {
                                    self.git_open_diff(
                                        entry.path.clone(),
                                        if self.git_ui.staging && group == "Staged" {
                                            DiffSide::Index
                                        } else {
                                            DiffSide::Working
                                        },
                                    );
                                }
                            }
                            response.context_menu(|ui| {
                                if ui.button("Show Diff").clicked() {
                                    self.git_open_diff(
                                        entry.path.clone(),
                                        if self.git_ui.staging && group == "Staged" {
                                            DiffSide::Index
                                        } else {
                                            DiffSide::Working
                                        },
                                    );
                                    ui.close();
                                }
                                if ui.button("Open File").clicked() {
                                    if let Some(r) = &self.service.git.repo {
                                        self.send(Command::OpenFile(r.root.join(&entry.path)));
                                    }
                                    ui.close();
                                }
                                if entry.conflict && ui.button("Resolve Conflicts…").clicked() {
                                    self.git_open_conflict(entry.path.clone());
                                    ui.close();
                                }
                                if !entry.conflict && ui.button("Stage").clicked() {
                                    self.git_prepare(LocalAction::Stage {
                                        paths: vec![entry.path.clone()],
                                    });
                                    ui.close();
                                }
                                if !entry.conflict && ui.button("Unstage").clicked() {
                                    self.git_prepare(LocalAction::Unstage {
                                        paths: vec![entry.path.clone()],
                                    });
                                    ui.close();
                                }
                                ui.menu_button("Move to Another Changelist", |ui| {
                                    for list in self.service.git.changelists.clone() {
                                        if ui.button(&list.name).clicked() {
                                            self.git_move_changelist(
                                                list.id,
                                                vec![entry.path.clone()],
                                            );
                                            ui.close();
                                        }
                                    }
                                    if ui.button("New Changelist…").clicked() {
                                        self.git_new_changelist();
                                        ui.close();
                                    }
                                });
                                if ui.button("Shelve Changes…").clicked() {
                                    self.git_open_shelve(vec![entry.path.clone()]);
                                    ui.close();
                                }
                                if ui.button("File History").clicked() {
                                    self.git_open_file_history(entry.path.clone());
                                    ui.close();
                                }
                                if ui.button("Annotate with Git Blame").clicked() {
                                    if let Some(root) =
                                        self.service.git.repo.as_ref().map(|r| r.root.clone())
                                    {
                                        self.send(Command::OpenFile(root.join(&entry.path)));
                                        let p = self.service.editor_preferences();
                                        self.send(Command::SetGitInsights {
                                            inline: p.git_current_line,
                                            hover: p.git_hover,
                                            gutter: true,
                                        });
                                    }
                                    ui.close();
                                }
                            });
                            if entry.conflict && ui.small_button("Merge…").clicked() {
                                self.git_open_conflict(entry.path.clone());
                            }
                        });
                    }
                }
                if entries.is_empty() {
                    ui.add_space(12.);
                    ui.horizontal(|ui| {
                        ui.add_space(f32::from(self.tokens.panel_content_inset_x));
                        ui.label(RichText::new("No changes").color(color(self.tokens.muted)));
                    });
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.add_space(12.);
            if ui.checkbox(&mut self.git_ui.amend, "Amend").changed() {
                if self.git_ui.amend {
                    self.git_ui.amend_backup = Some(self.git_ui.message.clone());
                    self.git_ui.amend_loaded = None;
                    if let Some(oid) = self
                        .service
                        .git
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.head.clone())
                    {
                        self.git_ui.amend_load_expected = Some(self.git_ui.message.clone());
                        self.git_send(GitCommand::CommitDetails { commit: oid });
                    }
                } else {
                    self.git_ui.amend_load_expected = None;
                    if self.git_ui.amend_loaded.as_ref() == Some(&self.git_ui.message)
                        && let Some(backup) = self.git_ui.amend_backup.take()
                    {
                        self.git_ui.message = backup;
                    }
                    self.git_ui.amend_backup = None;
                    self.git_ui.amend_loaded = None;
                }
            }
            let last = self
                .service
                .git
                .snapshot
                .as_ref()
                .and_then(|s| s.head.as_ref());
            if ui.link("last commit").clicked()
                && let Some(oid) = last.cloned()
            {
                self.git_open_log();
                self.git_send(GitCommand::CommitDetails { commit: oid });
            }
        });
        ui.add_space(4.);
        let width = (ui.available_width() - 24.).max(40.);
        ui.horizontal(|ui| {
            ui.add_space(12.);
            let message = ui.add_sized(
                [width, (bottom_height - 80.).max(55.)],
                egui::TextEdit::multiline(&mut self.git_ui.message)
                    .margin(egui::vec2(8., 6.))
                    .id(egui::Id::unique("git.commit.message"))
                    .hint_text("Commit Message")
                    .char_limit(65536)
                    .font(egui::TextStyle::Monospace),
            );
            ui.ctx()
                .accesskit_node_builder(message.id, |node| node.set_label("Commit Message"));
        });
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.add_space(12.);
            let ready = !self.service.git.is_busy()
                && !self.git_ui.message.trim().is_empty()
                && (!self.git_ui.selected.is_empty() || self.git_ui.staging || self.git_ui.amend);
            if ui
                .add_enabled(
                    ready,
                    self.tokens.dialog_button(
                        if self.git_ui.amend {
                            "Amend Commit"
                        } else {
                            "Commit"
                        },
                        true,
                    ),
                )
                .clicked()
            {
                self.git_commit(false);
            }
            if ui
                .add_enabled(
                    ready,
                    self.tokens.dialog_button(
                        if self.git_ui.amend {
                            "Amend Commit and Push…"
                        } else {
                            "Commit and Push…"
                        },
                        false,
                    ),
                )
                .clicked()
            {
                self.git_commit(true);
            }
        });
    }
}
pub(super) fn git_file_color(entry: &codisseu_core::git::StatusEntry, t: &Tokens) -> Color32 {
    if entry.conflict {
        color(t.git_conflict)
    } else if entry.untracked {
        color(t.git_untracked)
    } else if entry.index == 'D' || entry.worktree == 'D' {
        color(t.git_deleted)
    } else if entry.index == 'A' {
        color(t.git_added)
    } else {
        color(t.git_modified)
    }
}
