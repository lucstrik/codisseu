//! Explicit provider fixes over versioned LSP edits. No server commands or implicit saves.
use super::*;
use codisseu_language::{CodeActionItem, RenameFileEdits};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct CodeActionState {
    pub items: Vec<CodeActionItem>,
    pub files: Vec<RenamePreviewFile>,
    pub title: String,
    pub error: Option<String>,
    pub truncated: bool,
    context: Option<Context>,
    worker: Option<Receiver<Result<Vec<RenamePreviewFile>, String>>>,
    python_query: Option<Receiver<Result<Vec<PythonAction>, String>>>,
}
struct PythonAction {
    item: CodeActionItem,
    changes: Vec<RenameFileEdits>,
}
#[derive(PartialEq)]
enum Phase {
    Query,
    Choosing,
    Resolve,
    Preparing,
    Preview,
    Applying,
}
struct Context {
    id: DocumentId,
    path: PathBuf,
    version: u64,
    root: PathBuf,
    language: Language,
    versions: Vec<(DocumentId, u64)>,
    phase: Phase,
    changes: Vec<RenameFileEdits>,
    prepared: Vec<RenamePreviewFile>,
    token: Option<u64>,
    local_changes: std::collections::HashMap<u64, Vec<RenameFileEdits>>,
    cancel: Arc<AtomicBool>,
}
impl CodeActionState {
    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }
    pub fn is_pending(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| !matches!(c.phase, Phase::Choosing | Phase::Preview))
    }
    pub fn has_preview(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Preview)
    }
}
#[derive(Clone)]
struct OpenSnapshot {
    path: PathBuf,
    version: u64,
    text: Arc<str>,
    encoding: FileEncoding,
    read_only: bool,
}
impl AppService {
    fn code_actions_current(&self) -> bool {
        self.code_actions.context.as_ref().is_some_and(|c| {
            self.active == Some(c.id)
                && self.language_project_root().as_ref() == Some(&c.root)
                && self.active_document().is_some_and(|d| {
                    d.version == c.version && d.path == c.path && !d.is_read_only()
                })
                && c.versions
                    == self
                        .documents
                        .iter()
                        .map(|d| (d.id, d.version))
                        .collect::<Vec<_>>()
        })
    }
    pub(super) fn cancel_code_actions(&mut self) {
        if let Some(c) = self.code_actions.context.take() {
            c.cancel.store(true, Ordering::Release);
            let _ = self.language.dispatch(LanguageCommand::CancelCodeActions);
        }
        self.code_actions.worker = None;
        self.code_actions.python_query = None;
        self.code_actions.items.clear();
        self.code_actions.files.clear();
        self.code_actions.title.clear();
        self.code_actions.truncated = false;
    }
    pub(super) fn code_actions_failed(&mut self, error: String) {
        self.cancel_code_actions();
        self.code_actions.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }
    pub(super) fn request_code_actions(
        &mut self,
        id: DocumentId,
        version: u64,
        caret: usize,
    ) -> Result<(), String> {
        let doc = self
            .active_document()
            .filter(|d| d.id == id && d.version == version && !d.is_read_only())
            .ok_or("Correções: documento ativo mudou ou é somente leitura")?;
        doc.ensure_language_size()?;
        let language =
            Language::for_path(&doc.path).ok_or("Correções indisponíveis para esta linguagem")?;
        let python_source = (language == Language::Python).then(|| doc.text().to_owned());
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider da linguagem antes de consultar correções".into());
        }
        let (line, character) =
            scalar_lsp_position(doc.text(), caret).ok_or("Cursor inválido para correções")?;
        let path = doc.path.clone();
        let root = self.language_project_root().ok_or("Abra um projeto")?;
        let previous_error = self.code_actions.error.take();
        if previous_error.is_some() && self.last_error == previous_error {
            self.last_error = None;
        }
        self.cancel_code_actions();
        self.cancel_completion();
        self.cancel_navigation();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.sync_documents();
        if self.language_versions.get(&path) != Some(&version) {
            return Err("Aguarde a sincronização antes de consultar correções".into());
        }
        if language != Language::Python {
            self.language.dispatch(LanguageCommand::CodeActions {
                path: path.clone(),
                version,
                range: RenameRange {
                    start_line: line,
                    start_character: character,
                    end_line: line,
                    end_character: character,
                },
            })?;
        }
        self.code_actions.context = Some(Context {
            id,
            path,
            version,
            root,
            language,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Query,
            changes: vec![],
            prepared: vec![],
            token: None,
            local_changes: std::collections::HashMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        });
        if let Some(source) = python_source {
            let c = self.code_actions.context.as_ref().unwrap();
            let path = c.path.clone();
            let root = c.root.clone();
            let cancel = c.cancel.clone();
            let (tx, rx) = mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("codisseu-python-actions".into())
                .spawn(move || {
                    let result = (|| {
                        let mut actions = Vec::new();
                        for (index, operation, title) in [
                            (
                                0,
                                crate::python_formatting::Operation::SafeFixes,
                                "Ruff · Aplicar correções seguras no arquivo",
                            ),
                            (
                                1,
                                crate::python_formatting::Operation::Imports,
                                "Ruff · Organizar imports",
                            ),
                        ] {
                            if cancel.load(Ordering::Acquire) {
                                return Err("Consulta Ruff cancelada".into());
                            }
                            let after = crate::python_formatting::transform(
                                &source, &path, &root, operation,
                            )?;
                            if after == source {
                                continue;
                            }
                            actions.push(PythonAction {
                                item: CodeActionItem {
                                    token: (1u64 << 63) + index,
                                    title: title.into(),
                                    kind: Some("quickfix".into()),
                                    preferred: false,
                                    disabled_reason: None,
                                },
                                changes: vec![python_action_edit(&path, version, &source, &after)?],
                            });
                        }
                        Ok(actions)
                    })();
                    let _ = tx.send(result);
                })
                .map_err(|e| e.to_string())?;
            self.code_actions.python_query = Some(rx);
        }
        self.code_actions.error = None;
        self.status = "Consultando correções…".into();
        Ok(())
    }
    pub(super) fn preview_code_action(&mut self, token: u64) -> Result<(), String> {
        if !self.code_actions_current() {
            return Err("Correção obsoleta; consulte novamente".into());
        }
        let c = self
            .code_actions
            .context
            .as_ref()
            .ok_or("Consulte as correções primeiro")?;
        if !matches!(
            c.phase,
            Phase::Choosing | Phase::Preview | Phase::Resolve | Phase::Preparing
        ) {
            return Err("Aguarde ou cancele a correção atual".into());
        }
        let item = self
            .code_actions
            .items
            .iter()
            .find(|i| i.token == token)
            .ok_or("Correção não pertence ao resultado atual")?;
        if let Some(reason) = &item.disabled_reason {
            return Err(reason.clone());
        }
        let local = c.local_changes.get(&token).cloned();
        let title = item.title.clone();
        if local.is_none() {
            self.language
                .dispatch(LanguageCommand::ResolveCodeAction { token })?;
        }
        self.code_actions.worker = None;
        self.code_actions.title = title;
        self.code_actions.files.clear();
        let c = self.code_actions.context.as_mut().unwrap();
        c.cancel.store(true, Ordering::Release);
        c.cancel = Arc::new(AtomicBool::new(false));
        c.phase = Phase::Resolve;
        c.token = Some(token);
        if let Some(changes) = local {
            c.changes = changes;
            self.code_action_worker(false)?;
        }
        self.status = "Preparando preview da correção…".into();
        Ok(())
    }
    fn code_action_worker(&mut self, apply: bool) -> Result<(), String> {
        if !self.code_actions_current() {
            return Err("Buffers mudaram; consulte a correção novamente".into());
        }
        let c = self.code_actions.context.as_mut().unwrap();
        let root = c.root.clone();
        let language = c.language;
        let changes = c.changes.clone();
        let cancel = c.cancel.clone();
        let expected = apply.then(|| c.prepared.clone());
        let open = self
            .documents
            .iter()
            .map(|d| OpenSnapshot {
                path: d.path.clone(),
                version: d.version,
                text: Arc::from(d.text()),
                encoding: d.encoding(),
                read_only: d.is_read_only(),
            })
            .collect::<Vec<_>>();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("codisseu-code-action".into())
            .spawn(move || {
                let _ = send.send(prepare(
                    &root,
                    language,
                    &changes,
                    &open,
                    expected.as_deref(),
                    &cancel,
                ));
            })
            .map_err(|e| e.to_string())?;
        c.phase = if apply {
            Phase::Applying
        } else {
            Phase::Preparing
        };
        self.code_actions.worker = Some(receive);
        Ok(())
    }
    pub(super) fn apply_code_action(&mut self) -> Result<(), String> {
        if self
            .code_actions
            .context
            .as_ref()
            .is_none_or(|c| c.phase != Phase::Preview)
        {
            return Err("Revise o preview antes de aplicar a correção".into());
        }
        self.code_action_worker(true)
    }
    pub(super) fn poll_code_actions(&mut self) -> bool {
        if !self.code_actions.is_open() {
            return false;
        }
        if !self.code_actions_current() {
            self.code_actions_failed("Correção cancelada: documento ou buffers mudaram".into());
            return true;
        }
        let language = self.code_actions.context.as_ref().unwrap().language;
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            self.code_actions_failed("Correção cancelada: provider indisponível".into());
            return true;
        }
        let python = self.code_actions.context.as_ref().unwrap().language == Language::Python;
        if python && self.code_actions.context.as_ref().unwrap().phase == Phase::Query {
            let result = self
                .code_actions
                .python_query
                .as_ref()
                .and_then(|r| match r.try_recv() {
                    Ok(v) => Some(v),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(_) => Some(Err("Consulta Ruff interrompida".into())),
                });
            if let Some(result) = result {
                self.code_actions.python_query = None;
                match result {
                    Ok(actions) => {
                        let c = self.code_actions.context.as_mut().unwrap();
                        c.phase = Phase::Choosing;
                        self.code_actions.items.clear();
                        for action in actions {
                            c.local_changes.insert(action.item.token, action.changes);
                            self.code_actions.items.push(action.item);
                        }
                        self.status = if self.code_actions.items.is_empty() {
                            "Ruff não propôs alterações seguras neste arquivo".into()
                        } else {
                            "Correções Ruff calculadas sobre o buffer; revise a prévia".into()
                        };
                    }
                    Err(error) => self.code_actions_failed(error),
                }
                return true;
            }
            return false;
        }
        if !python && let Some(error) = self.language.code_actions_error.take() {
            self.code_actions_failed(error);
            return true;
        }
        let phase = &self.code_actions.context.as_ref().unwrap().phase;
        // A Sync can cancel the wire query without changing a local document.
        // Mirror that cancellation instead of leaving the modal pending forever.
        if !python
            && matches!(phase, Phase::Query | Phase::Resolve)
            && self.language.code_actions.is_none()
            && self.language.code_action_edit.is_none()
            && !self.language.code_actions_pending()
        {
            self.code_actions_failed(
                "Consulta cancelada durante sincronização; tente novamente".into(),
            );
            return true;
        }
        if *phase == Phase::Query {
            if let Some(result) = self.language.code_actions.take() {
                let c = self.code_actions.context.as_ref().unwrap();
                if result.path != c.path || result.version != c.version {
                    self.code_actions_failed("Resultado de correções obsoleto".into());
                    return true;
                }
                self.code_actions.items = result.items;
                self.code_actions.truncated = result.truncated;
                self.code_actions.context.as_mut().unwrap().phase = Phase::Choosing;
                self.status = if self.code_actions.items.is_empty() {
                    "Nenhuma correção rápida nesta posição".into()
                } else {
                    format!(
                        "{} correção(ões) disponível(is)",
                        self.code_actions.items.len()
                    )
                };
                // Empty is a valid completed query; keep the popup with an honest empty state.
                return true;
            }
        } else if *phase == Phase::Resolve {
            if let Some(result) = self.language.code_action_edit.take() {
                let c = self.code_actions.context.as_mut().unwrap();
                if result.path != c.path
                    || result.version != c.version
                    || Some(result.token) != c.token
                {
                    self.code_actions_failed("Edição da correção obsoleta".into());
                    return true;
                }
                c.changes = result.changes;
                if let Err(error) = self.code_action_worker(false) {
                    self.code_actions_failed(error);
                }
                return true;
            }
        } else if matches!(phase, Phase::Preparing | Phase::Applying) {
            let ready = self
                .code_actions
                .worker
                .as_ref()
                .and_then(|r| match r.try_recv() {
                    Ok(value) => Some(value),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(_) => Some(Err("Worker da correção interrompido".into())),
                });
            if let Some(result) = ready {
                self.code_actions.worker = None;
                match result {
                    Err(error) => self.code_actions_failed(error),
                    Ok(files) => {
                        let c = self.code_actions.context.as_mut().unwrap();
                        if c.phase == Phase::Applying {
                            // Only worker-revalidated private proposals reach the common atomic transaction.
                            self.cancel_code_actions();
                            if let Err(error) = self.apply_code_action_files(&files) {
                                self.code_actions_failed(error);
                            }
                        } else {
                            c.prepared = files.clone();
                            c.phase = Phase::Preview;
                            self.code_actions.files = files;
                            self.status =
                                "Preview da correção; aplique explicitamente para alterar buffers"
                                    .into();
                        }
                    }
                }
                return true;
            }
        }
        false
    }
}
fn check(cancel: &AtomicBool, started: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(3) {
        Err("Correção cancelada ou orçamento excedido".into())
    } else {
        Ok(())
    }
}
fn prepare(
    root: &Path,
    language: Language,
    changes: &[RenameFileEdits],
    open: &[OpenSnapshot],
    expected: Option<&[RenamePreviewFile]>,
    cancel: &AtomicBool,
) -> Result<Vec<RenamePreviewFile>, String> {
    let started = Instant::now();
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    if canonical != root || changes.is_empty() || changes.len() > 64 {
        return Err("Projeto ou quantidade de arquivos da correção inválidos".into());
    }
    let mut files = Vec::new();
    let mut bytes = 0usize;
    let mut paths = std::collections::HashSet::new();
    for change in changes {
        check(cancel, started)?;
        let path = &change.path;
        let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !paths.insert(path.clone())
            || !path.starts_with(root)
            || path.canonicalize().map_err(|e| e.to_string())? != *path
            || !meta.is_file()
            || meta.file_type().is_symlink()
            || meta.len() > 1024 * 1024
        {
            return Err(
                "Correção contém caminho externo, link, arquivo excessivo ou duplicado".into(),
            );
        }
        if Language::for_path(path) != Some(language) {
            return Err("A correção só pode alterar fontes existentes da mesma linguagem".into());
        }
        let (before, encoding) = if let Some(doc) = open.iter().find(|d| d.path == *path) {
            if doc.read_only || change.version.is_some_and(|v| v != doc.version) {
                return Err("Correção contém versão obsoleta ou biblioteca somente leitura".into());
            }
            (doc.text.clone(), doc.encoding)
        } else {
            if change.version.is_some() {
                return Err("Correção versionada aponta para documento fechado".into());
            }
            let mut text = String::new();
            fs::File::open(path)
                .map_err(|e| e.to_string())?
                .take(1024 * 1024 + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("Fonte deve ser UTF-8: {e}"))?;
            if text.len() > 1024 * 1024 {
                return Err("Fonte excede1MiB".into());
            }
            (Arc::from(text), FileEncoding::Utf8)
        };
        let mut edits = change.edits.clone();
        if before.contains("\r\n") && !before.replace("\r\n", "").contains('\n') {
            for edit in &mut edits {
                edit.new_text = edit.new_text.replace("\r\n", "\n").replace('\n', "\r\n");
            }
        }
        let after = rename::build_after(&before, &edits)?;
        if after == before.as_ref() {
            return Err("Correção não altera o arquivo".into());
        }
        bytes = bytes
            .checked_add(before.len() + after.len())
            .ok_or("Tamanho excessivo")?;
        if bytes > 4 * 1024 * 1024 {
            return Err("Correção excede orçamento4MiB".into());
        }
        if path.canonicalize().map_err(|e| e.to_string())? != *path {
            return Err("Caminho da correção mudou durante leitura".into());
        }
        files.push(RenamePreviewFile {
            encoding,
            path: path.clone(),
            before,
            after: Arc::from(after),
            edits,
            presentation: vec![],
        });
    }
    check(cancel, started)?;
    if expected.is_some_and(|e| e != files) {
        return Err("Arquivo mudou desde o preview da correção; consulte novamente".into());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn change(path: PathBuf, start: u32, end: u32, new_text: &str) -> RenameFileEdits {
        RenameFileEdits {
            path,
            version: None,
            edits: vec![RenameTextEdit {
                start_line: 0,
                start_character: start,
                end_line: 0,
                end_character: end,
                new_text: new_text.into(),
            }],
        }
    }
    #[test]
    fn closed_fix_revalidates_disk_and_refuses_escape_overlap_and_surrogate_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("ação.rs");
        fs::write(&path, "let emoji = \"😀\";let x=1;\r\n").unwrap();
        let c = change(path.clone(), 0, 0, "// fixed\n");
        let cancel = AtomicBool::new(false);
        let files = prepare(
            &root,
            Language::Rust,
            std::slice::from_ref(&c),
            &[],
            None,
            &cancel,
        )
        .unwrap();
        assert!(files[0].after.starts_with("// fixed\r\n"));
        assert!(
            prepare(
                &root,
                Language::Rust,
                std::slice::from_ref(&c),
                &[],
                Some(&files),
                &cancel
            )
            .is_ok()
        );
        fs::write(&path, "fn changed(){}\n").unwrap();
        assert!(
            prepare(
                &root,
                Language::Rust,
                std::slice::from_ref(&c),
                &[],
                Some(&files),
                &cancel
            )
            .is_err()
        );
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(
            prepare(
                &root,
                Language::Rust,
                &[change(outside.path().into(), 0, 0, "x")],
                &[],
                None,
                &cancel
            )
            .is_err()
        );
        let mut overlap = change(path.clone(), 3, 6, "new");
        overlap.edits.push(RenameTextEdit {
            start_line: 0,
            start_character: 4,
            end_line: 0,
            end_character: 7,
            new_text: "other".into(),
        });
        assert!(prepare(&root, Language::Rust, &[overlap], &[], None, &cancel).is_err());
        fs::write(&path, "😀x").unwrap();
        assert!(
            prepare(
                &root,
                Language::Rust,
                &[change(path.clone(), 1, 2, "x")],
                &[],
                None,
                &cancel
            )
            .is_err()
        );
        cancel.store(true, Ordering::Release);
        assert!(prepare(&root, Language::Rust, &[c], &[], None, &cancel).is_err());
    }
    #[test]
    fn open_dirty_versioned_buffer_wins_over_disk_and_readonly_never_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let path = root.join("main.rs");
        fs::write(&path, "disk").unwrap();
        let mut open = OpenSnapshot {
            path: path.clone(),
            version: 4,
            text: Arc::from("dirty"),
            encoding: FileEncoding::Utf8,
            read_only: false,
        };
        let mut c = change(path, 0, 5, "fixed");
        c.version = Some(4);
        let cancel = AtomicBool::new(false);
        let files = prepare(
            &root,
            Language::Rust,
            &[c.clone()],
            &[open.clone()],
            None,
            &cancel,
        )
        .unwrap();
        assert_eq!(files[0].before.as_ref(), "dirty");
        assert_eq!(files[0].after.as_ref(), "fixed");
        c.version = Some(3);
        assert!(
            prepare(
                &root,
                Language::Rust,
                &[c.clone()],
                &[open.clone()],
                None,
                &cancel
            )
            .is_err()
        );
        c.version = Some(4);
        open.read_only = true;
        assert!(prepare(&root, Language::Rust, &[c.clone()], &[open], None, &cancel).is_err());
        assert!(prepare(&root, Language::Rust, &[c], &[], None, &cancel).is_err());
    }
}

fn python_action_edit(
    path: &Path,
    version: u64,
    before: &str,
    after: &str,
) -> Result<RenameFileEdits, String> {
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before
        .chars()
        .rev()
        .take(before.chars().count() - prefix)
        .zip(after.chars().rev().take(after.chars().count() - prefix))
        .take_while(|(a, b)| a == b)
        .count();
    let start = scalar_lsp_position(before, prefix).ok_or("Início de correção Python inválido")?;
    let end = scalar_lsp_position(before, before.chars().count() - suffix)
        .ok_or("Fim de correção Python inválido")?;
    Ok(RenameFileEdits {
        path: path.into(),
        version: Some(version),
        edits: vec![RenameTextEdit {
            start_line: start.0,
            start_character: start.1,
            end_line: end.0,
            end_character: end.1,
            new_text: after
                .chars()
                .skip(prefix)
                .take(after.chars().count() - prefix - suffix)
                .collect(),
        }],
    })
}

#[cfg(test)]
mod python_diff_tests {
    use super::*;
    #[test]
    fn distant_python_fix_previews_its_actual_utf16_source_location() {
        let before = format!(
            "{}emoji = \"😀\"; value = 40\r\n",
            "# context\r\n".repeat(900)
        );
        let after = before.replace("40", "42");
        let action = python_action_edit(Path::new("main.py"), 7, &before, &after).unwrap();
        assert_eq!(action.edits[0].start_line, 900);
        assert_eq!(rename::build_after(&before, &action.edits).unwrap(), after);
        assert_eq!(action.edits[0].new_text, "2");
    }
}
