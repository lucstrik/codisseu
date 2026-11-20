use super::*;
use egui_kittest::{Harness, kittest::Queryable};

#[derive(Default)]
struct State {
    canvas: DrawingCanvasUi,
    scene: Scene,
    commits: usize,
    undo: usize,
    redo: usize,
    theme_applied: bool,
}
fn harness() -> Harness<'static, State> {
    Harness::builder().with_size([1000., 720.]).build_ui_state(
        |ui, state: &mut State| {
            if !state.theme_applied {
                super::super::Tokens::load().apply(ui.ctx());
                state.theme_applied = true;
            }
            let response = state.canvas.show(ui, &mut state.scene);
            state.commits += usize::from(response.changed);
            state.undo += usize::from(response.undo);
            state.redo += usize::from(response.redo);
        },
        State::default(),
    )
}
fn drag(h: &mut Harness<'static, State>, start: Pos2, end: Pos2) {
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.step();
    h.hover_at(end);
    h.step();
    h.drop_at(end);
    h.run();
}
#[test]
fn draw_drag_commits_once_and_native_selection_moves_resizes_duplicates_deletes() {
    let mut h = harness();
    h.run();
    h.get_by_label(Tool::Rectangle.label()).click();
    h.run();
    let start = h.state().canvas.canvas_rect.min + egui::vec2(150., 120.);
    drag(&mut h, start, start + egui::vec2(180., 100.));
    assert_eq!(h.state().scene.elements.len(), 1);
    assert_eq!(
        h.state().commits,
        1,
        "one gesture must be one document edit"
    );
    let original = h.state().scene.elements[0].clone();
    assert_eq!(original.kind, ElementKind::Rectangle);
    assert_eq!(original.width, 180.);
    assert_eq!(original.height, 100.);
    h.get_by_label(Tool::Select.label()).click();
    h.run();
    drag(
        &mut h,
        start + egui::vec2(80., 40.),
        start + egui::vec2(110., 60.),
    );
    assert_eq!(h.state().scene.elements[0].x, original.x + 30.);
    assert_eq!(h.state().scene.elements[0].y, original.y + 20.);
    let corner = start + egui::vec2(210., 120.);
    drag(&mut h, corner, corner + egui::vec2(40., 30.));
    assert_eq!(h.state().scene.elements[0].width, 220.);
    assert_eq!(h.state().scene.elements[0].height, 130.);
    h.get_by_label("Duplicar").click();
    h.run();
    assert_eq!(h.state().scene.elements.len(), 2);
    assert_ne!(
        h.state().scene.elements[0].id,
        h.state().scene.elements[1].id
    );
    h.get_by_label("Excluir").click();
    h.run();
    assert!(!h.state().scene.elements[0].is_deleted);
    assert!(h.state().scene.elements[1].is_deleted);
}
#[test]
fn arrow_hit_test_follows_segments_and_unknown_elements_keep_payload() {
    let mut scene = Scene::default();
    let mut arrow = Element::new(ElementKind::Arrow, 0., 0., 100., 100.);
    arrow.points = vec![[0., 0.], [100., 100.]];
    let id = arrow.id.clone();
    scene.elements.push(arrow);
    assert_eq!(hit(&scene, egui::pos2(50., 50.), 5.), Some(id));
    assert_eq!(hit(&scene, egui::pos2(5., 95.), 5.), None);
    let mut unknown = Element::new(ElementKind::Other("image".into()), 200., 100., 150., 100.);
    unknown
        .extra
        .insert("fileId".into(), "preserve-image-payload".into());
    let id = unknown.id.clone();
    scene.elements.push(unknown);
    let mut canvas = DrawingCanvasUi::default();
    canvas.selected.insert(id);
    assert!(canvas.duplicate(&mut scene));
    assert_eq!(
        scene.elements[2].extra.get("fileId").unwrap(),
        "preserve-image-payload"
    );
}
#[test]
fn tools_create_shapes_and_negative_line_geometry_and_fit_keep_coordinates() {
    for tool in [
        Tool::Ellipse,
        Tool::Diamond,
        Tool::Arrow,
        Tool::Line,
        Tool::Freedraw,
    ] {
        let mut h = harness();
        h.run();
        h.get_by_label(tool.label()).click();
        h.run();
        let start = h.state().canvas.canvas_rect.center();
        drag(&mut h, start, start - egui::vec2(120., 70.));
        let element = &h.state().scene.elements[0];
        assert_eq!(element.kind, tool.kind().unwrap());
        assert!(element.width >= 120. && element.height >= 70.);
        assert_eq!(h.state().commits, 1);
        let before = h.state().scene.to_json().unwrap();
        h.get_by_label("Enquadrar").click();
        h.run();
        assert_eq!(h.state().scene.to_json().unwrap(), before);
        assert_eq!(h.state().commits, 1, "view changes do not edit drawing");
    }
}
#[test]
fn text_editor_applies_unicode_text_and_cancel_does_not_modify_scene() {
    let mut h = harness();
    h.run();
    h.get_by_label(Tool::Text.label()).click();
    h.run();
    let point = h.state().canvas.canvas_rect.center();
    h.hover_at(point);
    h.drag_at(point);
    h.drop_at(point);
    h.run();
    h.event(egui::Event::Text("Arquitetura café 🦀\nAPI e banco".into()));
    h.run();
    h.get_by_label("Aplicar texto").click();
    h.run();
    assert_eq!(h.state().scene.elements.len(), 1);
    assert_eq!(
        h.state().scene.elements[0].text,
        "Arquitetura café 🦀\nAPI e banco"
    );
    assert_eq!(h.state().commits, 1);
    h.get_by_label("Editar texto").click();
    h.run();
    h.event(egui::Event::Text("não salvar".into()));
    h.run();
    h.get_by_label("Cancelar texto").click();
    h.run();
    assert_eq!(
        h.state().scene.elements[0].text,
        "Arquitetura café 🦀\nAPI e banco"
    );
    assert_eq!(h.state().commits, 1);
}
#[test]
fn undo_and_redo_request_common_document_history_without_private_stack() {
    let mut h = harness();
    h.run();
    h.get_by_label("Desfazer").click();
    h.run();
    h.get_by_label("Refazer").click();
    h.run();
    assert_eq!(h.state().undo, 1);
    assert_eq!(h.state().redo, 1);
    assert_eq!(h.state().commits, 0);
}
#[test]
fn zoom_about_pointer_preserves_world_anchor_and_color_parsing_is_bounded() {
    let mut canvas = DrawingCanvasUi::default();
    let rect = Rect::from_min_size(egui::pos2(80., 120.), egui::vec2(900., 600.));
    let pointer = egui::pos2(400., 400.);
    let world = canvas.world(pointer, rect);
    canvas.zoom_at(1.25, pointer, rect);
    assert!(canvas.world(pointer, rect).distance(world) < 0.001);
    assert_eq!(parse_color("#abc"), Color32::from_rgb(170, 187, 204));
    assert_eq!(parse_color("#ééé"), Color32::BLACK);
    assert_eq!(parse_color("transparent"), Color32::TRANSPARENT);
}

#[test]
fn escape_discards_uncommitted_preview_and_revision_change_closes_text_editor() {
    let mut h = harness();
    h.run();
    h.get_by_label(Tool::Rectangle.label()).click();
    h.run();
    let start = h.state().canvas.canvas_rect.center();
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.step();
    h.hover_at(start + egui::vec2(80., 70.));
    h.step();
    assert_eq!(h.state().scene.elements.len(), 1);
    assert_eq!(h.state().commits, 0);
    h.key_press(Key::Escape);
    h.step();
    h.drop_at(start + egui::vec2(80., 70.));
    h.run();
    assert!(h.state().scene.elements.is_empty());
    assert_eq!(h.state().commits, 0);
    h.state_mut().canvas.text_editor = Some(TextEditor {
        id: None,
        position: Pos2::ZERO,
        text: "obsolete draft".into(),
        focus: true,
    });
    h.state_mut().canvas.cancel_interaction();
    assert!(h.state().canvas.text_editor.is_none());
}

#[test]
fn coalesced_final_motion_and_release_use_full_drag_and_canvas_geometry_stays_fixed() {
    let mut h = harness();
    h.run();
    h.get_by_label(Tool::Rectangle.label()).click();
    h.run();
    let canvas = h.state().canvas.canvas_rect;
    let start = canvas.min + egui::vec2(120., 100.);
    h.hover_at(start);
    h.run();
    h.drag_at(start);
    h.step();
    h.hover_at(start + egui::vec2(17., 10.));
    h.step();
    h.step();
    assert_eq!(
        h.state().canvas.canvas_rect,
        canvas,
        "selection tools must not move the canvas during a gesture"
    );
    let end = start + egui::vec2(200., 120.);
    h.hover_at(end);
    h.drop_at(end);
    h.run();
    let element = &h.state().scene.elements[0];
    assert_eq!(element.width, 200.);
    assert_eq!(element.height, 120.);
    assert_eq!(h.state().commits, 1);
}
