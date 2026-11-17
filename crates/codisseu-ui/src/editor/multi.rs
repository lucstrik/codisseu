//! View-local occurrence selections; canonical edits still use versioned domain transactions.
use super::*;
use codisseu_core::{AppService, TextDelta};
use std::{collections::VecDeque, ops::Range};

const MAX_SELECTIONS: usize = 512;
#[derive(Default)]
pub(super) struct MultiSelection {
    pub extra: Vec<Selection>,
    stamp: Option<(u64, Selection)>,
    history: VecDeque<(u64, Selection, Vec<Selection>)>,
    scan: Option<Scan>,
    pending: usize,
    cursor_history: VecDeque<(u64, Selection, Vec<Selection>)>,
    skip_primary: bool,
}
struct Scan {
    version: u64,
    needle: Vec<char>,
    prefix: Vec<usize>,
    matched: usize,
    cursor: usize,
    pivot: usize,
    wrapped: bool,
}
impl MultiSelection {
    pub fn sync(&mut self, identity: u64, primary: Selection) {
        if self.stamp == Some((identity, primary)) {
            return;
        }
        self.scan = None;
        self.pending = 0;
        self.extra = if self.stamp.is_some_and(|s| s.0 != identity) {
            self.history
                .iter()
                .rev()
                .find(|(i, p, _)| *i == identity && *p == primary)
                .map_or_else(Vec::new, |(_, _, e)| e.clone())
        } else {
            Vec::new()
        };
        self.stamp = Some((identity, primary));
    }
    pub fn clear(&mut self) {
        self.extra.clear();
        self.scan = None;
        self.pending = 0;
        self.skip_primary = false;
    }
    pub fn remember(&mut self, identity: u64, primary: Selection) {
        self.history.retain(|(i, _, _)| *i != identity);
        self.history
            .push_back((identity, primary, self.extra.clone()));
        while self.history.len() > 128 {
            self.history.pop_front();
        }
        self.stamp = Some((identity, primary));
    }
    pub fn record_cursor(&mut self, identity: u64, primary: Selection) {
        let entry = (identity, primary, self.extra.clone());
        if self.cursor_history.back() != Some(&entry) {
            self.cursor_history.push_back(entry);
        }
        while self.cursor_history.len() > 128 {
            self.cursor_history.pop_front();
        }
    }
    pub fn install(
        &mut self,
        s: &mut AppService,
        id: DocumentId,
        all: Vec<Selection>,
        record: bool,
    ) {
        if all.is_empty() {
            return;
        }
        let d = s.active_document().unwrap();
        let identity = d.content_identity();
        let old = d.selection();
        let version = d.version;
        if record {
            self.record_cursor(identity, old);
        }
        let primary = *all.last().unwrap();
        let mut extra: Vec<Selection> = Vec::new();
        for item in all.into_iter().rev().skip(1) {
            let r = item.range();
            let overlap = |other: &Selection| {
                let q = other.range();
                item == *other || (r.start < q.end && r.end > q.start)
            };
            if !overlap(&primary) && !extra.iter().any(overlap) {
                extra.push(item);
            }
            if extra.len() >= MAX_SELECTIONS - 1 {
                s.status = "Limite de512 seleções simultâneas".into();
                break;
            }
        }
        extra.reverse();
        if super::input::dispatch(
            s,
            Command::SetSelection {
                id,
                expected_version: version,
                selection: primary,
            },
        ) {
            self.extra = extra;
            self.scan = None;
            self.pending = 0;
            self.skip_primary = false;
            self.remember(identity, primary);
        }
    }
    pub fn cursor_undo(&mut self, s: &mut AppService, id: DocumentId) {
        let identity = s.active_document().unwrap().content_identity();
        while let Some((stamp, primary, mut extra)) = self.cursor_history.pop_back() {
            if stamp == identity {
                extra.push(primary);
                self.install(s, id, extra, false);
                break;
            }
        }
    }
    pub fn toggle_caret(&mut self, s: &mut AppService, id: DocumentId, at: usize) {
        let primary = s.active_document().unwrap().selection();
        let mut all = self.all(primary);
        if let Some(index) = all
            .iter()
            .position(|selection| selection.is_caret() && selection.head == at)
        {
            if all.len() > 1 {
                all.remove(index);
            }
        } else {
            all.push(Selection::caret(at));
        }
        self.install(s, id, all, true);
    }
    pub fn request_all(&mut self, s: &mut AppService, id: DocumentId) {
        if s.active_document().unwrap().selection().is_caret() {
            self.request(s, id);
        }
        let d = s.active_document().unwrap();
        self.record_cursor(d.content_identity(), d.selection());
        self.pending = MAX_SELECTIONS - self.extra.len() - 1;
    }
    pub fn request_skip(&mut self, s: &mut AppService, id: DocumentId) {
        let d = s.active_document().unwrap();
        self.record_cursor(d.content_identity(), d.selection());
        self.request(s, id);
        self.skip_primary = true;
    }
    pub fn all(&self, primary: Selection) -> Vec<Selection> {
        let mut all = self.extra.clone();
        all.push(primary);
        all
    }
    pub fn request(&mut self, s: &mut AppService, id: DocumentId) {
        let d = s.active_document().unwrap();
        let primary = d.selection();
        self.record_cursor(d.content_identity(), primary);
        if primary.is_caret() {
            let r = codisseu_core::word_range(&d.snapshot(), primary.head);
            super::input::select(
                s,
                id,
                Selection {
                    anchor: r.start,
                    head: r.end,
                },
            );
            let d = s.active_document().unwrap();
            self.clear();
            self.remember(d.content_identity(), d.selection());
            return;
        }
        if self.extra.len() + 1 >= MAX_SELECTIONS {
            s.status = "Limite de 512 seleções simultâneas".into();
            return;
        }
        self.pending = (self.pending + 1).min(MAX_SELECTIONS - self.extra.len() - 1);
    }
    pub fn poll(&mut self, s: &mut AppService, id: DocumentId, ctx: &egui::Context) {
        if self.pending == 0 {
            return;
        }
        let d = s.active_document().unwrap();
        let snapshot = d.snapshot();
        let primary = d.selection();
        if self.scan.is_none() {
            let r = primary.range();
            if r.is_empty() {
                self.pending = 0;
                return;
            }
            if snapshot.slice(r.clone()).len_bytes() > 64 * 1024 {
                self.pending = 0;
                s.status = "Texto da seleção excede 64 KiB".into();
                return;
            }
            let needle: Vec<char> = snapshot.slice(r).chars().collect();
            let mut prefix = vec![0; needle.len()];
            let mut n = 0;
            for i in 1..needle.len() {
                while n > 0 && needle[i] != needle[n] {
                    n = prefix[n - 1];
                }
                if needle[i] == needle[n] {
                    n += 1;
                }
                prefix[i] = n;
            }
            self.scan = Some(Scan {
                version: d.version,
                needle,
                prefix,
                matched: 0,
                cursor: primary.range().end,
                pivot: primary.range().end,
                wrapped: false,
            });
        }
        let scan = self.scan.as_mut().unwrap();
        if scan.version != d.version {
            self.clear();
            return;
        }
        let started = Instant::now();
        let mut found = None;
        let mut done = false;
        for _ in 0..16384 {
            let limit = if scan.wrapped {
                scan.pivot
            } else {
                snapshot.len_chars()
            };
            if scan.cursor >= limit {
                if scan.wrapped {
                    done = true;
                    break;
                }
                scan.wrapped = true;
                scan.cursor = 0;
                scan.matched = 0;
                continue;
            }
            let c = snapshot.char(scan.cursor);
            scan.cursor += 1;
            while scan.matched > 0 && c != scan.needle[scan.matched] {
                scan.matched = scan.prefix[scan.matched - 1];
            }
            if c == scan.needle[scan.matched] {
                scan.matched += 1;
            }
            if scan.matched == scan.needle.len() {
                let r = scan.cursor - scan.matched..scan.cursor;
                scan.matched = scan.prefix[scan.matched - 1];
                let overlap = self.extra.iter().chain(std::iter::once(&primary)).any(|s| {
                    let e = s.range();
                    r.start < e.end && r.end > e.start
                });
                // Never select only a combining suffix or half a grapheme/CRLF.
                let boundary = |p| {
                    p == 0
                        || codisseu_core::next_grapheme(
                            &snapshot,
                            codisseu_core::previous_grapheme(&snapshot, p),
                        ) == p
                };
                if !overlap && boundary(r.start) && boundary(r.end) {
                    found = Some(r);
                    break;
                }
            }
            if started.elapsed() >= Duration::from_millis(2) {
                break;
            }
        }
        if let Some(r) = found {
            if !self.skip_primary {
                self.extra.push(primary);
            }
            self.skip_primary = false;
            super::input::select(
                s,
                id,
                Selection {
                    anchor: r.start,
                    head: r.end,
                },
            );
            let d = s.active_document().unwrap();
            self.remember(d.content_identity(), d.selection());
            self.pending -= 1;
            self.scan = None;
        } else if done {
            self.pending = 0;
            self.scan = None;
            s.status = "Todas as ocorrências já estão selecionadas".into();
        }
        if self.pending > 0 {
            ctx.request_repaint();
        }
    }
    pub fn replace(
        &mut self,
        s: &mut AppService,
        id: DocumentId,
        snapshot: &TextSnapshot,
        primary: Selection,
        insert: &str,
        deletion: Option<(bool, bool)>,
    ) -> bool {
        self.replace_many(
            s,
            id,
            snapshot,
            primary,
            &vec![insert.to_owned(); self.extra.len() + 1],
            deletion,
        )
    }
    pub fn paste(
        &mut self,
        s: &mut AppService,
        id: DocumentId,
        snapshot: &TextSnapshot,
        primary: Selection,
        text: &str,
    ) -> bool {
        let lines: Vec<_> = text
            .split('\n')
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect();
        let all = self.all(primary);
        if lines.len() != all.len() {
            return self.replace(s, id, snapshot, primary, text, None);
        }
        let mut indices: Vec<_> = (0..all.len()).collect();
        indices.sort_by_key(|&i| all[i].range().start);
        let mut inserts = vec![String::new(); all.len()];
        for (line, index) in lines.into_iter().zip(indices) {
            inserts[index] = line;
        }
        self.replace_many(s, id, snapshot, primary, &inserts, None)
    }
    pub fn insert_texts(
        &mut self,
        s: &mut AppService,
        id: DocumentId,
        snapshot: &TextSnapshot,
        primary: Selection,
        texts: &[String],
    ) -> bool {
        self.replace_many(s, id, snapshot, primary, texts, None)
    }
    fn replace_many(
        &mut self,
        s: &mut AppService,
        id: DocumentId,
        snapshot: &TextSnapshot,
        primary: Selection,
        inserts: &[String],
        deletion: Option<(bool, bool)>,
    ) -> bool {
        let mut selections = self.all(primary);
        let ranges: Vec<Range<usize>> = selections
            .iter()
            .map(|selection| {
                if !selection.is_caret() {
                    return selection.range();
                }
                let at = selection.head;
                match deletion {
                    Some((true, true)) => codisseu_core::previous_word(snapshot, at)..at,
                    Some((true, false)) => codisseu_core::previous_grapheme(snapshot, at)..at,
                    Some((false, true)) => at..codisseu_core::next_word(snapshot, at),
                    Some((false, false)) => at..codisseu_core::next_grapheme(snapshot, at),
                    None => at..at,
                }
            })
            .collect();
        let mut order: Vec<usize> = (0..ranges.len()).collect();
        order.sort_by_key(|&i| ranges[i].start);
        // Adjacent/overlapping deletion targets coalesce. A normal replacement
        // has disjoint selections by construction.
        let mut deltas: Vec<TextDelta> = Vec::new();
        for i in order {
            let r = ranges[i].clone();
            if let Some(last) = deltas.last_mut()
                && r.start <= last.range.end
                && deletion.is_some()
            {
                last.range.end = last.range.end.max(r.end);
            } else {
                deltas.push(TextDelta {
                    range: r,
                    insert: inserts[i].clone(),
                });
            }
        }
        for selection in &mut selections {
            let target = selection.range().end;
            let mut point = Selection::caret(target);
            for d in deltas.iter().rev() {
                point = point.mapped(d);
            }
            *selection = point;
        }
        let after = selections.pop().unwrap();
        let d = s.active_document().unwrap();
        let identity = d.content_identity();
        let version = d.version;
        self.remember(identity, primary);
        let ok = super::input::dispatch(
            s,
            Command::EditBatchTransaction {
                id,
                expected_version: version,
                deltas,
                before: primary,
                after,
            },
        );
        if ok {
            self.extra = selections;
            self.extra.sort_by_key(|s| s.head);
            self.extra.dedup();
            self.extra.retain(|s| *s != after);
            let d = s.active_document().unwrap();
            self.remember(d.content_identity(), after);
            self.scan = None;
            self.pending = 0;
        }
        ok
    }
}
