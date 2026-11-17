//! Native Markdown views over the editor's existing, versioned document.
use super::*;
use codisseu_core::{Document, ViewId};
use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Instant,
};
#[path = "markdown_parse.rs"]
mod parse;
#[path = "markdown_remote.rs"]
mod remote;
#[path = "markdown_render.rs"]
mod render;
pub(super) use remote::RemoteMarkdown;
#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(super) enum MarkdownMode {
    #[default]
    Source,
    Split,
    Preview,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum MarkdownAction {
    OpenLocal {
        path: PathBuf,
        anchor: Option<String>,
    },
    OpenExternal {
        url: String,
    },
    RevealSource {
        range: Range<usize>,
        line: usize,
    },
    ScrollSource {
        line: usize,
    },
    ToggleTask {
        id: DocumentId,
        version: u64,
        byte_offset: usize,
        checked: bool,
    },
}
pub(super) fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|ext| {
        matches!(
            ext.to_string_lossy().to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdown" | "mkd"
        )
    })
}
pub(super) fn validate_local_path(
    target: &Path,
    document: &Path,
    workspace: Option<&Path>,
) -> Result<PathBuf, String> {
    parse::validate_local_path(target, document, workspace)
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    id: DocumentId,
    version: u64,
    path: PathBuf,
    workspace: Option<PathBuf>,
}
struct Request {
    stamp: Stamp,
    snapshot: codisseu_core::TextSnapshot,
    cancel: Arc<AtomicBool>,
}
struct Reply {
    stamp: Stamp,
    result: Result<parse::Parsed, String>,
}
#[derive(Default)]
struct Mailbox {
    pending: VecDeque<Request>,
    completed: VecDeque<Reply>,
    expected: HashMap<DocumentId, Stamp>,
    active: Option<Arc<AtomicBool>>,
    stop: bool,
}
struct Worker {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    started: bool,
    start_error: Option<String>,
}
impl Default for Worker {
    fn default() -> Self {
        Self {
            mailbox: Arc::new((Mutex::new(Mailbox::default()), Condvar::new())),
            started: false,
            start_error: None,
        }
    }
}
impl Worker {
    fn start(&mut self) -> Result<(), String> {
        if let Some(error) = &self.start_error {
            return Err(error.clone());
        }
        if self.started {
            return Ok(());
        }
        let jobs = self.mailbox.clone();
        match thread::Builder::new().name("codisseu-markdown".into()).spawn(move || {
            loop {
                let request={
                    let (lock,wake)=&*jobs;
                    let Ok(mut state)=lock.lock() else {return;};
                    while state.pending.is_empty() && !state.stop {
                        let Ok(next)=wake.wait(state) else {return;}; state=next;
                    }
                    if state.stop {return;}
                    let request=state.pending.pop_front();
                    state.active=request.as_ref().map(|request|request.cancel.clone());
                    request
                };
                let Some(request)=request else {continue;};
                if request.cancel.load(Ordering::Relaxed) {continue;}
                // Full source materialization occurs only in this bounded worker.
                let result=if request.snapshot.len_bytes()>parse::MAX_SOURCE {
                    Err("Markdown preview is limited to 2 MiB; the source editor remains available".into())
                } else {
                    parse::parse(&request.snapshot.to_string(),&request.stamp.path,request.stamp.workspace.as_deref(),&request.cancel)
                };
                let (lock,_)=&*jobs;
                if let Ok(mut queue)=lock.lock() {
                    queue.active=None;
                    if queue.stop {return;}
                    if !request.cancel.load(Ordering::Relaxed) && queue.expected.get(&request.stamp.id)==Some(&request.stamp) {
                        queue.completed.retain(|reply|reply.stamp.id!=request.stamp.id);
                        queue.completed.push_back(Reply{stamp:request.stamp,result});
                    }
                }
            }
        }) {
            Ok(_) => {self.started=true;Ok(())},
            Err(error) => {let message=format!("Cannot start Markdown preview worker: {error}");self.start_error=Some(message.clone());Err(message)}
        }
    }
    fn expect(&self, documents: &HashMap<DocumentId, DocumentState>) {
        if let Ok(mut queue) = self.mailbox.0.lock() {
            queue.expected = documents
                .iter()
                .map(|(id, state)| (*id, state.stamp.clone()))
                .collect();
            let expected = queue.expected.clone();
            queue
                .completed
                .retain(|reply| expected.get(&reply.stamp.id) == Some(&reply.stamp));
            queue
                .pending
                .retain(|job| expected.get(&job.stamp.id) == Some(&job.stamp));
        }
    }
    fn submit(&mut self, request: Request) -> Result<(), String> {
        self.start()?;
        let (lock, wake) = &*self.mailbox;
        let mut queue = lock.lock().map_err(|_| "Markdown worker is unavailable")?;
        queue.pending.retain(|job| job.stamp.id != request.stamp.id);
        if queue.pending.len() >= 8 {
            return Err("Markdown preview queue is full".into());
        }
        queue.pending.push_back(request);
        wake.notify_one();
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.mailbox;
        if let Ok(mut queue) = lock.lock() {
            queue.stop = true;
            if let Some(active) = &queue.active {
                active.store(true, Ordering::Relaxed);
            }
            for job in queue.pending.drain(..) {
                job.cancel.store(true, Ordering::Relaxed);
            }
            wake.notify_one();
        }
    }
}

struct DocumentState {
    stamp: Stamp,
    snapshot: Option<codisseu_core::TextSnapshot>,
    due: Instant,
    cancel: Arc<AtomicBool>,
    waiting: bool,
    parsed: Option<Arc<parse::Parsed>>,
    rendered_version: u64,
    error: Option<String>,
    used: u64,
}
#[derive(Default)]
struct ViewState {
    used: u64,
    mode: MarkdownMode,
    sync: bool,
    source_line: Option<usize>,
    applied_source_line: Option<usize>,
    anchor: Option<String>,
    notice: Option<String>,
    render: render::RenderState,
    zoom: f32,
}
#[derive(Default)]
pub(super) struct MarkdownUi {
    worker: Worker,
    documents: HashMap<DocumentId, DocumentState>,
    views: BTreeMap<(ViewId, PathBuf), ViewState>,
    active_paths: BTreeMap<ViewId, PathBuf>,
    clock: u64,
}
impl Drop for MarkdownUi {
    fn drop(&mut self) {
        for state in self.documents.values() {
            state.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl MarkdownUi {
    pub(super) fn mode(&self, view: ViewId, path: &Path) -> MarkdownMode {
        self.views
            .get(&(view, path.to_path_buf()))
            .map_or(MarkdownMode::Source, |state| state.mode)
    }
    pub(super) fn set_mode(&mut self, view: ViewId, path: &Path, mode: MarkdownMode) {
        let key = (view, path.to_path_buf());
        if !self.views.contains_key(&key)
            && self.views.len() >= 32
            && let Some(old) = self
                .views
                .iter()
                .min_by_key(|(_, state)| state.used)
                .map(|(key, _)| key.clone())
        {
            self.views.remove(&old);
        }
        if let Some(previous) = self.active_paths.get(&view)
            && previous != path
            && let Some(state) = self.views.get_mut(&(view, previous.clone()))
        {
            state.render.release_resources();
        }
        if !self.active_paths.contains_key(&view)
            && self.active_paths.len() >= 32
            && let Some(old) = self.active_paths.keys().find(|old| **old != view).copied()
        {
            self.active_paths.remove(&old);
        }
        self.active_paths.insert(view, path.to_path_buf());
        let state = self.views.entry(key).or_insert_with(|| ViewState {
            sync: true,
            zoom: 1.,
            ..Default::default()
        });
        state.mode = mode;
        if mode == MarkdownMode::Source {
            state.render.release_resources();
        }
        state.used = self.clock;
    }
    pub(super) fn set_source_line(&mut self, view: ViewId, line: usize) {
        if let Some(path) = self.active_paths.get(&view)
            && let Some(state) = self.views.get_mut(&(view, path.clone()))
        {
            state.source_line = Some(line);
        }
    }
    pub(super) fn navigate_anchor(&mut self, view: ViewId, anchor: String) {
        if let Some(path) = self.active_paths.get(&view)
            && let Some(state) = self.views.get_mut(&(view, path.clone()))
        {
            state.anchor = Some(anchor);
        }
    }
    pub(super) fn toolbar(
        &mut self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        view: ViewId,
        path: &Path,
    ) -> MarkdownMode {
        let mode = self.mode(view, path);
        self.set_mode(view, path, mode);
        let state = self.views.get_mut(&(view, path.to_path_buf())).unwrap();
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0., color(tokens.editor));
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 0.5,
            egui::Stroke::new(1., color(tokens.border)),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("markdown.toolbar", view))
                .max_rect(rect.shrink2(egui::vec2(8., 2.)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 2.;
        for (choice, label, icon) in [
            (MarkdownMode::Source, "Markdown Source", 0),
            (MarkdownMode::Split, "Markdown Split", 1),
            (MarkdownMode::Preview, "Markdown Preview", 2),
        ] {
            let response = tool(&mut child, icon, label, state.mode == choice, tokens);
            if response.clicked() {
                state.mode = choice;
            }
        }
        child.separator();
        let sync = tool(
            &mut child,
            3,
            "Synchronize Markdown scrolling",
            state.sync,
            tokens,
        );
        if sync.clicked() {
            state.sync = !state.sync;
            state.applied_source_line = None;
        }
        if tool(&mut child, 4, "Zoom out Markdown preview", false, tokens).clicked() {
            state.zoom = (state.zoom - 0.1).max(0.7);
            state.render.clear_layout();
        }
        if child
            .add(
                egui::Button::new(format!("{}%", (state.zoom * 100.).round() as u32))
                    .frame(false)
                    .min_size(egui::vec2(42., 24.)),
            )
            .on_hover_text("Reset Markdown preview zoom")
            .clicked()
        {
            state.zoom = 1.;
            state.render.clear_layout();
        }
        if tool(&mut child, 5, "Zoom in Markdown preview", false, tokens).clicked() {
            state.zoom = (state.zoom + 0.1).min(2.);
            state.render.clear_layout();
        }
        state.mode
    }
    pub(super) fn preview(
        &mut self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        view: ViewId,
        doc: &Document,
        workspace: Option<&Path>,
    ) -> Vec<MarkdownAction> {
        self.clock = self.clock.wrapping_add(1);
        let stamp = Stamp {
            id: doc.id,
            version: doc.version,
            path: doc.path.clone(),
            workspace: workspace.map(Path::to_path_buf),
        };
        if !self
            .documents
            .get(&doc.id)
            .is_some_and(|state| state.stamp == stamp)
        {
            let previous = self.documents.remove(&doc.id);
            let first = previous.is_none();
            if let Some(previous) = &previous {
                previous.cancel.store(true, Ordering::Relaxed);
            }
            if self.documents.len() >= 8
                && let Some(old) = self
                    .documents
                    .iter()
                    .min_by_key(|(_, s)| s.used)
                    .map(|(id, _)| *id)
                && let Some(old) = self.documents.remove(&old)
            {
                old.cancel.store(true, Ordering::Relaxed);
            }
            self.documents.insert(
                doc.id,
                DocumentState {
                    stamp: stamp.clone(),
                    snapshot: Some(doc.snapshot()),
                    due: Instant::now()
                        + if first {
                            Duration::ZERO
                        } else {
                            Duration::from_millis(120)
                        },
                    cancel: Arc::new(AtomicBool::new(false)),
                    waiting: true,
                    parsed: previous.as_ref().and_then(|old| old.parsed.clone()),
                    rendered_version: previous
                        .as_ref()
                        .map_or(doc.version, |old| old.rendered_version),
                    error: None,
                    used: self.clock,
                },
            );
        }
        if let Some(state) = self.documents.get_mut(&doc.id) {
            state.used = self.clock;
        }
        self.worker.expect(&self.documents);
        for state in self.documents.values_mut() {
            if state.due <= Instant::now()
                && let Some(snapshot) = state.snapshot.take()
                && let Err(error) = self.worker.submit(Request {
                    stamp: state.stamp.clone(),
                    snapshot,
                    cancel: state.cancel.clone(),
                })
            {
                state.waiting = false;
                state.error = Some(error);
            }
        }
        let replies = if let Ok(mut queue) = self.worker.mailbox.0.lock() {
            queue.completed.drain(..).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for reply in replies {
            if let Some(state) = self.documents.get_mut(&reply.stamp.id)
                && state.stamp == reply.stamp
            {
                state.waiting = false;
                match reply.result {
                    Ok(parsed) => {
                        state.parsed = Some(Arc::new(parsed));
                        state.rendered_version = reply.stamp.version;
                        state.error = None;
                        for ((_, path), view) in &mut self.views {
                            if path == &reply.stamp.path {
                                view.applied_source_line = None;
                            }
                        }
                    }
                    Err(error) => {
                        state.parsed = None;
                        state.error = Some(error);
                    }
                }
            }
        }
        while self
            .documents
            .values()
            .filter_map(|state| state.parsed.as_ref())
            .map(|parsed| parsed.cost_bytes)
            .sum::<usize>()
            > 128 * 1024 * 1024
        {
            let oldest = self
                .documents
                .iter()
                .filter(|(id, _)| **id != doc.id)
                .min_by_key(|(_, state)| state.used)
                .map(|(id, _)| *id);
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(old) = self.documents.remove(&oldest) {
                old.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.worker.expect(&self.documents);
        let state = self.documents.get(&doc.id).unwrap();
        self.active_paths.insert(view, doc.path.clone());
        let view_state = self
            .views
            .entry((view, doc.path.clone()))
            .or_insert_with(|| ViewState {
                mode: MarkdownMode::Preview,
                sync: true,
                zoom: 1.,
                ..Default::default()
            });
        view_state.used = self.clock;
        if state.waiting {
            ui.ctx().request_repaint_after(Duration::from_millis(25));
            if state.parsed.is_none() {
                ui.label(
                    RichText::new("Updating Markdown preview…")
                        .small()
                        .color(color(tokens.muted)),
                );
            } else {
                ui.painter().text(
                    ui.available_rect_before_wrap().right_top() - egui::vec2(12., -4.),
                    egui::Align2::RIGHT_TOP,
                    "Updating…",
                    FontId::proportional(11.),
                    color(tokens.muted),
                );
            }
        }
        if let Some(error) = &state.error {
            crate::status_messages::status_message(ui, error);
            return Vec::new();
        }
        let Some(parsed) = &state.parsed else {
            return Vec::new();
        };
        let mut target = None;
        let mut anchor_line = None;
        if !state.waiting
            && let Some(anchor) = view_state.anchor.take()
        {
            target=parsed.blocks.iter().position(|block|matches!(&block.kind,parse::BlockKind::Rich{anchor:Some(id),..}if id==&anchor));
            if let Some(index) = target {
                view_state.notice = None;
                view_state.applied_source_line = view_state.source_line;
                anchor_line = Some(parsed.blocks[index].line);
            } else {
                view_state.notice = Some(format!("Heading #{anchor} was not found"));
            }
        }
        if target.is_none()
            && view_state.sync
            && view_state.source_line != view_state.applied_source_line
        {
            target = view_state.source_line.map(|line| {
                parsed
                    .blocks
                    .partition_point(|b| b.line <= line)
                    .saturating_sub(1)
            });
            view_state.applied_source_line = view_state.source_line;
        }
        if let Some(notice) = &view_state.notice {
            crate::status_messages::status_message(ui, notice);
        }
        for warning in &parsed.warnings {
            ui.label(RichText::new(warning).small().color(color(tokens.muted)));
        }
        let mut actions = ui
            .add_enabled_ui(!state.waiting, |ui| {
                render::show(
                    ui,
                    tokens,
                    view,
                    doc.id,
                    state.rendered_version,
                    parsed,
                    &mut view_state.render,
                    view_state.zoom.max(0.7),
                    view_state.sync,
                    target,
                )
            })
            .inner;
        if view_state.sync
            && let Some(line) = anchor_line
        {
            actions.push(MarkdownAction::ScrollSource { line });
        }
        actions.retain(|action| {
            if let MarkdownAction::OpenLocal {
                path,
                anchor: Some(anchor),
            } = action
                && path.as_os_str().is_empty()
            {
                view_state.anchor = Some(anchor.clone());
                ui.ctx().request_repaint();
                false
            } else {
                true
            }
        });
        actions
    }
}
fn tool(
    ui: &mut egui::Ui,
    icon: u8,
    label: &str,
    selected: bool,
    _tokens: &Tokens,
) -> egui::Response {
    let response = ui.add(
        egui::Button::new("")
            .selected(selected)
            .frame(selected)
            .min_size(egui::vec2(28., 24.)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::accesskit::Role::Button, true, selected, label)
    });
    let rect = egui::Rect::from_center_size(response.rect.center(), egui::vec2(16., 16.));
    crate::components::paint_markdown_icon(ui, rect, icon);
    response.on_hover_text(label)
}
