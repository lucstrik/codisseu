//! Copilot suggestions are view state. Accept uses the shared guarded editor transaction.
use super::*;
use codisseu_core::{
    EditKind, Selection, TextDelta,
    ai::copilot::{self as api, Client, Document, Suggestion},
};
use std::{cell::Cell, sync::Arc};
/// An empty IME update is a lifecycle notification, not an edit. Wayland can
/// emit it repeatedly when the input-method cursor rectangle is updated.
pub(super) fn text_change_event(event: &egui::Event) -> bool {
    match event {
        egui::Event::Text(text)
        | egui::Event::Paste(text)
        | egui::Event::Ime(egui::ImeEvent::Preedit { text, .. })
        | egui::Event::Ime(egui::ImeEvent::Commit(text)) => !text.is_empty(),
        egui::Event::Ime(egui::ImeEvent::DeleteSurrounding {
            before_chars,
            after_chars,
        }) => *before_chars > 0 || *after_chars > 0,
        egui::Event::Cut => true,
        _ => false,
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    document: DocumentId,
    view: codisseu_core::ViewId,
    path: PathBuf,
    version: u64,
    caret: usize,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SemanticChoice {
    pub range: std::ops::Range<usize>,
    pub text: String,
}
#[derive(Default)]
pub(super) struct CopilotUi {
    pub client: Option<Client>,
    pub settings: bool,
    attempted: Option<PathBuf>,
    observed: Option<Owner>,
    synced: Option<(DocumentId, u64)>,
    requested: Option<Owner>,
    manual_waiting: Option<Owner>,
    due: Option<f64>,
    shown: Cell<Option<(u64, usize)>>,
    workspace: Vec<PathBuf>,
    previous_ready: bool,
    was_blocked: bool,
    selected: Option<SemanticChoice>,
    continuation: Option<interactions::Continuation>,
    accepted_utf16: u32,
    palette_return: Option<std::time::Instant>,
    pub(super) program: String,
    pub error: Option<String>,
    trace_state: Option<serde_json::Value>,
}
impl CodisseuApp {
    fn copilot_diagnostic(&self, ctx: &egui::Context) -> serde_json::Value {
        let owner = self.copilot_owner();
        let c = self.copilot_ui.client.as_ref();
        serde_json::json!({"event":"ui_state",
            "focused":self.editor_focused(ctx),"blocked":self.copilot_blocked(ctx),
            "owner":owner.as_ref().map(|o|(o.version,o.caret)),
            "bytes":self.service.active_document().map(|d|d.len_bytes()),
            "signature":self.signature_popup.is_some(),"semantic":self.copilot_semantic_owns_input(),
            "snippet_live":self.snippet_ui.has_live(),"snippet_popup":self.snippet_ui.is_open(),
            "menu_bar":self.main_menu_open,"popup":egui::Popup::is_any_open(ctx),"modal":ctx.memory(|m|m.top_modal_layer().is_some()),
            "ready":c.is_some_and(|c|c.ready),"authenticated":c.is_some_and(|c|c.authenticated),
            "busy":c.is_some_and(|c|c.busy),"proposal":c.is_some_and(|c|c.suggestion.is_some()),
            "compatible":c.and_then(|c|c.suggestion.as_ref()).map(|s|self.copilot_semantic_compatible(s)),
            "due":self.copilot_ui.due.is_some(),"server_kind":c.map(|c|c.kind.as_str()),"has_error":c.is_some_and(|c|c.error.is_some()),"pending_notices":c.map_or(0,|c|c.messages.len())})
    }
    fn copilot_trace_state(&mut self, ctx: &egui::Context) {
        if std::env::var_os("CODISSEU_COPILOT_TRACE").is_none() {
            return;
        }
        let state = self.copilot_diagnostic(ctx);
        if self.copilot_ui.trace_state.as_ref() != Some(&state) {
            eprintln!("COPILOT_QA {state}");
            self.copilot_ui.trace_state = Some(state);
        }
    }
    fn copilot_owner(&self) -> Option<Owner> {
        let doc = self.service.active_document()?;
        let view = self.service.active_view()?;
        let sel = self.service.view(view)?.selection();
        let m = self
            .editor_bridge
            .view_metrics
            .get(&view)
            .unwrap_or(&self.editor_bridge.metrics);
        if doc.is_read_only()
            || doc.len_bytes() > api::MAX_DOCUMENT
            || !sel.is_caret()
            || m.document != Some(doc.id)
            || m.composing
            || m.selection_count != 1
        {
            return None;
        }
        Some(Owner {
            document: doc.id,
            view,
            path: doc.path.clone(),
            version: doc.version,
            caret: sel.head,
        })
    }
    fn copilot_blocked(&self, ctx: &egui::Context) -> bool {
        !self.editor_focused(ctx)
            || self.preferences_dialog
            || self.copilot_ui.settings
            || self.path_dialog.is_some()
            || self.confirm.is_some()
            || self.provider_dialog
            || self.exit_when_saved
            || self.project_search.open
            || self.snippet_ui.is_open()
            || self.snippet_ui.has_live()
            || self.git_ui.dialog.is_some()
            || self.git_ui.diff_open
            || self.transfer_modal_open()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some())
    }
    fn copilot_document(&self, owner: &Owner) -> Option<Document> {
        let doc = self
            .service
            .active_document()
            .filter(|d| d.id == owner.document && d.version == owner.version)?;
        let uri = if doc.is_untitled() {
            format!("untitled:codisseu/{}", doc.id)
        } else {
            api::file_uri(&doc.path).ok()?
        };
        Some(Document {
            uri,
            version: doc.version,
            language: api::language(&doc.path).into(),
            text: Arc::from(doc.text()),
            position: api::position(doc.text(), owner.caret)?,
        })
    }
    pub(super) fn copilot_poll(&mut self, ctx: &egui::Context) {
        let prefs = self.service.editor_preferences();
        if !prefs.copilot_enabled || self.exit_when_saved {
            self.copilot_ui.client = None;
            self.copilot_ui.attempted = None;
            self.copilot_ui.observed = None;
            self.copilot_ui.requested = None;
            self.copilot_ui.manual_waiting = None;
            self.copilot_ui.due = None;
            self.copilot_ui.previous_ready = false;
            self.copilot_ui.was_blocked = false;
            self.copilot_ui.selected = None;
            return;
        }
        let program = if prefs.copilot_program.is_empty() {
            api::default_program()
        } else {
            PathBuf::from(&prefs.copilot_program)
        };
        let roots = self.service.workspace_roots();
        if self.copilot_ui.attempted.as_ref() != Some(&program) {
            self.copilot_ui.client = None;
            self.copilot_ui.synced = None;
            self.copilot_ui.observed = None;
            self.copilot_ui.attempted = Some(program.clone());
            self.copilot_ui.workspace = roots.clone();
            self.copilot_ui.error = None;
            self.copilot_ui.program = prefs.copilot_program.clone();
            let repaint = ctx.clone();
            match Client::start_with_wake(
                program,
                roots.clone(),
                Arc::new(move || repaint.request_repaint()),
            ) {
                Ok(c) => self.copilot_ui.client = Some(c),
                Err(e) => self.copilot_ui.error = Some(e),
            }
        }
        if let Some(client) = &mut self.copilot_ui.client {
            if client.poll() {
                ctx.request_repaint();
            }
            if let Some((id, url)) = client
                .open_url
                .as_ref()
                .filter(|(_, url)| url.starts_with("https://github.com/"))
                .cloned()
            {
                ctx.open_url(egui::OpenUrl::new_tab(url));
                client.reply(id, serde_json::json!({"success":true}));
                client.open_url = None;
            }
            if client.ready
                && !client.authentication_checked
                && let Err(e) = client.sign_in()
            {
                self.copilot_ui.error = Some(e);
            }
            if self.copilot_ui.workspace != roots && client.folders(roots.clone()).is_ok() {
                self.copilot_ui.workspace = roots;
            }
        }
    }
    #[track_caller]
    fn copilot_cancel(&mut self) {
        if std::env::var_os("CODISSEU_COPILOT_TRACE").is_some()
            && self
                .copilot_ui
                .client
                .as_ref()
                .is_some_and(|c| c.busy || c.suggestion.is_some())
        {
            eprintln!(
                "COPILOT_QA {}",
                serde_json::json!({"event":"ui_cancel","line":std::panic::Location::caller().line()})
            );
        }
        if let Some(c) = &mut self.copilot_ui.client
            && (c.busy || c.suggestion.is_some() || self.copilot_ui.requested.is_some())
        {
            c.cancel();
        }
        self.copilot_ui.requested = None;
        self.copilot_ui.manual_waiting = None;
        self.copilot_ui.continuation = None;
        self.copilot_ui.accepted_utf16 = 0;
        self.copilot_ui.shown.set(None);
    }
    pub(super) fn copilot_tick(&mut self, ctx: &egui::Context) {
        self.copilot_trace_state(ctx);
        if self.copilot_ui.client.is_none() {
            return;
        }
        if self.copilot_blocked(ctx) {
            // The action palette can accept the already displayed proposal.
            // Keep its exact fingerprint, but neither render nor request code
            // while the palette owns focus. Other modals still discard it.
            let returning = self.focus_editor
                && self
                    .copilot_ui
                    .palette_return
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(1));
            if (self.project_search.open || returning) && self.copilot_can_accept_next_line() {
                self.copilot_ui.due = None;
                self.copilot_ui.was_blocked = true;
                return;
            }
            let waiting = self.copilot_ui.manual_waiting.clone();
            self.copilot_ui.palette_return = None;
            self.copilot_cancel();
            self.copilot_ui.manual_waiting = waiting;
            self.copilot_ui.due = None;
            self.copilot_ui.was_blocked = true;
            return;
        }
        let Some(owner) = self.copilot_owner() else {
            self.copilot_cancel();
            self.copilot_ui.observed = None;
            self.copilot_ui.due = None;
            return;
        };
        self.copilot_ui.palette_return = None;
        let now = ctx.input(|i| i.time);
        if self.copilot_ui.observed.as_ref() != Some(&owner) {
            let continuation = self.copilot_ui.continuation.take();
            let waiting = self
                .copilot_ui
                .manual_waiting
                .clone()
                .filter(|v| v == &owner);
            if continuation.is_none() {
                self.copilot_cancel();
            }
            self.copilot_ui.manual_waiting = waiting;
            // The editor may publish a new revision after the input frame (LSP
            // acceptance, Undo, formatting), or regain a caret after replacing
            // a selection. Debounce the validated context, not the transient
            // key event. Navigation/restoration also invalidate the old request.
            self.copilot_ui.due = Some(now + 0.35);
            self.copilot_ui.observed = Some(owner.clone());
            if let Some(previous) = continuation {
                if self.copilot_continue(previous, &owner) {
                    self.copilot_ui.due = None;
                    ctx.request_repaint();
                } else {
                    self.copilot_cancel();
                }
            }
        } else {
            // No effective edit/navigation: preserve the request and discard
            // only the temporary pre-input snapshot, including at document edges.
            self.copilot_ui.continuation = None;
        }
        let ready = self
            .copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.ready && c.authenticated);
        if ready
            && self.copilot_ui.synced != Some((owner.document, owner.version))
            && let Some(doc) = self.copilot_document(&owner)
        {
            match self.copilot_ui.client.as_ref().unwrap().document(doc) {
                Ok(()) => self.copilot_ui.synced = Some((owner.document, owner.version)),
                Err(e) => self.copilot_ui.error = Some(e),
            }
        }
        // The semantic widget contributes context; it must not suppress Copilot
        // requests for as long as a provider keeps its suggestion list open.
        let selected = self.copilot_semantic_choice();
        let selected_changed = selected != self.copilot_ui.selected;
        self.copilot_ui.selected = selected;
        let retained = self.copilot_ui.requested.as_ref() == Some(&owner)
            && self
                .copilot_ui
                .client
                .as_ref()
                .is_some_and(|c| c.suggestion.is_some());
        if ready
            && (((!self.copilot_ui.previous_ready || self.copilot_ui.was_blocked) && !retained)
                || selected_changed)
        {
            self.copilot_ui.due = Some(now + 0.35);
        }
        self.copilot_ui.previous_ready = ready;
        self.copilot_ui.was_blocked = false;
        if ready && self.copilot_ui.manual_waiting.as_ref() == Some(&owner) {
            self.copilot_ui.manual_waiting = None;
            self.copilot_request(true);
            return;
        }
        if ready && self.copilot_ui.due.is_some_and(|due| now >= due) {
            self.copilot_request(false);
            self.copilot_ui.due = None;
        }
        // Authentication/server events wake the UI. A pending debounce must not
        // keep repainting an idle editor while the account is disconnected.
        if ready && let Some(due) = self.copilot_ui.due {
            ctx.request_repaint_after(Duration::from_secs_f64((due - now).max(0.016)));
        }
    }
    fn copilot_request(&mut self, manual: bool) {
        let Some(owner) = self.copilot_owner() else {
            return;
        };
        let Some(doc) = self.copilot_document(&owner) else {
            return;
        };
        let selected = self.copilot_semantic_choice().and_then(|choice| {
            Some(serde_json::json!({"range":{"start":api::position(&doc.text,choice.range.start)?,"end":api::position(&doc.text,choice.range.end)?},"text":choice.text}))
        });
        if manual {
            self.copilot_ui.due = None;
        }
        if self.copilot_ui.client.is_none() {
            if manual {
                self.copilot_ui.settings = true;
            }
            return;
        }
        if let Some(c) = &mut self.copilot_ui.client {
            if !c.ready || !c.authenticated {
                if manual {
                    if c.authentication_checked && c.login.is_some() {
                        self.copilot_ui.manual_waiting = Some(owner);
                        self.copilot_ui.settings = true;
                    } else {
                        self.copilot_ui.manual_waiting = Some(owner);
                    }
                }
                return;
            }
            self.copilot_ui.manual_waiting = None;
            match c.complete_with_context(doc, manual, selected) {
                Ok(_) => {
                    self.copilot_ui.error = None;
                    self.copilot_ui.requested = Some(owner);
                    self.copilot_ui.shown.set(None);
                }
                Err(e) => self.copilot_ui.error = Some(e),
            }
        }
    }
    fn copilot_valid(&self, ctx: &egui::Context) -> Option<&Suggestion> {
        if self.copilot_blocked(ctx) {
            return None;
        }
        let owner = self.copilot_owner()?;
        if self.copilot_ui.requested.as_ref() != Some(&owner) {
            return None;
        }
        self.copilot_ui
            .client
            .as_ref()?
            .suggestion
            .as_ref()
            .filter(|s| s.document.version == owner.version && self.copilot_semantic_compatible(s))
    }
    fn copilot_semantic_choice(&self) -> Option<SemanticChoice> {
        self.copilot_completion_choice()
            .or_else(|| self.copilot_sql_choice())
    }
    fn copilot_semantic_compatible(&self, suggestion: &Suggestion) -> bool {
        if !self.copilot_semantic_owns_input() {
            return true;
        }
        let Some(choice) = self.copilot_semantic_choice() else {
            return false;
        };
        let Some(owner) = self.copilot_ui.requested.as_ref() else {
            return false;
        };
        let Some(doc) = self.service.active_document() else {
            return false;
        };
        if choice.range.start > owner.caret || choice.range.end < owner.caret {
            return false;
        }
        let prefix: String = doc
            .text()
            .chars()
            .skip(choice.range.start)
            .take(owner.caret - choice.range.start)
            .collect();
        let Some(projection) = suggestion.projection() else {
            return false;
        };
        let from = owner.caret + projection.consumed_source;
        let existing: String = doc
            .text()
            .chars()
            .skip(from)
            .take(choice.range.end.saturating_sub(from))
            .collect();
        format!("{prefix}{}{existing}", projection.text).starts_with(&choice.text)
    }
    pub(super) fn copilot_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.copilot_blocked(ctx) || !self.editor_focused(ctx) {
            return false;
        }
        if consume_exact(ctx, Modifiers::ALT, Key::Backslash) {
            ctx.input_mut(|i| {
                i.events
                    .retain(|e| !matches!(e,egui::Event::Text(text) if text=="\\"))
            });
            if !self.service.editor_preferences().copilot_enabled {
                self.copilot_ui.settings = true;
                self.copilot_ui.program = self.service.editor_preferences().copilot_program.clone();
            } else {
                self.copilot_request(true);
            }
            return true;
        }
        if self.copilot_inline_interaction(ctx) {
            return true;
        }
        let editing = ctx.input(|i|i.raw.events.iter().any(|e|text_change_event(e) || matches!(e,egui::Event::Key{key,pressed:true,..} if matches!(key,Key::Backspace|Key::Delete|Key::Enter|Key::ArrowLeft|Key::ArrowRight|Key::ArrowUp|Key::ArrowDown|Key::Home|Key::End|Key::PageUp|Key::PageDown))));
        if editing {
            if std::env::var_os("CODISSEU_COPILOT_TRACE").is_some() {
                let events = ctx.input(|i| {
                    i.raw
                        .events
                        .iter()
                        .filter_map(|e| match e {
                            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => {
                                Some(if text.is_empty() {
                                    "ime_preedit_empty"
                                } else {
                                    "ime_preedit"
                                })
                            }
                            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                                Some(if text.is_empty() {
                                    "ime_commit_empty"
                                } else {
                                    "ime_commit"
                                })
                            }
                            egui::Event::Ime(_) => Some("ime_lifecycle"),
                            egui::Event::Text(s) => {
                                Some(if s.is_empty() { "empty_text" } else { "text" })
                            }
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                eprintln!(
                    "COPILOT_QA {}",
                    serde_json::json!({"event":"editing_input","types":events})
                );
            }
            self.copilot_hide_for_input();
            return false;
        }
        if self.copilot_semantic_owns_input() {
            return false;
        }
        if (self.completion_popup.is_some() || self.sql_completion.is_open())
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            return false;
        }
        let waiting = self.copilot_ui.due.is_some()
            || self.copilot_ui.manual_waiting.is_some()
            || self.copilot_ui.client.as_ref().is_some_and(|c| c.busy);
        if (waiting || self.copilot_valid(ctx).is_some())
            && consume_exact(ctx, Modifiers::NONE, Key::Escape)
        {
            self.copilot_cancel();
            self.copilot_ui.due = None;
            return true;
        }
        if self.copilot_valid(ctx).is_some() {
            if self.copilot_tab_owns_indentation() {
                if consume_exact(ctx, Modifiers::NONE, Key::Tab) {
                    if self.completion_popup.take().is_some() {
                        self.send(Command::CancelCompletion);
                    }
                    self.sql_completion.dismiss_empty_for_copilot();
                    self.copilot_accept();
                    return true;
                }
            } else if ctx.input(|i| i.key_pressed(Key::Tab)) {
                self.copilot_hide_for_input();
            }
        }
        false
    }
    fn copilot_semantic_owns_input(&self) -> bool {
        (self.completion_popup.is_some() && self.completion_has_choices())
            || self.sql_completion.copilot_has_choices()
    }
    pub(super) fn copilot_can_accept_next_line(&self) -> bool {
        self.service.editor_preferences().copilot_enabled
            && self.copilot_owner().is_some_and(|owner| {
                self.copilot_ui.requested.as_ref() == Some(&owner)
                    && self.copilot_ui.client.as_ref().is_some_and(|client| {
                        client.ready
                            && client.authenticated
                            && client.suggestion.as_ref().is_some_and(|s| {
                                s.document.version == owner.version
                                    && self.copilot_semantic_compatible(s)
                            })
                    })
            })
    }
    pub(super) fn copilot_accept_next_line_command(&mut self) -> bool {
        if !self.copilot_can_accept_next_line() {
            return false;
        }
        self.copilot_accept_part(true);
        self.focus_editor = true;
        self.copilot_ui.palette_return = Some(std::time::Instant::now());
        true
    }
    fn copilot_accept(&mut self) {
        let Some(owner) = self.copilot_owner() else {
            return;
        };
        if self.copilot_ui.requested.as_ref() != Some(&owner) {
            return;
        }
        let Some(s) = self
            .copilot_ui
            .client
            .as_ref()
            .and_then(|c| c.suggestion.clone())
        else {
            return;
        };
        let before = Selection::caret(owner.caret);
        let after = Selection::caret(s.start + s.insert.chars().count());
        let result = self.service.dispatch(Command::EditTransaction {
            id: owner.document,
            expected_version: owner.version,
            delta: TextDelta {
                range: s.start..s.end,
                insert: s.insert,
            },
            kind: EditKind::Atomic,
            before,
            after,
        });
        match result {
            Ok(()) => {
                if let Some(c) = &self.copilot_ui.client {
                    c.accepted(s.item)
                }
            }
            Err(e) => {
                self.service.last_error = Some(e.clone());
                self.copilot_ui.error = Some(e);
            }
        }
        self.copilot_cancel();
        self.copilot_ui.due = None;
        self.focus_editor = true;
    }
    /// Called after this frame's editor input, while EditorBridge is temporarily
    /// borrowed by the native editor. Validate domain ownership, not old metrics.
    pub(super) fn copilot_editor_suggestion(
        &self,
        ctx: &egui::Context,
        id: DocumentId,
        view: codisseu_core::ViewId,
        version: u64,
        caret: usize,
    ) -> Option<Suggestion> {
        if self.copilot_blocked(ctx) || !self.service.editor_preferences().copilot_enabled {
            return None;
        }
        let owner = self.copilot_ui.requested.as_ref().filter(|o| {
            o.document == id
                && o.view == view
                && o.version == version
                && o.caret == caret
                && self.service.active_view() == Some(view)
        })?;
        self.service.active_document().filter(|d| {
            d.id == id && d.path == owner.path && d.version == version && !d.is_read_only()
        })?;
        self.service
            .view(view)
            .filter(|v| v.selection() == Selection::caret(caret))?;
        let client = self
            .copilot_ui
            .client
            .as_ref()
            .filter(|c| c.ready && c.authenticated)?;
        let suggestion = client
            .suggestion
            .as_ref()
            .filter(|s| s.document.version == version)?;
        self.copilot_semantic_compatible(suggestion)
            .then(|| suggestion.clone())
    }
    pub(super) fn copilot_did_show(&self, suggestion: &Suggestion) {
        let key = (
            suggestion.ticket,
            self.copilot_ui
                .client
                .as_ref()
                .map_or(0, |c| c.suggestion_index),
        );
        if self.copilot_ui.shown.get() != Some(key) {
            if let Some(client) = &self.copilot_ui.client {
                client.shown(suggestion.item.clone());
            }
            self.copilot_ui.shown.set(Some(key));
        }
    }
    pub(super) fn copilot_status(&mut self, ui: &mut egui::Ui) {
        let label = "Copilot";
        let activity = self.copilot_activity();
        let enabled = self.service.editor_preferences().copilot_enabled;
        let status = self
            .copilot_ui
            .error
            .as_deref()
            .or_else(|| {
                self.copilot_ui
                    .client
                    .as_ref()
                    .and_then(|c| c.error.as_deref())
            })
            .unwrap_or_else(|| {
                self.copilot_ui.client.as_ref().map_or(
                    if enabled {
                        "Preparando Copilot"
                    } else {
                        "Sugestões de código desativadas"
                    },
                    |c| c.status.as_str(),
                )
            });
        if ui
            .small_button(label)
            .on_hover_text(format!(
                "{activity}\n{status}\nConfigurar sugestões de código"
            ))
            .clicked()
        {
            self.copilot_ui.settings = true;
            self.copilot_ui.program = self.service.editor_preferences().copilot_program.clone();
        }
    }
    pub(super) fn copilot_activity(&self) -> String {
        if !self.service.editor_preferences().copilot_enabled {
            return "Sugestões desativadas".into();
        }
        let Some(c) = &self.copilot_ui.client else {
            return self
                .copilot_ui
                .error
                .clone()
                .unwrap_or_else(|| "Preparando o servidor Copilot…".into());
        };
        if let Some(error) = self.copilot_ui.error.as_ref().or(c.error.as_ref()) {
            return error.clone();
        }
        if !c.ready {
            return "O servidor Copilot não está disponível. Verifique a conexão.".into();
        }
        if c.kind == "Inactive" {
            return format!("O Copilot está inativo neste arquivo. {}", c.status);
        }
        if !c.messages.is_empty() {
            return "O Copilot enviou um aviso. Confira as opções abaixo.".into();
        }
        if !c.authenticated {
            return "Aguardando confirmação de acesso à conta Copilot.".into();
        }
        if c.kind == "Warning" && !c.status.is_empty() {
            return c.status.clone();
        }
        if c.busy {
            return "Buscando sugestão de código…".into();
        }
        if let Some(suggestion) = &c.suggestion {
            return if self.copilot_semantic_compatible(suggestion) {
                "Sugestão recebida. Ela aparece no editor quando ele está em foco.".into()
            } else {
                "A proposta recebida não corresponde ao item selecionado na lista de autocomplete."
                    .into()
            };
        }
        if self.copilot_ui.requested.is_some() {
            return "O Copilot não retornou uma sugestão exibível para esta posição.".into();
        }
        if let Some(d) = self.service.active_document() {
            if d.len_bytes() > api::MAX_DOCUMENT {
                return "Este arquivo excede o limite de contexto do Copilot (1 MiB).".into();
            }
            if d.is_read_only() {
                return "Abra um arquivo editável para receber sugestões.".into();
            }
        } else {
            return "Abra um arquivo para receber sugestões.".into();
        }
        "Pronto para solicitar sugestões enquanto você edita.".into()
    }
}
#[path = "copilot_interactions.rs"]
mod interactions;
#[path = "copilot_preferences.rs"]
mod preferences;
#[path = "copilot_words.rs"]
mod words;

#[cfg(test)]
#[path = "copilot_flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "copilot_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "copilot_login_tests.rs"]
mod login_tests;
#[cfg(test)]
#[path = "editor/copilot_projection_tests.rs"]
mod projection_tests;
#[cfg(test)]
#[path = "copilot_tests.rs"]
pub(crate) mod tests;
