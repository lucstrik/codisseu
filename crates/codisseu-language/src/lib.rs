//! Minimal real LSP client. No GUI dependencies and no synthetic diagnostics.
mod completion;
mod definition;
mod formatting;
mod index_progress;
mod laravel;
mod laravel_assist;
mod navigation_presentation;
mod php_attributes;
mod php_project;
mod php_references;
mod php_super;
mod protocol;
mod python_libraries;
mod python_settings;
mod rename;
mod rust_libraries;
mod rust_settings;
pub use python_libraries::python_library_root;
pub use rust_libraries::rust_library_root;
mod rust_actions;
mod rust_inlay;
pub mod semantic;
mod signature;
pub mod snippets;
mod worker;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum Language {
    Php,
    TypeScript,
    Rust,
    Python,
}
impl Language {
    pub fn for_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "php" => Some(Self::Php),
            "ts" | "tsx" | "js" | "jsx" => Some(Self::TypeScript),
            "rs" => Some(Self::Rust),
            "py" | "pyi" | "pyw" => Some(Self::Python),
            _ => None,
        }
    }
    fn id(self, path: &std::path::Path) -> &'static str {
        match (self, path.extension().and_then(|s| s.to_str())) {
            (Self::Php, _) => "php",
            (Self::Rust, _) => "rust",
            (Self::Python, _) => "python",
            (_, Some("tsx")) => "typescriptreact",
            (_, Some("jsx")) => "javascriptreact",
            (_, Some("js")) => "javascript",
            _ => "typescript",
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub language: Language,
    pub program: PathBuf,
    pub args: Vec<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ProviderState {
    Stopped,
    Starting,
    Ready,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub language: Language,
    pub state: ProviderState,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub version: Option<u64>,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub message: String,
    pub severity: u32,
    /// LSP DiagnosticTag.Unnecessary; absent in older persisted snapshots.
    #[serde(default)]
    pub unnecessary: bool,
    pub language: Language,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncFailure {
    pub version: u64,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HoverResult {
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    /// Plain display text: no HTML, Markdown or links are executed.
    pub text: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DefinitionTarget {
    pub path: PathBuf,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefinitionLabel {
    pub public: bool,
    pub name: String,
    pub namespace: String,
    pub kind: char,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefinitionPresentation {
    pub source_symbol: String,
    pub targets: Vec<Option<DefinitionLabel>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DefinitionResult {
    #[serde(default)]
    pub presentation: Option<DefinitionPresentation>,
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub targets: Vec<DefinitionTarget>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReferencesResult {
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub targets: Vec<DefinitionTarget>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenameRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RenameTextEdit {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub new_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrepareRenameResult {
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub range: Option<RenameRange>,
    pub placeholder: Option<String>,
    /// The server supports rename but did not advertise prepareProvider.
    /// No lexical or semantic range is invented by this fallback.
    pub used_fallback: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameFileEdits {
    pub path: PathBuf,
    pub version: Option<u64>,
    pub edits: Vec<RenameTextEdit>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameResult {
    /// Exact wire version of a temporary closed snapshot, never an editor version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transient_document_version: Option<u64>,
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub new_name: String,
    pub changes: Vec<RenameFileEdits>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CompletionEdit {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    pub new_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: Option<u32>,
    pub filter_text: Option<String>,
    pub sort_text: Option<String>,
    pub new_text: String,
    pub text_edit: Option<CompletionEdit>,
    pub insert_edit: Option<CompletionEdit>,
    pub additional_edits: Vec<CompletionEdit>,
    pub unsupported_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionResult {
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
    pub truncated: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SignatureHelpTrigger {
    Invoked,
    Character(String),
    ContentChange,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SignatureHelpCapabilities {
    pub trigger_characters: Vec<String>,
    pub retrigger_characters: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureParameter {
    /// Validated UTF-8 byte range inside SignatureInformation.label.
    pub label_range: std::ops::Range<usize>,
    pub documentation: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureInformation {
    pub label: String,
    pub parameters: Vec<SignatureParameter>,
    pub documentation: Option<String>,
    pub active_parameter: Option<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignatureHelpResult {
    pub path: PathBuf,
    pub version: u64,
    pub line: u32,
    pub character: u32,
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: Option<usize>,
    pub active_parameter: Option<usize>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FormattingCapabilities {
    pub document: bool,
    pub range: bool,
}
/// Scalar ranges validated against the exact synchronized request text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormattingEdit {
    pub range: std::ops::Range<usize>,
    pub new_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FormattingResult {
    pub path: PathBuf,
    pub version: u64,
    pub range: Option<RenameRange>,
    pub edits: Vec<FormattingEdit>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RustCapabilities {
    pub code_actions: bool,
    pub inlay_hints: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeActionItem {
    pub token: u64,
    pub title: String,
    pub kind: Option<String>,
    pub preferred: bool,
    pub disabled_reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeActionsResult {
    pub path: PathBuf,
    pub version: u64,
    pub range: RenameRange,
    pub items: Vec<CodeActionItem>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CodeActionEditResult {
    pub path: PathBuf,
    pub version: u64,
    pub token: u64,
    pub title: String,
    pub changes: Vec<RenameFileEdits>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlayHint {
    pub line: u32,
    pub character: u32,
    pub label: String,
    pub kind: Option<u32>,
    pub padding_left: bool,
    pub padding_right: bool,
    pub tooltip: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InlayHintsResult {
    pub path: PathBuf,
    pub version: u64,
    pub range: RenameRange,
    pub items: Vec<InlayHint>,
    pub truncated: bool,
}
#[derive(Debug)]
pub enum LanguageCommand {
    /// Configuration only: no interpreter or project file is executed by this command.
    PythonEnvironment {
        python: Option<PathBuf>,
    },
    CodeActions {
        path: PathBuf,
        version: u64,
        range: RenameRange,
    },
    ResolveCodeAction {
        token: u64,
    },
    CancelCodeActions,
    InlayHints {
        path: PathBuf,
        version: u64,
        range: RenameRange,
    },
    CancelInlayHints,
    Start {
        workspace: PathBuf,
        configs: Vec<ProviderConfig>,
    },
    Stop,
    AnswerServerChoice {
        token: u64,
        index: Option<usize>,
    },
    Hover {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    CancelHover,
    Formatting {
        path: PathBuf,
        version: u64,
        range: Option<RenameRange>,
        tab_size: u32,
        insert_spaces: bool,
    },
    CancelFormatting,
    Definition {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    /// Shares the versioned navigation response/cancellation slot with Definition.
    Implementation {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    SuperDefinition {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    TypeDefinition {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    CancelDefinition,
    References {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
        include_declaration: bool,
    },
    CancelReferences,
    PrepareRename {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    Rename {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
        new_name: String,
    },
    /// Query a closed, exact snapshot without adding a document to the public service.
    RenameClosed {
        path: PathBuf,
        text: String,
        line: u32,
        character: u32,
        new_name: String,
    },
    CancelRename,
    Completion {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
    },
    CancelCompletion,
    SignatureHelp {
        path: PathBuf,
        version: u64,
        line: u32,
        character: u32,
        trigger: SignatureHelpTrigger,
        is_retrigger: bool,
    },
    CancelSignatureHelp,
    Sync {
        path: PathBuf,
        version: u64,
        text: String,
    },
    Close {
        path: PathBuf,
    },
}
struct Request {
    id: u64,
    epoch: u64,
    command: LanguageCommand,
}
enum Event {
    RustCapabilities {
        language: Language,
        capabilities: RustCapabilities,
    },
    CodeActions {
        id: u64,
        result: Result<CodeActionsResult, String>,
    },
    CodeActionEdit {
        id: u64,
        result: Result<CodeActionEditResult, String>,
    },
    InlayHints {
        id: u64,
        result: Result<InlayHintsResult, String>,
    },
    ServerChoice(ServerChoice),
    ServerChoiceClosed(u64),
    Formatting {
        id: u64,
        result: Result<FormattingResult, String>,
    },
    FormattingCapabilities {
        language: Language,
        capabilities: FormattingCapabilities,
    },
    SignatureHelp {
        id: u64,
        result: Result<SignatureHelpResult, String>,
    },
    SignatureHelpCapabilities {
        language: Language,
        capabilities: SignatureHelpCapabilities,
    },
    Ack(u64),
    Hover {
        id: u64,
        result: Result<HoverResult, String>,
    },
    Definition {
        id: u64,
        result: Result<DefinitionResult, String>,
    },
    Completion {
        id: u64,
        result: Result<CompletionResult, String>,
    },
    References {
        id: u64,
        result: Result<ReferencesResult, String>,
    },
    PrepareRename {
        id: u64,
        result: Result<PrepareRenameResult, String>,
    },
    Rename {
        id: u64,
        result: Result<RenameResult, String>,
    },
    Status(ProviderStatus),
    Error(String),
    SyncFailed {
        path: PathBuf,
        failure: SyncFailure,
    },
    Diagnostics {
        path: PathBuf,
        version: u64,
        unversioned: bool,
        items: Vec<Diagnostic>,
    },
}
struct Envelope {
    epoch: u64,
    event: Event,
}
/// A bounded provider question associated with one current semantic request.
/// Titles are display text only. The worker retains and echoes the original item.
#[derive(Clone, Debug)]
pub struct ServerChoice {
    pub token: u64,
    pub language: Language,
    pub message: String,
    pub items: Vec<String>,
    owner: u64,
}
pub struct LanguageService {
    pub rust_capabilities: HashMap<Language, RustCapabilities>,
    pub code_actions: Option<CodeActionsResult>,
    pub code_action_edit: Option<CodeActionEditResult>,
    pub code_actions_error: Option<String>,
    current_code_actions: Option<(u64, PathBuf, u64)>,
    code_action_tokens: HashMap<u64, (PathBuf, u64)>,
    pub inlay_hints: Option<InlayHintsResult>,
    pub inlay_hints_error: Option<String>,
    current_inlay_hints: Option<(u64, PathBuf, u64)>,
    pub server_choices: Vec<ServerChoice>,
    pub formatting: Option<FormattingResult>,
    pub formatting_error: Option<String>,
    pub formatting_capabilities: HashMap<Language, FormattingCapabilities>,
    current_formatting: Option<(u64, PathBuf, u64)>,
    semantic_runtime: Option<semantic::TypeScriptRuntime>,
    pub signature_help: Option<SignatureHelpResult>,
    pub signature_help_error: Option<String>,
    pub signature_help_capabilities: HashMap<Language, SignatureHelpCapabilities>,
    current_signature_help: Option<(u64, PathBuf, u64, u32, u32)>,
    pub prepare_rename: Option<PrepareRenameResult>,
    pub rename: Option<RenameResult>,
    pub rename_error: Option<String>,
    current_rename: Option<(u64, PathBuf, Option<u64>)>,
    pub references: Option<ReferencesResult>,
    pub references_error: Option<String>,
    current_references: Option<(u64, PathBuf, u64)>,
    pub completion: Option<CompletionResult>,
    pub completion_error: Option<String>,
    current_completion: Option<(u64, PathBuf, u64)>,
    pub definition: Option<DefinitionResult>,
    pub definition_error: Option<String>,
    current_definition: Option<(u64, PathBuf, u64)>,
    pub hover: Option<HoverResult>,
    pub hover_error: Option<String>,
    current_hover: Option<(u64, PathBuf, u64)>,
    pub diagnostics: Vec<Diagnostic>,
    /// Client-associated publication versions. Check `unversioned_paths` before
    /// treating a value as server-confirmed: unversioned pushes are inferred.
    pub analyzed_versions: HashMap<PathBuf, u64>,
    /// Paths whose latest accepted publication omitted the server version.
    pub unversioned_paths: HashSet<PathBuf>,
    /// Asynchronous Sync failures; consumers may remove entries after handling them.
    pub sync_failures: HashMap<PathBuf, SyncFailure>,
    pub statuses: Vec<ProviderStatus>,
    pub last_error: Option<String>,
    sender: SyncSender<Request>,
    receiver: Receiver<Envelope>,
    pending: HashSet<u64>,
    versions: HashMap<PathBuf, u64>,
    epoch: u64,
    next: u64,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Default for LanguageService {
    fn default() -> Self {
        Self::new()
    }
}
impl LanguageService {
    pub fn typescript_runtime(&self) -> Option<semantic::TypeScriptRuntime> {
        self.semantic_runtime.clone()
    }
    pub fn new() -> Self {
        let (sender, rx) = mpsc::sync_channel(32);
        let (tx, receiver) = mpsc::sync_channel(128);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker = thread::spawn(move || worker::run(rx, tx, worker_stop));
        Self {
            rust_capabilities: HashMap::new(),
            code_actions: None,
            code_action_edit: None,
            code_actions_error: None,
            current_code_actions: None,
            code_action_tokens: HashMap::new(),
            inlay_hints: None,
            inlay_hints_error: None,
            current_inlay_hints: None,
            server_choices: Vec::new(),
            formatting: None,
            formatting_error: None,
            formatting_capabilities: HashMap::new(),
            current_formatting: None,
            semantic_runtime: None,
            signature_help: None,
            signature_help_error: None,
            signature_help_capabilities: HashMap::new(),
            current_signature_help: None,
            prepare_rename: None,
            rename: None,
            rename_error: None,
            current_rename: None,
            references: None,
            references_error: None,
            current_references: None,
            completion: None,
            completion_error: None,
            current_completion: None,
            definition: None,
            definition_error: None,
            current_definition: None,
            hover: None,
            hover_error: None,
            current_hover: None,
            diagnostics: Vec::new(),
            analyzed_versions: HashMap::new(),
            unversioned_paths: HashSet::new(),
            sync_failures: HashMap::new(),
            statuses: Vec::new(),
            last_error: None,
            sender,
            receiver,
            pending: HashSet::new(),
            versions: HashMap::new(),
            epoch: 0,
            next: 1,
            stop,
            worker: Some(worker),
        }
    }
    pub fn answer_server_choice(&mut self, token: u64, index: Option<usize>) -> Result<(), String> {
        let choice = self
            .server_choices
            .iter()
            .find(|c| c.token == token)
            .ok_or("Escolha do provider já encerrada")?;
        if index.is_some_and(|i| i >= choice.items.len()) {
            return Err("Opção do provider fora do intervalo".into());
        }
        self.dispatch(LanguageCommand::AnswerServerChoice { token, index })
    }
    fn server_choice_owner_current(&self, owner: u64) -> bool {
        self.current_definition
            .as_ref()
            .is_some_and(|r| r.0 == owner)
            || self
                .current_references
                .as_ref()
                .is_some_and(|r| r.0 == owner)
            || self
                .current_completion
                .as_ref()
                .is_some_and(|r| r.0 == owner)
            || self.current_hover.as_ref().is_some_and(|r| r.0 == owner)
            || self.current_rename.as_ref().is_some_and(|r| r.0 == owner)
            || self
                .current_formatting
                .as_ref()
                .is_some_and(|r| r.0 == owner)
            || self
                .current_signature_help
                .as_ref()
                .is_some_and(|r| r.0 == owner)
    }
    fn invalidate_python_environment(&mut self) {
        let python = |p: &std::path::Path| Language::for_path(p) == Some(Language::Python);
        macro_rules! clear {
            ($pending:ident,$result:ident,$error:ident) => {
                if self.$pending.as_ref().is_some_and(|r| python(&r.1))
                    || self.$result.as_ref().is_some_and(|r| python(&r.path))
                {
                    self.$pending = None;
                    self.$result = None;
                    self.$error = None;
                }
            };
        }
        clear!(current_hover, hover, hover_error);
        clear!(current_definition, definition, definition_error);
        clear!(current_completion, completion, completion_error);
        clear!(current_references, references, references_error);
        clear!(current_rename, rename, rename_error);
        clear!(current_formatting, formatting, formatting_error);
        clear!(current_signature_help, signature_help, signature_help_error);
        clear!(current_inlay_hints, inlay_hints, inlay_hints_error);
        clear!(current_code_actions, code_actions, code_actions_error);
        if self
            .prepare_rename
            .as_ref()
            .is_some_and(|r| python(&r.path))
        {
            self.prepare_rename = None;
        }
        if self
            .code_action_edit
            .as_ref()
            .is_some_and(|r| python(&r.path))
        {
            self.code_action_edit = None;
        }
        self.code_action_tokens.retain(|_, (p, _)| !python(p));
        self.diagnostics.retain(|d| d.language != Language::Python);
        self.analyzed_versions.retain(|p, _| !python(p));
        self.unversioned_paths.retain(|p| !python(p));
    }
    pub fn dispatch(&mut self, command: LanguageCommand) -> Result<(), String> {
        if let LanguageCommand::PythonEnvironment { python: Some(path) } = &command
            && (!path.is_absolute()
                || path.as_os_str().len() > 4096
                || path.as_os_str().is_empty()
                || path.to_str().is_none_or(|p| p.contains(['\0', '\n', '\r'])))
        {
            return Err("Interpretador Python deve ser caminho absoluto de até4096bytes".into());
        }
        let python_environment_change =
            matches!(&command, LanguageCommand::PythonEnvironment { .. });
        let rust_request = match &command {
            LanguageCommand::CodeActions { path, version, .. }
            | LanguageCommand::InlayHints { path, version, .. } => Some((path.clone(), *version)),
            LanguageCommand::ResolveCodeAction { token } => Some(
                self.code_action_tokens
                    .get(token)
                    .cloned()
                    .ok_or("Ação de código obsoleta ou desconhecida")?,
            ),
            _ => None,
        };
        if let Some((path, version)) = &rust_request
            && (Language::for_path(path).is_none()
                || (matches!(&command, LanguageCommand::InlayHints { .. })
                    && !matches!(
                        Language::for_path(path),
                        Some(Language::Rust | Language::Python)
                    ))
                || self.versions.get(path) != Some(version))
        {
            return Err("Recurso exige documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::AnswerServerChoice { token, index } = &command {
            let choice = self
                .server_choices
                .iter()
                .find(|c| c.token == *token)
                .ok_or("Escolha do provider já encerrada")?;
            if index.is_some_and(|i| i >= choice.items.len()) {
                return Err("Opção do provider fora do intervalo".into());
            }
        }
        if let LanguageCommand::Formatting {
            path,
            version,
            tab_size,
            ..
        } = &command
        {
            if self.versions.get(path) != Some(version) {
                return Err("Formatação exige documento sincronizado na versão atual".into());
            }
            if !(1..=16).contains(tab_size) {
                return Err("Indentação de formatação inválida".into());
            }
        }
        if let LanguageCommand::SignatureHelp {
            path,
            version,
            trigger,
            ..
        } = &command
        {
            if self.versions.get(path) != Some(version) {
                return Err("Assinatura exige documento sincronizado na versão atual".into());
            }
            if let SignatureHelpTrigger::Character(ch) = trigger
                && ch.chars().count() != 1
            {
                return Err("Gatilho de assinatura inválido".into());
            }
        }
        if let LanguageCommand::PrepareRename { path, version, .. }
        | LanguageCommand::Rename { path, version, .. } = &command
            && self.versions.get(path) != Some(version)
        {
            return Err("Rename exige documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::RenameClosed { path, text, .. } = &command {
            if self.versions.contains_key(path) {
                return Err("Snapshot de rename exige documento fechado".into());
            }
            if text.len() > 1024 * 1024 || text.contains('\0') {
                return Err("Snapshot de rename inválido ou acima de 1 MiB".into());
            }
        }
        if let LanguageCommand::Rename { new_name, .. }
        | LanguageCommand::RenameClosed { new_name, .. } = &command
        {
            rename::new_name(new_name)?;
        }
        if let LanguageCommand::References { path, version, .. } = &command
            && self.versions.get(path) != Some(version)
        {
            return Err("Referências exigem documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::Completion { path, version, .. } = &command
            && self.versions.get(path) != Some(version)
        {
            return Err("Completion exige documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::Definition { path, version, .. }
        | LanguageCommand::Implementation { path, version, .. }
        | LanguageCommand::SuperDefinition { path, version, .. }
        | LanguageCommand::TypeDefinition { path, version, .. } = &command
            && self.versions.get(path) != Some(version)
        {
            return Err("Definição exige documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::Hover { path, version, .. } = &command
            && self.versions.get(path) != Some(version)
        {
            return Err("Hover exige documento sincronizado na versão atual".into());
        }
        if let LanguageCommand::Sync {
            path,
            version,
            text,
        } = &command
        {
            if *version > i32::MAX as u64 {
                return Err("LSP: versão excede limite integer do protocolo".into());
            }
            if text.len() > 1024 * 1024 {
                return Err("LSP: documento excede 1 MiB".into());
            }
            if self.versions.len() >= 32 && !self.versions.contains_key(path) {
                return Err("LSP: limite de 32 documentos".into());
            }
            if self.versions.get(path).is_some_and(|old| old > version) {
                return Err("LSP: versão obsoleta".into());
            }
        }
        if let LanguageCommand::Start { configs, .. } = &command {
            if configs.is_empty() || configs.len() > 4 {
                return Err("Configure de um a quatro providers".into());
            }
            let mut languages = HashSet::new();
            for config in configs {
                if config.args.len() > 64
                    || config.args.iter().map(String::len).sum::<usize>() > 32768
                    || config.program.as_os_str().len() > 4096
                {
                    return Err("LSP: configuração excede limites de executável/argumentos".into());
                }
                if !languages.insert(config.language) {
                    return Err("Provider de linguagem duplicado".into());
                }
            }
        }
        let changes_epoch = matches!(
            &command,
            LanguageCommand::Start { .. } | LanguageCommand::Stop
        );
        let epoch = self.epoch + u64::from(changes_epoch);
        let id = self.next;
        let remembered = match &command {
            LanguageCommand::Sync { path, version, .. } => Some((path.clone(), Some(*version))),
            LanguageCommand::Close { path } => Some((path.clone(), None)),
            _ => None,
        };
        let formatting_request = if let LanguageCommand::Formatting { path, version, .. } = &command
        {
            Some((id, path.clone(), *version))
        } else {
            None
        };
        let invalidates_formatting = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::Formatting { .. } | LanguageCommand::CancelFormatting
            );
        let signature_request = if let LanguageCommand::SignatureHelp {
            path,
            version,
            line,
            character,
            ..
        } = &command
        {
            Some((id, path.clone(), *version, *line, *character))
        } else {
            None
        };
        let invalidates_signature = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::SignatureHelp { .. } | LanguageCommand::CancelSignatureHelp
            );
        let hover_request = if let LanguageCommand::Hover { path, version, .. } = &command {
            Some((id, path.clone(), *version))
        } else {
            None
        };
        let invalidates_hover = changes_epoch
            || matches!(
                &command,
                LanguageCommand::CancelHover | LanguageCommand::Hover { .. }
            )
            || remembered.as_ref().is_some_and(|(path, _)| {
                self.current_hover
                    .as_ref()
                    .is_some_and(|(_, p, _)| p == path)
                    || self.hover.as_ref().is_some_and(|h| &h.path == path)
            });
        let definition_request = if let LanguageCommand::Definition { path, version, .. }
        | LanguageCommand::Implementation { path, version, .. }
        | LanguageCommand::SuperDefinition { path, version, .. }
        | LanguageCommand::TypeDefinition { path, version, .. } =
            &command
        {
            Some((id, path.clone(), *version))
        } else {
            None
        };
        // Definitions can depend on any open buffer, including the target. A
        // change/close anywhere invalidates the entire unsaved snapshot context.
        let invalidates_definition = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::Definition { .. }
                    | LanguageCommand::Implementation { .. }
                    | LanguageCommand::SuperDefinition { .. }
                    | LanguageCommand::TypeDefinition { .. }
                    | LanguageCommand::CancelDefinition
            );
        let completion_request = if let LanguageCommand::Completion { path, version, .. } = &command
        {
            Some((id, path.clone(), *version))
        } else {
            None
        };
        let invalidates_completion = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::Completion { .. } | LanguageCommand::CancelCompletion
            );
        let references_request = if let LanguageCommand::References { path, version, .. } = &command
        {
            Some((id, path.clone(), *version))
        } else {
            None
        };
        let invalidates_references = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::References { .. } | LanguageCommand::CancelReferences
            );
        let rename_request = match &command {
            LanguageCommand::PrepareRename { path, version, .. }
            | LanguageCommand::Rename { path, version, .. } => {
                Some((id, path.clone(), Some(*version)))
            }
            LanguageCommand::RenameClosed { path, .. } => Some((id, path.clone(), None)),
            _ => None,
        };
        let invalidates_rename = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::PrepareRename { .. }
                    | LanguageCommand::Rename { .. }
                    | LanguageCommand::RenameClosed { .. }
                    | LanguageCommand::CancelRename
            );
        let start_configs = if let LanguageCommand::Start { configs, .. } = &command {
            Some(configs.clone())
        } else {
            None
        };
        let answering = if let LanguageCommand::AnswerServerChoice { token, .. } = &command {
            Some(*token)
        } else {
            None
        };
        let actions_request = matches!(
            &command,
            LanguageCommand::CodeActions { .. } | LanguageCommand::ResolveCodeAction { .. }
        );
        let hints_request = matches!(&command, LanguageCommand::InlayHints { .. });
        let invalidate_actions = changes_epoch
            || remembered.is_some()
            || matches!(
                &command,
                LanguageCommand::CodeActions { .. } | LanguageCommand::CancelCodeActions
            );
        let invalidate_hints = changes_epoch
            || remembered.is_some()
            || hints_request
            || matches!(&command, LanguageCommand::CancelInlayHints);
        self.sender
            .try_send(Request { id, epoch, command })
            .map_err(|_| "Fila LSP cheia ou worker indisponível".to_string())?;
        if python_environment_change {
            self.invalidate_python_environment();
        }
        if invalidate_actions {
            self.code_actions = None;
            self.code_action_edit = None;
            self.code_actions_error = None;
            self.current_code_actions = None;
            self.code_action_tokens.clear();
        }
        if actions_request {
            self.code_action_edit = None;
            self.code_actions_error = None;
            self.current_code_actions = rust_request
                .clone()
                .map(|(path, version)| (id, path, version));
        }
        if invalidate_hints {
            self.inlay_hints = None;
            self.inlay_hints_error = None;
            self.current_inlay_hints = if hints_request {
                rust_request.map(|(path, version)| (id, path, version))
            } else {
                None
            };
        }
        if let Some(token) = answering {
            self.server_choices.retain(|choice| choice.token != token);
        } else {
            self.server_choices.clear();
        }
        if invalidates_formatting {
            self.formatting = None;
            self.formatting_error = None;
            self.current_formatting = formatting_request;
        }
        if invalidates_signature {
            self.signature_help = None;
            self.signature_help_error = None;
            self.current_signature_help = signature_request;
        }
        if invalidates_rename {
            self.prepare_rename = None;
            self.rename = None;
            self.rename_error = None;
            self.current_rename = rename_request;
        }
        if invalidates_references {
            self.references = None;
            self.references_error = None;
            self.current_references = references_request;
        }
        if invalidates_completion {
            self.completion = None;
            self.completion_error = None;
            self.current_completion = completion_request;
        }
        if invalidates_definition {
            self.definition = None;
            self.definition_error = None;
            self.current_definition = definition_request;
        }
        if invalidates_hover {
            self.hover = None;
            self.hover_error = None;
            self.current_hover = hover_request;
        }
        self.next += 1;
        self.pending.insert(id);
        self.last_error = None;
        if changes_epoch {
            self.semantic_runtime = start_configs.as_ref().and_then(|configs| {
                configs
                    .iter()
                    .find_map(semantic::TypeScriptRuntime::from_config)
            });
            self.epoch = epoch;
            self.diagnostics.clear();
            self.analyzed_versions.clear();
            self.unversioned_paths.clear();
            self.sync_failures.clear();
            self.versions.clear();
            self.statuses.clear();
            self.rust_capabilities.clear();
            self.signature_help_capabilities.clear();
            self.formatting_capabilities.clear();
        }
        if let Some(configs) = start_configs {
            self.statuses = configs
                .into_iter()
                .map(|c| ProviderStatus {
                    language: c.language,
                    state: ProviderState::Starting,
                    detail: "Inicializando processo LSP".into(),
                })
                .collect();
        }
        if let Some((path, version)) = remembered {
            self.diagnostics.retain(|d| d.path != path);
            self.analyzed_versions.remove(&path);
            self.unversioned_paths.remove(&path);
            self.sync_failures.remove(&path);
            if let Some(version) = version {
                self.versions.insert(path, version);
            } else {
                self.versions.remove(&path);
            }
        }
        Ok(())
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(envelope) = self.receiver.try_recv() {
            if let Event::Ack(id) = envelope.event {
                self.pending.remove(&id);
                changed = true;
                continue;
            }
            if envelope.epoch != self.epoch {
                continue;
            }
            changed = true;
            match envelope.event {
                Event::RustCapabilities {
                    language,
                    capabilities,
                } => {
                    self.rust_capabilities.insert(language, capabilities);
                }
                Event::CodeActions { id, result } => {
                    let Some((current, path, version)) = &self.current_code_actions else {
                        continue;
                    };
                    if *current != id || self.versions.get(path) != Some(version) {
                        continue;
                    }
                    if result
                        .as_ref()
                        .is_ok_and(|r| &r.path != path || r.version != *version)
                    {
                        continue;
                    }
                    self.current_code_actions = None;
                    match result {
                        Ok(result) => {
                            for item in &result.items {
                                self.code_action_tokens
                                    .insert(item.token, (result.path.clone(), result.version));
                            }
                            self.code_actions = Some(result);
                        }
                        Err(error) => self.code_actions_error = Some(error),
                    }
                }
                Event::CodeActionEdit { id, result } => {
                    let Some((current, path, version)) = &self.current_code_actions else {
                        continue;
                    };
                    if *current != id || self.versions.get(path) != Some(version) {
                        continue;
                    }
                    if result
                        .as_ref()
                        .is_ok_and(|r| &r.path != path || r.version != *version)
                    {
                        continue;
                    }
                    self.current_code_actions = None;
                    match result {
                        Ok(result) => self.code_action_edit = Some(result),
                        Err(error) => self.code_actions_error = Some(error),
                    }
                }
                Event::InlayHints { id, result } => {
                    let Some((current, path, version)) = &self.current_inlay_hints else {
                        continue;
                    };
                    if *current != id || self.versions.get(path) != Some(version) {
                        continue;
                    }
                    if result
                        .as_ref()
                        .is_ok_and(|r| &r.path != path || r.version != *version)
                    {
                        continue;
                    }
                    self.current_inlay_hints = None;
                    match result {
                        Ok(result) => self.inlay_hints = Some(result),
                        Err(error) => self.inlay_hints_error = Some(error),
                    }
                }
                Event::ServerChoice(choice) => {
                    if self.server_choice_owner_current(choice.owner) {
                        self.server_choices
                            .retain(|c| c.language != choice.language);
                        if self.server_choices.len() < 2 {
                            self.server_choices.push(choice);
                        }
                    }
                }
                Event::ServerChoiceClosed(token) => {
                    self.server_choices.retain(|c| c.token != token)
                }
                Event::FormattingCapabilities {
                    language,
                    capabilities,
                } => {
                    self.formatting_capabilities.insert(language, capabilities);
                }
                Event::Formatting { id, result } => {
                    let Some((current, path, version)) = &self.current_formatting else {
                        continue;
                    };
                    if *current != id || self.versions.get(path) != Some(version) {
                        continue;
                    }
                    if result
                        .as_ref()
                        .is_ok_and(|r| &r.path != path || r.version != *version)
                    {
                        continue;
                    }
                    self.current_formatting = None;
                    match result {
                        Ok(r) => self.formatting = Some(r),
                        Err(e) => self.formatting_error = Some(e),
                    }
                }
                Event::SignatureHelpCapabilities {
                    language,
                    capabilities,
                } => {
                    self.signature_help_capabilities
                        .insert(language, capabilities);
                }
                Event::SignatureHelp { id, result } => {
                    let Some((current, path, version, line, character)) =
                        &self.current_signature_help
                    else {
                        continue;
                    };
                    if *current != id || self.versions.get(path) != Some(version) {
                        continue;
                    }
                    if let Ok(result) = &result
                        && (&result.path != path
                            || result.version != *version
                            || result.line != *line
                            || result.character != *character)
                    {
                        continue;
                    }
                    self.current_signature_help = None;
                    match result {
                        Ok(result) => self.signature_help = Some(result),
                        Err(error) => self.signature_help_error = Some(error),
                    }
                }

                Event::Hover { id, result } => {
                    if self
                        .current_hover
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path) != Some(version)
                        })
                    {
                        continue;
                    }
                    self.current_hover = None;
                    match result {
                        Ok(result) => self.hover = Some(result),
                        Err(error) => self.hover_error = Some(error),
                    }
                }
                Event::Definition { id, result } => {
                    if self
                        .current_definition
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path) != Some(version)
                        })
                    {
                        continue;
                    }
                    self.current_definition = None;
                    match result {
                        Ok(result) => self.definition = Some(result),
                        Err(error) => self.definition_error = Some(error),
                    }
                }
                Event::Completion { id, result } => {
                    if self
                        .current_completion
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path) != Some(version)
                        })
                    {
                        continue;
                    }
                    self.current_completion = None;
                    match result {
                        Ok(result) => self.completion = Some(result),
                        Err(error) => self.completion_error = Some(error),
                    }
                }
                Event::References { id, result } => {
                    if self
                        .current_references
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path) != Some(version)
                        })
                    {
                        continue;
                    }
                    self.current_references = None;
                    match result {
                        Ok(result) => self.references = Some(result),
                        Err(error) => self.references_error = Some(error),
                    }
                }
                Event::PrepareRename { id, result } => {
                    if self
                        .current_rename
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path).copied() != *version
                        })
                    {
                        continue;
                    }
                    self.current_rename = None;
                    match result {
                        Ok(result) => self.prepare_rename = Some(result),
                        Err(error) => self.rename_error = Some(error),
                    }
                }
                Event::Rename { id, result } => {
                    if self
                        .current_rename
                        .as_ref()
                        .is_none_or(|(current, path, version)| {
                            *current != id || self.versions.get(path).copied() != *version
                        })
                    {
                        continue;
                    }
                    self.current_rename = None;
                    match result {
                        Ok(result) => self.rename = Some(result),
                        Err(error) => self.rename_error = Some(error),
                    }
                }
                Event::Status(status) => {
                    if matches!(status.state, ProviderState::Failed | ProviderState::Stopped) {
                        self.server_choices
                            .retain(|c| c.language != status.language);
                        self.rust_capabilities.remove(&status.language);
                        if self
                            .code_actions
                            .as_ref()
                            .is_some_and(|r| Language::for_path(&r.path) == Some(status.language))
                            || self.current_code_actions.as_ref().is_some_and(|(_, p, _)| {
                                Language::for_path(p) == Some(status.language)
                            })
                        {
                            self.code_actions = None;
                            self.code_action_edit = None;
                            self.code_action_tokens.clear();
                            if self.current_code_actions.take().is_some() {
                                self.code_actions_error =
                                    Some("Provider de correções indisponível".into());
                            }
                        }
                        if matches!(status.language, Language::Rust | Language::Python)
                            && (self.inlay_hints.as_ref().is_some_and(|r| {
                                Language::for_path(&r.path) == Some(status.language)
                            }) || self
                                .current_inlay_hints
                                .as_ref()
                                .is_some_and(|r| Language::for_path(&r.1) == Some(status.language)))
                        {
                            self.inlay_hints = None;
                            if self.current_inlay_hints.take().is_some() {
                                self.inlay_hints_error =
                                    Some("Provider de inlay hints indisponível".into());
                            }
                        }
                        self.signature_help_capabilities.remove(&status.language);
                        self.formatting_capabilities.remove(&status.language);
                        if self
                            .current_formatting
                            .as_ref()
                            .is_some_and(|(_, p, _)| Language::for_path(p) == Some(status.language))
                        {
                            self.current_formatting = None;
                            self.formatting = None;
                            self.formatting_error =
                                Some("Provider de formatação indisponível".into());
                        }
                        if self
                            .signature_help
                            .as_ref()
                            .is_some_and(|r| Language::for_path(&r.path) == Some(status.language))
                        {
                            self.signature_help = None;
                        }

                        if self
                            .prepare_rename
                            .as_ref()
                            .is_some_and(|r| Language::for_path(&r.path) == Some(status.language))
                        {
                            self.prepare_rename = None;
                        }
                        if self
                            .rename
                            .as_ref()
                            .is_some_and(|r| Language::for_path(&r.path) == Some(status.language))
                        {
                            self.rename = None;
                        }
                        if self
                            .references
                            .as_ref()
                            .is_some_and(|r| Language::for_path(&r.path) == Some(status.language))
                        {
                            self.references = None;
                        }
                        if self
                            .completion
                            .as_ref()
                            .is_some_and(|c| Language::for_path(&c.path) == Some(status.language))
                        {
                            self.completion = None;
                        }
                        if self
                            .definition
                            .as_ref()
                            .is_some_and(|d| Language::for_path(&d.path) == Some(status.language))
                        {
                            self.definition = None;
                        }
                        if self
                            .hover
                            .as_ref()
                            .is_some_and(|h| Language::for_path(&h.path) == Some(status.language))
                        {
                            self.hover = None;
                        }
                        self.diagnostics.retain(|d| d.language != status.language);
                        self.analyzed_versions
                            .retain(|path, _| Language::for_path(path) != Some(status.language));
                        self.unversioned_paths
                            .retain(|path| Language::for_path(path) != Some(status.language));
                    }
                    self.statuses.retain(|s| s.language != status.language);
                    self.statuses.push(status);
                }
                Event::Error(error) => self.last_error = Some(error),
                Event::SyncFailed { path, failure } => {
                    if self.versions.get(&path) == Some(&failure.version) {
                        self.last_error = Some(failure.message.clone());
                        self.sync_failures.insert(path, failure);
                    }
                }
                Event::Diagnostics {
                    path,
                    version,
                    unversioned,
                    items,
                } => {
                    if self.versions.get(&path) != Some(&version) {
                        continue;
                    }
                    self.diagnostics.retain(|d| d.path != path);
                    self.diagnostics.extend(items);
                    if unversioned {
                        self.unversioned_paths.insert(path.clone());
                    } else {
                        self.unversioned_paths.remove(&path);
                    }
                    self.analyzed_versions.insert(path, version);
                }
                Event::Ack(_) => unreachable!(),
            }
        }
        changed
    }
    pub fn code_actions_pending(&self) -> bool {
        self.current_code_actions.is_some()
    }
    /// Background queries must not block normal Save/edit commands.
    pub fn inlay_hints_pending(&self) -> bool {
        self.current_inlay_hints.is_some()
    }
    pub fn formatting_pending(&self) -> bool {
        self.current_formatting.is_some()
    }
    pub fn signature_help_pending(&self) -> bool {
        self.current_signature_help.is_some()
    }
    pub fn hover_pending(&self) -> bool {
        self.current_hover.is_some()
    }
    pub fn definition_pending(&self) -> bool {
        self.current_definition.is_some()
    }
    pub fn completion_pending(&self) -> bool {
        self.current_completion.is_some()
    }
    pub fn references_pending(&self) -> bool {
        self.current_references.is_some()
    }
    pub fn rename_pending(&self) -> bool {
        self.current_rename.is_some()
    }
    pub fn is_busy(&self) -> bool {
        self.code_actions_pending()
            || self.formatting_pending()
            || self.signature_help_pending()
            || self.rename_pending()
            || self.references_pending()
            || self.completion_pending()
            || self.definition_pending()
            || self.hover_pending()
            || !self.pending.is_empty()
            || self
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Starting)
    }
    /// Accepted Start/Stop generation, independent of document and file jobs.
    pub fn lifecycle_generation(&self) -> u64 {
        self.epoch
    }
}
impl Drop for LanguageService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Normal application shutdown awaits Stop. Fallback destruction must
            // not block a UI thread on child I/O or shutdown deadlines.
            thread::spawn(move || {
                let _ = worker.join();
            });
        }
    }
}
/// Convert LSP UTF-16 column into a UTF-8 byte boundary. Reject surrogate halves.
pub fn utf16_byte_offset(text: &str, line: u32, column: u32) -> Option<usize> {
    let mut start = 0;
    for _ in 0..line {
        start += text.get(start..)?.find('\n')? + 1;
    }
    let tail = text
        .get(start..)?
        .split('\n')
        .next()?
        .trim_end_matches('\r');
    let mut units = 0;
    for (byte, ch) in tail.char_indices() {
        if units == column {
            return Some(start + byte);
        }
        units += ch.len_utf16() as u32;
        if units > column {
            return None;
        }
    }
    (units == column).then_some(start + tail.len())
}

#[cfg(test)]
mod hover_service_tests {
    use super::*;
    #[test]
    fn compiler_runtime_tracks_only_accepted_start_and_is_revoked_on_stop() {
        let (mut service, _requests, _events) = isolated();
        assert!(service.typescript_runtime().is_none());
        service
            .dispatch(LanguageCommand::Start {
                workspace: "/project".into(),
                configs: vec![ProviderConfig {
                    language: Language::TypeScript,
                    program: "/opt/node".into(),
                    args: vec![
                        "/opt/node_modules/typescript-language-server/lib/cli.mjs".into(),
                        "--stdio".into(),
                    ],
                }],
            })
            .unwrap();
        assert!(service.typescript_runtime().is_some());
        service.dispatch(LanguageCommand::Stop).unwrap();
        assert!(service.typescript_runtime().is_none());
    }
    fn isolated() -> (LanguageService, Receiver<Request>, SyncSender<Envelope>) {
        let (sender, requests) = mpsc::sync_channel(32);
        let (events, receiver) = mpsc::sync_channel(128);
        (
            LanguageService {
                rust_capabilities: HashMap::new(),
                code_actions: None,
                code_action_edit: None,
                code_actions_error: None,
                current_code_actions: None,
                code_action_tokens: HashMap::new(),
                inlay_hints: None,
                inlay_hints_error: None,
                current_inlay_hints: None,
                server_choices: Vec::new(),
                formatting: None,
                formatting_error: None,
                formatting_capabilities: HashMap::new(),
                current_formatting: None,
                semantic_runtime: None,
                signature_help: None,
                signature_help_error: None,
                signature_help_capabilities: HashMap::new(),
                current_signature_help: None,
                prepare_rename: None,
                rename: None,
                rename_error: None,
                current_rename: None,
                references: None,
                references_error: None,
                current_references: None,
                completion: None,
                completion_error: None,
                current_completion: None,
                definition: None,
                definition_error: None,
                current_definition: None,
                hover: None,
                hover_error: None,
                current_hover: None,
                diagnostics: vec![],
                analyzed_versions: HashMap::new(),
                unversioned_paths: HashSet::new(),
                sync_failures: HashMap::new(),
                statuses: vec![],
                last_error: None,
                sender,
                receiver,
                pending: HashSet::new(),
                versions: HashMap::new(),
                epoch: 0,
                next: 1,
                stop: Arc::new(AtomicBool::new(false)),
                worker: None,
            },
            requests,
            events,
        )
    }
    fn hover(path: &std::path::Path, version: u64) -> LanguageCommand {
        LanguageCommand::Hover {
            path: path.into(),
            version,
            line: 0,
            character: 0,
        }
    }
    fn reply(id: u64, epoch: u64, path: &std::path::Path, version: u64) -> Envelope {
        Envelope {
            epoch,
            event: Event::Hover {
                id,
                result: Ok(HoverResult {
                    path: path.into(),
                    version,
                    line: 0,
                    character: 0,
                    text: "result".into(),
                }),
            },
        }
    }
    #[test]
    fn hover_discards_out_of_order_cancelled_edited_closed_and_old_session_results() {
        let (mut service, requests, events) = isolated();
        let path = PathBuf::from("/fixture.ts");
        service.versions.insert(path.clone(), 1);
        service.dispatch(hover(&path, 1)).unwrap();
        let first = requests.recv().unwrap();
        service.dispatch(hover(&path, 1)).unwrap();
        let second = requests.recv().unwrap();
        events.send(reply(first.id, 0, &path, 1)).unwrap();
        service.poll();
        assert!(service.hover.is_none());
        assert!(service.hover_pending());
        events.send(reply(second.id, 0, &path, 1)).unwrap();
        service.poll();
        assert_eq!(service.hover.as_ref().unwrap().text, "result");
        assert!(!service.hover_pending());
        service.dispatch(hover(&path, 1)).unwrap();
        let third = requests.recv().unwrap();
        service
            .dispatch(LanguageCommand::Sync {
                path: path.clone(),
                version: 2,
                text: "new".into(),
            })
            .unwrap();
        requests.recv().unwrap();
        events.send(reply(third.id, 0, &path, 1)).unwrap();
        service.poll();
        assert!(service.hover.is_none());
        assert!(!service.hover_pending());
        for invalidate in [
            LanguageCommand::CancelHover,
            LanguageCommand::Close { path: path.clone() },
        ] {
            service.dispatch(hover(&path, 2)).unwrap();
            let request = requests.recv().unwrap();
            service.dispatch(invalidate).unwrap();
            requests.recv().unwrap();
            events.send(reply(request.id, 0, &path, 2)).unwrap();
            service.poll();
            assert!(service.hover.is_none());
        }
        service.versions.insert(path.clone(), 3);
        service.dispatch(hover(&path, 3)).unwrap();
        let request = requests.recv().unwrap();
        service.dispatch(LanguageCommand::Stop).unwrap();
        events.send(reply(request.id, 0, &path, 3)).unwrap();
        service.poll();
        assert!(service.hover.is_none());
    }
    #[test]
    fn hover_reports_matching_errors_only_and_rejects_unsynchronized_version() {
        let (mut service, requests, events) = isolated();
        let path = PathBuf::from("/fixture.php");
        assert!(service.dispatch(hover(&path, 1)).is_err());
        service.versions.insert(path.clone(), 1);
        service.dispatch(hover(&path, 1)).unwrap();
        let request = requests.recv().unwrap();
        events
            .send(Envelope {
                epoch: 0,
                event: Event::Hover {
                    id: request.id + 1,
                    result: Err("obsolete".into()),
                },
            })
            .unwrap();
        service.poll();
        assert!(service.hover_error.is_none());
        assert!(service.hover_pending());
        events
            .send(Envelope {
                epoch: 0,
                event: Event::Hover {
                    id: request.id,
                    result: Err("timeout".into()),
                },
            })
            .unwrap();
        service.poll();
        assert_eq!(service.hover_error.as_deref(), Some("timeout"));
        assert!(!service.hover_pending());
    }
    fn definition(path: &std::path::Path, version: u64) -> LanguageCommand {
        LanguageCommand::Definition {
            path: path.into(),
            version,
            line: 0,
            character: 0,
        }
    }
    fn definition_reply(id: u64, epoch: u64, path: &std::path::Path, version: u64) -> Envelope {
        Envelope {
            epoch,
            event: Event::Definition {
                id,
                result: Ok(DefinitionResult {
                    presentation: None,
                    path: path.into(),
                    version,
                    line: 0,
                    character: 0,
                    targets: vec![],
                }),
            },
        }
    }
    #[test]
    fn definition_discards_reordered_responses_and_any_changed_buffer_context() {
        let (mut service, requests, events) = isolated();
        let source = PathBuf::from("/source.ts");
        let target = PathBuf::from("/target.ts");
        service.versions.insert(source.clone(), 1);
        assert!(service.dispatch(definition(&source, 2)).is_err());
        service.dispatch(definition(&source, 1)).unwrap();
        let first = requests.recv().unwrap();
        service.dispatch(definition(&source, 1)).unwrap();
        let second = requests.recv().unwrap();
        events
            .send(definition_reply(first.id, 0, &source, 1))
            .unwrap();
        service.poll();
        assert!(service.definition.is_none());
        assert!(service.definition_pending());
        events
            .send(definition_reply(second.id, 0, &source, 1))
            .unwrap();
        service.poll();
        assert!(service.definition.is_some());
        assert!(!service.definition_pending());
        for invalidate in [
            LanguageCommand::Sync {
                path: target,
                version: 3,
                text: "changed target".into(),
            },
            LanguageCommand::CancelDefinition,
            LanguageCommand::Close {
                path: source.clone(),
            },
            LanguageCommand::Stop,
        ] {
            service.versions.insert(source.clone(), 1);
            service.dispatch(definition(&source, 1)).unwrap();
            let request = requests.recv().unwrap();
            service.dispatch(invalidate).unwrap();
            requests.recv().unwrap();
            events
                .send(definition_reply(request.id, 0, &source, 1))
                .unwrap();
            service.poll();
            assert!(service.definition.is_none());
            assert!(!service.definition_pending());
        }
    }
    #[test]
    fn definition_rejects_errors_of_old_ids_and_delivers_matching_error() {
        let (mut service, requests, events) = isolated();
        let path = PathBuf::from("/source.php");
        service.versions.insert(path.clone(), 1);
        service.dispatch(definition(&path, 1)).unwrap();
        let request = requests.recv().unwrap();
        for (id, message) in [(request.id + 1, "obsolete"), (request.id, "provider error")] {
            events
                .send(Envelope {
                    epoch: 0,
                    event: Event::Definition {
                        id,
                        result: Err(message.into()),
                    },
                })
                .unwrap();
            service.poll();
            if id != request.id {
                assert!(service.definition_error.is_none());
                assert!(service.definition_pending());
            }
        }
        assert_eq!(service.definition_error.as_deref(), Some("provider error"));
        assert!(!service.definition_pending());
    }
    fn completion(path: &std::path::Path, version: u64) -> LanguageCommand {
        LanguageCommand::Completion {
            path: path.into(),
            version,
            line: 0,
            character: 0,
        }
    }
    fn completion_reply(id: u64, epoch: u64, path: &std::path::Path, version: u64) -> Envelope {
        Envelope {
            epoch,
            event: Event::Completion {
                id,
                result: Ok(CompletionResult {
                    path: path.into(),
                    version,
                    line: 0,
                    character: 0,
                    items: vec![],
                    is_incomplete: false,
                    truncated: false,
                }),
            },
        }
    }
    #[test]
    fn completion_filters_old_ids_errors_epochs_and_any_changed_buffer_context() {
        let (mut s, requests, events) = isolated();
        let path = PathBuf::from("/source.ts");
        s.versions.insert(path.clone(), 1);
        assert!(s.dispatch(completion(&path, 2)).is_err());
        s.dispatch(completion(&path, 1)).unwrap();
        let first = requests.recv().unwrap();
        s.dispatch(completion(&path, 1)).unwrap();
        let second = requests.recv().unwrap();
        events
            .send(completion_reply(first.id, 0, &path, 1))
            .unwrap();
        s.poll();
        assert!(s.completion_pending());
        assert!(s.completion.is_none());
        events
            .send(completion_reply(second.id, 0, &path, 1))
            .unwrap();
        s.poll();
        assert!(s.completion.is_some());
        for invalidate in [
            LanguageCommand::Sync {
                path: "/dependency.ts".into(),
                version: 1,
                text: "changed".into(),
            },
            LanguageCommand::CancelCompletion,
            LanguageCommand::Close { path: path.clone() },
            LanguageCommand::Stop,
        ] {
            s.versions.insert(path.clone(), 1);
            s.dispatch(completion(&path, 1)).unwrap();
            let request = requests.recv().unwrap();
            s.dispatch(invalidate).unwrap();
            requests.recv().unwrap();
            events
                .send(completion_reply(request.id, 0, &path, 1))
                .unwrap();
            s.poll();
            assert!(s.completion.is_none());
            assert!(!s.completion_pending());
        }
    }
    #[test]
    fn completion_surfaces_matching_errors_only() {
        let (mut s, requests, events) = isolated();
        let path = PathBuf::from("/source.php");
        s.versions.insert(path.clone(), 1);
        s.dispatch(completion(&path, 1)).unwrap();
        let request = requests.recv().unwrap();
        for (id, err) in [(request.id + 1, "old"), (request.id, "failed")] {
            events
                .send(Envelope {
                    epoch: 0,
                    event: Event::Completion {
                        id,
                        result: Err(err.into()),
                    },
                })
                .unwrap();
            s.poll();
            if id != request.id {
                assert!(s.completion_error.is_none());
                assert!(s.completion_pending());
            }
        }
        assert_eq!(s.completion_error.as_deref(), Some("failed"));
        assert!(!s.completion_pending());
    }
    fn references(path: &std::path::Path, version: u64) -> LanguageCommand {
        LanguageCommand::References {
            path: path.into(),
            version,
            line: 0,
            character: 0,
            include_declaration: false,
        }
    }
    fn references_reply(id: u64, epoch: u64, path: &std::path::Path, version: u64) -> Envelope {
        Envelope {
            epoch,
            event: Event::References {
                id,
                result: Ok(ReferencesResult {
                    path: path.into(),
                    version,
                    line: 0,
                    character: 0,
                    targets: vec![],
                    truncated: false,
                }),
            },
        }
    }
    #[test]
    fn references_discards_old_ids_epochs_versions_and_all_buffer_changes() {
        let (mut s, requests, events) = isolated();
        let path = PathBuf::from("/source.ts");
        s.versions.insert(path.clone(), 1);
        assert!(s.dispatch(references(&path, 2)).is_err());
        s.dispatch(references(&path, 1)).unwrap();
        let first = requests.recv().unwrap();
        s.dispatch(references(&path, 1)).unwrap();
        let second = requests.recv().unwrap();
        events
            .send(references_reply(first.id, 0, &path, 1))
            .unwrap();
        s.poll();
        assert!(s.references_pending());
        assert!(s.references.is_none());
        events
            .send(references_reply(second.id, 0, &path, 1))
            .unwrap();
        s.poll();
        assert!(s.references.is_some());
        for invalidate in [
            LanguageCommand::Sync {
                path: "/dependency.ts".into(),
                version: 1,
                text: "updated".into(),
            },
            LanguageCommand::CancelReferences,
            LanguageCommand::Close { path: path.clone() },
            LanguageCommand::Stop,
        ] {
            s.versions.insert(path.clone(), 1);
            s.dispatch(references(&path, 1)).unwrap();
            let request = requests.recv().unwrap();
            s.dispatch(invalidate).unwrap();
            requests.recv().unwrap();
            events
                .send(references_reply(request.id, 0, &path, 1))
                .unwrap();
            s.poll();
            assert!(s.references.is_none());
            assert!(!s.references_pending());
        }
    }
    #[test]
    fn references_delivers_only_matching_errors() {
        let (mut s, requests, events) = isolated();
        let path = PathBuf::from("/source.php");
        s.versions.insert(path.clone(), 1);
        s.dispatch(references(&path, 1)).unwrap();
        let request = requests.recv().unwrap();
        for (id, err) in [(request.id + 1, "obsolete"), (request.id, "failed")] {
            events
                .send(Envelope {
                    epoch: 0,
                    event: Event::References {
                        id,
                        result: Err(err.into()),
                    },
                })
                .unwrap();
            s.poll();
            if id != request.id {
                assert!(s.references_error.is_none());
                assert!(s.references_pending());
            }
        }
        assert_eq!(s.references_error.as_deref(), Some("failed"));
        assert!(!s.references_pending());
    }
    fn rename_command(path: &std::path::Path, version: u64, prepare: bool) -> LanguageCommand {
        if prepare {
            LanguageCommand::PrepareRename {
                path: path.into(),
                version,
                line: 0,
                character: 3,
            }
        } else {
            LanguageCommand::Rename {
                path: path.into(),
                version,
                line: 0,
                character: 3,
                new_name: "forecast".into(),
            }
        }
    }
    fn rename_reply(
        id: u64,
        epoch: u64,
        path: &std::path::Path,
        version: u64,
        prepare: bool,
    ) -> Envelope {
        Envelope {
            epoch,
            event: if prepare {
                Event::PrepareRename {
                    id,
                    result: Ok(PrepareRenameResult {
                        path: path.into(),
                        version,
                        line: 0,
                        character: 3,
                        range: None,
                        placeholder: None,
                        used_fallback: true,
                    }),
                }
            } else {
                Event::Rename {
                    id,
                    result: Ok(RenameResult {
                        transient_document_version: None,
                        path: path.into(),
                        version,
                        line: 0,
                        character: 3,
                        new_name: "forecast".into(),
                        changes: vec![],
                    }),
                }
            },
        }
    }
    #[test]
    fn rename_and_prepare_discard_old_ids_epochs_versions_and_any_sync_close_stop() {
        for prepare in [true, false] {
            let (mut s, requests, events) = isolated();
            let p = PathBuf::from("/main.ts");
            s.versions.insert(p.clone(), 1);
            assert!(s.dispatch(rename_command(&p, 2, prepare)).is_err());
            s.dispatch(rename_command(&p, 1, prepare)).unwrap();
            let a = requests.recv().unwrap();
            s.dispatch(rename_command(&p, 1, prepare)).unwrap();
            let b = requests.recv().unwrap();
            events.send(rename_reply(a.id, 0, &p, 1, prepare)).unwrap();
            events.send(rename_reply(b.id, 1, &p, 1, prepare)).unwrap();
            s.poll();
            assert!(s.rename_pending());
            assert!(s.rename.is_none() && s.prepare_rename.is_none());
            events.send(rename_reply(b.id, 0, &p, 1, prepare)).unwrap();
            s.poll();
            assert!(!s.rename_pending());
            assert!(s.rename.is_some() || s.prepare_rename.is_some());
            for command in [
                LanguageCommand::Sync {
                    path: "/dependency.ts".into(),
                    version: 2,
                    text: "changed".into(),
                },
                LanguageCommand::CancelRename,
                LanguageCommand::Close { path: p.clone() },
                LanguageCommand::Stop,
            ] {
                s.versions.insert(p.clone(), 1);
                s.dispatch(rename_command(&p, 1, prepare)).unwrap();
                let r = requests.recv().unwrap();
                s.dispatch(command).unwrap();
                requests.recv().unwrap();
                events
                    .send(rename_reply(r.id, s.epoch, &p, 1, prepare))
                    .unwrap();
                s.poll();
                assert!(!s.rename_pending());
                assert!(s.rename.is_none() && s.prepare_rename.is_none());
            }
        }
    }
    fn signature(path: &std::path::Path) -> LanguageCommand {
        LanguageCommand::SignatureHelp {
            path: path.into(),
            version: 1,
            line: 0,
            character: 3,
            trigger: SignatureHelpTrigger::Invoked,
            is_retrigger: false,
        }
    }
    fn signature_reply(id: u64, epoch: u64, path: &std::path::Path, character: u32) -> Envelope {
        Envelope {
            epoch,
            event: Event::SignatureHelp {
                id,
                result: Ok(SignatureHelpResult {
                    path: path.into(),
                    version: 1,
                    line: 0,
                    character,
                    signatures: vec![],
                    active_signature: None,
                    active_parameter: None,
                    truncated: false,
                }),
            },
        }
    }
    #[test]
    fn signature_correlates_id_epoch_path_version_position_and_context_invalidation() {
        let (mut s, requests, events) = isolated();
        let path = PathBuf::from("/fixture.ts");
        assert!(s.dispatch(signature(&path)).is_err());
        s.versions.insert(path.clone(), 1);
        s.dispatch(signature(&path)).unwrap();
        let old = requests.recv().unwrap();
        s.dispatch(signature(&path)).unwrap();
        let current = requests.recv().unwrap();
        for event in [
            signature_reply(old.id, 0, &path, 3),
            signature_reply(current.id, 1, &path, 3),
            signature_reply(current.id, 0, &path, 4),
            signature_reply(current.id, 0, std::path::Path::new("/wrong.ts"), 3),
        ] {
            events.send(event).unwrap();
            s.poll();
            assert!(s.signature_help_pending());
            assert!(s.signature_help.is_none());
        }
        events
            .send(signature_reply(current.id, 0, &path, 3))
            .unwrap();
        s.poll();
        assert!(s.signature_help.is_some());
        for command in [
            LanguageCommand::Sync {
                path: "/dependency.ts".into(),
                version: 2,
                text: "changed".into(),
            },
            LanguageCommand::CancelSignatureHelp,
            LanguageCommand::Close { path: path.clone() },
            LanguageCommand::Stop,
        ] {
            s.versions.insert(path.clone(), 1);
            s.dispatch(signature(&path)).unwrap();
            let current = requests.recv().unwrap();
            s.dispatch(command).unwrap();
            requests.recv().unwrap();
            events
                .send(signature_reply(current.id, s.epoch, &path, 3))
                .unwrap();
            s.poll();
            assert!(!s.signature_help_pending());
            assert!(s.signature_help.is_none());
        }
    }
}
