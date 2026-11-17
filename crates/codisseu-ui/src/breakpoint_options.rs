//! Breakpoint configuration drafts; only Apply changes the shared DAP service.
use super::*;
use codisseu_core::debugger::{Breakpoint, DebugCommand as D};

pub(super) struct BreakpointDraft {
    point: Breakpoint,
    suspend: bool,
    focus: bool,
    original: Option<Breakpoint>,
    error: Option<String>,
}

impl CodisseuApp {
    pub(super) fn edit_breakpoint(&mut self, path: PathBuf, line: u32, logging: bool) {
        let original = self
            .service
            .debugger
            .breakpoints
            .iter()
            .find(|b| b.path == path && b.line == line)
            .cloned();
        let mut point = original
            .clone()
            .unwrap_or_else(|| Breakpoint::new(path, line));
        if logging && point.options.log_message.is_empty() {
            point.options.log_message = format!(
                "Breakpoint reached: {}:{}",
                point.path.file_name().unwrap_or_default().to_string_lossy(),
                point.line
            );
        }
        self.debugger_ui.breakpoint = Some(BreakpointDraft {
            suspend: point.options.log_message.trim().is_empty(),
            point,
            original,
            focus: true,
            error: None,
        });
    }

    pub(super) fn breakpoint_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.debugger_ui.breakpoint.take() else {
            return;
        };
        let mut close = false;
        let mut apply = false;
        let mut remove = false;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background))
            .inner_margin(16.);
        egui::Modal::new(egui::Id::unique("debug.breakpoint.options"))
            .frame(frame)
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width((ctx.content_rect().width() - 64.).clamp(260., 520.));
                ui.horizontal(|ui| {
                    ui.strong("Line Breakpoint");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Close, "Close Breakpoint Properties", false, 24.)
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add(
                    egui::Label::new(format!(
                        "{}:{}",
                        draft
                            .point
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        draft.point.line
                    ))
                    .truncate(),
                )
                .on_hover_text(draft.point.path.display().to_string());
                ui.add_space(8.);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut draft.point.enabled, "Enabled");
                    ui.add_space(16.);
                    ui.checkbox(&mut draft.suspend, "Suspend");
                });
                ui.add_space(8.);
                let condition = option_field(
                    ui,
                    "Condition:",
                    "Breakpoint condition",
                    &mut draft.point.options.condition,
                    "Boolean expression",
                    true,
                    self.tokens.control_height,
                );
                if draft.focus && draft.suspend {
                    condition.request_focus();
                }
                let _hit = option_field(
                    ui,
                    "Hit count:",
                    "Breakpoint hit count",
                    &mut draft.point.options.hit_condition,
                    "e.g. 5",
                    true,
                    self.tokens.control_height,
                );
                ui.separator();
                let log = option_field(
                    ui,
                    "Log message:",
                    "Breakpoint log message",
                    &mut draft.point.options.log_message,
                    "Message with {expression}",
                    !draft.suspend,
                    self.tokens.control_height,
                );
                if draft.focus && !draft.suspend {
                    log.request_focus();
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Uncheck Suspend to log without pausing. Use {expression} for values.",
                        )
                        .small()
                        .color(color(self.tokens.muted)),
                    )
                    .wrap(),
                );
                let mut candidate = draft.point.clone();
                if draft.suspend {
                    candidate.options.log_message.clear();
                } else if candidate.options.log_message.trim().is_empty() {
                    candidate.options.log_message = format!(
                        "Breakpoint reached: {}:{}",
                        candidate
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        candidate.line
                    );
                }
                let invalid = candidate.options.validate().err();
                for session in self.service.debugger.sessions.iter().filter(|s| {
                    s.state.is_active() && s.state != codisseu_core::debugger::DebugState::Starting
                }) {
                    if let Some(reason) = candidate.options.unsupported(&session.capabilities) {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("{}: {reason}", session.name))
                                    .small()
                                    .color(color(self.tokens.muted)),
                            )
                            .wrap(),
                        );
                    }
                }
                if let Some(error) = draft.error.as_ref().or(invalid.as_ref()) {
                    ui.colored_label(color(self.tokens.error), error);
                }
                ui.add_space(12.);
                ui.horizontal(|ui| {
                    remove = ui
                        .add_enabled(draft.original.is_some(), egui::Button::new("Remove"))
                        .clicked();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        apply = ui
                            .add_enabled(
                                invalid.is_none(),
                                egui::Button::new("Apply").fill(color(self.tokens.accent)),
                            )
                            .clicked();
                        close |= ui.button("Cancel").clicked();
                    });
                });
                close |= consume_exact(ctx, Modifiers::NONE, Key::Escape);
                apply |= !draft.focus
                    && invalid.is_none()
                    && consume_exact(ctx, Modifiers::NONE, Key::Enter);
            });
        draft.focus = false;
        if remove || apply {
            let current = self
                .service
                .debugger
                .breakpoints
                .iter()
                .find(|b| b.path == draft.point.path && b.line == draft.point.line);
            let unchanged = match (current, draft.original.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.enabled == b.enabled && a.options == b.options,
                _ => false,
            };
            if !unchanged {
                draft.error = Some(
                    "Breakpoint changed while editing. Cancel and reopen its properties.".into(),
                );
            } else {
                let command = if remove {
                    D::ToggleBreakpoint {
                        path: draft.point.path.clone(),
                        line: draft.point.line,
                    }
                } else {
                    if draft.suspend {
                        draft.point.options.log_message.clear();
                    } else if draft.point.options.log_message.trim().is_empty() {
                        draft.point.options.log_message = format!(
                            "Breakpoint reached: {}:{}",
                            draft
                                .point
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            draft.point.line
                        );
                    }
                    D::SetBreakpoint(draft.point.clone())
                };
                match self.service.dispatch(Command::Debug(command)) {
                    Ok(()) => close = true,
                    Err(error) => draft.error = Some(error),
                }
            }
        }
        if close {
            self.focus_editor = true;
        } else {
            self.debugger_ui.breakpoint = Some(draft);
        }
    }
}

fn option_field(
    ui: &mut egui::Ui,
    label: &str,
    name: &str,
    text: &mut String,
    hint: &str,
    enabled: bool,
    height: f32,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(100., height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_size(egui::vec2(100., height));
                ui.label(label);
            },
        );
        let response = ui.add_enabled(
            enabled,
            egui::TextEdit::singleline(text)
                .id_salt(name)
                .hint_text(hint)
                .align(egui::Align2::LEFT_CENTER)
                .min_size(egui::vec2(0., height))
                .desired_width(ui.available_width()),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, enabled, name)
        });
        response
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::debugger::{DebugConfiguration, DebugState};
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::time::Instant;

    fn wait(h: &mut Harness<CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            h.step();
            assert!(
                h.state()
                    .service
                    .debugger
                    .sessions
                    .iter()
                    .all(|s| s.state != DebugState::Failed),
                "{:?}",
                h.state().service.debugger.sessions
            );
            if predicate(h.state()) {
                return;
            }
            assert!(
                Instant::now() < end,
                "{:?}",
                h.state().service.debugger.sessions
            );
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    fn fill(h: &mut Harness<CodisseuApp>, label: &str, text: &str) {
        h.get_by_label(label).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.step();
        h.event(egui::Event::Text(text.into()));
        h.step();
        h.run();
    }
    fn click(
        h: &mut Harness<CodisseuApp>,
        label: &str,
        button: egui::PointerButton,
        modifiers: Modifiers,
    ) {
        let pos = h.get_by_label(label).rect().center();
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            });
            h.step();
        }
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run();
    }
    fn capture(h: &mut Harness<CodisseuApp>, name: &str, scale: f32) {
        if let Some(out) = std::env::var_os("CODISSEU_BREAKPOINT_OPTIONS_CAPTURES") {
            let out = PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(out.join(format!("{name}-{scale}.png")))
                .unwrap();
        }
    }
    #[test]
    fn breakpoint_properties_pointer_keyboard_cancel_apply_and_real_php_node() {
        for scale in [1., 1.25] {
            for (extension, source, variable, line, log_line) in [
                (
                    "php",
                    "<?php\nfor ($i=0;$i<6;$i++) {\n    $value=$i*2;\n    $seen=$value;\n}\necho 'finished';\n",
                    "$i",
                    4,
                    3,
                ),
                (
                    "js",
                    "for (let i=0;i<6;i++) {\n    const value=i*2;\n    const seen=value;\n}\nconsole.log('finished');\n",
                    "i",
                    3,
                    2,
                ),
            ] {
                let temp = tempfile::tempdir().unwrap();
                let path = temp.path().join(format!("loop.{extension}"));
                std::fs::write(&path, source).unwrap();
                let mut service = AppService::new(temp.path().join("profile/session.json"));
                service.dispatch(Command::OpenFile(path.clone())).unwrap();
                let mut app = CodisseuApp::new(service);
                app.show_project = false;
                app.show_database = false;
                let mut h = Harness::builder()
                    .with_size([1280., 900.])
                    .with_pixels_per_point(scale)
                    .with_max_steps(100)
                    .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), app);
                wait(&mut h, |a| {
                    !a.service.is_busy() && a.service.active_document().is_some()
                });
                h.run();
                let add = format!("Add breakpoint: {}:{line}", path.display());
                click(
                    &mut h,
                    &add,
                    egui::PointerButton::Secondary,
                    Modifiers::NONE,
                );
                h.get_by_label("Add Conditional Breakpoint…").click();
                h.run();
                fill(&mut h, "Breakpoint condition", &format!("{variable} === 3"));
                let left = h.get_by_label("Breakpoint condition").rect().left();
                assert!((h.get_by_label("Breakpoint hit count").rect().left() - left).abs() < 0.1);
                assert!(
                    (h.get_by_label("Breakpoint log message").rect().left() - left).abs() < 0.1
                );
                capture(&mut h, &format!("condition-{extension}"), scale);
                h.key_press(Key::Escape);
                h.run();
                assert!(h.state().service.debugger.breakpoints.is_empty());
                click(
                    &mut h,
                    &add,
                    egui::PointerButton::Secondary,
                    Modifiers::NONE,
                );
                h.get_by_label("Add Conditional Breakpoint…").click();
                h.run();
                fill(&mut h, "Breakpoint condition", &format!("{variable} === 3"));
                h.key_press(Key::Enter);
                h.run();
                assert!(h.state().debugger_ui.breakpoint.is_none());
                assert_eq!(
                    h.state().service.debugger.breakpoints[0].options.condition,
                    format!("{variable} === 3")
                );
                let log_add = format!("Add breakpoint: {}:{log_line}", path.display());
                click(
                    &mut h,
                    &log_add,
                    egui::PointerButton::Primary,
                    Modifiers::SHIFT,
                );
                fill(
                    &mut h,
                    "Breakpoint log message",
                    &format!("loop={{{variable}}}"),
                );
                assert!(
                    !h.get_by_label("Breakpoint log message")
                        .accesskit_node()
                        .is_disabled()
                );
                capture(&mut h, &format!("logging-{extension}"), scale);
                h.get_by_label("Apply").click();
                h.run();
                let marker = format!("Remove breakpoint: {}:{line}", path.display());
                let before = h.get_by_label(&marker).rect();
                click(
                    &mut h,
                    &marker,
                    egui::PointerButton::Primary,
                    Modifiers::ALT,
                );
                assert!(
                    !h.state()
                        .service
                        .debugger
                        .breakpoints
                        .iter()
                        .find(|b| b.line == line)
                        .unwrap()
                        .enabled
                );
                click(
                    &mut h,
                    &marker,
                    egui::PointerButton::Primary,
                    Modifiers::ALT,
                );
                assert!(
                    h.state()
                        .service
                        .debugger
                        .breakpoints
                        .iter()
                        .find(|b| b.line == line)
                        .unwrap()
                        .enabled
                );
                assert_eq!(h.get_by_label(&marker).rect(), before);
                let mut config = DebugConfiguration::for_file(&path, temp.path());
                config.trusted = true;
                h.state_mut()
                    .service
                    .dispatch(Command::Debug(D::Start(config)))
                    .unwrap();
                wait(&mut h, |a| {
                    a.service.debugger.sessions.first().is_some_and(|s| {
                        s.state == DebugState::Stopped && s.pending == 0 && !s.variables.is_empty()
                    })
                });
                let id = h.state().service.debugger.sessions[0].id;
                assert_eq!(h.state().service.debugger.sessions[0].frames[0].line, line);
                h.state_mut()
                    .service
                    .dispatch(Command::Debug(D::Evaluate {
                        session: id,
                        expression: variable.into(),
                    }))
                    .unwrap();
                wait(&mut h, |a| {
                    !a.service.debugger.sessions[0].evaluations.is_empty()
                });
                assert_eq!(
                    h.state().service.debugger.sessions[0].evaluations[0].result,
                    "3"
                );
                capture(&mut h, &format!("paused-{extension}"), scale);
                let output = h.state().service.debugger.sessions[0]
                    .output
                    .iter()
                    .map(|o| o.text.as_str())
                    .collect::<String>();
                assert!(
                    output.contains("loop=0") && output.contains("loop=3"),
                    "{output}"
                );
                h.state_mut()
                    .service
                    .dispatch(Command::Debug(D::Stop(id)))
                    .unwrap();
                wait(&mut h, |a| {
                    !a.service.debugger.sessions[0].state.is_active()
                });
                assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
                assert_eq!(h.state().service.active_document().unwrap().text(), source);
            }
        }
    }
}
