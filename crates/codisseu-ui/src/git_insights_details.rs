//! Native, read-only commit inspection and line history. Git never runs on the GUI thread.
use super::{Tokens, color};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_core::git::{
    CommitDetails, CommitEntry, Repository,
    autoria::{HistoryRequest, HistoryWorker, LineHistory},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

#[derive(Clone, Debug)]
pub(super) enum Action {
    OpenDiff {
        root: PathBuf,
        base: Option<String>,
        head: String,
        path: Option<PathBuf>,
    },
    OpenFileHistory {
        root: PathBuf,
        path: PathBuf,
    },
    RevealCommit {
        root: PathBuf,
        oid: String,
    },
    OpenHistoricalFile {
        root: PathBuf,
        oid: String,
        path: PathBuf,
        line: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    root: PathBuf,
    oid: String,
    path: PathBuf,
}
struct Read {
    key: Key,
    cancel: Arc<AtomicBool>,
}
struct Loaded {
    key: Key,
    root: PathBuf,
    path: PathBuf,
    metadata: CommitEntry,
    details: CommitDetails,
    file_patch: String,
    diff: Arc<super::git_diff::DiffData>,
    whole_commit: bool,
}
struct Reply {
    key: Key,
    result: Result<Loaded, String>,
}
struct Reader {
    send: Option<SyncSender<Read>>,
    receive: Receiver<Reply>,
    thread: Option<JoinHandle<()>>,
}
impl Reader {
    fn new() -> Result<Self, String> {
        let (send, requests) = mpsc::sync_channel::<Read>(1);
        let (replies, receive) = mpsc::sync_channel::<Reply>(1);
        let thread = std::thread::Builder::new()
            .name("git-commit-inspection".into())
            .spawn(move || {
                while let Ok(read) = requests.recv() {
                    let result = load(&read);
                    if replies
                        .send(Reply {
                            key: read.key,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            send: Some(send),
            receive,
            thread: Some(thread),
        })
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.send.take();
        // Dropping the receiver unblocks a worker publishing its last bounded reply.
        let (_, empty) = mpsc::sync_channel(1);
        drop(std::mem::replace(&mut self.receive, empty));
        if let Some(thread) = self.thread.take() {
            // Process cancellation/reaping can take time; never join from an egui frame.
            let _ = std::thread::Builder::new()
                .name("git-inspection-reaper".into())
                .spawn(move || {
                    let _ = thread.join();
                });
        }
    }
}
fn load(read: &Read) -> Result<Loaded, String> {
    let repo = Repository::discover(&read.key.root, &read.cancel).map_err(|e| e.to_string())?;
    if repo.info().root != read.key.root {
        return Err("O repositório da revisão mudou".into());
    }
    let mut details = repo
        .commit_details(&read.key.oid, &read.cancel)
        .map_err(|e| e.to_string())?;
    let whole_commit = read.key.path.as_os_str().is_empty();
    let path = if whole_commit {
        details
            .changes
            .first()
            .map(|change| change.path.clone())
            .unwrap_or_default()
    } else {
        repo.relative_path(&read.key.path)
            .map_err(|error| error.to_string())?
    };
    let mut metadata = repo
        .history_reference(0, 1, None, Some(&details.commit), &read.cancel)
        .map_err(|e| e.to_string())?
        .commits
        .into_iter()
        .next()
        .ok_or("Revisão sem metadados")?;
    let file_patch = if whole_commit {
        bounded(&String::from_utf8_lossy(&details.patch), 128 * 1024)
    } else {
        let patch = repo
            .revision_diff(
                details.parents.first().map(String::as_str),
                &details.commit,
                Some(&path),
                &read.cancel,
            )
            .map_err(|e| e.to_string())?;
        bounded(&String::from_utf8_lossy(&patch.bytes), 128 * 1024)
    };
    // The inspector renders a bounded file patch; retain no second whole-commit payload.
    details.patch = Vec::new();
    details.message = bounded(&details.message, 64 * 1024);
    // Metadata labels use Unicode scalar limits plus an explicit ellipsis; their
    // work is bounded even when Git's otherwise valid output contains megabytes.
    metadata.subject = metadata_label(&metadata.subject, 512);
    metadata.author = metadata_label(&metadata.author, 200);
    metadata.email = metadata_label(&metadata.email, 320);
    let diff = Arc::new(super::git_diff::DiffData::inspection(
        file_patch.clone(),
        &path,
    ));
    Ok(Loaded {
        key: read.key.clone(),
        root: repo.info().root,
        path,
        metadata,
        details,
        file_patch,
        diff,
        whole_commit,
    })
}

#[derive(Default)]
pub(super) struct DetailsUi {
    reader: Option<Reader>,
    desired: Option<Key>,
    active: Option<(Key, Arc<AtomicBool>)>,
    loaded: Option<Loaded>,
    patch: Option<(Key, usize, String)>,
    commit_open: bool,
    diff_view: super::git_diff_view::DiffView,
    diff_unified: bool,
    diff_no_wrap: bool,
    line: usize,
    history_worker: Option<HistoryWorker>,
    history_id: Option<u64>,
    history_open: bool,
    history_foreground: bool,
    history_pinned: bool,
    history: Option<LineHistory>,
    history_path: PathBuf,
    history_version: u64,
    history_error: Option<String>,
    pub(super) error: Option<String>,
}
impl Drop for DetailsUi {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(worker) = &self.history_worker {
            worker.cancel();
        }
    }
}
impl DetailsUi {
    pub(super) fn cancel(&mut self) {
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(worker) = &self.history_worker {
            worker.cancel();
        }
        self.desired = None;
        self.history_id = None;
        self.commit_open = false;
        self.history_open = false;
        self.loaded = None;
        self.patch = None;
        self.history = None;
    }
    pub(super) fn is_busy(&self) -> bool {
        self.active.is_some()
            || self
                .history_worker
                .as_ref()
                .is_some_and(HistoryWorker::is_busy)
    }
    pub(super) fn foreground_busy(&self, root: &Path) -> bool {
        (self.commit_open
            && self
                .active
                .as_ref()
                .is_some_and(|(key, _)| key.root == root))
            || (self.history_open
                && self.history_path.starts_with(root)
                && self.history_foreground
                && self
                    .history_worker
                    .as_ref()
                    .is_some_and(HistoryWorker::is_busy))
    }
    pub(super) fn follows_cursor(&self) -> bool {
        self.history_open && !self.history_pinned
    }
    pub(super) fn open_commit(&mut self, root: PathBuf, oid: String, path: PathBuf, line: usize) {
        self.commit_open = false;
        if self.error.is_some() {
            self.desired = None;
            self.error = None;
        }
        self.prefetch_commit(root, oid, path, line);
        self.commit_open = true;
    }
    pub(super) fn prefetch_commit(
        &mut self,
        root: PathBuf,
        oid: String,
        path: PathBuf,
        line: usize,
    ) {
        let key = Key { root, oid, path };
        if self.commit_open && self.desired.as_ref().is_some_and(|current| *current != key) {
            return;
        }
        self.line = line.max(1);
        if self.desired.as_ref() == Some(&key) {
            return;
        }
        if let Some((_, cancel)) = &self.active {
            cancel.store(true, Ordering::SeqCst);
        }
        self.desired = Some(key);
        self.loaded = None;
        self.patch = None;
        self.error = None;
        self.start_pending();
    }
    pub(super) fn hover_patch(
        &mut self,
        root: &Path,
        oid: &str,
        path: &Path,
        line: usize,
    ) -> Option<&str> {
        let key = Key {
            root: root.into(),
            oid: oid.into(),
            path: path.into(),
        };
        let loaded = self.loaded.as_ref().filter(|loaded| loaded.key == key)?;
        if !self
            .patch
            .as_ref()
            .is_some_and(|(k, l, _)| *k == key && *l == line)
        {
            self.patch =
                hunk_for_line(&loaded.file_patch, line).map(|text| (key.clone(), line, text));
        }
        self.patch
            .as_ref()
            .filter(|(k, l, _)| *k == key && *l == line)
            .map(|(_, _, text)| text.as_str())
    }
    pub(super) fn previous_diff(&self, root: &Path, oid: &str, path: &Path) -> Option<Action> {
        let loaded = self.loaded.as_ref().filter(|loaded| {
            loaded.key.root == root && loaded.key.oid == oid && loaded.key.path == path
        })?;
        Some(Action::OpenDiff {
            root: loaded.root.clone(),
            base: loaded.details.parents.first().cloned(),
            head: loaded.details.commit.clone(),
            path: (!loaded.whole_commit).then(|| loaded.path.clone()),
        })
    }
    pub(super) fn open_history(
        &mut self,
        workspace: PathBuf,
        path: PathBuf,
        document_version: u64,
        contents: Arc<str>,
        start: usize,
        end: usize,
    ) {
        let worker = self.history_worker.get_or_insert_with(HistoryWorker::new);
        self.history_open = true;
        self.history_foreground = true;
        self.history = None;
        self.history_error = None;
        self.history_path = path.clone();
        self.history_version = document_version;
        self.history_id = match worker.request(HistoryRequest {
            workspace,
            path,
            document_version,
            contents,
            start,
            end,
            limit: 50,
        }) {
            Ok(id) => Some(id),
            Err(error) => {
                self.history_error = Some(error.to_string());
                None
            }
        };
    }
    pub(super) fn silence_automatic_history_progress(&mut self) {
        self.history_foreground = false;
    }
    fn start_pending(&mut self) {
        if self.active.is_some() {
            return;
        }
        let Some(key) = self
            .desired
            .clone()
            .filter(|key| !self.loaded.as_ref().is_some_and(|l| l.key == *key))
        else {
            return;
        };
        if self.error.is_some() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        if self.reader.is_none() {
            match Reader::new() {
                Ok(reader) => self.reader = Some(reader),
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        }
        let reader = self.reader.as_ref().unwrap();
        if reader.send.as_ref().is_some_and(|send| {
            send.try_send(Read {
                key: key.clone(),
                cancel: cancel.clone(),
            })
            .is_ok()
        }) {
            self.active = Some((key, cancel));
        }
    }
    fn poll(&mut self) {
        if let Some(reader) = &self.reader {
            while let Ok(reply) = reader.receive.try_recv() {
                self.active = None;
                if self.desired.as_ref() != Some(&reply.key) {
                    continue;
                }
                match reply.result {
                    Ok(loaded) => {
                        self.loaded = Some(loaded);
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        }
        self.start_pending();
        if let Some(worker) = &self.history_worker {
            while let Some(reply) = worker.poll() {
                if Some(reply.id) != self.history_id
                    || reply.document_version != self.history_version
                    || reply.path != self.history_path
                {
                    continue;
                }
                self.history_id = None;
                match reply.result {
                    Ok(history) => self.history = Some(history),
                    Err(error) => self.history_error = Some(error.to_string()),
                }
            }
        }
    }
    pub(super) fn show(&mut self, ctx: &egui::Context, tokens: &Tokens) -> Vec<Action> {
        self.poll();
        if self.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
        let mut actions = Vec::new();
        let mut inspect = None;
        let mut open = self.commit_open;
        if open {
            let width = (ctx.content_rect().width() - 48.).clamp(260., 1100.);
            let height = (ctx.content_rect().height() - 48.).clamp(260., 800.);
            let modal=egui::Modal::new(egui::Id::unique("git.insights.commit"))
                .frame(egui::Frame::popup(&ctx.global_style()).fill(color(tokens.panel)).inner_margin(16))
                .show(ctx, |ui| {
                    tokens.form_style(ui);
                    ui.set_width(width-32.);
                    ui.spacing_mut().item_spacing.y=8.;
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Detalhes do commit").strong().size(16.));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                            if ui.button("Fechar").clicked(){open=false;}
                        });
                    });
                    ui.separator();
                    if let Some(loaded)=&self.loaded {
                        egui::Frame::NONE.fill(color(tokens.bar)).inner_margin(12).show(ui,|ui| {
                            ui.set_width((width-56.).max(120.));
                            ui.add(egui::Label::new(egui::RichText::new(&loaded.metadata.subject).strong().size(16.)).wrap());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new("Autor").color(color(tokens.muted)));
                                ui.add(egui::Label::new(format!("{} <{}>",loaded.metadata.author,loaded.metadata.email)).wrap());
                                ui.label(egui::RichText::new(super::git_log::git_date(loaded.metadata.timestamp)).color(color(tokens.muted)));
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label(egui::RichText::new(&loaded.details.commit).monospace().color(color(tokens.git_modified)));
                                if ui.small_button("Copiar SHA").clicked(){ctx.copy_text(loaded.details.commit.clone());}
                            });
                        });
                        ui.horizontal_wrapped(|ui| {
                            if ui.button("Abrir no Git Log").clicked(){actions.push(Action::RevealCommit{root:loaded.root.clone(),oid:loaded.details.commit.clone()});}
                            if ui.button("Comparar com anterior").clicked(){actions.push(Action::OpenDiff{root:loaded.root.clone(),base:loaded.details.parents.first().cloned(),head:loaded.details.commit.clone(),path:(!loaded.whole_commit).then(||loaded.path.clone())});}
                            if ui.add_enabled(!loaded.path.as_os_str().is_empty(),egui::Button::new("Histórico do arquivo")).clicked(){actions.push(Action::OpenFileHistory{root:loaded.root.clone(),path:loaded.path.clone()});}
                            if ui.add_enabled(!loaded.path.as_os_str().is_empty(),egui::Button::new("Arquivo nesta revisão")).clicked(){actions.push(Action::OpenHistoricalFile{root:loaded.root.clone(),oid:loaded.details.commit.clone(),path:loaded.path.clone(),line:self.line});}
                        });
                        egui::ScrollArea::vertical().id_salt("git.insights.commit.metadata").max_height((height*0.22).min(150.)).codisseu().show(ui,|ui| {
                            let message=loaded.details.message.strip_prefix(&loaded.metadata.subject).unwrap_or(&loaded.details.message).trim();
                            if !message.is_empty() {
                                ui.add(egui::Label::new(message).wrap().selectable(true));
                            }
                            ui.collapsing(format!("Arquivos alterados ({})",loaded.details.changes.len()),|ui| {
                                for change in loaded.details.changes.iter().take(500) {
                                    let ink=match change.status{'A'=>tokens.git_added,'D'=>tokens.git_deleted,_=>tokens.git_modified};
                                    if ui.selectable_label(false,egui::RichText::new(format!("{}  {}",change.status,change.path.display())).color(color(ink))).clicked(){actions.push(Action::OpenDiff{root:loaded.root.clone(),base:loaded.details.parents.first().cloned(),head:loaded.details.commit.clone(),path:Some(change.path.clone())});}
                                }
                                if loaded.details.changes.len()>500 {ui.label("Lista limitada aos primeiros 500 arquivos. Abra o Git Log para navegar.");}
                            });
                        });
                        ui.separator();
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(if loaded.whole_commit{"Alterações do commit".into()}else{loaded.path.display().to_string()}).strong());
                            ui.selectable_value(&mut self.diff_unified,false,"Lado a lado");
                            ui.selectable_value(&mut self.diff_unified,true,"Unificado");
                            let mut wrap=!self.diff_no_wrap;
                            if ui.checkbox(&mut wrap,"Quebrar linhas").changed(){self.diff_no_wrap = !wrap;}
                        });
                        let body=(height-32.-ui.min_rect().height()).max(90.);
                        egui::Frame::NONE.fill(color(tokens.editor)).inner_margin(8).show(ui,|ui| {
                            self.diff_view.show(ui,loaded.diff.clone(),tokens,self.diff_unified,!self.diff_no_wrap,body-16.);
                        });
                    } else if let Some(error)=&self.error {crate::status_messages::status_message(ui,error);}
                    else {ui.label("Consultando commit…");}
                });
            if modal.should_close() || !actions.is_empty() {
                open = false;
            }
        }
        self.commit_open = open;
        let mut open = self.history_open;
        if open {
            egui::Window::new("Histórico da linha / seleção").id(egui::Id::unique("git.insights.line.history"))
                .open(&mut open).default_width(690.).default_height(440.).show(ctx, |ui| {
                    tokens.form_style(ui);
                    ui.checkbox(&mut self.history_pinned, "Fixar seleção");
                    if let Some(history) = &self.history {
                        ui.label(format!("{} · linhas {}–{}", history.path.display(), history.start, history.end));
                        ui.label(egui::RichText::new("Histórico na revisão consultada · não altera seu arquivo").color(color(tokens.muted)));
                        egui::ScrollArea::vertical().id_salt("git.insights.line.history.rows").max_height(450.).codisseu().show(ui, |ui| {
                            for entry in &history.commits {
                                ui.group(|ui| {
                                    if ui.selectable_label(false, metadata_label(&entry.subject, 400)).clicked() {
                                        inspect = Some((history.root.clone(), entry.oid.clone(), history.path.clone(), history.start));
                                    }
                                    ui.label(egui::RichText::new(format!("{} · {} · {}", metadata_label(&entry.author, 100), super::git_log::git_date(entry.timestamp), &entry.oid[..entry.oid.len().min(8)])).color(color(tokens.muted)));
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Detalhes").clicked() { inspect = Some((history.root.clone(), entry.oid.clone(), history.path.clone(), history.start)); }
                                        if ui.small_button("Diff").clicked() { actions.push(Action::OpenDiff { root: history.root.clone(), base: entry.parents.first().cloned(), head: entry.oid.clone(), path: Some(history.path.clone()) }); }
                                        if ui.small_button("Copiar SHA").clicked() { ctx.copy_text(entry.oid.clone()); }
                                    });
                                });
                            }
                            if history.commits.is_empty() { ui.label("Nenhum commit encontrado para essas linhas."); }
                            if history.has_more { ui.label("Exibindo até 50 commits; abra o histórico do arquivo para continuar."); }
                        });
                        if ui.button("Histórico completo do arquivo").clicked() { actions.push(Action::OpenFileHistory { root: history.root.clone(), path: history.path.clone() }); }
                    } else if let Some(error) = &self.history_error { crate::status_messages::status_message(ui, error); }
                    else {  ui.label("Consultando histórico…"); }
                });
        }
        if self.history_open && !open {
            if let Some(worker) = &self.history_worker {
                worker.cancel();
            }
            self.history_id = None;
        }
        self.history_open = open;
        if let Some((root, oid, path, line)) = inspect {
            self.open_commit(root, oid, path, line);
        }
        actions
    }
}
fn bounded(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.into();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… trecho limitado", &text[..end])
}
fn metadata_label(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut label: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        label.push('…');
    }
    label
}
fn new_range(header: &str) -> Option<(usize, usize)> {
    let range = header
        .strip_prefix("@@ ")?
        .split_whitespace()
        .find(|part| part.starts_with('+'))?
        .strip_prefix('+')?;
    let mut split = range.split(',');
    Some((
        split.next()?.parse().ok()?,
        split.next().map(str::parse).transpose().ok()?.unwrap_or(1),
    ))
}
fn hunk_for_line(patch: &str, line: usize) -> Option<String> {
    let lines: Vec<_> = patch.lines().collect();
    let mut closest = None;
    for (index, text) in lines.iter().enumerate() {
        let Some((start, count)) = new_range(text) else {
            continue;
        };
        if line >= start && line < start.saturating_add(count.max(1)) {
            closest = Some(index);
            break;
        }
    }
    let index = closest?;
    let end = lines[index + 1..]
        .iter()
        .position(|line| line.starts_with("@@ "))
        .map_or(lines.len(), |n| n + index + 1);
    let (mut next_line, _) = new_range(lines[index])?;
    let mut group = Vec::new();
    let mut group_start = next_line;
    let mut added = 0usize;
    for text in &lines[index + 1..end] {
        if text.starts_with('+') || text.starts_with('-') {
            if group.is_empty() {
                group_start = next_line;
            }
            group.push(*text);
            if text.starts_with('+') {
                next_line += 1;
                added += 1;
            }
        } else if text.starts_with('\\') {
            // A no-final-newline marker neither advances the line nor splits its change.
        } else {
            if line >= group_start && line < group_start.saturating_add(added) {
                return Some(bounded(&group.join("\n"), 4096));
            }
            group.clear();
            added = 0;
            if text.starts_with(' ') {
                next_line += 1;
            }
        }
    }
    (line >= group_start && line < group_start.saturating_add(added))
        .then(|| bounded(&group.join("\n"), 4096))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_cannot_replace_an_open_inspector_but_explicit_navigation_can() {
        let dir = tempfile::tempdir().unwrap();
        let first = Key {
            root: dir.path().into(),
            oid: "a".repeat(40),
            path: "first.php".into(),
        };
        let mut details = DetailsUi::default();
        details.desired = Some(first.clone());
        details.commit_open = true;
        details.line = 4;
        details.prefetch_commit(dir.path().into(), "b".repeat(40), "second.php".into(), 8);
        assert_eq!(details.desired.as_ref(), Some(&first));
        assert_eq!(details.line, 4);
        assert!(!details.is_busy());
        details.open_commit(dir.path().into(), "b".repeat(40), "second.php".into(), 8);
        assert_eq!(details.desired.as_ref().unwrap().oid, "b".repeat(40));
        assert_eq!(details.line, 8);
        assert!(details.commit_open);
        details.cancel();
    }
    #[test]
    fn cancelled_inspection_drains_without_restart_or_a_stale_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut details = DetailsUi::default();
        details.open_commit(dir.path().into(), "a".repeat(40), "file.php".into(), 1);
        assert!(details.is_busy());
        details.cancel();
        let started = std::time::Instant::now();
        while details.is_busy() {
            details.poll();
            assert!(started.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        details.poll();
        assert!(details.desired.is_none());
        assert!(details.loaded.is_none());
        assert!(details.error.is_none());
        assert!(!details.commit_open);
        assert!(!details.history_open);
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap().trim().into()
    }
    #[test]
    fn inspector_reads_real_author_message_and_file_patch_without_changing_disk_or_head() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-b", "main"]);
        git(&root, &["config", "user.name", "Autoria Fixture"]);
        git(&root, &["config", "user.email", "fixture@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(root.join("a.php"), "<?php\n// antes\n").unwrap();
        std::fs::write(root.join("b.php"), "outro arquivo\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "Primeiro"]);
        let parent = git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join("a.php"), "<?php\n// depois 🌧\n").unwrap();
        std::fs::write(root.join("b.php"), "outra alteração\n").unwrap();
        git(&root, &["commit", "-am", "Segundo\n\nMensagem completa"]);
        let oid = git(&root, &["rev-parse", "HEAD"]);
        let before = git(&root, &["status", "--porcelain"]);
        let key = Key {
            root: root.clone(),
            oid: oid.clone(),
            path: "a.php".into(),
        };
        let loaded = load(&Read {
            key: key.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        })
        .unwrap();
        assert_eq!(loaded.metadata.author, "Autoria Fixture");
        assert_eq!(loaded.details.parents, vec![parent.clone()]);
        assert!(loaded.details.message.contains("Mensagem completa"));
        assert!(loaded.file_patch.contains("+// depois 🌧"));
        assert!(!loaded.file_patch.contains("outra alteração"));
        let mut details = DetailsUi::default();
        details.desired = Some(key);
        details.loaded = Some(loaded);
        details.line = 2;
        assert!(
            details
                .hover_patch(&root, &oid, Path::new("a.php"), 2)
                .unwrap()
                .contains("+// depois 🌧")
        );
        assert!(
            matches!(details.previous_diff(&root, &oid, Path::new("a.php")), Some(Action::OpenDiff { base: Some(base), .. }) if base == parent)
        );
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), oid);
        assert_eq!(git(&root, &["status", "--porcelain"]), before);
        assert_eq!(
            std::fs::read_to_string(root.join("a.php")).unwrap(),
            "<?php\n// depois 🌧\n"
        );
        use egui_kittest::{Harness, kittest::Queryable};
        details.commit_open = true;
        let mut harness = Harness::builder().with_size([1050., 820.]).build_ui_state(
            |ui, state: &mut (DetailsUi, Tokens, Vec<Action>)| {
                let styled = egui::Id::unique("git.details.test.styled");
                if !ui
                    .ctx()
                    .data(|d| d.get_temp::<bool>(styled).unwrap_or(false))
                {
                    state.1.apply(ui.ctx());
                    ui.ctx().data_mut(|d| d.insert_temp(styled, true));
                }
                state.2.extend(state.0.show(ui.ctx(), &state.1));
            },
            (details, Tokens::load(), Vec::new()),
        );
        harness.run();
        assert!(harness.query_by_label("Detalhes do commit").is_some());
        assert!(harness.state().0.diff_view.visible_rows > 0);
        if let Some(out) = std::env::var_os("CODISSEU_GIT_DETAILS_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            // Creating the test renderer rebuilds its font atlas. Prime it before
            // retaining preview galleys, as the native host does on startup.
            let _ = harness.render().unwrap();
            harness.state_mut().0.diff_view = Default::default();
            harness.run();
            harness
                .render()
                .unwrap()
                .save(PathBuf::from(&out).join("details-normal.png"))
                .unwrap();
            harness.set_size(egui::vec2(760., 680.));
            harness.ctx.set_pixels_per_point(1.25);
            harness.run();
            harness
                .render()
                .unwrap()
                .save(PathBuf::from(&out).join("details-compact-125.png"))
                .unwrap();
            harness.set_size(egui::vec2(1050., 820.));
            harness.ctx.set_pixels_per_point(1.);
            harness.run();
        }
        harness.get_by_label("Comparar com anterior").click();
        harness.run();
        assert!(harness.state().2.iter().any(|action| matches!(action, Action::OpenDiff { root: action_root, base: Some(base), head, path: Some(path) } if *action_root == root && *base == parent && *head == oid && path == Path::new("a.php"))));
        harness.state_mut().0.commit_open = true;
        harness.run();
        harness.get_by_label("Arquivo nesta revisão").click();
        harness.run();
        assert!(harness.state().2.iter().any(|action| matches!(action, Action::OpenHistoricalFile { root: action_root, oid: action_oid, path, line: 2 } if *action_root == root && *action_oid == oid && path == Path::new("a.php"))));
    }
    #[test]
    fn hover_uses_the_matching_new_hunk_and_never_an_unrelated_commit_patch() {
        let patch =
            "diff --git a/a b/a\n@@ -1,2 +1,2 @@\n-a\n+b\n c\n@@ -20 +20,2 @@\n-z\n+🌧\n+q\n";
        assert!(hunk_for_line(patch, 1).unwrap().contains("+b"));
        assert!(!hunk_for_line(patch, 1).unwrap().contains("+q"));
        assert!(hunk_for_line(patch, 21).unwrap().contains("+🌧"));
        assert!(hunk_for_line(patch, 8).is_none());
        assert!(hunk_for_line("GIT binary patch\nliteral 0", 1).is_none());
    }
    #[test]
    fn hover_contains_only_the_actual_change_group_for_the_selected_new_line() {
        let patch = "@@ -1,7 +1,8 @@\n unchanged\n-old first\n+new first\n middle\n-old second\n+new second\n+new extra\n last\n";
        assert_eq!(
            hunk_for_line(patch, 2).as_deref(),
            Some("-old first\n+new first")
        );
        assert_eq!(
            hunk_for_line(patch, 4).as_deref(),
            Some("-old second\n+new second\n+new extra")
        );
        assert_eq!(
            hunk_for_line(patch, 5).as_deref(),
            Some("-old second\n+new second\n+new extra")
        );
        assert!(hunk_for_line(patch, 1).is_none());
        assert!(hunk_for_line(patch, 3).is_none());
        assert!(hunk_for_line(patch, 6).is_none());
        assert_eq!(
            hunk_for_line(
                "@@ -1 +1 @@\n-before\n+after\n\\ No newline at end of file",
                1
            )
            .as_deref(),
            Some("-before\n+after")
        );
    }
    #[test]
    fn bounding_keeps_valid_unicode_and_marks_truncation() {
        assert_eq!(bounded("🌧a", 3), "\n… trecho limitado");
        assert_eq!(bounded("abc", 3), "abc");
        assert_eq!(metadata_label("á🌧xyz", 2), "á🌧…");
        assert_eq!(metadata_label("á🌧", 2), "á🌧");
        assert_eq!(
            metadata_label(&"🌧".repeat(1_000_000), 512).chars().count(),
            513
        );
    }
}
