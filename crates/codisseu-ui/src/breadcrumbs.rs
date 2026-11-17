//! Active-view structural trail. Parsing never runs on the UI thread.
use super::*;
use codisseu_core::{
    TextSnapshot,
    breadcrumbs::{self, Inspection, Kind},
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

struct Request {
    generation: u64,
    text: TextSnapshot,
    language: breadcrumbs::Language,
}
#[derive(Default)]
struct Mailbox {
    request: Mutex<Option<Request>>,
    ready: Condvar,
    stopped: AtomicBool,
}
type Reply = Option<(u64, Result<Inspection, String>)>;
#[derive(Default)]
pub(super) struct BreadcrumbUi {
    stamp: Option<(DocumentId, u64, PathBuf)>,
    mailbox: Arc<Mailbox>,
    epoch: Arc<AtomicU64>,
    reply: Arc<Mutex<Reply>>,
    started: bool,
    generation: u64,
    pending: bool,
    inspection: Inspection,
    error: Option<String>,
    focus_requested: bool,
    focused: Option<egui::Id>,
}
impl Drop for BreadcrumbUi {
    fn drop(&mut self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
        let _guard = self.mailbox.request.lock();
        self.mailbox.stopped.store(true, Ordering::Relaxed);
        self.mailbox.ready.notify_one();
    }
}
impl BreadcrumbUi {
    fn start(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let (mailbox, epoch, reply) =
            (self.mailbox.clone(), self.epoch.clone(), self.reply.clone());
        std::thread::Builder::new()
            .name("codisseu-breadcrumbs".into())
            .spawn(move || {
                loop {
                    let request = {
                        let Ok(mut slot) = mailbox.request.lock() else {
                            return;
                        };
                        while slot.is_none() && !mailbox.stopped.load(Ordering::Relaxed) {
                            slot = match mailbox.ready.wait(slot) {
                                Ok(s) => s,
                                Err(_) => return,
                            };
                        }
                        if mailbox.stopped.load(Ordering::Relaxed) {
                            return;
                        }
                        let Some(request) = slot.take() else {
                            continue;
                        };
                        request
                    };
                    let cancelled = || epoch.load(Ordering::Relaxed) != request.generation;
                    // Snapshot is cheap to clone; materialization is bounded and off-thread.
                    let source = request.text.to_string();
                    let result = breadcrumbs::inspect(&source, request.language, &cancelled);
                    if !cancelled()
                        && let Ok(mut slot) = reply.lock()
                    {
                        *slot = Some((request.generation, result));
                    }
                }
            })
            .map_err(|e| format!("Não foi possível analisar a estrutura: {e}"))?;
        self.started = true;
        Ok(())
    }
    fn refresh(&mut self, service: &AppService, ctx: &egui::Context) {
        let next = service
            .active_document()
            .map(|d| (d.id, d.version, d.path.clone()));
        if next != self.stamp {
            self.stamp = next;
            self.generation = self.epoch.fetch_add(1, Ordering::Relaxed) + 1;
            self.inspection = Inspection::default();
            self.pending = false;
            self.error = None;
            if let Ok(mut slot) = self.reply.lock() {
                *slot = None;
            }
            if let Ok(mut slot) = self.mailbox.request.lock() {
                *slot = None;
            }
            if let Some(doc) = service.active_document()
                && doc.len_bytes() <= breadcrumbs::MAX_BYTES
                && let Some(language) =
                    breadcrumbs::Language::for_extension(&highlight::syntax_key(&doc.path))
            {
                match self.start() {
                    Err(e) => self.error = Some(e),
                    Ok(()) => {
                        if let Ok(mut slot) = self.mailbox.request.lock() {
                            *slot = Some(Request {
                                generation: self.generation,
                                text: doc.snapshot(),
                                language,
                            });
                            self.pending = true;
                            self.mailbox.ready.notify_one();
                        }
                    }
                }
            }
        }
        if let Some((generation, result)) = self.reply.lock().ok().and_then(|mut s| s.take())
            && generation == self.generation
        {
            self.pending = false;
            match result {
                Ok(r) => self.inspection = r,
                Err(e) => self.error = Some(e),
            }
        }
        if self.pending {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
#[derive(Clone)]
enum Action {
    Reveal(PathBuf),
    Navigate(std::ops::Range<usize>),
}
struct Crumb {
    name: String,
    tooltip: String,
    kind: Option<Kind>,
    action: Action,
    file: bool,
}
impl Crumb {
    fn accessible(&self) -> String {
        format!(
            "Trilha: {} {}",
            self.kind
                .map_or(if self.file { "arquivo" } else { "pasta" }, Kind::label),
            self.name
        )
    }
}
fn crumb_button(
    ui: &mut egui::Ui,
    crumb: &Crumb,
    width: f32,
    tokens: &Tokens,
    id: egui::Id,
) -> egui::Response {
    let height = ui.available_height().max(1.);
    let response = ui
        .push_id(id, |ui| {
            ui.add_sized([width, height], egui::Button::new("").frame(false))
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            ui.is_enabled(),
            crumb.accessible(),
        )
    });
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(response.rect, 2., color(tokens.selection));
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(0.5),
            2.,
            egui::Stroke::new(1., color(tokens.accent)),
            egui::StrokeKind::Inside,
        );
    }
    let padding = tokens.breadcrumb_padding;
    let side = tokens.breadcrumb_icon_size.min(height - 2.);
    let icon = egui::Rect::from_center_size(
        egui::pos2(
            response.rect.left() + padding + side / 2.,
            response.rect.center().y,
        ),
        egui::vec2(side, side),
    );
    if let Some(kind) = crumb.kind {
        let letter = match kind {
            Kind::Class => "C",
            Kind::Interface => "I",
            Kind::Trait => "T",
            Kind::Enum => "E",
            Kind::Method => "m",
            Kind::Function | Kind::Closure => "f",
            Kind::Parameter => "p",
            Kind::Argument => "a",
            Kind::Call => "↪",
            Kind::Variable => "v",
            Kind::Namespace => "N",
        };
        ui.painter().circle_stroke(
            icon.center(),
            side * 0.43,
            egui::Stroke::new(1., color(tokens.accent)),
        );
        ui.painter().text(
            icon.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            FontId::proportional(side * 0.75),
            color(tokens.accent),
        );
    } else if crumb.file {
        components::paint_document_icon(ui, icon, &crumb.name, color(tokens.text));
    } else {
        components::paint_icon(ui, icon, Icon::Folder, color(tokens.muted));
    }
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(icon.right() + padding, response.rect.top()),
        egui::pos2(response.rect.right() - padding, response.rect.bottom()),
    );
    if text_rect.width() > 0. {
        let mut job = egui::text::LayoutJob::simple_singleline(
            crumb.name.clone(),
            FontId::proportional(tokens.ui_font_size),
            color(tokens.text),
        );
        job.wrap.max_width = text_rect.width();
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.fonts_mut(|f| f.layout_job(job));
        ui.painter().galley(
            egui::pos2(
                text_rect.left(),
                text_rect.center().y - galley.size().y / 2.,
            ),
            galley,
            color(tokens.text),
        );
    }
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&crumb.tooltip)
}
impl CodisseuApp {
    pub(super) fn breadcrumb_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.browser_active().is_some()
            || self.path_dialog.is_some()
            || self.preferences_dialog
            || self.confirm.is_some()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer().is_some())
        {
            return false;
        }
        // egui may surrender button focus on Escape before our shortcut pass.
        if self
            .breadcrumb_ui
            .focused
            .is_some_and(|id| ctx.memory(|m| m.has_focus(id) || m.focused().is_none()))
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.focus_editor = true;
            self.breadcrumb_ui.focused = None;
            return true;
        }
        if !self.trail().is_empty() && ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::Home)) {
            self.breadcrumb_ui.focus_requested = true;
            self.focus_editor = false;
            return true;
        }
        false
    }
    fn trail(&self) -> Vec<Crumb> {
        let Some(doc) = self.service.active_document() else {
            return vec![];
        };
        if self
            .breadcrumb_ui
            .stamp
            .as_ref()
            .is_none_or(|(id, version, path)| {
                *id != doc.id || *version != doc.version || *path != doc.path
            })
        {
            return vec![];
        }
        let caret = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
            .map_or(0, |v| v.selection().head);
        let scopes = self.breadcrumb_ui.inspection.at(caret);
        // User preference: operations occupy this space when no code context exists.
        if scopes.is_empty() || scopes.iter().all(|s| s.kind == Kind::Namespace) {
            return vec![];
        }
        let mut result = vec![];
        if let Some(root) = self.service.workspace_root_for(&doc.path) {
            let mut path = root.to_path_buf();
            result.push(Crumb {
                name: root
                    .file_name()
                    .unwrap_or(root.as_os_str())
                    .to_string_lossy()
                    .into(),
                tooltip: root.display().to_string(),
                kind: None,
                action: Action::Reveal(path.clone()),
                file: false,
            });
            if let Ok(relative) = doc.path.strip_prefix(root) {
                for component in relative.components() {
                    path.push(component);
                    result.push(Crumb {
                        name: component.as_os_str().to_string_lossy().into(),
                        tooltip: path.display().to_string(),
                        kind: None,
                        action: Action::Reveal(path.clone()),
                        file: path == doc.path,
                    });
                }
            }
        } else {
            result.push(Crumb {
                name: doc
                    .path
                    .file_name()
                    .unwrap_or(doc.path.as_os_str())
                    .to_string_lossy()
                    .into(),
                tooltip: doc.path.display().to_string(),
                kind: None,
                action: Action::Reveal(doc.path.clone()),
                file: true,
            });
        }
        result.extend(scopes.into_iter().map(|s| Crumb {
            name: s.name.clone(),
            tooltip: format!(
                "{} {} · linha {}\n{}",
                s.kind.label(),
                s.name,
                s.line + 1,
                doc.path.display()
            ),
            kind: Some(s.kind),
            action: Action::Navigate(s.target.clone()),
            file: false,
        }));
        result
    }
    fn breadcrumb_bar(&mut self, ui: &mut egui::Ui) {
        self.breadcrumb_ui.refresh(&self.service, ui.ctx());
        let crumbs = if self.browser_active().is_none() {
            self.trail()
        } else {
            vec![]
        };
        if crumbs.is_empty() {
            let raw = self.status_message_text(ui.ctx());
            let text = status_messages::friendly(&raw);
            ui.add(
                egui::Label::new(&text)
                    .truncate()
                    .show_tooltip_when_elided(false),
            )
            .on_hover_text(&raw);
            self.breadcrumb_ui.focus_requested = false;
            return;
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let (id, version, view) = (doc.id, doc.version, self.service.active_view());
        let mut action = None;
        ui.spacing_mut().item_spacing.x = self.tokens.breadcrumb_gap;
        ui.spacing_mut().button_padding = egui::vec2(self.tokens.breadcrumb_padding, 1.);
        ui.spacing_mut().interact_size.y = ui.available_height();
        if let Some(raw) = status_messages::current(ui.ctx()) {
            let r = icon_button(
                ui,
                Icon::Problems,
                "Mensagens da operação",
                false,
                ui.available_height(),
            );
            let r = r.on_hover_text(status_messages::friendly(&raw));
            egui::Popup::menu(&r).show(|ui| {
                ui.set_max_width(420.);
                ui.label(status_messages::friendly(&raw));
                if ui.button("Copiar detalhes técnicos").clicked() {
                    ui.ctx().copy_text(raw.clone());
                }
                ui.add(egui::Label::new(RichText::new(&raw).small()).wrap());
            });
        }
        let available = ui.available_width();
        let separator = 12.;
        let widths: Vec<f32> = crumbs
            .iter()
            .map(|c| {
                let text = ui
                    .fonts_mut(|f| {
                        f.layout_no_wrap(
                            c.name.clone(),
                            FontId::proportional(self.tokens.ui_font_size),
                            color(self.tokens.text),
                        )
                    })
                    .size()
                    .x;
                text.min(self.tokens.breadcrumb_max_label_width)
                    + self.tokens.breadcrumb_icon_size
                    + self.tokens.breadcrumb_padding * 3.
            })
            .collect();
        let mut first = 0;
        let overflow = self.tokens.breadcrumb_icon_size + self.tokens.breadcrumb_padding * 2.;
        while first + 1 < crumbs.len()
            && widths[first..].iter().sum::<f32>()
                + (crumbs.len() - first - 1) as f32 * separator
                + if first > 0 { overflow + separator } else { 0. }
                > available
        {
            first += 1;
        }
        if first > 0 {
            let r = ui
                .add_sized(
                    [overflow, ui.available_height()],
                    egui::Button::new("…").frame(false),
                )
                .on_hover_text("Caminho completo");
            r.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    "Trilha: caminho completo",
                )
            });
            egui::Popup::menu(&r).show(|ui| {
                for c in &crumbs[..first] {
                    if ui
                        .button(c.accessible())
                        .on_hover_text(&c.tooltip)
                        .clicked()
                    {
                        action = Some(c.action.clone());
                        ui.close();
                    }
                }
            });
            ui.add(egui::Label::new("›").selectable(false));
        }
        for (index, c) in crumbs.iter().enumerate().skip(first) {
            let width = widths[index].min(ui.available_width()).max(1.);
            let response = crumb_button(
                ui,
                c,
                width,
                &self.tokens,
                egui::Id::unique(("codisseu.breadcrumb", view, id, index)),
            );
            if self.breadcrumb_ui.focus_requested && index + 1 == crumbs.len() {
                response.request_focus();
                self.breadcrumb_ui.focus_requested = false;
            }
            if response.has_focus() {
                self.breadcrumb_ui.focused = Some(response.id);
            }
            if response.clicked() {
                action = Some(c.action.clone());
            }
            if index + 1 < crumbs.len() {
                ui.add(egui::Label::new("›").selectable(false));
            }
        }
        if let Some(action) = action {
            match action {
                Action::Reveal(path) => self.reveal_breadcrumb_path(path),
                Action::Navigate(range) => {
                    if self.service.active_view() == view {
                        self.send(Command::NavigateToRange {
                            id,
                            expected_version: version,
                            range,
                        });
                        self.focus_editor = true;
                    }
                }
            }
        }
    }
    pub(super) fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.browser_active().is_some() {
                ui.label("Browser");
            } else if let Some(doc) = self.service.active_document() {
                let read_only = doc.is_read_only();
                let snapshot = doc.snapshot();
                let line = snapshot.line(0);
                let n = line.len_chars();
                let newline = if n >= 2 && line.char(n - 2) == '\r' && line.char(n - 1) == '\n' {
                    "CRLF"
                } else {
                    "LF"
                };
                ui.label(format!(
                    "{newline}   Ln {}, Col {}",
                    self.cursor.0,
                    if self.cursor_column_pending {
                        "…".into()
                    } else {
                        self.cursor.1.to_string()
                    }
                ));
                self.encoding_status(ui);
                if read_only {
                    ui.label("Somente leitura").on_hover_text(
                        "Fonte externa da biblioteca Rust; edições e gravação protegidas",
                    );
                }
            } else {
                self.encoding_status(ui);
            }
            if self.browser_active().is_none() {
                self.language_status(ui);
            }
            self.copilot_status(ui);
            self.git_insights_status(ui);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width().max(0.), ui.available_height()),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| self.breadcrumb_bar(ui),
            );
        });
    }
}
