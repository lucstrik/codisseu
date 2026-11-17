//! Bounded, virtualized tool output. The complete payload remains copyable.
use super::*;

#[derive(Default)]
pub(super) struct ToolCards {
    entries: HashMap<String, ToolDetail>,
}
struct ToolDetail {
    text: String,
    lines: Vec<std::ops::Range<usize>>,
}
impl ToolCards {
    pub(super) fn retain(&mut self, entries: &[ai::Entry]) {
        self.entries
            .retain(|id, _| entries.iter().any(|e| &e.id == id));
    }
    pub(super) fn show(&mut self, ui: &mut egui::Ui, entry: &ai::Entry) {
        let title: String = entry
            .text
            .lines()
            .next()
            .unwrap_or("Comando")
            .chars()
            .take(120)
            .collect();
        let cache = self
            .entries
            .entry(entry.id.clone())
            .or_insert_with(|| ToolDetail {
                text: String::new(),
                lines: Vec::new(),
            });
        if cache.text != entry.detail {
            cache.text = entry.detail.clone();
            cache.lines.clear();
            let mut offset = 0;
            for line in cache.text.split_inclusive('\n') {
                cache
                    .lines
                    .push(offset..offset + line.trim_end_matches(['\r', '\n']).len());
                offset += line.len();
            }
        }
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        egui::CollapsingHeader::new(&title)
            .id_salt(("tool", &entry.id))
            .show(ui, |ui| {
                if ui.small_button("Copiar detalhes completos").clicked() {
                    ui.ctx()
                        .copy_text(format!("{}\n{}", entry.text, cache.text));
                }
                let font = egui::FontId::monospace(12.);
                let row_height = ui.fonts_mut(|f| f.row_height(&font));
                let output = egui::ScrollArea::both()
                    .id_salt(("output", &entry.id))
                    .max_height(220.)
                    .auto_shrink([false, true])
                    .codisseu()
                    .show_rows(ui, row_height, cache.lines.len(), |ui, rows| {
                        for row in rows {
                            let text = &cache.text[cache.lines[row].clone()];
                            let end = text
                                .char_indices()
                                .nth(4096)
                                .map_or(text.len(), |(at, _)| at);
                            let preview = if end < text.len() {
                                format!(
                                    "{} … [linha longa; copie os detalhes completos]",
                                    &text[..end]
                                )
                            } else {
                                text.to_owned()
                            };
                            ui.add(
                                egui::Label::new(RichText::new(preview).font(font.clone()))
                                    .selectable(true)
                                    .extend(),
                            );
                        }
                    });
                #[cfg(test)]
                ui.ctx().data_mut(|d| {
                    d.insert_temp(
                        egui::Id::unique(("codisseu.ai.test.tool", &entry.id)),
                        (output.inner_rect, output.state.clamped_offset()),
                    )
                });
                #[cfg(not(test))]
                let _ = output;
            });
    }
}
