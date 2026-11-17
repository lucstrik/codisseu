//! Shared appearance and native wheel-axis policy for existing egui scroll areas.
use egui::{Color32, Context, Id, Rect, Style, Ui, scroll_area::ScrollAreaOutput};
use std::{ops::Range, sync::Arc};

fn tint_key() -> Id {
    Id::unique("codisseu.scrollbar.tint")
}

pub(super) fn configure(ctx: &Context, style: &mut Style, tint: Color32) {
    ctx.data_mut(|data| data.insert_temp(tint_key(), tint));
    appearance(&mut style.spacing.scroll);
}

fn appearance(scroll: &mut egui::style::ScrollStyle) {
    // Keep space already reserved by solid bars (notably the code viewport).
    let allocated = scroll.allocated_width();
    scroll.floating = true;
    scroll.bar_width = 4.;
    scroll.floating_width = 2.;
    scroll.floating_allocated_width = allocated;
    scroll.bar_inner_margin = 0.;
    scroll.bar_outer_margin = 0.;
    scroll.foreground_color = false;
    scroll.dormant_background_opacity = 0.;
    scroll.active_background_opacity = 0.;
    scroll.interact_background_opacity = 0.;
    scroll.dormant_handle_opacity = 0.35;
    scroll.active_handle_opacity = 0.55;
    scroll.interact_handle_opacity = 0.9;
}

fn paint_scope<R>(
    ui: &mut Ui,
    horizontal_wheel: bool,
    show: impl FnOnce(&mut Ui, Arc<Style>) -> R,
) -> R {
    let original = Arc::clone(ui.style());
    let tint = ui
        .ctx()
        .data(|data| data.get_temp::<Color32>(tint_key()))
        .unwrap_or(Color32::from_gray(68));
    let style = ui.style_mut();
    if horizontal_wheel {
        // Let egui route the ordinary wheel into this horizontal strip. Content
        // keeps its original style, so nested/neighboring scroll areas are untouched.
        style.always_scroll_the_only_direction = true;
    }
    appearance(&mut style.spacing.scroll);
    for state in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
    ] {
        state.bg_fill = tint;
        state.corner_radius = egui::CornerRadius::same(2);
    }
    // No extra Ui/scope: preserve every existing scroll and child widget ID.
    let output = show(ui, Arc::clone(&original));
    ui.set_style(original);
    output
}

pub(super) trait ScrollAreaExt {
    fn codisseu(self) -> ThemedScrollArea;
}
impl ScrollAreaExt for egui::ScrollArea {
    fn codisseu(self) -> ThemedScrollArea {
        ThemedScrollArea(self, false)
    }
}

pub(super) struct ThemedScrollArea(egui::ScrollArea, bool);
impl ThemedScrollArea {
    /// Opt in only for horizontal strips; egui keeps hover, bounds and consumption.
    pub(super) fn wheel_to_horizontal(mut self) -> Self {
        self.1 = true;
        self
    }

    pub(super) fn show<R>(
        self,
        ui: &mut Ui,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> ScrollAreaOutput<R> {
        paint_scope(ui, self.1, |ui, original| {
            self.0.show(ui, |content| {
                content.set_style(original);
                add(content)
            })
        })
    }
    pub(super) fn show_rows<R>(
        self,
        ui: &mut Ui,
        row_height: f32,
        total: usize,
        add: impl FnOnce(&mut Ui, Range<usize>) -> R,
    ) -> ScrollAreaOutput<R> {
        paint_scope(ui, self.1, |ui, original| {
            self.0.show_rows(ui, row_height, total, |content, rows| {
                content.set_style(original);
                add(content, rows)
            })
        })
    }
    pub(super) fn show_viewport<R>(
        self,
        ui: &mut Ui,
        add: impl FnOnce(&mut Ui, Rect) -> R,
    ) -> ScrollAreaOutput<R> {
        paint_scope(ui, self.1, |ui, original| {
            self.0.show_viewport(ui, |content, viewport| {
                content.set_style(original);
                add(content, viewport)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    #[derive(Default)]
    struct State {
        decorated: bool,
        id: Option<Id>,
        viewport: Option<Rect>,
        offset: egui::Vec2,
    }

    #[test]
    fn decoration_preserves_scroll_identity_offset_viewport_and_content_styles() {
        let mut h = Harness::builder().with_size([400., 300.]).build_ui_state(
            |ui, state: &mut State| {
                ui.style_mut().animation_time = 0.;
                ui.spacing_mut().scroll = egui::style::ScrollStyle {
                    bar_width: 8.,
                    bar_inner_margin: 0.,
                    ..egui::style::ScrollStyle::solid()
                };
                ui.visuals_mut().widgets.inactive.bg_fill = Color32::RED;
                ui.visuals_mut().widgets.hovered.bg_fill = Color32::GREEN;
                ui.visuals_mut().widgets.active.bg_fill = Color32::BLUE;
                let original = Arc::clone(ui.style());
                let mut area = egui::ScrollArea::both()
                    .id_salt("existing-scroll")
                    .max_height(180.);
                if !state.decorated {
                    area = area.scroll_offset(egui::vec2(70., 120.));
                }
                let add = |content: &mut Ui| {
                    assert_eq!(
                        content.style(),
                        &original,
                        "scroll paint must not recolor buttons or fields"
                    );
                    content.set_min_size(egui::vec2(900., 900.));
                    content.button("Existing control")
                };
                let output = if state.decorated {
                    area.codisseu().show(ui, add)
                } else {
                    area.show(ui, add)
                };
                assert_eq!(
                    ui.style(),
                    &original,
                    "caller style restored after the scroll area"
                );
                if state.decorated {
                    assert_eq!(Some(output.id), state.id, "no extra Ui/scroll identity");
                    assert_eq!(
                        output.state.clamped_offset(),
                        state.offset,
                        "existing position survives visual change"
                    );
                    assert_eq!(
                        Some(output.inner_rect),
                        state.viewport,
                        "reserved gutter stays unchanged"
                    );
                } else {
                    state.id = Some(output.id);
                    state.viewport = Some(output.inner_rect);
                    state.offset = output.state.clamped_offset();
                }
            },
            State::default(),
        );
        h.run();
        h.state_mut().decorated = true;
        h.run();
    }
}
