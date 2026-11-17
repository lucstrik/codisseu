//! Directory-at-a-time Explorer model. Discovery budgets are not visibility filters.
use super::*;
use std::{collections::BTreeSet, path::Path};

#[derive(Default)]
pub(super) struct TreeState {
    pub loaded: BTreeSet<PathBuf>,
    pub pending: HashMap<u64, PathBuf>,
    pub errors: BTreeMap<PathBuf, String>,
    pub revision: u64,
}

/// Replace direct children without discarding cached descendants of surviving folders.
pub(super) fn merge(
    entries: &mut Vec<TreeEntry>,
    root: &Path,
    parent: &Path,
    children: Vec<TreeEntry>,
) {
    let (start, depth) = if parent == root {
        (0, None)
    } else {
        let Some(index) = entries.iter().position(|e| e.path == parent && e.is_dir) else {
            return;
        };
        (index + 1, Some(entries[index].depth))
    };
    let end = entries[start..]
        .iter()
        .position(|e| depth.is_some_and(|d| e.depth <= d))
        .map_or(entries.len(), |n| start + n);
    let old: Vec<_> = entries.drain(start..end).collect();
    let mut descendants = HashMap::new();
    let mut cursor = 0;
    while cursor < old.len() {
        let item = &old[cursor];
        let next = (cursor + 1..old.len())
            .find(|&n| old[n].depth <= item.depth)
            .unwrap_or(old.len());
        if item.is_dir {
            descendants.insert(item.path.clone(), old[cursor + 1..next].to_vec());
        }
        cursor = next;
    }
    let mut subtree = Vec::new();
    for child in children {
        let cached = child
            .is_dir
            .then(|| descendants.remove(&child.path))
            .flatten();
        subtree.push(child);
        if let Some(cached) = cached {
            subtree.extend(cached);
        }
    }
    entries.splice(start..start, subtree);
}

impl AppService {
    /// Epoch for full tree replacement (refresh/watch/filesystem ACK).
    pub fn tree_snapshot_revision(&self) -> u64 {
        self.project_tree.revision
    }
    pub fn tree_directory_loaded(&self, path: &Path) -> bool {
        self.project_tree.loaded.contains(path)
    }
    pub fn tree_directory_error(&self, path: &Path) -> Option<&str> {
        self.project_tree.errors.get(path).map(String::as_str)
    }
    pub fn tree_error(&self) -> Option<&str> {
        self.project_tree.errors.values().next().map(String::as_str)
    }
    pub(super) fn load_tree_directory(&mut self, path: PathBuf) -> Result<(), String> {
        let root = self
            .workspace
            .clone()
            .ok_or("Abra uma pasta antes de expandir diretórios")?;
        if self.workspace_root_for(&path).is_none()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || (path != root && !self.entries.iter().any(|e| e.path == path && e.is_dir))
        {
            return Err("Diretório não pertence à árvore atual".into());
        }
        if self.project_tree.loaded.contains(&path)
            || self.project_tree.pending.values().any(|p| p == &path)
        {
            return Ok(());
        }
        let id = self.next_job;
        self.enqueue(
            Work::Directory {
                root,
                path: path.clone(),
                revision: self.project_tree.revision,
            },
            Pending::Directory,
        )?;
        self.project_tree.errors.remove(&path);
        self.project_tree.pending.insert(id, path);
        Ok(())
    }
    pub(super) fn accept_tree(&mut self, workspace: Workspace) {
        let previous_roots = self.workspace_roots();
        self.workspace_file = workspace.file;
        self.workspace_folders = workspace.folders;
        if previous_roots != self.workspace_roots() {
            self.git_workspace = None;
        }
        self.project_encodings = workspace.project_encodings;
        for d in &self.documents {
            self.project_encodings
                .entry(d.project_root.clone())
                .or_insert(d.encoding);
        }
        self.entries = workspace.entries;
        self.tree_truncated = false;
        self.project_tree.loaded = workspace.loaded;
        self.project_tree.errors = workspace.errors;
        self.project_tree.revision += 1;
    }
}
