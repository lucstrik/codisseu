//! Sync dialogs mirror the observed two-pane Push and compact Pull layouts.
use super::git::{GitDialog, GitIntent};
use super::*;
use codisseu_core::git::{
    GitCommand,
    workflows::{PullMode, WorkflowAction},
};

impl CodisseuApp {
    fn git_confirm_workflow(&mut self, action: WorkflowAction) {
        if !self.git_begin_intent(GitIntent::ConfirmedAction) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        self.git_send(GitCommand::WorkflowPrepare(action));
    }
    pub(super) fn git_sync_window(&mut self, ctx: &egui::Context, mut dialog: GitDialog) {
        let preparing_before = self.git_ui.preview_requested;
        let mut close = false;
        let push = matches!(&dialog, GitDialog::Push { .. });
        let width = if push {
            self.tokens.git_push_width
        } else {
            self.tokens.git_pull_width
        };
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background))
            .inner_margin(0);
        egui::Modal::new(egui::Id::unique("git.sync.dialog"))
            .frame(frame)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.spacing_mut().item_spacing = egui::vec2(0., 0.);
                ui.set_width(width.min(ctx.content_rect().width() - 40.));
                match &mut dialog {
                    GitDialog::Push {
                        remote,
                        source,
                        target,
                        tags,
                        tag_mode,
                        commit,
                        requested,
                    } => {
                        ui.ctx().accesskit_node_builder(ui.scope_id(), |node| {
                            node.set_label("Push Commits")
                        });
                        if !*requested {
                            if !self.service.git.trusted {
                                self.git_send(GitCommand::Trust(true));
                            }
                            self.git_send(GitCommand::PushReview {
                                remote: remote.clone(),
                                source: source.clone(),
                                target: target.clone(),
                            });
                            *requested = true;
                        }
                        let review = self.service.git.push_review.clone().filter(|r| {
                            r.remote == *remote && r.source == *source && r.target == *target
                        });
                        let body_height = self
                            .tokens
                            .git_push_body_height
                            .min((ctx.content_rect().height() - 120.).max(220.));
                        let half = ui.available_width() / 2.;
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(half, body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_min_size(egui::vec2(half, body_height));
                                    let route = super::git::git_text_row(ui,&self.tokens,&format!("{} → {} : {}",source,remote,target),true,half,29.,15.);
                                    route.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::accesskit::Role::Button,
                                            true,
                                            "Push destination",
                                        )
                                    });
                                    egui::Popup::menu(&route).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                                        ui.set_min_width(300.);
                                        let mut changed = sync_field(ui, "Remote", remote);
                                        changed |= sync_field(ui, "Source branch", source);
                                        changed |= sync_field(ui, "Target branch", target);
                                        if changed {
                                            *commit = None;
                                        }
                                        if ui.button("Apply destination").clicked()
                                            || ui.input_mut(|i| {
                                                i.consume_key(Modifiers::NONE, Key::Enter)
                                            })
                                        {
                                            *requested = false;
                                            ui.close();
                                        }
                                    });
                                    egui::ScrollArea::vertical()
                                        .id_salt("git.push.commits")
                                        .auto_shrink([false, false])
                                        .max_height(body_height - 29.)
                                        .codisseu().show_rows(
                                            ui,
                                            19.,
                                            review.as_ref().map_or(0, |r| r.commits.len()),
                                            |ui, range| {
                                                if let Some(r) = &review {
                                                    for n in range {
                                                        let c = &r.commits[n];
                                                        let row = super::git::git_text_row(
                                                            ui,
                                                            &self.tokens,
                                                            &c.subject,
                                                            commit.as_ref() == Some(&c.oid),
                                                            half,
                                                            19.,
                                                            31.,
                                                        );
                                                        row.widget_info(|| {
                                                            egui::WidgetInfo::labeled(
                                                                egui::accesskit::Role::Button,
                                                                true,
                                                                format!(
                                                                    "Outgoing commit {} {}",
                                                                    &c.oid[..8],
                                                                    c.subject
                                                                ),
                                                            )
                                                        });
                                                        if row.clicked() {
                                                            *commit = Some(c.oid.clone());
                                                            self.git_send(
                                                                GitCommand::CommitDetails {
                                                                    commit: c.oid.clone(),
                                                                },
                                                            );
                                                        }
                                                    }
                                                }
                                            },
                                        );
                                },
                            );
                            let (divider, _) = ui.allocate_exact_size(
                                egui::vec2(1., body_height),
                                egui::Sense::hover(),
                            );
                            ui.painter()
                                .rect_filled(divider, 0., color(self.tokens.border));
                            let right = ui.available_width();
                            ui.allocate_ui_with_layout(
                                egui::vec2(right, body_height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_min_size(egui::vec2(right, body_height));
                                    header(ui, 35., |ui| {
                                        ui.add_space(6.);
                                        let path = self.git_ui.file.clone();
                                        if components::database_icon_button(
                                            ui,
                                            Icon::Compare,
                                            "Show outgoing diff",
                                            path.is_some(),
                                        )
                                        .clicked()
                                            && let Some(path) = path {
                                                self.git_show_outgoing_file(
                                                    commit.as_deref(),
                                                    review.as_ref(),
                                                    path,
                                                );
                                            }
                                        if components::database_icon_button(
                                            ui,
                                            Icon::Refresh,
                                            "Refresh outgoing commits",
                                            true,
                                        )
                                        .clicked()
                                        {
                                            *requested = false;
                                            *commit = None;
                                        }
                                    });
                                    let details = commit.as_ref().and_then(|oid| {
                                        self.service
                                            .git
                                            .details
                                            .as_ref()
                                            .filter(|d| &d.commit == oid)
                                    });
                                    let changes = details
                                        .map(|d| d.changes.clone())
                                        .or_else(|| review.as_ref().map(|r| r.changes.clone()))
                                        .unwrap_or_default();
                                    egui::ScrollArea::both()
                                        .id_salt("git.push.changes")
                                        .auto_shrink([false, false])
                                        .max_height(body_height - 35.)
                                        .codisseu().show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.add_space(12.);
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(16., 24.),
                                                    egui::Sense::hover(),
                                                );
                                                components::paint_icon(
                                                    ui,
                                                    rect,
                                                    Icon::Folder,
                                                    color(self.tokens.muted),
                                                );
                                                ui.label(format!(
                                                    "{} {}",
                                                    changes.len(),
                                                    if changes.len() == 1 {
                                                        "file"
                                                    } else {
                                                        "files"
                                                    }
                                                ));
                                            });
                                            for change in changes {
                                                ui.horizontal(|ui| {
                                                    ui.add_space(40.);
                                                    let (rect, _) = ui.allocate_exact_size(
                                                        egui::vec2(16., 24.),
                                                        egui::Sense::hover(),
                                                    );
                                                    components::paint_document_icon(
                                                        ui,
                                                        rect,
                                                        change.path.to_str().unwrap_or(""),
                                                        color(self.tokens.accent),
                                                    );
                                                    let tint = if change.status == 'A' {
                                                        self.tokens.git_added
                                                    } else if change.status == 'D' {
                                                        self.tokens.git_deleted
                                                    } else {
                                                        self.tokens.git_modified
                                                    };
                                                    let row = ui.add(
                                                        egui::Button::new(
                                                            RichText::new(
                                                                change.path.display().to_string(),
                                                            )
                                                            .color(color(tint)),
                                                        )
                                                        .frame(false),
                                                    );
                                                    if row.clicked() {
                                                        self.git_ui.file =
                                                            Some(change.path.clone());
                                                    }
                                                    if row.double_clicked() {
                                                        self.git_show_outgoing_file(
                                                            commit.as_deref(),
                                                            review.as_ref(),
                                                            change.path,
                                                        );
                                                    }
                                                });
                                            }
                                            if let Some(r) = &review {
                                                if r.commits.is_empty() {
                                                    ui.label(
                                                        RichText::new("No outgoing commits")
                                                            .color(color(self.tokens.muted)),
                                                    );
                                                }
                                                for warning in &r.warnings {
                                                    crate::status_messages::status_message(ui, warning);
                                                }
                                                if !r.can_push
                                                    && self
                                                        .service
                                                        .git
                                                        .snapshot
                                                        .as_ref()
                                                        .and_then(|s| s.head_ref.as_deref())
                                                        .is_some_and(|h| {
                                                            h.trim_start_matches("refs/heads/")
                                                                == source
                                                        })
                                                    && ui.button("Update…").clicked()
                                                {
                                                    self.git_ui.dialog = Some(GitDialog::Pull {
                                                        remote: remote.clone(),
                                                        branch: target.clone(),
                                                        mode: 1,
                                                    });
                                                    close = true;
                                                }
                                            }
                                            self.git_status_line(ui);
                                        });
                                },
                            );
                        });
                        header(ui, self.tokens.git_sync_footer_height, |ui| {
                            ui.spacing_mut().item_spacing.x=6.;
                            ui.add_space(14.);
                            let (help, response)=ui.allocate_exact_size(egui::vec2(22.,24.),egui::Sense::hover());
                            ui.painter().circle_stroke(help.center(),10.,egui::Stroke::new(1.,color(self.tokens.control_border)));
                            ui.painter().text(help.center(),egui::Align2::CENTER_CENTER,"?",FontId::proportional(13.),color(self.tokens.muted));
                            response.on_hover_text("Only the reviewed source, destination and tags are sent. No forced Push.");
                            ui.add_space(22.);
                            let tag_label = ui.checkbox(tags, "Push tags:");
                            egui::ComboBox::from_id_salt("git.push.tags")
                                .selected_text(if *tag_mode == 0 {
                                    "All"
                                } else {
                                    "Current Branch"
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(tag_mode, 0, "All");
                                    ui.selectable_value(tag_mode, 1, "Current Branch");
                                })
                                .response
                                .labelled_by(tag_label.id);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.add_space(12.);
                                    close |= ui
                                        .add(self.tokens.dialog_button("Cancel", false))
                                        .clicked();
                                    let ready = review.as_ref().is_some_and(|r| r.can_push)
                                        && !self.service.git.is_busy()
                                        && self.git_ui.queue.is_empty();
                                    let more=ui.add_enabled(ready,egui::Button::new("⌄").fill(color(self.tokens.accent)).min_size(egui::vec2(28.,self.tokens.control_height)));
                                    more.widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::Button,ready,"Push options"));
                                    egui::Popup::menu(&more).show(|ui|{
                                        for (mode,label) in [(codisseu_core::git::PushTagsMode::All,"Push with All Tags"),(codisseu_core::git::PushTagsMode::CurrentBranch,"Push with Current Branch Tags")] {
                                            if ui.button(label).clicked(){self.git_confirm_workflow(WorkflowAction::PushWithTags{remote:remote.clone(),source:source.clone(),target:target.clone(),mode});close=true;ui.close();}
                                        }
                                    });
                                    if ui
                                        .add_enabled(ready, self.tokens.dialog_button("Push", true))
                                        .clicked()
                                    {
                                        self.git_confirm_workflow(WorkflowAction::PushWithTags {
                                            remote: remote.clone(),
                                            source: source.clone(),
                                            target: target.clone(),
                                            mode: if !*tags {
                                                codisseu_core::git::PushTagsMode::None
                                            } else if *tag_mode == 0 {
                                                codisseu_core::git::PushTagsMode::All
                                            } else {
                                                codisseu_core::git::PushTagsMode::CurrentBranch
                                            },
                                        });
                                        close = true;
                                    }
                                },
                            );
                        });
                    }
                    GitDialog::Pull {
                        remote,
                        branch,
                        mode,
                    } => {
                        ui.ctx()
                            .accesskit_node_builder(ui.scope_id(), |node| node.set_label("Pull"));
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(12, 10))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized([85., 28.], egui::Label::new("git pull"));
                                    egui::ComboBox::from_id_salt("git.pull.remote")
                                        .width(90.)
                                        .selected_text(&*remote)
                                        .show_ui(ui, |ui| {
                                            for r in &self.service.git.remotes {
                                                ui.selectable_value(
                                                    remote,
                                                    r.name.clone(),
                                                    &r.name,
                                                );
                                            }
                                        });
                                    ui.add_sized(
                                        [238., 28.],
                                        egui::TextEdit::singleline(branch)
                                            .hint_text("Remote branch"),
                                    )
                                    .widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::accesskit::Role::TextInput,
                                            true,
                                            "Remote branch",
                                        )
                                    });
                                });
                                ui.add_space(10.);
                                ui.horizontal(|ui| {
                                    let options = ui.link("Modify options⌄");
                                    egui::Popup::menu(&options).show(|ui| {
                                        for (n, label) in ["Fast-forward only", "Merge", "Rebase"]
                                            .iter()
                                            .enumerate()
                                        {
                                            ui.selectable_value(mode, n, *label);
                                        }
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            close |= ui
                                                .add(self.tokens.dialog_button("Cancel", false))
                                                .clicked();
                                            if ui
                                                .add_enabled(
                                                    !self.service.git.is_busy()
                                                        && !remote.is_empty()
                                                        && !branch.is_empty(),
                                                    self.tokens.dialog_button("Pull", true),
                                                )
                                                .clicked()
                                            {
                                                self.git_confirm_workflow(WorkflowAction::Pull {
                                                    remote: remote.clone(),
                                                    branch: branch.clone(),
                                                    mode: [
                                                        PullMode::FastForwardOnly,
                                                        PullMode::Merge,
                                                        PullMode::Rebase,
                                                    ][*mode % 3],
                                                });
                                                close = true;
                                            }
                                        },
                                    );
                                });
                                if self.service.git.error.is_some() {
                                    self.git_status_line(ui);
                                }
                                ui.add_space(4.);
                            });
                    }
                    _ => unreachable!(),
                }
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
    fn git_show_outgoing_file(
        &mut self,
        selected: Option<&str>,
        review: Option<&codisseu_core::git::PushReview>,
        path: PathBuf,
    ) {
        if let Some(review) = review {
            let (base, head) = if let Some(selected) = selected {
                let Some(commit) = review.commits.iter().find(|c| c.oid == selected) else {
                    return;
                };
                (commit.parents.first().cloned(), commit.oid.clone())
            } else {
                (review.diff_base.clone(), review.local_oid.clone())
            };
            self.git_open_revision_diff(base, head, Some(path));
        }
    }
}
fn sync_field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    ui.horizontal(|ui| {
        let caption = ui.label(label);
        ui.text_edit_singleline(value)
            .labelled_by(caption.id)
            .changed()
    })
    .inner
}
