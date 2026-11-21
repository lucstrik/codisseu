//! Read-only diff using the editor lexer and bounded, viewport-only shaping.
use super::editor::PreviewSyntax;
use super::*;
use std::{collections::BTreeMap, path::Path, sync::Arc};

type Cell = Arc<DiffCell>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Context,
    Added,
    Deleted,
    Header,
}
struct DiffCell {
    text: String,
    old: Option<usize>,
    new: Option<usize>,
    kind: Kind,
    extension: Arc<str>,
    syntax: PreviewSyntax,
}
struct Row {
    cells: [Option<Cell>; 2],
}
#[derive(Default)]
pub(super) struct DiffContent {
    side: Vec<Row>,
    unified: Vec<Row>,
    max_line: usize,
}
impl DiffContent {
    /// Full-file comparison already has version headers supplied by its host.
    /// Keep source lines that resemble patch metadata: inspect parsed kinds only.
    pub(super) fn hide_patch_headers(&mut self) {
        let source = |r: &Row| r.cells.iter().flatten().any(|c| c.kind != Kind::Header);
        self.side.retain(source);
        self.unified.retain(source);
    }

    pub fn new(text: &str, path: Option<&Path>, file: bool) -> Self {
        let mut out = Self::default();
        let mut extension: Arc<str> = path.map(highlight::syntax_key).unwrap_or_default().into();
        let mut states = [PreviewSyntax::default(); 2];
        let (mut old, mut new) = (1, 1);
        let mut in_hunk = file;
        let mut removed = Vec::new();
        let mut added = Vec::new();
        let flush = |out: &mut Self, removed: &mut Vec<Cell>, added: &mut Vec<Cell>| {
            let mut l = removed.drain(..);
            let mut r = added.drain(..);
            loop {
                let cells = [l.next(), r.next()];
                if cells.iter().all(Option::is_none) {
                    break;
                }
                out.side.push(Row { cells });
            }
        };
        for line in text.lines() {
            let (kind, code) = if file {
                (Kind::Context, line)
            } else if line.starts_with("diff --git ") {
                flush(&mut out, &mut removed, &mut added);
                in_hunk = false;
                states = [PreviewSyntax::default(); 2];
                (Kind::Header, line)
            } else if !in_hunk && line.starts_with("+++ ") {
                if let Some(name) = line.strip_prefix("+++ b/") {
                    extension = highlight::syntax_key(Path::new(name)).into();
                }
                (Kind::Header, line)
            } else if line.starts_with("@@") {
                flush(&mut out, &mut removed, &mut added);
                let mut parts = line.split_whitespace().skip(1);
                old = parts
                    .next()
                    .and_then(|s| s.trim_start_matches('-').split(',').next()?.parse().ok())
                    .unwrap_or(1);
                new = parts
                    .next()
                    .and_then(|s| s.trim_start_matches('+').split(',').next()?.parse().ok())
                    .unwrap_or(1);
                states = [PreviewSyntax::default(); 2];
                in_hunk = true;
                (Kind::Header, line)
            } else if in_hunk && line.starts_with('-') {
                (Kind::Deleted, &line[1..])
            } else if in_hunk && line.starts_with('+') {
                (Kind::Added, &line[1..])
            } else if in_hunk && line.starts_with(' ') {
                (Kind::Context, &line[1..])
            } else {
                (Kind::Header, line)
            };
            if kind == Kind::Header
                && (line.starts_with("index ")
                    || line.starts_with("--- ")
                    || line.starts_with("+++ "))
            {
                continue;
            }
            let heading = line
                .strip_prefix("diff --git a/")
                .and_then(|s| s.split_once(" b/"))
                .map(|(_, name)| name);
            let code = if kind == Kind::Header {
                heading.unwrap_or(code)
            } else {
                code
            };
            let numbers = match kind {
                Kind::Context => [Some(old), Some(new)],
                Kind::Deleted => [Some(old), None],
                Kind::Added => [None, Some(new)],
                Kind::Header => [None, None],
            };
            let cell = |syntax| {
                Arc::new(DiffCell {
                    text: code.into(),
                    old: numbers[0],
                    new: numbers[1],
                    kind,
                    extension: extension.clone(),
                    syntax,
                })
            };
            let right = cell(states[1]);
            let left = cell(states[0]);
            out.unified.push(Row {
                cells: [
                    Some(if kind == Kind::Deleted {
                        left.clone()
                    } else {
                        right.clone()
                    }),
                    None,
                ],
            });
            match kind {
                Kind::Deleted => removed.push(left),
                Kind::Added => added.push(right),
                Kind::Context => {
                    flush(&mut out, &mut removed, &mut added);
                    if !file {
                        out.side.push(Row {
                            cells: [Some(left), Some(right)],
                        });
                    }
                }
                Kind::Header => {
                    flush(&mut out, &mut removed, &mut added);
                    // File headers remain visible in both views, including empty/binary patches.
                    out.side.push(Row {
                        cells: [Some(right), None],
                    });
                }
            }
            out.max_line = out.max_line.max(old).max(new);
            if numbers[0].is_some() {
                old += 1;
                states[0].advance(code, &extension);
            }
            if numbers[1].is_some() {
                new += 1;
                states[1].advance(code, &extension);
            }
        }
        flush(&mut out, &mut removed, &mut added);
        out
    }
}

#[derive(Default)]
pub(super) struct DiffView {
    source: Option<Arc<super::git_diff::DiffData>>,
    shape_key: Option<(u32, u32, u32, bool, bool, codisseu_core::ThemePreference)>,
    heights: Vec<f32>,
    offsets: Vec<f32>,
    layouts: BTreeMap<usize, [Option<Arc<egui::Galley>>; 2]>,
    widest: f32,
    cache_bytes: usize,
    pub visible_rows: usize,
}
impl DiffView {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        data: Arc<super::git_diff::DiffData>,
        tokens: &Tokens,
        unified: bool,
        wrap: bool,
        height: f32,
    ) {
        let width = ui.available_width().max(80.);
        let rows = if unified {
            &data.content.unified
        } else {
            &data.content.side
        };
        let key = (
            width.to_bits(),
            tokens.editor_font_size.to_bits(),
            ui.ctx().pixels_per_point().to_bits(),
            unified,
            wrap,
            tokens.theme,
        );
        let line_height = tokens.editor_line_height.max(tokens.editor_font_size + 2.);
        if self
            .source
            .as_ref()
            .is_none_or(|old| !Arc::ptr_eq(old, &data))
            || self.shape_key != Some(key)
        {
            self.source = Some(data.clone());
            self.shape_key = Some(key);
            self.heights = vec![line_height + 4.; rows.len()];
            self.offsets = (0..=rows.len())
                .map(|n| n as f32 * (line_height + 4.))
                .collect();
            self.layouts.clear();
            self.widest = width;
            self.cache_bytes = 0;
        }
        // Evict offscreen shaping before it can accumulate across a long review.
        if self.layouts.len() > 512 || self.cache_bytes > 8 * 1024 * 1024 {
            self.layouts.clear();
            self.cache_bytes = 0;
        }
        let digits = data.content.max_line.max(1).ilog10() + 1;
        let gutter = (digits as f32 * tokens.editor_font_size * 0.7 + 14.).max(44.)
            * if unified { 2. } else { 1. };
        let pane = if unified { width } else { (width - 1.) / 2. };
        let code_width = (pane - gutter - 12.).max(12.);
        self.visible_rows = 0;
        egui::ScrollArea::new([!wrap, true])
            .id_salt(("git.diff.code", unified))
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .codisseu()
            .show_viewport(ui, |ui, viewport| {
                let base = ui.cursor().min;
                let start = self
                    .offsets
                    .partition_point(|y| *y <= viewport.min.y)
                    .saturating_sub(1)
                    .min(rows.len());
                let mut end = start;
                let mut y = self.offsets[start];
                let mut geometry_changed = false;
                while end < rows.len() && y < viewport.max.y + line_height {
                    let row = &rows[end];
                    let is_new = !self.layouts.contains_key(&end);
                    let layouts = self.layouts.entry(end).or_insert_with(|| {
                        std::array::from_fn(|side| {
                            row.cells[side].as_ref().map(|cell| {
                                let text = expand_tabs(&cell.text);
                                let mut job = if cell.kind == Kind::Header {
                                    let mut job = egui::text::LayoutJob::default();
                                    job.append(
                                        &text,
                                        0.,
                                        egui::TextFormat {
                                            font_id: FontId::monospace(tokens.editor_font_size),
                                            color: color(tokens.muted),
                                            ..Default::default()
                                        },
                                    );
                                    job
                                } else {
                                    cell.syntax.layout_themed(
                                        &text,
                                        &cell.extension,
                                        FontId::monospace(tokens.editor_font_size),
                                        tokens.theme,
                                    )
                                };
                                for section in &mut job.sections {
                                    section.format.line_height = Some(line_height);
                                }
                                // Headings span both panes and never truncate a long path.
                                job.wrap.max_width = if wrap {
                                    if cell.kind == Kind::Header {
                                        width - 12.
                                    } else {
                                        code_width
                                    }
                                } else {
                                    f32::INFINITY
                                };
                                ui.fonts_mut(|fonts| fonts.layout_job(job))
                            })
                        })
                    });
                    let h = layouts
                        .iter()
                        .flatten()
                        .map(|g| g.size().y + 4.)
                        .fold(line_height + 4., f32::max);
                    geometry_changed |= self.heights[end] != h;
                    self.heights[end] = h;
                    if is_new {
                        self.cache_bytes += layouts
                            .iter()
                            .flatten()
                            .map(|g| g.job.text.len() + g.num_vertices * 32 + g.num_indices * 4)
                            .sum::<usize>();
                    }
                    if !wrap {
                        for galley in layouts.iter().flatten() {
                            self.widest = self.widest.max(
                                galley.size().x
                                    + gutter
                                    + 12.
                                    + if unified { 0. } else { pane + 1. },
                            );
                        }
                    }
                    y += h;
                    end += 1;
                }
                // Prefix geometry changes only when rows are first shaped or resized.
                if geometry_changed {
                    for n in start..self.heights.len() {
                        self.offsets[n + 1] = self.offsets[n] + self.heights[n];
                    }
                }
                let content_width = if wrap { width } else { self.widest };
                ui.set_min_size(egui::vec2(
                    content_width,
                    *self.offsets.last().unwrap_or(&0.),
                ));
                let visible_rect =
                    egui::Rect::from_min_size(base + viewport.min.to_vec2(), viewport.size());
                ui.painter()
                    .rect_filled(visible_rect, 0., color(tokens.editor));
                for (n, row) in rows.iter().enumerate().take(end).skip(start) {
                    let top = base.y + self.offsets[n];
                    let h = self.heights[n];
                    let layouts = &self.layouts[&n];
                    self.visible_rows += 1;
                    for (side, layout) in
                        layouts.iter().enumerate().take(if unified { 1 } else { 2 })
                    {
                        let x = base.x + viewport.min.x + side as f32 * (pane + 1.);
                        let rect =
                            egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(pane, h));
                        let Some(cell) = &row.cells[side] else {
                            continue;
                        };
                        let Some(galley) = layout else {
                            continue;
                        };
                        let header = cell.kind == Kind::Header;
                        let rect = if header {
                            egui::Rect::from_min_size(rect.min, egui::vec2(width, h))
                        } else {
                            rect
                        };
                        let fill = match cell.kind {
                            Kind::Added => tokens.git_diff_added,
                            Kind::Deleted => tokens.git_diff_deleted,
                            Kind::Header => tokens.shell_panel_background,
                            Kind::Context => tokens.editor,
                        };
                        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                        painter.rect_filled(rect, 0., color(fill));
                        if !header {
                            let numbers = if unified {
                                [cell.old, cell.new]
                            } else {
                                [if side == 0 { cell.old } else { cell.new }, None]
                            };
                            let number_width = if unified { gutter / 2. } else { gutter };
                            for (column, number) in numbers.iter().enumerate() {
                                if let Some(number) = number {
                                    painter.text(
                                        egui::pos2(
                                            x + number_width * (column + 1) as f32 - 8.,
                                            top + 2. + line_height / 2.,
                                        ),
                                        egui::Align2::RIGHT_CENTER,
                                        number.to_string(),
                                        FontId::monospace(tokens.editor_font_size - 1.),
                                        color(tokens.muted),
                                    );
                                }
                            }
                            painter.line_segment(
                                [egui::pos2(x + gutter, top), egui::pos2(x + gutter, top + h)],
                                egui::Stroke::new(1., color(tokens.border)),
                            );
                        }
                        let code_rect = egui::Rect::from_min_max(
                            egui::pos2(x + if header { 0. } else { gutter + 2. }, top),
                            rect.max,
                        );
                        painter
                            .with_clip_rect(code_rect.intersect(painter.clip_rect()))
                            .galley(
                                egui::pos2(
                                    x + if header { 6. } else { gutter + 6. } - viewport.min.x,
                                    top + 2.,
                                ),
                                galley.clone(),
                                color(tokens.text),
                            );
                        ui.interact(
                            rect,
                            ui.scope_id().with(("diff.line", n, side)),
                            egui::Sense::hover(),
                        )
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Label,
                                true,
                                &cell.text,
                            )
                        });
                    }
                    if !unified && row.cells[0].as_ref().is_none_or(|c| c.kind != Kind::Header) {
                        ui.painter().line_segment(
                            [
                                egui::pos2(base.x + viewport.min.x + pane, top),
                                egui::pos2(base.x + viewport.min.x + pane, top + h),
                            ],
                            egui::Stroke::new(1., color(tokens.border)),
                        );
                    }
                }
            });
    }
}
fn expand_tabs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.trim_end_matches('\r').chars() {
        if c == '\t' {
            let spaces = 4 - column % 4;
            out.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}

#[cfg(test)]
pub(super) fn assert_wrapped_view(view: &DiffView, multiline: bool) {
    assert!(view.visible_rows > 0);
    let galleys: Vec<_> = view
        .layouts
        .values()
        .flat_map(|pair| pair.iter().flatten())
        .collect();
    assert!(!galleys.is_empty());
    if multiline {
        assert!(galleys.iter().any(|g| g.rows.len() > 1));
    }
    for g in galleys {
        assert!(!g.elided, "soft wraps must preserve the entire line");
        assert!(g.job.wrap.max_width.is_finite());
        assert!(
            g.size().x <= g.job.wrap.max_width + 1.,
            "line exceeds pane: {} > {}",
            g.size().x,
            g.job.wrap.max_width
        );
        assert!(!g.job.text.contains('\t'));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn file_comparison_hides_only_patch_headers_and_preserves_source_numbers() {
        let mut content = DiffContent::new(
            "@@ -1,3 +1,3 @@\n @@ literal source\n-+++ old source\n++++ new source\n context\n",
            Some(Path::new("sample.php")),
            false,
        );
        content.hide_patch_headers();
        assert_eq!(content.side.len(), 3);
        assert!(
            content
                .unified
                .iter()
                .flat_map(|r| r.cells.iter().flatten())
                .all(|c| c.kind != Kind::Header)
        );
        assert_eq!(
            content.side[0].cells[0].as_ref().unwrap().text,
            "@@ literal source"
        );
        assert_eq!(content.side[0].cells[0].as_ref().unwrap().old, Some(1));
        assert_eq!(
            content.side[1].cells[1].as_ref().unwrap().text,
            "+++ new source"
        );
        assert_eq!(content.side[2].cells[1].as_ref().unwrap().new, Some(3));
    }
    #[test]
    fn shared_lexer_continues_comments_and_unified_preserves_patch_order() {
        let patch = "diff --git a/demo.php b/demo.php\n--- a/demo.php\n+++ b/demo.php\n@@ -1,5 +1,5 @@\n <?php\n /*\n- old comment\n+ new comment\n  */\n-\treturn 1;\n+\treturn '😀';\n";
        let content = DiffContent::new(patch, Some(Path::new("demo.php")), false);
        let deleted = content
            .unified
            .iter()
            .filter_map(|r| r.cells[0].as_ref())
            .find(|c| c.text.trim() == "old comment")
            .unwrap();
        let job = deleted
            .syntax
            .layout(&deleted.text, &deleted.extension, FontId::monospace(13.));
        let comment = highlight::layout("// comment", "php", FontId::monospace(13.));
        assert_eq!(
            job.sections[0].format.color,
            comment.sections[0].format.color
        );
        let code = content
            .unified
            .iter()
            .filter_map(|r| r.cells[0].as_ref())
            .find(|c| c.text.contains("return '😀'"))
            .unwrap();
        let job = code
            .syntax
            .layout(&code.text, &code.extension, FontId::monospace(13.));
        assert!(
            job.sections
                .windows(2)
                .any(|s| s[0].format.color != s[1].format.color)
        );
        assert_eq!(expand_tabs("\treturn\t'😀'"), "    return  '😀'");
        let order: Vec<_> = content
            .unified
            .iter()
            .filter_map(|r| r.cells[0].as_ref())
            .map(|c| c.text.as_str())
            .collect();
        assert!(
            order.iter().position(|s| s.contains("return 1")).unwrap()
                < order
                    .iter()
                    .position(|s| s.contains("return '😀'"))
                    .unwrap()
        );
        let paired = content
            .side
            .iter()
            .find(|r| {
                r.cells[0]
                    .as_ref()
                    .is_some_and(|c| c.text.contains("return 1"))
            })
            .unwrap();
        assert!(
            paired.cells[1]
                .as_ref()
                .unwrap()
                .text
                .contains("return '😀'")
        );
    }
    #[test]
    fn historical_source_plus_minus_and_hunk_like_text_are_plain_source() {
        let content = DiffContent::new(
            "+literal\n-literal\n@@ source\n",
            Some(Path::new("source.txt")),
            true,
        );
        assert_eq!(content.unified.len(), 3);
        for row in &content.unified {
            let cell = row.cells[0].as_ref().unwrap();
            assert!(cell.kind == Kind::Context);
            assert_eq!(cell.old, cell.new);
        }
        assert_eq!(
            content.unified[0].cells[0].as_ref().unwrap().text,
            "+literal"
        );
    }
}
