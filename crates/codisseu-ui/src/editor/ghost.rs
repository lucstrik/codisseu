//! Source-preserving insertion projection; virtual glyphs never enter the buffer.
use super::*;
use codisseu_core::ai::copilot::{InlineProjection, Suggestion};
use std::{ops::Range, sync::Arc};

struct SourceGlyph {
    scalar: usize,
    source_x: Range<f32>,
    target_x: Range<f32>,
    row: usize,
}
pub(super) struct Ghost {
    pub theme: codisseu_core::ThemePreference,
    pub ticket: u64,
    pub geometry_id: u64,
    pub version: u64,
    pub caret: usize,
    pub line: usize,
    pub split_x: f32,
    pub source_end: usize,
    pub source_end_x: f32,
    pub source_breaks: Vec<usize>,
    source_glyphs: Vec<SourceGlyph>,
    pub text: String,
    pub rows: Vec<Arc<egui::Galley>>,
    ligatures: bool,
    state: Option<lexical::State>,
}
impl Ghost {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        suggestion: &Suggestion,
        caret: usize,
        split_x: f32,
        column: usize,
        ext: &str,
        tokens: &Tokens,
        ligatures: bool,
        state: Option<lexical::State>,
    ) -> Self {
        let projection = suggestion.projection().unwrap_or_else(|| InlineProjection {
            text: suggestion.suffix.clone(),
            preserved: vec![],
            consumed_source: 0,
        });
        let line = snapshot.char_to_line(caret);
        let mut syntax = PreviewSyntax(state.unwrap_or(lexical::State::Normal));
        syntax.advance(
            &snapshot
                .slice(snapshot.line_to_char(line)..caret)
                .to_string(),
            ext,
        );
        let source = snapshot
            .slice(caret..caret + projection.consumed_source)
            .to_string();
        let source_job = syntax.layout_themed(
            &source,
            ext,
            FontId::monospace(tokens.editor_font_size),
            tokens.theme,
        );
        let source_formats = source
            .char_indices()
            .map(|(b, _)| {
                source_job
                    .sections
                    .iter()
                    .find(|s| s.byte_range.start.0 <= b && b < s.byte_range.end.0)
                    .map(|s| s.format.clone())
                    .unwrap_or_else(|| {
                        egui::TextFormat::simple(
                            FontId::monospace(tokens.editor_font_size),
                            color(tokens.text),
                        )
                    })
            })
            .collect::<Vec<_>>();
        let font = FontId::new(
            tokens.editor_font_size,
            egui::FontFamily::Name("codisseu.mono.italic".into()),
        );
        let mut source_glyphs = Vec::new();
        let mut rows = Vec::new();
        let mut global_byte = 0;
        let mut source_scalar = 0;
        for (row, raw) in projection.text.split('\n').enumerate() {
            let visible = raw.trim_end_matches('\r');
            let mut visual_column = if row == 0 { column } else { 0 };
            let mut expanded = String::new();
            let mut expanded_scalars = 0;
            let mut ownership: Vec<(Range<usize>, egui::TextFormat)> = Vec::new();
            let mut glyphs = Vec::new();
            for (byte, ch) in visible.char_indices() {
                let a = expanded.len();
                let ac = expanded_scalars;
                if ch == '\t' {
                    let n = 4 - visual_column % 4;
                    expanded.extend(std::iter::repeat_n(' ', n));
                    visual_column += n;
                    expanded_scalars += n;
                } else {
                    expanded.push(ch);
                    visual_column += 1;
                    expanded_scalars += 1;
                }
                if projection
                    .preserved
                    .iter()
                    .any(|r| r.contains(&(global_byte + byte)))
                {
                    let fmt = source_formats[source_scalar].clone();
                    if let Some((r, f)) = ownership
                        .last_mut()
                        .filter(|(r, f)| r.end == a && *f == fmt)
                    {
                        r.end = expanded.len();
                        let _ = f;
                    } else {
                        ownership.push((a..expanded.len(), fmt));
                    }
                    glyphs.push((caret + source_scalar, ac, expanded_scalars));
                    source_scalar += 1;
                }
            }
            let mut job = syntax.layout_themed(&expanded, ext, font.clone(), tokens.theme);
            let mut sections = Vec::new();
            for section in std::mem::take(&mut job.sections) {
                let a = section.byte_range.start.0;
                let b = section.byte_range.end.0;
                let mut cuts = vec![a, b];
                for (r, _) in &ownership {
                    if r.start < b && r.end > a {
                        cuts.extend([r.start.clamp(a, b), r.end.clamp(a, b)]);
                    }
                }
                cuts.sort_unstable();
                cuts.dedup();
                for span in cuts.windows(2) {
                    let mut part = section.clone();
                    part.byte_range = (section.byte_range.start + (span[0] - a))
                        ..(section.byte_range.start + (span[1] - a));
                    if let Some((_, fmt)) = ownership.iter().find(|(r, _)| r.contains(&span[0])) {
                        part.format = fmt.clone();
                    } else {
                        part.format.font_id.family = font.family.clone();
                        part.format.color = part
                            .format
                            .color
                            .gamma_multiply(tokens.copilot_ghost_opacity.clamp(0., 1.));
                    }
                    sections.push(part);
                }
            }
            job.sections = sections;
            layout::prepare_job(&mut job, tokens, ligatures);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            let row_x = if row == 0 { split_x } else { 0. };
            for (scalar, a, b) in glyphs {
                let x = |c| galley.pos_from_cursor(egui::text::CCursor::new(c)).min.x + row_x;
                source_glyphs.push(SourceGlyph {
                    scalar,
                    source_x: 0. ..0.,
                    target_x: x(a)..x(b),
                    row,
                });
            }
            syntax.advance(&format!("{visible}\n"), ext);
            global_byte += raw.len() + 1;
            rows.push(galley);
        }
        let mut source_breaks = vec![caret];
        let mut n = caret;
        for r in &projection.preserved {
            n += projection.text[r.clone()].chars().count();
            source_breaks.push(n);
        }
        static NEXT_GEOMETRY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            theme: tokens.theme,
            ticket: suggestion.ticket,
            geometry_id: NEXT_GEOMETRY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            version: suggestion.document.version,
            caret,
            line,
            split_x,
            source_end: caret + projection.consumed_source,
            source_end_x: split_x,
            source_breaks,
            source_glyphs,
            text: suggestion.suffix.clone(),
            rows,
            ligatures,
            state,
        }
    }
    pub fn sync_source_layout(
        &mut self,
        ui: &egui::Ui,
        snapshot: &TextSnapshot,
        source: &layout::LineLayout,
        tokens: &Tokens,
    ) {
        let start = snapshot.line_to_char(self.line);
        self.source_end_x = source
            .x(ui, snapshot, tokens, self.source_end - start)
            .unwrap_or(self.split_x);
        for g in &mut self.source_glyphs {
            g.source_x = source
                .x(ui, snapshot, tokens, g.scalar - start)
                .unwrap_or(self.split_x)
                ..source
                    .x(ui, snapshot, tokens, g.scalar - start + 1)
                    .unwrap_or(self.source_end_x);
        }
    }
    pub fn matches(
        &self,
        suggestion: &Suggestion,
        caret: usize,
        ligatures: bool,
        state: Option<lexical::State>,
    ) -> bool {
        self.ticket == suggestion.ticket
            && self.text == suggestion.suffix
            && self.version == suggestion.document.version
            && self.caret == caret
            && self.ligatures == ligatures
            && self.state == state
    }
    pub fn added_rows(&self) -> usize {
        self.rows.len().saturating_sub(1)
    }
    pub fn visual_row(&self, fold: &folding::FoldMap, line: usize) -> usize {
        fold.visual_row(line)
            + if line > self.line {
                self.added_rows()
            } else {
                0
            }
    }
    pub fn source_line(&self, fold: &folding::FoldMap, row: usize) -> usize {
        let at = fold.visual_row(self.line);
        if row > at && row <= at + self.added_rows() {
            self.line
        } else {
            fold.source_line(if row > at {
                row.saturating_sub(self.added_rows())
            } else {
                row
            })
        }
    }
    pub fn suffix_x(&self) -> f32 {
        self.rows.last().map_or(0., |r| r.size().x)
            + if self.added_rows() == 0 {
                self.split_x
            } else {
                0.
            }
    }
    pub fn width(&self, source_width: f32) -> f32 {
        self.rows
            .iter()
            .enumerate()
            .map(|(i, r)| r.size().x + if i == 0 { self.split_x } else { 0. })
            .fold(0., f32::max)
            .max(self.suffix_x() + (source_width - self.source_end_x).max(0.))
    }
    pub fn projected_width(&self, row: usize, source_width: f32) -> f32 {
        let width = self.rows[row].size().x + if row == 0 { self.split_x } else { 0. };
        if row + 1 == self.rows.len() {
            width.max(self.suffix_x() + (source_width - self.source_end_x).max(0.))
        } else {
            width
        }
    }
    pub fn wrap_cut(&self, row: usize, start: f32, limit: f32) -> f32 {
        use unicode_segmentation::UnicodeSegmentation;
        let galley = &self.rows[row];
        let offset = if row == 0 { self.split_x } else { 0. };
        let mut cut = offset;
        let mut word = None;
        let mut scalar = 0;
        let mut content = false;
        for cluster in galley.job.text.graphemes(true) {
            scalar += cluster.chars().count();
            let x = offset
                + galley
                    .pos_from_cursor(egui::text::CCursor::new(scalar))
                    .min
                    .x;
            if x > limit && cut > start {
                break;
            }
            cut = x;
            if cluster.chars().all(char::is_whitespace) && x > start && content {
                word = Some(x);
            } else if x > start && !cluster.chars().all(char::is_whitespace) {
                content = true;
            }
            if x >= limit {
                break;
            }
        }
        word.unwrap_or(cut)
    }
    pub fn source_position(&self, x: f32, left: bool) -> (f32, usize) {
        if x < self.split_x || (left && x == self.split_x) {
            return (x, 0);
        }
        for g in &self.source_glyphs {
            if g.source_x.start <= x && (x < g.source_x.end || (left && x == g.source_x.end)) {
                let ratio = ((x - g.source_x.start)
                    / (g.source_x.end - g.source_x.start).max(f32::EPSILON))
                .clamp(0., 1.);
                return (
                    g.target_x.start + ratio * (g.target_x.end - g.target_x.start),
                    g.row,
                );
            }
        }
        (self.suffix_x() + x - self.source_end_x, self.added_rows())
    }
    pub fn source_x(&self, x: f32) -> f32 {
        self.source_position(x, false).0
    }
    pub fn project_range(&self, a: f32, b: f32) -> Vec<(Range<f32>, usize)> {
        let mut spans = Vec::new();
        for g in &self.source_glyphs {
            let l = a.max(g.source_x.start);
            let r = b.min(g.source_x.end);
            if r > l {
                let scale =
                    (g.target_x.end - g.target_x.start) / (g.source_x.end - g.source_x.start);
                spans.push((
                    g.target_x.start + (l - g.source_x.start) * scale
                        ..g.target_x.start + (r - g.source_x.start) * scale,
                    g.row,
                ));
            }
        }
        if a < self.split_x {
            spans.push((a..b.min(self.split_x), 0));
        }
        if b > self.source_end_x {
            let l = a.max(self.source_end_x);
            spans.push((
                self.suffix_x() + l - self.source_end_x..self.suffix_x() + b - self.source_end_x,
                self.added_rows(),
            ));
        }
        spans
    }
    pub fn pointer(&self, fold: &folding::FoldMap, row: usize, x: f32) -> Option<f32> {
        let anchor = fold.visual_row(self.line);
        if row == anchor && x < self.split_x {
            return Some(x);
        }
        for g in &self.source_glyphs {
            if row == anchor + g.row && g.target_x.contains(&x) {
                let ratio =
                    (x - g.target_x.start) / (g.target_x.end - g.target_x.start).max(f32::EPSILON);
                return Some(g.source_x.start + ratio * (g.source_x.end - g.source_x.start));
            }
        }
        if row == anchor + self.added_rows() && x >= self.suffix_x() {
            return Some(x - self.suffix_x() + self.source_end_x);
        }
        None
    }
    /// The last real source endpoint before an inserted-only gap, including
    /// preserved source spans on previous projected rows.
    pub fn source_boundary(&self, row: usize, x: f32) -> usize {
        let mut source = self.caret;
        for glyph in &self.source_glyphs {
            if glyph.row < row || (glyph.row == row && glyph.target_x.end <= x) {
                source = glyph.scalar + 1;
            } else if glyph.row > row || glyph.target_x.start >= x {
                break;
            } else {
                return glyph.scalar
                    + usize::from(x >= (glyph.target_x.start + glyph.target_x.end) * 0.5);
            }
        }
        source
    }
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        snapshot: &TextSnapshot,
        source: &layout::LineLayout,
        painter: &egui::Painter,
        origin: egui::Pos2,
        tokens: &Tokens,
        unnecessary: &[Range<usize>],
        viewport_x: Range<f32>,
        visible_rects: &mut Vec<[f32; 4]>,
    ) -> bool {
        let first_visible_part = visible_rects.is_empty();
        let line_height = tokens.editor_line_height;
        let shift = self.suffix_x() - self.source_end_x;
        let suffix_origin = origin + egui::vec2(shift, self.added_rows() as f32 * line_height);
        let clip = egui::Rect::from_min_max(
            egui::pos2(origin.x + self.suffix_x(), suffix_origin.y),
            egui::pos2(painter.clip_rect().right(), suffix_origin.y + line_height),
        )
        .intersect(painter.clip_rect());
        if clip.is_positive() {
            source.paint(
                ui,
                snapshot,
                layout::LineInk {
                    tokens,
                    unnecessary,
                },
                &painter.with_clip_rect(clip),
                suffix_origin,
                (viewport_x.start - shift)..(viewport_x.end - shift),
            );
        }
        let mut shown = false;
        for (index, galley) in self.rows.iter().enumerate() {
            let at = origin
                + egui::vec2(
                    if index == 0 { self.split_x } else { 0. },
                    index as f32 * line_height,
                );
            let rect =
                egui::Rect::from_min_size(at, egui::vec2(galley.size().x.max(1.), line_height));
            let visible = rect.intersect(painter.clip_rect());
            if visible.is_positive() {
                painter.galley(at, galley.clone(), Color32::PLACEHOLDER);
                shown = true;
                if visible_rects.len() < 512 {
                    visible_rects.push([
                        visible.left(),
                        visible.top(),
                        visible.right(),
                        visible.bottom(),
                    ]);
                }
            }
        }
        if shown {
            let bounds = visible_rects.iter().fold(egui::Rect::NOTHING, |b, r| {
                b.union(egui::Rect::from_min_max(
                    egui::pos2(r[0], r[1]),
                    egui::pos2(r[2], r[3]),
                ))
            });
            let label = ui.scope_id().with(("copilot.ghost", self.ticket));
            if first_visible_part {
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .scope_id(label)
                        .max_rect(bounds)
                        .sense(egui::Sense::empty())
                        .accessibility_role(egui::accesskit::Role::Label),
                );
                child.expand_to_include_rect(bounds);
            }
            ui.ctx().accesskit_node_builder(label, |b| {
                b.set_role(egui::accesskit::Role::Label);
                b.set_value(format!("Sugestão Copilot: {}", self.text));
                b.set_bounds(egui::accesskit::Rect {
                    x0: bounds.left() as f64,
                    y0: bounds.top() as f64,
                    x1: bounds.right() as f64,
                    y1: bounds.bottom() as f64,
                });
            });
        }
        shown
    }
}
pub(super) fn visual_row(view: &EditorView, line: usize) -> usize {
    if view.wrap.enabled {
        return view.wrap.visual_row(&view.fold, line);
    }
    view.ghost.as_ref().map_or_else(
        || view.fold.visual_row(line),
        |g| g.visual_row(&view.fold, line),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn projection(rows: usize) -> Ghost {
        let ctx = egui::Context::default();
        let mut galley = None;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            galley = Some(ui.fonts_mut(|fonts| {
                fonts.layout_no_wrap(String::new(), FontId::monospace(13.), Color32::GRAY)
            }));
        });
        output.textures_delta.clear();
        Ghost {
            theme: codisseu_core::ThemePreference::Classic,
            ticket: 1,
            geometry_id: 0,
            version: 1,
            caret: 5,
            line: 1,
            split_x: 20.,
            source_end: 5,
            source_end_x: 20.,
            source_breaks: vec![],
            source_glyphs: vec![],
            text: String::new(),
            rows: vec![galley.unwrap(); rows],
            ligatures: true,
            state: None,
        }
    }
    #[test]
    fn ghost_rows_project_source_without_changing_fold_coordinates() {
        let mut fold = folding::FoldMap::new(
            8,
            vec![folding::FoldRange {
                start: 3,
                end: 5,
                kind: folding::FoldKind::Braces,
                level: 0,
            }],
        );
        fold.fold_at(3);
        let g = projection(3);
        assert_eq!(
            (
                g.visual_row(&fold, 0),
                g.visual_row(&fold, 1),
                g.visual_row(&fold, 2)
            ),
            (0, 1, 4)
        );
        assert_eq!(
            (
                g.source_line(&fold, 1),
                g.source_line(&fold, 2),
                g.source_line(&fold, 3),
                g.source_line(&fold, 4)
            ),
            (1, 1, 1, 2)
        );
        assert_eq!(g.visual_row(&fold, 6), 6);
        assert_eq!(g.source_line(&fold, 6), 6);
        assert!(fold.is_hidden(4));
        assert_eq!(fold.visible_line_count(), 6);
    }
    #[test]
    fn ghost_clicks_anchor_at_caret_and_source_suffix_hits_remain_source_positions() {
        let fold = folding::FoldMap::new(5, vec![]);
        let g = projection(3);
        assert_eq!(g.pointer(&fold, 1, 8.), Some(8.));
        assert_eq!(g.pointer(&fold, 1, 30.), None);
        assert_eq!(g.pointer(&fold, 2, 3.), None);
        assert_eq!(g.pointer(&fold, 3, 7.), Some(27.));
        let g = projection(1);
        assert_eq!(g.pointer(&fold, 1, 27.), Some(27.));
    }

    #[test]
    fn multipart_ghost_styles_only_insertions_and_maps_preserved_source() {
        use codisseu_core::ai::copilot::{Document, Position};
        let snapshot = TextSnapshot::from_str("call() tail");
        let suggestion = Suggestion {
            ticket: 1,
            document: Document {
                uri: "file:///owned.php".into(),
                version: 1,
                language: "php".into(),
                text: Arc::from(snapshot.to_string()),
                position: Position {
                    line: 0,
                    character: 5,
                },
            },
            start: 0,
            end: 6,
            insert: "call($fatura);".into(),
            suffix: "$fatura);".into(),
            item: serde_json::Value::Null,
        };
        let ctx = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&ctx);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut g = Ghost::build(
                ui,
                &snapshot,
                &suggestion,
                5,
                40.,
                5,
                "php",
                &tokens,
                true,
                None,
            );
            assert_eq!(g.source_end, 6);
            let job = &g.rows[0].job;
            assert_eq!(job.text, "$fatura);");
            for section in &job.sections {
                let real = &job.text[section.byte_range.start.0..section.byte_range.end.0] == ")";
                assert_eq!(section.format.color.a() == 255, real);
                assert_eq!(
                    section.format.font_id.family == egui::FontFamily::Monospace,
                    real
                );
            }
            let range = g.source_glyphs[0].target_x.clone();
            g.source_end_x = 40. + range.end - range.start;
            g.source_glyphs[0].source_x = 40. ..g.source_end_x;
            let fold = folding::FoldMap::new(1, vec![]);
            assert_eq!(g.pointer(&fold, 0, range.start + 1.), Some(41.));
            assert_eq!(g.pointer(&fold, 0, range.end + 1.), None);
            assert_eq!(g.source_x(40.), range.start);
            assert_eq!(g.source_x(g.source_end_x), g.suffix_x());
            assert_eq!(snapshot.to_string(), "call() tail");
        });
        output.textures_delta.clear();
    }
    #[test]
    fn ghost_shaping_keeps_crlf_tabs_source_bytes_and_alternative_identity() {
        use codisseu_core::ai::copilot::{Document, Position};
        let snapshot = TextSnapshot::from_str("<?php\r\n\tcall();\r\nunchanged");
        let before = snapshot.to_string();
        let caret = snapshot.line_to_char(1) + 6;
        let suggestion = Suggestion {
            ticket: 42,
            document: Document {
                uri: "file:///owned.php".into(),
                version: 7,
                language: "php".into(),
                text: Arc::from(before.as_str()),
                position: Position {
                    line: 1,
                    character: 6,
                },
            },
            start: caret,
            end: caret,
            insert: "first,\r\n\tsecond\r\n}".into(),
            suffix: "first,\r\n\tsecond\r\n}".into(),
            item: serde_json::Value::Null,
        };
        for scale in [1., 1.25] {
            let ctx = egui::Context::default();
            let tokens = Tokens::load();
            tokens.apply(&ctx);
            ctx.set_pixels_per_point(scale);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let g = Ghost::build(
                    ui,
                    &snapshot,
                    &suggestion,
                    caret,
                    40.,
                    9,
                    "php",
                    &tokens,
                    true,
                    Some(lexical::State::Normal),
                );
                assert_eq!(g.rows.len(), 3);
                assert_eq!(g.rows[1].job.text, "    second");
                for row in &g.rows {
                    for section in &row.job.sections {
                        assert_eq!(
                            section.format.font_id.family,
                            egui::FontFamily::Name("codisseu.mono.italic".into())
                        );
                        assert_eq!(section.format.line_height, Some(tokens.editor_line_height));
                        assert!(section.format.color.a() < 255);
                    }
                }
                assert!(g.matches(&suggestion, caret, true, Some(lexical::State::Normal)));
                let mut alternative = suggestion.clone();
                alternative.suffix = "different option".into();
                assert!(!g.matches(&alternative, caret, true, Some(lexical::State::Normal)));
                assert!(!g.matches(&suggestion, caret + 1, true, Some(lexical::State::Normal)));
                assert_eq!(snapshot.to_string(), before);
            });
            output.textures_delta.clear();
        }
    }
}
