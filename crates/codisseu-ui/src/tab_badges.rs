// Copyright 2000-2023 JetBrains s.r.o. and contributors (path coordinates).
// Adapted 2026 by Codisseu: cubic outlines flattened by an independent renderer.
// Licensed under Apache-2.0; see reference/public/intellij-oss/LICENSE-APACHE-2.0.txt.
// Public upstream revision 441b046f2985d841fd90bc64b7f312636644718d.
// Source/audit: reference/public/icons/sources.json. No proprietary extraction.
use egui::{Vec2, vec2};
use std::sync::OnceLock;
#[derive(Clone, Copy)]
enum Curve {
    Move(f32, f32),
    Line(f32, f32),
    Cubic([f32; 6]),
    Close,
}
struct Path {
    contours: Vec<Vec<Vec2>>,
    white: bool,
}
impl Path {
    fn new(white: bool, commands: &[Curve]) -> Self {
        let mut contours = Vec::new();
        let mut contour = Vec::new();
        let mut p = Vec2::ZERO;
        for command in commands {
            match *command {
                Curve::Move(x, y) => {
                    if !contour.is_empty() {
                        contours.push(std::mem::take(&mut contour));
                    }
                    p = vec2(x, y);
                    contour.push(p);
                }
                Curve::Line(x, y) => {
                    p = vec2(x, y);
                    contour.push(p);
                }
                Curve::Cubic([x1, y1, x2, y2, x3, y3]) => {
                    let start = p;
                    p = vec2(x3, y3);
                    flatten(start, vec2(x1, y1), vec2(x2, y2), p, 0, &mut contour);
                }
                Curve::Close => {
                    if let Some(first) = contour.first().copied() {
                        contour.push(first);
                    }
                    contours.push(std::mem::take(&mut contour));
                }
            }
        }
        if !contour.is_empty() {
            contours.push(contour);
        }
        Self { contours, white }
    }
}

/// Generic adaptive cubic flattening, in logical icon coordinates.
fn flatten(a: Vec2, b: Vec2, c: Vec2, d: Vec2, depth: u8, points: &mut Vec<Vec2>) {
    let delta = d - a;
    let len = delta.length();
    let distance = |p: Vec2| {
        if len > 0. {
            (delta.x * (a.y - p.y) - delta.y * (a.x - p.x)).abs() / len
        } else {
            (p - a).length()
        }
    };
    let error = distance(b).max(distance(c));
    let area_error = if len > 1e-9 {
        error * len
    } else {
        error * error
    };
    if area_error <= 0.1 || depth >= 10 {
        points.push(d);
        return;
    }
    let ab = (a + b) * 0.5;
    let bc = (b + c) * 0.5;
    let cd = (c + d) * 0.5;
    let abc = (ab + bc) * 0.5;
    let bcd = (bc + cd) * 0.5;
    let center = (abc + bcd) * 0.5;
    flatten(a, ab, abc, center, depth + 1, points);
    flatten(center, bcd, cd, d, depth + 1, points);
}
fn typescript() -> &'static [Path] {
    static PATHS: OnceLock<Vec<Path>> = OnceLock::new();
    PATHS.get_or_init(|| {
        vec![
            Path::new(
                false,
                &[
                    Curve::Move(14., 4.),
                    Curve::Cubic([14., 2.89543, 13.1046, 2., 12., 2.]),
                    Curve::Line(4., 2.),
                    Curve::Cubic([2.89543, 2., 2., 2.89543, 2., 4.]),
                    Curve::Line(2., 12.),
                    Curve::Cubic([2., 13.1046, 2.89543, 14., 4., 14.]),
                    Curve::Line(12., 14.),
                    Curve::Cubic([13.1046, 14., 14., 13.1046, 14., 12.]),
                    Curve::Line(14., 4.),
                    Curve::Close,
                ],
            ),
            Path::new(
                true,
                &[
                    Curve::Move(3.13501, 5.99999),
                    Curve::Line(7.87171, 5.99999),
                    Curve::Line(7.87171, 6.99999),
                    Curve::Line(6., 6.99999),
                    Curve::Line(6., 12.),
                    Curve::Line(5., 12.),
                    Curve::Line(5., 6.99999),
                    Curve::Line(3.13501, 6.99999),
                    Curve::Line(3.13501, 5.99999),
                    Curve::Close,
                ],
            ),
            Path::new(
                true,
                &[
                    Curve::Move(10.6833, 12.0902),
                    Curve::Cubic([10.2587, 12.0902, 9.8825, 12.0133, 9.55475, 11.8594]),
                    Curve::Cubic([9.227, 11.7026, 8.9705, 11.486, 8.78525, 11.2096]),
                    Curve::Cubic([8.6, 10.9303, 8.50595, 10.6111, 8.5031, 10.252]),
                    Curve::Line(9.51627, 10.252),
                    Curve::Cubic([9.51627, 10.4401, 9.56615, 10.6082, 9.6659, 10.7564]),
                    Curve::Cubic([9.76565, 10.9046, 9.90387, 11.02, 10.0806, 11.1027]),
                    Curve::Cubic([10.2601, 11.1825, 10.4653, 11.2224, 10.6962, 11.2224]),
                    Curve::Cubic([10.9213, 11.2224, 11.1194, 11.1853, 11.2904, 11.1112]),
                    Curve::Cubic([11.4614, 11.0371, 11.5925, 10.9331, 11.6837, 10.7992]),
                    Curve::Cubic([11.7777, 10.6624, 11.8248, 10.5056, 11.8248, 10.3289]),
                    Curve::Cubic([11.8248, 10.1066, 11.7578, 9.91567, 11.6238, 9.75607]),
                    Curve::Cubic([11.4899, 9.59362, 11.3118, 9.49387, 11.0895, 9.45682]),
                    Curve::Line(10.0763, 9.28582),
                    Curve::Cubic([9.7856, 9.23452, 9.53337, 9.13192, 9.31962, 8.97802]),
                    Curve::Cubic([9.10587, 8.82412, 8.94057, 8.63175, 8.82372, 8.4009]),
                    Curve::Cubic([8.70972, 8.1672, 8.65272, 7.91212, 8.65272, 7.63567]),
                    Curve::Cubic([8.65272, 7.30507, 8.73822, 7.00867, 8.90922, 6.74647]),
                    Curve::Cubic([9.08307, 6.48142, 9.3239, 6.2748, 9.6317, 6.1266]),
                    Curve::Cubic([9.9395, 5.97555, 10.2872, 5.90002, 10.6748, 5.90002]),
                    Curve::Cubic([11.0709, 5.90002, 11.4229, 5.9727, 11.7307, 6.11805]),
                    Curve::Cubic([12.0414, 6.2634, 12.2836, 6.46575, 12.4575, 6.7251]),
                    Curve::Cubic([12.6313, 6.98445, 12.7197, 7.28085, 12.7225, 7.6143]),
                    Curve::Line(11.7093, 7.6143),
                    Curve::Cubic([11.7093, 7.45185, 11.6666, 7.3065, 11.5811, 7.17825]),
                    Curve::Cubic([11.4956, 7.04715, 11.3759, 6.94597, 11.222, 6.87472]),
                    Curve::Cubic([11.0681, 6.80347, 10.89, 6.76785, 10.6876, 6.76785]),
                    Curve::Cubic([10.4853, 6.76785, 10.3071, 6.80205, 10.1532, 6.87045]),
                    Curve::Cubic([9.99935, 6.93885, 9.87965, 7.03575, 9.79415, 7.16115]),
                    Curve::Cubic([9.70865, 7.28655, 9.6659, 7.4319, 9.6659, 7.5972]),
                    Curve::Cubic([9.6659, 7.791, 9.7286, 7.95915, 9.854, 8.10165]),
                    Curve::Cubic([9.9794, 8.2413, 10.1433, 8.32822, 10.3456, 8.36242]),
                    Curve::Line(11.3417, 8.52487),
                    Curve::Cubic([11.6352, 8.57617, 11.896, 8.6859, 12.124, 8.85405]),
                    Curve::Cubic([12.3549, 9.0222, 12.533, 9.23167, 12.6584, 9.48247]),
                    Curve::Cubic([12.7838, 9.73327, 12.8465, 9.99975, 12.8465, 10.2819]),
                    Curve::Cubic([12.8465, 10.6239, 12.7553, 10.9331, 12.5729, 11.2096]),
                    Curve::Cubic([12.3905, 11.4832, 12.1354, 11.6983, 11.8077, 11.8551]),
                    Curve::Cubic([11.4799, 12.0118, 11.1051, 12.0902, 10.6833, 12.0902]),
                    Curve::Close,
                ],
            ),
        ]
    })
}

fn php() -> &'static [Path] {
    static PATHS: OnceLock<Vec<Path>> = OnceLock::new();
    PATHS.get_or_init(|| {
        vec![
            Path::new(
                false,
                &[
                    Curve::Move(1.26612, 5.6911),
                    Curve::Line(3.63899, 5.6911),
                    Curve::Cubic([4.33545, 5.69695, 4.84013, 5.89775, 5.15303, 6.29317]),
                    Curve::Cubic([5.46593, 6.6886, 5.56917, 7.22869, 5.46292, 7.91346]),
                    Curve::Cubic([5.42166, 8.22635, 5.33011, 8.53324, 5.18845, 8.83427]),
                    Curve::Cubic([5.05263, 9.13531, 4.86385, 9.40677, 4.62179, 9.64884]),
                    Curve::Cubic([4.3266, 9.95572, 4.01086, 10.1505, 3.67441, 10.2332]),
                    Curve::Cubic([3.33796, 10.3159, 2.98965, 10.3572, 2.62964, 10.3572]),
                    Curve::Line(1.56716, 10.3572),
                    Curve::Line(1.23071, 12.0394),
                    Curve::Line(0., 12.0394),
                    Curve::Line(1.26612, 5.6911),
                    Curve::Close,
                    Curve::Move(2.30204, 6.70046),
                    Curve::Line(1.7708, 9.35666),
                    Curve::Cubic([1.80622, 9.3625, 1.84163, 9.36551, 1.87705, 9.36551]),
                    Curve::Cubic([1.91831, 9.36551, 1.95974, 9.36551, 2.001, 9.36551]),
                    Curve::Cubic([2.56766, 9.37136, 3.03993, 9.31539, 3.41764, 9.19729]),
                    Curve::Cubic([3.79536, 9.07333, 4.04929, 8.64249, 4.17909, 7.9046]),
                    Curve::Cubic([4.28534, 7.28482, 4.17909, 6.92765, 3.86034, 6.83327]),
                    Curve::Cubic([3.54745, 6.73888, 3.15503, 6.69461, 2.68276, 6.70046]),
                    Curve::Cubic([2.61193, 6.7063, 2.54411, 6.70931, 2.47912, 6.70931]),
                    Curve::Cubic([2.42015, 6.70931, 2.35817, 6.70931, 2.29319, 6.70931]),
                    Curve::Line(2.30204, 6.70046),
                    Curve::Close,
                ],
            ),
            Path::new(
                false,
                &[
                    Curve::Move(6.86482, 4.),
                    Curve::Line(8.08667, 4.),
                    Curve::Line(7.74137, 5.69111),
                    Curve::Line(8.83926, 5.69111),
                    Curve::Cubic([9.44133, 5.70298, 9.88988, 5.82693, 10.1851, 6.06298]),
                    Curve::Cubic([10.4861, 6.29903, 10.5746, 6.74775, 10.4507, 7.40879]),
                    Curve::Line(9.85747, 10.3572),
                    Curve::Line(8.61791, 10.3572),
                    Curve::Line(9.18457, 7.5416),
                    Curve::Cubic([9.24354, 7.24641, 9.22583, 7.03692, 9.13144, 6.91297]),
                    Curve::Cubic([9.03706, 6.78901, 8.83342, 6.72703, 8.52052, 6.72703]),
                    Curve::Line(7.53772, 6.71818),
                    Curve::Line(6.8117, 10.3572),
                    Curve::Line(5.58984, 10.3572),
                    Curve::Line(6.86482, 4.),
                    Curve::Close,
                ],
            ),
            Path::new(
                false,
                &[
                    Curve::Move(11.7622, 5.6911),
                    Curve::Line(14.1351, 5.6911),
                    Curve::Cubic([14.8315, 5.69695, 15.3362, 5.89775, 15.6491, 6.29317]),
                    Curve::Cubic([15.962, 6.6886, 16.0653, 7.22869, 15.959, 7.91346]),
                    Curve::Cubic([15.9177, 8.22635, 15.8262, 8.53324, 15.6845, 8.83427]),
                    Curve::Cubic([15.5487, 9.13531, 15.3599, 9.40677, 15.1179, 9.64884]),
                    Curve::Cubic([14.8227, 9.95572, 14.507, 10.1505, 14.1705, 10.2332]),
                    Curve::Cubic([13.8341, 10.3159, 13.4857, 10.3572, 13.1257, 10.3572]),
                    Curve::Line(12.0633, 10.3572),
                    Curve::Line(11.7268, 12.0394),
                    Curve::Line(10.4961, 12.0394),
                    Curve::Line(11.7622, 5.6911),
                    Curve::Close,
                    Curve::Move(12.7981, 6.70046),
                    Curve::Line(12.2669, 9.35666),
                    Curve::Cubic([12.3023, 9.3625, 12.3377, 9.36551, 12.3731, 9.36551]),
                    Curve::Cubic([12.4144, 9.36551, 12.4558, 9.36551, 12.4971, 9.36551]),
                    Curve::Cubic([13.0638, 9.37136, 13.536, 9.31539, 13.9137, 9.19729]),
                    Curve::Cubic([14.2915, 9.07333, 14.5454, 8.64249, 14.6752, 7.9046]),
                    Curve::Cubic([14.7814, 7.28482, 14.6752, 6.92765, 14.3564, 6.83327]),
                    Curve::Cubic([14.0435, 6.73888, 13.6511, 6.69461, 13.1789, 6.70046]),
                    Curve::Cubic([13.108, 6.7063, 13.0402, 6.70931, 12.9752, 6.70931]),
                    Curve::Cubic([12.9162, 6.70931, 12.8543, 6.70931, 12.7893, 6.70931]),
                    Curve::Line(12.7981, 6.70046),
                    Curve::Close,
                ],
            ),
        ]
    })
}

/// Non-zero winding scanline coverage, once per bounded mesh cache miss.
pub(super) fn raster(
    php_icon: bool,
    size: Vec2,
    phase: Vec2,
    scale: f32,
    colors: [[u8; 3]; 3],
    opacity: f32,
) -> Vec<[u8; 3]> {
    let w = (phase.x + size.x).ceil() as usize;
    let h = (phase.y + size.y).ceil() as usize;
    let [blue, bg, white] = colors;
    let mut pixels = vec![bg; w * h];
    for path in if php_icon { php() } else { typescript() } {
        let mut coverage = vec![0_f32; w * h];
        let mut intersections = Vec::new();
        for y in 0..h {
            for sample in 0..8 {
                let yy = (y as f32 + (sample as f32 + 0.5) / 8. - phase.y) / scale;
                intersections.clear();
                for contour in &path.contours {
                    for edge in contour.windows(2) {
                        let [a, b] = [edge[0], edge[1]];
                        if (a.y <= yy && yy < b.y) || (b.y <= yy && yy < a.y) {
                            let x =
                                phase.x + scale * (a.x + (b.x - a.x) * (yy - a.y) / (b.y - a.y));
                            intersections.push((
                                (x * 256.).round() / 256.,
                                if b.y > a.y { 1_i32 } else { -1 },
                            ));
                        }
                    }
                }
                intersections.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                let mut winding = 0;
                let mut left: f32 = 0.;
                for &(right, direction) in &intersections {
                    if winding != 0 {
                        for x in
                            (left.floor().max(0.) as usize)..(right.ceil().max(0.) as usize).min(w)
                        {
                            coverage[y * w + x] +=
                                (right.min(x as f32 + 1.) - left.max(x as f32)).max(0.) / 8.;
                        }
                    }
                    winding += direction;
                    left = right;
                }
            }
        }
        let fg = if path.white { white } else { blue };
        for (c, a) in pixels.iter_mut().zip(coverage) {
            *c = super::tab_paint::blend_premult(*c, fg, a);
        }
    }
    if opacity < 1. {
        for pixel in &mut pixels {
            *pixel = super::tab_paint::blend_premult(bg, *pixel, opacity);
        }
    }
    pixels
}
