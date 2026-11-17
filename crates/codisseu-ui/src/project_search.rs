//! Search Everywhere combines real bounded providers without blocking the GUI.
use super::*;
use codisseu_core::search_everywhere::{
    self as engine, BufferSnapshot, Category, FileFilter, HitKind, Request, SearchEverywhereService,
};
use std::time::{Duration, Instant};

pub(super) struct ProjectSearch {
    pub open: bool,
    scope: Option<PathBuf>,
    query: String,
    focus: bool,
    category: Category,
    selected: usize,
    last_shift: Option<f64>,
    shift_down: bool,
    shift_used: bool,
    include_non_project: bool,
    preview: bool,
    disabled_actions: bool,
    owned_scope_text: Option<f64>,
    preview_pane: project_search_preview::PreviewPane,
    contributors: u8,
    result_height: f32,
    only_open: bool,
    filter: FileFilter,
    worker: SearchEverywhereService,
    key: Option<SearchKey>,
    refresh: u64,
    changed: Option<Instant>,
    labels: tab_label::TabLabels,
    rect: Option<egui::Rect>,
    recent: VecDeque<PathBuf>,
    history: VecDeque<String>,
    pending_split: Option<PathBuf>,
}
impl Default for ProjectSearch {
    fn default() -> Self {
        Self {
            open: false,
            scope: None,
            query: String::new(),
            focus: false,
            category: Category::All,
            selected: 0,
            last_shift: None,
            shift_down: false,
            shift_used: false,
            include_non_project: false,
            preview: false,
            disabled_actions: false,
            owned_scope_text: None,
            preview_pane: Default::default(),
            contributors: engine::ALL_CONTRIBUTORS,
            result_height: 0.,
            only_open: false,
            filter: FileFilter::All,
            worker: SearchEverywhereService::new(),
            key: None,
            refresh: 0,
            changed: None,
            labels: tab_label::TabLabels::default(),
            rect: None,
            recent: VecDeque::new(),
            history: VecDeque::new(),
            pending_split: None,
        }
    }
}
#[derive(Clone, PartialEq, Eq)]
struct SearchKey {
    root: Option<PathBuf>,
    versions: Vec<(PathBuf, u64)>,
    query: String,
    category: Category,
    include: bool,
    only_open: bool,
    filter: FileFilter,
    refresh: u64,
    contributors: u8,
}
#[derive(Clone)]
struct Item {
    title: String,
    detail: String,
    target: Target,
    score: usize,
    kind: Option<HitKind>,
    symbol: Option<codisseu_core::search_everywhere::Hit>,
    shortcut: &'static str,
    enabled: bool,
}
#[derive(Clone)]
enum Target {
    File(PathBuf, u32, u32, Option<u64>),
    Action(Action),
}
#[derive(Clone, Copy)]
enum Action {
    Debug,
    DebugConfigurations,
    OpenWorkspace,
    OpenFile,
    Settings,
    Terminal,
    Providers,
    Find,
    Replace,
    Save,
    SaveAll,
    Undo,
    Redo,
    Split,
    SourceControl,
    Commit,
    GitLog,
    CopilotAcceptNextLine,
}
fn location(query: &str) -> (&str, Option<(u32, u32)>) {
    let parts = query.rsplitn(3, ':').collect::<Vec<_>>();
    let number = |s: &str| s.parse::<u32>().ok().filter(|n| *n > 0);
    if parts.len() == 3
        && let (Some(column), Some(line)) = (number(parts[0]), number(parts[1]))
    {
        return (parts[2], Some((line - 1, column - 1)));
    }
    if parts.len() >= 2
        && let Some(line) = number(parts[0])
    {
        return (
            &query[..query.len() - parts[0].len() - 1],
            Some((line - 1, 0)),
        );
    }
    (query, None)
}
impl ProjectSearch {
    fn shift_trigger(&mut self, ctx: &egui::Context, enabled: bool) -> bool {
        let now = ctx.input(|i| i.time);
        let mut trigger = false;
        ctx.input(|i| {
            for event in &i.events {
                match event {
                    egui::Event::Key {
                        key: Key::ShiftLeft | Key::ShiftRight,
                        pressed,
                        repeat: false,
                        modifiers,
                        ..
                    } => {
                        if *pressed {
                            self.shift_down = true;
                            self.shift_used =
                                !enabled || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd;
                        } else if self.shift_down {
                            self.shift_down = false;
                            if !self.shift_used && enabled {
                                if self.last_shift.is_some_and(|last| now - last <= 0.5) {
                                    trigger = true;
                                    self.last_shift = None;
                                } else {
                                    self.last_shift = Some(now);
                                }
                            } else {
                                self.last_shift = None;
                            }
                        }
                    }
                    egui::Event::Key { pressed: true, .. }
                    | egui::Event::Text(_)
                    | egui::Event::PointerButton { pressed: true, .. }
                    | egui::Event::WindowFocused(false) => {
                        self.shift_used = true;
                        self.last_shift = None;
                    }
                    _ => {}
                }
            }
        });
        if !enabled {
            self.last_shift = None;
        }
        trigger
    }
}
impl CodisseuApp {
    pub fn project_search_result(&self) -> Option<&engine::Response> {
        self.project_search.worker.result.as_ref()
    }
    pub fn project_search_rect(&self) -> Option<egui::Rect> {
        self.project_search
            .open
            .then_some(self.project_search.rect)
            .flatten()
    }
    pub fn project_search_pending(&self) -> bool {
        self.project_search.open
            && (self.project_search.worker.pending()
                || self.project_search.changed.is_some()
                || (self.project_search.preview && self.project_search.preview_pane.pending()))
    }
    pub(super) fn open_project_search(&mut self) {
        self.project_search.scope = None;
        self.project_search.open = true;
        self.project_search.focus = true;
        self.project_search.selected = 0;
        self.project_search.last_shift = None;
        self.project_search.refresh = self.project_search.refresh.wrapping_add(1);
        self.project_search.result_height = 0.;
        self.project_search.key = None;
        self.project_search.changed = None;
        self.completion_popup = None;
    }
    pub(super) fn open_project_search_in(&mut self, root: PathBuf) {
        self.open_project_search();
        self.project_search.scope = Some(root);
        self.project_search.category = Category::Text;
        self.project_search.query.clear();
        self.project_search.only_open = false;
        self.project_search.include_non_project = false;
        self.project_search.contributors |= engine::TEXT;
    }
    pub(super) fn project_search_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if let Some(document) = self.service.active_document().filter(|d| !d.is_untitled())
            && self.project_search.recent.front() != Some(&document.path)
        {
            self.project_search
                .recent
                .retain(|path| path != &document.path);
            self.project_search.recent.push_front(document.path.clone());
            self.project_search.recent.truncate(32);
        }
        let blocked = self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.exit_when_saved
            || self.service.rename.is_open()
            || self.git_ui.dialog.is_some()
            || self.git_ui.diff_open
            || self.database_ui.dialog
            || self.database_ui.preview.is_some()
            || self.database_ui.edit.is_some();
        let text_field = ctx.egui_wants_keyboard_input() && !self.editor_focused(ctx);
        let double = self.project_search.shift_trigger(
            ctx,
            !blocked && (!text_field || self.project_search.open) && !egui::Popup::is_any_open(ctx),
        );
        if double {
            if self.project_search.open {
                self.project_search.include_non_project = !self.project_search.include_non_project;
            } else {
                self.open_project_search();
            }
        }
        if self.project_search.open {
            if consume_exact(ctx, Modifiers::COMMAND, Key::G) {
                self.project_search.focus = true;
            }
            let now = ctx.input(|i| i.time);
            if let Some(time) = self.project_search.owned_scope_text.take()
                && now - time < 0.25
            {
                let new_key = ctx.input(|i| {
                    i.events
                        .iter()
                        .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
                });
                if !new_key {
                    ctx.input_mut(|i| {
                        i.events
                            .retain(|e| !matches!(e,egui::Event::Text(t) if t=="n"||t=="N"))
                    });
                }
            }
            if !blocked
                && !egui::Popup::is_any_open(ctx)
                && consume_exact(ctx, Modifiers::ALT, Key::N)
            {
                if self.project_search.category == Category::Actions {
                    self.project_search.disabled_actions = !self.project_search.disabled_actions;
                } else {
                    self.project_search.include_non_project =
                        !self.project_search.include_non_project;
                }
                let mut removed = false;
                ctx.input_mut(|i| {
                    i.events.retain(|e| {
                        let hit = matches!(e,egui::Event::Text(t) if t=="n"||t=="N");
                        removed |= hit;
                        !hit
                    })
                });
                if !removed {
                    self.project_search.owned_scope_text = Some(now);
                }
                self.project_search.focus = true;
            }
        }
        self.project_search.open
    }
    fn sync_everywhere(&mut self, ctx: &egui::Context) {
        self.project_search.worker.poll();
        let key = SearchKey {
            root: self
                .project_search
                .scope
                .clone()
                .or_else(|| self.service.workspace_path().map(PathBuf::from)),
            versions: self
                .service
                .documents
                .iter()
                .map(|d| (d.path.clone(), d.version))
                .collect(),
            query: self.project_search.query.clone(),
            category: self.project_search.category,
            include: self.project_search.include_non_project,
            only_open: self.project_search.only_open,
            filter: self.project_search.filter,
            refresh: self.project_search.refresh,
            contributors: self.project_search.contributors,
        };
        if self.project_search.key.as_ref() != Some(&key) {
            self.project_search.worker.cancel();
            self.project_search.key = Some(key.clone());
            self.project_search.selected = 0;
            self.project_search.changed = Some(Instant::now());
        }
        // Small projects/opening start immediately; repeated edits coalesce before
        // starting filesystem work. Superseded workers are already cancelled.
        if self
            .project_search
            .changed
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(70) || key.query.is_empty())
        {
            let query = if matches!(key.category, Category::All | Category::Files) {
                location(key.query.trim()).0
            } else if key.category == Category::Text {
                key.query.as_str()
            } else {
                key.query.trim()
            };
            let request = Request {
                encoding_profile: Some(self.service.encoding_profile_path()),
                root: key.root,
                external_roots: vec![],
                buffers: self
                    .service
                    .documents
                    .iter()
                    .filter(|d| {
                        self.project_search
                            .scope
                            .as_ref()
                            .is_none_or(|scope| d.path.starts_with(scope))
                    })
                    .map(|d| BufferSnapshot {
                        path: d.path.clone(),
                        version: d.version,
                        text: d.snapshot(),
                    })
                    .collect(),
                query: query.into(),
                category: if key.category == Category::All
                    && query.is_empty()
                    && key.contributors & engine::FILES != 0
                {
                    Category::Files
                } else {
                    key.category
                },
                filter: if key.category == Category::Text {
                    key.filter
                } else {
                    FileFilter::All
                },
                include_non_project: key.include,
                only_open: (key.only_open && key.category != Category::Actions)
                    || (key.category == Category::All && query.is_empty()),
                refresh: key.refresh,
                preview: false,
                contributors: key.contributors,
            };
            if let Err(error) = self.project_search.worker.submit(request) {
                self.project_search.worker.error = Some(error);
            }
            self.project_search.changed = None;
        }
        if self.project_search_pending() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    fn everywhere_items(&self) -> Vec<Item> {
        let mut items = self
            .project_search
            .worker
            .result
            .as_ref()
            .map(|r| {
                r.hits
                    .iter()
                    .map(|h| Item {
                        title: h.title.clone(),
                        detail: h.detail.clone(),
                        target: Target::File(h.path.clone(), h.line, h.column, h.buffer_version),
                        score: h.score,
                        kind: Some(h.kind),
                        symbol: Some(h.clone()),
                        shortcut: "",
                        enabled: true,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if (self.project_search.category == Category::Actions
            || (self.project_search.category == Category::All
                && !self.project_search.query.trim().is_empty()
                && self.project_search.contributors & engine::ACTIONS != 0))
            && (self.project_search.category == Category::Actions || !self.project_search.only_open)
        {
            let document = self.service.active_document();
            for (action, title, shortcut, enabled) in [
                (Action::OpenWorkspace, "Open Folder…", "Ctrl+Shift+O", true),
                (Action::OpenFile, "Open File…", "Ctrl+O", true),
                (Action::Settings, "Settings…", "Ctrl+Alt+S", true),
                (Action::Terminal, "Terminal", "Alt+F12", true),
                (Action::Debug, "Debug", "Shift+F9", true),
                (
                    Action::DebugConfigurations,
                    "Run/Debug Configurations",
                    "",
                    true,
                ),
                (Action::Providers, "Configure Language Providers…", "", true),
                (Action::Find, "Find…", "Ctrl+F", document.is_some()),
                (Action::Replace, "Replace…", "Ctrl+H", document.is_some()),
                (Action::Save, "Save", "Ctrl+S", document.is_some()),
                (
                    Action::SaveAll,
                    "Save All",
                    "",
                    !self.service.documents.is_empty()
                        && self.service.documents.iter().all(|d| !d.is_untitled()),
                ),
                (
                    Action::Undo,
                    "Undo",
                    "Ctrl+Z",
                    document.is_some_and(|d| d.can_undo()),
                ),
                (
                    Action::Redo,
                    "Redo",
                    "Ctrl+Shift+Z",
                    document.is_some_and(|d| d.can_redo()),
                ),
                (Action::Split, "Split Right", "", document.is_some()),
                (
                    Action::CopilotAcceptNextLine,
                    "Copilot: aceitar próxima linha",
                    "",
                    self.copilot_can_accept_next_line(),
                ),
                (
                    Action::SourceControl,
                    "Source Control",
                    "",
                    self.service.workspace.is_some(),
                ),
                (
                    Action::Commit,
                    "Commit…",
                    "Ctrl+K",
                    self.service.git.repo.is_some(),
                ),
                (
                    Action::GitLog,
                    "Git Log",
                    "",
                    self.service.git.repo.is_some(),
                ),
            ] {
                let alias = match action {
                    Action::Settings => "Configurações preferências",
                    Action::OpenWorkspace => "Abrir pasta",
                    Action::OpenFile => "Abrir arquivo",
                    Action::Providers => "Configurar providers",
                    Action::CopilotAcceptNextLine => {
                        "GitHub Copilot Accept Next Line sugestão completar código linha"
                    }
                    _ => "",
                };
                if (enabled
                    || (self.project_search.category == Category::Actions
                        && self.project_search.disabled_actions))
                    && let Some(score) =
                        engine::fuzzy_score(title, self.project_search.query.trim()).or_else(|| {
                            engine::fuzzy_score(alias, self.project_search.query.trim())
                        })
                {
                    items.push(Item {
                        title: title.into(),
                        detail: "Actions".into(),
                        target: Target::Action(action),
                        score: score
                            + if self.project_search.category == Category::All {
                                40
                            } else {
                                0
                            },
                        kind: None,
                        symbol: None,
                        shortcut,
                        enabled,
                    });
                }
            }
        }
        let recent_only = self.project_search.category == Category::All
            && self.project_search.query.trim().is_empty();
        if recent_only {
            for item in &mut items {
                if let Target::File(path, ..) = &item.target {
                    item.score = self
                        .project_search
                        .recent
                        .iter()
                        .position(|p| p == path)
                        .unwrap_or(32);
                }
            }
        }
        if self.project_search.category == Category::All
            && self.project_search.query.trim().is_empty()
            && !self.project_search.only_open
            && self.project_search.contributors & engine::FILES != 0
        {
            for path in &self.project_search.recent {
                if !self.project_search.include_non_project
                    && self
                        .service
                        .workspace_path()
                        .is_some_and(|root| !path.starts_with(root))
                {
                    continue;
                }
                if items
                    .iter()
                    .any(|i| matches!(&i.target,Target::File(p,..) if p==path))
                {
                    continue;
                }
                let title = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let detail = path
                    .strip_prefix(self.service.workspace_path().unwrap_or(path))
                    .unwrap_or(path)
                    .display()
                    .to_string();
                let hit = engine::Hit {
                    kind: HitKind::File,
                    title: title.clone(),
                    detail: detail.clone(),
                    path: path.clone(),
                    line: 0,
                    column: 0,
                    buffer_version: None,
                    symbol_kind: None,
                    score: 0,
                    preview: None,
                };
                items.push(Item {
                    title,
                    detail,
                    target: Target::File(path.clone(), 0, 0, None),
                    score: self
                        .project_search
                        .recent
                        .iter()
                        .position(|p| p == path)
                        .unwrap_or(32),
                    kind: Some(HitKind::File),
                    symbol: Some(hit),
                    shortcut: "",
                    enabled: true,
                });
            }
        }
        items.sort_by(|a, b| {
            a.score
                .cmp(&b.score)
                .then_with(|| a.title.cmp(&b.title))
                .then_with(|| a.detail.cmp(&b.detail))
        });
        items.truncate(engine::MAX_RESULTS);
        items
    }
    pub(super) fn project_search_window(&mut self, ctx: &egui::Context) {
        if self.project_search.pending_split.is_some() && !self.service.is_busy() {
            let path = self.project_search.pending_split.take().unwrap();
            if self
                .service
                .active_document()
                .is_some_and(|d| d.path == path)
                && self.service.document_views().len() < 2
            {
                self.send(Command::CreateView);
                self.focus_editor = true;
                self.send(Command::SetEditorSplit(codisseu_core::EditorSplit {
                    horizontal: false,
                    ..self.service.editor_split()
                }));
                ctx.request_repaint();
            }
        }

        if !self.project_search.open {
            self.project_search.worker.poll();
            return;
        }
        self.sync_everywhere(ctx);
        let nested_popup_was_open = egui::Popup::is_any_open(ctx);
        let mut chosen = None;
        let mut cancel = false;
        let mut right_split = false;
        let size = ctx.content_rect().size();
        let preview_active =
            self.project_search.preview && self.project_search.category != Category::Actions;
        let width = 700f32.min((size.x - 34.).max(280.));
        // Observed SePopupHeaderPane/SeResultJBList background, PS262 Islands Dark.
        let background = Color32::from_rgb(38, 40, 44);
        let frame = egui::Frame::new()
            .fill(background)
            .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
            .corner_radius(8.)
            .inner_margin(0);
        let id = egui::Id::unique("codisseu.project.search");
        let modal = egui::Modal::new(id)
            .frame(frame)
            .backdrop_color(egui::Color32::TRANSPARENT)
            .area(egui::Modal::default_area(id).anchor(
                egui::Align2::CENTER_TOP,
                egui::vec2(0., (size.y * 0.12).min(120.)),
            ))
            .show(ctx, |ui| {
                ui.set_width(width);
                self.tokens.form_style(ui);
                ui.spacing_mut().item_spacing = egui::vec2(0., 0.);
                let (header_rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 43.), egui::Sense::hover());
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(header_rect.shrink2(egui::vec2(8., 0.)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        // Public component bounds in the recorded PS262 corpus:
                        // labels at x20/65/140/195/273/345 (popup-local).
                        // Advance by measured tab cells, independent of egui glyph widths.
                        ui.spacing_mut().item_spacing.x = 0.;
                        for (category, label) in categories() {
                            let selected = self.project_search.category == category;
                            let tab_width = match category {
                                Category::All => 45.,
                                Category::Classes => 75.,
                                Category::Files => 55.,
                                Category::Symbols => 78.,
                                Category::Actions => 72.,
                                Category::Text => 53.,
                            };
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(tab_width, 28.),
                                egui::Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    if category == Category::Actions {
                                        "Ações"
                                    } else {
                                        label
                                    },
                                )
                            });
                            if selected {
                                let rect = rect.with_max_x(rect.right() - 5.);
                                ui.painter().rect_filled(
                                    rect,
                                    4.,
                                    color(self.tokens.database_tab_selection),
                                );
                                ui.painter().rect_stroke(
                                    rect,
                                    4.,
                                    egui::Stroke::new(1., color(self.tokens.accent)),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            paint_search_label(
                                ui,
                                &mut self.project_search.labels,
                                rect.shrink2(egui::vec2(12., 0.)),
                                label,
                                color(self.tokens.text),
                                if selected {
                                    color(self.tokens.database_tab_selection)
                                } else {
                                    background
                                },
                            );
                            if response.clicked() {
                                self.project_search.category = category;
                                self.project_search.focus = true;
                            }
                        }
                        ui.spacing_mut().item_spacing.x = 8.;
                        let remaining = ui.available_width();
                        if remaining > 140. && self.project_search.category != Category::Actions {
                            let scoped = matches!(
                                self.project_search.category,
                                Category::Files | Category::Classes | Category::Symbols
                            );
                            ui.add_space((remaining - if scoped { 194. } else { 280. }).max(4.));
                            if matches!(
                                self.project_search.category,
                                Category::Files | Category::Classes | Category::Symbols
                            ) {
                                egui::ComboBox::from_id_salt("search.scope")
                                    .selected_text(if self.project_search.include_non_project {
                                        "All Places"
                                    } else {
                                        "Project Files"
                                    })
                                    .width(98.)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.project_search.include_non_project,
                                            false,
                                            "Project Files",
                                        );
                                        ui.selectable_value(
                                            &mut self.project_search.include_non_project,
                                            true,
                                            "All Places",
                                        );
                                    })
                                    .response
                                    .widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::accesskit::Role::ComboBox,
                                            true,
                                            "Search scope",
                                        )
                                    });
                            } else {
                                ui.checkbox(
                                    &mut self.project_search.include_non_project,
                                    "Include non-project items",
                                )
                                .on_hover_text(
                                    "Include dependency folders and open files outside the project",
                                );
                            }
                        }
                        if self.project_search.category != Category::Actions
                            && components::database_icon_button(
                                ui,
                                Icon::View,
                                "Show Preview",
                                true,
                            )
                            .clicked()
                        {
                            self.project_search.preview = !self.project_search.preview;
                            self.project_search.focus = true;
                            if !self.project_search.preview {
                                self.project_search.preview_pane.clear();
                            }
                        }
                        if self.project_search.category != Category::Actions {
                            let response = components::database_icon_button(
                                ui,
                                Icon::Filter,
                                "Search filters",
                                true,
                            );
                            egui::Popup::menu(&response).show(|ui| {
                                ui.set_min_width(220.);
                                if self.project_search.category == Category::All {
                                    ui.horizontal(|ui| {
                                        if ui.button("All").clicked() {
                                            self.project_search.contributors =
                                                engine::ALL_CONTRIBUTORS;
                                        }
                                        if ui.button("None").clicked() {
                                            self.project_search.contributors = 0;
                                        }
                                        if ui.button("Invert").clicked() {
                                            self.project_search.contributors ^=
                                                engine::ALL_CONTRIBUTORS;
                                        }
                                    });
                                    ui.separator();
                                    for (bit, label) in [
                                        (engine::ACTIONS, "Actions"),
                                        (engine::CLASSES, "Classes"),
                                        (engine::FILES, "Files"),
                                        (engine::SYMBOLS, "Symbols"),
                                        (engine::TEXT, "Text"),
                                    ] {
                                        let mut enabled =
                                            self.project_search.contributors & bit != 0;
                                        if ui.checkbox(&mut enabled, label).changed() {
                                            if enabled {
                                                self.project_search.contributors |= bit;
                                            } else {
                                                self.project_search.contributors &= !bit;
                                            }
                                        }
                                    }
                                    ui.separator();
                                }
                                if self.project_search.category != Category::Actions
                                    && ui
                                        .checkbox(
                                            &mut self.project_search.only_open,
                                            "Open files only",
                                        )
                                        .changed()
                                {
                                    ui.close();
                                }
                                if self.project_search.category == Category::Text {
                                    ui.separator();
                                    for (filter, label) in [
                                        (FileFilter::All, "All file types"),
                                        (FileFilter::Php, "*.php; *.phtml"),
                                        (FileFilter::JavaScript, "*.js; *.jsx"),
                                        (FileFilter::TypeScript, "*.ts; *.tsx"),
                                        (FileFilter::Sql, "*.sql"),
                                    ] {
                                        if ui
                                            .selectable_label(
                                                self.project_search.filter == filter,
                                                label,
                                            )
                                            .clicked()
                                        {
                                            self.project_search.filter = filter;
                                            ui.close();
                                        }
                                    }
                                }
                                if width < 650. {
                                    ui.separator();
                                    ui.checkbox(
                                        &mut self.project_search.include_non_project,
                                        "Include non-project items",
                                    );
                                }
                            });
                        } else {
                            ui.checkbox(
                                &mut self.project_search.disabled_actions,
                                "Include disabled actions",
                            );
                        }
                    },
                );
                ui.advance_cursor_after_rect(header_rect);
                if let Some(scope) = &self.project_search.scope {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("Find in Folder · {}", scope.display()))
                                .small()
                                .color(color(self.tokens.muted)),
                        )
                        .truncate(),
                    );
                }
                let (query_rect, _) =
                    ui.allocate_exact_size(egui::vec2(width, 43.), egui::Sense::hover());
                ui.painter().rect_stroke(
                    query_rect.shrink2(egui::vec2(8., 4.)),
                    4.,
                    egui::Stroke::new(1.5, color(self.tokens.accent)),
                    egui::StrokeKind::Inside,
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(query_rect.shrink2(egui::vec2(8., 4.)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        let (icon, history_button) =
                            ui.allocate_exact_size(egui::vec2(32., 24.), egui::Sense::click());
                        components::paint_icon(ui, icon, Icon::Search, color(self.tokens.muted));
                        history_button.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Button,
                                !self.project_search.history.is_empty(),
                                "Search history",
                            )
                        });
                        if !self.project_search.history.is_empty() {
                            components::paint_icon(
                                ui,
                                egui::Rect::from_center_size(
                                    icon.right_center() - egui::vec2(3., 0.),
                                    egui::vec2(8., 8.),
                                ),
                                Icon::Down,
                                color(self.tokens.muted),
                            );
                            egui::Popup::menu(&history_button).show(|ui| {
                                for query in self.project_search.history.clone() {
                                    if ui.button(&query).clicked() {
                                        self.project_search.query = query;
                                        self.project_search.focus = true;
                                        ui.close();
                                    }
                                }
                            });
                        }

                        let input = ui.add_sized(
                            [ui.available_width() - 34., 33.],
                            egui::TextEdit::singleline(&mut self.project_search.query)
                                .char_limit(512)
                                .frame(egui::Frame::NONE)
                                // Center layout itself so hint, caret, selection and
                                // pointer hit testing use the same text origin.
                                .align(egui::Align2::LEFT_CENTER)
                                .hint_text(if self.project_search.scope.is_some() {
                                    "Find in Folder"
                                } else {
                                    "Search Everywhere"
                                })
                                .id(egui::Id::unique("codisseu.project.search.query")),
                        );
                        input.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::TextInput,
                                true,
                                "Pesquisar no projeto",
                            )
                        });
                        if self.project_search.focus {
                            input.request_focus();
                            self.project_search.focus = false;
                        }
                        if components::database_icon_button(
                            ui,
                            Icon::Close,
                            "Fechar pesquisa",
                            true,
                        )
                        .clicked()
                        {
                            cancel = true;
                        }
                    },
                );
                ui.advance_cursor_after_rect(query_rect);
                if nested_popup_was_open && !egui::Popup::is_any_open(ctx) {
                    self.project_search.focus = true;
                }
                let mut reveal = false;
                let header_focus = ctx
                    .memory(|m| m.focused())
                    .is_some_and(|id| id != egui::Id::unique("codisseu.project.search.query"));
                let list_keys = !nested_popup_was_open && !egui::Popup::is_any_open(ctx);

                for (mods, key, category) in [
                    (
                        Modifiers::COMMAND.plus(Modifiers::SHIFT),
                        Key::A,
                        Category::Actions,
                    ),
                    (
                        Modifiers::COMMAND
                            .plus(Modifiers::ALT)
                            .plus(Modifiers::SHIFT),
                        Key::N,
                        Category::Symbols,
                    ),
                    (
                        Modifiers::COMMAND.plus(Modifiers::SHIFT),
                        Key::N,
                        Category::Files,
                    ),
                    (Modifiers::COMMAND, Key::N, Category::Classes),
                ] {
                    if list_keys && consume_exact(ctx, mods, key) {
                        self.project_search.category = category;
                        self.project_search.focus = true;
                    }
                }
                let cycle = if list_keys
                    && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab))
                {
                    -1
                } else if list_keys && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                    1
                } else {
                    0
                };
                if cycle != 0 {
                    let cats = categories();
                    let at = cats
                        .iter()
                        .position(|(c, _)| *c == self.project_search.category)
                        .unwrap_or(0);
                    self.project_search.category = cats[((at as i32 + cycle + 6) % 6) as usize].0;
                    self.project_search.focus = true;
                }
                self.sync_everywhere(ctx);
                let items = self.everywhere_items();
                let len = items.len();
                if len == 0 || !preview_active {
                    self.project_search.preview_pane.clear();
                }
                if list_keys
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowDown))
                    && len > 0
                {
                    self.project_search.selected = (self.project_search.selected + 1) % len;
                    reveal = true;
                }
                if list_keys
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowUp))
                    && len > 0
                {
                    self.project_search.selected = (self.project_search.selected + len - 1) % len;
                    reveal = true;
                }
                if list_keys
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageDown))
                    && len > 0
                {
                    self.project_search.selected = (self.project_search.selected + 10).min(len - 1);
                    reveal = true;
                }
                if list_keys
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::PageUp))
                    && len > 0
                {
                    self.project_search.selected = self.project_search.selected.saturating_sub(10);
                    reveal = true;
                }
                self.project_search.selected =
                    self.project_search.selected.min(len.saturating_sub(1));
                let split = list_keys
                    && !header_focus
                    && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Enter));
                let enter = list_keys
                    && !header_focus
                    && !split
                    && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                if (enter || split)
                    && let Some(item) = items.get(self.project_search.selected)
                    && item.enabled
                {
                    chosen = Some(item.target.clone());
                    right_split = split;
                }
                let wanted = (len.min(20) as f32 * 24. + 1.).max(if preview_active {
                    1.5 * (15. * self.tokens.editor_line_height + 8.)
                } else {
                    0.
                });
                self.project_search.result_height = self
                    .project_search
                    .result_height
                    .max(wanted)
                    .min((size.y - 260.).max(72.));
                let height = self.project_search.result_height;
                if len > 0 {
                    let (body, _) =
                        ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                    let list_width = width;
                    let list_height = if preview_active { height / 3. } else { height };
                    let mut results_ui = ui.new_child(egui::UiBuilder::new().max_rect(
                        egui::Rect::from_min_size(body.min, egui::vec2(list_width, list_height)),
                    ));
                    results_ui.set_clip_rect(ui.clip_rect().intersect(results_ui.max_rect()));
                    egui::ScrollArea::vertical()
                        .id_salt("search.everywhere.results")
                        .auto_shrink([false, false])
                        .max_height(list_height)
                        .min_scrolled_height(list_height)
                        .codisseu()
                        .show(&mut results_ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.;
                            for (index, item) in items.iter().enumerate() {
                                let selected = index == self.project_search.selected;
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 24.),
                                    egui::Sense::click(),
                                );
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::accesskit::Role::Button,
                                        item.enabled,
                                        match &item.target {
                                            Target::File(..) => match item.kind {
                                                Some(HitKind::Class) => format!(
                                                    "Abrir classe {} em {}",
                                                    item.title, item.detail
                                                ),
                                                Some(HitKind::Symbol) => format!(
                                                    "Abrir símbolo {} em {}",
                                                    item.title, item.detail
                                                ),
                                                Some(HitKind::Text) => {
                                                    format!("Abrir texto {}", item.detail)
                                                }
                                                _ => format!("Abrir {}", item.detail),
                                            },
                                            Target::Action(Action::Settings) => {
                                                "Abrir Configurações…".into()
                                            }
                                            Target::Action(Action::CopilotAcceptNextLine) => {
                                                "Copilot: aceitar próxima linha".into()
                                            }
                                            Target::Action(_) => format!("Abrir {}", item.title),
                                        },
                                    )
                                });
                                let bg = if selected {
                                    color(self.tokens.git_focus_selection)
                                } else {
                                    background
                                };
                                if selected {
                                    ui.painter().rect_filled(
                                        rect.shrink2(egui::vec2(8., 0.)),
                                        4.,
                                        bg,
                                    );
                                }
                                let icon = egui::Rect::from_center_size(
                                    egui::pos2(rect.left() + 23., rect.center().y),
                                    egui::vec2(16., 16.),
                                );
                                match &item.target {
                                    Target::File(path, ..) if item.kind == Some(HitKind::File) => {
                                        components::paint_document_icon(
                                            ui,
                                            egui::Rect::from_center_size(
                                                icon.center(),
                                                egui::vec2(16., 16.),
                                            ),
                                            path.to_str().unwrap_or_default(),
                                            color(self.tokens.accent),
                                        )
                                    }
                                    _ if item
                                        .symbol
                                        .as_ref()
                                        .is_some_and(|h| h.symbol_kind.is_some()) =>
                                    {
                                        paint_search_symbol(
                                            ui,
                                            icon,
                                            item.symbol
                                                .as_ref()
                                                .and_then(|h| h.symbol_kind)
                                                .unwrap(),
                                            &self.tokens,
                                        )
                                    }
                                    _ => components::paint_icon(
                                        ui,
                                        icon,
                                        if item.kind == Some(HitKind::Text) {
                                            Icon::File
                                        } else {
                                            Icon::Play
                                        },
                                        color(self.tokens.muted),
                                    ),
                                }
                                let title_width = ui
                                    .painter()
                                    .layout_no_wrap(
                                        item.title.clone(),
                                        FontId::proportional(13.),
                                        color(self.tokens.text),
                                    )
                                    .size()
                                    .x
                                    .min(rect.width() * 0.53);
                                let start = rect.left() + 37.;
                                paint_search_label(
                                    ui,
                                    &mut self.project_search.labels,
                                    egui::Rect::from_min_max(
                                        egui::pos2(start, rect.top()),
                                        egui::pos2(start + title_width + 2., rect.bottom()),
                                    ),
                                    &item.title,
                                    color(if item.enabled {
                                        self.tokens.text
                                    } else {
                                        self.tokens.muted
                                    }),
                                    bg,
                                );
                                let detail = if item.shortcut.is_empty() {
                                    &item.detail
                                } else {
                                    item.shortcut
                                };
                                paint_search_label(
                                    ui,
                                    &mut self.project_search.labels,
                                    egui::Rect::from_min_max(
                                        egui::pos2(start + title_width + 10., rect.top()),
                                        egui::pos2(rect.right() - 16., rect.bottom()),
                                    ),
                                    detail,
                                    color(self.tokens.muted),
                                    bg,
                                );
                                if reveal && selected {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                if item.enabled && response.clicked() {
                                    self.project_search.focus = true;
                                    self.project_search.selected = index;
                                    if !self.project_search.preview || response.double_clicked() {
                                        chosen = Some(item.target.clone());
                                    }
                                }
                            }
                        });
                    if preview_active {
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(body.left(), body.top() + list_height),
                            body.right_bottom(),
                        );
                        ui.painter().hline(
                            rect.x_range(),
                            rect.top(),
                            egui::Stroke::new(1., color(self.tokens.border)),
                        );
                        let mut preview_ui = ui.new_child(
                            egui::UiBuilder::new().max_rect(rect.shrink2(egui::vec2(8., 4.))),
                        );
                        preview_ui.set_clip_rect(ui.clip_rect().intersect(rect));
                        if let Some(hit) = items
                            .get(self.project_search.selected)
                            .and_then(|i| i.symbol.as_ref())
                        {
                            let root = self
                                .project_search
                                .scope
                                .clone()
                                .or_else(|| self.service.workspace_path().map(PathBuf::from));
                            self.project_search.preview_pane.show(
                                &mut preview_ui,
                                hit,
                                &self.service,
                                root,
                                self.project_search.refresh,
                                &self.tokens,
                            );
                        } else {
                            self.project_search.preview_pane.clear();
                            preview_ui.label("No file preview");
                        }
                    }
                } else if !self.project_search_pending() && !self.project_search.query.is_empty() {
                    ui.add_space(8.);
                    ui.horizontal(|ui| {
                        ui.add_space(12.);
                        ui.label("Nenhum resultado");
                    });
                    ui.add_space(8.);
                } else {
                    ui.add_space(4.);
                }
                let (footer, _) =
                    ui.allocate_exact_size(egui::vec2(width, 33.), egui::Sense::hover());
                ui.painter().hline(
                    footer.x_range(),
                    footer.top(),
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                let status = if self.project_search_pending() {
                    "Searching…".to_owned()
                } else if self
                    .project_search
                    .worker
                    .result
                    .as_ref()
                    .is_some_and(|r| r.truncated)
                {
                    "Partial results · search limit reached".to_owned()
                } else {
                    items
                        .get(self.project_search.selected)
                        .map(|i| match &i.target {
                            Target::File(path, ..) => path.display().to_string(),
                            Target::Action(_) => i.detail.clone(),
                        })
                        .unwrap_or_default()
                };
                paint_search_label(
                    ui,
                    &mut self.project_search.labels,
                    egui::Rect::from_min_max(
                        footer.left_top() + egui::vec2(12., 0.),
                        egui::pos2(footer.right() - 190., footer.bottom()),
                    ),
                    &status,
                    color(self.tokens.muted),
                    background,
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(egui::Rect::from_min_max(
                            egui::pos2(footer.right() - 190., footer.top()),
                            footer.right_bottom(),
                        ))
                        .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    |ui| {
                        ui.add_space(8.);
                        if ui
                            .add_enabled(
                                items
                                    .get(self.project_search.selected)
                                    .is_some_and(|i| matches!(i.target, Target::File(..))),
                                egui::Button::new("Open In Right Split").frame(false),
                            )
                            .on_hover_text("Shift+Enter")
                            .clicked()
                            && let Some(item) = items.get(self.project_search.selected)
                        {
                            chosen = Some(item.target.clone());
                            right_split = true;
                        }
                    },
                );
                ui.advance_cursor_after_rect(footer);
                if let Some(error) = &self.project_search.worker.error {
                    crate::status_messages::status_message(ui, error);
                }
            });
        self.project_search.rect = Some(modal.response.rect);
        self.sync_everywhere(ctx);
        let should_close = cancel || modal.should_close();
        if chosen.is_some() || should_close {
            let query = self.project_search.query.clone();
            if !query.trim().is_empty() {
                self.project_search.history.retain(|q| q != &query);
                self.project_search.history.push_front(query);
                self.project_search.history.truncate(32);
            }
        }
        if let Some(target) = chosen {
            self.project_search.open = false;
            self.project_search.worker.cancel();
            self.project_search.preview_pane.clear();
            self.activate_everywhere(target, right_split);
        } else if should_close {
            self.project_search.open = false;
            self.project_search.worker.cancel();
            self.project_search.preview_pane.clear();
            self.focus_editor = true;
        }
    }
    fn activate_everywhere(&mut self, target: Target, right_split: bool) {
        match target {
            Target::File(path, mut line, mut column, version) => {
                if version.is_some_and(|v| {
                    self.service
                        .documents
                        .iter()
                        .find(|d| d.path == path)
                        .is_none_or(|d| d.version != v)
                }) {
                    self.service.status = "Search result changed; search again".into();
                    return;
                }
                if matches!(
                    self.project_search.category,
                    Category::All | Category::Files
                ) && let Some((l, c)) = location(self.project_search.query.trim()).1
                {
                    line = l;
                    column = c;
                }
                if right_split && self.service.active_document().is_none() {
                    self.project_search.pending_split = Some(path.clone());
                } else if right_split {
                    if self.service.document_views().len() < 2 {
                        self.send(Command::CreateView);
                    } else if let Some(view) = self.service.document_views().last() {
                        let id = view.id;
                        self.send(Command::ActivateView { view: id });
                    }
                    self.send(Command::SetEditorSplit(codisseu_core::EditorSplit {
                        horizontal: false,
                        ..self.service.editor_split()
                    }));
                }
                self.jump_to = Some((path.clone(), line, column));
                let loaded = self
                    .service
                    .documents
                    .iter()
                    .find(|d| d.path == path)
                    .map(|d| d.id);
                if right_split && let (Some(id), Some(view)) = (loaded, self.service.active_view())
                {
                    self.send(Command::ShowDocumentInView { view, id });
                } else {
                    self.send(Command::OpenFile(path));
                }
                self.focus_editor = true;
            }
            Target::Action(action) => match action {
                Action::OpenWorkspace => self.open_path(PathAction::Workspace),
                Action::OpenFile => self.open_path(PathAction::File),
                Action::Settings => self.preferences_dialog = true,
                Action::Terminal => self.open_terminal(),
                Action::Debug => self.debugger_start_selected(),
                Action::DebugConfigurations => self.debugger_open_configurations(),
                Action::Providers => self.provider_dialog = true,
                Action::Find => self.open_search(false),
                Action::Replace => self.open_search(true),
                Action::Save => {
                    self.active_command("save");
                }
                Action::SaveAll => {
                    for id in self
                        .service
                        .documents
                        .iter()
                        .map(|d| d.id)
                        .collect::<Vec<_>>()
                    {
                        self.send(Command::Save { id });
                    }
                }
                Action::Undo => {
                    self.active_command("undo");
                }
                Action::Redo => {
                    self.active_command("redo");
                }
                Action::Split => self.split_editor(false),
                Action::SourceControl => self.git_open_source_control(),
                Action::Commit => self.git_open_commit(),
                Action::GitLog => self.git_open_log(),
                Action::CopilotAcceptNextLine => {
                    self.copilot_accept_next_line_command();
                }
            },
        }
    }
}
fn paint_search_symbol(ui: &egui::Ui, rect: egui::Rect, kind: engine::SymbolKind, tokens: &Tokens) {
    use engine::SymbolKind as K;
    let (letter, tint) = match kind {
        K::Class => ("C", color(tokens.git_modified)),
        K::Interface => ("I", color(tokens.git_added)),
        K::Trait => ("T", color(tokens.git_modified)),
        K::Enum => ("E", color(tokens.git_added)),
        K::Type => ("T", color(tokens.git_modified)),
        K::Function => ("f", color(tokens.git_conflict)),
        K::Method => ("m", color(tokens.git_conflict)),
        K::Property => ("p", color(tokens.git_modified)),
        K::Constant => ("c", color(tokens.git_added)),
        K::Variable => ("v", color(tokens.git_modified)),
    };
    ui.painter()
        .circle_stroke(rect.center(), 6., egui::Stroke::new(1., tint));
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        FontId::monospace(10.),
        tint,
    );
}

fn categories() -> [(Category, &'static str); 6] {
    [
        (Category::All, "All"),
        (Category::Classes, "Classes"),
        (Category::Files, "Files"),
        (Category::Symbols, "Symbols"),
        (Category::Actions, "Actions"),
        (Category::Text, "Text"),
    ]
}
fn paint_search_label(
    ui: &mut egui::Ui,
    labels: &mut tab_label::TabLabels,
    rect: egui::Rect,
    text: &str,
    foreground: Color32,
    background: Color32,
) {
    if rect.width() <= 0. {
        return;
    }
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.set_clip_rect(ui.clip_rect().intersect(rect));
    if !labels.paint(
        &child,
        text,
        egui::pos2(rect.left() + 1., rect.center().y),
        rect.width() - 1.,
        tab_label::LabelStyle {
            foreground,
            background,
            opacity: 1.,
            font_size: 13.,
        },
    ) {
        child.painter().text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.),
            foreground,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn location_unicode_and_invalid_suffix() {
        assert_eq!(location("ação.ts:5:3"), ("ação.ts", Some((4, 2))));
        assert_eq!(location("a.ts:5"), ("a.ts", Some((4, 0))));
        assert_eq!(location("a.ts:0"), ("a.ts:0", None));
    }
}
