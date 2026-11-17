//! One execution path for editor key bindings and visible editor actions.
use super::*;
use codisseu_core::{AppService, TextDelta};
use codisseu_editor::commands::{self as engine, Direction, EditorCommand as Edit};

#[derive(Clone)]
pub(super) enum Action {
    Edit(Edit),
    LineComment(Option<bool>),
    BlockComment,
    StartChord,
    MarkdownSplit,
    Format(bool),
    CodeActions,
    AddCursor(Direction),
    CursorsAtLineEnds,
    AllOccurrences,
    SkipOccurrence,
    CursorUndo,
    ExpandSelection,
    ShrinkSelection,
    MatchingBracket,
    Scroll(i32, bool),
    SmartHome(bool),
    Fold(FoldAction),
}

#[derive(Clone, Copy)]
pub(super) enum FoldAction {
    Fold,
    Unfold,
    Toggle,
    All,
    UnfoldAll,
    Level(usize),
    Comments,
    Regions(bool),
    Recursive(bool),
    Except(bool),
}

pub(super) fn planned(s: &mut AppService, id: DocumentId, view: &mut EditorView, command: Edit) {
    let Some(d) = s.active_document().filter(|d| d.id == id) else {
        s.last_error = Some("O documento ativo mudou; selecione o editor novamente".into());
        return;
    };
    let before = d.selection();
    let snapshot = d.snapshot();
    let all = view.multi.all(before);
    let command = match command {
        Edit::Indent(_) => Edit::Indent(view.indentation.unwrap_or(engine::Indentation::Spaces(4))),
        Edit::Outdent(_) => {
            Edit::Outdent(view.indentation.unwrap_or(engine::Indentation::Spaces(4)))
        }
        command => command,
    };
    match engine::plan(&snapshot, &all, command) {
        Ok(plan) => {
            let Some(&after) = plan.selections.last() else {
                return;
            };
            let version = d.version;
            let identity = d.content_identity();
            view.multi.record_cursor(identity, before);
            let ok = if plan.deltas.is_empty() {
                input::dispatch(
                    s,
                    Command::SetSelection {
                        id,
                        expected_version: version,
                        selection: after,
                    },
                )
            } else {
                view.multi.remember(identity, before);
                input::dispatch(
                    s,
                    Command::EditBatchTransaction {
                        id,
                        expected_version: version,
                        deltas: plan.deltas,
                        before,
                        after,
                    },
                )
            };
            if ok {
                view.multi.install(s, id, plan.selections, false);
                view.reveal = true;
                view.preferred_x = None;
            }
        }
        Err(error) => s.last_error = Some(error),
    }
}
fn comment(ext: &str) -> Option<&'static str> {
    match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "php" | "phtml" | "rs"
        | "c" | "h" | "cpp" | "java" | "go" | "swift" => Some("//"),
        "sql" | "lua" => Some("--"),
        "py" | "pyi" | "pyw" | "sh" | "bash" | "zsh" | "yaml" | "yml" | "toml" | "rb" | "r" => {
            Some("#")
        }
        _ => None,
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    ui: &egui::Ui,
    s: &mut AppService,
    id: DocumentId,
    view: &mut EditorView,
    ext: &str,
    t: &Tokens,
    action: Action,
    viewport: egui::Rect,
) {
    let Some(d) = s.active_document().filter(|d| d.id == id) else {
        s.last_error = Some("O documento ativo mudou; selecione o editor novamente".into());
        return;
    };
    let snap = d.snapshot();
    let primary = d.selection();
    match action {
        Action::Edit(command) => planned(s, id, view, command),
        Action::Fold(action) => {
            use FoldAction::*;
            let line = snap.char_to_line(primary.head);
            let current = view.fold.range_at(line).cloned();
            match action {
                Fold => {
                    view.fold.fold_at(line);
                }
                Unfold => {
                    view.fold.unfold_at(line);
                }
                Toggle => {
                    view.fold
                        .toggle_at(current.as_ref().map_or(line, |r| r.start));
                }
                All => view.fold.fold_all(),
                UnfoldAll => view.fold.unfold_all(),
                Level(level) => view.fold.fold_level(level),
                Comments | Regions(_) => {
                    let ranges = view.fold.ranges().to_vec();
                    for range in ranges {
                        let applicable = match action {
                            Comments => range.kind == folding::FoldKind::Comment,
                            Regions(_) => range.kind == folding::FoldKind::Region,
                            _ => false,
                        };
                        if applicable {
                            if matches!(action, Regions(false)) {
                                view.fold.unfold_at(range.start);
                            } else {
                                view.fold.fold_at(range.start);
                            }
                        }
                    }
                }
                Recursive(unfold) => {
                    if let Some(current) = &current {
                        let ranges = view.fold.ranges().to_vec();
                        for range in ranges {
                            if range.start >= current.start && range.end <= current.end {
                                if unfold {
                                    view.fold.unfold_at(range.start);
                                } else {
                                    view.fold.fold_at(range.start);
                                }
                            }
                        }
                    }
                }
                Except(unfold) => {
                    let ranges = view.fold.ranges().to_vec();
                    for range in ranges {
                        if !(range.start <= line && line <= range.end) {
                            if unfold {
                                view.fold.unfold_at(range.start);
                            } else {
                                view.fold.fold_at(range.start);
                            }
                        }
                    }
                }
            }
            if view.fold.is_hidden(line) {
                let header = view.fold.source_line(view.fold.visual_row(line));
                input::select(s, id, Selection::caret(snap.line_to_char(header)));
            }
            view.multi.clear();
            view.reveal = false;
            ui.ctx().request_repaint();
        }

        Action::LineComment(force) => {
            let Some(prefix) = comment(ext) else {
                if matches!(ext, "md" | "markdown" | "html" | "htm" | "xml" | "svg") {
                    planned(
                        s,
                        id,
                        view,
                        Edit::ToggleBlockComment {
                            open: "<!--".into(),
                            close: "-->".into(),
                        },
                    );
                } else {
                    s.status = "Esta linguagem não define comentário de linha".into();
                }
                return;
            };
            // Explicit Add/Remove Comment is idempotent; it must not toggle opposite intent.
            if let Some(add) = force {
                let mut lines = std::collections::BTreeSet::new();
                for selection in view.multi.all(primary) {
                    let r = selection.range();
                    let first = snap.char_to_line(r.start);
                    let mut last = snap.char_to_line(r.end);
                    if r.end > r.start && r.end == snap.line_to_char(last) {
                        last = last.saturating_sub(1);
                    }
                    lines.extend(first..=last);
                }
                let mut deltas = Vec::new();
                for line in lines {
                    let text = snap.line(line).to_string();
                    let white = text.chars().take_while(|c| matches!(c, ' ' | '\t')).count();
                    let rest: String = text.chars().skip(white).collect();
                    let commented = rest.starts_with(prefix);
                    let at = snap.line_to_char(line) + white;
                    if add && !commented {
                        deltas.push(TextDelta {
                            range: at..at,
                            insert: format!("{prefix} "),
                        });
                    }
                    if !add && commented {
                        let count = prefix.chars().count()
                            + usize::from(rest[prefix.len()..].starts_with(' '));
                        deltas.push(TextDelta {
                            range: at..at + count,
                            insert: String::new(),
                        });
                    }
                }
                apply_deltas(s, id, view, deltas);
            } else {
                planned(
                    s,
                    id,
                    view,
                    Edit::ToggleLineComment(engine::CommentSyntax {
                        prefix: prefix.into(),
                    }),
                );
            }
        }
        Action::BlockComment => {
            let (open, close) = if matches!(ext, "md" | "markdown" | "html" | "htm" | "xml" | "svg")
            {
                ("<!--", "-->")
            } else if matches!(
                ext,
                "js" | "jsx"
                    | "ts"
                    | "tsx"
                    | "mjs"
                    | "cjs"
                    | "mts"
                    | "cts"
                    | "php"
                    | "phtml"
                    | "rs"
                    | "c"
                    | "cpp"
                    | "h"
                    | "java"
                    | "go"
                    | "swift"
                    | "css"
                    | "scss"
                    | "less"
                    | "sql"
                    | "jsonc"
            ) {
                ("/*", "*/")
            } else {
                s.status = "Esta linguagem não define comentário de bloco".into();
                return;
            };
            planned(
                s,
                id,
                view,
                Edit::ToggleBlockComment {
                    open: open.into(),
                    close: close.into(),
                },
            );
        }
        Action::StartChord => {
            view.chord = Some(ui.input(|i| i.time));
            s.status = "Ctrl+K: aguardando a segunda tecla…".into();
            ui.ctx().request_repaint_after(Duration::from_secs(5));
        }
        Action::MarkdownSplit => {
            if matches!(ext, "md" | "markdown" | "mkd" | "mdown") {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(egui::Id::unique("codisseu.markdown.split.request"), true)
                });
            }
        }
        Action::CodeActions => {
            let doc = s.active_document().unwrap();
            input::dispatch(
                s,
                Command::CodeActions {
                    id,
                    expected_version: doc.version,
                    character_index: primary.head,
                },
            );
        }
        Action::Format(selection) => {
            let version = s.active_document().unwrap().version;
            input::dispatch(
                s,
                Command::FormatDocument {
                    id,
                    expected_version: version,
                    range: (selection && (ext != "sql" || !primary.is_caret()))
                        .then(|| primary.range()),
                },
            );
        }
        Action::AllOccurrences => view.multi.request_all(s, id),
        Action::SkipOccurrence => view.multi.request_skip(s, id),
        Action::CursorUndo => view.multi.cursor_undo(s, id),
        Action::CursorsAtLineEnds => {
            let mut selections = Vec::new();
            for selection in view.multi.all(primary) {
                let r = selection.range();
                let first = snap.char_to_line(r.start);
                let mut last = snap.char_to_line(r.end);
                if !r.is_empty() && r.end == snap.line_to_char(last) {
                    last = last.saturating_sub(1);
                }
                for line in first..=last {
                    selections.push(Selection::caret(content_end(&snap, line)));
                    if selections.len() >= 512 {
                        break;
                    }
                }
            }
            view.multi.install(s, id, selections, true);
            view.reveal = true;
        }
        Action::AddCursor(direction) => {
            let original = view.multi.all(primary);
            let mut all = original.clone();
            for selection in original {
                let line = snap.char_to_line(selection.head);
                let next = match direction {
                    Direction::Up => line.checked_sub(1),
                    Direction::Down => (line + 1 < snap.len_lines()).then_some(line + 1),
                };
                let Some(next) = next else { continue };
                let start = snap.line_to_char(line);
                let layout = view.cache.get(ui, &snap, line, ext, t);
                let Some(x) = layout.x(ui, &snap, t, selection.head - start) else {
                    continue;
                };
                let target = view.cache.get(ui, &snap, next, ext, t);
                let Some(column) = target.column(ui, &snap, t, x) else {
                    continue;
                };
                let at = (snap.line_to_char(next) + column).min(content_end(&snap, next));
                all.push(Selection::caret(snap_boundary(&snap, at)));
                if all.len() >= 512 {
                    break;
                }
            }
            view.multi.install(s, id, all, true);
            view.reveal = true;
        }
        Action::SmartHome(shift) => {
            let mut all = view.multi.all(primary);
            for selection in &mut all {
                let line = snap.char_to_line(selection.head);
                let start = snap.line_to_char(line);
                let first = start
                    + snap
                        .line(line)
                        .chars()
                        .take_while(|c| matches!(c, ' ' | '\t'))
                        .count();
                let wrapped = if view.wrap.enabled {
                    wrap_boundary(ui, &snap, view, selection.head, false, ext, t)
                } else {
                    None
                };
                let head = if let Some(head) =
                    wrapped.filter(|head| *head != selection.head && *head != start)
                {
                    head
                } else if selection.head == first {
                    start
                } else {
                    first.min(content_end(&snap, line))
                };
                *selection = Selection {
                    anchor: if shift { selection.anchor } else { head },
                    head,
                };
            }
            view.multi.install(s, id, all, true);
            view.reveal = true;
            view.preferred_x = None;
            view.wrap_previous_affinity = false;
        }
        Action::MatchingBracket => {
            if let Some(pair) = delimiters::matching(view, &snap, primary.head, ext) {
                let head = if primary.head == pair[0] || primary.head.saturating_sub(1) == pair[0] {
                    pair[1]
                } else {
                    pair[0]
                };
                view.multi
                    .install(s, id, vec![Selection::caret(head)], true);
                view.reveal = true;
            }
        }
        Action::ExpandSelection => {
            let current = primary.range();
            let word = codisseu_core::word_range(&snap, primary.head);
            let line = snap.char_to_line(current.start);
            let last = snap.char_to_line(current.end);
            let line_range = snap.line_to_char(line)..content_end(&snap, last);
            let next = if word.start <= current.start && word.end >= current.end && word != current
            {
                word
            } else if line_range.start <= current.start
                && line_range.end >= current.end
                && line_range != current
            {
                line_range
            } else {
                0..snap.len_chars()
            };
            view.selection_expansions.push(primary);
            if view.selection_expansions.len() > 64 {
                view.selection_expansions.remove(0);
            }
            let forward = primary.anchor <= primary.head;
            view.multi.install(
                s,
                id,
                vec![Selection {
                    anchor: if forward { next.start } else { next.end },
                    head: if forward { next.end } else { next.start },
                }],
                true,
            );
            view.reveal = true;
        }
        Action::ShrinkSelection => {
            if let Some(previous) = view.selection_expansions.pop()
                && previous.range().end <= snap.len_chars()
            {
                view.multi.install(s, id, vec![previous], true);
                view.reveal = true;
            }
        }
        Action::Scroll(direction, page) => {
            let amount = if page {
                viewport.height().max(t.editor_line_height)
            } else {
                t.editor_line_height
            };
            ui.scroll_with_delta(egui::vec2(0., -(direction as f32) * amount));
            view.reveal = false;
        }
    }
}
fn snap_boundary(s: &TextSnapshot, at: usize) -> usize {
    if at == 0 || codisseu_core::next_grapheme(s, codisseu_core::previous_grapheme(s, at)) == at {
        at
    } else {
        codisseu_core::previous_grapheme(s, at)
    }
}
pub(super) fn apply_deltas(
    s: &mut AppService,
    id: DocumentId,
    view: &mut EditorView,
    deltas: Vec<TextDelta>,
) {
    if deltas.is_empty() {
        return;
    }
    let d = s.active_document().unwrap();
    let before = d.selection();
    let version = d.version;
    let mut selections = view.multi.all(before);
    for selection in &mut selections {
        for delta in deltas.iter().rev() {
            *selection = selection.mapped(delta);
        }
    }
    let after = *selections.last().unwrap();
    view.multi.remember(d.content_identity(), before);
    if input::dispatch(
        s,
        Command::EditBatchTransaction {
            id,
            expected_version: version,
            deltas,
            before,
            after,
        },
    ) {
        view.multi.install(s, id, selections, false);
        view.reveal = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn navigate(
    ui: &egui::Ui,
    s: &mut AppService,
    id: DocumentId,
    view: &mut EditorView,
    ext: &str,
    t: &Tokens,
    key: Key,
    m: Modifiers,
    viewport: egui::Rect,
) -> bool {
    if view.multi.extra.is_empty()
        || !matches!(
            key,
            Key::ArrowLeft
                | Key::ArrowRight
                | Key::ArrowUp
                | Key::ArrowDown
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
        )
    {
        return false;
    }
    let d = s.active_document().unwrap();
    let snapshot = d.snapshot();
    let primary = d.selection();
    let mut all = view.multi.all(primary);
    let control = m.ctrl || m.command || m.mac_cmd;
    for selection in &mut all {
        let at = selection.head;
        let line = snapshot.char_to_line(at);
        let start = snapshot.line_to_char(line);
        let head = match key {
            Key::ArrowLeft if !m.shift && !selection.is_caret() => selection.range().start,
            Key::ArrowRight if !m.shift && !selection.is_caret() => selection.range().end,
            Key::ArrowLeft => {
                if control {
                    codisseu_core::previous_word(&snapshot, at)
                } else {
                    codisseu_core::previous_grapheme(&snapshot, at)
                }
            }
            Key::ArrowRight => {
                if control {
                    codisseu_core::next_word(&snapshot, at)
                } else {
                    codisseu_core::next_grapheme(&snapshot, at)
                }
            }
            Key::Home => {
                if control {
                    0
                } else if view.wrap.enabled {
                    wrap_boundary(ui, &snapshot, view, at, false, ext, t).unwrap_or(start)
                } else {
                    start
                }
            }
            Key::End => {
                if control {
                    snapshot.len_chars()
                } else if view.wrap.enabled {
                    wrap_boundary(ui, &snapshot, view, at, true, ext, t)
                        .unwrap_or(content_end(&snapshot, line))
                } else {
                    content_end(&snapshot, line)
                }
            }
            _ => {
                let step = if matches!(key, Key::PageUp | Key::PageDown) {
                    (viewport.height() / t.editor_line_height).ceil().max(1.) as usize
                } else {
                    1
                };
                let l = view.cache.get(ui, &snapshot, line, ext, t);
                let Some(x) = l.x(ui, &snapshot, t, at - start) else {
                    continue;
                };
                let Some((x, row)) = soft_wrap::source_position(view, line, x, false) else {
                    continue;
                };
                let target_row = if matches!(key, Key::ArrowUp | Key::PageUp) {
                    row.saturating_sub(step)
                } else {
                    row + step
                };
                let total = if view.wrap.enabled {
                    view.wrap.total(&view.fold)
                } else {
                    view.fold.visible_line_count()
                        + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
                };
                let Some((target, x)) =
                    soft_wrap::pointer_position(view, target_row.min(total.saturating_sub(1)), x)
                else {
                    continue;
                };
                let next = view.cache.get(ui, &snapshot, target, ext, t);
                let Some(column) = next.column(ui, &snapshot, t, x) else {
                    continue;
                };
                snap_boundary(
                    &snapshot,
                    (snapshot.line_to_char(target) + column).min(content_end(&snapshot, target)),
                )
            }
        };
        *selection = Selection {
            anchor: if m.shift { selection.anchor } else { head },
            head,
        };
    }
    view.multi.install(s, id, all, true);
    view.reveal = true;
    view.preferred_x = None;
    view.wrap_previous_affinity = key == Key::End;
    true
}

fn wrap_boundary(
    ui: &egui::Ui,
    snapshot: &TextSnapshot,
    view: &mut EditorView,
    at: usize,
    end: bool,
    ext: &str,
    t: &Tokens,
) -> Option<usize> {
    let line = snapshot.char_to_line(at);
    let start = snapshot.line_to_char(line);
    let layout = view.cache.get(ui, snapshot, line, ext, t);
    let x = layout.x(ui, snapshot, t, at - start)?;
    let (_, row) = soft_wrap::source_position(view, line, x, view.wrap_previous_affinity)?;
    let segment = view.wrap.segment(
        line,
        row - view.wrap.visual_row(&view.fold, line),
        layout.width(),
    )?;
    let (_, x) = soft_wrap::pointer_position(
        view,
        row,
        if end {
            segment.x.end - segment.x.start
        } else {
            0.
        },
    )?;
    Some((start + layout.column(ui, snapshot, t, x)?).min(content_end(snapshot, line)))
}

pub(super) fn menu_items(ext: &str) -> Vec<(&'static str, Action)> {
    use engine::{CaseTransform, Indentation};
    let mut items = vec![
        (
            "Mover linha acima    Alt+↑",
            Action::Edit(Edit::MoveLines(Direction::Up)),
        ),
        (
            "Mover linha abaixo    Alt+↓",
            Action::Edit(Edit::MoveLines(Direction::Down)),
        ),
        (
            "Duplicar acima    Ctrl+Shift+Alt+↑",
            Action::Edit(Edit::CopyLines(Direction::Up)),
        ),
        (
            "Duplicar abaixo    Ctrl+Shift+Alt+↓",
            Action::Edit(Edit::CopyLines(Direction::Down)),
        ),
        (
            "Excluir linhas    Ctrl+Shift+K",
            Action::Edit(Edit::DeleteLines),
        ),
        (
            "Inserir linha acima    Ctrl+Shift+Enter",
            Action::Edit(Edit::InsertLineAbove),
        ),
        (
            "Inserir linha abaixo    Ctrl+Enter",
            Action::Edit(Edit::InsertLineBelow),
        ),
        ("Unir linhas", Action::Edit(Edit::JoinLines)),
        ("Formatar documento    Ctrl+Shift+I", Action::Format(false)),
        ("Formatar seleção    Ctrl+K Ctrl+F", Action::Format(true)),
        (
            "Indentar linhas    Ctrl+]",
            Action::Edit(Edit::Indent(Indentation::Spaces(4))),
        ),
        (
            "Desindentar linhas    Ctrl+[",
            Action::Edit(Edit::Outdent(Indentation::Spaces(4))),
        ),
        ("Comentar linhas    Ctrl+/", Action::LineComment(None)),
        ("Comentar bloco    Ctrl+Shift+A", Action::BlockComment),
        (
            "Selecionar linha    Ctrl+L",
            Action::Edit(Edit::SelectLines),
        ),
        (
            "Selecionar ocorrências    Ctrl+Shift+L",
            Action::AllOccurrences,
        ),
        (
            "Cursores nos finais de linha    Shift+Alt+I",
            Action::CursorsAtLineEnds,
        ),
        ("Expandir seleção    Shift+Alt+→", Action::ExpandSelection),
        ("Reduzir seleção    Shift+Alt+←", Action::ShrinkSelection),
        (
            "Remover espaços finais    Ctrl+K Ctrl+X",
            Action::Edit(Edit::TrimTrailingWhitespace),
        ),
        (
            "Transformar em MAIÚSCULAS",
            Action::Edit(Edit::TransformCase(CaseTransform::Upper)),
        ),
        (
            "Transformar em minúsculas",
            Action::Edit(Edit::TransformCase(CaseTransform::Lower)),
        ),
    ];
    let label = match ext {
        "php" | "phtml" => Some("Formatar arquivo PHP ativo    Ctrl+Alt+L"),
        "sql" => Some("Formatar arquivo SQL ativo    Ctrl+Alt+L"),
        "js" | "jsx" => Some("Formatar arquivo JavaScript ativo    Ctrl+Alt+L"),
        "ts" | "tsx" => Some("Formatar arquivo TypeScript ativo    Ctrl+Alt+L"),
        "rs" => Some("Formatar arquivo Rust ativo    Ctrl+Alt+L"),
        "py" | "pyi" | "pyw" => Some("Formatar arquivo Python ativo    Ctrl+Alt+L"),
        _ => None,
    };
    if matches!(
        ext,
        "rs" | "php" | "js" | "jsx" | "ts" | "tsx" | "py" | "pyi" | "pyw"
    ) {
        items.insert(0, ("Correções rápidas    Alt+Enter", Action::CodeActions));
    }
    if let Some(label) = label {
        items.insert(0, (label, Action::Format(false)));
    }
    items
}
