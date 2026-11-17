//! Source-derived request fields and validation rules. No PHP application boot.
use super::resources::{ResultSet, argument, position, quoted_node, target, walk};
use crate::php_project::{Budget, ComposerProject, Map, Names, add_maps, children, locate, text};
use crate::{CompletionEdit, CompletionItem, DefinitionTarget};
use serde_json::Value;
use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};
use tree_sitter::{Node, Tree};
const FORM_REQUEST: &str = "Illuminate\\Foundation\\Http\\FormRequest";
const REQUEST: &str = "Illuminate\\Http\\Request";
const VALIDATOR: &str = "Illuminate\\Support\\Facades\\Validator";
#[derive(Clone)]
struct Entry {
    name: String,
    target: DefinitionTarget,
}
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Field,
    Property,
    Rule,
    Dependent,
    RuleClass,
}
struct Context {
    kind: Kind,
    range: Range<usize>,
    receiver: Option<String>,
    schema: Option<Range<usize>>,
    subject: Option<String>,
    token: String,
}
fn enclosing<'a>(mut node: Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    loop {
        if kinds.contains(&node.kind()) {
            return Some(node);
        }
        node = node.parent()?;
    }
}
fn callable(node: Node<'_>) -> Option<Node<'_>> {
    enclosing(
        node,
        &[
            "method_declaration",
            "function_definition",
            "anonymous_function",
            "arrow_function",
        ],
    )
}
fn class_name(node: Node<'_>, source: &str, names: &Names) -> Option<String> {
    let class = enclosing(node, &["class_declaration"])?;
    names.resolve(text(class.child_by_field_name("name")?, source))
}
fn variable_type(node: Node<'_>, object: Node<'_>, source: &str, names: &Names) -> Option<String> {
    if object.kind() != "variable_name" {
        return None;
    }
    let variable = text(object, source);
    if variable == "$this" {
        return class_name(node, source, names);
    }
    let function = callable(node)?;
    // Reassignments, references and closures need data-flow analysis. A type hint
    // alone must not let us use the previous value after a write.
    let mut changed = false;
    walk(function, &mut |n| {
        if n.start_byte() < node.start_byte()
            && matches!(
                n.kind(),
                "assignment_expression"
                    | "augmented_assignment_expression"
                    | "reference_assignment_expression"
            )
            && (n
                .child_by_field_name("left")
                .is_some_and(|n| text(n, source) == variable)
                || n.kind() == "reference_assignment_expression"
                    && n.child_by_field_name("right")
                        .is_some_and(|n| text(n, source) == variable))
        {
            changed = true;
        }
        if n.kind() == "simple_parameter"
            && n.child_by_field_name("name")
                .is_some_and(|n| text(n, source) == variable)
            && children(n).iter().any(|n| n.kind() == "reference_modifier")
        {
            changed = true;
        }
    });
    if changed {
        return None;
    }
    let parameters = function.child_by_field_name("parameters")?;
    let parameter = children(parameters).into_iter().find(|n| {
        n.kind() == "simple_parameter"
            && n.child_by_field_name("name")
                .is_some_and(|n| text(n, source) == variable)
    })?;
    let kind = parameter.child_by_field_name("type")?;
    names.resolve(text(kind, source))
}
fn request_object<'a>(node: Node<'a>, source: &str) -> Option<Node<'a>> {
    if node.kind() == "variable_name" {
        return Some(node);
    }
    if node.kind() == "member_call_expression" {
        let method = text(node.child_by_field_name("name")?, source);
        if matches!(method, "validated" | "safe")
            && children(node.child_by_field_name("arguments")?).is_empty()
        {
            return request_object(node.child_by_field_name("object")?, source);
        }
    }
    None
}
fn facade(node: Node<'_>, source: &str, names: &Names) -> bool {
    node.kind() == "scoped_call_expression"
        && node
            .child_by_field_name("scope")
            .and_then(|n| names.resolve(text(n, source)))
            .as_deref()
            == Some(VALIDATOR)
}
fn validation_array<'a>(
    node: Node<'a>,
    source: &str,
    names: &Names,
) -> Option<(Node<'a>, Option<String>)> {
    if node.kind() == "scoped_call_expression"
        && facade(node, source, names)
        && node
            .child_by_field_name("name")
            .is_some_and(|n| matches!(text(n, source), "make" | "validate"))
    {
        let array = argument(node, 1)?;
        return (array.kind() == "array_creation_expression").then_some((array, None));
    }
    if node.kind() == "member_call_expression"
        && node
            .child_by_field_name("name")
            .is_some_and(|n| matches!(text(n, source), "validate" | "validateWithBag"))
    {
        let array = argument(
            node,
            if text(node.child_by_field_name("name")?, source) == "validateWithBag" {
                1
            } else {
                0
            },
        )?;
        let object = node.child_by_field_name("object")?;
        let class = variable_type(node, object, source, names)?;
        return (array.kind() == "array_creation_expression").then_some((array, Some(class)));
    }
    None
}
fn rule_array<'a>(
    literal: Node<'a>,
    source: &str,
    names: &Names,
) -> Option<(Node<'a>, Option<String>)> {
    let mut node = literal;
    loop {
        if node.kind() == "array_creation_expression" {
            let parent = node.parent()?;
            if parent.kind() == "return_statement" {
                let method = enclosing(parent, &["method_declaration"])?;
                if text(method.child_by_field_name("name")?, source) == "rules" {
                    return Some((node, Some(class_name(method, source, names)?)));
                }
            }
            if parent.kind() == "argument" {
                let call = parent.parent()?.parent()?;
                if let Some((array, class)) = validation_array(call, source, names)
                    && array.id() == node.id()
                {
                    return Some((array, class));
                }
            }
        }
        node = node.parent()?;
    }
}
fn schema_fields(path: &Path, source: &str, array: Node<'_>) -> Option<Vec<Entry>> {
    let mut fields = Vec::new();
    for item in children(array) {
        if item.kind() == "comment" {
            continue;
        }
        let parts = children(item);
        if parts.len() != 2 {
            return None;
        }
        let (name, range) = quoted_node(parts[0], source)?;
        if name.is_empty() {
            return None;
        }
        fields.retain(|e: &Entry| e.name != name);
        fields.push(Entry {
            name,
            target: target(path, source, range),
        });
    }
    Some(fields)
}
fn literal_context(tree: &Tree, source: &str, byte: usize, names: &Names) -> Option<Context> {
    let mut found = None;
    walk(tree.root_node(), &mut |node| {
        if found.is_some() {
            return;
        }
        let Some((_, range)) = quoted_node(node, source) else {
            return;
        };
        if !(range.start <= byte && byte <= range.end) {
            return;
        }
        // Direct literal field arguments, plus arrays passed to only/except.
        let mut arg = node;
        while let Some(parent) = arg.parent() {
            if parent.kind() == "argument" {
                if let Some(call) = parent.parent().and_then(|n| n.parent())
                    && call.kind() == "member_call_expression"
                    && let Some(method) = call.child_by_field_name("name")
                    && matches!(
                        text(method, source),
                        "input"
                            | "query"
                            | "post"
                            | "only"
                            | "except"
                            | "validated"
                            | "has"
                            | "filled"
                            | "missing"
                    )
                    && let Some(object) = call
                        .child_by_field_name("object")
                        .and_then(|n| request_object(n, source))
                    && let Some(class) = variable_type(call, object, source, names)
                {
                    let method = text(method, source);
                    let first = argument(call, 0);
                    if matches!(method, "only" | "except" | "has" | "filled" | "missing")
                        || first.is_some_and(|n| n.id() == node.id())
                    {
                        found = Some(Context {
                            kind: Kind::Field,
                            range: range.clone(),
                            receiver: Some(class),
                            schema: None,
                            subject: Some(text(object, source).into()),
                            token: String::new(),
                        });
                        return;
                    }
                }
                break;
            }
            if !matches!(
                parent.kind(),
                "array_creation_expression" | "array_element_initializer"
            ) {
                break;
            }
            arg = parent;
        }
        if let Some(parent) = node.parent()
            && parent.kind() == "subscript_expression"
        {
            let parts = children(parent);
            if parts.len() == 2
                && parts[1].id() == node.id()
                && let Some(object) = request_object(parts[0], source)
                && let Some(class) = variable_type(parent, object, source, names)
            {
                found = Some(Context {
                    kind: Kind::Field,
                    range: range.clone(),
                    receiver: Some(class),
                    schema: None,
                    subject: Some(text(object, source).into()),
                    token: String::new(),
                });
                return;
            }
        }
        let Some((array, receiver)) = rule_array(node, source, names) else {
            return;
        };
        // Only values are rules. Keys declare fields and aren't a catalog of
        // arbitrary fields from other requests.
        let mut element = node;
        loop {
            let Some(parent) = element.parent() else {
                return;
            };
            if parent.id() == array.id() {
                break;
            }
            element = parent;
        }
        let parts = children(element);
        if parts.len() != 2 || parts[0].start_byte() <= byte && byte <= parts[0].end_byte() {
            return;
        }
        let direct_value = parts[1].id() == node.id()
            || parts[1].kind() == "array_creation_expression"
                && node.parent().is_some_and(|p| {
                    p.kind() == "array_element_initializer"
                        && children(p).len() == 1
                        && p.parent().is_some_and(|a| a.id() == parts[1].id())
                });
        if !direct_value {
            return;
        }
        let piped = parts[1].id() == node.id();
        let start = if piped {
            source[range.start..byte]
                .rfind('|')
                .map_or(range.start, |n| range.start + n + 1)
        } else {
            range.start
        };
        let end = if piped {
            source[byte..range.end]
                .find('|')
                .map_or(range.end, |n| byte + n)
        } else {
            range.end
        };
        let segment = &source[start..end];
        if let Some(colon) = segment.find(':') {
            if byte <= start + colon {
                found = Some(Context {
                    kind: Kind::Rule,
                    range: start..start + colon,
                    receiver,
                    schema: Some(array.byte_range()),
                    subject: None,
                    token: String::new(),
                });
            } else {
                let parameter_start = source[start + colon + 1..byte]
                    .rfind(',')
                    .map_or(start + colon + 1, |n| start + colon + 2 + n);
                let parameter_end = source[byte..end].find(',').map_or(end, |n| byte + n);
                found = Some(Context {
                    kind: Kind::Dependent,
                    range: parameter_start..parameter_end,
                    receiver,
                    schema: Some(array.byte_range()),
                    subject: None,
                    token: segment[..colon].into(),
                });
            }
        } else {
            found = Some(Context {
                kind: Kind::Rule,
                range: start..end,
                receiver,
                schema: Some(array.byte_range()),
                subject: None,
                token: String::new(),
            });
        }
    });
    found
}
fn context(tree: &Tree, source: &str, byte: usize, names: &Names) -> Option<Context> {
    if let Some(context) = literal_context(tree, source, byte, names) {
        return Some(context);
    }
    let mut found = None;
    walk(tree.root_node(), &mut |node| {
        if matches!(node.kind(), "name" | "qualified_name")
            && node.start_byte() <= byte
            && byte <= node.end_byte()
            && node
                .parent()
                .is_some_and(|p| p.kind() == "object_creation_expression")
            && let Some((array, receiver)) = rule_array(node, source, names)
        {
            found = Some(Context {
                kind: Kind::RuleClass,
                range: node.byte_range(),
                receiver,
                schema: Some(array.byte_range()),
                subject: None,
                token: String::new(),
            });
            return;
        }
        if node.kind() != "member_access_expression" {
            return;
        }
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        if name.kind() != "name" || !(name.start_byte() <= byte && byte <= name.end_byte()) {
            return;
        }
        let Some(object) = node
            .child_by_field_name("object")
            .and_then(|n| request_object(n, source))
        else {
            return;
        };
        let Some(class) = variable_type(node, object, source, names) else {
            return;
        };
        found = Some(Context {
            kind: Kind::Property,
            range: name.byte_range(),
            receiver: Some(class),
            schema: None,
            subject: Some(text(object, source).into()),
            token: String::new(),
        });
    });
    found
}
fn maps(project: &ComposerProject, budget: &mut Budget<'_>) -> Option<Vec<Map>> {
    let mut maps = Vec::new();
    for key in ["autoload", "autoload-dev"] {
        if let Some(value) = project.manifest.get(key) {
            add_maps(&mut maps, &project.root, value)?;
        }
    }
    let (_, installed) = budget.read(&project.root.join("vendor/composer/installed.json"))?;
    let data: Value = serde_json::from_str(&installed).ok()?;
    let packages = data.get("packages").unwrap_or(&data).as_array()?;
    if packages.len() > 1024 {
        return None;
    }
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if name.split('/').count() != 2 || name.contains("..") {
            continue;
        }
        let base = package["install-path"].as_str().map_or_else(
            || project.root.join("vendor").join(name),
            |p| project.root.join("vendor/composer").join(p),
        );
        if let Some(value) = package.get("autoload") {
            add_maps(&mut maps, &base, value)?;
        }
    }
    Some(maps)
}
fn request_fields(
    budget: &mut Budget<'_>,
    maps: &[Map],
    fqcn: &str,
    current_path: &Path,
    current_source: &str,
) -> Option<(Vec<Entry>, HashSet<String>)> {
    let mut class = fqcn.to_string();
    let mut seen = HashSet::new();
    let mut fields = None;
    let mut properties = HashSet::new();
    let mut reached_request = false;
    for _ in 0..32 {
        if !seen.insert(class.clone()) {
            return None;
        }
        let (path, mut source) = locate(budget, maps, &class)?;
        if path == current_path {
            source = current_source.into();
        }
        let tree = budget.tree(&source)?;
        let names = Names::parse(&tree, &source)?;
        let node = children(tree.root_node()).into_iter().find(|n| {
            n.kind() == "class_declaration"
                && n.child_by_field_name("name")
                    .and_then(|n| names.resolve(text(n, &source)))
                    .as_deref()
                    == Some(&class)
        })?;
        let body = node.child_by_field_name("body")?;
        for declaration in children(body) {
            if declaration.kind() == "property_declaration"
                && !children(declaration)
                    .iter()
                    .any(|n| n.kind() == "visibility_modifier" && text(*n, &source) != "public")
            {
                for element in children(declaration)
                    .into_iter()
                    .filter(|n| n.kind() == "property_element")
                {
                    if let Some(name) = element.child_by_field_name("name") {
                        properties.insert(text(name, &source).trim_start_matches('$').to_string());
                    }
                }
            }
            if declaration.kind() == "method_declaration" {
                walk(declaration, &mut |n| {
                    if n.kind() == "property_promotion_parameter"
                        && children(n).iter().any(|n| {
                            n.kind() == "visibility_modifier" && text(*n, &source) == "public"
                        })
                        && let Some(name) = n.child_by_field_name("name")
                    {
                        properties.insert(text(name, &source).trim_start_matches('$').to_string());
                    }
                });
            }
        }
        reached_request |= class == FORM_REQUEST || class == REQUEST;
        if !reached_request
            && children(body).iter().any(|n| {
                n.kind() == "method_declaration"
                    && n.child_by_field_name("name").is_some_and(|n| {
                        matches!(
                            text(n, &source),
                            "__get"
                                | "offsetGet"
                                | "input"
                                | "query"
                                | "post"
                                | "only"
                                | "except"
                                | "validated"
                                | "safe"
                                | "has"
                                | "filled"
                                | "missing"
                        )
                    })
            })
        {
            return None;
        }
        if !reached_request && children(body).iter().any(|n| n.kind() == "use_declaration") {
            return None;
        }
        if !reached_request
            && fields.is_none()
            && let Some(method) = children(body).into_iter().find(|n| {
                n.kind() == "method_declaration"
                    && n.child_by_field_name("name")
                        .is_some_and(|n| text(n, &source) == "rules")
            })
        {
            let statements: Vec<_> = children(method.child_by_field_name("body")?)
                .into_iter()
                .filter(|n| n.kind() != "comment")
                .collect();
            if statements.len() != 1 || statements[0].kind() != "return_statement" {
                return None;
            }
            let value = children(statements[0]).into_iter().next()?;
            if value.kind() != "array_creation_expression" {
                return None;
            }
            fields = Some(schema_fields(&path, &source, value)?);
        }
        let Some(base) = children(node)
            .into_iter()
            .find(|n| n.kind() == "base_clause")
        else {
            return reached_request.then(|| (fields.unwrap_or_default(), properties));
        };
        let parents = children(base);
        if parents.len() != 1 {
            return None;
        }
        class = names.resolve(text(parents[0], &source))?;
    }
    None
}
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (index, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}
fn framework_rules(budget: &mut Budget<'_>, maps: &[Map]) -> Option<Vec<Entry>> {
    let fqcn = "Illuminate\\Validation\\Concerns\\ValidatesAttributes";
    let (path, source) = locate(budget, maps, fqcn)?;
    let tree = budget.tree(&source)?;
    let names = Names::parse(&tree, &source)?;
    let node = children(tree.root_node()).into_iter().find(|n| {
        n.kind() == "trait_declaration"
            && n.child_by_field_name("name")
                .and_then(|n| names.resolve(text(n, &source)))
                .as_deref()
                == Some(fqcn)
    })?;
    let mut result = Vec::new();
    for method in children(node.child_by_field_name("body")?) {
        if method.kind() != "method_declaration" {
            continue;
        }
        let Some(name) = method.child_by_field_name("name") else {
            continue;
        };
        let Some(rule) = text(name, &source)
            .strip_prefix("validate")
            .filter(|n| n.starts_with(|c: char| c.is_ascii_uppercase()))
        else {
            continue;
        };
        result.push(Entry {
            name: snake(rule),
            target: target(&path, &source, name.byte_range()),
        });
    }
    Some(result)
}
// Data-use analysis is confined to installed framework methods: follow direct
// parameter forwarding and foreach values to getValue/Arr::{get,has} sinks.
fn dependent_parameter(
    budget: &mut Budget<'_>,
    maps: &[Map],
    rule: &str,
    index: usize,
) -> Option<()> {
    let (_, source) = locate(
        budget,
        maps,
        "Illuminate\\Validation\\Concerns\\ValidatesAttributes",
    )?;
    let tree = budget.tree(&source)?;
    let names = Names::parse(&tree, &source)?;
    let declaration = children(tree.root_node())
        .into_iter()
        .find(|n| n.kind() == "trait_declaration")?;
    let methods: Vec<_> = children(declaration.child_by_field_name("body")?)
        .into_iter()
        .filter(|n| n.kind() == "method_declaration")
        .collect();
    fn used(
        method: Node<'_>,
        source: &str,
        names: &Names,
        methods: &[Node<'_>],
        variable: &str,
        index: usize,
        depth: usize,
    ) -> bool {
        if depth > 4 {
            return false;
        }
        let mut found = false;
        walk(method, &mut |node| {
            if node.kind() == "member_call_expression" || node.kind() == "scoped_call_expression" {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| text(n, source))
                    .unwrap_or("");
                let sink = if node.kind() == "member_call_expression"
                    && node
                        .child_by_field_name("object")
                        .is_some_and(|n| text(n, source) == "$this")
                    && name == "getValue"
                {
                    argument(node, 0)
                } else if node.kind() == "scoped_call_expression"
                    && matches!(name, "get" | "has")
                    && node
                        .child_by_field_name("scope")
                        .and_then(|n| names.resolve(text(n, source)))
                        .as_deref()
                        == Some("Illuminate\\Support\\Arr")
                    && argument(node, 0).is_some_and(|n| text(n, source) == "$this->data")
                {
                    argument(node, 1)
                } else {
                    None
                };
                if let Some(sink) = sink {
                    walk(sink, &mut |n| {
                        if n.kind() == "subscript_expression" {
                            let parts = children(n);
                            if parts.len() == 2
                                && text(parts[0], source) == variable
                                && text(parts[1], source).parse::<usize>().ok() == Some(index)
                            {
                                found = true;
                            }
                        }
                    });
                    // A foreach over the whole parameter array supplies all its
                    // values as fields. Keys are deliberately excluded.
                    if sink.kind() == "variable_name" {
                        let mut parent = sink.parent();
                        while let Some(p) = parent {
                            if p.id() == method.id() {
                                break;
                            }
                            if p.kind() == "foreach_statement" {
                                let parts = children(p);
                                if parts.len() >= 3
                                    && text(parts[0], source) == variable
                                    && parts[1].kind() == "variable_name"
                                    && text(parts[1], source) == text(sink, source)
                                {
                                    found = true;
                                }
                            }
                            parent = p.parent();
                        }
                    }
                }
                if node.kind() == "member_call_expression"
                    && node
                        .child_by_field_name("object")
                        .is_some_and(|n| text(n, source) == "$this")
                {
                    for i in 0..4 {
                        let Some(arg) = argument(node, i) else {
                            continue;
                        };
                        let Some(callee) = methods.iter().find(|n| {
                            n.child_by_field_name("name")
                                .is_some_and(|n| text(n, source) == name)
                        }) else {
                            continue;
                        };
                        let Some(parameter) = callee
                            .child_by_field_name("parameters")
                            .and_then(|n| {
                                children(n)
                                    .into_iter()
                                    .filter(|n| n.kind() == "simple_parameter")
                                    .nth(i)
                            })
                            .and_then(|n| n.child_by_field_name("name"))
                        else {
                            continue;
                        };
                        if text(arg, source) == variable
                            && used(
                                *callee,
                                source,
                                names,
                                methods,
                                text(parameter, source),
                                index,
                                depth + 1,
                            )
                        {
                            found = true;
                        }
                        if arg.kind() == "array_creation_expression" {
                            for (forwarded_index, element) in children(arg).into_iter().enumerate()
                            {
                                let parts = children(element);
                                if parts.len() != 1 {
                                    continue;
                                }
                                let mut forwards = false;
                                walk(parts[0], &mut |n| {
                                    if n.kind() == "subscript_expression" {
                                        let parts = children(n);
                                        if parts.len() == 2
                                            && text(parts[0], source) == variable
                                            && text(parts[1], source).parse::<usize>().ok()
                                                == Some(index)
                                        {
                                            forwards = true;
                                        }
                                    }
                                });
                                if forwards
                                    && used(
                                        *callee,
                                        source,
                                        names,
                                        methods,
                                        text(parameter, source),
                                        forwarded_index,
                                        depth + 1,
                                    )
                                {
                                    found = true;
                                }
                            }
                        }
                    }
                }
            }
        });
        found
    }
    let method = methods.iter().find(|n| {
        n.child_by_field_name("name")
            .is_some_and(|n| snake(text(n, &source).strip_prefix("validate").unwrap_or("")) == rule)
    })?;
    used(*method, &source, &names, &methods, "$parameters", index, 0).then_some(())
}
fn provider_files(
    budget: &Budget<'_>,
    dir: &Path,
    depth: usize,
    visited: &mut usize,
    files: &mut Vec<PathBuf>,
    truncated: &mut bool,
) {
    if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
        return;
    }
    if depth > 12 || budget.check().is_none() {
        *truncated = true;
        return;
    }
    let Ok(canonical) = dir.canonicalize() else {
        return;
    };
    if !canonical.starts_with(budget.root) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&canonical) else {
        return;
    };
    for entry in entries {
        if *visited >= 4096 || budget.check().is_none() {
            *truncated = true;
            return;
        }
        *visited += 1;
        let Ok(entry) = entry else {
            *truncated = true;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *truncated = true;
            continue;
        };
        if kind.is_dir() {
            provider_files(budget, &entry.path(), depth + 1, visited, files, truncated);
        } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "php") {
            files.push(entry.path());
        }
    }
}
fn extensions(
    budget: &mut Budget<'_>,
    project: &ComposerProject,
    current_path: &Path,
    source: &str,
    tree: &Tree,
    names: &Names,
    truncated: &mut bool,
) -> Vec<Entry> {
    fn collect(path: &Path, source: &str, tree: &Tree, names: &Names) -> Vec<Entry> {
        let mut entries = Vec::new();
        walk(tree.root_node(), &mut |n| {
            if facade(n, source, names)
                && n.child_by_field_name("name").is_some_and(|n| {
                    matches!(
                        text(n, source),
                        "extend" | "extendImplicit" | "extendDependent"
                    )
                })
                && let Some((name, range)) = argument(n, 0).and_then(|n| quoted_node(n, source))
            {
                entries.push(Entry {
                    name,
                    target: target(path, source, range),
                });
            }
        });
        entries
    }
    let mut result = collect(current_path, source, tree, names);
    let mut files = Vec::new();
    provider_files(
        budget,
        &project.root.join("app/Providers"),
        0,
        &mut 0,
        &mut files,
        truncated,
    );
    files.sort();
    for path in files {
        if path == current_path {
            continue;
        }
        let Some((path, source)) = budget.read(&path) else {
            *truncated = true;
            break;
        };
        let Some(tree) = budget.tree(&source) else {
            *truncated = true;
            break;
        };
        if let Some(names) = Names::parse(&tree, &source) {
            result.extend(collect(&path, &source, &tree, &names));
        }
    }
    result
}
fn rule_classes(
    budget: &mut Budget<'_>,
    project: &ComposerProject,
    maps: &[Map],
    truncated: &mut bool,
) -> Vec<Entry> {
    let mut files = Vec::new();
    provider_files(
        budget,
        &project.root.join("app/Rules"),
        0,
        &mut 0,
        &mut files,
        truncated,
    );
    files.sort();
    let contracts: HashSet<_> = [
        "Illuminate\\Contracts\\Validation\\ValidationRule",
        "Illuminate\\Contracts\\Validation\\InvokableRule",
        "Illuminate\\Contracts\\Validation\\Rule",
    ]
    .into_iter()
    .filter(|contract| {
        let Some((_, source)) = locate(budget, maps, contract) else {
            return false;
        };
        let Some(tree) = budget.tree(&source) else {
            return false;
        };
        let Some(names) = Names::parse(&tree, &source) else {
            return false;
        };
        children(tree.root_node()).iter().any(|n| {
            n.kind() == "interface_declaration"
                && n.child_by_field_name("name")
                    .and_then(|n| names.resolve(text(n, &source)))
                    .as_deref()
                    == Some(*contract)
        })
    })
    .collect();
    let mut result = Vec::new();
    for path in files {
        let Some((path, source)) = budget.read(&path) else {
            *truncated = true;
            break;
        };
        let Some(tree) = budget.tree(&source) else {
            *truncated = true;
            break;
        };
        let Some(names) = Names::parse(&tree, &source) else {
            continue;
        };
        for declaration in children(tree.root_node())
            .into_iter()
            .filter(|n| n.kind() == "class_declaration" && !n.has_error())
        {
            if children(declaration)
                .iter()
                .any(|n| n.kind() == "abstract_modifier")
            {
                continue;
            }
            let Some(interfaces) = children(declaration)
                .into_iter()
                .find(|n| n.kind() == "class_interface_clause")
            else {
                continue;
            };
            if !children(interfaces).iter().any(|n| {
                names
                    .resolve(text(*n, &source))
                    .is_some_and(|n| contracts.contains(n.as_str()))
            }) {
                continue;
            }
            let Some(name) = declaration.child_by_field_name("name") else {
                continue;
            };
            let Some(name_value) = names.resolve(text(name, &source)) else {
                continue;
            };
            if locate(budget, maps, &name_value).is_none_or(|(located, _)| located != path) {
                continue;
            }
            result.push(Entry {
                name: name_value,
                target: target(&path, &source, name.byte_range()),
            });
        }
    }
    result
}
fn unclosed_quote(source: &str, byte: usize) -> Option<char> {
    let mut at = 0;
    let mut php = false;
    while at < source.len() {
        let rest = &source[at..];
        if !php {
            at += rest.find("<?")? + 2;
            php = true;
            continue;
        }
        if rest.starts_with("?>") {
            php = false;
            at += 2;
            continue;
        }
        if rest.starts_with("/*") {
            at += rest.find("*/").map_or(rest.len(), |n| n + 2);
            continue;
        }
        if rest.starts_with("//") || rest.starts_with('#') && !rest.starts_with("#[") {
            at += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<<<") {
            return None;
        }
        if matches!(source.as_bytes()[at], b'\'' | b'"') {
            let quote = source.as_bytes()[at];
            let start = at;
            at += 1;
            let mut plain = true;
            while at < source.len() && source.as_bytes()[at] != quote {
                if source.as_bytes()[at] == b'\\' {
                    plain = false;
                    at = (at + 2).min(source.len());
                } else {
                    if quote == b'"' && source.as_bytes()[at] == b'$' {
                        plain = false;
                    }
                    at += 1;
                }
            }
            if at == source.len() {
                return (plain && start < byte).then_some(quote as char);
            }
            at += 1;
        } else {
            at += rest.chars().next()?.len_utf8();
        }
    }
    None
}
/// Used by the dispatcher to retain semantic PHP members while augmenting
/// magic request-property completion. Performs bounded parsing, no filesystem.
pub(crate) fn property_context(source: &str, line: u32, character: u32) -> bool {
    let inspect = || -> Option<bool> {
        if source.len() > 1024 * 1024 {
            return None;
        }
        let byte = crate::utf16_byte_offset(source, line, character)?;
        let cancel = AtomicBool::new(false);
        let budget = Budget {
            buffers: &[],
            root: Path::new(""),
            cancel: &cancel,
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        let parsed = if source[..byte].ends_with("->") {
            format!("{}__CodisseuField{}", &source[..byte], &source[byte..])
        } else {
            source.into()
        };
        let tree = budget.tree(&parsed)?;
        let names = Names::parse(&tree, &parsed)?;
        Some(context(&tree, &parsed, byte, &names)?.kind == Kind::Property)
    };
    inspect().unwrap_or(false)
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn query(
    workspace: &Path,
    path: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
    completion: bool,
) -> Option<ResultSet> {
    if source.len() > 1024 * 1024 || path.to_str()?.ends_with(".blade.php") {
        return None;
    }
    let byte = crate::utf16_byte_offset(source, line, character)?;
    let mut budget = Budget {
        buffers,
        root: workspace,
        cancel,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    let mut parsed = source.to_string();
    let mut tree = budget.tree(&parsed)?;
    let mut names = Names::parse(&tree, &parsed)?;
    let mut selected = context(&tree, &parsed, byte, &names);
    if selected.is_none() && completion {
        if let Some(quote) = unclosed_quote(source, byte) {
            for suffix in [");", ");}", ");}}", "]);", "];}}", "]);}}"] {
                parsed = format!("{source}{quote}{suffix}");
                tree = budget.tree(&parsed)?;
                names = Names::parse(&tree, &parsed)?;
                selected = context(&tree, &parsed, byte, &names);
                if selected.is_some() {
                    break;
                }
            }
        } else if source[..byte].ends_with("->") {
            parsed = format!("{}__CodisseuField{}", &source[..byte], &source[byte..]);
            tree = budget.tree(&parsed)?;
            names = Names::parse(&tree, &parsed)?;
            selected = context(&tree, &parsed, byte, &names);
            if let Some(context) = &mut selected {
                context.range.end = byte;
            }
        }
    }
    let mut context = selected?;
    context.range.end = context.range.end.min(source.len());
    let project = ComposerProject::discover(&mut budget, path)?;
    if !project.is_laravel() {
        return None;
    }
    let maps = maps(&project, &mut budget)?;
    let (mut fields, properties) = if let Some(receiver) = &context.receiver {
        request_fields(
            &mut budget,
            &maps,
            receiver,
            path,
            if context.schema.is_some() {
                &parsed
            } else {
                source
            },
        )?
    } else {
        (Vec::new(), HashSet::new())
    };
    if context.kind == Kind::Property && properties.contains(source.get(context.range.clone())?) {
        return None;
    }
    if let Some(range) = &context.schema {
        let array = tree
            .root_node()
            .descendant_for_byte_range(range.start, range.end)?;
        fields = schema_fields(path, &parsed, array)?;
    } else {
        // Associate inline validation only with this exact receiver inside the
        // same callable. It is never a project-wide union of request fields.
        let at = tree.root_node().descendant_for_byte_range(byte, byte)?;
        if let Some(function) = callable(at) {
            walk(function, &mut |n| {
                if n.end_byte() > byte {
                    return;
                }
                if let Some((array, receiver)) = validation_array(n, source, &names)
                    && receiver == context.receiver
                    && n.child_by_field_name("object")
                        .is_some_and(|n| Some(text(n, source)) == context.subject.as_deref())
                    && callable(n).is_some_and(|owner| owner.id() == function.id())
                    && n.start_byte() < byte
                    && let Some(entries) = schema_fields(path, source, array)
                {
                    fields.extend(entries);
                }
            });
        }
    }
    let mut truncated = false;
    let mut entries = if context.kind == Kind::Rule {
        let mut entries = framework_rules(&mut budget, &maps)?;
        entries.extend(extensions(
            &mut budget,
            &project,
            path,
            source,
            &tree,
            &names,
            &mut truncated,
        ));
        entries
    } else if context.kind == Kind::RuleClass {
        rule_classes(&mut budget, &project, &maps, &mut truncated)
    } else {
        if context.kind == Kind::Dependent {
            let literal_start = source[..context.range.start].rfind(['\'', '"'])? + 1;
            let segment = &source[literal_start..context.range.start];
            let segment = segment.rsplit('|').next()?;
            let after_colon = segment.split_once(':')?.1;
            let index = after_colon.bytes().filter(|b| *b == b',').count();
            dependent_parameter(&mut budget, &maps, &context.token, index)?;
        }
        if context.kind == Kind::Property {
            fields.retain(|e| {
                !e.name.contains(['.', '*'])
                    && !properties.contains(&e.name)
                    && e.name.chars().all(|c| c.is_alphanumeric() || c == '_')
            });
        }
        fields
    };
    budget.check()?;
    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.target.path.cmp(&b.target.path)));
    let name = source.get(context.range.clone())?;
    let resolved_name = if context.kind == Kind::RuleClass {
        names.resolve(name)
    } else {
        Some(name.into())
    };
    let prefix = source.get(context.range.start..byte)?;
    let mut targets: Vec<_> = entries
        .iter()
        .filter(|e| Some(&e.name) == resolved_name.as_ref())
        .map(|e| e.target.clone())
        .collect();
    targets.dedup();
    if targets.len() > 64 {
        targets.clear();
        truncated = true;
    }
    let mut items = Vec::new();
    if completion {
        let (start_line, start_character) = position(source, context.range.start);
        let (end_line, end_character) = position(source, context.range.end);
        for entry in entries.into_iter().filter(|e| {
            if context.kind == Kind::RuleClass {
                e.name
                    .rsplit('\\')
                    .next()
                    .is_some_and(|n| n.starts_with(prefix))
                    || names
                        .resolve(prefix)
                        .is_some_and(|n| e.name.starts_with(&n))
            } else {
                e.name.starts_with(prefix)
            }
        }) {
            if items.iter().any(|i: &CompletionItem| i.label == entry.name) {
                continue;
            }
            if items.len() >= 128 {
                truncated = true;
                break;
            }
            let insertion = if context.kind == Kind::RuleClass {
                format!("\\{}", entry.name)
            } else {
                entry.name.clone()
            };
            items.push(CompletionItem {
                label: entry.name.clone(),
                detail: Some(format!(
                    "Laravel {} · inferred from {}",
                    if context.kind == Kind::Rule {
                        "validation rule"
                    } else if context.kind == Kind::RuleClass {
                        "validation class"
                    } else {
                        "request field"
                    },
                    entry
                        .target
                        .path
                        .strip_prefix(workspace)
                        .unwrap_or(&entry.target.path)
                        .display()
                )),
                kind: Some(match context.kind {
                    Kind::Rule => 12,
                    Kind::RuleClass => 7,
                    _ => 10,
                }),
                filter_text: Some(entry.name.clone()),
                sort_text: None,
                new_text: insertion.clone(),
                text_edit: Some(CompletionEdit {
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    new_text: insertion,
                }),
                insert_edit: None,
                additional_edits: Vec::new(),
                unsupported_reason: None,
            });
        }
    }
    if truncated && !completion {
        targets.clear();
    }
    Some(ResultSet {
        items,
        targets,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn put(root: &Path, name: &str, source: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path.canonicalize().unwrap()
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(
            root,
            "composer.json",
            r#"{"require":{"laravel/framework":"^12.0"},"autoload":{"psr-4":{"App\\":"app/"}}}"#,
        );
        put(
            root,
            "vendor/composer/installed.json",
            r#"{"packages":[{"name":"laravel/framework","autoload":{"psr-4":{"Illuminate\\":"src/Illuminate/"}}}]}"#,
        );
        put(
            root,
            "vendor/laravel/framework/src/Illuminate/Foundation/Http/FormRequest.php",
            "<?php namespace Illuminate\\Foundation\\Http; class FormRequest extends \\Illuminate\\Http\\Request {}",
        );
        put(
            root,
            "vendor/laravel/framework/src/Illuminate/Http/Request.php",
            "<?php namespace Illuminate\\Http; class Request {}",
        );
        // Own tiny source fixtures. Not an imitation of installed framework code.
        put(
            root,
            "vendor/laravel/framework/src/Illuminate/Validation/Concerns/ValidatesAttributes.php",
            r#"<?php namespace Illuminate\Validation\Concerns; use Illuminate\Support\Arr; trait ValidatesAttributes {
          public function validateRequired($attribute, $value) { return true; }
          public function validateEmail($attribute, $value) { return true; }
          public function validateCodisseuFixture($attribute, $value) { return true; }
          public function validateSame($attribute, $value, $parameters) { return Arr::get($this->data, $parameters[0]); }
          public function validateRequiredIf($attribute, $value, $parameters) { return $this->parseDependentRuleParameters($parameters); }
          public function parseDependentRuleParameters($parameters) { return Arr::get($this->data, $parameters[0]); }
          public function validateConfirmed($attribute, $value, $parameters) { return $this->validateSame($attribute, $value, [$parameters[0] ?? $attribute.'_confirmation']); }
          public function validateRequiredWith($attribute, $value, $parameters) { return $this->anyFailingRequired($parameters); }
          public function anyFailingRequired($attributes) { foreach ($attributes as $key) { $this->getValue($key); } }
          public function validateMax($attribute, $value, $parameters) { return $parameters[0]; }
        }"#,
        );
        put(
            root,
            "app/Http/Requests/StoreRequest.php",
            r#"<?php namespace App\Http\Requests; use Illuminate\Foundation\Http\FormRequest;
        class StoreRequest extends FormRequest { public function rules(): array { return ['email' => 'required|email', 'password'=>'required', 'password_confirmation'=>'same:password', 'items.*.name'=>'required', 'profile.name'=>'required']; } }"#,
        );
        put(root, "app/Probe.php", "<?php");
        dir
    }
    fn run(
        root: &Path,
        path: &str,
        marked: &str,
        buffers: &[(PathBuf, &str)],
        completion: bool,
    ) -> Option<ResultSet> {
        let byte = marked.find('¦').unwrap();
        let source = marked.replacen('¦', "", 1);
        let (line, character) = position(&source, byte);
        query(
            root,
            &root.join(path),
            &source,
            line,
            character,
            &AtomicBool::new(false),
            buffers,
            completion,
        )
    }
    fn php(tail: &str) -> String {
        format!(
            "<?php use App\\Http\\Requests\\StoreRequest as R; function store(R $request) {{ {tail} }}"
        )
    }
    fn labels(result: ResultSet) -> Vec<String> {
        result.items.into_iter().map(|i| i.label).collect()
    }
    #[test]
    fn form_request_fields_complete_only_for_proven_receivers() {
        let dir = fixture();
        for tail in [
            "$request->input('em¦');",
            "$request->query('em¦');",
            "$request->post('em¦');",
            "$request->only(['em¦']);",
            "$request->only('em¦', 'password');",
            "$request->except(['em¦']);",
            "$request->validated('em¦');",
            "$request->validated()['em¦'];",
            "$request->safe()->only(['em¦']);",
            "$request['em¦'];",
            "$request->em¦ail;",
        ] {
            let result = run(dir.path(), "app/Probe.php", &php(tail), &[], true)
                .unwrap_or_else(|| panic!("{tail}"));
            assert_eq!(labels(result), ["email"], "{tail}");
        }
        for code in [
            php("$other->input('em¦');"),
            "<?php function store($request) { $request->input('em¦'); }".into(),
            php("$request = new Unknown; $request->input('em¦');"),
            php("$request->input('x', 'em¦');"),
            php("$request->input(name: 'em¦');"),
            php("// $request->input('em¦');\n"),
            php("$s = \"$request->input('em¦')\";"),
            php("$request->input(\"$x.em¦\");"),
            php("$request->input('em\\x¦');"),
            php("fn() => $request->input('em¦');"),
        ] {
            assert!(
                run(dir.path(), "app/Probe.php", &code, &[], true).is_none(),
                "{code}"
            );
        }
    }
    #[test]
    fn fields_keep_dots_wildcards_suffix_and_utf16_ranges() {
        let dir = fixture();
        let code = php("echo '🌧';\r\n$request->input('items.*.na¦unfinished');");
        let item = run(dir.path(), "app/Probe.php", &code, &[], true)
            .unwrap()
            .items
            .remove(0);
        assert_eq!(item.label, "items.*.name");
        let edit = item.text_edit.unwrap();
        let source = code.replace('¦', "");
        let a = crate::utf16_byte_offset(&source, edit.start_line, edit.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, edit.end_line, edit.end_character).unwrap();
        assert_eq!(&source[a..b], "items.*.naunfinished");
        assert_eq!(
            format!("{}{}{}", &source[..a], edit.new_text, &source[b..]),
            source.replace("items.*.naunfinished", "items.*.name")
        );
        assert!(
            !labels(
                run(
                    dir.path(),
                    "app/Probe.php",
                    &php("$request->pr¦ofile;"),
                    &[],
                    true
                )
                .unwrap()
            )
            .contains(&"profile.name".into())
        );
    }
    #[test]
    fn definitions_target_exact_rules_keys_and_dirty_buffers() {
        let dir = fixture();
        let path = dir
            .path()
            .join("app/Http/Requests/StoreRequest.php")
            .canonicalize()
            .unwrap();
        let result = run(
            dir.path(),
            "app/Probe.php",
            &php("$request->input('em¦ail');"),
            &[],
            false,
        )
        .unwrap();
        assert_eq!(result.targets.len(), 1);
        let t = &result.targets[0];
        assert_eq!(t.path, path);
        let source = fs::read_to_string(&path).unwrap();
        let a = crate::utf16_byte_offset(&source, t.start_line, t.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, t.end_line, t.end_character).unwrap();
        assert_eq!(&source[a..b], "email");
        let dirty = "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest { public function rules() { return ['dirty'=>'required']; } }";
        assert_eq!(
            labels(
                run(
                    dir.path(),
                    "app/Probe.php",
                    &php("$request->input('¦');"),
                    &[(path.clone(), dirty)],
                    true
                )
                .unwrap()
            ),
            ["dirty"]
        );
        assert!(fs::read_to_string(&path).unwrap().contains("email"));
        let composer = dir.path().join("composer.json").canonicalize().unwrap();
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                &php("$request->input('¦');"),
                &[(composer, "{}")],
                true
            )
            .is_none()
        );
    }
    #[test]
    fn rules_catalog_uses_framework_source_and_extensions() {
        let dir = fixture();
        put(
            dir.path(),
            "app/Providers/ValidationProvider.php",
            "<?php namespace App\\Providers; use Illuminate\\Support\\Facades\\Validator as V; class ValidationProvider { function boot() { V::extend('custom_email', fn()=>true); } }",
        );
        let code = "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data, ['email'=>'required|cod¦unfinished']);";
        assert_eq!(
            labels(run(dir.path(), "app/Probe.php", code, &[], true).unwrap()),
            ["codisseu_fixture"]
        );
        let result = run(
            dir.path(),
            "app/Probe.php",
            &code.replace("cod¦unfinished", "custom_¦"),
            &[],
            true,
        )
        .unwrap();
        assert_eq!(labels(result), ["custom_email"]);
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                "<?php Other::make($data,['email'=>'req¦']);",
                &[],
                true
            )
            .is_none()
        );
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                "<?php $data = ['email'=>'req¦'];",
                &[],
                true
            )
            .is_none()
        );
        assert!(run(dir.path(),"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['em¦ail'=>'required']);",&[],true).is_none());
    }
    #[test]
    fn form_request_rule_values_and_same_file_this_use_current_source() {
        let dir = fixture();
        let code = "<?php namespace App\\Http\\Requests; use Illuminate\\Foundation\\Http\\FormRequest; class StoreRequest extends FormRequest { public function rules() { return ['email'=>'req¦unfinished']; } }";
        assert_eq!(
            labels(
                run(
                    dir.path(),
                    "app/Http/Requests/StoreRequest.php",
                    code,
                    &[],
                    true
                )
                .unwrap()
            ),
            ["required", "required_if", "required_with"]
        );
        let code = "<?php namespace App\\Http\\Requests; use Illuminate\\Foundation\\Http\\FormRequest; class StoreRequest extends FormRequest { public function rules() { return ['dirty'=>'required']; } function foo() { return $this->input('di¦'); } }";
        assert_eq!(
            labels(
                run(
                    dir.path(),
                    "app/Http/Requests/StoreRequest.php",
                    code,
                    &[],
                    true
                )
                .unwrap()
            ),
            ["dirty"]
        );
    }
    #[test]
    fn dependent_parameters_come_from_framework_field_accesses() {
        let dir = fixture();
        for rules in [
            "same:pass¦unfinished",
            "confirmed:pass¦unfinished",
            "required_if:pass¦unfinished,x",
            "required_with:email,pass¦unfinished",
        ] {
            let code = format!(
                "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'required', 'password'=>'required', 'other'=>'{rules}']);"
            );
            assert_eq!(
                labels(run(dir.path(), "app/Probe.php", &code, &[], true).unwrap()),
                ["password"],
                "{rules}"
            );
        }
        for rules in ["required_if:email,pass¦", "max:pass¦"] {
            let code = format!(
                "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['password'=>'required','other'=>'{rules}']);"
            );
            assert!(
                run(dir.path(), "app/Probe.php", &code, &[], true).is_none(),
                "{rules}"
            );
        }
    }
    #[test]
    fn inline_validation_fields_belong_to_exact_local_receiver() {
        let dir = fixture();
        let code = "<?php use Illuminate\\Http\\Request; function store(Request $request, Request $other) { $other->validate(['wrong'=>'required']); $request->validate(['right'=>'required']); $request->input('¦'); }";
        assert_eq!(
            labels(run(dir.path(), "app/Probe.php", code, &[], true).unwrap()),
            ["right"]
        );
        let code = "<?php use Illuminate\\Http\\Request; function store(Request $request) { function nested(Request $request) { $request->validate(['wrong'=>'required']); } $request->input('¦'); }";
        assert!(labels(run(dir.path(), "app/Probe.php", code, &[], true).unwrap()).is_empty());
    }
    #[test]
    fn dynamic_rules_traits_and_missing_framework_do_not_guess() {
        let dir = fixture();
        let path = dir
            .path()
            .join("app/Http/Requests/StoreRequest.php")
            .canonicalize()
            .unwrap();
        for body in [
            "public function rules() { return array_merge(parent::rules(), ['guess'=>'required']); }",
            "use UnknownTrait; public function rules() { return ['guess'=>'required']; }",
            "public function rules() { if ($x) return ['guess'=>'required']; return ['other'=>'required']; }",
        ] {
            let source = format!(
                "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest {{ {body} }}"
            );
            assert!(
                run(
                    dir.path(),
                    "app/Probe.php",
                    &php("$request->input('¦');"),
                    &[(path.clone(), &source)],
                    true
                )
                .is_none(),
                "{body}"
            );
        }
        fs::remove_file(dir.path().join("vendor/composer/installed.json")).unwrap();
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                &php("$request->input('¦');"),
                &[],
                true
            )
            .is_none()
        );
    }
    #[test]
    fn cancelled_queries_and_result_bounds_are_enforced() {
        let dir = fixture();
        assert!(
            query(
                dir.path(),
                &dir.path().join("app/Probe.php"),
                "<?php",
                0,
                5,
                &AtomicBool::new(true),
                &[],
                true
            )
            .is_none()
        );
        let fields = (0..140)
            .map(|i| format!("'field{i:03}'=>'required'"))
            .collect::<Vec<_>>()
            .join(",");
        let source = format!(
            "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest {{ function rules() {{ return [{fields}]; }} }}"
        );
        let path = dir
            .path()
            .join("app/Http/Requests/StoreRequest.php")
            .canonicalize()
            .unwrap();
        let result = run(
            dir.path(),
            "app/Probe.php",
            &php("$request->input('¦');"),
            &[(path, &source)],
            true,
        )
        .unwrap();
        assert_eq!(result.items.len(), 128);
        assert!(result.truncated);
    }
    #[test]
    fn declared_properties_inheritance_and_traits_preserve_semantic_provider() {
        let dir = fixture();
        let path = dir
            .path()
            .join("app/Http/Requests/StoreRequest.php")
            .canonicalize()
            .unwrap();
        for body in [
            "public $email; public function rules() { return ['email'=>'required','extra'=>'required']; }",
            "public function __construct(public string $email) {} public function rules() { return ['email'=>'required']; }",
        ] {
            let source = format!(
                "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest {{ {body} }}"
            );
            assert!(
                run(
                    dir.path(),
                    "app/Probe.php",
                    &php("$request->em¦ail;"),
                    &[(path.clone(), &source)],
                    false
                )
                .is_none()
            );
            assert!(
                labels(
                    run(
                        dir.path(),
                        "app/Probe.php",
                        &php("$request->em¦;"),
                        &[(path.clone(), &source)],
                        true
                    )
                    .unwrap()
                )
                .is_empty()
            );
            // Declared properties don't change the semantics of input('email').
            assert_eq!(
                labels(
                    run(
                        dir.path(),
                        "app/Probe.php",
                        &php("$request->input('em¦');"),
                        &[(path.clone(), &source)],
                        true
                    )
                    .unwrap()
                ),
                ["email"]
            );
        }
        put(
            dir.path(),
            "app/Http/Requests/BaseRequest.php",
            "<?php namespace App\\Http\\Requests; class BaseRequest extends \\Illuminate\\Foundation\\Http\\FormRequest { public $email; }",
        );
        let inherited = "<?php namespace App\\Http\\Requests; class StoreRequest extends BaseRequest { function rules() { return ['email'=>'required']; } }";
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                &php("$request->em¦ail;"),
                &[(path.clone(), inherited)],
                false
            )
            .is_none()
        );
        let using_trait = "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest { use FieldTrait; function rules() { return ['email'=>'required']; } }";
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                &php("$request->em¦ail;"),
                &[(path, using_trait)],
                false
            )
            .is_none()
        );
        let code = php("$request->em¦ail;");
        let byte = code.find('¦').unwrap();
        let source = code.replace('¦', "");
        let (line, character) = position(&source, byte);
        assert!(property_context(&source, line, character));
    }
    #[test]
    fn non_alias_import_ui_context_resolves() {
        let dir = fixture();
        let result=run(dir.path(),"app/Probe.php","<?php use App\\Http\\Requests\\StoreRequest; function save(StoreRequest $request) { $request->input('em¦OLD'); }",&[],true).unwrap();
        assert_eq!(labels(result), ["email"]);
    }
    #[test]
    fn unfinished_literals_and_empty_member_names_recover_without_writing() {
        let dir = fixture();
        for code in [php("$request->¦;"), "<?php use App\\Http\\Requests\\StoreRequest as R; function store(R $request) { $request->input('em¦".into()] {
            let result = run(dir.path(),"app/Probe.php",&code,&[],true).unwrap_or_else(|| panic!("{code}"));
            assert!(labels(result).contains(&"email".into()),"{code}");
        }
        let code =
            "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'req¦";
        assert!(
            labels(run(dir.path(), "app/Probe.php", code, &[], true).unwrap())
                .contains(&"required".into())
        );
        let code = "<?php namespace App\\Http\\Requests; class StoreRequest extends \\Illuminate\\Foundation\\Http\\FormRequest { public function rules() { return ['email'=>'req¦";
        assert!(
            labels(
                run(
                    dir.path(),
                    "app/Http/Requests/StoreRequest.php",
                    code,
                    &[],
                    true
                )
                .unwrap()
            )
            .contains(&"required".into())
        );
    }
    #[test]
    fn custom_rule_classes_need_real_validation_contract_and_full_replacement() {
        let dir = fixture();
        put(
            dir.path(),
            "vendor/laravel/framework/src/Illuminate/Contracts/Validation/ValidationRule.php",
            "<?php namespace Illuminate\\Contracts\\Validation; interface ValidationRule { public function validate($a,$v,$f); }",
        );
        let path = put(
            dir.path(),
            "app/Rules/CompanyEmail.php",
            "<?php namespace App\\Rules; use Illuminate\\Contracts\\Validation\\ValidationRule; class CompanyEmail implements ValidationRule { public function validate($a,$v,$f) {} }",
        );
        put(
            dir.path(),
            "app/Rules/CompanyOther.php",
            "<?php namespace App\\Rules; class CompanyOther {}",
        );
        put(
            dir.path(),
            "app/Rules/CompanyAbstract.php",
            "<?php namespace App\\Rules; abstract class CompanyAbstract implements \\Illuminate\\Contracts\\Validation\\Rule {}",
        );
        let code = "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>['required',new Company¦Unfinished]]);";
        let result = run(dir.path(), "app/Probe.php", code, &[], true).unwrap();
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert_eq!(item.new_text, "\\App\\Rules\\CompanyEmail");
        let edit = item.text_edit.as_ref().unwrap();
        let source = code.replace('¦', "");
        let a = crate::utf16_byte_offset(&source, edit.start_line, edit.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, edit.end_line, edit.end_character).unwrap();
        assert_eq!(&source[a..b], "CompanyUnfinished");
        let definition=run(dir.path(),"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; use App\\Rules\\CompanyEmail as E; V::make($data,['email'=>[new E¦]]);",&[],false).unwrap();
        assert_eq!(definition.targets.len(), 1);
        assert_eq!(definition.targets[0].path, path);
        assert!(
            run(
                dir.path(),
                "app/Probe.php",
                "<?php $x = new Company¦;",
                &[],
                true
            )
            .is_none()
        );
        assert!(run(dir.path(),"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>[new CompanyEmail('req¦')]]);",&[],true).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn provider_directory_symlink_ancestors_cannot_enumerate_outside_workspace() {
        use std::os::unix::fs::symlink;
        let dir = fixture();
        let outside = tempfile::tempdir().unwrap();
        put(outside.path(), "Providers/Unsafe.php", "<?php");
        fs::remove_dir_all(dir.path().join("app")).unwrap();
        symlink(outside.path(), dir.path().join("app")).unwrap();
        let cancel = AtomicBool::new(false);
        let budget = Budget {
            root: dir.path(),
            buffers: &[],
            cancel: &cancel,
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        let mut files = Vec::new();
        let mut visited = 0;
        let mut truncated = false;
        provider_files(
            &budget,
            &dir.path().join("app/Providers"),
            0,
            &mut visited,
            &mut files,
            &mut truncated,
        );
        assert!(files.is_empty());
        assert_eq!(visited, 0);
        assert!(!truncated);
    }
    #[test]
    fn regex_array_values_are_not_split_into_piped_rules() {
        let dir = fixture();
        assert!(run(dir.path(),"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>['regex:/first|req¦/']]);",&[],true).is_none());
    }
    #[test]
    fn completion_rule_range_keeps_other_rules_and_parameters() {
        let dir = fixture();
        let code = "<?php use Illuminate\\Support\\Facades\\Validator as V; echo '🌧';\r\nV::make($data,['email'=>'required|req¦unfinished:password,value|email']);";
        let result = run(dir.path(), "app/Probe.php", code, &[], true).unwrap();
        let item = result
            .items
            .iter()
            .find(|i| i.label == "required_if")
            .unwrap();
        let edit = item.text_edit.as_ref().unwrap();
        let source = code.replace('¦', "");
        let a = crate::utf16_byte_offset(&source, edit.start_line, edit.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, edit.end_line, edit.end_character).unwrap();
        assert_eq!(&source[a..b], "requnfinished");
        assert!(
            format!("{}{}{}", &source[..a], edit.new_text, &source[b..])
                .ends_with("'required|required_if:password,value|email']);")
        );
    }
    #[test]
    #[ignore = "explicit local installed framework fixture; CODISSEU_LARAVEL_REQUEST_REAL_ROOT"]
    fn installed_framework_rules_and_dependent_parameters_are_source_derived() {
        let root = PathBuf::from(
            std::env::var_os("CODISSEU_LARAVEL_REQUEST_REAL_ROOT").expect("explicit source root"),
        )
        .canonicalize()
        .unwrap();
        for code in [
            "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'email', 'password'=>'required', 'field'=>'same:pass¦']);",
            "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'email', 'password'=>'required', 'field'=>'required_if:pass¦,x']);",
            "<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'email', 'password'=>'required', 'field'=>'required_with:email,pass¦']);",
        ] {
            assert_eq!(
                labels(run(&root, "app/Probe.php", code, &[], true).unwrap()),
                ["password"]
            );
        }
        let result=run(&root,"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'em¦']);",&[],false).unwrap();
        assert_eq!(result.targets.len(), 0);
        let result=run(&root,"app/Probe.php","<?php use Illuminate\\Support\\Facades\\Validator as V; V::make($data,['email'=>'ema¦il']);",&[],false).unwrap();
        assert_eq!(result.targets.len(), 1);
        assert!(
            result.targets[0]
                .path
                .ends_with("Validation/Concerns/ValidatesAttributes.php")
        );
    }
}
