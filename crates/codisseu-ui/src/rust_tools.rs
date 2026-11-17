//! Explicit Cargo tasks reuse the native terminal and its process lifecycle.
use super::*;
use codisseu_core::rust_project::{self, CargoAction, CargoOptions};
use std::sync::mpsc::{self, Receiver};

#[derive(Default)]
pub(super) struct RustToolsUi {
    pub(super) open: bool,
    root: PathBuf,
    manifest: Option<PathBuf>,
    discovery: Option<Receiver<Result<(PathBuf, PathBuf), String>>>,
    planning: Option<Receiver<Result<rust_project::CargoPlan, String>>>,
    options: CargoOptions,
    program: String,
    arguments: String,
    trusted: bool,
    error: Option<String>,
}
impl CodisseuApp {
    pub(super) fn open_cargo(&mut self, action: CargoAction) {
        let Some(root) = self.service.language_project_root() else {
            self.service.last_error = Some("Abra um projeto Rust antes de usar Cargo".into());
            return;
        };
        let active = self.service.active_document().map(|d| d.path.clone());
        let (send, receive) = mpsc::sync_channel(1);
        let worker_root = root.clone();
        let spawn = std::thread::Builder::new()
            .name("codisseu-cargo-discovery".into())
            .spawn(move || {
                let _ = send.send(
                    rust_project::discover_manifest(&worker_root, active.as_deref())
                        .map(|manifest| (manifest, rust_project::cargo_program())),
                );
            });
        self.rust_tools = RustToolsUi {
            open: true,
            root,
            discovery: Some(receive),
            options: CargoOptions {
                action,
                ..Default::default()
            },

            error: spawn.err().map(|e| e.to_string()),
            ..Default::default()
        };
    }
    pub(super) fn cargo_dialog(&mut self, ctx: &egui::Context) {
        if !self.rust_tools.open {
            return;
        }
        if let Some(receiver) = &self.rust_tools.discovery {
            match receiver.try_recv() {
                Ok(result) => {
                    self.rust_tools.discovery = None;
                    match result {
                        Ok((manifest, program)) => {
                            self.rust_tools.manifest = Some(manifest);
                            self.rust_tools.program = program.display().to_string();
                        }
                        Err(error) => self.rust_tools.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(20))
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.rust_tools.discovery = None;
                    if self.rust_tools.error.is_none() {
                        self.rust_tools.error = Some("Descoberta Cargo interrompida".into());
                    }
                }
            }
        }
        let mut close = false;
        let mut launch = false;
        egui::Modal::new(egui::Id::unique("codisseu.cargo.tasks"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(16))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width((ctx.content_rect().width() - 80.).clamp(260., 700.));
                ui.horizontal(|ui| {
                    ui.strong("Rust · Cargo");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = icon_button(ui, Icon::Close, "Fechar Cargo", false, 26.).clicked();
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical().max_height((ctx.content_rect().height()-210.).max(120.)).codisseu().show(ui, |ui| {
                    if self.rust_tools.planning.is_some() { ui.disable(); }
                    if let Some(manifest) = &self.rust_tools.manifest {
                        ui.add(egui::Label::new(manifest.display().to_string()).wrap());
                    } else if self.rust_tools.discovery.is_some() { ui.label("Localizando Cargo.toml…"); }
                    ui.add_space(8.);
                    let draft = &mut self.rust_tools;
                    ui.horizontal_wrapped(|ui| {
                        for action in CargoAction::ALL { ui.selectable_value(&mut draft.options.action, action, action.label()); }
                    });
                    ui.add_space(8.);
                    for (label, value) in [("Executável Cargo", &mut draft.program), ("Pacote (opcional)", &mut draft.options.package), ("Binário / alvo (opcional)", &mut draft.options.target)] {
                        let l = ui.label(label);
                        ui.add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY)).labelled_by(l.id);
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.checkbox(&mut draft.options.release, "Release");
                        ui.checkbox(&mut draft.options.offline, "Offline");
                        ui.checkbox(&mut draft.options.locked, "Locked");
                        ui.checkbox(&mut draft.options.all_features, "All features");
                    });
                    if matches!(draft.options.action, CargoAction::Run | CargoAction::Test) {
                        let label = ui.label("Argumentos do programa/teste (um por linha)");
                        ui.add(egui::TextEdit::multiline(&mut draft.arguments).desired_rows(3).desired_width(f32::INFINITY)).labelled_by(label.id);
                    }
                    ui.add_space(8.);
                    ui.add(egui::Label::new("Cargo usa os arquivos salvos e a toolchain do projeto. Build scripts, proc macros, testes e programas podem executar código.").wrap());
                    if self.service.documents.iter().any(|d| d.is_dirty() && d.project_root() == draft.root) {
                        crate::status_messages::status_message(ui, "Salve os arquivos modificados antes de executar a tarefa.");
                    }
                    if let Some(error) = &draft.error { crate::status_messages::status_message(ui, error); }
                });
                ui.separator();
                ui.add_enabled(self.rust_tools.planning.is_none(), egui::Checkbox::new(&mut self.rust_tools.trusted, "Confio neste projeto e autorizo executar esta tarefa Cargo"));
                ui.horizontal(|ui| {
                    if ui.add(self.tokens.dialog_button("Cancelar", false)).clicked() { close = true; }
                    let dirty = self.service.documents.iter().any(|d| d.is_dirty() && d.project_root() == self.rust_tools.root);
                    if ui.add_enabled(self.rust_tools.trusted && self.rust_tools.manifest.is_some() && self.rust_tools.planning.is_none() && !dirty, self.tokens.dialog_button("Executar tarefa Cargo", true)).clicked() { launch = true; }
                });
            });
        close |= consume_exact(ctx, Modifiers::NONE, Key::Escape);
        if launch && self.rust_tools.planning.is_none() {
            let draft = &mut self.rust_tools;
            draft.options.arguments =
                if matches!(draft.options.action, CargoAction::Run | CargoAction::Test) {
                    draft.arguments.lines().map(str::to_owned).collect()
                } else {
                    vec![]
                };
            if let Some(manifest) = draft.manifest.clone() {
                let root = draft.root.clone();
                let program = draft.program.clone();
                let options = draft.options.clone();
                let (send, receive) = mpsc::sync_channel(1);
                match std::thread::Builder::new()
                    .name("codisseu-cargo-plan".into())
                    .spawn(move || {
                        let _ = send.send(rust_project::plan(
                            &root,
                            &manifest,
                            program.into(),
                            &options,
                        ));
                    }) {
                    Ok(_) => draft.planning = Some(receive),
                    Err(error) => draft.error = Some(error.to_string()),
                }
            }
        }
        let ready = self
            .rust_tools
            .planning
            .as_ref()
            .and_then(|r| match r.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Preparação Cargo interrompida".into()))
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(20));
                    None
                }
            });
        if let Some(result) = ready {
            self.rust_tools.planning = None;
            match result {
                Ok(plan)
                    if !close
                        && self.service.language_project_root().as_ref()
                            == Some(&self.rust_tools.root)
                        && self.rust_tools.trusted
                        && !self
                            .service
                            .documents
                            .iter()
                            .any(|d| d.is_dirty() && d.project_root() == self.rust_tools.root) =>
                {
                    let label = format!("Cargo {}", self.rust_tools.options.action.command());
                    let key = format!("cargo:{}:{:?}", plan.cwd.display(), plan.args);
                    if self
                        .terminal
                        .open_command(&plan.cwd, plan.program, plan.args, label, key)
                    {
                        self.show_activity = true;
                        self.tool_tab = ToolTab::Terminal;
                        close = true;
                    } else {
                        self.rust_tools.error = Some("Limite de sessões do terminal atingido; encerre uma sessão e tente novamente".into());
                    }
                }
                Ok(_) => {
                    self.rust_tools.error = Some(
                        "Projeto ou buffer mudou antes de iniciar Cargo; revise novamente".into(),
                    )
                }
                Err(error) => self.rust_tools.error = Some(error),
            }
        }
        if close {
            self.rust_tools.open = false;
            self.focus_editor = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::Selection;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::{fs, thread, time::Instant};
    fn wait(h: &mut Harness<'_, CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            h.step();
            if check(h.state()) {
                h.step();
                return;
            }
            assert!(
                Instant::now() < end,
                "{} {:?} cargo={:?}",
                h.state().service.status,
                h.state().service.last_error,
                h.state().rust_tools.error
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn setup(
        text: &str,
        scale: f32,
    ) -> (tempfile::TempDir, Harness<'static, CodisseuApp>, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projeto Rust ação");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname=\"codisseu_gui_rust\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
        )
        .unwrap();
        let path = root.join("src/main.rs");
        fs::write(&path, text).unwrap();
        let mut s = AppService::new(dir.path().join("profile/session.json"));
        s.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.is_busy() {
            s.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        s.dispatch(Command::OpenFile(path)).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_ui_state(|ui, a: &mut CodisseuApp| a.show(ui), CodisseuApp::new(s));
        wait(&mut h, |a| !a.service.is_busy());
        (dir, h, root)
    }
    fn capture(h: &mut Harness<'_, CodisseuApp>, name: &str) {
        if let Some(out) = std::env::var_os("CODISSEU_RUST_CAPTURES") {
            let out = PathBuf::from(out);
            fs::create_dir_all(&out).unwrap();
            h.run();
            h.render().unwrap().save(out.join(name)).unwrap();
        }
    }
    #[test]
    fn rust_templates_and_cargo_review_preserve_disk_and_require_explicit_launch() {
        for scale in [1., 1.25] {
            let (_dir, mut h, root) = setup("// ação 😀\r\nfn", scale);
            assert!(h.state().service.language.statuses.is_empty());
            let d = h.state().service.active_document().unwrap();
            assert_eq!(
                codisseu_core::Language::for_path(&d.path),
                Some(codisseu_core::Language::Rust)
            );
            let cmd = Command::SetSelection {
                id: d.id,
                expected_version: d.version,
                selection: Selection::caret(d.len_chars()),
            };
            h.state_mut().service.dispatch(cmd).unwrap();
            h.get_by_role(egui::accesskit::Role::MultilineTextInput)
                .focus();
            h.step();
            h.key_press(Key::Tab);
            h.step();
            h.step();
            assert!(
                h.state()
                    .service
                    .active_document()
                    .unwrap()
                    .text()
                    .contains("fn name(")
            );
            assert_eq!(
                fs::read_to_string(root.join("src/main.rs")).unwrap(),
                "// ação 😀\r\nfn"
            );
            capture(&mut h, &format!("snippet-{scale}.png"));
            h.key_press(Key::Escape);
            h.step();
            h.state_mut().open_cargo(CargoAction::Build);
            wait(&mut h, |a| a.rust_tools.manifest.is_some());
            h.run();
            h.get_by_label("Confio neste projeto e autorizo executar esta tarefa Cargo")
                .click();
            wait(&mut h, |a| a.rust_tools.trusted);
            assert!(
                h.get_by_label("Executar tarefa Cargo")
                    .accesskit_node()
                    .is_disabled()
            );
            let before = h
                .state()
                .service
                .active_document()
                .unwrap()
                .text()
                .to_owned();
            h.key_press_modifiers(Modifiers::COMMAND, Key::S);
            h.step();
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), before);
            assert_eq!(
                fs::read_to_string(root.join("src/main.rs")).unwrap(),
                "// ação 😀\r\nfn"
            );
            assert!(!h.state().terminal.has_sessions());
            capture(&mut h, &format!("cargo-dirty-{scale}.png"));
            h.key_press(Key::Escape);
            h.step();
            assert!(!h.state().rust_tools.open);
        }
    }
    #[test]
    #[ignore = "real Cargo GUI acceptance: uses the installed Rust toolchain"]
    fn real_cargo_run_from_review_passes_literal_arguments_and_debug_uses_rust_form() {
        let source = "// ação 😀\nfn main() { let a = std::env::args().nth(1).unwrap(); std::fs::write(\"cargo-result\", a).unwrap(); println!(\"Codisseu Rust\"); }\n";
        for scale in [1., 1.25] {
            let (_dir, mut h, root) = setup(source, scale);
            h.state_mut().open_cargo(CargoAction::Run);
            wait(&mut h, |a| a.rust_tools.manifest.is_some());
            assert!(
                h.get_by_label("Executar tarefa Cargo")
                    .accesskit_node()
                    .is_disabled()
            );
            assert!(!h.state().terminal.has_sessions());
            h.state_mut().rust_tools.options.offline = true;
            h.state_mut().rust_tools.arguments = "literal ; $(nothing) ' spaces".into();
            h.run();
            capture(&mut h, &format!("cargo-before-trust-{scale}.png"));
            h.get_by_label("Confio neste projeto e autorizo executar esta tarefa Cargo")
                .click();
            wait(&mut h, |a| a.rust_tools.trusted);
            capture(&mut h, &format!("cargo-review-{scale}.png"));
            h.get_by_label("Executar tarefa Cargo").click();
            wait(&mut h, |_| root.join("cargo-result").is_file());
            assert_eq!(
                fs::read_to_string(root.join("cargo-result")).unwrap(),
                "literal ; $(nothing) ' spaces"
            );
            assert!(!h.state().rust_tools.open);
            assert!(h.state().tool_tab == ToolTab::Terminal);
            assert_eq!(h.state().terminal.session_label(), "Cargo run");
            assert_eq!(
                fs::read_to_string(root.join("src/main.rs")).unwrap(),
                source
            );
            h.state_mut().debugger_open_configurations();
            h.run();
            capture(&mut h, &format!("rust-debug-config-{scale}.png"));
            assert_eq!(
                h.state().debugger_ui.draft.as_ref().unwrap().runtime,
                codisseu_core::debugger::DebugRuntime::Rust
            );
            assert!(h.query_by_label("Compiled Rust executable:").is_some());
            assert!(h.query_by_label("Node interpreter:").is_none());
            assert!(
                h.get_by_label("Start Debug Configuration")
                    .accesskit_node()
                    .is_disabled()
            );
            capture(&mut h, &format!("rust-debug-config-{scale}.png"));
            h.key_press(Key::Escape);
            h.step();
            h.state_mut().terminal.close();
            wait(&mut h, |a| !a.terminal.shutdown_pending());
            assert!(h.state().service.debugger.sessions.is_empty());
        }
    }
    #[test]
    #[ignore = "real Rust GUI acceptance: requires rust-analyzer, rust-src, rustfmt"]
    fn real_rust_format_shortcut_versions_dirty_buffer_undo_save_and_provider_shutdown() {
        let source = "// ação 😀\r\nfn main(){let number=21+21;println!(\"{}\",number);}\r\n";
        for scale in [1., 1.25] {
            let (_dir, mut h, root) = setup(source, scale);
            h.state_mut()
                .service
                .dispatch(Command::ConfigureProviders(vec![
                    codisseu_core::ProviderConfig {
                        language: codisseu_core::Language::Rust,
                        program: std::env::var_os("CODISSEU_RUST_ANALYZER")
                            .unwrap_or_else(|| "rust-analyzer".into())
                            .into(),
                        args: vec![],
                    },
                ]))
                .unwrap();
            h.state_mut()
                .service
                .dispatch(Command::StartProviders)
                .unwrap();
            wait(&mut h, |a| {
                !a.service.is_busy()
                    && a.service
                        .language
                        .statuses
                        .iter()
                        .any(|s| s.state == codisseu_core::ProviderState::Ready)
            });
            let d = h.state().service.active_document().unwrap();
            let cmd = Command::Edit {
                id: d.id,
                expected_version: d.version,
                text: source.replace("21+21", "40+2"),
            };
            h.state_mut().service.dispatch(cmd).unwrap();
            wait(&mut h, |a| !a.service.is_busy());
            let before = h
                .state()
                .service
                .active_document()
                .unwrap()
                .text()
                .to_owned();
            h.get_by_role(egui::accesskit::Role::MultilineTextInput)
                .focus();
            h.step();
            h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
            wait(&mut h, |a| !a.service.is_busy());
            assert!(
                h.state().service.last_error.is_none(),
                "{:?}",
                h.state().service.last_error
            );
            let formatted = h
                .state()
                .service
                .active_document()
                .unwrap()
                .text()
                .to_owned();
            assert!(formatted.contains("40 + 2"), "{formatted}");
            assert!(formatted.contains("ação 😀"));
            assert!(!formatted.replace("\r\n", "").contains('\n'));
            assert_eq!(
                fs::read_to_string(root.join("src/main.rs")).unwrap(),
                source
            );
            capture(&mut h, &format!("rust-formatted-{scale}.png"));
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.step();
            h.step();
            assert_eq!(h.state().service.active_document().unwrap().text(), before);
            h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
            h.step();
            h.step();
            assert_eq!(
                h.state().service.active_document().unwrap().text(),
                formatted
            );
            h.key_press_modifiers(Modifiers::COMMAND, Key::S);
            wait(&mut h, |a| !a.service.is_busy());
            assert_eq!(
                fs::read_to_string(root.join("src/main.rs")).unwrap(),
                formatted
            );
            h.state_mut()
                .service
                .dispatch(Command::StopProviders)
                .unwrap();
            wait(&mut h, |a| !a.service.is_busy());
            assert!(!h.state().service.providers_running());
        }
    }
}
