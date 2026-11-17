//! Explorer interactions. Filesystem writes run through the core worker.
use super::*;
use codisseu_core::ProjectFileCommand;
#[path = "project_file_actions.rs"]
mod actions;
#[path = "project_drag.rs"]
mod drag;
#[path = "project_selection.rs"]
mod selection;
use actions::{ExplorerAction, ExplorerExtras};

#[derive(Clone, Copy, PartialEq)]
enum NameKind {
    File,
    Folder,
    Rename,
}
#[derive(Clone)]
pub(super) struct NameInput {
    parent: PathBuf,
    source: Option<PathBuf>,
    kind: NameKind,
    value: String,
    focus: bool,
}
#[derive(Default)]
pub(super) struct ProjectFilesUi {
    selected: Option<PathBuf>,
    selection: std::collections::BTreeSet<PathBuf>,
    anchor: Option<PathBuf>,
    range_base: std::collections::BTreeSet<PathBuf>,
    selection_revision: (u64, u64),
    selection_modified: bool,
    focus: Option<egui::Id>,
    focus_path: Option<PathBuf>,
    pub(super) input: Option<NameInput>,
    submitted: Option<NameInput>,
    pub(super) trash: Option<Vec<PathBuf>>,
    trash_submitted: bool,
    trash_focus: bool,
    root_collapsed: bool,
    workspace: Option<PathBuf>,
    revision: u64,
    error: Option<String>,
    extras: ExplorerExtras,
    tree_width: f32,
    active_file: Option<PathBuf>,
    following: bool,
    reveal: Option<PathBuf>,
    scroll_to: Option<PathBuf>,
    drag_hover: Option<(PathBuf, f64)>,
}
#[derive(Clone)]
enum Action {
    New(PathBuf, bool),
    Rename(PathBuf),
    Trash(PathBuf),
    Git(&'static str, PathBuf),
    Extra(ExplorerAction, PathBuf),
    Remote(String, bool),
    Encoding(PathBuf, codisseu_core::FileEncoding),
}
impl CodisseuApp {
    pub(super) fn project_files_tick(&mut self) {
        if self.project_files_ui.workspace.as_deref() != self.service.workspace_path() {
            self.project_files_ui = ProjectFilesUi {
                workspace: self.service.workspace_path().map(PathBuf::from),
                ..Default::default()
            };
        }
        self.project_reveal_tick();
        // Expansion is presentation state; only expanded directories are read.
        // Queue a few requests per frame and let the shared worker publish them.
        let missing: Vec<_> = self
            .expanded
            .iter()
            .filter(|path| {
                !self.service.tree_directory_loaded(path)
                    && self.service.tree_directory_error(path).is_none()
            })
            .filter(|path| {
                self.service
                    .entries
                    .iter()
                    .any(|entry| entry.is_dir && entry.path == **path)
            })
            .take(8)
            .cloned()
            .collect();
        for path in missing {
            let _ = self.service.dispatch(Command::LoadDirectory(path));
        }
        self.project_extra_tick();
        if !self.service.project_files.pending && self.service.project_files.error.is_some() {
            self.project_files_ui.extras.moved_clipboard = false;
        }
        if self.project_files_ui.revision != self.service.project_files.revision {
            self.project_files_ui.revision = self.service.project_files.revision;
            self.reconcile_project_selection();
            if self.project_files_ui.extras.moved_clipboard {
                self.project_files_ui.extras.clipboard = None;
                self.project_files_ui.extras.moved_clipboard = false;
            }
            self.project_files_ui.submitted = None;
            self.project_files_ui.error = None;
            if let Some(path) = self.service.project_files.created.take() {
                for parent in path.ancestors().skip(1) {
                    self.expanded.insert(parent.to_owned());
                }
                self.select_project_single(path.clone());
                self.project_files_ui.root_collapsed = false;
                if self
                    .service
                    .active_document()
                    .is_some_and(|document| document.path == path)
                {
                    self.focus_editor = true;
                }
            } else if self.project_files_ui.selected.as_ref().is_some_and(|path| {
                self.service.workspace.as_ref() != Some(path)
                    && !self.service.entries.iter().any(|entry| entry.path == *path)
            }) {
                let remaining = self.project_files_ui.selection.first().cloned();
                if let Some(path) = remaining {
                    self.project_files_ui.selected = Some(path);
                } else {
                    self.project_files_ui.selected = self
                        .service
                        .entries
                        .iter()
                        .find(|entry| {
                            self.project_files_ui
                                .selected
                                .as_ref()
                                .is_none_or(|old| &entry.path > old)
                        })
                        .or_else(|| self.service.entries.last())
                        .map(|entry| entry.path.clone())
                        .or_else(|| self.service.workspace.clone());
                }
                self.project_files_ui.focus = None;
            }
        }
        self.reconcile_project_selection();
        if !self.service.project_files.pending {
            if self.project_files_ui.trash_submitted {
                self.project_files_ui.focus_path = self.project_files_ui.selected.clone();
                self.project_files_ui.scroll_to = self.project_files_ui.selected.clone();
            }
            self.project_files_ui.trash_submitted = false;
        }
        if !self.service.project_files.pending
            && self.service.project_files.error.is_some()
            && let Some(input) = self.project_files_ui.submitted.take()
        {
            self.project_files_ui.input = Some(NameInput {
                focus: true,
                ..input
            });
            self.project_files_ui.error = self.service.project_files.error.clone();
            self.focus_editor = false;
        }
    }
    pub(super) fn reveal_breadcrumb_path(&mut self, path: PathBuf) {
        let Some(root) = self.service.workspace_root_for(&path).map(PathBuf::from) else {
            return;
        };
        for parent in path
            .ancestors()
            .skip(1)
            .take_while(|p| p.starts_with(&root))
        {
            self.expanded.insert(parent.to_owned());
        }
        self.project_files_ui.root_collapsed = false;
        self.show_project = true;
        if path == root {
            self.select_project_single(path);
        } else {
            self.project_files_ui.reveal = Some(path);
        }
    }
    fn reveal_active_project_file(&mut self) {
        let Some(path) = self.service.active_document().map(|d| d.path.clone()) else {
            return;
        };
        let Some(root) = self.service.workspace_root_for(&path) else {
            return;
        };
        if !path.starts_with(root)
            || path == root
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return;
        }
        for parent in path.ancestors().skip(1).take_while(|p| p.starts_with(root)) {
            self.expanded.insert(parent.to_owned());
        }
        self.project_files_ui.root_collapsed = false;
        self.project_files_ui.reveal = Some(path);
    }
    fn project_reveal_tick(&mut self) {
        let active = self.service.active_document().map(|d| d.path.clone());
        let following = self.service.project_preferences().follow_active_file;
        let changed = active != self.project_files_ui.active_file;
        if changed || following != self.project_files_ui.following {
            self.project_files_ui.reveal = None;
            self.project_files_ui.scroll_to = None;
            let owns_selection = self.project_files_ui.trash_submitted
                || self.project_files_ui.trash.is_some()
                || ((self.project_files_ui.selection.len() > 1
                    || self.project_files_ui.selection_modified)
                    && self
                        .project_files_ui
                        .extras
                        .context
                        .as_ref()
                        .is_some_and(|ctx| self.project_tree_has_focus(ctx)));
            if following && self.project_files_ui.input.is_none() && !owns_selection {
                self.reveal_active_project_file();
            }
        }
        self.project_files_ui.active_file = active;
        self.project_files_ui.following = following;
        let Some(path) = self.project_files_ui.reveal.clone() else {
            return;
        };
        if self.project_files_ui.input.is_some() {
            self.project_files_ui.reveal = None;
            return;
        }
        // Follow only the requested branch; the normal expansion worker loads
        // its next known directory. Stop on missing/error rather than retrying.
        let Some(root) = self.service.workspace_root_for(&path) else {
            self.project_files_ui.reveal = None;
            return;
        };
        let mut ancestors: Vec<_> = path
            .ancestors()
            .skip(1)
            .take_while(|p| *p != root)
            .collect();
        ancestors.reverse();
        for parent in std::iter::once(root).chain(ancestors) {
            if self.service.tree_directory_error(parent).is_some()
                || (parent != root
                    && !self
                        .service
                        .entries
                        .iter()
                        .any(|e| e.is_dir && e.path == parent))
            {
                self.project_files_ui.reveal = None;
                return;
            }
            if !self.service.tree_directory_loaded(parent) {
                return;
            }
        }
        self.project_files_ui.reveal = None;
        if self.service.entries.iter().any(|e| e.path == path) {
            self.select_project_single(path.clone());
            self.project_files_ui.scroll_to = Some(path);
        }
    }
    fn start_project_name(&mut self, parent: PathBuf, kind: NameKind, source: Option<PathBuf>) {
        self.focus_editor = false;
        for ancestor in parent.ancestors() {
            self.expanded.insert(ancestor.to_owned());
        }
        self.project_files_ui.root_collapsed = false;
        let value = source
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.project_files_ui.input = Some(NameInput {
            parent,
            source,
            kind,
            value,
            focus: true,
        });
        self.project_files_ui.error = None;
        self.service.project_files.error = None;
    }
    fn project_tree_action(&mut self, action: Action) {
        match action {
            Action::Encoding(root, encoding) => {
                self.send(Command::SetFolderEncoding { root, encoding })
            }
            Action::Extra(action, path) => self.project_extra_action(action, path),
            Action::Remote(url, copy) => {
                if let Some(ctx) = &self.project_files_ui.extras.context {
                    if copy {
                        ctx.copy_text(url);
                    } else {
                        ctx.open_url(egui::OpenUrl::new_tab(url));
                    }
                }
            }
            Action::New(parent, folder) => self.start_project_name(
                parent,
                if folder {
                    NameKind::Folder
                } else {
                    NameKind::File
                },
                None,
            ),
            Action::Rename(path) => {
                if let Some(parent) = path.parent() {
                    self.start_project_name(parent.to_owned(), NameKind::Rename, Some(path));
                }
            }
            Action::Trash(path) => {
                self.focus_editor = false;
                let paths = if self.project_files_ui.selection.contains(&path) {
                    self.project_files_ui.selection.iter().cloned().collect()
                } else {
                    vec![path]
                };
                self.begin_project_trash(paths);
            }
            Action::Git(action, path) => self.git_explorer_target(action, path),
        }
    }
    pub(super) fn project_panel(&mut self, ui: &mut egui::Ui) {
        self.project_files_ui.extras.context = Some(ui.ctx().clone());
        let root = self.service.workspace.clone();
        let mut action = None;
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            ui.label("Project");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.;
                ui.add_space(6.);
                let options = icon_button(ui, Icon::MoreDots, "Project Options", false, 26.);
                egui::Popup::menu(&options).show(|ui| {
                    ui.menu_button("Behavior", |ui| {
                        let mut following = self.service.project_preferences().follow_active_file;
                        if ui
                            .checkbox(&mut following, "Always Select Opened File")
                            .changed()
                        {
                            self.send(Command::SetProjectFollowActiveFile(following));
                            ui.close();
                        }
                    });
                });
                ui.add_enabled_ui(
                    root.is_some() && !self.service.project_files.pending,
                    |ui| {
                        if icon_button(ui, Icon::CollapseAll, "Collapse All", false, 26.).clicked()
                        {
                            self.expanded.clear();
                            self.project_files_ui.reveal = None;
                            self.project_files_ui.scroll_to = None;
                        }
                        if icon_button(ui, Icon::Refresh, "Refresh Explorer", false, 26.).clicked()
                        {
                            self.send(Command::ProjectFile(ProjectFileCommand::Refresh));
                        }
                        let parent = self
                            .project_files_ui
                            .selected
                            .as_ref()
                            .and_then(|path| {
                                if root.as_ref() == Some(path)
                                    || self
                                        .service
                                        .entries
                                        .iter()
                                        .any(|entry| entry.path == *path && entry.is_dir)
                                {
                                    Some(path.clone())
                                } else {
                                    path.parent().map(PathBuf::from)
                                }
                            })
                            .filter(|path| self.service.workspace_root_for(path).is_some())
                            .or_else(|| {
                                self.service.active_document().and_then(|d| {
                                    self.service.workspace_root_for(&d.path).map(PathBuf::from)
                                })
                            })
                            .or_else(|| self.service.workspace_roots().first().cloned());
                        if let Some(parent) = parent {
                            if icon_button(ui, Icon::NewFolder, "New Folder…", false, 26.).clicked()
                            {
                                action = Some(Action::New(parent.clone(), true));
                            }
                            if icon_button(ui, Icon::NewFile, "New File…", false, 26.).clicked() {
                                action = Some(Action::New(parent, false));
                            }
                        }
                        if !self.service.project_preferences().follow_active_file {
                            let in_tree = self.service.active_document().is_some_and(|d| {
                                self.service.workspace_root_for(&d.path).is_some()
                            });
                            ui.add_enabled_ui(in_tree, |ui| {
                                if icon_button(ui, Icon::Locate, "Select Opened File", false, 26.)
                                    .clicked()
                                {
                                    self.reveal_active_project_file();
                                    self.focus_editor = true;
                                }
                            });
                        }
                    },
                );
            });
        });
        ui.add_space(4.);
        if self.project_files_ui.input.is_none()
            && let Some(error) = self
                .project_files_ui
                .error
                .as_ref()
                .or(self.service.project_files.error.as_ref())
                .map(String::as_str)
                .or(self.service.tree_error())
        {
            crate::status_messages::status_message(ui, error).on_hover_text(error);
        }
        let Some(root) = root else {
            egui::Frame::NONE
                .inner_margin(self.tokens.content_margin())
                .show(ui, |ui| {
                    ui.add(egui::Label::new("Abra uma pasta para explorar seus arquivos.").wrap());
                });
            return;
        };
        let menu_state = self.project_menu_state(&root);
        let root_name = self.service.workspace_label();
        let row = tree_row(
            ui,
            &root_name,
            &root_name,
            0,
            Some(!self.project_files_ui.root_collapsed),
            self.project_files_ui.selection.contains(&root),
            self.project_tree_has_focus(ui.ctx()),
        );
        if row.enabled() && self.project_files_ui.focus_path.as_ref() == Some(&root) {
            self.project_files_ui.focus_path = None;
            self.project_files_ui.focus = Some(row.id);
            row.request_focus();
        }
        self.project_drag_row(ui, &row, &root, true, false);
        if row.clicked() {
            let (modifiers, pointer) = self.project_row_click(ui, &row);
            let disclosure = self.project_disclosure_clicked(&row, 0, modifiers, pointer);
            let activate = if disclosure {
                self.project_files_ui.selected = Some(root.clone());
                true
            } else {
                self.select_project_click(&root, modifiers)
            };
            self.project_files_ui.focus = Some(row.id);
            row.request_focus();
            if activate {
                self.project_files_ui.root_collapsed = !self.project_files_ui.root_collapsed;
            }
            self.focus_editor = false;
        }
        if row.secondary_clicked() {
            if menu_state.git.as_ref().is_some_and(|root| {
                self.service
                    .git
                    .repo
                    .as_ref()
                    .is_some_and(|repository| &repository.root == root)
            }) && self.service.git.remotes.is_empty()
            {
                self.git_send(codisseu_core::git::GitCommand::Remotes);
            }
            self.select_project_context(&root, &row);
        }
        let selection_count = self.project_files_ui.selection.len();
        row.context_menu(|ui| {
            project_menu(
                ui,
                &root,
                true,
                true,
                &menu_state,
                &self.tokens,
                selection_count,
                &mut action,
            )
        });
        egui::ScrollArea::both()
            .id_salt("codisseu.tree.scroll")
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                self.project_drag_scroll(ui);
                if !self.project_files_ui.root_collapsed {
                    self.project_tree_rows(ui, &root, &mut action);
                }
                let rect = ui.available_rect_before_wrap();
                if rect.height() > 0. {
                    let blank = ui.allocate_rect(rect, egui::Sense::click());
                    self.project_drag_row(ui, &blank, &root, true, false);
                    blank.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Pane,
                            true,
                            "Explorer empty area",
                        )
                    });
                    if blank.clicked() {
                        self.select_project_single(root.clone());
                        self.project_files_ui.focus = Some(blank.id);
                        blank.request_focus();
                        self.focus_editor = false;
                    }
                    blank.context_menu(|ui| {
                        project_menu(
                            ui,
                            &root,
                            true,
                            true,
                            &menu_state,
                            &self.tokens,
                            1,
                            &mut action,
                        )
                    });
                }
            });
        if let Some(action) = action {
            self.project_tree_action(action);
        }
    }
    fn project_tree_rows(
        &mut self,
        ui: &mut egui::Ui,
        root: &std::path::Path,
        action: &mut Option<Action>,
    ) {
        // Discovery returns a preorder tree. Skip a collapsed subtree by depth;
        // do not rebuild/hash every ancestor path for every file on each frame.
        let entries = self.visible_project_entries();
        // An inline row follows its parent, including the root of an empty folder.
        if self
            .project_files_ui
            .input
            .as_ref()
            .is_some_and(|input| input.parent == root && input.kind != NameKind::Rename)
        {
            self.project_name_row(ui, 1);
        }
        // Keep inline editing laid out so its focus/scroll-to-me survives. In
        // normal browsing only construct widgets for the viewport (+1 row).
        let row_height = 24. + ui.spacing().item_spacing.y;
        let start_y = ui.next_widget_position().y;
        if let Some(path) = self.project_files_ui.scroll_to.take()
            && let Some(index) = entries
                .iter()
                .position(|&i| self.service.entries[i].path == path)
        {
            // The target widget may not exist in this frame's virtual viewport.
            // Request scroll using its logical row; next frame builds that row.
            let entry = &self.service.entries[entries[index]];
            let top = egui::pos2(
                ui.min_rect().left() + 24. + (entry.depth + 1) as f32 * 16. + 34.,
                start_y + index as f32 * row_height,
            );
            let mut target = egui::Rect::from_min_size(top, egui::vec2(64., row_height));
            let clip = ui.clip_rect();
            if top.x >= clip.left() && top.x < clip.right() {
                // Reveal vertically without centering an already visible name
                // horizontally. Truly off-screen columns retain horizontal reveal.
                target.min.x = clip.left();
                target.max.x = clip.right();
            }
            ui.scroll_to_rect(target, Some(egui::Align::Center));
        }
        let visible = if self.project_files_ui.input.is_some() {
            0..entries.len()
        } else {
            let start = ((ui.clip_rect().top() - start_y) / row_height)
                .floor()
                .max(0.) as usize;
            let end = ((ui.clip_rect().bottom() - start_y) / row_height)
                .ceil()
                .max(0.) as usize
                + 1;
            start.min(entries.len())..end.min(entries.len())
        };
        ui.set_min_width(self.project_files_ui.tree_width);
        ui.add_space(visible.start as f32 * row_height);
        for &index in &entries[visible.clone()] {
            let entry = self.service.entries[index].clone();
            let menu_state = self.project_menu_state(&entry.path);
            if self
                .project_files_ui
                .input
                .as_ref()
                .is_some_and(|input| input.source.as_ref() == Some(&entry.path))
            {
                self.project_name_row(ui, entry.depth + 1);
                continue;
            }
            let name = self
                .service
                .workspace_folder_name(&entry.path)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                });
            let name = if self
                .service
                .workspace_folders
                .iter()
                .any(|f| f.path == entry.path && !f.available)
            {
                format!("{name} (pasta ausente)")
            } else {
                name
            };
            let selected = self.project_files_ui.selection.contains(&entry.path);
            let selection_active = self.project_tree_has_focus(ui.ctx());
            let old_tint = ui.visuals().override_text_color;
            if let Some(status) =
                self.service
                    .git
                    .repo
                    .as_ref()
                    .and_then(|repo| entry.path.strip_prefix(&repo.root).ok())
                    .and_then(|path| {
                        self.service.git.snapshot.as_ref().and_then(|snapshot| {
                            snapshot.entries.iter().find(|item| item.path == path)
                        })
                    })
            {
                ui.visuals_mut().override_text_color =
                    Some(git_commit::git_file_color(status, &self.tokens));
            }
            let row = ui
                .push_id(&entry.path, |ui| {
                    tree_row(
                        ui,
                        &name,
                        &name,
                        entry.depth + 1,
                        entry.is_dir.then(|| self.expanded.contains(&entry.path)),
                        selected,
                        selection_active,
                    )
                })
                .inner
                .interact(egui::Sense::drag())
                .on_hover_text(entry.path.display().to_string());
            if row.enabled() && self.project_files_ui.focus_path.as_ref() == Some(&entry.path) {
                self.project_files_ui.focus_path = None;
                self.project_files_ui.focus = Some(row.id);
                row.request_focus();
            }
            self.project_drag_row(ui, &row, &entry.path, entry.is_dir, true);
            ui.visuals_mut().override_text_color = old_tint;
            if row.clicked() {
                let (modifiers, pointer) = self.project_row_click(ui, &row);
                let disclosure = entry.is_dir
                    && self.project_disclosure_clicked(&row, entry.depth + 1, modifiers, pointer);
                let activate = if disclosure {
                    self.project_files_ui.selected = Some(entry.path.clone());
                    true
                } else {
                    self.select_project_click(&entry.path, modifiers)
                };
                self.project_files_ui.focus = Some(row.id);
                row.request_focus();
                self.focus_editor = false;
                if activate && entry.is_dir {
                    row.request_focus();
                    self.focus_editor = false;
                    if !self.expanded.remove(&entry.path) {
                        self.expanded.insert(entry.path.clone());
                        let _ = self
                            .service
                            .dispatch(Command::LoadDirectory(entry.path.clone()));
                    }
                } else if activate {
                    self.send(Command::OpenFile(entry.path.clone()));
                    row.request_focus();
                    self.focus_editor = row.double_clicked();
                }
            }
            // Secondary click selects the context target without opening the editor.
            if row.secondary_clicked() {
                if menu_state.git.as_ref().is_some_and(|root| {
                    self.service
                        .git
                        .repo
                        .as_ref()
                        .is_some_and(|repository| &repository.root == root)
                }) && self.service.git.remotes.is_empty()
                {
                    self.git_send(codisseu_core::git::GitCommand::Remotes);
                }
                self.select_project_context(&entry.path, &row);
            }
            let selection_count = self.project_files_ui.selection.len();
            row.context_menu(|ui| {
                project_menu(
                    ui,
                    &entry.path,
                    entry.is_dir,
                    self.service.workspace_folder_name(&entry.path).is_some(),
                    &menu_state,
                    &self.tokens,
                    selection_count,
                    action,
                )
            });
            if entry.is_dir
                && self.expanded.contains(&entry.path)
                && self.project_files_ui.input.as_ref().is_some_and(|input| {
                    input.parent == entry.path && input.kind != NameKind::Rename
                })
            {
                self.project_name_row(ui, entry.depth + 2);
            }
        }
        self.project_files_ui.tree_width =
            self.project_files_ui.tree_width.max(ui.min_rect().width());
        ui.add_space((entries.len() - visible.end) as f32 * row_height);
    }
    fn project_name_row(&mut self, ui: &mut egui::Ui, depth: usize) {
        let error = self.project_files_ui.error.clone();
        let Some(input) = self.project_files_ui.input.as_mut() else {
            return;
        };
        let label = match input.kind {
            NameKind::File => "File name",
            NameKind::Folder => "Folder name",
            NameKind::Rename => "Rename file",
        };
        let mut enter = false;
        let mut changed = false;
        let viewport = ui.clip_rect();
        ui.horizontal(|ui| {
            ui.add_space(24. + depth as f32 * 16. + 34.);
            // The content can be wider than the viewport after measuring long
            // names. Inline editing must fit the visible column, not that cache.
            let width = (viewport.right() - ui.next_widget_position().x - 12.).max(24.);
            let focus = input.focus;
            let mut output = egui::TextEdit::singleline(&mut input.value)
                .id(egui::Id::unique("codisseu.explorer.name"))
                .hint_text(label)
                .desired_width(width)
                .show(ui);
            output.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, label)
            });
            if input.focus {
                output.response.request_focus();
                if input.kind == NameKind::Rename {
                    let count = std::path::Path::new(&input.value)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .map_or(input.value.chars().count(), |name| name.chars().count());
                    output
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(count),
                        )));
                    output.state.store(ui.ctx(), output.response.id);
                }
                input.focus = false;
            }
            if focus || output.response.has_focus() || output.response.changed() {
                // TextEdit keeps its caret visible internally. Its parent scroll
                // request must only reveal the row vertically: panning the whole
                // list leaves children to the left of the fixed root header.
                ui.scroll_to_rect(
                    egui::Rect::from_min_max(
                        egui::pos2(viewport.left(), output.response.rect.top()),
                        egui::pos2(viewport.right(), output.response.rect.bottom()),
                    ),
                    None,
                );
            }
            if error.is_some() {
                ui.painter().rect_stroke(
                    output.response.rect.expand(1.),
                    2.,
                    egui::Stroke::new(1., color(self.tokens.error)),
                    egui::StrokeKind::Outside,
                );
            }
            changed = output.response.changed();
            enter = output.response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));
        });
        if changed {
            self.project_files_ui.error = None;
            self.service.project_files.error = None;
        } else if let Some(error) = error {
            ui.horizontal(|ui| {
                ui.add_space(24. + depth as f32 * 16. + 34.);
                crate::status_messages::status_message(ui, &error).on_hover_text(error);
            });
        }
        if enter {
            let Some(input) = self.project_files_ui.input.take() else {
                return;
            };
            let command = match input.kind {
                NameKind::File => ProjectFileCommand::CreateFile {
                    parent: input.parent.clone(),
                    name: input.value.clone(),
                },
                NameKind::Folder => ProjectFileCommand::CreateFolder {
                    parent: input.parent.clone(),
                    name: input.value.clone(),
                },
                NameKind::Rename => ProjectFileCommand::RenameFile {
                    path: input.source.clone().unwrap(),
                    name: input.value.clone(),
                },
            };
            if let Err(error) = self.service.dispatch(Command::ProjectFile(command)) {
                if self.service.last_error.as_ref() == Some(&error) {
                    self.service.last_error = None;
                }
                self.project_files_ui.error = Some(error);
                self.project_files_ui.input = Some(NameInput {
                    focus: true,
                    ..input
                });
            } else {
                self.project_files_ui.error = None;
                self.project_files_ui.submitted = Some(input);
            }
        }
    }
    pub(super) fn project_tree_has_focus(&self, ctx: &egui::Context) -> bool {
        self.project_files_ui
            .focus
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)))
    }
    pub(super) fn project_file_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.project_files_ui.input.is_some() {
            if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
                self.project_files_ui.input = None;
                self.project_files_ui.error = None;
                self.service.project_files.error = None;
                return true;
            }
            return false;
        }
        if !self
            .project_files_ui
            .focus
            .is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)))
        {
            return false;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::A) {
            self.project_files_ui.selection = if self.project_files_ui.root_collapsed {
                Default::default()
            } else {
                self.visible_project_entries()
                    .iter()
                    .map(|&i| self.service.entries[i].path.clone())
                    .collect()
            };
            self.project_files_ui.anchor = self.project_files_ui.selection.first().cloned();
            self.project_files_ui.range_base.clear();
            self.project_files_ui.selection_modified = true;
            return true;
        }
        if consume_exact(ctx, Modifiers::NONE, Key::Delete) {
            let paths = self.project_files_ui.selection.iter().cloned().collect();
            self.begin_project_trash(paths);
            return true;
        }
        if self.project_files_ui.selection.len() != 1 {
            // These operations still have a single-item contract. Never let
            // an ambiguous multi-selection silently act on only the lead row.
            if consume_exact(ctx, Modifiers::NONE, Key::F2)
                || consume_exact(ctx, Modifiers::COMMAND, Key::C)
                || consume_exact(ctx, Modifiers::COMMAND, Key::X)
                || ctx.input_mut(|i| {
                    let before = i.events.len();
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Copy | egui::Event::Cut));
                    before != i.events.len()
                })
            {
                self.project_files_ui.error =
                    Some("Selecione um único item para esta ação.".into());
                return true;
            }
            return false;
        }
        let Some(path) = self.project_files_ui.selection.first().cloned() else {
            return false;
        };
        let is_root = self.service.workspace.as_ref() == Some(&path)
            || self.service.workspace_folder_name(&path).is_some();
        if self.project_extra_shortcuts(ctx, &path, is_root) {
            return true;
        }
        if !is_root && consume_exact(ctx, Modifiers::NONE, Key::F2) {
            self.project_tree_action(Action::Rename(path));
            return true;
        }
        false
    }
    pub(super) fn project_file_dialog(&mut self, ctx: &egui::Context) {
        self.project_extra_dialog(ctx);
        let Some(paths) = self.project_files_ui.trash.clone() else {
            return;
        };
        let dirty = self.service.documents.iter().any(|document| {
            document.is_dirty() && paths.iter().any(|path| document.path.starts_with(path))
        });
        let mut confirm = false;
        let mut cancel = false;
        let focus = std::mem::take(&mut self.project_files_ui.trash_focus);
        let modal = egui::Modal::new(egui::Id::unique("codisseu.explorer.trash"))
            .frame(self.tokens.dialog_frame(ctx))
            .show(ctx, |ui| {
                let title = if paths.len() == 1 {
                    let name = paths[0].file_name().unwrap_or_default().to_string_lossy();
                    format!("Move ‘{name}’ to Trash?")
                } else {
                    format!("Move {} items to Trash?", paths.len())
                };
                self.tokens.dialog_header(ui, ctx, 420., &title);
                ui.add(
                    egui::Label::new(if paths.len() == 1 {
                        "You can restore this item from the system Trash."
                    } else {
                        "You can restore these items from the system Trash."
                    })
                    .wrap(),
                );
                egui::Frame::new()
                    .fill(color(self.tokens.bar))
                    .stroke(egui::Stroke::new(1., color(self.tokens.border)))
                    .corner_radius(self.tokens.control_corner_radius)
                    .inner_margin(8)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("explorer.trash.review")
                            .max_height((ctx.content_rect().height() * 0.3).max(60.))
                            .auto_shrink([false, true])
                            .codisseu()
                            .show_rows(ui, 22., paths.len(), |ui, range| {
                                for path in &paths[range] {
                                    let label = self
                                        .service
                                        .workspace_root_for(path)
                                        .and_then(|root| path.strip_prefix(root).ok())
                                        .unwrap_or(path);
                                    ui.add(egui::Label::new(label.to_string_lossy()).truncate())
                                        .on_hover_text(path.display().to_string());
                                }
                            });
                    });
                if dirty {
                    ui.add(
                        egui::Label::new(
                            "Salve ou feche os arquivos com alterações antes de excluir.",
                        )
                        .wrap(),
                    );
                }
                self.tokens.dialog_footer(ui, |ui| {
                    let confirm_button = ui.add_enabled(
                        !dirty && !self.service.project_files.pending,
                        self.tokens.dialog_button("Move to Trash", true),
                    );
                    confirm = confirm_button.clicked();
                    let cancel_button = ui.add(self.tokens.dialog_button("Cancel", false));
                    cancel = cancel_button.clicked();
                    if focus {
                        if dirty {
                            cancel_button.request_focus();
                        } else {
                            confirm_button.request_focus();
                        }
                    }
                });
            });
        cancel |= modal.should_close();
        if confirm {
            match self
                .service
                .dispatch(Command::ProjectFile(ProjectFileCommand::TrashMany {
                    paths,
                })) {
                Ok(()) => self.project_files_ui.trash_submitted = true,
                Err(error) => {
                    self.service.last_error = None;
                    self.project_files_ui.error = Some(error);
                }
            }
            self.project_files_ui.trash = None;
        } else if cancel {
            self.project_files_ui.trash = None;
            // Restore after the modal layer disappears; requesting a background
            // widget during the closing frame can be rejected by the focus trap.
            self.project_files_ui.focus_path = self.project_files_ui.selected.clone();
            self.project_files_ui.scroll_to = self.project_files_ui.selected.clone();
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn project_menu(
    ui: &mut egui::Ui,
    path: &std::path::Path,
    folder: bool,
    root: bool,
    state: &actions::MenuState,
    tokens: &Tokens,
    selection_count: usize,
    action: &mut Option<Action>,
) {
    tokens.menu_style(ui.style_mut(), false);
    ui.set_min_width(302.);
    ui.set_max_width(360.);
    egui::ScrollArea::vertical()
        .id_salt(("explorer.context", path))
        .max_height((ui.ctx().content_rect().height() - 42.).max(160.))
        .auto_shrink([false, true])
        .codisseu()
        .show(ui, |ui| {
            if selection_count > 1 {
                ui.label(format!("{selection_count} items selected"));
                ui.separator();
                let delete =
                    ui.add_enabled(!root, egui::Button::new("Delete").shortcut_text("Del"));
                delete.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::Button,
                        delete.enabled(),
                        "Delete",
                    )
                });
                ui.ctx()
                    .accesskit_node_builder(delete.id, |node| node.set_keyboard_shortcut("Delete"));
                if delete.clicked() {
                    *action = Some(Action::Trash(path.to_owned()));
                    ui.close();
                }
                return;
            }
            if folder {
                ui.menu_button("Codificação da pasta", |ui| {
                    ui.add(egui::Label::new(path.display().to_string()).wrap());
                    ui.separator();
                    for encoding in codisseu_core::FileEncoding::ALL {
                        if ui.button(encoding.label()).clicked() {
                            *action = Some(Action::Encoding(path.to_path_buf(), encoding));
                            ui.close();
                        }
                    }
                });
                ui.separator();
            }
            actions::menu_contents(ui, path, folder, root, state, action)
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{
        thread,
        time::{Duration, Instant},
    };
    fn fixture() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("monolito");
        std::fs::create_dir(&root).unwrap();
        let mut service = AppService::new(dir.path().join("profile/session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_max_steps(40)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(service),
            );
        for _ in 0..5 {
            h.step();
        }
        (dir, h)
    }
    fn key(h: &mut Harness<'_, CodisseuApp>, mut modifiers: Modifiers, key: Key) {
        modifiers.command = modifiers.ctrl;
        h.key_press_modifiers(modifiers, key);
        h.step();
        h.step();
    }
    fn settle(h: &mut Harness<'_, CodisseuApp>) {
        let until = Instant::now() + Duration::from_secs(10);
        while h.state().service.is_busy() {
            h.step();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        h.step();
        h.step();
    }
    fn settle_reveal(h: &mut Harness<'_, CodisseuApp>) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            h.step();
            assert!(Instant::now() < until, "reveal did not settle");
            if !h.state().service.is_busy() && h.state().project_files_ui.reveal.is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        for _ in 0..5 {
            h.step();
        }
    }
    #[test]
    fn reveal_scrolls_horizontally_when_deep_ancestors_were_outside_virtual_viewport() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        for i in 0..40 {
            std::fs::create_dir(root.join(format!("a{i:02}"))).unwrap();
        }
        let mut parent = root.join("z-service");
        for _ in 0..20 {
            parent = parent.join("nested");
        }
        std::fs::create_dir_all(&parent).unwrap();
        let path = parent.join("Deep.php");
        std::fs::write(&path, "<?php").unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.state_mut().send(Command::OpenFile(path.clone()));
        settle_reveal(&mut h);
        let row = h
            .get_all_by_label("Deep.php")
            .into_iter()
            .find(|n| n.rect().left() < 340.)
            .unwrap();
        let depth = h
            .state()
            .service
            .entries
            .iter()
            .find(|e| e.path == path)
            .unwrap()
            .depth;
        let text_x = row.rect().left() + 24. + (depth + 1) as f32 * 16. + 34.;
        assert!(
            (50. ..270.).contains(&text_x),
            "file label outside Project viewport: {text_x}"
        );
    }
    #[test]
    fn following_opens_only_ancestors_and_scrolls_to_virtualized_file_without_editor_changes() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        let parent = root.join("service/app/Http");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::create_dir_all(root.join("unrelated/closed")).unwrap();
        for i in 0..150 {
            std::fs::write(parent.join(format!("a{i:03}.php")), "<?php").unwrap();
        }
        let path = parent.join("zz-target.php");
        std::fs::write(&path, "<?php // target").unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        assert!(!h.state().service.tree_directory_loaded(&parent));
        h.state_mut().send(Command::OpenFile(path.clone()));
        settle_reveal(&mut h);
        assert_eq!(h.state().project_files_ui.selected.as_ref(), Some(&path));
        assert!(h.state().service.tree_directory_loaded(&parent));
        assert!(
            !h.state()
                .service
                .tree_directory_loaded(&root.join("unrelated"))
        );
        let row = h
            .get_all_by_label("zz-target.php")
            .into_iter()
            .find(|n| n.rect().left() < 340.)
            .expect("target is in rendered viewport");
        assert!(
            row.rect().top() > 90. && row.rect().bottom() <= 860.,
            "{:?}",
            row.rect()
        );
        let d = h.state().service.active_document().unwrap();
        assert!(!d.is_dirty() && !d.can_undo());
        assert_eq!(d.text(), "<?php // target");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), d.text());
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        h.get_by_label("Collapse All").click();
        h.step();
        h.step();
        assert!(h.state().expanded.is_empty());
        for _ in 0..8 {
            h.step();
        }
        assert!(
            h.state().expanded.is_empty(),
            "stationary active file must not reopen collapsed folders"
        );
    }
    #[test]
    fn manual_target_and_behavior_menu_use_active_file_and_keep_caret_and_focus() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        let path = root.join("src/deep/Manual.php");
        std::fs::write(&path, "<?php // manual").unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.get_by_label("Project Options").click();
        h.step();
        h.step();
        h.get_by_label("Behavior").click();
        h.step();
        h.step();
        h.get_by_label("Always Select Opened File").click();
        h.step();
        h.step();
        assert!(!h.state().service.project_preferences().follow_active_file);
        h.state_mut().send(Command::OpenFile(path.clone()));
        settle_reveal(&mut h);
        assert!(!h.state().expanded.contains(&root.join("src")));
        let d = h.state().service.active_document().unwrap();
        let (id, version) = (d.id, d.version);
        h.state_mut().send(Command::SetSelection {
            id,
            expected_version: version,
            selection: codisseu_core::Selection::caret(7),
        });
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        h.get_by_label("Select Opened File").click();
        settle_reveal(&mut h);
        assert_eq!(h.state().project_files_ui.selected.as_ref(), Some(&path));
        assert_eq!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .selection()
                .head,
            7
        );
        assert!(
            h.get_by_role(egui::accesskit::Role::MultilineTextInput)
                .is_focused()
        );
        assert_eq!(
            h.state().service.active_document().unwrap().version,
            version
        );
        assert_eq!(h.state().service.documents.len(), 1);
    }
    #[test]
    fn pending_reveal_is_replaced_on_tab_change_and_cleared_on_root_change_or_outside_file() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        std::fs::create_dir_all(root.join("deep/nested")).unwrap();
        let old = root.join("deep/nested/old.php");
        let new = root.join("new.php");
        let outside = dir.path().join("outside.php");
        for p in [&old, &new, &outside] {
            std::fs::write(p, "<?php").unwrap();
        }
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.state_mut()
            .send(Command::SetProjectFollowActiveFile(false));
        for p in [&old, &new] {
            h.state_mut().send(Command::OpenFile(p.clone()));
            settle_reveal(&mut h);
        }
        let old_id = h
            .state()
            .service
            .documents
            .iter()
            .find(|d| d.path == old)
            .unwrap()
            .id;
        let view = h.state().service.active_view().unwrap();
        h.state_mut()
            .send(Command::ActivateTab { view, id: old_id });
        h.step();
        h.state_mut().reveal_active_project_file();
        assert_eq!(h.state().project_files_ui.reveal.as_ref(), Some(&old));
        let new_id = h
            .state()
            .service
            .documents
            .iter()
            .find(|d| d.path == new)
            .unwrap()
            .id;
        h.state_mut()
            .send(Command::ActivateTab { view, id: new_id });
        settle_reveal(&mut h);
        assert_ne!(h.state().project_files_ui.selected.as_ref(), Some(&old));
        h.state_mut().send(Command::OpenFile(outside));
        settle_reveal(&mut h);
        h.state_mut().reveal_active_project_file();
        assert!(h.state().project_files_ui.reveal.is_none());
        h.state_mut()
            .send(Command::SetProjectFollowActiveFile(true));
        h.state_mut().send(Command::CloseWorkspace);
        settle_reveal(&mut h);
        assert!(h.state().project_files_ui.reveal.is_none());
        assert!(h.state().project_files_ui.scroll_to.is_none());
    }
    #[test]
    fn removed_branch_stops_reveal_without_changing_open_buffer() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        let branch = root.join("vanished");
        std::fs::create_dir(&branch).unwrap();
        let path = branch.join("Open.php");
        std::fs::write(&path, "<?php // retained").unwrap();
        h.state_mut()
            .send(Command::SetProjectFollowActiveFile(false));
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.state_mut().send(Command::OpenFile(path.clone()));
        settle_reveal(&mut h);
        let version = h.state().service.active_document().unwrap().version;
        std::fs::remove_dir_all(&branch).unwrap();
        h.state_mut().reveal_active_project_file();
        settle_reveal(&mut h);
        assert!(h.state().project_files_ui.reveal.is_none());
        assert!(h.state().project_files_ui.scroll_to.is_none());
        assert!(
            h.state().service.tree_directory_error(&branch).is_some()
                || !h.state().service.entries.iter().any(|e| e.path == branch),
            "directory failure or a watcher reconciliation must terminate reveal"
        );
        for _ in 0..8 {
            h.step();
        }
        assert!(!h.state().service.is_busy());
        let d = h.state().service.active_document().unwrap();
        assert_eq!(d.path, path);
        assert_eq!(d.text(), "<?php // retained");
        assert_eq!(d.version, version);
        assert!(!d.can_undo());
        assert!(!h.state().show_activity);
    }
    #[test]
    fn empty_explorer_toolbar_inline_create_and_escape_are_functional() {
        let (dir, mut h) = fixture();
        h.get_by_label("New File…").click();
        h.step();
        h.step();
        h.event(egui::Event::Text("cancelled.txt".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert!(h.state().project_files_ui.input.is_none());
        assert!(!dir.path().join("monolito/cancelled.txt").exists());
        h.get_by_label("New Folder…").click();
        h.step();
        h.step();
        h.event(egui::Event::Text("app/Http".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        assert!(dir.path().join("monolito/app/Http").is_dir());
        h.get_by_label("Collapse All").click();
        h.step();
        h.step();
        h.get_by_label("New File…").click();
        h.step();
        h.step();
        h.event(egui::Event::Text("Ação.php".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        let path = dir.path().join("monolito/app/Http/Ação.php");
        assert!(path.is_file());
        assert_eq!(h.state().service.active_document().unwrap().path, path);
    }
    #[test]
    fn tree_f2_renames_generic_file_and_delete_cancel_keeps_window_and_disk() {
        let (dir, mut h) = fixture();
        let path = dir.path().join("monolito/notes.txt");
        std::fs::write(&path, "original").unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.get_by_label("notes.txt").click();
        h.step();
        h.step();
        settle(&mut h);
        key(&mut h, Modifiers::NONE, Key::F2);
        assert!(h.state().project_files_ui.input.is_some());
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Text("renamed.md".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        assert!(!path.exists());
        let renamed = dir.path().join("monolito/renamed.md");
        assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "original");
        h.get_all_by_label("renamed.md")
            .into_iter()
            .find(|node| node.rect().left() < 300.)
            .unwrap()
            .click();
        h.step();
        h.step();
        key(&mut h, Modifiers::NONE, Key::Delete);
        assert_eq!(
            h.state().project_files_ui.trash.as_deref(),
            Some([renamed.clone()].as_slice())
        );
        h.get_by_label("Cancel").click();
        h.step();
        h.step();
        assert!(h.state().project_files_ui.trash.is_none());
        assert!(renamed.exists());
        assert!(!h.state().allow_close);
        assert!(!h.state().exit_when_saved);
    }
    #[test]
    fn collision_restores_inline_name_without_opening_unrelated_tool_windows() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        std::fs::write(root.join("a.txt"), "source").unwrap();
        std::fs::write(root.join("existing.txt"), "destination").unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.get_by_label("a.txt").click();
        h.step();
        h.step();
        settle(&mut h);
        key(&mut h, Modifiers::NONE, Key::F2);
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Text("existing.txt".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        assert_eq!(
            h.state().project_files_ui.input.as_ref().unwrap().value,
            "existing.txt"
        );
        assert!(
            h.state()
                .project_files_ui
                .error
                .as_ref()
                .unwrap()
                .contains("Destino já existe")
        );
        assert!(h.state().service.last_error.is_none());
        assert!(!h.state().show_activity);
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "source"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("existing.txt")).unwrap(),
            "destination"
        );
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Text("renamed.md".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        assert_eq!(
            std::fs::read_to_string(root.join("renamed.md")).unwrap(),
            "source"
        );
        assert!(h.state().project_files_ui.input.is_none());
        assert!(h.state().project_files_ui.error.is_none());
    }
    #[test]
    fn f12_requests_definition_only_when_the_code_editor_is_focused() {
        let (dir, mut h) = fixture();
        let path = dir.path().join("monolito/main.ts");
        std::fs::write(&path, "export const target = 42;").unwrap();
        h.state_mut().send(Command::OpenFile(path));
        settle(&mut h);
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        key(&mut h, Modifiers::NONE, Key::F12);
        assert!(
            h.state()
                .service
                .navigation
                .error
                .as_ref()
                .unwrap()
                .contains("Inicie o provider")
        );
        h.state_mut().send(Command::CancelDefinition);
        h.step();
        assert!(h.state().service.navigation.error.is_none());
        h.get_by_label("New File…").click();
        h.step();
        h.step();
        key(&mut h, Modifiers::NONE, Key::F12);
        assert!(h.state().service.navigation.error.is_none());
        assert!(h.state().project_files_ui.input.is_some());
    }
    #[test]
    fn explorer_copy_cut_paste_and_folder_rename_preserve_saved_bytes() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        let source = root.join("original.txt");
        std::fs::write(&source, "á🙂 original").unwrap();
        std::fs::create_dir(root.join("destination")).unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.get_by_label("original.txt").click();
        h.step();
        h.step();
        settle(&mut h);
        key(&mut h, Modifiers::CTRL, Key::C);
        h.get_by_label("destination").click();
        h.step();
        h.step();
        key(&mut h, Modifiers::CTRL, Key::V);
        settle(&mut h);
        let copied = root.join("destination/original.txt");
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&copied).unwrap()
        );
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Cut, copied.clone()));
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Paste, root.clone()));
        settle(&mut h);
        assert!(
            h.state().project_files_ui.error.is_some()
                || h.state().service.project_files.error.is_some()
        );
        assert!(source.exists());
        assert!(copied.exists());
        h.state_mut()
            .project_tree_action(Action::Rename(root.join("destination")));
        h.step();
        h.step();
        key(&mut h, Modifiers::CTRL, Key::A);
        h.event(egui::Event::Text("renamed-folder".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Enter);
        settle(&mut h);
        assert!(root.join("renamed-folder/original.txt").is_file());
        assert!(!root.join("destination").exists());
        h.state_mut().project_tree_action(Action::Extra(
            ExplorerAction::Cut,
            root.join("renamed-folder/original.txt"),
        ));
        std::fs::create_dir(root.join("moved")).unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Paste, root.join("moved")));
        settle(&mut h);
        assert!(root.join("moved/original.txt").is_file());
        assert!(!root.join("renamed-folder/original.txt").exists());
        assert!(h.state().project_files_ui.extras.clipboard.is_none());
    }
    #[test]
    fn explorer_open_to_side_keeps_the_original_view_and_shared_document_model() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        let left = root.join("left.txt");
        let right = root.join("right.txt");
        std::fs::write(&left, "left").unwrap();
        std::fs::write(&right, "right").unwrap();
        h.state_mut().send(Command::OpenFile(left.clone()));
        settle(&mut h);
        let original = h.state().service.active_document().unwrap().id;
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Side, right.clone()));
        settle(&mut h);
        assert_eq!(h.state().service.document_views().len(), 2);
        assert_eq!(h.state().service.document_views()[0].document, original);
        assert_eq!(h.state().service.active_document().unwrap().path, right);
        assert_eq!(std::fs::read_to_string(left).unwrap(), "left");
    }
    #[test]
    fn explorer_context_menu_groups_are_specific_to_files_and_folders() {
        let path = PathBuf::from("/fixture/item");
        for folder in [false, true] {
            let path = path.clone();
            let mut h = Harness::builder().with_size([440., 850.]).build_ui_state(
                move |ui, action: &mut Option<Action>| {
                    let state = actions::MenuState::for_test();
                    actions::menu_contents(ui, &path, folder, false, &state, action);
                },
                None,
            );
            h.step();
            h.step();
            assert!(
                h.query_by_label("Open Containing Folder Alt+Ctrl+R")
                    .is_some()
            );
            assert!(h.query_by_label("Open in Integrated Terminal ").is_some());
            assert!(h.query_by_label("Copy Path Alt+Ctrl+C").is_some());
            assert!(h.query_by_label("Rename… F2").is_some());
            assert_eq!(h.query_by_label("New File… ").is_some(), folder);
            assert_eq!(
                h.query_by_label("Open to the Side Ctrl+Enter").is_some(),
                !folder
            );
            assert!(h.query_by_label("Add File to Chat").is_none());
            h.get_by_label("Copy Ctrl+C").click();
            h.step();
            assert!(matches!(
                h.state(),
                Some(Action::Extra(ExplorerAction::Copy, _))
            ));
        }
    }
    #[test]
    fn open_to_side_from_right_uses_the_other_existing_group_and_folder_shortcut_is_safe() {
        let (dir, mut h) = fixture();
        let root = dir.path().join("monolito");
        for name in ["one.txt", "two.txt", "three.txt"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        h.state_mut().send(Command::OpenFile(root.join("one.txt")));
        settle(&mut h);
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Side, root.join("two.txt")));
        settle(&mut h);
        let right_view = h.state().service.active_view().unwrap();
        let right_document = h.state().service.active_document().unwrap().id;
        h.state_mut()
            .project_tree_action(Action::Extra(ExplorerAction::Side, root.join("three.txt")));
        settle(&mut h);
        assert_ne!(h.state().service.active_view(), Some(right_view));
        assert_eq!(
            h.state().service.view(right_view).unwrap().document,
            right_document
        );
        assert_eq!(
            h.state().service.active_document().unwrap().path,
            root.join("three.txt")
        );
        std::fs::create_dir(root.join("folder")).unwrap();
        h.state_mut()
            .send(Command::ProjectFile(ProjectFileCommand::Refresh));
        settle(&mut h);
        h.get_by_label("folder").click();
        h.step();
        h.step();
        key(&mut h, Modifiers::CTRL, Key::Enter);
        settle(&mut h);
        assert!(h.state().service.last_error.is_none());
        assert!(
            h.state()
                .service
                .documents
                .iter()
                .all(|document| document.path != root.join("folder"))
        );
    }
    #[test]
    fn folder_encoding_menu_recovers_legacy_file_before_it_can_be_opened() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let folder = root.join("group/legacy");
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("a.php");
        std::fs::write(&path, b"<?php // \xe9").unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        while service.is_busy() {
            service.poll();
            thread::sleep(Duration::from_millis(2));
        }
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        while service.is_busy() {
            service.poll();
            thread::sleep(Duration::from_millis(2));
        }
        assert!(service.last_error.is_some());
        assert!(service.documents.is_empty());
        let target = folder.clone();
        let mut h = Harness::builder().with_size([800., 800.]).build_ui_state(
            move |ui, app: &mut CodisseuApp| {
                app.service.poll();
                let state = app.project_menu_state(&target);
                let mut action = None;
                project_menu(
                    ui,
                    &target,
                    true,
                    false,
                    &state,
                    &app.tokens,
                    1,
                    &mut action,
                );
                if let Some(action) = action {
                    app.project_tree_action(action);
                }
            },
            CodisseuApp::new(service),
        );
        h.run();
        h.get_by_label("Codificação da pasta").click();
        h.run();
        h.get_by_label("ISO-8859-1").click();
        h.run();
        while h.state().service.is_busy() {
            h.step();
            thread::sleep(Duration::from_millis(2));
        }
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        while h.state().service.is_busy() {
            h.step();
            thread::sleep(Duration::from_millis(2));
        }
        h.run();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "<?php // é"
        );
        assert_eq!(
            h.state().service.active_document().unwrap().project_root(),
            folder
        );
        assert_eq!(std::fs::read(path).unwrap(), b"<?php // \xe9");
    }
}
