//! Native client state for optional agent CLIs. No GUI, terminal emulation or credentials.
pub mod attachments;
pub mod commit_message;
pub mod copilot;
mod history;
pub mod models;
mod process;
pub mod profiles;
mod protocol;
mod runtime;
pub mod usage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Instant,
};

pub const MAX_TEXT: usize = 256 * 1024;
pub const MAX_PROMPT: usize = 64 * 1024;
const MAX_ENTRIES: usize = 128;
pub const MAX_RUNNING_CHATS: usize = 8;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[default]
    Codex,
    Claude,
}
impl Provider {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Consult,
    #[default]
    Agent,
    FullAccess,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: Provider,
    pub program: String,
    pub model: String,
    pub mode: Mode,
    pub effort: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::Codex,
            program: "codex".into(),
            model: String::new(),
            mode: Mode::Agent,
            effort: String::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub role: &'static str,
    pub text: String,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Approval {
    pub id: Value,
    pub description: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Pronto",
            Self::Running => "Trabalhando…",
            Self::Cancelling => "Interrompendo…",
            Self::Completed => "Concluído",
            Self::Cancelled => "Interrompido",
            Self::Failed => "Falha",
        }
    }
}
#[derive(Debug)]
pub enum Action {
    Send(String),
    SendWithAttachments {
        text: String,
        display_text: String,
        attachments: Vec<attachments::AttachmentInput>,
    },
    Models,
    Usage,
    /// Internal deletion operation; public callers use Command::DeleteChat.
    DeleteThread(String),
    Check,
    Login,
}
impl Action {
    pub fn prompt(&self) -> Option<&str> {
        match self {
            Self::Send(text) | Self::SendWithAttachments { text, .. } => Some(text),
            _ => None,
        }
    }
    pub fn inputs(&self) -> &[attachments::AttachmentInput] {
        match self {
            Self::SendWithAttachments { attachments, .. } => attachments,
            _ => &[],
        }
    }
}
#[derive(Debug)]
pub enum Command {
    Start { config: Config, action: Action },
    Configure(Config),
    SelectChat(String),
    DeleteChat(String),
    Cancel,
    NewConversation,
    Approve { id: Value, accept: bool },
}
#[derive(Debug)]
enum Event {
    Entry(Entry, bool),
    Session(String),
    Approval(Approval),
    Account(bool),
    Models(Vec<models::Model>),
    Usage(usage::Tokens),
    Limits(usage::Limits),
    LimitsUnavailable(String),
    LoginUrl(String),
    Note(String),
    End(Result<bool, String>),
}
struct Running {
    cancel: Arc<AtomicBool>,
    replies: SyncSender<(Value, bool)>,
    events: Receiver<Event>,
    worker: std::thread::JoinHandle<()>,
}
pub struct ChatService {
    // Purpose-specific ephemeral clients never persist a Codex thread. Normal
    // AI Chat keeps its established resumable behavior.
    ephemeral: bool,
    pub entries: Vec<Entry>,
    pub models: Vec<models::Model>,
    pub usage: Option<usage::Tokens>,
    pub limits: Option<usage::Limits>,
    pub limits_error: Option<String>,
    pending_delete: Option<(PathBuf, String)>,
    pub config: Config,
    pub config_revision: u64,
    pub profile_error: Option<String>,
    profile_worker: Option<profiles::Worker>,
    profile: profiles::Profile,
    profile_pending: usize,
    profile_dirty: bool,
    profile_loaded: bool,
    profile_writable: bool,
    profile_root: Option<PathBuf>,
    profile_path: Option<PathBuf>,
    active_chat: Option<String>,
    chat_sequence: u64,
    pub approvals: Vec<Approval>,
    pub state: State,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub authenticated: Option<bool>,
    pub login_url: Option<String>,
    pub elapsed_seconds: u64,
    session: Option<String>,
    binding: Option<(Config, PathBuf)>,
    running: Option<Running>,
    started: Option<Instant>,
    user_sequence: u64,
    background: BTreeMap<(PathBuf, String), runtime::Conversation>,
    polling_background: bool,
    history_changed: bool,
    history_snapshot: Option<Instant>,
}
impl Default for ChatService {
    fn default() -> Self {
        Self {
            ephemeral: false,
            entries: Vec::new(),
            models: Vec::new(),
            usage: None,
            limits: None,
            limits_error: None,
            pending_delete: None,
            config: Config::default(),
            config_revision: 0,
            profile_error: None,
            profile_worker: None,
            profile: profiles::Profile::default(),
            profile_pending: 0,
            profile_dirty: false,
            profile_loaded: true,
            profile_writable: true,
            profile_root: None,
            profile_path: None,
            active_chat: None,
            chat_sequence: 0,
            approvals: Vec::new(),
            state: State::Idle,
            error: None,
            notice: None,
            authenticated: None,
            login_url: None,
            elapsed_seconds: 0,
            session: None,
            binding: None,
            running: None,
            started: None,
            user_sequence: 0,
            background: BTreeMap::new(),
            polling_background: false,
            history_changed: false,
            history_snapshot: None,
        }
    }
}
impl ChatService {
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }
    pub fn session_id(&self) -> Option<&str> {
        self.session.as_deref()
    }
    pub fn dispatch(&mut self, command: Command, workspace: Option<PathBuf>) -> Result<(), String> {
        match command {
            Command::Configure(config) => self.configure(config),
            Command::SelectChat(id) => self.select_chat(id),
            Command::DeleteChat(id) => self.delete_chat(id),
            Command::Cancel => {
                self.cancel();
                Ok(())
            }
            Command::NewConversation => {
                if !self.profile_ready() {
                    return Err("Aguarde o perfil do AI Chat".into());
                }
                self.ensure_new_chat_capacity()?;
                let config = self
                    .preferred_config(self.config.provider)
                    .cloned()
                    .unwrap_or_else(|| self.config.clone());
                let models = if self.config.provider == config.provider
                    && self.config.program == config.program
                {
                    self.models.clone()
                } else {
                    Vec::new()
                };
                self.park_current();
                self.clear_current();
                self.config = config;
                // Runtime metadata is reusable only with the exact CLI whose
                // configuration the new conversation just inherited.
                self.models = models;
                self.config_revision += 1;
                self.store_current();
                Ok(())
            }
            Command::Approve { id, accept } => {
                if !self.approvals.iter().any(|a| a.id == id) {
                    return Err("Aprovação expirada".into());
                }
                self.running
                    .as_ref()
                    .ok_or("Agente encerrado")?
                    .replies
                    .try_send((id.clone(), accept))
                    .map_err(|_| "Fila de aprovações indisponível")?;
                self.approvals.retain(|a| a.id != id);
                Ok(())
            }
            Command::Start { config, action } => {
                if matches!(action, Action::DeleteThread(_)) {
                    return Err("Use a ação Apagar conversa do projeto".into());
                }
                let workspace = workspace.ok_or("Abra um projeto para usar AI Chat")?;
                self.sync_workspace(Some(&workspace));
                if self.is_running() {
                    return Err("Aguarde ou interrompa o turno desta conversa".into());
                }
                if self.running_count() >= MAX_RUNNING_CHATS {
                    return Err(
                        "Limite de8 conversas em execução; aguarde ou interrompa uma delas".into(),
                    );
                }
                if !self.profile_ready() {
                    return Err("Aguarde o perfil AI Chat".into());
                }
                self.ensure_workspace_capacity()?;
                if self.active_chat.is_none() {
                    self.ensure_new_chat_capacity()?;
                }
                if config.program.trim().is_empty() {
                    return Err("Configure o executável do agente".into());
                }
                if config.program.len() > 4096
                    || config.model.len() > 256
                    || config.effort.len() > 32
                {
                    return Err("Configuração excessivamente longa".into());
                }
                if let Some(prompt) = action.prompt()
                    && ((prompt.trim().is_empty() && action.inputs().is_empty())
                        || prompt.len() > MAX_PROMPT)
                {
                    return Err("Mensagem vazia ou maior que64KiB".into());
                }
                let binding = (config.clone(), workspace.clone());
                if self.binding.as_ref().is_some_and(|(old, root)| {
                    root != &workspace
                        || old.provider != config.provider
                        || old.program != config.program
                }) {
                    self.dispatch(Command::NewConversation, None)?;
                }
                self.binding = Some(binding);
                if self.config.provider != config.provider || self.config.program != config.program
                {
                    self.models.clear();
                }
                self.config = config.clone();
                if action.inputs().len() > attachments::MAX_ATTACHMENTS {
                    return Err("Limite de8 anexos por mensagem".into());
                }
                if let Some(prompt) = action.prompt() {
                    self.user_sequence += 1;
                    self.push_entry(
                        Entry {
                            id: format!("user-{}", self.user_sequence),
                            role: "Você",
                            text: match &action {
                                Action::SendWithAttachments { display_text, .. } => {
                                    if display_text.trim().is_empty() {
                                        "Anexos enviados".into()
                                    } else {
                                        display_text.clone()
                                    }
                                }
                                _ if prompt.is_empty() => "Anexos enviados".into(),
                                _ => prompt.into(),
                            },
                            detail: String::new(),
                        },
                        false,
                    );
                }
                // Assign the stable chat identity and persist the composed message
                // before events can arrive, including on a previously empty workspace.
                self.store_current();
                let request = process::Request {
                    ephemeral: self.ephemeral,
                    config,
                    workspace,
                    action,
                    session: self.session.clone(),
                    attachments: None,
                };
                self.launch(request)
            }
        }
    }
    fn launch(&mut self, request: process::Request) -> Result<(), String> {
        if self.running_count() >= MAX_RUNNING_CHATS {
            return Err("Limite de8 conversas em execução".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (out, events) = mpsc::sync_channel(64);
        let (replies, input) = mpsc::sync_channel(16);
        let stop = cancel.clone();
        let worker = std::thread::Builder::new()
            .name("codisseu-ai".into())
            .spawn(move || {
                let result = process::run(request, &stop, &out, input);
                // Always publish completion, including after cancellation. The service owns
                // this receiver until cleanup has finished; try_send avoids a drop deadlock.
                let _ = out.send(Event::End(result));
            })
            .map_err(|e| e.to_string())?;
        self.running = Some(Running {
            cancel,
            replies,
            events,
            worker,
        });
        self.state = State::Running;
        self.error = None;
        self.notice = None;
        self.login_url = None;
        self.approvals.clear();
        self.started = Some(Instant::now());
        Ok(())
    }
    pub fn is_deleting(&self) -> bool {
        self.pending_delete.is_some()
    }
    pub fn sync_workspace(&mut self, workspace: Option<&std::path::Path>) {
        if self.profile_root.as_deref() == workspace {
            return;
        }
        self.park_current();
        self.profile_root = workspace.map(std::path::Path::to_path_buf);
        if self
            .profile_path
            .as_ref()
            .zip(workspace)
            .is_some_and(|(path, root)| path.starts_with(root))
        {
            self.profile_writable = false;
            self.profile_dirty = false;
            self.profile_error = Some("Use um perfil AI Chat fora do diretório do projeto; histórico em memória nesta sessão".into());
        }
        self.restore_current();
    }
    pub fn cancel(&mut self) {
        // A delete may already have committed remotely. Let its acknowledgement
        // finish so the local record is removed consistently, including on close.
        if self.pending_delete.is_some() {
            return;
        }
        if let Some(r) = &self.running {
            r.cancel.store(true, Ordering::Release);
            self.state = State::Cancelling;
            self.approvals.clear();
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = self.poll_profile();
        // Every live conversation gets a bounded slice on each frame. Even a
        // noisy selected CLI cannot starve approvals or completion in another.
        let budget = 128 / self.running_count().max(1);
        changed |= self.poll_selected(budget);
        changed |= self.poll_background(budget);
        changed
    }
    fn poll_selected(&mut self, budget: usize) -> bool {
        let mut changed = false;
        if let Some(started) = self.started {
            self.elapsed_seconds = started.elapsed().as_secs();
        }
        for _ in 0..budget {
            let event = match self.running.as_ref().map(|r| r.events.try_recv()) {
                Some(Ok(e)) => e,
                Some(Err(mpsc::TryRecvError::Disconnected)) => {
                    Event::End(Err("Worker do agente encerrou sem resultado".into()))
                }
                _ => break,
            };
            changed = true;
            self.history_changed = true;
            if self.pending_delete.is_some()
                && matches!(
                    &event,
                    Event::Usage(_) | Event::Limits(_) | Event::LimitsUnavailable(_)
                )
            {
                continue;
            }
            match event {
                Event::Entry(entry, delta) => self.push_entry(entry, delta),
                Event::Session(id) => self.session = Some(id),
                Event::Approval(a) => {
                    if self.approvals.len() < 16 {
                        self.approvals.push(a)
                    } else {
                        self.cancel();
                        self.error = Some("Limite de aprovações excedido".into())
                    }
                }
                Event::Account(v) => self.authenticated = Some(v),
                Event::Models(models) => self.models = models,
                Event::Usage(tokens) => self.usage = Some(tokens),
                Event::Limits(limits) => {
                    self.limits = Some(limits);
                    self.limits_error = None;
                }
                Event::LimitsUnavailable(error) => {
                    self.limits = None;
                    self.limits_error = Some(error);
                }
                Event::LoginUrl(url) => self.login_url = Some(url),
                Event::Note(note) => self.notice = Some(note),
                Event::End(result) => {
                    let deleted = self.pending_delete.take();
                    let delete_succeeded = matches!(result, Ok(false));
                    self.state = match result {
                        Ok(true) => State::Cancelled,
                        Ok(false) => State::Completed,
                        Err(e) => {
                            self.error = Some(e);
                            State::Failed
                        }
                    };
                    self.running = None;
                    self.approvals.clear();
                    self.started = None;
                    if let Some((root, id)) = deleted {
                        if delete_succeeded {
                            self.remove_chat(&root, &id);
                        }
                        self.history_changed = false;
                    } else {
                        self.store_current();
                    }
                    break;
                }
            }
        }
        // Streaming checkpoints are throttled across events; completion above
        // always stores immediately. No transcript clone is made per token.
        if self.history_changed
            && self.pending_delete.is_none()
            && self
                .history_snapshot
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(750))
        {
            self.store_current();
        }
        changed
    }
    fn push_entry(&mut self, mut entry: Entry, delta: bool) {
        if entry.text.len() > MAX_TEXT || entry.detail.len() > MAX_TEXT {
            self.notice=Some("Saída longa: exibição limitada a256KiB por mensagem. A sessão do CLI mantém seu próprio histórico.".into());
        }
        clip(&mut entry.text, MAX_TEXT);
        clip(&mut entry.detail, MAX_TEXT);
        if let Some(old) = self
            .entries
            .iter_mut()
            .find(|old| old.id == entry.id && old.role == entry.role)
        {
            if delta {
                let room = MAX_TEXT.saturating_sub(old.text.len());
                clip(&mut entry.text, room);
                old.text.push_str(&entry.text)
            } else {
                *old = entry
            }
        } else {
            if self.entries.len() >= MAX_ENTRIES {
                self.entries.remove(0);
            }
            self.entries.push(entry)
        }
        // The persisted text draft shares this conversation's byte budget.
        let history_budget =
            profiles::MAX_CHAT_BYTES.saturating_sub(self.saved_draft().map_or(0, str::len));
        while self.entries.len() > 1
            && self
                .entries
                .iter()
                .map(|e| e.text.len() + e.detail.len())
                .sum::<usize>()
                > history_budget
        {
            self.entries.remove(0);
            self.notice = Some(
                "Histórico visível limitado a2MiB; a conversa do CLI continua na mesma sessão."
                    .into(),
            );
        }
    }
}
impl Drop for ChatService {
    fn drop(&mut self) {
        // Normal GUI shutdown already polls to completion. Direct service drop
        // also owns cleanup: close receivers before joining so a full output
        // queue cannot hold a worker while it reports its final result.
        self.finish_owned_processes();
    }
}
fn clip(s: &mut String, max: usize) {
    if s.len() > max {
        let mut at = max;
        while !s.is_char_boundary(at) {
            at -= 1
        }
        s.truncate(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_is_bounded_and_preserves_utf8() {
        let mut chat = ChatService::default();
        for n in 0..140 {
            chat.push_entry(
                Entry {
                    id: n.to_string(),
                    role: "Assistente",
                    text: "🌧".repeat(MAX_TEXT),
                    detail: String::new(),
                },
                false,
            );
        }
        assert!(chat.entries.len() <= 128);
        assert!(chat.entries.iter().all(|e| e.text.len() <= MAX_TEXT));
        assert!(
            chat.entries
                .iter()
                .map(|e| e.text.len() + e.detail.len())
                .sum::<usize>()
                <= 2 * 1024 * 1024
        );
    }
    #[test]
    fn empty_oversized_or_projectless_requests_never_start_process() {
        let mut chat = ChatService::default();
        for (prompt, root) in [
            (" ".into(), Some(PathBuf::from("/tmp"))),
            ("x".repeat(MAX_PROMPT + 1), Some(PathBuf::from("/tmp"))),
            ("hello".into(), None),
        ] {
            assert!(
                chat.dispatch(
                    Command::Start {
                        config: Config::default(),
                        action: Action::Send(prompt)
                    },
                    root
                )
                .is_err()
            );
            assert!(!chat.is_running());
            assert!(chat.entries.is_empty());
        }
    }
}
