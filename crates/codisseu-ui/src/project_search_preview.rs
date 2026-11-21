//! One bounded selected-source reader and virtual preview using the native editor lexer.
use super::*;
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
#[derive(Clone, PartialEq, Eq)]
struct Key {
    path: PathBuf,
    root: Option<PathBuf>,
    profile: PathBuf,
    version: Option<u64>,
    refresh: u64,
}
struct Job {
    generation: u64,
    key: Key,
    snapshot: Option<codisseu_core::TextSnapshot>,
}
struct Line {
    bytes: Range<usize>,
    syntax: editor::PreviewSyntax,
}
struct Source {
    text: String,
    extension: String,
    lines: Vec<Line>,
}
impl Source {
    fn new(
        text: String,
        path: &std::path::Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self, String> {
        if text.len() > 1024 * 1024 || text.contains('\0') {
            return Err("Prévia disponível para arquivos de texto até1MiB".into());
        }
        let extension = highlight::syntax_key(path);
        let mut syntax = editor::PreviewSyntax::default();
        let mut lines = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            if cancelled() {
                return Err("Prévia cancelada".into());
            }
            if lines.len() >= 100_000 {
                return Err("Prévia limitada a100000linhas".into());
            }
            let code = line.trim_end_matches(['\n', '\r']);
            if code.len() > 16 * 1024 {
                return Err("Prévia indisponível: uma linha excede16KiB".into());
            }
            lines.push(Line {
                bytes: start..start + code.len(),
                syntax,
            });
            syntax.advance(code, &extension);
            start += line.len();
        }
        if text.is_empty() || text.ends_with('\n') {
            lines.push(Line {
                bytes: text.len()..text.len(),
                syntax,
            });
        }
        Ok(Self {
            text,
            extension,
            lines,
        })
    }
}
struct Reply {
    generation: u64,
    result: Result<Arc<Source>, String>,
}
#[derive(Default)]
pub(super) struct PreviewPane {
    key: Option<Key>,
    generation: Arc<AtomicU64>,
    sender: Option<mpsc::SyncSender<Job>>,
    receiver: Option<mpsc::Receiver<Reply>>,
    waiting: Option<Job>,
    pending: bool,
    source: Option<Arc<Source>>,
    error: Option<String>,
    layouts: BTreeMap<usize, Arc<egui::Galley>>,
    shape: Option<(u32, u32, bool, codisseu_core::ThemePreference)>,
    widest: f32,
    target: Option<u32>,
    reveal: bool,
}
impl Drop for PreviewPane {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.sender.take();
    }
}
impl PreviewPane {
    pub(super) fn pending(&self) -> bool {
        self.pending
    }
    pub(super) fn clear(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.key = None;
        self.waiting = None;
        self.pending = false;
        self.source = None;
        self.error = None;
        self.layouts.clear();
        self.target = None;
    }
    fn start(&mut self) {
        if self.sender.is_some() {
            return;
        }
        let (sender, jobs) = mpsc::sync_channel::<Job>(1);
        let (replies, receiver) = mpsc::sync_channel(2);
        let generation = self.generation.clone();
        match std::thread::Builder::new()
            .name("search-selected-preview".into())
            .spawn(move || {
                while let Ok(job) = jobs.recv() {
                    let cancelled = || generation.load(Ordering::Acquire) != job.generation;
                    if cancelled() {
                        continue;
                    }
                    let result = (|| {
                        let text = if let Some(snapshot) = job.snapshot {
                            if snapshot.len_bytes() > 1024 * 1024 {
                                return Err("Prévia limitada a1MiB".into());
                            }
                            snapshot.to_string()
                        } else {
                            engine::load_preview(
                                &job.key.path,
                                job.key.root.as_deref(),
                                Some(&job.key.profile),
                                &cancelled,
                            )?
                            .text
                        };
                        Source::new(text, &job.key.path, &cancelled).map(Arc::new)
                    })();
                    if !cancelled()
                        && replies
                            .send(Reply {
                                generation: job.generation,
                                result,
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            }) {
            Ok(_) => {
                self.sender = Some(sender);
                self.receiver = Some(receiver);
            }
            Err(e) => {
                self.error = Some(e.to_string());
                self.pending = false;
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.receiver {
            while let Ok(reply) = receiver.try_recv() {
                if reply.generation == self.generation.load(Ordering::Acquire) {
                    self.pending = false;
                    match reply.result {
                        Ok(source) => {
                            self.source = Some(source);
                            self.reveal = true;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
        if let Some(job) = self.waiting.take()
            && let Some(sender) = &self.sender
        {
            match sender.try_send(job) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(job)) => self.waiting = Some(job),
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.pending = false;
                    self.error = Some("Prévia interrompida".into());
                }
            }
        }
        if self.pending {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    pub(super) fn show(
        &mut self,
        ui: &mut egui::Ui,
        hit: &engine::Hit,
        service: &AppService,
        root: Option<PathBuf>,
        refresh: u64,
        t: &Tokens,
    ) {
        let doc = service.documents.iter().find(|d| d.path == hit.path);
        let key = Key {
            path: hit.path.clone(),
            root,
            profile: service.encoding_profile_path(),
            version: doc.map(|d| d.version),
            refresh,
        };
        if self.key.as_ref() != Some(&key) {
            self.clear();
            self.key = Some(key.clone());
            self.pending = true;
            self.widest = 0.;
            self.shape = None;
            self.start();
            if self.sender.is_some() {
                self.waiting = Some(Job {
                    generation: self.generation.load(Ordering::Acquire),
                    key,
                    snapshot: doc.map(|d| d.snapshot()),
                });
            }
        }
        if self.target != Some(hit.line) {
            self.target = Some(hit.line);
            self.reveal = true;
        }
        self.poll(ui.ctx());
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0., color(t.editor));
        if let Some(error) = &self.error {
            ui.label(error);
            return;
        }
        let Some(source) = self.source.clone() else {
            ui.spinner();
            ui.label("Carregando prévia…");
            return;
        };
        let height = rect.height();
        let line_height = t.editor_line_height.max(t.editor_font_size + 2.);
        let gutter = t.editor_gutter_width.clamp(40., 52.);
        let inset = f32::from(t.navigation_popup_padding);
        let shape = (
            t.editor_font_size.to_bits(),
            ui.ctx().pixels_per_point().to_bits(),
            service.editor_preferences().ligatures,
            t.theme,
        );
        if self.shape != Some(shape) {
            self.layouts.clear();
            self.shape = Some(shape);
            self.widest = 0.;
        }
        if self.layouts.len() > 512 {
            self.layouts.clear();
        }
        let target = (hit.line as usize).min(source.lines.len().saturating_sub(1));
        let reveal = std::mem::take(&mut self.reveal);
        let id = egui::Id::unique(("search.preview.code", &hit.path));
        egui::ScrollArea::both()
            .id_salt(id)
            .auto_shrink([false, false])
            .max_height(height)
            .min_scrolled_height(height)
            .codisseu()
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.cursor().min;
                let total = source.lines.len() as f32 * line_height;
                // Pin the full content origin before allocating visible row widgets.
                // Otherwise the first scrolled row becomes min_rect.min, inflating
                // the virtual content height and leaving an empty tail.
                ui.allocate_space(egui::vec2(self.widest.max(viewport.width()), total));
                let first = (viewport.min.y / line_height).floor().max(0.) as usize;
                let end =
                    ((viewport.max.y / line_height).ceil() as usize + 1).min(source.lines.len());
                let left = origin.x + viewport.min.x;
                for index in first..end {
                    let line = &source.lines[index];
                    let text = &source.text[line.bytes.clone()];
                    let galley = self
                        .layouts
                        .entry(index)
                        .or_insert_with(|| {
                            editor::preview_layout_with_syntax(
                                ui,
                                text,
                                &source.extension,
                                t,
                                shape.2,
                                line.syntax,
                            )
                            .0
                        })
                        .clone();
                    self.widest = self.widest.max(galley.size().x + gutter + 2. * inset);
                    let top = origin.y + index as f32 * line_height;
                    let code_rect = egui::Rect::from_min_max(
                        egui::pos2(left + gutter, top),
                        egui::pos2(left + viewport.width(), top + line_height),
                    );
                    if index == target {
                        ui.painter().rect_filled(
                            code_rect.intersect(ui.clip_rect()),
                            0.,
                            color(t.shell_panel_background),
                        );
                    }
                    ui.painter()
                        .with_clip_rect(code_rect.intersect(ui.clip_rect()))
                        .galley(
                            egui::pos2(origin.x + gutter + inset, top),
                            galley,
                            color(t.text),
                        );
                    let number_rect = egui::Rect::from_min_size(
                        egui::pos2(left, top),
                        egui::vec2(gutter, line_height),
                    );
                    ui.painter().rect_filled(
                        number_rect.intersect(ui.clip_rect()),
                        0.,
                        color(t.editor),
                    );
                    ui.painter().text(
                        egui::pos2(number_rect.right() - inset, number_rect.center().y),
                        egui::Align2::RIGHT_CENTER,
                        (index + 1).to_string(),
                        FontId::monospace(t.editor_line_number_font_size),
                        color(t.muted),
                    );
                    ui.allocate_rect(code_rect, egui::Sense::hover())
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, text)
                        });
                }
                ui.set_min_width(self.widest.max(viewport.width()));
                ui.painter().vline(
                    left + gutter,
                    (origin.y + viewport.min.y)..=(origin.y + viewport.max.y),
                    egui::Stroke::new(1., color(t.editor_gutter_separator)),
                );
                if reveal {
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            origin + egui::vec2(0., target as f32 * line_height),
                            egui::vec2(1., line_height),
                        ),
                        Some(egui::Align::Center),
                    );
                }
            });
    }
}
use codisseu_core::search_everywhere as engine;
