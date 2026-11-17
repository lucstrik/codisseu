#![cfg(unix)]
use super::tests::{request, setup, wait};
use super::*;
use egui_kittest::kittest::Queryable;
use std::fs;

#[test]
fn copilot_empty_semantic_list_does_not_hide_or_block_a_real_inline_proposal() {
    let (_dir, _, mut h) = setup(1., "ts", "return 42;");
    request(&mut h);
    let doc = h.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    let caret = h
        .state()
        .service
        .view(h.state().service.active_view().unwrap())
        .unwrap()
        .selection()
        .head;
    h.state_mut().completion_popup = Some(crate::completion::Popup::fixture(id, version, caret));
    h.state_mut().service.completion.items.clear();
    h.step();
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nreturn 42;"
    );
    assert!(h.state().completion_popup.is_none());
}

#[test]
fn copilot_hover_keeps_inline_text_without_buttons_or_a_preview_card() {
    let (_dir, _, mut h) = setup(1., "js", "const result = 42;\nreturn result;");
    request(&mut h);
    let r = h.state().editor_bridge.metrics.copilot_ghost_rects[0];
    h.event(egui::Event::PointerMoved(egui::pos2(
        (r[0] + r[2]) / 2.,
        (r[1] + r[3]) / 2.,
    )));
    h.run_steps(4);
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    assert!(h.query_by_label("Aceitar · Tab").is_none());
    assert!(h.query_by_label("Palavra · Ctrl+→").is_none());
    assert!(
        h.query_by_label("Copilot · Tab aceita · Esc descarta")
            .is_none()
    );
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\n"
    );
    h.key_press_modifiers(Modifiers::CTRL, Key::ArrowRight);
    h.step();
    h.run_steps(3);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nconst"
    );
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
}

#[test]
fn copilot_alternative_keyboard_owns_the_native_bracket_text_event() {
    let (_dir, _, mut h) = setup(1., "js", "first");
    let program = PathBuf::from(h.state().service.editor_preferences().copilot_program);
    fs::write(
        program.with_extension("json"),
        serde_json::json!({"suggestions":["first","second"],"delay":0.02}).to_string(),
    )
    .unwrap();
    h.state_mut().copilot_ui.client = None;
    h.state_mut().copilot_ui.attempted = None;
    h.step();
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.authenticated)
    });
    request(&mut h);
    h.input_mut().events.extend([
        egui::Event::ModifiersChanged(Modifiers::ALT),
        egui::Event::Key {
            key: Key::CloseBracket,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
        egui::Event::Text("]".into()),
        egui::Event::Key {
            key: Key::CloseBracket,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::ALT,
        },
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    h.step();
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\n"
    );
    assert_eq!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .as_ref()
            .unwrap()
            .suffix,
        "second"
    );
    assert!(h.query_by_label("Sugestão Copilot: second").is_some());
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nsecond"
    );
}

#[test]
fn copilot_typed_prefix_retains_exact_remainder_without_another_network_request() {
    let (_dir, _, mut h) = setup(1., "php", "return 42;");
    request(&mut h);
    let ticket = h
        .state()
        .copilot_ui
        .client
        .as_ref()
        .unwrap()
        .suggestion
        .as_ref()
        .unwrap()
        .ticket;
    h.event(egui::Event::Text("ret".into()));
    h.step();
    h.step();
    let client = h.state().copilot_ui.client.as_ref().unwrap();
    let suggestion = client
        .suggestion
        .as_ref()
        .expect("matching typing retains remaining proposal");
    assert_eq!(suggestion.ticket, ticket);
    assert_eq!(suggestion.suffix, "urn 42;");
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nreturn 42;"
    );
}
#[test]
fn copilot_ctrl_right_accepts_one_word_and_keeps_remaining_inline() {
    let (_dir, file, mut h) = setup(1., "ts", "const result = 42;\nreturn result;");
    request(&mut h);
    h.key_press_modifiers(Modifiers::CTRL, Key::ArrowRight);
    h.step();
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nconst"
    );
    assert_eq!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .as_ref()
            .unwrap()
            .suffix,
        " result = 42;\nreturn result;"
    );
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    assert_eq!(fs::read_to_string(file).unwrap(), "// owned fixture\n");
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nconst result = 42;\nreturn result;"
    );
}
#[test]
fn copilot_accept_line_preserves_remainder_without_accepting_following_lines() {
    let (_dir, _, mut h) = setup(1., "sql", "SELECT id\nFROM users;");
    request(&mut h);
    h.state_mut().copilot_accept_part(true);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\nSELECT id\n"
    );
    assert_eq!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .as_ref()
            .unwrap()
            .suffix,
        "FROM users;"
    );
    h.key_press(Key::Escape);
    h.step();
    assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
}
#[test]
fn copilot_tab_indents_before_accepting_a_proposal_with_full_tab_stop_indentation() {
    let (_dir, _, mut h) = setup(1., "php", "    return 42;");
    request(&mut h);
    h.key_press(Key::Tab);
    h.step();
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\n    "
    );
    assert_eq!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .as_ref()
            .unwrap()
            .suffix,
        "return 42;"
    );
    h.key_press(Key::Tab);
    h.step();
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\n    return 42;"
    );
}
#[test]
fn copilot_returning_from_settings_triggers_a_new_suggestion_without_retyping() {
    let (_dir, _, mut h) = setup(1., "md", "Documentation");
    request(&mut h);
    h.get_by_label("Copilot").click();
    h.step();
    h.run_steps(5);
    assert!(h.state().copilot_ui.settings);
    assert!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .is_none()
    );
    h.get_by_label("Concluído").click();
    h.step();
    wait(&mut h, |a| a.editor_bridge.metrics.copilot_ghost_visible);
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        "// owned fixture\n"
    );
}
#[test]
fn copilot_keeps_requesting_with_semantic_widget_and_sends_selected_context() {
    let (_dir, _, mut h) = setup(1., "ts", "m(a, b)");
    let doc = h.state().service.active_document().unwrap();
    let (id, version) = (doc.id, doc.version);
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "su".into(),
        })
        .unwrap();
    let version = h.state().service.active_document().unwrap().version;
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version: version,
            selection: Selection::caret(2),
        })
        .unwrap();
    h.step();
    h.state_mut().completion_popup = Some(crate::completion::Popup::fixture(id, version, 2));
    h.state_mut().service.completion.items = vec![codisseu_core::CompletionItem {
        label: "sum".into(),
        detail: None,
        kind: Some(3),
        filter_text: None,
        sort_text: None,
        new_text: "sum".into(),
        text_edit: Some(codisseu_core::CompletionEdit {
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 2,
            new_text: "sum".into(),
        }),
        insert_edit: None,
        additional_edits: vec![],
        unsupported_reason: None,
    }];
    request(&mut h);
    assert!(h.state().completion_popup.is_some());
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    let path =
        PathBuf::from(h.state().service.editor_preferences().copilot_program).with_extension("log");
    let rows = fs::read_to_string(path).unwrap();
    assert!(
        rows.lines()
            .filter_map(|r| serde_json::from_str::<serde_json::Value>(r).ok())
            .any(|r| r["method"] == "textDocument/inlineCompletion"
                && r["params"]["context"]["selectedCompletionInfo"]["text"] == "sum")
    );
    // Changing the selected semantic item hides an incompatible old augmentation.
    h.state_mut().service.completion.items[0].new_text = "subtract".into();
    h.step();
    assert!(!h.state().editor_bridge.metrics.copilot_ghost_visible);
}

#[test]
fn copilot_and_semantic_completion_remain_enabled_with_expanded_main_menu_bar() {
    let (_dir, _, mut h) = setup(1., "php", "return 42;");
    h.get_by_label("Menu principal").click();
    h.step();
    assert!(h.state().main_menu_open);
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    h.step();
    assert!(!egui::Popup::is_any_open(&h.ctx));
    assert!(
        !h.state().copilot_blocked(&h.ctx),
        "Expanded menu bar is not an open menu popup"
    );
    h.event(egui::Event::Text(
        "// typing with menu bar visible\n".into(),
    ));
    h.step();
    wait(&mut h, |a| a.editor_bridge.metrics.copilot_ghost_visible);
}

#[test]
fn copilot_real_menu_popup_remains_blocking_and_inactive_status_is_explained() {
    let (_dir, _, mut h) = setup(1., "php", "return 42;");
    h.get_by_label("Menu principal").click();
    h.step();
    h.get_by_label("Arquivo").click();
    h.step();
    assert!(egui::Popup::is_any_open(&h.ctx));
    assert!(h.state().copilot_blocked(&h.ctx));
    let c = h.state_mut().copilot_ui.client.as_mut().unwrap();
    c.kind = "Inactive".into();
    c.status = "Controlled ignored file".into();
    assert!(
        h.state()
            .copilot_activity()
            .contains("inativo neste arquivo")
    );
    let diagnostic = h.state().copilot_diagnostic(&h.ctx).to_string();
    assert!(!diagnostic.contains("Controlled ignored file"));
    assert!(!diagnostic.contains("file://"));
    assert!(!diagnostic.contains(&h.state().service.editor_preferences().copilot_program));
}

#[test]
#[allow(deprecated)]
fn copilot_ime_lifecycle_and_empty_text_do_not_discard_unchanged_inline_suggestions() {
    let (_dir, _, mut h) = setup(1., "php", "return 42;");
    for event in [
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: String::new(),
            active_range_chars: None,
        }),
        egui::Event::Ime(egui::ImeEvent::Commit(String::new())),
        egui::Event::Ime(egui::ImeEvent::Enabled),
        egui::Event::Ime(egui::ImeEvent::Disabled),
        egui::Event::Text(String::new()),
        egui::Event::Paste(String::new()),
    ] {
        request(&mut h);
        let ticket = h
            .state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .as_ref()
            .unwrap()
            .ticket;
        h.event(event.clone());
        h.run_steps(3);
        assert!(
            h.state().editor_bridge.metrics.copilot_ghost_visible,
            "Lifecycle event hid ghost: {event:?}"
        );
        assert_eq!(
            h.state()
                .copilot_ui
                .client
                .as_ref()
                .unwrap()
                .suggestion
                .as_ref()
                .unwrap()
                .ticket,
            ticket
        );
    }
}
