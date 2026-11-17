use codisseu_editor::{EditKind, Selection, TextBuffer, TextDelta, commands::*};
fn selection(anchor: usize, head: usize) -> Selection {
    Selection { anchor, head }
}
fn run(
    source: &str,
    selections: Vec<Selection>,
    command: EditorCommand,
) -> (String, Vec<Selection>) {
    let mut buffer = TextBuffer::new(source.into()).unwrap();
    buffer.set_selection(selections[0]).unwrap();
    let plan = plan(&buffer.snapshot(), &selections, command).unwrap();
    if !plan.deltas.is_empty() {
        buffer
            .apply_batch_with_selection(plan.deltas, selections[0], plan.selections[0])
            .unwrap();
        let result = buffer.text().to_owned();
        assert!(buffer.undo());
        assert_eq!(buffer.text(), source);
        assert_eq!(buffer.selection(), selections[0]);
        assert!(!buffer.can_undo());
        assert!(buffer.redo());
        assert_eq!(buffer.text(), result);
        assert_eq!(buffer.selection(), plan.selections[0]);
    }
    (buffer.text().into(), plan.selections)
}
#[test]
fn directed_multiline_move_excludes_the_next_line_at_column_zero_and_undo_is_atomic() {
    let source = "one\r\ntwo\r\nthree\r\nfour";
    let (text, selected) = run(
        source,
        vec![selection(17, 5)],
        EditorCommand::MoveLines(Direction::Up),
    );
    assert_eq!(text, "two\r\nthree\r\none\r\nfour");
    assert_eq!(selected, vec![selection(12, 0)]);
    let (restored, selected) = run(&text, selected, EditorCommand::MoveLines(Direction::Down));
    assert_eq!(restored, source);
    assert_eq!(selected, vec![selection(17, 5)]);
}
#[test]
fn moving_last_unterminated_line_preserves_eol_slots_and_caret_column() {
    let (text, selected) = run(
        "a\r\nb\nc",
        vec![Selection::caret(4)],
        EditorCommand::MoveLines(Direction::Down),
    );
    assert_eq!(text, "a\r\nc\nb");
    assert_eq!(selected, vec![Selection::caret(6)]);
    let (text, selected) = run(
        "a\nb",
        vec![Selection::caret(3)],
        EditorCommand::MoveLines(Direction::Up),
    );
    assert_eq!(text, "b\na");
    assert_eq!(selected, vec![Selection::caret(1)]);
}
#[test]
fn move_round_trips_unicode_graphemes_mixed_endings_and_directed_partial_selections() {
    for newline in ["\n", "\r\n"] {
        for terminal in ["", newline] {
            let source = format!("alpha{newline}é 👩🏽‍💻{newline}雨{newline}last{terminal}");
            let buffer = TextBuffer::new(source.clone()).unwrap();
            let a = buffer.line_to_char(1).unwrap();
            let b = buffer.line_to_char(2).unwrap() + 1;
            for before in [selection(a, b), selection(b, a)] {
                let (text, selected) = run(
                    &source,
                    vec![before],
                    EditorCommand::MoveLines(Direction::Up),
                );
                assert!(text.contains("é 👩🏽‍💻"));
                let (restored, after) =
                    run(&text, selected, EditorCommand::MoveLines(Direction::Down));
                assert_eq!(restored, source);
                assert_eq!(after, vec![before]);
            }
        }
    }
}
#[test]
fn copying_lines_tracks_the_new_copy_with_unicode_and_without_final_newline() {
    for direction in [Direction::Up, Direction::Down] {
        let (text, selected) = run(
            "a\r\n👩‍💻",
            vec![Selection::caret(6)],
            EditorCommand::CopyLines(direction),
        );
        assert_eq!(text, "a\r\n👩‍💻\r\n👩‍💻");
        assert_eq!(
            selected,
            vec![Selection::caret(if direction == Direction::Up {
                6
            } else {
                11
            })]
        );
    }
    let (text, selected) = run(
        "one\r\ntwo\r\nthree\r\nfour",
        vec![selection(17, 5)],
        EditorCommand::CopyLines(Direction::Down),
    );
    assert_eq!(text, "one\r\ntwo\r\nthree\r\ntwo\r\nthree\r\nfour");
    assert_eq!(selected, vec![selection(29, 17)]);
}
#[test]
fn overlapping_and_adjacent_line_selections_are_transformed_only_once() {
    let source = "a\nb\nc\nd\ne";
    let (text, selected) = run(
        source,
        vec![Selection::caret(0), selection(2, 4), Selection::caret(4)],
        EditorCommand::Indent(Indentation::Spaces(2)),
    );
    assert_eq!(text, "  a\n  b\n  c\nd\ne");
    assert_eq!(selected[0], Selection::caret(2));
    let (text, _) = run(
        "a\nb\nc",
        vec![Selection::caret(2), Selection::caret(2)],
        EditorCommand::CopyLines(Direction::Down),
    );
    assert_eq!(text, "a\nb\nb\nc");
}
#[test]
fn delete_lines_handles_last_empty_line_all_lines_and_crlf_without_splitting() {
    for (source, caret, expected) in [
        ("a\r\nb", 4, "a"),
        ("a\r\n", 3, "a"),
        ("rain", 2, ""),
        ("", 0, ""),
    ] {
        let (text, _) = run(
            source,
            vec![Selection::caret(caret)],
            EditorCommand::DeleteLines,
        );
        assert_eq!(text, expected);
    }
    let (text, selected) = run("a\nb\nc", vec![selection(4, 0)], EditorCommand::DeleteLines);
    assert_eq!(text, "c");
    assert_eq!(selected, vec![Selection::caret(0)]);
}
#[test]
fn insertion_uses_local_eol_indentation_and_collapse_destination() {
    let (text, selected) = run(
        "  é\r\nlast",
        vec![Selection::caret(4)],
        EditorCommand::InsertLineAbove,
    );
    assert_eq!(text, "  \r\n  é\r\nlast");
    assert_eq!(selected, vec![Selection::caret(2)]);
    let (text, selected) = run(
        "  é\r\nlast",
        vec![Selection::caret(4)],
        EditorCommand::InsertLineBelow,
    );
    assert_eq!(text, "  é\r\n  \r\nlast");
    assert_eq!(selected, vec![Selection::caret(8)]);
    let (text, selected) = run(
        "  雨",
        vec![Selection::caret(3)],
        EditorCommand::InsertLineBelow,
    );
    assert_eq!(text, "  雨\n  ");
    assert_eq!(selected, vec![Selection::caret(6)]);
}
#[test]
fn indent_outdent_and_line_selection_preserve_direction_tabs_and_column_zero_exclusion() {
    let source = "\t  a\r\n    b";
    let before = selection(source.chars().count(), 0);
    let (text, selected) = run(
        source,
        vec![before],
        EditorCommand::Outdent(Indentation::Tabs { tab_width: 4 }),
    );
    assert_eq!(text, "  a\r\nb");
    assert_eq!(selected, vec![selection(6, 0)]);
    let (text, selected) = run(
        "one\ntwo\nthree",
        vec![selection(8, 5)],
        EditorCommand::SelectLines,
    );
    assert_eq!(text, "one\ntwo\nthree");
    assert_eq!(selected, vec![selection(8, 4)]);
}
#[test]
fn line_comment_round_trip_uses_common_indent_and_skips_blank_lines() {
    let source = "  a\r\n    b\r\n\r\n  c";
    let before = selection(source.chars().count(), 0);
    let command = EditorCommand::ToggleLineComment(CommentSyntax {
        prefix: "//".into(),
    });
    let (text, selected) = run(source, vec![before], command.clone());
    assert_eq!(text, "  // a\r\n  //   b\r\n\r\n  // c");
    assert!(selected[0].anchor > selected[0].head);
    let (restored, selected) = run(&text, selected, command);
    assert_eq!(restored, source);
    assert_eq!(selected, vec![before]);
}
#[test]
fn block_comment_round_trip_selects_content_and_empty_caret_stays_inside_pair() {
    let command = EditorCommand::ToggleBlockComment {
        open: "/*".into(),
        close: "*/".into(),
    };
    let (text, selected) = run("rain 雨", vec![selection(6, 5)], command.clone());
    assert_eq!(text, "rain /*雨*/");
    assert_eq!(selected, vec![selection(8, 7)]);
    let (text, selected) = run(&text, selected, command.clone());
    assert_eq!(text, "rain 雨");
    assert_eq!(selected, vec![selection(6, 5)]);
    let (text, selected) = run("", vec![Selection::caret(0)], command.clone());
    assert_eq!(text, "/**/");
    assert_eq!(selected, vec![Selection::caret(2)]);
    let (text, selected) = run(&text, selected, command);
    assert_eq!(text, "");
    assert_eq!(selected, vec![Selection::caret(0)]);
}
#[test]
fn join_lines_trims_boundary_indent_keeps_last_eol_and_maps_reverse_selection() {
    let source = "foo  \r\n  bar\r\n \tbaz\r\nlast";
    let (text, selected) = run(source, vec![selection(19, 0)], EditorCommand::JoinLines);
    assert_eq!(text, "foo bar baz\r\nlast");
    assert_eq!(selected, vec![selection(11, 0)]);
}
#[test]
fn delete_to_line_boundaries_and_case_expansion_are_atomic_unicode_edits() {
    let (text, _) = run(
        "é雨\r\nnext",
        vec![Selection::caret(2)],
        EditorCommand::DeleteAllRight,
    );
    assert_eq!(text, "é\r\nnext");
    let (text, _) = run(
        "é雨\r\nnext",
        vec![Selection::caret(2)],
        EditorCommand::DeleteAllLeft,
    );
    assert_eq!(text, "雨\r\nnext");
    let (text, selected) = run(
        "straße é 👩‍💻",
        vec![selection(6, 0)],
        EditorCommand::TransformCase(CaseTransform::Upper),
    );
    assert_eq!(text, "STRASSE é 👩‍💻");
    assert_eq!(selected, vec![selection(7, 0)]);
}
#[test]
fn trim_trailing_whitespace_is_explicit_preserves_crlf_and_remaps_all_carets() {
    let source = "  a \t\r\n \r\n雨\t";
    let (text, selected) = run(
        source,
        vec![Selection::caret(5), Selection::caret(12)],
        EditorCommand::TrimTrailingWhitespace,
    );
    assert_eq!(text, "  a\r\n\r\n雨");
    assert_eq!(selected, vec![Selection::caret(3), Selection::caret(8)]);
}
#[test]
fn invalid_options_grapheme_boundaries_and_limits_preserve_buffer_history_and_redo() {
    let mut buffer = TextBuffer::new("é\r\n👩‍💻".into()).unwrap();
    buffer
        .apply_delta(
            TextDelta {
                range: 0..0,
                insert: "x".into(),
            },
            EditKind::Atomic,
        )
        .unwrap();
    assert!(buffer.undo());
    let before = buffer.snapshot();
    let revision = buffer.revision();
    for (selections, command) in [
        (vec![Selection::caret(1)], EditorCommand::DeleteAllLeft),
        (vec![Selection::caret(3)], EditorCommand::DeleteAllRight),
        (
            vec![Selection::caret(0)],
            EditorCommand::Indent(Indentation::Spaces(0)),
        ),
        (
            vec![Selection::caret(0)],
            EditorCommand::ToggleLineComment(CommentSyntax {
                prefix: "\n".into(),
            }),
        ),
        (vec![Selection::caret(100)], EditorCommand::DeleteLines),
    ] {
        assert!(plan(&before, &selections, command).is_err());
    }
    assert_eq!(buffer.snapshot(), before);
    assert_eq!(buffer.revision(), revision);
    assert!(buffer.can_redo());
    assert!(!buffer.can_undo());
    let huge = TextBuffer::new("x".repeat(8 * 1024 * 1024 + 1)).unwrap();
    assert!(
        plan(
            &huge.snapshot(),
            &[Selection::caret(0)],
            EditorCommand::CopyLines(Direction::Down)
        )
        .is_err()
    );
}
#[test]
fn selection_only_and_boundary_noops_do_not_discard_redo_or_create_text_history() {
    let mut buffer = TextBuffer::new("one\ntwo".into()).unwrap();
    buffer
        .apply_delta(
            TextDelta {
                range: 0..0,
                insert: "x".into(),
            },
            EditKind::Atomic,
        )
        .unwrap();
    buffer.undo();
    let command = plan(
        &buffer.snapshot(),
        &[Selection::caret(0)],
        EditorCommand::MoveLines(Direction::Up),
    )
    .unwrap();
    assert!(command.deltas.is_empty());
    assert!(buffer.can_redo());
    let command = plan(
        &buffer.snapshot(),
        &[Selection::caret(1)],
        EditorCommand::SelectLines,
    )
    .unwrap();
    assert!(command.deltas.is_empty());
    buffer.set_selection(command.selections[0]).unwrap();
    assert!(buffer.can_redo());
}

#[test]
fn insertion_and_line_selection_keep_adjacent_carets_independent() {
    let (text, selected) = run(
        "a\nb",
        vec![Selection::caret(1), Selection::caret(3)],
        EditorCommand::InsertLineBelow,
    );
    assert_eq!(text, "a\n\nb\n");
    assert_eq!(selected, vec![Selection::caret(2), Selection::caret(5)]);
    let (_, selected) = run(
        "a\nb\nc",
        vec![Selection::caret(0), Selection::caret(2)],
        EditorCommand::SelectLines,
    );
    assert_eq!(selected, vec![selection(0, 2), selection(2, 4)]);
    let (text, _) = run(
        "a\nb\nc",
        vec![Selection::caret(0), Selection::caret(2)],
        EditorCommand::JoinLines,
    );
    assert_eq!(text, "a b c");
}

#[test]
fn deleting_final_empty_line_returns_to_start_of_surviving_line_as_observed() {
    let (text, selections) = run(
        "a\nb\n",
        vec![Selection::caret(4)],
        EditorCommand::DeleteLines,
    );
    assert_eq!(text, "a\nb");
    assert_eq!(selections, vec![Selection::caret(2)]);
}

#[test]
fn indent_tracks_original_first_character_but_keeps_intermediate_prefixes_selected() {
    for eol in ["\n", "\r\n"] {
        let source = format!("é 👩‍💻{eol}雨{eol}last");
        let until = format!("é 👩‍💻{eol}雨{eol}").chars().count();
        for (indentation, prefix) in [
            (Indentation::Tabs { tab_width: 4 }, "\t"),
            (Indentation::Spaces(2), "  "),
        ] {
            let expected = format!("{prefix}é 👩‍💻{eol}{prefix}雨{eol}last");
            let added = prefix.chars().count();
            for before in [selection(0, until), selection(until, 0)] {
                let (text, selected) =
                    run(&source, vec![before], EditorCommand::Indent(indentation));
                assert_eq!(text, expected);
                let after = if before.anchor == 0 {
                    selection(added, until + 2 * added)
                } else {
                    selection(until + 2 * added, added)
                };
                assert_eq!(selected, vec![after]);
                let buffer = TextBuffer::new(text.clone()).unwrap();
                let selected_text = buffer.slice(after.range()).unwrap().to_string();
                assert!(selected_text.starts_with("é"));
                assert!(selected_text.contains(&format!("{eol}{prefix}雨")));
                let (restored, selections) =
                    run(&text, selected, EditorCommand::Outdent(indentation));
                assert_eq!(restored, source);
                assert_eq!(selections, vec![before]);
            }
        }
    }
}

#[test]
fn indentation_detection_uses_tabs_majority_space_gcd_and_honest_default() {
    use codisseu_editor::TextSnapshot;
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str("\talpha\n\tbeta\n  gamma\n")),
        Indentation::Tabs { tab_width: 4 }
    );
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str("  alpha\n    beta\n      gamma\n")),
        Indentation::Spaces(2)
    );
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str("\talpha\n  beta\n")),
        Indentation::Spaces(2)
    );
    for source in [
        "",
        "rain\n👩‍💻\n",
        "         nine\n                  eighteen\n",
    ] {
        assert_eq!(
            detect_indentation(&TextSnapshot::from_str(source)),
            Indentation::Spaces(4)
        );
    }
}

#[test]
fn indentation_detection_never_samples_beyond_128_lines_or_32_kib() {
    use codisseu_editor::TextSnapshot;
    let late_tabs = format!("{}{}", "plain\n".repeat(128), "\ttoo late\n".repeat(200));
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str(&late_tabs)),
        Indentation::Spaces(4)
    );
    let oversized_first_line = format!("{}\n{}", "x".repeat(32 * 1024), "\ttoo late\n".repeat(200));
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str(&oversized_first_line)),
        Indentation::Spaces(4)
    );
    let exact_budget = format!("\t{}\n  outside budget", "x".repeat(32 * 1024 - 2));
    assert_eq!(
        detect_indentation(&TextSnapshot::from_str(&exact_budget)),
        Indentation::Tabs { tab_width: 4 }
    );
}
