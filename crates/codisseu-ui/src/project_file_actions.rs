//! Explorer commands are explicit and scoped to the selected item; no extension stubs.
use super::*;
use std::{
    io::Read,
    path::Path,
    sync::mpsc::{self, Receiver},
};
#[derive(Clone, Copy)]
pub(super) enum ExplorerAction {
    Side,
    Source,
    Preview,
    Containing,
    Terminal,
    Find,
    SelectCompare,
    Compare,
    Cut,
    Copy,
    Paste,
    Path,
    Relative,
}
#[derive(Default)]
pub(super) struct ExplorerExtras {
    pub(super) clipboard: Option<(PathBuf, bool)>,
    pub(super) context: Option<egui::Context>,
    compare: Option<PathBuf>,
    job: Option<Receiver<Result<Comparison, String>>>,
    comparison: Option<Comparison>,
    pending_open: Option<OpenRequest>,
    pub(super) moved_clipboard: bool,
    cancel_job: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl Drop for ExplorerExtras {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel_job {
            cancel.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}
#[derive(Clone)]
struct OpenRequest {
    path: PathBuf,
    action: ExplorerAction,
    original: Option<(codisseu_core::ViewId, DocumentId)>,
}
struct Comparison {
    left: PathBuf,
    right: PathBuf,
    lines: Vec<(String, String)>,
}
pub(super) struct MenuState {
    pub(super) git: Option<PathBuf>,
    clipboard: bool,
    compare: bool,
    remotes: Vec<(String, String)>,
    reference: Option<String>,
    busy: bool,
}
#[cfg(test)]
impl MenuState {
    pub(super) fn for_test() -> Self {
        Self {
            git: None,
            clipboard: false,
            compare: false,
            remotes: vec![],
            reference: None,
            busy: false,
        }
    }
}
impl CodisseuApp {
    pub(super) fn project_menu_state(&self, path: &Path) -> MenuState {
        let git = self
            .service
            .git
            .repositories
            .iter()
            .map(|repository| &repository.root)
            .chain(
                self.service
                    .git
                    .repo
                    .iter()
                    .map(|repository| &repository.root),
            )
            .filter(|root| path.starts_with(root))
            .max_by_key(|root| root.components().count())
            .cloned();
        let active = git.as_ref().is_some_and(|root| {
            self.service
                .git
                .repo
                .as_ref()
                .is_some_and(|repository| &repository.root == root)
        });
        MenuState {
            git,
            clipboard: self.project_files_ui.extras.clipboard.is_some(),
            compare: self.project_files_ui.extras.compare.is_some(),
            remotes: if active {
                self.service
                    .git
                    .remotes
                    .iter()
                    .map(|remote| (remote.name.clone(), remote.fetch_url.clone()))
                    .collect()
            } else {
                vec![]
            },
            busy: self.project_files_ui.extras.job.is_some(),
            reference: if active {
                self.service.git.snapshot.as_ref().and_then(|snapshot| {
                    snapshot.head_ref.clone().or_else(|| snapshot.head.clone())
                })
            } else {
                None
            },
        }
    }
    pub(super) fn project_extra_action(&mut self, action: ExplorerAction, path: PathBuf) {
        if matches!(action, ExplorerAction::Containing | ExplorerAction::Compare)
            && self.project_files_ui.extras.job.is_some()
        {
            self.project_files_ui.error = Some("Aguarde a ação de arquivos em andamento.".into());
            return;
        }
        match action {
            ExplorerAction::Copy | ExplorerAction::Cut => {
                // egui-winit consumes physical Ctrl+V and only emits Paste when the
                // OS clipboard contains text. Publish a path for that transport;
                // filesystem Paste still accepts only our validated internal item.
                if let Some(ctx) = &self.project_files_ui.extras.context {
                    ctx.copy_text(path.to_string_lossy().into_owned());
                }
                self.project_files_ui.extras.clipboard =
                    Some((path, matches!(action, ExplorerAction::Cut)));
            }
            ExplorerAction::Paste => {
                if let Some((source, move_item)) = self.project_files_ui.extras.clipboard.clone() {
                    let parent = self.project_parent(&path);
                    match self.service.dispatch(Command::ProjectFile(
                        ProjectFileCommand::Transfer {
                            source,
                            parent,
                            move_item,
                        },
                    )) {
                        Err(error) => {
                            self.service.last_error = None;
                            self.project_files_ui.error = Some(error);
                        }
                        Ok(()) => self.project_files_ui.extras.moved_clipboard = move_item,
                    }
                }
            }
            ExplorerAction::Path | ExplorerAction::Relative => {
                let value = if matches!(action, ExplorerAction::Relative) {
                    self.service
                        .workspace
                        .as_ref()
                        .and_then(|root| path.strip_prefix(root).ok())
                        .unwrap_or(&path)
                } else {
                    &path
                };
                if let Some(ctx) = self.project_files_ui.extras.context.as_ref() {
                    ctx.copy_text(value.to_string_lossy().into_owned());
                }
            }
            ExplorerAction::Side | ExplorerAction::Source | ExplorerAction::Preview => {
                // Opening is asynchronous. Create the target group only once the target
                // document exists, so an empty editor can also open to the side.
                let original = self
                    .service
                    .active_view()
                    .zip(self.service.active_document().map(|document| document.id));
                self.project_files_ui.extras.pending_open = Some(OpenRequest {
                    path: path.clone(),
                    action,
                    original,
                });
                self.send(Command::OpenFile(path));
            }
            ExplorerAction::Terminal => {
                let parent = self.project_parent(&path);
                self.docker_terminal_active = false;
                self.terminal.open_directory(&parent);
                self.show_activity = true;
                self.tool_tab = ToolTab::Terminal;
            }
            ExplorerAction::Find => self.open_project_search_in(self.project_parent(&path)),
            ExplorerAction::Containing => {
                let parent = path.parent().unwrap_or(&path).to_owned();
                let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                self.project_files_ui.extras.cancel_job = Some(cancel.clone());
                let (tx, rx) = mpsc::sync_channel(1);
                self.project_files_ui.extras.job = Some(rx);
                std::thread::spawn(move || {
                    #[cfg(target_os = "linux")]
                    let result = (|| -> Result<(), String> {
                        let mut child = std::process::Command::new("xdg-open")
                            .arg(&parent)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .map_err(|error| format!("Não foi possível abrir a pasta: {error}"))?;
                        let deadline = std::time::Instant::now() + Duration::from_secs(15);
                        loop {
                            match child.try_wait() {
                                Ok(Some(status)) => {
                                    return if status.success() {
                                        Ok(())
                                    } else {
                                        Err("O gerenciador de arquivos não conseguiu abrir a pasta.".into())
                                    };
                                }
                                Ok(None) => (),
                                Err(error) => {
                                    let _ = child.kill();
                                    let _ = child.wait();
                                    return Err(error.to_string());
                                }
                            }
                            if cancel.load(std::sync::atomic::Ordering::Acquire)
                                || std::time::Instant::now() >= deadline
                            {
                                let _ = child.kill();
                                let _ = child.wait();
                                return Err(
                                    "O gerenciador de arquivos não respondeu em 15 segundos."
                                        .into(),
                                );
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    })();
                    #[cfg(not(target_os = "linux"))]
                    let result: Result<(), String> = Err(
                        "Abrir a pasta no sistema ainda está disponível apenas no Linux.".into(),
                    );
                    let _ = tx.send(result.map(|_| Comparison {
                        left: PathBuf::new(),
                        right: PathBuf::new(),
                        lines: vec![],
                    }));
                });
            }
            ExplorerAction::SelectCompare => self.project_files_ui.extras.compare = Some(path),
            ExplorerAction::Compare => {
                if self.service.documents.iter().any(|document| {
                    document.is_dirty()
                        && (document.path == path
                            || self.project_files_ui.extras.compare.as_ref()
                                == Some(&document.path))
                }) {
                    self.project_files_ui.error = Some(
                        "Salve os dois arquivos antes de comparar as versões em disco.".into(),
                    );
                    return;
                }
                if let Some(left) = self.project_files_ui.extras.compare.clone() {
                    let (tx, rx) = mpsc::sync_channel(1);
                    self.project_files_ui.extras.job = Some(rx);
                    std::thread::spawn(move || {
                        let _ = tx.send(read_comparison(left, path));
                    });
                }
            }
        }
    }
    fn project_parent(&self, path: &Path) -> PathBuf {
        if self.service.workspace.as_deref() == Some(path)
            || self
                .service
                .entries
                .iter()
                .any(|entry| entry.path == path && entry.is_dir)
        {
            path.to_owned()
        } else {
            path.parent().unwrap_or(path).to_owned()
        }
    }
    pub(super) fn project_extra_tick(&mut self) {
        if let Some(job) = &self.project_files_ui.extras.job {
            if let Some(ctx) = &self.project_files_ui.extras.context {
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            match job.try_recv() {
                Ok(result) => {
                    self.project_files_ui.extras.job = None;
                    self.project_files_ui.extras.cancel_job = None;
                    match result {
                        Ok(comparison) if !comparison.left.as_os_str().is_empty() => {
                            self.project_files_ui.extras.comparison = Some(comparison)
                        }
                        Ok(_) => (),
                        Err(error) => self.project_files_ui.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.project_files_ui.extras.job = None;
                    self.project_files_ui.extras.cancel_job = None;
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        if let Some(OpenRequest {
            path,
            action,
            original,
        }) = self.project_files_ui.extras.pending_open.clone()
        {
            if self
                .service
                .active_document()
                .is_some_and(|d| d.path == path)
            {
                self.project_files_ui.extras.pending_open = None;
                if matches!(action, ExplorerAction::Side) {
                    let target = self.service.active_document().unwrap().id;
                    if let Some((view, id)) = original {
                        self.send(Command::ActivateTab { view, id });
                    }
                    self.split_editor(false);
                    let source_view = original.map(|(view, _)| view);
                    let destination = if source_view.is_some() {
                        self.service
                            .document_views()
                            .iter()
                            .find(|view| Some(view.id) != source_view)
                            .map(|view| view.id)
                    } else {
                        self.service.document_views().last().map(|view| view.id)
                    };
                    if let Some(view) = destination {
                        self.send(Command::ActivateView { view });
                        // OpenFile prefers an existing owner by design. Activating
                        // the already-loaded canonical document explicitly adds it
                        // to this group without rereading or redirecting to the left.
                        self.send(Command::Activate { id: target });
                    }
                }
                if let Some(view) = self.service.active_view() {
                    if matches!(action, ExplorerAction::Preview) {
                        self.markdown
                            .set_mode(view, &path, markdown::MarkdownMode::Preview);
                    }
                    if matches!(action, ExplorerAction::Source) {
                        self.markdown
                            .set_mode(view, &path, markdown::MarkdownMode::Source);
                    }
                }
                self.focus_editor = true;
            } else if !self.service.is_busy() {
                self.project_files_ui.extras.pending_open = None;
            }
        }
    }
    pub(super) fn project_extra_shortcuts(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        root: bool,
    ) -> bool {
        if (root
            || self
                .service
                .entries
                .iter()
                .any(|entry| entry.path == path && entry.is_dir))
            && consume_exact(ctx, Modifiers::CTRL, Key::Enter)
        {
            return true;
        }
        let event = ctx.input_mut(|input| {
            let at = input.events.iter().position(|event| {
                matches!(
                    event,
                    egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_)
                )
            });
            at.map(|index| input.events.remove(index))
        });
        let command = match event {
            Some(egui::Event::Copy) if !root => Some(ExplorerAction::Copy),
            Some(egui::Event::Cut) if !root => Some(ExplorerAction::Cut),
            Some(egui::Event::Paste(_)) => Some(ExplorerAction::Paste),
            _ => None,
        };
        let command = command.or_else(|| {
            if !root && consume_exact(ctx, Modifiers::CTRL, Key::C) {
                Some(ExplorerAction::Copy)
            } else if !root && consume_exact(ctx, Modifiers::CTRL, Key::X) {
                Some(ExplorerAction::Cut)
            } else if consume_exact(ctx, Modifiers::CTRL, Key::V) {
                Some(ExplorerAction::Paste)
            } else if consume_exact(ctx, Modifiers::CTRL.plus(Modifiers::ALT), Key::C) {
                Some(ExplorerAction::Path)
            } else if consume_exact(
                ctx,
                Modifiers::CTRL.plus(Modifiers::ALT).plus(Modifiers::SHIFT),
                Key::C,
            ) {
                Some(ExplorerAction::Relative)
            } else if consume_exact(ctx, Modifiers::CTRL.plus(Modifiers::ALT), Key::R) {
                Some(ExplorerAction::Containing)
            } else if consume_exact(ctx, Modifiers::ALT.plus(Modifiers::SHIFT), Key::F) {
                Some(ExplorerAction::Find)
            } else if !root
                && !self
                    .service
                    .entries
                    .iter()
                    .any(|entry| entry.path == path && entry.is_dir)
                && consume_exact(ctx, Modifiers::CTRL, Key::Enter)
            {
                Some(ExplorerAction::Side)
            } else {
                None
            }
        });
        if let Some(command) = command {
            self.project_extra_action(command, path.to_owned());
            true
        } else {
            false
        }
    }
    pub(super) fn project_extra_dialog(&mut self, ctx: &egui::Context) {
        let Some(comparison) = self.project_files_ui.extras.comparison.as_ref() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Compare Files")
            .id(egui::Id::unique("explorer.compare"))
            .open(&mut open)
            .default_size((900., 560.))
            .show(ctx, |ui| {
                ui.columns(2, |columns| {
                    columns[0].label(comparison.left.display().to_string());
                    columns[1].label(comparison.right.display().to_string());
                });
                ui.label(
                    RichText::new("Read-only · line-by-line comparison · saved files")
                        .small()
                        .color(color(self.tokens.muted)),
                );
                ui.separator();
                egui::ScrollArea::both()
                    .id_salt("explorer.compare.scroll")
                    .codisseu()
                    .show_rows(ui, 20., comparison.lines.len(), |ui, range| {
                        for index in range {
                            let (left, right) = &comparison.lines[index];
                            ui.columns(2, |columns| {
                                for (column, text) in columns.iter_mut().zip([left, right]) {
                                    column.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(format!("{}", index + 1))
                                                .monospace()
                                                .color(color(self.tokens.muted)),
                                        );
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(text).monospace().color(color(
                                                    if left == right {
                                                        self.tokens.text
                                                    } else {
                                                        self.tokens.accent
                                                    },
                                                )),
                                            )
                                            .truncate(),
                                        )
                                        .on_hover_text(text);
                                    });
                                }
                            });
                        }
                    });
            });
        if !open {
            self.project_files_ui.extras.comparison = None;
        }
    }
}
fn read_comparison(left: PathBuf, right: PathBuf) -> Result<Comparison, String> {
    fn read(path: &Path) -> Result<String, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.len() > 1024 * 1024 {
            return Err("A comparação aceita arquivos de texto regulares até 1 MiB.".into());
        }
        let mut text = String::new();
        file.take(1024 * 1024 + 1)
            .read_to_string(&mut text)
            .map_err(|_| "Arquivo binário ou texto sem UTF-8.".to_owned())?;
        if text.len() > 1024 * 1024 {
            return Err("Arquivo cresceu além do limite de comparação.".into());
        }
        Ok(text)
    }
    let a = read(&left)?;
    let b = read(&right)?;
    let mut a = a.lines();
    let mut b = b.lines();
    let mut lines = vec![];
    loop {
        let x = a.next();
        let y = b.next();
        if x.is_none() && y.is_none() {
            break;
        }
        if lines.len() >= 50_000
            || x.is_some_and(|line| line.len() > 16_384)
            || y.is_some_and(|line| line.len() > 16_384)
        {
            return Err("Comparação limitada a 50 mil linhas e 16 KiB por linha.".into());
        }
        lines.push((x.unwrap_or_default().into(), y.unwrap_or_default().into()));
    }
    Ok(Comparison { left, right, lines })
}
fn item(ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool) -> bool {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(label)
            .shortcut_text(shortcut)
            .min_size(egui::vec2(300., 28.)),
    );
    if label == "Delete" {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, response.enabled(), label)
        });
        ui.ctx()
            .accesskit_node_builder(response.id, |node| node.set_keyboard_shortcut("Delete"));
    }
    response.clicked()
}
pub(super) fn menu_contents(
    ui: &mut egui::Ui,
    path: &Path,
    folder: bool,
    root: bool,
    state: &MenuState,
    action: &mut Option<Action>,
) {
    let parent = if folder {
        path.to_owned()
    } else {
        path.parent().unwrap_or(path).to_owned()
    };
    let mut extra =
        |ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, key: ExplorerAction| {
            if item(ui, label, shortcut, enabled) {
                *action = Some(Action::Extra(key, path.to_owned()));
                ui.close();
            }
        };
    if folder {
        for (label, kind) in [("New File…", false), ("New Folder…", true)] {
            if item(ui, label, "", true) {
                *action = Some(Action::New(parent.clone(), kind));
                ui.close();
            }
        }
    } else {
        extra(
            ui,
            "Open to the Side",
            "Ctrl+Enter",
            true,
            ExplorerAction::Side,
        );
    }
    // Reborrow after the new-file actions: each section has consistent text and shortcut columns.
    let mut extra =
        |ui: &mut egui::Ui, label: &str, shortcut: &str, enabled: bool, key: ExplorerAction| {
            if item(ui, label, shortcut, enabled) {
                *action = Some(Action::Extra(key, path.to_owned()));
                ui.close();
            }
        };
    extra(
        ui,
        "Open Containing Folder",
        "Alt+Ctrl+R",
        !state.busy,
        ExplorerAction::Containing,
    );
    if !folder {
        ui.menu_button("Open With…", |ui| {
            extra(ui, "Text Editor", "", true, ExplorerAction::Source);
            if path.extension().and_then(|v| v.to_str()).is_some_and(|v| {
                matches!(v.to_ascii_lowercase().as_str(), "md" | "markdown" | "mdown")
            }) {
                extra(ui, "Markdown Preview", "", true, ExplorerAction::Preview);
            }
        });
    }
    extra(
        ui,
        "Open in Integrated Terminal",
        "",
        true,
        ExplorerAction::Terminal,
    );
    if folder {
        ui.separator();
        extra(
            ui,
            "Find in Folder…",
            "Alt+Shift+F",
            true,
            ExplorerAction::Find,
        );
    }
    if !folder {
        ui.separator();
        extra(
            ui,
            "Select for Compare",
            "",
            true,
            ExplorerAction::SelectCompare,
        );
        if state.compare {
            extra(
                ui,
                "Compare with Selected",
                "",
                !state.busy,
                ExplorerAction::Compare,
            );
        }
    }
    let has_git = state
        .git
        .as_ref()
        .is_some_and(|root| path.starts_with(root));
    if has_git {
        ui.separator();
        ui.menu_button("Open Changes", |ui| {
            for (key, label) in [("diff", "Compare with Index"), ("stage", "Stage Changes")] {
                if item(ui, label, "", key != "diff" || !folder) {
                    *action = Some(Action::Git(key, path.to_owned()));
                    ui.close();
                }
            }
        });
        let urls = remote_urls(state, path, folder);
        if !urls.is_empty() {
            ui.menu_button("Open on Remote (Web)", |ui| {
                for (name, url) in &urls {
                    if item(ui, name, "", true) {
                        *action = Some(Action::Remote(url.clone(), false));
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        if item(
            ui,
            if folder {
                "Folder History"
            } else {
                "File History"
            },
            "",
            true,
        ) {
            *action = Some(Action::Git("history", path.to_owned()));
            ui.close();
        }
        if item(ui, "Open Timeline", "", true) {
            *action = Some(Action::Git("history", path.to_owned()));
            ui.close();
        }
        if !folder && item(ui, "Annotate with Git Blame", "", true) {
            *action = Some(Action::Git("blame", path.to_owned()));
            ui.close();
        }
    }
    ui.separator();
    for (label, shortcut, enabled, key) in [
        ("Cut", "Ctrl+X", !root, ExplorerAction::Cut),
        ("Copy", "Ctrl+C", !root, ExplorerAction::Copy),
        ("Paste", "Ctrl+V", state.clipboard, ExplorerAction::Paste),
    ] {
        if item(ui, label, shortcut, enabled) {
            *action = Some(Action::Extra(key, path.to_owned()));
            ui.close();
        }
    }
    ui.separator();
    for (label, shortcut, key) in [
        ("Copy Path", "Alt+Ctrl+C", ExplorerAction::Path),
        (
            "Copy Relative Path",
            "Alt+Ctrl+Shift+C",
            ExplorerAction::Relative,
        ),
    ] {
        if item(ui, label, shortcut, true) {
            *action = Some(Action::Extra(key, path.to_owned()));
            ui.close();
        }
    }
    let urls = remote_urls(state, path, folder);
    if let Some((_, url)) = urls.first()
        && item(ui, "Copy Remote File URL", "", true)
    {
        *action = Some(Action::Remote(url.clone(), true));
        ui.close();
    }
    if urls.len() > 1 {
        ui.menu_button("Copy Remote File URL From…", |ui| {
            for (name, url) in &urls {
                if item(ui, name, "", true) {
                    *action = Some(Action::Remote(url.clone(), true));
                    ui.close();
                }
            }
        });
    }
    if !root {
        ui.separator();
        if item(ui, "Rename…", "F2", true) {
            *action = Some(Action::Rename(path.to_owned()));
            ui.close();
        }
        if item(ui, "Delete", "Del", true) {
            *action = Some(Action::Trash(path.to_owned()));
            ui.close();
        }
    }
}

fn encode_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}
/// Only known hosted URL conventions; strip userinfo and query credentials.
fn remote_urls(state: &MenuState, path: &Path, folder: bool) -> Vec<(String, String)> {
    let Some(relative) = state
        .git
        .as_ref()
        .and_then(|root| path.strip_prefix(root).ok())
    else {
        return vec![];
    };
    let Some(reference) = state.reference.as_deref() else {
        return vec![];
    };
    let reference = encode_segment(reference.strip_prefix("refs/heads/").unwrap_or(reference));
    let relative = relative
        .components()
        .map(|component| encode_segment(&component.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/");
    state
        .remotes
        .iter()
        .filter_map(|(name, remote)| {
            let remote = remote.split(['?', '#']).next()?;
            let (host, repo) = if let Some((_, rest)) = remote.split_once("://") {
                let (host, repo) = rest.split_once('/')?;
                (host.rsplit('@').next()?, repo)
            } else {
                let (host, repo) = remote.split_once(':')?;
                (host.rsplit('@').next()?, repo)
            };
            let mode = match host {
                "github.com" => {
                    if folder {
                        "tree"
                    } else {
                        "blob"
                    }
                }
                "gitlab.com" => {
                    if folder {
                        "-/tree"
                    } else {
                        "-/blob"
                    }
                }
                "bitbucket.org" => "src",
                _ => return None,
            };
            let repo = repo
                .strip_suffix(".git")
                .unwrap_or(repo)
                .trim_end_matches('/');
            if repo.is_empty() || repo.contains(['\\', ' ', '\n', '\r']) {
                return None;
            }
            Some((
                name.clone(),
                format!("https://{host}/{repo}/{mode}/{reference}/{relative}"),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_urls_strip_credentials_encode_paths_and_reject_unknown_hosts() {
        let mut state = MenuState {
            git: Some(PathBuf::from("/fixture")),
            clipboard: false,
            compare: false,
            remotes: vec![(
                "origin".into(),
                "https://person:secret@github.com/team/repo.git?token=secret".into(),
            )],
            reference: Some("refs/heads/feature/a".into()),
            busy: false,
        };
        assert_eq!(
            remote_urls(&state, Path::new("/fixture/a ç.php"), false)[0].1,
            "https://github.com/team/repo/blob/feature%2Fa/a%20%C3%A7.php"
        );
        state.remotes[0].1 = "git@gitlab.com:team/repo.git".into();
        assert!(
            remote_urls(&state, Path::new("/fixture/src"), true)[0]
                .1
                .contains("/-/tree/")
        );
        state.remotes[0].1 = "https://unknown.example/team/repo.git".into();
        assert!(remote_urls(&state, Path::new("/fixture/a.php"), false).is_empty());
        assert!(remote_urls(&state, Path::new("/outside/a.php"), false).is_empty());
    }
    #[test]
    fn comparison_reads_saved_utf8_lines_and_rejects_large_files_and_binary() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "ação\nold\n").unwrap();
        std::fs::write(&b, "ação\nnew\nextra\n").unwrap();
        let comparison = read_comparison(a.clone(), b.clone()).unwrap();
        assert_eq!(comparison.lines.len(), 3);
        assert_eq!(comparison.lines[1], ("old".into(), "new".into()));
        std::fs::write(&b, [0xff]).unwrap();
        assert!(read_comparison(a.clone(), b.clone()).is_err());
        std::fs::write(&b, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(read_comparison(a, b).is_err());
    }
    #[test]
    fn workspace_disposal_cancels_owned_external_job() {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut extras = ExplorerExtras::default();
        extras.cancel_job = Some(cancel.clone());
        drop(extras);
        assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
    }
}
