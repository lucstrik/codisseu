//! Bounded, independent PHPDoc tag generation for an explicit editing command.
//! This is syntax/literal inference, not project-wide type resolution.
use std::{
    ops::ControlFlow,
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser};

pub const MAX_BYTES: usize = 16 * 1024;
const MAX_NODES: usize = 4096;
const DEADLINE: Duration = Duration::from_millis(5);
const PREFIX: &str = "<?php class __CodisseuDoc {\n";

/// Generate tags for the immediately following named PHP function/method.
///
/// The caller supplies text *after* the `/**` line, without the opener. Comments
/// and unrelated code before the declaration are rejected. `None` means the
/// caller may insert a plain empty docblock instead. No input is modified.
pub fn function_tags(following: &str) -> Option<Vec<String>> {
    if following.len() > MAX_BYTES {
        return None;
    }
    let started = Instant::now();
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .ok()?;
    let source = format!("{PREFIX}{following}\n}}");
    let mut progress = |_: &tree_sitter::ParseState| {
        if started.elapsed() >= DEADLINE {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser.parse_with_options(
        &mut |offset, _| source.as_bytes().get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    )?;
    let root = tree.root_node();
    let mut cursor = root.walk();
    let class = root
        .named_children(&mut cursor)
        .find(|n| n.kind() == "class_declaration")?;
    let body = class.child_by_field_name("body")?;
    let declaration = body.named_child(0)?;
    if declaration.kind() != "method_declaration"
        || !source
            .get(PREFIX.len()..declaration.start_byte())?
            .trim()
            .is_empty()
    {
        return None;
    }
    // A recovered body is acceptable; a recovered signature is not.
    let parameters = declaration.child_by_field_name("parameters")?;
    let name = declaration.child_by_field_name("name")?;
    let body = declaration.child_by_field_name("body");
    let mut signature_cursor = declaration.walk();
    if declaration
        .children(&mut signature_cursor)
        .any(|n| Some(n) != body && (n.has_error() || n.is_missing() || n.kind() == "comment"))
        || parameters.has_error()
        || name.is_missing()
        || !text(parameters, &source)?.ends_with(')')
    {
        return None;
    }
    let mut budget = Budget { started, nodes: 0 };
    // Include unnamed/missing nodes in the budget and signature verification.
    let mut stack = vec![declaration];
    while let Some(node) = stack.pop() {
        budget.visit()?;
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    let mut tags = Vec::new();
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if parameter.kind() == "comment" {
            continue;
        }
        if !matches!(
            parameter.kind(),
            "simple_parameter" | "variadic_parameter" | "property_promotion_parameter"
        ) {
            return None;
        }
        let mut name = parameter.child_by_field_name("name")?;
        if name.kind() == "by_ref" {
            let mut cursor = name.walk();
            name = name
                .named_children(&mut cursor)
                .find(|n| n.kind() == "variable_name")?;
        }
        if name.kind() != "variable_name" || name.has_error() {
            return None;
        }
        let default = parameter.child_by_field_name("default_value");
        let mut ty = if let Some(ty) = parameter.child_by_field_name("type") {
            declared_type(ty, &source)?
        } else {
            default
                .and_then(|n| literal_type(n, &source))
                .filter(|ty| *ty != "null")
                .unwrap_or("mixed")
                .to_owned()
        };
        // Legacy PHP signatures permit implicit nullability via a null default.
        if default.and_then(|n| literal_type(n, &source)) == Some("null")
            && !ty
                .split(['|', '&', '(', ')'])
                .any(|part| part.eq_ignore_ascii_case("null") || part.eq_ignore_ascii_case("mixed"))
        {
            ty.push_str("|null");
        }
        let rest = if parameter.kind() == "variadic_parameter" {
            "..."
        } else {
            ""
        };
        tags.push(format!("@param {ty} {rest}{}", text(name, &source)?));
    }
    let name = text(name, &source)?;
    if !name.eq_ignore_ascii_case("__construct") && !name.eq_ignore_ascii_case("__destruct") {
        let ty = if let Some(ty) = declaration.child_by_field_name("return_type") {
            declared_type(ty, &source)?
        } else {
            infer_return(body, &source, &mut budget)?
        };
        tags.push(format!("@return {ty}"));
    }
    budget.visit()?;
    Some(tags)
}

struct Budget {
    started: Instant,
    nodes: usize,
}
impl Budget {
    fn visit(&mut self) -> Option<()> {
        self.nodes += 1;
        (self.nodes <= MAX_NODES && self.started.elapsed() < DEADLINE).then_some(())
    }
}
fn text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.byte_range())
}
fn declared_type(node: Node<'_>, source: &str) -> Option<String> {
    if node.has_error() || node.is_missing() {
        return None;
    }
    // Do not concatenate tokens around comments into a different type name.
    let raw = text(node, source)?;
    if raw.contains('/') || raw.contains('#') {
        return None;
    }
    let compact: String = raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    if compact.is_empty() {
        return None;
    }
    Some(if let Some(nullable) = compact.strip_prefix('?') {
        format!("{nullable}|null")
    } else {
        compact
    })
}
fn literal_type(mut node: Node<'_>, source: &str) -> Option<&'static str> {
    // Parentheses are bounded by the already checked tree/node budget.
    while node.kind() == "parenthesized_expression" {
        node = node.named_child(0)?;
    }
    if node.has_error() {
        return None;
    }
    match node.kind() {
        "integer" => portable_integer(text(node, source)?).then_some("int"),
        "float" => Some("float"),
        "boolean" => Some("bool"),
        "null" => Some("null"),
        "string" | "encapsed_string" | "heredoc" | "nowdoc" => Some("string"),
        "array_creation_expression" => Some("array"),
        "unary_op_expression" => {
            let raw = text(node, source)?.trim_start();
            let child = node.child_by_field_name("argument")?;
            if (raw.starts_with('+') || raw.starts_with('-'))
                && matches!(child.kind(), "integer" | "float")
            {
                literal_type(child, source)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn portable_integer(raw: &str) -> bool {
    let digits = raw.replace('_', "");
    let (radix, digits) = if digits.starts_with("0x") || digits.starts_with("0X") {
        (16, &digits[2..])
    } else if digits.starts_with("0b") || digits.starts_with("0B") {
        (2, &digits[2..])
    } else if digits.starts_with("0o") || digits.starts_with("0O") {
        (8, &digits[2..])
    } else if digits.starts_with('0') && digits.len() > 1 {
        (8, digits.as_str())
    } else {
        (10, digits.as_str())
    };
    // No target PHP runtime is consulted. Only promise int within the positive
    // 32-bit range shared by PHP's supported 32/64-bit integer representations.
    // Out-of-range tokens can evaluate to float; unknown is safer than a guess.
    u32::from_str_radix(digits, radix).is_ok_and(|value| value <= i32::MAX as u32)
}

fn infer_return(body: Option<Node<'_>>, source: &str, budget: &mut Budget) -> Option<String> {
    let Some(body) = body else {
        return Some("mixed".into());
    };
    if body.has_error() || body.end_byte() > source.len() - 2 {
        return Some("mixed".into());
    }
    let mut types = Vec::new();
    let mut stack = vec![body];
    let mut unknown = false;
    while let Some(node) = stack.pop() {
        budget.visit()?;
        match node.kind() {
            "anonymous_function"
            | "arrow_function"
            | "function_definition"
            | "class_declaration"
            | "anonymous_class"
            | "method_declaration" => continue,
            "yield_expression" => {
                unknown = true;
            }
            "return_statement" => {
                let mut cursor = node.walk();
                let value = node
                    .named_children(&mut cursor)
                    .find(|n| n.kind() != "comment");
                let ty = value.map_or(Some("void"), |n| literal_type(n, source));
                if let Some(ty) = ty {
                    if !types.contains(&ty) {
                        types.push(ty);
                    }
                } else {
                    unknown = true;
                }
                // A return expression itself may contain yield (e.g. [yield 1]).
                // Keep walking it, while skipping nested callable/class scopes.
            }
            _ => {}
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    let mut cursor = body.walk();
    let last = body
        .named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
        .last();
    // Without control-flow analysis, do not claim every path returns a literal.
    if unknown || (!types.is_empty() && last.is_none_or(|n| n.kind() != "return_statement")) {
        return Some("mixed".into());
    }
    if types.is_empty() {
        return Some("void".into());
    }
    if types.len() > 1 {
        for ty in &mut types {
            if *ty == "void" {
                *ty = "null";
            }
        }
        let mut unique = Vec::new();
        types.retain(|ty| {
            if unique.contains(ty) {
                false
            } else {
                unique.push(*ty);
                true
            }
        });
    }
    Some(types.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tags(source: &str) -> Vec<String> {
        function_tags(source).expect(source)
    }
    #[test]
    fn declared_signature_unicode_crlf_and_json_default() {
        assert_eq!(
            tags(
                "\r\n public function descrição(string $nome, $json = '{\"x\":\"a,b\"}'): string { return $nome; }"
            ),
            [
                "@param string $nome",
                "@param string $json",
                "@return string"
            ]
        );
    }
    #[test]
    fn attributes_multiline_promoted_and_reference_parameters() {
        assert_eq!(
            tags(
                "#[Example('a,b')]\npublic function __construct(\n #[Param] public readonly ?\\Domain\\Thing $thing, protected int &$count = 0, $other = null,\n) {}"
            ),
            [
                "@param \\Domain\\Thing|null $thing",
                "@param int $count",
                "@param mixed $other"
            ]
        );
    }
    #[test]
    fn unions_intersections_variadics_and_return_by_reference() {
        assert_eq!(
            tags("function &make((A&B)|C $x, ?int &...$values): (A&B)|null {}"),
            [
                "@param (A&B)|C $x",
                "@param int|null ...$values",
                "@return (A&B)|null"
            ]
        );
    }
    #[test]
    fn literal_defaults_and_unknowns() {
        assert_eq!(
            tags(
                "function f($i = -1, $f = +2.5, $b = true, $a = ['a,b', 1], $x = SOME_CONST, $n = null) {}"
            ),
            [
                "@param int $i",
                "@param float $f",
                "@param bool $b",
                "@param array $a",
                "@param mixed $x",
                "@param mixed $n",
                "@return void"
            ]
        );
    }
    #[test]
    fn implicit_nullable_defaults_preserve_the_accepted_parameter_domain() {
        assert_eq!(
            tags(
                "function f(string $name = null, ?int $count = NULL, mixed $x = null, int|null $y = null, (A&B)|C $z = null, $unknown = null) {}"
            ),
            [
                "@param string|null $name",
                "@param int|null $count",
                "@param mixed $x",
                "@param int|null $y",
                "@param (A&B)|C|null $z",
                "@param mixed $unknown",
                "@return void"
            ]
        );
    }
    #[test]
    fn integer_inference_is_portable_and_never_mislabels_overflow_as_int() {
        assert_eq!(
            tags(
                "function f($a = 2_147_483_647, $b = 0x7fffffff, $c = 0b101, $d = 0777, $e = 0o777) { return -1; }"
            ),
            [
                "@param int $a",
                "@param int $b",
                "@param int $c",
                "@param int $d",
                "@param int $e",
                "@return int"
            ]
        );
        for value in [
            "2147483648",
            "9223372036854775808",
            "-9223372036854775808",
            "0xffffffffffffffff",
            "0x80000000",
            "037777777777",
        ] {
            assert_eq!(
                tags(&format!("function f($x = {value}) {{ return {value}; }}")),
                ["@param mixed $x", "@return mixed"],
                "{value}"
            );
        }
    }
    #[test]
    fn yield_inside_return_expressions_still_marks_the_callable_unknown() {
        for value in ["[yield 1]", "array(yield 1)", "[yield from other()]"] {
            assert_eq!(
                tags(&format!("function f() {{ return {value}; }}")),
                ["@return mixed"],
                "{value}"
            );
        }
        assert_eq!(
            tags("function f() { return [function () { yield 1; }]; }"),
            ["@return array"]
        );
        assert_eq!(
            tags("function f() { return [fn() => yield 1]; }"),
            ["@return array"]
        );
    }
    #[test]
    fn return_literal_and_nested_scopes() {
        assert_eq!(
            tags(
                "function f() { $f = function () { return 1; }; $g = fn() => false; class C { function x() { return []; } } return 'ok'; }"
            ),
            ["@return string"]
        );
        assert_eq!(
            tags("function f() { if (ready()) return 1; return null; }"),
            ["@return int|null"]
        );
        assert_eq!(
            tags("function f() { return external(); }"),
            ["@return mixed"]
        );
        assert_eq!(
            tags("function f() { if (ready()) return 'yes'; }"),
            ["@return mixed"]
        );
        assert_eq!(
            tags("function f() { yield 1; return 'yes'; }"),
            ["@return mixed"]
        );
    }
    #[test]
    fn incomplete_body_keeps_declared_signature() {
        assert_eq!(
            tags("function f(string $s): string {"),
            ["@param string $s", "@return string"]
        );
        assert_eq!(tags("function f() {"), ["@return mixed"]);
        assert_eq!(tags("function f() { return 'ok';"), ["@return mixed"]);
    }
    #[test]
    fn void_destructor_and_abstract() {
        assert_eq!(tags("public function stop(): void {}"), ["@return void"]);
        assert!(tags("function __DESTRUCT() {}").is_empty());
        assert_eq!(
            tags("abstract public function make(int $a): static;"),
            ["@param int $a", "@return static"]
        );
    }
    #[test]
    fn reject_predecessors_closures_and_broken_signatures() {
        for source in [
            "/** old */ function f() {}",
            "#[Attr] /** old */ function f() {}",
            "// note\nfunction f() {}",
            "$x = 1; function f() {}",
            "'function f() {}';",
            "function () {}",
            "fn() => 1;",
            "function f(string $) {}",
            "function f($a, {",
            "function f(): {}",
            "function f($a)",
            "",
            "class C { function f() {} }",
        ] {
            assert!(function_tags(source).is_none(), "{source}");
        }
    }
    #[test]
    fn comments_do_not_change_literals_or_type_names() {
        assert_eq!(
            tags("function f(FooÁBar $x): \\Tipos\\Descrição { return $x; }"),
            ["@param FooÁBar $x", "@return \\Tipos\\Descrição"]
        );
        assert_eq!(
            tags("function f($x /* comma , */ = 'a,b') { /* return 2; */ return 'x'; }"),
            ["@param string $x", "@return string"]
        );
        assert_eq!(
            tags("function f() { if (ready()) return; if (other()) return 's'; return null; }"),
            ["@return null|string"]
        );
    }
    #[test]
    fn bounded_input_and_nodes() {
        assert!(function_tags(&" ".repeat(MAX_BYTES + 1)).is_none());
        let large = format!("function f() {{ {} }}", "$a = 1;".repeat(1500));
        assert!(large.len() < MAX_BYTES);
        assert!(function_tags(&large).is_none());
    }
}
