//! Three-way review UI. Source bytes and line matching come from the Git worker.
use super::editor::PreviewSyntax;
use super::git_merge_style::{MergeStyle, tool};
use super::*;
use codisseu_core::git::text_merge::{MergeKind, TextMerge};
use std::{collections::BTreeMap, ops::Range, sync::Arc};

#[derive(Clone, Default)]
struct Draft {
    decisions: Vec<[Option<bool>; 2]>,
    values: Vec<String>,
    manual: Vec<bool>,
    full: Option<String>,
    full_reviewed: bool,
}
struct Line {
    chunk: usize,
    first: bool,
    text: [Option<String>; 3],
    numbers: [Option<usize>; 3],
    syntax: [PreviewSyntax; 3],
    result_range: Option<Range<usize>>,
    base: Option<String>,
    omitted: Option<Range<usize>>,
}
#[derive(Default)]
pub(super) struct MergeUi {
    key: String,
    model: Option<Arc<TextMerge>>,
    full_sources: Option<[String; 2]>,
    change_numbers: Vec<usize>,
    states_dirty: bool,
    line_height: f32,
    simple: Vec<Option<String>>,
    pub delete_requested: bool,
    pub close_requested: bool,
    comparison: Option<(String, String, String, String)>,
    pub collapse_unchanged: bool,
    draft: Draft,
    undo: Vec<Draft>,
    redo: Vec<Draft>,
    lines: Vec<Line>,
    offsets: Vec<f32>,
    heights: Vec<f32>,
    layouts: BTreeMap<usize, [Option<Arc<egui::Galley>>; 3]>,
    layout_key: Option<(u32, u32, u32, bool)>,
    focus: Option<usize>,
    scroll_to: Option<usize>,
    pub no_wrap: bool,
    widest: f32,
    cache_bytes: usize,
    pub visible_rows: usize,
    pub revision: u64,
}
impl MergeUi {
    pub fn key_matches(&self, key: &str) -> bool {
        self.key == key
    }
    pub fn can_choose(&self) -> bool {
        self.draft.full.is_none()
    }
    pub fn sync(&mut self, key: String, model: Arc<TextMerge>, existing: Option<&str>) {
        if self.key == key {
            return;
        }
        *self = Self {
            key,
            model: Some(model),
            ..Default::default()
        };
        let model = self.model.as_ref().unwrap();
        self.simple = model
            .chunks
            .iter()
            .map(codisseu_core::git::text_merge::simple_resolution)
            .collect();
        let mut number = 0;
        self.change_numbers = model
            .chunks
            .iter()
            .map(|c| {
                if c.kind != MergeKind::Unchanged {
                    number += 1;
                }
                number
            })
            .collect();
        self.draft.values = model
            .chunks
            .iter()
            .map(|c| {
                if c.kind == MergeKind::BothSame {
                    c.ours.clone()
                } else {
                    c.base.clone()
                }
            })
            .collect();
        self.draft.decisions = model
            .chunks
            .iter()
            .map(|c| match c.kind {
                MergeKind::Unchanged | MergeKind::BothSame => [Some(false), Some(false)],
                MergeKind::OursOnly => [None, Some(false)],
                MergeKind::TheirsOnly => [Some(false), None],
                MergeKind::Conflict => [None, None],
            })
            .collect();
        self.draft.manual = vec![false; model.chunks.len()];
        if let Some(text) = existing {
            self.draft.full = Some(text.into());
        }
        self.rebuild();
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.draft.clone());
        self.redo.clear();
        while self.undo.len() > 16
            || self.undo.iter().map(draft_bytes).sum::<usize>() > 8 * 1024 * 1024
        {
            if self.undo.len() <= 1 {
                break;
            }
            self.undo.remove(0);
        }
    }
    pub fn result(&self) -> String {
        self.draft
            .full
            .clone()
            .unwrap_or_else(|| self.draft.values.concat())
    }
    fn resolved(&self, n: usize) -> bool {
        (self.draft.full.is_some() && self.draft.full_reviewed)
            || self.draft.manual[n]
            || self.draft.decisions[n].iter().all(Option::is_some)
    }
    pub fn pending(&self) -> usize {
        self.model.as_ref().map_or(0, |m| {
            m.chunks
                .iter()
                .enumerate()
                .filter(|(n, c)| c.kind != MergeKind::Unchanged && !self.resolved(*n))
                .count()
        })
    }
    pub fn conflicts(&self) -> usize {
        self.model.as_ref().map_or(0, |m| {
            m.chunks
                .iter()
                .enumerate()
                .filter(|(n, c)| c.kind == MergeKind::Conflict && !self.resolved(*n))
                .count()
        })
    }
    fn choose(&mut self, n: usize, side: usize, accept: bool, exclusive: bool) {
        if self.draft.full.is_some() {
            return;
        }
        self.checkpoint();
        self.draft.decisions[n][side] = Some(accept);
        if exclusive {
            self.draft.decisions[n][1 - side] = Some(!accept);
        }
        self.draft.manual[n] = false;
        let c = &self.model.as_ref().unwrap().chunks[n];
        self.draft.values[n] = match self.draft.decisions[n] {
            [Some(true), Some(true)] => format!("{}{}", c.ours, c.theirs),
            [Some(true), _] => c.ours.clone(),
            [_, Some(true)] => c.theirs.clone(),
            _ => c.base.clone(),
        };
        self.focus = Some(n);
        self.rebuild();
    }
    pub fn accept_all(&mut self, side: usize) {
        if self.draft.full.is_some() {
            return;
        }
        self.checkpoint();
        for (n, c) in self.model.as_ref().unwrap().chunks.iter().enumerate() {
            self.draft.decisions[n] = if side == 0 {
                [Some(true), Some(false)]
            } else {
                [Some(false), Some(true)]
            };
            self.draft.values[n] = if side == 0 {
                c.ours.clone()
            } else {
                c.theirs.clone()
            };
            self.draft.manual[n] = false;
        }
        self.rebuild();
    }
    fn undo(&mut self, redo: bool) {
        let source = if redo { &mut self.redo } else { &mut self.undo };
        if let Some(old) = source.pop() {
            let current = std::mem::replace(&mut self.draft, old);
            if redo {
                self.undo.push(current);
            } else {
                self.redo.push(current);
            }
            self.rebuild();
        }
    }
    fn non_conflicting(&mut self, side: Option<usize>) {
        if self.draft.full.is_some() {
            return;
        }
        self.checkpoint();
        for (n, c) in self.model.as_ref().unwrap().chunks.iter().enumerate() {
            let s = match c.kind {
                MergeKind::OursOnly => 0,
                MergeKind::TheirsOnly => 1,
                _ => continue,
            };
            if side.is_some_and(|side| s != side) || self.resolved(n) {
                continue;
            }
            self.draft.decisions[n] = if s == 0 {
                [Some(true), Some(false)]
            } else {
                [Some(false), Some(true)]
            };
            self.draft.values[n] = if s == 0 {
                c.ours.clone()
            } else {
                c.theirs.clone()
            };
        }
        self.rebuild();
    }
    fn navigate(&mut self, back: bool) {
        let m = self.model.as_ref().unwrap();
        let pending: Vec<_> = m
            .chunks
            .iter()
            .enumerate()
            .filter(|(n, c)| c.kind != MergeKind::Unchanged && !self.resolved(*n))
            .map(|(n, _)| n)
            .collect();
        let Some(first) = pending.first().copied() else {
            return;
        };
        let target = if back {
            pending
                .iter()
                .rev()
                .find(|n| self.focus.is_some_and(|f| **n < f))
                .copied()
                .unwrap_or(*pending.last().unwrap())
        } else {
            pending
                .iter()
                .find(|n| self.focus.is_none_or(|f| **n > f))
                .copied()
                .unwrap_or(first)
        };
        self.focus = Some(target);
        self.scroll_to = Some(target);
    }
    fn rebuild(&mut self) {
        let mut old_heights = std::collections::BTreeMap::<usize, Vec<f32>>::new();
        for (n, line) in self.lines.iter().enumerate() {
            old_heights
                .entry(line.chunk)
                .or_default()
                .push(self.heights.get(n).copied().unwrap_or(24.));
        }
        self.revision = self.revision.wrapping_add(1);
        self.lines.clear();
        self.heights.clear();
        self.layouts.clear();
        self.states_dirty = true;
        self.cache_bytes = 0;
        let Some(model) = &self.model else { return };
        let syntax = [PreviewSyntax::default(); 3];
        let mut result_line = 0;
        // The extension is applied when rendering; lexer states are prepared there.
        for (n, c) in model.chunks.iter().enumerate() {
            let values = [
                c.ours.as_str(),
                self.draft.values[n].as_str(),
                c.theirs.as_str(),
            ];
            let splits = values.map(|s| s.split_inclusive('\n').collect::<Vec<_>>());
            let base_lines = c.base.split_inclusive('\n').collect::<Vec<_>>();
            let count = splits.iter().map(Vec::len).max().unwrap_or(0).max(1);
            let mut result_byte = 0;
            for row in 0..count {
                if self.collapse_unchanged
                    && c.kind == MergeKind::Unchanged
                    && self.draft.values[n] == c.base
                    && count > 8
                    && row >= 3
                    && row < count - 3
                {
                    if row == 3 {
                        self.heights.push(self.line_height.max(22.));
                        self.lines.push(Line {
                            chunk: n,
                            first: false,
                            text: std::array::from_fn(|_| {
                                Some(format!("⋯ {} unchanged lines ⋯", count - 6))
                            }),
                            numbers: [None; 3],
                            syntax,
                            result_range: None,
                            base: None,
                            omitted: Some(3..count - 3),
                        });
                    }
                    result_byte += splits[1].get(row).map_or(0, |s| s.len());
                    continue;
                }

                let text = std::array::from_fn(|side| {
                    splits[side]
                        .get(row)
                        .map(|s| s.trim_end_matches('\n').trim_end_matches('\r').to_string())
                });
                let range = splits[1]
                    .get(row)
                    .map(|s| {
                        let start = result_byte;
                        result_byte += s.len();
                        start..start + s.trim_end_matches('\n').trim_end_matches('\r').len()
                    })
                    .or_else(|| (row == 0 && values[1].is_empty()).then_some(0..0));
                self.heights.push(
                    old_heights
                        .get(&n)
                        .and_then(|h| h.get(row))
                        .copied()
                        .unwrap_or(self.line_height.max(20.) + 4.),
                );
                self.lines.push(Line {
                    chunk: n,
                    first: row == 0,
                    numbers: [
                        text[0].as_ref().map(|_| c.ours_lines.start + row + 1),
                        text[1].as_ref().map(|_| result_line + row + 1),
                        text[2].as_ref().map(|_| c.theirs_lines.start + row + 1),
                    ],
                    text,
                    syntax,
                    result_range: range,
                    base: base_lines
                        .get(row)
                        .map(|s| s.trim_end_matches('\n').trim_end_matches('\r').to_string()),
                    omitted: None,
                });
            }
            result_line += splits[1].len();
        }
        self.offsets.clear();
        self.offsets.push(0.);
        for h in &self.heights {
            self.offsets.push(self.offsets.last().copied().unwrap() + h);
        }
        // Filled for the selected language once, never by rescanning every frame.
        for line in &mut self.lines {
            line.syntax = syntax;
        }
    }
    fn language_states(&mut self, extension: &str) {
        let mut states = [PreviewSyntax::default(); 3];
        for line in &mut self.lines {
            line.syntax = states;
            if let Some(range) = &line.omitted {
                for text in self.model.as_ref().unwrap().chunks[line.chunk]
                    .base
                    .lines()
                    .skip(range.start)
                    .take(range.len())
                {
                    for state in &mut states {
                        state.advance(text, extension);
                    }
                }
                continue;
            }

            for (side, state) in states.iter_mut().enumerate() {
                if let Some(text) = &line.text[side] {
                    state.advance(text, extension);
                }
            }
        }
    }
    fn compare(&mut self, a: usize, b: usize) {
        let source = |which| match which {
            0 => self
                .model
                .as_ref()
                .unwrap()
                .chunks
                .iter()
                .map(|c| c.base.as_str())
                .collect::<String>(),
            1 => self
                .model
                .as_ref()
                .unwrap()
                .chunks
                .iter()
                .map(|c| c.ours.as_str())
                .collect(),
            2 => self
                .model
                .as_ref()
                .unwrap()
                .chunks
                .iter()
                .map(|c| c.theirs.as_str())
                .collect(),
            _ => self.result(),
        };
        let labels = ["Base", "Left", "Right", "Result"];
        self.comparison = Some((labels[a].into(), source(a), labels[b].into(), source(b)));
    }
    pub fn comparison_window(&mut self, ctx: &egui::Context, extension: &str) {
        let Some((left, ltext, right, rtext)) = &self.comparison else {
            return;
        };
        let theme = MergeStyle::get();
        let mut close = false;
        egui::Modal::new(egui::Id::unique("git.merge.compare"))
            .frame(egui::Frame::window(&ctx.style_of(egui::Theme::Dark)).fill(color(theme.chrome)))
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 80.).clamp(280., 1000.));
                ui.horizontal(|ui| {
                    ui.strong(format!("{left} ↔ {right}"));
                    if ui.button("Close Comparison").clicked() {
                        close = true;
                    }
                });
                ui.columns(2, |columns| {
                    for (n, text) in [ltext, rtext].iter().enumerate() {
                        columns[n].label(if n == 0 { left } else { right });
                        egui::ScrollArea::both()
                            .id_salt(("merge.compare.source", n))
                            .max_height(400.)
                            .codisseu()
                            .show(&mut columns[n], |ui| {
                                let mut job = highlight::layout(
                                    text,
                                    extension,
                                    FontId::monospace(theme.font_size),
                                );
                                job.wrap.max_width = ui.available_width();
                                ui.label(job);
                            });
                    }
                });
            });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }
        if close {
            self.comparison = None;
        }
    }
    pub fn toolbar(&mut self, ui: &mut egui::Ui, stage: &mut bool) {
        let mut back = ui
            .ctx()
            .input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F7));
        let mut next = ui
            .ctx()
            .input_mut(|i| i.consume_key(Modifiers::NONE, Key::F7));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            back |= tool(
                ui,
                Some(Icon::Up),
                "",
                "Previous Conflict · Shift+F7",
                self.pending() > 0,
            )
            .clicked();
            next |= tool(
                ui,
                Some(Icon::Down),
                "",
                "Next Conflict · F7",
                self.pending() > 0,
            )
            .clicked();
            ui.separator();
            let compare = tool(ui, Some(Icon::Compare), "", "Compare Contents", true);
            egui::Popup::menu(&compare).show(|ui| {
                for (label, a, b) in [
                    ("Base ↔ Left", 0, 1),
                    ("Base ↔ Right", 0, 2),
                    ("Left ↔ Right", 1, 2),
                    ("Base ↔ Result", 0, 3),
                ] {
                    if ui.button(label).clicked() {
                        self.compare(a, b);
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.label("Apply non-conflicting changes:");
            for (label, side) in [("Left", Some(0)), ("All", None), ("Right", Some(1))] {
                let available = self.draft.full.is_none()
                    && self
                        .model
                        .as_ref()
                        .unwrap()
                        .chunks
                        .iter()
                        .enumerate()
                        .any(|(n, c)| {
                            !self.resolved(n)
                                && matches!(
                                    (c.kind, side),
                                    (MergeKind::OursOnly, None | Some(0))
                                        | (MergeKind::TheirsOnly, None | Some(1))
                                )
                        });
                let response = tool(
                    ui,
                    Some(if side == Some(0) {
                        Icon::MergeNonconflictingLeft
                    } else if side == Some(1) {
                        Icon::MergeNonconflictingRight
                    } else {
                        Icon::MergeNonconflictingAll
                    }),
                    label,
                    label,
                    available,
                );
                if response.clicked() {
                    self.non_conflicting(side);
                }
            }
            ui.separator();
            let enabled = self.draft.full.is_none()
                && self
                    .simple
                    .iter()
                    .enumerate()
                    .any(|(n, s)| s.is_some() && !self.resolved(n));
            let simple = tool(
                ui,
                Some(Icon::MergeSimple),
                "",
                "Resolve Simple Conflicts",
                enabled,
            );
            if simple.clicked() {
                self.checkpoint();
                for (n, value) in self.simple.iter().enumerate() {
                    if !self.resolved(n)
                        && let Some(value) = value
                    {
                        self.draft.values[n] = value.clone();
                        self.draft.manual[n] = true;
                    }
                }
                self.rebuild();
            }
            let collapse = tool(
                ui,
                Some(if self.collapse_unchanged {
                    Icon::ExpandAll
                } else {
                    Icon::CollapseAll
                }),
                "",
                "Collapse Unchanged Fragments",
                true,
            );
            if collapse.clicked() {
                self.collapse_unchanged = !self.collapse_unchanged;
                self.rebuild();
            }
            let settings = tool(ui, Some(Icon::Settings), "", "Merge Settings", true);
            egui::Popup::menu(&settings).show(|ui| {
                if ui
                    .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo Merge"))
                    .clicked()
                {
                    self.undo(false);
                    ui.close();
                }
                if ui
                    .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo Merge"))
                    .clicked()
                {
                    self.undo(true);
                    ui.close();
                }
                ui.separator();
                let mut wrap = !self.no_wrap;
                if ui.checkbox(&mut wrap, "Quebrar linhas").changed() {
                    self.no_wrap = !wrap;
                }
                if ui
                    .add_enabled(
                        self.draft.full.is_none(),
                        egui::Button::new("Editar resultado completo"),
                    )
                    .clicked()
                {
                    self.checkpoint();
                    self.draft.full = Some(self.result());
                    ui.close();
                }
                let target = self.focus.filter(|n| {
                    self.model.as_ref().unwrap().chunks[*n].kind != MergeKind::Unchanged
                        && !self.resolved(*n)
                });
                if ui
                    .add_enabled(
                        self.draft.full.is_none() && target.is_some(),
                        egui::Button::new("Mark Change Resolved"),
                    )
                    .clicked()
                {
                    self.checkpoint();
                    self.draft.manual[target.unwrap()] = true;
                    self.rebuild();
                    ui.close();
                }
                ui.checkbox(stage, "Stage as resolved");
                if ui.button("Copy Result").clicked() {
                    ui.ctx().copy_text(self.result());
                    ui.close();
                }
                if ui.button("Delete File…").clicked() {
                    self.delete_requested = true;
                    ui.close();
                }
                if ui.button("Cancel").clicked() {
                    self.close_requested = true;
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(format!(
                        "{} changes. {} conflicts.",
                        self.pending(),
                        self.conflicts()
                    ))
                    .truncate(),
                );
            });
        });
        if back {
            self.navigate(true);
        }
        if next {
            self.navigate(false);
        }
        // Text edits use their native local undo; merge decisions use this shared history.
        if self.draft.full.is_none() {
            if ui
                .ctx()
                .input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z))
            {
                self.undo(false);
            }
            if ui
                .ctx()
                .input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
            {
                self.undo(true);
            }
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, t: &Tokens, extension: &str, height: f32) {
        if self.draft.full.is_none()
            && ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.modifiers.command && i.key_pressed(Key::A))
        {
            self.checkpoint();
            let text = self.result();
            let len = text.chars().count();
            self.draft.full = Some(text);
            let id = egui::Id::unique("git.merge.result");
            let mut state = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(len),
                )));
            state.store(ui.ctx(), id);
            ui.ctx().memory_mut(|m| m.request_focus(id));
        }
        if self.draft.full.is_some() {
            self.full_editor(ui, t, extension, height);
            return;
        }
        let width = ui.available_width();
        let theme = MergeStyle::get();
        let gap = theme.connector_width;
        let pane = ((width - 2. * gap) / 3.).max(40.);
        let gutter = theme.number_width;
        let actions = theme.actions_width;
        let code_width = (pane - gutter - actions - 2. * theme.code_padding_x).max(8.);
        let line_height = theme.line_height.max(t.editor_font_size + 2.);
        self.line_height = line_height;
        if self.states_dirty {
            self.language_states(extension);
            self.states_dirty = false;
        }
        let key = (
            width.to_bits(),
            t.editor_font_size.to_bits(),
            ui.ctx().pixels_per_point().to_bits(),
            self.no_wrap,
        );
        if self.layout_key != Some(key) {
            self.language_states(extension);
            self.layout_key = Some(key);
            self.layouts.clear();
            self.cache_bytes = 0;
            self.widest = width;
            self.heights = vec![line_height; self.lines.len()];
            self.offsets = (0..=self.lines.len())
                .map(|n| n as f32 * (line_height))
                .collect();
        }
        if self.layouts.len() > 512 || self.cache_bytes > 8 * 1024 * 1024 {
            self.layouts.clear();
            self.cache_bytes = 0;
        }
        let scroll_target = self
            .scroll_to
            .take()
            .and_then(|c| self.lines.iter().position(|l| l.chunk == c))
            .map(|n| self.offsets[n]);
        let mut area = egui::ScrollArea::new([self.no_wrap, true])
            .id_salt("git.merge.aligned")
            .max_height(height)
            .min_scrolled_height(height)
            .auto_shrink([false, false]);
        // Navigate through the same scroll state as mouse/keyboard input.
        if let Some(y) = scroll_target {
            area = area.vertical_scroll_offset(y);
        }
        let mut decision = None;
        let mut edit = None;
        let mut reset = None;
        self.visible_rows = 0;
        area.codisseu().show_viewport(ui, |ui, viewport| {
            let origin=ui.cursor().min;
            let base=origin+egui::vec2(0.,theme.code_padding_y);
            let start = self
                .offsets
                .partition_point(|y| *y <= (viewport.min.y-theme.code_padding_y).max(0.))
                .saturating_sub(1)
                .min(self.lines.len());
            let mut end = start;
            let mut y = self.offsets[start];
            let mut changed = false;
            while end < self.lines.len() && y < viewport.max.y + line_height-theme.code_padding_y {
                let line = &self.lines[end];
                let fresh = !self.layouts.contains_key(&end);
                let conflicting=self.model.as_ref().unwrap().chunks[line.chunk].kind==MergeKind::Conflict && !self.resolved(line.chunk);
                let layouts = self.layouts.entry(end).or_insert_with(|| {
                    std::array::from_fn(|side| {
                        line.text[side].as_ref().map(|text| {
                            let text = expand_tabs(text);
                            let mut job = line.syntax[side].layout(
                                text.as_str(),
                                extension,
                                FontId::monospace(t.editor_font_size),
                            );
                            if conflicting {
                                let other=if side==1{line.text[0].as_deref()}else{line.base.as_deref()}.unwrap_or("");
                                emphasize(&mut job,&expand_tabs(other),color(theme.conflict_word));
                            }
                            for s in &mut job.sections {
                                s.format.line_height = Some(line_height);
                            }
                            job.wrap.max_width = if self.no_wrap {
                                f32::INFINITY
                            } else if side==1 {
                                (pane-gutter-2.*theme.code_padding_x).max(8.)
                            } else { code_width };
                            ui.fonts_mut(|f| f.layout_job(job))
                        })
                    })
                });
                let h = layouts
                    .iter()
                    .flatten()
                    .map(|g| g.size().y)
                    .fold(line_height, f32::max);
                changed |= self.heights[end] != h;
                self.heights[end] = h;
                if fresh {
                    self.cache_bytes += layouts
                        .iter()
                        .flatten()
                        .map(|g| g.job.text.len() + g.num_vertices * 32 + g.num_indices * 4)
                        .sum::<usize>();
                }
                if self.no_wrap {
                    for g in layouts.iter().flatten() {
                        self.widest = self.widest.max(width + (g.size().x - code_width).max(0.));
                    }
                }
                y += h;
                end += 1;
            }
            if changed {
                for n in start..self.heights.len() {
                    self.offsets[n + 1] = self.offsets[n] + self.heights[n];
                }
            }
            ui.set_min_size(egui::vec2(self.widest, *self.offsets.last().unwrap_or(&0.)+2.*theme.code_padding_y));
            let clip = egui::Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
            ui.painter().rect_filled(clip, 0., color(theme.editor));
            for n in start..end {
                self.visible_rows += 1;
                let line = &self.lines[n];
                let chunk = line.chunk;
                let kind = self.model.as_ref().unwrap().chunks[chunk].kind;
                let top = base.y + self.offsets[n];
                let h = self.heights[n];
                let resolved = self.resolved(chunk);
                let tint=if kind==MergeKind::Unchanged{color(theme.editor)}else if resolved{color(theme.added)}else if kind==MergeKind::Conflict{color(theme.conflict)}else{color(theme.modified)};
                if kind != MergeKind::Unchanged {
                    for boundary in 0..2 {
                        let left = base.x + viewport.min.x + pane + boundary as f32 * (pane + gap);
                        let right = left + gap;
                        let (lt, lb) = if line.text[boundary].is_some() {
                            (top, top + h)
                        } else {
                            (top + h / 2., top + h / 2.)
                        };
                        let (rt, rb) = if line.text[boundary + 1].is_some() {
                            (top, top + h)
                        } else {
                            (top + h / 2., top + h / 2.)
                        };
                        ui.painter().add(egui::Shape::convex_polygon(
                            vec![
                                egui::pos2(left, lt),
                                egui::pos2(right, rt),
                                egui::pos2(right, rb),
                                egui::pos2(left, lb),
                            ],
                            tint.gamma_multiply(0.5),
                            egui::Stroke::NONE,
                        ));
                    }
                }
                for side in 0..3 {
                    let x = base.x + viewport.min.x + side as f32 * (pane + gap);
                    let rect = egui::Rect::from_min_size(egui::pos2(x, top), egui::vec2(pane, h));
                    let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                    let cell_tint=if (kind==MergeKind::OursOnly && side==2)||(kind==MergeKind::TheirsOnly && side==0)||(side!=1 && resolved && self.draft.decisions[chunk][if side==0{0}else{1}]==Some(false))||(side==1 && kind!=MergeKind::Conflict && !resolved){color(theme.editor)}else{tint};
                    p.rect_filled(rect, 0., cell_tint);
                    if self.focus == Some(chunk) {
                        p.line_segment(
                            [rect.left_top(), rect.right_top()],
                            egui::Stroke::new(1., color(t.accent)),
                        );
                    }
                    if let Some(number) = line.numbers[side] {
                        p.text(
                            egui::pos2(if side==0{x+pane-7.}else{x+gutter-7.}, top + line_height / 2.),
                            egui::Align2::RIGHT_CENTER,
                            number.to_string(),
                            FontId::monospace(t.editor_font_size - 1.),
                            color(theme.line_number),
                        );
                    }
                    p.line_segment(
                        [egui::pos2(if side==0{x+pane-gutter-actions}else if side==2{x+gutter+actions}else{x+gutter}, top), egui::pos2(if side==0{x+pane-gutter-actions}else if side==2{x+gutter+actions}else{x+gutter}, top+h)],
                        egui::Stroke::new(1., color(theme.border)),
                    );
                    let text_rect = egui::Rect::from_min_max(
                        egui::pos2(x+if side==0{theme.code_padding_x}else{gutter+if side==2{actions}else{0.}+theme.code_padding_x},top),
                        egui::pos2(if side==0{x+pane-gutter-actions-theme.code_padding_x}else{rect.right()-theme.code_padding_x},rect.bottom()),
                    );
                    let mut row_response=None;
                    if side == 1
                        && let Some(range) = &line.result_range
                    {
                        let mut body = self.draft.values[chunk][range.clone()].to_string();
                        let mut edit_layouter =
                            |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
                                let mut job = line.syntax[1].layout(
                                    buffer.as_str(),
                                    extension,
                                    FontId::monospace(t.editor_font_size),
                                );
                                if kind==MergeKind::Conflict&&!resolved{emphasize(&mut job,line.text[0].as_deref().unwrap_or(""),color(theme.conflict_word));}
                                for s in &mut job.sections {
                                    s.format.line_height = Some(line_height);
                                }
                                job.wrap.max_width =
                                    if self.no_wrap { f32::INFINITY } else { wrap };
                                ui.fonts_mut(|f| f.layout_job(job))
                            };
                        let editor_rect = egui::Rect::from_min_size(
                            text_rect.min - egui::vec2(viewport.min.x, 0.),
                            egui::vec2(
                                if self.no_wrap {
                                    self.widest - width + text_rect.width()
                                } else {
                                    text_rect.width()
                                },
                                text_rect.height(),
                            ),
                        );
                        let response = ui
                            .scope_builder(egui::UiBuilder::new().max_rect(editor_rect), |ui| {
                                ui.set_clip_rect(text_rect.intersect(ui.clip_rect()));
                                ui.spacing_mut().interact_size.y=line_height;
                                ui.put(
                                    editor_rect,
                                    egui::TextEdit::multiline(&mut body)
                                        .id(ui.scope_id().with(("merge.result.line", chunk, n)))
                                        .desired_width(editor_rect.width())
                                        .desired_rows(1)
                                        .font(FontId::monospace(t.editor_font_size))
                                        .frame(egui::Frame::NONE)
                                        .margin(0.)
                                        .code_editor()
                                        .layouter(&mut edit_layouter),
                                )
                            })
                            .inner;
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::MultilineTextInput,
                                true,
                                format!("Merge Result line {}", line.numbers[1].unwrap_or(1)),
                            )
                        });
                        row_response=Some(response.clone());
                        if response.changed() {
                            edit = Some((chunk, range.clone(), body));
                        }
                        if response.has_focus() {
                            self.focus = Some(chunk);
                        }
                    } else if let Some(g) = &self.layouts[&n][side] {
                        p.with_clip_rect(text_rect.intersect(p.clip_rect())).galley(
                            egui::pos2(text_rect.left() - viewport.min.x, text_rect.top()),
                            g.clone(),
                            color(t.text),
                        );
                    }
                    let r = row_response.unwrap_or_else(||ui.interact(
                        rect,
                        ui.scope_id().with(("merge.context", n, side)),
                        egui::Sense::click(),
                    ));
                    r.context_menu(|ui| {
                        if kind != MergeKind::Unchanged {
                            if ui.button("Resolve using Left").clicked() {
                                decision = Some((chunk, 0, true, true));
                                ui.close();
                            }
                            if ui.button("Resolve using Right").clicked() {
                                decision = Some((chunk, 1, true, true));
                                ui.close();
                            }
                            if ui.button("Reset Change to Base").clicked() {
                                reset = Some(chunk);
                                ui.close();
                            }
                        }
                    });
                }
                if line.first && !matches!(kind, MergeKind::Unchanged | MergeKind::BothSame) {
                    for side in 0..2 {
                        if (side == 0 && kind == MergeKind::TheirsOnly)
                            || (side == 1 && kind == MergeKind::OursOnly)
                        {
                            continue;
                        }
                        let x = if side==0{base.x+viewport.min.x+pane-gutter-actions}else{base.x+viewport.min.x+2.*(pane+gap)+gutter};
                        for col in 0..2 {
                            let accept = if side == 0 { col == 1 } else { col == 0 };
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(x + col as f32 * 20., top + 2.),
                                egui::vec2(20., 20.),
                            );
                            let label = format!(
                                "{} {} change {}",
                                if accept { "Accept" } else { "Ignore" },
                                if side == 0 { "Left" } else { "Right" },
                                self.change_numbers[chunk]
                            );
                            let r = ui.interact(
                                rect,
                                ui.scope_id().with((&label, n)),
                                egui::Sense::click(),
                            );
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    &label,
                                )
                            });
                            let selected = self.draft.decisions[chunk][side] == Some(accept);
                            if r.hovered() || selected {
                                ui.painter().rect_filled(
                                    rect,
                                    2.,
                                    color(t.database_services_selection),
                                );
                            }
                            components::paint_icon(ui,egui::Rect::from_center_size(rect.center(),egui::vec2(14.,14.)),if accept{if side==0{Icon::MergeAcceptLeft}else{Icon::MergeAcceptRight}}else{Icon::MergeIgnore},color(theme.foreground));
                            let exclusive = r.clicked() && ui.input(|i|
                                i.modifiers.ctrl || i.modifiers.command || i.events.iter().any(|event|
                                    matches!(event,egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers} if rect.contains(*pos) && (modifiers.ctrl||modifiers.command))
                                )
                            );
                            let mut tooltip=egui::Tooltip::for_enabled(&r);
                            tooltip.popup=tooltip.popup.frame(egui::Frame::popup(ui.style()).fill(color(theme.tooltip)).inner_margin(theme.tooltip_padding).corner_radius(6));
                            tooltip.show(|ui|{ui.spacing_mut().item_spacing.y=0.;ui.label(RichText::new(if accept{"Accept"}else{"Ignore"}).color(color(theme.foreground)));ui.label(RichText::new("Ctrl+click to resolve conflict").color(color(theme.foreground)));});
                            if r.clicked(){decision=Some((chunk,side,accept,exclusive));}
                        }
                    }
                }
            }
        });
        if edit.is_some() {
            decision = None;
            reset = None;
        }
        if let Some((n, side, accept, exclusive)) = decision {
            self.choose(n, side, accept, exclusive);
        }
        if let Some(n) = reset {
            self.checkpoint();
            let c = &self.model.as_ref().unwrap().chunks[n];
            self.draft.values[n] = c.base.clone();
            self.draft.manual[n] = false;
            self.draft.decisions[n] = match c.kind {
                MergeKind::OursOnly => [None, Some(false)],
                MergeKind::TheirsOnly => [Some(false), None],
                _ => [None, None],
            };
            self.rebuild();
        }
        if let Some((n, range, text)) = edit {
            self.checkpoint();
            self.draft.values[n].replace_range(range, &text);
            self.draft.manual[n] = false;
            self.rebuild();
        }
    }
    fn full_editor(&mut self, ui: &mut egui::Ui, t: &Tokens, extension: &str, height: f32) {
        if self.full_sources.is_none() {
            self.full_sources = Some(std::array::from_fn(|side| {
                self.model
                    .as_ref()
                    .unwrap()
                    .chunks
                    .iter()
                    .map(|c| {
                        if side == 0 {
                            c.ours.as_str()
                        } else {
                            c.theirs.as_str()
                        }
                    })
                    .collect()
            }));
        }
        let mut reviewed = self.draft.full_reviewed;
        let mut review_changed = false;
        let header = ui
            .horizontal_wrapped(|ui| {
                ui.label("Resultado completo · Undo Merge volta às escolhas por trecho");
                review_changed = ui
                    .checkbox(&mut reviewed, "Marcar resultado como resolvido")
                    .changed();
            })
            .response
            .rect
            .height();
        let body_height = (height - header - 8.).max(60.);
        ui.columns(3, |columns| {
            for (side, column) in columns.iter_mut().enumerate() {
                egui::ScrollArea::vertical()
                    .id_salt(("merge.full", side))
                    .max_height(body_height)
                    .min_scrolled_height(body_height)
                    .codisseu()
                    .show(column, |ui| {
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(
                                MergeStyle::get().code_padding_x as i8,
                                MergeStyle::get().code_padding_y as i8,
                            ))
                            .show(ui, |ui| {
                                let mut layouter =
                                    |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
                                        let mut job = highlight::layout(
                                            buffer.as_str(),
                                            extension,
                                            FontId::monospace(t.editor_font_size),
                                        );
                                        job.wrap.max_width = wrap;
                                        ui.fonts_mut(|f| f.layout_job(job))
                                    };
                                if side == 1 {
                                    let response = ui.add(
                                        egui::TextEdit::multiline(
                                            self.draft.full.as_mut().unwrap(),
                                        )
                                        .id(egui::Id::unique("git.merge.result"))
                                        .code_editor()
                                        .desired_width(ui.available_width())
                                        .desired_rows((body_height / 22.) as usize)
                                        .layouter(&mut layouter),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::accesskit::Role::MultilineTextInput,
                                            true,
                                            "Merge Result",
                                        )
                                    });
                                    if response.changed() {
                                        self.draft.full_reviewed = false;
                                        self.revision = self.revision.wrapping_add(1);
                                    }
                                } else {
                                    let text = &self.full_sources.as_ref().unwrap()
                                        [if side == 0 { 0 } else { 1 }];
                                    let mut job = highlight::layout(
                                        text.as_str(),
                                        extension,
                                        FontId::monospace(t.editor_font_size),
                                    );
                                    job.wrap.max_width = ui.available_width();
                                    ui.label(job);
                                }
                            });
                    });
            }
        });
        if review_changed {
            self.draft.full_reviewed = reviewed;
            self.revision = self.revision.wrapping_add(1);
        }
    }
}
fn draft_bytes(d: &Draft) -> usize {
    d.values.iter().map(String::len).sum::<usize>() + d.full.as_ref().map_or(0, String::len)
}
fn expand_tabs(s: &str) -> String {
    let mut out = String::new();
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = 4 - col % 4;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}
fn emphasize(job: &mut egui::text::LayoutJob, other: &str, fill: egui::Color32) {
    let text = &job.text;
    if text.len() > 65536 || text == other {
        return;
    }
    let mut start = 0;
    for (a, b) in text.chars().zip(other.chars()) {
        if a != b {
            break;
        }
        start += a.len_utf8();
    }
    let mut suffix = 0;
    for (a, b) in text[start..]
        .chars()
        .rev()
        .zip(other[start..].chars().rev())
    {
        if a != b {
            break;
        }
        suffix += a.len_utf8();
    }
    let end = text.len() - suffix;
    if start >= end {
        return;
    }
    let mut sections = Vec::new();
    for section in std::mem::take(&mut job.sections) {
        let a = section.byte_range.start.0;
        let b = section.byte_range.end.0;
        let x = a.max(start);
        let y = b.min(end);
        if x >= y {
            sections.push(section);
            continue;
        }
        if a < x {
            let mut part = section.clone();
            part.byte_range = a.into()..x.into();
            sections.push(part);
        }
        let mut middle = section.clone();
        middle.byte_range = x.into()..y.into();
        middle.format.background = fill;
        sections.push(middle);
        if y < b {
            let mut part = section;
            part.byte_range = y.into()..b.into();
            sections.push(part);
        }
    }
    job.sections = sections;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    #[test]
    fn collapsing_context_keeps_result_and_full_lexical_context_and_precise_numbering() {
        let base = format!("<?php\n/*\n{}*/\nreturn 1;\n", " comment line\n".repeat(40));
        let ours = base.replace("return 1", "return 2");
        let theirs = base.replace("return 1", "return 3");
        let model =
            Arc::new(TextMerge::build(&base, &ours, &theirs, &AtomicBool::new(false)).unwrap());
        let mut ui = MergeUi::default();
        ui.sync("fixture".into(), model, None);
        let original = ui.result();
        let count = ui.lines.len();
        ui.collapse_unchanged = true;
        ui.rebuild();
        ui.language_states("php");
        assert!(ui.lines.len() < count);
        assert_eq!(ui.result(), original);
        assert!(ui.lines.iter().any(|l| l.omitted.is_some()));
        let line = ui
            .lines
            .iter()
            .find(|l| l.text[1].as_deref() == Some("return 1;"))
            .unwrap();
        assert_eq!(line.numbers[1], Some(44));
        let job = line.syntax[1].layout("return 1;", "php", FontId::monospace(13.));
        let plain = highlight::layout("return 1;", "php", FontId::monospace(13.));
        assert_eq!(job.sections[0].format.color, plain.sections[0].format.color);
        ui.collapse_unchanged = false;
        ui.rebuild();
        assert_eq!(ui.lines.len(), count);
        assert_eq!(ui.result(), original);
    }
    #[test]
    fn word_emphasis_preserves_utf8_bytes_and_only_marks_the_changed_range() {
        let text = "return 'ação 😀 old';";
        let mut job = highlight::layout(text, "php", FontId::monospace(13.));
        let fill = color(MergeStyle::get().conflict_word);
        emphasize(&mut job, "return 'ação 😀 new';", fill);
        assert_eq!(job.text, text);
        let marked = job
            .sections
            .iter()
            .filter(|s| s.format.background == fill)
            .map(|s| &text[s.byte_range.start.0..s.byte_range.end.0])
            .collect::<String>();
        assert_eq!(marked, "old");
    }
}
