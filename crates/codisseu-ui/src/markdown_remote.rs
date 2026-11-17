//! Bounded reader using the same parser/renderer, with local resources disabled.
use super::*;
use std::sync::mpsc::{self, Receiver, SyncSender};
type ParsedReply = (u64, Result<parse::Parsed, String>);
#[derive(Default)]
pub(crate) struct RemoteMarkdown {
    tx: Option<SyncSender<(u64, String)>>,
    rx: Option<Receiver<ParsedReply>>,
    busy: bool,
    source: String,
    version: u64,
    sent: u64,
    parsed: Option<parse::Parsed>,
    error: Option<String>,
    render: render::RenderState,
}
impl RemoteMarkdown {
    pub(crate) fn image_urls(&self) -> Vec<String> {
        self.parsed
            .as_ref()
            .map(|p| p.images.keys().cloned().collect())
            .unwrap_or_default()
    }
    pub(crate) fn install_image(&mut self, url: &str, image: Arc<egui::ColorImage>) {
        if let Some(p) = &mut self.parsed
            && p.images.get(url).is_some_and(Result::is_err)
        {
            let used = p
                .images
                .values()
                .filter_map(|i| i.as_ref().ok())
                .map(|i| i.pixels.len() * 4)
                .sum::<usize>();
            if used + image.pixels.len() * 4 > 24 * 1024 * 1024 {
                p.images.insert(
                    url.into(),
                    Err("Imagens desta leitura excederam24MiB".into()),
                );
                return;
            }
            p.images.insert(url.into(), Ok(image));
            self.render.clear_layout();
        }
    }
    pub(crate) fn image_error(&mut self, url: &str, error: String) {
        if let Some(p) = &mut self.parsed
            && let Some(Err(e)) = p.images.get_mut(url)
        {
            *e = error;
        }
    }
    pub(crate) fn set(&mut self, source: String) {
        if self.source != source {
            self.source = source;
            self.version = self.version.wrapping_add(1);
            self.parsed = None;
            self.error = None
        }
    }
    pub(crate) fn clear(&mut self) {
        self.set(String::new());
        self.render.release_resources()
    }
    fn prepare(&mut self, ui: &egui::Ui) {
        if self.tx.is_none() && self.error.is_none() {
            let (tx, jobs) = mpsc::sync_channel::<(u64, String)>(1);
            let (replies, rx) = mpsc::sync_channel(1);
            match thread::Builder::new()
                .name("codisseu-jira-markdown".into())
                .spawn(move || {
                    while let Ok((id, s)) = jobs.recv() {
                        let result = parse::parse_remote(&s, &AtomicBool::new(false));
                        if replies.send((id, result)).is_err() {
                            break;
                        }
                    }
                }) {
                Ok(_) => {
                    self.tx = Some(tx);
                    self.rx = Some(rx)
                }
                Err(_) => self.error = Some("Não foi possível iniciar a leitura Markdown".into()),
            }
        }
        if let Some(rx) = &self.rx
            && let Ok((id, p)) = rx.try_recv()
        {
            ui.ctx().request_repaint();
            self.busy = false;
            if id == self.version {
                match p {
                    Ok(p) => self.parsed = Some(p),
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if !self.busy
            && self.sent != self.version
            && let Some(tx) = &self.tx
        {
            if self.source.len() > parse::MAX_SOURCE {
                self.error = Some("Conteúdo acima do limite de leitura de2MiB".into())
            } else if tx.try_send((self.version, self.source.clone())).is_ok() {
                self.sent = self.version;
                self.busy = true
            }
        }
    }
    /// Compact, inert presentation for an already accepted documentation reply.
    pub(crate) fn show_documentation(
        &mut self,
        ui: &mut egui::Ui,
        tokens: &Tokens,
        extension: &str,
    ) {
        self.prepare(ui);
        if let Some(parsed) = &self.parsed {
            render::show_documentation(
                ui,
                tokens,
                self.version,
                parsed,
                &mut self.render,
                extension,
            );
        } else if let Some(error) = &self.error {
            ui.add(egui::Label::new(&self.source).wrap().selectable(true))
                .on_hover_text(error);
        } else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Preparando documentação…");
            });
        }
        if self.busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(20));
        }
    }
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        if self.source.is_empty() {
            ui.label("Sem conteúdo nesta seção.");
            return;
        }
        self.prepare(ui);
        if let Some(error) = &self.error {
            ui.label(error);
            return;
        }
        if let Some(p) = &self.parsed {
            let actions = render::show(
                ui,
                tokens,
                u64::MAX,
                u64::MAX,
                self.version,
                p,
                &mut self.render,
                1.,
                false,
                None,
            );
            for action in actions {
                if let MarkdownAction::OpenExternal { url } = action {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url))
                }
            }
        } else {
            ui.spinner();
            ui.label("Preparando leitura…");
        }
        if self.busy {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(30))
        }
    }
}
