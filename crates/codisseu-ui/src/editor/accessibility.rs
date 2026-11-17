//! Visible text runs plus off-screen selection endpoints; never a full-file value clone.
use super::*;
use egui::accesskit::{Action, Rect, Role, TextPosition, TextSelection};
#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    ui: &mut egui::Ui,
    id: egui::Id,
    snapshot: &TextSnapshot,
    selection: Selection,
    view: &mut EditorView,
    visible: std::ops::Range<usize>,
    origin: egui::Pos2,
    ext: &str,
    t: &Tokens,
    path: &std::path::Path,
) {
    if !ui.ctx().has_accesskit_node(id) {
        return;
    }
    view.accessible_positions.clear();
    let mut lines = visible
        .filter(|line| !view.fold.is_hidden(*line))
        .collect::<Vec<_>>();
    for point in [selection.anchor, selection.head] {
        lines.push(snapshot.char_to_line(point));
    }
    lines.sort_unstable();
    lines.dedup();
    let mut positions = BTreeMap::<usize, TextPosition>::new();
    for line in lines {
        let layout = view.cache.get(ui, snapshot, line, ext, t);
        let start = snapshot.line_to_char(line);
        let raw = snapshot.line(line);
        let n = raw.len_chars();
        let x = ui.clip_rect().left() - origin.x..ui.clip_rect().right() - origin.x;
        let mut chunks = Vec::new();
        let mut visible_ranges = Vec::new();
        if view.wrap.enabled {
            let first = (view.scroll.y / t.editor_line_height).floor().max(0.) as usize;
            let last = ((view.scroll.y + view.viewport_size.y) / t.editor_line_height)
                .ceil()
                .max(0.) as usize
                + 1;
            let mut rows = (first..last.min(view.wrap.total(&view.fold)))
                .filter(|row| view.wrap.source_line(&view.fold, *row) == line)
                .collect::<Vec<_>>();
            for point in [selection.anchor, selection.head] {
                if point >= start
                    && point <= start + n
                    && let Some(x) = layout.x(ui, snapshot, t, point - start)
                    && let Some((_, row)) = soft_wrap::caret_position(
                        view,
                        line,
                        x,
                        point == selection.head && view.wrap_previous_affinity,
                    )
                {
                    rows.push(row);
                }
            }
            rows.sort_unstable();
            rows.dedup();
            for row in rows {
                if let Some(mut range) =
                    soft_wrap::source_range(ui, snapshot, view, line, row, &layout, t)
                {
                    if range.end == layout.len() {
                        range.end = n;
                    }
                    chunks.extend([range.start, range.end]);
                    chunks.extend(
                        (range.start / 255..range.end.max(range.start + 1).div_ceil(255))
                            .map(|c| c * 255)
                            .filter(|c| *c >= range.start),
                    );
                    visible_ranges.push(range);
                }
            }
        } else {
            for (r, _) in layout.visible_text(ui, snapshot, t, x) {
                let end = if r.end == layout.len() { n } else { r.end };
                chunks.extend((r.start / 255..end.max(r.start + 1).div_ceil(255)).map(|c| c * 255));
            }
        }
        for point in [selection.anchor, selection.head] {
            if point >= start && point <= start + n {
                chunks.push((point - start).saturating_sub(1) / 255 * 255);
            }
        }
        if n == 0 {
            chunks.push(0);
        }
        let ghost_column = view
            .ghost
            .as_ref()
            .filter(|g| g.line == line)
            .map(|g| g.caret - start);
        if let Some(column) = ghost_column {
            chunks.push(column);
            chunks.extend(
                view.ghost
                    .as_ref()
                    .unwrap()
                    .source_breaks
                    .iter()
                    .map(|c| c - start),
            );
        }
        chunks.sort_unstable();
        chunks.dedup();
        let mut runs = Vec::new();
        let mut run_rows = Vec::new();
        if ghost_column == Some(0) && n > 0 {
            // At column zero, preserve an insertion endpoint before the ghost,
            // while the actual source run starts after its virtual final row.
            let anchor = id.with(("ghost.anchor", line));
            let node = anchor.accesskit_id();
            let y = origin.y + ghost::visual_row(view, line) as f32 * t.editor_line_height;
            let _anchor_ui = ui.new_child(
                egui::UiBuilder::new()
                    .scope_id(anchor)
                    .accessibility_parent(id)
                    .accessibility_role(Role::TextRun),
            );
            ui.ctx().accesskit_node_builder(anchor, |builder| {
                builder.set_role(Role::TextRun);
                builder.set_value(String::new());
                builder.set_bounds(Rect {
                    x0: origin.x as f64,
                    y0: y as f64,
                    x1: (origin.x + 1.) as f64,
                    y1: (y + t.editor_line_height) as f64,
                });
            });
            view.accessible_positions.insert(node, (start, 0));
            positions.insert(
                start,
                TextPosition {
                    node,
                    character_index: 0,
                },
            );
        }
        for (index, &a) in chunks.iter().enumerate() {
            let b = (a + 255)
                .min(n)
                .min(chunks.get(index + 1).copied().unwrap_or(n));
            if view.wrap.enabled
                && a < b
                && !visible_ranges
                    .iter()
                    .any(|range| a >= range.start && a < range.end)
            {
                continue;
            }
            let Some(x0) = layout.x(ui, snapshot, t, a) else {
                continue;
            };
            let Some(x1) = layout.x(ui, snapshot, t, b) else {
                continue;
            };
            let value = raw.slice(a..b).to_string();
            let run = id.with(("line", line, a));
            let node = run.accesskit_id();
            view.accessible_positions.insert(node, (start + a, b - a));
            let _run_ui = ui.new_child(
                egui::UiBuilder::new()
                    .scope_id(run)
                    .accessibility_parent(id)
                    .max_rect(ui.max_rect())
                    .accessibility_role(Role::TextRun),
            );
            let Some((display_x, row)) = soft_wrap::source_position(view, line, x0, false) else {
                continue;
            };
            let (display_end, end_row) =
                soft_wrap::source_position(view, line, x1, true).unwrap_or((display_x, row));
            let display_end = if row == end_row {
                display_end
            } else {
                display_x + 1.
            };
            let y = origin.y + row as f32 * t.editor_line_height;
            let xs = (a..=b)
                .map(|column| {
                    let x = layout.x(ui, snapshot, t, column).unwrap_or(x1);
                    soft_wrap::source_position(view, line, x, column == b)
                        .map_or(display_x, |(x, _)| x)
                })
                .collect::<Vec<_>>();
            ui.ctx().accesskit_node_builder(run, |builder| {
                builder.set_role(Role::TextRun);
                builder.set_value(value.clone());
                builder.set_bounds(Rect {
                    x0: (origin.x + display_x) as f64,
                    y0: y as f64,
                    x1: (origin.x + display_end.max(display_x + 1.)) as f64,
                    y1: (y + t.editor_line_height) as f64,
                });
                // Raw scalar text, never reconstructed from shaped glyphs (ZWJ/fallback).
                builder.set_character_lengths(
                    value
                        .chars()
                        .map(|c| c.len_utf8() as u8)
                        .collect::<Vec<_>>(),
                );
                builder.set_character_positions(
                    xs[..b - a]
                        .iter()
                        .map(|x| x - display_x)
                        .collect::<Vec<_>>(),
                );
                builder.set_character_widths(
                    xs.windows(2)
                        .map(|x| (x[1] - x[0]).max(0.))
                        .collect::<Vec<_>>(),
                );
                builder.set_text_direction(egui::accesskit::TextDirection::LeftToRight);
            });
            runs.push(run);
            run_rows.push(row);
            for point in [selection.anchor, selection.head] {
                if point >= start + a && point <= start + b {
                    let position = TextPosition {
                        node,
                        character_index: point - start - a,
                    };
                    if point == start + a
                        && !(point == selection.head && view.wrap_previous_affinity)
                        && ghost_column != Some(point - start)
                    {
                        positions.insert(point, position);
                    } else {
                        positions.entry(point).or_insert(position);
                    }
                }
            }
        }
        for (i, run) in runs.iter().enumerate() {
            ui.ctx().accesskit_node_builder(*run, |builder| {
                if i > 0 && run_rows[i - 1] == run_rows[i] {
                    builder.set_previous_on_line(runs[i - 1].accesskit_id());
                }
                if i + 1 < runs.len() && run_rows[i + 1] == run_rows[i] {
                    builder.set_next_on_line(runs[i + 1].accesskit_id());
                }
            });
        }
    }
    ui.ctx().accesskit_node_builder(id, |builder| {
        builder.set_role(Role::MultilineTextInput);
        builder.set_label(path.to_string_lossy().to_string());
        for action in [
            Action::Focus,
            Action::SetValue,
            Action::ReplaceSelectedText,
            Action::SetTextSelection,
        ] {
            builder.add_action(action);
        }
        if let (Some(anchor), Some(focus)) = (
            positions.get(&selection.anchor),
            positions.get(&selection.head),
        ) {
            builder.set_text_selection(TextSelection {
                anchor: *anchor,
                focus: *focus,
            });
        }
    });
}
