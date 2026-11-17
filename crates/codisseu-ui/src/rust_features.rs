//! Rust-specific affordances over shared document, language and terminal services.
use super::*;
use codisseu_core::{Language, ProviderState, rust_project};
use std::{
    hash::{Hash, Hasher},
    sync::mpsc::Receiver,
};

#[derive(Default)]
pub(super) struct RustFeatureUi {
    build_key: Option<(PathBuf, PathBuf)>,
    discovery: Option<Receiver<Result<PathBuf, String>>>,
    manifest: Option<PathBuf>,
    hint_target: Option<(DocumentId, u64, u64, usize, usize)>,
    hint_after: f64,
    hint_requested: bool,
    hint_attempts: u8,
    action_state: (bool, bool, bool),
    pub(super) quick_fixes: super::code_actions::PopupState,
}
impl CodisseuApp {
    pub(super) fn rust_build_enabled(&self) -> bool {
        self.service.active_document().is_some_and(|d| {
            !d.is_read_only()
                && (Language::for_path(&d.path) == Some(Language::Rust)
                    || d.path.file_name().is_some_and(|n| n == "Cargo.toml"))
        }) && self.rust_features.manifest.is_some()
            && self.rust_features.discovery.is_none()
    }
    pub(super) fn rust_build_toolbar(&mut self, ui: &mut egui::Ui) {
        let enabled = self.rust_build_enabled() && !self.exit_when_saved;
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                icon_button(
                    ui,
                    Icon::Build,
                    "Build Rust · Cargo build · Ctrl+F9",
                    false,
                    30.,
                )
            })
            .inner;
        if !enabled {
            response.clone().on_disabled_hover_text("Build disponível em arquivos Rust de um projeto Cargo. Outras linguagens não têm build integrado neste contexto.");
        }
        if response.clicked() {
            self.open_cargo(rust_project::CargoAction::Build);
        }
    }
    pub(super) fn rust_features_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            return;
        }
        let action_state = (
            self.service.code_actions.is_open(),
            self.service.code_actions.is_pending(),
            self.service.code_actions.has_preview(),
        );
        if action_state != self.rust_features.action_state {
            if self.rust_features.action_state.0 && !action_state.0 {
                self.focus_editor = true;
            }
            if action_state.0
                || (self.rust_features.action_state.0 && self.service.last_error.is_none())
            {
                status_messages::publish(ctx, &self.service.status);
            }
            self.rust_features.action_state = action_state;
        }
        let key = self
            .service
            .active_document()
            .filter(|d| {
                !d.is_read_only()
                    && (Language::for_path(&d.path) == Some(Language::Rust)
                        || d.path.file_name().is_some_and(|n| n == "Cargo.toml"))
            })
            .and_then(|d| {
                self.service
                    .language_project_root()
                    .map(|r| (r, d.path.clone()))
            });
        if key != self.rust_features.build_key {
            self.rust_features.build_key = key.clone();
            self.rust_features.manifest = None;
            self.rust_features.discovery = None;
            if let Some((root, path)) = key {
                let (send, receive) = mpsc::sync_channel(1);
                if std::thread::Builder::new()
                    .name("codisseu-build-context".into())
                    .spawn(move || {
                        let _ = send.send(rust_project::discover_manifest(&root, Some(&path)));
                    })
                    .is_ok()
                {
                    self.rust_features.discovery = Some(receive);
                }
            }
        }
        if let Some(receiver) = &self.rust_features.discovery {
            match receiver.try_recv() {
                Ok(result) => {
                    self.rust_features.manifest = result.ok();
                    self.rust_features.discovery = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.rust_features.discovery = None,
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(20))
                }
            }
        }
        let Some(doc) = self.service.active_document().filter(|d| {
            !d.is_read_only()
                && matches!(
                    Language::for_path(&d.path),
                    Some(Language::Rust | Language::Python)
                )
                && self.service.inlay_hints_enabled(d.id)
        }) else {
            if self.rust_features.hint_target.take().is_some() {
                let _ = self.service.dispatch(Command::CancelInlayHints);
            }
            return;
        };
        if self.service.code_actions.is_open() || self.rename_editor_input_blocked() {
            return;
        }
        let language = Language::for_path(&doc.path).unwrap();
        let supported = self
            .service
            .language
            .rust_capabilities
            .get(&language)
            .is_some_and(|c| c.inlay_hints);
        if !supported
            || !self
                .service
                .language
                .statuses
                .iter()
                .any(|s| s.language == language && s.state == ProviderState::Ready)
        {
            return;
        }
        let count = doc.line_count();
        let metrics = &self.editor_bridge.metrics;
        let first = if metrics.document == Some(doc.id) {
            metrics.visible_start.saturating_sub(12)
        } else {
            0
        };
        let end = if metrics.document == Some(doc.id) {
            (metrics.visible_end + 12).max(first + 1).min(count)
        } else {
            count.min(100)
        };
        // Bucket small scroll changes: requests follow the visible editor without running per pixel.
        let first = (first / 16) * 16;
        let end = (end.div_ceil(16) * 16)
            .min(count)
            .min(first + 256)
            .max(first + 1);
        let mut state = std::collections::hash_map::DefaultHasher::new();
        for d in &self.service.documents {
            d.id.hash(&mut state);
            d.version.hash(&mut state);
        }
        let target = (doc.id, doc.version, state.finish(), first, end);
        let now = ctx.input(|i| i.time);
        if self.rust_features.hint_target != Some(target) {
            self.rust_features.hint_target = Some(target);
            self.rust_features.hint_after = now + 0.25;
            self.rust_features.hint_requested = false;
            self.rust_features.hint_attempts = 0;
        }
        if self.rust_features.hint_requested
            && !self.service.inlay_hints.is_pending()
            && (self.service.inlay_hints.error.is_some()
                || self.service.inlay_hints.items.is_empty())
            && self.rust_features.hint_attempts < 3
        {
            self.rust_features.hint_attempts += 1;
            self.rust_features.hint_requested = false;
            self.rust_features.hint_after = now + f64::from(self.rust_features.hint_attempts);
        }
        if !self.rust_features.hint_requested && now >= self.rust_features.hint_after {
            if self
                .service
                .dispatch(Command::InlayHints {
                    id: target.0,
                    expected_version: target.1,
                    lines: first..end,
                })
                .is_ok()
            {
                self.rust_features.hint_requested = true;
            } else {
                self.rust_features.hint_after = now + 0.5;
            }
        }
        if !self.rust_features.hint_requested || self.service.inlay_hints.is_pending() {
            ctx.request_repaint_after(Duration::from_millis(30));
        }
    }
    pub(super) fn code_actions_window(&mut self, ctx: &egui::Context) {
        if self
            .service
            .active_document()
            .is_some_and(|d| Language::for_path(&d.path) != Some(Language::Rust))
        {
            self.quick_fix_popup(ctx);
            return;
        }
        if !self.service.code_actions.is_open() {
            return;
        }
        let pending = self.service.code_actions.is_pending();
        let preview = self.service.code_actions.has_preview();
        let mut cancel = false;
        let mut selected = None;
        let mut apply = false;
        egui::Modal::new(egui::Id::unique("codisseu.rust.code-actions"))
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(16)).show(ctx,|ui|{
                self.tokens.form_style(ui);
                ui.set_width((ctx.content_rect().width()-80.).clamp(260.,850.));
                ui.horizontal(|ui|{ui.strong(if preview{"Preview da correção Rust"}else{"Correções rápidas · Rust"});ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{cancel=icon_button(ui,Icon::Close,"Fechar correções Rust",false,26.).clicked();});});
                ui.separator();
                egui::ScrollArea::vertical().id_salt("rust.code-actions.body").max_height((ctx.content_rect().height()-180.).clamp(120.,420.)).auto_shrink([false,false]).codisseu().show(ui,|ui|{
                    if pending{ui.label("Consultando e validando a correção…");}
                    else if preview {
                        ui.label(RichText::new(&self.service.code_actions.title).strong());
                        for file in &self.service.code_actions.files {
                            ui.add(egui::Label::new(file.path.display().to_string()).wrap());
                            ui.columns(2,|cols|{
                                for (column,label,text) in [(0,"Antes",&file.before),(1,"Depois",&file.after)] {
                                    cols[column].label(label);
                                    let mut excerpt=text.lines().take(150).collect::<Vec<_>>().join("\n");
                                    if excerpt.len()>32768 {let end=excerpt.char_indices().map(|(i,_)|i).take_while(|i|*i<=32768).last().unwrap_or(0);excerpt.truncate(end);}
                                    egui::ScrollArea::both().id_salt(("rust.fix.preview",&file.path,column)).max_height(250.).auto_shrink([false,false]).codisseu().show(&mut cols[column],|ui|{
                                        rust_preview_editor(ui, &mut excerpt, &self.tokens);
                                    });
                                    if text.lines().count()>150||text.len()>32768{cols[column].label("Preview limitado; a proposta completa será aplicada.");}
                                }
                            });ui.separator();
                        }
                        ui.add(egui::Label::new("Aplicar altera os buffers. Revise e use Salvar para gravar os arquivos.").wrap());
                    }else if self.service.code_actions.items.is_empty(){ui.label("Nenhuma correção rápida nesta posição.");}
                    else{
                        for item in &self.service.code_actions.items {
                            let label=if item.preferred{format!("{}  · recomendada",item.title)}else{item.title.clone()};
                            let response=ui.add_enabled(item.disabled_reason.is_none(),egui::Button::new(label).min_size(egui::vec2(ui.available_width(),28.)));
                            if let Some(reason)=&item.disabled_reason{response.clone().on_disabled_hover_text(reason);}
                            if response.clicked(){selected=Some(item.token);}
                        }
                        if self.service.code_actions.truncated{ui.label("Lista limitada às primeiras correções recebidas.");}
                    }
                });
                ui.separator();ui.horizontal(|ui|{
                    if ui.add(self.tokens.dialog_button("Cancelar",false)).clicked(){cancel=true;}
                    if preview&&ui.add_enabled(!pending,self.tokens.dialog_button("Aplicar correção",true)).clicked(){apply=true;}
                });
            });
        cancel |= consume_exact(ctx, Modifiers::NONE, Key::Escape);
        if cancel {
            self.send(Command::CancelCodeActions);
            self.focus_editor = true;
        } else if let Some(token) = selected {
            self.send(Command::PreviewCodeAction { token });
        } else if apply {
            self.send(Command::ApplyCodeAction);
            self.focus_editor = true;
        }
    }
}

/// The exact lexical highlighter/palette used by the shared editor, bounded by
/// the excerpt before this function. Source is readonly; color is presentation.
fn rust_preview_job(source: &str, tokens: &Tokens) -> egui::text::LayoutJob {
    let mut job = highlight::layout(source, "rs", FontId::monospace(tokens.editor_font_size));
    job.wrap.max_width = f32::INFINITY;
    for section in &mut job.sections {
        section.format.line_height = Some(tokens.editor_line_height);
        section.format.extra_letter_spacing = tokens.editor_letter_spacing;
    }
    job
}
fn rust_preview_editor(ui: &mut egui::Ui, text: &mut String, tokens: &Tokens) {
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _width: f32| {
        ui.fonts_mut(|fonts| fonts.layout_job(rust_preview_job(buffer.as_str(), tokens)))
    };
    ui.add(
        egui::TextEdit::multiline(text)
            .code_editor()
            .interactive(false)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn readonly_preview_uses_shared_rust_colors_and_preserves_text() {
        let source = "// ação 😀\nfn combine(first: i32) -> i32 { let result = first + 42; println!(\"{result}\"); result }\n";
        let tokens = Tokens::load();
        let actual = rust_preview_job(source, &tokens);
        let editor = highlight::layout(source, "rs", FontId::monospace(tokens.editor_font_size));
        assert_eq!(actual.text, source);
        assert_eq!(actual.sections.len(), editor.sections.len());
        for (preview, source) in actual.sections.iter().zip(&editor.sections) {
            assert_eq!(preview.byte_range, source.byte_range);
            assert_eq!(preview.format.color, source.format.color);
            assert_eq!(preview.format.font_id, source.format.font_id);
        }
        let distinct = actual
            .sections
            .iter()
            .map(|s| s.format.color)
            .collect::<std::collections::HashSet<_>>();
        assert!(
            distinct.len() >= 5,
            "comments/functions/keywords/numbers/strings use their editor colors"
        );
    }
}
