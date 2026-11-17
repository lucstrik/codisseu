//! Services shows live sources and open consoles, never a fabricated query history.
use super::{CodisseuApp, Command, DocumentId, color, components};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_core::remote::Engine;
use eframe::egui::{self, Color32, FontId, Id, Key, Rect, Response, Stroke, Ui, Vec2};
use std::path::{Path, PathBuf};

struct Console {
    path: PathBuf,
    document: Option<DocumentId>,
    elapsed_ms: Option<u128>,
}

struct Source {
    id: Id,
    name: String,
    description: String,
    engine: Option<Engine>,
    consoles: Vec<Console>,
    table_result: Option<(String, u128)>,
}

impl CodisseuApp {
    pub(super) fn database_services_tree(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            let has_result = if self.database_ui.remote_results {
                self.service.remote_db.result.is_some()
            } else {
                self.service.db.result.is_some()
            };
            if components::database_icon_button(
                ui,
                components::Icon::Table,
                "Mostrar resultado do serviço",
                has_result,
            )
            .clicked()
            {
                self.database_ui.output = false;
            }
            if components::database_icon_button(
                ui,
                components::Icon::Terminal,
                "Mostrar saída do serviço",
                true,
            )
            .clicked()
            {
                self.database_ui.output = true;
            }
        });
        let mut sources = Vec::new();
        let remote_result_path = self.database_ui.result_source.clone();
        let sqlite_result_path = self.database_ui.sqlite_result_source.clone();
        let result_path = if self.database_ui.remote_results {
            remote_result_path.as_deref()
        } else {
            sqlite_result_path.as_deref()
        };
        if let Some(config) = &self.service.remote_db.active {
            let mut consoles: Vec<_> = self
                .service
                .documents
                .iter()
                .filter(|document| {
                    self.service.remote_db.console_bindings.get(&document.path) == Some(&config.id)
                })
                .map(|document| Console {
                    path: document.path.clone(),
                    document: Some(document.id),
                    elapsed_ms: None,
                })
                .collect();
            if let Some(result) = &self.service.remote_db.result
                && let Some(path) = remote_result_path.as_deref()
            {
                add_result_console(&mut consoles, path, result.elapsed_ms, &self.service);
            }
            sources.push(Source {
                id: Id::unique(("services.remote", &config.id)),
                name: config.name.clone(),
                description: format!(
                    "{} · {}:{} · {} · {} · {}",
                    config.engine.label(),
                    config.host,
                    config.port,
                    if config.database.is_empty() {
                        "Sem banco padrão"
                    } else {
                        &config.database
                    },
                    if config.read_only {
                        "Somente leitura"
                    } else {
                        "Leitura e escrita"
                    },
                    if self.service.remote_db.transaction {
                        "Transação aberta"
                    } else {
                        "Sem transação aberta"
                    },
                ),
                engine: Some(config.engine),
                consoles,
                table_result: if remote_result_path.is_none() {
                    self.service
                        .remote_db
                        .origin
                        .as_ref()
                        .zip(self.service.remote_db.result.as_ref())
                        .map(|((table, _), result)| {
                            (
                                format!("{}.{}", table.namespace, table.name),
                                result.elapsed_ms,
                            )
                        })
                } else {
                    None
                },
            });
        }
        if self.service.db.connected
            && let Some(path) = &self.service.db.config
        {
            let mut consoles = Vec::new();
            if let Some(result) = &self.service.db.result
                && let Some(path) = sqlite_result_path.as_deref()
            {
                add_result_console(&mut consoles, path, result.elapsed_ms, &self.service);
            }
            sources.push(Source {
                id: Id::unique(("services.sqlite", path)),
                name: file_name(path),
                description: format!("SQLite · {} · Somente leitura", path.display()),
                engine: None,
                consoles,
                table_result: None,
            });
        }

        let selection_id = Id::unique("codisseu.services.selection");
        let mut selected = ui.ctx().data(|data| data.get_temp::<Id>(selection_id));
        let root_id = Id::unique("codisseu.services.database");
        egui::ScrollArea::vertical()
            .id_salt("codisseu.services.tree.scroll")
            .codisseu()
            .show(ui, |ui| {
                let mut root_open = expanded(ui, root_id);
                let root = service_row(
                    ui,
                    root_id,
                    "Database",
                    "Serviços de banco de dados",
                    "",
                    0,
                    Some(root_open),
                    selected == Some(root_id),
                    None,
                    Some(components::Icon::Folder),
                    color(self.tokens.muted),
                    color(self.tokens.database_services_selection),
                );
                if update_expansion(ui, &root, root_id, &mut root_open) {
                    selected = Some(root_id);
                }
                if root_open {
                    if sources.is_empty() {
                        ui.add_space(4.);
                        ui.label(
                            egui::RichText::new("Nenhuma conexão ativa")
                                .size(12.)
                                .color(color(self.tokens.muted)),
                        );
                    }
                    for source in sources {
                        let mut open = expanded(ui, source.id);
                        let tint = color(match source.engine {
                            Some(Engine::Mysql | Engine::AuroraMysql) => {
                                self.tokens.database_engine_mysql
                            }
                            Some(Engine::Postgres | Engine::AuroraPostgres) => {
                                self.tokens.database_engine_postgres
                            }
                            Some(Engine::Mongo) => self.tokens.database_engine_mongo,
                            Some(Engine::Redis) => self.tokens.database_engine_redis,
                            Some(Engine::Memcached) => self.tokens.database_engine_memcached,
                            None => self.tokens.file_type_blue,
                        });
                        let row = service_row(
                            ui,
                            source.id,
                            &source.name,
                            &source.description,
                            "",
                            1,
                            Some(open),
                            selected == Some(source.id),
                            source.engine,
                            None,
                            tint,
                            color(self.tokens.database_services_selection),
                        );
                        if update_expansion(ui, &row, source.id, &mut open) {
                            selected = Some(source.id);
                        }
                        if !open {
                            continue;
                        }
                        if source.consoles.is_empty() && source.table_result.is_none() {
                            ui.horizontal(|ui| {
                                ui.add_space(60.);
                                ui.label(
                                    egui::RichText::new("Nenhum console aberto")
                                        .size(12.)
                                        .color(color(self.tokens.muted)),
                                );
                            });
                        }
                        for console in source.consoles {
                            let id = source.id.with(&console.path);
                            let name = file_name(&console.path);
                            let label = format!("Console {name}");
                            let description =
                                format!("{} · {}", console.path.display(), source.description);
                            let detail = if console.document.is_none() {
                                "fechado".into()
                            } else {
                                console
                                    .elapsed_ms
                                    .map(|ms| format!("{ms} ms"))
                                    .unwrap_or_default()
                            };
                            let active = selected == Some(id)
                                || (selected.is_none()
                                    && result_path == Some(console.path.as_path()));
                            let row = service_row(
                                ui,
                                id,
                                &label,
                                &description,
                                &detail,
                                2,
                                None,
                                active,
                                source.engine,
                                None,
                                tint,
                                color(self.tokens.database_services_selection),
                            );
                            if row.clicked()
                                || (row.has_focus()
                                    && ui.input(|input| input.key_pressed(Key::Enter)))
                            {
                                selected = Some(id);
                                if let Some(document) = console.document {
                                    self.send(Command::Activate { id: document });
                                    self.database_ui.remote_results = source.engine.is_some();
                                    self.database_ui.output = false;
                                    self.focus_editor = true;
                                }
                            }
                        }
                        if let Some((table, elapsed_ms)) = source.table_result {
                            let id = source.id.with(("table", &table));
                            let label = format!("Tabela {table}");
                            let row = service_row(
                                ui,
                                id,
                                &label,
                                &source.description,
                                &format!("{elapsed_ms} ms"),
                                2,
                                None,
                                selected == Some(id),
                                source.engine,
                                Some(components::Icon::Table),
                                color(self.tokens.muted),
                                color(self.tokens.database_services_selection),
                            );
                            if row.clicked()
                                || (row.has_focus()
                                    && ui.input(|input| input.key_pressed(Key::Enter)))
                            {
                                selected = Some(id);
                                self.database_ui.remote_results = true;
                                self.database_ui.output = false;
                            }
                        }
                    }
                }
            });
        if let Some(selected) = selected {
            ui.ctx()
                .data_mut(|data| data.insert_temp(selection_id, selected));
        }
    }
}

fn add_result_console(
    consoles: &mut Vec<Console>,
    path: &Path,
    elapsed_ms: u128,
    service: &codisseu_core::AppService,
) {
    if let Some(console) = consoles.iter_mut().find(|console| console.path == path) {
        console.elapsed_ms = Some(elapsed_ms);
    } else {
        consoles.push(Console {
            path: path.into(),
            document: service
                .documents
                .iter()
                .find(|document| document.path == path)
                .map(|document| document.id),
            elapsed_ms: Some(elapsed_ms),
        });
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn expanded(ui: &Ui, id: Id) -> bool {
    ui.ctx()
        .data(|data| data.get_temp::<bool>(id.with("expanded")).unwrap_or(true))
}

fn update_expansion(ui: &Ui, response: &Response, id: Id, open: &mut bool) -> bool {
    let keyboard = response.has_focus();
    let toggled =
        response.clicked() || (keyboard && ui.input(|input| input.key_pressed(Key::Enter)));
    if toggled {
        *open = !*open;
    }
    if keyboard {
        if ui.input(|input| input.key_pressed(Key::ArrowLeft)) {
            *open = false;
        }
        if ui.input(|input| input.key_pressed(Key::ArrowRight)) {
            *open = true;
        }
    }
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("expanded"), *open));
    toggled
}

#[allow(clippy::too_many_arguments)]
fn service_row(
    ui: &mut Ui,
    id: Id,
    name: &str,
    description: &str,
    detail: &str,
    depth: usize,
    expanded: Option<bool>,
    selected: bool,
    engine: Option<Engine>,
    icon_override: Option<components::Icon>,
    tint: Color32,
    inactive_selection: Color32,
) -> Response {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.), egui::Sense::hover());
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(description);
    if response.clicked() {
        response.request_focus();
    }
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    ..Default::default()
                },
            )
        });
    }
    let accessible_name = match depth {
        0 => "Serviços de banco".into(),
        1 => format!("Serviço {name}"),
        _ => name.into(),
    };
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::TreeItem,
            ui.is_enabled(),
            accessible_name.as_str(),
        )
    });
    ui.ctx().accesskit_node_builder(id, |node| {
        node.set_selected(selected);
        node.set_level(depth + 1);
        node.set_description(description);
        if let Some(open) = expanded {
            node.set_expanded(open);
        }
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(4., 0.)),
            4.,
            if selected && response.has_focus() {
                ui.visuals().selection.bg_fill
            } else if selected {
                inactive_selection
            } else {
                Color32::from_white_alpha(16)
            },
        );
    }
    let left = rect.left() + 12. + depth as f32 * 16.;
    let center = rect.center().y;
    if let Some(open) = expanded {
        let points = if open {
            vec![
                egui::pos2(left, center - 2.),
                egui::pos2(left + 4., center + 2.),
                egui::pos2(left + 8., center - 2.),
            ]
        } else {
            vec![
                egui::pos2(left + 2., center - 4.),
                egui::pos2(left + 6., center),
                egui::pos2(left + 2., center + 4.),
            ]
        };
        ui.painter().add(egui::Shape::line(
            points,
            Stroke::new(1.2, ui.visuals().weak_text_color()),
        ));
    }
    let icon = Rect::from_center_size(egui::pos2(left + 23., center), Vec2::splat(16.));
    if let Some(icon_override) = icon_override {
        components::paint_database_icon(ui, icon, icon_override, tint);
    } else {
        components::paint_database_engine(ui, icon, engine, tint);
    }
    let text_rect = Rect::from_min_max(
        egui::pos2(left + 37., rect.top()),
        rect.max - egui::vec2(8., 0.),
    );
    let painter = ui
        .painter()
        .with_clip_rect(text_rect.intersect(ui.clip_rect()));
    let galley = painter.layout_no_wrap(
        name.strip_prefix("Console ").unwrap_or(name).into(),
        FontId::proportional(13.),
        ui.visuals().text_color(),
    );
    let detail_x = text_rect.left() + galley.size().x + 7.;
    painter.galley(
        egui::pos2(text_rect.left(), center - galley.size().y / 2.),
        galley,
        ui.visuals().text_color(),
    );
    if !detail.is_empty() {
        painter.text(
            egui::pos2(detail_x, center),
            egui::Align2::LEFT_CENTER,
            detail,
            FontId::proportional(12.),
            ui.visuals().weak_text_color(),
        );
    }
    response
}
