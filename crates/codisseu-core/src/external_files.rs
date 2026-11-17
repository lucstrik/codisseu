//! Bounded, read-only filesystem reconciliation. All filesystem APIs run here
//! on a worker; the GUI accepts only a matching document version and baseline.
use super::*;
use notify::{EventKind, RecursiveMode, Watcher};
use std::{
    collections::{BTreeSet, hash_map::DefaultHasher},
    fs,
    hash::{Hash, Hasher},
    path::Path,
    time::{Duration, Instant, SystemTime},
};

const INTERVAL: Duration = Duration::from_millis(250);
const READ_BUDGET: u64 = 32 * 1024 * 1024;
struct WatchedDocument {
    id: DocumentId,
    version: u64,
    path: PathBuf,
    baseline: Arc<str>,
    encoding: FileEncoding,
}
struct Request {
    read_revision: u64,
    encoding_profile: PathBuf,
    root: Option<PathBuf>,
    tree: u64,
    directories: Vec<PathBuf>,
    loaded_directories: Vec<PathBuf>,
    files: Vec<WatchedDocument>,
}
struct Loaded {
    document: WatchedDocument,
    result: Result<Option<OpenedFile>, String>,
}
struct Batch {
    read_revision: u64,
    root: Option<PathBuf>,
    tree: u64,
    files: Vec<Loaded>,
    workspace: Option<Result<Workspace, String>>,
    warning: Option<String>,
}
#[derive(Clone)]
struct ChangedSnapshot {
    change: ExternalDocumentChange,
    text: Arc<str>,
    baseline: Arc<str>,
    encoding: FileEncoding,
}
pub(super) struct Service {
    sender: Option<SyncSender<Request>>,
    receiver: Receiver<Batch>,
    requested: Option<Instant>,
    pending: bool,
    subscribed: bool,
    failure: Option<String>,
    conflicts: HashMap<DocumentId, String>,
    changes: HashMap<DocumentId, ChangedSnapshot>,
    next_revision: u64,
    read_revision: u64,
}
impl Service {
    pub fn new() -> Self {
        let (sender, input) = mpsc::sync_channel(1);
        let (output, receiver) = mpsc::sync_channel(1);
        let (sender, failure) = match std::thread::Builder::new()
            .name("codisseu-file-watch".into())
            .spawn(move || run(input, output))
        {
            Ok(_) => (Some(sender), None),
            Err(error) => (
                None,
                Some(format!(
                    "Não foi possível iniciar atualização de arquivos: {error}"
                )),
            ),
        };
        Self {
            sender,
            receiver,
            requested: None,
            pending: false,
            subscribed: false,
            failure,
            conflicts: HashMap::new(),
            changes: HashMap::new(),
            next_revision: 1,
            read_revision: 0,
        }
    }
    fn record_change(&mut self, doc: &Document, file: &OpenedFile) {
        if self.changes.get(&doc.id).is_some_and(|old| {
            old.change.path == file.path
                && old.text.as_ref() == file.text
                && old.encoding == file.encoding
                && old.baseline == doc.saved_text
        }) {
            return;
        }
        let revision = self.next_revision;
        self.next_revision += 1;
        self.changes.insert(
            doc.id,
            ChangedSnapshot {
                change: ExternalDocumentChange {
                    revision,
                    path: file.path.clone(),
                },
                text: Arc::from(file.text.as_str()),
                baseline: doc.saved_text.clone(),
                encoding: file.encoding,
            },
        );
    }
}
fn signature(
    root: &Option<PathBuf>,
    entries: &[TreeEntry],
    loaded: &BTreeSet<PathBuf>,
    encodings: &BTreeMap<PathBuf, FileEncoding>,
) -> u64 {
    let mut hash = DefaultHasher::new();
    root.hash(&mut hash);
    loaded.hash(&mut hash);
    for (root, encoding) in encodings {
        root.hash(&mut hash);
        encoding.label().hash(&mut hash);
    }
    for entry in entries {
        entry.path.hash(&mut hash);
        entry.is_dir.hash(&mut hash);
        entry.depth.hash(&mut hash);
    }
    hash.finish()
}
impl AppService {
    pub(super) fn invalidate_external_file_reads(&mut self) {
        self.external_files.read_revision = self.external_files.read_revision.wrapping_add(1);
    }
    pub fn document_external_change(&self, id: DocumentId) -> Option<&ExternalDocumentChange> {
        self.external_files
            .changes
            .get(&id)
            .map(|change| &change.change)
    }
    pub fn document_save_pending(&self, id: DocumentId) -> bool {
        self.pending
            .values()
            .any(|pending| *pending == Pending::Save(id))
    }
    pub(super) fn clear_external_document_change(&mut self, id: DocumentId) {
        self.external_files.changes.remove(&id);
        if let Some(previous) = self.external_files.conflicts.remove(&id)
            && self.last_error.as_ref() == Some(&previous)
        {
            self.last_error = None;
        }
    }
    pub(super) fn resolve_external_document(
        &mut self,
        id: DocumentId,
        expected_version: u64,
        external_revision: u64,
        overwrite: bool,
    ) -> Result<(), String> {
        if self.document_save_pending(id) {
            return Err("Aguarde a gravação deste documento".into());
        }
        let change = self
            .external_files
            .changes
            .get(&id)
            .filter(|change| change.change.revision == external_revision)
            .cloned()
            .ok_or("A alteração externa mudou; confira a versão atual antes de decidir")?;
        let doc = self.document_mut(id)?;
        if doc.version != expected_version
            || doc.path != change.change.path
            || doc.saved_text != change.baseline
        {
            return Err("O documento mudou; confira suas alterações antes de decidir".into());
        }
        let work = if overwrite {
            if doc.encoding != change.encoding {
                return Err(
                    "A codificação externa mudou; recarregue o arquivo antes de salvar".into(),
                );
            }
            Work::Save {
                encoding: doc.encoding,
                id,
                path: doc.path.clone(),
                text: doc.snapshot(),
                expected: change.text,
                baseline: doc.saved_text.clone(),
                identity: doc.buffer.content_identity(),
            }
        } else {
            Work::ReloadDocument {
                id,
                version: expected_version,
                external_revision,
                path: doc.path.clone(),
                encoding: change.encoding,
                baseline: doc.saved_text.clone(),
                observed: change.text,
            }
        };
        self.invalidate_external_file_reads();
        self.enqueue(work, Pending::Save(id))
    }
    pub(super) fn accept_save_conflict(
        &mut self,
        id: DocumentId,
        baseline: Arc<str>,
        file: OpenedFile,
        error: String,
    ) -> Result<(), String> {
        let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == id) else {
            return Ok(());
        };
        if doc.path != file.path || doc.saved_text != baseline {
            return Err(error);
        }
        if doc.buffer.matches(&doc.saved_text) || doc.text() == file.text {
            accept_disk_snapshot(doc, file)?;
            self.clear_external_document_change(id);
            self.auto_save.saved(id);
            self.status = "Arquivo atualizado do disco".into();
            return Ok(());
        }
        self.external_files.record_change(doc, &file);
        self.external_files.conflicts.insert(id, error.clone());
        Err(error)
    }
    pub(super) fn accept_external_reload(
        &mut self,
        id: DocumentId,
        version: u64,
        revision: u64,
        baseline: Arc<str>,
        file: OpenedFile,
    ) -> Result<(), String> {
        if !self
            .external_files
            .changes
            .get(&id)
            .is_some_and(|change| change.change.revision == revision)
        {
            return Err("A alteração externa mudou; conteúdo aberto preservado".into());
        }
        let doc = self.document_mut(id)?;
        if doc.version != version || doc.path != file.path || doc.saved_text != baseline {
            return Err(
                "O documento foi editado durante a leitura; conteúdo aberto preservado".into(),
            );
        }
        accept_disk_snapshot(doc, file)?;
        self.clear_external_document_change(id);
        self.auto_save.saved(id);
        self.status = "Arquivo recarregado do disco".into();
        Ok(())
    }
    pub fn external_files_pending(&self) -> bool {
        self.external_files.pending
    }
    pub(super) fn poll_external_files(&mut self) -> bool {
        if self.encoding_pending() {
            return false;
        }
        let mut changed = false;
        self.external_files
            .conflicts
            .retain(|id, _| self.documents.iter().any(|doc| doc.id == *id));
        self.external_files
            .changes
            .retain(|id, _| self.documents.iter().any(|doc| doc.id == *id));
        if (self.workspace.is_some() || !self.documents.is_empty())
            && let Some(error) = self.external_files.failure.take()
        {
            self.last_error = Some(error);
            changed = true;
        }
        if let Ok(batch) = self.external_files.receiver.try_recv() {
            self.external_files.pending = false;
            if self.workspace_path() == batch.root.as_deref() {
                if let Some(warning) = batch.warning {
                    self.last_error = Some(warning);
                    changed = true;
                }
                if batch.tree
                    == signature(
                        &self.workspace,
                        &self.entries,
                        &self.project_tree.loaded,
                        &self.project_encodings,
                    )
                    && !self.project_files.pending
                    && !self.rename_resources_busy()
                    && let Some(workspace) = batch.workspace
                {
                    match workspace {
                        Ok(workspace) if Some(&workspace.path) == self.workspace.as_ref() => {
                            if self.entries != workspace.entries
                                || self.tree_truncated != workspace.truncated
                                || self.workspace_folders != workspace.folders
                            {
                                self.accept_tree(workspace);
                                changed = true;
                            }
                        }
                        Err(error) => {
                            self.last_error = Some(error);
                            changed = true;
                        }
                        _ => {}
                    }
                }
                for loaded in batch.files {
                    if batch.read_revision != self.external_files.read_revision {
                        break;
                    }
                    let captured = loaded.document;
                    // A read captured before a Save cannot replace its baseline
                    // while the write is waiting for acknowledgement.
                    if self.document_save_pending(captured.id) {
                        continue;
                    }
                    let Some(doc) = self.documents.iter_mut().find(|doc| doc.id == captured.id)
                    else {
                        continue;
                    };
                    if doc.path != captured.path
                        || doc.version != captured.version
                        || doc.encoding != captured.encoding
                        || doc.saved_text != captured.baseline
                    {
                        continue;
                    }
                    match loaded.result {
                        Ok(Some(file)) if file.path == doc.path => {
                            // Local typing alone is not an external change. The
                            // disk must first differ from the last saved base.
                            if file.text == doc.saved_text.as_ref() {
                                self.external_files.changes.remove(&doc.id);
                                if doc.disk_missing {
                                    doc.disk_missing = false;
                                    changed = true;
                                }
                                if let Some(previous) =
                                    self.external_files.conflicts.remove(&doc.id)
                                    && self.last_error.as_ref() == Some(&previous)
                                {
                                    self.last_error = None;
                                    changed = true;
                                }
                                continue;
                            }
                            if doc.text() != file.text {
                                if !doc.buffer.matches(&doc.saved_text) {
                                    self.external_files.record_change(doc, &file);
                                    let message = format!(
                                        "{} mudou no disco; suas alterações não salvas foram preservadas",
                                        doc.path.display()
                                    );
                                    self.external_files
                                        .conflicts
                                        .insert(doc.id, message.clone());
                                    self.last_error = Some(message);
                                    changed = true;
                                    continue;
                                }
                                match doc.buffer.replace(file.text.clone()) {
                                    Ok(_) => {
                                        doc.encoding = file.encoding;
                                        doc.version += 1;
                                        doc.saved_text = Arc::from(file.text);
                                        doc.saved_identity = Some(doc.buffer.content_identity());
                                        doc.dirty_cache.set(None);
                                        changed = true;
                                    }
                                    Err(error) => {
                                        self.last_error = Some(error);
                                        changed = true;
                                        continue;
                                    }
                                }
                            } else if doc.saved_text.as_ref() != file.text {
                                doc.saved_text = Arc::from(file.text);
                                doc.saved_identity = Some(doc.buffer.content_identity());
                                doc.dirty_cache.set(None);
                                changed = true;
                            }
                            if doc.disk_missing {
                                doc.disk_missing = false;
                                changed = true;
                            }
                            self.external_files.changes.remove(&doc.id);
                            self.auto_save.saved(doc.id);
                            if let Some(previous) = self.external_files.conflicts.remove(&doc.id)
                                && self.last_error.as_ref() == Some(&previous)
                            {
                                self.last_error = None;
                                changed = true;
                            }
                        }
                        Ok(None) => {
                            self.external_files.changes.remove(&doc.id);
                            doc.disk_missing = true;
                            let message = format!(
                                "{} foi removido no disco; conteúdo aberto preservado",
                                doc.path.display()
                            );
                            self.external_files
                                .conflicts
                                .insert(doc.id, message.clone());
                            self.last_error = Some(message);
                            changed = true;
                        }
                        Ok(Some(_)) => {
                            self.last_error = Some(
                                "Caminho externo mudou de identidade; buffer preservado".into(),
                            );
                            changed = true;
                        }
                        Err(error) => {
                            self.external_files.changes.remove(&doc.id);
                            self.external_files.conflicts.insert(doc.id, error.clone());
                            self.last_error = Some(error);
                            changed = true;
                        }
                    }
                }
            }
        }
        if !self.external_files.pending
            && self
                .external_files
                .requested
                .is_none_or(|at| at.elapsed() >= INTERVAL)
            && !self.project_files.pending
            && !self.rename_resources_busy()
            && !self
                .pending
                .values()
                .any(|p| matches!(p, Pending::Save(_) | Pending::Workspace | Pending::Restore))
        {
            let mut directories = BTreeSet::new();
            if let Some(root) = &self.workspace {
                directories.insert(root.clone());
                if let Some(parent) = root.parent() {
                    directories.insert(parent.to_path_buf());
                }
            }
            directories.extend(self.project_tree.loaded.iter().cloned());
            for doc in self.documents.iter().filter(|d| !d.is_untitled()) {
                if let Some(parent) = doc.path.parent() {
                    directories.insert(parent.to_path_buf());
                }
            }
            let subscribed = !directories.is_empty();
            if !subscribed && !self.external_files.subscribed {
                return changed;
            }
            let request = Request {
                read_revision: self.external_files.read_revision,
                encoding_profile: self.session_path.with_extension("encodings.json"),
                root: self.workspace_path().map(PathBuf::from),
                tree: signature(
                    &self.workspace,
                    &self.entries,
                    &self.project_tree.loaded,
                    &self.project_encodings,
                ),
                directories: directories.into_iter().collect(),
                loaded_directories: self.project_tree.loaded.iter().cloned().collect(),
                files: self
                    .documents
                    .iter()
                    .filter(|d| !d.is_untitled())
                    .map(|doc| WatchedDocument {
                        id: doc.id,
                        version: doc.version,
                        path: doc.path.clone(),
                        baseline: doc.saved_text.clone(),
                        encoding: doc.encoding,
                    })
                    .collect(),
            };
            if self
                .external_files
                .sender
                .as_ref()
                .is_some_and(|sender| sender.try_send(request).is_ok())
            {
                self.external_files.pending = true;
                self.external_files.subscribed = subscribed;
                self.external_files.requested = Some(Instant::now());
            }
        }
        changed
    }
}

fn accept_disk_snapshot(doc: &mut Document, file: OpenedFile) -> Result<(), String> {
    if doc.text() != file.text {
        doc.buffer.replace(file.text.clone())?;
        doc.version += 1;
    }
    doc.encoding = file.encoding;
    doc.saved_text = Arc::from(file.text);
    doc.saved_identity = Some(doc.buffer.content_identity());
    doc.dirty_cache.set(None);
    doc.disk_missing = false;
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
    identity: (u64, u64, i64, i64),
    regular: bool,
}
fn stamp(path: &Path) -> Result<Option<Stamp>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
        Ok(meta) => {
            #[cfg(unix)]
            let identity = {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec())
            };
            #[cfg(not(unix))]
            let identity = (0, 0, 0, 0);
            Ok(Some(Stamp {
                size: meta.len(),
                modified: meta.modified().ok(),
                identity,
                regular: meta.is_file() && !meta.file_type().is_symlink(),
            }))
        }
    }
}
pub(super) fn stable_document(path: &Path, encoding: FileEncoding) -> Result<OpenedFile, String> {
    let before = stamp(path)?;
    if !before.as_ref().is_some_and(|meta| meta.regular) {
        return Err("O destino não é um arquivo regular; conteúdo aberto preservado".into());
    }
    let file = worker::external_document(path, encoding)?;
    if stamp(path)? != before {
        return Err(
            "O arquivo mudou durante a leitura; tente novamente. Conteúdo aberto preservado".into(),
        );
    }
    Ok(file)
}
fn run(input: Receiver<Request>, output: SyncSender<Batch>) {
    let structure = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let notify_structure = structure.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.as_ref().map_or(true, |event| {
            matches!(
                event.kind,
                EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(notify::event::ModifyKind::Name(_))
                    | EventKind::Any
                    | EventKind::Other
            )
        }) {
            notify_structure.store(true, Ordering::Release);
        }
    });
    let mut watched = BTreeSet::new();
    let mut cache: HashMap<PathBuf, (Option<Stamp>, u64, Arc<str>)> = HashMap::new();
    let mut root = None;
    let mut cursor = 0usize;
    let mut reported = false;
    let mut previous_tree = None;
    let mut previous_read_revision = None;
    while let Ok(request) = input.recv() {
        if previous_read_revision != Some(request.read_revision) {
            cache.clear();
            previous_read_revision = Some(request.read_revision);
        }
        let mut warning = None;
        // A foreground operation may have replaced the tree while the last
        // watcher result was in flight. Reconcile again even if that event was
        // already coalesced into the rejected result.
        if previous_tree != Some(request.tree) {
            previous_tree = Some(request.tree);
            structure.store(true, Ordering::Release);
        }
        if root != request.root {
            root = request.root.clone();
            cache.clear();
            structure.store(true, Ordering::Release);
        }
        let desired: BTreeSet<_> = request.directories.into_iter().collect();
        match &mut watcher {
            Ok(watcher) => {
                for path in watched.difference(&desired) {
                    let _ = watcher.unwatch(path);
                }
                let mut registered = watched
                    .intersection(&desired)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                for path in desired.difference(&watched) {
                    match watcher.watch(path, RecursiveMode::NonRecursive) {
                        Ok(()) => {
                            registered.insert(path.clone());
                            structure.store(true, Ordering::Release);
                        }
                        Err(error) => {
                            if !reported {
                                warning = Some(format!(
                                    "Observação de arquivos limitada: {error}. Arquivos abertos continuam verificados periodicamente."
                                ));
                                reported = true;
                            }
                        }
                    }
                }
                watched = registered;
            }
            Err(error) => {
                if !reported {
                    warning = Some(format!(
                        "Notificações de arquivos indisponíveis: {error}. Arquivos abertos continuam verificados periodicamente."
                    ));
                    reported = true;
                }
            }
        }
        cache.retain(|path, _| request.files.iter().any(|doc| &doc.path == path));
        let mut files = Vec::new();
        let mut budget = READ_BUDGET;
        let count = request.files.len();
        let mut documents: Vec<_> = request.files.into_iter().map(Some).collect();
        for offset in 0..count {
            let index = (cursor + offset) % count;
            let document = documents[index].take().unwrap();
            let current = stamp(&document.path);
            if let Ok(ref current) = current {
                if cache
                    .get(&document.path)
                    .is_some_and(|(old, version, base)| {
                        old == current && *version == document.version && base == &document.baseline
                    })
                {
                    continue;
                }
                let size = current.as_ref().map_or(0, |meta| {
                    meta.size
                        .min(codisseu_editor::MAX_EDITOR_DOCUMENT_BYTES as u64)
                });
                if size > budget {
                    continue;
                }
                budget -= size;
            }
            let result = match current {
                Ok(current) => {
                    let result = match &current {
                        None => Ok(None),
                        Some(meta) if !meta.regular => Err(format!(
                            "{} não é mais arquivo regular; buffer preservado",
                            document.path.display()
                        )),
                        Some(_)
                            if fs::canonicalize(&document.path).ok().as_ref()
                                != Some(&document.path) =>
                        {
                            Err(
                                "Caminho externo mudou por link simbólico; buffer preservado"
                                    .into(),
                            )
                        }
                        Some(_) => {
                            worker::external_document(&document.path, document.encoding).map(Some)
                        }
                    };
                    // A writer can replace/truncate while we read. Retry rather
                    // than publishing a torn snapshot or caching it as current.
                    if stamp(&document.path).ok().as_ref() != Some(&current) {
                        continue;
                    }
                    cache.insert(
                        document.path.clone(),
                        (current, document.version, document.baseline.clone()),
                    );
                    result
                }
                Err(error) => Err(error),
            };
            files.push(Loaded { document, result });
        }
        cursor = if count == 0 { 0 } else { (cursor + 1) % count };
        let workspace = if structure.swap(false, Ordering::AcqRel) {
            request.root.as_ref().map(|path| {
                worker::external_workspace(
                    path,
                    &request.loaded_directories,
                    Some(&request.encoding_profile),
                )
            })
        } else {
            None
        };
        if output
            .send(Batch {
                read_revision: request.read_revision,
                root: request.root,
                tree: request.tree,
                files,
                workspace,
                warning,
            })
            .is_err()
        {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn until(app: &mut AppService, condition: impl Fn(&AppService) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            app.poll();
            if condition(app) {
                return;
            }
            assert!(Instant::now() < deadline, "{:?}", app.last_error);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn setup() -> (tempfile::TempDir, AppService, PathBuf, DocumentId) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir(&root).unwrap();
        let path = root.join("service.js");
        fs::write(&path, "const início = 1;\n").unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        until(&mut app, |s| !s.is_busy());
        app.dispatch(Command::OpenFile(path.clone())).unwrap();
        until(&mut app, |s| !s.is_busy());
        let id = app.active.unwrap();
        (dir, app, path, id)
    }
    #[test]
    fn live_external_stages_atomic_replacement_and_tree_creation_keep_same_tab() {
        let (_dir, mut app, path, id) = setup();
        let original_version = app.documents[0].version;
        app.dispatch(Command::SetSelection {
            id,
            expected_version: original_version,
            selection: Selection { anchor: 2, head: 4 },
        })
        .unwrap();
        fs::write(&path, "const início = 1;\nfunction etapaUm() {}\n").unwrap();
        until(&mut app, |s| s.documents[0].text().contains("etapaUm"));
        assert_eq!(app.active, Some(id));
        assert!(!app.documents[0].is_dirty());
        assert_eq!(
            app.documents[0].selection(),
            Selection { anchor: 2, head: 4 }
        );
        let replacement = path.with_extension("tmp");
        fs::write(&replacement, "const início = 1;\nfunction etapaDois() {}\n").unwrap();
        fs::rename(replacement, &path).unwrap();
        until(&mut app, |s| s.documents[0].text().contains("etapaDois"));
        let folder = path.parent().unwrap().join("generated");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("new.ts"), "export const added = 1;\n").unwrap();
        until(&mut app, |s| s.entries.iter().any(|e| e.path == folder));
        app.dispatch(Command::LoadDirectory(folder.clone()))
            .unwrap();
        until(&mut app, |s| {
            s.entries.iter().any(|e| e.path == folder.join("new.ts"))
        });
        assert!(app.documents[0].version >= original_version + 2);
        assert_eq!(app.document_views()[0].tabs[0], id);
        app.dispatch(Command::Undo { id }).unwrap();
        assert!(app.documents[0].is_dirty());
    }
    #[test]
    fn dirty_buffer_survives_external_change_and_save_retains_conflict_guard() {
        let (_dir, mut app, path, id) = setup();
        app.dispatch(Command::Edit {
            id,
            expected_version: app.documents[0].version,
            text: "const local = 2;\n".into(),
        })
        .unwrap();
        fs::write(&path, "const externo = 3;\n").unwrap();
        until(&mut app, |s| {
            s.last_error
                .as_ref()
                .is_some_and(|e| e.contains("não salvas"))
        });
        assert_eq!(app.documents[0].text(), "const local = 2;\n");
        app.dispatch(Command::Save { id }).unwrap();
        until(&mut app, |s| !s.is_busy());
        assert!(app.last_error.as_ref().unwrap().contains("Conflito"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "const externo = 3;\n");
        fs::write(&path, "const local = 2;\n").unwrap();
        until(&mut app, |s| !s.documents[0].is_dirty());
    }
    #[test]
    fn local_typing_without_disk_changes_never_reports_external_conflict() {
        let (_dir, mut app, _path, id) = setup();
        app.dispatch(Command::Edit {
            id,
            expected_version: app.documents[0].version,
            text: "// somente local\n".into(),
        })
        .unwrap();
        for _ in 0..2 {
            until(&mut app, |s| s.external_files.pending);
            until(&mut app, |s| !s.external_files.pending);
        }
        assert!(app.last_error.is_none(), "{:?}", app.last_error);
        assert!(app.documents[0].is_dirty());
        assert_eq!(app.documents[0].text(), "// somente local\n");
    }
    #[test]
    fn save_before_watcher_reconciles_clean_or_already_matching_disk_without_false_dirty() {
        for matching_local in [false, true] {
            let (_dir, mut app, path, id) = setup();
            let external = "// atualizado por outro editor: ação 😀\r\n";
            if matching_local {
                app.dispatch(Command::Edit {
                    id,
                    expected_version: app.documents[0].version,
                    text: external.into(),
                })
                .unwrap();
            }
            fs::write(&path, external).unwrap();
            // Deliberately do not poll the watcher between the external write
            // and Save: closing the app can reach precisely this ordering.
            app.dispatch(Command::Save { id }).unwrap();
            until(&mut app, |s| !s.document_save_pending(id));
            assert_eq!(app.documents[0].text(), external);
            assert!(!app.documents[0].is_dirty());
            assert_eq!(fs::read_to_string(&path).unwrap(), external);
            assert!(app.document_external_change(id).is_none());
            assert!(app.last_error.is_none(), "{:?}", app.last_error);
            app.dispatch(Command::Close { id, discard: false }).unwrap();
        }
    }
    fn conflict(app: &mut AppService, path: &Path, id: DocumentId) -> u64 {
        app.dispatch(Command::Edit {
            id,
            expected_version: app.documents[0].version,
            text: "// alteração local\r\n".into(),
        })
        .unwrap();
        fs::write(path, "// alteração externa\r\n").unwrap();
        app.dispatch(Command::Save { id }).unwrap();
        until(app, |s| !s.document_save_pending(id));
        assert!(app.documents[0].is_dirty());
        app.document_external_change(id).unwrap().revision
    }
    #[test]
    fn explicit_overwrite_revalidates_exact_external_snapshot_and_ack_keeps_newer_local_edit() {
        let (_dir, mut app, path, id) = setup();
        let revision = conflict(&mut app, &path, id);
        let version = app.documents[0].version;
        fs::write(&path, "// nova alteração externa\r\n").unwrap();
        app.dispatch(Command::OverwriteDocument {
            id,
            expected_version: version,
            external_revision: revision,
        })
        .unwrap();
        until(&mut app, |s| !s.document_save_pending(id));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "// nova alteração externa\r\n"
        );
        assert_eq!(app.documents[0].text(), "// alteração local\r\n");
        let current = app.document_external_change(id).unwrap().revision;
        assert_ne!(current, revision);
        assert!(
            app.dispatch(Command::OverwriteDocument {
                id,
                expected_version: version,
                external_revision: revision
            })
            .is_err()
        );
        app.dispatch(Command::OverwriteDocument {
            id,
            expected_version: version,
            external_revision: current,
        })
        .unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "// local posterior ao Save\r\n".into(),
        })
        .unwrap();
        until(&mut app, |s| !s.document_save_pending(id));
        assert_eq!(fs::read_to_string(&path).unwrap(), "// alteração local\r\n");
        assert_eq!(app.documents[0].text(), "// local posterior ao Save\r\n");
        assert!(app.documents[0].is_dirty());
        assert!(app.document_external_change(id).is_none());
    }
    #[test]
    fn watcher_read_from_before_save_cannot_clear_the_new_conflict_ack() {
        let (_dir, mut app, path, id) = setup();
        let doc = &app.documents[0];
        // A watcher read can finish before Save starts but arrive only after
        // its ACK. Document version/base alone cannot identify that old read.
        let old_read_revision = app.external_files.read_revision;
        let old_baseline = doc.saved_text.clone();
        let revision = conflict(&mut app, &path, id);
        let captured = WatchedDocument {
            id,
            version: app.documents[0].version,
            path: path.clone(),
            baseline: old_baseline.clone(),
            encoding: app.documents[0].encoding,
        };
        let (out, receiver) = mpsc::sync_channel(1);
        app.external_files.receiver = receiver;
        app.external_files.pending = true;
        out.send(Batch {
            read_revision: old_read_revision,
            root: app.workspace.clone(),
            tree: 0,
            workspace: None,
            warning: None,
            files: vec![Loaded {
                document: captured,
                result: Ok(Some(OpenedFile {
                    project_root: path.parent().unwrap().to_path_buf(),
                    path: path.clone(),
                    text: old_baseline.to_string(),
                    encoding: FileEncoding::Utf8,
                })),
            }],
        })
        .unwrap();
        app.poll_external_files();
        assert_eq!(app.document_external_change(id).unwrap().revision, revision);
        assert_eq!(app.documents[0].text(), "// alteração local\r\n");
        assert!(app.documents[0].is_dirty());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "// alteração externa\r\n"
        );
    }
    #[test]
    fn explicit_reload_rechecks_disk_and_local_versions_before_discarding_buffer() {
        let (_dir, mut app, path, id) = setup();
        let revision = conflict(&mut app, &path, id);
        let version = app.documents[0].version;
        assert!(
            app.dispatch(Command::ReloadDocument {
                id,
                expected_version: version - 1,
                external_revision: revision
            })
            .is_err()
        );
        fs::write(&path, "// externo novo: 😀\r\n").unwrap();
        app.dispatch(Command::ReloadDocument {
            id,
            expected_version: version,
            external_revision: revision,
        })
        .unwrap();
        until(&mut app, |s| !s.document_save_pending(id));
        assert_eq!(app.documents[0].text(), "// alteração local\r\n");
        let revision = app.document_external_change(id).unwrap().revision;
        app.dispatch(Command::ReloadDocument {
            id,
            expected_version: version,
            external_revision: revision,
        })
        .unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "// digitado durante recarga\n".into(),
        })
        .unwrap();
        until(&mut app, |s| !s.document_save_pending(id));
        assert_eq!(app.documents[0].text(), "// digitado durante recarga\n");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "// externo novo: 😀\r\n"
        );
        let version = app.documents[0].version;
        app.dispatch(Command::ReloadDocument {
            id,
            expected_version: version,
            external_revision: revision,
        })
        .unwrap();
        until(&mut app, |s| !s.document_save_pending(id));
        assert_eq!(app.documents[0].text(), "// externo novo: 😀\r\n");
        assert!(!app.documents[0].is_dirty());
        assert!(app.document_external_change(id).is_none());
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.documents[0].text(), "// digitado durante recarga\n");
        assert!(app.documents[0].is_dirty());
    }
    #[cfg(unix)]
    #[test]
    fn explicit_resolution_never_follows_a_replaced_symlink() {
        for overwrite in [false, true] {
            let (_dir, mut app, path, id) = setup();
            let revision = conflict(&mut app, &path, id);
            let target = path.with_file_name("outside.txt");
            fs::write(&target, "must remain untouched").unwrap();
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            let command = if overwrite {
                Command::OverwriteDocument {
                    id,
                    expected_version: app.documents[0].version,
                    external_revision: revision,
                }
            } else {
                Command::ReloadDocument {
                    id,
                    expected_version: app.documents[0].version,
                    external_revision: revision,
                }
            };
            app.dispatch(command).unwrap();
            until(&mut app, |s| !s.document_save_pending(id));
            assert!(app.last_error.is_some());
            assert_eq!(app.documents[0].text(), "// alteração local\r\n");
            assert!(app.documents[0].is_dirty());
            assert!(path.is_symlink());
            assert_eq!(fs::read_to_string(target).unwrap(), "must remain untouched");
        }
    }
    #[test]
    fn inactive_file_same_size_update_and_oversize_input_are_bounded() {
        let (_dir, mut app, path, id) = setup();
        let other = path.with_file_name("app.js");
        fs::write(&other, "const active = 1;\n").unwrap();
        app.dispatch(Command::OpenFile(other)).unwrap();
        until(&mut app, |s| !s.is_busy());
        let active = app.active;
        let previous = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "const início = 9;\n").unwrap();
        // The file size and mtime can be restored by an external tool. Unix
        // identity includes ctime, so this is still an observable disk change.
        #[cfg(unix)]
        fs::File::open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(previous))
            .unwrap();
        #[cfg(not(unix))]
        let _ = previous;
        until(&mut app, |s| {
            s.documents
                .iter()
                .find(|d| d.id == id)
                .unwrap()
                .text()
                .contains("= 9")
        });
        assert_eq!(app.active, active);
        fs::write(
            &path,
            vec![b'x'; codisseu_editor::MAX_EDITOR_DOCUMENT_BYTES + 1],
        )
        .unwrap();
        until(&mut app, |s| {
            s.last_error.as_ref().is_some_and(|e| e.contains("limite"))
        });
        assert!(
            app.documents
                .iter()
                .find(|d| d.id == id)
                .unwrap()
                .text()
                .contains("= 9")
        );
    }
    #[test]
    fn missing_binary_and_symlink_preserve_buffer_and_recreation_recovers() {
        let (_dir, mut app, path, _id) = setup();
        fs::remove_file(&path).unwrap();
        until(&mut app, |s| s.documents[0].disk_missing);
        assert!(app.documents[0].text().contains("início"));
        fs::write(&path, "const recriado = 1;\n").unwrap();
        until(&mut app, |s| s.documents[0].text().contains("recriado"));
        assert!(!app.documents[0].is_dirty());
        fs::write(&path, b"binary\0data").unwrap();
        until(&mut app, |s| {
            s.last_error.as_ref().is_some_and(|e| e.contains("binário"))
        });
        assert!(app.documents[0].text().contains("recriado"));
        #[cfg(unix)]
        {
            let target = path.with_extension("outside");
            fs::write(&target, "must not load").unwrap();
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(target, &path).unwrap();
            until(&mut app, |s| {
                s.last_error.as_ref().is_some_and(|e| e.contains("regular"))
            });
            assert!(app.documents[0].text().contains("recriado"));
        }
    }
    #[test]
    fn stale_version_baseline_path_and_workspace_are_never_applied() {
        let (_dir, mut app, path, id) = setup();
        for kind in [0, 1, 2, 3] {
            let doc = &app.documents[0];
            let captured = WatchedDocument {
                encoding: doc.encoding,
                id,
                version: if kind == 0 {
                    doc.version + 1
                } else {
                    doc.version
                },
                path: if kind == 1 {
                    path.with_extension("other")
                } else {
                    path.clone()
                },
                baseline: if kind == 2 {
                    Arc::from("wrong")
                } else {
                    doc.saved_text.clone()
                },
            };
            let (out, receiver) = mpsc::sync_channel(1);
            app.external_files.receiver = receiver;
            app.external_files.pending = true;
            out.send(Batch {
                read_revision: app.external_files.read_revision,
                root: if kind == 3 {
                    None
                } else {
                    app.workspace.clone()
                },
                tree: 0,
                workspace: None,
                warning: None,
                files: vec![Loaded {
                    document: captured,
                    result: Ok(Some(OpenedFile {
                        project_root: path.parent().unwrap().to_path_buf(),
                        encoding: FileEncoding::Utf8,
                        path: path.clone(),
                        text: "WRONG".into(),
                    })),
                }],
            })
            .unwrap();
            app.poll_external_files();
            assert!(app.documents[0].text().contains("início"));
        }
    }
}
