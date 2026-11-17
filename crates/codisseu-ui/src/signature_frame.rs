//! Procedural quadratic balloon frame. Shadow remains caller-owned.
//! Geometry is independent of reference pixels; AA coverage is integrated per row.
use egui::{Color32, Mesh, Rect, Shape, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};
use std::collections::VecDeque;

const MAX_PIXELS: usize = 65_536;
const MAX_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 32;
const SAMPLES: usize = 64;

#[derive(Clone, PartialEq, Eq)]
struct Key {
    size: [u32; 2],
    phase: [u32; 2],
    scale: u32,
    radius: u32,
    tip: u32,
    below: bool,
    colors: [[u8; 4]; 2],
}
#[derive(Clone, Default)]
struct Cache {
    entries: VecDeque<(Key, Mesh, usize)>,
    bytes: usize,
}
#[derive(Clone, Copy)]
struct Geometry {
    size: Vec2,
    phase: Vec2,
    radius: f32,
    tip: f32,
    scale: f32,
    below: bool,
}
impl Geometry {
    fn interval(self, y: f32, inset: f32) -> Option<(f32, f32)> {
        let w = self.size.x;
        let h = self.size.y;
        let r = (self.radius - inset).max(0.001);
        let yy = y - inset;
        let hh = h - 2. * inset;
        let body = if yy >= 0. && yy < hh {
            let distance = yy.min(hh - yy).clamp(0., r);
            // A quadratic corner from (r,0), control (0,0), to (0,r).
            let edge = r * (1. - (distance / r).sqrt()).powi(2);
            Some((inset + edge, w - inset - edge))
        } else {
            None
        };
        let tip_y = h + 7. * self.scale;
        let triangle_start = h - self.scale - inset;
        // Center the one-point diagonal stroke without extending the pointer height.
        let half_width = tip_y - y + self.scale * std::f32::consts::FRAC_1_SQRT_2
            - inset * std::f32::consts::SQRT_2;
        let triangle = (y >= triangle_start && y < tip_y && half_width > 0.)
            .then_some((self.tip - half_width, self.tip + half_width));
        match (body, triangle) {
            (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
            (Some(body), None) => Some(body),
            (None, Some(triangle)) => Some(triangle),
            (None, None) => None,
        }
    }
    fn coverage_row(self, row: usize, inset: f32) -> [Option<(f32, f32)>; SAMPLES] {
        std::array::from_fn(|index| {
            let mut y = row as f32 + (index as f32 + 0.5) / SAMPLES as f32 - self.phase.y;
            if self.below {
                y = self.size.y - y;
            }
            self.interval(y, inset)
                .map(|(a, b)| (a + self.phase.x, b + self.phase.x))
        })
    }
}
fn coverage(x: usize, samples: &[Option<(f32, f32)>; SAMPLES]) -> f32 {
    let x = x as f32;
    samples
        .iter()
        .flatten()
        .map(|(a, b)| (b.min(x + 1.) - a.max(x)).clamp(0., 1.))
        .sum::<f32>()
        / SAMPLES as f32
}
fn pixel(outer: f32, inner: f32, background: Color32, border: Color32) -> Color32 {
    let inner = inner.clamp(0., outer);
    let ring = outer - inner;
    let alpha = (outer * 255.).round() as u8;
    let components: [u8; 3] = std::array::from_fn(|i| {
        (background.to_array()[i] as f32 * inner + border.to_array()[i] as f32 * ring)
            .round()
            .clamp(0., f32::from(alpha)) as u8
    });
    Color32::from_rgba_premultiplied(components[0], components[1], components[2], alpha)
}
fn raster(geometry: Geometry, background: Color32, border: Color32) -> Option<Mesh> {
    let width = (geometry.phase.x + geometry.size.x).ceil() as usize + 1;
    let height = (geometry.phase.y
        + geometry.size.y
        + if geometry.below {
            0.
        } else {
            7. * geometry.scale
        })
    .ceil() as usize
        + 1;
    if width == 0
        || height == 0
        || width > 4096
        || height > 512
        || width.checked_mul(height)? > MAX_PIXELS
    {
        return None;
    }
    let mut mesh = Mesh::default();
    for y in 0..height {
        let outer = geometry.coverage_row(y, 0.);
        let inner = geometry.coverage_row(y, geometry.scale);
        let inner_span = inner
            .iter()
            .copied()
            .try_fold((f32::NEG_INFINITY, f32::INFINITY), |(a, b), item| {
                item.map(|(c, d)| (a.max(c), b.min(d)))
            });
        let mut spans: Vec<(Color32, usize, usize)> = Vec::new();
        for x in 0..width {
            let color = if inner_span.is_some_and(|(a, b)| x as f32 >= a && (x + 1) as f32 <= b) {
                background
            } else {
                pixel(coverage(x, &outer), coverage(x, &inner), background, border)
            };
            if color == Color32::TRANSPARENT {
                continue;
            }
            if let Some((last, _, right)) = spans.last_mut()
                && *last == color
                && *right == x
            {
                *right = x + 1;
            } else {
                spans.push((color, x, x + 1));
            }
        }
        for (color, left, right) in spans {
            mesh.add_colored_rect(
                Rect::from_min_max(
                    pos2(left as f32 / geometry.scale, y as f32 / geometry.scale),
                    pos2(
                        right as f32 / geometry.scale,
                        (y + 1) as f32 / geometry.scale,
                    ),
                ),
                color,
            );
        }
    }
    Some(mesh)
}
impl Cache {
    fn mesh(
        &mut self,
        key: Key,
        geometry: Geometry,
        background: Color32,
        border: Color32,
    ) -> Option<Mesh> {
        if let Some(index) = self.entries.iter().position(|(old, _, _)| old == &key) {
            let entry = self.entries.remove(index)?;
            let result = entry.1.clone();
            self.entries.push_back(entry);
            return Some(result);
        }
        let mesh = raster(geometry, background, border)?;
        let bytes = mesh.vertices.len() * std::mem::size_of::<egui::epaint::Vertex>()
            + mesh.indices.len() * std::mem::size_of::<u32>();
        while !self.entries.is_empty()
            && (self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES)
        {
            self.bytes -= self.entries.pop_front()?.2;
        }
        if bytes <= MAX_BYTES {
            self.bytes += bytes;
            self.entries.push_back((key, mesh.clone(), bytes));
        }
        Some(mesh)
    }
}
fn fallback(
    ui: &Ui,
    rect: Rect,
    tip_x: f32,
    below: bool,
    background: Color32,
    border: Color32,
    radius: f32,
) {
    ui.painter().rect(
        rect,
        radius,
        background,
        Stroke::new(1., border),
        StrokeKind::Inside,
    );
    let base = if below {
        rect.top() + 1.
    } else {
        rect.bottom() - 1.
    };
    let tip = if below {
        rect.top() - 7.
    } else {
        rect.bottom() + 7.
    };
    let points = vec![
        pos2(tip_x - 8., base),
        pos2(tip_x, tip),
        pos2(tip_x + 8., base),
    ];
    ui.painter().add(Shape::convex_polygon(
        points.clone(),
        background,
        Stroke::NONE,
    ));
    ui.painter()
        .add(Shape::line(points, Stroke::new(1., border)));
}
/// Paints the body/pointer only. The caller paints its shadow before this frame.
/// The mesh includes AA and the whole pointer; it never expands an ancestor clip.
#[allow(clippy::too_many_arguments)]
pub(super) fn paint(
    ui: &Ui,
    rect: Rect,
    tip_x: f32,
    below: bool,
    background: Color32,
    border: Color32,
    radius: f32,
) {
    if !rect.is_finite()
        || rect.width() < 32.
        || rect.height() < 12.
        || !tip_x.is_finite()
        || !radius.is_finite()
    {
        return;
    }
    let scale = ui.ctx().pixels_per_point();
    let radius = radius.clamp(0., (rect.height() * 0.5 - 1.).min(rect.width() * 0.5 - 8.));
    let tip_x = tip_x.clamp(rect.left() + radius + 8., rect.right() - radius - 8.);
    if !(0.75..=3.).contains(&scale)
        || background.a() != 255
        || border.a() != 255
        || ui.painter().opacity() != 1.
        || ui
            .ctx()
            .layer_transform_to_global(ui.layer_id())
            .is_some_and(|t| t != egui::emath::TSTransform::IDENTITY)
    {
        fallback(ui, rect, tip_x, below, background, border, radius);
        return;
    }
    let extent = rect.expand2(vec2(1., 9.));
    if !ui.is_rect_visible(extent) {
        return;
    }
    let origin = (rect.min.to_vec2() * scale).floor()
        - vec2(1., if below { (7. * scale).ceil() + 1. } else { 1. });
    let phase = rect.min.to_vec2() * scale - origin;
    let geometry = Geometry {
        size: rect.size() * scale,
        phase,
        radius: (radius + 0.5) * scale,
        tip: (tip_x - rect.left()) * scale + 0.5,
        scale,
        below,
    };
    let key = Key {
        size: [geometry.size.x.to_bits(), geometry.size.y.to_bits()],
        phase: [phase.x.to_bits(), phase.y.to_bits()],
        scale: scale.to_bits(),
        radius: geometry.radius.to_bits(),
        tip: geometry.tip.to_bits(),
        below,
        colors: [background.to_array(), border.to_array()],
    };
    let mesh = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<Cache>(egui::Id::unique("codisseu.signature.frame.cache"))
            .mesh(key, geometry, background, border)
    });
    if let Some(mut mesh) = mesh {
        mesh.translate(origin / scale);
        ui.painter().add(Shape::mesh(mesh));
    } else {
        fallback(ui, rect, tip_x, below, background, border, radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> Geometry {
        Geometry {
            size: vec2(183., 51.),
            phase: vec2(1., 1.),
            radius: 8.5,
            tip: 20.,
            scale: 1.,
            below: false,
        }
    }
    #[test]
    fn quadratic_interval_and_pointer_have_a_continuous_fill() {
        let g = geometry();
        let (left, right) = g.interval(4., 0.).unwrap();
        assert!((left - 8.5 * (1. - (4f32 / 8.5).sqrt()).powi(2)).abs() < 0.0001);
        assert!((right + left - 183.).abs() < 0.0001);
        for y in [49.9, 50., 50.1, 50.9, 51., 51.1] {
            let (outer_left, outer_right) = g.interval(y, 0.).unwrap();
            let (inner_left, inner_right) = g.interval(y, 1.).unwrap();
            assert!(outer_left <= inner_left && outer_right >= inner_right);
            assert!(inner_left < 20. && inner_right > 20.);
        }
        assert!(g.interval(58., 0.).is_none());
        let inverted = Geometry {
            phase: vec2(1., 8.),
            below: true,
            ..g
        };
        for y in 0..60 {
            for inset in [0., 1.] {
                let above = g.coverage_row(y, inset);
                let below = inverted.coverage_row(59 - y, inset);
                for x in [0, 1, 7, 19, 20, 21, 90, 182, 183] {
                    assert!((coverage(x, &above) - coverage(x, &below)).abs() < 0.00001);
                }
            }
        }
    }
    #[test]
    fn raster_is_bounded_and_cache_releases_old_geometry() {
        let background = Color32::from_rgb(30, 31, 34);
        let border = Color32::from_rgb(49, 50, 52);
        let mut cache = Cache::default();
        for index in 0..70 {
            let mut g = geometry();
            g.size.x += index as f32;
            let key = Key {
                size: [g.size.x.to_bits(), g.size.y.to_bits()],
                phase: [1f32.to_bits(); 2],
                scale: 1f32.to_bits(),
                radius: g.radius.to_bits(),
                tip: g.tip.to_bits(),
                below: false,
                colors: [background.to_array(), border.to_array()],
            };
            let mesh = cache.mesh(key, g, background, border).unwrap();
            assert!(mesh.is_valid());
            assert!(cache.entries.len() <= MAX_ENTRIES && cache.bytes <= MAX_BYTES);
        }
        let mut too_large = geometry();
        too_large.size = vec2(4096., 512.);
        assert!(raster(too_large, background, border).is_none());
    }
}
