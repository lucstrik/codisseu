//! Insertion-only projection. All source characters must survive in their order.
//! Independent implementation; no proprietary editor or model code is used.
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineProjection {
    /// Projected tail, excluding a common suffix that stays in the source layout.
    pub text: String,
    /// Byte ranges in `text` copied from the original source (normal, not ghost).
    pub preserved: Vec<Range<usize>>,
    /// Number of source scalars represented by these preserved ranges.
    pub consumed_source: usize,
}
impl InlineProjection {
    pub fn source_prefix(&self, target_scalars: usize) -> usize {
        let byte = self
            .text
            .char_indices()
            .nth(target_scalars)
            .map_or(self.text.len(), |(b, _)| b);
        self.preserved
            .iter()
            .map(|r| {
                self.text[r.start..r.end.min(byte).max(r.start)]
                    .chars()
                    .count()
            })
            .sum()
    }
    pub fn first_insertion(&self) -> usize {
        self.preserved
            .first()
            .filter(|r| r.start == 0)
            .map_or(0, |r| self.text[..r.end].chars().count())
    }
}

pub(super) fn project(source: &str, target: &str) -> Option<InlineProjection> {
    // Existing source is one bounded line; inserted text can span lines.
    if source.contains(['\r', '\n'])
        || source.encode_utf16().count() > 5000
        || target.len() > super::MAX_SUGGESTION
    {
        return None;
    }
    let tail = source
        .chars()
        .rev()
        .zip(target.chars().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let source_len = source.chars().count();
    let source_end = source
        .char_indices()
        .nth(source_len - tail)
        .map_or(source.len(), |(b, _)| b);
    let target_end = target
        .char_indices()
        .nth(target.chars().count() - tail)
        .map_or(target.len(), |(b, _)| b);
    let text = &target[..target_end];
    if text.is_empty() {
        return None;
    }
    let mut source_chars = source[..source_end].chars().peekable();
    let mut preserved: Vec<Range<usize>> = Vec::new();
    for (byte, ch) in text.char_indices() {
        if source_chars.peek() == Some(&ch) {
            source_chars.next();
            if let Some(last) = preserved.last_mut().filter(|r| r.end == byte) {
                last.end = byte + ch.len_utf8();
            } else {
                preserved.push(byte..byte + ch.len_utf8());
            }
        }
    }
    if source_chars.next().is_some() || preserved.len() > 128 {
        return None;
    }
    let consumed_source = source_len - tail;
    if text.chars().count() <= consumed_source {
        return None;
    }
    Some(InlineProjection {
        text: text.to_owned(),
        preserved,
        consumed_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_preserved_spans_and_inserted_newlines_are_insertion_only() {
        for (source, target, kept) in [
            ("$a,$b)", "$a, 1+$b);", vec!["$a,", "$b)"]),
            ("a,b)", "a,\n    new+b);", vec!["a,", "b)"]),
            ("😀x)", "😀+x);", vec!["😀", "x)"]),
        ] {
            let p = project(source, target).unwrap();
            assert_eq!(
                p.preserved
                    .iter()
                    .map(|r| &p.text[r.clone()])
                    .collect::<Vec<_>>(),
                kept
            );
            assert_eq!(p.consumed_source, source.chars().count());
            assert_eq!(
                p.preserved
                    .iter()
                    .map(|r| &p.text[r.clone()])
                    .collect::<String>(),
                source
            );
        }
        assert!(project("old)", "new);").is_none());
        assert!(project("a\nb", "a+x\nb").is_none());
        assert!(project("abc", "abc").is_none());
        let p = project("abc)", "a+bc)").unwrap();
        assert_eq!(p.text, "a+");
        assert_eq!(p.consumed_source, 1);
        assert_eq!(p.source_prefix(1), 1);
        assert_eq!(p.source_prefix(2), 1);
    }
}
