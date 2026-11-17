//! Git label rendering using the public Inter face and a per-application bounded cache.
use super::tab_label::{LabelStyle, TabLabels};
use egui::{Align2, Color32, FontId, Rect, Ui, UiBuilder, pos2};

pub(super) fn text(
    ui: &mut Ui,
    labels: &mut TabLabels,
    rect: Rect,
    text: &str,
    foreground: Color32,
    background: Color32,
) {
    if rect.width() <= 0. || rect.height() <= 0. || !ui.is_rect_visible(rect) {
        return;
    }
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    // The native LCD adapter reserves two points for the filter's left fringe.
    // Keep the first ink column aligned with the measured tree label inset.
    let anchor = pos2(rect.left() + 2., rect.center().y);
    if !labels.paint(
        &child,
        text,
        anchor,
        // Reserve the filter fringe in the raster buffer; the child clip still
        // prevents its opaque background from reaching any adjacent label.
        rect.width() + 2.,
        LabelStyle {
            foreground,
            background,
            opacity: 1.,
            font_size: 13.,
        },
    ) {
        child.painter().text(
            pos2(rect.left(), rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.),
            foreground,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_label_keeps_lcd_rendering_with_a_tight_measured_advance() {
        let ctx = egui::Context::default();
        let mut labels = TabLabels::for_git();
        let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                text(
                    ui,
                    &mut labels,
                    Rect::from_min_size(pos2(40., 40.), egui::vec2(70., 24.)),
                    "greeting.js",
                    Color32::from_rgb(112, 174, 255),
                    Color32::from_rgb(42, 67, 113),
                );
            });
        });
        assert_eq!(
            labels.has_cached_label("greeting.js"),
            cfg!(codisseu_tab_lcd)
        );
        output.textures_delta.clear();
    }
}
