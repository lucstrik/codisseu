//! Native Git presentation. Git data and mutations live in the bounded core worker.
use super::*;
use codisseu_core::git::{CommitSelection, DiffSide, GitCommand, LocalAction};
use std::collections::{BTreeSet, HashMap};

pub(super) struct GitRequest {
    pub command: GitCommand,
    pub foreground: bool,
}

#[derive(Default)]
pub(super) struct GitUi {
    pub labels: super::tab_label::TabLabels,
    pub refs_cache: Option<std::sync::Arc<super::git_log::RefRowsCache>>,
    pub commits_cache: Option<std::sync::Arc<super::git_log::CommitRowsCache>>,
    pub commit_open: bool,
    pub source_control: bool,
    pub scm: super::git_scm::SourceControlUi,
    pub focus_commit_tree: bool,
    pub commit_tab: usize,
    pub stash: Option<String>,
    pub shelf: Option<String>,
    pub conflict_stage: bool,
    pub merge_ui: super::git_merge_view::MergeUi,
    pub merge_result_revision: u64,
    pub merge_apply_next: bool,
    pub merge_markers: usize,
    pub dialog: Option<GitDialog>,
    pub diff_open: bool,
    pub diff_return: Option<Box<GitDialog>>,
    pub diff_range: Option<(Option<String>, String, Option<PathBuf>)>,
    pub diff_data: Option<std::sync::Arc<super::git_diff::DiffData>>,
    pub file_version_open: bool,
    pub message: String,
    pub commit_ai_notice: Option<String>,
    pub amend: bool,
    pub amend_load_expected: Option<String>,
    pub amend_backup: Option<String>,
    pub amend_loaded: Option<String>,
    pub staging: bool,
    pub groups: HashMap<String, bool>,
    pub commit_group: Option<String>,
    pub selected: BTreeSet<PathBuf>,
    pub selected_hunks: BTreeSet<usize>,
    pub file: Option<PathBuf>,
    pub commit: Option<String>,
    pub filter: String,
    pub branch_filter: String,
    pub popup_branch_filter: String,
    pub log_branch: String,
    pub page: usize,
    pub history_path: Option<PathBuf>,
    pub tab: usize,
    pub branches: bool,
    pub diff_unified: bool,
    pub diff_no_wrap: bool,
    pub diff_view: super::git_diff_view::DiffView,
    pub notice: String,
    pub output: String,
    pub queue: VecDeque<GitRequest>,
    pub foreground_request: Option<(u64, u64)>,
    pub generation: u64,
    pub revision: u64,
    pub preview_requested: bool,
    pub confirm_current_action: bool,
    pub preview_baseline: Option<u64>,
    pub applying: bool,
    pub apply_dispatched: bool,
    pub committing: bool,
    pub intent: Option<GitIntent>,
    pub approved_intent: Option<(u64, GitIntent)>,
    pub applying_intent: Option<GitIntent>,
    pub conflict_draft: Option<GitDialog>,
    pub operation_draft: Option<Box<GitDialog>>,
    pub push_after_commit: bool,
    pub drafts: HashMap<PathBuf, String>,
    pub root: Option<PathBuf>,
}
#[derive(Clone)]
pub(super) enum GitIntent {
    Commit { message: String, push: bool },
    Other,
    ConfirmedAction,
    SyncPull { root: PathBuf },
}
#[derive(Clone)]
pub(super) enum GitDialog {
    Scm(super::git_scm::ScmDialog),
    Preview,
    Action {
        kind: GitActionKind,
        name: String,
        other: String,
        message: String,
        mode: usize,
        check: bool,
    },
    Conflict {
        path: PathBuf,
        result: String,
        edited: bool,
    },
    Push {
        remote: String,
        source: String,
        target: String,
        tags: bool,
        tag_mode: usize,
        commit: Option<String>,
        requested: bool,
    },
    Pull {
        remote: String,
        branch: String,
        mode: usize,
    },
    Stash {
        message: String,
        keep_index: bool,
        untracked: bool,
    },
    Shelve {
        name: String,
        paths: Vec<PathBuf>,
        selected: BTreeSet<PathBuf>,
    },
    Unshelve {
        id: String,
        requested: bool,
        initialized: bool,
        selected: BTreeSet<PathBuf>,
        name: String,
        comment: String,
        active: bool,
        remove_applied: bool,
    },
    Changelist {
        id: Option<String>,
        name: String,
        active: bool,
    },
    InteractiveRebase {
        base: String,
        head: String,
        requested: bool,
        steps: Vec<codisseu_core::git::rebase::RebaseStep>,
        selected: usize,
        editing: bool,
    },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GitActionKind {
    Init,
    Clone,
    NewBranch,
    RenameBranch,
    DeleteBranch,
    Checkout,
    NewTag,
    DeleteTag,
    Fetch,
    Pull,
    Push,
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Reset,
    Stash,
    Stashes,
}
impl GitActionKind {
    pub fn title(self) -> &'static str {
        match self {
            Self::Init => "Create Git Repository",
            Self::Clone => "Clone Repository",
            Self::NewBranch => "New Branch",
            Self::RenameBranch => "Rename Branch",
            Self::DeleteBranch => "Delete Branch",
            Self::Checkout => "Checkout",
            Self::NewTag => "New Tag",
            Self::DeleteTag => "Delete Tag",
            Self::Fetch => "Fetch",
            Self::Pull => "Pull",
            Self::Push => "Push Commits",
            Self::Merge => "Merge into Current",
            Self::Rebase => "Rebase Current onto",
            Self::CherryPick => "Cherry-Pick",
            Self::Revert => "Revert Commit",
            Self::Reset => "Reset Current Branch",
            Self::Stash => "Stash Changes",
            Self::Stashes => "Unstash Changes",
        }
    }
}
impl CodisseuApp {
    pub(super) fn git_activity_busy(&self) -> bool {
        self.git_ui.preview_requested
            || self.git_ui.queue.iter().any(|request| request.foreground)
            || (self.service.git.is_busy()
                && self.git_ui.foreground_request
                    == Some((self.service.git.repo_generation, self.service.git.revision)))
            || self
                .service
                .git_repository_target()
                .is_some_and(|root| self.git_insights.foreground_busy(root))
    }
    // Use only an explicit upstream or an unambiguous single remote.
    pub(super) fn git_quick_remote(&self) -> Option<(String, String, String)> {
        let snapshot = self.service.git.snapshot.as_ref()?;
        let source = snapshot
            .head_ref
            .as_deref()?
            .strip_prefix("refs/heads/")?
            .to_owned();
        if let Some((remote, target)) = snapshot
            .upstream
            .as_deref()
            .and_then(|s| s.strip_prefix("refs/remotes/").unwrap_or(s).split_once('/'))
            && self.service.git.remotes.iter().any(|r| r.name == remote)
        {
            return Some((remote.into(), source, target.into()));
        }
        (self.service.git.remotes.len() == 1).then(|| {
            (
                self.service.git.remotes[0].name.clone(),
                source.clone(),
                source,
            )
        })
    }
    pub(super) fn git_history_loading(&self) -> bool {
        self.service.git.history_pending()
            || self.git_ui.queue.iter().any(|c| {
                matches!(
                    &c.command,
                    GitCommand::History { .. } | GitCommand::HistoryReference { .. }
                )
            })
    }
    pub(super) fn git_send(&mut self, command: GitCommand) {
        let foreground = matches!(
            &command,
            GitCommand::Prepare(_)
                | GitCommand::WorkflowPrepare(_)
                | GitCommand::ShelfPrepare(_)
                | GitCommand::ScmPrepare(_)
                | GitCommand::ConflictPrepare { .. }
                | GitCommand::HunksPrepare { .. }
                | GitCommand::RebasePrepare { .. }
                | GitCommand::Apply { .. }
                | GitCommand::WorkflowApply { .. }
                | GitCommand::Init { .. }
                | GitCommand::Clone { .. }
        );
        self.git_enqueue(command, foreground);
    }
    pub(super) fn git_send_foreground(&mut self, command: GitCommand) {
        self.git_enqueue(command, true);
    }
    pub(super) fn git_enqueue(&mut self, command: GitCommand, foreground: bool) {
        // History is a replaceable read. Preserve mutation/review ordering while
        // keeping only the latest queued page/ref when the user clicks rapidly.
        if matches!(
            command,
            GitCommand::History { .. } | GitCommand::HistoryReference { .. }
        ) {
            self.git_ui.queue.retain(|c| {
                !matches!(
                    &c.command,
                    GitCommand::History { .. } | GitCommand::HistoryReference { .. }
                )
            });
        }
        if self.git_ui.queue.len() < 16 {
            self.git_ui.queue.push_back(GitRequest {
                command,
                foreground,
            });
        }
    }
    pub(super) fn git_begin_intent(&mut self, intent: GitIntent) -> bool {
        if self.git_ui.preview_requested || self.git_ui.applying {
            self.git_ui.notice = "Aguarde a operação Git atual".into();
            return false;
        }
        self.git_ui.committing = matches!(&intent, GitIntent::Commit { .. });
        if !self.git_ui.committing {
            self.git_ui.push_after_commit = false;
        }
        self.git_ui.operation_draft = None;
        self.git_ui.confirm_current_action =
            self.service.tool_window_preferences().git_confirm_actions;
        self.git_ui.intent = Some(intent);
        self.git_ui.approved_intent = None;
        self.git_ui.apply_dispatched = false;
        self.git_ui.preview_baseline = self.service.git.preview.as_ref().map(|p| p.id);
        self.git_ui.preview_requested = true;
        true
    }
    pub(super) fn git_cancel_preview(&mut self) {
        self.git_ui.foreground_request = None;
        self.git_ui.preview_requested = false;
        self.git_ui.preview_baseline = None;
        self.git_ui.intent = None;
        self.git_ui.approved_intent = None;
        self.git_ui.committing = false;
        self.git_ui.push_after_commit = false;
    }
    pub(super) fn git_prepare(&mut self, action: LocalAction) {
        let intent = if let LocalAction::Commit { message, .. } = &action {
            GitIntent::Commit {
                message: message.clone(),
                push: self.git_ui.push_after_commit,
            }
        } else {
            GitIntent::Other
        };
        if !self.git_begin_intent(intent) {
            return;
        }
        if !self.service.git.trusted {
            self.git_send(GitCommand::Trust(true));
        }
        self.git_send(GitCommand::Prepare(action));
    }
    pub(super) fn git_action(&mut self, kind: GitActionKind, mut name: String) {
        if !self.service.tool_window_preferences().git_confirm_actions && !name.is_empty() {
            let action = match kind {
                GitActionKind::Checkout => Some(LocalAction::CheckoutRevision {
                    revision: name.clone(),
                }),
                GitActionKind::DeleteBranch => {
                    Some(LocalAction::DeleteBranch { name: name.clone() })
                }
                GitActionKind::DeleteTag => Some(LocalAction::DeleteTag { name: name.clone() }),
                _ => None,
            };
            if let Some(action) = action {
                self.git_prepare(action);
                return;
            }
            use codisseu_core::git::workflows::WorkflowAction as W;
            let workflow = match kind {
                GitActionKind::Merge => Some(W::Merge {
                    revision: name.clone(),
                }),
                GitActionKind::Rebase => Some(W::Rebase { onto: name.clone() }),
                GitActionKind::CherryPick => Some(W::CherryPick {
                    revision: name.clone(),
                }),
                GitActionKind::Revert => Some(W::Revert {
                    revision: name.clone(),
                }),
                _ => None,
            };
            if let Some(action) = workflow {
                self.git_prepare_workflow(action);
                return;
            }
        }
        if !self.service.tool_window_preferences().git_confirm_actions
            && matches!(
                kind,
                GitActionKind::Push | GitActionKind::Pull | GitActionKind::Fetch
            )
            && let Some((remote, source, target)) =
                self.git_quick_remote().filter(|(_, source, _)| {
                    name.is_empty()
                        || &name == source
                        || name.strip_prefix("refs/heads/") == Some(source.as_str())
                })
        {
            use codisseu_core::git::workflows::{PullMode, WorkflowAction as W};
            let action = match kind {
                GitActionKind::Push => W::PushTo {
                    remote,
                    source,
                    target,
                    tags: false,
                },
                GitActionKind::Pull => W::Pull {
                    remote,
                    branch: target,
                    mode: PullMode::FastForwardOnly,
                },
                _ => W::Fetch { remote },
            };
            self.git_prepare_workflow(action);
            return;
        }
        if name.is_empty() && matches!(kind, GitActionKind::Push | GitActionKind::Pull) {
            name = self
                .service
                .git
                .snapshot
                .as_ref()
                .and_then(|s| s.head_ref.as_deref())
                .unwrap_or("")
                .strip_prefix("refs/heads/")
                .unwrap_or("")
                .into();
        }
        let other = if matches!(
            kind,
            GitActionKind::Fetch | GitActionKind::Pull | GitActionKind::Push
        ) {
            "origin".into()
        } else {
            String::new()
        };
        if kind == GitActionKind::Push {
            self.git_ui.dialog = Some(GitDialog::Push {
                remote: other,
                source: name.clone(),
                target: name,
                tags: false,
                tag_mode: 0,
                commit: None,
                requested: false,
            });
            return;
        }
        if kind == GitActionKind::Pull {
            self.git_ui.dialog = Some(GitDialog::Pull {
                remote: other,
                branch: name,
                mode: 0,
            });
            return;
        }
        if kind == GitActionKind::Stash {
            self.git_ui.dialog = Some(GitDialog::Stash {
                message: String::new(),
                keep_index: false,
                untracked: false,
            });
            return;
        }
        if kind == GitActionKind::Stashes {
            self.git_ui.source_control = false;
            self.git_ui.commit_tab = 1;
            self.git_ui.commit_open = true;
            self.drawings_ui.open = false;
            self.show_project = true;
            self.git_request_stashes();
            return;
        }
        self.git_ui.dialog = Some(GitDialog::Action {
            kind,
            name,
            other,
            message: String::new(),
            mode: if kind == GitActionKind::Reset { 1 } else { 0 },
            check: kind == GitActionKind::NewBranch,
        });
        if matches!(kind, GitActionKind::Stashes) {
            self.git_request_stashes();
        }
    }
    pub(super) fn git_open_commit(&mut self) {
        if !self.service.tool_window_preferences().commit {
            self.git_open_source_control();
            return;
        }
        self.api_ui.open = false;
        self.docker_ui.open = false;
        self.git_ui.source_control = false;
        self.git_ui.commit_open = true;
        self.git_ui.focus_commit_tree = true;
        self.git_ui.commit_tab = 0;
        self.drawings_ui.open = false;
        self.show_project = true;
        self.git_send(GitCommand::Refresh);
    }
    pub(super) fn git_request_history(&mut self, refresh: bool) {
        let reference = if !refresh {
            self.service
                .git
                .history
                .as_ref()
                .and_then(|p| p.resolved_reference.clone())
        } else {
            None
        }
        .or_else(|| {
            if self.git_ui.log_branch.is_empty() {
                None
            } else if self.git_ui.log_branch.starts_with("refs/")
                || self.git_ui.log_branch == "HEAD"
                || ((self.git_ui.log_branch.len() == 40 || self.git_ui.log_branch.len() == 64)
                    && self
                        .git_ui
                        .log_branch
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit()))
            {
                Some(self.git_ui.log_branch.clone())
            } else {
                let name = &self.git_ui.log_branch;
                let remote = self
                    .service
                    .git
                    .branches
                    .iter()
                    .any(|b| b.name == *name && b.remote);
                Some(format!(
                    "refs/{}/{}",
                    if remote { "remotes" } else { "heads" },
                    name
                ))
            }
        });
        self.git_send(GitCommand::HistoryReference {
            skip: self.git_ui.page * 200,
            limit: 200,
            path: self.git_ui.history_path.clone(),
            reference,
        });
    }
    pub(super) fn git_open_log(&mut self) {
        self.show_activity = true;
        self.tool_tab = ToolTab::Git;
        self.git_ui.page = 0;
        self.git_ui.history_path = None;
        self.git_request_history(true);
        self.git_send(GitCommand::Branches);
        self.git_send(GitCommand::BranchPreferences);
        self.git_send(GitCommand::Tags);
    }
    pub(super) fn git_open_file_history(&mut self, path: PathBuf) {
        self.show_activity = true;
        self.tool_tab = ToolTab::Git;
        self.git_ui.page = 0;
        self.git_ui.history_path = Some(path);
        self.git_ui.log_branch.clear();
        self.git_request_history(true);
        self.git_send(GitCommand::Branches);
        self.git_send(GitCommand::BranchPreferences);
    }
    pub(super) fn git_open_diff(&mut self, path: PathBuf, side: DiffSide) {
        self.git_ui.commit = None;
        self.git_ui.diff_range = None;
        self.git_ui.file_version_open = false;
        self.git_ui.file = Some(path.clone());
        self.git_ui.selected_hunks.clear();
        self.git_ui.diff_open = true;
        self.git_send_foreground(GitCommand::Diff { path, side });
    }
    pub(super) fn git_open_revision_diff(
        &mut self,
        base: Option<String>,
        head: String,
        path: Option<PathBuf>,
    ) {
        self.git_ui.commit = Some(head.clone());
        self.git_ui.file = path.clone();
        self.git_ui.file_version_open = false;
        self.git_ui.diff_open = true;
        self.git_ui.diff_range = Some((base.clone(), head.clone(), path.clone()));
        self.git_send_foreground(GitCommand::RevisionDiff { base, head, path });
    }
    pub(super) fn poll_git_ui(&mut self) {
        self.git_bind_repository_ui();
        self.git_poll_repository_uis();
        self.poll_selected_git_ui();
    }
    pub(super) fn poll_selected_git_ui(&mut self) {
        if !self.service.git.is_busy()
            || self.git_ui.foreground_request
                != Some((self.service.git.repo_generation, self.service.git.revision))
        {
            self.git_ui.foreground_request = None;
        }
        let root = self.service.git.repo.as_ref().map(|r| r.root.clone());
        let binding_changed = self.git_ui.root != root;
        if binding_changed {
            if let Some(old) = self.git_ui.root.take() {
                self.git_ui.drafts.insert(old, self.git_ui.message.clone());
            }
            self.git_ui.message = root
                .as_ref()
                .and_then(|r| {
                    self.service
                        .git_draft(r)
                        .map(str::to_owned)
                        .or_else(|| self.git_ui.drafts.get(r).cloned())
                })
                .unwrap_or_default();
            self.git_ui.root = root;
            self.git_ui.selected.clear();
            self.git_ui.file = None;
            self.git_ui.commit = None;
            self.git_ui.commit_group = None;
            self.git_ui.focus_commit_tree = false;
            self.git_ui.stash = None;
            self.git_ui.shelf = None;
            self.git_ui.commit_tab = 0;
            self.git_ui.dialog = None;
            self.git_ui.diff_open = false;
            self.git_ui.diff_return = None;
            self.git_ui.queue.clear();
            self.git_ui.amend = false;
            self.git_ui.amend_load_expected = None;
            self.git_ui.amend_backup = None;
            self.git_ui.amend_loaded = None;
            self.git_cancel_preview();
            self.git_ui.applying_intent = None;
            self.git_ui.conflict_draft = None;
            self.git_ui.operation_draft = None;
            self.git_ui.applying = false;
            self.git_ui.apply_dispatched = false;
            self.git_ui.preview_requested = false;
        }
        if self.git_ui.generation != self.service.git.repo_generation {
            self.git_ui.generation = self.service.git.repo_generation;
            self.git_ui.queue.clear();
            self.git_ui.preview_requested = false;
        }
        if binding_changed && self.service.git.repo.is_some() {
            self.git_send(GitCommand::Branches);
            self.git_send(GitCommand::BranchPreferences);
            self.git_send(GitCommand::Tags);
            self.git_send(GitCommand::Remotes);
            self.git_request_shelves();
        }
        if self.git_ui.revision != self.service.git.revision {
            self.git_ui.revision = self.service.git.revision;
            if self.git_ui.amend_load_expected.is_some()
                && let Some(d) = self.service.git.details.as_ref().filter(|d| {
                    self.service
                        .git
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.head.as_ref())
                        == Some(&d.commit)
                })
            {
                let expected = self.git_ui.amend_load_expected.take();
                if self.git_ui.amend && expected.as_ref() == Some(&self.git_ui.message) {
                    self.git_ui.message = d.message.clone();
                    self.git_ui.amend_loaded = Some(d.message.clone());
                }
            }
            if let Some(s) = &self.service.git.snapshot {
                self.git_ui
                    .selected
                    .retain(|p| s.entries.iter().any(|e| &e.path == p));
            }
            if self.git_ui.preview_requested
                && self
                    .service
                    .git
                    .preview
                    .as_ref()
                    .is_some_and(|p| Some(p.id) != self.git_ui.preview_baseline)
            {
                self.git_ui.preview_requested = false;
                let intent = self.git_ui.intent.take().unwrap_or(GitIntent::Other);
                if !self.git_ui.confirm_current_action
                    || matches!(intent, GitIntent::ConfirmedAction)
                {
                    if let Some(preview) = self.service.git.preview.as_ref() {
                        self.git_send(GitCommand::Apply {
                            preview_id: preview.id,
                            repo_generation: preview.generation,
                        });
                        self.git_ui.applying = true;
                        self.git_ui.applying_intent = Some(intent);
                        self.git_ui.dialog = None;
                    }
                } else {
                    self.git_ui.approved_intent =
                        self.service.git.preview.as_ref().map(|p| (p.id, intent));
                    self.git_ui.dialog = Some(GitDialog::Preview);
                }
            }
            if self.git_ui.applying && self.git_ui.apply_dispatched && !self.service.git.is_busy() {
                self.git_ui.applying = false;
                self.git_ui.apply_dispatched = false;
                self.git_ui.selected_hunks.clear();
                if let Some(report) = &self.service.git.report {
                    self.git_ui
                        .output
                        .push_str(&format!("{}\n{}\n", report.summary, report.output));
                    if self.git_ui.output.len() > 256 * 1024 {
                        self.git_ui.output = self
                            .git_ui
                            .output
                            .chars()
                            .rev()
                            .take(128 * 1024)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect();
                    }
                }
                if self.service.git.report.as_ref().is_some_and(|r| r.success) {
                    let intent = self.git_ui.applying_intent.take();
                    let sync_push = matches!(&intent, Some(GitIntent::SyncPull { root }) if self.service.git.repo.as_ref().is_some_and(|r| &r.root == root));
                    let push = if let Some(GitIntent::Commit { message, push }) = intent {
                        if self.git_ui.message == message {
                            self.git_ui.message.clear();
                            self.git_ui.amend = false;
                            self.git_ui.amend_load_expected = None;
                            self.git_ui.amend_backup = None;
                            self.git_ui.amend_loaded = None;
                        }
                        push
                    } else {
                        false
                    };
                    let next_conflict =
                        if self.git_ui.merge_apply_next && self.git_ui.conflict_stage {
                            self.git_ui.conflict_draft.as_ref().and_then(|draft| {
                                if let GitDialog::Conflict { path, .. } = draft {
                                    self.service
                                        .git
                                        .snapshot
                                        .as_ref()
                                        .and_then(|s| {
                                            s.entries.iter().find(|e| e.conflict && &e.path != path)
                                        })
                                        .map(|e| e.path.clone())
                                } else {
                                    None
                                }
                            })
                        } else {
                            None
                        };
                    self.git_ui.conflict_draft = None;
                    self.git_ui.operation_draft = None;
                    self.git_ui.committing = false;
                    self.git_ui.push_after_commit = false;
                    self.git_ui.notice.clear();
                    self.git_ui.page = 0;
                    self.git_request_history(true);
                    self.git_send(GitCommand::Branches);
                    self.git_send(GitCommand::Tags);
                    self.git_send(GitCommand::Remotes);
                    self.git_send(GitCommand::BranchPreferences);
                    self.git_request_shelves();
                    if self.git_ui.commit_tab == 1 {
                        self.git_request_stashes();
                    }
                    if self.git_ui.diff_open
                        && !self.git_ui.file_version_open
                        && self.git_ui.commit.is_none()
                        && let Some(diff) = self.service.git.diff.as_ref()
                    {
                        self.git_send(GitCommand::Diff {
                            path: diff.path.clone(),
                            side: diff.side,
                        });
                    }
                    if let Some(path) = next_conflict {
                        self.git_open_conflict(path);
                    }
                    if push || sync_push {
                        self.git_action(GitActionKind::Push, String::new());
                    }
                } else {
                    if self.service.git.shelf_report.is_some() {
                        self.git_request_shelves();
                    }
                    self.git_ui.applying_intent = None;
                    self.git_ui.committing = false;
                    self.git_ui.push_after_commit = false;
                    if let Some(draft) = self.git_ui.conflict_draft.take() {
                        self.git_ui.dialog = Some(draft);
                    } else if let Some(draft) = self.git_ui.operation_draft.take() {
                        self.git_ui.dialog = Some(*draft);
                    }
                }
            }
            if self.git_ui.preview_requested
                && self.service.git.error.is_some()
                && !self.service.git.is_busy()
            {
                self.git_cancel_preview();
                if let Some(draft) = self.git_ui.operation_draft.take() {
                    self.git_ui.dialog = Some(*draft);
                }
            }
        }
        if !self.service.git.is_busy()
            && self
                .git_ui
                .queue
                .front()
                .is_none_or(|request| !self.service.git_file_operations_pending(&request.command))
            && let Some(request) = self.git_ui.queue.pop_front()
        {
            let command = request.command;
            let applies = matches!(
                &command,
                GitCommand::Apply { .. } | GitCommand::WorkflowApply { .. }
            );
            match self.service.dispatch(Command::Git(command)) {
                Ok(()) => {
                    self.git_ui.foreground_request = (request.foreground
                        && self.service.git.is_busy())
                    .then_some((self.service.git.repo_generation, self.service.git.revision));
                    if applies {
                        self.git_ui.applying = true;
                        self.git_ui.apply_dispatched = true;
                    }
                }
                Err(error) => {
                    self.git_ui.notice = error;
                    self.git_ui.queue.clear();
                    self.git_cancel_preview();
                    self.git_ui.applying = false;
                    self.git_ui.apply_dispatched = false;
                    self.git_ui.applying_intent = None;
                    if let Some(draft) = self.git_ui.operation_draft.take() {
                        self.git_ui.dialog = Some(*draft);
                    }
                }
            }
        }
        self.git_poll_source_control();
    }
    pub(super) fn sync_git_draft(&mut self) {
        if let Some(root) = self.git_ui.root.clone()
            && self.service.git_draft(&root).unwrap_or("") != self.git_ui.message
            && let Err(error) = self.service.dispatch(Command::SetGitDraft {
                root,
                message: self.git_ui.message.clone(),
            })
        {
            self.git_ui.notice = error;
        }
    }
    pub(super) fn git_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.git_ui.dialog.is_some() || self.git_ui.diff_open {
            return true;
        }
        if ctx.memory(|m| m.focused() == Some(egui::Id::unique("git.commit.message")))
            && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter))
        {
            self.git_commit(false);
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::T)) {
            self.git_action(GitActionKind::Pull, String::new());
            return true;
        }
        if self.git_ui.commit_open
            && !self.editor_focused(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::D))
        {
            if let Some(path) = self.git_ui.file.clone() {
                self.git_open_diff(path, DiffSide::Working);
            }
            return true;
        }
        if !self.editor_focused(ctx)
            && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::K))
        {
            self.git_action(GitActionKind::Push, String::new());
            return true;
        }
        if (!self.editor_focused(ctx) && ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::K)))
            || ctx.input_mut(|i| i.consume_key(Modifiers::CTRL.plus(Modifiers::ALT), Key::K))
        {
            self.git_open_commit();
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::Num9)) {
            self.git_open_log();
            return true;
        }
        false
    }
    pub(super) fn git_commit(&mut self, push: bool) {
        let paths = self.git_ui.selected.iter().cloned().collect::<Vec<_>>();
        let selection = if self.git_ui.staging {
            if paths.is_empty() {
                CommitSelection::AllStaged
            } else {
                CommitSelection::Paths(paths)
            }
        } else {
            CommitSelection::WorkingPaths(paths)
        };
        self.git_ui.notice.clear();
        self.git_ui.push_after_commit = push;
        self.git_prepare(LocalAction::Commit {
            selection,
            message: self.git_ui.message.clone(),
            amend: self.git_ui.amend,
        });
    }
    pub(super) fn git_status_line(&self, ui: &mut egui::Ui) {
        self.git_status_line_with_progress(ui, true);
    }
    pub(super) fn git_status_line_with_progress(&self, ui: &mut egui::Ui, _show_progress: bool) {
        if self.service.git.error.is_none() && self.git_ui.notice.is_empty() {
            return;
        }
        egui::Frame::NONE
            .inner_margin(self.tokens.content_margin())
            .show(ui, |ui| {
                if let Some(error) = &self.service.git.error {
                    crate::status_messages::status_message(ui, error.to_string());
                }
                if !self.git_ui.notice.is_empty() {
                    crate::status_messages::status_message(ui, &self.git_ui.notice);
                }
            });
    }
}

/// Original native row geometry; the reference's list labels align to the tree inset.
pub(super) fn git_text_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    text: &str,
    selected: bool,
    width: f32,
    height: f32,
    inset: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, text));
    if selected {
        ui.painter()
            .rect_filled(rect, 0., color(tokens.database_services_selection));
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0., color(tokens.border));
    }
    ui.painter()
        .with_clip_rect(rect.shrink2(egui::vec2(2., 0.)))
        .text(
            egui::pos2(rect.left() + inset, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(tokens.ui_font_size),
            color(tokens.text),
        );
    response
}
