//! Native-form text rasterization over TextEdit's real selection, IME and undo.
//! ASCII names use the same measured advances for hit testing and LCD paint.
//! Complex text/IME and hosts without FreeType retain the normal egui path.
use super::*;
use crate::signature_paint::{Run, SignaturePaint};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct NameInput {
    paint: SignaturePaint,
    ime: bool,
}
fn run(text: &str, tokens: &Tokens) -> Run {
    Run {
        text: text.into(),
        color: color(tokens.rename_input_text),
        bold: false,
        clip: false,
        padding: [0., 0.],
    }
}
impl NameInput {
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        name: &mut String,
        field: egui::Rect,
        frame: egui::Frame,
        tokens: &Tokens,
    ) -> egui::Response {
        for event in ui.input(|i| i.events.clone()) {
            if let egui::Event::Ime(event) = event {
                match event {
                    egui::ImeEvent::Preedit { text, .. } => self.ime = !text.is_empty(),
                    egui::ImeEvent::Commit(_) => self.ime = false,
                    _ => {}
                }
            }
        }
        let enabled = ui.is_enabled()
            && !self.ime
            && ui.painter().opacity() == 1.
            && ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .is_none_or(|t| t == egui::emath::TSTransform::IDENTITY);
        let scale = ui.ctx().pixels_per_point();
        let lcd_caret = enabled
            && !name.is_empty()
            && name.is_ascii()
            && name.len() <= 128
            && self
                .paint
                .measure(&[run(name, tokens)], tokens.ui_font_size, scale)
                .is_some_and(|width| {
                    ((width * scale).ceil() + 4.) * (26. * scale).ceil() <= 65_536.
                });
        let mut custom = false;
        let mut layout = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _: f32| {
            let text = buffer.as_str();
            let mut job = egui::text::LayoutJob::simple_singleline(
                text.into(),
                FontId::proportional(tokens.ui_font_size),
                color(tokens.rename_input_text),
            );
            job.sections[0].format.line_height = Some(17.);
            let base = ui.fonts_mut(|f| f.layout_job(job));
            custom = false;
            if !enabled
                || text.is_empty()
                || !text.is_ascii()
                || text.len() > 128
                || base.rows.len() != 1
            {
                return base;
            }
            let scale = ui.ctx().pixels_per_point();
            let mut advances = Vec::with_capacity(text.len());
            for byte in text.bytes() {
                let Some(width) = self.paint.measure(
                    &[run(&(byte as char).to_string(), tokens)],
                    tokens.ui_font_size,
                    scale,
                ) else {
                    return base;
                };
                advances.push(width);
            }
            // Match the rasterizer's bounded texture allocation. Long names or
            // large scales retain egui's shaped text instead of a blank field.
            let total: f32 = advances.iter().sum();
            if ((total * scale).ceil() + 4.) * (26. * scale).ceil() > 65_536. {
                return base;
            }
            let mut galley = (*base).clone();
            let row = Arc::make_mut(&mut galley.rows[0].row);
            if row.glyphs.len() != advances.len() {
                return base;
            }
            let mut x = 0.;
            for (glyph, width) in row.glyphs.iter_mut().zip(advances) {
                glyph.pos.x = x;
                glyph.advance_width = width;
                glyph.first_vertex = 0;
                x += width;
            }
            row.size.x = x;
            row.visuals = Default::default();
            galley.rect.max.x = x;
            galley.mesh_bounds = egui::Rect::NOTHING;
            galley.num_vertices = 0;
            galley.num_indices = 0;
            custom = true;
            Arc::new(galley)
        };
        let output = ui
            .scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                ui.visuals_mut().text_cursor.stroke = if lcd_caret {
                    egui::Stroke::NONE
                } else {
                    egui::Stroke::new(1. / scale, color(tokens.rename_input_text))
                };
                egui::TextEdit::singleline(name)
                    .frame(frame)
                    .id(egui::Id::unique("codisseu.rename.name"))
                    .desired_width(field.width())
                    .min_size(field.size())
                    .layouter(&mut layout)
                    .show(ui)
            })
            .inner;
        if custom {
            let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(field));
            clipped.set_clip_rect(output.text_clip_rect.intersect(ui.clip_rect()));
            let baseline = output.galley_pos + egui::vec2(0., 13.);
            let width = output.galley.size().x + 4.;
            let runs = [run(name, tokens)];
            self.paint.paint(
                &clipped,
                &runs,
                baseline,
                width,
                color(tokens.panel),
                tokens.ui_font_size,
            );
            if output.response.has_focus()
                && let Some(range) = output.cursor_range
            {
                let [start, end] = range.sorted_cursors();
                let x = |cursor| output.galley.pos_from_cursor(cursor).min.x + output.galley_pos.x;
                if !range.is_empty() {
                    let selection = egui::Rect::from_min_max(
                        egui::pos2(x(start), output.galley_pos.y),
                        egui::pos2(x(end), output.galley_pos.y + 17.),
                    );
                    clipped.painter().rect_filled(
                        selection,
                        0.,
                        color(tokens.rename_name_selection),
                    );
                    clipped.set_clip_rect(
                        selection
                            .intersect(output.text_clip_rect)
                            .intersect(ui.clip_rect()),
                    );
                    self.paint.paint(
                        &clipped,
                        &runs,
                        baseline,
                        width,
                        color(tokens.rename_name_selection),
                        tokens.ui_font_size,
                    );
                }
                // The opaque LCD texture covers the caret. Draw one neutral
                // pixel for a collapsed selection, never egui's second blue caret.
                clipped.set_clip_rect(output.text_clip_rect.intersect(ui.clip_rect()));
                if range.is_empty() && ui.input(|i| i.focused) {
                    let cursor = egui::Rect::from_min_size(
                        egui::pos2(x(range.primary), output.galley_pos.y),
                        egui::vec2(1. / ui.ctx().pixels_per_point(), 17.),
                    );
                    clipped
                        .painter()
                        .rect_filled(cursor, 0., color(tokens.rename_input_text));
                }
            }
        }
        output.response.response
    }
}

pub(super) fn button(
    ui: &mut egui::Ui,
    input: &mut NameInput,
    tokens: &Tokens,
    label: &str,
    rect: egui::Rect,
    primary: bool,
    enabled: bool,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if !enabled {
        child.disable();
    }
    let response = child.put(
        rect,
        egui::Button::new(RichText::new(label).color(Color32::TRANSPARENT))
            .frame(false)
            .min_size(rect.size()),
    );
    let fill = color(if primary {
        tokens.rename_primary
    } else if response.hovered() {
        tokens.border
    } else {
        tokens.rename_button_fill
    });
    child
        .painter()
        .rect_filled(rect, tokens.control_corner_radius, fill);
    if !primary || response.has_focus() {
        child.painter().rect_stroke(
            rect,
            tokens.control_corner_radius,
            egui::Stroke::new(
                1.,
                color(if response.has_focus() {
                    tokens.accent
                } else {
                    tokens.rename_button_border
                }),
            ),
            egui::StrokeKind::Inside,
        );
    }
    child.set_clip_rect(rect.shrink(1.).intersect(ui.clip_rect()));
    let foreground = if primary {
        Color32::WHITE
    } else {
        color(tokens.rename_text)
    };
    let runs = [Run {
        text: label.into(),
        color: foreground,
        bold: primary,
        clip: false,
        padding: [0., 0.],
    }];
    let width = input
        .paint
        .measure(&runs, tokens.ui_font_size, child.ctx().pixels_per_point());
    let painted = width.is_some_and(|width| {
        input.paint.paint(
            &child,
            &runs,
            egui::pos2(rect.center().x - width / 2., rect.center().y + 5.),
            width + 4.,
            fill,
            tokens.ui_font_size,
        )
    });
    if !painted {
        child.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(tokens.ui_font_size),
            foreground,
        );
    }
    response
}

pub(super) fn chevron(ui: &egui::Ui, center: egui::Pos2, color: Color32) {
    let points = [
        center + egui::vec2(-3.5, -1.5),
        center + egui::vec2(0., 2.),
        center + egui::vec2(3.5, -1.5),
    ];
    ui.painter()
        .line_segment([points[0], points[1]], egui::Stroke::new(1., color));
    ui.painter()
        .line_segment([points[1], points[2]], egui::Stroke::new(1., color));
}

pub(super) fn checkbox(
    ui: &mut egui::Ui,
    cache: &mut crate::tab_label::TabLabels,
    tokens: &Tokens,
    label: &str,
    rect: egui::Rect,
    checked: &mut bool,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.spacing_mut().icon_width = 16.;
    child.spacing_mut().icon_spacing = 8.;
    // Checkbox still owns accessibility, pointer and keyboard behavior; its
    // measured native icon is painted independently of label layout width.
    child.style_mut().visuals.widgets.inactive.bg_fill = Color32::TRANSPARENT;
    child.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    let widgets = &mut child.style_mut().visuals.widgets;
    for visual in [
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
    ] {
        visual.bg_fill = Color32::TRANSPARENT;
        visual.weak_bg_fill = Color32::TRANSPARENT;
        visual.bg_stroke = egui::Stroke::NONE;
        visual.fg_stroke = egui::Stroke::NONE;
    }
    let response = child.put(
        rect,
        egui::Checkbox::new(checked, RichText::new(label).color(Color32::TRANSPARENT)),
    );
    let box_rect = egui::Rect::from_min_size(rect.min + egui::vec2(1., 4.), egui::vec2(16., 16.));
    child.painter().rect_filled(
        box_rect,
        3.,
        color(if *checked {
            tokens.accent
        } else {
            tokens.panel
        }),
    );
    if *checked {
        let a = box_rect.min + egui::vec2(3.5, 8.5);
        let b = box_rect.min + egui::vec2(6.5, 11.5);
        let c = box_rect.min + egui::vec2(12.5, 4.5);
        child
            .painter()
            .line_segment([a, b], egui::Stroke::new(2.5, Color32::WHITE));
        child
            .painter()
            .line_segment([b, c], egui::Stroke::new(2.5, Color32::WHITE));
    } else {
        child.painter().rect_stroke(
            box_rect,
            3.,
            egui::Stroke::new(1., color(tokens.rename_checkbox_border)),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        child.painter().rect_stroke(
            box_rect.expand(2.),
            4.,
            egui::Stroke::new(1., color(tokens.accent)),
            egui::StrokeKind::Inside,
        );
    }
    source::paint_label(
        cache,
        &mut child,
        label,
        rect.left_center() + egui::vec2(26., 0.),
        rect,
        color(tokens.panel),
        tokens,
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    struct State {
        input: NameInput,
        text: String,
        focus: bool,
        tokens: Tokens,
    }
    #[test]
    fn lcd_field_preserves_selection_editing_and_unicode_fallback() {
        let mut harness = Harness::builder().build_ui_state(
            |ui, state: &mut State| {
                let label = ui.label("Nome");
                let field = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(300., 28.));
                let response = state
                    .input
                    .show(
                        ui,
                        &mut state.text,
                        field,
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(9, 5)),
                        &state.tokens,
                    )
                    .labelled_by(label.id);
                if state.focus {
                    response.request_focus();
                    state.focus = false;
                }
            },
            State {
                input: NameInput::default(),
                text: "rainfall".into(),
                focus: true,
                tokens: Tokens::load(),
            },
        );
        harness.step();
        let tokens = Tokens::load();
        let width = harness
            .state_mut()
            .input
            .paint
            .measure(&[run("rain", &tokens)], tokens.ui_font_size, 1.)
            .unwrap();
        let point = harness.get_by_label("Nome").rect().left_top() + egui::vec2(9. + width, 14.);
        harness.hover_at(point);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        harness.step();
        harness.event(egui::Event::Text("X".into()));
        harness.step();
        assert_eq!(harness.state().text, "rainXfall");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.event(egui::Event::Text("precipitation".into()));
        harness.step();
        assert_eq!(harness.state().text, "precipitation");
        harness.key_press(Key::ArrowLeft);
        harness.key_press_modifiers(Modifiers::SHIFT, Key::ArrowLeft);
        harness.event(egui::Event::Text("X".into()));
        harness.step();
        assert_eq!(harness.state().text, "precipitatiXn");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.event(egui::Event::Text("chuva_é".into()));
        harness.step();
        assert_eq!(harness.state().text, "chuva_é");
        harness.key_press(Key::Backspace);
        harness.step();
        assert_eq!(harness.state().text, "chuva_");
        harness.key_press_modifiers(Modifiers::COMMAND, Key::A);
        harness.key_press(Key::Backspace);
        harness.step();
        assert_eq!(harness.state().text, "");
        harness.event(egui::Event::Text("x".repeat(140)));
        harness.step();
        assert_eq!(harness.state().text, "x".repeat(140));
    }
    #[test]
    fn painted_button_retains_keyboard_activation_and_disabled_state() {
        struct State {
            input: NameInput,
            tokens: Tokens,
            clicks: usize,
            enabled: bool,
        }
        let mut harness = Harness::builder().build_ui_state(
            |ui, state: &mut State| {
                let rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(80., 28.));
                let response = button(
                    ui,
                    &mut state.input,
                    &state.tokens,
                    "Preview",
                    rect,
                    true,
                    state.enabled,
                );
                response.request_focus();
                if response.clicked() {
                    state.clicks += 1;
                }
            },
            State {
                input: Default::default(),
                tokens: Tokens::load(),
                clicks: 0,
                enabled: true,
            },
        );
        harness.step();
        assert!(harness.get_by_label("Preview").is_focused());
        harness.key_press(Key::Enter);
        harness.step();
        assert_eq!(harness.state().clicks, 1);
        harness.state_mut().enabled = false;
        harness.step();
        harness.key_press(Key::Space);
        harness.step();
        assert_eq!(harness.state().clicks, 1);
    }
}
