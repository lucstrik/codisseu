//! Bounded syntactic declarations for navigation; no semantic edits or providers.
use std::{
    ops::ControlFlow,
    path::Path,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Class,
    Interface,
    Trait,
    Enum,
    Type,
    Function,
    Method,
    Property,
    Constant,
    Variable,
}
impl Kind {
    pub fn is_class(self) -> bool {
        matches!(
            self,
            Self::Class | Self::Interface | Self::Trait | Self::Enum | Self::Type
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub container: String,
    pub kind: Kind,
    pub line: u32,
    pub column: u32,
}
#[derive(Clone, Debug, Default)]
pub struct Declarations {
    pub symbols: Vec<Symbol>,
    pub truncated: bool,
}

/// PHP and JS/TS/JSX/TSX use the existing public tree-sitter grammars. Strings,
/// comments and references do not become declarations. Positions use scalars.
pub fn declarations(path: &Path, text: &str, cancel: &dyn Fn() -> bool) -> Declarations {
    let mut result = Declarations::default();
    if text.len() > 1024 * 1024 || cancel() {
        result.truncated = true;
        return result;
    }
    let language = match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "py" | "pyi" | "pyw" => tree_sitter_python::LANGUAGE.into(),
        "php" | "phtml" => tree_sitter_php::LANGUAGE_PHP.into(),
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "js" | "jsx" | "mjs" | "cjs" | "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        _ => return result,
    };
    let deadline = Instant::now() + Duration::from_millis(200);
    let stopped = || cancel() || Instant::now() >= deadline;
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        result.truncated = true;
        return result;
    }
    let mut progress = |_: &tree_sitter::ParseState| {
        if stopped() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let Some(tree) = parser.parse_with_options(
        &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    ) else {
        result.truncated = true;
        return result;
    };
    let mut remaining = 200_000usize;
    visit(
        tree.root_node(),
        text,
        "",
        0,
        &stopped,
        &mut remaining,
        &mut result,
    );
    result
}
fn visit(
    node: Node<'_>,
    text: &str,
    container: &str,
    depth: usize,
    stopped: &dyn Fn() -> bool,
    remaining: &mut usize,
    out: &mut Declarations,
) {
    if *remaining == 0 || depth > 128 || stopped() || out.symbols.len() >= 4096 {
        out.truncated = true;
        return;
    }
    *remaining -= 1;
    let kind = match node.kind() {
        "class_declaration" | "abstract_class_declaration" | "class_definition" => {
            Some(Kind::Class)
        }
        "interface_declaration" => Some(Kind::Interface),
        "trait_declaration" => Some(Kind::Trait),
        "enum_declaration" => Some(Kind::Enum),
        "type_alias_declaration" => Some(Kind::Type),
        "function_definition" if crate::python_rename::is_method(node) => Some(Kind::Method),
        "function_declaration" | "function_definition" | "generator_function_declaration" => {
            Some(Kind::Function)
        }
        "method_declaration"
        | "method_definition"
        | "method_signature"
        | "abstract_method_signature" => Some(Kind::Method),
        "property_element" | "public_field_definition" | "property_signature" => {
            Some(Kind::Property)
        }
        "const_element" | "enum_case" => Some(Kind::Constant),
        "variable_declarator" => Some(Kind::Variable),
        _ => None,
    };
    let name_node = kind.and_then(|_| {
        node.child_by_field_name("name").or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor).find(|n| {
                matches!(
                    n.kind(),
                    "name" | "variable_name" | "property_identifier" | "identifier"
                )
            })
        })
    });
    let mut owner = None;
    if let (Some(kind), Some(name_node)) = (kind, name_node)
        && let Some(name) = text.get(name_node.byte_range())
        && !name.is_empty()
        && name.len() <= 512
        && !name.contains(['\n', '\r'])
    {
        let position = name_node.start_position();
        let start = name_node.start_byte().saturating_sub(position.column);
        let column = text
            .get(start..name_node.start_byte())
            .map_or(position.column, |s| s.chars().count());
        out.symbols.push(Symbol {
            name: name.into(),
            container: container.into(),
            kind,
            line: position.row as u32,
            column: column as u32,
        });
        if kind.is_class() || matches!(kind, Kind::Function | Kind::Method) {
            owner = Some(if container.is_empty() {
                name.into()
            } else {
                format!("{container}.{name}")
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if stopped() {
            out.truncated = true;
            break;
        }
        visit(
            child,
            text,
            owner.as_deref().unwrap_or(container),
            depth + 1,
            stopped,
            remaining,
            out,
        );
        if *remaining == 0 || out.symbols.len() >= 4096 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_php_and_typescript_declarations_skip_comment_string_and_references() {
        for (path, text, class, method) in [
            (
                "Greeter.ts",
                "// class Fake {}\nconst quoted = 'function fake() {}';\nexport class Greeter { greet(name: string) { return name; } }\nfunction greetingTitle() {}\nnew Greeter().greet('x');",
                "Greeter",
                "greet",
            ),
            (
                "Greeting.php",
                "<?php // class Fake {}\nclass Greeting { public function text() { return 'function fake()'; } }\nfunction heading() {}",
                "Greeting",
                "text",
            ),
        ] {
            let found = declarations(Path::new(path), text, &|| false);
            assert!(!found.truncated);
            assert!(
                found
                    .symbols
                    .iter()
                    .any(|s| s.kind.is_class() && s.name == class)
            );
            assert!(
                found
                    .symbols
                    .iter()
                    .any(|s| s.kind == Kind::Method && s.name == method && s.container == class)
            );
            assert!(
                !found
                    .symbols
                    .iter()
                    .any(|s| matches!(s.name.as_str(), "Fake" | "fake"))
            );
            assert_eq!(found.symbols.iter().filter(|s| s.name == method).count(), 1);
        }
    }
    #[test]
    fn unicode_position_and_cancellation_are_explicit() {
        let source = "const 雨 = 1; class A { café() {} }";
        let found = declarations(Path::new("a.ts"), source, &|| false);
        let method = found.symbols.iter().find(|s| s.name == "café").unwrap();
        assert_eq!(
            method.column as usize,
            source[..source.find("café").unwrap()].chars().count()
        );
        assert!(declarations(Path::new("a.ts"), source, &|| true).truncated);
    }
}
