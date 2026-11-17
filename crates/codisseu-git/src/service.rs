use crate::{
    conflicts::{self, ConflictFile, ConflictSnapshot, PreparedResolution, ResolutionChoice},
    local::{self, PreparedLocal},
    repository::args,
    workflows::{self, PreparedWorkflow, RemoteInfo, StashEntry, WorkflowAction},
    *,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    time::Duration,
};
// A preview handle must never alias one prepared by a sibling worker.
static NEXT_REPOSITORY_GENERATION: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Debug)]
pub enum GitCommand {
    /// Replace the workspace scope and discard the previous repository binding.
    OpenRoots {
        workspace: PathBuf,
        roots: Vec<PathBuf>,
    },
    DiscoverRepositories {
        workspace: PathBuf,
        max_depth: usize,
        max_repositories: usize,
        max_entries: usize,
    },
    DiscoverRoots {
        workspace: PathBuf,
        roots: Vec<PathBuf>,
    },
    BranchPreferences,
    SetBranchFavorite {
        reference: String,
        favorite: bool,
    },
    OpenWithin {
        workspace: PathBuf,
        path: PathBuf,
        trusted: bool,
    },
    Open {
        path: PathBuf,
        trusted: bool,
    },
    Init {
        path: PathBuf,
        initial_branch: String,
    },
    Clone {
        url: String,
        destination: PathBuf,
    },
    Trust(bool),
    Cancel,
    Refresh,
    Diff {
        path: PathBuf,
        side: DiffSide,
    },
    History {
        skip: usize,
        limit: usize,
        path: Option<PathBuf>,
    },
    HistoryReference {
        skip: usize,
        limit: usize,
        path: Option<PathBuf>,
        reference: Option<String>,
    },
    PushReview {
        remote: String,
        source: String,
        target: String,
    },
    RebaseRead {
        base: String,
    },
    RebasePrepare {
        base: String,
        head: String,
        steps: Vec<RebaseStep>,
    },
    Shelves,
    ShelfDeleted,
    Changelists,
    ShelfDeletedDetails {
        id: String,
    },
    ShelfDetails {
        id: String,
    },
    ShelfPrepare(ShelfAction),
    ScmPrepare(ScmAction),
    StashDetails {
        stash: String,
    },
    RevisionDiff {
        base: Option<String>,
        head: String,
        path: Option<PathBuf>,
    },
    Branches,
    Tags,
    Remotes,
    Stashes,
    Blame {
        path: PathBuf,
        start: usize,
        lines: usize,
    },
    FileAt {
        commit: String,
        path: PathBuf,
    },
    CommitDetails {
        commit: String,
    },
    HunksPrepare {
        path: PathBuf,
        hunks: Vec<usize>,
        unstage: bool,
        expected_patch: Vec<u8>,
    },
    Prepare(LocalAction),
    WorkflowPrepare(WorkflowAction),
    ConflictRead {
        path: PathBuf,
    },
    ConflictPrepare {
        path: PathBuf,
        choice: ResolutionChoice,
        stage: bool,
    },
    Apply {
        preview_id: u64,
        repo_generation: u64,
    },
    WorkflowApply {
        preview_id: u64,
        repo_generation: u64,
    },
}
enum Prepared {
    Scm(Box<crate::scm::PreparedScm>),
    Local(Box<PreparedLocal>),
    Workflow(Box<PreparedWorkflow>),
    Resolution(Box<PreparedResolution>),
    Push(Box<crate::sync::PreparedPush>),
    Rebase(Box<crate::rebase::PreparedRebase>),
    Shelf(Box<crate::shelf::PreparedShelf>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewEffect {
    IndexOnly,
    RequiresCleanBuffers,
}
enum Payload {
    Repositories(RepositoryDiscovery),
    BranchPreferences(BranchPreferences),
    Opened(RepositoryInfo, RepoSnapshot),
    Snapshot(RepoSnapshot),
    Diff(GitDiff),
    History(HistoryPage),
    Branches(Vec<BranchInfo>),
    Tags(Vec<TagInfo>),
    Remotes(Vec<RemoteInfo>),
    Stashes(Vec<StashEntry>),
    PushReview(PushReview),
    RebaseSeries(RebaseSeries),
    Shelves(Vec<ShelfInfo>),
    ShelfDeleted(Vec<ShelfInfo>),
    Changelists(Vec<Changelist>),
    ShelfDetails(ShelfDetails),
    StashDetails(StashDetails),
    RevisionDiff(RevisionDiff),
    Blame(Vec<BlameLine>),
    File(FileVersion),
    Details(CommitDetails),
    Preview(GitPreview, PreviewEffect),
    Conflict(ConflictFile),
    Report(
        GitReport,
        Option<RepoSnapshot>,
        Option<crate::shelf::ShelfReport>,
    ),
}
struct Event {
    generation: u64,
    result: Result<Payload, GitError>,
}
struct DiffInspection {
    epoch: u64,
    path: PathBuf,
    side: DiffSide,
    patch: Vec<u8>,
}
struct Job {
    generation: u64,
    command: GitCommand,
}
pub struct GitService {
    pub repositories: Vec<RepositorySummary>,
    pub discovery_truncated: bool,
    pub discovery_workspace: Option<PathBuf>,
    pub discovery_errors: Vec<crate::discovery::RepositoryDiscoveryError>,
    pub branch_preferences: BranchPreferences,
    pub repo: Option<RepositoryInfo>,
    pub snapshot: Option<RepoSnapshot>,
    pub diff: Option<GitDiff>,
    pub history: Option<HistoryPage>,
    pub branches: Vec<BranchInfo>,
    pub tags: Vec<TagInfo>,
    pub remotes: Vec<RemoteInfo>,
    pub stashes: Vec<StashEntry>,
    pub push_review: Option<PushReview>,
    pub rebase_series: Option<RebaseSeries>,
    pub shelves: Vec<ShelfInfo>,
    pub deleted_shelves: Vec<ShelfInfo>,
    pub changelists: Vec<Changelist>,
    pub shelf_details: Option<ShelfDetails>,
    pub stash_details: Option<StashDetails>,
    pub revision_diff: Option<RevisionDiff>,
    pub shelf_report: Option<crate::shelf::ShelfReport>,
    pub blame: Vec<BlameLine>,
    pub file: Option<FileVersion>,
    pub details: Option<CommitDetails>,
    pub conflict: Option<ConflictFile>,
    pub preview: Option<GitPreview>,
    // Authoritative operation classification is private and bound to its token.
    index_only_preview: Option<(u64, u64)>,
    pub report: Option<GitReport>,
    pub error: Option<String>,
    pub status: String,
    pub revision: u64,
    pub repo_generation: u64,
    pub trusted: bool,
    sender: Option<SyncSender<Job>>,
    receiver: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    trust: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    busy: bool,
    reading_history: bool,
    cancellable_read: bool,
}
impl Default for GitService {
    fn default() -> Self {
        Self::new()
    }
}
impl GitService {
    /// Merge status replies from another repository worker without replacing the
    /// workspace discovery catalog. Keep the existing shared cache budget.
    pub fn merge_repository_summaries(&mut self, summaries: Vec<RepositorySummary>) {
        if summaries.is_empty() {
            return;
        }
        for summary in summaries {
            if let Some(existing) = self
                .repositories
                .iter_mut()
                .find(|r| r.root == summary.root)
            {
                *existing = summary;
            } else if self.repositories.len() < 32 {
                self.repositories.push(summary);
            }
        }
        if let Some(root) = self
            .repo
            .as_ref()
            .map(|r| r.root.clone())
            .or_else(|| self.repositories.first().map(|r| r.root.clone()))
        {
            crate::discovery::bound_status_cache(&mut self.repositories, &root);
        }
    }
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        let (etx, erx) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let trust = Arc::new(AtomicBool::new(false));
        let epoch = Arc::new(AtomicU64::new(0));
        let (c, t, e) = (cancel.clone(), trust.clone(), epoch.clone());
        std::thread::Builder::new()
            .name("codisseu-git".into())
            .spawn(move || worker(rx, etx, c, t, e))
            .expect("Git worker thread");
        Self {
            repositories: vec![],
            discovery_truncated: false,
            discovery_workspace: None,
            discovery_errors: Vec::new(),
            branch_preferences: BranchPreferences::default(),
            repo: None,
            snapshot: None,
            diff: None,
            history: None,
            branches: vec![],
            tags: vec![],
            remotes: vec![],
            stashes: vec![],
            push_review: None,
            rebase_series: None,
            shelves: vec![],
            deleted_shelves: vec![],
            changelists: vec![],
            shelf_details: None,
            stash_details: None,
            revision_diff: None,
            shelf_report: None,
            blame: vec![],
            file: None,
            details: None,
            conflict: None,
            preview: None,
            index_only_preview: None,
            report: None,
            error: None,
            status: "Git não vinculado".into(),
            revision: 0,
            repo_generation: 0,
            trusted: false,
            sender: Some(tx),
            receiver: erx,
            cancel,
            trust,
            epoch,
            busy: false,
            reading_history: false,
            cancellable_read: false,
        }
    }
    fn update_repository_summary(&mut self, repo: &RepositoryInfo, snapshot: &RepoSnapshot) {
        let summary = RepositorySummary::from_snapshot(repo, snapshot);
        if let Some(existing) = self
            .repositories
            .iter_mut()
            .find(|item| item.root == repo.root)
        {
            *existing = summary;
        } else if self.repositories.len() < 32 {
            self.repositories.push(summary);
        }
        crate::discovery::bound_status_cache(&mut self.repositories, &repo.root);
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    /// Only speculative reads may be cancelled automatically during shutdown.
    /// Writes (including Init/Clone/preferences) retain their real ACK even when
    /// they are not covered by the workspace's Apply guard.
    pub fn cancellable_read_pending(&self) -> bool {
        self.busy && self.cancellable_read
    }
    /// A replacement page is being read; `history` remains the last accepted page.
    pub fn history_pending(&self) -> bool {
        self.reading_history
    }
    /// Only whole-file Stage/Unstage can run with an unsaved editor buffer.
    /// Apply consults private worker-derived metadata, never preview text or
    /// caller-writable presentation fields.
    pub fn command_changes_only_index(&self, command: &GitCommand) -> bool {
        match command {
            GitCommand::Prepare(LocalAction::Stage { .. } | LocalAction::Unstage { .. }) => true,
            GitCommand::Apply {
                preview_id,
                repo_generation,
            }
            | GitCommand::WorkflowApply {
                preview_id,
                repo_generation,
            } => {
                self.index_only_preview == Some((*preview_id, *repo_generation))
                    && *repo_generation == self.repo_generation
                    && self.preview.as_ref().is_some_and(|preview| {
                        preview.id == *preview_id && preview.generation == *repo_generation
                    })
            }
            _ => false,
        }
    }
    fn clear_preview(&mut self) {
        self.preview = None;
        self.index_only_preview = None;
    }
    pub fn dispatch(&mut self, command: GitCommand) -> Result<(), GitError> {
        if matches!(command, GitCommand::Cancel) {
            self.cancel.store(true, Ordering::SeqCst);
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.clear_preview();
            self.status = "Cancelamento Git solicitado".into();
            return Ok(());
        }
        if let GitCommand::Trust(value) = command {
            if self.busy {
                return Err(GitError::invalid(
                    "Aguarde a operação Git antes de alterar a confiança",
                ));
            }
            if self.trusted == value {
                return Ok(());
            }
            self.trusted = value;
            self.trust.store(value, Ordering::SeqCst);
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.clear_preview();
            return Ok(());
        }
        if self.busy {
            return Err(GitError::invalid("Há uma operação Git em andamento"));
        }
        if let GitCommand::HunksPrepare { expected_patch, .. } = &command
            && expected_patch.len() > 8 * 1024 * 1024
        {
            return Err(GitError::invalid("Patch exibido excede8MiB"));
        }
        let opening = matches!(
            command,
            GitCommand::Open { .. }
                | GitCommand::OpenWithin { .. }
                | GitCommand::Init { .. }
                | GitCommand::Clone { .. }
                | GitCommand::OpenRoots { .. }
        );
        let mutating = matches!(
            command,
            GitCommand::Prepare(_)
                | GitCommand::WorkflowPrepare(_)
                | GitCommand::ConflictPrepare { .. }
                | GitCommand::ShelfPrepare(_)
                | GitCommand::ScmPrepare(_)
                | GitCommand::HunksPrepare { .. }
                | GitCommand::RebasePrepare { .. }
                | GitCommand::PushReview { .. }
                | GitCommand::Apply { .. }
                | GitCommand::WorkflowApply { .. }
        );
        if mutating && !self.trusted {
            return Err(GitError::invalid(
                "Autorize as operações Git deste repositório antes de continuar",
            ));
        }
        if let GitCommand::Apply {
            preview_id,
            repo_generation,
        }
        | GitCommand::WorkflowApply {
            preview_id,
            repo_generation,
        } = &command
            && (*repo_generation != self.repo_generation
                || !self
                    .preview
                    .as_ref()
                    .is_some_and(|p| p.id == *preview_id && p.generation == *repo_generation))
        {
            return Err(GitError::new(
                GitErrorKind::Stale,
                "Prévia Git expirada ou de outro repositório",
            ));
        }
        if opening {
            self.repo_generation = NEXT_REPOSITORY_GENERATION.fetch_add(1, Ordering::Relaxed);
            self.clear();
            if matches!(command, GitCommand::OpenRoots { .. }) {
                self.repositories.clear();
                self.discovery_errors.clear();
                self.discovery_workspace = None;
                self.discovery_truncated = false;
            }
            let trusted = match &command {
                GitCommand::Open { trusted, .. } | GitCommand::OpenWithin { trusted, .. } => {
                    *trusted
                }
                GitCommand::OpenRoots { .. } => false,
                _ => true,
            };
            self.trusted = trusted;
            self.trust.store(trusted, Ordering::SeqCst);
            self.epoch.fetch_add(1, Ordering::SeqCst);
        } else if self.repo.is_none()
            && !matches!(
                command,
                GitCommand::DiscoverRepositories { .. } | GitCommand::DiscoverRoots { .. }
            )
        {
            return Err(GitError::invalid("Abra um repositório Git"));
        }
        match &command {
            GitCommand::Prepare(_)
            | GitCommand::WorkflowPrepare(_)
            | GitCommand::ConflictPrepare { .. }
            | GitCommand::ShelfPrepare(_)
            | GitCommand::ScmPrepare(_)
            | GitCommand::HunksPrepare { .. }
            | GitCommand::RebasePrepare { .. } => self.clear_preview(),
            GitCommand::PushReview { .. } => self.push_review = None,
            GitCommand::RebaseRead { .. } => self.rebase_series = None,
            GitCommand::ShelfDetails { .. } | GitCommand::ShelfDeletedDetails { .. } => {
                self.shelf_details = None
            }
            GitCommand::StashDetails { .. } => self.stash_details = None,
            GitCommand::RevisionDiff { .. } => self.revision_diff = None,
            GitCommand::Diff { .. } => self.diff = None,
            GitCommand::CommitDetails { .. } => self.details = None,
            GitCommand::ConflictRead { .. } => self.conflict = None,
            GitCommand::FileAt { .. } => self.file = None,
            GitCommand::Blame { .. } => self.blame.clear(),
            _ => {}
        }
        let cancellable_read = matches!(
            command,
            GitCommand::OpenRoots { .. }
                | GitCommand::DiscoverRepositories { .. }
                | GitCommand::DiscoverRoots { .. }
                | GitCommand::BranchPreferences
                | GitCommand::OpenWithin { .. }
                | GitCommand::Open { .. }
                | GitCommand::Refresh
                | GitCommand::Diff { .. }
                | GitCommand::History { .. }
                | GitCommand::HistoryReference { .. }
                | GitCommand::PushReview { .. }
                | GitCommand::RebaseRead { .. }
                | GitCommand::Shelves
                | GitCommand::ShelfDeleted
                | GitCommand::Changelists
                | GitCommand::ShelfDeletedDetails { .. }
                | GitCommand::ShelfDetails { .. }
                | GitCommand::StashDetails { .. }
                | GitCommand::RevisionDiff { .. }
                | GitCommand::Branches
                | GitCommand::Tags
                | GitCommand::Remotes
                | GitCommand::Stashes
                | GitCommand::Blame { .. }
                | GitCommand::FileAt { .. }
                | GitCommand::CommitDetails { .. }
                | GitCommand::ConflictRead { .. }
        );
        let reading_history = matches!(
            command,
            GitCommand::History { .. } | GitCommand::HistoryReference { .. }
        );
        self.cancel.store(false, Ordering::SeqCst);
        let queued = self
            .sender
            .as_ref()
            .ok_or_else(|| GitError::invalid("Worker Git encerrado"))
            .and_then(|sender| {
                sender
                    .try_send(Job {
                        generation: self.repo_generation,
                        command,
                    })
                    .map_err(|_| GitError::invalid("Fila Git ocupada ou encerrada"))
            });
        if let Err(error) = queued {
            if reading_history {
                self.history = None;
            }
            return Err(error);
        }
        self.busy = true;
        self.cancellable_read = cancellable_read;
        self.reading_history = reading_history;
        self.error = None;
        self.report = None;
        self.status = "Executando Git…".into();
        Ok(())
    }
    fn clear(&mut self) {
        self.reading_history = false;
        self.branch_preferences = BranchPreferences::default();
        self.repo = None;
        self.snapshot = None;
        self.diff = None;
        self.history = None;
        self.branches.clear();
        self.tags.clear();
        self.remotes.clear();
        self.stashes.clear();
        self.push_review = None;
        self.rebase_series = None;
        self.shelves.clear();
        self.deleted_shelves.clear();
        self.changelists.clear();
        self.shelf_details = None;
        self.stash_details = None;
        self.revision_diff = None;
        self.shelf_report = None;
        self.blame.clear();
        self.file = None;
        self.details = None;
        self.conflict = None;
        self.clear_preview();
        self.report = None;
        self.error = None;
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = match self.receiver.try_recv() {
                Ok(event) => event,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.busy {
                        self.busy = false;
                        if self.reading_history {
                            self.history = None;
                        }
                        self.reading_history = false;
                        self.error = Some("Worker Git encerrado".into());
                        changed = true;
                    }
                    break;
                }
            };
            if event.generation != self.repo_generation {
                continue;
            }
            self.busy = false;
            let reading_history = std::mem::take(&mut self.reading_history);
            changed = true;
            self.revision = self.revision.wrapping_add(1);
            // A completed page can still be queued when the user cancels its
            // pending presentation. Do not publish it as the newly selected ref.
            // Mutation reports retain their real outcome even after cancellation.
            let result = if reading_history {
                crate::repository::cancelled(&self.cancel).and(event.result)
            } else {
                event.result
            };
            let payload = match result {
                Ok(payload) => payload,
                Err(error) => {
                    if reading_history {
                        self.history = None;
                    }
                    self.error = Some(error.to_string());
                    self.status = error.to_string();
                    self.clear_preview();
                    continue;
                }
            };
            self.status = "Git atualizado".into();
            match payload {
                Payload::Repositories(discovery) => {
                    self.repositories = discovery.repositories;
                    self.discovery_truncated = discovery.truncated;
                    self.discovery_workspace = Some(discovery.workspace);
                    self.discovery_errors = discovery.errors;
                }
                Payload::BranchPreferences(preferences) => self.branch_preferences = preferences,
                Payload::Opened(repo, snapshot) => {
                    self.update_repository_summary(&repo, &snapshot);
                    self.repo = Some(repo);
                    self.snapshot = Some(snapshot);
                }
                Payload::Snapshot(s) => {
                    if let Some(repo) = self.repo.clone() {
                        self.update_repository_summary(&repo, &s);
                    }
                    self.snapshot = Some(s);
                    self.clear_preview();
                }
                Payload::Diff(d) => self.diff = Some(d),
                Payload::History(h) => self.history = Some(h),
                Payload::Branches(b) => self.branches = b,
                Payload::Tags(t) => self.tags = t,
                Payload::Remotes(r) => self.remotes = r,
                Payload::Stashes(s) => self.stashes = s,
                Payload::PushReview(r) => self.push_review = Some(r),
                Payload::RebaseSeries(r) => self.rebase_series = Some(r),
                Payload::Shelves(s) => self.shelves = s,
                Payload::ShelfDeleted(s) => self.deleted_shelves = s,
                Payload::Changelists(c) => self.changelists = c,
                Payload::ShelfDetails(s) => self.shelf_details = Some(s),
                Payload::StashDetails(s) => self.stash_details = Some(s),
                Payload::RevisionDiff(d) => self.revision_diff = Some(d),
                Payload::Blame(b) => self.blame = b,
                Payload::File(f) => self.file = Some(f),
                Payload::Details(d) => self.details = Some(d),
                Payload::Conflict(c) => self.conflict = Some(c),
                Payload::Preview(p, effect) => {
                    self.status = p.summary.clone();
                    self.index_only_preview =
                        (effect == PreviewEffect::IndexOnly).then_some((p.id, p.generation));
                    self.preview = Some(p);
                }
                Payload::Report(r, s, shelf) => {
                    if let Some(shelf) = shelf {
                        self.shelf_report = Some(shelf);
                    }
                    self.status = r.summary.clone();
                    if !r.success {
                        self.error = Some(r.summary.clone());
                    }
                    self.report = Some(r);
                    if let Some(s) = s {
                        if let Some(repo) = self.repo.clone() {
                            self.update_repository_summary(&repo, &s);
                        }
                        self.snapshot = Some(s);
                    }
                    self.clear_preview();
                }
            }
        }
        changed
    }
}
impl Drop for GitService {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
        self.sender.take();
    }
}
fn worker(
    receiver: Receiver<Job>,
    sender: SyncSender<Event>,
    cancel: Arc<AtomicBool>,
    trust: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
) {
    let mut repo: Option<Repository> = None;
    let mut prepared: Option<(u64, u64, u64, Prepared)> = None;
    let mut conflict: Option<ConflictSnapshot> = None;
    let mut push_inspection: Option<(u64, crate::sync::PushInspection)> = None;
    let mut rebase_inspection: Option<(u64, crate::rebase::RebaseInspection)> = None;
    let mut diff_inspection: Option<DiffInspection> = None;
    let mut next_id = 0u64;
    while let Ok(job) = receiver.recv() {
        let generation = job.generation;
        let result = (|| -> Result<Payload, GitError> {
            match job.command {
                GitCommand::OpenRoots { workspace, roots } => {
                    repo = None;
                    diff_inspection = None;
                    push_inspection = None;
                    rebase_inspection = None;
                    prepared = None;
                    conflict = None;
                    return Ok(Payload::Repositories(crate::discovery::discover_roots(
                        &workspace, &roots, &cancel,
                    )?));
                }
                GitCommand::DiscoverRoots { workspace, roots } => {
                    return Ok(Payload::Repositories(crate::discovery::discover_roots(
                        &workspace, &roots, &cancel,
                    )?));
                }
                GitCommand::DiscoverRepositories {
                    workspace,
                    max_depth,
                    max_repositories,
                    max_entries,
                } => {
                    return Ok(Payload::Repositories(
                        crate::discovery::discover_repositories(
                            &workspace,
                            max_depth,
                            max_repositories,
                            max_entries,
                            &cancel,
                        )?,
                    ));
                }
                command @ (GitCommand::Open { .. } | GitCommand::OpenWithin { .. }) => {
                    let (path, workspace) = match command {
                        GitCommand::Open { path, .. } => (path, None),
                        GitCommand::OpenWithin {
                            path, workspace, ..
                        } => (path, Some(workspace)),
                        _ => unreachable!(),
                    };
                    repo = None;
                    diff_inspection = None;
                    push_inspection = None;
                    rebase_inspection = None;
                    prepared = None;
                    conflict = None;
                    let r = Repository::discover(&path, &cancel)?;
                    if let Some(workspace) = workspace {
                        let workspace = std::fs::canonicalize(workspace)?;
                        let requested = std::fs::canonicalize(&path)?;
                        if requested != r.root
                            || (!r.root.starts_with(&workspace)
                                && Repository::discover(&workspace, &cancel)
                                    .map_or(true, |ancestor| ancestor.root != r.root))
                        {
                            return Err(GitError::invalid(
                                "Selecione um repositório dentro da pasta atual",
                            ));
                        }
                    }
                    let s = r.snapshot(&cancel)?;
                    let info = r.info();
                    repo = Some(r);
                    return Ok(Payload::Opened(info, s));
                }
                GitCommand::Init {
                    path,
                    initial_branch,
                } => {
                    repo = None;
                    diff_inspection = None;
                    push_inspection = None;
                    rebase_inspection = None;
                    prepared = None;
                    conflict = None;
                    if !path.is_dir() {
                        return Err(GitError::invalid(
                            "Escolha um diretório existente para iniciar Git",
                        ));
                    }
                    if Repository::discover(&path, &cancel).is_ok() {
                        return Err(GitError::invalid(
                            "O diretório já pertence a um repositório Git",
                        ));
                    }
                    if initial_branch.starts_with('-')
                        || initial_branch.is_empty()
                        || initial_branch.contains('\0')
                    {
                        return Err(GitError::invalid("Branch inicial inválida"));
                    }
                    crate::repository::checked(crate::process::run(
                        &path,
                        &args(&["check-ref-format", "--branch", &initial_branch]),
                        None,
                        &cancel,
                        Duration::from_secs(15),
                        &[],
                    )?)?;
                    crate::repository::checked(crate::process::run(
                        &path,
                        &args(&["init", "--initial-branch", &initial_branch, "--", "."]),
                        None,
                        &cancel,
                        Duration::from_secs(30),
                        &[],
                    )?)?;
                    let r = Repository::discover(&path, &cancel)?;
                    let s = r.snapshot(&cancel)?;
                    let info = r.info();
                    repo = Some(r);
                    return Ok(Payload::Opened(info, s));
                }
                GitCommand::Clone { url, destination } => {
                    repo = None;
                    diff_inspection = None;
                    push_inspection = None;
                    rebase_inspection = None;
                    prepared = None;
                    conflict = None;
                    if url.is_empty()
                        || url.starts_with('-')
                        || url.contains('\0')
                        || url.len() > 4096
                        || url.starts_with("ext::")
                        || url
                            .split("://")
                            .nth(1)
                            .is_some_and(|v| v.split('/').next().unwrap_or("").contains('@'))
                    {
                        return Err(GitError::invalid(
                            "URL inválida ou contém credenciais; use a configuração segura do Git",
                        ));
                    }
                    if destination.exists() {
                        return Err(GitError::invalid(
                            "Destino de clone já existe; escolha um novo diretório",
                        ));
                    }
                    let parent = destination
                        .parent()
                        .filter(|p| p.is_dir())
                        .ok_or_else(|| GitError::invalid("Diretório pai do clone não existe"))?;
                    let mut a = args(&["clone", "--", &url]);
                    a.push(destination.as_os_str().into());
                    crate::repository::checked(crate::process::run(
                        parent,
                        &a,
                        None,
                        &cancel,
                        Duration::from_secs(120),
                        &[],
                    )?)?;
                    let r = Repository::discover(&destination, &cancel)?;
                    let s = r.snapshot(&cancel)?;
                    let info = r.info();
                    repo = Some(r);
                    return Ok(Payload::Opened(info, s));
                }
                _ => {}
            }
            let r = repo
                .as_ref()
                .ok_or_else(|| GitError::invalid("Repositório Git não vinculado"))?;
            match job.command {
                GitCommand::BranchPreferences => Ok(Payload::BranchPreferences(
                    crate::preferences::load(r, &cancel)?,
                )),
                GitCommand::SetBranchFavorite {
                    reference,
                    favorite,
                } => Ok(Payload::BranchPreferences(
                    crate::preferences::set_favorite(r, &reference, favorite, &cancel)?,
                )),
                GitCommand::Refresh => {
                    prepared = None;
                    conflict = None;
                    Ok(Payload::Snapshot(r.snapshot(&cancel)?))
                }
                GitCommand::Diff { path, side } => {
                    diff_inspection = None;
                    let before = r.snapshot(&cancel)?;
                    let diff = r.diff(&path, side, &cancel)?;
                    if r.snapshot(&cancel)? != before {
                        return Err(GitError::new(
                            GitErrorKind::Stale,
                            "Arquivo mudou durante a leitura do diff",
                        ));
                    }
                    diff_inspection = Some(DiffInspection {
                        epoch: epoch.load(Ordering::SeqCst),
                        path: diff.path.clone(),
                        side,
                        patch: diff.bytes.clone(),
                    });
                    Ok(Payload::Diff(diff))
                }
                GitCommand::History { skip, limit, path } => Ok(Payload::History(r.history(
                    skip,
                    limit,
                    path.as_deref(),
                    &cancel,
                )?)),
                GitCommand::HistoryReference {
                    skip,
                    limit,
                    path,
                    reference,
                } => Ok(Payload::History(r.history_reference(
                    skip,
                    limit,
                    path.as_deref(),
                    reference.as_deref(),
                    &cancel,
                )?)),
                GitCommand::PushReview {
                    remote,
                    source,
                    target,
                } => {
                    push_inspection = None;
                    let p = crate::sync::inspect_push(r, &remote, &source, &target, &cancel)?;
                    let view = p.review().clone();
                    push_inspection = Some((epoch.load(Ordering::SeqCst), p));
                    Ok(Payload::PushReview(view))
                }
                GitCommand::RebaseRead { base } => {
                    rebase_inspection = None;
                    let p = crate::rebase::inspect_rebase(r, &base, &cancel)?;
                    let view = p.series().clone();
                    rebase_inspection = Some((epoch.load(Ordering::SeqCst), p));
                    Ok(Payload::RebaseSeries(view))
                }
                GitCommand::ShelfDeleted => Ok(Payload::ShelfDeleted(
                    crate::shelf::list_deleted_shelves(r, &cancel)?,
                )),
                GitCommand::Shelves => {
                    Ok(Payload::Shelves(crate::shelf::list_shelves(r, &cancel)?))
                }
                GitCommand::Changelists => Ok(Payload::Changelists(
                    crate::shelf::list_changelists(r, &cancel)?,
                )),
                GitCommand::ShelfDeletedDetails { id } => Ok(Payload::ShelfDetails(
                    crate::shelf::shelf_deleted_details(r, &id, &cancel)?,
                )),
                GitCommand::ShelfDetails { id } => Ok(Payload::ShelfDetails(
                    crate::shelf::shelf_details(r, &id, &cancel)?,
                )),
                GitCommand::RevisionDiff { base, head, path } => Ok(Payload::RevisionDiff(
                    r.revision_diff(base.as_deref(), &head, path.as_deref(), &cancel)?,
                )),
                GitCommand::StashDetails { stash } => {
                    Ok(Payload::StashDetails(r.stash_details(&stash, &cancel)?))
                }
                GitCommand::Branches => Ok(Payload::Branches(r.branches(&cancel)?)),
                GitCommand::Tags => Ok(Payload::Tags(r.tags(&cancel)?)),
                GitCommand::Remotes => Ok(Payload::Remotes(workflows::list_remotes(r, &cancel)?)),
                GitCommand::Stashes => Ok(Payload::Stashes(workflows::list_stashes(r, &cancel)?)),
                GitCommand::Blame { path, start, lines } => Ok(Payload::Blame(r.blame(
                    &path,
                    start,
                    lines.min(500),
                    &cancel,
                )?)),
                GitCommand::FileAt { commit, path } => {
                    Ok(Payload::File(r.file_at(&commit, &path, &cancel)?))
                }
                GitCommand::CommitDetails { commit } => {
                    Ok(Payload::Details(r.commit_details(&commit, &cancel)?))
                }
                GitCommand::ConflictRead { path } => {
                    let snapshot = conflicts::load_conflict(r, &path, &cancel)?;
                    let file = snapshot.file.clone();
                    conflict = Some(snapshot);
                    Ok(Payload::Conflict(file))
                }
                command @ (GitCommand::Prepare(_)
                | GitCommand::WorkflowPrepare(_)
                | GitCommand::ConflictPrepare { .. }
                | GitCommand::ShelfPrepare(_)
                | GitCommand::ScmPrepare(_)
                | GitCommand::HunksPrepare { .. }
                | GitCommand::RebasePrepare { .. }) => {
                    if !trust.load(Ordering::SeqCst) {
                        return Err(GitError::invalid("Operação exige autorização Git"));
                    }
                    prepared = None;
                    next_id = next_id.wrapping_add(1);
                    let (plan, summary, warnings, paths) = match command {
                        GitCommand::ScmPrepare(action) => {
                            let plan = crate::scm::prepare_scm(r, action, &cancel)?;
                            let summary = plan.preview.summary.clone();
                            let warnings = plan.preview.warnings.clone();
                            let paths = plan.preview.paths.clone();
                            (Prepared::Scm(Box::new(plan)), summary, warnings, paths)
                        }
                        GitCommand::HunksPrepare {
                            path,
                            hunks,
                            unstage,
                            expected_patch,
                        } => {
                            let p = prepare_visible_hunks(
                                r,
                                path,
                                hunks,
                                unstage,
                                &expected_patch,
                                &cancel,
                            )?;
                            let paths = p.paths.clone();
                            let (summary, warnings) = local_presentation(&p.action, paths.len());
                            (Prepared::Local(Box::new(p)), summary, warnings, paths)
                        }
                        GitCommand::Prepare(action) => {
                            let p = if let LocalAction::StageHunks {
                                path,
                                hunks,
                                unstage,
                            } = action
                            {
                                let origin = diff_inspection.as_ref().ok_or_else(|| {
                                    GitError::new(
                                        GitErrorKind::Stale,
                                        "Abra a diferença antes de selecionar os hunks",
                                    )
                                })?;
                                if origin.epoch != epoch.load(Ordering::SeqCst)
                                    || origin.path != r.relative_path(&path)?
                                    || origin.side
                                        != if unstage {
                                            DiffSide::Index
                                        } else {
                                            DiffSide::Working
                                        }
                                {
                                    return Err(GitError::new(
                                        GitErrorKind::Stale,
                                        "Hunks pertencem a outra diferença; atualize",
                                    ));
                                }
                                prepare_visible_hunks(
                                    r,
                                    path,
                                    hunks,
                                    unstage,
                                    &origin.patch,
                                    &cancel,
                                )?
                            } else {
                                local::prepare(r, action, &cancel)?
                            };
                            let paths = p.paths.clone();
                            let (summary, warnings) = local_presentation(&p.action, paths.len());
                            (Prepared::Local(Box::new(p)), summary, warnings, paths)
                        }
                        GitCommand::RebasePrepare { base, head, steps } => {
                            let (e, read) = rebase_inspection.as_ref().ok_or_else(|| {
                                GitError::new(
                                    GitErrorKind::Stale,
                                    "Carregue a série antes de preparar o rebase",
                                )
                            })?;
                            if *e != epoch.load(Ordering::SeqCst)
                                || read.series().base != base
                                || read.series().head != head
                            {
                                return Err(GitError::new(
                                    GitErrorKind::Stale,
                                    "Série de rebase diferente da exibida",
                                ));
                            }
                            let p = crate::rebase::prepare_rebase(r, read, &steps, &cancel)?;
                            (Prepared::Rebase(Box::new(p)),format!("Reaplicar {} commits da branch {}",steps.len(),read.series().branch),vec!["O plano reescreve o histórico da branch atual. Não há Push automático.".into()],vec![])
                        }
                        GitCommand::ShelfPrepare(action) => {
                            let p = crate::shelf::prepare_shelf(r, action, &cancel)?;
                            let (s, w, paths) = (
                                p.preview.summary.clone(),
                                p.preview.warnings.clone(),
                                p.preview.paths.clone(),
                            );
                            (Prepared::Shelf(Box::new(p)), s, w, paths)
                        }
                        GitCommand::WorkflowPrepare(WorkflowAction::PushTo {
                            remote,
                            source,
                            target,
                            tags,
                        }) => {
                            let (e, read) = push_inspection.as_ref().ok_or_else(|| {
                                GitError::new(
                                    GitErrorKind::Stale,
                                    "Revise os commits de saída antes de enviar",
                                )
                            })?;
                            let view = read.review();
                            if *e != epoch.load(Ordering::SeqCst)
                                || view.remote != remote
                                || view.source != source
                                || view.target != target
                            {
                                return Err(GitError::new(
                                    GitErrorKind::Stale,
                                    "Destino ou origem diferentes da revisão exibida",
                                ));
                            }
                            let p = crate::sync::prepare_push(r, read, tags, &cancel)?;
                            (
                                Prepared::Push(Box::new(p)),
                                format!("Enviar {source} para {remote}/{target}"),
                                view.warnings.clone(),
                                view.changes.iter().map(|c| c.path.clone()).collect(),
                            )
                        }
                        GitCommand::WorkflowPrepare(WorkflowAction::PushWithTags {
                            remote,
                            source,
                            target,
                            mode,
                        }) => {
                            let (e, read) = push_inspection.as_ref().ok_or_else(|| {
                                GitError::new(
                                    GitErrorKind::Stale,
                                    "Revise os commits de saída antes de enviar",
                                )
                            })?;
                            let view = read.review();
                            if *e != epoch.load(Ordering::SeqCst)
                                || view.remote != remote
                                || view.source != source
                                || view.target != target
                            {
                                return Err(GitError::new(
                                    GitErrorKind::Stale,
                                    "Destino ou origem diferentes da revisão exibida",
                                ));
                            }
                            let p = crate::sync::prepare_push_mode(r, read, mode, &cancel)?;
                            (
                                Prepared::Push(Box::new(p)),
                                format!("Enviar {source} para {remote}/{target}"),
                                view.warnings.clone(),
                                view.changes.iter().map(|c| c.path.clone()).collect(),
                            )
                        }
                        GitCommand::WorkflowPrepare(action) => {
                            let p = workflows::prepare_workflow(r, action, &cancel)?;
                            let (s, w) = (p.preview.summary.clone(), p.preview.warnings.clone());
                            (Prepared::Workflow(Box::new(p)), s, w, vec![])
                        }
                        GitCommand::ConflictPrepare {
                            path,
                            choice,
                            stage,
                        } => {
                            let c = conflict.as_ref().ok_or_else(|| {
                                GitError::invalid("Abra o conflito antes de preparar a resolução")
                            })?;
                            if r.relative_path(&path)? != c.file.path {
                                return Err(GitError::new(
                                    GitErrorKind::Stale,
                                    "Conflito exibido pertence a outro arquivo",
                                ));
                            }
                            let p = conflicts::prepare_resolution(r, c, choice, stage, &cancel)?;
                            let (s, w) = (p.preview.summary.clone(), p.preview.warnings.clone());
                            (Prepared::Resolution(Box::new(p)), s, w, vec![path])
                        }
                        _ => unreachable!(),
                    };
                    let effect = if matches!(&plan, Prepared::Local(local) if matches!(
                        &local.action, LocalAction::Stage { .. } | LocalAction::Unstage { .. }
                    )) {
                        PreviewEffect::IndexOnly
                    } else {
                        PreviewEffect::RequiresCleanBuffers
                    };
                    prepared = Some((next_id, generation, epoch.load(Ordering::SeqCst), plan));
                    Ok(Payload::Preview(
                        GitPreview {
                            id: next_id,
                            generation,
                            summary,
                            warnings,
                            paths,
                        },
                        effect,
                    ))
                }
                GitCommand::Apply {
                    preview_id,
                    repo_generation,
                }
                | GitCommand::WorkflowApply {
                    preview_id,
                    repo_generation,
                } => {
                    if !trust.load(Ordering::SeqCst) {
                        return Err(GitError::invalid("Operação exige autorização Git"));
                    }
                    let (id, plan_generation, plan_epoch, plan) =
                        prepared.take().ok_or_else(|| {
                            GitError::new(
                                GitErrorKind::Stale,
                                "Prévia Git ausente; prepare novamente",
                            )
                        })?;
                    if id != preview_id
                        || plan_generation != repo_generation
                        || plan_generation != generation
                        || plan_epoch != epoch.load(Ordering::SeqCst)
                    {
                        return Err(GitError::new(GitErrorKind::Stale, "Prévia Git expirada"));
                    }
                    let mut shelf_report = None;
                    let record_branch = matches!(&plan,Prepared::Local(local) if matches!(local.action,LocalAction::CheckoutBranch{..}|LocalAction::CreateBranch{checkout:true,..}));
                    let result = match plan {
                        Prepared::Scm(plan) => crate::scm::apply_scm(r, &plan, &cancel),
                        Prepared::Push(p) => {
                            crate::sync::apply_push(r, &p, &cancel).map(|v| GitReport {
                                success: v.success,
                                summary: v.summary,
                                output: v.output,
                                outcome_unknown: v.outcome_unknown,
                            })
                        }
                        Prepared::Rebase(p) => {
                            crate::rebase::apply_rebase(r, &p, &cancel).map(|v| GitReport {
                                success: v.success,
                                summary: v.summary,
                                output: v.output,
                                outcome_unknown: v.outcome_unknown,
                            })
                        }
                        Prepared::Shelf(p) => crate::shelf::apply_shelf(r, &p, &cancel).map(|v| {
                            let g = GitReport {
                                success: v.success,
                                summary: v.summary.clone(),
                                output: v.output.clone(),
                                outcome_unknown: v.outcome_unknown,
                            };
                            shelf_report = Some(v);
                            g
                        }),
                        Prepared::Local(p) => local::apply(r, &p, &cancel),
                        Prepared::Workflow(p) => {
                            workflows::apply_workflow(r, &p, &cancel).map(|v| GitReport {
                                success: v.success,
                                summary: v.summary,
                                output: v.output,
                                outcome_unknown: v.outcome_unknown,
                            })
                        }
                        Prepared::Resolution(p) => {
                            conflicts::apply_resolution(r, &p, &cancel).map(|v| GitReport {
                                success: v.error.is_none(),
                                summary: v.summary,
                                output: v.error.unwrap_or_default(),
                                outcome_unknown: v.outcome_unknown,
                            })
                        }
                    };
                    let mut report = match result {
                        Ok(r) => r,
                        Err(e) => GitReport {
                            success: false,
                            summary: e.to_string(),
                            output: String::new(),
                            outcome_unknown: !matches!(
                                e.kind,
                                GitErrorKind::Stale
                                    | GitErrorKind::Invalid
                                    | GitErrorKind::Unsupported
                            ),
                        },
                    };
                    if report.success
                        && record_branch
                        && let Err(error) =
                            crate::preferences::record_current(r, &AtomicBool::new(false))
                    {
                        report.output.push_str(&format!(
                            "\nBranch alterada; não foi possível salvar recentes: {error}"
                        ));
                    }
                    diff_inspection = None;
                    let snapshot = r.snapshot(&AtomicBool::new(false)).ok();
                    conflict = None;
                    push_inspection = None;
                    rebase_inspection = None;
                    Ok(Payload::Report(report, snapshot, shelf_report))
                }
                _ => Err(GitError::invalid("Comando Git fora de contexto")),
            }
        })();
        if sender.send(Event { generation, result }).is_err() {
            break;
        }
    }
}

fn local_presentation(action: &LocalAction, path_count: usize) -> (String, Vec<String>) {
    let files = if path_count == 1 {
        "1 arquivo".into()
    } else {
        format!("{path_count} arquivos")
    };
    let short = |revision: &str| revision.chars().take(12).collect::<String>();
    let summary = match action {
        LocalAction::Stage { .. } => format!("Preparar {files} no índice"),
        LocalAction::Unstage { .. } => format!("Retirar {files} do índice"),
        LocalAction::Discard { .. } => format!("Descartar alterações de {files}"),
        LocalAction::StageHunks { hunks, unstage, .. } => {
            let count = hunks.len();
            let snippets = if count == 1 {
                "1 trecho".into()
            } else {
                format!("{count} trechos")
            };
            if *unstage {
                format!("Retirar {snippets} do índice em {files}")
            } else {
                format!("Preparar {snippets} no índice em {files}")
            }
        }
        LocalAction::Commit { amend, .. } => {
            if *amend {
                if path_count == 0 {
                    "Alterar o último commit".into()
                } else {
                    format!("Alterar o último commit com {files}")
                }
            } else {
                format!("Criar commit com {files}")
            }
        }
        LocalAction::CreateBranch { name, checkout, .. } => {
            if *checkout {
                format!("Criar e abrir a branch {name}")
            } else {
                format!("Criar a branch {name}")
            }
        }
        LocalAction::CheckoutBranch { name } => format!("Abrir a branch {name}"),
        LocalAction::CheckoutRevision { revision } => {
            format!("Abrir a revisão {}", short(revision))
        }
        LocalAction::RenameBranch { old, new } => format!("Renomear a branch {old} para {new}"),
        LocalAction::DeleteBranch { name } => format!("Excluir a branch {name}"),
        LocalAction::CreateTag { name, target, .. } => {
            format!("Criar a tag {name} na revisão {}", short(target))
        }
        LocalAction::DeleteTag { name } => format!("Excluir a tag {name}"),
    };
    let warnings=match action{
        LocalAction::Discard{..}=>vec!["As alterações locais e preparadas dos arquivos selecionados serão descartadas.".into()],
        LocalAction::CheckoutRevision{..}=>vec!["A revisão será aberta com HEAD destacado. Crie uma branch para manter novos commits acessíveis.".into()],
        LocalAction::Commit{..}=>vec!["O commit respeitará os hooks e a assinatura configurados no Git.".into()],
        _=>vec![],
    };
    (summary, warnings)
}

fn prepare_visible_hunks(
    repo: &Repository,
    path: PathBuf,
    mut hunks: Vec<usize>,
    unstage: bool,
    expected_patch: &[u8],
    cancel: &AtomicBool,
) -> Result<PreparedLocal, GitError> {
    if expected_patch.len() > 8 * 1024 * 1024 {
        return Err(GitError::invalid("Patch exibido excede8MiB"));
    }
    let before = repo.snapshot(cancel)?;
    let diff = repo.diff(
        &path,
        if unstage {
            DiffSide::Index
        } else {
            DiffSide::Working
        },
        cancel,
    )?;
    if diff.bytes != expected_patch {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "A diferença mudou desde a seleção dos hunks. Atualize e selecione novamente.",
        ));
    }
    hunks.sort_unstable();
    hunks.dedup();
    let expected = local::selected_hunks_patch(&diff, &hunks)?;
    let prepared = local::prepare(
        repo,
        LocalAction::StageHunks {
            path,
            hunks,
            unstage,
        },
        cancel,
    )?;
    if prepared.snapshot != before
        || repo.snapshot(cancel)? != before
        || !local::prepared_hunks_match(&prepared, &expected)
    {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Arquivo ou hunks mudaram durante a preparação; atualize",
        ));
    }
    Ok(prepared)
}

#[cfg(test)]
mod history_delivery_tests {
    use super::*;
    fn cached() -> (tempfile::TempDir, GitService) {
        let dir = tempfile::tempdir().unwrap();
        let mut s = GitService::new();
        // Controlled service boundary: these tests do not claim a real Git read.
        s.repo = Some(RepositoryInfo {
            root: dir.path().into(),
            git_dir: dir.path().join(".git"),
            common_dir: dir.path().join(".git"),
            bare: false,
            linked_worktree: false,
        });
        s.history = Some(HistoryPage {
            resolved_reference: None,
            skip: 0,
            has_more: false,
            next_lanes: vec![],
            commits: vec![],
        });
        (dir, s)
    }
    #[test]
    fn cancelled_pending_history_does_not_publish_a_success_already_in_the_delivery_queue() {
        let (_dir, mut s) = cached();
        let page = s.history.clone().unwrap();
        let (tx, rx) = mpsc::channel();
        s.receiver = rx;
        s.busy = true;
        s.reading_history = true;
        tx.send(Event {
            generation: s.repo_generation,
            result: Ok(Payload::History(page)),
        })
        .unwrap();
        s.dispatch(GitCommand::Cancel).unwrap();
        assert!(s.poll());
        assert!(!s.is_busy());
        assert!(!s.history_pending());
        assert!(s.history.is_none());
        assert!(s.error.as_deref().is_some_and(|e| e.contains("cancelada")));
    }
    #[test]
    fn closed_delivery_never_leaves_old_page_as_result_for_a_new_request() {
        let (_dir, mut s) = cached();
        s.sender.take();
        assert!(
            s.dispatch(GitCommand::History {
                skip: 0,
                limit: 200,
                path: None
            })
            .is_err()
        );
        assert!(s.history.is_none());
        assert!(!s.history_pending());
        assert!(!s.is_busy());
    }
    #[test]
    fn refused_parallel_read_keeps_the_inflight_page_and_pending_state() {
        let (_dir, mut s) = cached();
        s.busy = true;
        s.reading_history = true;
        assert!(
            s.dispatch(GitCommand::History {
                skip: 0,
                limit: 200,
                path: None
            })
            .is_err()
        );
        assert!(s.history.is_some());
        assert!(s.history_pending());
        assert!(s.is_busy());
    }
}
