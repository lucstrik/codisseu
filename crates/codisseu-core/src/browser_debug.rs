//! Explicit browser debugging prepares its private CDP gateway off the GUI.
use super::*;
use debugger::{
    BrowserDebugBridge, DebugCommand, DebugConfiguration, DebugRequest, DebugRuntime, SessionId,
};
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, TryRecvError},
    time::Duration,
};

#[derive(Default)]
pub(super) struct BrowserDebug {
    epoch: u64,
    pending: Option<Pending>,
    bridges: BTreeMap<SessionId, (u64, BrowserDebugBridge)>,
    started: VecDeque<(SessionId, DebugConfiguration)>,
}
struct Pending {
    tab: u64,
    url: String,
    engine_pid: Option<u32>,
    workspace: Option<PathBuf>,
    documents: Vec<(DocumentId, u64)>,
    config: DebugConfiguration,
    receiver: Receiver<Result<BrowserDebugBridge, String>>,
    began: std::time::Instant,
}
impl AppService {
    pub(super) fn reserve_browser_cdp_epoch(&mut self) -> Result<u64, String> {
        self.browser_debug.epoch = self
            .browser_debug
            .epoch
            .checked_add(1)
            .ok_or("Limite de sessões CDP")?;
        Ok(self.browser_debug.epoch)
    }
    pub(super) fn browser_debug_active_for_tab(&self, id: u64) -> bool {
        self.browser_debug
            .pending
            .as_ref()
            .is_some_and(|p| p.tab == id)
            || self
                .browser_debug
                .bridges
                .values()
                .any(|(tab, _)| *tab == id)
    }
    pub fn browser_debug_pending(&self) -> bool {
        self.browser_debug.pending.is_some()
    }
    pub fn take_browser_debug_started(&mut self) -> Option<(SessionId, DebugConfiguration)> {
        self.browser_debug.started.pop_front()
    }
    pub(super) fn start_browser_debug(
        &mut self,
        mut config: DebugConfiguration,
    ) -> Result<(), String> {
        if self.browser_debug.pending.is_some() {
            return Err("Aguarde a preparação da depuração do navegador".into());
        }
        if !config.trusted {
            return Err("Autorize esta configuração de depuração antes de continuar".into());
        }
        if config.runtime != DebugRuntime::Chrome || config.request != DebugRequest::Attach {
            return Err("O navegador integrado exige conexão à aba existente".into());
        }
        let id = config
            .browser_tab
            .ok_or("Selecione uma aba do navegador integrado nesta sessão")?;
        if self
            .browser_debug
            .bridges
            .values()
            .any(|(tab, _)| *tab == id)
        {
            return Err("Esta aba já possui uma sessão de depuração".into());
        }
        let tab = self
            .browser
            .tabs
            .get(&id)
            .ok_or("Aba do navegador encerrada")?;
        if self.browser.state != browser::EngineState::Ready || tab.loading || tab.error.is_some() {
            return Err("Aguarde uma página válida terminar de carregar antes de depurar".into());
        }
        let url = browser::normalize_url(&tab.url)?;
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("Abra a aplicação HTTP/HTTPS antes de iniciar a depuração".into());
        }
        if self.documents.iter().any(Document::is_dirty) {
            return Err(
                "Salve as fontes modificadas antes de depurar o frontend servido em disco".into(),
            );
        }
        if !config.cwd.is_absolute()
            || !self
                .workspace_roots()
                .iter()
                .any(|root| config.cwd.starts_with(root))
        {
            return Err("Escolha uma raiz web dentro dos projetos abertos".into());
        }
        self.stop_browser_inspector_tab(id);
        self.close_browser_devtools(id);
        let epoch = self.reserve_browser_cdp_epoch()?;
        let channel = self.browser.devtools(id, epoch)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("codisseu-browser-debug-prepare".into())
            .spawn(move || {
                let _ = sender.send(BrowserDebugBridge::start(channel, id, epoch));
            })
            .map_err(|_| "Não foi possível preparar a depuração do navegador")?;
        config.browser_inspect_uri = None;
        config.url = url.clone();
        self.browser_debug.pending = Some(Pending {
            tab: id,
            url,
            engine_pid: self.browser.engine_pid(),
            workspace: self.workspace.clone(),
            documents: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            config,
            receiver,
            began: std::time::Instant::now(),
        });
        self.debugger.error = None;
        self.status = "Preparando depuração da aba integrada…".into();
        Ok(())
    }
    pub(super) fn stop_browser_debug_tab(&mut self, id: u64) {
        if self
            .browser_debug
            .pending
            .as_ref()
            .is_some_and(|p| p.tab == id)
        {
            self.browser_debug.pending = None;
            self.browser.end_devtools(id);
        }
        let sessions: Vec<_> = self
            .browser_debug
            .bridges
            .iter()
            .filter(|(_, (tab, _))| *tab == id)
            .map(|(&sid, _)| sid)
            .collect();
        for sid in sessions {
            let _ = self.debugger.execute(DebugCommand::Stop(sid));
            if let Some((tab, _)) = self.browser_debug.bridges.remove(&sid) {
                self.browser.end_devtools(tab);
            }
        }
    }
    pub(super) fn browser_debug_stop_session(&mut self, id: SessionId) {
        if let Some((tab, _)) = self.browser_debug.bridges.remove(&id) {
            self.browser.end_devtools(tab);
        }
    }
    pub(super) fn stop_browser_debug(&mut self) {
        if let Some(pending) = self.browser_debug.pending.take() {
            self.browser.end_devtools(pending.tab);
        }
        let ids: Vec<_> = self.browser_debug.bridges.keys().copied().collect();
        for id in ids {
            let _ = self.debugger.execute(DebugCommand::Stop(id));
        }
        let tabs: Vec<_> = self
            .browser_debug
            .bridges
            .values()
            .map(|(tab, _)| *tab)
            .collect();
        self.browser_debug.bridges.clear();
        for tab in tabs {
            self.browser.end_devtools(tab);
        }
    }
    pub(super) fn browser_debug_poll(&mut self) -> bool {
        let mut changed = false;
        let ended: Vec<_> = self
            .browser_debug
            .bridges
            .iter()
            .filter(|(sid, (tab, _))| {
                !self
                    .debugger
                    .sessions
                    .iter()
                    .any(|s| s.id == **sid && s.state.is_active())
                    || self.browser.state != browser::EngineState::Ready
                    || !self.browser.tabs.contains_key(tab)
            })
            .map(|(&sid, _)| sid)
            .collect();
        for sid in ended {
            if self
                .debugger
                .sessions
                .iter()
                .any(|s| s.id == sid && s.state.is_active())
            {
                let _ = self.debugger.execute(DebugCommand::Stop(sid));
            }
            if let Some((tab, _)) = self.browser_debug.bridges.remove(&sid) {
                self.browser.end_devtools(tab);
            }
            changed = true;
        }
        let Some(pending) = self.browser_debug.pending.as_ref() else {
            return changed;
        };
        let valid = self.workspace == pending.workspace
            && self.browser.engine_pid() == pending.engine_pid
            && self.browser.state == browser::EngineState::Ready
            && self
                .browser
                .tabs
                .get(&pending.tab)
                .is_some_and(|t| t.url == pending.url && !t.loading && t.error.is_none())
            && self
                .documents
                .iter()
                .map(|d| (d.id, d.version))
                .collect::<Vec<_>>()
                == pending.documents
            && !self.documents.iter().any(Document::is_dirty);
        if !valid || pending.began.elapsed() > Duration::from_secs(8) {
            let id = pending.tab;
            self.browser_debug.pending = None;
            self.browser.end_devtools(id);
            self.debugger.error = Some(
                "A aba ou o projeto mudou durante a preparação; revise a depuração novamente"
                    .into(),
            );
            return true;
        }
        let result = match pending.receiver.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("Preparação da depuração interrompida".into()))
            }
        };
        if let Some(result) = result {
            let mut pending = self.browser_debug.pending.take().unwrap();
            match result {
                Ok(bridge) => {
                    let endpoint = bridge.endpoint();
                    pending.config.port = endpoint.port;
                    pending.config.browser_inspect_uri = Some(endpoint.inspect_uri);
                    match self
                        .debugger
                        .execute(DebugCommand::Start(pending.config.clone()))
                    {
                        Ok(Some(id)) => {
                            self.browser_debug.bridges.insert(id, (pending.tab, bridge));
                            if self.browser_debug.started.len() >= 8 {
                                self.browser_debug.started.pop_front();
                            }
                            self.browser_debug.started.push_back((id, pending.config));
                            self.status = "Depuração do navegador integrado iniciada".into();
                        }
                        Ok(None) => {
                            self.debugger.error =
                                Some("Adaptador não iniciou a sessão de depuração".into())
                        }
                        Err(error) => self.debugger.error = Some(error),
                    }
                }
                Err(error) => self.debugger.error = Some(error),
            }
            changed = true;
        }
        changed
    }
}
