use crate::{transport::Connection, *};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

fn resolve_php_executable(
    configured: &std::path::Path,
    bundled: impl FnOnce() -> Result<Option<PathBuf>, String>,
) -> Result<PathBuf, String> {
    if configured.as_os_str() != "php" {
        return Ok(configured.to_path_buf());
    }
    Ok(bundled()?.unwrap_or_else(|| configured.to_path_buf()))
}

#[derive(Clone)]
enum Endpoint {
    #[cfg(unix)]
    Unix(PathBuf),
    #[cfg(not(unix))]
    Tcp(u16),
}
impl Endpoint {
    fn connect(&self, launch: Value) -> Result<Connection, String> {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => {
                let stream =
                    std::os::unix::net::UnixStream::connect(path).map_err(|x| x.to_string())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .map_err(|x| x.to_string())?;
                Ok(Connection::new(
                    stream.try_clone().map_err(|x| x.to_string())?,
                    stream,
                    launch,
                ))
            }
            #[cfg(not(unix))]
            Self::Tcp(port) => {
                let stream = std::net::TcpStream::connect(("127.0.0.1", *port))
                    .map_err(|x| x.to_string())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .map_err(|x| x.to_string())?;
                Ok(Connection::new(
                    stream.try_clone().map_err(|x| x.to_string())?,
                    stream,
                    launch,
                ))
            }
        }
    }
    fn argument(&self) -> std::ffi::OsString {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => path.as_os_str().to_os_string(),
            #[cfg(not(unix))]
            Self::Tcp(port) => port.to_string().into(),
        }
    }
}
pub(crate) enum WorkerEvent {
    Session(Box<DebugSession>),
}
#[derive(Clone)]
enum Purpose {
    Initialize,
    Launch,
    Configure,
    Control,
    Threads,
    Stack,
    Scopes,
    Variables(i64),
    Evaluate(String),
    Breakpoints(PathBuf, Vec<u32>, u64),
    ExceptionFilters(Vec<String>, u64),
    Ignore,
}
struct Pending {
    kind: Purpose,
    sent: Instant,
    epoch: u64,
}
pub(crate) struct Worker {
    epoch: u64,
    started: Instant,
    shutdown: Arc<AtomicBool>,
    session: DebugSession,
    config: DebugConfiguration,
    breakpoints: BTreeMap<PathBuf, Vec<u32>>,
    breakpoint_generations: BTreeMap<(usize, PathBuf), u64>,
    adapter_breakpoints: BTreeMap<(usize, i64), (PathBuf, u32)>,
    exception_capabilities: BTreeMap<usize, Value>,
    exception_generations: BTreeMap<usize, u64>,
    connections: Vec<Connection>,
    pending: BTreeMap<(usize, u64), Pending>,
    events: SyncSender<WorkerEvent>,
    commands: Receiver<DebugCommand>,
    adapter: Option<Child>,
    extras: Vec<Child>,
    logs: Receiver<String>,
    log_tx: SyncSender<String>,
    endpoint: Option<Endpoint>,
    owned_directory: Option<PathBuf>,
    active: usize,
    thread: i64,
    dirty: bool,
    stop_deadline: Option<Instant>,
}
impl Worker {
    pub fn run(
        session: DebugSession,
        config: DebugConfiguration,
        bps: Vec<Breakpoint>,
        commands: Receiver<DebugCommand>,
        events: SyncSender<WorkerEvent>,
        shutdown: Arc<AtomicBool>,
    ) -> DebugSession {
        let (log_tx, logs) = mpsc::sync_channel(64);
        let mut worker = Self {
            epoch: 0,
            started: Instant::now(),
            shutdown,
            session,
            config,
            breakpoints: BTreeMap::new(),
            breakpoint_generations: BTreeMap::new(),
            adapter_breakpoints: BTreeMap::new(),
            exception_capabilities: BTreeMap::new(),
            exception_generations: BTreeMap::new(),
            connections: vec![],
            pending: BTreeMap::new(),
            events,
            commands,
            adapter: None,
            extras: vec![],
            logs,
            log_tx,
            endpoint: None,
            owned_directory: None,
            active: 0,
            thread: 0,
            dirty: true,
            stop_deadline: None,
        };
        worker.session.breakpoints = bps.clone();
        for breakpoint in &mut worker.session.breakpoints {
            breakpoint.verified = false;
            breakpoint.message = None;
        }
        for b in bps.into_iter().filter(|x| x.enabled) {
            worker.breakpoints.entry(b.path).or_default().push(b.line)
        }
        if let Err(e) = worker.start() {
            if worker.shutdown.load(Ordering::Acquire) {
                worker.session.state = DebugState::Terminated;
            } else {
                worker.fail(e)
            }
        } else {
            worker.main_loop()
        }
        worker.cleanup();
        worker.publish();
        worker.session
    }
    fn node(&self) -> Result<PathBuf, String> {
        self.config
            .adapter_runtime
            .clone()
            .map_or_else(codisseu_runtime::node, Ok)
    }
    fn start(&mut self) -> Result<(), String> {
        let php = self.config.runtime == DebugRuntime::Php;
        let rust = self.config.runtime == DebugRuntime::Rust;
        let python = self.config.runtime == DebugRuntime::Python;
        let python_adapter = if python {
            Some(codisseu_runtime::python_debug_adapter()?)
        } else {
            None
        };
        let adapter = self.config.adapter.clone().map_or_else(
            || {
                if let Some((_, package)) = &python_adapter {
                    Ok(package.join("adapter/__main__.py"))
                } else if rust {
                    rust_debug_adapter()
                } else {
                    codisseu_runtime::debug_adapter(php)
                }
            },
            Ok,
        )?;
        if !adapter.is_file() {
            return Err(format!("Debug adapter not found: {}", adapter.display()));
        }
        if php && self.config.request == DebugRequest::Launch {
            self.config.executable = resolve_php_executable(&self.config.executable, || {
                codisseu_runtime::discover()?
                    .map(|resources| resources.tool("php"))
                    .transpose()
            })?;
            crate::php_runtime::validate_launch(&self.config, &self.shutdown)?;
        }
        if python
            && matches!(
                self.config.executable.to_str(),
                Some("" | "python" | "python3")
            )
        {
            self.config.executable = codisseu_runtime::python()?;
        }
        let mut command = if let Some((engine, _)) = &python_adapter {
            let mut command = Command::new(self.config.adapter_runtime.as_ref().unwrap_or(engine));
            command.arg("-I");
            if self.config.adapter.is_some() {
                command.arg(&adapter);
            } else {
                command.args(["-m", "debugpy.adapter"]);
            }
            command
        } else if rust {
            Command::new(&adapter)
        } else {
            let mut command = Command::new(self.node()?);
            command.arg(&adapter);
            command
        };
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        command.env_clear();
        for key in [
            "PATH",
            "HOME",
            "USER",
            "LOGNAME",
            "SHELL",
            "TMPDIR",
            "TEMP",
            "TMP",
            "XDG_RUNTIME_DIR",
            "DISPLAY",
            "WAYLAND_DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "LANG",
            "LC_ALL",
            "LC_CTYPE",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "SystemRoot",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        if rust {
            for key in ["RUSTUP_HOME", "CARGO_HOME", "RUSTUP_TOOLCHAIN"] {
                if let Some(value) = std::env::var_os(key) {
                    command.env(key, value);
                }
            }
        }
        if python {
            command.envs(codisseu_runtime::python_environment()?);
        }
        command
            .current_dir(&self.config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let launch = self.launch_arguments()?;
        if php || rust || python {
            let mut child = command.spawn().map_err(|e| {
                format!(
                    "Cannot start {} debug adapter: {e}",
                    if rust {
                        "Rust"
                    } else if python {
                        "Python"
                    } else {
                        "PHP"
                    }
                )
            })?;
            let reader = child.stdout.take().ok_or("Adapter stdout missing")?;
            let writer = child.stdin.take().ok_or("Adapter stdin missing")?;
            self.capture_logs(child.stderr.take());
            self.adapter = Some(child);
            self.connections
                .push(Connection::new(reader, writer, launch));
        } else {
            #[cfg(unix)]
            let endpoint = {
                use std::os::unix::fs::DirBuilderExt;
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|x| x.to_string())?
                    .as_nanos();
                let directory = std::env::temp_dir().join(format!(
                    "codisseu-dap-{}-{}-{nonce}",
                    std::process::id(),
                    self.session.id
                ));
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&directory)
                    .map_err(|x| x.to_string())?;
                self.owned_directory = Some(directory.clone());
                Endpoint::Unix(directory.join("adapter.sock"))
            };
            #[cfg(not(unix))]
            let endpoint = {
                let listener =
                    std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|x| x.to_string())?;
                Endpoint::Tcp(listener.local_addr().map_err(|x| x.to_string())?.port())
            };
            command.arg(endpoint.argument()).arg("127.0.0.1");
            let mut child = command
                .spawn()
                .map_err(|e| format!("Cannot start JS debug adapter: {e}"))?;
            self.capture_logs(child.stderr.take());
            self.capture_logs(child.stdout.take());
            self.adapter = Some(child);
            self.endpoint = Some(endpoint.clone());
            let deadline = Instant::now() + Duration::from_secs(5);
            let connection = loop {
                match endpoint.connect(launch.clone()) {
                    Ok(connection) => break connection,
                    Err(e) => {
                        if Instant::now() > deadline {
                            return Err(format!("Cannot connect to JS debug adapter: {e}"));
                        }
                        if self.shutdown.load(Ordering::Acquire)
                            || matches!(self.commands.try_recv(), Ok(DebugCommand::Stop(_)))
                        {
                            return Err("Debug startup cancelled".into());
                        }
                        thread::sleep(Duration::from_millis(15));
                    }
                }
            };
            self.connections.push(connection);
        }
        self.initialize(0)?;
        Ok(())
    }
    fn capture_logs<R: std::io::Read + Send + 'static>(&self, reader: Option<R>) {
        if let Some(reader) = reader {
            let tx = self.log_tx.clone();
            let secret = self.config.browser_inspect_uri.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(reader);
                loop {
                    let mut bytes = Vec::new();
                    match (&mut reader).take(65536).read_until(b'\n', &mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let mut text = String::from_utf8_lossy(&bytes).into_owned();
                            if let Some(secret) = &secret {
                                text = secret.redact(&text);
                            }
                            if tx.try_send(text).is_err() { /* bounded diagnostic output */ }
                        }
                    }
                }
            });
        }
    }
    fn launch_arguments(&self) -> Result<Value, String> {
        let c = &self.config;
        let program = c.program.as_ref().map(|x| x.to_string_lossy().into_owned());
        let executable =
            if c.runtime == DebugRuntime::Node && c.executable == std::path::Path::new("node") {
                self.node()?
            } else {
                c.executable.clone()
            };
        Ok(match c.runtime {
            DebugRuntime::Python => {
                let mut launch = json!({
                    "type":"python", "name":c.name, "request":"launch",
                    "python":[executable], "cwd":c.cwd, "args":c.args,
                    "env":c.env, "stopOnEntry":c.stop_on_entry,
                    "console":"internalConsole", "redirectOutput":true,
                    "justMyCode":true, "subProcess":false,
                    "pathMappings":c.path_mappings.iter().map(|(remote, local)| json!({"remoteRoot":remote,"localRoot":local})).collect::<Vec<_>>(),
                });
                if let Some(module) = &c.module {
                    launch["module"] = json!(module);
                } else {
                    launch["program"] = json!(program);
                }
                launch
            }
            DebugRuntime::Rust => json!({
                "type":"lldb", "name":c.name, "request":"launch",
                "program":program, "cwd":c.cwd, "args":c.args,
                "env":c.env, "stopOnEntry":c.stop_on_entry,
                "terminal":"console", "sourceLanguages":["rust"],
                "sourceMap":c.path_mappings,
            }),
            DebugRuntime::Php => {
                let mut v = json!({"type":"php","name":c.name,"request":"launch","hostname":if c.request==DebugRequest::Launch{"127.0.0.1"}else{c.listen_host.as_str()},"port":if c.request==DebugRequest::Launch{c.port}else if c.port==0{9003}else{c.port},"stopOnEntry":c.stop_on_entry,"pathMappings":c.path_mappings,"cwd":c.cwd,"env":c.env,"maxConnections":8,"xdebugSettings":{"max_children":100,"max_data":4096,"max_depth":3}});
                if c.request == DebugRequest::Launch {
                    v["program"] = json!(program);
                    v["runtimeExecutable"] = json!(executable);
                    v["runtimeArgs"] = json!([
                        "-d",
                        "xdebug.mode=debug",
                        "-d",
                        "xdebug.start_with_request=yes",
                        "-d",
                        "xdebug.client_host=127.0.0.1",
                        "-d",
                        "xdebug.client_port=${port}",
                        "-d",
                        "xdebug.discover_client_host=0"
                    ]);
                    v["args"] = json!(c.args);
                }
                v
            }
            DebugRuntime::Node => {
                let mut v = json!({"type":"pwa-node","__workspaceFolder":c.cwd,"name":c.name,"request":if c.request==DebugRequest::Launch{"launch"}else{"attach"},"cwd":c.cwd,"sourceMaps":c.source_maps,"sourceMapPathOverrides":c.path_mappings,"stopOnEntry":c.stop_on_entry,"console":"internalConsole","autoAttachChildProcesses":c.auto_attach_child_processes,"skipFiles":[],"env":c.env});
                if c.request == DebugRequest::Launch {
                    v["program"] = json!(program);
                    v["args"] = json!(c.args);
                    v["runtimeExecutable"] = json!(executable)
                } else {
                    v["address"] = json!("127.0.0.1");
                    v["port"] = json!(if c.port == 0 { 9229 } else { c.port });
                    v["restart"] = json!(false);
                }
                if c.path_mappings.is_empty() {
                    v.as_object_mut().unwrap().remove("sourceMapPathOverrides");
                }
                v
            }
            DebugRuntime::Chrome => {
                let mut v = json!({"type":"pwa-chrome","__workspaceFolder":c.cwd,"name":c.name,"request":if c.request==DebugRequest::Launch{"launch"}else{"attach"},"url":c.url,"webRoot":c.cwd,"cwd":c.cwd,"sourceMaps":c.source_maps,"sourceMapPathOverrides":c.path_mappings,"skipFiles":[],"console":"internalConsole"});
                if c.request == DebugRequest::Launch {
                    if !c.executable.as_os_str().is_empty() {
                        v["runtimeExecutable"] = json!(executable)
                    }
                    v["runtimeArgs"] = json!(c.args)
                } else {
                    v["address"] = json!("127.0.0.1");
                    v["port"] = json!(if c.port == 0 { 9222 } else { c.port });
                    if let Some(uri) = &c.browser_inspect_uri {
                        v["inspectUri"] = json!(uri.as_str());
                        v["trace"] = json!(false);
                        v["restart"] = json!(false);
                    }
                }
                if c.path_mappings.is_empty() {
                    v.as_object_mut().unwrap().remove("sourceMapPathOverrides");
                }
                v
            }
        })
    }
    fn initialize(&mut self, index: usize) -> Result<(), String> {
        let adapter_id = match self.config.runtime {
            DebugRuntime::Php => "php",
            DebugRuntime::Rust => "lldb",
            DebugRuntime::Python => "python",
            DebugRuntime::Node | DebugRuntime::Chrome => "pwa-node",
        };
        self.request(index,"initialize",json!({"clientID":"codisseu","clientName":"Codisseu","adapterID":adapter_id,"pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsVariableType":true,"supportsVariablePaging":true,"supportsRunInTerminalRequest":false,"supportsStartDebuggingRequest":!matches!(self.config.runtime, DebugRuntime::Rust | DebugRuntime::Python),"supportsProgressReporting":false}),Purpose::Initialize)?;
        Ok(())
    }
    fn request(
        &mut self,
        index: usize,
        cmd: &str,
        args: Value,
        kind: Purpose,
    ) -> Result<(), String> {
        if self.pending.len() >= 256 {
            return Err("Too many pending debug requests".into());
        }
        let seq = self.connections[index].request(cmd, args)?;
        self.pending.insert(
            (index, seq),
            Pending {
                kind,
                sent: Instant::now(),
                epoch: self.epoch,
            },
        );
        self.dirty = true;
        Ok(())
    }
    fn configure(&mut self, index: usize) -> Result<(), String> {
        if self.connections[index].configure_sent {
            return Ok(());
        }
        self.connections[index].configure_sent = true;
        for (path, lines) in self.breakpoints.clone() {
            self.send_breakpoints(index, path, lines)?
        }
        self.send_exception_filters(index, self.config.exception_filters.clone())?;
        self.request(index, "configurationDone", json!({}), Purpose::Configure)
    }
    fn send_breakpoints(
        &mut self,
        index: usize,
        path: PathBuf,
        lines: Vec<u32>,
    ) -> Result<(), String> {
        let generation = self
            .breakpoint_generations
            .entry((index, path.clone()))
            .or_default();
        *generation += 1;
        let generation = *generation;
        self.adapter_breakpoints
            .retain(|(connection, _), (source, _)| *connection != index || source != &path);
        let capabilities = self
            .exception_capabilities
            .get(&index)
            .cloned()
            .unwrap_or_default();
        let mut arguments = Vec::new();
        let mut sent_lines = Vec::new();
        let mut rejected = Vec::new();
        for line in lines {
            let options = self
                .session
                .breakpoints
                .iter()
                .find(|p| p.path == path && p.line == line)
                .map(|p| p.options.clone())
                .unwrap_or_default();
            if let Some(reason) = options.unsupported(&capabilities) {
                rejected.push((line, false, Some(reason.to_string())));
                continue;
            }
            let mut value = json!({"line": line});
            for (key, text) in [
                ("condition", options.condition),
                ("hitCondition", options.hit_condition),
                ("logMessage", options.log_message),
            ] {
                if !text.trim().is_empty() {
                    value[key] = json!(text);
                }
            }
            arguments.push(value);
            sent_lines.push(line);
        }
        self.update_breakpoints(path.clone(), rejected);
        self.request(index, "setBreakpoints", json!({"source":{"path":path,"name":path.file_name().map(|x|x.to_string_lossy())},"breakpoints":arguments,"sourceModified":false}), Purpose::Breakpoints(path,sent_lines,generation))
    }
    fn send_exception_filters(&mut self, index: usize, filters: Vec<String>) -> Result<(), String> {
        let capabilities = self
            .exception_capabilities
            .get(&index)
            .ok_or("Adapter capabilities are not ready")?;
        crate::service::validate_exception_filters(&filters, capabilities)?;
        let generation = self.exception_generations.entry(index).or_default();
        *generation += 1;
        let generation = *generation;
        self.request(
            index,
            "setExceptionBreakpoints",
            json!({"filters":filters}),
            Purpose::ExceptionFilters(filters, generation),
        )
    }
    fn clear_stop(&mut self) {
        self.session.frames.clear();
        self.session.scopes.clear();
        self.session.variables.clear();
        self.session.evaluations.clear();
        self.session.current_frame = None;
        self.session.current_thread = None;
        self.session.stopped_reason = None;
    }
    fn main_loop(&mut self) {
        while self.session.state.is_active() {
            if self.session.state == DebugState::Starting
                && self.started.elapsed() > Duration::from_secs(30)
            {
                self.fail("Debug target did not become ready within 30 seconds".into());
                break;
            }
            if self.shutdown.swap(false, Ordering::AcqRel) {
                let _ = self.command(DebugCommand::Stop(self.session.id));
            }
            let mut work = false;
            while let Ok(command) = self.commands.try_recv() {
                work = true;
                if let Err(e) = self.command(command) {
                    self.session.last_error = Some(e);
                    self.dirty = true
                }
            }
            while let Ok(log) = self.logs.try_recv() {
                self.output("adapter", log);
                work = true;
            }
            for index in 0..self.connections.len() {
                for _ in 0..64 {
                    match self.connections[index].rx.try_recv() {
                        Ok(Ok(message)) => {
                            work = true;
                            if let Err(e) = self.message(index, message) {
                                self.fail(e)
                            }
                        }
                        Ok(Err(e)) => {
                            if index == 0
                                && self.session.state != DebugState::Terminating
                                && self.session.state.is_active()
                            {
                                self.fail(e)
                            }
                            break;
                        }
                        Err(_) => break,
                    }
                }
            }
            if let Some(child) = self.adapter.as_mut()
                && let Ok(Some(code)) = child.try_wait()
                && self.session.state.is_active()
            {
                if self.session.state == DebugState::Terminating || code.success() {
                    self.session.state = DebugState::Terminated
                } else {
                    self.fail(format!("Debug adapter exited: {code}"))
                }
                self.dirty = true
            }
            if self.stop_deadline.is_some_and(|x| Instant::now() >= x) {
                self.session.state = DebugState::Terminated;
                self.dirty = true;
            }
            let expired: Vec<_> = self
                .pending
                .iter()
                .filter(|(_, p)| p.sent.elapsed() > Duration::from_secs(30))
                .map(|(k, _)| *k)
                .collect();
            for k in expired {
                let p = self.pending.remove(&k).unwrap();
                if matches!(
                    p.kind,
                    Purpose::Initialize | Purpose::Launch | Purpose::Configure
                ) {
                    self.fail("Debug adapter request timed out".into())
                } else {
                    self.session.last_error = Some("Debug request timed out".into());
                    self.dirty = true
                }
            }
            if self.dirty {
                self.publish()
            }
            if !work {
                thread::sleep(Duration::from_millis(8));
            }
        }
    }
    fn command(&mut self, cmd: DebugCommand) -> Result<(), String> {
        if self.session.state == DebugState::Terminating && !matches!(cmd, DebugCommand::Stop(_)) {
            return Ok(());
        }
        let i = self.active;
        let args = json!({"threadId":self.thread});
        match cmd {
            DebugCommand::Stop(_) => {
                self.epoch += 1;
                self.clear_stop();
                self.session.state = DebugState::Terminating;
                self.stop_deadline = Some(Instant::now() + Duration::from_millis(1200));
                let terminate = self.config.request == DebugRequest::Launch;
                for index in (0..self.connections.len()).rev() {
                    let _=self.request(index,"disconnect",json!({"restart":false,"terminateDebuggee":terminate,"suspendDebuggee":false}),Purpose::Ignore);
                }
                self.dirty = true;
                Ok(())
            }
            DebugCommand::Continue(_)
            | DebugCommand::Next(_)
            | DebugCommand::StepIn(_)
            | DebugCommand::StepOut(_)
            | DebugCommand::Pause(_) => {
                let command = match cmd {
                    DebugCommand::Continue(_) => "continue",
                    DebugCommand::Next(_) => "next",
                    DebugCommand::StepIn(_) => "stepIn",
                    DebugCommand::StepOut(_) => "stepOut",
                    _ => "pause",
                };
                if command == "pause" && self.session.capabilities["supportsPauseRequest"] == false
                {
                    return Err("This adapter cannot pause a running session".into());
                }
                self.epoch += 1;
                if command != "pause" {
                    self.session.state = DebugState::Running;
                    self.clear_stop();
                }
                self.request(i, command, args, Purpose::Control)
            }
            DebugCommand::Threads(_) => self.request(i, "threads", json!({}), Purpose::Threads),
            DebugCommand::SelectThread { thread_id, .. } => {
                self.epoch += 1;
                self.thread = thread_id;
                self.session.current_thread = Some(thread_id);
                self.session.frames.clear();
                self.session.scopes.clear();
                self.session.variables.clear();
                self.session.evaluations.clear();
                self.session.current_frame = None;
                self.request(
                    i,
                    "stackTrace",
                    json!({"threadId":thread_id,"startFrame":0,"levels":100}),
                    Purpose::Stack,
                )
            }
            DebugCommand::SelectFrame { frame_id, .. } => {
                self.epoch += 1;
                self.session.current_frame = Some(frame_id);
                self.session.scopes.clear();
                self.session.variables.clear();
                self.session.evaluations.clear();
                self.request(i, "scopes", json!({"frameId":frame_id}), Purpose::Scopes)
            }
            DebugCommand::Variables { reference, .. } => self.request(
                i,
                "variables",
                json!({"variablesReference":reference,"start":0,"count":200}),
                Purpose::Variables(reference),
            ),
            DebugCommand::Evaluate { expression, .. } => {
                if expression.len() > 16384 {
                    return Err("Debug expression exceeds 16 KiB".into());
                }
                // CodeLLDB's REPL defaults to debugger commands. The shared
                // Evaluate/Watches UI always requests an expression value.
                let context = if self.config.runtime == DebugRuntime::Rust {
                    "watch"
                } else {
                    "repl"
                };
                self.request(i,"evaluate",json!({"expression":expression,"frameId":self.session.current_frame,"context":context}),Purpose::Evaluate(expression))
            }
            DebugCommand::SetSourceBreakpoints { path, breakpoints } => {
                let lines: Vec<_> = breakpoints
                    .iter()
                    .filter(|b| b.enabled)
                    .map(|b| b.line)
                    .collect();
                self.session.breakpoints.retain(|x| x.path != path);
                self.session.breakpoints.extend(breakpoints);
                self.breakpoints.insert(path.clone(), lines.clone());
                for j in 0..self.connections.len() {
                    if self.connections[j].configure_sent {
                        self.send_breakpoints(j, path.clone(), lines.clone())?
                    }
                }
                Ok(())
            }
            DebugCommand::SetExceptionBreakpoints { filters, .. } => {
                // Validate every configured connection before changing any target.
                for index in 0..self.connections.len() {
                    if self.connections[index].configure_sent {
                        crate::service::validate_exception_filters(
                            &filters,
                            self.exception_capabilities
                                .get(&index)
                                .ok_or("Adapter capabilities are not ready")?,
                        )?;
                    }
                }
                self.config.exception_filters = filters.clone();
                for index in 0..self.connections.len() {
                    if self.connections[index].configure_sent {
                        self.send_exception_filters(index, filters.clone())?;
                    }
                }
                Ok(())
            }
            _ => Err("Unsupported worker command".into()),
        }
    }
    fn message(&mut self, index: usize, v: Value) -> Result<(), String> {
        match v["type"].as_str() {
            Some("response") => {
                let key = (index, v["request_seq"].as_u64().unwrap_or(0));
                let Some(p) = self.pending.remove(&key) else {
                    return Ok(());
                };
                self.dirty = true;
                if (p.epoch != self.epoch || index != self.active)
                    && matches!(
                        p.kind,
                        Purpose::Threads
                            | Purpose::Stack
                            | Purpose::Scopes
                            | Purpose::Variables(_)
                            | Purpose::Evaluate(_)
                            | Purpose::Control
                    )
                {
                    return Ok(());
                }
                let success = v["success"].as_bool().unwrap_or(false);
                if !success {
                    let message = limited(
                        v["message"]
                            .as_str()
                            .unwrap_or("Debug request failed")
                            .to_string(),
                        65536,
                    );
                    if let Purpose::Breakpoints(path, lines, generation) = &p.kind {
                        if self.breakpoint_generations.get(&(index, path.clone()))
                            == Some(generation)
                        {
                            self.update_breakpoints(
                                path.clone(),
                                lines
                                    .iter()
                                    .map(|line| (*line, false, Some(message.clone())))
                                    .collect(),
                            );
                        }
                        return Ok(());
                    }
                    if let Purpose::Evaluate(expression) = p.kind {
                        self.push_evaluation(Evaluation {
                            expression,
                            result: String::new(),
                            reference: 0,
                            error: Some(message),
                        });
                        return Ok(());
                    }
                    if matches!(
                        p.kind,
                        Purpose::Initialize | Purpose::Launch | Purpose::Configure
                    ) {
                        return Err(message);
                    }
                    self.session.last_error = Some(message);
                    return Ok(());
                }
                let b = &v["body"];
                match p.kind {
                    Purpose::Initialize => {
                        self.exception_capabilities.insert(index, b.clone());
                        if index == 0 {
                            self.session.capabilities = b.clone()
                        }
                        self.connections[index].initialized = true;
                        let launch = self.connections[index].launch.clone();
                        let request = launch["request"].as_str().unwrap_or("launch").to_string();
                        self.request(index, &request, launch, Purpose::Launch)?;
                    }
                    Purpose::Launch => {
                        if self.session.state == DebugState::Starting
                            && (matches!(
                                self.config.runtime,
                                DebugRuntime::Php | DebugRuntime::Rust | DebugRuntime::Python
                            ) || index > 0)
                        {
                            self.session.state = DebugState::Running
                        }
                    }
                    Purpose::Configure => {}
                    Purpose::Control => {}
                    Purpose::Threads => {
                        self.session.threads = b["threads"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .take(128)
                                    .map(|v| DebugThread {
                                        id: v["id"].as_i64().unwrap_or(0),
                                        name: limited(string(v, "name"), 4096),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if !self.session.threads.iter().any(|t| t.id == self.thread) {
                            self.thread = self.session.threads.first().map(|x| x.id).unwrap_or(0)
                        }
                        if !self.session.threads.is_empty()
                            && self.session.state == DebugState::Stopped
                        {
                            self.session.current_thread = Some(self.thread);
                            self.request(
                                index,
                                "stackTrace",
                                json!({"threadId":self.thread,"startFrame":0,"levels":100}),
                                Purpose::Stack,
                            )?;
                        }
                    }
                    Purpose::Stack => {
                        self.session.frames = b["stackFrames"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .take(100)
                                    .map(|v| StackFrame {
                                        id: v["id"].as_i64().unwrap_or(0),
                                        name: limited(string(v, "name"), 4096),
                                        path: v["source"]["path"].as_str().map(PathBuf::from),
                                        line: v["line"].as_u64().unwrap_or(1) as u32,
                                        column: v["column"].as_u64().unwrap_or(1) as u32,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        self.session.current_frame = self.session.frames.first().map(|x| x.id);
                        if let Some(id) = self.session.current_frame {
                            self.request(index, "scopes", json!({"frameId":id}), Purpose::Scopes)?;
                        }
                    }
                    Purpose::Scopes => {
                        self.session.scopes = b["scopes"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .take(32)
                                    .map(|v| Scope {
                                        name: limited(string(v, "name"), 4096),
                                        reference: v["variablesReference"].as_i64().unwrap_or(0),
                                        expensive: v["expensive"].as_bool().unwrap_or(false),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        for reference in self
                            .session
                            .scopes
                            .iter()
                            .filter(|x| !x.expensive)
                            .take(4)
                            .map(|x| x.reference)
                            .collect::<Vec<_>>()
                        {
                            self.request(
                                index,
                                "variables",
                                json!({"variablesReference":reference,"start":0,"count":200}),
                                Purpose::Variables(reference),
                            )?;
                        }
                    }
                    Purpose::Variables(reference) => {
                        let vars = b["variables"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .take(200)
                                    .map(|v| Variable {
                                        name: limited(string(v, "name"), 4096),
                                        value: limited(string(v, "value"), 16384),
                                        type_name: v["type"]
                                            .as_str()
                                            .map(|x| limited(x.to_string(), 4096)),
                                        reference: v["variablesReference"].as_i64().unwrap_or(0),
                                        evaluate_name: v["evaluateName"]
                                            .as_str()
                                            .map(str::to_string),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        self.session.variables.insert(reference, vars);
                        let mut trimmed = false;
                        while self.session.variables.len() > 128
                            || self
                                .session
                                .variables
                                .values()
                                .map(|vs| {
                                    vs.iter()
                                        .map(|v| {
                                            v.name.len()
                                                + v.value.len()
                                                + v.type_name.as_ref().map_or(0, String::len)
                                                + v.evaluate_name.as_ref().map_or(0, String::len)
                                        })
                                        .sum::<usize>()
                                })
                                .sum::<usize>()
                                > 4 * 1024 * 1024
                            || self.session.variables.values().map(Vec::len).sum::<usize>() > 10_000
                        {
                            self.session.variables.pop_first();
                            trimmed = true;
                        }
                        if trimmed {
                            self.session.last_error=Some("Variable display budget reached (4 MiB / 10,000 values / 128 references); older expansions were removed".into());
                        }
                    }
                    Purpose::Evaluate(expression) => {
                        self.push_evaluation(Evaluation {
                            expression,
                            result: limited(string(b, "result"), 65536),
                            reference: b["variablesReference"].as_i64().unwrap_or(0),
                            error: None,
                        });
                    }
                    Purpose::Breakpoints(path, lines, generation) => {
                        if self.breakpoint_generations.get(&(index, path.clone()))
                            != Some(&generation)
                        {
                            return Ok(());
                        }
                        let updates = b["breakpoints"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .enumerate()
                                    .filter_map(|(j, v)| {
                                        let line = lines.get(j).copied()?;
                                        if let Some(id) = v["id"].as_i64() {
                                            self.adapter_breakpoints
                                                .insert((index, id), (path.clone(), line));
                                        }
                                        Some((
                                            line,
                                            v["verified"].as_bool().unwrap_or(false),
                                            v["message"].as_str().map(str::to_string),
                                        ))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        self.update_breakpoints(path, updates);
                    }
                    Purpose::ExceptionFilters(filters, generation) => {
                        if self.config.exception_filters == filters
                            && self.exception_generations.get(&index) == Some(&generation)
                        {
                            self.session.exception_filters = filters;
                        }
                    }
                    Purpose::Ignore => {}
                }
            }
            Some("event") => {
                self.dirty = true;
                let b = &v["body"];
                if self.session.state == DebugState::Terminating
                    && !matches!(
                        v["event"].as_str(),
                        Some("output" | "terminated" | "exited")
                    )
                {
                    return Ok(());
                }
                match v["event"].as_str() {
                    Some("initialized") => self.configure(index)?,
                    Some("stopped") => {
                        // js-debug may keep a provisional marker until it is hit.
                        // Only an explicit hit ID from this adapter connection
                        // confirms the original marker; frames are not evidence.
                        if self.config.integrated_browser
                            && let Some(ids) = b["hitBreakpointIds"].as_array()
                        {
                            for id in ids.iter().filter_map(Value::as_i64) {
                                if let Some((path, line)) =
                                    self.adapter_breakpoints.get(&(index, id))
                                {
                                    for point in &mut self.session.breakpoints {
                                        if point.enabled
                                            && point.path == *path
                                            && point.line == *line
                                        {
                                            point.verified = true;
                                            point.message = None;
                                        }
                                    }
                                }
                            }
                        }
                        self.session.stop_epoch += 1;
                        self.epoch += 1;
                        self.active = index;
                        self.thread = b["threadId"].as_i64().unwrap_or(0);
                        self.session.state = DebugState::Stopped;
                        self.clear_stop();
                        self.session.current_thread = b["threadId"].as_i64();
                        self.session.stopped_reason = b["reason"].as_str().map(str::to_string);
                        self.request(index, "threads", json!({}), Purpose::Threads)?;
                    }
                    Some("continued") => {
                        if index != self.active {
                            return Ok(());
                        }
                        self.epoch += 1;
                        self.session.state = DebugState::Running;
                        self.clear_stop();
                    }
                    Some("output") if b["category"] != "telemetry" => self.output(
                        b["category"].as_str().unwrap_or("console"),
                        string(b, "output"),
                    ),
                    Some("exited") => {
                        self.session.exit_code = b["exitCode"].as_i64();
                        self.output(
                            "console",
                            format!(
                                "Process finished with exit code {}\n",
                                self.session.exit_code.unwrap_or(0)
                            ),
                        );
                    }
                    Some("thread") => {
                        if index != self.active {
                            return Ok(());
                        }
                        if b["reason"] == "started" {
                            if self.session.state != DebugState::Stopped {
                                self.thread = b["threadId"].as_i64().unwrap_or(0);
                            }
                            self.request(index, "threads", json!({}), Purpose::Threads)?;
                        }
                        if b["reason"] == "exited"
                            && self.config.runtime == DebugRuntime::Php
                            && self.config.request != DebugRequest::Launch
                        {
                            self.session
                                .threads
                                .retain(|x| Some(x.id) != b["threadId"].as_i64());
                            if b["threadId"].as_i64() == Some(self.thread) {
                                self.epoch += 1;
                                self.session.state = DebugState::Running;
                                self.clear_stop();
                            }
                        }
                        if b["reason"] == "started"
                            && self.session.runtime == DebugRuntime::Php
                            && self.session.state == DebugState::Starting
                        {
                            self.session.state = DebugState::Running;
                        }
                    }
                    Some("terminated") => {
                        if self.config.runtime == DebugRuntime::Node
                            && self.config.auto_attach_child_processes
                            && index > 0
                        {
                            if self.active == index && self.session.state != DebugState::Terminating
                            {
                                self.active = 0;
                                self.clear_stop();
                                self.session.state = DebugState::Running;
                            }
                            return Ok(());
                        }
                        if !(self.config.runtime == DebugRuntime::Php
                            && self.config.request != DebugRequest::Launch)
                        {
                            self.session.state = DebugState::Terminated;
                        } else {
                            self.epoch += 1;
                            self.session.state = DebugState::Running;
                            self.clear_stop();
                        }
                    }
                    Some("breakpoint") => {
                        let bp = &b["breakpoint"];
                        let target = bp["id"]
                            .as_i64()
                            .and_then(|id| self.adapter_breakpoints.get(&(index, id)).cloned());
                        // A source/line-only event cannot be tied to the current
                        // revision. Verification comes from its request ACK.

                        if let Some((path, line)) = target {
                            let removed = b["reason"] == "removed";
                            self.update_breakpoints(
                                path,
                                vec![(
                                    line,
                                    !removed && bp["verified"].as_bool().unwrap_or(false),
                                    bp["message"].as_str().map(str::to_string),
                                )],
                            );
                            if removed && let Some(id) = bp["id"].as_i64() {
                                self.adapter_breakpoints.remove(&(index, id));
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("request") => match v["command"].as_str() {
                Some("startDebugging") => {
                    let endpoint = self
                        .endpoint
                        .clone()
                        .ok_or("Unexpected child debug session")?;
                    if self.connections.len() >= 16 {
                        return Err("Too many nested debug sessions".into());
                    }
                    let mut launch = v["arguments"]["configuration"].clone();
                    launch["request"] = v["arguments"]["request"].clone();
                    let next = self.connections.len();
                    self.active = next;
                    self.thread = 0;
                    self.connections.push(endpoint.connect(launch)?);
                    self.initialize(next)?;
                    self.connections[index].response(&v, json!({}), true)?;
                }
                _ => self.connections[index].response(
                    &v,
                    json!({"error":{"id":1,"format":"Unsupported client request"}}),
                    false,
                )?,
            },
            _ => {}
        }
        Ok(())
    }
    fn update_breakpoints(&mut self, path: PathBuf, updates: Vec<(u32, bool, Option<String>)>) {
        for (line, verified, message) in &updates {
            if let Some(bp) = self
                .session
                .breakpoints
                .iter_mut()
                .find(|x| x.path == path && x.line == *line)
            {
                bp.verified = *verified;
                bp.message = message.clone();
            }
        }
        self.dirty = true;
    }
    fn push_evaluation(&mut self, evaluation: Evaluation) {
        self.session.evaluations.push(evaluation);
        while self.session.evaluations.len() > 100
            || self
                .session
                .evaluations
                .iter()
                .map(|x| {
                    x.expression.len() + x.result.len() + x.error.as_ref().map_or(0, String::len)
                })
                .sum::<usize>()
                > 1024 * 1024
        {
            self.session.evaluations.remove(0);
        }
    }
    fn output(&mut self, category: &str, text: String) {
        self.session.output.push_back(DebugOutput {
            category: limited(category.into(), 64),
            text: limited(text, 65536),
        });
        let mut trimmed = false;
        while self.session.output.len() > 512
            || self
                .session
                .output
                .iter()
                .map(|x| x.text.len() + x.category.len())
                .sum::<usize>()
                > 1024 * 1024
        {
            trimmed = true;
            self.session.output.pop_front();
        }
        if trimmed {
            self.session.last_error = Some(
                "Console display budget reached (1 MiB / 512 entries); older output was removed"
                    .into(),
            );
        }
        self.dirty = true;
    }
    fn fail(&mut self, message: String) {
        self.output("stderr", format!("{message}\n"));
        self.session.last_error = Some(message.clone());
        self.session.state = DebugState::Failed;
        self.dirty = true;
    }
    fn publish(&mut self) {
        self.session.pending = self.pending.len();
        if let Some(secret) = &self.config.browser_inspect_uri {
            secret.redact_session(&mut self.session);
        }
        if self
            .events
            .try_send(WorkerEvent::Session(Box::new(self.session.clone())))
            .is_ok()
        {
            self.dirty = false;
        }
    }
    fn cleanup(&mut self) {
        self.pending.clear();
        if let Some(mut child) = self.adapter.take() {
            terminate_owned_group(&child);
            let _ = child.kill();
            let _ = child.wait();
        }
        for mut child in self.extras.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.connections.clear();
        if let Some(directory) = self.owned_directory.take() {
            let _ = std::fs::remove_dir_all(directory);
        }
        self.dirty = true;
    }
}
fn rust_debug_adapter() -> Result<PathBuf, String> {
    if let Some(resources) = codisseu_runtime::discover()? {
        return resources.tool("rust-debug");
    }
    Ok(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tools/debug/codelldb/extension/adapter/codelldb"))
}
fn string(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}
fn limited(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut i = max;
        while !s.is_char_boundary(i) {
            i -= 1
        }
        s.truncate(i);
        s.push_str("… [truncated]");
    }
    s
}

#[cfg(unix)]
fn terminate_owned_group(child: &Child) {
    // The adapter is spawned in a new process group whose ID equals its PID.
    // External Attach/Listen processes never join this group.
    unsafe extern "C" {
        fn kill(pid: std::os::raw::c_int, signal: std::os::raw::c_int) -> std::os::raw::c_int;
    }
    if let Ok(pid) = i32::try_from(child.id())
        && pid > 1
    {
        // SAFETY: POSIX kill accepts a negative process-group ID; no pointer arguments.
        // Only the isolated group created above is addressed.
        unsafe {
            let _ = kill(-pid, 9);
        }
    }
}
#[cfg(not(unix))]
fn terminate_owned_group(_child: &Child) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unsupported_options_are_excluded_instead_of_becoming_unconditional() {
        let mut w = worker();
        let path = PathBuf::from("fixture.php");
        let mut point = Breakpoint::new(path.clone(), 3);
        point.options.condition = "$i === 3".into();
        w.session.breakpoints.push(point);
        w.breakpoints.insert(path.clone(), vec![3]);
        w.send_breakpoints(0, path.clone(), vec![3]).unwrap();
        assert!(
            w.pending
                .values()
                .any(|p| matches!(&p.kind, Purpose::Breakpoints(_, lines, _) if lines.is_empty()))
        );
        assert!(!w.session.breakpoints[0].verified);
        assert!(
            w.session.breakpoints[0]
                .message
                .as_ref()
                .unwrap()
                .contains("conditional")
        );
        w.exception_capabilities
            .insert(0, json!({"supportsConditionalBreakpoints":true}));
        w.send_breakpoints(0, path, vec![3]).unwrap();
        assert!(
            w.pending
                .values()
                .any(|p| matches!(&p.kind, Purpose::Breakpoints(_, lines, _) if lines == &[3]))
        );
        w.message(0,json!({"type":"response","request_seq":1,"success":true,"body":{"breakpoints":[{"id":1,"verified":true}]}})).unwrap();
        assert!(
            !w.session.breakpoints[0].verified,
            "Old acknowledgment must not certify the new options"
        );
    }
    #[test]
    fn default_php_uses_private_runtime_and_explicit_interpreter_is_preserved() {
        let default = std::path::Path::new("php");
        let private = PathBuf::from("/installation/resources/php");
        assert_eq!(
            resolve_php_executable(default, || Ok(Some(private.clone()))).unwrap(),
            private
        );
        for custom in ["/opt/php/bin/php", "./php", "php8.4"] {
            assert_eq!(
                resolve_php_executable(std::path::Path::new(custom), || panic!(
                    "explicit interpreter must not invoke resource discovery"
                ))
                .unwrap(),
                PathBuf::from(custom)
            );
        }
        assert_eq!(
            resolve_php_executable(default, || Ok(None)).unwrap(),
            default
        );
        assert_eq!(
            resolve_php_executable(default, || Err("broken bundle".into())).unwrap_err(),
            "broken bundle"
        );
    }
    fn worker() -> Worker {
        let config = DebugConfiguration::default();
        let (_, commands) = mpsc::sync_channel(32);
        let (events, _) = mpsc::sync_channel(8);
        let (log_tx, logs) = mpsc::sync_channel(64);
        let (_, rx) = mpsc::sync_channel(1);
        Worker {
            epoch: 0,
            started: Instant::now(),
            shutdown: Arc::new(AtomicBool::new(false)),
            session: DebugSession::new(1, &config),
            config,
            breakpoints: BTreeMap::new(),
            breakpoint_generations: BTreeMap::new(),
            adapter_breakpoints: BTreeMap::new(),
            exception_capabilities: BTreeMap::new(),
            exception_generations: BTreeMap::new(),
            connections: vec![Connection {
                writer: Box::new(std::io::sink()),
                rx,
                seq: 0,
                initialized: true,
                configure_sent: true,
                launch: json!({}),
            }],
            pending: BTreeMap::new(),
            events,
            commands,
            adapter: None,
            extras: vec![],
            logs,
            log_tx,
            endpoint: None,
            owned_directory: None,
            active: 0,
            thread: 0,
            dirty: false,
            stop_deadline: None,
        }
    }
    #[test]
    fn queued_controls_and_late_stops_cannot_resume_a_terminating_session() {
        let mut w = worker();
        w.session.state = DebugState::Stopped;
        w.session.current_frame = Some(7);
        w.command(DebugCommand::Stop(1)).unwrap();
        let disconnects = w.pending.len();
        w.command(DebugCommand::Continue(1)).unwrap();
        w.message(
            0,
            json!({"type":"event","event":"stopped","body":{"threadId":1,"reason":"breakpoint"}}),
        )
        .unwrap();
        w.message(
            0,
            json!({"type":"event","event":"continued","body":{"threadId":1}}),
        )
        .unwrap();
        assert_eq!(w.session.state, DebugState::Terminating);
        assert!(w.session.current_frame.is_none());
        assert_eq!(w.pending.len(), disconnects);
    }

    #[test]
    fn nested_node_exit_preserves_stop_until_parent_terminates() {
        let mut w = worker();
        w.config.runtime = DebugRuntime::Node;
        w.config.auto_attach_child_processes = true;
        w.active = 1;
        w.session.state = DebugState::Terminating;
        w.message(1, json!({"type":"event","event":"terminated","body":{}}))
            .unwrap();
        assert_eq!(w.session.state, DebugState::Terminating);
        w.session.state = DebugState::Stopped;
        w.message(1, json!({"type":"event","event":"terminated","body":{}}))
            .unwrap();
        assert_eq!(w.session.state, DebugState::Running);
        assert_eq!(w.active, 0);
        w.message(0, json!({"type":"event","event":"terminated","body":{}}))
            .unwrap();
        assert_eq!(w.session.state, DebugState::Terminated);
    }
    #[test]
    fn default_node_target_exit_ends_session_instead_of_waiting_for_more_children() {
        let mut w = worker();
        w.config.runtime = DebugRuntime::Node;
        assert!(!w.config.auto_attach_child_processes);
        w.active = 1;
        w.session.state = DebugState::Stopped;
        w.message(1, json!({"type":"event","event":"terminated","body":{}}))
            .unwrap();
        assert_eq!(w.session.state, DebugState::Terminated);
    }
    #[test]
    fn continued_clears_frame_handles_and_rejects_late_variables() {
        let mut w = worker();
        w.session.state = DebugState::Stopped;
        w.session.current_frame = Some(7);
        w.session.current_thread = Some(0);
        w.session.variables.insert(9, vec![]);
        w.pending.insert(
            (0, 1),
            Pending {
                kind: Purpose::Variables(9),
                sent: Instant::now(),
                epoch: 0,
            },
        );
        w.message(
            0,
            json!({"type":"event","event":"continued","body":{"threadId":0}}),
        )
        .unwrap();
        assert_eq!(w.session.state, DebugState::Running);
        assert!(
            w.session.current_frame.is_none()
                && w.session.current_thread.is_none()
                && w.session.variables.is_empty()
        );
        w.message(0,json!({"type":"response","request_seq":1,"success":true,"body":{"variables":[{"name":"old","value":"stale","variablesReference":0}]}})).unwrap();
        assert!(w.session.variables.is_empty());
    }
    #[test]
    fn zero_thread_id_remains_selected_when_not_first() {
        let mut w = worker();
        w.session.state = DebugState::Stopped;
        w.pending.insert(
            (0, 1),
            Pending {
                kind: Purpose::Threads,
                sent: Instant::now(),
                epoch: 0,
            },
        );
        w.message(0,json!({"type":"response","request_seq":1,"success":true,"body":{"threads":[{"id":3,"name":"other"},{"id":0,"name":"paused"}]}})).unwrap();
        assert_eq!(w.thread, 0);
        assert_eq!(w.session.current_thread, Some(0));
    }
    #[test]
    fn relocated_and_id_only_breakpoint_events_keep_original_marker_identity() {
        let mut w = worker();
        let path = PathBuf::from("example.ts");
        w.breakpoints.insert(path.clone(), vec![10]);
        w.session.breakpoints.push(Breakpoint {
            path: path.clone(),
            line: 10,
            enabled: true,
            verified: false,
            message: None,
            options: BreakpointOptions::default(),
            revision: 0,
        });
        w.breakpoint_generations.insert((0, path.clone()), 1);
        w.pending.insert(
            (0, 1),
            Pending {
                kind: Purpose::Breakpoints(path.clone(), vec![10], 1),
                sent: Instant::now(),
                epoch: 0,
            },
        );
        w.message(0,json!({"type":"response","request_seq":1,"success":true,"body":{"breakpoints":[{"id":77,"line":12,"verified":false}]}})).unwrap();
        w.message(0,json!({"type":"event","event":"breakpoint","body":{"reason":"changed","breakpoint":{"id":77,"verified":true}}})).unwrap();
        assert!(w.session.breakpoints[0].verified);
        assert_eq!(w.session.breakpoints[0].line, 10);
        w.message(0,json!({"type":"event","event":"breakpoint","body":{"reason":"removed","breakpoint":{"id":77}}})).unwrap();
        assert!(!w.session.breakpoints[0].verified);
    }
    #[test]
    fn integrated_breakpoint_verification_requires_actual_hit_id_on_same_connection() {
        for integrated in [false, true] {
            let mut w = worker();
            w.config.integrated_browser = integrated;
            let path = PathBuf::from("app.js");
            for line in [5, 6, 7] {
                let mut point = Breakpoint::new(path.clone(), line);
                point.message = Some("breakpoint.provisionalBreakpoint".into());
                point.enabled = line != 7;
                w.session.breakpoints.push(point);
            }
            w.adapter_breakpoints.insert((0, 77), (path.clone(), 5));
            w.adapter_breakpoints.insert((1, 77), (path.clone(), 6));
            w.adapter_breakpoints.insert((0, 99), (path, 7));
            w.message(0, json!({"type":"event","event":"stopped","body":{"threadId":1,"reason":"breakpoint","hitBreakpointIds":[77,99,12345]}})).unwrap();
            assert_eq!(w.session.breakpoints[0].verified, integrated);
            assert_eq!(w.session.breakpoints[0].message.is_none(), integrated);
            assert!(!w.session.breakpoints[1].verified);
            assert!(!w.session.breakpoints[2].verified);
        }
        let mut w = worker();
        w.config.integrated_browser = true;
        w.session
            .breakpoints
            .push(Breakpoint::new("app.js".into(), 5));
        w.adapter_breakpoints.insert((0, 77), ("app.js".into(), 5));
        w.message(
            0,
            json!({"type":"event","event":"stopped","body":{"threadId":1,"reason":"breakpoint"}}),
        )
        .unwrap();
        assert!(!w.session.breakpoints[0].verified);
    }
    #[test]
    fn stale_breakpoint_and_exception_acks_do_not_restore_old_state() {
        let mut w = worker();
        let path = PathBuf::from("example.js");
        w.breakpoints.insert(path.clone(), vec![10]);
        w.session.breakpoints.push(Breakpoint {
            path: path.clone(),
            line: 10,
            enabled: true,
            verified: false,
            message: None,
            options: BreakpointOptions::default(),
            revision: 0,
        });
        w.breakpoint_generations.insert((0, path.clone()), 3);
        w.pending.insert(
            (0, 1),
            Pending {
                kind: Purpose::Breakpoints(path, vec![10], 1),
                sent: Instant::now(),
                epoch: 0,
            },
        );
        w.message(0,json!({"type":"response","request_seq":1,"success":true,"body":{"breakpoints":[{"id":1,"verified":true}]}})).unwrap();
        assert!(!w.session.breakpoints[0].verified);
        w.exception_capabilities.insert(
            0,
            json!({"exceptionBreakpointFilters":[{"filter":"caught"}]}),
        );
        w.config.exception_filters = vec!["caught".into()];
        w.send_exception_filters(0, vec!["caught".into()]).unwrap();
        w.send_exception_filters(0, vec![]).unwrap();
        w.send_exception_filters(0, vec!["caught".into()]).unwrap();
        w.message(0, json!({"type":"response","request_seq":1,"success":true}))
            .unwrap();
        assert!(w.session.exception_filters.is_empty());
        w.message(0, json!({"type":"response","request_seq":3,"success":true}))
            .unwrap();
        assert_eq!(w.session.exception_filters, ["caught"]);
        assert!(
            w.send_exception_filters(0, vec!["unsupported".into()])
                .is_err()
        );
    }
}
