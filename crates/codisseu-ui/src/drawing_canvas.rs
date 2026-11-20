//! Native drawing surface for the interoperable Excalidraw scene format.
//! Gestures update the preview; only completed edits are returned to the common
//! document service, which owns undo, persistence and external-file conflicts.
use crate::drawing_theme::{CanvasTheme, ThemePalette};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_draw::{Element, ElementKind, Scene};
use egui::{
    Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2,
};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Tool {
    #[default]
    Select,
    Hand,
    Rectangle,
    Ellipse,
    Diamond,
    Arrow,
    Line,
    Freedraw,
    Text,
    Eraser,
}
impl Tool {
    fn label(self) -> &'static str {
        match self {
            Self::Select => "Selecionar · V ou 1",
            Self::Hand => "Mover tela · H ou 2",
            Self::Rectangle => "Retângulo · R ou 3",
            Self::Ellipse => "Elipse · O ou 4",
            Self::Diamond => "Losango · D ou 5",
            Self::Arrow => "Seta · A ou 6",
            Self::Line => "Linha · L ou 7",
            Self::Freedraw => "Desenho livre · P ou 8",
            Self::Text => "Texto · T ou 9",
            Self::Eraser => "Apagar · E",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Hand => "hand",
            Self::Rectangle => "rectangle",
            Self::Ellipse => "ellipse",
            Self::Diamond => "diamond",
            Self::Arrow => "arrow",
            Self::Line => "line",
            Self::Freedraw => "freedraw",
            Self::Text => "text",
            Self::Eraser => "eraser",
        }
    }
    fn shortcut(self) -> Option<&'static str> {
        Some(match self {
            Self::Select => "V",
            Self::Hand => "H",
            Self::Rectangle => "R",
            Self::Ellipse => "O",
            Self::Diamond => "D",
            Self::Arrow => "A",
            Self::Line => "L",
            Self::Freedraw => "P",
            Self::Text => "T",
            Self::Eraser => "E",
        })
    }
    fn kind(self) -> Option<ElementKind> {
        Some(match self {
            Self::Rectangle => ElementKind::Rectangle,
            Self::Ellipse => ElementKind::Ellipse,
            Self::Diamond => ElementKind::Diamond,
            Self::Arrow => ElementKind::Arrow,
            Self::Line => ElementKind::Line,
            Self::Freedraw => ElementKind::Freedraw,
            Self::Text => ElementKind::Text,
            Self::Select | Self::Hand | Self::Eraser => return None,
        })
    }
}

#[derive(Default)]
pub(super) struct CanvasResponse {
    /// One completed user edit. The caller serializes the scene once here.
    pub changed: bool,
    pub undo: bool,
    pub redo: bool,
    pub save: bool,
    pub source: bool,
    pub copy_svg: bool,
}

enum Gesture {
    Erase {
        originals: Vec<Element>,
    },
    Pan {
        origin: Vec2,
        start: Pos2,
    },
    Draw {
        id: String,
        start: Pos2,
    },
    Move {
        start: Pos2,
        originals: Vec<Element>,
    },
    Resize {
        id: String,
        original: Box<Element>,
        corner: usize,
        start: Pos2,
    },
    Marquee {
        start: Pos2,
        current: Pos2,
        additive: bool,
    },
}
struct TextEditor {
    id: Option<String>,
    position: Pos2,
    text: String,
    focus: bool,
    original: Option<Element>,
}

pub(super) struct DrawingCanvasUi {
    tool: Tool,
    selected: BTreeSet<String>,
    pan: Vec2,
    zoom: f32,
    stroke: Color32,
    fill: Color32,
    stroke_width: f64,
    font_size: f64,
    gesture: Option<Gesture>,
    text_editor: Option<TextEditor>,
    fit_pending: bool,
    /// Kept for deterministic interaction tests and native test adapters.
    pub canvas_rect: Rect,
    last_pointer: Option<Pos2>,
    locked: bool,
    menu_open: bool,
    opacity: f64,
    theme: CanvasTheme,
    overlay_rects: Vec<Rect>,
    save_requested: bool,
}
impl Default for DrawingCanvasUi {
    fn default() -> Self {
        Self {
            tool: Tool::Select,
            selected: BTreeSet::new(),
            pan: egui::vec2(64., 64.),
            zoom: 1.,
            stroke: Color32::from_rgb(30, 30, 30),
            fill: Color32::TRANSPARENT,
            stroke_width: 2.,
            font_size: 24.,
            gesture: None,
            text_editor: None,
            fit_pending: true,
            canvas_rect: Rect::NOTHING,
            last_pointer: None,
            locked: false,
            menu_open: false,
            opacity: 100.,
            theme: CanvasTheme::default(),
            overlay_rects: Vec::new(),
            save_requested: false,
        }
    }
}

impl DrawingCanvasUi {
    /// Drop transient previews when the common document service publishes a
    /// newer revision (Undo, reload or another view). The caller replaces Scene.
    pub(super) fn cancel_interaction(&mut self) {
        self.gesture = None;
        self.text_editor = None;
        self.last_pointer = None;
        self.menu_open = false;
    }

    pub(super) fn request_save(&mut self) {
        self.save_requested = true;
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene) -> CanvasResponse {
        let mut result = CanvasResponse::default();
        self.theme = CanvasTheme::from_scene(scene);
        self.selected
            .retain(|id| scene.elements.iter().any(|e| &e.id == id && !e.is_deleted));
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(egui::vec2(1., 80.)),
            Sense::click_and_drag(),
        );
        self.canvas_rect = rect;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Canvas,
                ui.is_enabled(),
                "Tela de desenho Excalidraw",
            )
        });
        if self.fit_pending && rect.width() > 10. {
            self.fit(scene, rect);
            self.fit_pending = false;
        }
        self.overlay_rects.clear();
        let mut chrome = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("drawing.chrome")
                .max_rect(rect),
        );
        chrome.set_clip_rect(rect.intersect(ui.clip_rect()));
        let palette = self.theme.palette(ui);
        self.toolbar(&chrome, rect, &palette);
        self.menu(&chrome, scene, rect, &palette, &mut result);
        self.footer(&chrome, rect, &palette, &mut result);
        if !self.menu_open && (self.tool.kind().is_some() || !self.selected.is_empty()) {
            self.properties(&chrome, scene, rect, &palette, &mut result);
        }
        if !ui.is_enabled() {
            self.cancel_gesture(scene);
            self.text_editor = None;
        } else if self.text_editor.is_none() {
            let pointer = ui.input(|i| i.pointer.interact_pos());
            let blocked = pointer.is_some_and(|p| self.overlay_rects.iter().any(|r| r.contains(p)));
            if self.gesture.is_some()
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.cancel_gesture(scene);
            }
            if !blocked && (response.clicked() || response.drag_started()) {
                response.request_focus();
                self.menu_open = false;
            }
            if response.has_focus() && !self.menu_open {
                self.keyboard(ui, scene, &mut result);
            }
            if response.hovered() && !blocked {
                let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
                if (zoom - 1.).abs() > 0.001 {
                    self.zoom_at(zoom, pointer.unwrap_or(rect.center()), rect);
                } else if scroll != Vec2::ZERO {
                    self.pan += scroll;
                }
            }
            if !blocked || self.gesture.is_some() {
                self.pointer(ui, scene, rect, &response, &mut result);
            }
            if response.hovered() && !blocked {
                let hand = self.tool == Tool::Hand || ui.input(|i| i.key_down(Key::Space));
                ui.ctx().set_cursor_icon(if hand {
                    egui::CursorIcon::Grab
                } else if self.tool == Tool::Select {
                    egui::CursorIcon::Default
                } else {
                    egui::CursorIcon::Crosshair
                });
            }
        }
        self.paint(ui, scene, rect);
        self.text_window(&chrome, scene, &mut result);
        result
    }

    fn toolbar(&mut self, ui: &Ui, rect: Rect, palette: &ThemePalette) {
        let width = (11.0_f32 * 32. + 10. * 3. + 25.).min((rect.width() - 124.).max(160.));
        let position = egui::pos2(rect.center().x - width * 0.5, rect.top() + 12.);
        let area = island(ui, "drawing.toolbar", position, palette, 6., |ui| {
            ui.set_max_width(width - 12.);
            egui::ScrollArea::horizontal()
                .id_salt("drawing.tools")
                .codisseu()
                .wheel_to_horizontal()
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 3.;
                    ui.horizontal(|ui| {
                        let lock_name = if self.locked { "lock" } else { "unlock" };
                        if glyph_button(
                            ui,
                            lock_name,
                            "Manter ferramenta ativa",
                            self.locked,
                            None,
                            palette,
                        )
                        .on_hover_text("Manter a ferramenta após desenhar")
                        .clicked()
                        {
                            self.locked = !self.locked;
                        }
                        ui.separator();
                        for tool in [
                            Tool::Hand,
                            Tool::Select,
                            Tool::Rectangle,
                            Tool::Diamond,
                            Tool::Ellipse,
                            Tool::Arrow,
                            Tool::Line,
                            Tool::Freedraw,
                            Tool::Text,
                            Tool::Eraser,
                        ] {
                            if glyph_button(
                                ui,
                                tool.icon(),
                                tool.label(),
                                self.tool == tool,
                                tool.shortcut(),
                                palette,
                            )
                            .clicked()
                            {
                                if self.tool != tool && tool != Tool::Select {
                                    self.selected.clear();
                                }
                                self.tool = tool;
                            }
                        }
                    });
                });
        });
        self.overlay_rects.push(area);
    }

    fn menu(
        &mut self,
        ui: &Ui,
        scene: &mut Scene,
        rect: Rect,
        palette: &ThemePalette,
        result: &mut CanvasResponse,
    ) {
        let area = island(
            ui,
            "drawing.menu-button",
            rect.min + egui::vec2(12., 12.),
            palette,
            2.,
            |ui| {
                if glyph_button(ui, "menu", "Menu do desenho", self.menu_open, None, palette)
                    .clicked()
                {
                    self.menu_open = !self.menu_open;
                }
            },
        );
        self.overlay_rects.push(area);
        if !self.menu_open {
            return;
        }
        let area = island(
            ui,
            "drawing.menu",
            rect.min + egui::vec2(12., 56.),
            palette,
            8.,
            |ui| {
                ui.set_width(218.);
                for (icon, label, shortcut, flag) in [
                    ("save", "Salvar desenho", "Ctrl+S", &mut result.save),
                    ("source", "Código JSON", "", &mut result.source),
                    ("export", "Copiar SVG", "", &mut result.copy_svg),
                ] {
                    if menu_item(ui, icon, label, shortcut, palette) {
                        *flag = true;
                        self.menu_open = false;
                    }
                }
                ui.add_space(4.);
                ui.separator();
                ui.add_space(4.);
                ui.label(
                    egui::RichText::new("Aparência")
                        .size(11.)
                        .color(palette.muted),
                );
                ui.horizontal(|ui| {
                    for mode in CanvasTheme::ALL {
                        if ui
                            .add(
                                egui::Button::new(mode.label())
                                    .selected(mode == self.theme)
                                    .min_size(egui::vec2(62., 28.)),
                            )
                            .clicked()
                        {
                            result.changed |= CanvasTheme::set_scene_mode(scene, mode);
                            self.theme = mode;
                        }
                    }
                });
                ui.add_space(6.);
                let mut grid = scene
                    .app_state
                    .get("codisseuGrid")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if ui.checkbox(&mut grid, "Mostrar grade").changed() {
                    scene.app_state.insert("codisseuGrid".into(), grid.into());
                    result.changed = true;
                }
            },
        );
        self.overlay_rects.push(area);
    }

    fn footer(&mut self, ui: &Ui, rect: Rect, palette: &ThemePalette, result: &mut CanvasResponse) {
        let position = egui::pos2(rect.left() + 12., rect.bottom() - 48.);
        let zoom = island(ui, "drawing.zoom", position, palette, 2., |ui| {
            ui.spacing_mut().item_spacing.x = 1.;
            ui.horizontal(|ui| {
                if glyph_button(ui, "zoom-out", "Diminuir zoom", false, None, palette).clicked() {
                    self.zoom_at(0.8, rect.center(), rect);
                }
                if ui
                    .add(
                        egui::Button::new(format!("{:.0}%", self.zoom * 100.))
                            .frame(false)
                            .min_size(egui::vec2(48., 32.)),
                    )
                    .on_hover_text("Restaurar zoom de 100%")
                    .clicked()
                {
                    self.zoom_at(1. / self.zoom, rect.center(), rect);
                }
                if glyph_button(ui, "zoom-in", "Aumentar zoom", false, None, palette).clicked() {
                    self.zoom_at(1.25, rect.center(), rect);
                }
                ui.separator();
                if glyph_button(ui, "fit", "Enquadrar", false, None, palette).clicked() {
                    self.fit_pending = true;
                }
            });
        });
        self.overlay_rects.push(zoom);
        let history = island(
            ui,
            "drawing.history",
            egui::pos2(zoom.right() + 8., position.y),
            palette,
            2.,
            |ui| {
                ui.spacing_mut().item_spacing.x = 1.;
                ui.horizontal(|ui| {
                    if glyph_button(ui, "undo", "Desfazer", false, None, palette)
                        .on_hover_text("Desfazer · Ctrl+Z")
                        .clicked()
                    {
                        result.undo = true;
                    }
                    if glyph_button(ui, "redo", "Refazer", false, None, palette)
                        .on_hover_text("Refazer · Ctrl+Shift+Z")
                        .clicked()
                    {
                        result.redo = true;
                    }
                });
            },
        );
        self.overlay_rects.push(history);
    }

    fn properties(
        &mut self,
        ui: &Ui,
        scene: &mut Scene,
        rect: Rect,
        palette: &ThemePalette,
        result: &mut CanvasResponse,
    ) {
        if rect.width() < 360. {
            return;
        }
        let mut changed = false;
        let has_text = self.tool == Tool::Text
            || scene
                .elements
                .iter()
                .any(|e| self.selected.contains(&e.id) && e.kind == ElementKind::Text);
        let has_shape = self.tool != Tool::Text
            && (self.selected.is_empty()
                || scene
                    .elements
                    .iter()
                    .any(|e| self.selected.contains(&e.id) && e.kind != ElementKind::Text));
        let desired_height = 120.0_f32
            + if has_shape { 140. } else { 0. }
            + if has_text { 70. } else { 0. }
            + if self.selected.is_empty() { 0. } else { 148. };
        let panel_height = desired_height.min((rect.height() - 176.).max(80.));
        let area = island(
            ui,
            "drawing.properties",
            rect.min + egui::vec2(12., 84.),
            palette,
            12.,
            |ui| {
                ui.set_width(170.);
                egui::ScrollArea::vertical()
                    .id_salt("drawing.properties-scroll")
                    .max_height(panel_height)
                    .min_scrolled_height(panel_height)
                    .codisseu()
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 8.;
                        ui.spacing_mut().slider_width = 170.;
                        ui.label(egui::RichText::new("Traço").size(12.));
                        changed |= color_palette(ui, &mut self.stroke, false, self.theme, palette);
                        if self.tool != Tool::Text
                            && (self.selected.is_empty()
                                || scene.elements.iter().any(|e| {
                                    self.selected.contains(&e.id) && e.kind != ElementKind::Text
                                }))
                        {
                            ui.label(egui::RichText::new("Fundo").size(12.));
                            changed |= color_palette(ui, &mut self.fill, true, self.theme, palette);
                            ui.label(egui::RichText::new("Espessura do traço").size(12.));
                            ui.horizontal(|ui| {
                                for width in [1., 2., 4.] {
                                    let response = ui.add(
                                        egui::Button::new("")
                                            .min_size(egui::vec2(38., 28.))
                                            .selected((self.stroke_width - width).abs() < 0.1),
                                    );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::accesskit::Role::Button,
                                            ui.is_enabled(),
                                            (self.stroke_width - width).abs() < 0.1,
                                            format!("Espessura {width}"),
                                        )
                                    });
                                    let r = response.rect;
                                    ui.painter().line_segment(
                                        [
                                            r.center() - egui::vec2(9., 0.),
                                            r.center() + egui::vec2(9., 0.),
                                        ],
                                        Stroke::new(width as f32, palette.text),
                                    );
                                    if response.clicked() {
                                        self.stroke_width = width;
                                        changed = true;
                                    }
                                }
                            });
                        }
                        if self.tool == Tool::Text
                            || scene.elements.iter().any(|e| {
                                self.selected.contains(&e.id) && e.kind == ElementKind::Text
                            })
                        {
                            ui.label(egui::RichText::new("Tamanho da fonte").size(12.));
                            ui.horizontal(|ui| {
                                for (label, size) in
                                    [("P", 16.), ("M", 24.), ("G", 36.), ("GG", 48.)]
                                {
                                    if ui
                                        .add(
                                            egui::Button::new(label)
                                                .selected((self.font_size - size).abs() < 0.1)
                                                .min_size(egui::vec2(32., 28.)),
                                        )
                                        .clicked()
                                    {
                                        self.font_size = size;
                                        changed = true;
                                    }
                                }
                            });
                        }
                        ui.label(egui::RichText::new("Opacidade").size(12.));
                        let opacity = ui.add(
                            egui::Slider::new(&mut self.opacity, 0.0..=100.)
                                .show_value(false)
                                .trailing_fill(true),
                        );
                        opacity.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::accesskit::Role::Slider,
                                ui.is_enabled(),
                                "Opacidade do elemento",
                            )
                        });
                        changed |= opacity.changed();
                        if !self.selected.is_empty() {
                            ui.label(egui::RichText::new("Camadas").size(12.));
                            ui.horizontal(|ui| {
                                if glyph_button(ui, "back", "Enviar atrás", false, None, palette)
                                    .clicked()
                                {
                                    result.changed |= self.reorder(scene, false);
                                }
                                if glyph_button(
                                    ui,
                                    "front",
                                    "Trazer à frente",
                                    false,
                                    None,
                                    palette,
                                )
                                .clicked()
                                {
                                    result.changed |= self.reorder(scene, true);
                                }
                            });
                            ui.label(egui::RichText::new("Ações").size(12.));
                            ui.horizontal(|ui| {
                                if glyph_button(ui, "duplicate", "Duplicar", false, None, palette)
                                    .clicked()
                                {
                                    result.changed |= self.duplicate(scene);
                                }
                                if glyph_button(ui, "delete", "Excluir", false, None, palette)
                                    .clicked()
                                {
                                    result.changed |= self.delete(scene);
                                }
                                if self.selected.len() == 1
                                    && let Some(element) = scene.elements.iter().find(|e| {
                                        self.selected.contains(&e.id) && e.kind == ElementKind::Text
                                    })
                                    && glyph_button(
                                        ui,
                                        "text",
                                        "Editar texto",
                                        false,
                                        None,
                                        palette,
                                    )
                                    .clicked()
                                {
                                    self.text_editor = Some(TextEditor {
                                        id: Some(element.id.clone()),
                                        position: egui::pos2(element.x as f32, element.y as f32),
                                        text: element.text.clone(),
                                        focus: true,
                                        original: Some(element.clone()),
                                    });
                                }
                            });
                        }
                    });
            },
        );
        self.overlay_rects.push(area);
        if changed {
            for element in scene
                .elements
                .iter_mut()
                .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
            {
                element.stroke_color = color_string(self.stroke);
                element.background_color = color_string(self.fill);
                element.stroke_width = self.stroke_width;
                element.opacity = self.opacity;
                if element.kind == ElementKind::Text {
                    element.font_size = self.font_size;
                    text_dimensions(ui, element);
                }
                element.touch();
                result.changed = true;
            }
        }
    }

    fn cancel_gesture(&mut self, scene: &mut Scene) {
        match self.gesture.take() {
            Some(Gesture::Erase { originals }) => {
                for original in originals {
                    if let Some(e) = scene.elements.iter_mut().find(|e| e.id == original.id) {
                        *e = original;
                    }
                }
            }
            Some(Gesture::Draw { id, .. }) => {
                scene.elements.retain(|e| e.id != id);
                self.selected.clear();
            }
            Some(Gesture::Move { originals, .. }) => {
                for original in originals {
                    if let Some(element) = scene.elements.iter_mut().find(|e| e.id == original.id) {
                        *element = original;
                    }
                }
            }
            Some(Gesture::Resize { id, original, .. }) => {
                if let Some(element) = scene.elements.iter_mut().find(|e| e.id == id) {
                    *element = *original;
                }
            }
            Some(Gesture::Pan { origin, .. }) => self.pan = origin,
            Some(Gesture::Marquee { .. }) | None => {}
        }
    }

    fn keyboard(&mut self, ui: &mut Ui, scene: &mut Scene, result: &mut CanvasResponse) {
        if self.gesture.is_some() {
            return;
        }
        ui.input_mut(|i| {
            let command = if cfg!(target_os = "macos") {
                Modifiers::MAC_CMD
            } else {
                Modifiers::CTRL
            };
            if i.consume_key(command | Modifiers::SHIFT, Key::Z) || i.consume_key(command, Key::Y) {
                result.redo = true;
            }
            if i.consume_key(command, Key::Z) {
                result.undo = true;
            }
            if i.consume_key(command, Key::A) {
                self.selected = scene
                    .elements
                    .iter()
                    .filter(|e| !e.is_deleted)
                    .map(|e| e.id.clone())
                    .collect();
                self.tool = Tool::Select;
            }
            if i.consume_key(Modifiers::NONE, Key::E) {
                self.tool = Tool::Eraser;
                self.selected.clear();
            }
            if i.consume_key(Modifiers::NONE, Key::Q) {
                self.locked = !self.locked;
            }
            if i.consume_key(command, Key::D) {
                result.changed |= self.duplicate(scene);
            }
            if i.consume_key(Modifiers::NONE, Key::Delete)
                || i.consume_key(Modifiers::NONE, Key::Backspace)
            {
                result.changed |= self.delete(scene);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                self.selected.clear();
                self.tool = Tool::Select;
            }
            if i.consume_key(Modifiers::SHIFT, Key::Num1) {
                self.fit_pending = true;
            }
            for (key, number, tool) in [
                (Key::V, Key::Num1, Tool::Select),
                (Key::H, Key::Num2, Tool::Hand),
                (Key::R, Key::Num3, Tool::Rectangle),
                (Key::O, Key::Num4, Tool::Ellipse),
                (Key::D, Key::Num5, Tool::Diamond),
                (Key::A, Key::Num6, Tool::Arrow),
                (Key::L, Key::Num7, Tool::Line),
                (Key::P, Key::Num8, Tool::Freedraw),
                (Key::T, Key::Num9, Tool::Text),
            ] {
                if i.consume_key(Modifiers::NONE, key) || i.consume_key(Modifiers::NONE, number) {
                    self.tool = tool;
                }
            }
            let distance = if i.modifiers.shift { 10. } else { 1. };
            let mut delta = Vec2::ZERO;
            for (key, direction) in [
                (Key::ArrowLeft, egui::vec2(-1., 0.)),
                (Key::ArrowRight, egui::vec2(1., 0.)),
                (Key::ArrowUp, egui::vec2(0., -1.)),
                (Key::ArrowDown, egui::vec2(0., 1.)),
            ] {
                if i.consume_key(
                    if i.modifiers.shift {
                        Modifiers::SHIFT
                    } else {
                        Modifiers::NONE
                    },
                    key,
                ) {
                    delta += direction * distance;
                }
            }
            if delta != Vec2::ZERO {
                for element in scene
                    .elements
                    .iter_mut()
                    .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
                {
                    element.x += delta.x as f64;
                    element.y += delta.y as f64;
                    element.touch();
                    result.changed = true;
                }
            }
        });
    }

    fn pointer(
        &mut self,
        ui: &Ui,
        scene: &mut Scene,
        rect: Rect,
        response: &egui::Response,
        result: &mut CanvasResponse,
    ) {
        let pointer = ui.input(|i| i.pointer.interact_pos()).or(self.last_pointer);
        self.last_pointer = pointer;
        let Some(pointer) = pointer else {
            return;
        };
        let world = self.world(pointer, rect);
        let shift = ui.input(|i| i.modifiers.shift);
        let middle = response.drag_started_by(PointerButton::Middle);
        if response.drag_started_by(PointerButton::Primary) || middle {
            let start = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
            let start_world = self.world(start, rect);
            if middle || self.tool == Tool::Hand || ui.input(|i| i.key_down(Key::Space)) {
                self.gesture = Some(Gesture::Pan {
                    origin: self.pan,
                    start,
                });
            } else if self.tool == Tool::Eraser {
                self.selected.clear();
                self.gesture = Some(Gesture::Erase {
                    originals: Vec::new(),
                });
            } else if self.tool == Tool::Select {
                if let Some((id, corner)) = self.resize_hit(scene, start, rect) {
                    let original = scene.elements.iter().find(|e| e.id == id).unwrap().clone();
                    self.gesture = Some(Gesture::Resize {
                        id,
                        original: Box::new(original),
                        corner,
                        start: start_world,
                    });
                } else if let Some(id) = hit(scene, start_world, 7. / self.zoom) {
                    if !self.selected.contains(&id) {
                        if !shift {
                            self.selected.clear();
                        }
                        self.selected.insert(id);
                    }
                    self.gesture = Some(Gesture::Move {
                        start: start_world,
                        originals: scene
                            .elements
                            .iter()
                            .filter(|e| self.selected.contains(&e.id))
                            .cloned()
                            .collect(),
                    });
                } else {
                    if !shift {
                        self.selected.clear();
                    }
                    self.gesture = Some(Gesture::Marquee {
                        start: start_world,
                        current: world,
                        additive: shift,
                    });
                }
            } else if self.tool != Tool::Text
                && let Some(kind) = self.tool.kind()
            {
                let mut element =
                    Element::new(kind, start_world.x as f64, start_world.y as f64, 0., 0.);
                self.apply_style(&mut element);
                let id = element.id.clone();
                scene.elements.push(element);
                self.selected.clear();
                self.selected.insert(id.clone());
                self.gesture = Some(Gesture::Draw {
                    id,
                    start: start_world,
                });
            }
        }
        if response.dragged()
            || response.dragged_by(PointerButton::Middle)
            || response.drag_stopped()
        {
            let erase_id = if self.tool == Tool::Eraser {
                hit(scene, world, 10. / self.zoom)
            } else {
                None
            };
            match &mut self.gesture {
                Some(Gesture::Erase { originals }) => {
                    if let Some(id) = erase_id
                        && let Some(element) = scene.elements.iter_mut().find(|e| e.id == id)
                    {
                        originals.push(element.clone());
                        element.is_deleted = true;
                    }
                }
                Some(Gesture::Pan { origin, start }) => self.pan = *origin + (pointer - *start),
                Some(Gesture::Move { start, originals }) => {
                    let delta = world - *start;
                    for original in originals {
                        if let Some(element) =
                            scene.elements.iter_mut().find(|e| e.id == original.id)
                        {
                            element.x = original.x + delta.x as f64;
                            element.y = original.y + delta.y as f64;
                        }
                    }
                }
                Some(Gesture::Resize {
                    id,
                    original,
                    corner,
                    start,
                }) => {
                    if let Some(element) = scene.elements.iter_mut().find(|e| &e.id == id) {
                        resize(element, original, *corner, world - *start, shift);
                    }
                }
                Some(Gesture::Draw { id, start }) => {
                    if let Some(element) = scene.elements.iter_mut().find(|e| &e.id == id) {
                        draw_preview(element, *start, world, shift);
                    }
                }
                Some(Gesture::Marquee { current, .. }) => *current = world,
                None => {}
            }
        }
        if response.drag_stopped_by(PointerButton::Primary)
            || response.drag_stopped_by(PointerButton::Middle)
        {
            match self.gesture.take() {
                Some(Gesture::Erase { originals }) => {
                    for original in originals {
                        if let Some(element) =
                            scene.elements.iter_mut().find(|e| e.id == original.id)
                        {
                            element.touch();
                            result.changed = true;
                        }
                    }
                }
                Some(Gesture::Draw { id, .. }) => {
                    if let Some(element) = scene.elements.iter_mut().find(|e| e.id == id) {
                        if element.width < 1. && element.height < 1. {
                            scene.elements.retain(|e| e.id != id);
                            self.selected.clear();
                        } else {
                            element.touch();
                            result.changed = true;
                        }
                    }
                }
                Some(Gesture::Move { originals, .. }) => {
                    for original in originals {
                        if let Some(element) =
                            scene.elements.iter_mut().find(|e| e.id == original.id)
                            && (element.x != original.x || element.y != original.y)
                        {
                            element.touch();
                            result.changed = true;
                        }
                    }
                }
                Some(Gesture::Resize { id, original, .. }) => {
                    if let Some(element) = scene.elements.iter_mut().find(|e| e.id == id)
                        && (element.x != original.x
                            || element.y != original.y
                            || element.width != original.width
                            || element.height != original.height)
                    {
                        element.touch();
                        result.changed = true;
                    }
                }
                Some(Gesture::Marquee {
                    start,
                    current,
                    additive,
                }) => {
                    if !additive {
                        self.selected.clear();
                    }
                    let selection = Rect::from_two_pos(start, current);
                    self.selected.extend(
                        scene
                            .elements
                            .iter()
                            .filter(|e| !e.is_deleted && selection.contains_rect(bounds(e)))
                            .map(|e| e.id.clone()),
                    );
                }
                Some(Gesture::Pan { .. }) | None => {}
            }
        }
        if result.changed && !self.locked && self.tool.kind().is_some() {
            self.tool = Tool::Select;
        }
        if response.clicked_by(PointerButton::Primary) {
            if self.tool == Tool::Eraser {
                if let Some(id) = hit(scene, world, 10. / self.zoom)
                    && let Some(element) = scene.elements.iter_mut().find(|e| e.id == id)
                {
                    element.is_deleted = true;
                    element.touch();
                    result.changed = true;
                }
            } else if self.tool == Tool::Text {
                self.text_editor = Some(TextEditor {
                    id: None,
                    position: world,
                    text: String::new(),
                    focus: true,
                    original: None,
                });
            } else if self.tool == Tool::Select {
                if let Some(id) = hit(scene, world, 7. / self.zoom) {
                    if shift && self.selected.contains(&id) {
                        self.selected.remove(&id);
                    } else {
                        if !shift {
                            self.selected.clear();
                        }
                        self.selected.insert(id.clone());
                        self.read_style(scene, &id);
                    }
                    if response.double_clicked()
                        && let Some(element) = scene
                            .elements
                            .iter()
                            .find(|e| e.id == id && e.kind == ElementKind::Text)
                    {
                        self.text_editor = Some(TextEditor {
                            id: Some(id),
                            position: egui::pos2(element.x as f32, element.y as f32),
                            text: element.text.clone(),
                            focus: true,
                            original: Some(element.clone()),
                        });
                    }
                } else {
                    self.selected.clear();
                }
            }
        }
    }

    fn text_window(&mut self, ui: &Ui, scene: &mut Scene, result: &mut CanvasResponse) {
        let Some(mut editor) = self.text_editor.take() else {
            if self.save_requested {
                result.save = true;
                self.save_requested = false;
            }
            return;
        };
        let just_opened = editor.focus;
        let palette = self.theme.palette(ui);
        let screen = self.screen(editor.position, self.canvas_rect);
        let width = (self.canvas_rect.width() - 32.).clamp(120., 320.);
        let position = egui::pos2(
            screen.x.clamp(
                self.canvas_rect.left() + 8.,
                (self.canvas_rect.right() - width - 8.).max(self.canvas_rect.left() + 8.),
            ),
            screen.y.clamp(
                self.canvas_rect.top() + 8.,
                (self.canvas_rect.bottom() - 110.).max(self.canvas_rect.top() + 8.),
            ),
        );
        let color = self.theme.scene_color(ui, self.stroke);
        let size = (self.font_size as f32 * self.zoom).clamp(10., 64.);
        let mut apply = self.save_requested || result.save;
        let mut cancel = false;
        let mut edited = false;
        let area = island(ui, "drawing.inline-text", position, &palette, 4., |ui| {
            ui.set_width(width);
            let field = ui.add(
                egui::TextEdit::multiline(&mut editor.text)
                    .font(FontId::proportional(size))
                    .text_color(color)
                    .frame(egui::Frame::NONE)
                    .margin(egui::vec2(4., 4.))
                    .desired_rows(2)
                    .desired_width(width)
                    .hint_text("Digite o texto…"),
            );
            field.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::TextInput,
                    true,
                    "Conteúdo do texto do desenho",
                )
            });
            if editor.focus {
                field.request_focus();
                editor.focus = false;
            }
            edited = field.changed();
            ui.horizontal(|ui| {
                let a = ui.add(
                    egui::Button::new(egui::RichText::new("Aplicar").size(11.))
                        .min_size(egui::vec2(0., 24.)),
                );
                a.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, "Aplicar texto")
                });
                apply |= a.clicked();
                let c = ui.add(
                    egui::Button::new(egui::RichText::new("Cancelar").size(11.))
                        .frame(false)
                        .min_size(egui::vec2(0., 24.)),
                );
                c.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, "Cancelar texto")
                });
                cancel = c.clicked();
                ui.weak(egui::RichText::new("Ctrl+Enter").size(10.));
            });
            ui.input_mut(|i| {
                cancel |= i.consume_key(Modifiers::NONE, Key::Escape);
                apply |= i.consume_key(Modifiers::CTRL, Key::Enter);
                if i.consume_key(Modifiers::CTRL, Key::S) {
                    apply = true;
                    self.save_requested = true;
                }
            });
        });
        if !just_opened
            && ui.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer.interact_pos().is_some_and(|p| !area.contains(p))
            })
        {
            apply = true;
        }
        if edited && !cancel {
            if let Some(id) = &editor.id {
                if let Some(element) = scene.elements.iter_mut().find(|e| &e.id == id) {
                    element.text = editor.text.clone();
                    text_dimensions(ui, element);
                    element.touch();
                    result.changed = true;
                }
            } else if !editor.text.is_empty() {
                let mut element = Element::new(
                    ElementKind::Text,
                    editor.position.x as f64,
                    editor.position.y as f64,
                    0.,
                    0.,
                );
                self.apply_style(&mut element);
                element.text = editor.text.clone();
                text_dimensions(ui, &mut element);
                element.touch();
                editor.id = Some(element.id.clone());
                self.selected.clear();
                self.selected.insert(element.id.clone());
                scene.elements.push(element);
                result.changed = true;
            }
        }
        if cancel {
            if let Some(id) = &editor.id {
                if let Some(original) = editor.original {
                    if let Some(element) = scene.elements.iter_mut().find(|e| &e.id == id)
                        && *element != original
                    {
                        *element = original;
                        result.changed = true;
                    }
                } else {
                    scene.elements.retain(|e| &e.id != id);
                    self.selected.remove(id);
                    result.changed = true;
                }
            }
        } else if !apply {
            self.text_editor = Some(editor);
        }
        if (apply || cancel) && !self.locked {
            self.tool = Tool::Select;
        }
        if self.save_requested {
            result.save = true;
            self.save_requested = false;
        }
    }

    fn world(&self, position: Pos2, rect: Rect) -> Pos2 {
        Pos2::ZERO + (position - rect.min - self.pan) / self.zoom
    }
    fn screen(&self, position: Pos2, rect: Rect) -> Pos2 {
        rect.min + self.pan + position.to_vec2() * self.zoom
    }
    fn zoom_at(&mut self, factor: f32, position: Pos2, rect: Rect) {
        if !rect.is_finite() {
            return;
        }
        let world = self.world(position, rect);
        self.zoom = (self.zoom * factor).clamp(0.1, 4.);
        self.pan = position - rect.min - world.to_vec2() * self.zoom;
    }
    fn fit(&mut self, scene: &Scene, rect: Rect) {
        let bounds = scene
            .elements
            .iter()
            .filter(|e| !e.is_deleted)
            .map(bounds)
            .fold(Rect::NOTHING, |a, b| a.union(b));
        if bounds.is_finite() {
            let available = (rect.size() - Vec2::splat(96.)).max(Vec2::splat(1.));
            self.zoom = (available.x / bounds.width().max(1.))
                .min(available.y / bounds.height().max(1.))
                .clamp(0.1, 1.5);
            self.pan = rect.size() * 0.5 - bounds.center().to_vec2() * self.zoom;
        } else {
            self.zoom = 1.;
            self.pan = Vec2::splat(64.);
        }
    }
    fn apply_style(&self, element: &mut Element) {
        element.stroke_color = color_string(self.stroke);
        element.background_color = color_string(self.fill);
        element.stroke_width = self.stroke_width;
        element.font_size = self.font_size;
        element.opacity = self.opacity;
    }
    fn read_style(&mut self, scene: &Scene, id: &str) {
        if let Some(element) = scene.elements.iter().find(|e| e.id == id) {
            self.stroke = parse_color(&element.stroke_color);
            self.fill = parse_color(&element.background_color);
            self.stroke_width = element.stroke_width;
            self.font_size = element.font_size;
            self.opacity = element.opacity;
        }
    }
    fn delete(&mut self, scene: &mut Scene) -> bool {
        let mut changed = false;
        for element in scene
            .elements
            .iter_mut()
            .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
        {
            element.is_deleted = true;
            element.touch();
            changed = true;
        }
        self.selected.clear();
        changed
    }
    fn duplicate(&mut self, scene: &mut Scene) -> bool {
        let copies: Vec<_> = scene
            .elements
            .iter()
            .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
            .cloned()
            .map(|mut e| {
                e.id = Element::new(e.kind.clone(), 0., 0., 1., 1.).id;
                e.x += 20.;
                e.y += 20.;
                for key in [
                    "boundElements",
                    "containerId",
                    "startBinding",
                    "endBinding",
                    "groupIds",
                    "frameId",
                    "index",
                ] {
                    e.extra.remove(key);
                }
                e.touch();
                e
            })
            .collect();
        let changed = !copies.is_empty();
        self.selected = copies.iter().map(|e| e.id.clone()).collect();
        scene.elements.extend(copies);
        changed
    }
    fn reorder(&self, scene: &mut Scene, front: bool) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let mut selected = Vec::new();
        let mut others = Vec::new();
        for element in scene.elements.drain(..) {
            if self.selected.contains(&element.id) {
                selected.push(element);
            } else {
                others.push(element);
            }
        }
        if front {
            others.append(&mut selected);
            scene.elements = others;
        } else {
            selected.append(&mut others);
            scene.elements = selected;
        }
        true
    }
    fn resize_hit(&self, scene: &Scene, pointer: Pos2, rect: Rect) -> Option<(String, usize)> {
        if self.selected.len() != 1 {
            return None;
        }
        let element = scene
            .elements
            .iter()
            .find(|e| self.selected.contains(&e.id) && !e.is_deleted)?;
        if element.kind == ElementKind::Text || element.angle.abs() > 0.001 {
            return None;
        }
        let b = bounds(element);
        for (index, point) in [
            b.left_top(),
            b.right_top(),
            b.right_bottom(),
            b.left_bottom(),
        ]
        .into_iter()
        .enumerate()
        {
            if self.screen(point, rect).distance(pointer) <= 9. {
                return Some((element.id.clone(), index));
            }
        }
        None
    }

    fn paint(&self, ui: &Ui, scene: &Scene, rect: Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let background = self.theme.canvas_background(ui, scene);
        let palette = self.theme.palette(ui);
        painter.rect_filled(rect, 0., background);
        let grid = 24. * self.zoom;
        if scene
            .app_state
            .get("codisseuGrid")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
            && grid >= 10.
        {
            let grid_color =
                if background.r() as u16 + background.g() as u16 + background.b() as u16 > 384 {
                    Color32::from_black_alpha(25)
                } else {
                    Color32::from_white_alpha(30)
                };
            let mut x = rect.left() + self.pan.x.rem_euclid(grid);
            while x < rect.right() {
                let mut y = rect.top() + self.pan.y.rem_euclid(grid);
                while y < rect.bottom() {
                    painter.circle_filled(egui::pos2(x, y), 0.65, grid_color);
                    y += grid;
                }
                x += grid;
            }
        }
        for element in scene.elements.iter().filter(|e| !e.is_deleted) {
            let b = bounds(element);
            let screen_bounds =
                Rect::from_two_pos(self.screen(b.min, rect), self.screen(b.max, rect));
            if !rect.intersects(screen_bounds.expand(16.)) {
                continue;
            }
            if !self
                .text_editor
                .as_ref()
                .is_some_and(|editor| editor.id.as_ref() == Some(&element.id))
            {
                self.paint_element(ui, &painter, element, rect);
            }
        }
        let selection_color = palette.accent;
        for element in scene
            .elements
            .iter()
            .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
        {
            let b = bounds(element);
            let b =
                Rect::from_two_pos(self.screen(b.min, rect), self.screen(b.max, rect)).expand(5.);
            painter.rect_stroke(
                b,
                2.,
                Stroke::new(1., selection_color),
                egui::StrokeKind::Middle,
            );
            if self.selected.len() == 1
                && element.kind != ElementKind::Text
                && element.angle.abs() < 0.001
            {
                let raw = Rect::from_two_pos(
                    self.screen(bounds(element).min, rect),
                    self.screen(bounds(element).max, rect),
                );
                for p in [
                    raw.left_top(),
                    raw.right_top(),
                    raw.right_bottom(),
                    raw.left_bottom(),
                ] {
                    painter.rect(
                        Rect::from_center_size(p, Vec2::splat(6.)),
                        1.,
                        palette.canvas,
                        Stroke::new(1., selection_color),
                        egui::StrokeKind::Middle,
                    );
                }
            }
        }
        if let Some(Gesture::Marquee { start, current, .. }) = &self.gesture {
            painter.rect(
                Rect::from_two_pos(self.screen(*start, rect), self.screen(*current, rect)),
                0.,
                selection_color.linear_multiply(0.12),
                Stroke::new(1., selection_color),
                egui::StrokeKind::Middle,
            );
        }
        let hint = if self.tool == Tool::Select {
            "Para mover a tela, segure Espaço ou arraste com o botão do meio"
        } else if self.tool == Tool::Text {
            "Clique para adicionar texto · Ctrl+Enter para aplicar"
        } else if self.tool == Tool::Eraser {
            "Clique ou arraste para apagar · Esc cancela"
        } else {
            "Arraste para desenhar · Shift mantém a proporção"
        };
        let galley = painter.layout(
            hint.into(),
            FontId::proportional(10.),
            palette.muted,
            (rect.width() - 32.).max(80.),
        );
        painter.galley(
            egui::pos2(rect.center().x - galley.size().x * 0.5, rect.top() + 66.),
            galley,
            palette.muted,
        );
        if scene.elements.iter().all(|e| e.is_deleted)
            && self.text_editor.is_none()
            && self.gesture.is_none()
            && matches!(self.tool, Tool::Select | Tool::Hand)
        {
            let y = rect.top() + rect.height() * 0.4;
            let logo = Rect::from_center_size(
                egui::pos2(rect.center().x, y - 34.),
                egui::vec2(260_f32.min(rect.width() - 32.), 40.),
            );
            crate::drawing_icons::paint(ui, logo, "logo", palette.accent);
            for (offset, text, size) in [
                (10., "Seus desenhos ficam nos arquivos deste projeto.", 14.),
                (38., "Salve para continuar ao reabrir o Codisseu.", 13.),
                (88., "Escolha uma ferramenta e comece a desenhar", 12.),
            ] {
                let galley = painter.layout(
                    text.into(),
                    FontId::proportional(size),
                    palette.muted,
                    (rect.width() - 40.).max(80.),
                );
                painter.galley(
                    egui::pos2(rect.center().x - galley.size().x * 0.5, y + offset),
                    galley,
                    palette.muted,
                );
            }
        }
    }

    fn paint_element(&self, ui: &Ui, painter: &egui::Painter, element: &Element, rect: Rect) {
        let opacity = (element.opacity / 100.).clamp(0., 1.) as f32;
        let stroke = Stroke::new(
            (element.stroke_width as f32 * self.zoom).max(0.25),
            self.theme
                .scene_color(ui, parse_color(&element.stroke_color))
                .linear_multiply(opacity),
        );
        let fill = self
            .theme
            .scene_color(ui, parse_color(&element.background_color))
            .linear_multiply(opacity);
        let local = Rect::from_two_pos(
            egui::pos2(element.x as f32, element.y as f32),
            egui::pos2(
                (element.x + element.width) as f32,
                (element.y + element.height) as f32,
            ),
        );
        let center = if element.points.is_empty() {
            local.center()
        } else {
            let [x1, y1, x2, y2] = element.bounds();
            egui::pos2(((x1 + x2) * 0.5) as f32, ((y1 + y2) * 0.5) as f32)
        };
        let rotate = |point: Pos2| {
            self.screen(
                center + egui::emath::Rot2::from_angle(element.angle as f32) * (point - center),
                rect,
            )
        };
        match element.kind {
            ElementKind::Rectangle | ElementKind::Diamond => {
                let points = if element.kind == ElementKind::Diamond {
                    vec![
                        local.center_top(),
                        local.right_center(),
                        local.center_bottom(),
                        local.left_center(),
                    ]
                } else {
                    vec![
                        local.left_top(),
                        local.right_top(),
                        local.right_bottom(),
                        local.left_bottom(),
                    ]
                };
                painter.add(egui::Shape::convex_polygon(
                    points.into_iter().map(rotate).collect(),
                    fill,
                    stroke,
                ));
            }
            ElementKind::Ellipse => {
                painter.add(egui::epaint::EllipseShape {
                    center: self.screen(center, rect),
                    radius: local.size() * (self.zoom * 0.5),
                    fill,
                    stroke,
                    angle: element.angle as f32,
                });
            }
            ElementKind::Arrow | ElementKind::Line | ElementKind::Freedraw => {
                let points: Vec<_> = element
                    .points
                    .iter()
                    .map(|p| {
                        rotate(egui::pos2(
                            element.x as f32 + p[0] as f32,
                            element.y as f32 + p[1] as f32,
                        ))
                    })
                    .collect();
                if points.len() > 1 {
                    painter.add(egui::Shape::line(points.clone(), stroke));
                    if element.kind == ElementKind::Arrow {
                        let head = element
                            .extra
                            .get("endArrowhead")
                            .map(|v| !v.is_null())
                            .unwrap_or(true);
                        if head {
                            paint_arrowhead(
                                painter,
                                points[points.len() - 2],
                                points[points.len() - 1],
                                stroke,
                                self.zoom,
                            );
                        }
                        if element
                            .extra
                            .get("startArrowhead")
                            .is_some_and(|v| !v.is_null())
                        {
                            paint_arrowhead(painter, points[1], points[0], stroke, self.zoom);
                        }
                    }
                }
            }
            ElementKind::Text => {
                let galley = painter.layout_no_wrap(
                    element.text.clone(),
                    FontId::proportional((element.font_size as f32 * self.zoom).clamp(1., 600.)),
                    stroke.color,
                );
                let shape = egui::epaint::TextShape::new(
                    self.screen(local.min, rect),
                    galley,
                    stroke.color,
                )
                .with_angle_and_anchor(element.angle as f32, Align2::CENTER_CENTER);
                painter.add(shape);
            }
            ElementKind::Other(ref name) => {
                let b = Rect::from_two_pos(
                    self.screen(bounds(element).min, rect),
                    self.screen(bounds(element).max, rect),
                );
                painter.rect_stroke(
                    b,
                    0.,
                    Stroke::new(1., Color32::GRAY),
                    egui::StrokeKind::Middle,
                );
                painter.text(
                    b.center(),
                    Align2::CENTER_CENTER,
                    format!("{name} · preservado"),
                    FontId::proportional(12.),
                    Color32::GRAY,
                );
            }
        }
    }
}

fn island<R>(
    parent: &Ui,
    name: &str,
    position: Pos2,
    palette: &ThemePalette,
    padding: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Rect {
    egui::Area::new(parent.scope_id().with(name))
        .order(egui::Order::Foreground)
        .fixed_pos(position)
        .movable(false)
        .constrain_to(parent.clip_rect().intersect(parent.max_rect()).shrink(4.))
        .constrain(true)
        .show(parent.ctx(), |ui| {
            ui.set_clip_rect(parent.clip_rect().intersect(parent.max_rect()));
            if !parent.is_enabled() {
                ui.disable();
            }
            let visuals = ui.visuals_mut();
            visuals.dark_mode = palette.canvas.r() < 128;
            visuals.override_text_color = Some(palette.text);
            visuals.panel_fill = palette.panel;
            visuals.window_fill = palette.panel;
            visuals.extreme_bg_color = palette.canvas;
            visuals.faint_bg_color = palette.hover;
            visuals.selection.bg_fill = palette.active;
            visuals.selection.stroke = Stroke::new(1., palette.accent);
            visuals.widgets.inactive.bg_fill = palette.hover;
            visuals.widgets.hovered.bg_fill = palette.hover;
            visuals.widgets.active.bg_fill = palette.active;
            ui.spacing_mut().item_spacing = egui::vec2(4., 6.);
            ui.spacing_mut().button_padding = egui::vec2(8., 4.);
            ui.spacing_mut().interact_size.y = 28.;
            egui::Frame::NONE
                .fill(palette.panel)
                .stroke(Stroke::new(1., palette.border))
                .corner_radius(8.)
                .inner_margin(padding)
                .shadow(egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 10,
                    spread: 0,
                    color: Color32::from_black_alpha(22),
                })
                .show(ui, contents);
        })
        .response
        .rect
}
fn glyph_button(
    ui: &mut Ui,
    name: &str,
    label: &str,
    selected: bool,
    shortcut: Option<&str>,
    palette: &ThemePalette,
) -> egui::Response {
    let response = ui
        .push_id(label, |ui| {
            ui.add(
                egui::Button::new("")
                    .frame(false)
                    .min_size(egui::vec2(32., 32.)),
            )
        })
        .inner;
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::accesskit::Role::Button,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            response.rect,
            5.,
            if selected {
                palette.active
            } else {
                palette.hover
            },
        );
    }
    let color = if selected {
        palette.accent
    } else {
        palette.text
    };
    crate::drawing_icons::paint(
        ui,
        Rect::from_center_size(
            response.rect.center() - egui::vec2(0., if shortcut.is_some() { 1. } else { 0. }),
            Vec2::splat(16.),
        ),
        name,
        color,
    );
    if let Some(shortcut) = shortcut {
        ui.painter().text(
            response.rect.right_bottom() - egui::vec2(3., 2.),
            Align2::RIGHT_BOTTOM,
            shortcut,
            FontId::proportional(8.),
            palette.muted,
        );
    }
    response.on_hover_text(label)
}
fn menu_item(ui: &mut Ui, name: &str, label: &str, shortcut: &str, palette: &ThemePalette) -> bool {
    let response = ui.add(
        egui::Button::new("")
            .frame(false)
            .min_size(egui::vec2(218., 34.)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
    });
    if response.hovered() {
        ui.painter().rect_filled(response.rect, 5., palette.hover);
    }
    crate::drawing_icons::paint(
        ui,
        Rect::from_center_size(
            egui::pos2(response.rect.left() + 18., response.rect.center().y),
            Vec2::splat(16.),
        ),
        name,
        palette.text,
    );
    ui.painter().text(
        egui::pos2(response.rect.left() + 38., response.rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.),
        palette.text,
    );
    ui.painter().text(
        egui::pos2(response.rect.right() - 8., response.rect.center().y),
        Align2::RIGHT_CENTER,
        shortcut,
        FontId::proportional(10.),
        palette.muted,
    );
    response.clicked()
}
fn color_palette(
    ui: &mut Ui,
    color: &mut Color32,
    fill: bool,
    theme: CanvasTheme,
    palette: &ThemePalette,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let values = if fill {
            [
                Color32::TRANSPARENT,
                Color32::from_rgb(255, 201, 201),
                Color32::from_rgb(178, 242, 187),
                Color32::from_rgb(165, 216, 255),
                Color32::from_rgb(255, 236, 153),
            ]
        } else {
            [
                Color32::from_rgb(30, 30, 30),
                Color32::from_rgb(224, 49, 49),
                Color32::from_rgb(47, 158, 68),
                Color32::from_rgb(25, 113, 194),
                Color32::from_rgb(240, 140, 0),
            ]
        };
        for value in values {
            let r = ui.add(
                egui::Button::new("")
                    .min_size(egui::vec2(21., 21.))
                    .fill(theme.scene_color(ui, value))
                    .stroke(Stroke::new(
                        if *color == value { 2. } else { 1. },
                        if *color == value {
                            palette.accent
                        } else {
                            palette.border
                        },
                    ))
                    .corner_radius(4.),
            );
            r.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    *color == value,
                    format!(
                        "{} {}",
                        if fill {
                            "Cor do fundo"
                        } else {
                            "Cor do traço"
                        },
                        color_string(value)
                    ),
                )
            });
            if value == Color32::TRANSPARENT {
                ui.painter().line_segment(
                    [
                        r.rect.left_bottom() + egui::vec2(3., -3.),
                        r.rect.right_top() + egui::vec2(-3., 3.),
                    ],
                    Stroke::new(1., palette.muted),
                );
            }
            if r.clicked() {
                *color = value;
                changed = true;
            }
        }
        ui.separator();
        changed |= ui
            .color_edit_button_srgba(color)
            .on_hover_text("Escolher cor personalizada")
            .changed();
    });
    changed
}
fn paint_arrowhead(painter: &egui::Painter, from: Pos2, end: Pos2, stroke: Stroke, zoom: f32) {
    let direction = (from - end).normalized();
    let normal = egui::vec2(-direction.y, direction.x);
    let length = (12. * zoom).max(stroke.width * 3.);
    painter.add(egui::Shape::line(
        vec![
            end + direction * length + normal * length * 0.45,
            end,
            end + direction * length - normal * length * 0.45,
        ],
        stroke,
    ));
}
fn parse_color(value: &str) -> Color32 {
    if value == "transparent" || value.is_empty() {
        return Color32::TRANSPARENT;
    }
    let hex = value.trim_start_matches('#');
    let byte = |i| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    if hex.is_ascii()
        && (hex.len() == 6 || hex.len() == 8)
        && let (Some(r), Some(g), Some(b)) = (byte(0), byte(2), byte(4))
    {
        return Color32::from_rgba_unmultiplied(
            r,
            g,
            b,
            if hex.len() == 8 {
                byte(6).unwrap_or(255)
            } else {
                255
            },
        );
    } else if hex.is_ascii()
        && hex.len() == 3
        && let Ok(n) = u16::from_str_radix(hex, 16)
    {
        return Color32::from_rgb(
            ((n >> 8) as u8) * 17,
            (((n >> 4) & 15) as u8) * 17,
            ((n & 15) as u8) * 17,
        );
    }
    match value {
        "white" => Color32::WHITE,
        "red" => Color32::RED,
        "blue" => Color32::BLUE,
        _ => Color32::BLACK,
    }
}
fn color_string(color: Color32) -> String {
    if color.a() == 0 {
        "transparent".into()
    } else if color.a() == 255 {
        format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b())
    } else {
        let [r, g, b, a] = color.to_srgba_unmultiplied();
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}
fn bounds(element: &Element) -> Rect {
    let local = if !element.points.is_empty() {
        element
            .points
            .iter()
            .map(|p| {
                egui::pos2(
                    element.x as f32 + p[0] as f32,
                    element.y as f32 + p[1] as f32,
                )
            })
            .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)))
    } else {
        Rect::from_two_pos(
            egui::pos2(element.x as f32, element.y as f32),
            egui::pos2(
                (element.x + element.width) as f32,
                (element.y + element.height) as f32,
            ),
        )
    };
    if element.angle.abs() < 0.0001 {
        local
    } else {
        let c = local.center();
        let rot = egui::emath::Rot2::from_angle(element.angle as f32);
        [
            local.left_top(),
            local.right_top(),
            local.right_bottom(),
            local.left_bottom(),
        ]
        .into_iter()
        .map(|p| c + rot * (p - c))
        .fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)))
    }
}
fn hit(scene: &Scene, point: Pos2, tolerance: f32) -> Option<String> {
    scene
        .elements
        .iter()
        .rev()
        .filter(|e| !e.is_deleted)
        .find(|e| {
            let b = bounds(e);
            if !b.expand(tolerance).contains(point) {
                return false;
            }
            if matches!(
                e.kind,
                ElementKind::Arrow | ElementKind::Line | ElementKind::Freedraw
            ) && e.points.len() > 1
            {
                let [x1, y1, x2, y2] = e.bounds();
                let center = egui::pos2(((x1 + x2) * 0.5) as f32, ((y1 + y2) * 0.5) as f32);
                let rot = egui::emath::Rot2::from_angle(e.angle as f32);
                let points: Vec<_> = e
                    .points
                    .iter()
                    .map(|p| {
                        center
                            + rot
                                * (egui::pos2(e.x as f32 + p[0] as f32, e.y as f32 + p[1] as f32)
                                    - center)
                    })
                    .collect();
                return points.windows(2).any(|p| {
                    segment_distance(point, p[0], p[1])
                        <= tolerance.max(e.stroke_width as f32 * 0.5)
                });
            }
            true
        })
        .map(|e| e.id.clone())
}
fn segment_distance(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let segment = b - a;
    let t = ((point - a).dot(segment) / segment.length_sq().max(0.0001)).clamp(0., 1.);
    point.distance(a + segment * t)
}
fn draw_preview(element: &mut Element, start: Pos2, mut current: Pos2, constrained: bool) {
    let mut delta = current - start;
    if constrained && !matches!(element.kind, ElementKind::Freedraw) {
        let m = delta.x.abs().max(delta.y.abs());
        if matches!(element.kind, ElementKind::Arrow | ElementKind::Line) {
            if delta.x.abs() > delta.y.abs() * 2. {
                delta.y = 0.;
            } else if delta.y.abs() > delta.x.abs() * 2. {
                delta.x = 0.;
            } else {
                delta = egui::vec2(m * delta.x.signum(), m * delta.y.signum());
            }
        } else {
            delta = egui::vec2(m * delta.x.signum(), m * delta.y.signum());
        }
        current = start + delta;
    }
    if matches!(
        element.kind,
        ElementKind::Arrow | ElementKind::Line | ElementKind::Freedraw
    ) {
        element.width = delta.x.abs() as f64;
        element.height = delta.y.abs() as f64;
        if element.kind == ElementKind::Freedraw {
            if element.points.is_empty() {
                element.points.push([0., 0.]);
            }
            let p = [delta.x as f64, delta.y as f64];
            if element.points.len() < 50_000
                && element
                    .points
                    .last()
                    .is_none_or(|old| (old[0] - p[0]).hypot(old[1] - p[1]) > 0.5)
            {
                element.points.push(p);
            }
            let b = bounds(element);
            element.width = b.width() as f64;
            element.height = b.height() as f64;
        } else {
            element.points = vec![[0., 0.], [delta.x as f64, delta.y as f64]];
        }
    } else {
        let b = Rect::from_two_pos(start, current);
        element.x = b.left() as f64;
        element.y = b.top() as f64;
        element.width = b.width() as f64;
        element.height = b.height() as f64;
    }
}
fn resize(
    element: &mut Element,
    original: &Element,
    corner: usize,
    delta: Vec2,
    constrained: bool,
) {
    let b = bounds(original);
    let corners = [
        b.left_top(),
        b.right_top(),
        b.right_bottom(),
        b.left_bottom(),
    ];
    let anchor = corners[(corner + 2) % 4];
    let mut moving = corners[corner] + delta;
    if constrained {
        let size = moving - anchor;
        let aspect = b.width() / b.height().max(0.01);
        moving = anchor + egui::vec2(size.x, size.x.abs() / aspect * size.y.signum());
    }
    let next = Rect::from_two_pos(anchor, moving);
    let sx = next.width().max(1.) / b.width().max(1.);
    let sy = next.height().max(1.) / b.height().max(1.);
    element.x = next.min.x as f64;
    element.y = next.min.y as f64;
    element.width = next.width().max(1.) as f64;
    element.height = next.height().max(1.) as f64;
    if !original.points.is_empty() {
        element.points = original
            .points
            .iter()
            .map(|p| {
                [
                    ((original.x + p[0]) as f32 - b.min.x) as f64 * sx as f64,
                    ((original.y + p[1]) as f32 - b.min.y) as f64 * sy as f64,
                ]
            })
            .collect();
    }
}
fn text_dimensions(ui: &Ui, element: &mut Element) {
    // Measure with the same font/shaping as the canvas, in scene coordinates.
    let size = ui
        .painter()
        .layout_no_wrap(
            element.text.clone(),
            FontId::proportional(element.font_size as f32),
            Color32::BLACK,
        )
        .size();
    element.width = size.x.max(1.) as f64;
    element.height = size.y.max(element.font_size as f32) as f64;
    element
        .extra
        .insert("originalText".into(), element.text.clone().into());
    element.extra.insert("lineHeight".into(), 1.25.into());
}

#[cfg(test)]
#[path = "drawing_canvas_tests.rs"]
mod tests;
