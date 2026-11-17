//! Explicit one-shot login operations in a document-pinned, isolated CDP world.
use super::*;
use browser::{DevToolsChannel, DevToolsEvent};
use browser_library::{LoadedLogin, Secret};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};
pub struct CapturedLogin {
    pub tab: u64,
    pub origin: String,
    pub username: String,
    pub password: Secret,
}
#[derive(Default)]
pub(super) struct BrowserLogin {
    pending: Option<Pending>,
    captured: Option<CapturedLogin>,
    pub notice: Option<String>,
}
struct Pending {
    tab: u64,
    url: String,
    pid: u32,
    stop: Arc<AtomicBool>,
    phase: Phase,
}
enum Phase {
    Vault {
        key: String,
        channel: DevToolsChannel,
    },
    Page(Receiver<Result<Option<CapturedLogin>, String>>),
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl AppService {
    pub fn browser_login_busy(&self) -> bool {
        self.browser_login.pending.is_some()
    }
    pub fn browser_login_notice(&self) -> Option<&str> {
        self.browser_login.notice.as_deref()
    }
    pub fn take_browser_login(&mut self) -> Option<CapturedLogin> {
        self.browser_login.captured.take()
    }
    pub(super) fn cancel_browser_login_tab(&mut self, id: u64) {
        if self
            .browser_login
            .pending
            .as_ref()
            .is_some_and(|p| p.tab == id)
        {
            self.cancel_browser_login();
        }
        if self
            .browser_login
            .captured
            .as_ref()
            .is_some_and(|p| p.tab == id)
        {
            self.browser_login.captured = None;
        }
    }
    pub fn cancel_browser_login(&mut self) {
        self.browser_login.notice = None;
        self.browser_login.pending = None;
        self.browser_login.captured = None;
        self.browser_library.take_login();
    }
    fn login_channel(&mut self, id: u64) -> Result<(String, u32, DevToolsChannel), String> {
        if self.browser_login.pending.is_some() {
            return Err("Aguarde a operação de acesso atual".into());
        }
        let tab = self.browser.tabs.get(&id).ok_or("Aba encerrada")?;
        if self.browser.state != browser::EngineState::Ready || tab.loading || tab.error.is_some() {
            return Err("Aguarde a página terminar de carregar".into());
        }
        browser_library::origin(&tab.url)?;
        let url = tab.url.clone();
        let pid = self.browser.engine_pid().ok_or("Motor encerrado")?;
        let epoch = self.reserve_browser_cdp_epoch()?;
        let channel=self.browser.devtools(id,epoch).map_err(|_|"Feche DevTools/inspetor ou encerre o debug desta aba para ler/preencher o formulário; o cadastro manual continua disponível")?;
        self.browser_login.notice = None;
        Ok((url, pid, channel))
    }
    pub fn read_browser_login(&mut self, id: u64) -> Result<(), String> {
        let (url, pid, channel) = self.login_channel(id)?;
        let stop = Arc::new(AtomicBool::new(false));
        let rx = spawn(channel, id, url.clone(), None, stop.clone())?;
        self.browser_login.pending = Some(Pending {
            tab: id,
            url,
            pid,
            stop,
            phase: Phase::Page(rx),
        });
        Ok(())
    }
    pub fn fill_browser_login(&mut self, id: u64, key: &str) -> Result<(), String> {
        let login = self
            .browser_library
            .state
            .logins
            .iter()
            .find(|l| l.id == key)
            .ok_or("Acesso não encontrado")?;
        let tab = self.browser.tabs.get(&id).ok_or("Aba encerrada")?;
        if browser_library::origin(&tab.url)? != login.origin {
            return Err("O acesso pertence a outra origem/porta".into());
        }
        let (url, pid, channel) = self.login_channel(id)?;
        self.browser_library
            .dispatch(browser_library::Action::LoadLogin(key.into()))?;
        self.browser_login.pending = Some(Pending {
            tab: id,
            url,
            pid,
            stop: Arc::new(AtomicBool::new(false)),
            phase: Phase::Vault {
                key: key.into(),
                channel,
            },
        });
        Ok(())
    }
    pub(super) fn poll_browser_login(&mut self) -> bool {
        let Some(p) = self.browser_login.pending.as_ref() else {
            self.browser_library.take_login();
            return false;
        };
        if self.browser.engine_pid() != Some(p.pid)
            || self.browser.state != browser::EngineState::Ready
            || self
                .browser
                .tabs
                .get(&p.tab)
                .is_none_or(|t| t.url != p.url || t.loading || t.error.is_some())
        {
            self.cancel_browser_login();
            self.browser_login.notice =
                Some("Página alterada: operação de acesso cancelada".into());
            return true;
        }
        match &p.phase {
            Phase::Vault { .. } => {
                if let Some(loaded) = self.browser_library.take_login() {
                    let mut p = self.browser_login.pending.take().unwrap();
                    let phase = std::mem::replace(&mut p.phase, Phase::Page(mpsc::channel().1));
                    if let Phase::Vault { key, channel } = phase {
                        if loaded.login.id != key {
                            self.browser_login.notice =
                                Some("Resultado de acesso obsoleto descartado".into());
                            return true;
                        }
                        match spawn(channel, p.tab, p.url.clone(), Some(loaded), p.stop.clone()) {
                            Ok(rx) => {
                                p.phase = Phase::Page(rx);
                                self.browser_login.pending = Some(p);
                            }
                            Err(e) => self.browser_login.notice = Some(e),
                        }
                    }
                    return true;
                }
                if !self.browser_library.is_busy() {
                    self.browser_login.notice = self.browser_library.error.clone();
                    self.browser_login.pending = None;
                    return true;
                }
            }
            Phase::Page(rx) => {
                let result = match rx.try_recv() {
                    Ok(v) => Some(v),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("Operação de acesso encerrada".into()))
                    }
                    _ => None,
                };
                if let Some(result) = result {
                    self.browser_login.pending = None;
                    match result {
                        Ok(Some(login)) => {
                            self.browser_login.captured = Some(login);
                            self.browser_login.notice =
                                Some("Confira o acesso e clique em Salvar no cofre".into());
                        }
                        Ok(None) => {
                            self.browser_login.notice =
                                Some("Acesso preenchido; confira e envie o formulário".into())
                        }
                        Err(e) => self.browser_login.notice = Some(e),
                    }
                    return true;
                }
            }
        }
        false
    }
}
struct Page {
    channel: DevToolsChannel,
    serial: u64,
    contexts: BTreeMap<i64, (String, String)>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
}
impl Page {
    fn event(&mut self, v: &Value) {
        if v["method"] == "Runtime.executionContextCreated" {
            let c = &v["params"]["context"];
            if let (Some(id), Some(unique), Some(frame)) = (
                c["id"].as_i64(),
                c["uniqueId"].as_str(),
                c["auxData"]["frameId"].as_str(),
            ) {
                if self.contexts.len() >= 64 {
                    self.contexts.pop_first();
                }
                self.contexts.insert(id, (unique.into(), frame.into()));
            }
        }
    }
    fn receive(&mut self) -> Result<Value, String> {
        loop {
            if self.stop.load(Ordering::Acquire) || Instant::now() > self.deadline {
                return Err("Operação cancelada ou página não respondeu".into());
            }
            match self.channel.events.recv_timeout(Duration::from_millis(25)) {
                Ok(DevToolsEvent::Message { json }) => {
                    let v: Value = serde_json::from_str(&json)
                        .map_err(|_| "Resposta inválida do navegador")?;
                    self.event(&v);
                    return Ok(v);
                }
                Ok(DevToolsEvent::Closed { .. }) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Página/contexto encerrado".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
            }
        }
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.serial += 1;
        let id = self.serial;
        self.channel
            .sender
            .send(json!({"id":id,"method":method,"params":params}).to_string())
            .map_err(|_| "Página não está disponível")?;
        loop {
            let v = self.receive()?;
            if v["id"] == id {
                if !v["error"].is_null() {
                    return Err("A página mudou ou recusou a operação de acesso".into());
                }
                return Ok(v["result"].clone());
            }
        }
    }
}
fn spawn(
    channel: DevToolsChannel,
    tab: u64,
    url: String,
    loaded: Option<LoadedLogin>,
    stop: Arc<AtomicBool>,
) -> Result<Receiver<Result<Option<CapturedLogin>, String>>, String> {
    let (tx, rx) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("codisseu-browser-login".into())
        .spawn(move || {
            let result = execute(channel, tab, &url, loaded, stop);
            let _ = tx.send(result);
        })
        .map_err(|_| "Não foi possível preparar acesso à página")?;
    Ok(rx)
}
fn execute(
    channel: DevToolsChannel,
    tab: u64,
    url: &str,
    loaded: Option<LoadedLogin>,
    stop: Arc<AtomicBool>,
) -> Result<Option<CapturedLogin>, String> {
    let expected = browser_library::origin(url)?;
    let mut p = Page {
        channel,
        serial: 0,
        contexts: BTreeMap::new(),
        stop,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let tree = p.call("Page.getFrameTree", json!({}))?;
    let frame = &tree["frameTree"]["frame"];
    let frame_id = frame["id"]
        .as_str()
        .ok_or("Página não tem contexto")?
        .to_owned();
    if browser_library::origin(frame["url"].as_str().unwrap_or(""))? != expected {
        return Err("A origem da página mudou".into());
    }
    // CEF retains domain state across observer leases. Reset only our exclusive
    // Runtime subscription so existing isolated contexts are replayed as well.
    p.call("Runtime.disable", json!({}))?;
    p.call("Runtime.enable", json!({}))?;
    let world = p.call(
        "Page.createIsolatedWorld",
        json!({"frameId":frame_id,"worldName":"codisseu-login"}),
    )?;
    let context_id = world["executionContextId"]
        .as_i64()
        .ok_or("Contexto de acesso indisponível")?;
    while !p.contexts.contains_key(&context_id) {
        p.receive()?;
    }
    let (unique, owner) = p.contexts.get(&context_id).unwrap().clone();
    if owner != frame_id {
        return Err("Contexto de outra página recusado".into());
    }
    let fill = loaded.is_some();
    let args = match loaded.as_ref() {
        Some(l) => {
            if l.login.origin != expected {
                return Err("Acesso de outra origem recusado".into());
            }
            json!([expected, l.login.username, l.password.expose()])
        }
        None => json!([expected]),
    };
    let expression = format!("({FORM_SCRIPT})(...{})", args);
    let result=p.call("Runtime.evaluate",json!({"expression":expression,"uniqueContextId":unique,"returnByValue":true,"userGesture":true,"timeout":1000,"silent":true,"disableBreaks":true}))?;
    let _ = p.call("Runtime.disable", json!({}));
    if !result["exceptionDetails"].is_null() {
        return Err("Formulário não disponível ou mudou durante o acesso".into());
    }
    let result = &result["result"]["value"];
    if result["ok"] != true {
        return Err("Use um formulário visível com um usuário e uma senha, sem OTP/troca de senha e com destino da mesma origem. Você pode cadastrar o acesso manualmente.".into());
    }
    if fill {
        return Ok(None);
    }
    let username = result["username"]
        .as_str()
        .ok_or("Usuário não encontrado")?
        .to_owned();
    let password = Secret::new(
        result["password"]
            .as_str()
            .ok_or("Senha não encontrada")?
            .to_owned(),
    );
    if username.len() > 1024 || password.expose().len() > 16384 {
        return Err("Acesso excede o limite".into());
    }
    Ok(Some(CapturedLogin {
        tab,
        origin: expected,
        username,
        password,
    }))
}
const FORM_SCRIPT: &str = r#"(origin, username, password) => {
 if (location.origin !== origin) return {ok:false};
 const visible = e => !e.disabled && !e.readOnly && e.getClientRects().length && getComputedStyle(e).visibility !== 'hidden';
 const passwords = [...document.querySelectorAll('input[type=password]')].filter(visible);
 if (passwords.length !== 1) return {ok:false};
 const p = passwords[0];
 if (p.autocomplete === 'new-password' || new URL(p.form?.action || location.href).origin !== origin) return {ok:false};
 const candidates = [...(p.form || document).querySelectorAll('input')].filter(e => visible(e) && ['text','email'].includes(e.type) && e.autocomplete !== 'one-time-code');
 const u = candidates.find(e=>e.autocomplete==='username') || candidates.filter(e=>Boolean(e.compareDocumentPosition(p)&Node.DOCUMENT_POSITION_FOLLOWING)).pop();
 if (!u) return {ok:false};
 if (password !== undefined) {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set;
  for (const [e,v] of [[u,username],[p,password]]) {set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));}
  return {ok:true};
 }
 return {ok:true,username:u.value,password:p.value};
}"#;

#[cfg(test)]
#[path = "browser_login_tests.rs"]
mod tests;
