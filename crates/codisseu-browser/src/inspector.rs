//! Native inspector state and actions. No executable CDP JSON reaches the GUI.
use std::{collections::BTreeMap, sync::Arc};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lifecycle {
    #[default]
    Stopped,
    Starting,
    Ready,
    Failed,
}
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub tab: u64,
    pub epoch: u64,
    pub engine_pid: u32,
    pub url: String,
    pub origin: String,
}
#[derive(Clone, Default, PartialEq)]
pub struct Snapshot {
    pub lifecycle: Lifecycle,
    pub session: Option<Session>,
    pub revision: u64,
    pub document_epoch: u64,
    pub busy: bool,
    pub error: Option<String>,
    pub errors: Vec<String>,
    pub dom: Vec<DomNode>,
    pub dom_truncated: bool,
    pub selected_node: Option<i64>,
    pub attributes: Vec<(String, String)>,
    pub styles: Vec<(String, String)>,
    pub properties: Vec<Property>,
    pub property_target: Option<String>,
    pub network: Vec<NetworkRequest>,
    pub bodies: BTreeMap<String, Body>,
    pub preserve_network: bool,
    pub network_enabled: bool,
    pub cookies: Vec<Cookie>,
    pub local_storage: Vec<(String, String)>,
    pub session_storage: Vec<(String, String)>,
    pub evaluation: Option<Evaluation>,
    pub watches: Vec<Evaluation>,
    pub paused: Option<Pause>,
    pub scope_values: BTreeMap<String, Vec<Property>>,
    pub breakpoints: Vec<Breakpoint>,
}
impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectorSnapshot")
            .field("lifecycle", &self.lifecycle)
            .field("revision", &self.revision)
            .field("dom_nodes", &self.dom.len())
            .field("requests", &self.network.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Default, PartialEq)]
pub struct DomNode {
    pub id: i64,
    pub parent: Option<i64>,
    pub depth: usize,
    pub node_type: u64,
    pub name: String,
    pub local_name: String,
    pub value: String,
    pub attributes: Vec<(String, String)>,
    pub child_count: usize,
    pub shadow_root: bool,
    pub custom_element: bool,
}
#[derive(Clone, Default, PartialEq)]
pub struct RemoteValue {
    pub kind: String,
    pub subtype: Option<String>,
    pub description: String,
    pub object_id: Option<String>,
}
#[derive(Clone, Default, PartialEq)]
pub struct Property {
    pub name: String,
    pub value: RemoteValue,
    pub writable: bool,
    pub enumerable: bool,
    pub getter: bool,
    pub setter: bool,
}
#[derive(Clone, Default, PartialEq)]
pub struct NetworkRequest {
    pub id: String,
    pub url: String,
    pub method: String,
    pub kind: String,
    pub document_url: String,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    pub post_data: Option<String>,
    pub status: Option<f64>,
    pub status_text: String,
    pub mime_type: String,
    pub protocol: String,
    pub timestamp: f64,
    pub duration_ms: Option<f64>,
    pub encoded_bytes: f64,
    pub decoded_bytes: Option<f64>,
    pub finished: bool,
    pub failed: Option<String>,
    pub from_cache: bool,
    pub initiator: String,
}
#[derive(Clone, Default, PartialEq)]
pub struct Body {
    pub request_id: String,
    pub text: String,
    pub base64: bool,
    pub truncated: bool,
    pub error: Option<String>,
}
#[derive(Clone, Default, PartialEq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<String>,
    pub expires: f64,
    pub session: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageKind {
    Local,
    Session,
}
#[derive(Clone, Default, PartialEq)]
pub struct Evaluation {
    pub expression: String,
    pub value: Option<RemoteValue>,
    pub error: Option<String>,
}
#[derive(Clone, Default, PartialEq)]
pub struct Scope {
    pub name: String,
    pub kind: String,
    pub object_id: String,
}
#[derive(Clone, Default, PartialEq)]
pub struct CallFrame {
    pub id: String,
    pub function_name: String,
    pub url: String,
    pub line: u32,
    pub column: u32,
    pub scopes: Vec<Scope>,
}
#[derive(Clone, Default, PartialEq)]
pub struct Pause {
    pub reason: String,
    pub frames: Vec<CallFrame>,
    pub selected: Option<String>,
}
#[derive(Clone, Default, PartialEq)]
pub struct Breakpoint {
    pub id: String,
    pub url: String,
    pub line: u32,
    pub condition: String,
}
#[derive(Clone)]
pub enum Action {
    RefreshDom,
    SelectNode {
        node_id: i64,
    },
    GetProperties {
        object_id: String,
    },
    Network {
        enabled: bool,
    },
    PreserveNetwork(bool),
    ClearNetwork,
    GetResponseBody {
        request_id: String,
    },
    Evaluate {
        expression: String,
    },
    SetWatches(Vec<String>),
    RefreshWatches,
    Pause,
    Resume,
    StepOver,
    StepInto,
    StepOut,
    SelectFrame {
        frame_id: String,
    },
    SetBreakpoint {
        url: String,
        line: u32,
        condition: String,
    },
    RemoveBreakpoint {
        breakpoint_id: String,
    },
    RefreshStorage,
    SetStorage {
        kind: StorageKind,
        key: String,
        value: String,
    },
    DeleteStorage {
        kind: StorageKind,
        key: String,
    },
    SetCookie {
        cookie: Cookie,
    },
    DeleteCookie {
        name: String,
        domain: String,
        path: String,
    },
}
impl std::fmt::Debug for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InspectorAction([REDACTED])")
    }
}
pub type Waker = Arc<dyn Fn() + Send + Sync>;
#[path = "inspector_dom.rs"]
mod dom;
#[path = "inspector_model.rs"]
mod model;
#[path = "inspector_service.rs"]
mod service;
#[path = "inspector_worker.rs"]
mod worker;
pub use service::InspectorService;
