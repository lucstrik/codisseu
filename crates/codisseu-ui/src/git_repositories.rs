//! Repository-owned UI protocols; workspace presentation remains shared.
use super::*;
use std::collections::HashMap;

#[derive(Default)]
pub(super) struct RepositoryUis {
    workspace: Option<PathBuf>,
    active: Option<PathBuf>,
    pub views: HashMap<PathBuf, git::GitUi>,
}
impl CodisseuApp {
    pub(super) fn git_bind_repository_ui(&mut self) {
        let workspace = self.service.workspace_path().map(PathBuf::from);
        if self.git_repositories.workspace != workspace {
            self.git_repositories.views.clear();
            self.git_repositories.active = None;
            self.git_repositories.workspace = workspace;
        }
        let target = self.service.git_repository_target().map(PathBuf::from);
        if self.git_repositories.active == target {
            return;
        }
        let mut next = target
            .as_ref()
            .and_then(|root| self.git_repositories.views.remove(root))
            .unwrap_or_else(|| git::GitUi {
                labels: tab_label::TabLabels::for_git(),
                branches: true,
                ..Default::default()
            });
        next.commit_open = self.git_ui.commit_open;
        next.source_control = self.git_ui.source_control;
        next.scm.copy_layout_from(&self.git_ui.scm);
        let previous = std::mem::replace(&mut self.git_ui, next);
        if let Some(root) = self.git_repositories.active.take() {
            self.git_repositories.views.insert(root, previous);
        }
        self.git_repositories.active = target;
    }
    pub(super) fn git_poll_repository_uis(&mut self) {
        let Some(foreground) = self.service.git_repository_target().map(PathBuf::from) else {
            return;
        };
        let roots = self
            .git_repositories
            .views
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for root in roots {
            if !self.service.activate_cached_git_repository(&root) {
                continue;
            }
            let mut view = self.git_repositories.views.remove(&root).unwrap();
            std::mem::swap(&mut self.git_ui, &mut view);
            self.poll_selected_git_ui();
            self.sync_git_draft();
            std::mem::swap(&mut self.git_ui, &mut view);
            self.git_repositories.views.insert(root, view);
            assert!(self.service.activate_cached_git_repository(&foreground));
        }
    }
    pub(super) fn git_repository_uis_pending(&self) -> bool {
        !self.git_ui.queue.is_empty()
            || self.git_ui.scm.has_pending()
            || self
                .git_repositories
                .views
                .values()
                .any(|ui| !ui.queue.is_empty() || ui.scm.has_pending())
    }
    pub(super) fn git_cancel_all_repository_uis(&mut self) {
        fn cancel(ui: &mut git::GitUi) {
            ui.queue.clear();
            ui.foreground_request = None;
            ui.preview_requested = false;
            ui.preview_baseline = None;
            ui.intent = None;
            ui.approved_intent = None;
            ui.scm.cancel_pending();
            if !ui.apply_dispatched {
                ui.applying = false;
            }
            match &mut ui.applying_intent {
                Some(git::GitIntent::Commit { push, .. }) => *push = false,
                Some(git::GitIntent::SyncPull { .. }) => {
                    ui.applying_intent = Some(git::GitIntent::Other)
                }
                _ => (),
            }
        }
        cancel(&mut self.git_ui);
        for view in self.git_repositories.views.values_mut() {
            cancel(view);
        }
    }
    fn git_repository_view(&self, root: &std::path::Path) -> Option<&git::GitUi> {
        if self.service.git_repository_target() == Some(root) {
            Some(&self.git_ui)
        } else {
            self.git_repositories.views.get(root)
        }
    }
    pub(super) fn git_repository_controls_busy(&self, root: &std::path::Path) -> bool {
        self.service
            .git_repository_service(root)
            .is_some_and(|g| g.is_busy())
            || self
                .git_repository_view(root)
                .is_some_and(|ui| ui.applying || ui.preview_requested || !ui.queue.is_empty())
    }
    pub(super) fn git_repository_activity_busy(&self, root: &std::path::Path) -> bool {
        if self.service.git_commit_messages.is_running(root) {
            return true;
        }
        if self.service.git_repository_target() == Some(root) {
            return self.git_activity_busy();
        }
        self.git_repository_view(root).is_some_and(|ui| {
            ui.preview_requested
                || ui.queue.iter().any(|r| r.foreground)
                || self
                    .service
                    .git_repository_service(root)
                    .is_some_and(|git| {
                        git.is_busy()
                            && ui.foreground_request == Some((git.repo_generation, git.revision))
                    })
        }) || self.git_insights.foreground_busy(root)
    }
}
