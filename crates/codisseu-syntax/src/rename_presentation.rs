//! Bounded syntactic labels for preview only. No binding or Apply authority.
use crate::{
    TextRange, ValidationLimits,
    ast::Language,
    incremental::{self, Budget},
};
use std::{path::Path, sync::atomic::AtomicBool};
use tree_sitter::Node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenameOccurrenceKind {
    Declaration,
    SemanticReference,
    Comment,
    String,
    PlainText,
    Unknown,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenameEnclosingKind {
    Class,
    Function,
    Method,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameEnclosingSymbol {
    pub kind: RenameEnclosingKind,
    pub name: String,
    pub range: TextRange,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenameOccurrencePresentation {
    pub kind: RenameOccurrenceKind,
    /// Outermost to innermost, using the original source and UTF-16 ranges.
    pub enclosing: Vec<RenameEnclosingSymbol>,
    pub unknown_reason: Option<String>,
}
impl RenameOccurrencePresentation {
    fn unknown(reason: &str) -> Self {
        Self {
            kind: RenameOccurrenceKind::Unknown,
            enclosing: vec![],
            unknown_reason: Some(reason.into()),
        }
    }
}
fn contains(node: Node<'_>, start: usize, end: usize) -> bool {
    node.start_byte() <= start && end <= node.end_byte()
}
fn declaration(node: Node<'_>, start: usize, end: usize) -> bool {
    let field = match node.kind() {
        "class_declaration"
        | "class_definition"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration"
        | "function_definition"
        | "function_declaration"
        | "function_signature"
        | "method_definition"
        | "method_declaration"
        | "method_signature"
        | "property_element"
        | "public_field_definition"
        | "property_signature"
        | "property_promotion_parameter"
        | "simple_parameter"
        | "variadic_parameter"
        | "variable_declarator"
        | "type_alias_declaration" => "name",
        "required_parameter" | "optional_parameter" => {
            if node.child_by_field_name("name").is_some() {
                "name"
            } else {
                "pattern"
            }
        }
        _ => return false,
    };
    node.child_by_field_name(field).is_some_and(|name| {
        // Destructuring keys are not declarations. Only the identifier binding
        // itself, or PHP's `$name` wrapper, is labeled here.
        matches!(
            name.kind(),
            "identifier"
                | "name"
                | "variable_name"
                | "property_identifier"
                | "private_property_identifier"
                | "type_identifier"
                | "shorthand_property_identifier_pattern"
        ) && contains(name, start, end)
    })
}
fn source_range(node: Node<'_>, source: &str) -> TextRange {
    let point = |byte: usize, row: usize| {
        let prefix = &source[..byte];
        let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count();
        (row as u32, column as u32)
    };
    let (a, b) = point(node.start_byte(), node.start_position().row);
    let (c, d) = point(node.end_byte(), node.end_position().row);
    TextRange {
        start_line: a,
        start_character: b,
        end_line: c,
        end_character: d,
    }
}
fn enclosing(node: Node<'_>, source: &str) -> Option<RenameEnclosingSymbol> {
    let kind = match node.kind() {
        "class_declaration"
        | "class_definition"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration"
        | "anonymous_class" => RenameEnclosingKind::Class,
        "function_definition"
        | "function_declaration"
        | "function_signature"
        | "arrow_function"
        | "anonymous_function"
        | "function_expression" => RenameEnclosingKind::Function,
        "method_definition" | "method_declaration" | "method_signature" => {
            RenameEnclosingKind::Method
        }
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")
        .map(|n| {
            let raw = &source[n.byte_range()];
            let mut name = raw.chars().take(128).collect::<String>();
            if name.len() < raw.len() {
                name.push('…');
            }
            name
        })
        .unwrap_or_else(|| "(anonymous)".into());
    Some(RenameEnclosingSymbol {
        kind,
        name,
        range: source_range(node, source),
    })
}
fn one(
    root: Node<'_>,
    source: &str,
    range: &TextRange,
    budget: &mut Budget<'_>,
) -> RenameOccurrencePresentation {
    let Some(start) = incremental::byte_offset(source, range.start_line, range.start_character)
    else {
        return RenameOccurrencePresentation::unknown("Invalid UTF-16 range");
    };
    let Some(end) = incremental::byte_offset(source, range.end_line, range.end_character) else {
        return RenameOccurrencePresentation::unknown("Invalid UTF-16 range");
    };
    if start >= end {
        return RenameOccurrencePresentation::unknown("Empty or reversed range");
    }
    let Some(mut node) = root.named_descendant_for_byte_range(start, end) else {
        return RenameOccurrencePresentation::unknown("No syntax node");
    };
    let mut kind = RenameOccurrenceKind::SemanticReference;
    let mut contexts = vec![];
    let mut is_module = false;
    let mut interpolation = false;
    loop {
        if budget.node().is_err() {
            return RenameOccurrencePresentation::unknown("Presentation cancelled or limited");
        }
        if node.is_error() || node.is_missing() {
            return RenameOccurrencePresentation::unknown("Invalid syntax");
        }
        if declaration(node, start, end) {
            kind = RenameOccurrenceKind::Declaration;
        }
        interpolation |= matches!(
            node.kind(),
            "variable_name" | "template_substitution" | "interpolation"
        );
        match node.kind() {
            "comment" => kind = RenameOccurrenceKind::Comment,
            "string" | "encapsed_string" | "string_literal" | "template_string" | "heredoc"
            | "nowdoc"
                if !interpolation =>
            {
                kind = RenameOccurrenceKind::String
            }
            "import_statement"
            | "export_statement"
            | "require_expression"
            | "require_once_expression"
            | "include_expression"
            | "include_once_expression" => is_module = true,
            "call_expression" => {
                if let Some(function) = node.child_by_field_name("function") {
                    is_module |= matches!(&source[function.byte_range()], "require" | "import");
                }
            }
            _ => {}
        }
        if let Some(context) = enclosing(node, source) {
            if contexts.len() == 32 {
                return RenameOccurrencePresentation::unknown(
                    "Presentation nesting exceeds 32 contexts",
                );
            }
            contexts.push(context);
        }
        let Some(parent) = node.parent() else {
            break;
        };
        node = parent;
    }
    if is_module && kind == RenameOccurrenceKind::String {
        kind = RenameOccurrenceKind::SemanticReference;
    }
    contexts.reverse();
    RenameOccurrencePresentation {
        kind,
        enclosing: contexts,
        unknown_reason: None,
    }
}

/// Exactly one output per requested occurrence; never filters or authorizes edits.
/// Parses each source once. Syntax/limits/cancellation produce visible Unknown.
pub fn rename_occurrence_presentation(
    path: &Path,
    source: &str,
    ranges: &[TextRange],
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Vec<RenameOccurrencePresentation> {
    let unknown = |reason| {
        ranges
            .iter()
            .map(|_| RenameOccurrencePresentation::unknown(reason))
            .collect()
    };
    if source.len() > 1024 * 1024 || ranges.len() > 1024 {
        return unknown("Presentation exceeds 1 MiB/1024 occurrences");
    }
    let mut budget = Budget::new(cancel, limits);
    if budget.check().is_err() {
        return unknown("Presentation cancelled or limited");
    }
    let extension = path.extension().and_then(|s| s.to_str());
    if matches!(extension, Some("txt" | "md")) {
        return ranges
            .iter()
            .map(|r| {
                if budget.node().is_err() {
                    return RenameOccurrencePresentation::unknown(
                        "Presentation cancelled or limited",
                    );
                }
                let start = incremental::byte_offset(source, r.start_line, r.start_character);
                let end = incremental::byte_offset(source, r.end_line, r.end_character);
                if !matches!((start,end),(Some(a),Some(b)) if a < b) {
                    return RenameOccurrencePresentation::unknown("Invalid UTF-16 range");
                }
                RenameOccurrencePresentation {
                    kind: RenameOccurrenceKind::PlainText,
                    enclosing: vec![],
                    unknown_reason: None,
                }
            })
            .collect();
    }
    let grammar = match extension {
        Some("py" | "pyi" | "pyw") => tree_sitter_python::LANGUAGE.into(),
        Some("php") => Language::Php.grammar(),
        Some("ts" | "js") => Language::TypeScript.grammar(),
        Some("tsx" | "jsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => return unknown("Presentation language unsupported"),
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&grammar).is_err() {
        return unknown("Presentation parser unavailable");
    }
    let Some(tree) = incremental::parse(&mut parser, source, None, &budget) else {
        return unknown("Presentation parse cancelled or limited");
    };
    if tree.root_node().has_error() {
        return unknown("Invalid syntax");
    }
    ranges
        .iter()
        .map(|range| one(tree.root_node(), source, range, &mut budget))
        .collect()
}
