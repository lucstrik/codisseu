//! Syntax and folding acceptance through the real editor, asynchronous workers and filesystem.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    collections::BTreeSet,
    fs, thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;

fn settle(h: &mut H) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.step();
        if !h.state().service.is_busy() && !h.state().editor_metrics().folding_pending {
            break;
        }
        assert!(Instant::now() < until, "{:?}", h.state().service.last_error);
        thread::sleep(Duration::from_millis(2));
    }
    h.step();
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}

fn fixture(name: &str, source: &str) -> (tempfile::TempDir, H) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    let path = root.join(name);
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenWorkspace(root)).unwrap();
    service.dispatch(Command::OpenFile(path)).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_max_steps(40)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h)
}

fn capture(h: &mut H, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_FORMAT_CAPTURES") {
        let out = std::path::PathBuf::from(out);
        fs::create_dir_all(&out).unwrap();
        h.event(egui::Event::PointerGone);
        h.step();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    }
}

fn jobs(shape: &egui::Shape, out: &mut Vec<egui::text::LayoutJob>) {
    match shape {
        egui::Shape::Text(text) => out.push((*text.galley.job).clone()),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                jobs(shape, out);
            }
        }
        _ => {}
    }
}

fn painted(h: &H) -> Vec<egui::text::LayoutJob> {
    let mut out = vec![];
    for shape in &h.output().shapes {
        jobs(&shape.shape, &mut out);
    }
    out
}

fn text(h: &H) -> String {
    h.state()
        .service
        .active_document()
        .unwrap()
        .snapshot()
        .to_string()
}

fn key(h: &mut H, mut modifiers: Modifiers, key: Key) {
    modifiers.command = modifiers.ctrl;
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.step();
}

fn select(h: &mut H, at: usize) {
    let document = h.state().service.active_document().unwrap();
    let (id, expected_version) = (document.id, document.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection::caret(at),
        })
        .unwrap();
    h.step();
}

fn fold_label(h: &H, action: &str, line: usize) -> String {
    let prefix = format!("{action} lines {line}–");
    h.get_all_by_role(Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .find(|label| label.starts_with(&prefix))
        .unwrap_or_else(|| panic!("Missing gutter {prefix}"))
}

fn click_fold(h: &mut H, action: &str, line: usize) {
    let label = fold_label(h, action, line);
    h.get_by_label(&label).click();
    h.step();
    h.step();
}

#[test]
fn additional_formats_render_colored_source_including_special_dotfile_names() {
    // Own inline fixtures; a comment and code/value distinguish real token colors
    // from a uniformly tinted plain text document, without pixel comparisons.
    let cases = [
        (
            "settings.json",
            "{\n  \"name\": \"ação\",\n  \"enabled\": true,\n  \"count\": 42\n}",
            "json",
        ),
        (
            "settings.xml",
            "<!-- Own fixture -->\n<service name=\"ação\" enabled=\"true\">42</service>\n",
            "xml",
        ),
        (
            "script.py",
            "# Own fixture\ndef welcome(name):\n    return \"Olá \" + name\n",
            "python",
        ),
        (
            "main.rs",
            "// Own fixture\nfn main() {\n    let count = 42;\n    println!(\"ação {}\", count);\n}\n",
            "rust",
        ),
        (
            ".env",
            "# Own fixture\nAPP_NAME=\"Codisseu ação\"\nAPP_PORT=8080\n",
            "dotenv",
        ),
        (
            ".env.local",
            "# Own fixture\nAPP_NAME=\"Local ação\"\nAPP_DEBUG=true\n",
            "dotenv-local",
        ),
        (
            ".gitignore",
            "# Own fixture\n/target/\n*.log\n!important.log\n",
            "gitignore",
        ),
        (
            "compose.yml",
            "# Own fixture\nservices:\n  app:\n    image: \"own:1\"\n    ports: [8080, 80]\n",
            "yaml-yml",
        ),
        (
            "config.yaml",
            "# Own fixture\nname: \"ação\"\nenabled: true\ncount: 42\n",
            "yaml",
        ),
        (
            ".editorconfig",
            "# Own fixture\nroot = true\n\n[*.rs]\nindent_style = space\nindent_size = 4\n",
            "editorconfig",
        ),
        (
            "Cargo.toml",
            "# Own fixture\n[package]\nname = \"own-fixture\"\nversion = \"0.1.0\"\n",
            "toml",
        ),
    ];
    for (name, source, capture_name) in cases {
        let (dir, mut h) = fixture(name, source);
        assert_eq!(text(&h), source, "{name}");
        let painted = painted(&h);
        let mut colors = BTreeSet::new();
        let mut colored_code_line = false;
        for line in source.lines().filter(|line| !line.is_empty()) {
            let job = painted
                .iter()
                .find(|job| job.text == line)
                .unwrap_or_else(|| {
                    panic!("{name}: source line absent from editor paint: {line:?}")
                });
            let mut line_colors = BTreeSet::new();
            for section in &job.sections {
                if !job.text[section.byte_range.start.0..section.byte_range.end.0]
                    .trim()
                    .is_empty()
                {
                    colors.insert(section.format.color.to_array());
                    line_colors.insert(section.format.color.to_array());
                }
            }
            if !["#", "//", "<!--"]
                .iter()
                .any(|prefix| line.trim_start().starts_with(prefix))
            {
                colored_code_line |= line_colors.len() >= 2;
            }
        }
        assert!(
            colors.len() >= 2,
            "{name}: source still paints uniformly: {colors:?}"
        );
        assert!(
            colored_code_line,
            "{name}: only comments received syntax colors"
        );
        assert!(
            !h.state().service.active_document().unwrap().is_dirty(),
            "{name}"
        );
        assert!(
            !h.state().service.active_document().unwrap().can_undo(),
            "{name}"
        );
        capture(&mut h, capture_name);
        assert_eq!(
            fs::read(dir.path().join("project").join(name)).unwrap(),
            source.as_bytes()
        );
    }
}

const JSON: &str = "{\r\n  \"service\": {\r\n    \"name\": \"ação 😀\",\r\n    \"ports\": [\r\n      8080,\r\n      8081\r\n    ],\r\n    \"empty\": {}\r\n  },\r\n  \"enabled\": true\r\n}";

#[test]
fn json_gutter_nests_objects_arrays_and_reveals_without_mutating_source_or_history() {
    let (dir, mut h) = fixture("nested.json", JSON);
    let version = h.state().service.active_document().unwrap().version;
    assert!(h.state().editor_metrics().folding_regions >= 3);
    capture(&mut h, "json-expanded");
    click_fold(&mut h, "Fold", 4);
    let array_hidden = h.state().editor_metrics().folded_lines;
    assert!(array_hidden > 0);
    assert!(!painted(&h).iter().any(|job| job.text.trim() == "8080,"));
    assert!(
        painted(&h).iter().any(|job| job.text == "    ],"),
        "JSON keeps the array closing delimiter visible"
    );
    click_fold(&mut h, "Fold", 2);
    assert!(h.state().editor_metrics().folded_lines > array_hidden);
    assert!(!painted(&h).iter().any(|job| job.text.contains("ação")));
    assert!(
        painted(&h).iter().any(|job| job.text == "  },"),
        "JSON keeps the object closing delimiter visible"
    );
    capture(&mut h, "json-collapsed");
    click_fold(&mut h, "Unfold", 2);
    assert_eq!(h.state().editor_metrics().folded_lines, array_hidden);
    click_fold(&mut h, "Unfold", 4);
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert_eq!(text(&h), JSON);
    let document = h.state().service.active_document().unwrap();
    assert_eq!(document.version, version);
    assert!(!document.is_dirty());
    assert!(!document.can_undo());
    assert!(!document.can_redo());
    click_fold(&mut h, "Fold", 4);
    click_fold(&mut h, "Fold", 2);
    click_fold(&mut h, "Fold", 1);
    let at = JSON[..JSON.find("8080").unwrap()].chars().count();
    select(&mut h, at);
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert!(painted(&h).iter().any(|job| job.text.trim() == "8080,"));
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        Selection::caret(at)
    );
    click_fold(&mut h, "Fold", 1);
    select(&mut h, 0);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.event(egui::Event::Text(" ".into()));
    h.step();
    assert_eq!(text(&h), format!(" {JSON}"));
    assert_eq!(
        h.state().editor_metrics().folded_lines,
        0,
        "edits invalidate old folding immediately"
    );
    key(&mut h, Modifiers::CTRL, Key::Z);
    settle(&mut h);
    assert_eq!(text(&h), JSON);
    assert!(h.state().service.active_document().unwrap().can_redo());
    click_fold(&mut h, "Fold", 1);
    assert!(h.state().service.active_document().unwrap().can_redo());
    assert!(h.get_by_role(Role::MultilineTextInput).is_focused());
    key(&mut h, Modifiers::CTRL, Key::Y);
    assert_eq!(
        text(&h),
        format!(" {JSON}"),
        "folding preserves the redo branch"
    );
    key(&mut h, Modifiers::CTRL, Key::Z);
    key(&mut h, Modifiers::CTRL, Key::S);
    settle(&mut h);
    assert_eq!(
        fs::read(dir.path().join("project/nested.json")).unwrap(),
        JSON.as_bytes()
    );
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}

#[test]
fn php_javascript_typescript_gutters_fold_nested_main_blocks_and_keep_edit_undo_exact() {
    let cases = [
        (
            "main.php",
            "<?php\nclass Welcome {\n    public function greet() {\n        return \"ação 😀 { text }\";\n    }\n}\n",
            2,
            3,
            "php-folded",
        ),
        (
            "main.js",
            "function welcome() {\n  const config = {\n    message: \"ação 😀 { text }\"\n  };\n  return config;\n}\n",
            1,
            2,
            "javascript-folded",
        ),
        (
            "main.ts",
            "class Welcome {\n  greet(): string {\n    return \"ação 😀 { text }\";\n  }\n}\n",
            1,
            2,
            "typescript-folded",
        ),
    ];
    for (name, source, outer, inner, capture_name) in cases {
        let (dir, mut h) = fixture(name, source);
        click_fold(&mut h, "Fold", inner);
        let inner_hidden = h.state().editor_metrics().folded_lines;
        assert!(inner_hidden > 0, "{name}");
        click_fold(&mut h, "Fold", outer);
        assert!(
            h.state().editor_metrics().folded_lines > inner_hidden,
            "{name}"
        );
        capture(&mut h, capture_name);
        click_fold(&mut h, "Unfold", outer);
        assert_eq!(
            h.state().editor_metrics().folded_lines,
            inner_hidden,
            "{name}"
        );
        assert_eq!(text(&h), source);
        assert!(!h.state().service.active_document().unwrap().is_dirty());
        assert!(!h.state().service.active_document().unwrap().can_undo());
        let at = source[..source.find("ação").unwrap()].chars().count();
        select(&mut h, at);
        assert_eq!(h.state().editor_metrics().folded_lines, 0, "{name}");
        h.get_by_role(Role::MultilineTextInput).focus();
        h.event(egui::Event::Text("X".into()));
        h.step();
        assert_eq!(text(&h), source.replacen("ação", "Xação", 1), "{name}");
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(text(&h), source, "{name}");
        key(&mut h, Modifiers::CTRL, Key::S);
        settle(&mut h);
        assert_eq!(
            fs::read(dir.path().join("project").join(name)).unwrap(),
            source.as_bytes()
        );
    }
}

#[test]
fn two_json_views_share_source_and_keep_independent_folding() {
    let (_dir, mut h) = fixture("nested.json", JSON);
    let first = h.state().service.active_view().unwrap();
    let document = h.state().service.active_document().unwrap().id;
    click_fold(&mut h, "Fold", 1);
    let first_hidden = h.state().editor_metrics().folded_lines;
    h.state_mut().service.dispatch(Command::CreateView).unwrap();
    settle(&mut h);
    let second = h.state().service.active_view().unwrap();
    assert_ne!(first, second);
    assert_eq!(h.state().service.active_document().unwrap().id, document);
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert!(fold_label(&h, "Fold", 1).starts_with("Fold"));
    assert!(fold_label(&h, "Unfold", 1).starts_with("Unfold"));
    capture(&mut h, "json-independent-split");
    h.state_mut()
        .service
        .dispatch(Command::ActivateView { view: first })
        .unwrap();
    h.step();
    assert_eq!(h.state().editor_metrics().folded_lines, first_hidden);
    h.state_mut()
        .service
        .dispatch(Command::ActivateView { view: second })
        .unwrap();
    h.step();
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert_eq!(text(&h), JSON);
    assert_eq!(h.state().service.documents.len(), 1);
}
