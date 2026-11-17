//! Query file receipts reconcile shared documents after filesystem commit.
use super::*;
impl AppService {
    pub(super) fn delete_database_console(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.pending.is_empty() || self.project_files.pending || self.rename.is_pending() {
            return Err("Aguarde as operações de arquivos antes de excluir a query".into());
        }
        if self
            .documents
            .iter()
            .any(|doc| doc.path == path && doc.is_dirty())
        {
            return Err("Salve ou descarte as alterações da query antes de excluir".into());
        }
        self.remote_db
            .dispatch(remote::RemoteCommand::DeleteConsole(path))
    }
    pub(super) fn finish_database_console_delete(&mut self, path: PathBuf) {
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.clear_search();
        if let Err(error) = self.close_language_document(&path) {
            self.last_error = Some(error);
        }
        self.language_versions.remove(&path);
        self.language_retries.remove(&path);
        self.language_skipped_versions.remove(&path);
        self.documents.retain(|doc| doc.path != path);
        if !self.documents.iter().any(|doc| Some(doc.id) == self.active) {
            self.active = self.documents.last().map(|doc| doc.id);
        }
        self.sync_views();
        self.status = "Query excluída".into();
    }

    pub(super) fn rename_database_console(
        &mut self,
        path: PathBuf,
        name: String,
    ) -> Result<(), String> {
        if !self.pending.is_empty() || self.project_files.pending || self.rename.is_pending() {
            return Err(
                "Aguarde a gravação ou abertura de arquivos antes de renomear a query".into(),
            );
        }
        let target = self.remote_db.console_rename_target(&path, &name)?;
        if target != path && self.documents.iter().any(|doc| doc.path == target) {
            return Err("Já existe um documento aberto com esse nome".into());
        }
        self.remote_db
            .dispatch(remote::RemoteCommand::RenameConsole { path, name })
    }
    pub(super) fn finish_database_console_rename(&mut self, receipt: remote::ConsoleRename) {
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.clear_search();
        if let Err(error) = self.close_language_document(&receipt.source) {
            self.last_error = Some(error);
        }
        self.language_versions.remove(&receipt.source);
        self.language_retries.remove(&receipt.source);
        self.language_skipped_versions.remove(&receipt.source);
        for doc in &mut self.documents {
            if doc.path == receipt.source {
                doc.path = receipt.path.clone();
            }
        }
        self.status = receipt.warning.unwrap_or_else(|| "Query renomeada".into());
    }
}
