use std::{path::PathBuf, process::ExitStatus};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitErrorKind {
    Invalid,
    Git,
    Io,
    Stale,
    Cancelled,
    Timeout,
    OutputLimit,
    Unsupported,
}
#[derive(Clone, Debug)]
pub struct GitError {
    pub kind: GitErrorKind,
    pub message: String,
}
impl GitError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: GitErrorKind::Invalid,
            message: message.into(),
        }
    }
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            kind: GitErrorKind::Unsupported,
            message: message.into(),
        }
    }
    pub(crate) fn new(kind: GitErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for GitError {}
impl From<std::io::Error> for GitError {
    fn from(error: std::io::Error) -> Self {
        Self::new(GitErrorKind::Io, error.to_string())
    }
}
#[derive(Debug)]
pub struct GitOutput {
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub status: ExitStatus,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryInfo {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub bare: bool,
    pub linked_worktree: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub index: char,
    pub worktree: char,
    pub untracked: bool,
    pub conflict: bool,
    pub submodule: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub root: PathBuf,
    pub head: Option<String>,
    pub head_ref: Option<String>,
    pub index_hash: String,
    pub worktree_hash: String,
    pub entries: Vec<StatusEntry>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
    pub upstream_counts_known: bool,
    pub operation: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    Working,
    Index,
}
#[derive(Clone, Debug)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<String>,
    pub patch: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct GitDiff {
    pub path: PathBuf,
    pub side: DiffSide,
    pub bytes: Vec<u8>,
    pub binary: bool,
    pub hunks: Vec<DiffHunk>,
}
#[derive(Clone, Debug)]
pub struct CommitEntry {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub subject: String,
    pub refs: Vec<String>,
    pub lane: usize,
    pub edges: Vec<(usize, usize)>,
}
#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub resolved_reference: Option<String>,
    pub skip: usize,
    pub commits: Vec<CommitEntry>,
    pub has_more: bool,
    pub next_lanes: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct BranchInfo {
    pub name: String,
    pub oid: String,
    pub current: bool,
    pub remote: bool,
    pub upstream: Option<String>,
    pub tracking: String,
    pub worktree: Option<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct TagInfo {
    pub name: String,
    pub oid: String,
    pub target: String,
    pub annotated: bool,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct BlameLine {
    pub line: usize,
    pub original_line: usize,
    pub oid: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub path: PathBuf,
    pub text: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct FileVersion {
    pub commit: String,
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub enum CommitSelection {
    AllStaged,
    Paths(Vec<PathBuf>),
    WorkingPaths(Vec<PathBuf>),
}
#[derive(Clone, Debug)]
pub enum LocalAction {
    Discard {
        paths: Vec<PathBuf>,
    },
    Stage {
        paths: Vec<PathBuf>,
    },
    Unstage {
        paths: Vec<PathBuf>,
    },
    StageHunks {
        path: PathBuf,
        hunks: Vec<usize>,
        unstage: bool,
    },
    Commit {
        selection: CommitSelection,
        message: String,
        amend: bool,
    },
    CreateBranch {
        name: String,
        start: Option<String>,
        checkout: bool,
    },
    CheckoutBranch {
        name: String,
    },
    CheckoutRevision {
        revision: String,
    },
    RenameBranch {
        old: String,
        new: String,
    },
    DeleteBranch {
        name: String,
    },
    CreateTag {
        name: String,
        target: String,
        message: Option<String>,
    },
    DeleteTag {
        name: String,
    },
}
#[derive(Clone, Debug)]
pub struct GitPreview {
    pub id: u64,
    pub generation: u64,
    pub summary: String,
    pub warnings: Vec<String>,
    pub paths: Vec<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct GitReport {
    pub success: bool,
    pub summary: String,
    pub output: String,
    pub outcome_unknown: bool,
}
#[derive(Clone, Debug)]
pub struct CommitChange {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub status: char,
}
#[derive(Clone, Debug)]
pub struct CommitDetails {
    pub commit: String,
    pub message: String,
    pub parents: Vec<String>,
    pub changes: Vec<CommitChange>,
    pub patch: Vec<u8>,
    pub binary: bool,
}

#[derive(Clone, Debug)]
pub struct StashDetails {
    pub reference: String,
    pub oid: String,
    pub changes: Vec<CommitChange>,
    pub patch: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionDiffQuery {
    pub base: Option<String>,
    pub head: String,
    pub path: Option<PathBuf>,
}
#[derive(Clone, Debug)]
pub struct RevisionDiff {
    /// Original request, distinct from the resolved immutable OIDs below.
    pub query: RevisionDiffQuery,
    pub base: Option<String>,
    pub head: String,
    pub path: Option<PathBuf>,
    pub bytes: Vec<u8>,
}
