//! Unicode navigation over copy-on-write rope snapshots. No full-buffer String.
use crate::TextSnapshot;
use std::ops::Range;
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete, UnicodeSegmentation};

/// Previous extended grapheme boundary in scalar indices. CRLF is one cluster.
pub fn previous_grapheme(text: &TextSnapshot, index: usize) -> usize {
    grapheme(text, index, false)
}
/// Next extended grapheme boundary; handles ZWJ, combining marks and flag pairs.
pub fn next_grapheme(text: &TextSnapshot, index: usize) -> usize {
    grapheme(text, index, true)
}
fn grapheme(text: &TextSnapshot, index: usize, forward: bool) -> usize {
    let index = index.min(text.len_chars());
    if (forward && index == text.len_chars()) || (!forward && index == 0) {
        return index;
    }
    let byte = text.char_to_byte(index);
    let mut cursor = GraphemeCursor::new(byte, text.len_bytes(), true);
    let (mut chunk, mut start, _, _) = text.chunk_at_byte(byte);
    loop {
        let result = if forward {
            cursor.next_boundary(chunk, start)
        } else {
            cursor.prev_boundary(chunk, start)
        };
        match result {
            Ok(Some(boundary)) => return text.byte_to_char(boundary),
            Ok(None) => return if forward { text.len_chars() } else { 0 },
            Err(GraphemeIncomplete::PreContext(end)) => {
                let (context, context_start, _, _) = text.chunk_at_byte(end - 1);
                cursor.provide_context(&context[..end - context_start], context_start);
            }
            Err(GraphemeIncomplete::PrevChunk) => {
                (chunk, start, _, _) = text.chunk_at_byte(start - 1);
            }
            Err(GraphemeIncomplete::NextChunk) => {
                (chunk, start, _, _) = text.chunk_at_byte(start + chunk.len());
            }
            Err(GraphemeIncomplete::InvalidOffset) => {
                unreachable!("rope chunks and cursor offsets share UTF-8 boundaries")
            }
        }
    }
}

/// Ctrl-left policy: start of the preceding/current UAX29 word, skipping separators.
/// Only the nearest lexical run is segmented, so a huge minified line does not
/// become a full String. A single giant ASCII identifier needs no String at all.
pub fn previous_word(text: &TextSnapshot, index: usize) -> usize {
    let mut end = index.min(text.len_chars());
    while end > 0 {
        let mut chars = text.chars_at(end);
        while let Some(c) = chars.prev() {
            if separator(c) {
                end -= 1;
            } else {
                break;
            }
        }
        if end == 0 {
            return 0;
        }
        let start = run_start(text, end);
        if simple_identifier(text, start..end) {
            return start;
        }
        let content = text.slice(start..end).to_string();
        if let Some(range) = word_spans(&content, start).last() {
            return range.start;
        }
        end = start;
    }
    0
}
/// Ctrl-right policy: end of the current/next UAX29 word, skipping separators.
pub fn next_word(text: &TextSnapshot, index: usize) -> usize {
    let mut start = index.min(text.len_chars());
    while start < text.len_chars() {
        let mut chars = text.chars_at(start);
        for c in &mut chars {
            if separator(c) {
                start += 1;
            } else {
                break;
            }
        }
        if start == text.len_chars() {
            return start;
        }
        let end = run_end(text, start);
        if simple_identifier(text, start..end) {
            return end;
        }
        // Include the run's prefix: UAX29 may distinguish e.g. a numeric colon
        // from a letter colon. Cutting at the caret would lose that context.
        let context_start = run_start(text, start);
        let content = text.slice(context_start..end).to_string();
        if let Some(range) = word_spans(&content, context_start).find(|r| r.end > start) {
            return range.end;
        }
        start = end;
    }
    text.len_chars()
}
/// Double-click selects a UAX29 word, otherwise one whole extended grapheme.
pub fn word_range(text: &TextSnapshot, index: usize) -> Range<usize> {
    let index = index.min(text.len_chars());
    if index == text.len_chars() {
        return index..index;
    }
    if !separator(text.char(index)) {
        let start = run_start(text, index);
        let end = run_end(text, index);
        if simple_identifier(text, start..end) {
            return start..end;
        }
        let content = text.slice(start..end).to_string();
        for range in word_spans(&content, start) {
            if range.contains(&index) {
                return range;
            }
        }
    }
    let boundary = previous_grapheme(text, index + 1);
    boundary..next_grapheme(text, boundary)
}
fn separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ';' | '/'
                | '\\'
                | '='
                | '+'
                | '*'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '!'
                | '?'
                | '@'
                | '#'
                | '%'
                | '^'
                | '~'
                | '`'
                | '-'
                | '&'
                | '|'
        )
}
fn run_start(text: &TextSnapshot, end: usize) -> usize {
    let mut start = end;
    let mut chars = text.chars_at(end);
    while let Some(c) = chars.prev() {
        if separator(c) {
            break;
        }
        start -= 1;
    }
    start
}
fn run_end(text: &TextSnapshot, start: usize) -> usize {
    let mut end = start;
    for c in text.chars_at(start) {
        if separator(c) {
            break;
        }
        end += 1;
    }
    end
}
fn simple_identifier(text: &TextSnapshot, range: Range<usize>) -> bool {
    let slice = text.slice(range);
    slice.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn word_spans(content: &str, start: usize) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut byte_cursor = 0;
    let mut scalar_cursor = start;
    content.unicode_word_indices().map(move |(byte, word)| {
        scalar_cursor += content[byte_cursor..byte].chars().count();
        let range = scalar_cursor..scalar_cursor + word.chars().count();
        byte_cursor = byte + word.len();
        scalar_cursor = range.end;
        range
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphemes_cross_rope_chunks_and_match_unicode_reference_both_directions() {
        for prefix in [0, 985, 1023, 2048] {
            let source = format!("{}e\u{301}👩🏽‍💻🇧🇷\r\nक्‍षz", "a".repeat(prefix));
            let rope = TextSnapshot::from_str(&source);
            let mut boundaries: Vec<usize> = source
                .grapheme_indices(true)
                .map(|(byte, _)| source[..byte].chars().count())
                .collect();
            boundaries.push(rope.len_chars());
            for pair in boundaries.windows(2) {
                assert_eq!(next_grapheme(&rope, pair[0]), pair[1], "{prefix} {pair:?}");
                assert_eq!(
                    previous_grapheme(&rope, pair[1]),
                    pair[0],
                    "{prefix} {pair:?}"
                );
                for inside in pair[0] + 1..pair[1] {
                    assert_eq!(next_grapheme(&rope, inside), pair[1]);
                    assert_eq!(previous_grapheme(&rope, inside), pair[0]);
                }
            }
        }
    }
    #[test]
    fn unicode_words_and_double_click_preserve_scalar_offsets() {
        let rope = TextSnapshot::from_str("ação e\u{301}lan 👩‍💻\r\n日本語 fim");
        assert_eq!(word_range(&rope, 2), 0..4);
        assert_eq!(word_range(&rope, 7), 5..10);
        assert_eq!(word_range(&rope, 12), 11..14);
        assert_eq!(previous_word(&rope, 10), 5);
        assert_eq!(next_word(&rope, 4), 10);
        assert_eq!(previous_word(&rope, 16), 5); // emoji separator -> previous word on the line
    }
}
