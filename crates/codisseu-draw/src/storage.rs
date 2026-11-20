use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::{MAX_SCENE_BYTES, Result, Scene};

/// Compare against the bytes actually loaded, rather than unreliable mtimes.
/// Default means a new destination that must not already exist.
#[derive(Clone, Debug, Default)]
pub struct DiskSnapshot {
    path: Option<PathBuf>,
    content: Option<Vec<u8>>,
    pub durability_warning: Option<String>,
}

impl DiskSnapshot {
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn exists(&self) -> bool {
        self.content.is_some()
    }
}

fn contextual(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn resolved_destination(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "Desenho sem nome de arquivo".to_owned())?;
    let parent = fs::canonicalize(parent).map_err(|e| contextual(parent, e))?;
    Ok(parent.join(name))
}

fn regular_metadata(path: &Path) -> Result<Metadata> {
    let meta = fs::symlink_metadata(path).map_err(|e| contextual(path, e))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(contextual(
            path,
            "Desenho precisa ser um arquivo regular, sem link simbólico",
        ));
    }
    if meta.len() > MAX_SCENE_BYTES as u64 {
        return Err(contextual(path, "Desenho excede 16 MiB"));
    }
    Ok(meta)
}

fn read_limited(path: &Path) -> Result<Vec<u8>> {
    regular_metadata(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|e| contextual(path, e))?;
    if !file.metadata().map_err(|e| contextual(path, e))?.is_file() {
        return Err(contextual(path, "Desenho precisa ser um arquivo regular"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_SCENE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| contextual(path, e))?;
    if bytes.len() > MAX_SCENE_BYTES {
        return Err(contextual(path, "Desenho excede 16 MiB"));
    }
    Ok(bytes)
}

pub fn load_scene(path: &Path) -> Result<(Scene, DiskSnapshot)> {
    // Resolve parent only, so a final-component symlink cannot escape inspection.
    let path = resolved_destination(path)?;
    let content = read_limited(&path)?;
    let text = std::str::from_utf8(&content).map_err(|e| contextual(&path, e))?;
    let scene = Scene::from_json(text)?;
    Ok((
        scene,
        DiskSnapshot {
            path: Some(path),
            content: Some(content),
            durability_warning: None,
        },
    ))
}

fn verify_existing(path: &Path, expected: &DiskSnapshot) -> Result<fs::Permissions> {
    if expected.path.as_deref() != Some(path)
        || fs::canonicalize(path).map_err(|e| contextual(path, e))? != path
    {
        return Err(contextual(
            path,
            "Caminho do desenho mudou; reabra o arquivo",
        ));
    }
    let meta = regular_metadata(path)?;
    if meta.permissions().readonly() {
        return Err(contextual(path, "Desenho é somente leitura"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            return Err(contextual(
                path,
                "Gravação de desenho com múltiplos hard links não permitida",
            ));
        }
    }
    if Some(read_limited(path)?.as_slice()) != expected.content.as_deref() {
        return Err(contextual(
            path,
            "Conflito: desenho mudou no disco; alterações locais preservadas",
        ));
    }
    Ok(meta.permissions())
}

/// Writes in the destination directory and atomically replaces only a verified
/// original. This is optimistic conflict detection, not a cross-process lock.
pub fn save_scene(path: &Path, scene: &Scene, expected: &DiskSnapshot) -> Result<DiskSnapshot> {
    let path = resolved_destination(path)?;
    let content = scene.to_json()?.into_bytes();
    let parent = path
        .parent()
        .ok_or_else(|| "Desenho sem diretório".to_owned())?;
    let permissions = if expected.content.is_some() {
        Some(verify_existing(&path, expected)?)
    } else {
        if fs::symlink_metadata(&path).is_ok() {
            return Err(contextual(
                &path,
                "Arquivo já existe; nenhum desenho foi sobrescrito",
            ));
        }
        None
    };
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| contextual(parent, e))?;
    temporary
        .write_all(&content)
        .map_err(|e| contextual(&path, e))?;
    if let Some(permissions) = permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .map_err(|e| contextual(&path, e))?;
    }
    temporary
        .as_file()
        .sync_all()
        .map_err(|e| contextual(&path, e))?;
    if expected.content.is_some() {
        verify_existing(&path, expected)?;
        temporary
            .persist(&path)
            .map_err(|e| contextual(&path, e.error))?;
    } else {
        // Unlike exists()+rename, this also protects a concurrently created file.
        temporary
            .persist_noclobber(&path)
            .map_err(|e| contextual(&path, e.error))?;
    }
    let mut durability_warning = None;
    #[cfg(unix)]
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        durability_warning = Some(format!(
            "Desenho salvo; sincronização do diretório falhou: {error}"
        ));
    }
    Ok(DiskSnapshot {
        path: Some(path),
        content: Some(content),
        durability_warning,
    })
}
