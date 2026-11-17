//! Platform folder selection. The native dialog waits outside the render thread.
use super::*;
use std::sync::{
    Arc,
    mpsc::{self, Receiver, TryRecvError},
};

type Backend = Arc<dyn Fn(Option<PathBuf>) -> Option<PathBuf> + Send + Sync>;
#[derive(Default)]
pub(super) struct FolderPicker {
    template: rfd::FileDialog,
    context: Option<egui::Context>,
    backend: Option<Backend>,
    pending: Option<Pending>,
}
struct Pending {
    origin: Option<PathBuf>,
    receive: Receiver<Option<PathBuf>>,
}
struct Selection {
    origin: Option<PathBuf>,
    result: Result<Option<PathBuf>, String>,
}
impl FolderPicker {
    fn request(&mut self, origin: Option<PathBuf>, workspace_file: bool) -> Result<bool, String> {
        if self.pending.is_some() {
            return Ok(false);
        }
        let start = origin
            .as_ref()
            .map(|path| {
                if codisseu_core::workspace_file::is_workspace_file(path) {
                    path.parent().unwrap_or(path).to_owned()
                } else {
                    path.clone()
                }
            })
            .or_else(|| {
                std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
                    .map(PathBuf::from)
            });
        let mut dialog = self.template.clone().set_title("Abrir projeto");
        if let Some(start) = &start {
            dialog = dialog.set_directory(start);
        }
        let backend = self.backend.clone();
        let context = self.context.clone();
        let (send, receive) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("codisseu-folder-picker".into())
            .spawn(move || {
                let selected = match backend {
                    Some(backend) => backend(start),
                    None if workspace_file => dialog
                        .set_title("Abrir workspace")
                        .add_filter("Workspace", &["code-workspace"])
                        .pick_file(),
                    None => dialog.pick_folder(),
                };
                let _ = send.send(selected);
                if let Some(context) = context {
                    context.request_repaint();
                }
            })
            .map_err(|e| format!("Não foi possível abrir o seletor de pastas: {e}"))?;
        self.pending = Some(Pending { origin, receive });
        Ok(true)
    }
    fn poll(&mut self) -> Option<Selection> {
        let pending = self.pending.as_ref()?;
        let result = match pending.receive.try_recv() {
            Ok(path) => Ok(path),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => {
                Err("O seletor de pastas foi encerrado inesperadamente".into())
            }
        };
        Some(Selection {
            origin: self.pending.take().unwrap().origin,
            result,
        })
    }
}
impl CodisseuApp {
    /// Supply a platform dialog adapter for embedded hosts and headless tests.
    /// Selected paths still pass through the ordinary workspace/trust service.
    pub fn set_folder_picker(
        &mut self,
        picker: impl Fn(Option<PathBuf>) -> Option<PathBuf> + Send + Sync + 'static,
    ) -> Result<(), String> {
        if self.folder_picker.pending.is_some() {
            return Err("Já existe um seletor aberto".into());
        }
        self.folder_picker.backend = Some(Arc::new(picker));
        Ok(())
    }
    pub(super) fn open_native_folder(&mut self) {
        match self
            .folder_picker
            .request(self.service.workspace_path().map(PathBuf::from), false)
        {
            Ok(true) => self.path_dialog = Some(PathAction::Workspace),
            Ok(false) => (),
            Err(error) => self.service.status = error,
        }
    }
    pub(super) fn open_native_workspace_file(&mut self) {
        match self
            .folder_picker
            .request(self.service.workspace_path().map(PathBuf::from), true)
        {
            Ok(true) => self.path_dialog = Some(PathAction::WorkspaceFile),
            Ok(false) => (),
            Err(error) => self.service.status = error,
        }
    }
    pub(super) fn poll_native_folder(&mut self, ctx: &egui::Context) {
        self.folder_picker.context = Some(ctx.clone());
        if let Some(selection) = self.folder_picker.poll() {
            if matches!(
                self.path_dialog,
                Some(PathAction::Workspace | PathAction::WorkspaceFile)
            ) {
                self.path_dialog = None;
            }
            // A late selection must not replace a different workspace opened meanwhile.
            if !self.exit_when_saved && self.service.workspace_path() == selection.origin.as_deref()
            {
                match selection.result {
                    Ok(Some(path)) => self.send(Command::OpenWorkspace(path)),
                    Ok(None) => (),
                    Err(error) => self.service.status = error,
                }
            }
        }
        if self.folder_picker.pending.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.poll_workspace_switch(ctx);
    }
    pub(super) fn abandon_native_folder(&mut self) {
        self.folder_picker.pending = None;
        if matches!(
            self.path_dialog,
            Some(PathAction::Workspace | PathAction::WorkspaceFile)
        ) {
            self.path_dialog = None;
        }
    }
    /// Whether an operating-system folder selection is in progress.
    pub fn folder_picker_pending(&self) -> bool {
        self.folder_picker.pending.is_some()
    }
    pub(super) fn parent_native_folder(&mut self, frame: &eframe::Frame) {
        self.folder_picker.template = rfd::FileDialog::new().set_parent(frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant},
    };
    fn app() -> (tempfile::TempDir, CodisseuApp) {
        let dir = tempfile::tempdir().unwrap();
        let app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        (dir, app)
    }
    fn settle(app: &mut CodisseuApp) {
        let ctx = egui::Context::default();
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll_native_folder(&ctx);
            app.service.poll();
            if !app.folder_picker_pending()
                && !app.service.is_busy()
                && (!app.workspace_switch_pending() || app.service.last_error.is_some())
            {
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn selected_folder_uses_normal_workspace_validation_and_does_not_start_providers() {
        let (dir, mut app) = app();
        let root = dir.path().join("projeto ação");
        std::fs::create_dir(&root).unwrap();
        let selected = root.clone();
        app.set_folder_picker(move |_| Some(selected.clone()))
            .unwrap();
        app.open_path(PathAction::Workspace);
        settle(&mut app);
        assert_eq!(app.service.workspace, Some(root));
        assert!(!app.service.providers_running());
        assert!(app.path_dialog.is_none());
        app.set_folder_picker(|_| Some(PathBuf::from("/codisseu-absent-native-fixture")))
            .unwrap();
        let before = app.service.workspace.clone();
        app.open_path(PathAction::Workspace);
        settle(&mut app);
        assert_eq!(app.service.workspace, before);
        assert!(app.service.last_error.is_some());
    }
    #[test]
    fn cancel_and_duplicate_request_preserve_current_workspace_and_dirty_document() {
        let (dir, mut app) = app();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let path = root.join("a.txt");
        std::fs::write(&path, "saved").unwrap();
        app.service
            .dispatch(Command::OpenWorkspace(root.clone()))
            .unwrap();
        settle(&mut app);
        app.service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        settle(&mut app);
        let doc = app.service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        app.service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: "dirty".into(),
            })
            .unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let called = count.clone();
        let start = root.clone();
        app.set_folder_picker(move |initial| {
            assert_eq!(initial, Some(start.clone()));
            called.fetch_add(1, Ordering::Relaxed);
            None
        })
        .unwrap();
        app.open_path(PathAction::Workspace);
        app.open_path(PathAction::Workspace);
        settle(&mut app);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert_eq!(app.service.workspace, Some(root));
        assert_eq!(app.service.active_document().unwrap().id, id);
        assert_eq!(app.service.active_document().unwrap().text(), "dirty");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "saved");
    }
    #[test]
    fn stale_result_cannot_replace_a_different_workspace() {
        let (dir, mut app) = app();
        let chosen = dir.path().join("chosen");
        let newer = dir.path().join("newer");
        std::fs::create_dir(&chosen).unwrap();
        std::fs::create_dir(&newer).unwrap();
        let (send, receive) = mpsc::sync_channel(1);
        app.folder_picker.pending = Some(Pending {
            origin: None,
            receive,
        });
        app.path_dialog = Some(PathAction::Workspace);
        app.service
            .dispatch(Command::OpenWorkspace(newer.clone()))
            .unwrap();
        while app.service.is_busy() {
            app.service.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        send.send(Some(chosen)).unwrap();
        settle(&mut app);
        assert_eq!(app.service.workspace, Some(newer));
        assert!(app.path_dialog.is_none());
    }
    #[test]
    fn welcome_and_shortcut_request_the_folder_adapter_without_a_path_textbox() {
        let (dir, mut app) = app();
        let root = dir.path().join("selected");
        std::fs::create_dir(&root).unwrap();
        let chosen = root.clone();
        app.set_folder_picker(move |_| Some(chosen.clone()))
            .unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        h.get_by_label("Abrir projeto    Ctrl+Shift+O").click();
        h.step();
        let until = Instant::now() + Duration::from_secs(10);
        while h.state().service.workspace.is_none()
            || h.state().folder_picker_pending()
            || h.state().workspace_switch_pending()
            || h.state().service.is_busy()
        {
            h.step();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(h.state().service.workspace, Some(root));
        assert!(h.query_by_label("Caminho").is_none());
        let count = Arc::new(AtomicUsize::new(0));
        let called = count.clone();
        h.state_mut()
            .set_folder_picker(move |_| {
                called.fetch_add(1, Ordering::Relaxed);
                None
            })
            .unwrap();
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O);
        h.step();
        while h.state().folder_picker_pending() {
            h.step();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
}
