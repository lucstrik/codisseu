//! View-only rows over the measured, source-preserving horizontal layout.
//! Unvisited lines have a provisional height; detailed rows are rebuilt from
//! sparse checkpoints under a frame budget instead of retaining a giant galley.
use super::*;
use std::ops::Range;

const FRAME_ROWS: usize = 256;
const CHECKPOINT_ROWS: usize = 128;
const RETAINED_ROWS: usize = 1024;
const RETAINED_LINES: usize = 128;

#[derive(Clone, Debug)]
pub(super) struct Segment {
    pub projection: usize,
    pub x: Range<f32>,
    pub touched: u64,
}
#[derive(Clone, Copy, Default)]
struct Cursor {
    row: usize,
    projection: usize,
    x: f32,
}
#[derive(Default)]
struct Line {
    cursor: Cursor,
    checkpoints: Vec<Cursor>,
    rows: BTreeMap<usize, Segment>,
    complete: bool,
}
#[derive(Default)]
pub(super) struct WrapMap {
    pub enabled: bool,
    width: f32,
    #[allow(clippy::type_complexity)]
    stamp: Option<(
        u64,
        u32,
        u32,
        u32,
        bool,
        Option<(u64, u64)>,
        (u64, bool, u32, usize),
    )>,
    counts: BTreeMap<usize, usize>,
    prefix: Vec<(usize, usize)>,
    prefix_dirty: bool,
    prefix_fold: Option<u64>,
    lines: BTreeMap<usize, Line>,
    hot: BTreeMap<usize, u64>,
    checkpoint_count: usize,
    clock: u64,
    budget: usize,
    pub pending: bool,
}
impl WrapMap {
    pub fn remove_ghost(&mut self, line: usize) {
        self.prefix_dirty |= self.counts.remove(&line).is_some();
        if let Some(state) = self.lines.remove(&line) {
            self.checkpoint_count = self
                .checkpoint_count
                .saturating_sub(state.checkpoints.len());
        }
        self.hot.remove(&line);
        if let Some(stamp) = &mut self.stamp {
            stamp.5 = None;
        }
    }
    #[cfg(test)]
    pub fn counts_for_test(&mut self, line: usize, ranges: &[Range<f32>]) {
        self.counts.insert(line, ranges.len());
        self.lines.insert(
            line,
            Line {
                cursor: Cursor {
                    row: ranges.len(),
                    projection: 0,
                    x: ranges.last().map_or(0., |range| range.end),
                },
                rows: ranges
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(row, x)| {
                        (
                            row,
                            Segment {
                                projection: 0,
                                x,
                                touched: 0,
                            },
                        )
                    })
                    .collect(),
                complete: true,
                ..Line::default()
            },
        );
        self.prefix_dirty = true;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn sync(
        &mut self,
        enabled: bool,
        width: f32,
        revision: u64,
        tokens: &Tokens,
        ligatures: bool,
        ghost: Option<&ghost::Ghost>,
        layout_stamp: (u64, bool, u32, usize),
    ) -> bool {
        let width = width.max(32.);
        let stamp = (
            revision,
            width.to_bits(),
            tokens.editor_font_size.to_bits(),
            tokens.editor_letter_spacing.to_bits(),
            ligatures,
            ghost.map(|ghost| (ghost.ticket, ghost.geometry_id)),
            layout_stamp,
        );
        let changed = self.enabled != enabled || self.stamp != Some(stamp);
        if changed {
            self.counts.clear();
            self.prefix.clear();
            self.lines.clear();
            self.hot.clear();
            self.checkpoint_count = 0;
            self.prefix_dirty = true;
            self.stamp = Some(stamp);
        }
        self.enabled = enabled;
        self.width = width;
        changed
    }
    pub fn begin_frame(&mut self) {
        self.budget = FRAME_ROWS;
        self.pending = false;
        self.clock = self.clock.wrapping_add(1);
    }
    pub fn rebuild(&mut self, fold: &folding::FoldMap) {
        if !self.prefix_dirty && self.prefix_fold == Some(fold.layout_epoch()) {
            return;
        }
        let mut extra = 0;
        self.prefix.clear();
        for (&line, &rows) in &self.counts {
            if !fold.is_hidden(line) {
                extra += rows.saturating_sub(1);
                self.prefix.push((line, extra));
            }
        }
        self.prefix_dirty = false;
        self.prefix_fold = Some(fold.layout_epoch());
    }
    pub fn visual_row(&self, fold: &folding::FoldMap, line: usize) -> usize {
        let count = self.prefix.partition_point(|(source, _)| *source < line);
        fold.visual_row(line) + count.checked_sub(1).map_or(0, |i| self.prefix[i].1)
    }
    pub fn total(&self, fold: &folding::FoldMap) -> usize {
        fold.visible_line_count() + self.prefix.last().map_or(0, |(_, extra)| *extra)
    }
    pub fn source_line(&self, fold: &folding::FoldMap, row: usize) -> usize {
        let mut a = 0;
        let mut b = fold.visible_line_count();
        while a < b {
            let mid = a + (b - a) / 2;
            if self.visual_row(fold, fold.source_line(mid)) <= row {
                a = mid + 1;
            } else {
                b = mid;
            }
        }
        fold.source_line(a.saturating_sub(1))
    }
    pub fn wrapped_lines(&self) -> usize {
        self.counts.len()
    }
    pub fn bytes(&self) -> usize {
        self.prefix.capacity() * std::mem::size_of::<(usize, usize)>()
            + self.counts.len() * (std::mem::size_of::<(usize, usize)>() + 32)
            + self.lines.len() * (std::mem::size_of::<Line>() + 32)
            + self.checkpoint_count * std::mem::size_of::<Cursor>() * 2
            + self
                .hot
                .keys()
                .filter_map(|key| self.lines.get(key))
                .map(|line| line.rows.len() * (std::mem::size_of::<Segment>() + 32))
                .sum::<usize>()
    }
    /// Measure the visible region and caret first. The same budget is also used
    /// to reconstruct evicted row windows from their sparse checkpoints.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        cache: &mut layout::LineCache,
        fold: &folding::FoldMap,
        ghost: Option<&ghost::Ghost>,
        viewport: Range<usize>,
        caret: usize,
        other_points: &[usize],
        ext: &str,
        tokens: &Tokens,
    ) {
        if !self.enabled {
            return;
        }
        self.rebuild(fold);
        let first = self.source_line(fold, viewport.start);
        let last = self
            .source_line(fold, viewport.end)
            .min(snapshot.len_lines() - 1);
        for line in (first..=last).chain(
            std::iter::once(caret)
                .chain(other_points.iter().copied())
                .map(|point| snapshot.char_to_line(point.min(snapshot.len_chars()))),
        ) {
            if fold.is_hidden(line) {
                continue;
            }
            let layout = cache.get(ui, snapshot, line, ext, tokens);
            self.measure_line(
                ui,
                snapshot,
                line,
                &layout,
                ghost.filter(|g| g.line == line),
                tokens,
            );
            for caret in std::iter::once(caret)
                .chain(other_points.iter().copied())
                .filter(|point| snapshot.char_to_line((*point).min(snapshot.len_chars())) == line)
            {
                let projection = ghost.filter(|g| g.line == line);
                if let Some(x) = layout.x(ui, snapshot, tokens, caret - snapshot.line_to_char(line))
                {
                    let (x, projected) =
                        projection.map_or((x, 0), |g| g.source_position(x, x == g.split_x));
                    if let Some((_, row)) = self.position(fold, line, projected, x, false) {
                        let local = row.saturating_sub(self.visual_row(fold, line));
                        if let Some(segment) = self
                            .lines
                            .get_mut(&line)
                            .and_then(|state| state.rows.get_mut(&local))
                        {
                            segment.touched = self.clock;
                        }
                    } else {
                        let checkpoint = self
                            .lines
                            .get(&line)
                            .and_then(|state| {
                                state
                                    .checkpoints
                                    .iter()
                                    .rev()
                                    .find(|c| c.projection == projected && c.x <= x)
                            })
                            .copied();
                        if let Some(checkpoint) = checkpoint {
                            self.reconstruct(
                                ui,
                                snapshot,
                                line,
                                checkpoint.row + CHECKPOINT_ROWS - 1,
                                &layout,
                                projection,
                                tokens,
                            );
                        }
                    }
                }
            }
        }
        self.rebuild(fold);
        for row in viewport.start..viewport.end.min(self.total(fold)) {
            let line = self.source_line(fold, row);
            let local = row.saturating_sub(self.visual_row(fold, line));
            if let Some(segment) = self
                .lines
                .get_mut(&line)
                .and_then(|state| state.rows.get_mut(&local))
            {
                segment.touched = self.clock;
            }
            if self
                .lines
                .get(&line)
                .is_some_and(|line| !line.rows.contains_key(&local))
            {
                let layout = cache.get(ui, snapshot, line, ext, tokens);
                self.reconstruct(
                    ui,
                    snapshot,
                    line,
                    local,
                    &layout,
                    ghost.filter(|g| g.line == line),
                    tokens,
                );
            }
        }
        for key in self.hot.keys() {
            if let Some(state) = self.lines.get_mut(key)
                && state.rows.len() > RETAINED_ROWS
            {
                let mut ages = state
                    .rows
                    .iter()
                    .map(|(&row, segment)| (segment.touched, row))
                    .collect::<Vec<_>>();
                ages.sort_unstable();
                for (_, row) in ages.into_iter().take(state.rows.len() - RETAINED_ROWS) {
                    state.rows.remove(&row);
                }
            }
        }
        while self.hot.len() > RETAINED_LINES {
            let Some(key) = self
                .hot
                .iter()
                .min_by_key(|(_, clock)| *clock)
                .map(|(&key, _)| key)
            else {
                break;
            };
            self.hot.remove(&key);
            if let Some(line) = self.lines.get_mut(&key) {
                line.rows.clear();
            }
        }
        if self.pending {
            ui.ctx().request_repaint();
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn measure_line(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        line: usize,
        layout: &layout::LineLayout,
        ghost: Option<&ghost::Ghost>,
        tokens: &Tokens,
    ) {
        if layout.is_complete() && ghost.is_none() && layout.width() <= self.width {
            self.prefix_dirty |= self.counts.remove(&line).is_some();
            self.lines.remove(&line);
            self.hot.remove(&line);
            return;
        }
        let state = self.lines.entry(line).or_default();
        self.hot.insert(line, self.clock);
        let allowance = self.budget.saturating_sub(FRAME_ROWS * 3 / 4);
        let mut built = 0;
        while !state.complete && built < allowance {
            if state.cursor.row.is_multiple_of(CHECKPOINT_ROWS)
                && state
                    .checkpoints
                    .last()
                    .is_none_or(|cursor| cursor.row != state.cursor.row)
            {
                state.checkpoints.push(state.cursor);
                self.checkpoint_count += 1;
            }
            let Some((mut segment, next, complete)) = next_segment(
                ui,
                snapshot,
                line,
                layout,
                ghost,
                tokens,
                self.width,
                state.cursor,
            ) else {
                break;
            };
            segment.touched = self.clock;
            state.rows.insert(state.cursor.row, segment);
            state.cursor = next;
            state.complete = complete;
            self.budget -= 1;
            built += 1;
        }
        let rows = state.cursor.row.max(1) + usize::from(!state.complete);
        if rows > 1 {
            self.prefix_dirty |= self.counts.insert(line, rows) != Some(rows);
        } else {
            self.prefix_dirty |= self.counts.remove(&line).is_some();
        }
        self.pending |= !state.complete;
    }
    #[allow(clippy::too_many_arguments)]
    fn reconstruct(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        line: usize,
        row: usize,
        layout: &layout::LineLayout,
        ghost: Option<&ghost::Ghost>,
        tokens: &Tokens,
    ) {
        let Some(state) = self.lines.get_mut(&line) else {
            return;
        };
        self.hot.insert(line, self.clock);
        let Some(mut cursor) = state
            .checkpoints
            .iter()
            .rev()
            .find(|cursor| cursor.row <= row)
            .copied()
        else {
            self.pending = true;
            return;
        };
        while cursor.row <= row + 48 && self.budget > 0 && cursor.row < state.cursor.row {
            let Some((mut segment, next, _)) = next_segment(
                ui, snapshot, line, layout, ghost, tokens, self.width, cursor,
            ) else {
                break;
            };
            segment.touched = self.clock;
            state.rows.insert(cursor.row, segment);
            cursor = next;
            self.budget -= 1;
        }
        self.pending |= !state.rows.contains_key(&row);
    }
    pub fn segment(&self, line: usize, row: usize, width: f32) -> Option<Segment> {
        self.lines.get(&line).map_or_else(
            || {
                Some(Segment {
                    projection: 0,
                    x: 0. ..width,
                    touched: 0,
                })
            },
            |line| line.rows.get(&row).cloned(),
        )
    }
    pub fn position(
        &self,
        fold: &folding::FoldMap,
        line: usize,
        projection: usize,
        x: f32,
        previous: bool,
    ) -> Option<(f32, usize)> {
        let start = self.visual_row(fold, line);
        let Some(state) = self.lines.get(&line) else {
            return Some((x, start));
        };
        state
            .rows
            .iter()
            .find(|(row, segment)| {
                segment.projection == projection
                    && x >= segment.x.start
                    && (x < segment.x.end
                        || (previous && x == segment.x.end)
                        || (state.complete && segment.x.end == x && **row + 1 == state.cursor.row))
            })
            .map(|(&row, segment)| ((x - segment.x.start).max(0.), start + row))
    }
}

#[allow(clippy::too_many_arguments)]
fn next_segment(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    line: usize,
    layout: &layout::LineLayout,
    ghost: Option<&ghost::Ghost>,
    tokens: &Tokens,
    width: f32,
    cursor: Cursor,
) -> Option<(Segment, Cursor, bool)> {
    let projected_rows = ghost.map_or(1, |g| g.rows.len());
    let row_width = ghost.map_or(layout.width(), |g| {
        g.projected_width(cursor.projection, layout.width())
    });
    let width_known = layout.is_complete() || cursor.projection + 1 < projected_rows;
    if !width_known && row_width < cursor.x + width {
        return None;
    }
    let limit = (cursor.x + width).min(row_width);
    let end = if row_width <= cursor.x + width && width_known {
        row_width
    } else {
        projected_cut(
            ui,
            snapshot,
            line,
            layout,
            ghost,
            tokens,
            cursor.projection,
            cursor.x,
            limit,
        )?
    };
    if end <= cursor.x && !(layout.is_complete() && row_width == 0.) {
        return None;
    }
    let row_complete = end >= row_width && width_known;
    let complete = row_complete && cursor.projection + 1 >= projected_rows;
    let next = Cursor {
        row: cursor.row + 1,
        projection: if row_complete && !complete {
            cursor.projection + 1
        } else {
            cursor.projection
        },
        x: if row_complete && !complete { 0. } else { end },
    };
    Some((
        Segment {
            projection: cursor.projection,
            x: cursor.x..end,
            touched: 0,
        },
        next,
        complete,
    ))
}
#[allow(clippy::too_many_arguments)]
fn projected_cut(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    line: usize,
    layout: &layout::LineLayout,
    ghost: Option<&ghost::Ghost>,
    tokens: &Tokens,
    row: usize,
    start: f32,
    limit: f32,
) -> Option<f32> {
    if let Some(g) = ghost {
        let left = if row == 0 { g.split_x } else { 0. };
        let right = left + g.rows[row].size().x;
        if limit >= left && limit < right {
            return Some(g.wrap_cut(row, start, limit));
        }
        if row + 1 == g.rows.len() && limit >= right {
            let shift = g.suffix_x() - g.source_end_x;
            return source_cut(
                ui,
                snapshot,
                line,
                layout,
                tokens,
                (start - shift).max(g.source_end_x),
                limit - shift,
            )
            .map(|x| x + shift);
        }
    }
    source_cut(ui, snapshot, line, layout, tokens, start, limit)
}
fn source_cut(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    line: usize,
    layout: &layout::LineLayout,
    tokens: &Tokens,
    start: f32,
    limit: f32,
) -> Option<f32> {
    if let Some(x) = layout.hint_wrap_cut(start, limit) {
        return Some(x);
    }
    let offset = snapshot.line_to_char(line);
    let first = layout.column(ui, snapshot, tokens, start)?;
    let mut column = layout.column(ui, snapshot, tokens, limit)?;
    column = codisseu_core::previous_grapheme(
        snapshot,
        offset + column + usize::from(column < layout.len()),
    )
    .saturating_sub(offset);
    while column > first && layout.x(ui, snapshot, tokens, column)? > limit {
        column = codisseu_core::previous_grapheme(snapshot, offset + column).saturating_sub(offset);
    }
    if column <= first {
        column = codisseu_core::next_grapheme(snapshot, offset + first)
            .saturating_sub(offset)
            .min(layout.len());
    }
    let mut split = column;
    while split > first {
        if split <= first {
            break;
        }
        let previous =
            codisseu_core::previous_grapheme(snapshot, offset + split).saturating_sub(offset);
        if snapshot.char(offset + previous).is_whitespace() {
            if snapshot
                .slice(offset + first..offset + previous)
                .chars()
                .any(|c| !c.is_whitespace())
            {
                column = split;
            }
            break;
        }
        split = previous;
    }
    layout.x(ui, snapshot, tokens, column)
}

pub(super) fn projected_position(
    wrap: &WrapMap,
    fold: &folding::FoldMap,
    ghost: Option<&ghost::Ghost>,
    line: usize,
    x: f32,
    previous: bool,
) -> Option<(f32, usize)> {
    let (x, row) = ghost
        .filter(|g| g.line == line)
        .map_or((x, 0), |g| g.source_position(x, previous));
    if wrap.enabled {
        wrap.position(fold, line, row, x, previous)
    } else {
        Some((
            x,
            ghost.map_or_else(|| fold.visual_row(line), |g| g.visual_row(fold, line)) + row,
        ))
    }
}
pub(super) fn projected_caret_position(
    wrap: &WrapMap,
    fold: &folding::FoldMap,
    ghost: Option<&ghost::Ghost>,
    line: usize,
    x: f32,
    previous: bool,
) -> Option<(f32, usize)> {
    if let Some(g) = ghost.filter(|g| g.line == line && x == g.split_x) {
        if wrap.enabled {
            return wrap.position(fold, line, 0, x, previous);
        }
        return Some((x, fold.visual_row(g.line)));
    }
    projected_position(wrap, fold, ghost, line, x, previous)
}
pub(super) fn caret_position(
    view: &EditorView,
    line: usize,
    x: f32,
    previous: bool,
) -> Option<(f32, usize)> {
    projected_caret_position(
        &view.wrap,
        &view.fold,
        view.ghost.as_ref(),
        line,
        x,
        previous,
    )
}
pub(super) fn source_position(
    view: &EditorView,
    line: usize,
    x: f32,
    previous: bool,
) -> Option<(f32, usize)> {
    projected_position(
        &view.wrap,
        &view.fold,
        view.ghost.as_ref(),
        line,
        x,
        previous,
    )
}
pub(super) fn pointer_position(view: &EditorView, row: usize, x: f32) -> Option<(usize, f32)> {
    let line = if view.wrap.enabled {
        view.wrap.source_line(&view.fold, row)
    } else {
        view.ghost.as_ref().map_or_else(
            || view.fold.source_line(row),
            |g| g.source_line(&view.fold, row),
        )
    };
    let (projected_row, x) = if view.wrap.enabled {
        let local = row.saturating_sub(view.wrap.visual_row(&view.fold, line));
        let segment = view.wrap.segment(line, local, view.wrap.width)?;
        (
            view.fold.visual_row(line) + segment.projection,
            x + segment.x.start,
        )
    } else {
        (row, x)
    };
    let x = if let Some(g) = view.ghost.as_ref().filter(|g| g.line == line) {
        g.pointer(&view.fold, projected_row, x)?
    } else {
        x
    };
    Some((line, x))
}
pub(super) fn source_range(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    view: &EditorView,
    line: usize,
    row: usize,
    layout: &layout::LineLayout,
    tokens: &Tokens,
) -> Option<Range<usize>> {
    let local = row.checked_sub(view.wrap.visual_row(&view.fold, line))?;
    let segment = view.wrap.segment(line, local, layout.width())?;
    let offset = snapshot.line_to_char(line);
    let convert = |x| {
        if let Some(g) = view.ghost.as_ref().filter(|g| g.line == line) {
            match g.pointer(
                &view.fold,
                view.fold.visual_row(line) + segment.projection,
                x,
            ) {
                Some(x) => layout.column(ui, snapshot, tokens, x),
                None => Some(
                    g.source_boundary(segment.projection, x)
                        .saturating_sub(offset)
                        .min(layout.len()),
                ),
            }
        } else {
            layout.column(ui, snapshot, tokens, x)
        }
    };
    let fallback = view
        .ghost
        .as_ref()
        .filter(|g| g.line == line)
        .map_or(0, |g| g.caret - offset);
    let a = convert(segment.x.start).unwrap_or(fallback);
    let b = convert(segment.x.end).unwrap_or(a);
    Some(a..b)
}
