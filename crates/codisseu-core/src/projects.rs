//! Project boundaries are resolved on the filesystem worker, never while painting.
use std::{
    fs,
    path::{Path, PathBuf},
};

const MARKERS: [&str; 12] = [
    ".git",
    "composer.json",
    "package.json",
    "tsconfig.json",
    "Cargo.toml",
    "pyproject.toml",
    ".python-version",
    ".php-version",
    ".nvmrc",
    ".node-version",
    "rust-toolchain.toml",
    "rust-toolchain",
];

pub(super) fn resolve_root(workspace: Option<&Path>, path: &Path) -> PathBuf {
    let start = if path.is_dir() {
        path
    } else {
        path.parent().unwrap_or(path)
    };
    let folders = workspace
        .filter(|p| super::workspace_file::is_workspace_file(p))
        .and_then(|p| super::workspace_file::read_folders(p).ok())
        .unwrap_or_default();
    let workspace = folders
        .iter()
        .find(|f| start.starts_with(&f.path))
        .map(|f| f.path.as_path())
        .or(workspace);
    let boundary = workspace.filter(|root| start.starts_with(root));
    let rust = path.extension().is_some_and(|e| e == "rs")
        || path.file_name().is_some_and(|n| n == "Cargo.toml");
    let mut candidate = None;
    for ancestor in start.ancestors() {
        // A dependency's own manifests must not start a new language server.
        if ancestor.file_name().is_some_and(|n| {
            matches!(
                n.to_str(),
                Some("vendor" | "node_modules" | "target" | ".git")
            )
        }) {
            candidate = None;
            continue;
        }
        if rust && cargo_workspace(ancestor) {
            return ancestor.to_path_buf();
        }
        if candidate.is_none()
            && MARKERS.iter().any(|marker| {
                fs::symlink_metadata(ancestor.join(marker)).is_ok_and(|m| {
                    !m.file_type().is_symlink()
                        && (m.is_file() || (*marker == ".git" && m.is_dir()))
                })
            })
        {
            candidate = Some(ancestor.to_path_buf());
        }
        if Some(ancestor) == boundary {
            break;
        }
    }
    candidate
        .or_else(|| boundary.map(PathBuf::from))
        .unwrap_or_else(|| start.to_path_buf())
}

impl super::AppService {
    /// Cached ownership only: no filesystem work on the UI thread.
    pub fn project_root_for_path(&self, path: &Path) -> PathBuf {
        self.documents
            .iter()
            .find(|d| d.path == path)
            .map(|d| d.project_root.clone())
            .or_else(|| {
                self.project_encodings
                    .keys()
                    .filter(|root| path.starts_with(root))
                    .max_by_key(|root| root.components().count())
                    .cloned()
            })
            .or_else(|| self.workspace_root_for(path).map(PathBuf::from))
            .unwrap_or_else(|| path.parent().unwrap_or(path).to_path_buf())
    }
    pub fn project_roots(&self) -> impl Iterator<Item = &PathBuf> {
        self.project_encodings.keys()
    }
    pub fn encoding_project_root(&self) -> Option<&Path> {
        self.active_document()
            .filter(|d| self.workspace_root_for(&d.path).is_some())
            .map(|d| d.project_root.as_path())
            .or_else(|| self.workspace_default_root())
    }
    pub(super) fn set_folder_encoding(
        &mut self,
        root: PathBuf,
        encoding: super::FileEncoding,
    ) -> Result<(), String> {
        self.workspace
            .as_ref()
            .ok_or("Abra uma pasta para escolher a codificação")?;
        if self.workspace_root_for(&root).is_none()
            || root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err("A pasta de encoding deve pertencer ao workspace".into());
        }
        if !self.pending.is_empty()
            || self.search_worker.is_busy()
            || self.git_repositories_busy()
            || self.project_files.pending
            || self.rename_stage.is_some()
        {
            return Err("Aguarde as operações antes de trocar a codificação".into());
        }
        let owns = |d: &&super::Document| {
            d.path.starts_with(&root)
                && (d.project_root == root || !d.project_root.starts_with(&root))
        };
        if self
            .documents
            .iter()
            .filter(owns)
            .any(super::Document::is_dirty)
        {
            return Err("Salve ou feche as alterações deste projeto antes de recarregar com outra codificação".into());
        }
        let files = self
            .documents
            .iter()
            .filter(owns)
            .map(|d| (d.id, d.path.clone()))
            .collect();
        self.cancel_formatting();
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.clear_search();
        self.enqueue(
            super::Work::Encoding {
                root,
                encoding,
                files,
            },
            super::Pending::Encoding,
        )
    }
}

fn cargo_workspace(root: &Path) -> bool {
    use std::io::Read;
    let path = root.join("Cargo.toml");
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return false;
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 1024 * 1024 {
        return false;
    }
    let mut text = String::new();
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    if file
        .take(1024 * 1024 + 1)
        .read_to_string(&mut text)
        .is_err()
        || text.len() > 1024 * 1024
    {
        return false;
    }
    toml::from_str::<toml::Value>(&text)
        .ok()
        .is_some_and(|value| value.get("workspace").is_some_and(toml::Value::is_table))
}
