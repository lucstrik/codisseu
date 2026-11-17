//! Read-only previous-version navigation for a captured editor and file history.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
#[derive(Clone, PartialEq, Eq)]
struct Owner {
    id: DocumentId,
    version: u64,
    path: PathBuf,
    workspace: Option<PathBuf>,
}
struct Loaded {
    history: Arc<codisseu_core::git::previous::FileHistory>,
    data: Arc<git_diff::DiffData>,
    equal: bool,
    before: String,
    after: String,
    has_previous: bool,
}
#[derive(Default)]
pub(super) struct CompareUi {
    owner: Option<Owner>,
    cancel: Option<Arc<AtomicBool>>,
    receiver: Option<mpsc::Receiver<Result<Loaded, String>>>,
    data: Option<Arc<git_diff::DiffData>>,
    error: Option<String>,
    equal: bool,
    history: Option<Arc<codisseu_core::git::previous::FileHistory>>,
    step: usize,
    working: bool,
    before: String,
    after: String,
    has_previous: bool,
    view: git_diff_view::DiffView,
    unified: bool,
    #[cfg(test)]
    geometry: [Option<egui::Rect>; 5],
}
impl Drop for CompareUi {
    fn drop(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, Ordering::SeqCst);
        }
    }
}
impl CompareUi {
    #[cfg(test)]
    pub(super) fn data_for_test(&self) -> Option<&Arc<git_diff::DiffData>> {
        self.data.as_ref()
    }
    #[cfg(test)]
    pub(super) fn error_for_test(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub(super) fn owner_is_open(&self) -> bool {
        self.owner.is_some()
    }
}
fn comparison_label(
    ui: &mut egui::Ui,
    text: impl Into<egui::WidgetText>,
    size: egui::Vec2,
) -> egui::Response {
    ui.allocate_ui_with_layout(
        size,
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(size);
            ui.add(
                egui::Label::new(text)
                    .truncate()
                    .show_tooltip_when_elided(false),
            )
        },
    )
    .inner
}
impl CodisseuApp {
    pub(super) fn start_editor_comparison(&mut self) {
        self.navigate_editor_comparison(false);
    }
    pub(super) fn next_editor_comparison(&mut self) {
        self.navigate_editor_comparison(true);
    }
    fn navigate_editor_comparison(&mut self, next: bool) {
        let Some(doc) = self.service.active_document().filter(|d| !d.is_untitled()) else {
            self.service.status = "Salve o arquivo antes de consultar seu histórico Git".into();
            return;
        };
        let owner = Owner {
            id: doc.id,
            version: doc.version,
            path: doc.path.clone(),
            workspace: self.service.workspace_path().map(PathBuf::from),
        };
        let snapshot = doc.snapshot();
        let encoding = doc.encoding();
        if self.editor_compare.owner.as_ref() == Some(&owner)
            && self.editor_compare.receiver.is_some()
            && self.editor_compare.history.is_none()
        {
            return;
        }
        let previous = if self.editor_compare.owner.as_ref() == Some(&owner) {
            self.editor_compare.history.clone()
        } else {
            None
        };
        if next && previous.is_none() {
            return;
        }
        let step = if let Some(h) = &previous {
            if next {
                self.editor_compare.step.saturating_sub(1)
            } else {
                self.editor_compare
                    .step
                    .saturating_add(1)
                    .min(h.last_step())
            }
        } else {
            0
        };
        let working = next && step == 0;
        if previous.is_some()
            && step == self.editor_compare.step
            && (!next || self.editor_compare.working)
        {
            return;
        }
        if previous.is_none() {
            self.editor_compare = CompareUi::default();
        }
        if let Some(c) = self.editor_compare.cancel.take() {
            c.store(true, Ordering::SeqCst);
        }
        self.editor_compare.step = step;
        self.editor_compare.working = working;
        self.editor_compare.owner = Some(owner.clone());
        self.editor_compare.error = None;
        let cancel = Arc::new(AtomicBool::new(false));
        self.editor_compare.cancel = Some(cancel.clone());
        let (sender, receiver) = mpsc::sync_channel(1);
        self.editor_compare.receiver = Some(receiver);
        if let Err(e) = std::thread::Builder::new()
            .name("editor-comparison".into())
            .spawn(move || {
                let result = (|| {
                    if snapshot.len_bytes() > codisseu_core::git::line_changes::MAX_BYTES {
                        return Err("Comparação limitada a4MiB por versão".into());
                    }
                    let history = match previous {
                        Some(h) => h,
                        None => Arc::new(
                            codisseu_core::git::previous::FileHistory::capture(
                                &owner.path,
                                encoding.encode(&snapshot.to_string())?,
                                &cancel,
                            )
                            .map_err(|e| e.to_string())?,
                        ),
                    };
                    let pair = if working {
                        history.working_pair(&cancel)
                    } else {
                        history.pair(step, &cancel)
                    }
                    .map_err(|e| e.to_string())?;
                    let base = encoding.decode(&pair.before)?.0;
                    let after = encoding.decode(&pair.after)?.0;
                    let (patch, equal) =
                        codisseu_core::git::line_changes::comparison_patch(&base, &after, &cancel)
                            .map_err(|e| e.to_string())?;
                    Ok(Loaded {
                        history,
                        data: Arc::new(git_diff::DiffData::comparison(patch, &owner.path)),
                        equal,
                        before: pair.before_label,
                        after: pair.after_label,
                        has_previous: pair.has_previous,
                    })
                })();
                let _ = sender.try_send(result);
            })
        {
            self.editor_compare.receiver = None;
            self.editor_compare.error = Some(e.to_string());
        }
    }
    pub(super) fn editor_comparison_window(&mut self, ctx: &egui::Context) {
        let Some(owner) = self.editor_compare.owner.clone() else {
            return;
        };
        let current = self.service.active_document().is_some_and(|d| {
            d.id == owner.id && d.version == owner.version && d.path == owner.path
        }) && self.service.workspace_path() == owner.workspace.as_deref();
        if !current {
            if let Some(c) = &self.editor_compare.cancel {
                c.store(true, Ordering::SeqCst);
            }
            self.editor_compare.receiver = None;
            self.editor_compare.data = None;
            self.editor_compare.error =
                Some("O arquivo ou o contexto mudou. Feche e reabra a comparação.".into());
        }
        if let Some(receiver) = &self.editor_compare.receiver {
            match receiver.try_recv() {
                Ok(result) => {
                    self.editor_compare.receiver = None;
                    match result {
                        Ok(result) => {
                            self.editor_compare.equal = result.equal;
                            self.editor_compare.data = Some(result.data);
                            self.editor_compare.history = Some(result.history);
                            self.editor_compare.before = result.before;
                            self.editor_compare.after = result.after;
                            self.editor_compare.has_previous = result.has_previous;
                        }
                        Err(e) => self.editor_compare.error = Some(e),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.editor_compare.receiver = None;
                    self.editor_compare.error = Some("A comparação foi interrompida".into());
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(25));
                }
            }
        }
        let mut close = false;
        let mut previous = false;
        let mut next = false;
        let margin = self.tokens.rename_dialog_margin;
        let code_inset = self.tokens.navigation_popup_padding;
        let bounds = ctx
            .content_rect()
            .shrink(f32::from(self.tokens.dialog_content_inset + code_inset));
        egui::Modal::new(egui::Id::unique("editor.comparison"))
            .frame(
                egui::Frame::popup(&ctx.global_style())
                    .fill(color(self.tokens.panel))
                    .inner_margin(margin),
            )
            .show(ctx, |ui| {
                self.tokens.form_style(ui);
                ui.set_width((bounds.width() - 2. * f32::from(margin)).max(160.));
                let top = ui.cursor().top();
                let _header = ui.horizontal(|ui| {
                    ui.set_min_height(self.tokens.database_toolbar_height);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = components::database_icon_button(
                            ui,
                            Icon::Close,
                            "Fechar comparação",
                            true,
                        )
                        .clicked();
                        let filename = owner
                            .path
                            .file_name()
                            .unwrap_or(owner.path.as_os_str())
                            .to_string_lossy();
                        let response = comparison_label(
                            ui,
                            egui::RichText::new(filename).strong(),
                            egui::vec2(ui.available_width(), self.tokens.database_toolbar_height),
                        );
                        #[cfg(test)]
                        {
                            self.editor_compare.geometry[0] = Some(response.rect);
                        }
                        response.on_hover_text(owner.path.display().to_string());
                    });
                });
                #[cfg(test)]
                {
                    self.editor_compare.geometry[1] = Some(_header.response.rect);
                }
                ui.separator();
                let loading = self.editor_compare.receiver.is_some();
                let _toolbar = ui.horizontal_wrapped(|ui| {
                    previous = components::database_icon_button(
                        ui,
                        Icon::Previous,
                        "Versão anterior",
                        self.editor_compare.history.is_some() && self.editor_compare.has_previous,
                    )
                    .on_hover_text("Versão anterior · Ctrl+Shift+G, vírgula")
                    .clicked();
                    next = components::database_icon_button(
                        ui,
                        Icon::Next,
                        "Próxima versão",
                        self.editor_compare.history.is_some()
                            && (self.editor_compare.step > 0 || !self.editor_compare.working),
                    )
                    .on_hover_text("Próxima versão · Ctrl+Shift+G, ponto")
                    .clicked();
                    ui.separator();
                    egui::ComboBox::from_id_salt("editor.comparison.layout")
                        .selected_text(if self.editor_compare.unified {
                            "Unificado"
                        } else {
                            "Lado a lado"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.editor_compare.unified,
                                false,
                                "Lado a lado",
                            );
                            ui.selectable_value(
                                &mut self.editor_compare.unified,
                                true,
                                "Unificado",
                            );
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::ComboBox,
                                true,
                                "Visualização da comparação",
                            )
                        });
                    if loading {
                        ui.spinner();
                        ui.label("Carregando comparação…");
                    } else if self.editor_compare.history.is_some() {
                        let boundary = if !self.editor_compare.has_previous {
                            Some("Início do histórico ou limite de200 revisões")
                        } else if self.editor_compare.working {
                            Some("Versão mais recente")
                        } else {
                            None
                        };
                        if let Some(label) = boundary {
                            ui.label(
                                egui::RichText::new(label)
                                    .small()
                                    .color(color(self.tokens.muted)),
                            );
                        }
                    }
                });
                #[cfg(test)]
                {
                    self.editor_compare.geometry[2] = Some(_toolbar.response.rect);
                }
                if let Some(error) = &self.editor_compare.error {
                    ui.label(error);
                }
                if let Some(data) = &self.editor_compare.data {
                    if self.editor_compare.equal {
                        ui.label(if data.text.is_empty() {
                            "Arquivo vazio nas duas versões"
                        } else {
                            "Sem diferenças entre estas versões"
                        });
                    }
                    egui::Frame::NONE
                        .fill(color(self.tokens.editor))
                        .stroke(egui::Stroke::new(1., color(self.tokens.border)))
                        .inner_margin(code_inset)
                        .show(ui, |ui| {
                            let width = ui.available_width();
                            let h = self.tokens.database_toolbar_height;
                            let _labels = ui
                                .scope(|ui| {
                                    ui.spacing_mut().item_spacing.x = 1.;
                                    ui.horizontal(|ui| {
                                        if self.editor_compare.unified {
                                            comparison_label(
                                                ui,
                                                format!(
                                                    "{} → {}",
                                                    self.editor_compare.before,
                                                    self.editor_compare.after
                                                ),
                                                egui::vec2(width, h),
                                            )
                                            .on_hover_text(format!(
                                                "{} → {}",
                                                self.editor_compare.before,
                                                self.editor_compare.after
                                            ));
                                        } else {
                                            let pane = (width - 1.) / 2.;
                                            comparison_label(
                                                ui,
                                                &self.editor_compare.before,
                                                egui::vec2(pane, h),
                                            )
                                            .on_hover_text(&self.editor_compare.before);
                                            comparison_label(
                                                ui,
                                                &self.editor_compare.after,
                                                egui::vec2(pane, h),
                                            )
                                            .on_hover_text(&self.editor_compare.after);
                                        }
                                    })
                                    .response
                                    .rect
                                })
                                .inner;
                            #[cfg(test)]
                            {
                                self.editor_compare.geometry[3] = Some(_labels);
                            }
                            ui.separator();
                            let height = (bounds.height()
                                - (ui.cursor().top() - top)
                                - f32::from(2 * margin + code_inset))
                            .max(48.);
                            #[cfg(test)]
                            {
                                self.editor_compare.geometry[4] = Some(egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(width, height),
                                ));
                            }
                            self.editor_compare.view.show(
                                ui,
                                data.clone(),
                                &self.tokens,
                                self.editor_compare.unified,
                                true,
                                height,
                            );
                        });
                } else {
                    ui.add_space(
                        (bounds.height()
                            - (ui.cursor().top() - top)
                            - f32::from(2 * margin + code_inset))
                        .max(48.),
                    );
                }
            });
        if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if previous {
            self.start_editor_comparison();
        }
        if next {
            self.next_editor_comparison();
        }
        if close {
            self.editor_compare = CompareUi::default();
            self.focus_editor = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    use std::{
        fs,
        path::Path,
        thread,
        time::{Duration, Instant},
    };
    type H = Harness<'static, CodisseuApp>;
    fn git(root: &Path, args: &[&str]) -> Vec<u8> {
        let o = std::process::Command::new("git")
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
            .current_dir(root)
            .output()
            .unwrap();
        assert!(o.status.success(), "{o:?}");
        o.stdout
    }
    fn settle(h: &mut H) {
        let until = Instant::now() + Duration::from_secs(15);
        for _ in 0..5 {
            h.step();
        }
        while h.state().service.is_busy() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < until);
        }
        h.step();
    }
    fn fixture(text: &str, repo: bool) -> (tempfile::TempDir, PathBuf, H) {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("project");
        fs::create_dir(&root).unwrap();
        let path = root.join("sample.php");
        fs::write(&path, text).unwrap();
        if repo {
            git(&root, &["init", "-q"]);
            git(&root, &["add", "sample.php"]);
            git(&root, &["commit", "-qm", "fixture"]);
        }
        let mut service = AppService::new(d.path().join("session.json"));
        service
            .dispatch(Command::SetAutoSave {
                enabled: false,
                delay_ms: 1000,
            })
            .unwrap();
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        while service.is_busy() {
            service.poll();
            thread::sleep(Duration::from_millis(2));
        }
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        let mut h = Harness::builder()
            .with_step_dt(1. / 60.)
            .with_size([1280., 800.])
            .with_pixels_per_point(
                std::env::var("CODISSEU_SHORTCUT_SCALE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1.),
            )
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h);
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        (d, path, h)
    }
    fn prefix(h: &mut H) {
        h.key_press_modifiers(
            Modifiers::CTRL
                .plus(Modifiers::COMMAND)
                .plus(Modifiers::SHIFT),
            Key::G,
        );
        h.step();
        h.step();
        assert!(h.state().editor_shortcuts.diff_pending());
        assert!(!h.state().git_ui.commit_open);
        assert!(!h.state().git_ui.source_control);
        assert!(!h.state().editor_compare.owner_is_open());
    }
    fn complete(h: &mut H) {
        complete_key(h, Key::Comma, ",");
    }
    fn complete_key(h: &mut H, key: Key, text: &str) {
        h.key_press(key);
        h.step();
        h.event(egui::Event::Text(text.into()));
        h.step();
        let end = Instant::now() + Duration::from_secs(15);
        while h.state().editor_compare.receiver.is_some() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < end);
        }
        h.step();
    }
    fn capture(h: &mut H, name: &str) {
        if let Some(out) = std::env::var_os("CODISSEU_COMPARE_CAPTURES") {
            let out = PathBuf::from(out);
            fs::create_dir_all(&out).unwrap();
            let scale = (h.ctx.pixels_per_point() * 100.) as u32;
            h.render()
                .unwrap()
                .save(out.join(format!("{name}-{}.png", scale)))
                .unwrap();
        }
    }
    #[test]
    fn chord_is_quiet_and_compares_unsaved_editor_without_touching_index_or_disk() {
        let (_d, path, mut h) = fixture("first\nbefore ação 🦀\nlast\n", true);
        let root = path.parent().unwrap();
        let index = git(root, &["show", ":sample.php"]);
        let disk = fs::read(&path).unwrap();
        h.event(egui::Event::Text("UNSAVED ".into()));
        h.step();
        let doc = h.state().service.active_document().unwrap();
        let source = doc.text().to_owned();
        let sel = doc.selection();
        let version = doc.version;
        prefix(&mut h);
        capture(&mut h, "prefix");
        complete(&mut h);
        let c = &h.state().editor_compare;
        assert!(c.error.is_none(), "{:?}", c.error);
        assert!(!c.equal);
        let text = &c.data.as_ref().unwrap().text;
        assert!(
            text.contains("UNSAVED")
                && text.contains("before ação 🦀")
                && text.contains("first")
                && text.contains("last")
        );
        assert!(!h.state().git_ui.commit_open);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.text(), source);
        assert_eq!(doc.selection(), sel);
        assert_eq!(doc.version, version);
        assert!(doc.is_dirty());
        assert_eq!(fs::read(&path).unwrap(), disk);
        assert_eq!(git(root, &["show", ":sample.php"]), index);
        capture(&mut h, "unsaved");
        h.get_by_label("Fechar comparação").click();
        h.step();
        settle(&mut h);
        assert!(!h.state().editor_compare.owner_is_open());
    }
    #[test]
    fn unchanged_and_empty_files_have_explicit_state_and_complete_content() {
        for source in ["first\nação\nlast\n", ""] {
            let (_d, _path, mut h) = fixture(source, true);
            prefix(&mut h);
            complete(&mut h);
            let c = &h.state().editor_compare;
            assert!(c.error.is_none(), "{:?}", c.error);
            assert_eq!(c.equal, source.is_empty());
            let text = &c.data.as_ref().unwrap().text;
            if source.is_empty() {
                h.get_by_label("Arquivo vazio nas duas versões");
            } else {
                assert!(text.contains("first") && text.contains("last"));
                h.get_by_label("Antes da criação do arquivo");
            }
            capture(
                &mut h,
                if source.is_empty() {
                    "empty"
                } else {
                    "unchanged"
                },
            );
        }
    }
    #[test]
    fn new_file_and_missing_repository_are_not_blank_windows() {
        for repo in [true, false] {
            let (_d, path, mut h) = fixture("new content\n", repo);
            if repo {
                git(path.parent().unwrap(), &["rm", "--cached", "sample.php"]);
            }
            prefix(&mut h);
            complete(&mut h);
            if repo {
                assert!(
                    h.state()
                        .editor_compare
                        .data
                        .as_ref()
                        .unwrap()
                        .text
                        .contains("+new content")
                );
            } else {
                assert!(
                    h.state()
                        .editor_compare
                        .error
                        .as_ref()
                        .is_some_and(|e| !e.is_empty())
                );
                assert!(h.state().editor_compare.data.is_none());
            }
            capture(&mut h, if repo { "new" } else { "no-repository" });
        }
    }
    #[test]
    fn repeated_chord_and_button_step_through_real_file_history() {
        let (_d, path, mut h) = fixture("version1\n", true);
        let root = path.parent().unwrap();
        for n in [2, 3] {
            fs::write(&path, format!("version{n}\n")).unwrap();
            git(root, &["add", "."]);
            git(root, &["commit", "-qm", "next"]);
        }
        fs::write(root.join("unrelated"), "noise").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "unrelated"]);
        let id = h.state().service.active_document().unwrap().id;
        h.state_mut()
            .service
            .dispatch(Command::Close { id, discard: true })
            .unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        settle(&mut h);
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        prefix(&mut h);
        complete(&mut h);
        for (step, before, after) in [
            (0, "version2", "version3"),
            (1, "version2", "version3"),
            (2, "version1", "version2"),
            (3, "", "version1"),
        ] {
            let c = &h.state().editor_compare;
            assert!(c.error.is_none(), "{:?}", c.error);
            assert_eq!(c.step, step);
            let t = &c.data.as_ref().unwrap().text;
            assert!(t.contains(after));
            assert!(before.is_empty() || t.contains(before));
            capture(&mut h, &format!("history-{step}"));
            if step < 3 {
                if step == 1 {
                    h.get_by_label("Versão anterior").click();
                    h.step();
                } else {
                    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
                    h.step();
                    assert!(h.state().editor_shortcuts.diff_pending());
                    complete(&mut h);
                }
                let end = Instant::now() + Duration::from_secs(15);
                while h.state().editor_compare.receiver.is_some() {
                    h.step();
                    thread::sleep(Duration::from_millis(2));
                    assert!(Instant::now() < end);
                }
                h.step();
            }
        }
        assert!(!h.state().editor_compare.has_previous);
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
        h.step();
        complete(&mut h);
        assert!(h.state().editor_compare.data.is_some());
        assert!(h.state().editor_compare.error.is_none());
        assert_eq!(fs::read_to_string(path).unwrap(), "version3\n");
        for step in [2, 1, 0] {
            h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
            h.step();
            complete_key(&mut h, Key::Period, ".");
            assert_eq!(h.state().editor_compare.step, step);
            assert!(h.state().editor_compare.error.is_none());
            capture(&mut h, &format!("forward-{step}"));
        }
        assert!(h.state().editor_compare.working && h.state().editor_compare.equal);
        assert!(
            h.state()
                .editor_compare
                .data
                .as_ref()
                .unwrap()
                .text
                .contains(" version3")
        );
        h.get_by_label("Versão mais recente");
        assert!(
            h.get_by_label("Próxima versão")
                .accesskit_node()
                .is_disabled()
        );
        let data = h.state().editor_compare.data.clone().unwrap();
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::G);
        h.step();
        complete_key(&mut h, Key::Period, ".");
        assert!(Arc::ptr_eq(
            &data,
            h.state().editor_compare.data.as_ref().unwrap()
        ));
        h.get_by_label("Versão anterior").click();
        h.step();
        let end = Instant::now() + Duration::from_secs(15);
        while h.state().editor_compare.receiver.is_some() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < end);
        }
        h.step();
        assert_eq!(h.state().editor_compare.step, 1);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "version3\n"
        );
        h.key_press(Key::Escape);
        h.step();
        assert!(!h.state().editor_compare.owner_is_open());
    }
    #[test]
    fn next_outside_comparison_is_quiet_and_staged_dirty_roundtrip_pins_capture() {
        let (_d, path, mut h) = fixture("HEAD\n", true);
        let source = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        prefix(&mut h);
        complete_key(&mut h, Key::Period, ".");
        assert!(!h.state().editor_compare.owner_is_open());
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        let root = path.parent().unwrap();
        // Stage the fixture before reopening it, so filesystem notifications do
        // not introduce an unrelated external-change dialog into this scenario.
        let id = h.state().service.active_document().unwrap().id;
        h.state_mut()
            .service
            .dispatch(Command::Close { id, discard: true })
            .unwrap();
        fs::write(&path, "STAGED\n").unwrap();
        git(root, &["add", "."]);
        fs::write(&path, "DISK\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        settle(&mut h);
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        h.event(egui::Event::Text("UNSAVED ".into()));
        h.step();
        let source = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        let version = h.state().service.active_document().unwrap().version;
        let selection = h.state().service.active_document().unwrap().selection();
        prefix(&mut h);
        complete(&mut h);
        assert!(
            h.state()
                .editor_compare
                .data
                .as_ref()
                .unwrap()
                .text
                .contains("-STAGED")
        );
        h.get_by_label("Versão anterior").click();
        h.step();
        // A new request supersedes the pending backward worker; old results cannot win.
        h.state_mut().next_editor_comparison();
        let end = Instant::now() + Duration::from_secs(15);
        while h.state().editor_compare.receiver.is_some() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < end);
        }
        h.step();
        assert_eq!(h.state().editor_compare.step, 0);
        let text = &h.state().editor_compare.data.as_ref().unwrap().text;
        assert!(text.contains("-STAGED") && text.contains("UNSAVED"));
        assert_eq!(fs::read(&path).unwrap(), b"DISK\n");
        assert_eq!(git(root, &["show", ":sample.php"]), b"STAGED\n");
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.text(), source);
        assert_eq!(doc.version, version);
        assert_eq!(doc.selection(), selection);
    }
    #[test]
    fn compact_header_hover_and_layout_fit_long_path_and_small_window() {
        let (_d, path, mut h) = fixture("<?php\necho 'sample';\n", true);
        let folder = path
            .parent()
            .unwrap()
            .join("deep folder")
            .join("another folder");
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("{}.php", "LongFileName".repeat(12)));
        fs::write(&path, "<?php\necho 'new';\n").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        settle(&mut h);
        h.state_mut().start_editor_comparison();
        let end = Instant::now() + Duration::from_secs(15);
        while h.state().editor_compare.receiver.is_some() {
            h.step();
            thread::sleep(Duration::from_millis(2));
            assert!(Instant::now() < end);
        }
        h.step();
        for size in [[1280., 800.], [640., 480.]] {
            h.set_size(egui::vec2(size[0], size[1]));
            settle(&mut h);
            let geo = h.state().editor_compare.geometry;
            let [name, header, toolbar, labels, viewport] = geo.map(|r| r.expect("geometry"));
            assert!(
                (name.left() - header.left()).abs() < 1. && name.right() <= header.right() + 1.
            );
            assert!(header.bottom() <= toolbar.top() + 1.);
            assert!((labels.left() - viewport.left()).abs() < 1.);
            assert!(
                (labels.width() - viewport.width()).abs() < 1.,
                "{geo:?} {size:?}"
            );
            assert!(viewport.bottom() <= size[1] - 12., "{geo:?} {size:?}");
            assert!(
                !h.get_all_by_label(&path.display().to_string())
                    .any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
            );
            capture(
                &mut h,
                if size[0] < 1000. {
                    "compact-long-name"
                } else {
                    "long-name"
                },
            );
        }
        let filename = h.state().editor_compare.geometry[0].unwrap();
        h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.);
        h.hover_at(filename.center());
        h.step();
        h.input_mut().time = Some(h.ctx.input(|i| i.time) + 1.);
        h.step();
        h.step();
        assert!(
            h.get_all_by_label(&path.display().to_string())
                .any(|n| n.accesskit_node().role() == egui::accesskit::Role::Label)
        );
        capture(&mut h, "path-tooltip");
        let mode = h.get_by_label("Visualização da comparação").rect();
        h.hover_at(mode.center());
        h.step();
        h.get_by_label("Visualização da comparação").click();
        h.step();
        h.step();
        h.get_by_label("Unificado").click();
        h.step();
        assert!(h.state().editor_compare.unified);
        capture(&mut h, "compact-unified");
        h.get_by_label("Fechar comparação").click();
        h.step();
        assert!(!h.state().editor_compare.owner_is_open());
    }
    #[test]
    fn escape_timeout_other_key_and_document_switch_cancel_prefix_and_ctrl_g_stays_go_to_line() {
        let (d, path, mut h) = fixture("one\ntwo\n", true);
        prefix(&mut h);
        h.key_press(Key::Escape);
        h.step();
        assert!(!h.state().editor_shortcuts.diff_pending());
        assert!(!h.state().git_ui.commit_open);
        prefix(&mut h);
        h.input_mut().time = Some(h.ctx.input(|i| i.time) + 6.);
        h.step();
        assert!(!h.state().editor_shortcuts.diff_pending());
        prefix(&mut h);
        h.key_press(Key::ArrowRight);
        h.step();
        assert!(!h.state().editor_shortcuts.diff_pending());
        prefix(&mut h);
        let other = d.path().join("other.php");
        fs::write(&other, "other").unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(other))
            .unwrap();
        settle(&mut h);
        assert!(!h.state().editor_shortcuts.diff_pending());
        assert!(!h.state().editor_compare.owner_is_open());
        h.key_press_modifiers(Modifiers::COMMAND, Key::G);
        h.step();
        h.step();
        h.get_by_label("Ir para linha");
        assert!(h.state().editor_shortcuts.go_to_line_open());
        assert_eq!(fs::read_to_string(path).unwrap(), "one\ntwo\n");
    }
}
