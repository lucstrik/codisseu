//! Optional Copilot LSP client. Credentials remain owned by the official server.
mod projection;
pub use projection::InlineProjection;
mod transport;
mod worker;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};
pub const MAX_DOCUMENT: usize = 1024 * 1024;
pub const MAX_SUGGESTION: usize = 16 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}
#[derive(Clone, Debug)]
pub struct Document {
    pub uri: String,
    pub version: u64,
    pub language: String,
    pub text: Arc<str>,
    pub position: Position,
}
#[derive(Clone, Debug)]
pub struct Suggestion {
    pub ticket: u64,
    pub document: Document,
    pub start: usize,
    pub end: usize,
    pub insert: String,
    pub suffix: String,
    pub item: Value,
}
impl Suggestion {
    /// Source-preserving projection; byte ranges refer to projected text.
    pub fn projection(&self) -> Option<InlineProjection> {
        let caret = offset(&self.document.text, self.document.position)?;
        if self.start > caret || caret > self.end {
            return None;
        }
        let prefix: String = self
            .document
            .text
            .chars()
            .skip(self.start)
            .take(caret - self.start)
            .collect();
        let source: String = self
            .document
            .text
            .chars()
            .skip(caret)
            .take(self.end - caret)
            .collect();
        projection::project(&source, self.insert.strip_prefix(&prefix)?)
    }
    /// Compatibility accessor for the first retained source span.
    pub fn preserved_suffix(&self) -> Option<std::ops::Range<usize>> {
        self.projection()?.preserved.into_iter().next()
    }
    /// Rebase after consuming a prefix of the same final, insertion-only edit.
    pub fn continued(&self, document: Document) -> Option<Self> {
        if document.uri != self.document.uri {
            return None;
        }
        let old = offset(&self.document.text, self.document.position)?;
        let caret = offset(&document.text, document.position)?;
        if caret <= old {
            return None;
        }
        let delta =
            document.text.chars().count() as isize - self.document.text.chars().count() as isize;
        let take = caret - old;
        let projection = self.projection()?;
        if take > projection.text.chars().count() {
            return None;
        }
        let consumed = projection.source_prefix(take);
        if take as isize - consumed as isize != delta {
            return None;
        }
        let old_prefix = self.document.text.chars().take(old);
        if !old_prefix.eq(document.text.chars().take(old)) {
            return None;
        }
        if !projection
            .text
            .chars()
            .take(take)
            .eq(document.text.chars().skip(old).take(take))
        {
            return None;
        }
        if !self
            .document
            .text
            .chars()
            .skip(old + consumed)
            .eq(document.text.chars().skip(caret))
        {
            return None;
        }
        let end = self.end.checked_add_signed(delta)?;
        let prefix_equal = self
            .document
            .text
            .chars()
            .take(self.start)
            .eq(document.text.chars().take(self.start));
        let suffix_equal = self
            .document
            .text
            .chars()
            .skip(self.end)
            .eq(document.text.chars().skip(end));
        if !prefix_equal || !suffix_equal {
            return None;
        }
        let mut item = self.item.clone();
        item["insertText"] = Value::String(self.insert.clone());
        item["range"] = serde_json::json!({"start":position(&document.text,self.start)?,"end":position(&document.text,end)?});
        let mut next = decode(document, self.ticket, item)?;
        next.item = self.item.clone();
        Some(next)
    }
}
#[derive(Clone, Debug)]
pub struct Message {
    pub id: Value,
    pub text: String,
    pub actions: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Login {
    pub code: String,
    pub command: Value,
}
#[derive(Debug)]
pub enum Event {
    Ready,
    Status { kind: String, message: String },
    Login(Option<Login>),
    Suggestion(Vec<Suggestion>, u64),
    CompletionError { ticket: u64, message: String },
    Error(String),
    AuthenticationError(String),
    Message(Message),
    OpenUrl { id: Value, url: String },
    Stopped,
    SignedOut,
}
pub(super) enum Input {
    Document(Document),
    Complete(Document, u64, bool, Option<Value>),
    Cancel,
    SignIn,
    FinishLogin(Value),
    SignOut,
    Shown(Value),
    Accepted(Value),
    PartiallyAccepted(Value, u32),
    Reply(Value, Value),
    Folders(Vec<PathBuf>),
}
pub(super) struct Events {
    sender: SyncSender<Event>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
impl Events {
    fn send(&self, event: Event) -> Result<(), ()> {
        self.sender.send(event).map_err(|_| ())?;
        (self.wake)();
        Ok(())
    }
}
pub struct Client {
    send: SyncSender<Input>,
    events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    pub ready: bool,
    pub authenticated: bool,
    pub authentication_checked: bool,
    pub busy: bool,
    pub status: String,
    pub kind: String,
    pub login: Option<Login>,
    pub login_in_progress: bool,
    pub messages: Vec<Message>,
    pub open_url: Option<(Value, String)>,
    pub error: Option<String>,
    pub suggestion: Option<Suggestion>,
    pub suggestion_index: usize,
    pub suggestion_count: usize,
    alternatives: Vec<Suggestion>,
    ticket: u64,
    recheck_after_auth_error: bool,
}
impl Client {
    pub fn start(program: PathBuf, folders: Vec<PathBuf>) -> Result<Self, String> {
        Self::start_with_wake(program, folders, Arc::new(|| {}))
    }
    pub fn start_with_wake(
        program: PathBuf,
        folders: Vec<PathBuf>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        if !program.is_absolute() || !program.is_file() {
            return Err(
                "Servidor Copilot não encontrado. Confira o caminho nas configurações.".into(),
            );
        }
        let (tx, rx) = mpsc::sync_channel(16);
        let (etx, erx) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::Builder::new()
            .name("copilot-client".into())
            .spawn(move || worker::run(program, folders, rx, Events { sender: etx, wake }, flag))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            send: tx,
            events: erx,
            stop,
            ready: false,
            authenticated: false,
            authentication_checked: false,
            busy: false,
            status: "Iniciando Copilot…".into(),
            kind: "Starting".into(),
            login: None,
            login_in_progress: false,
            messages: Vec::new(),
            open_url: None,
            error: None,
            suggestion: None,
            suggestion_index: 0,
            suggestion_count: 0,
            alternatives: Vec::new(),
            ticket: 0,
            recheck_after_auth_error: false,
        })
    }
    fn send(&self, input: Input) -> Result<(), String> {
        self.send
            .try_send(input)
            .map_err(|_| "Copilot está ocupado; tente novamente.".into())
    }
    pub fn document(&self, doc: Document) -> Result<(), String> {
        self.send(Input::Document(doc))
    }
    pub fn folders(&self, roots: Vec<PathBuf>) -> Result<(), String> {
        self.send(Input::Folders(roots))
    }
    pub fn complete(&mut self, doc: Document, manual: bool) -> Result<u64, String> {
        self.complete_with_context(doc, manual, None)
    }
    /// Preserve the semantic completion selected by the editor, as defined by
    /// InlineCompletionContext.selectedCompletionInfo in LSP3.18.
    pub fn complete_with_context(
        &mut self,
        doc: Document,
        manual: bool,
        selected_completion: Option<Value>,
    ) -> Result<u64, String> {
        self.cancel();
        self.ticket = self.ticket.wrapping_add(1);
        let ticket = self.ticket;
        self.send(Input::Complete(doc, ticket, manual, selected_completion))?;
        self.busy = true;
        self.error = None;
        Ok(ticket)
    }
    pub fn cancel(&mut self) {
        self.ticket = self.ticket.wrapping_add(1);
        self.busy = false;
        self.clear_suggestions();
        let _ = self.send(Input::Cancel);
    }
    fn clear_suggestions(&mut self) {
        self.suggestion = None;
        self.alternatives.clear();
        self.suggestion_index = 0;
        self.suggestion_count = 0;
    }
    /// Cycle only among already returned alternatives. No code or telemetry is
    /// accepted by changing the selected suggestion.
    pub fn cycle(&mut self, direction: i32) -> bool {
        if self.alternatives.len() < 2 || direction == 0 {
            return false;
        }
        self.suggestion_index = (self.suggestion_index as i64 + i64::from(direction))
            .rem_euclid(self.alternatives.len() as i64) as usize;
        self.suggestion = Some(self.alternatives[self.suggestion_index].clone());
        true
    }
    /// Retain the unaccepted remainder after the host editor has applied and
    /// verified an exact prefix insertion. Its original provider item is kept
    /// for partial/full acceptance telemetry. All other alternatives are stale.
    pub fn continue_suggestion(&mut self, suggestion: Suggestion) -> bool {
        let Some(previous) = self.suggestion.as_ref() else {
            return false;
        };
        if suggestion.ticket != self.ticket
            || previous.ticket != suggestion.ticket
            || previous.document.uri != suggestion.document.uri
            || suggestion.document.version <= previous.document.version
            || previous.item != suggestion.item
            || suggestion.suffix.is_empty()
            || suggestion.document.text.len() > MAX_DOCUMENT
            || suggestion.insert.len() > MAX_SUGGESTION * 2
            || suggestion.suffix.len() > MAX_SUGGESTION * 2
            || suggestion.start > suggestion.end
            || suggestion.end > suggestion.document.text.chars().count()
        {
            return false;
        }
        let Some(caret) = offset(&suggestion.document.text, suggestion.document.position) else {
            return false;
        };
        if !(suggestion.start..=suggestion.end).contains(&caret) {
            return false;
        }
        if suggestion
            .projection()
            .is_none_or(|p| p.text != suggestion.suffix)
        {
            return false;
        }
        // Continuing a preview must lead to the exact same full buffer as
        // accepting its original edit. Unrelated edits invalidate the remainder.
        let original_result = previous
            .document
            .text
            .chars()
            .take(previous.start)
            .chain(previous.insert.chars())
            .chain(previous.document.text.chars().skip(previous.end));
        let continued_result = suggestion
            .document
            .text
            .chars()
            .take(suggestion.start)
            .chain(suggestion.insert.chars())
            .chain(suggestion.document.text.chars().skip(suggestion.end));
        if !original_result.eq(continued_result) {
            return false;
        }
        self.alternatives = vec![suggestion.clone()];
        self.suggestion = Some(suggestion);
        self.suggestion_index = 0;
        self.suggestion_count = 1;
        self.busy = false;
        true
    }
    pub fn sign_in(&mut self) -> Result<(), String> {
        self.recheck_after_auth_error = false;
        self.login_in_progress = false;
        self.error = None;
        self.send(Input::SignIn)?;
        self.authentication_checked = true;
        Ok(())
    }
    pub fn finish_login(&mut self) -> Result<(), String> {
        if self.login_in_progress {
            return Ok(());
        }
        let l = self
            .login
            .as_ref()
            .ok_or("Solicite um código de acesso primeiro")?;
        self.send(Input::FinishLogin(l.command.clone()))?;
        self.login_in_progress = true;
        self.status = "Aguardando confirmação no navegador…".into();
        Ok(())
    }
    pub fn sign_out(&mut self) -> Result<(), String> {
        self.recheck_after_auth_error = false;
        self.login_in_progress = false;
        self.cancel();
        self.login = None;
        self.authenticated = false;
        self.send(Input::SignOut)
    }
    pub fn shown(&self, item: Value) {
        let _ = self.send(Input::Shown(item));
    }
    pub fn accepted(&self, item: Value) {
        let _ = self.send(Input::Accepted(item));
    }
    /// Length is cumulative UTF-16 from the beginning of the original insertText,
    /// including its pre-existing prefix, per the Copilot LSP extension.
    pub fn partially_accepted(&self, item: Value, accepted_length: u32) {
        let _ = self.send(Input::PartiallyAccepted(item, accepted_length));
    }
    pub fn reply(&self, id: Value, result: Value) {
        let _ = self.send(Input::Reply(id, result));
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(e) = self.events.try_recv() {
            changed = true;
            match e {
                Event::Ready => {
                    self.ready = true;
                    self.kind = "Normal".into();
                    self.status = "Servidor Copilot iniciado; verificando conta…".into()
                }
                Event::Status { kind, message } => {
                    // Normal is also emitted before signIn. Only a validated
                    // authentication response grants permission to request code.
                    if kind == "Error" {
                        // Only a previously validated account may arm recovery.
                        // Repeated failures/challenges cannot cause sign-in loops.
                        self.recheck_after_auth_error |= self.authenticated;
                        self.authenticated = false;
                        self.cancel();
                    }
                    let recheck = kind == "Normal"
                        && self.recheck_after_auth_error
                        && self.login.is_none()
                        && !self.login_in_progress;
                    self.kind = kind;
                    self.status = message;
                    if recheck {
                        // Normal also arrives before initial authentication, so
                        // it cannot itself grant access. Revalidate once via the
                        // official signIn response after this recovery edge.
                        self.recheck_after_auth_error = false;
                        if let Err(e) = self.sign_in() {
                            self.error = Some(e);
                        } else {
                            self.status = "Verificando novamente o acesso ao Copilot…".into();
                        }
                    }
                }
                Event::Login(login) => {
                    self.recheck_after_auth_error = false;
                    self.login_in_progress = false;
                    self.authenticated = login.is_none();
                    self.authentication_checked = true;
                    self.login = login;
                    self.status = if self.login.is_some() {
                        "Confirme o código no GitHub"
                    } else {
                        "Login do Copilot concluído"
                    }
                    .into()
                }
                Event::AuthenticationError(error) => {
                    self.recheck_after_auth_error = false;
                    self.login_in_progress = false;
                    self.authenticated = false;
                    self.authentication_checked = true;
                    self.login = None;
                    self.cancel();
                    self.kind = "Error".into();
                    self.status = "Copilot não autenticado".into();
                    self.error = Some(error);
                }
                Event::Suggestion(suggestions, ticket) => {
                    if ticket == self.ticket {
                        self.busy = false;
                        self.error = None;
                        self.clear_suggestions();
                        self.alternatives = suggestions;
                        self.suggestion_count = self.alternatives.len();
                        self.suggestion = self.alternatives.first().cloned();
                    }
                }
                Event::CompletionError { ticket, message } => {
                    if ticket == self.ticket {
                        self.error = Some(message);
                        self.busy = false;
                    }
                }
                Event::Error(e) => {
                    self.error = Some(e);
                    // An auxiliary request (e.g. acceptance telemetry) can fail
                    // while a newer completion is still running. Only that
                    // completion's ticket, authentication failure or Stopped
                    // may finish its busy state.
                }
                Event::Message(m) => {
                    if self.messages.len() < 4 {
                        self.messages.push(m)
                    } else {
                        self.reply(m.id, Value::Null);
                    }
                }
                Event::OpenUrl { id, url } => self.open_url = Some((id, url)),
                Event::SignedOut => {
                    self.recheck_after_auth_error = false;
                    self.cancel();
                    self.authenticated = false;
                    self.login = None;
                    self.status = "Conta Copilot desconectada".into();
                }
                Event::Stopped => {
                    self.recheck_after_auth_error = false;
                    self.login_in_progress = false;
                    self.ready = false;
                    self.authenticated = false;
                    self.cancel();
                    self.kind = "Stopped".into();
                }
            }
        }
        changed
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
pub fn default_program() -> PathBuf {
    std::env::var_os("CODISSEU_COPILOT_SERVER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".local/share/codisseu/tools/copilot/current/copilot-language-server")
        })
}
pub fn file_uri(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("Copilot precisa de caminho absoluto".into());
    }
    let mut s = "file://".to_owned();
    let text = path
        .to_str()
        .ok_or("O caminho precisa ser UTF-8 para o Copilot")?;
    for b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(b) {
            s.push(*b as char)
        } else {
            s.push_str(&format!("%{b:02X}"))
        }
    }
    Ok(s)
}
pub fn language(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "php" | "phtml" => "php",
        "sql" => "sql",
        "md" | "markdown" | "mdown" | "mkd" => "markdown",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "typescriptreact",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "json" | "jsonc" => "json",
        "py" | "pyi" => "python",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "xml" | "svg" => "xml",
        "yml" | "yaml" => "yaml",
        "sh" | "bash" => "shellscript",
        "toml" => "toml",
        "c" | "h" => "c",
        "cpp" | "hpp" => "cpp",
        "rb" => "ruby",
        _ => "plaintext",
    }
}
pub fn position(text: &str, scalar: usize) -> Option<Position> {
    let mut p = Position::default();
    let mut n = 0;
    for c in text.chars() {
        if n == scalar {
            return Some(p);
        }
        if c == '\n' {
            p.line += 1;
            p.character = 0
        } else {
            p.character += c.len_utf16() as u32
        }
        n += 1;
    }
    (n == scalar).then_some(p)
}
fn offset(text: &str, p: Position) -> Option<usize> {
    let mut at = Position::default();
    let mut n = 0;
    for c in text.chars() {
        if at == p {
            return Some(n);
        }
        if c == '\n' {
            at.line += 1;
            at.character = 0
        } else {
            at.character += c.len_utf16() as u32
        }
        n += 1;
    }
    (at == p).then_some(n)
}
pub(super) fn decode(doc: Document, ticket: u64, item: Value) -> Option<Suggestion> {
    let raw = item.get("insertText")?.as_str()?;
    if raw.len() > MAX_SUGGESTION || raw.contains('\0') {
        return None;
    }
    let caret = offset(&doc.text, doc.position)?;
    let (start, end) = if let Some(r) = item.get("range") {
        (
            offset(
                &doc.text,
                serde_json::from_value(r.get("start")?.clone()).ok()?,
            )?,
            offset(
                &doc.text,
                serde_json::from_value(r.get("end")?.clone()).ok()?,
            )?,
        )
    } else {
        (caret, caret)
    };
    if start > end || start > caret || end < caret {
        return None;
    }
    let prefix = doc
        .text
        .chars()
        .skip(start)
        .take(caret - start)
        .collect::<String>();
    let insert = raw.replace("\r\n", "\n").replace('\r', "\n");
    let insert = if doc.text.contains("\r\n") {
        insert.replace('\n', "\r\n")
    } else {
        insert
    };
    let suffix = insert.strip_prefix(&prefix)?.to_owned();
    if suffix.is_empty() {
        return None;
    }
    // A deletion/rewrite of existing suffix needs a diff presentation, not ghost text.
    let existing = doc
        .text
        .chars()
        .skip(caret)
        .take(end - caret)
        .collect::<String>();
    let ghost = projection::project(&existing, &suffix)?.text;
    Some(Suggestion {
        ticket,
        document: doc,
        start,
        end,
        insert,
        suffix: ghost,
        item,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn doc(text: &str, caret: usize) -> Document {
        Document {
            uri: "file:///owned.php".into(),
            version: 1,
            language: "php".into(),
            text: Arc::from(text),
            position: position(text, caret).unwrap(),
        }
    }
    #[test]
    fn copilot_utf16_ranges_prefix_and_crlf_are_exact() {
        let d = doc("😀 ret\r\n", 5);
        let s=decode(d,7,json!({"insertText":"return 42;\n}","range":{"start":{"line":0,"character":3},"end":{"line":0,"character":6}}})).unwrap();
        assert_eq!(s.start, 2);
        assert_eq!(s.end, 5);
        assert_eq!(s.insert, "return 42;\r\n}");
        assert_eq!(s.suffix, "urn 42;\r\n}");
        assert_eq!(
            offset(
                "😀",
                Position {
                    line: 0,
                    character: 1
                }
            ),
            None
        );
    }
    #[test]
    fn copilot_rejects_rewrites_invalid_ranges_snippets_and_oversized_text() {
        for item in [
            json!({"insertText":"other","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}}}),
            json!({"insertText":{"value":"$0"}}),
            json!({"insertText":"x","range":{"start":{"line":2,"character":0},"end":{"line":2,"character":0}}}),
            json!({"insertText":"a".repeat(MAX_SUGGESTION+1)}),
        ] {
            assert!(decode(doc("ret", 3), 1, item).is_none());
        }
        let d = doc("call()", 5);
        let s=decode(d,1,json!({"insertText":"value)","range":{"start":{"line":0,"character":5},"end":{"line":0,"character":6}}})).unwrap();
        assert_eq!(s.suffix, "value");
        assert_eq!(s.insert, "value)");
    }
    #[test]
    fn copilot_multipart_preserves_existing_parenthesis_and_trailing_insertion() {
        let d = doc("call() next", 5);
        let s=decode(d,1,json!({"insertText":"call($fatura);","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":6}}})).unwrap();
        assert_eq!(s.suffix, "$fatura);");
        assert_eq!(s.preserved_suffix(), Some(7..8));
        assert_eq!(s.insert, "call($fatura);");
        let next = s.continued(doc("call($fa) next", 8)).unwrap();
        assert_eq!(next.suffix, "tura);");
        assert_eq!(next.preserved_suffix(), Some(4..5));
        assert_eq!(next.end, 9);
        assert_eq!(next.insert, s.insert);
        assert_eq!(next.item, s.item);
        // Deletions and rewrites still cannot masquerade as inline insertions.
        assert!(decode(doc("call(old)",5),1,json!({"insertText":"call(new);","range":{"start":{"line":0,"character":0},"end":{"line":0,"character":9}}})).is_none());
    }
    #[test]
    fn copilot_any_text_extension_and_incremental_unicode_change() {
        for (name, id) in [
            ("a.php", "php"),
            ("a.sql", "sql"),
            ("a.md", "markdown"),
            ("a.markdown", "markdown"),
            ("a.mdown", "markdown"),
            ("a.MKD", "markdown"),
            ("a.js", "javascript"),
            ("a.ts", "typescript"),
            ("a.unknown", "plaintext"),
        ] {
            assert_eq!(language(Path::new(name)), id)
        }
        assert_eq!(
            worker::incremental("😀 hello\nkept", "😀 ação\nkept"),
            json!({"range":{"start":{"line":0,"character":3},"end":{"line":0,"character":7}},"text":"açã"})
        );
        let uri = file_uri(Path::new("/owned/a #😀.php")).unwrap();
        assert!(uri.contains("%20%23%F0%9F%98%80"));
    }
    #[test]
    fn copilot_framing_enforces_limits_and_lengths() {
        let mut out = Vec::new();
        transport::write(&mut out, &json!({"text":"ação 😀"})).unwrap();
        assert_eq!(
            transport::read(&mut out.as_slice()).unwrap(),
            json!({"text":"ação 😀"})
        );
        for raw in [
            b"Content-Length: 99999999\r\n\r\n".as_slice(),
            b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            b"Content-Length: 3\r\n\r\n{}",
        ] {
            assert!(transport::read(&mut &*raw).is_err());
        }
    }
}
