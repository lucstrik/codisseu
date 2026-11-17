//! UI description from syntax; this is not a semantic rename approval.
use crate::{
    ValidationLimits,
    ast::Language,
    incremental::{self, Budget},
};
use std::{path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenameSymbolKind {
    Class,
    Function,
    Method,
    Property,
    Parameter,
    Variable,
    Imported,
    #[default]
    Unknown,
}
pub fn classify_rename_symbol(
    path: &Path,
    text: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
) -> Result<RenameSymbolKind, String> {
    let language = match path.extension().and_then(|s| s.to_str()) {
        Some("php") => Language::Php,
        Some("ts" | "tsx" | "js" | "jsx") => Language::TypeScript,
        _ => return Ok(RenameSymbolKind::Unknown),
    };
    if text.len() > 1024 * 1024 {
        return Err("Descrição de símbolo excede 1 MiB".into());
    }
    let byte = incremental::byte_offset(text, line, character).ok_or("Posição UTF-16 inválida")?;
    let mut budget = Budget::new(cancel, ValidationLimits::default());
    let mut parser = tree_sitter::Parser::new();
    let grammar = if path.extension().is_some_and(|e| e == "tsx" || e == "jsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        language.grammar()
    };
    parser.set_language(&grammar).map_err(|e| e.to_string())?;
    let tree = incremental::parse(&mut parser, text, None, &budget)
        .ok_or("Descrição de símbolo cancelada ou excedida")?;
    let Some(mut node) = tree.root_node().descendant_for_byte_range(byte, byte) else {
        return Ok(RenameSymbolKind::Unknown);
    };
    loop {
        budget
            .node()
            .map_err(|_| "Descrição de símbolo cancelada ou excedida")?;
        let contains = |field| {
            node.child_by_field_name(field)
                .is_some_and(|n| n.start_byte() <= byte && byte <= n.end_byte())
        };
        let kind = match node.kind() {
            "class_declaration" | "interface_declaration" | "trait_declaration"
                if contains("name") =>
            {
                Some(RenameSymbolKind::Class)
            }
            "function_definition" | "function_declaration" | "function_signature"
                if contains("name") =>
            {
                Some(RenameSymbolKind::Function)
            }
            "method_definition" | "method_declaration" | "method_signature" if contains("name") => {
                Some(RenameSymbolKind::Method)
            }
            "property_element" | "public_field_definition" | "property_signature"
                if contains("name") =>
            {
                Some(RenameSymbolKind::Property)
            }
            "property_promotion_parameter" if contains("name") => Some(RenameSymbolKind::Property),
            "simple_parameter" | "required_parameter" | "optional_parameter"
                if contains("name") || contains("pattern") =>
            {
                let mut c = node.walk();
                let field = node
                    .children(&mut c)
                    .any(|n| matches!(n.kind(), "accessibility_modifier" | "readonly"));
                Some(if field {
                    RenameSymbolKind::Property
                } else {
                    RenameSymbolKind::Parameter
                })
            }
            "member_call_expression" | "scoped_call_expression" if contains("name") => {
                Some(RenameSymbolKind::Method)
            }
            "member_access_expression" | "scoped_property_access_expression"
                if contains("name") =>
            {
                Some(RenameSymbolKind::Property)
            }
            "member_expression" if contains("property") => Some(
                if node.parent().is_some_and(|p| {
                    p.kind() == "call_expression" && p.child_by_field_name("function") == Some(node)
                }) {
                    RenameSymbolKind::Method
                } else {
                    RenameSymbolKind::Property
                },
            ),
            "namespace_use_clause" | "import_specifier" => Some(RenameSymbolKind::Imported),
            "function_call_expression" | "call_expression" if contains("function") => {
                Some(RenameSymbolKind::Function)
            }
            "object_creation_expression" | "new_expression" | "named_type" => {
                Some(RenameSymbolKind::Class)
            }
            "variable_declarator" if contains("name") => Some(RenameSymbolKind::Variable),
            _ => None,
        };
        if let Some(kind) = kind {
            return Ok(kind);
        }
        let Some(parent) = node.parent() else { break };
        node = parent;
    }
    Ok(RenameSymbolKind::Unknown)
}
