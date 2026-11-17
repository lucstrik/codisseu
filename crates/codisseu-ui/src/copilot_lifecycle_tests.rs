//! Regressions for input that leaves the source/caret unchanged and Markdown modes.
#![cfg(unix)]
use super::tests::{request, setup, wait};
use super::*;
use egui_kittest::kittest::Queryable;

#[test]
fn copilot_noop_navigation_preserves_received_proposal_and_request_ticket() {
    for extension in ["php", "js", "sql", "md"] {
        let (_dir, file, mut h) = setup(1., extension, "remaining");
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
        let before = std::fs::read(&file).unwrap();
        // At EOF on an empty last line, End/Delete/Down have no source effect.
        for key in [Key::End, Key::Delete, Key::ArrowDown] {
            h.key_press(key);
            h.run_steps(3);
            assert!(
                h.state().editor_bridge.metrics.copilot_ghost_visible,
                "{extension}: {key:?} hid an unchanged proposal"
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
            assert_eq!(std::fs::read(&file).unwrap(), before);
        }
        h.key_press(Key::Tab);
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            "// owned fixture\nremaining"
        );
    }
}

#[test]
fn copilot_noop_navigation_does_not_cancel_an_inflight_request() {
    let (_dir, _, mut h) = setup(1., "md", "remaining");
    let program = PathBuf::from(h.state().service.editor_preferences().copilot_program);
    std::fs::write(
        program.with_extension("json"),
        serde_json::json!({"suggestion":"remaining","old_delay":0.5,"delay":0.5}).to_string(),
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
    h.key_press_modifiers(Modifiers::ALT, Key::Backslash);
    h.step();
    assert!(h.state().copilot_ui.client.as_ref().unwrap().busy);
    h.key_press(Key::End);
    h.step();
    assert!(
        h.state().copilot_ui.client.as_ref().unwrap().busy,
        "unchanged caret cancelled the request"
    );
    wait(&mut h, |a| {
        a.copilot_ui
            .client
            .as_ref()
            .is_some_and(|c| c.suggestion.is_some())
    });
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
}

#[test]
fn copilot_markdown_source_and_split_share_the_editor_and_preview_suspends_it() {
    use crate::markdown::MarkdownMode;
    let (_dir, file, mut h) = setup(1., "md", "remaining");
    request(&mut h);
    let view = h.state().service.active_view().unwrap();
    h.state_mut()
        .markdown
        .set_mode(view, &file, MarkdownMode::Split);
    h.run_steps(3);
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
    assert!(h.query_by_label("Sugestão Copilot: remaining").is_some());
    h.state_mut()
        .markdown
        .set_mode(view, &file, MarkdownMode::Preview);
    h.run_steps(3);
    assert!(h.state().copilot_blocked(&h.ctx));
    assert!(
        h.state()
            .copilot_ui
            .client
            .as_ref()
            .unwrap()
            .suggestion
            .is_none()
    );
    h.state_mut()
        .markdown
        .set_mode(view, &file, MarkdownMode::Source);
    h.state_mut().focus_editor = true;
    h.run_steps(3);
    request(&mut h);
    assert!(h.state().editor_bridge.metrics.copilot_ghost_visible);
}
