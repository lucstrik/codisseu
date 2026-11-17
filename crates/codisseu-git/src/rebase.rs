//! Explicit bounded interactive rebase using the installed Git sequencer and a
//! Rust editor entry point. Todo commands are generated, never user shell text.
use crate::{
    repository::{args, valid_oid},
    workflows, *,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RebaseAction {
    Pick,
    Reword,
    Edit,
    Squash,
    Fixup,
    Drop,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RebaseStep {
    pub oid: String,
    pub action: RebaseAction,
    pub message: Option<String>,
}
#[derive(Clone, Debug)]
pub struct RebaseCommit {
    pub commit: CommitEntry,
    pub message: String,
}
#[derive(Clone, Debug)]
pub struct RebaseSeries {
    pub base: String,
    pub head: String,
    pub branch: String,
    pub commits: Vec<RebaseCommit>,
}
#[derive(Clone, Debug)]
pub struct RebaseInspection {
    series: RebaseSeries,
    snapshot: RepoSnapshot,
}
impl RebaseInspection {
    pub fn series(&self) -> &RebaseSeries {
        &self.series
    }
}
#[derive(Clone, Debug)]
pub struct PreparedRebase {
    inspection: RebaseInspection,
    saved: Saved,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SavedStep {
    oid: String,
    action: RebaseAction,
    message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Saved {
    git_dir: PathBuf,
    base: String,
    head: String,
    todo: String,
    steps: Vec<SavedStep>,
}
fn state_path(repo: &Repository) -> PathBuf {
    repo.git_dir.join("codisseu/interactive-rebase.json")
}
pub fn inspect_rebase(
    repo: &Repository,
    base: &str,
    cancel: &AtomicBool,
) -> Result<RebaseInspection, GitError> {
    let snapshot = repo.snapshot(cancel)?;
    if snapshot.operation.is_some() || !snapshot.entries.is_empty() {
        return Err(GitError::invalid(
            "Rebase interativo exige árvore e índice limpos e nenhuma operação em andamento",
        ));
    }
    let branch = snapshot
        .head_ref
        .as_deref()
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .ok_or_else(|| GitError::invalid("Rebase interativo exige branch local ativa"))?
        .to_owned();
    let head = snapshot
        .head
        .clone()
        .ok_or_else(|| GitError::invalid("Não existe HEAD"))?;
    let base = repo.resolve(base, cancel)?;
    if !repo
        .run(
            &args(&["merge-base", "--is-ancestor", &base, &head]),
            None,
            cancel,
        )?
        .status
        .success()
    {
        return Err(GitError::invalid("A base precisa ser ancestral de HEAD"));
    }
    let raw = repo
        .run_checked(
            &args(&[
                "rev-list",
                "--reverse",
                "--topo-order",
                "--max-count=101",
                &format!("{base}..{head}"),
                "--",
            ]),
            None,
            cancel,
        )?
        .stdout;
    let ids = String::from_utf8_lossy(&raw)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if ids.is_empty() || ids.len() > 100 {
        return Err(GitError::invalid(
            "Selecione uma série de1a100commits após a base",
        ));
    }
    let mut commits = Vec::new();
    let mut previous = base.clone();
    for oid in ids {
        let page = repo.history_reference(0, 1, None, Some(&oid), cancel)?;
        let commit = page
            .commits
            .into_iter()
            .next()
            .ok_or_else(|| GitError::invalid("Commit ausente"))?;
        if commit.parents != vec![previous] {
            return Err(GitError::unsupported(
                "Rebase interativo suporta série linear sem merges",
            ));
        }
        let raw = repo
            .run_checked(
                &args(&[
                    "show",
                    "--no-patch",
                    "--encoding=UTF-8",
                    "--format=%B",
                    &oid,
                ]),
                None,
                cancel,
            )?
            .stdout;
        let message = String::from_utf8_lossy(crate::repository::trim_line(&raw)).into_owned();
        if message.len() > 65536 {
            return Err(GitError::new(
                GitErrorKind::OutputLimit,
                "Mensagem de commit maior que64KiB",
            ));
        }
        commits.push(RebaseCommit { commit, message });
        previous = oid;
    }
    if repo.snapshot(cancel)? != snapshot {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Série mudou durante a leitura",
        ));
    }
    Ok(RebaseInspection {
        series: RebaseSeries {
            base,
            head,
            branch,
            commits,
        },
        snapshot,
    })
}
pub fn prepare_rebase(
    repo: &Repository,
    inspection: &RebaseInspection,
    steps: &[RebaseStep],
    cancel: &AtomicBool,
) -> Result<PreparedRebase, GitError> {
    if repo.snapshot(cancel)? != inspection.snapshot {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Série mudou desde a leitura; atualize",
        ));
    }
    let expected = inspection
        .series
        .commits
        .iter()
        .map(|c| c.commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    let actual = steps
        .iter()
        .map(|s| s.oid.as_str())
        .collect::<BTreeSet<_>>();
    if steps.len() != expected.len() || actual != expected || actual.len() != steps.len() {
        return Err(GitError::invalid(
            "Cada commit da série deve aparecer exatamente uma vez",
        ));
    }
    let mut todo = String::new();
    let mut saved = Vec::new();
    let mut group: Option<String> = None;
    for step in steps {
        if !valid_oid(&step.oid) {
            return Err(GitError::invalid("OID de rebase inválido"));
        }
        let original = &inspection
            .series
            .commits
            .iter()
            .find(|c| c.commit.oid == step.oid)
            .unwrap()
            .message;
        if step
            .message
            .as_ref()
            .is_some_and(|m| m.trim().is_empty() || m.len() > 65536 || m.contains('\0'))
        {
            return Err(GitError::invalid(
                "Mensagem vazia, inválida ou maior que64KiB",
            ));
        }
        let action = match step.action {
            RebaseAction::Pick => "pick",
            RebaseAction::Reword => "reword",
            RebaseAction::Edit => "edit",
            RebaseAction::Squash => "squash",
            RebaseAction::Fixup => "fixup",
            RebaseAction::Drop => "drop",
        };
        let message = match step.action {
            RebaseAction::Reword => step
                .message
                .clone()
                .ok_or_else(|| GitError::invalid("Reword exige a nova mensagem completa"))?,
            RebaseAction::Squash => {
                let before = group.as_ref().ok_or_else(|| {
                    GitError::invalid("Squash exige um commit anterior preservado")
                })?;
                step.message.clone().unwrap_or_else(|| {
                    format!("{}\n\n{}\n", before.trim_end(), original.trim_end())
                })
            }
            RebaseAction::Fixup => group
                .clone()
                .ok_or_else(|| GitError::invalid("Fixup exige um commit anterior preservado"))?,
            _ => original.clone(),
        };
        if step.action != RebaseAction::Drop {
            group = Some(message.clone());
        }
        todo.push_str(&format!("{action} {}\n", step.oid));
        saved.push(SavedStep {
            oid: step.oid.clone(),
            action: step.action,
            message,
        });
    }
    let saved = Saved {
        git_dir: repo.git_dir.clone(),
        base: inspection.series.base.clone(),
        head: inspection.series.head.clone(),
        todo,
        steps: saved,
    };
    if serde_json::to_vec(&saved)
        .map_err(|e| GitError::invalid(e.to_string()))?
        .len()
        > 1024 * 1024
    {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Plano de rebase excede1MiB",
        ));
    }
    Ok(PreparedRebase {
        inspection: inspection.clone(),
        saved,
    })
}
fn load(path: &Path) -> Result<Saved, GitError> {
    let m = fs::symlink_metadata(path)?;
    if !m.is_file() || m.len() > 1024 * 1024 {
        return Err(GitError::invalid("Estado de rebase inválido"));
    }
    let saved: Saved = serde_json::from_slice(&fs::read(path)?)
        .map_err(|_| GitError::invalid("Estado de rebase corrompido"))?;
    if saved.steps.len() > 100
        || !valid_oid(&saved.base)
        || !valid_oid(&saved.head)
        || saved
            .steps
            .iter()
            .any(|s| !valid_oid(&s.oid) || s.message.len() > 65536 || s.message.contains('\0'))
    {
        return Err(GitError::invalid("Estado de rebase fora do contrato"));
    }
    Ok(saved)
}
fn editor_environment(
    repo: &Repository,
    helper: &Path,
) -> Result<(tempfile::TempDir, Vec<(OsString, OsString)>), GitError> {
    #[cfg(not(unix))]
    {
        let _ = (repo, helper);
        Err(GitError::unsupported(
            "Helper de rebase implementado em Unix",
        ))
    }
    #[cfg(unix)]
    {
        let dir = tempfile::Builder::new()
            .prefix("codisseu-git-editor-")
            .tempdir_in("/tmp")?;
        let editor = dir.path().join("codisseu-git-sequence-editor");
        std::os::unix::fs::symlink(fs::canonicalize(helper)?, &editor)?;
        if !editor
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
        {
            return Err(GitError::invalid("Caminho de helper não pode exigir shell"));
        }
        let env = vec![
            ("GIT_SEQUENCE_EDITOR".into(), editor.as_os_str().into()),
            ("GIT_EDITOR".into(), editor.as_os_str().into()),
            (
                "CODISSEU_GIT_HELPER_STATE".into(),
                state_path(repo).into_os_string(),
            ),
        ];
        Ok((dir, env))
    }
}
fn run(
    repo: &Repository,
    a: &[OsString],
    helper: &Path,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    let (_editor, env) = editor_environment(repo, helper)?;
    // The UI supplies the full intended message without generated Git comments.
    // Keep literal '#' lines instead of applying editor cleanup a second time.
    let mut command = args(&["-c", "commit.cleanup=verbatim"]);
    command.extend_from_slice(a);
    let result = crate::process::run(
        &repo.root,
        &command,
        None,
        cancel,
        Duration::from_secs(120),
        &env,
    );
    let mut report = workflows::report(repo, result, cancel);
    if report.success && report.operation == Some(OperationKind::Rebase) {
        report.summary =
            "Rebase pausado para edição. Prepare as alterações e continue ou aborte.".into();
    }
    if workflows::active_operation(repo) != Some(OperationKind::Rebase) {
        let _ = fs::remove_file(state_path(repo));
    }
    Ok(report)
}
pub fn apply_rebase(
    repo: &Repository,
    plan: &PreparedRebase,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    apply_rebase_with_helper(repo, plan, &std::env::current_exe()?, cancel)
}
pub fn apply_rebase_with_helper(
    repo: &Repository,
    plan: &PreparedRebase,
    helper: &Path,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    if repo.snapshot(cancel)? != plan.inspection.snapshot {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Plano de rebase obsoleto; prepare novamente",
        ));
    }
    let path = state_path(repo);
    let parent = path.parent().unwrap();
    if parent.exists() && fs::symlink_metadata(parent)?.file_type().is_symlink() {
        return Err(GitError::unsupported("Diretório de estado é symlink"));
    }
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(
        &serde_json::to_vec(&plan.saved).map_err(|e| GitError::invalid(e.to_string()))?,
    )?;
    file.as_file().sync_all()?;
    file.persist(&path).map_err(|e| GitError::from(e.error))?;
    run(
        repo,
        &args(&[
            "-c",
            "rebase.abbreviateCommands=false",
            "-c",
            "rebase.autoSquash=false",
            "rebase",
            "--interactive",
            "--force-rebase",
            "--no-autostash",
            "--no-update-refs",
            "--no-autosquash",
            "--empty=keep",
            &plan.saved.base,
        ]),
        helper,
        cancel,
    )
}
pub fn continue_rebase_with_helper(
    repo: &Repository,
    abort: bool,
    helper: &Path,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    if abort {
        return run(repo, &args(&["rebase", "--abort"]), helper, cancel);
    }
    let saved = load(&state_path(repo))?;
    if saved.git_dir != repo.git_dir {
        return Err(GitError::invalid("Estado pertence a outro repositório"));
    }
    run(
        repo,
        &args(&["rebase", if abort { "--abort" } else { "--continue" }]),
        helper,
        cancel,
    )
}
pub(crate) fn managed(repo: &Repository) -> bool {
    state_path(repo).is_file()
}
pub(crate) fn continue_rebase(
    repo: &Repository,
    abort: bool,
    cancel: &AtomicBool,
) -> Result<workflows::WorkflowReport, GitError> {
    continue_rebase_with_helper(repo, abort, &std::env::current_exe()?, cancel)
}
/// Call before initializing the GUI. Returns Some only for our editor symlink.
pub fn helper_main() -> Option<i32> {
    let mut args = std::env::args_os();
    let executable = PathBuf::from(args.next()?);
    if executable.file_name() != Some(std::ffi::OsStr::new("codisseu-git-sequence-editor")) {
        return None;
    }
    let state = std::env::var_os("CODISSEU_GIT_HELPER_STATE")?;
    let result = (|| -> Result<(), GitError> {
        let target = PathBuf::from(
            args.next()
                .ok_or_else(|| GitError::invalid("Editor sem arquivo"))?,
        );
        if args.next().is_some() {
            return Err(GitError::invalid("Argumentos inesperados do editor"));
        }
        let saved = load(Path::new(&state))?;
        let parent = fs::canonicalize(
            target
                .parent()
                .ok_or_else(|| GitError::invalid("Arquivo sem diretório"))?,
        )?;
        if parent != saved.git_dir && parent != saved.git_dir.join("rebase-merge") {
            return Err(GitError::invalid("Editor fora do estado Git"));
        }
        let metadata = fs::symlink_metadata(&target)?;
        if !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err(GitError::invalid("Arquivo de editor inválido"));
        }
        let content = if target.file_name() == Some(std::ffi::OsStr::new("git-rebase-todo")) {
            let mut generated = String::new();
            for step in &saved.steps {
                let action = match step.action {
                    RebaseAction::Pick => "pick",
                    RebaseAction::Reword => "reword",
                    RebaseAction::Edit => "edit",
                    RebaseAction::Squash => "squash",
                    RebaseAction::Fixup => "fixup",
                    RebaseAction::Drop => "drop",
                };
                generated.push_str(&format!("{action} {}\n", step.oid));
            }
            generated
        } else {
            let done = fs::read_to_string(saved.git_dir.join("rebase-merge/done"))?;
            let oid = done
                .lines()
                .rfind(|l| !l.starts_with('#'))
                .and_then(|l| l.split_whitespace().nth(1))
                .ok_or_else(|| GitError::invalid("Rebase sem ação atual"))?;
            saved
                .steps
                .iter()
                .find(|s| s.oid == oid)
                .ok_or_else(|| GitError::invalid("Ação fora do plano autorizado"))?
                .message
                .clone()
        };
        fs::write(&target, content)?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("Codisseu Git editor: {error}");
        Some(1)
    } else {
        Some(0)
    }
}

/// Bind a Continue/Abort approval to the sequencer files as well as HEAD/index.
pub(crate) fn state_fingerprint(
    repo: &Repository,
    cancel: &AtomicBool,
) -> Result<String, GitError> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for name in [
        "codisseu/interactive-rebase.json",
        "rebase-merge/git-rebase-todo",
        "rebase-merge/done",
        "rebase-merge/head-name",
        "rebase-merge/orig-head",
        "rebase-merge/onto",
        "rebase-merge/stopped-sha",
        "rebase-merge/amend",
        "rebase-merge/message",
        "rebase-merge/message-squash",
        "rebase-merge/message-fixup",
        "REBASE_HEAD",
    ] {
        hash.update(name.as_bytes());
        hash.update(
            crate::repository::file_hash(&repo.git_dir.join(name), cancel, 2 * 1024 * 1024)?
                .as_bytes(),
        );
    }
    Ok(format!("{:x}", hash.finalize()))
}
