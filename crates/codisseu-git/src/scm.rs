//! Source Control actions composed over the existing snapshot-bound Git worker.
use crate::{
    CommitSelection, GitError, GitErrorKind, GitReport, LocalAction, RepoSnapshot, Repository,
    local, repository::args,
};
use std::{ffi::OsString, path::PathBuf, sync::atomic::AtomicBool};
#[derive(Clone, Debug)]
pub struct CommitIdentity {
    pub name: String,
    pub email: String,
}
#[derive(Clone, Debug)]
pub enum ScmAction {
    DeleteUntracked {
        paths: Vec<PathBuf>,
    },
    CommitOptions {
        selection: CommitSelection,
        message: String,
        amend: bool,
        signoff: bool,
        coauthors: Vec<CommitIdentity>,
    },
    RemoteAdd {
        name: String,
        url: String,
    },
    RemoteRename {
        name: String,
        new_name: String,
    },
    RemoteSetUrl {
        name: String,
        url: String,
        push: bool,
    },
    RemoteRemove {
        name: String,
    },
    SetUpstream {
        branch: String,
        upstream: Option<String>,
    },
}
#[derive(Clone, Debug)]
pub struct ScmPreview {
    pub summary: String,
    pub warnings: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub destructive: bool,
}
pub struct PreparedScm {
    pub preview: ScmPreview,
    snapshot: RepoSnapshot,
    arguments: Vec<OsString>,
    local: Option<local::PreparedLocal>,
    delete: Option<Vec<DeleteEntry>>,
    preview_hash: String,
}
fn identity(identity: &CommitIdentity) -> Result<String, GitError> {
    if identity.name.trim().is_empty()
        || identity.name.len() > 1024
        || identity.email.is_empty()
        || identity.email.len() > 1024
        || !identity.email.contains('@')
        || identity
            .name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>'))
        || identity
            .email
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>'))
    {
        return Err(GitError::invalid(
            "Autor exige nome e email válidos, sem caracteres de controle",
        ));
    }
    Ok(format!("{} <{}>", identity.name.trim(), identity.email))
}
fn remote_name(repo: &Repository, name: &str, cancel: &AtomicBool) -> Result<(), GitError> {
    if name.is_empty()
        || name.len() > 1024
        || name.starts_with('-')
        || name.chars().any(char::is_control)
    {
        return Err(GitError::invalid("Nome de remoto inválido"));
    }
    repo.run_checked(
        &args(&[
            "check-ref-format",
            &format!("refs/remotes/{name}/codisseu-check"),
        ]),
        None,
        cancel,
    )?;
    Ok(())
}
fn url(value: &str) -> Result<(), GitError> {
    if value.is_empty()
        || value.len() > 4096
        || value.starts_with('-')
        || value.starts_with("ext::")
        || value.chars().any(char::is_control)
        || value
            .split("://")
            .nth(1)
            .is_some_and(|tail| tail.split('/').next().unwrap_or("").contains('@'))
        || value.contains('?')
    {
        return Err(GitError::invalid(
            "URL de remoto inválida ou contém credenciais; use o gerenciador seguro de credenciais Git",
        ));
    }
    Ok(())
}
fn remote_exists(repo: &Repository, name: &str, cancel: &AtomicBool) -> Result<bool, GitError> {
    Ok(repo
        .run_checked(&args(&["remote"]), None, cancel)?
        .stdout
        .split(|b| *b == b'\n')
        .any(|line| line == name.as_bytes()))
}
fn verify_ref(repo: &Repository, reference: &str, cancel: &AtomicBool) -> Result<(), GitError> {
    repo.run_checked(&args(&["check-ref-format", reference]), None, cancel)?;
    if !repo
        .run(
            &args(&["show-ref", "--verify", "--quiet", "--", reference]),
            None,
            cancel,
        )?
        .status
        .success()
    {
        return Err(GitError::invalid("Referência selecionada não existe"));
    }
    Ok(())
}
pub fn prepare_scm(
    repo: &Repository,
    action: ScmAction,
    cancel: &AtomicBool,
) -> Result<PreparedScm, GitError> {
    let snapshot = repo.snapshot(cancel)?;
    let mut preview = ScmPreview {
        summary: String::new(),
        warnings: vec![],
        paths: vec![],
        destructive: false,
    };
    let mut local = None;
    let mut delete = None;
    let arguments = match action {
        ScmAction::DeleteUntracked { paths } => {
            if paths.is_empty() || paths.len() > 512 {
                return Err(GitError::invalid("Selecione de1a512 arquivos novos"));
            }
            let mut entries = Vec::new();
            let mut seen = std::collections::BTreeSet::new();
            let mut bytes = 0;
            for path in paths {
                let path = repo.relative_path(&path)?;
                if !seen.insert(path.clone()) {
                    return Err(GitError::invalid("Arquivos novos duplicados"));
                }
                if !snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.path == path && entry.untracked)
                {
                    return Err(GitError::invalid(
                        "Excluir exige arquivos novos individuais, não ignorados e não rastreados",
                    ));
                }
                let entry = read_delete_entry(repo, &path)?;
                bytes += entry.bytes.len();
                if bytes > 32 * 1024 * 1024 {
                    return Err(GitError::invalid("Backup de exclusão excede32MiB"));
                }
                entries.push(entry);
            }
            preview.paths = entries.iter().map(|entry| entry.path.clone()).collect();
            preview.destructive = true;
            preview.summary = format!("Excluir {} arquivos novos selecionados", entries.len());
            preview.warnings.push("Somente arquivos regulares novos selecionados serão excluídos após backup durável em .git/codisseu/untracked-recovery. Pastas, links, arquivos rastreados e caminhos não selecionados não serão apagados. O caminho do backup aparecerá no resultado.".into());
            delete = Some(entries);
            vec![]
        }

        ScmAction::CommitOptions {
            selection,
            mut message,
            amend,
            signoff,
            coauthors,
        } => {
            if coauthors.len() > 32 {
                return Err(GitError::invalid("Commit aceita no máximo32 coautores"));
            }
            let mut trailers = coauthors
                .iter()
                .map(identity)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .map(|author| format!("Co-authored-by: {author}"))
                .collect::<Vec<_>>();
            if signoff {
                let raw = repo
                    .run_checked(&args(&["var", "GIT_COMMITTER_IDENT"]), None, cancel)?
                    .stdout;
                let raw = String::from_utf8(raw)
                    .map_err(|_| GitError::invalid("Identidade Git não UTF-8"))?;
                let end = raw
                    .rfind('>')
                    .ok_or_else(|| GitError::invalid("Identidade Git inválida"))?;
                let author = &raw[..=end];
                let (name, email) = author
                    .rsplit_once(" <")
                    .ok_or_else(|| GitError::invalid("Identidade Git inválida"))?;
                let author = identity(&CommitIdentity {
                    name: name.into(),
                    email: email.trim_end_matches('>').into(),
                })?;
                trailers.push(format!("Signed-off-by: {author}"));
            }
            trailers.dedup();
            let missing = trailers
                .into_iter()
                .filter(|trailer| {
                    !message
                        .rsplit("\n\n")
                        .next()
                        .unwrap_or("")
                        .lines()
                        .any(|line| line == trailer)
                })
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                message = message.trim_end_matches(['\r', '\n']).to_owned();
                let final_block = message.rsplit("\n\n").next().unwrap_or("");
                let has_trailers = !final_block.is_empty()
                    && final_block.lines().all(|line| {
                        line.split_once(": ").is_some_and(|(key, _)| {
                            !key.is_empty()
                                && key.chars().all(|character| {
                                    character.is_ascii_alphanumeric() || character == '-'
                                })
                        })
                    });
                if !message.is_empty() {
                    message.push_str(if has_trailers { "\n" } else { "\n\n" });
                }
                message.push_str(&missing.join("\n"));
                preview.warnings.extend(missing);
            }
            let plan = local::prepare(
                repo,
                LocalAction::Commit {
                    selection,
                    message,
                    amend,
                },
                cancel,
            )?;
            preview.paths = plan.paths.clone();
            preview.summary = format!(
                "{} · {} arquivos{}{}",
                if amend { "Alterar commit" } else { "Commit" },
                preview.paths.len(),
                if signoff { " · Signed-off-by" } else { "" },
                if coauthors.is_empty() {
                    String::new()
                } else {
                    format!(" · {} coautores", coauthors.len())
                }
            );
            preview.warnings.push("O Git usará os hooks e a assinatura configurados. Sign-off é um trailer de autoria e não substitui assinatura criptográfica.".into());
            local = Some(plan);
            vec![]
        }
        ScmAction::RemoteAdd { name, url: target } => {
            remote_name(repo, &name, cancel)?;
            url(&target)?;
            if remote_exists(repo, &name, cancel)? {
                return Err(GitError::invalid("Remoto já existe"));
            }
            preview.summary = format!("Adicionar remoto {name} · {}", crate::redact(&target));
            args(&["remote", "add", "--", &name, &target])
        }
        ScmAction::RemoteRename { name, new_name } => {
            remote_name(repo, &name, cancel)?;
            remote_name(repo, &new_name, cancel)?;
            if !remote_exists(repo, &name, cancel)? || remote_exists(repo, &new_name, cancel)? {
                return Err(GitError::invalid(
                    "Remoto original ausente ou destino já existe",
                ));
            }
            preview.summary = format!("Renomear remoto {name} para {new_name}");
            args(&["remote", "rename", "--", &name, &new_name])
        }
        ScmAction::RemoteSetUrl {
            name,
            url: target,
            push,
        } => {
            remote_name(repo, &name, cancel)?;
            url(&target)?;
            if !remote_exists(repo, &name, cancel)? {
                return Err(GitError::invalid("Remoto não existe"));
            }
            preview.summary = format!(
                "Alterar URL {} de {name} · {}",
                if push { "de envio" } else { "de leitura" },
                crate::redact(&target)
            );
            let mut command = args(&["remote", "set-url"]);
            if push {
                command.push("--push".into());
            }
            command.extend(args(&["--", &name, &target]));
            command
        }
        ScmAction::RemoteRemove { name } => {
            remote_name(repo, &name, cancel)?;
            if !remote_exists(repo, &name, cancel)? {
                return Err(GitError::invalid("Remoto não existe"));
            }
            preview.summary = format!("Remover remoto {name} desta cópia local");
            preview.destructive = true;
            preview.warnings.push("Remove a configuração local e referências de acompanhamento deste remoto. O servidor remoto não será alterado.".into());
            args(&["remote", "remove", "--", &name])
        }
        ScmAction::SetUpstream { branch, upstream } => {
            let branch = branch.strip_prefix("refs/heads/").unwrap_or(&branch);
            if branch.starts_with('-') || branch.is_empty() || branch.starts_with("refs/") {
                return Err(GitError::invalid("Selecione uma branch local"));
            }
            verify_ref(repo, &format!("refs/heads/{branch}"), cancel)?;
            if let Some(upstream) = upstream {
                if !upstream.starts_with("refs/heads/") && !upstream.starts_with("refs/remotes/") {
                    return Err(GitError::invalid(
                        "Upstream exige referência completa local/remota",
                    ));
                }
                verify_ref(repo, &upstream, cancel)?;
                preview.summary = format!("Acompanhar {upstream} em {branch}");
                args(&[
                    "branch",
                    &format!("--set-upstream-to={upstream}"),
                    "--",
                    branch,
                ])
            } else {
                preview.summary = format!("Remover acompanhamento de {branch}");
                args(&["branch", "--unset-upstream", "--", branch])
            }
        }
    };
    if repo.snapshot(cancel)? != snapshot {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Estado Git mudou durante a preparação",
        ));
    }
    let preview_hash = preview_digest(&preview);
    Ok(PreparedScm {
        delete,
        preview_hash,
        preview,
        snapshot,
        arguments,
        local,
    })
}
pub fn apply_scm(
    repo: &Repository,
    prepared: &PreparedScm,
    cancel: &AtomicBool,
) -> Result<GitReport, GitError> {
    if preview_digest(&prepared.preview) != prepared.preview_hash {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Prévia SCM foi alterada",
        ));
    }
    if repo.snapshot(cancel)? != prepared.snapshot {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Estado Git mudou; prepare a ação novamente",
        ));
    }
    if let Some(entries) = &prepared.delete {
        return apply_delete(repo, entries, &prepared.snapshot, cancel);
    }
    if let Some(local) = &prepared.local {
        return local::apply(repo, local, cancel);
    }
    let output = repo.run(&prepared.arguments, None, cancel)?;
    Ok(GitReport {
        success: output.status.success(),
        summary: if output.status.success() {
            prepared.preview.summary.clone()
        } else {
            format!("Git recusou a operação: {}", crate::redact(&output.stderr))
        },
        output: crate::redact(&format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            output.stderr
        )),
        outcome_unknown: false,
    })
}

fn preview_digest(preview: &ScmPreview) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(format!("{preview:?}").as_bytes()))
}
struct DeleteEntry {
    path: PathBuf,
    bytes: Vec<u8>,
    permissions: u32,
}
fn read_delete_entry(repo: &Repository, path: &std::path::Path) -> Result<DeleteEntry, GitError> {
    use std::io::Read;
    crate::repository::safe_parents(&repo.root, path)?;
    if path
        .components()
        .any(|part| crate::repository::os_bytes(part.as_os_str()).eq_ignore_ascii_case(b".git"))
    {
        return Err(GitError::invalid("Metadados Git não podem ser excluídos"));
    }
    let full = repo.root.join(path);
    let metadata = std::fs::symlink_metadata(&full)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 4 * 1024 * 1024
    {
        return Err(GitError::invalid(
            "Excluir aceita somente arquivos regulares até4MiB, sem links/pastas",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let mut bytes = Vec::new();
    options
        .open(full)?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(GitError::invalid("Arquivo novo excede4MiB"));
    }
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let permissions = 0;
    Ok(DeleteEntry {
        path: path.into(),
        bytes,
        permissions,
    })
}
fn ensure_recovery_directory(path: &std::path::Path) -> Result<(), GitError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(GitError::invalid(
            "Pasta de recuperação não pode ser arquivo ou symlink",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}
fn apply_delete(
    repo: &Repository,
    entries: &[DeleteEntry],
    expected: &RepoSnapshot,
    cancel: &AtomicBool,
) -> Result<GitReport, GitError> {
    use crate::repository::{cancelled, os_bytes};
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let _index_lock = DeletionIndexLock::acquire(repo)?;
    let checked = repo.snapshot(cancel)?;
    if checked != *expected {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Estado Git mudou antes do bloqueio de exclusão",
        ));
    }
    if entries.iter().any(|entry| {
        !checked
            .entries
            .iter()
            .any(|current| current.path == entry.path && current.untracked)
    }) {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Arquivo deixou de ser novo; atualize a prévia",
        ));
    }
    let root = repo.git_dir.join("codisseu");
    ensure_recovery_directory(&root)?;
    let recovery = root.join("untracked-recovery");
    ensure_recovery_directory(&recovery)?;
    if crate::shelf::storage_size(&root)?
        .saturating_add(
            entries
                .iter()
                .map(|entry| entry.bytes.len() as u64)
                .sum::<u64>()
                .saturating_mul(2),
        )
        .saturating_add(1024 * 1024)
        > 256 * 1024 * 1024
    {
        return Err(GitError::invalid(
            "Backup excederia256MiB; preserve/libere recuperações existentes explicitamente",
        ));
    }
    for entry in entries {
        cancelled(cancel)?;
        let current = read_delete_entry(repo, &entry.path)?;
        if current.bytes != entry.bytes || current.permissions != entry.permissions {
            return Err(GitError::new(
                GitErrorKind::Stale,
                "Arquivo novo mudou; revise a exclusão novamente",
            ));
        }
    }
    let temporary = tempfile::Builder::new()
        .prefix("batch-")
        .tempdir_in(&recovery)?;
    let mut records = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        cancelled(cancel)?;
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(temporary.path().join(format!("{index}.bin")))?;
        file.write_all(&entry.bytes)?;
        file.sync_all()?;
        records.push(serde_json::json!({"path":os_bytes(entry.path.as_os_str()),"file":format!("{index}.bin"),"actual_file":format!("{index}.removed"),"permissions":entry.permissions,"sha256":format!("{:x}",Sha256::digest(&entry.bytes))}));
    }
    let metadata = serde_json::to_vec_pretty(
        &serde_json::json!({"version":1,"root":os_bytes(repo.root.as_os_str()),"entries":records}),
    )
    .map_err(|_| GitError::invalid("Backup inválido"))?;
    if metadata.len() > 1024 * 1024 {
        return Err(GitError::invalid("Metadados de backup excedem1MiB"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temporary.path().join("metadata.json"))?;
    file.write_all(&metadata)?;
    file.sync_all()?;
    std::fs::File::open(temporary.path())?.sync_all()?;
    let backup = temporary.keep();
    std::fs::File::open(&recovery)?.sync_all()?;
    let mut removed: Vec<(&DeleteEntry, PathBuf)> = Vec::new();
    let result = (|| -> Result<(), GitError> {
        for (index, entry) in entries.iter().enumerate() {
            cancelled(cancel)?;
            let current = read_delete_entry(repo, &entry.path)?;
            if current.bytes != entry.bytes || current.permissions != entry.permissions {
                return Err(GitError::new(
                    GitErrorKind::Stale,
                    "Arquivo novo mudou durante a exclusão",
                ));
            }
            // Atomic relocation retains the exact inode even if an external
            // writer changes it between validation and removal. No unlink or
            // recursive delete is used for the selected worktree path.
            let moved = backup.join(format!("{index}.removed"));
            std::fs::rename(repo.root.join(&entry.path), &moved)?;
            removed.push((entry, moved.clone()));
            let backup_repo = Repository {
                root: backup.clone(),
                ..repo.clone()
            };
            let actual = read_delete_entry(
                &backup_repo,
                PathBuf::from(format!("{index}.removed")).as_path(),
            )?;
            if actual.bytes != entry.bytes || actual.permissions != entry.permissions {
                return Err(GitError::new(
                    GitErrorKind::Stale,
                    "Arquivo mudou no instante da exclusão; a versão movida será restaurada",
                ));
            }
            std::fs::File::open(&moved)?.sync_all()?;
            std::fs::File::open(&backup)?.sync_all()?;
            std::fs::File::open(repo.root.join(&entry.path).parent().unwrap())?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut incomplete = false;
        for (entry, moved) in removed {
            let full = repo.root.join(&entry.path);
            if crate::repository::safe_parents(&repo.root, &entry.path).is_err() {
                incomplete = true;
                continue;
            }
            // hard_link creates the destination only when it does not exist;
            // an externally recreated worktree path is never overwritten.
            match std::fs::hard_link(&moved, &full) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&moved);
                    if let Some(parent) = full.parent() {
                        let _ = std::fs::File::open(parent).and_then(|file| file.sync_all());
                    }
                }
                Err(_) => {
                    incomplete = true;
                }
            }
        }
        return Ok(GitReport {
            success: false,
            summary: if incomplete {
                "Exclusão interrompida; alguns arquivos exigem recuperação"
            } else {
                "Exclusão interrompida; arquivos removidos foram restaurados"
            }
            .into(),
            output: format!("{error}\nBackup durável: {}", backup.display()),
            outcome_unknown: incomplete,
        });
    }
    Ok(GitReport {
        success: true,
        summary: format!("{} arquivos novos excluídos", entries.len()),
        output: format!(
            "Backup durável: {}\nÍndice e arquivos não selecionados preservados.",
            backup.display()
        ),
        outcome_unknown: false,
    })
}

struct DeletionIndexLock {
    path: PathBuf,
}
impl DeletionIndexLock {
    fn acquire(repo: &Repository) -> Result<Self, GitError> {
        let path = repo.git_dir.join("index.lock");
        std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .map_err(|error| {
                GitError::new(
                    GitErrorKind::Io,
                    format!("Índice ocupado; exclusão não iniciada: {error}"),
                )
            })?;
        Ok(Self { path })
    }
}
impl Drop for DeletionIndexLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
