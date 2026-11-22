//! Bounded vertical and horizontal layout cache. No whole-document String.
use super::*;
use codisseu_core::{TextChange, TextSnapshot};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    ops::Range,
    rc::Rc,
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

/// epaint 0.37 shapes each section independently and exposes no feature switches.
/// Isolate ASCII graphemes to disable programming ligatures without rewriting text
/// or breaking Unicode clusters/connected non-ASCII scripts. This is applied only
/// to bounded editor lines/tiles, never to the UI or whole documents.
pub(super) fn prepare_job(job: &mut egui::text::LayoutJob, t: &Tokens, ligatures: bool) {
    if ligatures {
        for section in &mut job.sections {
            section.format.line_height = Some(t.editor_line_height);
            section.format.extra_letter_spacing = t.editor_letter_spacing;
        }
        job.wrap.max_width = f32::INFINITY;
        return;
    }
    let mut sections = Vec::new();
    for mut section in std::mem::take(&mut job.sections) {
        section.format.line_height = Some(t.editor_line_height);
        section.format.extra_letter_spacing = t.editor_letter_spacing;
        let range = section.byte_range.clone();
        let mut start = range.start;
        for (offset, cluster) in job.text[range.start.0..range.end.0].grapheme_indices(true) {
            if cluster.len() == 1 && cluster.is_ascii() {
                let index = range.start + offset;
                if start < index {
                    let mut part = section.clone();
                    part.byte_range = start..index;
                    sections.push(part);
                }
                let mut part = section.clone();
                part.byte_range = index..index + 1;
                sections.push(part);
                start = index + 1;
            }
        }
        if start < range.end {
            section.byte_range = start..range.end;
            sections.push(section);
        }
    }
    job.sections = sections;
    job.wrap.max_width = f32::INFINITY;
}

const FLAT_LINE_CHARS: usize = 2048;
const FRAME_TILE_BUDGET: usize = 8;
const CACHE_BYTES: usize = 4 * 1024 * 1024;
struct FlatLine {
    text: String,
    map: Vec<usize>,
    visual_map: Vec<usize>,
    hints: Vec<inlays::Span>,
    hint_bounds: RefCell<BTreeMap<usize, (u64, egui::Rect)>>,
    galley: Arc<egui::Galley>,
}
enum Content {
    Flat(FlatLine),
    Tiled(RefCell<tiles::TiledLine>),
}
pub(super) struct LineInk<'a> {
    pub tokens: &'a Tokens,
    pub unnecessary: &'a [Range<usize>],
}
pub(super) struct LineLayout {
    content: Content,
    source_start: Cell<usize>,
}
impl LineLayout {
    /// Hints have display glyphs without source scalar positions. Break those
    /// decorated galleys at their real glyph/word boundaries as well, rather
    /// than treating a whole hint and the adjacent source glyph as one unit.
    pub fn hint_wrap_cut(&self, start: f32, limit: f32) -> Option<f32> {
        let Content::Flat(flat) = &self.content else {
            return None;
        };
        if flat.hints.is_empty() {
            return None;
        }
        let mut scalar = 0;
        let mut cut = start;
        let mut word = None;
        let mut content = false;
        for cluster in flat.galley.job.text.graphemes(true) {
            scalar += cluster.chars().count();
            let x = flat
                .galley
                .pos_from_cursor(egui::text::CCursor::new(scalar))
                .min
                .x;
            if x <= start {
                continue;
            }
            if x > limit && cut > start {
                break;
            }
            cut = x;
            if cluster.chars().all(char::is_whitespace) && content {
                word = Some(x);
            } else if !cluster.chars().all(char::is_whitespace) {
                content = true;
            }
            if x >= limit {
                break;
            }
        }
        Some(word.unwrap_or(cut))
    }
    pub fn len(&self) -> usize {
        match &self.content {
            Content::Flat(f) => f.map.len().saturating_sub(1),
            Content::Tiled(t) => t.borrow().len(),
        }
    }
    pub fn width(&self) -> f32 {
        match &self.content {
            Content::Flat(f) => f.galley.size().x,
            Content::Tiled(t) => t.borrow().width(),
        }
    }
    pub fn is_complete(&self) -> bool {
        match &self.content {
            Content::Flat(_) => true,
            Content::Tiled(t) => t.borrow().is_complete(),
        }
    }
    pub fn scanned_chars(&self) -> usize {
        match &self.content {
            Content::Flat(_) => self.len(),
            Content::Tiled(t) => t.borrow().scanned_chars(),
        }
    }
    pub fn pending_error(&self) -> Option<String> {
        match &self.content {
            Content::Flat(_) => None,
            Content::Tiled(t) => t.borrow().error().map(str::to_owned),
        }
    }
    pub fn x(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        column: usize,
    ) -> Option<f32> {
        match &self.content {
            Content::Flat(f) => Some(
                f.galley
                    .pos_from_cursor(egui::text::CCursor::new(f.map[column.min(self.len())]))
                    .min
                    .x,
            ),
            Content::Tiled(tiles) => tiles.borrow_mut().x(ui, snapshot, t, column),
        }
    }
    /// Zero-based Codisseu visual column: one per scalar, tabs to the next stop of four.
    /// This is distinct from UTF-16 protocol positions and measured pixel widths.
    pub fn visual_column(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        column: usize,
    ) -> Option<usize> {
        match &self.content {
            Content::Flat(flat) => Some(flat.visual_map[column.min(self.len())]),
            Content::Tiled(tiles) => tiles.borrow_mut().visual_column(ui, snapshot, t, column),
        }
    }
    pub fn column(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        x: f32,
    ) -> Option<usize> {
        match &self.content {
            Content::Flat(f) => {
                let i: usize = f.galley.cursor_from_pos(egui::vec2(x, 0.)).index.into();
                if let Some(hint) = f.hints.iter().find(|h| h.display.contains(&i)) {
                    return Some(hint.column);
                }
                Some(
                    f.map
                        .partition_point(|&v| v <= i)
                        .saturating_sub(1)
                        .min(self.len()),
                )
            }
            Content::Tiled(tiles) => tiles.borrow_mut().column(ui, snapshot, t, x),
        }
    }
    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        snapshot: &TextSnapshot,
        ink: LineInk<'_>,
        painter: &egui::Painter,
        origin: egui::Pos2,
        viewport_x: Range<f32>,
    ) {
        let LineInk {
            tokens: t,
            unnecessary,
        } = ink;
        match &self.content {
            Content::Flat(f) => {
                let xs = unnecessary
                    .iter()
                    .map(|r| {
                        let x = |i: usize| {
                            f.galley
                                .pos_from_cursor(egui::text::CCursor::new(f.map[i.min(self.len())]))
                                .min
                                .x
                                + origin.x
                        };
                        x(r.start)..x(r.end)
                    })
                    .collect::<Vec<_>>();
                paint_decorated(painter, origin, f.galley.clone(), t, &xs);
                for (index, hint) in f.hints.iter().enumerate() {
                    let a = f
                        .galley
                        .pos_from_cursor(egui::text::CCursor::new(hint.display.start))
                        .min
                        .x;
                    let b = f
                        .galley
                        .pos_from_cursor(egui::text::CCursor::new(hint.display.end))
                        .min
                        .x;
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(a, 0.),
                        egui::vec2((b - a).max(1.), t.editor_line_height),
                    )
                    .intersect(painter.clip_rect());
                    if rect.is_positive() {
                        let response = ui.interact(
                            rect,
                            ui.scope_id().with((
                                "rust.inlay.hit",
                                self.source_start.get(),
                                origin.x.to_bits(),
                                origin.y.to_bits(),
                                index,
                            )),
                            egui::Sense::hover(),
                        );
                        let label = ui.scope_id().with((
                            "rust.inlay.label",
                            self.source_start.get(),
                            index,
                        ));
                        let pass = ui.ctx().cumulative_pass_nr();
                        let (first, bounds) = {
                            let mut map = f.hint_bounds.borrow_mut();
                            let previous = map.get(&index).copied();
                            let first = previous.is_none_or(|(old, _)| old != pass);
                            let bounds = previous
                                .filter(|(old, _)| *old == pass)
                                .map_or(rect, |(_, old)| old.union(rect));
                            map.insert(index, (pass, bounds));
                            (first, bounds)
                        };
                        if first {
                            let mut label_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .scope_id(label)
                                    .max_rect(bounds)
                                    .accessibility_role(egui::accesskit::Role::Label),
                            );
                            label_ui.expand_to_include_rect(bounds);
                        }
                        ui.ctx().accesskit_node_builder(label, |node| {
                            node.set_role(egui::accesskit::Role::Label);
                            node.set_value(hint.tooltip.clone());
                            node.set_bounds(egui::accesskit::Rect {
                                x0: bounds.left() as f64,
                                y0: bounds.top() as f64,
                                x1: bounds.right() as f64,
                                y1: bounds.bottom() as f64,
                            });
                        });
                        response.on_hover_text(&hint.tooltip);
                    }
                }
            }
            Content::Tiled(tiles) => {
                for tile in tiles.borrow_mut().visible(ui, snapshot, t, viewport_x) {
                    let clip = egui::Rect::from_min_max(
                        egui::pos2(origin.x + tile.x, origin.y),
                        egui::pos2(
                            origin.x + tile.x + tile.width,
                            origin.y + t.editor_line_height,
                        ),
                    );
                    let xs = unnecessary
                        .iter()
                        .filter_map(|r| {
                            let a = r.start.max(tile.chars.start);
                            let b = r.end.min(tile.chars.end);
                            (a < b).then(|| (origin.x + tile.x(a))..(origin.x + tile.x(b)))
                        })
                        .collect::<Vec<_>>();
                    paint_decorated(
                        &painter.with_clip_rect(painter.clip_rect().intersect(clip)),
                        egui::pos2(origin.x + tile.x - tile.context_x, origin.y),
                        tile.galley.clone(),
                        t,
                        &xs,
                    );
                }
            }
        }
    }
    /// Accessibility receives bounded windows with their real scalar ranges.
    pub fn visible_text(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        t: &Tokens,
        viewport_x: Range<f32>,
    ) -> Vec<(Range<usize>, String)> {
        match &self.content {
            Content::Flat(f) => vec![(0..self.len(), f.text.clone())],
            Content::Tiled(tiles) => tiles
                .borrow_mut()
                .visible(ui, snapshot, t, viewport_x)
                .into_iter()
                .map(|tile| (tile.chars.clone(), tile.text.clone()))
                .collect(),
        }
    }
    fn advance(
        &self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        line: usize,
        t: &Tokens,
        budget: usize,
    ) -> usize {
        if let Content::Tiled(tiles) = &self.content {
            let mut tiles = tiles.borrow_mut();
            tiles.retarget(snapshot, line);
            let before = tiles.built;
            tiles.advance(ui, snapshot, t, budget);
            tiles.built - before
        } else {
            0
        }
    }
    pub fn bytes(&self) -> usize {
        match &self.content {
            Content::Flat(f) => {
                f.text.capacity()
                    + (f.map.capacity() + f.visual_map.capacity()) * std::mem::size_of::<usize>()
                    + f.hints.iter().map(|h| h.tooltip.capacity()).sum::<usize>()
                    + f.hint_bounds.borrow().len() * 64
                    + f.galley.job.text.capacity()
                    + f.galley.job.sections.capacity()
                        * std::mem::size_of::<egui::text::LayoutSection>()
                    + f.galley.rows.capacity()
                        * std::mem::size_of::<egui::epaint::text::PlacedRow>()
                    + f.galley
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
            Content::Tiled(tiles) => tiles.borrow().bytes(),
        }
    }
}

/// Disjoint strips draw the original shaped galley exactly once at every pixel.
/// This changes only ink color: no reshaping, duplicate overlay, cache growth,
/// scalar geometry or selection/accessibility changes.
fn paint_decorated(
    painter: &egui::Painter,
    origin: egui::Pos2,
    galley: Arc<egui::Galley>,
    t: &Tokens,
    spans: &[Range<f32>],
) {
    let clip = painter.clip_rect();
    let mut left = clip.left();
    for span in spans {
        if !span.start.is_finite() || !span.end.is_finite() || span.start >= span.end {
            continue;
        }
        let start = span.start.clamp(left, clip.right());
        let end = span.end.clamp(start, clip.right());
        if start > left {
            let mut normal = clip;
            normal.min.x = left;
            normal.max.x = start;
            painter
                .with_clip_rect(normal)
                .galley(origin, galley.clone(), color(t.text));
        }
        if end > start {
            let mut faded = clip;
            faded.min.x = start;
            faded.max.x = end;
            painter
                .with_clip_rect(faded)
                .galley_with_override_text_color(
                    origin,
                    galley.clone(),
                    color(t.editor_unnecessary_text),
                );
        }
        left = end;
    }
    if left < clip.right() {
        let mut normal = clip;
        normal.min.x = left;
        painter
            .with_clip_rect(normal)
            .galley(origin, galley, color(t.text));
    }
}

#[derive(Default)]
pub(super) struct LineCache {
    disable_ligatures: bool,
    inlay_stamp: Option<(u64, u64)>,
    inlays: BTreeMap<usize, Vec<inlays::Hint>>,
    lines: BTreeMap<usize, Rc<LineLayout>>,
    revision: Option<u64>,
    style: Option<([u32; 4], codisseu_core::ThemePreference)>,
    font_epoch: Option<Arc<egui::Galley>>,
    lexical: lexical::Lexer,
    remaining_tiles: usize,
    flat_built: usize,
    pub built: usize,
    pub hits: usize,
}
impl LineCache {
    pub fn geometry_epoch(&self) -> usize {
        self.font_epoch
            .as_ref()
            .map_or(0, |epoch| Arc::as_ptr(epoch) as usize)
    }
    pub fn set_inlays(
        &mut self,
        source_revision: u64,
        hint_revision: u64,
        hints: BTreeMap<usize, Vec<inlays::Hint>>,
    ) {
        let stamp = Some((source_revision, hint_revision));
        if self.inlay_stamp != stamp || self.inlays != hints {
            self.lines
                .retain(|line, _| !self.inlays.contains_key(line) && !hints.contains_key(line));
            self.inlays = hints;
            self.inlay_stamp = stamp;
        }
    }
    pub fn set_ligatures(&mut self, enabled: bool) {
        let disabled = !enabled;
        if self.disable_ligatures != disabled {
            self.lines.clear();
            self.disable_ligatures = disabled;
        }
    }
    pub fn begin_document(
        &mut self,
        snapshot: TextSnapshot,
        revision: u64,
        change: Option<&TextChange>,
        ext: &str,
    ) {
        if let Some(from) = self.lexical.sync(snapshot, revision, change, ext) {
            self.lines.retain(|&line, _| line < from);
        }
    }
    pub fn lexical_state(&self, line: usize, ext: &str) -> Option<lexical::State> {
        self.lexical.context_state(line, ext)
    }
    pub fn lexical_pending(&self) -> bool {
        self.lexical.pending()
    }
    pub fn lexical_error(&self) -> Option<&str> {
        self.lexical.error()
    }
    pub fn begin_frame(&mut self) {
        self.remaining_tiles = FRAME_TILE_BUDGET;
        self.built = 0;
        self.flat_built = 0;
        self.hits = 0;
        for line in self.lines.values() {
            if let Content::Tiled(tiles) = &line.content {
                tiles.borrow_mut().reset_metrics();
            }
        }
    }
    pub fn finish_frame(&mut self) {
        self.built = self.flat_built
            + self
                .lines
                .values()
                .map(|line| {
                    if let Content::Tiled(tiles) = &line.content {
                        tiles.borrow().built
                    } else {
                        0
                    }
                })
                .sum::<usize>();
    }
    pub fn sync(&mut self, revision: u64, change: Option<&TextChange>, ui: &egui::Ui, t: &Tokens) {
        if self
            .inlay_stamp
            .is_some_and(|(source, _)| source != revision)
            && !self.inlays.is_empty()
        {
            self.lines.retain(|line, _| !self.inlays.contains_key(line));
            self.inlays.clear();
            self.inlay_stamp = None;
        }
        // Galleys contain atlas UVs, not just geometry. egui recreates its atlas
        // after font/options changes and when it fills up, without changing DPI
        // or document revision. A held galley requested on every frame is an
        // epoch witness: egui clears its own galley cache on each such reset.
        // Holding the Arc also prevents address reuse from hiding a reset.
        let epoch = ui.fonts_mut(|fonts| {
            fonts.layout_no_wrap(
                "M".into(),
                FontId::monospace(t.editor_font_size),
                Color32::WHITE,
            )
        });
        if self
            .font_epoch
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &epoch))
        {
            self.lines.clear();
        }
        self.font_epoch = Some(epoch);
        let style = (
            [
                t.editor_font_size.to_bits(),
                t.editor_line_height.to_bits(),
                t.editor_letter_spacing.to_bits(),
                ui.ctx().pixels_per_point().to_bits(),
            ],
            t.theme,
        );
        if self.style != Some(style) {
            self.lines.clear();
            self.style = Some(style);
        }
        if self.revision == Some(revision) {
            return;
        }
        if let (Some(previous), Some(c)) = (self.revision, change)
            && c.revision == previous + 1
            && c.revision == revision
        {
            let shift = c.new_lines.len() as isize - c.old_lines.len() as isize;
            self.lines = std::mem::take(&mut self.lines)
                .into_iter()
                .filter_map(|(i, v)| {
                    if i < c.old_lines.start {
                        Some((i, v))
                    } else if i >= c.old_lines.end {
                        Some((i.saturating_add_signed(shift), v))
                    } else {
                        None
                    }
                })
                .collect();
        } else {
            self.lines.clear();
        }
        self.revision = Some(revision);
    }
    pub fn get(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        line: usize,
        ext: &str,
        t: &Tokens,
    ) -> Rc<LineLayout> {
        let value = if let Some(value) = self.lines.get(&line) {
            self.hits += 1;
            value.clone()
        } else {
            let raw = snapshot.line(line);
            let value = if raw.len_chars() > FLAT_LINE_CHARS {
                LineLayout {
                    source_start: Cell::new(snapshot.line_to_char(line)),
                    content: Content::Tiled(RefCell::new(tiles::TiledLine::with_ligatures(
                        snapshot,
                        line,
                        !self.disable_ligatures,
                    ))),
                }
            } else {
                let text = raw.to_string().trim_end_matches(['\r', '\n']).to_owned();
                let mut rendered = String::with_capacity(text.len());
                let mut map = vec![0];
                let mut column = 0;
                for c in text.chars() {
                    if c == '\t' {
                        let n = 4 - column % 4;
                        rendered.extend(std::iter::repeat_n(' ', n));
                        column += n;
                    } else {
                        rendered.push(c);
                        column += 1;
                    }
                    map.push(column);
                }
                let mut job = lexical::layout_themed(
                    &rendered,
                    ext,
                    FontId::monospace(t.editor_font_size),
                    self.lexical.state(line),
                    t.theme,
                );
                let visual_map = map.clone();
                let (decorated, positions, hints) = inlays::decorate(
                    job,
                    &map,
                    self.inlays.get(&line).map(Vec::as_slice).unwrap_or(&[]),
                    t,
                );
                job = decorated;
                map = positions;
                prepare_job(&mut job, t, !self.disable_ligatures);
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                self.built += 1;
                self.flat_built += 1;
                LineLayout {
                    source_start: Cell::new(snapshot.line_to_char(line)),
                    content: Content::Flat(FlatLine {
                        text,
                        map,
                        visual_map,
                        hints,
                        hint_bounds: RefCell::new(BTreeMap::new()),
                        galley,
                    }),
                }
            };
            let value = Rc::new(value);
            self.lines.insert(line, value.clone());
            value
        };
        value.source_start.set(snapshot.line_to_char(line));
        let built = value.advance(ui, snapshot, line, t, self.remaining_tiles);
        self.remaining_tiles = self.remaining_tiles.saturating_sub(built);
        self.built += built;
        while self.lines.len() > 256 || self.bytes() > CACHE_BYTES {
            let Some(key) = self.lines.keys().copied().find(|&key| key != line) else {
                break;
            };
            self.lines.remove(&key);
        }
        if !value.is_complete() && value.pending_error().is_none() {
            ui.ctx().request_repaint();
        }
        value
    }
    pub fn inlay_count(&self, lines: Range<usize>) -> usize {
        self.inlays.range(lines).map(|(_, hints)| hints.len()).sum()
    }
    pub fn bytes(&self) -> usize {
        self.lines.values().map(|l| l.bytes()).sum()
    }
    pub fn width(&self) -> f32 {
        self.lines.values().map(|l| l.width()).fold(0., f32::max)
    }
    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_theme_rebuilds_cached_lines_without_a_document_edit_or_font_reset() {
        let ctx = egui::Context::default();
        let classic = Tokens::for_theme(codisseu_core::ThemePreference::Classic);
        let midnight = Tokens::for_theme(codisseu_core::ThemePreference::Midnight);
        classic.apply(&ctx);
        let source = "const answer = 42;";
        let snapshot = TextSnapshot::from_str(source);
        let mut cache = LineCache::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            cache.begin_document(snapshot.clone(), 0, None, "ts");
            cache.sync(0, None, ui, &classic);
            let old = cache.get(ui, &snapshot, 0, "ts", &classic);
            cache.sync(0, None, ui, &midnight);
            let new = cache.get(ui, &snapshot, 0, "ts", &midnight);
            assert!(!Rc::ptr_eq(&old, &new));
            let (Content::Flat(old), Content::Flat(new)) = (&old.content, &new.content) else {
                panic!("flat lines");
            };
            assert_ne!(
                old.galley.job.sections[0].format.color,
                new.galley.job.sections[0].format.color
            );
            assert_eq!(new.galley.job.text, source);
            assert_eq!(snapshot.to_string(), source);
        });
        output.textures_delta.clear();
    }
    #[test]
    fn decoration_paint_partitions_existing_flat_and_tiled_galleys_without_relayout() {
        for tiled in [false, true] {
            let context = egui::Context::default();
            let tokens = Tokens::load();
            tokens.apply(&context);
            let text = if tiled {
                "const unused = 42; ".repeat(300)
            } else {
                "const unused = 42;".into()
            };
            let snapshot = TextSnapshot::from_str(&text);
            let mut cache = LineCache::default();
            let mut expected = None;
            let mut output = context.run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    cache.begin_frame();
                    cache.sync(0, None, ui, &tokens);
                    let line = cache.get(ui, &snapshot, 0, "ts", &tokens);
                    let origin = egui::pos2(30., 30.);
                    expected = Some((
                        origin.x + line.x(ui, &snapshot, &tokens, 6).unwrap(),
                        origin.x + line.x(ui, &snapshot, &tokens, 12).unwrap(),
                    ));
                    let before = cache.built;
                    let painter = ui.painter().clone();
                    line.paint(
                        ui,
                        &snapshot,
                        LineInk {
                            tokens: &tokens,
                            unnecessary: std::slice::from_ref(&(6..12)),
                        },
                        &painter,
                        origin,
                        0.0..400.0,
                    );
                    assert_eq!(
                        cache.built, before,
                        "color decoration does not build another layout"
                    );
                });
            });
            output.textures_delta.clear();
            let (a, b) = expected.unwrap();
            let shapes: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|clipped| {
                    if let egui::Shape::Text(text) = &clipped.shape {
                        Some((clipped.clip_rect, text))
                    } else {
                        None
                    }
                })
                .collect();
            let faded: Vec<_> = shapes
                .iter()
                .filter(|(_, s)| {
                    s.override_text_color == Some(color(tokens.editor_unnecessary_text))
                })
                .collect();
            assert_eq!(faded.len(), 1);
            assert_eq!(faded[0].0.left(), a);
            assert_eq!(faded[0].0.right(), b);
            for (clip, shape) in &shapes {
                if shape.override_text_color.is_none() {
                    assert!(
                        clip.right() <= a || clip.left() >= b,
                        "normal and faded clips are disjoint"
                    );
                }
            }
            assert!(cache.bytes() <= CACHE_BYTES);
        }
    }
    #[test]
    fn toggling_ligatures_invalidates_both_layout_kinds_and_off_has_ascii_caret_cells() {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        let ascii = "// office ffi -> => != === <=";
        let snapshot = TextSnapshot::from_str(&format!("{ascii}\n{}", "-> ffi ".repeat(500)));
        let mut cache = LineCache::default();
        let mut previous: Option<(Rc<LineLayout>, Rc<LineLayout>)> = None;
        for enabled in [true, false, true] {
            context
                .run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        cache.set_ligatures(enabled);
                        cache.begin_frame();
                        cache.sync(0, None, ui, &tokens);
                        let flat = cache.get(ui, &snapshot, 0, "", &tokens);
                        let tiled = cache.get(ui, &snapshot, 1, "", &tokens);
                        if let Some((old_flat, old_tiled)) = &previous {
                            assert!(!Rc::ptr_eq(old_flat, &flat));
                            assert!(!Rc::ptr_eq(old_tiled, &tiled));
                        }
                        if !enabled {
                            for column in 0..=ascii.len() {
                                let x = flat.x(ui, &snapshot, &tokens, column).unwrap();
                                assert!((x - column as f32 * 8.0).abs() < 0.25);
                                assert_eq!(flat.column(ui, &snapshot, &tokens, x), Some(column));
                            }
                            for column in 0..=50 {
                                assert_eq!(
                                    tiled.x(ui, &snapshot, &tokens, column),
                                    Some(column as f32 * 8.0)
                                );
                            }
                        }
                        assert!(cache.bytes() <= CACHE_BYTES);
                        previous = Some((flat, tiled));
                    });
                })
                .textures_delta
                .clear();
        }
    }
    #[test]
    fn no_ligatures_keeps_unicode_clusters_connected_scripts_and_style_ranges() {
        let text = "// e\u{301} 👩‍💻 中文 العربية ffi ->";
        let mut job = highlight::layout(text, "", FontId::monospace(13.0));
        let t = Tokens::load();
        prepare_job(&mut job, &t, false);
        assert_eq!(job.text, text);
        let ranges: Vec<_> = job
            .sections
            .iter()
            .map(|s| s.byte_range.start.0..s.byte_range.end.0)
            .collect();
        let mut end = 0;
        for range in &ranges {
            assert_eq!(range.start, end);
            end = range.end;
        }
        assert_eq!(end, text.len());
        for cluster in ["e\u{301}", "👩‍💻", "العربية"] {
            let start = text.find(cluster).unwrap();
            let end = start + cluster.len();
            assert!(
                ranges.iter().any(|r| r.start <= start && r.end >= end),
                "{cluster} was split"
            );
        }
    }
    #[test]
    fn atlas_recreation_invalidates_flat_and_tiled_uvs_without_a_document_edit() {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        let snapshot = TextSnapshot::from_str(&format!("<?php é 🌧\n{}", "wide ".repeat(2000)));
        let mut cache = LineCache::default();
        let mut previous: Option<(Rc<LineLayout>, Rc<LineLayout>)> = None;
        for pass in 0..4 {
            if pass == 2 {
                context.global_style_mut(|s| s.visuals.text_options.font_hinting = false);
            }
            context
                .run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        cache.begin_frame();
                        cache.sync(0, None, ui, &tokens);
                        let flat = cache.get(ui, &snapshot, 0, "", &tokens);
                        let tiled = cache.get(ui, &snapshot, 1, "", &tokens);
                        if let Some((old_flat, old_tiled)) = &previous {
                            assert_eq!(Rc::ptr_eq(old_flat, &flat), pass != 2);
                            assert_eq!(Rc::ptr_eq(old_tiled, &tiled), pass != 2);
                        }
                        assert_eq!(
                            flat.visible_text(ui, &snapshot, &tokens, 0.0..1000.0)[0].1,
                            "<?php é 🌧"
                        );
                        assert!(cache.bytes() <= CACHE_BYTES);
                        previous = Some((flat, tiled));
                    });
                })
                .textures_delta
                .clear();
        }
    }
    #[test]
    fn cache_shifts_untouched_lines_and_retargets_long_line_snapshot_offsets() {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        let mut cache = LineCache::default();
        let mut snapshot =
            TextSnapshot::from_str(&format!("a\nb\n{}\nlast", "value ".repeat(2000)));
        let mut first = None;
        let mut long = None;
        context
            .run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    cache.begin_frame();
                    cache.sync(0, None, ui, &tokens);
                    first = Some(cache.get(ui, &snapshot, 1, "", &tokens));
                    long = Some(cache.get(ui, &snapshot, 2, "", &tokens));
                    assert!(cache.built <= FRAME_TILE_BUDGET + 1);
                    let built = cache.built;
                    cache.sync(0, None, ui, &tokens);
                    assert_eq!(cache.built, built);
                });
            })
            .textures_delta
            .clear();
        snapshot.insert(0, "new\n");
        let change = TextChange {
            revision: 1,
            old_chars: 0..0,
            new_chars: 0..4,
            old_lines: 0..1,
            new_lines: 0..2,
        };
        context
            .run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    cache.begin_frame();
                    cache.sync(1, Some(&change), ui, &tokens);
                    let shifted = cache.get(ui, &snapshot, 2, "", &tokens);
                    let shifted_long = cache.get(ui, &snapshot, 3, "", &tokens);
                    assert!(Rc::ptr_eq(first.as_ref().unwrap(), &shifted));
                    assert!(Rc::ptr_eq(long.as_ref().unwrap(), &shifted_long));
                    assert_eq!(
                        shifted.visible_text(ui, &snapshot, &tokens, 0.0..1000.0)[0].1,
                        "b"
                    );
                    let text = shifted_long.visible_text(ui, &snapshot, &tokens, 0.0..1000.0);
                    assert!(text[0].1.starts_with("value "));
                    assert!(shifted_long.x(ui, &snapshot, &tokens, 5).unwrap() > 0.);
                    assert!(cache.built <= FRAME_TILE_BUDGET);
                    assert!(cache.bytes() <= CACHE_BYTES);
                });
            })
            .textures_delta
            .clear();
    }
    #[test]
    fn sixteen_mib_single_line_first_frame_has_bounded_work_and_cache() {
        let snapshot =
            TextSnapshot::from_str(&"a".repeat(codisseu_core::MAX_EDITOR_DOCUMENT_BYTES));
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        for enabled in [true, false] {
            let mut cache = LineCache::default();
            cache.set_ligatures(enabled);
            context
                .run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        cache.begin_frame();
                        cache.sync(0, None, ui, &tokens);
                        let line = cache.get(ui, &snapshot, 0, "", &tokens);
                        assert!(!line.is_complete());
                        assert!(line.scanned_chars() <= FRAME_TILE_BUDGET * tiles::TILE_CHARS);
                        assert!(cache.built <= FRAME_TILE_BUDGET);
                        assert!(cache.bytes() <= CACHE_BYTES);
                        assert_eq!(line.x(ui, &snapshot, &tokens, snapshot.len_chars()), None);
                        let built = cache.built;
                        let _again = cache.get(ui, &snapshot, 0, "", &tokens);
                        assert_eq!(
                            cache.built, built,
                            "budget is shared across repeated lookups"
                        );
                    });
                })
                .textures_delta
                .clear();
        }
    }
    #[test]
    fn visual_columns_expand_tabs_independently_from_scalar_and_pixel_positions() {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        let mut cache = LineCache::default();
        context
            .run_ui(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    cache.begin_frame();
                    for (revision, text) in ["//\t", "//a\t"].into_iter().enumerate() {
                        let snapshot = TextSnapshot::from_str(text);
                        cache.sync(revision as u64, None, ui, &tokens);
                        let layout = cache.get(ui, &snapshot, 0, "", &tokens);
                        assert_eq!(
                            layout.visual_column(ui, &snapshot, &tokens, snapshot.len_chars()),
                            Some(4)
                        );
                        assert_eq!(layout.visual_column(ui, &snapshot, &tokens, 2), Some(2));
                    }
                });
            })
            .textures_delta
            .clear();
    }
}
