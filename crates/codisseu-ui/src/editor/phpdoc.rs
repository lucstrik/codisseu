//! Explicit, bounded PHPDoc edits. No workspace lookup or per-frame parsing.
use super::*;
use codisseu_core::{EditKind, TextDelta};

const CONTEXT_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Context {
    Code,
    Doc,
    Other,
}

/// PHP interpolation can contain quoted array keys and nested expressions.
/// Return None for unfinished/unknown constructs; never expose their contents as code.
fn quoted_end(b: &[u8], start: usize, depth: usize) -> Option<usize> {
    if depth > 32 {
        return None;
    }
    let quote = b[start];
    let mut i = start + 1;
    while i < b.len() {
        if b[i] == quote {
            return Some(i + 1);
        }
        if b[i] == b'\\' {
            i += 2;
        } else if quote != b'\'' && b[i..].starts_with(b"{$") {
            i = interpolation_end(b, i + 1, depth + 1)?;
        } else if quote != b'\'' && b[i..].starts_with(b"${") {
            i = interpolation_end(b, i + 2, depth + 1)?;
        } else {
            i += 1;
        }
    }
    None
}

fn interpolation_end(b: &[u8], mut i: usize, depth: usize) -> Option<usize> {
    if depth > 32 {
        return None;
    }
    let mut braces = 1usize;
    while i < b.len() {
        if matches!(b[i], b'\'' | b'"' | b'`') {
            i = quoted_end(b, i, depth + 1)?;
        } else if b[i..].starts_with(b"<<<") || b[i..].starts_with(b"?>") {
            return None;
        } else if b[i..].starts_with(b"/*") {
            i += b[i + 2..].windows(2).position(|w| w == b"*/")? + 4;
        } else if b[i..].starts_with(b"//") || b[i] == b'#' {
            i += b[i..].iter().position(|c| *c == b'\n')?;
        } else {
            if b[i] == b'{' {
                braces += 1;
            }
            if b[i] == b'}' {
                braces -= 1;
                if braces == 0 {
                    return Some(i + 1);
                }
            }
            i += 1;
        }
    }
    None
}

/// Conservative PHP island scanner. Unknown heredocs disable assistance.
fn context(prefix: &str) -> Context {
    let b = prefix.as_bytes();
    let mut php = false;
    let mut i = 0;
    while i < b.len() {
        let tail = &b[i..];
        if !php {
            let open = if tail
                .get(..5)
                .is_some_and(|t| t.eq_ignore_ascii_case(b"<?php"))
                && tail.get(5).is_none_or(u8::is_ascii_whitespace)
            {
                5
            } else if tail.starts_with(b"<?=") {
                3
            } else {
                0
            };
            if open > 0 {
                php = true;
                i += open;
            } else {
                i += 1;
            }
        } else if tail.starts_with(b"?>") {
            php = false;
            i += 2;
        } else if tail.starts_with(b"<<<") {
            return Context::Other;
        } else if tail.starts_with(b"/*") {
            let doc = tail.starts_with(b"/**");
            let Some(end) = tail[2..].windows(2).position(|w| w == b"*/") else {
                return if doc { Context::Doc } else { Context::Other };
            };
            i += end + 4;
        } else if tail.starts_with(b"//") || (b[i] == b'#' && !tail.starts_with(b"#[")) {
            while i < b.len() && b[i] != b'\n' && !b[i..].starts_with(b"?>") {
                i += 1;
            }
            if i == b.len() {
                return Context::Other;
            }
        } else if matches!(b[i], b'\'' | b'"' | b'`') {
            let Some(end) = quoted_end(b, i, 0) else {
                return Context::Other;
            };
            i = end;
        } else {
            i += 1;
        }
    }
    if php { Context::Code } else { Context::Other }
}

fn plan(s: &TextSnapshot, caret: usize, crlf: bool) -> Option<(TextDelta, Selection)> {
    if s.char_to_byte(caret) > CONTEXT_BYTES {
        return None;
    }
    let line = s.char_to_line(caret);
    let start = s.line_to_char(line);
    let end = content_end(s, line);
    if caret > end {
        return None;
    }
    let before = s.slice(start..caret).to_string();
    let indent: String = before
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect();
    let trimmed = before.trim_start_matches([' ', '\t']);
    let eol = if crlf { "\r\n" } else { "\n" };
    if trimmed.trim_end_matches([' ', '\t']) == "/**" {
        let opener = start + indent.chars().count();
        if context(&s.slice(..opener).to_string()) != Context::Code {
            return None;
        }
        // Don't swallow any code or description after the caret.
        if end - caret > 1024 {
            return None;
        }
        let tail = s.slice(caret..end).to_string();
        if !matches!(tail.trim(), "" | "*/") {
            return None;
        }
        let mut following = String::new();
        for c in s.slice(end..).chars() {
            if following.len() + c.len_utf8() > codisseu_core::phpdoc::MAX_BYTES {
                break;
            }
            following.push(c);
        }
        // Respect an existing block below an opener being edited.
        if following.trim_start().starts_with('*') || following.trim_start().starts_with("/**") {
            return None;
        }
        let mut insert = format!("{eol}{indent} * ");
        let after = Selection::caret(caret + insert.chars().count());
        for tag in codisseu_core::phpdoc::function_tags(&following).unwrap_or_default() {
            insert.push_str(&format!("{eol}{indent} * {tag}"));
        }
        insert.push_str(&format!("{eol}{indent} */"));
        Some((
            TextDelta {
                range: caret..end,
                insert,
            },
            after,
        ))
    } else if trimmed.starts_with('*')
        && !trimmed.starts_with("*/")
        && context(&s.slice(..caret).to_string()) == Context::Doc
    {
        let insert = format!("{eol}{indent}* ");
        let after = Selection::caret(caret + insert.chars().count());
        Some((
            TextDelta {
                range: caret..caret,
                insert,
            },
            after,
        ))
    } else {
        None
    }
}

pub(super) fn enter(
    service: &mut AppService,
    id: DocumentId,
    view: &mut EditorView,
    ext: &str,
) -> bool {
    if !matches!(ext, "php" | "phtml") || !view.multi.extra.is_empty() || view.composition.is_some()
    {
        return false;
    }
    let Some(d) = service.active_document() else {
        return false;
    };
    let before = d.selection();
    if !before.is_caret() {
        return false;
    }
    let Some((delta, after)) = plan(&d.snapshot(), before.head, view.crlf) else {
        return false;
    };
    let version = d.version;
    input::dispatch(
        service,
        Command::EditTransaction {
            id,
            expected_version: version,
            delta,
            kind: EditKind::Atomic,
            before,
            after,
        },
    );
    view.preferred_x = None;
    view.reveal = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_ignores_strings_comments_html_and_unknown_heredocs() {
        for prefix in [
            "",
            "<html>",
            "<?php /* unfinished",
            "<?php // unfinished",
            "<?php # comment",
            "<?php $s = '\n",
            "<?php $s = \"\n",
            "<?php $s = \"{$a[\"\n",
            "<?php <<<TXT\n",
            "<?php ?>\n",
        ] {
            assert_eq!(context(prefix), Context::Other, "{prefix}");
        }
        for prefix in [
            "<?php\n",
            "<?PHP\n",
            "html <?php // text\n",
            "<?php /** closed */\n",
            "<?php $s = '?>';\n",
            "<?php $s = \"{$a[\"key\"]}\";\n",
            "<?php $s = \"${a['key']}\";\n",
            "<?php #[Attribute]\n",
            "<?=\n",
            "<?php // ?>html<?php\n",
        ] {
            assert_eq!(context(prefix), Context::Code, "{prefix}");
        }
    }
    #[test]
    fn preserves_crlf_tabs_unicode_and_existing_inline_closer() {
        let source =
            "<?php\r\n// ação 😀\r\n\t/** */\r\n\tfunction nome(?string $nome): string {}\r\n";
        let caret = source[..source.find("/**").unwrap() + 3].chars().count();
        let (delta, after) = plan(&TextSnapshot::from_str(source), caret, true).unwrap();
        assert_eq!(
            delta.insert,
            "\r\n\t * \r\n\t * @param string|null $nome\r\n\t * @return string\r\n\t */"
        );
        assert_eq!(delta.range, caret..caret + 3);
        assert_eq!(after.head, caret + 6);
    }
    #[test]
    fn does_not_expand_ordinary_comments_existing_docs_or_large_context() {
        for (source, marker) in [
            ("<?php\n/*\nfunction f() {}", "/*"),
            ("<?php\n/** text\n", "text"),
            ("<?php\n/**\n * Existing\n */\nfunction f() {}", "/**"),
            ("<?php\n/**\n/** Existing */\nfunction f() {}", "/**"),
            ("<?php\n/** code\n", "/**"),
            ("<?php\n$x = '\n/**\n';", "/**"),
        ] {
            let caret = source.find(marker).unwrap() + marker.len();
            assert!(
                plan(&TextSnapshot::from_str(source), caret, false).is_none(),
                "{source}"
            );
        }
        let source = format!("<?php\n{}\n/**", " ".repeat(CONTEXT_BYTES));
        assert!(plan(&TextSnapshot::from_str(&source), source.len(), false).is_none());
    }
    #[test]
    fn continues_doc_lines_and_scaffolds_without_declaration() {
        let source = "<?php\n/**\n * descrição";
        let caret = source.chars().count();
        let (delta, _) = plan(&TextSnapshot::from_str(source), caret, false).unwrap();
        assert_eq!(delta.insert, "\n * ");
        let source = "<?php\n/**";
        let (delta, _) = plan(&TextSnapshot::from_str(source), source.len(), false).unwrap();
        assert_eq!(delta.insert, "\n * \n */");
    }
}
