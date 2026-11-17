use super::*;
impl CodisseuApp {
    pub(crate) fn api_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !self
            .api_ui
            .active_in_view(self.service.active_view().unwrap_or(0))
        {
            return false;
        }
        if self.api_ui.transfer.is_some()
            || self.api_ui.rename.is_some()
            || self.api_ui.move_item.is_some()
        {
            return true;
        }
        if ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::Enter)))
            && let Some(ApiTab::Request(id)) = self.api_ui.active.clone()
        {
            self.api_send_request(&id);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::S))) {
            self.api_save();
        }
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::W))) {
            self.api_ui.close_active();
        }
        if ctx.input_mut(|i| {
            i.consume_shortcut(&KeyboardShortcut::new(
                Modifiers::CTRL | Modifiers::SHIFT,
                Key::O,
            ))
        }) {
            self.open_native_folder();
        } else if ctx
            .input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::O)))
        {
            self.open_path(PathAction::File);
        }
        true
    }
    pub(super) fn api_save(&mut self) {
        if let Err(error) = self.service.api.save() {
            self.api_ui.notice = Some(error);
        }
    }
    pub(super) fn api_send_request(&mut self, id: &str) {
        if let Err(error) = self.service.api.send(id, self.api_ui.scripts_enabled) {
            self.api_ui.notice = Some(error);
        }
    }
    pub(super) fn api_copy_curl(&mut self, ctx: &egui::Context, id: &str) {
        if let Some((request, variables)) = self.service.api.workspace.request_context(id) {
            match export_curl(&request, &variables) {
                Ok(text) => ctx.copy_text(text),
                Err(error) => self.api_ui.notice = Some(error),
            }
        }
    }
    pub(super) fn api_export_collection(&mut self, id: &str) {
        if let Some(collection) = self
            .service
            .api
            .workspace
            .collections
            .iter()
            .find(|c| c.id == id)
        {
            let snapshot = persistent_snapshot(&ApiWorkspace {
                collections: vec![collection.clone()],
                ..ApiWorkspace::default()
            });
            match export_collection(&snapshot.collections[0]) {
                Ok(text) => self.api_transfer(true, text),
                Err(error) => self.api_ui.notice = Some(error),
            }
        }
    }
    pub(super) fn api_export_environment(&mut self, id: &str) {
        if let Some(env) = self
            .service
            .api
            .workspace
            .environments
            .iter()
            .find(|e| e.id == id)
        {
            let snapshot = persistent_snapshot(&ApiWorkspace {
                environments: vec![env.clone()],
                ..ApiWorkspace::default()
            });
            match export_environment(&snapshot.environments[0]) {
                Ok(text) => self.api_transfer(true, text),
                Err(error) => self.api_ui.notice = Some(error),
            }
        }
    }
    pub(super) fn api_transfer(&mut self, export: bool, text: String) {
        self.api_ui.transfer = Some(Transfer {
            text,
            path: String::new(),
            export,
            kind: 0,
        });
    }
    pub(super) fn api_response(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Response");
            ui.selectable_value(&mut self.api_ui.response_page, 0, "Body");
            ui.selectable_value(&mut self.api_ui.response_page, 1, "Headers");
            ui.selectable_value(&mut self.api_ui.response_page, 2, "Test Results");
            ui.selectable_value(&mut self.api_ui.response_page, 3, "Console");
            if ui.button("History").clicked() {
                self.api_open_tab(ApiTab::History);
            }
        });
        let history_version = self.service.api.response_revision;
        let Some(result) = self.service.api.responses.get(id) else {
            ui.add_space(24.);
            if self.service.api.running_request.as_deref() == Some(id) {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Sending request…");
                });
            } else {
                ui.weak("Send a request to see its response · Ctrl+Enter");
            }
            return;
        };
        if result.script_result.skipped {
            ui.weak(
                "Request ignorada pelo script de pre-request; nenhum envio HTTP foi realizado.",
            );
            return;
        }
        let response = &result.response;
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Status: {}", response.status));
            ui.label(format!("{} ms", response.elapsed_ms));
            ui.label(format!("{} bytes", response.size));
            if ui.button("Copiar response").clicked() {
                ui.ctx().copy_text(response.text());
            }
        });
        if response.truncated {
            ui.label("Resposta truncada no limite configurado.");
        }
        if let Some(error) = &result.script_error {
            crate::status_messages::status_message(ui, error);
        }
        egui::ScrollArea::both()
            .id_salt(("api.response", id, self.api_ui.response_page))
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| match self.api_ui.response_page {
                0 => {
                    if self
                        .api_ui
                        .response_cache
                        .as_ref()
                        .is_none_or(|(key, version, _)| key != id || *version != history_version)
                    {
                        self.api_ui.response_cache = Some((
                            id.into(),
                            history_version,
                            response.pretty_json().unwrap_or_else(|| response.text()),
                        ));
                    }
                    if let Some((_, _, text)) = &mut self.api_ui.response_cache {
                        api_area(ui, &mut text.as_str(), "Conteúdo da response", 14, true);
                    }
                }
                1 => {
                    egui::Grid::new("api.response.headers")
                        .striped(true)
                        .show(ui, |ui| {
                            for row in &response.headers {
                                ui.label(&row.key);
                                ui.label(&row.value);
                                ui.end_row();
                            }
                        });
                }
                2 => {
                    if result.script_result.tests.is_empty() {
                        ui.weak("Nenhuma asserção pm.test executada.");
                    }
                    for test in &result.script_result.tests {
                        ui.label(format!(
                            "{}  {}",
                            if test.passed { "PASS" } else { "FAIL" },
                            test.name
                        ));
                        if let Some(error) = &test.error {
                            ui.weak(error);
                        }
                    }
                }
                _ => {
                    if result.script_result.logs.is_empty() {
                        ui.weak("Nenhuma mensagem de console.");
                    }
                    for line in &result.script_result.logs {
                        ui.monospace(line);
                    }
                }
            });
    }
    pub(super) fn api_run_collection(&mut self, id: &str) {
        let options =
            match RunOptions::from_json(self.api_ui.run_iterations.max(1), &self.api_ui.run_data) {
                Ok(options) => options,
                Err(error) => {
                    self.api_ui.notice = Some(error);
                    return;
                }
            };
        match self
            .service
            .api
            .run_with_options(id, self.api_ui.scripts_enabled, options)
        {
            Ok(()) => self.api_ui.notice = None,
            Err(error) => self.api_ui.notice = Some(error),
        }
    }
    pub(super) fn api_runner_bar(&mut self, ui: &mut egui::Ui) {
        let Some(run) = self.service.api.runner.as_ref() else {
            return;
        };
        let mut cancel = false;
        let mut open = None;
        let mut history = false;
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Collection Run · {}", run.name));
            ui.label(format!("{}/{} requests", run.completed, run.total));
            ui.label(format!(
                "{} tests passed · {} failed",
                run.tests_passed, run.tests_failed
            ));
            if !run.finished {
                cancel = ui.button("Cancel run").clicked();
            } else {
                ui.label(if run.cancelled {
                    "Cancelled"
                } else if run.error.is_some() {
                    "Failed"
                } else {
                    "Completed"
                });
            }
            history = ui.button("Run results").clicked();
        });
        if !run.finished {
            ui.add(
                egui::ProgressBar::new(if run.total == 0 {
                    0.
                } else {
                    run.completed as f32 / run.total as f32
                })
                .show_percentage(),
            );
        }
        if let Some(error) = &run.error {
            crate::status_messages::status_message(ui, error);
        }
        if self.api_ui.active == Some(ApiTab::History) {
            egui::CollapsingHeader::new("Collection run results")
                .default_open(true)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("api.run.results")
                        .max_height(260.)
                        .codisseu()
                        .show(ui, |ui| {
                            for result in &run.results {
                                let status = result.status.map_or_else(
                                    || {
                                        if result.error.is_some() {
                                            "Error".into()
                                        } else {
                                            "Skipped".into()
                                        }
                                    },
                                    |s| s.to_string(),
                                );
                                let label = format!(
                                    "{}  {}    {} · {} ms · {} PASS / {} FAIL",
                                    result.method,
                                    result.name,
                                    status,
                                    result.elapsed_ms,
                                    result.tests_passed,
                                    result.tests_failed
                                );
                                if ui.selectable_label(false, label).clicked() {
                                    open = Some(result.request_id.clone());
                                }
                                if let Some(error) = &result.error {
                                    ui.weak(error);
                                }
                            }
                        });
                });
        }
        ui.separator();
        if cancel {
            self.service.api.cancel();
        }
        if history {
            self.api_open_tab(ApiTab::History);
        }
        if let Some(id) = open {
            self.api_open_tab(ApiTab::Request(id));
        }
    }
    pub(super) fn api_history_editor(&mut self, ui: &mut egui::Ui) {
        self.api_runner_bar(ui);
        ui.strong("Request History");
        ui.weak("Histórico desta sessão e deste projeto; abrir uma entrada não reenvia a request.");
        egui::ScrollArea::vertical().codisseu().show(ui, |ui| {
            for entry in self.service.api.history.clone().iter() {
                if ui
                    .selectable_label(
                        false,
                        format!(
                            "{}  {}     {} · {} ms",
                            entry.method,
                            entry.name,
                            if entry.status == 0 {
                                "Skipped".into()
                            } else {
                                entry.status.to_string()
                            },
                            entry.elapsed_ms
                        ),
                    )
                    .clicked()
                {
                    self.api_open_tab(ApiTab::Request(entry.request_id.clone()));
                }
            }
        });
    }
    pub(super) fn api_specs_editor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Specs");
            if ui.button("Importar OpenAPI").clicked() {
                self.api_transfer(false, String::new());
                if let Some(t) = &mut self.api_ui.transfer {
                    t.kind = 3;
                }
            }
        });
        ui.weak(
            "Importe uma especificação OpenAPI 3 em JSON para gerar uma collection neste projeto.",
        );
        let mut remove = None;
        egui::ScrollArea::vertical().codisseu().show(ui, |ui| {
            for spec in &self.service.api.workspace.specs {
                egui::CollapsingHeader::new(&spec.name)
                    .id_salt(&spec.id)
                    .show(ui, |ui| {
                        if ui.button("Copiar source").clicked() {
                            ui.ctx().copy_text(spec.source.clone());
                        }
                        if ui.button("Excluir spec").clicked() {
                            remove = Some(spec.id.clone());
                        }
                        let mut source = spec.source.as_str();
                        api_area(ui, &mut source, "Source da especificação", 14, true);
                    });
            }
        });
        if let Some(id) = remove {
            self.service.api.workspace.specs.retain(|s| s.id != id);
            self.api_ui.changed();
        }
    }
    pub(super) fn api_mocks_editor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Mocks");
            if ui.button("Novo mock").clicked() {
                self.service.api.workspace.mocks.push(MockDefinition {
                    id: new_id(),
                    name: "New Mock".into(),
                    routes: vec![MockRoute {
                        method: "GET".into(),
                        path: "/".into(),
                        status: 200,
                        headers: vec![KeyValue::new("Content-Type", "application/json")],
                        body: "{\"ok\":true}".into(),
                    }],
                });
                self.api_ui.changed();
            }
        });
        ui.weak("Servidores locais da sessão, disponíveis somente em 127.0.0.1. Configure uma rota e clique Start.");
        self.api_mock_controls(ui);
        let mut remove = None;
        egui::ScrollArea::vertical().codisseu().show(ui, |ui| {
            for mock in &mut self.service.api.workspace.mocks {
                ui.push_id(&mock.id, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if api_field(ui, &mut mock.name, "Nome do mock", "", 300., false).changed()
                        {
                            self.api_ui.changed();
                        }
                        if ui.button("Excluir mock").clicked() {
                            remove = Some(mock.id.clone());
                        }
                    });
                    let mut remove_route = None;
                    for (index, route) in mock.routes.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if api_field(
                                    ui,
                                    &mut route.method,
                                    "Método da rota mock",
                                    "",
                                    65.,
                                    false,
                                )
                                .changed()
                                {
                                    self.api_ui.changed();
                                }
                                let path_width = (ui.available_width() - 155.).max(80.);
                                if api_field(
                                    ui,
                                    &mut route.path,
                                    "Caminho da rota mock",
                                    "",
                                    path_width,
                                    false,
                                )
                                .changed()
                                {
                                    self.api_ui.changed();
                                }
                                if api_named(
                                    ui.add(
                                        egui::DragValue::new(&mut route.status).range(100..=599),
                                    ),
                                    "Status da rota mock",
                                    egui::accesskit::Role::SpinButton,
                                )
                                .changed()
                                {
                                    self.api_ui.changed();
                                }
                                if api_remove_button(ui, "Excluir rota mock").clicked() {
                                    remove_route = Some(index);
                                }
                            });
                            egui::CollapsingHeader::new("Headers")
                                .id_salt("headers")
                                .show(ui, |ui| {
                                    if kv_table(ui, &mut route.headers, false, "mock.headers") {
                                        self.api_ui.changed();
                                    }
                                });
                            if api_area(ui, &mut route.body, "Body da rota mock", 4, true).changed()
                            {
                                self.api_ui.changed();
                            }
                        });
                    }
                    if let Some(index) = remove_route {
                        mock.routes.remove(index);
                        self.api_ui.changed();
                    }
                    if ui.button("+ Route").clicked() {
                        mock.routes.push(MockRoute {
                            method: "GET".into(),
                            path: "/new".into(),
                            status: 200,
                            headers: vec![],
                            body: String::new(),
                        });
                        self.api_ui.changed();
                    }
                    ui.separator();
                });
            }
        });
        if let Some(id) = remove {
            self.service.api.workspace.mocks.retain(|m| m.id != id);
            self.api_ui.changed();
        }
    }
    pub(crate) fn api_windows(&mut self, ctx: &egui::Context) {
        self.api_move_dialog(ctx);
        if let Some((tab, mut name)) = self.api_ui.rename.take() {
            let mut open = true;
            let mut apply = false;
            egui::Window::new("Renomear")
                .id(egui::Id::unique("api.rename"))
                .open(&mut open)
                .collapsible(false)
                .show(ctx, |ui| {
                    api_field(ui, &mut name, "Novo nome", "", 300., false);
                    apply = ui.button("Renomear").clicked();
                });
            if apply && !name.trim().is_empty() {
                match &tab {
                    ApiTab::Collection(id) => {
                        if let Some(c) = self
                            .service
                            .api
                            .workspace
                            .collections
                            .iter_mut()
                            .find(|c| &c.id == id)
                        {
                            c.name = name;
                        }
                    }
                    ApiTab::Environment(id) => {
                        if let Some(e) = self
                            .service
                            .api
                            .workspace
                            .environments
                            .iter_mut()
                            .find(|e| &e.id == id)
                        {
                            e.name = name;
                        }
                    }
                    ApiTab::Request(id) | ApiTab::Folder(id) => {
                        for c in &mut self.service.api.workspace.collections {
                            if rename_item(&mut c.items, id, &name) {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
                self.api_ui.changed();
            } else if open {
                self.api_ui.rename = Some((tab, name));
            }
        }
        let Some(mut transfer) = self.api_ui.transfer.take() else {
            return;
        };
        let mut open = true;
        let mut apply = false;
        let mut close = false;
        egui::Window::new(if transfer.export{"Exportar API"}else{"Importar API"}).id(egui::Id::unique("api.transfer")).open(&mut open).default_width(650.).collapsible(false).show(ctx,|ui|{
            if !transfer.export{
                ui.horizontal(|ui|{for (i,label) in ["Collection","Environment","cURL","OpenAPI"].iter().enumerate(){ui.selectable_value(&mut transfer.kind,i,*label);}});
                ui.weak("Cole o conteúdo abaixo ou informe um arquivo. O importador não envia requisições.");
            }else{ui.weak("Exportação redige valores secretos. Revise o conteúdo antes de compartilhar.");}
            egui::ScrollArea::both().max_height(350.).codisseu().show(ui,|ui|{api_area(ui,&mut transfer.text,"Conteúdo para importar ou exportar",16,true);});
            ui.horizontal(|ui|{
                let path_width=(ui.available_width()-110.).max(100.);
                api_field(ui,&mut transfer.path,"Caminho do arquivo","Caminho do arquivo",path_width,false);
                if ui.add_enabled(!transfer.path.is_empty()&&self.api_ui.file_rx.is_none(),egui::Button::new(if transfer.export{"Salvar arquivo"}else{"Ler arquivo"})).clicked(){
                    let (tx,rx)=mpsc::channel();self.api_ui.file_rx=Some(rx);let path=PathBuf::from(&transfer.path);let export=transfer.export;let content=transfer.text.clone();
                    std::thread::spawn(move||{
                        let result=if export{write_export_file(&path,&content).map(|()|content)}else{
                            std::fs::metadata(&path).map_err(|e|e.to_string()).and_then(|meta|if meta.len()>16*1024*1024{Err("Arquivo excede 16 MiB".into())}else{std::fs::read_to_string(&path).map_err(|e|e.to_string())})
                        };let _=tx.send(result);
                    });
                }
            });
            ui.horizontal(|ui|{
                if transfer.export{if ui.button("Copiar conteúdo").clicked(){ctx.copy_text(transfer.text.clone());}}else{apply=ui.button("Importar").clicked();}
                if ui.button("Fechar").clicked(){ui.close();close=true;}
            });
            if let Some(error)=&self.api_ui.notice{crate::status_messages::status_message(ui, error);}
        });
        if close {
            open = false;
        }
        if apply {
            let result = self.api_import_text(transfer.kind, &transfer.text);
            match result {
                Ok(tab) => {
                    self.api_open_tab(tab);
                    self.api_ui.changed();
                    self.api_ui.notice = None;
                    open = false;
                }
                Err(error) => self.api_ui.notice = Some(error),
            }
        }
        if open {
            self.api_ui.transfer = Some(transfer);
        }
    }
    pub(super) fn api_mock_controls(&mut self, ui: &mut egui::Ui) {
        if let Some(url) = self.service.api.mock_url.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.monospace(&url);
                if ui.button("Copiar URL").clicked() {
                    ui.ctx().copy_text(url);
                }
                if ui.button("Stop").clicked() {
                    self.service.api.stop_mock();
                }
            });
        } else if self.service.api.mock_starting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Starting mock…");
            });
        } else {
            for (id, name) in self
                .service
                .api
                .workspace
                .mocks
                .iter()
                .map(|m| (m.id.clone(), m.name.clone()))
                .collect::<Vec<_>>()
            {
                if ui.button(format!("Start {name}")).clicked()
                    && let Err(error) = self.service.api.start_mock(&id)
                {
                    self.api_ui.notice = Some(error);
                }
            }
        }
    }
    pub(super) fn api_import_text(&mut self, kind: usize, text: &str) -> Result<ApiTab, String> {
        match kind {
            0 => {
                let mut c = import_collection(text)?;
                c.id = new_id();
                rekey_items(&mut c.items);
                let id = c.id.clone();
                self.service.api.workspace.collections.push(c);
                Ok(ApiTab::Collection(id))
            }
            1 => {
                let mut e = import_environment(text)?;
                e.id = new_id();
                let id = e.id.clone();
                self.service.api.workspace.environments.push(e);
                Ok(ApiTab::Environment(id))
            }
            2 => {
                let mut r = parse_curl(text)?;
                r.id = new_id();
                let id = r.id.clone();
                let c = self.service.api.workspace.collections.first_mut();
                if let Some(c) = c {
                    c.items.push(CollectionItem::Request(Box::new(r)));
                } else {
                    let mut c = Collection::default();
                    c.items.push(CollectionItem::Request(Box::new(r)));
                    self.service.api.workspace.collections.push(c);
                }
                Ok(ApiTab::Request(id))
            }
            _ => {
                let (spec, mut collection) = import_openapi(text)?;
                collection.id = new_id();
                rekey_items(&mut collection.items);
                let id = collection.id.clone();
                self.service.api.workspace.specs.push(spec);
                self.service.api.workspace.collections.push(collection);
                Ok(ApiTab::Collection(id))
            }
        }
    }
}
fn rekey_items(items: &mut [CollectionItem]) {
    for item in items {
        match item {
            CollectionItem::Request(r) => r.id = new_id(),
            CollectionItem::Folder { id, items, .. } => {
                *id = new_id();
                rekey_items(items);
            }
        }
    }
}
fn write_export_file(path: &std::path::Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    // create_new protects existing files from an accidental overwrite in the export form.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())
}
