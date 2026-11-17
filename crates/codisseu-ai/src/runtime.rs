//! Conversation-local state moves with the selected chat; process channels are
//! never shared or identified by provider item IDs across conversations.
use super::*;
#[derive(Default)]
pub(super) struct Conversation {
    entries: Vec<Entry>,
    models: Vec<models::Model>,
    usage: Option<usage::Tokens>,
    limits: Option<usage::Limits>,
    limits_error: Option<String>,
    pending_delete: Option<(PathBuf, String)>,
    config: Config,
    approvals: Vec<Approval>,
    state: State,
    error: Option<String>,
    notice: Option<String>,
    authenticated: Option<bool>,
    login_url: Option<String>,
    elapsed_seconds: u64,
    session: Option<String>,
    binding: Option<(Config, PathBuf)>,
    running: Option<Running>,
    started: Option<Instant>,
    user_sequence: u64,
    history_changed: bool,
    history_snapshot: Option<Instant>,
}
impl ChatService {
    pub fn active_chat_id(&self) -> Option<&str> {
        self.active_chat.as_deref()
    }
    pub fn running_count(&self) -> usize {
        usize::from(self.running.is_some())
            + self
                .background
                .values()
                .filter(|c| c.running.is_some())
                .count()
    }
    pub fn any_running(&self) -> bool {
        self.running_count() != 0
    }
    pub fn chat_status(&self, id: &str) -> Option<State> {
        if self.active_chat.as_deref() == Some(id) {
            return Some(self.state);
        }
        let root = self.profile_root.as_ref()?;
        self.background
            .get(&(root.clone(), id.into()))
            .map(|c| c.state)
            .or_else(|| {
                self.profile
                    .workspaces
                    .get(root)?
                    .chats
                    .iter()
                    .any(|c| c.id == id)
                    .then_some(State::Idle)
            })
    }
    pub fn chat_approval_count(&self, id: &str) -> usize {
        if self.active_chat.as_deref() == Some(id) {
            return self.approvals.len();
        }
        self.profile_root
            .as_ref()
            .and_then(|root| self.background.get(&(root.clone(), id.into())))
            .map_or(0, |c| c.approvals.len())
    }
    /// Shutdown signals all work. A deletion that may already have committed is
    /// allowed to deliver its ACK; normal close keeps polling until it finishes.
    pub fn cancel_all(&mut self) {
        self.cancel();
        for conversation in self.background.values_mut() {
            if conversation.pending_delete.is_none()
                && let Some(running) = &conversation.running
            {
                running.cancel.store(true, Ordering::Release);
                conversation.state = State::Cancelling;
                conversation.approvals.clear();
            }
        }
    }
    fn take_current(&mut self) -> Conversation {
        Conversation {
            entries: std::mem::take(&mut self.entries),
            models: std::mem::take(&mut self.models),
            usage: self.usage.take(),
            limits: self.limits.take(),
            limits_error: self.limits_error.take(),
            pending_delete: self.pending_delete.take(),
            config: std::mem::take(&mut self.config),
            approvals: std::mem::take(&mut self.approvals),
            state: std::mem::take(&mut self.state),
            error: self.error.take(),
            notice: self.notice.take(),
            authenticated: self.authenticated.take(),
            login_url: self.login_url.take(),
            elapsed_seconds: std::mem::take(&mut self.elapsed_seconds),
            session: self.session.take(),
            binding: self.binding.take(),
            running: self.running.take(),
            started: self.started.take(),
            user_sequence: std::mem::take(&mut self.user_sequence),
            history_changed: std::mem::take(&mut self.history_changed),
            history_snapshot: self.history_snapshot.take(),
        }
    }
    fn put_current(&mut self, c: Conversation) {
        self.entries = c.entries;
        self.models = c.models;
        self.usage = c.usage;
        self.limits = c.limits;
        self.limits_error = c.limits_error;
        self.pending_delete = c.pending_delete;
        self.config = c.config;
        self.approvals = c.approvals;
        self.state = c.state;
        self.error = c.error;
        self.notice = c.notice;
        self.authenticated = c.authenticated;
        self.login_url = c.login_url;
        self.elapsed_seconds = c.elapsed_seconds;
        self.session = c.session;
        self.binding = c.binding;
        self.running = c.running;
        self.started = c.started;
        self.user_sequence = c.user_sequence;
        self.history_changed = c.history_changed;
        self.history_snapshot = c.history_snapshot;
    }
    pub(super) fn finish_owned_processes(&mut self) {
        let mut processes = Vec::new();
        if let Some(running) = self.running.take() {
            processes.push(running);
        }
        for conversation in self.background.values_mut() {
            if let Some(running) = conversation.running.take() {
                processes.push(running);
            }
        }
        // Signal all before joining any, including a pending delete on an
        // unexpected drop. Normal cancel_all deliberately lets delete ACKs finish.
        for running in &processes {
            running.cancel.store(true, Ordering::Release);
        }
        for running in processes {
            let Running {
                events,
                replies,
                worker,
                ..
            } = running;
            drop(events);
            drop(replies);
            let _ = worker.join();
        }
    }
    pub(super) fn clear_current(&mut self) {
        self.put_current(Conversation::default());
        self.active_chat = None;
    }
    pub(super) fn park_current(&mut self) {
        // Changing a project with an untouched composer must not add an empty
        // chat. Explicit NewConversation and Start allocate identities themselves.
        if self.active_chat.is_some() || !self.entries.is_empty() || self.running.is_some() {
            self.store_current();
        }
        let key = self.profile_root.clone().zip(self.active_chat.clone());
        let mut conversation = self.take_current();
        if let Some(key) = key {
            if conversation.running.is_none() {
                conversation.entries.clear();
            }
            self.background.insert(key, conversation);
        }
        self.active_chat = None;
    }
    pub(super) fn restore_runtime(&mut self) {
        if let Some(key) = self.profile_root.clone().zip(self.active_chat.clone())
            && let Some(mut conversation) = self.background.remove(&key)
        {
            if conversation.running.is_none() {
                // Idle transcript already lives in the bounded profile, avoiding
                // a second complete copy for every visited conversation.
                conversation.entries = std::mem::take(&mut self.entries);
            }
            self.put_current(conversation);
        }
    }
    pub(super) fn key_running(&self, root: &std::path::Path, id: &str) -> bool {
        if self.profile_root.as_deref() == Some(root) && self.active_chat.as_deref() == Some(id) {
            return self.running.is_some();
        }
        self.background
            .get(&(root.to_path_buf(), id.into()))
            .is_some_and(|c| c.running.is_some())
    }
    pub(super) fn poll_background(&mut self, budget: usize) -> bool {
        let keys: Vec<_> = self
            .background
            .iter()
            .filter(|(_, c)| c.running.is_some())
            .map(|(key, _)| key.clone())
            .collect();
        if keys.is_empty() {
            return false;
        }
        let selected = self.take_current();
        let root = self.profile_root.take();
        let id = self.active_chat.take();
        self.polling_background = true;
        let mut changed = false;
        for key in keys {
            let Some(conversation) = self.background.remove(&key) else {
                continue;
            };
            self.put_current(conversation);
            self.profile_root = Some(key.0.clone());
            self.active_chat = Some(key.1.clone());
            changed |= self.poll_selected(budget);
            let mut conversation = self.take_current();
            if conversation.running.is_none() {
                conversation.entries.clear();
            }
            if self
                .profile
                .workspaces
                .get(&key.0)
                .is_some_and(|w| w.chats.iter().any(|c| c.id == key.1))
            {
                self.background.insert(key, conversation);
            }
        }
        self.polling_background = false;
        self.profile_root = root;
        self.active_chat = id;
        self.put_current(selected);
        changed
    }
    pub(super) fn prune_runtime(&mut self) {
        let profile = &self.profile;
        self.background.retain(|(root, id), c| {
            c.running.is_some()
                || profile
                    .workspaces
                    .get(root)
                    .is_some_and(|w| w.chats.iter().any(|chat| &chat.id == id))
        });
    }
}
