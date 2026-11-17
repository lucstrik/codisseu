//! Shared, typed IPC. Little-endian lengths precede UTF-8 JSON and bounded BGRA/CDP payloads.
use serde::{Deserialize, Serialize};
/// Privileged SDK frontend URL. Debug formatting never reveals the bearer nonce.
#[derive(Clone, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SecretUrl(String);
impl SecretUrl {
    pub fn devtools(value: String) -> Result<Self, String> {
        const PREFIX: &str =
            "devtools://devtools/bundled/inspector.html?can_dock=false&ws=127.0.0.1:";
        let invalid = || "Invalid private DevTools frontend endpoint".to_owned();
        const DEFAULT: &str =
            "devtools://devtools/bundled/devtools_app.html?can_dock=false&ws=127.0.0.1:";
        let endpoint = value
            .strip_prefix(DEFAULT)
            .or_else(|| value.strip_prefix(PREFIX))
            .ok_or_else(invalid)?;
        let (port, nonce) = endpoint.split_once('/').ok_or_else(invalid)?;
        let number: u16 = port.parse().map_err(|_| invalid())?;
        if number < 1024
            || port != number.to_string()
            || nonce.len() != 64
            || !nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(invalid());
        }
        Ok(Self(value))
    }
    /// For the isolated engine's CEF LoadURL only; never publish to page state or logs.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for SecretUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretUrl([REDACTED])")
    }
}
impl<'de> Deserialize<'de> for SecretUrl {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::devtools(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}
pub const VERSION: u32 = 2;
pub const MAX_JSON: usize = 64 * 1024;
pub const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
pub const MAX_TABS: usize = 8;
pub const MAX_CDP: usize = 4 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 8192;
pub type BrowserId = u64;
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Surface {
    Page,
    DevTools,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum KeyKind {
    RawDown,
    Up,
    Char,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
    pub left_button: bool,
    pub middle_button: bool,
    pub right_button: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClipboardAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Command {
    Open {
        id: BrowserId,
        url: String,
    },
    Close {
        id: BrowserId,
    },
    Navigate {
        id: BrowserId,
        url: String,
    },
    Back {
        id: BrowserId,
    },
    Forward {
        id: BrowserId,
    },
    Reload {
        id: BrowserId,
        ignore_cache: bool,
    },
    Stop {
        id: BrowserId,
    },
    Resize {
        id: BrowserId,
        surface: Surface,
        width: u32,
        height: u32,
        scale: f32,
    },
    Visible {
        id: BrowserId,
        surface: Surface,
        visible: bool,
    },
    Focus {
        id: BrowserId,
        surface: Surface,
        focused: bool,
    },
    MouseMove {
        id: BrowserId,
        surface: Surface,
        x: i32,
        y: i32,
        leave: bool,
        modifiers: Modifiers,
    },
    MouseClick {
        id: BrowserId,
        surface: Surface,
        x: i32,
        y: i32,
        button: MouseButton,
        up: bool,
        clicks: u8,
        modifiers: Modifiers,
    },
    MouseWheel {
        id: BrowserId,
        surface: Surface,
        x: i32,
        y: i32,
        delta_x: i32,
        delta_y: i32,
        modifiers: Modifiers,
    },
    Key {
        id: BrowserId,
        surface: Surface,
        kind: KeyKind,
        windows_key_code: i32,
        native_key_code: i32,
        modifiers: Modifiers,
        character: Option<char>,
        unmodified: Option<char>,
    },
    /// Commit text through Chromium IME. This never inserts text into an IDE document.
    Text {
        id: BrowserId,
        surface: Surface,
        text: String,
    },
    Composition {
        id: BrowserId,
        surface: Surface,
        text: String,
        selection_start: i32,
        selection_end: i32,
    },
    DevTools {
        id: BrowserId,
        open: bool,
    },
    DevToolsFrontend {
        id: BrowserId,
        url: SecretUrl,
    },
    Inspect {
        id: BrowserId,
        x: i32,
        y: i32,
    },
    Clipboard {
        id: BrowserId,
        surface: Surface,
        action: ClipboardAction,
    },
    DevToolsSubscribe {
        id: BrowserId,
        epoch: u64,
    },
    DevToolsMessage {
        id: BrowserId,
        epoch: u64,
    },
    DevToolsUnsubscribe {
        id: BrowserId,
        epoch: u64,
    },
    Shutdown,
}
impl Command {
    pub fn id(&self) -> Option<BrowserId> {
        match self {
            Self::Shutdown => None,
            Self::Open { id, .. }
            | Self::Close { id }
            | Self::Navigate { id, .. }
            | Self::Back { id }
            | Self::Forward { id }
            | Self::Reload { id, .. }
            | Self::Stop { id }
            | Self::Resize { id, .. }
            | Self::Visible { id, .. }
            | Self::Focus { id, .. }
            | Self::MouseMove { id, .. }
            | Self::MouseClick { id, .. }
            | Self::MouseWheel { id, .. }
            | Self::Key { id, .. }
            | Self::Text { id, .. }
            | Self::Composition { id, .. }
            | Self::DevTools { id, .. }
            | Self::DevToolsFrontend { id, .. }
            | Self::Inspect { id, .. }
            | Self::Clipboard { id, .. }
            | Self::DevToolsSubscribe { id, .. }
            | Self::DevToolsMessage { id, .. }
            | Self::DevToolsUnsubscribe { id, .. } => Some(*id),
        }
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineVersions {
    pub cef: String,
    pub chromium: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum Event {
    Ready {
        protocol: u32,
        versions: EngineVersions,
    },
    State {
        id: BrowserId,
        url: String,
        title: String,
        loading: bool,
        can_go_back: bool,
        can_go_forward: bool,
        error: Option<String>,
    },
    Frame {
        id: BrowserId,
        surface: Surface,
        width: u32,
        height: u32,
        seq: u64,
    },
    Closed {
        id: BrowserId,
    },
    DevToolsClosed {
        id: BrowserId,
    },
    Popup {
        id: BrowserId,
        url: String,
    },
    Error {
        id: Option<BrowserId>,
        message: String,
        fatal: bool,
    },
    DevToolsMessage {
        id: BrowserId,
        epoch: u64,
    },
    DevToolsDetached {
        id: BrowserId,
        epoch: u64,
        error: Option<String>,
    },
}
#[derive(Debug)]
pub struct Packet<T> {
    pub message: T,
    pub payload: Vec<u8>,
}
pub fn read<T: serde::de::DeserializeOwned>(
    reader: &mut impl std::io::Read,
) -> Result<Packet<T>, String> {
    let mut header = [0; 8];
    reader.read_exact(&mut header).map_err(|e| e.to_string())?;
    let json = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
    let bytes = u32::from_le_bytes(header[4..].try_into().unwrap()) as usize;
    if json == 0 || json > MAX_JSON || bytes > MAX_PAYLOAD {
        return Err("Browser IPC frame exceeds bounds".into());
    }
    let mut body = vec![0; json];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    let message =
        serde_json::from_slice(&body).map_err(|e| format!("Invalid browser IPC JSON: {e}"))?;
    let mut payload = vec![0; bytes];
    reader.read_exact(&mut payload).map_err(|e| e.to_string())?;
    Ok(Packet { message, payload })
}
pub fn write<T: Serialize>(
    writer: &mut impl std::io::Write,
    message: &T,
    payload: &[u8],
) -> Result<(), String> {
    let json = serde_json::to_vec(message).map_err(|e| e.to_string())?;
    if json.is_empty() || json.len() > MAX_JSON || payload.len() > MAX_PAYLOAD {
        return Err("Browser IPC frame exceeds bounds".into());
    }
    writer
        .write_all(&(json.len() as u32).to_le_bytes())
        .and_then(|_| writer.write_all(&(payload.len() as u32).to_le_bytes()))
        .and_then(|_| writer.write_all(&json))
        .and_then(|_| writer.write_all(payload))
        .map_err(|e| e.to_string())
}
pub fn frame_bytes(width: u32, height: u32) -> Result<usize, String> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("Invalid browser surface dimensions".into());
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or("Browser surface overflow")?;
    if bytes > MAX_PAYLOAD {
        return Err("Browser surface exceeds 32 MiB".into());
    }
    Ok(bytes)
}

/// Resize uses DIP/CSS units; OnPaint Frame dimensions are physical pixels.
pub fn physical_size(width: u32, height: u32, scale: f32) -> Result<(u32, u32), String> {
    if !scale.is_finite()
        || !(0.5..=4.).contains(&scale)
        || width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
    {
        return Err("Invalid browser viewport/scale".into());
    }
    let w = (width as f64 * scale as f64).ceil() as u32;
    let h = (height as f64 * scale as f64).ceil() as u32;
    frame_bytes(w, h)?;
    Ok((w, h))
}
pub fn validate_command(command: &Command) -> Result<(), String> {
    if command.id() == Some(0) {
        return Err("Browser tab id must be positive".into());
    }
    match command {
        Command::DevToolsSubscribe { epoch, .. }
        | Command::DevToolsMessage { epoch, .. }
        | Command::DevToolsUnsubscribe { epoch, .. }
            if *epoch == 0 =>
        {
            return Err("CDP epoch must be positive".into());
        }
        Command::Open { url, .. } | Command::Navigate { url, .. } => {
            crate::validate_url(url)?;
        }
        Command::Resize {
            width,
            height,
            scale,
            ..
        } => {
            physical_size(*width, *height, *scale)?;
        }
        Command::MouseClick { clicks, .. } if !(1..=3).contains(clicks) => {
            return Err("Invalid browser click count".into());
        }
        Command::MouseMove { x, y, .. }
        | Command::MouseClick { x, y, .. }
        | Command::MouseWheel { x, y, .. }
        | Command::Inspect { x, y, .. }
            if x.unsigned_abs() > 32768 || y.unsigned_abs() > 32768 =>
        {
            return Err("Browser pointer out of bounds".into());
        }
        Command::MouseWheel {
            delta_x, delta_y, ..
        } if delta_x.unsigned_abs() > 65536 || delta_y.unsigned_abs() > 65536 => {
            return Err("Browser wheel out of bounds".into());
        }
        Command::Composition {
            text,
            selection_start,
            selection_end,
            ..
        } => {
            let units = text.encode_utf16().count();
            let boundary = |value: i32| {
                if value < 0 {
                    return false;
                }
                let mut at = 0usize;
                if value == 0 {
                    return true;
                }
                for c in text.chars() {
                    at += c.len_utf16();
                    if at == value as usize {
                        return true;
                    }
                    if at > value as usize {
                        return false;
                    }
                }
                false
            };
            if text.len() > 16384
                || text.contains('\0')
                || selection_start > selection_end
                || *selection_end as usize > units
                || !boundary(*selection_start)
                || !boundary(*selection_end)
            {
                return Err("Invalid browser IME composition/UTF16 range".into());
            }
        }
        Command::Text { text, .. } if text.len() > 16384 || text.contains('\0') => {
            return Err("Browser IME text exceeds bound or contains NUL".into());
        }
        _ => {}
    }
    if serde_json::to_vec(command)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_JSON
    {
        return Err("Browser command JSON exceeds 64 KiB".into());
    }
    Ok(())
}
fn event_payload(event: &Event, bytes: usize) -> Result<(), String> {
    match event {
        Event::Frame {
            id,
            width,
            height,
            seq,
            ..
        } => {
            if *id == 0 || *seq == 0 || frame_bytes(*width, *height)? != bytes {
                return Err("Invalid browser frame dimensions/length/id/sequence".into());
            }
        }
        Event::DevToolsMessage { id, epoch } => {
            if *id == 0 || *epoch == 0 || bytes == 0 || bytes > MAX_CDP {
                return Err("Invalid CDP event payload/id/epoch".into());
            }
        }
        _ if bytes != 0 => return Err("Unexpected browser binary payload".into()),
        Event::DevToolsDetached { id, epoch, error }
            if *id == 0 || *epoch == 0 || error.as_ref().is_some_and(|e| e.len() > 8192) =>
        {
            return Err("Invalid CDP detached event".into());
        }
        Event::Ready { protocol, versions } => {
            if *protocol != VERSION || versions.cef.len() > 4096 || versions.chromium.len() > 4096 {
                return Err("Incompatible browser protocol/version".into());
            }
        }
        Event::State {
            id,
            url,
            title,
            error,
            ..
        } => {
            if *id == 0 || title.len() > 4096 || error.as_ref().is_some_and(|e| e.len() > 8192) {
                return Err("Invalid browser state metadata".into());
            }
            crate::validate_url(url)?;
        }
        Event::Popup { id, url } => {
            if *id == 0 {
                return Err("Invalid browser popup owner".into());
            }
            crate::validate_url(url)?;
        }
        Event::Error { message, .. } if message.len() > 8192 => {
            return Err("Browser error exceeds bound".into());
        }
        Event::Closed { id } | Event::DevToolsClosed { id } if *id == 0 => {
            return Err("Invalid browser tab id".into());
        }
        _ => {}
    }
    Ok(())
}
fn header(reader: &mut impl std::io::Read) -> Result<(usize, usize), String> {
    let mut h = [0; 8];
    reader.read_exact(&mut h).map_err(|e| e.to_string())?;
    let a = u32::from_le_bytes(h[..4].try_into().unwrap()) as usize;
    let b = u32::from_le_bytes(h[4..].try_into().unwrap()) as usize;
    if a == 0 || a > MAX_JSON || b > MAX_PAYLOAD {
        return Err("Browser IPC frame exceeds bounds".into());
    }
    Ok((a, b))
}
/// Validates frame dimensions and non-frame payload rules before allocating BGRA.
pub fn read_event(reader: &mut impl std::io::Read) -> Result<Packet<Event>, String> {
    let (a, b) = header(reader)?;
    let mut json = vec![0; a];
    reader.read_exact(&mut json).map_err(|e| e.to_string())?;
    let message =
        serde_json::from_slice(&json).map_err(|e| format!("Invalid browser event JSON: {e}"))?;
    event_payload(&message, b)?;
    let mut payload = vec![0; b];
    reader.read_exact(&mut payload).map_err(|e| e.to_string())?;
    if matches!(&message, Event::DevToolsMessage { .. }) {
        validate_cdp(&payload)?;
    }
    Ok(Packet { message, payload })
}
pub fn read_command_packet(reader: &mut impl std::io::Read) -> Result<Packet<Command>, String> {
    let (a, b) = header(reader)?;
    let mut json = vec![0; a];
    reader.read_exact(&mut json).map_err(|e| e.to_string())?;
    let command =
        serde_json::from_slice(&json).map_err(|e| format!("Invalid browser command JSON: {e}"))?;
    validate_command(&command)?;
    command_payload(&command, b)?;
    let mut payload = vec![0; b];
    reader.read_exact(&mut payload).map_err(|e| e.to_string())?;
    if matches!(&command, Command::DevToolsMessage { .. }) {
        validate_cdp(&payload)?;
    }
    Ok(Packet {
        message: command,
        payload,
    })
}
pub fn read_command(reader: &mut impl std::io::Read) -> Result<Command, String> {
    let p = read_command_packet(reader)?;
    if !p.payload.is_empty() {
        return Err("Use read_command_packet for CDP binary payload".into());
    }
    Ok(p.message)
}
pub fn write_event(
    writer: &mut impl std::io::Write,
    event: &Event,
    payload: &[u8],
) -> Result<(), String> {
    event_payload(event, payload.len())?;
    if matches!(event, Event::DevToolsMessage { .. }) {
        validate_cdp(payload)?;
    }
    write(writer, event, payload)
}
pub fn write_command(writer: &mut impl std::io::Write, command: &Command) -> Result<(), String> {
    write_command_packet(writer, command, &[])
}
pub fn write_command_packet(
    writer: &mut impl std::io::Write,
    command: &Command,
    payload: &[u8],
) -> Result<(), String> {
    validate_command(command)?;
    command_payload(command, payload.len())?;
    if matches!(command, Command::DevToolsMessage { .. }) {
        validate_cdp(payload)?;
    }
    write(writer, command, payload)
}
fn command_payload(command: &Command, bytes: usize) -> Result<(), String> {
    if matches!(command, Command::DevToolsMessage { .. }) {
        if bytes == 0 || bytes > MAX_CDP {
            return Err("CDP payload exceeds 4 MiB or is empty".into());
        }
    } else if bytes != 0 {
        return Err("Unexpected browser command binary payload".into());
    }
    Ok(())
}
pub fn validate_cdp(payload: &[u8]) -> Result<(), String> {
    if payload.is_empty() || payload.len() > MAX_CDP {
        return Err("CDP JSON exceeds 4 MiB".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(payload).map_err(|_| "Invalid CDP UTF8 JSON")?;
    if !value.is_object() {
        return Err("CDP message must be a JSON object".into());
    }
    Ok(())
}
