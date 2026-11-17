use super::{
    process::{Process, Request, emit},
    *,
};
use serde_json::json;
use std::{collections::HashSet, time::Duration};
fn string<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}
fn rpc(
    p: &mut Process,
    id: u64,
    method: &str,
    params: Value,
    stop: &AtomicBool,
    out: &SyncSender<Event>,
) -> Result<Value, String> {
    p.send(json!({"id":id,"method":method,"params":params}))?;
    let start = Instant::now();
    loop {
        if stop.load(Ordering::Acquire) {
            return Err("Interrompido".into());
        }
        if start.elapsed()
            > Duration::from_secs(if method == "account/rateLimits/read" {
                8
            } else {
                45
            })
        {
            return Err(format!("Tempo esgotado em {method}"));
        }
        if let Some(v) = p.next()? {
            if v.get("id") == Some(&json!(id)) && v.get("method").is_none() {
                if let Some(error) = v.get("error") {
                    return Err(format!("{method}: {}", string(error, "message")));
                }
                return v
                    .get("result")
                    .cloned()
                    .ok_or("Resposta sem resultado".into());
            }
            if string(&v, "method") == "account/rateLimits/updated"
                && let Some(limits) = usage::Limits::parse(&v["params"])
            {
                emit(out, Event::Limits(limits), stop)?;
            }
            if v.get("id").is_some() && v.get("method").is_some() {
                unsupported(p, &v, out, stop)?;
            }
        }
    }
}
fn unsupported(
    p: &mut Process,
    v: &Value,
    out: &SyncSender<Event>,
    stop: &AtomicBool,
) -> Result<(), String> {
    p.send(json!({"id":v["id"],"error":{"code":-32601,"message":"This client does not support this request; no permission was granted."}}))?;
    emit(
        out,
        Event::Note(format!(
            "Solicitação não suportada: {}. Nenhuma permissão concedida.",
            string(v, "method")
        )),
        stop,
    )
}
fn read_limits(p: &mut Process, stop: &AtomicBool, out: &SyncSender<Event>) -> Result<(), String> {
    match rpc(p, 6, "account/rateLimits/read", json!({}), stop, out) {
        Ok(value) => match usage::Limits::parse(&value) {
            Some(limits) => emit(out, Event::Limits(limits), stop),
            None => emit(
                out,
                Event::LimitsUnavailable("O CLI não retornou limites da conta".into()),
                stop,
            ),
        },
        Err(mut error) => {
            clip(&mut error, 300);
            emit(
                out,
                Event::LimitsUnavailable(format!("Limites indisponíveis: {error}")),
                stop,
            )
        }
    }
}
pub(super) fn codex(
    p: &mut Process,
    r: &Request,
    stop: &AtomicBool,
    out: &SyncSender<Event>,
    replies: &Receiver<(Value, bool)>,
) -> Result<(), String> {
    rpc(
        p,
        1,
        "initialize",
        json!({"clientInfo":{"name":"codisseu","title":"Codisseu","version":env!("CARGO_PKG_VERSION")}}),
        stop,
        out,
    )?;
    p.send(json!({"method":"initialized","params":{}}))?;
    match &r.action {
        Action::Usage => {
            read_limits(p, stop, out)?;
            return Ok(());
        }
        Action::DeleteThread(thread) => {
            rpc(p, 7, "thread/delete", json!({"threadId":thread}), stop, out)?;
            return Ok(());
        }
        Action::Models => {
            let mut all = Vec::new();
            let mut cursor = Value::Null;
            for n in 0..8 {
                let page = rpc(
                    p,
                    20 + n,
                    "model/list",
                    json!({"limit":32,"cursor":cursor,"includeHidden":false}),
                    stop,
                    out,
                )?;
                all.extend(models::page(&page)?);
                if all.len() > 128 {
                    return Err("Catálogo excedeu128 modelos".into());
                }
                cursor = page["nextCursor"].clone();
                if cursor.is_null() {
                    emit(out, Event::Models(all), stop)?;
                    read_limits(p, stop, out)?;
                    return Ok(());
                }
            }
            return Err("Catálogo excedeu8 páginas".into());
        }
        Action::Check => {
            let account = rpc(
                p,
                2,
                "account/read",
                json!({"refreshToken":false}),
                stop,
                out,
            )?;
            emit(out, Event::Account(!account["account"].is_null()), stop)?;
            return Ok(());
        }
        Action::Login => {
            let login = rpc(
                p,
                2,
                "account/login/start",
                json!({"type":"chatgpt"}),
                stop,
                out,
            )?;
            let url = string(&login, "authUrl");
            if !url.starts_with("https://auth.openai.com/") {
                return Err("O CLI devolveu um endereço de autenticação não reconhecido".into());
            }
            emit(out, Event::LoginUrl(url.into()), stop)?;
            let start = Instant::now();
            loop {
                if stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                if start.elapsed() > Duration::from_secs(300) {
                    return Err("Login expirou; tente conectar novamente".into());
                }
                if let Some(v) = p.next()? {
                    if string(&v, "method") == "account/login/completed" {
                        if v["params"]["success"] == true {
                            emit(out, Event::Account(true), stop)?;
                            return Ok(());
                        }
                        return Err("Login não concluído".into());
                    }
                    if v.get("id").is_some() && v.get("method").is_some() {
                        unsupported(p, &v, out, stop)?;
                    }
                }
            }
        }
        Action::Send(_) | Action::SendWithAttachments { .. } => {}
    }
    let mut params = json!({"cwd":r.workspace,"approvalPolicy":if r.config.mode==Mode::Agent{"on-request"}else{"never"},"approvalsReviewer":"user","sandbox":match r.config.mode{Mode::Agent=>"workspace-write",Mode::Consult=>"read-only",Mode::FullAccess=>"danger-full-access"}});
    if r.ephemeral {
        params["ephemeral"] = json!(true);
    }
    if !r.config.model.trim().is_empty() {
        params["model"] = json!(r.config.model.trim());
    }
    let method = if let Some(session) = &r.session {
        params["threadId"] = json!(session);
        "thread/resume"
    } else {
        "thread/start"
    };
    let result = rpc(p, 3, method, params, stop, out)?;
    let thread = string(&result["thread"], "id");
    if thread.is_empty() {
        return Err("CLI não retornou o identificador da conversa".into());
    }
    emit(out, Event::Session(thread.into()), stop)?;
    let mut prompt = r.action.prompt().unwrap_or_default().to_owned();
    if let Some(attachments) = &r.attachments
        && !attachments.text_context.is_empty()
    {
        prompt.push_str("\n\n");
        prompt.push_str(&attachments.text_context);
    }
    let mut input = Vec::new();
    if !prompt.is_empty() {
        input.push(json!({"type":"text","text":prompt}));
    }
    if let Some(attachments) = &r.attachments {
        for path in &attachments.images {
            input.push(json!({"type":"localImage","path":path}));
        }
    }
    let mut params = json!({"threadId":thread,"cwd":r.workspace,"input":input,"approvalPolicy":if r.config.mode==Mode::Agent{"on-request"}else{"never"},"approvalsReviewer":"user","sandboxPolicy":match r.config.mode{Mode::Agent=>json!({"type":"workspaceWrite","writableRoots":[r.workspace],"networkAccess":false}),Mode::Consult=>json!({"type":"readOnly","networkAccess":false}),Mode::FullAccess=>json!({"type":"dangerFullAccess"})}});
    if !r.config.model.is_empty() {
        params["model"] = json!(r.config.model);
    }
    if !r.config.effort.is_empty() {
        params["effort"] = json!(r.config.effort);
    }
    p.send(json!({"id":4,"method":"turn/start","params":params}))?;
    let mut last = Instant::now();
    let mut pending = HashSet::new();
    let mut turn_id = None;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        while let Ok((id, accept)) = replies.try_recv() {
            if pending.remove(&id.to_string()) {
                p.send(json!({"id":id,"result":{"decision":if accept{"accept"}else{"decline"}}}))?;
            }
        }
        if pending.is_empty() && last.elapsed() > Duration::from_secs(180) {
            return Err("Agente sem eventos por180s; o processo foi encerrado".into());
        }
        let Some(v) = p.next()? else { continue };
        let method = string(&v, "method");
        let a = &v["params"];
        if method.is_empty() {
            if let Some(e) = v.get("error") {
                return Err(format!("CLI: {}", string(e, "message")));
            }
            if v["id"] == 4 {
                let id = string(&v["result"]["turn"], "id");
                if !id.is_empty() {
                    turn_id = Some(id.to_owned());
                }
                last = Instant::now();
            }
            continue;
        }
        let event_thread = string(a, "threadId");
        let event_turn = if method == "turn/completed" || method == "turn/started" {
            string(&a["turn"], "id")
        } else {
            string(a, "turnId")
        };
        if (!event_thread.is_empty() && event_thread != thread)
            || turn_id
                .as_deref()
                .is_some_and(|id| !event_turn.is_empty() && event_turn != id)
        {
            if v.get("id").is_some() {
                p.send(json!({"id":v["id"],"error":{"code":-32602,"message":"Request does not belong to this conversation and turn; no permission was granted."}}))?;
            }
            continue;
        }
        last = Instant::now();
        if v.get("id").is_some() {
            if r.ephemeral {
                unsupported(p, &v, out, stop)?;
                return Err("A IA solicitou uma interação durante a geração da mensagem de commit; nenhuma permissão concedida".into());
            }
            if matches!(
                method,
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval"
            ) && r.config.mode == Mode::Agent
            {
                let description = format!(
                    "{}\n{}\n{}",
                    string(a, "command"),
                    string(a, "reason"),
                    a.get("networkApprovalContext")
                        .filter(|v| !v.is_null())
                        .map(Value::to_string)
                        .unwrap_or_default()
                );
                pending.insert(v["id"].to_string());
                emit(
                    out,
                    Event::Approval(Approval {
                        id: v["id"].clone(),
                        description: if description.trim().is_empty() {
                            format!("Alteração de arquivo: {}", string(a, "itemId"))
                        } else {
                            description
                        },
                    }),
                    stop,
                )?;
            } else {
                unsupported(p, &v, out, stop)?;
            }
            continue;
        }
        match method {
            "thread/tokenUsage/updated" if string(a, "threadId") == thread => {
                if let Some(tokens) = usage::Tokens::parse(&a["tokenUsage"]) {
                    emit(out, Event::Usage(tokens), stop)?;
                }
            }
            "account/rateLimits/updated" => {
                if let Some(limits) = usage::Limits::parse(a) {
                    emit(out, Event::Limits(limits), stop)?;
                }
            }
            "item/agentMessage/delta" => emit(
                out,
                Event::Entry(
                    Entry {
                        id: string(a, "itemId").into(),
                        role: "Assistente",
                        text: string(a, "delta").into(),
                        detail: String::new(),
                    },
                    true,
                ),
                stop,
            )?,
            "item/started" | "item/completed" => {
                let item = &a["item"];
                let kind = string(item, "type");
                let id = string(item, "id");
                if kind == "agentMessage" && method == "item/completed" {
                    emit(
                        out,
                        Event::Entry(
                            Entry {
                                id: id.into(),
                                role: "Assistente",
                                text: string(item, "text").into(),
                                detail: String::new(),
                            },
                            false,
                        ),
                        stop,
                    )?;
                } else if matches!(
                    kind,
                    "commandExecution" | "fileChange" | "mcpToolCall" | "webSearch" | "plan"
                ) {
                    let title = match kind {
                        "commandExecution" => format!("Comando · {}", string(item, "status")),
                        "fileChange" => format!("Arquivos · {}", string(item, "status")),
                        "plan" => "Plano".into(),
                        _ => format!("Ferramenta · {kind}"),
                    };
                    let mut detail = if kind == "commandExecution" {
                        format!(
                            "{}\n{}",
                            string(item, "command"),
                            string(item, "aggregatedOutput")
                        )
                    } else {
                        item.to_string()
                    };
                    clip(&mut detail, MAX_TEXT);
                    emit(
                        out,
                        Event::Entry(
                            Entry {
                                id: id.into(),
                                role: "Ferramenta",
                                text: title,
                                detail,
                            },
                            false,
                        ),
                        stop,
                    )?;
                }
            }
            "serverRequest/resolved" => {
                pending.remove(&a["requestId"].to_string());
            }
            "turn/completed" => {
                let turn = &a["turn"];
                return match string(turn, "status") {
                    "completed" => {
                        read_limits(p, stop, out)?;
                        Ok(())
                    }
                    "interrupted" => Err("Turno interrompido pelo agente".into()),
                    _ => Err(format!(
                        "Turno falhou: {}",
                        string(&turn["error"], "message")
                    )),
                };
            }
            "error" if a["willRetry"] != true => {
                return Err(format!("Codex: {}", string(&a["error"], "message")));
            }
            _ => {}
        }
    }
}
pub(super) fn claude(
    p: &mut Process,
    r: &Request,
    stop: &AtomicBool,
    out: &SyncSender<Event>,
) -> Result<(), String> {
    let mut prompt = r.action.prompt().unwrap_or_default().to_owned();
    if let Some(attachments) = &r.attachments {
        prompt.push_str("\n\n");
        prompt.push_str(&attachments.text_context);
    }
    p.send_text(prompt)?;
    p.close_input();
    let mut last = Instant::now();
    let mut id = String::from("claude-message");
    let mut final_seen = false;
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
        if last.elapsed() > Duration::from_secs(180) {
            return Err("Claude sem eventos por180s".into());
        }
        let Some(v) = p.next()? else { continue };
        last = Instant::now();
        // Child runs have their own identity; never merge them into the main answer.
        if !v["parent_tool_use_id"].is_null() {
            continue;
        }
        match string(&v, "type") {
            "system" => {
                let session = string(&v, "session_id");
                if !session.is_empty() {
                    emit(out, Event::Session(session.into()), stop)?
                }
            }
            "stream_event" => {
                let e = &v["event"];
                if string(e, "type") == "message_start" {
                    id = string(&e["message"], "id").into();
                }
                if string(&e["delta"], "type") == "text_delta" {
                    emit(
                        out,
                        Event::Entry(
                            Entry {
                                id: id.clone(),
                                role: "Assistente",
                                text: string(&e["delta"], "text").into(),
                                detail: String::new(),
                            },
                            true,
                        ),
                        stop,
                    )?;
                }
            }
            "assistant" => {
                let msg = &v["message"];
                let mid = string(msg, "id");
                if !mid.is_empty() {
                    id = mid.into();
                }
                let mut text = String::new();
                if let Some(blocks) = msg["content"].as_array() {
                    for block in blocks {
                        if string(block, "type") == "text" {
                            text.push_str(string(block, "text"));
                        }
                        if string(block, "type") == "tool_use" {
                            if r.ephemeral {
                                return Err("A IA tentou usar uma ferramenta durante a geração da mensagem de commit".into());
                            }
                            emit(
                                out,
                                Event::Entry(
                                    Entry {
                                        id: string(block, "id").into(),
                                        role: "Ferramenta",
                                        text: string(block, "name").into(),
                                        detail: block["input"].to_string(),
                                    },
                                    false,
                                ),
                                stop,
                            )?;
                        }
                    }
                }
                if !text.is_empty() {
                    final_seen = true;
                    emit(
                        out,
                        Event::Entry(
                            Entry {
                                id: id.clone(),
                                role: "Assistente",
                                text,
                                detail: String::new(),
                            },
                            false,
                        ),
                        stop,
                    )?;
                }
            }
            "result" => {
                let session = string(&v, "session_id");
                if !session.is_empty() {
                    emit(out, Event::Session(session.into()), stop)?;
                }
                if v["is_error"] == true {
                    return Err(format!("Claude: {} {}", string(&v, "result"), v["errors"]));
                }
                if !final_seen && !string(&v, "result").is_empty() {
                    emit(
                        out,
                        Event::Entry(
                            Entry {
                                id: id.clone(),
                                role: "Assistente",
                                text: string(&v, "result").into(),
                                detail: String::new(),
                            },
                            false,
                        ),
                        stop,
                    )?;
                }
                if v["permission_denials"]
                    .as_array()
                    .is_some_and(|a| !a.is_empty())
                {
                    emit(out,Event::Note("Claude recusou ferramentas que exigem autorização adicional; consulte a configuração do CLI.".into()),stop)?;
                }
                return p.wait_success(stop);
            }
            _ => {}
        }
    }
}
