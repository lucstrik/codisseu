//! Context actions: compact list and adjacent inert source preview (PhpStorm Help2026.2).
use super::*;
#[derive(Default)]
pub(super) struct PopupState {
    identity: Option<(DocumentId, u64)>,
    anchor: egui::Pos2,
    rect: Option<egui::Rect>,
    selected: usize,
    requested: Option<u64>,
    before: bool,
}
fn frame(tokens: &Tokens) -> egui::Frame {
    egui::Frame::new()
        .fill(color(tokens.panel))
        .stroke(egui::Stroke::new(1., color(tokens.border)))
        .corner_radius(tokens.shell_corner_radius)
        .inner_margin(8)
}
/// Keep the changed region visible even for a fix near the end of a large file.
fn excerpt(text: &str, line: usize) -> String {
    let start = line.saturating_sub(1);
    let mut value = text
        .lines()
        .skip(start)
        .take(14)
        .collect::<Vec<_>>()
        .join("\n");
    if value.len() > 16384 {
        let mut end = 16384;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
    }
    value
}
fn source_preview(ui: &mut egui::Ui, text: &str, ext: &str, tokens: &Tokens) {
    let mut job = highlight::layout(text, ext, FontId::monospace(tokens.editor_font_size));
    job.wrap.max_width = f32::INFINITY;
    for section in &mut job.sections {
        section.format.line_height = Some(tokens.editor_line_height);
    }
    ui.add(
        egui::Label::new(job)
            .selectable(true)
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}
impl CodisseuApp {
    pub(super) fn quick_fix_popup(&mut self, ctx: &egui::Context) {
        if !self.service.code_actions.is_open() {
            self.rust_features.quick_fixes = PopupState::default();
            return;
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let identity = (doc.id, doc.version);
        let ext = doc
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("ts")
            .to_owned();
        let language = if ext == "php" {
            "PHP"
        } else if matches!(ext.as_str(), "js" | "jsx") {
            "JavaScript"
        } else if matches!(ext.as_str(), "py" | "pyi" | "pyw") {
            "Python"
        } else {
            "TypeScript"
        };
        let state = &mut self.rust_features.quick_fixes;
        if state.identity != Some(identity) {
            *state = PopupState {
                identity: Some(identity),
                anchor: self.caret_screen.unwrap_or(egui::pos2(350., 140.)),
                ..Default::default()
            };
        }
        let take = |key| consume_exact(ctx, Modifiers::NONE, key);
        let mut cancel = take(Key::Escape);
        cancel |= state.rect.is_some_and(|r| {
            ctx.input(|i| {
                i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !r.contains(p))
            })
        });
        let count = self.service.code_actions.items.len();
        let mut scroll = false;
        if count > 0 {
            if take(Key::ArrowDown) {
                state.selected = (state.selected + 1) % count;
                scroll = true;
            }
            if take(Key::ArrowUp) {
                state.selected = (state.selected + count - 1) % count;
                scroll = true;
            }
            if state.requested.is_none() && !self.service.code_actions.is_pending() {
                state.selected = self
                    .service
                    .code_actions
                    .items
                    .iter()
                    .position(|i| i.preferred && i.disabled_reason.is_none())
                    .or_else(|| {
                        self.service
                            .code_actions
                            .items
                            .iter()
                            .position(|i| i.disabled_reason.is_none())
                    })
                    .unwrap_or(0);
            }
            state.selected = state.selected.min(count - 1);
        }
        let enter = take(Key::Enter);
        let ready = self.service.code_actions.has_preview();
        let pending = self.service.code_actions.is_pending();
        let available = ctx.content_rect();
        let width = (available.width() - 16.).clamp(280., 960.);
        let wide = width >= 640.;
        let list_width = if wide {
            (width * 0.43).clamp(230., 400.)
        } else {
            width
        };
        let height = (count.clamp(1, 8) as f32 * 26. + 96.).min(if wide {
            (available.height() - 100.).clamp(120., 340.)
        } else {
            (available.height() / 2. - 50.).clamp(100., 220.)
        });
        let minimum = if language == "Python" {
            if wide {
                (available.height() - 64.).clamp(140., 340.)
            } else {
                (available.height() - height - 48.).clamp(120., 300.)
            }
        } else {
            150.
        };
        let preview_height = height.max(minimum);
        let total_height = if wide {
            preview_height
        } else {
            height + preview_height + 8.
        };
        let x = state
            .anchor
            .x
            .min(available.right() - width - 8.)
            .max(available.left() + 8.);
        let y = (state.anchor.y + self.tokens.editor_line_height + 4.)
            .min(available.bottom() - total_height - 8.)
            .max(available.top() + 8.);
        let mut apply = false;
        let area = egui::Area::new(egui::Id::unique("codisseu.context-actions"))
            .order(egui::Order::Foreground)
            .fixed_pos(egui::pos2(x, y))
            .movable(false)
            .sense(egui::Sense::hover())
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8., 0.);
                ui.with_layout(
                    if wide {
                        egui::Layout::left_to_right(egui::Align::Min)
                    } else {
                        egui::Layout::top_down(egui::Align::Min)
                    },
                    |ui| {
                        frame(&self.tokens).show(ui, |ui| {
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.set_width(list_width - 16.);
                                ui.set_min_height(height - 16.);
                                ui.label(
                                    RichText::new(format!("Correções rápidas · {language}"))
                                        .color(color(self.tokens.muted)),
                                );
                                ui.add_space(6.);
                                egui::ScrollArea::vertical()
                                    .id_salt("context-actions.list")
                                    .max_height(height - 96.)
                                    .auto_shrink([false, false])
                                    .codisseu()
                                    .show(ui, |ui| {
                                        ui.spacing_mut().item_spacing.y = 0.;
                                        if count == 0 {
                                            ui.label(if pending {
                                                "Consultando correções…"
                                            } else {
                                                "Nenhuma correção rápida nesta posição."
                                            });
                                        }
                                        for (index, item) in
                                            self.service.code_actions.items.iter().enumerate()
                                        {
                                            let response = ui.add_sized(
                                                [ui.available_width(), 26.],
                                                egui::Button::new(
                                                    RichText::new(&item.title).color(color(
                                                        if item.disabled_reason.is_some() {
                                                            self.tokens.muted
                                                        } else {
                                                            self.tokens.text
                                                        },
                                                    )),
                                                )
                                                .selected(index == state.selected)
                                                .truncate()
                                                .fill(if index == state.selected {
                                                    color(self.tokens.selection)
                                                } else {
                                                    Color32::TRANSPARENT
                                                })
                                                .stroke(egui::Stroke::NONE)
                                                .corner_radius(self.tokens.control_corner_radius),
                                            );
                                            if scroll && index == state.selected {
                                                response.scroll_to_me(Some(egui::Align::Center));
                                            }
                                            if response.hovered()
                                                && ctx.input(|i| {
                                                    i.pointer.delta() != egui::Vec2::ZERO
                                                })
                                            {
                                                state.selected = index;
                                            }
                                            if response.clicked() {
                                                if index == state.selected
                                                    && state.requested == Some(item.token)
                                                    && ready
                                                {
                                                    apply = item.disabled_reason.is_none();
                                                }
                                                state.selected = index;
                                            }
                                            if let Some(reason) = &item.disabled_reason {
                                                response.on_hover_text(reason);
                                            }
                                        }
                                    });
                                ui.separator();
                                ui.label(
                                    RichText::new("↑↓ escolher · Enter aplicar · Esc fechar")
                                        .small()
                                        .color(color(self.tokens.muted)),
                                );
                                if self.service.code_actions.truncated {
                                    ui.label("Lista limitada a 128 ações.");
                                }
                                ui.horizontal(|ui| {
                                    if ui
                                        .add_enabled(
                                            ready && !pending,
                                            egui::Button::new("Aplicar correção"),
                                        )
                                        .clicked()
                                    {
                                        apply = true;
                                    }
                                    if ui.button("Cancelar").clicked() {
                                        cancel = true;
                                    }
                                });
                            });
                        });
                        {
                            frame(&self.tokens).show(ui, |ui| {
                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                    ui.set_width(if wide {
                                        width - list_width - 24.
                                    } else {
                                        width - 16.
                                    });
                                    ui.set_min_height(preview_height - 16.);
                                    ui.horizontal(|ui| {
                                        ui.label("Prévia da correção");
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui
                                                    .selectable_label(!state.before, "Depois")
                                                    .clicked()
                                                {
                                                    state.before = false;
                                                }
                                                if ui
                                                    .selectable_label(state.before, "Antes")
                                                    .clicked()
                                                {
                                                    state.before = true;
                                                }
                                            },
                                        );
                                    });
                                    ui.separator();
                                    egui::ScrollArea::both()
                                        .id_salt("context-actions.preview")
                                        .max_height(preview_height - 84.)
                                        .auto_shrink([false, false])
                                        .codisseu()
                                        .show(ui, |ui| {
                                            if let Some(item) =
                                                self.service.code_actions.items.get(state.selected)
                                            {
                                                if let Some(reason) = &item.disabled_reason {
                                                    ui.add(egui::Label::new(reason).wrap());
                                                } else if pending
                                                    || !ready
                                                    || state.requested != Some(item.token)
                                                {
                                                    ui.label("Preparando prévia…");
                                                } else {
                                                    for file in &self.service.code_actions.files {
                                                        ui.label(
                                                            RichText::new(
                                                                file.path
                                                                    .file_name()
                                                                    .unwrap_or_default()
                                                                    .to_string_lossy(),
                                                            )
                                                            .small()
                                                            .color(color(self.tokens.muted)),
                                                        );
                                                        let text = if state.before {
                                                            &file.before
                                                        } else {
                                                            &file.after
                                                        };
                                                        let line = file
                                                            .edits
                                                            .first()
                                                            .map_or(0, |e| e.start_line as usize);
                                                        source_preview(
                                                            ui,
                                                            &excerpt(text, line),
                                                            file.path
                                                                .extension()
                                                                .and_then(|s| s.to_str())
                                                                .unwrap_or(&ext),
                                                            &self.tokens,
                                                        );
                                                    }
                                                }
                                            }
                                        });
                                    ui.separator();
                                    ui.label(
                                        RichText::new("Aplicar altera o buffer. Ctrl+S salva.")
                                            .small()
                                            .color(color(self.tokens.muted)),
                                    );
                                });
                            });
                        }
                    },
                );
            });
        state.rect = Some(area.response.rect);
        let chosen = self.service.code_actions.items.get(state.selected);
        let token = chosen
            .filter(|i| i.disabled_reason.is_none())
            .map(|i| i.token);
        // An old ready preview cannot authorize a new selection in this frame.
        apply |= enter && ready && token.is_some() && token == state.requested;
        let request = token.filter(|t| Some(*t) != state.requested);
        if cancel {
            self.send(Command::CancelCodeActions);
            self.rust_features.quick_fixes = PopupState::default();
            self.focus_editor = true;
        } else if let Some(token) = request {
            state.requested = Some(token);
            self.send(Command::PreviewCodeAction { token });
        } else if apply && token == state.requested {
            self.send(Command::ApplyCodeAction);
            self.focus_editor = true;
        }
        if pending {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_starts_at_the_fix_context_instead_of_hiding_a_distant_change() {
        let mut lines = vec!["unchanged"; 1000];
        lines[900] = "return $number;";
        let text = lines.join("\r\n");
        let shown = excerpt(&text, 900);
        assert_eq!(shown.lines().nth(1), Some("return $number;"));
        assert!(shown.lines().count() <= 14);
        let enormous = "😀".repeat(20_000);
        let shown = excerpt(&enormous, 0);
        assert!(shown.len() <= 16384);
        assert!(shown.chars().all(|c| c == '😀'));
    }
}
