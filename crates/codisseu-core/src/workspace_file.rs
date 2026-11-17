//! Local VS Code workspace format. No tasks, settings or extensions are executed.
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFolder {
    pub name: String,
    pub path: PathBuf,
    pub available: bool,
}

pub fn is_workspace_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "code-workspace")
}

#[derive(Deserialize)]
struct File {
    folders: Vec<Folder>,
}
#[derive(Deserialize)]
struct Folder {
    path: PathBuf,
    name: Option<String>,
}

pub fn read_folders(path: &Path) -> Result<Vec<WorkspaceFolder>, String> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 1024 * 1024 {
        return Err("Workspace excede 1 MiB".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "Workspace deve ser UTF-8")?;
    let file: File = serde_json::from_slice(&jsonc(text.trim_start_matches('\u{feff}'))?)
        .map_err(|e| format!("Arquivo de workspace inválido: {e}"))?;
    if file.folders.is_empty() || file.folders.len() > 32 {
        return Err("Workspace deve conter de 1 a 32 pastas locais".into());
    }
    let base = path.parent().ok_or("Workspace sem diretório")?;
    let mut folders: Vec<WorkspaceFolder> = Vec::new();
    for folder in file.folders {
        let candidate = base.join(&folder.path);
        let (root, available) = match fs::canonicalize(&candidate) {
            Ok(root) => (root, true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A missing root remains visible but never authorizes file operations
                // or terminal startup. Canonicalize again on the next refresh.
                let mut normalized = PathBuf::new();
                for component in candidate.components() {
                    match component {
                        std::path::Component::CurDir => (),
                        std::path::Component::ParentDir => {
                            normalized.pop();
                        }
                        other => normalized.push(other.as_os_str()),
                    }
                }
                (normalized, false)
            }
            Err(e) => return Err(format!("Pasta do workspace {}: {e}", folder.path.display())),
        };
        if available && !root.is_dir() {
            return Err(format!("{} não é uma pasta", root.display()));
        }
        if folders.iter().any(|f| f.path == root) {
            continue;
        }
        if folders
            .iter()
            .any(|f| f.path.starts_with(&root) || root.starts_with(&f.path))
        {
            return Err(
                "Pastas sobrepostas no workspace: selecione a raiz ou seu subprojeto".into(),
            );
        }
        let name = folder.name.unwrap_or_else(|| {
            root.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        });
        if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err("Nome de pasta inválido no workspace".into());
        }
        folders.push(WorkspaceFolder {
            name,
            path: root,
            available,
        });
    }
    Ok(folders)
}

// Preserve strings and byte offsets. Only comments and trailing commas are
// blanked; JSON validation remains serde_json's responsibility.
fn jsonc(text: &str) -> Result<Vec<u8>, String> {
    let mut bytes = text.as_bytes().to_vec();
    let mut i = 0;
    let mut string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if string => {
                i += 2;
                continue;
            }
            b'"' => string = !string,
            b'/' if !string && bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    bytes[i] = b' ';
                    i += 1;
                }
                continue;
            }
            b'/' if !string && bytes.get(i + 1) == Some(&b'*') => {
                bytes[i] = b' ';
                bytes[i + 1] = b' ';
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    if bytes[i] != b'\n' {
                        bytes[i] = b' ';
                    }
                    i += 1;
                }
                if i + 1 >= bytes.len() {
                    return Err("Comentário de workspace não terminado".into());
                }
                bytes[i] = b' ';
                bytes[i + 1] = b' ';
                i += 2;
                continue;
            }
            _ => (),
        }
        i += 1;
    }
    i = 0;
    string = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if string => {
                i += 2;
                continue;
            }
            b'"' => string = !string,
            b',' if !string
                && bytes[i + 1..]
                    .iter()
                    .find(|b| !b.is_ascii_whitespace())
                    .is_some_and(|b| matches!(b, b'}' | b']')) =>
            {
                bytes[i] = b' ';
            }
            _ => (),
        }
        i += 1;
    }
    Ok(bytes)
}

impl super::AppService {
    /// A real declared directory, distinct from the workspace file identity.
    pub fn workspace_default_root(&self) -> Option<&Path> {
        if self.workspace_file.is_some() {
            self.workspace_folders
                .iter()
                .find(|f| f.available)
                .map(|f| f.path.as_path())
        } else {
            self.workspace.as_deref()
        }
    }
    /// Git can own an ancestor of a declared folder. The Git worker must still
    /// verify that this is that folder's actual repository before binding it.
    pub fn git_workspace_for(&self, repository: &Path) -> Option<&Path> {
        if self.workspace_file.is_some() {
            self.workspace_folders
                .iter()
                .find(|f| {
                    f.available
                        && (repository.starts_with(&f.path) || f.path.starts_with(repository))
                })
                .map(|f| f.path.as_path())
        } else {
            self.workspace.as_deref()
        }
    }
    pub fn workspace_path(&self) -> Option<&Path> {
        self.workspace_file.as_deref().or(self.workspace.as_deref())
    }
    pub fn workspace_label(&self) -> String {
        self.workspace_path()
            .map(|p| {
                if self.workspace_file.is_some() {
                    p.file_stem()
                } else {
                    p.file_name()
                }
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
            })
            .unwrap_or_else(|| "Codisseu".into())
    }
    pub fn workspace_root_for(&self, path: &Path) -> Option<&Path> {
        if self.workspace_file.is_some() {
            self.workspace_folders
                .iter()
                .find(|f| f.available && path.starts_with(&f.path))
                .map(|f| f.path.as_path())
        } else {
            self.workspace.as_deref().filter(|r| path.starts_with(r))
        }
    }
    pub fn workspace_roots(&self) -> Vec<PathBuf> {
        if self.workspace_file.is_some() {
            self.workspace_folders
                .iter()
                .filter(|f| f.available)
                .map(|f| f.path.clone())
                .collect()
        } else {
            self.workspace.iter().cloned().collect()
        }
    }
    pub fn workspace_folder_name(&self, path: &Path) -> Option<&str> {
        self.workspace_folders
            .iter()
            .find(|f| f.path == path)
            .map(|f| f.name.as_str())
    }
}
