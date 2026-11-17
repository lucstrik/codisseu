//! Presentation maps always refer to original result coordinates, including edits.
use super::*;
use codisseu_core::{remote::Cell, transfer::TransferData};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Instant,
};

pub(super) struct GridPresentation {
    source: String,
    columns: Vec<String>,
    pub hidden: BTreeSet<usize>,
    pub widths: BTreeMap<usize, f32>,
    drag: Option<(usize, f32)>,
    pub query: String,
    pub find_open: bool,
    pub match_case: bool,
    pub filter_rows: bool,
    pub focus_find: bool,
    pub find_button: Option<egui::Id>,
    reveal: Option<(usize, usize)>,
    rows: Vec<usize>,
    matches: Vec<bool>,
    match_count: usize,
    current_match: Option<usize>,
    revision: Option<u64>,
    dirty: bool,
    reset_selection: bool,
    job: Option<FilterJob>,
    proposals: BTreeMap<(usize, usize), Cell>,
    #[cfg(test)]
    scanned: usize,
    pub open_columns: bool,
    pub column_query: String,
    pub column_selected: usize,
    pub local: database_grid_filters::LocalFilters,
    select_column: Option<usize>,
}
impl Default for GridPresentation {
    fn default() -> Self {
        Self {
            source: String::new(),
            columns: vec![],
            hidden: BTreeSet::new(),
            widths: BTreeMap::new(),
            drag: None,
            query: String::new(),
            find_open: false,
            match_case: false,
            filter_rows: true,
            focus_find: false,
            find_button: None,
            reveal: None,
            rows: vec![],
            matches: vec![],
            match_count: 0,
            current_match: None,
            revision: None,
            dirty: true,
            reset_selection: false,
            job: None,
            proposals: BTreeMap::new(),
            #[cfg(test)]
            scanned: 0,
            open_columns: false,
            column_query: String::new(),
            column_selected: 0,
            local: Default::default(),
            select_column: None,
        }
    }
}
struct FilterJob {
    data: Arc<TransferData>,
    query: String,
    case: bool,
    columns: Vec<usize>,
    at: usize,
    matches: Vec<bool>,
    matched_rows: Vec<bool>,
    allowed_rows: Vec<bool>,
    filters: BTreeMap<usize, BTreeSet<database_grid_filters::Key>>,
}
impl GridPresentation {
    pub fn sync_source(&mut self, source: &str, columns: &[String]) {
        if self.source != source || self.columns != columns {
            *self = Self {
                source: source.into(),
                columns: columns.to_vec(),
                ..Self::default()
            };
        }
    }
    pub fn visible_columns(&self) -> Vec<usize> {
        (0..self.columns.len())
            .filter(|c| !self.hidden.contains(c))
            .collect()
    }
    pub fn visible_rows(&self) -> Vec<usize> {
        self.rows.clone()
    }
    pub fn changed(&mut self) {
        self.dirty = true;
        self.reset_selection = true;
    }
    pub fn toggle_column(&mut self, column: usize) {
        if !self.hidden.remove(&column) {
            self.hidden.insert(column);
        }
        self.changed();
    }
    pub fn reset(&mut self) {
        self.hidden.clear();
        self.widths.clear();
        self.query.clear();
        self.find_open = false;
        self.column_query.clear();
        self.local.clear();
        self.local.enabled = false;
        self.local.close();
        self.changed();
    }
    pub fn take_selection_reset(&mut self) -> bool {
        std::mem::take(&mut self.reset_selection)
    }
    pub fn has_projection(&self) -> bool {
        self.revision.is_some()
    }
    pub fn pending(&self) -> bool {
        self.job.is_some()
    }
    pub fn matches(&self, row: usize, column: usize) -> bool {
        self.find_open
            && !self.query.is_empty()
            && self.matches.get(row * self.columns.len() + column) == Some(&true)
    }
    pub fn prepare(
        &mut self,
        data: Arc<TransferData>,
        revision: u64,
        proposals: &BTreeMap<(usize, usize), Cell>,
    ) {
        if self.revision != Some(revision) {
            self.revision = Some(revision);
            self.rows = (0..data.rows.len()).collect();
            self.matches.clear();
            self.match_count = 0;
            self.job = None;
            self.dirty = true;
        }
        if ((self.find_open && !self.query.is_empty()) || self.local.active())
            && self.proposals != *proposals
        {
            self.proposals = proposals.clone();
            self.dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            self.current_match = None;
            self.reveal = None;
            let search = self.find_open && !self.query.is_empty();
            self.job = if search || self.local.active() {
                let filters = if self.local.enabled {
                    self.local.selections.clone()
                } else {
                    BTreeMap::new()
                };
                let mut columns: BTreeSet<_> = if search {
                    self.visible_columns().into_iter().collect()
                } else {
                    BTreeSet::new()
                };
                columns.extend(filters.keys().copied());
                Some(FilterJob {
                    query: if !search {
                        String::new()
                    } else if self.match_case {
                        self.query.clone()
                    } else {
                        self.query.to_lowercase()
                    },
                    case: self.match_case,
                    columns: columns.into_iter().collect(),
                    at: 0,
                    matches: vec![false; data.rows.len() * data.columns.len()],
                    matched_rows: vec![!search || !self.filter_rows; data.rows.len()],
                    allowed_rows: vec![true; data.rows.len()],
                    filters,
                    data: data.clone(),
                })
            } else {
                self.rows = (0..data.rows.len()).collect();
                self.matches.clear();
                self.match_count = 0;
                None
            };
        }
        let Some(job) = &mut self.job else {
            return;
        };
        let start = Instant::now();
        let total = job.data.rows.len() * job.columns.len();
        for _ in 0..4096 {
            if job.at >= total {
                break;
            }
            let row = job.at / job.columns.len();
            let col = job.columns[job.at % job.columns.len()];
            let value = self
                .proposals
                .get(&(row, col))
                .or_else(|| job.data.rows[row].get(col));
            let hit = if job.query.is_empty() || self.hidden.contains(&col) {
                false
            } else {
                let text = value.map(Cell::display).unwrap_or_default();
                if job.case {
                    text.contains(&job.query)
                } else {
                    text.to_lowercase().contains(&job.query)
                }
            };
            job.matches[row * job.data.columns.len() + col] = hit;
            #[cfg(test)]
            {
                self.scanned += 1;
            }
            job.matched_rows[row] |= hit;
            if let Some(accepted) = job.filters.get(&col) {
                job.allowed_rows[row] &=
                    value.is_some_and(|v| accepted.contains(&database_grid_filters::key(v)));
            }
            job.at += 1;
            if start.elapsed() >= Duration::from_millis(2) {
                break;
            }
        }
        if job.at >= total {
            self.rows = (0..job.data.rows.len())
                .filter(|r| job.allowed_rows[*r] && job.matched_rows[*r])
                .collect();
            for (row, allowed) in job.allowed_rows.iter().enumerate() {
                if !allowed {
                    let start = row * job.data.columns.len();
                    job.matches[start..start + job.data.columns.len()].fill(false);
                }
            }
            self.matches = std::mem::take(&mut job.matches);
            self.match_count = self.matches.iter().filter(|m| **m).count();
            self.job = None;
        }
    }
    fn next_match(&mut self, reverse: bool) -> Option<(usize, usize)> {
        let len = self.matches.len();
        if len == 0 {
            return None;
        }
        let start = self
            .current_match
            .unwrap_or(if reverse { 0 } else { len - 1 });
        for offset in 1..=len {
            let at = if reverse {
                (start + len - offset) % len
            } else {
                (start + offset) % len
            };
            if self.matches[at] {
                self.current_match = Some(at);
                self.reveal = Some((at / self.columns.len(), at % self.columns.len()));
                return self.reveal;
            }
        }
        None
    }
    pub fn take_scroll_target(
        &mut self,
        rows: &[usize],
        columns: &[usize],
        widths: &[f32],
        row_height: f32,
    ) -> Option<egui::Vec2> {
        let (row, col) = self.reveal.take()?;
        let r = rows.iter().position(|r| *r == row)?;
        let c = columns.iter().position(|c| *c == col)?;
        Some(egui::vec2(
            columns[..c].iter().map(|c| widths[*c]).sum(),
            r as f32 * row_height,
        ))
    }
    pub fn header(
        &mut self,
        ui: &mut egui::Ui,
        response: egui::Response,
        col: usize,
        name: &str,
        width: f32,
        tokens: &Tokens,
    ) -> egui::Response {
        response.context_menu(|ui| {
            ui.set_min_width(220.);
            if ui.button("Copy Column Name").clicked() {
                ui.ctx().copy_text(name.into());
                ui.close();
            }
            if ui.button("Select Column").clicked() {
                self.select_column = Some(col);
                ui.close();
            }
            if ui.button("Clear Local Filters").clicked() {
                self.local.clear();
                self.changed();
                ui.close();
            }
            ui.separator();
            if ui.button("Hide Column").clicked() {
                self.hidden.insert(col);
                self.changed();
                ui.close();
            }
            if ui.button("Hide Other Columns").clicked() {
                self.hidden = (0..self.columns.len()).filter(|c| *c != col).collect();
                self.changed();
                ui.close();
            }
            if ui.button("Show All Columns").clicked() {
                self.hidden.clear();
                self.changed();
                ui.close();
            }
            if ui.button("Show Column List").clicked() {
                self.open_columns = true;
                self.column_selected = col;
                ui.close();
            }
            ui.separator();
            if ui.button("Reset Column Width").clicked() {
                self.widths.remove(&col);
                ui.close();
            }
        });
        self.local.header(ui, &response, col, name, tokens);
        let edge = egui::Rect::from_center_size(
            egui::pos2(response.rect.right(), response.rect.center().y),
            egui::vec2(6., response.rect.height()),
        );
        let resize = ui
            .interact(
                edge,
                response.id.with("resize"),
                egui::Sense::click_and_drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        resize.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Slider,
                true,
                format!("Resize column {name} ({})", col + 1),
            )
        });
        if resize.drag_started() {
            self.drag = Some((col, width));
        }
        if resize.dragged()
            && let Some((index, start)) = self.drag
            && index == col
        {
            self.widths.insert(
                col,
                (start + resize.total_drag_delta().unwrap_or_default().x).clamp(64., 1200.),
            );
        }
        if resize.clicked() {
            resize.request_focus();
        }
        if resize.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resize.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        ..Default::default()
                    },
                )
            });
            let right = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight));
            let left = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
            if right || left {
                self.widths.insert(
                    col,
                    (width + if right { 16. } else { -16. }).clamp(64., 1200.),
                );
            }
        }
        if resize.drag_stopped() {
            self.drag = None;
        }
        // A separate double-click target on the divider restores its natural width.
        if resize.double_clicked() {
            self.widths.remove(&col);
        }
        if resize.hovered() || resize.dragged() {
            ui.painter().line_segment(
                [edge.center_top(), edge.center_bottom()],
                egui::Stroke::new(1., color(tokens.accent)),
            );
        }
        response
    }
}

impl CodisseuApp {
    pub(super) fn grid_inspection_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let focused = ctx.memory(|m| m.focused());
        let find_id = egui::Id::unique("database.grid.find");
        let in_grid = focused.is_some()
            && (focused == self.grid_focus
                || focused == self.transfer_views.grid.find_button
                || focused == Some(find_id));
        if !in_grid {
            return false;
        }
        if consume_exact(ctx, Modifiers::CTRL, Key::F12) {
            self.transfer_views.columns_open = true;
            return true;
        }
        if consume_exact(ctx, Modifiers::CTRL, Key::F) {
            self.transfer_views.grid.find_open = true;
            self.transfer_views.grid.focus_find = true;
            self.transfer_views.grid.changed();
            return true;
        }
        if focused == Some(find_id) {
            if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
                self.close_grid_find(ctx);
            }
            return true;
        }
        false
    }
    fn close_grid_find(&mut self, ctx: &egui::Context) {
        let grid = &mut self.transfer_views.grid;
        grid.find_open = false;
        grid.query.clear();
        grid.changed();
        if let Some(id) = grid.find_button {
            ctx.memory_mut(|m| m.request_focus(id));
        }
    }
    pub(super) fn grid_inspection_bar(&mut self, ui: &mut egui::Ui) {
        if self.transfer_views.mode != database_transfer_views::ResultView::Grid {
            return;
        }
        let Some(data) = self.result_snapshot() else {
            return;
        };
        let revision = self.result_transfer_revision();
        let proposals = if self.database_ui.remote_results {
            &self.database_ui.staged
        } else {
            static EMPTY: std::sync::OnceLock<BTreeMap<(usize, usize), Cell>> =
                std::sync::OnceLock::new();
            EMPTY.get_or_init(BTreeMap::new)
        };
        let grid = &mut self.transfer_views.grid;
        if grid.revision != Some(revision) {
            grid.local.close();
        }
        if grid.local.popup(ui, &data, proposals, &self.tokens) {
            grid.changed();
        }
        let mut navigate = None;
        if grid.find_open {
            let mut close = false;
            let mut next = false;
            let mut previous = false;
            ui.horizontal(|ui| {
                let (_, icon) = ui.allocate_space(egui::vec2(16., 22.));
                components::paint_icon(ui, icon, Icon::Search, color(self.tokens.muted));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut grid.query)
                        .id(find_id())
                        // Keep focus until our scoped Escape handler closes the search.
                        .event_filter(egui::EventFilter {
                            escape: true,
                            horizontal_arrows: true,
                            ..Default::default()
                        })
                        .char_limit(4096)
                        .hint_text("Find on current page")
                        .align(egui::Align2::LEFT_CENTER)
                        .desired_width((ui.available_width() - 252.).max(70.)),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Find on current page",
                    )
                });
                if grid.focus_find {
                    field.request_focus();
                    grid.focus_find = false;
                }
                let mut changed = field.changed();
                let case = ui
                    .selectable_label(grid.match_case, "Cc")
                    .on_hover_text("Match case");
                case.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::accesskit::Role::CheckBox,
                        true,
                        grid.match_case,
                        "Match case",
                    )
                });
                if case.clicked() {
                    grid.match_case = !grid.match_case;
                    changed = true;
                }
                changed |= ui.checkbox(&mut grid.filter_rows, "Filter rows").changed();
                if changed {
                    grid.changed();
                }
                previous = components::database_icon_button(
                    ui,
                    Icon::Up,
                    "Previous match",
                    !grid.pending() && grid.match_count > 0,
                )
                .clicked();
                next = components::database_icon_button(
                    ui,
                    Icon::Down,
                    "Next match",
                    !grid.pending() && grid.match_count > 0,
                )
                .clicked();
                close = components::database_icon_button(
                    ui,
                    Icon::Close,
                    "Close Find on Current Page",
                    true,
                )
                .clicked();
                if field.has_focus() {
                    if consume_exact(ui.ctx(), Modifiers::SHIFT, Key::Enter) {
                        previous = true;
                    } else if consume_exact(ui.ctx(), Modifiers::NONE, Key::Enter) {
                        next = true;
                    }
                }
            });
            if next || previous {
                navigate = Some(previous);
            }
            if close {
                let grid = &mut self.transfer_views.grid;
                grid.find_open = false;
                grid.query.clear();
                grid.changed();
                if let Some(id) = grid.find_button {
                    ui.ctx().memory_mut(|m| m.request_focus(id));
                }
            }
        }
        let grid = &mut self.transfer_views.grid;
        grid.prepare(data.clone(), revision, proposals);
        if grid.take_selection_reset() {
            self.grid_selection.clear();
        }
        if let Some(column) = grid.select_column.take()
            && !data.rows.is_empty()
        {
            self.grid_selection.clicked(0, column, Modifiers::NONE);
            self.grid_selection
                .clicked(data.rows.len() - 1, column, Modifiers::SHIFT);
            self.grid_focus = grid.find_button;
            if let Some(id) = self.grid_focus {
                ui.ctx().memory_mut(|m| m.request_focus(id));
            }
        }
        if let Some(reverse) = navigate
            && !grid.pending()
            && let Some((row, col)) = grid.next_match(reverse)
        {
            self.grid_selection.clicked(row, col, Modifiers::NONE);
        }
        if grid.pending() {
            ui.label("Filtering current page…");
            ui.ctx().request_repaint();
        } else if (grid.find_open && !grid.query.is_empty()) || grid.local.active() {
            let matches = if grid.find_open && !grid.query.is_empty() {
                format!(" · {} matches", grid.match_count)
            } else {
                String::new()
            };
            ui.label(
                egui::RichText::new(format!(
                    "{} / {} rows{matches} · current page",
                    grid.rows.len(),
                    data.rows.len()
                ))
                .small()
                .color(color(self.tokens.muted)),
            );
        }
    }
}
fn find_id() -> egui::Id {
    egui::Id::unique("database.grid.find")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Arc<TransferData> {
        Arc::new(TransferData {
            columns: vec!["id".into(), "name".into(), "note".into()],
            rows: vec![
                vec![
                    Cell::Number("9223372036854775807".into()),
                    Cell::Text("AÇÃO 😀".into()),
                    Cell::Null,
                ],
                vec![
                    Cell::Number("2".into()),
                    Cell::Text(String::new()),
                    Cell::Text("private".into()),
                ],
                vec![
                    Cell::Number("3".into()),
                    Cell::Text("ação".into()),
                    Cell::Text("".into()),
                ],
            ],
            source_truncated: true,
        })
    }
    fn finish(
        g: &mut GridPresentation,
        data: Arc<TransferData>,
        revision: u64,
        proposals: &BTreeMap<(usize, usize), Cell>,
    ) {
        for _ in 0..2000 {
            g.prepare(data.clone(), revision, proposals);
            if !g.pending() {
                return;
            }
        }
        panic!("filter did not complete");
    }
    #[test]
    fn filter_projection_hidden_columns_proposals_and_idle_cache_keep_original_identity() {
        let data = sample();
        let mut g = GridPresentation::default();
        g.sync_source("sqlite:one", &data.columns);
        g.find_open = true;
        g.query = "ação".into();
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0, 2]);
        assert!(g.matches(0, 1));
        assert!(!g.matches(1, 1));
        assert_eq!(g.next_match(false), Some((0, 1)));
        assert_eq!(g.next_match(false), Some((2, 1)));
        assert_eq!(
            g.take_scroll_target(&[0, 2], &[0, 1, 2], &[100., 200., 100.], 27.),
            Some(egui::vec2(100., 27.))
        );
        assert_eq!(g.next_match(true), Some((0, 1)));
        let scanned = g.scanned;
        for _ in 0..100 {
            g.prepare(data.clone(), 1, &BTreeMap::new());
        }
        assert_eq!(g.scanned, scanned);
        let mut selection = database_selection::GridSelection::default();
        selection.row_clicked(0, Modifiers::NONE);
        selection.row_clicked(2, Modifiers::SHIFT);
        g.toggle_column(2);
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        let copied = selection
            .snapshot_projected(&data, &g.visible_rows(), &g.visible_columns())
            .unwrap();
        assert_eq!(copied.columns, vec!["id", "name"]);
        assert_eq!(
            copied.rows[0][0],
            Cell::Number("9223372036854775807".into())
        );
        assert_eq!(copied.rows.len(), 2);
        let proposals = BTreeMap::from([((1, 1), Cell::Text("ação proposta".into()))]);
        finish(&mut g, data.clone(), 1, &proposals);
        assert_eq!(g.visible_rows(), vec![0, 1, 2]);
        g.toggle_column(1);
        finish(&mut g, data.clone(), 1, &proposals);
        assert!(g.visible_rows().is_empty());
        g.reset();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0, 1, 2]);
        assert_eq!(g.visible_columns(), vec![0, 1, 2]);
        g.query = "AÇÃO".into();
        g.find_open = true;
        g.match_case = true;
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0]);
        g.sync_source("mysql:two", &data.columns);
        assert!(!g.find_open);
        assert!(g.query.is_empty());
        assert!(!g.has_projection());
    }
    #[test]
    fn local_columns_intersect_search_hidden_columns_and_proposals_without_rescanning_idle() {
        let data = sample();
        let mut g = GridPresentation::default();
        g.sync_source("one", &data.columns);
        g.local.enabled = true;
        g.local
            .selections
            .insert(2, BTreeSet::from([database_grid_filters::key(&Cell::Null)]));
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0]);
        g.toggle_column(2);
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0]);
        g.find_open = true;
        g.query = "ação".into();
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.next_match(false), Some((0, 1)));
        assert!(!g.matches(2, 1));
        g.local.selections.insert(
            0,
            BTreeSet::from([database_grid_filters::key(&Cell::Number("3".into()))]),
        );
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert!(g.visible_rows().is_empty());
        let proposals = BTreeMap::from([((2, 2), Cell::Null)]);
        finish(&mut g, data.clone(), 1, &proposals);
        assert_eq!(g.visible_rows(), vec![2]);
        let scanned = g.scanned;
        for _ in 0..100 {
            g.prepare(data.clone(), 1, &proposals);
        }
        assert_eq!(g.scanned, scanned);
        g.local.enabled = false;
        g.changed();
        finish(&mut g, data.clone(), 1, &proposals);
        assert_eq!(g.visible_rows(), vec![0, 2]);
        assert_eq!(g.local.selections.len(), 2);
        g.local.enabled = true;
        g.changed();
        finish(&mut g, data.clone(), 1, &proposals);
        assert_eq!(g.visible_rows(), vec![2]);
        g.local.selections.insert(0, BTreeSet::new());
        g.changed();
        finish(&mut g, data.clone(), 1, &proposals);
        assert!(g.visible_rows().is_empty());
        g.reset();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0, 1, 2]);
        assert!(g.local.selections.is_empty());
        g.local.enabled = true;
        g.sync_source("two", &data.columns);
        assert!(!g.local.enabled);
    }
    #[test]
    fn duplicate_names_and_no_visible_columns_keep_ordinals_and_can_restore() {
        let data = Arc::new(TransferData {
            columns: vec!["same".into(), "same".into()],
            rows: vec![vec![
                Cell::Text("private".into()),
                Cell::Text("keep".into()),
            ]],
            source_truncated: false,
        });
        let mut g = GridPresentation::default();
        g.sync_source("one", &data.columns);
        g.toggle_column(0);
        g.find_open = true;
        g.query = "keep".into();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_columns(), vec![1]);
        assert_eq!(g.next_match(false), Some((0, 1)));
        let mut selection = database_selection::GridSelection::default();
        selection.row_clicked(0, Modifiers::NONE);
        assert_eq!(
            selection
                .snapshot_projected(&data, &g.visible_rows(), &g.visible_columns())
                .unwrap()
                .rows,
            vec![vec![Cell::Text("keep".into())]]
        );
        g.toggle_column(1);
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert!(g.visible_columns().is_empty() && g.visible_rows().is_empty());
        assert!(g.next_match(false).is_none());
        assert!(selection.snapshot_projected(&data, &[], &[]).is_err());
        g.reset();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_columns(), vec![0, 1]);
        g.query = "keep".into();
        g.find_open = true;
        g.changed();
        g.prepare(data.clone(), 1, &BTreeMap::new());
        g.filter_rows = false;
        g.changed();
        finish(&mut g, data, 1, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0]);
    }
    #[test]
    fn incremental_search_is_bounded_and_superseded_work_cannot_publish() {
        let data = Arc::new(TransferData {
            columns: (0..256).map(|i| format!("c{i}")).collect(),
            rows: (0..500)
                .map(|_| vec![Cell::Text("hay".into()); 256])
                .collect(),
            source_truncated: true,
        });
        let mut g = GridPresentation::default();
        g.sync_source("one", &data.columns);
        g.find_open = true;
        g.query = "missing".into();
        g.prepare(data.clone(), 1, &BTreeMap::new());
        assert!(g.pending());
        assert!(g.scanned <= 4096);
        g.query = "hay".into();
        g.changed();
        finish(&mut g, data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.visible_rows().len(), 500);
        let before = g.scanned;
        g.prepare(data.clone(), 1, &BTreeMap::new());
        assert_eq!(g.scanned, before);
        g.query = "missing".into();
        g.changed();
        g.prepare(data.clone(), 1, &BTreeMap::new());
        let new = sample();
        g.sync_source("two", &new.columns);
        finish(&mut g, new, 2, &BTreeMap::new());
        assert_eq!(g.visible_rows(), vec![0, 1, 2]);
        assert!(!g.pending());
    }
}
