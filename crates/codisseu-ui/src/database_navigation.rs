//! Query navigation uses the executed SQL snapshot, independently of editor changes.
use super::*;
use codisseu_core::query_page::{ColumnSort, MAX_PAGE_OFFSET, QueryPageRequest};
use codisseu_core::{DbCommand, remote};

impl database::DatabaseUi {
    pub(super) fn accept_query_page(&mut self, request: QueryPageRequest, remote: bool) {
        let (page, starts) = if remote {
            (&mut self.remote_page, &mut self.remote_page_starts)
        } else {
            (&mut self.sqlite_page, &mut self.sqlite_page_starts)
        };
        if request.offset == 0 {
            starts.clear();
        } else if let Some(previous) = page.as_ref() {
            if request.offset > previous.offset {
                starts.push(previous.offset);
            } else {
                starts.retain(|offset| *offset < request.offset);
            }
        }
        *page = Some(request);
    }
}

impl CodisseuApp {
    fn displayed_query_page(&self) -> Option<&QueryPageRequest> {
        if self.database_ui.remote_results {
            self.database_ui.remote_page.as_ref()
        } else {
            self.database_ui.sqlite_page.as_ref()
        }
    }

    pub(super) fn query_page_busy(&self) -> bool {
        if self.database_ui.remote_results {
            self.service.remote_db.is_busy()
                || !self.database_ui.staged.is_empty()
                || self.database_ui.edit.is_some()
        } else {
            self.service.db.is_busy()
                || !self.database_ui.staged.is_empty()
                || self.database_ui.edit.is_some()
        }
    }

    pub(super) fn navigate_query_page(&mut self, request: QueryPageRequest) {
        if self.query_page_busy() {
            return;
        }
        let result = if self.database_ui.remote_results {
            let Some(config) = self.service.remote_db.active.as_ref() else {
                return;
            };
            self.service
                .dispatch(Command::RemoteDatabase(remote::RemoteCommand::ExecutePage(
                    codisseu_core::query_page::BoundQueryPage {
                        datasource_id: config.id.clone(),
                        connection_generation: self.service.remote_db.connection_generation,
                        payload: request.clone(),
                    },
                )))
        } else {
            self.service
                .dispatch(Command::Database(DbCommand::ExecutePage(request.clone())))
        };
        match result {
            Ok(()) => {
                if self.database_ui.remote_results {
                    self.database_ui.remote_pending_page = Some(request);
                } else {
                    self.database_ui.sqlite_pending_page = Some(request);
                }
                self.database_ui.output = false;
                self.grid_selection = Default::default();
                self.grid_focus = None;
            }
            Err(error) => {
                if self.database_ui.remote_results {
                    self.service.remote_db.error = Some(error);
                } else {
                    self.service.db.last_error = Some(error);
                }
            }
        }
    }

    pub(super) fn sort_query_column(&mut self, column: usize) {
        let Some(mut request) = self.displayed_query_page().cloned() else {
            return;
        };
        request.sort = match request.sort {
            Some(sort) if sort.column == column && sort.descending => None,
            Some(sort) if sort.column == column => Some(ColumnSort {
                column,
                descending: true,
            }),
            _ => Some(ColumnSort {
                column,
                descending: false,
            }),
        };
        request.offset = 0;
        self.navigate_query_page(request);
    }

    pub(super) fn query_sort_state(&self) -> Option<Option<ColumnSort>> {
        self.displayed_query_page().map(|page| page.sort)
    }

    pub(super) fn query_navigation_toolbar(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.displayed_query_page().cloned() else {
            return;
        };
        let (rows, more) = if self.database_ui.remote_results {
            self.service
                .remote_db
                .result
                .as_ref()
                .map_or((0, false), |r| (r.rows.len(), r.truncated))
        } else {
            self.service
                .db
                .result
                .as_ref()
                .map_or((0, false), |r| (r.rows.len(), r.truncated))
        };
        let busy = self.query_page_busy();
        let page_size = if self.database_ui.remote_results {
            self.service
                .remote_db
                .active
                .as_ref()
                .map_or(500, |c| c.row_limit) as u64
        } else {
            500
        };
        let previous = if self.database_ui.remote_results {
            self.database_ui.remote_page_starts.last()
        } else {
            self.database_ui.sqlite_page_starts.last()
        }
        .copied()
        .unwrap_or_else(|| page.offset.saturating_sub(page_size));
        let mut offset = None;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.;
            if components::database_icon_button(ui, Icon::Refresh, "Recarregar página", !busy)
                .clicked()
            {
                offset = Some(page.offset);
            }
            if components::database_icon_button(
                ui,
                Icon::FirstPage,
                "Primeira página",
                !busy && page.offset > 0,
            )
            .clicked()
            {
                offset = Some(0);
            }
            if components::database_icon_button(
                ui,
                Icon::Previous,
                "Página anterior",
                !busy && page.offset > 0,
            )
            .on_hover_text("Ctrl+Alt+↑")
            .clicked()
            {
                offset = Some(previous);
            }
            let next = page.offset.saturating_add(rows as u64);
            if components::database_icon_button(
                ui,
                Icon::Next,
                "Próxima página",
                !busy && more && rows > 0 && next <= MAX_PAGE_OFFSET,
            )
            .on_hover_text("Ctrl+Alt+↓")
            .clicked()
            {
                offset = Some(next);
            }
            ui.add_space(6.);
            let start = if rows == 0 { 0 } else { page.offset + 1 };
            let end = if rows == 0 {
                0
            } else {
                page.offset + rows as u64
            };
            ui.label(format!("{start}–{end}{}", if more { "+" } else { "" }))
                .on_hover_text("Linhas desta página · total não contado");
        });
        if !busy
            && ui
                .ctx()
                .input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::ALT, Key::ArrowUp))
            && page.offset > 0
        {
            offset = Some(previous);
        }
        if !busy
            && ui
                .ctx()
                .input_mut(|i| i.consume_key(Modifiers::CTRL | Modifiers::ALT, Key::ArrowDown))
            && more
            && rows > 0
            && page.offset + rows as u64 <= MAX_PAGE_OFFSET
        {
            offset = Some(page.offset + rows as u64);
        }
        if let Some(offset) = offset {
            self.navigate_query_page(QueryPageRequest { offset, ..page });
        }
    }
}

/// Original vector affordance, matching the compact paired arrows of Database Tools.
pub(super) fn sort_header(
    ui: &egui::Ui,
    response: egui::Response,
    column: usize,
    name: &str,
    state: Option<Option<ColumnSort>>,
    available: bool,
    tokens: &Tokens,
) -> egui::Response {
    let Some(state) = state else {
        return response;
    };
    let sort = state.filter(|sort| sort.column == column);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            available,
            format!("Ordenar por {name}"),
        )
    });
    let x = response.rect.right() - 9.;
    let y = response.rect.center().y;
    for (descending, center) in [(false, y - 3.), (true, y + 3.)] {
        let tip = if descending { center + 2. } else { center - 2. };
        let base = if descending {
            center - 1.5
        } else {
            center + 1.5
        };
        let active = sort.is_some_and(|sort| sort.descending == descending) && available;
        ui.painter().add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x, tip),
                egui::pos2(x - 3., base),
                egui::pos2(x + 3., base),
            ],
            color(if active {
                tokens.database_grid_header_foreground
            } else {
                tokens.muted
            }),
            egui::Stroke::NONE,
        ));
    }
    response.on_hover_text(match sort {
        Some(sort) if sort.descending => "Restaurar ordem da consulta",
        Some(_) => "Ordenar decrescente · conjunto completo",
        None => "Ordenar crescente · conjunto completo",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    struct OwnedSourceCleanup {
        session: PathBuf,
        id: String,
    }
    impl Drop for OwnedSourceCleanup {
        fn drop(&mut self) {
            // Only the synthetic profile created by this test is ever loaded/deleted.
            let mut service =
                remote::RemoteService::new(self.session.with_extension("datasources.json"));
            let deadline = std::time::Instant::now() + Duration::from_secs(12);
            while service.is_busy() && std::time::Instant::now() < deadline {
                service.poll();
                std::thread::sleep(Duration::from_millis(2));
            }
            if service
                .dispatch(remote::RemoteCommand::DeleteRemembered(self.id.clone()))
                .is_ok()
            {
                while service.is_busy() && std::time::Instant::now() < deadline {
                    service.poll();
                    std::thread::sleep(Duration::from_millis(2));
                }
                if service.error.is_some() || service.is_busy() {
                    eprintln!("Owned synthetic credential cleanup failed");
                }
            }
        }
    }

    fn settle(h: &mut Harness<'_, CodisseuApp>) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            h.step();
            if !h.state().service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        h.run();
    }
    fn first(h: &Harness<'_, CodisseuApp>) -> i64 {
        codisseu_core::display_value(&h.state().service.db.result.as_ref().unwrap().rows[0][0])
            .parse()
            .unwrap()
    }
    #[test]
    fn query_navigation_header_and_toolbar_use_executed_snapshot_and_full_result() {
        let dir = tempfile::tempdir().unwrap();
        let database = dir.path().join("fixture.sqlite");
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, label TEXT); WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1205) INSERT INTO items SELECT x, printf('item-%04d',x) FROM n;").unwrap();
        drop(db);
        let file = dir.path().join("query.sql");
        let sql = "SELECT id, label FROM items ORDER BY id;";
        std::fs::write(&file, sql).unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        while app.service.is_busy() {
            app.service.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        app.service
            .dispatch(Command::OpenFile(file.clone()))
            .unwrap();
        app.service
            .dispatch(Command::Database(DbCommand::Connect(database)))
            .unwrap();
        app.show_project = false;
        app.show_database = false;
        app.show_activity = true;
        app.tool_tab = ToolTab::Results;
        let mut h = Harness::builder()
            .with_pixels_per_point(
                std::env::var("CODISSEU_QUERY_NAV_SCALE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.),
            )
            .with_size([1600., 1000.])
            .with_max_steps(60)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        h.state_mut().execute_sql();
        settle(&mut h);
        assert_eq!(first(&h), 1);
        assert_eq!(
            h.state().service.db.result.as_ref().unwrap().rows.len(),
            500
        );
        let query_ack = h.state().database_ui.sqlite_query_ack_revision;
        let header = h.get_by_label("Ordenar por id").rect();
        let next = h.get_by_label("Próxima página").rect();
        let history = h.get_by_label("Histórico de consultas").rect();
        assert!((next.center().y - history.center().y).abs() < 2.);
        assert!(next.left() > history.right());
        assert!(
            next.bottom() <= header.top(),
            "navigation is above the grid"
        );
        h.get_by_label("Próxima página").click();
        settle(&mut h);
        assert_eq!(first(&h), 501);
        h.get_by_label("Próxima página").click();
        settle(&mut h);
        assert_eq!(first(&h), 1001);
        assert_eq!(
            h.state().service.db.result.as_ref().unwrap().rows.len(),
            205
        );
        assert!(!h.state().service.db.result.as_ref().unwrap().truncated);
        h.get_by_label("Página anterior").click();
        settle(&mut h);
        assert_eq!(first(&h), 501);
        h.get_by_label("Ordenar por id").click();
        settle(&mut h);
        assert_eq!(first(&h), 1);
        h.get_by_label("Ordenar por id").click();
        settle(&mut h);
        assert_eq!(
            first(&h),
            1205,
            "sorting includes rows outside the old page"
        );
        assert_eq!(
            h.state().database_ui.sqlite_page.as_ref().unwrap().offset,
            0
        );
        assert_eq!(h.state().result_query(), Some(sql));
        assert_eq!(
            h.state().database_ui.sqlite_query_ack_revision,
            query_ack,
            "paging does not publish a new history entry"
        );
        if let Ok(out) = std::env::var("CODISSEU_QUERY_NAV_RENDER") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("query-sorted.png"))
                .unwrap();
        }
        h.get_by_label("Ordenar por id").click();
        settle(&mut h);
        assert_eq!(first(&h), 1);
        assert!(
            h.state()
                .database_ui
                .sqlite_page
                .as_ref()
                .unwrap()
                .sort
                .is_none()
        );
        h.state_mut().begin_query_export();
        // Export dialog takes the original executed query; page wrappers never replace it.
        assert_eq!(h.state().result_query(), Some(sql));
        assert_eq!(std::fs::read_to_string(file).unwrap(), sql);
    }

    #[test]
    fn page_starts_remember_short_payload_limited_pages() {
        let mut ui = database::DatabaseUi::default();
        for offset in [0, 67, 134, 201] {
            ui.accept_query_page(
                QueryPageRequest {
                    offset,
                    ..QueryPageRequest::first("SELECT 1".into())
                },
                false,
            );
        }
        assert_eq!(ui.sqlite_page_starts, vec![0, 67, 134]);
        ui.accept_query_page(
            QueryPageRequest {
                offset: 134,
                ..QueryPageRequest::first("SELECT 1".into())
            },
            false,
        );
        assert_eq!(ui.sqlite_page_starts, vec![0, 67]);
        ui.accept_query_page(QueryPageRequest::first("SELECT 1".into()), false);
        assert!(ui.sqlite_page_starts.is_empty());
    }

    #[test]
    #[ignore = "requires owned MySQL and desktop vault: CODISSEU_TEST_SYSTEM_KEYRING=1 CODISSEU_AUTOCONNECT_MY_PORT"]
    fn saved_query_after_restart_connects_its_source_and_keeps_the_executed_snapshot() {
        assert_eq!(std::env::var("CODISSEU_TEST_SYSTEM_KEYRING").unwrap(), "1");
        let dir = tempfile::tempdir().unwrap();
        let session = dir.path().join("session.json");
        let id = format!(
            "owned-ui-console-{}-{}",
            std::process::id(),
            dir.path().file_name().unwrap().to_string_lossy()
        );
        let _cleanup = OwnedSourceCleanup {
            session: session.clone(),
            id: id.clone(),
        };
        let app = CodisseuApp::new(AppService::new(session.clone()));
        let mut h = Harness::builder()
            .with_size([1600., 1000.])
            .with_max_steps(60)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(
                remote::RemoteCommand::SaveRemembered {
                    config: remote::ConnectionConfig {
                        id: id.clone(),
                        name: "SQL QA · fixture".into(),
                        database: "codisseu".into(),
                        host: "127.0.0.1".into(),
                        port: std::env::var("CODISSEU_AUTOCONNECT_MY_PORT")
                            .unwrap()
                            .parse()
                            .unwrap(),
                        username: "root".into(),
                        tls: false,
                        timeout_secs: 3,
                        save_credentials: true,
                        ..Default::default()
                    },
                    password: "codisseu_fixture".into(),
                },
            ))
            .unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(
                remote::RemoteCommand::CreateConsoleFor(id.clone()),
            ))
            .unwrap();
        settle(&mut h);
        let file = h.state().service.active_document().unwrap().path.clone();
        drop(h);
        let sql = "SELECT DATABASE() AS source, 123 AS value;";
        std::fs::write(&file, sql).unwrap();
        let mut app = CodisseuApp::new(AppService::new(session));
        app.show_project = false;
        app.show_activity = false;
        let mut h = Harness::builder()
            .with_size([1600., 1000.])
            .with_max_steps(60)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(file.clone()))
            .unwrap();
        settle(&mut h);
        assert!(h.state().service.remote_db.active.is_none());
        h.state_mut().execute_sql();
        // Change the editor while Connect + Execute is still queued; the request
        // must retain its SQL/source, without replacing the user's new buffer.
        let doc = h.state().service.active_document().unwrap();
        let command = Command::EditTransaction {
            id: doc.id,
            expected_version: doc.version,
            delta: codisseu_core::TextDelta {
                range: 0..doc.len_chars(),
                insert: "SELECT 999;".into(),
            },
            kind: codisseu_core::EditKind::Atomic,
            before: doc.selection(),
            after: codisseu_core::Selection::caret(0),
        };
        h.state_mut().service.dispatch(command).unwrap();
        settle(&mut h);
        let app = h.state();
        assert!(
            app.service.remote_db.error.is_none(),
            "{:?}",
            app.service.remote_db.error
        );
        assert_eq!(app.service.remote_db.active.as_ref().unwrap().id, id);
        assert_eq!(
            app.service.remote_db.result.as_ref().unwrap().rows[0],
            vec![
                remote::Cell::Text("codisseu".into()),
                remote::Cell::Number("123".into())
            ]
        );
        assert_eq!(app.database_ui.result_source.as_ref(), Some(&file));
        assert_eq!(app.result_query(), Some(sql));
        assert_eq!(app.service.active_document().unwrap().text(), "SELECT 999;");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), sql);
        let header = h.get_by_label("Ordenar por source").rect();
        let next = h.get_by_label("Próxima página").rect();
        assert!(next.bottom() <= header.top());
        if let Ok(out) = std::env::var("CODISSEU_QUERY_NAV_RENDER") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("saved-query-autoconnect.png"))
                .unwrap();
        }
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(
                remote::RemoteCommand::DeleteRemembered(id),
            ))
            .unwrap();
        settle(&mut h);
        assert!(h.state().service.remote_db.error.is_none());
        assert!(h.state().service.remote_db.profiles.is_empty());
    }

    #[test]
    #[ignore = "requires task-owned MySQL on explicit CODISSEU_IDLE_MY_PORT"]
    fn remote_query_after_idle_resumes_through_the_same_editor_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("query.sql");
        let sql = "SELECT CONNECTION_ID() AS session_id";
        std::fs::write(&file, sql).unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("session.json")));
        while app.service.is_busy() {
            app.service.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        app.service
            .dispatch(Command::OpenFile(file.clone()))
            .unwrap();
        app.show_project = false;
        let mut h = Harness::builder()
            .with_size([1400., 900.])
            .with_max_steps(60)
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        let config = remote::ConnectionConfig {
            id: "navigation-idle-ui".into(),
            name: "Disposable MySQL".into(),
            host: "127.0.0.1".into(),
            port: std::env::var("CODISSEU_IDLE_MY_PORT")
                .unwrap()
                .parse()
                .unwrap(),
            database: "codisseu".into(),
            username: "root".into(),
            tls: false,
            read_only: true,
            timeout_secs: 3,
            idle_secs: 5,
            save_credentials: false,
            ..Default::default()
        };
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(remote::RemoteCommand::Connect {
                config: config.clone(),
                password: "codisseu_fixture".into(),
                test: false,
            }))
            .unwrap();
        settle(&mut h);
        h.state_mut().execute_remote();
        settle(&mut h);
        let old = h.state().service.remote_db.result.as_ref().unwrap().rows[0][0].clone();
        let generation = h.state().service.remote_db.connection_generation;
        std::thread::sleep(Duration::from_millis(5400));
        // Execute directly before a GUI poll, reproducing the expiry boundary.
        h.state_mut().execute_remote();
        settle(&mut h);
        let app = h.state();
        assert!(
            app.service.remote_db.error.is_none(),
            "{:?}",
            app.service.remote_db.error
        );
        assert_eq!(app.service.remote_db.active.as_ref().unwrap().id, config.id);
        assert_eq!(app.service.remote_db.connection_generation, generation);
        assert!(!app.service.remote_db.suspended);
        assert_ne!(
            app.service.remote_db.result.as_ref().unwrap().rows[0][0],
            old
        );
        assert_eq!(app.database_ui.result_sql.as_deref(), Some(sql));
        assert_eq!(app.database_ui.result_source.as_ref(), Some(&file));
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(remote::RemoteCommand::Disconnect))
            .unwrap();
        settle(&mut h);
    }
}
