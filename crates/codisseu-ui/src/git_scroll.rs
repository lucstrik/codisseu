//! Retain Source Control's existing minimum drag-handle length.
use egui::Ui;

pub(super) fn scm_scroll_style(ui: &mut Ui) {
    ui.spacing_mut().scroll.handle_min_length = 24.;
}
