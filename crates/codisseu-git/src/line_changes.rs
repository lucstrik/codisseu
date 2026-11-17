//! Read-only index baseline and bounded Git line diff for editor gutters.
use crate::{GitError, Repository, repository::args};
use std::{ffi::OsString, ops::Range, path::Path, sync::atomic::AtomicBool};
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_LINES: usize = 100_000;

/// Full-context, read-only index/editor comparison; never writes the worktree.
/// Identical non-empty versions are represented as context in both columns.
pub fn comparison_patch(
    base: &str,
    current: &str,
    cancel: &AtomicBool,
) -> Result<(String, bool), GitError> {
    if [base, current]
        .iter()
        .any(|s| s.len() > MAX_BYTES || s.contains('\0'))
    {
        return Err(GitError::invalid(
            "Comparação requer texto até4MiB/100000linhas por versão",
        ));
    }
    let base = base.replace("\r\n", "\n").replace('\r', "\n");
    let current = current.replace("\r\n", "\n").replace('\r', "\n");
    if [base.as_str(), current.as_str()]
        .iter()
        .any(|s| s.split('\n').count() > MAX_LINES)
    {
        return Err(GitError::invalid(
            "Comparação limitada a100000linhas por versão",
        ));
    }
    if cancel.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(GitError::invalid("Comparação cancelada"));
    }
    if base == current {
        if base.is_empty() {
            return Ok((String::new(), true));
        }
        let count = base.lines().count();
        let mut patch = format!("@@ -1,{count} +1,{count} @@\n");
        for line in base.lines() {
            patch.push(' ');
            patch.push_str(line);
            patch.push('\n');
        }
        return Ok((patch, true));
    }
    let temp = tempfile::tempdir()?;
    std::fs::write(temp.path().join("index"), base)?;
    std::fs::write(temp.path().join("editor"), current)?;
    let output = crate::process::run(
        temp.path(),
        &args(&[
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--unified=100000",
            "--",
            "index",
            "editor",
        ]),
        None,
        cancel,
        std::time::Duration::from_secs(20),
        &[],
    )?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(GitError::invalid(crate::redact(&output.stderr)));
    }
    let text =
        String::from_utf8(output.stdout).map_err(|_| GitError::invalid("Comparação não é UTF8"))?;
    // Paths in temporary Git headers are not product content. The caller owns
    // the real filename and lexer; keep only the complete source hunks.
    let start = text
        .find("@@ ")
        .ok_or_else(|| GitError::invalid("Git não retornou conteúdo comparável"))?;
    Ok((text[start..].to_owned(), false))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Added,
    Modified,
    Deleted,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// Current buffer lines; empty ranges mark deletion at a line boundary.
    pub lines: Range<usize>,
    pub kind: Kind,
}
/// None means the path has no comparable index entry (ignored, conflict or link).
/// A new, non-ignored file has an empty baseline, including before the first commit.
pub fn index_baseline(path: &Path, cancel: &AtomicBool) -> Result<Option<Vec<u8>>, GitError> {
    let repo = Repository::discover(
        path.parent()
            .ok_or_else(|| GitError::invalid("Arquivo sem pasta"))?,
        cancel,
    )?;
    let relative = repo.relative_path(path)?;
    let mut arguments = args(&["ls-files", "--stage", "-z", "--"]);
    arguments.push(relative.as_os_str().into());
    let entries = repo.run_checked(&arguments, None, cancel)?.stdout;
    if entries.is_empty() {
        let mut arguments = args(&["check-ignore", "--quiet", "--"]);
        arguments.push(relative.as_os_str().into());
        // check-ignore takes literal paths itself and rejects Git's literal
        // pathspec magic. Override only this read-only command's inherited flag.
        let output = crate::process::run(
            &repo.root,
            &arguments,
            None,
            cancel,
            std::time::Duration::from_secs(20),
            &[(OsString::from("GIT_LITERAL_PATHSPECS"), OsString::from("0"))],
        )?;
        return match output.status.code() {
            Some(0) => Ok(None),
            Some(1) => Ok(Some(Vec::new())),
            _ => Err(GitError::invalid(
                "Não foi possível verificar arquivo ignorado",
            )),
        };
    }
    // Parse metadata only; filenames may contain tabs/newlines or non-UTF8 bytes.
    let mut records = entries.split(|b| *b == 0).filter(|r| !r.is_empty());
    let record = records
        .next()
        .ok_or_else(|| GitError::invalid("Índice Git vazio"))?;
    if records.next().is_some() {
        return Ok(None);
    }
    let metadata = record.split(|b| *b == b'\t').next().unwrap_or_default();
    let fields: Vec<_> = metadata.split(|b| *b == b' ').collect();
    if fields.len() != 3 || fields[2] != b"0" || !matches!(fields[0], b"100644" | b"100755") {
        return Ok(None);
    }
    let oid = std::str::from_utf8(fields[1]).map_err(|_| GitError::invalid("OID inválido"))?;
    let size = repo.run_checked(&args(&["cat-file", "-s", oid]), None, cancel)?;
    let size: usize = std::str::from_utf8(&size.stdout)
        .unwrap_or("")
        .trim()
        .parse()
        .map_err(|_| GitError::invalid("Tamanho Git inválido"))?;
    if size > MAX_BYTES {
        return Err(GitError::invalid("Marcas Git limitadas a4MiB por versão"));
    }
    let data = repo
        .run_checked(
            &[
                OsString::from("cat-file"),
                OsString::from("blob"),
                OsString::from(oid),
            ],
            None,
            cancel,
        )?
        .stdout;
    if data.contains(&0) {
        return Ok(None);
    }
    Ok(Some(data))
}
pub fn compare(base: &str, current: &str, cancel: &AtomicBool) -> Result<Vec<Change>, GitError> {
    if [base, current]
        .iter()
        .any(|t| t.len() > MAX_BYTES || t.contains('\0'))
    {
        return Err(GitError::invalid(
            "Marcas Git requerem texto até4MiB por versão",
        ));
    }
    // Git stores LF with autocrlf; editor line positions are independent of EOL.
    let base = base.replace("\r\n", "\n").replace('\r', "\n");
    let current = current.replace("\r\n", "\n").replace('\r', "\n");
    if base == current {
        return Ok(Vec::new());
    }
    let base_len = base.split_inclusive('\n').count();
    let current_len = current.split_inclusive('\n').count();
    if base_len > MAX_LINES || current_len > MAX_LINES {
        return Err(GitError::invalid("Marcas Git limitadas a100000linhas"));
    }
    let temp = tempfile::tempdir()?;
    let a = temp.path().join("index");
    let b = temp.path().join("buffer");
    std::fs::write(&a, base)?;
    std::fs::write(&b, current)?;
    let edits = crate::text_merge::edits(temp.path(), &a, &b, base_len, current_len, cancel)?;
    Ok(edits
        .into_iter()
        .map(|edit| Change {
            kind: if edit.base.is_empty() {
                Kind::Added
            } else if edit.side.is_empty() {
                Kind::Deleted
            } else {
                Kind::Modified
            },
            lines: edit.side,
        })
        .collect())
}
