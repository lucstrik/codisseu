use super::git::{GitActionKind as A, GitDialog, GitIntent};
use super::*;
use codisseu_core::git::workflows::{OperationKind, PullMode, ResetMode, WorkflowAction as W};
use codisseu_core::git::{GitCommand, LocalAction};
impl CodisseuApp {
    pub(super) fn git_branch_menu(&mut self, ui: &mut egui::Ui) {
        let current = self
            .service
            .git
            .snapshot
            .as_ref()
            .filter(|_| self.service.git.repo.is_some())
            .and_then(|s| s.head_ref.clone())
            .unwrap_or_else(|| {
                if self.service.git.repo.is_some() {
                    "Git".into()
                } else {
                    "Version Control".into()
                }
            });
        let name = current.strip_prefix("refs/heads/").unwrap_or(&current);
        let width = (ui
            .painter()
            .layout_no_wrap(
                name.to_owned(),
                FontId::proportional(13.),
                color(self.tokens.text),
            )
            .size()
            .x
            + 50.)
            .clamp(62., 230_f32.min(ui.available_width().max(62.)));
        ui.add_space(12.);
        let (rect, branch) = ui.allocate_exact_size(egui::vec2(width, 30.), egui::Sense::click());
        let branch = branch.on_hover_text(&current);
        branch.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, "Git Branches")
        });
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 12., rect.center().y),
                egui::vec2(16., 16.),
            ),
            Icon::Branch,
            color(self.tokens.muted),
        );
        super::git_paint::text(
            ui,
            &mut self.git_ui.labels,
            egui::Rect::from_min_max(
                rect.min + egui::vec2(27., 0.),
                rect.max - egui::vec2(15., 0.),
            ),
            name,
            color(self.tokens.text),
            color(self.tokens.bar),
        );
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(rect.right() - 7., rect.center().y),
                egui::vec2(10., 10.),
            ),
            Icon::Down,
            color(self.tokens.muted),
        );
        egui::Popup::menu(&branch)
            .frame(egui::Frame::popup(ui.style()).fill(color(self.tokens.git_popup_background)))
            .anchor(egui::Rect::from_min_max(
                rect.min,
                rect.max + egui::vec2(0., 5.),
            ))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                self.tokens.form_style(ui);
                ui.set_width(
                    (if self.service.git.repo.is_none() {
                        220_f32
                    } else {
                        359_f32
                    })
                    .min((ui.ctx().content_rect().width() - 40.).max(180.)),
                );
                if self.service.git.repo.is_none() {
                    ui.add_space(8.);
                    for (kind, label, icon) in [
                        (A::Init, "Create Git Repository…", Icon::Branch),
                        (A::Clone, "Clone Repository…", Icon::Import),
                    ] {
                        if branch_action_row(
                            ui,
                            &self.tokens,
                            &mut self.git_ui.labels,
                            label,
                            "",
                            icon,
                        )
                        .clicked()
                        {
                            self.git_action(kind, String::new());
                            ui.close();
                        }
                    }
                    ui.add_space(8.);
                    return;
                }
                ui.spacing_mut().item_spacing = egui::vec2(0., 0.);
                let bg = color(self.tokens.git_popup_background);
                ui.painter().rect_filled(ui.max_rect(), 6., bg);
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    ui.add_space(6.);
                    let search = ui.add_sized(
                        [(ui.available_width() - 58.).max(80.), 30.],
                        egui::TextEdit::singleline(&mut self.git_ui.popup_branch_filter)
                            .hint_text("Search branches and actions")
                            .margin(egui::vec2(8., 6.)),
                    );
                    ui.ctx().accesskit_node_builder(search.id, |n| {
                        n.set_label("Search branches and actions")
                    });
                    if components::database_icon_button(ui, Icon::Refresh, "Refresh branches", true)
                        .clicked()
                    {
                        self.git_send(GitCommand::Branches);
                        self.git_send(GitCommand::Tags);
                        self.git_send(GitCommand::BranchPreferences);
                    }
                    let options = components::database_icon_button(
                        ui,
                        Icon::MoreDots,
                        "Branch Options",
                        true,
                    );
                    egui::Popup::menu(&options).show(|ui| {
                        if ui.button("Git Log").clicked() {
                            self.git_open_log();
                            ui.close();
                        }
                        if ui.button("Stash Changes…").clicked() {
                            self.git_action(A::Stash, String::new());
                            ui.close();
                        }
                    });
                });
                ui.add_space(8.);
                for (kind, label, shortcut, icon) in [
                    (A::Pull, "Update Project…", "Ctrl+T", Icon::Import),
                    (A::Push, "Push…", "Ctrl+Shift+K", Icon::Export),
                ] {
                    if branch_action_row(
                        ui,
                        &self.tokens,
                        &mut self.git_ui.labels,
                        label,
                        shortcut,
                        icon,
                    )
                    .clicked()
                    {
                        self.git_action(kind, String::new());
                        ui.close();
                    }
                    if kind == A::Pull
                        && branch_action_row(
                            ui,
                            &self.tokens,
                            &mut self.git_ui.labels,
                            "Commit…",
                            "Ctrl+K",
                            Icon::Changes,
                        )
                        .clicked()
                    {
                        self.git_open_commit();
                        ui.close();
                    }
                }
                ui.add_space(8.);
                ui.separator();
                ui.add_space(8.);
                for (kind, label, icon) in [
                    (A::NewBranch, "New Branch…", Icon::Add),
                    (A::Checkout, "Checkout Tag or Revision…", Icon::Branch),
                ] {
                    if branch_action_row(ui, &self.tokens, &mut self.git_ui.labels, label, "", icon)
                        .clicked()
                    {
                        self.git_action(kind, String::new());
                        ui.close();
                    }
                }
                ui.add_space(8.);
                ui.separator();
                ui.add_space(8.);
                let branches = self.service.git.branches.clone();
                let query = self.git_ui.popup_branch_filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(264.)
                    .auto_shrink([false, true])
                    .codisseu()
                    .show(ui, |ui| {
                        let favorites = self.service.git.branch_preferences.favorites.clone();
                        let recent = self.service.git.branch_preferences.recent.clone();
                        for group in ["Recent", "Favorites", "Local", "Remote"] {
                            let key = format!("popup.branches.{group}");
                            let mut open = self
                                .git_ui
                                .groups
                                .get(&key)
                                .copied()
                                .unwrap_or(group != "Remote");
                            let mut matching = branches
                                .iter()
                                .filter(|b| {
                                    let reference = format!(
                                        "refs/{}/{}",
                                        if b.remote { "remotes" } else { "heads" },
                                        b.name
                                    );
                                    b.name.to_lowercase().contains(&query)
                                        && match group {
                                            "Recent" => b.current || recent.contains(&reference),
                                            "Favorites" => favorites.contains(&reference),
                                            "Remote" => b.remote,
                                            _ => !b.remote,
                                        }
                                })
                                .collect::<Vec<_>>();
                            matching.sort_by_key(|b| {
                                let reference = format!(
                                    "refs/{}/{}",
                                    if b.remote { "remotes" } else { "heads" },
                                    b.name
                                );
                                if group == "Recent" {
                                    (
                                        if b.current {
                                            0
                                        } else {
                                            1 + recent
                                                .iter()
                                                .position(|r| r == &reference)
                                                .unwrap_or(usize::MAX - 1)
                                        },
                                        b.name.as_str(),
                                    )
                                } else {
                                    (
                                        usize::from(!favorites.contains(&reference)),
                                        b.name.as_str(),
                                    )
                                }
                            });
                            if matching.is_empty() {
                                continue;
                            }
                            if branch_action_row(
                                ui,
                                &self.tokens,
                                &mut self.git_ui.labels,
                                group,
                                "",
                                if open { Icon::Down } else { Icon::Next },
                            )
                            .clicked()
                            {
                                open = !open;
                                self.git_ui.groups.insert(key.clone(), open);
                            }
                            if !open && query.is_empty() {
                                continue;
                            }
                            for b in matching {
                                let reference = format!(
                                    "refs/{}/{}",
                                    if b.remote { "remotes" } else { "heads" },
                                    b.name
                                );
                                ui.push_id((&key, &reference), |ui| {
                                    let response =
                                        egui::containers::menu::SubMenuButton::from_button(
                                            egui::Button::new(format!("       {}", b.name))
                                                .frame(false)
                                                .min_size(egui::vec2(ui.available_width(), 24.)),
                                        )
                                        .ui(ui, |ui| {
                                            self.git_branch_context(
                                                ui, &b.name, b.current, b.remote,
                                            )
                                        })
                                        .0;
                                    let fill = if response.hovered() {
                                        color(self.tokens.git_focus_selection)
                                    } else {
                                        bg
                                    };
                                    let rect = egui::Rect::from_min_max(
                                        response.rect.min + egui::vec2(6., 0.),
                                        egui::pos2(
                                            ui.max_rect().right() - 12.,
                                            response.rect.bottom(),
                                        ),
                                    );
                                    ui.painter().rect_filled(rect, 4., fill);
                                    components::paint_icon(
                                        ui,
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.left() + 20., rect.center().y),
                                            egui::vec2(16., 16.),
                                        ),
                                        if favorites.contains(&reference) {
                                            Icon::Star
                                        } else {
                                            Icon::Branch
                                        },
                                        if b.current {
                                            color(self.tokens.git_modified)
                                        } else {
                                            color(self.tokens.muted)
                                        },
                                    );
                                    super::git_paint::text(
                                        ui,
                                        &mut self.git_ui.labels,
                                        egui::Rect::from_min_max(
                                            rect.min + egui::vec2(36., 0.),
                                            rect.max - egui::vec2(18., 0.),
                                        ),
                                        &b.name,
                                        color(self.tokens.text),
                                        fill,
                                    );
                                    components::paint_icon(
                                        ui,
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.right() - 4., rect.center().y),
                                            egui::vec2(10., 10.),
                                        ),
                                        Icon::Next,
                                        color(self.tokens.muted),
                                    );
                                });
                            }
                        }
                        if !self.service.git.tags.is_empty() {
                            let key = "popup.branches.Tags";
                            let open = self.git_ui.groups.get(key).copied().unwrap_or(false);
                            if branch_action_row(
                                ui,
                                &self.tokens,
                                &mut self.git_ui.labels,
                                "Tags",
                                "",
                                if open { Icon::Down } else { Icon::Next },
                            )
                            .clicked()
                            {
                                self.git_ui.groups.insert(key.into(), !open);
                            }
                            if open || !query.is_empty() {
                                for tag in self
                                    .service
                                    .git
                                    .tags
                                    .clone()
                                    .iter()
                                    .filter(|t| t.name.to_lowercase().contains(&query))
                                {
                                    if branch_action_row(
                                        ui,
                                        &self.tokens,
                                        &mut self.git_ui.labels,
                                        &tag.name,
                                        "",
                                        Icon::Tag,
                                    )
                                    .clicked()
                                    {
                                        self.git_action(A::Checkout, tag.name.clone());
                                        ui.close();
                                    }
                                }
                            }
                        }
                    });
                if self.service.git.repo.is_none() {
                    if ui.button("Create Git Repository…").clicked() {
                        self.git_action(A::Init, String::new());
                        ui.close();
                    }
                    if ui.button("Clone Repository…").clicked() {
                        self.git_action(A::Clone, String::new());
                        ui.close();
                    }
                }
            });
    }
    pub(super) fn git_prepare_workflow(&mut self, action: W) {
        if !self.git_begin_intent(GitIntent::Other) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        if !self.git_ui.confirm_current_action {
            match &action {
                W::PushTo {
                    remote,
                    source,
                    target,
                    ..
                }
                | W::PushWithTags {
                    remote,
                    source,
                    target,
                    ..
                } => {
                    self.git_send(GitCommand::PushReview {
                        remote: remote.clone(),
                        source: source.clone(),
                        target: target.clone(),
                    });
                }
                _ => {}
            }
        }
        self.git_send(GitCommand::WorkflowPrepare(action));
    }
    pub(super) fn git_request_stashes(&mut self) {
        self.git_send(GitCommand::Stashes);
    }
    pub(super) fn git_operation_banner(&mut self, ui: &mut egui::Ui) {
        let op = self
            .service
            .git
            .snapshot
            .as_ref()
            .and_then(|s| s.operation.clone());
        if let Some(op) = op {
            ui.horizontal_wrapped(|ui| {
                crate::status_messages::status_message(ui, format!("{} in progress", op));
                let kind = if op.to_lowercase().contains("rebase") {
                    OperationKind::Rebase
                } else if op.to_lowercase().contains("cherry") {
                    OperationKind::CherryPick
                } else if op.to_lowercase().contains("revert") {
                    OperationKind::Revert
                } else {
                    OperationKind::Merge
                };
                if ui.button("Continue").clicked() {
                    self.git_prepare_workflow(W::Continue(kind));
                }
                if ui.button("Abort…").clicked() {
                    self.git_prepare_workflow(W::Abort(kind));
                }
                if ui.button("Resolve Conflicts…").clicked()
                    && let Some(path) = self
                        .service
                        .git
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.entries.iter().find(|e| e.conflict))
                        .map(|e| e.path.clone())
                {
                    self.git_open_conflict(path);
                }
            });
        }
    }
    pub(super) fn git_windows(&mut self, ctx: &egui::Context) {
        if self.git_ui.diff_open
            && (self.git_ui.dialog.is_none()
                || matches!(
                    &self.git_ui.dialog,
                    Some(GitDialog::Push { .. } | GitDialog::InteractiveRebase { .. })
                ))
        {
            if matches!(
                &self.git_ui.dialog,
                Some(GitDialog::Push { .. } | GitDialog::InteractiveRebase { .. })
            ) {
                self.git_ui.diff_return = self.git_ui.dialog.take().map(Box::new);
            }
            self.git_diff_window(ctx);
            return;
        }
        let Some(dialog) = self.git_ui.dialog.take() else {
            return;
        };
        if matches!(&dialog, GitDialog::Scm(_)) {
            self.git_scm_window(ctx, dialog);
            return;
        }
        if let GitDialog::Conflict { .. } = &dialog {
            self.git_conflict_window(ctx, dialog);
            return;
        }
        if matches!(
            &dialog,
            GitDialog::Action {
                kind: A::NewBranch | A::NewTag | A::Reset | A::Merge | A::Rebase,
                ..
            }
        ) {
            self.git_compact_action_window(ctx, dialog);
            return;
        }
        if matches!(&dialog, GitDialog::Push { .. } | GitDialog::Pull { .. }) {
            self.git_sync_window(ctx, dialog);
            return;
        }
        if matches!(
            &dialog,
            GitDialog::Stash { .. }
                | GitDialog::Shelve { .. }
                | GitDialog::Unshelve { .. }
                | GitDialog::Changelist { .. }
        ) {
            self.git_shelf_window(ctx, dialog);
            return;
        }
        if matches!(&dialog, GitDialog::InteractiveRebase { .. }) {
            self.git_rebase_window(ctx, dialog);
            return;
        }
        let preparing_before = self.git_ui.preview_requested;
        let was_preview = matches!(&dialog, GitDialog::Preview);
        let mut applied = false;
        let mut close = false;
        let mut keep = dialog;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background));
        egui::Modal::new(egui::Id::unique("git.action.dialog"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(560.);
                self.tokens.form_style(ui);
                match &mut keep {
                    GitDialog::Preview => {
                        ui.strong("Git · Review Changes");
                        ui.separator();
                        if let Some(preview) = self.service.git.preview.clone() {
                            ui.label(&preview.summary);
                            for warning in &preview.warnings {
                                crate::status_messages::status_message(ui, warning);
                            }
                            egui::ScrollArea::vertical()
                                .max_height(220.)
                                .codisseu()
                                .show(ui, |ui| {
                                    for path in &preview.paths {
                                        ui.label(path.display().to_string());
                                    }
                                });
                            if self.git_ui.committing {
                                ui.separator();
                                ui.label(&self.git_ui.message);
                            }
                            ui.add_space(12.);
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(
                                        !self.service.git.is_busy(),
                                        self.tokens.dialog_button("Apply", true),
                                    )
                                    .clicked()
                                {
                                    self.git_send(GitCommand::Apply {
                                        preview_id: preview.id,
                                        repo_generation: preview.generation,
                                    });
                                    self.git_ui.applying_intent = self
                                        .git_ui
                                        .approved_intent
                                        .take()
                                        .filter(|(id, _)| *id == preview.id)
                                        .map(|(_, i)| i);
                                    self.git_ui.applying = true;
                                    applied = true;
                                    close = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                    self.git_ui.push_after_commit = false;
                                }
                            });
                        } else {
                            ui.label("The preview is no longer current. Prepare the action again.");
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        }
                    }
                    GitDialog::Action {
                        kind,
                        name,
                        other,
                        message,
                        mode,
                        check,
                    } => {
                        ui.strong(kind.title());
                        ui.separator();
                        let branch = self
                            .service
                            .git
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.head_ref.as_ref())
                            .cloned()
                            .unwrap_or_default();
                        let root = self
                            .service
                            .git
                            .repo
                            .as_ref()
                            .map(|r| r.root.display().to_string())
                            .unwrap_or_else(|| {
                                self.service
                                    .workspace
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default()
                            });
                        ui.label(
                            RichText::new(format!("{root}  {branch}"))
                                .color(color(self.tokens.muted)),
                        );
                        match kind {
                            A::Init => {
                                if name.is_empty() {
                                    *name = self
                                        .service
                                        .workspace_default_root()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default();
                                }
                                git_field(ui, "Directory", name);
                                git_field(ui, "Initial branch", other);
                            }
                            A::Clone => {
                                git_field(ui, "URL", name);
                                git_field(ui, "Directory", other);
                            }
                            A::NewBranch => {
                                git_field(ui, "Branch name", other);
                                git_field(ui, "Start point", name);
                                ui.checkbox(check, "Checkout branch");
                            }
                            A::RenameBranch => {
                                git_field(ui, "Current name", name);
                                git_field(ui, "New name", other);
                            }
                            A::DeleteBranch | A::Checkout | A::DeleteTag => {
                                git_field(ui, "Name or revision", name);
                            }
                            A::NewTag => {
                                git_field(ui, "Tag name", other);
                                git_field(ui, "Commit", name);
                                git_field(ui, "Annotation (optional)", message);
                            }
                            A::Fetch | A::Pull | A::Push => {
                                git_field(ui, "Remote", other);
                                if *kind != A::Fetch {
                                    git_field(
                                        ui,
                                        if *kind == A::Push {
                                            "Branch"
                                        } else {
                                            "Remote branch"
                                        },
                                        name,
                                    );
                                }
                                if *kind == A::Pull {
                                    egui::ComboBox::from_id_salt("git.pull.mode")
                                        .selected_text(
                                            ["Fast-forward only", "Merge", "Rebase"][*mode % 3],
                                        )
                                        .show_ui(ui, |ui| {
                                            for (n, label) in
                                                ["Fast-forward only", "Merge", "Rebase"]
                                                    .iter()
                                                    .enumerate()
                                            {
                                                ui.selectable_value(mode, n, *label);
                                            }
                                        });
                                }
                                if *kind == A::Push {
                                    ui.label(format!("{} → {}/{}", name, other, name));
                                }
                            }
                            A::Merge | A::Rebase | A::CherryPick | A::Revert | A::Reset => {
                                git_field(ui, "Revision", name);
                                if *kind == A::Reset {
                                    egui::ComboBox::from_id_salt("git.reset.mode")
                                        .selected_text(
                                            [
                                                "Soft · keep index and files",
                                                "Mixed · reset index, keep files",
                                                "Hard · discard tracked changes",
                                            ][*mode % 3],
                                        )
                                        .show_ui(ui, |ui| {
                                            for (n, label) in
                                                ["Soft", "Mixed", "Hard"].iter().enumerate()
                                            {
                                                ui.selectable_value(mode, n, *label);
                                            }
                                        });
                                }
                            }
                            A::Stash => {
                                git_field(ui, "Message", message);
                                ui.checkbox(check, "Include untracked files");
                            }
                            A::Stashes => {
                                egui::ScrollArea::vertical()
                                    .max_height(240.)
                                    .codisseu()
                                    .show(ui, |ui| {
                                        for stash in &self.service.git.stashes {
                                            if ui
                                                .selectable_label(
                                                    *name == stash.reference,
                                                    format!(
                                                        "{}  {}",
                                                        stash.reference, stash.message
                                                    ),
                                                )
                                                .clicked()
                                            {
                                                *name = stash.reference.clone();
                                            }
                                        }
                                    });
                                git_field(ui, "Stash", name);
                                egui::ComboBox::from_id_salt("git.stash.action")
                                    .selected_text(["Apply", "Pop", "Drop"][*mode % 3])
                                    .show_ui(ui, |ui| {
                                        for (n, label) in
                                            ["Apply", "Pop", "Drop"].iter().enumerate()
                                        {
                                            ui.selectable_value(mode, n, *label);
                                        }
                                    });
                            }
                        }
                        ui.add_space(16.);
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !self.service.git.is_busy(),
                                    self.tokens.dialog_button(
                                        match kind {
                                            A::Init => "Create Repository",
                                            A::Clone => "Clone",
                                            _ => "Review…",
                                        },
                                        true,
                                    ),
                                )
                                .clicked()
                            {
                                match *kind {
                                    A::Init => {
                                        self.git_send(GitCommand::Init {
                                            path: PathBuf::from(&*name),
                                            initial_branch: if other.is_empty() {
                                                "main".into()
                                            } else {
                                                other.clone()
                                            },
                                        });
                                        close = true;
                                    }
                                    A::Clone => {
                                        self.git_send(GitCommand::Clone {
                                            url: name.clone(),
                                            destination: PathBuf::from(&*other),
                                        });
                                        close = true;
                                    }
                                    A::NewBranch => {
                                        self.git_prepare(LocalAction::CreateBranch {
                                            name: other.clone(),
                                            start: (!name.is_empty()).then(|| name.clone()),
                                            checkout: *check,
                                        });
                                        close = true;
                                    }
                                    A::RenameBranch => {
                                        self.git_prepare(LocalAction::RenameBranch {
                                            old: name.clone(),
                                            new: other.clone(),
                                        });
                                        close = true;
                                    }
                                    A::DeleteBranch => {
                                        self.git_prepare(LocalAction::DeleteBranch {
                                            name: name.clone(),
                                        });
                                        close = true;
                                    }
                                    A::Checkout => {
                                        self.git_prepare(LocalAction::CheckoutRevision {
                                            revision: name.clone(),
                                        });
                                        close = true;
                                    }
                                    A::NewTag => {
                                        self.git_prepare(LocalAction::CreateTag {
                                            name: other.clone(),
                                            target: if name.is_empty() {
                                                "HEAD".into()
                                            } else {
                                                name.clone()
                                            },
                                            message: (!message.is_empty()).then(|| message.clone()),
                                        });
                                        close = true;
                                    }
                                    A::DeleteTag => {
                                        self.git_prepare(LocalAction::DeleteTag {
                                            name: name.clone(),
                                        });
                                        close = true;
                                    }
                                    kind => {
                                        let action = match kind {
                                            A::Fetch => W::Fetch {
                                                remote: other.clone(),
                                            },
                                            A::Pull => W::Pull {
                                                remote: other.clone(),
                                                branch: name.clone(),
                                                mode: [
                                                    PullMode::FastForwardOnly,
                                                    PullMode::Merge,
                                                    PullMode::Rebase,
                                                ][*mode % 3],
                                            },
                                            A::Push => W::Push {
                                                remote: other.clone(),
                                                branch: name.clone(),
                                            },
                                            A::Merge => W::Merge {
                                                revision: name.clone(),
                                            },
                                            A::Rebase => W::Rebase { onto: name.clone() },
                                            A::CherryPick => W::CherryPick {
                                                revision: name.clone(),
                                            },
                                            A::Revert => W::Revert {
                                                revision: name.clone(),
                                            },
                                            A::Reset => W::Reset {
                                                revision: name.clone(),
                                                mode: [
                                                    ResetMode::Soft,
                                                    ResetMode::Mixed,
                                                    ResetMode::Hard,
                                                ][*mode % 3],
                                            },
                                            A::Stash => W::StashCreate {
                                                message: message.clone(),
                                                include_untracked: *check,
                                            },
                                            A::Stashes => {
                                                if *mode == 2 {
                                                    W::StashDrop {
                                                        stash: name.clone(),
                                                    }
                                                } else {
                                                    W::StashApply {
                                                        stash: name.clone(),
                                                        pop: *mode == 1,
                                                    }
                                                }
                                            }
                                            _ => unreachable!(),
                                        };
                                        self.git_prepare_workflow(action);
                                        close = true;
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                close = true;
                            }
                        });
                    }
                    GitDialog::Scm(_)
                    | GitDialog::Conflict { .. }
                    | GitDialog::Push { .. }
                    | GitDialog::Pull { .. }
                    | GitDialog::Stash { .. }
                    | GitDialog::Shelve { .. }
                    | GitDialog::Unshelve { .. }
                    | GitDialog::Changelist { .. }
                    | GitDialog::InteractiveRebase { .. } => unreachable!(),
                }
                self.git_status_line(ui);
            });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if close && was_preview && !applied {
            self.git_cancel_preview();
            self.git_ui.dialog = self
                .git_ui
                .conflict_draft
                .take()
                .or_else(|| self.git_ui.operation_draft.take().map(|d| *d));
        } else if close && !preparing_before && self.git_ui.preview_requested {
            self.git_ui.operation_draft = Some(Box::new(keep));
        } else if !close {
            self.git_ui.dialog = Some(keep);
        }
    }
}
fn git_field(ui: &mut egui::Ui, label: &str, text: &mut String) {
    ui.horizontal(|ui| {
        let caption = ui.add_sized([135., 28.], egui::Label::new(label));
        ui.add_sized(
            [390., 28.],
            egui::TextEdit::singleline(text).hint_text(label),
        )
        .labelled_by(caption.id);
    });
}

fn branch_action_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    labels: &mut super::tab_label::TabLabels,
    label: &str,
    shortcut: &str,
    icon: Icon,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
    let bg = if response.hovered() {
        color(tokens.git_focus_selection)
    } else {
        color(tokens.git_popup_background)
    };
    ui.painter()
        .rect_filled(rect.shrink2(egui::vec2(6., 0.)), 4., bg);
    components::paint_icon(
        ui,
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 22., rect.center().y),
            egui::vec2(16., 16.),
        ),
        icon,
        color(tokens.text),
    );
    let shortcut_width = ui
        .painter()
        .layout_no_wrap(
            shortcut.to_owned(),
            FontId::proportional(13.),
            color(tokens.muted),
        )
        .size()
        .x;
    super::git_paint::text(
        ui,
        labels,
        egui::Rect::from_min_max(
            rect.min + egui::vec2(36., 0.),
            rect.max - egui::vec2(shortcut_width + 16., 0.),
        ),
        label,
        color(tokens.text),
        bg,
    );
    if !shortcut.is_empty() {
        super::git_paint::text(
            ui,
            labels,
            egui::Rect::from_min_max(
                egui::pos2(rect.right() - shortcut_width - 12., rect.top()),
                rect.max - egui::vec2(8., 0.),
            ),
            shortcut,
            color(tokens.muted),
            bg,
        );
    }
    response
}
