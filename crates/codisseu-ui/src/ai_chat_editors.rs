//! Unsaved composer state belongs to a conversation, never to the visible panel.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct EditorKey {
    pub workspace: Option<PathBuf>,
    pub chat: Option<String>,
}

pub(super) struct ComposerState {
    draft: String,
    context: Option<(String, String)>,
    attachments: attachments_ui::AttachmentUi,
    ide_context: bool,
    mention_query: String,
    mention_open: bool,
    mention_index: usize,
    mention_dismissed: Option<String>,
    error: Option<String>,
    codex: String,
    claude: String,
    text_edit: Option<egui::text_edit::TextEditState>,
}

impl AiChatUi {
    fn composer_has_state(&self) -> bool {
        !self.draft.is_empty()
            || self.context.is_some()
            || !self.attachments.is_empty()
            || !self.ide_context
    }
    fn take_composer(&mut self) -> ComposerState {
        ComposerState {
            draft: std::mem::take(&mut self.draft),
            context: self.context.take(),
            attachments: std::mem::take(&mut self.attachments),
            ide_context: std::mem::replace(&mut self.ide_context, true),
            mention_query: std::mem::take(&mut self.mention_query),
            mention_open: std::mem::take(&mut self.mention_open),
            mention_index: std::mem::take(&mut self.mention_index),
            mention_dismissed: self.mention_dismissed.take(),
            error: self.error.take(),
            codex: std::mem::replace(&mut self.codex, "codex".into()),
            claude: std::mem::replace(&mut self.claude, "claude".into()),
            text_edit: self.ui_context.as_ref().and_then(|ctx| {
                egui::text_edit::TextEditState::load(ctx, egui::Id::unique("codisseu.ai.composer"))
            }),
        }
    }

    fn restore_composer(&mut self, state: ComposerState) {
        self.draft = state.draft;
        self.context = state.context;
        self.attachments = state.attachments;
        self.ide_context = state.ide_context;
        self.mention_query = state.mention_query;
        self.mention_open = state.mention_open;
        self.mention_index = state.mention_index;
        self.mention_dismissed = state.mention_dismissed;
        self.error = state.error;
        self.codex = state.codex;
        self.claude = state.claude;
        if let Some(ctx) = &self.ui_context {
            state
                .text_edit
                .unwrap_or_else(new_text_state)
                .store(ctx, egui::Id::unique("codisseu.ai.composer"));
        }
    }
}

impl CodisseuApp {
    // Editor view activation can occur after polling, in this same frame.
    // Resolve the destination before rendering controls or submitting a prompt.
    pub(super) fn ai_sync_workspace(&mut self) {
        if self.service.ai.workspace() != self.service.ai_workspace() {
            self.ai_flush_draft();
            let root = self
                .service
                .ai_workspace()
                .map(std::path::Path::to_path_buf);
            self.service.ai.sync_workspace(root.as_deref());
            self.ai_workspace_changed();
        }
    }
    pub(crate) fn ai_flush_draft(&mut self) {
        if !self.service.ai.profile_ready() || self.service.ai.workspace().is_none() {
            return;
        }
        let current = EditorKey {
            workspace: self
                .service
                .ai
                .workspace()
                .map(std::path::Path::to_path_buf),
            chat: self.service.ai.active_chat_id().map(str::to_owned),
        };
        if self.ai_chat.editor_key.as_ref() != Some(&current) {
            return;
        }
        if self.service.ai.saved_draft() == Some(self.ai_chat.draft.as_str())
            || (self.ai_chat.draft.is_empty() && current.chat.is_none())
        {
            self.ai_chat.draft_edited_at = None;
            return;
        }
        match self.service.ai.set_draft(&self.ai_chat.draft) {
            Ok(()) => {
                self.ai_sync_editor(false);
                self.ai_chat.draft_observed = self.ai_chat.draft.clone();
                self.ai_chat.draft_edited_at = None;
            }
            Err(error) => self.ai_chat.error = Some(error),
        }
    }
    pub(super) fn ai_checkpoint_draft(&mut self, ctx: &egui::Context) {
        if self.ai_chat.draft_observed != self.ai_chat.draft {
            self.ai_chat.draft_observed = self.ai_chat.draft.clone();
            self.ai_chat.draft_edited_at = Some(std::time::Instant::now());
        }
        if let Some(edited) = self.ai_chat.draft_edited_at {
            if edited.elapsed() >= Duration::from_millis(750) {
                self.ai_flush_draft();
            } else {
                ctx.request_repaint_after(
                    Duration::from_millis(750).saturating_sub(edited.elapsed()),
                );
            }
        }
    }
    pub(super) fn ai_can_navigate(&mut self, command: &ai::Command) -> bool {
        let target = match command {
            ai::Command::NewConversation => Some(None),
            ai::Command::SelectChat(id) => Some(Some(id)),
            _ => None,
        };
        if let Some(target) = target
            && self.ai_chat.editors.len() >= ai::profiles::MAX_WORKSPACES * ai::profiles::MAX_CHATS
            && self.ai_chat.composer_has_state()
            && !target.is_some_and(|id| {
                self.ai_chat.editors.keys().any(|key| {
                    key.workspace.as_deref() == self.service.ai.workspace()
                        && key.chat.as_ref() == Some(id)
                })
            })
        {
            self.ai_chat.error=Some("Limite de rascunhos em memória atingido. Envie ou limpe o rascunho atual antes de abrir outra conversa.".into());
            return false;
        }
        true
    }
    pub(super) fn ai_sync_editor(&mut self, navigation: bool) {
        let key = EditorKey {
            workspace: self
                .service
                .ai
                .workspace()
                .map(std::path::Path::to_path_buf),
            chat: self.service.ai.active_chat_id().map(str::to_owned),
        };
        let previous = self.ai_chat.editor_key.replace(key.clone());
        if previous.is_none()
            && let Some(ctx) = &self.ai_chat.ui_context
        {
            new_text_state().store(ctx, egui::Id::unique("codisseu.ai.composer"));
        }
        let changed = previous.as_ref().is_some_and(|previous| previous != &key);
        let acquired_identity = !navigation
            && previous.as_ref().is_some_and(|previous| {
                previous.workspace == key.workspace && previous.chat.is_none() && key.chat.is_some()
            });
        if changed && !acquired_identity {
            // Do not queue a metadata job behind a running turn: a cancellation
            // must finish as cancelled, without immediately starting model discovery.
            self.ai_chat.catalog_requested =
                !self.service.ai.models.is_empty() || self.service.ai.is_running();
        }
        if changed && !acquired_identity {
            if let Some(previous) = &previous
                && let (Some(root), Some(chat)) = (&previous.workspace, &previous.chat)
                // Deletion must not recreate an intentionally removed owner.
                && (previous.workspace != key.workspace
                    || self.service.ai.conversations().iter().any(|(id, _)| id == chat))
                && let Err(error) = self.service.ai.checkpoint_draft(root, chat, &self.ai_chat.draft)
            {
                self.service.ai.profile_error = Some(error);
            }
            let destination = self.ai_chat.editors.remove(&key).or_else(|| {
                if self
                    .ai_chat
                    .protected_composer
                    .as_ref()
                    .is_some_and(|(saved, _)| saved == &key)
                {
                    self.ai_chat
                        .protected_composer
                        .take()
                        .map(|(_, state)| state)
                } else {
                    None
                }
            });
            let previous = previous.expect("changed editor has a previous key");
            let state = self.ai_chat.take_composer();
            if let Some(ctx) = &self.ai_chat.ui_context {
                new_text_state().store(ctx, egui::Id::unique("codisseu.ai.composer"));
            }
            // The backend caps the same history at64projects×16conversations.
            // Empty default composers do not consume a cache entry.
            let has_state = !state.draft.is_empty()
                || state.context.is_some()
                || !state.attachments.is_empty()
                || !state.ide_context
                || state.error.is_some();
            if has_state {
                if self.ai_chat.editors.len()
                    < ai::profiles::MAX_WORKSPACES * ai::profiles::MAX_CHATS
                {
                    self.ai_chat.editors.insert(previous, state);
                } else {
                    self.ai_chat.protected_composer = Some((previous, state));
                }
            }
            if let Some(state) = destination {
                self.ai_chat.restore_composer(state);
            }
            self.ai_chat.cache.clear();
            self.ai_chat.tools = Default::default();
            self.ai_chat.slash = Default::default();
            self.ai_chat.open_control = None;
            self.ai_chat.delete_confirm = None;
            self.ai_chat.focus = true;
            self.ai_chat.config_revision = u64::MAX;
        }
        if self.ai_chat.editors.len() < ai::profiles::MAX_WORKSPACES * ai::profiles::MAX_CHATS
            && let Some((saved, state)) = self.ai_chat.protected_composer.take()
        {
            self.ai_chat.editors.insert(saved, state);
        }
        if self.ai_chat.protected_composer.is_some() {
            self.ai_chat.error=Some("Limite de rascunhos em memória atingido. O rascunho anterior está preservado; retorne à conversa anterior ou libere um rascunho para continuar.".into());
        }
        if self.service.ai.profile_ready()
            && self.service.ai.workspace() == self.service.ai_workspace()
        {
            let conversations = self.service.ai.conversations();
            self.ai_chat.editors.retain(|cached, _| {
                cached.workspace != key.workspace
                    || cached.chat.is_none()
                    || conversations
                        .iter()
                        .any(|(id, _)| cached.chat.as_ref() == Some(id))
            });
        }
        let other_rgba: usize = self
            .ai_chat
            .editors
            .values()
            .map(|state| state.attachments.rgba_bytes())
            .sum::<usize>()
            + self
                .ai_chat
                .protected_composer
                .as_ref()
                .map_or(0, |(_, state)| state.attachments.rgba_bytes());
        self.ai_chat.attachments.reserve_other_rgba(other_rgba);
        self.ai_chat.attachments.set_scope(&key);
        if self.service.ai.profile_ready()
            && self.ai_chat.config_revision != self.service.ai.config_revision
        {
            let config = self.service.ai.config.clone();
            self.ai_chat.provider = config.provider;
            match config.provider {
                Provider::Codex => self.ai_chat.codex = config.program,
                Provider::Claude => self.ai_chat.claude = config.program,
            }
            self.ai_chat.model = config.model;
            self.ai_chat.effort = config.effort;
            self.ai_chat.mode = config.mode;
            self.ai_chat.config_revision = self.service.ai.config_revision;
        }
        if self.service.ai.profile_ready() && self.ai_chat.draft_loaded_key.as_ref() != Some(&key) {
            let saved = self.service.ai.saved_draft();
            if self.ai_chat.draft.is_empty()
                && let Some(text) = saved
            {
                self.ai_chat.draft = text.to_owned();
            }
            self.ai_chat.draft_observed = self.ai_chat.draft.clone();
            self.ai_chat.draft_edited_at = if saved.unwrap_or("") == self.ai_chat.draft {
                None
            } else {
                Some(std::time::Instant::now())
            };
            self.ai_chat.draft_loaded_key = Some(key);
        }
    }
}
fn new_text_state() -> egui::text_edit::TextEditState {
    let mut state = egui::text_edit::TextEditState::default();
    state.set_undoer(egui::util::undoer::Undoer::with_settings(
        egui::util::undoer::Settings {
            max_undos: 8,
            ..Default::default()
        },
    ));
    state
}
