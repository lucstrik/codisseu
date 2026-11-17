//! No filesystem/process/socket work happens in BrowserService::dispatch or the GUI.
use crate::{
    Command, EngineState, Event, Frame, protocol,
    service::{EngineConfig, Envelope, Mailbox, Request, wake},
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
pub(crate) const FRAME_BUDGET: usize = 64 * 1024 * 1024;
fn status(mailbox: &Arc<Mutex<Mailbox>>, epoch: u64, state: EngineState, error: Option<String>) {
    if let Ok(mut m) = mailbox.lock()
        && m.epoch == epoch
    {
        m.status = Some((state, error));
        if matches!(state, EngineState::Stopped | EngineState::Failed) {
            m.frames.clear();
            m.states.clear();
            if state == EngineState::Stopped {
                m.pid = None;
            }
            crate::cdp::clear(&mut m, "Motor do navegador indisponível");
        }
    }
    wake(mailbox);
}
pub(crate) fn store_popup(m: &Arc<Mutex<Mailbox>>, epoch: u64, id: u64, url: String) {
    if let Ok(mut m) = m.lock()
        && m.epoch == epoch
        && m.popups.len() < 8
        && m.wanted.contains_key(&(id, crate::Surface::Page))
    {
        m.popups.push((id, url));
    }
}
pub(crate) fn take_popups(m: &Arc<Mutex<Mailbox>>, epoch: u64) -> Vec<(u64, String)> {
    m.lock()
        .ok()
        .filter(|m| m.epoch == epoch)
        .map(|mut m| std::mem::take(&mut m.popups))
        .unwrap_or_default()
}
#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs,
        os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
        process::{Child, Command as Process, Stdio},
        sync::mpsc::{self},
    };
    struct Outbound {
        command: Command,
        payload: Vec<u8>,
        _permit: Option<crate::cdp::Permit>,
    }
    impl Outbound {
        fn plain(command: Command) -> Self {
            Self {
                command,
                payload: vec![],
                _permit: None,
            }
        }
    }
    struct Session {
        epoch: u64,
        child: Child,
        dir: Option<tempfile::TempDir>,
        listener: UnixListener,
        socket: Option<UnixStream>,
        writer: Option<SyncSender<Outbound>>,
        threads: Vec<thread::JoinHandle<()>>,
        ready: Arc<AtomicBool>,
        owners: Arc<Mutex<BTreeSet<u64>>>,
        had_open: bool,
        failure: Arc<Mutex<Option<String>>>,
        pending: VecDeque<Outbound>,
        started: Instant,
        stopping: Option<Instant>,
        failed: bool,
        failure_message: Option<String>,
        stderr: Arc<Mutex<VecDeque<u8>>>,
        cleaned: bool,
    }
    impl Session {
        fn start(epoch: u64, config: Option<EngineConfig>) -> Result<Self, String> {
            let config = match config {
                Some(c) => c,
                None => {
                    let (program, runtime_dir) = codisseu_runtime::browser_engine()?;
                    EngineConfig {
                        program,
                        runtime_dir,
                    }
                }
            };
            let program = config
                .program
                .canonicalize()
                .map_err(|e| format!("Motor do navegador indisponível: {e}"))?;
            let runtime = config
                .runtime_dir
                .canonicalize()
                .map_err(|e| format!("Runtime do navegador indisponível: {e}"))?;
            if !program.is_file() || !runtime.is_dir() {
                return Err("Motor ou runtime do navegador inválido".into());
            }
            let dir = tempfile::Builder::new()
                .prefix("codisseu-browser-")
                .tempdir()
                .map_err(|e| e.to_string())?;
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            let socket = dir.path().join("channel.sock");
            let listener =
                UnixListener::bind(&socket).map_err(|e| format!("IPC do navegador: {e}"))?;
            fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
            listener.set_nonblocking(true).map_err(|e| e.to_string())?;
            let profile = dir.path().join("profile");
            fs::create_dir(&profile).map_err(|e| e.to_string())?;
            fs::set_permissions(&profile, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
            let mut command = Process::new(&program);
            command
                .args(["--socket"])
                .arg(&socket)
                .arg("--runtime")
                .arg(&runtime)
                .arg("--profile")
                .arg(&profile)
                .arg("--parent-pid")
                .arg(std::process::id().to_string())
                .current_dir(&runtime)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .process_group(0)
                .env_clear();
            for key in [
                "PATH",
                "DISPLAY",
                "XAUTHORITY",
                "WAYLAND_DISPLAY",
                "XDG_RUNTIME_DIR",
                "XDG_DATA_DIRS",
                "FONTCONFIG_PATH",
                "LANG",
                "LC_ALL",
                "SYSTEMROOT",
                "TZ",
                "LIBGL_DRIVERS_PATH",
                "LIBGL_ALWAYS_SOFTWARE",
                "GALLIUM_DRIVER",
                "VK_ICD_FILENAMES",
                "VK_DRIVER_FILES",
                "MESA_LOADER_DRIVER_OVERRIDE",
                "DRI_PRIME",
                "__NV_PRIME_RENDER_OFFLOAD",
                "__GLX_VENDOR_LIBRARY_NAME",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
            command
                .env("HOME", &profile)
                .env("XDG_CONFIG_HOME", profile.join("config"))
                .env("XDG_CACHE_HOME", profile.join("cache"))
                .env("XDG_DATA_HOME", profile.join("data"))
                .env("MESA_SHADER_CACHE_DIR", profile.join("cache/mesa"))
                .env("TMPDIR", dir.path())
                .env("CODISSEU_BROWSER_EXECUTABLE", &program);
            let child = command
                .spawn()
                .map_err(|e| format!("Não foi possível iniciar motor do navegador: {e}"))?;
            let mut session = Self {
                epoch,
                child,
                dir: Some(dir),
                listener,
                socket: None,
                writer: None,
                threads: vec![],
                ready: Arc::new(AtomicBool::new(false)),
                owners: Arc::new(Mutex::new(BTreeSet::new())),
                had_open: false,
                failure: Arc::new(Mutex::new(None)),
                pending: VecDeque::new(),
                started: Instant::now(),
                stopping: None,
                failed: false,
                failure_message: None,
                stderr: Arc::new(Mutex::new(VecDeque::new())),
                cleaned: false,
            };
            if let Some(mut pipe) = session.child.stderr.take() {
                let output = session.stderr.clone();
                match thread::Builder::new()
                    .name("codisseu-browser-stderr".into())
                    .spawn(move || {
                        use std::io::Read;
                        let mut bytes = [0u8; 2048];
                        while let Ok(n) = pipe.read(&mut bytes) {
                            if n == 0 {
                                break;
                            }
                            if let Ok(mut ring) = output.lock() {
                                for byte in &bytes[..n] {
                                    if ring.len() == 16384 {
                                        ring.pop_front();
                                    }
                                    ring.push_back(*byte);
                                }
                            }
                        }
                    }) {
                    Ok(task) => session.threads.push(task),
                    Err(e) => {
                        session.cleanup();
                        return Err(format!("Leitor do motor indisponível: {e}"));
                    }
                }
            }
            Ok(session)
        }
        fn connect(
            &mut self,
            tx: &SyncSender<Envelope>,
            mailbox: &Arc<Mutex<Mailbox>>,
        ) -> Result<(), String> {
            if self.socket.is_some() {
                return Ok(());
            }
            let stream = match self.listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(format!("IPC do navegador: {e}")),
            };
            if !peer_is_child(&stream, self.child.id())? {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                return Ok(());
            }
            let mut input = stream.try_clone().map_err(|e| e.to_string())?;
            let mut output = stream.try_clone().map_err(|e| e.to_string())?;
            let (send, receive) = mpsc::sync_channel::<Outbound>(64);
            let failure = self.failure.clone();
            self.threads.push(
                thread::Builder::new()
                    .name("codisseu-browser-writer".into())
                    .spawn(move || {
                        while let Ok(command) = receive.recv() {
                            if let Err(e) = protocol::write_command_packet(
                                &mut output,
                                &command.command,
                                &command.payload,
                            ) {
                                if let Ok(mut f) = failure.lock() {
                                    *f = Some(e);
                                }
                                break;
                            }
                        }
                    })
                    .map_err(|e| e.to_string())?,
            );
            let ready = self.ready.clone();
            let failure = self.failure.clone();
            let tx = tx.clone();
            let mailbox = mailbox.clone();
            let epoch = self.epoch;
            let owners = self.owners.clone();
            self.threads.push(
                thread::Builder::new()
                    .name("codisseu-browser-reader".into())
                    .spawn(move || {
                        let result = (|| -> Result<(), String> {
                            loop {
                                let packet = protocol::read_event(&mut input)?;
                                match packet.message {
                                    Event::Ready { protocol, versions } => {
                                        if ready.swap(true, Ordering::AcqRel) {
                                            return Err("Motor enviou Ready duplicado".into());
                                        }
                                        tx.try_send(Envelope {
                                            epoch,
                                            event: Event::Ready { protocol, versions },
                                        })
                                        .map_err(|_| "Fila de eventos do navegador cheia")?;
                                        wake(&mailbox);
                                    }
                                    Event::DevToolsMessage {
                                        id,
                                        epoch: debug_epoch,
                                    } => {
                                        let json = String::from_utf8(packet.payload)
                                            .map_err(|_| "CDP JSON não UTF8")?;
                                        let delivered = mailbox.lock().is_ok_and(|mut m| {
                                            m.epoch == epoch
                                                && crate::cdp::deliver(
                                                    &mut m,
                                                    id,
                                                    debug_epoch,
                                                    json,
                                                )
                                        });
                                        let _ = delivered;
                                    }
                                    Event::DevToolsDetached {
                                        id,
                                        epoch: debug_epoch,
                                        error,
                                    } => {
                                        if let Ok(mut m) = mailbox.lock()
                                            && m.epoch == epoch
                                            && m.subscriptions
                                                .get(&id)
                                                .is_some_and(|s| s.epoch == debug_epoch)
                                        {
                                            crate::cdp::close(&mut m, id, error);
                                        }
                                    }
                                    Event::Frame {
                                        id,
                                        surface,
                                        width,
                                        height,
                                        seq,
                                    } => {
                                        let wanted = mailbox.lock().ok().and_then(|m| {
                                            if m.epoch == epoch {
                                                m.wanted.get(&(id, surface)).copied()
                                            } else {
                                                None
                                            }
                                        });
                                        let Some(wanted) =
                                            wanted.filter(|d| d.visible && seq > d.seq)
                                        else {
                                            continue;
                                        };
                                        let (w, h) = protocol::physical_size(
                                            wanted.width,
                                            wanted.height,
                                            wanted.scale,
                                        )?;
                                        if width.abs_diff(w) > 1 || height.abs_diff(h) > 1 {
                                            continue;
                                        }
                                        let bgra: Arc<[u8]> = packet.payload.into();
                                        let mut rgba = bgra.to_vec();
                                        for pixel in rgba.chunks_exact_mut(4) {
                                            pixel.swap(0, 2);
                                        }
                                        let frame = Frame {
                                            width,
                                            height,
                                            seq,
                                            bgra,
                                            rgba: rgba.into(),
                                        };
                                        if let Ok(mut m) = mailbox.lock() {
                                            if m.epoch != epoch
                                                || !m.wanted.get(&(id, surface)).is_some_and(|d| {
                                                    d.visible
                                                        && seq > d.seq
                                                        && protocol::physical_size(
                                                            d.width, d.height, d.scale,
                                                        )
                                                        .is_ok_and(|(w, h)| {
                                                            width.abs_diff(w) <= 1
                                                                && height.abs_diff(h) <= 1
                                                        })
                                                })
                                            {
                                                continue;
                                            }
                                            m.frames.remove(&(id, surface));
                                            while m
                                                .frames
                                                .values()
                                                .map(|f| f.bgra.len() + f.rgba.len())
                                                .sum::<usize>()
                                                + frame.bgra.len()
                                                + frame.rgba.len()
                                                > FRAME_BUDGET
                                            {
                                                let Some(key) = m.frames.keys().next().copied()
                                                else {
                                                    break;
                                                };
                                                m.frames.remove(&key);
                                            }
                                            m.wanted.get_mut(&(id, surface)).unwrap().seq = seq;
                                            m.frames.insert((id, surface), frame);
                                        }
                                        wake(&mailbox);
                                    }
                                    event @ Event::State { id, .. } => {
                                        if let Ok(mut m) = mailbox.lock()
                                            && m.epoch == epoch
                                            && m.wanted.contains_key(&(id, crate::Surface::Page))
                                        {
                                            m.states.insert(id, event);
                                        }
                                        wake(&mailbox);
                                    }
                                    event => {
                                        if let Event::Closed { id } = &event
                                            && let Ok(mut ids) = owners.lock()
                                        {
                                            ids.remove(id);
                                        }
                                        let valid = mailbox.lock().is_ok_and(|m| {
                                            m.epoch == epoch
                                                && match &event {
                                                    Event::Closed { id }
                                                    | Event::DevToolsClosed { id }
                                                    | Event::Popup { id, .. } => m
                                                        .wanted
                                                        .contains_key(&(*id, crate::Surface::Page)),
                                                    Event::Error { id: Some(id), .. } => m
                                                        .wanted
                                                        .contains_key(&(*id, crate::Surface::Page)),
                                                    Event::Error { id: None, .. } => true,
                                                    _ => false,
                                                }
                                        });
                                        if valid {
                                            if let Event::Closed { id } = &event
                                                && let Ok(mut m) = mailbox.lock()
                                            {
                                                m.wanted.retain(|(owner, _), _| owner != id);
                                                m.frames.retain(|(owner, _), _| owner != id);
                                                crate::cdp::close(
                                                    &mut m,
                                                    *id,
                                                    Some("Aba encerrada pelo motor".into()),
                                                );
                                            }
                                            if let Event::DevToolsClosed { id } = &event
                                                && let Ok(mut m) = mailbox.lock()
                                            {
                                                if let Some(d) = m
                                                    .wanted
                                                    .get_mut(&(*id, crate::Surface::DevTools))
                                                {
                                                    d.visible = false;
                                                }
                                                m.frames.remove(&(*id, crate::Surface::DevTools));
                                            }
                                            if let Event::Error {
                                                fatal: true,
                                                message,
                                                ..
                                            } = &event
                                            {
                                                return Err(message.clone());
                                            }
                                            tx.try_send(Envelope { epoch, event }).map_err(
                                                |_| "Fila de eventos do navegador cheia",
                                            )?;
                                            wake(&mailbox);
                                        }
                                    }
                                }
                            }
                        })();
                        if let Err(e) = result {
                            if let Ok(mut failure) = failure.lock() {
                                *failure = Some(e);
                            }
                            wake(&mailbox);
                        }
                    })
                    .map_err(|e| e.to_string())?,
            );
            self.socket = Some(stream);
            self.writer = Some(send);
            Ok(())
        }
        fn track(&mut self, command: &Command) {
            match command {
                Command::Open { id, .. } => {
                    self.had_open = true;
                    if let Ok(mut ids) = self.owners.lock() {
                        ids.insert(*id);
                    }
                }
                Command::Close { id } => {
                    if let Ok(mut ids) = self.owners.lock() {
                        ids.remove(id);
                    }
                }
                _ => {}
            }
        }
        fn stop(&mut self) {
            if self.stopping.is_none() {
                self.stopping = Some(Instant::now());
                self.pending.clear();
                if let Some(writer) = &self.writer {
                    let _ = writer.try_send(Outbound::plain(Command::Shutdown));
                } else {
                    kill_group(self.child.id());
                    let _ = self.child.kill();
                }
            }
        }
        fn step(&mut self, tx: &SyncSender<Envelope>, mailbox: &Arc<Mutex<Mailbox>>) -> bool {
            if mailbox
                .lock()
                .is_ok_and(|m| m.epoch == self.epoch && m.shutdown_requested)
            {
                self.stop();
            }
            if self.stopping.is_none() {
                self.flush_cdp_cleanup(mailbox);
            }
            if self.stopping.is_none()
                && self.had_open
                && self.owners.lock().is_ok_and(|s| s.is_empty())
            {
                self.stop();
            }
            if self.stopping.is_none() {
                let failure = self.failure.lock().ok().and_then(|mut e| e.take());
                let error = failure
                    .or_else(|| self.connect(tx, mailbox).err())
                    .or_else(|| {
                        (!self.ready.load(Ordering::Acquire)
                            && self.started.elapsed() > Duration::from_secs(5))
                        .then(|| "Motor do navegador não ficou pronto em 5 segundos".into())
                    });
                if let Some(error) = error {
                    self.failed = true;
                    self.failure_message = Some(error.clone());
                    status(
                        mailbox,
                        self.epoch,
                        EngineState::Failed,
                        Some(self.diagnostic(&error)),
                    );
                    self.stop();
                } else if self.ready.load(Ordering::Acquire)
                    && let Some(writer) = &self.writer
                {
                    while let Some(command) = self.pending.pop_front() {
                        if let Command::DevToolsSubscribe { id, epoch }
                        | Command::DevToolsMessage { id, epoch } = &command.command
                        {
                            let (active, cleanup) = mailbox
                                .lock()
                                .map(|m| {
                                    (
                                        m.subscriptions.get(id).is_some_and(|s| s.epoch == *epoch),
                                        m.pending_cdp_cleanup.contains_key(id),
                                    )
                                })
                                .unwrap_or((false, true));
                            if !active {
                                continue;
                            }
                            if cleanup {
                                self.pending.push_front(command);
                                break;
                            }
                        }
                        match writer.try_send(command) {
                            Ok(()) => {}
                            Err(mpsc::TrySendError::Full(command)) => {
                                self.pending.push_front(command);
                                break;
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => {
                                self.failed = true;
                                self.failure_message = Some("Canal do motor encerrado".into());
                                self.stop();
                                break;
                            }
                        }
                    }
                }
            }
            let exited = match self.child.try_wait() {
                Ok(Some(result)) => {
                    if self.stopping.is_none() {
                        self.failed = true;
                        let message =
                            format!("Motor do navegador encerrou inesperadamente: {result}");
                        self.failure_message = Some(message.clone());
                        status(
                            mailbox,
                            self.epoch,
                            EngineState::Failed,
                            Some(self.diagnostic(&message)),
                        );
                    }
                    true
                }
                Ok(None) => false,
                Err(e) => {
                    self.failed = true;
                    let message = format!("Estado do motor indisponível: {e}");
                    self.failure_message = Some(message.clone());
                    status(
                        mailbox,
                        self.epoch,
                        EngineState::Failed,
                        Some(self.diagnostic(&message)),
                    );
                    true
                }
            };
            if self
                .stopping
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(4))
            {
                kill_group(self.child.id());
                let _ = self.child.kill();
            }
            if exited {
                self.cleanup();
                if let Ok(mut m) = mailbox.lock()
                    && m.epoch == self.epoch
                {
                    m.pid = None;
                }
                if self.failed
                    && let Some(message) = &self.failure_message
                {
                    status(
                        mailbox,
                        self.epoch,
                        EngineState::Failed,
                        Some(self.diagnostic(message)),
                    );
                }
                if !self.failed {
                    status(mailbox, self.epoch, EngineState::Stopped, None);
                }
                return true;
            }
            false
        }
        fn flush_cdp_cleanup(&mut self, mailbox: &Arc<Mutex<Mailbox>>) {
            let cleanup = mailbox
                .lock()
                .ok()
                .filter(|m| m.epoch == self.epoch)
                .map(|m| m.pending_cdp_cleanup.clone())
                .unwrap_or_default();
            for (id, epoch) in cleanup {
                self.pending.retain(|p| !matches!(&p.command,
                    Command::DevToolsSubscribe{id:owner,epoch:old} | Command::DevToolsMessage{id:owner,epoch:old}
                    if *owner == id && *old == epoch));
                if let Some(writer) = &self.writer
                    && writer
                        .try_send(Outbound::plain(Command::DevToolsUnsubscribe { id, epoch }))
                        .is_ok()
                    && let Ok(mut m) = mailbox.lock()
                    && m.epoch == self.epoch
                    && m.pending_cdp_cleanup.get(&id) == Some(&epoch)
                {
                    m.pending_cdp_cleanup.remove(&id);
                }
            }
        }
        fn diagnostic(&self, message: &str) -> String {
            let bytes = self
                .stderr
                .lock()
                .map(|r| r.iter().copied().collect::<Vec<_>>())
                .unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes);
            let detail = text
                .chars()
                .rev()
                .take(2048)
                .collect::<String>()
                .chars()
                .rev()
                .map(|c| {
                    if c.is_control() && c != '\n' && c != '\t' {
                        ' '
                    } else {
                        c
                    }
                })
                .collect::<String>();
            if detail.trim().is_empty() {
                message.into()
            } else {
                format!("{message} · {}", detail.trim())
            }
        }
        fn cleanup(&mut self) {
            if self.cleaned {
                return;
            }
            self.cleaned = true;
            kill_group(self.child.id());
            if let Some(socket) = self.socket.take() {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
            self.writer.take();
            let _ = self.child.kill();
            let _ = self.child.wait();
            for thread in self.threads.drain(..) {
                let _ = thread.join();
            }
            if let Some(dir) = self.dir.take()
                && let Err(e) = dir.close()
            {
                self.failed = true;
                self.failure_message = Some(format!(
                    "Perfil temporário do navegador não pôde ser removido: {e}"
                ));
            }
        }
    }
    impl Drop for Session {
        fn drop(&mut self) {
            self.cleanup();
        }
    }
    fn kill_group(pid: u32) {
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(target_os = "linux")]
    fn peer_is_child(socket: &UnixStream, pid: u32) -> Result<bool, String> {
        use std::os::fd::AsRawFd;
        let mut credential: libc::ucred = unsafe { std::mem::zeroed() };
        let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credential as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        if result != 0 {
            return Err("Não foi possível autenticar processo do navegador".into());
        }
        Ok(credential.pid == pid as i32 && credential.uid == unsafe { libc::geteuid() })
    }
    #[cfg(not(target_os = "linux"))]
    fn peer_is_child(_socket: &UnixStream, _pid: u32) -> Result<bool, String> {
        Err("Autenticação PID do navegador ainda não implementada nesta plataforma".into())
    }
    pub(crate) fn run(
        rx: Receiver<Request>,
        tx: SyncSender<Envelope>,
        mailbox: Arc<Mutex<Mailbox>>,
        stop: Arc<AtomicBool>,
    ) {
        let mut session: Option<Session> = None;
        loop {
            let received = if session.as_ref().is_some_and(|s| s.pending.len() >= 64) {
                thread::sleep(Duration::from_millis(5));
                Err(mpsc::RecvTimeoutError::Timeout)
            } else if session.is_none() {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            } else {
                rx.recv_timeout(Duration::from_millis(10))
            };
            let request = match received {
                Ok(r) => Some(r),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(_) => {
                    stop.store(true, Ordering::Release);
                    None
                }
            };
            if let Some(request) = request {
                if request.start {
                    // Explicit recoveries arrive after a failed process was reaped; keep GUI
                    // nonblocking even if the previous crash cleanup is still completing.
                    if let Some(mut old) = session.take() {
                        old.stop();
                        old.cleanup();
                    }
                    match Session::start(request.epoch, request.config) {
                        Ok(mut s) => {
                            if request.bootstrap.is_empty() {
                                s.track(&request.command);
                                s.pending.push_back(Outbound {
                                    command: request.command,
                                    payload: request.payload,
                                    _permit: request.permit,
                                });
                            } else {
                                for command in request.bootstrap {
                                    s.track(&command);
                                    s.pending.push_back(Outbound::plain(command));
                                }
                            }
                            if let Ok(mut m) = mailbox.lock()
                                && m.epoch == request.epoch
                            {
                                m.pid = Some(s.child.id());
                            }
                            session = Some(s);
                        }
                        Err(e) => status(&mailbox, request.epoch, EngineState::Failed, Some(e)),
                    }
                } else if let Some(s) = &mut session
                    && request.epoch == s.epoch
                {
                    if request.last_close || matches!(request.command, Command::Shutdown) {
                        s.stop();
                    } else if s.stopping.is_none() {
                        if s.pending.len() < 64 {
                            let allowed = match &request.command {
                                Command::DevToolsMessage { id, epoch }
                                | Command::DevToolsSubscribe { id, epoch } => {
                                    mailbox.lock().is_ok_and(|m| {
                                        m.epoch == request.epoch
                                            && m.subscriptions
                                                .get(id)
                                                .is_some_and(|sub| sub.epoch == *epoch)
                                    })
                                }
                                _ => true,
                            };
                            if allowed
                                && !matches!(&request.command, Command::DevToolsUnsubscribe { .. })
                            {
                                s.track(&request.command);
                                s.pending.push_back(Outbound {
                                    command: request.command,
                                    payload: request.payload,
                                    _permit: request.permit,
                                });
                            }
                        } else {
                            s.failed = true;
                            status(
                                &mailbox,
                                s.epoch,
                                EngineState::Failed,
                                Some("Fila do navegador cheia".into()),
                            );
                            s.stop();
                        }
                    }
                }
            }
            if stop.load(Ordering::Acquire) {
                if let Some(s) = &mut session {
                    s.stop();
                } else {
                    break;
                }
            }
            if let Some(s) = &mut session
                && s.step(&tx, &mailbox)
            {
                session = None;
            }
            if stop.load(Ordering::Acquire) && session.is_none() {
                break;
            }
        }
    }
}
#[cfg(unix)]
pub(crate) use unix::run;
#[cfg(not(unix))]
pub(crate) fn run(
    rx: Receiver<Request>,
    _tx: SyncSender<Envelope>,
    mailbox: Arc<Mutex<Mailbox>>,
    _stop: Arc<AtomicBool>,
) {
    while let Ok(r) = rx.recv() {
        status(
            &mailbox,
            r.epoch,
            EngineState::Failed,
            Some("Navegador integrado ainda exige Unix/Linux".into()),
        );
    }
}
