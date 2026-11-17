//! Explicit live templates over the shared versioned document, without saving.
use super::*;
use codisseu_core::{EditKind, Selection, TextChange, TextDelta, ViewId, snippets};
use std::ops::Range;

#[derive(Default)]
pub(super) struct SnippetUi {
    live: Option<LiveTemplate>,
    popup: Option<TemplatePopup>,
}
impl SnippetUi {
    pub(super) fn has_live(&self) -> bool {
        self.live.is_some()
    }
    pub(super) fn is_open(&self) -> bool {
        self.popup.is_some()
    }
}
struct LiveTemplate {
    document: DocumentId,
    view: ViewId,
    version: u64,
    stops: Vec<snippets::Tabstop>,
    selected: usize,
}
struct TemplatePopup {
    document: DocumentId,
    view: ViewId,
    version: u64,
    caret: usize,
    prefix: Range<usize>,
    choices: Vec<&'static snippets::Snippet>,
    selected: usize,
    anchor: egui::Pos2,
    rect: Option<egui::Rect>,
}

#[derive(Clone)]
pub(super) struct CompletionTemplate {
    pub template: &'static snippets::Snippet,
    pub prefix: Range<usize>,
}

impl LiveTemplate {
    fn update(&mut self, version: u64, change: Option<&TextChange>) -> bool {
        if version == self.version {
            return true;
        }
        if version != self.version + 1 {
            return false;
        }
        let Some(change) = change else {
            return false;
        };
        let selected = self.stops[self.selected].range.clone();
        if change.old_chars.start < selected.start || change.old_chars.end > selected.end {
            return false;
        }
        let delta = change.new_chars.len() as isize - change.old_chars.len() as isize;
        self.stops[self.selected].range.end = selected
            .end
            .checked_add_signed(delta)
            .unwrap_or(selected.start);
        for (index, stop) in self.stops.iter_mut().enumerate() {
            if index == self.selected || stop.range.end <= change.old_chars.start {
                continue;
            }
            if stop.range.start < change.old_chars.end {
                return false;
            }
            let Some(start) = stop.range.start.checked_add_signed(delta) else {
                return false;
            };
            let Some(end) = stop.range.end.checked_add_signed(delta) else {
                return false;
            };
            stop.range = start..end;
        }
        self.version = version;
        true
    }
}

impl CodisseuApp {
    pub(super) fn completion_templates(&self, caret: usize) -> Vec<CompletionTemplate> {
        let Some(doc) = self.service.active_document() else {
            return Vec::new();
        };
        if !matches!(
            snippets::context(&doc.path),
            Some(snippets::Context::Php | snippets::Context::Blade)
        ) {
            return Vec::new();
        }
        let snapshot = doc.snapshot();
        if caret > snapshot.len_chars() {
            return Vec::new();
        }
        let count = snapshot
            .chars_at(caret)
            .reversed()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '@'))
            .take(128)
            .count();
        if count == 0 {
            return Vec::new();
        }
        let prefix = caret - count..caret;
        if !self.editor_template_context(prefix.start) {
            return Vec::new();
        }
        let typed = snapshot.slice(prefix.clone()).to_string();
        let mut choices = snippets::candidates(&doc.path, &typed)
            .map(|template| CompletionTemplate {
                template,
                prefix: prefix.clone(),
            })
            .collect::<Vec<_>>();
        choices.sort_by_key(|choice| {
            (
                choice.template.trigger != typed,
                choice.template.trigger.len(),
            )
        });
        choices
    }

    pub(super) fn accept_completion_template(&mut self, choice: CompletionTemplate) {
        self.apply_template_choice(choice.template, choice.prefix);
    }

    pub(super) fn snippets_tick(&mut self) {
        let current_view = self.service.active_view();
        if let Some(live) = self.snippet_ui.live.as_mut() {
            let valid = self.service.active_document().is_some_and(|doc| {
                let selection = current_view
                    .and_then(|view| self.service.view(view))
                    .map(|view| view.selection());
                if doc.id != live.document
                    || current_view != Some(live.view)
                    || !live.update(doc.version, doc.last_change())
                {
                    return false;
                }
                let range = &live.stops[live.selected].range;
                selection
                    .is_some_and(|s| s.range().start >= range.start && s.range().end <= range.end)
            });
            if !valid {
                self.snippet_ui.live = None;
            }
        }
        if let Some(popup) = &self.snippet_ui.popup {
            let valid = current_view == Some(popup.view)
                && self
                    .service
                    .active_document()
                    .is_some_and(|doc| doc.id == popup.document && doc.version == popup.version)
                && current_view
                    .and_then(|view| self.service.view(view))
                    .is_some_and(|view| view.selection() == Selection::caret(popup.caret));
            if !valid {
                self.snippet_ui.popup = None;
            }
        }
    }

    pub(super) fn snippet_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        self.snippets_tick();
        if !self.editor_focused(ctx) {
            return false;
        }
        if self.snippet_ui.popup.is_some() {
            if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
                self.snippet_ui.popup = None;
                return true;
            }
            for (key, direction) in [(Key::ArrowDown, 1isize), (Key::ArrowUp, -1isize)] {
                if consume_exact(ctx, Modifiers::NONE, key) {
                    let popup = self.snippet_ui.popup.as_mut().unwrap();
                    if !popup.choices.is_empty() {
                        popup.selected = (popup.selected as isize + direction)
                            .rem_euclid(popup.choices.len() as isize)
                            as usize;
                    }
                    return true;
                }
            }
            if consume_exact(ctx, Modifiers::NONE, Key::Enter)
                || consume_exact(ctx, Modifiers::NONE, Key::Tab)
            {
                self.accept_snippet_choice();
                return true;
            }
        }
        let has_templates = self
            .service
            .active_document()
            .is_some_and(|doc| snippets::candidates(&doc.path, "").next().is_some());
        if has_templates && consume_exact(ctx, Modifiers::CTRL, Key::J) {
            self.open_snippet_list();
            return true;
        }
        if self.snippet_ui.live.is_some() {
            if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
                self.snippet_ui.live = None;
                return true;
            }
            let previous = consume_exact(ctx, Modifiers::SHIFT, Key::Tab);
            let next = consume_exact(ctx, Modifiers::NONE, Key::Tab);
            if previous || next {
                let mut live = self.snippet_ui.live.take().unwrap();
                live.selected = if previous {
                    live.selected.saturating_sub(1)
                } else {
                    (live.selected + 1).min(live.stops.len() - 1)
                };
                let stop = &live.stops[live.selected];
                self.send(Command::SetSelection {
                    id: live.document,
                    expected_version: live.version,
                    selection: Selection {
                        anchor: stop.range.start,
                        head: stop.range.end,
                    },
                });
                if stop.number != 0 {
                    self.snippet_ui.live = Some(live);
                }
                self.focus_editor = true;
                return true;
            }
        }
        if self.completion_popup.is_some() {
            return false;
        }
        // Tab keeps its regular indentation behavior unless an exact template exists.
        let expands = self
            .service
            .active_document()
            .zip(self.service.active_view())
            .is_some_and(|(doc, view)| {
                let selection = self.service.view(view).unwrap().selection();
                selection.is_caret()
                    && doc.language_supported()
                    && snippets::expand(&doc.path, doc.text(), selection.head).is_some_and(
                        |expansion| self.editor_template_context(expansion.replace.start),
                    )
            });
        if expands && consume_exact(ctx, Modifiers::NONE, Key::Tab) {
            let doc = self.service.active_document().unwrap();
            let view = self.service.active_view().unwrap();
            let selection = self.service.view(view).unwrap().selection();
            if let Some(expansion) = snippets::expand(&doc.path, doc.text(), selection.head) {
                self.apply_snippet(expansion);
                return true;
            }
        }
        false
    }

    fn open_snippet_list(&mut self) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let Some(view) = self.service.active_view() else {
            return;
        };
        let selection = self.service.view(view).unwrap().selection();
        if !selection.is_caret() || !doc.language_supported() {
            return;
        }
        let snapshot = doc.snapshot();
        let prefix_len = snapshot
            .chars_at(selection.head)
            .reversed()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '@'))
            .count();
        let prefix = selection.head - prefix_len..selection.head;
        if !self.editor_template_context(prefix.start) {
            return;
        }
        let typed = snapshot.slice(prefix.clone()).to_string();
        let choices = snippets::candidates(&doc.path, &typed).collect::<Vec<_>>();
        if choices.is_empty() {
            self.service.status = "Nenhum live template para este arquivo/prefixo".into();
            return;
        }
        self.snippet_ui.popup = Some(TemplatePopup {
            document: doc.id,
            view,
            version: doc.version,
            caret: selection.head,
            prefix,
            choices,
            selected: 0,
            anchor: self.caret_screen.unwrap_or(egui::pos2(360., 120.)),
            rect: None,
        });
        self.completion_popup = None;
        self.send(Command::CancelCompletion);
        self.snippet_ui.live = None;
        self.focus_editor = true;
    }

    fn accept_snippet_choice(&mut self) {
        let Some(popup) = self.snippet_ui.popup.take() else {
            return;
        };
        let Some(template) = popup.choices.get(popup.selected) else {
            return;
        };
        let Some(doc) = self.service.active_document() else {
            return;
        };
        if doc.id != popup.document
            || doc.version != popup.version
            || self.service.active_view() != Some(popup.view)
        {
            return;
        }
        self.apply_template_choice(template, popup.prefix);
    }

    fn apply_template_choice(
        &mut self,
        template: &'static snippets::Snippet,
        prefix: Range<usize>,
    ) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        if !self.editor_template_context(prefix.start) {
            return;
        }
        let mut snapshot = doc.snapshot();
        snapshot.remove(prefix.clone());
        snapshot.insert(prefix.start, template.trigger);
        let text = snapshot.to_string();
        if let Some(mut expansion) = snippets::expand(
            &doc.path,
            &text,
            prefix.start + template.trigger.chars().count(),
        ) {
            expansion.replace = prefix;
            self.apply_snippet(expansion);
        }
    }

    fn apply_snippet(&mut self, expansion: snippets::Expansion) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let id = doc.id;
        let version = doc.version;
        let Some(view) = self.service.active_view() else {
            return;
        };
        let before = self.service.view(view).unwrap().selection();
        let start = expansion.replace.start;
        let stops = expansion
            .tabstops
            .into_iter()
            .map(|mut stop| {
                stop.range.start += start;
                stop.range.end += start;
                stop
            })
            .collect::<Vec<_>>();
        let after = stops.first().map_or(
            Selection::caret(start + expansion.text.chars().count()),
            |stop| Selection {
                anchor: stop.range.start,
                head: stop.range.end,
            },
        );
        let command = Command::EditTransaction {
            id,
            expected_version: version,
            delta: TextDelta {
                range: expansion.replace,
                insert: expansion.text,
            },
            kind: EditKind::Atomic,
            before,
            after,
        };
        match self.service.dispatch(command) {
            Ok(()) => {
                let version = self.service.active_document().unwrap().version;
                self.snippet_ui.live = if stops.first().is_some_and(|stop| stop.number != 0) {
                    Some(LiveTemplate {
                        document: id,
                        view,
                        version,
                        stops,
                        selected: 0,
                    })
                } else {
                    None
                };
                self.focus_editor = true;
            }
            Err(error) => self.service.last_error = Some(error),
        }
    }

    pub(super) fn snippets_window(&mut self, ctx: &egui::Context) {
        self.snippets_tick();
        let Some(popup) = self.snippet_ui.popup.as_ref() else {
            return;
        };
        if popup.rect.is_some_and(|rect| {
            ctx.input(|i| {
                i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
            })
        }) {
            self.snippet_ui.popup = None;
            return;
        }
        let available = ctx.content_rect();
        let width = 440_f32.min((available.width() - 24.).max(160.));
        let height = popup.choices.len().min(8) as f32 * 26. + 30.;
        let x = popup
            .anchor
            .x
            .min(available.right() - width - 8.)
            .max(available.left() + 8.);
        let y = if popup.anchor.y + height < available.bottom() {
            popup.anchor.y
        } else {
            (popup.anchor.y - height - self.tokens.editor_line_height).max(available.top() + 8.)
        };
        let choices = popup.choices.clone();
        let selected = popup.selected;
        let mut chosen = None;
        let area = egui::Area::new(egui::Id::unique("codisseu.live.templates"))
            .order(egui::Order::Foreground)
            .movable(false)
            .sense(egui::Sense::hover())
            .fixed_pos(egui::pos2(x, y))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(color(self.tokens.panel))
                    .show(ui, |ui| {
                        ui.set_width(width - 16.);
                        ui.spacing_mut().item_spacing.y = 0.;
                        egui::ScrollArea::vertical()
                            .max_height(208.)
                            .codisseu()
                            .show(ui, |ui| {
                                for (index, item) in choices.iter().enumerate() {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(ui.available_width(), 26.),
                                        egui::Sense::click(),
                                    );
                                    if selected == index {
                                        ui.painter().rect_filled(
                                            rect,
                                            3.,
                                            color(self.tokens.selection),
                                        );
                                    }
                                    let text_color = color(self.tokens.text);
                                    components::paint_icon(
                                        ui,
                                        egui::Rect::from_center_size(
                                            rect.left_center() + egui::vec2(12., 0.),
                                            egui::vec2(16., 16.),
                                        ),
                                        Icon::Code,
                                        text_color,
                                    );
                                    ui.painter().with_clip_rect(rect).text(
                                        rect.left_center() + egui::vec2(26., 0.),
                                        egui::Align2::LEFT_CENTER,
                                        item.trigger,
                                        FontId::monospace(13.),
                                        text_color,
                                    );
                                    ui.painter()
                                        .with_clip_rect(egui::Rect::from_min_max(
                                            rect.min + egui::vec2(150., 0.),
                                            rect.max,
                                        ))
                                        .text(
                                            rect.left_center() + egui::vec2(150., 0.),
                                            egui::Align2::LEFT_CENTER,
                                            item.description,
                                            FontId::proportional(12.),
                                            color(self.tokens.muted),
                                        );
                                    response.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::Role::ListBoxOption,
                                            true,
                                            selected == index,
                                            format!("{} · {}", item.trigger, item.description),
                                        )
                                    });
                                    if selected == index {
                                        response.scroll_to_me(None);
                                    }
                                    if response.clicked() {
                                        chosen = Some(index);
                                    }
                                }
                            });
                        ui.add_space(6.);
                        ui.label(
                            RichText::new("Live Templates    Enter / Tab to insert")
                                .small()
                                .color(color(self.tokens.muted)),
                        );
                    });
            });
        if let Some(popup) = self.snippet_ui.popup.as_mut() {
            popup.rect = Some(area.response.rect);
        }
        if let Some(index) = chosen {
            self.snippet_ui.popup.as_mut().unwrap().selected = index;
            self.accept_snippet_choice();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn fixture(name: &str, text: &str) -> (tempfile::TempDir, Harness<'static, CodisseuApp>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, text).unwrap();
        let mut service = AppService::new(dir.path().join("session.json"));
        service
            .dispatch(Command::OpenWorkspace(dir.path().to_owned()))
            .unwrap();
        service.dispatch(Command::OpenFile(path)).unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(2));
        }
        let doc = service.active_document().unwrap();
        let (id, version, caret) = (doc.id, doc.version, doc.len_chars());
        service
            .dispatch(Command::SetSelection {
                id,
                expected_version: version,
                selection: Selection::caret(caret),
            })
            .unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 900.])
            .with_max_steps(40)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        for _ in 0..5 {
            h.step();
        }
        h.get_by_role(egui::accesskit::Role::MultilineTextInput)
            .focus();
        h.step();
        (dir, h)
    }
    fn key(h: &mut Harness<'_, CodisseuApp>, mut modifiers: Modifiers, key: Key) {
        modifiers.command = modifiers.ctrl;
        h.key_press_modifiers(modifiers, key);
        h.step();
        h.step();
    }
    fn selected(h: &Harness<'_, CodisseuApp>) -> String {
        let service = &h.state().service;
        let selection = service
            .view(service.active_view().unwrap())
            .unwrap()
            .selection()
            .range();
        service
            .active_document()
            .unwrap()
            .snapshot()
            .slice(selection)
            .to_string()
    }

    #[test]
    fn php_template_expands_navigates_tracks_edits_and_undo_preserves_disk() {
        let initial = "<?php\n// ação 😀\nlroute";
        let (dir, mut h) = fixture("routes.php", initial);
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .text()
                .contains("Route::get(")
        );
        assert_eq!(selected(&h), "get");
        h.event(egui::Event::Text("post".into()));
        h.step();
        h.step();
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), "/path");
        key(&mut h, Modifiers::SHIFT, Key::Tab);
        assert_eq!(selected(&h), "post");
        key(&mut h, Modifiers::NONE, Key::Escape);
        assert!(h.state().snippet_ui.live.is_none());
        key(&mut h, Modifiers::CTRL, Key::Z);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(h.state().service.active_document().unwrap().text(), initial);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("routes.php")).unwrap(),
            initial
        );
    }

    #[test]
    fn blade_template_and_partial_prefix_picker_use_the_shared_editor() {
        let (_dir, mut h) = fixture("welcome.blade.php", "😀\n@foreach");
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), "$items");
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), "$item");
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert!(h.state().snippet_ui.live.is_none());
        assert!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .text()
                .contains("@endforeach")
        );
        let (_dir, mut h) = fixture("routes.php", "<?php\nlro");
        key(&mut h, Modifiers::CTRL, Key::J);
        assert!(h.state().snippet_ui.is_open());
        assert_eq!(
            h.state().snippet_ui.popup.as_ref().unwrap().choices[0].trigger,
            "lroute"
        );
        key(&mut h, Modifiers::NONE, Key::Enter);
        assert!(!h.state().snippet_ui.is_open());
        assert_eq!(
            selected(&h),
            "get",
            "text={:?}, live={:?}",
            h.state().service.active_document().unwrap().text(),
            h.state()
                .snippet_ui
                .live
                .as_ref()
                .map(|s| (s.version, s.selected))
        );
        assert!(
            !h.state()
                .service
                .active_document()
                .unwrap()
                .text()
                .contains("${")
        );
    }

    #[test]
    fn placeholder_mapping_uses_source_positions_and_rejects_outside_or_stale_edits() {
        let mut live = LiveTemplate {
            document: 1,
            view: 1,
            version: 3,
            selected: 1,
            stops: vec![
                snippets::Tabstop {
                    number: 1,
                    range: 2..4,
                },
                snippets::Tabstop {
                    number: 2,
                    range: 20..24,
                },
                snippets::Tabstop {
                    number: 0,
                    range: 10..10,
                },
            ],
        };
        let change = TextChange {
            revision: 1,
            old_lines: 0..1,
            new_lines: 0..1,
            old_chars: 20..24,
            new_chars: 20..26,
        };
        assert!(live.update(4, Some(&change)));
        assert_eq!(live.stops[1].range, 20..26);
        assert_eq!(live.stops[2].range, 10..10);
        assert!(!live.update(6, Some(&change)));
        let outside = TextChange {
            old_chars: 0..1,
            new_chars: 0..2,
            ..change
        };
        assert!(!live.update(5, Some(&outside)));
    }

    #[test]
    fn php_templates_in_completion_expand_before_keywords_and_keep_atomic_undo() {
        let original = "<?php\n    fore";
        let (dir, mut h) = fixture("source.php", original);
        key(&mut h, Modifiers::CTRL, Key::Space);
        assert!(h.state().completion_has_choices());
        assert!(h.query_by_label("fore · Live Template").is_some());
        assert!(
            h.state().copilot_completion_choice().is_none(),
            "template placeholders are not semantic completion context"
        );
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), "$items");
        assert!(
            h.state()
                .service
                .active_document()
                .unwrap()
                .text()
                .contains("foreach ($items as $item) {\n        ")
        );
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert_eq!(selected(&h), "$item");
        key(&mut h, Modifiers::NONE, Key::Escape);
        key(&mut h, Modifiers::CTRL, Key::Z);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            original
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("source.php")).unwrap(),
            original
        );
    }

    #[test]
    fn php_template_completion_respects_comments_strings_and_member_context() {
        for original in [
            "<?php\n// fore",
            "<?php\n/*\nfore",
            "<?php\n$value = 'fore",
            "<?php\n$value->fore",
            "<?php\nSomeClass::fore",
            "<?php\ncall(fore",
            "<?php\ncall(value: fore",
        ] {
            let (_dir, mut h) = fixture("source.php", original);
            for _ in 0..20 {
                h.step();
                thread::sleep(Duration::from_millis(2));
            }
            let caret = h.state().service.active_document().unwrap().len_chars();
            assert!(
                h.state().completion_templates(caret).is_empty(),
                "{original}"
            );
            key(&mut h, Modifiers::NONE, Key::Tab);
            assert!(
                !h.state()
                    .service
                    .active_document()
                    .unwrap()
                    .text()
                    .contains("foreach ("),
                "{original}"
            );
        }
    }
}
