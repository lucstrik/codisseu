//! User-selected editor bindings; commands share the existing core services.
use super::*;
#[derive(Default)]
pub(super) struct EditorShortcuts {
    line: Option<String>,
    focus: bool,
    diff_chord_pending: bool,
    diff_chord_started: f64,
    diff_chord_owner: Option<(DocumentId, PathBuf, Option<PathBuf>)>,
    right_was_chat: bool,
}
#[cfg(test)]
impl EditorShortcuts {
    pub(super) fn diff_pending(&self) -> bool {
        self.diff_chord_pending
    }
    pub(super) fn go_to_line_open(&self) -> bool {
        self.line.is_some()
    }
}
impl CodisseuApp {
    pub(super) fn requested_editor_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        if self.editor_shortcuts.line.is_some() {
            return true;
        }
        if self.path_dialog.is_some()
            || self.preferences_dialog
            || self.git_ui.diff_open
            || self.database_ui.dialog
        {
            return false;
        }
        if self.editor_focused(ctx)
            && !self.ai_chat.settings
            && !self.provider_dialog
            && !self.debugger_ui.dialog
            && self.git_ui.dialog.is_none()
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
            && consume_exact(ctx, Modifiers::ALT, Key::Z)
        {
            // X11 can deliver the printable event alongside the shortcut key.
            // Own that event too, so toggling wrap cannot edit the source.
            ctx.input_mut(|input| {
                input.events.retain(
                    |event| !matches!(event, egui::Event::Text(text) if text == "z" || text == "Z"),
                );
            });
            self.editor_claim_shortcut_text(ctx, Key::Z);
            self.send(Command::SetEditorSoftWrap(
                !self.service.editor_preferences().soft_wrap,
            ));
            self.focus_editor = true;
            return true;
        }
        let owner = self.service.active_document().map(|d| {
            (
                d.id,
                d.path.clone(),
                self.service.workspace_path().map(PathBuf::from),
            )
        });
        let comparison_open = self.editor_compare.owner_is_open();
        let eligible =
            (comparison_open || self.editor_focused(ctx) || self.project_tree_has_focus(ctx))
                && !self.ai_chat.settings
                && !self.provider_dialog
                && !self.debugger_ui.dialog
                && self.git_ui.dialog.is_none()
                && !egui::Popup::is_any_open(ctx)
                && (comparison_open || ctx.memory(|m| m.top_modal_layer().is_none()));
        if self.editor_shortcuts.diff_chord_pending {
            if !eligible
                || self.editor_shortcuts.diff_chord_owner != owner
                || ctx.input(|i| i.time) - self.editor_shortcuts.diff_chord_started > 5.
            {
                self.editor_shortcuts.diff_chord_pending = false;
            } else if consume_exact(ctx, Modifiers::NONE, Key::Comma) {
                self.editor_shortcuts.diff_chord_pending = false;
                ctx.input_mut(|i| {
                    i.events
                        .retain(|e| !matches!(e,egui::Event::Text(t) if t==","))
                });
                self.editor_claim_shortcut_text(ctx, Key::Comma);
                self.start_editor_comparison();
                return true;
            } else if consume_exact(ctx, Modifiers::NONE, Key::Period) {
                self.editor_shortcuts.diff_chord_pending = false;
                ctx.input_mut(|i| {
                    i.events
                        .retain(|e| !matches!(e,egui::Event::Text(t) if t=="."))
                });
                self.editor_claim_shortcut_text(ctx, Key::Period);
                self.next_editor_comparison();
                return true;
            } else if ctx.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Text(_) | egui::Event::Key { pressed: true, .. }
                    )
                })
            }) {
                self.editor_shortcuts.diff_chord_pending = false;
            }
        }
        if eligible
            && owner.is_some()
            && consume_exact(ctx, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G)
        {
            self.editor_shortcuts.diff_chord_pending = true;
            self.editor_shortcuts.diff_chord_started = ctx.input(|i| i.time);
            self.editor_shortcuts.diff_chord_owner = owner;
            self.editor_claim_shortcut_text(ctx, Key::G);
            ctx.input_mut(|i| {
                i.events
                    .retain(|e| !matches!(e,egui::Event::Text(t) if t=="g" || t=="G"))
            });
            return true;
        }
        if comparison_open {
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::N) {
            self.send(Command::NewUntitled);
            self.focus_editor = true;
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::B) {
            self.show_project = !self.show_project;
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND.plus(Modifiers::ALT), Key::B) {
            if self.ai_chat.open {
                self.ai_chat.open = false;
                self.editor_shortcuts.right_was_chat = true;
            } else if self.editor_shortcuts.right_was_chat {
                self.ai_chat.open = true;
                self.editor_shortcuts.right_was_chat = false;
            } else {
                self.show_database = !self.show_database;
            }
            return true;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::G)
            && self.service.active_document().is_some()
        {
            self.editor_shortcuts.line = Some(String::new());
            self.editor_shortcuts.focus = true;
            return true;
        }
        false
    }
    pub(super) fn save_document_or_choose_path(&mut self, id: DocumentId) {
        if self
            .service
            .documents
            .iter()
            .any(|d| d.id == id && d.is_untitled())
        {
            self.open_path(PathAction::SaveUntitled(id));
        } else {
            self.send(Command::Save { id });
        }
    }
    pub(super) fn go_to_line_input(&mut self, ctx: &egui::Context) {
        let Some(mut value) = self.editor_shortcuts.line.take() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
        egui::Modal::new(egui::Id::unique("codisseu.goto.line"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(20))
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width((ctx.content_rect().width() - 64.).clamp(180., 360.));
                ui.strong("Ir para linha");
                let label = ui.label(self.service.active_document().map_or_else(
                    || "Número da linha".to_owned(),
                    |doc| format!("Número da linha (1–{})", doc.line_count()),
                ));
                let response = ui
                    .add(
                        egui::TextEdit::singleline(&mut value)
                            .id(egui::Id::unique("codisseu.goto.line.input"))
                            .hint_text("Ex.: 67")
                            .align(egui::Align2::LEFT_CENTER)
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(49., self.tokens.control_height)),
                    )
                    .labelled_by(label.id);
                if self.editor_shortcuts.focus {
                    response.request_focus();
                    self.editor_shortcuts.focus = false;
                }
                accept = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    accept |= ui.add(self.tokens.dialog_button("Ir", true)).clicked();
                    cancel = ui
                        .add(self.tokens.dialog_button("Cancelar", false))
                        .clicked();
                });
            });
        cancel |= consume_exact(ctx, Modifiers::NONE, Key::Escape);
        if accept && let Some(doc) = self.service.active_document() {
            if let Ok(line) = value.trim().parse::<usize>()
                && let Some(index) = line.checked_sub(1).and_then(|line| doc.line_to_char(line))
            {
                self.send(Command::SetSelection {
                    id: doc.id,
                    expected_version: doc.version,
                    selection: codisseu_core::Selection::caret(index),
                });
                self.focus_editor = true;
                return;
            }
            self.service.last_error =
                Some(format!("Informe uma linha entre 1 e {}", doc.line_count()));
            self.editor_shortcuts.focus = true;
        }
        if cancel {
            self.focus_editor = true;
        } else {
            self.editor_shortcuts.line = Some(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{thread, time::Instant};
    fn settle(h: &mut Harness<CodisseuApp>) {
        let until = Instant::now() + Duration::from_secs(15);
        let mut idle_frames = 0;
        // Git insight workers are separate from AppService I/O. Synthetic
        // repaint steps must not outrun them or enter the next periodic refresh.
        while idle_frames < 8 {
            h.step();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
            if h.state().service.is_busy() || h.state().git_insights.is_busy() {
                idle_frames = 0;
            } else {
                idle_frames += 1;
            }
        }
    }
    fn capture(h: &mut Harness<CodisseuApp>, name: &str) {
        if let Some(out) = std::env::var_os("CODISSEU_SHORTCUT_CAPTURES") {
            std::fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(PathBuf::from(out).join(format!("{name}.png")))
                .unwrap();
        }
    }
    #[test]
    fn requested_bindings_navigate_real_lines_save_untitled_toggle_panels_and_show_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.php");
        let text = (1..=140)
            .map(|i| format!("// linha {i}: ação 😀\u{2028}fim"))
            .collect::<Vec<_>>()
            .join("\r\n");
        std::fs::write(&path, &text).unwrap();
        let mut s = AppService::new(dir.path().join("profile/session.json"));
        s.dispatch(Command::OpenFile(path.clone())).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while s.is_busy() {
            s.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(100)
            .with_pixels_per_point(
                std::env::var("CODISSEU_SHORTCUT_SCALE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1.),
            )
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(s),
            );
        h.run();
        let status_left = h.get_by_label("Arquivo aberto").rect().left();
        h.key_press_modifiers(Modifiers::COMMAND, Key::G);
        h.run();
        assert!(h.state().editor_shortcuts.line.is_some());
        capture(&mut h, "goto-line");
        h.event(egui::Event::Text("67".into()));
        h.key_press(Key::Enter);
        h.run();
        assert_eq!(
            h.state().service.active_document().unwrap().char_to_line(
                h.state()
                    .service
                    .active_document()
                    .unwrap()
                    .selection()
                    .head
            ),
            Some(66)
        );
        assert_eq!(h.state().editor_bridge.metrics.lines, 140);
        capture(&mut h, "line-67");
        let left = h.state().show_project;
        h.key_press_modifiers(Modifiers::COMMAND, Key::B);
        h.run();
        assert_ne!(h.state().show_project, left);
        h.key_press_modifiers(Modifiers::COMMAND, Key::B);
        h.run();
        assert_eq!(h.state().show_project, left);
        let right = h.state().show_database;
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::B);
        h.run();
        assert_ne!(h.state().show_database, right);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::B);
        h.run();
        assert_eq!(h.state().show_database, right);
        h.key_press_modifiers(Modifiers::COMMAND, Key::N);
        h.run();
        assert!(h.state().service.active_document().unwrap().is_untitled());
        h.event(egui::Event::Text("<?php // novo 😀".into()));
        h.run();
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        h.run();
        assert!(matches!(
            h.state().path_dialog,
            Some(PathAction::SaveUntitled(_))
        ));
        capture(&mut h, "save-untitled");
        h.get_by_label("Cancelar").click();
        h.run();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "<?php // novo 😀"
        );
        h.key_press_modifiers(Modifiers::COMMAND, Key::S);
        h.run();
        h.state_mut().path_input = dir.path().join("new.php").to_string_lossy().into();
        h.get_by_label("Salvar arquivo").click();
        settle(&mut h);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("new.php")).unwrap(),
            "<?php // novo 😀"
        );
        assert!(!h.state().service.active_document().unwrap().is_untitled());
        h.state_mut().service.last_error =
            Some("Não foi possível abrir a pasta — mensagem longa de teste".into());
        h.run();
        let matches =
            h.get_all_by_label("Não foi possível abrir a pasta — mensagem longa de teste");
        assert_eq!(matches.count(), 1);
        assert!(
            (h.get_by_label("Não foi possível abrir a pasta — mensagem longa de teste")
                .rect()
                .left()
                - status_left)
                .abs()
                <= 1.
        );
        capture(&mut h, "status-error");
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
    }
    #[test]
    fn git_chord_opens_real_before_after_and_terminal_shortcut_toggles_workspace_picker() {
        let dir = tempfile::tempdir().unwrap();
        let api = dir.path().join("api");
        let web = dir.path().join("web");
        std::fs::create_dir(&api).unwrap();
        std::fs::create_dir(&web).unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .args([
                    "-c",
                    "init.templateDir=",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(args)
                .current_dir(&api)
                .output()
                .unwrap();
            assert!(output.status.success(), "{:?}", output);
        };
        git(&["init", "-q"]);
        std::fs::write(api.join("file.php"), "<?php // before\n").unwrap();
        git(&["add", "file.php"]);
        git(&["commit", "-qm", "fixture"]);
        std::fs::write(api.join("file.php"), "<?php // after\n").unwrap();
        let workspace = dir.path().join("team.code-workspace");
        std::fs::write(
            &workspace,
            r#"{"folders":[{"name":"API","path":"api"},{"name":"Web","path":"web"}]}"#,
        )
        .unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        app.service
            .dispatch(Command::OpenWorkspace(workspace))
            .unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(100)
            .with_pixels_per_point(
                std::env::var("CODISSEU_SHORTCUT_SCALE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1.),
            )
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::SelectGitRepository(api.clone()))
            .unwrap();
        settle(&mut h);
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(api.join("file.php")))
            .unwrap();
        settle(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
        settle(&mut h);
        h.key_press(Key::Comma);
        h.event(egui::Event::Text(",".into()));
        settle(&mut h);
        let until = Instant::now() + Duration::from_secs(15);
        while h.state().editor_compare.data_for_test().is_none() {
            h.step();
            assert!(
                Instant::now() < until,
                "{:?}",
                h.state().editor_compare.error_for_test()
            );
            thread::sleep(Duration::from_millis(2));
        }
        settle(&mut h);
        assert!(h.state().editor_compare.owner_is_open());
        assert!(!h.state().git_ui.commit_open);
        assert!(!h.state().git_ui.diff_open);
        let patch = &h.state().editor_compare.data_for_test().unwrap().text;
        assert!(patch.contains("before") && patch.contains("after"));
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "<?php // after\n"
        );
        capture(&mut h, "git-before-after");
        h.get_by_label("Fechar comparação").click();
        settle(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Backtick);
        settle(&mut h);
        assert!(h.state().show_activity);
        assert!(h.state().tool_tab == ToolTab::Terminal);
        assert!(h.query_by_label("Terminal em API").is_some());
        capture(&mut h, "terminal-workspace-picker");
        h.get_by_label("Terminal em API").click();
        settle(&mut h);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Backtick);
        settle(&mut h);
        assert!(!h.state().show_activity);
        assert_eq!(
            std::fs::read_to_string(api.join("file.php")).unwrap(),
            "<?php // after\n"
        );
        h.state_mut().terminal.close();
        let until = Instant::now() + Duration::from_secs(10);
        while h.state().terminal.shutdown_pending() {
            h.step();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    #[ignore = "Explicit read-only check of a user-selected local file; CODISSEU_REPORTED_FILE required"]
    fn reported_file_matches_disk_line_for_line_and_go_to_line_67() {
        let file =
            PathBuf::from(std::env::var_os("CODISSEU_REPORTED_FILE").expect("file required"));
        let source = std::fs::read_to_string(&file).unwrap();
        assert!(!source.contains('\r'));
        let mut lines = source.split_inclusive('\n').collect::<Vec<_>>();
        if source.ends_with('\n') {
            lines.push("");
        }
        let profile = tempfile::tempdir().unwrap();
        let mut service = AppService::new(profile.path().join("session.json"));
        service.dispatch(Command::OpenFile(file.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(100)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.text(), source);
        assert_eq!(doc.line_count(), lines.len());
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(doc.snapshot().line(i).to_string(), *line, "line {}", i + 1);
        }
        eprintln!(
            "Read-only verified {} logical lines and every line's exact text",
            lines.len()
        );
        h.key_press_modifiers(Modifiers::COMMAND, Key::G);
        h.run();
        h.event(egui::Event::Text("67".into()));
        h.key_press(Key::Enter);
        h.run();
        let d = h.state().service.active_document().unwrap();
        assert_eq!(d.char_to_line(d.selection().head), Some(66));
        assert_eq!(h.state().editor_bridge.metrics.lines, lines.len());
        capture(&mut h, "reported-file-line-67");
        assert_eq!(std::fs::read_to_string(file).unwrap(), source);
    }
    #[test]
    #[ignore = "Explicit read-only legacy source and save-on-copy check; CODISSEU_LEGACY_FILE required"]
    fn legacy_file_editorconfig_lines_and_save_roundtrip_on_copy() {
        let file = PathBuf::from(std::env::var_os("CODISSEU_LEGACY_FILE").expect("file required"));
        let bytes = std::fs::read(&file).unwrap();
        assert!(std::str::from_utf8(&bytes).is_err());
        let expected: String = bytes.iter().map(|&b| char::from(b)).collect();
        let profile = tempfile::tempdir().unwrap();
        let mut service = AppService::new(profile.path().join("session.json"));
        service.dispatch(Command::OpenFile(file.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_max_steps(100)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        let d = h.state().service.active_document().unwrap();
        assert_eq!(d.encoding(), codisseu_core::FileEncoding::Latin1);
        assert_eq!(d.text(), expected);
        assert_eq!(d.line_count(), 64);
        assert_eq!(h.state().editor_bridge.metrics.lines, 64);
        capture(&mut h, "legacy-64-lines-latin1");
        let copy_root = profile.path().join("copy");
        std::fs::create_dir(&copy_root).unwrap();
        std::fs::write(
            copy_root.join(".editorconfig"),
            "root = true\n[*]\ncharset = latin1\n",
        )
        .unwrap();
        let copy = copy_root.join("copy.php");
        std::fs::write(&copy, &bytes).unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(copy.clone()))
            .unwrap();
        settle(&mut h);
        let d = h.state().service.active_document().unwrap();
        let id = d.id;
        let version = d.version;
        h.state_mut()
            .service
            .dispatch(Command::Save { id })
            .unwrap();
        settle(&mut h);
        assert_eq!(std::fs::read(&copy).unwrap(), bytes);
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: format!("{expected}\n// ação\n"),
            })
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::Save { id })
            .unwrap();
        settle(&mut h);
        let mut changed = bytes.clone();
        changed.extend(b"\n// a\xe7\xe3o\n");
        assert_eq!(std::fs::read(&copy).unwrap(), changed);
        let version = h.state().service.active_document().unwrap().version;
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: format!("{expected}€"),
            })
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::Save { id })
            .unwrap();
        settle(&mut h);
        assert!(h.state().service.last_error.is_some());
        assert_eq!(std::fs::read(&copy).unwrap(), changed);
        assert_eq!(std::fs::read(file).unwrap(), bytes);
        eprintln!(
            "Latin1 EditorConfig detected;64 lines;byte-exact no-op save and accent edit on copy;unrepresentable euro rejected;original unchanged"
        );
    }
}
