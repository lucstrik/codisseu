//! Explicit, snapshot-bound Git workflows. No command is run through a shell.
use crate::{GitError, GitOutput, RepoSnapshot, Repository};
use std::{
    ffi::OsString,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullMode {
    FastForwardOnly,
    Merge,
    Rebase,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetMode {
    Soft,
    Mixed,
    Hard,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowAction {
    Merge {
        revision: String,
    },
    Rebase {
        onto: String,
    },
    Continue(OperationKind),
    Abort(OperationKind),
    StashCreate {
        message: String,
        include_untracked: bool,
    },
    StashApply {
        stash: String,
        pop: bool,
    },
    StashDrop {
        stash: String,
    },
    Fetch {
        remote: String,
    },
    Pull {
        remote: String,
        branch: String,
        mode: PullMode,
    },
    Push {
        remote: String,
        branch: String,
    },
    PushTo {
        remote: String,
        source: String,
        target: String,
        tags: bool,
    },
    PushWithTags {
        remote: String,
        source: String,
        target: String,
        mode: crate::sync::PushTagsMode,
    },
    StashCreateOptions {
        message: String,
        include_untracked: bool,
        keep_index: bool,
    },
    CherryPick {
        revision: String,
    },
    Revert {
        revision: String,
    },
    Reset {
        revision: String,
        mode: ResetMode,
    },
}
#[derive(Clone, Debug)]
pub struct WorkflowPreview {
    pub action: WorkflowAction,
    pub summary: String,
    pub warnings: Vec<String>,
    pub destructive: bool,
    pub changes_worktree: bool,
}
#[derive(Clone, Debug)]
pub struct PreparedWorkflow {
    pub preview: WorkflowPreview,
    action: WorkflowAction,
    snapshot: RepoSnapshot,
    target: Option<String>,
    stash: Option<StashEntry>,
    remote_urls: Vec<u8>,
    push: Option<crate::sync::PreparedPush>,
    rebase_state: Option<String>,
}
#[derive(Clone, Debug)]
pub struct WorkflowReport {
    pub success: bool,
    pub summary: String,
    pub output: String,
    pub conflicts: Vec<PathBuf>,
    pub operation: Option<OperationKind>,
    pub outcome_unknown: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashEntry {
    pub created_unix: i64,
    pub reference: String,
    pub oid: String,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    pub name: String,
    pub fetch_url: String,
    pub push_url: String,
}

pub fn list_remotes(repo: &Repository, cancel: &AtomicBool) -> Result<Vec<RemoteInfo>, GitError> {
    let output = repo.run_checked(&args(&["remote"]), None, cancel)?;
    let mut result = Vec::new();
    for name in output
        .stdout
        .split(|b| *b == b'\n')
        .filter(|s| !s.is_empty())
    {
        let name = std::str::from_utf8(name)
            .map_err(|_| GitError::unsupported("Nome de remoto sem UTF-8"))?;
        result.push(RemoteInfo {
            name: name.into(),
            fetch_url: crate::redact(
                String::from_utf8_lossy(&remote_urls(repo, name, false, cancel)?).trim(),
            ),
            push_url: crate::redact(
                String::from_utf8_lossy(&remote_urls(repo, name, true, cancel)?).trim(),
            ),
        });
    }
    Ok(result)
}

fn args(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}
fn text(output: GitOutput) -> Result<String, GitError> {
    String::from_utf8(output.stdout)
        .map(|s| s.trim().to_owned())
        .map_err(|_| GitError::invalid("Git retornou identificador sem UTF-8"))
}
fn safe_name(name: &str) -> Result<(), GitError> {
    if name.is_empty()
        || name.starts_with('-')
        || name.len() > 4096
        || name.contains(['\0', '\r', '\n'])
    {
        return Err(GitError::invalid("Referência Git inválida"));
    }
    Ok(())
}
fn oid(repo: &Repository, revision: &str, cancel: &AtomicBool) -> Result<String, GitError> {
    safe_name(revision)?;
    let value = text(repo.run_checked(
        &args(&[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ]),
        None,
        cancel,
    )?)?;
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(GitError::invalid("Git retornou objeto de commit inválido"));
    }
    Ok(value)
}
pub fn active_operation(repo: &Repository) -> Option<OperationKind> {
    if repo.git_dir.join("rebase-merge").exists() || repo.git_dir.join("rebase-apply").exists() {
        Some(OperationKind::Rebase)
    } else if repo.git_dir.join("MERGE_HEAD").exists() {
        Some(OperationKind::Merge)
    } else if repo.git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(OperationKind::CherryPick)
    } else if repo.git_dir.join("REVERT_HEAD").exists() {
        Some(OperationKind::Revert)
    } else {
        None
    }
}
fn clean(repo: &Repository, cancel: &AtomicBool) -> Result<(), GitError> {
    if !repo
        .run_checked(
            &args(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]),
            None,
            cancel,
        )?
        .stdout
        .is_empty()
    {
        return Err(GitError::invalid(
            "Há alterações locais ou arquivos não versionados. Faça commit ou stash explícito antes desta operação.",
        ));
    }
    Ok(())
}
fn branch(repo: &Repository, name: &str, cancel: &AtomicBool) -> Result<(), GitError> {
    safe_name(name)?;
    repo.run_checked(
        &args(&["check-ref-format", &format!("refs/heads/{name}")]),
        None,
        cancel,
    )?;
    Ok(())
}
pub(crate) fn remote_urls(
    repo: &Repository,
    name: &str,
    push: bool,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, GitError> {
    safe_name(name)?;
    let remotes = repo.run_checked(&args(&["remote"]), None, cancel)?;
    if !remotes
        .stdout
        .split(|b| *b == b'\n')
        .any(|r| r == name.as_bytes())
    {
        return Err(GitError::invalid(
            "Selecione um remoto configurado neste repositório",
        ));
    }
    let mut command = args(&["remote", "get-url", "--all"]);
    if push {
        command.push("--push".into());
    }
    command.push(name.into());
    Ok(repo.run_checked(&command, None, cancel)?.stdout)
}
pub fn list_stashes(repo: &Repository, cancel: &AtomicBool) -> Result<Vec<StashEntry>, GitError> {
    let output = repo.run_checked(
        &args(&[
            "stash",
            "list",
            "-z",
            "--format=%H%x09%gd%x09%ct%x09%gs",
            "--max-count=500",
        ]),
        None,
        cancel,
    )?;
    let mut result = Vec::new();
    for record in output.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let mut fields = record.splitn(4, |b| *b == b'\t');
        let identity = fields.next().unwrap_or_default();
        let reference = fields
            .next()
            .ok_or_else(|| GitError::invalid("Lista de stash inválida"))?;
        let created_unix = fields
            .next()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| GitError::invalid("Data de stash inválida"))?;
        let message = fields
            .next()
            .ok_or_else(|| GitError::invalid("Mensagem de stash ausente"))?;
        let identity = std::str::from_utf8(identity)
            .map_err(|_| GitError::invalid("OID de stash inválido"))?;
        let reference = std::str::from_utf8(reference)
            .map_err(|_| GitError::invalid("Referência de stash inválida"))?;
        if !matches!(identity.len(), 40 | 64)
            || !identity.bytes().all(|b| b.is_ascii_hexdigit())
            || !reference.starts_with("stash@{")
        {
            return Err(GitError::invalid("Identidade de stash inválida"));
        }
        result.push(StashEntry {
            created_unix,
            reference: reference.into(),
            oid: identity.into(),
            message: String::from_utf8_lossy(message).into(),
        });
    }
    Ok(result)
}
fn selected_stash(
    repo: &Repository,
    reference: &str,
    cancel: &AtomicBool,
) -> Result<StashEntry, GitError> {
    safe_name(reference)?;
    list_stashes(repo, cancel)?
        .into_iter()
        .find(|s| s.reference == reference)
        .ok_or_else(|| GitError::invalid("Stash não existe mais; atualize a lista"))
}
pub fn prepare_workflow(
    repo: &Repository,
    action: WorkflowAction,
    cancel: &AtomicBool,
) -> Result<PreparedWorkflow, GitError> {
    let snapshot = repo.snapshot(cancel)?;
    let mut plan = PreparedWorkflow {
        action: action.clone(),
        preview: WorkflowPreview {
            action: action.clone(),
            summary: String::new(),
            warnings: Vec::new(),
            destructive: false,
            changes_worktree: true,
        },
        snapshot,
        target: None,
        stash: None,
        remote_urls: Vec::new(),
        push: None,
        rebase_state: None,
    };
    if let Some(current) = active_operation(repo)
        && !matches!(action, WorkflowAction::Continue(kind) | WorkflowAction::Abort(kind) if kind == current)
    {
        return Err(GitError::invalid(
            "Conclua ou aborte a operação Git em andamento primeiro",
        ));
    }
    let summary = match &action {
        WorkflowAction::Merge { revision } => {
            clean(repo, cancel)?;
            plan.target = Some(oid(repo, revision, cancel)?);
            format!("Integrar {revision} na branch atual")
        }
        WorkflowAction::Rebase { onto } => {
            clean(repo, cancel)?;
            plan.target = Some(oid(repo, onto, cancel)?);
            plan.preview.warnings.push("Rebase reescreve os commits locais; outras branches não serão atualizadas automaticamente.".into());
            format!("Reaplicar commits locais sobre {onto}")
        }
        WorkflowAction::Continue(kind) => {
            if active_operation(repo) != Some(*kind) {
                return Err(GitError::invalid("Não há esta operação em andamento"));
            }
            if *kind == OperationKind::Rebase {
                plan.rebase_state = Some(crate::rebase::state_fingerprint(repo, cancel)?);
            }
            format!("Continuar {kind:?} após resolver e marcar os conflitos")
        }
        WorkflowAction::Abort(kind) => {
            if active_operation(repo) != Some(*kind) {
                return Err(GitError::invalid("Não há esta operação em andamento"));
            }
            if *kind == OperationKind::Rebase {
                plan.rebase_state = Some(crate::rebase::state_fingerprint(repo, cancel)?);
            }
            plan.preview.destructive = true;
            plan.preview.warnings.push("Alterações feitas durante a resolução poderão ser descartadas ao restaurar o estado anterior da operação.".into());
            format!("Abortar {kind:?} e restaurar o estado anterior")
        }
        WorkflowAction::StashCreate {
            message,
            include_untracked,
        } => {
            if message.len() > 8192 || message.contains('\0') {
                return Err(GitError::invalid("Mensagem de stash inválida"));
            }
            format!(
                "Guardar alterações locais em stash{}",
                if *include_untracked {
                    ", incluindo arquivos não versionados"
                } else {
                    ""
                }
            )
        }
        WorkflowAction::StashApply { stash, pop } => {
            let selected = selected_stash(repo, stash, cancel)?;
            plan.target = Some(selected.oid.clone());
            plan.stash = Some(selected);
            format!(
                "Aplicar {stash}{}",
                if *pop {
                    " e removê-lo somente após aplicação bem-sucedida"
                } else {
                    " sem removê-lo"
                }
            )
        }
        WorkflowAction::StashDrop { stash } => {
            let selected = selected_stash(repo, stash, cancel)?;
            plan.target = Some(selected.oid.clone());
            plan.stash = Some(selected);
            plan.preview.destructive = true;
            plan.preview.changes_worktree = false;
            plan.preview
                .warnings
                .push("O stash será retirado da lista; não há restauração automática.".into());
            format!("Remover {stash}")
        }
        WorkflowAction::Fetch { remote } => {
            plan.remote_urls = remote_urls(repo, remote, false, cancel)?;
            plan.preview.changes_worktree = false;
            format!("Buscar referências de {remote}; arquivos locais não serão integrados")
        }
        WorkflowAction::Pull {
            remote,
            branch: name,
            mode,
        } => {
            clean(repo, cancel)?;
            branch(repo, name, cancel)?;
            plan.remote_urls = remote_urls(repo, remote, false, cancel)?;
            format!("Buscar {remote}/{name} e integrar por {mode:?}")
        }
        WorkflowAction::Push {
            remote,
            branch: name,
        } => {
            branch(repo, name, cancel)?;
            plan.target = Some(oid(repo, &format!("refs/heads/{name}"), cancel)?);
            plan.remote_urls = remote_urls(repo, remote, true, cancel)?;
            plan.preview.changes_worktree = false;
            plan.preview.warnings.push("Envia somente commits da branch selecionada; alterações não commitadas não são enviadas. Push forçado não é usado.".into());
            format!("Enviar a branch {name} para {remote}/{name}")
        }
        WorkflowAction::PushTo {
            remote,
            source,
            target,
            tags,
        } => {
            let inspection = crate::sync::inspect_push(repo, remote, source, target, cancel)?;
            plan.push = Some(crate::sync::prepare_push(repo, &inspection, *tags, cancel)?);
            plan.preview.changes_worktree = false;
            format!("Enviar {source} para {remote}/{target}")
        }
        WorkflowAction::PushWithTags {
            remote,
            source,
            target,
            mode,
        } => {
            let inspection = crate::sync::inspect_push(repo, remote, source, target, cancel)?;
            plan.push = Some(crate::sync::prepare_push_mode(
                repo,
                &inspection,
                *mode,
                cancel,
            )?);
            plan.preview.changes_worktree = false;
            format!("Enviar {source} para {remote}/{target}")
        }
        WorkflowAction::StashCreateOptions {
            message,
            include_untracked,
            keep_index,
        } => {
            if message.len() > 8192 || message.contains('\0') {
                return Err(GitError::invalid("Mensagem de stash inválida"));
            }
            format!(
                "Guardar alterações locais em stash{}{}",
                if *keep_index {
                    " preservando o índice"
                } else {
                    ""
                },
                if *include_untracked {
                    ", incluindo arquivos não versionados"
                } else {
                    ""
                }
            )
        }
        WorkflowAction::CherryPick { revision } => {
            clean(repo, cancel)?;
            plan.target = Some(oid(repo, revision, cancel)?);
            format!("Aplicar o commit {revision} na branch atual")
        }
        WorkflowAction::Revert { revision } => {
            clean(repo, cancel)?;
            plan.target = Some(oid(repo, revision, cancel)?);
            format!("Criar commit que desfaz {revision}")
        }
        WorkflowAction::Reset { revision, mode } => {
            plan.target = Some(oid(repo, revision, cancel)?);
            plan.preview.destructive = true;
            plan.preview.changes_worktree = *mode == ResetMode::Hard;
            plan.preview.warnings.push(match mode { ResetMode::Soft=>"Move a branch; conserva índice e arquivos.",ResetMode::Mixed=>"Move a branch e substitui o índice; conserva os arquivos.",ResetMode::Hard=>"Descarta alterações de arquivos rastreados e pode remover arquivos não versionados que obstruam caminhos restaurados." }.into());
            format!("Reset {mode:?} para {revision}")
        }
    };
    plan.preview.summary = summary;
    if repo.snapshot(cancel)? != plan.snapshot {
        return Err(GitError::invalid(
            "Repositório mudou durante a preparação; prepare novamente",
        ));
    }
    Ok(plan)
}
pub fn unmerged_paths(repo: &Repository, cancel: &AtomicBool) -> Result<Vec<PathBuf>, GitError> {
    let output = repo.run_checked(
        &args(&["diff", "--name-only", "--diff-filter=U", "-z"]),
        None,
        cancel,
    )?;
    output
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(path_from_bytes)
        .collect()
}
pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, GitError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
    }
    #[cfg(not(unix))]
    {
        Ok(PathBuf::from(std::str::from_utf8(bytes).map_err(|_| {
            GitError::unsupported("Caminho Git sem Unicode nesta plataforma")
        })?))
    }
}
fn render_output(output: &GitOutput) -> String {
    let raw = format!(
        "{}{}{}",
        String::from_utf8_lossy(&output.stdout),
        if output.stdout.is_empty() || output.stderr.is_empty() {
            ""
        } else {
            "\n"
        },
        output.stderr
    );
    // Never expose URL userinfo or query credentials from remote porcelain output.
    raw.split_inclusive(char::is_whitespace)
        .map(|part| {
            let Some(scheme) = part.find("://") else {
                return part.to_owned();
            };
            let start = scheme + 3;
            let end = part[start..].find('/').map_or(part.len(), |n| start + n);
            let mut safe = part.to_owned();
            if let Some(at) = part[start..end].rfind('@') {
                safe.replace_range(start..start + at, "[redacted]");
            }
            if let Some(query) = safe.find('?') {
                safe.truncate(query);
                safe.push_str("?[redacted]");
            }
            safe
        })
        .collect()
}
pub(crate) fn report(
    repo: &Repository,
    result: Result<GitOutput, GitError>,
    cancel: &AtomicBool,
) -> WorkflowReport {
    let unknown = result.is_err();
    let (success, output) = match result {
        Ok(out) => (out.status.success(), render_output(&out)),
        Err(error) => (false, error.to_string()),
    };
    let conflicts = if cancel.load(Ordering::Relaxed) {
        Vec::new()
    } else {
        unmerged_paths(repo, cancel).unwrap_or_default()
    };
    WorkflowReport {
        success,
        summary: if success {
            "Operação Git concluída".into()
        } else if !conflicts.is_empty() {
            "A operação parou com conflitos; os arquivos e o estado Git foram preservados para resolução".into()
        } else {
            "A operação não foi concluída; confira o estado atual antes de repetir".into()
        },
        output,
        conflicts,
        operation: active_operation(repo),
        outcome_unknown: unknown,
    }
}
pub fn apply_workflow(
    repo: &Repository,
    plan: &PreparedWorkflow,
    cancel: &AtomicBool,
) -> Result<WorkflowReport, GitError> {
    if repo.snapshot(cancel)? != plan.snapshot {
        return Err(GitError::invalid(
            "HEAD, índice ou arquivos mudaram após a prévia; prepare novamente",
        ));
    }
    if plan.preview.action != plan.action {
        return Err(GitError::invalid("Prévia Git alterada; prepare novamente"));
    }
    if let Some(hash) = &plan.rebase_state
        && crate::rebase::state_fingerprint(repo, cancel)? != *hash
    {
        return Err(GitError::new(
            crate::GitErrorKind::Stale,
            "Plano do sequenciador mudou após a prévia; atualize",
        ));
    }
    let action = &plan.action;
    if let Some(expected) = &plan.stash
        && selected_stash(repo, &expected.reference, cancel)? != *expected
    {
        return Err(GitError::invalid("A lista de stash mudou após a prévia"));
    }
    let target = plan.target.as_deref().unwrap_or_default();
    if matches!(
        action,
        WorkflowAction::PushTo { .. } | WorkflowAction::PushWithTags { .. }
    ) {
        return crate::sync::apply_push(
            repo,
            plan.push
                .as_ref()
                .ok_or_else(|| GitError::invalid("Plano de Push ausente"))?,
            cancel,
        );
    }
    if matches!(
        action,
        WorkflowAction::Continue(OperationKind::Rebase)
            | WorkflowAction::Abort(OperationKind::Rebase)
    ) && crate::rebase::managed(repo)
    {
        return crate::rebase::continue_rebase(
            repo,
            matches!(action, WorkflowAction::Abort(_)),
            cancel,
        );
    }
    let command = match action {
        WorkflowAction::Merge { .. } => {
            args(&["merge", "--no-edit", "--no-autostash", "--", target])
        }
        WorkflowAction::Rebase { .. } => {
            args(&["rebase", "--no-autostash", "--no-update-refs", target])
        }
        WorkflowAction::Continue(kind) | WorkflowAction::Abort(kind) => {
            if active_operation(repo) != Some(*kind) {
                return Err(GitError::invalid("Estado da operação mudou após a prévia"));
            }
            args(&[
                match kind {
                    OperationKind::Merge => "merge",
                    OperationKind::Rebase => "rebase",
                    OperationKind::CherryPick => "cherry-pick",
                    OperationKind::Revert => "revert",
                },
                if matches!(action, WorkflowAction::Continue(_)) {
                    "--continue"
                } else {
                    "--abort"
                },
            ])
        }
        WorkflowAction::StashCreate {
            message,
            include_untracked,
        } => {
            // Git 2.47's internal clean receives magic pathspecs for -u. The
            // inherited literal flag prevents that cleanup. This command accepts
            // no user pathspecs, so disable it only for the built-in stash flow.
            let mut c = args(&["--no-literal-pathspecs", "stash", "push"]);
            if *include_untracked {
                c.push("--include-untracked".into());
            }
            c.extend(args(&["--message", message, "--"]));
            c
        }
        WorkflowAction::StashCreateOptions {
            message,
            include_untracked,
            keep_index,
        } => {
            let mut a = args(&["--no-literal-pathspecs", "stash", "push"]);
            if *include_untracked {
                a.push("--include-untracked".into());
            }
            if *keep_index {
                a.push("--keep-index".into());
            }
            a.extend(args(&["--message", message, "--"]));
            a
        }
        WorkflowAction::PushTo { .. } | WorkflowAction::PushWithTags { .. } => unreachable!(),
        WorkflowAction::StashDrop { stash } => args(&["stash", "drop", stash]),
        WorkflowAction::StashApply { pop, .. } => {
            let result = repo.run(&args(&["stash", "apply", target]), None, cancel);
            if !matches!(&result,Ok(out) if out.status.success()) || !*pop {
                return Ok(report(repo, result, cancel));
            }
            let expected = plan.stash.as_ref().unwrap();
            if selected_stash(repo, &expected.reference, cancel).is_ok_and(|s| s == *expected) {
                let mut result = report(
                    repo,
                    repo.run(&args(&["stash", "drop", &expected.reference]), None, cancel),
                    cancel,
                );
                if !result.success {
                    result.summary="Stash aplicado aos arquivos, mas não removido da lista; verifique antes de repetir".into();
                }
                return Ok(result);
            }
            return Ok(WorkflowReport {
                success: false,
                summary:
                    "Stash aplicado, mas a lista mudou; o stash não foi removido automaticamente"
                        .into(),
                output: String::new(),
                conflicts: Vec::new(),
                operation: active_operation(repo),
                outcome_unknown: false,
            });
        }
        WorkflowAction::Fetch { remote }
        | WorkflowAction::Pull { remote, .. }
        | WorkflowAction::Push { remote, .. } => {
            let push = matches!(action, WorkflowAction::Push { .. });
            if remote_urls(repo, remote, push, cancel)? != plan.remote_urls {
                return Err(GitError::invalid("Endereço do remoto mudou após a prévia"));
            }
            match action {
                WorkflowAction::Fetch { .. } => args(&["fetch", "--no-recurse-submodules", remote]),
                WorkflowAction::Push { branch, .. } => args(&[
                    "push",
                    "--porcelain",
                    remote,
                    &format!("{target}:refs/heads/{branch}"),
                ]),
                WorkflowAction::Pull { branch, mode, .. } => {
                    let fetched = repo.run(
                        &args(&[
                            "fetch",
                            "--no-recurse-submodules",
                            remote,
                            &format!("refs/heads/{branch}"),
                        ]),
                        None,
                        cancel,
                    );
                    if !matches!(&fetched,Ok(out) if out.status.success()) {
                        return Ok(report(repo, fetched, cancel));
                    }
                    // Fetch legitimately updates remote refs/ahead-behind metadata.
                    // Pull started clean, so any local file change appears in entries.
                    let after = repo.snapshot(cancel)?;
                    if after.head != plan.snapshot.head
                        || after.head_ref != plan.snapshot.head_ref
                        || after.index_hash != plan.snapshot.index_hash
                        || after.entries != plan.snapshot.entries
                    {
                        return Ok(WorkflowReport{success:false,summary:"Fetch concluído; arquivos locais mudaram e a integração não foi iniciada".into(),output:String::new(),conflicts:Vec::new(),operation:active_operation(repo),outcome_unknown:false});
                    }
                    let fetched = oid(repo, "FETCH_HEAD", cancel)?;
                    match mode {
                        PullMode::FastForwardOnly => args(&["merge", "--ff-only", "--", &fetched]),
                        PullMode::Merge => {
                            args(&["merge", "--no-edit", "--no-autostash", "--", &fetched])
                        }
                        PullMode::Rebase => {
                            args(&["rebase", "--no-autostash", "--no-update-refs", &fetched])
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        WorkflowAction::CherryPick { .. } => args(&["cherry-pick", "--no-edit", target]),
        WorkflowAction::Revert { .. } => args(&["revert", "--no-edit", target]),
        WorkflowAction::Reset { mode, .. } => args(&[
            "reset",
            match mode {
                ResetMode::Soft => "--soft",
                ResetMode::Mixed => "--mixed",
                ResetMode::Hard => "--hard",
            },
            target,
            "--",
        ]),
    };
    Ok(report(repo, repo.run(&command, None, cancel), cancel))
}
