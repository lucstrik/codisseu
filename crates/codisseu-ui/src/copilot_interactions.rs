//! Inline interaction shares the guarded editor transaction, including partial acceptance.
use super::*;

pub(super) struct Continuation {
    owner: Owner,
    suggestion: Suggestion,
}

impl CodisseuApp {
    fn copilot_continuation(&self) -> Option<Continuation> {
        Some(Continuation {
            owner: self.copilot_ui.requested.clone()?,
            suggestion: self.copilot_ui.client.as_ref()?.suggestion.clone()?,
        })
    }
    /// Capture a received proposal before input. The renderer and tick validate
    /// the resulting document/caret, so a no-op End/Delete cannot cancel a valid
    /// request or hide its result indefinitely. Real edits still require the
    /// exact source-preserving continuation check below.
    pub(super) fn copilot_hide_for_input(&mut self) {
        if let Some(previous) = self.copilot_continuation() {
            self.copilot_ui.continuation = Some(previous);
        }
    }
    pub(super) fn copilot_continue(&mut self, previous: Continuation, owner: &Owner) -> bool {
        let old = previous.owner;
        if old.document != owner.document
            || old.view != owner.view
            || old.path != owner.path
            || owner.version <= old.version
            || owner.caret <= old.caret
        {
            return false;
        }
        let Some(doc) = self.copilot_document(owner) else {
            return false;
        };
        let prior = &previous.suggestion;
        let final_result = prior
            .document
            .text
            .chars()
            .take(prior.start)
            .chain(prior.insert.chars())
            .chain(prior.document.text.chars().skip(prior.end));
        if final_result.eq(doc.text.chars()) {
            if let Some(client) = &self.copilot_ui.client {
                client.accepted(prior.item.clone());
            }
            self.copilot_cancel();
            return true;
        }
        let Some(suggestion) = prior.continued(doc) else {
            return false;
        };
        let accepted = suggestion
            .document
            .text
            .chars()
            .skip(suggestion.start)
            .take(owner.caret - suggestion.start)
            .map(|c| c.len_utf16() as u32)
            .sum();
        let Some(client) = &mut self.copilot_ui.client else {
            return false;
        };
        client.partially_accepted(suggestion.item.clone(), accepted);
        if !client.continue_suggestion(suggestion) {
            return false;
        }
        self.copilot_ui.requested = Some(owner.clone());
        self.copilot_ui.observed = Some(owner.clone());
        self.copilot_ui.accepted_utf16 = accepted;
        self.copilot_ui.shown.set(None);
        true
    }
    pub(super) fn copilot_tab_owns_indentation(&self) -> bool {
        let Some(suggestion) = self
            .copilot_ui
            .client
            .as_ref()
            .and_then(|c| c.suggestion.as_ref())
        else {
            return false;
        };
        let Some(owner) = self.copilot_ui.requested.as_ref() else {
            return false;
        };
        let Some(doc) = self.service.active_document() else {
            return false;
        };
        let snapshot = doc.snapshot();
        let start = snapshot.line_to_char(snapshot.char_to_line(owner.caret));
        let prefix = snapshot.slice(start..owner.caret);
        if !prefix.chars().all(|ch| matches!(ch, ' ' | '\t')) {
            return true;
        }
        let column = prefix.chars().fold(
            0usize,
            |n, ch| if ch == '\t' { n + 4 - n % 4 } else { n + 1 },
        );
        let indent = suggestion
            .suffix
            .chars()
            .take_while(|ch| matches!(ch, ' ' | '\t'))
            .fold(
                column,
                |n, ch| if ch == '\t' { n + 4 - n % 4 } else { n + 1 },
            )
            - column;
        indent < 4
    }
    pub(super) fn copilot_inline_interaction(&mut self, ctx: &egui::Context) -> bool {
        if self.copilot_valid(ctx).is_none() {
            return false;
        }
        let cycle = if consume_exact(ctx, Modifiers::ALT, Key::CloseBracket) {
            1
        } else if consume_exact(ctx, Modifiers::ALT, Key::OpenBracket) {
            -1
        } else {
            0
        };
        if cycle != 0 {
            ctx.input_mut(|i|i.events.retain(|e|!matches!(e,egui::Event::Text(text) if (cycle>0 && text=="]") || (cycle<0 && text=="["))));
            if let Some(client) = &mut self.copilot_ui.client {
                client.cycle(cycle);
            }
            self.copilot_ui.accepted_utf16 = 0;
            self.copilot_ui.shown.set(None);
            ctx.request_repaint();
            return true;
        }
        // A native text event takes precedence over an old suggestion in the same frame.
        if ctx.input(|i| i.raw.events.iter().any(text_change_event)) {
            return false;
        }
        if self.completion_popup.is_none()
            && !self.sql_completion.is_open()
            && !ctx.options(|o| o.screen_reader)
            && consume_exact(
                ctx,
                if cfg!(target_os = "macos") {
                    Modifiers::COMMAND
                } else {
                    Modifiers::CTRL
                },
                Key::ArrowRight,
            )
        {
            self.copilot_accept_part(false);
            return true;
        }
        false
    }
    pub(super) fn copilot_accept_part(&mut self, line: bool) {
        let Some(previous) = self.copilot_continuation() else {
            return;
        };
        let Some(projection) = previous.suggestion.projection() else {
            return;
        };
        let text = &projection.text;
        let leading = projection.first_insertion();
        let leading_byte = text
            .char_indices()
            .nth(leading)
            .map_or(text.len(), |(b, _)| b);
        let new_text = &text[leading_byte..];
        let take = leading
            + if line {
                new_text
                    .find('\n')
                    .map_or(new_text.chars().count(), |b| new_text[..=b].chars().count())
            } else {
                words::next_word(new_text, api::language(&previous.owner.path))
            };
        if take >= text.chars().count() {
            self.copilot_accept();
            return;
        }
        let insert = text.chars().take(take).collect::<String>();
        let before = Selection::caret(previous.owner.caret);
        let after = Selection::caret(previous.owner.caret + take);
        match self.service.dispatch(Command::EditTransaction {
            id: previous.owner.document,
            expected_version: previous.owner.version,
            delta: TextDelta {
                range: previous.owner.caret..previous.owner.caret + projection.source_prefix(take),
                insert,
            },
            kind: EditKind::Atomic,
            before,
            after,
        }) {
            Ok(()) => {
                if let Some(owner) = self.copilot_owner() {
                    self.copilot_continue(previous, &owner);
                }
                self.copilot_ui.due = None;
                self.focus_editor = true;
            }
            Err(e) => self.service.last_error = Some(e),
        }
    }
}
