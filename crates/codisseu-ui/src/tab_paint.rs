//! Procedural coverage of tiny tab primitives, emitted as bounded egui meshes.
//! No image assets or reference pixels; all coverage comes from geometric paths.
use egui::{Color32, Mesh, Rect, Shape, Ui, Vec2, pos2, vec2};
use std::collections::VecDeque;

const MAX_ENTRIES: usize = 32;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Pill,
    Close,
    TypeScript,
    Php,
    Feedback,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    kind: Kind,
    scale: u32,
    size: [u32; 2],
    phase: [u32; 2],
    colors: [[u8; 3]; 3],
    params: [u32; 4],
    hover: bool,
}
#[derive(Clone, Copy)]
pub(super) struct CloseStyle {
    pub foreground: Color32,
    pub background: Color32,
    pub hover_fill: Color32,
    pub hovered: bool,
    pub padding: Vec2,
    pub stroke: f32,
    pub hover_radius: f32,
}
struct Spec {
    kind: Kind,
    colors: [Color32; 3],
    params: [f32; 4],
    hover: bool,
}
#[derive(Default)]
pub(super) struct TabPaint {
    cache: VecDeque<(Key, Mesh, usize)>,
    bytes: usize,
}
fn rgb(c: Color32) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}
fn blend(bg: [u8; 3], fg: [u8; 3], alpha: f32) -> [u8; 3] {
    std::array::from_fn(|i| ((1. - alpha) * bg[i] as f32 + alpha * fg[i] as f32).round() as u8)
}
/// Integer source-over adapted to the measured tab-edge coverage.
pub(super) fn blend_premult(bg: [u8; 3], fg: [u8; 3], coverage: f32) -> [u8; 3] {
    let a = (coverage.clamp(0., 1.) * 255.).round() as u32;
    std::array::from_fn(|i| {
        (((fg[i] as u32 * a + 127) / 255) + ((bg[i] as u32 * (255 - a) + 127) / 255)).min(255) as u8
    })
}
/// Fractional pixel intersection with an ellipse-ended horizontal bar.
fn pill_coverage(x: f32, y: f32, size: Vec2, phase: Vec2, radius: Vec2, inset: f32) -> f32 {
    let ry = radius.y;
    if x >= phase.x + radius.x && x + 1. <= phase.x + size.x - radius.x {
        return ((y + 1.).min(phase.y + size.y) - y.max(phase.y)).clamp(0., 1.);
    }
    let mut coverage = 0.;
    for sample in 0..128 {
        let yy = y + (sample as f32 + 0.5) / 128. - phase.y;
        if !(0. ..size.y).contains(&yy) {
            continue;
        }
        let inset =
            inset + radius.x * (1. - (1. - ((yy - size.y * 0.5) / ry).powi(2)).max(0.).sqrt());
        let left = phase.x + inset;
        let right = phase.x + size.x - inset;
        coverage += (right.min(x + 1.) - left.max(x)).clamp(0., 1.);
    }
    coverage / 128.
}
fn segment_interval(y: f32, padding: Vec2, radius: f32) -> Option<(f32, f32)> {
    let start = padding;
    let end = Vec2::splat(16.) - padding;
    let delta = end - start;
    let normal = vec2(delta.y, -delta.x) * (radius / delta.length());
    let vertices = [start + normal, end + normal, end - normal, start - normal];
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for i in 0..4 {
        let a = vertices[i];
        let b = vertices[(i + 1) % 4];
        if (a.y.min(b.y)..a.y.max(b.y)).contains(&y) {
            let x = a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y);
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    for center in [start, end] {
        let dy = y - center.y;
        if dy.abs() <= radius {
            let dx = (radius * radius - dy * dy).max(0.).sqrt();
            lo = lo.min(center.x - dx);
            hi = hi.max(center.x + dx);
        }
    }
    (lo <= hi).then_some((lo, hi))
}
fn close_coverage(x: f32, y: f32, phase: Vec2, scale: f32, padding: Vec2, stroke: f32) -> f32 {
    let mut covered = 0.;
    for sy in 0..128 {
        let yy = (y + (sy as f32 + 0.5) / 128. - phase.y) / scale;
        let Some((lo, hi)) = segment_interval(yy, padding, stroke * 0.5) else {
            continue;
        };
        let (a, b) = (lo * scale + phase.x, hi * scale + phase.x);
        let (c, d) = ((16. - hi) * scale + phase.x, (16. - lo) * scale + phase.x);
        let clip = |lo: f32, hi: f32| (hi.min(x + 1.) - lo.max(x)).max(0.);
        covered += clip(a, b) + clip(c, d) - clip(a.max(c), b.min(d));
    }
    (covered / 128.).clamp(0., 1.)
}
fn circle_coverage(x: f32, y: f32, phase: Vec2, scale: f32, radius: f32) -> f32 {
    let r = radius * scale;
    let center = 8. * scale;
    let mut sum = 0.;
    for sy in 0..64 {
        let dy = y + (sy as f32 + 0.5) / 64. - phase.y - center;
        if dy.abs() >= r {
            continue;
        }
        let dx = (r * r - dy * dy).sqrt();
        sum += ((phase.x + center + dx).min(x + 1.) - (phase.x + center - dx).max(x)).clamp(0., 1.);
    }
    sum / 64.
}
fn round_rect_coverage(x: f32, y: f32, size: Vec2, phase: Vec2, radius: f32) -> f32 {
    let mut coverage = 0.;
    for sample in 0..128 {
        let yy = y + (sample as f32 + 0.5) / 128. - phase.y;
        if !(0. ..size.y).contains(&yy) {
            continue;
        }
        let dy = (radius - yy).max(yy - (size.y - radius)).max(0.);
        let inset = radius - (radius * radius - dy * dy).max(0.).sqrt();
        coverage +=
            ((phase.x + size.x - inset).min(x + 1.) - (phase.x + inset).max(x)).clamp(0., 1.);
    }
    coverage / 128.
}
impl TabPaint {
    pub fn feedback(&mut self, ui: &Ui, rect: Rect, colors: [Color32; 3], radius: f32) {
        self.paint(
            ui,
            rect,
            Spec {
                kind: Kind::Feedback,
                colors,
                params: [radius, 0., 0., 0.],
                hover: false,
            },
        );
    }
    pub fn badge(
        &mut self,
        ui: &Ui,
        rect: Rect,
        name: &str,
        colors: [Color32; 3],
        opacity: f32,
    ) -> bool {
        let kind = match name.rsplit('.').next() {
            Some("ts" | "tsx") => Kind::TypeScript,
            Some("php") => Kind::Php,
            _ => return false,
        };
        self.paint(
            ui,
            rect,
            Spec {
                kind,
                colors,
                params: [opacity, 0., 0., 0.],
                hover: false,
            },
        );
        true
    }
    pub fn close(&mut self, ui: &Ui, rect: Rect, style: CloseStyle) {
        self.paint(
            ui,
            rect,
            Spec {
                kind: Kind::Close,
                colors: [style.foreground, style.background, style.hover_fill],
                params: [
                    style.padding.x,
                    style.padding.y,
                    style.stroke,
                    style.hover_radius,
                ],
                hover: style.hovered,
            },
        );
    }
    pub fn underline(
        &mut self,
        ui: &Ui,
        rect: Rect,
        colors: [Color32; 3],
        radius: [f32; 2],
        inset: f32,
    ) {
        self.paint(
            ui,
            rect,
            Spec {
                kind: Kind::Pill,
                colors,
                params: [radius[0], radius[1], inset, 0.],
                hover: false,
            },
        );
    }
    fn paint(&mut self, ui: &Ui, rect: Rect, spec: Spec) {
        let Spec {
            kind,
            colors,
            params,
            hover,
        } = spec;
        if !ui.is_rect_visible(rect) {
            return;
        }
        let scale = ui.ctx().pixels_per_point();
        let min = rect.min.to_vec2() * scale;
        let origin = min.floor();
        let phase = min - origin;
        let size = rect.size() * scale;
        if size.x > 8192. || size.y > 128. || !size.is_finite() || !scale.is_finite() || scale <= 0.
        {
            // Bounded fallback keeps controls visible at unsupported extreme sizes.
            match kind {
                Kind::Pill | Kind::Feedback => {
                    ui.painter().rect_filled(rect, 2., colors[0]);
                }
                Kind::Close => {
                    let p = |v: Vec2| rect.min + v * rect.size();
                    for (a, b) in [
                        (vec2(0.3, 0.3), vec2(0.7, 0.7)),
                        (vec2(0.7, 0.3), vec2(0.3, 0.7)),
                    ] {
                        ui.painter()
                            .line_segment([p(a), p(b)], egui::Stroke::new(1., colors[0]));
                    }
                }
                Kind::TypeScript | Kind::Php => {
                    ui.painter().rect_filled(rect.shrink(2.), 2., colors[0]);
                }
            }
            return;
        }
        let key = Key {
            kind,
            scale: scale.to_bits(),
            size: [size.x.to_bits(), size.y.to_bits()],
            phase: [phase.x.to_bits(), phase.y.to_bits()],
            colors: colors.map(rgb),
            params: params.map(f32::to_bits),
            hover,
        };
        let index = self.cache.iter().position(|(k, _, _)| k == &key);
        let mut mesh = if let Some(index) = index {
            let entry = self.cache.remove(index).unwrap();
            let result = entry.1.clone();
            self.cache.push_back(entry);
            result
        } else {
            let mesh = Self::mesh(&key, size, phase, scale, params);
            let bytes = mesh.vertices.len() * std::mem::size_of::<egui::epaint::Vertex>()
                + mesh.indices.len() * std::mem::size_of::<u32>();
            while !self.cache.is_empty()
                && (self.cache.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES)
            {
                self.bytes -= self.cache.pop_front().unwrap().2;
            }
            if bytes <= MAX_BYTES {
                self.bytes += bytes;
                self.cache.push_back((key, mesh.clone(), bytes));
            }
            mesh
        };
        mesh.translate(origin / scale);
        ui.painter().add(Shape::mesh(mesh));
    }
    fn mesh(key: &Key, size: Vec2, phase: Vec2, scale: f32, params: [f32; 4]) -> Mesh {
        let [foreground, background, detail] = key.colors;
        let mut mesh = Mesh::default();
        let badge = matches!(key.kind, Kind::TypeScript | Kind::Php).then(|| {
            super::tab_badges::raster(
                key.kind == Kind::Php,
                size,
                phase,
                scale,
                key.colors,
                params[0],
            )
        });
        for y in 0..(phase.y + size.y).ceil() as usize {
            let mut spans: Vec<([u8; 3], usize, usize)> = Vec::new();
            for x in 0..(phase.x + size.x).ceil() as usize {
                let px = x as f32;
                let py = y as f32;
                let c = match key.kind {
                    Kind::Feedback => blend_premult(
                        background,
                        foreground,
                        round_rect_coverage(px, py, size, phase, params[0] * scale),
                    ),
                    Kind::TypeScript | Kind::Php => {
                        badge.as_ref().unwrap()[y * (phase.x + size.x).ceil() as usize + x]
                    }
                    Kind::Close => {
                        let base = if key.hover {
                            blend(
                                background,
                                detail,
                                circle_coverage(px, py, phase, scale, params[3]),
                            )
                        } else {
                            background
                        };
                        blend_premult(
                            base,
                            foreground,
                            close_coverage(
                                px,
                                py,
                                phase,
                                scale,
                                vec2(params[0], params[1]),
                                params[2],
                            ),
                        )
                    }
                    Kind::Pill => {
                        let a = pill_coverage(
                            px,
                            py,
                            size,
                            phase,
                            vec2(params[0], params[1]) * scale,
                            params[2] * scale,
                        );
                        let border = (py + 1. - (phase.y + size.y - scale)).clamp(0., 1.);
                        let base = blend_premult(background, foreground, a * (1. - border));
                        blend_premult(blend_premult(base, detail, border), foreground, a)
                    }
                };
                if let Some((last, _, right)) = spans.last_mut()
                    && *last == c
                {
                    *right = x + 1;
                } else {
                    spans.push((c, x, x + 1));
                }
            }
            for (c, left, right) in spans {
                // Opaque, physically aligned coverage quads avoid a second AA pass.
                mesh.add_colored_rect(
                    Rect::from_min_max(
                        pos2(left as f32 / scale, y as f32 / scale),
                        pos2(right as f32 / scale, (y + 1) as f32 / scale),
                    ),
                    Color32::from_rgb(c[0], c[1], c[2]),
                );
            }
        }
        mesh
    }
}
