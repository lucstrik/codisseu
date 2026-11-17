//! Bounded lexical editor assistance. Transactions still use the shared application service.
//! No AST/semantic claim: unknown states, regex ambiguity and heredocs stay literal.
use super::*;
use codisseu_core::{EditKind, TextChange, TextDelta};
const BUDGET: usize = 16_384;
#[derive(Default)]
pub(super) struct Pairs {
    revision: Option<u64>,
    entries: Vec<(usize, usize, char, char)>,
}
impl Pairs {
    pub fn sync(&mut self, s: &TextSnapshot, revision: u64, change: Option<&TextChange>) {
        if self.revision == Some(revision) {
            return;
        }
        if self.revision.is_some_and(|r| r + 1 == revision) {
            if let Some(c) = change.filter(|c| c.revision == revision) {
                self.entries.retain_mut(|(a, b, _, _)| {
                    if c.old_chars.contains(a) || c.old_chars.contains(b) {
                        return false;
                    }
                    for p in [a, b] {
                        if *p >= c.old_chars.end {
                            *p = *p - c.old_chars.len() + c.new_chars.len();
                        }
                    }
                    true
                });
            } else {
                self.entries.clear();
            }
        } else {
            self.entries.clear();
        }
        self.entries
            .retain(|&(a, b, o, c)| b < s.len_chars() && a < b && s.char(a) == o && s.char(b) == c);
        self.revision = Some(revision);
    }
    fn remember(&mut self, a: usize, b: usize, open: char, close: char) {
        if self.entries.len() == 128 {
            self.entries.remove(0);
        }
        self.entries.push((a, b, open, close));
    }
    pub fn empty_range(&self, s: Selection) -> Option<std::ops::Range<usize>> {
        if !s.is_caret() {
            return None;
        }
        self.entries
            .iter()
            .rev()
            .find_map(|&(a, b, _, _)| (a + 1 == b && s.head == b).then_some(a..b + 1))
    }
}
fn close(c: char) -> Option<char> {
    Some(match c {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '\'' => '\'',
        '"' => '"',
        '`' => '`',
        _ => return None,
    })
}
/// Returns code positions and final state. The prefix is bounded before allocation.
/// A slash that could be a JS regex, PHP heredoc, or unknown state disables assistance.
fn scan(text: &str, mut state: lexical::State, ext: &str) -> Option<(Vec<bool>, lexical::State)> {
    use lexical::State::*;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() > BUDGET || state == Unsupported {
        return None;
    }
    let js = matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts"
    );
    let sql = ext == "sql";
    let mut code = vec![false; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            Extended(_) => return None,
            Normal => {
                if c == '/' && next == Some('*') {
                    state = BlockComment;
                    i += 2;
                    continue;
                }
                if (c == '/' && next == Some('/'))
                    || (sql && c == '-' && next == Some('-'))
                    || (ext == "php" && c == '#')
                {
                    state = LineComment;
                } else if js && c == '/' || ext == "php" && c == '<' && next == Some('<') {
                    return None;
                } else if matches!(c, '\'' | '"' | '`') || sql && c == '[' {
                    state = Quoted {
                        closing: if sql && c == '[' { ']' } else { c },
                        escaped: false,
                    };
                } else {
                    code[i] = true;
                }
            }
            BlockComment | DocComment => {
                if c == '*' && next == Some('/') {
                    state = Normal;
                    i += 2;
                    continue;
                }
            }
            LineComment => {
                if c == '\n' {
                    state = Normal;
                }
            }
            Quoted { closing, escaped } => {
                if escaped {
                    state = Quoted {
                        closing,
                        escaped: false,
                    };
                } else if c == '\\' && !sql {
                    state = Quoted {
                        closing,
                        escaped: true,
                    };
                } else if c == closing {
                    if sql && next == Some(closing) {
                        i += 2;
                        continue;
                    }
                    state = Normal;
                }
            }
            Unsupported => return None,
        }
        i += 1;
    }
    Some((code, state))
}
fn context(view: &EditorView, s: &TextSnapshot, at: usize, ext: &str) -> Option<lexical::State> {
    let line = s.char_to_line(at);
    let start = s.line_to_char(line);
    if at - start > BUDGET {
        return None;
    }
    let state = if line == 0 {
        lexical::State::Normal
    } else {
        view.cache.lexical_state(line, ext)?
    };
    scan(&s.slice(start..at).to_string(), state, ext).map(|(_, state)| state)
}
pub(super) fn typed(
    service: &mut codisseu_core::AppService,
    id: DocumentId,
    view: &mut EditorView,
    ext: &str,
    text: String,
) -> Result<(), String> {
    let d = service.active_document().ok_or("Documento ausente")?;
    let s = d.snapshot();
    let before = d.selection();
    let version = d.version;
    let revision = d.last_change().map_or(0, |c| c.revision);
    view.pairs.sync(&s, revision, d.last_change());
    let mut chars = text.chars();
    let c = chars.next();
    let single = c.is_some() && chars.next().is_none();
    if single
        && before.is_caret()
        && view
            .pairs
            .entries
            .iter()
            .any(|&(_, b, _, close)| b == before.head && Some(close) == c)
    {
        return service.dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(before.head + 1),
        });
    }
    let closing = if single { c.and_then(close) } else { None };
    let quote = c.is_some_and(|c| matches!(c, '\'' | '"' | '`'));
    let next = s.get_char(before.range().end);
    let allowed_follow = next
        .is_none_or(|n| n.is_whitespace() || matches!(n, ')' | ']' | '}' | ',' | ';' | '.' | ':'));
    let known = context(view, &s, before.range().start, ext) == Some(lexical::State::Normal);
    let supported = matches!(
        ext,
        "php"
            | "phtml"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "mts"
            | "cts"
            | "mjs"
            | "cjs"
            | "sql"
            | "json"
            | "css"
    );
    if let Some(closing) = closing.filter(|_| {
        supported
            && known
            && (!before.is_caret() || allowed_follow)
            && (!quote || c != Some('`') || matches!(ext, "ts" | "js" | "tsx" | "jsx"))
    }) {
        let r = before.range();
        let content = s.slice(r.clone()).to_string();
        let after = if before.is_caret() {
            Selection::caret(r.start + 1)
        } else {
            Selection {
                anchor: before.anchor + 1,
                head: before.head + 1,
            }
        };
        service.dispatch(Command::EditTransaction {
            id,
            expected_version: version,
            delta: TextDelta {
                range: r.clone(),
                insert: format!("{}{content}{closing}", c.unwrap()),
            },
            kind: EditKind::Atomic,
            before,
            after,
        })?;
        let d = service.active_document().unwrap();
        view.pairs.sync(
            &d.snapshot(),
            d.last_change().map_or(0, |c| c.revision),
            d.last_change(),
        );
        view.pairs.remember(r.start, r.end + 1, c.unwrap(), closing);
        Ok(())
    } else {
        let r = before.range();
        let after = Selection::caret(r.start + text.chars().count());
        service.dispatch(Command::EditTransaction {
            id,
            expected_version: version,
            delta: TextDelta {
                range: r,
                insert: text,
            },
            kind: EditKind::Typing,
            before,
            after,
        })
    }
}
/// At most 16K scalar positions and 256 lines; cached by caller until text/caret changes.
pub(super) fn matching(
    view: &EditorView,
    s: &TextSnapshot,
    at: usize,
    ext: &str,
) -> Option<[usize; 2]> {
    let candidate = if at < s.len_chars() && matches!(s.char(at), '(' | ')' | '[' | ']' | '{' | '}')
    {
        at
    } else if at > 0 && matches!(s.char(at - 1), '(' | ')' | '[' | ']' | '{' | '}') {
        at - 1
    } else {
        return None;
    };
    let line = s.char_to_line(candidate);
    let first = line.saturating_sub(128);
    let last = (line + 129).min(s.len_lines());
    let start = s.line_to_char(first);
    let end = if last < s.len_lines() {
        s.line_to_char(last)
    } else {
        s.len_chars()
    };
    if end - start > BUDGET {
        return None;
    }
    let state = if first == 0 {
        lexical::State::Normal
    } else {
        view.cache.lexical_state(first, ext)?
    };
    let text = s.slice(start..end).to_string();
    let chars: Vec<char> = text.chars().collect();
    let (mask, _) = scan(&text, state, ext)?;
    let index = candidate - start;
    if !mask[index] {
        return None;
    }
    let c = chars[index];
    let forward = matches!(c, '(' | '[' | '{');
    let mut stack = vec![];
    let indices: Box<dyn Iterator<Item = usize>> = if forward {
        Box::new(index..chars.len())
    } else {
        Box::new((0..=index).rev())
    };
    for i in indices {
        if !mask[i] {
            continue;
        }
        let c = chars[i];
        if matches!(c, '(' | ')' | '[' | ']' | '{' | '}') {
            if matches!(c, '(' | '[' | '{') == forward {
                stack.push(c);
            } else {
                let top = stack.pop()?;
                if if forward {
                    close(top) != Some(c)
                } else {
                    close(c) != Some(top)
                } {
                    return None;
                }
                if stack.is_empty() {
                    return Some([candidate, start + i]);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::AppService;
    fn app(text: &str) -> (tempfile::TempDir, AppService, DocumentId, EditorView) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.ts");
        std::fs::write(&p, text).unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        s.dispatch(Command::OpenFile(p)).unwrap();
        let end = Instant::now() + std::time::Duration::from_secs(10);
        while s.is_busy() {
            s.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let id = s.active.unwrap();
        (dir, s, id, EditorView::default())
    }
    fn select(s: &mut AppService, id: DocumentId, selection: Selection) {
        let version = s.active_document().unwrap().version;
        s.dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection,
        })
        .unwrap();
    }
    #[test]
    fn each_pair_passes_its_own_closer_and_undo_is_atomic() {
        for (o, c) in [
            ('(', ')'),
            ('[', ']'),
            ('{', '}'),
            ('\'', '\''),
            ('"', '"'),
            ('`', '`'),
        ] {
            let (_dir, mut s, id, mut v) = app("");
            typed(&mut s, id, &mut v, "ts", o.to_string()).unwrap();
            assert_eq!(s.active_document().unwrap().text(), format!("{o}{c}"));
            assert_eq!(
                s.active_document().unwrap().selection(),
                Selection::caret(1)
            );
            assert_eq!(v.pairs.empty_range(Selection::caret(1)), Some(0..2));
            typed(&mut s, id, &mut v, "ts", c.to_string()).unwrap();
            assert_eq!(
                s.active_document().unwrap().selection(),
                Selection::caret(2)
            );
            assert_eq!(s.active_document().unwrap().text(), format!("{o}{c}"));
            s.dispatch(Command::Undo { id }).unwrap();
            assert_eq!(s.active_document().unwrap().text(), "");
            s.dispatch(Command::Redo { id }).unwrap();
            assert_eq!(s.active_document().unwrap().text(), format!("{o}{c}"));
        }
    }
    #[test]
    fn wrapping_reverse_unicode_selection_is_one_transaction_and_remaps_nested_pairs() {
        let (_dir, mut s, id, mut v) = app("😀e\u{301}");
        select(&mut s, id, Selection { anchor: 3, head: 0 });
        typed(&mut s, id, &mut v, "ts", "(".into()).unwrap();
        assert_eq!(s.active_document().unwrap().text(), "(😀e\u{301})");
        assert_eq!(
            s.active_document().unwrap().selection(),
            Selection { anchor: 4, head: 1 }
        );
        s.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(
            s.active_document().unwrap().selection(),
            Selection { anchor: 3, head: 0 }
        );
        select(&mut s, id, Selection::caret(3));
        typed(&mut s, id, &mut v, "ts", "[".into()).unwrap();
        typed(&mut s, id, &mut v, "ts", "{".into()).unwrap();
        typed(&mut s, id, &mut v, "ts", "}".into()).unwrap();
        typed(&mut s, id, &mut v, "ts", "]".into()).unwrap();
        assert_eq!(s.active_document().unwrap().text(), "😀e\u{301}[{}]");
        assert_eq!(
            s.active_document().unwrap().selection(),
            Selection::caret(7)
        );
    }
    #[test]
    fn comments_strings_regex_heredoc_and_unknown_context_stay_literal() {
        for (source, ext) in [
            ("// x", "ts"),
            ("/* x", "php"),
            ("const s = 'x", "ts"),
            ("const r = /x/; ", "ts"),
            ("$s = <<<TXT ", "php"),
        ] {
            let (_dir, mut s, id, mut v) = app(source);
            select(&mut s, id, Selection::caret(source.chars().count()));
            typed(&mut s, id, &mut v, ext, "(".into()).unwrap();
            assert_eq!(s.active_document().unwrap().text(), format!("{source}("));
        }
        let (_d, mut s, id, mut v) = app("line\n");
        select(&mut s, id, Selection::caret(5));
        typed(&mut s, id, &mut v, "ts", "(".into()).unwrap();
        assert_eq!(s.active_document().unwrap().text(), "line\n(");
    }
    #[test]
    fn matching_ignores_quoted_and_commented_brackets_and_rejects_crossing_or_long_ranges() {
        let view = EditorView::default();
        let s = TextSnapshot::from_str("foo({s: ')', x: [1,2]}) // [");
        assert_eq!(matching(&view, &s, 3, "ts"), Some([3, 22]));
        assert_eq!(matching(&view, &s, 26, "ts"), None);
        let s = TextSnapshot::from_str("([)]");
        assert_eq!(matching(&view, &s, 0, "ts"), None);
        let s = TextSnapshot::from_str(&format!("({})", "a".repeat(BUDGET)));
        assert_eq!(matching(&view, &s, 0, "ts"), None);
    }
}
