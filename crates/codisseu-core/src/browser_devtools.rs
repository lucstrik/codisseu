//! Official Chromium DevTools frontend, with a temporary page-only CDP gateway.
use super::*;
use debugger::BrowserDebugBridge;
use std::{
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};
#[derive(Default)]
pub(super) struct BrowserDevTools {
    pending: BTreeMap<u64, Pending>,
    live: BTreeMap<u64, BrowserDebugBridge>,
}
struct Pending {
    receiver: Receiver<Result<BrowserDebugBridge, String>>,
    engine_pid: Option<u32>,
    began: Instant,
}
impl AppService {
    pub fn browser_devtools_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"pending":self.browser_devtools.pending.keys().copied().collect::<Vec<_>>(),"gateways":self.browser_devtools.live.iter().map(|(id,bridge)|serde_json::json!({"id":id,"connected":bridge.is_connected(),"finished":bridge.is_finished(),"failed":bridge.error().is_some()})).collect::<Vec<_>>()})
    }
    pub fn browser_devtools_pending(&self, id: u64) -> bool {
        self.browser_devtools.pending.contains_key(&id)
    }
    pub(super) fn close_browser_devtools(&mut self, id: u64) {
        let owned = self.browser_devtools.pending.remove(&id).is_some()
            | self.browser_devtools.live.remove(&id).is_some();
        if owned {
            self.browser.end_devtools(id);
        }
        if self.browser.tabs.contains_key(&id) {
            let _ = self
                .browser
                .dispatch(browser::Command::DevTools { id, open: false });
        }
    }
    pub fn shutdown_browser(&mut self) {
        self.cancel_browser_login();
        self.stop_browser_inspector();
        self.stop_browser_debug();
        let ids: Vec<_> = self
            .browser_devtools
            .pending
            .keys()
            .chain(self.browser_devtools.live.keys())
            .copied()
            .collect();
        for id in ids {
            self.close_browser_devtools(id);
        }
        self.browser.shutdown();
    }
    pub(super) fn dispatch_browser_command(
        &mut self,
        command: browser::Command,
    ) -> Result<(), String> {
        if matches!(
            &command,
            browser::Command::Navigate { .. }
                | browser::Command::Reload { .. }
                | browser::Command::Back { .. }
                | browser::Command::Forward { .. }
                | browser::Command::Close { .. }
        ) && let Some(id) = command.id()
        {
            self.cancel_browser_login_tab(id);
        }
        match command {
            browser::Command::DevTools { id, open } => self.set_browser_devtools(id, open),
            browser::Command::Close { id } => {
                self.stop_browser_inspector_tab(id);
                self.close_browser_devtools(id);
                self.stop_browser_debug_tab(id);
                self.browser.dispatch(browser::Command::Close { id })
            }
            browser::Command::Shutdown => {
                self.shutdown_browser();
                Ok(())
            }
            browser::Command::DevToolsFrontend { .. } => Err(
                "O frontend DevTools é configurado somente pela sessão privada do navegador".into(),
            ),
            browser::Command::Inspect { .. } => Err(
                "Use o seletor de elementos do DevTools integrado para inspecionar a página".into(),
            ),
            command => self.browser.dispatch(command),
        }
    }
    fn set_browser_devtools(&mut self, id: u64, open: bool) -> Result<(), String> {
        if !open {
            self.close_browser_devtools(id);
            return Ok(());
        }
        if self.browser_debug_active_for_tab(id) {
            return Err("Encerre a sessão de debug desta aba antes de abrir DevTools".into());
        }
        if self.browser_devtools.pending.contains_key(&id)
            || self.browser_devtools.live.contains_key(&id)
        {
            return Ok(());
        }
        self.stop_browser_inspector_tab(id);
        let epoch = self.reserve_browser_cdp_epoch()?;
        let channel = self.browser.devtools(id, epoch)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("codisseu-devtools-prepare".into())
            .spawn(move || {
                let _ = sender.send(BrowserDebugBridge::start_page(channel, id, epoch));
            })
            .map_err(|_| "Não foi possível preparar DevTools")?;
        self.browser_devtools.pending.insert(
            id,
            Pending {
                receiver,
                engine_pid: self.browser.engine_pid(),
                began: Instant::now(),
            },
        );
        self.browser.last_error = None;
        self.status = "Preparando DevTools da aba integrada…".into();
        Ok(())
    }
    pub(super) fn browser_devtools_poll(&mut self) -> bool {
        let mut changed = false;
        let closed: Vec<_> = self
            .browser_devtools
            .live
            .iter()
            .filter(|(id, bridge)| {
                bridge.is_finished()
                    || self.browser.state != browser::EngineState::Ready
                    || !self.browser.tabs.get(id).is_some_and(|t| t.devtools_open)
            })
            .map(|(&id, _)| id)
            .collect();
        for id in closed {
            self.close_browser_devtools(id);
            changed = true;
        }
        let ids: Vec<_> = self.browser_devtools.pending.keys().copied().collect();
        for id in ids {
            let pending = &self.browser_devtools.pending[&id];
            if self.browser.state != browser::EngineState::Ready
                || !self.browser.tabs.contains_key(&id)
                || self.browser.engine_pid() != pending.engine_pid
                || pending.began.elapsed() > Duration::from_secs(8)
            {
                self.close_browser_devtools(id);
                changed = true;
                continue;
            }
            let result = match pending.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Preparação DevTools interrompida".into()))
                }
            };
            if let Some(result) = result {
                self.browser_devtools.pending.remove(&id);
                let result = result.and_then(|bridge| {
                    let url = bridge.frontend_url()?;
                    self.browser.open_devtools_frontend(id, url)?;
                    Ok(bridge)
                });
                match result {
                    Ok(bridge) => {
                        self.browser_devtools.live.insert(id, bridge);
                        self.status = "DevTools integrado aberto".into();
                    }
                    Err(error) => {
                        self.close_browser_devtools(id);
                        self.browser.last_error = Some(error);
                    }
                }
                changed = true;
            }
        }
        changed
    }
}
