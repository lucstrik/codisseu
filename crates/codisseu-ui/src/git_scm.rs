//! Source Control actions share the existing reviewed Git worker and repository binding.
use super::git::{GitActionKind as A, GitDialog, GitIntent};
use super::*;
use codisseu_core::git::{
    CommitSelection, DiffSide, GitCommand, LocalAction, RepositorySummary, StatusEntry,
    workflows::{PullMode, WorkflowAction as W},
};
#[path = "git_scm_routing.rs"]
mod routing;
use routing::ScmFileAction;

#[derive(Default)]
pub(super) struct SourceControlUi {
    workspace: Option<PathBuf>,
    pending: Option<(PathBuf, ScmUiAction)>,
    collapsed: HashSet<PathBuf>,
    section_collapsed: bool,
    pub signoff: bool,
}
#[derive(Clone)]
enum ScmUiAction {
    Explorer {
        kind: &'static str,
        path: PathBuf,
    },
    File {
        path: PathBuf,
        side: DiffSide,
        action: ScmFileAction,
    },
    Commit(bool),
    StageAll,
    UnstageAll,
    DiscardAll,
    DiscardUntracked,
    Git(A),
    Log,
    Output,
    Sync,
    Form(ScmForm),
    Refresh,
    CommitWindow,
    GenerateMessage,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ScmForm {
    RemoteAdd,
    RemoteRename,
    RemoteUrl,
    RemoteRemove,
    Upstream,
    Coauthors,
    Sync,
    Checkout,
}
#[derive(Clone)]
pub(super) struct ScmDialog {
    pub form: ScmForm,
    pub name: String,
    pub value: String,
    pub check: bool,
}
impl SourceControlUi {
    #[cfg(test)]
    pub(super) fn has_pending_stage(&self) -> bool {
        matches!(
            self.pending.as_ref(),
            Some((
                _,
                ScmUiAction::File {
                    action: ScmFileAction::Stage,
                    ..
                }
            ))
        )
    }
    pub(super) fn has_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(super) fn cancel_pending(&mut self) {
        self.pending = None;
    }
    pub(super) fn copy_layout_from(&mut self, other: &Self) {
        self.workspace.clone_from(&other.workspace);
        self.collapsed.clone_from(&other.collapsed);
        self.section_collapsed = other.section_collapsed;
        self.signoff = other.signoff;
    }
}
impl ScmDialog {
    fn new(form: ScmForm) -> Self {
        Self {
            form,
            name: String::new(),
            value: String::new(),
            check: false,
        }
    }
}
impl CodisseuApp {
    pub(super) fn git_open_source_control(&mut self) {
        self.api_ui.open = false;
        self.docker_ui.open = false;
        self.git_ui.commit_open = true;
        self.git_ui.source_control = true;
        self.drawings_ui.open = false;
        self.show_project = true;
        self.git_discover_repositories();
    }
    pub(super) fn git_discover_repositories(&mut self) {
        self.git_discover_repositories_with_progress(false);
    }
    pub(super) fn git_discover_repositories_with_progress(&mut self, foreground: bool) {
        if let Some(workspace) = self.service.workspace_path().map(PathBuf::from) {
            if self.git_ui.scm.workspace.as_ref() != Some(&workspace) {
                self.git_ui.scm.pending = None;
                self.git_ui.scm.collapsed.clear();
                self.git_ui.scm.workspace = Some(workspace.clone());
            }
            self.git_enqueue(
                GitCommand::DiscoverRepositories {
                    workspace,
                    max_depth: 6,
                    max_repositories: 32,
                    max_entries: 50000,
                },
                foreground,
            );
        }
    }
    pub(super) fn git_poll_source_control(&mut self) {
        if self.git_ui.scm.workspace.as_deref() != self.service.workspace_path() {
            self.git_ui.scm.pending = None;
            self.git_ui.scm.collapsed.clear();
            self.git_ui.scm.workspace = None;
        }
        if self.git_ui.scm.pending.is_some()
            && !self.service.git.is_busy()
            && self.service.git.error.is_some()
        {
            self.git_ui.scm.pending = None;
        }
        if let Some((root, action)) = self.git_ui.scm.pending.clone()
            && !self.service.git.is_busy()
            && self
                .service
                .git
                .repo
                .as_ref()
                .is_some_and(|repo| repo.root == root)
        {
            self.git_ui.scm.pending = None;
            self.git_scm_action(action);
        }
    }
    fn git_scm_target(&mut self, root: &std::path::Path, action: ScmUiAction) {
        if self.service.git_repository_target() != Some(root) {
            self.sync_git_draft();
            match self
                .service
                .dispatch(Command::SelectGitRepository(root.to_owned()))
            {
                Ok(()) => self.git_bind_repository_ui(),
                Err(error) => {
                    self.git_ui.notice = error;
                    return;
                }
            }
        }
        if self.git_ui.applying || self.git_ui.preview_requested || self.git_ui.dialog.is_some() {
            self.git_ui.notice = "Conclua ou cancele a revisão Git deste repositório".into();
            return;
        }
        if self
            .service
            .git
            .repo
            .as_ref()
            .is_some_and(|r| r.root == root)
        {
            self.git_scm_action(action);
        } else {
            self.git_ui.scm.pending = Some((root.to_owned(), action));
        }
    }
    fn git_scm_action(&mut self, action: ScmUiAction) {
        match action {
            ScmUiAction::Explorer { kind, path } => {
                let Some(relative) = self
                    .service
                    .git
                    .repo
                    .as_ref()
                    .and_then(|repo| path.strip_prefix(&repo.root).ok())
                    .map(PathBuf::from)
                else {
                    return;
                };
                match kind {
                    "history" if relative.as_os_str().is_empty() => self.git_open_log(),
                    "history" => self.git_open_file_history(relative),
                    "blame" => {
                        self.send(Command::OpenFile(path));
                        let p = self.service.editor_preferences();
                        self.send(Command::SetGitInsights {
                            inline: p.git_current_line,
                            hover: p.git_hover,
                            gutter: true,
                        });
                    }
                    "diff" => self.git_open_diff(relative, DiffSide::Working),
                    "stage" => {
                        let paths: Vec<_> = self
                            .git_scm_paths(false, false)
                            .into_iter()
                            .filter(|item| item.starts_with(&relative))
                            .collect();
                        if !paths.is_empty() {
                            self.git_prepare(LocalAction::Stage { paths });
                        } else {
                            self.git_ui.notice = "Nenhuma alteração elegível nesta seleção".into();
                        }
                    }
                    _ => (),
                }
            }
            ScmUiAction::File { path, side, action } => self.git_scm_route_file(path, side, action),
            ScmUiAction::Commit(push) => self.git_scm_commit(push),
            ScmUiAction::StageAll => {
                let paths = self.git_scm_paths(false, false);
                if !paths.is_empty() {
                    self.git_prepare(LocalAction::Stage { paths });
                }
            }
            ScmUiAction::UnstageAll => {
                let paths = self.git_scm_paths(true, false);
                if !paths.is_empty() {
                    self.git_prepare(LocalAction::Unstage { paths });
                }
            }
            ScmUiAction::DiscardAll => {
                let paths = self.git_scm_paths(false, true);
                if !paths.is_empty() {
                    self.git_prepare(LocalAction::Discard { paths });
                }
            }
            ScmUiAction::DiscardUntracked => {
                let paths = self
                    .service
                    .git
                    .snapshot
                    .as_ref()
                    .map(|s| {
                        s.entries
                            .iter()
                            .filter(|e| e.untracked)
                            .map(|e| e.path.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if !paths.is_empty() {
                    self.git_prepare_scm(codisseu_core::git::scm::ScmAction::DeleteUntracked {
                        paths,
                    });
                }
            }
            ScmUiAction::Git(kind) => self.git_action(kind, String::new()),
            ScmUiAction::Log => self.git_open_log(),
            ScmUiAction::CommitWindow => self.git_open_commit(),
            ScmUiAction::GenerateMessage => {
                if let Some(root) = self.service.git_repository_target().map(PathBuf::from) {
                    self.git_generate_commit_message(root);
                }
            }
            ScmUiAction::Output => {
                self.git_open_log();
                self.git_ui.tab = 1;
            }
            ScmUiAction::Sync => {
                if !self.service.tool_window_preferences().git_confirm_actions
                    && self.git_quick_remote().is_some()
                {
                    let form = ScmDialog::new(ScmForm::Sync);
                    self.git_scm_submit(&form);
                } else {
                    self.git_ui.dialog = Some(GitDialog::Scm(ScmDialog::new(ScmForm::Sync)));
                }
            }
            ScmUiAction::Form(form) => {
                self.git_ui.dialog = Some(GitDialog::Scm(ScmDialog::new(form)))
            }
            ScmUiAction::Refresh => {
                self.git_send_foreground(GitCommand::Refresh);
                self.git_send_foreground(GitCommand::Branches);
                self.git_send_foreground(GitCommand::Remotes);
            }
        }
    }
    pub(super) fn git_explorer_target(&mut self, kind: &'static str, path: PathBuf) {
        let root = self
            .service
            .git
            .repositories
            .iter()
            .map(|repo| &repo.root)
            .chain(self.service.git.repo.iter().map(|repo| &repo.root))
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned();
        if let Some(root) = root {
            self.git_scm_target(&root, ScmUiAction::Explorer { kind, path });
        }
    }
    fn git_scm_paths(&self, index: bool, tracked_only: bool) -> Vec<PathBuf> {
        self.service
            .git
            .snapshot
            .as_ref()
            .map(|s| {
                s.entries
                    .iter()
                    .filter(|entry| {
                        !entry.conflict
                            && (!tracked_only || (!entry.untracked && entry.index != 'A'))
                            && if tracked_only {
                                !matches!(entry.index, '.' | ' ' | '?')
                                    || !matches!(entry.worktree, '.' | ' ')
                            } else if index {
                                !matches!(entry.index, '.' | ' ' | '?')
                            } else {
                                entry.untracked || !matches!(entry.worktree, '.' | ' ')
                            }
                    })
                    .map(|e| e.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    fn git_scm_commit(&mut self, push: bool) {
        if self.git_ui.message.trim().is_empty() {
            self.git_ui.notice = "Escreva uma mensagem de commit".into();
            return;
        }
        self.git_ui.selected.clear();
        self.git_ui.push_after_commit = push;
        if self.git_ui.scm.signoff {
            self.git_scm_commit_options(push);
        } else {
            self.git_prepare(LocalAction::Commit {
                selection: CommitSelection::AllStaged,
                message: self.git_ui.message.clone(),
                amend: self.git_ui.amend,
            });
        }
    }
    pub(super) fn git_source_control_panel(&mut self, ui: &mut egui::Ui) {
        self.tokens.form_style(ui);
        super::git_scroll::scm_scroll_style(ui);
        ui.spacing_mut().item_spacing.y = 0.;
        let repository_count =
            if self.service.git.discovery_workspace.as_deref() == self.service.workspace_path() {
                self.service.git.repositories.len()
                    + usize::from(self.service.git.repo.as_ref().is_some_and(|active| {
                        !self
                            .service
                            .git
                            .repositories
                            .iter()
                            .any(|repo| repo.root == active.root)
                    }))
            } else {
                usize::from(self.service.git.repo.is_some())
            };
        let has_repositories = repository_count > 0;
        header(ui, if has_repositories { 26. } else { 32. }, |ui| {
            ui.add_space(5.);
            if has_repositories {
                let (section, response) =
                    ui.allocate_exact_size(egui::vec2(78., 26.), egui::Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::accesskit::Role::Button,
                        true,
                        !self.git_ui.scm.section_collapsed,
                        "Changes section",
                    )
                });
                components::paint_icon(
                    ui,
                    egui::Rect::from_center_size(
                        section.left_center() + egui::vec2(5., 0.),
                        egui::vec2(10., 12.),
                    ),
                    if self.git_ui.scm.section_collapsed {
                        Icon::Next
                    } else {
                        Icon::Down
                    },
                    color(self.tokens.muted),
                );
                super::git_paint::text(
                    ui,
                    &mut self.git_ui.labels,
                    egui::Rect::from_min_max(section.min + egui::vec2(16., 0.), section.max),
                    "Changes",
                    color(self.tokens.text),
                    color(self.tokens.panel),
                );
                if response.clicked() {
                    self.git_ui.scm.section_collapsed = !self.git_ui.scm.section_collapsed;
                }
                let count = repository_count;
                ui.label(
                    RichText::new(format!("({count})"))
                        .small()
                        .color(color(self.tokens.muted)),
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::Label,
                        true,
                        format!("Repositories ({count})"),
                    )
                });
            } else {
                ui.add_space(7.);
                ui.label("Source Control");
            }
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 28.),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if has_repositories
                        && components::database_icon_button(
                            ui,
                            Icon::CollapseAll,
                            "Collapse All Repositories",
                            true,
                        )
                        .clicked()
                    {
                        self.git_ui.scm.collapsed.extend(
                            self.service
                                .git
                                .repositories
                                .iter()
                                .map(|repo| repo.root.clone()),
                        );
                        if let Some(repo) = &self.service.git.repo {
                            self.git_ui.scm.collapsed.insert(repo.root.clone());
                        }
                    }
                    let more = components::database_icon_button(
                        ui,
                        Icon::MoreDots,
                        "Source Control actions",
                        true,
                    );
                    egui::Popup::menu(&more).show(|ui| {
                        if ui.button("Hide Source Control").clicked() {
                            self.show_project = false;
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.service.workspace.is_some() && !self.service.git.is_busy(),
                                egui::Button::new("Refresh Repositories"),
                            )
                            .clicked()
                        {
                            self.git_discover_repositories_with_progress(true);
                            ui.close();
                        }
                        if self.service.git.discovery_workspace.as_deref()
                            == self.service.workspace_path()
                            && !self.service.git.discovery_errors.is_empty()
                        {
                            ui.menu_button(
                                format!(
                                    "Repository discovery errors ({})",
                                    self.service.git.discovery_errors.len()
                                ),
                                |ui| {
                                    ui.set_max_width(440.);
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            for error in &self.service.git.discovery_errors {
                                                ui.label(
                                                    RichText::new(error.path.display().to_string())
                                                        .strong(),
                                                );
                                                ui.add(egui::Label::new(&error.message).wrap());
                                                ui.separator();
                                            }
                                        },
                                    );
                                },
                            );
                        }
                        if ui.button("Clone Repository…").clicked() {
                            self.git_action(A::Clone, String::new());
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                self.service.workspace.is_some() && !self.service.git.is_busy(),
                                egui::Button::new("Create Git Repository…"),
                            )
                            .clicked()
                        {
                            self.git_action(A::Init, String::new());
                            ui.close();
                        }
                        if has_repositories
                            && self.service.tool_window_preferences().commit
                            && ui.button("Commit Tool Window").clicked()
                        {
                            self.git_open_commit();
                            ui.close();
                        }
                        if has_repositories && ui.button("Show Commit Graph").clicked() {
                            self.git_open_log();
                            ui.close();
                        }
                    });
                },
            );
        });
        if self.git_ui.scm.workspace.as_deref() != self.service.workspace_path()
            && !self.service.git.is_busy()
        {
            self.git_discover_repositories();
        }
        if has_repositories && self.git_ui.scm.section_collapsed {
            return;
        }
        let mut repos =
            if self.service.git.discovery_workspace.as_deref() == self.service.workspace_path() {
                self.service.git.repositories.clone()
            } else {
                Vec::new()
            };
        if let Some(repo) = &self.service.git.repo
            && !repos.iter().any(|r| r.root == repo.root)
        {
            repos.insert(
                0,
                RepositorySummary {
                    root: repo.root.clone(),
                    git_dir: repo.git_dir.clone(),
                    head: None,
                    branch: None,
                    changes: None,
                    conflicts: None,
                    ahead: None,
                    behind: None,
                    entries: None,
                    entries_truncated: true,
                },
            );
        }
        if repos.is_empty() {
            self.git_empty_repository_panel(ui);
            return;
        }
        self.git_status_line_with_progress(ui, false);

        egui::ScrollArea::vertical()
            .id_salt("scm.repositories")
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                for repo in repos {
                    ui.push_id(&repo.root, |ui| self.git_scm_repository(ui, &repo));
                }
            });
    }
    /// Empty-state layout is shared with Commit, with only applicable actions.
    pub(super) fn git_empty_repository_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.service.git.is_busy() || !self.git_ui.queue.is_empty();
        let workspace = self.service.workspace_default_root().map(PathBuf::from);
        let error = self
            .service
            .git
            .error
            .as_deref()
            .filter(|error| !expected_missing_repository(error))
            .map(str::to_owned);
        egui::ScrollArea::vertical()
            .id_salt("git.empty.workspace")
            .auto_shrink([false, false])
            .codisseu().show(ui, |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(13, 13))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 12.;
                        ui.spacing_mut().interact_size.y = 26.;
                        ui.spacing_mut().button_padding = egui::vec2(8.,4.);
                        let width = ui.available_width();
                        if busy {
                            ui.horizontal(|ui| {
                                ui.add(egui::Label::new("Looking for Git repositories…").wrap());
                            });
                        } else if workspace.is_some() {
                            ui.add(egui::Label::new(RichText::new("This folder isn't under Git yet. Initialize a repository to track changes, review differences, and commit your work.").size(13.).line_height(Some(18.2))).wrap());
                        } else {
                            ui.add(
                                egui::Label::new(
                                    "Open a folder to get started with Source Control.",
                                )
                                .wrap(),
                            );
                        }
                        if workspace.is_none() {
                            if ui
                                .add_sized(
                                    [width, 26.],
                                    self.tokens.dialog_button("Open Folder", true),
                                )
                                .clicked()
                            {
                                self.open_path(PathAction::Workspace);
                            }
                        } else if ui
                            .add_enabled(
                                !busy,
                                self.tokens
                                    .dialog_button("Initialize Repository", true)
                                    .min_size(egui::vec2(width, 26.)),
                            )
                            .clicked()
                        {
                            self.git_action(A::Init, String::new());
                        }
                        ui.add(egui::Label::new(RichText::new("You can also clone an existing repository into a separate folder.").size(13.).line_height(Some(18.2))).wrap());
                        if ui
                            .add_enabled(
                                !busy,
                                self.tokens
                                    .dialog_button("Clone Repository", false)
                                    .min_size(egui::vec2(width, 26.)),
                            )
                            .clicked()
                        {
                            self.git_action(A::Clone, String::new());
                        }
                        ui.add_enabled_ui(!busy && workspace.is_some(), |ui| {
                            if ui.link("Refresh Repositories").clicked() {
                                self.git_discover_repositories_with_progress(true);
                            }
                        });
                        if let Some(error) = &error {
                            ui.separator();

                            crate::status_messages::status_message(ui, error);
                            if ui.link("Show Git Output").clicked() {
                                self.git_scm_action(ScmUiAction::Output);
                            }
                        }
                        if !self.git_ui.notice.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&self.git_ui.notice)
                                        .small()
                                        .color(color(self.tokens.muted)),
                                )
                                .wrap(),
                            );
                        }
                    });
            });
    }

    fn git_scm_repository(&mut self, ui: &mut egui::Ui, repo: &RepositorySummary) {
        let active = self
            .service
            .git
            .repo
            .as_ref()
            .is_some_and(|r| r.root == repo.root);
        let root = &repo.root;
        let title = self
            .service
            .workspace_folder_name(root)
            .map(std::borrow::Cow::Borrowed)
            .unwrap_or_else(|| {
                self.service
                    .workspace
                    .as_ref()
                    .and_then(|workspace| root.strip_prefix(workspace).ok())
                    .filter(|relative| !relative.as_os_str().is_empty())
                    .map_or_else(
                        || root.file_name().unwrap_or_default().to_string_lossy(),
                        |relative| relative.as_os_str().to_string_lossy(),
                    )
            });
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::accesskit::Role::Button,
                true,
                active,
                format!("Repository {}", root.display()),
            )
        });
        // egui maps WidgetInfo::selected to toggled; expose the actual
        // repository selection separately for native assistive technology.
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_selected(active));
        response.clone().on_hover_text(root.display().to_string());
        let bg = color(self.tokens.panel);
        ui.painter().rect_filled(rect, 0., bg);
        let collapsed = self.git_ui.scm.collapsed.contains(root);
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                rect.left_center() + egui::vec2(9., 0.),
                egui::vec2(12., 12.),
            ),
            if collapsed { Icon::Next } else { Icon::Down },
            color(self.tokens.muted),
        );
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                rect.left_center() + egui::vec2(28., 0.),
                egui::vec2(14., 14.),
            ),
            Icon::Changes,
            color(self.tokens.muted),
        );
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            egui::Rect::from_min_max(
                rect.min + egui::vec2(39., 0.),
                rect.max - egui::vec2(102., 0.),
            ),
            &title,
            color(self.tokens.text),
            bg,
        );
        // Paint within the existing header bounds: no row insertion or spinner.
        let progress = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 18., rect.bottom() - 2.),
            egui::pos2((rect.right() - 8.).max(rect.left() + 18.), rect.bottom()),
        );
        let progress_response = ui.interact(
            progress,
            ui.scope_id().with("git.activity.line"),
            egui::Sense::hover(),
        );
        if self.git_repository_activity_busy(root) {
            progress_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::ProgressIndicator,
                    true,
                    "Atualizando Git",
                )
            });
            let accent = color(self.tokens.accent);
            ui.painter()
                .rect_filled(progress, 0., accent.gamma_multiply(0.12));
            let phase = (ui.input(|i| i.time) / 1.4).fract() as f32;
            let segment = progress.width() * 0.24;
            let x = progress.left() + (progress.width() + segment) * phase - segment;
            let moving = egui::Rect::from_min_max(
                egui::pos2(x, progress.top()),
                egui::pos2(x + segment, progress.bottom()),
            )
            .intersect(progress);
            ui.painter().rect_filled(moving, 0., accent);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(32));
            progress_response.on_hover_text(
                self.service
                    .git_repository_service(root)
                    .filter(|git| git.is_busy())
                    .map_or("Consultando Git…", |git| git.status.as_str()),
            );
        }
        let more = scm_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(16., 0.),
                egui::vec2(22., 22.),
            ),
            Icon::MoreDots,
            &format!("Repository actions {}", root.display()),
            color(self.tokens.muted),
        );
        egui::Popup::menu(&more).show(|ui| self.git_scm_menu(ui, root));
        if scm_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(40., 0.),
                egui::vec2(22., 22.),
            ),
            Icon::Refresh,
            &format!("Refresh repository {}", root.display()),
            color(self.tokens.muted),
        )
        .clicked()
        {
            self.git_scm_target(root, ScmUiAction::Refresh);
        }
        if scm_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(64., 0.),
                egui::vec2(22., 22.),
            ),
            Icon::Commit,
            &format!("Commit repository {}", root.display()),
            color(self.tokens.muted),
        )
        .clicked()
        {
            self.git_scm_target(root, ScmUiAction::Commit(false));
        }
        if scm_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(88., 0.),
                egui::vec2(22., 22.),
            ),
            Icon::Branch,
            &format!("Checkout repository {}", root.display()),
            color(self.tokens.muted),
        )
        .clicked()
        {
            self.git_scm_target(root, ScmUiAction::Form(ScmForm::Checkout));
        }
        if response.clicked() {
            if response
                .interact_pointer_pos()
                .is_some_and(|p| p.x < rect.left() + 20.)
            {
                if collapsed {
                    self.git_ui.scm.collapsed.remove(root);
                } else {
                    self.git_ui.scm.collapsed.insert(root.clone());
                }
            } else {
                self.git_scm_target(root, ScmUiAction::Refresh);
            }
        }
        if collapsed {
            return;
        }
        ui.add_space(3.);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            ui.add_space(18.);
            let mut draft = if active {
                self.git_ui.message.clone()
            } else {
                self.service.git_draft(root).unwrap_or("").to_owned()
            };
            let width = (ui.available_width() - 18.).max(32.);
            let rows = draft.lines().count().clamp(1, 6);
            let height = (ui.text_style_height(&egui::TextStyle::Body) * rows as f32 + 8.)
                .max(self.tokens.control_height);
            let field_fill = ui.painter().add(egui::Shape::Noop);
            let edit = ui.add_sized(
                [(width - 27.).max(5.), height],
                egui::TextEdit::multiline(&mut draft)
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 4)))
                    .desired_rows(rows)
                    .char_limit(65536)
                    .hint_text("Message (Ctrl+Enter to commit…)")
                    .id(egui::Id::unique(("scm.message", root))),
            );
            ui.ctx().accesskit_node_builder(edit.id, |n| {
                n.set_label(format!("Commit message {}", root.display()))
            });
            let field = egui::Rect::from_min_max(
                edit.rect.left_top(),
                edit.rect.right_bottom() + egui::vec2(27., 0.),
            );
            ui.painter().set(
                field_fill,
                egui::Shape::rect_filled(field, 4., color(self.tokens.editor)),
            );
            ui.painter().rect_stroke(
                field,
                4.,
                egui::Stroke::new(
                    1.,
                    color(if edit.has_focus() {
                        self.tokens.accent
                    } else {
                        self.tokens.control_border
                    }),
                ),
                egui::StrokeKind::Inside,
            );
            // The generation request is bound to this repository and never
            // commits or stages changes. A second click can cancel it.
            let button = egui::Rect::from_min_size(
                egui::pos2(edit.rect.right(), edit.rect.top() + 1.),
                egui::vec2(26., self.tokens.control_height - 2.),
            );
            if scm_draft_icon(
                ui,
                button,
                &format!(
                    "{} {}",
                    if self.service.git_commit_messages.is_running(root) {
                        "Cancel commit message generation"
                    } else {
                        "Generate commit message"
                    },
                    root.display()
                ),
                color(self.tokens.muted),
                self.service.git_commit_messages.is_running(root),
            )
            .clicked()
            {
                self.git_scm_target(root, ScmUiAction::GenerateMessage);
            }
            ui.advance_cursor_after_rect(field);
            if edit.clicked() && !active {
                self.git_scm_target(root, ScmUiAction::Refresh);
            }
            if edit.changed() {
                if self.service.git_repository_target() == Some(root.as_path()) {
                    self.git_ui.message = draft.clone();
                } else if let Some(view) = self.git_repositories.views.get_mut(root) {
                    view.message = draft.clone();
                }
                if let Err(e) = self.service.dispatch(Command::SetGitDraft {
                    root: root.clone(),
                    message: draft,
                }) {
                    self.git_ui.notice = e;
                }
            }
            if edit.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter)) {
                self.git_scm_target(root, ScmUiAction::Commit(false));
            }
        });
        if let Some(notice) = self.git_commit_ai_notice(root) {
            ui.add(
                egui::Label::new(
                    RichText::new(notice)
                        .small()
                        .color(color(self.tokens.muted)),
                )
                .wrap(),
            );
        }
        ui.add_space(8.);
        let (ahead, behind) = if active {
            self.service
                .git
                .snapshot
                .as_ref()
                .map(|s| (Some(s.ahead), Some(s.behind)))
                .unwrap_or((None, None))
        } else {
            (repo.ahead, repo.behind)
        };
        let staged = self
            .service
            .git_repository_service(root)
            .and_then(|git| git.snapshot.as_ref())
            .map(|s| s.entries.as_slice())
            .or_else(|| repo.entries.as_deref().map(Vec::as_slice))
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|e| !matches!(e.index, '.' | ' ' | '?') && !e.conflict)
            });
        let accessible_label = if staged {
            "Commit Staged Changes".to_owned()
        } else if ahead.is_some_and(|n| n > 0) || behind.is_some_and(|n| n > 0) {
            format!(
                "Sync Changes {}↓ {}↑",
                behind.unwrap_or(0),
                ahead.unwrap_or(0)
            )
        } else {
            "Commit".into()
        };
        let ready = !self.git_repository_controls_busy(root)
            && (!active || staged || ahead.is_some_and(|n| n > 0) || behind.is_some_and(|n| n > 0));
        let (whole, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.), egui::Sense::hover());
        let button_rect = whole.shrink2(egui::vec2(18., 0.));
        let options_rect = egui::Rect::from_min_max(
            egui::pos2(button_rect.right() - 28., button_rect.top()),
            button_rect.right_bottom(),
        );
        let commit_rect = egui::Rect::from_min_max(
            button_rect.left_top(),
            egui::pos2(options_rect.left(), button_rect.bottom()),
        );
        let fill = color(self.tokens.accent).gamma_multiply(if ready { 1. } else { 0.45 });
        ui.painter().rect_filled(button_rect, 4., fill);
        let commit = ui.interact(
            commit_rect,
            egui::Id::unique(("scm.commit", root)),
            if ready {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        commit.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ready, &accessible_label)
        });
        let label = if !staged && (ahead.is_some_and(|n| n > 0) || behind.is_some_and(|n| n > 0)) {
            "Sync Changes"
        } else {
            "Commit"
        };
        let text_width = ui
            .painter()
            .layout_no_wrap(label.into(), FontId::proportional(13.), Color32::WHITE)
            .size()
            .x;
        let origin = egui::pos2(
            commit_rect.center().x - (text_width + 20.) / 2.,
            commit_rect.center().y,
        );
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(origin + egui::vec2(7., 0.), egui::vec2(14., 14.)),
            if label == "Commit" {
                Icon::Commit
            } else {
                Icon::Refresh
            },
            Color32::WHITE,
        );
        ui.painter().text(
            origin + egui::vec2(20., 0.),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.),
            Color32::WHITE,
        );
        ui.painter().vline(
            options_rect.left(),
            egui::Rangef::new(button_rect.top() + 5., button_rect.bottom() - 5.),
            egui::Stroke::new(1., Color32::from_white_alpha(100)),
        );
        if commit.clicked() {
            self.git_scm_target(
                root,
                if staged || !active {
                    ScmUiAction::Commit(false)
                } else {
                    ScmUiAction::Sync
                },
            );
        }
        let options = scm_icon(
            ui,
            options_rect,
            Icon::Down,
            &format!("Commit options {}", root.display()),
            Color32::WHITE,
        );
        egui::Popup::menu(&options).show(|ui| self.git_scm_commit_menu(ui, root));
        ui.add_space(4.);
        let entries = if active && repo.entries.is_none() {
            self.service
                .git
                .snapshot
                .as_ref()
                .map(|s| s.entries.clone())
        } else {
            None
        };
        if let Some(entries) = entries
            .as_deref()
            .or_else(|| repo.entries.as_ref().map(|entries| entries.as_slice()))
        {
            if entries.is_empty() {
                let unborn = if active {
                    self.service
                        .git
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.head.is_none())
                } else {
                    repo.head.is_none()
                };
                ui.horizontal(|ui| {
                    ui.add_space(18.);
                    ui.add(
                        egui::Label::new(
                            RichText::new(if unborn {
                                "No commits yet · add files to start"
                            } else {
                                "No changes"
                            })
                            .small()
                            .color(color(self.tokens.muted)),
                        )
                        .wrap(),
                    );
                });
            }
            self.git_scm_changes(ui, root, entries);
        } else {
            ui.horizontal(|ui| {
                ui.add_space(18.);
                ui.label(
                    RichText::new("Select to load changes")
                        .small()
                        .color(color(self.tokens.muted)),
                );
            });
        }
        // A full active snapshot is authoritative; the synthetic/cached repository
        // summary can still carry a partial-cache flag from before selection.
        if repo.entries_truncated && !(active && self.service.git.snapshot.is_some()) {
            ui.horizontal(|ui| {
                ui.add_space(18.);
                ui.label(
                    RichText::new("Partial file list · select to refresh")
                        .small()
                        .color(color(self.tokens.muted)),
                );
            });
        }
        ui.add_space(4.);
    }
    fn git_scm_menu(&mut self, ui: &mut egui::Ui, root: &std::path::Path) {
        self.tokens.menu_style(ui.style_mut(), false);
        ui.set_min_width(292.);
        for (name, action) in [
            ("Stash All Changes…", ScmUiAction::Git(A::Stash)),
            ("Show Commit Graph", ScmUiAction::Log),
            ("Commit", ScmUiAction::Commit(false)),
            ("Refresh", ScmUiAction::Refresh),
        ] {
            if ui.button(name).clicked() {
                self.git_scm_target(root, action);
                ui.close();
            }
        }
        ui.separator();
        for (name, action) in [
            ("Pull", ScmUiAction::Git(A::Pull)),
            ("Push", ScmUiAction::Git(A::Push)),
            ("Clone…", ScmUiAction::Git(A::Clone)),
            ("Checkout to…", ScmUiAction::Form(ScmForm::Checkout)),
            ("Fetch…", ScmUiAction::Git(A::Fetch)),
            ("Sync Changes…", ScmUiAction::Sync),
        ] {
            if ui.button(name).clicked() {
                self.git_scm_target(root, action);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Compose Commits…").clicked() {
            self.git_scm_target(root, ScmUiAction::CommitWindow);
            ui.close();
        }
        if ui
            .button(if self.service.git_commit_messages.is_running(root) {
                "Cancel Commit Message Generation"
            } else {
                "Generate Commit Message"
            })
            .clicked()
        {
            self.git_scm_target(root, ScmUiAction::GenerateMessage);
            ui.close();
        }
        if ui.button("Add Co-authors…").clicked() {
            self.git_scm_target(root, ScmUiAction::Form(ScmForm::Coauthors));
            ui.close();
        }
        ui.separator();
        ui.menu_button("Commit", |ui| self.git_scm_commit_menu(ui, root));
        ui.menu_button("Changes", |ui| {
            for (label, action) in [
                ("Stage All Changes", ScmUiAction::StageAll),
                ("Unstage All Changes", ScmUiAction::UnstageAll),
                ("Discard All Tracked Changes…", ScmUiAction::DiscardAll),
                ("Discard Untracked Files…", ScmUiAction::DiscardUntracked),
            ] {
                if ui.button(label).clicked() {
                    self.git_scm_target(root, action);
                    ui.close();
                }
            }
            if ui.button("Show Working Tree Changes").clicked() {
                self.git_scm_target(root, ScmUiAction::CommitWindow);
                ui.close();
            }
        });
        ui.menu_button("Pull, Push", |ui| {
            for (label, action) in [
                ("Pull…", ScmUiAction::Git(A::Pull)),
                ("Push…", ScmUiAction::Git(A::Push)),
                ("Fetch…", ScmUiAction::Git(A::Fetch)),
                ("Sync Changes…", ScmUiAction::Sync),
                ("Set Upstream…", ScmUiAction::Form(ScmForm::Upstream)),
            ] {
                if ui.button(label).clicked() {
                    self.git_scm_target(root, action);
                    ui.close();
                }
            }
        });
        ui.menu_button("Branch", |ui| {
            for (label, action) in [
                ("Create Branch…", ScmUiAction::Git(A::NewBranch)),
                ("Checkout to…", ScmUiAction::Form(ScmForm::Checkout)),
                ("Rename Branch…", ScmUiAction::Git(A::RenameBranch)),
                ("Delete Branch…", ScmUiAction::Git(A::DeleteBranch)),
                ("Merge Branch…", ScmUiAction::Git(A::Merge)),
                ("Rebase Branch…", ScmUiAction::Git(A::Rebase)),
            ] {
                if ui.button(label).clicked() {
                    self.git_scm_target(root, action);
                    ui.close();
                }
            }
        });
        ui.menu_button("Remote", |ui| {
            for (label, form) in [
                ("Add Remote…", ScmForm::RemoteAdd),
                ("Rename Remote…", ScmForm::RemoteRename),
                ("Change Remote URL…", ScmForm::RemoteUrl),
                ("Remove Remote…", ScmForm::RemoteRemove),
            ] {
                if ui.button(label).clicked() {
                    self.git_scm_target(root, ScmUiAction::Form(form));
                    ui.close();
                }
            }
        });
        ui.menu_button("Stash", |ui| {
            if ui.button("Stash Changes…").clicked() {
                self.git_scm_target(root, ScmUiAction::Git(A::Stash));
                ui.close();
            }
            if ui.button("Apply, Pop or Drop Stash…").clicked() {
                self.git_scm_target(root, ScmUiAction::Git(A::Stashes));
                ui.close();
            }
            if ui.button("Shelve Changes…").clicked() {
                self.git_scm_target(root, ScmUiAction::CommitWindow);
                ui.close();
            }
        });
        ui.menu_button("Tags", |ui| {
            if ui.button("Create Tag…").clicked() {
                self.git_scm_target(root, ScmUiAction::Git(A::NewTag));
                ui.close();
            }
            if ui.button("Delete Tag…").clicked() {
                self.git_scm_target(root, ScmUiAction::Git(A::DeleteTag));
                ui.close();
            }
        });
        ui.separator();
        if ui.button("Show Commit Graph").clicked() {
            self.git_scm_target(root, ScmUiAction::Log);
            ui.close();
        }
        if ui.button("Visualize Repository History").clicked() {
            self.git_scm_target(root, ScmUiAction::Log);
            ui.close();
        }
        ui.separator();
        if ui.button("Show Git Output").clicked() {
            self.git_scm_target(root, ScmUiAction::Output);
            ui.close();
        }
    }
    fn git_scm_commit_menu(&mut self, ui: &mut egui::Ui, root: &std::path::Path) {
        for (label, push) in [
            ("Commit Staged Changes", false),
            ("Commit Staged Changes and Push…", true),
        ] {
            if ui.button(label).clicked() {
                self.git_scm_target(root, ScmUiAction::Commit(push));
                ui.close();
            }
        }
        if ui.button("Commit Selected Changes…").clicked() {
            self.git_scm_target(root, ScmUiAction::CommitWindow);
            ui.close();
        }
        if ui.button("Stage All Changes").clicked() {
            self.git_scm_target(root, ScmUiAction::StageAll);
            ui.close();
        }
        ui.separator();
        let active = self
            .service
            .git
            .repo
            .as_ref()
            .is_some_and(|repo| repo.root == root);
        ui.add_enabled_ui(active, |ui| {
            ui.checkbox(&mut self.git_ui.amend, "Amend Last Commit");
            ui.checkbox(&mut self.git_ui.scm.signoff, "Add Signed-off-by");
        });
        if ui.button("Add Co-authors…").clicked() {
            self.git_scm_target(root, ScmUiAction::Form(ScmForm::Coauthors));
            ui.close();
        }
    }
    fn git_scm_changes(
        &mut self,
        ui: &mut egui::Ui,
        root: &std::path::Path,
        entries: &[StatusEntry],
    ) {
        let groups = [
            (
                "Merge Changes",
                entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.conflict)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>(),
                DiffSide::Working,
            ),
            (
                "Staged Changes",
                entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.conflict && !matches!(e.index, '.' | ' ' | '?'))
                    .map(|(i, _)| i)
                    .collect(),
                DiffSide::Index,
            ),
            (
                "Changes",
                entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        !e.conflict && (e.untracked || !matches!(e.worktree, '.' | ' '))
                    })
                    .map(|(i, _)| i)
                    .collect(),
                DiffSide::Working,
            ),
        ];
        for (label, indices, side) in groups {
            if indices.is_empty() {
                continue;
            }
            let key = format!("scm.group.{}.{label}", root.display());
            let open = *self.git_ui.groups.get(&key).unwrap_or(&true);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    format!("{label} ({})", indices.len()),
                )
            });
            components::paint_icon(
                ui,
                egui::Rect::from_center_size(
                    rect.left_center() + egui::vec2(16., 0.),
                    egui::vec2(12., 12.),
                ),
                if open { Icon::Down } else { Icon::Next },
                color(self.tokens.muted),
            );
            super::git_paint::text(
                ui,
                &mut self.git_ui.labels,
                egui::Rect::from_min_max(
                    rect.min + egui::vec2(27., 0.),
                    rect.max - egui::vec2(76., 0.),
                ),
                label,
                color(self.tokens.text),
                color(self.tokens.panel),
            );
            scm_badge(
                ui,
                egui::pos2(rect.right() - 18., rect.center().y),
                indices.len(),
                color(self.tokens.accent),
            );
            let show = response.hovered() || response.has_focus() || ui.rect_contains_pointer(rect);
            let stage = scm_hover_icon(
                ui,
                egui::Rect::from_center_size(
                    rect.right_center() - egui::vec2(45., 0.),
                    egui::vec2(22., 22.),
                ),
                if side == DiffSide::Index {
                    Icon::Remove
                } else {
                    Icon::Add
                },
                if side == DiffSide::Index {
                    "Unstage All Changes"
                } else {
                    "Stage All Changes"
                },
                color(self.tokens.muted),
                (show, label != "Merge Changes"),
            );
            if stage.clicked() {
                self.git_scm_target(
                    root,
                    if side == DiffSide::Index {
                        ScmUiAction::UnstageAll
                    } else {
                        ScmUiAction::StageAll
                    },
                );
            }
            let discard = scm_hover_icon(
                ui,
                egui::Rect::from_center_size(
                    rect.right_center() - egui::vec2(69., 0.),
                    egui::vec2(22., 22.),
                ),
                Icon::Revert,
                "Discard All Tracked Changes…",
                color(self.tokens.muted),
                (show, side == DiffSide::Working && label != "Merge Changes"),
            );
            if discard.clicked() {
                self.git_scm_target(root, ScmUiAction::DiscardAll);
            }
            if response.clicked() {
                self.git_ui.groups.insert(key, !open);
            }
            if open {
                // The outer repository list owns scrolling. Reserve the complete
                // group extent, but build widgets only for its clipped viewport.
                let (group, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), indices.len() as f32 * 22.),
                    egui::Sense::hover(),
                );
                let visible = ui.clip_rect().intersect(group);
                if visible.is_positive() {
                    let first = (((visible.top() - group.top()) / 22.).floor().max(0.) as usize)
                        .min(indices.len());
                    let end = ((((visible.bottom() - group.top()) / 22.).ceil().max(0.) as usize)
                        + 1)
                    .min(indices.len());
                    let mut rows = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(("scm.files", root, label))
                            .max_rect(egui::Rect::from_min_max(
                                group.min + egui::vec2(0., first as f32 * 22.),
                                group.right_bottom(),
                            )),
                    );
                    rows.set_clip_rect(ui.clip_rect().intersect(group));
                    rows.spacing_mut().item_spacing.y = 0.;
                    for (offset, entry_index) in indices[first..end].iter().enumerate() {
                        if let Some(entry) = entries.get(*entry_index) {
                            rows.push_id((root, label, first + offset), |ui| {
                                self.git_scm_file(ui, root, entry, side)
                            });
                        }
                    }
                }
            }
        }
    }
    fn git_scm_file(
        &mut self,
        ui: &mut egui::Ui,
        root: &std::path::Path,
        entry: &StatusEntry,
        side: DiffSide,
    ) {
        let (rect, row) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.), egui::Sense::click());
        row.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                format!(
                    "SCM file {} {}",
                    if side == DiffSide::Index {
                        "staged"
                    } else {
                        "working"
                    },
                    if self
                        .service
                        .git
                        .repo
                        .as_ref()
                        .is_some_and(|r| r.root == root)
                    {
                        entry.path.display().to_string()
                    } else {
                        format!("{} in {}", entry.path.display(), root.display())
                    }
                ),
            )
        });
        let active = self
            .service
            .git
            .repo
            .as_ref()
            .is_some_and(|r| r.root == root);
        let selected = active && self.git_ui.file.as_ref() == Some(&entry.path);
        let bg = if selected {
            color(self.tokens.database_services_selection)
        } else {
            color(self.tokens.panel)
        };
        ui.painter().rect_filled(rect, 0., bg);
        let show = row.hovered() || row.has_focus() || ui.rect_contains_pointer(rect);
        components::paint_document_icon(
            ui,
            egui::Rect::from_min_size(rect.min + egui::vec2(19., 3.), egui::vec2(16., 16.)),
            entry.path.to_str().unwrap_or_default(),
            color(self.tokens.accent),
        );
        let status = if entry.conflict {
            'C'
        } else if entry.untracked {
            'U'
        } else if side == DiffSide::Index {
            entry.index
        } else {
            entry.worktree
        };
        let tint = if status == 'R' {
            color(self.tokens.git_added)
        } else {
            super::git_commit::git_file_color(entry, &self.tokens)
        };
        let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
        let name_width = ui
            .painter()
            .layout_no_wrap(name.to_string(), FontId::proportional(13.), tint)
            .size()
            .x;
        let right = rect.right() - if show { 76. } else { 26. };
        let start = rect.left() + 38.;
        let end = (start + name_width + 1.).min(right);
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            egui::Rect::from_min_max(
                egui::pos2(start, rect.top()),
                egui::pos2(end, rect.bottom()),
            ),
            &name,
            tint,
            bg,
        );
        let repo = root.file_name().unwrap_or_default().to_string_lossy();
        let parent = entry.path.parent().filter(|p| !p.as_os_str().is_empty());
        let detail = if let Some(parent) = parent {
            format!("{repo} • {}", parent.display())
        } else {
            repo.to_string()
        };
        let detail_rect = egui::Rect::from_min_max(
            egui::pos2(end + 6., rect.top()),
            egui::pos2(right, rect.bottom()),
        );
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            detail_rect,
            &detail,
            color(self.tokens.muted),
            bg,
        );
        if status == 'D' {
            ui.painter().line_segment(
                [
                    egui::pos2(start, rect.center().y),
                    egui::pos2(
                        right.min(
                            end + 6.
                                + ui.painter()
                                    .layout_no_wrap(
                                        detail,
                                        FontId::proportional(13.),
                                        color(self.tokens.muted),
                                    )
                                    .size()
                                    .x,
                        ),
                        rect.center().y,
                    ),
                ],
                egui::Stroke::new(1., color(self.tokens.muted)),
            );
        }
        ui.painter().text(
            egui::pos2(rect.right() - 14., rect.center().y),
            egui::Align2::CENTER_CENTER,
            status,
            FontId::proportional(12.),
            tint,
        );
        let open = scm_hover_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(61., 0.),
                egui::vec2(22., 22.),
            ),
            Icon::OpenFile,
            &format!(
                "Open File {}{}{}",
                if side == DiffSide::Index {
                    "staged "
                } else {
                    ""
                },
                entry.path.display(),
                if active {
                    String::new()
                } else {
                    format!(" in {}", root.display())
                }
            ),
            color(self.tokens.muted),
            (
                show,
                entry.worktree != 'D' && !(entry.index == 'D' && entry.worktree == '.'),
            ),
        );
        let mut action_clicked = open.clicked();
        if action_clicked {
            self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::OpenFile);
        }
        if !entry.conflict {
            let action = scm_hover_icon(
                ui,
                egui::Rect::from_center_size(
                    rect.right_center() - egui::vec2(39., 0.),
                    egui::vec2(22., 22.),
                ),
                if side == DiffSide::Index {
                    Icon::Remove
                } else {
                    Icon::Add
                },
                &format!(
                    "{} {}",
                    if side == DiffSide::Index {
                        "Unstage"
                    } else {
                        "Stage"
                    },
                    if active {
                        entry.path.display().to_string()
                    } else {
                        format!("{} in {}", entry.path.display(), root.display())
                    }
                ),
                color(self.tokens.muted),
                (show, true),
            );
            if action.clicked() {
                action_clicked = true;
                self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::Stage);
            }
        }
        if row.clicked() && !action_clicked {
            self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::Select);
            row.request_focus();
        }
        if row.double_clicked() && !action_clicked {
            self.git_scm_file_target(
                root,
                entry.path.clone(),
                side,
                if entry.conflict {
                    ScmFileAction::Conflict
                } else {
                    ScmFileAction::Diff
                },
            );
        }
        row.context_menu(|ui| {
            for (label, action) in [
                ("Open File", ScmFileAction::OpenFile),
                ("Open Changes", ScmFileAction::Diff),
            ] {
                if ui.button(label).clicked() {
                    self.git_scm_file_target(root, entry.path.clone(), side, action);
                    ui.close();
                }
            }
            if !entry.conflict
                && ui
                    .button(if side == DiffSide::Index {
                        "Unstage Changes"
                    } else {
                        "Stage Changes"
                    })
                    .clicked()
            {
                self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::Stage);
                ui.close();
            }
            if !entry.conflict && entry.index != 'A' && ui.button("Discard Changes…").clicked() {
                self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::Discard);
                ui.close();
            }
            if entry.conflict && ui.button("Resolve Conflicts…").clicked() {
                self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::Conflict);
                ui.close();
            }
            if ui.button("File History").clicked() {
                self.git_scm_file_target(root, entry.path.clone(), side, ScmFileAction::History);
                ui.close();
            }
        });
    }
    fn git_prepare_scm(&mut self, action: codisseu_core::git::scm::ScmAction) {
        if !self.git_begin_intent(GitIntent::Other) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        self.git_send(GitCommand::ScmPrepare(action));
    }
    fn git_scm_commit_options(&mut self, push: bool) {
        let message = self.git_ui.message.clone();
        if !self.git_begin_intent(GitIntent::Commit {
            message: message.clone(),
            push,
        }) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        self.git_send(GitCommand::ScmPrepare(
            codisseu_core::git::scm::ScmAction::CommitOptions {
                selection: CommitSelection::AllStaged,
                message,
                amend: self.git_ui.amend,
                signoff: self.git_ui.scm.signoff,
                coauthors: Vec::new(),
            },
        ));
    }
    pub(super) fn git_scm_window(&mut self, ctx: &egui::Context, dialog: GitDialog) {
        let GitDialog::Scm(mut form) = dialog else {
            return;
        };
        let preparing = self.git_ui.preview_requested;
        let mut close = false;
        if form.name.is_empty()
            && matches!(
                form.form,
                ScmForm::RemoteRename
                    | ScmForm::RemoteUrl
                    | ScmForm::RemoteRemove
                    | ScmForm::Upstream
            )
        {
            form.name = self
                .service
                .git
                .remotes
                .first()
                .map(|r| r.name.clone())
                .unwrap_or_default();
        }
        let title = match form.form {
            ScmForm::RemoteAdd => "Add Remote",
            ScmForm::RemoteRename => "Rename Remote",
            ScmForm::RemoteUrl => "Change Remote URL",
            ScmForm::RemoteRemove => "Remove Remote",
            ScmForm::Upstream => "Set Upstream Branch",
            ScmForm::Coauthors => "Add Co-authors",
            ScmForm::Sync => "Sync Changes",
            ScmForm::Checkout => "Checkout to",
        };
        egui::Modal::new(egui::Id::unique("git.scm.dialog"))
            .frame(egui::Frame::window(&ctx.style_of(egui::Theme::Dark)).inner_margin(12))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width(500f32.min(ctx.content_rect().width() - 64.));
                ui.heading(title);
                ui.add_space(12.);
                let mut submit = false;
                match form.form {
                    ScmForm::RemoteAdd | ScmForm::RemoteRename | ScmForm::RemoteUrl
                    | ScmForm::RemoteRemove | ScmForm::Upstream => {
                        if form.form == ScmForm::RemoteAdd {
                            scm_field(ui, "Remote name", &mut form.name);
                        } else {
                            let label = ui.label("Remote:");
                            egui::ComboBox::from_id_salt("scm.remote")
                                .selected_text(&form.name)
                                .show_ui(ui, |ui| {
                                    for remote in &self.service.git.remotes {
                                        ui.selectable_value(&mut form.name, remote.name.clone(), &remote.name);
                                    }
                                }).response.labelled_by(label.id);
                        }
                        match form.form {
                            ScmForm::RemoteAdd | ScmForm::RemoteUrl => {
                                scm_field(ui, "Remote URL", &mut form.value);
                                if form.form == ScmForm::RemoteUrl {
                                    ui.checkbox(&mut form.check, "Push URL");
                                }
                            }
                            ScmForm::RemoteRename => scm_field(ui, "New remote name", &mut form.value),
                            ScmForm::RemoteRemove => {
                                ui.label("This removes the local remote configuration. It does not delete the server repository.");
                            }
                            ScmForm::Upstream => {
                                scm_field(ui, "Remote branch", &mut form.value);
                                ui.checkbox(&mut form.check, "Remove upstream instead");
                            }
                            _ => {}
                        }
                    }
                    ScmForm::Coauthors => {
                        ui.label("One Name <email> per line. Adds explicit Co-authored-by trailers to this draft.");
                        let label = ui.label("Co-authors");
                        ui.add_sized([ui.available_width(), 130.], egui::TextEdit::multiline(&mut form.value).char_limit(32768)).labelled_by(label.id);
                    }
                    ScmForm::Sync => {
                        ui.label("Pull first, then review outgoing commits in the Push window. Publishing requires a separate confirmation.");
                        ui.checkbox(&mut form.check, "Rebase instead of fast-forward only");
                    }
                    ScmForm::Checkout => {
                        scm_field(ui, "Branch or revision", &mut form.value);
                        egui::ScrollArea::vertical().max_height(220.).codisseu().show(ui, |ui| {
                            for branch in self.service.git.branches.clone() {
                                if ui.selectable_label(form.value == branch.name, &branch.name).clicked() {
                                    form.value = if branch.remote { format!("refs/remotes/{}", branch.name) } else { branch.name };
                                }
                            }
                        });
                    }
                }
                ui.add_space(12.);
                let use_message = matches!(form.form, ScmForm::Coauthors);
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 28.), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close |= ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                    submit = ui.add_enabled(!self.service.git.is_busy(), self.tokens.dialog_button(if use_message { "Use Message" } else { "Review" }, true)).clicked();
                });
                if submit { close = self.git_scm_submit(&form); }
                self.git_status_line(ui);
            });
        if close && !preparing && self.git_ui.preview_requested {
            self.git_ui.operation_draft = Some(Box::new(GitDialog::Scm(form)));
        } else if !close {
            self.git_ui.dialog = Some(GitDialog::Scm(form));
        }
    }
    fn git_scm_submit(&mut self, form: &ScmDialog) -> bool {
        use codisseu_core::git::scm::ScmAction as S;
        match form.form {
            ScmForm::RemoteAdd => self.git_prepare_scm(S::RemoteAdd {
                name: form.name.clone(),
                url: form.value.clone(),
            }),
            ScmForm::RemoteRename => self.git_prepare_scm(S::RemoteRename {
                name: form.name.clone(),
                new_name: form.value.clone(),
            }),
            ScmForm::RemoteUrl => self.git_prepare_scm(S::RemoteSetUrl {
                name: form.name.clone(),
                url: form.value.clone(),
                push: form.check,
            }),
            ScmForm::RemoteRemove => self.git_prepare_scm(S::RemoteRemove {
                name: form.name.clone(),
            }),
            ScmForm::Upstream => {
                let branch = self
                    .service
                    .git
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.head_ref.as_ref())
                    .cloned()
                    .unwrap_or_default();
                self.git_prepare_scm(S::SetUpstream {
                    branch,
                    upstream: if form.check {
                        None
                    } else {
                        Some(format!("refs/remotes/{}/{}", form.name, form.value))
                    },
                });
            }
            ScmForm::Coauthors => match with_coauthors(&self.git_ui.message, &form.value) {
                Ok(message) => {
                    self.git_ui.message = message;
                    self.sync_git_draft();
                    return true;
                }
                Err(e) => {
                    self.git_ui.notice = e;
                    return false;
                }
            },
            ScmForm::Checkout => {
                if self
                    .service
                    .git
                    .branches
                    .iter()
                    .any(|b| !b.remote && b.name == form.value)
                {
                    self.git_prepare(LocalAction::CheckoutBranch {
                        name: form.value.clone(),
                    });
                } else {
                    self.git_prepare(LocalAction::CheckoutRevision {
                        revision: form.value.clone(),
                    });
                }
            }
            ScmForm::Sync => {
                let Some(root) = self.service.git.repo.as_ref().map(|r| r.root.clone()) else {
                    return false;
                };
                let Some(snapshot) = &self.service.git.snapshot else {
                    return false;
                };
                let Some(upstream) = snapshot.upstream.as_ref() else {
                    self.git_ui.notice = "Configure an upstream before syncing".into();
                    return false;
                };
                let Some((remote, branch)) = upstream.split_once('/') else {
                    self.git_ui.notice = "Upstream is not a remote branch".into();
                    return false;
                };
                let action = W::Pull {
                    remote: remote.into(),
                    branch: branch.into(),
                    mode: if form.check {
                        PullMode::Rebase
                    } else {
                        PullMode::FastForwardOnly
                    },
                };
                if !self.git_begin_intent(GitIntent::SyncPull { root }) {
                    return false;
                }
                if !self.service.git.trusted {
                    self.git_send(GitCommand::Trust(true));
                }
                self.git_send(GitCommand::WorkflowPrepare(action));
            }
        }
        self.git_ui.preview_requested
    }
}
fn scm_icon(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Icon,
    label: &str,
    tint: Color32,
) -> egui::Response {
    let response = ui.interact(rect, ui.make_persistent_id(label), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 2., ui.visuals().widgets.hovered.weak_bg_fill);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
    });
    components::paint_icon(
        ui,
        egui::Rect::from_center_size(rect.center(), egui::vec2(16., 16.)),
        icon,
        tint,
    );
    response.on_hover_text(label)
}
fn scm_hover_icon(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Icon,
    label: &str,
    tint: Color32,
    visibility: (bool, bool),
) -> egui::Response {
    let (visible, enabled) = visibility;
    let response = ui.interact(
        rect,
        if enabled {
            ui.make_persistent_id((label, true))
        } else {
            ui.make_persistent_id((label, false, rect.center().y.to_bits()))
        },
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, label));
    if visible || response.hovered() || response.has_focus() {
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(16., 16.)),
            icon,
            if enabled {
                tint
            } else {
                tint.gamma_multiply(0.4)
            },
        );
    }
    response.on_hover_text(label)
}
fn scm_badge(ui: &egui::Ui, center: egui::Pos2, count: usize, fill: Color32) {
    let label = count.to_string();
    let width = (label.len() as f32 * 7. + 8.).max(18.);
    ui.painter().rect_filled(
        egui::Rect::from_center_size(center, egui::vec2(width, 18.)),
        9.,
        fill,
    );
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.),
        Color32::WHITE,
    );
}
fn scm_draft_icon(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    tint: Color32,
    generating: bool,
) -> egui::Response {
    let response = ui.interact(rect, ui.make_persistent_id(label), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
    if generating {
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(rect.center(), egui::vec2(14., 14.)),
            Icon::Stop,
            tint,
        );
        return response.on_hover_text("Cancelar geração da mensagem de commit");
    }
    let center = rect.center();
    for (offset, size) in [(egui::vec2(-2., 2.), 4.), (egui::vec2(3., -4.), 2.5)] {
        let c = center + offset;
        ui.painter().add(egui::Shape::closed_line(
            vec![
                c + egui::vec2(0., -size),
                c + egui::vec2(size, 0.),
                c + egui::vec2(0., size),
                c + egui::vec2(-size, 0.),
            ],
            egui::Stroke::new(1., tint),
        ));
    }
    response.on_hover_text("Gerar mensagem de commit com IA")
}
fn scm_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let caption = ui.label(label);
    ui.add_sized(
        [ui.available_width(), 28.],
        egui::TextEdit::singleline(value),
    )
    .labelled_by(caption.id);
}
fn with_coauthors(message: &str, authors: &str) -> Result<String, String> {
    let mut trailers = Vec::new();
    for author in authors.lines().filter(|s| !s.trim().is_empty()) {
        let author = author.trim();
        let Some((name, email)) = author.rsplit_once('<') else {
            return Err("Use Name <email> for every co-author".into());
        };
        let email = email
            .strip_suffix('>')
            .ok_or("Use Name <email> for every co-author")?;
        if name.trim().is_empty()
            || !email.contains('@')
            || email
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>'))
            || author.len() > 1024
            || trailers.len() >= 32
        {
            return Err("Invalid co-author identity or more than32 authors".into());
        }
        let trailer = format!("Co-authored-by: {} <{}>", name.trim(), email);
        if !message.lines().any(|line| line == trailer) && !trailers.contains(&trailer) {
            trailers.push(trailer);
        }
    }
    let mut result = message.trim_end().to_owned();
    if !trailers.is_empty() {
        result.push_str("\n\n");
        result.push_str(&trailers.join("\n"));
    }
    if result.len() > 65536 {
        return Err("Commit message exceeds64KiB".into());
    }
    Ok(result)
}

// A missing .git is an ordinary empty-workspace state. Other failures remain visible.
pub(super) fn expected_missing_repository(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    ![
        "permission denied",
        "access denied",
        "permissão",
        "timeout",
        "timed out",
    ]
    .iter()
    .any(|detail| error.contains(detail))
        && (error.contains("not a git repository") || error.contains("não é um repositório git"))
}

#[cfg(test)]
mod empty_workspace_tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::{
        path::Path,
        thread,
        time::{Duration, Instant},
    };

    fn cli(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn settle(app: &mut AppService) {
        let until = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
    }
    fn fixture(repository: u8, commit: bool) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("empty-project");
        std::fs::create_dir(&root).unwrap();
        if repository > 0 {
            cli(&root, &["init", "-b", "main"]);
        }
        if repository > 1 {
            std::fs::write(root.join("hello.txt"), "hello\n").unwrap();
            cli(&root, &["add", "hello.txt"]);
            cli(
                &root,
                &[
                    "-c",
                    "user.name=Codisseu Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "-m",
                    "Initial fixture",
                ],
            );
        }
        let mut service = AppService::new(dir.path().join("session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        settle(&mut service);
        service
            .dispatch(Command::SetCommitToolWindowVisible(true))
            .unwrap();
        let mut app = CodisseuApp::new(service);
        app.git_ui.scm.workspace = app.service.workspace.clone();
        app.git_ui.commit_open = true;
        app.git_ui.source_control = !commit;
        let mut h = Harness::builder().with_size([298., 640.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                if !app.styled {
                    app.tokens.apply(ui.ctx());
                    app.styled = true;
                }
                app.git_commit_panel(ui);
            },
            app,
        );
        h.run();
        (dir, h)
    }
    fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
        if let Some(directory) = std::env::var_os("CODISSEU_WORKSPACE_WIDGET_CAPTURES") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            h.render()
                .unwrap()
                .save(directory.join(format!("{name}.png")))
                .unwrap();
        }
    }

    #[test]
    fn absent_repository_has_standard_header_actions_and_no_commit_controls() {
        for commit in [false, true] {
            let (_dir, mut h) = fixture(0, commit);
            assert!(h.query_by_label("Initialize Repository").is_some());
            assert!(h.query_by_label("Clone Repository").is_some());
            assert!(h.query_by_label("Changes section").is_none());
            assert!(h.query_by_label("Repositories (0)").is_none());
            assert!(h.query_by_label("Collapse All Repositories").is_none());
            assert!(
                h.query_by_role(egui::accesskit::Role::MultilineTextInput)
                    .is_none()
            );
            let init = h.get_by_label("Initialize Repository").rect();
            let clone = h.get_by_label("Clone Repository").rect();
            assert_eq!(init.width(), clone.width());
            assert_eq!(init.height(), 26.);
            assert!(init.width() > 240. && init.right() < 298. && init.left() >= 13.);
            assert!(clone.top() > init.bottom());
            capture(&mut h, if commit { "commit-empty" } else { "scm-empty" });
            h.get_by_label("Initialize Repository").click();
            h.run();
            assert!(matches!(
                h.state().git_ui.dialog,
                Some(GitDialog::Action { kind: A::Init, .. })
            ));
            assert!(
                !h.state()
                    .service
                    .workspace
                    .as_ref()
                    .unwrap()
                    .join(".git")
                    .exists()
            );
        }
    }

    #[test]
    fn empty_state_hides_expected_absence_but_displays_operational_errors() {
        let (_dir, mut h) = fixture(0, false);
        let expected = "fatal: not a git repository (or any of the parent directories): .git";
        h.state_mut().service.git.error = Some(expected.into());
        h.run();
        assert!(h.query_by_label(expected).is_none());
        assert!(h.query_by_label("Show Git Output").is_none());
        let genuine = "Permission denied while reading Git configuration";
        h.state_mut().service.git.error = Some(genuine.into());
        h.run();
        assert!(h.query_by_label(genuine).is_none());
        assert_eq!(
            crate::status_messages::current(&h.ctx).as_deref(),
            Some(genuine)
        );
        assert!(h.query_by_label("Show Git Output").is_some());
        assert!(!expected_missing_repository(
            "permission denied\nfatal: not a git repository"
        ));
        capture(&mut h, "scm-empty-error");
    }

    #[test]
    fn actual_unborn_and_clean_repositories_keep_their_own_source_control_states() {
        for (repository, label) in [
            (1, "No commits yet · add files to start"),
            (2, "No changes"),
        ] {
            let (_dir, mut h) = fixture(repository, false);
            h.state_mut().service.git.discovery_truncated = true;
            h.run();
            assert!(h.state().service.git.repo.is_some());
            assert!(h.query_by_label("Initialize Repository").is_none());
            assert!(h.query_by_label(label).is_some());
            assert!(h.query_by_label("Repositories (1)").is_some());
            assert!(
                h.query_by_label("Partial file list · select to refresh")
                    .is_none()
            );
            assert!(h.get_by_label("Commit").accesskit_node().is_disabled());
            capture(
                &mut h,
                if repository == 1 {
                    "scm-unborn"
                } else {
                    "scm-clean"
                },
            );
        }
    }

    #[test]
    fn real_discovery_in_progress_disables_empty_state_mutation_actions() {
        let (_dir, mut h) = fixture(0, false);
        let workspace = h.state().service.workspace.clone().unwrap();
        h.state_mut()
            .service
            .git
            .dispatch(GitCommand::DiscoverRepositories {
                workspace,
                max_depth: 6,
                max_repositories: 32,
                max_entries: 50000,
            })
            .unwrap();
        h.step();
        h.step(); // The worker is real; this panel harness deliberately withholds polling its ACK.
        assert!(h.state().service.git.is_busy());
        assert!(h.query_by_label("Looking for Git repositories…").is_some());
        assert!(
            h.get_by_label("Initialize Repository")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            h.get_by_label("Clone Repository")
                .accesskit_node()
                .is_disabled()
        );
        capture(&mut h, "scm-scanning");
    }
}
