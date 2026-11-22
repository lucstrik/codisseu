//! Workspace/document guards around the asynchronous Git service.
use super::*;

fn writes_repository_files(command: &git::GitCommand) -> bool {
    matches!(
        command,
        git::GitCommand::Prepare(_)
            | git::GitCommand::HunksPrepare { .. }
            | git::GitCommand::Apply { .. }
            | git::GitCommand::WorkflowApply { .. }
            | git::GitCommand::WorkflowPrepare(_)
            | git::GitCommand::ConflictPrepare { .. }
            | git::GitCommand::RebasePrepare { .. }
            | git::GitCommand::ShelfPrepare(_)
            | git::GitCommand::ScmPrepare(_)
    )
}
pub(super) fn validate_git_drafts(drafts: &BTreeMap<PathBuf, String>) -> Result<(), String> {
    if drafts.len() > 32
        || drafts
            .iter()
            .any(|(p, s)| !p.is_absolute() || s.contains('\0') || s.len() > 65536)
        || drafts
            .iter()
            .map(|(p, s)| p.as_os_str().len() + s.len())
            .sum::<usize>()
            > 256 * 1024
    {
        return Err("Rascunhos Git excedem o limite de sessão".into());
    }
    Ok(())
}
pub(super) struct RepositorySession {
    pub service: git::GitService,
    pub applying: bool,
    target: Option<PathBuf>,
}
impl AppService {
    pub fn git_selected_repository(&self) -> Option<&std::path::Path> {
        self.git.repo.as_ref().map(|repo| repo.root.as_path())
    }
    /// Binding identity also exists while an explicitly discovered root opens.
    pub fn git_repository_target(&self) -> Option<&std::path::Path> {
        self.git_selected_repository()
            .or(self.git_target.as_deref())
    }
    pub fn git_repository_service(&self, root: &std::path::Path) -> Option<&git::GitService> {
        if self.git_repository_target() == Some(root) {
            Some(&self.git)
        } else {
            self.git_sessions.get(root).map(|s| &s.service)
        }
    }
    pub fn git_repositories_busy(&self) -> bool {
        self.git.is_busy() || self.git_sessions.values().any(|s| s.service.is_busy())
    }
    /// Select a previously authorized binding without reopening or cancelling it.
    /// Used by the UI pump to advance each repository's prepare/apply protocol.
    pub fn activate_cached_git_repository(&mut self, root: &std::path::Path) -> bool {
        if self.git_repository_target() == Some(root) {
            return true;
        }
        let Some(session) = self.git_sessions.remove(root) else {
            return false;
        };
        self.replace_git_session(session);
        true
    }
    fn replace_git_session(&mut self, mut next: RepositorySession) {
        let old_root = self.git_repository_target().map(PathBuf::from);
        // Discovery is workspace presentation state, not duplicated for every worker.
        self.git
            .merge_repository_summaries(std::mem::take(&mut next.service.repositories));
        std::mem::swap(&mut self.git.repositories, &mut next.service.repositories);
        std::mem::swap(
            &mut self.git.discovery_truncated,
            &mut next.service.discovery_truncated,
        );
        std::mem::swap(
            &mut self.git.discovery_workspace,
            &mut next.service.discovery_workspace,
        );
        std::mem::swap(
            &mut self.git.discovery_errors,
            &mut next.service.discovery_errors,
        );
        let previous = RepositorySession {
            service: std::mem::replace(&mut self.git, next.service),
            applying: std::mem::replace(&mut self.git_applying, next.applying),
            target: std::mem::replace(&mut self.git_target, next.target),
        };
        if let Some(root) = old_root {
            self.git_sessions.insert(root, previous);
        }
    }
    pub(super) fn select_git_repository(&mut self, path: PathBuf) -> Result<(), String> {
        let workspace = self
            .workspace
            .clone()
            .ok_or_else(|| "Abra uma pasta antes de escolher um repositório".to_owned())?;
        if !path.is_absolute() {
            return Err("Selecione um caminho absoluto de repositório".into());
        }
        // Selecting a subdirectory of a known repository reuses that binding,
        // rather than creating two workers for the same index.
        let path = if std::fs::canonicalize(&path).ok().as_ref() == Some(&path) {
            let known = self
                .git
                .repositories
                .iter()
                .filter(|r| path.starts_with(&r.root))
                .max_by_key(|r| r.root.components().count());
            // An undiscovered nested repository must still open its own index.
            known
                .filter(|r| {
                    path.ancestors()
                        .take_while(|p| p.starts_with(&r.root))
                        .find(|p| p.join(".git").exists())
                        == Some(r.root.as_path())
                })
                .map_or(path.clone(), |r| r.root.clone())
        } else {
            path
        };
        // Never carry a cache across a workspace change or an init/clone binding.
        if self.git_workspace.as_deref() != self.workspace_path()
            || self.git_open_destination.is_some()
        {
            return Err("Aguarde a detecção Git da pasta atual".into());
        }
        if self.git_repository_service(&path).is_some()
            && std::fs::canonicalize(&path).ok().as_deref() != Some(path.as_path())
        {
            return Err("Identidade do repositório mudou; abra novamente o workspace".into());
        }
        if self
            .git_repository_service(&path)
            .is_some_and(|git| !git.is_busy() && git.repo.is_none())
        {
            self.git_sessions.remove(&path);
            if self.git_repository_target() == Some(path.as_path()) {
                self.git_target = None;
            }
        }
        if self.activate_cached_git_repository(&path) {
            return Ok(());
        }
        if self.git_repository_target().is_none() && self.git.is_busy() {
            return Err("Aguarde a descoberta dos repositórios".into());
        }
        if self.git_sessions.len() >= 31 && self.git_repository_target().is_some() {
            return Err("Até32repositórios Git podem permanecer abertos por workspace".into());
        }
        let scope = if self.workspace_file.is_some() {
            self.git_workspace_for(&path)
                .ok_or("Repositório fora das pastas do workspace")?
                .to_path_buf()
        } else {
            workspace
        };
        let known = self.git.repositories.iter().any(|r| r.root == path);
        let mut service = git::GitService::new();
        service
            .dispatch(git::GitCommand::OpenWithin {
                workspace: scope,
                path: path.clone(),
                trusted: false,
            })
            .map_err(|error| error.to_string())?;
        self.replace_git_session(RepositorySession {
            service,
            applying: false,
            target: known.then_some(path),
        });
        self.git_autoselect = false;
        Ok(())
    }
    pub(super) fn git_path_applying(&self, path: &std::path::Path) -> bool {
        (self.git_applying
            && self
                .git_repository_target()
                .is_some_and(|root| path.starts_with(root) || root.starts_with(path)))
            || self.git_sessions.iter().any(|(root, session)| {
                session.applying && (path.starts_with(root) || root.starts_with(path))
            })
    }
    pub(super) fn guard_git_apply(&self, command: &Command) -> Result<(), String> {
        if !self.git_applying && !self.git_sessions.values().any(|s| s.applying) {
            return Ok(());
        }
        let document = match command {
            Command::Edit { id, .. }
            | Command::EditDelta { id, .. }
            | Command::EditTransaction { id, .. }
            | Command::EditBatchTransaction { id, .. }
            | Command::Save { id }
            | Command::ReloadDocument { id, .. }
            | Command::OverwriteDocument { id, .. }
            | Command::Undo { id }
            | Command::Redo { id }
            | Command::FormatDocument { id, .. }
            | Command::ReplaceMatches { id, .. }
            | Command::RestoreLocalRevision { id, .. }
            | Command::PrepareRename { id, .. }
            | Command::PrepareFileRename { id, .. }
            | Command::Completion { id, .. }
            | Command::CodeActions { id, .. }
            | Command::Close { id, .. }
            | Command::CloseTab { id, .. } => Some(*id),
            Command::AcceptCompletion { .. } => self.active,
            _ => None,
        };
        let blocked = if let Some(id) = document {
            self.documents
                .iter()
                .any(|d| d.id == id && self.git_path_applying(&d.path))
        } else {
            match command {
                Command::OpenFile(path) | Command::CreateFile(path) => self.git_path_applying(path),
                Command::SaveUntitled { path, .. } => self.git_path_applying(path),
                Command::SetGitDraft { .. }
                | Command::SetCommitToolWindowVisible(_)
                | Command::SetGitConfirmActions(_)
                | Command::SetCopilotEnabled(_)
                | Command::SetCopilotProgram(_)
                | Command::Git(_)
                | Command::SelectGitRepository(_)
                | Command::CancelAll
                | Command::SetSelection { .. }
                | Command::SetViewScroll { .. }
                | Command::SetViewScrollAnchor { .. }
                | Command::SetEditorSoftWrap(_)
                | Command::SetTheme(_)
                | Command::ActivateTab { .. }
                | Command::ShowDocumentInView { .. }
                | Command::Activate { .. }
                | Command::LoadDirectory(_)
                | Command::SetGitInsights { .. }
                | Command::EndEditGroup { .. }
                | Command::PersistSession
                | Command::SetShellLayout(_) => false,
                _ => true,
            }
        };
        if blocked {
            Err(
                "Aguarde a operação Git deste repositório antes de editar ou gravar arquivos"
                    .into(),
            )
        } else {
            Ok(())
        }
    }
    pub fn git_draft(&self, root: &std::path::Path) -> Option<&str> {
        self.git_drafts.get(root).map(String::as_str)
    }
    pub(super) fn set_git_draft(&mut self, root: PathBuf, message: String) -> Result<(), String> {
        if !root.is_absolute() || message.len() > 65536 || message.contains('\0') {
            return Err("Mensagem Git deve ter até64KiB e caminho absoluto".into());
        }
        if message.is_empty() {
            self.git_drafts.remove(&root);
            return Ok(());
        }
        let mut drafts = self.git_drafts.clone();
        drafts.insert(root.clone(), message);
        validate_git_drafts(&drafts)?;
        self.git_drafts = drafts;
        Ok(())
    }
    pub fn git_has_unsaved_documents(&self) -> bool {
        let root = self.git.repo.as_ref().map(|r| &r.root);
        self.documents
            .iter()
            .any(|d| d.is_dirty() && root.is_none_or(|r| d.path.starts_with(r)))
    }
    /// The UI retains a queued mutation until already accepted file operations
    /// acknowledge completion. This mirrors the dispatch guard; it does not
    /// grant permission to write or conceal permanent validation errors.
    pub fn git_file_operations_pending(&self, command: &git::GitCommand) -> bool {
        writes_repository_files(command)
            && (self.pending.values().any(|pending| match pending {
                Pending::Directory | Pending::GitTreeRead => false,
                Pending::GitRead(generation) => *generation == self.git.repo_generation,
                Pending::Save(id) => self.documents.iter().any(|doc| {
                    doc.id == *id
                        && self
                            .git_selected_repository()
                            .is_none_or(|root| doc.path.starts_with(root))
                }),
                _ => true,
            }) || self.rename.is_pending())
    }
    pub(super) fn dispatch_git(&mut self, command: git::GitCommand) -> Result<(), String> {
        let command = if self.workspace_file.is_some()
            && matches!(
                command,
                git::GitCommand::DiscoverRepositories { .. }
                    | git::GitCommand::DiscoverRoots { .. }
                    | git::GitCommand::OpenRoots { .. }
            ) {
            git::GitCommand::DiscoverRoots {
                workspace: self.workspace_path().unwrap().to_path_buf(),
                roots: self.workspace_roots(),
            }
        } else {
            command
        };
        let writing = writes_repository_files(&command);
        if writing {
            if self.git_has_unsaved_documents() && !self.git.command_changes_only_index(&command) {
                return Err(
                    "Salve os arquivos alterados no editor antes de aplicar operações Git".into(),
                );
            }
            if self.git_file_operations_pending(&command) {
                return Err("Aguarde as operações de arquivos antes de aplicar Git".into());
            }
            if self.git_workspace.as_deref() != self.workspace_path()
                || !self.workspace_roots().iter().any(|w| {
                    self.git
                        .repo
                        .as_ref()
                        .is_some_and(|r| w.starts_with(&r.root) || r.root.starts_with(w))
                })
            {
                return Err("Aguarde a detecção Git da pasta atual".into());
            }
        }
        let opening = match &command {
            git::GitCommand::Clone { destination, .. } => Some(destination.clone()),
            git::GitCommand::Init { path, .. } => Some(path.clone()),
            _ => None,
        };
        let applying = matches!(
            &command,
            git::GitCommand::Apply { .. } | git::GitCommand::WorkflowApply { .. }
        );
        self.git.dispatch(command).map_err(|e| e.to_string())?;
        if applying {
            self.git_applying = true;
        }
        if opening.is_some() {
            self.git_open_destination = opening;
        }
        Ok(())
    }
    pub(super) fn poll_git(&mut self) -> bool {
        let mut changed = self.git.poll();
        if !self.git.is_busy()
            && let Some(path) = self.git_open_destination.take()
            && self.git.repo.is_some()
        {
            if self.workspace_file.is_some() && self.workspace_root_for(&path).is_some() {
                // Creating Git in a declared folder must retain the workspace file.
                self.git_workspace = None;
            } else if let Err(error) = self.dispatch(Command::OpenWorkspace(path.clone())) {
                self.last_error = Some(format!("Git: repositório em {}. {error}", path.display()));
            } else {
                self.git_workspace = Some(path);
            }
        }
        let mut completed = Vec::new();
        if self.git_applying && !self.git.is_busy() {
            self.git_applying = false;
            if let Some(root) = self.git_selected_repository() {
                completed.push((root.to_owned(), self.git.repo_generation));
            }
        }
        let mut summaries = Vec::new();
        for (root, session) in &mut self.git_sessions {
            if session.service.poll() {
                changed = true;
                if let Some(workspace) = session.service.discovery_workspace.take() {
                    self.git.discovery_workspace = Some(workspace);
                    self.git.discovery_truncated =
                        std::mem::take(&mut session.service.discovery_truncated);
                    self.git.discovery_errors =
                        std::mem::take(&mut session.service.discovery_errors);
                }
                summaries.append(&mut session.service.repositories);
            }
            if session.applying && !session.service.is_busy() {
                session.applying = false;
                completed.push((root.clone(), session.service.repo_generation));
            }
        }
        self.git.merge_repository_summaries(summaries);
        for (root, generation) in completed {
            self.reload_git_repository(&root, generation);
            changed = true;
        }
        if !self.git.is_busy()
            && self.git_workspace.as_deref() != self.workspace_path()
            && let Some(identity) = self.workspace_path().map(PathBuf::from)
        {
            let command = if self.workspace_file.is_some() {
                git::GitCommand::OpenRoots {
                    workspace: identity.clone(),
                    roots: self.workspace_roots(),
                }
            } else {
                git::GitCommand::Open {
                    path: identity.clone(),
                    trusted: false,
                }
            };
            if self.git.dispatch(command).is_ok() {
                self.git_autoselect = self.workspace_file.is_some();
                self.git_workspace = Some(identity);
                changed = true;
            }
        } else if self.git_autoselect
            && !self.git.is_busy()
            && self.git.discovery_workspace.as_deref() == self.workspace_path()
        {
            self.git_autoselect = false;
            if let Some(repository) = self
                .git
                .repositories
                .iter()
                .find(|r| r.entries.is_some())
                .map(|r| r.root.clone())
                && let Err(error) = self.select_git_repository(repository)
            {
                self.last_error = Some(error);
            }
            changed = true;
        }
        changed
    }
    fn reload_git_repository(&mut self, root: &std::path::Path, generation: u64) {
        let files = self
            .documents
            .iter()
            .filter(|d| !d.is_dirty() && d.path.starts_with(root))
            .map(|d| (d.id, d.version, d.path.clone()))
            .collect::<Vec<_>>();
        if !files.is_empty()
            && let Err(e) = self.enqueue(Work::GitReload(files), Pending::GitRead(generation))
        {
            self.last_error = Some(e);
        }
        if let Some(path) = self.workspace_path().map(PathBuf::from) {
            // Refreshing the tree is a read, not a pending workspace switch.
            let _ = self.enqueue(Work::Workspace(path), Pending::GitTreeRead);
        }
    }
    pub(super) fn finish_git_reload(
        &mut self,
        files: Vec<(DocumentId, u64, Result<Option<OpenedFile>, String>)>,
    ) -> Result<(), String> {
        for (id, version, loaded) in files {
            let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) else {
                continue;
            };
            if doc.version != version || doc.is_dirty() {
                continue;
            }
            match loaded {
                Ok(Some(file)) => {
                    if doc.path != file.path {
                        self.last_error =
                            Some("Arquivo mudou de identidade após Git; buffer preservado".into());
                        continue;
                    }
                    let text_changed = doc.text() != file.text;
                    if text_changed
                        || doc.encoding != file.encoding
                        || doc.project_root != file.project_root
                    {
                        if text_changed {
                            doc.buffer.replace(file.text.clone())?;
                        }
                        doc.encoding = file.encoding;
                        doc.project_root = file.project_root.clone();
                        doc.version += 1;
                        doc.saved_text = Arc::from(file.text);
                        doc.saved_identity = Some(doc.buffer.content_identity());
                        doc.dirty_cache.set(None);
                        self.project_encodings
                            .entry(file.project_root)
                            .or_insert(file.encoding);
                    }
                }
                Ok(None) => {
                    doc.disk_missing = true;
                    self.last_error = Some(format!(
                        "Git removeu {}; buffer preservado para recuperação",
                        doc.path.display()
                    ));
                }
                Err(e) => self.last_error = Some(e),
            }
        }
        self.sync_documents();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        process::Command as Process,
        time::{Duration, Instant},
    };
    fn cli(path: &std::path::Path, args: &[&str]) -> String {
        let o = Process::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8(o.stdout).unwrap()
    }
    fn fixture() -> (tempfile::TempDir, AppService) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repository");
        fs::create_dir(&repo).unwrap();
        cli(&repo, &["init", "-b", "main"]);
        cli(&repo, &["config", "user.name", "Codisseu Fixture"]);
        cli(&repo, &["config", "user.email", "fixture@example.invalid"]);
        cli(&repo, &["config", "commit.gpgsign", "false"]);
        fs::write(repo.join("app.js"), "const original = 1;\n").unwrap();
        cli(&repo, &["add", "app.js"]);
        cli(&repo, &["commit", "-m", "Initial"]);
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenWorkspace(repo.clone())).unwrap();
        settle(&mut app);
        app.dispatch(Command::Git(git::GitCommand::Trust(true)))
            .unwrap();
        settle(&mut app);
        (dir, app)
    }
    fn settle(app: &mut AppService) {
        let start = Instant::now();
        loop {
            app.poll();
            if !app.is_busy() {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "worker timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn apply(app: &mut AppService, action: git::LocalAction) {
        app.dispatch(Command::Git(git::GitCommand::Prepare(action)))
            .unwrap();
        settle(app);
        let p = app
            .git
            .preview
            .clone()
            .unwrap_or_else(|| panic!("{:?}", app.git.error));
        app.dispatch(Command::Git(git::GitCommand::Apply {
            preview_id: p.id,
            repo_generation: p.generation,
        }))
        .unwrap();
        settle(app);
        assert!(
            app.git.report.as_ref().is_some_and(|r| r.success),
            "{:?} {:?}",
            app.git.report,
            app.git.error
        );
    }
    #[test]
    fn git_commit_selection_and_unsaved_buffer_guard() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        app.dispatch(Command::OpenFile(root.join("app.js")))
            .unwrap();
        settle(&mut app);
        let id = app.active.unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "const original = 2;\n".into(),
        })
        .unwrap();
        assert!(
            app.dispatch(Command::Git(git::GitCommand::Prepare(
                git::LocalAction::Commit {
                    selection: git::CommitSelection::WorkingPaths(vec!["app.js".into()]),
                    message: "Unsaved buffer must not be committed".into(),
                    amend: false,
                }
            )))
            .unwrap_err()
            .contains("Salve")
        );
        assert_eq!(cli(&root, &["status", "--porcelain"]), "");
        app.dispatch(Command::Save { id }).unwrap();
        settle(&mut app);
        fs::write(root.join("excluded.txt"), "not committed\n").unwrap();
        app.dispatch(Command::Git(git::GitCommand::Refresh))
            .unwrap();
        settle(&mut app);
        apply(
            &mut app,
            git::LocalAction::Commit {
                selection: git::CommitSelection::WorkingPaths(vec!["app.js".into()]),
                message: "Selected from working tree".into(),
                amend: false,
            },
        );
        assert_eq!(
            cli(&root, &["show", "HEAD:app.js"]),
            "const original = 2;\n"
        );
        assert_eq!(cli(&root, &["status", "--porcelain"]), "?? excluded.txt\n");
        assert!(!app.documents[0].is_dirty());
    }
    #[test]
    fn git_checkout_reloads_clean_documents_and_preserves_concurrent_edit() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        cli(&root, &["checkout", "-b", "feature"]);
        fs::write(root.join("app.js"), "const feature = 2;\n").unwrap();
        cli(&root, &["commit", "-am", "Feature"]);
        cli(&root, &["checkout", "main"]);
        app.dispatch(Command::OpenFile(root.join("app.js")))
            .unwrap();
        settle(&mut app);
        app.dispatch(Command::Git(git::GitCommand::Refresh))
            .unwrap();
        settle(&mut app);
        apply(
            &mut app,
            git::LocalAction::CheckoutBranch {
                name: "feature".into(),
            },
        );
        let d = app.active_document().unwrap();
        assert_eq!(d.text(), "const feature = 2;\n");
        assert!(!d.is_dirty());
        assert!(d.version > 1);
        let id = d.id;
        let version = d.version;
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "unsaved user content".into(),
        })
        .unwrap();
        app.finish_git_reload(vec![(
            id,
            version,
            Ok(Some(OpenedFile {
                project_root: root.clone(),
                encoding: FileEncoding::Utf8,
                path: root.join("app.js"),
                text: "obsolete disk".into(),
            })),
        )])
        .unwrap();
        assert_eq!(
            app.active_document().unwrap().text(),
            "unsaved user content"
        );
    }
    #[test]
    fn git_checkout_refreshes_project_and_codec_when_ascii_content_is_identical() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        let child = root.join("service");
        fs::create_dir(&child).unwrap();
        let path = child.join("ascii.php");
        fs::write(&path, "plain ASCII\n").unwrap();
        fs::write(
            root.join(".editorconfig"),
            "root=true\n[*]\ncharset=utf-8\n",
        )
        .unwrap();
        cli(&root, &["add", "."]);
        cli(&root, &["commit", "-m", "ASCII base"]);
        cli(&root, &["checkout", "-b", "independent-service"]);
        fs::write(child.join("composer.json"), "{}\n").unwrap();
        fs::write(child.join(".editorconfig"), "[*.php]\ncharset=latin1\n").unwrap();
        cli(&root, &["add", "."]);
        cli(
            &root,
            &["commit", "-m", "Project and charset metadata only"],
        );
        cli(&root, &["checkout", "main"]);
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        settle(&mut app);
        let before = app.active_document().unwrap();
        let (id, initial_version) = (before.id, before.version);
        assert_eq!(before.project_root(), root);
        assert_eq!(before.encoding(), FileEncoding::Utf8);
        app.dispatch(Command::Git(git::GitCommand::Refresh))
            .unwrap();
        settle(&mut app);
        apply(
            &mut app,
            git::LocalAction::CheckoutBranch {
                name: "independent-service".into(),
            },
        );
        let document = app.active_document().unwrap();
        assert_eq!(document.text(), "plain ASCII\n");
        assert_eq!(document.project_root(), child);
        assert_eq!(document.encoding(), FileEncoding::Latin1);
        assert!(document.version > initial_version);
        assert!(!document.is_dirty());
        assert_eq!(fs::read(&path).unwrap(), b"plain ASCII\n");
        // Metadata from an earlier generation must not rebind a newer buffer.
        app.finish_git_reload(vec![(
            id,
            initial_version,
            Ok(Some(OpenedFile {
                path: path.clone(),
                project_root: root.clone(),
                encoding: FileEncoding::Utf8,
                text: "plain ASCII\n".into(),
            })),
        )])
        .unwrap();
        assert_eq!(app.active_document().unwrap().project_root(), child);
        apply(
            &mut app,
            git::LocalAction::CheckoutBranch {
                name: "main".into(),
            },
        );
        let document = app.active_document().unwrap();
        assert_eq!(document.project_root(), root);
        assert_eq!(document.encoding(), FileEncoding::Utf8);
        let version = document.version;
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "local café".into(),
        })
        .unwrap();
        let dirty_version = app.active_document().unwrap().version;
        app.finish_git_reload(vec![(
            id,
            dirty_version,
            Ok(Some(OpenedFile {
                path,
                project_root: child,
                encoding: FileEncoding::Latin1,
                text: "plain ASCII\n".into(),
            })),
        )])
        .unwrap();
        let document = app.active_document().unwrap();
        assert_eq!(document.project_root(), root);
        assert_eq!(document.encoding(), FileEncoding::Utf8);
        assert_eq!(document.text(), "local café");
        assert!(document.is_dirty());
    }
    #[test]
    fn git_removed_empty_buffer_stays_recoverable() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        fs::write(root.join("empty.txt"), "").unwrap();
        app.dispatch(Command::OpenFile(root.join("empty.txt")))
            .unwrap();
        settle(&mut app);
        let d = app.active_document().unwrap();
        let (id, version) = (d.id, d.version);
        app.finish_git_reload(vec![(id, version, Ok(None))])
            .unwrap();
        assert!(app.active_document().unwrap().is_dirty());
        assert_eq!(app.active_document().unwrap().text(), "");
    }
    #[test]
    fn git_drafts_round_trip_and_old_sessions_remain_compatible() {
        let (dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        let message = "Corrigir integração Git\n\nPreservar observações e Unicode: ação 🌧";
        app.dispatch(Command::SetGitDraft {
            root: root.clone(),
            message: message.into(),
        })
        .unwrap();
        app.dispatch(Command::PersistSession).unwrap();
        settle(&mut app);
        let mut restored = AppService::new(dir.path().join("session.json"));
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert_eq!(restored.git_draft(&root), Some(message));
        restored
            .dispatch(Command::SetGitDraft {
                root: root.clone(),
                message: String::new(),
            })
            .unwrap();
        assert_eq!(restored.git_draft(&root), None);
        assert!(
            restored
                .dispatch(Command::SetGitDraft {
                    root: root.clone(),
                    message: "a".repeat(65537)
                })
                .is_err()
        );
        assert!(
            restored
                .dispatch(Command::SetGitDraft {
                    root: "relative".into(),
                    message: "message".into()
                })
                .is_err()
        );
    }
    #[test]
    fn thirty_two_repository_drafts_persist_without_silent_eviction() {
        let (dir, mut app) = fixture();
        for number in 0..32 {
            app.dispatch(Command::SetGitDraft {
                root: dir.path().join(format!("project-{number}")),
                message: format!("Message {number}\nBody"),
            })
            .unwrap();
        }
        let expected = app.git_drafts.clone();
        assert!(
            app.dispatch(Command::SetGitDraft {
                root: dir.path().join("project-33"),
                message: "Retain all existing drafts".into(),
            })
            .is_err()
        );
        assert_eq!(app.git_drafts, expected);
        app.dispatch(Command::PersistSession).unwrap();
        settle(&mut app);
        let mut restored = AppService::new(dir.path().join("session.json"));
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert_eq!(restored.git_drafts, expected);
        for number in 0..3 {
            restored
                .dispatch(Command::SetGitDraft {
                    root: dir.path().join(format!("project-{number}")),
                    message: "x".repeat(65536),
                })
                .unwrap();
        }
        let before_overflow = restored.git_drafts.clone();
        assert!(
            restored
                .dispatch(Command::SetGitDraft {
                    root: dir.path().join("project-3"),
                    message: "x".repeat(65536),
                })
                .is_err()
        );
        assert_eq!(restored.git_drafts, before_overflow);
    }
    #[test]
    fn selected_child_repository_keeps_binding_and_mutations_inside_that_repo() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        fs::create_dir(&workspace).unwrap();
        for name in ["first", "second"] {
            let root = workspace.join(name);
            fs::create_dir(&root).unwrap();
            cli(&root, &["init", "-b", "main"]);
            let hooks = root.join(".git/fixture-hooks");
            fs::create_dir(&hooks).unwrap();
            for (key, value) in [
                ("user.name", "Multirepo Fixture"),
                ("user.email", "fixture@example.invalid"),
                ("commit.gpgsign", "false"),
                ("core.hooksPath", hooks.to_str().unwrap()),
            ] {
                cli(&root, &["config", key, value]);
            }
            fs::write(root.join("app.js"), "base\n").unwrap();
            cli(&root, &["add", "."]);
            cli(&root, &["commit", "-m", "Base"]);
        }
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenWorkspace(workspace.clone()))
            .unwrap();
        settle(&mut app);
        let first = workspace.join("first");
        let second = workspace.join("second");
        app.dispatch(Command::SelectGitRepository(first.clone()))
            .unwrap();
        settle(&mut app);
        for _ in 0..4 {
            app.poll();
        }
        assert_eq!(app.git_selected_repository(), Some(first.as_path()));
        app.dispatch(Command::Git(git::GitCommand::Trust(true)))
            .unwrap();
        settle(&mut app);
        fs::write(first.join("app.js"), "selected repo\n").unwrap();
        fs::write(second.join("app.js"), "other repo\n").unwrap();
        apply(
            &mut app,
            git::LocalAction::Stage {
                paths: vec!["app.js".into()],
            },
        );
        assert_eq!(
            cli(&first, &["diff", "--cached", "--name-only"]),
            "app.js\n"
        );
        assert_eq!(cli(&second, &["diff", "--cached", "--name-only"]), "");
        app.dispatch(Command::SelectGitRepository(second.clone()))
            .unwrap();
        settle(&mut app);
        assert_eq!(app.git_selected_repository(), Some(second.as_path()));
        assert!(!app.git.trusted);
    }
    #[test]
    fn repository_previews_cancel_and_trust_remain_owned_by_their_root() {
        let (dir, mut app) = fixture();
        let first = dir.path().join("repository");
        let second = dir.path().join("second");
        fs::create_dir(&second).unwrap();
        cli(&second, &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            cli(&second, &["config", key, value]);
        }
        fs::write(second.join("app.js"), "second base\n").unwrap();
        cli(&second, &["add", "."]);
        cli(&second, &["commit", "-m", "Second base"]);
        app.dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        settle(&mut app);
        fs::write(first.join("app.js"), "first changed\n").unwrap();
        fs::write(second.join("app.js"), "second changed\n").unwrap();
        app.dispatch(Command::SelectGitRepository(first.clone()))
            .unwrap();
        settle(&mut app);
        app.dispatch(Command::Git(git::GitCommand::Trust(true)))
            .unwrap();
        app.dispatch(Command::Git(git::GitCommand::Prepare(
            git::LocalAction::Stage {
                paths: vec!["app.js".into()],
            },
        )))
        .unwrap();
        settle(&mut app);
        let a = app.git.preview.clone().unwrap();
        let child = first.join("child");
        fs::create_dir(&child).unwrap();
        app.dispatch(Command::SelectGitRepository(child.clone()))
            .unwrap();
        assert_eq!(app.git_selected_repository(), Some(first.as_path()));
        assert_eq!(app.git.preview.as_ref().unwrap().generation, a.generation);
        app.dispatch(Command::SelectGitRepository(second.clone()))
            .unwrap();
        settle(&mut app);
        assert!(!app.git.trusted);
        app.dispatch(Command::Git(git::GitCommand::Trust(true)))
            .unwrap();
        app.dispatch(Command::Git(git::GitCommand::Prepare(
            git::LocalAction::Stage {
                paths: vec!["app.js".into()],
            },
        )))
        .unwrap();
        settle(&mut app);
        let b = app.git.preview.clone().unwrap();
        assert_ne!(
            a.generation, b.generation,
            "preview IDs can repeat; generations must not"
        );
        assert!(
            app.dispatch(Command::Git(git::GitCommand::Apply {
                preview_id: a.id,
                repo_generation: a.generation,
            }))
            .is_err()
        );
        app.dispatch(Command::Git(git::GitCommand::Cancel)).unwrap();
        assert!(app.git.preview.is_none());
        assert!(
            app.git_repository_service(&first)
                .unwrap()
                .preview
                .is_some()
        );
        app.dispatch(Command::SelectGitRepository(first.clone()))
            .unwrap();
        assert!(app.git.trusted);
        assert_eq!(app.git.preview.as_ref().unwrap().generation, a.generation);
        app.dispatch(Command::Git(git::GitCommand::Apply {
            preview_id: a.id,
            repo_generation: a.generation,
        }))
        .unwrap();
        settle(&mut app);
        assert_eq!(
            cli(&first, &["diff", "--cached", "--name-only"]),
            "app.js\n"
        );
        assert_eq!(cli(&second, &["diff", "--cached", "--name-only"]), "");
        let nested = child.join("nested");
        fs::create_dir(&nested).unwrap();
        cli(&nested, &["init", "-b", "main"]);
        app.dispatch(Command::SelectGitRepository(nested.clone()))
            .unwrap();
        settle(&mut app);
        assert_eq!(
            app.git_selected_repository(),
            Some(nested.as_path()),
            "an undiscovered nested repository must never reuse its parent's index"
        );
        app.dispatch(Command::OpenWorkspace(second.clone()))
            .unwrap();
        settle(&mut app);
        assert!(
            app.git_repository_service(&first).is_none(),
            "workspace swap must discard old bindings"
        );
    }
    #[cfg(unix)]
    #[test]
    fn cancelling_one_running_commit_does_not_cancel_a_sibling_worker() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, mut app) = fixture();
        let first = dir.path().join("repository");
        let second = dir.path().join("second");
        fs::create_dir(&second).unwrap();
        cli(&second, &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            cli(&second, &["config", key, value]);
        }
        fs::write(second.join("app.js"), "second base\n").unwrap();
        cli(&second, &["add", "."]);
        cli(&second, &["commit", "-m", "Second base"]);
        for root in [&first, &second] {
            fs::write(root.join("app.js"), "changed\n").unwrap();
            cli(root, &["add", "."]);
            let hooks = root.join(".git/test-hooks");
            fs::create_dir(&hooks).unwrap();
            cli(root, &["config", "core.hooksPath", hooks.to_str().unwrap()]);
            let hook = hooks.join("pre-commit");
            fs::write(
                &hook,
                "#!/bin/sh\ntouch .git/entered\nwhile [ ! -f .git/release ]; do sleep 0.05; done\n",
            )
            .unwrap();
            fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
        }
        struct Release(Vec<PathBuf>);
        impl Drop for Release {
            fn drop(&mut self) {
                for path in &self.0 {
                    let _ = fs::write(path, "release");
                }
            }
        }
        let release = Release(vec![
            first.join(".git/release"),
            second.join(".git/release"),
        ]);
        app.dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        settle(&mut app);
        let mut heads = Vec::new();
        for root in [&first, &second] {
            heads.push(cli(root, &["rev-parse", "HEAD"]));
            app.dispatch(Command::SelectGitRepository(root.clone()))
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.git.is_busy() {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            app.dispatch(Command::Git(git::GitCommand::Trust(true)))
                .unwrap();
            app.dispatch(Command::Git(git::GitCommand::Prepare(
                git::LocalAction::Commit {
                    selection: git::CommitSelection::AllStaged,
                    message: "Independent commit".into(),
                    amend: false,
                },
            )))
            .unwrap();
            while app.git.is_busy() {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            let preview = app
                .git
                .preview
                .clone()
                .unwrap_or_else(|| panic!("{:?}", app.git.error));
            app.dispatch(Command::Git(git::GitCommand::Apply {
                preview_id: preview.id,
                repo_generation: preview.generation,
            }))
            .unwrap();
            while !root.join(".git/entered").exists() {
                app.poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        app.dispatch(Command::SelectGitRepository(first.clone()))
            .unwrap();
        app.dispatch(Command::Git(git::GitCommand::Cancel)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.git.is_busy() {
            app.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.git.error.is_some());
        assert_eq!(cli(&first, &["rev-parse", "HEAD"]), heads[0]);
        assert!(app.git_repository_service(&second).unwrap().is_busy());
        assert_eq!(cli(&second, &["rev-parse", "HEAD"]), heads[1]);
        drop(release);
        settle(&mut app);
        assert_ne!(cli(&second, &["rev-parse", "HEAD"]), heads[1]);
        assert!(
            app.git_repository_service(&second)
                .unwrap()
                .report
                .as_ref()
                .unwrap()
                .success
        );
        assert!(app.git_repository_service(&second).unwrap().error.is_none());
    }
    #[test]
    fn repository_selection_rejects_outside_root_and_symlink_escape() {
        let (outside, mut app) = fixture();
        let workspace = tempfile::tempdir().unwrap();
        app.dispatch(Command::OpenWorkspace(workspace.path().into()))
            .unwrap();
        settle(&mut app);
        let root = outside.path().join("repository");
        app.dispatch(Command::SelectGitRepository(root)).unwrap();
        settle(&mut app);
        assert!(app.git.repo.is_none());
        assert!(
            app.git
                .error
                .as_ref()
                .is_some_and(|error| error.contains("dentro"))
        );
        #[cfg(unix)]
        {
            let link = workspace.path().join("escape");
            std::os::unix::fs::symlink(outside.path().join("repository"), &link).unwrap();
            app.dispatch(Command::SelectGitRepository(link)).unwrap();
            settle(&mut app);
            assert!(app.git.repo.is_none());
            assert!(
                app.git
                    .error
                    .as_ref()
                    .is_some_and(|error| error.contains("dentro"))
            );
        }
        for _ in 0..4 {
            app.poll();
        }
        assert!(app.git.repo.is_none());
    }
    #[test]
    fn shutdown_cancels_git_history_in_active_and_parked_repository_services() {
        let (dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        let mut parked = git::GitService::new();
        parked
            .dispatch(git::GitCommand::Open {
                path: root.clone(),
                trusted: true,
            })
            .unwrap();
        let parked_key = dir.path().join("parked");
        app.git_sessions.insert(
            parked_key.clone(),
            RepositorySession {
                service: parked,
                applying: false,
                target: Some(root),
            },
        );
        settle(&mut app);
        app.git
            .dispatch(git::GitCommand::History {
                skip: 0,
                limit: 10,
                path: None,
            })
            .unwrap();
        app.git_sessions
            .get_mut(&parked_key)
            .unwrap()
            .service
            .dispatch(git::GitCommand::History {
                skip: 0,
                limit: 10,
                path: None,
            })
            .unwrap();
        assert!(app.shutdown_blockers().contains(&"Git"));
        assert!(app.shutdown_blockers().contains(&"Git de outros projetos"));
        app.cancel_shutdown_reads();
        settle(&mut app);
        for service in [&app.git, &app.git_sessions[&parked_key].service] {
            assert!(
                service.history.is_none(),
                "A cancelled ready page must not be published"
            );
            assert!(
                service
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("cancelada"))
            );
        }
    }

    #[test]
    fn shutdown_preserves_git_mutation_in_active_and_parked_repository_services() {
        for parked in [false, true] {
            let (dir, mut app) = fixture();
            let root = app.workspace.clone().unwrap();
            fs::write(root.join("app.js"), "const original = 2;\n").unwrap();
            app.dispatch(Command::Git(git::GitCommand::Prepare(
                git::LocalAction::Stage {
                    paths: vec!["app.js".into()],
                },
            )))
            .unwrap();
            settle(&mut app);
            let preview = app.git.preview.clone().unwrap();
            app.dispatch(Command::Git(git::GitCommand::Apply {
                preview_id: preview.id,
                repo_generation: preview.generation,
            }))
            .unwrap();
            assert!(app.git_applying);
            let key = dir.path().join("parked");
            if parked {
                app.git_sessions.insert(
                    key.clone(),
                    RepositorySession {
                        service: std::mem::take(&mut app.git),
                        applying: std::mem::take(&mut app.git_applying),
                        target: Some(root.clone()),
                    },
                );
                // Keep automatic discovery from creating an unrelated replacement
                // while this test exercises the parked worker's acknowledgement.
                app.git_workspace = app.workspace_path().map(PathBuf::from);
            }
            app.cancel_shutdown_reads();
            settle(&mut app);
            let service = if parked {
                &app.git_sessions[&key].service
            } else {
                &app.git
            };
            assert!(
                service.report.as_ref().is_some_and(|r| r.success),
                "{:?}",
                service.error
            );
            assert_eq!(cli(&root, &["diff", "--cached", "--name-only"]), "app.js\n");
        }
    }
    #[test]
    fn shutdown_preserves_accepted_git_init_outside_apply_guard() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("new-repository");
        fs::create_dir(&root).unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::Git(git::GitCommand::Init {
            path: root.clone(),
            initial_branch: "main".into(),
        }))
        .unwrap();
        assert!(!app.git_applying);
        assert!(!app.git.cancellable_read_pending());
        app.cancel_shutdown_reads();
        settle(&mut app);
        assert!(root.join(".git").is_dir());
        assert_eq!(app.git_selected_repository(), Some(root.as_path()));
        assert!(app.git.error.is_none(), "{:?}", app.git.error);
    }
    #[test]
    fn index_only_stage_and_unstage_preserve_unsaved_buffers_and_disk() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        fs::write(root.join("app.js"), "const disk = 'ação 😀';\n").unwrap();
        app.dispatch(Command::OpenFile(root.join("app.js")))
            .unwrap();
        settle(&mut app);
        let doc = app.active_document().unwrap();
        let (id, expected_version) = (doc.id, doc.version);
        app.dispatch(Command::Edit {
            id,
            expected_version,
            text: "const unsaved = 'local';\n".into(),
        })
        .unwrap();
        let version = app.active_document().unwrap().version;
        let head = cli(&root, &["rev-parse", "HEAD"]);
        let index = cli(&root, &["write-tree"]);
        for (step, action) in [
            git::LocalAction::Stage {
                paths: vec!["app.js".into()],
            },
            git::LocalAction::Unstage {
                paths: vec!["app.js".into()],
            },
        ]
        .into_iter()
        .enumerate()
        {
            apply(&mut app, action);
            let doc = app.active_document().unwrap();
            assert_eq!(doc.text(), "const unsaved = 'local';\n");
            assert_eq!(doc.version, version);
            assert!(doc.is_dirty());
            assert_eq!(
                fs::read_to_string(root.join("app.js")).unwrap(),
                "const disk = 'ação 😀';\n"
            );
            assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
            if step == 0 {
                assert_eq!(
                    cli(&root, &["show", ":app.js"]),
                    "const disk = 'ação 😀';\n"
                );
            } else {
                assert_eq!(cli(&root, &["write-tree"]), index);
            }
        }
        assert_eq!(cli(&root, &["write-tree"]), index);
    }
    #[test]
    fn caller_modified_preview_cannot_bypass_dirty_guard_for_discard_apply() {
        let (_dir, mut app) = fixture();
        let root = app.workspace.clone().unwrap();
        fs::write(root.join("app.js"), "const disk = 2;\n").unwrap();
        app.dispatch(Command::OpenFile(root.join("app.js")))
            .unwrap();
        settle(&mut app);
        app.dispatch(Command::Git(git::GitCommand::Prepare(
            git::LocalAction::Discard {
                paths: vec!["app.js".into()],
            },
        )))
        .unwrap();
        settle(&mut app);
        let preview = app.git.preview.as_mut().unwrap();
        preview.summary = "Stage app.js".into();
        let (preview_id, repo_generation) = (preview.id, preview.generation);
        let doc = app.active_document().unwrap();
        let (id, expected_version) = (doc.id, doc.version);
        app.dispatch(Command::Edit {
            id,
            expected_version,
            text: "do not lose this buffer\n".into(),
        })
        .unwrap();
        let command = git::GitCommand::Apply {
            preview_id,
            repo_generation,
        };
        assert!(!app.git.command_changes_only_index(&command));
        let index = cli(&root, &["write-tree"]);
        let head = cli(&root, &["rev-parse", "HEAD"]);
        assert!(
            app.dispatch(Command::Git(command))
                .unwrap_err()
                .contains("Salve")
        );
        assert_eq!(cli(&root, &["write-tree"]), index);
        assert_eq!(cli(&root, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read_to_string(root.join("app.js")).unwrap(),
            "const disk = 2;\n"
        );
        assert_eq!(
            app.active_document().unwrap().text(),
            "do not lose this buffer\n"
        );
        assert!(app.active_document().unwrap().is_dirty());
    }
}
