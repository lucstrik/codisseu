use crate::{BrowserId, Command, EngineVersions, Surface, protocol, worker};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub program: PathBuf,
    pub runtime_dir: PathBuf,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum EngineState {
    #[default]
    Stopped,
    Starting,
    Ready,
    Stopping,
    Failed,
}
#[derive(Clone, Debug)]
pub struct TabState {
    pub id: BrowserId,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub error: Option<String>,
    pub devtools_open: bool,
    /// Debug subscription owning the last successfully enqueued frontend.
    pub frontend_epoch: u64,
    pub visible: bool,
    pub devtools_visible: bool,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub seq: u64,
    pub bgra: Arc<[u8]>,
    pub rgba: Arc<[u8]>,
}
pub(crate) type Waker = Arc<dyn Fn() + Send + Sync>;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Desired {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub visible: bool,
    pub focus: bool,
    pub seq: u64,
}
impl Default for Desired {
    fn default() -> Self {
        Self {
            width: 800,
            height: 600,
            scale: 1.,
            visible: false,
            focus: false,
            seq: 0,
        }
    }
}
#[derive(Default)]
pub(crate) struct Mailbox {
    pub epoch: u64,
    pub wanted: BTreeMap<(BrowserId, Surface), Desired>,
    pub frames: BTreeMap<(BrowserId, Surface), Frame>,
    pub states: BTreeMap<BrowserId, crate::Event>,
    pub status: Option<(EngineState, Option<String>)>,
    pub waker: Option<Waker>,
    pub popups: Vec<(BrowserId, String)>,
    pub subscriptions: BTreeMap<BrowserId, crate::cdp::Subscription>,
    pub last_cdp_epoch: BTreeMap<BrowserId, u64>,
    pub pid: Option<u32>,
    pub pending_cdp_cleanup: BTreeMap<BrowserId, u64>,
    pub shutdown_requested: bool,
}
pub(crate) fn wake(mailbox: &Arc<Mutex<Mailbox>>) {
    let waker = mailbox.lock().ok().and_then(|m| m.waker.clone());
    if let Some(waker) = waker {
        waker();
    }
}
pub(crate) struct Request {
    pub epoch: u64,
    pub command: Command,
    pub start: bool,
    pub config: Option<EngineConfig>,
    pub last_close: bool,
    pub bootstrap: Vec<Command>,
    pub payload: Vec<u8>,
    pub permit: Option<crate::cdp::Permit>,
}
pub(crate) struct Envelope {
    pub epoch: u64,
    pub event: crate::Event,
}
pub struct BrowserService {
    pub state: EngineState,
    pub tabs: BTreeMap<BrowserId, TabState>,
    pub frames: BTreeMap<(BrowserId, Surface), Frame>,
    pub versions: Option<EngineVersions>,
    pub last_error: Option<String>,
    config: Option<EngineConfig>,
    pub(crate) send: SyncSender<Request>,
    receive: Receiver<Envelope>,
    pub(crate) mailbox: Arc<Mutex<Mailbox>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
    pub(crate) epoch: u64,
    last_id: u64,
    #[cfg(test)]
    before_start_commit: Option<Box<dyn FnOnce() + Send>>,
    #[cfg(test)]
    after_start_commit: Option<Box<dyn FnOnce() + Send>>,
}
impl Default for BrowserService {
    fn default() -> Self {
        Self::new()
    }
}
impl BrowserService {
    pub fn new() -> Self {
        let (send, rx) = mpsc::sync_channel(64);
        let (tx, receive) = mpsc::sync_channel(64);
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let m = mailbox.clone();
        let signal = stop.clone();
        let spawned = thread::Builder::new()
            .name("codisseu-browser-broker".into())
            .spawn(move || worker::run(rx, tx, m, signal));
        let (worker, state, last_error) = match spawned {
            Ok(w) => (Some(w), EngineState::Stopped, None),
            Err(e) => (
                None,
                EngineState::Failed,
                Some(format!(
                    "Não foi possível iniciar serviço do navegador: {e}"
                )),
            ),
        };
        Self {
            state,
            tabs: BTreeMap::new(),
            frames: BTreeMap::new(),
            versions: None,
            last_error,
            config: None,
            send,
            receive,
            mailbox,
            stop,
            worker,
            epoch: 0,
            last_id: 0,
            #[cfg(test)]
            before_start_commit: None,
            #[cfg(test)]
            after_start_commit: None,
        }
    }
    pub fn set_waker(&mut self, waker: Waker) {
        if let Ok(mut m) = self.mailbox.lock() {
            m.waker = Some(waker);
        }
    }
    pub fn configure(&mut self, config: EngineConfig) -> Result<(), String> {
        if matches!(
            self.state,
            EngineState::Starting | EngineState::Ready | EngineState::Stopping
        ) {
            return Err("Feche o navegador antes de trocar o motor".into());
        }
        if !config.program.is_absolute() || !config.runtime_dir.is_absolute() {
            return Err("Motor e runtime precisam de caminhos absolutos".into());
        }
        self.config = Some(config);
        Ok(())
    }
    /// Called only after explicit user F12/Inspect and an authenticated page gateway.
    pub fn open_devtools_frontend(&mut self, id: u64, url: crate::SecretUrl) -> Result<(), String> {
        if self.state != EngineState::Ready || !self.tabs.contains_key(&id) {
            return Err("Navegador indisponível para DevTools".into());
        }
        let frontend_epoch = self
            .mailbox
            .lock()
            .ok()
            .and_then(|m| m.subscriptions.get(&id).map(|s| s.epoch))
            .ok_or("DevTools requer um canal autorizado ativo")?;
        let command = Command::DevToolsFrontend { id, url };
        protocol::validate_command(&command)?;
        self.send
            .try_send(Request {
                epoch: self.epoch,
                command,
                start: false,
                config: None,
                last_close: false,
                bootstrap: vec![],
                payload: vec![],
                permit: None,
            })
            .map_err(|_| "Fila do navegador cheia ou encerrada".to_owned())?;
        let tab = self.tabs.get_mut(&id).unwrap();
        tab.devtools_open = true;
        tab.frontend_epoch = frontend_epoch;
        Ok(())
    }
    pub fn dispatch(&mut self, mut command: Command) -> Result<(), String> {
        match &mut command {
            Command::Open { url, .. } | Command::Navigate { url, .. } => {
                *url = crate::normalize_url(url)?
            }
            _ => {}
        }
        protocol::validate_command(&command)?;
        if matches!(
            &command,
            Command::Clipboard {
                action: crate::ClipboardAction::Paste,
                ..
            }
        ) {
            return Err("Colar exige texto literal fornecido pelo backend da interface".into());
        }
        if matches!(
            &command,
            Command::DevToolsFrontend { .. }
                | Command::DevToolsSubscribe { .. }
                | Command::DevToolsMessage { .. }
                | Command::DevToolsUnsubscribe { .. }
        ) {
            return Err("Use o canal DevTools autorizado para comandos CDP".into());
        }
        if matches!(self.state, EngineState::Stopping)
            && !matches!(&command, Command::Shutdown | Command::Close { .. })
        {
            return Err("Aguarde o encerramento do motor do navegador".into());
        }
        let open = matches!(&command, Command::Open { .. });
        let last_close = matches!(&command, Command::Close { .. }) && self.tabs.len() == 1;
        if let Command::Open { id, .. } = &command {
            if self.tabs.len() >= protocol::MAX_TABS {
                return Err("Limite de 8 abas do navegador".into());
            }
            if *id <= self.last_id {
                return Err("Identificador do navegador precisa ser novo e crescente".into());
            }
        } else if let Some(id) = command.id()
            && !self.tabs.contains_key(&id)
        {
            return Err("Aba do navegador já encerrada ou desconhecida".into());
        }
        // Recovery requires an explicit user gesture. A crash itself never restarts.
        if self.state == EngineState::Failed && !open {
            match &command {
                Command::Reload { .. } => return self.restart(),
                Command::Navigate { id, url } => {
                    self.tabs.get_mut(id).unwrap().url = url.clone();
                    return self.restart();
                }
                Command::Close { id } => {
                    self.remove(*id);
                    return Ok(());
                }
                Command::Shutdown => {
                    self.tabs.clear();
                    self.frames.clear();
                    if let Ok(mut m) = self.mailbox.lock() {
                        m.wanted.clear();
                        m.frames.clear();
                        m.states.clear();
                        m.popups.clear();
                        crate::cdp::clear(&mut m, "Navegador encerrado");
                    }
                    return Ok(());
                }
                _ => {
                    return Err(
                        "Motor indisponível; use Recarregar para reiniciar explicitamente".into(),
                    );
                }
            }
        }
        if last_close || matches!(&command, Command::Shutdown) {
            if let Ok(mut m) = self.mailbox.lock() {
                m.shutdown_requested = true;
            }
            let _ = self.send.try_send(Request {
                epoch: self.epoch,
                command: Command::Shutdown,
                start: false,
                config: None,
                last_close: true,
                bootstrap: vec![],
                payload: vec![],
                permit: None,
            });
            if let Command::Close { id } = command {
                self.remove(id);
            } else {
                self.tabs.clear();
                self.frames.clear();
                if let Ok(mut m) = self.mailbox.lock() {
                    m.wanted.clear();
                    m.frames.clear();
                    m.states.clear();
                    crate::cdp::clear(&mut m, "Navegador encerrado");
                }
            }
            if self.state != EngineState::Stopped {
                self.state = EngineState::Stopping;
            }
            return Ok(());
        }
        let start = open && matches!(self.state, EngineState::Stopped | EngineState::Failed);
        let epoch = if start {
            self.epoch
                .checked_add(1)
                .ok_or("Epoch do navegador excedido")?
        } else {
            self.epoch
        };
        let mut bootstrap = if start && !self.tabs.is_empty() {
            self.restoration()?
        } else {
            vec![]
        };
        if !bootstrap.is_empty() {
            bootstrap.push(command.clone());
        }
        self.enqueue_committed(Request {
            epoch,
            command: command.clone(),
            start,
            config: if start { self.config.clone() } else { None },
            last_close,
            bootstrap,
            payload: vec![],
            permit: None,
        })?;
        match command {
            Command::Open { id, url } => {
                self.last_id = id;
                self.tabs.insert(
                    id,
                    TabState {
                        id,
                        url,
                        title: "Navegador".into(),
                        loading: true,
                        can_go_back: false,
                        can_go_forward: false,
                        error: None,
                        devtools_open: false,
                        frontend_epoch: 0,
                        visible: false,
                        devtools_visible: false,
                    },
                );
            }
            Command::Close { id } => {
                self.remove(id);
                if last_close {
                    self.state = EngineState::Stopping;
                }
            }
            Command::Shutdown => {
                self.tabs.clear();
                self.frames.clear();
                if let Ok(mut m) = self.mailbox.lock() {
                    m.wanted.clear();
                    m.frames.clear();
                    m.states.clear();
                    crate::cdp::clear(&mut m, "Navegador encerrado");
                }
                if self.state != EngineState::Stopped {
                    self.state = EngineState::Stopping;
                }
            }
            Command::Navigate { id, url } => {
                let t = self.tabs.get_mut(&id).unwrap();
                t.url = url;
                t.loading = true;
                t.error = None;
            }
            Command::Resize {
                id,
                surface,
                width,
                height,
                scale,
            } => {
                self.frames.remove(&(id, surface));
                if let Ok(mut m) = self.mailbox.lock() {
                    m.frames.remove(&(id, surface));
                    if let Some(d) = m.wanted.get_mut(&(id, surface)) {
                        d.width = width;
                        d.height = height;
                        d.scale = scale;
                    }
                }
            }
            Command::Visible {
                id,
                surface,
                visible,
            } => {
                let t = self.tabs.get_mut(&id).unwrap();
                match surface {
                    Surface::Page => t.visible = visible,
                    Surface::DevTools => t.devtools_visible = visible,
                }
                if !visible {
                    self.frames.remove(&(id, surface));
                }
                if let Ok(mut m) = self.mailbox.lock() {
                    if let Some(d) = m.wanted.get_mut(&(id, surface)) {
                        d.visible = visible;
                    }
                    if !visible {
                        m.frames.remove(&(id, surface));
                    }
                }
            }
            Command::Focus {
                id,
                surface,
                focused,
            } => {
                if let Ok(mut m) = self.mailbox.lock()
                    && let Some(d) = m.wanted.get_mut(&(id, surface))
                {
                    d.focus = focused;
                }
            }
            Command::DevTools { id, open } => {
                self.tabs.get_mut(&id).unwrap().devtools_open = open;
                if !open {
                    self.tabs.get_mut(&id).unwrap().devtools_visible = false;
                    self.frames.remove(&(id, Surface::DevTools));
                    if let Ok(mut m) = self.mailbox.lock() {
                        m.frames.remove(&(id, Surface::DevTools));
                        if let Some(d) = m.wanted.get_mut(&(id, Surface::DevTools)) {
                            d.visible = false;
                        }
                    }
                }
            }
            Command::Inspect { id, .. } => self.tabs.get_mut(&id).unwrap().devtools_open = true,
            _ => {}
        }
        Ok(())
    }
    /// The broker may receive immediately, but cannot publish PID/state/frames until
    /// its epoch and desired surfaces have committed under this same mailbox gate.
    /// A rejected nonblocking send leaves the old session completely unchanged.
    fn enqueue_committed(&mut self, request: Request) -> Result<(), String> {
        let start = request.start;
        let epoch = request.epoch;
        let opened = match &request.command {
            Command::Open { id, .. } => Some(*id),
            _ => None,
        };
        let mailbox = self.mailbox.clone();
        let mut m = mailbox
            .lock()
            .map_err(|_| "Estado do navegador indisponível")?;
        self.send
            .try_send(request)
            .map_err(|_| "Fila do navegador cheia ou serviço encerrado".to_owned())?;
        #[cfg(test)]
        if start && let Some(hook) = self.before_start_commit.take() {
            hook();
        }
        if start {
            self.begin_locked(epoch, &mut m);
        }
        if let Some(id) = opened {
            m.wanted.insert((id, Surface::Page), Desired::default());
            m.wanted.insert((id, Surface::DevTools), Desired::default());
        }
        drop(m);
        #[cfg(test)]
        if start && let Some(hook) = self.after_start_commit.take() {
            hook();
        }
        Ok(())
    }
    fn begin_locked(&mut self, epoch: u64, m: &mut Mailbox) {
        self.epoch = epoch;
        self.state = EngineState::Starting;
        self.last_error = None;
        self.versions = None;
        self.frames.clear();
        crate::cdp::clear(m, "Sessão do navegador substituída");
        m.epoch = epoch;
        m.pid = None;
        m.frames.clear();
        m.states.clear();
        m.status = None;
        m.popups.clear();
        m.pending_cdp_cleanup.clear();
        m.last_cdp_epoch.clear();
        m.shutdown_requested = false;
        for d in m.wanted.values_mut() {
            d.seq = 0;
        }
    }
    pub fn restart(&mut self) -> Result<(), String> {
        if !matches!(self.state, EngineState::Failed | EngineState::Stopped) || self.tabs.is_empty()
        {
            return Err("Reinício exige motor parado e aba aberta".into());
        }
        let requests = self.restoration()?;
        let epoch = self
            .epoch
            .checked_add(1)
            .ok_or("Epoch do navegador excedido")?;
        self.enqueue_committed(Request {
            epoch,
            command: Command::Shutdown,
            start: true,
            config: self.config.clone(),
            last_close: false,
            bootstrap: requests,
            payload: vec![],
            permit: None,
        })?;
        for t in self.tabs.values_mut() {
            t.loading = true;
            t.error = None;
            t.can_go_back = false;
            t.can_go_forward = false;
        }
        Ok(())
    }
    fn restoration(&self) -> Result<Vec<Command>, String> {
        let tabs = self.tabs.values().cloned().collect::<Vec<_>>();
        let desired = self
            .mailbox
            .lock()
            .map_err(|_| "Estado do navegador indisponível")?
            .wanted
            .clone();
        // At most8tabs *7restorationcommands =56 commands, below the bounded queue.
        let mut requests = Vec::new();
        for t in &tabs {
            requests.push(Command::Open {
                id: t.id,
                url: t.url.clone(),
            });
            if t.devtools_open {
                requests.push(Command::DevTools {
                    id: t.id,
                    open: true,
                });
            }
            for surface in [Surface::Page, Surface::DevTools] {
                if let Some(d) = desired.get(&(t.id, surface)) {
                    requests.push(Command::Resize {
                        id: t.id,
                        surface,
                        width: d.width,
                        height: d.height,
                        scale: d.scale,
                    });
                    if d.visible {
                        requests.push(Command::Visible {
                            id: t.id,
                            surface,
                            visible: true,
                        });
                    }
                    if d.focus {
                        requests.push(Command::Focus {
                            id: t.id,
                            surface,
                            focused: true,
                        });
                    }
                }
            }
        }
        Ok(requests)
    }
    fn remove(&mut self, id: BrowserId) {
        self.tabs.remove(&id);
        if self.tabs.is_empty() && self.state == EngineState::Ready {
            self.state = EngineState::Stopping;
        }

        self.frames.retain(|(owner, _), _| *owner != id);
        if let Ok(mut m) = self.mailbox.lock() {
            m.wanted.retain(|(owner, _), _| *owner != id);
            m.frames.retain(|(owner, _), _| *owner != id);
            m.states.remove(&id);
            m.last_cdp_epoch.remove(&id);
            m.pending_cdp_cleanup.remove(&id);
            crate::cdp::close(&mut m, id, Some("Aba encerrada".into()));
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        let mut events = Vec::new();
        for _ in 0..64 {
            let Ok(e) = self.receive.try_recv() else {
                break;
            };
            if e.epoch == self.epoch {
                events.push(e.event);
            }
        }
        if let Ok(mut m) = self.mailbox.lock()
            && m.epoch == self.epoch
        {
            events.extend(std::mem::take(&mut m.states).into_values());
            for (key, frame) in std::mem::take(&mut m.frames) {
                if self.tabs.contains_key(&key.0) && m.wanted.get(&key).is_some_and(|d| d.visible) {
                    while self
                        .frames
                        .values()
                        .map(|f| f.bgra.len() + f.rgba.len())
                        .sum::<usize>()
                        + frame.bgra.len()
                        + frame.rgba.len()
                        > worker::FRAME_BUDGET
                    {
                        let Some(old) = self.frames.keys().copied().find(|k| *k != key) else {
                            break;
                        };
                        self.frames.remove(&old);
                    }
                    self.frames.insert(key, frame);
                    changed = true;
                }
            }
            if let Some((state, error)) = m.status.take() {
                self.state = state;
                if let Some(e) = error {
                    self.last_error = Some(e.clone());
                    for t in self.tabs.values_mut() {
                        t.loading = false;
                        t.error = Some(e.clone());
                    }
                    self.frames.clear();
                }
                changed = true;
            }
        }
        for event in events {
            changed = true;
            match event {
                crate::Event::Ready { versions, .. } => {
                    self.versions = Some(versions);
                    if self.state != EngineState::Stopping && self.state != EngineState::Failed {
                        self.state = EngineState::Ready;
                    }
                }
                crate::Event::State {
                    id,
                    url,
                    title,
                    loading,
                    can_go_back,
                    can_go_forward,
                    error,
                } => {
                    if self.state != EngineState::Failed
                        && let Some(t) = self.tabs.get_mut(&id)
                    {
                        t.url = url;
                        t.title = title
                            .chars()
                            .map(|c| if c.is_control() { ' ' } else { c })
                            .collect();
                        t.loading = loading;
                        t.can_go_back = can_go_back;
                        t.can_go_forward = can_go_forward;
                        t.error = error;
                    }
                }
                crate::Event::Closed { id } => self.remove(id),
                crate::Event::DevToolsClosed { id } => {
                    if let Some(t) = self.tabs.get_mut(&id) {
                        t.devtools_open = false;
                        t.devtools_visible = false;
                    }
                    self.frames.remove(&(id, Surface::DevTools));
                }
                crate::Event::Error { id, message, fatal } => {
                    if let Some(id) = id
                        && let Some(t) = self.tabs.get_mut(&id)
                    {
                        t.error = Some(message.clone());
                        t.loading = false;
                    }
                    if fatal {
                        self.last_error = Some(message);
                        self.state = EngineState::Failed;
                        self.frames.clear();
                    }
                }
                crate::Event::Popup { id, url } => {
                    worker::store_popup(&self.mailbox, self.epoch, id, url);
                }
                crate::Event::Frame { .. }
                | crate::Event::DevToolsMessage { .. }
                | crate::Event::DevToolsDetached { .. } => {}
            }
        }
        changed
    }
    pub fn take_popups(&mut self) -> Vec<(BrowserId, String)> {
        worker::take_popups(&self.mailbox, self.epoch)
    }
    pub fn engine_pid(&self) -> Option<u32> {
        self.mailbox.lock().ok().and_then(|m| m.pid)
    }
    pub fn tabs(&self) -> &BTreeMap<BrowserId, TabState> {
        &self.tabs
    }
    pub fn frame(&self, id: BrowserId, surface: Surface) -> Option<&Frame> {
        self.frames.get(&(id, surface))
    }
    pub fn take_frame(&mut self, id: BrowserId, surface: Surface) -> Option<Frame> {
        self.frames.remove(&(id, surface))
    }
    pub fn is_busy(&self) -> bool {
        matches!(self.state, EngineState::Starting | EngineState::Stopping)
            || (self.state == EngineState::Failed && self.engine_pid().is_some())
    }
    pub fn shutdown(&mut self) {
        let _ = self.dispatch(Command::Shutdown);
    }
}
impl Drop for BrowserService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.send.try_send(Request {
            epoch: self.epoch,
            command: Command::Shutdown,
            start: false,
            config: None,
            last_close: false,
            bootstrap: vec![],
            payload: vec![],
            permit: None,
        });
        if let Some(worker) = self.worker.take() {
            let _ = thread::Builder::new()
                .name("codisseu-browser-reaper".into())
                .spawn(move || {
                    let _ = worker.join();
                });
        }
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use std::{sync::TryLockError, time::Duration};

    fn request() -> Request {
        Request {
            epoch: 7,
            command: Command::Stop { id: 1 },
            start: false,
            config: None,
            last_close: false,
            bootstrap: vec![],
            payload: vec![],
            permit: None,
        }
    }
    fn service() -> (BrowserService, Receiver<Request>) {
        let (send, rx) = mpsc::sync_channel(64);
        let (_, receive) = mpsc::sync_channel(64);
        (
            BrowserService {
                state: EngineState::Stopped,
                tabs: BTreeMap::new(),
                frames: BTreeMap::new(),
                versions: None,
                last_error: None,
                config: None,
                send,
                receive,
                mailbox: Arc::new(Mutex::new(Mailbox::default())),
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
                epoch: 0,
                last_id: 0,
                before_start_commit: None,
                after_start_commit: None,
            },
            rx,
        )
    }
    fn seed_failed(s: &mut BrowserService) {
        s.state = EngineState::Failed;
        s.epoch = 7;
        s.last_id = 1;
        s.last_error = Some("old failure".into());
        s.tabs.insert(
            1,
            TabState {
                id: 1,
                url: "http://localhost:5173/".into(),
                title: "Existing tab".into(),
                loading: false,
                can_go_back: true,
                can_go_forward: true,
                error: Some("old tab error".into()),
                devtools_open: false,
                frontend_epoch: 0,
                visible: true,
                devtools_visible: false,
            },
        );
        let mut m = s.mailbox.lock().unwrap();
        m.epoch = 7;
        m.pid = Some(12345);
        m.status = Some((EngineState::Failed, Some("old failure".into())));
        m.popups.push((1, "https://example.com/".into()));
        for surface in [Surface::Page, Surface::DevTools] {
            m.wanted.insert(
                (1, surface),
                Desired {
                    width: 321,
                    height: 123,
                    scale: 1.25,
                    visible: true,
                    focus: true,
                    seq: 41,
                },
            );
        }
    }
    fn observation(s: &BrowserService) -> String {
        let m = s.mailbox.lock().unwrap();
        format!(
            "{:?}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}",
            s.state,
            s.tabs,
            s.frames,
            s.versions,
            s.last_error,
            s.epoch,
            s.last_id,
            m.epoch,
            m.pid,
            m.wanted,
            m.frames,
            m.states,
            m.status,
            m.popups,
            m.shutdown_requested
        )
    }
    fn publication_is_atomic(restart: bool) {
        let (mut s, rx) = service();
        if restart {
            seed_failed(&mut s);
        }
        let old_epoch = s.epoch;
        let mailbox = s.mailbox.clone();
        let (probe_tx, probe_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::sync_channel(1);
        // This worker is forced to observe the exact point after send succeeds but
        // before begin/wanted commit. It cannot enter the mailbox at that point.
        let worker = thread::spawn(move || {
            let r = rx.recv().unwrap();
            assert!(r.start);
            assert_eq!(r.epoch, old_epoch + 1);
            let gated = matches!(mailbox.try_lock(), Err(TryLockError::WouldBlock));
            probe_tx.send(gated).unwrap();
            let mut m = mailbox.lock().unwrap();
            assert_eq!(m.epoch, r.epoch);
            assert!(m.wanted.contains_key(&(1, Surface::Page)));
            assert!(m.wanted.contains_key(&(1, Surface::DevTools)));
            assert_eq!(m.wanted[&(1, Surface::Page)].seq, 0);
            if restart {
                let d = m.wanted[&(1, Surface::Page)];
                assert_eq!(
                    (d.width, d.height, d.scale, d.visible, d.focus),
                    (321, 123, 1.25, true, true)
                );
            }
            // Same guarded PID publication as the real broker. A late GUI begin
            // or duplicate wanted insert would erase these observed values.
            m.pid = Some(24680);
            m.wanted.get_mut(&(1, Surface::Page)).unwrap().seq = 9;
            m.status = Some((EngineState::Ready, None));
            drop(m);
            done_tx.send(()).unwrap();
        });
        s.before_start_commit = Some(Box::new(move || {
            assert!(
                probe_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
                "broker observed the old mailbox after start was sent"
            )
        }));
        s.after_start_commit = Some(Box::new(move || {
            done_rx.recv_timeout(Duration::from_secs(3)).unwrap()
        }));
        if restart {
            s.restart().unwrap();
        } else {
            s.dispatch(Command::Open {
                id: 1,
                url: "localhost:5173".into(),
            })
            .unwrap();
        }
        worker.join().unwrap();
        assert_eq!(s.engine_pid(), Some(24680));
        assert_eq!(
            s.mailbox.lock().unwrap().wanted[&(1, Surface::Page)].seq,
            9,
            "late wanted insertion erased an already published frame sequence"
        );
        s.poll();
        assert_eq!(s.state, EngineState::Ready);
        assert_eq!(s.engine_pid(), Some(24680));
        assert!(s.tabs.contains_key(&1));
    }
    #[test]
    fn first_open_commits_epoch_and_both_surfaces_before_broker_pid_publication() {
        publication_is_atomic(false);
    }
    #[test]
    fn restart_commits_epoch_preserving_desired_geometry_before_broker_pid_publication() {
        publication_is_atomic(true);
    }
    #[test]
    fn rejected_start_and_restart_preserve_old_session_on_full_or_disconnected_queue() {
        for restart in [false, true] {
            for disconnected in [false, true] {
                let (mut s, rx) = service();
                seed_failed(&mut s);
                let before = observation(&s);
                let held_receiver = if disconnected {
                    drop(rx);
                    None
                } else {
                    for _ in 0..64 {
                        s.send.try_send(request()).ok().unwrap();
                    }
                    Some(rx)
                };
                let result = if restart {
                    s.restart()
                } else {
                    s.dispatch(Command::Open {
                        id: 2,
                        url: "localhost:5174".into(),
                    })
                };
                assert!(result.is_err());
                assert_eq!(
                    observation(&s),
                    before,
                    "rejected start mutated old session: restart={restart}, disconnected={disconnected}"
                );
                drop(held_receiver);
            }
        }
    }
}
