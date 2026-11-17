//! Dedicated CEF process. No IDE APIs, remote debugging port or unsandboxed fallback.
mod frontend_assets;
use cef::{args::Args, *};
use codisseu_browser::protocol::{
    self, BrowserId, ClipboardAction, Command as Input, Event, KeyKind, Modifiers, MouseButton,
    Surface,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{MetadataExt, PermissionsExt},
            net::UnixStream,
        },
    },
    path::PathBuf,
    rc::Rc as LocalRc,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
// Bounded bursts from ordinary public pages must fit while one OSR frame is in flight.
// Keep control capacity independent of CDP so overload still detaches/resumes cleanly.
const OUTPUT_EVENTS: usize = 1024;
const OUTPUT_CONTROL_RESERVE: usize = 32;
const OUTPUT_CDP_EVENTS: usize = OUTPUT_EVENTS - OUTPUT_CONTROL_RESERVE;
const OUTPUT_EVENT_BYTES: usize = 16 * 1024 * 1024;
// These five IDs never enter a controller pending map, including after ownership changes.
const CDP_RESERVED_ID_START: i64 = 2147483643;
type Local = LocalRc<RefCell<State>>;
type Key = (BrowserId, Surface);
struct Packet {
    event: Event,
    bytes: Vec<u8>,
}
#[derive(Default)]
struct Queue {
    events: VecDeque<Packet>,
    event_bytes: usize,
    frames: BTreeMap<Key, Packet>,
    states: BTreeMap<BrowserId, Packet>,
}
#[derive(Default)]
struct Output {
    queue: Mutex<Queue>,
    wake: Condvar,
    dead: AtomicBool,
}
impl Output {
    fn send(&self, event: Event, bytes: Vec<u8>) {
        if self.dead.load(Ordering::Relaxed) {
            return;
        }
        let mut queue = self.queue.lock().unwrap();
        match &event {
            Event::Frame { id, surface, .. } => {
                queue
                    .frames
                    .insert((*id, *surface), Packet { event, bytes });
            }
            Event::State { id, .. } => {
                queue.states.insert(*id, Packet { event, bytes });
            }
            _ => {
                if queue.events.len() >= OUTPUT_EVENTS
                    || queue.event_bytes.saturating_add(bytes.len()) > OUTPUT_EVENT_BYTES
                {
                    self.dead.store(true, Ordering::Relaxed);
                } else {
                    queue.event_bytes += bytes.len();
                    queue.events.push_back(Packet { event, bytes });
                }
            }
        }
        self.wake.notify_one();
    }
    fn cdp(&self, id: BrowserId, epoch: u64, bytes: Vec<u8>) -> bool {
        if self.dead.load(Ordering::Relaxed) {
            return false;
        }
        let mut queue = self.queue.lock().unwrap();
        // Reserve control slots so a slow CDP reader detaches its controller,
        // resumes the target and keeps ordinary browsing alive.
        if queue.events.len() >= OUTPUT_CDP_EVENTS
            || queue.event_bytes.saturating_add(bytes.len()) > OUTPUT_EVENT_BYTES
        {
            return false;
        }
        queue.event_bytes += bytes.len();
        queue.events.push_back(Packet {
            event: Event::DevToolsMessage { id, epoch },
            bytes,
        });
        self.wake.notify_one();
        true
    }
    fn forget(&self, key: Key) {
        let mut queue = self.queue.lock().unwrap();
        queue.frames.remove(&key);
    }
    fn forget_state(&self, id: BrowserId) {
        self.queue.lock().unwrap().states.remove(&id);
    }
    fn writer(self: Arc<Self>, mut socket: UnixStream) {
        loop {
            let packet = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if self.dead.load(Ordering::Relaxed) {
                        return;
                    }
                    if let Some(p) = q.events.pop_front() {
                        q.event_bytes -= p.bytes.len();
                        break p;
                    }
                    if let Some((_, p)) = q.states.pop_first() {
                        break p;
                    }
                    if let Some((_, p)) = q.frames.pop_first() {
                        break p;
                    }
                    q = self.wake.wait(q).unwrap();
                }
            };
            if protocol::write_event(&mut socket, &packet.event, &packet.bytes).is_err() {
                self.dead.store(true, Ordering::Relaxed);
                return;
            }
        }
    }
}
#[derive(Clone, Copy)]
struct ViewIdentity {
    id: BrowserId,
    surface: Surface,
}
#[derive(Clone)]
struct ClientData {
    identity: ViewIdentity,
    out: Arc<Output>,
    frontend_ws: Option<String>,
}
const _: fn() = || {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ClientData>();
    send_sync::<ViewIdentity>();
    send_sync::<Arc<AtomicBool>>();
    send_sync::<frontend_assets::Registry>();
};
struct View {
    browser: Option<Browser>,
    width: u32,
    height: u32,
    scale: f32,
    visible: bool,
    popup: Option<Rect>,
    frontend_ws: Option<String>,
    base: Vec<u8>,
    popup_pixels: Vec<u8>,
    popup_size: (u32, u32),
    base_size: (u32, u32),
}
impl Default for View {
    fn default() -> Self {
        Self {
            browser: None,
            width: 1024,
            height: 768,
            scale: 1.0,
            visible: false,
            popup: None,
            frontend_ws: None,
            base: vec![],
            popup_pixels: vec![],
            popup_size: (0, 0),
            base_size: (0, 0),
        }
    }
}
#[derive(Default)]
struct Page {
    url: String,
    title: String,
    loading: bool,
    back: bool,
    forward: bool,
    error: Option<String>,
}
struct Subscription {
    epoch: u64,
    alive: Arc<AtomicBool>,
    _registration: Registration,
    pending: Arc<Mutex<BTreeMap<i64, i64>>>,
}
struct State {
    cdp: BTreeMap<BrowserId, Subscription>,
    next_cdp_id: i64,
    assets_access: frontend_assets::Registry,
    views: BTreeMap<Key, View>,
    pages: BTreeMap<BrowserId, Page>,
    sequences: BTreeMap<Key, u64>,
    out: Arc<Output>,
    closing: bool,
}
fn emit_state(state: &Local, id: BrowserId) {
    let s = state.borrow();
    if let Some(p) = s.pages.get(&id) {
        s.out.send(
            Event::State {
                id,
                url: p.url.clone(),
                title: p.title.clone(),
                loading: p.loading,
                can_go_back: p.back,
                can_go_forward: p.forward,
                error: p.error.clone(),
            },
            vec![],
        )
    }
}
fn frontend_diagnostic(text: &str) -> String {
    let mut output = String::new();
    for word in text.split_whitespace() {
        if !output.is_empty() {
            output.push(' ');
        }
        if word.contains("ws://") || word.contains("wss://") || word.contains("ws=") {
            output.push_str("[private endpoint]");
            continue;
        }
        // Query strings and long hexadecimal capabilities are never logged.
        let word = word.split('?').next().unwrap_or_default();
        let mut run = String::new();
        for c in word.chars().chain(std::iter::once(' ')) {
            if c.is_ascii_hexdigit() {
                run.push(c);
            } else {
                if run.len() >= 64 {
                    output.push_str("[secret]");
                } else {
                    output.push_str(&run);
                }
                run.clear();
                if c != ' ' {
                    output.push(c);
                }
            }
        }
        if output.len() > 2048 {
            break;
        }
    }
    output
}
fn bounded(text: &CefString, limit: usize) -> String {
    let mut out = String::new();
    if let Some(units) = text.as_slice() {
        for character in char::decode_utf16(units.iter().copied()) {
            let c = character.unwrap_or(char::REPLACEMENT_CHARACTER);
            if out.len() + c.len_utf8() > limit {
                break;
            }
            out.push(c);
        }
    }
    out
}
fn error(state: &Local, id: Option<BrowserId>, message: impl Into<String>) {
    state.borrow().out.send(
        Event::Error {
            id,
            message: message.into(),
            fatal: false,
        },
        vec![],
    )
}
fn browser(state: &Local, key: Key) -> Option<Browser> {
    state.borrow().views.get(&key)?.browser.clone()
}
fn frontend_loopback_permission(
    surface: Surface,
    origin: &str,
    permissions: u32,
    has_endpoint: bool,
) -> bool {
    if surface != Surface::DevTools
        || !has_endpoint
        || permissions != PermissionRequestTypes::LOOPBACK_NETWORK.get_raw()
    {
        return false;
    }
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    url.scheme() == "codisseu-devtools"
        && url.host_str() == Some("frontend")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.path().is_empty() || url.path() == "/")
        && url.query().is_none()
        && url.fragment().is_none()
}
fn frontend_resource(url: &str, ws: Option<&str>) -> bool {
    url.starts_with("codisseu-devtools://frontend/bundled/")
        || url.starts_with("blob:codisseu-devtools://frontend/")
        || url.starts_with("data:")
        || ws == Some(url)
}
fn allowed(url: &str) -> bool {
    url == "about:blank" || url.starts_with("http://") || url.starts_with("https://")
}
// CefApp::GetBrowserProcessHandler is explicitly multi-threaded (cef_app.h).
// Only the ready signal crosses that getter; actual State stays UI-local.
wrap_app! {struct EngineApp{ready:Arc<AtomicBool>}impl App{
 fn on_before_command_line_processing(&self,kind:Option<&CefString>,line:Option<&mut CommandLine>){
  if kind.is_some_and(|k|!k.to_string().is_empty()){return}
 if let Some(line)=line{
   let program=CefString::from(&line.program());line.reset();line.set_program(Some(&program));
   for switch in ["no-startup-window","disable-background-networking","disable-component-update","disable-sync","no-default-browser-check"]{line.append_switch(Some(&switch.into()));}
   line.append_switch_with_value(Some(&"ozone-platform".into()),Some(&"x11".into()));
  }
 }
 fn on_register_custom_schemes(&self,registrar:Option<&mut SchemeRegistrar>){if let Some(registrar)=registrar{frontend_assets::register(registrar);}}
 fn render_process_handler(&self)->Option<RenderProcessHandler>{Some(RendererLabel::new())}
 fn browser_process_handler(&self)->Option<BrowserProcessHandler>{Some(ProcessHandler::new(self.ready.clone()))}
}}
wrap_render_process_handler! {struct RendererLabel;impl RenderProcessHandler{fn on_web_kit_initialized(&self){
 // A truthful process label also lets the acceptance collector distinguish a
 // renderer fork from its inherited zygote argv without weakening its sandbox.
 unsafe{libc::prctl(libc::PR_SET_NAME,c"CodisseuRender".as_ptr(),0,0,0);}
}}}
wrap_browser_process_handler! {struct ProcessHandler{ready:Arc<AtomicBool>}impl BrowserProcessHandler{
 fn on_context_initialized(&self){self.ready.store(true,Ordering::Release);}
 fn on_before_child_process_launch(&self,line:Option<&mut CommandLine>){
  // CEF may otherwise choose /proc/self/exe, which names the dynamic loader
  // when the portable broker was started through its private glibc loader.
  // This callback can run on IO: it never reads/clones the UI-local state.
  if let(Some(line),Some(program))=(line,std::env::var_os("CODISSEU_BROWSER_EXECUTABLE")){line.set_program(Some(&program.to_string_lossy().as_ref().into()));}
 }
}}
fn composited(view: &View) -> Vec<u8> {
    let mut output = view.base.clone();
    let Some(rect) = view.popup.as_ref() else {
        return output;
    };
    let x = (rect.x as f32 * view.scale).round() as i32;
    let y = (rect.y as f32 * view.scale).round() as i32;
    let (w, h) = view.base_size;
    let (pw, ph) = view.popup_size;
    let begin_x = x.max(0);
    let end_x = (x + pw as i32).min(w as i32);
    if begin_x >= end_x {
        return output;
    }
    for row in 0..ph as i32 {
        let target_y = y + row;
        if target_y < 0 || target_y >= h as i32 {
            continue;
        }
        let to = ((target_y as u32 * w + begin_x as u32) * 4) as usize;
        let from = ((row as u32 * pw + (begin_x - x) as u32) * 4) as usize;
        let length = (end_x - begin_x) as usize * 4;
        if to + length <= output.len() && from + length <= view.popup_pixels.len() {
            output[to..to + length].copy_from_slice(&view.popup_pixels[from..from + length]);
        }
    }
    output
}
wrap_render_handler! {struct Paint{identity:ViewIdentity}impl RenderHandler{
 fn view_rect(&self,_browser:Option<&mut Browser>,rect:Option<&mut Rect>){let Some(state)=callback_state()else{return ;};
  if let(Some(rect),Some(view))=(rect,state.borrow().views.get(&(self.identity.id,self.identity.surface))){rect.width=view.width as i32;rect.height=view.height as i32;}
 }
 fn screen_info(&self,_browser:Option<&mut Browser>,info:Option<&mut ScreenInfo>)->i32{let Some(state)=callback_state()else{return 0;};
  if let(Some(info),Some(view))=(info,state.borrow().views.get(&(self.identity.id,self.identity.surface))){info.device_scale_factor=view.scale;info.rect=Rect{x:0,y:0,width:view.width as i32,height:view.height as i32};info.available_rect=info.rect.clone();return 1}0
 }
 fn on_popup_show(&self,_browser:Option<&mut Browser>,show:i32){let Some(state)=callback_state()else{return ;};
  if show==0{if let Some(v)=state.borrow_mut().views.get_mut(&(self.identity.id,self.identity.surface)){v.popup=None;v.popup_pixels=Vec::new();}
 if let Some(h)=host(&state,(self.identity.id,self.identity.surface)){h.invalidate(PaintElementType::VIEW);}}
 }
 fn on_popup_size(&self,_browser:Option<&mut Browser>,rect:Option<&Rect>){let Some(state)=callback_state()else{return ;};
  if let Some(v)=state.borrow_mut().views.get_mut(&(self.identity.id,self.identity.surface)){v.popup=rect.cloned();v.popup_pixels=Vec::new();}
 }
 fn on_paint(&self,_browser:Option<&mut Browser>,kind:PaintElementType,_dirty:Option<&[Rect]>,buffer:*const u8,width:i32,height:i32){let Some(state)=callback_state()else{return ;};
  if width<=0||height<=0||buffer.is_null(){return}let Ok(bytes)=protocol::frame_bytes(width as u32,height as u32)else{return};
  // SAFETY: CEF owns exactly width*height BGRA pixels for this callback only.
  let pixels=unsafe{std::slice::from_raw_parts(buffer,bytes)};
  let packet={let mut s=state.borrow_mut();let out=s.out.clone();let sequence=s.sequences.entry((self.identity.id,self.identity.surface)).or_default();*sequence=sequence.saturating_add(1);let sequence=*sequence;
   let Some(v)=s.views.get_mut(&(self.identity.id,self.identity.surface))else{return};if !v.visible{return}
   if kind==PaintElementType::VIEW{v.base=pixels.to_vec();v.base_size=(width as u32,height as u32);}
   else if kind==PaintElementType::POPUP{v.popup_pixels=pixels.to_vec();v.popup_size=(width as u32,height as u32);}else{return}
   if v.base.is_empty(){return}(out,Event::Frame{id:self.identity.id,surface:self.identity.surface,width:v.base_size.0,height:v.base_size.1,seq:sequence},composited(v))
  };packet.0.send(packet.1,packet.2);
 }
}}
wrap_display_handler! {struct Display{identity:ViewIdentity}impl DisplayHandler{
 fn on_console_message(&self,_browser:Option<&mut Browser>,level:LogSeverity,message:Option<&CefString>,_source:Option<&CefString>,_line:i32)->i32{if self.identity.surface==Surface::DevTools{if matches!(level,LogSeverity::ERROR|LogSeverity::WARNING|LogSeverity::FATAL)&&let Some(message)=message{eprintln!("DevTools frontend: {}",frontend_diagnostic(&bounded(message,4096)));}return 1;}0}
 fn on_address_change(&self,_browser:Option<&mut Browser>,frame:Option<&mut Frame>,url:Option<&CefString>){let Some(state)=callback_state()else{return ;};if self.identity.surface==Surface::Page&&frame.is_some_and(|f|f.is_main()!=0){if let Some(p)=state.borrow_mut().pages.get_mut(&self.identity.id){p.url=url.map(|s|bounded(s,16384)).unwrap_or_default();}emit_state(&state,self.identity.id);}}
 fn on_title_change(&self,_browser:Option<&mut Browser>,title:Option<&CefString>){let Some(state)=callback_state()else{return ;};if self.identity.surface==Surface::Page{if let Some(p)=state.borrow_mut().pages.get_mut(&self.identity.id){p.title=title.map(|s|bounded(s,4096)).unwrap_or_default();}emit_state(&state,self.identity.id);}}
}}
wrap_load_handler! {struct Load{identity:ViewIdentity}impl LoadHandler{
 fn on_loading_state_change(&self,_browser:Option<&mut Browser>,loading:i32,back:i32,forward:i32){let Some(state)=callback_state()else{return ;};
 if self.identity.surface==Surface::Page{if let Some(p)=state.borrow_mut().pages.get_mut(&self.identity.id){p.loading=loading!=0;p.back=back!=0;p.forward=forward!=0;if loading!=0{p.error=None;}}emit_state(&state,self.identity.id);}}
 fn on_load_error(&self,_browser:Option<&mut Browser>,frame:Option<&mut Frame>,code:Errorcode,text:Option<&CefString>,_url:Option<&CefString>){let Some(state)=callback_state()else{return ;};if self.identity.surface==Surface::DevTools&&frame.as_ref().is_some_and(|f|f.is_main()!=0)&&code!=Errorcode::ABORTED{error(&state,Some(self.identity.id),format!("Frontend DevTools não carregou ({code:?})"));}
 if self.identity.surface==Surface::Page&&frame.is_some_and(|f|f.is_main()!=0)&&code!=Errorcode::ABORTED{if let Some(p)=state.borrow_mut().pages.get_mut(&self.identity.id){p.error=Some(format!("{} ({code:?})",text.map(|s|bounded(s,8000)).unwrap_or_default()));p.loading=false;}emit_state(&state,self.identity.id);}}
}}
wrap_life_span_handler! {struct Life{identity:ViewIdentity}impl LifeSpanHandler{
 fn on_after_created(&self,browser:Option<&mut Browser>){let Some(state)=callback_state()else{return ;};if let Some(b)=browser{if self.identity.surface==Surface::DevTools{let s=state.borrow();if let Some(ws)=s.views.get(&(self.identity.id,self.identity.surface)).and_then(|v|v.frontend_ws.clone()){s.assets_access.lock().unwrap().insert(b.identifier(),ws);}}
 if let Some(v)=state.borrow_mut().views.get_mut(&(self.identity.id,self.identity.surface)){v.browser=Some(b.clone());}
 if let Some(host)=b.host(){let visible=state.borrow().views.get(&(self.identity.id,self.identity.surface)).is_some_and(|v|v.visible);host.was_hidden(i32::from(!visible));}}}
 fn on_before_close(&self,_browser:Option<&mut Browser>){let Some(state)=callback_state()else{return ;};if self.identity.surface==Surface::DevTools&&let Some(b)=_browser{state.borrow().assets_access.lock().unwrap().remove(&b.identifier());}
 if self.identity.surface==Surface::Page{if let Some(h)=host(&state,(self.identity.id,Surface::DevTools)){h.close_browser(1);}else{state.borrow_mut().views.remove(&(self.identity.id,Surface::DevTools));}let epoch=state.borrow().cdp.get(&self.identity.id).map(|s|s.epoch);if let Some(epoch)=epoch{unsubscribe(&state,self.identity.id,epoch,Some("Página fechada".into()));}}let mut s=state.borrow_mut();s.views.remove(&(self.identity.id,self.identity.surface));s.out.forget((self.identity.id,self.identity.surface));if self.identity.surface==Surface::Page{s.pages.remove(&self.identity.id);s.out.forget_state(self.identity.id);s.out.send(Event::Closed{id:self.identity.id},vec![]);}else{s.out.send(Event::DevToolsClosed{id:self.identity.id},vec![]);}}
 fn on_before_popup(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_popup_id:i32,url:Option<&CefString>,_name:Option<&CefString>,_disposition:WindowOpenDisposition,_gesture:i32,_features:Option<&PopupFeatures>,_window:Option<&mut WindowInfo>,_client:Option<&mut Option<Client>>,_settings:Option<&mut BrowserSettings>,_extra:Option<&mut Option<DictionaryValue>>,_no_js:Option<&mut i32>)->i32{let Some(state)=callback_state()else{return 1;};if self.identity.surface==Surface::Page&&let Some(url)=url.map(ToString::to_string).filter(|u|allowed(u)){state.borrow().out.send(Event::Popup{id:self.identity.id,url},vec![]);}1}
}}
wrap_request_handler! {struct Requests{out:Arc<Output>,id:BrowserId,surface:Surface,frontend_ws:Option<String>}impl RequestHandler{
 fn resource_request_handler(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_request:Option<&mut Request>,_navigation:i32,_download:i32,_initiator:Option<&CefString>,_disable_default:Option<&mut i32>)->Option<ResourceRequestHandler>{Some(ResourcePolicy::new(self.surface,self.frontend_ws.clone()))}
 fn on_before_browse(&self,_browser:Option<&mut Browser>,frame:Option<&mut Frame>,request:Option<&mut Request>,_gesture:i32,_redirect:i32)->i32{if self.surface==Surface::DevTools{let url=request.as_ref().map(|r|CefString::from(&r.url()).to_string()).unwrap_or_default();if !url.starts_with("codisseu-devtools://frontend/bundled/"){return 1}}
 if self.surface==Surface::Page&&frame.is_some_and(|f|f.is_main()!=0){let url=request.map(|r|CefString::from(&r.url()).to_string()).unwrap_or_default();if !allowed(&url){self.out.send(Event::Error{id:Some(self.id),message:"Protocolo de navegação bloqueado".into(),fatal:false},vec![]);return 1}}0}
}}
wrap_resource_request_handler! {struct ResourcePolicy{surface:Surface,frontend_ws:Option<String>}impl ResourceRequestHandler{
 fn on_before_resource_load(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,request:Option<&mut Request>,_callback:Option<&mut Callback>)->ReturnValue{
  if self.surface==Surface::DevTools{let url=request.map(|r|CefString::from(&r.url()).to_string()).unwrap_or_default();if !frontend_resource(&url,self.frontend_ws.as_deref()){return ReturnValue::CANCEL;}}ReturnValue::CONTINUE}
 fn on_protocol_execution(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_request:Option<&mut Request>,allow:Option<&mut i32>){if let Some(allow)=allow{*allow=0;}}
}}
wrap_context_menu_handler! {struct ContextMenu;impl ContextMenuHandler{fn on_before_context_menu(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_params:Option<&mut ContextMenuParams>,model:Option<&mut MenuModel>){if let Some(model)=model{model.clear();}}}}
wrap_download_handler! {struct Downloads;impl DownloadHandler{fn can_download(&self,_browser:Option<&mut Browser>,_url:Option<&CefString>,_method:Option<&CefString>)->i32{0}fn on_before_download(&self,_browser:Option<&mut Browser>,_item:Option<&mut DownloadItem>,_name:Option<&CefString>,_callback:Option<&mut BeforeDownloadCallback>)->i32{1}fn on_download_updated(&self,_browser:Option<&mut Browser>,_item:Option<&mut DownloadItem>,callback:Option<&mut DownloadItemCallback>){if let Some(callback)=callback{callback.cancel();}}}}

wrap_jsdialog_handler! {struct JsDialogs;impl JsdialogHandler{
 fn on_jsdialog(&self,_browser:Option<&mut Browser>,_origin:Option<&CefString>,_kind:JsdialogType,_message:Option<&CefString>,_default:Option<&CefString>,_callback:Option<&mut JsdialogCallback>,suppress:Option<&mut i32>)->i32{if let Some(suppress)=suppress{*suppress=1;}0}
 fn on_before_unload_dialog(&self,_browser:Option<&mut Browser>,_message:Option<&CefString>,_reload:i32,callback:Option<&mut JsdialogCallback>)->i32{if let Some(callback)=callback{callback.cont(1,None);}1}
}}
wrap_dialog_handler! {struct FileDialogs;impl DialogHandler{
 fn on_file_dialog(&self,_browser:Option<&mut Browser>,_mode:FileDialogMode,_title:Option<&CefString>,_path:Option<&CefString>,_filters:Option<&mut CefStringList>,_extensions:Option<&mut CefStringList>,_descriptions:Option<&mut CefStringList>,callback:Option<&mut FileDialogCallback>)->i32{if let Some(callback)=callback{callback.cancel();}1}
}}
wrap_permission_handler! {struct Permissions{surface:Surface,has_endpoint:bool}impl PermissionHandler{
 fn on_request_media_access_permission(&self,_browser:Option<&mut Browser>,_frame:Option<&mut Frame>,_origin:Option<&CefString>,_permissions:u32,callback:Option<&mut MediaAccessCallback>)->i32{if let Some(callback)=callback{callback.cont(0);}1}
 fn on_show_permission_prompt(&self,_browser:Option<&mut Browser>,_prompt:u64,origin:Option<&CefString>,permissions:u32,callback:Option<&mut PermissionPromptCallback>)->i32{
  let allow=origin.is_some_and(|o|frontend_loopback_permission(self.surface,&o.to_string(),permissions,self.has_endpoint));
  if let Some(callback)=callback{callback.cont(if allow{PermissionRequestResult::ACCEPT}else{PermissionRequestResult::DENY});}1
 }
}}

wrap_client! {struct ClientHandlers{data:ClientData}impl Client{
 fn render_handler(&self)->Option<RenderHandler>{Some(Paint::new(self.data.identity))}
 fn display_handler(&self)->Option<DisplayHandler>{Some(Display::new(self.data.identity))}
 fn load_handler(&self)->Option<LoadHandler>{Some(Load::new(self.data.identity))}
 fn life_span_handler(&self)->Option<LifeSpanHandler>{Some(Life::new(self.data.identity))}
 fn request_handler(&self)->Option<RequestHandler>{Some(Requests::new(self.data.out.clone(),self.data.identity.id,self.data.identity.surface,self.data.frontend_ws.clone()))}
 fn jsdialog_handler(&self)->Option<JsdialogHandler>{Some(JsDialogs::new())}
 fn dialog_handler(&self)->Option<DialogHandler>{Some(FileDialogs::new())}
 fn permission_handler(&self)->Option<PermissionHandler>{Some(Permissions::new(self.data.identity.surface,self.data.frontend_ws.is_some()))}
 fn context_menu_handler(&self)->Option<ContextMenuHandler>{Some(ContextMenu::new())}
 fn download_handler(&self)->Option<DownloadHandler>{Some(Downloads::new())}
}}
fn client(state: &Local, id: BrowserId, surface: Surface) -> Client {
    let (out, ws) = {
        let s = state.borrow();
        (
            s.out.clone(),
            s.views
                .get(&(id, surface))
                .and_then(|v| v.frontend_ws.clone()),
        )
    };
    ClientHandlers::new(ClientData {
        identity: ViewIdentity { id, surface },
        out,
        frontend_ws: ws,
    })
}

fn flags(m: Modifiers) -> u32 {
    let mut f = 0;
    for (enabled, value) in [
        (m.caps_lock, 1),
        (m.shift, 2),
        (m.ctrl, 4),
        (m.alt, 8),
        (m.left_button, 16),
        (m.middle_button, 32),
        (m.right_button, 64),
        (m.meta, 128),
        (m.num_lock, 256),
    ] {
        if enabled {
            f |= value
        }
    }
    f
}
fn host(state: &Local, key: Key) -> Option<BrowserHost> {
    browser(state, key)?.host()
}
fn open(state: &Local, id: BrowserId, url: String) {
    if !allowed(&url) {
        error(state, Some(id), "URL deve usar HTTP/HTTPS ou about:blank");
        return;
    }
    if state.borrow().pages.len() >= protocol::MAX_TABS || state.borrow().pages.contains_key(&id) {
        error(state, Some(id), "Limite de abas ou identificador duplicado");
        return;
    }
    {
        let mut s = state.borrow_mut();
        s.pages.insert(
            id,
            Page {
                url: url.clone(),
                ..Default::default()
            },
        );
        s.views.insert((id, Surface::Page), View::default());
    }
    let info = WindowInfo {
        windowless_rendering_enabled: 1,
        runtime_style: RuntimeStyle::ALLOY,
        shared_texture_enabled: 0,
        ..Default::default()
    };
    let settings = BrowserSettings {
        windowless_frame_rate: 30,
        ..Default::default()
    };
    let mut client = client(state, id, Surface::Page);
    if browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&url.as_str().into()),
        Some(&settings),
        None,
        None,
    )
    .is_none()
    {
        state.borrow_mut().views.remove(&(id, Surface::Page));
        state.borrow_mut().pages.remove(&id);
        error(state, Some(id), "CEF não criou a página");
    }
}
// CEF 156 ShowDevTools creates a Chrome-style native window and explicitly
// rejects windowless rendering. Host its unmodified bundled frontend instead.
fn devtools(state: &Local, id: BrowserId, opened: bool, _inspect: Option<Point>) {
    if host(state, (id, Surface::Page)).is_none() {
        return;
    }
    if !opened {
        if let Some(h) = host(state, (id, Surface::DevTools)) {
            h.close_browser(1);
        } else {
            state.borrow_mut().views.remove(&(id, Surface::DevTools));
            state
                .borrow()
                .out
                .send(Event::DevToolsClosed { id }, vec![]);
        }
        return;
    }
    state
        .borrow_mut()
        .views
        .entry((id, Surface::DevTools))
        .or_insert_with(|| View {
            visible: true,
            ..Default::default()
        });
}
fn frontend(state: &Local, id: BrowserId, url: &str) {
    if host(state, (id, Surface::Page)).is_none() {
        return;
    }
    devtools(state, id, true, None);
    // Protocol SecretUrl validates the exact bundled document and nonce endpoint.
    let Some(endpoint) = url.split_once("&ws=").map(|(_, ws)| format!("ws://{ws}")) else {
        return;
    };
    if let Some(h) = host(state, (id, Surface::DevTools)) {
        h.close_browser(1);
        error(
            state,
            Some(id),
            "Feche o DevTools anterior antes de reabrir",
        );
        return;
    }
    if let Some(v) = state.borrow_mut().views.get_mut(&(id, Surface::DevTools)) {
        v.frontend_ws = Some(endpoint);
    }
    let url = url
        .replacen("devtools://devtools", frontend_assets::ORIGIN, 1)
        .replacen("?can_dock=false&ws=", "?ws=", 1);
    let info = WindowInfo {
        windowless_rendering_enabled: 1,
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    };
    let settings = BrowserSettings {
        windowless_frame_rate: 30,
        ..Default::default()
    };
    let mut client = client(state, id, Surface::DevTools);
    if browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&url.as_str().into()),
        Some(&settings),
        None,
        None,
    )
    .is_none()
    {
        state.borrow_mut().views.remove(&(id, Surface::DevTools));
        error(state, Some(id), "CEF não criou o frontend DevTools");
    }
}

fn response_for_controller(
    bytes: &[u8],
    pending: &Mutex<BTreeMap<i64, i64>>,
) -> Result<Option<Vec<u8>>, String> {
    if bytes.len() > protocol::MAX_CDP {
        return Err("Resposta CDP excede limite".into());
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "Resposta CDP inválida")?;
    if !value.is_object() {
        return Err("Resposta CDP inválida".into());
    }
    if let Some(wire) = value.get("id") {
        let Some(wire) = wire.as_i64() else {
            return Ok(None);
        };
        let Some(original) = pending.lock().unwrap().remove(&wire) else {
            return Ok(None);
        };
        value["id"] = original.into();
    }
    let encoded = serde_json::to_vec(&value).map_err(|_| "Resposta CDP inválida")?;
    if encoded.len() > protocol::MAX_CDP {
        return Err("Resposta CDP excede limite".into());
    }
    Ok(Some(encoded))
}
wrap_dev_tools_message_observer! {struct ProtocolObserver{out:Arc<Output>,id:BrowserId,epoch:u64,alive:Arc<AtomicBool>,pending:Arc<Mutex<BTreeMap<i64,i64>>>}impl DevToolsMessageObserver{
 fn on_dev_tools_message(&self,_browser:Option<&mut Browser>,message:Option<&[u8]>)->i32{
  if !self.alive.load(Ordering::Relaxed){return 1;}
  if let Some(bytes)=message{match response_for_controller(bytes,&self.pending){
   Ok(Some(bytes))=>{if !self.out.cdp(self.id,self.epoch,bytes){self.alive.store(false,Ordering::Relaxed);self.out.send(Event::DevToolsDetached{id:self.id,epoch:self.epoch,error:Some("Fila CDP excedeu o limite".into())},vec![]);}},Ok(None)=>{},
   Err(error)=>{self.alive.store(false,Ordering::Relaxed);self.out.send(Event::DevToolsDetached{id:self.id,epoch:self.epoch,error:Some(error)},vec![]);}
  }}1
 }
 fn on_dev_tools_agent_detached(&self,_browser:Option<&mut Browser>){if self.alive.swap(false,Ordering::Relaxed){self.out.send(Event::DevToolsDetached{id:self.id,epoch:self.epoch,error:Some("CEF desconectou o target".into())},vec![]);}}
}}

fn controller_cleanup_messages(epoch: u64) -> [Vec<u8>; 5] {
    let group = format!("codisseu-native-{epoch}");
    [
        serde_json::json!({"id":CDP_RESERVED_ID_START,"method":"Overlay.hideHighlight"}),
        serde_json::json!({"id":CDP_RESERVED_ID_START+1,"method":"Runtime.releaseObjectGroup","params":{"objectGroup":group}}),
        serde_json::json!({"id":CDP_RESERVED_ID_START+2,"method":"Runtime.releaseObjectGroup","params":{"objectGroup":format!("{group}-dom")}}),
        serde_json::json!({"id":CDP_RESERVED_ID_START+3,"method":"Debugger.resume"}),
        serde_json::json!({"id":CDP_RESERVED_ID_START+4,"method":"Debugger.disable"}),
    ].map(|v| serde_json::to_vec(&v).expect("fixed controller cleanup message"))
}
fn unsubscribe(state: &Local, id: BrowserId, epoch: u64, reason: Option<String>) {
    if !state
        .borrow()
        .cdp
        .get(&id)
        .is_some_and(|s| s.epoch == epoch)
    {
        return;
    }
    let sub = state.borrow_mut().cdp.remove(&id).unwrap();
    sub.alive.store(false, Ordering::Relaxed);
    if let Some(h) = host(state, (id, Surface::Page)) {
        for message in controller_cleanup_messages(epoch) {
            h.send_dev_tools_message(Some(&message));
        }
    }
    drop(sub);
    state.borrow().out.send(
        Event::DevToolsDetached {
            id,
            epoch,
            error: reason,
        },
        vec![],
    );
}
fn subscribe(state: &Local, id: BrowserId, epoch: u64) {
    if state
        .borrow()
        .cdp
        .get(&id)
        .is_some_and(|s| s.epoch == epoch && s.alive.load(Ordering::Relaxed))
    {
        return;
    }
    let old = state.borrow().cdp.get(&id).map(|s| s.epoch);
    if let Some(old) = old {
        unsubscribe(state, id, old, None);
    }
    let Some(h) = host(state, (id, Surface::Page)) else {
        state.borrow().out.send(
            Event::DevToolsDetached {
                id,
                epoch,
                error: Some("Página indisponível".into()),
            },
            vec![],
        );
        return;
    };
    let alive = Arc::new(AtomicBool::new(true));
    let pending = Arc::new(Mutex::new(BTreeMap::new()));
    let mut observer = ProtocolObserver::new(
        state.borrow().out.clone(),
        id,
        epoch,
        alive.clone(),
        pending.clone(),
    );
    if let Some(registration) = h.add_dev_tools_message_observer(Some(&mut observer)) {
        state.borrow_mut().cdp.insert(
            id,
            Subscription {
                epoch,
                alive,
                _registration: registration,
                pending,
            },
        );
    } else {
        state.borrow().out.send(
            Event::DevToolsDetached {
                id,
                epoch,
                error: Some("CEF recusou observador CDP".into()),
            },
            vec![],
        );
    }
}

fn send_cdp(state: &Local, id: BrowserId, epoch: u64, payload: &[u8]) {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(payload) else {
        return;
    };
    let Some(original) = value
        .get("id")
        .and_then(|v| v.as_i64())
        .filter(|id| *id >= 0)
    else {
        error(state, Some(id), "Comando CDP sem identificador inteiro");
        return;
    };
    let (wire, pending) = {
        let mut s = state.borrow_mut();
        let Some(sub) = s
            .cdp
            .get(&id)
            .filter(|sub| sub.epoch == epoch && sub.alive.load(Ordering::Relaxed))
        else {
            return;
        };
        let pending = sub.pending.clone();
        if pending.lock().unwrap().len() >= 4096 || s.next_cdp_id >= CDP_RESERVED_ID_START {
            drop(s);
            unsubscribe(
                state,
                id,
                epoch,
                Some("Limite de comandos CDP pendentes".into()),
            );
            return;
        }
        let wire = s.next_cdp_id;
        s.next_cdp_id += 1;
        (wire, pending)
    };
    value["id"] = wire.into();
    let Ok(bytes) = serde_json::to_vec(&value) else {
        return;
    };
    if bytes.len() > protocol::MAX_CDP {
        return;
    }
    pending.lock().unwrap().insert(wire, original);
    if host(state, (id, Surface::Page)).is_none_or(|h| h.send_dev_tools_message(Some(&bytes)) == 0)
    {
        pending.lock().unwrap().remove(&wire);
        error(state, Some(id), "CEF recusou comando CDP");
    }
}

fn handle(state: &Local, input: Input, payload: &[u8]) {
    let Some(id) = input.id() else {
        let controllers = state
            .borrow()
            .cdp
            .iter()
            .map(|(id, s)| (*id, s.epoch))
            .collect::<Vec<_>>();
        for (id, epoch) in controllers {
            unsubscribe(state, id, epoch, None);
        }
        state.borrow_mut().closing = true;
        state.borrow_mut().views.retain(|_, v| v.browser.is_some());
        let browsers = state
            .borrow()
            .views
            .values()
            .filter_map(|v| v.browser.clone())
            .collect::<Vec<_>>();
        for b in browsers {
            if let Some(h) = b.host() {
                h.close_browser(1);
            }
        }
        return;
    };
    match input {
        Input::Open { id, url } => open(state, id, url),
        Input::Close { id } => {
            if let Some(h) = host(state, (id, Surface::Page)) {
                h.close_browser(1);
            }
        }
        Input::Navigate { id, url } => {
            if !allowed(&url) {
                error(state, Some(id), "Protocolo de URL bloqueado");
            } else if let Some(f) = browser(state, (id, Surface::Page)).and_then(|b| b.main_frame())
            {
                f.load_url(Some(&url.as_str().into()));
            }
        }
        Input::Back { .. } => {
            if let Some(b) = browser(state, (id, Surface::Page)) {
                b.go_back();
            }
        }
        Input::Forward { .. } => {
            if let Some(b) = browser(state, (id, Surface::Page)) {
                b.go_forward();
            }
        }
        Input::Reload { ignore_cache, .. } => {
            if let Some(b) = browser(state, (id, Surface::Page)) {
                if ignore_cache {
                    b.reload_ignore_cache();
                } else {
                    b.reload();
                }
            }
        }
        Input::Stop { .. } => {
            if let Some(b) = browser(state, (id, Surface::Page)) {
                b.stop_load();
            }
        }
        Input::Resize {
            surface,
            width,
            height,
            scale,
            ..
        } => {
            if width == 0
                || height == 0
                || !scale.is_finite()
                || !(0.5..=4.).contains(&scale)
                || protocol::frame_bytes(
                    (width as f32 * scale).ceil() as u32,
                    (height as f32 * scale).ceil() as u32,
                )
                .is_err()
            {
                error(state, Some(id), "Dimensões da página excedem o limite");
                return;
            }
            if let Some(v) = state.borrow_mut().views.get_mut(&(id, surface)) {
                v.width = width;
                v.height = height;
                v.scale = scale;
                v.base.clear();
            }
            if let Some(h) = host(state, (id, surface)) {
                h.notify_screen_info_changed();
                h.was_resized();
            }
        }
        Input::Visible {
            surface, visible, ..
        } => {
            if let Some(v) = state.borrow_mut().views.get_mut(&(id, surface)) {
                v.visible = visible;
                if !visible {
                    v.base = Vec::new();
                    v.popup_pixels = Vec::new();
                }
            }
            if let Some(h) = host(state, (id, surface)) {
                h.was_hidden(i32::from(!visible));
                if !visible {
                    state.borrow().out.forget((id, surface));
                }
                if visible {
                    h.invalidate(PaintElementType::VIEW);
                }
            }
        }
        Input::Focus {
            surface, focused, ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                h.set_focus(i32::from(focused));
                if !focused {
                    h.send_capture_lost_event();
                }
            }
        }
        Input::MouseMove {
            surface,
            x,
            y,
            leave,
            modifiers,
            ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                h.send_mouse_move_event(
                    Some(&MouseEvent {
                        x,
                        y,
                        modifiers: flags(modifiers),
                    }),
                    i32::from(leave),
                );
            }
        }
        Input::MouseClick {
            surface,
            x,
            y,
            button,
            up,
            clicks,
            modifiers,
            ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                h.send_mouse_click_event(
                    Some(&MouseEvent {
                        x,
                        y,
                        modifiers: flags(modifiers),
                    }),
                    match button {
                        MouseButton::Left => MouseButtonType::LEFT,
                        MouseButton::Middle => MouseButtonType::MIDDLE,
                        MouseButton::Right => MouseButtonType::RIGHT,
                    },
                    i32::from(up),
                    clicks.clamp(1, 3) as i32,
                );
            }
        }
        Input::MouseWheel {
            surface,
            x,
            y,
            delta_x,
            delta_y,
            modifiers,
            ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                h.send_mouse_wheel_event(
                    Some(&MouseEvent {
                        x,
                        y,
                        modifiers: flags(modifiers),
                    }),
                    delta_x,
                    delta_y,
                );
            }
        }
        Input::Key {
            surface,
            kind,
            windows_key_code,
            native_key_code,
            modifiers,
            character,
            unmodified,
            ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                h.send_key_event(Some(&KeyEvent {
                    type_: match kind {
                        KeyKind::RawDown => KeyEventType::RAWKEYDOWN,
                        KeyKind::Up => KeyEventType::KEYUP,
                        KeyKind::Char => KeyEventType::CHAR,
                    },
                    windows_key_code,
                    native_key_code,
                    modifiers: flags(modifiers),
                    character: character
                        .filter(|c| (*c as u32) <= 0xffff)
                        .map(|c| c as u16)
                        .unwrap_or(0),
                    unmodified_character: unmodified
                        .filter(|c| (*c as u32) <= 0xffff)
                        .map(|c| c as u16)
                        .unwrap_or(0),
                    ..Default::default()
                }));
            }
        }
        Input::Text { surface, text, .. } => {
            if text.len() > 65536 {
                error(state, Some(id), "Texto excede o limite");
                return;
            }
            if let Some(h) = host(state, (id, surface)) {
                h.ime_commit_text(
                    Some(&text.as_str().into()),
                    Some(&Range {
                        from: u32::MAX,
                        to: u32::MAX,
                    }),
                    0,
                );
            }
        }
        Input::Composition {
            surface,
            text,
            selection_start,
            selection_end,
            ..
        } => {
            if let Some(h) = host(state, (id, surface)) {
                if text.is_empty() {
                    h.ime_cancel_composition();
                } else {
                    h.ime_set_composition(
                        Some(&text.as_str().into()),
                        None,
                        Some(&Range {
                            from: u32::MAX,
                            to: u32::MAX,
                        }),
                        Some(&Range {
                            from: selection_start as u32,
                            to: selection_end as u32,
                        }),
                    );
                }
            }
        }
        Input::DevToolsSubscribe { epoch, .. } => subscribe(state, id, epoch),
        Input::DevToolsUnsubscribe { epoch, .. } => unsubscribe(state, id, epoch, None),
        Input::DevToolsMessage { epoch, .. } => send_cdp(state, id, epoch, payload),
        Input::DevToolsFrontend { url, .. } => frontend(state, id, url.as_str()),
        Input::DevTools { open, .. } => devtools(state, id, open, None),
        Input::Inspect { x, y, .. } => devtools(state, id, true, Some(Point { x, y })),
        Input::Clipboard {
            surface, action, ..
        } => {
            if let Some(f) = browser(state, (id, surface)).and_then(|b| b.focused_frame()) {
                match action {
                    ClipboardAction::Copy => f.copy(),
                    ClipboardAction::Cut => f.cut(),
                    ClipboardAction::Paste => {
                        error(state, Some(id), "Colar requer texto literal do aplicativo")
                    }
                    ClipboardAction::SelectAll => f.select_all(),
                }
            }
        }
        _ => error(state, Some(id), "Comando do navegador ainda não suportado"),
    }
}
thread_local! {
 static UI_STATE:RefCell<Option<Local>>=const{RefCell::new(None)};
 static UI_PUMP:RefCell<Option<LocalRc<RefCell<EngineLoop>>>>=const{RefCell::new(None)};
}
fn callback_state() -> Option<Local> {
    if currently_on(ThreadId::UI) == 0 {
        return None;
    }
    UI_STATE.with(|state| state.borrow().clone())
}
fn callback_pump() -> Option<LocalRc<RefCell<EngineLoop>>> {
    if currently_on(ThreadId::UI) == 0 {
        return None;
    }
    UI_PUMP.with(|pump| pump.borrow().clone())
}
struct EngineLoop {
    state: Local,
    ready: Arc<AtomicBool>,
    rx: mpsc::Receiver<protocol::Packet<Input>>,
    out: Arc<Output>,
    parent: i32,
    start: Instant,
    announced: bool,
    closing_at: Option<Instant>,
    error: Option<String>,
}
impl EngineLoop {
    fn tick(&mut self) -> bool {
        let detached = self
            .state
            .borrow()
            .cdp
            .iter()
            .filter(|(_, s)| !s.alive.load(Ordering::Relaxed))
            .map(|(id, s)| (*id, s.epoch))
            .collect::<Vec<_>>();
        for (id, epoch) in detached {
            unsubscribe(
                &self.state,
                id,
                epoch,
                Some("Controlador CDP desconectado".into()),
            );
        }
        if self.ready.load(Ordering::Acquire) {
            if !self.announced {
                self.out.send(
                    Event::Ready {
                        protocol: protocol::VERSION,
                        versions: protocol::EngineVersions {
                            cef: "156.0.15+gcf671c0".into(),
                            chromium: "156.0.8078.80".into(),
                        },
                    },
                    vec![],
                );
                self.announced = true;
            }
            for _ in 0..128 {
                if self.state.borrow().closing {
                    break;
                }
                match self.rx.try_recv() {
                    Ok(packet) => handle(&self.state, packet.message, &packet.payload),
                    Err(_) => break,
                }
            }
        } else if self.start.elapsed() > Duration::from_secs(20) {
            self.error = Some("CEF initialization deadline exceeded".into());
            return false;
        }
        if (self.out.dead.load(Ordering::Relaxed) || unsafe { libc::getppid() } != self.parent)
            && !self.state.borrow().closing
        {
            handle(&self.state, Input::Shutdown, &[]);
        }
        if self.state.borrow().closing {
            if self.closing_at.is_none() {
                self.closing_at = Some(Instant::now());
            }
            if self.state.borrow().views.is_empty() {
                return false;
            }
            if self
                .closing_at
                .is_some_and(|t| t.elapsed() > Duration::from_secs(5))
            {
                self.error = Some("CEF shutdown deadline exceeded".into());
                return false;
            }
        }
        true
    }
}
wrap_task! {struct IpcTask;impl Task{fn execute(&self){
 let Some(pump)=callback_pump()else{return;};
 let again=pump.borrow_mut().tick();
 if again{let mut next=IpcTask::new();if post_delayed_task(ThreadId::UI,Some(&mut next),8)==0{pump.borrow_mut().error=Some("CEF rejected IPC scheduling".into());quit_message_loop();}}
 else{quit_message_loop();}
}}}

unsafe extern "C" {
    fn cef_version_info(entry: i32) -> i32;
}
fn runtime_version() -> Result<(), String> {
    let expected = [156, 0, 15, 3660, 156, 0, 8078, 80];
    for (entry, expected) in expected.into_iter().enumerate() {
        // SAFETY: read-only stable CEF version API from the SDK linked by this process.
        if unsafe { cef_version_info(entry as i32) } != expected {
            return Err("Private CEF version does not match the pinned engine".into());
        }
    }
    Ok(())
}
fn verify_peer(socket: &UnixStream, parent: i32) -> Result<(), String> {
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut size = std::mem::size_of_val(&peer) as libc::socklen_t;
    // SAFETY: peer is a correctly-sized writable ucred and the socket is live.
    let result = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut peer as *mut libc::ucred).cast(),
            &mut size,
        )
    };
    if result != 0
        || size as usize != std::mem::size_of_val(&peer)
        || peer.pid != parent
        || peer.uid != unsafe { libc::getuid() }
    {
        return Err("Unexpected browser IPC peer".into());
    }
    Ok(())
}
fn run() -> Result<(), String> {
    runtime_version()?;
    let args = Args::new();
    let ready = Arc::new(AtomicBool::new(false));
    let mut app = EngineApp::new(ready.clone());
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let result = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if result >= 0 {
        std::process::exit(result)
    }
    let mut socket = None;
    let mut runtime = None;
    let mut profile = None;
    let mut parent = None;
    let mut argv = std::env::args_os().skip(1);
    while let Some(arg) = argv.next() {
        match arg.to_str() {
            Some("--socket" | "--ipc") => socket = argv.next().map(PathBuf::from),
            Some("--runtime") => runtime = argv.next().map(PathBuf::from),
            Some("--profile") => profile = argv.next().map(PathBuf::from),
            Some("--parent-pid") => {
                parent = argv
                    .next()
                    .and_then(|v| v.to_str().and_then(|v| v.parse::<i32>().ok()))
            }
            _ => return Err("Unknown browser engine argument".into()),
        }
    }
    let runtime = runtime
        .ok_or("Missing private CEF runtime")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let profile = profile
        .ok_or("Missing private browser profile")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let metadata = std::fs::metadata(&profile).map_err(|e| e.to_string())?;
    if !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::getuid() }
    {
        return Err("Browser profile must be private0700".into());
    }
    let parent = parent.ok_or("Missing parent PID")?;
    if parent <= 1 || unsafe { libc::getppid() } != parent {
        return Err("Unexpected browser parent PID".into());
    }
    let socket = UnixStream::connect(socket.ok_or("Missing private IPC socket")?)
        .map_err(|e| e.to_string())?;
    verify_peer(&socket, parent)?;
    let writer = socket.try_clone().map_err(|e| e.to_string())?;
    let out = Arc::new(Output::default());
    let txout = out.clone();
    thread::spawn(move || txout.writer(writer));
    let (tx, rx) = mpsc::sync_channel::<protocol::Packet<Input>>(128);
    let input_out = out.clone();
    thread::spawn(move || {
        let mut socket = socket;
        while let Ok(packet) = protocol::read_command_packet(&mut socket) {
            if tx.send(packet).is_err() {
                break;
            }
        }
        input_out.dead.store(true, Ordering::Relaxed);
    });
    let state = LocalRc::new(RefCell::new(State {
        cdp: BTreeMap::new(),
        next_cdp_id: 1,
        assets_access: Arc::new(Mutex::new(BTreeMap::new())),
        views: BTreeMap::new(),
        pages: BTreeMap::new(),
        sequences: BTreeMap::new(),
        out: out.clone(),
        closing: false,
    }));
    UI_STATE.with(|slot| *slot.borrow_mut() = Some(state.clone()));
    let executable = std::env::var_os("CODISSEU_BROWSER_EXECUTABLE")
        .or_else(|| std::env::args_os().next())
        .ok_or("Missing browser executable")?;
    let executable = PathBuf::from(executable)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let settings = Settings {
        no_sandbox: 0,
        windowless_rendering_enabled: 1,
        external_message_pump: 0,
        multi_threaded_message_loop: 0,
        browser_subprocess_path: executable.to_string_lossy().as_ref().into(),
        resources_dir_path: runtime.to_string_lossy().as_ref().into(),
        locales_dir_path: runtime.join("locales").to_string_lossy().as_ref().into(),
        root_cache_path: profile.to_string_lossy().as_ref().into(),
        remote_debugging_port: 0,
        log_file: profile.join("cef.log").to_string_lossy().as_ref().into(),
        ..Default::default()
    };
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        return Err("CEF initialization failed; sandbox is required".into());
    }
    if let Err(error) = frontend_assets::install(&runtime, state.borrow().assets_access.clone()) {
        shutdown();
        return Err(error);
    }
    let pump = LocalRc::new(RefCell::new(EngineLoop {
        state,
        ready,
        rx,
        out: out.clone(),
        parent,
        start: Instant::now(),
        announced: false,
        closing_at: None,
        error: None,
    }));
    UI_PUMP.with(|slot| *slot.borrow_mut() = Some(pump.clone()));
    let mut task = IpcTask::new();
    if post_task(ThreadId::UI, Some(&mut task)) == 0 {
        return Err("CEF rejected IPC scheduling".into());
    }
    run_message_loop();
    if let Some(error) = pump.borrow_mut().error.take() {
        return Err(error);
    }
    shutdown();
    UI_PUMP.with(|slot| {
        slot.borrow_mut().take();
    });
    UI_STATE.with(|slot| {
        slot.borrow_mut().take();
    });
    out.dead.store(true, Ordering::Relaxed);
    out.wake.notify_all();
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("Codisseu browser engine: {error}");
        std::process::exit(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_authorized_frontend_can_request_its_loopback_permission() {
        let permission = PermissionRequestTypes::LOOPBACK_NETWORK.get_raw();
        for origin in [
            "codisseu-devtools://frontend",
            "codisseu-devtools://frontend/",
        ] {
            assert!(frontend_loopback_permission(
                Surface::DevTools,
                origin,
                permission,
                true
            ));
            assert!(!frontend_loopback_permission(
                Surface::Page,
                origin,
                permission,
                true
            ));
            assert!(!frontend_loopback_permission(
                Surface::DevTools,
                origin,
                permission,
                false
            ));
            assert!(!frontend_loopback_permission(
                Surface::DevTools,
                origin,
                permission | PermissionRequestTypes::LOCAL_NETWORK.get_raw(),
                true
            ));
        }
        for origin in [
            "https://example.com/",
            "codisseu-devtools://frontend.evil/",
            "codisseu-devtools://frontend:1234/",
            "codisseu-devtools://user@frontend/",
            "codisseu-devtools://frontend/other",
            "codisseu-devtools://frontend/?query",
            "codisseu-devtools://frontend/#fragment",
        ] {
            assert!(!frontend_loopback_permission(
                Surface::DevTools,
                origin,
                permission,
                true
            ));
        }
    }

    #[test]
    fn frontend_diagnostics_redact_queries_websocket_urls_and_capabilities() {
        let token = "c".repeat(64);
        let input = format!(
            "WebSocket ws://127.0.0.1:40000/{token} failed; inspector.html?ws=127.0.0.1:40000/{token} token={token} source.js:42"
        );
        let output = frontend_diagnostic(&input);
        assert!(!output.contains(&token));
        assert!(!output.contains("127.0.0.1"));
        assert!(output.contains("source.js:42"));
    }

    #[test]
    fn cdp_responses_cannot_cross_controller_epochs_or_reused_client_ids() {
        let old = Mutex::new(BTreeMap::from([(41, 1)]));
        let new = Mutex::new(BTreeMap::from([(42, 1)]));
        let stale = br#"{"id":41,"result":{"old":true}}"#;
        assert!(response_for_controller(stale, &new).unwrap().is_none());
        assert!(response_for_controller(stale, &old).unwrap().is_some());
        let fresh = response_for_controller(br#"{"id":42,"result":{"new":true}}"#, &new)
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fresh).unwrap();
        assert_eq!(value["id"], 1);
        assert_eq!(value["result"]["new"], true);
        assert!(
            response_for_controller(br#"{"id":42,"result":{}}"#, &new)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn popup_clips_negative_origin_without_destroying_page_pixels() {
        let view = View {
            base: vec![10; 3 * 2 * 4],
            base_size: (3, 2),
            popup: Some(Rect {
                x: -1,
                y: 1,
                width: 3,
                height: 2,
            }),
            popup_pixels: [vec![20; 4], vec![30; 4], vec![40; 4], vec![50; 12]].concat(),
            popup_size: (3, 2),
            ..Default::default()
        };
        let painted = composited(&view);
        assert_eq!(&painted[..12], &[10; 12]);
        assert_eq!(
            &painted[12..],
            &[vec![30; 4], vec![40; 4], vec![10; 4]].concat()
        );
        assert!(view.base.iter().all(|pixel| *pixel == 10));
    }

    #[test]
    fn frontend_resources_are_confined_to_bundled_assets_and_its_exact_gateway() {
        let endpoint = Some("ws://127.0.0.1:45678/secret");
        assert!(frontend_resource(
            "codisseu-devtools://frontend/bundled/core/sdk/sdk.js",
            endpoint
        ));
        assert!(frontend_resource("ws://127.0.0.1:45678/secret", endpoint));
        for url in [
            "https://example.com/",
            "file:///etc/passwd",
            "ws://127.0.0.1:45678/wrong",
            "chrome://settings/",
        ] {
            assert!(!frontend_resource(url, endpoint));
        }
    }

    #[test]
    fn controller_cleanup_releases_only_its_epoch_groups_and_uses_reserved_ids() {
        let pending = Mutex::new(BTreeMap::from([(CDP_RESERVED_ID_START - 1, 7)]));
        for epoch in [1_u64, 42, u64::MAX] {
            let packets = controller_cleanup_messages(epoch);
            let values = packets
                .iter()
                .map(|p| serde_json::from_slice::<serde_json::Value>(p).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(values[0]["method"], "Overlay.hideHighlight");
            assert_eq!(values[1]["method"], "Runtime.releaseObjectGroup");
            assert_eq!(
                values[1]["params"]["objectGroup"],
                format!("codisseu-native-{epoch}")
            );
            assert_eq!(
                values[2]["params"]["objectGroup"],
                format!("codisseu-native-{epoch}-dom")
            );
            assert_eq!(values[3]["method"], "Debugger.resume");
            assert_eq!(values[4]["method"], "Debugger.disable");
            for (i, value) in values.iter().enumerate() {
                let id = CDP_RESERVED_ID_START + i as i64;
                assert_eq!(value["id"], id);
                let reply = serde_json::to_vec(&serde_json::json!({"id":id,"result":{}})).unwrap();
                assert_eq!(response_for_controller(&reply, &pending).unwrap(), None);
            }
        }
        assert_eq!(pending.lock().unwrap().len(), 1);
    }

    #[test]
    fn public_page_cdp_bursts_are_bounded_and_keep_all_control_reservations() {
        let output = Output::default();
        // More than the previous120-message ceiling, with no unbounded retention.
        for _ in 0..OUTPUT_CDP_EVENTS {
            assert!(output.cdp(1, 1, b"{}".to_vec()));
        }
        assert!(!output.cdp(1, 1, b"{}".to_vec()));
        assert!(!output.dead.load(Ordering::Relaxed));
        for _ in 0..OUTPUT_CONTROL_RESERVE {
            output.send(
                Event::DevToolsDetached {
                    id: 1,
                    epoch: 1,
                    error: Some("controlled overload".into()),
                },
                vec![],
            );
        }
        {
            let queue = output.queue.lock().unwrap();
            assert_eq!(queue.events.len(), OUTPUT_EVENTS);
            assert_eq!(queue.event_bytes, OUTPUT_CDP_EVENTS * 2);
        }
        assert!(!output.dead.load(Ordering::Relaxed));
        output.send(
            Event::Error {
                id: None,
                message: "control limit".into(),
                fatal: false,
            },
            vec![],
        );
        assert!(output.dead.load(Ordering::Relaxed));
    }

    #[test]
    fn full_byte_budget_still_allows_empty_control_and_rejects_more_cdp() {
        let output = Output::default();
        for _ in 0..OUTPUT_EVENT_BYTES / protocol::MAX_CDP {
            assert!(output.cdp(1, 1, vec![0; protocol::MAX_CDP]));
        }
        assert!(!output.cdp(1, 1, vec![0]));
        output.send(
            Event::DevToolsDetached {
                id: 1,
                epoch: 1,
                error: None,
            },
            vec![],
        );
        let queue = output.queue.lock().unwrap();
        assert_eq!(queue.event_bytes, OUTPUT_EVENT_BYTES);
        assert_eq!(queue.events.len(), 5);
        assert!(!output.dead.load(Ordering::Relaxed));
    }

    #[test]
    fn stalled_cdp_consumer_cannot_accumulate_unbounded_events() {
        let output = Output::default();
        for _ in 0..4 {
            assert!(output.cdp(1, 1, vec![0; protocol::MAX_CDP]));
        }
        assert!(!output.cdp(1, 1, vec![0; protocol::MAX_CDP]));
        assert!(!output.dead.load(Ordering::Relaxed));
        let queue = output.queue.lock().unwrap();
        assert_eq!(queue.event_bytes, 16 * 1024 * 1024);
        assert_eq!(queue.events.len(), 4);
    }
}
