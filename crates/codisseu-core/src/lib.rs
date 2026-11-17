//! Shared application service. Filesystem work runs on bounded background workers.
mod ai_workspace;
mod auto_save;
mod browser_debug;
mod browser_devtools;
mod browser_inspector;
pub mod browser_library;
mod browser_login;
mod database_consoles;
mod debug_integration;
pub mod docker;
mod editorconfig;
mod encoding;
mod external_files;
mod git_line_changes;
mod shutdown;
mod workspace_change;
pub mod workspace_file;
pub use encoding::FileEncoding;
pub use workspace_change::WorkspaceChangeBlocker;
pub mod git_commit_message;
mod git_integration;
pub mod js_tests;
pub mod php_tests;
mod project_files;
mod project_language;
mod project_tree;
mod projects;
pub mod python_project;
pub mod python_tests;
pub mod rust_project;
pub use codisseu_api as api;
pub use codisseu_debug as debugger;
pub use codisseu_jira as jira;
pub use codisseu_language::snippets;
pub use project_files::{ProjectFileCommand, ProjectFileState};
pub mod search_everywhere;
pub use codisseu_git as git;
mod shell_layout;
mod view_session;
pub use shell_layout::ShellLayout;
mod views;
pub use codisseu_syntax::{
    RenameEnclosingKind, RenameEnclosingSymbol, RenameOccurrenceKind, RenameOccurrencePresentation,
};
pub use view_session::EditorSplit;
pub use views::{DocumentView, MAX_EDITOR_VIEWS, SourceScrollAnchor, ViewId};
mod code_actions;
mod inlay_hints;
pub use code_actions::CodeActionState;
pub use inlay_hints::InlayHintState;
mod completion;
mod formatting;
pub mod local_history;
mod php_formatting;
mod project_versions;
pub use project_versions::{ProjectRuntime, ProjectVersion, ProjectVersions};
mod python_formatting;
mod signature;
pub use completion::{CompletionSelection, CompletionState};
pub use formatting::FormattingState;
pub use signature::SignatureHelpState;
mod navigation;
mod references;
pub use references::{ReferenceItem, ReferenceState};
mod rename;
pub use codisseu_language::{RenameRange, RenameTextEdit};
pub use codisseu_syntax::RenameSymbolKind;
pub use codisseu_syntax::python_rename::validate_candidate as validate_python_rename_candidate;
pub use codisseu_syntax::rust_rename::validate_candidate as validate_rust_rename_candidate;
pub use codisseu_syntax::{
    Conflict, ConflictKind, ConflictLocation, Coverage, IssueKind, RenameValidation, TextRange,
    ValidationIssue, ValidationScope,
};
pub use codisseu_syntax::{Conflict as RenameConflict, Coverage as RenameCoverage};
pub use codisseu_syntax::{
    RenameOptions, RenameSearchScope, validate_file_rename_name, validate_rename_candidate,
};
pub use rename::inheritors::{RenameInheritorChoice, RenameInheritorKind};
pub use rename::resources::ResourceLocation as RenameResourceMove;
pub use rename::{RenameConflictPreview, RenamePreviewFile, RenameState};
mod search;
pub use navigation::{NavigationPosition, NavigationSelection, NavigationState};
mod provider_startup;
mod worker;
mod worker_compiler;
pub use provider_startup::ProviderStartupStatus;

pub use codisseu_db::mongo_text;
pub use codisseu_db::query_export;
pub use codisseu_db::query_page;
pub use codisseu_db::sql_completion;
pub use codisseu_db::sql_text;
pub use codisseu_db::{DbCommand, DbService, display_value, value_type_name};
pub use codisseu_db::{remote, transfer, transfer_formats_extra, transfer_nosql};
pub use codisseu_syntax::breadcrumbs;
pub use codisseu_syntax::navigation_symbols;
pub use codisseu_syntax::phpdoc;

use codisseu_editor::TextBuffer;
pub use codisseu_editor::{
    EditKind, MAX_EDITOR_DOCUMENT_BYTES, MAX_EDITOR_DOCUMENT_LINES, Selection, TextChange,
    TextDelta, TextSnapshot, next_grapheme, next_word, previous_grapheme, previous_word,
    word_range,
};
pub use codisseu_language::{
    CompletionEdit, CompletionItem, DefinitionTarget, Language, LanguageCommand, LanguageService,
    ProviderConfig, ProviderState, SignatureHelpResult, SignatureHelpTrigger, SignatureInformation,
    SignatureParameter, utf16_byte_offset,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

pub type DocumentId = u64;
pub const MAX_DOCUMENTS: usize = 32;
const QUEUE_SIZE: usize = 32;

/// Identifies the exact external snapshot offered for an explicit resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalDocumentChange {
    pub revision: u64,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct Document {
    pub id: DocumentId,
    pub path: PathBuf,
    pub version: u64,
    buffer: TextBuffer,
    saved_text: Arc<str>,
    saved_identity: Option<u64>,
    dirty_cache: std::cell::Cell<Option<(u64, bool)>>,
    disk_missing: bool,
    untitled: bool,
    encoding: FileEncoding,
    project_root: PathBuf,
    read_only: bool,
}

impl Document {
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }
    pub fn is_untitled(&self) -> bool {
        self.untitled
    }
    pub fn project_root(&self) -> &std::path::Path {
        &self.project_root
    }
    pub fn encoding(&self) -> FileEncoding {
        self.encoding
    }
    pub fn text(&self) -> &str {
        self.buffer.text()
    }
    pub fn snapshot(&self) -> TextSnapshot {
        self.buffer.snapshot()
    }
    pub fn content_identity(&self) -> u64 {
        self.buffer.content_identity()
    }
    pub fn len_chars(&self) -> usize {
        self.buffer.len_chars()
    }
    pub fn len_bytes(&self) -> usize {
        self.buffer.len_bytes()
    }
    pub fn line_count(&self) -> usize {
        self.buffer.line_count()
    }
    pub fn line_to_char(&self, line: usize) -> Option<usize> {
        self.buffer.line_to_char(line)
    }
    pub fn char_to_line(&self, index: usize) -> Option<usize> {
        self.buffer.char_to_line(index)
    }
    pub fn last_change(&self) -> Option<&TextChange> {
        self.buffer.last_change()
    }
    pub fn language_supported(&self) -> bool {
        self.len_bytes() <= codisseu_editor::MAX_DOCUMENT_BYTES
    }
    fn ensure_language_size(&self) -> Result<(), String> {
        if self.language_supported() {
            Ok(())
        } else {
            Err("Documento acima de 1 MiB: editor disponível; inteligência de linguagem e rename indisponíveis".into())
        }
    }
    pub fn selection(&self) -> Selection {
        self.buffer.selection()
    }
    pub fn selection_revision(&self) -> u64 {
        self.buffer.selection_revision()
    }
    pub fn is_dirty(&self) -> bool {
        if self.read_only {
            return false;
        }
        if self.disk_missing {
            return true;
        }
        if self.saved_identity == Some(self.buffer.content_identity()) {
            return false;
        }
        if let Some((revision, dirty)) = self.dirty_cache.get()
            && revision == self.buffer.revision()
        {
            return dirty;
        }
        let dirty = self.buffer.len_bytes() != self.saved_text.len()
            || !self.buffer.matches(&self.saved_text);
        self.dirty_cache.set(Some((self.buffer.revision(), dirty)));
        dirty
    }
    pub fn can_undo(&self) -> bool {
        !self.read_only && self.buffer.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        !self.read_only && self.buffer.can_redo()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: PathBuf,
    pub is_dir: bool,
    pub depth: usize,
}

/// Literal matches in Unicode scalar indices, for one immutable document version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub document_id: DocumentId,
    pub version: u64,
    pub query: String,
    pub ranges: Vec<Range<usize>>,
    pub truncated: bool,
}

#[derive(Debug)]
pub enum Command {
    Browser(browser::Command),
    PhpTests(php_tests::TestCommand),
    JsTests(js_tests::TestCommand),
    Python(python_project::ProjectCommand),
    PythonTests(python_tests::TestCommand),
    SetProjectEncoding(FileEncoding),
    SetFolderEncoding {
        root: PathBuf,
        encoding: FileEncoding,
    },
    Docker(docker::DockerCommand),
    Ai(ai::Command),
    GenerateGitCommitMessage {
        root: PathBuf,
    },
    CancelGitCommitMessage {
        root: Option<PathBuf>,
    },
    ProjectFile(ProjectFileCommand),
    CloseWorkspace,
    Debug(debugger::DebugCommand),
    SaveDebugConfiguration(debugger::DebugConfiguration),
    DeleteDebugConfiguration(String),
    Git(git::GitCommand),
    SelectGitRepository(PathBuf),
    SetGitDraft {
        root: PathBuf,
        message: String,
    },
    SetEditorLigatures(bool),
    SetEditorSoftWrap(bool),
    SetCopilotEnabled(bool),
    SetCopilotProgram(String),
    SetAutoSave {
        enabled: bool,
        delay_ms: u64,
    },
    SetRustInlayHints(bool),
    SetPythonInlayHints(bool),
    SetProjectFollowActiveFile(bool),
    SetShellLayout(ShellLayout),
    SetCommitToolWindowVisible(bool),
    SetGitConfirmActions(bool),
    SetGitInsights {
        inline: bool,
        hover: bool,
        gutter: bool,
    },
    SetEditorSplit(EditorSplit),
    SetViewScroll {
        view: ViewId,
        scroll: [f32; 2],
    },
    SetViewScrollAnchor {
        view: ViewId,
        document: DocumentId,
        anchor: Option<SourceScrollAnchor>,
    },
    CreateView,
    ActivateView {
        view: ViewId,
    },
    ShowDocumentInView {
        view: ViewId,
        id: DocumentId,
    },
    CloseView {
        view: ViewId,
    },
    PrepareFileRename {
        id: DocumentId,
        expected_version: u64,
    },
    PrepareRename {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    SetRenameOptions {
        options: RenameOptions,
    },
    PreviewRename {
        new_name: String,
    },
    UpdateRenameInheritor {
        token: u64,
        index: usize,
        selected: bool,
        new_name: String,
    },
    ConfirmRenameInheritors {
        token: u64,
    },
    EditRenameInheritors {
        token: u64,
    },
    SelectRenameMethod {
        token: u64,
        index: usize,
        selected: bool,
    },
    ConfirmRenameMethods {
        token: u64,
    },
    ApplyRename,
    CancelRename,
    References {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    ChooseReference {
        index: usize,
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    CancelReferences,
    Completion {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    AcceptCompletion {
        index: usize,
        replace: bool,
    },
    CancelCompletion,
    SignatureHelp {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
        trigger: SignatureHelpTrigger,
        is_retrigger: bool,
    },
    CancelSignatureHelp,
    FormatDocument {
        id: DocumentId,
        expected_version: u64,
        range: Option<Range<usize>>,
    },
    CancelFormatting,
    CodeActions {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    PreviewCodeAction {
        token: u64,
    },
    ApplyCodeAction,
    CancelCodeActions,
    InlayHints {
        id: DocumentId,
        expected_version: u64,
        lines: Range<usize>,
    },
    CancelInlayHints,
    OpenWorkspace(PathBuf),
    LoadDirectory(PathBuf),
    OpenFile(PathBuf),
    ShowLocalHistory {
        id: DocumentId,
    },
    SelectLocalRevision {
        revision: u64,
    },
    LabelLocalHistory {
        id: DocumentId,
        expected_version: u64,
        label: String,
    },
    RestoreLocalRevision {
        id: DocumentId,
        expected_version: u64,
        revision: u64,
    },
    NewUntitled,
    SaveUntitled {
        id: DocumentId,
        path: PathBuf,
    },
    CreateFile(PathBuf),
    ActivateTab {
        view: ViewId,
        id: DocumentId,
    },
    /// Move an existing tab to a final, zero-based index within its group.
    ReorderTab {
        view: ViewId,
        id: DocumentId,
        index: usize,
    },
    CloseTab {
        view: ViewId,
        id: DocumentId,
        discard: bool,
    },
    Edit {
        id: DocumentId,
        expected_version: u64,
        text: String,
    },
    EditDelta {
        id: DocumentId,
        expected_version: u64,
        delta: TextDelta,
        kind: EditKind,
    },
    /// Atomic text and directed selection update, in scalar indices.
    EditTransaction {
        id: DocumentId,
        expected_version: u64,
        delta: TextDelta,
        kind: EditKind,
        before: Selection,
        after: Selection,
    },
    EditBatchTransaction {
        id: DocumentId,
        expected_version: u64,
        deltas: Vec<TextDelta>,
        before: Selection,
        after: Selection,
    },
    SetSelection {
        id: DocumentId,
        expected_version: u64,
        selection: Selection,
    },
    EndEditGroup {
        id: DocumentId,
    },
    Save {
        id: DocumentId,
    },
    ReloadDocument {
        id: DocumentId,
        expected_version: u64,
        external_revision: u64,
    },
    OverwriteDocument {
        id: DocumentId,
        expected_version: u64,
        external_revision: u64,
    },
    Undo {
        id: DocumentId,
    },
    Redo {
        id: DocumentId,
    },
    Activate {
        id: DocumentId,
    },
    Close {
        id: DocumentId,
        discard: bool,
    },
    Find {
        id: DocumentId,
        expected_version: u64,
        query: String,
    },
    ReplaceMatches {
        id: DocumentId,
        expected_version: u64,
        query: String,
        replacement: String,
        selection: Option<Range<usize>>,
    },
    CancelSearch,
    Hover {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    /// Passive pointer documentation: preserves other editor requests and Undo grouping.
    PreviewHover {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    CancelHover,
    Definition {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    Implementation {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    SuperDefinition {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    TypeDefinition {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    PreviewDefinition {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    CancelDefinitionPreview,
    ShowDefinitionTargets,
    ChooseDefinition {
        index: usize,
    },
    CancelDefinition,
    NavigateBack {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    /// Local structural navigation using the same history as definitions.
    NavigateToRange {
        id: DocumentId,
        expected_version: u64,
        range: Range<usize>,
    },
    NavigateForward {
        id: DocumentId,
        expected_version: u64,
        character_index: usize,
    },
    PersistSession,
    RestoreSession,
    RestoreSessionWithFallback {
        legacy: Option<PathBuf>,
    },
    CancelAll,
    Database(DbCommand),
    RemoteDatabase(remote::RemoteCommand),
    /// Explicitly bind unassigned pre-project database metadata to this project.
    ClaimLegacyDatabase,
    ConfigureProviders(Vec<ProviderConfig>),
    ConfigureProjectProviders {
        root: PathBuf,
        configs: Vec<ProviderConfig>,
    },
    LoadProviderConfig(PathBuf),
    LoadProjectProviderConfig {
        root: PathBuf,
        path: PathBuf,
    },
    StartProviders,
    StopProviders,
    /// Desktop opt-in; the profile must be outside the workspace. No process is
    /// started until this workspace/configuration was explicitly trusted.
    EnableProviderAutostart {
        profile: PathBuf,
        fallback_config: Option<PathBuf>,
    },
    SetProvidersAutomatic(bool),
    ExecuteSql {
        id: DocumentId,
        expected_version: u64,
        selection: Option<std::ops::Range<usize>>,
    },
}

/// Profile-wide editor preferences, independent of document text and history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorPreferences {
    pub copilot_enabled: bool,
    pub copilot_program: String,
    pub auto_save: bool,
    pub auto_save_delay_ms: u64,
    pub ligatures: bool,
    pub soft_wrap: bool,
    pub rust_inlay_hints: bool,
    pub python_inlay_hints: bool,
    pub git_current_line: bool,
    pub git_hover: bool,
    pub git_file_blame: bool,
}
impl Default for EditorPreferences {
    fn default() -> Self {
        Self {
            copilot_enabled: false,
            copilot_program: String::new(),
            auto_save: true,
            auto_save_delay_ms: 1000,
            ligatures: true,
            soft_wrap: true,
            rust_inlay_hints: true,
            python_inlay_hints: true,
            git_current_line: true,
            git_hover: true,
            git_file_blame: false,
        }
    }
}

/// Profile-wide Project presentation preference; no document or filesystem effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectPreferences {
    pub follow_active_file: bool,
}
impl Default for ProjectPreferences {
    fn default() -> Self {
        Self {
            follow_active_file: true,
        }
    }
}

/// Profile-wide tool-window visibility, independent of Git repository state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolWindowPreferences {
    pub commit: bool,
    pub git_confirm_actions: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Session {
    #[serde(default)]
    shell_layout: ShellLayout,
    #[serde(default)]
    git_drafts: BTreeMap<PathBuf, String>,
    #[serde(default)]
    editor_state: Option<view_session::EditorSession>,
    #[serde(default)]
    editor_preferences: EditorPreferences,
    #[serde(default)]
    project_preferences: ProjectPreferences,
    #[serde(default)]
    tool_window_preferences: ToolWindowPreferences,
    schema_version: u32,
    workspace: Option<PathBuf>,
    tabs: Vec<PathBuf>,
    active: Option<PathBuf>,
    #[serde(default)]
    database_path: Option<PathBuf>,
    #[serde(default)]
    provider_configs: Vec<ProviderConfig>,
}

struct OpenedFile {
    project_root: PathBuf,
    encoding: FileEncoding,
    path: PathBuf,
    text: String,
}
struct Workspace {
    file: Option<PathBuf>,
    folders: Vec<workspace_file::WorkspaceFolder>,
    project_encodings: BTreeMap<PathBuf, FileEncoding>,
    encoding: FileEncoding,
    path: PathBuf,
    entries: Vec<TreeEntry>,
    truncated: bool,
    loaded: std::collections::BTreeSet<PathBuf>,
    errors: BTreeMap<PathBuf, String>,
}
struct Restored {
    omitted_database_consoles: bool,
    shell_layout: ShellLayout,
    git_drafts: BTreeMap<PathBuf, String>,
    editor_state: Option<view_session::EditorSession>,
    editor_preferences: EditorPreferences,
    project_preferences: ProjectPreferences,
    tool_window_preferences: ToolWindowPreferences,
    workspace: Option<Workspace>,
    files: Vec<OpenedFile>,
    active: Option<PathBuf>,
    database_path: Option<PathBuf>,
    provider_configs: Vec<ProviderConfig>,
}
enum Work {
    Encoding {
        root: PathBuf,
        encoding: FileEncoding,
        files: Vec<(DocumentId, PathBuf)>,
    },
    Directory {
        root: PathBuf,
        path: PathBuf,
        revision: u64,
    },
    ProjectFile {
        directories: Vec<PathBuf>,
        workspace: PathBuf,
        command: ProjectFileCommand,
        open_paths: Vec<PathBuf>,
    },
    RenameResources {
        workspace: PathBuf,
        moves: Vec<rename::resources::ResourceMove>,
    },
    RenameInspect {
        workspace: PathBuf,
        buffers: Vec<(PathBuf, Arc<str>)>,
        token: u64,
        path: PathBuf,
        text: Arc<str>,
        line: u32,
        character: u32,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    RenameFilePreview {
        token: u64,
        workspace: PathBuf,
        search: rename::RenameSearch,
        new_name: String,
        buffers: Vec<(PathBuf, Arc<str>)>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    RenamePreview {
        token: u64,
        workspace: PathBuf,
        changes: Vec<codisseu_language::RenameFileEdits>,
        new_name: String,
        buffers: Vec<(PathBuf, u64, Arc<str>)>,
        context_limitation: Option<String>,
        local_origin: Option<(PathBuf, TextRange)>,
        search: rename::RenameSearch,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    RenameChoices {
        token: u64,
        workspace: PathBuf,
        prepared: Box<rename::PreparedRename>,
        new_name: String,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    RenameExpand {
        token: u64,
        workspace: PathBuf,
        prepared: Box<rename::PreparedRename>,
        result: codisseu_language::RenameResult,
        open_versions: Vec<(PathBuf, u64)>,
        new_name: String,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    RenameVerify {
        token: u64,
        workspace: PathBuf,
        files: Vec<RenamePreviewFile>,
        resources: Vec<rename::resources::ResourceMove>,
        search: rename::RenameSearch,
        open: Vec<PathBuf>,
        snapshots: Vec<codisseu_syntax::RenameSnapshot>,
        corpus: rename::RenameCorpus,
        new_name: String,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    },
    ReferencePreviews {
        token: u64,
        targets: Vec<DefinitionTarget>,
        workspace: PathBuf,
        open_buffers: Vec<OpenedFile>,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        allow_rust_libraries: bool,
        python_interpreter: Option<PathBuf>,
    },
    ReferenceOpen {
        token: u64,
        target: DefinitionTarget,
        workspace: PathBuf,
        expected: Arc<str>,
        open: bool,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        allow_rust_libraries: bool,
        python_interpreter: Option<PathBuf>,
    },
    Completion {
        token: u64,
        id: DocumentId,
        version: u64,
        text: String,
        caret: usize,
        primary: CompletionEdit,
        additional: Vec<CompletionEdit>,
        php_callable: bool,
    },
    Navigation {
        token: u64,
        target: DefinitionTarget,
        workspace: PathBuf,
        open_paths: Vec<PathBuf>,
        allow_rust_libraries: bool,
        python_interpreter: Option<PathBuf>,
    },
    GitReload(Vec<(DocumentId, u64, PathBuf)>),
    Open(PathBuf),
    Create(PathBuf),
    Workspace(PathBuf),
    SaveNew {
        id: DocumentId,
        path: PathBuf,
        text: TextSnapshot,
        identity: u64,
    },
    Save {
        encoding: FileEncoding,
        id: DocumentId,
        path: PathBuf,
        text: TextSnapshot,
        expected: Arc<str>,
        baseline: Arc<str>,
        identity: u64,
    },
    ReloadDocument {
        id: DocumentId,
        version: u64,
        external_revision: u64,
        path: PathBuf,
        encoding: FileEncoding,
        baseline: Arc<str>,
        observed: Arc<str>,
    },
    Persist {
        path: PathBuf,
        session: Session,
    },
    Restore {
        path: PathBuf,
        legacy: Option<PathBuf>,
        profile_anchor: PathBuf,
    },
    ProviderConfig {
        root: Option<PathBuf>,
        path: PathBuf,
    },
}
struct Job {
    workspace: Option<PathBuf>,
    id: u64,
    generation: u64,
    work: Work,
}
enum Completed {
    Directory {
        root: PathBuf,
        path: PathBuf,
        revision: u64,
        result: Result<Vec<TreeEntry>, String>,
    },
    Encoding {
        root: PathBuf,
        encoding: FileEncoding,
        files: Vec<(DocumentId, OpenedFile)>,
    },
    ProjectFile {
        receipt: project_files::FileReceipt,
        opened: Option<Result<OpenedFile, String>>,
        project_roots: Vec<(PathBuf, PathBuf)>,
        refresh: Result<Workspace, String>,
    },
    RenameResources(Result<rename::resources::ResourceReceipt, rename::resources::ResourceError>),
    RenameInspected {
        token: u64,
        name: Option<String>,
        kind: RenameSymbolKind,
    },
    Rename {
        token: u64,
        prepared: Box<rename::PreparedRename>,
        apply: bool,
    },
    ReferencePreviews {
        token: u64,
        items: Vec<ReferenceItem>,
        cache: HashMap<PathBuf, Arc<str>>,
    },
    ReferenceOpen {
        token: u64,
        target: DefinitionTarget,
        file: Option<OpenedFile>,
        read_only: bool,
    },
    Completion {
        token: u64,
        id: DocumentId,
        version: u64,
        text: String,
        caret: usize,
    },
    Navigation {
        token: u64,
        target: DefinitionTarget,
        file: Option<OpenedFile>,
        read_only: bool,
    },
    GitReload(Vec<(DocumentId, u64, Result<Option<OpenedFile>, String>)>),
    Open(OpenedFile),
    Created(OpenedFile),
    Workspace(Workspace),
    Saved {
        previous: Option<Arc<str>>,
        new_path: Option<PathBuf>,
        id: DocumentId,
        text: Arc<str>,
        identity: u64,
        warning: Option<String>,
    },
    SaveConflict {
        id: DocumentId,
        baseline: Arc<str>,
        file: OpenedFile,
        error: String,
    },
    ReloadDocument {
        id: DocumentId,
        version: u64,
        external_revision: u64,
        baseline: Arc<str>,
        file: OpenedFile,
    },
    Persisted,
    Restored(Restored),
    ProviderConfig {
        root: Option<PathBuf>,
        configs: Vec<ProviderConfig>,
    },
}
struct Reply {
    id: u64,
    generation: u64,
    result: Result<Completed, String>,
}
#[derive(Clone, Copy, PartialEq)]
enum Pending {
    Encoding,
    ProjectFile,
    RenameResources,
    Rename(u64),
    References(u64),
    Completion(u64),
    Navigation(u64),
    Read,
    GitRead(u64),
    GitTreeRead,
    Directory,
    Workspace,
    Save(DocumentId),
    Persist,
    Restore,
}

pub use codisseu_ai as ai;
pub use codisseu_browser as browser;

pub struct AppService {
    pub jira: jira::Service,
    pub browser: browser::BrowserService,
    pub browser_library: browser_library::BrowserLibrary,
    browser_login: browser_login::BrowserLogin,
    pub browser_inspector: browser::inspector::InspectorService,
    browser_debug: browser_debug::BrowserDebug,
    browser_devtools: browser_devtools::BrowserDevTools,
    pub python: python_project::Service,
    pub python_tests: python_tests::Service,
    pub php_tests: php_tests::Service,
    pub js_tests: js_tests::Service,
    pub local_history: local_history::Service,
    pub api: api::ApiService,
    pub docker: docker::DockerService,
    external_files: external_files::Service,
    pub ai: ai::ChatService,
    pub git_commit_messages: git_commit_message::Service,
    pub project_files: ProjectFileState,
    pub debugger: debugger::DebugService,
    pub debug_configurations: Vec<debugger::DebugConfiguration>,
    pub debug_profile_error: Option<String>,
    debug_profiles: debug_integration::DebugProfiles,
    shell_layout: ShellLayout,
    editor_preferences: EditorPreferences,
    auto_save: auto_save::State,
    project_versions: project_versions::Service,
    git_line_changes: git_line_changes::Service,
    project_preferences: ProjectPreferences,
    tool_window_preferences: ToolWindowPreferences,
    project_encoding: FileEncoding,
    project_encodings: BTreeMap<PathBuf, FileEncoding>,
    project_languages: project_language::ProjectLanguages,
    pub rename: RenameState,
    rename_history: Option<rename::RenameHistory>,
    rename_stage: Option<rename::StagedRename>,
    pub references: ReferenceState,
    pub completion: CompletionState,
    pub code_actions: CodeActionState,
    pub inlay_hints: InlayHintState,
    pub signature_help: SignatureHelpState,
    pub formatting: FormattingState,
    pub navigation: NavigationState,
    pub documents: Vec<Document>,
    pub active: Option<DocumentId>,
    pub workspace: Option<PathBuf>,
    pub workspace_file: Option<PathBuf>,
    pub workspace_folders: Vec<workspace_file::WorkspaceFolder>,
    pub entries: Vec<TreeEntry>,
    pub tree_truncated: bool,
    project_tree: project_tree::TreeState,
    pub status: String,
    pub last_error: Option<String>,
    pub search: Option<SearchResult>,
    pub search_error: Option<String>,
    search_worker: search::Worker,
    pub git: git::GitService,
    git_sessions: BTreeMap<PathBuf, git_integration::RepositorySession>,
    git_target: Option<PathBuf>,
    git_drafts: BTreeMap<PathBuf, String>,
    git_workspace: Option<PathBuf>,
    git_autoselect: bool,
    git_applying: bool,
    git_open_destination: Option<PathBuf>,
    pub db: DbService,
    pub remote_db: remote::RemoteService,
    pub language: LanguageService,
    pub database_path: Option<PathBuf>,
    pub provider_configs: Vec<ProviderConfig>,
    provider_configs_explicit: bool,
    automatic_providers: provider_startup::AutomaticProviders,
    provider_operation: Option<provider_startup::ProviderOperation>,
    language_versions: HashMap<PathBuf, u64>,
    language_skipped_versions: HashMap<PathBuf, u64>,
    language_retries: HashMap<PathBuf, (u64, u8)>,
    session_path: PathBuf,
    profile_anchor: PathBuf,
    session_writable: bool,
    sender: SyncSender<Job>,
    receiver: Receiver<Reply>,
    generation: Arc<AtomicU64>,
    pending: BTreeMap<u64, Pending>,
    next_job: u64,
    next_document: u64,
    views: Vec<DocumentView>,
    active_view: Option<ViewId>,
    next_view: u64,
    editor_split: EditorSplit,
}

impl AppService {
    /// Spawns a worker; performs no filesystem access on this thread.
    pub fn new(session_path: PathBuf) -> Self {
        Self::new_with_profile(session_path.clone(), session_path)
    }

    /// Separate per-window editor restoration from stable project data storage.
    /// Explicit sessions can keep both paths equal to isolate fixtures/profiles.
    pub fn new_with_profile(session_path: PathBuf, profile_anchor: PathBuf) -> Self {
        let (sender, jobs) = mpsc::sync_channel(QUEUE_SIZE);
        let (replies, receiver) = mpsc::sync_channel(QUEUE_SIZE);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let encoding_profile = session_path.with_extension("encodings.json");
        std::thread::Builder::new()
            .name("codisseu-files".into())
            .spawn(move || worker::run(jobs, replies, worker_generation, encoding_profile))
            .expect("Não foi possível criar o worker de arquivos");
        Self {
            jira: jira::Service::new(session_path.with_extension("jira.json")),
            browser: browser::BrowserService::new(),
            browser_library: browser_library::BrowserLibrary::new(
                session_path.with_extension("browser-library.json"),
            ),
            browser_login: Default::default(),
            browser_inspector: browser::inspector::InspectorService::new(),
            browser_debug: Default::default(),
            browser_devtools: Default::default(),
            python_tests: python_tests::Service::new(
                session_path.with_extension("python-tests.json"),
            ),
            python: python_project::Service::new(session_path.with_extension("python.json")),
            php_tests: php_tests::Service::new(session_path.with_extension("php-tests.json")),
            js_tests: js_tests::Service::new(session_path.with_extension("js-tests.json")),
            external_files: external_files::Service::new(),
            local_history: local_history::Service::new(session_path.with_extension("history")),
            docker: docker::DockerService::default(),
            api: api::ApiService::new(profile_anchor.with_extension("api")),
            ai: ai::ChatService::new(profile_anchor.with_extension("ai.json")),
            git_commit_messages: Default::default(),
            project_files: ProjectFileState::default(),
            debugger: debugger::DebugService::new(),
            debug_configurations: Vec::new(),
            debug_profile_error: None,
            debug_profiles: debug_integration::DebugProfiles::new(
                session_path.with_extension("debugger.json"),
            ),
            editor_preferences: EditorPreferences::default(),
            auto_save: auto_save::State::default(),
            project_versions: project_versions::Service::default(),
            git_line_changes: git_line_changes::Service::new(),
            project_preferences: ProjectPreferences::default(),
            tool_window_preferences: ToolWindowPreferences::default(),
            project_encoding: FileEncoding::default(),
            project_encodings: BTreeMap::new(),
            project_languages: Default::default(),
            shell_layout: ShellLayout::default(),
            rename: RenameState::default(),
            rename_history: None,
            rename_stage: None,
            references: ReferenceState::default(),
            navigation: NavigationState::default(),
            completion: CompletionState::default(),
            code_actions: CodeActionState::default(),
            inlay_hints: InlayHintState::default(),
            signature_help: SignatureHelpState::default(),
            formatting: FormattingState::default(),
            documents: Vec::new(),
            active: None,
            workspace: None,
            workspace_file: None,
            workspace_folders: Vec::new(),
            entries: Vec::new(),
            tree_truncated: false,
            project_tree: Default::default(),
            status: "Abra uma pasta ou um arquivo".into(),
            last_error: None,
            search: None,
            search_error: None,
            search_worker: search::Worker::new(Arc::clone(&generation)),
            git: git::GitService::new(),
            git_sessions: BTreeMap::new(),
            git_target: None,
            git_drafts: BTreeMap::new(),
            git_workspace: None,
            git_autoselect: false,
            git_applying: false,
            git_open_destination: None,
            db: DbService::new(),
            remote_db: remote::RemoteService::new_project_scoped(profile_anchor.clone()),
            language: LanguageService::new(),
            database_path: None,
            provider_configs: Vec::new(),
            provider_configs_explicit: false,
            automatic_providers: provider_startup::AutomaticProviders::default(),
            provider_operation: None,
            language_versions: HashMap::new(),
            language_skipped_versions: HashMap::new(),
            language_retries: HashMap::new(),
            session_path,
            profile_anchor,
            session_writable: true,
            sender,
            receiver,
            generation,
            pending: BTreeMap::new(),
            next_job: 1,
            next_document: 1,
            views: Vec::new(),
            active_view: None,
            next_view: 0,
            editor_split: EditorSplit::default(),
        }
    }

    pub fn editor_preferences(&self) -> EditorPreferences {
        self.editor_preferences.clone()
    }

    pub fn project_preferences(&self) -> ProjectPreferences {
        self.project_preferences
    }

    pub fn tool_window_preferences(&self) -> ToolWindowPreferences {
        self.tool_window_preferences
    }

    pub fn active_document(&self) -> Option<&Document> {
        self.documents.iter().find(|d| Some(d.id) == self.active)
    }

    pub fn is_busy(&self) -> bool {
        self.local_history.is_busy() || self.is_busy_except_history()
    }
    fn is_busy_except_history(&self) -> bool {
        self.non_git_work_pending()
            || self.jira.is_busy()
            || self.browser_library.is_busy()
            || self.git.is_busy()
            || self.git_sessions.values().any(|s| s.service.is_busy())
            || self.debug_profiles.pending()
            || self.ai.profile_pending()
            || self.api.is_busy()
    }

    /// Global jobs that are not cancelled by the Git tool window's own control.
    pub fn non_git_work_pending(&self) -> bool {
        self.session_work_pending() || self.remote_db.is_busy()
    }

    fn session_work_pending(&self) -> bool {
        self.python.is_busy()
            || self.code_actions.is_pending()
            || self.formatting.is_pending()
            || self.rename.is_pending()
            || self.references.is_pending()
            || self.completion.is_pending()
            || self.signature_help.is_pending()
            || self.navigation.is_pending()
            || !self.pending.is_empty()
            || self.search_worker.is_busy()
            || self.db.is_busy()
            || self.language.is_busy()
            || self.project_language_busy()
            || self.automatic_providers.is_busy()
    }

    /// Current Find or any accepted replacement; cancelled Find acks can still drain.
    pub fn search_pending(&self) -> bool {
        self.search_worker.search_pending()
    }

    fn clear_search(&mut self) {
        self.search_worker.cancel_find();
        self.search = None;
        self.search_error = None;
    }

    pub fn providers_running(&self) -> bool {
        self.language
            .statuses
            .iter()
            .any(|p| matches!(p.state, ProviderState::Starting | ProviderState::Ready))
    }

    fn start_configured_providers(&mut self) -> Result<(), String> {
        // CLI --start-providers and the desktop's automatic policy share this
        // path. Repeating Start must not replace an already starting session.
        if self.providers_running() {
            return Ok(());
        }
        if self
            .pending
            .values()
            .any(|p| matches!(p, Pending::Workspace | Pending::Restore))
        {
            return Err("Aguarde a abertura da pasta antes de iniciar providers".into());
        }
        let workspace = self
            .language_project_root()
            .ok_or("Abra uma pasta antes de iniciar os providers")?;
        let configs = self.project_provider_configs(&workspace);
        if configs.is_empty() {
            return Err("Configure os executáveis de linguagem antes de iniciar".into());
        }
        if !self.project_provider_configuration_authorized(&workspace, &configs) {
            return Err("Confie e configure explicitamente os providers deste projeto".into());
        }
        if self.active_document().is_none() && self.project_roots().any(|root| root != &workspace) {
            self.project_languages.defer();
            self.status =
                "Workspace autorizado; abra um arquivo para analisar apenas seu projeto".into();
            return Ok(());
        }
        self.cancel_navigation();
        self.cancel_completion();
        self.cancel_signature_help();
        self.cancel_references();
        self.cancel_rename();
        self.language_versions.clear();
        self.language_skipped_versions.clear();
        self.language_retries.clear();
        self.language.dispatch(LanguageCommand::Start {
            workspace: workspace.clone(),
            configs: configs.clone(),
        })?;
        self.project_languages.started(workspace, configs);
        self.provider_operation = Some(provider_startup::ProviderOperation::Starting);
        self.status = "Inicializando providers de linguagem".into();
        Ok(())
    }

    fn sync_documents(&mut self) {
        self.poll_project_languages();
        if !self
            .language
            .statuses
            .iter()
            .any(|p| p.state == ProviderState::Ready)
        {
            return;
        }
        for doc in &self.documents {
            if doc.is_read_only() {
                continue;
            }
            if self
                .active_language_project()
                .is_some_and(|root| doc.project_root() != root)
            {
                continue;
            }
            let Some(language) = Language::for_path(&doc.path) else {
                continue;
            };
            if !self
                .language
                .statuses
                .iter()
                .any(|p| p.language == language && p.state == ProviderState::Ready)
            {
                continue;
            }
            if self.language_versions.get(&doc.path) == Some(&doc.version) {
                continue;
            }
            if !doc.language_supported() {
                if self.language_skipped_versions.get(&doc.path) == Some(&doc.version) {
                    continue;
                }
                if self.language_versions.contains_key(&doc.path) {
                    if let Err(error) = self.language.dispatch(LanguageCommand::Close {
                        path: doc.path.clone(),
                    }) {
                        self.last_error = Some(error);
                        break;
                    }
                    self.language_versions.remove(&doc.path);
                }
                self.language_skipped_versions
                    .insert(doc.path.clone(), doc.version);
                // No full String and no repeated dispatch/error while polling a large file.
                self.status = "Documento acima de 1 MiB: editor disponível; inteligência de linguagem e rename indisponíveis".into();
                continue;
            }
            self.language_skipped_versions.remove(&doc.path);
            match self.language.dispatch(LanguageCommand::Sync {
                path: doc.path.clone(),
                version: doc.version,
                text: doc.text().into(),
            }) {
                Ok(()) => {
                    self.language_versions.insert(doc.path.clone(), doc.version);
                }
                Err(error) => {
                    self.last_error = Some(error);
                    break;
                }
            }
        }
    }

    pub fn dispatch(&mut self, command: Command) -> Result<(), String> {
        // Web input is independent of document transactions, filesystem guards
        // and language requests. It must never end typing groups or clear a
        // dirty buffer simply because a browser frame received mouse input.
        let command = match command {
            Command::Browser(command) => return self.dispatch_browser_command(command),
            // Ordering is metadata only: do not touch buffers, typing groups,
            // providers or in-flight file operations for a tab move.
            Command::ReorderTab { view, id, index } => return self.reorder_tab(view, id, index),
            command => command,
        };
        if self.remote_db.console_delete_pending() {
            return Err("Aguarde a exclusão da query".into());
        }
        if self.remote_db.console_rename_pending()
            && matches!(
                command,
                Command::Save { .. }
                    | Command::OverwriteDocument { .. }
                    | Command::ReloadDocument { .. }
                    | Command::PersistSession
                    | Command::OpenWorkspace(_)
                    | Command::CloseWorkspace
                    | Command::RestoreSession
                    | Command::RestoreSessionWithFallback { .. }
                    | Command::ProjectFile(_)
            )
        {
            return Err("Aguarde a operação da query antes de gravar ou trocar o projeto".into());
        }
        if self.rename_stage.is_some() {
            return Err("Aguarde a transação de arquivos de Rename; edição e cancelamento estão suspensos até o ACK".into());
        }
        self.sync_views();
        let is_rename = matches!(
            &command,
            Command::PrepareRename { .. }
                | Command::PrepareFileRename { .. }
                | Command::PreviewRename { .. }
                | Command::ApplyRename
        );
        let duplicate_rename = matches!(&command, Command::ApplyRename)
            && (self.rename.applying()
                || ((self.rename.semantic_blocked()
                    || self.rename.choosing_methods()
                    || self.rename.choosing_inheritors())
                    && self.rename_current()));
        let is_references = matches!(
            &command,
            Command::References { .. } | Command::ChooseReference { .. }
        );
        let duplicate_reference =
            matches!(&command, Command::ChooseReference { .. }) && self.references.is_opening();
        let is_replacement = matches!(command, Command::ReplaceMatches { .. });
        let is_search = matches!(
            command,
            Command::Find { .. } | Command::ReplaceMatches { .. } | Command::CancelSearch
        );
        let is_navigation = matches!(
            &command,
            Command::Definition { .. }
                | Command::Implementation { .. }
                | Command::SuperDefinition { .. }
                | Command::TypeDefinition { .. }
                | Command::ChooseDefinition { .. }
                | Command::NavigateBack { .. }
                | Command::NavigateToRange { .. }
                | Command::NavigateForward { .. }
        );
        let is_completion = matches!(
            &command,
            Command::Completion { .. } | Command::AcceptCompletion { .. }
        );
        let preserve_accepted =
            matches!(&command, Command::AcceptCompletion { .. }) && self.completion.is_applying();
        let is_formatting = matches!(&command, Command::FormatDocument { .. });
        let is_signature = matches!(&command, Command::SignatureHelp { .. });
        let speculative = matches!(
            &command,
            Command::PreviewDefinition { .. }
                | Command::CancelDefinitionPreview
                | Command::PreviewHover { .. }
                | Command::CancelHover
                | Command::InlayHints { .. }
                | Command::CancelInlayHints
        );
        let actions = matches!(
            &command,
            Command::CodeActions { .. }
                | Command::PreviewCodeAction { .. }
                | Command::ApplyCodeAction
        );
        let result = self.execute(command);
        if actions && let Err(error) = &result {
            self.code_actions_failed(error.clone());
        }
        self.sync_views();
        if is_formatting && let Err(error) = &result {
            self.formatting_failed(error.clone());
        }
        if is_signature && let Err(error) = &result {
            self.signature_help_failed(error.clone());
        }
        if is_rename
            && !duplicate_rename
            && let Err(e) = &result
        {
            self.rename_failed(e.clone());
        }
        if is_references
            && !duplicate_reference
            && let Err(error) = &result
        {
            self.references_failed(error.clone());
        }
        if is_completion
            && !preserve_accepted
            && let Err(error) = &result
        {
            self.completion_failed(error.clone());
        }
        if is_navigation && let Err(error) = &result {
            self.navigation_failed(error.clone());
        }
        if let Err(error) = &result
            && !speculative
        {
            self.status = error.clone();
            if is_search {
                self.search_error = Some(error.clone());
                if is_replacement {
                    self.last_error = Some(error.clone());
                }
            } else {
                self.last_error = Some(error.clone());
            }
        }
        result
    }

    pub fn encoding_profile_path(&self) -> PathBuf {
        self.session_path.with_extension("encodings.json")
    }
    pub fn project_encoding(&self) -> FileEncoding {
        self.encoding_project_root()
            .and_then(|r| self.project_encodings.get(r))
            .copied()
            .unwrap_or(self.project_encoding)
    }
    pub fn encoding_pending(&self) -> bool {
        self.pending.values().any(|p| *p == Pending::Encoding)
    }
    fn execute(&mut self, command: Command) -> Result<(), String> {
        if self.encoding_pending() {
            return Err("Aguarde a troca de codificação do projeto".into());
        }

        self.guard_read_only_document(&command)?;
        self.guard_project_file_mutation(&command)?;
        if self.rename_stage.is_some() {
            return Err("Aguarde o ACK da transação de Rename".into());
        }
        if !matches!(
            &command,
            Command::EditDelta { .. }
                | Command::EditTransaction { .. }
                | Command::EditBatchTransaction { .. }
                | Command::SetSelection { .. }
                | Command::EndEditGroup { .. }
                | Command::SetEditorSplit(_)
                | Command::SetShellLayout(_)
                | Command::SetCommitToolWindowVisible(_)
                | Command::SetGitConfirmActions(_)
                | Command::SetProjectFollowActiveFile(_)
                | Command::LoadDirectory(_)
                | Command::SetViewScroll { .. }
                | Command::SetViewScrollAnchor { .. }
                | Command::ShowDocumentInView { .. }
                | Command::SetEditorSoftWrap(_)
                | Command::SignatureHelp { .. }
                | Command::CancelSignatureHelp
                | Command::PreviewDefinition { .. }
                | Command::CancelDefinitionPreview
                | Command::PreviewHover { .. }
                | Command::CancelHover
                | Command::InlayHints { .. }
                | Command::CancelInlayHints
        ) {
            for doc in &mut self.documents {
                doc.buffer.end_edit_group();
            }
        }
        if self.pending.values().any(|p| *p == Pending::Restore)
            && !matches!(command, Command::CancelAll | Command::CancelSearch)
        {
            return Err("Aguarde a restauração da sessão ou cancele a operação".into());
        }
        self.guard_git_apply(&command)?;
        match command {
            Command::Browser(command) => self.dispatch_browser_command(command)?,
            Command::PhpTests(command) => self.dispatch_php_tests(command)?,
            Command::JsTests(command) => self.dispatch_js_tests(command)?,
            Command::Python(command) => self.dispatch_python(command)?,
            Command::PythonTests(command) => self.dispatch_python_tests(command)?,
            Command::SetPythonInlayHints(enabled) => {
                self.editor_preferences.python_inlay_hints = enabled;
                if !enabled {
                    self.cancel_inlay_hints();
                }
            }
            Command::Docker(command) => self.docker.dispatch(command)?,
            Command::Ai(command) => {
                let root = self.ai_workspace().map(PathBuf::from);
                self.ai.sync_workspace(root.as_deref());
                self.ai.dispatch(command, root)?;
            }
            Command::GenerateGitCommitMessage { root } => self.generate_git_commit_message(root)?,
            Command::CancelGitCommitMessage { root } => {
                self.git_commit_messages.cancel(root.as_deref())
            }
            Command::ProjectFile(command) => self.project_file_command(command)?,
            Command::CloseWorkspace => self.close_workspace()?,
            Command::Debug(command) => self.dispatch_debug(command)?,
            Command::SaveDebugConfiguration(config) => self.save_debug_configuration(config)?,
            Command::DeleteDebugConfiguration(id) => self.delete_debug_configuration(id)?,
            Command::FormatDocument {
                id,
                expected_version,
                range,
            } => self.request_formatting(id, expected_version, range)?,
            Command::CodeActions {
                id,
                expected_version,
                character_index,
            } => self.request_code_actions(id, expected_version, character_index)?,
            Command::PreviewCodeAction { token } => self.preview_code_action(token)?,
            Command::ApplyCodeAction => self.apply_code_action()?,
            Command::CancelCodeActions => self.cancel_code_actions(),
            Command::InlayHints {
                id,
                expected_version,
                lines,
            } => self.request_inlay_hints(id, expected_version, lines)?,
            Command::CancelInlayHints => self.cancel_inlay_hints(),
            Command::CancelFormatting => {
                self.cancel_formatting();
                self.status = "Formatação cancelada".into();
            }
            Command::Git(command) => self.dispatch_git(command)?,
            Command::SelectGitRepository(path) => self.select_git_repository(path)?,
            Command::SetGitDraft { root, message } => self.set_git_draft(root, message)?,
            Command::SetGitInsights {
                inline,
                hover,
                gutter,
            } => {
                self.editor_preferences.git_current_line = inline;
                self.editor_preferences.git_hover = hover;
                self.editor_preferences.git_file_blame = gutter;
            }
            Command::SetGitConfirmActions(confirm) => {
                self.tool_window_preferences.git_confirm_actions = confirm;
            }
            Command::SetCommitToolWindowVisible(visible) => {
                self.tool_window_preferences.commit = visible;
            }
            Command::SetShellLayout(layout) => {
                layout.validate()?;
                self.shell_layout = layout;
            }
            Command::SetProjectFollowActiveFile(enabled) => {
                self.project_preferences.follow_active_file = enabled;
            }
            Command::SetRustInlayHints(enabled) => {
                self.editor_preferences.rust_inlay_hints = enabled;
                if !enabled {
                    self.cancel_inlay_hints();
                }
            }
            Command::SetAutoSave { enabled, delay_ms } => {
                if !(250..=60_000).contains(&delay_ms) {
                    return Err(
                        "Intervalo do salvamento automático deve ser entre250 e60000ms".into(),
                    );
                }
                self.editor_preferences.auto_save = enabled;
                self.editor_preferences.auto_save_delay_ms = delay_ms;
                self.auto_save.reset_timers();
                self.status = "Preferência de salvamento automático atualizada".into();
            }
            Command::SetCopilotEnabled(enabled) => {
                self.editor_preferences.copilot_enabled = enabled;
            }
            Command::SetCopilotProgram(program) => {
                if program.len() > 4096
                    || program.contains('\0')
                    || (!program.is_empty() && !std::path::Path::new(&program).is_absolute())
                {
                    return Err("Informe um caminho absoluto para o servidor Copilot".into());
                }
                self.editor_preferences.copilot_program = program;
            }
            Command::SetEditorLigatures(enabled) => {
                self.editor_preferences.ligatures = enabled;
                self.status = "Preferência de ligaduras atualizada".into();
            }
            Command::SetEditorSoftWrap(enabled) => {
                self.editor_preferences.soft_wrap = enabled;
                self.status = if enabled {
                    "Quebra automática de linha ativada"
                } else {
                    "Quebra automática de linha desativada"
                }
                .into();
            }
            Command::SetEditorSplit(split) => {
                split.validate()?;
                self.editor_split = split;
            }
            Command::SetViewScroll { view, scroll } => {
                if !view_session::valid_scroll(scroll) {
                    return Err("Scroll de grupo inválido".into());
                }
                let v = self
                    .views
                    .iter_mut()
                    .find(|v| v.id == view)
                    .ok_or("Visualização não encontrada")?;
                let position = v
                    .positions
                    .get_mut(&v.document)
                    .ok_or("Posição de visualização ausente")?;
                if position.scroll != scroll {
                    position.source_anchor = None;
                }
                position.scroll = scroll;
            }
            Command::SetViewScrollAnchor {
                view,
                document,
                anchor,
            } => {
                let doc = self
                    .documents
                    .iter()
                    .find(|doc| doc.id == document)
                    .ok_or("Documento da âncora não encontrado")?;
                if anchor
                    .is_some_and(|anchor| !anchor.is_valid() || anchor.character > doc.len_chars())
                {
                    return Err("Âncora de leitura inválida".into());
                }
                let view = self
                    .views
                    .iter_mut()
                    .find(|candidate| candidate.id == view && candidate.document == document)
                    .ok_or("Documento/visualização da âncora não correspondem")?;
                view.positions
                    .get_mut(&document)
                    .ok_or("Posição de visualização ausente")?
                    .source_anchor = anchor;
            }
            Command::CreateView => self.create_view()?,
            Command::ActivateView { view } => self.activate_view(view)?,
            Command::ShowDocumentInView { view, id } => self.show_document_in_view(view, id)?,
            Command::CloseView { view } => self.close_view(view)?,
            Command::ActivateTab { view, id } => self.activate_tab(view, id)?,
            Command::ReorderTab { view, id, index } => self.reorder_tab(view, id, index)?,
            Command::CloseTab { view, id, discard } => self.close_tab(view, id, discard)?,
            Command::PrepareFileRename {
                id,
                expected_version,
            } => self.prepare_file_rename(id, expected_version)?,
            Command::PrepareRename {
                id,
                expected_version,
                character_index,
            } => self.prepare_rename(id, expected_version, character_index)?,
            Command::SetRenameOptions { options } => self.set_rename_options(options)?,
            Command::PreviewRename { new_name } => self.preview_rename(new_name)?,
            Command::UpdateRenameInheritor {
                token,
                index,
                selected,
                new_name,
            } => self.update_rename_inheritor(token, index, selected, new_name)?,
            Command::EditRenameInheritors { token } => self.edit_rename_inheritors(token)?,
            Command::ConfirmRenameInheritors { token } => self.confirm_rename_inheritors(token)?,
            Command::SelectRenameMethod {
                token,
                index,
                selected,
            } => self.select_rename_method(token, index, selected)?,
            Command::ConfirmRenameMethods { token } => self.confirm_rename_methods(token)?,
            Command::ApplyRename => self.apply_rename()?,
            Command::CancelRename => {
                self.cancel_rename();
                self.rename.error = None;
            }
            Command::References {
                id,
                expected_version,
                character_index,
            } => self.request_references(id, expected_version, character_index)?,
            Command::ChooseReference {
                index,
                id,
                expected_version,
                character_index,
            } => self.choose_reference(index, id, expected_version, character_index)?,
            Command::CancelReferences => {
                self.cancel_references();
                self.references.error = None;
                self.status = "Referências fechadas".into();
            }
            Command::SignatureHelp {
                id,
                expected_version,
                character_index,
                trigger,
                is_retrigger,
            } => self.request_signature_help(
                id,
                expected_version,
                character_index,
                trigger,
                is_retrigger,
            )?,
            Command::CancelSignatureHelp => {
                self.cancel_signature_help();
            }
            Command::Completion {
                id,
                expected_version,
                character_index,
            } => self.request_completion(id, expected_version, character_index)?,
            Command::AcceptCompletion { index, replace } => {
                self.accept_completion(index, replace)?
            }
            Command::CancelCompletion => {
                self.cancel_completion();
                self.completion.error = None;
                self.status = "Sugestões fechadas".into();
            }
            Command::Definition {
                id,
                expected_version,
                character_index,
            } => self.request_definition(id, expected_version, character_index)?,
            Command::Implementation {
                id,
                expected_version,
                character_index,
            } => self.request_implementation(id, expected_version, character_index)?,
            Command::SuperDefinition {
                id,
                expected_version,
                character_index,
            } => self.request_super_definition(id, expected_version, character_index)?,
            Command::TypeDefinition {
                id,
                expected_version,
                character_index,
            } => self.request_type_definition(id, expected_version, character_index)?,
            Command::PreviewDefinition {
                id,
                expected_version,
                character_index,
            } => self.preview_definition(id, expected_version, character_index),
            Command::CancelDefinitionPreview => self.cancel_definition_preview(),
            Command::ShowDefinitionTargets => self.show_definition_targets()?,
            Command::ChooseDefinition { index } => self.choose_definition(index)?,
            Command::CancelDefinition => {
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.navigation.error = None;
                self.status = "Navegação cancelada".into();
            }
            Command::NavigateBack {
                id,
                expected_version,
                character_index,
            } => self.navigate_history(id, expected_version, character_index, false)?,
            Command::NavigateToRange {
                id,
                expected_version,
                range,
            } => {
                self.navigate_to_range(id, expected_version, range)?;
            }
            Command::NavigateForward {
                id,
                expected_version,
                character_index,
            } => self.navigate_history(id, expected_version, character_index, true)?,
            Command::CancelHover => self.language.dispatch(LanguageCommand::CancelHover)?,
            Command::Hover {
                id,
                expected_version,
                character_index,
            } => self.request_documentation_hover(id, expected_version, character_index, false)?,
            Command::PreviewHover {
                id,
                expected_version,
                character_index,
            } => self.request_documentation_hover(id, expected_version, character_index, true)?,
            Command::SetProjectEncoding(encoding) => {
                let root = self
                    .encoding_project_root()
                    .ok_or("Abra um projeto para escolher sua codificação")?
                    .to_path_buf();
                self.set_folder_encoding(root, encoding)?;
            }
            Command::SetFolderEncoding { root, encoding } => {
                self.set_folder_encoding(root, encoding)?
            }
            Command::OpenWorkspace(path) => {
                if let Some(blocker) = self.workspace_change_blocker(&path) {
                    return Err(blocker.message().into());
                }
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.generation.fetch_add(1, Ordering::SeqCst);
                self.enqueue(Work::Workspace(path), Pending::Workspace)?;
            }
            Command::LoadDirectory(path) => self.load_tree_directory(path)?,
            Command::NewUntitled => {
                let n = (1..)
                    .find(|n| {
                        !self.documents.iter().any(|d| {
                            d.untitled
                                && d.path.as_os_str()
                                    == std::ffi::OsStr::new(&format!("Untitled-{n}"))
                        })
                    })
                    .unwrap();
                let id = self.insert_document(OpenedFile {
                    path: PathBuf::from(format!("Untitled-{n}")),
                    project_root: self
                        .workspace_roots()
                        .first()
                        .cloned()
                        .or_else(|| std::env::current_dir().ok())
                        .unwrap_or_default(),
                    text: String::new(),
                    encoding: FileEncoding::Utf8,
                })?;
                self.document_mut(id)?.untitled = true;
                self.active = Some(id);
                self.status = "Novo arquivo sem salvar".into();
            }
            Command::SaveUntitled { id, path } => {
                if self.pending.values().any(|p| *p == Pending::Save(id)) {
                    return Err("Este documento já está sendo salvo".into());
                }
                if self.documents.iter().any(|d| !d.untitled && d.path == path) {
                    return Err("O destino já está aberto".into());
                }
                let doc = self.document_mut(id)?;
                if !doc.untitled {
                    return Err("O documento já possui um caminho".into());
                }
                let work = Work::SaveNew {
                    id,
                    path,
                    text: doc.snapshot(),
                    identity: doc.content_identity(),
                };
                self.enqueue(work, Pending::Save(id))?;
            }
            Command::CreateFile(path) => {
                if self.documents.len()
                    + self
                        .pending
                        .values()
                        .filter(|p| **p == Pending::Read)
                        .count()
                    >= MAX_DOCUMENTS
                {
                    return Err("Feche uma aba antes de criar outro arquivo (limite32)".into());
                }
                let path = if path.is_relative() {
                    self.workspace
                        .as_ref()
                        .map_or_else(|| path.clone(), |root| root.join(&path))
                } else {
                    path
                };
                self.enqueue(Work::Create(path), Pending::Read)?;
            }
            Command::OpenFile(path) if workspace_file::is_workspace_file(&path) => {
                self.execute(Command::OpenWorkspace(path))?;
            }
            Command::OpenFile(path) => {
                // Canonical duplicate detection happens after reading, so an
                // already-open file can still be activated at the tab limit.
                self.enqueue(Work::Open(path), Pending::Read)?;
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
            }
            Command::ShowLocalHistory { id } => self.show_local_history(id)?,
            Command::SelectLocalRevision { revision } => self.select_local_revision(revision)?,
            Command::LabelLocalHistory {
                id,
                expected_version,
                label,
            } => self.label_local_history(id, expected_version, label)?,
            Command::RestoreLocalRevision {
                id,
                expected_version,
                revision,
            } => self.restore_local_revision(id, expected_version, revision)?,
            Command::SetSelection {
                id,
                expected_version,
                selection,
            } => {
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Seleção obsoleta: versão do documento mudou".into());
                }
                let changed = doc.buffer.selection() != selection;
                doc.buffer.set_selection(selection)?;
                if changed {
                    self.cancel_signature_help();
                }
            }
            Command::EditTransaction {
                id,
                expected_version,
                delta,
                kind,
                before,
                after,
            } => {
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Edição obsoleta: versão do documento mudou".into());
                }
                if doc
                    .buffer
                    .apply_delta_with_selection(delta, kind, before, after)?
                {
                    doc.version += 1;
                    self.cancel_navigation();
                    self.cancel_completion();
                    self.cancel_signature_help();
                    self.cancel_references();
                    self.cancel_rename();
                    self.clear_search();
                }
                self.status = "Documento alterado".into();
            }
            Command::EditBatchTransaction {
                id,
                expected_version,
                deltas,
                before,
                after,
            } => {
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Edição obsoleta: versão do documento mudou".into());
                }
                if doc
                    .buffer
                    .apply_batch_with_selection(deltas.clone(), before, after)?
                {
                    doc.version += 1;
                    // Map passive views through each real delta rather than the enclosing span.
                    let mut reverse = deltas;
                    reverse.sort_by_key(|d| std::cmp::Reverse(d.range.start));
                    self.remap_views(id, &reverse);
                    self.cancel_navigation();
                    self.cancel_completion();
                    self.cancel_signature_help();
                    self.cancel_references();
                    self.cancel_rename();
                    self.clear_search();
                }
                self.status = "Documento alterado".into();
            }
            Command::EndEditGroup { id } => self.document_mut(id)?.buffer.end_edit_group(),
            Command::EditDelta {
                id,
                expected_version,
                delta,
                kind,
            } => {
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Edição obsoleta: versão do documento mudou".into());
                }
                if doc.buffer.apply_delta(delta, kind)? {
                    doc.version += 1;
                    self.cancel_navigation();
                    self.cancel_completion();
                    self.cancel_signature_help();
                    self.cancel_references();
                    self.cancel_rename();
                    self.clear_search();
                }
                self.status = "Documento alterado".into();
            }
            Command::Edit {
                id,
                expected_version,
                text,
            } => {
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Edição obsoleta: versão do documento mudou".into());
                }
                if doc.buffer.replace(text)? {
                    doc.version += 1;
                    self.cancel_navigation();
                    self.cancel_completion();
                    self.cancel_signature_help();
                    self.cancel_references();
                    self.cancel_rename();
                    self.clear_search();
                }
                self.status = "Documento alterado".into();
            }
            Command::Undo { id } => {
                if self.rename_undo(id, false)? {
                    return Ok(());
                }
                let doc = self.document_mut(id)?;
                if !doc.buffer.undo() {
                    return Err("Não há edição para desfazer".into());
                }
                doc.version += 1;
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.clear_search();
                self.status = "Edição desfeita".into();
            }
            Command::Redo { id } => {
                if self.rename_undo(id, true)? {
                    return Ok(());
                }
                let doc = self.document_mut(id)?;
                if !doc.buffer.redo() {
                    return Err("Não há edição para refazer".into());
                }
                doc.version += 1;
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.clear_search();
                self.status = "Edição refeita".into();
            }
            Command::ReloadDocument {
                id,
                expected_version,
                external_revision,
            } => self.resolve_external_document(id, expected_version, external_revision, false)?,
            Command::OverwriteDocument {
                id,
                expected_version,
                external_revision,
            } => self.resolve_external_document(id, expected_version, external_revision, true)?,
            Command::Save { id } => {
                if self.pending.values().any(|p| *p == Pending::Save(id)) {
                    return Err("Este documento já está sendo salvo".into());
                }
                let doc = self.document_mut(id)?;
                if doc.untitled {
                    return Err("Escolha um caminho para salvar o novo arquivo".into());
                }
                let work = Work::Save {
                    encoding: doc.encoding,
                    id,
                    path: doc.path.clone(),
                    text: doc.snapshot(),
                    expected: doc.saved_text.clone(),
                    baseline: doc.saved_text.clone(),
                    identity: doc.buffer.content_identity(),
                };
                self.invalidate_external_file_reads();
                self.enqueue(work, Pending::Save(id))?;
            }
            Command::Activate { id } => {
                self.document_mut(id)?;
                if self.active != Some(id) {
                    self.clear_search();
                }
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.active = Some(id);
            }
            Command::Close { id, discard } => {
                if self.pending.values().any(|p| *p == Pending::Save(id)) {
                    return Err("Aguarde a gravação antes de fechar este documento".into());
                }
                if self.document_mut(id)?.is_dirty() && !discard {
                    return Err("Documento tem alterações não salvas".into());
                }
                if let Some(path) = self
                    .documents
                    .iter()
                    .find(|d| d.id == id)
                    .map(|d| d.path.clone())
                {
                    self.close_language_document(&path)?;
                    self.language_versions.remove(&path);
                    self.language_retries.remove(&path);
                    self.language_skipped_versions.remove(&path);
                }
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.clear_search();
                self.documents.retain(|d| d.id != id);
                if self.active == Some(id) {
                    self.active = self.documents.last().map(|d| d.id);
                }
            }
            Command::Find {
                id,
                expected_version,
                query,
            } => {
                codisseu_editor::search::validate_query(&query)?;
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Busca obsoleta: versão do documento mudou".into());
                }
                let request = search::Request {
                    document_id: id,
                    version: expected_version,
                    text: doc.text().into(),
                    query,
                    operation: search::Operation::Find,
                };
                self.search_worker.enqueue(request)?;
                self.search = None;
                self.search_error = None;
            }
            Command::ReplaceMatches {
                id,
                expected_version,
                query,
                replacement,
                selection,
            } => {
                codisseu_editor::search::validate_query(&query)?;
                if query.is_empty() {
                    return Err("Informe um texto de busca antes de substituir".into());
                }
                if replacement.len() > codisseu_editor::MAX_DOCUMENT_BYTES
                    || replacement.contains('\0')
                {
                    return Err("Substituição deve ter até 1 MiB e não conter NUL".into());
                }
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("Substituição obsoleta: versão do documento mudou".into());
                }
                let request = search::Request {
                    document_id: id,
                    version: expected_version,
                    text: doc.text().into(),
                    query,
                    operation: search::Operation::Replace {
                        replacement,
                        selection,
                    },
                };
                self.search_worker.enqueue(request)?;
                self.search_error = None;
                self.status = "Calculando substituição".into();
            }
            Command::CancelSearch => self.clear_search(),
            Command::PersistSession => {
                // History cannot alter the session snapshot; its private worker
                // drains independently. Do not make NewUntitled block session Save.
                if self.is_busy_except_history() {
                    return Err("Aguarde as operações antes de salvar a sessão".into());
                }
                if !self.session_writable {
                    return Err("Sessão anterior inválida: ela foi preservada; escolha outro arquivo de sessão".into());
                }
                let session = Session {
                    shell_layout: self.shell_layout,
                    git_drafts: self.git_drafts.clone(),
                    editor_state: self.save_editor_state(),
                    editor_preferences: self.editor_preferences.clone(),
                    project_preferences: self.project_preferences,
                    tool_window_preferences: self.tool_window_preferences,
                    schema_version: 2,
                    workspace: self.workspace_path().map(PathBuf::from),
                    tabs: self
                        .documents
                        .iter()
                        .filter(|d| !d.untitled && !d.is_read_only())
                        .map(|d| d.path.clone())
                        .collect(),
                    active: self
                        .active_document()
                        .filter(|d| !d.untitled && !d.is_read_only())
                        .map(|d| d.path.clone()),
                    database_path: self.database_path.clone(),
                    provider_configs: self.provider_configs.clone(),
                };
                self.enqueue(
                    Work::Persist {
                        path: self.session_path.clone(),
                        session,
                    },
                    Pending::Persist,
                )?;
            }
            Command::RestoreSession | Command::RestoreSessionWithFallback { .. } => {
                let legacy = if let Command::RestoreSessionWithFallback { legacy } = command {
                    legacy
                } else {
                    None
                };
                if self.session_work_pending()
                    || self.remote_db.has_pending_operation()
                    || !self.documents.is_empty()
                    || self.workspace.is_some()
                {
                    return Err(
                        "Restauração requer uma sessão vazia sem operações pendentes".into(),
                    );
                }
                // Cancelling a restore must not authorize overwriting the
                // session whose contents have not been validated and applied.
                self.session_writable = false;
                self.enqueue(
                    Work::Restore {
                        path: self.session_path.clone(),
                        legacy,
                        profile_anchor: self.profile_anchor.clone(),
                    },
                    Pending::Restore,
                )?;
            }
            Command::CancelAll => {
                self.python_tests.cancel();
                self.python.cancel();
                self.php_tests.cancel();
                self.js_tests.cancel();
                if let Some(id) = self.js_tests.debug_session {
                    let _ = self.debugger.execute(debugger::DebugCommand::Stop(id));
                }
                self.cancel_code_actions();
                self.cancel_inlay_hints();
                self.docker.cancel();
                self.cancel_formatting();
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.clear_search();
                self.generation.fetch_add(1, Ordering::SeqCst);
                if self.git.is_busy() {
                    self.git
                        .dispatch(git::GitCommand::Cancel)
                        .map_err(|e| e.to_string())?;
                }
                for session in self.git_sessions.values_mut() {
                    session
                        .service
                        .dispatch(git::GitCommand::Cancel)
                        .map_err(|e| e.to_string())?;
                }
                if self.db.is_busy() {
                    self.db.dispatch(DbCommand::Cancel)?;
                }
                if self.remote_db.is_busy() {
                    self.remote_db.dispatch(remote::RemoteCommand::Cancel)?;
                }
                self.language.dispatch(LanguageCommand::CancelHover)?;
                self.status = "Cancelamento solicitado; aguardando operações em andamento".into();
            }
            Command::Database(command) => {
                if !matches!(command, DbCommand::Cancel)
                    && self.pending.values().any(|p| *p == Pending::Workspace)
                {
                    return Err("Aguarde a abertura da pasta antes de operar o banco".into());
                }
                self.db.dispatch(command)?;
            }
            Command::RemoteDatabase(command) => {
                if !matches!(command, remote::RemoteCommand::Cancel)
                    && self.pending.values().any(|p| *p == Pending::Workspace)
                {
                    return Err("Aguarde a abertura da pasta antes de operar o banco".into());
                }
                match command {
                    remote::RemoteCommand::RenameConsole { path, name } => {
                        self.rename_database_console(path, name)?
                    }
                    remote::RemoteCommand::DeleteConsole(path) => {
                        self.delete_database_console(path)?
                    }
                    command => self.remote_db.dispatch(command)?,
                }
            }
            Command::ClaimLegacyDatabase => {
                if self.workspace_open_pending() {
                    return Err("Aguarde a abertura do projeto antes de vincular os bancos".into());
                }
                self.remote_db.claim_legacy_project()?;
            }
            Command::LoadProviderConfig(path) => {
                if self.any_project_providers_running()
                    || self.project_language_busy()
                    || self.language.is_busy()
                {
                    return Err("Pare os providers antes de alterar a configuração".into());
                }
                self.enqueue(Work::ProviderConfig { root: None, path }, Pending::Read)?;
            }
            Command::LoadProjectProviderConfig { root, path } => {
                if self.any_project_providers_running()
                    || self.project_language_busy()
                    || self.language.is_busy()
                {
                    return Err("Pare os providers antes de alterar a configuração".into());
                }
                self.enqueue(
                    Work::ProviderConfig {
                        root: Some(root),
                        path,
                    },
                    Pending::Read,
                )?;
            }
            Command::ConfigureProjectProviders { root, configs } => {
                self.configure_project_providers(root, configs)?
            }
            Command::ConfigureProviders(configs) => {
                if self.any_project_providers_running()
                    || self.project_language_busy()
                    || self.language.is_busy()
                {
                    return Err("Pare os providers antes de alterar a configuração".into());
                }
                if configs.len() > 4 {
                    return Err(
                        "São aceitos até quatro providers: PHP, TypeScript, Rust e Python".into(),
                    );
                }
                self.provider_configs = configs;
                self.provider_configs_explicit = true;
            }
            Command::StartProviders => {
                if let Some(doc) = self
                    .active_document()
                    .filter(|d| Language::for_path(&d.path) == Some(Language::Python))
                {
                    let root = doc.project_root().to_path_buf();
                    if !self.python.is_busy()
                        && self.python.config.as_ref().is_none_or(|c| c.root != root)
                    {
                        let roots = self.python_allowed_roots();
                        let _ = self
                            .python
                            .dispatch(python_project::ProjectCommand::Inspect(root), &roots);
                    }
                }
                self.start_configured_providers()?;
                if let Some(workspace) = &self.workspace {
                    self.automatic_providers
                        .authorize(workspace.clone(), self.provider_configs.clone());
                }
            }
            Command::EnableProviderAutostart {
                profile,
                fallback_config,
            } => {
                self.enable_provider_startup(profile, fallback_config)?;
            }
            Command::SetProvidersAutomatic(enabled) => {
                self.set_provider_automatic(enabled)?;
            }
            Command::StopProviders => {
                self.stop_project_languages()?;
                self.cancel_formatting();
                self.automatic_providers.suspend();
                self.cancel_navigation();
                self.cancel_completion();
                self.cancel_signature_help();
                self.cancel_references();
                self.cancel_rename();
                self.language_versions.clear();
                self.language_skipped_versions.clear();
                self.language_retries.clear();
                self.language.dispatch(LanguageCommand::Stop)?;
                self.provider_operation = Some(provider_startup::ProviderOperation::Stopping {
                    through_file_job: self.next_job.saturating_sub(1),
                });
                self.status = "Encerrando providers de linguagem".into();
            }
            Command::ExecuteSql {
                id,
                expected_version,
                selection,
            } => {
                if self.pending.values().any(|p| *p == Pending::Workspace) {
                    return Err("Aguarde a abertura da pasta antes de executar SQL".into());
                }
                let doc = self.document_mut(id)?;
                if doc.version != expected_version {
                    return Err("SQL obsoleto: documento mudou".into());
                }
                if doc.path.extension().and_then(|s| s.to_str()) != Some("sql") {
                    return Err(
                        "Abra um arquivo .sql para executar a seleção ou o documento".into(),
                    );
                }
                let sql = if let Some(range) = selection {
                    if range.start > range.end || range.end > doc.text().chars().count() {
                        return Err("Seleção SQL fora do documento".into());
                    }
                    doc.text()
                        .chars()
                        .skip(range.start)
                        .take(range.end - range.start)
                        .collect()
                } else {
                    doc.text().into()
                };
                self.db.dispatch(DbCommand::Execute { sql })?;
            }
        }
        Ok(())
    }

    fn report_search_error(&mut self, error: String, is_replacement: bool) {
        // A following debounced Find can clear the bar's transient error. Keep
        // rejected user mutations visible in the application's persistent error.
        if is_replacement {
            self.status = error.clone();
            self.last_error = Some(error.clone());
        }
        self.search_error = Some(error);
    }

    fn document_mut(&mut self, id: DocumentId) -> Result<&mut Document, String> {
        self.documents
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or_else(|| "Documento não encontrado".into())
    }

    fn guard_read_only_document(&self, command: &Command) -> Result<(), String> {
        let id = match command {
            Command::Edit { id, .. }
            | Command::EditDelta { id, .. }
            | Command::EditTransaction { id, .. }
            | Command::EditBatchTransaction { id, .. }
            | Command::ReplaceMatches { id, .. }
            | Command::Save { id }
            | Command::ReloadDocument { id, .. }
            | Command::OverwriteDocument { id, .. }
            | Command::SaveUntitled { id, .. }
            | Command::Undo { id }
            | Command::Redo { id }
            | Command::FormatDocument { id, .. }
            | Command::RestoreLocalRevision { id, .. }
            | Command::PrepareRename { id, .. }
            | Command::PrepareFileRename { id, .. }
            | Command::Completion { id, .. }
            | Command::CodeActions { id, .. } => Some(*id),
            Command::AcceptCompletion { .. } => self.active,
            _ => None,
        };
        if id.is_some_and(|id| {
            self.documents
                .iter()
                .any(|d| d.id == id && d.is_read_only())
        }) {
            return Err("Biblioteca em modo somente leitura; edição, formatação, rename e gravação estão bloqueados".into());
        }
        Ok(())
    }

    fn enqueue(&mut self, work: Work, pending: Pending) -> Result<(), String> {
        if self.pending.len() >= QUEUE_SIZE {
            return Err("Fila de arquivos cheia; aguarde ou cancele".into());
        }
        let id = self.next_job;
        self.sender
            .try_send(Job {
                workspace: self.workspace_path().map(PathBuf::from),
                id,
                generation: self.generation.load(Ordering::SeqCst),
                work,
            })
            .map_err(|_| "Worker indisponível ou fila cheia".to_owned())?;
        self.pending.insert(id, pending);
        self.next_job += 1;
        if !matches!(
            pending,
            Pending::Directory | Pending::GitTreeRead | Pending::GitRead(_)
        ) {
            self.last_error = None;
            self.status = "Operação de arquivo em andamento".into();
        }
        Ok(())
    }

    /// Nonblocking completion pump, shared by GUI, CLI and tests.
    pub fn poll(&mut self) -> bool {
        let mut changed = self.browser.poll();
        changed |= self.jira.poll();
        changed |= self.ai.poll();
        let commit_workspace = self.workspace_path().map(PathBuf::from);
        changed |= self.git_commit_messages.poll(commit_workspace.as_deref());
        changed |= self.docker.poll();
        changed |= self.api.poll();
        while let Ok(reply) = self.receiver.try_recv() {
            changed = true;
            let pending = self.pending.remove(&reply.id);
            self.project_tree.pending.remove(&reply.id);
            if let Some(Pending::Rename(token)) = pending
                && self.rename.context_token() != Some(token)
            {
                continue;
            }
            if let Some(Pending::References(token)) = pending
                && self.references.context_token() != Some(token)
            {
                continue;
            }
            if let Some(Pending::Navigation(token)) = pending
                && self.navigation.context_token() != Some(token)
            {
                continue;
            }
            if let Some(Pending::Completion(token)) = pending
                && self.completion.context_token() != Some(token)
            {
                continue;
            }
            let obsolete = reply.generation != self.generation.load(Ordering::SeqCst);
            // A committed write must always be acknowledged, even after cancellation.
            if obsolete
                && !matches!(
                    pending,
                    Some(
                        Pending::Save(_)
                            | Pending::Persist
                            | Pending::RenameResources
                            | Pending::ProjectFile
                    )
                )
            {
                self.status = "Operação cancelada; resposta de leitura descartada".into();
                continue;
            }
            match reply.result {
                Ok(completed) => {
                    if let Err(error) = self.apply(completed) {
                        self.status = error.clone();
                        self.last_error = Some(error);
                    }
                    self.sync_views();
                }
                Err(error) => {
                    if let Some(Pending::Save(id)) = pending {
                        self.auto_save.failed(id);
                    }
                    if pending == Some(Pending::ProjectFile) {
                        self.project_files.pending = false;
                        self.project_files.error = Some(error.clone());
                        self.project_files.clear_lock();
                        self.status =
                            "Operação de arquivos falhou; confira a árvore Project".into();
                        continue;
                    }
                    if matches!(pending, Some(Pending::RenameResources)) {
                        self.rename_stage = None;
                        self.rename_failed(error);
                        continue;
                    }
                    if matches!(pending, Some(Pending::Rename(_))) {
                        self.rename_failed(error);
                        continue;
                    }
                    if matches!(pending, Some(Pending::References(_))) {
                        self.references_failed(error);
                        continue;
                    }
                    if matches!(pending, Some(Pending::Completion(_))) {
                        self.completion_failed(error);
                        continue;
                    }
                    if matches!(pending, Some(Pending::Navigation(_))) {
                        self.navigation_failed(error);
                        continue;
                    }
                    if pending == Some(Pending::Restore) {
                        self.session_writable = false;
                    }
                    self.status = error.clone();
                    self.last_error = Some(error);
                }
            }
        }
        while let Some(reply) = self.search_worker.poll_one() {
            changed = true;
            let Some(reply) = reply else {
                continue;
            };
            let is_replacement = reply.is_replacement();
            let current = self
                .documents
                .iter()
                .any(|doc| doc.id == reply.document_id && doc.version == reply.version);
            if !current {
                if is_replacement {
                    self.report_search_error(
                        "Substituição descartada: documento fechado ou versão alterada".into(),
                        true,
                    );
                }
                continue;
            }
            match reply.result {
                Ok(search::Completion::Found(result)) => self.search = Some(result),
                Ok(search::Completion::Replaced(replacement)) => {
                    // Reuse Edit's validation/history and apply exactly one undo step.
                    if let Err(error) = self.execute(Command::Edit {
                        id: reply.document_id,
                        expected_version: reply.version,
                        text: replacement.text,
                    }) {
                        self.report_search_error(error, true);
                    } else {
                        self.status = format!(
                            "{} ocorrência(s) substituída(s); salve para gravar",
                            replacement.count
                        );
                    }
                }
                Err(error) => self.report_search_error(error, is_replacement),
            }
        }
        changed |= self.poll_git();
        changed |= self.poll_debugger();
        changed |= self.browser_debug_poll();
        changed |= self.browser_devtools_poll();
        changed |= self.browser_inspector_poll();
        changed |= self.browser_library.poll();
        changed |= self.poll_browser_login();
        if self.python.poll() {
            if let Some(error) = &self.python.error {
                self.last_error = Some(error.clone());
            } else if self
                .python
                .config
                .as_ref()
                .is_some_and(|c| self.language_project_root().as_ref() == Some(&c.root))
            {
                if self
                    .active_document()
                    .is_some_and(|d| Language::for_path(&d.path) == Some(Language::Python))
                {
                    self.cancel_completion();
                    self.cancel_navigation();
                    self.cancel_signature_help();
                    self.cancel_references();
                    self.cancel_rename();
                    self.cancel_inlay_hints();
                    self.cancel_code_actions();
                }
                let _ = self.language.dispatch(LanguageCommand::PythonEnvironment {
                    python: self.python.interpreter.clone(),
                });
            }
            changed = true;
        }
        changed |= self.python_tests.poll();
        changed |= self.php_tests.poll();
        changed |= self.poll_js_tests();
        changed |= self.db.poll();
        changed |= self.remote_db.poll();
        if let Some(path) = self.remote_db.console_deleted.take() {
            self.finish_database_console_delete(path);
        }
        if let Some(receipt) = self.remote_db.console_renamed.take() {
            self.finish_database_console_rename(receipt);
        }
        if let Some(path) = self.remote_db.console_path.take() {
            let _ = self.dispatch(Command::OpenFile(path));
        }
        changed |= self.poll_project_languages();
        changed |= self.language.poll();
        changed |= self.poll_provider_operation();
        if self.db.connected && !self.db.is_busy() {
            self.database_path = self.db.config.clone();
        }
        let mut rename_sync_error = false;
        let mut references_sync_error = None;
        let mut navigation_sync_error = None;
        let mut completion_sync_error = None;
        let mut signature_sync_error = None;
        let mut formatting_sync_error = None;
        for (path, failure) in self.language.sync_failures.drain() {
            if !self
                .documents
                .iter()
                .any(|d| d.path == path && d.version == failure.version)
            {
                continue;
            }
            if self.rename.is_open() {
                rename_sync_error = true;
            }
            if self.references.is_open() {
                references_sync_error = Some(format!(
                    "Referências canceladas: falha de sincronização: {}",
                    failure.message
                ));
            }
            if self.navigation.context_token().is_some() {
                navigation_sync_error = Some(format!(
                    "Definição cancelada: falha de sincronização de {}: {}",
                    path.display(),
                    failure.message
                ));
            }
            if self.formatting.is_pending() {
                formatting_sync_error = Some(format!(
                    "Formatação cancelada: falha de sincronização: {}",
                    failure.message
                ));
            }
            if self.signature_help.is_open() {
                signature_sync_error = Some(format!(
                    "Assinatura cancelada: falha de sincronização: {}",
                    failure.message
                ));
            }
            if self.completion.is_open() {
                completion_sync_error = Some(format!(
                    "Sugestões canceladas: falha de sincronização de {}: {}",
                    path.display(),
                    failure.message
                ));
            }
            let attempt = self
                .language_retries
                .entry(path.clone())
                .or_insert((failure.version, 0));
            if attempt.0 != failure.version {
                *attempt = (failure.version, 0);
            }
            if attempt.1 < 2 {
                attempt.1 += 1;
                self.language_versions.remove(&path);
                self.status = format!("Reenviando documento ao provider: tentativa {}", attempt.1);
            } else {
                self.last_error = Some(format!(
                    "Provider não recebeu {} após três tentativas: {}. Reinicie o provider ou edite novamente.",
                    path.display(),
                    failure.message
                ));
            }
        }
        if rename_sync_error {
            self.rename_failed("Falha de sincronização durante rename".into());
        }
        if let Some(error) = references_sync_error {
            self.references_failed(error);
            changed = true;
        }
        if let Some(error) = navigation_sync_error {
            self.navigation_failed(error);
            changed = true;
        }
        if let Some(error) = completion_sync_error {
            self.completion_failed(error);
            changed = true;
        }
        if let Some(error) = signature_sync_error {
            self.signature_help_failed(error);
            changed = true;
        }
        if let Some(e) = formatting_sync_error {
            self.formatting_failed(e);
            changed = true;
        }
        changed |= self.poll_external_files();
        changed |= self.local_history.poll();
        if let Some(warning) = self.local_history.warning.take() {
            self.last_error = Some(warning);
            changed = true;
        }
        self.sync_documents();
        changed |= self.poll_code_actions();
        changed |= self.poll_inlay_hints();
        changed |= self.poll_formatting();
        changed |= self.poll_signature_help();
        changed |= self.poll_definition();
        changed |= self.poll_completion();
        changed |= self.poll_references();
        changed |= self.poll_rename();
        changed |= self.poll_provider_startup();
        self.sync_views();
        let ai_root = self.ai_workspace().map(PathBuf::from);
        self.ai.sync_workspace(ai_root.as_deref());
        changed
    }

    fn insert_document(&mut self, file: OpenedFile) -> Result<DocumentId, String> {
        if let Some(doc) = self.documents.iter().find(|d| d.path == file.path) {
            return Ok(doc.id);
        }
        if self.documents.len() >= MAX_DOCUMENTS {
            return Err("Limite M1 de 32 documentos abertos".into());
        }
        let buffer = TextBuffer::new(file.text.clone())?;
        let id = self.next_document;
        self.next_document += 1;
        self.project_encodings
            .entry(file.project_root.clone())
            .or_insert(file.encoding);
        self.documents.push(Document {
            project_root: file.project_root,
            id,
            path: file.path,
            version: 1,
            buffer,
            saved_text: Arc::from(file.text),
            saved_identity: Some(1),
            dirty_cache: std::cell::Cell::new(None),
            disk_missing: false,
            untitled: false,
            encoding: file.encoding,
            read_only: false,
        });
        let doc = self.documents.last().unwrap();
        self.local_history
            .record(doc.path.clone(), None, doc.saved_text.clone(), "Opened");
        Ok(id)
    }

    fn apply(&mut self, completed: Completed) -> Result<(), String> {
        match completed {
            Completed::Directory {
                root,
                path,
                revision,
                result,
            } => {
                if self.workspace.as_ref() == Some(&root) && self.project_tree.revision == revision
                {
                    match result {
                        Ok(children) => {
                            project_tree::merge(
                                &mut self.entries,
                                self.workspace_file.as_deref().unwrap_or(&root),
                                &path,
                                children,
                            );
                            self.project_tree.loaded.insert(path.clone());
                            self.project_tree.errors.remove(&path);
                            // In-flight replies carry the same base revision; they
                            // merge disjoint directories without invalidating peers.
                        }
                        Err(error) => {
                            self.project_tree.errors.insert(path, error);
                        }
                    }
                }
            }
            Completed::Encoding {
                root,
                encoding,
                files,
            } => {
                for (id, file) in files {
                    let doc = self.document_mut(id)?;
                    let selection = doc.selection();
                    doc.buffer = TextBuffer::new(file.text.clone())?;
                    doc.buffer.set_selection(Selection {
                        anchor: selection.anchor.min(doc.buffer.len_chars()),
                        head: selection.head.min(doc.buffer.len_chars()),
                    })?;
                    doc.version += 1;
                    doc.saved_text = Arc::from(file.text);
                    doc.saved_identity = Some(doc.buffer.content_identity());
                    doc.dirty_cache.set(None);
                    doc.disk_missing = false;
                    doc.encoding = file.encoding;
                    doc.project_root = file.project_root;
                }
                self.project_encodings.insert(root.clone(), encoding);
                if self.workspace.as_ref() == Some(&root) {
                    self.project_encoding = encoding;
                }
                self.status = format!(
                    "Codificação do projeto: {}; preferência salva",
                    encoding.label()
                );
            }
            Completed::ProjectFile {
                receipt,
                opened,
                project_roots,
                refresh,
            } => self.accept_project_file(receipt, opened, project_roots, refresh)?,
            Completed::GitReload(files) => self.finish_git_reload(files)?,
            Completed::RenameResources(result) => self.finish_resource_rename(result),
            Completed::RenameInspected { token, name, kind } => {
                self.finish_rename_inspection(token, name, kind)?
            }
            Completed::Rename {
                token,
                prepared,
                apply,
            } => {
                if let Err(e) = self.finish_rename(token, *prepared, apply) {
                    self.rename_failed(e);
                }
            }
            Completed::ReferencePreviews {
                token,
                items,
                cache,
            } => {
                if let Err(e) = self.finish_reference_previews(token, items, cache) {
                    self.references_failed(e);
                }
            }
            Completed::ReferenceOpen {
                token,
                target,
                file,
                read_only,
            } => {
                if let Err(e) = self.finish_reference_open(token, target, file, read_only) {
                    self.references_failed(e);
                }
            }
            Completed::Completion {
                token,
                id,
                version,
                text,
                caret,
            } => self.finish_completion(token, id, version, text, caret)?,
            Completed::Navigation {
                token,
                target,
                file,
                read_only,
            } => {
                if let Err(error) = self.finish_navigation(token, target, file, read_only) {
                    self.navigation_failed(error);
                }
            }
            Completed::Created(file) => {
                self.apply(Completed::Open(file))?;
                self.status = "Arquivo criado; digite e salve com Ctrl+S".into();
                if let Some(root) = self.workspace_path().map(PathBuf::from) {
                    let _ = self.enqueue(Work::Workspace(root), Pending::Read);
                }
            }
            Completed::Open(file) => {
                self.cancel_references();
                self.cancel_rename();
                self.clear_search();
                let id = self.insert_document(file)?;
                // Opening through tree/path prefers an existing local instance.
                if !self
                    .views
                    .iter()
                    .any(|v| Some(v.id) == self.active_view && v.tabs.contains(&id))
                    && let Some(owner) = self
                        .views
                        .iter()
                        .find(|v| v.tabs.contains(&id))
                        .map(|v| v.id)
                {
                    self.activate_view(owner)?;
                }
                self.active = Some(id);
                self.status = "Arquivo aberto".into();
            }
            Completed::Workspace(workspace) => {
                if self.workspace_path()
                    != Some(workspace.file.as_deref().unwrap_or(&workspace.path))
                {
                    if self.documents.iter().any(Document::is_dirty) {
                        return Err(
                            "Troca de pasta interrompida: surgiram alterações não salvas".into(),
                        );
                    }
                    self.stop_project_languages()?;
                    if self.providers_running() {
                        self.language.dispatch(LanguageCommand::Stop)?;
                    }
                    if self.db.connected {
                        self.db.dispatch(DbCommand::Disconnect)?;
                    }
                }
                let project_identity = workspace
                    .file
                    .clone()
                    .unwrap_or_else(|| workspace.path.clone());
                let database_change = self
                    .remote_db
                    .prepare_project_change(Some(project_identity.clone()))?;
                self.api.set_project_with_root(
                    project_identity.clone(),
                    workspace
                        .folders
                        .iter()
                        .find(|f| f.available)
                        .map(|f| f.path.clone())
                        .or_else(|| workspace.file.is_none().then(|| workspace.path.clone())),
                )?;
                self.remote_db.apply_project_change(database_change);
                if self.workspace_path()
                    != Some(workspace.file.as_deref().unwrap_or(&workspace.path))
                {
                    self.git_workspace = None;
                    self.git_sessions.clear();
                    self.git_target = None;
                    self.language_versions.clear();
                    self.language_skipped_versions.clear();
                    self.language_retries.clear();
                    self.database_path = None;
                    self.clear_search();
                    self.navigation = NavigationState::default();
                    self.documents.clear();
                    self.active = None;
                }
                self.project_encoding = workspace.encoding;
                self.workspace = Some(workspace.path.clone());
                self.automatic_providers.opened_workspace();
                self.accept_tree(workspace);
                self.status = "Pasta aberta; nenhum script foi executado".into();
            }
            Completed::SaveConflict {
                id,
                baseline,
                file,
                error,
            } => {
                self.auto_save.failed(id);
                self.accept_save_conflict(id, baseline, file, error)?;
            }
            Completed::ReloadDocument {
                id,
                version,
                external_revision,
                baseline,
                file,
            } => {
                self.accept_external_reload(id, version, external_revision, baseline, file)?;
            }
            Completed::Saved {
                previous,
                new_path,
                id,
                text,
                identity,
                warning,
            } => {
                self.auto_save.saved(id);
                self.clear_external_document_change(id);
                if let Some(doc) = self.documents.iter_mut().find(|d| d.id == id) {
                    let saved_changed = doc.saved_text != text || new_path.is_some();
                    if let Some(path) = new_path {
                        doc.project_root = projects::resolve_root(None, &path);
                        doc.path = path;
                        doc.untitled = false;
                        doc.version += 1;
                    }
                    doc.disk_missing = false;
                    doc.saved_identity = Some(identity);
                    doc.saved_text = text;
                    doc.dirty_cache.set(None);
                    if saved_changed {
                        self.js_tests.saved(&doc.path);
                    }
                    self.local_history.record(
                        doc.path.clone(),
                        previous,
                        doc.saved_text.clone(),
                        "Saved",
                    );
                }
                self.status = warning.unwrap_or_else(|| "Arquivo salvo".into());
            }
            Completed::Persisted => {
                self.status = "Sessão salva".into();
            }
            Completed::ProviderConfig { root, configs } => {
                if let Some(root) = root {
                    self.configure_project_providers(root, configs)?;
                } else {
                    self.execute(Command::ConfigureProviders(configs))?;
                }
                self.status =
                    "Providers configurados; inicie explicitamente para analisar o projeto".into();
            }
            Completed::Restored(restored) => {
                if let Some(workspace) = &restored.workspace {
                    let project_identity = workspace
                        .file
                        .clone()
                        .unwrap_or_else(|| workspace.path.clone());
                    let database_change = self
                        .remote_db
                        .prepare_project_change(Some(project_identity.clone()))?;
                    self.api.set_project_with_root(
                        project_identity.clone(),
                        workspace
                            .folders
                            .iter()
                            .find(|f| f.available)
                            .map(|f| f.path.clone())
                            .or_else(|| workspace.file.is_none().then(|| workspace.path.clone())),
                    )?;
                    self.remote_db.apply_project_change(database_change);
                }
                self.shell_layout = restored.shell_layout;
                self.git_drafts = restored.git_drafts;
                self.editor_preferences = restored.editor_preferences;
                self.project_preferences = restored.project_preferences;
                self.tool_window_preferences = restored.tool_window_preferences;
                self.database_path = restored.database_path;
                self.provider_configs = restored.provider_configs;
                self.provider_configs_explicit = false;
                if let Some(workspace) = restored.workspace {
                    self.project_encoding = workspace.encoding;
                    self.workspace = Some(workspace.path.clone());
                    self.accept_tree(workspace);
                }
                for file in restored.files {
                    self.insert_document(file)?;
                }
                self.active = self
                    .documents
                    .iter()
                    .find(|d| Some(&d.path) == restored.active.as_ref())
                    .or_else(|| self.documents.first())
                    .map(|d| d.id);
                if let Some(state) = restored.editor_state {
                    self.restore_editor_state(state);
                }
                self.status = if restored.omitted_database_consoles {
                    "Sessão restaurada; consultas antigas preservadas fora desta janela".into()
                } else {
                    "Sessão restaurada".into()
                };
                self.session_writable = true;
            }
        }
        Ok(())
    }
}

impl Drop for AppService {
    fn drop(&mut self) {
        // No blocking join on the UI thread. The worker exits when channels disconnect.
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// Convert a Unicode scalar caret into an LSP UTF-16 position, rejecting the
/// interior of CRLF rather than sending a position beyond the logical line.
fn scalar_lsp_position(text: &str, index: usize) -> Option<(u32, u32)> {
    let byte = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .nth(index)?;
    let prefix = &text[..byte];
    let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
    let tail = prefix.rsplit('\n').next()?;
    let column = tail.encode_utf16().count() as u32;
    (utf16_byte_offset(text, line, column) == Some(byte)).then_some((line, column))
}

#[cfg(test)]
mod hover_position_tests {
    use super::scalar_lsp_position;
    #[test]
    fn scalar_caret_maps_emoji_combining_and_crlf_without_clamping() {
        let text = "😀e\u{301}\r\nvalue";
        assert_eq!(scalar_lsp_position(text, 1), Some((0, 2)));
        assert_eq!(scalar_lsp_position(text, 3), Some((0, 4)));
        assert_eq!(scalar_lsp_position(text, 4), None);
        assert_eq!(scalar_lsp_position(text, 5), Some((1, 0)));
        assert_eq!(scalar_lsp_position(text, 10), Some((1, 5)));
        assert_eq!(scalar_lsp_position(text, 11), None);
    }
}

#[cfg(test)]
mod large_sync_guard_tests {
    use super::*;
    #[test]
    fn oversized_documents_are_skipped_once_without_queuing_sync_or_repeating_status() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = AppService::new(dir.path().join("session.json"));
        let path = dir.path().join("large.ts");
        app.insert_document(OpenedFile {
            project_root: PathBuf::from("/large"),
            encoding: FileEncoding::Utf8,
            path: path.clone(),
            text: "a".repeat(codisseu_editor::MAX_DOCUMENT_BYTES + 1),
        })
        .unwrap();
        app.language
            .statuses
            .push(codisseu_language::ProviderStatus {
                language: Language::TypeScript,
                state: ProviderState::Ready,
                detail: "test: dispatch guard only, no real provider".into(),
            });
        app.sync_documents();
        assert!(app.status.contains("1 MiB"));
        assert_eq!(app.language_skipped_versions.get(&path), Some(&1));
        assert!(!app.language_versions.contains_key(&path));
        assert!(!app.language.is_busy());
        app.status = "later user action".into();
        for _ in 0..100 {
            app.sync_documents();
        }
        assert_eq!(app.status, "later user action");
        assert!(!app.language.is_busy());
    }
}

impl AppService {
    pub fn python_interpreter_for(&self, path: &std::path::Path) -> Option<PathBuf> {
        if self.python.is_busy() || self.python.error.is_some() {
            return None;
        }
        self.python
            .config
            .as_ref()
            .filter(|c| path.starts_with(&c.root))?;
        self.python.interpreter.clone()
    }
}

impl AppService {
    pub fn inlay_hints_enabled(&self, id: DocumentId) -> bool {
        self.documents.iter().find(|d| d.id == id).is_some_and(|d| {
            match Language::for_path(&d.path) {
                Some(Language::Rust) => self.editor_preferences.rust_inlay_hints,
                Some(Language::Python) => self.editor_preferences.python_inlay_hints,
                _ => false,
            }
        })
    }
    fn dispatch_python(&mut self, command: python_project::ProjectCommand) -> Result<(), String> {
        if let python_project::ProjectCommand::Prepare { config, action, .. } = &command
            && action.executes_project()
            && self
                .documents
                .iter()
                .any(|d| d.is_dirty() && d.path.starts_with(&config.root))
        {
            return Err(
                "Salve os arquivos modificados antes de executar Python; nenhum Save automático"
                    .into(),
            );
        }
        let roots = self.python_allowed_roots();
        self.python.dispatch(command, &roots)
    }
}

impl AppService {
    fn dispatch_python_tests(&mut self, command: python_tests::TestCommand) -> Result<(), String> {
        let root = match &command {
            python_tests::TestCommand::Inspect(r) => Some(r.as_path()),
            python_tests::TestCommand::Run { config, .. } => Some(config.root.as_path()),
            python_tests::TestCommand::Cancel => None,
            _ => self.python_tests.last_root(),
        };
        if let Some(root) = root
            && !matches!(command, python_tests::TestCommand::Inspect(_))
            && self
                .documents
                .iter()
                .any(|d| d.is_dirty() && d.path.starts_with(root))
        {
            return Err("Salve os arquivos modificados antes de executar testes Python".into());
        }
        if let Some(c) = &self.python.config {
            self.python_tests
                .set_project_interpreter(c.root.clone(), self.python.interpreter.clone());
        }
        let roots = self.python_allowed_roots();
        self.python_tests.dispatch(command, roots)
    }
}

impl AppService {
    fn python_allowed_roots(&self) -> Vec<PathBuf> {
        let mut roots = self.workspace_roots();
        for d in &self.documents {
            if !d.is_read_only() && !roots.iter().any(|r| r == d.project_root()) {
                roots.push(d.project_root().to_path_buf());
            }
        }
        roots
    }
}
