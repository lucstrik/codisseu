//! Internal Explorer moves share the validated filesystem worker with Cut/Paste.
use super::*;
use std::path::Path;

#[derive(Clone)]
struct DragItem {
    workspace: PathBuf,
    source: PathBuf,
}

impl CodisseuApp {
    fn project_drag_allowed(&self, path: &Path) -> bool {
        self.service.workspace_root_for(path).is_some_and(|root| {
            root != path
                && path
                    .strip_prefix(root)
                    .is_ok_and(|relative| !relative.components().any(|c| c.as_os_str() == ".git"))
        })
    }

    pub(super) fn project_drag_row(
        &mut self,
        ui: &egui::Ui,
        row: &egui::Response,
        path: &Path,
        folder: bool,
        source_allowed: bool,
    ) {
        let available = !self.service.project_files.pending
            && self.project_files_ui.input.is_none()
            && self.project_files_ui.trash.is_none();
        let multiple = self.project_files_ui.selection.len() > 1
            && self.project_files_ui.selection.contains(path);
        if multiple && row.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        }
        if available && source_allowed && !multiple && self.project_drag_allowed(path) {
            row.dnd_set_drag_payload(DragItem {
                workspace: self.service.workspace_path().unwrap().to_owned(),
                source: path.to_owned(),
            });
            if row.drag_started() {
                self.select_project_single(path.to_owned());
                self.project_files_ui.focus = Some(row.id);
                row.request_focus();
                self.focus_editor = false;
                self.project_files_ui.drag_hover = None;
            }
        }
        let Some(item) = row.dnd_hover_payload::<DragItem>() else {
            return;
        };
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            egui::DragAndDrop::clear_payload(ui.ctx());
            self.project_files_ui.drag_hover = None;
            return;
        }
        let parent = if folder {
            path
        } else {
            path.parent().unwrap_or(path)
        };
        let valid = available
            && self.service.workspace_path() == Some(item.workspace.as_path())
            && self.project_drag_allowed(&item.source)
            && self.service.workspace_root_for(&item.source)
                == self.service.workspace_root_for(parent)
            && !parent.starts_with(&item.source)
            && item.source.parent() != Some(parent)
            && !parent.components().any(|c| c.as_os_str() == ".git");
        ui.ctx().set_cursor_icon(if valid {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if valid {
            ui.painter().rect_stroke(
                row.rect.intersect(ui.clip_rect()),
                2.,
                egui::Stroke::new(1., ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
            let now = ui.input(|i| i.time);
            if self
                .project_files_ui
                .drag_hover
                .as_ref()
                .is_none_or(|(old, _)| old != parent)
            {
                self.project_files_ui.drag_hover = Some((parent.to_owned(), now));
            }
            if folder
                && self
                    .project_files_ui
                    .drag_hover
                    .as_ref()
                    .is_some_and(|(_, start)| now - start >= 0.65)
            {
                if path == item.workspace {
                    self.project_files_ui.root_collapsed = false;
                } else if self.expanded.insert(path.to_owned()) {
                    let _ = self
                        .service
                        .dispatch(Command::LoadDirectory(path.to_owned()));
                }
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
            egui::Area::new(egui::Id::unique("project.move.preview"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(pointer + egui::vec2(16., 16.))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(ui.spacing().tooltip_width);
                        ui.label(if valid {
                            format!("Mover para {}", parent.display())
                        } else {
                            "Destino indisponível".into()
                        });
                    });
                });
        }
        if let Some(item) = row.dnd_release_payload::<DragItem>() {
            self.project_files_ui.drag_hover = None;
            if valid
                && let Err(error) =
                    self.service
                        .dispatch(Command::ProjectFile(ProjectFileCommand::Transfer {
                            source: item.source.clone(),
                            parent: parent.to_owned(),
                            move_item: true,
                        }))
            {
                self.project_files_ui.error = Some(error);
            }
        }
    }

    pub(super) fn project_drag_scroll(&mut self, ui: &egui::Ui) {
        if egui::DragAndDrop::has_payload_of_type::<DragItem>(ui.ctx()) {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                egui::DragAndDrop::clear_payload(ui.ctx());
                self.project_files_ui.drag_hover = None;
                return;
            }
            if let Some(point) = ui.input(|i| i.pointer.hover_pos()) {
                let rect = ui.clip_rect();
                if rect.contains(point) {
                    let delta = if point.y < rect.top() + 24. {
                        12.
                    } else if point.y > rect.bottom() - 24. {
                        -12.
                    } else {
                        0.
                    };
                    if delta != 0. {
                        ui.scroll_with_delta(egui::vec2(0., delta));
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
    }
}
