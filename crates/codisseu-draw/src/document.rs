use std::collections::VecDeque;
use std::path::Path;

use crate::{DiskSnapshot, Result, Scene, load_scene, save_scene};

const MAX_HISTORY_ENTRIES: usize = 64;
const MAX_HISTORY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
struct HistoryEntry {
    scene: Scene,
    bytes: usize,
}

#[derive(Clone, Debug)]
pub struct DrawingDocument {
    scene: Scene,
    saved_scene: Scene,
    disk: DiskSnapshot,
    undo: VecDeque<HistoryEntry>,
    redo: VecDeque<HistoryEntry>,
    revision: u64,
}

impl DrawingDocument {
    pub fn new(scene: Scene) -> Result<Self> {
        scene.to_json()?;
        Ok(Self {
            saved_scene: scene.clone(),
            scene,
            disk: DiskSnapshot::default(),
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            revision: 1,
        })
    }

    pub fn open(path: &Path) -> Result<Self> {
        let (scene, disk) = load_scene(path)?;
        let mut document = Self::new(scene)?;
        document.disk = disk;
        Ok(document)
    }

    pub fn create(path: &Path, scene: Scene) -> Result<Self> {
        let mut document = Self::new(scene)?;
        document.save_as(path)?;
        Ok(document)
    }

    pub fn scene(&self) -> &Scene {
        &self.scene
    }
    pub fn path(&self) -> Option<&Path> {
        self.disk.path()
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn is_dirty(&self) -> bool {
        self.scene != self.saved_scene
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn disk_snapshot(&self) -> &DiskSnapshot {
        &self.disk
    }

    pub fn edit(&mut self, edit: impl FnOnce(&mut Scene)) -> Result<bool> {
        let mut next = self.scene.clone();
        edit(&mut next);
        self.replace_scene(next)
    }

    /// One committed gesture is one history entry; intermediate previews should
    /// remain in the canvas until the user releases the pointer.
    pub fn replace_scene(&mut self, next: Scene) -> Result<bool> {
        next.to_json()?;
        if next == self.scene {
            return Ok(false);
        }
        let bytes = self.scene.to_json()?.len();
        let old = std::mem::replace(&mut self.scene, next);
        Self::push(&mut self.undo, HistoryEntry { scene: old, bytes });
        self.redo.clear();
        self.revision = self.revision.saturating_add(1);
        Ok(true)
    }

    fn push(history: &mut VecDeque<HistoryEntry>, entry: HistoryEntry) {
        history.push_back(entry);
        while history.len() > MAX_HISTORY_ENTRIES
            || history.iter().map(|entry| entry.bytes).sum::<usize>() > MAX_HISTORY_BYTES
        {
            history.pop_front();
        }
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        let bytes = self.scene.to_json().map_or(0, |value| value.len());
        let current = std::mem::replace(&mut self.scene, previous.scene);
        Self::push(
            &mut self.redo,
            HistoryEntry {
                scene: current,
                bytes,
            },
        );
        self.revision = self.revision.saturating_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        let bytes = self.scene.to_json().map_or(0, |value| value.len());
        let current = std::mem::replace(&mut self.scene, next.scene);
        Self::push(
            &mut self.undo,
            HistoryEntry {
                scene: current,
                bytes,
            },
        );
        self.revision = self.revision.saturating_add(1);
        true
    }

    pub fn save(&mut self) -> Result<()> {
        let path = self
            .path()
            .ok_or_else(|| "Escolha um caminho para salvar o desenho".to_owned())?
            .to_owned();
        self.disk = save_scene(&path, &self.scene, &self.disk)?;
        self.saved_scene = self.scene.clone();
        Ok(())
    }

    pub fn save_as(&mut self, path: &Path) -> Result<()> {
        if self.path() == Some(path) {
            return self.save();
        }
        self.disk = save_scene(path, &self.scene, &DiskSnapshot::default())?;
        self.saved_scene = self.scene.clone();
        Ok(())
    }
}
