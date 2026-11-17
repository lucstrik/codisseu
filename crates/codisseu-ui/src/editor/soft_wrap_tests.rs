//! Controlled projection fixtures, never a claim about live model responses.
use super::*;

#[test]
fn wrapping_boundary_affinity_and_folded_source_rows_share_one_index() {
    let mut fold = folding::FoldMap::new(
        5,
        vec![folding::FoldRange {
            start: 1,
            end: 3,
            kind: folding::FoldKind::Braces,
            level: 0,
        }],
    );
    let mut wrap = soft_wrap::WrapMap::default();
    wrap.enabled = true;
    wrap.counts_for_test(0, &[0. ..10., 10. ..20., 20. ..30.]);
    wrap.rebuild(&fold);
    assert_eq!(wrap.total(&fold), 7);
    assert_eq!(wrap.source_line(&fold, 2), 0);
    assert_eq!(wrap.source_line(&fold, 3), 1);
    assert_eq!(wrap.position(&fold, 0, 0, 10., false), Some((0., 1)));
    assert_eq!(wrap.position(&fold, 0, 0, 10., true), Some((10., 0)));
    assert_eq!(wrap.position(&fold, 0, 0, 30., false), Some((10., 2)));
    fold.fold_at(1);
    wrap.rebuild(&fold);
    assert_eq!(wrap.total(&fold), 5);
    assert_eq!(wrap.source_line(&fold, 4), 4);
}

#[test]
fn parameter_and_type_inlays_reflow_without_source_revision_or_content_changes() {
    for scale in [1., 1.25] {
        let context = egui::Context::default();
        context.set_pixels_per_point(scale);
        let tokens = Tokens::load();
        tokens.apply(&context);
        let snapshot = TextSnapshot::from_str(
            "let result = calculate(first, second, third, fourth, fifth);\n",
        );
        let original = snapshot.clone();
        let mut cache = layout::LineCache::default();
        let mut wrap = soft_wrap::WrapMap::default();
        let fold = folding::FoldMap::new(snapshot.len_lines(), vec![]);
        let mut measured = Vec::new();
        for (hint_revision, hints) in [
            (0, BTreeMap::new()),
            (
                1,
                BTreeMap::from([(
                    0,
                    vec![
                        inlays::Hint {
                            column: 10,
                            text: ": CalculationResult ".into(),
                            tooltip: Some("Controlled type hint".into()),
                        },
                        inlays::Hint {
                            column: 23,
                            text: "source: ".into(),
                            tooltip: Some("Controlled parameter hint".into()),
                        },
                    ],
                )]),
            ),
            (2, BTreeMap::new()),
        ] {
            cache.set_inlays(0, hint_revision, hints);
            for _ in 0..8 {
                let mut output = context.run_ui(egui::RawInput::default(), |ui| {
                    cache.begin_frame();
                    cache.begin_document(snapshot.clone(), 0, None, "ts");
                    cache.sync(0, None, ui, &tokens);
                    wrap.sync(
                        true,
                        180.,
                        0,
                        &tokens,
                        true,
                        None,
                        (hint_revision, true, scale.to_bits(), cache.geometry_epoch()),
                    );
                    wrap.begin_frame();
                    wrap.prepare(
                        ui,
                        &snapshot,
                        &mut cache,
                        &fold,
                        None,
                        0..20,
                        0,
                        &[],
                        "ts",
                        &tokens,
                    );
                    let line = cache.get(ui, &snapshot, 0, "ts", &tokens);
                    let point = line.x(ui, &snapshot, &tokens, 40).unwrap();
                    assert!(wrap.position(&fold, 0, 0, point, false).is_some());
                    assert_eq!(line.column(ui, &snapshot, &tokens, point), Some(40));
                });
                output.textures_delta.clear();
                if !wrap.pending {
                    break;
                }
            }
            assert!(!wrap.pending);
            measured.push(wrap.total(&fold));
            assert_eq!(snapshot, original);
        }
        assert!(
            measured[1] > measured[0],
            "type/parameter hints consume visual width: {measured:?}"
        );
        assert_eq!(
            measured[2], measured[0],
            "disabling hints must restore wrapping without editing source"
        );
    }
}

#[cfg(unix)]
#[test]
fn wrapped_multipart_copilot_preserves_source_spans_and_accepts_then_undoes_atomically() {
    use crate::copilot::tests::{request, setup, wait};
    use egui_kittest::kittest::Queryable;
    use std::fs;
    for scale in [1., 1.25] {
        let (_directory, file, mut harness) = setup(scale, "js", "unused");
        harness.set_size(egui::vec2(430., 740.));
        let tail = " // retained_source_tail repeated_words verify_mapping";
        let first = format!("const result = f($a,$b);{tail}");
        let source = format!("{first}\nfollowing();\n");
        let target = format!(
            "const result = f($a,\n    {} + $b);{tail}",
            "generated_argument, ".repeat(12)
        );
        let program = PathBuf::from(harness.state().service.editor_preferences().copilot_program);
        harness
            .state_mut()
            .service
            .dispatch(Command::SetCopilotEnabled(false))
            .unwrap();
        harness.step();
        fs::write(program.with_extension("json"),serde_json::json!({"items":[{
            "insertText":target,"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":first.chars().count()}}
        }]}).to_string()).unwrap();
        let doc = harness.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        harness
            .state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: source.clone(),
            })
            .unwrap();
        let version = harness.state().service.active_document().unwrap().version;
        let caret = "const result = f(".chars().count();
        harness
            .state_mut()
            .service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version,
                selection: Selection::caret(caret),
            })
            .unwrap();
        harness
            .state_mut()
            .service
            .dispatch(Command::SetCopilotEnabled(true))
            .unwrap();
        wait(&mut harness, |app| {
            app.copilot_ui
                .client
                .as_ref()
                .is_some_and(|client| client.authenticated)
        });
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        harness.step();
        request(&mut harness);
        wait(&mut harness, |app| {
            app.editor_metrics().copilot_ghost_visible && !app.editor_metrics().wrap_pending
        });
        let metrics = harness.state().editor_metrics();
        assert!(metrics.visual_rows > metrics.lines + metrics.copilot_ghost_rows);
        assert!(!metrics.copilot_ghost_rects.is_empty());
        assert!(
            metrics
                .wrap_row_rects
                .iter()
                .all(|row| row.start_char <= source.chars().count()
                    && row.end_char <= source.chars().count())
        );
        assert_eq!(
            harness.state().service.active_document().unwrap().text(),
            source
        );
        harness.key_press(Key::Tab);
        harness.step();
        assert_eq!(
            harness.state().service.active_document().unwrap().text(),
            format!("{target}\nfollowing();\n")
        );
        harness.key_press_modifiers(Modifiers::CTRL, Key::Z);
        harness.step();
        assert_eq!(
            harness.state().service.active_document().unwrap().text(),
            source
        );
        assert_eq!(fs::read(&file).unwrap(), b"// owned fixture\n");
    }
}

#[cfg(unix)]
#[test]
fn first_vertical_key_dismisses_ghost_rows_and_moves_to_the_real_wrapped_source_row() {
    use crate::copilot::tests::{request, setup, wait};
    use egui_kittest::kittest::Queryable;
    for scale in [1., 1.25] {
        let (_directory, _file, mut harness) = setup(
            scale,
            "ts",
            &format!(
                "{}\n{}\n",
                "generated_words ".repeat(12),
                "more_virtual_words ".repeat(8)
            ),
        );
        harness
            .state_mut()
            .service
            .dispatch(Command::SetCopilotEnabled(false))
            .unwrap();
        harness.set_size(egui::vec2(430., 740.));
        let source = format!("{}\nlast();\n", "alpha beta gamma delta ".repeat(20));
        let doc = harness.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        harness
            .state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: source.clone(),
            })
            .unwrap();
        let version = harness.state().service.active_document().unwrap().version;
        harness
            .state_mut()
            .service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version,
                selection: Selection::caret(0),
            })
            .unwrap();
        wait(&mut harness, |app| {
            !app.editor_metrics().wrap_pending && app.editor_metrics().wrap_row_rects.len() > 2
        });
        let next = harness
            .state()
            .editor_metrics()
            .wrap_row_rects
            .iter()
            .find(|row| row.source_line == 0 && row.start_char > 0)
            .unwrap()
            .start_char;
        harness
            .state_mut()
            .service
            .dispatch(Command::SetCopilotEnabled(true))
            .unwrap();
        wait(&mut harness, |app| {
            app.copilot_ui
                .client
                .as_ref()
                .is_some_and(|client| client.authenticated)
        });
        harness
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        harness.step();
        request(&mut harness);
        wait(&mut harness, |app| {
            app.editor_metrics().copilot_ghost_visible && !app.editor_metrics().wrap_pending
        });
        harness.key_press(Key::ArrowDown);
        harness.step();
        harness.step();
        assert_eq!(
            harness
                .state()
                .service
                .active_document()
                .unwrap()
                .selection(),
            Selection::caret(next)
        );
        assert_eq!(
            harness.state().service.active_document().unwrap().text(),
            source
        );
        assert!(!harness.state().editor_metrics().copilot_ghost_visible);
    }
}

#[cfg(unix)]
#[test]
fn queued_fold_then_down_rebuilds_wrapped_prefix_before_the_second_key() {
    use crate::copilot::tests::{setup, wait};
    use egui_kittest::kittest::Queryable;
    let (_directory, _file, mut harness) = setup(1., "ts", "unused");
    harness
        .state_mut()
        .service
        .dispatch(Command::SetCopilotEnabled(false))
        .unwrap();
    harness.set_size(egui::vec2(430., 740.));
    let source = format!(
        "function outer() {{\n    const message = '{}';\n    return message;\n}}\nafter();\n",
        "long wrapped content ".repeat(60)
    );
    let doc = harness.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    harness
        .state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: source.clone(),
        })
        .unwrap();
    let version = harness.state().service.active_document().unwrap().version;
    harness
        .state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(0),
        })
        .unwrap();
    wait(&mut harness, |app| {
        !app.editor_metrics().folding_pending
            && app.editor_metrics().folding_regions > 0
            && !app.editor_metrics().wrap_pending
            && app.editor_metrics().wrapped_lines > 0
    });
    let expected = harness
        .state()
        .service
        .active_document()
        .unwrap()
        .snapshot()
        .line_to_char(4);
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    harness.step();
    // Both actions deliberately arrive in one native event batch. Painting
    // cannot repair an already chosen canonical destination after this loop.
    harness.key_press_modifiers(Modifiers::CTRL.plus(Modifiers::SHIFT), Key::OpenBracket);
    harness.key_press(Key::ArrowDown);
    harness.step();
    harness.step();
    assert_eq!(harness.state().editor_metrics().folded_lines, 3);
    assert_eq!(
        harness
            .state()
            .service
            .active_document()
            .unwrap()
            .selection(),
        Selection::caret(expected)
    );
    assert_eq!(
        harness.state().service.active_document().unwrap().text(),
        source
    );
}

#[cfg(unix)]
#[test]
fn cycling_same_ticket_copilot_alternatives_reflows_short_and_long_projections() {
    use crate::copilot::tests::{request, setup, wait};
    use egui_kittest::kittest::Queryable;
    use std::fs;
    let (_directory, _file, mut harness) = setup(1., "ts", "unused");
    harness
        .state_mut()
        .service
        .dispatch(Command::SetCopilotEnabled(false))
        .unwrap();
    harness.step();
    harness.set_size(egui::vec2(430., 740.));
    let short = "short";
    let long = format!(
        "{}\n{}\n",
        "generated_argument ".repeat(20),
        "another_argument ".repeat(15)
    );
    let program = PathBuf::from(harness.state().service.editor_preferences().copilot_program);
    fs::write(
        program.with_extension("json"),
        serde_json::json!({"suggestions":[short,long]}).to_string(),
    )
    .unwrap();
    let source = "tail();\n";
    let doc = harness.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    harness
        .state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: source.into(),
        })
        .unwrap();
    let version = harness.state().service.active_document().unwrap().version;
    harness
        .state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(0),
        })
        .unwrap();
    harness
        .state_mut()
        .service
        .dispatch(Command::SetCopilotEnabled(true))
        .unwrap();
    wait(&mut harness, |app| {
        app.copilot_ui
            .client
            .as_ref()
            .is_some_and(|client| client.authenticated)
    });
    harness
        .get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    harness.step();
    request(&mut harness);
    wait(&mut harness, |app| {
        app.editor_metrics().copilot_ghost_visible && !app.editor_metrics().wrap_pending
    });
    let ticket = harness
        .state()
        .copilot_ui
        .client
        .as_ref()
        .unwrap()
        .suggestion
        .as_ref()
        .unwrap()
        .ticket;
    let rows_short = harness.state().editor_metrics().visual_rows;
    harness.key_press_modifiers(Modifiers::ALT, Key::CloseBracket);
    harness.step();
    harness.step();
    wait(&mut harness, |app| !app.editor_metrics().wrap_pending);
    let suggestion = harness
        .state()
        .copilot_ui
        .client
        .as_ref()
        .unwrap()
        .suggestion
        .as_ref()
        .unwrap();
    assert_eq!(
        suggestion.ticket, ticket,
        "cycling reuses the same provider ticket"
    );
    assert_eq!(suggestion.suffix, long);
    assert!(
        harness.state().editor_metrics().visual_rows > rows_short + 3,
        "long alternative must rebuild projected wrap rows"
    );
    assert_eq!(
        harness.state().service.active_document().unwrap().text(),
        source
    );
    harness.key_press_modifiers(Modifiers::ALT, Key::OpenBracket);
    harness.step();
    harness.step();
    wait(&mut harness, |app| !app.editor_metrics().wrap_pending);
    assert_eq!(harness.state().editor_metrics().visual_rows, rows_short);
    harness.key_press(Key::Tab);
    harness.step();
    assert_eq!(
        harness.state().service.active_document().unwrap().text(),
        format!("{short}{source}")
    );
    harness.key_press_modifiers(Modifiers::CTRL, Key::Z);
    harness.step();
    assert_eq!(
        harness.state().service.active_document().unwrap().text(),
        source
    );
}
