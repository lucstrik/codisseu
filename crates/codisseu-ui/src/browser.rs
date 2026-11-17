//! Native browser tabs. Chromium supplies web pixels; egui owns all IDE chrome.
#[path = "browser_input.rs"]
mod input;
use super::*;
use codisseu_core::{
    ViewId,
    browser::{BrowserId, Command as BrowserCommand, EngineState, Surface},
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct BrowserUi {
    pub(super) library: super::browser_library_ui::LibraryUi,
    tabs: BTreeMap<BrowserId, Tab>,
    active: BTreeMap<ViewId, BrowserId>,
    revealed: BTreeMap<ViewId, (BrowserId, String)>,
    frontend_epochs: BTreeMap<BrowserId, u64>,
    textures: BTreeMap<(BrowserId, Surface), Texture>,
    sizes: BTreeMap<(BrowserId, Surface), (u32, u32, u32)>,
    rects: BTreeMap<(BrowserId, Surface), egui::Rect>,
    chrome: BTreeMap<BrowserId, egui::Rect>,
    next_id: BrowserId,
    waker_set: bool,
    hovered: std::collections::BTreeSet<(BrowserId, Surface)>,
    pointer_capture: Option<(BrowserId, Surface)>,
    pending_paste: Option<(BrowserId, Surface)>,
    paste_requested: Option<std::time::Instant>,
    paste_canceled: bool,
    focused: Option<(BrowserId, Surface)>,
    focus_page: Option<BrowserId>,
    shortcut_releases: std::collections::BTreeSet<Key>,
}
struct Tab {
    view: ViewId,
    address: String,
    focus_address: bool,
    select_address: bool,
    notice: Option<String>,
    split: f32,
}
struct Texture {
    handle: egui::TextureHandle,
    seq: u64,
}
impl BrowserUi {
    pub(super) fn active_in_view(&self, view: ViewId) -> bool {
        self.active
            .get(&view)
            .is_some_and(|id| self.tabs.contains_key(id))
    }
    pub(super) fn deactivate(&mut self, view: ViewId) {
        self.active.remove(&view);
        self.revealed.remove(&view);
    }
    pub(super) fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}
impl CodisseuApp {
    /// CLI opening waits for document/session I/O so a late OpenFile result
    /// cannot steal the explicitly requested browser tab on the first frame.
    pub fn queue_browser_url(&mut self, url: &str) {
        self.startup.push_back(StartupAction::Browser(url.into()));
    }
    pub fn open_browser_url(&mut self, url: &str) {
        self.browser_ui.next_id = self.browser_ui.next_id.saturating_add(1);
        let id = self.browser_ui.next_id;
        if let Err(error) = self
            .service
            .dispatch(Command::Browser(BrowserCommand::Open {
                id,
                url: url.into(),
            }))
        {
            self.service.last_error = Some(error);
            return;
        }
        let view = self.service.active_view().unwrap_or(0);
        self.jira_deactivate_view(view);
        self.browser_ui.tabs.insert(
            id,
            Tab {
                view,
                address: url.into(),
                focus_address: true,
                select_address: true,
                notice: None,
                split: 0.56,
            },
        );
        self.browser_ui.active.insert(view, id);
        if self.api_ui.active_in_view(view) {
            self.api_ui.active = None;
        }
        self.focus_editor = false;
    }
    pub(super) fn browser_debug_configuration(&mut self, id: BrowserId) {
        use codisseu_core::debugger::{DebugConfiguration, DebugRequest, DebugRuntime};
        let Some(root) = self
            .service
            .active_document()
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| self.service.workspace_path().map(PathBuf::from))
        else {
            if let Some(tab) = self.browser_ui.tabs.get_mut(&id) {
                tab.notice=Some("Abra um projeto para mapear as fontes no depurador da IDE; DevTools funciona sem projeto".into());
            }
            return;
        };
        let Some(tab) = self.service.browser.tabs.get(&id) else {
            return;
        };
        let config = DebugConfiguration {
            id: format!("browser-{id}"),
            name: format!("Navegador integrado · {}", tab.title),
            runtime: DebugRuntime::Chrome,
            request: DebugRequest::Attach,
            cwd: root,
            url: tab.url.clone(),
            integrated_browser: true,
            browser_tab: Some(id),
            trusted: false,
            ..DebugConfiguration::default()
        };
        self.debugger_edit_configuration(config);
        self.debugger_ui.dialog = true;
    }
    pub(super) fn browser_active(&self) -> Option<BrowserId> {
        self.browser_ui
            .active
            .get(&self.service.active_view().unwrap_or(0))
            .copied()
    }
    pub(super) fn browser_inspector_open(&mut self, id: BrowserId) {
        let error = self.service.start_browser_inspector(id).err();
        if error.is_none() {
            self.browser_inspector_ui.started(id);
        }
        if let Some(tab) = self.browser_ui.tabs.get_mut(&id) {
            tab.notice = error;
        }
    }
    pub(super) fn browser_send(&mut self, command: BrowserCommand) {
        let id = command.id();
        let clears_notice = matches!(
            &command,
            BrowserCommand::Navigate { .. }
                | BrowserCommand::Reload { .. }
                | BrowserCommand::Back { .. }
                | BrowserCommand::Forward { .. }
        );
        let result = self.service.dispatch(Command::Browser(command));
        if let Some(tab) = id.and_then(|id| self.browser_ui.tabs.get_mut(&id)) {
            match result {
                Err(error) => tab.notice = Some(error),
                Ok(()) if clears_notice => tab.notice = None,
                Ok(()) => {}
            }
        }
    }
    pub(super) fn browser_close(&mut self, id: BrowserId) {
        self.browser_send(BrowserCommand::Close { id });
        if let Some(tab) = self.browser_ui.tabs.remove(&id) {
            let was_active = self.browser_ui.active.get(&tab.view) == Some(&id);
            self.browser_ui.active.retain(|_, active| *active != id);
            if was_active
                && let Some((&next, _)) = self
                    .browser_ui
                    .tabs
                    .iter()
                    .rev()
                    .find(|(_, t)| t.view == tab.view)
            {
                self.browser_ui.active.insert(tab.view, next);
                self.browser_ui.focus_page = Some(next);
            } else if was_active {
                self.focus_editor = true;
            }
        }
        self.browser_ui.textures.retain(|(tab, _), _| *tab != id);
        self.browser_ui.sizes.retain(|(tab, _), _| *tab != id);
        self.browser_ui.rects.retain(|(tab, _), _| *tab != id);
        self.browser_ui.chrome.remove(&id);
    }
    pub(super) fn browser_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            return;
        }
        self.browser_filter_shortcut_releases(ctx);
        self.browser_saved_tick(ctx);
        if self.service.browser_inspector.state.lifecycle
            == codisseu_core::browser::inspector::Lifecycle::Stopped
        {
            self.browser_inspector_ui.clear_stopped();
        }
        if !self.browser_ui.waker_set {
            let inspector_ctx = ctx.clone();
            self.service
                .browser_inspector
                .set_waker(std::sync::Arc::new(move || inspector_ctx.request_repaint()));
            let saved_ctx = ctx.clone();
            let ctx = ctx.clone();
            self.service
                .browser
                .set_waker(std::sync::Arc::new(move || ctx.request_repaint()));
            self.service
                .browser_library
                .set_waker(std::sync::Arc::new(move || saved_ctx.request_repaint()));
            self.browser_ui.waker_set = true;
        }
        if let Some(id) = self.browser_ui.focus_page {
            if self.browser_active() != Some(id) || !self.browser_ui.tabs.contains_key(&id) {
                self.browser_ui.focus_page = None;
            } else if !self.browser_input_blocked(ctx)
                && self.service.browser.state == EngineState::Ready
            {
                self.browser_ui.focus_page = None;
                ctx.memory_mut(|m| {
                    m.request_focus(egui::Id::unique(("browser.surface", id, Surface::Page)))
                });
                self.browser_send(BrowserCommand::Focus {
                    id,
                    surface: Surface::Page,
                    focused: true,
                });
                self.browser_ui.focused = Some((id, Surface::Page));
            }
        }
        if let Some((id, surface)) = self.browser_ui.pending_paste {
            let in_time = self
                .browser_ui
                .paste_requested
                .is_some_and(|when| when.elapsed() < Duration::from_secs(5));
            let valid = in_time
                && self.service.browser.state == EngineState::Ready
                && self
                    .browser_ui
                    .tabs
                    .get(&id)
                    .is_some_and(|t| t.view == self.service.active_view().unwrap_or(0))
                && self
                    .browser_ui
                    .active
                    .get(&self.service.active_view().unwrap_or(0))
                    == Some(&id)
                && (surface == Surface::Page
                    || self
                        .service
                        .browser
                        .tabs
                        .get(&id)
                        .is_some_and(|t| t.devtools_open))
                && !self.browser_modal_input_blocked(ctx);
            self.browser_ui.paste_canceled |= !valid;
            // A paste requested by our menu owns its reply even if a tab closes
            // or focus changes. Consume it before any document TextEdit sees it.
            let reply = ctx.input_mut(|i| {
                i.events
                    .iter()
                    .position(|e| matches!(e, egui::Event::Paste(_)))
                    .and_then(|at| match i.events.remove(at) {
                        egui::Event::Paste(text) => Some(text),
                        _ => None,
                    })
            });
            if let Some(text) = reply {
                self.browser_ui.pending_paste = None;
                self.browser_ui.paste_requested = None;
                if !self.browser_ui.paste_canceled {
                    ctx.memory_mut(|m| {
                        m.request_focus(egui::Id::unique(("browser.surface", id, surface)))
                    });
                    self.browser_send(BrowserCommand::Focus {
                        id,
                        surface,
                        focused: true,
                    });
                    self.browser_ui.focused = Some((id, surface));
                    self.browser_send(BrowserCommand::Text { id, surface, text });
                }
                self.browser_ui.paste_canceled = false;
            } else if !in_time {
                self.browser_ui.pending_paste = None;
                self.browser_ui.paste_requested = None;
                self.browser_ui.paste_canceled = false;
            }
        }
        let views: Vec<_> = self.service.document_views().iter().map(|v| v.id).collect();
        if let Some(&first) = views.first() {
            for tab in self.browser_ui.tabs.values_mut() {
                if !views.contains(&tab.view) {
                    tab.view = first;
                }
            }
            let orphan: Vec<_> = self
                .browser_ui
                .active
                .iter()
                .filter(|(v, _)| !views.contains(v))
                .map(|(v, id)| (*v, *id))
                .collect();
            for (view, id) in orphan {
                self.browser_ui.active.remove(&view);
                self.browser_ui.active.entry(first).or_insert(id);
            }
        }
        self.browser_ui
            .tabs
            .retain(|id, _| self.service.browser.tabs.contains_key(id));
        self.browser_ui
            .active
            .retain(|_, id| self.browser_ui.tabs.contains_key(id));
        self.browser_ui
            .revealed
            .retain(|view, _| self.browser_ui.active.contains_key(view));
        self.browser_ui
            .textures
            .retain(|(id, _), _| self.browser_ui.tabs.contains_key(id));
        self.browser_ui.frontend_epochs.retain(|id, _| {
            self.service
                .browser
                .tabs
                .get(id)
                .is_some_and(|tab| tab.devtools_open)
        });
        for tab in self
            .service
            .browser
            .tabs
            .values()
            .filter(|tab| tab.devtools_open)
        {
            if self
                .browser_ui
                .frontend_epochs
                .insert(tab.id, tab.frontend_epoch)
                != Some(tab.frontend_epoch)
            {
                // A reopened SDK frontend is a new CEF surface, even if Close
                // and Open happened between two app polls. Resend its geometry
                // instead of retaining the previous surface's resize cache.
                self.browser_ui.sizes.remove(&(tab.id, Surface::DevTools));
                self.browser_ui
                    .textures
                    .remove(&(tab.id, Surface::DevTools));
                if self.browser_ui.focused == Some((tab.id, Surface::DevTools)) {
                    self.browser_ui.focused = None;
                }
            }
        }
        let focused = if self.browser_input_blocked(ctx) {
            None
        } else {
            let widget = ctx.memory(|m| m.focused());
            self.browser_ui.tabs.keys().find_map(|&id| {
                [Surface::Page, Surface::DevTools]
                    .into_iter()
                    .find_map(|surface| {
                        (widget == Some(egui::Id::unique(("browser.surface", id, surface))))
                            .then_some((id, surface))
                    })
            })
        };
        if self.browser_ui.focused != focused {
            if let Some((id, surface)) = self.browser_ui.focused {
                self.browser_send(BrowserCommand::Focus {
                    id,
                    surface,
                    focused: false,
                });
            }
            if let Some((id, surface)) = focused {
                self.browser_send(BrowserCommand::Focus {
                    id,
                    surface,
                    focused: true,
                });
            }
            self.browser_ui.focused = focused;
        }
        for (origin, url) in self.service.browser.take_popups() {
            let view = self.browser_ui.tabs.get(&origin).map(|t| t.view);
            if let Some(view) = view
                && self.service.view(view).is_some()
            {
                self.send(Command::ActivateView { view });
            }
            self.open_browser_url(&url);
        }
        // Retain GPU images only for visible surfaces. Web state stays in CEF,
        // while hidden tabs don't accumulate full-resolution texture copies.
        self.browser_ui.textures.retain(|(id, surface), _| {
            self.browser_ui.active.values().any(|active| active == id)
                && (*surface == Surface::Page
                    || self
                        .service
                        .browser
                        .tabs
                        .get(id)
                        .is_some_and(|t| t.devtools_open))
        });
        if self.service.browser.state == EngineState::Failed {
            self.browser_ui.textures.clear();
        }
        let states: Vec<_> = self.service.browser.tabs.values().cloned().collect();
        for state in states {
            let visible = self.browser_ui.active.values().any(|id| *id == state.id);
            for (surface, was, desired) in [
                (Surface::Page, state.visible, visible),
                (
                    Surface::DevTools,
                    state.devtools_visible,
                    visible && state.devtools_open,
                ),
            ] {
                if was != desired {
                    self.browser_send(BrowserCommand::Visible {
                        id: state.id,
                        surface,
                        visible: desired,
                    });
                }
            }
            if let Some(tab) = self.browser_ui.tabs.get_mut(&state.id)
                && !ctx.memory(|m| m.has_focus(egui::Id::unique(("browser.address", state.id))))
                && !tab.focus_address
                && tab.notice.is_none()
                && state.error.is_none()
                && !state.url.is_empty()
            {
                tab.address = state.url;
            }
        }
        if self.browser_ui.pending_paste.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        if self.service.browser.is_busy()
            || self
                .service
                .browser
                .tabs
                .keys()
                .any(|id| self.service.browser_devtools_pending(*id))
        {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    pub(super) fn browser_tabs(&mut self, ui: &mut egui::Ui, view: ViewId) {
        let tabs: Vec<_> = self
            .browser_ui
            .tabs
            .iter()
            .filter(|(_, t)| t.view == view)
            .map(|(&id, _)| id)
            .collect();
        let mut close = None;
        for id in tabs {
            let state = self.service.browser.tabs.get(&id).cloned();
            let title = state
                .as_ref()
                .map(|s| s.title.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("Navegador");
            let selected = self.browser_ui.active.get(&view) == Some(&id);
            let font = FontId::proportional(self.tokens.ui_font_size);
            let text_width = ui
                .painter()
                .layout_no_wrap(title.into(), font.clone(), color(self.tokens.text))
                .size()
                .x;
            let width =
                (text_width + self.tokens.tab_text_inset + self.tokens.tab_trailing_space + 22.)
                    .clamp(120., 240.);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(width.floor(), self.tokens.tab_height),
                egui::Sense::hover(),
            );
            let signature = (id, title.to_owned());
            if selected && self.browser_ui.revealed.get(&view) != Some(&signature) {
                // A newly activated or retitled tab must reveal its close control
                // inside the existing horizontal tab strip, including compact views.
                ui.scroll_to_rect(rect, None);
                self.browser_ui.revealed.insert(view, signature);
            }
            let close_rect =
                egui::Rect::from_min_max(egui::pos2(rect.right() - 28., rect.top() + 1.), rect.max);
            let body =
                egui::Rect::from_min_max(rect.min, egui::pos2(close_rect.left(), rect.bottom()));
            let response = ui.interact(
                body,
                egui::Id::unique(("browser.tab", view, id)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    selected,
                    format!("Navegador: {title}"),
                )
            });
            if response.clicked() {
                if self.service.view(view).is_some() {
                    self.send(Command::ActivateView { view });
                }
                self.jira_deactivate_view(view);
                self.browser_ui.active.insert(view, id);
                if self.api_ui.active_in_view(view) {
                    self.api_ui.active = None;
                }
                self.focus_editor = false;
                ui.memory_mut(|m| {
                    m.request_focus(egui::Id::unique(("browser.surface", id, Surface::Page)))
                });
                self.browser_send(BrowserCommand::Focus {
                    id,
                    surface: Surface::Page,
                    focused: true,
                });
                self.browser_ui.focused = Some((id, Surface::Page));
            }
            if response.hovered() {
                ui.painter().rect_filled(
                    rect.shrink(2.),
                    3.,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            globe(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 19., rect.center().y),
                    egui::Vec2::splat(16.),
                ),
                color(self.tokens.text),
            );
            ui.painter()
                .with_clip_rect(body.shrink2(egui::vec2(4., 0.)))
                .text(
                    egui::pos2(rect.left() + 34., rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    title,
                    font,
                    color(self.tokens.text),
                );
            let x = ui.interact(
                close_rect,
                egui::Id::unique(("browser.tab.close", view, id)),
                egui::Sense::click(),
            );
            x.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    format!("Fechar navegador {title}"),
                )
            });
            if selected || response.hovered() || x.hovered() {
                self.tab_paint.close(
                    ui,
                    egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 16., rect.center().y),
                        egui::Vec2::splat(self.tokens.tab_close_size),
                    ),
                    tab_paint::CloseStyle {
                        foreground: color(if x.hovered() {
                            self.tokens.tab_close_hover_foreground
                        } else {
                            self.tokens.tab_close_foreground
                        }),
                        background: color(self.tokens.editor),
                        hover_fill: color(self.tokens.tab_close_hover_background),
                        hovered: x.hovered(),
                        padding: egui::vec2(
                            self.tokens.tab_close_padding[0],
                            self.tokens.tab_close_padding[1],
                        ),
                        stroke: self.tokens.tab_close_stroke,
                        hover_radius: self.tokens.tab_close_hover_radius,
                    },
                );
            }
            if x.clicked() || response.clicked_by(egui::PointerButton::Middle) {
                close = Some(id);
            }
            if selected {
                self.tab_paint.underline(
                    ui,
                    egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 4.), rect.max),
                    [
                        color(self.tokens.accent),
                        color(self.tokens.editor),
                        color(self.tokens.border),
                    ],
                    self.tokens.tab_underline_radius,
                    self.tokens.tab_underline_inset,
                );
            }
            response.on_hover_text(state.as_ref().map_or("Navegador integrado", |s| &s.url));
        }
        if let Some(id) = close {
            self.browser_close(id);
        }
    }
    fn browser_toolbar_tools(&mut self, ui: &mut egui::Ui, id: BrowserId, devtools_open: bool) {
        use super::browser_library_ui::Mode;
        let can_save = self
            .service
            .browser
            .tabs
            .get(&id)
            .is_some_and(|t| codisseu_core::browser_library::origin(&t.url).is_ok());
        if ui
            .add_enabled_ui(can_save, |ui| {
                icon_button(ui, Icon::Star, "Salvar página nos favoritos", false, 24.)
            })
            .inner
            .clicked()
        {
            self.browser_saved_open(id, Mode::Bookmark);
        }
        if icon_button(ui, Icon::Folder, "Favoritos do navegador", false, 24.).clicked() {
            self.browser_saved_open(id, Mode::Favorites);
        }
        if icon_button(
            ui,
            Icon::PrimaryKey,
            "Acessos e senhas deste site",
            false,
            24.,
        )
        .clicked()
        {
            self.browser_saved_open(id, Mode::Access);
        }
        if icon_button(
            ui,
            Icon::Locate,
            "Inspecionar página",
            self.browser_inspector_active(id),
            24.,
        )
        .clicked()
        {
            if self.browser_inspector_active(id) {
                self.service.stop_browser_inspector();
            } else {
                self.browser_inspector_open(id);
            }
        }
        if icon_button(
            ui,
            Icon::Code,
            "Ferramentas do desenvolvedor (F12)",
            devtools_open,
            24.,
        )
        .clicked()
        {
            self.browser_send(BrowserCommand::DevTools {
                id,
                open: !devtools_open,
            });
        }
        if icon_button(ui, Icon::Debug, "Depurar esta aba na IDE", false, 24.).clicked() {
            self.browser_debug_configuration(id);
        }
        if icon_button(ui, Icon::Add, "Nova aba de navegador", false, 24.).clicked() {
            self.open_browser_url("about:blank");
        }
    }
    pub(super) fn browser_panel(&mut self, ui: &mut egui::Ui, view: ViewId) {
        let Some(id) = self.browser_ui.active.get(&view).copied() else {
            return;
        };
        let Some(state) = self.service.browser.tabs.get(&id).cloned() else {
            return;
        };
        let mut navigate = None;
        let chrome = egui::Frame::new()
            .fill(color(self.tokens.panel))
            .inner_margin(egui::Margin::symmetric(5, 4))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4., 4.);
                let compact = ui.available_width() < 420.;
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled_ui(state.can_go_back, |ui| {
                            icon_button(
                                ui,
                                Icon::Previous,
                                "Voltar no navegador (Alt+Left)",
                                false,
                                24.,
                            )
                        })
                        .inner
                        .clicked()
                    {
                        self.browser_send(BrowserCommand::Back { id });
                    }
                    if ui
                        .add_enabled_ui(state.can_go_forward, |ui| {
                            icon_button(
                                ui,
                                Icon::Next,
                                "Avançar no navegador (Alt+Right)",
                                false,
                                24.,
                            )
                        })
                        .inner
                        .clicked()
                    {
                        self.browser_send(BrowserCommand::Forward { id });
                    }
                    let (icon, label) = if state.loading {
                        (Icon::Stop, "Parar carregamento")
                    } else {
                        (Icon::Refresh, "Recarregar página (F5)")
                    };
                    if icon_button(ui, icon, label, false, 24.).clicked() {
                        self.browser_send(if state.loading {
                            BrowserCommand::Stop { id }
                        } else {
                            BrowserCommand::Reload {
                                id,
                                ignore_cache: false,
                            }
                        });
                    }
                    if let Some(tab) = self.browser_ui.tabs.get_mut(&id) {
                        let key = egui::Id::unique(("browser.address", id));
                        let response = egui::TextEdit::singleline(&mut tab.address)
                            .id(key)
                            .hint_text("URL · localhost:5173 ou https://…")
                            .desired_width(
                                (ui.available_width() - if compact { 0. } else { 200. }).max(40.),
                            )
                            .char_limit(8192)
                            .show(ui);
                        response.response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::TextInput,
                                ui.is_enabled(),
                                "Endereço do navegador",
                            )
                        });
                        if tab.focus_address {
                            response.response.request_focus();
                            if tab.select_address {
                                let mut edit_state = response.state;
                                edit_state.cursor.set_char_range(Some(
                                    egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(tab.address.chars().count()),
                                    ),
                                ));
                                edit_state.store(ui.ctx(), key);
                            }
                            tab.focus_address = false;
                            tab.select_address = false;
                        }
                        if response.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter))
                        {
                            navigate = Some(tab.address.clone());
                        }
                    }
                    if !compact {
                        self.browser_toolbar_tools(ui, id, state.devtools_open);
                    }
                });
                if compact {
                    ui.horizontal(|ui| self.browser_toolbar_tools(ui, id, state.devtools_open));
                }
            });
        self.browser_ui
            .chrome
            .insert(id, chrome.response.rect.intersect(ui.clip_rect()));
        if let Some(url) = navigate {
            self.browser_send(BrowserCommand::Navigate { id, url });
        }
        let error = self
            .browser_ui
            .tabs
            .get(&id)
            .and_then(|t| t.notice.clone())
            .or(state.error.clone())
            .or_else(|| self.service.browser.last_error.clone());
        if let Some(error) = error {
            egui::Frame::new()
                .fill(color(self.tokens.panel))
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color(self.tokens.error), error);
                        if ui.button("Tentar novamente").clicked() {
                            if let Some(tab) = self.browser_ui.tabs.get_mut(&id) {
                                tab.notice = None;
                            }
                            self.browser_send(BrowserCommand::Reload {
                                id,
                                ignore_cache: false,
                            });
                        }
                    });
                });
        }
        let rect = ui.available_rect_before_wrap();
        if self.browser_inspector_active(id) {
            self.browser_inspector_layout(ui, id, rect);
        } else if state.devtools_open {
            let stacked = rect.width() < 700.;
            let ratio = self.browser_ui.tabs.get(&id).map_or(0.56, |t| t.split);
            let at = if stacked {
                rect.top() + rect.height() * ratio
            } else {
                rect.left() + rect.width() * ratio
            };
            let (page, tools, divider) = if stacked {
                (
                    egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), at - 3.)),
                    egui::Rect::from_min_max(egui::pos2(rect.left(), at + 3.), rect.max),
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left(), at - 3.),
                        egui::pos2(rect.right(), at + 3.),
                    ),
                )
            } else {
                (
                    egui::Rect::from_min_max(rect.min, egui::pos2(at - 3., rect.bottom())),
                    egui::Rect::from_min_max(egui::pos2(at + 3., rect.top()), rect.max),
                    egui::Rect::from_min_max(
                        egui::pos2(at - 3., rect.top()),
                        egui::pos2(at + 3., rect.bottom()),
                    ),
                )
            };
            let drag = ui.interact(
                divider,
                egui::Id::unique(("browser.devtools.split", id)),
                egui::Sense::drag(),
            );
            drag.clone().on_hover_cursor(if stacked {
                egui::CursorIcon::ResizeVertical
            } else {
                egui::CursorIcon::ResizeHorizontal
            });
            if drag.dragged()
                && let Some(pos) = drag.interact_pointer_pos()
                && let Some(tab) = self.browser_ui.tabs.get_mut(&id)
            {
                tab.split = if stacked {
                    (pos.y - rect.top()) / rect.height()
                } else {
                    (pos.x - rect.left()) / rect.width()
                }
                .clamp(0.2, 0.8);
            }
            ui.painter()
                .rect_filled(divider, 0., color(self.tokens.border));
            self.browser_surface(ui, id, Surface::Page, page);
            self.browser_surface(ui, id, Surface::DevTools, tools);
        } else {
            self.browser_surface(ui, id, Surface::Page, rect);
        }
        ui.advance_cursor_after_rect(rect);
    }
    pub(super) fn browser_surface(
        &mut self,
        ui: &mut egui::Ui,
        id: BrowserId,
        surface: Surface,
        rect: egui::Rect,
    ) {
        self.browser_ui.rects.insert((id, surface), rect);
        if rect.width() < 16. || rect.height() < 16. {
            return;
        }
        if surface == Surface::Page
            && self
                .service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| t.error.is_some())
        {
            self.browser_ui.textures.remove(&(id, surface));
            ui.painter()
                .rect_filled(rect, 0., color(self.tokens.editor));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Não foi possível carregar a página. Revise o endereço ou use Voltar/Recarregar.",
                FontId::proportional(self.tokens.ui_font_size),
                color(self.tokens.muted),
            );
            return;
        }

        let scale = ui.ctx().pixels_per_point().clamp(0.5, 4.);
        let width = rect.width().floor() as u32;
        let height = rect.height().floor() as u32;
        let size = (width, height, scale.to_bits());
        if self.browser_ui.sizes.get(&(id, surface)) != Some(&size) {
            self.browser_ui.sizes.insert((id, surface), size);
            self.browser_send(BrowserCommand::Resize {
                id,
                surface,
                width,
                height,
                scale,
            });
        }
        if let Some(frame) = self.service.browser.take_frame(id, surface) {
            let newer = self
                .browser_ui
                .textures
                .get(&(id, surface))
                .is_none_or(|t| t.seq != frame.seq);
            if newer {
                let image = egui::ColorImage::from_rgba_premultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.rgba,
                );
                if let Some(texture) = self.browser_ui.textures.get_mut(&(id, surface)) {
                    texture.handle.set(image, egui::TextureOptions::LINEAR);
                    texture.seq = frame.seq;
                } else {
                    self.browser_ui.textures.insert(
                        (id, surface),
                        Texture {
                            handle: ui.ctx().load_texture(
                                format!("browser.{id}.{surface:?}"),
                                image,
                                egui::TextureOptions::LINEAR,
                            ),
                            seq: frame.seq,
                        },
                    );
                }
            }
        }
        ui.painter().rect_filled(rect, 0., Color32::WHITE);
        if let Some(texture) = self.browser_ui.textures.get(&(id, surface)) {
            ui.painter()
                .with_clip_rect(rect.intersect(ui.clip_rect()))
                .image(
                    texture.handle.id(),
                    rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
                    Color32::WHITE,
                );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.service.browser.state == EngineState::Failed {
                    "Navegador indisponível · consulte o erro acima"
                } else {
                    "Preparando navegador…"
                },
                FontId::proportional(self.tokens.ui_font_size),
                Color32::DARK_GRAY,
            );
        }
        self.browser_surface_input(ui, id, surface, rect);
    }
    pub(super) fn browser_capture_ready(&self) -> bool {
        self.browser_ui.is_empty()
            || (self.service.browser.state == EngineState::Ready
                && self.browser_ui.active.values().all(|id| {
                    self.service
                        .browser
                        .tabs
                        .get(id)
                        .is_some_and(|t| !t.loading && t.error.is_none())
                        && self.browser_ui.textures.contains_key(&(*id, Surface::Page))
                }))
    }
    pub(super) fn browser_capture_state(&self) -> serde_json::Value {
        serde_json::json!({"engine":format!("{:?}",self.service.browser.state),"engine_pid":self.service.browser.engine_pid(),"versions":self.service.browser.versions,"devtools_state":self.service.browser_devtools_diagnostics(),
            "native_inspector":{"lifecycle":format!("{:?}",self.service.browser_inspector.state.lifecycle),"tab":self.service.browser_inspector.state.session.as_ref().map(|s|s.tab),"document_epoch":self.service.browser_inspector.state.document_epoch,"requests":self.service.browser_inspector.state.network.len(),"dom_nodes":self.service.browser_inspector.state.dom.len(),"cookies_count":self.service.browser_inspector.state.cookies.len(),"paused":self.service.browser_inspector.state.paused.is_some()},
            "tabs":self.service.browser.tabs.values().map(|t|serde_json::json!({"id":t.id,"url":t.url,"title":t.title,"loading":t.loading,"error":t.error,"devtools":t.devtools_open,"visible":t.visible})).collect::<Vec<_>>(),
            "surfaces":self.browser_ui.textures.iter().map(|((id,surface),t)|serde_json::json!({"id":id,"surface":format!("{surface:?}"),"seq":t.seq,"size":t.handle.size(),"rect":self.browser_ui.rects.get(&(*id,*surface)).map(|r|[r.left(),r.top(),r.right(),r.bottom()])})).collect::<Vec<_>>()})
    }
    pub(super) fn browser_input_blocked(&self, ctx: &egui::Context) -> bool {
        self.browser_modal_input_blocked(ctx) || egui::Popup::is_any_open(ctx)
    }
    pub(super) fn browser_modal_input_blocked(&self, ctx: &egui::Context) -> bool {
        self.browser_ui.library.open
            || ctx.memory(|m| m.top_modal_layer().is_some())
            || self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.debugger_ui.dialog
            || self.debugger_ui.breakpoint.is_some()
            || self.python_ui.open
            || self.python_tests_ui.settings
            || self.rust_tools.open
            || self.js_tests_ui.settings
            || self.php_tests_ui.settings
            || self.confirm.is_some()
            || self.database_ui.dialog
            || self.git_ui.dialog.is_some()
            || self.exit_when_saved
    }
    fn browser_cycle_tabs(&mut self, ctx: &egui::Context, backwards: bool) -> bool {
        #[derive(PartialEq)]
        enum Resource {
            Document(DocumentId),
            Api(api_client::ApiTab),
            Browser(BrowserId),
            Jira(crate::jira_tabs::Kind),
        }
        let view = self.service.active_view().unwrap_or(0);
        if !self.browser_ui.tabs.values().any(|t| t.view == view) && !self.jira_ui.tabs.has(view) {
            return false;
        }
        let mut tabs: Vec<_> = self
            .service
            .view(view)
            .map(|v| v.tabs().iter().copied().map(Resource::Document).collect())
            .unwrap_or_default();
        if self.api_ui.view.unwrap_or(0) == view {
            tabs.extend(self.api_ui.tabs.iter().cloned().map(Resource::Api));
        }
        tabs.extend(
            self.browser_ui
                .tabs
                .iter()
                .filter(|(_, t)| t.view == view)
                .map(|(&id, _)| Resource::Browser(id)),
        );
        for kind in [crate::jira_tabs::Kind::Hours, crate::jira_tabs::Kind::Issue] {
            if self.jira_ui.tabs.owner(kind) == Some(view) {
                tabs.push(Resource::Jira(kind));
            }
        }
        if tabs.is_empty() {
            return false;
        }
        let current = self
            .jira_ui
            .tabs
            .selected(view)
            .map(Resource::Jira)
            .or_else(|| {
                self.browser_ui
                    .active
                    .get(&view)
                    .copied()
                    .map(Resource::Browser)
            })
            .or_else(|| {
                if self.api_ui.active_in_view(view) {
                    self.api_ui.active.clone().map(Resource::Api)
                } else {
                    None
                }
            })
            .or_else(|| {
                self.service
                    .view_document(view)
                    .map(|d| Resource::Document(d.id))
            });
        let index = current
            .as_ref()
            .and_then(|current| tabs.iter().position(|tab| tab == current))
            .unwrap_or(0);
        let next = if backwards {
            (index + tabs.len() - 1) % tabs.len()
        } else {
            (index + 1) % tabs.len()
        };
        match &tabs[next] {
            Resource::Document(id) => {
                self.browser_ui.deactivate(view);
                self.api_ui.active = None;
                self.send(Command::ActivateTab { view, id: *id });
                self.focus_editor = true;
            }
            Resource::Jira(kind) => {
                self.jira_open_tab(*kind);
            }
            Resource::Api(tab) => {
                self.jira_deactivate_view(view);
                self.browser_ui.deactivate(view);
                self.api_ui.active = Some(tab.clone());
            }
            Resource::Browser(id) => {
                self.jira_deactivate_view(view);
                self.api_ui.active = None;
                self.browser_ui.active.insert(view, *id);
                self.focus_editor = false;
                ctx.memory_mut(|m| {
                    m.request_focus(egui::Id::unique(("browser.surface", *id, Surface::Page)))
                });
                self.browser_send(BrowserCommand::Focus {
                    id: *id,
                    surface: Surface::Page,
                    focused: true,
                });
                self.browser_ui.focused = Some((*id, Surface::Page));
            }
        }
        true
    }
    fn browser_filter_shortcut_releases(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                !matches!(event, egui::Event::Key { key, pressed: false, .. }
                if self.browser_ui.shortcut_releases.remove(key))
            })
        });
    }
    pub(super) fn browser_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let pressed = |ctx: &egui::Context| {
            ctx.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::Key {
                            key, pressed: true, ..
                        } = event
                        {
                            Some(*key)
                        } else {
                            None
                        }
                    })
                    .collect::<std::collections::BTreeSet<_>>()
            })
        };
        let before = pressed(ctx);
        let handled = self.browser_shortcuts_inner(ctx);
        if handled {
            let after = pressed(ctx);
            self.browser_ui
                .shortcut_releases
                .extend(before.difference(&after).copied());
            // A native action owns the complete key gesture. In particular the
            // release of Ctrl+W must not reach the newly focused next CEF page.
            self.browser_filter_shortcut_releases(ctx);
        }
        handled
    }
    fn browser_shortcuts_inner(&mut self, ctx: &egui::Context) -> bool {
        if self.browser_input_blocked(ctx) {
            return false;
        }
        let backwards = ctx.input(|i| i.modifiers.shift);
        if (self
            .browser_ui
            .tabs
            .values()
            .any(|t| t.view == self.service.active_view().unwrap_or(0))
            || self
                .jira_ui
                .tabs
                .has(self.service.active_view().unwrap_or(0)))
            && (consume_exact(ctx, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Tab)
                || consume_exact(ctx, Modifiers::CTRL, Key::Tab))
            && self.browser_cycle_tabs(ctx, backwards)
        {
            return true;
        }
        let Some(id) = self.browser_active() else {
            return false;
        };
        if ctx.memory(|m| m.focused()).is_some_and(|focus| {
            focus != egui::Id::unique(("browser.address", id))
                && focus != egui::Id::unique(("browser.surface", id, Surface::Page))
                && focus != egui::Id::unique(("browser.surface", id, Surface::DevTools))
                && !(self.browser_inspector_active(id) && self.browser_inspector_ui.has_focus(ctx))
                && !self.browser_ui.chrome.get(&id).is_some_and(|rect| {
                    ctx.read_response(focus)
                        .is_some_and(|response| rect.contains(response.rect.center()))
                })
        }) {
            return false;
        }
        // Keep application commands available without forwarding source-editing
        // shortcuts to the document behind this browser tab.
        if consume_exact(ctx, Modifiers::ALT, Key::F12) {
            self.toggle_terminal();
            return true;
        }
        if consume_exact(ctx, Modifiers::CTRL | Modifiers::ALT, Key::S) {
            self.preferences_dialog = true;
            return true;
        }
        if consume_exact(ctx, Modifiers::CTRL | Modifiers::ALT, Key::Tab) {
            self.next_editor_group();
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND | Modifiers::SHIFT, Key::O) {
            self.open_path(PathAction::Workspace);
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::O) {
            self.open_path(PathAction::File);
            return true;
        }
        if consume_exact(ctx, Modifiers::SHIFT, Key::F9) {
            self.browser_debug_configuration(id);
            return true;
        }
        if self.browser_inspector_active(id) {
            use codisseu_core::browser::inspector::Action;
            let action = if consume_exact(ctx, Modifiers::SHIFT, Key::F8) {
                Some(Action::StepOut)
            } else if consume_exact(ctx, Modifiers::NONE, Key::F8) {
                Some(Action::StepOver)
            } else if consume_exact(ctx, Modifiers::NONE, Key::F7) {
                Some(Action::StepInto)
            } else if consume_exact(ctx, Modifiers::NONE, Key::F9) {
                Some(Action::Resume)
            } else {
                None
            };
            if let Some(action) = action {
                self.browser_inspector_action(action);
                return true;
            }
        } else {
            self.debugger_execution_shortcuts(ctx);
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::D) {
            self.browser_saved_open(id, super::browser_library_ui::Mode::Bookmark);
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::L) {
            if let Some(tab) = self.browser_ui.tabs.get_mut(&id) {
                tab.focus_address = true;
                tab.select_address = true;
            }
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::W) {
            self.browser_close(id);
            return true;
        }
        if consume_exact(ctx, Modifiers::NONE, Key::F12) {
            let open = self
                .service
                .browser
                .tabs
                .get(&id)
                .is_some_and(|t| !t.devtools_open);
            self.browser_send(BrowserCommand::DevTools { id, open });
            return true;
        }
        if consume_exact(ctx, Modifiers::NONE, Key::F5)
            || consume_exact(ctx, Modifiers::COMMAND, Key::R)
        {
            self.browser_send(BrowserCommand::Reload {
                id,
                ignore_cache: false,
            });
            return true;
        }
        if consume_exact(ctx, Modifiers::ALT, Key::ArrowLeft) {
            self.browser_send(BrowserCommand::Back { id });
            return true;
        }
        if consume_exact(ctx, Modifiers::ALT, Key::ArrowRight) {
            self.browser_send(BrowserCommand::Forward { id });
            return true;
        }
        // Browser text/shortcuts must never reach the document behind the tab.
        true
    }
}
fn globe(ui: &egui::Ui, rect: egui::Rect, tint: Color32) {
    let p = ui.painter();
    let c = rect.center();
    let r = rect.width() * 0.4;
    let stroke = egui::Stroke::new(1., tint);
    p.circle_stroke(c, r, stroke);
    p.line_segment([c - egui::vec2(r, 0.), c + egui::vec2(r, 0.)], stroke);
    let points = (0..=24)
        .map(|i| {
            let a = i as f32 * std::f32::consts::TAU / 24.;
            c + egui::vec2(a.cos() * r * 0.45, a.sin() * r)
        })
        .collect();
    p.add(egui::Shape::closed_line(points, stroke));
}
