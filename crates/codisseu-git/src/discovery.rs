//! Bounded repository discovery; never follows directory links or enters .git.
use crate::{
    GitError, GitErrorKind, RepoSnapshot, Repository, StatusEntry,
    repository::{args, cancelled, os_bytes, parse_status_limited},
};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct RepositorySummary {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub changes: Option<usize>,
    pub conflicts: Option<usize>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
    pub entries: Option<Arc<Vec<StatusEntry>>>,
    pub entries_truncated: bool,
}
impl RepositorySummary {
    pub(crate) fn from_snapshot(repo: &crate::RepositoryInfo, snapshot: &RepoSnapshot) -> Self {
        Self {
            root: repo.root.clone(),
            git_dir: repo.git_dir.clone(),
            head: snapshot.head.clone(),
            branch: snapshot.head_ref.clone(),
            changes: Some(snapshot.entries.len()),
            conflicts: Some(
                snapshot
                    .entries
                    .iter()
                    .filter(|entry| entry.conflict)
                    .count(),
            ),
            ahead: snapshot.upstream_counts_known.then_some(snapshot.ahead),
            behind: snapshot.upstream_counts_known.then_some(snapshot.behind),
            entries: Some(Arc::new(snapshot.entries.clone())),
            entries_truncated: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct RepositoryDiscovery {
    pub workspace: PathBuf,
    pub repositories: Vec<RepositorySummary>,
    pub truncated: bool,
    pub visited_entries: usize,
    /// Local failures do not hide healthy sibling repositories. Retained messages
    /// are bounded and redacted; cancellation still aborts the whole request.
    pub errors: Vec<RepositoryDiscoveryError>,
}
#[derive(Clone, Debug)]
pub struct RepositoryDiscoveryError {
    pub path: PathBuf,
    pub message: String,
}
/// Discover only explicit workspace folders, with shared result/entry budgets.
pub fn discover_roots(
    workspace: &Path,
    folders: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<RepositoryDiscovery, GitError> {
    if folders.len() > 32 {
        return Err(GitError::invalid(
            "Workspace deve conter até32raízes disponíveis",
        ));
    }
    let mut result = RepositoryDiscovery {
        workspace: workspace.to_path_buf(),
        repositories: Vec::new(),
        truncated: false,
        visited_entries: 0,
        errors: Vec::new(),
    };
    let start = Instant::now();
    let mut budget = StatusBudget {
        entries: 10000,
        bytes: 32 * 1024 * 1024,
        known: BTreeSet::new(),
    };
    // Admit each declared project's own repository before spending the shared
    // traversal budget on any one directory (which may be a huge non-Git tree).
    for folder in folders {
        cancelled(cancel)?;
        if start.elapsed() > Duration::from_secs(20) {
            result.truncated = true;
            break;
        }
        match Repository::discover(folder, cancel) {
            Ok(repo) if budget.known.insert(repo.root.clone()) => {
                result.repositories.push(summary_or_error(
                    &repo,
                    cancel,
                    &mut budget,
                    &mut result.errors,
                    &mut result.truncated,
                )?);
            }
            Err(error) if error.kind == GitErrorKind::Cancelled => return Err(error),
            _ => (), // The traversal below records local filesystem/Git errors.
        }
    }
    for folder in folders {
        cancelled(cancel)?;
        if result.repositories.len() >= 32 || result.visited_entries >= 50000 {
            result.truncated = true;
            break;
        }
        match discover_with_budget(
            folder,
            6,
            32 - result.repositories.len(),
            50000 - result.visited_entries,
            cancel,
            start,
            &mut budget,
        ) {
            Ok(found) => {
                result.truncated |= found.truncated;
                result.visited_entries += found.visited_entries;
                for repo in found.repositories {
                    if !result.repositories.iter().any(|r| r.root == repo.root) {
                        result.repositories.push(repo);
                    }
                }
                for error in found.errors.into_iter().take(32 - result.errors.len()) {
                    result.errors.push(error);
                }
            }
            Err(error) => record_error(folder, error, &mut result.errors, &mut result.truncated)?,
        }
    }
    Ok(result)
}
pub fn discover_repositories(
    workspace: &Path,
    max_depth: usize,
    max_repositories: usize,
    max_entries: usize,
    cancel: &AtomicBool,
) -> Result<RepositoryDiscovery, GitError> {
    discover_with_budget(
        workspace,
        max_depth,
        max_repositories,
        max_entries,
        cancel,
        Instant::now(),
        &mut StatusBudget {
            entries: 10000,
            bytes: 32 * 1024 * 1024,
            known: BTreeSet::new(),
        },
    )
}
fn discover_with_budget(
    workspace: &Path,
    max_depth: usize,
    max_repositories: usize,
    max_entries: usize,
    cancel: &AtomicBool,
    start: Instant,
    status_budget: &mut StatusBudget,
) -> Result<RepositoryDiscovery, GitError> {
    if max_depth > 16
        || !(1..=32).contains(&max_repositories)
        || !(1..=50000).contains(&max_entries)
    {
        return Err(GitError::invalid(
            "Descoberta Git: profundidade≤16, repositórios1–32 e entradas1–50000",
        ));
    }
    let workspace = fs::canonicalize(workspace)?;
    if !workspace.is_dir() {
        return Err(GitError::invalid(
            "Selecione uma pasta para descobrir repositórios",
        ));
    }
    let mut roots = BTreeSet::new();
    let mut repositories = Vec::new();
    let mut queue = VecDeque::from([(workspace.clone(), 0)]);
    let mut visited_entries = 0;
    let mut truncated = false;
    let mut errors = Vec::new();
    match Repository::discover(&workspace, cancel) {
        Ok(repo) => {
            roots.insert(repo.root.clone());
            if status_budget.known.insert(repo.root.clone()) {
                repositories.push(summary_or_error(
                    &repo,
                    cancel,
                    status_budget,
                    &mut errors,
                    &mut truncated,
                )?);
            }
        }
        Err(error) if workspace.join(".git").exists() || error.kind == GitErrorKind::Cancelled => {
            record_error(&workspace, error, &mut errors, &mut truncated)?;
        }
        Err(_) => (),
    }
    'walk: while let Some((directory, depth)) = queue.pop_front() {
        cancelled(cancel)?;
        if start.elapsed() > Duration::from_secs(20) {
            return Err(GitError::new(
                GitErrorKind::Timeout,
                "Descoberta Git excedeu20s; escolha uma pasta menor",
            ));
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory != workspace => {
                record_error(&directory, error.into(), &mut errors, &mut truncated)?;
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            cancelled(cancel)?;
            visited_entries += 1;
            if visited_entries > max_entries {
                truncated = true;
                break 'walk;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_error(&directory, error.into(), &mut errors, &mut truncated)?;
                    continue;
                }
            };
            let metadata = match fs::symlink_metadata(entry.path()) {
                Ok(metadata) => metadata,
                Err(error) => {
                    record_error(&entry.path(), error.into(), &mut errors, &mut truncated)?;
                    continue;
                }
            };
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if [
                ".git",
                "node_modules",
                "vendor",
                "target",
                ".cache",
                ".venv",
                "venv",
            ]
            .iter()
            .any(|excluded| name == *excluded)
            {
                continue;
            }
            if depth >= max_depth {
                truncated = true;
                continue;
            }
            let path = entry.path();
            if fs::symlink_metadata(path.join(".git")).is_ok() {
                let repo = match Repository::discover(&path, cancel) {
                    Ok(repo) => repo,
                    Err(error) => {
                        record_error(&path, error, &mut errors, &mut truncated)?;
                        // An invalid parent can still contain independent projects.
                        queue.push_back((path, depth + 1));
                        continue;
                    }
                };
                if !repo.root.starts_with(&workspace) {
                    return Err(GitError::invalid(
                        "Repositório descoberto escapa da pasta selecionada",
                    ));
                }
                if roots.insert(repo.root.clone()) && status_budget.known.insert(repo.root.clone())
                {
                    if repositories.len() == max_repositories {
                        truncated = true;
                        break 'walk;
                    }
                    repositories.push(summary_or_error(
                        &repo,
                        cancel,
                        status_budget,
                        &mut errors,
                        &mut truncated,
                    )?);
                }
            }
            if depth < max_depth {
                queue.push_back((path, depth + 1));
            } else {
                truncated = true;
            }
        }
    }
    truncated |= repositories.iter().any(|item| item.entries_truncated);
    repositories.sort_by(|a, b| a.root.cmp(&b.root));
    Ok(RepositoryDiscovery {
        workspace,
        repositories,
        truncated,
        visited_entries,
        errors,
    })
}
fn record_error(
    path: &Path,
    error: GitError,
    errors: &mut Vec<RepositoryDiscoveryError>,
    truncated: &mut bool,
) -> Result<(), GitError> {
    if error.kind == GitErrorKind::Cancelled {
        return Err(error);
    }
    if errors.len() < 32 {
        errors.push(RepositoryDiscoveryError {
            path: path.to_owned(),
            message: crate::redact(&error.message).chars().take(2048).collect(),
        });
    } else {
        *truncated = true;
    }
    Ok(())
}
fn summary_or_error(
    repo: &Repository,
    cancel: &AtomicBool,
    budget: &mut StatusBudget,
    errors: &mut Vec<RepositoryDiscoveryError>,
    truncated: &mut bool,
) -> Result<RepositorySummary, GitError> {
    match summary(repo, cancel, budget) {
        Ok(summary) => Ok(summary),
        Err(error) => {
            record_error(&repo.root, error, errors, truncated)?;
            Ok(RepositorySummary {
                root: repo.root.clone(),
                git_dir: repo.git_dir.clone(),
                head: None,
                branch: None,
                changes: None,
                conflicts: None,
                ahead: None,
                behind: None,
                entries: None,
                entries_truncated: true,
            })
        }
    }
}
struct StatusBudget {
    entries: usize,
    bytes: usize,
    known: BTreeSet<PathBuf>,
}
fn summary(
    repo: &Repository,
    cancel: &AtomicBool,
    budget: &mut StatusBudget,
) -> Result<RepositorySummary, GitError> {
    let head = repo.run(&args(&["rev-parse", "--verify", "HEAD"]), None, cancel)?;
    let branch = repo.run(&args(&["symbolic-ref", "-q", "HEAD"]), None, cancel)?;
    let mut result = RepositorySummary {
        root: repo.root.clone(),
        git_dir: repo.git_dir.clone(),
        head: head
            .status
            .success()
            .then(|| String::from_utf8_lossy(&head.stdout).trim().to_owned()),
        branch: branch
            .status
            .success()
            .then(|| String::from_utf8_lossy(&branch.stdout).trim().to_owned()),
        changes: None,
        conflicts: None,
        ahead: None,
        behind: None,
        entries: None,
        entries_truncated: true,
    };
    if budget.entries == 0 || budget.bytes == 0 || repo.bare {
        return Ok(result);
    }
    let raw = match repo.snapshot_status(cancel) {
        Ok(raw) => raw,
        Err(error) if error.kind == GitErrorKind::OutputLimit => {
            budget.bytes = 0;
            return Ok(result);
        }
        Err(error) => return Err(error),
    };
    if raw.len() > budget.bytes {
        budget.bytes = 0;
        return Ok(result);
    }
    budget.bytes -= raw.len();
    let ((entries, head, branch, _, ahead, behind), mut truncated) =
        parse_status_limited(&raw, budget.entries)?;
    result.head = head;
    result.branch = branch;
    if raw
        .split(|byte| *byte == 0)
        .any(|line| line.starts_with(b"# branch.ab "))
    {
        result.ahead = Some(ahead);
        result.behind = Some(behind);
    }
    let mut cached = Vec::new();
    for entry in entries {
        let bytes = entry_bytes(&entry);
        if bytes > budget.bytes {
            budget.bytes = 0;
            truncated = true;
            break;
        }
        budget.bytes -= bytes;
        budget.entries -= 1;
        cached.push(entry);
    }
    if !truncated {
        result.changes = Some(cached.len());
        result.conflicts = Some(cached.iter().filter(|entry| entry.conflict).count());
    }
    result.entries = Some(Arc::new(cached));
    result.entries_truncated = truncated;
    Ok(result)
}

fn entry_bytes(entry: &StatusEntry) -> usize {
    std::mem::size_of::<StatusEntry>()
        + os_bytes(entry.path.as_os_str()).len()
        + entry
            .original_path
            .as_ref()
            .map_or(0, |path| os_bytes(path.as_os_str()).len())
        + entry.submodule.len()
}
/// Bound the retained cache after selecting/refreshing repositories as well as discovery.
/// The selected repository gets first budget; evicted prefixes are explicitly partial.
pub(crate) fn bound_status_cache(repositories: &mut [RepositorySummary], preferred: &Path) {
    let mut budget = StatusBudget {
        entries: 10000,
        bytes: 32 * 1024 * 1024,
        known: BTreeSet::new(),
    };
    let preferred = repositories.iter().position(|repo| repo.root == preferred);
    let order = preferred
        .into_iter()
        .chain((0..repositories.len()).filter(|index| Some(*index) != preferred))
        .collect::<Vec<_>>();
    for index in order {
        let summary = &mut repositories[index];
        if let Some(entries) = &mut summary.entries {
            let mut keep = 0;
            for entry in entries.iter() {
                let bytes = entry_bytes(entry);
                if budget.entries == 0 || bytes > budget.bytes {
                    break;
                }
                budget.entries -= 1;
                budget.bytes -= bytes;
                keep += 1;
            }
            if keep < entries.len() {
                *entries = Arc::new(entries[..keep].to_vec());
                summary.entries_truncated = true;
                summary.changes = None;
                summary.conflicts = None;
            }
        }
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn selected_cache_retains_priority_inside_the_byte_and_entry_budgets() {
        let mut repositories = (0..4)
            .map(|number| RepositorySummary {
                root: PathBuf::from(format!("/fixture/{number}")),
                git_dir: PathBuf::from(format!("/fixture/{number}/.git")),
                head: None,
                branch: None,
                changes: Some(2500),
                conflicts: Some(0),
                ahead: None,
                behind: None,
                entries: Some(Arc::new(
                    (0..2500)
                        .map(|index| StatusEntry {
                            path: PathBuf::from(format!("{}/file-{index}", "a".repeat(4000))),
                            original_path: None,
                            index: '?',
                            worktree: '?',
                            untracked: true,
                            conflict: false,
                            submodule: "N...".into(),
                        })
                        .collect(),
                )),
                entries_truncated: false,
            })
            .collect::<Vec<_>>();
        bound_status_cache(&mut repositories, Path::new("/fixture/3"));
        let entries = repositories
            .iter()
            .filter_map(|summary| summary.entries.as_ref())
            .flat_map(|entries| entries.iter())
            .collect::<Vec<_>>();
        assert!(entries.len() <= 10000);
        assert!(
            entries
                .iter()
                .map(|entry| entry_bytes(entry))
                .sum::<usize>()
                <= 32 * 1024 * 1024
        );
        assert_eq!(repositories[3].entries.as_ref().unwrap().len(), 2500);
        assert!(!repositories[3].entries_truncated);
        assert!(repositories.iter().any(|summary| summary.entries_truncated
            && summary.changes.is_none()
            && summary.conflicts.is_none()));
    }
}
