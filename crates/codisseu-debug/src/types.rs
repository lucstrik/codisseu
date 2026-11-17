use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
};
pub type SessionId = u64;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DebugRuntime {
    Php,
    #[default]
    Node,
    Chrome,
    Rust,
    Python,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DebugRequest {
    #[default]
    Launch,
    Attach,
    Listen,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DebugState {
    Starting,
    Running,
    Stopped,
    Terminating,
    Terminated,
    Failed,
}
impl DebugState {
    pub fn is_active(self) -> bool {
        !matches!(self, Self::Terminated | Self::Failed)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugConfiguration {
    pub id: String,
    pub name: String,
    pub runtime: DebugRuntime,
    pub request: DebugRequest,
    pub executable: PathBuf,
    pub program: Option<PathBuf>,
    /// Python module target (`python -m`), mutually exclusive with program.
    pub module: Option<String>,
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub port: u16,
    pub listen_host: String,
    pub url: String,
    /// An integrated target must be explicitly selected again after restoration.
    pub integrated_browser: bool,
    #[serde(skip)]
    pub browser_tab: Option<u64>,
    /// Session-only secret; custom Debug never includes the nonce.
    #[serde(skip)]
    pub browser_inspect_uri: Option<crate::BrowserInspectUri>,
    pub path_mappings: BTreeMap<String, String>,
    pub source_maps: bool,
    /// Opt-in child processes for runners such as Vitest.
    pub auto_attach_child_processes: bool,
    pub stop_on_entry: bool,
    pub exception_filters: Vec<String>,
    pub adapter: Option<PathBuf>,
    pub adapter_runtime: Option<PathBuf>,
    pub trusted: bool,
}
impl Default for DebugConfiguration {
    fn default() -> Self {
        Self {
            id: "default".into(),
            name: "Debug".into(),
            runtime: DebugRuntime::Node,
            request: DebugRequest::Launch,
            executable: "node".into(),
            program: None,
            module: None,
            cwd: PathBuf::new(),
            args: vec![],
            env: BTreeMap::new(),
            port: 0,
            listen_host: "127.0.0.1".into(),
            url: "http://localhost:3000".into(),
            integrated_browser: false,
            browser_tab: None,
            browser_inspect_uri: None,
            path_mappings: BTreeMap::new(),
            source_maps: true,
            auto_attach_child_processes: false,
            stop_on_entry: false,
            exception_filters: vec![],
            adapter: None,
            adapter_runtime: None,
            trusted: false,
        }
    }
}
impl DebugConfiguration {
    pub fn for_file(path: impl AsRef<Path>, cwd: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let php = path.extension().is_some_and(|x| x == "php");
        let rust = path.extension().is_some_and(|x| x == "rs");
        let python = path
            .extension()
            .is_some_and(|x| matches!(x.to_str(), Some("py" | "pyw" | "pyi")));
        let python_stub = path.extension().is_some_and(|x| x == "pyi");
        Self {
            id: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            runtime: if php {
                DebugRuntime::Php
            } else if rust {
                DebugRuntime::Rust
            } else if python {
                DebugRuntime::Python
            } else {
                DebugRuntime::Node
            },
            executable: if php {
                "php"
            } else if rust {
                ""
            } else if python {
                "python3"
            } else {
                "node"
            }
            .into(),
            // Rust sources require an explicit Cargo build and binary selection.
            program: (!rust && !python_stub).then(|| path.to_path_buf()),
            cwd: cwd.as_ref().to_path_buf(),
            ..Self::default()
        }
    }
}
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum DebugCommand {
    Start(DebugConfiguration),
    Stop(SessionId),
    Continue(SessionId),
    Next(SessionId),
    StepIn(SessionId),
    StepOut(SessionId),
    Pause(SessionId),
    Threads(SessionId),
    SelectThread {
        session: SessionId,
        thread_id: i64,
    },
    SelectFrame {
        session: SessionId,
        frame_id: i64,
    },
    Variables {
        session: SessionId,
        reference: i64,
    },
    Evaluate {
        session: SessionId,
        expression: String,
    },
    SetBreakpointsEnabled(bool),
    SetExceptionBreakpoints {
        session: SessionId,
        filters: Vec<String>,
    },
    ToggleBreakpoint {
        path: PathBuf,
        line: u32,
    },
    SetBreakpoints {
        path: PathBuf,
        lines: Vec<u32>,
    },
    SetBreakpoint(Breakpoint),
    SetSourceBreakpoints {
        path: PathBuf,
        breakpoints: Vec<Breakpoint>,
    },
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BreakpointOptions {
    pub condition: String,
    pub hit_condition: String,
    pub log_message: String,
}
impl BreakpointOptions {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.condition, &self.hit_condition, &self.log_message]
            .iter()
            .any(|s| s.len() > 16384 || s.contains('\0'))
        {
            return Err("Breakpoint expressions must be at most 16 KiB, without NUL".into());
        }
        Ok(())
    }
    pub fn unsupported(&self, capabilities: &serde_json::Value) -> Option<&'static str> {
        [
            (
                !self.condition.trim().is_empty(),
                "supportsConditionalBreakpoints",
                "Adapter does not support conditional breakpoints",
            ),
            (
                !self.hit_condition.trim().is_empty(),
                "supportsHitConditionalBreakpoints",
                "Adapter does not support hit conditions",
            ),
            (
                !self.log_message.trim().is_empty(),
                "supportsLogPoints",
                "Adapter does not support logpoints",
            ),
        ]
        .into_iter()
        .find_map(|(used, key, message)| (used && capabilities[key] != true).then_some(message))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Breakpoint {
    pub path: PathBuf,
    pub line: u32,
    pub enabled: bool,
    #[serde(skip)]
    pub verified: bool,
    #[serde(skip)]
    pub message: Option<String>,
    #[serde(default)]
    pub options: BreakpointOptions,
    #[serde(skip)]
    pub revision: u64,
}
impl Breakpoint {
    pub fn new(path: PathBuf, line: u32) -> Self {
        Self {
            path,
            line,
            enabled: true,
            verified: false,
            message: None,
            options: BreakpointOptions::default(),
            revision: 0,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugThread {
    pub id: i64,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub path: Option<PathBuf>,
    pub line: u32,
    pub column: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scope {
    pub name: String,
    pub reference: i64,
    pub expensive: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    pub reference: i64,
    pub evaluate_name: Option<String>,
}
#[derive(Debug, Clone)]
pub struct DebugOutput {
    pub category: String,
    pub text: String,
}
#[derive(Debug, Clone)]
pub struct Evaluation {
    pub expression: String,
    pub result: String,
    pub reference: i64,
    pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct DebugSession {
    pub id: SessionId,
    pub name: String,
    pub runtime: DebugRuntime,
    pub state: DebugState,
    pub threads: Vec<DebugThread>,
    pub frames: Vec<StackFrame>,
    pub scopes: Vec<Scope>,
    pub variables: BTreeMap<i64, Vec<Variable>>,
    pub output: VecDeque<DebugOutput>,
    pub last_error: Option<String>,
    pub current_frame: Option<i64>,
    pub current_thread: Option<i64>,
    pub capabilities: serde_json::Value,
    pub evaluations: Vec<Evaluation>,
    pub stopped_reason: Option<String>,
    pub pending: usize,
    pub exit_code: Option<i64>,
    pub stop_epoch: u64,
    pub breakpoints: Vec<Breakpoint>,
    pub exception_filters: Vec<String>,
}
impl DebugSession {
    pub(crate) fn new(id: SessionId, c: &DebugConfiguration) -> Self {
        Self {
            id,
            name: c.name.clone(),
            runtime: c.runtime,
            state: DebugState::Starting,
            threads: vec![],
            frames: vec![],
            scopes: vec![],
            variables: BTreeMap::new(),
            output: VecDeque::new(),
            last_error: None,
            current_frame: None,
            current_thread: None,
            capabilities: serde_json::json!({}),
            evaluations: vec![],
            stopped_reason: None,
            pending: 0,
            exit_code: None,
            stop_epoch: 0,
            breakpoints: vec![],
            exception_filters: vec![],
        }
    }
}
