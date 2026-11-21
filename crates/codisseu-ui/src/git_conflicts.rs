use super::git::{GitDialog, GitIntent};
use super::git_merge_style::{MergeStyle, lock, tool};
use super::*;
use codisseu_core::git::{
    GitCommand,
    conflicts::{ConflictSide, ResolutionChoice},
};
impl CodisseuApp {
    pub(super) fn git_open_conflict(&mut self, path: PathBuf) {
        self.git_ui.merge_ui = Default::default();
        self.git_ui.merge_result_revision = 0;
        self.git_ui.conflict_stage = true;
        self.git_ui.merge_apply_next = true;
        self.git_ui.dialog = Some(GitDialog::Conflict {
            path: path.clone(),
            result: String::new(),
            edited: false,
        });
        self.git_send_foreground(GitCommand::ConflictRead { path });
    }
    pub(super) fn git_conflict_window(&mut self, ctx: &egui::Context, dialog: GitDialog) {
        let GitDialog::Conflict {
            path,
            mut result,
            mut edited,
        } = dialog
        else {
            return;
        };
        let file = self
            .service
            .git
            .conflict
            .as_ref()
            .filter(|f| f.path == path);
        let extension = highlight::syntax_key(&path);
        let model = file.and_then(|f| f.text_merge.as_ref()).cloned();
        if let Some(model) = &model {
            let f = file.unwrap();
            let key = format!(
                "{:?}:{:?}:{:?}:{:?}:{:?}",
                self.service.git.repo.as_ref().map(|r| &r.root),
                path,
                f.base.as_ref().map(|s| &s.oid),
                f.ours.as_ref().map(|s| &s.oid),
                f.theirs.as_ref().map(|s| &s.oid)
            );
            let working = if self.git_ui.merge_ui.key_matches(&key) {
                None
            } else {
                f.working
                    .as_deref()
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .filter(|s| count_markers(s) == 0)
            };
            self.git_ui.merge_ui.sync(key, model.clone(), working);
            if !edited || self.git_ui.merge_result_revision != self.git_ui.merge_ui.revision {
                result = self.git_ui.merge_ui.result();
                self.git_ui.merge_markers = count_markers(&result);
                self.git_ui.merge_result_revision = self.git_ui.merge_ui.revision;
                edited = true;
            }
        } else if !edited && let Some(f) = file {
            result = f
                .working
                .as_deref()
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or_default()
                .into();
            edited = true;
        }
        let theme = MergeStyle::get_for_theme(self.tokens.theme);
        let size = ctx.content_rect().size();
        let width = (size.x - 2.).clamp(240., 2000.);
        let height = (size.y - 2.).clamp(220., 1200.);
        let mut close = false;
        let mut action = None;
        let modal_id = egui::Id::unique("git.merge.conflict");
        let position = ctx.content_rect().min + (size - egui::vec2(width + 2., height + 2.)) / 2.;
        egui::Modal::new(modal_id)
            .area(
                egui::Area::new(modal_id)
                    .kind(egui::UiKind::Modal)
                    .sense(egui::Sense::hover())
                    .order(egui::Order::Foreground)
                    .fixed_pos(position)
                    .constrain(false),
            )
            .frame(
                egui::Frame::NONE
                    .fill(color(theme.chrome))
                    .stroke(egui::Stroke::new(1., color(theme.border)))
                    .corner_radius(8),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.set_max_size(egui::vec2(width, height));
                ui.set_min_height(height);
                theme.apply(ui);
                let (title, _) = ui.allocate_exact_size(
                    egui::vec2(width, theme.title_height),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .with_clip_rect(title.shrink2(egui::vec2(36., 0.)))
                    .text(
                        title.center(),
                        egui::Align2::CENTER_CENTER,
                        format!(
                            "Merge Revisions for {}",
                            self.service
                                .git
                                .repo
                                .as_ref()
                                .map(|r| r.root.join(&path))
                                .unwrap_or_else(|| path.clone())
                                .display()
                        ),
                        FontId::new(
                            theme.font_size,
                            egui::FontFamily::Name("codisseu.inter.bold".into()),
                        ),
                        color(theme.muted),
                    );
                ui.interact(
                    title,
                    ui.scope_id().with("merge.title"),
                    egui::Sense::hover(),
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::Label,
                        true,
                        path.display().to_string(),
                    )
                });
                ui.scope_builder(
                    egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                        title.right_top() - egui::vec2(30., 0.),
                        egui::vec2(28., 28.),
                    )),
                    |ui| {
                        if tool(ui, theme, Some(Icon::Close), "", "Close Merge", true).clicked() {
                            close = true;
                        }
                    },
                );
                if let Some(file) = file {
                    ui.allocate_ui_with_layout(
                        egui::vec2(width, theme.toolbar_height),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.add_space(8.);
                            if model.is_some() {
                                self.git_ui.merge_ui.toolbar(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.conflict_stage,
                                );
                            } else {
                                ui.label("Text merge unavailable for this file");
                                ui.menu_button("Merge Settings", |ui| {
                                    ui.checkbox(
                                        &mut self.git_ui.conflict_stage,
                                        "Stage as resolved",
                                    );
                                    if ui.button("Delete File…").clicked() {
                                        action = Some(ResolutionChoice::Delete);
                                        ui.close();
                                    }
                                    if ui.button("Cancel").clicked() {
                                        close = true;
                                        ui.close();
                                    }
                                });
                            }
                        },
                    );
                    if let Some(error) = &self.service.git.error {
                        ui.add(
                            egui::Label::new(
                                RichText::new(error).color(color(self.tokens.git_conflict)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(error);
                    }
                    if !self.git_ui.notice.is_empty() {
                        ui.add(egui::Label::new(&self.git_ui.notice).truncate())
                            .on_hover_text(&self.git_ui.notice);
                    }
                    if let Some(warning) = file
                        .unsupported_reason
                        .as_ref()
                        .or_else(|| file.warnings.first())
                    {
                        ui.add(egui::Label::new(RichText::new(warning).small()).truncate())
                            .on_hover_text(file.warnings.join("\n"));
                    }
                    let gap = theme.connector_width;
                    let pane = (width - 2. * gap) / 3.;
                    let (header, _) = ui.allocate_exact_size(
                        egui::vec2(width, theme.header_height),
                        egui::Sense::hover(),
                    );
                    let branch = self
                        .service
                        .git
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.head_ref.as_deref())
                        .map(|s| s.trim_start_matches("refs/heads/"))
                        .unwrap_or("Ours");
                    source_header(
                        ui,
                        theme,
                        egui::Rect::from_min_size(header.min, egui::vec2(pane, header.height())),
                        &format!("Changes from {branch}"),
                        file.ours.as_ref(),
                        "Left",
                        &path,
                    );
                    ui.painter().text(
                        header.min + egui::vec2(pane + gap + 8., header.height() / 2.),
                        egui::Align2::LEFT_CENTER,
                        "Result",
                        FontId::proportional(theme.font_size),
                        color(theme.foreground),
                    );
                    let right_name = file
                        .theirs
                        .as_ref()
                        .map(|s| s.oid.chars().take(8).collect::<String>())
                        .unwrap_or_else(|| "deleted".into());
                    source_header(
                        ui,
                        theme,
                        egui::Rect::from_min_size(
                            header.min + egui::vec2(2. * (pane + gap), 0.),
                            egui::vec2(pane, header.height()),
                        ),
                        &format!("Changes from {right_name}"),
                        file.theirs.as_ref(),
                        "Right",
                        &path,
                    );
                    let used = ui.cursor().top() - ui.min_rect().top();
                    let body_height = (height - used - theme.footer_height).max(40.);
                    if model.is_some() {
                        self.git_ui
                            .merge_ui
                            .show(ui, &self.tokens, &extension, body_height);
                        if self.git_ui.merge_result_revision != self.git_ui.merge_ui.revision {
                            result = self.git_ui.merge_ui.result();
                            self.git_ui.merge_markers = count_markers(&result);
                            self.git_ui.merge_result_revision = self.git_ui.merge_ui.revision;
                        }
                        if std::mem::take(&mut self.git_ui.merge_ui.delete_requested) {
                            action = Some(ResolutionChoice::Delete);
                        }
                        close |= std::mem::take(&mut self.git_ui.merge_ui.close_requested);
                    } else {
                        legacy_content(
                            ui,
                            file,
                            &mut result,
                            &extension,
                            body_height,
                            &self.tokens,
                        );
                    }
                    let markers = if model.is_some() {
                        self.git_ui.merge_markers
                    } else {
                        count_markers(&result)
                    };
                    let allowed = file.unsupported_reason.is_none() && !self.service.git.is_busy();
                    let ready = allowed
                        && file.editable
                        && markers == 0
                        && (model.is_none() || self.git_ui.merge_ui.pending() == 0);
                    egui::Frame::NONE
                        .fill(color(theme.chrome))
                        .inner_margin(egui::Margin::symmetric(14, 9))
                        .show(ui, |ui| {
                            ui.set_min_height(theme.footer_height - 18.);
                            ui.horizontal(|ui| {
                                let left = ui.add_enabled(
                                    if model.is_some() {
                                        self.git_ui.merge_ui.can_choose()
                                    } else {
                                        allowed && file.ours.is_some()
                                    },
                                    egui::Button::new("Accept Left").min_size(egui::vec2(99., 25.)),
                                );
                                if left.clicked() {
                                    if model.is_some() {
                                        self.git_ui.merge_ui.accept_all(0);
                                        result = self.git_ui.merge_ui.result();
                                        self.git_ui.merge_markers = count_markers(&result);
                                        self.git_ui.merge_result_revision =
                                            self.git_ui.merge_ui.revision;
                                    } else {
                                        action = Some(ResolutionChoice::Ours);
                                    }
                                }
                                let right = ui.add_enabled(
                                    if model.is_some() {
                                        self.git_ui.merge_ui.can_choose()
                                    } else {
                                        allowed && file.theirs.is_some()
                                    },
                                    egui::Button::new("Accept Right")
                                        .min_size(egui::vec2(106., 25.)),
                                );
                                if right.clicked() {
                                    if model.is_some() {
                                        self.git_ui.merge_ui.accept_all(1);
                                        result = self.git_ui.merge_ui.result();
                                        self.git_ui.merge_markers = count_markers(&result);
                                        self.git_ui.merge_result_revision =
                                            self.git_ui.merge_ui.revision;
                                    } else {
                                        action = Some(ResolutionChoice::Theirs);
                                    }
                                }
                                let ready = ready
                                    || allowed
                                        && file.editable
                                        && markers == 0
                                        && model.is_some()
                                        && self.git_ui.merge_ui.pending() == 0;
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add_enabled(
                                                ready,
                                                egui::Button::new(
                                                    RichText::new("Apply Changes")
                                                        .color(egui::Color32::WHITE),
                                                )
                                                .fill(color(theme.accent))
                                                .min_size(egui::vec2(120., 28.)),
                                            )
                                            .clicked()
                                        {
                                            self.git_ui.merge_apply_next = true;
                                            action =
                                                Some(ResolutionChoice::EditedText(result.clone()));
                                        }
                                        if ui
                                            .add_enabled(
                                                ready,
                                                egui::Button::new("Save and Close")
                                                    .min_size(egui::vec2(122., 28.)),
                                            )
                                            .clicked()
                                        {
                                            self.git_ui.merge_apply_next = false;
                                            action =
                                                Some(ResolutionChoice::EditedText(result.clone()));
                                        }
                                    },
                                );
                            });
                        });
                } else {
                    ui.add_space(16.);

                    ui.label("Loading base and both revisions…");
                }
            });
        if model.is_some() {
            self.git_ui
                .merge_ui
                .comparison_window(ctx, &extension, &self.tokens);
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if let Some(choice) = action
            && self.git_begin_intent(GitIntent::Other)
        {
            self.git_ui.conflict_draft = Some(GitDialog::Conflict {
                path: path.clone(),
                result: result.clone(),
                edited,
            });
            if !self.service.git.trusted {
                self.git_send(GitCommand::Trust(true));
            }
            self.git_send(GitCommand::ConflictPrepare {
                path: path.clone(),
                choice,
                stage: self.git_ui.conflict_stage,
            });
        }
        if !close {
            self.git_ui.dialog = Some(GitDialog::Conflict {
                path,
                result,
                edited,
            });
        }
    }
}
fn source_header(
    ui: &mut egui::Ui,
    theme: &MergeStyle,
    rect: egui::Rect,
    label: &str,
    side: Option<&ConflictSide>,
    name: &str,
    path: &std::path::Path,
) {
    lock(
        ui,
        egui::Rect::from_min_size(rect.min + egui::vec2(5., 3.), egui::vec2(14., 14.)),
        theme,
    );
    let painter = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        rect.min,
        rect.max - egui::vec2(92., 0.),
    ));
    painter.text(
        rect.min + egui::vec2(21., rect.height() / 2.),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(theme.font_size),
        color(theme.foreground),
    );
    let details = egui::Rect::from_min_size(
        rect.right_top() - egui::vec2(88., 0.),
        egui::vec2(84., rect.height()),
    );
    let response = ui.interact(
        details,
        ui.scope_id().with(("merge.details", name)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            true,
            format!("Show {name} Details"),
        )
    });
    ui.painter().text(
        details.center(),
        egui::Align2::CENTER_CENTER,
        "Show Details",
        FontId::proportional(theme.font_size),
        color(theme.link),
    );
    egui::Popup::menu(&response).show(|ui| {
        ui.strong(format!("{name} · {}", path.display()));
        if let Some(side) = side {
            ui.label(format!("Git object: {}", side.oid));
            ui.label(format!("Mode: {}", side.mode));
            ui.label(format!("{} bytes", side.bytes.as_ref().map_or(0, Vec::len)));
        } else {
            ui.label("File deleted on this side");
        }
    });
}
fn legacy_content(
    ui: &mut egui::Ui,
    file: &codisseu_core::git::conflicts::ConflictFile,
    result: &mut String,
    ext: &str,
    height: f32,
    tokens: &Tokens,
) {
    ui.columns(3, |columns| {
        for (n, column) in columns.iter_mut().enumerate() {
            egui::ScrollArea::both()
                .id_salt(("merge.fallback", n))
                .max_height(height)
                .min_scrolled_height(height)
                .codisseu()
                .show(column, |ui| {
                    if n == 1 {
                        ui.add_enabled(
                            file.editable,
                            egui::TextEdit::multiline(result)
                                .code_editor()
                                .desired_width(ui.available_width())
                                .desired_rows((height / 22.) as usize),
                        );
                    } else {
                        let side = if n == 0 {
                            file.ours.as_ref()
                        } else {
                            file.theirs.as_ref()
                        };
                        let text = side_text(side);
                        let mut job = highlight::layout_themed(
                            &text,
                            ext,
                            FontId::monospace(13.),
                            tokens.theme,
                        );
                        job.wrap.max_width = ui.available_width();
                        ui.label(job);
                    }
                });
        }
    });
}
fn side_text(side: Option<&ConflictSide>) -> String {
    match side {
        None => "File deleted on this side".into(),
        Some(s) if s.binary => "Binary file".into(),
        Some(s) if s.truncated => "File exceeds preview limit".into(),
        Some(s) => s
            .bytes
            .as_deref()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .unwrap_or_default(),
    }
}
fn count_markers(text: &str) -> usize {
    text.lines()
        .filter(|l| {
            l.starts_with("<<<<<<< ")
                || l.starts_with("||||||| ")
                || *l == "======="
                || l.starts_with(">>>>>>> ")
        })
        .count()
}
