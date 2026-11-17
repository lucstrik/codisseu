//! Native reconstruction of the measured VS Code terminal surface (see contract).
use super::*;
use crate::scrollbars::ScrollAreaExt as _;
use crate::{CodisseuApp, ToolTab};

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    foreground: Color32,
    muted: Color32,
    selected: Color32,
    border: Color32,
    hover: Color32,
    popup: Color32,
}
impl Palette {
    fn new(tokens: &crate::Tokens) -> Self {
        Self {
            background: crate::color(tokens.panel),
            foreground: crate::color(tokens.text),
            muted: crate::color(tokens.muted),
            selected: crate::color(tokens.selection),
            border: crate::color(tokens.border),
            hover: crate::color(tokens.tab_more_hover_background),
            popup: crate::color(tokens.navigation_popup_background),
        }
    }
}

#[derive(Default)]
pub(crate) struct TerminalUi {
    pub(crate) maximized: bool,
    picker: bool,
    picker_focus: bool,
    filter: String,
    selection: usize,
    sidebar_width: Option<f32>,
    resize_start: f32,
}
fn ui_font(size: f32) -> FontId {
    FontId::new(size, egui::FontFamily::Name("codisseu.terminal.ui".into()))
}
fn glyph(ui: &egui::Ui, position: Pos2, ch: char, size: f32, tint: Color32) {
    ui.painter().text(
        position,
        egui::Align2::CENTER_CENTER,
        ch,
        FontId::new(size, egui::FontFamily::Name("codisseu.codicons".into())),
        tint,
    );
}
fn action(
    ui: &mut egui::Ui,
    palette: Palette,
    ch: char,
    label: &str,
    width: f32,
    size: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 4., palette.selected);
    }
    glyph(
        ui,
        rect.center(),
        ch,
        size,
        if ui.is_enabled() {
            palette.foreground
        } else {
            palette.muted
        },
    );
    response.on_hover_text(label)
}
fn row_action_button(
    ui: &egui::Ui,
    id: egui::Id,
    rect: Rect,
    palette: Palette,
    ch: char,
    label: String,
) -> egui::Response {
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), &label)
    });
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect.shrink(1.), 3., palette.hover);
    }
    glyph(ui, rect.center(), ch, 16., palette.foreground);
    response.on_hover_text(label)
}
#[derive(Clone)]
struct Tab {
    id: u64,
    group: u64,
    shell: String,
    cwd: PathBuf,
    active: bool,
    split: bool,
}
impl TerminalPane {
    pub(super) fn session_count(&self) -> usize {
        usize::from(self.session.is_some())
            + self.splits.iter().map(Self::session_count).sum::<usize>()
            + self.parked.iter().map(Self::session_count).sum::<usize>()
    }
    fn tabs(&self) -> Vec<Tab> {
        let mut groups: Vec<_> = std::iter::once(self).chain(self.parked.iter()).collect();
        groups.sort_by_key(|p| p.identity);
        let mut tabs = Vec::new();
        for group in groups {
            for (index, pane) in std::iter::once(group)
                .chain(group.splits.iter())
                .enumerate()
            {
                if pane.session.is_none() {
                    continue;
                }
                tabs.push(Tab {
                    id: pane.identity,
                    group: group.identity,
                    shell: pane
                        .shell
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    cwd: pane.cwd.clone(),
                    active: group.identity == self.identity
                        && pane.identity == self.focused_child.unwrap_or(self.identity),
                    split: index > 0,
                });
            }
        }
        tabs
    }
    fn activate(&mut self, tab: &Tab) {
        if self.identity != tab.group {
            if let Some(index) = self.parked.iter().position(|p| p.identity == tab.group) {
                self.select_session(index);
            } else {
                return;
            }
        }
        self.focused_child = (tab.id != self.identity).then_some(tab.id);
        self.focus = self.identity == tab.id;
        for split in &mut self.splits {
            split.focus = split.identity == tab.id;
        }
    }
    fn split_session(&mut self, ctx: &egui::Context) {
        if self.session_count() >= 16 || self.splits.len() >= 3 {
            self.message = Some("Limite de 16 sessões ou 4 terminais por divisão atingido".into());
            return;
        }
        let source = self
            .splits
            .iter()
            .find(|p| p.is_focused(ctx) || Some(p.identity) == self.focused_child)
            .unwrap_or(self);
        let cwd = source.cwd.clone();
        if source.session.is_none() {
            return;
        }
        let mut pane = Self::default();
        pane.open(&cwd);
        self.focus = false;
        for split in &mut self.splits {
            split.focus = false;
        }
        self.focused_child = Some(pane.identity);
        self.splits.push(pane);
    }
    fn kill_tab(&mut self, tab: &Tab) {
        let previous = self.tabs().into_iter().find(|t| t.active && t.id != tab.id);
        self.activate(tab);
        if self.identity == tab.id {
            self.close_active_session();
        } else if let Some(index) = self.splits.iter().position(|p| p.identity == tab.id) {
            let mut pane = self.splits.remove(index);
            pane.close();
            self.closing.append(&mut pane.closing);
            self.focused_child = None;
            self.request_focus();
        }
        if let Some(previous) = previous
            && self.tabs().iter().any(|t| t.id == previous.id)
        {
            self.activate(&previous);
        }
    }
    fn kill_focused(&mut self, ctx: &egui::Context) {
        let id = self
            .splits
            .iter()
            .find(|p| p.is_focused(ctx) || Some(p.identity) == self.focused_child)
            .map_or(self.identity, |p| p.identity);
        if let Some(tab) = self.tabs().into_iter().find(|t| t.id == id) {
            self.kill_tab(&tab);
        }
    }
    fn clear_focused(&mut self, ctx: &egui::Context) {
        let index = self
            .splits
            .iter()
            .position(|p| p.is_focused(ctx) || Some(p.identity) == self.focused_child);
        let pane = match index {
            Some(i) => &mut self.splits[i],
            None => self,
        };
        pane.parser = vt100::Parser::new(nz(pane.dimensions.0), nz(pane.dimensions.1), SCROLLBACK);
        pane.selection = None;
        pane.request_focus();
    }
    fn show_group(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let rect = ui.available_rect_before_wrap();
        let count = self.splits.len() + 1;
        let width = ((rect.width() - (count - 1) as f32) / count as f32).max(1.);
        let mut splits = std::mem::take(&mut self.splits);
        for (index, pane) in std::iter::once(&mut *self)
            .chain(splits.iter_mut())
            .enumerate()
        {
            let area = Rect::from_min_size(
                rect.min + egui::vec2(index as f32 * (width + 1.), 0.),
                egui::vec2(width, rect.height()),
            );
            if index > 0 {
                ui.painter().vline(
                    area.left() - 1.,
                    area.y_range(),
                    Stroke::new(1., palette.border),
                );
            }
            let content = area;
            let content = Rect::from_min_max(
                content.min + egui::vec2(20., 8.),
                content.max - egui::vec2(8., 8.),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("terminal.pane", pane.identity))
                    .max_rect(content)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(content.intersect(ui.clip_rect()));
            child.visuals_mut().extreme_bg_color = palette.background;
            child.visuals_mut().override_text_color = Some(palette.foreground);
            pane.show_screen(&mut child, true);
        }
        if self.is_focused(ui.ctx()) {
            self.focused_child = None;
        }
        for split in &splits {
            if split.is_focused(ui.ctx()) {
                self.focused_child = Some(split.identity);
            }
        }
        self.splits = splits;
        ui.advance_cursor_after_rect(rect);
    }
}
impl CodisseuApp {
    pub(crate) fn terminal_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.terminal_ui.picker {
            return true;
        }
        if (self.show_activity
            && self.tool_tab == ToolTab::Terminal
            && self.terminal.is_focused(ctx)
            && crate::consume_exact(ctx, egui::Modifiers::COMMAND, Key::J))
            || crate::consume_exact(ctx, egui::Modifiers::COMMAND, Key::Backtick)
        {
            self.toggle_terminal();
            return true;
        }
        if crate::consume_exact(
            ctx,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            Key::Backtick,
        ) {
            if self.show_activity && self.tool_tab == ToolTab::Terminal {
                self.show_activity = false;
                self.focus_editor = true;
            } else {
                self.show_activity = true;
                self.tool_tab = ToolTab::Terminal;
                self.docker_terminal_active = false;
                if self.service.workspace_file.is_some() && self.service.workspace_roots().len() > 1
                {
                    self.choose_terminal_directory();
                } else {
                    self.open_local_terminal();
                }
            }
            return true;
        }
        if self.show_activity
            && self.tool_tab == ToolTab::Terminal
            && !self.docker_terminal_active
            && crate::consume_exact(
                ctx,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                Key::Num5,
            )
        {
            self.terminal.split_session(ctx);
            return true;
        }
        false
    }
    fn choose_terminal_directory(&mut self) {
        let roots = self.service.workspace_roots();
        if roots.len() == 1 {
            self.terminal.open_directory(&roots[0]);
            self.focus_editor = false;
        } else if roots.len() > 1 {
            self.terminal_ui.picker = true;
            self.terminal_ui.picker_focus = true;
            self.terminal_ui.filter.clear();
            self.terminal_ui.selection = 0;
        } else if self.service.workspace_file.is_none() {
            if let Ok(path) = std::env::current_dir() {
                self.terminal.open_directory(&path);
            }
        } else {
            self.service.status = "Nenhuma pasta disponível para abrir o terminal".into();
        }
    }
    fn terminal_launch_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(320.);
        if ui.button("Novo terminal…").clicked() {
            self.choose_terminal_directory();
            ui.close();
        }
        if ui.button("Dividir terminal").clicked() {
            self.terminal.split_session(ui.ctx());
            ui.close();
        }
        ui.separator();
        let shell = std::env::var_os("SHELL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("sh"));
        let name = format!(
            "{} (Padrão)",
            shell.file_name().unwrap_or_default().to_string_lossy()
        );
        if ui.button(name).clicked() {
            self.choose_terminal_directory();
            ui.close();
        }
        ui.separator();
        if ui.button("Escolher pasta de trabalho…").clicked() {
            self.choose_terminal_directory();
            ui.close();
        }
    }
    fn terminal_more_menu(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(300.);
        if ui.button("Limpar terminal").clicked() {
            self.terminal.clear_focused(ui.ctx());
            ui.close();
        }
        if ui.button("Dividir terminal").clicked() {
            self.terminal.split_session(ui.ctx());
            ui.close();
        }
        if ui.button("Encerrar terminal atual").clicked() {
            self.terminal.kill_focused(ui.ctx());
            ui.close();
        }
        if ui.button("Encerrar todos os terminais").clicked() {
            self.terminal.close();
            ui.close();
        }
        ui.separator();
        for (tab, label) in [
            (ToolTab::Results, "Services"),
            (ToolTab::Git, "Git"),
            (ToolTab::References, "Referências"),
            (ToolTab::Docker, "Docker"),
        ] {
            if ui.button(label).clicked() {
                self.tool_tab = tab;
                ui.close();
            }
        }
    }
    pub(crate) fn tool_panel_header(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::new(&self.tokens);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 32.), Sense::hover());
        let is_terminal = self.tool_tab == ToolTab::Terminal && !self.docker_terminal_active;
        let single = self.terminal.session_count() <= 1;
        let actions_width = if is_terminal && single { 276. } else { 134. };
        let left = Rect::from_min_max(
            rect.min + egui::vec2(2., 4.),
            egui::pos2(
                (rect.right() - actions_width).max(rect.left() + 72.),
                rect.bottom() - 4.,
            ),
        );
        let mut tabs = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("terminal.navigation")
                .max_rect(left)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        tabs.set_clip_rect(left.intersect(ui.clip_rect()));
        tabs.spacing_mut().item_spacing.x = 4.;
        let nav = [
            (ToolTab::Git, "Git"),
            (ToolTab::Debug, "Debug"),
            (
                if self.php_tests_ui.python {
                    ToolTab::PythonTests
                } else if self.php_tests_ui.js || self.service.js_tests.debug_session.is_some() {
                    ToolTab::JsTests
                } else {
                    ToolTab::PhpTests
                },
                "Run",
            ),
            (ToolTab::Terminal, "Terminal"),
            (ToolTab::Results, "Services"),
            (ToolTab::Activity, "Atividade"),
            (ToolTab::References, "Referências"),
            (ToolTab::Docker, "Docker"),
        ];
        egui::ScrollArea::horizontal()
            .id_salt("terminal.tools.navigation")
            .max_width(left.width())
            .max_height(24.)
            .min_scrolled_height(24.)
            .auto_shrink([false, false])
            .codisseu()
            .show(&mut tabs, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.;
                    for (tab, label) in nav {
                        let width = ui
                            .painter()
                            .layout_no_wrap(label.into(), ui_font(13.), palette.foreground)
                            .size()
                            .x
                            + 16.;
                        let (area, response) =
                            ui.allocate_exact_size(egui::vec2(width, 24.), Sense::click());
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::accesskit::Role::Tab,
                                true,
                                tab == self.tool_tab
                                    || (tab == ToolTab::References
                                        && self.tool_tab == ToolTab::Rename),
                                label,
                            )
                        });
                        if tab == self.tool_tab
                            || (tab == ToolTab::References && self.tool_tab == ToolTab::Rename)
                            || response.hovered()
                        {
                            ui.painter().rect_filled(
                                area,
                                4.,
                                if tab == self.tool_tab
                                    || (tab == ToolTab::References
                                        && self.tool_tab == ToolTab::Rename)
                                {
                                    palette.selected
                                } else {
                                    palette.hover
                                },
                            );
                        }
                        ui.painter().text(
                            area.center(),
                            egui::Align2::CENTER_CENTER,
                            label,
                            ui_font(13.),
                            if tab == self.tool_tab
                                || (tab == ToolTab::References && self.tool_tab == ToolTab::Rename)
                            {
                                palette.foreground
                            } else {
                                palette.muted
                            },
                        );
                        if response.clicked() {
                            self.activate_tool_tab(tab);
                        }
                    }
                });
            });
        let right = Rect::from_min_max(
            egui::pos2(
                (rect.right() - actions_width).max(left.right()),
                rect.top() + 5.,
            ),
            rect.max - egui::vec2(4., 5.),
        );
        let mut toolbar = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("terminal.toolbar")
                .max_rect(right)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        toolbar.set_clip_rect(right.intersect(ui.clip_rect()));
        toolbar.spacing_mut().item_spacing.x = 4.;
        if action(
            &mut toolbar,
            palette,
            '\u{ea76}',
            "Ocultar painel inferior",
            22.,
            16.,
        )
        .clicked()
        {
            self.show_activity = false;
            self.focus_editor = true;
        }
        if !is_terminal {
            let label = match self.tool_tab {
                ToolTab::Results => "Configurar serviços de banco",
                ToolTab::Debug => "Configurar depuração",
                ToolTab::Docker => "Configurar ferramenta Docker",
                ToolTab::PhpTests => "Configurar testes PHP",
                ToolTab::JsTests => "Configurar testes JS/TS",
                ToolTab::PythonTests => "Configurar testes Python",
                _ => "Providers",
            };
            if crate::components::icon_button(
                &mut toolbar,
                crate::components::Icon::Settings,
                label,
                false,
                22.,
            )
            .clicked()
            {
                match self.tool_tab {
                    ToolTab::Results => self.database_ui.dialog = true,
                    ToolTab::Debug => self.debugger_open_configurations(),
                    ToolTab::Docker => self.docker_open_settings(),
                    ToolTab::PhpTests => self.open_php_tests(true),
                    ToolTab::JsTests => self.open_js_tests(true),
                    ToolTab::PythonTests => self.open_python_tests(true),
                    _ => self.provider_dialog = true,
                }
            }
            return;
        }
        let (icon, label) = if self.terminal_ui.maximized {
            ('\u{eb4d}', "Restaurar painel")
        } else {
            ('\u{eb4c}', "Maximizar painel")
        };
        if action(&mut toolbar, palette, icon, label, 22., 16.).clicked() {
            self.terminal_ui.maximized = !self.terminal_ui.maximized;
        }
        let (separator, _) = toolbar.allocate_exact_size(egui::vec2(1., 16.), Sense::hover());
        toolbar.painter().vline(
            separator.center().x,
            separator.y_range(),
            Stroke::new(1., palette.muted),
        );
        let more = action(
            &mut toolbar,
            palette,
            '\u{ea7c}',
            "Mais ações do terminal",
            22.,
            16.,
        );
        egui::Popup::menu(&more).show(|ui| self.terminal_more_menu(ui));
        if single {
            if action(
                &mut toolbar,
                palette,
                '\u{ea81}',
                "Encerrar terminal atual",
                22.,
                16.,
            )
            .clicked()
            {
                self.terminal.kill_focused(toolbar.ctx());
            }
            if action(
                &mut toolbar,
                palette,
                '\u{eb56}',
                "Dividir terminal",
                22.,
                16.,
            )
            .clicked()
            {
                self.terminal.split_session(toolbar.ctx());
            }
        }
        toolbar.spacing_mut().item_spacing.x = 0.;
        let arrow = action(
            &mut toolbar,
            palette,
            '\u{eab4}',
            "Escolher pasta do terminal",
            16.,
            12.,
        );
        egui::Popup::menu(&arrow).show(|ui| self.terminal_launch_menu(ui));
        if action(&mut toolbar, palette, '\u{ea60}', "Novo terminal", 22., 16.).clicked() {
            self.choose_terminal_directory();
        }
        if single {
            toolbar.add_space(6.);
            let title = self
                .terminal
                .tabs()
                .first()
                .map(|t| {
                    format!(
                        "{} - {}",
                        t.shell,
                        self.service
                            .workspace_folder_name(&t.cwd)
                            .unwrap_or_else(|| t
                                .cwd
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or(""))
                    )
                })
                .unwrap_or_default();
            if title.is_empty() {
                return;
            }
            // Reserve the icon inside the toolbar clip before giving the rest
            // to the title. RTL allocation previously placed the icon outside it.
            let (title_rect, response) = toolbar.allocate_exact_size(
                egui::vec2(toolbar.available_width().max(0.), 22.),
                Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &title)
            });
            if title_rect.width() >= 20. {
                glyph(
                    &toolbar,
                    egui::pos2(title_rect.left() + 9., title_rect.center().y),
                    '\u{ea85}',
                    16.,
                    palette.foreground,
                );
                let text_rect =
                    Rect::from_min_max(title_rect.min + egui::vec2(22., 0.), title_rect.max);
                let mut text = toolbar.new_child(
                    egui::UiBuilder::new()
                        .id_salt("terminal.single.title")
                        .max_rect(text_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                text.set_clip_rect(text_rect.intersect(toolbar.clip_rect()));
                text.add(
                    egui::Label::new(egui::RichText::new(&title).font(ui_font(11.))).truncate(),
                );
            }
            if response.clicked() {
                self.terminal.request_focus();
            }
            response.on_hover_text(title);
        }
    }
    pub(crate) fn terminal_reference_panel(&mut self, ui: &mut egui::Ui) {
        let palette = Palette::new(&self.tokens);
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0., palette.background);
        ui.spacing_mut().item_spacing.y = 0.;
        ui.visuals_mut().override_text_color = Some(palette.foreground);
        ui.style_mut().override_font_id = Some(ui_font(13.));
        self.tool_panel_header(ui);
        let area = ui.available_rect_before_wrap();
        let tabs = self.terminal.tabs();
        let mut width = 0.;
        if tabs.len() > 1 {
            let maximum = (area.width() - 100.).clamp(48., 480.);
            width = self
                .terminal_ui
                .sidebar_width
                .unwrap_or(120.)
                .clamp(48., maximum);
            let divider = Rect::from_min_max(
                egui::pos2(area.right() - width - 3., area.top()),
                egui::pos2(area.right() - width + 3., area.bottom()),
            );
            let resize = ui.interact(
                divider,
                egui::Id::unique("terminal.tabs.resize"),
                Sense::click_and_drag(),
            );
            resize.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Splitter,
                    true,
                    "Largura da lista de terminais",
                )
            });
            if resize.hovered() || resize.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if resize.drag_started() {
                self.terminal_ui.resize_start = width;
            }
            if resize.dragged() {
                width = (self.terminal_ui.resize_start
                    - resize.total_drag_delta().unwrap_or_default().x)
                    .clamp(48., maximum);
                self.terminal_ui.sidebar_width = Some(width);
            }
            if resize.double_clicked() {
                width = 120_f32.clamp(48., maximum);
                self.terminal_ui.sidebar_width = Some(width);
            }
            if resize.has_focus() {
                let direction = ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft) {
                        10.
                    } else if i.consume_key(egui::Modifiers::NONE, Key::ArrowRight) {
                        -10.
                    } else {
                        0.
                    }
                });
                if direction != 0. {
                    width = (width + direction).clamp(48., maximum);
                    self.terminal_ui.sidebar_width = Some(width);
                }
            }
        }
        if width > 0. {
            let sidebar =
                Rect::from_min_max(egui::pos2(area.right() - width, area.top()), area.max);
            ui.painter().vline(
                sidebar.left(),
                sidebar.y_range(),
                Stroke::new(1., palette.border),
            );
            let mut list = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("terminal.tabs")
                    .max_rect(sidebar.shrink2(egui::vec2(4., 0.)))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            list.set_clip_rect(sidebar.intersect(ui.clip_rect()));
            list.spacing_mut().item_spacing.y = 0.;
            egui::ScrollArea::vertical()
                .id_salt("terminal.tab.scroll")
                .codisseu()
                .show(&mut list, |ui| {
                    for tab in tabs {
                        let alias = self
                            .service
                            .workspace_folder_name(&tab.cwd)
                            .map(str::to_owned)
                            .unwrap_or_else(|| {
                                tab.cwd
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned()
                            });
                        let (row, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 22.),
                            Sense::click(),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::accesskit::Role::Tab,
                                true,
                                tab.active,
                                format!("Terminal {} em {alias}", tab.id),
                            )
                        });
                        if tab.active || response.hovered() {
                            ui.painter().rect_filled(
                                row,
                                4.,
                                if tab.active {
                                    palette.selected
                                } else {
                                    palette.hover
                                },
                            );
                        }
                        let indent = if tab.split { 12. } else { 0. };
                        glyph(
                            ui,
                            egui::pos2(row.left() + 12. + indent, row.center().y),
                            '\u{ea85}',
                            16.,
                            palette.foreground,
                        );
                        let split_rect = Rect::from_min_size(
                            egui::pos2(row.right() - 44., row.top()),
                            egui::vec2(22., 22.),
                        );
                        let kill_rect = Rect::from_min_size(
                            egui::pos2(row.right() - 22., row.top()),
                            egui::vec2(22., 22.),
                        );
                        let split_id = ui.scope_id().with(("terminal.row.split", tab.id));
                        let kill_id = ui.scope_id().with(("terminal.row.kill", tab.id));
                        let hovered = ui.rect_contains_pointer(row)
                            || response.has_focus()
                            || ui.memory(|m| m.has_focus(split_id) || m.has_focus(kill_id));
                        let buttons = hovered && row.width() >= 80.;
                        let text_right = if buttons {
                            split_rect.left()
                        } else {
                            row.right()
                        };
                        let text_rect = Rect::from_min_max(
                            row.min + egui::vec2(22. + indent, 0.),
                            egui::pos2(text_right.max(row.left() + 22. + indent), row.bottom()),
                        );
                        ui.painter().with_clip_rect(text_rect).text(
                            text_rect.left_center(),
                            egui::Align2::LEFT_CENTER,
                            format!("{}  {alias}", tab.shell),
                            ui_font(13.),
                            palette.foreground,
                        );
                        let mut row_action = false;
                        if buttons {
                            let split = row_action_button(
                                ui,
                                split_id,
                                split_rect,
                                palette,
                                '\u{eb56}',
                                format!("Dividir terminal {}", tab.id),
                            );
                            let kill = row_action_button(
                                ui,
                                kill_id,
                                kill_rect,
                                palette,
                                '\u{ea81}',
                                format!("Excluir terminal {}", tab.id),
                            );
                            if split.clicked() {
                                self.terminal.activate(&tab);
                                self.terminal.split_session(ui.ctx());
                                row_action = true;
                            }
                            if kill.clicked() {
                                self.terminal.kill_tab(&tab);
                                row_action = true;
                            }
                        }
                        if response.clicked() && !row_action {
                            self.terminal.activate(&tab);
                            self.focus_editor = false;
                        }
                        response
                            .clone()
                            .on_hover_text(tab.cwd.display().to_string());
                        response.context_menu(|ui| {
                            if ui.button("Dividir terminal").clicked() {
                                self.terminal.activate(&tab);
                                self.terminal.split_session(ui.ctx());
                                ui.close();
                            }
                            if ui.button("Encerrar terminal atual").clicked() {
                                self.terminal.kill_tab(&tab);
                                ui.close();
                            }
                        });
                    }
                });
        }
        let content = Rect::from_min_max(
            area.min,
            egui::pos2(
                area.right() - width - if width > 0. { 3. } else { 0. },
                area.bottom(),
            ),
        );
        let mut terminal = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("terminal.content")
                .max_rect(content)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        terminal.set_clip_rect(content.intersect(ui.clip_rect()));
        self.terminal.show_group(&mut terminal, palette);
        ui.advance_cursor_after_rect(area);
    }
    pub(crate) fn terminal_root_picker(&mut self, ctx: &egui::Context) {
        let palette = Palette::new(&self.tokens);
        if !self.terminal_ui.picker {
            return;
        }
        let roots: Vec<_> = self
            .service
            .workspace_roots()
            .into_iter()
            .map(|path| {
                let name = self
                    .service
                    .workspace_folder_name(&path)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    });
                (name, path)
            })
            .collect();
        let id = egui::Id::unique("terminal.root.picker");
        let mut chosen = None;
        let modal = egui::Modal::new(id)
            .area(
                egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, egui::vec2(0., 6.)),
            )
            .backdrop_color(Color32::TRANSPARENT)
            .frame(
                egui::Frame::new()
                    .fill(palette.popup)
                    .stroke(Stroke::new(1., palette.border))
                    .corner_radius(6.)
                    .inner_margin(6.),
            )
            .show(ctx, |ui| {
                ui.set_width(588_f32.min(ctx.content_rect().width() - 28.));
                ui.spacing_mut().item_spacing.y = 4.;
                ui.style_mut().override_font_id = Some(ui_font(13.));
                let submit = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
                let next = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowDown));
                let previous = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::ArrowUp));
                let input = ui.add_sized(
                    [ui.available_width(), 26.],
                    egui::TextEdit::singleline(&mut self.terminal_ui.filter)
                        .hint_text("Selecione a pasta de trabalho para o novo terminal")
                        .align(egui::Align2::LEFT_CENTER),
                );
                input.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Pasta do novo terminal",
                    )
                });
                if self.terminal_ui.picker_focus {
                    input.request_focus();
                    self.terminal_ui.picker_focus = false;
                }
                if input.changed() {
                    self.terminal_ui.selection = 0;
                }
                let roots: Vec<_> = roots
                    .iter()
                    .filter(|(name, path)| {
                        format!("{name} {}", path.display())
                            .to_lowercase()
                            .contains(&self.terminal_ui.filter.to_lowercase())
                    })
                    .cloned()
                    .collect();
                self.terminal_ui.selection = self
                    .terminal_ui
                    .selection
                    .min(roots.len().saturating_sub(1));
                if next && !roots.is_empty() {
                    self.terminal_ui.selection = (self.terminal_ui.selection + 1) % roots.len();
                }
                if previous && !roots.is_empty() {
                    self.terminal_ui.selection =
                        (self.terminal_ui.selection + roots.len() - 1) % roots.len();
                }
                if submit {
                    chosen = roots
                        .get(self.terminal_ui.selection)
                        .map(|(_, p)| p.clone());
                }
                egui::ScrollArea::vertical()
                    .max_height(220.)
                    .codisseu()
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        if roots.is_empty() {
                            ui.label("Nenhuma pasta encontrada");
                        }
                        for (index, (name, path)) in roots.iter().enumerate() {
                            let (row, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 22.),
                                Sense::click(),
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::Button,
                                    true,
                                    format!("Terminal em {name}"),
                                )
                            });
                            if index == self.terminal_ui.selection || response.hovered() {
                                ui.painter().rect_filled(row, 3., palette.selected);
                            }
                            let name_galley = ui.painter().layout_no_wrap(
                                name.clone(),
                                ui_font(13.),
                                palette.foreground,
                            );
                            let x = row.left() + 8.;
                            ui.painter().galley(
                                egui::pos2(x, row.center().y - name_galley.size().y / 2.),
                                name_galley.clone(),
                                palette.foreground,
                            );
                            ui.painter().with_clip_rect(row).text(
                                egui::pos2(x + name_galley.size().x + 6., row.center().y),
                                egui::Align2::LEFT_CENTER,
                                path.display().to_string(),
                                ui_font(12.),
                                palette.muted,
                            );
                            if response.clicked() {
                                chosen = Some(path.clone());
                            }
                        }
                    });
            });
        if modal.should_close() {
            self.terminal_ui.picker = false;
            self.terminal.request_focus();
        }
        if let Some(path) = chosen {
            self.terminal_ui.picker = false;
            self.terminal.open_directory(&path);
            self.focus_editor = false;
        }
    }
}

#[cfg(test)]
#[path = "terminal_chrome_tests.rs"]
mod chrome_tests;
