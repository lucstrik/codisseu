//! Transient tab gesture state. Only the drop commits group metadata.
use codisseu_core::{Command, DocumentId, ViewId};
use egui::{Color32, PointerButton, Rect, Response, Ui};

pub(super) struct TabDrag {
    pub view: ViewId,
    pub document: DocumentId,
    label: String,
}

pub(super) fn begin(
    drag: &mut Option<TabDrag>,
    view: ViewId,
    document: DocumentId,
    label: &str,
    response: &Response,
) {
    if response.drag_started_by(PointerButton::Primary) {
        *drag = Some(TabDrag {
            view,
            document,
            label: label.to_owned(),
        });
    }
}

pub(super) struct DragStyle {
    pub accent: Color32,
    pub background: Color32,
    pub foreground: Color32,
    pub font_size: f32,
}

pub(super) fn finish(
    ui: &mut Ui,
    drag: &mut Option<TabDrag>,
    view: ViewId,
    tabs: &[(DocumentId, Rect)],
    style: DragStyle,
    tools_follow: bool,
) -> Option<Command> {
    let DragStyle {
        accent,
        background,
        foreground,
        font_size,
    } = style;
    let gesture = drag.as_ref().filter(|d| d.view == view)?;
    let Some(source) = tabs.iter().position(|(id, _)| *id == gesture.document) else {
        *drag = None;
        return None;
    };
    let pointer = ui.input(|i| i.pointer.hover_pos());
    let released = ui.input(|i| i.pointer.button_released(PointerButton::Primary));
    let viewport = ui.clip_rect();
    // Restrict dropping to this group's document strip, including its trailing
    // whitespace. HTTP/browser tabs in the same row are separate tab models.
    let last = tabs.last()?.1;
    let zone = Rect::from_min_max(
        egui::pos2(viewport.left(), last.top()),
        egui::pos2(
            if tools_follow {
                last.right().min(viewport.right())
            } else {
                viewport.right()
            },
            last.bottom(),
        ),
    );
    let valid =
        pointer.filter(|p| viewport.width() >= 2. && zone.contains(*p) && viewport.contains(*p));
    let mut destination = None;
    if let Some(pointer) = valid {
        // The source is omitted when computing the insertion slot, which makes
        // moving in either direction and dropping on oneself unambiguous.
        let index = tabs
            .iter()
            .filter(|(id, rect)| *id != gesture.document && pointer.x >= rect.center().x)
            .count();
        destination = Some(index);
        let others = || tabs.iter().filter(|(id, _)| *id != gesture.document);
        let x = others().nth(index).map_or_else(
            || {
                others()
                    .next_back()
                    .map_or(tabs[source].1.left(), |(_, r)| r.right())
            },
            |(_, r)| r.left(),
        );
        ui.painter().vline(
            x.clamp(viewport.left() + 1., viewport.right() - 1.),
            (last.top() + 3.)..=(last.bottom() - 3.),
            egui::Stroke::new(2., accent),
        );
        // Time based and bounded, so a stationary pointer can reach hidden tabs
        // without a loop over frames or dependence on file size.
        if !released {
            let edge = 28_f32.min(viewport.width() * 0.25);
            let velocity = if pointer.x < viewport.left() + edge {
                (viewport.left() + edge - pointer.x) / edge
            } else if pointer.x > viewport.right() - edge {
                -(pointer.x - viewport.right() + edge) / edge
            } else {
                0.
            };
            if velocity != 0. {
                let dt = ui.input(|i| i.stable_dt).min(0.05);
                ui.scroll_with_delta_animation(
                    egui::vec2(velocity * 650. * dt, 0.),
                    egui::style::ScrollAnimation::none(),
                );
                ui.ctx().request_repaint();
            }
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::unique("codisseu.tab.drag.preview"),
        ));
        let label = painter.layout_no_wrap(
            gesture.label.clone(),
            egui::FontId::proportional(font_size),
            foreground,
        );
        let size = label.size() + egui::vec2(16., 10.);
        let screen = ui.ctx().content_rect();
        let pos = egui::pos2(
            (pointer.x + 12.).min(screen.right() - size.x),
            (pointer.y + 16.).min(screen.bottom() - size.y),
        );
        let rect = Rect::from_min_size(pos, size);
        painter.rect(
            rect,
            4.,
            background,
            egui::Stroke::new(1., accent),
            egui::StrokeKind::Inside,
        );
        painter.galley(pos + egui::vec2(8., 5.), label, foreground);
    } else {
        ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
    }
    if released {
        let gesture = drag.take()?;
        return destination
            .filter(|&index| index != source)
            .map(|index| Command::ReorderTab {
                view,
                id: gesture.document,
                index,
            });
    }
    None
}
