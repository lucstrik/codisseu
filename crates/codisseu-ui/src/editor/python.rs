//! Python indentation uses the same bounded lexical state as the visible editor.
use super::*;
use codisseu_editor::commands::Indentation;
pub(super) fn indentation(
    prefix: &str,
    state: Option<lexical::State>,
    style: Indentation,
) -> String {
    let base = prefix
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect::<String>();
    if prefix.len() > 16 * 1024 {
        return base;
    }
    let Some(lexical::State::Extended(state)) = state else {
        return base;
    };
    let mut code = vec![true; prefix.len()];
    if highlight::extended::scan(
        prefix.chars(),
        highlight::extended::Language::Python,
        state,
        |range, kind| {
            if matches!(
                kind,
                highlight::extended::Kind::String
                    | highlight::extended::Kind::Comment
                    | highlight::extended::Kind::DocComment
            ) {
                for value in &mut code[range] {
                    *value = false;
                }
            }
        },
        || false,
    )
    .is_none()
    {
        return base;
    }
    let logical = prefix
        .char_indices()
        .map(|(i, c)| if code[i] { c } else { ' ' })
        .collect::<String>();
    let trimmed = logical.trim();
    let unit = match style {
        Indentation::Spaces(width) => " ".repeat(width.clamp(1, 8)),
        Indentation::Tabs { .. } => "\t".into(),
    };
    if trimmed.ends_with(':') {
        return format!("{base}{unit}");
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let balance = trimmed.chars().fold(0i32, |n, c| match c {
        '(' | '[' | '{' => n + 1,
        ')' | ']' | '}' => n - 1,
        _ => n,
    });
    if balance == 0 && matches!(first, "return" | "raise" | "break" | "continue" | "pass") {
        if base.ends_with('\t') {
            return base[..base.len() - 1].into();
        }
        return base[..base.len().saturating_sub(unit.len())].into();
    }
    base
}
#[cfg(test)]
mod tests {
    use super::*;
    fn indent(source: &str) -> String {
        indentation(
            source,
            Some(lexical::State::Extended(Default::default())),
            Indentation::Spaces(4),
        )
    }
    #[test]
    fn python_blocks_comments_strings_unicode_and_dedent_follow_source() {
        assert_eq!(
            indent("    if message == \"ação😀\": # comment:"),
            "        "
        );
        assert_eq!(indent("    value = \"not a block:\""), "    ");
        assert_eq!(indent("    # if fake:"), "    ");
        assert_eq!(indent("    return value"), "");
        assert_eq!(indent("        raise ValueError(\"bad:\")"), "    ");
        assert_eq!(indent("    return ("), "    ");
        assert_eq!(indent("    message = f\"text: {value}\""), "    ");
        assert_eq!(
            indentation(
                "\tif value:",
                Some(lexical::State::Extended(Default::default())),
                Indentation::Tabs { tab_width: 4 }
            ),
            "\t\t"
        );
    }
    #[test]
    fn python_docstring_continuation_does_not_create_code_indentation() {
        let state = highlight::extended::scan(
            "\"\"\"doc\n".chars(),
            highlight::extended::Language::Python,
            Default::default(),
            |_, _| {},
            || false,
        )
        .unwrap();
        assert_eq!(
            indentation(
                "    example:",
                Some(lexical::State::Extended(state)),
                Indentation::Spaces(4)
            ),
            "    "
        );
        assert_eq!(
            indentation("    if value:", None, Indentation::Spaces(4)),
            "    "
        );
    }
}
