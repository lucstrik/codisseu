use super::*;
use serde_json::json;
use std::{
    collections::BTreeMap,
    io::{BufReader, Read},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::TryRecvError,
    time::{Duration, Instant},
};
enum Pending {
    Initialize,
    Login,
    Finish,
    SignOut,
    Complete(Document, u64),
    Other,
}
// Explicit local QA only: metadata, never source text, paths or credentials.
fn trace(value: Value) {
    if std::env::var_os("CODISSEU_COPILOT_TRACE").is_some() {
        eprintln!("COPILOT_QA {value}");
    }
}
struct Rpc {
    child: Child,
    input: ChildStdin,
    receive: Receiver<Result<Value, String>>,
    reader: Option<std::thread::JoinHandle<()>>,
    stderr: Option<std::thread::JoinHandle<()>>,
    next: u64,
    pending: BTreeMap<u64, (Pending, Instant)>,
}
impl Rpc {
    fn start(program: PathBuf) -> Result<Self, String> {
        let mut cmd = Command::new(program);
        cmd.arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(home) = std::env::var_os("HOME") {
            cmd.current_dir(home);
        }
        cmd.env_remove("NODE_OPTIONS").env_remove("NODE_PATH");
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Não foi possível iniciar Copilot: {e}"))?;
        let input = child
            .stdin
            .take()
            .ok_or("Entrada do servidor indisponível")?;
        let output = child
            .stdout
            .take()
            .ok_or("Saída do servidor indisponível")?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or("Canal do servidor indisponível")?;
        let (tx, receive) = mpsc::sync_channel(16);
        let reader = std::thread::spawn(move || {
            let mut out = BufReader::new(output);
            loop {
                let msg = transport::read(&mut out);
                let failed = msg.is_err();
                if tx.send(msg).is_err() || failed {
                    break;
                }
            }
        });
        let stderr = std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            receive,
            reader: Some(reader),
            stderr: Some(stderr),
            next: 0,
            pending: BTreeMap::new(),
        })
    }
    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        transport::write(
            &mut self.input,
            &json!({"jsonrpc":"2.0","method":method,"params":params}),
        )
    }
    fn request(&mut self, method: &str, params: Value, kind: Pending) -> Result<(), String> {
        if self.pending.len() >= 16 {
            return Err("Fila Copilot cheia".into());
        }
        self.next += 1;
        let id = self.next;
        transport::write(
            &mut self.input,
            &json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )?;
        self.pending.insert(id, (kind, Instant::now()));
        Ok(())
    }
    fn reply(&mut self, id: Value, value: Value) -> Result<(), String> {
        transport::write(
            &mut self.input,
            &json!({"jsonrpc":"2.0","id":id,"result":value}),
        )
    }
    fn cancel(&mut self) {
        let ids = self
            .pending
            .iter()
            .filter_map(|(&id, (p, _))| matches!(p, Pending::Complete(..)).then_some(id))
            .collect::<Vec<_>>();
        for id in ids {
            self.pending.remove(&id);
            let _ = self.notify("$/cancelRequest", json!({"id":id}));
        }
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            let _ = killpg(Pid::from_raw(self.child.id() as i32), Signal::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        let (_, empty) = mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.receive, empty));
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
        if let Some(h) = self.stderr.take() {
            let _ = h.join();
        }
    }
}
fn folders(paths: &[PathBuf]) -> Value {
    Value::Array(paths.iter().filter_map(|p|file_uri(p).ok().map(|uri|json!({"uri":uri,"name":p.file_name().unwrap_or_default().to_string_lossy()}))).collect())
}
fn sync(rpc: &mut Rpc, current: &mut Option<Document>, doc: Document) -> Result<(), String> {
    if doc.text.len() > MAX_DOCUMENT || doc.text.contains('\0') {
        return Err("Copilot atende documentos de texto de até1MiB".into());
    }
    if current.as_ref().is_none_or(|old| old.uri != doc.uri) {
        if let Some(old) = current.take() {
            rpc.notify(
                "textDocument/didClose",
                json!({"textDocument":{"uri":old.uri}}),
            )?;
        }
        rpc.notify("textDocument/didOpen",json!({"textDocument":{"uri":doc.uri,"version":doc.version,"languageId":doc.language,"text":doc.text.as_ref()}}))?;
        rpc.notify(
            "textDocument/didFocus",
            json!({"textDocument":{"uri":doc.uri}}),
        )?;
    } else if let Some(old) = current.as_ref().filter(|old| old.version != doc.version) {
        let change = incremental(&old.text, &doc.text);
        rpc.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":doc.uri,"version":doc.version},"contentChanges":[change]}),
        )?;
    }
    *current = Some(doc);
    Ok(())
}
pub(super) fn incremental(old: &str, new: &str) -> Value {
    let start = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let old_len = old.chars().count();
    let new_len = new.chars().count();
    let tail = old
        .chars()
        .rev()
        .zip(new.chars().rev())
        .take((old_len - start).min(new_len - start))
        .take_while(|(a, b)| a == b)
        .count();
    json!({"range":{"start":position(old,start).unwrap(),"end":position(old,old_len-tail).unwrap()},"text":new.chars().skip(start).take(new_len-start-tail).collect::<String>()})
}
fn small(value: &Value, key: &str, limit: usize) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(limit)
        .collect()
}
fn notification(rpc: &mut Rpc, v: Value, events: &Events) -> Result<(), String> {
    let method = v.get("method").and_then(Value::as_str).unwrap_or("");
    let p = &v["params"];
    let id = v.get("id").cloned();
    match method {
        "didChangeStatus" => {
            let _ = events.send(Event::Status {
                kind: small(p, "kind", 32),
                message: small(p, "message", 2048),
            });
        }
        "window/showMessageRequest" => {
            if let Some(id) = id {
                let actions = p["actions"]
                    .as_array()
                    .map(|a| a.iter().take(8).map(|a| small(a, "title", 128)).collect())
                    .unwrap_or_default();
                let _ = events.send(Event::Message(Message {
                    id,
                    text: small(p, "message", 2048),
                    actions,
                }));
            }
        }
        "window/showMessage" => {
            let _ = events.send(Event::Status {
                kind: "Warning".into(),
                message: small(p, "message", 2048),
            });
        }
        "window/showDocument" => {
            if let Some(id) = id {
                let url = small(p, "uri", 2048);
                if url.starts_with("https://") {
                    let _ = events.send(Event::OpenUrl { id, url });
                } else {
                    rpc.reply(id, json!({"success":false}))?;
                }
            }
        }
        "workspace/configuration" => {
            if let Some(id) = id {
                let result = p["items"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .take(64)
                            .map(|i| {
                                if i["section"] == "telemetry" {
                                    json!({"telemetryLevel":"off"})
                                } else {
                                    Value::Null
                                }
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                rpc.reply(id, json!(result))?;
            }
        }
        "client/registerCapability"
        | "client/unregisterCapability"
        | "window/workDoneProgress/create" => {
            if let Some(id) = id {
                rpc.reply(id, Value::Null)?;
            }
        }
        _ => {
            if let Some(id) = id {
                transport::write(
                    &mut rpc.input,
                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Método não suportado"}}),
                )?;
            }
        }
    }
    Ok(())
}
pub(super) fn run(
    program: PathBuf,
    mut roots: Vec<PathBuf>,
    input: Receiver<Input>,
    events: Events,
    stop: Arc<AtomicBool>,
) {
    let result = (|| -> Result<(), String> {
        let mut rpc = Rpc::start(program)?;
        let mut current = None;
        let mut ready = false;
        rpc.request("initialize",json!({"processId":std::process::id(),"workspaceFolders":folders(&roots),"capabilities":{"workspace":{"workspaceFolders":true,"configuration":true},"window":{"showDocument":{"support":true}},"textDocument":{"inlineCompletion":{"dynamicRegistration":false}}},"initializationOptions":{"editorInfo":{"name":"Codisseu","version":env!("CARGO_PKG_VERSION")},"editorPluginInfo":{"name":"Codisseu Copilot","version":env!("CARGO_PKG_VERSION")}}}),Pending::Initialize)?;
        while !stop.load(Ordering::SeqCst) {
            if ready {
                for _ in 0..16 {
                    let action = match input.try_recv() {
                        Ok(v) => v,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return Ok(()),
                    };
                    let completion_ticket = match &action {
                        Input::Complete(_, ticket, ..) => Some(*ticket),
                        _ => None,
                    };
                    let done = match action {
                        Input::Document(doc) => sync(&mut rpc, &mut current, doc),
                        Input::Complete(doc, ticket, manual, selected) => {
                            rpc.cancel();
                            sync(&mut rpc, &mut current, doc.clone())?;
                            let mut context = json!({"triggerKind":if manual{1}else{2}});
                            if let Some(selected) = selected {
                                context["selectedCompletionInfo"] = selected;
                            }
                            rpc.request("textDocument/inlineCompletion",json!({"textDocument":{"uri":doc.uri,"version":doc.version},"position":doc.position,"context":context,"formattingOptions":{"tabSize":4,"insertSpaces":true}}),Pending::Complete(doc,ticket))?;
                            trace(json!({"event":"request","ticket":ticket,"manual":manual}));
                            Ok(())
                        }
                        Input::Cancel => {
                            rpc.cancel();
                            Ok(())
                        }
                        Input::SignIn => rpc.request("signIn", json!({}), Pending::Login),
                        Input::FinishLogin(command) => {
                            if command["command"] == "github.copilot.finishDeviceFlow" {
                                rpc.request("workspace/executeCommand", command, Pending::Finish)
                            } else {
                                Err("Comando de login inesperado".into())
                            }
                        }
                        Input::SignOut => rpc.request("signOut", json!({}), Pending::SignOut),
                        Input::Shown(item) => {
                            rpc.notify("textDocument/didShowCompletion", json!({"item":item}))
                        }
                        Input::Accepted(item) => {
                            if let Some(cmd) = item.get("command").filter(|cmd| {
                                cmd["command"] == "github.copilot.didAcceptCompletionItem"
                            }) {
                                rpc.request("workspace/executeCommand", cmd.clone(), Pending::Other)
                            } else {
                                Ok(())
                            }
                        }
                        Input::PartiallyAccepted(item, accepted_length) => rpc.notify(
                            "textDocument/didPartiallyAcceptCompletion",
                            json!({"item":item,"acceptedLength":accepted_length}),
                        ),
                        Input::Reply(id, result) => rpc.reply(id, result),
                        Input::Folders(paths) => {
                            let old = folders(&roots);
                            roots = paths;
                            rpc.notify(
                                "workspace/didChangeWorkspaceFolders",
                                json!({"event":{"added":folders(&roots),"removed":old}}),
                            )
                        }
                    };
                    if let Err(e) = done {
                        let _ = events.send(match completion_ticket {
                            Some(ticket) => Event::CompletionError { ticket, message: e },
                            None => Event::Error(e),
                        });
                    }
                }
            }
            for _ in 0..32 {
                let value = match rpc.receive.try_recv() {
                    Ok(Ok(v)) => v,
                    Ok(Err(_)) => return Err("Conexão com o servidor Copilot encerrada".into()),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        return Err("Servidor Copilot desconectou".into());
                    }
                };
                if value.get("method").is_some() {
                    notification(&mut rpc, value, &events)?;
                    continue;
                }
                let Some(id) = value["id"].as_u64() else {
                    continue;
                };
                let Some((pending, _)) = rpc.pending.remove(&id) else {
                    continue;
                };
                if let Some(error) = value.get("error") {
                    trace(
                        json!({"event":"rpc_error","code":error["code"].as_i64(),"completion":matches!(&pending,Pending::Complete(..))}),
                    );
                    if matches!(&pending, Pending::Complete(..))
                        && matches!(error["code"].as_i64(), Some(-32800 | -32802))
                    {
                        if let Pending::Complete(_, ticket) = pending {
                            let _ = events.send(Event::Suggestion(Vec::new(), ticket));
                        }
                        continue;
                    }
                    let message = small(error, "message", 2048);
                    let event = match pending {
                        Pending::Login | Pending::Finish => Event::AuthenticationError(message),
                        Pending::Complete(_, ticket) => Event::CompletionError { ticket, message },
                        Pending::Initialize => return Err(message),
                        Pending::SignOut | Pending::Other => Event::Error(message),
                    };
                    let _ = events.send(event);
                    continue;
                }
                let result = &value["result"];
                match pending {
                    Pending::Initialize => {
                        rpc.notify("initialized", json!({}))?;
                        rpc.notify(
                            "workspace/didChangeConfiguration",
                            json!({"settings":{"telemetry":{"telemetryLevel":"off"}}}),
                        )?;
                        ready = true;
                        let _ = events.send(Event::Ready);
                    }
                    Pending::Login => {
                        let code = small(result, "userCode", 128);
                        if !code.is_empty()
                            && result["command"]["command"] == "github.copilot.finishDeviceFlow"
                        {
                            let _ = events.send(Event::Login(Some(Login {
                                code,
                                command: result["command"].clone(),
                            })));
                        } else if result["status"] == "AlreadySignedIn" {
                            let _ = events.send(Event::Login(None));
                        } else {
                            let _ = events.send(Event::AuthenticationError(
                                "O servidor Copilot não confirmou o acesso. Entre novamente no GitHub.".into(),
                            ));
                        }
                    }
                    Pending::Finish => {
                        // A successful executeCommand response alone does not
                        // establish entitlement/authentication. Recheck with the
                        // documented signIn response before enabling requests.
                        rpc.request("signIn", json!({}), Pending::Login)?;
                    }
                    Pending::SignOut => {
                        let _ = events.send(Event::SignedOut);
                    }
                    Pending::Complete(doc, ticket) => {
                        let suggestions =
                            result["items"].as_array().map_or_else(Vec::new, |items| {
                                items
                                    .iter()
                                    .take(8)
                                    .filter_map(|item| decode(doc.clone(), ticket, item.clone()))
                                    .collect::<Vec<_>>()
                            });
                        trace(
                            json!({"event":"result","ticket":ticket,"items":result["items"].as_array().map_or(0,Vec::len),"usable":!suggestions.is_empty()}),
                        );
                        let _ = events.send(Event::Suggestion(suggestions, ticket));
                    }
                    Pending::Other => (),
                }
            }
            let expired = rpc
                .pending
                .iter()
                .filter_map(|(&id, (p, at))| {
                    (at.elapsed()
                        > Duration::from_secs(if matches!(p, Pending::Finish) {
                            300
                        } else {
                            30
                        }))
                    .then_some(id)
                })
                .collect::<Vec<_>>();
            for id in expired {
                let pending = rpc.pending.remove(&id).map(|(p, _)| p);
                let _ = rpc.notify("$/cancelRequest", json!({"id":id}));
                let message = "Copilot demorou a responder; tente novamente.".into();
                let event = match pending {
                    Some(Pending::Login | Pending::Finish) => Event::AuthenticationError(message),
                    Some(Pending::Complete(_, ticket)) => {
                        Event::CompletionError { ticket, message }
                    }
                    Some(Pending::Initialize) => return Err(message),
                    _ => Event::Error(message),
                };
                let _ = events.send(event);
            }
            if let Some(exit) = rpc.child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!("Servidor Copilot encerrou: {exit}"));
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = events.send(Event::Error(error));
    }
    let _ = events.send(Event::Stopped);
}
