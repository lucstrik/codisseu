use super::*;
use codisseu_core::{
    AppService, EditKind, TextDelta, next_grapheme, next_word, previous_grapheme, previous_word,
    word_range,
};

pub(super) fn dispatch(s: &mut AppService, c: Command) -> bool {
    match s.dispatch(c) {
        Ok(()) => true,
        Err(e) => {
            s.last_error = Some(e);
            false
        }
    }
}
pub(super) fn select(s: &mut AppService, id: DocumentId, selection: Selection) {
    let Some(d) = s.active_document().filter(|d| d.id == id) else {
        return;
    };
    let version = d.version;
    dispatch(
        s,
        Command::SetSelection {
            id,
            expected_version: version,
            selection,
        },
    );
}
fn edit(
    s: &mut AppService,
    id: DocumentId,
    range: std::ops::Range<usize>,
    text: String,
    kind: EditKind,
) {
    let Some(d) = s.active_document().filter(|d| d.id == id) else {
        s.last_error = Some("O documento ativo mudou; selecione o editor novamente".into());
        return;
    };
    let version = d.version;
    let before = d.selection();
    let after = Selection::caret(range.start + text.chars().count());
    dispatch(
        s,
        Command::EditTransaction {
            id,
            expected_version: version,
            delta: TextDelta {
                range,
                insert: text,
            },
            kind,
            before,
            after,
        },
    );
}
fn pointer(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    view: &mut EditorView,
    p: egui::Pos2,
    origin: egui::Pos2,
    ext: &str,
    t: &Tokens,
) -> Option<usize> {
    let row = ((p.y - origin.y) / t.editor_line_height).floor().max(0.) as usize;
    let Some((line, x)) = soft_wrap::pointer_position(view, row, p.x - origin.x) else {
        return view.ghost.as_ref().map(|g| g.caret);
    };
    let line = line.min(snapshot.len_lines() - 1);
    let l = view.cache.get(ui, snapshot, line, ext, t);
    let at = (snapshot.line_to_char(line) + l.column(ui, snapshot, t, x)?)
        .min(content_end(snapshot, line));
    if view.wrap.enabled {
        let endpoint = l.x(ui, snapshot, t, at - snapshot.line_to_char(line))?;
        view.wrap_previous_affinity = soft_wrap::source_position(view, line, endpoint, false)
            .is_some_and(|(_, mapped)| mapped != row);
    }
    // Hit-testing snaps to a genuine grapheme endpoint while preserving actual glyph widths.
    let previous = previous_grapheme(snapshot, at);
    let next = next_grapheme(snapshot, previous);
    if next == at || at == 0 {
        Some(at)
    } else {
        let start = snapshot.line_to_char(line);
        let a = l.x(ui, snapshot, t, previous.saturating_sub(start))?;
        let b = l.x(ui, snapshot, t, next.saturating_sub(start))?;
        if x < (a + b) * 0.5 {
            Some(previous)
        } else {
            Some(next.min(content_end(snapshot, line)))
        }
    }
}
pub(super) fn copy(s: &AppService, ui: &egui::Ui, view: &EditorView) -> Option<String> {
    let d = s.active_document()?;
    let snapshot = d.snapshot();
    let mut selections = view.multi.all(d.selection());
    selections.sort_by_key(|s| s.range().start);
    let linewise = selections.iter().all(|s| s.is_caret());
    let text = if linewise {
        let lines: std::collections::BTreeSet<_> = selections
            .iter()
            .map(|s| snapshot.char_to_line(s.head))
            .collect();
        let mut out = String::new();
        for line in lines {
            let text = snapshot.line(line).to_string();
            out.push_str(&text);
            if !text.ends_with('\n') {
                out.push_str(if view.crlf { "\r\n" } else { "\n" });
            }
        }
        out
    } else {
        selections
            .iter()
            .map(|s| snapshot.slice(s.range()).to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let key = egui::Id::unique("codisseu.editor.line.clipboard");
    ui.ctx().data_mut(|data| {
        if linewise {
            data.insert_temp(key, text.clone());
        } else {
            data.remove::<String>(key);
        }
    });
    ui.ctx().copy_text(text.clone());
    Some(text)
}
pub(super) fn cut(s: &mut AppService, ui: &egui::Ui, id: DocumentId, view: &mut EditorView) {
    let Some(d) = s.active_document().filter(|d| d.id == id) else {
        return;
    };
    let snapshot = d.snapshot();
    let before = d.selection();
    let all = view.multi.all(before);
    if copy(s, ui, view).is_some() {
        if all.iter().all(|s| s.is_caret()) {
            commands::planned(
                s,
                id,
                view,
                codisseu_editor::commands::EditorCommand::DeleteLines,
            );
        } else if view.multi.extra.is_empty() {
            edit(s, id, before.range(), String::new(), EditKind::Atomic);
        } else {
            view.multi.replace(s, id, &snapshot, before, "", None);
        }
        view.reveal = true;
    }
}

fn pasted(text: String, crlf: bool) -> String {
    if !crlf {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut previous = '\0';
    for c in text.chars() {
        if c == '\n' && previous != '\r' {
            out.push('\r');
        }
        out.push(c);
        previous = c;
    }
    out
}
pub(super) fn detect_indentation(
    snapshot: &TextSnapshot,
) -> codisseu_editor::commands::Indentation {
    codisseu_editor::commands::detect_indentation(snapshot)
}

fn indent(s: &mut AppService, id: DocumentId, view: &mut EditorView, back: bool) {
    use codisseu_editor::commands::{EditorCommand, Indentation};
    let d = s.active_document().unwrap();
    let snapshot = d.snapshot();
    let primary = d.selection();
    let all = view.multi.all(primary);
    let style = view.indentation.unwrap_or(Indentation::Spaces(4));
    if back || all.iter().any(|selection| !selection.is_caret()) {
        commands::planned(
            s,
            id,
            view,
            if back {
                EditorCommand::Outdent(style)
            } else {
                EditorCommand::Indent(style)
            },
        );
        return;
    }
    let texts: Vec<_> = all
        .iter()
        .map(|selection| match style {
            Indentation::Tabs { .. } => "\t".to_owned(),
            Indentation::Spaces(width) => {
                let line = snapshot.char_to_line(selection.head);
                let start = snapshot.line_to_char(line);
                let column = snapshot
                    .slice(start..selection.head)
                    .chars()
                    .fold(0, |column, c| {
                        if c == '\t' {
                            column + width - column % width
                        } else {
                            column + 1
                        }
                    });
                " ".repeat(width - column % width)
            }
        })
        .collect();
    view.multi.insert_texts(s, id, &snapshot, primary, &texts);
    view.reveal = true;
}
#[allow(clippy::too_many_arguments)]
pub(super) fn handle(
    ui: &egui::Ui,
    service: &mut AppService,
    id: DocumentId,
    view_id: ViewId,
    view: &mut EditorView,
    response: &mut egui::Response,
    rect: egui::Rect,
    origin: egui::Pos2,
    ext: &str,
    t: &Tokens,
) {
    let events = ui.input(|i| i.events.clone());
    let modal = ui.memory(|m| m.top_modal_layer().is_some());
    if modal || service.navigation.choices.len() > 1 {
        return;
    }
    if view
        .chord
        .is_some_and(|time| ui.input(|i| i.time) - time >= 5.)
    {
        view.chord = None;
        if service.status.starts_with("Ctrl+K:") {
            service.status = "Editor".into();
        }
    }
    let doc = service.view_document(view_id).unwrap();
    // Navigation dismisses a suggestion before choosing a destination. Virtual
    // inserted rows must not swallow the first Down/Home key or become a
    // canonical cursor destination when the suggestion disappears this frame.
    if response.has_focus()
        && view.ghost.is_some()
        && events.iter().any(|event| {
            matches!(
                event,
                egui::Event::Key {
                    key: Key::ArrowLeft
                        | Key::ArrowRight
                        | Key::ArrowUp
                        | Key::ArrowDown
                        | Key::Home
                        | Key::End
                        | Key::PageUp
                        | Key::PageDown,
                    pressed: true,
                    ..
                }
            )
        })
    {
        let line = view.ghost.take().unwrap().line;
        view.wrap.remove_ghost(line);
        view.wrap.rebuild(&view.fold);
        let snapshot = doc.snapshot();
        let selection = service.view(view_id).unwrap().selection();
        view.wrap.prepare(
            ui,
            &snapshot,
            &mut view.cache,
            &view.fold,
            None,
            (view.scroll.y / t.editor_line_height).floor().max(0.) as usize
                ..((view.scroll.y + rect.height()) / t.editor_line_height)
                    .ceil()
                    .max(0.) as usize
                    + 2,
            selection.head,
            &[selection.anchor],
            ext,
            t,
        );
    }
    view.multi.sync(
        doc.content_identity(),
        service.view(view_id).unwrap().selection(),
    );
    if view
        .composition
        .as_ref()
        .is_some_and(|c| c.version != service.view_document(view_id).unwrap().version)
    {
        view.composition = None;
        view.ime_cancelled = true;
    }
    for event in events {
        let doc = service.view_document(view_id).unwrap();
        if view.selection_expansion_stamp != Some(doc.content_identity()) {
            view.selection_expansions.clear();
            view.selection_expansion_stamp = Some(doc.content_identity());
        }
        let version = doc.version;
        let snapshot = doc.snapshot();
        if view.fold.revision() != doc.content_identity()
            || view.fold.line_count() != snapshot.len_lines()
        {
            view.fold.sync(doc.content_identity(), snapshot.len_lines());
            view.fold_service.cancel();
            view.fold_requested = None;
            view.fold_truncated = false;
        }
        let before = service.view(view_id).unwrap().selection();
        if view
            .composition
            .as_ref()
            .is_some_and(|c| c.version != version)
        {
            view.composition = None;
            view.ime_cancelled = true;
        }
        if let egui::Event::Text(text) = &event
            && let Some((key, _)) = view
                .owned_text
                .take()
                .filter(|(_, time)| ui.input(|i| i.time) - time < 0.25)
        {
            let typed = Key::from_name(text).or_else(|| Key::from_name(&text.to_ascii_uppercase()));
            if typed == Some(key)
                || matches!(
                    (key, text.as_str()),
                    (Key::OpenBracket, "{") | (Key::CloseBracket, "}")
                )
            {
                continue;
            }
        }
        if response.has_focus()
            && view
                .chord
                .is_some_and(|time| ui.input(|i| i.time) - time < 5.)
        {
            let action = match &event {
                egui::Event::Copy => Some(commands::Action::LineComment(Some(true))),
                egui::Event::Cut => Some(commands::Action::Edit(
                    codisseu_editor::commands::EditorCommand::TrimTrailingWhitespace,
                )),
                _ => None,
            };
            if let Some(action) = action {
                view.chord = None;
                if service.status.starts_with("Ctrl+K:") {
                    service.status = "Editor".into();
                }
                commands::run(ui, service, id, view, ext, t, action, rect);
                continue;
            }
            if matches!(event, egui::Event::Paste(_)) {
                view.chord = None;
                service.status = "Sequência de teclas cancelada".into();
                continue;
            }
        }
        match event {
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: true,
                ..
            } if rect.contains(pos)
                && response.contains_pointer()
                && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()) =>
            {
                if service.active_view() != Some(view_id) {
                    dispatch(service, Command::ActivateView { view: view_id });
                }
                view.context_position =
                    pointer(ui, &snapshot, view, pos, origin, ext, t).map(|at| (version, at));
                if let Some((_, at)) = view.context_position
                    && (before.is_caret() || !before.range().contains(&at))
                {
                    view.multi.clear();
                    select(service, id, Selection::caret(at));
                }
                view.drag_anchor = None;
            }
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            } if rect.contains(pos)
                && response.contains_pointer()
                && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()) =>
            {
                if service.active_view() != Some(view_id) {
                    dispatch(service, Command::ActivateView { view: view_id });
                }
                response.request_focus();
                if !modifiers.alt {
                    view.multi.clear();
                }
                if view.composition.take().is_some() {
                    view.ime_cancelled = true;
                }
                view.preferred_x = None;
                let Some(at) = pointer(ui, &snapshot, view, pos, origin, ext, t) else {
                    ui.ctx().request_repaint();
                    continue;
                };
                if modifiers.alt && !(modifiers.ctrl || modifiers.command) {
                    view.multi.toggle_caret(service, id, at);
                    view.drag_anchor = None;
                    view.reveal = true;
                    continue;
                }
                if (modifiers.ctrl || modifiers.command) && !modifiers.shift {
                    select(service, id, Selection::caret(at));
                    view.drag_anchor = None;
                    view.definition_click = Some((at, modifiers.alt));
                    view.last_interaction = ui.input(|i| i.time);
                    continue;
                }
                let selection = if response.triple_clicked() {
                    let line = snapshot.char_to_line(at);
                    Selection {
                        anchor: snapshot.line_to_char(line),
                        head: if line + 1 < snapshot.len_lines() {
                            snapshot.line_to_char(line + 1)
                        } else {
                            snapshot.len_chars()
                        },
                    }
                } else if response.double_clicked() {
                    let r = word_range(&snapshot, at);
                    Selection {
                        anchor: r.start,
                        head: r.end,
                    }
                } else {
                    Selection {
                        anchor: if modifiers.shift { before.anchor } else { at },
                        head: at,
                    }
                };
                view.drag_anchor = Some(selection.anchor);
                select(service, id, selection);
                view.last_interaction = ui.input(|i| i.time);
            }
            egui::Event::PointerMoved(pos)
                if view.drag_anchor.is_some() && ui.input(|i| i.pointer.primary_down()) =>
            {
                let Some(at) = pointer(ui, &snapshot, view, pos, origin, ext, t) else {
                    ui.ctx().request_repaint();
                    continue;
                };
                select(
                    service,
                    id,
                    Selection {
                        anchor: view.drag_anchor.unwrap(),
                        head: at,
                    },
                );
            }
            egui::Event::PointerButton {
                button: egui::PointerButton::Primary,
                pressed: false,
                pos,
                modifiers,
                ..
            } => {
                // egui recognizes completed clicks on release, which can be a
                // different native frame from the press. Testing only press
                // silently lost double-clicks delivered by the desktop backend.
                if rect.contains(pos)
                    && !modifiers.command
                    && response.contains_pointer()
                    && (response.double_clicked() || response.triple_clicked())
                    && let Some(at) = pointer(ui, &snapshot, view, pos, origin, ext, t)
                {
                    let r = if response.triple_clicked() {
                        let line = snapshot.char_to_line(at);
                        snapshot.line_to_char(line)..if line + 1 < snapshot.len_lines() {
                            snapshot.line_to_char(line + 1)
                        } else {
                            snapshot.len_chars()
                        }
                    } else {
                        word_range(&snapshot, at)
                    };
                    select(
                        service,
                        id,
                        Selection {
                            anchor: r.start,
                            head: r.end,
                        },
                    );
                }
                let settled = service.view(view_id).unwrap().selection();
                if view.drag_anchor.take().is_some() && !settled.is_caret() {
                    ui.ctx().send_cmd(egui::OutputCommand::TextSelectionSettled(
                        snapshot.slice(settled.range()).to_string(),
                    ));
                }
            }
            egui::Event::MiddleClickPaste { pos, text }
                if rect.contains(pos)
                    && response.contains_pointer()
                    && ui.ctx().layer_id_at(pos) == Some(ui.layer_id()) =>
            {
                if service.active_view() != Some(view_id) {
                    dispatch(service, Command::ActivateView { view: view_id });
                }
                response.request_focus();
                let Some(at) = pointer(ui, &snapshot, view, pos, origin, ext, t) else {
                    ui.ctx().request_repaint();
                    continue;
                };
                select(service, id, Selection::caret(at));
                edit(
                    service,
                    id,
                    at..at,
                    pasted(text, view.crlf),
                    EditKind::Atomic,
                );
                view.reveal = true;
            }
            egui::Event::AccessKitActionRequest(request)
                if request.target_node == editor_id(view_id).accesskit_id()
                    && request.target_tree == egui::accesskit::TreeId::ROOT =>
            {
                if service.active_view() != Some(view_id) {
                    dispatch(service, Command::ActivateView { view: view_id });
                }
                match (request.action, request.data) {
                    (
                        egui::accesskit::Action::SetValue,
                        Some(egui::accesskit::ActionData::Value(text)),
                    ) => edit(
                        service,
                        id,
                        0..snapshot.len_chars(),
                        text.to_string(),
                        EditKind::Atomic,
                    ),
                    (
                        egui::accesskit::Action::ReplaceSelectedText,
                        Some(egui::accesskit::ActionData::Value(text)),
                    ) => edit(
                        service,
                        id,
                        before.range(),
                        text.to_string(),
                        EditKind::Atomic,
                    ),
                    (
                        egui::accesskit::Action::SetTextSelection,
                        Some(egui::accesskit::ActionData::SetTextSelection(sel)),
                    ) => {
                        let a =
                            view.accessible_positions
                                .get(&sel.anchor.node)
                                .and_then(|(a, n)| {
                                    (sel.anchor.character_index <= *n)
                                        .then(|| a.checked_add(sel.anchor.character_index))
                                        .flatten()
                                });
                        let h =
                            view.accessible_positions
                                .get(&sel.focus.node)
                                .and_then(|(h, n)| {
                                    (sel.focus.character_index <= *n)
                                        .then(|| h.checked_add(sel.focus.character_index))
                                        .flatten()
                                });
                        if let (Some(anchor), Some(head)) = (a, h) {
                            select(service, id, Selection { anchor, head });
                            view.reveal = true;
                        }
                    }
                    (egui::accesskit::Action::Focus, _) => {
                        if service.active_view() != Some(view_id) {
                            dispatch(service, Command::ActivateView { view: view_id });
                        }
                        response.request_focus();
                    }
                    _ => {}
                }
            }
            _ if !response.has_focus()
                || service.active_view() != Some(view_id)
                || egui::Popup::is_any_open(ui.ctx()) => {}
            egui::Event::Copy => {
                copy(service, ui, view);
            }
            egui::Event::Cut => {
                cut(service, ui, id, view);
            }
            egui::Event::Paste(text) => {
                let copied_line = ui
                    .ctx()
                    .data(|data| {
                        data.get_temp::<String>(egui::Id::unique("codisseu.editor.line.clipboard"))
                    })
                    .is_some_and(|copied| {
                        copied.replace("\r\n", "\n") == text.replace("\r\n", "\n")
                    });
                let all = view.multi.all(before);
                if copied_line && all.iter().all(|s| s.is_caret()) {
                    let insert = pasted(text, view.crlf);
                    let blocks: Vec<_> = insert.split_inclusive('\n').collect();
                    let lines: std::collections::BTreeSet<_> =
                        all.iter().map(|s| snapshot.char_to_line(s.head)).collect();
                    let distribute = blocks.len() == lines.len();
                    let deltas = lines
                        .into_iter()
                        .enumerate()
                        .map(|(index, line)| {
                            let at = snapshot.line_to_char(line);
                            TextDelta {
                                range: at..at,
                                insert: if distribute {
                                    blocks[index].to_owned()
                                } else {
                                    insert.clone()
                                },
                            }
                        })
                        .collect();
                    commands::apply_deltas(service, id, view, deltas);
                    continue;
                }
                if view.composition.take().is_some() {
                    view.ime_cancelled = true;
                }
                if !view.multi.extra.is_empty() {
                    view.multi
                        .paste(service, id, &snapshot, before, &pasted(text, view.crlf));
                } else {
                    edit(
                        service,
                        id,
                        before.range(),
                        pasted(text, view.crlf),
                        EditKind::Atomic,
                    );
                }
                view.reveal = true;
            }
            egui::Event::Text(text) if view.composition.is_none() => {
                if !view.multi.extra.is_empty() {
                    view.multi
                        .replace(service, id, &snapshot, before, &text, None);
                } else if let Err(error) = delimiters::typed(service, id, view, ext, text) {
                    service.last_error = Some(error);
                }
                view.preferred_x = None;
                view.reveal = true;
            }
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text,
                active_range_chars,
            }) => {
                if text.is_empty() {
                    view.composition = None;
                } else if text.len() <= 64 * 1024 && !text.contains(['\0', '\r', '\n']) {
                    view.ime_cancelled = false;
                    view.composition_revision = view.composition_revision.wrapping_add(1);
                    let initial = view.composition.take();
                    view.composition = Some(Composition {
                        version,
                        selection: initial.as_ref().map_or(before, |c| c.selection),
                        text,
                        active: active_range_chars,
                    });
                }
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                let c = view.composition.take();
                if view.ime_cancelled {
                    view.ime_cancelled = false;
                    continue;
                }
                if !text.is_empty() {
                    let r = c.as_ref().map_or(before.range(), |c| c.selection.range());
                    if view.multi.extra.is_empty() {
                        edit(service, id, r, text, EditKind::Atomic);
                    } else {
                        view.multi
                            .replace(service, id, &snapshot, before, &text, None);
                    }
                    view.reveal = true;
                }
            }
            egui::Event::Ime(egui::ImeEvent::DeleteSurrounding {
                before_chars,
                after_chars,
            }) => {
                if let Some(a) = before.head.checked_sub(before_chars)
                    && let Some(b) = before.head.checked_add(after_chars)
                    && b <= snapshot.len_chars()
                {
                    view.composition = None;
                    edit(service, id, a..b, String::new(), EditKind::Atomic);
                    view.reveal = true;
                }
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // Modifier keys are real Key events in egui-winit. They must not
                // consume an armed chord while its next modifier is pressed.
                if matches!(
                    key,
                    Key::ShiftLeft
                        | Key::ShiftRight
                        | Key::ControlLeft
                        | Key::ControlRight
                        | Key::AltLeft
                        | Key::AltRight
                        | Key::SuperLeft
                        | Key::SuperRight
                ) {
                    continue;
                }
                view.last_interaction = ui.input(|i| i.time);
                if view.composition.is_some() {
                    if key == Key::Escape {
                        view.composition = None;
                        view.ime_cancelled = true;
                    }
                    continue;
                }
                if key != Key::Escape {
                    view.ime_cancelled = false;
                }
                let now = ui.input(|i| i.time);
                view.owned_text = None;
                let chord = view.chord.take().is_some_and(|time| now - time < 5.);
                if chord && service.status.starts_with("Ctrl+K:") {
                    service.status = "Editor".into();
                }
                if let Some(action) = keymap::binding(key, modifiers, chord, ext) {
                    view.owned_text = Some((key, now));
                    commands::run(ui, service, id, view, ext, t, action, rect);
                    continue;
                }
                if chord {
                    view.owned_text = Some((key, now));
                    service.status = "Sequência de teclas cancelada".into();
                    continue;
                }
                let control = modifiers.ctrl || modifiers.command || modifiers.mac_cmd;
                if commands::navigate(ui, service, id, view, ext, t, key, modifiers, rect) {
                    continue;
                }
                let mut destination = None;
                let caret = before.head.min(snapshot.len_chars());
                let line = snapshot.char_to_line(caret);
                let start = snapshot.line_to_char(line);
                let end = content_end(&snapshot, line);
                match key {
                    Key::D if control && !modifiers.shift && !modifiers.alt => {
                        view.multi.request(service, id);
                        view.reveal = true;
                    }
                    Key::Escape => {
                        if view.multi.extra.is_empty() && !before.is_caret() {
                            select(service, id, Selection::caret(before.head));
                        }
                        view.multi.clear();
                    }
                    Key::A if control && !modifiers.alt => {
                        view.multi.clear();
                        select(
                            service,
                            id,
                            Selection {
                                anchor: 0,
                                head: snapshot.len_chars(),
                            },
                        );
                        view.reveal = true;
                    }
                    Key::C if control && !modifiers.alt => {
                        copy(service, ui, view);
                    }
                    Key::X if control && !modifiers.alt => {
                        cut(service, ui, id, view);
                    }
                    Key::Z if control && !modifiers.alt => {
                        dispatch(
                            service,
                            if modifiers.shift {
                                Command::Redo { id }
                            } else {
                                Command::Undo { id }
                            },
                        );
                        view.reveal = true;
                    }
                    Key::Y if control && !modifiers.alt => {
                        dispatch(service, Command::Redo { id });
                        view.reveal = true;
                    }
                    Key::S if control && !modifiers.alt => {
                        dispatch(service, Command::Save { id });
                    }
                    Key::ArrowLeft | Key::ArrowRight => {
                        view.wrap_previous_affinity = false;
                        view.preferred_x = None;
                        destination = Some(if !modifiers.shift && !before.is_caret() {
                            if key == Key::ArrowLeft {
                                before.range().start
                            } else {
                                before.range().end
                            }
                        } else if key == Key::ArrowLeft {
                            if control {
                                previous_word(&snapshot, caret)
                            } else {
                                previous_grapheme(&snapshot, caret)
                            }
                        } else if control {
                            next_word(&snapshot, caret)
                        } else {
                            next_grapheme(&snapshot, caret)
                        });
                    }
                    Key::Home | Key::End => {
                        view.preferred_x = None;
                        let previous_affinity = view.wrap_previous_affinity;
                        view.wrap_previous_affinity = key == Key::End;
                        if control {
                            destination = Some(if key == Key::Home {
                                0
                            } else {
                                snapshot.len_chars()
                            });
                        } else if view.wrap.enabled {
                            let layout = view.cache.get(ui, &snapshot, line, ext, t);
                            let position =
                                layout.x(ui, &snapshot, t, caret - start).and_then(|x| {
                                    soft_wrap::source_position(view, line, x, previous_affinity)
                                });
                            if let Some((_, row)) = position {
                                let local = row - view.wrap.visual_row(&view.fold, line);
                                if let Some(segment) =
                                    view.wrap.segment(line, local, layout.width())
                                {
                                    let desired = if key == Key::Home {
                                        0.
                                    } else {
                                        segment.x.end - segment.x.start
                                    };
                                    let target = soft_wrap::pointer_position(view, row, desired)
                                        .and_then(|(_, x)| layout.column(ui, &snapshot, t, x))
                                        .map(|column| start + column)
                                        .unwrap_or(caret)
                                        .min(end);
                                    destination = Some(if target == caret {
                                        if key == Key::Home { start } else { end }
                                    } else {
                                        target
                                    });
                                }
                            }
                        } else {
                            destination = Some(if key == Key::Home { start } else { end });
                        }
                    }
                    Key::ArrowUp | Key::ArrowDown | Key::PageUp | Key::PageDown => {
                        let current = view.cache.get(ui, &snapshot, line, ext, t);
                        let Some(current_x) = current.x(ui, &snapshot, t, caret - start) else {
                            view.reveal = true;
                            continue;
                        };
                        let Some((current_x, current_row)) = soft_wrap::source_position(
                            view,
                            line,
                            current_x,
                            view.wrap_previous_affinity,
                        ) else {
                            view.reveal = true;
                            continue;
                        };
                        let x = *view.preferred_x.get_or_insert(current_x);
                        let step = if matches!(key, Key::PageUp | Key::PageDown) {
                            (rect.height() / t.editor_line_height).ceil().max(1.) as usize
                        } else {
                            1
                        };
                        if matches!(key, Key::PageUp | Key::PageDown) {
                            let amount = step.saturating_sub(1) as f32 * t.editor_line_height;
                            ui.scroll_with_delta(egui::vec2(
                                0.,
                                if key == Key::PageUp { amount } else { -amount },
                            ));
                        }
                        let row = if matches!(key, Key::ArrowUp | Key::PageUp) {
                            current_row.saturating_sub(step)
                        } else {
                            current_row.saturating_add(step)
                        };
                        let total = if view.wrap.enabled {
                            view.wrap.total(&view.fold)
                        } else {
                            view.fold.visible_line_count()
                                + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
                        };
                        let row = row.min(total.saturating_sub(1));
                        let Some((target, target_x)) = soft_wrap::pointer_position(view, row, x)
                        else {
                            view.reveal = true;
                            continue;
                        };
                        let next = view.cache.get(ui, &snapshot, target, ext, t);
                        let point = snapshot.line_to_char(target)
                            + match next.column(ui, &snapshot, t, target_x) {
                                Some(column) => column,
                                None => {
                                    view.reveal = true;
                                    continue;
                                }
                            };
                        if let Some(endpoint) =
                            next.x(ui, &snapshot, t, point - snapshot.line_to_char(target))
                        {
                            view.wrap_previous_affinity =
                                soft_wrap::source_position(view, target, endpoint, false)
                                    .is_some_and(|(_, mapped)| mapped != row);
                        }
                        let point = point.min(content_end(&snapshot, target));
                        let a = previous_grapheme(&snapshot, point);
                        let b = next_grapheme(&snapshot, a);
                        destination = Some(if point == b || point == 0 {
                            point
                        } else {
                            a.max(snapshot.line_to_char(target))
                        });
                    }
                    Key::Backspace | Key::Delete => {
                        if !view.multi.extra.is_empty() {
                            view.multi.replace(
                                service,
                                id,
                                &snapshot,
                                before,
                                "",
                                Some((key == Key::Backspace, control)),
                            );
                            view.reveal = true;
                            continue;
                        }
                        let d = service.active_document().unwrap();
                        view.pairs.sync(
                            &snapshot,
                            d.last_change().map_or(0, |c| c.revision),
                            d.last_change(),
                        );
                        if key == Key::Backspace
                            && !control
                            && let Some(r) = view.pairs.empty_range(before)
                        {
                            edit(service, id, r, String::new(), EditKind::Atomic);
                            view.reveal = true;
                            continue;
                        }
                        let r = if !before.is_caret() {
                            before.range()
                        } else if key == Key::Backspace {
                            (if control {
                                previous_word(&snapshot, caret)
                            } else {
                                previous_grapheme(&snapshot, caret)
                            })..caret
                        } else {
                            caret..if control {
                                next_word(&snapshot, caret)
                            } else {
                                next_grapheme(&snapshot, caret)
                            }
                        };
                        edit(
                            service,
                            id,
                            r,
                            String::new(),
                            if key == Key::Backspace {
                                EditKind::Backspace
                            } else {
                                EditKind::Delete
                            },
                        );
                        view.preferred_x = None;
                        view.reveal = true;
                    }
                    Key::Enter => {
                        if modifiers.is_none() && phpdoc::enter(service, id, view, ext) {
                            continue;
                        }
                        if !view.multi.extra.is_empty() {
                            view.multi.replace(
                                service,
                                id,
                                &snapshot,
                                before,
                                if view.crlf { "\r\n" } else { "\n" },
                                None,
                            );
                            view.reveal = true;
                            continue;
                        }
                        let prefix = snapshot.slice(start..caret).to_string();
                        let indentation = if matches!(ext, "py" | "pyi" | "pyw") {
                            let line = snapshot.char_to_line(caret);
                            python::indentation(
                                &prefix,
                                view.cache.lexical_state(line, ext),
                                view.indentation
                                    .unwrap_or(codisseu_editor::commands::Indentation::Spaces(4)),
                            )
                        } else {
                            prefix
                                .chars()
                                .take_while(|c| matches!(c, ' ' | '\t'))
                                .collect()
                        };
                        edit(
                            service,
                            id,
                            before.range(),
                            format!("{}{indentation}", if view.crlf { "\r\n" } else { "\n" }),
                            EditKind::Atomic,
                        );
                        view.reveal = true;
                    }
                    Key::Insert if modifiers.shift && !control => {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                    }
                    Key::Insert if control && !modifiers.shift => {
                        copy(service, ui, view);
                    }
                    Key::Tab if !control && !modifiers.alt => {
                        indent(service, id, view, modifiers.shift);
                        view.reveal = true;
                        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                    }
                    _ => {}
                }
                if let Some(head) = destination {
                    view.multi.clear();
                    select(
                        service,
                        id,
                        Selection {
                            anchor: if modifiers.shift { before.anchor } else { head },
                            head,
                        },
                    );
                    view.reveal = true;
                    ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
                }
            }
            _ => {}
        }
    }
    if response.has_focus() && service.active_view() == Some(view_id) {
        let before = service.active_document().unwrap().selection();
        view.multi.poll(service, id, ui.ctx());
        if service.active_document().unwrap().selection() != before {
            view.reveal = true;
        }
    }
    if let Some(anchor) = view.drag_anchor
        && ui.input(|i| i.pointer.primary_down())
        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
    {
        let dx = if pos.x < rect.left() {
            pos.x - rect.left()
        } else if pos.x > rect.right() {
            pos.x - rect.right()
        } else {
            0.
        };
        let dy = if pos.y < rect.top() {
            pos.y - rect.top()
        } else if pos.y > rect.bottom() {
            pos.y - rect.bottom()
        } else {
            0.
        };
        if dx != 0. || dy != 0. {
            let step = egui::vec2(dx.clamp(-200., 200.), dy.clamp(-200., 200.))
                * ui.input(|i| i.stable_dt.min(0.05))
                * 10.;
            ui.scroll_with_delta(-step);
            ui.ctx().request_repaint();
            let snapshot = service.view_document(view_id).unwrap().snapshot();
            if let Some(head) = pointer(ui, &snapshot, view, pos, origin, ext, t) {
                select(service, id, Selection { anchor, head });
            }
        }
    }
    let doc = service.view_document(view_id).unwrap();
    if doc.version != view.stamp.map_or(doc.version, |s| s.0) {
        response.mark_changed();
    }
}
