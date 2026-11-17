//! Saved query catalog, available independently of a live datasource or workspace.
use super::*;
use codisseu_core::remote::{ConnectionConfig, RemoteCommand};
pub(super) struct QueryLibrary {
    expanded: bool,
    collapsed_sources: std::collections::BTreeSet<String>,
    rename: Option<Rename>,
    delete: Option<ConnectionConfig>,
    delete_query: Option<DeleteQuery>,
}

struct DeleteQuery {
    path: PathBuf,
    submitted: bool,
    error: Option<String>,
}
struct Rename {
    path: PathBuf,
    name: String,
    focus: bool,
    submitted: Option<PathBuf>,
    error: Option<String>,
}
impl Default for QueryLibrary {
    fn default() -> Self {
        Self {
            expanded: true,
            collapsed_sources: Default::default(),
            rename: None,
            delete: None,
            delete_query: None,
        }
    }
}
fn query_source_caption(service: &codisseu_core::remote::RemoteService, id: &str) -> String {
    let profiles = &service.profiles;
    if let Some(config) = profiles.iter().find(|config| config.id == id) {
        if profiles
            .iter()
            .filter(|other| other.name == config.name)
            .count()
            > 1
        {
            let location = format!(
                "{} · {}@{}:{}",
                config.name, config.database, config.host, config.port
            );
            let mut same: Vec<_> = profiles
                .iter()
                .filter(|other| {
                    other.name == config.name
                        && other.database == config.database
                        && other.host == config.host
                        && other.port == config.port
                })
                .map(|other| &other.id)
                .collect();
            same.sort();
            if same.len() > 1 {
                return format!(
                    "{location} · {}",
                    same.iter()
                        .position(|other| other.as_str() == id)
                        .unwrap_or(0)
                        + 1
                );
            }
            return location;
        }
        return config.name.clone();
    }
    format!("Conexão removida · {id}")
}
pub(super) fn query_engine(
    service: &codisseu_core::remote::RemoteService,
    path: &std::path::Path,
) -> Option<codisseu_core::remote::Engine> {
    let id = service.console_bindings.get(path)?;
    service
        .profiles
        .iter()
        .find(|config| config.id == *id)
        .map(|config| config.engine)
}
pub(super) fn query_label(
    service: &codisseu_core::remote::RemoteService,
    path: &std::path::Path,
    tab: bool,
) -> String {
    let name = if tab
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sql"))
    {
        path.file_stem()
    } else {
        path.file_name()
    }
    .unwrap_or_default()
    .to_string_lossy();
    match service.console_bindings.get(path) {
        Some(id) => format!("{name} [{}]", query_source_caption(service, id)),
        // Unbound files keep the ordinary editor's filename, including extension.
        None => path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

impl CodisseuApp {
    pub(super) fn query_library_modal_open(&self) -> bool {
        self.query_library.rename.is_some()
            || self.query_library.delete.is_some()
            || self.query_library.delete_query.is_some()
    }
    pub(super) fn rename_query_dialog(&mut self, path: PathBuf) {
        let name = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.query_library.rename = Some(Rename {
            path,
            name,
            focus: true,
            submitted: None,
            error: None,
        });
    }
    pub(super) fn delete_query_dialog(&mut self, path: PathBuf) {
        self.query_library.delete_query = Some(DeleteQuery {
            path,
            submitted: false,
            error: None,
        });
    }
    pub(super) fn delete_datasource_dialog(&mut self, config: ConnectionConfig) {
        self.query_library.delete = Some(config);
    }
    pub(super) fn query_file_label(&self, path: &std::path::Path) -> String {
        query_label(&self.service.remote_db, path, false)
    }
    pub(super) fn saved_queries_tree(&mut self, ui: &mut egui::Ui) {
        let mut groups = std::collections::BTreeMap::<String, Vec<PathBuf>>::new();
        for (path, id) in &self.service.remote_db.console_bindings {
            groups.entry(id.clone()).or_default().push(path.clone());
        }
        if groups.is_empty() {
            return;
        }
        let row = components::database_object_row(
            ui,
            egui::Id::unique("database.queries"),
            components::TreeObjectRow {
                name: "Queries",
                detail: "",
                description: "Queries salvas por conexão",
                depth: 0,
                expanded: Some(self.query_library.expanded),
                selected: false,
                icon: Icon::Folder,
            },
            color(self.tokens.text),
            color(self.tokens.muted),
        );
        if row.clicked() {
            self.query_library.expanded = !self.query_library.expanded;
        }
        if !self.query_library.expanded {
            return;
        }
        let mut groups: Vec<_> = groups
            .into_iter()
            .map(|(id, files)| {
                (
                    query_source_caption(&self.service.remote_db, &id),
                    id,
                    files,
                )
            })
            .collect();
        groups.sort_by(|a, b| {
            a.0.to_lowercase()
                .cmp(&b.0.to_lowercase())
                .then(a.1.cmp(&b.1))
        });
        for (caption, id, mut files) in groups {
            let expanded = !self.query_library.collapsed_sources.contains(&id);
            let row = components::database_query_row(
                ui,
                egui::Id::unique(("database.queries.source", &id)),
                components::TreeObjectRow {
                    name: &caption,
                    detail: "",
                    description: "Queries desta conexão",
                    depth: 1,
                    expanded: Some(expanded),
                    selected: false,
                    icon: Icon::Folder,
                },
                self.service
                    .remote_db
                    .profiles
                    .iter()
                    .find(|config| config.id == id)
                    .map(|config| config.engine),
                &self.tokens,
            );
            if row.clicked() {
                if expanded {
                    self.query_library.collapsed_sources.insert(id.clone());
                } else {
                    self.query_library.collapsed_sources.remove(&id);
                }
            }
            if !expanded {
                continue;
            }
            files.sort_by(|a, b| a.file_name().cmp(&b.file_name()).then(a.cmp(b)));
            for path in files {
                self.saved_query_row(ui, path, &caption);
            }
        }
    }
    fn saved_query_row(&mut self, ui: &mut egui::Ui, path: PathBuf, source: &str) {
        let label = self.query_file_label(&path);
        let selected = self
            .service
            .active_document()
            .is_some_and(|doc| doc.path == path);
        let description = format!("{source} · duplo clique para abrir · F2 para renomear");
        let row = components::database_query_row(
            ui,
            egui::Id::unique(("database.query", &path)),
            components::TreeObjectRow {
                name: &path.file_name().unwrap_or_default().to_string_lossy(),
                detail: &format!("[{source}]"),
                description: &description,
                depth: 2,
                expanded: None,
                selected,
                icon: Icon::Code,
            },
            query_engine(&self.service.remote_db, &path),
            &self.tokens,
        )
        .on_hover_text(self.query_file_label(&path));
        row.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::TreeItem, ui.is_enabled(), &label)
        });
        if row.clicked() {
            row.request_focus();
        }
        if row.double_clicked()
            || (row.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)))
        {
            self.send(Command::OpenFile(path.clone()));
            self.focus_editor = true;
        }
        if row.has_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F2)) {
            self.rename_query_dialog(path.clone());
        }
        row.context_menu(|ui| {
            if ui.button("Abrir query").clicked() {
                self.send(Command::OpenFile(path.clone()));
                self.focus_editor = true;
                ui.close();
            }
            if ui.button("Renomear query…").clicked() {
                self.rename_query_dialog(path.clone());
                ui.close();
            }
            if ui.button("Excluir query…").clicked() {
                self.delete_query_dialog(path.clone());
                ui.close();
            }
        });
    }
    pub(super) fn query_library_windows(&mut self, ctx: &egui::Context) {
        if let Some(state) = &self.query_library.rename
            && let Some(target) = &state.submitted
            && !self.service.remote_db.is_busy()
        {
            if self.service.remote_db.console_bindings.contains_key(target)
                && (self.service.remote_db.error.is_none()
                    || !self
                        .service
                        .remote_db
                        .console_bindings
                        .contains_key(&state.path))
            {
                let old = state.path.clone();
                let new = target.clone();
                if self.database_ui.result_source.as_ref() == Some(&old) {
                    self.database_ui.result_source = Some(new.clone());
                }
                if self.database_ui.sqlite_result_source.as_ref() == Some(&old) {
                    self.database_ui.sqlite_result_source = Some(new);
                }
                self.query_library.rename = None;
                self.focus_editor = true;
            } else if let Some(state) = &mut self.query_library.rename {
                state.submitted = None;
                state.error = self.service.remote_db.error.clone();
            }
        }
        if self.query_library.rename.is_some() {
            let query_caption =
                self.query_file_label(&self.query_library.rename.as_ref().unwrap().path);
            let mut close = false;
            let mut apply = false;
            egui::Modal::new(egui::Id::unique("database.rename.query"))
                .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16))
                .show(ctx, |ui| {
                    ui.set_width(400.);
                    self.tokens.form_style(ui);
                    ui.heading("Renomear query");
                    ui.label(&query_caption);
                    let state = self.query_library.rename.as_mut().unwrap();
                    let busy = state.submitted.is_some();
                    let label = ui.label("Nome da query:");
                    let input = ui
                        .add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut state.name)
                                .desired_width(f32::INFINITY)
                                .char_limit(200),
                        )
                        .labelled_by(label.id);
                    if state.focus {
                        input.request_focus();
                        state.focus = false;
                    }
                    if let Some(error) = &state.error {
                        crate::status_messages::status_message(ui, error);
                    }
                    ui.horizontal(|ui| {
                        apply = ui
                            .add_enabled(!busy, self.tokens.dialog_button("Renomear", true))
                            .clicked();
                        close = ui
                            .add_enabled(!busy, self.tokens.dialog_button("Cancelar", false))
                            .clicked();
                    });
                    apply |= !busy && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                    close |= !busy && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                });
            if apply {
                let state = self.query_library.rename.as_mut().unwrap();
                let target = self
                    .service
                    .remote_db
                    .console_rename_target(&state.path, &state.name);
                match target.and_then(|target| {
                    self.service
                        .dispatch(Command::RemoteDatabase(RemoteCommand::RenameConsole {
                            path: state.path.clone(),
                            name: state.name.clone(),
                        }))
                        .map(|_| target)
                }) {
                    Ok(target) => {
                        state.submitted = Some(target);
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            if close {
                self.query_library.rename = None;
                self.focus_editor = true;
            }
        }
        if self
            .query_library
            .delete_query
            .as_ref()
            .is_some_and(|state| state.submitted)
            && !self.service.remote_db.is_busy()
        {
            let state = self.query_library.delete_query.as_mut().unwrap();
            if !self
                .service
                .remote_db
                .console_bindings
                .contains_key(&state.path)
            {
                if self.database_ui.result_source.as_ref() == Some(&state.path) {
                    self.database_ui.result_source = None;
                }
                if self.database_ui.sqlite_result_source.as_ref() == Some(&state.path) {
                    self.database_ui.sqlite_result_source = None;
                }
                self.query_library.delete_query = None;
            } else {
                state.submitted = false;
                state.error = self.service.remote_db.error.clone();
            }
        }
        if self.query_library.delete_query.is_some() {
            let query_caption =
                self.query_file_label(&self.query_library.delete_query.as_ref().unwrap().path);
            let mut close = false;
            let mut apply = false;
            egui::Modal::new(egui::Id::unique("database.delete.query"))
                .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16))
                .show(ctx, |ui| {
                    ui.set_width(400.);
                    self.tokens.form_style(ui);
                    ui.heading("Excluir query");
                    let state = self.query_library.delete_query.as_ref().unwrap();
                    ui.label(&query_caption);
                    ui.label("O arquivo será excluído permanentemente. Os dados do banco serão preservados.");
                    if let Some(error) = &state.error { crate::status_messages::status_message(ui, error); }
                    ui.horizontal(|ui| {
                        apply = ui.add_enabled(!state.submitted, self.tokens.dialog_button("Excluir", true)).clicked();
                        close = ui.add_enabled(!state.submitted, self.tokens.dialog_button("Cancelar", false)).clicked();
                    });
                    close |= !state.submitted && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                });
            if apply {
                let state = self.query_library.delete_query.as_mut().unwrap();
                match self
                    .service
                    .dispatch(Command::RemoteDatabase(RemoteCommand::DeleteConsole(
                        state.path.clone(),
                    ))) {
                    Ok(()) => {
                        state.submitted = true;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            if close {
                self.query_library.delete_query = None;
            }
        }
        if let Some(config) = self.query_library.delete.clone() {
            let mut close = false;
            let mut apply = false;
            egui::Modal::new(egui::Id::unique("database.delete.source"))
                .frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16))
                .show(ctx, |ui| {
                    ui.set_width(400.);
                    self.tokens.form_style(ui);
                    ui.heading("Excluir conexão");
                    ui.label(&config.name);
                    ui.label("As queries salvas e os dados do banco serão preservados.");
                    ui.horizontal(|ui| {
                        apply = ui.add(self.tokens.dialog_button("Excluir", true)).clicked();
                        close = ui
                            .add(self.tokens.dialog_button("Cancelar", false))
                            .clicked();
                    });
                    close |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                });
            if apply {
                self.remote_send(RemoteCommand::DeleteRemembered(config.id));
                close = true;
            }
            if close {
                self.query_library.delete = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    fn settle(h: &mut Harness<'_, CodisseuApp>) {
        let until = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            h.step();
            if !h.state().service.is_busy() {
                break;
            }
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        h.run();
    }
    fn harness() -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
        let dir = tempfile::tempdir().unwrap();
        let mut s = AppService::new(dir.path().join("session.json"));
        let until = std::time::Instant::now() + Duration::from_secs(10);
        while s.is_busy() {
            s.poll();
            assert!(std::time::Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        s.dispatch(Command::RemoteDatabase(RemoteCommand::Save(
            ConnectionConfig {
                id: "fixture".into(),
                name: "Fixture SQL".into(),
                save_credentials: false,
                ..Default::default()
            },
        )))
        .unwrap();
        while s.is_busy() {
            s.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        let h = Harness::builder()
            .with_size([1280., 800.])
            .with_step_dt(1. / 120.)
            .with_max_steps(60)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(s),
            );
        (dir, h)
    }
    #[test]
    fn saved_query_catalog_rename_from_tab_and_delete_source_preserve_query() {
        let (_dir, mut h) = harness();
        settle(&mut h);
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Fixture SQL")
            .click_secondary();
        h.run();
        assert!(h.query_by_label("Excluir…").is_some());
        h.get_by_label("Nova query").click();
        settle(&mut h);
        assert!(h.state().service.remote_db.active.is_none());
        assert_eq!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .path
                .file_name()
                .unwrap(),
            "query-1.sql"
        );
        h.get_by_role_and_label(egui::accesskit::Role::Button, "query-1 [Fixture SQL]")
            .click_secondary();
        h.run();
        h.get_by_label("Renomear query…").click();
        h.run();
        h.get_by_label("Nome da query:").click();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text("Pedidos".into()));
        h.run();
        if let Ok(out) = std::env::var("CODISSEU_QUERY_LIBRARY_RENDER") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("rename-query.png"))
                .unwrap();
        }
        h.get_by_label("Renomear").click();
        settle(&mut h);
        assert_eq!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .path
                .file_name()
                .unwrap(),
            "Pedidos.sql"
        );
        if let Ok(out) = std::env::var("CODISSEU_QUERY_LIBRARY_RENDER") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("queries-library.png"))
                .unwrap();
        }
        h.get_by_label("Fechar Pedidos [Fixture SQL]").click();
        settle(&mut h);
        assert!(h.state().service.documents.is_empty());
        assert_eq!(h.get_all_by_label("Pedidos.sql [Fixture SQL]").count(), 1);
        h.get_by_label("Pedidos.sql [Fixture SQL]")
            .click_secondary();
        h.run();
        assert!(h.query_by_label("Renomear query…").is_some());
        h.get_by_label("Excluir query…").click();
        h.run();
        h.get_by_label("Cancelar").click();
        settle(&mut h);
        assert!(
            h.state()
                .service
                .remote_db
                .console_bindings
                .keys()
                .any(|p| p.file_name().unwrap() == "Pedidos.sql" && p.exists())
        );
        h.get_by_label("Pedidos.sql [Fixture SQL]")
            .click_secondary();
        h.run();
        h.get_by_label("Abrir query").click();
        settle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "SELECT 1;\n"
        );
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Fixture SQL")
            .click_secondary();
        h.run();
        h.get_by_label("Excluir…").click();
        h.run();
        if let Ok(out) = std::env::var("CODISSEU_QUERY_LIBRARY_RENDER") {
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("delete-source.png"))
                .unwrap();
        }
        h.get_by_label("Excluir").click();
        settle(&mut h);
        assert!(h.state().service.remote_db.profiles.is_empty());
        assert!(
            h.state()
                .service
                .remote_db
                .console_bindings
                .keys()
                .any(|p| p.file_name().unwrap() == "Pedidos.sql" && p.exists())
        );
        assert!(h.query_by_label("Queries").is_some());
        assert!(
            h.state()
                .service
                .remote_db
                .console_bindings
                .values()
                .all(|id| id == "fixture")
        );
        h.get_by_role_and_label(
            egui::accesskit::Role::Button,
            "Pedidos [Conexão removida · fixture]",
        )
        .click_secondary();
        h.run();
        assert!(h.query_by_label("Renomear query…").is_some());
        h.get_by_label("Excluir query…").click();
        h.run();
        if let Ok(out) = std::env::var("CODISSEU_QUERY_LIBRARY_RENDER") {
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("delete-query.png"))
                .unwrap();
        }
        let deleted_path = h.state().service.active_document().unwrap().path.clone();
        h.get_by_label("Excluir").click();
        settle(&mut h);
        assert!(!deleted_path.exists());
        assert!(h.state().service.documents.is_empty());
        assert!(h.state().service.remote_db.console_bindings.is_empty());
    }
    #[test]
    fn datasource_groups_tabs_and_menus_keep_identical_query_names_separate() {
        use codisseu_core::remote::Engine;
        let (_dir, mut h) = harness();
        settle(&mut h);
        let mut paths = std::collections::BTreeMap::new();
        for (id, name, engine) in [
            ("dev", "DEV", Engine::Mysql),
            ("qa", "QA", Engine::Mysql),
            ("prod", "PROD", Engine::Postgres),
            ("cache", "Cache", Engine::Redis),
        ] {
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::Save(
                    ConnectionConfig {
                        id: id.into(),
                        name: name.into(),
                        engine,
                        save_credentials: false,
                        ..Default::default()
                    },
                )))
                .unwrap();
            settle(&mut h);
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(
                    id.into(),
                )))
                .unwrap();
            settle(&mut h);
            paths.insert(
                id,
                h.state().service.active_document().unwrap().path.clone(),
            );
        }
        let tree = egui::accesskit::Role::TreeItem;
        for name in ["DEV", "QA", "PROD", "Cache"] {
            assert!(h.query_by_role_and_label(tree, name).is_some());
        }
        for name in ["DEV", "QA", "PROD"] {
            assert!(
                h.query_by_role_and_label(tree, &format!("query-1.sql [{name}]"))
                    .is_some()
            );
        }
        assert!(
            h.query_by_role_and_label(tree, "query-1.txt [Cache]")
                .is_some()
        );
        assert!(h.state().service.remote_db.active.is_none());
        assert_eq!(
            paths
                .values()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
        h.get_by_role_and_label(tree, "QA").click();
        h.run();
        assert!(
            h.query_by_role_and_label(tree, "query-1.sql [QA]")
                .is_none()
        );
        assert!(
            h.query_by_role_and_label(tree, "query-1.sql [DEV]")
                .is_some()
        );
        h.get_by_role_and_label(tree, "QA").click();
        h.run();
        h.get_by_role_and_label(tree, "query-1.sql [DEV]")
            .click_secondary();
        h.run();
        h.get_by_label("Abrir query").click();
        settle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().path,
            paths["dev"]
        );
        assert!(h.state().service.remote_db.active.is_none());
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "query-1 [DEV]")
                .is_some()
        );
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "query-1 [QA]")
                .is_some()
        );
        if let Ok(out) = std::env::var("CODISSEU_QUERY_LIBRARY_RENDER") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join("datasource-groups.png"))
                .unwrap();
        }
        h.get_by_role_and_label(tree, "query-1.sql [QA]")
            .click_secondary();
        h.run();
        h.get_by_label("Excluir query…").click();
        h.run();
        assert!(h.get_all_by_label("query-1.sql [QA]").count() >= 1);
        h.get_by_label("Excluir").click();
        settle(&mut h);
        assert!(!paths["qa"].exists());
        assert!(paths["dev"].exists());
        assert!(paths["prod"].exists());
        assert!(paths["cache"].exists());
        // A profile label edit keeps file identity and updates every visible caption.
        h.state_mut()
            .service
            .dispatch(Command::RemoteDatabase(RemoteCommand::Save(
                ConnectionConfig {
                    id: "dev".into(),
                    name: "DEV novo".into(),
                    save_credentials: false,
                    ..Default::default()
                },
            )))
            .unwrap();
        settle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().path,
            paths["dev"]
        );
        assert!(
            h.query_by_role_and_label(tree, "query-1.sql [DEV novo]")
                .is_some()
        );
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "query-1 [DEV novo]")
                .is_some()
        );
    }
    #[test]
    fn active_datasource_tab_is_revealed_when_long_source_names_overflow_the_strip() {
        let (_dir, mut h) = harness();
        settle(&mut h);
        let mut first = None;
        for name in [
            "ExampleGroup QA",
            "ExampleGroup PROD",
            "Cache Redis",
            "ExampleGroup DEV",
        ] {
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::Save(
                    ConnectionConfig {
                        id: name.into(),
                        name: name.into(),
                        save_credentials: false,
                        ..Default::default()
                    },
                )))
                .unwrap();
            settle(&mut h);
            h.state_mut()
                .service
                .dispatch(Command::RemoteDatabase(RemoteCommand::CreateConsoleFor(
                    name.into(),
                )))
                .unwrap();
            settle(&mut h);
            first.get_or_insert(h.state().service.active_document().unwrap().id);
        }
        let button = egui::accesskit::Role::Button;
        let end = h.get_by_label("Mais ações do grupo").rect().left();
        let active = h
            .get_by_role_and_label(button, "query-1 [ExampleGroup DEV]")
            .rect();
        assert!(
            active.left() > 200. && active.right() <= end + 1.,
            "active query hidden: {active:?} / {end}"
        );
        let view = h.state().service.active_view().unwrap();
        h.state_mut()
            .service
            .dispatch(Command::ActivateTab {
                view,
                id: first.unwrap(),
            })
            .unwrap();
        settle(&mut h);
        let active = h
            .get_by_role_and_label(button, "query-1 [ExampleGroup QA]")
            .rect();
        assert!(
            active.left() > 200. && active.right() <= end + 1.,
            "first query hidden: {active:?}"
        );
    }
    #[test]
    fn connecting_metadata_is_icon_only_and_services_output_is_explicit() {
        let (_dir, mut h) = harness();
        settle(&mut h);
        // Presentation state only, no claim of a SQL server connection.
        let app = h.state_mut();
        app.service.remote_db.active = Some(ConnectionConfig {
            id: "fixture".into(),
            name: "Fixture SQL".into(),
            ..Default::default()
        });
        app.service.remote_db.namespaces_loaded = true;
        app.database_ui.remote_results = true;
        app.show_activity = true;
        app.tool_tab = ToolTab::Results;
        h.run();
        assert!(
            h.query_by_label("Carregue os schemas para navegar.")
                .is_none()
        );
        assert!(h.query_by_label("Operação em andamento…").is_none());
        assert!(!h.state().database_ui.output);
        assert!(h.query_by_label("All databases").is_none());
        h.state_mut().service.remote_db.error = Some("Erro remoto de teste".into());
        h.run();
        assert!(h.get_all_by_label("Erro remoto de teste").count() >= 1);
        h.state_mut().database_ui.remote_results = false;
        h.state_mut().service.db.last_error = Some("Erro SQLite de teste".into());
        h.run();
        assert!(h.query_by_label("Erro SQLite de teste").is_some());
    }
}
