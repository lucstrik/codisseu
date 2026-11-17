//! Atomic in-memory workspace rename; all filesystem verification runs on the worker.
use super::*;
mod file;
pub(crate) mod inheritors;
pub(crate) mod resource_config;
#[cfg(test)]
mod resource_tests;
pub(crate) mod resources;
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
#[derive(Clone, Debug)]
pub struct RenamePreviewFile {
    pub encoding: FileEncoding,
    pub path: PathBuf,
    pub before: Arc<str>,
    pub after: Arc<str>,
    pub edits: Vec<RenameTextEdit>,
    /// Worker-produced display labels; never part of semantic approval.
    pub presentation: Vec<codisseu_syntax::RenameOccurrencePresentation>,
}
impl PartialEq for RenamePreviewFile {
    fn eq(&self, other: &Self) -> bool {
        self.encoding == other.encoding
            && self.path == other.path
            && self.before == other.before
            && self.after == other.after
            && self.edits == other.edits
    }
}
impl Eq for RenamePreviewFile {}
#[derive(Clone, Debug)]
pub struct RenameConflictPreview {
    pub path: PathBuf,
    pub before: Arc<str>,
}
#[derive(Default)]
pub struct RenameState {
    options: RenameOptions,
    php_file_options: Option<RenameOptions>,
    web_file_options: Option<RenameOptions>,
    pub symbol_kind: codisseu_syntax::RenameSymbolKind,
    pub placeholder: Option<String>,
    pub new_name: String,
    pub files: Vec<RenamePreviewFile>,
    pub validation: Option<RenameValidation>,
    pub conflict_previews: Vec<RenameConflictPreview>,
    pub error: Option<String>,
    pub prepared: bool,
    pub used_fallback: bool,
    resource_moves: Vec<resources::ResourceLocation>,
    context: Option<Context>,
    next_token: u64,
}
#[derive(Clone, Default)]
pub(super) struct RenameCorpus {
    pub paths: Vec<PathBuf>,
    pub limitation: Option<String>,
}
#[derive(Clone)]
pub(super) struct PreparedRename {
    pub search: RenameSearch,
    pub files: Vec<RenamePreviewFile>,
    pub snapshots: Vec<codisseu_syntax::RenameSnapshot>,
    pub validation: RenameValidation,
    pub corpus: RenameCorpus,
    pub resources: Vec<resources::ResourceMove>,
}
#[derive(Clone, Default)]
pub(super) struct RenameSearch {
    pub python_interpreter: Option<PathBuf>,
    pub file_rename: bool,
    pub resource_configs: Arc<Vec<resource_config::ConfigSnapshot>>,
    pub resource_config_dirty: bool,
    pub inheritors: Arc<inheritors::Choices>,
    pub compiler: Option<Arc<codisseu_language::semantic::TypeScriptRuntime>>,
    pub compiler_config: Option<Option<worker_compiler::Configuration>>,
    /// Compiler locations are query origins only. All edits still come from LSP.
    pub compiler_missing: Vec<RenameQueryTarget>,
    pub line: u32,
    pub character: u32,
    pub options: RenameOptions,
    pub origin: PathBuf,
    pub old: String,
    pub text_snapshots: Vec<codisseu_syntax::RenameSnapshot>,
}
struct Context {
    search: RenameSearch,
    token: u64,
    origin: NavigationPosition,
    path: PathBuf,
    line: u32,
    character: u32,
    versions: Vec<(DocumentId, u64)>,
    snapshots: Vec<codisseu_syntax::RenameSnapshot>,
    proposal: Vec<RenamePreviewFile>,
    resources: Vec<resources::ResourceMove>,
    validation: Option<RenameValidation>,
    corpus: RenameCorpus,
    phase: Phase,
    methods: Vec<codisseu_syntax::RelatedMethod>,
    selected_methods: Vec<bool>,
    expansion: Option<Expansion>,
    local_semantic: bool,
    cancel: Arc<AtomicBool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RenameQueryTarget {
    pub path: PathBuf,
    pub range: TextRange,
}
struct Expansion {
    pending: VecDeque<RenameQueryTarget>,
    current: Option<RenameQueryTarget>,
    attempted: Vec<RenameQueryTarget>,
    started: Instant,
}
#[derive(PartialEq)]
enum Phase {
    Inspecting,
    Preparing,
    Prepared,
    Requesting,
    Building,
    Expanding,
    Selecting,
    ChoosingInheritors,
    Preview,
    Applying,
}
impl RenameState {
    pub fn options(&self) -> RenameOptions {
        self.options
    }
    pub fn resource_moves(&self) -> &[resources::ResourceLocation] {
        &self.resource_moves
    }
    pub fn symbol_kind(&self) -> codisseu_syntax::RenameSymbolKind {
        self.symbol_kind
    }
    pub fn choosing_methods(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Selecting)
    }
    pub fn related_methods(&self) -> &[codisseu_syntax::RelatedMethod] {
        self.context.as_ref().map_or(&[], |c| c.methods.as_slice())
    }
    pub fn method_selected(&self, index: usize) -> bool {
        self.context
            .as_ref()
            .and_then(|c| c.selected_methods.get(index))
            .copied()
            .unwrap_or(false)
    }
    pub fn can_confirm_methods(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Selecting && c.selected_methods.iter().all(|s| *s))
    }
    pub fn semantic_blocked(&self) -> bool {
        !self.is_pending()
            && self
                .validation
                .as_ref()
                .is_some_and(|v| v.coverage != Coverage::Verified)
    }
    pub fn can_apply(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Preview)
            && self
                .validation
                .as_ref()
                .is_some_and(|v| v.coverage == Coverage::Verified)
    }
    pub fn is_open(&self) -> bool {
        self.context.is_some()
    }
    pub fn is_prepared(&self) -> bool {
        self.prepared
    }
    pub fn is_pending(&self) -> bool {
        self.context.as_ref().is_some_and(|c| {
            !matches!(
                c.phase,
                Phase::Prepared | Phase::Selecting | Phase::ChoosingInheritors | Phase::Preview
            )
        })
    }
    pub fn context_token(&self) -> Option<u64> {
        self.context.as_ref().map(|c| c.token)
    }
    pub(super) fn applying(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|c| c.phase == Phase::Applying)
    }
}
#[derive(Clone)]
pub(super) struct RenameHistory {
    resources: Vec<resources::ResourceLocation>,
    entries: Vec<(DocumentId, u64, Arc<str>, Arc<str>)>,
    undone: bool,
    mappings: BTreeMap<DocumentId, (Vec<TextDelta>, Vec<TextDelta>)>,
}
pub(super) struct StagedRename {
    recovery_documents: Vec<Document>,
    documents: Vec<Document>,
    next_document: u64,
    history: RenameHistory,
    view_mappings: Vec<(DocumentId, Vec<TextDelta>)>,
    paths: Vec<resources::ResourceLocation>,
    message: String,
}
impl AppService {
    /// Includes a resource-only rename, which deliberately adds no buffer delta.
    pub fn can_undo_document(&self, id: DocumentId) -> bool {
        !self.rename_resources_busy()
            && (self.rename_history_ready(id, false)
                || self.documents.iter().any(|d| d.id == id && d.can_undo()))
    }
    pub fn can_redo_document(&self, id: DocumentId) -> bool {
        !self.rename_resources_busy()
            && (self.rename_history_ready(id, true)
                || self.documents.iter().any(|d| d.id == id && d.can_redo()))
    }
    fn rename_history_ready(&self, id: DocumentId, redo: bool) -> bool {
        self.rename_history.as_ref().is_some_and(|history| {
            history.undone == redo
                && history.entries.iter().any(|e| e.0 == id)
                && history.entries.iter().all(|(id, version, before, after)| {
                    self.documents.iter().any(|d| {
                        d.id == *id
                            && d.version == *version
                            && d.text()
                                == if redo {
                                    before.as_ref()
                                } else {
                                    after.as_ref()
                                }
                            && (before == after || if redo { d.can_redo() } else { d.can_undo() })
                    })
                })
        })
    }
    pub fn rename_resources_busy(&self) -> bool {
        self.rename_stage.is_some()
    }
    pub(super) fn cancel_rename(&mut self) {
        if let Some(c) = self.rename.context.take() {
            c.cancel.store(true, Ordering::Release);
            let _ = self.language.dispatch(LanguageCommand::CancelRename);
        }
        self.rename.files.clear();
        self.rename.resource_moves.clear();
        self.rename.validation = None;
        self.rename.conflict_previews.clear();
        self.rename.prepared = false;
    }
    pub(super) fn rename_failed(&mut self, e: String) {
        self.cancel_rename();
        self.rename.error = Some(e.clone());
        self.last_error = Some(e.clone());
        self.status = e;
    }
    pub(super) fn rename_current(&self) -> bool {
        self.rename.context.as_ref().is_some_and(|c| {
            c.versions.len() == self.documents.len()
                && c.versions.iter().all(|(id, v)| {
                    self.documents
                        .iter()
                        .any(|d| d.id == *id && d.version == *v)
                })
        })
    }
    pub(super) fn prepare_rename(
        &mut self,
        id: DocumentId,
        version: u64,
        caret: usize,
    ) -> Result<(), String> {
        let origin = self.navigation_position(id, version, caret)?;
        let doc = self.active_document().ok_or("Abra um documento")?;
        let path = doc.path.clone();
        let (line, character) = scalar_lsp_position(doc.text(), caret).ok_or("Posição inválida")?;
        self.cancel_rename();
        self.cancel_navigation();
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
                        .active_language_project()
                        .is_none_or(|r| d.project_root() == r)
            })
            .any(|d| {
                Language::for_path(&d.path).is_some_and(|lang| {
                    self.language
                        .statuses
                        .iter()
                        .any(|s| s.language == lang && s.state == ProviderState::Ready)
                }) && self.language_versions.get(&d.path) != Some(&d.version)
            })
        {
            return Err("Aguarde sincronização de todos os buffers".into());
        }
        let mut inspect_buffers = Vec::new();
        if Language::for_path(&path) == Some(Language::Php) {
            let mut bytes = 0usize;
            for document in self.documents.iter().filter(|d| {
                Language::for_path(&d.path) == Some(Language::Php)
                    && self
                        .active_language_project()
                        .is_none_or(|r| d.project_root() == r)
            }) {
                bytes = bytes.saturating_add(document.len_bytes());
                if document.len_bytes() > codisseu_editor::MAX_DOCUMENT_BYTES
                    || bytes > 4 * 1024 * 1024
                {
                    return Err(
                        "Preparação de rename excede 1 MiB por buffer/4 MiB de contexto".into(),
                    );
                }
                inspect_buffers.push((document.path.clone(), Arc::from(document.text())));
            }
        }
        let inspect = true;
        self.rename.next_token += 1;
        self.rename.context = Some(Context {
            search: RenameSearch::default(),
            token: self.rename.next_token,
            origin,
            path,
            line,
            character,
            versions: self.documents.iter().map(|d| (d.id, d.version)).collect(),
            snapshots: Vec::new(),
            proposal: Vec::new(),
            resources: Vec::new(),
            validation: None,
            corpus: RenameCorpus::default(),
            phase: if inspect {
                Phase::Inspecting
            } else {
                Phase::Preparing
            },
            methods: vec![],
            selected_methods: vec![],
            expansion: None,
            local_semantic: false,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        self.rename.error = None;
        self.rename.placeholder = None;
        self.rename.new_name.clear();
        if inspect {
            let c = self.rename.context.as_ref().unwrap();
            self.enqueue(
                Work::RenameInspect {
                    workspace: self.language_project_root().ok_or("Abra um workspace")?,
                    buffers: inspect_buffers,
                    token: c.token,
                    path: c.path.clone(),
                    text: Arc::from(self.documents.iter().find(|d| d.id == id).unwrap().text()),
                    line,
                    character,
                    cancel: c.cancel.clone(),
                },
                Pending::Rename(c.token),
            )?;
        } else {
            self.language.dispatch(LanguageCommand::PrepareRename {
                path: self.rename.context.as_ref().unwrap().path.clone(),
                version,
                line,
                character,
            })?;
        }
        Ok(())
    }
    pub(super) fn finish_rename_inspection(
        &mut self,
        token: u64,
        name: Option<String>,
        kind: codisseu_syntax::RenameSymbolKind,
    ) -> Result<(), String> {
        if !self.rename_current() {
            return Ok(());
        }
        let Some(c) = self
            .rename
            .context
            .as_mut()
            .filter(|c| c.token == token && c.phase == Phase::Inspecting)
        else {
            return Ok(());
        };
        self.rename.symbol_kind = kind;
        if let Some(name) = name {
            c.local_semantic = true;
            c.phase = Phase::Prepared;
            self.rename.new_name = name.clone();
            self.rename.placeholder = Some(name);
            self.rename.prepared = true;
            self.rename.used_fallback = false;
        } else {
            self.language.dispatch(LanguageCommand::PrepareRename {
                path: c.path.clone(),
                version: c.origin.version,
                line: c.line,
                character: c.character,
            })?;
            c.phase = Phase::Preparing;
        }
        Ok(())
    }
    pub(super) fn set_rename_options(&mut self, options: RenameOptions) -> Result<(), String> {
        let c = self
            .rename
            .context
            .as_ref()
            .ok_or("Prepare rename primeiro")?;
        if !matches!(c.phase, Phase::Prepared | Phase::Preview) {
            return Err("Altere opções antes de solicitar preview".into());
        }
        let c = self.rename.context.as_mut().unwrap();
        c.phase = Phase::Prepared;
        c.proposal.clear();
        c.resources.clear();
        c.validation = None;
        self.rename.files.clear();
        self.rename.resource_moves.clear();
        self.rename.validation = None;
        self.rename.options = options;
        if self.rename.is_file_rename() {
            if self
                .rename
                .context
                .as_ref()
                .unwrap()
                .path
                .extension()
                .is_some_and(|e| e == "php")
            {
                self.rename.options.search_text = false;
                self.rename.php_file_options = Some(self.rename.options);
            } else {
                self.rename.web_file_options = Some(options);
            }
        }
        Ok(())
    }
    pub(super) fn rename_search(&self) -> RenameSearch {
        RenameSearch {
            python_interpreter: self
                .rename
                .context
                .as_ref()
                .filter(|c| Language::for_path(&c.path) == Some(Language::Python))
                .and_then(|c| self.python_interpreter_for(&c.path)),
            file_rename: self.rename.is_file_rename(),
            resource_configs: Arc::default(),
            resource_config_dirty: self.documents.iter().any(|d| {
                self.language_project_root()
                    .is_none_or(|root| d.project_root() == root)
                    && d.is_dirty()
                    && d.path.file_name().is_some_and(|n| {
                        n == "tsconfig.json" || n == "jsconfig.json" || n == "composer.json"
                    })
            }),
            inheritors: Arc::default(),
            compiler: if self.documents.iter().any(|d| {
                self.language_project_root()
                    .is_none_or(|root| d.project_root() == root)
                    && d.is_dirty()
                    && d.path
                        .file_name()
                        .is_some_and(|n| n == "tsconfig.json" || n == "jsconfig.json")
            }) {
                None
            } else {
                self.language.typescript_runtime().map(Arc::new)
            },
            compiler_config: None,
            compiler_missing: vec![],
            line: self.rename.context.as_ref().map_or(0, |c| c.line),
            character: self.rename.context.as_ref().map_or(0, |c| c.character),
            options: if self.rename.context.as_ref().is_some_and(|c| {
                matches!(
                    Language::for_path(&c.path),
                    Some(Language::Rust | Language::Python)
                )
            }) {
                RenameOptions {
                    search_comments_strings: false,
                    search_text: false,
                    rename_accessors: false,
                    rename_inheritors: false,
                    ..self.rename.options
                }
            } else {
                self.rename.options
            },
            origin: self
                .rename
                .context
                .as_ref()
                .map(|c| c.path.clone())
                .unwrap_or_default(),
            old: self
                .rename
                .placeholder
                .clone()
                .unwrap_or_default()
                .trim_start_matches('$')
                .to_owned(),
            text_snapshots: vec![],
        }
    }
    pub(super) fn preview_rename(&mut self, new_name: String) -> Result<(), String> {
        if self.rename.is_file_rename() {
            return self.preview_file_rename(new_name);
        }
        if self
            .rename
            .context
            .as_ref()
            .is_some_and(|c| Language::for_path(&c.path) == Some(Language::Rust))
        {
            codisseu_syntax::rust_rename::validate_candidate(&new_name).map_err(str::to_owned)?;
        } else if self
            .rename
            .context
            .as_ref()
            .is_some_and(|c| Language::for_path(&c.path) == Some(Language::Python))
        {
            codisseu_syntax::python_rename::validate_candidate(&new_name).map_err(str::to_owned)?;
        } else {
            codisseu_syntax::validate_rename_candidate(&new_name).map_err(str::to_owned)?;
        }
        if !self.rename_current() {
            return Err("Rename obsoleto".into());
        }
        let c = self
            .rename
            .context
            .as_ref()
            .ok_or("Prepare rename primeiro")?;
        if !self.rename.prepared || c.phase == Phase::Applying {
            return Err("Aguarde preparação/aplicação".into());
        }
        if c.local_semantic {
            let mut buffers = Vec::new();
            let mut bytes = 0usize;
            for d in &self.documents {
                if d.is_read_only() {
                    continue;
                }
                if self
                    .active_language_project()
                    .is_some_and(|r| d.project_root() != r)
                {
                    continue;
                }
                if Language::for_path(&d.path).is_none()
                    && !(self.rename.options.search_text
                        && matches!(
                            d.path.extension().and_then(|s| s.to_str()),
                            Some("txt" | "md")
                        ))
                {
                    continue;
                }
                if !d.language_supported() {
                    return Err("Contexto aberto de rename excede 1 MiB".into());
                }
                bytes += d.len_bytes() * 2;
                if bytes > 8 * 1024 * 1024 {
                    return Err("Contexto aberto excede 8 MiB".into());
                }
                buffers.push((d.path.clone(), d.version, Arc::<str>::from(d.text())));
            }
            let token = c.token;
            let work = Work::RenamePreview {
                token,
                workspace: self.language_project_root().ok_or("Abra um workspace")?,
                changes: vec![],
                new_name: new_name.clone(),
                buffers,
                context_limitation: None,
                search: self.rename_search(),
                local_origin: Some((
                    c.path.clone(),
                    TextRange {
                        start_line: c.line,
                        start_character: c.character,
                        end_line: c.line,
                        end_character: c.character,
                    },
                )),
                cancel: c.cancel.clone(),
            };
            self.enqueue(work, Pending::Rename(token))?;
            self.rename.context.as_mut().unwrap().phase = Phase::Building;
            self.rename.new_name = new_name;
            self.rename.files.clear();
            self.rename.resource_moves.clear();
            self.rename.validation = None;
            return Ok(());
        }
        self.language.dispatch(LanguageCommand::Rename {
            path: c.path.clone(),
            version: c.origin.version,
            line: c.line,
            character: c.character,
            new_name: new_name.clone(),
        })?;
        let c = self.rename.context.as_mut().unwrap();
        c.cancel.store(true, Ordering::Release);
        c.cancel = Arc::new(AtomicBool::new(false));
        self.rename.next_token += 1;
        c.token = self.rename.next_token;
        c.phase = Phase::Requesting;
        c.expansion = None;
        c.methods.clear();
        c.selected_methods.clear();
        c.snapshots.clear();
        c.proposal.clear();
        c.validation = None;
        c.corpus = RenameCorpus::default();
        self.rename.new_name = new_name;
        self.rename.files.clear();
        self.rename.resource_moves.clear();
        self.rename.validation = None;
        self.rename.conflict_previews.clear();
        self.rename.error = None;
        Ok(())
    }
    pub(super) fn poll_rename(&mut self) -> bool {
        let Some(c) = self.rename.context.as_ref() else {
            return false;
        };
        if !self.rename_current() {
            self.cancel_rename();
            return true;
        }
        if c.expansion
            .as_ref()
            .is_some_and(|e| e.started.elapsed() > Duration::from_secs(30))
            && matches!(c.phase, Phase::Expanding | Phase::Building)
        {
            self.rename_failed("Consulta de referências de rename excedeu 30 segundos".into());
            return true;
        }
        if !matches!(
            c.phase,
            Phase::Preparing | Phase::Requesting | Phase::Expanding
        ) {
            return false;
        }
        if let Some(e) = self.language.rename_error.take() {
            self.rename_failed(e);
            return true;
        }
        if c.phase == Phase::Preparing {
            let Some(r) = self.language.prepare_rename.take() else {
                return false;
            };
            if r.path != c.path
                || r.version != c.origin.version
                || r.line != c.line
                || r.character != c.character
            {
                return false;
            }
            let placeholder = r.placeholder.or_else(|| {
                r.range.and_then(|r| {
                    let text = self
                        .documents
                        .iter()
                        .find(|d| d.id == c.origin.document_id)?
                        .text();
                    let a = utf16_byte_offset(text, r.start_line, r.start_character)?;
                    let b = utf16_byte_offset(text, r.end_line, r.end_character)?;
                    text.get(a..b).map(str::to_owned)
                })
            });
            self.rename.new_name = placeholder.clone().unwrap_or_default();
            self.rename.placeholder = placeholder;
            self.rename.used_fallback = r.used_fallback;
            self.rename.prepared = true;
            self.rename.context.as_mut().unwrap().phase = Phase::Prepared;
            return true;
        }
        let Some(r) = self.language.rename.take() else {
            return false;
        };
        if c.phase == Phase::Expanding {
            let method = c
                .expansion
                .as_ref()
                .and_then(|e| e.current.as_ref())
                .expect("rename query");
            let version = self
                .documents
                .iter()
                .find(|d| d.path == method.path)
                .map_or(0, |d| d.version);
            if r.path != method.path
                || r.version != version
                || r.line != method.range.start_line
                || r.character != method.range.start_character
                || r.new_name != self.rename.new_name
            {
                self.rename_failed(
                    "Resposta relacionada não corresponde ao snapshot solicitado".into(),
                );
                return true;
            }
            let token = c.token;
            let work = Work::RenameExpand {
                token,
                workspace: self.language_project_root().expect("rename workspace"),
                prepared: Box::new(PreparedRename {
                    files: c.proposal.clone(),
                    snapshots: c.snapshots.clone(),
                    validation: c
                        .validation
                        .as_ref()
                        .expect("private related validation")
                        .clone(),
                    corpus: c.corpus.clone(),
                    resources: c.resources.clone(),
                    search: c.search.clone(),
                }),
                result: r,
                open_versions: self
                    .documents
                    .iter()
                    .map(|d| (d.path.clone(), d.version))
                    .collect(),
                new_name: self.rename.new_name.clone(),
                cancel: c.cancel.clone(),
            };
            match self.enqueue(work, Pending::Rename(token)) {
                Ok(()) => self.rename.context.as_mut().unwrap().phase = Phase::Building,
                Err(e) => self.rename_failed(e),
            }
            return true;
        }
        if r.path != c.path
            || r.version != c.origin.version
            || r.line != c.line
            || r.character != c.character
            || r.new_name != self.rename.new_name
        {
            return false;
        }
        let token = c.token;
        let cancel = c.cancel.clone();
        let Some(workspace) = self.language_project_root() else {
            self.rename_failed("Abra um workspace".into());
            return true;
        };
        // Copy proposed targets first; unedited dirty context must never silently
        // fall back to disk when the bounded queue cannot retain it.
        let relevant = |d: &&Document| r.changes.iter().any(|f| f.path == d.path);
        let ordered = self
            .documents
            .iter()
            .filter(relevant)
            .chain(self.documents.iter().filter(|d| !relevant(d)));
        let mut buffers = Vec::new();
        let mut bytes = 0usize;
        let mut context_limitation = None;
        for d in ordered {
            if d.is_read_only() {
                continue;
            }
            if d.project_root() != workspace {
                continue;
            }
            if !d.language_supported() {
                context_limitation =
                    Some("Contexto aberto de rename excede 1 MiB por texto".into());
                break;
            }
            if bytes + d.len_bytes() > 8 * 1024 * 1024 {
                if r.changes.iter().any(|f| f.path == d.path) {
                    self.rename_failed("Proposta de rename excede cache 8 MiB".into());
                    return true;
                }
                context_limitation = Some("Contexto aberto excede cache 8 MiB".into());
                break;
            }
            bytes += d.len_bytes();
            buffers.push((d.path.clone(), d.version, Arc::<str>::from(d.text())));
        }
        match self.enqueue(
            Work::RenamePreview {
                token,
                workspace,
                changes: r.changes,
                new_name: self.rename.new_name.clone(),
                buffers,
                context_limitation,
                local_origin: None,
                search: self.rename_search(),
                cancel,
            },
            Pending::Rename(token),
        ) {
            Ok(()) => self.rename.context.as_mut().unwrap().phase = Phase::Building,
            Err(e) => self.rename_failed(e),
        }
        true
    }
    fn request_related_rename(&mut self) -> Result<bool, String> {
        let c = self.rename.context.as_mut().expect("rename context");
        let expansion = c.expansion.as_mut().expect("rename expansion");
        if expansion.started.elapsed() > Duration::from_secs(30) {
            return Err("Consulta de referências de rename excedeu 30 segundos".into());
        }
        let Some(method) = expansion.pending.pop_front() else {
            return Ok(false);
        };
        if expansion.attempted.len() >= 32 || expansion.attempted.contains(&method) {
            return Err("Rename excede 32 consultas únicas de referências".into());
        }
        let command = if let Some(doc) = self.documents.iter().find(|d| d.path == method.path) {
            LanguageCommand::Rename {
                path: doc.path.clone(),
                version: doc.version,
                line: method.range.start_line,
                character: method.range.start_character,
                new_name: self.rename.new_name.clone(),
            }
        } else {
            let snapshot = c
                .snapshots
                .iter()
                .find(|s| s.path == method.path)
                .ok_or("Método relacionado fora dos snapshots")?;
            LanguageCommand::RenameClosed {
                path: method.path.clone(),
                text: snapshot.before.to_string(),
                line: method.range.start_line,
                character: method.range.start_character,
                new_name: self.rename.new_name.clone(),
            }
        };
        self.language.dispatch(command)?;
        expansion.attempted.push(method.clone());
        expansion.current = Some(method);
        c.phase = Phase::Expanding;
        self.status = "Consultando referências para completar o rename".into();
        Ok(true)
    }
    pub(super) fn select_rename_method(
        &mut self,
        token: u64,
        index: usize,
        selected: bool,
    ) -> Result<(), String> {
        if !self.rename_current() {
            return Err("Seleção de rename obsoleta".into());
        }
        let c = self
            .rename
            .context
            .as_mut()
            .ok_or("Prepare rename primeiro")?;
        if c.token != token || c.phase != Phase::Selecting {
            return Err("Seleção de rename obsoleta".into());
        }
        *c.selected_methods
            .get_mut(index)
            .ok_or("Método relacionado inválido")? = selected;
        Ok(())
    }
    pub(super) fn confirm_rename_methods(&mut self, token: u64) -> Result<(), String> {
        if !self.rename_current() || self.rename.context_token() != Some(token) {
            return Err("Seleção de rename obsoleta".into());
        }
        if !self.rename.can_confirm_methods() {
            return Err("Seleção parcial rompe a cadeia de override; mantenha todos os métodos selecionados".into());
        }
        self.rename.context.as_mut().unwrap().phase = Phase::Preview;
        Ok(())
    }
    pub(super) fn apply_rename(&mut self) -> Result<(), String> {
        if !self.pending.is_empty() || self.search_worker.is_busy() {
            return Err("Aguarde operações de arquivo antes de Apply Rename".into());
        }
        if !self.rename_current() {
            return Err("Rename obsoleto".into());
        }
        let c = self
            .rename
            .context
            .as_ref()
            .ok_or("Prepare rename primeiro")?;
        if c.phase != Phase::Preview {
            return Err("Aguarde preview/aplicação".into());
        }
        if !self.rename.can_apply() {
            return Err(
                "Rename bloqueado: há conflitos ou análise não suportada no escopo declarado"
                    .into(),
            );
        }
        let token = c.token;
        let work = Work::RenameVerify {
            token,
            workspace: self.language_project_root().ok_or("Abra um workspace")?,
            files: c.proposal.clone(),
            resources: c.resources.clone(),
            search: c.search.clone(),
            snapshots: c.snapshots.clone(),
            corpus: c.corpus.clone(),
            new_name: self.rename.new_name.clone(),
            open: self.documents.iter().map(|d| d.path.clone()).collect(),
            cancel: c.cancel.clone(),
        };
        self.enqueue(work, Pending::Rename(token))?;
        self.rename.context.as_mut().unwrap().phase = Phase::Applying;
        Ok(())
    }
    pub(super) fn finish_rename(
        &mut self,
        token: u64,
        prepared: PreparedRename,
        apply: bool,
    ) -> Result<(), String> {
        if self.rename.context_token() != Some(token) || !self.rename_current() {
            return Ok(());
        }
        let PreparedRename {
            files,
            snapshots,
            validation,
            corpus,
            resources,
            search,
        } = prepared;
        let verified = validation.coverage == Coverage::Verified && corpus.limitation.is_none();
        // A provider may omit an interface-typed call from its first proposal.
        // Only actual related LSP queries may complete those edits; never apply
        // while a missing reference remains or while another check failed.
        let expandable = verified
            || (corpus.limitation.is_none()
                && validation.issues.is_empty()
                && !validation.conflicts.is_empty()
                && validation
                    .conflicts
                    .iter()
                    .all(|c| c.kind == codisseu_syntax::ConflictKind::MissingReference));
        self.rename.context.as_mut().unwrap().corpus = corpus;
        self.rename.context.as_mut().unwrap().search = search;
        self.rename.conflict_previews = snapshots
            .iter()
            .map(|s| RenameConflictPreview {
                path: s.path.clone(),
                before: s.before.clone(),
            })
            .collect();
        if apply
            && verified
            && self.rename.context.as_ref().is_some_and(|c| {
                c.methods != validation.related_methods || !c.selected_methods.iter().all(|s| *s)
            })
        {
            return Err("Métodos relacionados mudaram; refaça a seleção".into());
        }
        if !apply && expandable && !verified {
            let c = self.rename.context.as_mut().unwrap();
            if !c.local_semantic && !c.search.compiler_missing.is_empty() {
                let expansion = c.expansion.get_or_insert_with(|| Expansion {
                    pending: VecDeque::new(),
                    current: None,
                    attempted: vec![],
                    started: Instant::now(),
                });
                for target in &c.search.compiler_missing {
                    if !expansion.attempted.contains(target)
                        && !expansion
                            .pending
                            .iter()
                            .any(|queued| queued.path == target.path)
                    {
                        if expansion.attempted.len() + expansion.pending.len() >= 32 {
                            return Err("Rename excede 32 consultas únicas de referências".into());
                        }
                        expansion.pending.push_back(target.clone());
                    }
                }
                c.snapshots = snapshots.clone();
                c.proposal = files.clone();
                c.resources = resources.clone();
                c.validation = Some(validation.clone());
                self.rename.validation = Some(validation.clone());
                if self.request_related_rename()? {
                    return Ok(());
                }
            }
        }
        if !apply
            && expandable
            && !self
                .rename
                .context
                .as_ref()
                .is_some_and(|c| c.local_semantic)
            && !validation.related_methods.is_empty()
        {
            let c = self.rename.context.as_mut().unwrap();
            if c.expansion.is_none() {
                if validation.related_methods.len() > 32 {
                    return Err("Rename excede 32 consultas de declarações relacionadas".into());
                }
                c.methods = validation.related_methods.clone();
                c.expansion = Some(Expansion {
                    pending: c
                        .methods
                        .iter()
                        .map(|method| RenameQueryTarget {
                            path: method.path.clone(),
                            range: method.range.clone(),
                        })
                        .collect(),
                    current: None,
                    attempted: vec![],
                    started: Instant::now(),
                });
            } else if c.methods != validation.related_methods {
                return Err(
                    "Consulta relacionada alterou o conjunto de declarações; refaça preview".into(),
                );
            }
            c.snapshots = snapshots.clone();
            c.proposal = files.clone();
            c.validation = Some(validation.clone());
            self.rename.validation = Some(validation.clone());
            if self.request_related_rename()? {
                return Ok(());
            }
        }
        if !apply {
            let c = self.rename.context.as_mut().unwrap();
            c.methods = if verified {
                validation.related_methods.clone()
            } else {
                vec![]
            };
            c.selected_methods = vec![true; c.methods.len()];
        }
        self.rename.context.as_mut().unwrap().validation = Some(validation.clone());
        self.rename.validation = Some(validation);
        self.rename.context.as_mut().unwrap().snapshots = snapshots;
        self.rename.context.as_mut().unwrap().proposal = files.clone();
        self.rename.context.as_mut().unwrap().resources = resources.clone();
        self.rename.resource_moves = resources
            .iter()
            .map(|r| resources::ResourceLocation {
                source: r.source.clone(),
                target: r.target.clone(),
            })
            .collect();
        if !apply || !verified {
            if files.iter().any(|f| f.before == f.after) {
                return Err("Rename contém arquivo sem alteração efetiva".into());
            }
            self.rename.files = files;
            let c = self.rename.context.as_mut().unwrap();
            c.phase = if !c.search.inheritors.rows.is_empty() && !c.search.inheritors.confirmed {
                Phase::ChoosingInheritors
            } else if verified && !c.methods.is_empty() {
                Phase::Selecting
            } else {
                Phase::Preview
            };
            self.status = format!(
                "Preview de rename: {} alterações em {} arquivos",
                self.rename
                    .files
                    .iter()
                    .map(|f| f.edits.len())
                    .sum::<usize>(),
                self.rename.files.len()
            );
            if !verified {
                self.status = "Rename bloqueado: confira conflitos e limites da análise".into();
            }
            return Ok(());
        }
        let stage = self.stage_rename(&files, &resources)?;
        if resources.is_empty() {
            self.commit_rename_stage(stage);
        } else {
            self.queue_resource_stage(stage, resources)?;
        }
        Ok(())
    }
    /// Shared atomic buffer transaction for a separately validated code action.
    pub(super) fn apply_code_action_files(
        &mut self,
        files: &[RenamePreviewFile],
    ) -> Result<(), String> {
        let mut stage = self.stage_rename(files, &[])?;
        stage.message = "Correção aplicada aos buffers; conteúdo não salvo".into();
        self.commit_rename_stage(stage);
        Ok(())
    }
    fn stage_rename(
        &self,
        files: &[RenamePreviewFile],
        resources: &[resources::ResourceMove],
    ) -> Result<StagedRename, String> {
        let mut documents = self.documents.clone();
        let mut next = self.next_document;
        let mut recovery_documents = self.documents.clone();
        let mut entries = Vec::new();
        let mut mappings = BTreeMap::new();
        for f in files {
            if !documents.iter().any(|d| d.path == f.path) {
                if documents.len() >= MAX_DOCUMENTS {
                    return Err("Rename excederia 32 documentos abertos".into());
                }
                documents.push(Document {
                    read_only: false,
                    untitled: false,
                    project_root: self.project_root_for_path(&f.path),
                    id: next,
                    path: f.path.clone(),
                    version: 1,
                    buffer: TextBuffer::new(f.before.to_string())?,
                    saved_text: f.before.clone(),
                    saved_identity: Some(1),
                    dirty_cache: std::cell::Cell::new(None),
                    disk_missing: false,
                    encoding: f.encoding,
                });
                next += 1;
            }
            let doc = documents.iter_mut().find(|d| d.path == f.path).unwrap();
            if doc.is_read_only() {
                return Err("Rename não pode alterar biblioteca Rust somente leitura".into());
            }
            if !recovery_documents.iter().any(|d| d.id == doc.id) {
                recovery_documents.push(doc.clone());
            }
            if doc.text() != f.before.as_ref() {
                return Err("Buffer mudou antes da aplicação".into());
            }
            let forward = rename_deltas(&f.before, &f.edits)?;
            let snapshot = TextSnapshot::from_str(&f.before);
            let mut removed = 0;
            let mut inserted = 0;
            let mut inverse = Vec::new();
            for delta in forward.iter().rev() {
                let start = delta.range.start - removed + inserted;
                let len = delta.insert.chars().count();
                inverse.push(TextDelta {
                    range: start..start + len,
                    insert: snapshot.slice(delta.range.clone()).to_string(),
                });
                removed += delta.range.len();
                inserted += len;
            }
            inverse.reverse();
            let selection = renamed_selection(doc.selection(), &f.before, &f.edits)?;
            if doc
                .buffer
                .replace_with_selection(f.after.to_string(), selection)?
            {
                doc.version += 1;
            }
            entries.push((doc.id, doc.version, f.before.clone(), f.after.clone()));
            mappings.insert(doc.id, (forward, inverse));
        }
        let paths = resources
            .iter()
            .map(|r| resources::ResourceLocation {
                source: r.source.clone(),
                target: r.target.clone(),
            })
            .collect::<Vec<_>>();
        for r in resources {
            if documents.iter().any(|d| d.path == r.target) {
                return Err("Destino de Rename já está aberto".into());
            }
            let doc = documents
                .iter_mut()
                .find(|d| d.path == r.source)
                .ok_or("Arquivo movido não pertence ao commit de buffers")?;
            if doc.saved_text.as_bytes() != r.expected_disk.as_ref() {
                return Err("Conteúdo salvo mudou desde preview; refaça Rename".into());
            }
            if !entries.iter().any(|entry| entry.0 == doc.id) {
                let text: Arc<str> = Arc::from(doc.text());
                entries.push((doc.id, doc.version, text.clone(), text));
            }
            doc.path = r.target.clone();
        }
        let view_mappings = mappings
            .iter()
            .map(|(&id, (forward, _))| (id, forward.clone()))
            .collect();
        Ok(StagedRename {
            recovery_documents,
            documents,
            next_document: next,
            history: RenameHistory {
                entries,
                mappings,
                undone: false,
                resources: paths.clone(),
            },
            view_mappings,
            paths,
            message: "Rename aplicado aos buffers; conteúdo não salvo".into(),
        })
    }
    fn queue_resource_stage(
        &mut self,
        stage: StagedRename,
        moves: Vec<resources::ResourceMove>,
    ) -> Result<(), String> {
        if !self.pending.is_empty() || self.search_worker.is_busy() {
            return Err("Aguarde operações de arquivo antes de mover recursos".into());
        }
        let workspace = self.workspace.clone().ok_or("Workspace ausente")?;
        self.enqueue(
            Work::RenameResources { workspace, moves },
            Pending::RenameResources,
        )?;
        self.rename_stage = Some(stage);
        self.status = "Aplicando transação de arquivos de Rename".into();
        Ok(())
    }
    fn commit_rename_stage(&mut self, stage: StagedRename) {
        self.documents = stage.documents;
        self.next_document = stage.next_document;
        self.rename_history = Some(stage.history);
        for (id, deltas) in stage.view_mappings {
            self.remap_views(id, &deltas);
        }
        for moved in &stage.paths {
            let _ = self.close_language_document(&moved.source);
            self.language_versions.remove(&moved.source);
            self.language_skipped_versions.remove(&moved.source);
            self.language_retries.remove(&moved.source);
            for entry in &mut self.entries {
                if entry.path == moved.source {
                    entry.path = moved.target.clone();
                }
            }
        }
        if !stage.paths.is_empty() {
            sort_resource_tree(&mut self.entries);
        }
        self.cancel_rename();
        self.rename.error = None;
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.clear_search();
        self.status = stage.message;
    }
    pub(super) fn finish_resource_rename(
        &mut self,
        result: Result<resources::ResourceReceipt, resources::ResourceError>,
    ) {
        let Some(stage) = self.rename_stage.take() else {
            self.last_error = Some("ACK de transação sem estágio de recuperação".into());
            return;
        };
        match result {
            Ok(receipt) => {
                // Receipt is private worker data and must acknowledge the exact
                // noncancelable transaction. No fallible buffer work remains.
                assert_eq!(receipt.moves, stage.paths, "resource ACK identity");
                self.commit_rename_stage(stage);
            }
            Err(error) => {
                // If an external writer prevents rollback, keep original buffer
                // content but follow the actual surviving resource paths.
                for moved in &error.recovery {
                    if !self.documents.iter().any(|d| d.path == moved.source)
                        && let Some(doc) = stage
                            .recovery_documents
                            .iter()
                            .find(|d| d.path == moved.source)
                    {
                        self.documents.push(doc.clone());
                        self.next_document = stage.next_document;
                    }
                    for doc in &mut self.documents {
                        if doc.path == moved.source {
                            doc.path = moved.target.clone();
                        }
                    }
                    for entry in &mut self.entries {
                        if entry.path == moved.source {
                            entry.path = moved.target.clone();
                        }
                    }
                    let _ = self.close_language_document(&moved.source);
                    self.language_versions.remove(&moved.source);
                }
                if !error.recovery.is_empty() {
                    sort_resource_tree(&mut self.entries);
                    self.rename_history = None;
                }
                self.rename_failed(error.to_string());
            }
        }
    }
    pub(super) fn rename_undo(&mut self, id: DocumentId, redo: bool) -> Result<bool, String> {
        let Some(history) = self.rename_history.as_ref() else {
            return Ok(false);
        };
        if history.undone != redo || !history.entries.iter().any(|e| e.0 == id) {
            return Ok(false);
        }
        if !self.rename_history_ready(id, redo) {
            self.rename_history = None;
            return Ok(false);
        }
        let mut history = history.clone();
        let mut documents = self.documents.clone();
        let mut moves = Vec::new();
        let mut paths = Vec::new();
        for resource in &history.resources {
            let (source, target) = if redo {
                (&resource.source, &resource.target)
            } else {
                (&resource.target, &resource.source)
            };
            if documents.iter().any(|d| &d.path == target) {
                return Err("Destino de Undo/Redo já está aberto".into());
            }
            let doc = documents
                .iter_mut()
                .find(|d| &d.path == source)
                .ok_or("Documento de recurso não está aberto")?;
            moves.push(resources::ResourceMove {
                source: source.clone(),
                target: target.clone(),
                expected_disk: Arc::from(doc.saved_text.as_bytes()),
            });
            paths.push(resources::ResourceLocation {
                source: source.clone(),
                target: target.clone(),
            });
            doc.path = target.clone();
        }
        for (id, version, before, after) in &mut history.entries {
            let doc = documents.iter_mut().find(|d| d.id == *id).unwrap();
            if before != after
                && !(if redo {
                    doc.buffer.redo()
                } else {
                    doc.buffer.undo()
                })
            {
                return Err("Histórico mudou antes da preparação".into());
            }
            doc.version += 1;
            *version = doc.version;
        }
        history.undone = !redo;
        let view_mappings = history
            .mappings
            .iter()
            .map(|(&id, (forward, inverse))| {
                (
                    id,
                    if redo {
                        forward.clone()
                    } else {
                        inverse.clone()
                    },
                )
            })
            .collect();
        let stage = StagedRename {
            recovery_documents: self.documents.clone(),
            documents,
            next_document: self.next_document,
            history,
            view_mappings,
            paths,
            message: "Rename desfeito/refeito em buffers e caminhos".into(),
        };
        if moves.is_empty() {
            self.commit_rename_stage(stage);
        } else {
            self.queue_resource_stage(stage, moves)?;
        }
        Ok(true)
    }
}
// Preserve the workspace walker order: directories before files, then lexical
// siblings, with descendants immediately following their containing directory.
fn sort_resource_tree(entries: &mut [TreeEntry]) {
    entries.sort_by(|a, b| {
        let mut left = a.path.components().peekable();
        let mut right = b.path.components().peekable();
        loop {
            match (left.next(), right.next()) {
                (Some(l), Some(r)) => {
                    let l_file = left.peek().is_none() && !a.is_dir;
                    let r_file = right.peek().is_none() && !b.is_dir;
                    let order = l_file.cmp(&r_file).then_with(|| l.cmp(&r));
                    if !order.is_eq() {
                        return order;
                    }
                }
                (None, None) => return std::cmp::Ordering::Equal,
                (None, _) => return std::cmp::Ordering::Less,
                (_, None) => return std::cmp::Ordering::Greater,
            }
        }
    });
}

fn renamed_selection(
    mut selection: Selection,
    text: &str,
    edits: &[RenameTextEdit],
) -> Result<Selection, String> {
    for delta in rename_deltas(text, edits)? {
        selection = selection.mapped(&delta);
    }
    Ok(selection)
}
fn rename_deltas(text: &str, edits: &[RenameTextEdit]) -> Result<Vec<TextDelta>, String> {
    let mut deltas = edits
        .iter()
        .map(|e| {
            let start = utf16_byte_offset(text, e.start_line, e.start_character)
                .ok_or("Range de rename inválido")?;
            let end = utf16_byte_offset(text, e.end_line, e.end_character)
                .ok_or("Range de rename inválido")?;
            Ok(TextDelta {
                range: text[..start].chars().count()..text[..end].chars().count(),
                insert: e.new_text.clone(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    deltas.sort_by_key(|d| std::cmp::Reverse(d.range.start));
    Ok(deltas)
}
pub(super) fn build_after(before: &str, edits: &[RenameTextEdit]) -> Result<String, String> {
    if edits.is_empty() || edits.len() > 1024 {
        return Err("Quantidade de edições inválida".into());
    }
    let mut ranges = Vec::new();
    for e in edits {
        let a = utf16_byte_offset(before, e.start_line, e.start_character)
            .ok_or("Início UTF-16 inválido")?;
        let b =
            utf16_byte_offset(before, e.end_line, e.end_character).ok_or("Fim UTF-16 inválido")?;
        if a > b {
            return Err("Range invertido".into());
        }
        ranges.push((a, b, &e.new_text));
    }
    ranges.sort_by_key(|r| (r.0, r.1));
    for p in ranges.windows(2) {
        if p[1].0 < p[0].1 || p[1].0 == p[0].0 {
            return Err("Edições sobrepostas".into());
        }
    }
    let size = ranges
        .iter()
        .try_fold(before.len(), |n, (a, b, t)| {
            n.checked_sub(b - a)?.checked_add(t.len())
        })
        .ok_or("Tamanho inválido")?;
    if size > codisseu_editor::MAX_DOCUMENT_BYTES {
        return Err("Resultado excede 1 MiB".into());
    }
    let mut after = String::with_capacity(size);
    let mut end = 0;
    for (a, b, t) in ranges {
        after.push_str(&before[end..a]);
        after.push_str(t);
        end = b;
    }
    after.push_str(&before[end..]);
    TextBuffer::new(after.clone())?;
    Ok(after)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };
    fn idle(a: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(5);
        loop {
            a.poll();
            if !a.is_busy() {
                break;
            }
            assert!(Instant::now() < end, "{}", a.status);
            thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture() -> (tempfile::TempDir, AppService, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.ts");
        let b = root.join("b.ts");
        fs::write(&a, "/* 😀 */ class Weather { target() {} }\r\n").unwrap();
        fs::write(&b, "new Weather().target();\n").unwrap();
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root)).unwrap();
        idle(&mut app);
        app.dispatch(Command::OpenFile(a.clone())).unwrap();
        idle(&mut app);
        (dir, app, a, b)
    }
    fn edit(start: u32) -> RenameTextEdit {
        RenameTextEdit {
            start_line: 0,
            start_character: start,
            end_line: 0,
            end_character: start + 6,
            new_text: "forecast".into(),
        }
    }
    pub(super) fn incoming(app: &mut AppService, changes: Vec<codisseu_language::RenameFileEdits>) {
        app.cancel_rename();
        let d = app.active_document().unwrap();
        let path = d.path.clone();
        let origin = NavigationPosition {
            document_id: d.id,
            version: d.version,
            character_index: 0,
        };
        let version = d.version;
        app.rename.next_token += 1;
        app.rename.context = Some(Context {
            search: RenameSearch::default(),
            token: app.rename.next_token,
            origin,
            path: path.clone(),
            line: 0,
            character: 0,
            versions: app.documents.iter().map(|d| (d.id, d.version)).collect(),
            snapshots: Vec::new(),
            proposal: Vec::new(),
            resources: Vec::new(),
            validation: None,
            corpus: RenameCorpus::default(),
            phase: Phase::Requesting,
            methods: vec![],
            selected_methods: vec![],
            expansion: None,
            local_semantic: false,
            cancel: Arc::new(AtomicBool::new(false)),
        });
        app.rename.prepared = true;
        app.rename.error = None;
        app.rename.new_name = "forecast".into();
        app.language.rename = Some(codisseu_language::RenameResult {
            transient_document_version: None,
            path,
            version,
            line: 0,
            character: 0,
            new_name: "forecast".into(),
            changes,
        });
        app.poll();
    }
    fn changes(
        a: &std::path::Path,
        b: &std::path::Path,
        version: u64,
    ) -> Vec<codisseu_language::RenameFileEdits> {
        vec![
            codisseu_language::RenameFileEdits {
                path: a.to_path_buf(),
                version: Some(version),
                edits: vec![edit(25)],
            },
            codisseu_language::RenameFileEdits {
                path: b.to_path_buf(),
                version: Some(0),
                edits: vec![edit(14)],
            },
        ]
    }
    #[test]
    fn rename_view_mapping_uses_each_utf16_edit_not_the_enclosing_span() {
        let text = "😀a.rainfall(); b.rainfall();";
        let edits = text
            .match_indices("rainfall")
            .map(|(at, _)| {
                let start = text[..at].encode_utf16().count() as u32;
                RenameTextEdit {
                    start_line: 0,
                    start_character: start,
                    end_line: 0,
                    end_character: start + 8,
                    new_text: "precipitation".into(),
                }
            })
            .collect::<Vec<_>>();
        let between = text[..text.find("b.").unwrap()].chars().count();
        assert_eq!(
            renamed_selection(Selection::caret(between), text, &edits).unwrap(),
            Selection::caret(between + 5)
        );
        assert_eq!(
            renamed_selection(Selection::caret(0), text, &edits).unwrap(),
            Selection::caret(0)
        );
    }
    #[test]
    fn passive_view_between_rename_edits_keeps_directed_selection_through_workspace_history() {
        let (_dir, mut app, a, b) = fixture();
        let id = app.active.unwrap();
        let text = "/* 😀 */ class Weather { target() {} run() { this.target(); } }\r\n";
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: text.into(),
        })
        .unwrap();
        let between = text[..text.find("run").unwrap()].chars().count();
        let selection = Selection {
            anchor: between + 3,
            head: between,
        };
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 2,
            selection,
        })
        .unwrap();
        let left = app.active_view().unwrap();
        app.dispatch(Command::CreateView).unwrap();
        app.dispatch(Command::SetSelection {
            id,
            expected_version: 2,
            selection: Selection::caret(0),
        })
        .unwrap();
        let mut proposal = changes(&a, &b, 2);
        proposal[0].edits = text
            .match_indices("target")
            .map(|(at, _)| {
                let start = text[..at].encode_utf16().count() as u32;
                RenameTextEdit {
                    start_line: 0,
                    start_character: start,
                    end_line: 0,
                    end_character: start + 6,
                    new_text: "forecast".into(),
                }
            })
            .collect();
        incoming(&mut app, proposal);
        idle(&mut app);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        let shifted = Selection {
            anchor: selection.anchor + 2,
            head: selection.head + 2,
        };
        assert_eq!(app.view(left).unwrap().selection(), shifted);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.view(left).unwrap().selection(), selection);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.view(left).unwrap().selection(), shifted);
    }
    #[test]
    fn dirty_open_closed_atomic_no_save_and_group_history_survives_activation_save() {
        let (_dir, mut app, a, b) = fixture();
        let id = app.active.unwrap();
        let dirty = "/* 😀 */ class Weather { target() {} }\r\n// unsaved";
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.into(),
        })
        .unwrap();
        incoming(&mut app, changes(&a, &b, 2));
        idle(&mut app);
        assert_eq!(app.rename.files.len(), 2, "{:?}", app.rename.error);
        app.dispatch(Command::ApplyRename).unwrap();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        idle(&mut app);
        assert!(!app.rename.is_open());
        assert_eq!(app.documents.len(), 2);
        assert!(app.documents.iter().all(Document::is_dirty));
        assert_eq!(
            fs::read_to_string(&a).unwrap(),
            "/* 😀 */ class Weather { target() {} }\r\n"
        );
        assert_eq!(fs::read_to_string(&b).unwrap(), "new Weather().target();\n");
        let other = app.documents.iter().find(|d| d.path == b).unwrap().id;
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(
            app.documents.iter().find(|d| d.id == id).unwrap().text(),
            dirty
        );
        assert_eq!(
            app.documents.iter().find(|d| d.id == other).unwrap().text(),
            "new Weather().target();\n"
        );
        app.dispatch(Command::Activate { id: other }).unwrap();
        app.dispatch(Command::Save { id: other }).unwrap();
        idle(&mut app);
        app.dispatch(Command::OpenFile(a.clone())).unwrap();
        idle(&mut app);
        app.dispatch(Command::Redo { id }).unwrap();
        assert!(app.documents.iter().all(|d| d.text().contains("forecast")));
        app.dispatch(Command::OpenFile(b)).unwrap();
        idle(&mut app);
        assert_ne!(id, other);
        let new = app.workspace.as_ref().unwrap().join("c.ts");
        fs::write(&new, "c").unwrap();
        app.dispatch(Command::OpenFile(new)).unwrap();
        idle(&mut app);
        assert_ne!(app.active.unwrap(), other);
    }
    #[test]
    fn external_change_rejects_whole_batch() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        fs::write(&b, "// moved\nnew Weather().target();").unwrap();
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.as_ref().unwrap().contains("externamente"));
        assert_eq!(app.documents.len(), 1);
        assert!(!app.documents[0].is_dirty());
    }
    #[test]
    fn invalid_ranges_versions_and_overlaps_expose_no_partial_preview() {
        let (_dir, mut app, a, b) = fixture();
        for mode in 0..4 {
            let mut c = changes(&a, &b, 1);
            match mode {
                0 => c[0].edits[0].start_character = 4,
                1 => c[0].edits.push(edit(25)),
                2 => c[0].version = Some(99),
                _ => c[1].version = Some(9),
            }
            incoming(&mut app, c);
            idle(&mut app);
            assert!(app.rename.error.is_some());
            assert!(app.rename.files.is_empty());
            assert!(!app.documents[0].is_dirty());
        }
    }
    #[test]
    fn cancellation_new_request_and_changed_buffer_discard_old_worker_results() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        app.dispatch(Command::CancelRename).unwrap();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert_eq!(app.rename.files.len(), 2);
        let id = app.active.unwrap();
        app.dispatch(Command::ApplyRename).unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "intermediate".into(),
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].text(), "intermediate");
        assert!(!app.rename.is_open());
    }
    #[test]
    fn intermediate_edit_is_never_overwritten_by_workspace_undo() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        let id = app.active.unwrap();
        let other = app.documents.iter().find(|d| d.path == b).unwrap().id;
        app.dispatch(Command::Edit {
            id: other,
            expected_version: 2,
            text: "intermediate".into(),
        })
        .unwrap();
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(
            app.documents.iter().find(|d| d.id == other).unwrap().text(),
            "intermediate"
        );
    }
    #[test]
    fn invalid_identifier_candidates_are_rejected_before_context_lookup() {
        let (_dir, mut app, _, _) = fixture();
        for name in ["foo bar", "9foo", "🌧", "foo;evil()", ""] {
            let e = app
                .dispatch(Command::PreviewRename {
                    new_name: name.into(),
                })
                .unwrap_err();
            assert!(e.contains("identificador Unicode") || e.contains("entre 1 e 1024 bytes"));
        }
    }
    #[test]
    fn public_report_removal_during_related_query_keeps_private_validation() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        let context = app.rename.context.as_mut().unwrap();
        assert!(context.validation.is_some());
        let method = codisseu_syntax::RelatedMethod {
            path: a.clone(),
            range: TextRange {
                start_line: 0,
                start_character: 27,
                end_line: 0,
                end_character: 33,
            },
            container_name: "Weather".into(),
            name: "target".into(),
        };
        context.phase = Phase::Expanding;
        context.expansion = Some(Expansion {
            pending: VecDeque::new(),
            current: Some(RenameQueryTarget {
                path: method.path,
                range: method.range,
            }),
            attempted: vec![],
            started: Instant::now(),
        });
        app.rename.validation = None;
        app.language.rename = Some(codisseu_language::RenameResult {
            transient_document_version: None,
            path: a.clone(),
            version: 1,
            line: 0,
            character: 27,
            new_name: "forecast".into(),
            changes: changes(&a, &b, 1),
        });
        assert!(app.poll_rename());
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        assert!(app.rename.can_apply());
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].version, 1);
    }
    #[test]
    fn capacity_preflight_preserves_every_open_buffer() {
        let (_dir, mut app, a, b) = fixture();
        for n in 1..MAX_DOCUMENTS {
            let path = a.with_file_name(format!("extra{n}.txt"));
            fs::write(&path, "unchanged").unwrap();
            app.dispatch(Command::OpenFile(path)).unwrap();
            idle(&mut app);
        }
        let id = app.documents.iter().find(|d| d.path == a).unwrap().id;
        app.dispatch(Command::Activate { id }).unwrap();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.as_ref().unwrap().contains("32 documentos"));
        assert_eq!(app.documents.len(), MAX_DOCUMENTS);
        assert!(app.documents.iter().all(|d| !d.is_dirty()));
    }
    #[test]
    fn two_dirty_buffers_keep_prior_undo_after_group_reversal() {
        let (_dir, mut app, a, b) = fixture();
        let first = app.active.unwrap();
        app.dispatch(Command::Edit {
            id: first,
            expected_version: 1,
            text: "/* 😀 */ class Weather { target() {} }\r\n// dirty A".into(),
        })
        .unwrap();
        app.dispatch(Command::OpenFile(b.clone())).unwrap();
        idle(&mut app);
        let second = app.active.unwrap();
        app.dispatch(Command::Edit {
            id: second,
            expected_version: 1,
            text: "new Weather().target();\n// dirty B".into(),
        })
        .unwrap();
        app.dispatch(Command::Activate { id: first }).unwrap();
        let mut edits = changes(&a, &b, 2);
        edits[1].version = Some(2);
        incoming(&mut app, edits);
        idle(&mut app);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        app.dispatch(Command::Undo { id: first }).unwrap();
        assert!(app.documents.iter().all(|d| d.text().contains("dirty")));
        app.dispatch(Command::Undo { id: second }).unwrap();
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == second)
                .unwrap()
                .text(),
            "new Weather().target();\n"
        );
        assert!(
            app.documents
                .iter()
                .find(|d| d.id == first)
                .unwrap()
                .text()
                .contains("dirty A")
        );
    }
    #[test]
    fn dirty_collision_blocks_typed_apply_without_losing_report_or_history() {
        let (_dir, mut app, a, b) = fixture();
        let id = app.active.unwrap();
        let dirty = "/* 😀 */ class Weather { target() {} forecast() {} }\r\n";
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.into(),
        })
        .unwrap();
        incoming(&mut app, changes(&a, &b, 2));
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Rejected
        );
        assert!(app.rename.semantic_blocked());
        assert!(!app.rename.can_apply());
        assert_eq!(app.rename.files.len(), 2);
        let token = app.rename.context_token();
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.rename.context_token(), token);
        assert!(app.rename.error.is_none());
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].text(), dirty);
        assert!(
            app.rename
                .conflict_previews
                .iter()
                .any(|f| f.path == a && f.before.as_ref() == dirty)
        );
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(
            app.documents[0].text(),
            "/* 😀 */ class Weather { target() {} }\r\n"
        );
        assert_eq!(fs::read_to_string(b).unwrap(), "new Weather().target();\n");
    }
    #[test]
    fn nonmethod_and_unsupported_contexts_never_become_applicable() {
        let (_dir, mut app, a, b) = fixture();
        let id = app.active.unwrap();
        let dirty = "/* 😀 */ class Weather { target() {} [computed]() {} }\r\n";
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: dirty.into(),
        })
        .unwrap();
        incoming(&mut app, changes(&a, &b, 2));
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Unsupported
        );
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert!(app.rename.is_open());
        assert_eq!(app.documents[0].text(), dirty);
        app.dispatch(Command::Edit {
            id,
            expected_version: 2,
            text: "target();".into(),
        })
        .unwrap();
        let mut c = changes(&a, &b, 3);
        c[0].edits = vec![edit(0)];
        incoming(&mut app, c);
        idle(&mut app);
        assert!(!app.rename.can_apply());
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.documents[0].text(), "target();");
    }
    #[test]
    fn apply_rechecks_ast_even_if_public_report_was_replaced() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        let verified = app.rename.validation.clone().unwrap();
        assert_eq!(verified.coverage, Coverage::Verified);
        let id = app.active.unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: 1,
            text: "/* 😀 */ class Weather { target() {} forecast() {} }\r\n".into(),
        })
        .unwrap();
        incoming(&mut app, changes(&a, &b, 2));
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Rejected
        );
        app.rename.validation = Some(verified);
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Rejected
        );
        assert_eq!(app.documents.len(), 1);
        assert_eq!(app.documents[0].version, 2);
        assert!(app.rename.is_open());
    }
    #[test]
    fn unedited_open_context_is_checked_and_versions_guard_apply() {
        let (_dir, mut app, a, b) = fixture();
        let source = app.active.unwrap();
        let context = a.with_file_name("context.ts");
        fs::write(&context, "class Other {}\n").unwrap();
        app.dispatch(Command::OpenFile(context.clone())).unwrap();
        idle(&mut app);
        let other = app.active.unwrap();
        app.dispatch(Command::Edit {
            id: other,
            expected_version: 1,
            text: "class Other { broken( ".into(),
        })
        .unwrap();
        app.dispatch(Command::Activate { id: source }).unwrap();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Unsupported
        );
        assert!(
            app.rename
                .validation
                .as_ref()
                .unwrap()
                .issues
                .iter()
                .any(|i| i.path == context)
        );
        assert!(app.dispatch(Command::ApplyRename).is_err());
        app.dispatch(Command::Undo { id: other }).unwrap();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert!(app.rename.can_apply());
        app.dispatch(Command::ApplyRename).unwrap();
        app.dispatch(Command::Edit {
            id: other,
            expected_version: 3,
            text: "class Other { changed() {} }".into(),
        })
        .unwrap();
        idle(&mut app);
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == source)
                .unwrap()
                .version,
            1
        );
        assert!(!app.documents.iter().any(|d| d.path == b));
    }
    #[test]
    #[cfg(unix)]
    fn replaced_closed_path_cannot_pass_semantic_apply_guard() {
        let (_dir, mut app, a, b) = fixture();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert!(app.rename.can_apply());
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.ts");
        fs::write(&target, "new Weather().target();\n").unwrap();
        fs::remove_file(&b).unwrap();
        std::os::unix::fs::symlink(&target, &b).unwrap();
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.as_ref().unwrap().contains("mudou"));
        assert_eq!(app.documents.len(), 1);
        assert!(!app.documents[0].is_dirty());
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "new Weather().target();\n"
        );
    }
    #[test]
    fn corpus_new_removed_and_changed_closed_context_abort_entire_apply() {
        for mode in 0..3 {
            let (_dir, mut app, a, b) = fixture();
            let extra = a.with_file_name("context.ts");
            fs::write(&extra, "class Independent {}\n").unwrap();
            incoming(&mut app, changes(&a, &b, 1));
            idle(&mut app);
            assert!(app.rename.can_apply());
            match mode {
                0 => fs::write(a.with_file_name("new.ts"), "class NewlyKnown {}\n").unwrap(),
                1 => fs::remove_file(extra).unwrap(),
                _ => fs::write(extra, "class Independent { newMember() {} }\n").unwrap(),
            }
            app.dispatch(Command::ApplyRename).unwrap();
            idle(&mut app);
            assert!(app.rename.error.as_ref().unwrap().contains("mudou"));
            assert_eq!(app.documents.len(), 1);
            assert_eq!(app.documents[0].version, 1);
            assert!(!app.documents[0].is_dirty());
        }
    }
    #[test]
    fn corpus_exclusions_and_limits_are_explicit_without_partial_approval() {
        let (_dir, mut app, a, b) = fixture();
        let root = app.workspace.clone().unwrap();
        for excluded in [
            ".git",
            "target",
            "node_modules",
            "vendor",
            ".codisseu",
            ".storm",
            ".tools",
            "artifacts",
        ] {
            let folder = root.join(excluded);
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("broken.ts"), "class Invalid {").unwrap();
        }
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert!(app.rename.can_apply());
        assert!(
            app.rename
                .validation
                .as_ref()
                .unwrap()
                .note
                .contains("artifacts")
        );
        for n in 0..63 {
            fs::write(
                root.join(format!("context{n}.ts")),
                format!("class C{n} {{}}\n"),
            )
            .unwrap();
        }
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Unsupported
        );
        assert!(
            app.rename
                .validation
                .as_ref()
                .unwrap()
                .issues
                .iter()
                .any(|i| i.message.contains("64"))
        );
        assert_eq!(app.rename.files.len(), 2);
        assert!(app.dispatch(Command::ApplyRename).is_err());
        assert_eq!(app.documents.len(), 1);
    }
    #[test]
    fn context_large_file_and_depth_limits_preserve_preview_and_disk() {
        for mode in 0..3 {
            let (_dir, mut app, a, b) = fixture();
            let root = app.workspace.clone().unwrap();
            match mode {
                0 => {
                    fs::write(
                        root.join("large.ts"),
                        " ".repeat(codisseu_editor::MAX_DOCUMENT_BYTES + 1),
                    )
                    .unwrap();
                }
                1 => {
                    let mut p = root.clone();
                    for _ in 0..9 {
                        p = p.join("deep");
                        fs::create_dir(&p).unwrap();
                    }
                }
                _ => {
                    for n in 0..513 {
                        fs::write(root.join(format!("entry{n}.txt")), "").unwrap();
                    }
                }
            }
            incoming(&mut app, changes(&a, &b, 1));
            idle(&mut app);
            assert_eq!(
                app.rename.validation.as_ref().unwrap().coverage,
                Coverage::Unsupported,
                "{:?}",
                app.rename.validation
            );
            assert!(app.dispatch(Command::ApplyRename).is_err());
            assert_eq!(app.rename.files.len(), 2);
            assert!(!app.documents[0].is_dirty());
        }
    }
    #[test]
    fn lexical_root_prefix_does_not_authorize_an_external_proposal() {
        let (_dir, mut app, a, b) = fixture();
        let root = app.workspace.clone().unwrap();
        let prefix = format!("{}-outside", root.file_name().unwrap().to_string_lossy());
        let outside = tempfile::Builder::new()
            .prefix(&prefix)
            .tempdir_in(root.parent().unwrap())
            .unwrap();
        let target = outside.path().join("outside.ts");
        fs::write(&target, "new Weather().target();\n").unwrap();
        let mut edits = changes(&a, &b, 1);
        edits[1].path = target;
        incoming(&mut app, edits);
        idle(&mut app);
        assert!(
            app.rename
                .error
                .as_ref()
                .unwrap()
                .contains("fora do workspace")
        );
        assert!(!app.documents[0].is_dirty());
        assert_eq!(app.documents.len(), 1);
    }
    #[test]
    fn oversized_open_dirty_context_stays_unsupported_even_if_public_report_is_spoofed() {
        let (_dir, mut app, a, b) = fixture();
        let source = app.active.unwrap();
        for n in 0..9 {
            let path = a.with_file_name(format!("open{n}.ts"));
            fs::write(&path, "// disk\n").unwrap();
            app.dispatch(Command::OpenFile(path)).unwrap();
            idle(&mut app);
            let id = app.active.unwrap();
            let text = format!(
                "// dirty {}",
                " ".repeat(codisseu_editor::MAX_DOCUMENT_BYTES - 9)
            );
            app.dispatch(Command::Edit {
                id,
                expected_version: 1,
                text,
            })
            .unwrap();
        }
        app.dispatch(Command::Activate { id: source }).unwrap();
        incoming(&mut app, changes(&a, &b, 1));
        idle(&mut app);
        assert_eq!(app.rename.files.len(), 2);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Unsupported
        );
        assert!(
            app.rename
                .validation
                .as_ref()
                .unwrap()
                .issues
                .iter()
                .any(|i| i.message.contains("8 MiB"))
        );
        app.rename.validation.as_mut().unwrap().coverage = Coverage::Verified;
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Unsupported
        );
        assert_eq!(
            app.documents
                .iter()
                .find(|d| d.id == source)
                .unwrap()
                .version,
            1
        );
        assert!(!app.documents.iter().any(|d| d.path == b));
    }
}
