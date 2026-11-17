//! Optional Jira Cloud integration. One bounded worker owns HTTPS, OS vault and files.
pub mod client;
pub mod content;
pub mod media;
pub mod model;
pub mod period;
pub mod timesheet;
pub use model::*;
pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}
use client::{Identity, JiraClient};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub struct Secret(Zeroizing<String>);
impl Default for Secret {
    fn default() -> Self {
        Self::new(String::new())
    }
}
impl Secret {
    pub fn new(s: String) -> Self {
        Self(Zeroizing::new(s))
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn edit(&mut self) -> &mut String {
        &mut self.0
    }
}
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
enum Op {
    Save(Profile),
    Connect(Connection, Option<Secret>, bool),
    Restore(Connection),
    Disconnect,
    Forget(Connection),
    Search(Filters, Option<String>, usize, u64),
    Detail(String),
    Comments(String, usize),
    Worklogs(String, usize),
    Suggest(Picker, String, Option<String>),
    Write(WorklogDraft),
}
enum Reply {
    Loaded(Box<Profile>),
    Saved,
    Connected(
        Identity,
        Option<String>,
        Option<media::ImageClient>,
        Box<JiraClient>,
        bool,
    ),
    NoSavedToken,
    Disconnected,
    Forgot,
    Page(Page, usize, u64),
    Detail(Box<Detail>),
    Comments(String, Vec<Comment>, usize, usize),
    Worklogs(String, Vec<Worklog>, usize, usize),
    Suggestions(Picker, String, Vec<Choice>),
    Written(String, u32),
}
struct Request {
    epoch: u64,
    op: Op,
}
struct Response {
    epoch: u64,
    result: Result<Reply, String>,
    write: bool,
    scope: Scope,
}
enum Scope {
    Global,
    Search(u64),
    Card(String),
    Suggest(Picker, String),
}
pub struct Service {
    pub timesheet: timesheet::Service,
    pub media: media::Media,
    pub profile: Profile,
    pub loaded: bool,
    pub connected: bool,
    pub identity: Option<Identity>,
    pub page: Page,
    pub page_index: usize,
    pub detail: Option<Detail>,
    pub selected: Option<String>,
    pub suggestions: Vec<Choice>,
    pub suggestion_for: Option<(Picker, String)>,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub revision: u64,
    tx: Option<SyncSender<Request>>,
    rx: Receiver<Response>,
    pending: bool,
    started: Instant,
    epoch: u64,
    query: u64,
    dirty: Option<Instant>,
    search: Option<(usize, Option<String>)>,
    cursors: Vec<Option<String>>,
    intent: Option<Op>,
    select_due: bool,
    profile_error: bool,
    restoring: bool,
}
impl Service {
    pub fn new(path: PathBuf) -> Self {
        Self::with_client(path, JiraClient::new)
    }
    fn with_client(
        path: PathBuf,
        make_client: impl Fn(Connection, Secret) -> Result<JiraClient, String> + Send + 'static,
    ) -> Self {
        let (tx, jobs) = mpsc::sync_channel(1);
        let (replies, rx) = mpsc::sync_channel(1);
        let spawn = std::thread::Builder::new()
            .name("codisseu-jira".into())
            .spawn(move || worker(path, jobs, replies, make_client));
        let error = spawn
            .err()
            .map(|_| "Não foi possível iniciar o worker Jira".into());
        Self {
            timesheet: timesheet::Service::default(),
            media: media::Media::default(),
            profile: Profile::default(),
            loaded: false,
            connected: false,
            identity: None,
            page: Page::default(),
            page_index: 0,
            detail: None,
            selected: None,
            suggestions: vec![],
            suggestion_for: None,
            error: error.clone(),
            notice: None,
            revision: 0,
            tx: if error.is_none() { Some(tx) } else { None },
            rx,
            pending: error.is_none(),
            started: Instant::now(),
            epoch: 0,
            query: 0,
            dirty: None,
            search: None,
            cursors: vec![None],
            intent: None,
            select_due: false,
            profile_error: false,
            restoring: false,
        }
    }
    pub fn is_busy(&self) -> bool {
        self.main_busy() || self.timesheet.busy
    }
    pub fn main_busy(&self) -> bool {
        self.pending
            || self.dirty.is_some()
            || self.intent.is_some()
            || self.search.is_some()
            || self.select_due
    }
    pub fn request_pending(&self) -> bool {
        self.pending
    }
    pub fn restoring(&self) -> bool {
        self.restoring
    }
    fn restore_saved_connection(&mut self) {
        let mut c = self.profile.connection.clone();
        if self.profile.enabled && self.profile.auto_connect && c.validate().is_ok() {
            self.intent = Some(Op::Restore(c));
        }
    }
    pub fn set_profile(&mut self, profile: Profile) -> Result<(), String> {
        if !self.loaded || self.profile_error {
            return Err("Aguarde a leitura ou corrija o arquivo de preferências Jira".into());
        }
        profile.validate()?;
        let connection_changed = profile.connection != self.profile.connection;
        let disabled = !profile.enabled;
        let filters_changed = profile.filters != self.profile.filters;
        let was_enabled = self.profile.enabled;
        self.profile = profile;
        self.dirty = Some(Instant::now());
        self.error = None;
        if connection_changed || disabled {
            self.disconnect_state();
            self.intent = Some(Op::Disconnect)
        } else if !was_enabled {
            self.restore_saved_connection();
        } else if filters_changed {
            self.query = self.query.wrapping_add(1);
            self.cursors = vec![None];
            self.search = self.connected.then_some((0, None));
            self.page = Page::default();
            self.page_index = 0
        }
        Ok(())
    }
    pub fn set_filters(&mut self, filters: Filters) -> Result<(), String> {
        let mut p = self.profile.clone();
        p.filters = filters;
        self.set_profile(p)
    }
    fn disconnect_state(&mut self) {
        self.restoring = false;
        self.timesheet.connection(None);
        self.media.session(None);
        self.epoch = self.epoch.wrapping_add(1);
        self.connected = false;
        self.identity = None;
        self.page = Page::default();
        self.detail = None;
        self.selected = None;
        self.suggestions.clear();
        self.suggestion_for = None;
        self.search = None;
        self.select_due = false;
        self.cursors = vec![None]
    }
    pub fn disconnect(&mut self) {
        self.disconnect_state();
        self.profile.auto_connect = false;
        self.dirty = Some(Instant::now());
        self.intent = Some(Op::Disconnect)
    }
    pub fn connect(
        &mut self,
        mut connection: Connection,
        token: Option<Secret>,
        remember: bool,
    ) -> Result<(), String> {
        if !self.loaded || !self.profile.enabled {
            return Err("Ative a integração Jira nas configurações".into());
        }
        connection.validate()?;
        let mut p = self.profile.clone();
        p.connection = connection.clone();
        self.set_profile(p)?;
        self.disconnect_state();
        self.intent = Some(Op::Connect(connection, token, remember));
        self.notice = None;
        self.error = None;
        Ok(())
    }
    pub fn forget(&mut self) {
        self.disconnect_state();
        self.profile.auto_connect = false;
        self.dirty = Some(Instant::now());
        self.intent = Some(Op::Forget(self.profile.connection.clone()))
    }
    pub fn refresh(&mut self) {
        if self.connected {
            self.query = self.query.wrapping_add(1);
            self.cursors = vec![None];
            self.search = Some((0, None));
            self.error = None
        }
    }
    pub fn next_page(&mut self) {
        if !self.pending
            && self.page_index < 63
            && let Some(next) = self.page.next.clone()
        {
            if self.cursors.len() <= self.page_index + 1 {
                self.cursors.push(Some(next.clone()))
            } else {
                self.cursors[self.page_index + 1] = Some(next.clone())
            }
            self.search = Some((self.page_index + 1, Some(next)))
        }
    }
    pub fn previous_page(&mut self) {
        if !self.pending && self.page_index > 0 {
            let i = self.page_index - 1;
            self.search = Some((i, self.cursors[i].clone()))
        }
    }
    pub fn select(&mut self, key: &str) -> Result<(), String> {
        if !self.connected {
            return Err("Conecte sua conta Jira antes de abrir uma tarefa".into());
        }
        if !issue_key(key) {
            return Err("Chave inválida".into());
        }
        self.selected = Some(key.into());
        self.detail = None;
        self.select_due = true;
        self.error = None;
        Ok(())
    }
    pub fn comments(&mut self, offset: usize) {
        if let Some(k) = &self.selected {
            self.intent = Some(Op::Comments(k.clone(), offset))
        }
    }
    pub fn worklogs(&mut self, offset: usize) {
        if let Some(k) = &self.selected {
            self.intent = Some(Op::Worklogs(k.clone(), offset))
        }
    }
    pub fn suggest(&mut self, p: Picker, text: String) {
        self.suggestions.clear();
        self.suggestion_for = Some((p, text.clone()));
        self.intent = Some(Op::Suggest(
            p,
            text,
            self.profile
                .filters
                .team_field
                .as_ref()
                .map(|f| f.id.clone()),
        ))
    }
    pub fn post_worklog(&mut self, d: WorklogDraft) -> Result<(), String> {
        d.validate()?;
        if self.main_busy() || !self.connected {
            return Err("Aguarde a operação atual e confira a conexão".into());
        }
        if self.selected.as_deref() != Some(&d.key) {
            return Err("O card selecionado mudou; revise o apontamento".into());
        }
        self.intent = Some(Op::Write(d));
        self.notice = None;
        self.error = None;
        Ok(())
    }
    fn submit(&mut self, op: Op) {
        if let Some(tx) = &self.tx {
            let restoring = matches!(&op, Op::Restore(_));
            match tx.try_send(Request {
                epoch: self.epoch,
                op,
            }) {
                Ok(()) => {
                    self.pending = true;
                    self.restoring = restoring;
                    self.started = Instant::now()
                }
                Err(_) => {
                    self.error = Some("Worker Jira indisponível".into());
                    self.tx = None
                }
            }
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = self.media.poll();
        changed |= self.timesheet.poll();
        if let Ok(response) = self.rx.try_recv() {
            self.pending = false;
            self.restoring = false;
            changed = true;
            self.revision = self.revision.wrapping_add(1);
            let current = match &response.scope {
                Scope::Global => true,
                Scope::Search(q) => *q == self.query,
                Scope::Card(k) => self.selected.as_deref() == Some(k),
                Scope::Suggest(p, s) => self
                    .suggestion_for
                    .as_ref()
                    .is_some_and(|(a, b)| a == p && b == s),
            };
            if (response.epoch == self.epoch && current)
                || matches!(response.result, Ok(Reply::Loaded(_) | Reply::Saved))
            {
                if matches!(
                    &response.result,
                    Ok(Reply::Page(..)
                        | Reply::Detail(..)
                        | Reply::Comments(..)
                        | Reply::Worklogs(..)
                        | Reply::Suggestions(..)
                        | Reply::Connected(..)
                        | Reply::Written(..)
                        | Reply::Forgot)
                ) {
                    self.error = None;
                }
                match response.result {
                    Err(e) => {
                        self.error = Some(e);
                        if !self.loaded {
                            self.loaded = true;
                            self.profile_error = true
                        }
                    }
                    Ok(Reply::Loaded(p)) => {
                        self.profile = *p;
                        self.loaded = true;
                        self.restore_saved_connection();
                    }
                    Ok(Reply::Saved | Reply::NoSavedToken) => (),
                    Ok(Reply::Connected(id, warning, images, client, reconnect)) => {
                        if self.profile.auto_connect != reconnect {
                            self.profile.auto_connect = reconnect;
                            self.dirty = Some(Instant::now());
                        }
                        self.timesheet
                            .connection(Some((*client, id.account.clone())));
                        self.media.session(images);
                        self.identity = Some(id);
                        self.connected = true;
                        self.notice = warning;
                        self.refresh()
                    }
                    Ok(Reply::Disconnected | Reply::Forgot) => (),
                    Ok(Reply::Page(p, i, q)) => {
                        if q == self.query {
                            self.page = p;
                            self.page_index = i
                        }
                    }
                    Ok(Reply::Detail(d)) => {
                        if self.selected.as_deref() == Some(&d.issue.key) {
                            self.detail = Some(*d)
                        }
                    }
                    Ok(Reply::Comments(k, a, total, offset)) => {
                        if let Some(d) = &mut self.detail
                            && d.issue.key == k
                        {
                            d.comments = a;
                            d.comment_total = total;
                            d.comment_offset = offset
                        }
                    }
                    Ok(Reply::Worklogs(k, a, total, offset)) => {
                        if let Some(d) = &mut self.detail
                            && d.issue.key == k
                        {
                            d.worklogs = a;
                            d.worklog_total = total;
                            d.worklog_offset = offset
                        }
                    }
                    Ok(Reply::Suggestions(p, s, a)) => {
                        if self.suggestion_for == Some((p, s)) {
                            self.suggestions = a
                        }
                    }
                    Ok(Reply::Written(key, seconds)) => {
                        self.timesheet.stale = true;
                        if let Some(d) = &mut self.detail
                            && d.issue.key == key
                        {
                            d.issue.spent += i64::from(seconds);
                        }
                        self.notice = Some(format!("Horas registradas em {key}."));
                        self.worklogs(0)
                    }
                }
            } else if response.write {
                self.notice=Some("Uma gravação terminou após trocar de conexão. Confira os registros no Jira antes de reenviar.".into())
            }
        }
        if self.pending && self.started.elapsed() > Duration::from_secs(30) && self.error.is_none()
        {
            self.error=Some("A operação Jira/cofre está demorando. Aguarde ou desbloqueie o cofre; ela pode concluir depois.".into());
            changed = true
        }
        if !self.pending && self.loaded && self.tx.is_some() {
            if self
                .dirty
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(350))
            {
                self.dirty = None;
                self.submit(Op::Save(self.profile.clone()));
                changed = true
            } else if let Some(op) = self.intent.take() {
                self.submit(op);
                changed = true
            } else if self.connected && self.profile.enabled {
                if self.select_due {
                    self.select_due = false;
                    if let Some(k) = &self.selected {
                        self.submit(Op::Detail(k.clone()));
                        changed = true
                    }
                } else if self.dirty.is_none()
                    && let Some((i, c)) = self.search.take()
                {
                    self.submit(Op::Search(self.profile.filters.clone(), c, i, self.query));
                    changed = true
                }
            }
        }
        changed
    }
}
fn vault(c: &Connection) -> Result<keyring::Entry, String> {
    if !cfg!(any(
        target_os = "linux",
        target_os = "windows",
        target_os = "macos"
    )) {
        return Err("Cofre indisponível nesta plataforma".into());
    }
    let id = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(1, &c.site, &c.api, &c.email))
                .map_err(|_| "Identidade inválida")?
        )
    );
    keyring::Entry::new("io.codisseu.Codisseu.jira.v1", &id)
        .map_err(|_| "Cofre do sistema indisponível".into())
}
fn load_token(c: &Connection) -> Result<Secret, String> {
    saved_token(c)?.ok_or_else(|| "Token não encontrado no cofre. Informe o token de API.".into())
}
fn saved_token(c: &Connection) -> Result<Option<Secret>, String> {
    let s = match vault(c)?.get_password() {
        Ok(s) => s,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(_) => return Err("Não foi possível ler o token salvo. Desbloqueie o cofre do sistema e tente conectar novamente.".into()),
    };
    if s.len() > 16384 {
        drop(Secret::new(s));
        return Err("Token no cofre acima do limite".into());
    }
    Ok(Some(Secret::new(s)))
}
fn worker(
    path: PathBuf,
    rx: Receiver<Request>,
    tx: SyncSender<Response>,
    make_client: impl Fn(Connection, Secret) -> Result<JiraClient, String>,
) {
    let initial = read_profile(&path);
    let mut expected = initial.as_ref().ok().and_then(|(_, b)| b.clone());
    let loaded = initial.map(|(p, _)| Reply::Loaded(Box::new(p)));
    if tx
        .send(Response {
            epoch: 0,
            result: loaded,
            write: false,
            scope: Scope::Global,
        })
        .is_err()
    {
        return;
    }
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return,
    };
    let mut client: Option<JiraClient> = None;
    while let Ok(r) = rx.recv() {
        let write = matches!(r.op, Op::Write(_));
        let scope = match &r.op {
            Op::Search(_, _, _, q) => Scope::Search(*q),
            Op::Detail(k) | Op::Comments(k, _) | Op::Worklogs(k, _) => Scope::Card(k.clone()),
            Op::Suggest(p, s, _) => Scope::Suggest(*p, s.clone()),
            _ => Scope::Global,
        };
        let result = match r.op {
            Op::Save(p) => save_profile(&path, &p, &mut expected).map(|_| Reply::Saved),
            Op::Disconnect => {
                client = None;
                Ok(Reply::Disconnected)
            }
            Op::Forget(c) => {
                client = None;
                vault(&c).and_then(|v| match v.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(Reply::Forgot),
                    Err(_) => Err("Não foi possível remover o token do cofre".into()),
                })
            }
            op @ (Op::Connect(..) | Op::Restore(_)) => {
                let (c, t, remember, automatic) = match op {
                    Op::Connect(c, t, remember) => (c, t, remember, false),
                    Op::Restore(c) => (c, None, false, true),
                    _ => unreachable!(),
                };
                client = None;
                let from_vault = t.is_none();
                let token = if automatic {
                    saved_token(&c)
                } else {
                    t.map(Ok).unwrap_or_else(|| load_token(&c)).map(Some)
                };
                match token.and_then(|t| t.map(|t| make_client(c.clone(), t)).transpose()) {
                    Err(e) => Err(e),
                    Ok(None) => Ok(Reply::NoSavedToken),
                    Ok(Some(mut cl)) => match rt.block_on(cl.connect()) {
                        Err(e) => Err(e),
                        Ok(id) => {
                            let warning = if remember {
                                vault(&c).and_then(|v|v.set_password(cl.token_value()).map_err(|_|"Conectado apenas nesta sessão: não foi possível salvar o token no cofre".into())).err()
                            } else {
                                None
                            };
                            let images = cl.media_client().ok();
                            cl.fork().map(|report| {
                                client = Some(cl);
                                let reconnect = from_vault || (remember && warning.is_none());
                                Reply::Connected(id, warning, images, Box::new(report), reconnect)
                            })
                        }
                    },
                }
            }
            op => {
                if let Some(c) = &mut client {
                    rt.block_on(async {
                        match op {
                            Op::Search(f, cursor, i, q) => {
                                c.team_field = f
                                    .team_field
                                    .as_ref()
                                    .map(|f| f.id.clone())
                                    .or_else(|| c.default_team_field.clone());
                                c.search(&f, cursor).await.map(|p| Reply::Page(p, i, q))
                            }
                            Op::Detail(k) => c.detail(&k).await.map(|d| Reply::Detail(Box::new(d))),
                            Op::Comments(k, o) => c
                                .comments(&k, o)
                                .await
                                .map(|(a, n)| Reply::Comments(k, a, n, o)),
                            Op::Worklogs(k, o) => c
                                .worklogs(&k, o)
                                .await
                                .map(|(a, n)| Reply::Worklogs(k, a, n, o)),
                            Op::Suggest(p, s, field) => {
                                c.team_field = field.or_else(|| c.default_team_field.clone());
                                c.suggestions(p, &s)
                                    .await
                                    .map(|a| Reply::Suggestions(p, s, a))
                            }
                            Op::Write(d) => c
                                .post_worklog(&d)
                                .await
                                .map(|_| Reply::Written(d.key, d.minutes * 60)),
                            _ => unreachable!(),
                        }
                    })
                } else {
                    Err("Conecte sua conta Jira".into())
                }
            }
        };
        if tx
            .send(Response {
                epoch: r.epoch,
                result,
                write,
                scope,
            })
            .is_err()
        {
            break;
        }
    }
}
fn read_profile(path: &Path) -> Result<(Profile, Option<Vec<u8>>), String> {
    use std::io::Read;
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Profile::default(), None));
        }
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 1024 * 1024 => (),
        _ => return Err("Preferências Jira inválidas/inacessíveis; arquivo preservado".into()),
    }
    let mut b = vec![];
    std::fs::File::open(path)
        .and_then(|f| f.take(1024 * 1024 + 1).read_to_end(&mut b))
        .map_err(|_| "Não foi possível ler as preferências Jira")?;
    if b.len() > 1024 * 1024 {
        return Err("Preferências Jira acima de1MiB".into());
    }
    let p: Profile = serde_json::from_slice(&b)
        .map_err(|_| "Preferências Jira inválidas; arquivo preservado")?;
    p.validate()?;
    Ok((p, Some(b)))
}
fn save_profile(path: &Path, p: &Profile, expected: &mut Option<Vec<u8>>) -> Result<(), String> {
    use std::io::Write;
    p.validate()?;
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Preferências Jira recusam link simbólico".into());
    }
    let parent = path.parent().ok_or("Caminho do perfil inválido")?;
    std::fs::create_dir_all(parent).map_err(|_| "Não foi possível criar o diretório Jira")?;
    let lock_path = path.with_extension("jira.lock");
    let _lock = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|_| "Preferências Jira ocupadas por outra sessão; aguarde antes de salvar")?;
    struct Guard(PathBuf);
    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = Guard(lock_path);
    if read_profile(path)?.1 != *expected {
        return Err("As preferências Jira foram alteradas por outra sessão. Reabra a IDE para carregar a versão atual; arquivo preservado.".into());
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Não foi possível preparar as preferências Jira")?;
    let bytes = serde_json::to_vec_pretty(p).map_err(|_| "Preferências Jira inválidas")?;
    file.write_all(&bytes)
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "Não foi possível gravar preferências Jira")?;
    file.persist(path)
        .map_err(|_| "Não foi possível publicar preferências Jira")?;
    *expected = Some(bytes);
    Ok(())
}
#[cfg(test)]
mod reconnect_tests;
#[cfg(test)]
mod tests;
