use crate::scrollbars::ScrollAreaExt as _;
use crate::{CodisseuApp, Command, color, egui};
use codisseu_core::{DocumentId, Language, ProviderState, ViewId};
use std::time::Duration;

const HOVER_DELAY: f64 = 0.5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HoverKey {
    document: DocumentId,
    view: ViewId,
    version: u64,
    character: usize,
}

struct Candidate {
    key: HoverKey,
    since: f64,
    requested: bool,
}
#[derive(Default)]
pub(super) struct HoverUi {
    candidate: Option<Candidate>,
    automatic: bool,
    rect: Option<egui::Rect>,
    seen: u64,
    last_inside: f64,
    content: crate::markdown::RemoteMarkdown,
}
impl HoverUi {
    pub(super) fn is_automatic(&self) -> bool {
        self.automatic
    }
}

impl CodisseuApp {
    pub(crate) fn request_documentation(&mut self) {
        self.documentation_hover.automatic = false;
        self.documentation_hover.rect = None;
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let version = doc.version;
        let character_index = self
            .caret
            .filter(|(owner, _)| *owner == id)
            .map_or(0, |(_, index)| index);
        self.documentation_anchor = self.caret_screen.unwrap_or(egui::pos2(
            self.tokens.sidebar_width + self.tokens.rail_width,
            self.tokens.main_toolbar_height + self.tokens.tab_height,
        )) + egui::vec2(0.0, self.tokens.row_height);
        self.documentation = Some((id, version));
        self.documentation_error = self
            .service
            .dispatch(Command::Hover {
                id,
                expected_version: version,
                character_index,
            })
            .err();
    }

    pub(crate) fn close_documentation(&mut self) {
        let automatic = self.documentation_hover.automatic;
        self.documentation_hover.automatic = false;
        self.documentation_hover.rect = None;
        self.documentation = None;
        self.documentation_hover.content.clear();
        self.documentation_error = None;
        self.send(Command::CancelHover);
        if !automatic {
            self.focus_editor = true;
        }
    }

    /// The renderer supplies a genuine source identifier, never a guessed column.
    /// This query does not move the caret or take keyboard focus from the editor.
    pub(super) fn documentation_hover_target(
        &mut self,
        ctx: &egui::Context,
        view: ViewId,
        document: DocumentId,
        version: u64,
        target: Option<(usize, egui::Pos2)>,
    ) {
        self.documentation_hover.seen = ctx.cumulative_frame_nr();
        let over_popup = self.documentation_hover.automatic
            && self.documentation_hover.rect.is_some_and(|rect| {
                ctx.input(|input| {
                    input
                        .pointer
                        .hover_pos()
                        .is_some_and(|point| rect.contains(point))
                })
            });
        let language_ready = self.service.active_document().is_some_and(|doc| {
            doc.id == document
                && doc.version == version
                && Language::for_path(&doc.path).is_some_and(|language| {
                    self.service.language.statuses.iter().any(|status| {
                        status.language == language && status.state == ProviderState::Ready
                    })
                })
        });
        let interrupted = ctx.input(|input| {
            input.pointer.any_down()
                || input.is_scrolling()
                || input.modifiers.any()
                || input.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Cut
                    ) || (matches!(event, egui::Event::Ime(_))
                        && crate::copilot::text_change_event(event))
                })
        });
        let blocked = !language_ready
            || (interrupted && !over_popup)
            || self.service.active_view() != Some(view)
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some())
            || self.path_dialog.is_some()
            || self.provider_dialog
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.project_search.open
            || self.snippet_ui.is_open()
            || self.snippet_ui.has_live()
            || self.completion_popup.is_some()
            || self.rename_editor_input_blocked();
        if blocked {
            if self.documentation_hover.automatic {
                self.close_documentation();
            }
            // Keep a dismissed symbol armed as already requested: a stationary
            // pointer must not reopen a tooltip while the user types elsewhere.
            if let Some(candidate) = &mut self.documentation_hover.candidate {
                candidate.requested = true;
            }
            return;
        }
        if self.documentation_hover.automatic
            && self
                .documentation_hover
                .candidate
                .as_ref()
                .is_some_and(|candidate| {
                    candidate.key.document == document
                        && candidate.key.view == view
                        && candidate.key.version == version
                })
            && over_popup
        {
            self.documentation_hover.last_inside = ctx.input(|input| input.time);
            return;
        }
        let Some((character, anchor)) = target else {
            if self.documentation_hover.automatic {
                let remaining =
                    0.2 - (ctx.input(|input| input.time) - self.documentation_hover.last_inside);
                if self.documentation_hover.rect.is_some() && remaining > 0. {
                    ctx.request_repaint_after(Duration::from_secs_f64(remaining));
                    return;
                }
                self.close_documentation();
            }
            self.documentation_hover.candidate = None;
            return;
        };
        // Explicit Ctrl+Q remains independent of pointer movement.
        if self.documentation.is_some() && !self.documentation_hover.automatic {
            return;
        }
        let now = ctx.input(|input| input.time);
        self.documentation_hover.last_inside = now;
        let key = HoverKey {
            document,
            view,
            version,
            character,
        };
        if self
            .documentation_hover
            .candidate
            .as_ref()
            .is_none_or(|c| c.key != key)
        {
            if self.documentation_hover.automatic {
                self.close_documentation();
            }
            self.documentation_hover.candidate = Some(Candidate {
                key,
                since: now,
                requested: false,
            });
        }
        let candidate = self.documentation_hover.candidate.as_mut().unwrap();
        if candidate.requested {
            return;
        }
        let remaining = HOVER_DELAY - (now - candidate.since);
        if remaining > 0. {
            ctx.request_repaint_after(Duration::from_secs_f64(remaining));
            return;
        }
        candidate.requested = true;
        self.documentation_hover.automatic = true;
        self.documentation_anchor = anchor + egui::vec2(0., self.tokens.editor_line_height);
        self.documentation = Some((document, version));
        self.documentation_error = self
            .service
            .dispatch(Command::PreviewHover {
                id: document,
                expected_version: version,
                character_index: character,
            })
            .err();
        ctx.request_repaint();
    }

    pub(crate) fn documentation_window(&mut self, ctx: &egui::Context) {
        if self.documentation_hover.automatic
            && self.documentation_hover.seen != ctx.cumulative_frame_nr()
        {
            self.close_documentation();
        }
        let Some((id, version)) = self.documentation else {
            return;
        };
        let Some(doc) = self
            .service
            .active_document()
            .filter(|d| d.id == id && d.version == version)
        else {
            self.close_documentation();
            return;
        };
        let extension = crate::highlight::syntax_key(&doc.path);
        let language = &self.service.language;
        let error = self
            .documentation_error
            .as_ref()
            .or(language.hover_error.as_ref());
        let automatic = self.documentation_hover.automatic;
        if automatic && (error.is_some() || language.hover_pending()) {
            return;
        }
        let text = if let Some(error) = error {
            error.as_str()
        } else if language.hover_pending() {
            "Consultando provider…"
        } else if let Some(result) = language
            .hover
            .as_ref()
            .filter(|h| h.path == doc.path && h.version == version)
        {
            if result.text.is_empty() && automatic {
                return;
            } else if result.text.is_empty() {
                "Sem documentação nesta posição."
            } else {
                &result.text
            }
        } else {
            "Consulta encerrada. Use Ctrl+Q para consultar novamente."
        };
        self.documentation_hover.content.set(text.to_owned());
        let mut open = true;
        let response = egui::Window::new(
            egui::RichText::new("Documentação do símbolo").size(self.tokens.ui_font_size),
        )
        .id(egui::Id::unique("codisseu.documentation"))
        .open(&mut open)
        .current_pos(self.documentation_anchor)
        .movable(false)
        .frame(
            egui::Frame::window(&ctx.style_of(ctx.theme()))
                .fill(color(self.tokens.navigation_popup_background))
                .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
                .inner_margin(self.tokens.documentation_padding),
        )
        .default_width(self.tokens.documentation_width)
        .max_width((ctx.content_rect().width() - 24.).max(120.))
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("LSP · documento atual")
                    .small()
                    .color(color(self.tokens.muted)),
            );
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(self.tokens.documentation_max_height)
                .codisseu()
                .show(ui, |ui| {
                    if error.is_some() || language.hover_pending() {
                        ui.add(
                            egui::Label::new(egui::RichText::new(text).color(color(
                                if error.is_some() {
                                    self.tokens.error
                                } else {
                                    self.tokens.text
                                },
                            )))
                            .wrap()
                            .selectable(true),
                        );
                    } else {
                        self.documentation_hover.content.show_documentation(
                            ui,
                            &self.tokens,
                            &extension,
                        );
                    }
                });
        });
        self.documentation_hover.rect = response.map(|response| response.response.rect);
        if !open {
            self.close_documentation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::{AppService, ProviderConfig, Selection};
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{fs, thread, time::Instant};

    type H = Harness<'static, CodisseuApp>;
    fn settle(h: &mut H, check: impl Fn(&CodisseuApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            h.step();
            if check(h.state()) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{:?}",
                h.state().service.language.statuses
            );
            thread::sleep(Duration::from_millis(3));
        }
    }
    fn fixture(source: &str) -> (tempfile::TempDir, H) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.php");
        fs::write(&path, source).unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service
            .dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        while service.is_busy() {
            service.poll();
            thread::sleep(Duration::from_millis(2));
        }
        service.dispatch(Command::OpenFile(path)).unwrap();
        while service.is_busy() {
            service.poll();
            thread::sleep(Duration::from_millis(2));
        }
        service
            .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
                language: Language::Php,
                program: "python3".into(),
                args: vec![
                    format!(
                        "{}/tests/fixtures/php_assistance_server.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    dir.path().join("protocol.jsonl").display().to_string(),
                ],
            }]))
            .unwrap();
        service.dispatch(Command::StartProviders).unwrap();
        let mut h = Harness::builder()
            .with_size([1100., 800.])
            .with_step_dt(1. / 60.)
            .with_max_steps(80)
            .build_ui_state(
                |ui, app: &mut CodisseuApp| app.show(ui),
                CodisseuApp::new(service),
            );
        settle(&mut h, |app| {
            app.service
                .language
                .statuses
                .iter()
                .any(|status| status.state == ProviderState::Ready)
                && !app.service.is_busy()
        });
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        (dir, h)
    }
    fn point_at(h: &mut H, line: usize, index: usize) -> egui::Pos2 {
        let rect = h
            .get_by_role(egui::accesskit::Role::MultilineTextInput)
            .rect();
        let line_height = h.state().tokens.editor_line_height;
        for x in (1..500).step_by(2) {
            let point = rect.min + egui::vec2(x as f32, (line as f32 + 0.5) * line_height);
            h.hover_at(point);
            h.step();
            if h.state()
                .documentation_hover
                .candidate
                .as_ref()
                .is_some_and(|candidate| candidate.key.character == index)
            {
                return point;
            }
        }
        panic!("source identifier {index} not hit by pointer");
    }
    fn count_requests(dir: &tempfile::TempDir) -> usize {
        fs::read_to_string(dir.path().join("protocol.jsonl"))
            .unwrap()
            .lines()
            .filter(|line| line.contains("textDocument/hover"))
            .count()
    }

    #[test]
    fn rich_popup_wraps_long_php_signatures_and_stays_inside_small_viewport() {
        let source = "<?php\nclass Main { function run() {} }\n";
        let (_dir, mut h) = fixture(source);
        h.set_size(egui::vec2(850., 600.));
        let doc = h.state().service.active_document().unwrap();
        let (id, version, path) = (doc.id, doc.version, doc.path.clone());
        let signature = format!(
            "<?php public function {}(string $name): void;",
            "LongMethodName".repeat(24)
        );
        let text =
            format!("### Long class\n\nCustomer documentation.\n\n```php\n{signature}\n```\n");
        let app = h.state_mut();
        app.service.language.hover = Some(serde_json::from_value(serde_json::json!({"path":path,"version":version,"line":1,"character":1,"text":text})).unwrap());
        app.documentation = Some((id, version));
        app.documentation_anchor = egui::pos2(820., 570.);
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            h.step();
            if h.query_by_label_contains("LongMethodName").is_some() {
                break;
            }
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(3));
        }
        // Allow egui's area constraints to settle after async rich layout changes height.
        h.run_steps(3);
        let rect = h.state().documentation_hover.rect.unwrap();
        assert!(
            rect.left() >= 0. && rect.right() <= 850. && rect.top() >= 0. && rect.bottom() <= 600.,
            "{rect:?}"
        );
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        assert_eq!(
            h.state().service.active_document().unwrap().version,
            version
        );
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().documentation.is_none());
        assert!(
            h.get_by_role(egui::accesskit::Role::MultilineTextInput)
                .is_focused()
        );
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        settle(&mut h, |app| !app.service.is_busy());
    }

    #[test]
    fn pointer_documentation_waits_preserves_selection_and_escape_does_not_reopen() {
        let source = "<?php\n/* 😀 e\u{301} */ $greeter->greet('Ada');\n";
        let (dir, mut h) = fixture(source);
        let expected = source[..source.find("greet('").unwrap()].chars().count();
        let before = h.state().service.active_document().unwrap().selection();
        let version = h.state().service.active_document().unwrap().version;
        let point = point_at(&mut h, 1, expected);
        assert!(h.state().documentation.is_none(), "delay must elapse first");
        for _ in 0..35 {
            h.step();
        }
        settle(&mut h, |app| app.documentation_hover.rect.is_some());
        assert!(
            h.query_by_label_contains("Fixture method documentation")
                .is_some()
        );
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            before
        );
        assert_eq!(
            h.state().service.active_document().unwrap().version,
            version
        );
        assert!(
            h.get_by_role(egui::accesskit::Role::MultilineTextInput)
                .is_focused()
        );
        let requests = count_requests(&dir);
        for _ in 0..60 {
            h.step();
        }
        assert_eq!(
            count_requests(&dir),
            requests,
            "idle does not repeat requests"
        );
        h.key_press(egui::Key::Escape);
        h.step();
        for _ in 0..40 {
            h.step();
        }
        assert!(h.state().documentation.is_none());
        assert_eq!(
            count_requests(&dir),
            requests,
            "Escape suppresses the same symbol"
        );
        h.hover_at(point + egui::vec2(400., 0.));
        h.step();
        assert!(h.state().documentation_hover.candidate.is_none());
        assert_eq!(
            fs::read_to_string(dir.path().join("source.php")).unwrap(),
            source
        );
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        settle(&mut h, |app| !app.service.is_busy());
    }

    #[test]
    fn pointer_popup_can_be_read_and_new_revision_invalidates_old_documentation() {
        let source = "<?php\n$greeter->greet('Ada');\n";
        let (_dir, mut h) = fixture(source);
        let expected = source.find("greet('").unwrap();
        point_at(&mut h, 1, expected);
        for _ in 0..35 {
            h.step();
        }
        settle(&mut h, |app| app.documentation_hover.rect.is_some());
        let rect = h.state().documentation_hover.rect.unwrap();
        h.hover_at(rect.center());
        for _ in 0..45 {
            h.step();
        }
        assert!(h.state().documentation.is_some());
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: rect.center(),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            assert!(
                h.state().documentation.is_some(),
                "selecting documentation must not close it"
            );
        }
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -40.),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        h.step();
        assert!(
            h.state().documentation.is_some(),
            "reading/scrolling documentation must preserve it"
        );
        let doc = h.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        h.state_mut()
            .service
            .dispatch(Command::Edit {
                id,
                expected_version: version,
                text: source.replace("Ada", "Updated"),
            })
            .unwrap();
        h.step();
        assert!(h.state().documentation.is_none());
        h.state_mut()
            .service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version + 1,
                selection: Selection::caret(0),
            })
            .unwrap();
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            Selection::caret(0)
        );
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        settle(&mut h, |app| !app.service.is_busy());
    }

    #[test]
    fn passive_documentation_preserves_parameter_info() {
        let (_dir, mut h) = fixture("<?php\n$greeter->greet(");
        let doc = h.state().service.active_document().unwrap();
        let (id, version, caret) = (doc.id, doc.version, doc.len_chars());
        h.state_mut()
            .service
            .dispatch(Command::SignatureHelp {
                id,
                expected_version: version,
                character_index: caret,
                trigger: codisseu_core::SignatureHelpTrigger::Invoked,
                is_retrigger: false,
            })
            .unwrap();
        settle(&mut h, |app| !app.service.signature_help.is_pending());
        assert!(
            h.state().service.signature_help.result.is_some(),
            "{:?}",
            h.state().service.signature_help.error
        );
        h.state_mut()
            .service
            .dispatch(Command::PreviewHover {
                id,
                expected_version: version,
                character_index: "<?php\n$greeter->".len(),
            })
            .unwrap();
        settle(&mut h, |app| !app.service.language.hover_pending());
        assert!(
            h.state().service.signature_help.result.is_some(),
            "passive hover must preserve signature"
        );
        assert!(h.state().service.language.hover.is_some());
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        settle(&mut h, |app| !app.service.is_busy());
    }

    #[test]
    fn completion_and_parameter_info_coexist_while_typing_and_when_list_is_empty() {
        let (_dir, mut h) = fixture("<?php\n$greeter->greet('Storm', 2);");
        let doc = h.state().service.active_document().unwrap();
        let (id, version) = (doc.id, doc.version);
        let caret = doc.text().find("Storm").unwrap() + 2;
        h.state_mut()
            .service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version,
                selection: Selection::caret(caret),
            })
            .unwrap();
        h.step();
        h.state_mut()
            .request_signature(codisseu_core::SignatureHelpTrigger::Invoked, false);
        settle(&mut h, |app| app.service.signature_help.result.is_some());
        h.state_mut().request_completion();
        settle(&mut h, |app| !app.service.completion.is_pending());
        assert!(!h.state().service.completion.items.is_empty());
        assert!(h.state().signature_popup.is_some());
        assert!(h.state().service.signature_help.result.is_some());
        h.event(egui::Event::Text("X".into()));
        h.step();
        for _ in 0..100 {
            h.step();
            thread::sleep(Duration::from_millis(2));
        }
        let version = h.state().service.active_document().unwrap().version;
        assert!(
            h.state().signature_popup.is_some(),
            "empty automatic completion cannot dismiss parameter info"
        );
        assert_eq!(
            h.state()
                .service
                .signature_help
                .result
                .as_ref()
                .map(|result| result.version),
            Some(version)
        );
        // Explicit requests and their keyboard handling share the same policy.
        h.state_mut().request_completion();
        assert!(h.state().completion_popup.is_some());
        assert!(h.state().signature_popup.is_some());
        settle(&mut h, |app| !app.service.completion.is_pending());
        assert!(h.state().service.signature_help.result.is_some());
        assert!(h.state().signature_popup.is_some());
        h.state_mut()
            .service
            .dispatch(Command::StopProviders)
            .unwrap();
        settle(&mut h, |app| !app.service.is_busy());
    }
}
