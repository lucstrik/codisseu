//! Bounded read-only rows use the same shaping as the editable document.
use super::*;

pub(super) fn accessible_label(
    cache: &mut crate::tab_label::TabLabels,
    ui: &mut egui::Ui,
    text: &str,
    rect: egui::Rect,
    tokens: &Tokens,
) -> egui::Response {
    let response = ui.put(
        rect,
        egui::Label::new(RichText::new(text).color(Color32::TRANSPARENT)),
    );
    paint_label(
        cache,
        ui,
        text,
        rect.left_center(),
        rect,
        color(tokens.panel),
        tokens,
    );
    response
}

pub(super) fn paint_label(
    cache: &mut crate::tab_label::TabLabels,
    ui: &mut egui::Ui,
    text: &str,
    anchor: egui::Pos2,
    clip: egui::Rect,
    background: Color32,
    tokens: &Tokens,
) {
    let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(clip));
    clipped.set_clip_rect(clip.intersect(ui.clip_rect()));
    if !cache.paint(
        &clipped,
        text,
        anchor,
        (clip.right() - anchor.x).max(0.),
        crate::tab_label::LabelStyle {
            foreground: color(tokens.rename_text),
            background,
            opacity: 1.,
            font_size: tokens.ui_font_size,
        },
    ) {
        clipped.painter().text(
            anchor,
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(tokens.ui_font_size),
            color(tokens.rename_text),
        );
    }
}

pub(super) struct SourceLine {
    pub galley: std::sync::Arc<egui::Galley>,
    positions: Vec<usize>,
}
impl SourceLine {
    pub fn new(
        ui: &egui::Ui,
        text: &str,
        extension: &str,
        tokens: &Tokens,
        ligatures: bool,
    ) -> Self {
        let (galley, positions) = editor::preview_layout(ui, text, extension, tokens, ligatures);
        Self { galley, positions }
    }
    pub fn x(&self, scalar: usize) -> Option<f32> {
        let column = *self.positions.get(scalar)?;
        Some(
            self.galley
                .pos_from_cursor(egui::text::CCursor::new(column))
                .min
                .x,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_shaping_maps_tabs_unicode_and_ligature_preferences() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let tokens = Tokens::load();
                let text = "\t🌧 e\u{301} rainfall()\r";
                for enabled in [true, false] {
                    let line = SourceLine::new(ui, text, "php", &tokens, enabled);
                    let range =
                        conflicts::conflict_symbol_columns(text, 0, Some((0, 7, 0, 15))).unwrap();
                    assert_eq!(range, 6..14);
                    assert_eq!(line.positions[1], 4);
                    assert!(line.x(range.end).unwrap() > line.x(range.start).unwrap());
                    assert_eq!(line.galley.job.text.as_str(), "    🌧 e\u{301} rainfall()");
                    assert_eq!(line.x(usize::MAX), None);
                }
            });
        });
        output.textures_delta.clear();
    }
}
