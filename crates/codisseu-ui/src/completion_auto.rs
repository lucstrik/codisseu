//! Debounced typing completion. Scheduling does not read the filesystem, alter
//! text or synthesize semantic items; requests use the existing versioned path.
use super::*;
use codisseu_core::{Language, ProviderState, TextSnapshot, ViewId};
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Fingerprint {
    document: DocumentId,
    view: ViewId,
    path: PathBuf,
    version: u64,
    caret: usize,
}
struct Pending {
    fingerprint: Fingerprint,
    due: f64,
}
#[derive(Default)]
pub(super) struct Automatic {
    pending: Option<Pending>,
    requested: Option<Fingerprint>,
    observed: Option<Fingerprint>,
}
impl Automatic {
    fn cancel(&mut self) {
        self.pending = None;
    }
    fn observe(&mut self, fingerprint: Fingerprint, typing: bool, now: f64) {
        if self.observed.as_ref() != Some(&fingerprint) {
            self.cancel();
        }
        if typing && self.requested.as_ref() != Some(&fingerprint) {
            self.pending = Some(Pending {
                fingerprint: fingerprint.clone(),
                due: now + 0.25,
            });
        }
        self.observed = Some(fingerprint);
    }
    fn ready(&mut self, fingerprint: &Fingerprint, now: f64, blocked: bool) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| &p.fingerprint != fingerprint)
        {
            self.cancel();
        }
        if !blocked && self.pending.as_ref().is_some_and(|p| p.due <= now) {
            self.pending = None;
            self.requested = Some(fingerprint.clone());
            true
        } else {
            false
        }
    }
}

/// Event::Text distinguishes committed typing from Paste/Cut/IME and document
/// actions. A shortcut or navigation in the same frame invalidates the batch.
fn typed_chars(events: &[egui::Event], modifiers: Modifiers) -> usize {
    if modifiers.ctrl || modifiers.command || modifiers.alt {
        return 0;
    }
    let mut count = 0;
    for event in events {
        if matches!(event, egui::Event::Ime(_)) {
            if crate::copilot::text_change_event(event) {
                return 0;
            }
            continue;
        }
        match event {
            egui::Event::Text(text) => {
                if text.len() > 128 || text.chars().any(char::is_control) {
                    return 0;
                }
                count += text.chars().count();
            }
            egui::Event::Paste(_)
            | egui::Event::MouseWheel { .. }
            | egui::Event::MiddleClickPaste { .. }
            | egui::Event::Cut
            | egui::Event::PointerButton { pressed: true, .. } => return 0,
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if modifiers.ctrl
                || modifiers.command
                || modifiers.alt
                || matches!(
                    key,
                    Key::Escape
                        | Key::Tab
                        | Key::Enter
                        | Key::Backspace
                        | Key::Delete
                        | Key::ArrowUp
                        | Key::ArrowDown
                        | Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::Home
                        | Key::End
                        | Key::PageUp
                        | Key::PageDown
                ) =>
            {
                return 0;
            }
            _ => {}
        }
    }
    count.min(128)
}
fn interrupts(events: &[egui::Event], modifiers: Modifiers) -> bool {
    modifiers.ctrl
        || modifiers.command
        || modifiers.alt
        || events.iter().any(|e| {
            if matches!(e, egui::Event::Ime(_)) {
                return crate::copilot::text_change_event(e);
            }
            if matches!(e, egui::Event::Key { pressed:true, modifiers, .. }
                if modifiers.ctrl || modifiers.command || modifiers.alt)
            {
                return true;
            }
            matches!(
                e,
                egui::Event::Paste(_)
                    | egui::Event::MouseWheel { .. }
                    | egui::Event::MiddleClickPaste { .. }
                    | egui::Event::Cut
                    | egui::Event::PointerButton { pressed: true, .. }
                    | egui::Event::Key {
                        pressed: true,
                        key: Key::Escape
                            | Key::Tab
                            | Key::Enter
                            | Key::Backspace
                            | Key::Delete
                            | Key::ArrowUp
                            | Key::ArrowDown
                            | Key::ArrowLeft
                            | Key::ArrowRight
                            | Key::Home
                            | Key::End
                            | Key::PageUp
                            | Key::PageDown,
                        ..
                    }
            )
        })
}
fn useful_suffix(snapshot: &TextSnapshot, caret: usize, language: Language) -> bool {
    if caret == 0 || caret > snapshot.len_chars() {
        return false;
    }
    let last = snapshot.char(caret - 1);
    last.is_alphanumeric()
        || matches!(last, '_' | '$')
        || (matches!(language, Language::TypeScript | Language::Rust) && last == '.')
        || (language == Language::Rust
            && caret >= 2
            && snapshot.char(caret - 2) == ':'
            && last == ':')
        || (language == Language::Php && matches!(last, '\'' | '"' | '@'))
        || (language == Language::Php
            && matches!(last, '.' | '/')
            && quoted_prefix(snapshot, caret))
        || (language == Language::Php
            && caret >= 2
            && matches!((snapshot.char(caret - 2), last), ('-', '>') | (':', ':')))
}

fn quoted_prefix(snapshot: &TextSnapshot, caret: usize) -> bool {
    let line = snapshot.char_to_line(caret);
    let start = snapshot.line_to_char(line);
    if caret - start > 4096 {
        return false;
    }
    let mut quote = None;
    let mut escaped = false;
    for ch in snapshot.slice(start..caret).chars() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        }
    }
    quote.is_some()
}

impl CodisseuApp {
    pub(super) fn automatic_completion(&mut self, ctx: &egui::Context) {
        // Read original input before checking focus/readiness: shortcuts may have
        // consumed processed events or moved focus earlier in this frame.
        let (typed, interrupt, now) = ctx.input(|i| {
            (
                typed_chars(&i.raw.events, i.modifiers),
                interrupts(&i.raw.events, i.modifiers),
                i.time,
            )
        });
        if interrupt {
            self.completion_auto.cancel();
            self.completion_auto.requested = None;
            self.completion_auto.observed = None;
            return;
        }
        if self.completion_auto.requested.is_some()
            && self.completion_popup.is_some()
            && !self.service.completion.is_pending()
            && !self.completion_has_choices()
        {
            self.completion_popup = None;
        }
        let blocked = !self.editor_focused(ctx)
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.exit_when_saved
            || self.project_search.open
            || self.snippet_ui.is_open()
            || self.debugger_ui.dialog
            || self.git_ui.dialog.is_some()
            || self.git_ui.diff_open
            || self.database_ui.dialog
            || self.database_ui.edit.is_some()
            || self.database_ui.preview.is_some()
            || self.transfer_modal_open()
            || self.project_files_ui.trash.is_some()
            || self.service.provider_startup().needs_trust;
        if blocked {
            self.completion_auto.cancel();
            self.completion_auto.observed = None;
            return;
        }
        let Some(view) = self.service.active_view() else {
            self.completion_auto.cancel();
            return;
        };
        let Some(doc) = self.service.active_document() else {
            self.completion_auto.cancel();
            return;
        };
        let Some(language) = Language::for_path(&doc.path) else {
            self.completion_auto.cancel();
            return;
        };
        let Some(selection) = self.service.view(view).map(|v| v.selection()) else {
            self.completion_auto.cancel();
            return;
        };
        let metrics = &self.editor_bridge.metrics;
        if !doc.language_supported()
            || !selection.is_caret()
            || metrics.document != Some(doc.id)
            || metrics.composing
            || metrics.selection_count != 1
            || !self
                .service
                .language
                .statuses
                .iter()
                .any(|p| p.language == language && p.state == ProviderState::Ready)
        {
            if self.completion_auto.requested.is_some() && self.completion_popup.take().is_some() {
                self.send(Command::CancelCompletion);
            }
            self.completion_auto.cancel();
            return;
        }
        let fingerprint = Fingerprint {
            document: doc.id,
            view,
            path: doc.path.clone(),
            version: doc.version,
            caret: selection.head,
        };
        if self.caret != Some((doc.id, selection.head)) {
            self.completion_auto.cancel();
            return;
        }
        let actual_change = doc.last_change().is_some_and(|change| {
            change.new_chars.end == selection.head
                && !change.new_chars.is_empty()
                && change.new_chars.len() <= typed
        });
        let new_revision = self
            .completion_auto
            .observed
            .as_ref()
            .is_none_or(|previous| {
                previous.document != fingerprint.document || previous.version != fingerprint.version
            });
        let typing = typed > 0
            && actual_change
            && new_revision
            && useful_suffix(&doc.snapshot(), selection.head, language);
        self.completion_auto
            .observe(fingerprint.clone(), typing, now);
        if self.completion_auto.ready(
            &fingerprint,
            now,
            self.service.completion.is_pending()
                || self.service.completion.is_applying()
                || self.completion_popup.is_some(),
        ) {
            self.request_completion();
            ctx.request_repaint();
        } else if let Some(pending) = &self.completion_auto.pending {
            ctx.request_repaint_after(Duration::from_secs_f64((pending.due - now).max(0.016)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fingerprint(version: u64, caret: usize) -> Fingerprint {
        Fingerprint {
            document: 1,
            view: 1,
            path: "/fixture/App.php".into(),
            version,
            caret,
        }
    }
    #[test]
    fn typing_waits_for_quiet_period_and_dispatches_once_per_fingerprint() {
        let mut state = Automatic::default();
        let a = fingerprint(1, 4);
        state.observe(a.clone(), true, 1.);
        assert!(!state.ready(&a, 1.24, false));
        let b = fingerprint(2, 5);
        state.observe(b.clone(), true, 1.2);
        assert!(!state.ready(&b, 1.44, false));
        assert!(state.ready(&b, 1.46, false));
        assert!(!state.ready(&b, 2., false));
        state.observe(b.clone(), false, 3.);
        assert!(!state.ready(&b, 4., false));
    }
    #[test]
    fn changing_caret_view_or_revision_cancels_scheduled_query() {
        for changed in [
            fingerprint(2, 5),
            fingerprint(1, 6),
            Fingerprint {
                view: 2,
                ..fingerprint(1, 5)
            },
        ] {
            let mut state = Automatic::default();
            state.observe(fingerprint(1, 5), true, 0.);
            state.observe(changed.clone(), false, 0.3);
            assert!(!state.ready(&changed, 1., false));
        }
        let mut state = Automatic::default();
        state.observe(fingerprint(1, 5), true, 0.);
        state.cancel();
        assert!(!state.ready(&fingerprint(1, 5), 1., false));
    }
    #[test]
    fn pending_request_blocks_dispatch_without_restarting_debounce() {
        let mut state = Automatic::default();
        let a = fingerprint(1, 5);
        state.observe(a.clone(), true, 0.);
        assert!(!state.ready(&a, 1., true));
        assert!(state.ready(&a, 1.1, false));
    }
    #[test]
    fn clipboard_shortcuts_templates_navigation_and_ime_never_schedule_typing() {
        let text = egui::Event::Text("forecast".into());
        assert_eq!(typed_chars(std::slice::from_ref(&text), Modifiers::NONE), 8);
        for event in [
            egui::Event::Paste("forecast".into()),
            egui::Event::Ime(egui::ImeEvent::Commit("forecast".into())),
            egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
            egui::Event::Key {
                key: Key::ArrowLeft,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            },
        ] {
            assert_eq!(
                typed_chars(&[text.clone(), event.clone()], Modifiers::NONE),
                0
            );
            assert!(interrupts(&[event], Modifiers::NONE));
        }
        assert_eq!(typed_chars(&[text], Modifiers::CTRL), 0);
    }
    #[test]
    fn member_triggers_are_language_aware_and_unicode_safe() {
        for (text, language, expected) in [
            ("🌧a", Language::Php, true),
            ("$", Language::Php, true),
            ("User::", Language::Php, true),
            ("$x->", Language::Php, true),
            ("obj.", Language::TypeScript, true),
            ("$a.", Language::Php, false),
            ("view('admin.", Language::Php, true),
            ("asset('css/", Language::Php, true),
            ("view('", Language::Php, true),
            ("hello ", Language::Php, false),
        ] {
            let s = TextSnapshot::from_str(text);
            assert_eq!(
                useful_suffix(&s, s.len_chars(), language),
                expected,
                "{text}"
            );
        }
    }
}

#[cfg(test)]
mod ime_lifecycle_tests {
    use super::*;
    #[test]
    fn empty_ime_notifications_preserve_semantic_debounce_but_composition_interrupts() {
        for event in [
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: String::new(),
                active_range_chars: None,
            }),
            egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
        ] {
            assert!(!interrupts(std::slice::from_ref(&event), Modifiers::NONE));
            assert_eq!(
                typed_chars(&[egui::Event::Text("a".into()), event], Modifiers::NONE),
                1
            );
        }
        assert!(interrupts(
            &[egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "あ".into(),
                active_range_chars: None
            })],
            Modifiers::NONE
        ));
    }
}
