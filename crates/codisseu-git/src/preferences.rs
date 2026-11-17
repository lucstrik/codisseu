//! Worktree-local branch presentation preferences, bounded and atomically saved.
use crate::{
    GitError, Repository,
    repository::{args, cancelled},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::atomic::AtomicBool,
};
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BranchPreferences {
    pub recent: Vec<String>,
    pub favorites: Vec<String>,
}
#[derive(Serialize, Deserialize)]
struct Document {
    version: u32,
    preferences: BranchPreferences,
}
fn location(repo: &Repository) -> Result<PathBuf, GitError> {
    let directory = repo.git_dir.join("codisseu");
    match fs::symlink_metadata(&directory) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(GitError::invalid(
                "Preferências Git não aceitam links simbólicos",
            ));
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    Ok(directory.join("branches.json"))
}
fn validate(preferences: &BranchPreferences) -> Result<(), GitError> {
    if preferences.recent.len() > 16
        || preferences.favorites.len() > 128
        || preferences
            .recent
            .iter()
            .chain(&preferences.favorites)
            .any(|reference| {
                reference.len() > 4096
                    || !reference.starts_with("refs/")
                    || reference.chars().any(char::is_control)
            })
    {
        return Err(GitError::invalid(
            "Preferências Git excedem16 recentes/128 favoritos ou contêm referências inválidas",
        ));
    }
    Ok(())
}
pub fn load(repo: &Repository, cancel: &AtomicBool) -> Result<BranchPreferences, GitError> {
    cancelled(cancel)?;
    let path = location(repo)?;
    let metadata = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BranchPreferences::default());
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err(GitError::invalid("Arquivo de preferências Git inválido"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let document: Document = serde_json::from_slice(&bytes)
        .map_err(|_| GitError::invalid("Preferências Git inválidas"))?;
    if document.version != 1 {
        return Err(GitError::invalid(
            "Versão das preferências Git desconhecida",
        ));
    }
    validate(&document.preferences)?;
    Ok(document.preferences)
}
fn update(
    repo: &Repository,
    cancel: &AtomicBool,
    modify: impl FnOnce(&mut BranchPreferences),
) -> Result<BranchPreferences, GitError> {
    let path = location(repo)?;
    let directory = path.parent().unwrap();
    fs::create_dir_all(directory)?;
    let _ = location(repo)?;
    let lockpath = directory.join("branches.lock");
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true).read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW).mode(0o600);
    }
    let lock = options.open(&lockpath)?;
    if !lock.metadata()?.is_file() {
        return Err(GitError::invalid("Bloqueio de preferências inválido"));
    }
    lock.try_lock().map_err(|error| {
        GitError::new(
            crate::GitErrorKind::Io,
            format!("Preferências Git ocupadas; aguarde: {error}"),
        )
    })?;
    // Read while holding the same stable inode lock. Process crashes release the
    // OS lock; no stale create_new lock can permanently block this user action.
    let mut preferences = load(repo, cancel)?;
    modify(&mut preferences);
    validate(&preferences)?;
    cancelled(cancel)?;
    let json = serde_json::to_vec(&Document {
        version: 1,
        preferences: preferences.clone(),
    })
    .map_err(|_| GitError::invalid("Não foi possível salvar preferências Git"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&json)?;
    temporary.as_file().sync_all()?;
    cancelled(cancel)?;
    temporary
        .persist(&path)
        .map_err(|error| GitError::from(error.error))?;
    fs::File::open(directory)?.sync_all()?;
    Ok(preferences)
}
pub fn set_favorite(
    repo: &Repository,
    reference: &str,
    favorite: bool,
    cancel: &AtomicBool,
) -> Result<BranchPreferences, GitError> {
    if !reference.starts_with("refs/heads/") && !reference.starts_with("refs/remotes/") {
        return Err(GitError::invalid(
            "Favoritos exigem referência de branch completa",
        ));
    }
    repo.run_checked(&args(&["check-ref-format", reference]), None, cancel)?;
    let exists = repo.run(
        &args(&["show-ref", "--verify", "--quiet", "--", reference]),
        None,
        cancel,
    )?;
    if favorite && !exists.status.success() {
        return Err(GitError::invalid("Branch favorita não existe"));
    }
    update(repo, cancel, |preferences| {
        preferences.favorites.retain(|value| value != reference);
        if favorite {
            preferences.favorites.push(reference.into());
            preferences.favorites.sort();
        }
    })
}
pub(crate) fn record_current(
    repo: &Repository,
    cancel: &AtomicBool,
) -> Result<BranchPreferences, GitError> {
    let branch = repo.run(&args(&["symbolic-ref", "-q", "HEAD"]), None, cancel)?;
    if !branch.status.success() {
        return load(repo, cancel);
    }
    let reference = String::from_utf8_lossy(&branch.stdout).trim().to_owned();
    update(repo, cancel, |preferences| {
        preferences.recent.retain(|entry| entry != &reference);
        preferences.recent.insert(0, reference);
        preferences.recent.truncate(16);
    })
}
