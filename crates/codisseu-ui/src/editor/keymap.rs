//! Editor-only Linux default bindings; application shortcuts keep their own contexts.
use super::commands::{Action, FoldAction};
use super::*;
use codisseu_editor::commands::{Direction, EditorCommand as Edit, Indentation};

pub(super) fn binding(key: Key, m: Modifiers, chord: bool, ext: &str) -> Option<Action> {
    let sql = ext == "sql";
    let ctrl = m.ctrl || m.command || m.mac_cmd;
    let indent = Indentation::Spaces(4);
    if chord {
        return match (key, ctrl, m.shift, m.alt) {
            (Key::Num0, true, false, false) => Some(Action::Fold(FoldAction::All)),
            (Key::J, true, false, false) => Some(Action::Fold(FoldAction::UnfoldAll)),
            (Key::L, true, false, false) => Some(Action::Fold(FoldAction::Toggle)),
            (Key::L, true, true, false) => Some(Action::Fold(FoldAction::Recursive(false))),
            (Key::OpenBracket, true, false, false) => {
                Some(Action::Fold(FoldAction::Recursive(false)))
            }
            (Key::CloseBracket, true, false, false) => {
                Some(Action::Fold(FoldAction::Recursive(true)))
            }
            (Key::Slash, true, false, false) => Some(Action::Fold(FoldAction::Comments)),
            (Key::Num8, true, false, false) => Some(Action::Fold(FoldAction::Regions(true))),
            (Key::Num9, true, false, false) => Some(Action::Fold(FoldAction::Regions(false))),
            (Key::Minus, true, false, false) => Some(Action::Fold(FoldAction::Except(false))),
            (Key::Equals | Key::Plus, true, false, false) => {
                Some(Action::Fold(FoldAction::Except(true)))
            }
            (
                Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7,
                true,
                false,
                false,
            ) => Some(Action::Fold(FoldAction::Level(match key {
                Key::Num1 => 1,
                Key::Num2 => 2,
                Key::Num3 => 3,
                Key::Num4 => 4,
                Key::Num5 => 5,
                Key::Num6 => 6,
                _ => 7,
            }))),

            (Key::F, true, false, false) => Some(Action::Format(true)),
            (Key::C, true, false, false) => Some(Action::LineComment(Some(true))),
            (Key::U, true, false, false) => Some(Action::LineComment(Some(false))),
            (Key::D, true, false, false) => Some(Action::SkipOccurrence),
            (Key::X, true, false, false) => Some(Action::Edit(Edit::TrimTrailingWhitespace)),
            (Key::V, false, false, false) => Some(Action::MarkdownSplit),
            _ => None,
        };
    }
    let direction = if key == Key::ArrowUp {
        Direction::Up
    } else {
        Direction::Down
    };
    match (key, ctrl, m.shift, m.alt) {
        (Key::Enter, false, false, true)
            if matches!(
                ext,
                "rs" | "php" | "js" | "jsx" | "ts" | "tsx" | "py" | "pyi" | "pyw"
            ) =>
        {
            Some(Action::CodeActions)
        }
        (Key::L, true, false, true)
            if matches!(
                ext,
                "sql" | "php" | "phtml" | "js" | "jsx" | "ts" | "tsx" | "rs" | "py" | "pyi" | "pyw"
            ) =>
        {
            Some(Action::Format(false))
        }
        (Key::I, true, true, false) | (Key::F, false, true, true) => Some(Action::Format(false)),
        (Key::OpenBracket | Key::OpenCurlyBracket, true, true, false) => {
            Some(Action::Fold(FoldAction::Fold))
        }
        (Key::CloseBracket | Key::CloseCurlyBracket, true, true, false) => {
            Some(Action::Fold(FoldAction::Unfold))
        }

        (Key::K, true, false, false) => Some(Action::StartChord),
        (Key::ArrowUp | Key::ArrowDown, false, false, true) => {
            Some(Action::Edit(Edit::MoveLines(direction)))
        }
        (Key::ArrowUp | Key::ArrowDown, true, true, true) => {
            Some(Action::Edit(Edit::CopyLines(direction)))
        }
        (Key::ArrowUp | Key::ArrowDown, false, true, true) => Some(Action::AddCursor(direction)),
        (Key::ArrowUp | Key::ArrowDown, true, true, false)
        | (Key::ArrowUp | Key::ArrowDown, true, false, true) => Some(Action::AddCursor(direction)),
        (Key::K, true, true, false) => Some(Action::Edit(Edit::DeleteLines)),
        (Key::Enter, true, false, false) if !sql => Some(Action::Edit(Edit::InsertLineBelow)),
        (Key::Enter, true, true, false) => Some(Action::Edit(Edit::InsertLineAbove)),
        (Key::CloseBracket, true, false, false) => Some(Action::Edit(Edit::Indent(indent))),
        (Key::OpenBracket, true, false, false) => Some(Action::Edit(Edit::Outdent(indent))),
        (Key::L, true, false, false) => Some(Action::Edit(Edit::SelectLines)),
        (Key::Slash, true, false, false) => Some(Action::LineComment(None)),
        (Key::A, true, true, false) | (Key::A, false, true, true) => Some(Action::BlockComment),
        (Key::L, true, true, false) | (Key::F2, true, false, false) => Some(Action::AllOccurrences),
        (Key::U, true, false, false) => Some(Action::CursorUndo),
        (Key::I, false, true, true) => Some(Action::CursorsAtLineEnds),
        (Key::ArrowRight, false, true, true) => Some(Action::ExpandSelection),
        (Key::ArrowLeft, false, true, true) => Some(Action::ShrinkSelection),
        (Key::Backslash, true, true, false) => Some(Action::MatchingBracket),
        (Key::ArrowUp | Key::ArrowDown, true, false, false) => Some(Action::Scroll(
            if key == Key::ArrowUp { -1 } else { 1 },
            false,
        )),
        (Key::PageUp | Key::PageDown, false, false, true) => Some(Action::Scroll(
            if key == Key::PageUp { -1 } else { 1 },
            true,
        )),
        (Key::Home, false, false, false) | (Key::Home, false, true, false) => {
            Some(Action::SmartHome(m.shift))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_whole_file_shortcut_supports_php_sql_javascript_and_typescript() {
        let modifiers = Modifiers::CTRL.plus(Modifiers::ALT);
        for ext in ["sql", "php", "phtml", "js", "jsx", "ts", "tsx", "rs"] {
            assert!(matches!(
                binding(Key::L, modifiers, false, ext),
                Some(Action::Format(false))
            ));
        }
        assert!(binding(Key::L, modifiers, false, "txt").is_none());
        assert!(binding(Key::Enter, Modifiers::CTRL, false, "sql").is_none());
    }
}
