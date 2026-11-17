//! Merge-only tokens sampled from the user reference; geometry's 2x mapping is inferred.
//! Evidence: docs/git-merge-reference-20261104.md. Not a claim about local defaults.
use super::*;
#[derive(serde::Deserialize)]
pub(super) struct MergeStyle {
    pub chrome: [u8; 3],
    pub editor: [u8; 3],
    pub border: [u8; 3],
    pub button_border: [u8; 3],
    pub foreground: [u8; 3],
    pub muted: [u8; 3],
    pub line_number: [u8; 3],
    pub conflict: [u8; 3],
    pub conflict_word: [u8; 3],
    pub added: [u8; 3],
    pub modified: [u8; 3],
    pub accent: [u8; 3],
    pub link: [u8; 3],
    pub tooltip: [u8; 3],
    pub title_height: f32,
    pub toolbar_height: f32,
    pub header_height: f32,
    pub footer_height: f32,
    pub font_size: f32,
    pub line_height: f32,
    pub connector_width: f32,
    pub number_width: f32,
    pub actions_width: f32,
    pub code_padding_x: f32,
    pub code_padding_y: f32,
    pub tooltip_padding: i8,
}
impl MergeStyle {
    pub fn get() -> &'static Self {
        static STYLE: std::sync::OnceLock<MergeStyle> = std::sync::OnceLock::new();
        STYLE.get_or_init(|| {
            serde_json::from_str(include_str!("../../../design/git-merge-tokens.json"))
                .expect("audited merge tokens")
        })
    }
    pub fn apply(&self, ui: &mut egui::Ui) {
        ui.style_mut().visuals.override_text_color = Some(color(self.foreground));
        ui.style_mut().visuals.panel_fill = color(self.chrome);
        ui.style_mut().visuals.window_fill = color(self.chrome);
        ui.style_mut().visuals.extreme_bg_color = color(self.editor);
        ui.style_mut().visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        ui.style_mut().visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.style_mut().visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1., color(self.button_border));
        ui.style_mut().visuals.widgets.active.bg_fill = color(self.accent);
        ui.style_mut().visuals.widgets.hovered.bg_fill = color(self.border);
        ui.style_mut().visuals.hyperlink_color = color(self.link);
        ui.style_mut().visuals.window_shadow = egui::epaint::Shadow::NONE;
        ui.style_mut().override_font_id = Some(FontId::proportional(self.font_size));
        ui.spacing_mut().item_spacing = egui::vec2(8., 0.);
        ui.spacing_mut().interact_size.y = 24.;
        ui.spacing_mut().button_padding = egui::vec2(12., 4.);
    }
}
/// Flat toolbar action with public/original glyph and a semantic label.
pub(super) fn tool(
    ui: &mut egui::Ui,
    icon: Option<Icon>,
    label: &str,
    accessible: &str,
    enabled: bool,
) -> egui::Response {
    let style = MergeStyle::get();
    let label_width = if label.is_empty() {
        0.
    } else {
        ui.painter()
            .layout_no_wrap(
                label.into(),
                FontId::proportional(style.font_size),
                color(style.foreground),
            )
            .size()
            .x
    };
    let width = label_width + if icon.is_some() { 22. } else { 0. } + 8.;
    let (r, response) = ui.allocate_exact_size(
        egui::vec2(width.max(24.), 26.),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, enabled, accessible)
    });
    if enabled && response.hovered() {
        ui.painter().rect_filled(r, 3., color(style.border));
    }
    let tint = color(if enabled {
        style.foreground
    } else {
        style.line_number
    });
    if let Some(icon) = icon {
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                egui::pos2(r.left() + 11., r.center().y),
                egui::vec2(16., 16.),
            ),
            icon,
            tint,
        );
    }
    if !label.is_empty() {
        ui.painter().text(
            egui::pos2(
                r.left() + if icon.is_some() { 22. } else { 4. },
                r.center().y,
            ),
            egui::Align2::LEFT_CENTER,
            label,
            FontId::proportional(style.font_size),
            tint,
        );
    }
    response.on_hover_text(accessible)
}
pub(super) fn lock(ui: &egui::Ui, rect: egui::Rect) {
    let c = rect.center();
    let tint = color(MergeStyle::get().foreground);
    let s = egui::Stroke::new(1., tint);
    ui.painter().rect_stroke(
        egui::Rect::from_center_size(c + egui::vec2(0., 2.), egui::vec2(9., 7.)),
        2.,
        s,
        egui::StrokeKind::Inside,
    );
    ui.painter().add(egui::Shape::line(
        vec![
            c + egui::vec2(-3., -1.),
            c + egui::vec2(-3., -4.),
            c + egui::vec2(0., -5.),
            c + egui::vec2(3., -4.),
            c + egui::vec2(3., -1.),
        ],
        s,
    ));
}
