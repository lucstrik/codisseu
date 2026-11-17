//! Persisted query files. The database worker owns both files and their index.
use crate::remote::{ConnectionConfig, Engine};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
fn persist_index(path: &Path, bindings: &BTreeMap<PathBuf, String>) -> Result<(), String> {
    if serde_json::to_vec_pretty(bindings)
        .map_err(|e| e.to_string())?
        .len()
        >= 256 * 1024
    {
        return Err("Índice de queries excede o limite de 256 KiB".into());
    }
    super::remote::atomic_json(path, bindings)
}

/// Stable and portable even when datasource names/IDs contain path characters.
fn directory(root: &Path, owner: &str) -> PathBuf {
    root.join(format!("datasource-{:x}", Sha256::digest(owner.as_bytes())))
}
fn regular_directory(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Diretório de queries deve ser uma pasta regular, sem link simbólico".into());
    }
    Ok(())
}
fn source_owner(
    root: &Path,
    bindings: &BTreeMap<PathBuf, String>,
    source: &Path,
) -> Result<String, String> {
    let owner = bindings.get(source).ok_or("Query não cadastrada")?;
    let parent = source.parent().ok_or("Query sem diretório")?;
    if parent != root && parent != directory(root, owner) {
        return Err("Query fora do diretório do datasource vinculado".into());
    }
    regular_directory(root)?;
    regular_directory(parent)?;
    let metadata = std::fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Query deve ser arquivo regular, sem link simbólico".into());
    }
    Ok(owner.clone())
}
fn name_in_use(
    bindings: &BTreeMap<PathBuf, String>,
    owner: &str,
    target: &Path,
    except: Option<&Path>,
) -> bool {
    bindings.iter().any(|(path, id)| {
        id == owner && Some(path.as_path()) != except && path.file_name() == target.file_name()
    })
}

pub(crate) fn target(source: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 200
        || name.starts_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
    {
        return Err("Informe um nome de query sem diretórios ou caracteres especiais".into());
    }
    let extension = source
        .extension()
        .and_then(|e| e.to_str())
        .ok_or("Query sem extensão")?;
    let suffix = format!(".{extension}");
    let file = if name.to_lowercase().ends_with(&suffix.to_lowercase()) {
        name.to_owned()
    } else {
        format!("{name}{suffix}")
    };
    Ok(source.with_file_name(file))
}
/// Legacy files are moved into their source namespace only on explicit rename.
/// Startup never rewrites paths held by session restore or open documents.
pub(crate) fn rename_target(
    root: &Path,
    bindings: &BTreeMap<PathBuf, String>,
    source: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    let owner = bindings.get(source).ok_or("Query não cadastrada")?;
    let path = target(source, name)?;
    if source.parent() == Some(root) {
        Ok(directory(root, owner).join(path.file_name().ok_or("Query sem nome")?))
    } else {
        Ok(path)
    }
}

pub(crate) fn create(
    dir: &Path,
    index: &Path,
    bindings: &mut BTreeMap<PathBuf, String>,
    config: &ConnectionConfig,
) -> Result<PathBuf, String> {
    if bindings.len() >= 256 {
        return Err("Limite de 256 queries cadastradas".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    regular_directory(dir)?;
    let owner_dir = directory(dir, &config.id);
    std::fs::create_dir_all(&owner_dir).map_err(|e| e.to_string())?;
    regular_directory(&owner_dir)?;
    let extension = if config.engine == Engine::Mongo {
        "json"
    } else if config.engine.is_sql() {
        "sql"
    } else {
        "txt"
    };
    for number in 1..=10000 {
        let file = owner_dir.join(format!("query-{number}.{extension}"));
        if name_in_use(bindings, &config.id, &file, None) {
            continue;
        }
        let mut output = match std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&file)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        };
        let sample = match config.engine {
            Engine::Mongo => "{\"collection\":\"users\",\"filter\":{},\"limit\":100}\n",
            Engine::Redis => "PING\n",
            Engine::Memcached => "stats\n",
            _ => "SELECT 1;\n",
        };
        let mut next = bindings.clone();
        next.insert(file.clone(), config.id.clone());
        let result = (|| {
            output
                .write_all(sample.as_bytes())
                .map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
            persist_index(index, &next)
        })();
        if let Err(error) = result {
            drop(output);
            let _ = std::fs::remove_file(&file);
            return Err(error);
        }
        *bindings = next;
        return Ok(file);
    }
    Err("Não foi possível reservar um nome de query".into())
}
pub(crate) fn rename(
    dir: &Path,
    index: &Path,
    bindings: &mut BTreeMap<PathBuf, String>,
    source: &Path,
    name: &str,
) -> Result<crate::remote::ConsoleRename, String> {
    let owner = source_owner(dir, bindings, source)?;
    let path = rename_target(dir, bindings, source, name)?;
    if path == source {
        return Ok(crate::remote::ConsoleRename {
            source: source.into(),
            path,
            warning: None,
        });
    }
    if bindings.contains_key(&path) || name_in_use(bindings, &owner, &path, Some(source)) {
        return Err("Já existe uma query com esse nome neste datasource".into());
    }
    let parent = path.parent().ok_or("Query sem diretório")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    regular_directory(parent)?;
    // Same-filesystem hard link reserves the destination without overwriting.
    // Publish the durable index before removing the old name; failure keeps it.
    std::fs::hard_link(source, &path)
        .map_err(|e| format!("Não foi possível renomear a query: {e}"))?;
    let mut next = bindings.clone();
    next.remove(source);
    next.insert(path.clone(), owner);
    if let Err(error) = persist_index(index, &next) {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    *bindings = next;
    let warning = std::fs::remove_file(source)
        .err()
        .map(|_| "Query renomeada; o nome antigo não pôde ser removido".into());
    Ok(crate::remote::ConsoleRename {
        source: source.into(),
        path,
        warning,
    })
}

/// Remove only a registered managed file, publishing its index before unlinking.
/// An index failure preserves the file; an unlink failure restores its index.
pub(crate) fn delete(
    dir: &Path,
    index: &Path,
    bindings: &mut BTreeMap<PathBuf, String>,
    source: &Path,
) -> Result<(), String> {
    source_owner(dir, bindings, source)?;
    let mut next = bindings.clone();
    next.remove(source);
    persist_index(index, &next)?;
    if let Err(error) = std::fs::remove_file(source) {
        persist_index(index, bindings).map_err(|rollback| {
            format!("Arquivo preservado, mas o índice precisa ser recuperado: {error}; {rollback}")
        })?;
        return Err(format!("Não foi possível excluir a query: {error}"));
    }
    *bindings = next;
    Ok(())
}
