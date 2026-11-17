//! Read-only, literal commit search independent from SCM mutation queues.
//!
//! No revision means HEAD. The returned resolved OID can be supplied as the
//! next page's reference to keep pagination stable. Query searches subject and
//! body, author searches name/email, and both filters must match. Git's literal
//! case-insensitive matching under the runner's C locale folds ASCII case;
//! non-ASCII characters retain Git's locale-dependent matching semantics.
//! Path filters accept a file or directory, including historical paths, without
//! --follow (the existing file-history service handles rename following).
use crate::{
    CommitEntry, GitError, GitErrorKind, Repository,
    autoria::Lane,
    repository::{args, cancelled, os_bytes, safe_parents, trim_line, valid_oid},
};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub workspace: PathBuf,
    pub root: PathBuf,
    pub query: String,
    pub author: String,
    pub path: Option<PathBuf>,
    pub reference: Option<String>,
    pub skip: usize,
    pub limit: usize,
}
#[derive(Clone, Debug)]
pub struct SearchPage {
    pub root: PathBuf,
    pub commits: Vec<CommitEntry>,
    pub has_more: bool,
    pub skip: usize,
    /// Immutable OID used for this read; None only for an unborn HEAD.
    pub resolved_reference: Option<String>,
}
#[derive(Debug)]
pub struct Reply {
    pub id: u64,
    pub request: Request,
    pub result: Result<SearchPage, GitError>,
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
        Self {
            lane: Lane::new("git-commit-search", |id, request, cancel| Reply {
                id,
                result: search(&request, cancel),
                request,
            }),
        }
    }
    pub fn request(&self, request: Request) -> Result<u64, GitError> {
        validate(&request)?;
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
fn validate(request: &Request) -> Result<(), GitError> {
    if request.limit == 0 || request.limit > 200 || request.skip > 5000 {
        return Err(GitError::invalid(
            "Busca Git limitada a200 commits por página/5000 de deslocamento",
        ));
    }
    if request.query.len() > 4096 || request.author.len() > 1024 {
        return Err(GitError::invalid(
            "Busca Git aceita texto até4096 bytes e autor até1024 bytes",
        ));
    }
    if [&request.query, &request.author]
        .iter()
        .any(|value| value.contains(['\0', '\r', '\n']))
    {
        return Err(GitError::invalid(
            "Filtros Git devem ser texto em uma única linha, sem NUL",
        ));
    }
    if request.reference.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 1024 || value.starts_with('-') || value.contains('\0')
    }) {
        return Err(GitError::invalid("Referência de busca Git inválida"));
    }
    if !request.workspace.is_absolute() || !request.root.is_absolute() {
        return Err(GitError::invalid(
            "Workspace e repositório da busca devem ser absolutos",
        ));
    }
    for path in [&request.workspace, &request.root] {
        if path.components().any(|part| {
            matches!(part, Component::ParentDir | Component::CurDir)
                || os_bytes(part.as_os_str()).contains(&0)
        }) {
            return Err(GitError::invalid("Caminho de busca Git inválido"));
        }
    }
    Ok(())
}
fn repository(request: &Request, cancel: &AtomicBool) -> Result<Repository, GitError> {
    validate(request)?;
    cancelled(cancel)?;
    let workspace = fs::canonicalize(&request.workspace)?;
    let root = fs::canonicalize(&request.root)?;
    if !workspace.is_dir() || !root.is_dir() {
        return Err(GitError::invalid("Busca Git requer diretórios reais"));
    }
    if root.starts_with(&workspace) {
        let relative = request
            .root
            .strip_prefix(&workspace)
            .map_err(|_| GitError::invalid("O repositório deve permanecer dentro do workspace"))?;
        if relative.components().any(|part| part.as_os_str() == ".git") {
            return Err(GitError::invalid(
                "Busca não pode usar diretórios internos Git",
            ));
        }
        // Appending a non-existent leaf makes safe_parents check root itself.
        safe_parents(&workspace, &relative.join("codisseu-path-check"))?;
    } else if workspace.starts_with(&root) {
        let discovered = Repository::discover(&workspace, cancel)?;
        if discovered.root != root {
            return Err(GitError::invalid(
                "Workspace pertence a outro repositório Git",
            ));
        }
    } else {
        return Err(GitError::invalid("Repositório de busca fora do workspace"));
    }
    let repository = Repository::discover(&root, cancel)?;
    if repository.root != root || repository.bare {
        return Err(GitError::invalid(
            "Selecione a raiz da árvore de trabalho Git para buscar",
        ));
    }
    Ok(repository)
}
fn filter_path(repo: &Repository, path: Option<&Path>) -> Result<Option<PathBuf>, GitError> {
    let Some(path) = path else { return Ok(None) };
    let path = if path.is_absolute() {
        path.strip_prefix(&repo.root)
            .map_err(|_| GitError::invalid("Filtro de caminho fora do repositório"))?
    } else {
        path
    };
    let mut relative = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(value) if value != ".git" && !os_bytes(value).contains(&0) => {
                relative.push(value);
            }
            Component::CurDir => {}
            _ => return Err(GitError::invalid("Filtro de caminho Git inválido")),
        }
    }
    if relative.as_os_str().is_empty() {
        return Ok(None);
    }
    safe_parents(&repo.root, &relative)?;
    match fs::symlink_metadata(repo.root.join(&relative)) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(GitError::unsupported("Filtro por symlink não é seguido"));
        }
        Ok(metadata) if !metadata.is_file() && !metadata.is_dir() => {
            return Err(GitError::unsupported(
                "Filtro de caminho requer arquivo ou diretório",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(Some(relative))
}
pub fn search(request: &Request, cancel: &AtomicBool) -> Result<SearchPage, GitError> {
    let repo = repository(request, cancel)?;
    repo.search_validated(request, cancel)
}
impl Repository {
    pub fn search_history(
        &self,
        request: &Request,
        cancel: &AtomicBool,
    ) -> Result<SearchPage, GitError> {
        let expected = repository(request, cancel)?;
        if self.root != expected.root || self.git_dir != expected.git_dir {
            return Err(GitError::invalid("Busca pertence a outro repositório Git"));
        }
        self.search_validated(request, cancel)
    }
    fn search_validated(
        &self,
        request: &Request,
        cancel: &AtomicBool,
    ) -> Result<SearchPage, GitError> {
        let path = filter_path(self, request.path.as_deref())?;
        let resolved = if let Some(reference) = &request.reference {
            Some(self.resolve(reference, cancel)?)
        } else {
            let output = self.run(
                &args(&["rev-parse", "--verify", "--quiet", "HEAD"]),
                None,
                cancel,
            )?;
            if output.status.success() {
                let oid = std::str::from_utf8(trim_line(&output.stdout))
                    .map_err(|_| GitError::invalid("HEAD da busca inválido"))?;
                if !valid_oid(oid) {
                    return Err(GitError::invalid("HEAD da busca inválido"));
                }
                Some(oid.into())
            } else if output.status.code() == Some(1) {
                None
            } else {
                return Err(GitError::new(
                    GitErrorKind::Git,
                    crate::redact(&output.stderr),
                ));
            }
        };
        let Some(reference) = resolved.as_ref() else {
            return Ok(SearchPage {
                root: self.root.clone(),
                commits: vec![],
                has_more: false,
                skip: request.skip,
                resolved_reference: None,
            });
        };
        let mut arguments = args(&[
            "log",
            "--topo-order",
            "--date-order",
            "--no-patch",
            "--no-show-signature",
            "--no-ext-diff",
            "--no-textconv",
            "--encoding=UTF-8",
            "-z",
            "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x00%D",
            "--fixed-strings",
            "--regexp-ignore-case",
            &format!("--max-count={}", request.limit + 1),
            &format!("--skip={}", request.skip),
        ]);
        if !request.query.is_empty() {
            arguments.push(format!("--grep={}", request.query).into());
        }
        if !request.author.is_empty() {
            arguments.push(format!("--author={}", request.author).into());
        }
        arguments.push(reference.into());
        arguments.push("--".into());
        if let Some(path) = path {
            arguments.push(path.into_os_string());
        }
        let bytes = self.run_checked(&arguments, None, cancel)?.stdout;
        let mut fields = bytes.split(|byte| *byte == 0).collect::<Vec<_>>();
        if fields.last().is_some_and(|field| field.is_empty()) {
            fields.pop();
        }
        if fields.len() % 7 != 0 {
            return Err(GitError::invalid("Formato da busca Git inesperado"));
        }
        let mut commits = Vec::new();
        for row in fields.chunks_exact(7) {
            if commits.len() > request.limit {
                return Err(GitError::new(
                    GitErrorKind::OutputLimit,
                    "Git retornou mais commits que o limite solicitado",
                ));
            }
            let oid = std::str::from_utf8(row[0])
                .map_err(|_| GitError::invalid("OID na busca inválido"))?;
            if !valid_oid(oid) {
                return Err(GitError::invalid("OID na busca inválido"));
            }
            let parents = std::str::from_utf8(row[1])
                .map_err(|_| GitError::invalid("Parents na busca inválidos"))?
                .split_ascii_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if parents.iter().any(|p| !valid_oid(p)) {
                return Err(GitError::invalid("Parent OID na busca inválido"));
            }
            let text = |value: &[u8], limit: usize| -> Result<String, GitError> {
                if value.len() > limit {
                    return Err(GitError::new(
                        GitErrorKind::OutputLimit,
                        "Metadado da busca excedeu orçamento",
                    ));
                }
                Ok(String::from_utf8_lossy(value).into_owned())
            };
            commits.push(CommitEntry {
                oid: oid.into(),
                parents,
                author: text(row[2], 4096)?,
                email: text(row[3], 4096)?,
                timestamp: std::str::from_utf8(row[4])
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| GitError::invalid("Timestamp da busca inválido"))?,
                subject: text(row[5], 8192)?,
                refs: text(row[6], 16384)?
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect(),
                lane: 0,
                edges: vec![],
            });
        }
        let has_more = commits.len() > request.limit;
        commits.truncate(request.limit);
        cancelled(cancel)?;
        Ok(SearchPage {
            root: self.root.clone(),
            commits,
            has_more,
            skip: request.skip,
            resolved_reference: resolved,
        })
    }
}
