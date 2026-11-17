//! Own native inspection independently from the SDK frontend and DAP adapter.
use super::*;
use browser::inspector::{Action, Lifecycle, Session};
impl AppService {
    pub fn start_browser_inspector(&mut self, id: u64) -> Result<(), String> {
        let tab = self
            .browser
            .tabs
            .get(&id)
            .ok_or("Aba do navegador encerrada")?;
        if self.browser.state != browser::EngineState::Ready || tab.loading || tab.error.is_some() {
            return Err("Aguarde uma página válida terminar de carregar".into());
        }
        let url = browser::validate_url(&tab.url)?;
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("Abra uma página HTTP/HTTPS antes de inspecionar".into());
        }
        self.stop_browser_inspector();
        self.close_browser_devtools(id);
        self.stop_browser_debug_tab(id);
        let epoch = self.reserve_browser_cdp_epoch()?;
        let channel = self.browser.devtools(id, epoch)?;
        self.browser_inspector.start(
            Session {
                tab: id,
                epoch,
                engine_pid: self.browser.engine_pid().ok_or("Motor encerrado")?,
                url,
                origin: String::new(),
            },
            channel,
        )?;
        self.status = "Inspetor nativo iniciado".into();
        Ok(())
    }
    pub fn stop_browser_inspector(&mut self) {
        if let Some(session) = &self.browser_inspector.state.session {
            self.browser.end_devtools(session.tab)
        }
        self.browser_inspector.stop();
    }
    pub fn dispatch_browser_inspector(&mut self, action: Action) -> Result<(), String> {
        self.browser_inspector.dispatch(action)
    }
    pub(super) fn stop_browser_inspector_tab(&mut self, id: u64) {
        if self
            .browser_inspector
            .state
            .session
            .as_ref()
            .is_some_and(|s| s.tab == id)
        {
            self.stop_browser_inspector()
        }
    }
    pub(super) fn browser_inspector_poll(&mut self) -> bool {
        let mut changed = self.browser_inspector.poll();
        let invalid = self
            .browser_inspector
            .state
            .session
            .as_ref()
            .is_some_and(|s| {
                self.browser.state != browser::EngineState::Ready
                    || self.browser.engine_pid() != Some(s.engine_pid)
                    || !self.browser.tabs.contains_key(&s.tab)
            });
        if invalid {
            self.stop_browser_inspector();
            changed = true;
        }
        if self.browser_inspector.state.lifecycle == Lifecycle::Failed
            && let Some(s) = &self.browser_inspector.state.session
        {
            self.browser.end_devtools(s.tab)
        }
        changed
    }
}
