//! Widget-independent UTF-8 buffer and bounded undo history.
pub mod commands;
mod navigation;
pub mod search;
pub use navigation::{next_grapheme, next_word, previous_grapheme, previous_word, word_range};

/// Cheap copy-on-write snapshot; scalar/line conversions stay in the rope.
pub type TextSnapshot = ropey::Rope;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextChange {
    pub revision: u64,
    /// Half-open line spans; line numbers after the span may shift.
    pub old_lines: std::ops::Range<usize>,
    pub new_lines: std::ops::Range<usize>,
    /// Exact scalar spans, also for grouped Undo/Redo and passive view anchors.
    pub old_chars: std::ops::Range<usize>,
    pub new_chars: std::ops::Range<usize>,
}

use ropey::Rope;
use std::{
    collections::VecDeque,
    ops::Range,
    sync::OnceLock,
    time::{Duration, Instant},
};

/// Semantic provider/rename cap; kept separate from the editor limit.
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
pub const MAX_EDITOR_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
/// Canonical Ropey lines, including an empty final line after a line break.
pub const MAX_EDITOR_DOCUMENT_LINES: usize = 1_000_000;
const MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;
const MAX_HISTORY_ENTRIES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditKind {
    Atomic,
    Typing,
    Backspace,
    Delete,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextDelta {
    pub range: Range<usize>,
    pub insert: String,
}
impl TextDelta {
    /// Preserve insertion/deletion intent in repeated text using the resulting scalar caret.
    /// A prefix-first diff alone can move an inserted character into an identical suffix,
    /// incorrectly splitting contiguous typing into separate undo groups.
    pub fn at_caret(before: &str, after: &str, caret: usize) -> Self {
        let after_len = after.chars().count();
        if caret > after_len {
            return Self::between(before, after);
        }
        let suffix = before
            .chars()
            .rev()
            .zip(after.chars().rev())
            .take(after_len - caret)
            .take_while(|(a, b)| a == b)
            .count();
        let before_end = before.len()
            - before
                .chars()
                .rev()
                .take(suffix)
                .map(char::len_utf8)
                .sum::<usize>();
        let after_end = after.len()
            - after
                .chars()
                .rev()
                .take(suffix)
                .map(char::len_utf8)
                .sum::<usize>();
        let prefix = before[..before_end]
            .chars()
            .zip(after[..after_end].chars())
            .take_while(|(a, b)| a == b)
            .count();
        let after_start = after
            .char_indices()
            .nth(prefix)
            .map_or(after.len(), |(b, _)| b);
        Self {
            range: prefix..before.chars().count() - suffix,
            insert: after[after_start..after_end].into(),
        }
    }
    /// A minimal scalar interval, respecting UTF8 boundaries (including CRLF/combining text).
    pub fn between(before: &str, after: &str) -> Self {
        let prefix = before
            .chars()
            .zip(after.chars())
            .take_while(|(a, b)| a == b)
            .count();
        let a = before
            .char_indices()
            .nth(prefix)
            .map_or(before.len(), |(b, _)| b);
        let b = after
            .char_indices()
            .nth(prefix)
            .map_or(after.len(), |(b, _)| b);
        let suffix = before[a..]
            .chars()
            .rev()
            .zip(after[b..].chars().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let removed = before[a..].chars().count() - suffix;
        let end = after.len()
            - after[b..]
                .chars()
                .rev()
                .take(suffix)
                .map(char::len_utf8)
                .sum::<usize>();
        Self {
            range: prefix..prefix + removed,
            insert: after[b..end].into(),
        }
    }
}
/// Directed selection in Unicode scalar indices, independent of any GUI widget.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}
impl Selection {
    pub fn caret(index: usize) -> Self {
        Self {
            anchor: index,
            head: index,
        }
    }
    pub fn is_caret(self) -> bool {
        self.anchor == self.head
    }
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }
    pub fn validate(self, len: usize) -> Result<(), String> {
        if self.anchor > len || self.head > len {
            Err("Seleção fora do documento".into())
        } else {
            Ok(())
        }
    }
    /// Positions inside a replaced span land after its inserted text.
    pub fn mapped(self, delta: &TextDelta) -> Self {
        let count = delta.insert.chars().count();
        let map = |p: usize| {
            if p < delta.range.start {
                p
            } else if p >= delta.range.end {
                p - delta.range.len() + count
            } else {
                delta.range.start + count
            }
        };
        Self {
            anchor: map(self.anchor),
            head: map(self.head),
        }
    }
}
#[derive(Clone, Debug)]
struct HistoryEntry {
    before: Rope,
    after: Rope,
    /// Sum of changed UTF8 bytes, not the shared snapshot logical sizes.
    change_bytes: usize,
    before_identity: u64,
    after_identity: u64,
    before_selection: Selection,
    after_selection: Selection,
    old_chars: Range<usize>,
    new_chars: Range<usize>,
    batch_maps: Option<(SelectionMap, SelectionMap)>,
}
type SelectionMap = Vec<(Range<usize>, usize)>;
#[derive(Clone, Debug)]
struct EditGroup {
    kind: EditKind,
    caret: usize,
    at: Instant,
}
#[derive(Debug)]
pub struct TextBuffer {
    rope: Rope,
    text: OnceLock<String>,
    revision: u64,
    identity: u64,
    next_identity: u64,
    last_change: Option<TextChange>,
    last_selection_map: Option<SelectionMap>,
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    group: Option<EditGroup>,
    selection: Selection,
    selection_revision: u64,
}

// Staging refactorings shares rope/history snapshots. The optional legacy text
// cache is rebuilt only if requested; cloning it would copy every open document.
impl Clone for TextBuffer {
    fn clone(&self) -> Self {
        Self {
            rope: self.rope.clone(),
            text: OnceLock::new(),
            revision: self.revision,
            identity: self.identity,
            next_identity: self.next_identity,
            last_change: self.last_change.clone(),
            last_selection_map: self.last_selection_map.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            group: self.group.clone(),
            selection: self.selection,
            selection_revision: self.selection_revision,
        }
    }
}

impl TextBuffer {
    pub fn new(text: String) -> Result<Self, String> {
        validate(&text)?;
        Ok(Self {
            rope: Rope::from_str(&text),
            text: OnceLock::from(text),
            revision: 0,
            identity: 1,
            next_identity: 2,
            last_change: None,
            last_selection_map: None,
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            group: None,
            selection: Selection::default(),
            selection_revision: 0,
        })
    }

    pub fn text(&self) -> &str {
        self.text.get_or_init(|| self.rope.to_string())
    }
    pub fn snapshot(&self) -> TextSnapshot {
        self.rope.clone()
    }
    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }
    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }
    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }
    pub fn line_to_char(&self, line: usize) -> Option<usize> {
        self.rope.try_line_to_char(line).ok()
    }
    pub fn char_to_line(&self, index: usize) -> Option<usize> {
        self.rope.try_char_to_line(index).ok()
    }
    pub fn line(&self, line: usize) -> Option<ropey::RopeSlice<'_>> {
        self.rope.get_line(line)
    }
    pub fn slice(&self, range: Range<usize>) -> Option<ropey::RopeSlice<'_>> {
        self.rope.get_slice(range)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Identity survives undo/redo, unlike the monotonic invalidation revision.
    pub fn content_identity(&self) -> u64 {
        self.identity
    }
    pub fn last_change(&self) -> Option<&TextChange> {
        self.last_change.as_ref()
    }
    /// Exact sparse anchor mapping for the latest batch edit or its Undo/Redo.
    pub fn last_selection_map(&self) -> Option<&[(Range<usize>, usize)]> {
        self.last_selection_map.as_deref()
    }
    /// Comparison never populates the legacy contiguous String cache.
    pub fn matches(&self, text: &str) -> bool {
        self.rope == text
    }
    fn changed(
        &mut self,
        old_lines: Range<usize>,
        new_lines: Range<usize>,
        old_chars: Range<usize>,
        new_chars: Range<usize>,
    ) {
        self.text.take();
        self.last_selection_map = None;
        self.revision += 1;
        self.last_change = Some(TextChange {
            revision: self.revision,
            old_lines,
            new_lines,
            old_chars,
            new_chars,
        });
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }
    pub fn selection_revision(&self) -> u64 {
        self.selection_revision
    }
    /// View-only movement never creates an undo entry or discards redo.
    pub fn set_selection(&mut self, selection: Selection) -> Result<(), String> {
        selection.validate(self.rope.len_chars())?;
        if self.selection != selection {
            self.end_edit_group();
            self.selection = selection;
            self.selection_revision += 1;
        }
        Ok(())
    }
    /// Atomic replacement, preserving directed view positions through the actual delta.
    pub fn replace(&mut self, text: String) -> Result<bool, String> {
        validate(&text)?;
        let delta = TextDelta::between(self.text(), &text);
        let after = self.selection.mapped(&delta);
        self.apply_delta_with_selection(delta, EditKind::Atomic, self.selection, after)
    }
    pub fn replace_with_selection(
        &mut self,
        text: String,
        after: Selection,
    ) -> Result<bool, String> {
        validate(&text)?;
        let delta = TextDelta::between(self.text(), &text);
        self.apply_delta_with_selection(delta, EditKind::Atomic, self.selection, after)
    }

    pub fn end_edit_group(&mut self) {
        self.group = None;
    }
    /// One atomic history entry for non-overlapping edits in original scalar coordinates.
    /// Only changed text is counted toward history; untouched gaps remain shared Rope nodes.
    pub fn apply_batch_with_selection(
        &mut self,
        mut deltas: Vec<TextDelta>,
        before: Selection,
        after: Selection,
    ) -> Result<bool, String> {
        if deltas.is_empty() || deltas.len() > 8192 {
            return Err("Lote deve conter entre 1 e 8192 edições".into());
        }
        before.validate(self.len_chars())?;
        deltas.sort_by_key(|d| (d.range.start, d.range.end));
        let mut end = 0;
        let mut previous = None;
        let mut bytes = self.len_bytes();
        let mut chars = self.len_chars();
        let mut change_bytes = 0;
        let mut changed = false;
        for d in &deltas {
            if d.range.start > d.range.end
                || d.range.end > self.len_chars()
                || d.range.start < end
                || previous == Some(d.range.start)
                || d.insert.contains('\0')
            {
                return Err("Lote contém edições inválidas ou sobrepostas".into());
            }
            let removed = self.rope.slice(d.range.clone()).len_bytes();
            changed |= self.rope.slice(d.range.clone()) != d.insert.as_str();
            bytes = bytes - removed + d.insert.len();
            chars = chars - d.range.len() + d.insert.chars().count();
            change_bytes += removed + d.insert.len();
            end = d.range.end;
            previous = Some(d.range.start);
        }
        if bytes > MAX_EDITOR_DOCUMENT_BYTES {
            return Err("Resultado excede 16 MiB no editor".into());
        }
        after.validate(chars)?;
        if !changed {
            self.set_selection(after)?;
            return Ok(false);
        }
        let mut candidate = self.rope.clone();
        for d in deltas.iter().rev() {
            candidate.remove(d.range.clone());
            candidate.insert(d.range.start, &d.insert);
        }
        if candidate.len_lines() > MAX_EDITOR_DOCUMENT_LINES {
            return Err(line_limit_error());
        }
        let old_chars = deltas[0].range.start..end;
        let new_end = if chars >= self.len_chars() {
            old_chars.end + (chars - self.len_chars())
        } else {
            old_chars.end - (self.len_chars() - chars)
        };
        let new_chars = old_chars.start..new_end;
        let old_lines = self.rope.char_to_line(old_chars.start.saturating_sub(1))
            ..(self.rope.char_to_line(end) + 1).min(self.rope.len_lines());
        let new_lines =
            old_lines.start..(candidate.char_to_line(new_chars.end) + 1).min(candidate.len_lines());
        let mut removed = 0;
        let mut inserted = 0;
        let mut inverse = Vec::with_capacity(deltas.len());
        let mut forward = Vec::with_capacity(deltas.len());
        for d in &deltas {
            let n = d.insert.chars().count();
            let start = d.range.start - removed + inserted;
            inverse.push((start..start + n, d.range.len()));
            forward.push((d.range.clone(), n));
            removed += d.range.len();
            inserted += n;
        }
        forward.reverse();
        inverse.reverse();
        self.undo.push_back(HistoryEntry {
            before: self.rope.clone(),
            after: candidate.clone(),
            change_bytes,
            before_identity: self.identity,
            after_identity: self.next_identity,
            before_selection: before,
            after_selection: after,
            old_chars: old_chars.clone(),
            new_chars: new_chars.clone(),
            batch_maps: Some((forward.clone(), inverse)),
        });
        self.rope = candidate;
        self.identity = self.next_identity;
        self.next_identity += 1;
        self.redo.clear();
        self.end_edit_group();
        self.changed(old_lines, new_lines, old_chars, new_chars);
        self.last_selection_map = Some(forward);
        self.selection = after;
        self.selection_revision += 1;
        trim_history(&mut self.undo);
        Ok(true)
    }
    pub fn apply_delta(&mut self, delta: TextDelta, kind: EditKind) -> Result<bool, String> {
        self.apply_delta_at(delta, kind, Instant::now())
    }
    fn apply_delta_at(
        &mut self,
        delta: TextDelta,
        kind: EditKind,
        now: Instant,
    ) -> Result<bool, String> {
        let before = if kind == EditKind::Atomic {
            self.selection
        } else {
            Selection::caret(if kind == EditKind::Backspace {
                delta.range.end
            } else {
                delta.range.start
            })
        };
        let after = if kind == EditKind::Atomic {
            before.mapped(&delta)
        } else {
            Selection::caret(delta.range.start + delta.insert.chars().count())
        };
        self.apply_delta_with_selection_at(delta, kind, now, before, after)
    }
    pub fn apply_delta_with_selection(
        &mut self,
        delta: TextDelta,
        kind: EditKind,
        before: Selection,
        after: Selection,
    ) -> Result<bool, String> {
        self.apply_delta_with_selection_at(delta, kind, Instant::now(), before, after)
    }
    fn apply_delta_with_selection_at(
        &mut self,
        delta: TextDelta,
        kind: EditKind,
        now: Instant,
        before: Selection,
        after: Selection,
    ) -> Result<bool, String> {
        let r = delta.range;
        if r.start > r.end || r.end > self.rope.len_chars() {
            return Err("Intervalo escalar inválido".into());
        }
        before.validate(self.rope.len_chars())?;
        after.validate(self.rope.len_chars() - r.len() + delta.insert.chars().count())?;
        if delta.insert.contains('\0') {
            return Err("Edição contém NUL".into());
        }
        let a = self.rope.char_to_byte(r.start);
        let b = self.rope.char_to_byte(r.end);
        if self.rope.len_bytes() - (b - a) + delta.insert.len() > MAX_EDITOR_DOCUMENT_BYTES {
            return Err("Resultado excede 16 MiB no editor".into());
        }
        if self.rope.slice(r.clone()) == delta.insert.as_str() {
            self.set_selection(after)?;
            return Ok(false);
        }
        let eligible = after.is_caret()
            && after.head == r.start + delta.insert.chars().count()
            && match kind {
                EditKind::Typing => {
                    (r.is_empty() || before.range() == r)
                        && !delta.insert.is_empty()
                        && !delta.insert.contains(['\r', '\n'])
                }
                EditKind::Backspace | EditKind::Delete => {
                    before.is_caret() && !r.is_empty() && delta.insert.is_empty()
                }
                EditKind::Atomic => false,
            };
        let merge = eligible
            && before.is_caret()
            && before == self.selection
            && self.group.as_ref().is_some_and(|g| {
                g.kind == kind
                    && now
                        .checked_duration_since(g.at)
                        .is_some_and(|d| d <= Duration::from_secs(2))
                    && g.caret
                        == if kind == EditKind::Backspace {
                            r.end
                        } else {
                            r.start
                        }
            });
        // Prepare and validate the copy-on-write result before touching the live
        // rope, history stacks, group, selection, cache or revision.
        let mut candidate = self.rope.clone();
        candidate.remove(r.clone());
        candidate.insert(r.start, &delta.insert);
        if candidate.len_lines() > MAX_EDITOR_DOCUMENT_LINES {
            return Err(line_limit_error());
        }
        let old_lines = self.rope.char_to_line(r.start.saturating_sub(1))
            ..(self.rope.char_to_line(r.end) + 1).min(self.rope.len_lines());
        let before_rope = self.rope.clone();
        let before_identity = self.identity;
        self.redo.clear();
        self.rope = candidate;
        let new_end = r.start + delta.insert.chars().count();
        let new_lines =
            old_lines.start..(self.rope.char_to_line(new_end) + 1).min(self.rope.len_lines());
        self.identity = self.next_identity;
        self.next_identity += 1;
        self.changed(old_lines, new_lines, r.clone(), r.start..new_end);
        self.selection = after;
        self.selection_revision += 1;
        if merge {
            let entry = self.undo.back_mut().expect("active edit group has history");
            entry.after = self.rope.clone();
            entry.after_identity = self.identity;
            entry.change_bytes += b - a + delta.insert.len();
            entry.after_selection = after;
            entry.old_chars.start = entry.old_chars.start.min(r.start);
            entry.old_chars.end += r.end.saturating_sub(entry.new_chars.end);
            entry.new_chars.start = entry.new_chars.start.min(r.start);
            entry.new_chars.end = Selection::caret(entry.new_chars.end)
                .mapped(&TextDelta {
                    range: r.clone(),
                    insert: delta.insert.clone(),
                })
                .head;
        } else {
            self.undo.push_back(HistoryEntry {
                before: before_rope,
                after: self.rope.clone(),
                change_bytes: b - a + delta.insert.len(),
                before_identity,
                after_identity: self.identity,
                before_selection: before,
                after_selection: after,
                old_chars: r.clone(),
                new_chars: r.start..new_end,
                batch_maps: None,
            });
        }
        trim_history(&mut self.undo);
        self.group = eligible.then_some(EditGroup {
            kind,
            caret: r.start + delta.insert.chars().count(),
            at: now,
        });
        Ok(true)
    }

    pub fn undo(&mut self) -> bool {
        self.end_edit_group();
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        let old_lines = 0..self.rope.len_lines();
        self.rope = previous.before.clone();
        self.identity = previous.before_identity;
        self.changed(
            old_lines,
            0..self.rope.len_lines(),
            previous.new_chars.clone(),
            previous.old_chars.clone(),
        );
        self.selection = previous.before_selection;
        self.last_selection_map = previous
            .batch_maps
            .as_ref()
            .map(|(_, inverse)| inverse.clone());
        self.selection_revision += 1;
        self.redo.push_back(previous);
        trim_history(&mut self.redo);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_edit_group();
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        let old_lines = 0..self.rope.len_lines();
        self.rope = next.after.clone();
        self.identity = next.after_identity;
        self.changed(
            old_lines,
            0..self.rope.len_lines(),
            next.old_chars.clone(),
            next.new_chars.clone(),
        );
        self.selection = next.after_selection;
        self.last_selection_map = next.batch_maps.as_ref().map(|(forward, _)| forward.clone());
        self.selection_revision += 1;
        self.undo.push_back(next);
        trim_history(&mut self.undo);
        true
    }

    /// Absolute scalar index, not a grapheme or byte offset.
    pub fn char_to_utf16(&self, index: usize) -> Result<usize, String> {
        if index > self.rope.len_chars() {
            return Err("Índice Unicode fora do documento".into());
        }
        Ok(self.rope.char_to_utf16_cu(index))
    }

    /// Reject a UTF-16 position splitting a surrogate pair.
    pub fn utf16_to_char(&self, units: usize) -> Result<usize, String> {
        let index = self
            .rope
            .try_utf16_cu_to_char(units)
            .map_err(|_| "Posição UTF-16 fora do documento".to_string())?;
        if self.rope.char_to_utf16_cu(index) != units {
            Err("Posição UTF-16 divide um par substituto".into())
        } else {
            Ok(index)
        }
    }
}

/// Uses Ropey's configured LF/CRLF/CR line conversion with the
/// same semantics as `Rope::len_lines()`, without allocating a second buffer.
pub fn validate_editor_line_count(text: &str) -> Result<(), String> {
    let lines = ropey::str_utils::byte_to_line_idx(text, text.len()) + 1;
    if lines > MAX_EDITOR_DOCUMENT_LINES {
        Err(line_limit_error())
    } else {
        Ok(())
    }
}
fn line_limit_error() -> String {
    "Documento excede o limite do editor de 1.000.000 linhas (incluindo a linha final vazia)".into()
}

fn validate(text: &str) -> Result<(), String> {
    if text.len() > MAX_EDITOR_DOCUMENT_BYTES {
        Err("Documento excede o limite do editor de 16 MiB".into())
    } else if text.contains('\0') {
        Err("Arquivo binário (NUL) não é editável na M1".into())
    } else {
        validate_editor_line_count(text)
    }
}

fn trim_history(history: &mut VecDeque<HistoryEntry>) {
    let mut bytes: usize = history.iter().map(|e| e.change_bytes).sum();
    while history.len() > MAX_HISTORY_ENTRIES || bytes > MAX_HISTORY_BYTES {
        if let Some(old) = history.pop_front() {
            bytes -= old.change_bytes;
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_crlf_and_history_round_trip() {
        let original = "<?php\r\n// ação e\u{301} 😀\r\nsem newline";
        let mut b = TextBuffer::new(original.into()).unwrap();
        b.replace(format!("{original}!")).unwrap();
        assert!(b.undo());
        assert_eq!(b.text(), original);
        assert!(b.redo());
        assert_eq!(b.text(), format!("{original}!"));
        b.undo();
        b.replace("ramo novo".into()).unwrap();
        assert!(!b.can_redo());
    }

    #[test]
    fn utf16_never_confuses_bytes_scalars_or_surrogates() {
        let b = TextBuffer::new("á😀e\u{301}".into()).unwrap();
        assert_eq!(b.char_to_utf16(2).unwrap(), 3);
        assert_eq!(b.utf16_to_char(3).unwrap(), 2);
        assert!(b.utf16_to_char(2).is_err());
        assert_eq!(b.utf16_to_char(5).unwrap(), 4);
        assert!(b.char_to_utf16(5).is_err());
    }

    #[test]
    fn history_is_bounded_and_oversized_edit_is_atomic() {
        let mut b = TextBuffer::new("start".into()).unwrap();
        for n in 0..200 {
            b.replace(n.to_string()).unwrap();
        }
        assert!(b.undo.len() <= MAX_HISTORY_ENTRIES);
        let before = b.text().to_owned();
        assert!(
            b.replace("x".repeat(MAX_EDITOR_DOCUMENT_BYTES + 1))
                .is_err()
        );
        assert_eq!(b.text(), before);
    }
}

#[cfg(test)]
mod delta_tests {
    use super::*;
    #[test]
    fn minimal_deltas_preserve_unicode_crlf_and_shared_suffixes() {
        let corpus = [
            "",
            "á😀e\u{301}\r\n",
            "<?php\r\n",
            "😀🌧abc",
            "aé😀z",
            "a😀éz",
            "same same",
            "same",
        ];
        for before in corpus {
            for after in corpus {
                let delta = TextDelta::between(before, after);
                let mut b = TextBuffer::new(before.into()).unwrap();
                let changed = b.apply_delta(delta, EditKind::Atomic).unwrap();
                assert_eq!(b.text(), after);
                if changed {
                    assert!(b.undo());
                    assert_eq!(b.text(), before);
                    assert!(b.redo());
                    assert_eq!(b.text(), after);
                }
            }
        }
    }
    #[test]
    fn caret_anchored_repeated_text_keeps_actual_position_and_one_undo() {
        for (base, addition, offset) in [
            ("weather", "stormwind", 0),
            ("aaaa", "aaa", 2),
            ("é😀é", "é😀é", 0),
        ] {
            let mut b = TextBuffer::new(base.into()).unwrap();
            for (caret, c) in (offset..).zip(addition.chars()) {
                let mut next = b.text().to_owned();
                let byte = next
                    .char_indices()
                    .nth(caret)
                    .map_or(next.len(), |(n, _)| n);
                next.insert(byte, c);
                let delta = TextDelta::at_caret(b.text(), &next, caret + 1);
                assert_eq!(delta.range, caret..caret);
                b.apply_delta(delta, EditKind::Typing).unwrap();
                assert_eq!(b.text(), next);
            }
            assert!(b.undo());
            assert_eq!(b.text(), base);
        }
        assert_eq!(TextDelta::at_caret("aaa", "aa", 0).range, 0..1);
        assert_eq!(TextDelta::at_caret("aaa", "aa", 2).range, 2..3);
    }
    #[test]
    fn contiguous_typing_with_observed_pause_groups_and_explicit_boundaries_split() {
        let now = Instant::now();
        let mut b = TextBuffer::new("".into()).unwrap();
        for (i, c) in "weather".chars().enumerate() {
            b.apply_delta_at(
                TextDelta {
                    range: i..i,
                    insert: c.to_string(),
                },
                EditKind::Typing,
                now + Duration::from_millis(i as u64 * 100),
            )
            .unwrap();
        }
        b.apply_delta_at(
            TextDelta {
                range: 7..7,
                insert: "rain".into(),
            },
            EditKind::Typing,
            now + Duration::from_millis(1880),
        )
        .unwrap();
        assert!(b.undo());
        assert_eq!(b.text(), "");
        assert!(b.redo());
        assert_eq!(b.text(), "weatherrain");
        b.apply_delta_at(
            TextDelta {
                range: 11..11,
                insert: "!".into(),
            },
            EditKind::Typing,
            now + Duration::from_secs(5),
        )
        .unwrap();
        b.undo();
        assert_eq!(b.text(), "weatherrain");
        b.end_edit_group();
        b.apply_delta(
            TextDelta {
                range: 11..11,
                insert: "x".into(),
            },
            EditKind::Typing,
        )
        .unwrap();
        b.apply_delta(
            TextDelta {
                range: 12..12,
                insert: "\n".into(),
            },
            EditKind::Typing,
        )
        .unwrap();
        b.undo();
        assert_eq!(b.text(), "weatherrainx");
        b.undo();
        assert_eq!(b.text(), "weatherrain");
    }
    #[test]
    fn deletion_directions_selection_and_paste_have_distinct_history() {
        let mut b = TextBuffer::new("a😀bc".into()).unwrap();
        b.apply_delta(
            TextDelta {
                range: 3..4,
                insert: "".into(),
            },
            EditKind::Backspace,
        )
        .unwrap();
        b.apply_delta(
            TextDelta {
                range: 2..3,
                insert: "".into(),
            },
            EditKind::Backspace,
        )
        .unwrap();
        assert_eq!(b.text(), "a😀");
        b.undo();
        assert_eq!(b.text(), "a😀bc");
        b.apply_delta(
            TextDelta {
                range: 1..2,
                insert: "".into(),
            },
            EditKind::Delete,
        )
        .unwrap();
        b.apply_delta(
            TextDelta {
                range: 1..2,
                insert: "".into(),
            },
            EditKind::Delete,
        )
        .unwrap();
        b.undo();
        assert_eq!(b.text(), "a😀bc");
        b.apply_delta(
            TextDelta {
                range: 1..2,
                insert: "雨".into(),
            },
            EditKind::Typing,
        )
        .unwrap();
        b.apply_delta(
            TextDelta {
                range: 2..2,
                insert: "paste".into(),
            },
            EditKind::Atomic,
        )
        .unwrap();
        b.undo();
        assert_eq!(b.text(), "a雨bc");
        b.undo();
        assert_eq!(b.text(), "a😀bc");
    }
    #[test]
    fn invalid_deltas_are_atomic_and_do_not_destroy_history_or_redo() {
        let mut b = TextBuffer::new("😀".into()).unwrap();
        b.apply_delta(
            TextDelta {
                range: 1..1,
                insert: "a".into(),
            },
            EditKind::Typing,
        )
        .unwrap();
        b.undo();
        for delta in [
            TextDelta {
                range: 2..2,
                insert: "x".into(),
            },
            TextDelta {
                range: Range { start: 1, end: 0 },
                insert: "".into(),
            },
            TextDelta {
                range: 0..0,
                insert: "\0".into(),
            },
            TextDelta {
                range: 0..0,
                insert: "x".repeat(MAX_EDITOR_DOCUMENT_BYTES),
            },
        ] {
            assert!(b.apply_delta(delta, EditKind::Typing).is_err());
            assert_eq!(b.text(), "😀");
            assert!(b.can_redo());
        }
        b.redo();
        assert_eq!(b.text(), "😀a");
    }
}

#[cfg(test)]
mod selection_history_tests {
    use super::*;
    #[test]
    fn directed_replacement_restores_edit_endpoints_after_unrelated_navigation() {
        for before in [
            Selection { anchor: 2, head: 5 },
            Selection { anchor: 5, head: 2 },
        ] {
            let mut b = TextBuffer::new("a😀e\u{301}z!".into()).unwrap();
            b.set_selection(before).unwrap();
            b.apply_delta_with_selection(
                TextDelta {
                    range: 2..5,
                    insert: "雨".into(),
                },
                EditKind::Atomic,
                before,
                Selection::caret(3),
            )
            .unwrap();
            b.set_selection(Selection::caret(0)).unwrap();
            assert!(b.undo());
            assert_eq!(b.text(), "a😀e\u{301}z!");
            assert_eq!(b.selection(), before);
            b.set_selection(Selection::caret(6)).unwrap();
            assert!(b.redo());
            assert_eq!(b.text(), "a😀雨!");
            assert_eq!(b.selection(), Selection::caret(3));
            assert!(b.undo());
            assert_eq!(b.selection(), before);
        }
    }
    #[test]
    fn merged_group_keeps_first_selection_and_last_result_and_redo_branch() {
        let now = Instant::now();
        let mut b = TextBuffer::new("// \r\n".into()).unwrap();
        let mut caret = 3;
        for c in "😀e\u{301}rain".chars() {
            b.apply_delta_with_selection_at(
                TextDelta {
                    range: caret..caret,
                    insert: c.to_string(),
                },
                EditKind::Typing,
                now,
                Selection::caret(caret),
                Selection::caret(caret + 1),
            )
            .unwrap();
            caret += 1;
        }
        let result = b.text().to_owned();
        assert!(b.undo());
        assert_eq!(b.text(), "// \r\n");
        assert_eq!(b.selection(), Selection::caret(3));
        b.set_selection(Selection::caret(0)).unwrap();
        assert!(b.redo());
        assert_eq!(b.text(), result);
        assert_eq!(b.selection(), Selection::caret(caret));
        b.undo();
        b.apply_delta_with_selection(
            TextDelta {
                range: 3..3,
                insert: "other".into(),
            },
            EditKind::Atomic,
            Selection::caret(3),
            Selection::caret(8),
        )
        .unwrap();
        assert!(!b.can_redo());
        assert_eq!(b.text(), "// other\r\n");
    }
    #[test]
    fn per_key_typing_over_selection_groups_the_first_replacement_with_the_word() {
        for before in [
            Selection { anchor: 2, head: 9 },
            Selection { anchor: 9, head: 2 },
        ] {
            let mut b = TextBuffer::new("😀 weather!".into()).unwrap();
            b.set_selection(before).unwrap();
            let now = Instant::now();
            for (n, ch) in "storm".chars().enumerate() {
                let selection = b.selection();
                b.apply_delta_with_selection_at(
                    TextDelta {
                        range: selection.range(),
                        insert: ch.to_string(),
                    },
                    EditKind::Typing,
                    now + Duration::from_millis(n as u64 * 100),
                    selection,
                    Selection::caret(3 + n),
                )
                .unwrap();
            }
            assert_eq!(b.text(), "😀 storm!");
            assert!(b.undo());
            assert_eq!(b.text(), "😀 weather!");
            assert_eq!(b.selection(), before);
            assert!(!b.can_undo());
            assert!(b.redo());
            assert_eq!(b.selection(), Selection::caret(7));
        }
    }
    #[test]
    fn invalid_view_transaction_preserves_text_selections_redo_and_revision() {
        let mut b = TextBuffer::new("😀abc".into()).unwrap();
        b.apply_delta(
            TextDelta {
                range: 1..1,
                insert: "x".into(),
            },
            EditKind::Typing,
        )
        .unwrap();
        b.undo();
        let selection = b.selection();
        let revision = b.selection_revision();
        assert!(b.set_selection(Selection::caret(5)).is_err());
        for (before, after) in [
            (Selection::caret(5), Selection::caret(2)),
            (Selection::caret(1), Selection::caret(6)),
        ] {
            assert!(
                b.apply_delta_with_selection(
                    TextDelta {
                        range: 1..2,
                        insert: "雨".into()
                    },
                    EditKind::Atomic,
                    before,
                    after
                )
                .is_err()
            );
            assert_eq!(b.text(), "😀abc");
            assert_eq!(b.selection(), selection);
            assert_eq!(b.selection_revision(), revision);
            assert!(b.can_redo());
        }
        assert!(b.redo());
        assert_eq!(b.text(), "😀xabc");
    }
    #[test]
    fn history_budget_counts_changed_bytes_and_view_only_changes_do_not_add_steps() {
        let mut b = TextBuffer::new("start".into()).unwrap();
        for n in 0..180 {
            b.replace(format!("{n}{}", "x".repeat(40_000))).unwrap();
        }
        assert!(b.undo.len() <= MAX_HISTORY_ENTRIES);
        assert!(b.undo.iter().map(|e| e.change_bytes).sum::<usize>() <= MAX_HISTORY_BYTES);
        let steps = b.undo.len();
        let endpoint = Selection::caret(b.text().chars().count());
        for n in 0..20 {
            b.set_selection(Selection::caret(n)).unwrap();
        }
        assert_eq!(b.undo.len(), steps);
        b.undo();
        assert!(b.can_redo());
        b.set_selection(Selection::caret(0)).unwrap();
        assert!(b.redo());
        assert_eq!(b.selection(), endpoint);
    }
}

#[cfg(test)]
mod incremental_snapshot_tests {
    use super::*;
    #[test]
    fn snapshots_are_immutable_and_edits_undo_redo_do_not_materialize_legacy_text() {
        let mut buffer = TextBuffer::new("ab\r\ncd\ne\u{301}👩‍💻".into()).unwrap();
        let old = buffer.snapshot();
        let saved_identity = buffer.content_identity();
        buffer
            .apply_delta(
                TextDelta {
                    range: 5..6,
                    insert: "changed".into(),
                },
                EditKind::Atomic,
            )
            .unwrap();
        let edited = buffer.snapshot();
        assert!(buffer.text.get().is_none());
        assert_eq!(old.to_string(), "ab\r\ncd\ne\u{301}👩‍💻");
        assert_eq!(buffer.last_change().unwrap().old_lines, 1..2);
        assert_eq!(buffer.last_change().unwrap().new_lines, 1..2);
        assert!(buffer.undo());
        assert_eq!(buffer.content_identity(), saved_identity);
        assert!(buffer.text.get().is_none());
        assert_eq!(buffer.snapshot(), old);
        assert!(buffer.redo());
        assert!(buffer.text.get().is_none());
        assert_eq!(buffer.snapshot(), edited);
        assert_eq!(buffer.revision(), 3);
        assert_eq!(buffer.line_count(), 3);
        assert_eq!(buffer.line_to_char(1), Some(4));
        assert_eq!(buffer.char_to_line(4), Some(1));
        assert_eq!(buffer.line(1).unwrap(), "cchanged\n");
        assert!(buffer.line(3).is_none());
        assert!(buffer.slice(100..101).is_none());
    }
    #[test]
    fn crlf_join_invalidation_and_rejected_transactions_leave_snapshot_revision_untouched() {
        let mut buffer = TextBuffer::new("ab\r\ncd\nef".into()).unwrap();
        buffer
            .apply_delta(
                TextDelta {
                    range: 2..4,
                    insert: String::new(),
                },
                EditKind::Backspace,
            )
            .unwrap();
        assert_eq!(buffer.last_change().unwrap().old_lines, 0..2);
        assert_eq!(buffer.last_change().unwrap().new_lines, 0..1);
        let snapshot = buffer.snapshot();
        let change = buffer.last_change().cloned();
        let identity = buffer.content_identity();
        for delta in [
            TextDelta {
                range: 0..100,
                insert: "x".into(),
            },
            TextDelta {
                range: 0..0,
                insert: "\0".into(),
            },
            TextDelta {
                range: 0..0,
                insert: "x".repeat(MAX_EDITOR_DOCUMENT_BYTES),
            },
        ] {
            assert!(buffer.apply_delta(delta, EditKind::Atomic).is_err());
            assert_eq!(buffer.snapshot(), snapshot);
            assert_eq!(buffer.last_change(), change.as_ref());
            assert_eq!(buffer.content_identity(), identity);
        }
        assert!(buffer.undo());
        assert_eq!(buffer.snapshot(), "ab\r\ncd\nef");
    }
    #[test]
    fn backspace_and_delete_cluster_transactions_preserve_direction_and_undo_bytes() {
        for source in ["e\u{301}", "👩🏽‍💻", "🇧🇷", "\r\n"] {
            let mut buffer = TextBuffer::new(format!("a{source}z")).unwrap();
            let end = 1 + source.chars().count();
            let range = previous_grapheme(&buffer.snapshot(), end)..end;
            assert_eq!(range.start, 1);
            buffer
                .apply_delta_with_selection(
                    TextDelta {
                        range,
                        insert: String::new(),
                    },
                    EditKind::Backspace,
                    Selection::caret(end),
                    Selection::caret(1),
                )
                .unwrap();
            assert_eq!(buffer.snapshot(), "az");
            assert!(buffer.undo());
            assert_eq!(buffer.selection(), Selection::caret(end));
            assert_eq!(buffer.snapshot(), format!("a{source}z"));
            let next = next_grapheme(&buffer.snapshot(), 1);
            buffer
                .apply_delta_with_selection(
                    TextDelta {
                        range: 1..next,
                        insert: String::new(),
                    },
                    EditKind::Delete,
                    Selection::caret(1),
                    Selection::caret(1),
                )
                .unwrap();
            assert_eq!(buffer.snapshot(), "az");
            assert!(buffer.undo());
            assert_eq!(buffer.snapshot(), format!("a{source}z"));
        }
    }
}

#[cfg(test)]
mod large_document_tests {
    use super::*;
    #[test]
    fn measured_sizes_retain_local_history_and_full_limit_paste_and_replacement() {
        for size in [1024 * 1024, 4 * 1024 * 1024, MAX_EDITOR_DOCUMENT_BYTES] {
            let mut buffer = TextBuffer::new("a".repeat(size)).unwrap();
            for position in [0, size / 2, size - 1] {
                buffer
                    .apply_delta(
                        TextDelta {
                            range: position..position + 1,
                            insert: "b".into(),
                        },
                        EditKind::Atomic,
                    )
                    .unwrap();
                assert!(buffer.text.get().is_none());
                assert_eq!(buffer.snapshot().char(position), 'b');
                assert!(buffer.undo());
                assert_eq!(buffer.snapshot().char(position), 'a');
                assert!(buffer.redo());
                assert!(buffer.undo());
            }
            assert_eq!(buffer.len_bytes(), size);
        }
        let mut buffer = TextBuffer::new(String::new()).unwrap();
        buffer
            .apply_delta(
                TextDelta {
                    range: 0..0,
                    insert: "a".repeat(MAX_EDITOR_DOCUMENT_BYTES),
                },
                EditKind::Atomic,
            )
            .unwrap();
        assert!(buffer.undo());
        assert_eq!(buffer.len_chars(), 0);
        assert!(buffer.redo());
        buffer
            .apply_delta(
                TextDelta {
                    range: 0..MAX_EDITOR_DOCUMENT_BYTES,
                    insert: "b".repeat(MAX_EDITOR_DOCUMENT_BYTES),
                },
                EditKind::Atomic,
            )
            .unwrap();
        assert_eq!(
            buffer.undo.len(),
            1,
            "32 MiB replacement evicts older changed-byte budget"
        );
        assert!(buffer.undo());
        assert_eq!(buffer.snapshot().char(MAX_EDITOR_DOCUMENT_BYTES - 1), 'a');
        assert!(buffer.redo());
        assert_eq!(buffer.snapshot().char(MAX_EDITOR_DOCUMENT_BYTES - 1), 'b');
        assert!(
            buffer
                .undo
                .iter()
                .map(|entry| entry.change_bytes)
                .sum::<usize>()
                <= MAX_HISTORY_BYTES
        );
        assert!(
            buffer
                .redo
                .iter()
                .map(|entry| entry.change_bytes)
                .sum::<usize>()
                <= MAX_HISTORY_BYTES
        );
    }
}

#[cfg(test)]
mod line_limit_tests {
    use super::*;
    #[test]
    fn canonical_line_limit_counts_crlf_unicode_and_last_empty_line_without_truncation() {
        for line_break in ["\n", "\r\n", "\r"] {
            let source = line_break.repeat(MAX_EDITOR_DOCUMENT_LINES - 1);
            let buffer = TextBuffer::new(source.clone()).unwrap();
            assert_eq!(buffer.line_count(), MAX_EDITOR_DOCUMENT_LINES);
            assert!(buffer.matches(&source));
            let exceeded = format!("{source}{line_break}");
            assert_eq!(
                TextSnapshot::from_str(&exceeded).len_lines(),
                MAX_EDITOR_DOCUMENT_LINES + 1
            );
            assert!(
                validate_editor_line_count(&exceeded)
                    .unwrap_err()
                    .contains("1.000.000 linhas")
            );
            assert!(TextBuffer::new(exceeded).is_err());
        }
    }
    #[test]
    fn line_limit_rejection_preserves_directed_history_group_and_all_revisions() {
        let mut buffer = TextBuffer::new("\n".repeat(MAX_EDITOR_DOCUMENT_LINES - 1)).unwrap();
        buffer
            .apply_delta(
                TextDelta {
                    range: 0..0,
                    insert: "x".into(),
                },
                EditKind::Typing,
            )
            .unwrap();
        assert!(buffer.undo());
        let before = Selection { anchor: 4, head: 1 };
        buffer.set_selection(before).unwrap();
        let original = buffer.snapshot();
        let revision = buffer.revision();
        let selection_revision = buffer.selection_revision();
        let change = buffer.last_change().cloned();
        let identity = buffer.content_identity();
        for (delta, after) in [
            (
                TextDelta {
                    range: 0..0,
                    insert: "\n".into(),
                },
                Selection::caret(1),
            ),
            (
                TextDelta {
                    range: 0..0,
                    insert: "\r\n".into(),
                },
                Selection::caret(2),
            ),
            (
                TextDelta {
                    range: 0..1,
                    insert: "\n\n".into(),
                },
                Selection::caret(2),
            ),
        ] {
            assert!(
                buffer
                    .apply_delta_with_selection(delta, EditKind::Atomic, before, after)
                    .unwrap_err()
                    .contains("1.000.000 linhas")
            );
            assert_eq!(buffer.snapshot(), original);
            assert_eq!(buffer.selection(), before);
            assert_eq!(buffer.revision(), revision);
            assert_eq!(buffer.selection_revision(), selection_revision);
            assert_eq!(buffer.last_change(), change.as_ref());
            assert_eq!(buffer.content_identity(), identity);
            assert!(buffer.can_redo());
            assert!(!buffer.can_undo());
        }
        assert!(buffer.redo());
        assert_eq!(buffer.snapshot().char(0), 'x');
        assert!(buffer.undo());
        buffer
            .apply_delta(
                TextDelta {
                    range: 0..0,
                    insert: "a".into(),
                },
                EditKind::Typing,
            )
            .unwrap();
        let previous_entries = buffer.undo.len();
        assert!(
            buffer
                .apply_delta(
                    TextDelta {
                        range: 1..1,
                        insert: "\n".into()
                    },
                    EditKind::Typing
                )
                .is_err()
        );
        buffer
            .apply_delta(
                TextDelta {
                    range: 1..1,
                    insert: "b".into(),
                },
                EditKind::Typing,
            )
            .unwrap();
        assert_eq!(
            buffer.undo.len(),
            previous_entries,
            "rejection must not split an accepted typing group"
        );
        assert!(buffer.undo());
        assert_eq!(buffer.snapshot(), original);
    }
}

#[cfg(test)]
mod view_delta_tests {
    use super::*;
    fn unchanged_outside(before: &str, after: &str, c: &TextChange) {
        let before = TextSnapshot::from_str(before);
        let after = TextSnapshot::from_str(after);
        assert_eq!(
            before.slice(..c.old_chars.start).to_string(),
            after.slice(..c.new_chars.start).to_string()
        );
        assert_eq!(
            before.slice(c.old_chars.end..).to_string(),
            after.slice(c.new_chars.end..).to_string()
        );
    }
    #[test]
    fn grouped_typing_backspace_delete_and_directed_replacement_expose_exact_scalar_spans() {
        for kind in [EditKind::Typing, EditKind::Backspace, EditKind::Delete] {
            let original = "α😀abcdef\r\nend";
            let mut b = TextBuffer::new(original.into()).unwrap();
            for step in 0..3 {
                let (range, insert) = match kind {
                    EditKind::Typing => (2 + step..2 + step, "汉"),
                    EditKind::Backspace => (5 - step..6 - step, ""),
                    _ => (2..3, ""),
                };
                let before = b.snapshot().to_string();
                b.apply_delta(
                    TextDelta {
                        range,
                        insert: insert.into(),
                    },
                    kind,
                )
                .unwrap();
                unchanged_outside(&before, b.text(), b.last_change().unwrap());
            }
            let final_text = b.text().to_owned();
            assert!(b.undo());
            assert_eq!(b.text(), original);
            unchanged_outside(&final_text, b.text(), b.last_change().unwrap());
            assert!(b.redo());
            assert_eq!(b.text(), final_text);
            unchanged_outside(original, b.text(), b.last_change().unwrap());
        }
    }
}

#[cfg(test)]
mod staging_clone_tests {
    use super::*;
    #[test]
    fn staged_clone_shares_snapshots_without_copying_cache_or_mutating_original_history() {
        let mut original = TextBuffer::new("Weather 😀\r\n".into()).unwrap();
        original
            .replace_with_selection(
                "Weather dirty 😀\r\n".into(),
                Selection {
                    anchor: 7,
                    head: 13,
                },
            )
            .unwrap();
        let revision = original.revision();
        let identity = original.content_identity();
        let mut staged = original.clone();
        assert!(staged.text.get().is_none());
        assert_eq!(staged.revision(), revision);
        assert_eq!(staged.content_identity(), identity);
        assert_eq!(staged.selection(), original.selection());
        staged
            .replace_with_selection("Forecast dirty 😀\r\n".into(), Selection::caret(8))
            .unwrap();
        assert_eq!(original.text(), "Weather dirty 😀\r\n");
        assert_eq!(original.revision(), revision);
        assert!(staged.undo());
        assert_eq!(staged.text(), original.text());
        assert_eq!(staged.selection(), original.selection());
        assert!(staged.undo());
        assert_eq!(staged.text(), "Weather 😀\r\n");
        assert!(original.can_undo());
        assert!(!original.can_redo());
        assert!(staged.redo());
        assert!(staged.redo());
        assert_eq!(staged.text(), "Forecast dirty 😀\r\n");
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    #[test]
    fn unicode_batch_changes_only_targets_and_is_one_history_step() {
        let original = "ação 😀 ação\r\nação";
        let mut b = TextBuffer::new(original.into()).unwrap();
        let before = Selection {
            anchor: 7,
            head: 11,
        };
        let after = Selection::caret(5);
        b.apply_batch_with_selection(
            vec![0..4, 7..11, 13..17]
                .into_iter()
                .map(|range| TextDelta {
                    range,
                    insert: "雨".into(),
                })
                .collect(),
            before,
            after,
        )
        .unwrap();
        assert_eq!(b.text(), "雨 😀 雨\r\n雨");
        assert_eq!(b.revision(), 1);
        assert!(b.undo());
        assert_eq!(b.text(), original);
        assert_eq!(b.selection(), before);
        assert!(!b.undo());
        assert!(b.redo());
        assert_eq!(b.text(), "雨 😀 雨\r\n雨");
        assert_eq!(b.selection(), after);
    }
    #[test]
    fn invalid_batch_preserves_text_selection_revision_history_and_redo() {
        let mut b = TextBuffer::new("abc def".into()).unwrap();
        b.apply_delta(
            TextDelta {
                range: 0..0,
                insert: "X".into(),
            },
            EditKind::Atomic,
        )
        .unwrap();
        b.undo();
        for deltas in [
            vec![
                TextDelta {
                    range: 0..4,
                    insert: "x".into(),
                },
                TextDelta {
                    range: 3..5,
                    insert: "x".into(),
                },
            ],
            vec![TextDelta {
                range: 8..8,
                insert: "x".into(),
            }],
            vec![TextDelta {
                range: 0..0,
                insert: "\0".into(),
            }],
            vec![TextDelta {
                range: 0..0,
                insert: "x".repeat(MAX_EDITOR_DOCUMENT_BYTES),
            }],
        ] {
            let revision = b.revision();
            assert!(
                b.apply_batch_with_selection(deltas, Selection::caret(0), Selection::caret(0))
                    .is_err()
            );
            assert_eq!(b.text(), "abc def");
            assert_eq!(b.revision(), revision);
            assert!(b.can_redo());
            assert!(!b.can_undo());
        }
    }
    #[test]
    fn widely_spaced_batch_does_not_charge_unchanged_gaps_to_history_budget() {
        let original = format!("a{}b", " ".repeat(12 * 1024 * 1024));
        let mut b = TextBuffer::new(original.clone()).unwrap();
        for _ in 0..4 {
            let n = b.len_chars();
            b.apply_batch_with_selection(
                vec![
                    TextDelta {
                        range: 0..1,
                        insert: "x".into(),
                    },
                    TextDelta {
                        range: n - 1..n,
                        insert: "y".into(),
                    },
                ],
                Selection::caret(0),
                Selection::caret(1),
            )
            .unwrap();
            let n = b.len_chars();
            b.apply_batch_with_selection(
                vec![
                    TextDelta {
                        range: 0..1,
                        insert: "a".into(),
                    },
                    TextDelta {
                        range: n - 1..n,
                        insert: "b".into(),
                    },
                ],
                Selection::caret(0),
                Selection::caret(1),
            )
            .unwrap();
        }
        for _ in 0..8 {
            assert!(b.undo());
        }
        assert_eq!(b.text(), original);
    }
}

#[cfg(test)]
mod conventional_line_tests {
    use super::*;
    #[test]
    fn editor_lines_match_lf_crlf_cr_without_splitting_unicode_controls() {
        for eol in ["\n", "\r\n", "\r"] {
            let content = (1..=140)
                .map(|n| format!("line{n}:\u{b}\u{c}\u{85}\u{2028}\u{2029}"))
                .collect::<Vec<_>>()
                .join(eol);
            let buffer = TextBuffer::new(content.clone()).unwrap();
            assert_eq!(buffer.line_count(), 140);
            let pos = buffer.line_to_char(66).unwrap();
            assert!(
                buffer
                    .snapshot()
                    .slice(pos..)
                    .to_string()
                    .starts_with("line67:")
            );
            assert_eq!(buffer.char_to_line(pos), Some(66));
            assert_eq!(buffer.text(), content);
            let with_final = TextBuffer::new(format!("{content}{eol}")).unwrap();
            assert_eq!(with_final.line_count(), 141);
        }
    }
}
