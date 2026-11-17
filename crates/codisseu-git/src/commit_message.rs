//! Bounded, read-only context for commit messages; never stages or commits.
use crate::{DiffSide, GitError, Repository};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
pub const MAX_CONTEXT_BYTES: usize = 24 * 1024;
const MAX_FILES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub root: PathBuf,
    pub staged: bool,
    pub text: String,
    head: Option<String>,
    index_hash: String,
    worktree_hash: String,
}

pub fn collect(root: &Path, cancel: &AtomicBool) -> Result<Context, GitError> {
    let repo = Repository::discover(root, cancel)?;
    if repo.root != root || repo.bare {
        return Err(GitError::invalid(
            "Selecione a raiz de um repositório de trabalho",
        ));
    }
    let before = repo.snapshot(cancel)?;
    if before.entries.iter().any(|entry| entry.conflict) {
        return Err(GitError::invalid(
            "Resolva os conflitos antes de gerar a mensagem",
        ));
    }
    let staged = before
        .entries
        .iter()
        .any(|entry| !entry.untracked && !matches!(entry.index, '.' | ' ' | '?'));
    let files: Vec<_> = before
        .entries
        .iter()
        .filter(|entry| {
            if staged {
                !entry.untracked && !matches!(entry.index, '.' | ' ' | '?')
            } else {
                entry.untracked || !matches!(entry.worktree, '.' | ' ')
            }
        })
        .collect();
    if files.is_empty() {
        return Err(GitError::invalid(
            "Não há alterações para gerar uma mensagem de commit",
        ));
    }
    if files.len() > MAX_FILES {
        return Err(GitError::invalid(
            "A geração aceita até64arquivos; prepare uma seleção menor",
        ));
    }
    let mut text = format!(
        "Diff scope: {}\nChanged files: {}\n",
        if staged {
            "staged changes only"
        } else {
            "working tree and untracked files"
        },
        files.len()
    );
    for entry in files {
        let diff = repo.diff(
            &entry.path,
            if staged {
                DiffSide::Index
            } else {
                DiffSide::Working
            },
            cancel,
        )?;
        if diff.binary {
            text.push_str(&format!("\nBinary change: {:?}\n", entry.path));
        } else {
            let patch = std::str::from_utf8(&diff.bytes)
                .map_err(|_| GitError::invalid("O diff não é UTF-8; conteúdo preservado"))?;
            text.push('\n');
            text.push_str(patch);
        }
        if text.len() > MAX_CONTEXT_BYTES || text.contains('\0') {
            return Err(GitError::invalid(
                "O diff excede24KiB de contexto; prepare uma seleção menor",
            ));
        }
    }
    let after = repo.snapshot(cancel)?;
    if before.head != after.head
        || before.index_hash != after.index_hash
        || before.worktree_hash != after.worktree_hash
    {
        return Err(GitError::invalid(
            "O repositório mudou durante a leitura; gere novamente",
        ));
    }
    Ok(Context {
        root: repo.root,
        staged,
        text,
        head: before.head,
        index_hash: before.index_hash,
        worktree_hash: before.worktree_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-b", "main"]);
        for (k, v) in [
            ("user.name", "Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            git(dir.path(), &["config", k, v]);
        }
        fs::write(dir.path().join("a.ts"), "export const greeting = 'old';\n").unwrap();
        git(dir.path(), &["add", "a.ts"]);
        git(dir.path(), &["commit", "-m", "Initial"]);
        dir
    }
    #[test]
    fn commit_message_context_prefers_index_and_never_mutates_index_head_or_worktree() {
        let dir = fixture();
        let root = dir.path();
        let path = root.join("a.ts");
        fs::write(&path, "export const greeting = 'staged ação😀';\n").unwrap();
        git(root, &["add", "a.ts"]);
        fs::write(&path, "export const greeting = 'unstaged different';\n").unwrap();
        fs::write(root.join("new.ts"), "untracked content\n").unwrap();
        let head = git(root, &["rev-parse", "HEAD"]);
        let index = fs::read(root.join(".git/index")).unwrap();
        let disk = fs::read(&path).unwrap();
        let context = collect(root, &AtomicBool::new(false)).unwrap();
        assert!(context.staged);
        assert!(context.text.contains("staged ação😀"));
        assert!(!context.text.contains("unstaged different"));
        assert!(!context.text.contains("untracked content"));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(fs::read(path).unwrap(), disk);
        assert_eq!(git(root, &["rev-parse", "HEAD"]), head);
    }
    #[test]
    fn commit_message_context_includes_new_working_files_and_detects_same_length_edits() {
        let dir = fixture();
        let root = dir.path();
        fs::write(root.join("a.ts"), "export const greeting = 'new';\n").unwrap();
        fs::write(root.join("new.ts"), "// included new file\n").unwrap();
        let a = collect(root, &AtomicBool::new(false)).unwrap();
        assert!(!a.staged);
        assert!(a.text.contains("included new file"));
        fs::write(root.join("a.ts"), "export const greeting = 'two';\n").unwrap();
        let b = collect(root, &AtomicBool::new(false)).unwrap();
        assert_ne!(a, b);
        assert!(collect(root, &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn commit_message_context_empty_and_oversized_diffs_fail_without_changes() {
        let dir = fixture();
        let root = dir.path();
        assert!(collect(root, &AtomicBool::new(false)).is_err());
        fs::write(root.join("large.ts"), "x".repeat(MAX_CONTEXT_BYTES + 1)).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        assert!(
            collect(root, &AtomicBool::new(false))
                .unwrap_err()
                .to_string()
                .contains("24KiB")
        );
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }
}
