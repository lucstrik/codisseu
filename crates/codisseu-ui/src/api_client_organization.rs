use super::*;
impl CodisseuApp {
    pub(super) fn api_parent(&mut self) -> String {
        match self.api_ui.active.as_ref() {
            Some(ApiTab::Folder(id) | ApiTab::Collection(id)) => return id.clone(),
            Some(ApiTab::Request(id)) => {
                if let Some(parent) = self.service.api.workspace.parent_id(id) {
                    return parent;
                }
            }
            _ => {}
        }
        self.service
            .api
            .workspace
            .collections
            .first()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| self.service.api.workspace.new_collection("New Collection"))
    }
    pub(super) fn api_add_folder(&mut self, parent: &str) {
        match self.service.api.workspace.new_folder(parent, "New Folder") {
            Ok(id) => {
                self.api_open_tab(ApiTab::Folder(id));
                self.api_ui.changed();
            }
            Err(error) => self.api_ui.notice = Some(error),
        }
    }
    pub(super) fn api_organize_menu(&mut self, ui: &mut egui::Ui, id: &str) {
        if ui.button("Duplicar").clicked() {
            match self.service.api.workspace.duplicate_item(id) {
                Ok(new) => {
                    let tab = if matches!(
                        self.service.api.workspace.item(&new),
                        Some(CollectionItem::Folder { .. })
                    ) {
                        ApiTab::Folder(new)
                    } else {
                        ApiTab::Request(new)
                    };
                    self.api_open_tab(tab);
                    self.api_ui.changed();
                }
                Err(error) => self.api_ui.notice = Some(error),
            }
            ui.close();
        }
        if ui.button("Mover para…").clicked() {
            self.api_ui.move_item = Some(id.into());
            ui.close();
        }
        for (label, down) in [("Mover acima", false), ("Mover abaixo", true)] {
            if ui.button(label).clicked() {
                if self.service.api.workspace.reorder_item(id, down) {
                    self.api_ui.changed();
                }
                ui.close();
            }
        }
    }
    pub(super) fn api_move_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.api_ui.move_item.clone() else {
            return;
        };
        let mut open = true;
        let mut target = None;
        egui::Window::new("Mover request ou pasta")
            .id(egui::Id::unique("api.move"))
            .open(&mut open)
            .default_width(420.)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("Escolha a collection ou pasta de destino:");
                egui::ScrollArea::vertical()
                    .max_height(350.)
                    .codisseu()
                    .show(ui, |ui| {
                        for (parent, label, _) in self.service.api.workspace.destinations(Some(&id))
                        {
                            if ui.selectable_label(false, &label).clicked() {
                                target = Some(parent);
                            }
                        }
                    });
            });
        if let Some(target) = target {
            match self.service.api.workspace.move_item(&id, &target) {
                Ok(()) => {
                    self.api_ui.changed();
                    open = false;
                }
                Err(error) => self.api_ui.notice = Some(error),
            }
        }
        if !open {
            self.api_ui.move_item = None;
        }
    }
    pub(super) fn api_folder_editor(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.weak(self.service.api.workspace.request_path(id).join(" / "));
        let (mut request, mut folder, mut run) = (false, false, false);
        ui.horizontal_wrapped(|ui| {
            request = ui.button("Nova request").clicked();
            folder = ui.button("Nova subpasta").clicked();
            run = ui
                .add_enabled(
                    self.service.api.running_request.is_none(),
                    egui::Button::new("Executar pasta"),
                )
                .clicked();
            ui.checkbox(
                &mut self.api_ui.scripts_enabled,
                "Executar scripts desta request e da collection",
            );
            self.api_environment_picker(ui);
        });
        self.api_run_options(ui);
        self.api_runner_bar(ui);
        if let Some(CollectionItem::Folder {
            name,
            auth,
            pre_request_script,
            post_response_script,
            ..
        }) = self.service.api.workspace.item_mut(id)
        {
            let mut changed = api_field(ui, name, "Nome da pasta", "", 300., false).changed();
            egui::ScrollArea::vertical().id_salt(("api.folder.editor",id)).codisseu().show(ui,|ui|{
                ui.strong("Authorization");changed|=auth_form(ui,auth);
                ui.weak("As requests com Inherit usam a autorização desta pasta; Inherit aqui usa a pasta acima ou collection.");
                ui.separator();ui.strong("Scripts herdados pelas requests");
                ui.label("Pre-request");changed|=script_field(ui,pre_request_script,"folder.pre");
                ui.label("Post-response");changed|=script_field(ui,post_response_script,"folder.post");
            });
            if changed {
                self.api_ui.changed();
            }
        }
        if request {
            self.api_new_request(Some(id.into()));
        }
        if folder {
            self.api_add_folder(id);
        }
        if run {
            self.api_run_collection(id);
        }
    }
    pub(super) fn api_run_options(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Opções de execução").id_salt("api.run.options").show(ui,|ui|{
            ui.horizontal_wrapped(|ui|{ui.label("Iterações");ui.add(egui::DragValue::new(&mut self.api_ui.run_iterations).range(1..=256));});
            ui.label("Dados por iteração (array JSON opcional)");
            api_area(ui,&mut self.api_ui.run_data,"Dados do runner",3,true);
            ui.weak("Exemplo: [{\"customer_id\":\"1\"},{\"customer_id\":\"2\"}]. Dados são temporários e acessíveis com pm.iterationData.get(). Máximo256 envios.");
        });
    }
}
