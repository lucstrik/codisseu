//! Every cached repository row resolves its own root and a fresh snapshot before acting.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum ScmFileAction {
    Select,
    Stage,
    Discard,
    Diff,
    OpenFile,
    History,
    Conflict,
}
impl CodisseuApp {
    pub(super) fn git_scm_file_target(
        &mut self,
        root: &std::path::Path,
        path: PathBuf,
        side: DiffSide,
        action: ScmFileAction,
    ) {
        self.git_scm_target(root, ScmUiAction::File { path, side, action });
    }

    pub(super) fn git_scm_route_file(
        &mut self,
        path: PathBuf,
        side: DiffSide,
        action: ScmFileAction,
    ) {
        let entry = self
            .service
            .git
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.entries.iter().find(|entry| entry.path == path))
            .cloned();
        match action {
            ScmFileAction::Select => {
                self.git_ui.file = Some(path);
            }
            ScmFileAction::OpenFile => {
                if let Some(repo) = &self.service.git.repo {
                    self.send(Command::OpenFile(repo.root.join(path)));
                }
            }
            ScmFileAction::History => self.git_open_file_history(path),
            ScmFileAction::Diff => self.git_open_diff(path, side),
            ScmFileAction::Conflict => {
                if entry.is_some_and(|entry| entry.conflict) {
                    self.git_open_conflict(path);
                } else {
                    self.git_ui.notice = "Conflito mudou; atualize o repositório".into();
                }
            }
            ScmFileAction::Stage => {
                let Some(entry) = entry.filter(|entry| {
                    !entry.conflict
                        && if side == DiffSide::Index {
                            !entry.untracked && !matches!(entry.index, '.' | ' ' | '?')
                        } else {
                            entry.untracked || !matches!(entry.worktree, '.' | ' ')
                        }
                }) else {
                    self.git_ui.notice =
                        "O arquivo mudou; atualize o repositório antes de preparar a alteração"
                            .into();
                    return;
                };
                self.git_prepare(if side == DiffSide::Index {
                    LocalAction::Unstage {
                        paths: vec![entry.path],
                    }
                } else {
                    LocalAction::Stage {
                        paths: vec![entry.path],
                    }
                });
            }
            ScmFileAction::Discard => {
                let Some(entry) = entry.filter(|entry| !entry.conflict && entry.index != 'A')
                else {
                    self.git_ui.notice =
                        "O arquivo mudou; atualize o repositório antes de descartar".into();
                    return;
                };
                if entry.untracked {
                    self.git_prepare_scm(codisseu_core::git::scm::ScmAction::DeleteUntracked {
                        paths: vec![path],
                    });
                } else {
                    self.git_prepare(LocalAction::Discard { paths: vec![path] });
                }
            }
        }
    }
}
