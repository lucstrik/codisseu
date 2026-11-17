//! PHP test runner: tree and console follow the documented PhpStorm Run layout.
#[cfg(test)]
#[path = "php_tests_tests.rs"]
mod tests;
use super::*;
use codisseu_core::php_tests::{Config, Framework, Outcome, Scope, State, TestCommand};
use std::collections::{BTreeMap, BTreeSet};
macro_rules! test_value {
    ($app:expr,$field:ident) => {
        (if $app.tool_tab == ToolTab::PythonTests {
            &$app.service.python_tests.$field
        } else if $app.tool_tab == ToolTab::JsTests {
            &$app.service.js_tests.$field
        } else {
            &$app.service.php_tests.$field
        })
    };
}

#[derive(Clone)]
struct Row {
    label: String,
    case: Option<usize>,
    key: Option<String>,
    depth: usize,
    outcome: Option<Outcome>,
    seconds: f64,
}
pub(super) struct TestsUi {
    pub settings: bool,
    pub(super) js: bool,
    pub(super) python: bool,
    draft: Option<Config>,
    revision: u64,
    selected: Option<usize>,
    passed: bool,
    skipped: bool,
    sort: u8,
    collapsed: BTreeSet<String>,
    rows: Vec<Row>,
    cache: Option<(u64, bool, bool, u8)>,
    width: Option<f32>,
    console: Option<ConsoleCache>,
    config_error: Option<String>,
}
impl Default for TestsUi {
    fn default() -> Self {
        Self {
            settings: false,
            js: false,
            python: false,
            draft: None,
            revision: u64::MAX,
            selected: None,
            passed: true,
            skipped: true,
            sort: 0,
            collapsed: BTreeSet::new(),
            rows: vec![],
            cache: None,
            width: None,
            console: None,
            config_error: None,
        }
    }
}
struct ConsoleCache {
    key: (u64, usize, Option<usize>),
    text: String,
    lines: Vec<String>,
}
fn millis(seconds: f64) -> String {
    if seconds < 1. {
        format!("{} ms", (seconds * 1000.).round() as u64)
    } else {
        format!("{seconds:.2} s")
    }
}
fn outcome(cases: impl Iterator<Item = Outcome>) -> Option<Outcome> {
    cases.max_by_key(|o| match o {
        Outcome::Passed => 0,
        Outcome::Skipped => 1,
        Outcome::Failed => 2,
        Outcome::Error => 3,
    })
}
fn test_filter_button(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    passed: bool,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(26., 26.), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::accesskit::Role::CheckBox, true, selected, label)
    });
    if selected || response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(2.), 4., color(tokens.border));
    }
    let center = rect.center();
    let stroke = egui::Stroke::new(1.5, color(tokens.muted));
    if passed {
        ui.painter().line_segment(
            [center + egui::vec2(-5., 0.), center + egui::vec2(-1., 4.)],
            stroke,
        );
        ui.painter().line_segment(
            [center + egui::vec2(-1., 4.), center + egui::vec2(6., -4.)],
            stroke,
        );
    } else {
        ui.painter().circle_stroke(center, 7., stroke);
        ui.painter().line_segment(
            [center + egui::vec2(-5., 5.), center + egui::vec2(5., -5.)],
            stroke,
        );
    }
    response.on_hover_text(label)
}
fn state_icon(ui: &egui::Ui, center: egui::Pos2, state: Option<Outcome>, tokens: &Tokens) {
    let p = ui.painter();
    let stroke = egui::Stroke::new(1.6, Color32::WHITE);
    match state {
        Some(Outcome::Passed) => {
            let c = color(tokens.php_test_passed);
            p.line_segment(
                [center + egui::vec2(-4., 0.), center + egui::vec2(-1., 3.)],
                egui::Stroke::new(2., c),
            );
            p.line_segment(
                [center + egui::vec2(-1., 3.), center + egui::vec2(5., -4.)],
                egui::Stroke::new(2., c),
            );
        }
        Some(Outcome::Failed | Outcome::Error) => {
            p.circle_filled(center, 7., color(tokens.php_test_failed));
            p.line_segment(
                [
                    center + egui::vec2(-2.6, -2.6),
                    center + egui::vec2(2.6, 2.6),
                ],
                stroke,
            );
            p.line_segment(
                [
                    center + egui::vec2(-2.6, 2.6),
                    center + egui::vec2(2.6, -2.6),
                ],
                stroke,
            );
        }
        Some(Outcome::Skipped) => {
            p.circle_stroke(center, 6., egui::Stroke::new(1.3, color(tokens.muted)));
            p.line_segment(
                [center + egui::vec2(-4., 4.), center + egui::vec2(4., -4.)],
                egui::Stroke::new(1.3, color(tokens.muted)),
            );
        }
        None => {
            p.circle_stroke(center, 5., egui::Stroke::new(1.3, color(tokens.muted)));
        }
    }
}
impl CodisseuApp {
    pub(super) fn open_php_tests(&mut self, settings: bool) {
        self.tool_tab = ToolTab::PhpTests;
        self.show_activity = true;
        self.php_tests_ui.settings |= settings;
        let Some(root) = self.service.php_test_root() else {
            self.service.last_error = Some("Abra um projeto PHP para executar testes".into());
            return;
        };
        if !self.service.php_tests.is_busy()
            && self
                .php_tests_ui
                .draft
                .as_ref()
                .is_none_or(|c| c.root != root)
        {
            self.php_tests_ui.draft = Some(Config::new(root.clone()));
            self.send(Command::PhpTests(TestCommand::Inspect(root)));
        }
    }
    fn php_test_command(&mut self, command: TestCommand) {
        if self.tool_tab == ToolTab::PythonTests {
            use codisseu_core::python_tests::TestCommand as P;
            let command = match command {
                TestCommand::RunSelected(i) => P::RunSelected(i),
                TestCommand::RerunFailed => P::RerunFailed,
                TestCommand::RerunLast => P::RerunLast,
                TestCommand::Cancel => P::Cancel,
                _ => return,
            };
            self.python_test_command(command);
            return;
        }
        if self.tool_tab == ToolTab::JsTests {
            use codisseu_core::js_tests::TestCommand as J;
            let command = match command {
                TestCommand::RunSelected(i) => J::RunSelected(i),
                TestCommand::RerunFailed => J::RerunFailed,
                TestCommand::RerunLast => J::RerunLast,
                TestCommand::Cancel => J::Cancel,
                _ => return,
            };
            self.js_test_command(command);
            return;
        }
        self.php_tests_ui.config_error = None;
        if let Err(error) = self.service.dispatch(Command::PhpTests(command)) {
            self.php_tests_ui.config_error = Some(error);
        }
        self.tool_tab = ToolTab::PhpTests;
        self.show_activity = true;
    }
    pub(super) fn run_php_test_file(&mut self) {
        let Some(path) = self
            .service
            .active_document()
            .filter(|d| d.path.extension().is_some_and(|e| e == "php"))
            .map(|d| d.path.clone())
        else {
            self.service.last_error = Some("Abra um arquivo PHP de teste".into());
            return;
        };
        self.open_php_tests(false);
        // Initial inspect is asynchronous. Review configuration before first execution.
        if self.service.php_tests.is_busy() {
            self.php_tests_ui.settings = true;
            self.php_tests_ui.config_error =
                Some("Confira a configuração e use Executar arquivo ativo".into());
            return;
        }
        if let Some(config) = self.php_tests_ui.draft.clone() {
            self.php_test_command(TestCommand::Run {
                config,
                scope: Scope::File(path),
            });
        }
    }
    fn sync_php_tests_ui(&mut self) {
        let js = self.tool_tab == ToolTab::JsTests;
        let python = self.tool_tab == ToolTab::PythonTests;
        let revision = if python {
            self.service.python_tests.revision
        } else if js {
            self.service.js_tests.revision
        } else {
            self.service.php_tests.revision
        };
        if self.php_tests_ui.revision == revision
            && self.php_tests_ui.js == js
            && self.php_tests_ui.python == python
        {
            return;
        }
        if self.php_tests_ui.js != js || self.php_tests_ui.python != python {
            self.php_tests_ui.collapsed.clear();
            self.php_tests_ui.console = None;
        }
        self.php_tests_ui.js = js;
        self.php_tests_ui.python = python;
        self.php_tests_ui.revision = revision;
        self.php_tests_ui.cache = None;
        if !js
            && !python
            && let Some(config) = &self.service.php_tests.config
        {
            self.php_tests_ui.draft = Some(config.clone());
        }
        let report = if python {
            &self.service.python_tests.report
        } else if js {
            &self.service.js_tests.report
        } else {
            &self.service.php_tests.report
        };
        self.php_tests_ui.selected = if !self.tests_busy()
            && report.error.is_none()
            && (if python {
                self.service.python_tests.state
            } else if js {
                self.service.js_tests.state
            } else {
                self.service.php_tests.state
            }) != State::Cancelled
        {
            report.cases.iter().position(|c| c.outcome.is_failure())
        } else {
            None
        };
    }
    fn tests_busy(&self) -> bool {
        if self.tool_tab == ToolTab::PythonTests {
            self.service.python_tests.is_busy()
        } else if self.tool_tab == ToolTab::JsTests {
            self.service.js_tests.is_busy()
        } else {
            self.service.php_tests.is_busy()
        }
    }
    fn tests_last_root(&self) -> Option<&std::path::Path> {
        if self.tool_tab == ToolTab::PythonTests {
            self.service.python_tests.last_root().or_else(|| {
                self.service
                    .python_tests
                    .config
                    .as_ref()
                    .map(|c| c.root.as_path())
            })
        } else if self.tool_tab == ToolTab::JsTests {
            self.service.js_tests.last_root()
        } else {
            self.service.php_tests.last_root()
        }
    }
    fn tests_framework(&self) -> &'static str {
        if self.tool_tab == ToolTab::PythonTests {
            self.service
                .python_tests
                .config
                .as_ref()
                .map_or("pytest / unittest", |c| c.framework.label())
        } else if self.tool_tab == ToolTab::JsTests {
            self.service
                .js_tests
                .config
                .as_ref()
                .map_or("Vitest / Jest", |c| c.framework.label())
        } else {
            self.service
                .php_tests
                .config
                .as_ref()
                .map_or("PHPUnit / Pest", |c| c.framework.label())
        }
    }
    fn open_current_tests(&mut self) {
        if self.tool_tab == ToolTab::PythonTests {
            self.open_python_tests(true)
        } else if self.tool_tab == ToolTab::JsTests {
            self.open_js_tests(true)
        } else {
            self.open_php_tests(true)
        }
    }
    fn run_current_test_file(&mut self) {
        if self.tool_tab == ToolTab::PythonTests {
            self.run_python_test_file()
        } else if self.tool_tab == ToolTab::JsTests {
            self.run_js_test_file()
        } else {
            self.run_php_test_file()
        }
    }
    pub(super) fn php_tests_settings(&mut self, ctx: &egui::Context) {
        if self.tool_tab == ToolTab::PhpTests {
            self.sync_php_tests_ui();
        }
        if !self.php_tests_ui.settings {
            return;
        }
        let busy = self.service.php_tests.is_busy();
        let mut close = false;
        let mut run = None;
        let result = egui::Modal::new(egui::Id::unique("php-tests.configuration")).frame(egui::Frame::popup(&ctx.style_of(egui::Theme::Dark)).fill(color(self.tokens.bar))).show(ctx, |ui| {
            ui.set_width(560_f32.min((ctx.content_rect().width() - 64.).max(240.)));
            self.tokens.form_style(ui);
            ui.horizontal(|ui| { ui.strong("Run/Debug Configurations · PHP Tests"); ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| { if icon_button(ui, Icon::Close, "Fechar configuração de testes", false, 24.).clicked() { close = true; } }); });
            ui.separator();
            let Some(config) = self.php_tests_ui.draft.as_mut() else { ui.label("Abra um projeto PHP."); return; };
            egui::ScrollArea::vertical().id_salt("php-tests.settings.scroll").max_height((ctx.content_rect().height() - 230.).max(100.)).codisseu().show(ui, |ui| {
                ui.label("Diretório do projeto"); ui.label(config.root.display().to_string());
                ui.add_space(8.);
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Framework");
                        let combo = egui::ComboBox::from_id_salt("php-tests.framework").selected_text(config.framework.label()).show_ui(ui, |ui| {
                            for framework in [Framework::Auto, Framework::PhpUnit, Framework::Pest] { ui.selectable_value(&mut config.framework, framework, framework.label()); }
                        });
                        combo.response.labelled_by(label.id);
                    });
                    for (label, value, hint) in [("Interpretador PHP", &mut config.php, "Automático · PHP do Codisseu ou PATH"), ("Runner", &mut config.runner, "vendor/bin/pest ou vendor/bin/phpunit"), ("Configuração XML", &mut config.xml, "phpunit.xml ou phpunit.xml.dist (automático)")] {
                        ui.label(label);
                        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY).char_limit(4096)).widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, label));
                    }
                    ui.horizontal(|ui| { ui.label("Prazo máximo (segundos):"); ui.add(egui::DragValue::new(&mut config.timeout_secs).range(1..=3600)); });
                });
                ui.add_space(8.);
                ui.label(RichText::new("Executar inicia o runner e o bootstrap deste projeto. Salve as alterações antes de executar.").small().color(color(self.tokens.muted)));
                if let Some(error) = self.php_tests_ui.config_error.as_ref().or(self.service.php_tests.report.error.as_ref()) { ui.colored_label(color(self.tokens.error), error); }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, self.tokens.dialog_button("Executar suite", true)).clicked() { run = Some(Scope::Suite); }
                let file = self.service.active_document().filter(|d| d.path.starts_with(&config.root) && d.path.extension().is_some_and(|e| e == "php")).map(|d| d.path.clone());
                if ui.add_enabled(!busy && file.is_some(), self.tokens.dialog_button("Executar arquivo ativo", false)).clicked() { run = file.map(Scope::File); }
                if ui.add(self.tokens.dialog_button("Cancelar", false)).clicked() { close = true; }
                if busy { ui.spinner(); }
            });
        });
        if result.should_close() {
            close = true;
        }
        if let Some(scope) = run
            && let Some(config) = self.php_tests_ui.draft.clone()
        {
            self.php_test_command(TestCommand::Run { config, scope });
            if self.php_tests_ui.config_error.is_none() {
                close = true;
            }
        }
        if close {
            self.php_tests_ui.settings = false;
        }
    }
    pub(super) fn php_tests_panel(&mut self, ui: &mut egui::Ui) {
        self.sync_php_tests_ui();
        let busy = self.tests_busy();
        let mut action = None;
        header(ui, self.tokens.php_test_toolbar_height, |ui| {
            ui.add_space(4.);
            let run = ui
                .add_enabled_ui(!busy, |ui| {
                    icon_button(ui, Icon::Refresh, "Rerun · Ctrl+F5", false, 26.)
                })
                .inner
                .clicked();
            if run {
                if self.tests_last_root().is_some() {
                    action = Some(TestCommand::RerunLast);
                } else {
                    self.open_current_tests();
                }
            }
            if ui
                .add_enabled_ui(
                    if self.tool_tab == ToolTab::PythonTests {
                        self.service.python_tests.can_rerun_failed()
                    } else if self.tool_tab == ToolTab::JsTests {
                        self.service.js_tests.can_rerun_failed()
                    } else {
                        self.service.php_tests.can_rerun_failed()
                    },
                    |ui| icon_button(ui, Icon::Problems, "Rerun Failed Tests", false, 26.),
                )
                .inner
                .clicked()
            {
                action = Some(TestCommand::RerunFailed);
            }
            if ui
                .add_enabled_ui(!busy && self.php_tests_ui.selected.is_some(), |ui| {
                    icon_button(ui, Icon::Play, "Run selected test", false, 26.)
                })
                .inner
                .clicked()
            {
                action = self.php_tests_ui.selected.map(TestCommand::RunSelected);
            }
            if ui
                .add_enabled_ui(busy, |ui| {
                    icon_button(ui, Icon::Stop, "Stop · Ctrl+F2", false, 26.)
                })
                .inner
                .clicked()
            {
                action = Some(TestCommand::Cancel);
            }
            if self.tool_tab == ToolTab::PythonTests
                && ui
                    .add_enabled_ui(!busy && self.php_tests_ui.selected.is_some(), |ui| {
                        icon_button(ui, Icon::Debug, "Debug selected Python test", false, 26.)
                    })
                    .inner
                    .clicked()
                && let Some(index) = self.php_tests_ui.selected
            {
                self.python_debug_selected_test(index);
            }
            if self.tool_tab == ToolTab::JsTests {
                if ui
                    .add_enabled_ui(!busy && self.php_tests_ui.selected.is_some(), |ui| {
                        icon_button(ui, Icon::Debug, "Debug selected test", false, 26.)
                    })
                    .inner
                    .clicked()
                    && let Some(i) = self.php_tests_ui.selected
                {
                    self.js_test_command(codisseu_core::js_tests::TestCommand::DebugSelected(i));
                }
                let armed = self.service.js_tests.watching;
                if icon_button(
                    ui,
                    Icon::Refresh,
                    "Rerun Automatically · after Save",
                    armed,
                    26.,
                )
                .clicked()
                {
                    self.js_test_command(codisseu_core::js_tests::TestCommand::Watch(!armed));
                }
                if armed {
                    ui.label(
                        RichText::new(if self.service.js_tests.watch_pending {
                            "Save · pendente"
                        } else {
                            "Save · auto"
                        })
                        .small()
                        .color(color(self.tokens.muted)),
                    );
                }
            }
            ui.separator();
            if test_filter_button(
                ui,
                "Show Passed",
                self.php_tests_ui.passed,
                true,
                &self.tokens,
            )
            .clicked()
            {
                self.php_tests_ui.passed = !self.php_tests_ui.passed;
            }
            if test_filter_button(
                ui,
                "Show Ignored",
                self.php_tests_ui.skipped,
                false,
                &self.tokens,
            )
            .clicked()
            {
                self.php_tests_ui.skipped = !self.php_tests_ui.skipped;
            }
            if icon_button(ui, Icon::ExpandAll, "Expand All Tests", false, 26.).clicked() {
                self.php_tests_ui.collapsed.clear();
                self.php_tests_ui.cache = None;
            }
            if icon_button(ui, Icon::CollapseAll, "Collapse All Tests", false, 26.).clicked() {
                self.php_tests_ui.collapsed.insert("all".into());
                self.php_tests_ui.cache = None;
            }
            ui.menu_button("⋮", |ui| {
                ui.selectable_value(&mut self.php_tests_ui.sort, 0, "By Declaration Order");
                ui.selectable_value(&mut self.php_tests_ui.sort, 1, "Sort Alphabetically");
                ui.selectable_value(&mut self.php_tests_ui.sort, 2, "Sort By Duration");
                ui.separator();
                if ui
                    .add_enabled(!busy, egui::Button::new("Executar suite"))
                    .clicked()
                {
                    self.open_current_tests();
                    ui.close();
                }
                if ui
                    .add_enabled(!busy, egui::Button::new("Executar arquivo de testes"))
                    .clicked()
                {
                    self.run_current_test_file();
                    ui.close();
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Settings, "Configurar testes", false, 26.).clicked() {
                    self.open_current_tests();
                }
                let root = self
                    .tests_last_root()
                    .or_else(|| self.php_tests_ui.draft.as_ref().map(|c| c.root.as_path()));
                if let Some(root) = root {
                    ui.label(
                        RichText::new(root.file_name().unwrap_or_default().to_string_lossy())
                            .color(color(self.tokens.muted)),
                    )
                    .on_hover_text(root.display().to_string());
                }
                if busy {
                    ui.spinner();
                }
            });
        });
        if let Some(error) = if self.tool_tab == ToolTab::PythonTests {
            &self.python_tests_ui.error
        } else if self.tool_tab == ToolTab::JsTests {
            &self.js_tests_ui.error
        } else {
            &self.php_tests_ui.config_error
        } {
            ui.colored_label(color(self.tokens.error), error);
        }
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 180. || rect.height() < 20. {
            return;
        }
        let width = self
            .php_tests_ui
            .width
            .unwrap_or(rect.width() * self.tokens.php_test_tree_fraction)
            .clamp(120., (rect.width() - 120.).max(120.));
        let divider = egui::Rect::from_min_size(
            egui::pos2(rect.left() + width, rect.top()),
            egui::vec2(5., rect.height()),
        );
        let response = ui.interact(
            divider,
            ui.scope_id().with("php-tests.splitter"),
            egui::Sense::drag(),
        );
        if response.dragged() {
            self.php_tests_ui.width = Some(
                (width + response.drag_delta().x).clamp(120., (rect.width() - 120.).max(120.)),
            );
        }
        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        ui.painter().line_segment(
            [divider.left_top(), divider.left_bottom()],
            egui::Stroke::new(1., color(self.tokens.border)),
        );
        let tree_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(divider.left(), rect.bottom()));
        let console_rect =
            egui::Rect::from_min_max(egui::pos2(divider.right(), rect.top()), rect.max);
        let mut tree = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("php-tests.tree")
                .max_rect(tree_rect),
        );
        tree.set_clip_rect(tree_rect.intersect(ui.clip_rect()));
        self.php_test_tree(&mut tree);
        let mut console = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("php-tests.console")
                .max_rect(console_rect),
        );
        console.set_clip_rect(console_rect.intersect(ui.clip_rect()));
        self.php_test_console(&mut console);
        ui.allocate_rect(rect, egui::Sense::hover());
        if let Some(action) = action {
            self.php_test_command(action);
        }
    }
    fn php_test_tree(&mut self, ui: &mut egui::Ui) {
        let key = (
            *test_value!(self, revision),
            self.php_tests_ui.passed,
            self.php_tests_ui.skipped,
            self.php_tests_ui.sort,
        );
        if self.php_tests_ui.cache != Some(key) {
            self.php_tests_ui.rows = self.php_test_rows();
            self.php_tests_ui.cache = Some(key);
        }
        let mut toggle = None;
        let mut open = None;
        let mut run = None;
        egui::ScrollArea::vertical()
            .id_salt("php-tests.tree.rows")
            .auto_shrink([false, false])
            .codisseu()
            .show_rows(
                ui,
                self.tokens.php_test_tree_row_height,
                self.php_tests_ui.rows.len(),
                |ui, range| {
                    for n in range {
                        let row = &self.php_tests_ui.rows[n];
                        let selected = if let Some(case) = row.case {
                            self.php_tests_ui.selected == Some(case)
                        } else {
                            self.php_tests_ui.selected.is_none()
                                && row.key.as_deref() == Some("all")
                        };
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), self.tokens.php_test_tree_row_height),
                            egui::Sense::click(),
                        );
                        let label = if let Some(case) = row.case {
                            format!("Teste {}", test_value!(self, report).cases[case].name)
                        } else {
                            row.label.clone()
                        };
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::accesskit::Role::Button,
                                true,
                                selected,
                                label.as_str(),
                            )
                        });
                        if selected || response.hovered() {
                            ui.painter().rect_filled(
                                rect.shrink2(egui::vec2(4., 0.)),
                                4.,
                                color(if selected {
                                    self.tokens.selection
                                } else {
                                    self.tokens.border
                                }),
                            );
                        }
                        let x = rect.left() + 8. + row.depth as f32 * self.tokens.php_test_indent;
                        if let Some(group) = &row.key {
                            ui.painter().text(
                                egui::pos2(x, rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                if self.php_tests_ui.collapsed.contains(group) {
                                    "›"
                                } else {
                                    "⌄"
                                },
                                egui::FontId::proportional(13.),
                                color(self.tokens.muted),
                            );
                        }
                        state_icon(
                            ui,
                            egui::pos2(x + 24., rect.center().y),
                            row.outcome,
                            &self.tokens,
                        );
                        let duration = millis(row.seconds);
                        let duration_rect = egui::Rect::from_min_max(
                            egui::pos2((rect.right() - 65.).max(x + 40.), rect.top()),
                            rect.right_bottom(),
                        );
                        let text_rect = egui::Rect::from_min_max(
                            egui::pos2(x + 36., rect.top()),
                            egui::pos2(duration_rect.left() - 4., rect.bottom()),
                        );
                        ui.painter()
                            .with_clip_rect(text_rect.intersect(ui.clip_rect()))
                            .text(
                                text_rect.left_center(),
                                egui::Align2::LEFT_CENTER,
                                &row.label,
                                egui::FontId::proportional(13.),
                                color(self.tokens.text),
                            );
                        ui.painter()
                            .with_clip_rect(duration_rect.intersect(ui.clip_rect()))
                            .text(
                                rect.right_center() - egui::vec2(8., 0.),
                                egui::Align2::RIGHT_CENTER,
                                duration,
                                egui::FontId::proportional(12.),
                                color(self.tokens.muted),
                            );
                        if response.clicked() {
                            self.php_tests_ui.selected = row.case;
                            if let Some(key) = &row.key {
                                toggle = Some(key.clone());
                            }
                        }
                        if response.double_clicked() {
                            open = row.case;
                        }
                        let case = row.case;
                        response.context_menu(|ui| {
                            if ui
                                .add_enabled(
                                    case.is_some() && !self.tests_busy(),
                                    egui::Button::new("Executar este teste"),
                                )
                                .clicked()
                            {
                                run = case;
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    case.is_some(),
                                    egui::Button::new("Abrir fonte do teste"),
                                )
                                .clicked()
                            {
                                open = case;
                                ui.close();
                            }
                        });
                        response.on_hover_text(&row.label);
                    }
                },
            );
        if let Some(key) = toggle {
            if !self.php_tests_ui.collapsed.remove(&key) {
                self.php_tests_ui.collapsed.insert(key);
            }
            self.php_tests_ui.cache = None;
        }
        if let Some(index) = open {
            self.php_test_open_source(index);
        }
        if let Some(index) = run {
            self.php_test_command(TestCommand::RunSelected(index));
        }
    }
    fn python_test_rows(&self) -> Vec<Row> {
        let cases = &self.service.python_tests.report.cases;
        let root = self.tests_last_root();
        let mut files = BTreeMap::<String, Vec<usize>>::new();
        for (index, case) in cases.iter().enumerate().filter(|(_, c)| {
            (self.php_tests_ui.passed || c.outcome != Outcome::Passed)
                && (self.php_tests_ui.skipped || c.outcome != Outcome::Skipped)
        }) {
            let file = case
                .path
                .as_ref()
                .map(|p| {
                    root.and_then(|r| p.strip_prefix(r).ok())
                        .unwrap_or(p)
                        .display()
                        .to_string()
                })
                .unwrap_or_else(|| "Coleta / inicialização".into());
            files.entry(file).or_default().push(index);
        }
        let mut rows = Vec::new();
        for (file, indices) in files {
            let key = format!("python.file:{file}");
            rows.push(Row {
                label: file.clone(),
                case: None,
                key: Some(key.clone()),
                depth: 0,
                outcome: outcome(indices.iter().map(|&i| cases[i].outcome)),
                seconds: indices.iter().map(|&i| cases[i].seconds).sum(),
            });
            if self.php_tests_ui.collapsed.contains(&key) {
                continue;
            }
            let mut classes = BTreeMap::<String, Vec<usize>>::new();
            for index in indices {
                classes
                    .entry(cases[index].class.clone())
                    .or_default()
                    .push(index);
            }
            for (class, mut indices) in classes {
                if self.php_tests_ui.sort == 1 {
                    indices.sort_by(|&a, &b| cases[a].name.cmp(&cases[b].name));
                } else if self.php_tests_ui.sort == 2 {
                    indices.sort_by(|&a, &b| cases[b].seconds.total_cmp(&cases[a].seconds));
                }
                let depth = if class.is_empty() {
                    1
                } else {
                    let key = format!("python.class:{file}:{class}");
                    rows.push(Row {
                        label: class,
                        case: None,
                        key: Some(key.clone()),
                        depth: 1,
                        outcome: outcome(indices.iter().map(|&i| cases[i].outcome)),
                        seconds: indices.iter().map(|&i| cases[i].seconds).sum(),
                    });
                    if self.php_tests_ui.collapsed.contains(&key) {
                        continue;
                    }
                    2
                };
                for index in indices {
                    let c = &cases[index];
                    rows.push(Row {
                        label: if c.title.is_empty() {
                            c.name.clone()
                        } else {
                            c.title.clone()
                        },
                        case: Some(index),
                        key: None,
                        depth,
                        outcome: Some(c.outcome),
                        seconds: c.seconds,
                    });
                }
            }
        }
        rows
    }
    fn php_test_rows(&self) -> Vec<Row> {
        if self.tool_tab == ToolTab::PythonTests {
            return self.python_test_rows();
        }
        let cases = &test_value!(self, report).cases;
        if matches!(self.tool_tab, ToolTab::JsTests | ToolTab::PythonTests) && !cases.is_empty() {
            let mut rows = vec![];
            let mut files = BTreeMap::<String, Vec<usize>>::new();
            for (i, c) in cases.iter().enumerate().filter(|(_, c)| {
                (self.php_tests_ui.passed || c.outcome != Outcome::Passed)
                    && (self.php_tests_ui.skipped || c.outcome != Outcome::Skipped)
            }) {
                files.entry(c.class.clone()).or_default().push(i);
            }
            for (file, mut indices) in files {
                match self.php_tests_ui.sort {
                    1 => indices.sort_by(|a, b| cases[*a].name.cmp(&cases[*b].name)),
                    2 => indices.sort_by(|a, b| cases[*b].seconds.total_cmp(&cases[*a].seconds)),
                    _ => {}
                }
                let key = format!("class:{file}");
                rows.push(Row {
                    label: file,
                    case: None,
                    key: Some(key.clone()),
                    depth: 0,
                    outcome: outcome(indices.iter().map(|i| cases[*i].outcome)),
                    seconds: indices.iter().map(|i| cases[*i].seconds).sum(),
                });
                if !self.php_tests_ui.collapsed.contains("all")
                    && !self.php_tests_ui.collapsed.contains(&key)
                {
                    js_suite_rows(
                        &mut rows,
                        cases,
                        &indices,
                        &[],
                        1,
                        &key,
                        &self.php_tests_ui.collapsed,
                    );
                }
            }
            return rows;
        }
        let mut rows = vec![Row {
            label: "Test Results".into(),
            case: None,
            key: Some("all".into()),
            depth: 0,
            outcome: outcome(cases.iter().map(|c| c.outcome)),
            seconds: cases.iter().map(|c| c.seconds).sum(),
        }];
        if self.php_tests_ui.collapsed.contains("all") {
            return rows;
        }
        if !cases.is_empty() {
            rows.push(Row {
                label: self.tests_framework().to_owned(),
                case: None,
                key: Some("configuration".into()),
                depth: 1,
                outcome: outcome(cases.iter().map(|c| c.outcome)),
                seconds: cases.iter().map(|c| c.seconds).sum(),
            });
            if self.php_tests_ui.collapsed.contains("configuration") {
                return rows;
            }
        }
        let mut groups = BTreeMap::<String, Vec<usize>>::new();
        for (i, case) in cases.iter().enumerate().filter(|(_, c)| {
            (self.php_tests_ui.passed || c.outcome != Outcome::Passed)
                && (self.php_tests_ui.skipped || c.outcome != Outcome::Skipped)
        }) {
            groups.entry(case.class.clone()).or_default().push(i);
        }
        for (class, mut indices) in groups {
            let group = format!("class:{class}");
            rows.push(Row {
                label: class,
                key: Some(group.clone()),
                case: None,
                depth: 2,
                outcome: outcome(indices.iter().map(|i| cases[*i].outcome)),
                seconds: indices.iter().map(|i| cases[*i].seconds).sum(),
            });
            if self.php_tests_ui.collapsed.contains(&group) {
                continue;
            }
            match self.php_tests_ui.sort {
                1 => indices.sort_by(|a, b| cases[*a].name.cmp(&cases[*b].name)),
                2 => indices.sort_by(|a, b| cases[*b].seconds.total_cmp(&cases[*a].seconds)),
                _ => {}
            }
            if self.tool_tab == ToolTab::JsTests {
                js_suite_rows(
                    &mut rows,
                    cases,
                    &indices,
                    &[],
                    3,
                    &group,
                    &self.php_tests_ui.collapsed,
                );
                continue;
            }
            let mut seen = BTreeSet::new();
            for &i in &indices {
                let case = &cases[i];
                if let Some((method, _)) = case.name.split_once(" with data set ") {
                    if !seen.insert(method.to_owned()) {
                        continue;
                    }
                    let children: Vec<_> = indices
                        .iter()
                        .copied()
                        .filter(|j| {
                            cases[*j]
                                .name
                                .split_once(" with data set ")
                                .is_some_and(|(m, _)| m == method)
                        })
                        .collect();
                    let key = format!("dataset:{}::{method}", case.class);
                    rows.push(Row {
                        label: method.to_owned(),
                        case: None,
                        key: Some(key.clone()),
                        depth: 3,
                        outcome: outcome(children.iter().map(|j| cases[*j].outcome)),
                        seconds: children.iter().map(|j| cases[*j].seconds).sum(),
                    });
                    if !self.php_tests_ui.collapsed.contains(&key) {
                        for j in children {
                            let c = &cases[j];
                            rows.push(Row {
                                label: format!(
                                    "with data set {}",
                                    c.name.split_once(" with data set ").unwrap().1
                                ),
                                case: Some(j),
                                key: None,
                                depth: 4,
                                outcome: Some(c.outcome),
                                seconds: c.seconds,
                            });
                        }
                    }
                } else {
                    rows.push(Row {
                        label: case.name.clone(),
                        case: Some(i),
                        key: None,
                        depth: 3,
                        outcome: Some(case.outcome),
                        seconds: case.seconds,
                    });
                }
            }
        }
        rows
    }
    fn php_test_open_source(&mut self, index: usize) {
        if let Some(case) = test_value!(self, report).cases.get(index)
            && let Some(path) = &case.path
        {
            self.jump_to = Some((path.clone(), case.line.unwrap_or(1).saturating_sub(1), 0));
            self.send(Command::OpenFile(path.clone()));
            self.focus_editor = true;
        }
    }
    fn php_test_console(&mut self, ui: &mut egui::Ui) {
        let report = &test_value!(self, report);
        let [passed, failed, errors, skipped] = report.counts();
        let state = *test_value!(self, state);
        let summary = if self.tests_busy()
            || report.cases.is_empty()
            || matches!(state, State::Cancelled | State::Error)
        {
            state.label().to_owned()
        } else {
            format!(
                "Tests failed: {}, passed: {passed} of {} tests{} – {}",
                failed + errors,
                report.cases.len(),
                if skipped > 0 {
                    format!(", ignored: {skipped}")
                } else {
                    String::new()
                },
                millis(report.seconds)
            )
        };
        let mut console = false;
        let mut copy = false;
        header(ui, self.tokens.php_test_toolbar_height, |ui| {
            ui.add_space(8.);
            let width = (ui.available_width() - 62.).max(20.);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 28.),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(summary).color(color(
                            if matches!(state, State::Failed | State::Error) {
                                self.tokens.error
                            } else {
                                self.tokens.text
                            },
                        )))
                        .truncate(),
                    );
                },
            );
            console = icon_button(ui, Icon::Terminal, "Console da execução", false, 24.).clicked();
            copy = icon_button(ui, Icon::Copy, "Copiar saída dos testes", false, 24.).clicked();
        });
        if matches!(self.tool_tab, ToolTab::JsTests | ToolTab::PythonTests)
            && !report.cases.is_empty()
        {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 3.), egui::Sense::hover());
            ui.painter().rect_filled(
                rect,
                1.,
                color(if failed + errors > 0 {
                    self.tokens.php_test_failed
                } else {
                    self.tokens.php_test_passed
                }),
            );
        }
        if console {
            self.php_tests_ui.selected = None;
        }
        let selected = self.php_tests_ui.selected.and_then(|i| report.cases.get(i));
        let source = selected.and_then(|c| c.path.as_ref().map(|p| (p.clone(), c.line)));
        let key = (
            *test_value!(self, revision),
            report.output.len(),
            self.php_tests_ui.selected,
        );
        if self
            .php_tests_ui
            .console
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            let mut text = if let Some(case) = selected {
                if case.details.is_empty() {
                    format!(
                        "{}\n{} · {}",
                        case.name,
                        case.outcome.label(),
                        millis(case.seconds)
                    )
                } else {
                    case.details.clone()
                }
            } else if let Some(error) = &report.error {
                format!("{error}\n\n{}", report.output)
            } else if report.output.is_empty() {
                "Execute a suite ou um arquivo de testes.\nOs resultados reais aparecerão na árvore à esquerda.".into()
            } else {
                report.output.clone()
            };
            if report.output_truncated && selected.is_none() {
                text.push_str("\nSaída limitada a1MiB.");
            }
            let mut lines: Vec<String> = text
                .lines()
                .take(20_000)
                .map(|line| line.chars().take(4096).collect())
                .collect();
            if text.lines().count() > 20_000 {
                lines.push("Visualização limitada a20000 linhas; Copiar saída conserva o texto disponível.".into());
            }
            self.php_tests_ui.console = Some(ConsoleCache { key, text, lines });
        }
        let cache = self.php_tests_ui.console.as_ref().unwrap();
        if copy {
            ui.ctx().copy_text(cache.text.clone());
        }
        let text_color = color(
            if selected.is_some_and(|c| c.outcome.is_failure()) || report.error.is_some() {
                self.tokens.error
            } else {
                self.tokens.text
            },
        );
        let max_height = (ui.available_height() - if source.is_some() { 30. } else { 0. }).max(20.);
        egui::ScrollArea::both()
            .id_salt(("php-tests.output", self.php_tests_ui.selected))
            .auto_shrink([false, false])
            .max_height(max_height)
            .codisseu()
            .show_rows(ui, 19., cache.lines.len(), |ui, range| {
                for index in range {
                    ui.add(
                        egui::Label::new(
                            RichText::new(&cache.lines[index])
                                .monospace()
                                .color(text_color),
                        )
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
            });
        if let Some((path, line)) = source {
            let label = format!(
                "{}{}",
                path.display(),
                line.map(|n| format!(":{n}")).unwrap_or_default()
            );
            if ui
                .link(label)
                .on_hover_text("Abrir fonte do teste")
                .clicked()
                && let Some(index) = self.php_tests_ui.selected
            {
                self.php_test_open_source(index);
            }
        }
    }
    pub(super) fn php_tests_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if !matches!(
            self.tool_tab,
            ToolTab::PhpTests | ToolTab::JsTests | ToolTab::PythonTests
        ) || !self.show_activity
        {
            return false;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::F5)) {
            self.php_test_command(TestCommand::RerunLast);
            return true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::F2)) {
            self.php_test_command(TestCommand::Cancel);
            return true;
        }
        false
    }
}

// Suite identity comes from reporter segments, never splitting a display name.
fn js_suite_rows(
    rows: &mut Vec<Row>,
    cases: &[codisseu_core::php_tests::TestCase],
    indices: &[usize],
    prefix: &[String],
    depth: usize,
    parent: &str,
    collapsed: &BTreeSet<String>,
) {
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for &index in indices {
        let c = &cases[index];
        if let Some(suite) = c.ancestors.get(prefix.len()) {
            groups.entry(suite.clone()).or_default().push(index);
        } else {
            rows.push(Row {
                label: if c.title.is_empty() {
                    c.name.clone()
                } else {
                    c.title.clone()
                },
                case: Some(index),
                key: None,
                depth,
                outcome: Some(c.outcome),
                seconds: c.seconds,
            });
        }
    }
    for (suite, indices) in groups {
        let mut path = prefix.to_vec();
        path.push(suite.clone());
        let key = format!(
            "{parent}:{}",
            serde_json::to_string(&path).unwrap_or_default()
        );
        rows.push(Row {
            label: suite,
            case: None,
            key: Some(key.clone()),
            depth,
            outcome: outcome(indices.iter().map(|i| cases[*i].outcome)),
            seconds: indices.iter().map(|i| cases[*i].seconds).sum(),
        });
        if !collapsed.contains(&key) {
            js_suite_rows(rows, cases, &indices, &path, depth + 1, parent, collapsed);
        }
    }
}
