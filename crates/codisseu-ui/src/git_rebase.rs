//! Linear interactive rebase: ordered commit rows, toolbar actions and inline messages.
use super::git::{GitDialog, GitIntent};
use super::*;
use codisseu_core::git::{
    GitCommand,
    rebase::{RebaseAction as Action, RebaseStep},
};
impl CodisseuApp {
    pub(super) fn git_open_interactive_rebase(&mut self, base: String) {
        self.git_ui.dialog = Some(GitDialog::InteractiveRebase {
            base,
            head: String::new(),
            requested: false,
            steps: vec![],
            selected: 0,
            editing: false,
        });
    }
    pub(super) fn git_rebase_window(&mut self, ctx: &egui::Context, mut dialog: GitDialog) {
        let GitDialog::InteractiveRebase {
            base,
            head,
            requested,
            steps,
            selected,
            editing,
        } = &mut dialog
        else {
            return;
        };
        let preparing_before = self.git_ui.preview_requested;
        if !*requested {
            if !self.service.git.trusted {
                self.git_send(GitCommand::Trust(true));
            }
            self.git_send(GitCommand::RebaseRead { base: base.clone() });
            *requested = true;
        }
        let series = self.service.git.rebase_series.clone().filter(|s| {
            s.base == *base
                && !self
                    .git_ui
                    .queue
                    .iter()
                    .any(|c| matches!(&c.command, GitCommand::RebaseRead { .. }))
        });
        if head.is_empty()
            && let Some(series) = &series
        {
            *head = series.head.clone();
            *steps = series
                .commits
                .iter()
                .map(|c| RebaseStep {
                    oid: c.commit.oid.clone(),
                    action: Action::Pick,
                    message: None,
                })
                .collect();
            if let Some(first) = steps.first() {
                self.git_send(GitCommand::CommitDetails {
                    commit: first.oid.clone(),
                });
            }
        }
        let mut close = false;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background))
            .inner_margin(0);
        egui::Modal::new(egui::Id::unique("git.rebase.interactive"))
            .frame(frame)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                ui.set_width(
                    self.tokens
                        .git_rebase_width
                        .min(ctx.content_rect().width() - 48.),
                );
                let height = self
                    .tokens
                    .git_rebase_height
                    .min(ctx.content_rect().height() - 70.);
                let half = ui.available_width() / 2.;
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(half, height - 51.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_size(egui::vec2(half, height - 51.));
                            header(ui, 29., |ui| {
                                ui.add_space(5.);
                                if components::database_icon_button(
                                    ui,
                                    Icon::Up,
                                    "Move commit up",
                                    !*editing && *selected > 0,
                                )
                                .clicked()
                                {
                                    steps.swap(*selected, *selected - 1);
                                    *selected -= 1;
                                }
                                if components::database_icon_button(
                                    ui,
                                    Icon::Down,
                                    "Move commit down",
                                    !*editing && *selected + 1 < steps.len(),
                                )
                                .clicked()
                                {
                                    steps.swap(*selected, *selected + 1);
                                    *selected += 1;
                                }
                                if components::database_icon_button(
                                    ui,
                                    Icon::Revert,
                                    "Pick",
                                    !*editing && !steps.is_empty(),
                                )
                                .clicked()
                                {
                                    steps[*selected].action = Action::Pick;
                                    steps[*selected].message = None;
                                }
                                if components::database_icon_button(
                                    ui,
                                    Icon::Pause,
                                    "Stop to Edit",
                                    !*editing && !steps.is_empty(),
                                )
                                .clicked()
                                {
                                    steps[*selected].action = Action::Edit;
                                }
                                if ui
                                    .add_enabled(
                                        !*editing && !steps.is_empty(),
                                        egui::Button::new("Reword")
                                            .frame(false)
                                            .min_size(egui::vec2(82., 28.)),
                                    )
                                    .clicked()
                                {
                                    let row = &mut steps[*selected];
                                    row.action = Action::Reword;
                                    if row.message.is_none() {
                                        row.message = series
                                            .as_ref()
                                            .and_then(|s| {
                                                s.commits.iter().find(|c| c.commit.oid == row.oid)
                                            })
                                            .map(|c| c.message.clone());
                                    }
                                    *editing = true;
                                }
                                let squash = ui.add_enabled(
                                    !*editing && *selected > 0,
                                    egui::Button::new("Squash⌄")
                                        .frame(false)
                                        .min_size(egui::vec2(100., 28.)),
                                );
                                egui::Popup::menu(&squash).show(|ui| {
                                    for (action, label) in
                                        [(Action::Squash, "Squash"), (Action::Fixup, "Fixup")]
                                    {
                                        if ui.button(label).clicked() {
                                            steps[*selected].action = action;
                                            ui.close();
                                        }
                                    }
                                });
                                if ui
                                    .add_enabled(
                                        !*editing && !steps.is_empty(),
                                        egui::Button::new("Drop")
                                            .frame(false)
                                            .min_size(egui::vec2(70., 28.)),
                                    )
                                    .clicked()
                                {
                                    steps[*selected].action = Action::Drop;
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.add_space(10.);
                                        if ui.link("Reset").clicked()
                                            && let Some(series) = &series
                                        {
                                            *steps = series
                                                .commits
                                                .iter()
                                                .map(|c| RebaseStep {
                                                    oid: c.commit.oid.clone(),
                                                    action: Action::Pick,
                                                    message: None,
                                                })
                                                .collect();
                                            *selected = 0;
                                            *editing = false;
                                        }
                                    },
                                );
                            });
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .id_salt("git.rebase.rows")
                                .max_height(height - 80.)
                                .codisseu()
                                .show(ui, |ui| {
                                    for (n, row) in steps.iter_mut().enumerate() {
                                        let source = series.as_ref().and_then(|s| {
                                            s.commits.iter().find(|c| c.commit.oid == row.oid)
                                        });
                                        let original = source
                                            .map(|c| c.commit.subject.as_str())
                                            .unwrap_or(&row.oid);
                                        let message = row.message.as_deref().unwrap_or(original);
                                        let folded =
                                            matches!(row.action, Action::Squash | Action::Fixup);
                                        let changed = matches!(row.action, Action::Reword)
                                            || row.message.is_some();
                                        if *editing && n == *selected {
                                            egui::Frame::new()
                                                .fill(color(self.tokens.selection))
                                                .inner_margin(8)
                                                .show(ui, |ui| {
                                                    let draft =
                                                        row.message.get_or_insert_with(|| {
                                                            source
                                                                .map(|c| c.message.clone())
                                                                .unwrap_or_else(|| row.oid.clone())
                                                        });
                                                    ui.add_sized(
                                                        [half - 24., 82.],
                                                        egui::TextEdit::multiline(draft)
                                                            .font(egui::TextStyle::Monospace),
                                                    )
                                                    .widget_info(|| {
                                                        egui::WidgetInfo::labeled(
                                                            egui::accesskit::Role::TextInput,
                                                            true,
                                                            "Rebase commit message",
                                                        )
                                                    });
                                                    ui.horizontal(|ui| {
                                                        ui.label(
                                                            RichText::new(
                                                                "Ctrl+Enter to apply message",
                                                            )
                                                            .small()
                                                            .color(color(self.tokens.muted)),
                                                        );
                                                        if ui.button("Done").clicked()
                                                            || ctx.input_mut(|i| {
                                                                i.consume_key(
                                                                    Modifiers::CTRL,
                                                                    Key::Enter,
                                                                )
                                                            })
                                                        {
                                                            *editing = false;
                                                        }
                                                    });
                                                });
                                            continue;
                                        }
                                        let (rect, response) = ui.allocate_exact_size(
                                            egui::vec2(half, self.tokens.git_rebase_row_height),
                                            egui::Sense::click(),
                                        );
                                        response.widget_info(|| {
                                            egui::WidgetInfo::labeled(
                                                egui::accesskit::Role::Button,
                                                true,
                                                format!("Rebase commit {} {}", n + 1, original),
                                            )
                                        });
                                        if n == *selected {
                                            ui.painter().rect_filled(
                                                rect,
                                                0.,
                                                color(self.tokens.database_services_selection),
                                            );
                                        }
                                        let lane = rect.left() + 8.;
                                        let line = color(self.tokens.muted);
                                        if folded {
                                            let x = rect.left() + 31.;
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(x, rect.top()),
                                                    egui::pos2(x, rect.center().y),
                                                ],
                                                egui::Stroke::new(1., line),
                                            );
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(x, rect.center().y),
                                                    egui::pos2(x + 12., rect.center().y),
                                                ],
                                                egui::Stroke::new(1., line),
                                            );
                                        } else if matches!(row.action, Action::Edit) {
                                            components::paint_icon(
                                                ui,
                                                egui::Rect::from_center_size(
                                                    egui::pos2(lane, rect.center().y),
                                                    egui::vec2(12., 12.),
                                                ),
                                                Icon::Pause,
                                                line,
                                            );
                                        } else {
                                            ui.painter().line_segment(
                                                [
                                                    egui::pos2(lane, rect.top()),
                                                    egui::pos2(lane, rect.bottom()),
                                                ],
                                                egui::Stroke::new(1., line),
                                            );
                                            if !matches!(row.action, Action::Drop) {
                                                ui.painter().circle_filled(
                                                    egui::pos2(lane, rect.center().y),
                                                    3.5,
                                                    line,
                                                );
                                            }
                                        }
                                        let text = message.lines().next().unwrap_or_default();
                                        let tint = if matches!(row.action, Action::Drop) {
                                            self.tokens.muted
                                        } else if changed {
                                            self.tokens.accent
                                        } else {
                                            self.tokens.text
                                        };
                                        let pos = egui::pos2(
                                            rect.left() + 28. + if folded { 16. } else { 0. },
                                            rect.center().y,
                                        );
                                        let painter = ui
                                            .painter()
                                            .with_clip_rect(rect.shrink2(egui::vec2(3., 0.)));
                                        painter.text(
                                            pos,
                                            egui::Align2::LEFT_CENTER,
                                            text,
                                            FontId::proportional(13.),
                                            color(tint),
                                        );
                                        if matches!(row.action, Action::Drop) {
                                            painter.line_segment(
                                                [
                                                    pos,
                                                    egui::pos2(
                                                        (pos.x
                                                            + ui.fonts_mut(|f| {
                                                                f.layout_no_wrap(
                                                                    text.to_owned(),
                                                                    FontId::proportional(13.),
                                                                    color(tint),
                                                                )
                                                            })
                                                            .size()
                                                            .x)
                                                            .min(rect.right() - 10.),
                                                        pos.y,
                                                    ),
                                                ],
                                                egui::Stroke::new(1., color(tint)),
                                            );
                                        }
                                        if response.clicked() && !*editing {
                                            *selected = n;
                                            self.git_send(GitCommand::CommitDetails {
                                                commit: row.oid.clone(),
                                            });
                                        }
                                        response.on_hover_text(format!(
                                            "{:?} · {}",
                                            row.action, row.oid
                                        ));
                                    }
                                    if steps.is_empty() {
                                        ui.add_space(16.);
                                        ui.label("No commits after the selected base");
                                    }
                                });
                        },
                    );
                    let (divider, _) =
                        ui.allocate_exact_size(egui::vec2(1., height - 51.), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(divider, 0., color(self.tokens.border));
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), height - 51.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_size(egui::vec2(half - 1., height - 51.));
                            header(ui, 35., |ui| {
                                ui.add_space(8.);
                                if components::database_icon_button(
                                    ui,
                                    Icon::Compare,
                                    "Show commit diff",
                                    !steps.is_empty(),
                                )
                                .clicked()
                                    && let Some(row) = steps.get(*selected)
                                {
                                    let parent = series
                                        .as_ref()
                                        .and_then(|s| {
                                            s.commits.iter().find(|c| c.commit.oid == row.oid)
                                        })
                                        .and_then(|c| c.commit.parents.first().cloned());
                                    self.git_open_revision_diff(parent, row.oid.clone(), None);
                                }
                            });
                            let details = steps
                                .get(*selected)
                                .and_then(|s| {
                                    self.service
                                        .git
                                        .details
                                        .as_ref()
                                        .filter(|d| d.commit == s.oid)
                                })
                                .cloned();
                            egui::ScrollArea::vertical()
                                .id_salt("git.rebase.files")
                                .auto_shrink([false, false])
                                .max_height((height - 51.) * 0.61)
                                .min_scrolled_height((height - 51.) * 0.61)
                                .codisseu()
                                .show(ui, |ui| {
                                    if let Some(d) = &details {
                                        ui.horizontal(|ui| {
                                            ui.add_space(16.);
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
                                                d.changes.len(),
                                                if d.changes.len() == 1 {
                                                    "file"
                                                } else {
                                                    "files"
                                                }
                                            ));
                                        });
                                        for c in &d.changes {
                                            ui.horizontal(|ui| {
                                                ui.add_space(40.);
                                                let (rect, _) = ui.allocate_exact_size(
                                                    egui::vec2(16., 24.),
                                                    egui::Sense::hover(),
                                                );
                                                components::paint_document_icon(
                                                    ui,
                                                    rect,
                                                    c.path.to_str().unwrap_or(""),
                                                    color(self.tokens.accent),
                                                );
                                                let r = ui.selectable_label(
                                                    self.git_ui.file.as_ref() == Some(&c.path),
                                                    c.path.display().to_string(),
                                                );
                                                if r.clicked() {
                                                    self.git_ui.file = Some(c.path.clone());
                                                }
                                                if r.double_clicked() {
                                                    self.git_open_revision_diff(
                                                        d.parents.first().cloned(),
                                                        d.commit.clone(),
                                                        Some(c.path.clone()),
                                                    );
                                                }
                                            });
                                        }
                                    }
                                });
                            ui.separator();
                            if let Some(d) = details {
                                egui::Frame::new().inner_margin(14).show(ui, |ui| {
                                    ui.add_space(4.);
                                    ui.label(
                                        RichText::new(d.message.lines().next().unwrap_or_default())
                                            .monospace()
                                            .strong(),
                                    );
                                    ui.add_space(16.);
                                    if let Some(c) = series.as_ref().and_then(|s| {
                                        s.commits.iter().find(|c| c.commit.oid == d.commit)
                                    }) {
                                        ui.label(format!(
                                            "{} {} <{}> on {}",
                                            &d.commit[..8],
                                            c.commit.author,
                                            c.commit.email,
                                            super::git_log::git_date(c.commit.timestamp)
                                        ));
                                    }
                                });
                            }
                            self.git_status_line(ui);
                        },
                    );
                });
                header(ui, 51., |ui| {
                    ui.add_space(12.);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(12.);
                        close |= ui.add(self.tokens.dialog_button("Cancel", false)).clicked();
                        if ui
                            .add_enabled(
                                !*editing
                                    && !steps.is_empty()
                                    && !head.is_empty()
                                    && !self.service.git.is_busy(),
                                self.tokens.dialog_button("Start Rebasing", true),
                            )
                            .clicked()
                            && self.git_begin_intent(GitIntent::ConfirmedAction)
                        {
                            if !self.service.git.trusted {
                                self.git_send(GitCommand::Trust(true));
                            }
                            self.git_send(GitCommand::RebasePrepare {
                                base: base.clone(),
                                head: head.clone(),
                                steps: steps.clone(),
                            });
                            close = true;
                        }
                    });
                });
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
