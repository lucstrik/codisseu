//! Explicit, widget-independent edit plans in original Unicode scalar coordinates.
use crate::{
    MAX_EDITOR_DOCUMENT_BYTES, MAX_EDITOR_DOCUMENT_LINES, Selection, TextDelta, TextSnapshot,
};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Up,
    Down,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Indentation {
    Spaces(usize),
    Tabs { tab_width: usize },
}
/// Detect the existing indentation from at most128 lines/32KiB without copying the rope.
/// Tabs must outnumber space-prefixed samples; otherwise use their GCD (1–8), default4.
/// Tab display width is4 because a file does not encode its visual tab-stop preference.
pub fn detect_indentation(snapshot: &TextSnapshot) -> Indentation {
    let mut tabs = 0;
    let mut spaces = 0;
    let mut width = 0usize;
    let mut bytes = 0;
    for line in snapshot.lines().take(128) {
        bytes += line.len_bytes();
        if bytes > 32 * 1024 {
            break;
        }
        if line.len_chars() == 0 {
            continue;
        }
        if line.char(0) == '\t' {
            tabs += 1;
        } else {
            let count = line.chars().take_while(|c| *c == ' ').count();
            if count > 0 && count <= 16 {
                spaces += 1;
                width = if width == 0 {
                    count
                } else {
                    let (mut a, mut b) = (width, count);
                    while b != 0 {
                        (a, b) = (b, a % b);
                    }
                    a
                };
            }
        }
    }
    if tabs > spaces {
        Indentation::Tabs { tab_width: 4 }
    } else {
        Indentation::Spaces(if (1..=8).contains(&width) { width } else { 4 })
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommentSyntax {
    pub prefix: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaseTransform {
    Lower,
    Upper,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EditorCommand {
    MoveLines(Direction),
    CopyLines(Direction),
    DeleteLines,
    InsertLineAbove,
    InsertLineBelow,
    Indent(Indentation),
    Outdent(Indentation),
    JoinLines,
    SelectLines,
    ToggleLineComment(CommentSyntax),
    ToggleBlockComment { open: String, close: String },
    DeleteAllLeft,
    DeleteAllRight,
    TrimTrailingWhitespace,
    TransformCase(CaseTransform),
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandEdit {
    pub deltas: Vec<TextDelta>,
    pub selections: Vec<Selection>,
}
struct Op {
    delta: TextDelta,
    targets: Vec<(usize, Selection)>,
}
#[derive(Clone)]
struct Group {
    first: usize,
    last: usize,
    selections: Vec<usize>,
}
fn start(text: &TextSnapshot, line: usize) -> usize {
    text.line_to_char(line)
}
fn end(text: &TextSnapshot, line: usize) -> usize {
    start(text, line) + text.line(line).len_chars()
}
fn parts(text: &TextSnapshot, line: usize) -> (String, String) {
    let raw = text.line(line).to_string();
    let ending = if raw.ends_with("\r\n") {
        "\r\n"
    } else {
        match raw.chars().next_back() {
            Some('\n') => "\n",
            Some('\r') => "\r",
            _ => "",
        }
    };
    (raw[..raw.len() - ending.len()].into(), ending.into())
}
fn local_eol(text: &TextSnapshot, line: usize) -> String {
    let ending = parts(text, line).1;
    if !ending.is_empty() {
        ending
    } else if line > 0 {
        parts(text, line - 1).1
    } else {
        "\n".into()
    }
}
fn groups(text: &TextSnapshot, selections: &[Selection], adjacent: bool) -> Vec<Group> {
    let mut groups = selections
        .iter()
        .enumerate()
        .map(|(i, selection)| {
            let range = selection.range();
            let first = text.char_to_line(range.start);
            let mut last = text.char_to_line(range.end);
            if !range.is_empty() && last > first && range.end == start(text, last) {
                last -= 1;
            }
            Group {
                first,
                last,
                selections: vec![i],
            }
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|g| (g.first, g.last));
    let mut merged: Vec<Group> = Vec::new();
    for group in groups {
        if let Some(previous) = merged.last_mut()
            && group.first <= previous.last + usize::from(adjacent)
        {
            previous.last = previous.last.max(group.last);
            previous.selections.extend(group.selections);
        } else {
            merged.push(group);
        }
    }
    merged
}
fn selection_from(range: Range<usize>, original: Selection) -> Selection {
    if original.anchor > original.head {
        Selection {
            anchor: range.end,
            head: range.start,
        }
    } else {
        Selection {
            anchor: range.start,
            head: range.end,
        }
    }
}
fn shifted(selection: Selection, amount: usize) -> Selection {
    Selection {
        anchor: selection.anchor + amount,
        head: selection.head + amount,
    }
}
fn map_group(
    text: &TextSnapshot,
    group: &Group,
    selections: &[Selection],
    insert: &str,
    target_line: usize,
) -> Vec<(usize, Selection)> {
    let replacement = TextSnapshot::from_str(insert);
    let length = group.last - group.first + 1;
    let map = |position: usize| {
        if position == end(text, group.last) && !parts(text, group.last).1.is_empty() {
            let row = target_line + length;
            return if row < replacement.len_lines() {
                replacement.line_to_char(row)
            } else {
                replacement.len_chars()
            };
        }
        let row = text.char_to_line(position);
        let target = target_line + row.saturating_sub(group.first);
        let col = position - start(text, row);
        let content = parts(&replacement, target).0.chars().count();
        replacement.line_to_char(target) + col.min(content)
    };
    group
        .selections
        .iter()
        .map(|&i| {
            (
                i,
                Selection {
                    anchor: map(selections[i].anchor),
                    head: map(selections[i].head),
                },
            )
        })
        .collect()
}
fn finish(
    text: &TextSnapshot,
    selections: &[Selection],
    mut ops: Vec<Op>,
) -> Result<CommandEdit, String> {
    ops.sort_by_key(|op| (op.delta.range.start, op.delta.range.end));
    let mut previous = None;
    for op in &ops {
        if previous.is_some_and(|(a, b)| op.delta.range.start < b || op.delta.range.start == a) {
            return Err("Comando gerou edições sobrepostas".into());
        }
        previous = Some((op.delta.range.start, op.delta.range.end));
    }
    let mut after = selections.to_vec();
    for op in ops.iter().rev() {
        for selection in &mut after {
            *selection = selection.mapped(&op.delta);
        }
    }
    let mut removed = 0;
    let mut inserted = 0;
    for op in &ops {
        let offset = op.delta.range.start - removed + inserted;
        for (index, target) in &op.targets {
            after[*index] = shifted(*target, offset);
        }
        removed += op.delta.range.len();
        inserted += op.delta.insert.chars().count();
    }
    let mut candidate = text.clone();
    for op in ops.iter().rev() {
        candidate.remove(op.delta.range.clone());
        candidate.insert(op.delta.range.start, &op.delta.insert);
    }
    if candidate.len_bytes() > MAX_EDITOR_DOCUMENT_BYTES
        || candidate.len_lines() > MAX_EDITOR_DOCUMENT_LINES
    {
        return Err("Comando excede os limites de16MiB/1.000.000linhas do editor".into());
    }
    for selection in &after {
        selection.validate(candidate.len_chars())?;
    }
    let deltas = ops
        .into_iter()
        .filter_map(|op| {
            (text.slice(op.delta.range.clone()) != op.delta.insert.as_str()).then_some(op.delta)
        })
        .collect();
    Ok(CommandEdit {
        deltas,
        selections: after,
    })
}
fn width(indentation: Indentation) -> Result<usize, String> {
    let width = match indentation {
        Indentation::Spaces(n) | Indentation::Tabs { tab_width: n } => n,
    };
    if !(1..=16).contains(&width) {
        Err("Indentação deve ter de1a16 colunas".into())
    } else {
        Ok(width)
    }
}
fn syntax(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 32 || value.chars().any(char::is_control) {
        Err("Delimitador de comentário inválido".into())
    } else {
        Ok(())
    }
}
fn scalar_prefix(value: &str, count: usize) -> &str {
    &value[..value
        .char_indices()
        .nth(count)
        .map_or(value.len(), |(byte, _)| byte)]
}
fn convert(value: &str, case: CaseTransform) -> String {
    match case {
        CaseTransform::Lower => value.to_lowercase(),
        CaseTransform::Upper => value.to_uppercase(),
    }
}
/// Plan one explicit edit; the integrator must bind the snapshot revision before applying.
/// Empty `deltas` is selection-only/no-op and must not create text history.
pub fn plan(
    text: &TextSnapshot,
    selections: &[Selection],
    command: EditorCommand,
) -> Result<CommandEdit, String> {
    if selections.is_empty() || selections.len() > 512 {
        return Err("Comando exige de1a512 seleções".into());
    }
    for selection in selections {
        selection.validate(text.len_chars())?;
        for position in [selection.anchor, selection.head] {
            if position < text.len_chars()
                && crate::previous_grapheme(text, crate::next_grapheme(text, position)) != position
            {
                return Err("Seleção corta um grafema ou CRLF".into());
            }
        }
    }
    if let EditorCommand::Indent(indentation) | EditorCommand::Outdent(indentation) = command {
        width(indentation)?;
    }
    if let EditorCommand::ToggleLineComment(ref comment) = command {
        syntax(&comment.prefix)?;
    }
    if let EditorCommand::ToggleBlockComment {
        ref open,
        ref close,
    } = command
    {
        syntax(open)?;
        syntax(close)?;
    }
    let delete_lines = command == EditorCommand::DeleteLines;
    let mut ops = Vec::new();
    let groups = groups(
        text,
        selections,
        !matches!(
            command,
            EditorCommand::InsertLineAbove
                | EditorCommand::InsertLineBelow
                | EditorCommand::SelectLines
                | EditorCommand::JoinLines
        ),
    );
    if command == EditorCommand::SelectLines {
        let mut after = selections.to_vec();
        for group in groups {
            for i in group.selections {
                after[i] = selection_from(
                    start(text, group.first)..end(text, group.last),
                    selections[i],
                );
            }
        }
        return Ok(CommandEdit {
            deltas: vec![],
            selections: after,
        });
    }
    match command {
        EditorCommand::MoveLines(direction) => {
            for group in groups {
                if (direction == Direction::Up && group.first == 0)
                    || (direction == Direction::Down && group.last + 1 == text.len_lines())
                {
                    continue;
                }
                let first = if direction == Direction::Up {
                    group.first - 1
                } else {
                    group.first
                };
                let last = if direction == Direction::Down {
                    group.last + 1
                } else {
                    group.last
                };
                let mut insert = String::new();
                for target in first..=last {
                    let source = if direction == Direction::Up {
                        if target == last { first } else { target + 1 }
                    } else if target == first {
                        last
                    } else {
                        target - 1
                    };
                    insert.push_str(&parts(text, source).0);
                    insert.push_str(&parts(text, target).1);
                }
                let target = usize::from(direction == Direction::Down);
                let targets = map_group(text, &group, selections, &insert, target);
                ops.push(Op {
                    delta: TextDelta {
                        range: start(text, first)..end(text, last),
                        insert,
                    },
                    targets,
                });
            }
        }
        EditorCommand::CopyLines(direction) => {
            for group in groups {
                let range = start(text, group.first)..end(text, group.last);
                let block = text.slice(range.clone()).to_string();
                let separator = if parts(text, group.last).1.is_empty() {
                    local_eol(text, group.last)
                } else {
                    String::new()
                };
                let offset = if direction == Direction::Down {
                    block.chars().count() + separator.chars().count()
                } else {
                    0
                };
                let targets = group
                    .selections
                    .iter()
                    .map(|&i| {
                        (
                            i,
                            Selection {
                                anchor: selections[i].anchor - range.start + offset,
                                head: selections[i].head - range.start + offset,
                            },
                        )
                    })
                    .collect();
                ops.push(Op {
                    delta: TextDelta {
                        range,
                        insert: format!("{block}{separator}{block}"),
                    },
                    targets,
                });
            }
        }
        EditorCommand::DeleteLines => {
            for group in groups {
                let mut range = start(text, group.first)..end(text, group.last);
                if range.end == text.len_chars()
                    && parts(text, group.last).1.is_empty()
                    && group.first > 0
                {
                    range.start -= parts(text, group.first - 1).1.chars().count();
                }
                ops.push(Op {
                    delta: TextDelta {
                        range,
                        insert: String::new(),
                    },
                    targets: group
                        .selections
                        .iter()
                        .map(|&i| (i, Selection::caret(0)))
                        .collect(),
                });
            }
        }
        EditorCommand::InsertLineAbove | EditorCommand::InsertLineBelow => {
            for group in groups {
                let above = command == EditorCommand::InsertLineAbove;
                let line = if above { group.first } else { group.last };
                let content = parts(text, line).0;
                let indentation = content
                    .chars()
                    .take_while(|c| matches!(c, ' ' | '\t'))
                    .collect::<String>();
                let eol = local_eol(text, line);
                let at = if above {
                    start(text, line)
                } else {
                    end(text, line)
                };
                let missing = !above && parts(text, line).1.is_empty();
                let insert = if missing {
                    format!("{eol}{indentation}")
                } else {
                    format!("{indentation}{eol}")
                };
                let caret =
                    indentation.chars().count() + if missing { eol.chars().count() } else { 0 };
                ops.push(Op {
                    delta: TextDelta {
                        range: at..at,
                        insert,
                    },
                    targets: group
                        .selections
                        .iter()
                        .map(|&i| (i, Selection::caret(caret)))
                        .collect(),
                });
            }
        }
        EditorCommand::Indent(indentation) | EditorCommand::Outdent(indentation) => {
            let outdent = matches!(command, EditorCommand::Outdent(_));
            let columns = width(indentation)?;
            let prefix = match indentation {
                Indentation::Spaces(n) => " ".repeat(n),
                Indentation::Tabs { .. } => "\t".into(),
            };
            for group in groups {
                ops.push(transform_lines(
                    text,
                    selections,
                    &group,
                    false,
                    |content, _| {
                        if !outdent {
                            return (0, 0, prefix.clone());
                        }
                        let mut col = 0;
                        let mut count = 0;
                        for c in content.chars() {
                            if c == ' ' {
                                col += 1;
                            } else if c == '\t' {
                                col += columns - col % columns;
                            } else {
                                break;
                            }
                            count += 1;
                            if col >= columns {
                                break;
                            }
                        }
                        (0, count, String::new())
                    },
                ));
            }
        }
        EditorCommand::ToggleLineComment(comment) => {
            for group in groups {
                let mut uncomment = true;
                let mut nonempty = false;
                let mut indent = usize::MAX;
                for line in group.first..=group.last {
                    let body = parts(text, line).0;
                    if body.trim().is_empty() {
                        continue;
                    }
                    nonempty = true;
                    let n = body.chars().take_while(|c| matches!(c, ' ' | '\t')).count();
                    indent = indent.min(n);
                    uncomment &= body[scalar_prefix(&body, n).len()..].starts_with(&comment.prefix);
                }
                if !nonempty {
                    uncomment = false;
                    indent = 0;
                }
                ops.push(transform_lines(
                    text,
                    selections,
                    &group,
                    true,
                    |content, _| {
                        if content.trim().is_empty() && nonempty {
                            return (0, 0, String::new());
                        }
                        if uncomment {
                            let n = content
                                .chars()
                                .take_while(|c| matches!(c, ' ' | '\t'))
                                .count();
                            let tail =
                                &content[scalar_prefix(content, n).len() + comment.prefix.len()..];
                            (
                                n,
                                comment.prefix.chars().count() + usize::from(tail.starts_with(' ')),
                                String::new(),
                            )
                        } else {
                            (indent, 0, format!("{} ", comment.prefix))
                        }
                    },
                ));
            }
        }
        EditorCommand::JoinLines => {
            let mut joined_groups = Vec::new();
            for mut group in groups {
                if group.first == group.last && group.last + 1 < text.len_lines() {
                    group.last += 1;
                }
                if let Some(previous) = joined_groups.last_mut() as Option<&mut Group>
                    && group.first <= previous.last
                {
                    previous.last = previous.last.max(group.last);
                    previous.selections.extend(group.selections);
                } else {
                    joined_groups.push(group);
                }
            }
            for group in joined_groups {
                if group.first == group.last {
                    continue;
                }
                let mut insert = String::new();
                let mut offsets = BTreeMap::new();
                for line in group.first..=group.last {
                    let body = parts(text, line).0;
                    let trimmed = if line == group.first {
                        body.trim_end_matches([' ', '\t'])
                    } else {
                        body.trim_matches([' ', '\t'])
                    };
                    if !insert.is_empty() && !trimmed.is_empty() {
                        insert.push(' ');
                    }
                    let removed = if line == group.first {
                        0
                    } else {
                        body.chars().take_while(|c| matches!(c, ' ' | '\t')).count()
                    };
                    offsets.insert(
                        line,
                        (insert.chars().count(), removed, trimmed.chars().count()),
                    );
                    insert.push_str(trimmed);
                }
                let map = |p: usize| {
                    let line = text.char_to_line(p).clamp(group.first, group.last);
                    let (offset, removed, length) = offsets[&line];
                    offset + (p - start(text, line)).saturating_sub(removed).min(length)
                };
                let targets = group
                    .selections
                    .iter()
                    .map(|&i| {
                        (
                            i,
                            Selection {
                                anchor: map(selections[i].anchor),
                                head: map(selections[i].head),
                            },
                        )
                    })
                    .collect();
                let last_content =
                    end(text, group.last) - parts(text, group.last).1.chars().count();
                ops.push(Op {
                    delta: TextDelta {
                        range: start(text, group.first)..last_content,
                        insert,
                    },
                    targets,
                });
            }
        }
        EditorCommand::DeleteAllLeft | EditorCommand::DeleteAllRight => {
            let left = command == EditorCommand::DeleteAllLeft;
            let ranges = selections
                .iter()
                .map(|selection| {
                    if !selection.is_caret() {
                        return selection.range();
                    }
                    let line = text.char_to_line(selection.head);
                    if left {
                        start(text, line)..selection.head
                    } else {
                        selection.head..end(text, line) - parts(text, line).1.chars().count()
                    }
                })
                .collect::<Vec<_>>();
            ops = range_ops(ranges, |range| TextDelta {
                range,
                insert: String::new(),
            });
        }
        EditorCommand::TransformCase(case) => {
            let ranges = selections
                .iter()
                .map(|selection| {
                    if selection.is_caret() {
                        crate::word_range(text, selection.head)
                    } else {
                        selection.range()
                    }
                })
                .collect::<Vec<_>>();
            ops = range_ops(ranges, |range| TextDelta {
                insert: convert(&text.slice(range.clone()).to_string(), case),
                range,
            });
            for op in &mut ops {
                for (i, selection) in selections.iter().enumerate() {
                    if selection.range().start >= op.delta.range.start
                        && selection.range().end <= op.delta.range.end
                    {
                        let map = |p| {
                            convert(&text.slice(op.delta.range.start..p).to_string(), case)
                                .chars()
                                .count()
                        };
                        op.targets.push((
                            i,
                            Selection {
                                anchor: map(selection.anchor),
                                head: map(selection.head),
                            },
                        ));
                    }
                }
            }
        }
        EditorCommand::ToggleBlockComment { open, close } => {
            let ranges = selections
                .iter()
                .map(|selection| selection.range())
                .collect::<Vec<_>>();
            for mut range in merged_ranges(ranges, true) {
                let body = text.slice(range.clone()).to_string();
                let open_count = open.chars().count();
                let close_count = close.chars().count();
                let surrounding = range.start >= open_count
                    && range.end + close_count <= text.len_chars()
                    && text.slice(range.start - open_count..range.start) == open.as_str()
                    && text.slice(range.end..range.end + close_count) == close.as_str();
                let (insert, removed, added) = if body.starts_with(&open)
                    && body.ends_with(&close)
                    && body.len() >= open.len() + close.len()
                {
                    (
                        body[open.len()..body.len() - close.len()].to_owned(),
                        open_count,
                        0,
                    )
                } else if surrounding {
                    range.start -= open_count;
                    range.end += close_count;
                    (body, open_count, 0)
                } else {
                    (format!("{open}{body}{close}"), 0, open_count)
                };
                let count = insert.chars().count();
                let targets = selections
                    .iter()
                    .enumerate()
                    .filter(|(_, selection)| {
                        selection.range().start >= range.start && selection.range().end <= range.end
                    })
                    .map(|(i, selection)| {
                        let map = |p: usize| {
                            (p - range.start)
                                .saturating_sub(removed)
                                .min(count.saturating_sub(added))
                                + added
                        };
                        (
                            i,
                            Selection {
                                anchor: map(selection.anchor),
                                head: map(selection.head),
                            },
                        )
                    })
                    .collect();
                ops.push(Op {
                    delta: TextDelta { range, insert },
                    targets,
                });
            }
        }
        EditorCommand::TrimTrailingWhitespace => {
            let group = Group {
                first: 0,
                last: text.len_lines() - 1,
                selections: (0..selections.len()).collect(),
            };
            ops.push(transform_lines(
                text,
                selections,
                &group,
                true,
                |content, _| {
                    let trimmed = content.trim_end_matches([' ', '\t']);
                    (
                        trimmed.chars().count(),
                        content[trimmed.len()..].chars().count(),
                        String::new(),
                    )
                },
            ));
        }
        EditorCommand::SelectLines => unreachable!(),
    }
    let mut result = finish(text, selections, ops)?;
    if delete_lines && !result.deltas.is_empty() {
        let mut after = text.clone();
        for delta in result.deltas.iter().rev() {
            after.remove(delta.range.clone());
            after.insert(delta.range.start, &delta.insert);
        }
        for selection in &mut result.selections {
            *selection = Selection::caret(after.line_to_char(after.char_to_line(selection.head)));
        }
    }
    Ok(result)
}
fn merged_ranges(mut ranges: Vec<Range<usize>>, keep_empty: bool) -> Vec<Range<usize>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut result: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if !keep_empty && range.is_empty() {
            continue;
        }
        if let Some(previous) = result.last_mut()
            && range.start <= previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            result.push(range);
        }
    }
    result
}
fn range_ops(
    ranges: Vec<Range<usize>>,
    mut build: impl FnMut(Range<usize>) -> TextDelta,
) -> Vec<Op> {
    merged_ranges(ranges, false)
        .into_iter()
        .map(|range| Op {
            delta: build(range),
            targets: vec![],
        })
        .collect()
}
/// One replacement per contiguous line block; endpoint metadata stays bounded by512.
fn transform_lines(
    text: &TextSnapshot,
    selections: &[Selection],
    group: &Group,
    keep_line_start: bool,
    mut transform: impl FnMut(&str, usize) -> (usize, usize, String),
) -> Op {
    let range = start(text, group.first)..end(text, group.last);
    let mut points: BTreeMap<usize, Vec<(usize, bool, usize)>> = BTreeMap::new();
    let mut mapped = selections.to_vec();
    for &i in &group.selections {
        for (is_head, p) in [(false, selections[i].anchor), (true, selections[i].head)] {
            let line = text.char_to_line(p);
            points
                .entry(line)
                .or_default()
                .push((i, is_head, p - start(text, line)));
        }
    }
    let mut insert = String::new();
    let mut count = 0;
    for line in group.first..=group.last {
        let (body, eol) = parts(text, line);
        let (at, removed, added) = transform(&body, line);
        let prefix = scalar_prefix(&body, at);
        let suffix = &body[scalar_prefix(&body, at + removed).len()..];
        if let Some(points) = points.get(&line) {
            for &(i, is_head, col) in points {
                let next = if col < at || (keep_line_start && col == 0 && !selections[i].is_caret())
                {
                    col
                } else if col >= at + removed {
                    col - removed + added.chars().count()
                } else {
                    at + added.chars().count()
                };
                if is_head {
                    mapped[i].head = count + next;
                } else {
                    mapped[i].anchor = count + next;
                }
            }
        }
        insert.push_str(prefix);
        insert.push_str(&added);
        insert.push_str(suffix);
        insert.push_str(&eol);
        count += prefix.chars().count()
            + added.chars().count()
            + suffix.chars().count()
            + eol.chars().count();
    }
    if let Some(points) = points.get(&(group.last + 1)) {
        for &(i, is_head, _) in points {
            if is_head {
                mapped[i].head = count;
            } else {
                mapped[i].anchor = count;
            }
        }
    }
    Op {
        delta: TextDelta { range, insert },
        targets: group.selections.iter().map(|&i| (i, mapped[i])).collect(),
    }
}
