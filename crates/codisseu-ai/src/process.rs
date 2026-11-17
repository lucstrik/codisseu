use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command as ProcessCommand, Stdio},
    sync::mpsc,
    thread,
    time::Duration,
};
const MAX_LINE: u64 = 2 * 1024 * 1024;
pub(super) struct Request {
    pub ephemeral: bool,
    pub config: Config,
    pub workspace: PathBuf,
    pub action: Action,
    pub session: Option<String>,
    pub attachments: Option<attachments::PreparedAttachments>,
}
enum Input {
    Json(Value),
    Text(String),
    Close,
}
pub(super) struct Process {
    child: Child,
    input: Option<SyncSender<Input>>,
    lines: Receiver<Result<Value, String>>,
    cleaned: bool,
}
impl Process {
    fn spawn(request: &Request, cancel: &Arc<AtomicBool>) -> Result<Self, String> {
        let mut command = ProcessCommand::new(&request.config.program);
        // Preserve CLI authentication and platform discovery, without forwarding
        // arbitrary IDE/connector/database environment secrets to an agent.
        command.env_clear();
        for (name, value) in std::env::vars_os() {
            if allowed_environment(&name.to_string_lossy(), request.config.provider) {
                command.env(name, value);
            }
        }
        command
            .current_dir(&request.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        match request.config.provider {
            Provider::Codex => {
                command.args(["app-server", "--listen", "stdio://"]);
            }
            Provider::Claude => {
                if request.action.prompt().is_none() {
                    return Err("Claude: instale e autentique o CLI com claude auth login; depois envie uma mensagem".into());
                }
                command.args([
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--include-partial-messages",
                ]);
                if request.config.mode == Mode::FullAccess {
                    command.args([
                        "--permission-mode",
                        "bypassPermissions",
                        "--tools",
                        "default",
                    ]);
                } else {
                    let tools = if request.config.mode == Mode::Consult {
                        "Read,Glob,Grep"
                    } else {
                        "Read,Glob,Grep,Edit,Write"
                    };
                    command.args([
                        "--permission-mode",
                        "dontAsk",
                        "--tools",
                        tools,
                        "--allowedTools",
                        tools,
                    ]);
                }
                if let Some(session) = &request.session {
                    command.args(["--resume", session]);
                }
                if !request.config.model.trim().is_empty() {
                    command.args(["--model", request.config.model.trim()]);
                }
            }
        }
        // Each CLI and its descendants belong to an owned group, never the IDE group.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child=command.spawn().map_err(|e|format!("Não foi possível iniciar {}: {e}. Confira o executável nas configurações do AI Chat.",request.config.provider.label()))?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let (writer, writes) = mpsc::sync_channel(8);
        thread::spawn(move || {
            while let Ok(input) = writes.recv() {
                let result = match input {
                    Input::Json(value) => serde_json::to_writer(&mut stdin, &value)
                        .map_err(std::io::Error::other)
                        .and_then(|_| stdin.write_all(b"\n")),
                    Input::Text(text) => stdin
                        .write_all(text.as_bytes())
                        .and_then(|_| stdin.write_all(b"\n")),
                    Input::Close => break,
                };
                if result.and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let input = Some(writer);
        let (sender, lines) = mpsc::sync_channel(64);
        let stop = cancel.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let n = reader
                    .by_ref()
                    .take(MAX_LINE + 1)
                    .read_until(b'\n', &mut bytes);
                let value = match n {
                    Ok(0) => break,
                    Ok(n) if n as u64 > MAX_LINE => Err("Mensagem do CLI excedeu2MiB".into()),
                    Ok(_) => serde_json::from_slice(&bytes)
                        .map_err(|_| "Saída do agente não é JSONL válido".into()),
                    Err(e) => Err(e.to_string()),
                };
                let fatal = value.is_err();
                if !emit_channel(&sender, value, &stop) || fatal {
                    break;
                }
            }
        });
        // Drain diagnostics without recording tokens, account data, or uncontrolled logs.
        thread::spawn(move || {
            let mut reader = stderr;
            let mut b = [0u8; 4096];
            while matches!(reader.read(&mut b),Ok(n)if n>0) {}
        });
        Ok(Self {
            child,
            input,
            lines,
            cleaned: false,
        })
    }
    pub(super) fn send(&mut self, value: Value) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("Entrada do CLI encerrada")?
            .try_send(Input::Json(value))
            .map_err(|_| "Fila de entrada do CLI indisponível".into())
    }
    pub(super) fn next(&mut self) -> Result<Option<Value>, String> {
        match self.lines.recv_timeout(Duration::from_millis(20)) {
            Ok(v) => v.map(Some),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = self.child.try_wait().map_err(|e| e.to_string())?;
                Err(format!(
                    "O CLI encerrou antes do resultado final (código {}).",
                    status
                        .and_then(|s| s.code())
                        .map_or("desconhecido".into(), |n| n.to_string())
                ))
            }
        }
    }
    pub(super) fn send_text(&mut self, text: String) -> Result<(), String> {
        self.input
            .as_ref()
            .ok_or("Entrada do CLI encerrada")?
            .try_send(Input::Text(text))
            .map_err(|_| "Fila de entrada do CLI indisponível".into())
    }
    pub(super) fn close_input(&mut self) {
        if let Some(input) = self.input.take() {
            let _ = input.try_send(Input::Close);
        }
    }
    pub(super) fn wait_success(&mut self, stop: &AtomicBool) -> Result<(), String> {
        for _ in 0..500 {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(format!("Claude encerrou com {status}"))
                };
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("Claude publicou resultado mas não encerrou em5s".into())
    }
    fn cleanup(&mut self) {
        if self.cleaned {
            return;
        }
        self.cleaned = true;
        self.input.take();
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            let _ = killpg(Pid::from_raw(self.child.id() as i32), Signal::SIGTERM);
        }
        for _ in 0..20 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
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
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.cleanup();
    }
}
pub(super) fn emit(out: &SyncSender<Event>, event: Event, stop: &AtomicBool) -> Result<(), String> {
    if emit_channel(out, event, stop) {
        Ok(())
    } else {
        Err("Operação interrompida".into())
    }
}
fn emit_channel<T>(sender: &SyncSender<T>, mut value: T, stop: &AtomicBool) -> bool {
    loop {
        if stop.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(value) {
            Ok(()) => return true,
            Err(mpsc::TrySendError::Disconnected(_)) => return false,
            Err(mpsc::TrySendError::Full(v)) => {
                value = v;
                thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
pub(super) fn run(
    request: Request,
    stop: &Arc<AtomicBool>,
    out: &SyncSender<Event>,
    replies: Receiver<(Value, bool)>,
) -> Result<bool, String> {
    let mut request = request;
    // Resolve once on this I/O worker. A relative cwd sent after chdir would be
    // interpreted relative to the project a second time by the app server.
    // Attachments, child cwd and protocol writable roots must share this value.
    request.workspace = request
        .workspace
        .canonicalize()
        .map_err(|error| format!("Projeto indisponível: {error}"))?;
    if !request.workspace.is_dir() {
        return Err("O projeto não é uma pasta".into());
    }
    if stop.load(Ordering::Acquire) {
        return Ok(true);
    }
    if !request.action.inputs().is_empty() {
        request.attachments = Some(attachments::prepare(
            &request.workspace,
            request.action.inputs(),
        )?);
        if request.config.provider == Provider::Claude
            && request
                .attachments
                .as_ref()
                .is_some_and(|a| !a.images.is_empty())
        {
            return Err(
                "Imagens ainda não são suportadas pelo adaptador Claude; selecione Codex".into(),
            );
        }
    }
    if stop.load(Ordering::Acquire) {
        return Ok(true);
    }
    let mut process = Process::spawn(&request, stop)?;
    let result = match request.config.provider {
        Provider::Codex => protocol::codex(&mut process, &request, stop, out, &replies),
        Provider::Claude => protocol::claude(&mut process, &request, stop, out),
    };
    process.cleanup();
    // Cancellation becomes final only after the process has actually been reaped.
    if stop.load(Ordering::Acquire) {
        Ok(true)
    } else {
        result.map(|_| false)
    }
}

fn allowed_environment(name: &str, provider: Provider) -> bool {
    const COMMON: &[&str] = &[
        "HOME",
        "USER",
        "LOGNAME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "PATH",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_CTYPE",
        "SHELL",
        "TMPDIR",
        "TMP",
        "TEMP",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "NODE_EXTRA_CA_CERTS",
    ];
    COMMON.iter().any(|key| name.eq_ignore_ascii_case(key))
        || match provider {
            Provider::Codex => matches!(
                name,
                "CODEX_HOME" | "OPENAI_API_KEY" | "CODEX_API_KEY" | "OPENAI_BASE_URL"
            ),
            Provider::Claude => matches!(
                name,
                "CLAUDE_CONFIG_DIR"
                    | "ANTHROPIC_API_KEY"
                    | "ANTHROPIC_AUTH_TOKEN"
                    | "ANTHROPIC_BASE_URL"
                    | "CLAUDE_CODE_OAUTH_TOKEN"
            ),
        }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_platform_and_selected_provider_environment_is_forwarded() {
        assert!(allowed_environment("Path", Provider::Codex));
        assert!(allowed_environment("CODEX_HOME", Provider::Codex));
        assert!(!allowed_environment("DATABASE_URL", Provider::Codex));
        assert!(!allowed_environment("ANTHROPIC_API_KEY", Provider::Codex));
        assert!(allowed_environment("ANTHROPIC_API_KEY", Provider::Claude));
        assert!(!allowed_environment("OPENAI_API_KEY", Provider::Claude));
    }
}
