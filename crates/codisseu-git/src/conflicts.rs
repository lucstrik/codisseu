//! Three-stage conflicts use immutable index object IDs and an exact repository snapshot.
use crate::workflows::path_from_bytes;
use crate::{GitError, RepoSnapshot, Repository};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

pub const MAX_CONFLICT_TEXT: u64 = 4 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictSide {
    pub oid: String,
    pub mode: String,
    pub bytes: Option<Vec<u8>>,
    pub binary: bool,
    pub truncated: bool,
}
#[derive(Clone, Debug)]
pub struct ConflictFile {
    pub path: PathBuf,
    pub base: Option<ConflictSide>,
    pub ours: Option<ConflictSide>,
    pub theirs: Option<ConflictSide>,
    pub working: Option<Vec<u8>>,
    pub text_merge: Option<std::sync::Arc<crate::text_merge::TextMerge>>,
    pub editable: bool,
    pub warnings: Vec<String>,
    pub unsupported_reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkingState {
    bytes: Option<Vec<u8>>,
    length: u64,
    mode: u32,
    symlink: bool,
    modified: Option<std::time::SystemTime>,
}
#[derive(Clone, Debug)]
pub struct ConflictSnapshot {
    pub file: ConflictFile,
    snapshot: RepoSnapshot,
    working_state: Option<WorkingState>,
    certificate: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionChoice {
    EditedText(String),
    Ours,
    Theirs,
    Delete,
}
#[derive(Clone, Debug)]
pub struct ConflictResolutionPreview {
    pub path: PathBuf,
    pub choice: ResolutionChoice,
    pub stage: bool,
    pub summary: String,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct PreparedResolution {
    pub preview: ConflictResolutionPreview,
    conflict: ConflictSnapshot,
    choice: ResolutionChoice,
    stage: bool,
}
#[derive(Clone, Debug)]
pub struct ResolutionReport {
    pub worktree_written: bool,
    pub staged: bool,
    pub summary: String,
    pub error: Option<String>,
    pub outcome_unknown: bool,
}

fn args(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(OsString::from).collect()
}
fn fingerprint(file: &ConflictFile) -> [u8; 32] {
    let mut hash = Sha256::new();
    let mut field = |bytes: &[u8]| {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    };
    field(file.path.as_os_str().as_encoded_bytes());
    for side in [&file.base, &file.ours, &file.theirs] {
        if let Some(side) = side {
            field(b"present");
            field(side.oid.as_bytes());
            field(side.mode.as_bytes());
            field(&[u8::from(side.binary), u8::from(side.truncated)]);
            if let Some(bytes) = &side.bytes {
                field(b"bytes");
                field(bytes);
            } else {
                field(b"omitted");
            }
        } else {
            field(b"absent");
        }
    }
    if let Some(bytes) = &file.working {
        field(b"working");
        field(bytes);
    } else {
        field(b"missing");
    }
    field(&[u8::from(file.editable)]);
    field(
        file.unsupported_reason
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    for warning in &file.warnings {
        field(warning.as_bytes());
    }
    hash.finalize().into()
}
fn checked_path(repo: &Repository, path: &Path) -> Result<PathBuf, GitError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || path.components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .trim_end_matches(['.', ' '])
                .eq_ignore_ascii_case(".git")
        })
    {
        return Err(GitError::invalid(
            "Caminho de conflito precisa ser relativo ao repositório",
        ));
    }
    let full = repo.root.join(path);
    let mut parent = repo.root.clone();
    for component in path.parent().unwrap_or(Path::new("")).components() {
        parent.push(component.as_os_str());
        let meta = fs::symlink_metadata(&parent)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(GitError::invalid(
                "Pai do conflito é symlink ou não é pasta; escrita recusada",
            ));
        }
    }
    if let Ok(meta) = fs::symlink_metadata(&full)
        && !meta.is_file()
        && !meta.file_type().is_symlink()
    {
        return Err(GitError::unsupported(
            "Conflito não é arquivo regular nem symlink",
        ));
    }
    Ok(full)
}
fn working_state(path: &Path) -> Result<Option<WorkingState>, GitError> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode()
    };
    #[cfg(not(unix))]
    let mode = u32::from(meta.permissions().readonly());
    let symlink = meta.file_type().is_symlink();
    let bytes = if symlink {
        let target = fs::read_link(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Some(target.as_os_str().as_bytes().to_vec())
        }
        #[cfg(not(unix))]
        {
            Some(
                target
                    .to_str()
                    .ok_or_else(|| GitError::unsupported("Symlink sem Unicode"))?
                    .as_bytes()
                    .to_vec(),
            )
        }
    } else if meta.len() <= MAX_CONFLICT_TEXT {
        Some(fs::read(path)?)
    } else {
        None
    };
    Ok(Some(WorkingState {
        bytes,
        length: meta.len(),
        mode,
        symlink,
        modified: meta.modified().ok(),
    }))
}
fn blob(
    repo: &Repository,
    mode: String,
    identity: String,
    cancel: &AtomicBool,
) -> Result<ConflictSide, GitError> {
    if !matches!(mode.as_str(), "100644" | "100755" | "120000" | "160000")
        || !matches!(identity.len(), 40 | 64)
        || !identity.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(GitError::invalid("Estágio de conflito inválido"));
    }
    if mode == "160000" {
        return Ok(ConflictSide {
            oid: identity,
            mode,
            bytes: None,
            binary: true,
            truncated: false,
        });
    }
    let size = String::from_utf8(
        repo.run_checked(&args(&["cat-file", "-s", &identity]), None, cancel)?
            .stdout,
    )
    .map_err(|_| GitError::invalid("Tamanho de blob inválido"))?
    .trim()
    .parse::<u64>()
    .map_err(|_| GitError::invalid("Tamanho de blob inválido"))?;
    let bytes = if size <= MAX_CONFLICT_TEXT {
        Some(
            repo.run_checked(&args(&["cat-file", "blob", &identity]), None, cancel)?
                .stdout,
        )
    } else {
        None
    };
    let binary = bytes
        .as_ref()
        .is_none_or(|b| b.contains(&0) || std::str::from_utf8(b).is_err());
    Ok(ConflictSide {
        oid: identity,
        mode,
        bytes,
        binary,
        truncated: size > MAX_CONFLICT_TEXT,
    })
}
fn index_stages(
    repo: &Repository,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<[Option<ConflictSide>; 3], GitError> {
    let mut command = args(&[
        "--literal-pathspecs",
        "ls-files",
        "--unmerged",
        "--stage",
        "-z",
        "--",
    ]);
    command.push(path.as_os_str().into());
    let out = repo.run_checked(&command, None, cancel)?;
    let mut sides: [Option<ConflictSide>; 3] = [None, None, None];
    for row in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let at = row
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| GitError::invalid("Estágio sem caminho"))?;
        if path_from_bytes(&row[at + 1..])? != path {
            return Err(GitError::invalid("Git retornou outro caminho de conflito"));
        }
        let header = std::str::from_utf8(&row[..at])
            .map_err(|_| GitError::invalid("Cabeçalho de conflito inválido"))?;
        let fields: Vec<_> = header.split(' ').collect();
        if fields.len() != 3 {
            return Err(GitError::invalid("Cabeçalho de estágio inválido"));
        }
        let stage = fields[2]
            .parse::<usize>()
            .map_err(|_| GitError::invalid("Número de estágio inválido"))?;
        if !(1..=3).contains(&stage) || sides[stage - 1].is_some() {
            return Err(GitError::invalid("Estágio de conflito duplicado"));
        }
        sides[stage - 1] = Some(blob(repo, fields[0].into(), fields[1].into(), cancel)?);
    }
    if sides.iter().all(Option::is_none) {
        return Err(GitError::invalid("Este caminho não tem conflito no índice"));
    }
    Ok(sides)
}
fn separated_rename_group(repo: &Repository, cancel: &AtomicBool) -> Result<bool, GitError> {
    let out = repo.run_checked(
        &args(&["ls-files", "--unmerged", "--stage", "-z"]),
        None,
        cancel,
    )?;
    let mut stages = std::collections::BTreeMap::<Vec<u8>, u8>::new();
    for row in out.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Some(at) = row.iter().position(|b| *b == b'\t') else {
            return Err(GitError::invalid("Índice de conflitos inválido"));
        };
        let stage = row[..at].last().copied().unwrap_or_default();
        if !(b'1'..=b'3').contains(&stage) {
            return Err(GitError::invalid("Estágio de conflito inválido"));
        }
        *stages.entry(row[at + 1..].to_vec()).or_default() |= 1 << (stage - b'1');
    }
    // A base-only path with side-only paths is the unresolved split rename group,
    // not an ordinary modify/delete or add/add conflict.
    Ok(stages.len() > 1 && stages.values().any(|v| *v == 1))
}
pub fn load_conflict(
    repo: &Repository,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<ConflictSnapshot, GitError> {
    let full = checked_path(repo, path)?;
    let snapshot = repo.snapshot(cancel)?;
    let [base, ours, theirs] = index_stages(repo, path, cancel)?;
    let working_state = working_state(&full)?;
    let mut warnings = Vec::new();
    if base.is_none() {
        warnings.push("Conflito Add/Add: não existe versão base para este caminho.".into());
    }
    if ours.is_none() || theirs.is_none() {
        warnings.push(
            "Um lado removeu o arquivo. Para aceitar a remoção escolha Delete explicitamente."
                .into(),
        );
    }
    if crate::workflows::active_operation(repo) == Some(crate::workflows::OperationKind::Rebase) {
        warnings.push("Durante rebase, Ours é a base de destino já reaplicada; Theirs é o commit sendo reaplicado.".into());
    }
    #[cfg(unix)]
    if let Ok(meta) = fs::symlink_metadata(&full) {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            warnings.push("A gravação atômica substitui esta entrada; outros hardlinks conservam o conteúdo anterior.".into());
        }
    }
    let unsupported_reason = if [&base, &ours, &theirs]
        .into_iter()
        .flatten()
        .any(|s| s.mode == "160000")
    {
        Some("Conflito de submódulo requer resolução explícita no submódulo; não será alterado automaticamente.".into())
    } else if separated_rename_group(repo, cancel)? {
        Some("Há renomes divididos entre caminhos conflitantes. A resolução agrupada de renomes ainda não está disponível; resolva externamente e atualize o status.".into())
    } else {
        None
    };
    let editable = unsupported_reason.is_none()
        && [&base, &ours, &theirs]
            .into_iter()
            .flatten()
            .all(|s| matches!(s.mode.as_str(), "100644" | "100755") && !s.binary)
        && working_state
            .as_ref()
            .is_none_or(|w| !w.symlink && w.length <= MAX_CONFLICT_TEXT);
    if !editable && unsupported_reason.is_none() {
        warnings.push("Conteúdo binário, symlink ou maior que 4 MiB: editor de texto indisponível; escolha um lado existente ou Delete explicitamente.".into());
    }
    let text_merge = if editable {
        fn text(side: Option<&ConflictSide>) -> &str {
            side.and_then(|s| s.bytes.as_deref())
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("")
        }
        match crate::text_merge::TextMerge::build(
            text(base.as_ref()),
            text(ours.as_ref()),
            text(theirs.as_ref()),
            cancel,
        ) {
            Ok(model) => Some(std::sync::Arc::new(model)),
            Err(error) if cancel.load(Ordering::Relaxed) => return Err(error),
            Err(error) => {
                warnings.push(format!("Comparação por trechos indisponível: {error}. Use o editor de resultado ou escolha um lado completo."));
                None
            }
        }
    } else {
        None
    };
    let file = ConflictFile {
        path: path.into(),
        base,
        ours,
        theirs,
        working: working_state.as_ref().and_then(|w| w.bytes.clone()),
        text_merge,
        editable,
        warnings,
        unsupported_reason,
    };
    if repo.snapshot(cancel)? != snapshot {
        return Err(GitError::invalid(
            "Conflito mudou durante leitura; abra novamente",
        ));
    }
    let certificate = fingerprint(&file);
    Ok(ConflictSnapshot {
        file,
        snapshot,
        working_state,
        certificate,
    })
}
pub fn prepare_resolution(
    repo: &Repository,
    conflict: &ConflictSnapshot,
    choice: ResolutionChoice,
    stage: bool,
    cancel: &AtomicBool,
) -> Result<PreparedResolution, GitError> {
    if fingerprint(&conflict.file) != conflict.certificate {
        return Err(GitError::invalid(
            "Metadados do conflito alterados; abra novamente",
        ));
    }
    if let Some(reason) = &conflict.file.unsupported_reason {
        return Err(GitError::unsupported(reason));
    }
    if repo.snapshot(cancel)? != conflict.snapshot {
        return Err(GitError::invalid(
            "Conflito obsoleto; abra novamente antes de aplicar",
        ));
    }
    match &choice {
        ResolutionChoice::EditedText(text) => {
            if !conflict.file.editable {
                return Err(GitError::unsupported(
                    "Este conflito não aceita editor de texto",
                ));
            }
            if text.len() as u64 > MAX_CONFLICT_TEXT || text.contains('\0') {
                return Err(GitError::invalid(
                    "Resultado precisa ser texto UTF-8 sem NUL até 4 MiB",
                ));
            }
        }
        ResolutionChoice::Ours if conflict.file.ours.is_none() => {
            return Err(GitError::invalid(
                "Ours removeu este caminho; escolha Delete explicitamente",
            ));
        }
        ResolutionChoice::Theirs if conflict.file.theirs.is_none() => {
            return Err(GitError::invalid(
                "Theirs removeu este caminho; escolha Delete explicitamente",
            ));
        }
        _ => {}
    }
    let mut warnings = conflict.file.warnings.clone();
    if matches!(choice, ResolutionChoice::Delete) {
        warnings.push("Esta ação remove o arquivo do diretório de trabalho.".into());
    }
    let preview = ConflictResolutionPreview {
        path: conflict.file.path.clone(),
        summary: format!(
            "Aplicar {}{}",
            match choice {
                ResolutionChoice::EditedText(_) => "resultado editado",
                ResolutionChoice::Ours => "Ours",
                ResolutionChoice::Theirs => "Theirs",
                ResolutionChoice::Delete => "remoção explícita",
            },
            if stage {
                " e marcar o caminho como resolvido no índice"
            } else {
                " somente no arquivo; conflito continua no índice"
            }
        ),
        choice,
        stage,
        warnings,
    };
    Ok(PreparedResolution {
        choice: preview.choice.clone(),
        stage,
        preview,
        conflict: conflict.clone(),
    })
}
fn atomic_text(path: &Path, text: &str, conflict: &ConflictSnapshot) -> Result<(), GitError> {
    let parent = path
        .parent()
        .ok_or_else(|| GitError::invalid("Arquivo sem pasta"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(text.as_bytes())?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(GitError::invalid("Tipo do arquivo mudou antes da gravação"));
        }
        temporary.as_file().set_permissions(meta.permissions())?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = conflict
                .file
                .ours
                .as_ref()
                .or(conflict.file.theirs.as_ref())
                .map_or("100644", |s| s.mode.as_str());
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(if mode == "100755" {
                    0o755
                } else {
                    0o644
                }))?;
        }
    }
    temporary.as_file().sync_all()?;
    if working_state(path)? != conflict.working_state {
        return Err(GitError::invalid(
            "Arquivo mudou durante a gravação; original preservado",
        ));
    }
    temporary
        .persist(path)
        .map_err(|e| GitError::from(e.error))?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn apply_resolution(
    repo: &Repository,
    plan: &PreparedResolution,
    cancel: &AtomicBool,
) -> Result<ResolutionReport, GitError> {
    if plan.preview.choice != plan.choice
        || plan.preview.stage != plan.stage
        || plan.preview.path != plan.conflict.file.path
    {
        return Err(GitError::invalid(
            "Prévia de resolução alterada; prepare novamente",
        ));
    }
    let path = &plan.conflict.file.path;
    let full = checked_path(repo, path)?;
    if repo.snapshot(cancel)? != plan.conflict.snapshot {
        return Err(GitError::invalid(
            "HEAD, índice ou arquivos mudaram após a prévia de resolução",
        ));
    }
    if working_state(&full)? != plan.conflict.working_state {
        return Err(GitError::invalid(
            "Conteúdo/tipo/permissões do conflito mudaram; prepare novamente",
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(GitError::invalid("Resolução cancelada antes da escrita"));
    }
    let result = match &plan.preview.choice {
        ResolutionChoice::EditedText(text) => atomic_text(&full, text, &plan.conflict),
        ResolutionChoice::Delete => match fs::remove_file(&full) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        },
        ResolutionChoice::Ours | ResolutionChoice::Theirs => {
            let mut command = args(&[
                "--literal-pathspecs",
                "checkout",
                if plan.preview.choice == ResolutionChoice::Ours {
                    "--ours"
                } else {
                    "--theirs"
                },
                "--",
            ]);
            command.push(path.as_os_str().into());
            repo.run_checked(&command, None, cancel).map(|_| ())
        }
    };
    if let Err(error) = result {
        return Ok(ResolutionReport {
            worktree_written: working_state(&full).ok()
                != Some(plan.conflict.working_state.clone()),
            staged: false,
            summary: "A resolução não foi concluída; confira o arquivo e o índice".into(),
            error: Some(error.to_string()),
            outcome_unknown: true,
        });
    }
    if !plan.preview.stage {
        return Ok(ResolutionReport {
            worktree_written: true,
            staged: false,
            summary: "Resultado gravado; conflito permanece no índice até marcar como resolvido"
                .into(),
            error: None,
            outcome_unknown: false,
        });
    }
    let mut command = if matches!(plan.preview.choice, ResolutionChoice::Delete) {
        args(&["--literal-pathspecs", "rm", "--force", "--"])
    } else {
        args(&["--literal-pathspecs", "add", "--"])
    };
    command.push(path.as_os_str().into());
    match repo.run_checked(&command,None,cancel){
        Ok(_)=>Ok(ResolutionReport{worktree_written:true,staged:true,summary:"Resultado gravado e caminho marcado como resolvido; Continue ainda é uma ação separada".into(),error:None,outcome_unknown:false}),
        Err(error)=>Ok(ResolutionReport{worktree_written:true,staged:false,summary:"Arquivo alterado, mas o índice não foi confirmado; confira o status antes de continuar".into(),error:Some(error.to_string()),outcome_unknown:true})
    }
}
