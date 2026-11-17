//! UI-driven policy; edits themselves remain buffer-only commands.
use super::*;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct State {
    observed: HashMap<DocumentId, (u64, Instant)>,
    blocked: HashSet<DocumentId>,
    in_flight: HashSet<DocumentId>,
}
impl State {
    pub fn reset_timers(&mut self) {
        self.observed.clear();
    }
    pub fn failed(&mut self, id: DocumentId) {
        if self.in_flight.remove(&id) {
            self.blocked.insert(id);
        }
    }
    pub fn saved(&mut self, id: DocumentId) {
        self.in_flight.remove(&id);
        self.blocked.remove(&id);
        // A newer version may have been edited during the write. Start its delay
        // after this ACK rather than immediately queueing another disk write.
        self.observed.remove(&id);
    }
}
impl AppService {
    /// Called by the interactive host after polling completions. No filesystem
    /// calls occur here; every write goes through the usual guarded Save worker.
    pub fn tick_auto_save(&mut self) {
        self.tick_auto_save_at(Instant::now());
    }
    fn tick_auto_save_at(&mut self, now: Instant) {
        let ids: HashSet<_> = self.documents.iter().map(|d| d.id).collect();
        self.auto_save.observed.retain(|id, _| ids.contains(id));
        self.auto_save.blocked.retain(|id| ids.contains(id));
        if !self.editor_preferences.auto_save {
            return;
        }
        // Serialize against all filesystem work and semantic transactions. This
        // includes pending Git mutations and project moves/deletes.
        let guarded = !self.pending.is_empty()
            || self.rename.is_open()
            || self.rename_stage.is_some()
            || self.project_files.pending;
        let delay = Duration::from_millis(
            self.editor_preferences
                .auto_save_delay_ms
                .clamp(250, 60_000),
        );
        let mut due = None;
        for doc in &self.documents {
            if !doc.is_dirty()
                || doc.untitled
                || doc.read_only
                || doc.disk_missing
                || self.document_external_change(doc.id).is_some()
            {
                self.auto_save.observed.remove(&doc.id);
                continue;
            }
            let git_guarded = self.git_path_applying(&doc.path);
            let entry = self
                .auto_save
                .observed
                .entry(doc.id)
                .or_insert((doc.version, now));
            if entry.0 != doc.version {
                *entry = (doc.version, now);
            }
            if !guarded
                && !git_guarded
                && !self.auto_save.blocked.contains(&doc.id)
                && !self.auto_save.in_flight.contains(&doc.id)
                && now.saturating_duration_since(entry.1) >= delay
            {
                due = Some(doc.id);
                break;
            }
        }
        if let Some(id) = due {
            match self.dispatch(Command::Save { id }) {
                Ok(()) => {
                    self.auto_save.in_flight.insert(id);
                }
                Err(error) => {
                    self.auto_save.blocked.insert(id);
                    self.status = error.clone();
                    self.last_error = Some(error);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settle(s: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(15);
        while s.is_busy() {
            s.poll();
            assert!(Instant::now() < end, "{}", s.status);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn open(s: &mut AppService, path: &std::path::Path) -> DocumentId {
        s.dispatch(Command::OpenFile(path.into())).unwrap();
        settle(s);
        s.active.unwrap()
    }
    fn edit(s: &mut AppService, id: DocumentId, text: &str) {
        let version = s.documents.iter().find(|d| d.id == id).unwrap().version;
        s.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: text.into(),
        })
        .unwrap();
    }
    #[test]
    fn debounce_all_documents_disable_and_undo_after_real_write() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.php");
        let b = dir.path().join("b.ts");
        std::fs::write(&a, "original\r\n").unwrap();
        std::fs::write(&b, "original\n").unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        let id = open(&mut s, &a);
        let second = open(&mut s, &b);
        assert!(s.editor_preferences().auto_save);
        edit(&mut s, id, "ação 😀\r\n");
        edit(&mut s, second, "const n = 2;\n");
        let t = Instant::now();
        s.tick_auto_save_at(t);
        s.tick_auto_save_at(t + Duration::from_millis(999));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "original\r\n");
        edit(&mut s, id, "nova ação 😀\r\n");
        s.tick_auto_save_at(t + Duration::from_millis(999));
        s.tick_auto_save_at(t + Duration::from_millis(1100));
        settle(&mut s);
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "const n = 2;\n");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "original\r\n");
        s.tick_auto_save_at(t + Duration::from_millis(2000));
        settle(&mut s);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "nova ação 😀\r\n");
        s.dispatch(Command::Undo { id }).unwrap();
        s.dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 1000,
        })
        .unwrap();
        s.tick_auto_save_at(t + Duration::from_secs(5));
        s.tick_auto_save_at(t + Duration::from_secs(8));
        settle(&mut s);
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "nova ação 😀\r\n");
        assert!(s.documents.iter().find(|d| d.id == id).unwrap().is_dirty());
    }
    #[test]
    fn failed_external_conflict_pauses_retries_until_manual_save_ack() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "base").unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        let id = open(&mut s, &path);
        edit(&mut s, id, "local");
        std::fs::write(&path, "external").unwrap();
        let t = Instant::now();
        s.tick_auto_save_at(t);
        s.tick_auto_save_at(t + Duration::from_secs(2));
        settle(&mut s);
        assert!(s.last_error.is_some());
        assert!(s.auto_save.blocked.contains(&id));
        edit(&mut s, id, "more local");
        s.tick_auto_save_at(t + Duration::from_secs(4));
        s.tick_auto_save_at(t + Duration::from_secs(6));
        assert!(s.pending.is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(s.active_document().unwrap().text(), "more local");
        std::fs::write(&path, "base").unwrap();
        s.dispatch(Command::Save { id }).unwrap();
        settle(&mut s);
        assert!(!s.auto_save.blocked.contains(&id));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "more local");
    }
    #[test]
    fn known_external_conflict_never_retries_automatic_save_after_dismissing_ui() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conflict.txt");
        std::fs::write(&path, "base").unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        let id = open(&mut s, &path);
        edit(&mut s, id, "local");
        std::fs::write(&path, "external").unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while s.document_external_change(id).is_none() {
            s.poll();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        s.last_error = None;
        let now = Instant::now();
        s.tick_auto_save_at(now);
        s.tick_auto_save_at(now + Duration::from_secs(5));
        edit(&mut s, id, "more local");
        s.tick_auto_save_at(now + Duration::from_secs(10));
        s.tick_auto_save_at(now + Duration::from_secs(20));
        assert!(!s.document_save_pending(id));
        assert!(s.last_error.is_none());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "external");
        assert_eq!(s.active_document().unwrap().text(), "more local");
    }
    #[test]
    fn newer_edit_during_write_remains_dirty_and_untitled_never_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "base").unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        let id = open(&mut s, &path);
        edit(&mut s, id, "first");
        let t = Instant::now();
        s.tick_auto_save_at(t);
        s.tick_auto_save_at(t + Duration::from_secs(2));
        edit(&mut s, id, "second");
        settle(&mut s);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        assert!(s.active_document().unwrap().is_dirty());
        s.tick_auto_save_at(t + Duration::from_secs(3));
        s.tick_auto_save_at(t + Duration::from_secs(5));
        settle(&mut s);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        s.dispatch(Command::NewUntitled).unwrap();
        let untitled = s.active.unwrap();
        edit(&mut s, untitled, "untitled");
        s.tick_auto_save_at(t + Duration::from_secs(6));
        s.tick_auto_save_at(t + Duration::from_secs(8));
        assert!(s.pending.is_empty());
        assert!(s.active_document().unwrap().is_untitled());
        assert!(s.active_document().unwrap().is_dirty());
    }
    #[test]
    fn preferences_migrate_and_roundtrip_without_text_changes() {
        let old: EditorPreferences = serde_json::from_str(r#"{"ligatures":false}"#).unwrap();
        assert!(old.auto_save);
        assert_eq!(old.auto_save_delay_ms, 1000);
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session.json");
        let mut s = AppService::new(session.clone());
        assert!(
            s.dispatch(Command::SetAutoSave {
                enabled: true,
                delay_ms: 0
            })
            .is_err()
        );
        s.dispatch(Command::SetAutoSave {
            enabled: false,
            delay_ms: 2500,
        })
        .unwrap();
        settle(&mut s);
        s.dispatch(Command::PersistSession).unwrap();
        settle(&mut s);
        let mut restored = AppService::new(session);
        restored.dispatch(Command::RestoreSession).unwrap();
        settle(&mut restored);
        assert!(!restored.editor_preferences().auto_save);
        assert_eq!(restored.editor_preferences().auto_save_delay_ms, 2500);
    }
}
