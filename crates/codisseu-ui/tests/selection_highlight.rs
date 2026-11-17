//! Passive text decorations through the shared native editor widget.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

type H = Harness<'static, CodisseuApp>;
fn settle(h: &mut H) {
    let deadline = Instant::now() + Duration::from_secs(20);
    for _ in 0..4 {
        h.step();
    }
    while h.state().service.is_busy()
        || h.state().editor_metrics().layout_pending
        || h.state().editor_metrics().selection_highlight_pending
    {
        assert!(
            Instant::now() < deadline,
            "editor did not settle: {:?}",
            h.state().editor_metrics()
        );
        thread::sleep(Duration::from_millis(1));
        h.step();
    }
    h.step();
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}
fn fixture(
    source: &str,
    extension: &str,
    scale: f32,
    size: [f32; 2],
) -> (tempfile::TempDir, H, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("main.{extension}"));
    fs::write(&path, source).unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(scale)
        .with_step_dt(1. / 60.)
        .build_ui_state(
            |ui, app: &mut CodisseuApp| app.show(ui),
            CodisseuApp::new(service),
        );
    settle(&mut h);
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    (dir, h, path)
}
fn select(h: &mut H, range: std::ops::Range<usize>) {
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection {
                anchor: range.start,
                head: range.end,
            },
        })
        .unwrap();
    settle(h);
}
fn key(h: &mut H, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    h.step();
    h.step();
}
fn capture(h: &mut H, name: &str) {
    if let Some(directory) = std::env::var_os("CODISSEU_SELECTION_CAPTURES") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        h.render()
            .unwrap()
            .save(directory.join(format!("{name}.png")))
            .unwrap();
    }
}
fn scalar_at(source: &str, needle: &str) -> usize {
    source[..source.find(needle).unwrap()].chars().count()
}
fn passive_rectangles(h: &H) -> Vec<egui::Rect> {
    let tokens: serde_json::Value =
        serde_json::from_str(include_str!("../../../design/tokens.json")).unwrap();
    let rgba = tokens["editor_selection_highlight"].as_array().unwrap();
    let fill = egui::Color32::from_rgba_unmultiplied(
        rgba[0].as_u64().unwrap() as u8,
        rgba[1].as_u64().unwrap() as u8,
        rgba[2].as_u64().unwrap() as u8,
        rgba[3].as_u64().unwrap() as u8,
    );
    fn collect(shape: &egui::Shape, fill: egui::Color32, output: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == fill => output.push(rect.rect),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, fill, output);
                }
            }
            _ => (),
        }
    }
    let mut output = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, fill, &mut output);
    }
    output
}

#[test]
fn separate_pointer_frames_select_word_and_paint_substrings_without_editing_them() {
    let original = "selection selectionDatabase Selection SELECTION\n// selection\n\"selection\"";
    for scale in [1., 1.25] {
        let (_dir, mut h, path) = fixture(original, "ts", scale, [1280., 800.]);
        let version = h.state().service.active_document().unwrap().version;
        let pos = h.get_by_role(Role::MultilineTextInput).rect().left_top() + egui::vec2(15., 8.);
        h.event(egui::Event::PointerMoved(pos));
        h.step();
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Modifiers::NONE,
                });
                h.step();
            }
        }
        settle(&mut h);
        let doc = h.state().service.active_document().unwrap();
        assert_eq!(doc.selection(), Selection { anchor: 0, head: 9 });
        assert_eq!(doc.version, version);
        assert!(!doc.is_dirty());
        assert_eq!(h.state().editor_metrics().selection_count, 1);
        assert_eq!(h.state().editor_metrics().selection_highlights, 5);
        capture(&mut h, &format!("double-click-{scale}"));
        h.event(egui::Event::Copy);
        h.step();
        assert!(h.output().platform_output.commands.iter().any(
            |command| matches!(command,egui::OutputCommand::CopyText(text) if text=="selection")
        ));
        h.event(egui::Event::Text("X".into()));
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            original.replacen("selection", "X", 1)
        );
        assert_eq!(h.state().editor_metrics().selection_count, 1);
        key(&mut h, Modifiers::COMMAND, Key::S);
        settle(&mut h);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            original.replacen("selection", "X", 1)
        );
        key(&mut h, Modifiers::COMMAND, Key::Z);
        settle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            original
        );
        key(&mut h, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        settle(&mut h);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            original.replacen("selection", "X", 1)
        );
    }
}

#[test]
fn unicode_tabs_crlf_grapheme_boundaries_and_multiline_whitespace_limits() {
    let source = "😀\tação\tAÇÃO açãO ação\r\nß SS ss ẞ\r\ne\u{301} É e\u{301}\r\n    \r\n"
        .to_owned()
        + &"q".repeat(201);
    let (_dir, mut h, _) = fixture(&source, "php", 1., [1280., 800.]);
    let start = scalar_at(&source, "ação");
    select(&mut h, start..start + 4);
    assert_eq!(h.state().editor_metrics().selection_highlights, 3);
    let sharp = scalar_at(&source, "ß");
    select(&mut h, sharp..sharp + 1);
    assert_eq!(h.state().editor_metrics().selection_highlights, 1);
    let combined = scalar_at(&source, "e\u{301}");
    select(&mut h, combined..combined + 2);
    assert_eq!(h.state().editor_metrics().selection_highlights, 1);
    // A partial grapheme must not decorate the initial scalar inside e + acute.
    select(&mut h, combined..combined + 1);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
    select(&mut h, start..sharp + 1);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
    let blank = scalar_at(&source, "    ");
    select(&mut h, blank..blank + 4);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
    let long = scalar_at(&source, &"q".repeat(201));
    select(&mut h, long..long + 201);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
}

#[test]
fn blur_preserves_decorations_caret_clears_and_ime_suppresses_transient_matches() {
    let (_dir, mut h, _) = fixture(
        "selection Selection selectionDatabase",
        "js",
        1.,
        [1280., 800.],
    );
    select(&mut h, 0..9);
    assert_eq!(h.state().editor_metrics().selection_highlights, 2);
    h.get_by_label("Menu principal").focus();
    h.step();
    h.step();
    assert!(!h.get_by_role(Role::MultilineTextInput).is_focused());
    assert_eq!(h.state().editor_metrics().selection_highlights, 2);
    capture(&mut h, "blur-keeps-passive");
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.event(egui::Event::Ime(egui::ImeEvent::Preedit {
        text: "selection".into(),
        active_range_chars: None,
    }));
    h.step();
    h.step();
    assert!(h.state().editor_metrics().composing);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
    key(&mut h, Modifiers::NONE, Key::Escape);
    settle(&mut h);
    select(&mut h, 9..9);
    assert_eq!(h.state().editor_metrics().selection_highlights, 0);
}

#[test]
fn independent_views_documents_and_languages_share_only_the_matching_rules() {
    for extension in ["php", "js", "ts", "html", "css", "json", "sql", "rs", "py"] {
        let (_dir, mut h, _) = fixture(
            "selection Selection selectionDatabase\nother OTHER other",
            extension,
            1.,
            [1280., 800.],
        );
        select(&mut h, 0..9);
        assert_eq!(
            h.state().editor_metrics().selection_highlights,
            2,
            "{extension}"
        );
        assert_eq!(h.state().editor_metrics().selection_count, 1);
    }
    let source = "selection Selection selectionDatabase\nother OTHER other";
    let (dir, mut h, _) = fixture(source, "ts", 1., [1280., 800.]);
    select(&mut h, 0..9);
    let left = h.state().service.active_view().unwrap();
    let original = h.state().service.active.unwrap();
    h.state_mut().service.dispatch(Command::CreateView).unwrap();
    let right = h.state().service.document_views()[1].id;
    h.state_mut()
        .service
        .dispatch(Command::ActivateView { view: right })
        .unwrap();
    let other = scalar_at(source, "other");
    select(&mut h, other..other + 5);
    assert_eq!(
        h.state().editor_view_metrics()[&left].selection_highlights,
        2
    );
    assert_eq!(
        h.state().editor_view_metrics()[&right].selection_highlights,
        2
    );
    let path = dir.path().join("different.sql");
    fs::write(&path, "SELECT 1;").unwrap();
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path))
        .unwrap();
    settle(&mut h);
    assert_eq!(
        h.state().editor_view_metrics()[&left].selection_highlights,
        2
    );
    assert_eq!(
        h.state().editor_view_metrics()[&right].selection_highlights,
        0
    );
    h.state_mut()
        .service
        .dispatch(Command::ActivateTab {
            view: right,
            id: original,
        })
        .unwrap();
    settle(&mut h);
    assert_eq!(
        h.state().editor_view_metrics()[&right].selection_highlights,
        2
    );
    assert_eq!(
        h.state().service.view(left).unwrap().selection(),
        Selection { anchor: 0, head: 9 }
    );
}

#[test]
fn huge_document_scans_only_the_viewport_and_reuses_ready_windows_after_scroll() {
    let source = "selection selectionDatabase\n".repeat(50_000);
    let (_dir, mut h, _) = fixture(&source, "txt", 1., [1280., 800.]);
    select(&mut h, 0..9);
    assert!(h.state().editor_metrics().selection_highlights > 20);
    assert!(h.state().editor_metrics().selection_highlight_scanned_chars <= 32_768);
    assert_eq!(
        h.state().editor_metrics().selection_highlight_scanned_chars,
        0,
        "settled windows should be cached"
    );
    let view = h.state().service.active_view().unwrap();
    h.state_mut()
        .service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: [0., 360_000.],
        })
        .unwrap();
    h.step();
    assert!(h.state().editor_metrics().selection_highlight_scanned_chars <= 32_768);
    settle(&mut h);
    assert!(h.state().editor_metrics().visible_start > 10_000);
    assert!(h.state().editor_metrics().selection_highlights > 20);
    assert_eq!(
        h.state().service.active_document().unwrap().selection(),
        Selection { anchor: 0, head: 9 }
    );
    assert_eq!(
        h.state().editor_metrics().selection_highlight_scanned_chars,
        0
    );
    assert!(!h.state().editor_metrics().selection_highlight_limited);
    assert!(h.state().editor_metrics().cache_bytes <= 4 * 1024 * 1024);
    capture(&mut h, "distant-viewport");
}

#[test]
fn long_logical_line_highlights_a_match_crossing_a_layout_tile_without_giant_rectangles() {
    let source = format!(
        "selection {}SeLeCtIoN{}",
        "x".repeat(1010),
        "x".repeat(6000)
    );
    let (_dir, mut h, _) = fixture(&source, "txt", 1., [1280., 800.]);
    // This corpus specifically exercises horizontal tiles, independently of the
    // product's default presentation preference.
    h.state_mut()
        .service
        .dispatch(Command::SetEditorSoftWrap(false))
        .unwrap();
    settle(&mut h);
    select(&mut h, 0..9);
    let view = h.state().service.active_view().unwrap();
    h.state_mut()
        .service
        .dispatch(Command::SetViewScroll {
            view,
            scroll: [7950., 0.],
        })
        .unwrap();
    settle(&mut h);
    assert!(h.state().editor_metrics().scroll_x > 7000.);
    assert_eq!(h.state().editor_metrics().selection_highlights, 1);
    let rectangles = passive_rectangles(&h);
    assert_eq!(rectangles.len(), 1);
    assert!(
        rectangles[0].width() > 20. && rectangles[0].width() < 150.,
        "a nine-character match must not span a tile: {:?}",
        rectangles[0]
    );
    assert!(rectangles[0].height() < 30.);
    assert!(h.state().editor_metrics().selection_highlight_scanned_chars <= 32_768);
    assert!(h.state().editor_metrics().cache_bytes <= 4 * 1024 * 1024);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    capture(&mut h, "cross-tile");
}

#[test]
fn scan_budget_defers_large_visible_corpus_and_completes_without_rescanning_ready_windows() {
    let line = format!("selection {} selectionDatabase\n", "x".repeat(1100));
    let source = line.repeat(300);
    let (_dir, mut h, _) = fixture(&source, "txt", 1., [4096., 3000.]);
    // Keep the original count of visible logical lines: the scan-budget and
    // 4096-match ceilings are under test, rather than soft-wrap geometry.
    h.state_mut()
        .service
        .dispatch(Command::SetEditorSoftWrap(false))
        .unwrap();
    settle(&mut h);
    let doc = h.state().service.active_document().unwrap();
    let (id, expected_version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: Selection { anchor: 0, head: 9 },
        })
        .unwrap();
    let mut saw_pending = false;
    for _ in 0..12 {
        h.step();
        let metrics = h.state().editor_metrics();
        assert!(metrics.selection_highlight_scanned_chars <= 32_768);
        saw_pending |= metrics.selection_highlight_pending;
    }
    assert!(saw_pending, "fixture must exceed the per-frame scan budget");
    settle(&mut h);
    assert!(h.state().editor_metrics().selection_highlights > 80);
    assert_eq!(
        h.state().editor_metrics().selection_highlight_scanned_chars,
        0
    );
    assert!(!h.state().editor_metrics().selection_highlight_pending);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
    select(&mut h, 10..11);
    assert_eq!(h.state().editor_metrics().selection_highlights, 4096);
    assert!(h.state().editor_metrics().selection_highlight_limited);
    assert_eq!(h.state().editor_metrics().selection_count, 1);
}

#[test]
fn folded_source_is_not_painted_and_unfold_restores_passive_matches_without_source_changes() {
    let source = "selection\nfunction selectionDatabase() {\n  const a = 'selection';\n  const b = 'Selection';\n}\n// SELECTION\n";
    let (_dir, mut h, _) = fixture(source, "ts", 1., [1280., 800.]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.state().editor_metrics().folding_pending {
        h.step();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(h.state().editor_metrics().folding_regions > 0);
    select(&mut h, 0..9);
    assert_eq!(h.state().editor_metrics().selection_highlights, 4);
    key(
        &mut h,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        Key::OpenBracket,
    );
    // Fold the function by placing only the caret on its header, then restore
    // the first-line query without revealing the hidden body.
    select(&mut h, 10..10);
    key(
        &mut h,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        Key::OpenBracket,
    );
    select(&mut h, 0..9);
    assert!(h.state().editor_metrics().folded_lines > 0);
    assert_eq!(h.state().editor_metrics().selection_highlights, 2);
    key(
        &mut h,
        Modifiers::CTRL.plus(Modifiers::SHIFT),
        Key::CloseBracket,
    );
    // Opening at the first line may have no fold, so use the common unfold-all chord.
    key(&mut h, Modifiers::CTRL, Key::K);
    key(&mut h, Modifiers::CTRL, Key::J);
    settle(&mut h);
    assert_eq!(h.state().editor_metrics().folded_lines, 0);
    assert_eq!(h.state().editor_metrics().selection_highlights, 4);
    assert_eq!(h.state().service.active_document().unwrap().text(), source);
    assert!(!h.state().service.active_document().unwrap().is_dirty());
}
