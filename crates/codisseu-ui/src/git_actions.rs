//! Compact Git forms measured in PhpStorm PS-262.10968.76, Islands Dark/AWT1.
//! Geometry: artifacts/m5-git-20261024/reference-original/{new-branch,tag-dialog,
//! reset-dialog,merge-dialog,rebase-dialog}. Unsupported force/overwrite/validate
//! controls are omitted; the supported actions still prepare a separate review.
use super::git::{GitActionKind as A, GitDialog};
use super::*;
use codisseu_core::git::{
    LocalAction,
    workflows::{ResetMode, WorkflowAction as W},
};

impl CodisseuApp {
    pub(super) fn git_compact_action_window(&mut self, ctx: &egui::Context, mut dialog: GitDialog) {
        let GitDialog::Action { kind, .. } = &dialog else {
            self.git_ui.dialog = Some(dialog);
            return;
        };
        let (natural_width, natural_height, action_label): (f32, f32, &str) = match kind {
            A::NewBranch => (452., 122., "Create"),
            A::NewTag => (766., 301., "Create Tag"),
            A::Reset => (766., 192., "Reset"),
            A::Merge => (424., 90., "Merge"),
            A::Rebase => (479., 96., "Rebase"),
            _ => {
                self.git_ui.dialog = Some(dialog);
                return;
            }
        };
        let preparing_before = self.git_ui.preview_requested;
        let mut close = false;
        let width = natural_width.min((ctx.content_rect().width() - 40.).max(240.));
        let height = natural_height.min((ctx.content_rect().height() - 40.).max(90.));
        let root = self
            .service
            .git
            .repo
            .as_ref()
            .map(|r| r.root.display().to_string())
            .unwrap_or_default();
        let branch = self
            .service
            .git
            .snapshot
            .as_ref()
            .and_then(|s| s.head_ref.as_deref())
            .map(|s| s.trim_start_matches("refs/heads/").to_owned())
            .unwrap_or_else(|| "Detached HEAD".into());
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background))
            .inner_margin(0);
        egui::Modal::new(egui::Id::unique("git.compact.action"))
            .frame(frame)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                let (bounds, _) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                let rect = |x: f32, y: f32, w: f32, h: f32| {
                    egui::Rect::from_min_size(
                        bounds.min
                            + egui::vec2(x * width / natural_width, y * height / natural_height),
                        egui::vec2(w * width / natural_width, h * height / natural_height),
                    )
                };
                let GitDialog::Action {
                    kind,
                    name,
                    other,
                    message,
                    mode,
                    check,
                } = &mut dialog
                else {
                    unreachable!()
                };
                ui.ctx().accesskit_node_builder(ui.scope_id(), |node| {
                    node.set_label(match kind {
                        A::NewBranch => "Create New Branch",
                        A::NewTag => "New Tag",
                        A::Reset => "Reset HEAD",
                        A::Merge => "Merge",
                        A::Rebase => "Rebase",
                        _ => unreachable!(),
                    });
                });
                let mut message_has_focus = false;
                match kind {
                    A::NewBranch => {
                        compact_label(ui, rect(12., 14., 88., 22.), "Branch Name:");
                        compact_field(
                            ui,
                            rect(106., 14., 330., 22.),
                            other,
                            "Branch Name",
                            "",
                            "git.branch.name",
                        );
                        let checkout = ui.put(
                            rect(106., 45., 135., 25.),
                            egui::Checkbox::new(check, "Checkout branch"),
                        );
                        checkout.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::CheckBox,
                                true,
                                "Checkout branch",
                            )
                        });
                        let from = if name.is_empty() {
                            "HEAD"
                        } else {
                            name.as_str()
                        };
                        let from = ui.put(
                            rect(249., 45., 188., 25.),
                            egui::Button::new(format!("From {}", compact_revision_label(from)))
                                .frame(false),
                        );
                        from.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Button,
                                true,
                                "Branch start point",
                            )
                        });
                        egui::Popup::menu(&from)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                                ui.set_min_width(300.);
                                let label = ui.label("Start point:");
                                ui.add(egui::TextEdit::singleline(name).hint_text("HEAD"))
                                    .labelled_by(label.id)
                                    .widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::accesskit::Role::TextInput,
                                            true,
                                            "Start point",
                                        )
                                    });
                                self.compact_revision_choices(ui, name);
                            });
                    }
                    A::NewTag | A::Reset => {
                        compact_label(ui, rect(12., 8., 104., 34.), "Git Root:");
                        let root_rect = rect(122., 8., 632., 34.);
                        ui.painter().rect(
                            root_rect.shrink(3.),
                            self.tokens.control_corner_radius,
                            color(self.tokens.panel),
                            egui::Stroke::new(1., color(self.tokens.control_border)),
                            egui::StrokeKind::Inside,
                        );
                        compact_label(ui, root_rect.shrink2(egui::vec2(7., 0.)), &root)
                            .on_hover_text(&root);
                        compact_label(ui, rect(12., 43., 104., 25.), "Current Branch:");
                        compact_label(ui, rect(122., 43., 632., 25.), &branch);
                        if *kind == A::NewTag {
                            compact_label(ui, rect(12., 69., 104., 34.), "Tag Name:");
                            compact_field(
                                ui,
                                rect(122., 69., 632., 34.),
                                other,
                                "Tag Name",
                                "",
                                "git.tag.name",
                            );
                            compact_label(ui, rect(12., 138., 104., 34.), "Commit:");
                            self.compact_revision_field(
                                ui,
                                rect(122., 138., 632., 34.),
                                name,
                                "Commit",
                                "HEAD",
                                "git.tag.commit",
                            );
                            compact_label(ui, rect(12., 177., 104., 25.), "Message:");
                            let edit = ui.put(
                                rect(122., 177., 632., 74.),
                                egui::TextEdit::multiline(message)
                                    .id(egui::Id::unique("git.tag.message"))
                                    .font(egui::TextStyle::Monospace)
                                    .margin(egui::vec2(6., 4.)),
                            );
                            edit.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::TextInput,
                                    true,
                                    "Tag message",
                                )
                            });
                            message_has_focus = edit.has_focus();
                        } else {
                            compact_label(ui, rect(12., 69., 104., 34.), "Reset Type:");
                            ui.scope_builder(
                                egui::UiBuilder::new().max_rect(rect(122., 69., 106., 34.)),
                                |ui| {
                                    egui::ComboBox::from_id_salt("git.compact.reset.mode")
                                        .width(84.)
                                        .selected_text(match *mode % 3 {
                                            0 => "Soft",
                                            1 => "Mixed",
                                            _ => "Hard",
                                        })
                                        .show_ui(ui, |ui| {
                                            for (n, label) in
                                                [(1, "Mixed"), (0, "Soft"), (2, "Hard")]
                                            {
                                                ui.selectable_value(mode, n, label);
                                            }
                                        })
                                        .response
                                        .widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::accesskit::Role::ComboBox,
                                                true,
                                                "Reset Type",
                                            )
                                        });
                                },
                            );
                            compact_label(ui, rect(12., 108., 104., 34.), "To Commit:");
                            self.compact_revision_field(
                                ui,
                                rect(122., 108., 632., 34.),
                                name,
                                "To Commit",
                                "HEAD",
                                "git.reset.commit",
                            );
                        }
                    }
                    A::Merge | A::Rebase => {
                        let command_width = if *kind == A::Merge { 100. } else { 85. };
                        let line = rect(12., 10., natural_width - 24., 28.);
                        ui.painter().rect(
                            line,
                            0.,
                            color(self.tokens.panel),
                            egui::Stroke::new(1., color(self.tokens.control_border)),
                            egui::StrokeKind::Inside,
                        );
                        compact_label(
                            ui,
                            rect(18., 10., command_width - 8., 28.),
                            if *kind == A::Merge {
                                "git merge"
                            } else {
                                "git rebase"
                            },
                        );
                        self.compact_revision_field(
                            ui,
                            rect(
                                12. + command_width,
                                10.,
                                natural_width - 24. - command_width,
                                28.,
                            ),
                            name,
                            if *kind == A::Merge {
                                "Merge revision"
                            } else {
                                "Rebase onto"
                            },
                            "branch or hash",
                            "git.operation.revision",
                        );
                    }
                    _ => unreachable!(),
                }
                let ready = !self.service.git.is_busy()
                    && !self.git_ui.preview_requested
                    && !self.git_ui.applying
                    && self.git_ui.queue.is_empty()
                    && match kind {
                        A::NewBranch | A::NewTag => !other.trim().is_empty(),
                        A::Reset => true,
                        _ => !name.trim().is_empty(),
                    };
                let footer = natural_height - 42.;
                if *kind != A::NewBranch {
                    let help = ui.put(
                        rect(12., footer, 28., 34.),
                        egui::Button::new("").frame(false),
                    );
                    let center = help.rect.center();
                    ui.painter().circle_stroke(
                        center,
                        10.,
                        egui::Stroke::new(1., color(self.tokens.control_border)),
                    );
                    ui.painter().text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        "?",
                        FontId::proportional(13.),
                        color(self.tokens.muted),
                    );
                    help.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Button,
                            true,
                            "Action help",
                        )
                    });
                    egui::Popup::menu(&help).show(|ui| {
                        ui.set_max_width(300.);
                        ui.label("Review the prepared changes before applying them.");
                        if *kind == A::Reset {
                            ui.label(match *mode % 3 {
                                0 => "Soft keeps the index and working files.",
                                1 => "Mixed resets the index and keeps working files.",
                                _ => "Hard replaces tracked working files and the index.",
                            });
                        }
                    });
                }
                let cancel_rect = rect(natural_width - 90., footer, 78., 34.);
                close |= ui
                    .put(cancel_rect, self.tokens.dialog_button("Cancel", false))
                    .clicked();
                let action_width = if *kind == A::NewTag { 103. } else { 78. };
                let action_rect = rect(
                    natural_width - 96. - action_width,
                    footer,
                    action_width,
                    34.,
                );
                let clicked = ui
                    .scope_builder(egui::UiBuilder::new().max_rect(action_rect), |ui| {
                        ui.add_enabled(
                            ready,
                            self.tokens
                                .dialog_button(action_label, true)
                                .min_size(action_rect.size()),
                        )
                    })
                    .inner
                    .clicked();
                let enter = ready
                    && !message_has_focus
                    && !egui::Popup::is_any_open(ctx)
                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                if !close && (clicked || enter) {
                    match kind {
                        A::NewBranch => self.git_prepare(LocalAction::CreateBranch {
                            name: other.clone(),
                            start: (!name.trim().is_empty()).then(|| name.clone()),
                            checkout: *check,
                        }),
                        A::NewTag => self.git_prepare(LocalAction::CreateTag {
                            name: other.clone(),
                            target: if name.trim().is_empty() {
                                "HEAD".into()
                            } else {
                                name.clone()
                            },
                            message: (!message.is_empty()).then(|| message.clone()),
                        }),
                        A::Reset => self.git_prepare_workflow(W::Reset {
                            revision: if name.trim().is_empty() {
                                "HEAD".into()
                            } else {
                                name.clone()
                            },
                            mode: match *mode % 3 {
                                0 => ResetMode::Soft,
                                1 => ResetMode::Mixed,
                                _ => ResetMode::Hard,
                            },
                        }),
                        A::Merge => self.git_prepare_workflow(W::Merge {
                            revision: name.clone(),
                        }),
                        A::Rebase => self.git_prepare_workflow(W::Rebase { onto: name.clone() }),
                        _ => unreachable!(),
                    }
                    close = !preparing_before && self.git_ui.preview_requested;
                }
                if self.service.git.error.is_some() || !self.git_ui.notice.is_empty() {
                    ui.add_space(6.);
                    self.git_status_line(ui);
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

    fn compact_revision_field(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        value: &mut String,
        label: &str,
        hint: &str,
        salt: &str,
    ) {
        ui.painter().rect(
            rect,
            self.tokens.control_corner_radius,
            color(self.tokens.panel),
            egui::Stroke::new(1., color(self.tokens.control_border)),
            egui::StrokeKind::Inside,
        );
        let input = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(26., 0.));
        let edit = ui.put(
            input,
            egui::TextEdit::singleline(value)
                .id(egui::Id::unique(salt))
                .font(FontId::proportional(13.))
                .hint_text(hint)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(6, 4))),
        );
        edit.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, label)
        });
        let button = ui.put(
            egui::Rect::from_min_max(egui::pos2(input.right(), rect.top()), rect.max),
            egui::Button::new("").frame(false),
        );
        components::paint_icon(
            ui,
            button.rect.shrink(6.),
            Icon::Down,
            color(self.tokens.muted),
        );
        button.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                format!("Choose {label}"),
            )
        });
        egui::Popup::menu(&button).show(|ui| {
            ui.set_min_width(260.);
            self.compact_revision_choices(ui, value);
        });
    }

    fn compact_revision_choices(&self, ui: &mut egui::Ui, value: &mut String) {
        egui::ScrollArea::vertical()
            .max_height(240.)
            .codisseu()
            .show(ui, |ui| {
                for branch in &self.service.git.branches {
                    if ui
                        .selectable_label(*value == branch.name, &branch.name)
                        .clicked()
                    {
                        *value = branch.name.clone();
                        ui.close();
                    }
                }
                for tag in &self.service.git.tags {
                    if ui.selectable_label(*value == tag.name, &tag.name).clicked() {
                        *value = tag.name.clone();
                        ui.close();
                    }
                }
                if self.service.git.branches.is_empty() && self.service.git.tags.is_empty() {
                    ui.label(
                        RichText::new("Type a branch, tag or commit in the field")
                            .color(color(self.tokens.muted)),
                    );
                }
            });
    }
}

fn compact_label(ui: &mut egui::Ui, rect: egui::Rect, text: &str) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, text));
    ui.painter().with_clip_rect(rect).text(
        egui::pos2(rect.left(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.),
        ui.visuals().text_color(),
    );
    response
}

fn compact_revision_label(value: &str) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(22).collect();
    if chars.next().is_some() {
        format!("{prefix}…")
    } else {
        prefix
    }
}

fn compact_field(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    value: &mut String,
    label: &str,
    hint: &str,
    salt: &str,
) {
    ui.put(
        rect,
        egui::TextEdit::singleline(value)
            .id(egui::Id::unique(salt))
            .font(FontId::proportional(13.))
            .hint_text(hint)
            .margin(egui::vec2(6., 3.)),
    )
    .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, label));
}
