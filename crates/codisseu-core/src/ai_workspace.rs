//! Resolve the AI working directory from declared folders, never the carrier
//! directory of a .code-workspace file. Runtime conversations retain their
//! original binding when another folder becomes visible.
use std::path::Path;

impl super::AppService {
    pub fn ai_workspace(&self) -> Option<&Path> {
        self.active_document()
            .and_then(|document| self.workspace_root_for(&document.path))
            .or_else(|| self.workspace_default_root())
    }
}
