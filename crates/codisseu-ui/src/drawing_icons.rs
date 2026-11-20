//! Official MIT Excalidraw/Tabler artwork; pinned SVG/PNG provenance and notices
//! are in reference/public/excalidraw-icons. Raster assets are alpha-only: the
//! native theme supplies their foreground color, without modifying geometry.
use egui::{self, Color32, Rect, TextureHandle, Ui, Vec2};
use std::collections::{HashMap, VecDeque};

macro_rules! asset {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../assets/excalidraw/toolbar-", $name, ".png")).as_slice(),
            Vec2::splat(64.),
        )
    };
}

fn asset(name: &str) -> Option<(&'static str, &'static [u8], Vec2)> {
    Some(match name {
        "lock" | "lock-closed" => asset!("lock"),
        "unlock" | "lock-open" => asset!("unlock"),
        "hand" => asset!("hand"),
        "select" | "selection" => asset!("select"),
        "rectangle" => asset!("rectangle"),
        "diamond" => asset!("diamond"),
        "ellipse" => asset!("ellipse"),
        "arrow" => asset!("arrow"),
        "line" => asset!("line"),
        "freedraw" => asset!("freedraw"),
        "text" => asset!("text"),
        "eraser" => asset!("eraser"),
        "more" => asset!("more"),
        "menu" => asset!("menu"),
        "zoom-in" | "zoom-plus" => asset!("zoom-in"),
        "zoom-out" | "zoom-minus" => asset!("zoom-out"),
        "undo" => asset!("undo"),
        "redo" => asset!("redo"),
        "fit" => asset!("fit"),
        "duplicate" => asset!("duplicate"),
        "delete" => asset!("delete"),
        "front" => asset!("front"),
        "back" => asset!("back"),
        "grid" => asset!("grid"),
        "source" => asset!("source"),
        "save" => asset!("save"),
        "export" => asset!("export"),
        "note" => asset!("note"),
        "image" => asset!("image"),
        "library" => asset!("library"),
        "help" => asset!("help"),
        "open" => asset!("open"),
        "logo" => (
            "logo",
            include_bytes!("../assets/excalidraw/toolbar-logo.png").as_slice(),
            egui::vec2(844., 120.),
        ),
        "logo-text" => (
            "logo-text",
            include_bytes!("../assets/excalidraw/toolbar-logo-text.png").as_slice(),
            egui::vec2(900., 110.),
        ),
        "excalidraw" => (
            "excalidraw",
            include_bytes!("../assets/excalidraw/excalidraw-symbol.png").as_slice(),
            Vec2::splat(64.),
        ),
        _ => return None,
    })
}

type IconKey = (&'static str, u16, u16);
#[derive(Clone, Default)]
struct IconCache {
    textures: HashMap<IconKey, TextureHandle>,
    order: VecDeque<IconKey>,
}

/// Paint inside `rect`, preserving the official artwork's aspect ratio. Ordinary
/// toolbar callers supply a 16px icon rectangle within their 32px hit target.
pub(super) fn paint(ui: &Ui, rect: Rect, name: &str, tint: Color32) {
    let Some((name, bytes, source_size)) = asset(name) else {
        return;
    };
    if !rect.is_finite() || rect.width() <= 0. || rect.height() <= 0. {
        return;
    }
    let ratio = (rect.width() / source_size.x).min(rect.height() / source_size.y);
    let size = source_size * ratio;
    let physical = (size * ui.ctx().pixels_per_point())
        .round()
        .max(Vec2::splat(1.));
    let scale = (source_size.max_elem().min(1024.) / physical.max_elem()).min(1.);
    let width = (physical.x * scale).round().max(1.) as u16;
    let height = (physical.y * scale).round().max(1.) as u16;
    let key = (name, width, height);
    let cache_id = egui::Id::unique("codisseu.excalidraw-icon.cache");
    let cached = ui.ctx().data_mut(|data| {
        data.get_temp_mut_or_default::<IconCache>(cache_id)
            .textures
            .get(&key)
            .cloned()
    });
    let texture = cached.unwrap_or_else(|| {
        let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .expect("bundled audited Excalidraw icon is a valid PNG")
            .to_rgba8();
        let image = image::imageops::resize(
            &decoded,
            u32::from(width),
            u32::from(height),
            image::imageops::FilterType::Lanczos3,
        );
        let texture = ui.ctx().load_texture(
            format!("codisseu.excalidraw.{name}.{width}x{height}"),
            egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        ui.ctx().data_mut(|data| {
            let cache = data.get_temp_mut_or_default::<IconCache>(cache_id);
            if cache.textures.len() >= 128
                && let Some(old) = cache.order.pop_front()
            {
                cache.textures.remove(&old);
            }
            cache.order.push_back(key);
            cache.textures.insert(key, texture.clone());
        });
        texture
    });
    ui.painter().image(
        texture.id(),
        Rect::from_center_size(rect.center(), size),
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
        tint.gamma_multiply(if ui.is_enabled() { 1. } else { 0.4 }),
    );
}
