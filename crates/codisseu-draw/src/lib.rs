//! Native Excalidraw scene model, bounded editing history and safe file storage.
//!
//! The JSON format is interoperable; the renderer is implemented by Codisseu.
//! File operations are synchronous and must run on the application's I/O worker.

mod document;
mod model;
mod storage;
mod svg;

pub use document::DrawingDocument;
pub use model::{Element, ElementKind, MAX_ELEMENTS, MAX_SCENE_BYTES, Point, Scene};
pub use storage::{DiskSnapshot, load_scene, save_scene};

pub type Result<T> = std::result::Result<T, String>;
