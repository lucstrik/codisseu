//! Native PTY terminal. No shell, filesystem operation or blocking I/O runs on the GUI thread.
use egui::{self, Color32, FontId, Key, Pos2, Rect, Sense, Stroke, Vec2};
use portable_pty::{CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    num::NonZeroU16,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

const SCROLLBACK: usize = 2_000;
const INPUT_LIMIT: usize = 1_048_576;
#[path = "terminal_surface.rs"]
mod surface;
pub(crate) use surface::TerminalUi;
static NEXT_TERMINAL_ID: AtomicU64 = AtomicU64::new(1);

const TERMINAL_ID: &str = "codisseu-native-terminal";
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

fn nz(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value.max(1)).unwrap()
}
fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[derive(Debug)]
enum Status {
    Started,
    Exited(u32),
    Failed(String),
}

type TerminalEnvironment = Vec<(String, Option<String>)>;
type TerminalLaunch = (PathBuf, Vec<String>, TerminalEnvironment);
type WakeContext = Arc<Mutex<Option<egui::Context>>>;
fn wake(context: &WakeContext) {
    let context = context.lock().ok().and_then(|context| context.clone());
    if let Some(context) = context {
        context.request_repaint();
    }
}

struct WorkerCompletion {
    done: Arc<AtomicBool>,
    context: WakeContext,
}
impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        wake(&self.context);
    }
}

struct Session {
    output: Receiver<Vec<u8>>,
    status: Receiver<Status>,
    input: SyncSender<Vec<u8>>,
    resize: SyncSender<PtySize>,
    stop: Arc<AtomicBool>,
    wake_context: WakeContext,
    done: Arc<AtomicBool>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Session {
    fn start(cwd: PathBuf, shell: PathBuf, args: Vec<String>) -> Self {
        Self::start_environment(cwd, shell, args, vec![])
    }
    fn start_environment(
        cwd: PathBuf,
        shell: PathBuf,
        args: Vec<String>,
        environment: TerminalEnvironment,
    ) -> Self {
        let (output_tx, output) = mpsc::sync_channel(32);
        let (status_tx, status) = mpsc::channel();
        let (input, input_rx) = mpsc::sync_channel::<Vec<u8>>(8);
        let (resize, resize_rx) = mpsc::sync_channel::<PtySize>(8);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let wake_context: WakeContext = Arc::new(Mutex::new(None));
        let worker_wake = wake_context.clone();
        let done = Arc::new(AtomicBool::new(false));
        let worker_done = done.clone();
        thread::spawn(move || {
            let _completion = WorkerCompletion {
                done: worker_done,
                context: worker_wake.clone(),
            };
            let result = (|| -> Result<(), String> {
                let pair = portable_pty::native_pty_system()
                    .openpty(size(DEFAULT_ROWS, DEFAULT_COLS))
                    .map_err(|e| e.to_string())?;
                let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
                let mut writer = pair.master.take_writer().map_err(|e| e.to_string())?;
                let mut command = CommandBuilder::new(&shell);
                command.args(args);
                command.cwd(cwd);
                command.env("TERM", "xterm-256color");
                command.env("COLORTERM", "truecolor");
                for (key, value) in environment {
                    if let Some(value) = value {
                        command.env(key, value);
                    } else {
                        command.env_remove(key);
                    }
                }
                let mut child = pair
                    .slave
                    .spawn_command(command)
                    .map_err(|e| format!("{}: {e}", shell.display()))?;
                drop(pair.slave);
                let shell_pid = child.process_id();
                let _ = status_tx.send(Status::Started);
                wake(&worker_wake);
                let output_status = status_tx.clone();
                let output_wake = worker_wake.clone();
                let input_wake = worker_wake.clone();
                thread::spawn(move || {
                    let mut bytes = [0_u8; 8192];
                    loop {
                        match reader.read(&mut bytes) {
                            Ok(0) => break,
                            Ok(n) => {
                                if output_tx.send(bytes[..n].to_vec()).is_err() {
                                    break;
                                }
                                wake(&output_wake);
                            }
                            // Linux PTYs report EIO when the slave closes; process exit is authoritative.
                            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(_) => break,
                        }
                    }
                    drop(output_tx);
                    wake(&output_wake);
                });
                thread::spawn(move || {
                    while let Ok(bytes) = input_rx.recv() {
                        if let Err(e) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                            let _ = output_status
                                .send(Status::Failed(format!("Falha na entrada do terminal: {e}")));
                            wake(&input_wake);
                            break;
                        }
                    }
                });
                loop {
                    if worker_stop.load(Ordering::Acquire) {
                        terminate_groups(pair.master.as_ref(), shell_pid);
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    match child.try_wait() {
                        Ok(Some(exit)) => {
                            terminate_groups(pair.master.as_ref(), shell_pid);
                            let _ = status_tx.send(Status::Exited(exit.exit_code()));
                            wake(&worker_wake);
                            break;
                        }
                        Ok(None) => (),
                        Err(error) => {
                            terminate_groups(pair.master.as_ref(), shell_pid);
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(error.to_string());
                        }
                    }
                    match resize_rx.recv_timeout(Duration::from_millis(15)) {
                        Ok(mut latest) => {
                            while let Ok(newer) = resize_rx.try_recv() {
                                latest = newer;
                            }
                            if let Err(error) = pair.master.resize(latest) {
                                terminate_groups(pair.master.as_ref(), shell_pid);
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(error.to_string());
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            terminate_groups(pair.master.as_ref(), shell_pid);
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => (),
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                let _ = status_tx.send(Status::Failed(error));
                wake(&worker_wake);
            }
        });
        Self {
            output,
            status,
            input,
            resize,
            stop,
            wake_context,
            done,
        }
    }
}

fn terminate_groups(master: &dyn portable_pty::MasterPty, shell_pid: Option<u32>) {
    #[cfg(unix)]
    {
        use nix::{
            sys::signal::{Signal, killpg},
            unistd::Pid,
        };
        #[cfg(target_os = "linux")]
        if let Some(session_id) = shell_pid {
            // The PTY child calls setsid before exec. Include background jobs in this owned
            // session; killing only the foreground group would leak a `sleep ... &` job.
            if let Ok(entries) = std::fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    let Some(pid) = entry
                        .file_name()
                        .to_str()
                        .and_then(|s| s.parse::<i32>().ok())
                    else {
                        continue;
                    };
                    if pid > 1
                        && nix::unistd::getsid(Some(Pid::from_raw(pid)))
                            .is_ok_and(|id| id.as_raw() == session_id as i32)
                    {
                        let _ = nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGKILL);
                    }
                }
            }
        }
        // These IDs come only from this PTY. Foreground jobs can have a different group than the shell.
        if let Some(group) = master.process_group_leader().filter(|p| *p > 1) {
            let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
        }
        if let Some(pid) = shell_pid.filter(|p| *p > 1) {
            let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = (master, shell_pid);
}

/// Persistent terminal session, created explicitly when the user opens the tool window.
/// Hiding the tool window preserves its process. `close`/Drop terminates the owned session.
pub(crate) struct TerminalPane {
    identity: u64,
    splits: Vec<TerminalPane>,
    focused_child: Option<u64>,
    shell: PathBuf,
    session: Option<Session>,
    parser: vt100::Parser,
    dimensions: (u16, u16),
    focus: bool,
    running: bool,
    message: Option<String>,
    selection: Option<(usize, usize)>,
    cwd: PathBuf,
    last_poll_frame: Option<u64>,
    parked: Vec<TerminalPane>,
    label: Option<String>,
    command_key: Option<String>,
    launch: Option<TerminalLaunch>,
    closing: Vec<Arc<AtomicBool>>,
}
impl Default for TerminalPane {
    fn default() -> Self {
        Self {
            identity: NEXT_TERMINAL_ID.fetch_add(1, Ordering::Relaxed),
            splits: Vec::new(),
            focused_child: None,
            shell: PathBuf::new(),
            session: None,
            parser: vt100::Parser::new(nz(DEFAULT_ROWS), nz(DEFAULT_COLS), SCROLLBACK),
            dimensions: (DEFAULT_ROWS, DEFAULT_COLS),
            focus: false,
            running: false,
            message: None,
            selection: None,
            cwd: PathBuf::new(),
            last_poll_frame: None,
            parked: Vec::new(),
            label: None,
            command_key: None,
            launch: None,
            closing: Vec::new(),
        }
    }
}
impl TerminalPane {
    pub(crate) fn open(&mut self, cwd: &Path) {
        if self.session.is_none() {
            self.cwd = cwd.to_path_buf();
            #[cfg(unix)]
            let shell = std::env::var_os("SHELL")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/bin/sh"));
            #[cfg(windows)]
            let shell = std::env::var_os("COMSPEC")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("cmd.exe"));
            #[cfg(not(any(unix, windows)))]
            let shell = PathBuf::from("sh");
            #[cfg(unix)]
            let args = vec!["-i".to_owned()];
            #[cfg(not(unix))]
            let args = vec![];
            self.parser = vt100::Parser::new(nz(DEFAULT_ROWS), nz(DEFAULT_COLS), SCROLLBACK);
            self.dimensions = (DEFAULT_ROWS, DEFAULT_COLS);
            self.shell = shell.clone();
            self.session = Some(Session::start(cwd.to_path_buf(), shell, args));
            self.message = Some("Iniciando terminal…".into());
            self.running = true;
            self.last_poll_frame = None;
        }
        self.request_focus();
    }
    /// Launch an explicit executable in a real PTY. Arguments never pass through
    /// the host shell. Matching live sessions retain input/history on reopening.
    pub(crate) fn open_command(
        &mut self,
        cwd: &Path,
        program: PathBuf,
        args: Vec<String>,
        label: String,
        key: String,
    ) -> bool {
        self.open_command_environment(cwd, program, args, label, key, vec![])
    }
    pub(crate) fn open_command_environment(
        &mut self,
        cwd: &Path,
        program: PathBuf,
        args: Vec<String>,
        label: String,
        key: String,
        environment: TerminalEnvironment,
    ) -> bool {
        if self.command_key.as_ref() == Some(&key) && self.running {
            self.request_focus();
            return true;
        }
        if let Some(index) = self
            .parked
            .iter()
            .position(|p| p.command_key.as_ref() == Some(&key) && p.running)
        {
            self.select_session(index);
            return true;
        }
        if !self.prepare_directory_session() {
            return false;
        }
        self.cwd = cwd.into();
        self.parser = vt100::Parser::new(nz(DEFAULT_ROWS), nz(DEFAULT_COLS), SCROLLBACK);
        self.dimensions = (DEFAULT_ROWS, DEFAULT_COLS);
        self.launch = Some((program.clone(), args.clone(), environment.clone()));
        self.shell = program.clone();
        self.session = Some(Session::start_environment(
            cwd.into(),
            program,
            args,
            environment,
        ));
        self.label = Some(label);
        self.command_key = Some(key);
        self.message = Some("Conectando ao terminal…".into());
        self.running = true;
        self.last_poll_frame = None;
        self.request_focus();
        true
    }
    pub(crate) fn command_running(&self, key: &str) -> bool {
        (self.command_key.as_deref() == Some(key) && self.running)
            || self
                .parked
                .iter()
                .any(|p| p.command_key.as_deref() == Some(key) && p.running)
    }
    pub(crate) fn has_sessions(&self) -> bool {
        self.session.is_some() || !self.parked.is_empty()
    }
    pub(crate) fn session_label(&self) -> String {
        self.label.clone().unwrap_or_else(|| {
            format!(
                "Terminal · {}",
                self.cwd.file_name().unwrap_or_default().to_string_lossy()
            )
        })
    }
    fn restart(&mut self) {
        let cwd = self.cwd.clone();
        let launch = self.launch.clone();
        let label = self.label.clone().unwrap_or_default();
        let key = self.command_key.clone().unwrap_or_default();
        self.close_current();
        if let Some((program, args, environment)) = launch {
            self.open_command_environment(&cwd, program, args, label, key, environment);
        } else {
            self.open(&cwd);
        }
    }
    /// Explorer requests a new PTY: the recorded launch cwd cannot prove where
    /// an existing shell is after `cd`, SSH, or another interactive command.
    pub(crate) fn open_directory(&mut self, cwd: &Path) {
        if self.prepare_directory_session() {
            self.open(cwd);
        }
    }
    fn prepare_directory_session(&mut self) -> bool {
        if self.session.is_none() {
            return true;
        }
        if self.session_count() >= 16 {
            self.message =
                Some("Limite de 16 terminais; feche uma sessão antes de abrir outra.".into());
            return false;
        }
        let mut parked = std::mem::take(&mut self.parked);
        let previous = std::mem::take(self);
        parked.push(previous);
        self.parked = parked;
        true
    }
    fn select_session(&mut self, index: usize) {
        let mut parked = std::mem::take(&mut self.parked);
        let next = parked.swap_remove(index);
        let previous = std::mem::replace(self, next);
        parked.push(previous);
        self.parked = parked;
        self.request_focus();
    }
    pub(crate) fn request_focus(&mut self) {
        self.focus = true;
    }
    pub(crate) fn is_focused(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(self.input_id()))
            || self.splits.iter().any(|p| p.is_focused(ctx))
    }
    fn input_id(&self) -> egui::Id {
        egui::Id::unique((TERMINAL_ID, self.identity))
    }
    pub(crate) fn close(&mut self) {
        for mut pane in std::mem::take(&mut self.splits) {
            pane.close();
            self.closing.append(&mut pane.closing);
        }
        for mut pane in std::mem::take(&mut self.parked) {
            pane.close();
            self.closing.append(&mut pane.closing);
        }
        self.close_current();
    }
    pub(crate) fn shutdown_pending(&self) -> bool {
        self.closing
            .iter()
            .any(|done| !done.load(Ordering::Acquire))
            || self.parked.iter().any(Self::shutdown_pending)
            || self.splits.iter().any(Self::shutdown_pending)
    }
    fn close_current(&mut self) {
        self.closing.retain(|done| !done.load(Ordering::Acquire));
        if let Some(session) = self.session.take() {
            if !session.done.load(Ordering::Acquire) {
                self.closing.push(session.done.clone());
            }
            drop(session);
        }
        self.running = false;
        self.selection = None;
        self.message = None;
    }
    pub(crate) fn close_active_session(&mut self) {
        if !self.splits.is_empty() {
            let mut splits = std::mem::take(&mut self.splits);
            let parked = std::mem::take(&mut self.parked);
            self.close_current();
            let mut next = splits.remove(0);
            next.closing.append(&mut self.closing);
            *self = next;
            self.splits = splits;
            self.parked = parked;
            self.request_focus();
            return;
        }
        self.close_current();
        if !self.parked.is_empty() {
            let mut parked = std::mem::take(&mut self.parked);
            let mut next = parked.remove(0);
            next.closing.append(&mut self.closing);
            *self = next;
            self.parked = parked;
            self.request_focus();
        }
    }
    fn poll(&mut self) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        // Bounded work per frame and bounded output queue; producer receives backpressure.
        for _ in 0..32 {
            match session.output.try_recv() {
                Ok(bytes) => self.parser.process(&bytes),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    break;
                }
            }
        }
        while let Ok(status) = session.status.try_recv() {
            match status {
                Status::Started => self.message = None,
                Status::Exited(code) => {
                    self.running = false;
                    self.message = Some(format!("Processo encerrado (código {code})."));
                }
                Status::Failed(error) => {
                    self.running = false;
                    self.message = Some(error);
                }
            }
        }
    }
    fn send(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() || !self.running {
            return;
        }
        if bytes.len() > INPUT_LIMIT {
            self.message = Some("Colagem excede o limite de 1 MiB do terminal.".into());
            return;
        }
        if let Some(session) = &self.session {
            match session.input.try_send(bytes) {
                Ok(()) => {
                    self.parser.screen_mut().set_scrollback(0);
                    self.selection = None;
                }
                Err(TrySendError::Full(_)) => {
                    self.message =
                        Some("Terminal ocupado; aguarde antes de enviar mais texto.".into())
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.message = Some("A entrada do terminal foi encerrada.".into())
                }
            }
        }
    }
    /// Call every application frame, including when the pane is hidden. Workers wake the
    /// context on output/status changes, so an idle shell never forces continuous redraw.
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        self.closing.retain(|done| !done.load(Ordering::Acquire));
        if self.shutdown_pending() {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
        if let Some(session) = &self.session {
            match session.wake_context.try_lock() {
                Ok(mut wake) => {
                    if wake.is_none() {
                        *wake = Some(ctx.clone());
                    }
                }
                Err(_) => ctx.request_repaint(),
            }
        }
        let frame = ctx.cumulative_frame_nr();
        if self.last_poll_frame != Some(frame) {
            self.last_poll_frame = Some(frame);
            self.poll();
            for pane in self.parked.iter_mut().chain(self.splits.iter_mut()) {
                pane.tick(ctx);
            }
        }
    }
    pub(crate) fn session_picker(&mut self, ui: &mut egui::Ui) {
        if self.parked.is_empty() {
            ui.label(egui::RichText::new(self.session_label()).weak().size(11.));
            return;
        }
        let mut selected = None;
        egui::ComboBox::from_id_salt("terminal.sessions")
            .selected_text(self.session_label())
            .show_ui(ui, |ui| {
                ui.label(self.cwd.display().to_string());
                for (index, pane) in self.parked.iter().enumerate() {
                    if ui.selectable_label(false, pane.session_label()).clicked() {
                        selected = Some(index);
                    }
                }
            })
            .response
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::ComboBox,
                    true,
                    "Sessões de terminal",
                )
            });
        if let Some(index) = selected {
            self.select_session(index);
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui) {
        if !self.parked.is_empty() {
            ui.horizontal(|ui| {
                self.session_picker(ui);
                if ui.small_button("Close Session").clicked() {
                    self.close_active_session();
                }
            });
        }
        self.show_content(ui);
    }
    pub(crate) fn show_content(&mut self, ui: &mut egui::Ui) {
        self.show_screen(ui, false);
    }
    fn show_screen(&mut self, ui: &mut egui::Ui, reference_style: bool) {
        self.tick(ui.ctx());
        if let Some(message) = self.message.clone() {
            ui.horizontal(|ui| {
                crate::status_messages::status_message(ui, message);
                if !self.running && ui.small_button("Reiniciar").clicked() {
                    self.restart();
                }
            });
        }
        let font = if reference_style {
            FontId::new(14., egui::FontFamily::Name("codisseu.terminal.mono".into()))
        } else {
            FontId::monospace(13.)
        };
        let cell_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.);
        let cell_size = egui::vec2(cell_width, if reference_style { 17. } else { 19. });
        let available = ui.available_size().max(egui::vec2(cell_width, 19.));
        let (rect, _) = ui.allocate_exact_size(available, Sense::hover());
        let response = ui.interact(rect, self.input_id(), Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Terminal, true, "Terminal integrado")
        });
        let dimensions = (
            (rect.height() / cell_size.y).floor().clamp(1., 200.) as u16,
            (rect.width() / cell_size.x).floor().clamp(1., 500.) as u16,
        );
        if dimensions != self.dimensions
            && let Some(session) = &self.session
            && session
                .resize
                .try_send(size(dimensions.0, dimensions.1))
                .is_ok()
        {
            self.dimensions = dimensions;
            self.parser.set_size(nz(dimensions.0), nz(dimensions.1));
            self.selection = None;
        }
        let accepts_input = ui.is_enabled() && ui.memory(|m| m.top_modal_layer().is_none());
        if accepts_input && (self.focus || response.clicked() || response.drag_started()) {
            response.request_focus();
            self.focus = false;
        }
        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta.abs() > 0.1 {
                let current = self.parser.screen().scrollback() as isize;
                let movement = (delta / cell_size.y).round() as isize;
                self.parser
                    .screen_mut()
                    .set_scrollback(current.saturating_add(movement).max(0) as usize);
                self.selection = None;
            }
        }
        if let Some(pointer) = response.interact_pointer_pos() {
            let cell = pointer_cell(pointer, rect, cell_size, self.dimensions);
            if response.drag_started() {
                self.selection = Some((cell, cell));
            }
            if response.dragged()
                && let Some((_, end)) = &mut self.selection
            {
                *end = cell;
            }
            if response.clicked() {
                self.selection = None;
            }
        }
        response.context_menu(|ui| {
            if ui.button("Copiar seleção").clicked() {
                ui.ctx().copy_text(self.selected_text());
                ui.close();
            }
            if ui.button("Colar").clicked() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
            if ui.button("Copiar área visível").clicked() {
                ui.ctx().copy_text(self.parser.screen().contents());
                ui.close();
            }
        });
        if response.has_focus() && accepts_input {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            let (row, col) = self.parser.screen().cursor_position();
            ui.output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Terminal,
                    rect,
                    cursor_rect: Rect::from_min_size(
                        rect.min + egui::vec2(col as f32 * cell_size.x, row as f32 * cell_size.y),
                        cell_size,
                    ),
                    should_interrupt_composition: false,
                })
            });
            let modifiers = ui.input(|i| i.modifiers);
            let events = ui.input_mut(|i| std::mem::take(&mut i.events));
            for event in events {
                match &event {
                    // eframe's bridge turns Ctrl+C/X into clipboard events without per-event
                    // modifiers. A quick modifier release can already have changed i.modifiers;
                    // non-macOS Copy/Cut therefore mean terminal Ctrl+C/X. Explicit copy uses
                    // Ctrl+Shift+C or the context menu; macOS Cmd+C remains ordinary copying.
                    egui::Event::Copy
                        if !cfg!(target_os = "macos")
                            || (modifiers.ctrl && !modifiers.shift && !modifiers.mac_cmd) =>
                    {
                        self.send(vec![3])
                    }
                    egui::Event::Cut
                        if !cfg!(target_os = "macos")
                            || (modifiers.ctrl && !modifiers.shift && !modifiers.mac_cmd) =>
                    {
                        self.send(vec![24])
                    }
                    egui::Event::Copy => ui.ctx().copy_text(self.selected_text()),
                    egui::Event::Key {
                        key: Key::V,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.ctrl && modifiers.shift => ui
                        .ctx()
                        .send_viewport_cmd(egui::ViewportCommand::RequestPaste),
                    egui::Event::Key {
                        key: Key::C,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.ctrl && modifiers.shift => {
                        ui.ctx().copy_text(self.selected_text())
                    }
                    _ => {
                        if let Some(bytes) = encode_event(
                            &event,
                            self.parser.screen().application_cursor(),
                            self.parser.screen().bracketed_paste(),
                        ) {
                            self.send(bytes);
                        }
                    }
                }
            }
        }
        self.paint(
            ui,
            rect,
            cell_size,
            font,
            response.has_focus(),
            reference_style,
        );
    }
    fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection else {
            return String::new();
        };
        let (start, end) = (start.min(end), start.max(end));
        let cols = self.dimensions.1 as usize;
        let mut text = String::new();
        for index in start..=end {
            let row = index / cols;
            let col = index % cols;
            if index != start && col == 0 {
                text = text.trim_end_matches(' ').to_owned();
                text.push('\n');
            }
            if let Some(cell) = self.parser.screen().cell(row as u16, col as u16) {
                if cell.has_contents() {
                    text.push_str(cell.contents());
                } else if !cell.is_wide_continuation() {
                    text.push(' ');
                }
            }
        }
        text.trim_end_matches(' ').to_owned()
    }
    fn paint(
        &self,
        ui: &egui::Ui,
        rect: Rect,
        cell_size: Vec2,
        font: FontId,
        focused: bool,
        reference_style: bool,
    ) {
        let painter = ui.painter().with_clip_rect(rect);
        let background = ui.visuals().extreme_bg_color;
        let foreground = ui.visuals().text_color();
        painter.rect_filled(rect, 0., background);
        let screen = self.parser.screen();
        for row in 0..self.dimensions.0 {
            for col in 0..self.dimensions.1 {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                let position =
                    rect.min + egui::vec2(col as f32 * cell_size.x, row as f32 * cell_size.y);
                let bounds = Rect::from_min_size(position, cell_size);
                let mut fg = ansi_color(cell.fgcolor(), foreground);
                let mut bg = ansi_color(cell.bgcolor(), background);
                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.dim() {
                    fg = fg.gamma_multiply(0.65);
                }
                let index = row as usize * self.dimensions.1 as usize + col as usize;
                if self
                    .selection
                    .is_some_and(|(a, b)| index >= a.min(b) && index <= a.max(b))
                {
                    bg = ui.visuals().selection.bg_fill;
                }
                if bg != background {
                    painter.rect_filled(bounds, 0., bg);
                }
                if cell.has_contents() && !cell.is_wide_continuation() {
                    painter.text(
                        position + egui::vec2(0., 1.),
                        egui::Align2::LEFT_TOP,
                        cell.contents(),
                        font.clone(),
                        fg,
                    );
                    if cell.bold() {
                        painter.text(
                            position + egui::vec2(0.35, 1.),
                            egui::Align2::LEFT_TOP,
                            cell.contents(),
                            font.clone(),
                            fg,
                        );
                    }
                    if cell.underline() {
                        painter.line_segment(
                            [
                                bounds.left_bottom() - egui::vec2(0., 2.),
                                bounds.right_bottom() - egui::vec2(0., 2.),
                            ],
                            Stroke::new(1., fg),
                        );
                    }
                }
            }
        }
        if (focused || reference_style)
            && self.running
            && !screen.hide_cursor()
            && screen.scrollback() == 0
        {
            let (row, col) = screen.cursor_position();
            let cursor = Rect::from_min_size(
                rect.min + egui::vec2(col as f32 * cell_size.x, row as f32 * cell_size.y),
                cell_size,
            );
            if reference_style && focused {
                painter.rect_filled(cursor, 0., foreground);
            } else {
                painter.rect_stroke(
                    cursor,
                    0.,
                    Stroke::new(1., foreground),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }
}

fn pointer_cell(pointer: Pos2, rect: Rect, cell: Vec2, dimensions: (u16, u16)) -> usize {
    let local = pointer - rect.min;
    let row = (local.y / cell.y)
        .floor()
        .clamp(0., dimensions.0.saturating_sub(1) as f32) as usize;
    let col = (local.x / cell.x)
        .floor()
        .clamp(0., dimensions.1.saturating_sub(1) as f32) as usize;
    row * dimensions.1 as usize + col
}
fn encode_event(
    event: &egui::Event,
    application_cursor: bool,
    bracketed_paste: bool,
) -> Option<Vec<u8>> {
    match event {
        egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
            Some(text.as_bytes().to_vec())
        }
        egui::Event::Paste(text) => {
            // Do not permit pasted escape characters to close a bracketed paste or inject VT controls.
            let clean = text
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\x1b', "");
            Some(if bracketed_paste {
                format!("\x1b[200~{clean}\x1b[201~").into_bytes()
            } else {
                clean.replace('\n', "\r").into_bytes()
            })
        }
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            if modifiers.ctrl
                && !modifiers.shift
                && let Some(character) = key.name().chars().next()
                && key.name().len() == 1
                && character.is_ascii_alphabetic()
            {
                return Some(vec![character.to_ascii_uppercase() as u8 - b'@']);
            }
            let cursor_prefix = if application_cursor { "\x1bO" } else { "\x1b[" };
            let value = match key {
                Key::Enter => "\r",
                Key::Tab if modifiers.shift => "\x1b[Z",
                Key::Tab => "\t",
                Key::Backspace => "\x7f",
                Key::Escape => "\x1b",
                Key::ArrowUp => return Some(format!("{cursor_prefix}A").into_bytes()),
                Key::ArrowDown => return Some(format!("{cursor_prefix}B").into_bytes()),
                Key::ArrowRight => return Some(format!("{cursor_prefix}C").into_bytes()),
                Key::ArrowLeft => return Some(format!("{cursor_prefix}D").into_bytes()),
                Key::Home => "\x1b[H",
                Key::End => "\x1b[F",
                Key::Insert => "\x1b[2~",
                Key::Delete => "\x1b[3~",
                Key::PageUp => "\x1b[5~",
                Key::PageDown => "\x1b[6~",
                Key::F1 => "\x1bOP",
                Key::F2 => "\x1bOQ",
                Key::F3 => "\x1bOR",
                Key::F4 => "\x1bOS",
                Key::F5 => "\x1b[15~",
                Key::F6 => "\x1b[17~",
                Key::F7 => "\x1b[18~",
                Key::F8 => "\x1b[19~",
                Key::F9 => "\x1b[20~",
                Key::F10 => "\x1b[21~",
                Key::F11 => "\x1b[23~",
                Key::F12 => "\x1b[24~",
                _ => return None,
            };
            Some(value.as_bytes().to_vec())
        }
        _ => None,
    }
}
fn ansi_color(color: vt100::Color, default: Color32) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(index) => {
            const BASIC: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let [r, g, b] = if index < 16 {
                BASIC[index as usize]
            } else if index >= 232 {
                [8 + (index - 232) * 10; 3]
            } else {
                let index = index - 16;
                let component = |n| if n == 0 { 0 } else { 55 + 40 * n };
                [
                    component(index / 36),
                    component(index / 6 % 6),
                    component(index % 6),
                ]
            };
            Color32::from_rgb(r, g, b)
        }
    }
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;

#[cfg(test)]
mod explorer_sessions_tests {
    use super::*;
    fn session(stop: Arc<AtomicBool>) -> Session {
        let (_, output) = mpsc::channel();
        let (_, status) = mpsc::channel();
        let (input, _) = mpsc::sync_channel(1);
        let (resize, _) = mpsc::sync_channel(1);
        Session {
            output,
            status,
            input,
            resize,
            stop,
            wake_context: Arc::new(Mutex::new(None)),
            done: Arc::new(AtomicBool::new(true)),
        }
    }
    #[test]
    fn opening_directory_never_reuses_launch_cwd_and_preserves_previous_sessions() {
        let a = Arc::new(AtomicBool::new(false));
        let b = Arc::new(AtomicBool::new(false));
        let mut pane = TerminalPane {
            session: Some(session(a.clone())),
            cwd: PathBuf::from("/first"),
            ..Default::default()
        };
        pane.parked.push(TerminalPane {
            session: Some(session(b.clone())),
            cwd: PathBuf::from("/second"),
            ..Default::default()
        });
        // Exercise the allocation decision without starting a real shell. A
        // caller can now launch in /first or /second without reusing either PTY.
        assert!(pane.prepare_directory_session());
        assert!(pane.session.is_none());
        assert_eq!(pane.parked.len(), 2);
        assert_eq!(pane.parked[0].cwd, PathBuf::from("/second"));
        assert_eq!(pane.parked[1].cwd, PathBuf::from("/first"));
        assert!(!a.load(Ordering::Acquire));
        assert!(!b.load(Ordering::Acquire));
        pane.close();
        assert!(a.load(Ordering::Acquire));
        assert!(b.load(Ordering::Acquire));
        assert!(pane.parked.is_empty());
    }
    #[test]
    fn directory_session_limit_keeps_all_existing_processes_and_current_selection() {
        let stops: Vec<_> = (0..16).map(|_| Arc::new(AtomicBool::new(false))).collect();
        let mut pane = TerminalPane {
            session: Some(session(stops[0].clone())),
            cwd: PathBuf::from("/current"),
            ..Default::default()
        };
        for stop in &stops[1..] {
            pane.parked.push(TerminalPane {
                session: Some(session(stop.clone())),
                ..Default::default()
            });
        }
        assert!(!pane.prepare_directory_session());
        assert_eq!(pane.cwd, PathBuf::from("/current"));
        assert!(pane.session.is_some());
        assert_eq!(pane.parked.len(), 15);
        assert!(stops.iter().all(|stop| !stop.load(Ordering::Acquire)));
        pane.close();
        assert!(stops.iter().all(|stop| stop.load(Ordering::Acquire)));
    }
}

#[cfg(test)]
#[path = "terminal_multi_project_tests.rs"]
mod multi_project_tests;
