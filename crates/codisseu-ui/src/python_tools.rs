//! Python tasks use existing native terminal, modal tokens and explicit project configuration.
use super::*;
use codisseu_core::{
    Language,
    python_project::{Action, Config, ProjectCommand},
};
#[derive(Default)]
pub(super) struct PythonUi {
    pub open: bool,
    root: Option<PathBuf>,
    draft: Option<Config>,
    action: Action,
    arguments: String,
    environment: String,
    values: String,
    trusted: bool,
    error: Option<String>,
    revision: u64,
    launch: bool,
    versions: Vec<(DocumentId, u64)>,
    refresh: Option<(String, PathBuf)>,
}
impl CodisseuApp {
    pub(super) fn python_file(&self) -> Option<PathBuf> {
        self.service
            .active_document()
            .filter(|d| {
                !d.is_read_only()
                    && matches!(
                        d.path.extension().and_then(|e| e.to_str()),
                        Some("py" | "pyw")
                    )
            })
            .map(|d| d.path.clone())
    }
    pub(super) fn python_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            self.python_ui.refresh = None;
            return;
        }
        if let Some((key, root)) = &self.python_ui.refresh
            && !self.terminal.command_running(key)
            && !self.service.python.is_busy()
        {
            let root = root.clone();
            self.python_ui.refresh = None;
            let _ = self
                .service
                .dispatch(Command::Python(ProjectCommand::Inspect(root)));
        }
        let root = self
            .service
            .active_document()
            .filter(|d| !d.is_read_only() && Language::for_path(&d.path) == Some(Language::Python))
            .map(|d| d.project_root().to_path_buf());
        if let Some(root) = root
            && self.python_ui.root.as_ref() != Some(&root)
        {
            self.service.python.cancel();
            self.python_ui.root = Some(root.clone());
            self.python_ui.draft = None;
            self.python_ui.trusted = false;
            self.python_ui.launch = false;
            if let Err(e) = self
                .service
                .dispatch(Command::Python(ProjectCommand::Inspect(root)))
            {
                self.python_ui.error = Some(e);
            }
        }
        if self.python_ui.revision != self.service.python.revision {
            self.python_ui.revision = self.service.python.revision;
            if let Some(config) = &self.service.python.config
                && self.service.python.error.is_none()
                && self.python_ui.root.as_ref() == Some(&config.root)
            {
                self.python_ui.arguments = config.arguments.join("\n");
                self.python_ui.environment = config
                    .environment
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                self.python_ui.draft = Some(config.clone());
            }
        }
        if self.python_ui.launch && !self.service.python.is_busy() {
            self.python_ui.launch = false;
            if let Some(plan) = self.service.python.plan.take() {
                let current = self
                    .service
                    .documents
                    .iter()
                    .map(|d| (d.id, d.version))
                    .collect::<Vec<_>>();
                let changed = plan.action.executes_project()
                    && (current != self.python_ui.versions
                        || self
                            .service
                            .documents
                            .iter()
                            .any(|d| d.is_dirty() && d.path.starts_with(&plan.root)));
                if !self.python_ui.trusted
                    || self.python_ui.root.as_ref() != Some(&plan.root)
                    || changed
                {
                    self.python_ui.error =
                        Some("Projeto ou arquivos mudaram; revise a execução novamente".into());
                } else {
                    let key = format!(
                        "python:{}:{:?}:{:?}:{:?}",
                        plan.program.display(),
                        plan.cwd,
                        plan.args,
                        plan.env
                    );
                    if self.terminal.open_command_environment(
                        &plan.cwd,
                        plan.program,
                        plan.args,
                        format!("Python · {}", plan.action.label()),
                        key.clone(),
                        plan.env,
                    ) {
                        if matches!(
                            plan.action,
                            Action::CreateVenv
                                | Action::InstallPackages
                                | Action::InstallRequirements
                        ) {
                            self.python_ui.refresh = Some((key, plan.root));
                        }
                        self.show_activity = true;
                        self.tool_tab = ToolTab::Terminal;
                        self.python_ui.open = false;
                    } else {
                        self.python_ui.error =
                            Some("Limite de terminais atingido; encerre uma sessão".into());
                    }
                }
            }
        }
        if self.service.python.is_busy() || self.python_ui.refresh.is_some() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    pub(super) fn open_python(&mut self, action: Action) {
        let root = self
            .service
            .active_document()
            .map(|d| d.project_root().to_path_buf())
            .or_else(|| self.service.workspace_roots().first().cloned());
        let Some(root) = root else {
            self.service.last_error = Some("Abra um projeto Python".into());
            return;
        };
        self.python_ui.open = true;
        self.python_ui.action = action;
        self.python_ui.trusted = false;
        self.python_ui.error = None;
        if self.python_ui.root.as_ref() != Some(&root) || self.python_ui.draft.is_none() {
            self.python_ui.root = Some(root.clone());
            if !self.service.python.is_busy() {
                let _ = self
                    .service
                    .dispatch(Command::Python(ProjectCommand::Inspect(root)));
            }
        }
    }
    pub(super) fn python_dialog(&mut self, ctx: &egui::Context) {
        if !self.python_ui.open {
            return;
        }
        let busy = self.service.python.is_busy();
        let file = self.python_file();
        let mut close = false;
        let mut execute = false;
        let mut save = false;
        let mut refresh = false;
        let before = self.python_ui.draft.clone();
        let before_action = self.python_ui.action;
        let before_values = (
            self.python_ui.arguments.clone(),
            self.python_ui.environment.clone(),
            self.python_ui.values.clone(),
        );
        // Anchor the fixed-height form from its top edge. Centering an Area from
        // the previous measured height can move the footer under the pointer
        // while the first layout settles (especially on compact viewports).
        let viewport = ctx.content_rect();
        let width = (viewport.width() - 80.).clamp(260., 700.);
        let body_height = (viewport.height() - 220.).clamp(110., 480.);
        let offset = egui::vec2(
            ((viewport.width() - width - 32.) / 2.).max(12.),
            ((viewport.height() - body_height - 166.) / 2.).max(12.),
        );
        let area = egui::Modal::default_area(egui::Id::unique("python.project.tools"))
            .anchor(egui::Align2::LEFT_TOP, offset);
        let modal=egui::Modal::new(egui::Id::unique("python.project.tools")).area(area).frame(egui::Frame::window(&ctx.global_style()).inner_margin(16)).show(ctx,|ui|{
            self.tokens.form_style(ui);ui.set_width(width);
            ui.horizontal(|ui|{ui.strong("Python · Executar e ambiente");ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=icon_button(ui,Icon::Close,"Fechar Python",false,26.).clicked();});});ui.separator();
            egui::ScrollArea::vertical().id_salt("python.project.scroll").max_height(body_height).auto_shrink([false,false]).codisseu().show(ui,|ui|{
                if busy{ui.spinner();ui.label("Preparando configuração Python…");}
                let Some(config)=self.python_ui.draft.as_mut()else{return;};
                ui.add_enabled_ui(!busy,|ui|{
                    ui.label(config.root.display().to_string());
                    if let Some(interpreter)=&self.service.python.interpreter{ui.add(egui::Label::new(format!("Interpretador detectado: {}",interpreter.display())).wrap());}
                    ui.add_space(8.);ui.horizontal_wrapped(|ui|{for action in Action::ALL{ui.selectable_value(&mut self.python_ui.action,action,action.label());}});ui.add_space(8.);
                    for(label,value,hint)in[("Interpretador Python",&mut config.interpreter,"Automático: .venv / venv / Python do Codisseu"),("Diretório de execução",&mut config.cwd,"Raiz do projeto (automático)")]{let label=ui.label(label);ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY).char_limit(4096)).labelled_by(label.id);}
                    match self.python_ui.action {
                        Action::RunFile|Action::Check|Action::Lint=>{ui.label(file.as_ref().map_or_else(||"Selecione uma fonte .py/.pyw editável".into(),|p|p.display().to_string()));},
                        Action::RunModule=>{let l=ui.label("Módulo Python (-m)");ui.add(egui::TextEdit::singleline(&mut config.module).hint_text("pacote.main").desired_width(f32::INFINITY)).labelled_by(l.id);},
                        Action::CreateVenv=>{let l=ui.label("Pasta do ambiente virtual");ui.add(egui::TextEdit::singleline(&mut config.venv).desired_width(f32::INFINITY)).labelled_by(l.id);ui.label("Cria uma pasta nova; ambiente existente não é sobrescrito.");},
                        Action::InstallPackages|Action::InstallRequirements=>{let text=if self.python_ui.action==Action::InstallPackages{"Pacotes/requisitos (um por linha)"}else{"Arquivo requirements do projeto"};let l=ui.label(text);ui.add(egui::TextEdit::multiline(&mut self.python_ui.values).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(l.id);ui.add(egui::Label::new("Instala apenas no ambiente virtual selecionado. A instalação pode baixar dependências e executar o build dos pacotes.").wrap());},
                        Action::Repl=>{ui.label("Console interativo Python no terminal integrado; Ctrl+D encerra.");},
                    }
                    if matches!(self.python_ui.action,Action::RunFile|Action::RunModule){let l=ui.label("Argumentos Python (um por linha)");ui.add(egui::TextEdit::multiline(&mut self.python_ui.arguments).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(l.id);}
                    let l=ui.label("Ambiente Python (NOME=valor, um por linha)");ui.add(egui::TextEdit::multiline(&mut self.python_ui.environment).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(l.id);
                    ui.add_space(8.);ui.add(egui::Label::new("Execução usa arquivos salvos. O interpretador escolhido também será usado pelo analisador, pelos testes e pela depuração deste projeto. Criar/instalar só ocorre ao executar a ação escolhida.").wrap());
                });
                if let Some(error)=self.python_ui.error.as_ref().or(self.service.python.error.as_ref()){ui.colored_label(color(self.tokens.error),error);}
            });ui.separator();
            ui.add_enabled(!busy,egui::Checkbox::new(&mut self.python_ui.trusted,"Confio neste projeto e autorizo esta ação Python"));
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!busy,self.tokens.dialog_button("Atualizar ambiente",false)).clicked(){refresh=true;}
                if ui.add_enabled(!busy&&self.python_ui.draft.is_some(),self.tokens.dialog_button("Salvar configuração Python",false)).clicked(){save=true;}
                if ui.add(self.tokens.dialog_button("Cancelar",false)).clicked(){close=true;}
                let needs_file=matches!(self.python_ui.action,Action::RunFile|Action::Check|Action::Lint);
                if ui.add_enabled(!busy&&self.python_ui.trusted&&self.python_ui.draft.is_some()&&(!needs_file||file.is_some()),self.tokens.dialog_button("Executar ação Python",true)).clicked(){execute=true;}
            });
        });
        if before != self.python_ui.draft
            || before_action != self.python_ui.action
            || before_values
                != (
                    self.python_ui.arguments.clone(),
                    self.python_ui.environment.clone(),
                    self.python_ui.values.clone(),
                )
        {
            self.python_ui.trusted = false;
        }
        if refresh && let Some(root) = self.python_ui.root.clone() {
            let _ = self
                .service
                .dispatch(Command::Python(ProjectCommand::Inspect(root)));
        }
        if (execute || save)
            && let Some(mut config) = self.python_ui.draft.clone()
        {
            config.arguments = self
                .python_ui
                .arguments
                .lines()
                .map(str::to_owned)
                .collect();
            let mut environment = std::collections::BTreeMap::new();
            let mut error = None;
            for line in self
                .python_ui
                .environment
                .lines()
                .filter(|l| !l.trim().is_empty())
            {
                if let Some((k, v)) = line.split_once('=') {
                    environment.insert(k.into(), v.into());
                } else {
                    error = Some("Use NOME=valor nas variáveis de ambiente".to_string());
                }
            }
            config.environment = environment;
            if let Some(error) = error {
                self.python_ui.error = Some(error);
            } else {
                self.python_ui.versions = self
                    .service
                    .documents
                    .iter()
                    .map(|d| (d.id, d.version))
                    .collect();
                let command = if execute {
                    ProjectCommand::Prepare {
                        config,
                        action: self.python_ui.action,
                        path: file,
                        values: self
                            .python_ui
                            .values
                            .lines()
                            .filter(|s| !s.trim().is_empty())
                            .map(str::to_owned)
                            .collect(),
                    }
                } else {
                    ProjectCommand::Save(config)
                };
                match self.service.dispatch(Command::Python(command)) {
                    Ok(()) => {
                        self.python_ui.launch = execute;
                        self.python_ui.error = None;
                    }
                    Err(error) => self.python_ui.error = Some(error),
                }
            }
        }
        if modal.should_close() || close {
            if self.service.python.error.is_some()
                && !self.service.python.is_busy()
                && let Some(root) = self.python_ui.root.clone()
            {
                let _ = self
                    .service
                    .dispatch(Command::Python(ProjectCommand::Inspect(root)));
            }
            self.python_ui.open = false;
            self.python_ui.launch = false;
            self.focus_editor = true;
        }
    }
}
