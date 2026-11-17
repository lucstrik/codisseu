//! Run/debug configuration review, independent of project script discovery.
use super::*;
use codisseu_core::debugger::{DebugCommand as D, DebugConfiguration, DebugRequest, DebugRuntime};
use std::collections::BTreeMap;

impl CodisseuApp {
    pub(super) fn debugger_edit_configuration(&mut self, config: DebugConfiguration) {
        self.debugger_ui.arguments = config.args.join("\n");
        self.debugger_ui.environment = pairs_text(&config.env);
        self.debugger_ui.mappings = pairs_text(&config.path_mappings);
        self.debugger_ui.selected_configuration = Some(config.id.clone());
        self.debugger_ui.draft = Some(config);
        self.debugger_ui.config_error = None;
    }
    pub(super) fn debugger_dialog(&mut self, ctx: &egui::Context) {
        if !self.debugger_ui.dialog {
            return;
        }
        let Some(mut config) = self.debugger_ui.draft.take() else {
            self.debugger_open_configurations();
            return;
        };
        if config.integrated_browser && config.browser_tab.is_none() {
            let matches: Vec<_> = self
                .service
                .browser
                .tabs
                .values()
                .filter(|t| t.url == config.url)
                .map(|t| t.id)
                .collect();
            config.browser_tab = if matches.len() == 1 {
                matches.first().copied()
            } else if self.service.browser.tabs.len() == 1 {
                self.service.browser.tabs.keys().next().copied()
            } else {
                None
            };
        }
        let browser_before = (
            config.browser_tab,
            config.cwd.clone(),
            config.adapter.clone(),
            config.adapter_runtime.clone(),
            config.source_maps,
            config.stop_on_entry,
        );
        let mut close = false;
        let mut selection = None;
        let mut new_runtime = None;
        let mut launch = false;
        let mut apply = false;
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(color(self.tokens.shell_panel_background));
        let id = egui::Id::unique("debug.configurations");
        let mut modal = egui::Modal::new(id);
        if config.integrated_browser {
            let viewport = ctx.content_rect();
            let width = (viewport.width() - 72.).clamp(420., 920.);
            let body = (viewport.height() - 270.).clamp(180., 470.);
            modal = modal.area(egui::Modal::default_area(id).anchor(
                egui::Align2::LEFT_TOP,
                egui::vec2(
                    ((viewport.width() - width - 12.) / 2.).max(12.),
                    ((viewport.height() - body - 200.) / 2.).max(12.),
                ),
            ));
        }
        modal.frame(frame).show(ctx, |ui| {
            self.tokens.form_style(ui);
            super::git_scroll::scm_scroll_style(ui);
            let size = ctx.content_rect().size();
            ui.set_width((size.x - 72.).clamp(420., 920.));
            ui.horizontal(|ui| {
                ui.strong("Run/Debug Configurations");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, Icon::Close, "Close Run/Debug Configurations", false, 26.).clicked() { close = true; }
                });
            });
            ui.separator();
            let height = (size.y - 270.).clamp(180., 470.);
            let width = ui.available_width();
            ui.horizontal_top(|ui| {
                ui.set_height(height);
                ui.allocate_ui_with_layout(egui::vec2(190_f32.min(width * 0.3), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.horizontal(|ui| {
                        let plus = icon_button(ui, Icon::Add, "Add New Configuration", false, 26.);
                        egui::Popup::menu(&plus).show(|ui| {
                            for (label, runtime) in [("PHP Script", DebugRuntime::Php), ("Node.js / TypeScript", DebugRuntime::Node), ("JavaScript Debug (Chrome)", DebugRuntime::Chrome), ("Rust / CodeLLDB", DebugRuntime::Rust), ("Python / debugpy", DebugRuntime::Python)] {
                                if ui.button(label).clicked() { new_runtime = Some(runtime); ui.close(); }
                            }
                        });
                        if icon_button(ui, Icon::Remove, "Remove Configuration", false, 26.).clicked() {
                            self.send(Command::DeleteDebugConfiguration(config.id.clone()));
                            selection = self.service.debug_configurations.iter().find(|c| c.id != config.id).cloned();
                        }
                        if icon_button(ui, Icon::Copy, "Copy Configuration", false, 26.).clicked() {
                            let mut copy = config.clone(); self.debugger_ui.configuration_sequence += 1;
                            copy.id = format!("{}-copy-{}", config.id, self.debugger_ui.configuration_sequence); copy.name.push_str(" copy"); copy.trusted = false; selection = Some(copy);
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("debug.configurations.list").max_height((height - 36.).max(0.)).min_scrolled_height((height - 36.).max(0.)).auto_shrink([false, false]).codisseu().show(ui, |ui| {
                        for (label, runtime) in [("PHP", DebugRuntime::Php), ("Node.js", DebugRuntime::Node), ("JavaScript Debug", DebugRuntime::Chrome), ("Rust", DebugRuntime::Rust), ("Python", DebugRuntime::Python)] {
                            let configs = self.service.debug_configurations.iter().filter(|c| c.runtime == runtime).cloned().collect::<Vec<_>>();
                            let contains_draft = config.runtime == runtime && !configs.iter().any(|c| c.id == config.id);
                            if configs.is_empty() && !contains_draft { continue; }
                            egui::CollapsingHeader::new(label).id_salt(("debug.config.group", label)).default_open(true).show(ui, |ui| {
                                for saved in configs {
                                    if ui.selectable_label(saved.id == config.id, &saved.name).clicked() { selection = Some(saved); }
                                }
                                if contains_draft { ui.selectable_label(true, &config.name).on_hover_text("Unsaved configuration"); }
                            });
                        }
                    });
                });
                ui.separator();
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width().max(0.), height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical().id_salt("debug.configuration.form").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).codisseu().show(ui, |ui| {
                        form_text(ui, "Name", &mut config.name);
                        ui.add_space(10.);
                        ui.horizontal(|ui| {
                            ui.strong(if config.integrated_browser {"Navegador integrado"} else {runtime_label(config.runtime)}); ui.add_space(12.); ui.label("Configuration");
                        });
                        if config.runtime == DebugRuntime::Chrome && ui.checkbox(&mut config.integrated_browser,"Usar aba do navegador integrado do Codisseu").changed() {
                            config.trusted=false;config.browser_inspect_uri=None;config.browser_tab=self.browser_active();
                            if config.integrated_browser {config.request=DebugRequest::Attach;}
                        }
                        ui.separator(); ui.add_space(8.);
                        ui.spacing_mut().text_edit_width = (ui.available_width() - 148.).max(80.);
                        egui::Grid::new("debug.configuration.grid").num_columns(2).spacing([12., 8.]).show(ui, |ui| {
                            let mode_label = ui.label("Mode:");
                            let mode_response = egui::ComboBox::from_id_salt("debug.request").selected_text(request_label(config.request)).show_ui(ui, |ui| {
                                if !config.integrated_browser {ui.selectable_value(&mut config.request, DebugRequest::Launch, "Launch");}
                                if !matches!(config.runtime, DebugRuntime::Php | DebugRuntime::Rust | DebugRuntime::Python) { ui.selectable_value(&mut config.request, DebugRequest::Attach, "Attach"); }
                                if config.runtime == DebugRuntime::Php { ui.selectable_value(&mut config.request, DebugRequest::Listen, "Listen for PHP Debug Connections"); }
                            }); mode_response.response.labelled_by(mode_label.id); ui.end_row();
                            if config.request == DebugRequest::Launch && config.runtime != DebugRuntime::Rust {
                                path_field(ui, if config.runtime == DebugRuntime::Php {"PHP interpreter:"} else if config.runtime == DebugRuntime::Chrome {"Browser executable:"} else if config.runtime == DebugRuntime::Python {"Python interpreter:"} else {"Node interpreter:"}, &mut config.executable);
                                if config.runtime == DebugRuntime::Python {
                                    let label = ui.label("Target:");
                                    let mut module_target = config.module.is_some();
                                    let response = egui::ComboBox::from_id_salt("debug.python.target").selected_text(if module_target {"Module (-m)"} else {"File"}).show_ui(ui, |ui| {
                                        ui.selectable_value(&mut module_target, false, "File");
                                        ui.selectable_value(&mut module_target, true, "Module (-m)");
                                    }); response.response.labelled_by(label.id); ui.end_row();
                                    if module_target {
                                        config.program = None;
                                        let module = config.module.get_or_insert_with(String::new);
                                        let label = ui.label("Module:"); ui.add(egui::TextEdit::singleline(module).id_salt("debug.python.module").hint_text("package.module, pytest or unittest")).labelled_by(label.id); ui.end_row();
                                    } else { config.module = None; }
                                }
                                if config.runtime != DebugRuntime::Chrome && config.module.is_none() {
                                    let mut program = config.program.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                                    let label = ui.label("File:"); ui.add(egui::TextEdit::singleline(&mut program).id_salt("debug.config.file")).labelled_by(label.id); ui.end_row();
                                    config.program = (!program.trim().is_empty()).then(|| program.into());
                                }
                            }
                            if config.runtime == DebugRuntime::Rust {
                                let mut program = config.program.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                                let label = ui.label("Compiled Rust executable:"); ui.add(egui::TextEdit::singleline(&mut program).id_salt("debug.config.file")).labelled_by(label.id); ui.end_row();
                                config.program = (!program.trim().is_empty()).then(|| program.into());
                            }
                            if config.integrated_browser {
                                let label=ui.label("Aba do navegador:");
                                let selected=config.browser_tab.and_then(|id|self.service.browser.tabs.get(&id)).map(|t|t.title.as_str()).unwrap_or("Selecione uma aba");
                                let combo=egui::ComboBox::from_id_salt("debug.browser.tab").selected_text(selected).show_ui(ui,|ui|{
                                    for tab in self.service.browser.tabs.values(){ui.selectable_value(&mut config.browser_tab,Some(tab.id),format!("{} · {}",tab.title,tab.url));}
                                });combo.response.labelled_by(label.id);ui.end_row();
                                if let Some(tab)=config.browser_tab.and_then(|id|self.service.browser.tabs.get(&id)){config.url=tab.url.clone();}
                            }
                            path_field(ui, if config.integrated_browser {"Web root:"}else{"Working directory:"}, &mut config.cwd);
                            if config.runtime == DebugRuntime::Php && config.request == DebugRequest::Listen { let label = ui.label("Listen address:"); ui.add(egui::TextEdit::singleline(&mut config.listen_host).id_salt("debug.listen.address")).labelled_by(label.id); ui.end_row(); }
                            if config.runtime == DebugRuntime::Chrome { let label = ui.label("URL:"); ui.add_enabled(!config.integrated_browser,egui::TextEdit::singleline(&mut config.url).id_salt("debug.config.url")).labelled_by(label.id); ui.end_row(); }
                            if !config.integrated_browser && (config.request != DebugRequest::Launch || config.runtime == DebugRuntime::Php) {
                                let label = ui.label(if config.runtime == DebugRuntime::Php {"Xdebug port:"} else {"Inspector port:"});
                                ui.add(egui::DragValue::new(&mut config.port).range(0..=65535).speed(1)).labelled_by(label.id); ui.end_row();
                            }
                            if !config.integrated_browser {
                            let arguments_label = ui.label("Arguments:");
                            ui.add(egui::TextEdit::multiline(&mut self.debugger_ui.arguments).desired_rows(2).hint_text("One argument per line")).labelled_by(arguments_label.id); ui.end_row();
                            let environment_label = ui.label("Environment:");
                            ui.add(egui::TextEdit::multiline(&mut self.debugger_ui.environment).desired_rows(2).password(true).hint_text("NAME=value, one per line; session only")).labelled_by(environment_label.id); ui.end_row();
                            }
                            if matches!(config.runtime, DebugRuntime::Php | DebugRuntime::Rust | DebugRuntime::Python) {
                                let mappings_label = ui.label("Path mappings:"); ui.add(egui::TextEdit::multiline(&mut self.debugger_ui.mappings).desired_rows(2).hint_text("remote path=local path, one per line")).labelled_by(mappings_label.id); ui.end_row();
                            }
                        });
                        ui.add_space(8.);
                        if matches!(config.runtime, DebugRuntime::Node | DebugRuntime::Chrome) { ui.checkbox(&mut config.source_maps, "Enable source maps (TypeScript / bundled JavaScript)"); }
                        ui.checkbox(&mut config.stop_on_entry, "Break at first line");
                        egui::CollapsingHeader::new("Advanced · Debug adapter").id_salt("debug.adapter.advanced").show(ui, |ui| {
                            let mut adapter = config.adapter.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            let mut runtime = config.adapter_runtime.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                            form_text(ui, if config.runtime == DebugRuntime::Rust { "CodeLLDB executable (automatic when empty)" } else if config.runtime == DebugRuntime::Python { "debugpy adapter script (automatic when empty)" } else { "Adapter script (automatic when empty)" }, &mut adapter);
                            if config.runtime != DebugRuntime::Rust { form_text(ui, if config.runtime == DebugRuntime::Python { "Adapter Python interpreter (automatic when empty)" } else { "Adapter Node runtime (automatic when empty)" }, &mut runtime); }
                            config.adapter = (!adapter.trim().is_empty()).then(|| adapter.into()); config.adapter_runtime = (!runtime.trim().is_empty()).then(|| runtime.into());
                        });

                        if config.integrated_browser {ui.add(egui::Label::new("Conecta somente à aba selecionada. Web root mapeia URLs para as fontes; o servidor deve estar em execução. Stop desanexa e mantém a página aberta.").wrap());}
                        if config.runtime == DebugRuntime::Php && config.request == DebugRequest::Listen { ui.label("Configure Xdebug client_host, client_port and path mappings for this listener. For Docker, use the host address reachable from the container."); if !matches!(config.listen_host.as_str(), "127.0.0.1" | "localhost" | "::1") { crate::status_messages::status_message(ui, "Docker/remote connections: the listener will accept connections on this address. Configure Xdebug client_host and path mappings."); } }
                        if config.runtime == DebugRuntime::Python { ui.add(egui::Label::new("The selected Python interpreter or virtual environment runs your file or module. The private debugpy engine is independent; your environment does not need debugpy installed.").wrap()); }
                        if config.runtime == DebugRuntime::Rust { ui.add(egui::Label::new("Build the program first with Rust · Cargo → Build, then select its executable under target/debug. The Cargo dev profile includes debug symbols; Release requires debug symbols in Cargo.toml. This action does not compile or save files.").wrap()); }
                        if config.runtime == DebugRuntime::Node && config.program.as_ref().is_some_and(|p| p.extension().is_some_and(|e| e == "ts" || e == "tsx")) { ui.label("For compiled TypeScript, select the generated JavaScript entry point with source maps enabled. Breakpoints remain in the original .ts file."); }
                    });
                });
            });
            ui.separator();
            if config.integrated_browser && browser_before!=(config.browser_tab,config.cwd.clone(),config.adapter.clone(),config.adapter_runtime.clone(),config.source_maps,config.stop_on_entry) {config.trusted=false;}
            ui.checkbox(&mut config.trusted, "I trust this configuration and authorize executing its code");
            ui.label(RichText::new("Environment values and execution authorization are kept only for this IDE session.").small().color(color(self.tokens.muted)));
            if let Some(error) = self.debugger_ui.config_error.as_ref().or(self.service.debug_profile_error.as_ref()) { crate::status_messages::status_message(ui, error); }
            if let Some(error) = &self.service.debugger.error { crate::status_messages::status_message(ui, error); }
            let dirty = self.service.documents.iter().any(|d| d.is_dirty());
            if dirty && (config.request == DebugRequest::Launch || config.integrated_browser) { crate::status_messages::status_message(ui, "Save modified files before launching. Debugging runs the files on disk."); }
            ui.horizontal(|ui| {
                ui.label(RichText::new("Each launch creates an independent debug session.").small().color(color(self.tokens.muted)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let start = ui.add_enabled(config.trusted && (!dirty || (config.request != DebugRequest::Launch && !config.integrated_browser)) && (!config.integrated_browser || (config.browser_tab.is_some() && !self.service.browser_debug_pending())), self.tokens.dialog_button("Debug", true));
                    ui.ctx().accesskit_node_builder(start.id, |node| node.set_label("Start Debug Configuration"));
                    if start.clicked() { launch = true; }
                    if ui.add(self.tokens.dialog_button("Cancel", false)).clicked() { close = true; }
                    if ui.add(self.tokens.dialog_button("Apply", false)).clicked() { apply = true; }
                });
            });
        });
        if apply || launch {
            match parse_pairs(&self.debugger_ui.environment).and_then(|env| {
                parse_pairs(&self.debugger_ui.mappings).map(|mapping| (env, mapping))
            }) {
                Ok((env, mappings)) => {
                    config.args = self
                        .debugger_ui
                        .arguments
                        .lines()
                        .filter(|a| !a.is_empty())
                        .map(str::to_owned)
                        .collect();
                    config.env = env;
                    config.path_mappings = mappings;
                    if config.name.trim().is_empty() {
                        self.debugger_ui.config_error =
                            Some("Configuration name is required".into());
                    } else {
                        self.send(Command::SaveDebugConfiguration(config.clone()));
                        self.debugger_ui.selected_configuration = Some(config.id.clone());
                        if launch {
                            let before = self.service.debugger.sessions.last().map(|s| s.id);
                            self.send(Command::Debug(D::Start(config.clone())));
                            if self.service.browser_debug_pending() {
                                self.show_activity = true;
                                self.tool_tab = ToolTab::Debug;
                                close = true;
                            }
                            if let Some(id) = self
                                .service
                                .debugger
                                .sessions
                                .last()
                                .map(|s| s.id)
                                .filter(|id| Some(*id) != before)
                            {
                                self.debugger_ui.session_configs.insert(id, config.clone());
                                self.debugger_ui.selected_session = Some(id);
                                self.show_activity = true;
                                self.tool_tab = ToolTab::Debug;
                                close = true;
                            }
                        }
                        self.debugger_ui.config_error = None;
                    }
                }
                Err(error) => self.debugger_ui.config_error = Some(error),
            }
        }
        self.debugger_ui.draft = Some(config);
        if let Some(runtime) = new_runtime {
            self.debugger_ui.configuration_sequence += 1;
            let cwd = self.service.workspace.clone().unwrap_or_default();
            let config = DebugConfiguration {
                id: format!(
                    "debug-{}-{}",
                    std::process::id(),
                    self.debugger_ui.configuration_sequence
                ),
                name: runtime_label(runtime).into(),
                runtime,
                cwd,
                executable: match runtime {
                    DebugRuntime::Php => "php",
                    DebugRuntime::Node => "node",
                    DebugRuntime::Python => "python3",
                    DebugRuntime::Chrome | DebugRuntime::Rust => "",
                }
                .into(),
                port: if runtime == DebugRuntime::Php {
                    9003
                } else {
                    0
                },
                ..Default::default()
            };
            let mut config = config;
            if runtime == DebugRuntime::Python
                && let Some(interpreter) = self.service.python_interpreter_for(&config.cwd)
            {
                config.executable = interpreter;
            }
            self.debugger_edit_configuration(config);
        } else if let Some(config) = selection {
            self.debugger_edit_configuration(config);
        }
        if close || consume_exact(ctx, Modifiers::NONE, Key::Escape) {
            self.debugger_ui.dialog = false;
            self.debugger_ui.draft = None;
        }
    }
}
fn runtime_label(runtime: DebugRuntime) -> &'static str {
    match runtime {
        DebugRuntime::Php => "PHP",
        DebugRuntime::Node => "Node.js / TypeScript",
        DebugRuntime::Chrome => "JavaScript Debug",
        DebugRuntime::Rust => "Rust / CodeLLDB",
        DebugRuntime::Python => "Python / debugpy",
    }
}
fn request_label(request: DebugRequest) -> &'static str {
    match request {
        DebugRequest::Launch => "Launch",
        DebugRequest::Attach => "Attach",
        DebugRequest::Listen => "Listen for PHP Debug Connections",
    }
}
fn form_text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.horizontal(|ui| {
        let response = ui.label(label);
        ui.add(
            egui::TextEdit::singleline(value)
                .id_salt(label)
                .desired_width(ui.available_width().max(0.)),
        )
        .labelled_by(response.id);
    });
}
fn path_field(ui: &mut egui::Ui, label: &str, value: &mut PathBuf) {
    let mut text = value.display().to_string();
    let response = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .id_salt(label)
            .hint_text(if label == "Browser executable:" {
                "Automatic when empty"
            } else {
                ""
            }),
    )
    .labelled_by(response.id)
    .on_hover_text(text.clone());
    ui.end_row();
    *value = text.into();
}
fn pairs_text(pairs: &BTreeMap<String, String>) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("\n")
}
fn parse_pairs(text: &str) -> Result<BTreeMap<String, String>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| "Use NAME=value, one entry per line".to_owned())?;
            if key.trim().is_empty() {
                return Err("Empty environment variable or path mapping".into());
            }
            Ok((key.trim().to_owned(), value.to_owned()))
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_pairs_preserve_values_and_reject_ambiguous_input() {
        let values = parse_pairs("A=value=two\n/remote=/local path\n").unwrap();
        assert_eq!(values["A"], "value=two");
        assert_eq!(values["/remote"], "/local path");
        assert!(parse_pairs("A").is_err());
        assert!(parse_pairs("=oops").is_err());
    }
}
