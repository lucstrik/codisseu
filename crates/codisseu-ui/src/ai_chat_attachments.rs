//! Composer attachments; filesystem reads and native file dialogs stay off the GUI.
use crate::components::{Icon, icon_button, paint_icon};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_core::ai::attachments::{
    AttachmentInput, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, rgba_size,
};
use egui::{ColorImage, Context, TextureHandle, Ui};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};

#[derive(Default)]
pub(super) struct AttachmentUi {
    pub(super) error: Option<String>,
    entries: Vec<Entry>,
    pending: Option<PendingPicker>,
    clipboard: Option<PendingClipboard>,
    pasted_text: Option<String>,
    generation: u64,
    next_image: u64,
    rgba_bytes: usize,
    other_rgba_bytes: usize,
    scope: Option<egui::Id>,
}

struct Entry {
    input: AttachmentInput,
    label: String,
    thumbnail: Option<ColorImage>,
    texture: Option<TextureHandle>,
    image_id: u64,
}

struct PendingPicker {
    origin: PathBuf,
    generation: u64,
    receive: Receiver<Option<Vec<PathBuf>>>,
}

static CLIPBOARD_WORKERS: AtomicUsize = AtomicUsize::new(0);
struct ClipboardPermit;
impl Drop for ClipboardPermit {
    fn drop(&mut self) {
        CLIPBOARD_WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

struct PendingClipboard {
    generation: u64,
    receive: Receiver<ClipboardContent>,
}

enum ClipboardContent {
    Files(Vec<PathBuf>),
    Image(Arc<ColorImage>),
    Text(String),
    Empty,
    Error(String),
}

/// Register the visible composer as the native paste owner. The backend checks
/// current focus when the key arrives, after this frame's focus transitions.
pub(super) fn set_native_paste_enabled(ctx: &Context, enabled: bool) {
    let key = egui::Id::unique("egui_winit.defer_clipboard_paste");
    ctx.data_mut(|data| {
        if enabled {
            data.insert_temp(key, egui::Id::unique("codisseu.ai.composer"));
        } else {
            data.remove::<egui::Id>(key);
        }
    });
}

fn read_clipboard() -> ClipboardContent {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return ClipboardContent::Error("Não foi possível acessar a área de transferência.".into());
    };
    // MIME, not the spelling of text, establishes that the user copied files.
    if let Ok(mut paths) = clipboard.get().file_list()
        && !paths.is_empty()
    {
        paths.truncate(MAX_ATTACHMENTS + 1);
        return ClipboardContent::Files(paths);
    }
    if let Ok(image) = clipboard.get_image() {
        let dimensions = u32::try_from(image.width)
            .ok()
            .zip(u32::try_from(image.height).ok());
        let Some((width, height)) = dimensions else {
            return ClipboardContent::Error("A imagem colada possui dimensões inválidas.".into());
        };
        match rgba_size(width, height) {
            Ok(size) if image.bytes.len() == size => {
                return ClipboardContent::Image(Arc::new(ColorImage::from_rgba_unmultiplied(
                    [image.width, image.height],
                    &image.bytes,
                )));
            }
            _ => {
                return ClipboardContent::Error(
                    "A imagem colada é inválida ou excede o limite de tamanho.".into(),
                );
            }
        }
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => ClipboardContent::Text(text.replace("\r\n", "\n")),
        _ => ClipboardContent::Empty,
    }
}

impl AttachmentUi {
    pub(super) fn rgba_bytes(&self) -> usize {
        self.rgba_bytes
    }
    pub(super) fn reserve_other_rgba(&mut self, bytes: usize) {
        self.other_rgba_bytes = bytes;
    }
    pub(super) fn set_scope(&mut self, scope: &(impl std::hash::Hash + std::fmt::Debug)) {
        self.scope = Some(egui::Id::unique(scope));
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.clipboard = None;
        self.pasted_text = None;
        self.error = None;
        self.rgba_bytes = 0;
        self.generation = self.generation.wrapping_add(1);
        // An already open OS dialog finishes normally. Its old generation is ignored.
    }

    pub(super) fn inputs(&self) -> Vec<AttachmentInput> {
        self.entries
            .iter()
            .map(|entry| entry.input.clone())
            .collect()
    }

    pub(super) fn add_project_file(&mut self, path: PathBuf) {
        self.add_file(path, true);
    }

    fn add_file(&mut self, path: PathBuf, project_file: bool) {
        if self.entries.iter().any(|entry| {
            matches!(&entry.input, AttachmentInput::ProjectFile(existing) | AttachmentInput::ExternalFile(existing) if existing == &path)
        }) {
            return;
        }
        if self.len() >= MAX_ATTACHMENTS {
            self.error = Some("Anexe no máximo 8 arquivos ou imagens por mensagem.".into());
            return;
        }
        let label = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let input = if project_file {
            AttachmentInput::ProjectFile(path)
        } else {
            AttachmentInput::ExternalFile(path)
        };
        self.entries.push(Entry {
            input,
            label,
            thumbnail: None,
            texture: None,
            image_id: 0,
        });
        self.error = None;
    }

    pub(super) fn choose_files(&mut self, origin: Option<PathBuf>, ctx: Context) {
        if self.pending.is_some() {
            return;
        }
        let Some(origin) = origin else {
            self.error = Some("Abra um projeto antes de anexar arquivos.".into());
            return;
        };
        if self.len() >= MAX_ATTACHMENTS {
            self.error = Some("Anexe no máximo 8 arquivos ou imagens por mensagem.".into());
            return;
        }
        let initial = origin.clone();
        let (send, receive) = mpsc::sync_channel(1);
        let spawn = std::thread::Builder::new()
            .name("codisseu-ai-file-picker".into())
            .spawn(move || {
                let selected = rfd::FileDialog::new()
                    .set_title("Anexar arquivos ao chat")
                    .set_directory(initial)
                    .pick_files()
                    .map(|mut paths| {
                        // Bound the message to the GUI even for very large selections.
                        paths.truncate(MAX_ATTACHMENTS + 1);
                        paths
                    });
                let _ = send.send(selected);
                ctx.request_repaint();
            });
        match spawn {
            Ok(_) => {
                self.pending = Some(PendingPicker {
                    origin,
                    generation: self.generation,
                    receive,
                });
                self.error = None;
            }
            Err(error) => {
                self.error = Some(format!(
                    "Não foi possível abrir o seletor de arquivos: {error}"
                ))
            }
        }
    }

    pub(super) fn poll(&mut self, ctx: &Context, workspace: Option<&Path>) -> bool {
        let clipboard_changed = self.poll_clipboard(ctx);
        self.poll_picker(ctx, workspace) || clipboard_changed
    }

    fn poll_picker(&mut self, ctx: &Context, workspace: Option<&Path>) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };
        let result = match pending.receive.try_recv() {
            Ok(paths) => Ok(paths),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(100));
                return false;
            }
            Err(TryRecvError::Disconnected) => {
                Err("O seletor de arquivos foi encerrado inesperadamente.".to_owned())
            }
        };
        let pending = self.pending.take().expect("pending picker checked above");
        if pending.generation != self.generation || workspace != Some(pending.origin.as_path()) {
            return false;
        }
        match result {
            Ok(Some(paths)) => {
                for path in paths.into_iter().take(MAX_ATTACHMENTS + 1) {
                    self.add_file(path, false);
                }
            }
            Ok(None) => (),
            Err(error) => self.error = Some(error),
        }
        ctx.request_repaint();
        true
    }

    pub(super) fn has_pending_input(&self) -> bool {
        self.clipboard.is_some() || self.pasted_text.is_some()
    }

    fn poll_clipboard(&mut self, ctx: &Context) -> bool {
        let Some(pending) = &self.clipboard else {
            return false;
        };
        let content = match pending.receive.try_recv() {
            Ok(content) => content,
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(40));
                return false;
            }
            Err(TryRecvError::Disconnected) => ClipboardContent::Error(
                "A leitura da área de transferência foi interrompida.".into(),
            ),
        };
        let pending = self.clipboard.take().expect("pending clipboard checked");
        if pending.generation != self.generation {
            return false;
        }
        match content {
            ClipboardContent::Files(paths) => {
                for path in paths {
                    self.add_file(path, false);
                }
            }
            ClipboardContent::Image(image) => self.add_image(image),
            ClipboardContent::Text(text) => self.pasted_text = Some(text),
            ClipboardContent::Empty => (),
            ClipboardContent::Error(error) => self.error = Some(error),
        }
        ctx.request_repaint();
        true
    }

    fn start_clipboard(&mut self, ctx: &Context) {
        if self.clipboard.is_some() || self.pasted_text.is_some() {
            self.error = Some("Aguarde a colagem anterior antes de colar novamente.".into());
            return;
        }
        if CLIPBOARD_WORKERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |workers| {
                (workers < 4).then_some(workers + 1)
            })
            .is_err()
        {
            self.error =
                Some("Aguarde a leitura da área de transferência nas outras conversas.".into());
            return;
        }
        let permit = ClipboardPermit;
        let (send, receive) = mpsc::sync_channel(1);
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("codisseu-ai-clipboard".into())
            .spawn(move || {
                let _permit = permit;
                let _ = send.send(read_clipboard());
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.clipboard = Some(PendingClipboard {
                    generation: self.generation,
                    receive,
                })
            }
            Err(error) => {
                self.error = Some(format!(
                    "Não foi possível ler a área de transferência: {error}"
                ))
            }
        }
    }

    /// Called before rendering the composer; other focused surfaces retain their events.
    pub(super) fn consume_paste(&mut self, ctx: &Context) {
        if ctx.memory(|memory| memory.focused()) != Some(egui::Id::unique("codisseu.ai.composer")) {
            return;
        }
        let (images, native_paste) = ctx.input_mut(|input| {
            let mut images = Vec::new();
            let mut native_paste = false;
            input.events.retain(|event| match event {
                egui::Event::PasteImage(image) => {
                    images.push(image.clone());
                    false
                }
                egui::Event::Paste(text) if text.is_empty() => {
                    native_paste = true;
                    false
                }
                _ => true,
            });
            (images, native_paste)
        });
        for image in images {
            self.add_image(image);
        }
        if native_paste {
            self.start_clipboard(ctx);
        }
        // Replay normal text only while this draft owns focus. A response can never
        // paste into another editor or another conversation after navigation.
        if let Some(text) = self.pasted_text.take() {
            ctx.input_mut(|input| input.events.push(egui::Event::Paste(text)));
        }
    }

    /// Native files are attached only when released over the visible chat panel.
    /// Paths are validated/read by the existing attachment service at send time.
    pub(super) fn consume_drop(
        &mut self,
        ctx: &Context,
        bounds: egui::Rect,
        workspace: Option<&Path>,
    ) -> bool {
        let inside = ctx.input(|input| {
            input
                .pointer
                .latest_pos()
                .is_some_and(|pos| bounds.contains(pos))
        });
        if !inside || workspace.is_none() {
            return false;
        }
        let files = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if files.is_empty() {
            return false;
        }
        for file in files.into_iter().take(MAX_ATTACHMENTS + 1) {
            if file.path().is_absolute() {
                self.add_file(file.path().to_path_buf(), false);
            } else {
                self.error = Some(
                    "Este anexo não fornece um arquivo local. Salve-o antes de anexar.".into(),
                );
            }
        }
        ctx.request_repaint();
        true
    }

    pub(super) fn add_image(&mut self, image: Arc<ColorImage>) {
        if self.len() >= MAX_ATTACHMENTS {
            self.error = Some("Anexe no máximo 8 arquivos ou imagens por mensagem.".into());
            return;
        }
        let dimensions = u32::try_from(image.size[0])
            .ok()
            .zip(u32::try_from(image.size[1]).ok());
        let Some((width, height)) = dimensions else {
            self.error = Some("A imagem colada possui dimensões inválidas.".into());
            return;
        };
        let size = match rgba_size(width, height) {
            Ok(size) => size,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        if image.pixels.len() != size / 4 {
            self.error = Some("Os pixels da imagem colada estão incompletos ou inválidos.".into());
            return;
        }
        if size
            > MAX_ATTACHMENT_BYTES
                .saturating_sub(self.rgba_bytes.saturating_add(self.other_rgba_bytes))
        {
            self.error = Some("As imagens dos rascunhos abertos excedem 24 MiB. Remova uma imagem para continuar.".into());
            return;
        }
        // All dimensions/counts and the aggregate budget are checked before allocation.
        let mut rgba = Vec::with_capacity(size);
        for pixel in &image.pixels {
            rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
        }
        let thumbnail = thumbnail(&image);
        self.next_image = self.next_image.wrapping_add(1);
        let name = format!("Imagem colada {}", self.next_image);
        self.entries.push(Entry {
            input: AttachmentInput::ImageRgba {
                width,
                height,
                rgba,
                name: name.clone(),
            },
            label: name,
            thumbnail: Some(thumbnail),
            texture: None,
            image_id: self.next_image,
        });
        self.rgba_bytes += size;
        self.error = None;
    }

    pub(super) fn show(&mut self, ui: &mut Ui) {
        if self.is_empty() {
            return;
        }
        // Keep two complete chips reachable in the narrow composer. Fixed label
        // widths can put the second remove button outside the scroll viewport,
        // while still exposing its (non-clickable) bounds to accessibility.
        let chip_width = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.).max(0.);
        let mut remove = None;
        egui::ScrollArea::horizontal()
            .id_salt(("codisseu.ai.attachment.chips", self.scope))
            .max_height(56.)
            .codisseu()
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (index, entry) in self.entries.iter_mut().enumerate() {
                        egui::Frame::new()
                            .fill(ui.visuals().faint_bg_color)
                            .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
                            .corner_radius(6)
                            .inner_margin(egui::Margin::symmetric(6, 4))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if entry.texture.is_none()
                                        && let Some(thumbnail) = entry.thumbnail.take()
                                    {
                                        entry.texture = Some(ui.ctx().load_texture(
                                            format!("codisseu.ai.attachment.{}", entry.image_id),
                                            thumbnail,
                                            egui::TextureOptions::LINEAR,
                                        ));
                                    }
                                    if let Some(texture) = &entry.texture {
                                        ui.add(
                                            egui::Image::new(texture)
                                                .max_size(egui::vec2(30., 30.)),
                                        );
                                    } else {
                                        let (rect, _) = ui.allocate_exact_size(
                                            egui::vec2(18., 24.),
                                            egui::Sense::hover(),
                                        );
                                        paint_icon(ui, rect, Icon::File, ui.visuals().text_color());
                                    }
                                    let path = match &entry.input {
                                        AttachmentInput::ProjectFile(path)
                                        | AttachmentInput::ExternalFile(path) => {
                                            path.display().to_string()
                                        }
                                        AttachmentInput::ImageRgba { width, height, .. } => {
                                            format!("{width} × {height}")
                                        }
                                    };
                                    let icon_width =
                                        if entry.texture.is_some() { 30. } else { 18. };
                                    let label_width = (chip_width
                                        - icon_width
                                        - 22.
                                        - 12.
                                        - 2. * ui.spacing().item_spacing.x)
                                        .clamp(8., 72.);
                                    ui.add_sized(
                                        [label_width, 22.],
                                        egui::Label::new(&entry.label)
                                            .truncate()
                                            .sense(egui::Sense::hover()),
                                    )
                                    .on_hover_text(path);
                                    if icon_button(
                                        ui,
                                        Icon::Close,
                                        &format!("Remover anexo {}", entry.label),
                                        false,
                                        22.,
                                    )
                                    .clicked()
                                    {
                                        remove = Some(index);
                                    }
                                });
                            });
                    }
                });
            });
        if let Some(index) = remove {
            let removed = self.entries.remove(index);
            if let AttachmentInput::ImageRgba { rgba, .. } = removed.input {
                self.rgba_bytes = self.rgba_bytes.saturating_sub(rgba.len());
            }
            self.error = None;
        }
    }
}

fn thumbnail(image: &ColorImage) -> ColorImage {
    let longest = image.size[0].max(image.size[1]);
    let width = (image.size[0] * longest.min(160) / longest).max(1);
    let height = (image.size[1] * longest.min(160) / longest).max(1);
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        let source_y = y * image.size[1] / height;
        for x in 0..width {
            let source_x = x * image.size[0] / width;
            pixels.push(image.pixels[source_y * image.size[0] + source_x]);
        }
    }
    ColorImage::new([width, height], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct TestDrop(PathBuf);
    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &Path {
            &self.0
        }
        fn bytes(&self) -> Result<Vec<u8>, String> {
            panic!("drop must never read files on the GUI thread")
        }
    }

    #[test]
    fn native_clipboard_delegation_registers_owner_independent_of_prior_focus() {
        let ctx = Context::default();
        let key = egui::Id::unique("egui_winit.defer_clipboard_paste");
        let composer = egui::Id::unique("codisseu.ai.composer");
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("editor")));
        set_native_paste_enabled(&ctx, true);
        assert_eq!(
            ctx.data(|data| data.get_temp::<egui::Id>(key)),
            Some(composer)
        );
        // Focus can change after ai_tick without rewriting the registration.
        ctx.memory_mut(|memory| memory.request_focus(composer));
        assert_eq!(
            ctx.data(|data| data.get_temp::<egui::Id>(key)),
            Some(composer)
        );
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("settings")));
        assert_eq!(
            ctx.data(|data| data.get_temp::<egui::Id>(key)),
            Some(composer)
        );
        set_native_paste_enabled(&ctx, false);
        assert_eq!(ctx.data(|data| data.get_temp::<egui::Id>(key)), None);
    }

    #[test]
    fn drop_consumes_only_files_inside_chat_and_preserves_other_targets() {
        let ctx = Context::default();
        let mut attachments = AttachmentUi::default();
        let bounds = egui::Rect::from_min_max(egui::pos2(100., 10.), egui::pos2(400., 400.));
        let input = egui::RawInput {
            dropped_files: vec![Arc::new(TestDrop("/fixture/image.png".into()))],
            events: vec![egui::Event::PointerMoved(egui::pos2(50., 50.))],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            assert!(!attachments.consume_drop(ui.ctx(), bounds, Some(Path::new("/fixture"))));
            assert_eq!(ui.ctx().input(|input| input.raw.dropped_files.len()), 1);
        });
        output.textures_delta.clear();
        let input = egui::RawInput {
            dropped_files: vec![Arc::new(TestDrop("/fixture/image.png".into()))],
            events: vec![egui::Event::PointerMoved(egui::pos2(150., 50.))],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            assert!(!attachments.consume_drop(ui.ctx(), bounds, None));
            assert!(attachments.consume_drop(ui.ctx(), bounds, Some(Path::new("/fixture"))));
            assert!(ui.ctx().input(|input| input.raw.dropped_files.is_empty()));
            assert!(!attachments.consume_drop(ui.ctx(), bounds, Some(Path::new("/fixture"))));
        });
        output.textures_delta.clear();
        assert!(
            matches!(&attachments.inputs()[0], AttachmentInput::ExternalFile(path) if path == Path::new("/fixture/image.png"))
        );
    }

    #[test]
    fn clipboard_result_stays_in_its_draft_and_plain_path_remains_text() {
        let ctx = Context::default();
        let mut attachments = AttachmentUi::default();
        let (send, receive) = mpsc::sync_channel(1);
        attachments.clipboard = Some(PendingClipboard {
            generation: 0,
            receive,
        });
        send.send(ClipboardContent::Text("/fixture/arquivo.php".into()))
            .ok();
        assert!(attachments.poll(&ctx, Some(Path::new("/fixture"))));
        assert!(attachments.is_empty());
        assert!(attachments.has_pending_input());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.ctx().memory_mut(|memory| memory.request_focus(egui::Id::unique("editor")));
            attachments.consume_paste(ui.ctx());
            assert!(ui.ctx().input(|input| input.events.is_empty()));
            ui.ctx().memory_mut(|memory| memory.request_focus(egui::Id::unique("codisseu.ai.composer")));
            attachments.consume_paste(ui.ctx());
            assert!(ui.ctx().input(|input| matches!(&input.events[..], [egui::Event::Paste(text)] if text == "/fixture/arquivo.php")));
        });
        output.textures_delta.clear();
        assert!(!attachments.has_pending_input());
        let (send, receive) = mpsc::sync_channel(1);
        attachments.clipboard = Some(PendingClipboard {
            generation: 0,
            receive,
        });
        attachments.clear();
        assert!(
            send.send(ClipboardContent::Files(vec!["/fixture/stale.txt".into()]))
                .is_err()
        );
        assert!(!attachments.poll(&ctx, Some(Path::new("/fixture"))));
        assert!(attachments.is_empty());
    }

    #[test]
    #[ignore = "requires a dedicated native clipboard/display; never run against the personal clipboard"]
    fn native_clipboard_files_image_and_text_use_real_mime_data() {
        assert_eq!(
            std::env::var("CODISSEU_TEST_CLIPBOARD").as_deref(),
            Ok("isolated")
        );
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("ação 🦀.txt");
        std::fs::write(&path, "fixture").unwrap();
        let mut owner = arboard::Clipboard::new().unwrap();
        owner.set().file_list(&[&path]).unwrap();
        assert!(
            matches!(read_clipboard(), ClipboardContent::Files(paths) if paths == [path.clone()])
        );
        owner
            .set_image(arboard::ImageData {
                width: 2,
                height: 1,
                bytes: std::borrow::Cow::Borrowed(&[255, 0, 0, 255, 0, 255, 0, 255]),
            })
            .unwrap();
        assert!(
            matches!(read_clipboard(), ClipboardContent::Image(image) if image.size == [2,1] && image.pixels[0] == egui::Color32::RED)
        );
        owner.set_text(path.display().to_string()).unwrap();
        assert!(
            matches!(read_clipboard(), ClipboardContent::Text(text) if text == path.display().to_string())
        );
    }

    #[test]
    fn image_preflight_rejects_malformed_pixels_and_counts_before_conversion() {
        let mut attachments = AttachmentUi::default();
        let malformed = ColorImage {
            size: [8192, 8192],
            source_size: egui::Vec2::ZERO,
            pixels: vec![],
        };
        attachments.add_image(Arc::new(malformed));
        assert!(attachments.is_empty());
        assert!(attachments.error.is_some());
        attachments.add_image(Arc::new(ColorImage {
            size: [2, 2],
            source_size: egui::Vec2::ZERO,
            pixels: vec![],
        }));
        assert!(attachments.is_empty());
        assert!(attachments.error.as_ref().unwrap().contains("pixels"));
        for _ in 0..MAX_ATTACHMENTS {
            attachments.add_image(Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)));
        }
        attachments.add_image(Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)));
        assert_eq!(attachments.len(), MAX_ATTACHMENTS);
        assert_eq!(attachments.rgba_bytes, MAX_ATTACHMENTS * 4);
        assert!(attachments.error.is_some());
        attachments.clear();
        assert!(attachments.inputs().is_empty());
        assert_eq!(attachments.rgba_bytes, 0);
    }

    #[test]
    fn rgba_and_small_thumbnail_have_correct_dimensions_without_upscaling() {
        let mut attachments = AttachmentUi::default();
        attachments.add_image(Arc::new(ColorImage::filled(
            [1000, 500],
            egui::Color32::RED,
        )));
        let thumb = attachments.entries[0].thumbnail.as_ref().unwrap();
        assert_eq!(thumb.size, [160, 80]);
        assert_eq!(thumb.pixels.len(), 12800);
        assert!(
            matches!(&attachments.inputs()[0], AttachmentInput::ImageRgba {width:1000,height:500,rgba,..} if rgba.len()==2_000_000 && rgba[..4]==[255,0,0,255])
        );
        let tiny = thumbnail(&ColorImage::filled([2, 1], egui::Color32::GREEN));
        assert_eq!(tiny.size, [2, 1]);
    }

    #[test]
    fn paste_only_consumes_images_when_composer_is_focused() {
        let ctx = Context::default();
        let mut attachments = AttachmentUi::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let ctx = ui.ctx();
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("editor")));
            ctx.input_mut(|input| {
                input.events.push(egui::Event::PasteImage(Arc::new(ColorImage::filled([2,1],egui::Color32::RED))));
                input.events.push(egui::Event::Paste("texto preservado".into()));
            });
            attachments.consume_paste(ctx);
            assert!(attachments.is_empty());
            assert_eq!(ctx.input(|input| input.events.len()), 2);
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique("codisseu.ai.composer")));
            attachments.consume_paste(ctx);
            assert_eq!(attachments.len(), 1);
            assert!(ctx.input(|input| matches!(&input.events[..], [egui::Event::Paste(text)] if text=="texto preservado")));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn picker_result_is_scoped_to_workspace_and_generation() {
        let ctx = Context::default();
        let mut attachments = AttachmentUi::default();
        let origin = PathBuf::from("/fixture/origin");
        let (send, receive) = mpsc::sync_channel(1);
        attachments.pending = Some(PendingPicker {
            origin: origin.clone(),
            generation: 0,
            receive,
        });
        send.send(Some(vec![PathBuf::from("/fixture/selected.txt")]))
            .unwrap();
        assert!(!attachments.poll(&ctx, Some(Path::new("/fixture/other"))));
        assert!(attachments.is_empty());
        let (send, receive) = mpsc::sync_channel(1);
        attachments.pending = Some(PendingPicker {
            origin: origin.clone(),
            generation: attachments.generation,
            receive,
        });
        attachments.clear();
        send.send(Some(vec![PathBuf::from("/fixture/selected.txt")]))
            .unwrap();
        assert!(!attachments.poll(&ctx, Some(&origin)));
        assert!(attachments.is_empty());
        let (send, receive) = mpsc::sync_channel(1);
        attachments.pending = Some(PendingPicker {
            origin: origin.clone(),
            generation: attachments.generation,
            receive,
        });
        send.send(Some(vec![PathBuf::from("/fixture/ação.txt")]))
            .unwrap();
        assert!(attachments.poll(&ctx, Some(&origin)));
        assert!(
            matches!(&attachments.inputs()[0],AttachmentInput::ExternalFile(path) if path==Path::new("/fixture/ação.txt"))
        );
    }

    #[test]
    fn project_mentions_are_unique_and_pending_rgba_has_aggregate_budget() {
        let mut attachments = AttachmentUi::default();
        attachments.add_project_file("ação 🦀.ts".into());
        attachments.add_project_file("ação 🦀.ts".into());
        assert_eq!(attachments.len(), 1);
        assert!(
            matches!(&attachments.inputs()[0],AttachmentInput::ProjectFile(path) if path==Path::new("ação 🦀.ts"))
        );
        attachments.rgba_bytes = MAX_ATTACHMENT_BYTES - 3;
        attachments.add_image(Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)));
        assert_eq!(attachments.len(), 1);
        assert!(attachments.error.as_ref().unwrap().contains("24 MiB"));
    }

    #[test]
    fn other_conversations_reserve_the_shared_clipboard_pixel_budget() {
        let mut attachments = AttachmentUi::default();
        attachments.reserve_other_rgba(MAX_ATTACHMENT_BYTES - 3);
        attachments.add_image(Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)));
        assert!(attachments.is_empty());
        assert!(attachments.error.as_ref().unwrap().contains("rascunhos"));
        attachments.reserve_other_rgba(0);
        attachments.add_image(Arc::new(ColorImage::filled([1, 1], egui::Color32::RED)));
        assert_eq!(attachments.len(), 1);
    }

    #[test]
    fn thumbnail_texture_stays_small_and_remove_button_releases_pending_pixels() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut attachments = AttachmentUi::default();
        attachments.add_image(Arc::new(ColorImage::filled(
            [1000, 500],
            egui::Color32::RED,
        )));
        let mut harness = Harness::builder().with_size([400., 120.]).build_ui_state(
            |ui, attachments: &mut AttachmentUi| attachments.show(ui),
            attachments,
        );
        assert_eq!(
            harness.state().entries[0].texture.as_ref().unwrap().size(),
            [160, 80]
        );
        harness
            .get_by_label("Remover anexo Imagem colada 1")
            .click();
        harness.step();
        assert!(harness.state().is_empty());
        assert_eq!(harness.state().rgba_bytes, 0);
    }

    #[test]
    fn narrow_composer_keeps_file_and_full_thumbnail_remove_actions_inside_viewport() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut attachments = AttachmentUi::default();
        attachments.add_project_file("long-fixture-name.php".into());
        attachments.add_image(Arc::new(ColorImage::filled([80, 80], egui::Color32::RED)));
        let viewport_id = egui::Id::unique("codisseu.ai.test.attachment.viewport");
        let mut harness = Harness::builder().with_size([244., 100.]).build_ui_state(
            move |ui, attachments: &mut AttachmentUi| {
                let viewport = ui.available_rect_before_wrap();
                ui.ctx()
                    .data_mut(|data| data.insert_temp(viewport_id, viewport));
                attachments.show(ui);
            },
            attachments,
        );
        let viewport = harness
            .ctx
            .data(|data| data.get_temp::<egui::Rect>(viewport_id))
            .unwrap();
        for label in [
            "Remover anexo long-fixture-name.php",
            "Remover anexo Imagem colada 1",
        ] {
            let rect = harness.get_by_label(label).rect();
            assert!(
                viewport.contains_rect(rect),
                "Remove action outside composer viewport: {rect:?}, {viewport:?}"
            );
        }
        harness
            .get_by_label("Remover anexo Imagem colada 1")
            .click();
        harness.step();
        assert_eq!(harness.state().len(), 1);
        assert_eq!(harness.state().rgba_bytes, 0);
        assert!(
            matches!(&harness.state().inputs()[0], AttachmentInput::ProjectFile(path) if path == Path::new("long-fixture-name.php"))
        );
    }
}
