//! Typed readiness shared by the UI intent queue and the actual workspace command.
use super::*;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceChangeBlocker {
    Busy(&'static str),
    UnsavedDocuments,
}
impl WorkspaceChangeBlocker {
    pub fn message(self) -> &'static str {
        match self {
            Self::Busy(message) => message,
            Self::UnsavedDocuments => "Salve os arquivos alterados antes de abrir outro projeto",
        }
    }
}
impl AppService {
    pub fn workspace_change_blocker(&self, path: &Path) -> Option<WorkspaceChangeBlocker> {
        use WorkspaceChangeBlocker::{Busy, UnsavedDocuments};
        let changing = self.workspace_path() != Some(path);
        if self.encoding_pending() || self.rename_stage.is_some() || self.project_files.pending {
            Some(Busy("Aguardando a operação de arquivos terminar"))
        } else if self.remote_db.console_delete_pending() || self.remote_db.console_rename_pending()
        {
            Some(Busy("Aguardando a gravação das consultas"))
        } else if self.pending.values().any(|p| {
            matches!(
                p,
                Pending::Save(_) | Pending::Persist | Pending::Restore | Pending::Workspace
            )
        }) {
            Some(Busy("Aguardando a gravação ou abertura de arquivos"))
        } else if changing && self.debugger_has_sessions() {
            Some(Busy("Encerre as sessões de debug para abrir outro projeto"))
        } else if self.db.is_busy() {
            Some(Busy("Aguardando a operação de banco de dados terminar"))
        } else if changing && self.remote_db.transaction {
            Some(Busy(
                "Faça Commit ou Rollback no banco antes de abrir outro projeto",
            ))
        } else if self.remote_db.has_pending_operation() {
            Some(Busy("Aguardando a operação de banco de dados terminar"))
        } else if self.language.is_busy() || self.project_language_busy() {
            Some(Busy("Aguardando a análise do editor terminar"))
        } else if self.git_repositories_busy() {
            Some(Busy("Aguardando a operação de Git terminar"))
        } else if self.pending.len() >= QUEUE_SIZE {
            Some(Busy("Aguardando a fila de arquivos"))
        } else if changing && self.documents.iter().any(Document::is_dirty) {
            Some(UnsavedDocuments)
        } else {
            None
        }
    }
    pub fn workspace_open_pending(&self) -> bool {
        self.pending.values().any(|p| *p == Pending::Workspace)
    }
    /// Invalidate the accepted workspace read. Committed writes still own their ACK.
    pub fn cancel_workspace_open(&mut self) {
        if self.workspace_open_pending() {
            self.generation.fetch_add(1, Ordering::SeqCst);
            self.status = "Abertura de projeto cancelada".into();
        }
    }
    /// Lifecycle pause only; the saved automatic-start preference is unchanged.
    pub fn suspend_provider_startup(&mut self) {
        self.automatic_providers.suspend();
    }
    /// Cancel read-only editor requests without invalidating any accepted file write.
    /// File transactions retain their own ACK and cancellation rules.
    pub fn cancel_workspace_reads(&mut self) -> Result<(), String> {
        if self.rename_stage.is_some() || self.encoding_pending() || self.project_files.pending {
            return Err("Aguardando a operação de arquivos terminar".into());
        }
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.language.dispatch(LanguageCommand::CancelHover)
    }
}
