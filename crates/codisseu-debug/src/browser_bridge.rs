//! A session-only, single-page CDP transport gateway for the unmodified js-debug adapter.
//! Target sessions here are explicit virtual routing IDs, never invented runtime results.
use codisseu_browser::{DevToolsChannel, DevToolsEvent};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fmt,
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tungstenite::{Message, WebSocket, protocol::WebSocketConfig};

const MAX_CDP: usize = 4 * 1024 * 1024;
const MAX_HEADER: usize = 16 * 1024;
const INFO_ID: u64 = 2_147_483_000;

#[derive(Clone)]
pub struct BrowserInspectUri(String);
impl fmt::Debug for BrowserInspectUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BrowserInspectUri([redacted])")
    }
}
impl BrowserInspectUri {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}
impl BrowserInspectUri {
    pub(crate) fn redact(&self, text: &str) -> String {
        let text = text.replace(&self.0, "[browser debug endpoint redacted]");
        let token = self.0.rsplit('/').next().unwrap_or("");
        if token.len() == 64 {
            text.replace(token, "[browser debug token redacted]")
        } else {
            text
        }
    }
    pub(crate) fn redact_session(&self, session: &mut crate::DebugSession) {
        fn optional(uri: &BrowserInspectUri, value: &mut Option<String>) {
            if let Some(text) = value {
                *text = uri.redact(text);
            }
        }
        fn path(uri: &BrowserInspectUri, value: &mut Option<std::path::PathBuf>) {
            if let Some(p) = value {
                let text = p.to_string_lossy();
                let clean = uri.redact(&text);
                if clean != text {
                    *p = clean.into();
                }
            }
        }
        fn json_strings(uri: &BrowserInspectUri, value: &mut Value) {
            match value {
                Value::String(text) => *text = uri.redact(text),
                Value::Array(a) => {
                    for v in a {
                        json_strings(uri, v);
                    }
                }
                Value::Object(m) => {
                    for v in m.values_mut() {
                        json_strings(uri, v);
                    }
                }
                _ => {}
            }
        }
        session.name = self.redact(&session.name);
        optional(self, &mut session.last_error);
        optional(self, &mut session.stopped_reason);
        for thread in &mut session.threads {
            thread.name = self.redact(&thread.name);
        }
        for frame in &mut session.frames {
            frame.name = self.redact(&frame.name);
            path(self, &mut frame.path);
        }
        for scope in &mut session.scopes {
            scope.name = self.redact(&scope.name);
        }
        for variable in session.variables.values_mut().flatten() {
            variable.name = self.redact(&variable.name);
            variable.value = self.redact(&variable.value);
            optional(self, &mut variable.type_name);
            optional(self, &mut variable.evaluate_name);
        }
        for output in &mut session.output {
            output.category = self.redact(&output.category);
            output.text = self.redact(&output.text);
        }
        for evaluation in &mut session.evaluations {
            evaluation.expression = self.redact(&evaluation.expression);
            evaluation.result = self.redact(&evaluation.result);
            optional(self, &mut evaluation.error);
        }
        for breakpoint in &mut session.breakpoints {
            optional(self, &mut breakpoint.message);
        }
        for filter in &mut session.exception_filters {
            *filter = self.redact(filter);
        }
        json_strings(self, &mut session.capabilities);
    }
}

#[derive(Clone, Debug)]
pub struct BrowserDebugEndpoint {
    pub port: u16,
    pub inspect_uri: BrowserInspectUri,
}

pub struct BrowserDebugBridge {
    endpoint: BrowserDebugEndpoint,
    mode: GatewayMode,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
    activity: Arc<Activity>,
}
#[derive(Default)]
struct Activity {
    connected: AtomicBool,
    paused: AtomicU64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GatewayMode {
    Adapter,
    Page,
}
impl fmt::Debug for BrowserDebugBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowserDebugBridge")
            .field("endpoint", &self.endpoint)
            .field("finished", &self.is_finished())
            .finish()
    }
}
trait Peer: Send {
    fn send(&self, json: String) -> Result<(), String>;
    fn receive(&self, timeout: Duration) -> Result<Option<String>, String>;
}
impl Peer for DevToolsChannel {
    fn send(&self, json: String) -> Result<(), String> {
        self.sender.send(json)
    }
    fn receive(&self, timeout: Duration) -> Result<Option<String>, String> {
        match self.events.recv_timeout(timeout) {
            Ok(DevToolsEvent::Message { json }) => Ok(Some(json)),
            Ok(DevToolsEvent::Closed { message }) => {
                Err(message.unwrap_or_else(|| "Browser debug page closed".into()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(_) => Err("Browser debug channel closed".into()),
        }
    }
}
impl BrowserDebugBridge {
    /// May wait up to two seconds for real Target.getTargetInfo. Call off the GUI thread.
    /// The caller must acquire this channel only after explicit Debug authorization.
    pub fn start(channel: DevToolsChannel, tab: u64, epoch: u64) -> Result<Self, String> {
        Self::start_peer(Box::new(channel), tab, epoch)
    }
    /// Original Chromium DevTools frontend; direct CDP for this page, without routing sessions.
    pub fn start_page(channel: DevToolsChannel, tab: u64, epoch: u64) -> Result<Self, String> {
        Self::start_peer_mode(Box::new(channel), tab, epoch, GatewayMode::Page)
    }
    fn start_peer(peer: Box<dyn Peer>, tab: u64, epoch: u64) -> Result<Self, String> {
        Self::start_peer_mode(peer, tab, epoch, GatewayMode::Adapter)
    }
    fn start_peer_mode(
        peer: Box<dyn Peer>,
        tab: u64,
        epoch: u64,
        mode: GatewayMode,
    ) -> Result<Self, String> {
        if tab == 0 || epoch == 0 {
            return Err("Browser debug requires a live tab and epoch".into());
        }
        peer.send(json!({"id":INFO_ID,"method":"Target.getTargetInfo","params":{}}).to_string())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let info = loop {
            if Instant::now() >= deadline {
                return Err("Browser did not identify the selected debug target".into());
            }
            if let Some(raw) = peer.receive(Duration::from_millis(20))? {
                let value = parse(&raw)?;
                if value["id"] == INFO_ID {
                    if value.get("error").is_some() {
                        return Err("Browser page does not support Target.getTargetInfo".into());
                    }
                    let info = value["result"]["targetInfo"].clone();
                    if info["type"] != "page"
                        || info["targetId"]
                            .as_str()
                            .is_none_or(|s| s.is_empty() || s.len() > 512)
                        || info["url"].as_str().is_none_or(|s| s.len() > 16384)
                        || info["title"].as_str().is_some_and(|s| s.len() > 4096)
                    {
                        return Err("Browser returned invalid page target identity".into());
                    }
                    break info;
                }
            }
        };
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| "Unable to bind private browser debug transport")?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| "Unable to create browser debug transport secret")?;
        let token = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let endpoint = BrowserDebugEndpoint {
            port: address.port(),
            inspect_uri: BrowserInspectUri(format!("ws://{address}/{token}")),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let activity = Arc::new(Activity::default());
        let worker_activity = activity.clone();
        let (worker_stop, worker_done, worker_error) = (stop.clone(), done.clone(), error.clone());
        let worker = thread::Builder::new()
            .name("codisseu-browser-debug-bridge".into())
            .spawn(move || {
                let mut router = Router::new_mode(info, epoch, mode);
                if let Err(e) = run(
                    listener,
                    address,
                    token,
                    &*peer,
                    &mut router,
                    &worker_stop,
                    &worker_activity,
                ) && !worker_stop.load(Ordering::Acquire)
                    && let Ok(mut error) = worker_error.lock()
                {
                    *error = Some(e);
                }
                // Best effort; engine-side unsubscribe also releases the agent/pause.
                let _ = peer.send(
                    json!({"id":INFO_ID+1,"method":"Debugger.resume","params":{}}).to_string(),
                );
                let _ = peer.send(
                    json!({"id":INFO_ID+2,"method":"Debugger.disable","params":{}}).to_string(),
                );
                drop(peer);
                worker_activity.connected.store(false, Ordering::Release);
                worker_done.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            endpoint,
            mode,
            stop,
            done,
            error,
            worker: Some(worker),
            activity,
        })
    }
    pub fn endpoint(&self) -> BrowserDebugEndpoint {
        self.endpoint.clone()
    }
    pub fn frontend_url(&self) -> Result<codisseu_browser::SecretUrl, String> {
        if self.mode != GatewayMode::Page {
            return Err("The debugger adapter bridge is not a DevTools frontend channel".into());
        }
        let address = self
            .endpoint
            .inspect_uri
            .as_str()
            .strip_prefix("ws://")
            .ok_or("Invalid private DevTools endpoint")?;
        codisseu_browser::SecretUrl::devtools(format!(
            "devtools://devtools/bundled/devtools_app.html?can_dock=false&ws={address}"
        ))
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
    pub fn is_connected(&self) -> bool {
        self.activity.connected.load(Ordering::Acquire)
    }
    /// Count of actual Debugger.paused messages, without payloads or secrets.
    pub fn paused_events(&self) -> u64 {
        self.activity.paused.load(Ordering::Acquire)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }
    /// Optional bounded acknowledgement for teardown tests/workers; never needed in the GUI.
    pub fn shutdown(&mut self, timeout: Duration) -> bool {
        self.stop();
        let deadline = Instant::now() + timeout.min(Duration::from_secs(2));
        while !self.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        if self.is_finished() {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            true
        } else {
            false
        }
    }
}
impl Drop for BrowserDebugBridge {
    fn drop(&mut self) {
        self.stop(); /* Detached worker polls stop; no GUI join. */
    }
}

fn parse(raw: &str) -> Result<Value, String> {
    if raw.is_empty() || raw.len() > MAX_CDP {
        return Err("Browser debug message exceeds4MiB".into());
    }
    let value: Value = serde_json::from_str(raw).map_err(|_| "Invalid browser CDP JSON")?;
    if !value.is_object() {
        return Err("Browser CDP message must be an object".into());
    }
    Ok(value)
}
fn failure(value: &Value, code: i32, message: &str) -> Value {
    let mut reply = json!({"id":value["id"],"error":{"code":code,"message":message}});
    if let Some(session) = value.get("sessionId") {
        reply["sessionId"] = session.clone();
    }
    reply
}
enum Routed {
    Local(Vec<Value>),
    Forward(Value),
}
struct Router {
    info: Value,
    browser: String,
    page: String,
    children: BTreeSet<String>,
    pending: BTreeSet<u64>,
    mode: GatewayMode,
}
impl Router {
    #[cfg(test)]
    fn new(info: Value, epoch: u64) -> Self {
        Self::new_mode(info, epoch, GatewayMode::Adapter)
    }
    fn new_mode(info: Value, epoch: u64, mode: GatewayMode) -> Self {
        let id = info["targetId"].as_str().unwrap_or_default();
        let browser = format!("codisseu-routing-browser:{id}:{epoch}");
        let page = format!("codisseu-routing-page:{id}:{epoch}");
        Self {
            info,
            browser,
            page,
            children: BTreeSet::new(),
            pending: BTreeSet::new(),
            mode,
        }
    }
    fn request(&mut self, mut value: Value) -> Result<Routed, String> {
        let id = value["id"]
            .as_u64()
            .filter(|id| *id > 0 && *id < INFO_ID)
            .ok_or("Invalid browser CDP command id")?;
        let method = value["method"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 128)
            .ok_or("Invalid browser CDP method")?
            .to_owned();
        let session = value["sessionId"].as_str().unwrap_or("").to_owned();
        if self.mode == GatewayMode::Page {
            if !session.is_empty() && !self.children.contains(&session) {
                return Ok(Routed::Local(vec![failure(
                    &value,
                    -32602,
                    "Unknown page child session",
                )]));
            }
            let outside = (method.starts_with("Browser.") && method != "Browser.getVersion")
                || (method.starts_with("Target.")
                    && !matches!(
                        method.as_str(),
                        "Target.getTargetInfo" | "Target.setAutoAttach" | "Target.detachFromTarget"
                    ))
                || (method == "Target.getTargetInfo"
                    && value["params"]
                        .get("targetId")
                        .is_some_and(|id| *id != self.info["targetId"]))
                || (method == "Target.detachFromTarget"
                    && value["params"]["sessionId"]
                        .as_str()
                        .is_none_or(|id| !self.children.contains(id)));
            if outside {
                return Ok(Routed::Local(vec![failure(
                    &value,
                    -32601,
                    "Command is outside the inspected page",
                )]));
            }
            if method == "Page.navigate"
                && value["params"]["url"]
                    .as_str()
                    .is_none_or(|url| codisseu_browser::validate_url(url).is_err())
            {
                return Ok(Routed::Local(vec![failure(
                    &value,
                    -32602,
                    "Unsupported page navigation URL",
                )]));
            }
            if self.pending.len() >= 256 || !self.pending.insert(id) {
                return Err("Too many or duplicate browser debug requests".into());
            }
            return Ok(Routed::Forward(value));
        }
        if session.is_empty() || session == self.browser {
            let body = match method.as_str() {
                "Target.attachToBrowserTarget" => Some(json!({"sessionId":self.browser})),
                "Target.setDiscoverTargets" => Some(json!({})),
                "Target.getTargets" => Some(json!({"targetInfos":[self.info]})),
                "Target.attachToTarget" if value["params"]["targetId"] == self.info["targetId"] => {
                    Some(json!({"sessionId":self.page}))
                }
                "Target.detachFromTarget"
                    if value["params"]["sessionId"] == self.page
                        || value["params"]["sessionId"] == self.browser =>
                {
                    Some(json!({}))
                }
                _ => None,
            };
            let Some(body) = body else {
                return Ok(Routed::Local(vec![failure(
                    &value,
                    -32601,
                    "Unsupported single-page routing command",
                )]));
            };
            let mut response = json!({"id":id,"result":body});
            if !session.is_empty() {
                response["sessionId"] = json!(session);
            }
            let mut replies = vec![response];
            if method == "Target.setDiscoverTargets" && value["params"]["discover"] == true {
                replies.push(json!({"method":"Target.targetCreated","sessionId":self.browser,"params":{"targetInfo":self.info}}));
            }
            return Ok(Routed::Local(replies));
        }
        if session != self.page && !self.children.contains(&session) {
            return Ok(Routed::Local(vec![failure(
                &value,
                -32602,
                "Unknown page routing session",
            )]));
        }
        let supported = [
            "Runtime.",
            "Debugger.",
            "Profiler.",
            "Page.",
            "Network.",
            "DOM.",
            "DOMDebugger.",
            "ServiceWorker.",
        ]
        .iter()
        .any(|p| method.starts_with(p))
            || method == "Browser.getVersion"
            || method == "Target.setAutoAttach"
            || method == "Target.getTargetInfo";
        if !supported
            || (method == "Target.getTargetInfo"
                && value["params"]
                    .get("targetId")
                    .is_some_and(|id| *id != self.info["targetId"]))
        {
            return Ok(Routed::Local(vec![failure(
                &value,
                -32601,
                "Command is outside the selected browser page",
            )]));
        }
        if method == "Page.navigate"
            && value["params"]["url"]
                .as_str()
                .is_none_or(|url| codisseu_browser::validate_url(url).is_err())
        {
            return Ok(Routed::Local(vec![failure(
                &value,
                -32602,
                "Unsupported browser navigation URL",
            )]));
        }
        if self.pending.len() >= 256 || !self.pending.insert(id) {
            return Err("Too many or duplicate browser debug requests".into());
        }
        if session == self.page {
            value.as_object_mut().unwrap().remove("sessionId");
        }
        Ok(Routed::Forward(value))
    }
    fn event(&mut self, mut value: Value) -> Result<Option<Value>, String> {
        if let Some(id) = value["id"].as_u64()
            && !self.pending.remove(&id)
        {
            return Ok(None);
        }
        let session = value["sessionId"].as_str().unwrap_or("");
        if !session.is_empty() && !self.children.contains(session) {
            return Ok(None);
        }
        if value["method"] == "Target.attachedToTarget" {
            if self.children.len() >= 32 {
                return Err("Browser debug descendant target limit exceeded".into());
            }
            if let Some(id) = value["params"]["sessionId"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() < 512)
            {
                self.children.insert(id.to_owned());
            }
        } else if value["method"] == "Target.detachedFromTarget"
            && let Some(id) = value["params"]["sessionId"].as_str()
        {
            self.children.remove(id);
        }
        if self.mode == GatewayMode::Adapter
            && value["sessionId"].as_str().is_none_or(str::is_empty)
        {
            value["sessionId"] = json!(self.page);
        }
        Ok(Some(value))
    }
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |n, (a, b)| n | (a ^ b)) == 0
}
fn header(stream: &mut TcpStream, stop: &AtomicBool) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_millis(500);
    let mut bytes = vec![0u8; MAX_HEADER];
    loop {
        if stop.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("Browser debug handshake timed out".into());
        }
        match stream.peek(&mut bytes) {
            Ok(0) => return Err("Browser debug peer closed".into()),
            Ok(n) if bytes[..n].windows(4).any(|w| w == b"\r\n\r\n") => {
                bytes.truncate(n);
                return Ok(bytes);
            }
            Ok(n) if n == MAX_HEADER => return Err("Browser debug header exceeds16KiB".into()),
            Ok(_) => thread::sleep(Duration::from_millis(2)),
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(_) => return Err("Browser debug handshake failed".into()),
        }
    }
}
fn deny(stream: &mut TcpStream) {
    let _ = stream
        .write_all(b"HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
}
fn accept(
    mut stream: TcpStream,
    address: SocketAddr,
    token: &str,
    stop: &AtomicBool,
    mode: GatewayMode,
) -> Result<Option<WebSocket<TcpStream>>, String> {
    stream.set_nonblocking(true).map_err(|e| e.to_string())?;
    let bytes = match header(&mut stream, stop) {
        Ok(h) => h,
        Err(_) => {
            deny(&mut stream);
            return Ok(None);
        }
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(s) => s,
        Err(_) => {
            deny(&mut stream);
            return Ok(None);
        }
    };
    let lines = text.split("\r\n").collect::<Vec<_>>();
    let mut parts = lines.first().unwrap_or(&"").split(' ');
    let (method, path, version) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let values = lines
        .iter()
        .skip(1)
        .filter_map(|s| s.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim()))
        .collect::<Vec<_>>();
    let host = values
        .iter()
        .filter(|(k, _)| k == "host")
        .collect::<Vec<_>>();
    let allowed_host = host.len() == 1
        && ([format!("localhost:{}", address.port()), address.to_string()]
            .iter()
            .any(|s| host[0].1 == s)
            // The pinned upstream js-debug transport deliberately sends this
            // literal authority, including on WebSocket requests. No origins
            // are accepted in adapter mode, and the nonce is still mandatory.
            || (mode == GatewayMode::Adapter && host[0].1 == "localhost"));
    let origins = values
        .iter()
        .filter(|(k, _)| k == "origin")
        .collect::<Vec<_>>();
    let origin_allowed = origins.is_empty()
        || (mode == GatewayMode::Page
            && origins.len() == 1
            && matches!(
                origins[0].1,
                "devtools://devtools" | "codisseu-devtools://frontend"
            ));
    if cfg!(debug_assertions) && std::env::var_os("CODISSEU_BROWSER_AUTH_DIAGNOSTIC").is_some() {
        let origin_class = if origins.is_empty() {
            "absent"
        } else if origins[0].1 == "devtools://devtools" {
            "sdk-native"
        } else if origins[0].1 == "codisseu-devtools://frontend" {
            "sdk-custom"
        } else if origins[0].1 == "null" {
            "opaque-null"
        } else {
            "other"
        };
        eprintln!(
            "Owned browser handshake diagnostic: origin_class={origin_class} host_match={allowed_host} nonce_path_match={} upgrade={}",
            constant_eq(path.as_bytes(), format!("/{token}").as_bytes()),
            values.iter().any(|(k, _)| k == "upgrade")
        );
    }
    if method != "GET" || version != "HTTP/1.1" || !allowed_host || !origin_allowed {
        deny(&mut stream);
        return Ok(None);
    }
    if path == "/json/version" && !values.iter().any(|(k, _)| k == "upgrade") {
        // Consume the peeked request before closing, otherwise Linux may reset
        // the connection and a strict HTTP client discards the discovery reply.
        let header_length = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or("Incomplete browser debug header")?
            + 4;
        stream
            .read_exact(&mut vec![0; header_length])
            .map_err(|_| "Browser debug HTTP request closed")?;
        // js-debug always probes this first. Never publish the capability token.
        let body = json!({"Browser":"Codisseu single-page gateway","Protocol-Version":"1.3","webSocketDebuggerUrl":format!("ws://{address}/unavailable")}).to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        return Ok(None);
    }
    if !constant_eq(path.as_bytes(), format!("/{token}").as_bytes()) {
        deny(&mut stream);
        return Ok(None);
    }
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_CDP))
        .max_frame_size(Some(MAX_CDP))
        .write_buffer_size(0)
        .max_write_buffer_size(8 * 1024 * 1024);
    match tungstenite::accept_with_config(stream, Some(config)) {
        Ok(socket) => {
            socket
                .get_ref()
                .set_nonblocking(true)
                .map_err(|e| e.to_string())?;
            Ok(Some(socket))
        }
        Err(_) => Ok(None),
    }
}
fn ws_error(error: tungstenite::Error) -> Result<(), String> {
    match error {
        tungstenite::Error::Io(e)
            if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
        {
            Ok(())
        }
        _ => Err("Browser debug WebSocket closed or exceeded bounds".into()),
    }
}
fn send(socket: &mut WebSocket<TcpStream>, value: Value) -> Result<(), String> {
    let json = value.to_string();
    if json.len() > MAX_CDP {
        return Err("Browser debug response exceeds4MiB".into());
    }
    match socket.send(Message::Text(json.into())) {
        Ok(()) => Ok(()),
        Err(error) => ws_error(error),
    }
}
fn run(
    listener: TcpListener,
    address: SocketAddr,
    token: String,
    peer: &dyn Peer,
    router: &mut Router,
    stop: &AtomicBool,
    activity: &Activity,
) -> Result<(), String> {
    let mut socket = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut attempts = 0u32;
    while !stop.load(Ordering::Acquire) {
        if let Ok((mut stream, remote)) = listener.accept() {
            attempts += 1;
            if attempts > 128 {
                return Err("Browser debug handshake limit exceeded".into());
            }
            if socket.is_some() || !remote.ip().is_loopback() {
                deny(&mut stream);
            } else {
                socket = accept(stream, address, &token, stop, router.mode)?;
                if socket.is_some() {
                    activity.connected.store(true, Ordering::Release);
                }
            }
        }
        if socket.is_none() && Instant::now() >= deadline {
            return Err("Browser debug adapter did not connect".into());
        }
        if let Some(socket) = &mut socket {
            match socket.read() {
                Ok(Message::Text(raw)) => match router.request(parse(raw.as_str())?)? {
                    Routed::Local(replies) => {
                        for reply in replies {
                            send(socket, reply)?;
                        }
                    }
                    Routed::Forward(value) => peer.send(value.to_string())?,
                },
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => return Err("Browser debugger requires UTF-8 text messages".into()),
                Err(error) => ws_error(error)?,
            }
            for _ in 0..32 {
                let Some(raw) = peer.receive(Duration::ZERO)? else {
                    break;
                };
                if let Some(value) = router.event(parse(&raw)?)? {
                    if value["method"] == "Debugger.paused" {
                        activity.paused.fetch_add(1, Ordering::AcqRel);
                    }
                    send(socket, value)?;
                }
            }
            if let Err(error) = socket.flush() {
                ws_error(error)?;
            }
        } else if let Some(raw) = peer.receive(Duration::ZERO)? {
            let _ = router.event(parse(&raw)?)?;
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

#[cfg(test)]
#[path = "browser_bridge_tests.rs"]
mod tests;
