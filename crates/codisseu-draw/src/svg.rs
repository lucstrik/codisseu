use std::fmt::Write;

use crate::{Element, ElementKind, Point, Result, Scene};

fn escaped(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn color(value: &str, fallback: &str) -> String {
    if value == "transparent" {
        return "none".into();
    }
    let hex = value.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if hex
        || matches!(
            value,
            "black"
                | "white"
                | "red"
                | "green"
                | "blue"
                | "gray"
                | "grey"
                | "yellow"
                | "orange"
                | "purple"
                | "pink"
                | "none"
        )
    {
        value.into()
    } else {
        fallback.into()
    }
}

fn rotated(point: Point, center: Point, angle: f64) -> Point {
    let [x, y] = [point[0] - center[0], point[1] - center[1]];
    let (sin, cos) = angle.sin_cos();
    [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos]
}

fn scene_bounds(scene: &Scene) -> [f64; 4] {
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for element in scene.elements.iter().filter(|element| !element.is_deleted) {
        let [x0, y0, x1, y1] = element.bounds();
        let center = [
            element.x + element.width / 2.0,
            element.y + element.height / 2.0,
        ];
        for corner in [[x0, y0], [x1, y0], [x1, y1], [x0, y1]] {
            let [x, y] = rotated(corner, center, element.angle);
            let margin = element.stroke_width.max(1.0) / 2.0
                + if element.kind == ElementKind::Arrow {
                    32.0
                } else {
                    0.0
                };
            bounds[0] = bounds[0].min(x - margin);
            bounds[1] = bounds[1].min(y - margin);
            bounds[2] = bounds[2].max(x + margin);
            bounds[3] = bounds[3].max(y + margin);
        }
    }
    if bounds[0].is_finite() {
        [
            bounds[0] - 20.0,
            bounds[1] - 20.0,
            bounds[2] + 20.0,
            bounds[3] + 20.0,
        ]
    } else {
        [0.0, 0.0, 800.0, 600.0]
    }
}

pub(crate) fn export(scene: &Scene) -> Result<String> {
    scene.validate()?;
    if scene.unsupported_count() > 0 {
        return Err("SVG indisponível: o desenho contém elementos ainda não renderizados pelo Codisseu; o arquivo Excalidraw original é preservado".into());
    }
    let [x0, y0, x1, y1] = scene_bounds(scene);
    let width = (x1 - x0).max(1.0);
    let height = (y1 - y0).max(1.0);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"{x0} {y0} {width} {height}\">\n"
    );
    let background = color(
        scene
            .app_state
            .get("viewBackgroundColor")
            .and_then(|value| value.as_str())
            .unwrap_or("#ffffff"),
        "#ffffff",
    );
    writeln!(
        svg,
        "<rect x=\"{x0}\" y=\"{y0}\" width=\"{width}\" height=\"{height}\" fill=\"{background}\"/>"
    )
    .expect("write to String");
    for element in scene.elements.iter().filter(|element| !element.is_deleted) {
        append_element(&mut svg, element);
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

fn append_element(svg: &mut String, element: &Element) {
    let stroke = color(&element.stroke_color, "#1b1b1f");
    let fill = color(&element.background_color, "none");
    let dash = match element
        .extra
        .get("strokeStyle")
        .and_then(|value| value.as_str())
    {
        Some("dashed") => " stroke-dasharray=\"8 6\"",
        Some("dotted") => " stroke-dasharray=\"2 6\"",
        _ => "",
    };
    writeln!(svg,"<g transform=\"translate({} {}) rotate({} {} {})\" opacity=\"{}\" stroke=\"{}\" fill=\"{}\" stroke-width=\"{}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{}>",element.x,element.y,element.angle.to_degrees(),element.width/2.0,element.height/2.0,element.opacity/100.0,stroke,fill,element.stroke_width,dash).expect("write to String");
    match element.kind {
        ElementKind::Rectangle => {
            writeln!(
                svg,
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                element.width.min(0.0),
                element.height.min(0.0),
                element.width.abs(),
                element.height.abs()
            )
            .expect("write to String");
        }
        ElementKind::Ellipse => {
            writeln!(
                svg,
                "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"/>",
                element.width / 2.0,
                element.height / 2.0,
                element.width.abs() / 2.0,
                element.height.abs() / 2.0
            )
            .expect("write to String");
        }
        ElementKind::Diamond => {
            writeln!(
                svg,
                "<polygon points=\"{},0 {},{} {},{} 0,{}\"/>",
                element.width / 2.0,
                element.width,
                element.height / 2.0,
                element.width / 2.0,
                element.height,
                element.height / 2.0
            )
            .expect("write to String");
        }
        ElementKind::Line | ElementKind::Arrow | ElementKind::Freedraw => {
            let points: Vec<Point> = if element.points.is_empty() {
                vec![[0.0, 0.0], [element.width, element.height]]
            } else {
                element.points.clone()
            };
            let coordinates = points
                .iter()
                .map(|[x, y]| format!("{x},{y}"))
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(svg, "<polyline points=\"{coordinates}\" fill=\"none\"/>")
                .expect("write to String");
            if element.kind == ElementKind::Arrow && points.len() > 1 {
                if element
                    .extra
                    .get("endArrowhead")
                    .is_none_or(|value| !value.is_null())
                {
                    arrowhead(
                        svg,
                        points[points.len() - 2],
                        points[points.len() - 1],
                        element.stroke_width,
                    );
                }
                if element
                    .extra
                    .get("startArrowhead")
                    .is_some_and(|value| !value.is_null())
                {
                    arrowhead(svg, points[1], points[0], element.stroke_width);
                }
            }
        }
        ElementKind::Text => {
            let font_size = element.font_size;
            let line_height = element
                .extra
                .get("lineHeight")
                .and_then(|value| value.as_f64())
                .filter(|value| value.is_finite() && (0.5..=5.0).contains(value))
                .unwrap_or(1.25);
            let (anchor, x) = match element
                .extra
                .get("textAlign")
                .and_then(|value| value.as_str())
            {
                Some("center") => ("middle", element.width / 2.0),
                Some("right") => ("end", element.width),
                _ => ("start", 0.0),
            };
            writeln!(svg,"<text fill=\"{stroke}\" stroke=\"none\" font-family=\"sans-serif\" font-size=\"{font_size}\" text-anchor=\"{anchor}\" xml:space=\"preserve\">").expect("write to String");
            for (line, text) in element.text.split('\n').enumerate() {
                writeln!(
                    svg,
                    "<tspan x=\"{x}\" y=\"{}\">{}</tspan>",
                    font_size + line as f64 * font_size * line_height,
                    escaped(text)
                )
                .expect("write to String");
            }
            svg.push_str("</text>\n");
        }
        ElementKind::Other(_) => unreachable!("unsupported elements rejected before export"),
    }
    svg.push_str("</g>\n");
}

fn arrowhead(svg: &mut String, from: Point, to: Point, stroke_width: f64) {
    let angle = (to[1] - from[1]).atan2(to[0] - from[0]);
    let length = (stroke_width * 4.0 + 8.0).min(32.0);
    let a = angle + 0.45;
    let b = angle - 0.45;
    writeln!(
        svg,
        "<polyline points=\"{},{} {},{} {},{}\" fill=\"none\"/>",
        to[0] - length * a.cos(),
        to[1] - length * a.sin(),
        to[0],
        to[1],
        to[0] - length * b.cos(),
        to[1] - length * b.sin()
    )
    .expect("write to String");
}
