//! Explorer selection is independent of open documents and filesystem jobs.
use super::*;
use std::path::Path;

impl CodisseuApp {
    pub(super) fn project_row_click(
        &self,
        ui: &egui::Ui,
        row: &egui::Response,
    ) -> (Modifiers, Option<egui::Pos2>) {
        // Control release can arrive in the same frame as mouse release. Use
        // the click's snapshot rather than the modifiers of the later event.
        ui.input(|input| {
            input
                .events
                .iter()
                .rev()
                .find_map(|event| match event {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers,
                    } if row.rect.contains(*pos) => Some((*modifiers, Some(*pos))),
                    _ => None,
                })
                .unwrap_or((input.modifiers, row.interact_pointer_pos()))
        })
    }

    pub(super) fn project_disclosure_clicked(
        &self,
        row: &egui::Response,
        depth: usize,
        modifiers: Modifiers,
        pointer: Option<egui::Pos2>,
    ) -> bool {
        // Same inset/indentation as the shared Explorer row renderer. The
        // disclosure changes expansion/focus, never the selected paths.
        let left = row.rect.left() + 24. + depth as f32 * 16.;
        modifiers.is_none()
            && pointer.is_some_and(|point| point.x >= left - 4. && point.x < left + 12.)
    }
    pub(super) fn select_project_single(&mut self, path: PathBuf) {
        self.project_files_ui.selection.clear();
        self.project_files_ui.selection.insert(path.clone());
        self.project_files_ui.anchor = Some(path.clone());
        self.project_files_ui.range_base.clear();
        self.project_files_ui.selection_modified = false;
        self.project_files_ui.selected = Some(path);
    }

    pub(super) fn visible_project_entries(&self) -> Vec<usize> {
        let mut hidden_depth = None;
        self.service
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if hidden_depth.is_some_and(|depth| entry.depth > depth) {
                    return None;
                }
                hidden_depth =
                    (entry.is_dir && !self.expanded.contains(&entry.path)).then_some(entry.depth);
                Some(index)
            })
            .collect()
    }

    /// Returns true only for ordinary activation, never for selection modifiers.
    pub(super) fn select_project_click(&mut self, path: &Path, modifiers: Modifiers) -> bool {
        let additive = modifiers.ctrl || modifiers.command;
        if additive || modifiers.shift {
            self.project_files_ui.selection_modified = true;
        }
        if modifiers.shift {
            let indices = if self.project_files_ui.root_collapsed {
                vec![]
            } else {
                self.visible_project_entries()
            };
            let root = self.service.workspace.as_deref();
            let paths: Vec<_> = root
                .into_iter()
                .chain(
                    indices
                        .iter()
                        .map(|&i| self.service.entries[i].path.as_path()),
                )
                .collect();
            let at = paths.iter().position(|p| *p == path);
            let anchor = self
                .project_files_ui
                .anchor
                .as_deref()
                .and_then(|anchor| paths.iter().position(|p| *p == anchor));
            let range: Vec<_> = match (anchor, at) {
                (Some(a), Some(b)) => paths[a.min(b)..=a.max(b)]
                    .iter()
                    .map(|p| p.to_path_buf())
                    .collect(),
                _ => vec![path.to_owned()],
            };
            if !additive {
                self.project_files_ui.selection = self.project_files_ui.range_base.clone();
            }
            self.project_files_ui.selection.extend(range);
            if anchor.is_none() {
                self.project_files_ui.anchor = Some(path.to_owned());
            }
        } else if additive {
            if !self.project_files_ui.selection.remove(path) {
                self.project_files_ui.selection.insert(path.to_owned());
            }
            self.project_files_ui.anchor = Some(path.to_owned());
            self.project_files_ui.range_base = self.project_files_ui.selection.clone();
            self.project_files_ui.range_base.remove(path);
        } else {
            self.select_project_single(path.to_owned());
        }
        self.project_files_ui.selected = Some(path.to_owned());
        !additive && !modifiers.shift
    }

    pub(super) fn select_project_context(&mut self, path: &Path, row: &egui::Response) {
        if !self.project_files_ui.selection.contains(path) {
            self.select_project_single(path.to_owned());
        }
        self.project_files_ui.selected = Some(path.to_owned());
        self.project_files_ui.focus = Some(row.id);
        row.request_focus();
        self.focus_editor = false;
    }

    pub(super) fn reconcile_project_selection(&mut self) {
        let revision = (
            self.service.tree_snapshot_revision(),
            self.service.project_files.revision,
        );
        if self.project_files_ui.selection_revision == revision {
            return;
        }
        self.project_files_ui.selection_revision = revision;
        let root = self.service.workspace.as_ref();
        if self.project_files_ui.selection.is_empty() && self.project_files_ui.anchor.is_none() {
            return;
        }
        let entries: HashSet<_> = self
            .service
            .entries
            .iter()
            .map(|entry| &entry.path)
            .collect();
        self.project_files_ui
            .selection
            .retain(|path| root == Some(path) || entries.contains(path));
        self.project_files_ui
            .range_base
            .retain(|path| root == Some(path) || entries.contains(path));
        if self
            .project_files_ui
            .anchor
            .as_ref()
            .is_some_and(|path| root != Some(path) && !entries.contains(path))
        {
            self.project_files_ui.anchor = None;
        }
    }

    pub(super) fn begin_project_trash(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        if paths.iter().any(|path| {
            self.service
                .workspace_root_for(path)
                .is_none_or(|root| root == path)
                || path.components().any(|c| c.as_os_str() == ".git")
        }) {
            self.project_files_ui.error =
                Some("Raízes de projeto e metadados Git não podem ser excluídos.".into());
            return;
        }
        // A selected parent includes its descendants. Validate all selected
        // paths before reducing the review targets; do not hide invalid items.
        let requested: std::collections::BTreeSet<_> = paths.into_iter().collect();
        let paths = requested
            .iter()
            .filter(|path| {
                !path
                    .ancestors()
                    .skip(1)
                    .any(|parent| requested.contains(parent))
            })
            .cloned()
            .collect();
        self.focus_editor = false;
        self.project_files_ui.trash = Some(paths);
        self.project_files_ui.trash_focus = true;
    }
}
