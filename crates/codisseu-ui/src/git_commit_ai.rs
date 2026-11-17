//! One click requests AI; only an unchanged repository draft receives the result.
use super::*;
impl CodisseuApp {
    pub(super) fn git_generate_commit_message(&mut self, root: PathBuf) {
        if self.service.git_commit_messages.is_running(&root) {
            self.send(Command::CancelGitCommitMessage {
                root: Some(root.clone()),
            });
            self.git_commit_ai_notice_set(
                &root,
                Some("Interrompendo a geração da mensagem…".into()),
            );
            return;
        }
        self.sync_git_draft();
        self.git_commit_ai_notice_set(&root, None);
        if let Err(error) = self
            .service
            .dispatch(Command::GenerateGitCommitMessage { root: root.clone() })
        {
            self.git_commit_ai_notice_set(&root, Some(error));
        }
    }
    pub(super) fn git_commit_ai_tick(&mut self, ctx: &egui::Context) {
        while let Some(event) = self.service.git_commit_messages.take_result() {
            ctx.request_repaint();
            if self.service.workspace_path() != Some(event.workspace.as_path()) {
                continue;
            }
            let message = match event.result {
                Ok(message) if self.service.git_draft(&event.root).unwrap_or("") == event.draft => {
                    message
                }
                Ok(_) => {
                    self.git_commit_ai_notice_set(
                        &event.root,
                        Some("Sua mensagem foi editada durante a geração e foi preservada.".into()),
                    );
                    continue;
                }
                Err(error) => {
                    self.git_commit_ai_notice_set(&event.root, Some(error));
                    continue;
                }
            };
            if let Err(error) = self.service.dispatch(Command::SetGitDraft {
                root: event.root.clone(),
                message: message.clone(),
            }) {
                self.git_commit_ai_notice_set(&event.root, Some(error));
                continue;
            }
            if self.service.git_repository_target() == Some(event.root.as_path()) {
                self.git_ui.message = message;
                self.git_ui.commit_ai_notice = None;
            } else if let Some(view) = self.git_repositories.views.get_mut(&event.root) {
                view.message = message;
                view.commit_ai_notice = None;
            }
        }
    }
    pub(super) fn git_commit_ai_notice(&self, root: &std::path::Path) -> Option<&str> {
        if self.service.git_repository_target() == Some(root) {
            self.git_ui.commit_ai_notice.as_deref()
        } else {
            self.git_repositories
                .views
                .get(root)
                .and_then(|view| view.commit_ai_notice.as_deref())
        }
    }
    fn git_commit_ai_notice_set(&mut self, root: &std::path::Path, message: Option<String>) {
        if self.service.git_repository_target() == Some(root) {
            self.git_ui.commit_ai_notice = message;
        } else if let Some(view) = self.git_repositories.views.get_mut(root) {
            view.commit_ai_notice = message;
        }
    }
}
