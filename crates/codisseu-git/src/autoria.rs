//! Independent, bounded authorship reads for an editor's current UTF-8 buffer.
//!
//! No editor content is saved to disk. Git receives the buffer on stdin, and
//! responses retain the document version and request identity supplied by the
//! caller. A lane has one active job, one replaceable pending job and one reply.
use crate::{
    CommitEntry, GitError, GitErrorKind, Repository, RepositoryInfo,
    repository::{args, cancelled, os_bytes, path_from_bytes, safe_parents, trim_line, valid_oid},
};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub const MAX_CONTENTS: usize = 4 * 1024 * 1024;
pub const MAX_LINES: usize = 500;
const MAX_DOCUMENT_LINES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Tracked,
    Added,
    Untracked,
    Unborn,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub workspace: PathBuf,
    pub path: PathBuf,
    pub document_version: u64,
    pub contents: Arc<str>,
    /// Git line numbers are one based, including in every response.
    pub start: usize,
    pub lines: usize,
}
#[derive(Clone, Debug)]
pub struct Line {
    pub line: usize,
    pub original_line: usize,
    pub oid: String,
    pub author: String,
    pub email: String,
    pub timestamp: i64,
    pub timezone: String,
    pub summary: String,
    /// Historical path, relative to the repository returned in the snapshot.
    pub path: PathBuf,
    pub previous_oid: Option<String>,
    pub previous_path: Option<PathBuf>,
    pub uncommitted: bool,
    pub text: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub repository: RepositoryInfo,
    pub head: Option<String>,
    pub state: FileState,
    pub lines: Vec<Line>,
}
#[derive(Debug)]
pub struct Reply {
    pub id: u64,
    pub document_version: u64,
    pub path: PathBuf,
    pub result: Result<Snapshot, GitError>,
}

struct Mailbox<Q, A> {
    pending: Option<(u64, Q)>,
    active: Option<(u64, Arc<AtomicBool>)>,
    reply: Option<A>,
    latest: u64,
    closing: bool,
}
pub(crate) struct Lane<Q, A> {
    shared: Arc<(Mutex<Mailbox<Q, A>>, Condvar)>,
}
impl<Q: Send + 'static, A: Send + 'static> Lane<Q, A> {
    pub(crate) fn new<F>(name: &str, mut run: F) -> Self
    where
        F: FnMut(u64, Q, &AtomicBool) -> A + Send + 'static,
    {
        let shared = Arc::new((
            Mutex::new(Mailbox {
                pending: None,
                active: None,
                reply: None,
                latest: 0,
                closing: false,
            }),
            Condvar::new(),
        ));
        let background = shared.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                loop {
                    let (lock, wake) = &*background;
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    while state.pending.is_none() && !state.closing {
                        state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    if state.closing {
                        break;
                    }
                    let (id, job) = state.pending.take().expect("pending job checked");
                    let cancel = Arc::new(AtomicBool::new(false));
                    state.active = Some((id, cancel.clone()));
                    drop(state);
                    let reply = run(id, job, &cancel);
                    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    state.active = None;
                    if state.latest == id && !state.closing && !cancel.load(Ordering::SeqCst) {
                        state.reply = Some(reply);
                    }
                }
            })
            .expect("cannot start Git authorship worker");
        Self { shared }
    }
    pub(crate) fn request(&self, job: Q) -> Result<u64, GitError> {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        let id = state
            .latest
            .checked_add(1)
            .ok_or_else(|| GitError::invalid("Identificador de autoria esgotado"))?;
        state.latest = id;
        if let Some((_, cancel)) = &state.active {
            cancel.store(true, Ordering::SeqCst);
        }
        state.pending = Some((id, job));
        state.reply = None;
        wake.notify_one();
        Ok(id)
    }
    pub(crate) fn cancel(&self) {
        let (lock, _) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.latest = state.latest.saturating_add(1);
        state.pending = None;
        state.reply = None;
        if let Some((_, cancel)) = &state.active {
            cancel.store(true, Ordering::SeqCst);
        }
    }
    pub(crate) fn poll(&self) -> Option<A> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reply
            .take()
    }
    pub(crate) fn is_busy(&self) -> bool {
        let state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        state.pending.is_some() || state.active.is_some()
    }
}
impl<Q, A> Drop for Lane<Q, A> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.closing = true;
        state.pending = None;
        state.reply = None;
        if let Some((_, cancel)) = &state.active {
            cancel.store(true, Ordering::SeqCst);
        }
        wake.notify_one();
        // The worker owns its process until the existing runner kills/reaps it.
        // Dropping this GUI-facing handle never waits for external Git.
    }
}

pub struct Worker {
    lane: Lane<Request, Reply>,
}
impl Default for Worker {
    fn default() -> Self {
        Self::new()
    }
}
impl Worker {
    pub fn new() -> Self {
        // The one-entry cache belongs exclusively to this worker thread.
        let mut cache = None;
        Self {
            lane: Lane::new("git-authorship", move |id, request: Request, cancel| {
                Reply {
                    id,
                    document_version: request.document_version,
                    path: request.path.clone(),
                    result: cached_read(&mut cache, &request, cancel, Instant::now(), &mut inspect),
                }
            }),
        }
    }
    pub fn request(&self, request: Request) -> Result<u64, GitError> {
        validate_request(&request.contents, request.start, request.lines)?;
        self.lane.request(request)
    }
    pub fn cancel(&self) {
        self.lane.cancel();
    }
    pub fn poll(&self) -> Option<Reply> {
        self.lane.poll()
    }
    pub fn is_busy(&self) -> bool {
        self.lane.is_busy()
    }
}

// Revalidate on every read; only the expensive blame traversal is cached. Git
// configuration, attributes and mailmap changes without a HEAD/index change are
// observed after at most30s. No errors or failed/cancelled validations are cached.
const CACHE_TTL: Duration = Duration::from_secs(30);
#[derive(Clone, Debug, PartialEq, Eq)]
struct FolderIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(unix))]
    created: Option<std::time::SystemTime>,
}
fn folder_identity(path: &Path) -> Result<FolderIdentity, GitError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir() {
        return Err(GitError::invalid("Identidade Git não é diretório"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(FolderIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(FolderIdentity {
            created: metadata.created().ok(),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct CacheIdentity {
    repository: RepositoryInfo,
    folders: [FolderIdentity; 3],
    head: Option<String>,
    index: Vec<u8>,
}
struct CacheEntry {
    request: Request,
    identity: CacheIdentity,
    snapshot: Snapshot,
    created: Instant,
}
fn index_entry(repo: &Repository, path: &Path, cancel: &AtomicBool) -> Result<Vec<u8>, GitError> {
    Ok(repo
        .run_checked(
            &[
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                repo.relative_path(path)?.into_os_string(),
            ],
            None,
            cancel,
        )?
        .stdout)
}
fn folder_identities(repo: &Repository) -> Result<[FolderIdentity; 3], GitError> {
    Ok([
        folder_identity(&repo.root)?,
        folder_identity(&repo.git_dir)?,
        folder_identity(&repo.common_dir)?,
    ])
}
fn cache_identity(
    request: &Request,
    cancel: &AtomicBool,
) -> Result<(Repository, CacheIdentity), GitError> {
    cancelled(cancel)?;
    validate_contents(&request.contents, request.start, request.lines)?;
    let repo = repository_for(&request.workspace, &request.path, cancel)?;
    let identity = CacheIdentity {
        repository: repo.info(),
        folders: folder_identities(&repo)?,
        head: head(&repo, cancel)?,
        index: index_entry(&repo, &request.path, cancel)?,
    };
    same_head(&repo, &identity.head, cancel)?;
    Ok((repo, identity))
}
fn verify_cache_identity(
    repo: &Repository,
    request: &Request,
    identity: &CacheIdentity,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    // Recheck paths as well as Git metadata. A previously valid entry must not
    // bypass a newly introduced file/directory symlink or replaced repository.
    validate_path(&request.workspace, &request.path)?;
    same_head(repo, &identity.head, cancel)?;
    if index_entry(repo, &request.path, cancel)? != identity.index
        || folder_identities(repo)? != identity.folders
    {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "Repositório ou índice mudou durante a leitura de autoria",
        ));
    }
    cancelled(cancel)
}
fn cached_read<F>(
    cache: &mut Option<CacheEntry>,
    request: &Request,
    cancel: &AtomicBool,
    now: Instant,
    read: &mut F,
) -> Result<Snapshot, GitError>
where
    F: FnMut(&Request, &AtomicBool) -> Result<Snapshot, GitError>,
{
    let result = (|| {
        let (repo, identity) = cache_identity(request, cancel)?;
        if let Some(entry) = cache.as_ref()
            && now.saturating_duration_since(entry.created) < CACHE_TTL
            && entry.request == *request
            && entry.identity == identity
        {
            verify_cache_identity(&repo, request, &identity, cancel)?;
            // This clone runs on the worker, never in a GUI frame.
            let snapshot = entry.snapshot.clone();
            cancelled(cancel)?;
            return Ok(snapshot);
        }
        // Release an expired or nonmatching entry before allocating a new result.
        *cache = None;
        let snapshot = read(request, cancel)?;
        verify_cache_identity(&repo, request, &identity, cancel)?;
        if snapshot.repository != identity.repository || snapshot.head != identity.head {
            return Err(GitError::new(
                GitErrorKind::Stale,
                "Identidade Git mudou durante a leitura de autoria",
            ));
        }
        *cache = Some(CacheEntry {
            request: request.clone(),
            identity,
            snapshot: snapshot.clone(),
            created: now,
        });
        Ok(snapshot)
    })();
    if result.is_err() {
        *cache = None;
    }
    result
}

fn validate_request(contents: &str, start: usize, lines: usize) -> Result<(), GitError> {
    if contents.len() > MAX_CONTENTS {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Autoria do editor limitada a4MiB por documento",
        ));
    }
    if start == 0 || !(1..=MAX_LINES).contains(&lines) || start.checked_add(lines).is_none() {
        return Err(GitError::invalid(
            "Autoria requer linha inicial≥1 e1–500 linhas",
        ));
    }
    Ok(())
}
fn validate_contents(contents: &str, start: usize, lines: usize) -> Result<(), GitError> {
    validate_request(contents, start, lines)?;
    if contents.contains('\0') {
        return Err(GitError::unsupported(
            "Autoria indisponível para conteúdo binário",
        ));
    }
    let total = contents.bytes().filter(|b| *b == b'\n').count() + 1;
    if total > MAX_DOCUMENT_LINES {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Autoria do editor limitada a100000 linhas por documento",
        ));
    }
    if start > total {
        return Err(GitError::invalid("Linha de autoria fora do buffer atual"));
    }
    Ok(())
}

/// Validate paths without following symlinks, including an already-open file
/// that has since been removed from disk. Ancestors must remain real folders.
fn validate_path(workspace: &Path, path: &Path) -> Result<(PathBuf, PathBuf), GitError> {
    if !workspace.is_absolute() || !path.is_absolute() {
        return Err(GitError::invalid(
            "Workspace e arquivo devem ter caminhos absolutos",
        ));
    }
    let workspace = fs::canonicalize(workspace)?;
    if !workspace.is_dir() {
        return Err(GitError::invalid("Workspace não é diretório"));
    }
    let relative = path
        .strip_prefix(&workspace)
        .map_err(|_| GitError::invalid("Autoria fora do workspace recusada"))?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| match component {
            Component::Normal(value) => value == ".git" || os_bytes(value).contains(&0),
            _ => true,
        })
    {
        return Err(GitError::invalid("Caminho de autoria inválido"));
    }
    safe_parents(&workspace, relative)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
            return Err(GitError::unsupported(
                "Autoria requer arquivo regular, sem symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((workspace, path.to_path_buf()))
}
fn repository_for(
    workspace: &Path,
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Repository, GitError> {
    let (workspace, path) = validate_path(workspace, path)?;
    let parent = path
        .parent()
        .ok_or_else(|| GitError::invalid("Arquivo sem diretório"))?;
    let repo = Repository::discover(parent, cancel)?;
    if repo.bare || !(repo.root.starts_with(&workspace) || workspace.starts_with(&repo.root)) {
        return Err(GitError::invalid(
            "Repositório de autoria fora do workspace",
        ));
    }
    repo.relative_path(&path)?;
    Ok(repo)
}
fn head(repo: &Repository, cancel: &AtomicBool) -> Result<Option<String>, GitError> {
    let output = repo.run(
        &args(&["rev-parse", "--verify", "--quiet", "HEAD"]),
        None,
        cancel,
    )?;
    if output.status.success() {
        let oid = std::str::from_utf8(trim_line(&output.stdout))
            .map_err(|_| GitError::invalid("HEAD não Unicode"))?;
        if !valid_oid(oid) {
            return Err(GitError::invalid("HEAD inválido"));
        }
        Ok(Some(oid.into()))
    } else if output.status.code() == Some(1) {
        Ok(None)
    } else {
        Err(GitError::new(
            GitErrorKind::Git,
            crate::redact(&output.stderr),
        ))
    }
}
fn same_head(
    repo: &Repository,
    expected: &Option<String>,
    cancel: &AtomicBool,
) -> Result<(), GitError> {
    if &head(repo, cancel)? != expected {
        return Err(GitError::new(
            GitErrorKind::Stale,
            "HEAD mudou durante a leitura de autoria",
        ));
    }
    Ok(())
}

pub fn inspect(request: &Request, cancel: &AtomicBool) -> Result<Snapshot, GitError> {
    cancelled(cancel)?;
    validate_contents(&request.contents, request.start, request.lines)?;
    let repo = repository_for(&request.workspace, &request.path, cancel)?;
    repo.blame_contents(
        &request.path,
        &request.contents,
        request.start,
        request.lines,
        cancel,
    )
}

impl Repository {
    pub fn blame_contents(
        &self,
        path: &Path,
        contents: &str,
        start: usize,
        lines: usize,
        cancel: &AtomicBool,
    ) -> Result<Snapshot, GitError> {
        validate_contents(contents, start, lines)?;
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        validate_path(&self.root, &absolute)?;
        let path = self.relative_path(&absolute)?;
        // Avoid attributing a nested repository's file to the outer repository.
        let actual = Repository::discover(absolute.parent().unwrap_or(&self.root), cancel)?;
        if actual.root != self.root || actual.git_dir != self.git_dir {
            return Err(GitError::invalid(
                "Arquivo pertence a outro repositório Git",
            ));
        }
        let before = head(self, cancel)?;
        let index = self
            .run_checked(
                &[
                    "ls-files".into(),
                    "--stage".into(),
                    "-z".into(),
                    "--".into(),
                    path.clone().into_os_string(),
                ],
                None,
                cancel,
            )?
            .stdout;
        let mut in_index = false;
        for record in index.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let (header, _) = split_once(record, b'\t')
                .ok_or_else(|| GitError::invalid("Entrada de índice sem caminho"))?;
            let fields = header.split(|b| *b == b' ').collect::<Vec<_>>();
            if fields.len() != 3 || !matches!(fields[0], b"100644" | b"100755") {
                return Err(GitError::unsupported(
                    "Autoria indisponível para symlinks ou submódulos",
                ));
            }
            if fields[2] != b"0" {
                return Err(GitError::unsupported(
                    "Resolva os conflitos antes de consultar autoria",
                ));
            }
            in_index = true;
        }
        let in_head = if let Some(oid) = &before {
            !self
                .run_checked(
                    &[
                        "ls-tree".into(),
                        "-z".into(),
                        oid.into(),
                        "--".into(),
                        path.clone().into_os_string(),
                    ],
                    None,
                    cancel,
                )?
                .stdout
                .is_empty()
        } else {
            false
        };
        let mut state = if before.is_none() {
            FileState::Unborn
        } else if !in_index && !in_head {
            FileState::Untracked
        } else if in_index && !in_head {
            FileState::Added
        } else {
            FileState::Tracked
        };
        let all = contents.split('\n').collect::<Vec<_>>();
        let end = start.saturating_add(lines).saturating_sub(1).min(all.len());
        let physical = if contents.is_empty() {
            0
        } else {
            all.len() - usize::from(contents.ends_with('\n'))
        };
        let mut result = Vec::new();
        if matches!(state, FileState::Tracked | FileState::Added) && start <= physical {
            let count = end.min(physical) - start + 1;
            let output = self.run(
                &[
                    "blame".into(),
                    "--line-porcelain".into(),
                    "--root".into(),
                    "--encoding=UTF-8".into(),
                    "--contents".into(),
                    "-".into(),
                    "-L".into(),
                    format!("{start},+{count}").into(),
                    "--".into(),
                    path.clone().into_os_string(),
                ],
                Some(contents.as_bytes()),
                cancel,
            )?;
            if output.status.success() {
                result = parse_porcelain(self, &output.stdout, start, count)?;
                if result.iter().any(|line| !line.uncommitted) {
                    state = FileState::Tracked;
                }
            } else if in_index && !in_head && output.stderr.contains("no such path") {
                state = FileState::Added;
            } else {
                return Err(GitError::new(
                    GitErrorKind::Git,
                    crate::redact(output.stderr.trim()),
                ));
            }
        }
        if state != FileState::Tracked {
            result = (start..=end)
                .map(|number| uncommitted(number, &path, all[number - 1].as_bytes()))
                .collect();
        } else if end > physical {
            result.push(uncommitted(end, &path, b""));
        }
        same_head(self, &before, cancel)?;
        let after_index = self
            .run_checked(
                &[
                    "ls-files".into(),
                    "--stage".into(),
                    "-z".into(),
                    "--".into(),
                    path.clone().into_os_string(),
                ],
                None,
                cancel,
            )?
            .stdout;
        if after_index != index {
            return Err(GitError::new(
                GitErrorKind::Stale,
                "Índice mudou durante a leitura de autoria",
            ));
        }
        cancelled(cancel)?;
        Ok(Snapshot {
            repository: self.info(),
            head: before,
            state,
            lines: result,
        })
    }
}
fn uncommitted(line: usize, path: &Path, text: &[u8]) -> Line {
    Line {
        line,
        original_line: line,
        oid: String::new(),
        author: String::new(),
        email: String::new(),
        timestamp: 0,
        timezone: String::new(),
        summary: "Alterações não confirmadas".into(),
        path: path.into(),
        previous_oid: None,
        previous_path: None,
        uncommitted: true,
        text: text.to_vec(),
    }
}
fn bounded(value: &[u8], limit: usize) -> Result<String, GitError> {
    if value.len() > limit {
        return Err(GitError::new(
            GitErrorKind::OutputLimit,
            "Metadado de autoria excede orçamento",
        ));
    }
    Ok(String::from_utf8_lossy(value).into_owned())
}
fn parse_porcelain(
    repo: &Repository,
    bytes: &[u8],
    start: usize,
    count: usize,
) -> Result<Vec<Line>, GitError> {
    let mut output = Vec::with_capacity(count);
    let mut current = None;
    for raw in bytes.split(|b| *b == b'\n') {
        if let Some(text) = raw.strip_prefix(b"\t") {
            let mut line: Line = current
                .take()
                .ok_or_else(|| GitError::invalid("Blame sem cabeçalho"))?;
            if line.line != start + output.len() || output.len() >= count {
                return Err(GitError::invalid("Blame devolveu linhas fora do intervalo"));
            }
            line.text = text.to_vec();
            if line.uncommitted {
                line.author.clear();
                line.email.clear();
                line.timestamp = 0;
                line.timezone.clear();
                line.summary = "Alterações não confirmadas".into();
            }
            output.push(line);
        } else if raw
            .split(|b| *b == b' ')
            .next()
            .is_some_and(|oid| std::str::from_utf8(oid).is_ok_and(valid_oid))
        {
            if current.is_some() {
                return Err(GitError::invalid("Blame sem conteúdo da linha anterior"));
            }
            let text = std::str::from_utf8(raw)
                .map_err(|_| GitError::invalid("Cabeçalho blame inválido"))?;
            let fields = text.split_ascii_whitespace().collect::<Vec<_>>();
            if !(3..=4).contains(&fields.len()) {
                return Err(GitError::invalid("Cabeçalho blame incompleto"));
            }
            let number = fields[2]
                .parse()
                .map_err(|_| GitError::invalid("Linha blame inválida"))?;
            let mut line = uncommitted(number, Path::new(""), b"");
            line.original_line = fields[1]
                .parse()
                .map_err(|_| GitError::invalid("Linha original blame inválida"))?;
            line.oid = fields[0].into();
            line.uncommitted = fields[0].bytes().all(|b| b == b'0');
            current = Some(line);
        } else if let Some(line) = &mut current {
            if let Some(value) = raw.strip_prefix(b"author ") {
                line.author = bounded(value, 4096)?;
            } else if let Some(value) = raw.strip_prefix(b"author-mail ") {
                line.email = bounded(value, 4096)?.trim_matches(['<', '>']).into();
            } else if let Some(value) = raw.strip_prefix(b"author-time ") {
                line.timestamp = std::str::from_utf8(value)
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| GitError::invalid("Data blame inválida"))?;
            } else if let Some(value) = raw.strip_prefix(b"author-tz ") {
                line.timezone = bounded(value, 16)?;
            } else if let Some(value) = raw.strip_prefix(b"summary ") {
                line.summary = bounded(value, 8192)?;
            } else if let Some(value) = raw.strip_prefix(b"filename ") {
                line.path = repo.relative_path(&path_from_bytes(&unquote(value)?)?)?;
            } else if let Some(value) = raw.strip_prefix(b"previous ") {
                let (oid, path) = split_once(value, b' ')
                    .ok_or_else(|| GitError::invalid("Previous blame inválido"))?;
                let oid = std::str::from_utf8(oid)
                    .map_err(|_| GitError::invalid("Previous OID inválido"))?;
                if !valid_oid(oid) {
                    return Err(GitError::invalid("Previous OID inválido"));
                }
                line.previous_oid = Some(oid.into());
                line.previous_path = Some(repo.relative_path(&path_from_bytes(&unquote(path)?)?)?);
            }
        }
    }
    if current.is_some()
        || output.len() != count
        || output.iter().any(|l| l.path.as_os_str().is_empty())
    {
        return Err(GitError::invalid("Resposta blame incompleta"));
    }
    Ok(output)
}
fn split_once(bytes: &[u8], separator: u8) -> Option<(&[u8], &[u8])> {
    let at = bytes.iter().position(|b| *b == separator)?;
    Some((&bytes[..at], &bytes[at + 1..]))
}
fn unquote(raw: &[u8]) -> Result<Vec<u8>, GitError> {
    if !raw.starts_with(b"\"") {
        return Ok(raw.to_vec());
    }
    if raw.len() < 2 || !raw.ends_with(b"\"") {
        return Err(GitError::invalid("Caminho Git quoted inválido"));
    }
    let mut result = Vec::new();
    let mut i = 1;
    while i < raw.len() - 1 {
        let byte = raw[i];
        i += 1;
        if byte != b'\\' {
            result.push(byte);
            continue;
        }
        let next = *raw
            .get(i)
            .filter(|_| i < raw.len() - 1)
            .ok_or_else(|| GitError::invalid("Escape Git incompleto"))?;
        i += 1;
        result.push(match next {
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'b' => 8,
            b'f' => 12,
            b'v' => 11,
            b'"' => b'"',
            b'\\' => b'\\',
            b'0'..=b'7' => {
                if i + 1 >= raw.len() - 1 || !raw[i..i + 2].iter().all(|b| matches!(b, b'0'..=b'7'))
                {
                    return Err(GitError::invalid("Escape octal Git inválido"));
                }
                let value = u16::from(next - b'0') * 64
                    + u16::from(raw[i] - b'0') * 8
                    + u16::from(raw[i + 1] - b'0');
                i += 2;
                u8::try_from(value).map_err(|_| GitError::invalid("Escape Git fora da faixa"))?
            }
            _ => return Err(GitError::invalid("Escape Git desconhecido")),
        });
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct HistoryRequest {
    pub workspace: PathBuf,
    pub path: PathBuf,
    pub document_version: u64,
    pub contents: Arc<str>,
    pub start: usize,
    pub end: usize,
    pub limit: usize,
}
#[derive(Clone, Debug)]
pub struct LineHistory {
    pub root: PathBuf,
    pub head: String,
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
    pub commits: Vec<CommitEntry>,
    pub has_more: bool,
}
#[derive(Debug)]
pub struct HistoryReply {
    pub id: u64,
    pub document_version: u64,
    pub path: PathBuf,
    pub result: Result<LineHistory, GitError>,
}
pub struct HistoryWorker {
    lane: Lane<HistoryRequest, HistoryReply>,
}
impl Default for HistoryWorker {
    fn default() -> Self {
        Self::new()
    }
}
impl HistoryWorker {
    pub fn new() -> Self {
        Self {
            lane: Lane::new("git-line-history", |id, request: HistoryRequest, cancel| {
                HistoryReply {
                    id,
                    document_version: request.document_version,
                    path: request.path.clone(),
                    result: (|| {
                        let repo = repository_for(&request.workspace, &request.path, cancel)?;
                        repo.line_history(
                            &request.path,
                            request.start,
                            request.end,
                            Some(&request.contents),
                            request.limit,
                            cancel,
                        )
                    })(),
                }
            }),
        }
    }
    pub fn request(&self, request: HistoryRequest) -> Result<u64, GitError> {
        validate_history(request.start, request.end, request.limit)?;
        validate_request(&request.contents, request.start, 1)?;
        self.lane.request(request)
    }
    pub fn cancel(&self) {
        self.lane.cancel();
    }
    pub fn poll(&self) -> Option<HistoryReply> {
        self.lane.poll()
    }
    pub fn is_busy(&self) -> bool {
        self.lane.is_busy()
    }
}
fn validate_history(start: usize, end: usize, limit: usize) -> Result<(), GitError> {
    if start == 0
        || end < start
        || end.saturating_sub(start) >= MAX_LINES
        || !(1..=100).contains(&limit)
    {
        return Err(GitError::invalid(
            "Histórico de linhas requer intervalo1–500 e limite1–100 commits",
        ));
    }
    Ok(())
}
impl Repository {
    /// Follow the selected HEAD line range. When a current editor buffer is
    /// supplied it must equal HEAD; silently interpreting dirty line numbers in
    /// HEAD would show a different line's history.
    pub fn line_history(
        &self,
        path: &Path,
        start: usize,
        end: usize,
        contents: Option<&str>,
        limit: usize,
        cancel: &AtomicBool,
    ) -> Result<LineHistory, GitError> {
        validate_history(start, end, limit)?;
        let absolute = if path.is_absolute() {
            path.into()
        } else {
            self.root.join(path)
        };
        validate_path(&self.root, &absolute)?;
        let path = self.relative_path(&absolute)?;
        let actual = Repository::discover(absolute.parent().unwrap_or(&self.root), cancel)?;
        if actual.root != self.root || actual.git_dir != self.git_dir {
            return Err(GitError::invalid(
                "Arquivo pertence a outro repositório Git",
            ));
        }
        let before = head(self, cancel)?.ok_or_else(|| {
            GitError::unsupported("Histórico indisponível antes do primeiro commit")
        })?;
        let version = self.file_at(&before, &path, cancel)?;
        if version.bytes.len() > MAX_CONTENTS || version.bytes.contains(&0) {
            return Err(GitError::unsupported(
                "Histórico de linhas requer texto até4MiB",
            ));
        }
        if let Some(contents) = contents {
            if contents.as_bytes() != version.bytes {
                return Err(GitError::unsupported(
                    "Histórico de linhas requer conteúdo idêntico ao HEAD; use o histórico do arquivo para alterações locais",
                ));
            }
        } else {
            let mut options = fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK);
            }
            let file = options.open(&absolute)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() > MAX_CONTENTS as u64 {
                return Err(GitError::unsupported(
                    "Histórico de linhas requer arquivo regular até4MiB",
                ));
            }
            let mut disk = Vec::new();
            file.take(MAX_CONTENTS as u64 + 1).read_to_end(&mut disk)?;
            if disk != version.bytes {
                return Err(GitError::unsupported(
                    "Histórico de linhas requer arquivo idêntico ao HEAD",
                ));
            }
        }
        let text = std::str::from_utf8(&version.bytes)
            .map_err(|_| GitError::unsupported("Histórico de linhas requer UTF-8"))?;
        let physical = text.bytes().filter(|b| *b == b'\n').count()
            + usize::from(!text.is_empty() && !text.ends_with('\n'));
        if end > physical {
            return Err(GitError::invalid(
                "Intervalo de histórico fora das linhas do HEAD",
            ));
        }
        // A single argument holds -L's literal range/path grammar. It never
        // becomes a shell command or pathspec; NUL cannot enter OsString args.
        let mut range = OsString::from(format!("{start},{end}:"));
        range.push(path.as_os_str());
        let arguments = vec![
            "log".into(),
            "--no-patch".into(),
            "--no-show-signature".into(),
            "--no-ext-diff".into(),
            "--no-textconv".into(),
            "--encoding=UTF-8".into(),
            "-z".into(),
            "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s".into(),
            format!("--max-count={}", limit + 1).into(),
            "-L".into(),
            range,
            before.clone().into(),
        ];
        let raw = self.run_checked(&arguments, None, cancel)?.stdout;
        let mut fields = raw.split(|b| *b == 0).collect::<Vec<_>>();
        if fields.last().is_some_and(|field| field.is_empty()) {
            fields.pop();
        }
        if fields.len() % 6 != 0 {
            return Err(GitError::invalid(
                "Formato de histórico de linhas inesperado",
            ));
        }
        let mut commits = Vec::new();
        for (index, row) in fields.chunks_exact(6).enumerate() {
            if index > limit {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Histórico excedeu limite solicitado",
                ));
            }
            let oid = std::str::from_utf8(row[0])
                .map_err(|_| GitError::invalid("OID de histórico inválido"))?;
            if !valid_oid(oid) {
                return Err(GitError::invalid("OID de histórico inválido"));
            }
            let parents = std::str::from_utf8(row[1])
                .map_err(|_| GitError::invalid("Parents inválidos"))?
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if parents.iter().any(|p| !valid_oid(p)) {
                return Err(GitError::invalid("Parent inválido"));
            }
            commits.push(CommitEntry {
                oid: oid.into(),
                parents,
                author: bounded(row[2], 4096)?,
                email: bounded(row[3], 4096)?,
                timestamp: std::str::from_utf8(row[4])
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| GitError::invalid("Data do histórico inválida"))?,
                subject: bounded(row[5], 8192)?,
                refs: vec![],
                lane: 0,
                edges: vec![],
            });
        }
        let has_more = commits.len() > limit;
        commits.truncate(limit);
        same_head(self, &Some(before.clone()), cancel)?;
        Ok(LineHistory {
            root: self.root.clone(),
            head: before,
            path,
            start,
            end,
            commits,
            has_more,
        })
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use std::{cell::Cell, process::Command};
    fn git(root: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().into()
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Cache Fixture"),
            ("user.email", "cache@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            git(dir.path(), &["config", key, value]);
        }
        let hooks = dir.path().join(".git/empty-hooks");
        fs::create_dir(&hooks).unwrap();
        git(
            dir.path(),
            &["config", "core.hooksPath", hooks.to_str().unwrap()],
        );
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/a"), "one\ntwo\n").unwrap();
        fs::write(dir.path().join("src/b"), "one\ntwo\n").unwrap();
        git(dir.path(), &["add", "--all"]);
        git(dir.path(), &["commit", "-m", "Initial cache"]);
        dir
    }
    fn request(root: &Path) -> Request {
        Request {
            workspace: root.into(),
            path: root.join("src/a"),
            document_version: 7,
            contents: Arc::from("one\ntwo\n"),
            start: 1,
            lines: 2,
        }
    }
    fn read(
        cache: &mut Option<CacheEntry>,
        request: &Request,
        now: Instant,
        reads: &Cell<usize>,
    ) -> Result<Snapshot, GitError> {
        cached_read(
            cache,
            request,
            &AtomicBool::new(false),
            now,
            &mut |request, cancel| {
                reads.set(reads.get() + 1);
                inspect(request, cancel)
            },
        )
    }
    #[test]
    fn identical_request_hits_without_running_uncached_inspect_or_blame() {
        let dir = fixture();
        let mut request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        let first = read(&mut cache, &request, now, &reads).unwrap();
        let second = read(&mut cache, &request, now + Duration::from_secs(3), &reads).unwrap();
        assert_eq!(
            reads.get(),
            1,
            "a cache hit must bypass inspect and its real Git blame"
        );
        assert_eq!(first.head, second.head);
        assert_eq!(first.lines[0].oid, second.lines[0].oid);
        // Equal content from a different allocation is still a correct hit.
        request.contents = Arc::from(String::from("one\ntwo\n"));
        read(&mut cache, &request, now + Duration::from_secs(6), &reads).unwrap();
        assert_eq!(reads.get(), 1);
    }
    #[test]
    fn buffer_version_range_path_workspace_and_repository_changes_miss() {
        let dir = fixture();
        let mut request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        read(&mut cache, &request, now, &reads).unwrap();
        request.contents = Arc::from("changed\ntwo\n");
        assert!(read(&mut cache, &request, now, &reads).unwrap().lines[0].uncommitted);
        assert_eq!(reads.get(), 2);
        request.document_version += 1;
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 3);
        request.start = 2;
        request.lines = 1;
        let page = read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(page.lines[0].line, 2);
        assert_eq!(reads.get(), 4);
        request.path = dir.path().join("src/b");
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 5);
        request.workspace = dir.path().join("src");
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 6);
        let other = fixture();
        request.workspace = other.path().into();
        request.path = other.path().join("src/a");
        let page = read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(page.repository.root, other.path());
        assert_eq!(reads.get(), 7);
    }
    #[test]
    fn head_and_path_index_changes_miss_while_unrelated_index_changes_can_hit() {
        let dir = fixture();
        let request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        read(&mut cache, &request, now, &reads).unwrap();
        git(dir.path(), &["commit", "--allow-empty", "-m", "New HEAD"]);
        let page = read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 2);
        assert_eq!(
            page.head.as_deref(),
            Some(git(dir.path(), &["rev-parse", "HEAD"]).as_str())
        );
        git(dir.path(), &["update-index", "--chmod=+x", "src/a"]);
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 3);
        git(dir.path(), &["update-index", "--chmod=+x", "src/b"]);
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(
            reads.get(),
            3,
            "unrelated index entries do not change this blame input"
        );
    }
    #[test]
    fn ttl_expires_at_thirty_seconds_and_refreshes_mailmap_without_head_change() {
        let dir = fixture();
        let request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        let first = read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(first.lines[0].author, "Cache Fixture");
        fs::write(
            dir.path().join(".mailmap"),
            "Mapped Fixture <mapped@example.invalid> Cache Fixture <cache@example.invalid>\n",
        )
        .unwrap();
        let cached = read(&mut cache, &request, now + Duration::from_secs(29), &reads).unwrap();
        assert_eq!(cached.lines[0].author, "Cache Fixture");
        assert_eq!(reads.get(), 1);
        let refreshed = read(&mut cache, &request, now + CACHE_TTL, &reads).unwrap();
        assert_eq!(reads.get(), 2);
        assert_eq!(refreshed.head, first.head);
        assert_eq!(refreshed.lines[0].author, "Mapped Fixture");
    }
    #[cfg(unix)]
    #[test]
    fn cached_file_and_directory_symlinks_are_revalidated_before_every_hit() {
        let dir = fixture();
        let request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        read(&mut cache, &request, now, &reads).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), "secret").unwrap();
        fs::remove_file(&request.path).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), &request.path).unwrap();
        assert_eq!(
            read(&mut cache, &request, now, &reads).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(reads.get(), 1);
        assert!(cache.is_none());
        fs::remove_file(&request.path).unwrap();
        fs::write(&request.path, "one\ntwo\n").unwrap();
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 2);
        fs::rename(dir.path().join("src"), dir.path().join("src-original")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("src-original"), dir.path().join("src"))
            .unwrap();
        assert_eq!(
            read(&mut cache, &request, now, &reads).unwrap_err().kind,
            GitErrorKind::Unsupported
        );
        assert_eq!(reads.get(), 2);
        assert!(cache.is_none());
    }
    #[cfg(unix)]
    #[test]
    fn replaced_git_directory_identity_misses_even_with_identical_head_and_index() {
        fn copy_directory(source: &Path, target: &Path) {
            fs::create_dir(target).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let destination = target.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_directory(&entry.path(), &destination)
                } else {
                    fs::copy(entry.path(), destination).unwrap();
                }
            }
        }
        let dir = fixture();
        let request = request(dir.path());
        let mut cache = None;
        let reads = Cell::new(0);
        let now = Instant::now();
        read(&mut cache, &request, now, &reads).unwrap();
        let original = cache.as_ref().unwrap().identity.clone();
        let saved = dir.path().join("old-git");
        fs::rename(dir.path().join(".git"), &saved).unwrap();
        copy_directory(&saved, &dir.path().join(".git"));
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(reads.get(), 2);
        let replacement = &cache.as_ref().unwrap().identity;
        assert_eq!(original.head, replacement.head);
        assert_eq!(original.index, replacement.index);
        assert_ne!(original.folders, replacement.folders);
    }
    #[test]
    fn errors_cancellation_and_concurrent_metadata_changes_never_enter_cache() {
        let dir = fixture();
        let request = request(dir.path());
        let mut cache = None;
        let now = Instant::now();
        let failures = Cell::new(0);
        for _ in 0..2 {
            let error = cached_read(
                &mut cache,
                &request,
                &AtomicBool::new(false),
                now,
                &mut |_, _| {
                    failures.set(failures.get() + 1);
                    Err(GitError::new(GitErrorKind::Git, "controlled read failure"))
                },
            )
            .unwrap_err();
            assert_eq!(error.kind, GitErrorKind::Git);
            assert!(cache.is_none());
        }
        assert_eq!(failures.get(), 2);
        let reads = Cell::new(0);
        read(&mut cache, &request, now, &reads).unwrap();
        assert_eq!(
            cached_read(
                &mut cache,
                &request,
                &AtomicBool::new(true),
                now,
                &mut inspect
            )
            .unwrap_err()
            .kind,
            GitErrorKind::Cancelled
        );
        assert!(cache.is_none());
        let error = cached_read(
            &mut cache,
            &request,
            &AtomicBool::new(false),
            now,
            &mut |request, cancel| {
                let snapshot = inspect(request, cancel)?;
                git(
                    dir.path(),
                    &["commit", "--allow-empty", "-m", "Concurrent HEAD"],
                );
                Ok(snapshot)
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::Stale);
        assert!(cache.is_none());
        let error = cached_read(
            &mut cache,
            &request,
            &AtomicBool::new(false),
            now,
            &mut |request, cancel| {
                let snapshot = inspect(request, cancel)?;
                git(dir.path(), &["update-index", "--chmod=+x", "src/a"]);
                Ok(snapshot)
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, GitErrorKind::Stale);
        assert!(cache.is_none());
    }
}
