//! Application-facing persistence adapter; filesystem work is isolated in profiles::Worker.
use super::*;
use profiles::{Reply, StoredChat, StoredEntry};
impl ChatService {
    pub fn new(path: PathBuf) -> Self {
        let mut service = Self::default();
        match profiles::Worker::new(path.clone()) {
            Ok(worker) => {
                service.profile_worker = Some(worker);
                service.profile_pending = 1;
                service.profile_loaded = false;
                service.profile_path = Some(path);
            }
            Err(error) => service.profile_error = Some(error),
        }
        service
    }
    pub fn profile_pending(&self) -> bool {
        self.profile_pending > 0 || self.profile_dirty
    }
    pub fn profile_ready(&self) -> bool {
        self.profile_worker.is_none() || self.profile_loaded
    }
    pub fn workspace(&self) -> Option<&std::path::Path> {
        self.profile_root.as_deref()
    }
    /// Last explicitly configured defaults for this provider in the current
    /// workspace. Selecting an old conversation never updates this preference.
    pub fn preferred_config(&self, provider: Provider) -> Option<&Config> {
        self.profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))?
            .defaults
            .iter()
            .find(|config| config.provider == provider)
    }
    fn remember_default(&mut self, config: Config) {
        let Some(root) = &self.profile_root else {
            return;
        };
        let workspace = self.profile.workspaces.entry(root.clone()).or_default();
        workspace
            .defaults
            .retain(|previous| previous.provider != config.provider);
        workspace.defaults.push(config);
        self.profile_dirty = true;
    }
    pub fn conversations(&self) -> Vec<(String, String)> {
        self.profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))
            .map(|w| {
                let mut chats: Vec<_> = w.chats.iter().rev().collect();
                // Stable sorting preserves reverse insertion order for legacy
                // timestamps (zero) and ties, without rewriting the profile.
                chats.sort_by_key(|chat| std::cmp::Reverse(chat.updated_at));
                chats
                    .into_iter()
                    .map(|c| (c.id.clone(), c.title.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Unix milliseconds of the last transcript change; absent for legacy
    /// records and unknown IDs. Selecting or editing a draft is not activity.
    pub fn chat_updated_at(&self, id: &str) -> Option<u64> {
        let chat = self
            .profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))?
            .chats
            .iter()
            .find(|chat| chat.id == id)?;
        (chat.updated_at != 0).then_some(chat.updated_at)
    }
    /// Text only. Attachments and their bytes never enter the profile.
    pub fn saved_draft(&self) -> Option<&str> {
        let root = self.profile_root.as_ref()?;
        let id = self.active_chat.as_ref()?;
        self.profile
            .workspaces
            .get(root)?
            .chats
            .iter()
            .find(|chat| &chat.id == id)
            .map(|chat| chat.draft.as_str())
    }
    /// Checkpoint a composer whose owner was parked during application polling.
    /// The explicit existing identity prevents a late UI draft from being saved
    /// to whichever conversation happens to be selected now.
    pub fn checkpoint_draft(
        &mut self,
        workspace: &std::path::Path,
        id: &str,
        draft: &str,
    ) -> Result<(), String> {
        if !self.profile_ready() {
            return Err("Aguarde o perfil do AI Chat".into());
        }
        if draft.len() > MAX_PROMPT {
            return Err("Rascunho maior que64KiB".into());
        }
        let chat = self
            .profile
            .workspaces
            .get_mut(workspace)
            .and_then(|workspace| workspace.chats.iter_mut().find(|chat| chat.id == id))
            .ok_or("Conversa não pertence ao projeto informado para guardar o rascunho")?;
        let body_bytes = chat
            .entries
            .iter()
            .map(|entry| entry.text.len() + entry.detail.len())
            .sum::<usize>();
        if body_bytes.saturating_add(draft.len()) > profiles::MAX_CHAT_BYTES {
            return Err("Rascunho e histórico da conversa excedem2MiB".into());
        }
        if chat.draft != draft {
            chat.draft = draft.to_owned();
            self.profile_dirty = true;
        }
        Ok(())
    }
    pub fn set_draft(&mut self, draft: &str) -> Result<(), String> {
        if draft.len() > MAX_PROMPT {
            return Err("Rascunho maior que64KiB".into());
        }
        if !self.profile_ready() {
            return Err("Aguarde o perfil do AI Chat".into());
        }
        // A flush of an untouched Home composer does not create empty history.
        if self.active_chat.is_none() && draft.is_empty() {
            return Ok(());
        }
        if self.saved_draft() == Some(draft) {
            return Ok(());
        }
        let bytes = self
            .entries
            .iter()
            .map(|entry| entry.text.len() + entry.detail.len())
            .sum::<usize>();
        if bytes.saturating_add(draft.len()) > profiles::MAX_CHAT_BYTES {
            return Err("Rascunho e histórico da conversa excedem2MiB".into());
        }
        if self.active_chat.is_none() {
            self.ensure_new_chat_capacity()?;
            self.store_current();
        }
        let root = self
            .profile_root
            .as_ref()
            .ok_or("Abra um projeto para guardar o rascunho")?;
        let id = self
            .active_chat
            .as_ref()
            .ok_or("Conversa indisponível para guardar o rascunho")?;
        let chat = self
            .profile
            .workspaces
            .get_mut(root)
            .and_then(|workspace| workspace.chats.iter_mut().find(|chat| &chat.id == id))
            .ok_or("Conversa não pertence ao projeto aberto")?;
        chat.draft = draft.to_owned();
        self.profile_dirty = true;
        Ok(())
    }
    pub fn external_history_provider(&self, id: &str) -> Option<Provider> {
        let chat = self
            .profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))
            .and_then(|w| w.chats.iter().find(|c| c.id == id))?;
        chat.session.as_ref()?;
        match chat.provider.as_str() {
            "codex" => Some(Provider::Codex),
            "claude" => Some(Provider::Claude),
            _ => None,
        }
    }
    pub(super) fn ensure_workspace_capacity(&self) -> Result<(), String> {
        let root = self
            .profile_root
            .as_ref()
            .ok_or("Abra um projeto para usar AI Chat")?;
        if !self.profile.workspaces.contains_key(root)
            && self.profile.workspaces.len() >= profiles::MAX_WORKSPACES
        {
            return Err("Limite de64 projetos no histórico AI; nenhuma tarefa foi iniciada".into());
        }
        Ok(())
    }
    pub(super) fn ensure_new_chat_capacity(&self) -> Result<(), String> {
        self.ensure_workspace_capacity()?;
        if self
            .profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))
            .is_some_and(|workspace| workspace.chats.len() >= profiles::MAX_CHATS)
        {
            return Err(
                "Limite de16 conversas neste projeto; apague uma conversa antiga para criar outra"
                    .into(),
            );
        }
        Ok(())
    }
    pub(super) fn configure(&mut self, config: Config) -> Result<(), String> {
        if !self.profile_ready() {
            return Err("Aguarde o perfil do AI Chat".into());
        }
        if self.is_running() {
            return Err(
                "Interrompa o turno desta conversa antes de alterar suas permissões ou modelo"
                    .into(),
            );
        }
        if config.program.trim().is_empty()
            || config.program.len() > 4096
            || config.model.len() > 256
            || config.effort.len() > 32
        {
            return Err("Configuração do agente vazia ou excessiva".into());
        }
        if self.profile_root.is_some() {
            self.ensure_workspace_capacity()?;
        }
        if self.config.provider != config.provider || self.config.program != config.program {
            if self.session.is_some() || !self.entries.is_empty() {
                self.ensure_new_chat_capacity()?;
                self.park_current();
                self.clear_current();
                self.config_revision += 1;
            }
            self.models.clear();
            self.limits = None;
            self.limits_error = None;
        }
        self.remember_default(config.clone());
        self.config = config;
        self.store_current();
        Ok(())
    }
    pub(super) fn select_chat(&mut self, id: String) -> Result<(), String> {
        if !self.profile_ready() {
            return Err("Aguarde o perfil do AI Chat".into());
        }
        let root = self.profile_root.clone().ok_or("Abra um projeto")?;
        if !self
            .profile
            .workspaces
            .get(&root)
            .is_some_and(|w| w.chats.iter().any(|c| c.id == id))
        {
            return Err("Conversa não pertence ao projeto aberto".into());
        }
        if self.active_chat.as_deref() == Some(id.as_str()) {
            return Ok(());
        }
        self.park_current();
        self.profile.workspaces.get_mut(&root).unwrap().active = Some(id);
        self.restore_current();
        self.profile_dirty = true;
        Ok(())
    }
    pub(super) fn delete_chat(&mut self, id: String) -> Result<(), String> {
        if !self.profile_ready() {
            return Err("Aguarde o perfil do AI Chat".into());
        }
        if !self.profile_writable {
            return Err(
                "Histórico somente em memória: resolva o erro de gravação antes de apagar".into(),
            );
        }
        let root = self.profile_root.clone().ok_or("Abra um projeto")?;
        if self.key_running(&root, &id) {
            return Err("Interrompa esta conversa antes de apagá-la".into());
        }
        let chat = self
            .profile
            .workspaces
            .get(&root)
            .and_then(|w| w.chats.iter().find(|c| c.id == id))
            .cloned()
            .ok_or("Conversa não pertence ao projeto aberto")?;
        self.store_current();
        if chat.provider == "codex"
            && let Some(session) = chat.session
        {
            let request = process::Request {
                ephemeral: false,
                config: Config {
                    provider: Provider::Codex,
                    program: chat.program,
                    model: chat.model,
                    effort: chat.effort,
                    mode: stored_mode(&chat.mode),
                },
                workspace: root.clone(),
                action: Action::DeleteThread(session),
                session: None,
                attachments: None,
            };
            if self.running_count() >= MAX_RUNNING_CHATS {
                return Err("Limite de8 conversas em execução; aguarde antes de apagar".into());
            }
            let original = self.active_chat.clone();
            if original.as_deref() != Some(id.as_str()) {
                self.select_chat(id.clone())?;
            }
            let launched = self.launch(request);
            if launched.is_ok() {
                self.pending_delete = Some((root, id));
            }
            if let Some(original) = original
                && self.active_chat.as_deref() != Some(original.as_str())
            {
                self.select_chat(original)?;
            }
            launched?;
        } else {
            self.remove_chat(&root, &id);
            if chat.provider == "claude" {
                self.notice=Some("Conversa removida do Codisseu. O histórico externo do Claude continua sob controle do CLI.".into());
            }
        }
        Ok(())
    }
    pub(super) fn remove_chat(&mut self, root: &std::path::Path, id: &str) {
        let Some(workspace) = self.profile.workspaces.get_mut(root) else {
            return;
        };
        let was_active = workspace.active.as_deref() == Some(id);
        workspace.chats.retain(|chat| chat.id != id);
        if was_active {
            workspace.active = workspace.chats.last().map(|c| c.id.clone());
        }
        self.background.remove(&(root.to_path_buf(), id.to_owned()));
        if was_active && self.profile_root.as_deref() == Some(root) && !self.polling_background {
            self.restore_current();
        }
        self.profile_dirty = true;
    }
    pub(super) fn store_current(&mut self) {
        let Some(root) = self.profile_root.clone() else {
            return;
        };
        if !self.profile_ready() {
            return;
        }
        if self.active_chat.is_none() {
            self.chat_sequence += 1;
            self.active_chat = Some(format!(
                "chat-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos(),
                self.chat_sequence
            ));
        }
        let id = self.active_chat.clone().unwrap();
        let config = &self.config;
        let entries = self
            .entries
            .iter()
            .map(|e| StoredEntry {
                id: e.id.clone(),
                role: e.role.into(),
                text: e.text.clone(),
                detail: e.detail.clone(),
            })
            .collect::<Vec<_>>();
        let previous = self
            .profile
            .workspaces
            .get(&root)
            .and_then(|workspace| workspace.chats.iter().find(|chat| chat.id == id));
        let updated_at = match previous {
            Some(chat) if chat.entries == entries => chat.updated_at,
            _ => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        };
        let title = self
            .entries
            .iter()
            .find(|e| e.role == "Você")
            .map(|e| {
                e.text
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(80)
                    .collect()
            })
            .unwrap_or_else(|| "Nova conversa".into());
        let record = StoredChat {
            id: id.clone(),
            title,
            provider: config.provider.label().to_lowercase(),
            program: config.program.clone(),
            model: config.model.clone(),
            effort: config.effort.clone(),
            mode: match config.mode {
                Mode::Agent => "agent",
                Mode::Consult => "consult",
                Mode::FullAccess => "full",
            }
            .into(),
            session: self.session.clone(),
            usage: self.usage.clone(),
            draft: self
                .profile
                .workspaces
                .get(&root)
                .and_then(|workspace| workspace.chats.iter().find(|chat| chat.id == id))
                .map(|chat| chat.draft.clone())
                .unwrap_or_default(),
            entries,
            updated_at,
        };
        if self.profile.workspaces.len() >= profiles::MAX_WORKSPACES
            && !self.profile.workspaces.contains_key(&root)
        {
            self.profile_error = Some("Limite de64 projetos no histórico AI".into());
            return;
        }
        let workspace = self.profile.workspaces.entry(root).or_default();
        if let Some(old) = workspace.chats.iter_mut().find(|c| c.id == id) {
            *old = record
        } else {
            if workspace.chats.len() >= profiles::MAX_CHATS {
                self.profile_error = Some("Limite de16 conversas neste projeto; apague uma conversa antiga para criar outra".into());
                return;
            }
            workspace.chats.push(record);
        }
        if !self.polling_background {
            workspace.active = Some(id);
        }
        self.profile_dirty = true;
        self.history_changed = false;
        self.history_snapshot = Some(Instant::now());
        self.prune_runtime();
    }
    pub(super) fn restore_current(&mut self) {
        self.clear_current();
        if let Some(config) = self
            .profile_root
            .as_ref()
            .and_then(|root| self.profile.workspaces.get(root))
            .and_then(|workspace| workspace.defaults.last())
        {
            self.config = config.clone();
        }
        let chat = self
            .profile_root
            .as_ref()
            .and_then(|r| self.profile.workspaces.get(r))
            .and_then(|w| {
                w.active
                    .as_ref()
                    .and_then(|id| w.chats.iter().find(|c| &c.id == id))
            })
            .cloned();
        if let Some(chat) = chat {
            let provider = if chat.provider == "claude" {
                Provider::Claude
            } else {
                Provider::Codex
            };
            self.config = Config {
                provider,
                program: chat.program,
                model: chat.model,
                effort: chat.effort,
                mode: stored_mode(&chat.mode),
            };
            self.entries = chat
                .entries
                .into_iter()
                .filter_map(|e| {
                    Some(Entry {
                        id: e.id,
                        role: match e.role.as_str() {
                            "Você" => "Você",
                            "Assistente" => "Assistente",
                            "Ferramenta" => "Ferramenta",
                            _ => return None,
                        },
                        text: e.text,
                        detail: e.detail,
                    })
                })
                .collect();
            self.user_sequence = self
                .entries
                .iter()
                .filter_map(|e| e.id.strip_prefix("user-")?.parse::<u64>().ok())
                .max()
                .unwrap_or(0);
            self.session = chat.session;
            self.usage = chat.usage;
            self.active_chat = Some(chat.id);
            if let Some(root) = &self.profile_root {
                self.binding = Some((self.config.clone(), root.clone()));
            }
        }
        self.restore_runtime();
        self.config_revision += 1;
    }
    pub(super) fn poll_profile(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            let Some(reply) = self.profile_worker.as_ref().and_then(|w| w.poll()) else {
                break;
            };
            changed = true;
            self.profile_pending = self.profile_pending.saturating_sub(1);
            match reply {
                Reply::Loaded(result) => {
                    self.profile_loaded = true;
                    match result {
                        Ok(profile) => {
                            self.profile = profile;
                            self.restore_current();
                        }
                        Err(error) => {
                            self.profile_error = Some(error);
                            self.profile_writable = false;
                        }
                    }
                }
                Reply::Saved { result, .. } => {
                    if let Err(error) = result {
                        self.profile_error = Some(error);
                        self.profile_writable = false;
                    }
                }
            }
        }
        if self.profile_dirty
            && self.profile_writable
            && self.profile_ready()
            && self.profile_pending == 0
        {
            if let Some(worker) = &mut self.profile_worker {
                match worker.save(self.profile.clone()) {
                    Ok(_) => {
                        self.profile_pending += 1;
                        self.profile_dirty = false
                    }
                    Err(error) => {
                        self.profile_error = Some(error);
                        self.profile_writable = false;
                        self.profile_dirty = false
                    }
                }
            } else {
                self.profile_dirty = false;
            }
        }
        if !self.profile_writable {
            self.profile_dirty = false;
        }
        changed
    }
}

fn stored_mode(value: &str) -> Mode {
    match value {
        "consult" => Mode::Consult,
        "full" => Mode::FullAccess,
        _ => Mode::Agent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scoped_draft_checkpoint_rejects_missing_owners_and_shared_byte_overflow_without_mutation() {
        let mut chat = ChatService::default();
        let root = PathBuf::from("/workspace");
        chat.sync_workspace(Some(&root));
        chat.dispatch(Command::NewConversation, None).unwrap();
        let id = chat.active_chat_id().unwrap().to_owned();
        chat.poll();
        let before = chat.profile.clone();
        for (owner, owner_id, draft) in [
            (
                std::path::Path::new("/missing"),
                id.as_str(),
                "draft".to_owned(),
            ),
            (root.as_path(), "missing-chat", "draft".to_owned()),
            (root.as_path(), id.as_str(), "x".repeat(MAX_PROMPT + 1)),
        ] {
            assert!(chat.checkpoint_draft(owner, owner_id, &draft).is_err());
            assert_eq!(chat.profile, before);
            assert!(!chat.profile_dirty);
        }
        let record = &mut chat.profile.workspaces.get_mut(&root).unwrap().chats[0];
        record.entries = (0..profiles::MAX_CHAT_BYTES / MAX_TEXT)
            .map(|n| StoredEntry {
                id: format!("entry-{n}"),
                role: "Assistente".into(),
                text: "x".repeat(MAX_TEXT),
                detail: String::new(),
            })
            .collect();
        let before = chat.profile.clone();
        assert!(chat.checkpoint_draft(&root, &id, "x").is_err());
        assert_eq!(chat.profile, before);
        assert!(!chat.profile_dirty);
        assert_eq!(chat.active_chat_id(), Some(id.as_str()));
        assert!(!chat.any_running());
    }
    #[test]
    fn scoped_draft_checkpoint_waits_for_profile_loading_without_allocating_an_owner() {
        let directory = tempfile::tempdir().unwrap();
        let mut chat = ChatService::new(directory.path().join("profile.json"));
        let before = chat.profile.clone();
        assert!(
            chat.checkpoint_draft(std::path::Path::new("/workspace"), "chat", "draft")
                .unwrap_err()
                .contains("Aguarde")
        );
        assert_eq!(chat.profile, before);
        assert!(!chat.profile_dirty);
        assert!(chat.active_chat_id().is_none());
    }
    #[test]
    fn recent_activity_order_ignores_selection_configuration_and_unsent_drafts() {
        let mut chat = ChatService::default();
        let root = PathBuf::from("/workspace");
        chat.sync_workspace(Some(&root));
        chat.dispatch(Command::NewConversation, None).unwrap();
        let a = chat.active_chat_id().unwrap().to_owned();
        assert!(chat.chat_updated_at(&a).unwrap() > 0);
        chat.dispatch(Command::NewConversation, None).unwrap();
        let b = chat.active_chat_id().unwrap().to_owned();
        let workspace = chat.profile.workspaces.get_mut(&root).unwrap();
        workspace.chats[0].updated_at = 1;
        workspace.chats[1].updated_at = 2;
        assert_eq!(chat.conversations()[0].0, b);
        chat.dispatch(Command::SelectChat(a.clone()), None).unwrap();
        chat.set_draft("Rascunho sem enviar").unwrap();
        let mut config = chat.config.clone();
        config.effort = "high".into();
        chat.dispatch(Command::Configure(config), None).unwrap();
        chat.poll();
        assert_eq!(chat.chat_updated_at(&a), Some(1));
        assert_eq!(chat.conversations()[0].0, b);
        chat.push_entry(
            Entry {
                id: "user-1".into(),
                role: "Você",
                text: "Mensagem enviada".into(),
                detail: String::new(),
            },
            false,
        );
        chat.store_current();
        assert!(chat.chat_updated_at(&a).unwrap() > 2);
        assert_eq!(chat.conversations()[0].0, a);
        assert_eq!(chat.chat_updated_at(&b), Some(2));
        assert_eq!(chat.chat_updated_at("unknown"), None);
        // No new activity: re-checkpointing leaves the recorded instant intact.
        let at = chat.chat_updated_at(&a);
        chat.store_current();
        assert_eq!(chat.chat_updated_at(&a), at);
    }
    #[test]
    fn legacy_unknown_times_keep_reverse_insertion_order_without_fabricated_ages() {
        let mut chat = ChatService::default();
        let root = PathBuf::from("/workspace");
        chat.sync_workspace(Some(&root));
        for _ in 0..3 {
            chat.dispatch(Command::NewConversation, None).unwrap();
        }
        let workspace = chat.profile.workspaces.get_mut(&root).unwrap();
        for record in &mut workspace.chats {
            record.updated_at = 0;
        }
        let expected: Vec<_> = workspace
            .chats
            .iter()
            .rev()
            .map(|record| record.id.clone())
            .collect();
        assert_eq!(
            chat.conversations()
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            expected
        );
        chat.dispatch(Command::SelectChat(expected[2].clone()), None)
            .unwrap();
        chat.store_current();
        assert_eq!(chat.chat_updated_at(&expected[2]), None);
        assert_eq!(chat.conversations()[0].0, expected[0]);
    }
    #[test]
    fn full_workspace_registry_refuses_start_and_new_before_allocating_chat_or_process() {
        let mut chat = ChatService::default();
        for n in 0..profiles::MAX_WORKSPACES {
            chat.profile.workspaces.insert(
                PathBuf::from(format!("/workspace-{n}")),
                profiles::StoredWorkspace::default(),
            );
        }
        let root = PathBuf::from("/workspace-extra");
        chat.sync_workspace(Some(&root));
        assert!(
            chat.dispatch(Command::NewConversation, None)
                .unwrap_err()
                .contains("64")
        );
        assert!(
            chat.dispatch(
                Command::Start {
                    config: Config::default(),
                    action: Action::Send("hello".into())
                },
                Some(root.clone())
            )
            .unwrap_err()
            .contains("64")
        );
        assert_eq!(chat.active_chat_id(), None);
        assert!(!chat.any_running());
        assert!(chat.entries.is_empty());
        assert_eq!(chat.profile.workspaces.len(), profiles::MAX_WORKSPACES);
        assert!(!chat.profile.workspaces.contains_key(&root));
    }
    #[test]
    fn new_chat_limit_preserves_selected_record_and_configuration() {
        let mut chat = ChatService::default();
        chat.sync_workspace(Some(std::path::Path::new("/workspace")));
        for _ in 0..profiles::MAX_CHATS {
            chat.dispatch(Command::NewConversation, None).unwrap();
        }
        let id = chat.active_chat_id().unwrap().to_owned();
        let config = chat.config.clone();
        let conversations = chat.conversations();
        assert!(
            chat.dispatch(Command::NewConversation, None)
                .unwrap_err()
                .contains("16")
        );
        assert_eq!(chat.active_chat_id(), Some(id.as_str()));
        assert_eq!(chat.config, config);
        assert_eq!(chat.conversations(), conversations);
    }
    #[test]
    fn draft_updates_are_scoped_idempotent_and_preserved_by_snapshots() {
        let mut chat = ChatService::default();
        chat.sync_workspace(Some(std::path::Path::new("/workspace")));
        assert_eq!(chat.saved_draft(), None);
        chat.set_draft("").unwrap();
        assert!(chat.active_chat_id().is_none());
        chat.set_draft("ação 🌧 rascunho A").unwrap();
        let a = chat.active_chat_id().unwrap().to_owned();
        chat.poll();
        assert!(!chat.profile_dirty);
        chat.set_draft("ação 🌧 rascunho A").unwrap();
        assert!(
            !chat.profile_dirty,
            "unchanged draft must not schedule profile writes"
        );
        chat.store_current();
        assert_eq!(chat.saved_draft(), Some("ação 🌧 rascunho A"));
        assert!(chat.set_draft(&"x".repeat(MAX_PROMPT + 1)).is_err());
        assert_eq!(chat.saved_draft(), Some("ação 🌧 rascunho A"));
        chat.dispatch(Command::NewConversation, None).unwrap();
        assert_eq!(chat.saved_draft(), Some(""));
        chat.set_draft("rascunho B").unwrap();
        chat.dispatch(Command::SelectChat(a), None).unwrap();
        assert_eq!(chat.saved_draft(), Some("ação 🌧 rascunho A"));
        chat.set_draft("").unwrap();
        chat.store_current();
        assert_eq!(chat.saved_draft(), Some(""));
        assert!(!chat.any_running());
    }
    #[test]
    fn streamed_transcript_respects_saved_draft_budget() {
        let mut chat = ChatService::default();
        chat.sync_workspace(Some(std::path::Path::new("/workspace")));
        chat.set_draft(&"x".repeat(MAX_PROMPT)).unwrap();
        for n in 0..9 {
            chat.push_entry(
                Entry {
                    id: n.to_string(),
                    role: "Assistente",
                    text: "y".repeat(MAX_TEXT),
                    detail: String::new(),
                },
                false,
            );
        }
        let body: usize = chat
            .entries
            .iter()
            .map(|entry| entry.text.len() + entry.detail.len())
            .sum();
        assert!(body + chat.saved_draft().unwrap().len() <= profiles::MAX_CHAT_BYTES);
        chat.store_current();
        assert_eq!(chat.saved_draft().unwrap().len(), MAX_PROMPT);
    }
    #[test]
    fn new_chat_inherits_catalog_only_for_its_exact_provider_and_program() {
        let mut chat = ChatService::default();
        chat.sync_workspace(Some(std::path::Path::new("/workspace")));
        chat.dispatch(Command::NewConversation, None).unwrap();
        let a = chat.active_chat_id().unwrap().to_owned();
        let model = models::Model {
            id: "codex-runtime-model".into(),
            ..models::Model::default()
        };
        chat.models = vec![model.clone()];
        let config = chat.config.clone();
        chat.set_draft("A: rascunho").unwrap();
        chat.dispatch(Command::NewConversation, None).unwrap();
        assert_eq!(chat.models.as_slice(), std::slice::from_ref(&model));
        assert_eq!(chat.config, config);
        assert!(chat.entries.is_empty());
        assert!(chat.usage.is_none());
        assert_eq!(chat.saved_draft(), Some(""));
        let mut other = chat.config.clone();
        other.program = "another-codex-cli".into();
        chat.dispatch(Command::Configure(other), None).unwrap();
        assert!(
            chat.models.is_empty(),
            "different executable cannot inherit the old catalog"
        );
        chat.models = vec![models::Model {
            id: "other-program-model".into(),
            ..models::Model::default()
        }];
        let mut other = chat.config.clone();
        other.provider = Provider::Claude;
        chat.dispatch(Command::Configure(other), None).unwrap();
        assert!(
            chat.models.is_empty(),
            "different provider cannot inherit the old catalog"
        );
        chat.dispatch(Command::SelectChat(a), None).unwrap();
        assert_eq!(chat.models, [model]);
        assert_eq!(chat.config, config);
        assert_eq!(chat.saved_draft(), Some("A: rascunho"));
    }
}
