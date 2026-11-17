//! Literal, case-sensitive search for the first M3 slice, not semantic rename.
//! All public offsets are Unicode scalar indices, never UTF-8 bytes or graphemes.
use crate::MAX_EDITOR_DOCUMENT_BYTES as MAX_DOCUMENT_BYTES;
use std::ops::Range;

pub const MAX_SEARCH_MATCHES: usize = 10_000;
pub const MAX_QUERY_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct LiteralMatches {
    pub ranges: Vec<Range<usize>>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct Replacement {
    pub text: String,
    pub count: usize,
}

pub fn validate_query(query: &str) -> Result<(), String> {
    if query.len() > MAX_QUERY_BYTES {
        Err("Busca excede o limite de 64 KiB".into())
    } else if query.contains('\0') {
        Err("Busca não aceita NUL".into())
    } else {
        Ok(())
    }
}

/// Searches left-to-right, without overlap or Unicode normalization.
pub fn find_literal(
    text: &str,
    query: &str,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<LiteralMatches, String> {
    crate::validate(text)?;
    validate_query(query)?;
    check()?;
    let mut found = LiteralMatches {
        ranges: Vec::new(),
        truncated: false,
    };
    if query.is_empty() {
        return Ok(found);
    }
    let query_chars = query.chars().count();
    let (mut byte_cursor, mut scalar_cursor) = (0, 0);
    scan(text, query, &mut check, |range| {
        if found.ranges.len() == MAX_SEARCH_MATCHES {
            found.truncated = true;
            return Ok(false);
        }
        scalar_cursor += text[byte_cursor..range.start].chars().count();
        found
            .ranges
            .push(scalar_cursor..scalar_cursor + query_chars);
        scalar_cursor += query_chars;
        byte_cursor = range.end;
        Ok(true)
    })?;
    Ok(found)
}

/// Computes every replacement independently from the capped display results.
/// The caller applies the returned text once, after rechecking the document version.
pub fn replace_literal(
    text: &str,
    query: &str,
    replacement: &str,
    selection: Option<Range<usize>>,
    mut check: impl FnMut() -> Result<(), String>,
) -> Result<Replacement, String> {
    crate::validate(text)?;
    validate_query(query)?;
    crate::validate(replacement)?;
    check()?;
    if query.is_empty() {
        return Err("Informe um texto de busca antes de substituir".into());
    }
    let selected = if let Some(range) = selection {
        if range.start > range.end {
            return Err("Intervalo de substituição inválido".into());
        }
        let mut start = None;
        let mut end = None;
        for (index, (byte, _)) in text
            .char_indices()
            .chain(std::iter::once((text.len(), '\0')))
            .enumerate()
        {
            if index % 1024 == 0 {
                check()?;
            }
            if index == range.start {
                start = Some(byte);
            }
            if index == range.end {
                end = Some(byte);
                break;
            }
        }
        let bytes = start
            .zip(end)
            .map(|(start, end)| start..end)
            .ok_or("Intervalo de substituição fora do documento")?;
        if &text[bytes.clone()] != query {
            return Err("A seleção não coincide com o texto de busca".into());
        }
        Some(bytes)
    } else {
        None
    };
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    let mut count = 0;
    scan(text, query, &mut check, |range| {
        if selected.as_ref().is_some_and(|selected| *selected != range) {
            return Ok(true);
        }
        append(&mut output, &text[cursor..range.start])?;
        append(&mut output, replacement)?;
        cursor = range.end;
        count += 1;
        Ok(selected.is_none())
    })?;
    if selected.is_some() && count == 0 {
        return Err("A seleção não é uma ocorrência completa da busca sem sobreposição".into());
    }
    check()?;
    append(&mut output, &text[cursor..])?;
    Ok(Replacement {
        text: output,
        count,
    })
}

fn append(output: &mut String, text: &str) -> Result<(), String> {
    if text.len() > MAX_DOCUMENT_BYTES - output.len() {
        return Err("Substituição excederia o limite de 16 MiB do editor".into());
    }
    output.push_str(text);
    Ok(())
}

// KMP gives bounded linear work even for repeated prefixes. Checking every 1024
// bytes includes long no-match scans and pattern preparation, not just matches.
fn scan(
    text: &str,
    query: &str,
    check: &mut impl FnMut() -> Result<(), String>,
    mut visit: impl FnMut(Range<usize>) -> Result<bool, String>,
) -> Result<(), String> {
    let needle = query.as_bytes();
    let mut prefix = vec![0; needle.len()];
    let mut matched = 0;
    for index in 1..needle.len() {
        if index % 1024 == 0 {
            check()?;
        }
        while matched > 0 && needle[index] != needle[matched] {
            matched = prefix[matched - 1];
        }
        if needle[index] == needle[matched] {
            matched += 1;
        }
        prefix[index] = matched;
    }
    matched = 0;
    for (index, byte) in text.bytes().enumerate() {
        if index % 1024 == 0 {
            check()?;
        }
        while matched > 0 && byte != needle[matched] {
            matched = prefix[matched - 1];
        }
        if byte == needle[matched] {
            matched += 1;
        }
        if matched == needle.len() {
            // A valid UTF-8 needle cannot start at a continuation byte.
            if !visit(index + 1 - matched..index + 1)? {
                return check();
            }
            matched = 0; // non-overlapping occurrences
        }
    }
    check()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_crlf_combining_and_literal_metacharacters() {
        let text = "😀e\u{301}\r\n[.*] ação [.*]";
        assert_eq!(
            find_literal(text, "[.*]", || Ok(())).unwrap().ranges,
            vec![5..9, 15..19]
        );
        assert_eq!(find_literal(text, "é", || Ok(())).unwrap().ranges, vec![]);
        assert_eq!(
            find_literal(text, "e\u{301}", || Ok(())).unwrap().ranges,
            vec![1..3]
        );
        let replaced = replace_literal(text, "[.*]", "💡", Some(15..19), || Ok(())).unwrap();
        assert_eq!(replaced.text, "😀e\u{301}\r\n[.*] ação 💡");
        assert_eq!(replaced.count, 1);
        assert!(replace_literal(text, "[.*]", "x", Some(6..9), || Ok(())).is_err());
    }

    #[test]
    fn non_overlapping_exact_limit_and_replace_all_ignores_result_cap() {
        assert_eq!(
            find_literal("aaaaa", "aa", || Ok(())).unwrap().ranges,
            vec![0..2, 2..4]
        );
        assert!(replace_literal("aaaaa", "aa", "b", Some(1..3), || Ok(())).is_err());
        let exact = find_literal(&"x".repeat(MAX_SEARCH_MATCHES), "x", || Ok(())).unwrap();
        assert!(!exact.truncated);
        let text = "x".repeat(MAX_SEARCH_MATCHES + 1);
        let found = find_literal(&text, "x", || Ok(())).unwrap();
        assert!(found.truncated);
        assert_eq!(found.ranges.len(), MAX_SEARCH_MATCHES);
        let replaced = replace_literal(&text, "x", "😀", None, || Ok(())).unwrap();
        assert_eq!(replaced.count, MAX_SEARCH_MATCHES + 1);
        assert_eq!(replaced.text, "😀".repeat(MAX_SEARCH_MATCHES + 1));
    }

    #[test]
    fn generated_unicode_cases_match_independent_standard_library_model() {
        let alphabet = ["a", "b", "😀", "é", "\u{301}", "\r", "\n"];
        let mut seed = 17_u64;
        let mut choose = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            alphabet[(seed >> 32) as usize % alphabet.len()]
        };
        for _ in 0..400 {
            let text: String = (0..32).map(|_| choose()).collect();
            let query: String = (0..3).map(|_| choose()).collect();
            let expected: Vec<_> = text
                .match_indices(&query)
                .map(|(byte, matched)| {
                    let start = text[..byte].chars().count();
                    start..start + matched.chars().count()
                })
                .collect();
            assert_eq!(
                find_literal(&text, &query, || Ok(())).unwrap().ranges,
                expected
            );
            assert_eq!(
                replace_literal(&text, &query, "🌩️", None, || Ok(()))
                    .unwrap()
                    .text,
                text.replace(&query, "🌩️")
            );
        }
    }

    #[test]
    fn cancellation_limits_empty_and_noop() {
        let mut checks = 0;
        let cancelled = find_literal(&"a".repeat(MAX_DOCUMENT_BYTES), "z", || {
            checks += 1;
            if checks == 4 {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        });
        assert_eq!(cancelled.unwrap_err(), "cancelled");
        assert_eq!(checks, 4);
        assert!(
            find_literal("abc", "", || Ok(()))
                .unwrap()
                .ranges
                .is_empty()
        );
        assert!(replace_literal("abc", "", "x", None, || Ok(())).is_err());
        assert!(
            replace_literal("x", "x", &"a".repeat(MAX_DOCUMENT_BYTES + 1), None, || Ok(
                ()
            ))
            .is_err()
        );
        assert!(
            replace_literal("xx", "x", &"a".repeat(MAX_DOCUMENT_BYTES), None, || Ok(())).is_err()
        );
        assert_eq!(
            replace_literal("abc", "x", "z", None, || Ok(()))
                .unwrap()
                .text,
            "abc"
        );
        assert!(find_literal("abc", &"x".repeat(MAX_QUERY_BYTES + 1), || Ok(())).is_err());
    }
}
