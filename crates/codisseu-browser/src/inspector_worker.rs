use super::{
    model::*,
    service::{Request, Shared},
    *,
};
use crate::{DevToolsChannel, DevToolsEvent};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::Ordering, mpsc::Receiver},
    time::{Duration, Instant},
};
const BODY: usize = 1024 * 1024;
#[derive(Clone)]
enum Tag {
    Enable,
    Network(bool),
    Noop,
    FrameTree,
    World,
    Walk,
    WalkNodes,
    NodeRef(i64),
    NodeDetails(i64),
    Props(String, bool),
    Body(String),
    Eval(String, Option<usize>),
    Cookies,
    Storage(StorageKind),
    Mutation,
    SetBp(String, u32, String),
    RemoveBp(String),
}
struct Pending {
    tag: Tag,
    document: u64,
    dom_generation: u64,
    pause_generation: u64,
    object_generation: u64,
    sent: Instant,
    method: &'static str,
}
struct Worker {
    channel: DevToolsChannel,
    shared: Arc<Shared>,
    state: Snapshot,
    pending: BTreeMap<u64, Pending>,
    next: u64,
    objects: BTreeSet<String>,
    dom_objects: BTreeSet<String>,
    object_generation: u64,
    node_ids: BTreeSet<i64>,
    node_indices: BTreeMap<i64, usize>,
    dom_handle: Option<String>,
    dom_generation: u64,
    pause_generation: u64,
    init: usize,
    dirty: bool,
    published: Instant,
    group: String,
}
impl Worker {
    fn error(&mut self, s: impl Into<String>) {
        let s = bounded(&s.into(), 1024);
        self.state.error = Some(s.clone());
        self.state.errors.push(s);
        if self.state.errors.len() > 16 {
            self.state.errors.remove(0);
        }
        self.dirty = true;
    }
    fn send(&mut self, method: &'static str, params: Value, tag: Tag) -> Result<(), String> {
        if self.pending.len() >= 64 {
            return Err("Limite de consultas simultâneas do inspetor".into());
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or("IDs do inspetor esgotados")?;
        let id = self.next;
        self.channel
            .sender
            .send(json!({"id":id,"method":method,"params":params}).to_string())?;
        self.pending.insert(
            id,
            Pending {
                tag,
                document: self.state.document_epoch,
                dom_generation: self.dom_generation,
                pause_generation: self.pause_generation,
                object_generation: self.object_generation,
                sent: Instant::now(),
                method,
            },
        );
        self.dirty = true;
        Ok(())
    }
    fn fire(&mut self, method: &'static str, params: Value, tag: Tag) {
        if let Err(e) = self.send(method, params, tag) {
            self.error(e)
        }
    }
    fn publish(&mut self, force: bool) {
        if !self.dirty || (!force && self.published.elapsed() < Duration::from_millis(50)) {
            return;
        }
        self.state.busy = !self.pending.is_empty();
        self.state.revision = self.shared.revision.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut l) = self.shared.latest.lock() {
            *l = Some(self.state.clone())
        }
        let w = self.shared.waker.lock().ok().and_then(|w| w.clone());
        if let Some(w) = w {
            w()
        }
        self.published = Instant::now();
        self.dirty = false;
    }
    fn register(&mut self, r: &mut RemoteValue) {
        if let Some(id) = &r.object_id {
            if self.objects.len() < 512 || self.objects.contains(id) {
                self.objects.insert(id.clone());
            } else {
                r.object_id = None;
            }
        }
    }
    fn collect_props(&mut self, v: &Value) -> Vec<Property> {
        let mut ps = properties(v);
        for p in &mut ps {
            self.register(&mut p.value)
        }
        ps
    }
    fn release_objects(&mut self) {
        self.object_generation += 1;
        self.objects.clear();
        self.dom_objects.clear();
        self.state.evaluation = None;
        self.state.properties.clear();
        self.state.property_target = None;
        self.state.scope_values.clear();
        for w in &mut self.state.watches {
            w.value = None;
            w.error = None;
        }
        for group in [self.group.clone(), format!("{}-dom", self.group)] {
            self.fire(
                "Runtime.releaseObjectGroup",
                json!({"objectGroup":group}),
                Tag::Noop,
            );
        }
        self.dom_handle = None;
        for id in std::mem::take(&mut self.dom_objects) {
            self.objects.remove(&id);
        }
        self.node_indices.clear();
        self.node_ids.clear();
        self.state.dom.clear();
        self.state.selected_node = None;
        self.state.attributes.clear();
        self.state.styles.clear();
        self.fire("Overlay.hideHighlight", json!({}), Tag::Noop);
        self.error("Limite de referências atingido; atualize o DOM ou as variáveis");
    }
    fn refresh_storage(&mut self) {
        let s = self.state.session.as_ref().unwrap().clone();
        self.fire("Network.getCookies", json!({"urls":[s.url]}), Tag::Cookies);
        for kind in [StorageKind::Local, StorageKind::Session] {
            self.fire("DOMStorage.getDOMStorageItems",json!({"storageId":{"securityOrigin":s.origin,"isLocalStorage":kind==StorageKind::Local}}),Tag::Storage(kind));
        }
    }
    fn eval(&mut self, expression: String, watch: Option<usize>) {
        let tag = Tag::Eval(expression.clone(), watch);
        if let Some(frame) = self.state.paused.as_ref().and_then(|p| p.selected.as_ref()) {
            self.fire("Debugger.evaluateOnCallFrame",json!({"callFrameId":frame,"expression":expression,"objectGroup":self.group,"returnByValue":false,"generatePreview":false,"silent":false}),tag)
        } else {
            self.fire("Runtime.evaluate",json!({"expression":expression,"objectGroup":self.group,"returnByValue":false,"generatePreview":false,"awaitPromise":false,"timeout":2000,"userGesture":false}),tag)
        }
    }
    fn watches(&mut self) {
        let expressions = self
            .state
            .watches
            .iter()
            .map(|w| w.expression.clone())
            .collect::<Vec<_>>();
        for (i, e) in expressions.into_iter().enumerate() {
            self.eval(e, Some(i))
        }
    }
    fn select_frame(&mut self, id: String) -> Result<(), String> {
        let scopes = self
            .state
            .paused
            .as_ref()
            .and_then(|p| p.frames.iter().find(|f| f.id == id))
            .ok_or("Frame de pausa obsoleto")?
            .scopes
            .clone();
        self.state.paused.as_mut().unwrap().selected = Some(id);
        self.state.scope_values.clear();
        for scope in scopes {
            if self.objects.len() < 512 {
                self.objects.insert(scope.object_id.clone());
                self.fire("Runtime.getProperties",json!({"objectId":scope.object_id,"ownProperties":true,"accessorPropertiesOnly":false,"generatePreview":false}),Tag::Props(scope.object_id,true));
            }
        }
        self.watches();
        Ok(())
    }
    fn action(&mut self, action: Action) -> Result<(), String> {
        validate_action(&action)?;
        match action {
            Action::RefreshDom => self.refresh_dom(),
            Action::SelectNode {node_id} => {
                let index = *self.node_indices.get(&node_id).ok_or("Nó DOM obsoleto ou desconhecido")?;
                let handle = self.dom_handle.clone().ok_or("Árvore DOM obsoleta")?;
                self.state.selected_node = Some(node_id);
                self.state.attributes.clear();
                self.state.styles.clear();
                self.state.properties.clear();
                self.fire("Runtime.callFunctionOn", json!({
                    "objectId":handle,"functionDeclaration":dom::NODE,
                    "arguments":[{"value":index}],"returnByValue":false,
                    "generatePreview":false,"objectGroup":format!("{}-dom",self.group)
                }), Tag::NodeRef(node_id));
            }
            Action::GetProperties {object_id} => {
                if !self.objects.contains(&object_id) {return Err("Objeto obsoleto ou desconhecido".into());}
                let scope = self.state.paused.as_ref().and_then(|p|
                    p.selected.as_ref().and_then(|id|p.frames.iter().find(|f|&f.id==id)))
                    .is_some_and(|f|f.scopes.iter().any(|s|s.object_id==object_id));
                if !scope {self.state.property_target=Some(object_id.clone());}
                self.fire("Runtime.getProperties",json!({"objectId":object_id,
                    "ownProperties":true,"accessorPropertiesOnly":false,"generatePreview":false
                }),Tag::Props(object_id,scope));
            }
            Action::Network {enabled} => {
                let params = if enabled {json!({"maxTotalBufferSize":8*BODY,"maxResourceBufferSize":BODY,"maxPostDataSize":65536})} else {json!({})};
                self.fire(if enabled {"Network.enable"} else {"Network.disable"},params,Tag::Network(enabled));
            }
            Action::PreserveNetwork(v) => self.state.preserve_network=v,
            Action::ClearNetwork => {self.state.network.clear();self.state.bodies.clear();}
            Action::GetResponseBody {request_id} => {
                let request = self.state.network.iter().find(|r|r.id==request_id).ok_or("Request desconhecido ou removido")?;
                match body_availability(request) {
                    Ok(true) => self.fire("Network.getResponseBody",json!({"requestId":request_id}),Tag::Body(request_id)),
                    availability => {
                        while self.state.bodies.len()>=4 {
                            let key=self.state.bodies.keys().next().cloned().unwrap();
                            self.state.bodies.remove(&key);
                        }
                        self.state.bodies.insert(request_id.clone(),Body {
                            request_id,error:availability.err().map(str::to_owned),..Default::default()
                        });
                    }
                }
            }
            Action::Evaluate {expression} => self.eval(expression,None),
            Action::SetWatches(expressions) => {
                self.state.watches=expressions.into_iter().map(|expression|Evaluation {expression,..Default::default()}).collect();
                self.watches();
            }
            Action::RefreshWatches => self.watches(),
            Action::Pause => self.fire("Debugger.pause",json!({}),Tag::Noop),
            Action::Resume | Action::StepOver | Action::StepInto | Action::StepOut => {
                if self.state.paused.is_none() {return Err("A página não está pausada".into());}
                let method=match action {
                    Action::Resume=>"Debugger.resume",Action::StepOver=>"Debugger.stepOver",
                    Action::StepInto=>"Debugger.stepInto",_=>"Debugger.stepOut"
                };
                self.fire(method,json!({}),Tag::Noop);
            }
            Action::SelectFrame {frame_id} => self.select_frame(frame_id)?,
            Action::SetBreakpoint {url,line,condition} => {
                crate::validate_url(&url)?;
                self.fire("Debugger.setBreakpointByUrl",json!({"url":url,"lineNumber":line,"condition":condition}),Tag::SetBp(url,line,condition));
            }
            Action::RemoveBreakpoint {breakpoint_id} => {
                if !self.state.breakpoints.iter().any(|b|b.id==breakpoint_id) {return Err("Breakpoint desconhecido".into());}
                self.fire("Debugger.removeBreakpoint",json!({"breakpointId":breakpoint_id}),Tag::RemoveBp(breakpoint_id));
            }
            Action::RefreshStorage => self.refresh_storage(),
            Action::SetStorage {kind,key,value} => self.fire("DOMStorage.setDOMStorageItem",json!({
                "storageId":{"securityOrigin":self.state.session.as_ref().unwrap().origin,"isLocalStorage":kind==StorageKind::Local},
                "key":key,"value":value
            }),Tag::Mutation),
            Action::DeleteStorage {kind,key} => self.fire("DOMStorage.removeDOMStorageItem",json!({
                "storageId":{"securityOrigin":self.state.session.as_ref().unwrap().origin,"isLocalStorage":kind==StorageKind::Local},"key":key
            }),Tag::Mutation),
            Action::SetCookie {mut cookie} => {
                let session=self.state.session.as_ref().unwrap();
                let u=url::Url::parse(&session.url).map_err(|_|"URL ativa inválida")?;
                let host=u.host_str().ok_or("Host inválido")?;
                if cookie.domain.is_empty() {cookie.domain=host.into();}
                if cookie.domain.trim_start_matches('.')!=host && !self.state.cookies.iter().any(|c|c.name==cookie.name&&c.domain==cookie.domain&&c.path==cookie.path) {
                    return Err("Cookie não pertence à origem ativa".into());
                }
                let mut params=json!({"url":session.url,"name":cookie.name,"value":cookie.value,
                    "domain":cookie.domain,"path":cookie.path,"secure":cookie.secure,"httpOnly":cookie.http_only});
                if let Some(s)=cookie.same_site {params["sameSite"]=json!(s);}
                if !cookie.session && cookie.expires>0. {params["expires"]=json!(cookie.expires);}
                self.fire("Network.setCookie",params,Tag::Mutation);
            }
            Action::DeleteCookie {name,domain,path} => {
                if !self.state.cookies.iter().any(|c|c.name==name&&c.domain==domain&&c.path==path) {
                    return Err("Cookie obsoleto ou fora da origem ativa".into());
                }
                self.fire("Network.deleteCookies",json!({"url":self.state.session.as_ref().unwrap().url,"name":name,"domain":domain,"path":path}),Tag::Mutation);
            }
        }
        self.dirty = true;
        Ok(())
    }
    fn refresh_dom(&mut self) {
        self.dom_generation += 1;
        self.dom_handle = None;
        for id in std::mem::take(&mut self.dom_objects) {
            self.objects.remove(&id);
        }
        self.node_indices.clear();
        self.node_ids.clear();
        self.state.dom.clear();
        self.state.dom_truncated = false;
        self.state.selected_node = None;
        self.state.attributes.clear();
        self.state.styles.clear();
        self.state.properties.clear();
        self.state.property_target = None;
        self.fire("Overlay.hideHighlight", json!({}), Tag::Noop);
        self.fire(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":format!("{}-dom",self.group)}),
            Tag::Noop,
        );
        self.fire("Page.getFrameTree", json!({}), Tag::FrameTree);
    }
    fn reset_context(&mut self) {
        self.state.document_epoch += 1;
        self.pause_generation += 1;
        self.node_ids.clear();
        self.node_indices.clear();
        self.dom_handle = None;
        self.dom_generation += 1;
        self.objects.clear();
        self.state.dom.clear();
        self.state.selected_node = None;
        self.state.attributes.clear();
        self.state.styles.clear();
        self.state.properties.clear();
        self.state.scope_values.clear();
        self.state.cookies.clear();
        self.state.local_storage.clear();
        self.state.session_storage.clear();
        self.state.bodies.clear();
        self.state.property_target = None;
        self.state.paused = None;
        self.state.evaluation = None;
        for w in &mut self.state.watches {
            w.value = None;
            w.error = None;
        }
        self.fire(
            "Runtime.releaseObjectGroup",
            json!({"objectGroup":self.group}),
            Tag::Noop,
        );
        self.dirty = true;
    }
    fn response(&mut self, v: Value) {
        let Some(id) = v["id"].as_u64() else { return };
        let Some(p) = self.pending.remove(&id) else {
            return;
        };
        if matches!(p.tag, Tag::Enable) {
            self.init = self.init.saturating_sub(1);
            if self.init == 0 && self.state.lifecycle != Lifecycle::Failed {
                self.state.lifecycle = Lifecycle::Ready
            }
        }
        if p.document != self.state.document_epoch {
            return;
        }
        if matches!(
            p.tag,
            Tag::FrameTree
                | Tag::World
                | Tag::Walk
                | Tag::WalkNodes
                | Tag::NodeRef(_)
                | Tag::NodeDetails(_)
        ) && p.dom_generation != self.dom_generation
        {
            return;
        }

        if v.get("error").is_some() {
            if matches!(p.tag, Tag::Enable) {
                self.state.lifecycle = Lifecycle::Failed;
                self.state.network_enabled = false;
            }
            self.error(format!(
                "{} recusado (código {})",
                p.method,
                v["error"]["code"].as_i64().unwrap_or(0)
            ));
            return;
        }
        if matches!(p.tag, Tag::Props(_, _) | Tag::Eval(_, _) | Tag::NodeRef(_))
            && p.object_generation != self.object_generation
        {
            return;
        }
        if (matches!(p.tag, Tag::Props(_, true)) || p.method == "Debugger.evaluateOnCallFrame")
            && p.pause_generation != self.pause_generation
        {
            return;
        }
        let r = &v["result"];
        match p.tag {
            Tag::Enable | Tag::Noop => {}
            Tag::Network(enabled) => self.state.network_enabled = enabled,
            Tag::FrameTree => {
                let frame = text(&r["frameTree"]["frame"]["id"], 1024);
                if !frame.is_empty() {
                    self.fire("Page.createIsolatedWorld",json!({"frameId":frame,"worldName":"Codisseu native DOM","grantUniveralAccess":false}),Tag::World)
                }
            }
            Tag::World => {
                if let Some(context) = r["executionContextId"].as_i64() {
                    self.fire("Runtime.evaluate",json!({"contextId":context,"expression":dom::WALK,"objectGroup":format!("{}-dom",self.group),"returnByValue":false,"generatePreview":false,"timeout":2000,"silent":true}),Tag::Walk)
                }
            }
            Tag::Walk => {
                if let Some(id) = r["result"]["objectId"].as_str().filter(|s| s.len() <= 1024) {
                    self.dom_handle = Some(id.into());
                    self.fire("Runtime.callFunctionOn",json!({"objectId":id,"functionDeclaration":dom::NODES,"returnByValue":true,"generatePreview":false}),Tag::WalkNodes)
                } else {
                    self.error("Não foi possível ler o DOM isolado")
                }
            }
            Tag::WalkNodes => {
                self.state.dom = walk_nodes(&r["result"]["value"]["nodes"], self.dom_generation);
                self.state.dom_truncated =
                    r["result"]["value"]["truncated"].as_bool().unwrap_or(false);
                self.node_ids = self.state.dom.iter().map(|n| n.id).collect();
                self.node_indices = self
                    .state
                    .dom
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.id, i))
                    .collect();
            }
            Tag::NodeRef(node) => {
                if self.state.selected_node == Some(node) {
                    let mut object = remote(&r["result"]);
                    self.register(&mut object);
                    if let Some(id) = object.object_id {
                        self.dom_objects.insert(id.clone());
                        if self
                            .state
                            .dom
                            .iter()
                            .any(|n| n.id == node && n.node_type == 1)
                        {
                            self.fire("Overlay.highlightNode",json!({"objectId":id,"highlightConfig":{"showInfo":true,"contentColor":{"r":83,"g":156,"b":219,"a":0.25},"borderColor":{"r":83,"g":156,"b":219,"a":0.8}}}),Tag::Noop);
                        } else {
                            self.fire("Overlay.hideHighlight", json!({}), Tag::Noop);
                        }
                        self.state.property_target = Some(id.clone());
                        self.fire("Runtime.callFunctionOn",json!({"objectId":id,"functionDeclaration":dom::DETAILS,"returnByValue":true,"generatePreview":false}),Tag::NodeDetails(node));
                        self.fire("Runtime.getProperties",json!({"objectId":id,"ownProperties":true,"accessorPropertiesOnly":false,"generatePreview":false}),Tag::Props(id,false));
                    }
                }
            }
            Tag::NodeDetails(node) => {
                if self.state.selected_node == Some(node) {
                    let value = &r["result"]["value"];
                    self.state.attributes = value["attributes"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .take(32)
                                .filter_map(|v| {
                                    v.as_array()
                                        .filter(|p| p.len() == 2)
                                        .map(|p| (text(&p[0], 256), text(&p[1], 1024)))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.state.styles = value["styles"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .take(512)
                                .filter_map(|v| {
                                    v.as_array()
                                        .filter(|p| p.len() == 2)
                                        .map(|p| (text(&p[0], 256), text(&p[1], 1024)))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                }
            }
            Tag::Props(id, scope) => {
                let props = self.collect_props(&r["result"]);
                if self.dom_objects.contains(&id) {
                    self.dom_objects
                        .extend(props.iter().filter_map(|p| p.value.object_id.clone()));
                }
                if scope {
                    if self.state.scope_values.len() < 32 {
                        self.state.scope_values.insert(id, props);
                    }
                } else if self.state.property_target.as_deref() == Some(&id) {
                    self.state.properties = props;
                }
            }
            Tag::Body(id) => {
                if self.state.network.iter().any(|n| n.id == id) {
                    while self.state.bodies.len() >= 4 {
                        let k = self.state.bodies.keys().next().cloned().unwrap();
                        self.state.bodies.remove(&k);
                    }
                    let source = r["body"].as_str().unwrap_or("");
                    self.state.bodies.insert(
                        id.clone(),
                        Body {
                            request_id: id,
                            text: bounded(source, BODY),
                            base64: r["base64Encoded"].as_bool().unwrap_or(false),
                            truncated: source.len() > BODY,
                            error: None,
                        },
                    );
                }
            }
            Tag::Eval(expression, watch) => {
                let mut e = Evaluation {
                    expression,
                    ..Default::default()
                };
                if r.get("exceptionDetails").is_some() {
                    e.error = Some("A expressão lançou uma exceção na página".into());
                } else {
                    let mut value = remote(&r["result"]);
                    self.register(&mut value);
                    e.value = Some(value)
                }
                if let Some(i) = watch {
                    if self
                        .state
                        .watches
                        .get(i)
                        .is_some_and(|w| w.expression == e.expression)
                    {
                        self.state.watches[i] = e;
                    }
                } else {
                    self.state.evaluation = Some(e)
                }
            }
            Tag::Cookies => {
                let mut bytes = 0;
                self.state.cookies = r["cookies"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .take(128)
                            .map(cookie)
                            .filter(|c| {
                                bytes +=
                                    c.name.len() + c.value.len() + c.domain.len() + c.path.len();
                                bytes <= BODY
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            }
            Tag::Storage(kind) => {
                let items = storage(&r["entries"]);
                if kind == StorageKind::Local {
                    self.state.local_storage = items
                } else {
                    self.state.session_storage = items
                }
            }
            Tag::Mutation => {
                if r["success"] == false {
                    self.error("A alteração foi recusada pela página");
                }
                self.refresh_storage();
            }
            Tag::SetBp(url, line, condition) => {
                if let Some(id) = r["breakpointId"].as_str().filter(|s| s.len() <= 1024)
                    && self.state.breakpoints.len() < 128
                {
                    self.state.breakpoints.push(Breakpoint {
                        id: id.into(),
                        url,
                        line,
                        condition,
                    });
                }
            }
            Tag::RemoveBp(id) => self.state.breakpoints.retain(|b| b.id != id),
        }
        if self.objects.len() >= 512 {
            self.release_objects();
        }
        self.dirty = true;
    }
    fn event(&mut self, v: Value) {
        if v.get("id").is_some() {
            self.response(v);
            return;
        }
        if !self.state.network_enabled
            && v["method"]
                .as_str()
                .is_some_and(|m| m.starts_with("Network."))
        {
            return;
        }
        let p = &v["params"];
        match v["method"].as_str().unwrap_or("") {
            "DOM.documentUpdated" => {
                self.node_ids.clear();
                self.state.dom.clear();
                self.refresh_dom()
            }
            "Runtime.executionContextsCleared" => self.reset_context(),
            "Page.frameNavigated" if p["frame"].get("parentId").is_none() => {
                let url = text(&p["frame"]["url"], 8192);
                if let Ok(origin) = origin(&url) {
                    self.reset_context();
                    let session = self.state.session.as_mut().unwrap();
                    session.url = url;
                    session.origin = origin;
                    self.state.cookies.clear();
                    self.state.local_storage.clear();
                    self.state.session_storage.clear();
                    self.state.bodies.clear();
                    if !self.state.preserve_network {
                        self.state.network.clear()
                    }
                    self.refresh_dom();
                    self.refresh_storage();
                } else {
                    self.state.lifecycle = Lifecycle::Failed;
                    self.error("Página ativa não possui origem HTTP/HTTPS inspecionável");
                }
            }
            "Debugger.paused" => {
                self.pause_generation += 1;
                self.objects.clear();
                self.state.scope_values.clear();
                self.state.paused = Some(native_pause(p));
                if let Some(id) = self.state.paused.as_ref().and_then(|p| p.selected.clone()) {
                    let _ = self.select_frame(id);
                }
            }
            "Debugger.resumed" => {
                self.pause_generation += 1;
                for w in &mut self.state.watches {
                    w.value = None;
                    w.error = None;
                }
                self.state.paused = None;
                self.state.scope_values.clear();
                self.objects.clear();
                self.state.properties.clear();
                self.state.property_target = None;
            }
            "Network.requestWillBeSent" => {
                let id = text(&p["requestId"], 1024);
                if id.is_empty() {
                    return;
                }
                let r = &p["request"];
                let req = NetworkRequest {
                    id: id.clone(),
                    url: text(&r["url"], 8192),
                    method: text(&r["method"], 64),
                    kind: text(&p["type"], 64),
                    document_url: text(&p["documentURL"], 8192),
                    request_headers: pairs(&r["headers"], 64),
                    post_data: r["postData"].as_str().map(|s| bounded(s, 65536)),
                    timestamp: p["timestamp"].as_f64().unwrap_or(0.),
                    initiator: text(&p["initiator"]["type"], 128),
                    ..Default::default()
                };
                self.state.network.retain(|n| n.id != id);
                self.state.network.push(req);
                self.trim_network();
            }
            "Network.responseReceived" => {
                let id = p["requestId"].as_str().unwrap_or("");
                if let Some(n) = self.state.network.iter_mut().find(|n| n.id == id) {
                    let r = &p["response"];
                    n.status = r["status"].as_f64();
                    n.status_text = text(&r["statusText"], 256);
                    n.mime_type = text(&r["mimeType"], 256);
                    n.protocol = text(&r["protocol"], 64);
                    n.response_headers = pairs(&r["headers"], 64);
                    n.from_cache = r["fromDiskCache"].as_bool().unwrap_or(false)
                        || r["fromServiceWorker"].as_bool().unwrap_or(false);
                }
            }
            "Network.dataReceived" => {
                if let Some(n) = self
                    .state
                    .network
                    .iter_mut()
                    .find(|n| n.id == p["requestId"].as_str().unwrap_or(""))
                    && let Some(size) = p["dataLength"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.)
                {
                    n.decoded_bytes = Some((n.decoded_bytes.unwrap_or(0.) + size).min(1e12));
                }
            }
            "Network.loadingFinished" | "Network.loadingFailed" => {
                let id = p["requestId"].as_str().unwrap_or("");
                if let Some(n) = self.state.network.iter_mut().find(|n| n.id == id) {
                    n.finished = true;
                    n.encoded_bytes = p["encodedDataLength"].as_f64().unwrap_or(0.);
                    n.duration_ms = p["timestamp"]
                        .as_f64()
                        .map(|t| ((t - n.timestamp) * 1000.).max(0.));
                    if v["method"] == "Network.loadingFailed" {
                        n.failed = Some(text(&p["errorText"], 512))
                    }
                }
            }
            "Network.requestServedFromCache" => {
                if let Some(n) = self
                    .state
                    .network
                    .iter_mut()
                    .find(|n| n.id == p["requestId"].as_str().unwrap_or(""))
                {
                    n.from_cache = true
                }
            }
            _ => return,
        }
        self.trim_network();
        self.dirty = true;
    }
    fn trim_network(&mut self) {
        while self.state.network.len() > 200
            || self
                .state
                .network
                .iter()
                .map(|n| {
                    n.url.len()
                        + n.document_url.len()
                        + n.post_data.as_ref().map_or(0, String::len)
                        + n.request_headers
                            .iter()
                            .chain(&n.response_headers)
                            .map(|(a, b)| a.len() + b.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
                > 4 * BODY
        {
            let n = self.state.network.remove(0);
            self.state.bodies.remove(&n.id);
        }
    }
    fn receive(&mut self, event: DevToolsEvent) -> bool {
        match event {
            DevToolsEvent::Message { json } => {
                if let Ok(v) = serde_json::from_str(&json) {
                    self.event(v)
                } else {
                    self.error("Resposta inválida do inspetor");
                    return false;
                }
            }
            DevToolsEvent::Closed { .. } => {
                self.error("Canal da página encerrado");
                return false;
            }
        }
        true
    }
}
// Only request bodies whose transferred AND decoded sizes were observed within the cap.
// Empty responses require no potentially large CDP reply.
fn body_availability(r: &NetworkRequest) -> Result<bool, &'static str> {
    if !r.finished {
        return Err("A requisição ainda não terminou");
    }
    if r.failed.is_some() {
        return Err("A requisição falhou; corpo indisponível");
    }
    if r.method == "HEAD" || matches!(r.status, Some(204.0 | 205.0 | 304.0)) {
        return Ok(false);
    }
    if r.encoded_bytes == 0. && r.decoded_bytes == Some(0.) {
        return Ok(false);
    }
    if !r.encoded_bytes.is_finite() || r.encoded_bytes < 0. || r.encoded_bytes > BODY as f64 {
        return Err("Corpo excede o limite transferido de1MiB");
    }
    let decoded = r
        .decoded_bytes
        .ok_or("Tamanho decodificado não disponível; leitura recusada");
    if decoded?.is_finite() && decoded? >= 0. && decoded? <= BODY as f64 {
        Ok(true)
    } else {
        Err("Corpo excede o limite decodificado de1MiB")
    }
}
pub(super) fn validate_action(a: &Action) -> Result<(), String> {
    let short = |s: &str, n: usize| s.len() <= n && !s.contains('\0');
    let valid = match a {
        Action::Evaluate { expression } => short(expression, 16384),
        Action::SetWatches(v) => v.len() <= 32 && v.iter().all(|s| short(s, 16384)),
        Action::GetProperties { object_id } => short(object_id, 1024),
        Action::SelectFrame { frame_id } => short(frame_id, 1024),
        Action::GetResponseBody { request_id } => short(request_id, 1024),
        Action::SetBreakpoint {
            url,
            line,
            condition,
        } => *line <= 10_000_000 && short(url, 8192) && short(condition, 16384),
        Action::RemoveBreakpoint { breakpoint_id } => short(breakpoint_id, 1024),
        Action::SetStorage { key, value, .. } => short(key, 4096) && short(value, 65536),
        Action::DeleteStorage { key, .. } => short(key, 4096),
        Action::SetCookie { cookie: c } => {
            short(&c.name, 1024)
                && !c.name.is_empty()
                && !c.name.chars().any(|c| c.is_control() || " ;,=".contains(c))
                && short(&c.value, 8192)
                && short(&c.domain, 512)
                && short(&c.path, 2048)
                && c.path.starts_with('/')
                && c.expires.is_finite()
                && c.same_site
                    .as_ref()
                    .is_none_or(|v| matches!(v.as_str(), "Strict" | "Lax" | "None"))
        }
        Action::DeleteCookie { name, domain, path } => {
            short(name, 1024) && short(domain, 512) && short(path, 2048)
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err("Entrada do inspetor inválida ou excessiva".into())
    }
}
pub(super) fn run(
    channel: DevToolsChannel,
    rx: Receiver<Request>,
    shared: Arc<Shared>,
    state: Snapshot,
) {
    let group = format!("codisseu-native-{}", state.session.as_ref().unwrap().epoch);
    let mut w = Worker {
        channel,
        shared,
        state: Snapshot {
            network_enabled: true,
            ..state
        },
        pending: BTreeMap::new(),
        next: 0,
        objects: BTreeSet::new(),
        dom_objects: BTreeSet::new(),
        object_generation: 0,
        node_ids: BTreeSet::new(),
        node_indices: BTreeMap::new(),
        dom_handle: None,
        dom_generation: 0,
        pause_generation: 0,
        init: 7,
        dirty: true,
        published: Instant::now(),
        group,
    };
    for (method, params) in [
        ("Page.enable", json!({})),
        ("Runtime.enable", json!({})),
        (
            "Network.enable",
            json!({"maxTotalBufferSize":8*BODY,"maxResourceBufferSize":BODY,"maxPostDataSize":65536}),
        ),
        ("DOMStorage.enable", json!({})),
        ("DOM.enable", json!({})),
        ("Overlay.enable", json!({})),
        ("Debugger.enable", json!({"maxScriptsCacheSize":BODY})),
    ] {
        w.fire(method, params, Tag::Enable)
    }
    w.refresh_dom();
    w.refresh_storage();
    w.publish(true);
    let mut live = true;
    while live && !w.shared.stop.load(Ordering::Acquire) && w.state.lifecycle != Lifecycle::Failed {
        let mut drained = 0;
        while let Ok(e) = w.channel.events.try_recv() {
            drained += 1;
            if !w.receive(e) {
                live = false;
                break;
            }
            if drained >= 256 {
                break;
            }
        }
        for _ in 0..16 {
            let Ok(r) = rx.try_recv() else { break };
            if r.document != w.state.document_epoch {
                w.error("A página mudou; atualize o inspetor antes desta ação");
                continue;
            }
            if let Err(e) = w.action(r.action) {
                w.error(e)
            }
        }
        let expired = w
            .pending
            .iter()
            .filter(|(_, p)| p.sent.elapsed() > Duration::from_secs(8))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in expired {
            if let Some(p) = w.pending.remove(&id) {
                if matches!(p.tag, Tag::Enable) {
                    w.init = w.init.saturating_sub(1);
                    w.state.lifecycle = Lifecycle::Failed;
                }
                w.error(format!("Tempo limite de {}", p.method));
            }
        }
        w.publish(false);
        if drained == 0 && live {
            match w.channel.events.recv_timeout(Duration::from_millis(5)) {
                Ok(e) => live = w.receive(e),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => live = false,
                Err(_) => {}
            }
        }
    }
    if !w.shared.stop.load(Ordering::Acquire) {
        let error = w
            .state
            .error
            .clone()
            .unwrap_or_else(|| "Canal da página encerrado".into());
        w.state = Snapshot {
            lifecycle: Lifecycle::Failed,
            error: Some(error.clone()),
            errors: vec![error],
            ..Default::default()
        };
        w.dirty = true;
        w.publish(true)
    }
    let _ = w
        .channel
        .sender
        .send(json!({"id":2_000_000_004u64,"method":"Overlay.hideHighlight"}).to_string());
    let _ = w
        .channel
        .sender
        .send(json!({"id":2_000_000_001u64,"method":"Debugger.resume"}).to_string());
    let _ = w
        .channel
        .sender
        .send(json!({"id":2_000_000_002u64,"method":"Debugger.disable"}).to_string());
    let _=w.channel.sender.send(json!({"id":2_000_000_003u64,"method":"Runtime.releaseObjectGroup","params":{"objectGroup":w.group}}).to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_gate_requires_finished_and_observed_decoded_size_preventing_gzip_bombs() {
        let mut r = NetworkRequest {
            finished: true,
            encoded_bytes: 1024.,
            decoded_bytes: Some(4096.),
            ..Default::default()
        };
        assert_eq!(body_availability(&r), Ok(true));
        r.decoded_bytes = Some((BODY + 1) as f64);
        assert!(body_availability(&r).is_err());
        r.decoded_bytes = None;
        assert!(body_availability(&r).is_err());
        r.decoded_bytes = Some(10.);
        r.finished = false;
        assert!(body_availability(&r).is_err());
        r.finished = true;
        r.encoded_bytes = (BODY + 1) as f64;
        assert!(body_availability(&r).is_err());
        r.encoded_bytes = 10.;
        r.decoded_bytes = Some(f64::NAN);
        assert!(body_availability(&r).is_err());
    }
    #[test]
    fn completed_empty_http_responses_need_no_cdp_body_fetch() {
        for status in [204., 205., 304.] {
            assert_eq!(
                body_availability(&NetworkRequest {
                    status: Some(status),
                    finished: true,
                    ..Default::default()
                }),
                Ok(false)
            );
        }
        assert_eq!(
            body_availability(&NetworkRequest {
                method: "HEAD".into(),
                finished: true,
                ..Default::default()
            }),
            Ok(false)
        );
        assert_eq!(
            body_availability(&NetworkRequest {
                finished: true,
                decoded_bytes: Some(0.),
                ..Default::default()
            }),
            Ok(false)
        );
    }
    #[test]
    fn typed_actions_reject_excessive_secret_inputs_and_accept_first_line_breakpoints() {
        assert!(
            validate_action(&Action::SetBreakpoint {
                url: "https://example.com/app.js".into(),
                line: 0,
                condition: String::new()
            })
            .is_ok()
        );
        assert!(validate_action(&Action::SetWatches(vec!["secret".into(); 33])).is_err());
        assert!(
            validate_action(&Action::Evaluate {
                expression: "a".repeat(16385)
            })
            .is_err()
        );
        assert!(
            validate_action(&Action::SetCookie {
                cookie: Cookie {
                    name: "invalid;cookie".into(),
                    path: "/".into(),
                    ..Default::default()
                }
            })
            .is_err()
        );
        assert!(
            !format!(
                "{:?}",
                Action::Evaluate {
                    expression: "fixture-secret-token".into()
                }
            )
            .contains("fixture-secret-token")
        );
    }
}
