//! Presentation of provider parameter labels, never discovery of call semantics.
use super::{Tokens, color, signature_paint::Run};
use codisseu_core::SignatureInformation;

pub(super) fn runs(
    signature: &SignatureInformation,
    active: Option<usize>,
    php: bool,
    tokens: &Tokens,
) -> Vec<Run> {
    if signature.parameters.is_empty() {
        return vec![Run {
            text: if signature
                .label
                .split_once('(')
                .is_some_and(|(_, tail)| tail.trim_start().starts_with(')'))
            {
                "<no parameters>".into()
            } else {
                signature.label.clone()
            },
            color: color(tokens.signature_foreground),
            bold: false,
            clip: !php,
            padding: [0., 0.],
        }];
    }
    let mut runs = Vec::new();
    for (index, parameter) in signature.parameters.iter().enumerate() {
        let label = signature
            .label
            .get(parameter.label_range.clone())
            .unwrap_or("");
        let bold = active == Some(index);
        let last = index + 1 == signature.parameters.len();
        if php {
            let display = php_parameter(label, parameter.documentation.as_deref());
            runs.push(Run {
                text: display,
                color: color(tokens.signature_foreground),
                bold,
                clip: !php,
                padding: [0., 0.],
            });
            if !last {
                runs.push(Run {
                    text: ", ".into(),
                    color: color(tokens.signature_foreground),
                    bold: false,
                    clip: !php,
                    padding: [0., 0.],
                });
            }
        } else {
            let name_color = color(if bold {
                tokens.signature_active
            } else {
                tokens.signature_inactive_name
            });
            let type_color = color(if bold {
                tokens.signature_type
            } else {
                tokens.signature_inactive_type
            });
            if let Some((name, ty)) = label.split_once(':') {
                runs.push(Run {
                    text: name.to_owned(),
                    color: name_color,
                    bold,
                    clip: !php,
                    padding: [3., 0.],
                });
                runs.push(Run {
                    text: ":".into(),
                    color: name_color,
                    bold,
                    clip: true,
                    padding: [0., 0.],
                });
                runs.push(Run {
                    text: ty.to_owned(),
                    color: type_color,
                    bold,
                    clip: !php,
                    padding: [0., if last { 3. } else { 0. }],
                });
            } else {
                runs.push(Run {
                    text: label.to_owned(),
                    color: name_color,
                    bold,
                    clip: !php,
                    padding: [3., if last { 3. } else { 0. }],
                });
            }
            if !last {
                runs.push(Run {
                    text: ",".into(),
                    color: name_color,
                    bold,
                    clip: !php,
                    padding: [0., 3.],
                });
                runs.push(Run {
                    text: "\u{2009}".into(),
                    color: name_color,
                    bold: false,
                    clip: !php,
                    padding: [0., 0.],
                });
            }
        }
    }
    runs
}

fn php_parameter(label: &str, documentation: Option<&str>) -> String {
    // Phpactor returns the variable as label and its declared type/default in
    // parameter documentation. Reorder only that exact single-line declaration.
    // Unknown Markdown/prose stays the original label; it is not parsed as code.
    if let Some(name) = label.strip_prefix('$')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && let Some(doc) = documentation.filter(|d| !d.contains(['\n', '\r', '`']))
        && let Some((ty, suffix)) = doc.split_once(label)
        && !ty.trim().is_empty()
        && !ty.trim().contains(char::is_whitespace)
        && (suffix.trim().is_empty() || suffix.trim().starts_with('='))
    {
        return format!("{name}: {}{suffix}", ty.trim());
    }
    label.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn php_presentation_uses_only_explicit_provider_declaration() {
        assert_eq!(php_parameter("$name", Some("string $name")), "name: string");
        assert_eq!(
            php_parameter("$count", Some("int $count = 1")),
            "count: int = 1"
        );
        for doc in [
            None,
            Some("Documentation for $name"),
            Some("```string $name```"),
            Some("string $name\ntext"),
        ] {
            assert_eq!(php_parameter("$name", doc), "$name");
        }
        assert_eq!(
            php_parameter("name: string", Some("unrelated")),
            "name: string"
        );
    }
}
