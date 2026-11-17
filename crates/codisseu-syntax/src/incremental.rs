use crate::{IssueKind, RenameSnapshot, TextRange, ValidationLimits};
use std::{
    ops::{ControlFlow, Range},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tree_sitter::{InputEdit, ParseOptions, Parser, Point, Tree};
pub(crate) struct Budget<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
    remaining: usize,
}
impl<'a> Budget<'a> {
    pub(crate) fn new(cancel: &'a AtomicBool, limits: ValidationLimits) -> Self {
        Self {
            cancel,
            deadline: Instant::now()
                .checked_add(limits.max_duration)
                .unwrap_or_else(Instant::now),
            remaining: limits.max_nodes,
        }
    }
    pub(crate) fn check(&self) -> Result<(), IssueKind> {
        if self.cancel.load(Ordering::Acquire) {
            Err(IssueKind::Cancelled)
        } else if Instant::now() >= self.deadline || self.remaining == 0 {
            Err(IssueKind::Limit)
        } else {
            Ok(())
        }
    }
    pub(crate) fn node(&mut self) -> Result<(), IssueKind> {
        self.check()?;
        self.remaining -= 1;
        Ok(())
    }
}
pub(crate) fn parse(
    parser: &mut Parser,
    text: &str,
    old: Option<&Tree>,
    budget: &Budget<'_>,
) -> Option<Tree> {
    budget.check().ok()?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if budget.check().is_err() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    parser.parse_with_options(
        &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
        old,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    )
}
#[derive(Clone, Debug)]
pub(crate) struct ByteEdit {
    pub before: Range<usize>,
    pub after: Range<usize>,
    pub new_text: String,
}
impl ByteEdit {
    pub(crate) fn input_edit(&self, text: &str) -> InputEdit {
        let start = point(text, self.before.start);
        let mut end = start;
        for byte in self.new_text.bytes() {
            if byte == b'\n' {
                end.row += 1;
                end.column = 0;
            } else {
                end.column += 1;
            }
        }
        InputEdit {
            start_byte: self.before.start,
            old_end_byte: self.before.end,
            new_end_byte: self.before.start + self.new_text.len(),
            start_position: start,
            old_end_position: point(text, self.before.end),
            new_end_position: end,
        }
    }
}
fn point(text: &str, byte: usize) -> Point {
    let prefix = &text[..byte];
    let row = prefix.bytes().filter(|b| *b == b'\n').count();
    let column = prefix.rsplit('\n').next().unwrap_or("").len();
    Point::new(row, column)
}
pub(crate) fn byte_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text.get(start..)?.find('\n')? + 1;
    }
    let tail = text
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut units = 0;
    for (byte, ch) in tail.char_indices() {
        if units == col {
            return Some(start + byte);
        }
        units += ch.len_utf16() as u32;
        if units > col {
            return None;
        }
    }
    (units == col).then_some(start + tail.len())
}
pub(crate) fn text_range(text: &str, range: Range<usize>) -> TextRange {
    let pos = |byte| {
        let p = &text[..byte];
        (
            p.bytes().filter(|b| *b == b'\n').count() as u32,
            p.rsplit('\n').next().unwrap_or("").encode_utf16().count() as u32,
        )
    };
    let (a, b) = pos(range.start);
    let (c, d) = pos(range.end);
    TextRange {
        start_line: a,
        start_character: b,
        end_line: c,
        end_character: d,
    }
}
pub(crate) fn prepare_edits(
    snapshot: &RenameSnapshot,
    new_name: &str,
    budget: &Budget<'_>,
) -> Result<Vec<ByteEdit>, String> {
    let mut edits = Vec::with_capacity(snapshot.edits.len());
    for edit in &snapshot.edits {
        budget.check().map_err(
            |_| "Validação cancelada ou orçamento excedido durante conversão de edições",
        )?;
        let r = &edit.range;
        let start = byte_offset(&snapshot.before, r.start_line, r.start_character)
            .ok_or("Início UTF-16 inválido")?;
        let end = byte_offset(&snapshot.before, r.end_line, r.end_character)
            .ok_or("Fim UTF-16 inválido")?;
        if start >= end || edit.new_text != new_name {
            return Err(
                "Edição não é substituição exata de identificador pelo nome proposto".into(),
            );
        }
        edits.push(ByteEdit {
            before: start..end,
            after: 0..0,
            new_text: edit.new_text.clone(),
        });
    }
    edits.sort_by_key(|e| e.before.start);
    for pair in edits.windows(2) {
        if pair[0].before.end > pair[1].before.start {
            return Err("Edições sobrepostas".into());
        }
    }
    let mut applied = snapshot.before.to_string();
    for edit in edits.iter().rev() {
        budget.check().map_err(
            |_| "Validação cancelada ou orçamento excedido durante montagem da proposta",
        )?;
        applied.replace_range(edit.before.clone(), &edit.new_text);
    }
    if applied != snapshot.after.as_ref() {
        return Err("Texto posterior não corresponde exatamente às edições".into());
    }
    let mut delta = 0isize;
    for edit in &mut edits {
        let start = edit
            .before
            .start
            .checked_add_signed(delta)
            .ok_or("Delta de edição inválido")?;
        edit.after = start..start + edit.new_text.len();
        delta += edit.new_text.len() as isize - (edit.before.end - edit.before.start) as isize;
    }
    Ok(edits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RenameEdit, RenameSnapshot};
    use std::sync::Arc;
    #[test]
    fn edited_before_tree_reparse_matches_fresh_and_reuses_unchanged_class_subtree() {
        for (language, path, prefix) in [
            (crate::ast::Language::TypeScript, "A.ts", ""),
            (crate::ast::Language::Php, "A.php", "<?php "),
        ] {
            let keyword = if language == crate::ast::Language::Php {
                "function "
            } else {
                ""
            };
            let before = format!(
                "{prefix}/* 😀 e\u{301} */ class A {{ {keyword}rainfall(){{}} }}\r\nclass Stable {{ {keyword}keep(){{}} }}\r\n"
            );
            let start = before.find("rainfall").unwrap();
            let after = before.replace("rainfall", "muchLongerForecast");
            let snap = RenameSnapshot {
                path: path.into(),
                before: Arc::from(before.clone()),
                after: Arc::from(after.clone()),
                edits: vec![RenameEdit {
                    range: text_range(&before, start..start + 8),
                    new_text: "muchLongerForecast".into(),
                }],
            };
            let c = AtomicBool::new(false);
            let budget = Budget::new(&c, ValidationLimits::default());
            let mut parser = Parser::new();
            parser.set_language(&language.grammar()).unwrap();
            let mut old = parse(&mut parser, &before, None, &budget).unwrap();
            let mut old_cursor = old.root_node().walk();
            let stable_id = old
                .root_node()
                .named_children(&mut old_cursor)
                .find(|node| {
                    node.child_by_field_name("name")
                        .is_some_and(|name| &before[name.byte_range()] == "Stable")
                })
                .unwrap()
                .id();
            drop(old_cursor);
            let edits = prepare_edits(&snap, "muchLongerForecast", &budget).unwrap();
            for e in edits.iter().rev() {
                old.edit(&e.input_edit(&before));
            }
            let incremental = parse(&mut parser, &after, Some(&old), &budget).unwrap();
            let fresh = parse(&mut parser, &after, None, &budget).unwrap();
            assert_eq!(
                incremental.root_node().to_sexp(),
                fresh.root_node().to_sexp()
            );
            // Reused node identity is evidence beyond merely calling parse(Some(tree)).
            let mut cursor = incremental.root_node().walk();
            let reused = incremental
                .root_node()
                .named_children(&mut cursor)
                .any(|n| n.id() == stable_id);
            assert!(
                reused,
                "unchanged sibling subtree must be reused: {}",
                incremental.root_node().to_sexp()
            );
        }
    }
}
