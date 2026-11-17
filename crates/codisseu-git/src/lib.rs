//! Native Git CLI worker. No shell interpolation, GUI dependency, or implicit push.
pub mod autoria;
pub mod commit_message;
pub mod conflicts;
pub mod discovery;
mod history;
pub mod line_changes;
mod local;
mod model;
pub mod preferences;
pub mod previous;
mod process;
pub mod rebase;
mod repository;
pub mod scm;
pub mod search;
mod service;
pub mod shelf;
pub mod sync;
pub mod text_merge;
pub mod workflows;
pub use conflicts::{ConflictFile, ConflictSide, ResolutionChoice};
pub use discovery::{RepositoryDiscovery, RepositorySummary};
pub use model::*;
pub use preferences::BranchPreferences;
pub use rebase::{RebaseAction, RebaseCommit, RebaseSeries, RebaseStep};
pub use repository::Repository;
pub use scm::{CommitIdentity, ScmAction};
pub use service::{GitCommand, GitService};
pub use shelf::{Changelist, ShelfAction, ShelfDetails, ShelfFileStatus, ShelfInfo};
pub use sync::{PushReview, PushTagsMode};
pub use workflows::{OperationKind, PullMode, RemoteInfo, ResetMode, StashEntry, WorkflowAction};
/// Redact URL userinfo from diagnostics; file/diff payloads are never rewritten.
pub(crate) fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    let mut from = 0;
    while let Some(relative) = output[from..].find("://") {
        let start = from + relative + 3;
        let end = output[start..]
            .find(|c: char| c == '/' || c.is_whitespace() || matches!(c, '\'' | '"'))
            .map_or(output.len(), |n| start + n);
        if let Some(at) = output[start..end].rfind('@') {
            let at = start + at;
            output.replace_range(start..at, "<redacted>");
            from = start + "<redacted>@".len();
        } else {
            from = end;
        }
        if from >= output.len() {
            break;
        }
    }
    let mut start = 0;
    while let Some(relative) = output[start..].find("://") {
        let url_start = start + relative + 3;
        let url_end = output[url_start..]
            .find(char::is_whitespace)
            .map_or(output.len(), |n| url_start + n);
        if let Some(q) = output[url_start..url_end].find('?') {
            let q = url_start + q;
            output.replace_range(q + 1..url_end, "<redacted>");
            start = q + 1 + "<redacted>".len();
        } else {
            start = url_end;
        }
        if start >= output.len() {
            break;
        }
    }
    output
}

#[cfg(test)]
mod redaction_tests {
    #[test]
    fn secrets() {
        assert_eq!(
            super::redact("To https://u:secret@host/repo?token=secret"),
            "To https://<redacted>@host/repo?<redacted>"
        );
    }
}
