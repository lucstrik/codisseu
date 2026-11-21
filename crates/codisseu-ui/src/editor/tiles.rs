//! Incremental horizontal layout for unusually long logical lines.
//!
//! Prefix widths come only from real egui galleys. Unmeasured coordinates return
//! None; callers must show pending layout and retry rather than invent widths.
//! Text is never truncated. One pathological extended grapheme above 4096
//! scalars is an explicit layout error, while the document bytes remain intact.
use super::*;
use codisseu_core::{TextSnapshot, next_grapheme, previous_grapheme};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

pub(super) const TILE_CHARS: usize = 1024;
const MAX_CLUSTER_CHARS: usize = 4096;
const MAX_RETAINED_LAYOUTS: usize = 16;
const MAX_LAYOUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_ADVANCE_TILES: usize = 8;

#[derive(Clone)]
struct TileMetric {
    chars: Range<usize>,
    x: f32,
    width: f32,
    visual_column: usize,
}

pub(super) struct TileLayout {
    /// Scalar columns relative to the logical line, excluding shaping context.
    pub chars: Range<usize>,
    pub x: f32,
    pub width: f32,
    /// Draw at x - context_x, clipped to x..x+width. Neighbour context is real
    /// text used for shaping, never an assumed glyph advance.
    pub context_x: f32,
    pub galley: Arc<egui::Galley>,
    map: Vec<usize>,
    pub text: String,
}
impl TileLayout {
    pub fn x(&self, column: usize) -> f32 {
        let local = column
            .saturating_sub(self.chars.start)
            .min(self.chars.len());
        self.x
            + self
                .galley
                .pos_from_cursor(egui::text::CCursor::new(self.map[local]))
                .min
                .x
            - self.context_x
    }
    pub fn column(&self, x: f32) -> usize {
        let cursor: usize = self
            .galley
            .cursor_from_pos(egui::vec2(x - self.x + self.context_x, 0.))
            .index
            .into();
        self.chars.start
            + self
                .map
                .partition_point(|&p| p <= cursor)
                .saturating_sub(1)
                .min(self.chars.len())
    }
    fn bytes(&self) -> usize {
        // Major retained allocation capacities; egui also owns a frame cache.
        self.text.capacity()
            + self.map.capacity() * std::mem::size_of::<usize>()
            + self.galley.job.text.capacity()
            + self.galley.job.sections.capacity() * std::mem::size_of::<egui::text::LayoutSection>()
            + self.galley.rows.capacity() * std::mem::size_of::<egui::epaint::text::PlacedRow>()
            + self
                .galley
                .rows
                .iter()
                .map(|row| {
                    row.glyphs.capacity() * std::mem::size_of::<egui::epaint::text::Glyph>()
                        + row.visuals.mesh.vertices.capacity()
                            * std::mem::size_of::<egui::epaint::Vertex>()
                        + row.visuals.mesh.indices.capacity() * std::mem::size_of::<u32>()
                })
                .sum::<usize>()
    }
}

pub(super) struct TiledLine {
    ligatures: bool,
    line_start: usize,
    len: usize,
    next: usize,
    visual_column: usize,
    width: f32,
    metrics: Vec<TileMetric>,
    layouts: BTreeMap<usize, (u64, Arc<TileLayout>)>,
    clock: u64,
    lazy_budget: usize,
    error: Option<String>,
    pub built: usize,
    pub hits: usize,
}
impl TiledLine {
    #[cfg(test)]
    pub fn new(snapshot: &TextSnapshot, line: usize) -> Self {
        Self::with_ligatures(snapshot, line, true)
    }
    pub fn with_ligatures(snapshot: &TextSnapshot, line: usize, ligatures: bool) -> Self {
        let line_start = snapshot.line_to_char(line);
        let mut len = snapshot.line(line).len_chars();
        while len > 0 && matches!(snapshot.char(line_start + len - 1), '\r' | '\n') {
            len -= 1;
        }
        Self {
            ligatures,
            line_start,
            len,
            next: 0,
            visual_column: 0,
            width: 0.,
            metrics: vec![],
            layouts: BTreeMap::new(),
            clock: 0,
            lazy_budget: 8,
            error: None,
            built: 0,
            hits: 0,
        }
    }
    pub fn retarget(&mut self, snapshot: &TextSnapshot, line: usize) {
        self.line_start = snapshot.line_to_char(line);
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn width(&self) -> f32 {
        self.width
    }
    pub fn scanned_chars(&self) -> usize {
        self.next
    }
    pub fn is_complete(&self) -> bool {
        self.next == self.len
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn reset_metrics(&mut self) {
        self.lazy_budget = 8;
        self.built = 0;
        self.hits = 0;
    }
    #[cfg(test)]
    pub fn cached_tiles(&self) -> usize {
        self.layouts.len()
    }
    pub fn bytes(&self) -> usize {
        self.metrics.capacity() * std::mem::size_of::<TileMetric>()
            + self.layouts.values().map(|(_, l)| l.bytes()).sum::<usize>()
    }
    /// At most eight bounded galleys per call. Caller should advance at most once
    /// per frame and request another repaint while the line remains pending.
    pub fn advance(&mut self, ui: &egui::Ui, snapshot: &TextSnapshot, t: &Tokens, budget: usize) {
        for _ in 0..budget.min(MAX_ADVANCE_TILES) {
            if self.is_complete() || self.error.is_some() {
                break;
            }
            let Some(end) = self.next_end(snapshot) else {
                break;
            };
            let metric = TileMetric {
                chars: self.next..end,
                x: self.width,
                width: 0.,
                visual_column: self.visual_column,
            };
            let (layout, next_column) = self.build(ui, snapshot, t, &metric);
            let mut metric = metric;
            metric.width = layout.width;
            self.next = end;
            self.visual_column = next_column;
            self.width += metric.width;
            let index = self.metrics.len();
            self.metrics.push(metric);
            self.retain(index, Arc::new(layout));
            self.built += 1;
        }
    }
    fn next_end(&mut self, snapshot: &TextSnapshot) -> Option<usize> {
        let tentative = (self.next + TILE_CHARS).min(self.len);
        if tentative == self.len {
            return Some(tentative);
        }
        let mut end =
            previous_grapheme(snapshot, self.line_start + tentative + 1) - self.line_start;
        if end <= self.next {
            let boundary = bounded_cluster_end(
                snapshot,
                self.line_start + self.next,
                self.line_start + self.len,
            );
            if let Some(boundary) = boundary {
                end = boundary - self.line_start;
            } else {
                self.error = Some(
                    "Layout indisponível: grafema acima de 4096 escalares; conteúdo preservado"
                        .into(),
                );
                return None;
            }
        }
        // Prefer a whitespace cut (short look-back), preserving ordinary font
        // shaping runs. Grapheme context is retained for other boundaries.
        let floor = self.next.max(end.saturating_sub(128));
        let mut chars = snapshot.chars_at(self.line_start + end);
        for index in (floor..end).rev() {
            if chars.prev().is_some_and(char::is_whitespace) {
                return Some(index + 1);
            }
        }
        Some(end)
    }
    fn retain(&mut self, index: usize, layout: Arc<TileLayout>) {
        self.clock += 1;
        self.layouts.insert(index, (self.clock, layout));
        while self.layouts.len() > MAX_RETAINED_LAYOUTS
            || (self.layouts.len() > 1
                && self.layouts.values().map(|(_, l)| l.bytes()).sum::<usize>() > MAX_LAYOUT_BYTES)
        {
            let key = self
                .layouts
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(&key, _)| key)
                .unwrap();
            self.layouts.remove(&key);
        }
    }
    fn get(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        index: usize,
    ) -> Option<Arc<TileLayout>> {
        if let Some((at, layout)) = self.layouts.get_mut(&index) {
            self.clock += 1;
            *at = self.clock;
            self.hits += 1;
            return Some(layout.clone());
        }
        if self.lazy_budget == 0 {
            ui.ctx().request_repaint();
            return None;
        }
        self.lazy_budget -= 1;
        let (layout, _) = self.build(ui, snapshot, t, &self.metrics[index]);
        let layout = Arc::new(layout);
        self.retain(index, layout.clone());
        self.built += 1;
        Some(layout)
    }
    pub fn x(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        column: usize,
    ) -> Option<f32> {
        let column = column.min(self.len);
        if column == 0 {
            return Some(0.);
        }
        if column > self.next {
            return None;
        }
        if column == self.next {
            return Some(self.width);
        }
        let index = self.metrics.partition_point(|m| m.chars.end <= column);
        self.get(ui, snapshot, t, index)
            .map(|layout| layout.x(column))
    }
    pub fn visual_column(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        column: usize,
    ) -> Option<usize> {
        let column = column.min(self.len);
        if column == 0 {
            return Some(0);
        }
        if column > self.next {
            return None;
        }
        if column == self.next {
            return Some(self.visual_column);
        }
        let index = self
            .metrics
            .partition_point(|metric| metric.chars.end <= column);
        let visual_start = self.metrics[index].visual_column;
        let scalar_start = self.metrics[index].chars.start;
        self.get(ui, snapshot, t, index)
            .map(|layout| visual_start + layout.map[column - scalar_start] - layout.map[0])
    }
    pub fn column(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        x: f32,
    ) -> Option<usize> {
        if x <= 0. {
            return Some(0);
        }
        if x >= self.width {
            return self.is_complete().then_some(self.len);
        }
        let index = self
            .metrics
            .partition_point(|m| m.x + m.width <= x)
            .min(self.metrics.len() - 1);
        self.get(ui, snapshot, t, index)
            .map(|layout| layout.column(x))
    }
    pub fn visible(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        x: Range<f32>,
    ) -> Vec<Arc<TileLayout>> {
        let first = self.metrics.partition_point(|m| m.x + m.width < x.start);
        let last = self.metrics.partition_point(|m| m.x < x.end);
        // Usual viewport intersects one or two tiles. The cap also bounds a
        // pathological zoom-out frame; callers can report pending beyond it.
        (first..last.min(first + MAX_RETAINED_LAYOUTS))
            .filter_map(|i| self.get(ui, snapshot, t, i))
            .collect()
    }
    fn build(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        metric: &TileMetric,
    ) -> (TileLayout, usize) {
        let start = self.line_start + metric.chars.start;
        let end = self.line_start + metric.chars.end;
        let left = if metric.chars.start > 0 {
            previous_grapheme(snapshot, start)
        } else {
            start
        };
        let left = if start - left <= MAX_CLUSTER_CHARS
            && !snapshot.slice(left..start).chars().any(char::is_whitespace)
        {
            left
        } else {
            start
        };
        let right = if metric.chars.end < self.len {
            bounded_cluster_end(snapshot, end, self.line_start + self.len).unwrap_or(end)
        } else {
            end
        };
        let text = snapshot.slice(start..end).to_string();
        let mut rendered = String::new();
        let mut map = Vec::with_capacity(metric.chars.len() + 1);
        let mut column = metric.visual_column.saturating_sub(start - left);
        let mut rendered_column = 0;
        for (offset, c) in snapshot.slice(left..right).chars().enumerate() {
            let position = left + offset;
            if position >= start && position <= end {
                map.push(rendered_column);
            }
            if c == '\t' {
                let n = 4 - column % 4;
                rendered.extend(std::iter::repeat_n(' ', n));
                column += n;
                rendered_column += n;
            } else {
                rendered.push(c);
                column += 1;
                rendered_column += 1;
            }
        }
        if right == end {
            map.push(rendered_column);
        }
        let next_column = metric.visual_column + map.last().copied().unwrap_or(0)
            - map.first().copied().unwrap_or(0);
        // Long-line highlighting is deliberately plain, matching the existing
        // lexical highlighter's large-input fallback instead of false tokens.
        let mut job = highlight::layout_themed(
            &rendered,
            "",
            FontId::monospace(t.editor_font_size),
            t.theme,
        );
        super::layout::prepare_job(&mut job, t, self.ligatures);
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        let context_x = galley
            .pos_from_cursor(egui::text::CCursor::new(map[0]))
            .min
            .x;
        let width = galley
            .pos_from_cursor(egui::text::CCursor::new(*map.last().unwrap()))
            .min
            .x
            - context_x;
        (
            TileLayout {
                chars: metric.chars.clone(),
                x: metric.x,
                width,
                context_x,
                galley,
                map,
                text,
            },
            next_column,
        )
    }
}

/// Probe only a bounded prefix before asking for the next complete cluster.
/// A huge combining sequence must not be scanned to its far end on the GUI.
fn bounded_cluster_end(snapshot: &TextSnapshot, start: usize, line_end: usize) -> Option<usize> {
    if line_end - start <= MAX_CLUSTER_CHARS {
        return Some(next_grapheme(snapshot, start).min(line_end));
    }
    let boundary = previous_grapheme(snapshot, start + MAX_CLUSTER_CHARS + 1);
    if boundary <= start {
        None
    } else {
        Some(next_grapheme(snapshot, start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn with_ui(mut run: impl FnMut(&egui::Ui, &Tokens)) {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        context
            .run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| run(ui, &tokens));
            })
            .textures_delta
            .clear();
    }
    #[test]
    fn long_line_advances_bounded_tiles_and_unknown_coordinates_stay_pending() {
        let snapshot = TextSnapshot::from_str(&"let weather = 'ação'; ".repeat(20_000));
        let mut tiles = TiledLine::new(&snapshot, 0);
        with_ui(|ui, tokens| {
            assert_eq!(tiles.x(ui, &snapshot, tokens, tiles.len()), None);
            tiles.advance(ui, &snapshot, tokens, usize::MAX);
            assert!(tiles.scanned_chars() > 0);
            assert!(tiles.scanned_chars() <= MAX_ADVANCE_TILES * TILE_CHARS);
            assert!(!tiles.is_complete());
            assert_eq!(tiles.x(ui, &snapshot, tokens, tiles.len()), None);
            assert_eq!(
                tiles.column(ui, &snapshot, tokens, tiles.width() + 100.),
                None
            );
            let first = tiles.visible(ui, &snapshot, tokens, 0.0..1000.0);
            assert_eq!(first[0].chars.start, 0);
            for tile in first {
                for column in [tile.chars.start, tile.chars.end] {
                    let x = tiles.x(ui, &snapshot, tokens, column).unwrap();
                    assert_eq!(tiles.column(ui, &snapshot, tokens, x), Some(column));
                }
            }
            assert!(tiles.bytes() < 4 * 1024 * 1024);
        });
    }
    #[test]
    fn tile_boundaries_preserve_graphemes_tabs_and_real_measured_widths() {
        let source = format!("{}\te\u{301}👩🏽‍💻🇧🇷 AV ffi", "a".repeat(1019));
        let snapshot = TextSnapshot::from_str(&source);
        let mut tiles = TiledLine::new(&snapshot, 0);
        with_ui(|ui, tokens| {
            tiles.advance(ui, &snapshot, tokens, MAX_ADVANCE_TILES);
            assert!(tiles.is_complete());
            let all = tiles.visible(ui, &snapshot, tokens, 0.0..f32::INFINITY);
            assert_eq!(
                all.iter()
                    .map(|tile| tile.text.as_str())
                    .collect::<String>(),
                source
            );
            let mut end = 0;
            for tile in &all {
                assert_eq!(tile.chars.start, end);
                assert_eq!(
                    previous_grapheme(&snapshot, next_grapheme(&snapshot, tile.chars.start)),
                    tile.chars.start
                );
                end = tile.chars.end;
            }
            assert_eq!(end, snapshot.len_chars());
            // The same font/style and tab expansion policy as the ordinary line.
            let mut rendered = String::new();
            let mut column = 0;
            for c in source.chars() {
                if c == '\t' {
                    let n = 4 - column % 4;
                    rendered.extend(std::iter::repeat_n(' ', n));
                    column += n;
                } else {
                    rendered.push(c);
                    column += 1;
                }
            }
            let mut job =
                highlight::layout(&rendered, "", FontId::monospace(tokens.editor_font_size));
            for section in &mut job.sections {
                section.format.line_height = Some(tokens.editor_line_height);
                section.format.extra_letter_spacing = tokens.editor_letter_spacing;
            }
            let full = ui.fonts_mut(|fonts| fonts.layout_job(job));
            assert!(
                (tiles.width() - full.size().x).abs() <= 1.,
                "tiled={} full={}",
                tiles.width(),
                full.size().x
            );
            assert_eq!(
                tiles.x(ui, &snapshot, tokens, snapshot.len_chars()),
                Some(tiles.width())
            );
        });
    }
    #[test]
    fn oversized_cluster_is_explicit_and_never_materialized_or_truncated() {
        let snapshot =
            TextSnapshot::from_str(&format!("e{}", "\u{301}".repeat(MAX_CLUSTER_CHARS + 10)));
        let mut tiles = TiledLine::new(&snapshot, 0);
        with_ui(|ui, tokens| {
            tiles.advance(ui, &snapshot, tokens, MAX_ADVANCE_TILES);
            assert!(tiles.error().unwrap().contains("4096"));
            assert_eq!(tiles.scanned_chars(), 0);
            assert!(!tiles.is_complete());
            assert_eq!(tiles.cached_tiles(), 0);
            assert_eq!(snapshot.len_chars(), MAX_CLUSTER_CHARS + 11);
        });
    }
    #[test]
    fn visual_columns_carry_tab_stops_across_tiles_and_wait_for_unmeasured_columns() {
        let source = format!(
            "{}\tx{}\t{}",
            "a".repeat(1023),
            "a".repeat(1021),
            "x".repeat(10_000)
        );
        let snapshot = TextSnapshot::from_str(&source);
        let mut tiles = TiledLine::new(&snapshot, 0);
        with_ui(|ui, tokens| {
            assert_eq!(
                tiles.visual_column(ui, &snapshot, tokens, snapshot.len_chars()),
                None
            );
            tiles.advance(ui, &snapshot, tokens, MAX_ADVANCE_TILES);
            assert_eq!(tiles.visual_column(ui, &snapshot, tokens, 1023), Some(1023));
            assert_eq!(tiles.visual_column(ui, &snapshot, tokens, 1024), Some(1024));
            assert_eq!(tiles.visual_column(ui, &snapshot, tokens, 2046), Some(2046));
            assert_eq!(tiles.visual_column(ui, &snapshot, tokens, 2047), Some(2048));
            assert_eq!(
                tiles.visual_column(ui, &snapshot, tokens, snapshot.len_chars()),
                None
            );
            let count = tiles.scanned_chars();
            let visual = tiles.visual_column(ui, &snapshot, tokens, count).unwrap();
            assert_eq!(visual, count + 1);
        });
    }
}
