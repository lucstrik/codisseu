//! Bounded, optional LCD runs for signature help; portable shaping remains caller-owned.
use egui::{Color32, Pos2, Rect, TextureHandle, TextureOptions, Ui, pos2, vec2};
use std::collections::VecDeque;

const MAX_PIXELS: usize = 65_536;
const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub(super) struct Run {
    pub text: String,
    pub color: Color32,
    pub bold: bool,
    pub clip: bool,
    /// Logical spacing before/after the run, independent of font space width.
    pub padding: [f32; 2],
}
#[derive(Clone, PartialEq, Eq)]
struct RasterRun {
    text: String,
    color: [u8; 4],
    bold: bool,
    clip: bool,
    padding: [i32; 2],
}
#[derive(Clone, PartialEq, Eq)]
struct MeasureKey {
    runs: Vec<RasterRun>,
    font_size: i32,
    scale: u32,
}
#[derive(PartialEq, Eq)]
struct Key {
    measure: MeasureKey,
    size: [usize; 2],
    background: [u8; 4],
}
struct Entry {
    key: Key,
    texture: TextureHandle,
    bytes: usize,
}
pub(super) struct SignaturePaint {
    engine: Option<Engine>,
    entries: VecDeque<Entry>,
    measures: VecDeque<(MeasureKey, i32)>,
    bytes: usize,
}
impl Default for SignaturePaint {
    fn default() -> Self {
        Self {
            engine: Engine::new(),
            entries: VecDeque::new(),
            measures: VecDeque::new(),
            bytes: 0,
        }
    }
}
fn measure_key(runs: &[Run], size: f32, scale: f32) -> Option<MeasureKey> {
    if runs.is_empty()
        || runs.len() > 32
        || !(6. ..=24.).contains(&size)
        || !(0.75..=3.).contains(&scale)
        || runs
            .iter()
            .map(|run| run.text.chars().count())
            .sum::<usize>()
            > 128
        || runs.iter().all(|run| run.text.is_empty())
    {
        return None;
    }
    let mut raster = Vec::with_capacity(runs.len());
    for run in runs {
        if !run
            .text
            .chars()
            .all(|c| c.is_ascii_graphic() || c == ' ' || c == '\u{2009}')
            || run
                .padding
                .iter()
                .any(|p| !p.is_finite() || !(0. ..=64.).contains(p))
        {
            return None;
        }
        raster.push(RasterRun {
            text: run.text.clone(),
            color: run.color.to_array(),
            bold: run.bold,
            clip: run.clip,
            padding: run.padding.map(|p| (p * scale).round() as i32),
        });
    }
    Some(MeasureKey {
        runs: raster,
        font_size: (size * scale * 64.).round() as i32,
        scale: scale.to_bits(),
    })
}
impl SignaturePaint {
    fn measured_pixels(&mut self, key: &MeasureKey) -> Option<i32> {
        if let Some(index) = self.measures.iter().position(|(old, _)| old == key) {
            let entry = self.measures.remove(index)?;
            let width = entry.1;
            self.measures.push_front(entry);
            return Some(width);
        }
        let width = self.engine.as_mut()?.measure(key)?;
        if self.measures.len() >= MAX_ENTRIES {
            self.measures.pop_back();
        }
        self.measures.push_front((key.clone(), width));
        Some(width)
    }
    /// Logical advance, including run padding; None requires portable shaping.
    pub fn measure(&mut self, runs: &[Run], size: f32, scale: f32) -> Option<f32> {
        self.engine.as_ref()?;
        let key = measure_key(runs, size, scale)?;
        self.measured_pixels(&key).map(|width| width as f32 / scale)
    }
    /// `baseline` is the FreeType cursor baseline in logical points, without hidden offsets.
    /// Returns false before painting if the caller must use portable text rendering.
    pub fn paint(
        &mut self,
        ui: &Ui,
        runs: &[Run],
        baseline: Pos2,
        max_width: f32,
        background: Color32,
        size: f32,
    ) -> bool {
        let scale = ui.ctx().pixels_per_point();
        let Some(measure) = measure_key(runs, size, scale) else {
            return false;
        };
        if self.engine.is_none()
            || !baseline.is_finite()
            || !max_width.is_finite()
            || max_width <= 0.
            || background.a() != 255
            || runs.iter().any(|run| run.color.a() != 255)
            || ui.painter().opacity() != 1.
            || !ui.is_enabled()
            || ui
                .ctx()
                .layer_transform_to_global(ui.layer_id())
                .is_some_and(|t| t != egui::emath::TSTransform::IDENTITY)
        {
            return false;
        }
        let Some(advance) = self.measured_pixels(&measure) else {
            return false;
        };
        if advance as f32 > max_width * scale + 0.01 {
            return false;
        }
        let width = advance as usize + 4;
        let height = (26. * scale).ceil() as usize;
        if width > 4096 || width * height > MAX_PIXELS {
            return false;
        }
        let ascent = (20. * scale).ceil() as i32;
        let rect = Rect::from_min_size(
            pos2(
                ((baseline.x * scale).round() - 2.) / scale,
                ((baseline.y * scale).round() - ascent as f32) / scale,
            ),
            vec2(width as f32 / scale, height as f32 / scale),
        );
        if !ui.is_rect_visible(rect) {
            return true;
        }
        let key = Key {
            measure,
            size: [width, height],
            background: background.to_array(),
        };
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            let entry = self
                .entries
                .remove(index)
                .expect("existing signature cache entry");
            self.entries.push_front(entry);
        } else {
            let Some(rgb) = self
                .engine
                .as_mut()
                .and_then(|engine| engine.render(&key, ascent))
            else {
                return false;
            };
            let pixels = rgb
                .chunks_exact(3)
                .map(|c| Color32::from_rgb(c[0], c[1], c[2]))
                .collect();
            let bytes = width * height * 4;
            while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
                let Some(old) = self.entries.pop_back() else {
                    break;
                };
                self.bytes -= old.bytes;
            }
            let texture = ui.ctx().load_texture(
                "codisseu.signature.lcd",
                egui::ColorImage::new(key.size, pixels),
                TextureOptions::NEAREST,
            );
            self.entries.push_front(Entry {
                key,
                texture,
                bytes,
            });
            self.bytes += bytes;
        }
        ui.painter().image(
            self.entries.front().expect("signature raster").texture.id(),
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
    fn codisseu_signature_face_measure(
        face: *mut std::ffi::c_void,
        text: *const u32,
        count: usize,
        size: i32,
        bold: i32,
        advance: *mut i32,
    ) -> i32;
    fn codisseu_signature_face_render(
        face: *mut std::ffi::c_void,
        text: *const u32,
        count: usize,
        size: i32,
        bold: i32,
        width: i32,
        height: i32,
        cursor: i32,
        baseline: i32,
        foreground: *const u8,
        rgb: *mut u8,
        len: usize,
        clip_right: i32,
    ) -> i32;
}
impl Engine {
    fn new() -> Option<Self> {
        #[cfg(codisseu_tab_lcd)]
        {
            static FONT: &[u8] =
                include_bytes!("../../../reference/public/fonts/InterVariable.ttf");
            // SAFETY: immutable public font bytes outlive this exclusively owned face.
            std::ptr::NonNull::new(unsafe { codisseu_tab_face_new(FONT.as_ptr(), FONT.len()) })
                .map(Self)
        }
        #[cfg(not(codisseu_tab_lcd))]
        None
    }
    #[cfg(codisseu_tab_lcd)]
    fn run_advance(&mut self, run: &RasterRun, size: i32) -> Option<i32> {
        let text: Vec<u32> = run.text.chars().map(u32::from).collect();
        let mut advance = 0;
        // SAFETY: face is exclusively borrowed; bounded input and a valid output scalar.
        let ok = unsafe {
            codisseu_signature_face_measure(
                self.0.as_ptr(),
                text.as_ptr(),
                text.len(),
                size,
                i32::from(run.bold),
                &mut advance,
            )
        };
        (ok == 1).then_some(advance)
    }
    fn measure(&mut self, key: &MeasureKey) -> Option<i32> {
        #[cfg(codisseu_tab_lcd)]
        {
            let mut width = 0i32;
            for run in &key.runs {
                width = width
                    .checked_add(self.run_advance(run, key.font_size)?)?
                    .checked_add(run.padding[0])?
                    .checked_add(run.padding[1])?;
                if width > 4092 {
                    return None;
                }
            }
            Some(width)
        }
        #[cfg(not(codisseu_tab_lcd))]
        {
            let _ = key;
            None
        }
    }
    fn render(&mut self, key: &Key, baseline: i32) -> Option<Vec<u8>> {
        if key.size[0] == 0
            || key.size[0] > 4096
            || key.size[1] == 0
            || key.size[1] > 256
            || key.size[0] * key.size[1] > MAX_PIXELS
        {
            return None;
        }
        #[cfg(codisseu_tab_lcd)]
        {
            let mut rgb = key.background[..3].repeat(key.size[0] * key.size[1]);
            let mut cursor = 2;
            for run in &key.measure.runs {
                cursor += run.padding[0];
                let advance = self.run_advance(run, key.measure.font_size)?;
                let text: Vec<u32> = run.text.chars().map(u32::from).collect();
                // SAFETY: exclusively owned face; all slices and dimensions validated by C.
                let ok = unsafe {
                    codisseu_signature_face_render(
                        self.0.as_ptr(),
                        text.as_ptr(),
                        text.len(),
                        key.measure.font_size,
                        i32::from(run.bold),
                        key.size[0] as i32,
                        key.size[1] as i32,
                        cursor,
                        baseline,
                        run.color.as_ptr(),
                        rgb.as_mut_ptr(),
                        rgb.len(),
                        if run.clip {
                            cursor + advance + run.padding[1]
                        } else {
                            key.size[0] as i32
                        },
                    )
                };
                if ok != 1 {
                    return None;
                }
                cursor += advance + run.padding[1];
            }
            Some(rgb)
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
        // SAFETY: this face is owned by this engine and no C call retains it.
        unsafe { codisseu_tab_face_drop(self.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(text: &str, bold: bool) -> Run {
        Run {
            text: text.into(),
            color: Color32::from_rgb(188, 190, 196),
            bold,
            clip: false,
            padding: [0., 0.],
        }
    }
    #[test]
    fn rejects_shaping_text_and_unbounded_signature_runs() {
        assert!(measure_key(&[run("name: string", false)], 13., 1.).is_some());
        assert!(measure_key(&[run("\u{2009}", false)], 13., 1.).is_some());
        for value in ["👩‍💻", "e\u{301}", "Arabic العربية", &"x".repeat(129)] {
            assert!(measure_key(&[run(value, false)], 13., 1.).is_none());
        }
        let mut padded = run("name", false);
        padded.padding[0] = f32::NAN;
        assert!(measure_key(&[padded], 13., 1.).is_none());
    }
    #[test]
    fn weight_changes_do_not_leak_and_spacing_is_measured() {
        let mut painter = SignaturePaint::default();
        #[cfg(codisseu_tab_lcd)]
        {
            let regular = painter
                .measure(&[run("count: number", false)], 13., 1.)
                .expect("public font regular");
            let bold = painter
                .measure(&[run("count: number", true)], 13., 1.)
                .expect("public font bold");
            assert_eq!(bold, regular, "synthetic bold preserves regular advances");
            assert_eq!(
                painter.measure(&[run("count: number", false)], 13., 1.),
                Some(regular)
            );
            let mut padded = run("count: number", false);
            padded.padding = [3., 3.];
            assert_eq!(painter.measure(&[padded], 13., 1.), Some(regular + 6.));
            for index in 0..100 {
                assert!(
                    painter
                        .measure(&[run(&format!("value{index}"), index % 2 == 0)], 13., 1.)
                        .is_some()
                );
            }
            assert_eq!(painter.measures.len(), MAX_ENTRIES);
        }
        #[cfg(not(codisseu_tab_lcd))]
        assert!(
            painter
                .measure(&[run("count: number", false)], 13., 1.)
                .is_none()
        );
    }

    #[test]
    fn textures_are_bounded_and_disabled_engine_never_reuses_stale_pixels() {
        let mut painter = SignaturePaint::default();
        let supported = painter.engine.is_some();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |root| {
            egui::CentralPanel::default().show(root, |ui| {
                for index in 0..70 {
                    let runs = [run(&format!("name{index}: string"), index % 2 == 0)];
                    assert_eq!(
                        painter.paint(ui, &runs, pos2(30., 60.), 180., Color32::BLACK, 13.),
                        supported
                    );
                    assert!(painter.entries.len() <= MAX_ENTRIES && painter.bytes <= MAX_BYTES);
                }
                assert!(!painter.paint(
                    ui,
                    &[run("unbounded text", false)],
                    pos2(30., 60.),
                    1.,
                    Color32::BLACK,
                    13.
                ));
                let before = painter.entries.len();
                painter.engine = None;
                assert!(!painter.paint(
                    ui,
                    &[run("name69: string", true)],
                    pos2(30., 60.),
                    180.,
                    Color32::BLACK,
                    13.
                ));
                assert!(
                    painter
                        .measure(&[run("name69: string", true)], 13., 1.)
                        .is_none()
                );
                assert_eq!(painter.entries.len(), before);
            });
        });
        output.textures_delta.clear();
        if supported {
            assert_eq!(painter.entries.len(), MAX_ENTRIES);
        }
    }
}
