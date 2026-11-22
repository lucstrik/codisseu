//! Drawing presentation only: authored scene colors remain unchanged.
//!
//! The published Excalidraw theme uses `invert(93%) hue-rotate(180deg)`.
//! Sources inspected on 2026-11-20 (documented, not a local browser measurement):
//! https://github.com/excalidraw/excalidraw/blob/2c3dcc06ec807e81ab9437eceb1a8f7a3c590743/packages/common/src/constants.ts
//! https://github.com/excalidraw/excalidraw/blob/2c3dcc06ec807e81ab9437eceb1a8f7a3c590743/packages/excalidraw/css/theme.scss
//! The display transform below implements the sRGB CSS filter equations:
//! https://www.w3.org/TR/filter-effects-1/#feColorMatrixElement
//! Palette roles select the corresponding published theme color values; their
//! use in native widgets is Codisseu's own implementation.
use codisseu_draw::Scene;
use egui::{Color32, Ui};

const MODE_KEY: &str = "codisseuTheme";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CanvasTheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ThemePalette {
    pub canvas: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub hover: Color32,
    pub active: Color32,
    pub accent: Color32,
}

impl CanvasTheme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "Sistema",
            Self::Light => "Claro",
            Self::Dark => "Escuro",
        }
    }

    pub fn from_scene(scene: &Scene) -> Self {
        match scene
            .app_state
            .get(MODE_KEY)
            .and_then(|value| value.as_str())
        {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn set_scene_mode(scene: &mut Scene, mode: Self) -> bool {
        set_scene_mode(scene, mode)
    }

    pub fn is_dark(self, ui: &Ui) -> bool {
        match self {
            Self::System => ui.visuals().dark_mode,
            Self::Light => false,
            Self::Dark => true,
        }
    }

    pub fn scene_color(self, ui: &Ui, color: Color32) -> Color32 {
        display_color(color, self.is_dark(ui))
    }

    pub fn canvas_background(self, ui: &Ui, scene: &Scene) -> Color32 {
        let authored = scene
            .app_state
            .get("viewBackgroundColor")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .and_then(|value| match value {
                "transparent" => Some(Color32::TRANSPARENT),
                "white" => Some(Color32::WHITE),
                "black" => Some(Color32::BLACK),
                _ => value
                    .parse::<egui::ecolor::HexColor>()
                    .ok()
                    .map(|hex| hex.color()),
            });
        authored.map_or_else(
            || self.palette(ui).canvas,
            |color| self.scene_color(ui, color),
        )
    }

    pub fn palette(self, ui: &Ui) -> ThemePalette {
        if self == Self::System {
            let visuals = ui.visuals();
            return ThemePalette {
                canvas: visuals.extreme_bg_color,
                panel: visuals.panel_fill,
                border: visuals.window_stroke.color,
                text: visuals.text_color(),
                muted: visuals.weak_text_color(),
                hover: visuals.widgets.hovered.bg_fill,
                active: visuals.selection.bg_fill,
                accent: visuals.hyperlink_color,
            };
        }
        if self.is_dark(ui) {
            ThemePalette {
                canvas: Color32::from_rgb(18, 18, 18),
                panel: Color32::from_rgb(35, 35, 41),
                border: Color32::from_rgb(70, 70, 79),
                text: Color32::from_rgb(227, 227, 232),
                muted: Color32::from_rgb(184, 184, 184),
                hover: Color32::from_rgb(46, 45, 57),
                active: Color32::from_rgb(64, 62, 106),
                accent: Color32::from_rgb(168, 165, 255),
            }
        } else {
            ThemePalette {
                canvas: Color32::WHITE,
                panel: Color32::WHITE,
                border: Color32::from_rgb(197, 197, 208),
                text: Color32::from_rgb(27, 27, 31),
                muted: Color32::from_rgb(92, 92, 92),
                hover: Color32::from_rgb(241, 240, 255),
                active: Color32::from_rgb(224, 223, 255),
                accent: Color32::from_rgb(105, 101, 219),
            }
        }
    }
}

/// Returns whether the shared document needs an edit/undo entry. Defaults do
/// not add metadata just by rendering or opening a menu.
pub(super) fn set_scene_mode(scene: &mut Scene, mode: CanvasTheme) -> bool {
    if CanvasTheme::from_scene(scene) == mode {
        return false;
    }
    let value = match mode {
        CanvasTheme::System => "system",
        CanvasTheme::Light => "light",
        CanvasTheme::Dark => "dark",
    };
    scene.app_state.insert(MODE_KEY.into(), value.into());
    true
}

/// CSS filters operate on unpremultiplied sRGB channels. Apply inversion and
/// the 180° hue matrix, then clamp to the display gamut and premultiply again.
/// Transparent colors remain transparent, and the alpha channel is preserved.
pub(super) fn display_color(color: Color32, dark: bool) -> Color32 {
    if !dark || color.a() == 0 {
        return color;
    }
    let [r, g, b, alpha] = color.to_srgba_unmultiplied();
    let invert = |channel: u8| 0.93 * 255. - 0.86 * f64::from(channel);
    let (r, g, b) = (invert(r), invert(g), invert(b));
    let byte = |channel: f64| channel.round().clamp(0., 255.) as u8;
    Color32::from_rgba_unmultiplied(
        byte(-0.574 * r + 1.430 * g + 0.144 * b),
        byte(0.426 * r + 0.430 * g + 0.144 * b),
        byte(0.426 * r + 1.430 * g - 0.856 * b),
        alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn css_filter_keeps_authored_black_strokes_visible_on_white_canvas() {
        assert_eq!(display_color(Color32::WHITE, true), Color32::from_gray(18));
        assert_eq!(display_color(Color32::BLACK, true), Color32::from_gray(237));
        assert_eq!(
            display_color(Color32::from_gray(30), true),
            Color32::from_gray(211)
        );
        assert_eq!(
            display_color(Color32::RED, true),
            Color32::from_rgb(255, 144, 144)
        );
        assert_eq!(
            display_color(Color32::BLUE, true),
            Color32::from_rgb(206, 206, 255)
        );
        for color in [Color32::TRANSPARENT, Color32::RED, Color32::from_gray(30)] {
            assert_eq!(display_color(color, false), color);
        }
        assert_eq!(
            display_color(Color32::TRANSPARENT, true),
            Color32::TRANSPARENT
        );
        let translucent = Color32::from_rgba_unmultiplied(30, 60, 90, 128);
        assert_eq!(display_color(translucent, true).a(), translucent.a());
    }

    #[test]
    fn theme_roundtrip_preserves_authored_colors_and_unrecognized_state() {
        let mut scene = Scene::default();
        let mut rectangle =
            codisseu_draw::Element::new(codisseu_draw::ElementKind::Rectangle, 10., 20., 120., 80.);
        rectangle.stroke_color = "#1e1e1e".into();
        rectangle.background_color = "#a5d8ff80".into();
        scene.elements.push(rectangle);
        let authored_elements = scene.elements.clone();
        scene
            .app_state
            .insert("viewBackgroundColor".into(), json!("#fffce8"));
        scene.app_state.insert("theme".into(), json!("light"));
        scene
            .app_state
            .insert("custom".into(), json!({"preserve": [1, 2, 3]}));
        let authored = scene.app_state.clone();
        assert_eq!(CanvasTheme::from_scene(&scene), CanvasTheme::System);
        assert!(!set_scene_mode(&mut scene, CanvasTheme::System));
        assert_eq!(scene.app_state, authored);
        for mode in [CanvasTheme::Dark, CanvasTheme::Light, CanvasTheme::System] {
            assert!(set_scene_mode(&mut scene, mode));
            assert!(!set_scene_mode(&mut scene, mode));
            let restored = Scene::from_json(&scene.to_json().unwrap()).unwrap();
            assert_eq!(CanvasTheme::from_scene(&restored), mode);
            let mut state = restored.app_state.clone();
            state.remove(MODE_KEY);
            assert_eq!(state, authored);
            assert_eq!(restored.elements, authored_elements);
            assert_eq!(restored.files, scene.files);
        }
    }

    #[test]
    fn system_follows_native_visuals_and_explicit_modes_override_it() {
        let context = egui::Context::default();
        let scene = Scene::default();
        for dark in [true, false, true] {
            context.set_visuals(if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            });
            let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(CanvasTheme::System.is_dark(ui), dark);
                assert!(!CanvasTheme::Light.is_dark(ui));
                assert!(CanvasTheme::Dark.is_dark(ui));
                let expected = if dark {
                    Color32::from_gray(18)
                } else {
                    Color32::WHITE
                };
                assert_eq!(CanvasTheme::System.canvas_background(ui, &scene), expected);
                let system = CanvasTheme::System.palette(ui);
                assert_eq!(system.canvas, ui.visuals().extreme_bg_color);
                assert_eq!(system.panel, ui.visuals().panel_fill);
                assert_eq!(system.text, ui.visuals().text_color());
                // Explicit drawing modes retain their own published palette.
                assert_eq!(CanvasTheme::Dark.palette(ui).canvas, Color32::from_gray(18));
                assert_eq!(CanvasTheme::Light.palette(ui).canvas, Color32::WHITE);
            });
            // This test inspects styles and colors without a GPU renderer.
            output.textures_delta.clear();
        }
        assert!(!scene.app_state.contains_key(MODE_KEY));
    }

    #[test]
    fn invalid_custom_mode_uses_system_without_rewriting_the_document() {
        let mut scene = Scene::default();
        scene
            .app_state
            .insert(MODE_KEY.into(), json!({"future": true}));
        let original = scene.to_json().unwrap();
        assert_eq!(CanvasTheme::from_scene(&scene), CanvasTheme::System);
        assert!(!set_scene_mode(&mut scene, CanvasTheme::System));
        assert_eq!(scene.to_json().unwrap(), original);
    }
}
