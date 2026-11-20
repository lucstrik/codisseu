//! Drawing files use the shared document, worker, conflict and history services.
use super::*;

impl AppService {
    pub(super) fn poll_drawings(&mut self) -> bool {
        let mut changed = self.drawings.poll();
        if self.workspace_open_pending() || self.drawings.is_busy() {
            return changed;
        }
        let project = self.workspace_path().map(PathBuf::from);
        if self.drawings.project != project {
            if let Err(error) = self.drawings.set_project(project, self.workspace_roots()) {
                self.last_error = Some(error);
            }
            return true;
        }
        let paths = self.drawings.take_open();
        let mut remaining = MAX_DOCUMENTS.saturating_sub(self.documents.len());
        for path in paths {
            if self.documents.iter().any(|doc| doc.path == path) {
                continue;
            }
            if remaining == 0 {
                self.status = "Alguns desenhos não foram reabertos: limite de 32 documentos; use o painel Desenhos".into();
                break;
            }
            match self.dispatch(Command::OpenFile(path)) {
                Ok(()) => remaining -= 1,
                Err(error) => self.last_error = Some(error),
            }
            changed = true;
        }
        if self.drawings.error.is_none()
            && self.drawings.project.is_some()
            && let Some(path) = self.documents.iter().find_map(|doc| {
                (drawings::is_drawing(&doc.path)
                    && !self.drawings.paths.contains(&doc.path)
                    && self.workspace_root_for(&doc.path).is_some())
                .then(|| doc.path.clone())
            })
        {
            if let Err(error) = self.drawings.register(path) {
                self.last_error = Some(error);
            }
            changed = true;
        }
        changed
    }
}
