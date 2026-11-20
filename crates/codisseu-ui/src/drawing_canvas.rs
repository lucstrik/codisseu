//! Native drawing surface for the interoperable Excalidraw scene format.
//! Gestures update the preview; only completed edits are returned to the common
//! document service, which owns undo, persistence and external-file conflicts.
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
        }
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
            Self::Select | Self::Hand => return None,
        })
    }
}

#[derive(Default)]
pub(super) struct CanvasResponse {
    /// One completed user edit. The caller serializes the scene once here.
    pub changed: bool,
    pub undo: bool,
    pub redo: bool,
}

enum Gesture {
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
    }

    pub(super) fn show(&mut self, ui: &mut Ui, scene: &mut Scene) -> CanvasResponse {
        let mut result = CanvasResponse::default();
        self.selected
            .retain(|id| scene.elements.iter().any(|e| &e.id == id && !e.is_deleted));
        self.toolbar(ui, &mut result);
        self.properties(ui, scene, &mut result);
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
        if !ui.is_enabled() {
            self.cancel_gesture(scene);
            self.text_editor = None;
            self.paint(ui, scene, rect);
            return result;
        }
        let editor_open = self.text_editor.is_some();
        if !editor_open {
            if self.gesture.is_some()
                && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
            {
                self.cancel_gesture(scene);
            }
            if response.clicked() || response.drag_started() {
                response.request_focus();
            }
            if response.has_focus() {
                self.keyboard(ui, scene, &mut result);
            }
            if response.hovered() {
                let (scroll, zoom, pointer) =
                    ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
                if (zoom - 1.).abs() > 0.001 {
                    self.zoom_at(zoom, pointer.unwrap_or(rect.center()), rect);
                } else if scroll != Vec2::ZERO {
                    self.pan += scroll;
                }
            }
            self.pointer(ui, scene, rect, &response, &mut result);
        }
        self.paint(ui, scene, rect);
        if response.hovered() {
            let hand = self.tool == Tool::Hand || ui.input(|i| i.key_down(Key::Space));
            ui.ctx().set_cursor_icon(if hand {
                egui::CursorIcon::Grab
            } else if self.tool == Tool::Select {
                egui::CursorIcon::Default
            } else {
                egui::CursorIcon::Crosshair
            });
        }
        self.text_window(ui, scene, &mut result);
        result
    }

    fn toolbar(&mut self, ui: &mut Ui, result: &mut CanvasResponse) {
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("drawing.tools")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().button_padding = egui::vec2(10., 6.);
                            for tool in [
                                Tool::Select,
                                Tool::Hand,
                                Tool::Rectangle,
                                Tool::Ellipse,
                                Tool::Diamond,
                                Tool::Arrow,
                                Tool::Line,
                                Tool::Freedraw,
                                Tool::Text,
                            ] {
                                let response = ui
                                    .push_id(tool.label(), |ui| {
                                        ui.add(
                                            egui::Button::new(" ")
                                                .selected(self.tool == tool)
                                                .min_size(egui::vec2(34., 30.)),
                                        )
                                    })
                                    .inner;
                                response.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::accesskit::Role::Button,
                                        ui.is_enabled(),
                                        self.tool == tool,
                                        tool.label(),
                                    )
                                });
                                paint_tool(
                                    ui.painter(),
                                    response.rect.shrink(8.),
                                    tool,
                                    ui.visuals().text_color(),
                                );
                                if response.clicked() {
                                    self.tool = tool;
                                }
                                response.on_hover_text(tool.label());
                            }
                            ui.separator();
                            if ui.button("Desfazer").clicked() {
                                result.undo = true;
                            }
                            if ui.button("Refazer").clicked() {
                                result.redo = true;
                            }
                            ui.separator();
                            if ui.button("−").on_hover_text("Diminuir zoom").clicked() {
                                self.zoom_at(0.8, self.canvas_rect.center(), self.canvas_rect);
                            }
                            ui.label(format!("{:.0}%", self.zoom * 100.));
                            if ui.button("+").on_hover_text("Aumentar zoom").clicked() {
                                self.zoom_at(1.25, self.canvas_rect.center(), self.canvas_rect);
                            }
                            if ui
                                .button("Enquadrar")
                                .on_hover_text("Mostrar todo o desenho · Shift+1")
                                .clicked()
                            {
                                self.fit_pending = true;
                            }
                        });
                    });
            });
    }

    fn properties(&mut self, ui: &mut Ui, scene: &mut Scene, result: &mut CanvasResponse) {
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("drawing.properties")
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().button_padding = egui::vec2(10., 5.);
                            ui.label("Traço");
                            let stroke_changed = ui
                                .color_edit_button_srgba(&mut self.stroke)
                                .on_hover_text("Cor do traço e do texto")
                                .changed();
                            ui.label("Fundo");
                            let fill_changed = ui
                                .color_edit_button_srgba(&mut self.fill)
                                .on_hover_text("Cor do preenchimento · transparência disponível")
                                .changed();
                            let width_changed = ui
                                .add(
                                    egui::DragValue::new(&mut self.stroke_width)
                                        .range(0.5..=16.)
                                        .speed(0.25)
                                        .prefix("Espessura "),
                                )
                                .changed();
                            let font_changed = ui
                                .add(
                                    egui::DragValue::new(&mut self.font_size)
                                        .range(8.0..=144.)
                                        .speed(1.)
                                        .prefix("Texto "),
                                )
                                .changed();
                            if stroke_changed || fill_changed || width_changed || font_changed {
                                for element in scene
                                    .elements
                                    .iter_mut()
                                    .filter(|e| self.selected.contains(&e.id) && !e.is_deleted)
                                {
                                    if stroke_changed {
                                        element.stroke_color = color_string(self.stroke);
                                    }
                                    if fill_changed {
                                        element.background_color = color_string(self.fill);
                                    }
                                    if width_changed {
                                        element.stroke_width = self.stroke_width;
                                    }
                                    if font_changed && element.kind == ElementKind::Text {
                                        element.font_size = self.font_size;
                                        text_dimensions(ui, element);
                                    }
                                    element.touch();
                                    result.changed = true;
                                }
                            }
                            if !self.selected.is_empty() {
                                ui.separator();
                                ui.label(format!("{} selecionado(s)", self.selected.len()));
                                if self.selected.len() == 1
                                    && let Some(element) = scene.elements.iter().find(|e| {
                                        self.selected.contains(&e.id) && e.kind == ElementKind::Text
                                    })
                                    && ui.button("Editar texto").clicked()
                                {
                                    self.text_editor = Some(TextEditor {
                                        id: Some(element.id.clone()),
                                        position: egui::pos2(element.x as f32, element.y as f32),
                                        text: element.text.clone(),
                                        focus: true,
                                    });
                                }
                                if ui.button("Duplicar").clicked() {
                                    result.changed |= self.duplicate(scene);
                                }
                                if ui.button("Excluir").clicked() {
                                    result.changed |= self.delete(scene);
                                }
                                if ui.button("Trazer à frente").clicked() {
                                    result.changed |= self.reorder(scene, true);
                                }
                                if ui.button("Enviar atrás").clicked() {
                                    result.changed |= self.reorder(scene, false);
                                }
                            } else {
                                ui.weak("Arraste para desenhar · Espaço para mover · Ctrl+roda para zoom");
                            }
                        });
                    });
            });
    }

    fn cancel_gesture(&mut self, scene: &mut Scene) {
        match self.gesture.take() {
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
            match &mut self.gesture {
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
        if response.clicked_by(PointerButton::Primary) {
            if self.tool == Tool::Text {
                self.text_editor = Some(TextEditor {
                    id: None,
                    position: world,
                    text: String::new(),
                    focus: true,
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
                            position: world,
                            text: element.text.clone(),
                            focus: true,
                        });
                    }
                } else {
                    self.selected.clear();
                }
            }
        }
    }

    fn text_window(&mut self, ui: &Ui, scene: &mut Scene, result: &mut CanvasResponse) {
        let Some(editor) = &mut self.text_editor else {
            return;
        };
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Texto do desenho")
            .id(ui.scope_id().with("drawing.text"))
            .collapsible(false)
            .resizable(true)
            .default_width(380.)
            .show(ui.ctx(), |ui| {
                ui.spacing_mut().button_padding = egui::vec2(12., 6.);
                let field = ui.add(
                    egui::TextEdit::multiline(&mut editor.text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text("Digite o texto…")
                        .margin(egui::vec2(10., 8.)),
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
                ui.horizontal(|ui| {
                    apply = ui.button("Aplicar texto").clicked();
                    cancel = ui.button("Cancelar texto").clicked();
                });
                ui.input_mut(|i| {
                    cancel |= i.consume_key(Modifiers::NONE, Key::Escape);
                    apply |= i.consume_key(Modifiers::CTRL, Key::Enter);
                });
            });
        if apply {
            let editor = self.text_editor.take().unwrap();
            if let Some(id) = editor.id {
                if let Some(element) = scene.elements.iter_mut().find(|e| e.id == id)
                    && element.text != editor.text
                {
                    element.text = editor.text;
                    text_dimensions(ui, element);
                    element.touch();
                    result.changed = true;
                }
            } else if !editor.text.trim().is_empty() {
                let mut element = Element::new(
                    ElementKind::Text,
                    editor.position.x as f64,
                    editor.position.y as f64,
                    0.,
                    0.,
                );
                self.apply_style(&mut element);
                element.text = editor.text;
                text_dimensions(ui, &mut element);
                element.touch();
                self.selected.clear();
                self.selected.insert(element.id.clone());
                scene.elements.push(element);
                result.changed = true;
            }
            self.tool = Tool::Select;
        } else if cancel {
            self.text_editor = None;
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
    }
    fn read_style(&mut self, scene: &Scene, id: &str) {
        if let Some(element) = scene.elements.iter().find(|e| e.id == id) {
            self.stroke = parse_color(&element.stroke_color);
            self.fill = parse_color(&element.background_color);
            self.stroke_width = element.stroke_width;
            self.font_size = element.font_size;
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
        let background = scene
            .app_state
            .get("viewBackgroundColor")
            .and_then(|v| v.as_str())
            .map(parse_color)
            .filter(|c| *c != Color32::TRANSPARENT)
            .unwrap_or(Color32::from_rgb(250, 250, 250));
        painter.rect_filled(rect, 0., background);
        let grid = 24. * self.zoom;
        if grid >= 10. {
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
            self.paint_element(&painter, element, rect);
        }
        let selection_color = ui.visuals().selection.bg_fill;
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
                        Rect::from_center_size(p, Vec2::splat(7.)),
                        1.,
                        Color32::WHITE,
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
        if scene.elements.iter().all(|e| e.is_deleted) {
            let fg = if background.r() as u16 + background.g() as u16 + background.b() as u16 > 384
            {
                Color32::from_gray(100)
            } else {
                Color32::from_gray(170)
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Escolha uma ferramenta e arraste para começar",
                FontId::proportional(15.),
                fg,
            );
        }
    }

    fn paint_element(&self, painter: &egui::Painter, element: &Element, rect: Rect) {
        let opacity = (element.opacity / 100.).clamp(0., 1.) as f32;
        let stroke = Stroke::new(
            (element.stroke_width as f32 * self.zoom).max(0.25),
            parse_color(&element.stroke_color).linear_multiply(opacity),
        );
        let fill = parse_color(&element.background_color).linear_multiply(opacity);
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

fn paint_tool(painter: &egui::Painter, rect: Rect, tool: Tool, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    let c = rect.center();
    match tool {
        Tool::Select => {
            painter.add(egui::Shape::closed_line(
                vec![
                    rect.left_top(),
                    rect.left_bottom(),
                    c,
                    rect.right_center(),
                    rect.left_top(),
                ],
                stroke,
            ));
        }
        Tool::Hand => {
            painter.circle_stroke(c, rect.width() * 0.3, stroke);
            for x in [-0.25, 0., 0.25] {
                let p = egui::pos2(c.x + x * rect.width(), c.y);
                painter.line_segment([p, p - egui::vec2(0., 6.)], stroke);
            }
        }
        Tool::Rectangle => {
            painter.rect_stroke(rect, 1., stroke, egui::StrokeKind::Middle);
        }
        Tool::Ellipse => {
            painter.add(egui::epaint::EllipseShape::stroke(
                c,
                rect.size() * 0.5,
                stroke,
            ));
        }
        Tool::Diamond => {
            painter.add(egui::Shape::closed_line(
                vec![
                    rect.center_top(),
                    rect.right_center(),
                    rect.center_bottom(),
                    rect.left_center(),
                ],
                stroke,
            ));
        }
        Tool::Arrow | Tool::Line => {
            painter.line_segment([rect.left_bottom(), rect.right_top()], stroke);
            if tool == Tool::Arrow {
                paint_arrowhead(painter, rect.left_bottom(), rect.right_top(), stroke, 0.7);
            }
        }
        Tool::Freedraw => {
            painter.add(egui::Shape::line(
                vec![
                    rect.left_bottom(),
                    egui::pos2(rect.left() + 3., rect.top() + 3.),
                    egui::pos2(rect.right() - 3., rect.bottom() - 3.),
                    rect.right_top(),
                ],
                stroke,
            ));
        }
        Tool::Text => {
            painter.text(
                c,
                Align2::CENTER_CENTER,
                "T",
                FontId::proportional(20.),
                color,
            );
        }
    }
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
