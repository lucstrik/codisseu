//! SQL catalog completion shares the bounded database worker and editor undo history.
use super::*;
use codisseu_core::{
    EditKind, Selection, TextDelta,
    remote::{ConnectionConfig, Engine, RemoteCommand, TableRef},
    sql_completion::{self as sql, Context, Suggestion},
    sql_text::SqlDialect,
};
use std::collections::BTreeSet;
#[path = "sql_completion_paint.rs"]
mod paint;

#[derive(Default)]
pub(super) struct SqlCompletion {
    popup: Option<Popup>,
}

impl SqlCompletion {
    pub(super) fn copilot_has_choices(&self) -> bool {
        self.popup.as_ref().is_some_and(|p| !p.items.is_empty())
    }
    pub(super) fn dismiss_empty_for_copilot(&mut self) {
        if !self.copilot_has_choices() {
            self.popup = None;
        }
    }
    pub(super) fn is_open(&self) -> bool {
        self.popup.is_some()
    }
}
struct Popup {
    document: DocumentId,
    version: u64,
    caret: usize,
    source: String,
    generation: u64,
    context: Context,
    items: Vec<Suggestion>,
    requested: BTreeSet<String>,
    selected: usize,
    scroll_selection: bool,
    anchor: egui::Pos2,
    rect: Option<egui::Rect>,
}
fn source(config: &ConnectionConfig) -> String {
    format!(
        "{}:{:?}:{}:{}:{}",
        config.id, config.engine, config.host, config.port, config.database
    )
}
fn dialect(config: &ConnectionConfig) -> SqlDialect {
    match config.engine {
        Engine::Postgres | Engine::AuroraPostgres => SqlDialect::Postgres,
        _ => SqlDialect::Mysql,
    }
}
impl CodisseuApp {
    pub(super) fn copilot_sql_choice(&self) -> Option<crate::copilot::SemanticChoice> {
        let popup = self.sql_completion.popup.as_ref()?;
        self.service
            .active_document()
            .filter(|d| d.id == popup.document && d.version == popup.version)?;
        let item = popup.items.get(popup.selected)?;
        Some(crate::copilot::SemanticChoice {
            range: popup.context.range.clone(),
            text: item.insert.clone(),
        })
    }
    pub(super) fn request_sql_completion(&mut self) -> bool {
        let Some(doc) = self.service.active_document() else {
            return false;
        };
        if !doc
            .path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
        {
            return false;
        }
        self.completion_popup = None;
        self.sql_completion.popup = None;
        let Some(config) = self
            .service
            .remote_db
            .active
            .as_ref()
            .filter(|c| c.engine.is_sql())
        else {
            self.service.status =
                "Conecte o datasource SQL para completar tabelas e colunas".into();
            return true;
        };
        if self
            .service
            .remote_db
            .console_bindings
            .get(&doc.path)
            .is_some_and(|id| id != &config.id)
        {
            self.service.status = "Conecte o datasource vinculado a este console".into();
            return true;
        }
        let selection = self
            .service
            .active_view()
            .and_then(|id| self.service.view(id))
            .map_or_else(|| doc.selection(), |view| view.selection());
        if !selection.is_caret() {
            return true;
        }
        let Some(context) = sql::context(doc.text(), selection.head, dialect(config)) else {
            return true;
        };
        self.sql_completion.popup = Some(Popup {
            document: doc.id,
            version: doc.version,
            caret: selection.head,
            source: source(config),
            generation: self.service.remote_db.session_generation,
            context,
            items: vec![],
            requested: BTreeSet::new(),
            selected: 0,
            scroll_selection: true,
            anchor: self.caret_screen.unwrap_or(egui::pos2(353., 104.)),
            rect: None,
        });
        self.focus_editor = true;
        true
    }
    pub(super) fn sql_completion_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let Some(popup) = self.sql_completion.popup.as_ref() else {
            return false;
        };
        let inside = popup.rect.is_some_and(|r| {
            ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| r.contains(p)))
        });
        if !self.editor_focused(ctx) && !inside {
            self.sql_completion.popup = None;
            return false;
        }
        let take = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        if take(Key::Escape) {
            self.sql_completion.popup = None;
            self.focus_editor = true;
            return true;
        }
        let count = popup.items.len();
        if count > 0 {
            if take(Key::ArrowDown) {
                let p = self.sql_completion.popup.as_mut().unwrap();
                p.selected = (p.selected + 1) % count;
                p.scroll_selection = true;
                return true;
            }
            if take(Key::ArrowUp) {
                let p = self.sql_completion.popup.as_mut().unwrap();
                p.selected = (p.selected + count - 1) % count;
                p.scroll_selection = true;
                return true;
            }
            if take(Key::Enter) || take(Key::Tab) {
                self.apply_sql_completion();
                return true;
            }
        }
        if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key {key,pressed:true,modifiers,..}
            if matches!(key,Key::ArrowLeft|Key::ArrowRight|Key::Home|Key::End|Key::PageUp|Key::PageDown)
            || ((modifiers.command||modifiers.ctrl||modifiers.alt) && *key!=Key::Space)))) {
            self.sql_completion.popup = None;
        }
        false
    }
    fn apply_sql_completion(&mut self) {
        let Some(popup) = self.sql_completion.popup.take() else {
            return;
        };
        let Some(doc) = self.service.active_document() else {
            return;
        };
        if doc.id != popup.document
            || doc.version != popup.version
            || self.service.remote_db.session_generation != popup.generation
            || self
                .service
                .remote_db
                .active
                .as_ref()
                .is_none_or(|c| source(c) != popup.source)
        {
            return;
        }
        let before = self
            .service
            .active_view()
            .and_then(|id| self.service.view(id))
            .map_or_else(|| doc.selection(), |v| v.selection());
        if !before.is_caret() || before.head != popup.caret {
            return;
        }
        let Some(item) = popup.items.get(popup.selected) else {
            return;
        };
        let after = Selection::caret(popup.context.range.start + item.insert.chars().count());
        self.send(Command::EditTransaction {
            id: popup.document,
            expected_version: popup.version,
            delta: TextDelta {
                range: popup.context.range,
                insert: item.insert.clone(),
            },
            kind: EditKind::Atomic,
            before,
            after,
        });
        self.focus_editor = true;
    }
    pub(super) fn update_sql_completion(&mut self, ctx: &egui::Context) {
        let typing = self.editor_focused(ctx) && ctx.input(|i| i.events.iter().any(|e| {
            matches!(e, egui::Event::Text(text) if text.chars().last().is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$')))
        }));
        if self.path_dialog.is_some()
            || self.database_ui.dialog
            || self.preferences_dialog
            || self.confirm.is_some()
        {
            self.sql_completion.popup = None;
            return;
        }
        if typing && self.sql_completion.popup.is_none() {
            self.request_sql_completion();
        }
        let Some(popup) = self.sql_completion.popup.as_mut() else {
            return;
        };
        let Some(doc) = self.service.active_document() else {
            self.sql_completion.popup = None;
            return;
        };
        let Some(config) = self.service.remote_db.active.as_ref() else {
            self.sql_completion.popup = None;
            return;
        };
        let selection = self
            .service
            .active_view()
            .and_then(|id| self.service.view(id))
            .map_or_else(|| doc.selection(), |v| v.selection());
        if doc.id != popup.document
            || source(config) != popup.source
            || self.service.remote_db.session_generation != popup.generation
            || !selection.is_caret()
            || self
                .service
                .remote_db
                .console_bindings
                .get(&doc.path)
                .is_some_and(|id| id != &config.id)
            || (selection.head != popup.caret && !typing)
            || (doc.version != popup.version && !typing)
        {
            self.sql_completion.popup = None;
            return;
        }
        let d = dialect(config);
        if doc.version != popup.version || selection.head != popup.caret {
            let Some(context) = sql::context(doc.text(), selection.head, d) else {
                self.sql_completion.popup = None;
                return;
            };
            popup.requested.retain(|key| !key.starts_with("objects:"));
            popup.context = context;
            popup.version = doc.version;
            popup.caret = selection.head;
            popup.selected = 0;
            popup.scroll_selection = true;
            popup.anchor = self.caret_screen.unwrap_or(popup.anchor);
        }
        let remote = &self.service.remote_db;
        // PostgreSQL public is offered only when it exists. Custom search_path is
        // intentionally not guessed: use a schema qualifier for those objects.
        let default = if config.database.is_empty() || d == SqlDialect::Postgres {
            if d == SqlDialect::Postgres && remote.namespaces.iter().any(|n| n == "public") {
                "public"
            } else {
                ""
            }
        } else {
            &config.database
        };
        // Completion searches the typed prefix independently of the explorer's page/filter.
        let mut suggestion_objects = remote.namespace_objects.clone();
        let mut target_namespaces = vec![];
        if popup.context.relation_position && !default.is_empty() {
            target_namespaces.push(default.to_owned());
        }
        if let Some(ns) = popup
            .context
            .qualifiers
            .first()
            .filter(|ns| remote.namespaces.contains(ns))
        {
            target_namespaces.push(ns.clone());
        }
        for ns in &target_namespaces {
            if let Some(objects) = remote
                .completion_tables
                .get(&(ns.clone(), popup.context.prefix.clone()))
            {
                suggestion_objects.insert(ns.clone(), objects.clone());
            } else {
                suggestion_objects.remove(ns);
            }
        }
        popup.items = sql::suggestions(
            &popup.context,
            d,
            default,
            &remote.namespaces,
            &suggestion_objects,
            &remote.table_columns,
        );
        popup.selected = popup.selected.min(popup.items.len().saturating_sub(1));
        if remote.is_busy() {
            return;
        }
        let mut next = None;
        if !remote.namespaces_loaded && !popup.requested.contains("namespaces") {
            next = Some(("namespaces".to_owned(), RemoteCommand::ListNamespaces));
        } else {
            for ns in target_namespaces {
                let prefix = popup.context.prefix.clone();
                let key = format!("objects:{ns}:{prefix}");
                if !remote
                    .completion_tables
                    .contains_key(&(ns.clone(), prefix.clone()))
                    && !popup.requested.contains(&key)
                {
                    next = Some((
                        key,
                        RemoteCommand::CompleteTables {
                            namespace: ns,
                            prefix,
                        },
                    ));
                    break;
                }
            }
            // A FROM prefix is not a complete relation yet. Fetching columns
            // for every partially typed table would issue needless failing SQL.
            if next.is_none() && !popup.context.relation_position {
                let mut relations = popup.context.relations.clone();
                if popup.context.qualifiers.len() >= 2 {
                    relations.push(sql::Relation {
                        namespace: popup.context.qualifiers[0].clone(),
                        name: popup.context.qualifiers[1].clone(),
                        alias: String::new(),
                    });
                }
                for relation in relations {
                    let ns = sql::relation_namespace(&relation, default);
                    if ns.is_empty() || !remote.namespaces.contains(&ns) {
                        continue;
                    }
                    let key = format!("columns:{ns}:{}", relation.name);
                    if !remote
                        .table_columns
                        .contains_key(&(ns.clone(), relation.name.clone()))
                        && !popup.requested.contains(&key)
                    {
                        next = Some((
                            key,
                            RemoteCommand::InspectColumns(TableRef {
                                namespace: ns,
                                name: relation.name,
                            }),
                        ));
                        break;
                    }
                }
            }
        }
        if let Some((key, command)) = next {
            popup.requested.insert(key);
            if let Err(error) = self.service.dispatch(Command::RemoteDatabase(command)) {
                self.service.remote_db.error = Some(error);
            }
            ctx.request_repaint();
        }
    }
    pub(super) fn sql_completion_window(&mut self, ctx: &egui::Context) {
        let Some(popup) = self.sql_completion.popup.as_ref() else {
            return;
        };
        if popup.rect.is_some_and(|r| {
            ctx.input(|i| {
                i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !r.contains(p))
            })
        }) {
            self.sql_completion.popup = None;
            return;
        }
        let available = ctx.content_rect();
        let rows: Vec<_> = popup
            .items
            .iter()
            .map(|item| {
                paint::Row::new(
                    item,
                    &popup.context,
                    self.service.remote_db.active.as_ref(),
                    &self.service.remote_db.table_columns,
                )
            })
            .collect();
        let font = FontId::monospace(self.tokens.editor_font_size);
        let natural_width = rows
            .iter()
            .zip(&popup.items)
            .map(|(row, item)| row.preferred_width(ctx, item, &font))
            .fold(self.tokens.completion_width, f32::max)
            + 2. * self.tokens.completion_padding as f32
            + 2.
            + ctx.style_of(ctx.theme()).spacing.scroll.allocated_width();
        let width = natural_width.min((available.width() - 8.).max(120.));
        let height = popup.items.len().clamp(1, self.tokens.completion_max_rows) as f32
            * self.tokens.completion_row_height
            + self.tokens.completion_footer_height
            + 2. * self.tokens.completion_padding as f32
            - 1.;
        let anchor = popup.anchor
            + egui::vec2(
                self.tokens.completion_anchor_offset[0],
                self.tokens.completion_anchor_offset[1],
            );
        let x = anchor
            .x
            .min(available.right() - width - 4.)
            .max(available.left() + 4.);
        let y = if anchor.y + height <= available.bottom() {
            anchor.y
        } else {
            (anchor.y - height - self.tokens.editor_line_height).max(available.top() + 4.)
        };
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("codisseu.sql_completion"))
            .order(egui::Order::Foreground)
            .movable(false)
            .sense(egui::Sense::hover())
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(color(self.tokens.bar))
                    .stroke(egui::Stroke::new(1., color(self.tokens.border)))
                    .corner_radius(self.tokens.completion_frame_corner_radius)
                    .inner_margin(egui::Margin {
                        left: self.tokens.completion_padding,
                        right: self.tokens.completion_padding,
                        top: self.tokens.completion_padding,
                        bottom: self.tokens.completion_padding - 1,
                    })
                    .show(ui, |ui| {
                        ui.set_width(width - 2. * self.tokens.completion_padding as f32 - 2.);
                        ui.spacing_mut().item_spacing.y = 0.;
                        if popup.items.is_empty() {
                            ui.label(if self.service.remote_db.is_busy() {
                                "Carregando catálogo…"
                            } else {
                                "Nenhuma sugestão nesta posição"
                            });
                        }
                        egui::ScrollArea::vertical()
                            .animated(false)
                            .id_salt("sql_completion_list")
                            .max_height(
                                self.tokens.completion_max_rows as f32
                                    * self.tokens.completion_row_height,
                            )
                            .codisseu()
                            .show(ui, |ui| {
                                for (index, (item, row)) in
                                    popup.items.iter().zip(&rows).enumerate()
                                {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(
                                            ui.available_width(),
                                            self.tokens.completion_row_height,
                                        ),
                                        egui::Sense::click(),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::accesskit::Role::ListBoxOption,
                                            true,
                                            index == popup.selected,
                                            &item.label,
                                        )
                                    });
                                    if index == popup.selected || response.hovered() {
                                        ui.painter().rect_filled(
                                            rect,
                                            if index == popup.selected {
                                                self.tokens.control_corner_radius
                                            } else {
                                                0
                                            },
                                            color(if index == popup.selected {
                                                self.tokens.selection
                                            } else {
                                                self.tokens.border
                                            }),
                                        );
                                    }
                                    if index == popup.selected && popup.scroll_selection {
                                        response.scroll_to_me(None);
                                    }
                                    row.paint(ui, rect, item, &popup.context.prefix, &self.tokens);
                                    response
                                        .clone()
                                        .on_hover_text(format!("{}\n{}", item.label, item.detail));
                                    if response.clicked() {
                                        chosen = Some(index);
                                    }
                                }
                            });
                        let (footer, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), self.tokens.completion_footer_height),
                            egui::Sense::hover(),
                        );
                        let footer =
                            footer.expand2(egui::vec2(self.tokens.completion_padding as f32, 0.));
                        ui.painter()
                            .rect_filled(footer, 0., color(self.tokens.border));
                        let hint =
                            if self.service.remote_db.is_busy() {
                                "Catálogo…  ·  Esc fechar"
                            } else {
                                if self.service.remote_db.completion_more.iter().any(
                                    |(ns, prefix)| {
                                        *prefix == popup.context.prefix
                                            && (popup.context.qualifiers.first() == Some(ns)
                                                || popup.context.qualifiers.is_empty())
                                    },
                                ) {
                                    "Mais tabelas: refine o nome  ·  Enter inserir"
                                } else {
                                    "Enter / Tab inserir  ·  Esc fechar"
                                }
                            };
                        ui.painter().with_clip_rect(footer).text(
                            footer.left_center() + egui::vec2(8., 0.),
                            egui::Align2::LEFT_CENTER,
                            hint,
                            FontId::proportional(self.tokens.completion_footer_font_size),
                            color(self.tokens.completion_footer_text),
                        );
                    });
            });
        if let Some(popup) = self.sql_completion.popup.as_mut() {
            popup.rect = Some(area.response.rect);
            popup.scroll_selection = false;
        }
        if let Some(index) = chosen {
            self.sql_completion.popup.as_mut().unwrap().selected = index;
            self.apply_sql_completion();
        }
    }
}

#[cfg(test)]
mod visual_tests {
    use super::*;
    use codisseu_core::remote::{DbObject, RemoteColumn};
    use egui_kittest::Harness;
    use std::{collections::BTreeMap, path::PathBuf, thread, time::Instant};

    #[test]
    #[ignore = "explicit wgpu SQL popup widget render; synthetic metadata, no server connection"]
    fn sql_popup_widgets_render_cached_column_pk_and_table_metadata() {
        let output = PathBuf::from(
            std::env::var("CODISSEU_SQL_POPUP_RENDER").expect("explicit artifact directory"),
        );
        std::fs::create_dir_all(&output).unwrap();
        for (name, marked) in [
            ("columns", "SELECT c.| FROM corrier.remetentes c;"),
            ("tables", "SELECT * FROM corrier.|;"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut service = AppService::new(directory.path().join("fixture.json"));
            let deadline = Instant::now() + Duration::from_secs(10);
            while service.is_busy() {
                service.poll();
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(1));
            }
            let config = ConnectionConfig {
                name: "@localhost".into(),
                database: "corrier".into(),
                ..Default::default()
            };
            let columns = BTreeMap::from([(
                ("corrier".into(), "remetentes".into()),
                ["id", "nome", "email"]
                    .into_iter()
                    .map(|name| RemoteColumn {
                        name: name.into(),
                        data_type: if name == "id" { "INTEGER" } else { "TEXT" }.into(),
                        primary_key: name == "id",
                        nullable: name != "id",
                    })
                    .collect(),
            )]);
            let cursor = marked[..marked.find('|').unwrap()].chars().count();
            let context =
                sql::context(&marked.replace('|', ""), cursor, SqlDialect::Mysql).unwrap();
            let objects = BTreeMap::from([(
                "corrier".into(),
                vec![DbObject {
                    namespace: "corrier".into(),
                    name: "remetentes".into(),
                    kind: "table".into(),
                    columns: vec![],
                }],
            )]);
            let items = sql::suggestions(
                &context,
                SqlDialect::Mysql,
                "corrier",
                &["corrier".into()],
                &objects,
                &columns,
            );
            assert!(!items.is_empty());
            // This is a renderer fixture only: no Connect command or catalog
            // request is sent, and the app's normal update path is not called.
            service.remote_db.active = Some(config.clone());
            service.remote_db.table_columns = columns;
            let mut app = CodisseuApp::new(service);
            app.sql_completion.popup = Some(Popup {
                document: 0,
                version: 0,
                caret: cursor,
                source: source(&config),
                generation: 0,
                context,
                items,
                requested: BTreeSet::new(),
                selected: 0,
                scroll_selection: true,
                anchor: egui::pos2(70., 40.),
                rect: None,
            });
            let mut initialized = false;
            let mut harness = Harness::builder()
                .with_size([640., 240.])
                .with_max_steps(30)
                .build_ui_state(
                    |ui, app| {
                        if !initialized {
                            app.tokens.apply(ui.ctx());
                            initialized = true;
                        }
                        ui.label(
                            "SQL completion widget · synthetic cached metadata · no connection",
                        );
                        app.sql_completion_window(ui.ctx());
                    },
                    app,
                );
            harness.run();
            harness
                .render()
                .expect("real wgpu widget render")
                .save(output.join(format!("sql-popup-{name}-widget.png")))
                .unwrap();
        }
    }
}
