//! Datasources and result editing over the shared application database worker.
use super::*;
use codisseu_core::remote::{
    self, Cell, CellEdit, ConnectionConfig, EditBatch, Engine, MongoAuthentication,
    MongoReadPreference, RemoteCommand, TableRef,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct DatabaseUi {
    project_scope: Option<PathBuf>,
    project_scope_initialized: bool,
    pub dialog: bool,
    pub sqlite: bool,
    pub remote_results: bool,
    pub(super) result_source: Option<PathBuf>,
    pub(super) output: bool,
    pub(super) result_sql: Option<String>,
    pub(super) query_ack_revision: Option<u64>,
    pub(super) pending_sql: Option<String>,
    pending_console_source: Option<String>,
    pub(super) sqlite_result_sql: Option<String>,
    pub(super) sqlite_pending_sql: Option<String>,
    pub(super) sqlite_result_source: Option<PathBuf>,
    pub(super) sqlite_pending_source: Option<PathBuf>,
    pub(super) remote_page: Option<codisseu_core::query_page::QueryPageRequest>,
    pub(super) remote_pending_page: Option<codisseu_core::query_page::QueryPageRequest>,
    pub(super) sqlite_page: Option<codisseu_core::query_page::QueryPageRequest>,
    pub(super) sqlite_pending_page: Option<codisseu_core::query_page::QueryPageRequest>,
    pub(super) sqlite_query_ack_revision: Option<u64>,
    pub(super) remote_page_starts: Vec<u64>,
    pub(super) sqlite_page_starts: Vec<u64>,
    pub(super) pending_result_source: Option<Option<PathBuf>>,
    sqlite_revision: Option<u64>,
    config: ConnectionConfig,
    password: String,
    options: bool,
    ssl: bool,
    pub preview: Option<String>,
    preview_anchor: Option<egui::Pos2>,
    pub edit: Option<(usize, usize, String, bool)>,
    pub(super) edit_focus: bool,
    pub(super) edit_error: Option<String>,
    pub(super) refresh_after_edit: bool,
    pub(super) sqlite_enable_query: Option<String>,
    pub(super) sqlite_enable_source: Option<PathBuf>,
    pub(super) edit_notice: Option<String>,
    pub(super) editing_remote: Option<bool>,
    pub(super) edit_reload_sql: Option<String>,
    pub(super) edit_reload_source: Option<PathBuf>,
    pub(super) edit_reload_remote: bool,
    pub(super) edit_reload_page: Option<codisseu_core::query_page::QueryPageRequest>,
    pub(super) edit_binding: Option<String>,
    pub(super) staged: BTreeMap<(usize, usize), Cell>,
    revision: u64,
    pub(super) selected: Option<(usize, usize)>,
    selected_source: Option<String>,
    expanded_namespaces: BTreeSet<String>,
    catalog_filters: BTreeMap<String, String>,
    requested_namespaces: BTreeSet<String>,
    expanded_tables: BTreeSet<(String, String)>,
    requested_tables: BTreeSet<(String, String)>,
    pub(super) table: Option<TableRef>,
    pub(super) page_offset: u64,
    session_identity: Option<String>,
    namespaces_requested: bool,
    #[cfg(test)]
    catalog_commands: usize,
    pub exit_confirm: bool,
    exit_wait: bool,
    widths: Vec<f32>,
}
impl DatabaseUi {
    fn select(&mut self, c: ConnectionConfig) {
        self.selected_source = Some(c.id.clone());
        self.sqlite = false;
        self.config = c;
        self.password.clear();
        self.options = false;
        self.ssl = false;
    }
    fn add(&mut self, engine: Engine) {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        self.select(ConnectionConfig {
            id: id.to_string(),
            engine,
            name: format!("{} @ localhost", engine.label()),
            port: engine.port(),
            tls: engine != Engine::Memcached,
            ..ConnectionConfig::default()
        });
    }
}
fn engine_icon(ui: &mut egui::Ui, engine: Engine) {
    static TOKENS: std::sync::OnceLock<Tokens> = std::sync::OnceLock::new();
    // Engine brand colors are intentionally shared by both app themes.
    let tokens = TOKENS.get_or_init(|| Tokens::for_theme(codisseu_core::ThemePreference::Classic));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18., 20.), egui::Sense::hover());
    let tint = match engine {
        Engine::Mysql | Engine::AuroraMysql => color(tokens.database_engine_mysql),
        Engine::Postgres | Engine::AuroraPostgres => color(tokens.database_engine_postgres),
        Engine::Mongo => color(tokens.database_engine_mongo),
        Engine::Redis => color(tokens.database_engine_redis),
        Engine::Memcached => color(tokens.database_engine_memcached),
    };
    components::paint_database_engine(ui, rect, Some(engine), tint);
}
// Reference toolbar footprint: 26px spacing, 22px hover tile. Icons are original vectors.
fn db_icon(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    components::database_icon_button(ui, icon, label, enabled)
}
impl CodisseuApp {
    pub(super) fn database_exit_guard(&mut self) -> bool {
        if self.database_ui.exit_confirm || self.database_ui.exit_wait {
            return true;
        }
        if self.service.remote_db.is_busy() {
            if !self.database_ui.refresh_after_edit {
                self.remote_send(RemoteCommand::Cancel);
            }
            self.database_ui.exit_wait = true;
            return true;
        }
        if self.service.db.is_busy() {
            self.database_ui.exit_wait = true;
            return true;
        }
        if self.service.remote_db.transaction
            || self.service.db.transaction
            || !self.database_ui.staged.is_empty()
            || self.database_ui.edit.is_some()
        {
            self.database_ui.exit_confirm = true;
            return true;
        }
        false
    }
    pub(super) fn sync_database_ui(&mut self) {
        if self.database_ui.edit.is_none() && self.database_ui.staged.is_empty() {
            self.database_ui.editing_remote = None;
            self.database_ui.edit_binding = None;
        } else if let Some(remote) = self.database_ui.editing_remote {
            // Proposed row indices belong to exactly one result/session.
            self.database_ui.remote_results = remote;
        }
        let project_scope = self.service.workspace_path().map(PathBuf::from);
        if !self.database_ui.project_scope_initialized {
            // A caller may have prepared current-project startup state before
            // the first frame. Only an accepted transition invalidates it.
            self.database_ui.project_scope = project_scope.clone();
            self.database_ui.project_scope_initialized = true;
        } else if self.database_ui.project_scope != project_scope {
            self.database_ui = DatabaseUi {
                project_scope,
                project_scope_initialized: true,
                ..Default::default()
            };
            self.database_input.clear();
            self.query_library = Default::default();
            self.sql_productivity = Default::default();
            self.query_export_ui = Default::default();
            self.transfer_ui = Default::default();
            self.transfer_views = Default::default();
            self.grid_selection = Default::default();
        }
        let identity = self.service.remote_db.active.as_ref().map(|c| {
            format!(
                "{}:{}:{}:{}:{}",
                self.service.remote_db.connection_generation, c.id, c.host, c.port, c.database
            )
        });
        if identity != self.database_ui.session_identity {
            let connecting_query = self
                .database_ui
                .pending_console_source
                .as_ref()
                .is_some_and(|id| {
                    self.service
                        .remote_db
                        .active
                        .as_ref()
                        .is_some_and(|c| &c.id == id)
                });
            self.database_ui.session_identity = identity;
            self.database_ui.output = false;
            self.database_ui.result_source = None;
            self.database_ui.result_sql = None;
            self.database_ui.query_ack_revision = None;
            if !connecting_query {
                self.database_ui.pending_sql = None;
                self.database_ui.pending_result_source = None;
                self.database_ui.pending_console_source = None;
            }
            self.database_ui.remote_page = None;
            self.database_ui.remote_page_starts.clear();
            self.database_ui.remote_pending_page = None;
            self.database_ui.expanded_namespaces.clear();
            self.database_ui.expanded_tables.clear();
            self.database_ui.requested_namespaces.clear();
            self.database_ui.requested_tables.clear();
            self.database_ui.namespaces_requested = false;
            if self.service.remote_db.active.is_some() {
                self.database_ui.sqlite = false;
            }
            self.database_ui.catalog_filters.clear();
            self.database_ui.table = None;
            self.database_ui.page_offset = 0;
            self.database_ui.widths.clear();
            self.database_ui.selected = None;
            self.database_ui.preview = None;
            if self.database_ui.editing_remote != Some(false) {
                self.database_ui.edit = None;
            }
        }
        // Catalog lifetime belongs to the session, independently of panel visibility.
        // A loaded snapshot can also have come from SQL completion while hidden.
        if !self.service.remote_db.namespaces_loaded
            && !self.database_ui.namespaces_requested
            && !self.service.remote_db.is_busy()
            && self
                .service
                .remote_db
                .active
                .as_ref()
                .is_some_and(|c| c.engine.is_sql())
        {
            self.database_ui.namespaces_requested = true;
            self.remote_send(RemoteCommand::ListNamespaces);
        }
        if self.database_ui.revision != self.service.remote_db.revision {
            self.database_ui.revision = self.service.remote_db.revision;
            if let Some(page) = self.database_ui.remote_pending_page.take() {
                self.database_ui.accept_query_page(page, true);
            }
            if let Some(source) = self.database_ui.pending_result_source.take() {
                self.database_ui.pending_console_source = None;
                self.database_ui.result_source = source;
                self.database_ui.result_sql = self.database_ui.pending_sql.take();
                self.database_ui.query_ack_revision = self
                    .database_ui
                    .result_sql
                    .as_ref()
                    .map(|_| self.service.remote_db.revision);
                self.database_ui.remote_page = self
                    .database_ui
                    .result_sql
                    .as_ref()
                    .filter(|_| {
                        self.service
                            .remote_db
                            .active
                            .as_ref()
                            .is_some_and(|c| c.engine.is_sql())
                    })
                    .filter(|sql| {
                        self.service
                            .remote_db
                            .active
                            .as_ref()
                            .is_some_and(|c| remote::supports_query_pages(sql, c.engine))
                    })
                    .map(|sql| codisseu_core::query_page::QueryPageRequest::first(sql.clone()));
                self.database_ui.remote_page_starts.clear();
            }
            if self.database_ui.editing_remote != Some(false)
                && (self.database_ui.edit_binding.is_none() || self.database_ui.refresh_after_edit)
            {
                self.database_ui.staged.clear();
            }
            self.database_ui.selected = None;
            self.database_ui.widths.clear();
            if self.database_ui.editing_remote != Some(false)
                && (self.database_ui.edit_binding.is_none() || self.database_ui.refresh_after_edit)
            {
                self.database_ui.edit = None;
            }
            self.database_ui.preview = None;
        }
        if !self.service.remote_db.is_busy() && self.service.remote_db.error.is_some() {
            self.database_ui.pending_result_source = None;
            self.database_ui.pending_sql = None;
            self.database_ui.pending_console_source = None;
            self.database_ui.remote_pending_page = None;
        }
        let sqlite_revision = self.service.db.result_revision();
        if sqlite_revision != self.database_ui.sqlite_revision {
            self.database_ui.sqlite_revision = sqlite_revision;
            if sqlite_revision.is_some() && self.database_ui.sqlite_pending_page.is_some() {
                let page = self.database_ui.sqlite_pending_page.take().unwrap();
                self.database_ui.accept_query_page(page, false);
            } else if self.database_ui.sqlite_pending_page.is_none() {
                self.database_ui.sqlite_result_sql = if sqlite_revision.is_some() {
                    self.database_ui.sqlite_pending_sql.take()
                } else {
                    None
                };
                self.database_ui.sqlite_result_source = if sqlite_revision.is_some() {
                    self.database_ui.sqlite_pending_source.take()
                } else {
                    None
                };
                self.database_ui.sqlite_query_ack_revision = sqlite_revision;
                self.database_ui.sqlite_page = self
                    .database_ui
                    .sqlite_result_sql
                    .as_ref()
                    .filter(|sql| codisseu_core::query_page::is_select(sql))
                    .map(|sql| codisseu_core::query_page::QueryPageRequest::first(sql.clone()));
                self.database_ui.sqlite_page_starts.clear();
            }
        }
        if !self.service.db.is_busy() && self.service.db.last_error.is_some() {
            self.database_ui.sqlite_pending_source = None;
            self.database_ui.sqlite_pending_sql = None;
            self.database_ui.sqlite_pending_page = None;
        }
        self.grid_edit_tick();
        if self.database_ui.exit_wait
            && !self.service.remote_db.is_busy()
            && !self.service.db.is_busy()
        {
            self.database_ui.exit_wait = false;
            if self.service.remote_db.error.is_none() && self.service.db.last_error.is_none() {
                self.request_exit();
            } else {
                self.database_ui.exit_confirm = true;
            }
        }
    }
    pub(super) fn remote_send(&mut self, command: RemoteCommand) {
        #[cfg(test)]
        if matches!(
            command,
            RemoteCommand::ListNamespaces
                | RemoteCommand::CatalogPage(_)
                | RemoteCommand::InspectColumns(_)
        ) {
            self.database_ui.catalog_commands += 1;
        }
        if (!self.database_ui.staged.is_empty() || self.database_ui.edit.is_some())
            && matches!(
                command,
                RemoteCommand::Connect { .. }
                    | RemoteCommand::ConnectRemembered { .. }
                    | RemoteCommand::Execute(_)
                    | RemoteCommand::Explain(_)
                    | RemoteCommand::ExecutePage(_)
                    | RemoteCommand::ExecuteConsole { .. }
                    | RemoteCommand::ExecuteBound { .. }
                    | RemoteCommand::LoadTable(_)
                    | RemoteCommand::LoadTablePage { .. }
                    | RemoteCommand::Disconnect
                    | RemoteCommand::Delete(_)
                    | RemoteCommand::DeleteRemembered(_)
                    | RemoteCommand::Commit
                    | RemoteCommand::Rollback
            )
        {
            self.service.remote_db.error =
                Some("Envie ou descarte as alterações locais antes desta ação".into());
            return;
        }
        let table = match &command {
            RemoteCommand::LoadTable(t) | RemoteCommand::LoadTablePage { table: t, .. } => {
                Some(t.clone())
            }
            _ => None,
        };
        let reset_page = matches!(command, RemoteCommand::LoadTable(_));
        if let Err(e) = self.service.dispatch(Command::RemoteDatabase(command)) {
            self.service.remote_db.error = Some(e);
        } else if let Some(table) = table {
            self.database_ui.pending_result_source = Some(None);
            self.database_ui.pending_sql = None;
            self.database_ui.table = Some(table);
            if reset_page {
                self.database_ui.page_offset = 0;
            }
        }
    }
    fn open_datasources(&mut self) {
        let target = self
            .database_ui
            .selected_source
            .as_ref()
            .and_then(|id| self.service.remote_db.profiles.iter().find(|c| &c.id == id))
            .or(self.service.remote_db.active.as_ref())
            .or_else(|| self.service.remote_db.profiles.first())
            .cloned();
        if let Some(config) = target {
            if self.database_ui.config.id != config.id || self.database_ui.config.id.is_empty() {
                self.database_ui.select(config);
            }
        } else if self.database_ui.config.id.is_empty() {
            self.database_ui.add(Engine::Mysql);
        }
        self.database_ui.dialog = true;
    }
    fn refresh_remote_catalog(&mut self) {
        if self.service.remote_db.is_busy() {
            return;
        }
        let sql = self
            .service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|c| c.engine.is_sql());
        self.service.remote_db.clear_catalog();
        self.database_ui.requested_namespaces.clear();
        self.database_ui.requested_tables.clear();
        self.database_ui.namespaces_requested = sql;
        self.remote_send(if sql {
            RemoteCommand::ListNamespaces
        } else {
            RemoteCommand::Inspect
        });
    }
    fn connect_datasource(&mut self, profile: ConnectionConfig) {
        let active = self
            .service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|c| c.id == profile.id);
        self.database_ui.select(profile.clone());
        if !active {
            self.remote_send(RemoteCommand::ConnectRemembered {
                config: profile,
                password: String::new(),
                test: false,
            });
        }
    }
    fn datasource_add_menu(&mut self, ui: &mut egui::Ui) {
        let response = db_icon(ui, Icon::Add, "Adicionar datasource", true);
        egui::Popup::menu(&response)
            .frame(egui::Frame::popup(ui.style()).fill(color(self.tokens.bar)))
            .show(|ui| {
                ui.set_width(260.);
                ui.spacing_mut().item_spacing.y = 0.;
                ui.add_sized(
                    [260., 24.],
                    egui::Label::new(
                        RichText::new("Data Sources")
                            .small()
                            .color(color(self.tokens.muted)),
                    ),
                );
                for engine in Engine::ALL {
                    if database_menu_item(ui, engine.label(), Some(engine), &self.tokens).clicked()
                    {
                        self.database_ui.add(engine);
                        self.database_ui.dialog = true;
                        ui.close();
                    }
                }
                ui.separator();
                if database_menu_item(ui, "SQLite (arquivo)", None, &self.tokens).clicked() {
                    self.database_ui.sqlite = true;
                    self.database_ui.remote_results = false;
                    ui.close();
                }
            });
    }
    pub(super) fn remote_database_panel(&mut self, ui: &mut egui::Ui) {
        let busy = self.service.remote_db.is_busy();
        let connected = self.service.remote_db.active.is_some();
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            ui.label("Database");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(8.);
                if db_icon(ui, Icon::Settings, "Data Sources and Drivers", true).clicked() {
                    self.open_datasources();
                }
                self.datasource_add_menu(ui);
                if db_icon(ui, Icon::Refresh, "Atualizar objetos", !busy && connected).clicked() {
                    self.refresh_remote_catalog();
                }
                if db_icon(
                    ui,
                    Icon::Disconnect,
                    "Desconectar datasource",
                    !busy && connected,
                )
                .clicked()
                {
                    self.remote_send(RemoteCommand::Disconnect);
                }
            });
        });
        let profiles = self.service.remote_db.profiles.clone();
        egui::ScrollArea::both()
            .id_salt("database.explorer.scroll")
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                for profile in profiles {
                    let active = self
                        .service
                        .remote_db
                        .active
                        .as_ref()
                        .is_some_and(|c| c.id == profile.id);
                    let selected = self.database_ui.selected_source.as_ref() == Some(&profile.id);
                    let response = ui
                        .push_id(("database.source", &profile.id), |ui| {
                            datasource_row(
                                ui,
                                &profile,
                                selected || active,
                                active,
                                25.,
                                &self.tokens,
                            )
                        })
                        .inner;
                    if response.clicked() || response.secondary_clicked() {
                        self.database_ui.select(profile.clone());
                    }
                    let popup = egui::Popup::default_response_id(&response);
                    // The first right click opens a popup over this row. Its foreground
                    // layer can receive the second click, so keep the gesture tied to
                    // the originating row and its open menu instead of its Response alone.
                    let right_double = response.double_clicked_by(egui::PointerButton::Secondary)
                        || (egui::Popup::is_id_open(ui.ctx(), popup)
                            && ui.input(|i| {
                                i.pointer
                                    .button_double_clicked(egui::PointerButton::Secondary)
                                    && i.pointer
                                        .interact_pos()
                                        .is_some_and(|p| response.rect.contains(p))
                            }));
                    if response.double_clicked() || right_double {
                        egui::Popup::close_id(ui.ctx(), popup);
                        self.connect_datasource(profile.clone());
                    } else {
                        response.context_menu(|ui| {
                            if ui
                                .add_enabled(
                                    !active
                                        && !busy
                                        && !self.service.remote_db.transaction
                                        && self.database_ui.staged.is_empty(),
                                    egui::Button::new("Conectar"),
                                )
                                .clicked()
                            {
                                self.connect_datasource(profile.clone());
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    active && !busy && self.database_ui.staged.is_empty(),
                                    egui::Button::new("Desconectar"),
                                )
                                .clicked()
                            {
                                self.remote_send(RemoteCommand::Disconnect);
                                ui.close();
                            }
                            if ui.button("Propriedades…").clicked() {
                                self.database_ui.select(profile.clone());
                                self.database_ui.dialog = true;
                                ui.close();
                            }
                            if ui
                                .add_enabled(!busy, egui::Button::new("Nova query"))
                                .clicked()
                            {
                                self.remote_send(RemoteCommand::CreateConsoleFor(
                                    profile.id.clone(),
                                ));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    !busy
                                        && (!active
                                            || (!self.service.remote_db.transaction
                                                && self.database_ui.staged.is_empty())),
                                    egui::Button::new("Excluir…"),
                                )
                                .clicked()
                            {
                                self.delete_datasource_dialog(profile.clone());
                                ui.close();
                            }
                        });
                    }
                    if active {
                        ui.horizontal(|ui| {
                            ui.add_space(22.);
                            ui.spacing_mut().item_spacing.x = 0.;
                            if db_icon(ui, Icon::Code, "Console", !busy).clicked() {
                                self.database_ui.remote_results = true;
                                self.remote_send(RemoteCommand::CreateConsole);
                            }
                            if db_icon(ui, Icon::Refresh, "Carregar objetos", !busy).clicked() {
                                self.refresh_remote_catalog();
                            }
                        });
                        ui.scope(|ui| {
                            if profile.engine.is_sql() {
                                self.sql_catalog_tree(ui, &profile.id);
                            } else {
                                for object in self.service.remote_db.objects.clone() {
                                    let row = components::database_object_row(
                                        ui,
                                        egui::Id::unique((
                                            "nosql.object",
                                            &profile.id,
                                            &object.namespace,
                                            &object.name,
                                        )),
                                        components::TreeObjectRow {
                                            name: &object.name,
                                            detail: &object.kind,
                                            description: "Objeto remoto",
                                            depth: 1,
                                            expanded: None,
                                            selected: false,
                                            icon: Icon::Table,
                                        },
                                        color(self.tokens.database_grid_foreground),
                                        color(self.tokens.database_completion_key),
                                    );
                                    row.on_hover_text(&object.namespace);
                                }
                            }
                        });
                    }
                }
                self.saved_queries_tree(ui);
                if self.service.remote_db.profiles.is_empty()
                    && !self.database_ui.sqlite
                    && self.service.remote_db.console_bindings.is_empty()
                {
                    egui::ScrollArea::vertical()
                        .id_salt("database.empty.scroll")
                        .auto_shrink([false, false])
                        .codisseu()
                        .show(ui, |ui| self.database_empty_state(ui));
                }
                ui.add_space(8.);
                let status = self
                    .service
                    .remote_db
                    .error
                    .as_deref()
                    .unwrap_or(&self.service.remote_db.status);
                if self.service.remote_db.error.is_some() {
                    crate::status_messages::status_message(ui, status).on_hover_text(status);
                }
                if self.service.remote_db.transaction {
                    crate::status_messages::status_message(
                        ui,
                        "Transação aberta · Commit / Rollback",
                    );
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner().on_hover_text("Carregando banco de dados");
                        if db_icon(ui, Icon::Stop, "Cancelar operação", true).clicked() {
                            self.remote_send(RemoteCommand::Cancel);
                        }
                    });
                }
                self.sqlite_database_panel(ui);
            });
    }
    fn database_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.add_space(18.);
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(12, 0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.;
                ui.label(
                    RichText::new("Create Data Source")
                        .color(color(self.tokens.muted))
                        .size(12.),
                );
                ui.add_space(2.);
                for (label, engine) in [("PostgreSQL", Engine::Postgres), ("MySQL", Engine::Mysql)]
                {
                    if database_empty_card(ui, label, Some(engine), false, &self.tokens).clicked() {
                        self.database_ui.add(engine);
                        self.database_ui.dialog = true;
                    }
                }
                if database_empty_card(ui, "SQLite", None, false, &self.tokens).clicked() {
                    self.database_ui.sqlite = true;
                    self.database_ui.remote_results = false;
                }
                let all = database_empty_card(ui, "All Data Sources…", None, true, &self.tokens);
                egui::Popup::menu(&all).show(|ui| {
                    ui.set_width(248.);
                    for engine in Engine::ALL {
                        if database_menu_item(ui, engine.label(), Some(engine), &self.tokens)
                            .clicked()
                        {
                            self.database_ui.add(engine);
                            self.database_ui.dialog = true;
                            ui.close();
                        }
                    }
                });
                ui.add_space(22.);
                ui.label(
                    RichText::new("NoSQL Data Sources")
                        .color(color(self.tokens.muted))
                        .size(12.),
                );
                ui.add_space(2.);
                for engine in [Engine::Mongo, Engine::Redis, Engine::Memcached] {
                    if database_empty_card(ui, engine.label(), Some(engine), false, &self.tokens)
                        .clicked()
                    {
                        self.database_ui.add(engine);
                        self.database_ui.dialog = true;
                    }
                }
            });
    }
    pub(super) fn catalog_filter_focused(&self, ctx: &egui::Context) -> bool {
        self.service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|source| {
                self.database_ui
                    .expanded_namespaces
                    .iter()
                    .any(|namespace| {
                        ctx.memory(|m| {
                            m.has_focus(egui::Id::unique((
                                "codisseu.catalog_filter",
                                source.id.as_str(),
                                namespace,
                            )))
                        })
                    })
            })
    }
    fn sql_catalog_tree(&mut self, ui: &mut egui::Ui, source: &str) {
        if !self.service.remote_db.namespaces_loaded {
            return;
        }
        for namespace in self.service.remote_db.namespaces.clone() {
            let open = self.database_ui.expanded_namespaces.contains(&namespace);
            let response = components::database_object_row(
                ui,
                egui::Id::unique(("db.namespace", source, &namespace)),
                components::TreeObjectRow {
                    name: &namespace,
                    detail: "",
                    description: "Schema/database · expandir para carregar tabelas",
                    depth: 1,
                    expanded: Some(open),
                    selected: false,
                    icon: Icon::Schema,
                },
                color(self.tokens.database_grid_foreground),
                color(self.tokens.database_completion_key),
            );
            if response.clicked() {
                response.request_focus();
            }
            let keyboard_open = response.has_focus()
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight));
            let keyboard_close = response.has_focus()
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
            if response.clicked() || keyboard_open || keyboard_close {
                if (open && !keyboard_open) || keyboard_close {
                    self.database_ui.expanded_namespaces.remove(&namespace);
                    self.database_ui.requested_namespaces.remove(&namespace);
                } else {
                    self.database_ui
                        .expanded_namespaces
                        .insert(namespace.clone());
                }
            }
            if !self.database_ui.expanded_namespaces.contains(&namespace) {
                continue;
            }
            let mut filter = self
                .database_ui
                .catalog_filters
                .get(&namespace)
                .cloned()
                .unwrap_or_default();
            let mut apply = false;
            ui.push_id(("catalog.filter", source, &namespace), |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(38.);
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut filter)
                            .id(egui::Id::unique((
                                "codisseu.catalog_filter",
                                source,
                                &namespace,
                            )))
                            .char_limit(256)
                            .hint_text("Nome da tabela…")
                            .margin(egui::vec2(4., 2.))
                            .desired_width(
                                (ui.available_width() - 26. - ui.spacing().item_spacing.x - 12.)
                                    .max(40.),
                            ),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            true,
                            format!("Filtrar tabelas de {namespace}"),
                        )
                    });
                    apply = (response.has_focus() || response.lost_focus())
                        && ui.input(|i| i.key_pressed(Key::Enter));
                    if apply {
                        ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                    }
                    apply |= db_icon(
                        ui,
                        Icon::Search,
                        &format!("Aplicar filtro de {namespace}"),
                        !self.service.remote_db.is_busy(),
                    )
                    .clicked();
                });
            });
            self.database_ui
                .catalog_filters
                .insert(namespace.clone(), filter.clone());
            if apply && !self.service.remote_db.is_busy() {
                self.database_ui
                    .requested_namespaces
                    .insert(namespace.clone());
                self.remote_send(RemoteCommand::CatalogPage(remote::CatalogRequest {
                    namespace: namespace.clone(),
                    prefix: filter.clone(),
                    ..Default::default()
                }));
            }
            if let Some(position) = self
                .service
                .remote_db
                .namespace_pages
                .get(&namespace)
                .cloned()
            {
                let busy = self.service.remote_db.is_busy();
                let mut request = None;
                if position.previous.is_some()
                    || position.next.is_some()
                    || position.request.after.is_some()
                    || position.request.before.is_some()
                {
                    ui.horizontal(|ui| {
                        ui.add_space(38.);
                        if db_icon(
                            ui,
                            Icon::Revert,
                            &format!("Primeira página de {namespace}"),
                            !busy
                                && (position.request.after.is_some()
                                    || position.request.before.is_some()),
                        )
                        .clicked()
                        {
                            request = Some(remote::CatalogRequest {
                                namespace: namespace.clone(),
                                prefix: position.request.prefix.clone(),
                                ..Default::default()
                            });
                        }
                        if db_icon(
                            ui,
                            Icon::Previous,
                            &format!("Tabelas anteriores de {namespace}"),
                            !busy && position.previous.is_some(),
                        )
                        .clicked()
                        {
                            request = Some(remote::CatalogRequest {
                                namespace: namespace.clone(),
                                prefix: position.request.prefix.clone(),
                                before: position.previous.clone(),
                                after: None,
                            });
                        }
                        if db_icon(
                            ui,
                            Icon::Next,
                            &format!("Próximas tabelas de {namespace}"),
                            !busy && position.next.is_some(),
                        )
                        .clicked()
                        {
                            request = Some(remote::CatalogRequest {
                                namespace: namespace.clone(),
                                prefix: position.request.prefix.clone(),
                                after: position.next.clone(),
                                before: None,
                            });
                        }
                    });
                }
                if let Some(request) = request {
                    self.remote_send(RemoteCommand::CatalogPage(request));
                }
            }
            let Some(objects) = self
                .service
                .remote_db
                .namespace_objects
                .get(&namespace)
                .cloned()
            else {
                if !self.service.remote_db.is_busy()
                    && !self.database_ui.requested_namespaces.contains(&namespace)
                {
                    self.database_ui
                        .requested_namespaces
                        .insert(namespace.clone());
                    self.remote_send(RemoteCommand::CatalogPage(remote::CatalogRequest {
                        namespace: namespace.clone(),
                        prefix: filter.clone(),
                        ..Default::default()
                    }));
                } else if !self.service.remote_db.is_busy() {
                    ui.horizontal(|ui| {
                        ui.add_space(38.);
                        ui.weak("Tabelas não carregadas");
                        if db_icon(
                            ui,
                            Icon::Refresh,
                            &format!("Recarregar tabelas de {namespace}"),
                            true,
                        )
                        .clicked()
                        {
                            self.database_ui.requested_namespaces.remove(&namespace);
                        }
                    });
                }
                continue;
            };
            if objects.is_empty() {
                ui.indent(("db.empty", &namespace), |ui| {
                    ui.weak("Nenhuma tabela acessível");
                });
            }
            for object in objects {
                let key = (namespace.clone(), object.name.clone());
                let open = self.database_ui.expanded_tables.contains(&key);
                let response = components::database_object_row(
                    ui,
                    egui::Id::unique(("db.table", source, &key)),
                    components::TreeObjectRow {
                        name: &object.name,
                        detail: if object.kind == "VIEW" { "view" } else { "" },
                        description: "Tabela · expandir colunas; duplo clique abre dados",
                        depth: 2,
                        expanded: Some(open),
                        selected: self
                            .database_ui
                            .table
                            .as_ref()
                            .is_some_and(|t| t.namespace == namespace && t.name == object.name),
                        icon: Icon::Table,
                    },
                    color(self.tokens.database_grid_foreground),
                    color(self.tokens.database_completion_key),
                );
                if response.clicked() {
                    response.request_focus();
                }
                let table = TableRef {
                    namespace: namespace.clone(),
                    name: object.name.clone(),
                };
                if response.double_clicked() && !self.service.remote_db.is_busy() {
                    self.database_ui.remote_results = true;
                    self.show_activity = true;
                    self.tool_tab = ToolTab::Results;
                    self.remote_send(RemoteCommand::LoadTable(table.clone()));
                } else {
                    let keyboard_open = response.has_focus()
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight));
                    let keyboard_close = response.has_focus()
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowLeft));
                    let arrow_clicked = response.clicked()
                        && response
                            .interact_pointer_pos()
                            .is_some_and(|p| p.x < response.rect.left() + 58.);
                    if arrow_clicked || keyboard_open || keyboard_close {
                        if (open && !keyboard_open) || keyboard_close {
                            self.database_ui.expanded_tables.remove(&key);
                            self.database_ui.requested_tables.remove(&key);
                        } else {
                            self.database_ui.expanded_tables.insert(key.clone());
                        }
                    }
                }
                if !self.database_ui.expanded_tables.contains(&key) {
                    continue;
                }
                let Some(columns) = self.service.remote_db.table_columns.get(&key).cloned() else {
                    if !self.service.remote_db.is_busy()
                        && !self.database_ui.requested_tables.contains(&key)
                    {
                        self.database_ui.requested_tables.insert(key.clone());
                        self.remote_send(RemoteCommand::InspectColumns(table));
                    } else if !self.service.remote_db.is_busy() {
                        ui.horizontal(|ui| {
                            ui.add_space(54.);
                            ui.weak("Colunas não carregadas");
                            if db_icon(
                                ui,
                                Icon::Refresh,
                                &format!("Recarregar colunas de {}.{}", key.0, key.1),
                                true,
                            )
                            .clicked()
                            {
                                self.database_ui.requested_tables.remove(&key);
                            }
                        });
                    }
                    continue;
                };
                for column in columns {
                    components::database_object_row(
                        ui,
                        egui::Id::unique(("db.column", source, &key, &column.name)),
                        components::TreeObjectRow {
                            name: &column.name,
                            detail: &column.data_type,
                            description: if column.primary_key {
                                "Coluna · chave primária"
                            } else {
                                "Coluna"
                            },
                            depth: 3,
                            expanded: None,
                            selected: false,
                            icon: if column.primary_key {
                                Icon::PrimaryKey
                            } else {
                                Icon::Column
                            },
                        },
                        color(self.tokens.database_grid_foreground),
                        color(self.tokens.database_completion_key),
                    );
                }
            }
        }
    }
    pub(super) fn datasource_window(&mut self, ctx: &egui::Context) {
        if !self.database_ui.dialog {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.database_ui.dialog = false;
            self.focus_editor = true;
            return;
        }
        let mut close = false;
        let mut save = false;
        let mut connect = false;
        let mut test = false;
        let mut delete = false;
        let width = (self.tokens.database_dialog_width - 2.)
            .min((ctx.content_rect().width() - 32.).max(300.));
        let height = self
            .tokens
            .database_dialog_height
            .min((ctx.content_rect().height() - 32.).max(300.));
        let tree_width = (self.tokens.database_dialog_tree_width - 1.).min(width * 0.32);
        let body_height = height - 51.;
        let frame = egui::Frame::new()
            .fill(color(self.tokens.panel))
            .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
            .corner_radius(self.tokens.control_corner_radius);
        egui::Modal::new(egui::Id::unique("codisseu.datasources")).frame(frame).show(ctx, |ui| {
            ui.set_width(width);
            self.tokens.form_style(ui);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(tree_width, body_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_size(egui::vec2(tree_width, body_height));
                    let origin=ui.cursor().min;
                    let rail=egui::Rect::from_min_size(origin,egui::vec2(45.,body_height));
                    ui.painter().line_segment([rail.right_top(),rail.right_bottom()],egui::Stroke::new(1.,color(self.tokens.border)));
                    components::paint_database_icon(ui,egui::Rect::from_center_size(origin+egui::vec2(22.,16.),egui::vec2(16.,16.)),Icon::Database,color(self.tokens.database_grid_foreground));
                    egui::Frame::new().inner_margin(egui::Margin {left:53,right:8,top:8,bottom:8}).show(ui, |ui| {
                        ui.set_width(tree_width - 61.);
                        ui.horizontal(|ui|{ui.set_min_height(30.);ui.add_space(4.);ui.label(RichText::new("Data Sources").strong());});
                        ui.horizontal(|ui| {
                            self.datasource_add_menu(ui);
                            delete = db_icon(ui, Icon::Remove, "Remover cadastro selecionado", !self.service.remote_db.is_busy() && self.service.remote_db.profiles.iter().any(|c| c.id == self.database_ui.config.id)).clicked();
                        });
                        ui.add_space(8.);
                        ui.horizontal(|ui|{ui.set_min_height(22.);ui.label(RichText::new("Project Data Sources").color(color(self.tokens.muted)));});
                        egui::ScrollArea::vertical().id_salt("ds.profiles").max_height((body_height - 110.).max(80.)).codisseu().show(ui, |ui| {
                            for c in self.service.remote_db.profiles.clone() {
                                if datasource_row(ui, &c, c.id == self.database_ui.config.id, false, 14., &self.tokens).clicked() {
                                    self.database_ui.select(c);
                                }
                            }
                            if !self.service.remote_db.profiles.iter().any(|c| c.id == self.database_ui.config.id) {
                                datasource_row(ui, &self.database_ui.config, true, false, 14., &self.tokens);
                            }
                        });
                    });
                });
                let (divider, _) = ui.allocate_exact_size(egui::vec2(1., body_height), egui::Sense::hover());
                ui.painter().rect_filled(divider, 0., color(self.tokens.border));
                ui.allocate_ui_with_layout(egui::vec2(width-tree_width-1., body_height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_height(body_height);
                    egui::Frame::new().inner_margin(egui::Margin{left:12,right:12,top:12,bottom:3}).show(ui, |ui| {
                        ui.set_width(width-tree_width-25.);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.;
                            let label = ui.add_sized([45., self.tokens.control_height], egui::Label::new("Name:"));
                            let field_width = (ui.available_width()-42.).max(40.);
                            ui.add_sized([field_width,self.tokens.control_height], egui::TextEdit::singleline(&mut self.database_ui.config.name).id_salt("ds.name").margin(egui::vec2(8.,5.))).labelled_by(label.id);
                            close |= db_icon(ui, Icon::Close, "Fechar datasources", true).clicked();
                        });
                        ui.add_space(10.);
                        ui.horizontal(|ui| {
                            let general = !self.database_ui.options && !self.database_ui.ssl;
                            if database_tab(ui,"General",general,&self.tokens).clicked() { self.database_ui.options=false; self.database_ui.ssl=false; }
                            if database_tab(ui,"Options",self.database_ui.options,&self.tokens).clicked() { self.database_ui.options=true; self.database_ui.ssl=false; }
                            if database_tab(ui,"SSL",self.database_ui.ssl,&self.tokens).clicked() { self.database_ui.ssl=true; self.database_ui.options=false; }
                        });
                        ui.add_space(20.);
                        let form_height = (body_height-129.).max(100.);
                        egui::ScrollArea::vertical().id_salt("ds.body.scroll").max_height(form_height).min_scrolled_height(form_height).auto_shrink([false,false]).codisseu().show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = self.tokens.database_form_row_gap;
                            let label_width = if self.database_ui.options {self.tokens.database_options_label_width} else {self.tokens.database_form_label_width};
                            let c = &mut self.database_ui.config;
                            if self.database_ui.ssl {
                                db_form_row(ui, "TLS:", label_width, |ui, _| { ui.checkbox(&mut c.tls,"Verificar certificado e hostname"); });
                                db_form_row(ui, "CA certificate:", label_width, |ui, label| {
                                    let hint = if !c.ca_pem.is_empty() { "Certificado salvo no cadastro" } else if c.automatic_rds_ca() { if c.engine == Engine::Mongo { "Automático · AWS DocumentDB" } else { "Automático · AWS RDS" } } else { "Automático · certificados públicos/sistema" };
                                    let has_certificate=!c.ca_file.is_empty() || !c.ca_pem.is_empty();
                                    let width=(ui.available_width()-if has_certificate {118.} else {3.}).max(60.);
                                    ui.add(egui::TextEdit::singleline(&mut c.ca_file).id_salt("ds.ca_file").hint_text(hint).desired_width(width).margin(egui::vec2(8.,5.))).labelled_by(label).on_hover_text("Automático: certificados públicos/do sistema e CA oficial AWS quando aplicável. PEM escolhido é guardado no cadastro ao aplicar, sem depender do caminho original. Certificado e hostname sempre verificados.");
                                    if has_certificate && ui.button("Automático").clicked() {c.ca_file.clear();c.ca_pem=Default::default();}
                                });
                            } else if self.database_ui.options {
                                if c.engine == Engine::Mongo {
                                    db_form_row(ui, "Authentication:", label_width, |ui, label| {
                                        egui::ComboBox::from_id_salt("ds.mongo.authentication").selected_text(c.mongo.authentication.label()).show_ui(ui, |ui| {
                                            for value in MongoAuthentication::ALL { ui.selectable_value(&mut c.mongo.authentication, value, value.label()); }
                                        }).response.labelled_by(label);
                                    });
                                    db_form_row(ui, "Replica set:", label_width, |ui, label| {
                                        ui.add(egui::TextEdit::singleline(&mut c.mongo.replica_set).id_salt("ds.mongo.replica_set").hint_text("Opcional").desired_width(ui.available_width()-3.).margin(egui::vec2(8.,5.))).labelled_by(label);
                                    });
                                    db_form_row(ui, "Connection mode:", label_width, |ui, label| {
                                        egui::ComboBox::from_id_salt("ds.mongo.mode").selected_text(if c.mongo.direct_connection { "Direta · um servidor" } else { "Replica set · descoberta" }).show_ui(ui, |ui| {
                                            ui.selectable_value(&mut c.mongo.direct_connection, true, "Direta · um servidor");
                                            ui.selectable_value(&mut c.mongo.direct_connection, false, "Replica set · descoberta");
                                        }).response.labelled_by(label);
                                    });
                                    db_form_row(ui, "Read preference:", label_width, |ui, label| {
                                        egui::ComboBox::from_id_salt("ds.mongo.preference").selected_text(c.mongo.read_preference.label()).show_ui(ui, |ui| {
                                            for value in MongoReadPreference::ALL { ui.selectable_value(&mut c.mongo.read_preference, value, value.label()); }
                                        }).response.labelled_by(label);
                                    });
                                }
                                db_form_row(ui, "Session:", label_width, |ui, _| { ui.checkbox(&mut c.read_only,"Read-only · somente leitura"); });
                                db_form_row(ui, "Transaction control:", label_width, |ui, _| { ui.label(if c.read_only || !c.engine.is_sql() { "Read-only" } else { "Manual" }); });
                                db_form_row(ui, "Query timeout (s):", label_width, |ui, label| { ui.add(egui::DragValue::new(&mut c.timeout_secs).range(1..=120)).labelled_by(label); });
                                db_form_row(ui, "Disconnect idle (s):", label_width, |ui, label| { ui.add(egui::DragValue::new(&mut c.idle_secs).range(5..=3600)).labelled_by(label); });
                                db_form_row(ui, "Result page size:", label_width, |ui, label| { ui.add(egui::DragValue::new(&mut c.row_limit).range(1..=500)).labelled_by(label); });
                                ui.add_space(12.);
                                ui.label(RichText::new(if c.engine == Engine::Mongo { "Pool de1 por servidor; descoberta e monitoramento usam conexões adicionais." } else { "Uma sessão ativa. O teste fecha a conexão ao terminar." }).color(color(self.tokens.muted)));
                                if c.engine == Engine::Mongo { ui.label(RichText::new("O teste fecha o cliente ao terminar. Leituras e escritas sem retry automático.").color(color(self.tokens.muted))); }
                                ui.label(RichText::new("Enviar alterações e Commit são ações separadas.").color(color(self.tokens.muted)));
                            } else {
                                db_form_row(ui, "Driver:", label_width, |ui, _| { engine_icon(ui,c.engine); ui.label(c.engine.label()); });
                                ui.add_space(49.);
                                db_form_row(ui, "Host:", label_width, |ui, label| {
                                    let available = ui.available_width();
                                    let host_width = (available-self.tokens.database_form_trailing_width).max(80.);
                                    ui.add_sized([host_width,self.tokens.control_height],egui::TextEdit::singleline(&mut c.host).id_salt("ds.host").margin(egui::vec2(8.,5.))).labelled_by(label);
                                    ui.add_space(14.);
                                    let port_label = ui.label("Port:");
                                    ui.add_space(10.);
                                    ui.add_sized([self.tokens.database_port_width,self.tokens.control_height],egui::DragValue::new(&mut c.port).range(1..=65535)).labelled_by(port_label.id);
                                });
                                ui.add_space(11.);
                                db_form_row(ui, "Authentication:", label_width, |ui, _| { ui.label(if c.engine==Engine::Memcached { "Sem autenticação" } else { "User & Password" }); });
                                db_form_row(ui, "User:", label_width, |ui, label| { ui.add_enabled(c.engine!=Engine::Memcached,egui::TextEdit::singleline(&mut c.username).id_salt("ds.user").desired_width((ui.available_width()-self.tokens.database_form_trailing_width).max(80.)).margin(egui::vec2(8.,5.))).labelled_by(label); });
                                db_form_row(ui, "Password:", label_width, |ui, label| {
                                    ui.add_enabled(c.engine!=Engine::Memcached,egui::TextEdit::singleline(&mut self.database_ui.password).password(true).hint_text(if c.save_credentials { "<saved or enter password>" } else { "<hidden>" }).id_salt("ds.password").desired_width((ui.available_width()-self.tokens.database_form_trailing_width).max(80.)).margin(egui::vec2(8.,5.))).labelled_by(label);
                                    ui.add_space(14.);
                                    ui.label("Save:");
                                    ui.add_space(4.);
                                    let response=egui::ComboBox::from_id_salt("ds.save_password").selected_text(if c.save_credentials {"Forever"} else {"Never"}).width(self.tokens.database_save_width).show_ui(ui,|ui| {
                                        ui.selectable_value(&mut c.save_credentials,true,"Forever");
                                        ui.selectable_value(&mut c.save_credentials,false,"Never");
                                    }).response;
                                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox,true,"Lembrar senha no cofre do sistema"));
                                    response.on_hover_text("Lembrar senha no cofre do sistema");
                                });
                                ui.add_space(12.);
                                db_form_row(ui, "Database:", label_width, |ui, label| { ui.add_enabled(c.engine!=Engine::Memcached,egui::TextEdit::singleline(&mut c.database).id_salt("ds.database").desired_width(ui.available_width()-3.).hint_text(if matches!(c.engine,Engine::Mysql|Engine::AuroraMysql) { "Todos os bancos disponíveis" } else { "" }).margin(egui::vec2(8.,5.))).labelled_by(label); });
                                if c.engine==Engine::Mongo {
                                    db_form_row(ui,"Auth database:",label_width,|ui,label| { ui.add(egui::TextEdit::singleline(&mut c.auth_database).id_salt("ds.auth_database").desired_width(ui.available_width()-3.).margin(egui::vec2(8.,5.))).labelled_by(label); });
                                }
                                ui.add_space(28.);
                                db_form_row(ui,"Endpoint:",label_width,|ui,_| {
                                    egui::Frame::new().stroke(egui::Stroke::new(1.,color(self.tokens.control_border))).corner_radius(self.tokens.control_corner_radius).inner_margin(egui::Margin::symmetric(8,5)).show(ui,|ui| {
                                        ui.set_width((ui.available_width()-2.).max(40.));
                                        ui.add(egui::Label::new(RichText::new(format!("{}:{}{}",c.host,c.port,if c.database.is_empty(){String::new()}else{format!("/{}",c.database)})).monospace()).truncate());
                                    });
                                });
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 12.;
                            if ui.add_enabled(!self.service.remote_db.is_busy(),egui::Button::new(RichText::new("Test Connection").color(color(self.tokens.accent))).frame(false).min_size(egui::vec2(106.,28.))).clicked() { test=true; }
                            let status = self.service.remote_db.error.as_deref().unwrap_or(&self.service.remote_db.status);
                            crate::status_messages::status_message(ui, status).on_hover_text(status);
                        });
                    });
                });
            });
            let (line,_) = ui.allocate_exact_size(egui::vec2(width,1.),egui::Sense::hover());
            ui.painter().rect_filled(line,0.,color(self.tokens.border));
            egui::Frame::new().inner_margin(egui::Margin::symmetric(15,10)).show(ui, |ui| {
                ui.set_width(width-30.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui| {
                    ui.spacing_mut().item_spacing.x=12.;
                    save = ui.add_enabled(!self.service.remote_db.is_busy(),self.tokens.dialog_button("Apply",false)).clicked();
                    close |= ui.add(self.tokens.dialog_button("Cancel",false)).clicked();
                    connect = ui.add_enabled(!self.service.remote_db.is_busy(),self.tokens.dialog_button("Connect",true)).clicked();
                });
            });
        });
        if save {
            self.remote_send(RemoteCommand::SaveRemembered {
                config: self.database_ui.config.clone(),
                password: self.database_ui.password.clone(),
            });
        }
        if delete {
            self.remote_send(RemoteCommand::DeleteRemembered(
                self.database_ui.config.id.clone(),
            ));
        }
        if test || connect {
            if !self.database_ui.staged.is_empty() {
                self.service.remote_db.error =
                    Some("Descarte ou envie as alterações locais antes de trocar a conexão".into());
            } else {
                self.database_ui.remote_results = true;
                self.remote_send(RemoteCommand::ConnectRemembered {
                    config: self.database_ui.config.clone(),
                    password: self.database_ui.password.clone(),
                    test,
                });
                if connect {
                    close = true;
                }
            }
        }
        if close {
            self.database_ui.dialog = false;
            self.database_ui.password.clear();
        }
    }
    pub(super) fn execute_remote(&mut self) {
        self.database_ui.output = false;
        if !self.database_ui.staged.is_empty() || self.database_ui.edit.is_some() {
            self.service.remote_db.error =
                Some("Envie ou descarte as alterações antes de executar outra consulta".into());
            return;
        }
        if let Some(doc) = self.service.active_document() {
            if let Err(error) = self.service.remote_db.console_target(&doc.path) {
                self.service.remote_db.error = Some(error);
                self.database_ui.remote_results = true;
                self.show_activity = true;
                self.tool_tab = ToolTab::Results;
                return;
            }
            let sql = match self.selected_execution_sql() {
                Ok(sql) => sql,
                Err(error) => {
                    self.service.remote_db.error = Some(error);
                    self.database_ui.remote_results = true;
                    self.show_activity = true;
                    self.tool_tab = ToolTab::Results;
                    return;
                }
            };
            let path = doc.path.clone();
            let target = self.service.remote_db.console_bindings.get(&path).cloned();
            let command = if let Some(datasource_id) = target.clone() {
                RemoteCommand::ExecuteConsole {
                    sql: sql.clone(),
                    datasource_id,
                }
            } else {
                RemoteCommand::Execute(sql.clone())
            };
            self.database_ui.remote_results = true;
            self.database_ui.output = false;
            match self.service.dispatch(Command::RemoteDatabase(command)) {
                Ok(()) => {
                    self.database_ui.table = None;
                    self.database_ui.page_offset = 0;
                    self.database_ui.pending_result_source = Some(Some(path));
                    self.database_ui.pending_sql = Some(sql);
                    self.database_ui.pending_console_source = target;
                }
                Err(error) => self.service.remote_db.error = Some(error),
            }
            self.show_activity = true;
            self.tool_tab = ToolTab::Results;
        }
    }
    pub(super) fn edit_batch(&self) -> Option<EditBatch> {
        if !self.grid_edit_binding_valid() || !self.grid_edit_budget_valid() {
            return None;
        }
        if !self.database_ui.remote_results {
            return self.sqlite_edit_batch();
        }
        let (table, columns) = self.service.remote_db.origin.as_ref()?;
        let r = self.service.remote_db.result.as_ref()?;
        Some(EditBatch {
            table: table.clone(),
            columns: columns.clone(),
            edits: self
                .database_ui
                .staged
                .iter()
                .filter_map(|(&(row, col), value)| {
                    Some(CellEdit {
                        row: r.rows.get(row)?.clone(),
                        column: r.columns.get(col)?.clone(),
                        value: value.clone(),
                    })
                })
                .collect(),
        })
    }
    pub(super) fn can_edit_remote(&self) -> bool {
        self.service
            .remote_db
            .active
            .as_ref()
            .is_some_and(|c| c.engine.is_sql() && !c.read_only)
            && !self.service.remote_db.active.as_ref().is_some_and(|a| {
                self.service
                    .remote_db
                    .profiles
                    .iter()
                    .any(|p| p.id == a.id && p.read_only)
            })
            && self
                .service
                .remote_db
                .origin
                .as_ref()
                .is_some_and(|(_, c)| c.iter().any(|v| v.primary_key))
            && !self.service.remote_db.is_busy()
    }
    pub(super) fn remote_results(&mut self, ui: &mut egui::Ui) {
        let busy = self.service.remote_db.is_busy();
        let active = self.service.remote_db.active.is_some();
        let edit = self.can_edit_remote();
        ui.spacing_mut().item_spacing.y = 0.;
        if let Some(e) = &self.service.remote_db.error {
            self.query_execution_error(ui, e);
        }
        ui.horizontal(|ui| {
            ui.set_min_height(self.tokens.database_toolbar_height);
            ui.spacing_mut().item_spacing.x = 0.;
            if db_icon(
                ui,
                Icon::Play,
                "Execute",
                !busy
                    && (active
                        || self.service.active_document().is_some_and(|d| {
                            self.service
                                .remote_db
                                .console_bindings
                                .contains_key(&d.path)
                        })),
            )
            .on_hover_text("Ctrl+Enter · executa seleção ou instrução sob o cursor")
            .clicked()
            {
                self.execute_remote();
            }
            if db_icon(ui, Icon::Stop, "Stop", busy).clicked() {
                self.remote_send(RemoteCommand::Cancel);
            }
            self.sql_productivity_toolbar(ui);
            self.query_navigation_toolbar(ui);
            ui.separator();
            if db_icon(
                ui,
                Icon::Submit,
                "Submit",
                edit && (!self.database_ui.staged.is_empty() || self.database_ui.edit.is_some())
                    && !busy,
            )
            .on_hover_text("Enviar alterações; Commit separado")
            .clicked()
            {
                self.submit_grid_edits(false);
            }
            let preview_button = db_icon(
                ui,
                Icon::SqlPreview,
                "SQL Preview",
                !self.database_ui.staged.is_empty(),
            );
            if preview_button.clicked()
                && let Some(batch) = self.edit_batch()
                && let Some(c) = &self.service.remote_db.active
            {
                match remote::preview(&batch, c.engine) {
                    Ok(s) => {
                        self.database_ui.preview = Some(s);
                        self.database_ui.preview_anchor = Some(preview_button.rect.left_top());
                    }
                    Err(e) => self.service.remote_db.error = Some(e),
                }
            }
            if db_icon(
                ui,
                Icon::Revert,
                "Revert",
                !self.database_ui.staged.is_empty(),
            )
            .clicked()
            {
                self.discard_grid_edits();
            }
            ui.separator();
            if db_icon(
                ui,
                Icon::Commit,
                "Commit",
                (self.service.remote_db.transaction
                    || !self.database_ui.staged.is_empty()
                    || self.database_ui.edit.is_some())
                    && !busy,
            )
            .clicked()
            {
                self.submit_grid_edits(true);
            }
            if db_icon(
                ui,
                Icon::Rollback,
                "Rollback",
                self.service.remote_db.transaction && !busy,
            )
            .clicked()
            {
                self.rollback_grid_edits();
            }
            self.result_transfer_toolbar(ui);
        });
        self.grid_edit_toolbar(ui);
        if let Some(table) = self.database_ui.table.clone() {
            ui.horizontal(|ui| {
                let limit = self
                    .service
                    .remote_db
                    .active
                    .as_ref()
                    .map_or(100, |c| c.row_limit) as u64;
                if db_icon(ui, Icon::Refresh, "Reload page", active && !busy).clicked() {
                    self.remote_send(RemoteCommand::LoadTablePage {
                        table: table.clone(),
                        offset: self.database_ui.page_offset,
                    });
                }
                if db_icon(
                    ui,
                    Icon::Previous,
                    "Previous",
                    active && !busy && self.database_ui.page_offset > 0,
                )
                .clicked()
                {
                    self.database_ui.page_offset =
                        self.database_ui.page_offset.saturating_sub(limit);
                    self.remote_send(RemoteCommand::LoadTablePage {
                        table: table.clone(),
                        offset: self.database_ui.page_offset,
                    });
                }
                if db_icon(
                    ui,
                    Icon::Next,
                    "Next",
                    active
                        && !busy
                        && self
                            .service
                            .remote_db
                            .result
                            .as_ref()
                            .is_some_and(|r| r.truncated),
                )
                .clicked()
                {
                    self.database_ui.page_offset =
                        self.database_ui.page_offset.saturating_add(limit);
                    self.remote_send(RemoteCommand::LoadTablePage {
                        table: table.clone(),
                        offset: self.database_ui.page_offset,
                    });
                }
                let rows = self
                    .service
                    .remote_db
                    .result
                    .as_ref()
                    .map_or(0, |r| r.rows.len()) as u64;
                let start = if rows == 0 {
                    0
                } else {
                    self.database_ui.page_offset + 1
                };
                let end = if rows == 0 {
                    0
                } else {
                    self.database_ui.page_offset + rows
                };
                ui.label(format!("{start}–{end}"))
                    .on_hover_text("Linhas desta página · use as setas para navegar");
            });
        }
        if self.alternative_results(ui) {
            return;
        }
        self.grid_inspection_bar(ui);
        let columns = self.transfer_views.grid.visible_columns();
        let rows = self.transfer_views.grid.visible_rows();
        let sort_state = self.query_sort_state();
        let sort_available = !self.query_page_busy();
        let edit_columns = self.grid_edit_columns();
        let binding_valid = self.grid_edit_binding_valid();
        let Some(result) = self.service.remote_db.result.as_ref() else {
            ui.label(
                RichText::new("Abra um console ou selecione uma tabela.")
                    .color(color(self.tokens.muted)),
            );
            return;
        };
        if self.database_ui.widths.is_empty() {
            self.database_ui.widths = result
                .columns
                .iter()
                .enumerate()
                .map(|(col, name)| {
                    let font = egui::FontId::monospace(self.tokens.database_grid_font_size);
                    let measure = |text: String| {
                        ui.fonts_mut(|fonts| {
                            fonts
                                .layout_no_wrap(text, font.clone(), egui::Color32::WHITE)
                                .size()
                                .x
                        })
                    };
                    let values = result
                        .rows
                        .iter()
                        .filter_map(|r| r.get(col))
                        .map(|v| measure(v.display().chars().take(32).collect()) + 10.)
                        .fold(0_f32, f32::max);
                    values
                        .max(measure(name.chars().take(64).collect()) + 32.)
                        .round()
                        .clamp(163., 260.)
                })
                .collect();
        }
        let mut natural = self.database_ui.widths.clone();
        for (&column, &width) in &self.transfer_views.grid.widths {
            if let Some(w) = natural.get_mut(column) {
                *w = width;
            }
        }
        let total_width: f32 = columns.iter().filter_map(|c| natural.get(*c)).sum();
        let cols = result.columns.clone();
        let row_height = self.tokens.database_grid_row_height;
        let reveal = self
            .transfer_views
            .grid
            .take_scroll_target(&rows, &columns, &natural, row_height);
        let mut pending_edit = None;
        let mut sort_column = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            let scroll = egui::ScrollArea::horizontal()
                .id_salt("remote.results.horizontal")
                .auto_shrink([false, false]);
            let scroll = if let Some(reveal) = reveal {
                scroll.scroll_offset(reveal)
            } else {
                scroll
            };
            scroll.codisseu().show(ui, |ui| {
                ui.set_min_width(27. + total_width);
                ui.horizontal(|ui| {
                    grid_cell(
                        ui,
                        "#",
                        25.,
                        row_height,
                        [true, false, false, false, false, true, false, false],
                        &self.tokens,
                    );
                    for &col in &columns {
                        let name = &cols[col];
                        let primary =
                            self.service
                                .remote_db
                                .origin
                                .as_ref()
                                .is_some_and(|(_, columns)| {
                                    columns.iter().any(|c| c.name == *name && c.primary_key)
                                });
                        let response = grid_cell(
                            ui,
                            name,
                            natural[col],
                            row_height,
                            [
                                true,
                                false,
                                false,
                                false,
                                false,
                                false,
                                primary,
                                self.database_ui
                                    .selected
                                    .is_some_and(|(_, column)| column == col),
                            ],
                            &self.tokens,
                        )
                        .on_hover_text(name);
                        let response = self.transfer_views.grid.header(
                            ui,
                            response,
                            col,
                            name,
                            natural[col],
                            &self.tokens,
                        );
                        if database_navigation::sort_header(
                            ui,
                            response,
                            col,
                            name,
                            sort_state,
                            sort_available,
                            &self.tokens,
                        )
                        .clicked()
                        {
                            sort_column = Some(col);
                        }
                    }
                });
                let scroll = egui::ScrollArea::vertical()
                    .id_salt("remote.results.vertical")
                    .auto_shrink([false, false]);
                let scroll = if let Some(reveal) = reveal {
                    scroll.scroll_offset(reveal)
                } else {
                    scroll
                };
                scroll
                    .codisseu()
                    .show_rows(ui, row_height, rows.len(), |ui, visible| {
                        for visual_index in visible {
                            let index = rows[visual_index];
                            let r = index + 1;
                            ui.horizontal(|ui| {
                                let gutter = grid_cell(
                                    ui,
                                    &(self
                                        .database_ui
                                        .remote_page
                                        .as_ref()
                                        .map_or(0, |p| p.offset)
                                        + r as u64)
                                        .to_string(),
                                    25.,
                                    row_height,
                                    [
                                        r == 0,
                                        false,
                                        r > 0
                                            && self
                                                .database_ui
                                                .staged
                                                .keys()
                                                .any(|&(row, _)| row == r - 1),
                                        false,
                                        false,
                                        true,
                                        false,
                                        r > 0 && self.grid_selection.contains(r - 1, 0),
                                    ],
                                    &self.tokens,
                                );
                                if gutter.clicked() {
                                    self.grid_selection.row_clicked(
                                        index,
                                        database_selection::click_modifiers(ui),
                                    );
                                    gutter.request_focus();
                                    self.grid_focus = Some(gutter.id);
                                }
                                for &col in &columns {
                                    let name = &cols[col];
                                    let staged = if r > 0 && binding_valid {
                                        self.database_ui.staged.get(&(r - 1, col)).cloned()
                                    } else {
                                        None
                                    };
                                    let value = if r == 0 {
                                        None
                                    } else {
                                        staged.as_ref().or_else(|| {
                                            result.rows.get(r - 1).and_then(|row| row.get(col))
                                        })
                                    };
                                    let text =
                                        value.map(Cell::display).unwrap_or_else(|| name.clone());
                                    let selected =
                                        r > 0 && self.grid_selection.contains(r - 1, col);
                                    let response = grid_cell(
                                        ui,
                                        &text,
                                        natural[col],
                                        row_height,
                                        [
                                            r == 0,
                                            selected,
                                            staged.is_some(),
                                            matches!(value, Some(Cell::Number(_))),
                                            matches!(value, Some(Cell::Null)),
                                            false,
                                            r == 0
                                                && self
                                                    .service
                                                    .remote_db
                                                    .origin
                                                    .as_ref()
                                                    .is_some_and(|(_, columns)| {
                                                        columns.iter().any(|c| {
                                                            c.name == *name && c.primary_key
                                                        })
                                                    }),
                                            self.database_ui.selected.is_some_and(
                                                |(row, column)| {
                                                    if r == 0 {
                                                        column == col
                                                    } else {
                                                        row == r - 1
                                                    }
                                                },
                                            ),
                                        ],
                                        &self.tokens,
                                    )
                                    .on_hover_text(&text);
                                    if self.transfer_views.grid.matches(r - 1, col) {
                                        ui.painter().rect_stroke(
                                            response.rect.shrink(1.),
                                            0.,
                                            egui::Stroke::new(1., color(self.tokens.accent)),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if r > 0 && response.clicked() {
                                        self.database_ui.selected = Some((r - 1, col));
                                        self.grid_selection.clicked(
                                            r - 1,
                                            col,
                                            database_selection::click_modifiers(ui),
                                        );
                                        response.request_focus();
                                        self.grid_focus = Some(response.id);
                                    }
                                    if r > 0 {
                                        database_cell_edit::inline_cell_editor(
                                            ui,
                                            &response,
                                            r - 1,
                                            col,
                                            edit_columns.get(col),
                                            &mut self.database_ui,
                                            &self.tokens,
                                        );
                                        if response.double_clicked()
                                            && edit
                                            && edit_columns
                                                .get(col)
                                                .is_some_and(database_cell_edit::editable_column)
                                        {
                                            pending_edit = Some((r - 1, col));
                                        }
                                    }
                                    if response.has_focus()
                                        && ui.input_mut(|i| {
                                            i.consume_key(Modifiers::SHIFT, Key::Enter)
                                        })
                                        && let Some(value) = value
                                    {
                                        self.transfer_views.open_value(r - 1, col, value, false);
                                    }
                                    response.context_menu(|ui| {
                                        if ui.button("Copy value").clicked() {
                                            ui.ctx().copy_text(
                                                value
                                                    .map(Cell::clipboard_text)
                                                    .unwrap_or_else(|| name.clone()),
                                            );
                                            ui.close();
                                        }
                                    });
                                }
                            });
                        }
                    });
            });
        });
        if let Some((row, col)) = pending_edit {
            self.start_grid_edit(row, col);
        }
        if let Some(column) = sort_column {
            self.sort_query_column(column);
        }
    }
    pub(super) fn database_edit_windows(&mut self, ctx: &egui::Context) {
        if self.database_ui.exit_confirm {
            let mut action = if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                1
            } else {
                0
            };
            egui::Modal::new(egui::Id::unique("database.exit.pending")).show(ctx,|ui|{
                ui.set_width(460.);ui.heading("Alterações de banco pendentes");
                if !self.database_ui.staged.is_empty(){ui.label(format!("{} célula(s) editada(s) ainda não foram enviadas.",self.database_ui.staged.len()));}
                if self.service.remote_db.transaction || self.service.db.transaction{ui.label("Há uma transação aberta. Escolha Commit ou descarte com Rollback antes de sair.");}
                if let Some(e)=&self.service.remote_db.error{crate::status_messages::status_message(ui, e);}
                ui.horizontal(|ui|{
                    if ui.button("Voltar ao banco").clicked(){action=1;}
                    if ui.button("Descartar e sair").clicked(){action=2;}
                    if ui.add_enabled(self.database_ui.staged.is_empty()&&self.database_ui.edit.is_none()&&(self.service.remote_db.transaction || self.service.db.transaction),egui::Button::new("Commit e sair")).clicked(){action=3;}
                });
            });
            match action {
                1 => {
                    self.database_ui.exit_confirm = false;
                    self.show_activity = true;
                    self.tool_tab = ToolTab::Results;
                }
                2 | 3 => {
                    self.database_ui.exit_confirm = false;
                    if action == 2 {
                        self.discard_grid_edits();
                    }
                    if self.service.db.transaction {
                        self.send(Command::Database(if action == 3 {
                            codisseu_core::DbCommand::Commit
                        } else {
                            codisseu_core::DbCommand::Rollback
                        }));
                        self.database_ui.exit_wait = true;
                    } else if self.service.remote_db.transaction {
                        self.remote_send(if action == 3 {
                            RemoteCommand::Commit
                        } else {
                            RemoteCommand::Rollback
                        });
                        self.database_ui.exit_wait = true;
                    } else {
                        self.service.remote_db.error = None;
                        self.request_exit();
                    }
                }
                _ => {}
            }
            return;
        }

        if (self.database_ui.preview.is_some() || self.database_ui.edit.is_some())
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.database_ui.preview = None;
            self.database_ui.edit = None;
            return;
        }
        if let Some(text) = self.database_ui.preview.clone() {
            let mut open = true;
            let width = self
                .tokens
                .database_preview_width
                .min(ctx.content_rect().width() - 32.);
            let anchor = self
                .database_ui
                .preview_anchor
                .unwrap_or(ctx.content_rect().center());
            let position = egui::pos2(
                (anchor.x - width / 2.)
                    .clamp(12., (ctx.content_rect().right() - width - 12.).max(12.)),
                (anchor.y - self.tokens.database_preview_height - 70.).max(12.),
            );
            let mut submit = false;
            egui::Window::new("SQL Preview · alterações pendentes")
                .id(egui::Id::unique("db.preview"))
                .title_bar(false)
                .collapsible(false)
                .resizable(false)
                .fixed_pos(position)
                .fixed_size(egui::vec2(width, self.tokens.database_preview_height + 48.))
                .frame(
                    egui::Frame::new()
                        .fill(color(self.tokens.panel))
                        .inner_margin(12)
                        .stroke(egui::Stroke::new(1., color(self.tokens.popup_border))),
                )
                .show(ctx, |ui| {
                    self.tokens.form_style(ui);
                    let mut content = text.clone();
                    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap: f32| {
                        let mut job = highlight::layout_themed(
                            buffer.as_str(),
                            "sql",
                            egui::FontId::monospace(self.tokens.database_grid_font_size),
                            self.tokens.theme,
                        );
                        job.wrap.max_width = wrap;
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    egui::ScrollArea::both()
                        .max_height(self.tokens.database_preview_height)
                        .min_scrolled_height(self.tokens.database_preview_height)
                        .codisseu()
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut content)
                                    .code_editor()
                                    .interactive(false)
                                    .frame(egui::Frame::NONE)
                                    .desired_rows(7)
                                    .desired_width(width - 24.)
                                    .layouter(&mut layouter),
                            );
                        });
                    ui.horizontal(|ui| {
                        if db_icon(ui, Icon::Copy, "Copy SQL + parameters", true).clicked() {
                            ui.ctx().copy_text(text.clone());
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(self.tokens.dialog_button("Cancel", false)).clicked() {
                                open = false;
                            }
                            submit = ui
                                .add_enabled(
                                    !self.service.remote_db.is_busy() && self.can_edit_remote(),
                                    self.tokens.dialog_button("Submit", true),
                                )
                                .clicked();
                        });
                    });
                });
            if submit {
                self.submit_grid_edits(false);
                open = false;
            }
            if !open {
                self.database_ui.preview = None;
            }
        }
    }
}
pub(super) fn grid_cell(
    ui: &mut egui::Ui,
    text: &str,
    width: f32,
    height: f32,
    flags: [bool; 8],
    tokens: &Tokens,
) -> egui::Response {
    let [
        heading,
        selected,
        dirty,
        numeric,
        null,
        row_header,
        primary_key,
        active_row,
    ] = flags;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    let fill = if selected && dirty {
        color(tokens.database_grid_dirty_selection)
    } else if selected {
        color(tokens.database_grid_selection)
    } else if dirty {
        color(tokens.database_dirty)
    } else if heading && !active_row {
        color(tokens.database_grid_header)
    } else if active_row {
        color(tokens.database_grid_active_row)
    } else {
        color(tokens.editor)
    };
    ui.painter().rect_filled(rect, 0., fill);
    ui.painter().line_segment(
        [
            rect.right_top() - egui::vec2(0.5, 0.),
            rect.right_bottom() - egui::vec2(0.5, 0.),
        ],
        egui::Stroke::new(1., color(tokens.database_grid_separator)),
    );
    ui.painter().line_segment(
        [
            rect.left_bottom() - egui::vec2(0., 0.5),
            rect.right_bottom() - egui::vec2(0., 0.5),
        ],
        egui::Stroke::new(1., color(tokens.database_grid_separator)),
    );
    let mut text_rect = rect.shrink2(egui::vec2(if row_header { 2. } else { 3. }, 0.));
    if heading && !row_header {
        text_rect.max.x -= 37.;
    }
    let p = ui.painter().with_clip_rect(text_rect);
    let tint = color(if null || row_header {
        tokens.database_grid_null
    } else if heading || selected {
        tokens.database_grid_header_foreground
    } else {
        tokens.database_grid_foreground
    });
    let text_left = if heading && !row_header {
        components::paint_database_column(
            ui,
            egui::Rect::from_center_size(
                rect.left_center() + egui::vec2(11., 0.),
                egui::vec2(16., 16.),
            ),
            primary_key,
            color(tokens.database_grid_header_foreground),
            color(tokens.database_completion_key),
        );
        24.
    } else {
        3.
    };
    let (pos, align) = if numeric || row_header {
        (
            rect.right_center() - egui::vec2(4., 0.),
            egui::Align2::RIGHT_CENTER,
        )
    } else {
        (
            rect.left_center() + egui::vec2(text_left, 0.),
            egui::Align2::LEFT_CENTER,
        )
    };
    p.text(
        pos,
        align,
        if null {
            "<null>"
        } else if heading && row_header {
            ""
        } else {
            text
        },
        egui::FontId::monospace(if row_header {
            tokens.database_row_header_font_size
        } else {
            tokens.database_grid_font_size
        }),
        tint,
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Cell, true, selected, text));
    response
}

// Full-width selection, matching the database tool rather than a text-sized pill.
fn datasource_row(
    ui: &mut egui::Ui,
    config: &ConnectionConfig,
    selected: bool,
    active: bool,
    inset: f32,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), tokens.row_height),
        egui::Sense::click(),
    );
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(4., 0.)),
            tokens.control_corner_radius,
            color(if selected {
                tokens.selection
            } else {
                tokens.border
            }),
        );
    }
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + inset, rect.center().y),
        egui::vec2(16., 16.),
    );
    let tint = match config.engine {
        Engine::Mysql | Engine::AuroraMysql => tokens.database_engine_mysql,
        Engine::Postgres | Engine::AuroraPostgres => tokens.database_engine_postgres,
        Engine::Mongo => tokens.database_engine_mongo,
        Engine::Redis => tokens.database_engine_redis,
        Engine::Memcached => tokens.database_engine_memcached,
    };
    components::paint_database_engine(ui, icon, Some(config.engine), color(tint));
    let painter = ui.painter().with_clip_rect(egui::Rect::from_min_max(
        egui::pos2(rect.left() + inset + 14., rect.top()),
        egui::pos2(rect.right() - 18., rect.bottom()),
    ));
    painter.text(
        egui::pos2(rect.left() + inset + 14., rect.center().y),
        egui::Align2::LEFT_CENTER,
        &config.name,
        egui::FontId::proportional(tokens.ui_font_size),
        color(tokens.text),
    );
    if active {
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 12., rect.center().y),
            2.,
            color(tokens.accent),
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.),
            tokens.control_corner_radius,
            egui::Stroke::new(1., color(tokens.accent)),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, &config.name)
    });
    response.on_hover_text(format!(
        "{} · {}:{}",
        config.engine.label(),
        config.host,
        config.port
    ))
}

#[cfg(test)]
#[path = "database_explorer_tests.rs"]
mod explorer_tests;

fn database_tab(ui: &mut egui::Ui, label: &str, selected: bool, tokens: &Tokens) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.into(),
        egui::FontId::proportional(tokens.ui_font_size - 1.),
        color(tokens.text),
    );
    let (id, allocated) = ui.allocate_space(egui::vec2(galley.size().x + 18., 28.));
    // Paint/hit-test the inset without changing the form's following layout origin.
    let rect = allocated.translate(egui::vec2(-9., 0.));
    let response = ui.interact(rect, id, egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink(0.5),
            4.,
            color(if selected {
                tokens.database_tab_selection
            } else {
                tokens.border
            }),
        );
    }
    ui.painter().galley(
        egui::pos2(rect.left() + 9., rect.center().y - galley.size().y / 2.),
        galley,
        color(tokens.text),
    );
    if selected {
        ui.painter().rect_stroke(
            rect.shrink(0.5),
            4.,
            egui::Stroke::new(1., color(tokens.database_tab_border)),
            egui::StrokeKind::Inside,
        );
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response
}
fn db_form_row(
    ui: &mut egui::Ui,
    label: &str,
    label_width: f32,
    add: impl FnOnce(&mut egui::Ui, egui::Id),
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.;
        let (rect, label_response) =
            ui.allocate_exact_size(egui::vec2(label_width, 28.), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        label_response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            add(ui, label_response.id);
        });
    });
}

fn database_menu_item(
    ui: &mut egui::Ui,
    label: &str,
    engine: Option<Engine>,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), tokens.menu_row_height),
        egui::Sense::click(),
    );
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 3., color(tokens.selection));
    }
    let tint = match engine {
        Some(Engine::Mysql | Engine::AuroraMysql) => tokens.database_engine_mysql,
        Some(Engine::Postgres | Engine::AuroraPostgres) => tokens.database_engine_postgres,
        Some(Engine::Mongo) => tokens.database_engine_mongo,
        Some(Engine::Redis) => tokens.database_engine_redis,
        Some(Engine::Memcached) => tokens.database_engine_memcached,
        None => tokens.database_engine_mysql,
    };
    components::paint_database_engine(
        ui,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(14., 0.),
            egui::vec2(16., 16.),
        ),
        engine,
        color(tint),
    );
    ui.painter().text(
        rect.left_center() + egui::vec2(30., 0.),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(tokens.ui_font_size),
        color(tokens.text),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

/// Working datasource choices in the shape of the observed empty Database cards.
fn database_empty_card(
    ui: &mut egui::Ui,
    label: &str,
    engine: Option<Engine>,
    more: bool,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.), egui::Sense::click());
    ui.painter().rect_filled(
        rect,
        7.,
        color(if response.hovered() || response.has_focus() {
            tokens.border
        } else {
            tokens.bar
        }),
    );
    let icon = egui::Rect::from_center_size(
        rect.left_center() + egui::vec2(22., 0.),
        egui::vec2(16., 16.),
    );
    if more {
        components::paint_icon(ui, icon, Icon::MoreDots, color(tokens.muted));
        components::paint_icon(
            ui,
            egui::Rect::from_center_size(
                rect.right_center() - egui::vec2(18., 0.),
                egui::vec2(12., 12.),
            ),
            Icon::Next,
            color(tokens.muted),
        );
    } else {
        let tint = match engine {
            Some(Engine::Mysql | Engine::AuroraMysql) => tokens.database_engine_mysql,
            Some(Engine::Postgres | Engine::AuroraPostgres) => tokens.database_engine_postgres,
            Some(Engine::Mongo) => tokens.database_engine_mongo,
            Some(Engine::Redis) => tokens.database_engine_redis,
            Some(Engine::Memcached) => tokens.database_engine_memcached,
            None => tokens.database_engine_mysql,
        };
        components::paint_database_engine(ui, icon, engine, color(tint));
    }
    let text_rect = egui::Rect::from_min_max(
        rect.min + egui::vec2(42., 0.),
        rect.max - egui::vec2(32., 0.),
    );
    ui.painter().with_clip_rect(text_rect).text(
        text_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(tokens.ui_font_size),
        color(tokens.text),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}

#[cfg(test)]
mod project_scope_tests {
    use super::*;

    fn settle(app: &mut CodisseuApp) {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.service.poll();
            app.sync_database_ui();
            app.sql_productivity_tick();
            if !app.service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.service.last_error.is_none());
    }

    fn seed_project_ui(app: &mut CodisseuApp, query: PathBuf) {
        app.database_ui.select(ConnectionConfig {
            id: "shipping-only".into(),
            name: "Shipping private connection".into(),
            host: "shipping.invalid".into(),
            database: "shipping".into(),
            ..Default::default()
        });
        app.database_ui.password = "shipping private password".into();
        app.database_ui.dialog = true;
        app.database_ui.preview = Some("Shipping private preview".into());
        app.database_ui.edit = Some((0, 0, "Shipping private cell".into(), false));
        app.database_ui.result_sql = Some("SELECT 'shipping'".into());
        app.database_ui.result_source = Some(query.clone());
        app.database_ui.sqlite_result_sql = Some("SELECT 'shipping SQLite'".into());
        app.database_ui.sqlite_result_source = Some(query.clone());
        app.database_ui
            .catalog_filters
            .insert("shipping".into(), "private filter".into());
        app.database_ui
            .expanded_namespaces
            .insert("shipping".into());
        app.database_input = "shipping-private.sqlite".into();
        app.transfer_views.columns_open = true;
        app.rename_query_dialog(query);
    }

    fn assert_cleared(app: &CodisseuApp) {
        assert_eq!(app.database_ui.config, ConnectionConfig::default());
        assert!(app.database_ui.password.is_empty());
        assert!(app.database_ui.selected_source.is_none());
        assert!(!app.database_ui.dialog);
        assert!(app.database_ui.preview.is_none());
        assert!(app.database_ui.edit.is_none());
        assert!(app.database_ui.result_sql.is_none());
        assert!(app.database_ui.result_source.is_none());
        assert!(app.database_ui.sqlite_result_sql.is_none());
        assert!(app.database_ui.sqlite_result_source.is_none());
        assert!(app.database_ui.catalog_filters.is_empty());
        assert!(app.database_ui.expanded_namespaces.is_empty());
        assert!(app.database_input.is_empty());
        assert!(!app.query_library_modal_open());
        assert!(!app.sql_productivity_modal_open());
        assert!(!app.query_export_modal_open());
        assert!(!app.transfer_views.columns_open);
    }

    #[test]
    fn database_ui_keeps_same_project_state_but_clears_credentials_results_and_dialogs_on_a_b_a() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("shipping");
        let b = dir.path().join("game");
        for root in [&a, &b] {
            std::fs::create_dir(root).unwrap();
        }
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        settle(&mut app);
        app.service
            .dispatch(Command::OpenWorkspace(a.clone()))
            .unwrap();
        settle(&mut app);
        seed_project_ui(&mut app, a.join("shipping.sql"));
        app.service
            .dispatch(Command::OpenWorkspace(a.clone()))
            .unwrap();
        settle(&mut app);
        assert_eq!(app.database_ui.config.name, "Shipping private connection");
        assert_eq!(app.database_ui.password, "shipping private password");
        assert!(app.query_library_modal_open());

        app.service.dispatch(Command::OpenWorkspace(b)).unwrap();
        settle(&mut app);
        assert_cleared(&app);
        app.service.dispatch(Command::OpenWorkspace(a)).unwrap();
        settle(&mut app);
        assert_cleared(&app);
        app.service.dispatch(Command::CloseWorkspace).unwrap();
        settle(&mut app);
        assert_cleared(&app);
    }

    #[test]
    fn neighboring_workspace_files_are_distinct_database_ui_scopes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("shared-root");
        std::fs::create_dir(&root).unwrap();
        let files = [
            dir.path().join("one.code-workspace"),
            dir.path().join("two.code-workspace"),
        ];
        for file in &files {
            std::fs::write(
                file,
                serde_json::json!({"folders":[{"path":root}]}).to_string(),
            )
            .unwrap();
        }
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        settle(&mut app);
        app.service
            .dispatch(Command::OpenWorkspace(files[0].clone()))
            .unwrap();
        settle(&mut app);
        seed_project_ui(&mut app, root.join("shipping.sql"));
        app.service
            .dispatch(Command::OpenWorkspace(files[1].clone()))
            .unwrap();
        settle(&mut app);
        assert_eq!(app.service.workspace.as_deref(), Some(dir.path()));
        assert_eq!(app.database_ui.project_scope.as_ref(), Some(&files[1]));
        assert_cleared(&app);
    }
}
