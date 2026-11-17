//! Project-scoped native HTTP workbench. The worker owns network and disk work.
use super::*;
use codisseu_core::{ViewId, api::*};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ApiTab {
    Request(String),
    Collection(String),
    Folder(String),
    Globals,
    Environment(String),
    Specs,
    Mocks,
    History,
}
#[derive(Default)]
pub(super) struct ApiUi {
    pub open: bool,
    pub active: Option<ApiTab>,
    pub tabs: Vec<ApiTab>,
    pub view: Option<ViewId>,
    filter: String,
    move_item: Option<String>,
    url_edit: Option<(String, String)>,
    run_iterations: u32,
    run_data: String,
    request_page: usize,
    request_fraction: Option<f32>,
    response_page: usize,
    script_page: usize,
    rename: Option<(ApiTab, String)>,
    transfer: Option<Transfer>,
    dirty_at: Option<std::time::Instant>,
    project: Option<PathBuf>,
    notice: Option<String>,
    file_rx: Option<mpsc::Receiver<Result<String, String>>>,
    scripts_enabled: bool,
    response_cache: Option<(String, u64, String)>,
}
struct Transfer {
    text: String,
    path: String,
    export: bool,
    kind: usize,
}
impl ApiUi {
    pub(super) fn active_in_view(&self, view: ViewId) -> bool {
        self.active.is_some() && self.view.unwrap_or(0) == view
    }
    pub(super) fn close_active(&mut self) {
        if let Some(active) = self.active.take() {
            self.tabs.retain(|tab| tab != &active);
            self.active = self.tabs.last().cloned();
        }
    }
    pub(super) fn open_tab(&mut self, tab: ApiTab) {
        if !self.tabs.contains(&tab) {
            self.tabs.push(tab.clone());
        }
        self.active = Some(tab);
    }
    fn changed(&mut self) {
        self.dirty_at = Some(std::time::Instant::now());
    }
}
#[derive(Default)]
struct RequestActions {
    send: bool,
    curl: bool,
    changed: bool,
}

impl CodisseuApp {
    pub(super) fn api_open_tab(&mut self, tab: ApiTab) {
        self.jira_deactivate_view(self.service.active_view().unwrap_or(0));
        self.browser_ui
            .deactivate(self.service.active_view().unwrap_or(0));
        self.api_ui.view = Some(self.service.active_view().unwrap_or(0));
        self.api_ui.open_tab(tab);
    }
    pub(super) fn api_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            return;
        }
        // The service identity can be a .code-workspace file. Its carrier
        // directory alone does not identify the collection or its UI state.
        if self.api_ui.project != self.service.api.project {
            self.api_ui.project = self.service.api.project.clone();
            self.api_ui.tabs.clear();
            self.api_ui.active = None;
            self.api_ui.dirty_at = None;
            self.api_ui.transfer = None;
            self.api_ui.rename = None;
            self.api_ui.move_item = None;
            self.api_ui.url_edit = None;
            self.api_ui.notice = None;
            self.api_ui.file_rx = None;
            self.api_ui.response_cache = None;
            self.api_ui.scripts_enabled = false;
            self.api_ui.run_iterations = 1;
            self.api_ui.run_data.clear();
            self.api_ui.filter.clear();
        }
        if self
            .api_ui
            .view
            .is_some_and(|view| self.service.view(view).is_none())
        {
            self.api_ui.view = Some(self.service.active_view().unwrap_or(0));
        }
        if !self.service.api.loading {
            let workspace = &self.service.api.workspace;
            self.api_ui.tabs.retain(|tab| match tab {
                ApiTab::Request(id) => workspace.find_request(id).is_some(),
                ApiTab::Folder(id) => workspace.item(id).is_some(),
                ApiTab::Collection(id) => workspace.collections.iter().any(|c| &c.id == id),
                ApiTab::Environment(id) => workspace.environments.iter().any(|e| &e.id == id),
                _ => true,
            });
            if self
                .api_ui
                .active
                .as_ref()
                .is_some_and(|tab| !self.api_ui.tabs.contains(tab))
            {
                self.api_ui.active = self.api_ui.tabs.last().cloned();
            }
        }
        if self
            .api_ui
            .dirty_at
            .is_some_and(|time| time.elapsed() > Duration::from_millis(600))
        {
            self.api_ui.dirty_at = None;
            if let Err(error) = self.service.api.save() {
                self.api_ui.notice = Some(error);
            }
        }
        if self.api_ui.dirty_at.is_some() || self.service.api.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
        if let Some(rx) = &self.api_ui.file_rx {
            match rx.try_recv() {
                Ok(Ok(text)) => {
                    if let Some(transfer) = &mut self.api_ui.transfer {
                        transfer.text = text;
                    }
                    self.api_ui.file_rx = None;
                }
                Ok(Err(error)) => {
                    self.api_ui.notice = Some(error);
                    self.api_ui.file_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.api_ui.file_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(80));
                }
            }
        }
    }
    pub(super) fn api_tabs(&mut self, ui: &mut egui::Ui, view: ViewId) {
        if self.api_ui.view.unwrap_or(0) != view {
            return;
        }
        let mut close = None;
        for tab in self.api_ui.tabs.clone() {
            let name = match &tab {
                ApiTab::Request(id) => self
                    .service
                    .api
                    .workspace
                    .find_request(id)
                    .map(|r| format!("{} {}", r.method, r.name)),
                ApiTab::Collection(id) => self
                    .service
                    .api
                    .workspace
                    .collections
                    .iter()
                    .find(|c| &c.id == id)
                    .map(|c| c.name.clone()),
                ApiTab::Environment(id) => self
                    .service
                    .api
                    .workspace
                    .environments
                    .iter()
                    .find(|e| &e.id == id)
                    .map(|e| e.name.clone()),
                ApiTab::Folder(id) => self
                    .service
                    .api
                    .workspace
                    .item(id)
                    .map(|i| i.name().to_owned()),
                ApiTab::Globals => Some("Variáveis do projeto".into()),
                ApiTab::Specs => Some("Specs".into()),
                ApiTab::Mocks => Some("Mocks".into()),
                ApiTab::History => Some("History".into()),
            };
            let Some(name) = name else {
                continue;
            };
            let selected = self.api_ui.active.as_ref() == Some(&tab);
            let font = FontId::proportional(self.tokens.ui_font_size);
            let galley =
                ui.painter()
                    .layout_no_wrap(name.clone(), font.clone(), color(self.tokens.text));
            let width =
                (galley.size().x + self.tokens.tab_text_inset + self.tokens.tab_trailing_space)
                    .clamp(90., 240.)
                    .floor();
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(width, self.tokens.tab_height),
                egui::Sense::hover(),
            );
            let glyph = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 16., rect.center().y),
                egui::Vec2::splat(self.tokens.tab_close_size),
            );
            let close_rect = egui::Rect::from_min_max(
                egui::pos2(glyph.left(), rect.top() + 1.),
                egui::pos2(glyph.right(), rect.bottom()),
            );
            let body = egui::Rect::from_min_max(
                rect.min + egui::vec2(0., 1.),
                egui::pos2(close_rect.left(), rect.bottom()),
            );
            let response = ui.interact(
                body,
                egui::Id::unique(("api.tab", view, &tab)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    selected,
                    &name,
                )
            });
            if response.clicked() {
                if self.service.view(view).is_some() {
                    self.send(Command::ActivateView { view });
                }
                self.browser_ui.deactivate(view);
                self.jira_deactivate_view(view);
                self.api_ui.active = Some(tab.clone());
            }
            if response.hovered() {
                ui.painter().rect_filled(
                    rect.shrink(2.),
                    3.,
                    ui.visuals().widgets.hovered.weak_bg_fill,
                );
            }
            let painter = ui
                .painter()
                .with_clip_rect(body.shrink2(egui::vec2(4., 0.)));
            painter.text(
                egui::pos2(rect.left() + self.tokens.tab_text_inset, rect.center().y),
                egui::Align2::LEFT_CENTER,
                &name,
                font,
                color(self.tokens.text),
            );
            let close_response = ui.interact(
                close_rect,
                egui::Id::unique(("api.tab.close", view, &tab)),
                egui::Sense::click(),
            );
            close_response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    format!("Fechar API {name}"),
                )
            });
            if selected || response.hovered() || close_response.hovered() {
                let hovered = close_response.hovered() || close_response.has_focus();
                self.tab_paint.close(
                    ui,
                    glyph,
                    tab_paint::CloseStyle {
                        foreground: color(if hovered {
                            self.tokens.tab_close_hover_foreground
                        } else {
                            self.tokens.tab_close_foreground
                        }),
                        background: color(self.tokens.editor),
                        hover_fill: color(self.tokens.tab_close_hover_background),
                        hovered,
                        padding: egui::vec2(
                            self.tokens.tab_close_padding[0],
                            self.tokens.tab_close_padding[1],
                        ),
                        stroke: self.tokens.tab_close_stroke,
                        hover_radius: self.tokens.tab_close_hover_radius,
                    },
                );
            }
            if close_response.clicked() {
                close = Some(tab.clone());
            }
            if selected {
                self.tab_paint.underline(
                    ui,
                    egui::Rect::from_min_max(
                        egui::pos2(rect.left(), rect.bottom() - 4.),
                        rect.right_bottom(),
                    ),
                    [
                        color(self.tokens.accent),
                        color(self.tokens.editor),
                        color(self.tokens.border),
                    ],
                    self.tokens.tab_underline_radius,
                    self.tokens.tab_underline_inset,
                );
            }
        }
        if let Some(tab) = close {
            self.api_ui.tabs.retain(|t| t != &tab);
            if self.api_ui.active.as_ref() == Some(&tab) {
                self.api_ui.active = self.api_ui.tabs.last().cloned();
            }
        }
    }
    pub(super) fn api_panel(&mut self, ui: &mut egui::Ui) {
        self.tokens.form_style(ui);
        let loading = self.service.api.loading;
        if loading {
            ui.disable();
        }
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            ui.strong("API Client");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.);
                if icon_button(ui, Icon::Remove, "Ocultar API Client", false, 26.).clicked() {
                    self.api_ui.open = false;
                }
                if icon_button(
                    ui,
                    Icon::Import,
                    "Importar collection, environment ou cURL",
                    false,
                    26.,
                )
                .clicked()
                {
                    self.api_transfer(false, String::new());
                }
                let add = components::database_icon_button(
                    ui,
                    Icon::Add,
                    "Adicionar API",
                    !loading && self.service.workspace.is_some(),
                );
                egui::Popup::menu(&add)
                    .frame(egui::Frame::popup(ui.style()).fill(color(self.tokens.bar)))
                    .show(|ui| {
                        if ui.button("Collection").clicked() {
                            let id = self.service.api.workspace.new_collection("New Collection");
                            self.api_ui.changed();
                            self.api_open_tab(ApiTab::Collection(id));
                            ui.close();
                        }
                        if ui.button("Pasta").clicked() {
                            let parent = self.api_parent();
                            self.api_add_folder(&parent);
                            ui.close();
                        }
                        if ui.button("Environment").clicked() {
                            let id = self
                                .service
                                .api
                                .workspace
                                .new_environment("New Environment");
                            self.api_ui.changed();
                            self.api_open_tab(ApiTab::Environment(id));
                            ui.close();
                        }
                        if ui.button("Request").clicked() {
                            self.api_new_request(None);
                            ui.close();
                        }
                    });
            });
        });
        egui::Frame::new()
            .inner_margin(self.tokens.content_margin())
            .show(ui, |ui| {
                if let Some(error) = &self.service.api.error {
                    crate::status_messages::status_message(ui, error);
                }
                if !self.service.api.failed_saves.is_empty()
                    && ui.button("Retry failed saves").clicked()
                    && let Err(error) = self.service.api.retry_failed_saves()
                {
                    self.api_ui.notice = Some(error);
                }
                if loading {
                    ui.weak("Carregando perfil API…");
                }
                if self.service.workspace.is_none() {
                    ui.weak("Abra um projeto para organizar suas collections.");
                    return;
                }
                let filter_width = ui.available_width();
                api_field(
                    ui,
                    &mut self.api_ui.filter,
                    "Filtrar collections e requests",
                    "Filtrar collections e requests",
                    filter_width,
                    false,
                );
                egui::ScrollArea::vertical()
                    .id_salt("api.collections")
                    .auto_shrink([false, false])
                    .codisseu()
                    .show(ui, |ui| {
                        egui::CollapsingHeader::new("COLLECTIONS")
                            .default_open(true)
                            .show(ui, |ui| {
                                if self.service.api.workspace.collections.is_empty() {
                                    ui.weak("Nenhuma collection neste projeto");
                                }
                                for collection in self
                                    .service
                                    .api
                                    .workspace
                                    .collections
                                    .iter()
                                    .map(tree_collection)
                                    .collect::<Vec<_>>()
                                {
                                    let filter = self.api_ui.filter.to_lowercase();
                                    if !filter.is_empty()
                                        && !collection.name.to_lowercase().contains(&filter)
                                        && !items_match(&collection.items, &filter)
                                    {
                                        continue;
                                    }
                                    let tab = ApiTab::Collection(collection.id.clone());
                                    let shown = egui::CollapsingHeader::new(&collection.name)
                                        .id_salt((&collection.id, "collection"))
                                        .default_open(true)
                                        .show(ui, |ui| {
                                            self.api_tree(
                                                ui,
                                                &collection.items,
                                                if collection.name.to_lowercase().contains(&filter)
                                                {
                                                    ""
                                                } else {
                                                    &filter
                                                },
                                            );
                                        });
                                    if shown.header_response.double_clicked() {
                                        self.api_open_tab(tab.clone());
                                    }
                                    shown.header_response.context_menu(|ui| {
                                        if ui.button("Run collection").clicked() {
                                            self.api_run_collection(&collection.id);
                                            self.api_open_tab(tab.clone());
                                            ui.close();
                                        }
                                        if ui.button("Abrir collection").clicked() {
                                            self.api_open_tab(tab.clone());
                                            ui.close();
                                        }
                                        if ui.button("Nova request").clicked() {
                                            self.api_new_request(Some(collection.id.clone()));
                                            ui.close();
                                        }
                                        if ui.button("Nova pasta").clicked() {
                                            self.api_add_folder(&collection.id);
                                            ui.close();
                                        }
                                        if ui.button("Renomear").clicked() {
                                            self.api_ui.rename =
                                                Some((tab.clone(), collection.name.clone()));
                                            ui.close();
                                        }
                                        if ui.button("Exportar").clicked() {
                                            self.api_export_collection(&collection.id);
                                            ui.close();
                                        }
                                        if ui.button("Excluir collection").clicked() {
                                            self.service
                                                .api
                                                .workspace
                                                .collections
                                                .retain(|c| c.id != collection.id);
                                            self.api_remove_tab(&tab);
                                            self.api_ui.changed();
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        ui.add_space(12.);
                        ui.separator();
                        egui::CollapsingHeader::new("ENVIRONMENTS")
                            .default_open(true)
                            .show(ui, |ui| {
                                if ui
                                    .selectable_label(
                                        self.service.api.workspace.active_environment.is_none(),
                                        "No environment",
                                    )
                                    .clicked()
                                {
                                    self.service.api.workspace.active_environment = None;
                                    self.api_ui.changed();
                                }
                                for env in self
                                    .service
                                    .api
                                    .workspace
                                    .environments
                                    .iter()
                                    .map(|e| Environment {
                                        id: e.id.clone(),
                                        name: e.name.clone(),
                                        variables: vec![],
                                    })
                                    .collect::<Vec<_>>()
                                {
                                    let tab = ApiTab::Environment(env.id.clone());
                                    let selected =
                                        self.service.api.workspace.active_environment.as_ref()
                                            == Some(&env.id);
                                    let response = ui.selectable_label(
                                        self.api_ui.active.as_ref() == Some(&tab),
                                        format!(
                                            "{}{}",
                                            env.name,
                                            if selected { "  ✓" } else { "" }
                                        ),
                                    );
                                    if response.clicked() {
                                        self.api_open_tab(tab.clone());
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button("Usar este environment").clicked() {
                                            self.service.api.workspace.active_environment =
                                                Some(env.id.clone());
                                            self.api_ui.changed();
                                            ui.close();
                                        }
                                        if ui.button("Renomear").clicked() {
                                            self.api_ui.rename =
                                                Some((tab.clone(), env.name.clone()));
                                            ui.close();
                                        }
                                        if ui.button("Exportar").clicked() {
                                            self.api_export_environment(&env.id);
                                            ui.close();
                                        }
                                        if ui.button("Excluir environment").clicked() {
                                            self.service
                                                .api
                                                .workspace
                                                .environments
                                                .retain(|e| e.id != env.id);
                                            if selected {
                                                self.service.api.workspace.active_environment =
                                                    None;
                                            }
                                            self.api_remove_tab(&tab);
                                            self.api_ui.changed();
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        ui.add_space(12.);
                        ui.separator();
                        for (name, tab) in [
                            ("VARIÁVEIS DO PROJETO", ApiTab::Globals),
                            ("SPECS", ApiTab::Specs),
                            ("MOCKS", ApiTab::Mocks),
                            ("HISTORY", ApiTab::History),
                        ] {
                            if ui
                                .selectable_label(self.api_ui.active.as_ref() == Some(&tab), name)
                                .clicked()
                            {
                                self.api_open_tab(tab);
                            }
                        }
                    });
            });
    }
    fn api_remove_tab(&mut self, tab: &ApiTab) {
        self.api_ui.tabs.retain(|t| t != tab);
        if self.api_ui.active.as_ref() == Some(tab) {
            self.api_ui.active = self.api_ui.tabs.last().cloned();
        }
    }
    fn api_new_request(&mut self, collection: Option<String>) {
        let parent = collection.unwrap_or_else(|| self.api_parent());
        if let Some(id) = self
            .service
            .api
            .workspace
            .new_request(&parent, "New Request")
        {
            self.api_open_tab(ApiTab::Request(id));
            self.api_ui.changed();
        } else {
            self.api_ui.notice = Some(
                "Não foi possível criar a request: destino ausente ou limite de itens atingido."
                    .into(),
            );
        }
    }
    fn api_tree(&mut self, ui: &mut egui::Ui, items: &[CollectionItem], filter: &str) {
        for item in items {
            match item {
                CollectionItem::Request(request) => {
                    if !filter.is_empty()
                        && !request.name.to_lowercase().contains(filter)
                        && !request.url.to_lowercase().contains(filter)
                    {
                        continue;
                    }
                    let tab = ApiTab::Request(request.id.clone());
                    let response = ui.selectable_label(
                        self.api_ui.active.as_ref() == Some(&tab),
                        format!("{}  {}", request.method, request.name),
                    );
                    if response.clicked() {
                        self.api_open_tab(tab.clone());
                    }
                    response.context_menu(|ui| {
                        if ui.button("Renomear").clicked() {
                            self.api_ui.rename = Some((tab.clone(), request.name.clone()));
                            ui.close();
                        }
                        self.api_organize_menu(ui, &request.id);
                        if ui.button("Copiar como cURL").clicked() {
                            self.api_copy_curl(ui.ctx(), &request.id);
                            ui.close();
                        }
                        if ui.button("Excluir request").clicked() {
                            for c in &mut self.service.api.workspace.collections {
                                remove_item(&mut c.items, &request.id);
                            }
                            self.api_remove_tab(&tab);
                            self.api_ui.changed();
                            ui.close();
                        }
                    });
                }
                CollectionItem::Folder {
                    id, name, items, ..
                } => {
                    if !filter.is_empty()
                        && !name.to_lowercase().contains(filter)
                        && !items_match(items, filter)
                    {
                        continue;
                    }
                    let tab = ApiTab::Folder(id.clone());
                    let tree_id = ui.make_persistent_id(("api.folder", id));
                    let mut state =
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            tree_id,
                            true,
                        );
                    if !filter.is_empty() {
                        state.set_open(true);
                    }
                    let (_, shown, _) = state
                        .show_header(ui, |ui| {
                            ui.spacing_mut().item_spacing.x = 2.;
                            ui.spacing_mut().button_padding.x = 4.;
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(18., 20.), egui::Sense::hover());
                            components::paint_icon(ui, rect, Icon::Folder, color(self.tokens.text));
                            let response = ui
                                .selectable_label(self.api_ui.active.as_ref() == Some(&tab), name);
                            if response.clicked() {
                                self.api_open_tab(tab.clone());
                            }
                            response
                        })
                        .body(|ui| {
                            self.api_tree(
                                ui,
                                items,
                                if name.to_lowercase().contains(filter) {
                                    ""
                                } else {
                                    filter
                                },
                            )
                        });
                    shown.inner.context_menu(|ui| {
                        if ui.button("Abrir pasta").clicked() {
                            self.api_open_tab(tab.clone());
                            ui.close();
                        }
                        if ui.button("Executar pasta").clicked() {
                            self.api_run_collection(id);
                            ui.close();
                        }
                        self.api_organize_menu(ui, id);
                        if ui.button("Nova request").clicked() {
                            self.api_new_request(Some(id.clone()));
                            ui.close();
                        }
                        if ui.button("Nova subpasta").clicked() {
                            self.api_add_folder(id);
                            ui.close();
                        }
                        if ui.button("Renomear pasta").clicked() {
                            self.api_ui.rename = Some((ApiTab::Folder(id.clone()), name.clone()));
                            ui.close();
                        }
                        if ui.button("Excluir pasta").clicked() {
                            for c in &mut self.service.api.workspace.collections {
                                remove_item(&mut c.items, id);
                            }
                            self.api_ui.changed();
                            ui.close();
                        }
                    });
                }
            }
        }
    }
    pub(super) fn api_editor(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        let clicked = ui.ctx().input(|input| {
            input.pointer.primary_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|pos| rect.contains(pos))
        });
        if clicked
            && let Some(view) = self.api_ui.view
            && self.service.view(view).is_some()
            && self.service.active_view() != Some(view)
        {
            self.send(Command::ActivateView { view });
        }
        self.tokens.form_style(ui);
        let Some(tab) = self.api_ui.active.clone() else {
            return;
        };
        egui::Frame::new()
            .inner_margin(self.tokens.content_margin())
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if let Some(error) = &self.service.api.error {
                    crate::status_messages::status_message(ui, error);
                }
                if let Some(notice) = &self.api_ui.notice {
                    ui.horizontal_wrapped(|ui| {
                        crate::status_messages::status_message(ui, notice);
                    });
                }
                match tab {
                    ApiTab::Request(id) => self.api_request_editor(ui, &id),
                    ApiTab::Environment(id) => self.api_environment_editor(ui, &id),
                    ApiTab::Collection(id) => self.api_collection_editor(ui, &id),
                    ApiTab::Folder(id) => self.api_folder_editor(ui, &id),
                    ApiTab::Globals => { ui.strong("Variáveis do projeto"); ui.weak("Compartilhadas entre collections apenas neste projeto. Ambiente e variáveis locais têm prioridade.");if kv_table(ui,&mut self.service.api.workspace.globals,true,"project.globals"){self.api_ui.changed();} },
                    ApiTab::Specs => self.api_specs_editor(ui),
                    ApiTab::Mocks => self.api_mocks_editor(ui),
                    ApiTab::History => self.api_history_editor(ui),
                }
            });
    }
    fn api_environment_picker(&mut self, ui: &mut egui::Ui) {
        let mut selected = self.service.api.workspace.active_environment.clone();
        let label = self
            .service
            .api
            .workspace
            .active_environment()
            .map_or("No environment", |e| e.name.as_str());
        let combo = egui::ComboBox::from_id_salt("api.active.environment")
            .selected_text(label)
            .width(180.)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "No environment");
                for env in &self.service.api.workspace.environments {
                    ui.selectable_value(&mut selected, Some(env.id.clone()), &env.name);
                }
            });
        api_named(
            combo.response,
            "Ambiente ativo",
            egui::accesskit::Role::ComboBox,
        );
        if selected != self.service.api.workspace.active_environment {
            self.service.api.workspace.active_environment = selected;
            self.api_ui.changed();
        }
    }
    fn api_request_editor(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(request) = self.service.api.workspace.find_request(id) else {
            ui.weak("Request removida.");
            return;
        };
        let name = request.name.clone();
        ui.horizontal(|ui| {
            ui.strong(name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.api_environment_picker(ui)
            });
        });
        let full_height = ui.available_height().max(1.);
        let busy = self.service.api.running_request.is_some();
        let request = self
            .service
            .api
            .workspace
            .find_request_mut(id)
            .expect("request checked above");
        let actions = request_form(ui, request, &mut self.api_ui, busy);
        if actions.changed {
            self.api_ui.changed();
        }
        if actions.curl {
            self.api_copy_curl(ui.ctx(), id);
        }
        if actions.send {
            if busy {
                self.service.api.cancel();
            } else {
                self.api_send_request(id);
            }
        }
        let (split, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.), egui::Sense::drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Splitter,
                ui.is_enabled(),
                "Redimensionar request e response",
            )
        });
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        ui.painter().hline(
            split.x_range(),
            split.center().y,
            egui::Stroke::new(1., color(self.tokens.border)),
        );
        if response.dragged() {
            self.api_ui.request_fraction = Some(
                (self.api_ui.request_fraction.unwrap_or(0.49)
                    + response.drag_delta().y / full_height)
                    .clamp(0.15, 0.8),
            );
        }
        self.api_response(ui, id);
    }
    fn api_environment_editor(&mut self, ui: &mut egui::Ui, id: &str) {
        let active = self.service.api.workspace.active_environment.as_deref() == Some(id);
        let mut activate = false;
        let mut export = false;
        if let Some(env) = self
            .service
            .api
            .workspace
            .environments
            .iter_mut()
            .find(|e| e.id == id)
        {
            ui.horizontal_wrapped(|ui| {
                if api_field(ui, &mut env.name, "Nome do environment", "", 220., false).changed() {
                    self.api_ui.changed();
                }
                activate = ui.selectable_label(active, "Usar environment").clicked();
                export = ui.button("Exportar").clicked();
            });
            ui.weak("Variáveis {{nome}} são resolvidas apenas neste projeto. Valores marcados como secretos ficam na sessão.");
            if kv_table(ui, &mut env.variables, true, "environment.variables") {
                self.api_ui.changed();
            }
        }
        if activate {
            self.service.api.workspace.active_environment = Some(id.into());
            self.api_ui.changed();
        }
        if export {
            self.api_export_environment(id);
        }
    }
    fn api_collection_editor(&mut self, ui: &mut egui::Ui, id: &str) {
        let running = self
            .service
            .api
            .runner
            .as_ref()
            .is_some_and(|run| !run.finished);
        let mut run = false;
        ui.horizontal_wrapped(|ui| {
            run = ui
                .add_enabled(
                    !running && !self.service.api.loading,
                    egui::Button::new("Run collection"),
                )
                .clicked();
            ui.checkbox(
                &mut self.api_ui.scripts_enabled,
                "Executar scripts desta request e da collection",
            );
            self.api_environment_picker(ui);
        });
        if run {
            self.api_run_collection(id);
        }
        self.api_run_options(ui);
        self.api_runner_bar(ui);
        let mut add = false;
        let mut folder = false;
        let mut export = false;
        if let Some(c) = self
            .service
            .api
            .workspace
            .collections
            .iter_mut()
            .find(|c| c.id == id)
        {
            ui.horizontal_wrapped(|ui| {
                if api_field(ui, &mut c.name, "Nome da collection", "", 220., false).changed() {
                    self.api_ui.changed();
                }
                add = ui.button("Nova request").clicked();
                folder = ui.button("Nova pasta").clicked();
                export = ui.button("Exportar").clicked();
            });
            egui::ScrollArea::vertical().codisseu().show(ui, |ui| {
                ui.label("Description");
                if api_area(ui, &mut c.description, "Descrição da collection", 3, false).changed()
                {
                    self.api_ui.changed();
                }
                ui.separator();
                ui.strong("Variables");
                if kv_table(ui, &mut c.variables, true, "collection.variables") {
                    self.api_ui.changed();
                }
                ui.separator();
                ui.strong("Authorization");
                if auth_form(ui, &mut c.auth) {
                    self.api_ui.changed();
                }
                ui.separator();
                ui.strong("Scripts");
                ui.label("Pre-request");
                if script_field(ui, &mut c.pre_request_script, "collection.pre") {
                    self.api_ui.changed();
                }
                ui.label("Post-response");
                if script_field(ui, &mut c.post_response_script, "collection.post") {
                    self.api_ui.changed();
                }
            });
        }
        if add {
            self.api_new_request(Some(id.into()));
        }
        if folder {
            self.api_add_folder(id);
        }
        if export {
            self.api_export_collection(id);
        }
    }
}

fn items_match(items: &[CollectionItem], filter: &str) -> bool {
    items.iter().any(|item| {
        item.name().to_lowercase().contains(filter)
            || match item {
                CollectionItem::Folder { items, .. } => items_match(items, filter),
                CollectionItem::Request(r) => r.url.to_lowercase().contains(filter),
            }
    })
}
fn remove_item(items: &mut Vec<CollectionItem>, id: &str) {
    items.retain(|item| item.id() != id);
    for item in items {
        if let CollectionItem::Folder { items, .. } = item {
            remove_item(items, id);
        }
    }
}
fn rename_item(items: &mut [CollectionItem], id: &str, name: &str) -> bool {
    for item in items {
        match item {
            CollectionItem::Request(r) if r.id == id => {
                r.name = name.into();
                return true;
            }
            CollectionItem::Folder {
                id: folder,
                name: n,
                ..
            } if folder == id => {
                *n = name.into();
                return true;
            }
            CollectionItem::Folder { items, .. } => {
                if rename_item(items, id, name) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

#[path = "api_client_organization.rs"]
mod organization;

#[path = "api_client_views.rs"]
mod views;

fn api_named(response: egui::Response, label: &str, role: egui::accesskit::Role) -> egui::Response {
    response.widget_info(|| egui::WidgetInfo::labeled(role, response.enabled(), label));
    response
}
fn api_field(
    ui: &mut egui::Ui,
    text: &mut String,
    label: &str,
    hint: &str,
    width: f32,
    secret: bool,
) -> egui::Response {
    let response = ui.add(
        egui::TextEdit::singleline(text)
            .hint_text(hint)
            .desired_width(width.min(ui.max_rect().width().max(40.)))
            .password(secret),
    );
    api_named(response, label, egui::accesskit::Role::TextInput)
}
fn api_area(
    ui: &mut egui::Ui,
    text: &mut dyn egui::TextBuffer,
    label: &str,
    rows: usize,
    code: bool,
) -> egui::Response {
    let edit = egui::TextEdit::multiline(text)
        .desired_rows(rows)
        .desired_width(f32::INFINITY);
    let edit = if code { edit.code_editor() } else { edit };
    let response = ui.add(edit);
    api_named(response, label, egui::accesskit::Role::TextInput)
}
fn api_checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    let response = ui.checkbox(value, "");
    api_named(response, label, egui::accesskit::Role::CheckBox)
}
fn api_remove_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = egui::vec2(4., 5.);
        let height = ui.spacing().interact_size.y;
        let response = ui.add_sized([24., height], egui::Button::new("×"));
        api_named(response, label, egui::accesskit::Role::Button).on_hover_text(label)
    })
    .inner
}
fn kv_table(ui: &mut egui::Ui, rows: &mut Vec<KeyValue>, secrets: bool, id: &str) -> bool {
    let mut changed = false;
    let mut remove = None;
    let mut draft = KeyValue::default();
    let mut append = false;
    let floating = ui.spacing().scroll.floating;
    ui.spacing_mut().scroll.floating = false;
    egui::ScrollArea::both()
        .id_salt(id)
        .max_height(260.)
        .codisseu()
        .show(ui, |ui| {
            let width = ((ui.available_width() - if secrets { 130. } else { 80. }) / 3.)
                .floor()
                .max(90.);
            let header_shape = ui.painter().add(egui::Shape::Noop);
            let mut header_rect = egui::Rect::NOTHING;
            let mut columns = Vec::new();
            let mut row_bottoms = Vec::new();
            let table = egui::Grid::new(id)
                .striped(true)
                .spacing([6., 4.])
                .min_col_width(0.)
                .show(ui, |ui| {
                    for (label, size) in [
                        ("", 18.),
                        ("Key", width),
                        ("Value", width),
                        ("Description", width),
                    ] {
                        let response = ui.add_sized(
                            [size, 24.],
                            egui::Label::new(RichText::new(label).weak()).halign(egui::Align::Min),
                        );
                        header_rect = header_rect.union(response.rect);
                        columns.push(response.rect.right() + 3.);
                    }
                    if secrets {
                        let response = ui.add_sized(
                            [44., 24.],
                            egui::Label::new(RichText::new("Secret").weak())
                                .halign(egui::Align::Min),
                        );
                        header_rect = header_rect.union(response.rect);
                        columns.push(response.rect.right() + 3.);
                    }
                    let response = ui.add_sized([24., 24.], egui::Label::new(""));
                    header_rect = header_rect.union(response.rect);
                    ui.end_row();
                    for (index, row) in rows.iter_mut().enumerate() {
                        let (edited, deleted, bottom) =
                            kv_row(ui, row, secrets, id, index, width, false);
                        changed |= edited;
                        if deleted {
                            remove = Some(index);
                        }
                        row_bottoms.push(bottom);
                        ui.end_row();
                    }
                    // The last row is an editor affordance, never a persisted empty parameter.
                    let (edited, _, bottom) =
                        kv_row(ui, &mut draft, secrets, id, rows.len(), width, true);
                    append = edited;
                    row_bottoms.push(bottom);
                    ui.end_row();
                });
            let rect = table.response.rect;
            let border = ui.visuals().widgets.noninteractive.bg_stroke;
            ui.painter().set(
                header_shape,
                egui::Shape::rect_filled(
                    egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.right(), header_rect.bottom() + 2.),
                    ),
                    0.,
                    ui.visuals().faint_bg_color,
                ),
            );
            for x in columns {
                ui.painter().vline(x, rect.y_range(), border);
            }
            ui.painter()
                .hline(rect.x_range(), header_rect.bottom() + 2., border);
            for y in row_bottoms {
                ui.painter().hline(rect.x_range(), y + 2., border);
            }
            ui.painter()
                .rect_stroke(rect.expand(1.), 0., border, egui::StrokeKind::Inside);
        });
    ui.spacing_mut().scroll.floating = floating;
    if let Some(index) = remove {
        rows.remove(index);
        changed = true;
    }
    if append {
        rows.push(draft);
        changed = true;
    }
    if ui.button("+ Adicionar linha").clicked() {
        rows.push(KeyValue::default());
        changed = true;
    }
    changed
}
fn kv_row(
    ui: &mut egui::Ui,
    row: &mut KeyValue,
    secrets: bool,
    id: &str,
    index: usize,
    width: f32,
    draft: bool,
) -> (bool, bool, f32) {
    let label = format!("{} linha {}", id, index + 1);
    let mut changed = false;
    changed |= api_checkbox(ui, &mut row.enabled, &format!("Habilitar {label}")).changed();
    let key_response = api_field(
        ui,
        &mut row.key,
        &format!("Chave {label}"),
        "Key",
        width,
        false,
    );
    changed |= key_response.changed();
    changed |= api_field(
        ui,
        &mut row.value,
        &format!("Valor {label}"),
        "Value",
        width,
        row.secret,
    )
    .changed();
    let description = api_field(
        ui,
        &mut row.description,
        &format!("Descrição {label}"),
        "Description",
        width,
        false,
    );
    changed |= description.changed();
    if secrets {
        changed |= api_checkbox(ui, &mut row.secret, &format!("Segredo de {label}"))
            .on_hover_text("Manter valor somente nesta sessão")
            .changed();
    }
    let deleted = if draft {
        ui.add_sized([24., ui.spacing().interact_size.y], egui::Label::new(""));
        false
    } else {
        api_remove_button(ui, &format!("Excluir {label}")).clicked()
    };
    (changed, deleted, description.rect.bottom())
}
fn auth_form(ui: &mut egui::Ui, auth: &mut Auth) -> bool {
    let mut index = match auth {
        Auth::Inherit => 0,
        Auth::None => 1,
        Auth::Bearer { .. } => 2,
        Auth::Basic { .. } => 3,
        Auth::ApiKey { .. } => 4,
    };
    let old = index;
    let mut changed = false;
    let combo = egui::ComboBox::from_id_salt("api.auth.type")
        .selected_text(
            [
                "Inherit auth from parent",
                "No Auth",
                "Bearer Token",
                "Basic Auth",
                "API Key",
            ][index],
        )
        .show_ui(ui, |ui| {
            for (i, name) in [
                "Inherit auth from parent",
                "No Auth",
                "Bearer Token",
                "Basic Auth",
                "API Key",
            ]
            .iter()
            .enumerate()
            {
                ui.selectable_value(&mut index, i, *name);
            }
        });
    api_named(
        combo.response,
        "Tipo de autorização",
        egui::accesskit::Role::ComboBox,
    );
    if old != index {
        *auth = match index {
            0 => Auth::Inherit,
            1 => Auth::None,
            2 => Auth::Bearer {
                token: String::new(),
            },
            3 => Auth::Basic {
                username: String::new(),
                password: String::new(),
            },
            _ => Auth::ApiKey {
                key: String::new(),
                value: String::new(),
                in_query: false,
            },
        };
        changed = true;
    }
    match auth {
        Auth::Bearer { token } => {
            ui.label("Token");
            changed |=
                api_field(ui, token, "Bearer Token", "{{token}}", f32::INFINITY, true).changed();
        }
        Auth::Basic { username, password } => {
            ui.label("Username");
            changed |= api_field(ui, username, "Username de Basic Auth", "", 300., false).changed();
            ui.label("Password");
            changed |= api_field(ui, password, "Password de Basic Auth", "", 300., true).changed();
        }
        Auth::ApiKey {
            key,
            value,
            in_query,
        } => {
            ui.label("Key");
            changed |= api_field(ui, key, "Nome da API Key", "", 300., false).changed();
            ui.label("Value");
            changed |= api_field(ui, value, "Valor da API Key", "", 300., true).changed();
            changed |= ui
                .checkbox(in_query, "Adicionar à query (desmarcado: Header)")
                .changed();
        }
        Auth::Inherit => {
            ui.weak("Usa a autorização da pasta ou collection.");
        }
        Auth::None => {
            ui.weak("Esta request não adiciona autorização automaticamente.");
        }
    }
    changed
}
fn script_field(ui: &mut egui::Ui, text: &mut String, id: &str) -> bool {
    let response = ui.add(
        egui::TextEdit::multiline(text)
            .id_salt(id)
            .code_editor()
            .desired_width(f32::INFINITY)
            .desired_rows(9)
            .hint_text("// JavaScript · pm.environment, pm.response, pm.test"),
    );
    api_named(
        response,
        &format!("Script {id}"),
        egui::accesskit::Role::TextInput,
    )
    .changed()
}
fn is_curl(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|word| matches!(word, "curl" | "curl.exe"))
}
fn api_url_field(
    ui: &mut egui::Ui,
    request: &mut ApiRequest,
    state: &mut ApiUi,
    pasted_curl: Option<&str>,
    width: f32,
) -> bool {
    if state
        .url_edit
        .as_ref()
        .is_none_or(|(id, _)| id != &request.id)
    {
        state.url_edit = Some((request.id.clone(), request.display_url()));
    }
    let text = &mut state.url_edit.as_mut().expect("URL draft").1;
    let response = ui.add_sized(
        [width, 30.],
        egui::TextEdit::singleline(text).hint_text("Enter URL or paste cURL"),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::TextInput,
            ui.is_enabled(),
            "URL da requisição",
        )
    });
    let changed = response.changed();
    let edited = state.url_edit.as_ref().expect("URL draft").1.clone();
    if changed && !is_curl(&edited) {
        request.set_url(&edited);
    }
    if response.changed() && response.has_focus() && (pasted_curl.is_some() || is_curl(&edited)) {
        let source = pasted_curl.unwrap_or(&edited);
        match parse_curl(source) {
            Ok(mut parsed) => {
                parsed.id = request.id.clone();
                parsed.name = request.name.clone();
                *request = parsed;
                state.url_edit = None;
                state.notice = None;
            }
            Err(error) => {
                state.notice = Some(error);
            }
        }
    }
    changed
}
fn request_form(
    ui: &mut egui::Ui,
    request: &mut ApiRequest,
    state: &mut ApiUi,
    busy: bool,
) -> RequestActions {
    let mut actions = RequestActions::default();
    // Preserve the original clipboard bytes: single-line TextEdit replaces newlines,
    // which would otherwise destroy POSIX backslash-newline continuations.
    let pasted_curl = ui.input(|input| {
        input.events.iter().find_map(|event| {
            if let egui::Event::Paste(text) = event {
                is_curl(text).then(|| text.clone())
            } else {
                None
            }
        })
    });
    ui.add_space(8.);
    let narrow = ui.available_width() < 360.;
    let bar = |ui: &mut egui::Ui| {
        let combo = egui::ComboBox::from_id_salt("api.method")
            .width(94.)
            .selected_text(&request.method)
            .show_ui(ui, |ui| {
                for method in ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"] {
                    actions.changed |= ui
                        .selectable_value(&mut request.method, method.into(), method)
                        .changed();
                }
            });
        api_named(
            combo.response,
            "Método HTTP",
            egui::accesskit::Role::ComboBox,
        );
        if !narrow {
            let width = (ui.available_width() - 78. - ui.spacing().item_spacing.x).max(80.);
            actions.changed |= api_url_field(ui, request, state, pasted_curl.as_deref(), width);
        }
        actions.send = ui
            .add_sized(
                [78., 30.],
                egui::Button::new(if busy { "Cancel" } else { "Send" }),
            )
            .clicked();
    };
    if narrow {
        ui.horizontal_wrapped(bar);
    } else {
        ui.horizontal(bar);
    }
    if narrow {
        let width = ui.available_width();
        actions.changed |= api_url_field(ui, request, state, pasted_curl.as_deref(), width);
    }
    ui.horizontal_wrapped(|ui| {
        for (i, name) in [
            "Params",
            "Authorization",
            "Headers",
            "Body",
            "Scripts",
            "Settings",
        ]
        .iter()
        .enumerate()
        {
            let populated = match i {
                0 => !request.params.is_empty(),
                1 => !matches!(request.auth, Auth::None | Auth::Inherit),
                2 => !request.headers.is_empty(),
                3 => !matches!(request.body, RequestBody::None),
                4 => {
                    !request.pre_request_script.is_empty()
                        || !request.post_response_script.is_empty()
                }
                _ => false,
            };
            ui.selectable_value(
                &mut state.request_page,
                i,
                format!("{name}{}", if populated { " ·" } else { "" }),
            );
        }
        if ui
            .button("cURL")
            .on_hover_text("Copiar request como cURL")
            .clicked()
        {
            actions.curl = true;
        }
    });
    ui.separator();
    ui.checkbox(
        &mut state.scripts_enabled,
        "Executar scripts desta request e da collection",
    )
    .on_hover_text("Habilite somente scripts que você revisou. Aplicado ao clicar Send.");
    let height = (ui.available_height() * state.request_fraction.unwrap_or(0.49)).max(50.);
    egui::ScrollArea::vertical().id_salt("api.request.fields").max_height(height).min_scrolled_height(height).auto_shrink([false,false]).codisseu().show(ui, |ui| {
        match state.request_page {
            0=>{ui.label("Query Params");if kv_table(ui,&mut request.params,false,"request.params"){actions.changed=true;state.url_edit=None;}
                if !request.path_variables.is_empty(){ui.separator();ui.label("Path Variables");actions.changed|=kv_table(ui,&mut request.path_variables,false,"request.path");}}
            1=>{actions.changed|=auth_form(ui,&mut request.auth);}
            2=>{actions.changed|=kv_table(ui,&mut request.headers,false,"request.headers");}
            3=>{actions.changed|=body_form(ui,&mut request.body);}
            4=>{
                ui.horizontal(|ui| {ui.selectable_value(&mut state.script_page,0,"Pre-request");ui.selectable_value(&mut state.script_page,1,"Post-response");});
                ui.weak("JavaScript isolado: pm.environment, pm.collectionVariables, pm.response, pm.test e pm.expect. Sem acesso ao sistema ou rede.");
                if state.script_page==0 {actions.changed|=script_field(ui,&mut request.pre_request_script,"request.pre");}else{
                    actions.changed|=script_field(ui,&mut request.post_response_script,"request.post");
                    if ui.button("Inserir captura de token").clicked(){request.post_response_script.push_str("\nconst data = pm.response.json();\npm.environment.set(\"token\", data.access_token);\n");actions.changed=true;}
                    ui.weak("Escolha um environment, ajuste data.access_token ao JSON e use Bearer Token = {{token}}.");
                }
            }
            _=>{
                ui.label("Request timeout (ms)");actions.changed|=api_named(ui.add(egui::DragValue::new(&mut request.settings.timeout_ms).range(1..=300_000)),"Request timeout (ms)",egui::accesskit::Role::SpinButton).changed();
                actions.changed|=ui.checkbox(&mut request.settings.follow_redirects,"Automatically follow redirects").changed();
                actions.changed|=ui.checkbox(&mut request.settings.verify_tls,"Enable SSL certificate verification").changed();
                actions.changed|=ui.checkbox(&mut request.settings.use_cookies,"Use cookie jar").changed();
                ui.label("Response limit (bytes)");actions.changed|=api_named(ui.add(egui::DragValue::new(&mut request.settings.max_response_bytes).range(1024..=32*1024*1024)),"Response limit (bytes)",egui::accesskit::Role::SpinButton).changed();
            }
        }
    });
    actions
}
fn body_form(ui: &mut egui::Ui, body: &mut RequestBody) -> bool {
    let mut index = match body {
        RequestBody::None => 0,
        RequestBody::Raw { .. } => 1,
        RequestBody::UrlEncoded(_) => 2,
        RequestBody::FormData(_) => 3,
        RequestBody::Binary(_) | RequestBody::BinaryFile { .. } => 4,
    };
    let old = index;
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (i, label) in [
            "none",
            "raw",
            "x-www-form-urlencoded",
            "form-data",
            "binary",
        ]
        .iter()
        .enumerate()
        {
            ui.selectable_value(&mut index, i, *label);
        }
    });
    if old != index {
        *body = match index {
            0 => RequestBody::None,
            1 => RequestBody::Raw {
                text: String::new(),
                content_type: "application/json".into(),
            },
            2 => RequestBody::UrlEncoded(vec![]),
            3 => RequestBody::FormData(vec![]),
            _ => RequestBody::BinaryFile {
                path: PathBuf::new(),
            },
        };
        changed = true;
    }
    match body {
        RequestBody::Raw { text, content_type } => {
            ui.horizontal(|ui| {
                ui.label("Content-Type");
                changed |=
                    api_field(ui, content_type, "Content-Type do body", "", 300., false).changed();
            });
            changed |= api_area(ui, text, "Conteúdo do body", 10, true).changed();
        }
        RequestBody::UrlEncoded(rows) => {
            changed |= kv_table(ui, rows, false, "request.body");
        }
        RequestBody::FormData(rows) => {
            changed |= multipart_form(ui, rows);
        }
        RequestBody::BinaryFile { path } => {
            ui.label("Arquivo do body binário");
            let mut source = path.to_string_lossy().into_owned();
            let field = ui.add(
                egui::TextEdit::singleline(&mut source)
                    .desired_width(f32::INFINITY)
                    .hint_text("/caminho/arquivo.bin"),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::TextInput,
                    ui.is_enabled(),
                    "Arquivo do body binário",
                )
            });
            if field.changed() {
                *path = PathBuf::from(source);
                changed = true;
            }
            ui.weak("O arquivo é lido em segundo plano somente quando você clicar Send.");
        }
        RequestBody::Binary(bytes) => {
            ui.label(format!("{} bytes", bytes.len()));
            if ui.button("Usar arquivo local").clicked() {
                *body = RequestBody::BinaryFile {
                    path: PathBuf::new(),
                };
                changed = true;
            }
            ui.weak("Conteúdo binário em memória. Send envia esses bytes como body.");
        }
        _ => {}
    }
    changed
}

fn multipart_form(ui: &mut egui::Ui, rows: &mut Vec<KeyValue>) -> bool {
    let mut changed = false;
    let mut remove = None;
    let floating = ui.spacing().scroll.floating;
    ui.spacing_mut().scroll.floating = false;
    ui.weak("Cada campo pode ser Text ou File. Arquivos são lidos somente ao enviar a request.");
    egui::ScrollArea::both()
        .id_salt("api.multipart")
        .max_height(260.)
        .codisseu()
        .show(ui, |ui| {
            let width = ((ui.available_width() - 165.) / 3.).max(95.);
            egui::Grid::new("api.multipart.rows")
                .striped(true)
                .spacing([6., 4.])
                .show(ui, |ui| {
                    ui.label("");
                    ui.weak("Key");
                    ui.weak("Type");
                    ui.weak("Value / File path");
                    ui.weak("Description");
                    ui.label("");
                    ui.end_row();
                    for (index, row) in rows.iter_mut().enumerate() {
                        changed |= api_checkbox(
                            ui,
                            &mut row.enabled,
                            &format!("Habilitar campo multipart {}", index + 1),
                        )
                        .changed();
                        changed |= api_field(
                            ui,
                            &mut row.key,
                            &format!("Chave do campo multipart {}", index + 1),
                            "Key",
                            width,
                            false,
                        )
                        .changed();
                        let mut file = row.file_path.is_some();
                        let old = file;
                        let combo = egui::ComboBox::from_id_salt(("multipart.type", index))
                            .width(65.)
                            .selected_text(if file { "File" } else { "Text" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut file, false, "Text");
                                ui.selectable_value(&mut file, true, "File");
                            });
                        api_named(
                            combo.response,
                            &format!("Tipo do campo multipart {}", index + 1),
                            egui::accesskit::Role::ComboBox,
                        );
                        if file != old {
                            row.file_path = if file { Some(PathBuf::new()) } else { None };
                            changed = true;
                        }
                        if let Some(path) = &mut row.file_path {
                            let mut source = path.to_string_lossy().into_owned();
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut source)
                                    .desired_width(width)
                                    .hint_text("/caminho/arquivo"),
                            );
                            field.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::TextInput,
                                    ui.is_enabled(),
                                    format!("Arquivo multipart {}", row.key),
                                )
                            });
                            if field.changed() {
                                *path = PathBuf::from(source);
                                changed = true;
                            }
                        } else {
                            changed |= api_field(
                                ui,
                                &mut row.value,
                                &format!("Valor do campo multipart {}", index + 1),
                                "Value",
                                width,
                                row.secret,
                            )
                            .changed();
                        }
                        changed |= api_field(
                            ui,
                            &mut row.description,
                            &format!("Descrição do campo multipart {}", index + 1),
                            "Description",
                            width,
                            false,
                        )
                        .changed();
                        if api_remove_button(ui, "Excluir campo multipart").clicked() {
                            remove = Some(index);
                        }
                        ui.end_row();
                    }
                });
        });
    ui.spacing_mut().scroll.floating = floating;
    if let Some(index) = remove {
        rows.remove(index);
        changed = true;
    }
    if ui.button("+ Adicionar campo multipart").clicked() {
        rows.push(KeyValue::default());
        changed = true;
    }
    changed
}

// Copy only navigation metadata; payloads and scripts never enter the tree snapshot.
fn tree_collection(c: &Collection) -> Collection {
    Collection {
        id: c.id.clone(),
        name: c.name.clone(),
        items: tree_items(&c.items),
        ..Collection::default()
    }
}
fn tree_items(items: &[CollectionItem]) -> Vec<CollectionItem> {
    items
        .iter()
        .map(|item| match item {
            CollectionItem::Request(r) => CollectionItem::Request(Box::new(ApiRequest {
                id: r.id.clone(),
                name: r.name.clone(),
                method: r.method.clone(),
                url: r.url.clone(),
                ..ApiRequest::default()
            })),
            CollectionItem::Folder {
                id, name, items, ..
            } => CollectionItem::Folder {
                id: id.clone(),
                name: name.clone(),
                items: tree_items(items),
                auth: Auth::Inherit,
                pre_request_script: String::new(),
                post_response_script: String::new(),
            },
        })
        .collect()
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn ready_parameter_row_is_persisted_only_after_editing() {
        let mut h = Harness::builder().with_size([850., 550.]).build_ui_state(
            |ui, rows: &mut Vec<KeyValue>| {
                kv_table(ui, rows, false, "request.params");
            },
            Vec::<KeyValue>::new(),
        );
        h.run();
        assert!(h.state().is_empty());
        h.get_by_role_and_label(
            egui::accesskit::Role::TextInput,
            "Chave request.params linha 1",
        )
        .click();
        h.run();
        h.event(egui::Event::Paste("page".into()));
        h.run();
        assert_eq!(h.state().len(), 1);
        assert_eq!(h.state()[0].key, "page");
        assert!(h.query_by_label("Chave request.params linha 2").is_some());
        h.run();
        assert_eq!(h.state().len(), 1);
    }

    #[test]
    fn binary_file_path_is_an_editable_reference_before_explicit_send() {
        let mut h = Harness::builder().with_size([850., 550.]).build_ui_state(
            |ui, body: &mut RequestBody| {
                body_form(ui, body);
            },
            RequestBody::None,
        );
        h.get_by_label("binary").click();
        h.run();
        h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Arquivo do body binário")
            .click();
        h.event(egui::Event::Paste(
            "fixtures/uploads/arquivo-ainda-inexistente.bin".into(),
        ));
        h.run();
        assert_eq!(
            h.state(),
            &RequestBody::BinaryFile {
                path: PathBuf::from("fixtures/uploads/arquivo-ainda-inexistente.bin")
            }
        );
    }

    #[test]
    fn multipart_can_select_file_and_edit_reference_without_reading_it() {
        let mut h = Harness::builder().with_size([1000., 550.]).build_ui_state(
            |ui, body: &mut RequestBody| {
                body_form(ui, body);
            },
            RequestBody::FormData(vec![KeyValue::new("upload", "")]),
        );
        h.get_by_label("Tipo do campo multipart 1").click();
        h.run();
        h.get_by_label("File").click();
        h.run();
        h.get_by_label("Arquivo multipart upload").click();
        h.event(egui::Event::Paste("fixtures/uploads/ação.png".into()));
        h.run();
        let RequestBody::FormData(rows) = h.state() else {
            panic!("multipart body changed kind");
        };
        assert_eq!(
            rows[0].file_path,
            Some(PathBuf::from("fixtures/uploads/ação.png"))
        );
        assert!(rows[0].value.is_empty());
        h.get_by_label("Tipo do campo multipart 1").click();
        h.run();
        h.get_by_label("Text").click();
        h.run();
        let RequestBody::FormData(rows) = h.state() else {
            unreachable!()
        };
        assert!(rows[0].file_path.is_none());
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn settle(h: &mut Harness<'_, CodisseuApp>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            h.step();
            if !h.state().service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        h.run();
        assert!(h.state().service.last_error.is_none());
    }

    #[test]
    fn neighboring_workspace_files_clear_http_tabs_imports_filters_and_late_file_results() {
        let dir = tempfile::tempdir().unwrap();
        let carrier = dir.path().join("carrier");
        let roots = [dir.path().join("shipping"), dir.path().join("game")];
        for root in [&carrier, &roots[0], &roots[1]] {
            std::fs::create_dir(root).unwrap();
        }
        let projects = [
            carrier.join("shipping.code-workspace"),
            carrier.join("game.code-workspace"),
        ];
        for (project, root) in projects.iter().zip(&roots) {
            std::fs::write(
                project,
                format!(
                    r#"{{"folders":[{{"path":{}}}]}}"#,
                    serde_json::to_string(root).unwrap()
                ),
            )
            .unwrap();
        }
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        app.service
            .dispatch(Command::OpenWorkspace(projects[0].clone()))
            .unwrap();
        let mut h = Harness::builder().with_size([850., 700.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                app.service.poll();
                app.api_tick(ui.ctx());
                app.api_panel(ui);
            },
            app,
        );
        settle(&mut h);
        let collection = h
            .state_mut()
            .service
            .api
            .new_collection("Shipping endpoints");
        h.state_mut()
            .api_ui
            .open_tab(ApiTab::Collection(collection));
        h.state_mut().api_ui.filter = "shipping".into();
        h.state_mut().api_ui.notice = Some("Shipping-only notice".into());
        h.state_mut().api_ui.run_data = "[{\"company\":\"shipping\"}]".into();
        h.state_mut().api_ui.transfer = Some(Transfer {
            text: "Shipping import".into(),
            path: String::new(),
            export: false,
            kind: 0,
        });
        let (tx, rx) = mpsc::channel();
        h.state_mut().api_ui.file_rx = Some(rx);
        h.run();
        assert!(h.query_by_label("Shipping endpoints").is_some());

        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(projects[1].clone()))
            .unwrap();
        settle(&mut h);
        assert_eq!(h.state().service.workspace.as_ref(), Some(&carrier));
        assert_eq!(h.state().api_ui.project.as_ref(), Some(&projects[1]));
        assert!(h.state().api_ui.tabs.is_empty());
        assert!(h.state().api_ui.active.is_none());
        assert!(h.state().api_ui.transfer.is_none());
        assert!(h.state().api_ui.file_rx.is_none());
        assert!(h.state().api_ui.filter.is_empty());
        assert!(h.state().api_ui.notice.is_none());
        assert!(h.state().api_ui.run_data.is_empty());
        assert!(tx.send(Ok("late Shipping import".into())).is_err());
        assert!(h.query_by_label("Shipping endpoints").is_none());
        assert!(
            h.query_by_label("Nenhuma collection neste projeto")
                .is_some()
        );

        h.state_mut()
            .service
            .dispatch(Command::OpenWorkspace(projects[0].clone()))
            .unwrap();
        settle(&mut h);
        assert!(h.query_by_label("Shipping endpoints").is_some());
        assert!(h.state().api_ui.active.is_none());
        assert!(h.state().api_ui.transfer.is_none());
    }
}
