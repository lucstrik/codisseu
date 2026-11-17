use crate::{
    DiffHunk, DiffSide, GitDiff, GitError, GitErrorKind, GitOutput, RepoSnapshot, RepositoryInfo,
    StatusEntry, process,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Repository {
    pub(crate) root: PathBuf,
    pub(crate) git_dir: PathBuf,
    pub(crate) common_dir: PathBuf,
    pub(crate) bare: bool,
}
pub(crate) fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
pub(crate) fn trim_line(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}
#[cfg(unix)]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, GitError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(PathBuf::from(OsStr::from_bytes(bytes)))
}
#[cfg(not(unix))]
pub(crate) fn path_from_bytes(bytes: &[u8]) -> Result<PathBuf, GitError> {
    Ok(PathBuf::from(std::str::from_utf8(bytes).map_err(|_| {
        GitError::unsupported("Caminho Git não Unicode nesta plataforma")
    })?))
}
#[cfg(unix)]
pub(crate) fn os_bytes(value: &OsStr) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    value.as_bytes()
}
#[cfg(not(unix))]
pub(crate) fn os_bytes(value: &OsStr) -> &[u8] {
    value.to_str().unwrap_or("").as_bytes()
}
pub(crate) fn cancelled(cancel: &AtomicBool) -> Result<(), GitError> {
    if cancel.load(Ordering::SeqCst) {
        Err(GitError::new(
            GitErrorKind::Cancelled,
            "Operação Git cancelada",
        ))
    } else {
        Ok(())
    }
}
pub(crate) fn checked(output: GitOutput) -> Result<GitOutput, GitError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(GitError::new(
            GitErrorKind::Git,
            if output.stderr.trim().is_empty() {
                format!("Git encerrou com {}", output.status)
            } else {
                crate::redact(output.stderr.trim())
            },
        ))
    }
}
impl Repository {
    pub fn discover(path: &Path, cancel: &AtomicBool) -> Result<Self, GitError> {
        let canonical = fs::canonicalize(path)?;
        let cwd = if canonical.is_file() {
            canonical
                .parent()
                .ok_or_else(|| GitError::invalid("Arquivo sem diretório"))?
                .to_owned()
        } else {
            canonical
        };
        let run = |a: &[&str]| {
            checked(process::run(
                &cwd,
                &args(a),
                None,
                cancel,
                Duration::from_secs(15),
                &[],
            )?)
        };
        let bare = trim_line(&run(&["rev-parse", "--is-bare-repository"])?.stdout) == b"true";
        let root = if bare {
            cwd.clone()
        } else {
            fs::canonicalize(path_from_bytes(trim_line(
                &run(&["rev-parse", "--path-format=absolute", "--show-toplevel"])?.stdout,
            ))?)?
        };
        let git_dir = fs::canonicalize(path_from_bytes(trim_line(
            &run(&["rev-parse", "--absolute-git-dir"])?.stdout,
        ))?)?;
        let common_dir = fs::canonicalize(path_from_bytes(trim_line(
            &run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?.stdout,
        ))?)?;
        Ok(Self {
            root,
            git_dir,
            common_dir,
            bare,
        })
    }
    pub fn info(&self) -> RepositoryInfo {
        RepositoryInfo {
            root: self.root.clone(),
            git_dir: self.git_dir.clone(),
            common_dir: self.common_dir.clone(),
            bare: self.bare,
            linked_worktree: self.git_dir != self.common_dir,
        }
    }
    pub fn run(
        &self,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
        cancel: &AtomicBool,
    ) -> Result<GitOutput, GitError> {
        self.run_timeout(arguments, stdin, cancel, Duration::from_secs(20))
    }
    pub fn run_timeout(
        &self,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<GitOutput, GitError> {
        process::run(&self.root, arguments, stdin, cancel, timeout, &[])
    }
    pub fn run_checked(
        &self,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
        cancel: &AtomicBool,
    ) -> Result<GitOutput, GitError> {
        checked(self.run(arguments, stdin, cancel)?)
    }
    pub(crate) fn run_env(
        &self,
        arguments: &[OsString],
        stdin: Option<&[u8]>,
        cancel: &AtomicBool,
        environment: &[(OsString, OsString)],
    ) -> Result<GitOutput, GitError> {
        checked(process::run(
            &self.root,
            arguments,
            stdin,
            cancel,
            Duration::from_secs(120),
            environment,
        )?)
    }
    pub fn resolve(&self, revision: &str, cancel: &AtomicBool) -> Result<String, GitError> {
        if revision.is_empty()
            || revision.len() > 1024
            || revision.starts_with('-')
            || revision.contains('\0')
        {
            return Err(GitError::invalid("Revisão Git inválida"));
        }
        let output = self.run_checked(
            &args(&[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{commit}}"),
            ]),
            None,
            cancel,
        )?;
        let oid = std::str::from_utf8(trim_line(&output.stdout))
            .map_err(|_| GitError::invalid("OID Git inválido"))?;
        if !valid_oid(oid) {
            return Err(GitError::invalid("OID Git inválido"));
        }
        Ok(oid.into())
    }
    pub fn relative_path(&self, path: &Path) -> Result<PathBuf, GitError> {
        let path = if path.is_absolute() {
            path.strip_prefix(&self.root)
                .map_err(|_| GitError::invalid("Caminho fora do repositório"))?
        } else {
            path
        };
        let mut relative = PathBuf::new();
        for part in path.components() {
            match part {
                Component::Normal(value) => relative.push(value),
                Component::CurDir => {}
                _ => {
                    return Err(GitError::invalid(
                        "Caminho Git não pode escapar do repositório",
                    ));
                }
            }
        }
        if relative.as_os_str().is_empty() || os_bytes(relative.as_os_str()).contains(&0) {
            return Err(GitError::invalid("Caminho Git vazio/inválido"));
        }
        Ok(relative)
    }
    // Ignore submodule working trees entirely; recover only staged gitlink changes
    // from the superproject index without invoking any submodule status/helpers.
    pub(crate) fn snapshot_status(&self, cancel: &AtomicBool) -> Result<Vec<u8>, GitError> {
        let mut raw = self
            .run_checked(
                &args(&[
                    "status",
                    "--porcelain=v2",
                    "-z",
                    "--branch",
                    "--untracked-files=all",
                    "--ignore-submodules=all",
                ]),
                None,
                cancel,
            )?
            .stdout;
        let present = parse_status_limited(&raw, 10000)?.0.0;
        let cached = self
            .run_checked(
                &args(&[
                    "diff",
                    "--cached",
                    "--raw",
                    "-z",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--ignore-submodules=none",
                ]),
                None,
                cancel,
            )?
            .stdout;
        let mut fields = cached.split(|b| *b == 0);
        while let Some(header) = fields.next() {
            if header.is_empty() {
                continue;
            }
            let path = fields
                .next()
                .ok_or_else(|| GitError::invalid("Gitlink staged sem caminho"))?;
            let parts = header.split(|b| *b == b' ').collect::<Vec<_>>();
            if parts.len() != 5 || !parts[0].starts_with(b":") || parts[4].len() != 1 {
                return Err(GitError::invalid("Raw diff do índice inválido"));
            }
            let old = &parts[0][1..];
            let new = parts[1];
            if old != b"160000" && new != b"160000" {
                continue;
            }
            let path_buf = path_from_bytes(path)?;
            if present.iter().any(|e| e.path == path_buf) {
                continue;
            }
            raw.extend_from_slice(b"1 ");
            raw.push(parts[4][0]);
            raw.extend_from_slice(b". S... ");
            for part in [old, new, new, parts[2], parts[3]] {
                raw.extend_from_slice(part);
                raw.push(b' ');
            }
            raw.extend_from_slice(path);
            raw.push(0);
        }
        Ok(raw)
    }
    pub fn snapshot(&self, cancel: &AtomicBool) -> Result<RepoSnapshot, GitError> {
        let raw = if self.bare {
            Vec::new()
        } else {
            self.snapshot_status(cancel)?
        };
        let (entries, mut head, mut head_ref, upstream, ahead, behind) = parse_status(&raw)?;
        if self.bare {
            head = self.resolve("HEAD", cancel).ok();
            let symbolic = self.run(&args(&["symbolic-ref", "-q", "HEAD"]), None, cancel)?;
            if symbolic.status.success() {
                head_ref = Some(String::from_utf8_lossy(trim_line(&symbolic.stdout)).into_owned());
            }
        }
        let index_hash = file_hash(&self.git_dir.join("index"), cancel, 256 * 1024 * 1024)?;
        let mut hash = Sha256::new();
        hash.update(&raw);
        let refs = self.run_checked(
            &args(&[
                "for-each-ref",
                "--format=%(refname)%00%(objectname)%00",
                "refs/heads",
                "refs/remotes",
                "refs/tags",
            ]),
            None,
            cancel,
        )?;
        hash.update(&refs.stdout);
        for path in [
            self.common_dir.join("config"),
            self.git_dir.join("config.worktree"),
        ] {
            hash.update(file_hash(&path, cancel, 4 * 1024 * 1024)?.as_bytes());
        }
        let mut budget = 64 * 1024 * 1024usize;
        for entry in &entries {
            cancelled(cancel)?;
            hash.update(os_bytes(entry.path.as_os_str()));
            let path = self.root.join(&entry.path);
            safe_parents(&self.root, &entry.path)?;
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    hash.update(b"symlink");
                    hash.update(os_bytes(fs::read_link(&path)?.as_os_str()));
                }
                Ok(metadata) if metadata.is_file() => {
                    if metadata.len() > budget as u64 {
                        return Err(GitError::new(
                            GitErrorKind::OutputLimit,
                            "Arquivos alterados excedem64 MiB de verificação; reduza a seleção/alterações antes de mutar pela IDE",
                        ));
                    }
                    budget -= metadata.len() as usize;
                    hash.update(file_hash(&path, cancel, 32 * 1024 * 1024)?.as_bytes());
                }
                Ok(metadata) if metadata.is_dir() => {
                    hash.update(b"directory/submodule");
                }
                Ok(_) => {
                    return Err(GitError::unsupported(
                        "Arquivo especial não pode entrar no snapshot Git",
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hash.update(b"deleted")
                }
                Err(error) => return Err(error.into()),
            }
        }
        if !self.bare {
            let current = self.snapshot_status(cancel)?;
            if current != raw
                || file_hash(&self.git_dir.join("index"), cancel, 256 * 1024 * 1024)? != index_hash
                || self
                    .run_checked(
                        &args(&[
                            "for-each-ref",
                            "--format=%(refname)%00%(objectname)%00",
                            "refs/heads",
                            "refs/remotes",
                            "refs/tags",
                        ]),
                        None,
                        cancel,
                    )?
                    .stdout
                    != refs.stdout
            {
                return Err(GitError::new(
                    GitErrorKind::Stale,
                    "O repositório mudou durante a leitura. Atualize novamente.",
                ));
            }
        }
        let operation = [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry-pick"),
            ("REVERT_HEAD", "revert"),
        ]
        .into_iter()
        .find_map(|(file, operation)| {
            self.git_dir
                .join(file)
                .exists()
                .then(|| operation.to_owned())
        });
        Ok(RepoSnapshot {
            root: self.root.clone(),
            head,
            head_ref,
            index_hash,
            worktree_hash: format!("{:x}", hash.finalize()),
            entries,
            upstream,
            ahead,
            behind,
            upstream_counts_known: raw
                .split(|byte| *byte == 0)
                .any(|line| line.starts_with(b"# branch.ab ")),
            operation,
        })
    }
    pub fn diff(
        &self,
        path: &Path,
        side: DiffSide,
        cancel: &AtomicBool,
    ) -> Result<GitDiff, GitError> {
        let path = self.relative_path(path)?;
        let mut arguments = args(&[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--binary",
            "--full-index",
            "--unified=3",
        ]);
        if side == DiffSide::Index {
            arguments.push("--cached".into());
        }
        arguments.push("--".into());
        arguments.push(path.as_os_str().into());
        let mut bytes = self.run_checked(&arguments, None, cancel)?.stdout;
        if bytes.is_empty() && side == DiffSide::Working {
            let tracked = self.run(
                &[
                    "ls-files".into(),
                    "--error-unmatch".into(),
                    "--".into(),
                    path.as_os_str().into(),
                ],
                None,
                cancel,
            )?;
            if !tracked.status.success() {
                safe_parents(&self.root, &path)?;
                let null = if cfg!(unix) { "/dev/null" } else { "NUL" };
                let a = vec![
                    "diff".into(),
                    "--no-index".into(),
                    "--no-ext-diff".into(),
                    "--no-textconv".into(),
                    "--no-color".into(),
                    "--binary".into(),
                    "--full-index".into(),
                    "--".into(),
                    null.into(),
                    path.as_os_str().into(),
                ];
                let result = self.run(&a, None, cancel)?;
                if !matches!(result.status.code(), Some(0 | 1)) {
                    return Err(GitError::new(
                        GitErrorKind::Git,
                        crate::redact(&result.stderr),
                    ));
                }
                bytes = result.stdout;
            }
        }
        let binary = bytes.windows(16).any(|w| w == b"GIT binary patch")
            || bytes.windows(12).any(|w| w == b"Binary files");
        let hunks = if binary { vec![] } else { parse_hunks(&bytes) };
        Ok(GitDiff {
            path,
            side,
            bytes,
            binary,
            hunks,
        })
    }
}
pub(crate) fn valid_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}
pub(crate) fn file_hash(
    path: &Path,
    cancel: &AtomicBool,
    limit: usize,
) -> Result<String, GitError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok("missing".into()),
        Err(e) => return Err(e.into()),
    };
    if !metadata.is_file() {
        return Err(GitError::unsupported(
            "Estado Git usa arquivo especial/symlink; não abrir implicitamente",
        ));
    }
    if metadata.len() > limit as u64 {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Arquivo excede orçamento de verificação Git",
        ));
    }
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    let mut total = 0usize;
    let started = Instant::now();
    loop {
        cancelled(cancel)?;
        if started.elapsed() > Duration::from_secs(15) {
            return Err(GitError::new(
                GitErrorKind::Timeout,
                "Verificação de arquivo Git excedeu prazo",
            ));
        }
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        total += count;
        if total > limit {
            return Err(GitError::new(
                GitErrorKind::OutputLimit,
                "Arquivo cresceu além do orçamento Git",
            ));
        }
        hash.update(&bytes[..count]);
    }
    let after = fs::metadata(path)?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Arquivo mudou durante a verificação Git; atualize",
        ));
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn parse_hunks(bytes: &[u8]) -> Vec<DiffHunk> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return vec![];
    };
    let lines = text.split_inclusive('\n').collect::<Vec<_>>();
    let Some(first) = lines.iter().position(|line| line.starts_with("@@ ")) else {
        return vec![];
    };
    let header = lines[..first].concat();
    let starts = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| line.starts_with("@@ ").then_some(i))
        .collect::<Vec<_>>();
    starts
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let end = starts.get(i + 1).copied().unwrap_or(lines.len());
            let body = lines[*start..end].concat();
            DiffHunk {
                header: lines[*start].trim_end().into(),
                lines: lines[*start + 1..end]
                    .iter()
                    .map(|line| (*line).to_owned())
                    .collect(),
                patch: format!("{header}{body}").into_bytes(),
            }
        })
        .collect()
}
pub(crate) type ParsedStatus = (
    Vec<StatusEntry>,
    Option<String>,
    Option<String>,
    Option<String>,
    u64,
    u64,
);
fn parse_status(bytes: &[u8]) -> Result<ParsedStatus, GitError> {
    let (parsed, truncated) = parse_status_limited(bytes, 10000)?;
    if truncated {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Status excede10000 arquivos alterados",
        ));
    }
    Ok(parsed)
}
pub(crate) fn parse_status_limited(
    bytes: &[u8],
    limit: usize,
) -> Result<(ParsedStatus, bool), GitError> {
    let mut records = bytes.split(|b| *b == 0);
    let mut entries = Vec::new();
    let (mut head, mut branch, mut upstream) = (None, None, None);
    let (mut ahead, mut behind) = (0, 0);
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if record.starts_with(b"# ") {
            let header = String::from_utf8_lossy(record);
            if let Some(value) = header.strip_prefix("# branch.oid ") {
                if value != "(initial)" {
                    if !valid_oid(value) {
                        return Err(GitError::invalid("Status Git com OID inválido"));
                    }
                    head = Some(value.into());
                }
            } else if let Some(value) = header.strip_prefix("# branch.head ") {
                if value != "(detached)" {
                    branch = Some(format!("refs/heads/{value}"));
                }
            } else if let Some(value) = header.strip_prefix("# branch.upstream ") {
                upstream = Some(value.into());
            } else if let Some(value) = header.strip_prefix("# branch.ab ") {
                let fields = value.split_whitespace().collect::<Vec<_>>();
                if fields.len() == 2 {
                    ahead = fields[0]
                        .trim_start_matches('+')
                        .parse()
                        .map_err(|_| GitError::invalid("Ahead Git inválido"))?;
                    behind = fields[1]
                        .trim_start_matches('-')
                        .parse()
                        .map_err(|_| GitError::invalid("Behind Git inválido"))?;
                }
            }
            continue;
        }
        if entries.len() >= limit {
            return Ok(((entries, head, branch, upstream, ahead, behind), true));
        }
        let (path, original, index, worktree, untracked, conflict, submodule) = match record[0] {
            b'?' => {
                if !record.starts_with(b"? ") {
                    return Err(GitError::invalid("Status untracked inválido"));
                }
                (&record[2..], None, '?', '?', true, false, "N...".into())
            }
            b'1' | b'2' | b'u' => {
                let fields = if record[0] == b'1' {
                    record.splitn(9, |b| *b == b' ').collect::<Vec<_>>()
                } else if record[0] == b'2' {
                    record.splitn(10, |b| *b == b' ').collect::<Vec<_>>()
                } else {
                    record.splitn(11, |b| *b == b' ').collect::<Vec<_>>()
                };
                let expected = if record[0] == b'1' {
                    9
                } else if record[0] == b'2' {
                    10
                } else {
                    11
                };
                if fields.len() != expected || fields[1].len() != 2 {
                    return Err(GitError::invalid("Status porcelain v2 inválido"));
                }
                let original = if record[0] == b'2' {
                    Some(path_from_bytes(records.next().ok_or_else(|| {
                        GitError::invalid("Rename Git sem origem")
                    })?)?)
                } else {
                    None
                };
                (
                    *fields.last().unwrap(),
                    original,
                    char::from(fields[1][0]),
                    char::from(fields[1][1]),
                    false,
                    record[0] == b'u',
                    String::from_utf8_lossy(fields[2]).into_owned(),
                )
            }
            b'!' => continue,
            _ => return Err(GitError::invalid("Registro porcelain v2 não reconhecido")),
        };
        entries.push(StatusEntry {
            path: path_from_bytes(path)?,
            original_path: original,
            index,
            worktree,
            untracked,
            conflict,
            submodule,
        });
    }
    Ok(((entries, head, branch, upstream, ahead, behind), false))
}

pub(crate) fn safe_parents(root: &Path, path: &Path) -> Result<(), GitError> {
    let mut current = root.to_path_buf();
    let components = path.components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(GitError::unsupported(
                    "Diretório ancestral é symlink; leitura fora da árvore recusada",
                ));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(GitError::unsupported(
                    "Diretório ancestral não é diretório regular",
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
