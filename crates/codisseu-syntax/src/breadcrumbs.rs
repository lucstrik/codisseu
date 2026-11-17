//! Syntactic containment only. Coordinates are scalar source indices, never LSP columns.
use std::{
    collections::BTreeSet,
    ops::{ControlFlow, Range},
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser};
pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_SCOPES: usize = 2048;
const MAX_NODES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Php,
    TypeScript,
    Tsx,
}
impl Language {
    pub fn for_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" => Self::Php,
            "ts" | "js" | "mts" | "cts" | "mjs" | "cjs" => Self::TypeScript,
            "tsx" | "jsx" => Self::Tsx,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Namespace,
    Class,
    Interface,
    Trait,
    Enum,
    Function,
    Method,
    Closure,
    Variable,
    Parameter,
    Call,
    Argument,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Namespace => "Namespace",
            Self::Class => "Classe",
            Self::Interface => "Interface",
            Self::Trait => "Trait",
            Self::Enum => "Enum",
            Self::Function => "Função",
            Self::Method => "Método",
            Self::Closure => "Função anônima",
            Self::Variable => "Variável",
            Self::Parameter => "Parâmetro",
            Self::Call => "Chamada",
            Self::Argument => "Argumento",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub name: String,
    pub kind: Kind,
    pub range: Range<usize>,
    pub target: Range<usize>,
    pub line: usize,
}
#[derive(Clone, Debug, Default)]
pub struct Inspection {
    pub scopes: Vec<Scope>,
    pub truncated: bool,
}
impl Inspection {
    pub fn at(&self, caret: usize) -> Vec<&Scope> {
        if self.truncated {
            return vec![];
        }
        self.scopes
            .iter()
            .filter(|s| {
                s.range.contains(&caret)
                    || (matches!(s.kind, Kind::Argument | Kind::Parameter) && caret == s.range.end)
            })
            .collect()
    }
}
struct Raw {
    name: String,
    kind: Kind,
    range: Range<usize>,
    target: Range<usize>,
}
fn name(node: Node<'_>, source: &str) -> Option<(String, Range<usize>)> {
    let node = node
        .child_by_field_name("name")
        .or_else(|| node.child_by_field_name("pattern"))?;
    if node.is_missing() || node.has_error() {
        return None;
    }
    let text = source.get(node.byte_range())?;
    if text.is_empty() || text.len() > 256 {
        return None;
    }
    Some((
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        node.byte_range(),
    ))
}
fn classify(node: Node<'_>, source: &str) -> Option<Raw> {
    let mut range = node.byte_range();
    let (kind, fallback) = match node.kind() {
        "namespace_definition" | "internal_module" | "module" => {
            if node.kind() == "namespace_definition" && node.child_by_field_name("body").is_none() {
                let mut next = node.next_named_sibling();
                range.end = source.len();
                while let Some(sibling) = next {
                    if sibling.kind() == "namespace_definition" {
                        range.end = sibling.start_byte();
                        break;
                    }
                    next = sibling.next_named_sibling();
                }
            }
            (Kind::Namespace, "namespace")
        }
        "class_declaration" | "abstract_class_declaration" | "class" | "anonymous_class" => {
            (Kind::Class, "classe anônima")
        }
        "interface_declaration" => (Kind::Interface, "interface"),
        "trait_declaration" => (Kind::Trait, "trait"),
        "enum_declaration" => (Kind::Enum, "enum"),
        "function_definition" | "function_declaration" | "generator_function_declaration" => {
            (Kind::Function, "função")
        }
        "method_declaration"
        | "method_definition"
        | "method_signature"
        | "abstract_method_signature" => (Kind::Method, "método"),
        "arrow_function"
        | "anonymous_function"
        | "anonymous_function_creation_expression"
        | "function_expression"
        | "generator_function" => (Kind::Closure, "função anônima"),
        "simple_parameter"
        | "variadic_parameter"
        | "property_promotion_parameter"
        | "required_parameter"
        | "optional_parameter" => (Kind::Parameter, "parâmetro"),
        "variable_declarator"
            if node.child_by_field_name("value").is_some_and(|v| {
                matches!(v.kind(), "arrow_function" | "function_expression" | "class")
            }) =>
        {
            (Kind::Variable, "variável")
        }
        "call_expression"
        | "member_call_expression"
        | "nullsafe_member_call_expression"
        | "scoped_call_expression"
        | "function_call_expression"
        | "new_expression"
        | "object_creation_expression" => {
            let args = node.child_by_field_name("arguments").or_else(|| {
                let mut c = node.walk();
                node.named_children(&mut c)
                    .find(|n| n.kind() == "arguments")
            })?;
            let callee = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| node.child_by_field_name("constructor"))?;
            let target = callee
                .child_by_field_name("property")
                .or_else(|| callee.child_by_field_name("name"))
                .unwrap_or(callee);
            let text = source.get(target.byte_range())?;
            if text.is_empty() || text.len() > 256 {
                return None;
            }
            return Some(Raw {
                name: format!(
                    "{}(…)",
                    text.split_whitespace().collect::<Vec<_>>().join(" ")
                ),
                kind: Kind::Call,
                range: args.byte_range(),
                target: target.byte_range(),
            });
        }
        "identifier"
            if node.parent().is_some_and(|p| {
                p.kind() == "arrow_function"
                    && p.child_by_field_name("parameter")
                        .is_some_and(|n| n.id() == node.id())
            }) =>
        {
            return Some(Raw {
                name: source.get(node.byte_range())?.into(),
                kind: Kind::Parameter,
                range: range.clone(),
                target: range,
            });
        }
        _ => return None,
    };
    let (label, target) = name(node, source)
        .unwrap_or_else(|| (fallback.to_owned(), node.start_byte()..node.start_byte()));
    Some(Raw {
        name: label,
        kind,
        range,
        target,
    })
}

fn argument(node: Node<'_>) -> Option<Raw> {
    let parent = node.parent()?;
    if parent.kind() != "arguments" || node.kind() == "comment" {
        return None;
    }
    let mut cursor = parent.walk();
    let index = parent
        .named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
        .position(|n| n.id() == node.id())?;
    Some(Raw {
        name: format!("argumento {}", index + 1),
        kind: Kind::Argument,
        range: node.byte_range(),
        target: node.byte_range(),
    })
}

pub fn inspect(
    source: &str,
    language: Language,
    cancelled: &dyn Fn() -> bool,
) -> Result<Inspection, String> {
    if source.len() > MAX_BYTES {
        return Ok(Inspection {
            truncated: true,
            ..Default::default()
        });
    }
    let started = Instant::now();
    let stopped = || cancelled() || started.elapsed() >= Duration::from_millis(150);
    if stopped() {
        return Err("Análise de estrutura cancelada".into());
    }
    let mut parser = Parser::new();
    let grammar = match language {
        Language::Php => tree_sitter_php::LANGUAGE_PHP.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
    };
    parser.set_language(&grammar).map_err(|e| e.to_string())?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if stopped() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser
        .parse_with_options(
            &mut |offset, _| source.as_bytes().get(offset..).unwrap_or_default(),
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or("Análise de estrutura cancelada ou excedeu150ms")?;
    let mut stack = vec![(tree.root_node(), 0)];
    let mut raw = vec![];
    let mut visited = 0;
    while let Some((node, depth)) = stack.pop() {
        visited += 1;
        if stopped() {
            return Err("Análise de estrutura cancelada ou excedeu150ms".into());
        }
        if visited > MAX_NODES || depth > 256 || raw.len() >= MAX_SCOPES {
            return Ok(Inspection {
                truncated: true,
                ..Default::default()
            });
        }
        if !node.is_missing() {
            if let Some(scope) = argument(node) {
                raw.push(scope);
            }
            if let Some(scope) = classify(node, source) {
                raw.push(scope);
            }
        }
        let begin = stack.len();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push((child, depth + 1));
        }
        stack[begin..].reverse();
    }
    // One linear Unicode conversion for every boundary, not a prefix scan per node.
    let boundaries: BTreeSet<usize> = raw
        .iter()
        .flat_map(|s| [s.range.start, s.range.end, s.target.start, s.target.end])
        .collect();
    let mut boundaries = boundaries.into_iter().peekable();
    let mut positions = std::collections::BTreeMap::new();
    let mut line = 0;
    let mut cr = false;
    for (scalar, (byte, ch)) in source
        .char_indices()
        .chain(std::iter::once((source.len(), '\0')))
        .enumerate()
    {
        if scalar % 256 == 0 && stopped() {
            return Err("Conversão de estrutura cancelada".into());
        }
        if boundaries.peek() == Some(&byte) {
            positions.insert(byte, (scalar, line));
            boundaries.next();
        }
        if ch == '\r' {
            line += 1;
            cr = true;
        } else if ch == '\n' {
            if !cr {
                line += 1;
            }
            cr = false;
        } else {
            cr = false;
        }
        if boundaries.peek().is_none() {
            break;
        }
    }
    let scopes = raw
        .into_iter()
        .map(|s| Scope {
            name: s.name,
            kind: s.kind,
            range: positions[&s.range.start].0..positions[&s.range.end].0,
            target: positions[&s.target.start].0..positions[&s.target.end].0,
            line: positions[&s.target.start].1,
        })
        .collect();
    Ok(Inspection {
        scopes,
        truncated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn trail(text: &str, needle: &str, language: Language) -> Vec<String> {
        let at = text[..text.find(needle).unwrap() + 1].chars().count();
        inspect(text, language, &|| false)
            .unwrap()
            .at(at)
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }
    #[test]
    fn php_class_method_formal_parameter_and_call_arguments_have_real_containment() {
        let text = "<?php\r\nnamespace Demo; /* 😀 é */ class Main { function run(Greeting $greeter): string { return $greeter->greet('Ada', 2); } }\n// function fake() {}\n";
        assert_eq!(
            trail(text, "$greeter):", Language::Php),
            ["Demo", "Main", "run", "$greeter"]
        );
        assert_eq!(
            trail(text, "Ada", Language::Php),
            ["Demo", "Main", "run", "greet(…)", "argumento 1"]
        );
        assert_eq!(
            trail(text, "2);", Language::Php),
            ["Demo", "Main", "run", "greet(…)", "argumento 2"]
        );
        assert_eq!(trail(text, "fake", Language::Php), ["Demo"]);
        let inspected = inspect(text, Language::Php, &|| false).unwrap();
        let run = inspected.scopes.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(
            text.chars()
                .skip(run.target.start)
                .take(run.target.len())
                .collect::<String>(),
            "run"
        );
        assert_eq!(run.line, 1);
    }
    #[test]
    fn web_scopes_closures_generics_and_unicode_use_ast_for_js_ts_and_tsx() {
        for language in [Language::TypeScript, Language::Tsx] {
            let text = "// 😀 é\nexport class Service { run(value: string) { return this.greet(value, 2); } greet(a: string, b: number) {} }\nconst callback = (item: number) => { return item + 1; };\nconst text='function fake() {}';";
            assert_eq!(
                trail(text, "value: string", language),
                ["Service", "run", "value"]
            );
            assert_eq!(
                trail(text, "value, 2", language),
                ["Service", "run", "greet(…)", "argumento 1"]
            );
            assert_eq!(
                trail(text, "item + 1", language),
                ["callback", "função anônima"]
            );
            assert!(trail(text, "fake", language).is_empty());
        }
        let text = "class Main { run(value) { return value; } }";
        assert_eq!(
            trail(text, "return value", Language::TypeScript),
            ["Main", "run"]
        );
        let end = text.find(" } }").unwrap() + 3;
        assert_eq!(
            inspect(text, Language::TypeScript, &|| false)
                .unwrap()
                .at(end)
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Main"]
        );
    }
    #[test]
    fn nested_calls_keep_outer_argument_and_single_arrow_parameter() {
        let source = "function run() { return send(build(2)); }";
        assert_eq!(
            trail(source, "2)", Language::TypeScript),
            ["run", "send(…)", "argumento 1", "build(…)", "argumento 1"]
        );
        let source = "const callback = item => item + 1;";
        assert_eq!(
            trail(source, "item =>", Language::TypeScript),
            ["callback", "função anônima", "item"]
        );
    }
    #[test]
    fn incomplete_bodies_keep_named_parents_and_limits_never_publish_partial_chain() {
        let r = inspect(
            "<?php class Main { function run() { $x = ; } }",
            Language::Php,
            &|| false,
        )
        .unwrap();
        assert!(r.scopes.iter().any(|s| s.name == "run"));
        assert!(inspect("<?php", Language::Php, &|| true).is_err());
        assert!(
            inspect(&" ".repeat(MAX_BYTES + 1), Language::Php, &|| false)
                .unwrap()
                .truncated
        );
    }
}
