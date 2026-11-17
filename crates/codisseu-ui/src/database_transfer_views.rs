//! Local presentation of the already loaded snapshot. Never sends a database command.
use super::{
    Tokens, color,
    components::{Icon, database_icon_button},
};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_core::{remote::Cell, transfer::TransferData};
use eframe::egui::{self, RichText};

const PREVIEW_CELLS: usize = 512;
const PREVIEW_TEXT_BYTES: usize = 128 * 1024;
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum ResultView {
    #[default]
    Grid,
    Tree,
    Text,
    Transpose,
}
#[derive(Default)]
pub(super) struct TransferViews {
    pub mode: ResultView,
    baseline: Option<TransferData>,
    comparison: bool,
    compare_revision: Option<u64>,
    differences: Vec<Difference>,
    difference_count: usize,
    compare_note: String,
    text_revision: Option<u64>,
    text_preview: String,
    query_open: bool,
    query_snapshot: Option<String>,
    pub(super) columns_open: bool,
    columns_was_open: bool,
    pub(super) grid: super::database_grid::GridPresentation,
    value: Option<ValueView>,
}
struct Difference {
    row: usize,
    column: String,
    before: String,
    after: String,
}
struct ValueView {
    row: usize,
    column: usize,
    text: String,
    kind: &'static str,
}
impl TransferViews {
    /// Parent renders this inside a right-to-left toolbar: Settings, View, Compare calls.
    pub fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        data: &TransferData,
        revision: u64,
        query: Option<&str>,
        tokens: &Tokens,
    ) {
        let settings = database_icon_button(ui, Icon::Settings, "Show Options Menu", true);
        egui::Popup::menu(&settings)
            .frame(egui::Frame::popup(ui.style()).fill(color(tokens.bar)))
            .show(|ui| {
                ui.set_min_width(220.);
                if ui
                    .add_enabled(query.is_some(), egui::Button::new("View Query"))
                    .clicked()
                {
                    self.query_snapshot = query.map(str::to_owned);
                    self.query_open = true;
                    ui.close();
                }
                if ui
                    .add_enabled(query.is_some(), egui::Button::new("Copy Query"))
                    .clicked()
                {
                    if let Some(query) = query {
                        ui.ctx().copy_text(query.to_owned());
                    }
                    ui.close();
                }
                ui.separator();
                if ui.button("Reset View").clicked() {
                    self.mode = ResultView::Grid;
                    self.comparison = false;
                    self.text_revision = None;
                    self.grid.reset();
                    ui.close();
                }
                if ui.button("Show Column List").clicked() {
                    self.columns_open = true;
                    ui.close();
                }
                if ui.button("Clear Local Filters").clicked() {
                    self.grid.local.clear();
                    self.grid.changed();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !data.rows.is_empty() && !data.columns.is_empty(),
                        egui::Button::new("Open First Value"),
                    )
                    .clicked()
                {
                    self.open_value(0, 0, &data.rows[0][0], false);
                    ui.close();
                }
            });
        let view = database_icon_button(ui, Icon::View, "View as", true);
        egui::Popup::menu(&view)
            .frame(egui::Frame::popup(ui.style()).fill(color(tokens.bar)))
            .show(|ui| {
                ui.set_min_width(180.);
                for (mode, label) in [
                    (ResultView::Grid, "Table"),
                    (ResultView::Tree, "Tree"),
                    (ResultView::Text, "Text"),
                    (ResultView::Transpose, "Transpose"),
                ] {
                    if ui.selectable_label(self.mode == mode, label).clicked() {
                        self.mode = mode;
                        ui.close();
                    }
                }
            });
        let compare = database_icon_button(ui, Icon::Compare, "Compare Data", true);
        egui::Popup::menu(&compare)
            .frame(egui::Frame::popup(ui.style()).fill(color(tokens.bar)))
            .show(|ui| {
                ui.set_min_width(240.);
                if ui.button("Capture Current Snapshot as Baseline").clicked() {
                    if data.validate().is_ok() {
                        self.baseline = Some(data.clone());
                        self.compare_revision = None;
                        self.comparison = true;
                    }
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.baseline.is_some(),
                        egui::Button::new("Compare with Baseline"),
                    )
                    .clicked()
                {
                    self.comparison = true;
                    self.compare_revision = None;
                    ui.close();
                }
                if ui
                    .add_enabled(self.baseline.is_some(), egui::Button::new("Clear Baseline"))
                    .clicked()
                {
                    self.baseline = None;
                    self.comparison = false;
                    self.differences.clear();
                    ui.close();
                }
            });
        self.refresh_comparison(data, revision);
    }
    /// Returns false for Table so the existing virtualized, editable grid stays authoritative.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        data: &TransferData,
        revision: u64,
        _query: Option<&str>,
        tokens: &Tokens,
    ) -> bool {
        self.refresh_comparison(data, revision);
        if std::mem::take(&mut self.grid.open_columns) {
            self.columns_open = true;
        }
        self.modals(ui, data, tokens);
        if self.comparison {
            self.comparison_panel(ui, data, tokens);
            return true;
        }
        match self.mode {
            ResultView::Grid => false,
            ResultView::Tree => {
                self.tree(ui, data, tokens);
                true
            }
            ResultView::Text => {
                self.text(ui, data, revision, tokens);
                true
            }
            ResultView::Transpose => {
                self.transposed(ui, data, tokens);
                true
            }
        }
    }
    /// Value inspection is read-only. Grid changes remain in the parent's proven staging path.
    pub fn open_value(&mut self, row: usize, column: usize, cell: &Cell, _editable: bool) {
        self.value = Some(ValueView {
            row,
            column,
            text: cell.clipboard_text(),
            kind: match cell {
                Cell::Null => "NULL",
                Cell::Text(_) => "Text",
                Cell::Number(_) => "Number",
                Cell::Bool(_) => "Boolean",
                Cell::Json(_) => "JSON",
                Cell::Binary(_) => "Binary (hex)",
            },
        });
    }
    fn refresh_comparison(&mut self, data: &TransferData, revision: u64) {
        if !self.comparison || self.compare_revision == Some(revision) {
            return;
        }
        self.compare_revision = Some(revision);
        self.differences.clear();
        self.difference_count = 0;
        let Some(baseline) = self.baseline.as_ref() else {
            return;
        };
        if baseline.columns != data.columns {
            self.compare_note =
                "As colunas mudaram. Capture uma nova baseline para comparar os valores.".into();
            return;
        }
        for row in 0..baseline.rows.len().max(data.rows.len()) {
            for (column, name) in data.columns.iter().enumerate() {
                let before = baseline.rows.get(row).and_then(|r| r.get(column));
                let after = data.rows.get(row).and_then(|r| r.get(column));
                if before != after {
                    self.difference_count += 1;
                    if self.differences.len() < PREVIEW_CELLS {
                        self.differences.push(Difference {
                            row,
                            column: name.clone(),
                            before: before
                                .map(Cell::display)
                                .unwrap_or_else(|| "<linha ausente>".into()),
                            after: after
                                .map(Cell::display)
                                .unwrap_or_else(|| "<linha ausente>".into()),
                        });
                    }
                }
            }
        }
        self.compare_note = format!(
            "{} células diferentes · {} → {} linhas · comparação por posição no snapshot",
            self.difference_count,
            baseline.rows.len(),
            data.rows.len()
        );
        if data.source_truncated || baseline.source_truncated {
            self.compare_note
                .push_str(" · origem limitada à página carregada");
        }
    }
    fn comparison_panel(&mut self, ui: &mut egui::Ui, data: &TransferData, tokens: &Tokens) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Compare Data")
                    .strong()
                    .color(color(tokens.text)),
            );
            ui.label(
                RichText::new(&self.compare_note)
                    .small()
                    .color(color(tokens.muted)),
            );
            if ui.small_button("Close").clicked() {
                self.comparison = false;
            }
        });
        if let Some(baseline) = &self.baseline {
            let height = (ui.available_height() - 60.).max(100.);
            ui.columns(2, |columns| {
                comparison_grid(&mut columns[0], "Baseline", baseline, data, height, tokens);
                comparison_grid(&mut columns[1], "Current", data, baseline, height, tokens);
            });
        }
        if !self.differences.is_empty() {
            egui::ScrollArea::both()
                .id_salt("snapshot-comparison")
                .max_height(90.)
                .codisseu()
                .show(ui, |ui| {
                    egui::Grid::new("snapshot-diff-values")
                        .striped(true)
                        .show(ui, |ui| {
                            for label in ["Row", "Column", "Baseline", "Current"] {
                                ui.strong(label);
                            }
                            ui.end_row();
                            for difference in &self.differences {
                                ui.label((difference.row + 1).to_string());
                                ui.label(&difference.column);
                                ui.label(&difference.before);
                                ui.label(&difference.after);
                                ui.end_row();
                            }
                        });
                });
        }
        if self.difference_count > self.differences.len() {
            ui.label(
                RichText::new(
                    "Prévia limitada a 512 diferenças; o contador cobre todo o snapshot carregado.",
                )
                .small()
                .color(color(tokens.muted)),
            );
        }
        ui.separator();
    }
    fn tree(&mut self, ui: &mut egui::Ui, data: &TransferData, tokens: &Tokens) {
        ui.label(
            RichText::new("Tree · snapshot carregado · duplo clique no valor para abrir")
                .small()
                .color(color(tokens.muted)),
        );
        let mut budget = PREVIEW_CELLS;
        egui::ScrollArea::both().id_salt("snapshot-tree").codisseu().show(ui,|ui|{for(row_index,row)in data.rows.iter().enumerate(){egui::CollapsingHeader::new(format!("[{}]  {} fields",row_index+1,row.len())).id_salt(("result-tree-row",row_index)).show(ui,|ui|{for(column,cell)in row.iter().enumerate(){if budget==0{ui.label("Prévia limitada a 512 valores abertos. Recolha linhas para ver outros valores.");break;}budget-=1;ui.horizontal(|ui|{ui.label(RichText::new(data.columns.get(column).map(String::as_str).unwrap_or("?")).color(color(tokens.database_grid_header_foreground)));let response=ui.selectable_label(false,RichText::new(cell.display()).monospace().color(color(if matches!(cell,Cell::Null){tokens.database_grid_null}else{tokens.database_grid_foreground})));if response.double_clicked(){self.open_value(row_index,column,cell,false);}});}});}});
    }
    fn text(&mut self, ui: &mut egui::Ui, data: &TransferData, revision: u64, tokens: &Tokens) {
        if self.text_revision != Some(revision) {
            self.text_revision = Some(revision);
            self.text_preview.clear();
            let mut count = 0;
            self.text_preview.push_str(
                &data
                    .columns
                    .iter()
                    .map(|value| preview_csv_field(&value.chars().take(256).collect::<String>()))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            self.text_preview.push('\n');
            'rows: for row in &data.rows {
                for (index, cell) in row.iter().enumerate() {
                    if count >= PREVIEW_CELLS {
                        break 'rows;
                    }
                    let field = match cell {
                        Cell::Null => r"\N".into(),
                        Cell::Text(value) => {
                            preview_csv_field(&value.chars().take(512).collect::<String>())
                        }
                        _ => preview_csv_field(&cell.display()),
                    };
                    if self.text_preview.len() + field.len() + 2 > PREVIEW_TEXT_BYTES {
                        break 'rows;
                    }
                    if index > 0 {
                        self.text_preview.push(',');
                    }
                    self.text_preview.push_str(&field);
                    count += 1;
                }
                self.text_preview.push('\n');
            }
        }
        ui.label(RichText::new("Text · prévia CSV abreviada de até 512 células/128 KiB. Export/Copy usa o extrator completo escolhido.").small().color(color(tokens.muted)));
        egui::ScrollArea::both()
            .id_salt("snapshot-text")
            .codisseu()
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut self.text_preview)
                        .font(egui::TextStyle::Monospace)
                        .interactive(false)
                        .desired_width(f32::INFINITY),
                );
            });
    }
    fn transposed(&mut self, ui: &mut egui::Ui, data: &TransferData, tokens: &Tokens) {
        ui.label(
            RichText::new(
                "Transpose · somente apresentação · até 512 células · duplo clique para abrir valor",
            )
            .small()
            .color(color(tokens.muted)),
        );
        let row_limit = if data.columns.is_empty() {
            0
        } else {
            data.rows
                .len()
                .min((PREVIEW_CELLS / data.columns.len()).max(1))
        };
        egui::ScrollArea::both()
            .id_salt("snapshot-transpose")
            .codisseu()
            .show(ui, |ui| {
                egui::Grid::new("snapshot-transposed-grid")
                    .striped(true)
                    .min_row_height(tokens.database_grid_row_height)
                    .show(ui, |ui| {
                        ui.strong("Column");
                        for row in 0..row_limit {
                            ui.strong((row + 1).to_string());
                        }
                        ui.end_row();
                        for (column, name) in data.columns.iter().enumerate() {
                            ui.label(
                                RichText::new(name)
                                    .color(color(tokens.database_grid_header_foreground)),
                            );
                            for row in 0..row_limit {
                                if let Some(cell) = data.rows[row].get(column) {
                                    let response = ui.selectable_label(
                                        false,
                                        RichText::new(cell.display()).monospace(),
                                    );
                                    if response.double_clicked() {
                                        self.open_value(row, column, cell, false);
                                    }
                                }
                            }
                            ui.end_row();
                        }
                    });
            });
    }
    fn modals(&mut self, ui: &mut egui::Ui, data: &TransferData, tokens: &Tokens) {
        if self.query_open {
            let mut open = true;
            egui::Window::new("Query")
                .open(&mut open)
                .default_size([720., 320.])
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(
                        RichText::new("SQL que produziu este snapshot")
                            .small()
                            .color(color(tokens.muted)),
                    );
                    if let Some(query) = &self.query_snapshot {
                        if ui.button("Copy Query").clicked() {
                            ui.ctx().copy_text(query.clone());
                        }
                        egui::ScrollArea::both()
                            .max_height(380.)
                            .codisseu()
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(query).monospace())
                                        .selectable(true),
                                );
                            });
                    } else {
                        ui.label("Consulta indisponível para este snapshot.");
                    }
                });
            self.query_open = open;
        }
        if self.columns_open {
            let mut close = false;
            let opening = !self.columns_was_open;
            let frame = super::database_grid_filters::popup_frame(ui, tokens);
            let modal = egui::Modal::new(egui::Id::unique("database.column.list"))
                .backdrop_color(egui::Color32::TRANSPARENT)
                .frame(frame)
                .show(ui.ctx(), |ui| {
                    ui.set_width(320_f32.min(ui.ctx().content_rect().width() - 48.));
                    ui.horizontal(|ui| {
                        ui.strong("Column List");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            close =
                                database_icon_button(ui, Icon::Close, "Close Column List", true)
                                    .clicked();
                        });
                    });
                    let down = ui
                        .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown));
                    let up =
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp));
                    let toggle =
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space));
                    if toggle {
                        ui.input_mut(|i| {
                            i.events
                                .retain(|e| !matches!(e,egui::Event::Text(t) if t==" "))
                        });
                    }
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.grid.column_query)
                            .align(egui::Align2::LEFT_CENTER)
                            .hint_text("Search columns")
                            .char_limit(512)
                            .desired_width(f32::INFINITY),
                    );
                    if opening {
                        field.request_focus();
                    }
                    field.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            true,
                            "Search columns",
                        )
                    });
                    let columns: Vec<_> = data
                        .columns
                        .iter()
                        .enumerate()
                        .filter(|(_, name)| {
                            name.to_lowercase()
                                .contains(&self.grid.column_query.to_lowercase())
                        })
                        .collect();
                    if !columns.iter().any(|(i, _)| *i == self.grid.column_selected)
                        && let Some((first, _)) = columns.first()
                    {
                        self.grid.column_selected = *first;
                    }
                    let selected = columns
                        .iter()
                        .position(|(i, _)| *i == self.grid.column_selected)
                        .unwrap_or(0);
                    if (down || up) && !columns.is_empty() {
                        let next = if down {
                            (selected + 1) % columns.len()
                        } else {
                            (selected + columns.len() - 1) % columns.len()
                        };
                        self.grid.column_selected = columns[next].0;
                    }
                    if toggle && !columns.is_empty() {
                        self.grid.toggle_column(self.grid.column_selected);
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("database.column.list.scroll")
                        .max_height((ui.ctx().content_rect().height() - 180.).clamp(80., 320.))
                        .codisseu()
                        .show(ui, |ui| {
                            for (index, name) in columns {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), tokens.row_height),
                                    egui::Sense::click(),
                                );
                                if self.grid.column_selected == index || response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        4.,
                                        color(if self.grid.column_selected == index {
                                            tokens.selection
                                        } else {
                                            tokens.border
                                        }),
                                    );
                                }
                                let hidden = self.grid.hidden.contains(&index);
                                let tint = color(if hidden {
                                    tokens.muted
                                } else {
                                    tokens.database_grid_foreground
                                });
                                let galley = ui.painter().layout_no_wrap(
                                    name.clone(),
                                    egui::TextStyle::Body.resolve(ui.style()),
                                    tint,
                                );
                                let text_rect = egui::Rect::from_min_max(
                                    egui::pos2(rect.left() + 26., rect.top()),
                                    rect.right_bottom(),
                                );
                                let text_pos = egui::pos2(
                                    text_rect.left(),
                                    rect.center().y - galley.size().y / 2.,
                                );
                                let text_width = galley.size().x.min(text_rect.width());
                                let painter = ui.painter().with_clip_rect(text_rect);
                                painter.galley(text_pos, galley, tint);
                                if hidden {
                                    painter.line_segment(
                                        [
                                            text_rect.left_center(),
                                            text_rect.left_center() + egui::vec2(text_width, 0.),
                                        ],
                                        egui::Stroke::new(1., tint),
                                    );
                                }
                                let icon = egui::Rect::from_center_size(
                                    egui::pos2(
                                        response.rect.left() + 12.,
                                        response.rect.center().y,
                                    ),
                                    egui::vec2(14., 14.),
                                );
                                super::components::paint_icon(
                                    ui,
                                    icon,
                                    Icon::Column,
                                    color(tokens.muted),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::ListBoxOption,
                                        true,
                                        format!("Column {}: {name}", index + 1),
                                    )
                                });
                                ui.ctx().accesskit_node_builder(response.id, |node| {
                                    node.set_selected(self.grid.column_selected == index)
                                });
                                if response.clicked() {
                                    self.grid.column_selected = index;
                                }
                                if response.double_clicked() {
                                    self.grid.toggle_column(index);
                                }
                                if (down || up) && self.grid.column_selected == index {
                                    response.scroll_to_me(None);
                                }
                            }
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Show All Columns").clicked() {
                            self.grid.hidden.clear();
                            self.grid.changed();
                        }
                        ui.label(
                            RichText::new("Space · Show/Hide")
                                .small()
                                .color(color(tokens.muted)),
                        );
                    });
                    close |=
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                });
            if close || modal.should_close() {
                self.columns_open = false;
                if let Some(id) = self.grid.find_button {
                    ui.ctx().memory_mut(|m| m.request_focus(id));
                }
            }
        }
        self.columns_was_open = self.columns_open;
        if let Some(value) = &mut self.value {
            let mut open = true;
            egui::Window::new("Value Editor")
                .open(&mut open)
                .default_size([620., 360.])
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Row {} · Column {} · {} · leitura",
                            value.row + 1,
                            value.column + 1,
                            value.kind
                        ));
                        if ui.button("Copy Value").clicked() {
                            ui.ctx().copy_text(value.text.clone());
                        }
                    });
                    egui::ScrollArea::both()
                        .max_height(480.)
                        .codisseu()
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut value.text)
                                    .font(egui::TextStyle::Monospace)
                                    .interactive(false)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                });
            if !open {
                self.value = None;
            }
        }
    }
}

fn preview_csv_field(value: &str) -> String {
    if value.is_empty() || value == r"\N" || value.contains([',', '"', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.into()
    }
}

fn comparison_grid(
    ui: &mut egui::Ui,
    title: &str,
    data: &TransferData,
    other: &TransferData,
    height: f32,
    tokens: &Tokens,
) {
    ui.label(RichText::new(title).strong());
    egui::ScrollArea::both()
        .id_salt(("comparison-grid", title))
        .max_height(height.min(300.))
        .codisseu()
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let columns = data.columns.len().min(32);
            let rows = data.rows.len().min((512 / columns.max(1)).min(32));
            egui::Grid::new(("comparison-table", title))
                .spacing(egui::Vec2::ZERO)
                .show(ui, |ui| {
                    super::database::grid_cell(
                        ui,
                        "#",
                        25.,
                        27.,
                        [true, false, false, false, false, true, false, false],
                        tokens,
                    );
                    for name in data.columns.iter().take(columns) {
                        super::database::grid_cell(
                            ui,
                            name,
                            163.,
                            27.,
                            [true, false, false, false, false, false, false, false],
                            tokens,
                        );
                    }
                    ui.end_row();
                    for (row, values) in data.rows.iter().take(rows).enumerate() {
                        super::database::grid_cell(
                            ui,
                            &(row + 1).to_string(),
                            25.,
                            27.,
                            [false, false, false, false, false, true, false, false],
                            tokens,
                        );
                        for (column, cell) in values.iter().take(columns).enumerate() {
                            let text = match cell {
                                Cell::Text(s) | Cell::Number(s) | Cell::Json(s) => {
                                    s.chars().take(256).collect()
                                }
                                _ => cell.display(),
                            };
                            let changed =
                                other.rows.get(row).and_then(|r| r.get(column)) != Some(cell);
                            super::database::grid_cell(
                                ui,
                                &text,
                                163.,
                                27.,
                                [
                                    false,
                                    false,
                                    changed,
                                    matches!(cell, Cell::Number(_)),
                                    matches!(cell, Cell::Null),
                                    false,
                                    false,
                                    false,
                                ],
                                tokens,
                            );
                        }
                        ui.end_row();
                    }
                });
            if rows < data.rows.len() || columns < data.columns.len() {
                ui.label("Prévia limitada a512células; contador compara todo o snapshot.");
            }
        });
}
