//! Explicit Docker CLI integration. Opening a workspace never starts this service.
//! All daemon operations use a captured context and full container identity.
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

mod process;
#[cfg(test)]
mod tests;

const MAX_CONTAINERS: usize = 4096;
const MAX_CONTEXTS: usize = 128;
const JSON_FORMAT: &str = "{{json .}}";
const CONTAINERS_FORMAT: &str = r#"{"ID":{{json .ID}},"Names":{{json .Names}},"Image":{{json .Image}},"State":{{json .State}},"Status":{{json .Status}},"Ports":{{json .Ports}},"ComposeProject":{{json (.Label "com.docker.compose.project")}},"ComposeService":{{json (.Label "com.docker.compose.service")}}}"#;
const DETAILS_FORMAT: &str = r#"{"ID":{{json .Id}},"Name":{{json .Name}},"Image":{{json .Config.Image}},"State":{{json .State.Status}},"Created":{{json .Created}},"Ports":{{json .NetworkSettings.Ports}},"Mounts":{{json .Mounts}},"Networks":{{json .NetworkSettings.Networks}},"WorkingDir":{{json .Config.WorkingDir}},"User":{{json .Config.User}},"RestartPolicy":{{json .HostConfig.RestartPolicy.Name}}}"#;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DockerContext {
    pub name: String,
    pub description: String,
    pub current: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: String,
    pub compose_project: Option<String>,
    pub compose_service: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContainerDetails {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
    pub created: String,
    pub ports: String,
    pub mounts: Vec<String>,
    pub networks: Vec<String>,
    pub working_dir: String,
    pub user: String,
    pub restart_policy: String,
}
#[derive(Clone, Debug, Default)]
pub struct DockerState {
    pub context: Option<String>,
    pub contexts: Vec<DockerContext>,
    pub containers: Vec<Container>,
    pub selected: Option<String>,
    pub details: Option<ContainerDetails>,
    pub logs: String,
    pub error: Option<String>,
    pub notice: Option<String>,
    pub pending: Option<String>,
    pub truncated: bool,
    pub connected: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Remove,
    StopAndRemove,
}
impl ContainerAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "Iniciar",
            Self::Stop => "Parar",
            Self::Restart => "Reiniciar",
            Self::Pause => "Pausar",
            Self::Unpause => "Continuar",
            Self::Remove => "Excluir",
            Self::StopAndRemove => "Parar e excluir",
        }
    }
    pub fn can_apply(self, container: &Container) -> bool {
        match self {
            Self::Start => matches!(container.state.as_str(), "created" | "exited"),
            Self::Stop | Self::Restart => {
                matches!(container.state.as_str(), "running" | "restarting")
            }
            Self::Pause => container.state == "running",
            Self::Unpause => container.state == "paused",
            Self::Remove => matches!(container.state.as_str(), "created" | "exited" | "dead"),
            Self::StopAndRemove => matches!(
                container.state.as_str(),
                "created" | "exited" | "dead" | "running" | "restarting" | "paused"
            ),
        }
    }
    fn argument(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Pause => "pause",
            Self::Unpause => "unpause",
            Self::Remove | Self::StopAndRemove => "rm",
        }
    }
}
#[derive(Clone, Debug)]
pub enum DockerCommand {
    Refresh,
    SelectContext(String),
    Select(String),
    Logs { id: String },
    Action { id: String, action: ContainerAction },
    Configure { program: PathBuf },
    Cancel,
}
#[derive(Clone, Debug)]
pub struct ExecSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub context: String,
    pub id: String,
}
struct Active {
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<Result<DockerState, String>>,
    mutation: bool,
    background: bool,
}
pub struct DockerService {
    pub state: DockerState,
    program: PathBuf,
    active: Option<Active>,
    deferred: Option<DockerCommand>,
}
impl Default for DockerService {
    fn default() -> Self {
        Self {
            state: DockerState::default(),
            program: "docker".into(),
            active: None,
            deferred: None,
        }
    }
}
impl DockerService {
    pub fn program(&self) -> &Path {
        &self.program
    }
    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }
    /// Quiet revalidation keeps the last complete view until the worker replies.
    pub fn refresh_in_background(&mut self) -> Result<(), String> {
        if !self.state.connected {
            return Err("Conecte ao Docker antes da atualização automática".into());
        }
        self.start(DockerCommand::Refresh, true)
    }
    pub fn is_background_refresh(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.background) && self.deferred.is_none()
    }
    pub fn is_foreground_busy(&self) -> bool {
        self.is_busy() && !self.is_background_refresh()
    }
    /// User intents preempt a read, but its process must be collected first.
    pub fn interrupt_background_refresh(&mut self) {
        if self.is_background_refresh() {
            self.active
                .as_ref()
                .unwrap()
                .cancel
                .store(true, Ordering::SeqCst);
        }
    }
    pub fn dispatch(&mut self, command: DockerCommand) -> Result<(), String> {
        if matches!(command, DockerCommand::Cancel) {
            self.cancel();
            return Ok(());
        }
        if self.is_background_refresh() {
            self.interrupt_background_refresh();
            self.deferred = Some(command);
            return Ok(());
        }
        self.start(command, false)
    }
    fn start(&mut self, command: DockerCommand, background: bool) -> Result<(), String> {
        if self.is_busy() {
            return Err("Aguarde ou cancele a operação Docker atual".into());
        }
        if let DockerCommand::Configure { program } = command {
            if program.as_os_str().is_empty() {
                return Err("Informe o executável Docker".into());
            }
            self.program = program;
            self.state = DockerState::default();
            return Ok(());
        }
        let label = match &command {
            DockerCommand::SelectContext(context) => {
                if !self.state.contexts.iter().any(|c| &c.name == context) {
                    return Err("Contexto Docker não listado; atualize a conexão".into());
                }
                self.state.context = Some(context.clone());
                self.state.containers.clear();
                self.state.selected = None;
                self.state.details = None;
                self.state.logs.clear();
                self.state.connected = false;
                "Conectando".to_owned()
            }
            DockerCommand::Select(id) => {
                self.container(id)?;
                self.state.selected = Some(id.clone());
                self.state.details = None;
                self.state.logs.clear();
                "Carregando detalhes".to_owned()
            }
            DockerCommand::Logs { id } => {
                self.selected_container(id)?;
                "Carregando logs".to_owned()
            }
            DockerCommand::Action { id, action } => {
                if !action.can_apply(self.selected_container(id)?) {
                    return Err(
                        "Ação indisponível no estado atual do container; atualize a lista".into(),
                    );
                }
                action.label().to_owned()
            }
            DockerCommand::Refresh => "Atualizando".to_owned(),
            DockerCommand::Configure { .. } | DockerCommand::Cancel => unreachable!(),
        };
        let mutation = matches!(command, DockerCommand::Action { .. });
        let snapshot = self.state.clone();
        let program = self.program.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let handle = std::thread::Builder::new()
            .name("codisseu-docker".into())
            .spawn(move || execute(&program, snapshot, command, &worker_cancel))
            .map_err(|e| format!("Não foi possível iniciar Docker: {e}"))?;
        if !background {
            self.state.error = None;
            self.state.notice = None;
            self.state.pending = Some(label);
        }
        self.active = Some(Active {
            cancel,
            handle,
            mutation,
            background,
        });
        Ok(())
    }
    fn container(&self, id: &str) -> Result<&Container, String> {
        if !self.state.connected || self.state.context.is_none() || !valid_id(id) {
            return Err("Conexão ou identidade Docker inválida; atualize a lista".into());
        }
        self.state
            .containers
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| "Container não listado no contexto atual; atualize a lista".into())
    }
    fn selected_container(&self, id: &str) -> Result<&Container, String> {
        if self.state.selected.as_deref() != Some(id) {
            return Err("A seleção do container mudou; selecione novamente".into());
        }
        self.container(id)
    }
    pub fn shell_args(&self, id: &str, shell: &str) -> Result<Vec<String>, String> {
        Ok(self.exec_spec(id, shell)?.args)
    }
    pub fn exec_spec(&self, id: &str, shell: &str) -> Result<ExecSpec, String> {
        if self.is_busy() {
            return Err("Aguarde a operação Docker atual antes de abrir o shell".into());
        }
        let container = self.selected_container(id)?;
        if container.state != "running" {
            return Err("O container precisa estar em execução para abrir um shell".into());
        }
        if !matches!(shell, "sh" | "bash") {
            return Err("Shell Docker suportado: sh ou bash".into());
        }
        let context = self
            .state
            .context
            .clone()
            .ok_or("Contexto Docker ausente")?;
        Ok(ExecSpec {
            program: self.program.clone(),
            args: vec![
                "--context".into(),
                context.clone(),
                "exec".into(),
                "-it".into(),
                id.into(),
                shell.into(),
            ],
            context,
            id: id.into(),
        })
    }
    pub fn cancel(&mut self) {
        self.deferred = None;
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::SeqCst);
            if !active.background {
                self.state.pending = Some("Cancelando".into());
            }
        }
    }
    pub fn poll(&mut self) -> bool {
        if !self.active.as_ref().is_some_and(|a| a.handle.is_finished()) {
            return false;
        }
        let active = self.active.take().expect("active checked");
        let cancelled = active.cancel.load(Ordering::SeqCst);
        match active.handle.join() {
            Ok(Ok(state)) if !cancelled => self.state = state,
            _ if cancelled && active.background => {}
            result => {
                let message = if cancelled {
                    "Operação Docker interrompida".to_owned()
                } else {
                    match result {
                        Ok(Err(error)) => error,
                        _ => "Worker Docker encerrado inesperadamente".into(),
                    }
                };
                self.state.error = Some(message);
                self.state.pending = None;
                if active.mutation {
                    self.state.connected = false;
                    self.state.notice=Some("A interrupção não desfaz ações no daemon. Atualize para conferir o estado real".into());
                }
            }
        }
        if let Some(command) = self.deferred.take()
            && let Err(error) = self.dispatch(command)
        {
            self.state.error = Some(error);
        }
        true
    }
    /// Request shutdown without blocking the caller. The owner keeps polling
    /// until `is_busy()` becomes false; only completed workers are joined.
    pub fn shutdown(&mut self) {
        self.cancel();
        self.poll();
    }
}
impl Drop for DockerService {
    fn drop(&mut self) {
        self.shutdown();
        // An unfinished worker owns its process and cancellation flag. Dropping
        // JoinHandle detaches that reaper rather than waiting on the GUI thread.
        // Normal application shutdown retains this service until poll gets ACK.
    }
}
fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn field(value: &Value, name: &str) -> String {
    inert(value.get(name).and_then(Value::as_str).unwrap_or_default())
}
fn optional(value: &Value, name: &str) -> Option<String> {
    let s = field(value, name);
    (!s.is_empty()).then_some(s)
}
fn json_lines(bytes: &[u8], limit: usize) -> Result<Vec<Value>, String> {
    let source = std::str::from_utf8(bytes).map_err(|_| "Docker retornou JSON sem UTF-8 válido")?;
    let mut values = Vec::new();
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        if values.len() >= limit {
            return Err(format!("Docker excedeu o limite de {limit} registros"));
        }
        values.push(
            serde_json::from_str(line).map_err(|e| format!("Resposta Docker inválida: {e}"))?,
        );
    }
    Ok(values)
}
fn parse_contexts(bytes: &[u8]) -> Result<Vec<DockerContext>, String> {
    let mut result = Vec::new();
    for value in json_lines(bytes, MAX_CONTEXTS)? {
        let name = field(&value, "Name");
        if name.is_empty()
            || name.len() > 256
            || name.chars().any(char::is_control)
            || name.starts_with('-')
        {
            return Err("Nome de contexto Docker inválido".into());
        }
        if result.iter().any(|c: &DockerContext| c.name == name) {
            return Err("Contextos Docker duplicados".into());
        }
        result.push(DockerContext {
            name,
            description: field(&value, "Description"),
            current: value
                .get("Current")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    Ok(result)
}
fn parse_containers(bytes: &[u8]) -> Result<Vec<Container>, String> {
    let mut result = Vec::new();
    for value in json_lines(bytes, MAX_CONTAINERS)? {
        let id = field(&value, "ID");
        if !valid_id(&id) || result.iter().any(|c: &Container| c.id == id) {
            return Err("Identidade de container Docker inválida ou duplicada".into());
        }
        result.push(Container {
            id,
            name: field(&value, "Names"),
            image: field(&value, "Image"),
            state: field(&value, "State"),
            status: field(&value, "Status"),
            ports: field(&value, "Ports"),
            compose_project: optional(&value, "ComposeProject"),
            compose_service: optional(&value, "ComposeService"),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(result)
}
fn parse_details(bytes: &[u8], id: &str) -> Result<ContainerDetails, String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|e| format!("Detalhes Docker inválidos: {e}"))?;
    if field(&value, "ID") != id {
        return Err("Identidade dos detalhes Docker difere da seleção".into());
    }
    let ports = value
        .get("Ports")
        .filter(|v| !v.is_null())
        .map(|v| inert(&v.to_string()))
        .unwrap_or_default();
    if value
        .get("Mounts")
        .and_then(Value::as_array)
        .is_some_and(|a| a.len() > 256)
        || value
            .get("Networks")
            .and_then(Value::as_object)
            .is_some_and(|n| n.len() > 256)
    {
        return Err("Detalhes Docker excedem o limite de 256 mounts ou redes".into());
    }
    let mounts = value
        .get("Mounts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|m| {
                    format!(
                        "{} → {} ({})",
                        field(m, "Source"),
                        field(m, "Destination"),
                        field(m, "Type")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let networks = value
        .get("Networks")
        .and_then(Value::as_object)
        .map(|n| {
            n.iter()
                .map(|(name, data)| format!("{} · {}", inert(name), field(data, "IPAddress")))
                .collect()
        })
        .unwrap_or_default();
    Ok(ContainerDetails {
        id: id.into(),
        name: field(&value, "Name").trim_start_matches('/').into(),
        image: field(&value, "Image"),
        state: field(&value, "State"),
        created: field(&value, "Created"),
        ports,
        mounts,
        networks,
        working_dir: field(&value, "WorkingDir"),
        user: field(&value, "User"),
        restart_policy: field(&value, "RestartPolicy"),
    })
}
/// Render as plain text; discard terminal escape sequences and C0/C1 controls.
fn inert(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if !c.is_control() || matches!(c, '\n' | '\r' | '\t') {
            result.push(c);
        }
    }
    result
}
fn args(context: &str, tail: &[&str]) -> Vec<String> {
    ["--context", context]
        .into_iter()
        .chain(tail.iter().copied())
        .map(String::from)
        .collect()
}
fn inspect(
    program: &Path,
    context: &str,
    id: &str,
    cancel: &AtomicBool,
) -> Result<ContainerDetails, String> {
    let output = process::run(
        program,
        &args(
            context,
            &["container", "inspect", "--format", DETAILS_FORMAT, id],
        ),
        cancel,
        false,
    )?;
    parse_details(&output.stdout, id)
}
fn inventory(program: &Path, context: &str, cancel: &AtomicBool) -> Result<Vec<Container>, String> {
    let output = process::run(
        program,
        &args(
            context,
            &[
                "container",
                "ls",
                "--all",
                "--no-trunc",
                "--format",
                CONTAINERS_FORMAT,
            ],
        ),
        cancel,
        false,
    )?;
    parse_containers(&output.stdout)
}
fn execute(
    program: &Path,
    mut state: DockerState,
    command: DockerCommand,
    cancel: &AtomicBool,
) -> Result<DockerState, String> {
    if matches!(
        command,
        DockerCommand::Refresh | DockerCommand::SelectContext(_)
    ) {
        let output = process::run(
            program,
            &["context", "ls", "--format", JSON_FORMAT].map(String::from),
            cancel,
            false,
        )?;
        state.contexts = parse_contexts(&output.stdout)?;
        let context = state
            .context
            .clone()
            .or_else(|| {
                state
                    .contexts
                    .iter()
                    .find(|c| c.current)
                    .map(|c| c.name.clone())
            })
            .ok_or("Docker não informou um contexto atual")?;
        if !state.contexts.iter().any(|c| c.name == context) {
            return Err("O contexto Docker selecionado não está mais disponível".into());
        }
        state.context = Some(context.clone());
        for item in &mut state.contexts {
            item.current = item.name == context;
        }
        state.containers = inventory(program, &context, cancel)?;
        state.connected = true;
        state.truncated = false;
        if let Some(id) = state.selected.clone() {
            if state.containers.iter().any(|c| c.id == id) {
                state.details = Some(inspect(program, &context, &id, cancel)?);
            } else {
                state.selected = None;
                state.details = None;
                state.logs.clear();
            }
        }
    } else {
        let context = state.context.as_deref().ok_or("Contexto Docker ausente")?;
        match command {
            DockerCommand::Select(id) => {
                state.details = Some(inspect(program, context, &id, cancel)?)
            }
            DockerCommand::Logs { id } => {
                let output = process::run(
                    program,
                    &args(
                        context,
                        &["container", "logs", "--tail", "500", "--timestamps", &id],
                    ),
                    cancel,
                    true,
                )?;
                let mut bytes = output.stdout;
                bytes.extend_from_slice(&output.stderr);
                state.logs = inert(&String::from_utf8_lossy(&bytes));
                state.truncated = output.truncated;
            }
            DockerCommand::Action { id, action } => {
                // Recheck daemon state immediately before mutation, rather than trusting a stale list.
                let current = inspect(program, context, &id, cancel)?;
                let name = state
                    .containers
                    .iter()
                    .find(|container| container.id == id)
                    .map(|container| container.name.clone())
                    .unwrap_or_else(|| id[..12].to_owned());
                let container = Container {
                    state: current.state,
                    ..Container::default()
                };
                if !action.can_apply(&container) {
                    return Err(
                        "O estado do container mudou; atualize antes de repetir a ação".into(),
                    );
                }
                if action == ContainerAction::StopAndRemove {
                    stop_before_remove(program, context, &id, &container.state, cancel)?;
                }
                let tail = if matches!(action, ContainerAction::Stop | ContainerAction::Restart) {
                    vec!["container", action.argument(), "--time", "10", &id]
                } else {
                    vec!["container", action.argument(), &id]
                };
                process::run(program, &args(context, &tail), cancel, false).map_err(|error| {
                    if matches!(
                        action,
                        ContainerAction::Remove | ContainerAction::StopAndRemove
                    ) {
                        format!("Não foi possível excluir o container “{name}”: {error}")
                    } else {
                        error
                    }
                })?;
                state.containers = inventory(program, context, cancel)?;
                if state.containers.iter().any(|c| c.id == id) {
                    state.details = Some(inspect(program, context, &id, cancel)?);
                } else {
                    state.selected = None;
                    state.details = None;
                    state.logs.clear();
                }
                state.notice = Some(
                    if matches!(
                        action,
                        ContainerAction::Remove | ContainerAction::StopAndRemove
                    ) {
                        format!("Container “{name}” excluído.")
                    } else {
                        format!("{}: operação concluída", action.label())
                    },
                );
            }
            _ => unreachable!(),
        }
    }
    state.error = None;
    state.pending = None;
    Ok(state)
}

/// The combined action is an explicit user intent. Ordinary Remove never stops
/// a container that became active after its confirmation.
fn stop_before_remove(
    program: &Path,
    context: &str,
    id: &str,
    state: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if state == "paused" {
        process::run(
            program,
            &args(context, &["container", "unpause", id]),
            cancel,
            false,
        )
        .map_err(|error| {
            format!("Não foi possível retomar o container para pará-lo. A exclusão não foi executada: {error}")
        })?;
    }
    if matches!(state, "running" | "restarting" | "paused") {
        process::run(
            program,
            &args(context, &["container", "stop", "--timeout", "10", id]),
            cancel,
            false,
        )
        .map_err(|error| {
            format!("Não foi possível parar o container. A exclusão não foi executada: {error}")
        })?;
    }
    let stopped = inspect(program, context, id, cancel)?;
    if !ContainerAction::Remove.can_apply(&Container {
        state: stopped.state,
        ..Container::default()
    }) {
        return Err("O container continua ativo ou mudou de estado; não foi excluído. Atualize a lista antes de tentar novamente".into());
    }
    Ok(())
}
