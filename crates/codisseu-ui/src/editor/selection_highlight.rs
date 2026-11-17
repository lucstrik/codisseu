//! Passive, viewport-local literal matches. Never installs editing selections.
use super::*;
use std::{collections::BTreeMap, ops::Range};

const MAX_QUERY: usize = 200;
const FRAME_CHARS: usize = 32_768;
const CACHE_WINDOWS: usize = 256;
const CACHE_RANGE_BYTES: usize = 512 * 1024;
const WINDOW_MATCHES: usize = 2048;
pub(super) const FRAME_MARKS: usize = 4096;

#[derive(Default)]
pub(super) struct SelectionHighlights {
    stamp: Option<(u64, Selection)>,
    needle: Vec<char>,
    prefix: Vec<usize>,
    windows: BTreeMap<(usize, usize), Window>,
    cache_bytes: usize,
    clock: u64,
    pub scanned: usize,
    pub pending: bool,
    pub limited: bool,
}

struct Window {
    used: u64,
    ranges: Vec<Range<usize>>,
    limited: bool,
}
impl Window {
    fn bytes(&self) -> usize {
        self.ranges.capacity() * std::mem::size_of::<Range<usize>>()
    }
}

fn equal(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}
fn boundary(snapshot: &TextSnapshot, at: usize) -> bool {
    at == 0
        || codisseu_core::next_grapheme(snapshot, codisseu_core::previous_grapheme(snapshot, at))
            == at
}

impl SelectionHighlights {
    pub fn begin(
        &mut self,
        identity: u64,
        snapshot: &TextSnapshot,
        selection: Selection,
        suppressed: bool,
    ) {
        self.scanned = 0;
        self.pending = false;
        self.limited = false;
        let stamp = (!suppressed).then_some((identity, selection));
        if self.stamp == stamp {
            return;
        }
        self.stamp = stamp;
        self.needle.clear();
        self.prefix.clear();
        self.windows.clear();
        self.cache_bytes = 0;
        let range = selection.range();
        if suppressed
            || range.is_empty()
            || range.len() > MAX_QUERY
            || range.end > snapshot.len_chars()
        {
            return;
        }
        let slice = snapshot.slice(range);
        if slice.chars().any(|c| matches!(c, '\r' | '\n')) || slice.chars().all(char::is_whitespace)
        {
            return;
        }
        self.needle.extend(slice.chars());
        self.prefix.resize(self.needle.len(), 0);
        let mut matched = 0;
        for i in 1..self.needle.len() {
            while matched > 0 && !equal(self.needle[i], self.needle[matched]) {
                matched = self.prefix[matched - 1];
            }
            if equal(self.needle[i], self.needle[matched]) {
                matched += 1;
            }
            self.prefix[i] = matched;
        }
    }
    pub fn query_len(&self) -> usize {
        self.needle.len()
    }

    /// Absolute scalar window already bounded by measured horizontal geometry.
    pub fn ranges(&mut self, snapshot: &TextSnapshot, window: Range<usize>) -> Vec<Range<usize>> {
        if self.needle.is_empty() || window.is_empty() {
            return Vec::new();
        }
        self.clock = self.clock.wrapping_add(1);
        let key = (window.start, window.end);
        if let Some(window) = self.windows.get_mut(&key) {
            window.used = self.clock;
            self.limited |= window.limited;
            return window.ranges.clone();
        }
        if window.len() > FRAME_CHARS {
            self.limited = true;
            return Vec::new();
        }
        if window.len() > FRAME_CHARS.saturating_sub(self.scanned) {
            self.pending = true;
            return Vec::new();
        }
        let mut ranges = Vec::new();
        let mut matched = 0;
        let mut limited = false;
        for (offset, c) in snapshot.slice(window.clone()).chars().enumerate() {
            self.scanned += 1;
            while matched > 0 && !equal(c, self.needle[matched]) {
                matched = self.prefix[matched - 1];
            }
            if equal(c, self.needle[matched]) {
                matched += 1;
            }
            if matched == self.needle.len() {
                let end = window.start + offset + 1;
                let start = end - self.needle.len();
                if boundary(snapshot, start) && boundary(snapshot, end) {
                    ranges.push(start..end);
                    if ranges.len() == WINDOW_MATCHES {
                        limited = true;
                        break;
                    }
                    matched = 0;
                    continue;
                }
                matched = self.prefix[matched - 1];
            }
        }
        self.limited |= limited;
        let cached = Window {
            used: self.clock,
            ranges: ranges.clone(),
            limited,
        };
        while self.windows.len() >= CACHE_WINDOWS
            || self.cache_bytes + cached.bytes() > CACHE_RANGE_BYTES
        {
            if let Some(oldest) = self
                .windows
                .iter()
                .min_by_key(|(_, window)| window.used)
                .map(|(key, _)| *key)
            {
                self.cache_bytes -= self
                    .windows
                    .remove(&oldest)
                    .expect("existing window")
                    .bytes();
            }
        }
        self.cache_bytes += cached.bytes();
        self.windows.insert(key, cached);
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(text: &str, selection: Range<usize>) -> Vec<Range<usize>> {
        let snapshot = TextSnapshot::from_str(text);
        let mut highlights = SelectionHighlights::default();
        highlights.begin(
            1,
            &snapshot,
            Selection {
                anchor: selection.start,
                head: selection.end,
            },
            false,
        );
        let ranges = highlights.ranges(&snapshot, 0..snapshot.len_chars());
        assert_eq!(snapshot.to_string(), text);
        ranges
    }

    #[test]
    fn literal_substrings_ignore_unicode_case_without_expansion_or_normalization() {
        assert_eq!(
            matches("selection selectionDatabase Selection SELECTION", 0..9),
            vec![0..9, 10..19, 28..37, 38..47]
        );
        assert_eq!(matches("ação AÇÃO açãO", 0..4), vec![0..4, 5..9, 10..14]);
        assert_eq!(matches("ß ẞ SS ss", 0..1), vec![0..1, 2..3]);
        assert_eq!(matches("é e\u{301} É", 0..1), vec![0..1, 5..6]);
        assert_eq!(matches("aa aaaa", 0..2), vec![0..2, 3..5, 5..7]);
        // A combining suffix, half CRLF or a ZWJ component is never a painted match.
        assert_eq!(matches("e e\u{301} E", 0..1), vec![0..1, 5..6]);
        assert_eq!(matches("👩 👩‍💻", 0..1), vec![0..1]);
    }

    #[test]
    fn selection_content_identity_and_preedit_invalidate_cached_ranges() {
        let snapshot = TextSnapshot::from_str("word WORD next NEXT\r\nword");
        let mut highlights = SelectionHighlights::default();
        let word = Selection { anchor: 4, head: 0 };
        highlights.begin(10, &snapshot, word, false);
        assert_eq!(highlights.ranges(&snapshot, 0..19), vec![0..4, 5..9]);
        highlights.begin(10, &snapshot, word, false);
        assert_eq!(highlights.ranges(&snapshot, 0..19), vec![0..4, 5..9]);
        assert_eq!(
            highlights.scanned, 0,
            "idle view must not rescan cached source"
        );
        highlights.begin(10, &snapshot, word, true);
        assert!(highlights.ranges(&snapshot, 0..19).is_empty());
        highlights.begin(
            10,
            &snapshot,
            Selection {
                anchor: 10,
                head: 14,
            },
            false,
        );
        assert_eq!(highlights.ranges(&snapshot, 0..19), vec![10..14, 15..19]);
        let edited = TextSnapshot::from_str("word WORD gone GONE\r\nword");
        highlights.begin(
            11,
            &edited,
            Selection {
                anchor: 10,
                head: 14,
            },
            false,
        );
        assert_eq!(highlights.ranges(&edited, 0..19), vec![10..14, 15..19]);
        assert!(
            highlights.scanned > 0,
            "content update cannot reuse stale window results"
        );
        for selection in [
            Selection::caret(0),
            Selection {
                anchor: 0,
                head: 21,
            },
            Selection { anchor: 4, head: 5 },
        ] {
            highlights.begin(11, &edited, selection, false);
            assert!(highlights.ranges(&edited, 0..19).is_empty());
        }
        let large = TextSnapshot::from_str(&"x".repeat(201));
        highlights.begin(
            12,
            &large,
            Selection {
                anchor: 0,
                head: 201,
            },
            false,
        );
        assert_eq!(highlights.query_len(), 0);
    }

    #[test]
    fn multi_megabyte_source_scans_only_requested_windows_and_resumes_with_bounded_cache() {
        let source = format!("needle{}needle", "x".repeat(2 * 1024 * 1024));
        let snapshot = TextSnapshot::from_str(&source);
        let selection = Selection { anchor: 0, head: 6 };
        let mut highlights = SelectionHighlights::default();
        highlights.begin(1, &snapshot, selection, false);
        assert_eq!(highlights.ranges(&snapshot, 0..20_000), vec![0..6]);
        assert!(highlights.ranges(&snapshot, 20_000..40_000).is_empty());
        assert!(highlights.pending);
        assert_eq!(highlights.scanned, 20_000);
        highlights.begin(1, &snapshot, selection, false);
        assert_eq!(highlights.ranges(&snapshot, 0..20_000), vec![0..6]);
        assert!(highlights.ranges(&snapshot, 20_000..40_000).is_empty());
        assert!(!highlights.pending);
        assert_eq!(highlights.scanned, 20_000);
        for start in 0..300 {
            highlights.begin(1, &snapshot, selection, false);
            highlights.ranges(&snapshot, start..start + 16);
            assert!(highlights.windows.len() <= CACHE_WINDOWS);
        }
        assert_eq!(snapshot.len_bytes(), source.len());
    }

    #[test]
    fn dense_occurrences_evict_cached_payloads_before_exceeding_memory_budget() {
        let snapshot = TextSnapshot::from_str(&"a".repeat(5000));
        let mut highlights = SelectionHighlights::default();
        for start in 0..64 {
            highlights.begin(1, &snapshot, Selection { anchor: 0, head: 1 }, false);
            assert_eq!(
                highlights.ranges(&snapshot, start..start + 2048).len(),
                2048
            );
            assert!(highlights.cache_bytes <= CACHE_RANGE_BYTES);
        }
        assert!(
            highlights.windows.len() < 64,
            "payload cap must evict windows even below count cap"
        );
    }
}
