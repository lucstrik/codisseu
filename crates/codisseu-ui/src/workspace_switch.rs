//! Retain the selected path while safe backend acknowledgements drain.
use super::*;
use codisseu_core::WorkspaceChangeBlocker;

#[derive(Default)]
pub(super) struct WorkspaceSwitchUi {
    pending: Option<Request>,
}

struct Request {
    origin: Option<PathBuf>,
    path: PathBuf,
    cancelled_reads: bool,
    saving: bool,
    opening: bool,
    error: Option<String>,
    message: String,
}
impl CodisseuApp {
    pub(super) fn workspace_switch_pending(&self) -> bool {
        self.workspace_switch.pending.is_some()
    }
    pub(super) fn request_workspace_switch(&mut self, path: PathBuf) {
        if self.exit_when_saved || self.workspace_switch_pending() {
            return;
        }
        self.workspace_switch.pending = Some(Request {
            origin: self.service.workspace_path().map(PathBuf::from),
            path,
            cancelled_reads: false,
            saving: false,
            opening: false,
            error: None,
            message: "Preparando abertura do projeto".into(),
        });
    }
    pub(super) fn poll_workspace_switch(&mut self, ctx: &egui::Context) {
        let Some(mut request) = self.workspace_switch.pending.take() else {
            return;
        };
        if self.exit_when_saved {
            return;
        }
        if request.opening {
            if self.service.workspace_open_pending() {
                ctx.request_repaint_after(Duration::from_millis(20));
            } else if self.service.workspace_path() != request.origin.as_deref()
                || (self.service.last_error.is_none()
                    && self.service.workspace_path() == Some(request.path.as_path()))
            {
                self.focus_editor = true;
                return;
            } else {
                request.opening = false;
                request.error = Some(
                    self.service
                        .last_error
                        .clone()
                        .unwrap_or_else(|| "Não foi possível abrir o projeto escolhido".into()),
                );
            }
        } else if self.service.workspace_path() != request.origin.as_deref() {
            return;
        } else if request.error.is_none() {
            if self.service.workspace_path() != Some(request.path.as_path()) {
                if !self.database_ui.staged.is_empty() || self.database_ui.edit.is_some() {
                    request.error = Some("Há alterações na grade do banco. Aplique ou cancele essas alterações antes de abrir outro projeto.".into());
                    self.workspace_switch.pending = Some(request);
                    return;
                }
                if self.transfer_work_pending() {
                    request.message = "Aguardando a transferência de dados terminar".into();
                    self.workspace_switch.pending = Some(request);
                    ctx.request_repaint_after(Duration::from_millis(30));
                    return;
                }
            }
            if !request.cancelled_reads {
                request.cancelled_reads = self.service.cancel_workspace_reads().is_ok();
            }
            match self.service.workspace_change_blocker(&request.path) {
                Some(WorkspaceChangeBlocker::Busy(message)) => {
                    request.message = message.into();
                    ctx.request_repaint_after(Duration::from_millis(30));
                }
                Some(WorkspaceChangeBlocker::UnsavedDocuments) => {
                    if request.saving {
                        request.saving = false;
                        request.error = Some(self.service.last_error.clone().unwrap_or_else(|| "Há arquivos que ainda não foram salvos. O projeto atual foi mantido.".into()));
                    }
                    request.message =
                        "Salve os arquivos alterados antes de abrir outro projeto".into();
                }
                None => {
                    self.ai_flush_draft();
                    match self
                        .service
                        .dispatch(Command::OpenWorkspace(request.path.clone()))
                    {
                        Ok(()) => {
                            request.opening = true;
                            request.message = "Abrindo projeto".into();
                            ctx.request_repaint_after(Duration::from_millis(20));
                        }
                        Err(error) => request.error = Some(error),
                    }
                }
            }
        }
        self.workspace_switch.pending = Some(request);
    }
    pub(super) fn workspace_switch_window(&mut self, ctx: &egui::Context) {
        let Some(request) = self.workspace_switch.pending.as_ref() else {
            return;
        };
        let dirty = matches!(
            self.service.workspace_change_blocker(&request.path),
            Some(WorkspaceChangeBlocker::UnsavedDocuments)
        );
        let untitled = self
            .service
            .documents
            .iter()
            .any(|d| d.is_dirty() && d.is_untitled());
        let mut cancel = false;
        let mut save = false;
        let modal = egui::Modal::new(egui::Id::unique("codisseu.workspace.switch"))
            .frame(self.tokens.dialog_frame(ctx))
            .show(ctx, |ui| {
                let title = if request.error.is_some() {
                    "Não foi possível abrir o projeto"
                } else if dirty {
                    "Alterações não salvas"
                } else {
                    "Abrir projeto"
                };
                self.tokens.dialog_header(ui, ctx, 500., title);
                ui.add(
                    egui::Label::new(
                        RichText::new(request.path.display().to_string())
                            .color(color(self.tokens.text)),
                    )
                    .wrap(),
                );
                if let Some(error) = &request.error {
                    ui.add(
                        egui::Label::new(RichText::new(error).color(color(self.tokens.error)))
                            .wrap(),
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if !dirty {
                            ui.add(
                                egui::Spinner::new()
                                    .size(16.)
                                    .color(color(self.tokens.muted)),
                            );
                        }
                        ui.label(&request.message);
                    });
                    if dirty && untitled {
                        ui.add(
                            egui::Label::new(
                                RichText::new("Volte ao editor para salvar os arquivos sem nome.")
                                    .color(color(self.tokens.muted)),
                            )
                            .wrap(),
                        );
                    }
                }
                self.tokens.dialog_footer(ui, |ui| {
                    if dirty && !untitled && request.error.is_none() {
                        save = ui
                            .add(self.tokens.dialog_button("Salvar e abrir", true))
                            .clicked();
                    }
                    cancel = ui
                        .add(self.tokens.dialog_button(
                            if dirty || request.error.is_some() {
                                "Voltar ao editor"
                            } else {
                                "Cancelar abertura"
                            },
                            false,
                        ))
                        .clicked();
                });
            });
        cancel |= modal.is_top_modal
            && !modal.any_popup_open
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
        if cancel {
            self.service.cancel_workspace_open();
            self.workspace_switch = Default::default();
            self.focus_editor = true;
        } else if save {
            let ids: Vec<_> = self
                .service
                .documents
                .iter()
                .filter(|d| d.is_dirty())
                .map(|d| d.id)
                .collect();
            let mut error = None;
            for id in ids {
                if let Err(failed) = self.service.dispatch(Command::Save { id }) {
                    error = Some(failed);
                    break;
                }
            }
            if let Some(request) = &mut self.workspace_switch.pending {
                request.saving = error.is_none();
                request.error = error;
                request.message = "Salvando arquivos antes de abrir o projeto".into();
            }
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod dialog_style_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[derive(Clone, Copy)]
    enum Dialog {
        Switch,
        Trash,
        Shutdown,
    }

    fn harness(
        dir: &std::path::Path,
        scale: f32,
        width: f32,
        dialog: Dialog,
    ) -> Harness<'static, CodisseuApp> {
        let mut app = CodisseuApp::new(AppService::new(dir.join("profile/session.json")));
        match dialog {
            Dialog::Switch => {
                app.request_workspace_switch(PathBuf::from(
                    "/home/fixture/Projetos/Projeto de integração com nome longo e acentuação",
                ));
                app.workspace_switch.pending.as_mut().unwrap().error = Some(
                    "Não foi possível abrir a pasta escolhida. O projeto atual foi mantido.".into(),
                );
            }
            Dialog::Trash => {
                app.project_files_ui.trash = Some(vec![
                    PathBuf::from("src/Relatório de integração com nome longo.php"),
                    PathBuf::from("src/secondary.ts"),
                ]);
            }
            Dialog::Shutdown => {
                app.exit_when_saved = true;
                app.shutdown_started = Some(Instant::now() - Duration::from_secs(1));
            }
        }
        let mut h = Harness::builder()
            .with_size([width, 600.])
            .with_pixels_per_point(scale)
            .build_ui_state(
                move |ui, app: &mut CodisseuApp| {
                    if !app.styled {
                        app.tokens.apply(ui.ctx());
                        app.styled = true;
                    }
                    let ctx = ui.ctx();
                    match dialog {
                        Dialog::Switch => app.workspace_switch_window(ctx),
                        Dialog::Trash => app.project_file_dialog(ctx),
                        Dialog::Shutdown => app.shutdown_window(ctx),
                    }
                },
                app,
            );
        // Modal placement and font upload settle without waiting for the spinner
        // to stop repainting. This is a real widget render, not native-window QA.
        for _ in 0..12 {
            h.step();
        }
        h
    }

    fn visit(shape: &egui::epaint::Shape, callback: &mut impl FnMut(&egui::epaint::Shape)) {
        callback(shape);
        if let egui::epaint::Shape::Vec(shapes) = shape {
            for shape in shapes {
                visit(shape, callback);
            }
        }
    }

    #[test]
    fn dialog_style_real_modals_keep_font_padding_buttons_and_cancel_in_two_scales() {
        for scale in [1., 1.25] {
            for width in [800., 320.] {
                for (dialog, name, title, cancel) in [
                    (
                        Dialog::Switch,
                        "workspace",
                        "Não foi possível abrir o projeto",
                        "Voltar ao editor",
                    ),
                    (Dialog::Trash, "trash", "Move 2 items to Trash?", "Cancel"),
                    (
                        Dialog::Shutdown,
                        "shutdown",
                        "Fechando o Codisseu",
                        "Voltar ao editor",
                    ),
                ] {
                    let dir = tempfile::tempdir().unwrap();
                    let mut h = harness(dir.path(), scale, width, dialog);
                    let heading = h.get_by_label(title).rect();
                    let button = h.get_by_label(cancel).rect();
                    assert_eq!(button.height(), h.state().tokens.control_height);
                    let viewport = h.ctx.content_rect();
                    assert!(
                        viewport.contains_rect(heading),
                        "{name} heading: {heading:?}"
                    );
                    assert!(viewport.contains_rect(button), "{name} button: {button:?}");
                    let mut frame = None;
                    let mut heading_font = false;
                    for clipped in &h.output().shapes {
                        visit(&clipped.shape, &mut |shape| match shape {
                            egui::epaint::Shape::Rect(rect)
                                if rect.fill == color(h.state().tokens.panel)
                                    && rect.rect.contains_rect(heading)
                                    && rect.rect.contains_rect(button) =>
                            {
                                frame = Some(rect.rect);
                            }
                            egui::epaint::Shape::Text(text) if text.galley.text() == title => {
                                heading_font = true;
                                assert!(
                                    text.galley.job.sections.iter().all(|section| section
                                        .format
                                        .font_id
                                        .size
                                        == h.state().tokens.ui_font_size)
                                );
                            }
                            _ => {}
                        });
                    }
                    assert!(heading_font, "{name} title must use the UI font");
                    let frame = frame.expect("themed modal frame");
                    let inset = f32::from(h.state().tokens.dialog_content_inset);
                    assert!(heading.left() - frame.left() >= inset - 1.);
                    assert!(heading.top() - frame.top() >= inset - 1.);
                    assert!(frame.bottom() - button.bottom() >= inset - 1.);
                    assert!(frame.right() - button.right() >= inset - 1.);
                    if let Some(out) = std::env::var_os("CODISSEU_DIALOG_CAPTURE_DIR") {
                        let out = PathBuf::from(out);
                        std::fs::create_dir_all(&out).unwrap();
                        h.render()
                            .unwrap()
                            .save(out.join(format!("{name}-{width}-{scale}.png")))
                            .unwrap();
                    }
                    h.get_by_label(cancel).click();
                    h.step();
                    match dialog {
                        Dialog::Switch => assert!(!h.state().workspace_switch_pending()),
                        Dialog::Trash => assert!(h.state().project_files_ui.trash.is_none()),
                        Dialog::Shutdown => assert!(!h.state().exit_when_saved),
                    }
                    assert!(h.state().focus_editor || matches!(dialog, Dialog::Trash));
                    assert!(!dir.path().join("profile/session.json").exists());
                }
            }
        }
    }
}
