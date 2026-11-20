//! Native drawing presentation over the common JSON document service.
use super::*;
use codisseu_core::{DocumentId, ViewId, drawings::is_drawing};
use codisseu_draw::Scene;

#[derive(Default)]
pub(super) struct DrawingsUi {
    pub open: bool,
    name: String,
    project: Option<PathBuf>,
    entries: std::collections::BTreeMap<(ViewId, DocumentId), DrawingView>,
}
struct DrawingView {
    version: u64,
    scene: Result<Scene, String>,
    source: bool,
    canvas: crate::drawing_canvas::DrawingCanvasUi,
}

impl CodisseuApp {
    pub(super) fn drawings_tick(&mut self) {
        if self.drawings_ui.project != self.service.workspace_path().map(PathBuf::from) {
            self.drawings_ui.project = self.service.workspace_path().map(PathBuf::from);
            self.drawings_ui.name.clear();
            self.drawings_ui.entries.clear();
        }
    }
    pub(super) fn drawings_panel(&mut self, ui: &mut egui::Ui) {
        components::header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            ui.strong("Desenhos");
        });
        egui::Frame::new().inner_margin(12.).show(ui, |ui| {
            ui.weak("Excalidraw · arquivos do projeto");
            ui.add_space(8.);
            if self.service.workspace.is_none() {
                ui.label("Abra um projeto para criar desenhos.");
                return;
            }
            ui.label("Nome ou caminho relativo");
            ui.add(
                egui::TextEdit::singleline(&mut self.drawings_ui.name)
                    .hint_text("arquitetura ou docs/fluxo.excalidraw")
                    .desired_width(f32::INFINITY),
            ).widget_info(|| egui::WidgetInfo::labeled(
                egui::accesskit::Role::TextInput, true, "Caminho do novo desenho",
            ));
            let busy = self.service.drawings.is_busy();
            if ui.add_enabled(!busy && !self.drawings_ui.name.trim().is_empty(),
                egui::Button::new("Criar desenho").min_size(egui::vec2(0., self.tokens.control_height)),
            ).clicked() {
                match self.service.drawings.create(self.drawings_ui.name.trim().to_owned()) {
                    Ok(()) => self.drawings_ui.name.clear(),
                    Err(error) => self.service.drawings.error = Some(error),
                }
            }
            if busy { ui.label("Atualizando desenhos…"); }
            if let Some(error) = &self.service.drawings.error {
                ui.colored_label(color(self.tokens.error), error);
                if ui.add_enabled(!busy, egui::Button::new("Recarregar catálogo")).clicked() {
                    let project = self.service.workspace_path().map(PathBuf::from);
                    if let Err(error) = self.service.drawings.set_project(project, self.service.workspace_roots()) {
                        self.service.drawings.error = Some(error);
                    }
                }
            }
            ui.add_space(8.);
            ui.separator();
            let paths = self.service.drawings.paths.clone();
            if paths.is_empty() && !busy {
                ui.weak("Nenhum desenho neste projeto.");
            }
            egui::ScrollArea::vertical().id_salt("drawings.catalog").codisseu().show(ui, |ui| {
                for path in paths {
                    let roots = self.service.workspace_roots();
                    let label = roots.iter()
                        .find_map(|root| path.strip_prefix(root).ok())
                        .unwrap_or(&path).display().to_string();
                    let active = self.service.active_document().is_some_and(|d| d.path == path);
                    if ui.selectable_label(active, label).on_hover_text(path.display().to_string()).clicked() {
                        let view = self.service.active_view().unwrap_or(0);
                        self.jira_deactivate_view(view);
                        self.browser_ui.deactivate(view);
                        self.api_ui.active = None;
                        self.send(Command::OpenFile(path));
                    }
                }
            });
            ui.add_space(8.);
            ui.weak("Também abra .excalidraw pela árvore ou em Arquivo → Abrir. Os desenhos deste projeto são restaurados ao reabrir.");
        });
    }

    pub(super) fn drawing_editor(&mut self, ui: &mut egui::Ui, view: ViewId) {
        self.drawings_ui.entries.retain(|(owner, id), _| {
            self.service
                .view(*owner)
                .is_some_and(|v| v.tabs().contains(id))
        });
        let Some(doc) = self.service.view_document(view) else {
            return;
        };
        let id = doc.id;
        let version = doc.version;
        let read_only = doc.is_read_only();
        let key = (view, id);
        let mut entry = self
            .drawings_ui
            .entries
            .remove(&key)
            .unwrap_or_else(|| DrawingView {
                version,
                scene: parse_scene(doc.text()),
                source: false,
                canvas: Default::default(),
            });
        if entry.version != version {
            entry.canvas.cancel_interaction();
            entry.scene = parse_scene(doc.text());
            entry.version = version;
        }
        let old_source = entry.source;
        let mut save = false;
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong("Excalidraw");
                    ui.selectable_value(&mut entry.source, false, "Desenho");
                    ui.selectable_value(&mut entry.source, true, "Código JSON");
                    save = ui
                        .add_enabled(!read_only, egui::Button::new("Salvar desenho"))
                        .clicked();
                    if ui.button("Copiar SVG").clicked() {
                        match entry
                            .scene
                            .as_ref()
                            .map_err(Clone::clone)
                            .and_then(Scene::export_svg)
                        {
                            Ok(svg) => {
                                ui.ctx().copy_text(svg);
                                self.service.status = "SVG copiado".into();
                            }
                            Err(error) => self.service.last_error = Some(error),
                        }
                    }
                });
            });
        if old_source != entry.source {
            entry.canvas.cancel_interaction();
            if let Some(doc) = self.service.view_document(view) {
                entry.scene = parse_scene(doc.text());
                entry.version = doc.version;
            }
        }
        if entry.source {
            self.editor(ui, view);
        } else {
            self.editor_bridge.regions.remove(&view);
            self.focus_editor = false;
            match &mut entry.scene {
                Ok(scene) => {
                    let result = ui
                        .add_enabled_ui(!read_only, |ui| entry.canvas.show(ui, scene))
                        .inner;
                    if result.changed {
                        match scene.to_json() {
                            Ok(text) => {
                                self.send(Command::EndEditGroup { id });
                                let commit = self.service.dispatch(Command::Edit {
                                    id,
                                    expected_version: version,
                                    text,
                                });
                                self.send(Command::EndEditGroup { id });
                                if let Err(error) = commit {
                                    self.service.last_error = Some(error);
                                    entry.version = u64::MAX;
                                } else if let Some(doc) = self.service.view_document(view) {
                                    entry.version = doc.version;
                                }
                            }
                            Err(error) => {
                                self.service.last_error = Some(error);
                                entry.version = 0;
                            }
                        }
                    }
                    if result.undo {
                        self.send(Command::Undo { id });
                    }
                    if result.redo {
                        self.send(Command::Redo { id });
                    }
                }
                Err(error) => {
                    egui::Frame::new().inner_margin(16.).show(ui, |ui| {
                        ui.colored_label(color(self.tokens.error), format!("Não foi possível abrir o desenho: {error}"));
                        ui.label("Use Código JSON para corrigir o arquivo. O conteúdo original foi preservado.");
                    });
                }
            }
        }
        if save {
            self.send(Command::Save { id });
        }
        self.drawings_ui.entries.insert(key, entry);
    }

    pub(super) fn drawings_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let Some(view) = self.service.active_view() else {
            return false;
        };
        if self.api_ui.active_in_view(view)
            || self.browser_ui.active_in_view(view)
            || self.jira_active_in_view(view)
        {
            return false;
        }
        let Some(doc) = self
            .service
            .view_document(view)
            .filter(|d| is_drawing(&d.path))
        else {
            return false;
        };
        if self
            .drawings_ui
            .entries
            .get(&(view, doc.id))
            .is_some_and(|entry| entry.source)
            || text_field_focused(ctx)
        {
            return false;
        }
        let id = doc.id;
        if consume_exact(ctx, Modifiers::COMMAND, Key::S) {
            self.send(Command::Save { id });
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z)
            || consume_exact(ctx, Modifiers::COMMAND, Key::Y)
        {
            self.send(Command::Redo { id });
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::Z) {
            self.send(Command::Undo { id });
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::W) {
            self.request_close_tab(view, id);
            return true;
        }
        false
    }
}
fn parse_scene(text: &str) -> Result<Scene, String> {
    if text.trim().is_empty() {
        Ok(Scene::default())
    } else {
        Scene::from_json(text)
    }
}
