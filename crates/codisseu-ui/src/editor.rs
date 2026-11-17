//! Native virtual code editor. Text, directed selection and history belong to the domain.
use super::*;
use codisseu_core::{Selection, TextSnapshot, ViewId};
use std::{collections::BTreeMap, time::Instant};
mod accessibility;
mod commands;
pub(super) mod debug_marker;
mod decorations;
mod delimiters;
mod folding;
mod ghost;
mod git_changes;
mod inlays;
mod input;
mod keymap;
mod layout;
mod lexical;
mod links;
mod multi;
mod navigation_gutter;
mod phpdoc;
mod python;
mod selection_highlight;
mod soft_wrap;
#[cfg(test)]
mod soft_wrap_tests;
pub(crate) use navigation_gutter::NavigationGutter;
mod tiles;

/// Lightweight lexical context for read-only fragments; no document or provider.
#[derive(Clone, Copy)]
pub(super) struct PreviewSyntax(lexical::State);
impl Default for PreviewSyntax {
    fn default() -> Self {
        Self(lexical::State::Normal)
    }
}
impl PreviewSyntax {
    pub fn advance(&mut self, text: &str, extension: &str) {
        self.0 = lexical::preview_advance(text, extension, self.0);
    }
    pub fn layout(self, text: &str, extension: &str, font: FontId) -> egui::text::LayoutJob {
        lexical::layout(text, extension, font, Some(self.0))
    }
}

/// Read-only semantic previews share the editor's shaping and ligature policy.
/// The caller supplies a bounded line and keeps its original scalar positions.
pub(super) fn preview_layout(
    ui: &egui::Ui,
    text: &str,
    extension: &str,
    tokens: &Tokens,
    ligatures: bool,
) -> (std::sync::Arc<egui::Galley>, Vec<usize>) {
    preview_fragment_layout(ui, text, extension, tokens, ligatures, None)
}
pub(super) fn preview_layout_with_syntax(
    ui: &egui::Ui,
    text: &str,
    extension: &str,
    tokens: &Tokens,
    ligatures: bool,
    syntax: PreviewSyntax,
) -> (std::sync::Arc<egui::Galley>, Vec<usize>) {
    preview_fragment_layout(ui, text, extension, tokens, ligatures, Some(syntax))
}
fn preview_fragment_layout(
    ui: &egui::Ui,
    text: &str,
    extension: &str,
    tokens: &Tokens,
    ligatures: bool,
    syntax: Option<PreviewSyntax>,
) -> (std::sync::Arc<egui::Galley>, Vec<usize>) {
    let mut rendered = String::with_capacity(text.len());
    let mut positions = vec![0];
    let mut column = 0;
    for c in text.trim_end_matches('\r').chars() {
        if c == '\t' {
            let spaces = 4 - column % 4;
            rendered.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            rendered.push(c);
            column += 1;
        }
        positions.push(column);
    }
    let font = FontId::monospace(tokens.editor_font_size);
    let mut job = if let Some(syntax) = syntax {
        syntax.layout(&rendered, extension, font)
    } else {
        highlight::layout(&rendered, extension, font)
    };
    layout::prepare_job(&mut job, tokens, ligatures);
    (ui.fonts_mut(|f| f.layout_job(job)), positions)
}

#[derive(Default)]
pub(super) struct EditorBridge {
    views: BTreeMap<(ViewId, DocumentId), EditorView>,
    pub regions: BTreeMap<ViewId, egui::Rect>,
    pub view_metrics: BTreeMap<ViewId, EditorMetrics>,
    pub metrics: EditorMetrics,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct EditorMetrics {
    pub document: Option<DocumentId>,
    pub lines: usize,
    pub visible_start: usize,
    pub visible_end: usize,
    pub layouts_built: usize,
    pub cache_hits: usize,
    pub cached_lines: usize,
    pub cache_bytes: usize,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub frame_micros: u128,
    pub composing: bool,
    /// Whether the computed caret rectangle lies within this frame's viewport.
    pub caret_in_viewport: bool,
    /// Accepted semantic Unnecessary spans intersecting the visible lines.
    pub unnecessary_spans: usize,
    pub inlay_hints: usize,
    pub layout_pending: bool,
    pub highlighting_pending: bool,
    pub layout_error: Option<String>,
    pub selection_count: usize,
    pub selection_highlights: usize,
    pub selection_highlight_scanned_chars: usize,
    pub selection_highlight_pending: bool,
    pub selection_highlight_limited: bool,
    pub folding_pending: bool,
    pub folding_regions: usize,
    pub folded_lines: usize,
    pub folding_partial: bool,
    pub navigation_symbols: usize,
    pub navigation_pending: bool,
    pub navigation_parses: u64,
    pub navigation_incremental_parses: u64,
    pub navigation_partial: bool,
    /// View-only Copilot rows, absent from source line count/history.
    pub copilot_ghost_rows: usize,
    pub copilot_ghost_visible: bool,
    /// Visible ghost glyph rows in logical screen coordinates; never source ranges.
    pub copilot_ghost_rects: Vec<[f32; 4]>,
    pub soft_wrap: bool,
    /// Estimated until unvisited source lines have been measured.
    pub visual_rows: usize,
    pub wrapped_lines: usize,
    pub wrap_pending: bool,
    pub wrap_row_rects: Vec<WrapRowMetrics>,
    pub wrap_index_bytes: usize,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct WrapRowMetrics {
    pub source_line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub rect: [f32; 4],
}
#[derive(Default)]
struct EditorView {
    wrap: soft_wrap::WrapMap,
    wrap_previous_affinity: bool,
    wrap_anchor: Option<(usize, usize, f32)>,
    ghost: Option<ghost::Ghost>,
    cache: layout::LineCache,
    inlay_stamp: Option<(u64, u64, bool)>,
    preview_cache: layout::LineCache,
    composition_revision: u64,
    scroll: egui::Vec2,
    viewport_size: egui::Vec2,
    stamp: Option<(u64, Selection)>,
    reveal: bool,
    had_focus: bool,
    last_interaction: f64,
    preferred_x: Option<f32>,
    drag_anchor: Option<usize>,
    /// Source position at the secondary click, independent of a preserved selection.
    context_position: Option<(u64, usize)>,
    definition_click: Option<(usize, bool)>,
    link_hover: Option<links::Hover>,
    composition: Option<Composition>,
    ime_cancelled: bool,
    crlf: bool,
    pairs: delimiters::Pairs,
    matching_stamp: Option<(u64, usize)>,
    matching: Option<[usize; 2]>,
    accessible_positions: BTreeMap<egui::accesskit::NodeId, (usize, usize)>,
    multi: multi::MultiSelection,
    selection_highlights: selection_highlight::SelectionHighlights,
    chord: Option<f64>,
    owned_text: Option<(Key, f64)>,
    selection_expansions: Vec<Selection>,
    selection_expansion_stamp: Option<u64>,
    fold: folding::FoldMap,
    fold_service: folding::FoldService,
    fold_requested: Option<u64>,
    fold_truncated: bool,
    indentation: Option<codisseu_editor::commands::Indentation>,
}
struct Composition {
    version: u64,
    selection: Selection,
    text: String,
    active: Option<std::ops::Range<usize>>,
}
pub(super) fn editor_id(id: ViewId) -> egui::Id {
    egui::Id::unique(("codisseu.editor", id))
}
fn content_end(s: &TextSnapshot, line: usize) -> usize {
    let start = s.line_to_char(line);
    let text = s.line(line);
    let mut end = start + text.len_chars();
    while end > start && matches!(s.char(end - 1), '\r' | '\n') {
        end -= 1;
    }
    end
}
impl CodisseuApp {
    /// Template triggers must stay in a known code context. Reuse the editor's
    /// background line lexer and inspect only this line's bounded prefix.
    pub(super) fn editor_template_context(&self, at: usize) -> bool {
        let Some(doc) = self.service.active_document() else {
            return false;
        };
        if !matches!(
            codisseu_core::snippets::context(&doc.path),
            Some(codisseu_core::snippets::Context::Php)
        ) {
            return true;
        }
        let Some(view_id) = self.service.active_view() else {
            return false;
        };
        let Some(view) = self.editor_bridge.views.get(&(view_id, doc.id)) else {
            return false;
        };
        let snapshot = doc.snapshot();
        if at > snapshot.len_chars() {
            return false;
        }
        let line = snapshot.char_to_line(at);
        let start = snapshot.line_to_char(line);
        if at - start > 4096 {
            return false;
        }
        let Some(state) = view.cache.lexical_state(line, "php") else {
            return false;
        };
        let prefix = snapshot.slice(start..at).to_string();
        if lexical::preview_advance(&prefix, "php", state) != lexical::State::Normal {
            return false;
        }
        // A template is a statement/declaration, never an object member or an
        // argument fragment with a coincidentally identical identifier.
        let prefix = prefix.trim_end();
        prefix.is_empty()
            || prefix.ends_with("<?php")
            || prefix.ends_with(['{', '}', ';'])
            || (prefix.ends_with(':')
                && !prefix.ends_with("::")
                && (prefix.starts_with("case ") || prefix == "default:"))
    }

    /// Global editor shortcuts can own a native text companion arriving in the
    /// following frame. Reuse the existing, view-scoped chord text guard.
    pub(super) fn editor_claim_shortcut_text(&mut self, ctx: &egui::Context, key: Key) {
        let Some(view_id) = self.service.active_view() else {
            return;
        };
        let Some(document) = self.service.view_document(view_id).map(|doc| doc.id) else {
            return;
        };
        self.editor_bridge
            .views
            .entry((view_id, document))
            .or_default()
            .owned_text = Some((key, ctx.input(|input| input.time)));
    }
    pub(super) fn editor_owns_chord(&self, ctx: &egui::Context) -> bool {
        if !self.editor_focused(ctx) {
            return false;
        }
        let armed = self
            .service
            .active_view()
            .and_then(|v| {
                self.service
                    .view_document(v)
                    .and_then(|d| self.editor_bridge.views.get(&(v, d.id)))
            })
            .and_then(|v| v.chord)
            .is_some_and(|time| ctx.input(|i| i.time) - time < 5.);
        armed || ctx.input(|i|i.events.iter().any(|e|matches!(e,egui::Event::Key{key:Key::K,pressed:true,modifiers,..} if (modifiers.ctrl||modifiers.command)&&!modifiers.shift&&!modifiers.alt)))
    }
    pub(super) fn editor_actions_menu(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let extension = self
            .service
            .active_document()
            .map(|d| highlight::syntax_key(&d.path))
            .unwrap_or_default();
        ui.add_enabled_ui(self.service.active_document().is_some(), |ui| {
            ui.menu_button("Ações de edição", |ui| {
                for (label, action) in commands::menu_items(&extension) {
                    if ui.button(label).clicked() {
                        chosen = Some(action);
                        ui.close();
                    }
                }
            });
        });
        if let Some(action) = chosen {
            let Some(view_id) = self.service.active_view() else {
                return;
            };
            let Some(doc) = self.service.active_document() else {
                return;
            };
            let id = doc.id;
            let path = doc.path.clone();
            let ext = highlight::syntax_key(&path);
            let mut bridge = std::mem::take(&mut self.editor_bridge);
            let rect = bridge
                .regions
                .get(&view_id)
                .copied()
                .unwrap_or(ui.max_rect());
            let view = bridge.views.entry((view_id, id)).or_default();
            commands::run(
                ui,
                &mut self.service,
                id,
                view,
                &ext,
                &self.tokens,
                action,
                rect,
            );
            self.editor_bridge = bridge;
            if markdown::is_markdown(&path)
                && self.markdown.mode(view_id, &path) == markdown::MarkdownMode::Preview
            {
                self.markdown
                    .set_mode(view_id, &path, markdown::MarkdownMode::Split);
            }
            self.focus_editor = true;
        }
    }
    pub fn editor_metrics(&self) -> &EditorMetrics {
        &self.editor_bridge.metrics
    }
    pub(super) fn editor(&mut self, ui: &mut egui::Ui, view_id: ViewId) {
        let started = Instant::now();
        let Some(doc) = self.service.view_document(view_id) else {
            ui.vertical_centered(|ui| {
                ui.add_space(100.);
                ui.heading("Seu workspace, no Codisseu");
                ui.label(
                    "Abra arquivos PHP, JavaScript, TypeScript, Rust, SQL ou outros textos UTF-8.",
                );
                if ui.button("Escolher pasta").clicked() {
                    self.open_path(PathAction::Workspace);
                }
            });
            return;
        };
        let id = doc.id;
        let initial_version = doc.version;
        let mut completion_signature = false;
        let path = doc.path.clone();
        let extension = highlight::syntax_key(&path);
        let mut bridge = std::mem::take(&mut self.editor_bridge);
        bridge.views.retain(|(owner, id), _| {
            self.service
                .view(*owner)
                .is_some_and(|v| v.tabs().contains(id))
        });
        for (owner, view) in &mut bridge.views {
            if Some(owner.0) != self.service.active_view() && view.composition.take().is_some() {
                view.ime_cancelled = true;
            }
        }
        let is_active = self.service.active_view() == Some(view_id);
        if is_active {
            if matches!(extension.as_str(), "php" | "phtml") {
                self.php_navigation.schedule(
                    id,
                    &path,
                    initial_version,
                    doc.snapshot(),
                    doc.last_change(),
                );
            } else {
                self.php_navigation.cancel();
            }
            self.php_navigation.poll();
            if self.php_navigation.pending() {
                ui.ctx().request_repaint_after(Duration::from_millis(25));
            }
            if let Some(error) = &self.php_navigation.error {
                crate::status_messages::publish(ui.ctx(), error);
            }
            if self.php_navigation.truncated {
                crate::status_messages::publish(
                    ui.ctx(),
                    "Indicadores PHP parciais: limite de1MiB,2048 declarações ou tempo de análise atingido",
                );
            }
        }
        let navigation_anchors = self.php_navigation.anchors(id, initial_version);
        let navigation_stats = self.php_navigation.stats();
        let navigation_pending = self.php_navigation.pending();
        let navigation_partial = self.php_navigation.truncated;
        let view = bridge.views.entry((view_id, id)).or_default();
        if view.wrap_anchor.is_none()
            && (self.service.editor_preferences().soft_wrap
                || (view.stamp.is_none()
                    && self
                        .service
                        .view(view_id)
                        .is_some_and(|v| v.has_restored_viewport())))
            && let Some(anchor) = self
                .service
                .view(view_id)
                .and_then(|view| view.source_scroll_anchor())
        {
            let snapshot = doc.snapshot();
            let character = anchor.character.min(snapshot.len_chars());
            view.wrap_anchor = Some((snapshot.char_to_line(character), character, anchor.offset));
        }

        view.scroll = egui::Vec2::from(self.service.view(view_id).unwrap().scroll());
        if view.stamp.is_none() {
            let snapshot = doc.snapshot();
            let line = snapshot.line(0);
            let n = line.len_chars();
            view.indentation = Some(input::detect_indentation(&snapshot));
            view.crlf = n >= 2 && line.char(n - 2) == '\r' && line.char(n - 1) == '\n';
        }
        // Apply programmatic movement before input; history endpoints are already canonical.
        let mut target = None;
        if is_active && let Some((p, l, c)) = self.jump_to.take() {
            if p == path {
                let s = doc.snapshot();
                if (l as usize) < s.len_lines() {
                    let line = s.line(l as usize).to_string();
                    if let Some(b) = codisseu_core::utf16_byte_offset(&line, 0, c) {
                        target = Some(Selection::caret(
                            s.line_to_char(l as usize) + line[..b].chars().count(),
                        ));
                    }
                }
            } else {
                self.jump_to = Some((p, l, c));
            }
        }
        if is_active
            && let Some((owner, v, r)) = self.search_panel.selection.take()
            && owner == id
            && v == initial_version
        {
            target = Some(Selection {
                anchor: r.start,
                head: r.end,
            });
        }
        if is_active
            && let Some(s) = self.service.navigation.selection.take()
            && s.document_id == id
            && s.version == initial_version
        {
            target = Some(Selection::caret(s.range.start));
            self.focus_editor = true;
        }
        if is_active
            && let Some(s) = self.service.completion.selection.take()
            && s.document_id == id
            && s.version == initial_version
        {
            target = Some(Selection::caret(s.character_index));
            completion_signature = s.trigger_signature;
            self.focus_editor = true;
        }
        if let Some(selection) = target {
            self.send(Command::SetSelection {
                id,
                expected_version: self.service.view_document(view_id).unwrap().version,
                selection,
            });
            view.reveal = true;
        }
        let doc = self.service.view_document(view_id).unwrap();
        let mut snapshot = doc.snapshot();
        let mut selection = self.service.view(view_id).unwrap().selection();
        if view
            .ghost
            .as_ref()
            .is_some_and(|g| g.version != doc.version || selection != Selection::caret(g.caret))
        {
            view.ghost = None;
        }
        if view
            .stamp
            .is_some_and(|stamp| stamp != (doc.version, selection))
        {
            view.wrap_previous_affinity = false;
        }
        if (view.stamp.is_none() && !self.service.view(view_id).unwrap().has_restored_viewport())
            || (is_active && view.stamp.is_some() && view.stamp != Some((doc.version, selection)))
        {
            view.reveal = true;
        }
        let revision = doc.content_identity();
        if view.fold.revision() != revision || view.fold.line_count() != snapshot.len_lines() {
            view.fold.sync(revision, snapshot.len_lines());
            view.fold_service.cancel();
            view.fold_requested = None;
            view.fold_truncated = false;
        }
        if view.fold_requested != Some(revision)
            && view
                .fold_service
                .request(revision, snapshot.clone(), extension.clone())
                .is_ok()
        {
            view.fold_requested = Some(revision);
        }
        if let Some(result) = view.fold_service.poll() {
            if result.revision == revision {
                view.fold_truncated = result.inspection.truncated;
            }
            view.fold
                .replace_ranges(result.revision, result.line_count, result.inspection.ranges);
        }
        if view.fold_service.pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if view.reveal {
            view.fold.reveal_selection(&snapshot, selection);
        }
        let ligatures = self.service.editor_preferences().ligatures;
        view.cache.set_ligatures(ligatures);
        view.preview_cache.set_ligatures(ligatures);
        view.cache.begin_frame();
        let inlay_stamp = (
            doc.version,
            self.service.inlay_hints.revision,
            self.service.inlay_hints_enabled(id),
        );
        if view.inlay_stamp != Some(inlay_stamp) {
            let hints = inlays::collect(&self.service, id, doc.version, &snapshot);
            view.cache.set_inlays(
                doc.last_change().map_or(0, |c| c.revision),
                self.service.inlay_hints.revision,
                hints,
            );
            view.inlay_stamp = Some(inlay_stamp);
        }
        view.cache.begin_document(
            snapshot.clone(),
            doc.last_change().map_or(0, |c| c.revision),
            doc.last_change(),
            &extension,
        );
        view.cache.sync(
            doc.last_change().map_or(0, |c| c.revision),
            doc.last_change(),
            ui,
            &self.tokens,
        );
        if view.matching_stamp != Some((doc.version, selection.head))
            || view.cache.lexical_pending()
        {
            view.matching = if selection.is_caret() {
                delimiters::matching(view, &snapshot, selection.head, &extension)
            } else {
                None
            };
            // Retry an unknown lexical state after the background result arrives.
            view.matching_stamp =
                (!view.cache.lexical_pending()).then_some((doc.version, selection.head));
        }
        let group_clip = ui.clip_rect();
        let mut all = ui.available_rect_before_wrap();
        // In a vertical split the second editor's gutter is measured from the
        // divider itself. Its clip still starts after that one-pixel divider.
        if !self.service.editor_split().horizontal
            && self.service.document_views().get(1).map(|v| v.id) == Some(view_id)
        {
            all.min.x -= 1.;
        }
        let line_height = self.tokens.editor_line_height;
        let count = snapshot.len_lines();
        let blame_width = self
            .git_insights
            .gutter_width(
                &path,
                initial_version,
                is_active && self.service.editor_preferences().git_file_blame,
            )
            .min(
                (all.width() * self.tokens.git_blame_width_fraction)
                    .min(self.tokens.git_blame_max_width),
            );
        let blame_width = if blame_width >= 160. { blame_width } else { 0. };
        let navigation_width = if matches!(extension.as_str(), "php" | "phtml") {
            14.
        } else {
            0.
        };
        // Reserve Git independently of the PHP/navigation and debug lanes.
        // Keeping the slot even with no changes avoids shifting code on stage.
        let gutter_width = self.tokens.editor_git_lane_width
            + navigation_width
            + blame_width
            + self.tokens.editor_gutter_width
            + (count.to_string().len().saturating_sub(2) as f32) * 7.;
        let code = egui::Rect::from_min_max(
            all.min + egui::vec2(gutter_width + 8., self.tokens.editor_top_padding),
            all.max - egui::vec2(0., self.tokens.editor_bottom_padding),
        );
        let soft_wrap = self.service.editor_preferences().soft_wrap;
        let old_row = (view.scroll.y / line_height).floor().max(0.) as usize;
        let old_top = if view.wrap.enabled {
            view.wrap.source_line(&view.fold, old_row)
        } else {
            view.fold.source_line(old_row)
        };
        let old_top_char = {
            let source = if view.wrap.enabled {
                soft_wrap::pointer_position(view, old_row, 0.)
            } else {
                Some((old_top, view.scroll.x))
            };
            source
                .and_then(|(line, x)| {
                    let layout = view
                        .cache
                        .get(ui, &snapshot, line, &extension, &self.tokens);
                    layout
                        .column(ui, &snapshot, &self.tokens, x)
                        .map(|column| snapshot.line_to_char(line) + column)
                })
                .unwrap_or_else(|| snapshot.line_to_char(old_top))
        };
        let old_caret_y = {
            let line = snapshot.char_to_line(selection.head.min(snapshot.len_chars()));
            let layout = view
                .cache
                .get(ui, &snapshot, line, &extension, &self.tokens);
            layout
                .x(
                    ui,
                    &snapshot,
                    &self.tokens,
                    selection.head - snapshot.line_to_char(line),
                )
                .and_then(|x| soft_wrap::caret_position(view, line, x, view.wrap_previous_affinity))
                .map(|(_, row)| row as f32 * line_height)
        };
        let was_caret_visible = old_caret_y.is_some_and(|y| {
            y >= view.scroll.y && y + line_height <= view.scroll.y + view.viewport_size.y
        });
        let wrap_changed = view.wrap.sync(
            soft_wrap,
            code.width() - self.tokens.editor_wrap_right_padding,
            if view.composition.is_some() {
                view.composition_revision
            } else {
                revision
            },
            &self.tokens,
            ligatures,
            view.ghost.as_ref(),
            (
                inlay_stamp.1,
                inlay_stamp.2,
                ui.ctx().pixels_per_point().to_bits(),
                view.cache.geometry_epoch(),
            ),
        );
        view.wrap.begin_frame();
        if wrap_changed && view.wrap_anchor.is_none() {
            view.wrap_anchor = Some((old_top, old_top_char, view.scroll.y % line_height));
        }
        let extra_points = [
            selection.anchor.min(snapshot.len_chars()),
            view.wrap_anchor.map_or(selection.head, |(_, point, _)| {
                point.min(snapshot.len_chars())
            }),
        ];
        if soft_wrap {
            view.scroll.x = 0.;
            view.wrap.prepare(
                ui,
                &snapshot,
                &mut view.cache,
                &view.fold,
                view.ghost.as_ref(),
                (view.scroll.y / line_height).floor().max(0.) as usize
                    ..((view.scroll.y + code.height()) / line_height)
                        .ceil()
                        .max(0.) as usize
                        + 2,
                selection.head.min(snapshot.len_chars()),
                &extra_points,
                &extension,
                &self.tokens,
            );
        }
        if let Some((_, point, offset)) = view.wrap_anchor {
            let point = point.min(snapshot.len_chars());
            let line = snapshot.char_to_line(point);
            let layout = view
                .cache
                .get(ui, &snapshot, line, &extension, &self.tokens);
            if let Some((_, row)) = layout
                .x(
                    ui,
                    &snapshot,
                    &self.tokens,
                    point - snapshot.line_to_char(line),
                )
                .and_then(|x| soft_wrap::caret_position(view, line, x, false))
            {
                view.scroll.y = row as f32 * line_height + offset;
                view.wrap_anchor = None;
            }
        }
        if (wrap_changed || view.viewport_size != code.size()) && was_caret_visible {
            view.reveal = true;
        }
        view.viewport_size = code.size();
        let mut response = ui.interact(code, editor_id(view_id), egui::Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::MultilineTextInput,
                true,
                path.to_string_lossy(),
            )
        });
        if is_active
            && self.focus_editor
            && self.path_dialog.is_none()
            && !self.provider_dialog
            && !self.service.provider_startup().needs_trust
            && self.project_files_ui.input.is_none()
            && self.project_files_ui.trash.is_none()
            && !self.snippet_ui.is_open()
            && !self.debugger_ui.dialog
            && !self.rust_tools.open
            && !self.python_ui.open
            && !self.python_tests_ui.settings
            && !self.js_tests_ui.settings
            && !self.php_tests_ui.settings
            && self.debugger_ui.breakpoint.is_none()
            && !self.database_ui.dialog
            && !self.database_ui.exit_confirm
            && self.database_ui.preview.is_none()
            && self.database_ui.edit.is_none()
            && !self.preferences_dialog
            && !self.project_search.open
            && self.confirm.is_none()
            && ui.memory(|m| m.top_modal_layer().is_none())
        {
            if response.has_focus() && self.service.active_view() == Some(view_id) {
                self.focus_editor = false;
            } else {
                response.request_focus();
            }
        }
        if response.has_focus() && self.service.active_view() == Some(view_id) {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    editor_id(view_id),
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
        }
        let reveal_requested = view.reveal;
        if view.reveal {
            let line = snapshot.char_to_line(selection.head.min(snapshot.len_chars()));
            let l = view
                .cache
                .get(ui, &snapshot, line, &extension, &self.tokens);
            let x = l.x(
                ui,
                &snapshot,
                &self.tokens,
                selection.head - snapshot.line_to_char(line),
            );
            let position = x.and_then(|x| {
                soft_wrap::caret_position(view, line, x, view.wrap_previous_affinity)
            });
            let y =
                position.map_or(ghost::visual_row(view, line), |(_, row)| row) as f32 * line_height;
            let h = (code.height() - 16.).max(line_height);
            if selection.is_caret() && selection.head == snapshot.len_chars() {
                // Keep EOF context below the caret only while it fits. A short
                // split viewport must never scroll the caret above its own top
                // merely to expose the virtual five-line trailing spacer.
                view.scroll.y = ((if soft_wrap {
                    view.wrap.total(&view.fold)
                } else {
                    view.fold.visible_line_count()
                }) as f32
                    * line_height
                    + 5. * line_height
                    - code.height())
                .max(0.)
                .min(y);
            } else if y < view.scroll.y {
                view.scroll.y = y;
            } else if y + line_height > view.scroll.y + h {
                view.scroll.y = (y + line_height - h).max(0.);
            }
            if let Some(x) = x.filter(|_| !soft_wrap || position.is_some()) {
                let x = position.map_or(x, |(x, _)| x);
                if !soft_wrap {
                    if x < view.scroll.x {
                        view.scroll.x = x;
                    } else if x + 8. > view.scroll.x + code.width() - 20. {
                        view.scroll.x = (x + 28. - code.width()).max(0.);
                    }
                }
                view.reveal = false;
            } else {
                ui.ctx().request_repaint();
            }
        }

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(code).id_salt((
            "codisseu.editor.viewport",
            view_id,
            id,
        )));
        child.set_clip_rect(code.intersect(ui.clip_rect()));
        // Preserve the code viewport gutter and motion settings. Shared decoration
        // paints the thin handle without changing this viewport or content styles.
        let style = child.style_mut();
        style.spacing.scroll = egui::style::ScrollStyle {
            bar_width: 8.,
            bar_inner_margin: 0.,
            ..egui::style::ScrollStyle::solid()
        };
        style.spacing.scroll.fade.strength = 0.;
        style.spacing.scroll.kinetic.decay_time = 0.;
        style.spacing.scroll.kinetic.rubber_band = false;

        let mut visible = 0..0;
        let mut unnecessary_spans = 0;
        let mut selection_highlights = 0;
        let mut cursor_rect = egui::Rect::NOTHING;
        let mut rename_rect = None;
        let mut fold_toggle = None;
        let mut fold_targets = Vec::new();
        let mut breakpoint_toggle = None;
        let mut breakpoint_targets = Vec::new();
        let mut authorship_targets = Vec::new();
        let debug_source = matches!(
            extension.as_str(),
            "php"
                | "phtml"
                | "js"
                | "jsx"
                | "mjs"
                | "cjs"
                | "ts"
                | "tsx"
                | "mts"
                | "cts"
                | "rs"
                | "py"
                | "pyw"
        );
        let selected_debug_session = self
            .service
            .debugger
            .sessions
            .iter()
            .find(|session| Some(session.id) == self.debugger_ui.selected_session);
        let breakpoints: std::collections::BTreeMap<_, _> = self
            .service
            .debugger
            .breakpoints
            .iter()
            .filter(|point| point.path == path)
            .map(|point| {
                let checked = selected_debug_session.and_then(|session| {
                    session.breakpoints.iter().find(|candidate| {
                        candidate.path == point.path && candidate.line == point.line
                    })
                });
                (
                    point.line.saturating_sub(1) as usize,
                    (
                        debug_marker::state(
                            point,
                            checked,
                            selected_debug_session.is_some_and(|s| s.state.is_active()),
                            self.debugger_ui.muted,
                        ),
                        checked.and_then(|point| point.message.clone()),
                    ),
                )
            })
            .collect();
        let execution_line = self
            .service
            .debugger
            .sessions
            .iter()
            .find(|session| {
                Some(session.id) == self.debugger_ui.selected_session
                    && session.state == codisseu_core::debugger::DebugState::Stopped
            })
            .and_then(|session| {
                session
                    .frames
                    .iter()
                    .find(|frame| Some(frame.id) == session.current_frame)
            })
            .filter(|frame| frame.path.as_ref() == Some(&path))
            .filter(|_| {
                !self
                    .service
                    .view_document(view_id)
                    .is_some_and(|document| document.is_dirty())
            })
            .map(|frame| frame.line.saturating_sub(1) as usize);
        let mut layout_pending = false;
        let mut layout_error = None;
        let mut ghost_visible = false;
        let mut ghost_rects = Vec::new();
        let mut wrap_row_rects = Vec::new();
        let mut canonical_viewport = true;
        let mut documentation_target = None;
        let output = egui::ScrollArea::both()
            .id_salt(("codisseu.editor.scroll", view_id, id))
            .auto_shrink([false, false])
            .animated(false)
            .scroll_source(egui::scroll_area::ScrollSource {
                drag: egui::scroll_area::DragScroll::Never,
                ..Default::default()
            })
            .scroll_offset(view.scroll)
            .codisseu()
            .show_viewport(&mut child, |ui, viewport| {
                let origin = ui.min_rect().min;
                if !self.rename_editor_input_blocked()
                    && !self.project_search.open
                    && !self.service.code_actions.is_open()
                {
                    input::handle(
                        ui,
                        &mut self.service,
                        id,
                        view_id,
                        view,
                        &mut response,
                        code,
                        origin,
                        &extension,
                        &self.tokens,
                    );
                }
                let version = self.service.view_document(view_id).unwrap().version;
                snapshot = self.service.view_document(view_id).unwrap().snapshot();
                // Input may have removed the row beneath a stationary pointer.
                // Fold/wrap and shaped lines still describe the pre-input version
                // here; speculative hit-testing resumes on the next synced frame.
                if version != initial_version {
                    ui.ctx().request_repaint();
                }
                let link_enabled = is_active
                    && version == initial_version
                    && !self.rename_editor_input_blocked()
                    && !self.project_search.open;
                links::update(
                    ui,
                    &mut self.service,
                    view,
                    &response,
                    &snapshot,
                    id,
                    version,
                    code.intersect(ui.clip_rect()),
                    origin,
                    &extension,
                    &self.tokens,
                    link_enabled,
                );
                let link = links::resolved(&self.service, view, id, version);
                if is_active && link_enabled && !self.service.code_actions.is_open() {
                    documentation_target = links::documentation_target(
                        ui,
                        view,
                        &response,
                        &snapshot,
                        code.intersect(ui.clip_rect()),
                        origin,
                        &extension,
                        &self.tokens,
                    );
                }
                let doc = self.service.view_document(view_id).unwrap();
                selection = self.service.view(view_id).unwrap().selection();
                let inline_display = if is_active {
                    self.rename_inline_snapshot(id, doc.version, &snapshot)
                } else {
                    None
                };
                let doc = self.service.view_document(view_id).unwrap();
                let ghost_suggestion = if is_active
                    && view.composition.is_none()
                    && view.multi.extra.is_empty()
                    && inline_display.is_none()
                    && selection.is_caret()
                {
                    self.copilot_editor_suggestion(
                        ui.ctx(),
                        id,
                        view_id,
                        doc.version,
                        selection.head,
                    )
                } else {
                    None
                };
                // Compose against a cheap private Rope snapshot. The canonical text and
                // history are untouched; prefix/suffix and multiline replacement reflow.
                canonical_viewport = view.composition.is_none() && inline_display.is_none();
                let mut display_snapshot = snapshot.clone();
                let mut render_selection = selection;
                let mut render_revision = doc.last_change().map_or(0, |c| c.revision);
                let mut render_change = doc.last_change();
                let mut preview_cache = std::mem::take(&mut view.preview_cache);
                let cache = if let Some((display, revision, range)) = &inline_display {
                    display_snapshot = display.clone();
                    render_selection = Selection::caret(range.start);
                    render_revision = *revision;
                    render_change = None;
                    preview_cache.begin_frame();
                    &mut preview_cache
                } else if let Some(composition) = &view.composition {
                    let r = composition.selection.range();
                    display_snapshot.remove(r.clone());
                    display_snapshot.insert(r.start, &composition.text);
                    let at = composition
                        .active
                        .as_ref()
                        .map_or(composition.text.chars().count(), |r| r.end)
                        .min(composition.text.chars().count());
                    render_selection = Selection::caret(r.start + at);
                    render_revision = view.composition_revision;
                    render_change = None;
                    preview_cache.begin_frame();
                    &mut preview_cache
                } else {
                    &mut view.cache
                };
                cache.begin_document(
                    display_snapshot.clone(),
                    render_revision,
                    render_change,
                    &extension,
                );
                cache.sync(render_revision, render_change, ui, &self.tokens);
                if let Some(suggestion) = &ghost_suggestion {
                    let line = snapshot.char_to_line(selection.head);
                    let state = cache.lexical_state(line, &extension);
                    if !view
                        .ghost
                        .as_ref()
                        .is_some_and(|g| g.matches(suggestion, selection.head, ligatures, state))
                    {
                        let source = cache.get(ui, &snapshot, line, &extension, &self.tokens);
                        let column = selection.head - snapshot.line_to_char(line);
                        view.ghost = source
                            .x(ui, &snapshot, &self.tokens, column)
                            .zip(source.visual_column(ui, &snapshot, &self.tokens, column))
                            .map(|(x, visual)| {
                                let mut ghost = ghost::Ghost::build(
                                    ui,
                                    &snapshot,
                                    suggestion,
                                    selection.head,
                                    x,
                                    visual,
                                    &extension,
                                    &self.tokens,
                                    ligatures,
                                    state,
                                );
                                ghost.sync_source_layout(ui, &snapshot, &source, &self.tokens);
                                ghost
                            });
                    }
                } else {
                    view.ghost = None;
                }
                let unnecessary = decorations::unnecessary_ranges(
                    &self.service.language,
                    &path,
                    doc.version,
                    &snapshot,
                    view.composition.is_some() || inline_display.is_some(),
                );
                let count = display_snapshot.len_lines();
                view.fold.sync(doc.content_identity(), count);
                let wrap_revision = if view.composition.is_some() || inline_display.is_some() {
                    render_revision
                } else {
                    doc.content_identity()
                };
                view.wrap.sync(
                    soft_wrap,
                    code.width() - self.tokens.editor_wrap_right_padding,
                    wrap_revision,
                    &self.tokens,
                    ligatures,
                    view.ghost.as_ref(),
                    (
                        inlay_stamp.1,
                        inlay_stamp.2,
                        ui.ctx().pixels_per_point().to_bits(),
                        cache.geometry_epoch(),
                    ),
                );
                if soft_wrap {
                    view.wrap.prepare(
                        ui,
                        &display_snapshot,
                        cache,
                        &view.fold,
                        view.ghost.as_ref(),
                        (viewport.min.y / line_height).floor().max(0.) as usize
                            ..(viewport.max.y / line_height).ceil().max(0.) as usize + 2,
                        render_selection.head.min(display_snapshot.len_chars()),
                        &[
                            render_selection.anchor.min(display_snapshot.len_chars()),
                            view.wrap_anchor
                                .map_or(render_selection.head, |(_, point, _)| {
                                    point.min(display_snapshot.len_chars())
                                }),
                        ],
                        &extension,
                        &self.tokens,
                    );
                }
                let rows = if soft_wrap {
                    view.wrap.total(&view.fold)
                } else {
                    view.fold.visible_line_count()
                        + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
                };
                let first_row =
                    ((viewport.min.y / line_height).floor().max(0.) as usize).min(rows - 1);
                let last_row =
                    ((viewport.max.y / line_height).ceil().max(0.) as usize + 2).min(rows);
                let source_line = |row| {
                    if soft_wrap {
                        view.wrap.source_line(&view.fold, row)
                    } else {
                        view.ghost.as_ref().map_or_else(
                            || view.fold.source_line(row),
                            |g| g.source_line(&view.fold, row),
                        )
                    }
                };
                visible = source_line(first_row)
                    ..(source_line(last_row.saturating_sub(1)) + 1).min(count);
                let ghost_width = view.ghost.as_ref().map_or(0., |g| {
                    let source = cache.get(ui, &snapshot, g.line, &extension, &self.tokens);
                    g.width(source.width())
                });
                ui.set_min_size(egui::vec2(
                    if soft_wrap {
                        (code.width() - 12.).max(1.)
                    } else {
                        (cache.width().max(ghost_width) + 48.).max(code.width() - 12.)
                    },
                    (rows as f32 * line_height + line_height * 5.).max(code.height()),
                ));
                let caret_line = display_snapshot
                    .char_to_line(render_selection.head.min(display_snapshot.len_chars()));
                let active = self.service.active_view() == Some(view_id)
                    && (response.has_focus()
                        || (matches!(self.tool_tab, ToolTab::References | ToolTab::Rename)
                            && self.show_activity));
                // The original keeps each group's own caret row visible even
                // while another editor group owns keyboard focus.
                let caret_row_visible = active || self.service.document_views().len() > 1;
                let painter = ui.painter().with_clip_rect(code.intersect(ui.clip_rect()));
                let gutter_painter = egui::Painter::new(
                    ui.ctx().clone(),
                    ui.layer_id(),
                    egui::Rect::from_min_max(all.min, egui::pos2(code.left(), code.bottom()))
                        .intersect(group_clip),
                );
                let painted_selections = view.multi.all(render_selection);
                view.selection_highlights.begin(
                    doc.content_identity(),
                    &snapshot,
                    selection,
                    view.composition.is_some() || inline_display.is_some(),
                );
                let mut real_ranges: Vec<_> = painted_selections
                    .iter()
                    .map(|s| s.range())
                    .filter(|r| !r.is_empty())
                    .collect();
                real_ranges.sort_unstable_by_key(|r| r.start);
                let caret_visual_row = {
                    let layout =
                        cache.get(ui, &display_snapshot, caret_line, &extension, &self.tokens);
                    layout
                        .x(
                            ui,
                            &display_snapshot,
                            &self.tokens,
                            render_selection.head - display_snapshot.line_to_char(caret_line),
                        )
                        .and_then(|x| {
                            soft_wrap::projected_caret_position(
                                &view.wrap,
                                &view.fold,
                                view.ghost.as_ref(),
                                caret_line,
                                x,
                                view.wrap_previous_affinity,
                            )
                        })
                        .map(|(_, row)| row)
                };
                let render_rows: Vec<_> = if soft_wrap {
                    (first_row..last_row)
                        .filter_map(|row| {
                            let line = view.wrap.source_line(&view.fold, row);
                            let local = row - view.wrap.visual_row(&view.fold, line);
                            let segment = view.wrap.segment(
                                line,
                                local,
                                code.width() - self.tokens.editor_wrap_right_padding,
                            )?;
                            Some((line, row, local == 0, segment))
                        })
                        .collect()
                } else {
                    visible
                        .clone()
                        .filter(|line| !view.fold.is_hidden(*line))
                        .map(|line| {
                            let row = view.ghost.as_ref().map_or_else(
                                || view.fold.visual_row(line),
                                |g| g.visual_row(&view.fold, line),
                            );
                            (
                                line,
                                row,
                                true,
                                soft_wrap::Segment {
                                    projection: 0,
                                    x: 0. ..f32::INFINITY,
                                    touched: 0,
                                },
                            )
                        })
                        .collect()
                };
                for (line, visual_row, first_segment, segment) in render_rows {
                    let l = cache.get(ui, &display_snapshot, line, &extension, &self.tokens);
                    let row_y = origin.y + visual_row as f32 * line_height;
                    let y = row_y - segment.projection as f32 * line_height;
                    let origin = origin - egui::vec2(segment.x.start, 0.);
                    let viewport = egui::Rect::from_min_max(
                        viewport.min + egui::vec2(segment.x.start, 0.),
                        viewport.max + egui::vec2(segment.x.start, 0.),
                    );
                    let row_painter = if soft_wrap {
                        painter.with_clip_rect(painter.clip_rect().intersect(
                            egui::Rect::from_min_size(
                                egui::pos2(code.left(), row_y),
                                egui::vec2(code.width(), line_height),
                            ),
                        ))
                    } else {
                        painter.clone()
                    };
                    let painter = if soft_wrap {
                        row_painter.with_clip_rect(row_painter.clip_rect().intersect(
                            egui::Rect::from_min_size(
                                egui::pos2(code.left(), row_y),
                                egui::vec2(
                                    (segment.x.end - segment.x.start).max(1.).min(code.width()),
                                    line_height,
                                ),
                            ),
                        ))
                    } else {
                        row_painter.clone()
                    };
                    let gutter_painter = if first_segment {
                        gutter_painter.clone()
                    } else {
                        gutter_painter.with_clip_rect(egui::Rect::NOTHING)
                    };
                    if soft_wrap && wrap_row_rects.len() < 1024 {
                        let map_x = |x| {
                            view.ghost
                                .as_ref()
                                .filter(|g| g.line == line)
                                .map_or(Some(x), |g| {
                                    g.pointer(
                                        &view.fold,
                                        view.fold.visual_row(line) + segment.projection,
                                        x,
                                    )
                                })
                        };
                        let base = display_snapshot.line_to_char(line);
                        let a = map_x(segment.x.start)
                            .and_then(|x| l.column(ui, &display_snapshot, &self.tokens, x))
                            .unwrap_or_else(|| view.ghost.as_ref().map_or(0, |g| g.caret - base));
                        let b = map_x(segment.x.end)
                            .and_then(|x| l.column(ui, &display_snapshot, &self.tokens, x))
                            .unwrap_or(a);
                        wrap_row_rects.push(WrapRowMetrics {
                            source_line: line,
                            start_char: base + a,
                            end_char: base + b,
                            rect: [code.left(), row_y, code.right() - 8., row_y + line_height],
                        });
                    }
                    let start = display_snapshot.line_to_char(line);
                    let end = content_end(&display_snapshot, line);
                    if line == caret_line
                        && caret_row_visible
                        && (!soft_wrap || caret_visual_row == Some(visual_row))
                    {
                        row_painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(code.left(), row_y),
                                egui::vec2(code.width(), line_height),
                            ),
                            0.,
                            color(self.tokens.caret_row),
                        );
                        gutter_painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(all.left(), row_y),
                                egui::vec2(code.left() - all.left(), line_height),
                            ),
                            0.,
                            color(self.tokens.caret_row),
                        );
                    }
                    if execution_line == Some(line) {
                        row_painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(code.left(), row_y),
                                egui::vec2(code.width(), line_height),
                            ),
                            0.,
                            color(self.tokens.selection).gamma_multiply(0.55),
                        );
                    }
                    if debug_source && first_segment {
                        let center = egui::pos2(
                            code.left() - self.tokens.editor_breakpoint_center_offset,
                            y + line_height / 2.,
                        );
                        let point = breakpoints.get(&line);
                        let label = format!(
                            "{} breakpoint: {}:{}",
                            if point.is_some() { "Remove" } else { "Add" },
                            path.display(),
                            line + 1
                        );
                        let tooltip = point
                            .and_then(|(_, message)| message.clone())
                            .map(|message| format!("{}: {message}", point.unwrap().0.label()))
                            .unwrap_or_else(|| {
                                point
                                    .map_or("Toggle line breakpoint · Ctrl+F8", |(state, _)| {
                                        state.label()
                                    })
                                    .into()
                            });
                        breakpoint_targets.push((
                            line,
                            egui::Rect::from_center_size(center, egui::vec2(18., line_height))
                                .intersect(group_clip),
                            label,
                            tooltip,
                        ));
                        if let Some((state, _)) = point {
                            state.paint(&gutter_painter, center, &self.tokens);
                        }
                        if execution_line == Some(line) {
                            gutter_painter.add(egui::Shape::convex_polygon(
                                vec![
                                    center + egui::vec2(-4., -5.),
                                    center + egui::vec2(5., 0.),
                                    center + egui::vec2(-4., 5.),
                                ],
                                color(self.tokens.accent),
                                egui::Stroke::NONE,
                            ));
                        }
                    }
                    if first_segment
                        && let Some(range) =
                            view.fold.ranges().iter().find(|range| range.start == line)
                    {
                        let collapsed = view.fold.collapsed_at(line);
                        let button_rect = egui::Rect::from_center_size(
                            egui::pos2(
                                code.left() - self.tokens.editor_fold_center_offset,
                                y + line_height / 2.,
                            ),
                            egui::vec2(13., line_height),
                        );
                        fold_targets.push((
                            line,
                            range.end,
                            button_rect.intersect(group_clip),
                            collapsed,
                        ));
                        // ScrollArea's painter/input are clipped to the code column.
                        // Paint and interact in the gutter's coordinate space instead.
                        gutter_painter.text(
                            button_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            if collapsed { "\u{eab6}" } else { "\u{eab4}" },
                            FontId::new(16., egui::FontFamily::Name("codisseu.codicons".into())),
                            color(self.tokens.muted),
                        );
                        if collapsed
                            && let Some(right) =
                                l.x(ui, &display_snapshot, &self.tokens, end - start)
                        {
                            let (right, ellipsis_y) = view
                                .ghost
                                .as_ref()
                                .filter(|g| g.line == line)
                                .map_or((right, y), |g| {
                                    (g.source_x(right), y + g.added_rows() as f32 * line_height)
                                });
                            painter.text(
                                egui::pos2(origin.x + right + 4., ellipsis_y + line_height / 2.),
                                egui::Align2::LEFT_CENTER,
                                "…",
                                FontId::monospace(self.tokens.editor_font_size),
                                color(self.tokens.muted),
                            );
                        }
                    }
                    // Source decorations/text before the insertion stay in place;
                    // the suffix is painted after the virtual rows, with its source
                    // syntax and inlays retained by the original line layout.
                    let full_line_painter = painter.clone();
                    let painter = if let Some(g) = view.ghost.as_ref().filter(|g| g.line == line) {
                        painter.with_clip_rect(painter.clip_rect().intersect(
                            egui::Rect::from_min_max(
                                painter.clip_rect().min,
                                egui::pos2(origin.x + g.split_x, painter.clip_rect().bottom()),
                            ),
                        ))
                    } else {
                        painter.clone()
                    };
                    let query_len = view.selection_highlights.query_len();
                    if query_len > 0
                        && let Some(left) =
                            l.column(ui, &display_snapshot, &self.tokens, viewport.min.x)
                    {
                        let right = l
                            .column(ui, &display_snapshot, &self.tokens, viewport.max.x)
                            .unwrap_or(l.scanned_chars());
                        let from = start + left.saturating_sub(query_len);
                        let to = (start + right.saturating_add(query_len))
                            .min(start + l.scanned_chars())
                            .min(end);
                        for range in view.selection_highlights.ranges(&snapshot, from..to) {
                            let index = real_ranges.partition_point(|r| r.end <= range.start);
                            if real_ranges.get(index).is_some_and(|r| r.start < range.end) {
                                continue;
                            }
                            if selection_highlights >= selection_highlight::FRAME_MARKS {
                                view.selection_highlights.limited = true;
                                break;
                            }
                            if let (Some(a), Some(b)) = (
                                l.x(ui, &display_snapshot, &self.tokens, range.start - start),
                                l.x(ui, &display_snapshot, &self.tokens, range.end - start),
                            ) {
                                let rect = egui::Rect::from_min_max(
                                    egui::pos2(origin.x + a, y),
                                    egui::pos2(origin.x + b, y + line_height),
                                );
                                if rect.intersect(painter.clip_rect()).is_positive() {
                                    let fill = self.tokens.editor_selection_highlight;
                                    let border = self.tokens.editor_selection_highlight_border;
                                    painter.rect(
                                        rect,
                                        1.,
                                        Color32::from_rgba_unmultiplied(
                                            fill[0], fill[1], fill[2], fill[3],
                                        ),
                                        egui::Stroke::new(
                                            1. / ui.ctx().pixels_per_point(),
                                            Color32::from_rgba_unmultiplied(
                                                border[0], border[1], border[2], border[3],
                                            ),
                                        ),
                                        egui::StrokeKind::Inside,
                                    );
                                    selection_highlights += 1;
                                }
                            }
                        }
                    }
                    for &painted_selection in &painted_selections {
                        let range = painted_selection.range();
                        if range.start < start + display_snapshot.line(line).len_chars()
                            && range.end > start
                        {
                            let a = l.x(
                                ui,
                                &display_snapshot,
                                &self.tokens,
                                range.start.saturating_sub(start),
                            );
                            let b = if range.end > end {
                                Some(code.right() - origin.x)
                            } else {
                                l.x(
                                    ui,
                                    &display_snapshot,
                                    &self.tokens,
                                    range.end.saturating_sub(start).min(l.scanned_chars()),
                                )
                            };
                            if let (Some(a), Some(b)) = (a, b) {
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(origin.x + a, y),
                                        egui::pos2(origin.x + b, y + line_height),
                                    ),
                                    0.,
                                    color(self.tokens.editor_selection),
                                );
                            }
                        }
                        if active
                            && painted_selection != render_selection
                            && painted_selection.head >= start
                            && painted_selection.head <= end
                            && let Some(x) = l.x(
                                ui,
                                &display_snapshot,
                                &self.tokens,
                                painted_selection.head - start,
                            )
                            && (!soft_wrap
                                || soft_wrap::projected_caret_position(
                                    &view.wrap,
                                    &view.fold,
                                    view.ghost.as_ref(),
                                    line,
                                    x,
                                    false,
                                )
                                .is_some_and(|(_, row)| row == visual_row))
                        {
                            painter.rect_filled(
                                egui::Rect::from_min_size(
                                    egui::pos2(
                                        origin.x + x,
                                        y + (line_height - self.tokens.editor_caret_height) / 2.,
                                    ),
                                    egui::vec2(
                                        self.tokens.editor_caret_width,
                                        self.tokens.editor_caret_height,
                                    ),
                                ),
                                0.,
                                color(self.tokens.editor_caret),
                            );
                        }
                    }
                    if active
                        && view.composition.is_none()
                        && let Some(pair) = view.matching
                    {
                        for at in pair {
                            if at >= start
                                && at < end
                                && let (Some(a), Some(b)) = (
                                    l.x(ui, &display_snapshot, &self.tokens, at - start),
                                    l.x(ui, &display_snapshot, &self.tokens, at - start + 1),
                                )
                            {
                                painter.rect_filled(
                                    egui::Rect::from_min_max(
                                        egui::pos2(origin.x + a, y),
                                        egui::pos2(origin.x + b, y + line_height),
                                    ),
                                    0.,
                                    color(self.tokens.editor_matching_bracket),
                                );
                            }
                        }
                    }
                    if let Some((_, _, range)) = &inline_display
                        && range.start >= start
                        && range.start <= end
                        && let (Some(a), Some(b)) = (
                            l.x(ui, &display_snapshot, &self.tokens, range.start - start),
                            l.x(ui, &display_snapshot, &self.tokens, range.end - start),
                        )
                        && (!soft_wrap
                            || soft_wrap::projected_caret_position(
                                &view.wrap,
                                &view.fold,
                                view.ghost.as_ref(),
                                line,
                                a,
                                false,
                            )
                            .is_some_and(|(_, row)| row == visual_row))
                    {
                        rename_rect = Some(egui::Rect::from_min_max(
                            egui::pos2(origin.x + a, y),
                            egui::pos2(origin.x + b, y + line_height),
                        ));
                    }
                    let line_unnecessary = decorations::line_ranges(&unnecessary, start, end);
                    unnecessary_spans += line_unnecessary.len();
                    l.paint(
                        ui,
                        &display_snapshot,
                        layout::LineInk {
                            tokens: &self.tokens,
                            unnecessary: &line_unnecessary,
                        },
                        &painter,
                        egui::pos2(origin.x, y),
                        viewport.min.x..viewport.max.x,
                    );
                    if let Some(range) = &link
                        && range.start >= start
                        && range.end <= end
                        && let (Some(a), Some(b)) = (
                            l.x(ui, &display_snapshot, &self.tokens, range.start - start),
                            l.x(ui, &display_snapshot, &self.tokens, range.end - start),
                        )
                    {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        painter.line_segment(
                            [
                                egui::pos2(origin.x + a, y + line_height - 2.),
                                egui::pos2(origin.x + b, y + line_height - 2.),
                            ],
                            egui::Stroke::new(1., color(self.tokens.accent)),
                        );
                    }
                    if is_active && view.composition.is_none() && inline_display.is_none() {
                        if first_segment
                            && blame_width > 0.
                            && let Some(blame) =
                                self.git_insights.line(&path, doc.version, line + 1)
                        {
                            let rect = egui::Rect::from_min_max(
                                egui::pos2(code.left() - blame_width, y),
                                egui::pos2(code.left() - 8., y + line_height),
                            )
                            .intersect(group_clip);
                            let tint = if blame.oid
                                == self
                                    .git_insights
                                    .line(&path, doc.version, caret_line + 1)
                                    .map(|l| l.oid.as_str())
                                    .unwrap_or("")
                            {
                                self.tokens.git_log_selection
                            } else {
                                self.tokens.panel
                            };
                            gutter_painter.rect_filled(rect, 0., color(tint));
                            let age = git_insights::heat(blame.timestamp, blame.uncommitted);
                            gutter_painter.rect_filled(
                                egui::Rect::from_min_max(
                                    rect.right_top() - egui::vec2(2., 0.),
                                    rect.right_bottom(),
                                ),
                                0.,
                                age,
                            );
                            let first = self
                                .git_insights
                                .line(&path, doc.version, line)
                                .is_none_or(|previous| previous.oid != blame.oid)
                                || line == visible.start;
                            if first {
                                let subject = if blame.uncommitted {
                                    "Alterações não confirmadas"
                                } else {
                                    &blame.summary
                                };
                                let label = format!(
                                    "{}  {}",
                                    &blame.oid[..blame.oid.len().min(7)],
                                    git_insights::truncate(subject, 80)
                                );
                                let font = FontId::monospace(self.tokens.editor_font_size - 1.);
                                let text_clip = egui::Rect::from_min_max(
                                    rect.min + egui::vec2(8., 0.),
                                    rect.max - egui::vec2(90., 0.),
                                );
                                gutter_painter.with_clip_rect(text_clip).text(
                                    text_clip.left_center(),
                                    egui::Align2::LEFT_CENTER,
                                    label,
                                    font.clone(),
                                    color(self.tokens.text),
                                );
                                gutter_painter.text(
                                    rect.right_center() - egui::vec2(8., 0.),
                                    egui::Align2::RIGHT_CENTER,
                                    if blame.uncommitted {
                                        "agora".into()
                                    } else {
                                        git_insights::relative_time(blame.timestamp)
                                    },
                                    FontId::proportional(11.),
                                    color(self.tokens.muted),
                                );
                            }
                            if rect.is_positive() {
                                authorship_targets.push((line + 1, rect));
                            }
                        }
                        if line == caret_line
                            && self.service.editor_preferences().git_current_line
                            && let Some(blame) =
                                self.git_insights.line(&path, doc.version, line + 1)
                            && let Some(x) = l.x(ui, &display_snapshot, &self.tokens, end - start)
                        {
                            let endpoint = soft_wrap::projected_position(
                                &view.wrap,
                                &view.fold,
                                view.ghost.as_ref(),
                                line,
                                x,
                                false,
                            );
                            let (local_x, end_row) = endpoint.unwrap_or((x, visual_row));
                            let source_end_x = origin.x + segment.x.start + local_x;
                            let source_end_y = origin.y + end_row as f32 * line_height;
                            if !soft_wrap || end_row == visual_row {
                                let at = egui::pos2(
                                    source_end_x + self.tokens.git_authorship_gap,
                                    source_end_y + line_height / 2.,
                                );
                                let galley = ui.fonts_mut(|fonts| {
                                    fonts.layout_no_wrap(
                                        git_insights::annotation(blame),
                                        FontId::monospace(self.tokens.editor_font_size - 1.),
                                        Color32::from_rgba_unmultiplied(
                                            self.tokens.git_authorship_color[0],
                                            self.tokens.git_authorship_color[1],
                                            self.tokens.git_authorship_color[2],
                                            self.tokens.git_authorship_color[3],
                                        ),
                                    )
                                });
                                let rect = egui::Rect::from_min_size(
                                    egui::pos2(at.x, source_end_y),
                                    egui::vec2(galley.size().x, line_height),
                                )
                                .intersect(code)
                                .intersect(group_clip)
                                .intersect(row_painter.clip_rect());
                                row_painter.galley(
                                    at - egui::vec2(0., galley.size().y / 2.),
                                    galley,
                                    Color32::PLACEHOLDER,
                                );
                                if rect.is_positive() {
                                    authorship_targets.push((line + 1, rect));
                                }
                            }
                        }
                    }
                    if let Some(composition) = &view.composition {
                        let r = composition.selection.range();
                        let len = composition.text.chars().count();
                        if r.start >= start
                            && r.start <= end
                            && let (Some(a), Some(b)) = (
                                l.x(ui, &display_snapshot, &self.tokens, r.start - start),
                                l.x(ui, &display_snapshot, &self.tokens, r.start - start + len),
                            )
                        {
                            painter.line_segment(
                                [
                                    egui::pos2(origin.x + a, y + line_height - 1.),
                                    egui::pos2(origin.x + b, y + line_height - 1.),
                                ],
                                egui::Stroke::new(1., color(self.tokens.editor_caret)),
                            );
                        }
                    }
                    if let Some(g) = view.ghost.as_ref().filter(|g| g.line == line) {
                        if active && let Some(pair) = view.matching {
                            for at in pair.into_iter().filter(|at| *at >= g.caret && *at < end) {
                                if let (Some(a), Some(b)) = (
                                    l.x(ui, &snapshot, &self.tokens, at - start),
                                    l.x(ui, &snapshot, &self.tokens, at - start + 1),
                                ) {
                                    for (span, row) in g.project_range(a, b) {
                                        let mapped_y = y + row as f32 * line_height;
                                        full_line_painter.rect_filled(
                                            egui::Rect::from_min_max(
                                                egui::pos2(origin.x + span.start, mapped_y),
                                                egui::pos2(
                                                    origin.x + span.end,
                                                    mapped_y + line_height,
                                                ),
                                            ),
                                            0.,
                                            color(self.tokens.editor_matching_bracket),
                                        );
                                    }
                                }
                            }
                        }
                        ghost_visible |= g.paint(
                            ui,
                            &snapshot,
                            &l,
                            &full_line_painter,
                            egui::pos2(origin.x, y),
                            &self.tokens,
                            &line_unnecessary,
                            viewport.min.x..viewport.max.x,
                            &mut ghost_rects,
                        );
                        if let Some(range) = &link
                            && range.end > g.caret
                            && range.end <= end
                            && let (Some(a), Some(b)) = (
                                l.x(
                                    ui,
                                    &snapshot,
                                    &self.tokens,
                                    range.start.max(g.caret) - start,
                                ),
                                l.x(ui, &snapshot, &self.tokens, range.end - start),
                            )
                        {
                            for (span, row) in g.project_range(a, b) {
                                let mapped_y = y + row as f32 * line_height;
                                full_line_painter.line_segment(
                                    [
                                        egui::pos2(
                                            origin.x + span.start,
                                            mapped_y + line_height - 2.,
                                        ),
                                        egui::pos2(
                                            origin.x + span.end,
                                            mapped_y + line_height - 2.,
                                        ),
                                    ],
                                    egui::Stroke::new(1., color(self.tokens.accent)),
                                );
                            }
                        }
                    }
                    layout_pending |= !l.is_complete();
                    layout_error = layout_error.take().or_else(|| l.pending_error());
                    if first_segment {
                        git_changes::paint(
                            &gutter_painter,
                            self.service.git_line_changes(doc.id, doc.version),
                            line,
                            egui::pos2(
                                all.left()
                                    + self.tokens.editor_line_number_offset
                                    + (count.to_string().len().saturating_sub(2) as f32) * 7.
                                    + self.tokens.editor_git_number_gap,
                                y,
                            ),
                            line_height,
                            &self.tokens,
                        );
                        gutter_painter.text(
                            egui::pos2(
                                all.left()
                                    + self.tokens.editor_line_number_offset
                                    + (count.to_string().len().saturating_sub(2) as f32) * 7.,
                                y + self.tokens.editor_line_number_y,
                            ),
                            egui::Align2::RIGHT_TOP,
                            (line + 1).to_string(),
                            FontId::monospace(self.tokens.editor_line_number_font_size),
                            color(if line == caret_line && caret_row_visible {
                                self.tokens.caret_line_number
                            } else {
                                [75, 80, 89]
                            }),
                        );
                    }
                }
                gutter_painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(code.left() - 3., all.top()),
                        egui::pos2(code.left() - 2., all.bottom()),
                    ),
                    0.,
                    color(self.tokens.editor_gutter_separator),
                );
                let l = cache.get(ui, &display_snapshot, caret_line, &extension, &self.tokens);
                let caret_x = l.x(
                    ui,
                    &display_snapshot,
                    &self.tokens,
                    render_selection.head - display_snapshot.line_to_char(caret_line),
                );
                let projected_caret = caret_x.and_then(|x| {
                    soft_wrap::projected_caret_position(
                        &view.wrap,
                        &view.fold,
                        view.ghost.as_ref(),
                        caret_line,
                        x,
                        view.wrap_previous_affinity,
                    )
                });
                let caret_x = projected_caret.map(|(x, _)| x);
                let scale = ui.ctx().pixels_per_point();
                let x = ((origin.x + caret_x.unwrap_or(0.)) * scale).round() / scale;
                layout_pending |= !l.is_complete();
                layout_error = layout_error.take().or_else(|| l.pending_error());
                let y = origin.y + projected_caret.map_or(0, |(_, row)| row) as f32 * line_height;
                layout_pending |= soft_wrap && (view.wrap.pending || projected_caret.is_none());
                // Center the insertion bar on the measured boundary, while
                // retaining its full width at the left edge of the code area.
                // Do not clamp an offscreen caret back into the viewport.
                let caret_left = x - self.tokens.editor_caret_width / 2.;
                let caret_left = if x >= code.left() - 1. / ui.ctx().pixels_per_point() {
                    caret_left.max(code.left())
                } else {
                    caret_left
                };
                cursor_rect = egui::Rect::from_min_size(
                    egui::pos2(
                        caret_left,
                        y + (line_height - self.tokens.editor_caret_height) / 2.,
                    ),
                    egui::vec2(
                        self.tokens.editor_caret_width,
                        self.tokens.editor_caret_height,
                    ),
                );
                if response.has_focus()
                    && self.service.active_view() == Some(view_id)
                    && caret_x.is_some()
                {
                    let visuals = &ui.visuals().text_cursor;
                    let now = ui.input(|i| i.time);
                    let cycle = (visuals.on_duration + visuals.off_duration) as f64;
                    if !visuals.blink
                        || (now - view.last_interaction).max(0.) % cycle.max(0.01)
                            < visuals.on_duration as f64
                    {
                        let mut mesh = egui::Mesh::default();
                        mesh.add_colored_rect(cursor_rect, color(self.tokens.editor_caret));
                        painter.add(egui::Shape::mesh(mesh));
                    }
                    if visuals.blink {
                        ui.ctx().request_repaint_after(Duration::from_millis(250));
                    }
                    ui.ctx().output_mut(|o| {
                        o.ime = Some(egui::output::IMEOutput {
                            purpose: egui::IMEPurpose::Normal,
                            rect: code,
                            cursor_rect,
                            should_interrupt_composition: false,
                        })
                    });
                }
                if ghost_visible && let Some(suggestion) = &ghost_suggestion {
                    self.copilot_did_show(suggestion);
                }
                if view.composition.is_some() && caret_x.is_some() {
                    ui.scroll_to_rect(cursor_rect, None);
                }
                preview_cache.finish_frame();
                view.preview_cache = if view.composition.is_some() || inline_display.is_some() {
                    preview_cache
                } else {
                    layout::LineCache::default()
                };
                accessibility::update(
                    ui,
                    editor_id(view_id),
                    &snapshot,
                    selection,
                    view,
                    visible.clone(),
                    origin,
                    &extension,
                    &self.tokens,
                    &path,
                );
                view.stamp = Some((doc.version, selection));
            });
        // The ScrollArea clips input to the code region. Gutter hit targets must
        // belong to this parent UI, just as their painting uses the group clip.
        let mut gutter_action = None;
        let mut last_line = None;
        let navigation_current = self
            .service
            .view_document(view_id)
            .is_some_and(|d| d.version == initial_version);
        for anchor in navigation_anchors.iter().filter(|a| {
            navigation_current && visible.contains(&a.line) && !view.fold.is_hidden(a.line)
        }) {
            if last_line == Some(anchor.line) {
                continue;
            }
            last_line = Some(anchor.line);
            let y = code.top() - view.scroll.y
                + ghost::visual_row(view, anchor.line) as f32 * line_height;
            // A single compact lane leaves independent targets for breakpoint and folding.
            let rect = egui::Rect::from_center_size(
                egui::pos2(code.left() - 47., y + line_height / 2.),
                egui::vec2(14., line_height),
            )
            .intersect(group_clip);
            if !rect.is_positive() {
                continue;
            }
            let has_super = anchor.has_parent_relations;
            let (label, icon, action) = if has_super {
                (format!("Ir ao super de {}", anchor.name), Icon::Up, 5)
            } else if anchor.can_find_implementations {
                (
                    format!("Buscar implementações de {}", anchor.name),
                    Icon::Down,
                    1,
                )
            } else {
                (
                    format!("Localizar usos de {}", anchor.name),
                    Icon::GitReveal,
                    0,
                )
            };
            let hit = ui.interact(
                rect,
                egui::Id::unique((
                    "php.gutter.navigation",
                    view_id,
                    id,
                    anchor.scalar_range.start,
                )),
                egui::Sense::click(),
            );
            hit.widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), &label)
            });
            crate::components::paint_icon_monochrome(ui, rect, icon, color(self.tokens.muted));
            if hit.clicked() {
                gutter_action = Some((anchor.scalar_range.start, action));
            }
            hit.context_menu(|ui| {
                if ui
                    .button(format!("Localizar usos de {}", anchor.name))
                    .clicked()
                {
                    gutter_action = Some((anchor.scalar_range.start, 0));
                    ui.close();
                }
                if has_super
                    && ui
                        .button(format!("Ir ao super de {}", anchor.name))
                        .clicked()
                {
                    gutter_action = Some((anchor.scalar_range.start, 5));
                    ui.close();
                }
                if anchor.can_find_implementations
                    && ui
                        .button(format!("Buscar implementações de {}", anchor.name))
                        .clicked()
                {
                    gutter_action = Some((anchor.scalar_range.start, 1));
                    ui.close();
                }
                for parent in &anchor.parent_types {
                    if ui
                        .button(format!("Ir ao contrato/supertipo {}", parent.name))
                        .clicked()
                    {
                        gutter_action = Some((parent.scalar_range.start, 2));
                        ui.close();
                    }
                }
            });
            hit.on_hover_text(format!("{label}. Botão direito: usos, implementações e contratos. Destinos confirmados pelo suporte PHP."))
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        for (line, end, rect, collapsed) in fold_targets {
            let hit = ui.interact(
                rect,
                egui::Id::unique(("codisseu.fold", view_id, id, line)),
                egui::Sense::click(),
            );
            hit.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    format!(
                        "{} lines {}–{}",
                        if collapsed { "Unfold" } else { "Fold" },
                        line + 1,
                        end + 1
                    ),
                )
            });
            if hit.clicked() {
                fold_toggle = Some(line);
            }
            hit.on_hover_text(if collapsed {
                "Expandir bloco"
            } else {
                "Recolher bloco"
            })
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        for (line, rect, label, tooltip) in breakpoint_targets {
            let hit = ui.interact(
                rect,
                egui::Id::unique(("debug.breakpoint", view_id, id, line)),
                egui::Sense::click(),
            );
            hit.widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label)
            });
            if hit.clicked() {
                if ui.input(|i| i.modifiers.alt) {
                    if let Some(mut point) = self
                        .service
                        .debugger
                        .breakpoints
                        .iter()
                        .find(|b| b.path == path && b.line == line as u32 + 1)
                        .cloned()
                    {
                        point.enabled = !point.enabled;
                        self.send(Command::Debug(
                            codisseu_core::debugger::DebugCommand::SetBreakpoint(point),
                        ));
                    }
                } else if ui.input(|i| i.modifiers.shift) {
                    self.edit_breakpoint(path.clone(), line as u32 + 1, true);
                } else {
                    breakpoint_toggle = Some(line as u32 + 1);
                }
            }
            hit.context_menu(|ui| {
                let point = self
                    .service
                    .debugger
                    .breakpoints
                    .iter()
                    .find(|b| b.path == path && b.line == line as u32 + 1)
                    .cloned();
                if let Some(mut point) = point {
                    if ui.button("Edit Breakpoint…").clicked() {
                        self.edit_breakpoint(path.clone(), point.line, false);
                        ui.close();
                    }
                    if ui
                        .button(if point.enabled {
                            "Disable Breakpoint"
                        } else {
                            "Enable Breakpoint"
                        })
                        .clicked()
                    {
                        point.enabled = !point.enabled;
                        self.send(Command::Debug(
                            codisseu_core::debugger::DebugCommand::SetBreakpoint(point),
                        ));
                        ui.close();
                    }
                    if ui.button("Remove Breakpoint").clicked() {
                        breakpoint_toggle = Some(line as u32 + 1);
                        ui.close();
                    }
                } else {
                    if ui.button("Add Breakpoint").clicked() {
                        breakpoint_toggle = Some(line as u32 + 1);
                        ui.close();
                    }
                    if ui.button("Add Conditional Breakpoint…").clicked() {
                        self.edit_breakpoint(path.clone(), line as u32 + 1, false);
                        ui.close();
                    }
                    if ui.button("Add Logging Breakpoint…").clicked() {
                        self.edit_breakpoint(path.clone(), line as u32 + 1, true);
                        ui.close();
                    }
                }
            });
            hit.on_hover_text(tooltip)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        for (line, rect) in authorship_targets {
            self.git_insights_hover(
                ui,
                rect,
                &path,
                self.service.view_document(view_id).unwrap().version,
                line,
            );
        }
        if let Some(line) = fold_toggle {
            if self.service.active_view() != Some(view_id) {
                self.send(Command::ActivateView { view: view_id });
            }
            response.request_focus();
            view.fold.toggle_at(line);
            if view
                .fold
                .is_hidden(snapshot.char_to_line(selection.head.min(snapshot.len_chars())))
            {
                let target = Selection::caret(snapshot.line_to_char(line));
                input::select(&mut self.service, id, target);
                selection = target;
            }
            view.reveal = false;
            ui.ctx().request_repaint();
        }
        if let Some(line) = breakpoint_toggle {
            self.send(Command::Debug(
                codisseu_core::debugger::DebugCommand::ToggleBreakpoint {
                    path: path.clone(),
                    line,
                },
            ));
            self.focus_editor = true;
        }
        if let Some(rect) = rename_rect {
            self.rename_inline_geometry(id, initial_version, rect, code);
        }
        if ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|point| code.contains(point))
                && (input.pointer.any_down() || input.is_scrolling())
        }) {
            view.wrap_anchor = None;
        }
        view.scroll = output.state.clamped_offset();
        if soft_wrap {
            view.scroll.x = 0.;
        }
        let scroll = [view.scroll.x, view.scroll.y];
        if self
            .service
            .view(view_id)
            .is_some_and(|v| v.scroll() != scroll)
        {
            self.send(Command::SetViewScroll {
                view: view_id,
                scroll,
            });
        }
        if canonical_viewport {
            let row = (view.scroll.y / line_height).floor().max(0.) as usize;
            let source =
                soft_wrap::pointer_position(view, row, if soft_wrap { 0. } else { view.scroll.x });
            let character = if let Some((_, character, _)) = view.wrap_anchor {
                Some(character)
            } else if let Some((line, x)) = source {
                let layout = view
                    .cache
                    .get(ui, &snapshot, line, &extension, &self.tokens);
                layout
                    .column(ui, &snapshot, &self.tokens, x)
                    .map(|column| snapshot.line_to_char(line) + column)
            } else {
                view.ghost.as_ref().map(|ghost| ghost.caret)
            };
            if let Some(character) = character {
                let anchor = codisseu_core::SourceScrollAnchor {
                    character: character.min(snapshot.len_chars()),
                    offset: view
                        .wrap_anchor
                        .map_or(view.scroll.y % line_height, |(_, _, offset)| offset),
                };
                if self
                    .service
                    .view(view_id)
                    .is_some_and(|v| v.source_scroll_anchor() != Some(anchor))
                {
                    self.send(Command::SetViewScrollAnchor {
                        view: view_id,
                        document: id,
                        anchor: Some(anchor),
                    });
                }
            }
        }
        // ScrollArea initially clamps against the previous frame's bounds after a
        // resize/document change. Retry the requested reveal with the new bounds.
        // Physical pixel rounding at fractional scale must not trigger an endless reveal.
        if reveal_requested
            && !code
                .expand(1. / ui.ctx().pixels_per_point())
                .contains_rect(cursor_rect)
        {
            view.reveal = true;
            ui.ctx().request_repaint();
        }
        if response.lost_focus() {
            view.chord = None;
            if view.composition.take().is_some() {
                view.ime_cancelled = true;
            }
            let _ = self.service.dispatch(Command::EndEditGroup { id });
        }
        view.had_focus = response.has_focus();
        if self.service.active_view() == Some(view_id) {
            self.caret = Some((id, selection.head));
            self.selection = Some((id, selection.range()));
            self.caret_screen = Some(cursor_rect.left_bottom());
            if let Some(popup) = self.signature_popup.as_mut() {
                popup.anchor = cursor_rect.left_bottom();
            }
            let line = snapshot.char_to_line(selection.head);
            let caret_layout = view
                .cache
                .get(ui, &snapshot, line, &extension, &self.tokens);
            let visual_column = caret_layout
                .visual_column(
                    ui,
                    &snapshot,
                    &self.tokens,
                    selection.head - snapshot.line_to_char(line),
                )
                .map(|c| c + 1);
            self.cursor_column_pending = visual_column.is_none();
            self.cursor = (
                line + 1,
                visual_column.unwrap_or(selection.head - snapshot.line_to_char(line) + 1),
            );
        }
        if self.service.active_view() == Some(view_id)
            && let Some((owner, v, index)) = self.service.completion.anchor()
            && owner == id
            && v == self.service.view_document(view_id).unwrap().version
            && let Some(popup) = self.completion_popup.as_mut()
        {
            let line = snapshot.char_to_line(index);
            let l = view
                .cache
                .get(ui, &snapshot, line, &extension, &self.tokens);
            let source_x = l
                .x(
                    ui,
                    &snapshot,
                    &self.tokens,
                    index - snapshot.line_to_char(line),
                )
                .unwrap_or(0.);
            let (x, row) = soft_wrap::caret_position(view, line, source_x, false)
                .unwrap_or((source_x, ghost::visual_row(view, line)));
            popup.anchor = egui::pos2(
                code.left() - view.scroll.x + x,
                code.top() - view.scroll.y + (row + 1) as f32 * line_height,
            );
        }
        if view.cache.lexical_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        if let Some(error) = view.cache.lexical_error() {
            layout_error = Some(error.to_owned());
        }
        view.cache.finish_frame();
        let metrics = EditorMetrics {
            document: Some(id),
            soft_wrap,
            visual_rows: if soft_wrap {
                view.wrap.total(&view.fold)
            } else {
                view.fold.visible_line_count()
                    + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
            },
            wrapped_lines: if soft_wrap {
                view.wrap.wrapped_lines()
            } else {
                0
            },
            wrap_pending: soft_wrap && view.wrap.pending,
            wrap_row_rects,
            wrap_index_bytes: view.wrap.bytes(),
            lines: snapshot.len_lines(),
            visible_start: visible.start,
            visible_end: visible.end,
            layouts_built: view.cache.built + view.preview_cache.built,
            cache_hits: view.cache.hits,
            cached_lines: view.cache.len(),
            cache_bytes: view.cache.bytes() + view.preview_cache.bytes(),
            scroll_x: view.scroll.x,
            scroll_y: view.scroll.y,
            frame_micros: started.elapsed().as_micros(),
            composing: view.composition.is_some(),
            caret_in_viewport: code
                .expand(1. / ui.ctx().pixels_per_point())
                .contains_rect(cursor_rect),
            unnecessary_spans,
            inlay_hints: view.cache.inlay_count(visible.clone()),
            layout_pending,
            highlighting_pending: view.cache.lexical_pending(),
            layout_error: layout_error.clone(),
            selection_count: view.multi.extra.len() + 1,
            selection_highlights,
            selection_highlight_scanned_chars: view.selection_highlights.scanned,
            selection_highlight_pending: view.selection_highlights.pending,
            selection_highlight_limited: view.selection_highlights.limited,
            folding_pending: view.fold_service.pending(),
            folding_regions: view.fold.ranges().len(),
            folded_lines: view
                .fold
                .line_count()
                .saturating_sub(view.fold.visible_line_count()),
            folding_partial: view.fold_truncated,
            navigation_symbols: navigation_anchors.len(),
            navigation_pending,
            navigation_parses: navigation_stats.parses,
            navigation_incremental_parses: navigation_stats.incremental_parses,
            navigation_partial,
            copilot_ghost_rows: view.ghost.as_ref().map_or(0, |g| g.rows.len()),
            copilot_ghost_visible: ghost_visible,
            copilot_ghost_rects: ghost_rects,
        };
        if layout_pending && layout_error.is_none() {
            ui.ctx().request_repaint();
        }
        if view.selection_highlights.pending {
            ui.ctx().request_repaint();
        }
        if let Some(message) = layout_error
            .as_deref()
            .or(layout_pending.then_some("Calculando layout da linha longa…"))
        {
            crate::status_messages::publish(ui.ctx(), message);
        }
        let mut context_action = None;
        let mut context_rename = None;
        response.context_menu(|ui| {
            if self.service.active_view() != Some(view_id) {
                self.send(Command::ActivateView { view: view_id });
            }
            if self.service.active_view() != Some(view_id) {
                ui.label("Ative este editor antes de editar");
                return;
            }

            if ui.button("Desfazer    Ctrl+Z").clicked() {
                self.send(Command::Undo { id });
                self.focus_editor = true;
                response.request_focus();
                ui.close();
            }
            if ui.button("Refazer    Ctrl+Y").clicked() {
                self.send(Command::Redo { id });
                self.focus_editor = true;
                response.request_focus();
                ui.close();
            }
            ui.separator();
            if ui.button("Copiar    Ctrl+C").clicked() {
                input::copy(&self.service, ui, view);
                self.focus_editor = true;
                response.request_focus();
                ui.close();
            }
            if ui.button("Recortar    Ctrl+X").clicked() {
                input::cut(&mut self.service, ui, id, view);
                self.focus_editor = true;
                response.request_focus();
                ui.close();
            }
            if ui.button("Colar    Ctrl+V").clicked() {
                response.request_focus();
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                self.focus_editor = true;
                response.request_focus();
                ui.close();
            }
            ui.separator();
            for (label, action) in [
                ("Ir à declaração · F12", 2),
                ("Localizar usos · Shift+F12", 0),
                ("Ir à implementação · Ctrl+F12", 1),
                ("Ir ao supermétodo/supertipo · Ctrl+Alt+U", 5),
                ("Ir ao tipo · Ctrl+Shift+B", 6),
                ("Completar código · Ctrl+Espaço", 3),
                ("Documentação · Ctrl+Q", 4),
            ] {
                if ui.button(label).clicked() {
                    gutter_action = Some((selection.head, action));
                    ui.close();
                }
            }
            ui.separator();
            ui.menu_button("Refatorar", |ui| {
                let target = view.context_position;
                let supported = self.service.view_document(view_id).is_some_and(|doc| {
                    !doc.is_read_only()
                        && codisseu_core::Language::for_path(&doc.path).is_some()
                        && target.is_some_and(|(version, _)| version == doc.version)
                });
                if ui
                    .add_enabled(
                        supported,
                        egui::Button::new("Renomear símbolo…    Shift+F6"),
                    )
                    .clicked()
                {
                    context_rename = target;
                    ui.close();
                }
            });
            ui.separator();
            if matches!(extension.as_str(), "py" | "pyw")
                && ui.button("Executar testes Python…").clicked()
            {
                self.run_python_test_file();
                ui.close();
            }
            if extension == "php" && ui.button("Executar arquivo de testes PHP").clicked() {
                self.run_php_test_file();
                ui.close();
            }
            ui.menu_button("Git · autoria e histórico", |ui| {
                self.git_insights_menu(ui)
            });
            let history_enabled = self
                .service
                .documents
                .iter()
                .any(|doc| doc.id == id && !doc.is_untitled());
            ui.add_enabled_ui(history_enabled, |ui| {
                ui.menu_button("Local History", |ui| {
                    if ui.button("Show History").clicked() {
                        self.open_file_history(false);
                        ui.close();
                    }
                    if ui.button("Put Label…").clicked() {
                        self.open_file_history(true);
                        ui.close();
                    }
                });
            });
            ui.separator();
            for (label, action) in commands::menu_items(&extension) {
                if ui.button(label).clicked() {
                    context_action = Some(action);
                    self.focus_editor = true;
                    response.request_focus();
                    ui.close();
                }
            }
        });
        if let Some(action) = context_action {
            commands::run(
                ui,
                &mut self.service,
                id,
                view,
                &extension,
                &self.tokens,
                action,
                code,
            );
            response.request_focus();
        }
        bridge.view_metrics.insert(view_id, metrics.clone());
        if self.service.active_view() == Some(view_id) {
            bridge.metrics = metrics;
        }
        let definition_click = view.definition_click.take();
        let declaration_click = navigation_current
            && definition_click
                .as_ref()
                .is_some_and(|(at, implementation)| {
                    !implementation
                        && navigation_anchors
                            .iter()
                            .any(|a| a.scalar_range.contains(at))
                });
        self.editor_bridge = bridge;
        if let Some((version, at)) = context_rename {
            self.request_rename_at(view_id, id, version, at);
        }
        if completion_signature {
            self.request_signature(codisseu_core::SignatureHelpTrigger::Invoked, false);
        }
        if is_active {
            self.documentation_hover_target(
                ui.ctx(),
                view_id,
                id,
                self.service.view_document(view_id).unwrap().version,
                documentation_target,
            );
        }
        if let Some((at, implementation)) = definition_click {
            self.caret = Some((id, at));
            if implementation {
                self.go_to_implementation();
            } else if declaration_click {
                self.request_references();
            } else {
                self.go_to_definition();
            }
        }
        if let Some((at, action)) = gutter_action {
            if self.service.active_view() != Some(view_id) {
                self.send(Command::ActivateView { view: view_id });
            }
            self.caret = Some((id, at));
            match action {
                0 => self.request_references(),
                1 => self.go_to_implementation(),
                3 => self.request_completion(),
                4 => self.request_documentation(),
                5 => self.go_to_super_definition(),
                6 => self.go_to_type_definition(),
                _ => self.go_to_definition(),
            }
        }
        ui.advance_cursor_after_rect(all);
    }
}

/// Uses the same bounded shaping policy as editor lines, without changing a document.
pub(super) fn font_preview(ui: &mut egui::Ui, t: &Tokens, ligatures: bool) {
    let mut job = highlight::layout(
        "office ffi  ->  =>  !=  ===  <=",
        "",
        FontId::monospace(t.editor_font_size),
    );
    layout::prepare_job(&mut job, t, ligatures);
    ui.label(job);
}
