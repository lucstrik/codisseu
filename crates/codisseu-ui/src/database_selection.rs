//! Result selection is independent of the renderer and never fills holes with NULL.
use codisseu_core::transfer::TransferData;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Cells,
    Rows,
}
#[derive(Clone, Copy, Debug)]
struct Rect {
    first_row: usize,
    last_row: usize,
    first_col: usize,
    last_col: usize,
}
impl Rect {
    fn between(a: (usize, usize), b: (usize, usize)) -> Self {
        Self {
            first_row: a.0.min(b.0),
            last_row: a.0.max(b.0),
            first_col: a.1.min(b.1),
            last_col: a.1.max(b.1),
        }
    }
    fn contains(self, row: usize, col: usize) -> bool {
        self.first_row <= row
            && row <= self.last_row
            && self.first_col <= col
            && col <= self.last_col
    }
}

/// Use the click event's modifiers: a key release later in the same frame must
/// not turn Shift+click into a plain click.
pub(super) fn click_modifiers(ui: &egui::Ui) -> egui::Modifiers {
    ui.input(|input| {
        input
            .events
            .iter()
            .rev()
            .find_map(|event| match event {
                egui::Event::PointerButton {
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers,
                    ..
                } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(input.modifiers)
    })
}

/// Shift ranges remain compact even before source bounds are known. Ctrl/Cmd
/// toggles are sparse. Snapshot validates bounds and rectangularity explicitly.
#[derive(Default, Debug)]
pub(super) struct GridSelection {
    mode: Mode,
    anchor: Option<(usize, usize)>,
    ranges: Vec<Rect>,
    toggled: BTreeSet<(usize, usize)>,
    revision: Option<(String, Option<u64>)>,
}
impl GridSelection {
    pub(super) fn clicked(&mut self, row: usize, col: usize, modifiers: egui::Modifiers) {
        self.select(Mode::Cells, (row, col), modifiers);
    }
    pub(super) fn row_clicked(&mut self, row: usize, modifiers: egui::Modifiers) {
        self.select(Mode::Rows, (row, 0), modifiers);
    }
    fn select(&mut self, mode: Mode, point: (usize, usize), modifiers: egui::Modifiers) {
        let additive = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
        if self.mode != mode {
            self.clear();
            self.mode = mode;
        }
        if modifiers.shift {
            let anchor = *self.anchor.get_or_insert(point);
            let rectangle = Rect::between(anchor, point);
            if !additive {
                self.ranges.clear();
                self.toggled.clear();
            } else {
                self.toggled
                    .retain(|&(row, col)| !rectangle.contains(row, col));
            }
            self.ranges.push(rectangle);
        } else if additive {
            self.anchor = Some(point);
            if !self.toggled.insert(point) {
                self.toggled.remove(&point);
            }
        } else {
            self.ranges = vec![Rect::between(point, point)];
            self.toggled.clear();
            self.anchor = Some(point);
        }
    }
    pub(super) fn contains(&self, row: usize, col: usize) -> bool {
        let col = if self.mode == Mode::Rows { 0 } else { col };
        self.ranges.iter().any(|range| range.contains(row, col))
            ^ self.toggled.contains(&(row, col))
    }
    pub(super) fn has_rows(&self) -> bool {
        self.mode == Mode::Rows && !self.is_empty()
    }
    pub(super) fn is_empty(&self) -> bool {
        if self
            .toggled
            .iter()
            .any(|&(row, col)| !self.ranges.iter().any(|range| range.contains(row, col)))
        {
            return false;
        }
        for range in &self.ranges {
            // If every coordinate is toggled off, the iteration is bounded by
            // the number of sparse toggles; otherwise it stops at the first hit.
            for row in range.first_row..=range.last_row {
                for col in range.first_col..=range.last_col {
                    if !self.toggled.contains(&(row, col)) {
                        return false;
                    }
                }
            }
        }
        true
    }
    pub(super) fn clear(&mut self) {
        self.mode = Mode::Cells;
        self.anchor = None;
        self.ranges.clear();
        self.toggled.clear();
    }
    /// Both origin and result identity participate; an identical numeric query id
    /// from a different datasource can never retain a stale selection.
    pub(super) fn sync_revision(&mut self, source: &str, revision: Option<u64>) -> bool {
        let next = (source.to_owned(), revision);
        if self.revision.as_ref() == Some(&next) {
            return false;
        }
        self.clear();
        self.revision = Some(next);
        true
    }
    pub(super) fn snapshot_projected(
        &self,
        source: &TransferData,
        rows: &[usize],
        columns: &[usize],
    ) -> Result<TransferData, String> {
        source.validate()?;
        if rows.iter().any(|r| *r >= source.rows.len())
            || columns.iter().any(|c| *c >= source.columns.len())
        {
            return Err("Visible projection no longer belongs to this result".into());
        }
        let mut selection = Self::default();
        for (r, &original_row) in rows.iter().enumerate() {
            for (c, &original_column) in columns.iter().enumerate() {
                if self.contains(original_row, original_column) {
                    selection.toggled.insert((r, c));
                }
            }
        }
        let projected = TransferData {
            columns: columns.iter().map(|c| source.columns[*c].clone()).collect(),
            rows: rows
                .iter()
                .map(|r| {
                    columns
                        .iter()
                        .map(|c| source.rows[*r][*c].clone())
                        .collect()
                })
                .collect(),
            source_truncated: source.source_truncated,
        };
        selection.snapshot(&projected)
    }
    pub(super) fn snapshot(&self, source: &TransferData) -> Result<TransferData, String> {
        source.validate()?;
        if self.is_empty() {
            return Err("Selecione células ou linhas para copiar".into());
        }
        let row_count = source.rows.len();
        let col_count = source.columns.len();
        if self
            .ranges
            .iter()
            .any(|r| r.last_row >= row_count || r.last_col >= col_count)
            || self
                .toggled
                .iter()
                .any(|&(row, col)| row >= row_count || col >= col_count)
        {
            return Err("Seleção não pertence ao resultado atual; selecione novamente".into());
        }
        let (rows, cols) = if self.mode == Mode::Rows {
            (
                (0..row_count)
                    .filter(|&row| self.contains(row, 0))
                    .collect::<Vec<_>>(),
                (0..col_count).collect::<Vec<_>>(),
            )
        } else {
            let mut rows = BTreeSet::new();
            let mut cols = BTreeSet::new();
            let mut selected = 0usize;
            for row in 0..row_count {
                for col in 0..col_count {
                    if self.contains(row, col) {
                        rows.insert(row);
                        cols.insert(col);
                        selected += 1;
                    }
                }
            }
            if selected != rows.len() * cols.len() {
                return Err("A seleção tem lacunas. Selecione um retângulo ou use os números de linha para copiar linhas inteiras; nenhum NULL será inventado".into());
            }
            (rows.into_iter().collect(), cols.into_iter().collect())
        };
        if rows.is_empty() || cols.is_empty() {
            return Err("Seleção vazia no resultado atual".into());
        }
        let data = TransferData {
            columns: cols
                .iter()
                .map(|&col| source.columns[col].clone())
                .collect(),
            rows: rows
                .into_iter()
                .map(|row| {
                    cols.iter()
                        .map(|&col| source.rows[row][col].clone())
                        .collect()
                })
                .collect(),
            source_truncated: source.source_truncated,
        };
        data.validate()?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::remote::Cell;
    fn data() -> TransferData {
        TransferData {
            columns: vec!["id".into(), "name".into(), "value".into()],
            rows: vec![
                vec![
                    Cell::Number("1".into()),
                    Cell::Text("Ação".into()),
                    Cell::Null,
                ],
                vec![
                    Cell::Number("2".into()),
                    Cell::Text("".into()),
                    Cell::Binary(vec![0, 255]),
                ],
                vec![
                    Cell::Number("3".into()),
                    Cell::Text("third".into()),
                    Cell::Number("9223372036854775807".into()),
                ],
            ],
            source_truncated: true,
        }
    }
    #[test]
    fn click_modifiers_survive_shift_release_later_in_the_same_frame() {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            events: vec![
                egui::Event::PointerButton {
                    pos: egui::pos2(10., 10.),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::SHIFT,
                },
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            assert!(!ui.input(|i| i.modifiers.shift));
            assert!(click_modifiers(ui).shift);
        });
        output.textures_delta.clear();
    }
    #[test]
    fn one_cell_and_shift_rectangle_keep_types_and_original_column_order() {
        let mut selection = GridSelection::default();
        selection.clicked(2, 2, egui::Modifiers::NONE);
        assert_eq!(
            selection.snapshot(&data()).unwrap().rows,
            vec![vec![Cell::Number("9223372036854775807".into())]]
        );
        selection.clicked(0, 1, egui::Modifiers::SHIFT);
        let selected = selection.snapshot(&data()).unwrap();
        assert_eq!(selected.columns, vec!["name", "value"]);
        assert_eq!(selected.rows.len(), 3);
        assert_eq!(selected.rows[0][1], Cell::Null);
        assert_eq!(selected.rows[1][1], Cell::Binary(vec![0, 255]));
        assert!(selected.source_truncated);
    }
    #[test]
    fn ctrl_rows_are_ordered_and_always_include_all_columns() {
        let mut selection = GridSelection::default();
        selection.row_clicked(2, egui::Modifiers::NONE);
        selection.row_clicked(0, egui::Modifiers::CTRL);
        assert!(selection.has_rows());
        assert!(selection.contains(2, 999));
        assert!(!selection.contains(1, 1));
        let selected = selection.snapshot(&data()).unwrap();
        assert_eq!(selected.columns, data().columns);
        assert_eq!(
            selected.rows,
            vec![data().rows[0].clone(), data().rows[2].clone()]
        );
        selection.row_clicked(2, egui::Modifiers::CTRL);
        assert!(!selection.contains(2, 1));
        assert_eq!(selection.snapshot(&data()).unwrap().rows.len(), 1);
    }
    #[test]
    fn shift_rows_use_anchor_and_ctrl_shift_adds_ranges_without_dropping_rows() {
        let mut selection = GridSelection::default();
        selection.row_clicked(2, egui::Modifiers::NONE);
        selection.row_clicked(1, egui::Modifiers::SHIFT);
        assert_eq!(selection.snapshot(&data()).unwrap().rows.len(), 2);
        selection.row_clicked(0, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        assert_eq!(selection.snapshot(&data()).unwrap().rows, data().rows);
    }
    #[test]
    fn sparse_cells_reject_holes_and_never_synthesize_null() {
        let mut selection = GridSelection::default();
        selection.clicked(0, 0, egui::Modifiers::NONE);
        selection.clicked(2, 2, egui::Modifiers::CTRL);
        assert!(selection.snapshot(&data()).unwrap_err().contains("lacunas"));
        selection.clicked(2, 2, egui::Modifiers::CTRL);
        assert_eq!(
            selection.snapshot(&data()).unwrap().rows,
            vec![vec![Cell::Number("1".into())]]
        );
        selection.clicked(0, 0, egui::Modifiers::CTRL);
        assert!(selection.is_empty());
        assert!(selection.snapshot(&data()).is_err());
    }
    #[test]
    fn switching_cell_and_row_modes_does_not_mix_incompatible_selections() {
        let mut selection = GridSelection::default();
        selection.clicked(1, 1, egui::Modifiers::NONE);
        selection.row_clicked(2, egui::Modifiers::CTRL);
        assert_eq!(
            selection.snapshot(&data()).unwrap().rows,
            vec![data().rows[2].clone()]
        );
        selection.clicked(0, 2, egui::Modifiers::SHIFT);
        assert!(!selection.has_rows());
        assert_eq!(
            selection.snapshot(&data()).unwrap().rows,
            vec![vec![Cell::Null]]
        );
    }
    #[test]
    fn changed_origin_or_revision_clears_selection_and_stale_bounds_are_rejected() {
        let mut selection = GridSelection::default();
        assert!(selection.sync_revision("one", Some(1)));
        selection.clicked(0, 0, egui::Modifiers::NONE);
        assert!(!selection.sync_revision("one", Some(1)));
        assert!(!selection.is_empty());
        assert!(selection.sync_revision("two", Some(1)));
        assert!(selection.is_empty());
        selection.row_clicked(0, egui::Modifiers::NONE);
        assert!(selection.sync_revision("two", Some(2)));
        assert!(selection.is_empty());
        selection.clicked(usize::MAX, usize::MAX, egui::Modifiers::NONE);
        assert!(selection.snapshot(&data()).unwrap_err().contains("atual"));
        selection.clear();
        selection.clicked(0, 0, egui::Modifiers::NONE);
        selection.clicked(usize::MAX, usize::MAX, egui::Modifiers::SHIFT);
        assert!(selection.snapshot(&data()).is_err());
    }
    #[test]
    fn ctrl_shift_union_rejects_nonrectangular_holes_but_accepts_complete_rectangle() {
        let mut selection = GridSelection::default();
        selection.clicked(0, 0, egui::Modifiers::NONE);
        selection.clicked(0, 1, egui::Modifiers::CTRL);
        selection.clicked(1, 1, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        assert!(selection.snapshot(&data()).is_err());
        selection.clicked(1, 0, egui::Modifiers::CTRL);
        let selected = selection.snapshot(&data()).unwrap();
        assert_eq!(selected.rows.len(), 2);
        assert_eq!(selected.columns, vec!["id", "name"]);
    }
}
