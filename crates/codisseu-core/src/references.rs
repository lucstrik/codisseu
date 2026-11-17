//! Versioned references, inert previews and reusable navigation without writes.
use super::*;
use std::sync::atomic::AtomicBool;

pub const MAX_REFERENCE_TARGETS: usize = 256;
pub(super) const MAX_REFERENCE_CACHE: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, Serialize)]
pub struct ReferenceItem {
    pub target: DefinitionTarget,
    pub preview: String,
    pub preview_text: String,
    pub preview_first_line: u32,
}
#[derive(Default)]
pub struct ReferenceState {
    pub items: Vec<ReferenceItem>,
    pub truncated: bool,
    pub error: Option<String>,
    context: Option<Context>,
    next_token: u64,
}
struct Context {
    token: u64,
    origin: NavigationPosition,
    line: u32,
    character: u32,
    versions: Vec<(DocumentId, u64)>,
    phase: Phase,
    cache: HashMap<PathBuf, Arc<str>>,
    cancel: Arc<AtomicBool>,
    navigation_origin: Option<NavigationPosition>,
}
#[derive(PartialEq)]
enum Phase {
    Requesting,
    Loading,
    Choosing,
    Opening,
}
impl ReferenceState {
    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }
    pub fn is_pending(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase != Phase::Choosing)
    }
    pub fn context_token(&self) -> Option<u64> {
        self.context.as_ref().map(|c| c.token)
    }
    pub(super) fn is_opening(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Opening)
    }
}
impl AppService {
    pub(super) fn cancel_references(&mut self) {
        if let Some(c) = self.references.context.take() {
            c.cancel.store(true, Ordering::Release);
            let _ = self.language.dispatch(LanguageCommand::CancelReferences);
        }
        self.references.items.clear();
        self.references.truncated = false;
    }
    pub(super) fn references_failed(&mut self, error: String) {
        self.cancel_references();
        self.references.error = Some(error.clone());
        self.last_error = Some(error.clone());
        self.status = error;
    }
    fn references_current(&self) -> bool {
        self.references.context.as_ref().is_some_and(|c| {
            c.versions.len() == self.documents.len()
                && c.versions.iter().all(|(id, v)| {
                    self.documents
                        .iter()
                        .any(|d| d.id == *id && d.version == *v)
                })
        })
    }
    pub(super) fn request_references(
        &mut self,
        id: DocumentId,
        version: u64,
        caret: usize,
    ) -> Result<(), String> {
        let origin = self.navigation_position(id, version, caret)?;
        let doc = self.active_document().ok_or("Abra um documento")?;
        if doc.is_read_only() {
            return Err(
                "Biblioteca Rust somente leitura: volte ao projeto para consultar referências"
                    .into(),
            );
        }
        let (line, character) =
            scalar_lsp_position(doc.text(), caret).ok_or("Referências: posição inválida")?;
        let path = doc.path.clone();
        let language = Language::for_path(&path).ok_or("Sem provider para este documento")?;
        if !self
            .language
            .statuses
            .iter()
            .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return Err("Inicie o provider antes de buscar referências".into());
        }
        self.cancel_navigation();
        self.cancel_rename();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.sync_documents();
        if self
            .documents
            .iter()
            .filter(|d| {
                !d.is_read_only()
                    && self
                        .language_project_root()
                        .is_none_or(|root| d.project_root() == root)
            })
            .any(|d| {
                Language::for_path(&d.path).is_some_and(|lang| {
                    self.language
                        .statuses
                        .iter()
                        .any(|s| s.language == lang && s.state == ProviderState::Ready)
                        && self.language_versions.get(&d.path) != Some(&d.version)
                })
            })
        {
            return Err("Aguarde a sincronização dos buffers antes de buscar referências".into());
        }
        self.language.dispatch(LanguageCommand::References {
            path,
            version,
            line,
            character,
            include_declaration: false,
        })?;
        self.references.next_token += 1;
        self.references.context = Some(Context {
            token: self.references.next_token,
            origin,
            line,
            character,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Requesting,
            cache: HashMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            navigation_origin: None,
        });
        self.references.error = None;
        self.status = "Buscando referências…".into();
        Ok(())
    }
    pub(super) fn poll_references(&mut self) -> bool {
        let Some(c) = self.references.context.as_ref() else {
            return false;
        };
        if !self.references_current() {
            self.cancel_references();
            return true;
        }
        if c.phase != Phase::Requesting {
            return false;
        }
        if let Some(error) = self.language.references_error.take() {
            self.references_failed(error);
            return true;
        }
        let Some(result) = self.language.references.take() else {
            return false;
        };
        let c = self.references.context.as_ref().unwrap();
        let Some(doc) = self.documents.iter().find(|d| d.id == c.origin.document_id) else {
            self.cancel_references();
            return true;
        };
        if result.path != doc.path
            || result.version != c.origin.version
            || result.line != c.line
            || result.character != c.character
        {
            self.cancel_references();
            return true;
        }
        let Some(workspace) = self.language_project_root() else {
            self.references_failed("Abra um workspace".into());
            return true;
        };
        let token = c.token;
        let cancel = c.cancel.clone();
        let truncated = result.truncated || result.targets.len() > MAX_REFERENCE_TARGETS;
        let targets: Vec<_> = result
            .targets
            .into_iter()
            .take(MAX_REFERENCE_TARGETS)
            .collect();
        // Only relevant open buffers cross the queue, bounded by the same cache limit.
        let mut open_buffers = Vec::new();
        let mut size = 0;
        for d in &self.documents {
            if targets.iter().any(|t| t.path == d.path) {
                if !d.language_supported() {
                    self.references_failed("Referências: documento aberto excede 1 MiB".into());
                    return true;
                }
                size += d.len_bytes();
                if size > MAX_REFERENCE_CACHE {
                    self.references_failed("Referências excedem cache de8MiB".into());
                    return true;
                }
                open_buffers.push(OpenedFile {
                    project_root: d.project_root.clone(),
                    encoding: d.encoding,
                    path: d.path.clone(),
                    text: d.text().into(),
                });
            }
        }
        let python_interpreter = self
            .documents
            .iter()
            .find(|d| {
                d.id == c.origin.document_id
                    && Language::for_path(&d.path) == Some(Language::Python)
            })
            .and_then(|d| self.python_interpreter_for(&d.path));
        let allow_rust_libraries = self.documents.iter().any(|d| {
            d.id == c.origin.document_id && Language::for_path(&d.path) == Some(Language::Rust)
        });
        match self.enqueue(
            Work::ReferencePreviews {
                token,
                targets,
                workspace,
                open_buffers,
                cancel,
                allow_rust_libraries,
                python_interpreter,
            },
            Pending::References(token),
        ) {
            Ok(()) => {
                self.references.context.as_mut().unwrap().phase = Phase::Loading;
                self.references.truncated = truncated;
                self.status = "Preparando referências…".into();
            }
            Err(e) => self.references_failed(e),
        }
        true
    }
    /// Display already-resolved navigation targets through the same bounded,
    /// versioned preview/navigation service. No synthetic provider request/reply.
    pub(super) fn present_navigation_targets(
        &mut self,
        origin: NavigationPosition,
        versions: Vec<(DocumentId, u64)>,
        targets: Vec<DefinitionTarget>,
    ) -> Result<(), String> {
        let workspace = self.language_project_root().ok_or("Abra um workspace")?;
        self.cancel_references();
        let mut open_buffers = Vec::new();
        let mut bytes = 0;
        for doc in &self.documents {
            if targets.iter().any(|t| t.path == doc.path) {
                doc.ensure_language_size()?;
                bytes += doc.len_bytes();
                if bytes > MAX_REFERENCE_CACHE {
                    return Err("Destinos excedem cache de8MiB".into());
                }
                open_buffers.push(OpenedFile {
                    project_root: doc.project_root.clone(),
                    encoding: doc.encoding,
                    path: doc.path.clone(),
                    text: doc.text().into(),
                });
            }
        }
        self.references.next_token += 1;
        let token = self.references.next_token;
        let cancel = Arc::new(AtomicBool::new(false));
        let python_interpreter = self
            .documents
            .iter()
            .find(|d| {
                d.id == origin.document_id && Language::for_path(&d.path) == Some(Language::Python)
            })
            .and_then(|d| self.python_interpreter_for(&d.path));
        let allow_rust_libraries = self.documents.iter().any(|d| {
            d.id == origin.document_id && Language::for_path(&d.path) == Some(Language::Rust)
        });
        self.enqueue(
            Work::ReferencePreviews {
                token,
                targets,
                workspace,
                open_buffers,
                cancel: cancel.clone(),
                allow_rust_libraries,
                python_interpreter,
            },
            Pending::References(token),
        )?;
        self.references.context = Some(Context {
            token,
            origin,
            line: 0,
            character: 0,
            versions,
            phase: Phase::Loading,
            cache: HashMap::new(),
            cancel,
            navigation_origin: None,
        });
        self.references.error = None;
        self.status = "Preparando destinos de navegação…".into();
        Ok(())
    }

    pub(super) fn finish_reference_previews(
        &mut self,
        token: u64,
        items: Vec<ReferenceItem>,
        cache: HashMap<PathBuf, Arc<str>>,
    ) -> Result<(), String> {
        if self.references.context_token() != Some(token) || !self.references_current() {
            return Ok(());
        }
        let c = self.references.context.as_mut().unwrap();
        c.cache = cache;
        c.phase = Phase::Choosing;
        self.references.items = items;
        self.status = if self.references.items.is_empty() {
            "Nenhuma referência encontrada".into()
        } else {
            format!("{} referência(s)", self.references.items.len())
        };
        Ok(())
    }
    pub(super) fn choose_reference(
        &mut self,
        index: usize,
        id: DocumentId,
        version: u64,
        caret: usize,
    ) -> Result<(), String> {
        let origin = self.navigation_position(id, version, caret)?;
        if !self.references_current() {
            return Err("Referências obsoletas: documento mudou".into());
        }
        let c = self
            .references
            .context
            .as_ref()
            .ok_or("Busque referências primeiro")?;
        if c.phase != Phase::Choosing {
            return Err("Aguarde a operação de referências".into());
        }
        let target = self
            .references
            .items
            .get(index)
            .ok_or("Referência inválida")?
            .target
            .clone();
        let expected = c
            .cache
            .get(&target.path)
            .cloned()
            .ok_or("Referência sem texto validado")?;
        let token = c.token;
        let workspace = self.language_project_root().ok_or("Abra um workspace")?;
        let cancel = c.cancel.clone();
        let open = self.documents.iter().any(|d| d.path == target.path);
        let python_interpreter = self
            .documents
            .iter()
            .find(|d| {
                d.id == c.origin.document_id
                    && Language::for_path(&d.path) == Some(Language::Python)
            })
            .and_then(|d| self.python_interpreter_for(&d.path));
        let allow_rust_libraries = self.documents.iter().any(|d| {
            d.id == c.origin.document_id && Language::for_path(&d.path) == Some(Language::Rust)
        });
        self.cancel_navigation();
        self.cancel_rename();
        self.cancel_completion();
        self.cancel_signature_help();
        self.enqueue(
            Work::ReferenceOpen {
                token,
                target,
                workspace,
                expected,
                open,
                cancel,
                allow_rust_libraries,
                python_interpreter,
            },
            Pending::References(token),
        )?;
        let c = self.references.context.as_mut().unwrap();
        c.phase = Phase::Opening;
        c.navigation_origin = Some(origin);
        self.status = "Abrindo referência…".into();
        Ok(())
    }
    pub(super) fn finish_reference_open(
        &mut self,
        token: u64,
        target: DefinitionTarget,
        file: Option<OpenedFile>,
        read_only: bool,
    ) -> Result<(), String> {
        if self.references.context_token() != Some(token) || !self.references_current() {
            return Ok(());
        }
        let origin = self
            .references
            .context
            .as_ref()
            .and_then(|c| c.navigation_origin.clone())
            .ok_or("Origem de referência ausente")?;
        self.activate_navigation_target(origin, target, file, read_only)?;
        let c = self.references.context.as_mut().unwrap();
        c.versions = self.documents.iter().map(|d| (d.id, d.version)).collect();
        c.phase = Phase::Choosing;
        c.navigation_origin = None;
        self.references.error = None;
        if self.navigation.warning.is_none() {
            self.status = "Referência aberta".into();
        }
        Ok(())
    }
}

pub(super) fn preview(text: &str, target: &DefinitionTarget) -> Result<String, String> {
    let start = utf16_byte_offset(text, target.start_line, target.start_character)
        .ok_or("Início de referência UTF-16 inválido")?;
    let end = utf16_byte_offset(text, target.end_line, target.end_character)
        .ok_or("Fim de referência UTF-16 inválido")?;
    if end < start {
        return Err("Intervalo de referência invertido".into());
    }
    let line = text
        .split('\n')
        .nth(target.start_line as usize)
        .ok_or("Linha de referência inválida")?
        .trim_end_matches('\r');
    let mut chars = line.chars();
    let mut output: String = chars
        .by_ref()
        .take(512)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if chars.next().is_some() {
        output.push('…');
    }
    Ok(output)
}

pub(super) fn preview_context(text: &str, target: &DefinitionTarget) -> (String, u32) {
    let first = target.start_line.saturating_sub(3);
    let mut result = String::new();
    for (index, line) in text
        .split('\n')
        .skip(first as usize)
        .take((target.start_line - first + 4) as usize)
        .enumerate()
    {
        if index > 0 {
            if result.len() == 8192 {
                break;
            }
            result.push('\n');
        }
        for ch in line.chars() {
            let ch = if ch.is_control() && ch != '\t' && ch != '\r' {
                ' '
            } else {
                ch
            };
            if result.len() + ch.len_utf8() > 8192 {
                return (result, first);
            }
            result.push(ch);
        }
    }
    (result, first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn idle(app: &mut AppService) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            app.poll();
            if !app.is_busy() {
                return;
            }
            assert!(Instant::now() < until, "{}", app.status);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn open(app: &mut AppService, path: &std::path::Path) -> DocumentId {
        app.dispatch(Command::OpenFile(path.into())).unwrap();
        idle(app);
        app.active.unwrap()
    }
    fn target(path: PathBuf, text: &str) -> DefinitionTarget {
        let byte = text.find("target").unwrap();
        let index = text[..byte].chars().count();
        let (line, character) = scalar_lsp_position(text, index).unwrap();
        DefinitionTarget {
            path,
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character + 6,
        }
    }
    fn fixture() -> (tempfile::TempDir, AppService, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = root.join("main.ts");
        let dest = root.join("ação.ts");
        fs::write(&source, "/* 🌧 e\u{301} */ target();\r\n").unwrap();
        fs::write(&dest, "/* 🌧 e\u{301} */ export const target = 42;\r\n").unwrap();
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root)).unwrap();
        idle(&mut app);
        open(&mut app, &source);
        (dir, app, source, dest)
    }
    fn incoming(app: &mut AppService, targets: Vec<DefinitionTarget>) {
        app.cancel_references();
        let doc = app.active_document().unwrap();
        let origin = NavigationPosition {
            document_id: doc.id,
            version: doc.version,
            character_index: 0,
        };
        let path = doc.path.clone();
        let version = doc.version;
        app.references.next_token += 1;
        app.references.context = Some(Context {
            token: app.references.next_token,
            origin,
            line: 0,
            character: 0,
            versions: app.documents.iter().map(|d| (d.id, d.version)).collect(),
            phase: Phase::Requesting,
            cache: HashMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            navigation_origin: None,
        });
        app.language.references = Some(codisseu_language::ReferencesResult {
            path,
            version,
            line: 0,
            character: 0,
            targets,
            truncated: false,
        });
        app.poll();
    }
    fn choose(app: &mut AppService, index: usize) {
        let d = app.active_document().unwrap();
        app.dispatch(Command::ChooseReference {
            index,
            id: d.id,
            expected_version: d.version,
            character_index: 0,
        })
        .unwrap();
        idle(app);
    }
    #[test]
    fn references_preserve_dirty_unicode_buffers_and_remain_navigable_with_history() {
        let (_dir, mut app, source, dest) = fixture();
        let source_id = app.active.unwrap();
        let source_disk = fs::read_to_string(&source).unwrap();
        let dest_disk = fs::read_to_string(&dest).unwrap();
        let dirty_source = format!("{}// dirty source", source_disk);
        app.dispatch(Command::Edit {
            id: source_id,
            expected_version: 1,
            text: dirty_source.clone(),
        })
        .unwrap();
        let dest_id = open(&mut app, &dest);
        let dirty_dest = dest_disk.replace("42", "99");
        app.dispatch(Command::Edit {
            id: dest_id,
            expected_version: 1,
            text: dirty_dest.clone(),
        })
        .unwrap();
        app.dispatch(Command::Activate { id: source_id }).unwrap();
        incoming(
            &mut app,
            vec![
                target(dest.clone(), &dirty_dest),
                target(source.clone(), &dirty_source),
            ],
        );
        idle(&mut app);
        assert!(app.references.items[0].preview.contains("99"));
        assert_eq!(app.references.items[0].preview_text, dirty_dest);
        assert_eq!(app.references.items[0].preview_first_line, 0);
        choose(&mut app, 0);
        assert_eq!(app.active, Some(dest_id));
        assert!(app.references.is_open());
        assert_eq!(app.references.items.len(), 2);
        let selection = app.navigation.selection.as_ref().unwrap();
        let d = app.active_document().unwrap();
        assert_eq!(
            d.text()
                .chars()
                .skip(selection.range.start)
                .take(6)
                .collect::<String>(),
            "target"
        );
        choose(&mut app, 1);
        assert_eq!(app.active, Some(source_id));
        assert!(app.references.is_open());
        app.dispatch(Command::NavigateBack {
            id: source_id,
            expected_version: 2,
            character_index: 0,
        })
        .unwrap();
        assert_eq!(app.active, Some(dest_id));
        assert_eq!(app.active_document().unwrap().text(), dirty_dest);
        app.dispatch(Command::Undo { id: dest_id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), dest_disk);
        assert_eq!(fs::read_to_string(source).unwrap(), source_disk);
        assert_eq!(fs::read_to_string(dest).unwrap(), dest_disk);
    }
    #[test]
    fn newly_opened_target_joins_snapshot_and_duplicate_choose_preserves_pending() {
        let (_dir, mut app, source, dest) = fixture();
        let source_id = app.active.unwrap();
        let text = fs::read_to_string(&dest).unwrap();
        incoming(
            &mut app,
            vec![
                target(dest.clone(), &text),
                target(source.clone(), &fs::read_to_string(&source).unwrap()),
            ],
        );
        idle(&mut app);
        let command = || Command::ChooseReference {
            index: 0,
            id: source_id,
            expected_version: 1,
            character_index: 0,
        };
        app.dispatch(command()).unwrap();
        assert!(app.dispatch(command()).is_err());
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().path, dest);
        assert!(app.references.is_open());
        assert_eq!(app.documents.len(), 2);
        choose(&mut app, 1);
        assert_eq!(app.active, Some(source_id));
        assert!(app.references.is_open());
    }
    #[test]
    fn closed_reference_changed_externally_is_not_opened_at_old_offsets() {
        let (_dir, mut app, _source, dest) = fixture();
        let source_id = app.active.unwrap();
        let text = fs::read_to_string(&dest).unwrap();
        incoming(&mut app, vec![target(dest.clone(), &text)]);
        idle(&mut app);
        fs::write(&dest, format!("// moved\n{text}")).unwrap();
        choose(&mut app, 0);
        assert_eq!(app.active, Some(source_id));
        assert_eq!(app.documents.len(), 1);
        assert!(!app.references.is_open());
        assert!(
            app.references
                .error
                .as_ref()
                .unwrap()
                .contains("mudou externamente")
        );
    }
    #[test]
    fn edits_cancel_previews_and_cancelled_errors_do_not_poison_new_result() {
        let (_dir, mut app, _source, dest) = fixture();
        let id = app.active.unwrap();
        let text = fs::read_to_string(&dest).unwrap();
        incoming(&mut app, vec![target(dest.clone(), &text)]);
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "target(); // new".into(),
        })
        .unwrap();
        idle(&mut app);
        assert!(!app.references.is_open());
        assert!(app.references.items.is_empty());
        incoming(
            &mut app,
            vec![target(dest.with_file_name("missing.ts"), &text)],
        );
        app.dispatch(Command::CancelReferences).unwrap();
        incoming(&mut app, vec![target(dest, &text)]);
        idle(&mut app);
        assert!(app.references.error.is_none());
        assert_eq!(app.references.items.len(), 1);
        app.language.sync_failures.insert(
            app.active_document().unwrap().path.clone(),
            codisseu_language::SyncFailure {
                version: 2,
                message: "controlled failure".into(),
            },
        );
        app.poll();
        assert!(!app.references.is_open());
        assert!(
            app.references
                .error
                .as_ref()
                .unwrap()
                .contains("sincronização")
        );
    }
    #[test]
    fn reference_preview_validates_ranges_paths_and_budget_before_exposing_items() {
        let (_dir, mut app, _source, dest) = fixture();
        let text = fs::read_to_string(&dest).unwrap();
        let mut broken = target(dest.clone(), &text);
        broken.start_character = 4; // inside the emoji surrogate
        incoming(&mut app, vec![broken]);
        idle(&mut app);
        assert!(app.references.error.is_some());
        assert!(app.references.items.is_empty());
        let outside = tempfile::tempdir().unwrap();
        let p = outside.path().join("target.ts");
        fs::write(&p, &text).unwrap();
        incoming(&mut app, vec![target(p.clone(), &text)]);
        idle(&mut app);
        assert!(
            app.references
                .error
                .as_ref()
                .unwrap()
                .contains("fora do workspace")
        );
        #[cfg(unix)]
        {
            let link = dest.with_file_name("link.ts");
            std::os::unix::fs::symlink(p, &link).unwrap();
            incoming(&mut app, vec![target(link, &text)]);
            idle(&mut app);
            assert!(
                app.references
                    .error
                    .as_ref()
                    .unwrap()
                    .contains("fora do workspace")
            );
        }
        let mut targets = Vec::new();
        for n in 0..9 {
            let path = dest.with_file_name(format!("large{n}.ts"));
            let content = format!(
                "target{}",
                " ".repeat(codisseu_editor::MAX_DOCUMENT_BYTES - 6)
            );
            fs::write(&path, &content).unwrap();
            targets.push(target(path, &content));
        }
        incoming(&mut app, targets);
        idle(&mut app);
        assert!(
            app.references
                .error
                .as_ref()
                .unwrap()
                .contains("cache de8MiB")
        );
        assert!(app.references.items.is_empty());
    }
    #[test]
    fn empty_and_truncated_reference_results_are_explicit() {
        let (_dir, mut app, _source, dest) = fixture();
        incoming(&mut app, vec![]);
        idle(&mut app);
        assert!(app.references.is_open());
        assert!(app.references.items.is_empty());
        assert_eq!(app.status, "Nenhuma referência encontrada");
        let text = fs::read_to_string(&dest).unwrap();
        incoming(&mut app, vec![target(dest, &text); 257]);
        idle(&mut app);
        assert_eq!(app.references.items.len(), 256);
        assert!(app.references.truncated);
    }
    #[test]
    fn excerpt_is_bounded_and_inert_with_combining_and_control_characters() {
        let text = format!("target\t\x1b{}", "🌧e\u{301}".repeat(3000));
        let t = target("x.ts".into(), &text);
        let line = preview(&text, &t).unwrap();
        assert!(!line.contains('\x1b'));
        assert!(!line.contains('\t'));
        assert!(line.ends_with('…'));
        assert!(line.chars().count() <= 513);
        let (context, first) = preview_context(&text, &t);
        assert_eq!(first, 0);
        assert!(context.len() <= 8192);
        assert!(!context.contains('\x1b'));
    }
}
