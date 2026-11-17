mod pipe;
mod rust_features;
use crate::{
    CompletionResult, DefinitionResult, DefinitionTarget, Diagnostic, Envelope, Event,
    FormattingCapabilities, FormattingResult, HoverResult, Language, LanguageCommand,
    PrepareRenameResult, ProviderConfig, ProviderState, ProviderStatus, ReferencesResult,
    RenameRange, RenameResult, Request, ServerChoice, SignatureHelpCapabilities,
    SignatureHelpResult, SignatureHelpTrigger, SyncFailure, protocol,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    io::Read,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

struct Document {
    path: PathBuf,
    version: u64,
    text: String,
    language: Language,
}
struct PendingRename {
    id: u64,
    path: PathBuf,
    canonical: PathBuf,
    version: u64,
    line: u32,
    character: u32,
    new_name: Option<String>,
    started: Instant,
    closed_text: Option<String>,
}
impl PendingRename {
    fn wire_id(&self) -> String {
        format!(
            "{}{}",
            if self.new_name.is_some() {
                "rename-"
            } else {
                "prepare-rename-"
            },
            self.id
        )
    }
    fn error(&self, error: String) -> Event {
        if self.new_name.is_some() {
            Event::Rename {
                id: self.id,
                result: Err(error),
            }
        } else {
            Event::PrepareRename {
                id: self.id,
                result: Err(error),
            }
        }
    }
}
#[derive(Clone, Copy, PartialEq)]
enum ReferenceFamilyPhase {
    Original,
    Definition,
    Base,
}
struct ReferenceFamily {
    method: String,
    phase: ReferenceFamilyPhase,
}
struct PendingReferences {
    id: u64,
    result: ReferencesResult,
    started: Instant,
    definition_phase: bool,
    declarations: Vec<DefinitionTarget>,
    source_uri: String,
    family: Option<ReferenceFamily>,
}
struct PendingCompletion {
    id: u64,
    result: CompletionResult,
    canonical: PathBuf,
    started: Instant,
}
struct PendingDefinition {
    id: u64,
    implementation: bool,
    type_definition: bool,
    super_search: Option<crate::php_super::Search>,
    super_depth: usize,
    result: DefinitionResult,
    started: Instant,
}
impl PendingDefinition {
    fn wire_id(&self) -> String {
        match &self.super_search {
            Some(search) => format!("super-{}-{}", self.id, search.step),
            None => format!("definition-{}", self.id),
        }
    }
}
struct PendingSignature {
    id: u64,
    result: SignatureHelpResult,
    started: Instant,
}
struct PendingFormatting {
    id: u64,
    canonical: PathBuf,
    result: FormattingResult,
    started: Instant,
}
struct PendingHover {
    id: u64,
    result: HoverResult,
    started: Instant,
}
enum ChoiceDecode {
    Malformed,
    Overflow,
}
fn decode_server_choice(value: &Value) -> Result<(String, Vec<Value>), ChoiceDecode> {
    let params = value
        .get("params")
        .and_then(Value::as_object)
        .ok_or(ChoiceDecode::Malformed)?;
    if !params
        .get("type")
        .and_then(Value::as_u64)
        .is_some_and(|t| (1..=4).contains(&t))
    {
        return Err(ChoiceDecode::Malformed);
    }
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .ok_or(ChoiceDecode::Malformed)?;
    if message.len() > 16384 {
        return Err(ChoiceDecode::Overflow);
    }
    let actions = match params.get("actions") {
        None => vec![],
        Some(v) => {
            let actions = v.as_array().ok_or(ChoiceDecode::Malformed)?;
            if actions.len() > 64 {
                return Err(ChoiceDecode::Overflow);
            }
            actions.clone()
        }
    };
    if actions.len() > 64 {
        return Err(ChoiceDecode::Overflow);
    }
    let mut bytes = 0;
    for action in &actions {
        let title = action
            .as_object()
            .and_then(|a| a.get("title"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or(ChoiceDecode::Malformed)?;
        if title.len() > 512 {
            return Err(ChoiceDecode::Overflow);
        }
        bytes += action.to_string().len();
        if bytes > 65536 {
            return Err(ChoiceDecode::Overflow);
        }
    }
    Ok((message.into(), actions))
}
static NEXT_SERVER_CHOICE: AtomicU64 = AtomicU64::new(1);
struct PendingServerChoice {
    token: u64,
    id: Value,
    actions: Vec<Value>,
    owner: u64,
    started: Instant,
}
struct Connection {
    rust_features: rust_features::State,
    server_choice: Option<PendingServerChoice>,
    formatting_capabilities: FormattingCapabilities,
    formatting: Option<PendingFormatting>,
    signature_help_capabilities: Option<SignatureHelpCapabilities>,
    signature_help: Option<PendingSignature>,
    rename_supported: bool,
    prepare_rename_supported: bool,
    rename: Option<PendingRename>,
    transient_document: Option<PathBuf>,
    reference_transient: Option<PathBuf>,
    typescript_reference_adapter: bool,
    typescript_action_adapter: bool,
    references_supported: bool,
    references: Option<PendingReferences>,
    completion_supported: bool,
    completion: Option<PendingCompletion>,
    definition_supported: bool,
    implementation_supported: bool,
    type_definition_supported: bool,
    super_transient: Option<PathBuf>,
    definition: Option<PendingDefinition>,
    hover_supported: bool,
    hover: Option<PendingHover>,
    language: Language,
    python_environment: Option<PathBuf>,
    child: Child,
    output: Receiver<Result<Value, String>>,
    input: Option<SyncSender<Value>>,
    alive: Arc<AtomicBool>,
    tasks: Vec<thread::JoinHandle<()>>,
    state: ProviderState,
    started: Instant,
    phpactor_index: bool,
    phpactor_completion: bool,
    index_progress: Option<crate::index_progress::IndexProgress>,
    opened: HashMap<PathBuf, u64>,
    _stderr: Arc<Mutex<VecDeque<u8>>>,
}
fn emit(tx: &SyncSender<Envelope>, stop: &AtomicBool, epoch: u64, event: Event) {
    let mut value = Envelope { epoch, event };
    loop {
        match tx.try_send(value) {
            Ok(()) => return,
            Err(TrySendError::Disconnected(_)) => return,
            Err(TrySendError::Full(v)) => {
                value = v;
                if stop.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
fn bounded_output(
    tx: &SyncSender<Result<Value, String>>,
    alive: &AtomicBool,
    mut value: Result<Value, String>,
) {
    loop {
        match tx.try_send(value) {
            Ok(()) => return,
            Err(TrySendError::Disconnected(_)) => return,
            Err(TrySendError::Full(v)) => {
                value = v;
                if !alive.load(Ordering::Acquire) {
                    return;
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
impl Connection {
    fn semantic_owner(&self) -> Option<u64> {
        let owners = [
            self.definition.as_ref().map(|r| r.id),
            self.references.as_ref().map(|r| r.id),
            self.completion.as_ref().map(|r| r.id),
            self.hover.as_ref().map(|r| r.id),
            self.rename.as_ref().map(|r| r.id),
            self.formatting.as_ref().map(|r| r.id),
            self.signature_help.as_ref().map(|r| r.id),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        (owners.len() == 1).then(|| owners[0])
    }
    fn choice_owner_current(&self, owner: u64) -> bool {
        self.definition.as_ref().is_some_and(|r| r.id == owner)
            || self.references.as_ref().is_some_and(|r| r.id == owner)
            || self.completion.as_ref().is_some_and(|r| r.id == owner)
            || self.hover.as_ref().is_some_and(|r| r.id == owner)
            || self.rename.as_ref().is_some_and(|r| r.id == owner)
            || self.formatting.as_ref().is_some_and(|r| r.id == owner)
            || self.signature_help.as_ref().is_some_and(|r| r.id == owner)
    }
    fn answer_choice(
        &mut self,
        index: Option<usize>,
        tx: &SyncSender<Envelope>,
        stop: &AtomicBool,
        epoch: u64,
    ) -> Result<(), String> {
        if index.is_some_and(|i| {
            self.server_choice
                .as_ref()
                .is_none_or(|c| i >= c.actions.len())
        }) {
            return Err("Opção do provider fora do intervalo".into());
        }
        let Some(choice) = self.server_choice.take() else {
            return Err("Escolha do provider já encerrada".into());
        };
        let elapsed = choice.started.elapsed();
        macro_rules! resume {
            ($field:ident) => {
                if let Some(pending) = &mut self.$field
                    && pending.id == choice.owner
                {
                    pending.started += elapsed;
                }
            };
        }
        resume!(definition);
        resume!(references);
        resume!(completion);
        resume!(hover);
        resume!(rename);
        resume!(formatting);
        resume!(signature_help);
        emit(tx, stop, epoch, Event::ServerChoiceClosed(choice.token));
        let result = index.map_or(Value::Null, |i| choice.actions[i].clone());
        self.send(json!({"jsonrpc":"2.0","id":choice.id,"result":result}))
    }

    fn start(
        config: ProviderConfig,
        workspace: &std::path::Path,
        python: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        let mut command = Command::new(&config.program);
        command
            .args(&config.args)
            .current_dir(workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Deliberately avoid inheriting tokens, passwords and unrelated application settings.
        command.env_clear();
        for key in [
            "HOME",
            "USERPROFILE",
            "PATH",
            "SYSTEMROOT",
            "TMPDIR",
            "TEMP",
            "XDG_CACHE_HOME",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        if config.language == Language::Rust {
            for key in [
                "CARGO_HOME",
                "RUSTUP_HOME",
                "RUSTUP_TOOLCHAIN",
                "RUSTC",
                "RUSTDOC",
                "RUSTFMT",
            ] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
            if let Some(resources) = codisseu_runtime::discover()?
                && resources
                    .providers()?
                    .iter()
                    .any(|p| p.language == "Rust" && p.program == config.program)
            {
                command.envs(codisseu_runtime::rust_environment()?);
            }
        }
        command.env("LANG", "C.UTF-8").env("XDEBUG_MODE", "off");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().map_err(|e| {
            format!(
                "{:?}: não foi possível iniciar {}: {e}",
                config.language,
                config.program.display()
            )
        })?;
        let stdout = child.stdout.take().ok_or("LSP stdout indisponível")?;
        let stdin = child.stdin.take().ok_or("LSP stdin indisponível")?;
        let stderr = child.stderr.take().ok_or("LSP stderr indisponível")?;
        let alive = Arc::new(AtomicBool::new(true));
        let alive_reader = alive.clone();
        let alive_writer = alive.clone();
        let (tx, output) = mpsc::sync_channel(8);
        let writer_tx = tx.clone();
        let reader = thread::spawn(move || {
            let mut stdout = match pipe::Pipe::new(stdout, alive_reader.clone()) {
                Ok(pipe) => pipe,
                Err(error) => {
                    bounded_output(&tx, &alive_reader, Err(format!("LSP stdout: {error}")));
                    return;
                }
            };
            while alive_reader.load(Ordering::Acquire) {
                match protocol::read_frame(&mut stdout) {
                    Ok(value) => bounded_output(&tx, &alive_reader, Ok(value)),
                    Err(error) => {
                        bounded_output(&tx, &alive_reader, Err(error));
                        break;
                    }
                }
            }
        });
        let (input, rx) = mpsc::sync_channel(32);
        let writer = thread::spawn(move || {
            let mut stdin = match pipe::Pipe::new(stdin, alive_writer.clone()) {
                Ok(pipe) => pipe,
                Err(error) => {
                    bounded_output(
                        &writer_tx,
                        &alive_writer,
                        Err(format!("LSP stdin: {error}")),
                    );
                    return;
                }
            };
            while let Ok(value) = rx.recv() {
                if let Err(error) = protocol::write_frame(&mut stdin, &value) {
                    bounded_output(&writer_tx, &alive_writer, Err(error.to_string()));
                    break;
                }
            }
        });
        let stderr_buffer = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_read = stderr_buffer.clone();
        let alive_stderr = alive.clone();
        let stderr_task = thread::spawn(move || {
            let Ok(mut stderr) = pipe::Pipe::new(stderr, alive_stderr) else {
                return;
            };
            let mut buffer = [0; 1024];
            while let Ok(count) = stderr.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                if let Ok(mut captured) = stderr_read.lock() {
                    for byte in &buffer[..count] {
                        if captured.len() == 32768 {
                            captured.pop_front();
                        }
                        captured.push_back(*byte);
                    }
                }
            }
        });
        let typescript_package = if config.language == Language::TypeScript {
            config
                .args
                .first()
                .and_then(|script| {
                    std::path::Path::new(script)
                        .parent()?
                        .parent()
                        .map(|root| root.join("package.json"))
                })
                .and_then(|path| {
                    let mut bytes = Vec::new();
                    std::fs::File::open(path)
                        .ok()?
                        .take(16385)
                        .read_to_end(&mut bytes)
                        .ok()?;
                    Some(bytes)
                })
                .filter(|bytes| bytes.len() <= 16384)
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        } else {
            None
        };
        let python_environment = if config.language == Language::Python {
            python
                .map(PathBuf::from)
                .or_else(|| codisseu_runtime::python().ok())
        } else {
            None
        };
        let instance = Self {
            rust_features: rust_features::State::default(),
            server_choice: None,
            rename_supported: false,
            prepare_rename_supported: false,
            rename: None,
            transient_document: None,
            reference_transient: None,
            // TLS omits serverInfo. Its configured package metadata identifies
            // the adapter; unrelated TypeScript LSP providers keep standard behavior.
            typescript_reference_adapter: typescript_package
                .as_ref()
                .is_some_and(|p| p["name"] == "typescript-language-server"),
            typescript_action_adapter: typescript_package.as_ref().is_some_and(|p| {
                p["name"] == "typescript-language-server" && p["version"] == "6.0.2"
            }),
            references_supported: false,
            references: None,
            signature_help_capabilities: None,
            signature_help: None,
            completion_supported: false,
            completion: None,
            definition_supported: false,
            implementation_supported: false,
            type_definition_supported: false,
            super_transient: None,
            definition: None,
            formatting_capabilities: FormattingCapabilities::default(),
            formatting: None,
            hover_supported: false,
            hover: None,
            language: config.language,
            python_environment,
            child,
            output,
            input: Some(input),
            alive,
            tasks: vec![reader, writer, stderr_task],
            state: ProviderState::Starting,
            started: Instant::now(),
            phpactor_index: false,
            phpactor_completion: false,
            index_progress: None,
            opened: HashMap::new(),
            _stderr: stderr_buffer,
        };
        let uri = protocol::file_uri(workspace)?;
        let mut options = json!({});
        if config.language == Language::Rust {
            options = crate::rust_settings::options();
        }
        if config.language == Language::TypeScript
            && let Some(script) = config.args.first()
            && let Some(modules) = std::path::Path::new(script)
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
        {
            options = json!({"tsserver":{"path":modules.join("typescript/lib/tsserver.js")},"disableAutomaticTypingAcquisition":true});
        }
        let mut initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "processId":std::process::id(),"clientInfo":{"name":"Codisseu","version":"0.1.0"},
            "rootUri":uri,"workspaceFolders":[{"uri":uri,"name":"Codisseu workspace"}],
            "capabilities":{"window":{"workDoneProgress":true},"general":{"positionEncodings":["utf-16"]},"workspace":{"configuration":true,"applyEdit":false,"workspaceEdit":{"documentChanges":true,"resourceOperations":[]}},"textDocument":{"formatting":{"dynamicRegistration":false},"rangeFormatting":{"dynamicRegistration":false},"signatureHelp":{"dynamicRegistration":false,"contextSupport":true,"signatureInformation":{"documentationFormat":["plaintext"],"parameterInformation":{"labelOffsetSupport":true},"activeParameterSupport":true}},"rename":{"dynamicRegistration":false,"prepareSupport":true,"honorsChangeAnnotations":false},"references":{"dynamicRegistration":false},"completion":{"contextSupport":true,"insertTextMode":1,"completionItem":{"snippetSupport":false,"commitCharactersSupport":false,"documentationFormat":["plaintext"],"insertReplaceSupport":true,"insertTextModeSupport":{"valueSet":[1]}},"completionList":{"itemDefaults":["editRange","insertTextFormat","insertTextMode"]}},"definition":{"linkSupport":true},"typeDefinition":{"dynamicRegistration":false,"linkSupport":true},"implementation":{"dynamicRegistration":false,"linkSupport":true},"hover":{"contentFormat":["plaintext"]},"synchronization":{"didSave":false},"publishDiagnostics":{"versionSupport":true,"tagSupport":{"valueSet":[1]}}}},
            "initializationOptions":options
        }});
        {
            initialize["params"]["capabilities"]["textDocument"]["codeAction"] = json!({"dynamicRegistration":false,"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["quickfix","refactor","refactor.rewrite","refactor.extract","refactor.inline"]}},"isPreferredSupport":true,"disabledSupport":true,"dataSupport":true,"resolveSupport":{"properties":["edit"]}});
        }
        if matches!(config.language, Language::Rust | Language::Python) {
            initialize["params"]["capabilities"]["textDocument"]["inlayHint"] =
                json!({"dynamicRegistration":false});
        }
        instance.send(initialize)?;
        Ok(instance)
    }
    fn ready(
        &mut self,
        documents: &HashMap<PathBuf, Document>,
        tx: &SyncSender<Envelope>,
        stop: &AtomicBool,
        epoch: u64,
    ) -> Result<(), String> {
        if self.language == Language::Python {
            self.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeConfiguration","params":{"settings":crate::python_settings::settings(self.python_environment.as_deref())}}))?;
        }
        self.state = ProviderState::Ready;
        for (path, document) in documents {
            self.sync(path, document)?;
        }
        emit(
            tx,
            stop,
            epoch,
            Event::Status(ProviderStatus {
                language: self.language,
                state: ProviderState::Ready,
                detail: "LSP inicializado · UTF-16 · diagnósticos reais".into(),
            }),
        );
        Ok(())
    }
    fn send(&self, value: Value) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("Provider encerrado")?
            .try_send(value)
            .map_err(|_| "Fila do provider cheia ou processo indisponível".into())
    }
    fn sync(&mut self, canonical: &PathBuf, document: &Document) -> Result<(), String> {
        if self.state != ProviderState::Ready || self.language != document.language {
            return Ok(());
        }
        let uri = protocol::file_uri(canonical)?;
        if self.opened.contains_key(canonical) {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":document.version},"contentChanges":[{"text":document.text}]}}))?;
        } else {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":document.language.id(canonical),"version":document.version,"text":document.text}}}))?;
        }
        self.opened.insert(canonical.clone(), document.version);
        Ok(())
    }
    fn close_transient_document(&mut self) -> Result<(), String> {
        if let Some(path) = &self.transient_document {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":protocol::file_uri(path)?}}}))?;
            self.transient_document = None;
        }
        Ok(())
    }
    fn cancel_rename(&mut self) {
        if let Some(pending) = self.rename.take() {
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":pending.wire_id()}}));
        }
        // Retry a full queue on the worker loop; never forget a live snapshot lease.
        let _ = self.close_transient_document();
    }
    fn close_reference_document(&mut self) -> Result<(), String> {
        if let Some(path) = &self.reference_transient {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":protocol::file_uri(path)?}}}))?;
            self.reference_transient = None;
        }
        Ok(())
    }
    fn cancel_references(&mut self) {
        if let Some(pending) = self.references.take() {
            let prefix = match pending.family.as_ref().map(|f| f.phase) {
                Some(ReferenceFamilyPhase::Definition) => "references-family-definition-",
                Some(ReferenceFamilyPhase::Base) => "references-family-base-",
                _ => {
                    if pending.definition_phase {
                        "references-definition-"
                    } else {
                        "references-"
                    }
                }
            };
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("{prefix}{}",pending.id)}}));
        }
        let _ = self.close_reference_document();
    }
    fn cancel_completion(&mut self) {
        if let Some(pending) = self.completion.take() {
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("completion-{}",pending.id)}}));
        }
    }
    fn close_super_document(&mut self) -> Result<(), String> {
        if let Some(path) = &self.super_transient {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":protocol::file_uri(path)?}}}))?;
            self.super_transient = None;
        }
        Ok(())
    }
    fn send_super_query(&mut self, pending: &mut PendingDefinition) -> Result<bool, String> {
        self.close_super_document()?;
        let Some(query) = pending.super_search.as_mut().and_then(|s| s.next()) else {
            return Ok(false);
        };
        let uri = protocol::file_uri(&query.path)?;
        if !self.opened.contains_key(&query.path) {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"php","version":1,"text":query.text}}}))?;
            self.super_transient = Some(query.path.clone());
        }
        pending.super_depth = query.depth();
        self.send(json!({"jsonrpc":"2.0","id":pending.wire_id(),"method":"textDocument/definition","params":{"textDocument":{"uri":uri},"position":{"line":query.line,"character":query.character}}}))?;
        Ok(true)
    }
    fn cancel_definition(&mut self) {
        if let Some(pending) = self.definition.take() {
            let _ = self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":pending.wire_id()}}));
        }
        let _ = self.close_super_document();
    }
    fn cancel_signature_help(&mut self) {
        if let Some(pending) = self.signature_help.take() {
            let _ = self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("signature-{}",pending.id)}}));
        }
    }
    fn cancel_formatting(&mut self) {
        if let Some(pending) = self.formatting.take() {
            let _=self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("formatting-{}",pending.id)}}));
        }
    }
    fn cancel_hover(&mut self) {
        if let Some(pending) = self.hover.take() {
            let _ = self.send(json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":format!("hover-{}",pending.id)}}));
        }
    }
    fn stop(&mut self) {
        if self.state == ProviderState::Stopped {
            return;
        }
        let _ = self.send(json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if let Ok(Ok(value)) = self.output.recv_timeout(Duration::from_millis(10))
                && value.get("id") == Some(&json!(2))
            {
                break;
            }
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
        }
        let _ = self.send(json!({"jsonrpc":"2.0","method":"exit"}));
        self.input.take();
        let deadline = Instant::now() + Duration::from_millis(500);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        // The process group also contains tsserver children; kill any survivors.
        #[cfg(unix)]
        unsafe {
            unsafe extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            kill(-(self.child.id() as i32), 9);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.alive.store(false, Ordering::Release);
        for task in self.tasks.drain(..) {
            let _ = task.join();
        }
        self.state = ProviderState::Stopped;
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn run(rx: Receiver<Request>, tx: SyncSender<Envelope>, stop: Arc<AtomicBool>) {
    let mut clients: Vec<Connection> = Vec::new();
    let mut documents: HashMap<PathBuf, Document> = HashMap::new();
    let mut workspace: Option<PathBuf> = None;
    let mut epoch = 0;
    let mut python_environment: Option<PathBuf> = None;
    while !stop.load(Ordering::Acquire) {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(request) => {
                if matches!(
                    request.command,
                    LanguageCommand::Start { .. } | LanguageCommand::Stop
                ) {
                    epoch = request.epoch;
                }
                let attempted: Vec<Language> =
                    if let LanguageCommand::Start { configs, .. } = &request.command {
                        configs.iter().map(|c| c.language).collect()
                    } else {
                        Vec::new()
                    };
                let preparing_rename =
                    matches!(&request.command, LanguageCommand::PrepareRename { .. });
                let renaming = matches!(
                    &request.command,
                    LanguageCommand::Rename { .. } | LanguageCommand::RenameClosed { .. }
                );
                let rename_name = if let LanguageCommand::Rename { new_name, .. }
                | LanguageCommand::RenameClosed { new_name, .. } =
                    &request.command
                {
                    Some(new_name.clone())
                } else {
                    None
                };
                let referencing = matches!(&request.command, LanguageCommand::References { .. });
                let formatting = matches!(&request.command, LanguageCommand::Formatting { .. });
                let code_actions = matches!(&request.command, LanguageCommand::CodeActions { .. });
                let resolving_action =
                    matches!(&request.command, LanguageCommand::ResolveCodeAction { .. });
                let inlay_hints = matches!(&request.command, LanguageCommand::InlayHints { .. });
                let signing = matches!(&request.command, LanguageCommand::SignatureHelp { .. });
                let completing = matches!(&request.command, LanguageCommand::Completion { .. });
                let hovering = matches!(&request.command, LanguageCommand::Hover { .. });
                let implementing =
                    matches!(&request.command, LanguageCommand::Implementation { .. });
                let super_defining =
                    matches!(&request.command, LanguageCommand::SuperDefinition { .. });
                let type_defining =
                    matches!(&request.command, LanguageCommand::TypeDefinition { .. });
                let defining = implementing
                    || super_defining
                    || type_defining
                    || matches!(&request.command, LanguageCommand::Definition { .. });
                for client in &mut clients {
                    if matches!(&request.command, LanguageCommand::PythonEnvironment { .. })
                        && client.language == Language::Python
                    {
                        client.cancel_rust_actions(true);
                        client.cancel_rust_hints();
                        client.cancel_formatting();
                        client.cancel_signature_help();
                        client.cancel_rename();
                        client.cancel_references();
                        client.cancel_completion();
                        client.cancel_definition();
                        client.hover = None;
                        client.rust_features.diagnostics.clear();
                    }
                    if let LanguageCommand::Sync { path, .. } | LanguageCommand::Close { path } =
                        &request.command
                        && let Ok(path) = path.canonicalize()
                    {
                        client.rust_features.diagnostics.remove(&path);
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::CodeActions { .. }
                            | LanguageCommand::CancelCodeActions
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_rust_actions(true);
                    } else if resolving_action {
                        client.cancel_rust_actions(false);
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::InlayHints { .. }
                            | LanguageCommand::CancelInlayHints
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_rust_hints();
                    }
                    if !matches!(&request.command, LanguageCommand::AnswerServerChoice { .. })
                        && client.server_choice.is_some()
                        && !(matches!(&request.command, LanguageCommand::PythonEnvironment { .. })
                            && client.language != Language::Python)
                    {
                        let _ = client.answer_choice(None, &tx, &stop, epoch);
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::Formatting { .. }
                            | LanguageCommand::CancelFormatting
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_formatting();
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::SignatureHelp { .. }
                            | LanguageCommand::CancelSignatureHelp
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_signature_help();
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::PrepareRename { .. }
                            | LanguageCommand::Rename { .. }
                            | LanguageCommand::RenameClosed { .. }
                            | LanguageCommand::CancelRename
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_rename();
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::References { .. }
                            | LanguageCommand::CancelReferences
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_references();
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::Completion { .. }
                            | LanguageCommand::CancelCompletion
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                    ) {
                        client.cancel_completion();
                    }
                    if matches!(
                        &request.command,
                        LanguageCommand::Definition { .. }
                            | LanguageCommand::Implementation { .. }
                            | LanguageCommand::SuperDefinition { .. }
                            | LanguageCommand::TypeDefinition { .. }
                            | LanguageCommand::CancelDefinition
                            | LanguageCommand::Start { .. }
                            | LanguageCommand::Stop
                            | LanguageCommand::Sync { .. }
                            | LanguageCommand::Close { .. }
                    ) {
                        client.cancel_definition();
                    }
                    let invalidates = match &request.command {
                        LanguageCommand::PythonEnvironment { .. } => false,
                        LanguageCommand::Hover { .. }
                        | LanguageCommand::CancelHover
                        | LanguageCommand::Start { .. }
                        | LanguageCommand::Stop => true,
                        LanguageCommand::CodeActions { .. }
                        | LanguageCommand::ResolveCodeAction { .. }
                        | LanguageCommand::CancelCodeActions
                        | LanguageCommand::InlayHints { .. }
                        | LanguageCommand::CancelInlayHints
                        | LanguageCommand::Formatting { .. }
                        | LanguageCommand::CancelFormatting
                        | LanguageCommand::Definition { .. }
                        | LanguageCommand::Implementation { .. }
                        | LanguageCommand::SuperDefinition { .. }
                        | LanguageCommand::TypeDefinition { .. }
                        | LanguageCommand::CancelDefinition
                        | LanguageCommand::Completion { .. }
                        | LanguageCommand::CancelCompletion
                        | LanguageCommand::References { .. }
                        | LanguageCommand::CancelReferences
                        | LanguageCommand::PrepareRename { .. }
                        | LanguageCommand::Rename { .. }
                        | LanguageCommand::RenameClosed { .. }
                        | LanguageCommand::CancelRename
                        | LanguageCommand::SignatureHelp { .. }
                        | LanguageCommand::CancelSignatureHelp
                        | LanguageCommand::AnswerServerChoice { .. } => false,
                        LanguageCommand::Sync { path, .. } | LanguageCommand::Close { path } => {
                            client
                                .hover
                                .as_ref()
                                .is_some_and(|h| &h.result.path == path)
                        }
                    };
                    if invalidates {
                        client.cancel_hover();
                    }
                }
                let syncing = match &request.command {
                    LanguageCommand::Sync { path, version, .. } => Some((path.clone(), *version)),
                    _ => None,
                };
                let result: Result<(), String> = (|| {
                    for client in &mut clients {
                        if client.definition.is_none() {
                            client.close_super_document()?;
                        }
                        if client.rename.is_none() {
                            client.close_transient_document()?;
                        }
                    }
                    match request.command {
                        LanguageCommand::AnswerServerChoice { token, index } => {
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.server_choice
                                        .as_ref()
                                        .is_some_and(|choice| choice.token == token)
                                })
                                .ok_or("Escolha do provider já encerrada")?;
                            client.answer_choice(index, &tx, &stop, epoch)?;
                        }
                        LanguageCommand::CancelCodeActions
                        | LanguageCommand::CancelInlayHints
                        | LanguageCommand::CancelFormatting
                        | LanguageCommand::CancelHover
                        | LanguageCommand::CancelDefinition
                        | LanguageCommand::CancelCompletion
                        | LanguageCommand::CancelReferences
                        | LanguageCommand::CancelRename
                        | LanguageCommand::CancelSignatureHelp => {}
                        LanguageCommand::CodeActions {
                            path,
                            version,
                            range,
                        }
                        | LanguageCommand::InlayHints {
                            path,
                            version,
                            range,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto")?;
                            client.request_rust_feature(
                                request.id,
                                path,
                                version,
                                range,
                                document,
                                inlay_hints,
                            )?;
                        }
                        LanguageCommand::ResolveCodeAction { token } => {
                            let root = workspace.as_deref().ok_or("Workspace indisponível")?;
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.has_code_action_token(token)
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Ação obsoleta ou provider indisponível")?;
                            if let Some(event) =
                                client.request_rust_resolve(request.id, token, &documents, root)?
                            {
                                emit(&tx, &stop, epoch, event);
                            }
                        }
                        LanguageCommand::PrepareRename {
                            path,
                            version,
                            line,
                            character,
                        }
                        | LanguageCommand::Rename {
                            path,
                            version,
                            line,
                            character,
                            ..
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado para rename")?;
                            if document.version != version {
                                return Err("Rename de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para rename".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para rename")?;
                            if !client.rename_supported {
                                return Err("Provider não oferece rename".into());
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            if preparing_rename && !client.prepare_rename_supported {
                                emit(
                                    &tx,
                                    &stop,
                                    epoch,
                                    Event::PrepareRename {
                                        id: request.id,
                                        result: Ok(PrepareRenameResult {
                                            path,
                                            version,
                                            line,
                                            character,
                                            range: None,
                                            placeholder: None,
                                            used_fallback: true,
                                        }),
                                    },
                                );
                            } else {
                                if let Some(name) = &rename_name {
                                    crate::rename::new_name(name)?;
                                }
                                let pending = PendingRename {
                                    id: request.id,
                                    path,
                                    canonical: canonical.clone(),
                                    version,
                                    line,
                                    character,
                                    new_name: rename_name.clone(),
                                    started: Instant::now(),
                                    closed_text: None,
                                };
                                let mut params = json!({"textDocument":{"uri":protocol::file_uri(&canonical)?},"position":{"line":line,"character":character}});
                                if let Some(name) = &pending.new_name {
                                    params["newName"] = json!(name);
                                }
                                client.send(json!({"jsonrpc":"2.0","id":pending.wire_id(),"method":if preparing_rename{"textDocument/prepareRename"}else{"textDocument/rename"},"params":params}))?;
                                client.rename = Some(pending);
                            }
                        }
                        LanguageCommand::RenameClosed {
                            path,
                            text,
                            line,
                            character,
                            new_name,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let root = workspace
                                .as_ref()
                                .ok_or("Workspace de rename indisponível")?;
                            if canonical != path
                                || !path.starts_with(root)
                                || documents.contains_key(&path)
                            {
                                return Err("Snapshot de rename não é caminho fechado canônico do workspace".into());
                            }
                            if text.len() > 1024 * 1024
                                || crate::utf16_byte_offset(&text, line, character).is_none()
                            {
                                return Err("Snapshot/posição de rename inválidos".into());
                            }
                            let mut actual = String::new();
                            std::fs::File::open(&path)
                                .map_err(|e| e.to_string())?
                                .take(1024 * 1024 + 1)
                                .read_to_string(&mut actual)
                                .map_err(|e| e.to_string())?;
                            if actual != text
                                || path.canonicalize().map_err(|e| e.to_string())? != path
                            {
                                return Err(
                                    "Arquivo fechado mudou antes da consulta relacionada".into()
                                );
                            }
                            let language = Language::for_path(&path)
                                .ok_or("Linguagem de rename indisponível")?;
                            let client = clients
                                .iter_mut()
                                .find(|c| c.language == language && c.state == ProviderState::Ready)
                                .ok_or("Provider não está pronto para rename")?;
                            if !client.rename_supported {
                                return Err("Provider não oferece rename".into());
                            }
                            client.close_transient_document()?;
                            let uri = protocol::file_uri(&path)?;
                            // This lease belongs to this request only. It is deliberately absent
                            // from documents/opened, so diagnostics and Sync cannot adopt it.
                            client.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":language.id(&path),"version":1,"text":text}}}))?;
                            client.transient_document = Some(path.clone());
                            let pending = PendingRename {
                                id: request.id,
                                path: path.clone(),
                                canonical,
                                version: 0,
                                line,
                                character,
                                new_name: Some(new_name.clone()),
                                started: Instant::now(),
                                closed_text: Some(text),
                            };
                            if let Err(error) = client.send(json!({"jsonrpc":"2.0","id":pending.wire_id(),"method":"textDocument/rename","params":{"textDocument":{"uri":uri},"position":{"line":line,"character":character},"newName":new_name}})) {
                                let _ = client.close_transient_document();
                                return Err(error);
                            }
                            client.rename = Some(pending);
                        }
                        LanguageCommand::References {
                            path,
                            version,
                            line,
                            character,
                            include_declaration,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            if document.version != version {
                                return Err("Referências de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para referências".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para referências")?;
                            if !client.references_supported {
                                return Err("Provider não oferece referências".into());
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            let definition_phase =
                                client.typescript_reference_adapter && !include_declaration;
                            if definition_phase && !client.definition_supported {
                                return Err("O adaptador TypeScript exige definitionProvider para excluir declarações com segurança".into());
                            }
                            let family = if client.phpactor_completion
                                && client.definition_supported
                                && !include_declaration
                            {
                                crate::php_references::method_at(&document.text, line, character)
                                    .map(|method| ReferenceFamily {
                                        method,
                                        phase: ReferenceFamilyPhase::Original,
                                    })
                            } else {
                                None
                            };
                            let source_uri = protocol::file_uri(&canonical)?;
                            client.send(if definition_phase {
                                json!({"jsonrpc":"2.0","id":format!("references-definition-{}",request.id),"method":"textDocument/definition","params":{"textDocument":{"uri":source_uri},"position":{"line":line,"character":character}}})
                            } else {
                                json!({"jsonrpc":"2.0","id":format!("references-{}",request.id),"method":"textDocument/references","params":{"textDocument":{"uri":source_uri},"position":{"line":line,"character":character},"context":{"includeDeclaration":include_declaration}}})
                            })?;
                            client.references = Some(PendingReferences {
                                id: request.id,
                                definition_phase,
                                declarations: vec![],
                                source_uri,
                                family,
                                result: ReferencesResult {
                                    path,
                                    version,
                                    line,
                                    character,
                                    targets: vec![],
                                    truncated: false,
                                },
                                started: Instant::now(),
                            });
                        }
                        LanguageCommand::SignatureHelp {
                            path,
                            version,
                            line,
                            character,
                            trigger,
                            is_retrigger,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            if document.version != version {
                                return Err("Assinatura de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para assinatura".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para assinatura")?;
                            let caps = client
                                .signature_help_capabilities
                                .as_ref()
                                .ok_or("Provider não oferece assinatura de chamadas")?;
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            let mut context = json!({"triggerKind": match &trigger { SignatureHelpTrigger::Invoked => 1, SignatureHelpTrigger::Character(_) => 2, SignatureHelpTrigger::ContentChange => 3 }, "isRetrigger":is_retrigger});
                            if let SignatureHelpTrigger::Character(ch) = trigger {
                                if !caps.trigger_characters.contains(&ch)
                                    && !(is_retrigger && caps.retrigger_characters.contains(&ch))
                                {
                                    return Err(
                                        "Gatilho de assinatura não anunciado pelo provider".into(),
                                    );
                                }
                                context["triggerCharacter"] = json!(ch);
                            }
                            client.send(json!({"jsonrpc":"2.0","id":format!("signature-{}",request.id),"method":"textDocument/signatureHelp","params":{"textDocument":{"uri":protocol::file_uri(&canonical)?},"position":{"line":line,"character":character},"context":context}}))?;
                            client.signature_help = Some(PendingSignature {
                                id: request.id,
                                result: SignatureHelpResult {
                                    path,
                                    version,
                                    line,
                                    character,
                                    signatures: vec![],
                                    active_signature: None,
                                    active_parameter: None,
                                    truncated: false,
                                },
                                started: Instant::now(),
                            });
                        }
                        LanguageCommand::Completion {
                            path,
                            version,
                            line,
                            character,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            if document.version != version {
                                return Err("Completion de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para completion".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para completion")?;
                            if !client.completion_supported {
                                return Err("Provider não oferece completion".into());
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            client.send(json!({"jsonrpc":"2.0","id":format!("completion-{}",request.id),"method":"textDocument/completion","params":{"textDocument":{"uri":protocol::file_uri(&canonical)?},"position":{"line":line,"character":character},"context":{"triggerKind":1}}}))?;
                            client.completion = Some(PendingCompletion {
                                id: request.id,
                                result: CompletionResult {
                                    path,
                                    version,
                                    line,
                                    character,
                                    items: vec![],
                                    is_incomplete: false,
                                    truncated: false,
                                },
                                canonical,
                                started: Instant::now(),
                            });
                        }
                        LanguageCommand::Definition {
                            path,
                            version,
                            line,
                            character,
                        }
                        | LanguageCommand::Implementation {
                            path,
                            version,
                            line,
                            character,
                        }
                        | LanguageCommand::SuperDefinition {
                            path,
                            version,
                            line,
                            character,
                        }
                        | LanguageCommand::TypeDefinition {
                            path,
                            version,
                            line,
                            character,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            if document.version != version {
                                return Err("Definição de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para definição".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para definição")?;
                            if super_defining && document.language != Language::Php {
                                return Err(
                                    "Navegação aos ascendentes implementada para PHP".into()
                                );
                            }
                            if type_defining {
                                if !client.type_definition_supported {
                                    return Err("Provider não oferece declaração de tipo".into());
                                }
                            } else if implementing {
                                if !client.implementation_supported {
                                    return Err(
                                        "Provider não oferece navegação para implementações".into(),
                                    );
                                }
                            } else if !client.definition_supported {
                                return Err("Provider não oferece definição".into());
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            let method = if type_defining {
                                "textDocument/typeDefinition"
                            } else if implementing {
                                "textDocument/implementation"
                            } else {
                                "textDocument/definition"
                            };
                            let mut pending = PendingDefinition {
                                id: request.id,
                                implementation: implementing,
                                type_definition: type_defining,
                                super_search: if super_defining {
                                    Some(crate::php_super::Search::start(
                                        &canonical,
                                        &document.text,
                                        line,
                                        character,
                                    )?)
                                } else {
                                    None
                                },
                                super_depth: 0,
                                result: DefinitionResult {
                                    presentation: None,
                                    path,
                                    version,
                                    line,
                                    character,
                                    targets: vec![],
                                },
                                started: Instant::now(),
                            };
                            if super_defining {
                                match client.send_super_query(&mut pending) {
                                    Ok(true) => client.definition = Some(pending),
                                    Ok(false) => emit(
                                        &tx,
                                        &stop,
                                        epoch,
                                        Event::Definition {
                                            id: pending.id,
                                            result: Ok(pending.result),
                                        },
                                    ),
                                    Err(error) => {
                                        let _ = client.close_super_document();
                                        return Err(error);
                                    }
                                }
                            } else {
                                client.send(json!({"jsonrpc":"2.0","id":pending.wire_id(),"method":method,"params":{"textDocument":{"uri":protocol::file_uri(&canonical)?},"position":{"line":line,"character":character}}}))?;
                                client.definition = Some(pending);
                            }
                        }
                        LanguageCommand::Formatting {
                            path,
                            version,
                            range,
                            tab_size,
                            insert_spaces,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado para formatação")?;
                            if document.version != version {
                                return Err("Formatação de versão obsoleta".into());
                            }
                            if let Some(r) = &range {
                                crate::rename::validate_range(&document.text, r)?;
                                if (r.start_line, r.start_character) > (r.end_line, r.end_character)
                                {
                                    return Err("Seleção de formatação invertida".into());
                                }
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para formatação")?;
                            if !(if range.is_some() {
                                client.formatting_capabilities.range
                            } else {
                                client.formatting_capabilities.document
                            }) {
                                return Err(
                                    "Provider não oferece esta modalidade de formatação".into()
                                );
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            let mut params = json!({"textDocument":{"uri":protocol::file_uri(&canonical)?},"options":{"tabSize":tab_size,"insertSpaces":insert_spaces}});
                            if let Some(r) = &range {
                                params["range"] = json!({"start":{"line":r.start_line,"character":r.start_character},"end":{"line":r.end_line,"character":r.end_character}});
                            }
                            client.send(json!({"jsonrpc":"2.0","id":format!("formatting-{}",request.id),"method":if range.is_some(){"textDocument/rangeFormatting"}else{"textDocument/formatting"},"params":params}))?;
                            client.formatting = Some(PendingFormatting {
                                id: request.id,
                                canonical,
                                result: FormattingResult {
                                    path,
                                    version,
                                    range,
                                    edits: vec![],
                                },
                                started: Instant::now(),
                            });
                        }
                        LanguageCommand::Hover {
                            path,
                            version,
                            line,
                            character,
                        } => {
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let document = documents
                                .get(&canonical)
                                .ok_or("Documento não sincronizado")?;
                            if document.version != version {
                                return Err("Hover de versão obsoleta".into());
                            }
                            if crate::utf16_byte_offset(&document.text, line, character).is_none() {
                                return Err("Posição UTF-16 inválida para hover".into());
                            }
                            let client = clients
                                .iter_mut()
                                .find(|c| {
                                    c.language == document.language
                                        && c.state == ProviderState::Ready
                                })
                                .ok_or("Provider não está pronto para hover")?;
                            if !client.hover_supported {
                                return Err("Provider não oferece hover".into());
                            }
                            if client.opened.get(&canonical) != Some(&version) {
                                return Err("Versão ainda não entregue ao provider".into());
                            }
                            client.send(json!({"jsonrpc":"2.0","id":format!("hover-{}",request.id),"method":"textDocument/hover","params":{"textDocument":{"uri":protocol::file_uri(&canonical)?},"position":{"line":line,"character":character}}}))?;
                            client.hover = Some(PendingHover {
                                id: request.id,
                                result: HoverResult {
                                    path,
                                    version,
                                    line,
                                    character,
                                    text: String::new(),
                                },
                                started: Instant::now(),
                            });
                        }
                        LanguageCommand::PythonEnvironment { python } => {
                            python_environment = python;
                            for client in clients
                                .iter_mut()
                                .filter(|c| c.language == Language::Python)
                            {
                                client.python_environment = python_environment
                                    .clone()
                                    .or_else(|| codisseu_runtime::python().ok());
                                if client.state == ProviderState::Ready {
                                    client.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeConfiguration","params":{"settings":crate::python_settings::settings(client.python_environment.as_deref())}}))?;
                                }
                            }
                        }
                        LanguageCommand::Start {
                            workspace: path,
                            configs,
                        } => {
                            for client in &mut clients {
                                client.stop();
                            }
                            clients.clear();
                            documents.clear();
                            workspace = None;
                            let root = path.canonicalize().map_err(|e| e.to_string())?;
                            if !root.is_dir() {
                                return Err("Workspace LSP deve ser uma pasta".into());
                            }
                            for config in configs {
                                let language = config.language;
                                match Connection::start(
                                    config,
                                    &root,
                                    python_environment.as_deref(),
                                ) {
                                    Ok(client) => clients.push(client),
                                    Err(error) => {
                                        emit(
                                            &tx,
                                            &stop,
                                            epoch,
                                            Event::Status(ProviderStatus {
                                                language,
                                                state: ProviderState::Failed,
                                                detail: error.clone(),
                                            }),
                                        );
                                        emit(&tx, &stop, epoch, Event::Error(error));
                                    }
                                }
                            }
                            workspace = Some(root);
                        }
                        LanguageCommand::Stop => {
                            for client in &mut clients {
                                client.stop();
                                emit(
                                    &tx,
                                    &stop,
                                    epoch,
                                    Event::Status(ProviderStatus {
                                        language: client.language,
                                        state: ProviderState::Stopped,
                                        detail: "Processo encerrado e recolhido".into(),
                                    }),
                                );
                            }
                            clients.clear();
                            documents.clear();
                            workspace = None;
                        }
                        LanguageCommand::Sync {
                            path,
                            version,
                            text,
                        } => {
                            let Some(language) = Language::for_path(&path) else {
                                return Ok(());
                            };
                            let canonical = path.canonicalize().map_err(|e| e.to_string())?;
                            let root = workspace
                                .as_ref()
                                .ok_or("Inicie os providers explicitamente antes de sincronizar")?;
                            if !canonical.starts_with(root) {
                                return Err("Documento fora do workspace LSP".into());
                            }
                            if !clients.iter().any(|client| {
                                client.language == language
                                    && matches!(
                                        client.state,
                                        ProviderState::Starting | ProviderState::Ready
                                    )
                            }) {
                                return Err(
                                    "Provider da linguagem indisponível; inicie-o novamente".into(),
                                );
                            }
                            if documents
                                .get(&canonical)
                                .is_some_and(|d| d.version > version)
                            {
                                return Err("Versão LSP obsoleta".into());
                            }
                            let document = Document {
                                path,
                                version,
                                text,
                                language,
                            };
                            for client in &mut clients {
                                client.sync(&canonical, &document)?;
                            }
                            documents.insert(canonical, document);
                        }
                        LanguageCommand::Close { path } => {
                            let canonical = path.canonicalize().unwrap_or(path);
                            documents.remove(&canonical);
                            for client in &mut clients {
                                if client.opened.remove(&canonical).is_some() {
                                    client.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":protocol::file_uri(&canonical)?}}}))?;
                                }
                            }
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    if preparing_rename {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::PrepareRename {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if renaming {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Rename {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if code_actions {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::CodeActions {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if resolving_action {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::CodeActionEdit {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if inlay_hints {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::InlayHints {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if formatting {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Formatting {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if signing {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::SignatureHelp {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if hovering {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Hover {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if referencing {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::References {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if completing {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Completion {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if defining {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Definition {
                                id: request.id,
                                result: Err(error.clone()),
                            },
                        );
                    }
                    if let Some((path, version)) = syncing {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::SyncFailed {
                                path,
                                failure: SyncFailure {
                                    version,
                                    message: error.clone(),
                                },
                            },
                        );
                    }
                    for language in attempted {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Status(ProviderStatus {
                                language,
                                state: ProviderState::Failed,
                                detail: error.clone(),
                            }),
                        );
                    }
                    if !formatting
                        && !code_actions
                        && !resolving_action
                        && !inlay_hints
                        && !signing
                        && !hovering
                        && !defining
                        && !completing
                        && !referencing
                        && !preparing_rename
                        && !renaming
                    {
                        emit(&tx, &stop, epoch, Event::Error(error));
                    }
                }
                emit(&tx, &stop, request.epoch, Event::Ack(request.id));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        for client in &mut clients {
            if client.state == ProviderState::Failed {
                continue;
            }
            let mut failure = None;
            if client.rename.is_none() {
                let _ = client.close_transient_document();
            }
            for _ in 0..32 {
                let Ok(message) = client.output.try_recv() else {
                    break;
                };
                let value = match message {
                    Ok(value) => value,
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                };
                if let Some(event) =
                    client.rust_feature_response(&value, &documents, workspace.as_deref())
                {
                    emit(&tx, &stop, epoch, event);
                    continue;
                }
                if value.get("method").is_none()
                    && client
                        .rename
                        .as_ref()
                        .is_some_and(|r| value.get("id") == Some(&json!(r.wire_id())))
                {
                    let pending = client.rename.take().expect("matched rename");
                    if let Err(error) = client.close_transient_document() {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            pending.error(format!(
                                "Fechamento do snapshot relacionado falhou: {error}"
                            )),
                        );
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            pending.error(format!(
                                "Rename recusado: {}",
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Erro do provider")
                                    .chars()
                                    .take(2048)
                                    .collect::<String>()
                            )),
                        );
                    } else if let Some(new_name) = &pending.new_name {
                        let result = workspace
                            .as_ref()
                            .ok_or_else(|| "Workspace de rename indisponível".to_string())
                            .and_then(|root| {
                                crate::rename::workspace_edit(value.get("result"), root)
                            })
                            .and_then(|changes| {
                                for file in &changes {
                                    if file.path == pending.canonical
                                        && let Some(text) = &pending.closed_text
                                    {
                                        if file.version.is_some_and(|v| v != 1) {
                                            return Err(
                                                "Versão do snapshot temporário divergiu".into()
                                            );
                                        }
                                        for edit in &file.edits {
                                            crate::rename::validate_range(
                                                text,
                                                &RenameRange {
                                                    start_line: edit.start_line,
                                                    start_character: edit.start_character,
                                                    end_line: edit.end_line,
                                                    end_character: edit.end_character,
                                                },
                                            )?;
                                        }
                                    }
                                    if let Some(document) = documents.get(&file.path) {
                                        if file.version.is_some_and(|v| v != document.version) {
                                            return Err(
                                                "Edições de rename usam versão obsoleta do destino"
                                                    .into(),
                                            );
                                        }
                                        for edit in &file.edits {
                                            crate::rename::validate_range(
                                                &document.text,
                                                &RenameRange {
                                                    start_line: edit.start_line,
                                                    start_character: edit.start_character,
                                                    end_line: edit.end_line,
                                                    end_character: edit.end_character,
                                                },
                                            )?;
                                        }
                                    }
                                }
                                Ok(RenameResult {
                                    transient_document_version: pending
                                        .closed_text
                                        .as_ref()
                                        .map(|_| 1),
                                    path: pending.path.clone(),
                                    version: pending.version,
                                    line: pending.line,
                                    character: pending.character,
                                    new_name: new_name.clone(),
                                    changes,
                                })
                            });
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Rename {
                                id: pending.id,
                                result,
                            },
                        );
                    } else {
                        let result = documents
                            .get(&pending.canonical)
                            .filter(|doc| doc.version == pending.version)
                            .ok_or_else(|| "Snapshot de preparação indisponível".to_string())
                            .and_then(|doc| {
                                crate::rename::prepare(
                                    value.get("result"),
                                    &doc.text,
                                    pending.line,
                                    pending.character,
                                )
                            })
                            .map(|(range, placeholder)| PrepareRenameResult {
                                path: pending.path.clone(),
                                version: pending.version,
                                line: pending.line,
                                character: pending.character,
                                range,
                                placeholder,
                                used_fallback: false,
                            });
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::PrepareRename {
                                id: pending.id,
                                result,
                            },
                        );
                    }
                } else if value.get("method").is_none()
                    && client.references.as_ref().is_some_and(|r| {
                        r.family
                            .as_ref()
                            .is_some_and(|f| f.phase == ReferenceFamilyPhase::Definition)
                            && value.get("id")
                                == Some(&json!(format!("references-family-definition-{}", r.id)))
                    })
                {
                    let mut pending = client
                        .references
                        .take()
                        .expect("matched PHP references base");
                    let attempt = (|| -> Result<Option<(PathBuf, String, u32, u32)>, String> {
                        if pending.started.elapsed() > Duration::from_secs(10) {
                            return Err("Timeout de referências".into());
                        }
                        if value.get("error").is_some() {
                            return Err("Provider não resolveu base de referências".into());
                        }
                        let root = workspace.as_ref().ok_or("Workspace indisponível")?;
                        let targets = crate::definition::targets(value.get("result"), root)?;
                        if targets.is_empty() {
                            return Ok(None);
                        }
                        if targets.len() != 1 {
                            return Err("Base de referências ambígua".into());
                        }
                        let target = &targets[0];
                        if target.path == pending.result.path
                            && (target.start_line, target.start_character)
                                <= (pending.result.line, pending.result.character)
                            && (pending.result.line, pending.result.character)
                                <= (target.end_line, target.end_character)
                        {
                            return Ok(None);
                        }
                        let text = if let Some(document) = documents.get(&target.path) {
                            document.text.clone()
                        } else {
                            crate::php_references::read_base(&target.path, root)
                                .ok_or("Base de referências não certificada")?
                        };
                        let method = &pending.family.as_ref().unwrap().method;
                        let (line, character) =
                            crate::php_references::target_method(&text, target, method)
                                .ok_or("Método da base não certificado")?;
                        Ok(Some((target.path.clone(), text, line, character)))
                    })();
                    match attempt {
                        Ok(Some((path, text, line, character))) => {
                            let uri = protocol::file_uri(&path);
                            let sent=uri.and_then(|uri|{
                                client.close_reference_document()?;
                                if !client.opened.contains_key(&path){
                                    client.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"php","version":1,"text":text}}}))?;
                                    client.reference_transient=Some(path);
                                }
                                client.send(json!({"jsonrpc":"2.0","id":format!("references-family-base-{}",pending.id),"method":"textDocument/references","params":{"textDocument":{"uri":uri},"position":{"line":line,"character":character},"context":{"includeDeclaration":false}}}))
                            });
                            if sent.is_ok() {
                                pending.family.as_mut().unwrap().phase = ReferenceFamilyPhase::Base;
                                client.references = Some(pending);
                            } else {
                                pending.result.truncated = true;
                                let _ = client.close_reference_document();
                                emit(
                                    &tx,
                                    &stop,
                                    epoch,
                                    Event::References {
                                        id: pending.id,
                                        result: Ok(pending.result),
                                    },
                                );
                            }
                        }
                        other => {
                            if other.is_err() {
                                pending.result.truncated = true;
                            }
                            emit(
                                &tx,
                                &stop,
                                epoch,
                                Event::References {
                                    id: pending.id,
                                    result: Ok(pending.result),
                                },
                            );
                        }
                    }
                } else if value.get("method").is_none()
                    && client.references.as_ref().is_some_and(|r| {
                        r.definition_phase
                            && value.get("id")
                                == Some(&json!(format!("references-definition-{}", r.id)))
                    })
                {
                    let mut pending = client
                        .references
                        .take()
                        .expect("matched private definition");
                    let result = if pending.started.elapsed() > Duration::from_secs(10) {
                        Err("Timeout de referências após 10 segundos".into())
                    } else if let Some(error) = value.get("error") {
                        Err(format!(
                            "Não foi possível excluir declarações TypeScript: {}",
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("falha de definição")
                                .chars()
                                .take(2048)
                                .collect::<String>()
                        ))
                    } else {
                        workspace.as_ref().ok_or_else(||"Workspace indisponível".to_string()).and_then(|root|crate::definition::targets(value.get("result"),root)).and_then(|targets| {
                            if targets.is_empty() {Err("TypeScript não retornou definição para excluir declarações com segurança".into())} else {Ok(targets)}
                        })
                    };
                    match result {
                        Ok(targets) => {
                            pending.declarations = targets;
                            pending.definition_phase = false;
                            match client.send(json!({"jsonrpc":"2.0","id":format!("references-{}",pending.id),"method":"textDocument/references","params":{"textDocument":{"uri":pending.source_uri},"position":{"line":pending.result.line,"character":pending.result.character},"context":{"includeDeclaration":false}}})) {
                                Ok(()) => client.references=Some(pending),
                                Err(error) => emit(&tx,&stop,epoch,Event::References{id:pending.id,result:Err(error)}),
                            }
                        }
                        Err(error) => emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::References {
                                id: pending.id,
                                result: Err(error),
                            },
                        ),
                    }
                } else if value.get("method").is_none()
                    && client.references.as_ref().is_some_and(|r| {
                        !r.definition_phase
                            && value.get("id")
                                == Some(&json!(format!(
                                    "{}{}",
                                    if r.family
                                        .as_ref()
                                        .is_some_and(|f| f.phase == ReferenceFamilyPhase::Base)
                                    {
                                        "references-family-base-"
                                    } else {
                                        "references-"
                                    },
                                    r.id
                                )))
                    })
                {
                    let mut pending = client.references.take().expect("matched references");
                    let parsed = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Referências falharam: {}",
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("falha do provider")
                                .chars()
                                .take(2048)
                                .collect::<String>()
                        ))
                    } else {
                        workspace
                            .as_ref()
                            .ok_or_else(|| "Workspace de referências indisponível".to_string())
                            .and_then(|root| {
                                crate::definition::references_for_language(
                                    value.get("result"),
                                    root,
                                    client.language,
                                    client.python_environment.as_deref(),
                                )
                            })
                    };
                    let is_base = pending
                        .family
                        .as_ref()
                        .is_some_and(|f| f.phase == ReferenceFamilyPhase::Base);
                    let mut failure = None;
                    match parsed {
                        Ok((targets, truncated)) => {
                            pending.result.truncated |= truncated;
                            for target in targets {
                                if !pending.declarations.contains(&target)
                                    && !pending.result.targets.contains(&target)
                                {
                                    if pending.result.targets.len() < 256 {
                                        pending.result.targets.push(target);
                                    } else {
                                        pending.result.truncated = true;
                                    }
                                }
                            }
                        }
                        Err(_error) if is_base => pending.result.truncated = true,
                        Err(error) => failure = Some(error),
                    }
                    if failure.is_none()
                        && pending
                            .family
                            .as_ref()
                            .is_some_and(|f| f.phase == ReferenceFamilyPhase::Original)
                    {
                        pending.family.as_mut().unwrap().phase = ReferenceFamilyPhase::Definition;
                        match client.send(json!({"jsonrpc":"2.0","id":format!("references-family-definition-{}",pending.id),"method":"textDocument/definition","params":{"textDocument":{"uri":pending.source_uri},"position":{"line":pending.result.line,"character":pending.result.character}}})) {
                            Ok(())=>client.references=Some(pending),
                            Err(_)=>{pending.result.truncated=true;emit(&tx,&stop,epoch,Event::References{id:pending.id,result:Ok(pending.result)});}
                        }
                    } else {
                        let _ = client.close_reference_document();
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::References {
                                id: pending.id,
                                result: failure.map_or_else(|| Ok(pending.result), Err),
                            },
                        );
                    }
                } else if value.get("method").is_none()
                    && client.signature_help.as_ref().is_some_and(|h| {
                        value.get("id") == Some(&json!(format!("signature-{}", h.id)))
                    })
                {
                    let pending = client.signature_help.take().expect("matched signature");
                    let result = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Assinatura falhou: {}",
                            error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("erro LSP")
                                .chars()
                                .take(2048)
                                .collect::<String>()
                        ))
                    } else {
                        crate::signature::decode(value.get("result"), pending.result)
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::SignatureHelp {
                            id: pending.id,
                            result,
                        },
                    );
                } else if value.get("method").is_none()
                    && client.completion.as_ref().is_some_and(|c| {
                        value.get("id") == Some(&json!(format!("completion-{}", c.id)))
                    })
                {
                    let pending = client.completion.take().expect("matched completion");
                    let result = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Completion falhou: {}",
                            error.to_string().chars().take(2048).collect::<String>()
                        ))
                    } else {
                        documents
                            .get(&pending.canonical)
                            .filter(|d| d.version == pending.result.version)
                            .ok_or_else(|| "Snapshot completion obsoleto".to_string())
                            .and_then(|document| {
                                let normalized = client
                                    .phpactor_completion
                                    .then(|| {
                                        crate::completion::normalize_phpactor(value.get("result"))
                                    })
                                    .flatten();
                                let mut result = crate::completion::decode(
                                    normalized.as_ref().or_else(|| value.get("result")),
                                    pending.result.path,
                                    pending.result.version,
                                    (pending.result.line, pending.result.character),
                                    &document.text,
                                )?;
                                if document.language == Language::Php
                                    && let Some(root) = workspace.as_ref()
                                    && let Some(resources) = crate::laravel_assist::query(
                                        root,
                                        &result.path,
                                        &document.text,
                                        result.line,
                                        result.character,
                                        &stop,
                                        &documents
                                            .iter()
                                            .map(|(p, d)| (p.clone(), d.text.as_str()))
                                            .collect::<Vec<_>>(),
                                        true,
                                    )
                                {
                                    if resources.supplemental {
                                        crate::laravel_assist::merge_completion(
                                            &mut result.items,
                                            resources.result.items,
                                            &mut result.truncated,
                                        );
                                        result.truncated |= resources.result.truncated;
                                    } else {
                                        result.items = resources.result.items;
                                        result.truncated = resources.result.truncated;
                                    }
                                } else if result.items.is_empty()
                                    && document.language == Language::Php
                                    && let Some(root) = workspace.as_ref()
                                {
                                    let buffers = documents
                                        .iter()
                                        .map(|(p, d)| (p.clone(), d.text.as_str()))
                                        .collect::<Vec<_>>();
                                    if let Some((items, truncated)) =
                                        crate::php_attributes::completions(
                                            root,
                                            &result.path,
                                            &document.text,
                                            result.line,
                                            result.character,
                                            &stop,
                                            &buffers,
                                        )
                                    {
                                        result.items = items;
                                        result.truncated = truncated;
                                    } else {
                                        result.items = crate::laravel::completions(
                                            root,
                                            &document.text,
                                            result.line,
                                            result.character,
                                            &stop,
                                            &documents
                                                .iter()
                                                .map(|(path, doc)| {
                                                    (path.clone(), doc.text.as_str())
                                                })
                                                .collect::<Vec<_>>(),
                                        );
                                    }
                                }
                                Ok(result)
                            })
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Completion {
                            id: pending.id,
                            result,
                        },
                    );
                } else if value.get("method").is_none()
                    && client
                        .definition
                        .as_ref()
                        .is_some_and(|d| value.get("id") == Some(&json!(d.wire_id())))
                {
                    let mut pending = client.definition.take().expect("matched definition");
                    if pending.super_search.is_some() {
                        let attempt = (|| -> Result<bool, String> {
                            client.close_super_document()?;
                            if value.get("error").is_some() {
                                return Err("Provider não resolveu supertipo".into());
                            }
                            let root = workspace.as_ref().ok_or("Workspace indisponível")?;
                            let targets = crate::definition::targets(value.get("result"), root)?;
                            if targets.is_empty() {
                                return Err("Supertipo não resolvido; navegação incompleta".into());
                            }
                            for target in targets {
                                if stop.load(Ordering::Acquire)
                                    || pending.started.elapsed() > Duration::from_secs(10)
                                {
                                    return Err(
                                        "Navegação de ascendentes cancelada ou excedeu10segundos"
                                            .into(),
                                    );
                                }
                                let text = if let Some(d) = documents.get(&target.path) {
                                    d.text.clone()
                                } else {
                                    crate::php_references::read_base(&target.path, root)
                                        .ok_or("Fonte do supertipo não pôde ser validada")?
                                };
                                pending.super_search.as_mut().unwrap().accept(
                                    &target,
                                    &text,
                                    pending.super_depth,
                                )?;
                            }
                            client.send_super_query(&mut pending)
                        })();
                        match attempt {
                            Ok(true) => client.definition = Some(pending),
                            other => {
                                let _ = client.close_super_document();
                                let result = match other {
                                    Ok(false) => {
                                        pending.result.targets =
                                            pending.super_search.take().unwrap().targets;
                                        if let Some(root) = workspace.as_ref()
                                            && let Ok(canonical) =
                                                pending.result.path.canonicalize()
                                            && let Some(document) = documents.get(&canonical)
                                        {
                                            pending.result.presentation =
                                                crate::navigation_presentation::describe(
                                                    root,
                                                    &document.text,
                                                    &pending.result,
                                                    &documents
                                                        .iter()
                                                        .map(|(p, d)| (p.clone(), d.text.as_str()))
                                                        .collect::<Vec<_>>(),
                                                    &stop,
                                                );
                                        }
                                        crate::navigation_presentation::sort(&mut pending.result);
                                        Ok(pending.result)
                                    }
                                    Err(error) => Err(error),
                                    Ok(true) => unreachable!(),
                                };
                                emit(
                                    &tx,
                                    &stop,
                                    epoch,
                                    Event::Definition {
                                        id: pending.id,
                                        result,
                                    },
                                );
                            }
                        }
                        continue;
                    }

                    let result = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Definição falhou: {}",
                            error.to_string().chars().take(2048).collect::<String>()
                        ))
                    } else {
                        workspace
                            .as_ref()
                            .ok_or_else(|| "Workspace de definição indisponível".to_string())
                            .and_then(|root| crate::definition::targets_for_language(value.get("result"), root, client.language, client.python_environment.as_deref()))
                            .and_then(|mut targets| {
                                if !pending.implementation && !pending.type_definition && client.language == Language::Php
                                    && let Some(root) = workspace.as_ref()
                                    && let Ok(canonical) = pending.result.path.canonicalize()
                                    && let Some(document) = documents.get(&canonical)
                                    && document.version == pending.result.version
                                {
                                    let buffers = documents
                                        .iter()
                                        .map(|(p, d)| (p.clone(), d.text.as_str()))
                                        .collect::<Vec<_>>();
                                    if let Some(resources) = crate::laravel_assist::query(
                                        root,
                                        &pending.result.path,
                                        &document.text,
                                        pending.result.line,
                                        pending.result.character,
                                        &stop,
                                        &buffers,
                                        false,
                                    ) {
                                        if resources.result.truncated {
                                            return Err("Definição Laravel excedeu o orçamento de leitura ou de destinos; resultado parcial não será aberto".into());
                                        }
                                        if !resources.supplemental || !resources.result.targets.is_empty() {
                                            targets = resources.result.targets;
                                        }
                                    } else if targets.is_empty() {
                                        targets = crate::laravel::definitions(
                                            root,
                                            &document.text,
                                            pending.result.line,
                                            pending.result.character,
                                            &stop,
                                            &documents
                                                .iter()
                                                .map(|(path, doc)| {
                                                    (path.clone(), doc.text.as_str())
                                                })
                                                .collect::<Vec<_>>(),
                                        );
                                    }
                                }
                                pending.result.targets = targets;
                                if client.language == Language::Php && pending.result.targets.len() > 1
                                    && let Ok(canonical) = pending.result.path.canonicalize()
                                    && let Some(document) = documents.get(&canonical)
                                {
                                    pending.result.presentation = crate::navigation_presentation::describe(
                                        workspace.as_deref().unwrap(), &document.text, &pending.result,
                                        &documents.iter().map(|(p,d)|(p.clone(),d.text.as_str())).collect::<Vec<_>>(), &stop);
                                }
                                crate::navigation_presentation::sort(&mut pending.result);
                                Ok(pending.result)
                            })
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Definition {
                            id: pending.id,
                            result,
                        },
                    );
                } else if value.get("method").is_none()
                    && client
                        .hover
                        .as_ref()
                        .is_some_and(|h| value.get("id") == Some(&json!(format!("hover-{}", h.id))))
                {
                    let mut pending = client.hover.take().expect("matched pending hover");
                    let result = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Hover falhou: {}",
                            error.to_string().chars().take(2048).collect::<String>()
                        ))
                    } else {
                        hover_text(value.get("result")).map(|text| {
                            pending.result.text = text;
                            pending.result
                        })
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Hover {
                            id: pending.id,
                            result,
                        },
                    );
                } else if value.get("method").is_none()
                    && client.formatting.as_ref().is_some_and(|f| {
                        value.get("id") == Some(&json!(format!("formatting-{}", f.id)))
                    })
                {
                    let pending = client.formatting.take().expect("matched formatting");
                    let result = if let Some(error) = value.get("error") {
                        Err(format!(
                            "Formatação falhou: {}",
                            error.to_string().chars().take(2048).collect::<String>()
                        ))
                    } else {
                        documents
                            .get(&pending.canonical)
                            .filter(|d| d.version == pending.result.version)
                            .ok_or_else(|| "Snapshot de formatação obsoleto".to_string())
                            .and_then(|d| {
                                crate::formatting::decode(
                                    value.get("result"),
                                    pending.result.path,
                                    pending.result.version,
                                    pending.result.range,
                                    &d.text,
                                )
                            })
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Formatting {
                            id: pending.id,
                            result,
                        },
                    );
                } else if value.get("id") == Some(&json!(1)) && value.get("method").is_none() {
                    if let Some(error) = value.get("error") {
                        failure = Some(format!("initialize falhou: {error}"));
                        break;
                    }
                    if let Some(event) = client.rust_capabilities(&value) {
                        emit(&tx, &stop, epoch, event);
                    }
                    if let Some(encoding) = value
                        .pointer("/result/capabilities/positionEncoding")
                        .and_then(Value::as_str)
                        && encoding != "utf-16"
                    {
                        failure = Some("Provider não negociou UTF-16".into());
                        break;
                    }
                    if value
                        .pointer("/result/capabilities/textDocumentSync")
                        .is_none()
                    {
                        failure = Some("Provider não anunciou sincronização de texto".into());
                        break;
                    }
                    if let Err(error) =
                        client.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))
                    {
                        failure = Some(error);
                        break;
                    }
                    client.rename_supported = value
                        .pointer("/result/capabilities/renameProvider")
                        .is_some_and(|v| {
                            v == &Value::Bool(true)
                                || v.as_object().is_some_and(|options| {
                                    options.get("prepareProvider").is_none_or(Value::is_boolean)
                                })
                        });
                    client.prepare_rename_supported = value
                        .pointer("/result/capabilities/renameProvider/prepareProvider")
                        == Some(&Value::Bool(true));
                    client.references_supported = value
                        .pointer("/result/capabilities/referencesProvider")
                        .is_some_and(|v| v == &Value::Bool(true) || v.is_object());
                    client.signature_help_capabilities = crate::signature::capabilities(
                        value.pointer("/result/capabilities/signatureHelpProvider"),
                    );
                    if let Some(capabilities) = &client.signature_help_capabilities {
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::SignatureHelpCapabilities {
                                language: client.language,
                                capabilities: capabilities.clone(),
                            },
                        );
                    }
                    client.completion_supported = value
                        .pointer("/result/capabilities/completionProvider")
                        .is_some_and(Value::is_object);
                    client.definition_supported = value
                        .pointer("/result/capabilities/definitionProvider")
                        .is_some_and(|v| v == &Value::Bool(true) || v.is_object());
                    client.type_definition_supported = value
                        .pointer("/result/capabilities/typeDefinitionProvider")
                        .is_some_and(|v| v == &Value::Bool(true) || v.is_object());
                    client.implementation_supported = value
                        .pointer("/result/capabilities/implementationProvider")
                        .is_some_and(|v| v == &Value::Bool(true) || v.is_object());
                    client.formatting_capabilities = FormattingCapabilities {
                        document: crate::formatting::supported(
                            value.pointer("/result/capabilities/documentFormattingProvider"),
                        ),
                        range: crate::formatting::supported(
                            value.pointer("/result/capabilities/documentRangeFormattingProvider"),
                        ),
                    };
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::FormattingCapabilities {
                            language: client.language,
                            capabilities: client.formatting_capabilities.clone(),
                        },
                    );
                    client.hover_supported = value
                        .pointer("/result/capabilities/hoverProvider")
                        .is_some_and(|v| v == &Value::Bool(true) || v.is_object());
                    client.phpactor_index = client.language == Language::Php
                        && value
                            .pointer("/result/serverInfo/name")
                            .and_then(Value::as_str)
                            == Some("phpactor/phpactor");
                    client.phpactor_completion = client.phpactor_index;
                    if client.phpactor_index {
                        client.started = Instant::now();
                        client.index_progress =
                            Some(crate::index_progress::IndexProgress::new(client.started));
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Status(ProviderStatus {
                                language: client.language,
                                state: ProviderState::Starting,
                                detail: "Phpactor: indexando workspace antes de aceitar consultas"
                                    .into(),
                            }),
                        );
                    } else if let Err(error) = client.ready(&documents, &tx, &stop, epoch) {
                        failure = Some(error);
                        break;
                    }
                } else if let Some(method) = value.get("method").and_then(Value::as_str) {
                    if let Some(id) = value.get("id") {
                        if method == "window/showMessageRequest" {
                            let response = match decode_server_choice(&value) {
                                Err(ChoiceDecode::Malformed) => Some(
                                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":"Parâmetros inválidos de escolha do provider"}}),
                                ),
                                Err(ChoiceDecode::Overflow) => {
                                    Some(json!({"jsonrpc":"2.0","id":id,"result":null}))
                                }
                                Ok((message, actions)) => {
                                    if actions.is_empty()
                                        || client.server_choice.is_some()
                                        || client.semantic_owner().is_none()
                                    {
                                        Some(json!({"jsonrpc":"2.0","id":id,"result":null}))
                                    } else {
                                        let owner = client.semantic_owner().unwrap();
                                        let token =
                                            NEXT_SERVER_CHOICE.fetch_add(1, Ordering::Relaxed);
                                        let items = actions
                                            .iter()
                                            .map(|a| a["title"].as_str().unwrap().to_owned())
                                            .collect();
                                        client.server_choice = Some(PendingServerChoice {
                                            token,
                                            id: id.clone(),
                                            actions,
                                            owner,
                                            started: Instant::now(),
                                        });
                                        emit(
                                            &tx,
                                            &stop,
                                            epoch,
                                            Event::ServerChoice(ServerChoice {
                                                token,
                                                language: client.language,
                                                message,
                                                items,
                                                owner,
                                            }),
                                        );
                                        None
                                    }
                                }
                            };
                            if let Some(response) = response
                                && let Err(error) = client.send(response)
                            {
                                failure = Some(error);
                                break;
                            }
                            continue;
                        }
                        let response = match method {
                            "workspace/configuration" => {
                                let result = if client.language == Language::Rust {
                                    crate::rust_settings::configuration(
                                        value.pointer("/params/items"),
                                    )
                                } else if client.language == Language::Python {
                                    crate::python_settings::configuration(
                                        value.pointer("/params/items"),
                                        client.python_environment.as_deref(),
                                    )
                                } else {
                                    vec![
                                        Value::Null;
                                        value
                                            .pointer("/params/items")
                                            .and_then(Value::as_array)
                                            .map_or(0, Vec::len)
                                    ]
                                };
                                json!({"jsonrpc":"2.0","id":id,"result":result})
                            }
                            "window/workDoneProgress/create" => {
                                json!({"jsonrpc":"2.0","id":id,"result":null})
                            }
                            _ => {
                                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Método não implementado por Codisseu M2"}})
                            }
                        };
                        if let Err(error) = client.send(response) {
                            failure = Some(error);
                            break;
                        }
                    } else if method == "$/progress" && client.phpactor_index {
                        use crate::index_progress::Update;
                        match client
                            .index_progress
                            .as_mut()
                            .unwrap()
                            .receive(&value["params"], Instant::now())
                        {
                            Update::Ignored => {}
                            Update::Working(detail) => emit(
                                &tx,
                                &stop,
                                epoch,
                                Event::Status(ProviderStatus {
                                    language: client.language,
                                    state: ProviderState::Starting,
                                    detail,
                                }),
                            ),
                            Update::Finished => {
                                client.phpactor_index = false;
                                client.index_progress = None;
                                if let Err(error) = client.ready(&documents, &tx, &stop, epoch) {
                                    failure = Some(error);
                                    break;
                                }
                            }
                        }
                    } else if method == "textDocument/publishDiagnostics" {
                        let params = &value["params"];
                        let Some(path) = params
                            .get("uri")
                            .and_then(Value::as_str)
                            .and_then(protocol::uri_path)
                        else {
                            continue;
                        };
                        let Some(document) = documents.get(&path) else {
                            continue;
                        };
                        if client.language != document.language
                            || client.opened.get(&path) != Some(&document.version)
                        {
                            continue;
                        }
                        let version = params.get("version").and_then(Value::as_u64);
                        if version.is_some_and(|v| v != document.version) {
                            continue;
                        }
                        let Some(diagnostics) = params.get("diagnostics").and_then(Value::as_array)
                        else {
                            continue;
                        };
                        if diagnostics.len() > 200 {
                            emit(
                                &tx,
                                &stop,
                                epoch,
                                Event::Status(ProviderStatus {
                                    language: client.language,
                                    state: ProviderState::Ready,
                                    detail:
                                        "Diagnósticos limitados aos primeiros 200 por documento"
                                            .into(),
                                }),
                            );
                        }
                        let mut items = Vec::new();
                        let mut action_diagnostics = Vec::new();
                        let mut action_bytes = 0usize;
                        for diagnostic in diagnostics.iter().take(200) {
                            let position = |section: &str, field: &str| {
                                diagnostic["range"][section][field]
                                    .as_u64()
                                    .and_then(|n| u32::try_from(n).ok())
                            };
                            let (
                                Some(start_line),
                                Some(start_character),
                                Some(end_line),
                                Some(end_character),
                            ) = (
                                position("start", "line"),
                                position("start", "character"),
                                position("end", "line"),
                                position("end", "character"),
                            )
                            else {
                                continue;
                            };
                            if (start_line, start_character) > (end_line, end_character)
                                || crate::utf16_byte_offset(
                                    &document.text,
                                    start_line,
                                    start_character,
                                )
                                .is_none()
                                || crate::utf16_byte_offset(&document.text, end_line, end_character)
                                    .is_none()
                            {
                                continue;
                            }
                            let message = diagnostic
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .chars()
                                .take(2048)
                                .collect();
                            action_bytes += diagnostic.to_string().len();
                            if action_bytes <= 1024 * 1024 {
                                action_diagnostics.push(diagnostic.clone());
                            }
                            items.push(Diagnostic {
                                path: document.path.clone(),
                                version,
                                start_line,
                                start_character,
                                end_line,
                                end_character,
                                message,
                                severity: diagnostic
                                    .get("severity")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(1) as u32,
                                unnecessary: diagnostic_unnecessary(diagnostic),
                                language: client.language,
                            });
                        }
                        client
                            .rust_features
                            .diagnostics
                            .insert(path.clone(), (document.version, action_diagnostics));
                        emit(
                            &tx,
                            &stop,
                            epoch,
                            Event::Diagnostics {
                                path: document.path.clone(),
                                version: document.version,
                                unversioned: version.is_none(),
                                items,
                            },
                        );
                    }
                }
            }
            if client.server_choice.as_ref().is_some_and(|c| {
                c.started.elapsed() > Duration::from_secs(60)
                    || !client.choice_owner_current(c.owner)
            }) {
                let _ = client.answer_choice(None, &tx, &stop, epoch);
            }
            if client.server_choice.is_none()
                && client
                    .rename
                    .as_ref()
                    .is_some_and(|r| r.started.elapsed() > Duration::from_secs(10))
            {
                let error = client
                    .rename
                    .as_ref()
                    .unwrap()
                    .error("Timeout de rename após 10 segundos".into());
                client.cancel_rename();
                emit(&tx, &stop, epoch, error);
            }
            if client.references.is_none() {
                let _ = client.close_reference_document();
            }
            if client.server_choice.is_none()
                && client
                    .references
                    .as_ref()
                    .is_some_and(|r| r.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.references.as_ref().unwrap().id;
                client.cancel_references();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::References {
                        id,
                        result: Err("Timeout de referências após 10 segundos".into()),
                    },
                );
            }
            if client.server_choice.is_none()
                && client
                    .signature_help
                    .as_ref()
                    .is_some_and(|h| h.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.signature_help.as_ref().unwrap().id;
                client.cancel_signature_help();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::SignatureHelp {
                        id,
                        result: Err("Timeout de assinatura após 10 segundos".into()),
                    },
                );
            }
            if client.server_choice.is_none()
                && client
                    .completion
                    .as_ref()
                    .is_some_and(|c| c.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.completion.as_ref().unwrap().id;
                client.cancel_completion();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::Completion {
                        id,
                        result: Err("Timeout de completion após 10 segundos".into()),
                    },
                );
            }
            if client.definition.is_none() {
                let _ = client.close_super_document();
            }
            if client.server_choice.is_none()
                && client
                    .definition
                    .as_ref()
                    .is_some_and(|d| d.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.definition.as_ref().unwrap().id;
                client.cancel_definition();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::Definition {
                        id,
                        result: Err("Timeout de definição após 10 segundos".into()),
                    },
                );
            }
            if client.server_choice.is_none()
                && client
                    .formatting
                    .as_ref()
                    .is_some_and(|f| f.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.formatting.as_ref().unwrap().id;
                client.cancel_formatting();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::Formatting {
                        id,
                        result: Err("Timeout de formatação após10 segundos".into()),
                    },
                );
            }
            if client.server_choice.is_none()
                && client
                    .hover
                    .as_ref()
                    .is_some_and(|h| h.started.elapsed() > Duration::from_secs(10))
            {
                let id = client.hover.as_ref().unwrap().id;
                client.cancel_hover();
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::Hover {
                        id,
                        result: Err("Timeout de hover após 10 segundos".into()),
                    },
                );
            }
            for event in client.rust_feature_deadlines() {
                emit(&tx, &stop, epoch, event);
            }
            if client.state == ProviderState::Starting {
                if let Some(error) = client
                    .index_progress
                    .as_ref()
                    .and_then(|index| index.timeout(Instant::now()))
                {
                    failure = Some(error.into());
                } else if !client.phpactor_index
                    && client.started.elapsed() > Duration::from_secs(15)
                {
                    failure = Some("Timeout de initialize após 15 segundos".into());
                }
            }
            if let Ok(Some(status)) = client.child.try_wait() {
                failure = Some(format!("Processo LSP encerrou: {status}"));
            }
            if let Some(error) = failure {
                if let Some(choice) = client.server_choice.take() {
                    emit(&tx, &stop, epoch, Event::ServerChoiceClosed(choice.token));
                }
                if let Some(pending) = client.rename.take() {
                    emit(&tx, &stop, epoch, pending.error(error.clone()));
                }
                if let Some(pending) = client.references.take() {
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::References {
                            id: pending.id,
                            result: Err(error.clone()),
                        },
                    );
                }
                if let Some(pending) = client.signature_help.take() {
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::SignatureHelp {
                            id: pending.id,
                            result: Err(error.clone()),
                        },
                    );
                }
                if let Some(pending) = client.completion.take() {
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Completion {
                            id: pending.id,
                            result: Err(error.clone()),
                        },
                    );
                }
                if let Some(pending) = client.definition.take() {
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Definition {
                            id: pending.id,
                            result: Err(error.clone()),
                        },
                    );
                }
                if let Some(pending) = client.hover.take() {
                    emit(
                        &tx,
                        &stop,
                        epoch,
                        Event::Hover {
                            id: pending.id,
                            result: Err(error.clone()),
                        },
                    );
                }
                client.stop();
                client.state = ProviderState::Failed;
                emit(
                    &tx,
                    &stop,
                    epoch,
                    Event::Status(ProviderStatus {
                        language: client.language,
                        state: ProviderState::Failed,
                        detail: error.clone(),
                    }),
                );
                emit(&tx, &stop, epoch, Event::Error(error));
            }
        }
    }
    for client in &mut clients {
        client.stop();
    }
}

/// Preserve provider text as inert text, including any Markdown syntax.
fn hover_text(result: Option<&Value>) -> Result<String, String> {
    let value = result.ok_or("Resposta hover sem result")?;
    if value.is_null() {
        return Ok(String::new());
    }
    let contents = value.get("contents").ok_or("Resposta hover sem contents")?;
    fn part(value: &Value) -> Result<&str, String> {
        if let Some(text) = value.as_str() {
            return Ok(text);
        }
        if value
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "plaintext" && kind != "markdown")
        {
            return Err("Formato hover desconhecido".into());
        }
        if value.get("kind").is_none() && value.get("language").and_then(Value::as_str).is_none() {
            return Err("Conteúdo hover inválido".into());
        }
        value
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| "Conteúdo hover inválido".into())
    }
    let parts = match contents.as_array() {
        Some(values) => values.iter().map(part).collect::<Result<Vec<_>, _>>()?,
        None => vec![part(contents)?],
    };
    if parts.iter().map(|s| s.len()).sum::<usize>() + parts.len().saturating_sub(1) * 2 > 32768 {
        return Err("Documentação excede limite de 32 KiB".into());
    }
    Ok(parts.join("\n\n"))
}
/// Unknown tags are valid extensions, but malformed/oversized arrays cannot
/// enable a semantic decoration. Keep their diagnostic message independently.
fn diagnostic_unnecessary(diagnostic: &Value) -> bool {
    diagnostic
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.len() <= 8
                && tags.iter().all(|tag| {
                    tag.as_u64()
                        .is_some_and(|n| n > 0 && u32::try_from(n).is_ok())
                })
                && tags.iter().any(|tag| tag.as_u64() == Some(1))
        })
}

#[cfg(test)]
mod hover_tests {
    use super::*;
    #[test]
    fn parses_inert_hover_variants_and_rejects_invalid_or_oversized() {
        assert_eq!(hover_text(Some(&Value::Null)).unwrap(), "");
        assert_eq!(
            hover_text(Some(
                &json!({"contents":["hello",{"language":"php","value":"🌧"}]})
            ))
            .unwrap(),
            "hello\n\n🌧"
        );
        assert_eq!(
            hover_text(Some(
                &json!({"contents":{"kind":"markdown","value":"[link](file:///tmp/secret)"}})
            ))
            .unwrap(),
            "[link](file:///tmp/secret)"
        );
        assert!(hover_text(None).is_err());
        assert!(hover_text(Some(&json!({"contents":{"value":"bad"}}))).is_err());
        assert!(hover_text(Some(&json!({"contents":"x".repeat(32769)}))).is_err());
    }
}

#[cfg(all(test, target_os = "linux"))]
mod shutdown_tests {
    use super::*;
    use std::fs::{self, OpenOptions};

    #[test]
    fn provider_stop_joins_transport_even_when_other_processes_hold_output_pipes() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("provider.py");
        let ready = dir.path().join("ready");
        let exit = dir.path().join("exit");
        fs::write(&script, "import pathlib,sys,time\npathlib.Path(sys.argv[1]).touch()\nwhile not pathlib.Path(sys.argv[2]).exists(): time.sleep(.01)\n").unwrap();
        let mut connection = Connection::start(
            ProviderConfig {
                language: Language::Php,
                program: "/usr/bin/python3".into(),
                args: vec![
                    script.display().to_string(),
                    ready.display().to_string(),
                    exit.display().to_string(),
                ],
            },
            dir.path(),
            None,
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        // Retain the child's write endpoints as an inherited descendant would.
        // No escaped/orphan process is needed, and dropping guards cleans up a
        // failed baseline even if its old blocking joins have not completed.
        let stdout = OpenOptions::new()
            .write(true)
            .open(format!("/proc/{}/fd/1", connection.child.id()))
            .unwrap();
        let stderr = OpenOptions::new()
            .write(true)
            .open(format!("/proc/{}/fd/2", connection.child.id()))
            .unwrap();
        fs::write(exit, "").unwrap();
        while connection.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        let (done, result) = mpsc::channel();
        let task = thread::spawn(move || {
            connection.stop();
            assert_eq!(connection.state, ProviderState::Stopped);
            assert!(connection.tasks.is_empty());
            assert!(connection.child.try_wait().unwrap().is_some());
            done.send(()).unwrap();
        });
        let stopped = result.recv_timeout(Duration::from_secs(2));
        drop((stdout, stderr));
        task.join().unwrap();
        stopped.expect("Stop cannot await EOF from pipes retained outside the provider group");
    }
}
