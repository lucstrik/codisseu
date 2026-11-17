use super::*;
use crate::DevToolsChannel;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
};
pub(super) struct Request {
    pub action: Action,
    pub document: u64,
}
pub(super) struct Shared {
    pub latest: Mutex<Option<Snapshot>>,
    pub waker: Mutex<Option<Waker>>,
    pub stop: AtomicBool,
    pub revision: Arc<AtomicU64>,
}
pub struct InspectorService {
    pub state: Snapshot,
    send: Option<SyncSender<Request>>,
    shared: Option<Arc<Shared>>,
    waker: Option<Waker>,
    revision: Arc<AtomicU64>,
}
impl Default for InspectorService {
    fn default() -> Self {
        Self {
            state: Snapshot::default(),
            send: None,
            shared: None,
            waker: None,
            revision: Arc::new(AtomicU64::new(1)),
        }
    }
}
impl InspectorService {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_waker(&mut self, waker: Waker) {
        self.waker = Some(waker.clone());
        if let Some(s) = &self.shared
            && let Ok(mut w) = s.waker.lock()
        {
            *w = Some(waker)
        }
    }
    pub fn start(&mut self, mut session: Session, channel: DevToolsChannel) -> Result<(), String> {
        let origin = model::origin(&session.url)?;
        if session.tab == 0
            || session.epoch == 0
            || session.engine_pid == 0
            || (!session.origin.is_empty() && session.origin != origin)
        {
            return Err("Sessão do inspetor inválida".into());
        }
        session.origin = origin;
        self.stop();
        let state = Snapshot {
            lifecycle: Lifecycle::Starting,
            session: Some(session),
            revision: self.revision.fetch_add(1, Ordering::Relaxed),
            document_epoch: 1,
            ..Default::default()
        };
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            waker: Mutex::new(self.waker.clone()),
            stop: AtomicBool::new(false),
            revision: self.revision.clone(),
        });
        let (send, rx) = mpsc::sync_channel(64);
        let s = shared.clone();
        let initial = state.clone();
        thread::Builder::new()
            .name("codisseu-native-inspector".into())
            .spawn(move || worker::run(channel, rx, s, initial))
            .map_err(|_| "Não foi possível iniciar inspetor")?;
        self.state = state;
        self.shared = Some(shared);
        self.send = Some(send);
        Ok(())
    }
    pub fn stop(&mut self) {
        if let Some(s) = self.shared.take() {
            s.stop.store(true, Ordering::Release)
        }
        self.send = None;
        self.state = Snapshot {
            revision: self.revision.fetch_add(1, Ordering::Relaxed),
            ..Default::default()
        };
    }
    pub fn dispatch(&mut self, action: Action) -> Result<(), String> {
        worker::validate_action(&action)?;
        if !matches!(self.state.lifecycle, Lifecycle::Starting | Lifecycle::Ready) {
            return Err("Inspetor não está ativo".into());
        }
        self.send
            .as_ref()
            .ok_or("Inspetor parado")?
            .try_send(Request {
                action,
                document: self.state.document_epoch,
            })
            .map_err(|_| "Fila do inspetor cheia ou encerrada".into())
    }
    pub fn poll(&mut self) -> bool {
        if let Some(s) = &self.shared
            && let Ok(mut latest) = s.latest.lock()
            && let Some(state) = latest.take()
        {
            self.state = state;
            return true;
        }
        false
    }
}
impl Drop for InspectorService {
    fn drop(&mut self) {
        self.stop()
    }
}
