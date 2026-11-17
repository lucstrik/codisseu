use crate::{
    repository::{args, checked, file_hash},
    *,
};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub(crate) struct PreparedLocal {
    pub snapshot: RepoSnapshot,
    pub action: LocalAction,
    pub paths: Vec<PathBuf>,
    patch: Option<Vec<u8>>,
}
fn validate_ref(
    repo: &Repository,
    name: &str,
    branch: bool,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    if name.is_empty() || name.starts_with('-') || name.len() > 1024 || name.contains('\0') {
        return Err(GitError::invalid("Nome de referência inválido"));
    }
    let arguments = if branch {
        args(&["check-ref-format", "--branch", name])
    } else {
        args(&["check-ref-format", &format!("refs/tags/{name}")])
    };
    repo.run_checked(&arguments, None, cancel)?;
    Ok(())
}
fn paths(
    repo: &Repository,
    snapshot: &RepoSnapshot,
    input: &[PathBuf],
) -> Result<Vec<PathBuf>, GitError> {
    if input.is_empty() || input.len() > 512 {
        return Err(GitError::invalid("Selecione de 1 a 512 arquivos"));
    }
    let mut out = Vec::new();
    for path in input {
        let path = repo.relative_path(path)?;
        let entry = snapshot
            .entries
            .iter()
            .find(|e| e.path == path || e.original_path.as_ref() == Some(&path))
            .ok_or_else(|| GitError::invalid("Arquivo não está no snapshot de alterações"))?;
        if entry.conflict {
            return Err(GitError::invalid(
                "Resolva o conflito antes de preparar este arquivo",
            ));
        }
        if entry.submodule != "N..." {
            return Err(GitError::unsupported(
                "Alterações de submódulos exigem operação no próprio repositório",
            ));
        }
        for p in std::iter::once(&entry.path).chain(entry.original_path.iter()) {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
    }
    Ok(out)
}
pub(crate) fn prepare(
    repo: &Repository,
    action: LocalAction,
    cancel: &AtomicBool,
) -> Result<PreparedLocal, GitError> {
    if repo.bare {
        return Err(GitError::unsupported(
            "Operação local exige árvore de trabalho",
        ));
    }
    let snapshot = repo.snapshot(cancel)?;
    let mut action = action;
    let mut selected = Vec::new();
    let mut patch = None;
    match &mut action {
        LocalAction::Discard { paths: p } => {
            selected = paths(repo, &snapshot, p)?;
            if snapshot.head.is_none()
                || selected.iter().any(|path| {
                    snapshot
                        .entries
                        .iter()
                        .any(|e| e.path == *path && (e.untracked || e.index == 'A'))
                })
            {
                return Err(GitError::invalid(
                    "Rollback exige arquivos existentes no HEAD; arquivos novos não são apagados",
                ));
            }
            *p = selected.clone();
        }
        LocalAction::Stage { paths: p } | LocalAction::Unstage { paths: p } => {
            selected = paths(repo, &snapshot, p)?;
            *p = selected.clone();
        }
        LocalAction::StageHunks {
            path,
            hunks,
            unstage,
        } => {
            selected = paths(repo, &snapshot, std::slice::from_ref(path))?;
            if selected.len() != 1 {
                return Err(GitError::unsupported(
                    "Prepare renomeações como arquivo completo",
                ));
            }
            *path = selected[0].clone();
            let diff = repo.diff(
                path,
                if *unstage {
                    DiffSide::Index
                } else {
                    DiffSide::Working
                },
                cancel,
            )?;
            let text = std::str::from_utf8(&diff.bytes)
                .map_err(|_| GitError::unsupported("Hunks exigem arquivo de texto UTF-8"))?;
            if diff.binary
                || hunks.is_empty()
                || hunks.len() > 1000
                || text.lines().any(|l| {
                    l.starts_with("rename ")
                        || l.starts_with("new file ")
                        || l.starts_with("deleted file ")
                        || l.starts_with("old mode ")
                        || l.starts_with("new mode ")
                })
            {
                return Err(GitError::unsupported(
                    "Prepare este tipo de alteração como arquivo completo",
                ));
            }
            hunks.sort_unstable();
            hunks.dedup();
            patch = Some(selected_hunks_patch(&diff, hunks)?);
        }
        LocalAction::Commit {
            selection,
            message,
            amend,
        } => {
            if message.trim().is_empty() || message.len() > 65536 || message.contains('\0') {
                return Err(GitError::invalid(
                    "Mensagem de commit vazia ou maior que64 KiB",
                ));
            }
            if *amend && snapshot.head.is_none() {
                return Err(GitError::invalid("Não existe commit para alterar"));
            }
            if snapshot.operation.is_some() {
                return Err(GitError::invalid(
                    "Finalize a operação Git em andamento antes de criar commit selecionado",
                ));
            }
            selected = match selection {
                CommitSelection::AllStaged => snapshot
                    .entries
                    .iter()
                    .filter(|e| !e.untracked && e.index != '.' && !e.conflict)
                    .flat_map(|e| std::iter::once(e.path.clone()).chain(e.original_path.clone()))
                    .collect(),
                CommitSelection::Paths(p) | CommitSelection::WorkingPaths(p) => {
                    if p.is_empty() && *amend {
                        vec![]
                    } else {
                        paths(repo, &snapshot, p)?
                    }
                }
            };
            if selected.is_empty() && !*amend {
                return Err(GitError::invalid("Nenhuma alteração selecionada"));
            }
            if !matches!(selection, CommitSelection::WorkingPaths(_))
                && selected.iter().any(|p| {
                    snapshot
                        .entries
                        .iter()
                        .any(|e| e.path == *p && (e.untracked || e.index == '.' || e.conflict))
                })
            {
                return Err(GitError::invalid(
                    "Seleção contém arquivo sem alteração preparada no índice",
                ));
            }
        }
        LocalAction::CreateBranch { name, start, .. } => {
            validate_ref(repo, name, true, cancel)?;
            *start = Some(repo.resolve(start.as_deref().unwrap_or("HEAD"), cancel)?);
        }
        LocalAction::CheckoutRevision { revision } => {
            *revision = repo.resolve(revision, cancel)?;
        }
        LocalAction::CheckoutBranch { name } | LocalAction::DeleteBranch { name } => {
            validate_ref(repo, name, true, cancel)?;
            repo.resolve(&format!("refs/heads/{name}"), cancel)?;
        }
        LocalAction::RenameBranch { old, new } => {
            validate_ref(repo, old, true, cancel)?;
            validate_ref(repo, new, true, cancel)?;
            repo.resolve(&format!("refs/heads/{old}"), cancel)?;
        }
        LocalAction::CreateTag {
            name,
            target,
            message,
        } => {
            validate_ref(repo, name, false, cancel)?;
            *target = repo.resolve(target, cancel)?;
            if message
                .as_ref()
                .is_some_and(|m| m.len() > 65536 || m.contains('\0'))
            {
                return Err(GitError::invalid("Mensagem de tag inválida"));
            }
        }
        LocalAction::DeleteTag { name } => validate_ref(repo, name, false, cancel)?,
    }
    Ok(PreparedLocal {
        snapshot,
        action,
        paths: selected,
        patch,
    })
}

struct IndexLock {
    path: PathBuf,
    published: bool,
}
impl IndexLock {
    fn acquire(repo: &Repository) -> Result<Self, GitError> {
        let path = repo.git_dir.join("index.lock");
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| {
                GitError::new(
                    GitErrorKind::Io,
                    format!("Índice ocupado ou inacessível: {e}"),
                )
            })?;
        Ok(Self {
            path,
            published: false,
        })
    }
    fn publish(&mut self, source: &Path, index: &Path) -> Result<(), GitError> {
        let mut dest = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&self.path)?;
        let mut src = fs::File::open(source)?;
        std::io::copy(&mut src, &mut dest)?;
        if let Ok(meta) = fs::metadata(index) {
            dest.set_permissions(meta.permissions())?;
        }
        dest.sync_all()?;
        drop(dest);
        fs::rename(&self.path, index)?;
        self.published = true;
        Ok(())
    }
}
impl Drop for IndexLock {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct TempIndex {
    _dir: tempfile::TempDir,
    path: PathBuf,
}
impl TempIndex {
    fn new(repo: &Repository, copy: bool, cancel: &AtomicBool) -> Result<Self, GitError> {
        let dir = tempfile::Builder::new()
            .prefix("codisseu-index-")
            .tempdir_in(&repo.git_dir)?;
        let index = Self {
            path: dir.path().join("index"),
            _dir: dir,
        };
        if copy && repo.git_dir.join("index").exists() {
            let source = repo.git_dir.join("index");
            let modified = fs::metadata(&source)?.modified()?;
            fs::copy(source, &index.path)?;
            // Git uses the index timestamp to detect racily clean entries. A
            // freshly timestamped copy could hide same-size, same-second edits.
            fs::File::options()
                .write(true)
                .open(&index.path)?
                .set_times(fs::FileTimes::new().set_modified(modified))?;
        } else {
            let mut a = args(&["read-tree"]);
            if copy {
                a.push("--empty".into());
            } else {
                let head = repo.run(&args(&["rev-parse", "--verify", "HEAD"]), None, cancel)?;
                a.push(
                    if head.status.success() {
                        "HEAD"
                    } else {
                        "--empty"
                    }
                    .into(),
                );
            }
            index.run(repo, &a, None, cancel)?;
        }
        Ok(index)
    }
    fn run(
        &self,
        repo: &Repository,
        a: &[OsString],
        input: Option<&[u8]>,
        cancel: &AtomicBool,
    ) -> Result<GitOutput, GitError> {
        repo.run_env(
            a,
            input,
            cancel,
            &[("GIT_INDEX_FILE".into(), self.path.as_os_str().into())],
        )
    }
}
fn with_paths(prefix: &[&str], paths: &[PathBuf]) -> Vec<OsString> {
    let mut a = args(prefix);
    a.push("--".into());
    a.extend(paths.iter().map(|p| p.as_os_str().to_os_string()));
    a
}
fn remove(
    index: &TempIndex,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    if !paths.is_empty() {
        index.run(
            repo,
            &with_paths(&["update-index", "--force-remove"], paths),
            None,
            cancel,
        )?;
    }
    Ok(())
}
fn copy_real_entries(
    index: &TempIndex,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let data = repo
        .run_checked(
            &with_paths(&["ls-files", "--stage", "-z"], paths),
            None,
            cancel,
        )?
        .stdout;
    remove(index, repo, paths, cancel)?;
    if !data.is_empty() {
        index.run(
            repo,
            &args(&["update-index", "-z", "--index-info"]),
            Some(&data),
            cancel,
        )?;
    }
    Ok(())
}
fn copy_head_entries(
    index: &TempIndex,
    repo: &Repository,
    paths: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    if paths.is_empty() {
        return Ok(());
    }
    let raw = repo
        .run_checked(
            &with_paths(&["ls-tree", "-rz", "HEAD"], paths),
            None,
            cancel,
        )?
        .stdout;
    let mut data = Vec::new();
    for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| GitError::invalid("Árvore Git inválida"))?;
        let fields = record[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(GitError::invalid("Árvore Git inválida"));
        }
        data.extend_from_slice(fields[0]);
        data.push(b' ');
        data.extend_from_slice(fields[2]);
        data.extend_from_slice(b" 0\t");
        data.extend_from_slice(&record[tab + 1..]);
        data.push(0);
    }
    remove(index, repo, paths, cancel)?;
    if !data.is_empty() {
        index.run(
            repo,
            &args(&["update-index", "-z", "--index-info"]),
            Some(&data),
            cancel,
        )?;
    }
    Ok(())
}
fn stale(repo: &Repository, before: &RepoSnapshot, cancel: &AtomicBool) -> Result<(), GitError> {
    if repo.snapshot(cancel)? != *before {
        Err(GitError::new(
            GitErrorKind::Stale,
            "O repositório mudou desde a prévia. Atualize e prepare novamente.",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn apply(
    repo: &Repository,
    plan: &PreparedLocal,
    cancel: &AtomicBool,
) -> Result<GitReport, GitError> {
    stale(repo, &plan.snapshot, cancel)?;
    let mut output = Vec::new();
    match &plan.action {
        LocalAction::Stage { .. }
        | LocalAction::Unstage { .. }
        | LocalAction::StageHunks { .. } => {
            let mut lock = IndexLock::acquire(repo)?;
            let index = TempIndex::new(repo, true, cancel)?;
            match &plan.action {
                LocalAction::Stage { paths } => {
                    index.run(repo, &with_paths(&["add", "--all"], paths), None, cancel)?;
                }
                LocalAction::Unstage { paths } => {
                    if plan.snapshot.head.is_some() {
                        index.run(
                            repo,
                            &with_paths(&["reset", "-q", "HEAD"], paths),
                            None,
                            cancel,
                        )?;
                    } else {
                        remove(&index, repo, paths, cancel)?;
                    }
                }
                LocalAction::StageHunks { unstage, .. } => {
                    let mut a = args(&["apply", "--cached", "--whitespace=nowarn"]);
                    if *unstage {
                        a.push("--reverse".into());
                    }
                    index.run(repo, &a, plan.patch.as_deref(), cancel)?;
                }
                _ => unreachable!(),
            }
            stale(repo, &plan.snapshot, cancel)?;
            lock.publish(&index.path, &repo.git_dir.join("index"))?;
        }
        LocalAction::Discard { paths } => {
            if plan.snapshot.head.is_none()
                || paths.iter().any(|p| {
                    plan.snapshot
                        .entries
                        .iter()
                        .any(|e| e.path == *p && (e.untracked || e.index == 'A'))
                })
            {
                return Err(GitError::invalid(
                    "Rollback exige arquivos existentes no HEAD; arquivos novos não são apagados",
                ));
            }
            let result = repo.run_checked(
                &with_paths(
                    &[
                        "restore",
                        &format!("--source={}", plan.snapshot.head.as_deref().unwrap()),
                        "--staged",
                        "--worktree",
                    ],
                    paths,
                ),
                None,
                cancel,
            )?;
            output.extend(result.stdout);
            output.extend(result.stderr.as_bytes());
        }
        LocalAction::Commit {
            selection,
            message,
            amend,
        } => {
            let mut lock = IndexLock::acquire(repo)?;
            let index = TempIndex::new(repo, false, cancel)?;
            if matches!(selection, CommitSelection::WorkingPaths(_)) {
                index.run(
                    repo,
                    &with_paths(&["add", "--all"], &plan.paths),
                    None,
                    cancel,
                )?;
            } else {
                copy_real_entries(&index, repo, &plan.paths, cancel)?;
            }
            stale(repo, &plan.snapshot, cancel)?;
            let mut a = args(&["commit", "--file", "-"]);
            if *amend {
                a.push("--amend".into());
            }
            let result = index.run(repo, &a, Some(message.as_bytes()), cancel)?;
            output.extend(result.stdout);
            output.extend(result.stderr.as_bytes());
            let merged = TempIndex::new(repo, true, cancel)?;
            copy_head_entries(&merged, repo, &plan.paths, cancel)?;
            if file_hash(&repo.git_dir.join("index"), cancel, 256 * 1024 * 1024)?
                != plan.snapshot.index_hash
            {
                return Err(GitError::new(
                    GitErrorKind::Stale,
                    "Commit criado, mas o índice mudou externamente. Atualize o estado antes de continuar.",
                ));
            }
            lock.publish(&merged.path, &repo.git_dir.join("index"))?;
        }
        other => {
            let a = match other {
                LocalAction::CreateBranch {
                    name,
                    start,
                    checkout,
                } => {
                    if *checkout {
                        args(&["switch", "-c", name, start.as_deref().unwrap()])
                    } else {
                        args(&["branch", name, start.as_deref().unwrap()])
                    }
                }
                LocalAction::CheckoutRevision { revision } => {
                    args(&["switch", "--detach", revision])
                }
                LocalAction::CheckoutBranch { name } => args(&["switch", "--", name]),
                LocalAction::RenameBranch { old, new } => args(&["branch", "-m", old, new]),
                LocalAction::DeleteBranch { name } => args(&["branch", "-d", "--", name]),
                LocalAction::CreateTag {
                    name,
                    target,
                    message,
                } => {
                    if let Some(message) = message {
                        args(&["tag", "-a", "-m", message, "--", name, target])
                    } else {
                        args(&["tag", "--", name, target])
                    }
                }
                LocalAction::DeleteTag { name } => args(&["tag", "-d", "--", name]),
                _ => unreachable!(),
            };
            let result = checked(repo.run(&a, None, cancel)?)?;
            output.extend(result.stdout);
            output.extend(result.stderr.as_bytes());
        }
    }
    Ok(GitReport {
        success: true,
        summary: "Operação Git concluída; estado atualizado".into(),
        output: crate::redact(&String::from_utf8_lossy(&output)),
        outcome_unknown: false,
    })
}

pub(crate) fn selected_hunks_patch(diff: &GitDiff, hunks: &[usize]) -> Result<Vec<u8>, GitError> {
    let mut combined = Vec::new();
    for (i, index) in hunks.iter().enumerate() {
        let hunk = diff
            .hunks
            .get(*index)
            .ok_or_else(|| GitError::invalid("Hunk não existe na diferença exibida"))?;
        if i == 0 {
            combined.extend_from_slice(&hunk.patch);
        } else {
            let start = hunk
                .patch
                .windows(4)
                .position(|w| w == b"\n@@ ")
                .map(|i| i + 1)
                .ok_or_else(|| GitError::invalid("Patch sem hunk"))?;
            combined.extend_from_slice(&hunk.patch[start..]);
        }
    }
    Ok(combined)
}
pub(crate) fn prepared_hunks_match(plan: &PreparedLocal, expected: &[u8]) -> bool {
    plan.patch.as_deref() == Some(expected)
}
