//! Python test configuration; results reuse the native shared Run tree and console.
use super::*;
use codisseu_core::python_tests::{Config, Framework, Scope, TestCommand};
#[derive(Default)]
pub(super) struct PythonTestsUi {
    pub settings: bool,
    pub draft: Option<Config>,
    pub error: Option<String>,
    revision: u64,
    trusted: bool,
}
impl CodisseuApp {
    pub(super) fn open_python_tests(&mut self, settings: bool) {
        self.tool_tab = ToolTab::PythonTests;
        self.show_activity = true;
        self.python_tests_ui.settings |= settings;
        let Some(root) = self.service.php_test_root() else {
            self.python_tests_ui.error = Some("Abra um projeto Python".into());
            return;
        };
        if !self.service.python_tests.is_busy()
            && self
                .python_tests_ui
                .draft
                .as_ref()
                .is_none_or(|c| c.root != root)
        {
            self.python_tests_ui.draft = Some(Config::new(root.clone()));
            self.python_tests_ui.trusted = false;
            self.python_test_command(TestCommand::Inspect(root));
        }
    }
    pub(super) fn python_test_command(&mut self, command: TestCommand) {
        self.python_tests_ui.error = None;
        if let Err(error) = self.service.dispatch(Command::PythonTests(command)) {
            self.python_tests_ui.error = Some(error.clone());
            self.service.last_error = Some(error);
        }
        self.tool_tab = ToolTab::PythonTests;
        self.show_activity = true;
    }
    pub(super) fn run_python_test_file(&mut self) {
        self.open_python_tests(true);
    }
    pub(super) fn python_debug_selected_test(&mut self, index: usize) {
        match self.service.python_tests.debug_selected(index) {
            Ok(config) => {
                self.debugger_edit_configuration(config);
                self.debugger_ui.dialog = true;
                self.show_activity = true;
            }
            Err(error) => self.python_tests_ui.error = Some(error),
        }
    }
    pub(super) fn python_tests_settings(&mut self, ctx: &egui::Context) {
        if self.python_tests_ui.revision != self.service.python_tests.revision {
            self.python_tests_ui.revision = self.service.python_tests.revision;
            if let Some(config) = &self.service.python_tests.config {
                self.python_tests_ui.draft = Some(config.clone());
            }
        }
        if !self.python_tests_ui.settings {
            return;
        }
        let busy = self.service.python_tests.is_busy();
        let mut close = false;
        let mut command = None;
        let before = self.python_tests_ui.draft.clone();
        let modal=egui::Modal::new(egui::Id::unique("python.tests.configuration")).frame(egui::Frame::window(&ctx.global_style()).inner_margin(16)).show(ctx,|ui|{
            self.tokens.form_style(ui);ui.set_width((ctx.content_rect().width()-80.).clamp(260.,640.));
            ui.horizontal(|ui|{ui.strong("Python · pytest / unittest");ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{close=icon_button(ui,Icon::Close,"Fechar configuração de testes Python",false,26.).clicked();});});ui.separator();
            egui::ScrollArea::vertical().id_salt("python.tests.settings.scroll").max_height((ctx.content_rect().height()-235.).clamp(100.,380.)).auto_shrink([false,false]).codisseu().show(ui,|ui|{
                let Some(config)=self.python_tests_ui.draft.as_mut()else{ui.label("Preparando configuração Python…");return;};
                ui.label(config.root.display().to_string());ui.add_space(8.);
                ui.add_enabled_ui(!busy,|ui|{
                    ui.horizontal_wrapped(|ui|{let label=ui.label("Framework Python");let combo=egui::ComboBox::from_id_salt("python.tests.framework").selected_text(config.framework.label()).show_ui(ui,|ui|{for f in[Framework::Auto,Framework::Pytest,Framework::Unittest]{ui.selectable_value(&mut config.framework,f,f.label());}});combo.response.labelled_by(label.id);});
                    let l=ui.label("Interpretador dos testes Python");ui.add(egui::TextEdit::singleline(&mut config.interpreter).hint_text("Ambiente Python do projeto (automático)").desired_width(f32::INFINITY)).labelled_by(l.id);
                    ui.horizontal(|ui|{let label=ui.label("Prazo máximo (segundos)");ui.add(egui::DragValue::new(&mut config.timeout_secs).range(1..=3600)).labelled_by(label.id);});
                    ui.add_space(8.);ui.add(egui::Label::new("Executar importa e executa os testes e plugins do projeto. Os resultados aparecem no painel Run, com árvore, saída e navegação. Salve os arquivos modificados antes de executar.").wrap());
                    ui.add(egui::Label::new("O Python do Codisseu já inclui pytest. Para outro ambiente virtual, instale pytest nele ou escolha unittest. Debug de um caso usa o formulário de depuração existente.").wrap());
                });
                if let Some(error)=self.python_tests_ui.error.as_ref().or(self.service.python_tests.report.error.as_ref()){ui.colored_label(color(self.tokens.error),error);}
            });ui.separator();
            ui.add_enabled(!busy,egui::Checkbox::new(&mut self.python_tests_ui.trusted,"Confio neste projeto e autorizo executar os testes Python"));
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!busy&&self.python_tests_ui.trusted,self.tokens.dialog_button("Executar suite Python",true)).clicked()&&let Some(config)=self.python_tests_ui.draft.clone(){command=Some(TestCommand::Run{config,scope:Scope::Suite});}
                let file=self.python_file();
                if ui.add_enabled(!busy&&self.python_tests_ui.trusted&&file.is_some(),self.tokens.dialog_button("Executar arquivo Python",false)).clicked()&&let(Some(config),Some(path))=(self.python_tests_ui.draft.clone(),file){command=Some(TestCommand::Run{config,scope:Scope::File(path)});}
                if ui.add(self.tokens.dialog_button("Cancelar",false)).clicked(){close=true;}
                if busy{ui.spinner();}
            });
        });
        if before != self.python_tests_ui.draft {
            self.python_tests_ui.trusted = false;
            command = None;
        }
        if let Some(command) = command {
            self.python_test_command(command);
            if self.python_tests_ui.error.is_none() {
                close = true;
            }
        }
        if modal.should_close() || close {
            self.python_tests_ui.settings = false;
            self.focus_editor = true;
        }
    }
}
