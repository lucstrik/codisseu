//! Explicit workspace file operations. Filesystem and system Trash run on the worker.
use super::*;
use std::path::{Component, Path};

#[derive(Clone, Debug)]
pub enum ProjectFileCommand {
    CreateFile {
        parent: PathBuf,
        name: String,
    },
    CreateFolder {
        parent: PathBuf,
        name: String,
    },
    Trash {
        path: PathBuf,
    },
    TrashMany {
        paths: Vec<PathBuf>,
    },
    RenameFile {
        path: PathBuf,
        name: String,
    },
    Transfer {
        source: PathBuf,
        parent: PathBuf,
        move_item: bool,
    },
    Refresh,
}
#[derive(Default)]
pub struct ProjectFileState {
    pub pending: bool,
    pub error: Option<String>,
    pub created: Option<PathBuf>,
    pub revision: u64,
    /// Paths actually acknowledged as moved, including successful parts of a batch.
    pub trashed: Vec<PathBuf>,
    locked_paths: Vec<PathBuf>,
}
impl ProjectFileState {
    pub(super) fn clear_lock(&mut self) {
        self.locked_paths.clear();
    }
}
#[derive(Debug)]
pub(super) enum FileResultKind {
    File,
    Folder,
    Trash,
    TrashedMany {
        paths: Vec<PathBuf>,
        failures: Vec<(PathBuf, String)>,
    },
    Refresh,
    Renamed {
        source: PathBuf,
    },
    Copied,
}
pub(super) struct FileReceipt {
    pub workspace: PathBuf,
    pub path: PathBuf,
    pub kind: FileResultKind,
}

impl AppService {
    pub(super) fn project_file_command(
        &mut self,
        command: ProjectFileCommand,
    ) -> Result<(), String> {
        if self.project_files.pending {
            return Err("Aguarde a operação de arquivos em andamento".into());
        }
        if self.pending.values().any(|pending| {
            matches!(
                pending,
                Pending::Save(_)
                    | Pending::Persist
                    | Pending::Workspace
                    | Pending::RenameResources
                    | Pending::Read
                    | Pending::GitRead(_)
                    | Pending::Restore
            )
        }) {
            return Err("Aguarde as gravações ou a abertura da pasta".into());
        }
        let workspace = self
            .workspace
            .clone()
            .ok_or("Abra uma pasta para gerenciar arquivos")?;
        if matches!(command, ProjectFileCommand::CreateFile { .. })
            && self.documents.len() >= MAX_DOCUMENTS
        {
            return Err("Feche uma aba antes de criar outro arquivo (limite32)".into());
        }
        let trash_paths: &[PathBuf] = match &command {
            ProjectFileCommand::Trash { path } => std::slice::from_ref(path),
            ProjectFileCommand::TrashMany { paths } => paths,
            _ => &[],
        };
        if matches!(command, ProjectFileCommand::TrashMany { .. })
            && (trash_paths.is_empty() || trash_paths.len() > 10_000)
        {
            return Err("Selecione de 1 a 10000 itens para mover para a Lixeira".into());
        }
        for path in trash_paths {
            if self
                .workspace_root_for(path)
                .is_none_or(|root| root == path)
            {
                return Err(
                    "Selecione um item dentro do projeto; a raiz não pode ser excluída".into(),
                );
            }
            if self
                .documents
                .iter()
                .any(|document| document.path.starts_with(path) && document.is_dirty())
            {
                return Err("Salve ou feche os arquivos com alterações não salvas antes de mover para a Lixeira".into());
            }
        }
        if let ProjectFileCommand::Transfer {
            source,
            move_item: false,
            ..
        } = &command
            && self
                .documents
                .iter()
                .any(|document| document.path.starts_with(source) && document.is_dirty())
        {
            return Err("Salve os arquivos alterados antes de copiar".into());
        }
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.clear_search();
        let locks = if let ProjectFileCommand::Trash { path }
        | ProjectFileCommand::RenameFile { path, .. }
        | ProjectFileCommand::Transfer { source: path, .. } = &command
        {
            vec![path.clone()]
        } else if let ProjectFileCommand::TrashMany { paths } = &command {
            paths.clone()
        } else {
            Vec::new()
        };
        self.enqueue(
            Work::ProjectFile {
                directories: self.project_tree.loaded.iter().cloned().collect(),
                workspace,
                command,
                open_paths: self
                    .documents
                    .iter()
                    .map(|document| document.path.clone())
                    .collect(),
            },
            Pending::ProjectFile,
        )?;
        self.project_files.pending = true;
        self.project_files.locked_paths = locks;
        self.project_files.trashed.clear();
        self.project_files.error = None;
        Ok(())
    }
    pub(super) fn guard_project_file_mutation(&self, command: &Command) -> Result<(), String> {
        if self.project_files.pending
            && matches!(
                command,
                Command::OpenFile(_) | Command::CreateFile(_) | Command::SaveUntitled { .. }
            )
        {
            return Err(
                "Aguarde a operação de arquivos antes de abrir ou criar outro arquivo".into(),
            );
        }
        if self.project_files.locked_paths.is_empty() {
            return Ok(());
        }
        let locked = |target: &Path| {
            self.project_files
                .locked_paths
                .iter()
                .any(|path| target.starts_with(path))
        };
        if matches!(command, Command::OpenFile(target) if locked(target)) {
            return Err("Aguarde a operação de arquivos".into());
        }
        let document = match command {
            Command::Edit { id, .. }
            | Command::EditDelta { id, .. }
            | Command::EditTransaction { id, .. }
            | Command::EditBatchTransaction { id, .. }
            | Command::Save { id }
            | Command::ReloadDocument { id, .. }
            | Command::OverwriteDocument { id, .. }
            | Command::ReplaceMatches { id, .. }
            | Command::RestoreLocalRevision { id, .. }
            | Command::Undo { id }
            | Command::Redo { id }
            | Command::Close { id, .. }
            | Command::CloseTab { id, .. } => {
                self.documents.iter().find(|document| document.id == *id)
            }
            _ => None,
        };
        if document.is_some_and(|document| locked(&document.path)) {
            return Err("Aguarde a conclusão do envio deste arquivo à Lixeira".into());
        }
        Ok(())
    }
    pub(super) fn accept_project_file(
        &mut self,
        receipt: FileReceipt,
        opened: Option<Result<OpenedFile, String>>,
        project_roots: Vec<(PathBuf, PathBuf)>,
        refresh: Result<Workspace, String>,
    ) -> Result<(), String> {
        self.project_files.pending = false;
        self.project_files.locked_paths.clear();
        self.project_files.revision += 1;
        if self.workspace.as_ref() != Some(&receipt.workspace) {
            self.status = "Operação de arquivos concluída no projeto anterior".into();
            return Ok(());
        }
        let (trashed, failures) = match &receipt.kind {
            FileResultKind::Trash => (vec![receipt.path.clone()], Vec::new()),
            FileResultKind::TrashedMany { paths, failures } => (paths.clone(), failures.clone()),
            _ => (Vec::new(), Vec::new()),
        };
        self.project_files.trashed = trashed.clone();
        if !trashed.is_empty() {
            let paths: std::collections::BTreeSet<_> = trashed.iter().cloned().collect();
            let removed = |path: &Path| path.ancestors().any(|ancestor| paths.contains(ancestor));
            let ids: Vec<_> = self
                .documents
                .iter()
                .filter(|document| removed(&document.path))
                .map(|document| (document.id, document.is_dirty()))
                .collect();
            for (id, dirty) in ids {
                if dirty {
                    if let Some(document) =
                        self.documents.iter_mut().find(|document| document.id == id)
                    {
                        document.disk_missing = true;
                    }
                } else {
                    self.execute(Command::Close { id, discard: false })?;
                }
            }
            self.entries.retain(|entry| !removed(&entry.path));
        }
        let status = match receipt.kind {
            FileResultKind::File => {
                self.project_files.created = Some(receipt.path.clone());
                match opened {
                    Some(Ok(file)) => self.apply(Completed::Open(file))?,
                    Some(Err(error)) => {
                        self.project_files.error = Some(error.clone());
                        self.last_error = Some(error);
                    }
                    None => {
                        self.project_files.error =
                            Some("Arquivo criado; abra-o novamente para editar".into());
                    }
                }
                "Arquivo criado; digite e salve com Ctrl+S"
            }
            FileResultKind::Folder => {
                self.project_files.created = Some(receipt.path.clone());
                "Pasta criada"
            }
            FileResultKind::Renamed { source } => {
                let paths: Vec<_> = self
                    .documents
                    .iter()
                    .filter(|document| document.path.starts_with(&source))
                    .map(|document| document.path.clone())
                    .collect();
                for old in paths {
                    self.close_language_document(&old)?;
                    self.language_versions.remove(&old);
                    self.language_retries.remove(&old);
                    self.language_skipped_versions.remove(&old);
                }
                for document in &mut self.documents {
                    if let Ok(relative) = document.path.strip_prefix(&source) {
                        document.path = if relative.as_os_str().is_empty() {
                            receipt.path.clone()
                        } else {
                            receipt.path.join(relative)
                        };
                    }
                }
                for document in &mut self.documents {
                    if let Some((_, root)) = project_roots
                        .iter()
                        .find(|(path, _)| *path == document.path)
                    {
                        document.project_root = root.clone();
                    }
                }
                self.project_files.created = Some(receipt.path.clone());
                "Arquivo renomeado; conteúdo e alterações preservados"
            }
            FileResultKind::Copied => {
                self.project_files.created = Some(receipt.path.clone());
                "Item copiado"
            }
            FileResultKind::Trash => "Item movido para a Lixeira",
            FileResultKind::TrashedMany { .. } => "Seleção processada para a Lixeira",
            FileResultKind::Refresh => "Árvore de arquivos atualizada",
        };
        match refresh {
            Ok(workspace) => {
                self.accept_tree(workspace);
            }
            Err(error) => {
                self.project_files.error =
                    Some(format!("Operação concluída; atualize a árvore: {error}"));
            }
        }
        if !failures.is_empty() {
            let details = failures
                .iter()
                .map(|(path, error)| format!("{}: {error}", path.display()))
                .collect::<Vec<_>>()
                .join("\n");
            let refresh_error = self
                .project_files
                .error
                .take()
                .map(|error| format!("\n{error}"))
                .unwrap_or_default();
            self.project_files.error = Some(format!(
                "{} item(ns) movido(s) para a Lixeira; {} não concluído(s).\n{details}{refresh_error}",
                trashed.len(),
                failures.len()
            ));
            self.status = format!(
                "Lixeira: {} item(ns) movido(s), {} não concluído(s); confira o Project",
                trashed.len(),
                failures.len()
            );
        } else if trashed.len() > 1 {
            self.status = format!("{} itens movidos para a Lixeira", trashed.len());
        } else {
            self.status = status.into();
        }
        Ok(())
    }
    pub(super) fn close_workspace(&mut self) -> Result<(), String> {
        if self.documents.iter().any(Document::is_dirty) {
            return Err("Salve ou feche os documentos alterados antes de fechar o projeto".into());
        }
        if self.session_work_pending()
            || self.remote_db.has_pending_operation()
            || self.git_repositories_busy()
            || self.project_files.pending
            || self.debugger_has_sessions()
        {
            return Err("Aguarde as operações ou encerre o debug antes de fechar o projeto".into());
        }
        if self.remote_db.transaction {
            return Err("Faça Commit ou Rollback no banco antes de fechar o projeto".into());
        }
        self.stop_project_languages()?;
        if self.providers_running() {
            self.execute(Command::StopProviders)?;
        }
        let database_change = self.remote_db.prepare_project_change(None)?;
        self.api.clear_project()?;
        self.remote_db.apply_project_change(database_change);
        if self.db.connected {
            self.db.dispatch(DbCommand::Disconnect)?;
        }
        self.database_path = None;
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.clear_search();
        self.automatic_providers.suspend();
        self.workspace = None;
        self.workspace_file = None;
        self.workspace_folders.clear();
        self.project_encodings.clear();
        self.project_tree = Default::default();
        self.project_encoding = FileEncoding::Utf8;
        self.documents.clear();
        self.active = None;
        self.entries.clear();
        self.tree_truncated = false;
        self.git = git::GitService::new();
        self.git_sessions.clear();
        self.git_target = None;
        self.git_workspace = None;
        self.navigation = NavigationState::default();
        self.status = "Projeto fechado".into();
        Ok(())
    }
}

pub(super) fn relative_name(name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name.len() > 4096 || name.chars().any(char::is_control) {
        return Err("Informe um nome válido".into());
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
        || path.components().any(|part| part.as_os_str() == ".git")
    {
        return Err("Use um caminho relativo dentro do projeto, sem .git ou '..'".into());
    }
    Ok(path.into())
}

#[cfg(test)]
mod batch_receipt_tests {
    use super::*;

    #[test]
    fn partial_trash_ack_closes_only_successes_and_keeps_dirty_recovery_buffers() {
        let owned = tempfile::tempdir().unwrap();
        let root = owned.path().join("project");
        std::fs::create_dir(&root).unwrap();
        let mut app = AppService::new(owned.path().join("profile/session.json"));
        app.workspace = Some(root.clone());
        let mut ids = Vec::new();
        for name in ["moved.txt", "failed.txt", "dirty-recovery.txt", "other.txt"] {
            ids.push(
                app.insert_document(OpenedFile {
                    project_root: root.clone(),
                    encoding: FileEncoding::Utf8,
                    path: root.join(name),
                    text: format!("original {name}"),
                })
                .unwrap(),
            );
        }
        app.active = Some(ids[2]);
        app.sync_views();
        app.execute(Command::Edit {
            id: ids[2],
            expected_version: 1,
            text: "recovery draft 😀".into(),
        })
        .unwrap();
        let before = app
            .documents
            .iter()
            .find(|doc| doc.id == ids[1])
            .unwrap()
            .clone();
        app.project_files.pending = true;
        app.project_files.locked_paths =
            vec![root.join("moved.txt"), root.join("dirty-recovery.txt")];
        app.accept_project_file(
            FileReceipt {
                workspace: root.clone(),
                path: root.clone(),
                kind: FileResultKind::TrashedMany {
                    paths: vec![root.join("moved.txt"), root.join("dirty-recovery.txt")],
                    failures: vec![(root.join("failed.txt"), "controlled desktop failure".into())],
                },
            },
            None,
            Vec::new(),
            Err("controlled refresh failure".into()),
        )
        .unwrap();
        assert!(!app.project_files.pending);
        assert!(app.project_files.locked_paths.is_empty());
        assert!(app.documents.iter().all(|doc| doc.id != ids[0]));
        let failed = app.documents.iter().find(|doc| doc.id == ids[1]).unwrap();
        assert_eq!(failed.path, before.path);
        assert_eq!(failed.version, before.version);
        assert_eq!(failed.text(), before.text());
        assert!(!failed.disk_missing);
        let recovered = app.documents.iter().find(|doc| doc.id == ids[2]).unwrap();
        assert_eq!(recovered.text(), "recovery draft 😀");
        assert!(recovered.is_dirty() && recovered.disk_missing);
        assert!(app.documents.iter().any(|doc| doc.id == ids[3]));
        assert_eq!(
            app.project_files.trashed,
            [root.join("moved.txt"), root.join("dirty-recovery.txt")]
        );
        let error = app.project_files.error.as_ref().unwrap();
        assert!(error.contains("2 item(ns)"));
        assert!(error.contains("failed.txt") && error.contains("controlled desktop failure"));
        assert!(error.contains("controlled refresh failure"));
        app.execute(Command::Undo { id: ids[2] }).unwrap();
        assert_eq!(
            app.documents
                .iter()
                .find(|doc| doc.id == ids[2])
                .unwrap()
                .text(),
            "original dirty-recovery.txt"
        );
    }
}
