//! Project-owned metadata paths. Legacy files stay in place and keep one owner.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOwner {
    schema_version: u32,
    workspace: Option<PathBuf>,
}

pub(super) struct Storage {
    anchor: PathBuf,
    owner: Option<PathBuf>,
}

pub(super) struct MetadataLock {
    _file: fs::File,
}
impl MetadataLock {
    pub(super) fn acquire(path: &Path) -> Result<Self, String> {
        let lock_path = path.with_extension("lock");
        fs::create_dir_all(lock_path.parent().ok_or("Cadastro sem diretório")?)
            .map_err(|e| e.to_string())?;
        if fs::symlink_metadata(&lock_path)
            .is_ok_and(|m| !m.is_file() || m.file_type().is_symlink())
        {
            return Err("Lock de cadastro deve ser arquivo regular".into());
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| e.to_string())?;
        file.try_lock()
            .map_err(|_| "Outra janela está gravando este cadastro; tente novamente")?;
        // The OS releases the lock if the process crashes. Keep its empty inode
        // so another waiting/opening window never locks an unrelated inode.
        Ok(Self { _file: file })
    }
}

pub(super) fn empty_path(anchor: &Path) -> PathBuf {
    anchor
        .with_extension("project-data")
        .join("without-project/database/datasources.json")
}

/// Runs on the file-session worker. Ordinary user files are unaffected; managed
/// consoles must belong to the restored project, including legacy association.
pub(crate) fn restorable_console(anchor: &Path, project: Option<&Path>, path: &Path) -> bool {
    let Ok(anchor) = canonical_anchor(anchor) else {
        return false;
    };
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let legacy = anchor.parent().unwrap_or(Path::new(".")).join("consoles");
    let managed =
        path.starts_with(&legacy) || path.starts_with(anchor.with_extension("project-data"));
    if !managed {
        return true;
    }
    let Ok(storage) = Storage::new(anchor) else {
        return false;
    };
    let Ok(profile) = storage.path(project) else {
        return false;
    };
    let directory = profile.parent().unwrap_or(Path::new(".")).join("consoles");
    if !path.starts_with(&directory) {
        return false;
    }
    super::remote::read_bindings(&profile.with_extension("console-bindings.json"), &directory)
        .is_ok_and(|bindings| {
            bindings
                .keys()
                .any(|p| fs::canonicalize(p).is_ok_and(|p| p == path))
        })
}

fn canonical_anchor(anchor: &Path) -> Result<PathBuf, String> {
    let absolute = std::path::absolute(anchor).map_err(|e| e.to_string())?;
    let parent = absolute.parent().ok_or("Perfil sem diretório")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let parent = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    Ok(parent.join(absolute.file_name().ok_or("Perfil sem nome")?))
}

fn limited_read(path: &Path) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("Associação de bancos excede o limite de tamanho".into());
    }
    Ok(bytes)
}

fn owner_from_marker(path: &Path) -> Result<Option<PathBuf>, String> {
    let marker: LegacyOwner = serde_json::from_slice(&limited_read(path)?)
        .map_err(|_| "Associação de bancos inválida; arquivo preservado")?;
    if marker.schema_version != 1
        || marker.workspace.as_ref().is_some_and(|p| {
            !p.is_absolute()
                || p.components().any(|c| {
                    matches!(
                        c,
                        std::path::Component::ParentDir | std::path::Component::CurDir
                    )
                })
        })
    {
        return Err(
            "Versão ou projeto da associação de bancos inválido; arquivo preservado".into(),
        );
    }
    Ok(marker.workspace)
}

impl Storage {
    /// Called only by the DB worker, before any window can persist its session.
    pub(super) fn new(anchor: PathBuf) -> Result<Self, String> {
        let anchor = canonical_anchor(&anchor)?;
        let marker = anchor.with_extension("database-scope.json");
        let owner = match fs::symlink_metadata(&marker) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() {
                    return Err("Associação de bancos deve ser arquivo regular".into());
                }
                owner_from_marker(&marker)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // The last global session may already have been saved after
                // visiting an unrelated project. Only an explicit claim owns it.
                let owner = None;
                let parent = marker
                    .parent()
                    .ok_or("Associação de bancos sem diretório")?;
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
                tmp.write_all(
                    &serde_json::to_vec_pretty(&LegacyOwner {
                        schema_version: 1,
                        workspace: owner.clone(),
                    })
                    .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                tmp.as_file().sync_all().map_err(|e| e.to_string())?;
                match tmp.persist_noclobber(&marker) {
                    Ok(_) => owner,
                    // Another window may initialize the same stable profile.
                    Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        owner_from_marker(&marker)?
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self { anchor, owner })
    }

    pub(super) fn refresh_owner(&mut self) -> Result<(), String> {
        self.owner = owner_from_marker(&self.anchor.with_extension("database-scope.json"))?;
        Ok(())
    }
    pub(super) fn metadata_guard(
        &mut self,
        project: Option<&Path>,
        path: &Path,
    ) -> Result<MetadataLock, String> {
        let guard = MetadataLock::acquire(&self.anchor.with_extension("database-scope.json"))?;
        self.refresh_owner()?;
        if self.path(project)? != path {
            return Err(
                "O vínculo dos bancos mudou em outra janela; reabra o projeto antes de gravar"
                    .into(),
            );
        }
        Ok(guard)
    }

    pub(super) fn legacy_available(&self) -> bool {
        self.owner.is_none()
            && [
                self.anchor.with_extension("datasources.json"),
                self.anchor
                    .with_extension("datasources.console-bindings.json"),
            ]
            .iter()
            .any(|p| match fs::symlink_metadata(p) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
                Err(_) => true,
                Ok(metadata) if !metadata.is_file() || metadata.len() > 256 * 1024 => true,
                Ok(_) => {
                    let value = limited_read(p)
                        .ok()
                        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
                    match value {
                        Some(serde_json::Value::Array(v)) => !v.is_empty(),
                        Some(serde_json::Value::Object(v)) => !v.is_empty(),
                        _ => true, // Invalid/unreadable data is preserved and recoverable.
                    }
                }
            })
    }

    pub(super) fn claim(&mut self, project: &Path) -> Result<(), String> {
        if fs::canonicalize(project).map_err(|e| e.to_string())? != project {
            return Err("Identidade do projeto mudou; vínculo não alterado".into());
        }
        let marker = self.anchor.with_extension("database-scope.json");
        let _lock = MetadataLock::acquire(&marker)?;
        self.owner = owner_from_marker(&marker)?;
        if let Some(owner) = &self.owner {
            return if owner == project {
                Ok(())
            } else {
                Err("Os bancos anteriores já foram vinculados a outro projeto".into())
            };
        }
        if !self.legacy_available() {
            return Err("Não há cadastro anterior para vincular".into());
        }
        let current = self.path(Some(project))?;
        for path in [
            current.clone(),
            current.with_extension("console-bindings.json"),
        ] {
            if !path.exists() {
                continue;
            }
            let bytes = limited_read(&path)?;
            let value: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|_| "Cadastro atual inválido; arquivos preservados")?;
            if !matches!(&value, serde_json::Value::Array(v) if v.is_empty())
                && !matches!(&value, serde_json::Value::Object(v) if v.is_empty())
            {
                return Err(
                    "O projeto já possui conexões ou queries; vínculo anterior preservado".into(),
                );
            }
        }
        // Replacing the association never rewrites the original datasource,
        // query, binding or credential files.
        super::remote::atomic_json(
            &marker,
            &LegacyOwner {
                schema_version: 1,
                workspace: Some(project.to_path_buf()),
            },
        )?;
        self.owner = Some(project.to_path_buf());
        Ok(())
    }

    pub(super) fn path(&self, project: Option<&Path>) -> Result<PathBuf, String> {
        let Some(project) = project else {
            return Ok(empty_path(&self.anchor));
        };
        let canonical = fs::canonicalize(project).map_err(|e| e.to_string())?;
        if canonical != project {
            return Err("Identidade do projeto mudou; bancos anteriores preservados".into());
        }
        if self.owner.as_deref() == Some(project) {
            return Ok(self.anchor.with_extension("datasources.json"));
        }
        // Encoded OS bytes distinguish non-UTF-8 Unix paths as well.
        let hash = Sha256::digest(project.as_os_str().as_encoded_bytes());
        Ok(self
            .anchor
            .with_extension("project-data")
            .join(format!("{hash:x}"))
            .join("database/datasources.json"))
    }
}
