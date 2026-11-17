//! Bounded asynchronous Debug Adapter Protocol sessions, independent of a GUI.
mod browser_bridge;
mod php_runtime;
mod service;
mod transport;
mod types;
mod worker;
pub use browser_bridge::{BrowserDebugBridge, BrowserDebugEndpoint, BrowserInspectUri};
pub use service::DebugService;
pub use types::*;
