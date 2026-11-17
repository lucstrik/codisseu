//! Native presentation for real, session-scoped debugger state.
use super::*;
use codisseu_core::debugger::{
    DebugCommand as D, DebugConfiguration, DebugRuntime, DebugSession, DebugState, SessionId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

pub(super) struct ExceptionDraft {
    pub(super) filters: Vec<String>,
    sent: Instant,
}
impl ExceptionDraft {
    pub(super) fn new(filters: Vec<String>) -> Self {
        Self {
            filters,
            sent: Instant::now(),
        }
    }
}

type StopStamp = (
    DebugState,
    Option<i64>,
    Option<(Option<PathBuf>, u32, u32)>,
    u64,
    Option<i64>,
);
#[derive(Default)]
pub(super) struct DebuggerUi {
    pub(super) dialog: bool,
    pub(super) breakpoint: Option<super::breakpoint_options::BreakpointDraft>,
    pub(super) draft: Option<DebugConfiguration>,
    pub(super) selected_configuration: Option<String>,
    pub(super) selected_session: Option<SessionId>,
    pub(super) arguments: String,
    pub(super) environment: String,
    pub(super) mappings: String,
    pub(super) config_error: Option<String>,
    pub(super) session_configs: BTreeMap<SessionId, DebugConfiguration>,
    pub(super) console: bool,
    pub(super) process_console: bool,
    pub(super) muted: bool,
    pub(super) selected_threads: BTreeMap<SessionId, i64>,
    pub(super) expressions: BTreeMap<SessionId, String>,
    pub(super) focus_expression: bool,
    pub(super) expression_widget: Option<egui::Id>,
    // Confirmed state remains in DebugSession; rapid menu edits compose on this draft.
    pub(super) exception_drafts: BTreeMap<SessionId, ExceptionDraft>,
    pub(super) frame_width: Option<f32>,
    pub(super) watches: BTreeMap<SessionId, Vec<String>>,
    pub(super) last_stop: BTreeMap<SessionId, StopStamp>,
    pub(super) requested_variables: BTreeSet<(SessionId, i64)>,
    pub(super) configuration_sequence: u64,
    pub(super) restart: Option<(SessionId, DebugConfiguration)>,
}

impl CodisseuApp {
    pub(super) fn debugger_open_configurations(&mut self) {
        self.debugger_ui.dialog = true;
        if self.debugger_ui.draft.is_none() {
            let saved = self
                .service
                .debug_configurations
                .iter()
                .find(|c| Some(&c.id) == self.debugger_ui.selected_configuration.as_ref())
                .or_else(|| self.service.debug_configurations.first())
                .cloned();
            let config = saved.unwrap_or_else(|| self.debugger_current_file_configuration());
            self.debugger_edit_configuration(config);
        }
    }
    fn debugger_current_file_configuration(&self) -> DebugConfiguration {
        let cwd = self.service.language_project_root().unwrap_or_default();
        let mut configuration = self
            .service
            .active_document()
            .map(|d| DebugConfiguration::for_file(&d.path, &cwd))
            .unwrap_or_else(|| DebugConfiguration {
                cwd,
                ..Default::default()
            });
        if configuration.runtime == DebugRuntime::Python
            && let Some(document) = self.service.active_document()
            && let Some(interpreter) = self.service.python_interpreter_for(&document.path)
        {
            configuration.executable = interpreter;
        }
        configuration
    }
    pub(super) fn debugger_start_selected(&mut self) {
        // Configuration review is explicit; opening a workspace never launches code.
        self.debugger_open_configurations();
    }
    pub(super) fn debugger_toolbar(&mut self, ui: &mut egui::Ui) {
        if icon_button(ui, Icon::Debug, "Debug…  Shift+F9", false, 30.).clicked() {
            self.debugger_start_selected();
        }
    }
    pub(super) fn debugger_tick(&mut self, _ctx: &egui::Context) {
        while let Some((id, config)) = self.service.take_browser_debug_started() {
            self.debugger_ui.session_configs.insert(id, config);
            self.debugger_ui.selected_session = Some(id);
            self.show_activity = true;
            self.tool_tab = ToolTab::Debug;
        }
        self.debugger_ui.muted = !self.service.debugger.breakpoints_enabled();
        self.debugger_ui.exception_drafts.retain(|id, draft| {
            self.service
                .debugger
                .sessions
                .iter()
                .find(|s| s.id == *id)
                .is_some_and(|s| {
                    s.state.is_active()
                        && s.exception_filters != draft.filters
                        // An earlier evaluation error is not a failure of this request.
                        // DAP has a 30s request deadline; discard unconfirmed UI drafts too.
                        && draft.sent.elapsed() < Duration::from_secs(35)
                })
        });
        for session in &self.service.debugger.sessions {
            if session.state != DebugState::Starting
                && session.pending == 0
                && let Some(config) = self.debugger_ui.session_configs.get_mut(&session.id)
            {
                config.exception_filters = session.exception_filters.clone();
            }
        }
        if let Some((id, config)) = self.debugger_ui.restart.clone()
            && self
                .service
                .debugger
                .sessions
                .iter()
                .find(|session| session.id == id)
                .is_some_and(|session| !session.state.is_active())
        {
            self.debugger_ui.restart = None;
            let before = self
                .service
                .debugger
                .sessions
                .last()
                .map(|session| session.id);
            self.send(Command::Debug(D::Start(config.clone())));
            if let Some(next) = self
                .service
                .debugger
                .sessions
                .last()
                .map(|session| session.id)
                .filter(|next| Some(*next) != before)
            {
                self.debugger_ui.session_configs.insert(next, config);
                self.debugger_ui.selected_session = Some(next);
            }
        }
        if self.debugger_ui.selected_session.is_none() {
            self.debugger_ui.selected_session = self.service.debugger.sessions.last().map(|s| s.id);
        }
        let changed = self
            .service
            .debugger
            .sessions
            .iter()
            .filter_map(|s| {
                let selected = s
                    .frames
                    .iter()
                    .find(|frame| Some(frame.id) == s.current_frame)
                    .or_else(|| s.frames.first());
                let key = (
                    s.state,
                    s.current_frame,
                    selected.map(|frame| (frame.path.clone(), frame.line, frame.column)),
                    s.stop_epoch,
                    s.current_thread,
                );
                (self.debugger_ui.last_stop.get(&s.id) != Some(&key)).then_some((
                    s.id,
                    key,
                    s.frames
                        .iter()
                        .find(|frame| Some(frame.id) == s.current_frame)
                        .or_else(|| s.frames.first())
                        .cloned(),
                ))
            })
            .collect::<Vec<_>>();
        for (id, state, frame) in changed {
            self.debugger_ui.last_stop.insert(id, state.clone());
            self.debugger_ui
                .requested_variables
                .retain(|(session, _)| *session != id);
            if state.0 == DebugState::Stopped && state.1.is_some() {
                self.debugger_ui.selected_session = Some(id);
                self.show_activity = true;
                self.tool_tab = ToolTab::Debug;
                if let Some(frame) = frame {
                    self.debugger_reveal_frame(&frame);
                }
                for expression in self
                    .debugger_ui
                    .watches
                    .get(&id)
                    .cloned()
                    .unwrap_or_default()
                {
                    self.send(Command::Debug(D::Evaluate {
                        session: id,
                        expression,
                    }));
                }
            }
        }
    }
    pub(super) fn debugger_shortcuts(&mut self, ctx: &egui::Context) {
        if self.debugger_ui.dialog || self.debugger_ui.breakpoint.is_some() {
            return;
        }
        if consume_exact(ctx, Modifiers::SHIFT, Key::F9) {
            self.debugger_start_selected();
        }
        if !text_field_focused(ctx)
            && consume_exact(ctx, Modifiers::CTRL | Modifiers::SHIFT, Key::F8)
            && let Some(doc) = self.service.active_document()
        {
            let path = doc.path.clone();
            let cursor = self
                .service
                .active_view()
                .and_then(|v| self.service.view(v))
                .map(|v| v.selection().head)
                .unwrap_or_else(|| doc.selection().head);
            let line = doc.char_to_line(cursor).unwrap_or(0) as u32 + 1;
            self.edit_breakpoint(path, line, false);
            return;
        }
        if !text_field_focused(ctx)
            && consume_exact(ctx, Modifiers::CTRL, Key::F8)
            && let Some(doc) = self.service.active_document()
        {
            let path = doc.path.clone();
            let cursor = self
                .service
                .active_view()
                .and_then(|view| self.service.view(view))
                .map(|view| view.selection().head)
                .unwrap_or_else(|| doc.selection().head);
            let line = doc.char_to_line(cursor).unwrap_or(0) as u32 + 1;
            self.send(Command::Debug(D::ToggleBreakpoint { path, line }));
        }
        self.debugger_execution_shortcuts(ctx);
    }
    /// Session controls also work while a browser surface has keyboard focus.
    /// Source breakpoint commands remain scoped to the editor above.
    pub(super) fn debugger_execution_shortcuts(&mut self, ctx: &egui::Context) {
        let Some(id) = self.debugger_ui.selected_session else {
            return;
        };
        let state = self
            .service
            .debugger
            .sessions
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.state);
        if state == Some(DebugState::Stopped) {
            if consume_exact(ctx, Modifiers::ALT, Key::F8) {
                self.debugger_ui.console = false;
                self.debugger_ui.focus_expression = true;
                if let Some(widget) = self.debugger_ui.expression_widget {
                    ctx.memory_mut(|m| m.request_focus(widget));
                }
                ctx.request_repaint();
                self.show_activity = true;
                self.tool_tab = ToolTab::Debug;
            }
            for (key, modifiers, command) in [
                (Key::F9, Modifiers::NONE, D::Continue(id)),
                (Key::F8, Modifiers::NONE, D::Next(id)),
                (Key::F7, Modifiers::NONE, D::StepIn(id)),
                (Key::F8, Modifiers::SHIFT, D::StepOut(id)),
            ] {
                if consume_exact(ctx, modifiers, key) {
                    self.send(Command::Debug(command));
                }
            }
        }
        if consume_exact(ctx, Modifiers::SHIFT, Key::F5) && state.is_some_and(DebugState::is_active)
        {
            self.debugger_ui.restart = None;
            self.send(Command::Debug(D::Stop(id)));
        }
    }
    fn debugger_reveal_frame(&mut self, frame: &codisseu_core::debugger::StackFrame) {
        if let Some(path) = &frame.path {
            self.jump_to = Some((
                path.clone(),
                frame.line.saturating_sub(1),
                frame.column.saturating_sub(1),
            ));
            self.send(Command::OpenFile(path.clone()));
        }
    }
    pub(super) fn debugger_panel(&mut self, ui: &mut egui::Ui) {
        super::git_scroll::scm_scroll_style(ui);
        ui.spacing_mut().item_spacing = egui::vec2(0., 0.);
        ui.horizontal(|ui| {
            ui.add_space(12.);
            ui.strong("Debug");
            ui.add_space(12.);
            let sessions = self
                .service
                .debugger
                .sessions
                .iter()
                .map(|s| (s.id, s.name.clone(), s.state))
                .collect::<Vec<_>>();
            egui::ScrollArea::horizontal()
                .id_salt("debug.session.tabs")
                .max_width((ui.available_width() - 32.).max(0.))
                .codisseu()
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (id, name, state) in sessions {
                            let selected = self.debugger_ui.selected_session == Some(id);
                            let response = ui
                                .selectable_label(selected, name)
                                .on_hover_text(format!("{} · session {id}", state_label(state)));
                            if response.clicked() {
                                self.debugger_ui.selected_session = Some(id);
                                let frame =
                                    self.service
                                        .debugger
                                        .sessions
                                        .iter()
                                        .find(|session| {
                                            session.id == id && session.state == DebugState::Stopped
                                        })
                                        .and_then(|session| {
                                            session.frames.iter().find(|frame| {
                                                Some(frame.id) == session.current_frame
                                            })
                                        })
                                        .cloned();
                                if let Some(frame) = frame {
                                    self.debugger_reveal_frame(&frame);
                                }
                            }
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::accesskit::Role::Tab,
                                    true,
                                    selected,
                                    format!("Debug session {id}"),
                                )
                            });
                            ui.add_space(8.);
                        }
                    });
                });
            if icon_button(ui, Icon::Add, "New Debug Configuration", false, 28.).clicked() {
                self.debugger_open_configurations();
            }
        });
        ui.separator();
        let session = self
            .debugger_ui
            .selected_session
            .and_then(|id| self.service.debugger.sessions.iter().find(|s| s.id == id))
            .cloned();
        let Some(session) = session else {
            ui.vertical_centered(|ui| {
                ui.add_space(24.);
                if self.service.browser_debug_pending() {
                    ui.spinner();
                    ui.label("Preparando a depuração da aba integrada…");
                } else {
                    ui.label(RichText::new("No debug sessions").color(color(self.tokens.muted)));
                }
                ui.add_space(8.);
                if ui.button("Edit Configurations…").clicked() {
                    self.debugger_open_configurations();
                }
                ui.add_space(8.);
                ui.label("PHP / Xdebug · JavaScript · TypeScript · Rust · Browser");
                if let Some(error) = &self.service.debugger.error {
                    crate::status_messages::status_message(ui, error);
                }
            });
            return;
        };
        self.debugger_controls(ui, &session);
        ui.separator();
        if session.state == DebugState::Stopped
            && session
                .frames
                .iter()
                .find(|frame| Some(frame.id) == session.current_frame)
                .and_then(|frame| frame.path.as_ref())
                .is_some_and(|path| {
                    self.service
                        .documents
                        .iter()
                        .any(|doc| &doc.path == path && doc.is_dirty())
                })
        {
            crate::status_messages::status_message(
                ui,
                "Source differs from the file on disk. Save and restart for accurate line mapping.",
            );
        }

        if let Some(error) = &session.last_error {
            crate::status_messages::status_message(ui, error);
        }
        if self.debugger_ui.console {
            self.debugger_console(ui, &session);
        } else {
            let height = ui.available_height().max(0.);
            let width = ui.available_width();
            ui.horizontal_top(|ui| {
                let frame_width = self
                    .debugger_ui
                    .frame_width
                    .unwrap_or((width * 0.20).clamp(160., 300.))
                    .clamp(100., (width * 0.65).max(100.))
                    .min((width - 100.).max(0.));
                ui.allocate_ui_with_layout(
                    egui::vec2(frame_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.debugger_frames(ui, &session);
                    },
                );
                let (splitter, response) =
                    ui.allocate_exact_size(egui::vec2(6., height), egui::Sense::drag());
                ui.painter().vline(
                    splitter.center().x,
                    splitter.y_range(),
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::Splitter,
                        true,
                        "Resize debug frames",
                    )
                });
                let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if response.dragged() {
                    self.debugger_ui.frame_width = Some(
                        (frame_width + response.drag_delta().x)
                            .clamp(100., (width * 0.65).max(100.)),
                    );
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width().max(0.), height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.debugger_variables(ui, &session);
                    },
                );
            });
        }
    }
    fn debugger_controls(&mut self, ui: &mut egui::Ui, session: &DebugSession) {
        egui::ScrollArea::horizontal()
            .id_salt(("debug.controls", session.id))
            .codisseu()
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(5.);
                    let stopped = session.state == DebugState::Stopped;
                    if let Some(config) = self.debugger_ui.session_configs.get(&session.id).cloned()
                    {
                        let can_restart = self.debugger_ui.restart.is_none();
                        if ui
                            .add_enabled_ui(can_restart, |ui| {
                                icon_button(ui, Icon::Refresh, "Rerun Debug Session", false, 28.)
                            })
                            .inner
                            .clicked()
                        {
                            if session.state.is_active() {
                                self.debugger_ui.restart = Some((session.id, config));
                                self.send(Command::Debug(D::Stop(session.id)));
                            } else {
                                let before = self
                                    .service
                                    .debugger
                                    .sessions
                                    .last()
                                    .map(|session| session.id);
                                self.send(Command::Debug(D::Start(config.clone())));
                                if let Some(next) = self
                                    .service
                                    .debugger
                                    .sessions
                                    .last()
                                    .map(|session| session.id)
                                    .filter(|next| Some(*next) != before)
                                {
                                    self.debugger_ui.session_configs.insert(next, config);
                                    self.debugger_ui.selected_session = Some(next);
                                }
                            }
                        }
                    }

                    for (icon, label, enabled, command) in [
                        (
                            Icon::Play,
                            "Resume Program  F9",
                            stopped,
                            D::Continue(session.id),
                        ),
                        (
                            Icon::Pause,
                            "Pause Program",
                            session.state == DebugState::Running
                                && session.capabilities["supportsPauseRequest"] != false,
                            D::Pause(session.id),
                        ),
                        (
                            Icon::Stop,
                            "Stop Debug Session  Shift+F5",
                            session.state.is_active(),
                            D::Stop(session.id),
                        ),
                        (
                            Icon::StepOver,
                            "Step Over  F8",
                            stopped,
                            D::Next(session.id),
                        ),
                        (
                            Icon::StepInto,
                            "Step Into  F7",
                            stopped,
                            D::StepIn(session.id),
                        ),
                        (
                            Icon::StepOut,
                            "Step Out  Shift+F8",
                            stopped,
                            D::StepOut(session.id),
                        ),
                    ] {
                        let clicked = ui
                            .add_enabled_ui(enabled, |ui| icon_button(ui, icon, label, false, 28.))
                            .inner
                            .clicked();
                        if clicked {
                            self.send(Command::Debug(command));
                        }
                    }
                    if icon_button(
                        ui,
                        Icon::MuteBreakpoints,
                        "Mute Breakpoints",
                        self.debugger_ui.muted,
                        28.,
                    )
                    .clicked()
                    {
                        self.debugger_ui.muted = !self.debugger_ui.muted;
                        self.send(Command::Debug(D::SetBreakpointsEnabled(
                            !self.debugger_ui.muted,
                        )));
                    }
                    let breakpoints = icon_button(ui, Icon::Debug, "View Breakpoints", false, 28.);
                    egui::Popup::menu(&breakpoints).show(|ui| {
                        ui.strong("Breakpoints");
                        let items = self.service.debugger.breakpoints.clone();
                        if items.is_empty() {
                            ui.label("No breakpoints");
                        }
                        for bp in items {
                            let checked = session
                                .breakpoints
                                .iter()
                                .find(|p| p.path == bp.path && p.line == bp.line);
                            let state = editor::debug_marker::state(
                                &bp,
                                checked,
                                session.state.is_active(),
                                self.debugger_ui.muted,
                            );
                            ui.horizontal(|ui| {
                                let mut enabled = bp.enabled;
                                if ui
                                    .checkbox(
                                        &mut enabled,
                                        format!(
                                            "Enabled {}:{}",
                                            bp.path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy(),
                                            bp.line
                                        ),
                                    )
                                    .changed()
                                {
                                    let mut point = bp.clone();
                                    point.enabled = enabled;
                                    self.send(Command::Debug(D::SetBreakpoint(point)));
                                }
                                if icon_button(
                                    ui,
                                    Icon::Settings,
                                    &format!("Edit breakpoint {}:{}", bp.path.display(), bp.line),
                                    false,
                                    24.,
                                )
                                .clicked()
                                {
                                    self.edit_breakpoint(bp.path.clone(), bp.line, false);
                                    ui.close();
                                }
                                if icon_button(
                                    ui,
                                    Icon::Remove,
                                    &format!("Remove breakpoint {}:{}", bp.path.display(), bp.line),
                                    false,
                                    24.,
                                )
                                .clicked()
                                {
                                    self.send(Command::Debug(D::ToggleBreakpoint {
                                        path: bp.path.clone(),
                                        line: bp.line,
                                    }));
                                }
                                let response = ui.selectable_label(
                                    false,
                                    format!(
                                        "{}:{} · {}",
                                        bp.path.file_name().unwrap_or_default().to_string_lossy(),
                                        bp.line,
                                        state.label()
                                    ),
                                );
                                if response.clicked() {
                                    self.jump_to =
                                        Some((bp.path.clone(), bp.line.saturating_sub(1), 0));
                                    self.send(Command::OpenFile(bp.path.clone()));
                                    ui.close();
                                }
                                response.on_hover_text(
                                    checked
                                        .and_then(|p| p.message.as_deref())
                                        .unwrap_or(state.label()),
                                );
                            });
                        }
                    });
                    self.debugger_exception_breakpoints(ui, session);
                    ui.separator();
                    if ui
                        .selectable_label(!self.debugger_ui.console, "Threads & Variables")
                        .clicked()
                    {
                        self.debugger_ui.console = false;
                    }
                    if ui
                        .selectable_label(
                            self.debugger_ui.console && !self.debugger_ui.process_console,
                            "Debugger Console",
                        )
                        .clicked()
                    {
                        self.debugger_ui.console = true;
                        self.debugger_ui.process_console = false;
                    }
                    if ui
                        .selectable_label(
                            self.debugger_ui.console && self.debugger_ui.process_console,
                            "Process Console",
                        )
                        .clicked()
                    {
                        self.debugger_ui.console = true;
                        self.debugger_ui.process_console = true;
                    }
                    if icon_button(ui, Icon::Locate, "Show Execution Point", false, 28.).clicked()
                        && let Some(frame) = session
                            .frames
                            .iter()
                            .find(|f| Some(f.id) == session.current_frame)
                            .or_else(|| session.frames.first())
                    {
                        self.debugger_reveal_frame(frame);
                    }
                    if icon_button(ui, Icon::Settings, "Edit Debug Configurations", false, 28.)
                        .clicked()
                    {
                        self.debugger_open_configurations();
                    }
                    ui.add_space(12.);
                    ui.label(
                        RichText::new(state_label(session.state)).color(color(self.tokens.muted)),
                    );
                    if session.state == DebugState::Stopped
                        && let Some(reason) = &session.stopped_reason
                    {
                        ui.add_space(8.);
                        let reason = match reason.as_str() {
                            "breakpoint" => "Stopped at breakpoint",
                            "exception" => "Stopped on exception",
                            "step" => "Step complete",
                            "pause" => "Paused by user",
                            "entry" => "Stopped at entry",
                            _ => reason,
                        };
                        ui.label(RichText::new(reason).color(color(self.tokens.muted)));
                    }
                    if session.pending > 0 {
                        ui.spinner();
                    }
                });
            });
    }
    fn debugger_exception_breakpoints(&mut self, ui: &mut egui::Ui, session: &DebugSession) {
        let filters = session
            .capabilities
            .get("exceptionBreakpointFilters")
            .and_then(|v| v.as_array());
        let enabled = session.state.is_active() && filters.is_some_and(|f| !f.is_empty());
        let button = ui
            .add_enabled_ui(enabled, |ui| {
                icon_button(
                    ui,
                    Icon::Filter,
                    "Exception Breakpoints",
                    !session.exception_filters.is_empty(),
                    28.,
                )
            })
            .inner;
        egui::Popup::menu(&button).show(|ui| {
            ui.strong("Exception Breakpoints");
            ui.weak("Pause on exceptions in this session");
            if self.debugger_ui.exception_drafts.contains_key(&session.id) {
                ui.weak("Changes awaiting adapter confirmation…");
            }
            let mut selected = self
                .debugger_ui
                .exception_drafts
                .get(&session.id)
                .map(|draft| draft.filters.clone())
                .unwrap_or_else(|| session.exception_filters.clone());
            let mut changed = false;
            for filter in filters.into_iter().flatten().take(32) {
                let Some(id) = filter.get("filter").and_then(|v| v.as_str()) else {
                    continue;
                };
                let label = filter.get("label").and_then(|v| v.as_str()).unwrap_or(id);
                let mut checked = selected.iter().any(|s| s == id);
                let response = ui.checkbox(&mut checked, label);
                if response.changed() {
                    changed = true;
                    selected.retain(|s| s != id);
                    if checked {
                        selected.push(id.into());
                    }
                }
                if let Some(description) = filter.get("description").and_then(|v| v.as_str()) {
                    response.on_hover_text(description);
                }
            }
            if changed {
                self.debugger_ui
                    .exception_drafts
                    .insert(session.id, ExceptionDraft::new(selected.clone()));
                self.send(Command::Debug(D::SetExceptionBreakpoints {
                    session: session.id,
                    filters: selected,
                }));
                if self.service.debugger.error.is_some() {
                    self.debugger_ui.exception_drafts.remove(&session.id);
                }
            }
        });
    }
    fn debugger_frames(&mut self, ui: &mut egui::Ui, session: &DebugSession) {
        ui.horizontal(|ui| {
            ui.add_space(8.);
            ui.strong("Frames");
        });
        ui.separator();
        egui::ScrollArea::both()
            .id_salt(("debug.frames", session.id))
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                if session.state != DebugState::Stopped {
                    ui.add_space(8.);
                    ui.label(
                        RichText::new("Frames are available when paused")
                            .color(color(self.tokens.muted)),
                    );
                    return;
                }
                let mut thread_id = session
                    .current_thread
                    .filter(|id| session.threads.iter().any(|t| t.id == *id))
                    .or_else(|| {
                        self.debugger_ui
                            .selected_threads
                            .get(&session.id)
                            .copied()
                            .filter(|id| session.threads.iter().any(|t| t.id == *id))
                    })
                    .or_else(|| session.threads.first().map(|t| t.id))
                    .unwrap_or_default();
                let before = thread_id;
                let caption = session
                    .threads
                    .iter()
                    .find(|t| t.id == thread_id)
                    .map(|t| t.name.as_str())
                    .unwrap_or("Thread");
                let thread_response = egui::ComboBox::from_id_salt(("debug.thread", session.id))
                    .selected_text(caption)
                    .wrap_mode(egui::TextWrapMode::Truncate)
                    .width((ui.available_width() - 8.).max(0.))
                    .show_ui(ui, |ui| {
                        for thread in &session.threads {
                            ui.selectable_value(&mut thread_id, thread.id, &thread.name);
                        }
                    });
                ui.ctx()
                    .accesskit_node_builder(thread_response.response.id, |node| {
                        node.set_label("Debug Thread")
                    });
                if thread_id != before {
                    self.debugger_ui
                        .selected_threads
                        .insert(session.id, thread_id);
                    self.send(Command::Debug(D::SelectThread {
                        session: session.id,
                        thread_id,
                    }));
                }
                for frame in &session.frames {
                    let location = frame
                        .path
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "<source unavailable>".into());
                    let selected = Some(frame.id) == session.current_frame;
                    let label = format!("{}  {}:{}", frame.name, location, frame.line);
                    let response = ui.add_sized(
                        [ui.available_width(), 24.],
                        egui::Button::selectable(selected, label).truncate(),
                    );
                    if response.clicked() {
                        self.debugger_ui
                            .requested_variables
                            .retain(|(id, _)| *id != session.id);
                        self.send(Command::Debug(D::SelectFrame {
                            session: session.id,
                            frame_id: frame.id,
                        }));
                        self.debugger_reveal_frame(frame);
                    }
                    if let Some(path) = &frame.path {
                        response.on_hover_text(path.display().to_string());
                    }
                }
            });
    }
    fn debugger_variables(&mut self, ui: &mut egui::Ui, session: &DebugSession) {
        ui.horizontal(|ui| {
            ui.add_space(8.);
            ui.strong("Variables");
        });
        ui.separator();
        ui.horizontal(|ui| {
            let response = ui.add_sized(
                [(ui.available_width() - 78.).max(0.), 26.],
                egui::TextEdit::singleline(
                    self.debugger_ui.expressions.entry(session.id).or_default(),
                )
                .hint_text("Evaluate expression")
                .id_salt(("debug.expression", session.id)),
            );
            ui.ctx()
                .accesskit_node_builder(response.id, |node| node.set_label("Evaluate expression"));
            self.debugger_ui.expression_widget = Some(response.id);
            if std::mem::take(&mut self.debugger_ui.focus_expression) {
                response.request_focus();
            }
            response.clone().on_hover_text(
                "Evaluate expression · Alt+F8\nExpressions execute in the selected frame",
            );
            // A single-line TextEdit surrenders focus on Enter before returning.
            let enter = session.state == DebugState::Stopped
                && (response.has_focus() || response.lost_focus())
                && consume_exact(ui.ctx(), Modifiers::NONE, Key::Enter);
            let evaluate = ui
                .add_enabled(
                    session.state == DebugState::Stopped
                        && self
                            .debugger_ui
                            .expressions
                            .get(&session.id)
                            .is_some_and(|s| !s.trim().is_empty()),
                    egui::Button::new("Evaluate"),
                )
                .clicked();
            if evaluate || enter {
                self.debugger_evaluate(session.id, false);
                if enter {
                    response.request_focus();
                }
            }
        });

        let footer = 35.;
        egui::ScrollArea::both()
            .id_salt(("debug.variables", session.id))
            .max_height((ui.available_height() - footer).max(0.))
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                if session.state != DebugState::Stopped {
                    ui.add_space(8.);
                    ui.label(
                        RichText::new("Variables are available when paused")
                            .color(color(self.tokens.muted)),
                    );
                    return;
                }
                let default_scope = session
                    .scopes
                    .iter()
                    .find(|scope| !scope.expensive)
                    .map(|scope| scope.reference);
                for scope in &session.scopes {
                    let response = egui::CollapsingHeader::new(&scope.name)
                        .id_salt((session.id, session.current_frame, "scope", scope.reference))
                        .default_open(Some(scope.reference) == default_scope)
                        .show(ui, |ui| {
                            self.debugger_variable_children(ui, session, scope.reference, 0)
                        });
                    if response.body_response.is_some() {
                        self.debugger_request_variables(session, scope.reference);
                    }
                }
                if self
                    .debugger_ui
                    .watches
                    .get(&session.id)
                    .is_some_and(|w| !w.is_empty())
                {
                    egui::CollapsingHeader::new("Watches")
                        .id_salt((session.id, "watches"))
                        .default_open(true)
                        .show(ui, |ui| {
                            for expression in self
                                .debugger_ui
                                .watches
                                .get(&session.id)
                                .cloned()
                                .unwrap_or_default()
                            {
                                let evaluation = session
                                    .evaluations
                                    .iter()
                                    .rev()
                                    .find(|e| e.expression == expression);
                                ui.horizontal(|ui| {
                                    if icon_button(
                                        ui,
                                        Icon::Remove,
                                        &format!("Remove Watch {expression}"),
                                        false,
                                        22.,
                                    )
                                    .clicked()
                                        && let Some(watches) =
                                            self.debugger_ui.watches.get_mut(&session.id)
                                    {
                                        watches.retain(|w| w != &expression);
                                    }
                                    let value = evaluation
                                        .map(|e| e.error.as_deref().unwrap_or(&e.result))
                                        .unwrap_or("Evaluating…");
                                    ui.monospace(format!("{expression} = {value}"));
                                });
                                if let Some(evaluation) = evaluation
                                    && evaluation.reference > 0
                                {
                                    self.debugger_variable_children(
                                        ui,
                                        session,
                                        evaluation.reference,
                                        0,
                                    );
                                }
                            }
                        });
                }
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    session.state == DebugState::Stopped
                        && self
                            .debugger_ui
                            .expressions
                            .get(&session.id)
                            .is_some_and(|s| !s.trim().is_empty()),
                    egui::Button::new("Add to Watches"),
                )
                .clicked()
            {
                self.debugger_evaluate(session.id, true);
            }
            if let Some(evaluation) = session.evaluations.last() {
                ui.add_space(8.);
                ui.label(
                    egui::RichText::new(evaluation.error.as_deref().unwrap_or(&evaluation.result))
                        .monospace(),
                )
                .on_hover_text(&evaluation.expression);
            }
        });
    }
    fn debugger_evaluate(&mut self, session: SessionId, watch: bool) {
        if !self
            .service
            .debugger
            .sessions
            .iter()
            .any(|s| s.id == session && s.state == DebugState::Stopped)
        {
            return;
        }
        let expression = self
            .debugger_ui
            .expressions
            .get(&session)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();
        if expression.is_empty() {
            return;
        }
        if watch {
            let watches = self.debugger_ui.watches.entry(session).or_default();
            if !watches.contains(&expression) {
                watches.push(expression.clone());
            }
        }
        self.send(Command::Debug(D::Evaluate {
            session,
            expression,
        }));
    }
    fn debugger_request_variables(&mut self, session: &DebugSession, reference: i64) {
        if reference > 0
            && !session.variables.contains_key(&reference)
            && self
                .debugger_ui
                .requested_variables
                .insert((session.id, reference))
        {
            self.send(Command::Debug(D::Variables {
                session: session.id,
                reference,
            }));
        }
    }
    fn debugger_variable_children(
        &mut self,
        ui: &mut egui::Ui,
        session: &DebugSession,
        reference: i64,
        depth: usize,
    ) {
        if depth > 12 {
            ui.label("Expand limit reached");
            return;
        }
        let Some(variables) = session.variables.get(&reference) else {
            self.debugger_request_variables(session, reference);
            ui.label(
                RichText::new(if session.last_error.is_some() {
                    "Unable to load variables. Select the frame to retry."
                } else {
                    "Loading…"
                })
                .color(color(self.tokens.muted)),
            );
            return;
        };
        for variable in variables {
            let text = format!(
                "{} = {}{}",
                variable.name,
                variable.value,
                variable
                    .type_name
                    .as_ref()
                    .map(|t| format!("  {{{t}}}"))
                    .unwrap_or_default()
            );
            if variable.reference > 0 {
                egui::CollapsingHeader::new(RichText::new(text).monospace())
                    .id_salt((session.id, session.current_frame, reference, &variable.name))
                    .show(ui, |ui| {
                        self.debugger_variable_children(ui, session, variable.reference, depth + 1)
                    });
            } else {
                ui.label(RichText::new(text).monospace());
            }
        }
    }
    fn debugger_console(&mut self, ui: &mut egui::Ui, session: &DebugSession) {
        egui::ScrollArea::both()
            .id_salt(("debug.console", session.id))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                ui.add_space(6.);
                for output in &session.output {
                    let process_output = matches!(output.category.as_str(), "stdout" | "stderr");
                    if process_output != self.debugger_ui.process_console {
                        continue;
                    }
                    ui.label(RichText::new(&output.text).monospace().color(color(
                        if output.category == "stderr" {
                            self.tokens.error
                        } else {
                            self.tokens.text
                        },
                    )));
                }
            });
    }
}
fn state_label(state: DebugState) -> &'static str {
    match state {
        DebugState::Starting => "Starting…",
        DebugState::Running => "Running",
        DebugState::Stopped => "Paused",
        DebugState::Terminating => "Stopping…",
        DebugState::Terminated => "Finished",
        DebugState::Failed => "Failed",
    }
}

#[cfg(test)]
#[path = "debugger_tests.rs"]
mod tests;
