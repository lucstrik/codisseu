//! Vitest/Jest configuration; results use the shared Run tree and console.
use super::*;
use codisseu_core::js_tests::{Config, Framework, Scope, TestCommand};
#[derive(Default)]
pub(super) struct JsTestsUi {
    pub settings: bool,
    pub draft: Option<Config>,
    pub error: Option<String>,
    revision: u64,
    debug_session: Option<u64>,
}
impl CodisseuApp {
    pub(super) fn open_js_tests(&mut self, settings: bool) {
        self.tool_tab = ToolTab::JsTests;
        self.show_activity = true;
        self.js_tests_ui.settings |= settings;
        let Some(root) = self.service.php_test_root() else {
            self.js_tests_ui.error = Some("Abra um projeto JavaScript/TypeScript".into());
            return;
        };
        if !self.service.js_tests.is_busy()
            && self
                .js_tests_ui
                .draft
                .as_ref()
                .is_none_or(|c| c.root != root)
        {
            self.js_tests_ui.draft = Some(Config::new(root.clone()));
            self.js_test_command(TestCommand::Inspect(root));
        }
    }
    pub(super) fn js_test_command(&mut self, command: TestCommand) {
        self.js_tests_ui.error = None;
        if let Err(e) = self.service.dispatch(Command::JsTests(command)) {
            self.js_tests_ui.error = Some(e.clone());
            self.service.last_error = Some(e);
        }
        self.tool_tab = ToolTab::JsTests;
        self.show_activity = true;
    }
    pub(super) fn run_js_test_file(&mut self) {
        self.open_js_tests(false);
        if self.service.js_tests.is_busy() {
            self.js_tests_ui.settings = true;
            return;
        }
        if let Some(path) = self.service.active_document().map(|d| d.path.clone())
            && let Some(config) = self.js_tests_ui.draft.clone()
        {
            self.js_test_command(TestCommand::Run {
                config,
                scope: Scope::File(path),
            });
        }
    }
    pub(super) fn js_tests_settings(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.service.js_tests.debug_session
            && self.js_tests_ui.debug_session != Some(id)
        {
            self.js_tests_ui.debug_session = Some(id);
            self.debugger_ui.selected_session = Some(id);
            self.tool_tab = ToolTab::Debug;
            self.show_activity = true;
        }
        if self.js_tests_ui.revision != self.service.js_tests.revision {
            self.js_tests_ui.revision = self.service.js_tests.revision;
            if let Some(c) = &self.service.js_tests.config {
                self.js_tests_ui.draft = Some(c.clone());
            }
        }
        if !self.js_tests_ui.settings {
            return;
        }
        let busy = self.service.js_tests.is_busy();
        let mut close = false;
        let mut command = None;
        let modal=egui::Modal::new(egui::Id::unique("js-tests.configuration")).frame(egui::Frame::popup(&ctx.style_of(egui::Theme::Dark)).fill(color(self.tokens.bar))).show(ctx,|ui|{
            ui.set_width(560_f32.min((ctx.content_rect().width()-64.).max(240.)));self.tokens.form_style(ui);
            ui.horizontal(|ui|{ui.strong("Run/Debug Configurations · Vitest / Jest");ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{if icon_button(ui,Icon::Close,"Fechar configuração de testes JS/TS",false,24.).clicked(){close=true;}});});ui.separator();
            let Some(config)=self.js_tests_ui.draft.as_mut()else{ui.label("Abra um projeto JavaScript/TypeScript.");return;};
            egui::ScrollArea::vertical().id_salt("js-tests.settings.scroll").max_height((ctx.content_rect().height()-230.).max(100.)).codisseu().show(ui,|ui|{
                ui.label("Diretório do projeto");ui.label(config.root.display().to_string());ui.add_space(8.);
                ui.add_enabled_ui(!busy,|ui|{
                    ui.horizontal(|ui|{let label=ui.label("Framework JS/TS");let combo=egui::ComboBox::from_id_salt("js-tests.framework").selected_text(config.framework.label()).show_ui(ui,|ui|{for framework in [Framework::Auto,Framework::Vitest,Framework::Jest]{ui.selectable_value(&mut config.framework,framework,framework.label());}});combo.response.labelled_by(label.id);});
                    for (label,value,hint) in [("Node.js",&mut config.node,"Automático · runtime do Codisseu"),("Runner JS/TS",&mut config.runner,"node_modules/vitest/vitest.mjs ou jest/bin/jest.js"),("Configuração JS/TS",&mut config.configuration,"vitest.config.ts ou jest.config.cjs (automático)")]{ui.label(label);ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY).char_limit(4096)).widget_info(||egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput,true,label));}
                    ui.horizontal(|ui|{ui.label("Prazo máximo (segundos):");ui.add(egui::DragValue::new(&mut config.timeout_secs).range(1..=3600));});
                });
                ui.add_space(8.);ui.label(RichText::new("Executar inicia o framework/configuração deste projeto. Rerun Automatically é ativado na toolbar após a primeira execução e reage ao Save.").small().color(color(self.tokens.muted)));
                if let Some(error)=self.js_tests_ui.error.as_ref().or(self.service.js_tests.report.error.as_ref()){ui.colored_label(color(self.tokens.error),error);}
            });ui.separator();
            ui.horizontal_wrapped(|ui|{
                if ui.add_enabled(!busy,self.tokens.dialog_button("Executar suite JS/TS",true)).clicked(){command=Some(TestCommand::Run{config:config.clone(),scope:Scope::Suite});}
                let file=self.service.active_document().filter(|d|d.path.starts_with(&config.root)).map(|d|d.path.clone());
                if ui.add_enabled(!busy&&file.is_some(),self.tokens.dialog_button("Executar arquivo JS/TS",false)).clicked(){command=file.map(|p|TestCommand::Run{config:config.clone(),scope:Scope::File(p)});}
                if ui.add_enabled(!busy,self.tokens.dialog_button("Debug suite JS/TS",false)).clicked(){command=Some(TestCommand::Debug{config:config.clone(),scope:Scope::Suite});}
                if ui.add(self.tokens.dialog_button("Cancelar",false)).clicked(){close=true;}
                if busy{ui.spinner();}
            });
        });
        if modal.should_close() {
            close = true;
        }
        if let Some(command) = command {
            self.js_test_command(command);
            if self.js_tests_ui.error.is_none() {
                close = true;
            }
        }
        if close {
            self.js_tests_ui.settings = false;
        }
    }
}
