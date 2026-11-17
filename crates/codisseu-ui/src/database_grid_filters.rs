//! Typed, bounded filters over the loaded page; never a database request.
use super::*;
use codisseu_core::{remote::Cell, transfer::TransferData};
use std::collections::{BTreeMap, BTreeSet};

const MAX_KEYS: usize = 8 * 1024 * 1024;
pub(super) type Key = Vec<u8>;
pub(super) fn key(cell: &Cell) -> Key {
    let (kind, bytes): (u8, &[u8]) = match cell {
        Cell::Null => (0, &[]),
        Cell::Text(s) => (1, s.as_bytes()),
        Cell::Number(s) => (2, s.as_bytes()),
        Cell::Bool(false) => (3, &[0]),
        Cell::Bool(true) => (3, &[1]),
        Cell::Json(s) => (4, s.as_bytes()),
        Cell::Binary(b) => (5, b),
    };
    let mut key = Vec::with_capacity(bytes.len() + 1);
    key.push(kind);
    key.extend_from_slice(bytes);
    key
}

#[derive(Default)]
pub(super) struct LocalFilters {
    pub enabled: bool,
    pub selections: BTreeMap<usize, BTreeSet<Key>>,
    pub open: Option<usize>,
    anchor: egui::Pos2,
    values: Vec<(Key, String, usize)>,
    query: String,
    focus: bool,
    error: Option<String>,
}
pub(super) fn popup_frame(ui: &egui::Ui, tokens: &Tokens) -> egui::Frame {
    egui::Frame::popup(ui.style())
        .fill(color(tokens.bar))
        .stroke(egui::Stroke::new(1., color(tokens.popup_border)))
        .corner_radius(tokens.control_corner_radius)
        .inner_margin(tokens.navigation_popup_padding)
}
impl LocalFilters {
    pub fn active(&self) -> bool {
        self.enabled && !self.selections.is_empty()
    }
    pub fn close(&mut self) {
        self.open = None;
        self.values.clear();
        self.error = None;
    }
    pub fn clear(&mut self) {
        self.selections.clear();
        self.error = None;
    }
    pub fn toolbar(&mut self, ui: &mut egui::Ui, tokens: &Tokens) -> bool {
        let button =
            components::database_icon_button(ui, Icon::Filter, "Enable Local Filter", true);
        button.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::accesskit::Role::CheckBox,
                ui.is_enabled(),
                self.enabled,
                "Enable Local Filter",
            )
        });
        if self.enabled {
            ui.painter().rect_stroke(
                button.rect.shrink(2.),
                3.,
                egui::Stroke::new(1., color(tokens.accent)),
                egui::StrokeKind::Inside,
            );
        }
        if button.clicked() {
            self.enabled = !self.enabled;
            self.close();
            return true;
        }
        false
    }
    pub fn header(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        column: usize,
        name: &str,
        tokens: &Tokens,
    ) {
        if !self.enabled {
            return;
        }
        let rect = egui::Rect::from_center_size(
            egui::pos2(response.rect.right() - 29., response.rect.center().y),
            egui::vec2(18., 22.),
        );
        let filter = ui
            .interact(rect, response.id.with("local-filter"), egui::Sense::click())
            .on_hover_text(format!("Filter {name} · current page"));
        filter.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                format!("Filter column {}: {name}", column + 1),
            )
        });
        // Opaque tile reserves the icon from the header's text painter.
        ui.painter()
            .rect_filled(rect, 0., color(tokens.database_grid_header));
        components::paint_icon(
            ui,
            rect.shrink(2.),
            Icon::Filter,
            color(if self.selections.contains_key(&column) {
                tokens.accent
            } else {
                tokens.muted
            }),
        );
        if filter.clicked() {
            self.open = Some(column);
            self.anchor = response.rect.left_bottom();
            self.values.clear();
            self.query.clear();
            self.focus = true;
            self.error = None;
        }
    }
    /// A rejected choice leaves the previous filter intact.
    fn choose(&mut self, column: usize, selected: BTreeSet<Key>) -> bool {
        let bytes = self
            .selections
            .iter()
            .filter(|(c, _)| **c != column)
            .flat_map(|(_, v)| v)
            .chain(selected.iter())
            .map(Vec::len)
            .sum::<usize>();
        if bytes > MAX_KEYS {
            self.error =
                Some("Local filter values exceed 8 MiB; clear another column filter.".into());
            return false;
        }
        self.error = None;
        self.selections.insert(column, selected);
        true
    }
    pub fn popup(
        &mut self,
        ui: &mut egui::Ui,
        data: &TransferData,
        proposals: &BTreeMap<(usize, usize), Cell>,
        tokens: &Tokens,
    ) -> bool {
        let Some(column) = self.open else {
            return false;
        };
        let Some(name) = data.columns.get(column) else {
            self.close();
            return false;
        };
        if self.focus {
            let mut values: BTreeMap<Key, (String, usize)> = BTreeMap::new();
            for (row, cells) in data.rows.iter().enumerate() {
                if let Some(cell) = proposals.get(&(row, column)).or_else(|| cells.get(column)) {
                    let label = match cell {
                        Cell::Null => "<null>".into(),
                        Cell::Text(s) if s.is_empty() => "<empty text>".into(),
                        Cell::Text(_) => format!("{} · text", cell.display()),
                        Cell::Json(_) => format!("{} · JSON", cell.display()),
                        Cell::Bool(_) => format!("{} · boolean", cell.display()),
                        Cell::Binary(bytes) => format!(
                            "BINARY[{}] · {}{}",
                            bytes.len(),
                            Cell::Binary(bytes.iter().take(32).copied().collect()).clipboard_text(),
                            if bytes.len() > 32 { "…" } else { "" }
                        ),
                        _ => cell.display().replace(['\n', '\r', '\t'], " "),
                    };
                    let entry = values.entry(key(cell)).or_insert((label, 0));
                    entry.1 += 1;
                }
            }
            self.values = values.into_iter().map(|(k, (s, n))| (k, s, n)).collect();
        }
        let id = egui::Id::unique("database.local-filter");
        let mut close = false;
        let mut changed = false;
        let modal = egui::Modal::new(id)
            .area(
                egui::Area::new(id)
                    .kind(egui::UiKind::Modal)
                    .order(egui::Order::Foreground)
                    .fixed_pos(self.anchor),
            )
            .backdrop_color(egui::Color32::TRANSPARENT)
            .frame(popup_frame(ui, tokens))
            .show(ui.ctx(), |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                ui.spacing_mut().interact_size.y = tokens.row_height;
                ui.set_width(300_f32.min(ui.ctx().content_rect().width() - 48.));
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = components::database_icon_button(
                            ui,
                            Icon::Close,
                            "Close Column Filter",
                            true,
                        )
                        .clicked();
                    });
                });
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Search values")
                        .desired_width(f32::INFINITY)
                        .char_limit(512)
                        .align(egui::Align2::LEFT_CENTER),
                );
                field.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Search values",
                    )
                });
                if self.focus {
                    field.request_focus();
                    self.focus = false;
                }
                ui.horizontal(|ui| {
                    if ui.button("Select All Values").clicked() {
                        self.selections.remove(&column);
                        changed = true;
                    }
                    if ui.button("Clear Values").clicked() {
                        changed |= self.choose(column, BTreeSet::new());
                    }
                });
                let query = self.query.to_lowercase();
                let mut updates = Vec::new();
                egui::ScrollArea::vertical()
                    .id_salt("column-filter-values")
                    .max_height((ui.ctx().content_rect().height() - 210.).clamp(80., 280.))
                    .codisseu()
                    .show(ui, |ui| {
                        for (index, (key, label, count)) in self.values.iter().enumerate() {
                            if !label.to_lowercase().contains(&query) {
                                continue;
                            }
                            let mut checked =
                                self.selections.get(&column).is_none_or(|v| v.contains(key));
                            let text = format!("{label} ({count})");
                            let response = ui.add(egui::Checkbox::new(
                                &mut checked,
                                egui::RichText::new(&text).monospace(),
                            ));
                            response.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::accesskit::Role::CheckBox,
                                    true,
                                    checked,
                                    format!("Filter value {}: {text}", index + 1),
                                )
                            });
                            if response.changed() {
                                updates.push((key.clone(), checked));
                            }
                        }
                    });
                if !updates.is_empty() {
                    let mut selected =
                        self.selections.get(&column).cloned().unwrap_or_else(|| {
                            self.values.iter().map(|(k, _, _)| k.clone()).collect()
                        });
                    for (key, checked) in updates {
                        if checked {
                            selected.insert(key);
                        } else {
                            selected.remove(&key);
                        }
                    }
                    changed |= self.choose(column, selected);
                }
                ui.separator();
                ui.label(
                    egui::RichText::new("Current page only")
                        .small()
                        .color(color(tokens.muted)),
                );
                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(color(tokens.error)));
                }
            });
        if close || modal.should_close() {
            self.close();
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_preserve_types_full_payload_and_bounded_choices_fail_without_replacement() {
        let values = [
            Cell::Null,
            Cell::Text("NULL".into()),
            Cell::Text("".into()),
            Cell::Number("1".into()),
            Cell::Text("1".into()),
            Cell::Bool(true),
            Cell::Json("1".into()),
            Cell::Binary(vec![1]),
            Cell::Number("9223372036854775807".into()),
        ];
        assert_eq!(
            values.iter().map(key).collect::<BTreeSet<_>>().len(),
            values.len()
        );
        let a = Cell::Text("a".repeat(513));
        let b = Cell::Text(format!("{}b", "a".repeat(512)));
        assert_eq!(a.display(), b.display());
        assert_ne!(key(&a), key(&b));
        let mut filter = LocalFilters::default();
        assert!(filter.choose(1, BTreeSet::from([key(&Cell::Null)])));
        assert!(!filter.choose(1, BTreeSet::from([vec![7; MAX_KEYS + 1]])));
        assert_eq!(filter.selections[&1], BTreeSet::from([key(&Cell::Null)]));
        assert!(filter.error.is_some());
    }
}
