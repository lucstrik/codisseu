use codisseu_draw::{DrawingDocument, Element, ElementKind, Scene};
use serde_json::json;

fn rectangle() -> Element {
    Element::new(ElementKind::Rectangle, 10.0, 20.0, 140.0, 70.0)
}

#[test]
fn excalidraw_roundtrip_preserves_unknown_elements_assets_and_properties() {
    let input = json!({"type":"excalidraw","version":2,"source":"https://excalidraw.com","customScene":{"future":true},"elements":[{"id":"a","type":"image","x":2,"y":3,"width":40,"height":50,"fileId":"asset","futureField":[1,2],"created":null}],"appState":{"customPreference":17},"files":{"asset":{"dataURL":"data:image/png;base64,AA==","mimeType":"image/png"}}});
    let mut scene = Scene::from_json(&input.to_string()).unwrap();
    scene.elements.push(rectangle());
    let exported: serde_json::Value = serde_json::from_str(&scene.to_json().unwrap()).unwrap();
    assert_eq!(exported["customScene"], input["customScene"]);
    assert_eq!(exported["elements"][0]["fileId"], "asset");
    assert_eq!(exported["elements"][0]["futureField"], json!([1, 2]));
    assert_eq!(exported["elements"][0]["created"], serde_json::Value::Null);
    assert_eq!(exported["files"], input["files"]);
    assert_eq!(exported["appState"], input["appState"]);
    assert_eq!(scene.unsupported_count(), 1);
    assert!(scene.export_svg().is_err());
}

#[test]
fn edits_undo_redo_and_saved_dirty_state_are_real() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("architecture.excalidraw");
    let mut document = DrawingDocument::create(&path, Scene::default()).unwrap();
    assert!(!document.is_dirty());
    assert!(
        document
            .edit(|scene| scene.elements.push(rectangle()))
            .unwrap()
    );
    assert!(document.is_dirty());
    assert!(document.undo());
    assert!(document.scene().elements.is_empty());
    assert!(!document.is_dirty());
    assert!(document.redo());
    document.save().unwrap();
    assert!(!document.is_dirty());
    let reopened = DrawingDocument::open(&path).unwrap();
    assert_eq!(reopened.scene(), document.scene());
    assert!(document.undo());
    assert!(document.is_dirty());
    assert!(
        document
            .edit(|scene| scene.elements.push(Element::new(
                ElementKind::Ellipse,
                1.0,
                2.0,
                3.0,
                4.0
            )))
            .unwrap()
    );
    assert!(!document.can_redo());
}

#[test]
fn external_change_is_preserved_and_failed_save_keeps_local_scene() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flow.excalidraw");
    let mut document = DrawingDocument::create(&path, Scene::default()).unwrap();
    document
        .edit(|scene| scene.elements.push(rectangle()))
        .unwrap();
    std::fs::write(&path, "external data").unwrap();
    assert!(document.save().unwrap_err().contains("Conflito"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "external data");
    assert!(document.is_dirty());
    assert_eq!(document.scene().elements.len(), 1);
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn create_and_save_as_never_overwrite_an_existing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flow.excalidraw");
    std::fs::write(&path, "keep this").unwrap();
    assert!(DrawingDocument::create(&path, Scene::default()).is_err());
    let mut document = DrawingDocument::new(Scene::default()).unwrap();
    assert!(document.save_as(&path).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep this");
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
}

#[test]
fn invalid_scene_edit_rolls_back_without_history_and_input_limits_are_enforced() {
    let mut document = DrawingDocument::new(Scene::default()).unwrap();
    assert!(
        document
            .edit(|scene| {
                let mut el = rectangle();
                el.x = f64::NAN;
                scene.elements.push(el);
            })
            .is_err()
    );
    assert!(document.scene().elements.is_empty());
    assert!(!document.can_undo());
    assert!(Scene::from_json(r#"{"type":"excalidraw","version":3,"elements":[]}"#).is_err());
    assert!(Scene::from_json(r#"{"type":"excalidraw","version":2,"elements":[{"id":"x","type":"text","opacity":101}]}"#).is_err());
    assert!(Scene::from_json(&" ".repeat(codisseu_draw::MAX_SCENE_BYTES + 1)).is_err());
}

#[test]
fn all_supported_shapes_export_with_escaped_unicode_text_and_rotation() {
    let mut scene = Scene::default();
    for (i, kind) in [
        ElementKind::Rectangle,
        ElementKind::Ellipse,
        ElementKind::Diamond,
        ElementKind::Arrow,
        ElementKind::Line,
        ElementKind::Freedraw,
        ElementKind::Text,
    ]
    .into_iter()
    .enumerate()
    {
        let mut element = Element::new(kind, 100.0 * i as f64, 20.0, 80.0, 50.0);
        element.angle = 0.5;
        element.text = "Olá <script> & \"ação\"\n🦀".into();
        element.stroke_color = "url(https://example.invalid/private)".into();
        scene.elements.push(element);
    }
    let svg = scene.export_svg().unwrap();
    for token in [
        "<svg",
        "<rect",
        "<ellipse",
        "<polygon",
        "<polyline",
        "<text",
        "rotate(",
        "&lt;script&gt;",
        "&amp;",
        "🦀",
    ] {
        assert!(svg.contains(token), "{token}");
    }
    assert!(!svg.contains("<script>"));
    assert!(!svg.contains("https://example.invalid"));
    assert_eq!(Scene::from_json(&scene.to_json().unwrap()).unwrap(), scene);
}

#[test]
fn metadata_touch_preserves_creation_time_and_updates_text_source() {
    let mut text = Element::new(ElementKind::Text, 1.0, 2.0, 3.0, 4.0);
    let created = text.extra["created"].clone();
    text.text = "ação".into();
    text.touch();
    assert_eq!(text.extra["version"], 2);
    assert_eq!(text.extra["created"], created);
    assert_eq!(text.extra["originalText"], "ação");
}

#[cfg(unix)]
#[test]
fn symlink_replacement_and_hardlinks_cannot_be_saved() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flow.excalidraw");
    let target = temp.path().join("target.excalidraw");
    let mut document = DrawingDocument::create(&path, Scene::default()).unwrap();
    std::fs::write(&target, "keep target").unwrap();
    std::fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(document.save().is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep target");
    assert!(DrawingDocument::open(&path).is_err());
    std::fs::remove_file(&path).unwrap();
    let document = DrawingDocument::create(&path, Scene::default()).unwrap();
    let mut opened = document;
    let alias = temp.path().join("alias.excalidraw");
    std::fs::hard_link(&path, &alias).unwrap();
    opened
        .edit(|scene| scene.elements.push(rectangle()))
        .unwrap();
    assert!(opened.save().unwrap_err().contains("hard links"));
    assert!(
        DrawingDocument::open(&alias)
            .unwrap()
            .scene()
            .elements
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn successful_save_preserves_file_permissions() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("flow.excalidraw");
    let mut document = DrawingDocument::create(&path, Scene::default()).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    document
        .edit(|scene| scene.elements.push(rectangle()))
        .unwrap();
    document.save().unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
}
