//! Browser process boundary. The IDE never links CEF; all I/O runs off the GUI.
mod cdp;
pub mod protocol;
mod service;
mod urls;
pub use cdp::{DevToolsChannel, DevToolsEvent, DevToolsReceiver, DevToolsSender};
mod worker;
pub use protocol::{
    BrowserId, ClipboardAction, Command, EngineVersions, Event, KeyKind, Modifiers, MouseButton,
    SecretUrl, Surface,
};
pub use service::{BrowserService, EngineConfig, EngineState, Frame, TabState};
pub use urls::{normalize_url, validate_url};

pub mod inspector;
