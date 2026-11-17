//! Bounded LCD tab labels. Complex text and unsupported hosts keep egui shaping.
use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Ui, pos2, vec2};
use std::collections::VecDeque;

const MAX_PIXELS: usize = 65_536;
const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) struct LabelStyle {
    pub foreground: Color32,
    pub background: Color32,
    pub opacity: f32,
    pub font_size: f32,
}
#[derive(PartialEq, Eq)]
struct Key {
    text: String,
    size: [usize; 2],
    font_size: i32,
    scale: u32,
    cursor: i32,
    colors: [[u8; 4]; 2],
    opacity: u8,
}
struct Entry {
    key: Key,
    texture: TextureHandle,
    bytes: usize,
}
pub(super) struct TabLabels {
    engine: Option<Engine>,
    entries: VecDeque<Entry>,
    bytes: usize,
    limits: (usize, usize),
}
impl Default for TabLabels {
    fn default() -> Self {
        Self {
            engine: Engine::new(),
            entries: VecDeque::new(),
            bytes: 0,
            limits: (MAX_ENTRIES, MAX_BYTES),
        }
    }
}
fn simple_label(text: &str) -> bool {
    !text.is_empty()
        && text.chars().take(129).count() <= 128
        && text
            .chars()
            .all(|c| c.is_ascii_graphic() || c == ' ' || c == '•')
}
/// Blend text intensity before rasterization; icon alpha is a separate token.
pub(super) fn dimmed_color(background: Color32, foreground: Color32, amount: f32) -> Color32 {
    let bg = background.to_array();
    let fg = foreground.to_array();
    let rgb: [u8; 3] = std::array::from_fn(|i| {
        let b = f64::from(bg[i]);
        let f = f64::from(fg[i]);
        (b * b + (f * f - b * b) * f64::from(amount.clamp(0., 1.)))
            .sqrt()
            .round() as u8
    });
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
impl TabLabels {
    #[cfg(test)]
    pub(super) fn has_cached_label(&self, text: &str) -> bool {
        self.entries.iter().any(|entry| entry.key.text == text)
    }
    /// Git lists have more visible labels than a tab strip; still bounded per app.
    pub fn for_git() -> Self {
        Self {
            limits: (192, 4 * MAX_BYTES),
            ..Self::default()
        }
    }
    /// Returns false before painting whenever the portable renderer is required.
    pub fn paint(
        &mut self,
        ui: &Ui,
        text: &str,
        anchor: Pos2,
        max_width: f32,
        style: LabelStyle,
    ) -> bool {
        let scale = ui.ctx().pixels_per_point();
        if self.engine.is_none()
            || !simple_label(text)
            || !(0.75..=3.).contains(&scale)
            || !(6. ..=24.).contains(&style.font_size)
            || !anchor.is_finite()
            || !max_width.is_finite()
            || max_width <= 0.
            || ui.painter().opacity() != 1.
            || ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .is_some_and(|transform| transform != egui::emath::TSTransform::IDENTITY)
            || !ui.is_enabled()
            || style.foreground.a() != 255
            || style.background.a() != 255
            || !style.opacity.is_finite()
        {
            return false;
        }
        let width = (max_width * scale).ceil().min(4094.) as usize + 2;
        let height = (26. * scale).ceil() as usize;
        if width * height > MAX_PIXELS {
            return false;
        }
        let left = (anchor.x * scale).round() - 2.;
        let baseline = ((anchor.y + 5.) * scale).round();
        let ascent = (20. * scale).ceil();
        let rect = Rect::from_min_size(
            pos2(left / scale, (baseline - ascent) / scale),
            vec2(width as f32 / scale, height as f32 / scale),
        );
        if !ui.is_rect_visible(rect) {
            return true;
        }
        let key = Key {
            text: text.to_owned(),
            size: [width, height],
            font_size: (style.font_size * scale * 64.).round() as i32,
            scale: scale.to_bits(),
            // FreeType's LCD bearing includes its filter fringe. The egui
            // anchor reserves two more points than the observed ink origin.
            cursor: ((anchor.x - 1.) * scale).round() as i32 - left as i32,
            colors: [style.foreground.to_array(), style.background.to_array()],
            opacity: (style.opacity.clamp(0., 1.) * 255.).round() as u8,
        };
        if let Some(index) = self.entries.iter().position(|e| e.key == key) {
            let entry = self.entries.remove(index).expect("existing cache entry");
            self.entries.push_front(entry);
        } else {
            let Some(rgb) = self
                .engine
                .as_mut()
                .and_then(|engine| engine.render(&key, ascent as i32))
            else {
                return false;
            };
            let pixels = rgb
                .chunks_exact(3)
                .map(|rgb| Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                .collect();
            let bytes = width * height * 4;
            while self.entries.len() >= self.limits.0 || self.bytes + bytes > self.limits.1 {
                let Some(old) = self.entries.pop_back() else {
                    break;
                };
                self.bytes -= old.bytes;
            }
            let image = egui::ColorImage::new(key.size, pixels);
            let texture =
                ui.ctx()
                    .load_texture("codisseu.tab.label.lcd", image, TextureOptions::NEAREST);
            self.entries.push_front(Entry {
                key,
                texture,
                bytes,
            });
            self.bytes += bytes;
        }
        ui.painter().image(
            self.entries.front().expect("painted entry").texture.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
            Color32::WHITE,
        );
        true
    }
}

#[cfg(codisseu_tab_lcd)]
struct Engine(std::ptr::NonNull<std::ffi::c_void>);
#[cfg(not(codisseu_tab_lcd))]
struct Engine;
#[cfg(codisseu_tab_lcd)]
unsafe extern "C" {
    fn codisseu_tab_face_new(font: *const u8, len: usize) -> *mut std::ffi::c_void;
    fn codisseu_tab_face_drop(face: *mut std::ffi::c_void);
    fn codisseu_tab_face_render(
        face: *mut std::ffi::c_void,
        text: *const u32,
        count: usize,
        size: i32,
        width: i32,
        height: i32,
        cursor: i32,
        baseline: i32,
        foreground: *const u8,
        opacity: u32,
        rgb: *mut u8,
        len: usize,
    ) -> i32;
}
impl Engine {
    fn new() -> Option<Self> {
        #[cfg(codisseu_tab_lcd)]
        {
            static FONT: &[u8] =
                include_bytes!("../../../reference/public/fonts/InterVariable.ttf");
            // SAFETY: the face borrows immutable static bytes until Drop.
            std::ptr::NonNull::new(unsafe { codisseu_tab_face_new(FONT.as_ptr(), FONT.len()) })
                .map(Self)
        }
        #[cfg(not(codisseu_tab_lcd))]
        None
    }
    fn render(&mut self, key: &Key, baseline: i32) -> Option<Vec<u8>> {
        if key.size[0] == 0
            || key.size[0] > 4096
            || key.size[1] == 0
            || key.size[1] > 256
            || key.size[0] * key.size[1] > MAX_PIXELS
            || !simple_label(&key.text)
        {
            return None;
        }
        #[cfg(codisseu_tab_lcd)]
        {
            let text: Vec<u32> = key.text.chars().map(u32::from).collect();
            let mut rgb = key.colors[1][..3].repeat(key.size[0] * key.size[1]);
            // SAFETY: an exclusively borrowed live face, bounded buffers and lengths;
            // the C adapter validates dimensions and never retains these pointers.
            let ok = unsafe {
                codisseu_tab_face_render(
                    self.0.as_ptr(),
                    text.as_ptr(),
                    text.len(),
                    key.font_size,
                    key.size[0] as i32,
                    key.size[1] as i32,
                    key.cursor,
                    baseline,
                    key.colors[0].as_ptr(),
                    u32::from(key.opacity),
                    rgb.as_mut_ptr(),
                    rgb.len(),
                )
            };
            (ok == 1).then_some(rgb)
        }
        #[cfg(not(codisseu_tab_lcd))]
        {
            let _ = (key, baseline);
            None
        }
    }
}
#[cfg(codisseu_tab_lcd)]
impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: this uniquely owned handle is released exactly once.
        unsafe { codisseu_tab_face_drop(self.0.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shaping_and_budget_inputs_keep_portable_fallback() {
        assert!(simple_label("editor-style.php •"));
        for text in ["", "café.php", "e\u{301}.ts", "مرحبا.ts", "🙂.ts", "a\nb"] {
            assert!(!simple_label(text));
        }
        assert!(!simple_label(&"a".repeat(129)));
    }
    #[test]
    fn native_raster_is_bounded_and_has_independent_channels() {
        let engine = Engine::new();
        if cfg!(codisseu_tab_lcd) {
            assert!(engine.is_some(), "FreeType LCD initialization failed");
        }
        let Some(mut engine) = engine else {
            return;
        };
        let mut key = Key {
            text: "editor-views.ts".into(),
            size: [103, 26],
            font_size: 13 * 64,
            scale: 1f32.to_bits(),
            cursor: 3,
            colors: [[223, 225, 229, 255], [30, 31, 34, 255]],
            opacity: 255,
        };
        let mask = engine.render(&key, 20).unwrap();
        assert_eq!(mask.len(), 103 * 26 * 3);
        assert!(mask.chunks_exact(3).any(|c| c[0] < 40 && c[2] > 65));
        key.opacity = 0;
        assert!(
            engine
                .render(&key, 20)
                .unwrap()
                .chunks_exact(3)
                .all(|c| c == [30, 31, 34])
        );
        key.opacity = 255;
        key.size[0] = 20;
        assert!(
            engine.render(&key, 20).is_none(),
            "must not paint into close target"
        );
        key.size[0] = 180;
        key.text = "editor-style.php •".into();
        assert!(
            engine.render(&key, 20).is_some(),
            "dirty suffix is real text"
        );
        key.size = [4096, 256];
        assert!(
            engine.render(&key, 20).is_none(),
            "reject oversized native buffer"
        );
    }
    #[test]
    fn cache_evicts_textures_and_rejects_complex_or_overlapping_labels() {
        let mut labels = TabLabels::default();
        let supported = labels.engine.is_some();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                let style = LabelStyle {
                    foreground: Color32::WHITE,
                    background: Color32::BLACK,
                    opacity: 1.,
                    font_size: 13.,
                };
                let anchor = pos2(20., 40.);
                for i in 0..70 {
                    assert_eq!(
                        labels.paint(ui, &format!("document-{i}.ts"), anchor, 160., style),
                        supported
                    );
                    assert!(labels.entries.len() <= MAX_ENTRIES && labels.bytes <= MAX_BYTES);
                }
                assert!(!labels.paint(ui, "café.php", anchor, 160., style));
                assert!(!labels.paint(ui, "long-name.ts", anchor, 5., style));
                let cache_size = labels.entries.len();
                labels.engine = None;
                assert!(!labels.paint(ui, "file.ts", anchor, 160., style));
                assert_eq!(
                    labels.entries.len(),
                    cache_size,
                    "no stale cache painted after engine failure"
                );
            });
        });
        output.textures_delta.clear();
        if supported {
            assert_eq!(labels.entries.len(), MAX_ENTRIES);
        }
    }
}
