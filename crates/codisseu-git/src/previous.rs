//! Immutable, bounded file-history navigation for the editor. No Git mutations.
use crate::{
    GitError, Repository,
    line_changes::{MAX_BYTES, index_baseline},
    repository::{args, path_from_bytes, valid_oid},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
#[derive(Clone)]
enum Version {
    Bytes(String, Vec<u8>),
    Commit(String, PathBuf),
    Empty,
}
pub struct FileHistory {
    repo: Repository,
    versions: Vec<Version>,
    clean_working: Option<Vec<u8>>,
    truncated: bool,
}
pub struct PreviousPair {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub before_label: String,
    pub after_label: String,
    pub has_previous: bool,
}
fn bounded(bytes: Vec<u8>) -> Result<Vec<u8>, GitError> {
    if bytes.len() > MAX_BYTES || bytes.contains(&0) {
        Err(GitError::invalid(
            "Histórico requer texto de até4MiB por versão",
        ))
    } else {
        Ok(bytes)
    }
}
impl FileHistory {
    pub fn capture(path: &Path, editor: Vec<u8>, cancel: &AtomicBool) -> Result<Self, GitError> {
        let editor = bounded(editor)?;
        let repo = Repository::discover(path, cancel)?;
        let relative = repo.relative_path(path)?;
        let index = index_baseline(path, cancel)?.ok_or_else(|| {
            GitError::invalid("Arquivo ignorado, em conflito ou sem versão regular comparável")
        })?;
        let status = repo.snapshot(cancel)?;
        let mut historical_path = status
            .entries
            .iter()
            .find(|e| e.path == relative && e.index == 'R')
            .and_then(|e| e.original_path.clone())
            .unwrap_or(relative);
        let mut commits = Vec::new();
        if let Some(head) = status.head {
            let mut a = args(&[
                "log",
                "--follow",
                "--format=%x00%H%x00",
                "--name-status",
                "-z",
                "--max-count=201",
                &head,
                "--",
            ]);
            a.push(historical_path.as_os_str().into());
            let raw = repo.run_checked(&a, None, cancel)?.stdout;
            let mut fields = raw.split(|b| *b == 0).peekable();
            while let Some(field) = fields.next() {
                let field = field.strip_prefix(b"\n").unwrap_or(field);
                if field.is_empty() {
                    continue;
                }
                if let Ok(oid) = std::str::from_utf8(field)
                    && valid_oid(oid)
                {
                    commits.push(Version::Commit(oid.to_owned(), historical_path.clone()));
                    continue;
                }
                let first = fields
                    .next()
                    .ok_or_else(|| GitError::invalid("Histórico sem caminho"))?;
                if matches!(field.first(), Some(b'R' | b'C')) {
                    let _new = fields
                        .next()
                        .ok_or_else(|| GitError::invalid("Rename sem destino"))?;
                    historical_path = path_from_bytes(first)?;
                } else if !matches!(field.first(), Some(b'A' | b'M' | b'D' | b'T' | b'U')) {
                    return Err(GitError::invalid("Formato de histórico inesperado"));
                }
            }
        }
        let truncated = commits.len() > 200;
        commits.truncate(200);
        let head_bytes = match commits.first() {
            Some(Version::Commit(oid, p)) => bounded(repo.file_at(oid, p, cancel)?.bytes)?,
            _ => Vec::new(),
        };
        let clean = editor == index && index == head_bytes && !commits.is_empty();
        let mut versions = Vec::new();
        if !clean {
            if editor != index || commits.is_empty() {
                versions.push(Version::Bytes("Versão do editor".into(), editor.clone()));
            }
            if index != head_bytes {
                versions.push(Version::Bytes("Índice Git".into(), index));
            }
        }
        versions.extend(commits);
        if !truncated {
            versions.push(Version::Empty);
        }
        Ok(Self {
            repo,
            versions,
            clean_working: clean.then_some(editor),
            truncated,
        })
    }
    fn read(&self, v: &Version, c: &AtomicBool) -> Result<(Vec<u8>, String), GitError> {
        match v {
            Version::Bytes(label, b) => Ok((b.clone(), label.clone())),
            Version::Commit(oid, p) => Ok((
                bounded(self.repo.file_at(oid, p, c)?.bytes)?,
                format!("{} · {}", &oid[..8], p.display()),
            )),
            Version::Empty => Ok((Vec::new(), "Antes da criação do arquivo".into())),
        }
    }
    /// Last bounded pair, including the clean editor-to-commit transition.
    pub fn last_step(&self) -> usize {
        self.versions.len().saturating_sub(2) + usize::from(self.clean_working.is_some())
    }
    /// Moving forward eventually compares HEAD/index with the captured editor,
    /// even when both contain identical text (GitLens' current-version endpoint).
    pub fn working_pair(&self, cancel: &AtomicBool) -> Result<PreviousPair, GitError> {
        let mut pair = self.pair(0, cancel)?;
        if let Some(editor) = &self.clean_working {
            let (bytes, label) = self.read(&self.versions[0], cancel)?;
            pair.before = bytes;
            pair.before_label = label;
            pair.after = editor.clone();
            pair.after_label = "Versão do editor".into();
        }
        Ok(pair)
    }
    pub fn pair(&self, step: usize, c: &AtomicBool) -> Result<PreviousPair, GitError> {
        if step > 201 {
            return Err(GitError::invalid("Limite de200 revisões nesta comparação"));
        }
        let index = if self.clean_working.is_some() {
            step.saturating_sub(1)
        } else {
            step
        };
        let after = self
            .versions
            .get(index)
            .ok_or_else(|| GitError::invalid("Não há versão anterior"))?;
        let before = self.versions.get(index + 1).ok_or_else(|| {
            GitError::invalid(if self.truncated {
                "Limite de200 revisões nesta comparação"
            } else {
                "Início do histórico: não há versão anterior"
            })
        })?;
        let (before, before_label) = self.read(before, c)?;
        let (after, after_label) = if step == 0 {
            if let Some(b) = &self.clean_working {
                (b.clone(), "Versão do editor".into())
            } else {
                self.read(after, c)?
            }
        } else {
            self.read(after, c)?
        };
        let has_previous =
            (self.clean_working.is_some() && step == 0) || index + 2 < self.versions.len();
        Ok(PreviousPair {
            before,
            after,
            before_label,
            after_label,
            has_previous,
        })
    }
}
