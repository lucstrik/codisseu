//! Bounded Eloquent inference from installed declarations and unsaved snapshots.
//! This is independent source analysis: PHP, its autoloader and the database are
//! never started. Unproved receivers and return values do not acquire a type.
use super::{BUILDER, MODEL, QUERY, maps, resources::ResultSet};
use crate::php_project::{Budget, ComposerProject, Map, Names, children, locate, text};
use crate::{CompletionEdit, CompletionItem, DefinitionTarget};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};
use tree_sitter::Node;
const SCOPE: &str = "Illuminate\\Database\\Eloquent\\Attributes\\Scope";
const USE_BUILDER: &str = "Illuminate\\Database\\Eloquent\\Attributes\\UseEloquentBuilder";
#[derive(Clone, Copy, PartialEq)]
enum Return {
    Fluent,
    Model,
    GenericValue,
    Other,
}
#[derive(Clone)]
struct Member {
    name: String,
    target: DefinitionTarget,
    kind: u32,
    returns: Return,
    related: Option<(String, bool)>,
    attribute_scope: bool,
    scope_chain: Vec<String>,
}
#[derive(Clone, Default)]
struct Info {
    parent: Option<String>,
    traits: Vec<String>,
    methods: Vec<Member>,
    scopes: Vec<Member>,
    fields: Vec<Member>,
    declared_properties: HashSet<String>,
    builder: Option<String>,
    builder_attribute: bool,
    builder_property: bool,
    builder_override: bool,
    factory_override: bool,
    unsafe_hooks: bool,
    source: String,
}
fn target(path: &Path, source: &str, n: Node<'_>) -> DefinitionTarget {
    let column = |byte: usize| {
        let line = source[..byte].rfind('\n').map_or(0, |p| p + 1);
        source[line..byte].encode_utf16().count() as u32
    };
    let (start_line, start_character) = (n.start_position().row as u32, column(n.start_byte()));
    let (end_line, end_character) = (n.end_position().row as u32, column(n.end_byte()));
    DefinitionTarget {
        path: path.into(),
        start_line,
        start_character,
        end_line,
        end_character,
    }
}
fn walk<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        out.push(n);
        stack.extend(children(n).into_iter().rev());
    }
}
fn descendants(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}
fn literal(node: Node<'_>, source: &str) -> Option<String> {
    let s = text(node, source);
    if !matches!(node.kind(), "string" | "encapsed_string") || s.len() < 2 {
        return None;
    }
    let v = &s[1..s.len() - 1];
    (!v.contains(['\\', '$'])).then(|| v.to_owned())
}
fn class_constant(n: Node<'_>, source: &str, names: &Names) -> Option<String> {
    if n.kind() != "class_constant_access_expression" {
        return None;
    }
    let c = children(n);
    if c.len() != 2 || !text(c[1], source).eq_ignore_ascii_case("class") {
        return None;
    }
    names.resolve(text(c[0], source))
}
fn attributes(node: Node<'_>, source: &str, names: &Names) -> Vec<(String, Option<String>)> {
    children(node)
        .into_iter()
        .filter(|n| n.kind() == "attribute_list")
        .flat_map(descendants)
        .filter(|n| n.kind() == "attribute")
        .filter_map(|n| {
            let name = n.child_by_field_name("name").or_else(|| {
                children(n)
                    .into_iter()
                    .find(|n| matches!(n.kind(), "name" | "qualified_name"))
            })?;
            let value = descendants(n)
                .into_iter()
                .find_map(|n| class_constant(n, source, names));
            Some((names.resolve(text(name, source))?, value))
        })
        .collect()
}
fn return_kind(method: Node<'_>, source: &str) -> Return {
    if let Some(t) = method.child_by_field_name("return_type")
        && matches!(text(t, source), "static" | "self")
    {
        return Return::Fluent;
    }
    let doc = method
        .prev_named_sibling()
        .filter(|n| n.kind() == "comment")
        .map(|n| text(n, source))
        .unwrap_or("");
    for line in doc.lines() {
        if let Some((_, tail)) = line.split_once("@return ") {
            let ty = tail.split_whitespace().next().unwrap_or("");
            if matches!(
                ty,
                "$this" | "static" | "self" | "$this|null" | "static|null" | "self|null"
            ) {
                return Return::Fluent;
            }
            if matches!(ty, "TModel" | "TModel|null") {
                return Return::Model;
            }
            if matches!(ty, "TValue" | "TValue|null") {
                return Return::GenericValue;
            }
        }
    }
    let returns = descendants(method)
        .into_iter()
        .filter(|n| n.kind() == "return_statement")
        .collect::<Vec<_>>();
    if !returns.is_empty()
        && returns.iter().all(|n| {
            children(*n)
                .first()
                .is_some_and(|n| text(*n, source) == "$this")
        })
    {
        return Return::Fluent;
    }
    Return::Other
}
fn scope_return(method: Node<'_>, source: &str) -> (Return, Vec<String>) {
    let parameter = method
        .child_by_field_name("parameters")
        .and_then(|p| children(p).first().copied())
        .and_then(|p| p.child_by_field_name("name"))
        .map(|n| text(n, source));
    let mut chain = Vec::new();
    if descendants(method).into_iter().any(|n| {
        matches!(
            n.kind(),
            "assignment_expression"
                | "reference_assignment_expression"
                | "augmented_assignment_expression"
        ) && n
            .child_by_field_name("left")
            .is_some_and(|left| Some(text(left, source)) == parameter)
    }) {
        return (Return::Other, chain);
    }
    for ret in descendants(method)
        .into_iter()
        .filter(|n| n.kind() == "return_statement")
    {
        let Some(mut value) = children(ret).first().copied() else {
            continue;
        };
        if value.kind() == "null" {
            continue;
        }
        while value.kind() == "member_call_expression" {
            let Some(name) = value.child_by_field_name("name") else {
                return (Return::Other, Vec::new());
            };
            chain.push(text(name, source).to_owned());
            let Some(object) = value.child_by_field_name("object") else {
                return (Return::Other, Vec::new());
            };
            value = object;
        }
        if Some(text(value, source)) != parameter {
            return (Return::Other, Vec::new());
        }
    }
    (Return::Fluent, chain)
}
fn inspect(budget: &Budget<'_>, path: &Path, source: &str, name: &str) -> Option<Info> {
    let tree = budget.tree(source)?;
    let names = Names::parse(&tree, source)?;
    let node = children(tree.root_node()).into_iter().find(|n| {
        matches!(n.kind(), "class_declaration" | "trait_declaration")
            && n.child_by_field_name("name")
                .is_some_and(|n| names.resolve(text(n, source)).as_deref() == Some(name))
    })?;
    if node.has_error() {
        return None;
    }
    let mut info = Info {
        source: source.into(),
        ..Info::default()
    };
    for n in children(node) {
        if n.kind() == "base_clause" {
            let c = children(n);
            if c.len() != 1 {
                return None;
            }
            info.parent = names.resolve(text(c[0], source));
        }
    }
    for (attr, builder) in attributes(node, source, &names) {
        if attr == USE_BUILDER {
            info.builder = Some(builder?);
            info.builder_attribute = true;
        }
    }
    for member in children(node.child_by_field_name("body")?) {
        if member.kind() == "use_declaration" {
            for t in children(member) {
                if matches!(t.kind(), "name" | "qualified_name") {
                    info.traits.push(names.resolve(text(t, source))?);
                } else if !name.starts_with("Illuminate\\") {
                    // User trait adaptations need exact alias/visibility handling.
                    return None;
                }
            }
        }
        if member.kind() == "property_declaration" {
            for p in children(member)
                .into_iter()
                .filter(|p| p.kind() == "property_element")
            {
                let property = text(p.child_by_field_name("name")?, source);
                info.declared_properties
                    .insert(property.trim_start_matches('$').to_owned());
                let value = p.child_by_field_name("default_value").or_else(|| {
                    children(p).into_iter().find(|n| {
                        matches!(
                            n.kind(),
                            "array_creation_expression" | "class_constant_access_expression"
                        )
                    })
                });
                if property == "$builder" && !info.builder_attribute && !info.builder_override {
                    info.builder = Some(class_constant(value?, source, &names)?);
                    info.builder_property = true;
                }
                if matches!(property, "$fillable" | "$casts")
                    && let Some(array) = value.filter(|n| n.kind() == "array_creation_expression")
                {
                    for e in children(array)
                        .into_iter()
                        .filter(|e| e.kind() == "array_element_initializer")
                    {
                        let values = children(e);
                        let Some(n) = (if property == "$fillable" {
                            values.last()
                        } else {
                            values.first()
                        })
                        .copied() else {
                            continue;
                        };
                        if let Some(field) = literal(n, source) {
                            info.fields.push(Member {
                                name: field,
                                target: target(path, source, n),
                                kind: 10,
                                returns: Return::Other,
                                related: None,
                                attribute_scope: false,
                                scope_chain: Vec::new(),
                            });
                        }
                    }
                }
            }
        }
        if member.kind() != "method_declaration" {
            continue;
        }
        let n = member.child_by_field_name("name")?;
        let method = text(n, source).to_owned();
        if matches!(
            method.as_str(),
            "belongsTo" | "hasOne" | "morphOne" | "hasMany" | "belongsToMany" | "morphMany"
        ) {
            info.factory_override = true;
        }
        if method == "casts" {
            let returns = descendants(member)
                .into_iter()
                .filter(|n| n.kind() == "return_statement")
                .collect::<Vec<_>>();
            if returns.len() == 1
                && let Some(array) = children(returns[0])
                    .first()
                    .copied()
                    .filter(|n| n.kind() == "array_creation_expression")
            {
                for element in children(array)
                    .into_iter()
                    .filter(|n| n.kind() == "array_element_initializer")
                {
                    if let Some(key) = children(element).first().copied()
                        && let Some(field) = literal(key, source)
                    {
                        info.fields.push(Member {
                            name: field,
                            target: target(path, source, key),
                            kind: 10,
                            returns: Return::Other,
                            related: None,
                            attribute_scope: false,
                            scope_chain: Vec::new(),
                        });
                    }
                }
            }
        }
        let mods = children(member);
        let private = mods
            .iter()
            .any(|n| n.kind() == "visibility_modifier" && text(*n, source) == "private");
        let public = !mods
            .iter()
            .any(|n| n.kind() == "visibility_modifier" && text(*n, source) != "public");
        let mut entry = Member {
            name: method.clone(),
            target: target(path, source, n),
            kind: 2,
            returns: return_kind(member, source),
            related: None,
            attribute_scope: false,
            scope_chain: Vec::new(),
        };
        if name.starts_with("Illuminate\\") {
            if public {
                info.methods.push(entry);
            }
            continue;
        }
        if matches!(entry.returns, Return::Model | Return::GenericValue) {
            entry.returns = Return::Other;
        }
        if public && method.starts_with("scope") && method.len() > 5 {
            let mut chars = method[5..].chars();
            let first = chars.next()?;
            entry.name = format!("{}{}", first.to_lowercase(), chars.as_str());
            (entry.returns, entry.scope_chain) = scope_return(member, source);
            info.scopes.push(entry.clone());
        } else if !private
            && attributes(member, source, &names)
                .iter()
                .any(|(name, _)| name == SCOPE)
        {
            (entry.returns, entry.scope_chain) = scope_return(member, source);
            entry.attribute_scope = true;
            info.scopes.push(entry.clone());
        }
        entry.name = method.clone();
        if method == "newEloquentBuilder" && name != MODEL {
            let declared = member
                .child_by_field_name("return_type")
                .and_then(|n| names.resolve(text(n, source)));
            let returned = descendants(member)
                .into_iter()
                .filter(|n| n.kind() == "return_statement")
                .collect::<Vec<_>>();
            let created = if returned.len() == 1 {
                children(returned[0])
                    .first()
                    .copied()
                    .filter(|n| n.kind() == "object_creation_expression")
                    .and_then(|n| {
                        children(n)
                            .into_iter()
                            .find(|n| matches!(n.kind(), "name" | "qualified_name"))
                    })
                    .and_then(|n| names.resolve(text(n, source)))
            } else {
                None
            };
            info.builder = Some(created.or(declared)?);
            info.builder_override = true;
        } else if matches!(
            method.as_str(),
            "query"
                | "newQuery"
                | "newModelQuery"
                | "newQueryWithoutScopes"
                | "__call"
                | "__callStatic"
                | "resolveCustomBuilderClass"
                | "forwardCallTo"
                | "forwardDecoratedCallTo"
        ) {
            info.unsafe_hooks = true;
        }
        // Relationship properties are proved only for a direct, single return
        // of a standard relationship factory with a literal Related::class.
        let returns = descendants(member)
            .into_iter()
            .filter(|n| n.kind() == "return_statement")
            .collect::<Vec<_>>();
        if public
            && returns.len() == 1
            && let Some(call) = children(returns[0])
                .first()
                .copied()
                .filter(|n| n.kind() == "member_call_expression")
        {
            let object = call.child_by_field_name("object")?;
            let factory = text(call.child_by_field_name("name")?, source);
            if text(object, source) == "$this"
                && matches!(
                    factory,
                    "belongsTo" | "hasOne" | "morphOne" | "hasMany" | "belongsToMany" | "morphMany"
                )
            {
                let related = call
                    .child_by_field_name("arguments")
                    .and_then(|a| children(a).first().copied())
                    .and_then(|a| children(a).first().copied())
                    .and_then(|n| class_constant(n, source, &names));
                if let Some(related) = related {
                    entry.related = Some((
                        related,
                        matches!(factory, "belongsTo" | "hasOne" | "morphOne"),
                    ));
                    let mut field = entry.clone();
                    field.kind = 10;
                    info.fields.push(field);
                }
            }
        }
        if public {
            info.methods.push(entry);
        }
    }
    Some(info)
}
struct Engine<'a> {
    budget: Budget<'a>,
    maps: Vec<Map>,
    cache: HashMap<String, Info>,
    framework: Info,
}
impl Engine<'_> {
    fn info(&mut self, name: &str) -> Option<Info> {
        self.budget.check()?;
        if let Some(i) = self.cache.get(name) {
            return Some(i.clone());
        }
        let (path, source) = locate(&mut self.budget, &self.maps, name)?;
        let i = inspect(&self.budget, &path, &source, name)?;
        self.cache.insert(name.into(), i.clone());
        Some(i)
    }
    fn family(&mut self, name: &str, base: &str) -> Option<Vec<Info>> {
        let mut out = Vec::new();
        let mut queue = vec![name.to_owned()];
        let mut seen = HashSet::new();
        let mut reached = false;
        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if seen.len() > 32 {
                return None;
            }
            if name == base {
                reached = true;
                continue;
            }
            let info = self.info(&name)?;
            if let Some(p) = &info.parent {
                queue.push(p.clone());
            }
            queue.extend(info.traits.iter().rev().cloned());
            out.push(info);
        }
        reached.then_some(out)
    }
    fn model(&mut self, name: &str) -> Option<Vec<Info>> {
        let mut family = self.family(name, MODEL)?;
        // A real PHP property wins over Eloquent's magic field/relation lookup.
        // Include traits and ancestors, conservatively even when visibility
        // would require a more detailed caller-scope analysis.
        let properties: HashSet<_> = family
            .iter()
            .flat_map(|i| i.declared_properties.iter().cloned())
            .collect();
        for info in &mut family {
            info.fields.retain(|m| !properties.contains(&m.name));
        }
        if family.iter().any(|i| i.factory_override) {
            for info in &mut family {
                info.fields.retain(|m| m.related.is_none());
                for m in &mut info.methods {
                    m.related = None;
                }
            }
        }
        if family.iter().any(|i| i.unsafe_hooks) {
            return None;
        }
        Some(family)
    }
    fn builder(&mut self, family: &[Info]) -> Option<String> {
        if family.iter().any(|i| {
            (i.builder_attribute && !self.framework.source.contains("resolveCustomBuilderClass"))
                || (i.builder_property && !self.framework.source.contains("static::$builder"))
        }) {
            return None;
        }
        let builder = family
            .iter()
            .find(|i| i.builder_override)
            .and_then(|i| i.builder.clone())
            .or_else(|| {
                family
                    .first()
                    .filter(|i| i.builder_attribute)
                    .and_then(|i| i.builder.clone())
            })
            .or_else(|| {
                family
                    .iter()
                    .find(|i| i.builder_property)
                    .and_then(|i| i.builder.clone())
            })
            .unwrap_or_else(|| BUILDER.into());
        if builder != BUILDER
            && self
                .family(&builder, BUILDER)?
                .iter()
                .any(|info| info.unsafe_hooks)
        {
            return None;
        }
        Some(builder)
    }
    fn methods(&mut self, model: &str) -> Option<Vec<Member>> {
        let family = self.model(model)?;
        let builder = self.builder(&family)?;
        let mut result: Vec<Member> = Vec::new();
        let mut queue = vec![QUERY.into(), BUILDER.into(), builder];
        let mut seen = HashSet::new();
        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            if seen.len() > 32 {
                return None;
            }
            let info = self.info(&name)?;
            if let Some(p) = info.parent {
                queue.push(p);
            }
            queue.extend(info.traits.into_iter().rev());
            let value_is_model = name == "Illuminate\\Database\\Concerns\\BuildsQueries"
                && info.source.contains("@template TValue")
                && self.cache.get(BUILDER).is_some_and(|i| {
                    i.source
                        .contains("@use \\Illuminate\\Database\\Concerns\\BuildsQueries<TModel>")
                });
            for mut m in info.methods {
                if m.returns == Return::GenericValue {
                    m.returns = if value_is_model {
                        Return::Model
                    } else {
                        Return::Other
                    };
                }
                if !m.name.starts_with('_')
                    && !result.iter().any(|v| v.name.eq_ignore_ascii_case(&m.name))
                {
                    result.push(m);
                }
            }
        }
        if self.framework.source.contains("function hasNamedScope") {
            for i in &family {
                for scope in &i.scopes {
                    // Modern attributed scopes need support in the installed Model.
                    if scope.attribute_scope
                        && !self.framework.source.contains("isScopeMethodWithAttribute")
                    {
                        continue;
                    }
                    if !result
                        .iter()
                        .any(|m: &Member| m.name.eq_ignore_ascii_case(&scope.name))
                    {
                        let mut scope = scope.clone();
                        if !scope.scope_chain.iter().all(|name| {
                            result.iter().any(|m| {
                                m.name.eq_ignore_ascii_case(name) && m.returns == Return::Fluent
                            })
                        }) {
                            scope.returns = Return::Other;
                        }
                        result.push(scope);
                    }
                }
            }
        }
        Some(result)
    }
}
#[derive(Clone)]
struct Receiver {
    model: String,
    builder: bool,
}
fn infer(
    engine: &mut Engine<'_>,
    node: Node<'_>,
    source: &str,
    names: &Names,
    depth: usize,
) -> Option<Receiver> {
    if depth > 24 {
        return None;
    }
    engine.budget.check()?;
    match node.kind() {
        "object_creation_expression" => {
            let n = children(node)
                .into_iter()
                .find(|n| matches!(n.kind(), "name" | "qualified_name"))?;
            let model = names.resolve(text(n, source))?;
            engine.model(&model)?;
            Some(Receiver {
                model,
                builder: false,
            })
        }
        "scoped_call_expression" => {
            let model = names.resolve(text(node.child_by_field_name("scope")?, source))?;
            let family = engine.model(&model)?;
            let method = text(node.child_by_field_name("name")?, source);
            if family.iter().any(|i| {
                i.methods
                    .iter()
                    .any(|m| m.name.eq_ignore_ascii_case(method))
            }) {
                return None;
            }
            if matches!(method, "query" | "newQuery") {
                return Some(Receiver {
                    model,
                    builder: true,
                });
            }
            let entry = engine
                .methods(&model)?
                .into_iter()
                .find(|m| m.name.eq_ignore_ascii_case(method))?;
            match entry.returns {
                Return::Fluent => Some(Receiver {
                    model,
                    builder: true,
                }),
                Return::Model => Some(Receiver {
                    model,
                    builder: false,
                }),
                Return::Other | Return::GenericValue => None,
            }
        }
        "member_call_expression" => {
            let receiver = infer(
                engine,
                node.child_by_field_name("object")?,
                source,
                names,
                depth + 1,
            )?;
            let method = text(node.child_by_field_name("name")?, source);
            if !receiver.builder && matches!(method, "query" | "newQuery") {
                return Some(Receiver {
                    builder: true,
                    ..receiver
                });
            }
            let methods = if receiver.builder {
                engine.methods(&receiver.model)?
            } else {
                engine
                    .model(&receiver.model)?
                    .into_iter()
                    .flat_map(|i| i.methods)
                    .collect()
            };
            let entry = methods
                .into_iter()
                .find(|m| m.name.eq_ignore_ascii_case(method))?;
            if let Some((related, _)) = entry.related {
                engine.model(&related)?;
                return Some(Receiver {
                    model: related,
                    builder: true,
                });
            }
            match entry.returns {
                Return::Fluent => Some(receiver),
                Return::Model => Some(Receiver {
                    builder: false,
                    ..receiver
                }),
                Return::Other | Return::GenericValue => None,
            }
        }
        "member_access_expression" => {
            let receiver = infer(
                engine,
                node.child_by_field_name("object")?,
                source,
                names,
                depth + 1,
            )?;
            if receiver.builder {
                return None;
            }
            let field = text(node.child_by_field_name("name")?, source);
            let entry = engine
                .model(&receiver.model)?
                .into_iter()
                .flat_map(|i| i.fields)
                .find(|m| m.name == field)?;
            let (related, single) = entry.related?;
            if !single {
                return None;
            }
            engine.model(&related)?;
            Some(Receiver {
                model: related,
                builder: false,
            })
        }
        "variable_name" => {
            let variable = text(node, source);
            let mut parent = node.parent()?;
            loop {
                if matches!(
                    parent.kind(),
                    "function_definition"
                        | "method_declaration"
                        | "anonymous_function"
                        | "arrow_function"
                ) {
                    break;
                }
                if parent.parent().is_none() {
                    break;
                }
                parent = parent.parent()?;
            }
            if variable == "$this" {
                let mut c = parent;
                while c.kind() != "class_declaration" {
                    c = c.parent()?;
                }
                let model = names.resolve(text(c.child_by_field_name("name")?, source))?;
                engine.model(&model)?;
                return Some(Receiver {
                    model,
                    builder: false,
                });
            }
            // A typed parameter is local to the containing function. Reassignment
            // makes inference unsupported instead of reusing a stale type.
            let mut assignment = None;
            for n in descendants(parent) {
                if n.start_byte() >= node.start_byte() {
                    continue;
                }
                if matches!(
                    n.kind(),
                    "assignment_expression"
                        | "reference_assignment_expression"
                        | "augmented_assignment_expression"
                ) && n
                    .child_by_field_name("left")
                    .is_some_and(|n| text(n, source) == variable)
                {
                    if n.kind() != "assignment_expression" {
                        return None;
                    }
                    let mut scope = n;
                    while scope.parent().is_some()
                        && !matches!(
                            scope.kind(),
                            "function_definition"
                                | "method_declaration"
                                | "anonymous_function"
                                | "arrow_function"
                                | "program"
                        )
                    {
                        scope = scope.parent()?;
                    }
                    if scope != parent {
                        continue;
                    }
                    let statement = n.parent()?;
                    let block = statement.parent()?;
                    let direct = statement.kind() == "expression_statement"
                        && (block == parent
                            || (block.kind() == "compound_statement"
                                && block.parent() == Some(parent)));
                    if !direct {
                        return None;
                    }
                    assignment = Some(n);
                }
            }
            if let Some(assignment) = assignment {
                return infer(
                    engine,
                    assignment.child_by_field_name("right")?,
                    source,
                    names,
                    depth + 1,
                );
            }
            let params = parent.child_by_field_name("parameters")?;
            let param = children(params).into_iter().find(|n| {
                n.child_by_field_name("name")
                    .is_some_and(|n| text(n, source) == variable)
            })?;
            let model = names.resolve(text(param.child_by_field_name("type")?, source))?;
            engine.model(&model)?;
            Some(Receiver {
                model,
                builder: false,
            })
        }
        _ => None,
    }
}
/// Additional source-derived suggestions/targets. `None` means the context was
/// not proved; the caller must preserve its existing provider response.
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
        root: workspace,
        cancel,
        buffers,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    budget.check()?;
    let project = ComposerProject::discover(&mut budget, path)?;
    budget.root = &project.root;
    let map = maps(&mut budget)?;
    let mut token_end = byte;
    while source
        .get(token_end..)
        .and_then(|s| s.chars().next())
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        token_end += source[token_end..].chars().next()?.len_utf8();
    }
    let parsed = if completion {
        let tail = &source[token_end..];
        let suffix = if tail.trim_start().starts_with('(') {
            ""
        } else if tail.trim_start().starts_with(';') {
            "()"
        } else {
            "();"
        };
        format!("{}__codisseu_placeholder{suffix}{tail}", &source[..byte])
    } else {
        source.to_owned()
    };
    let tree = budget.tree(&parsed)?;
    let names = Names::parse(&tree, &parsed)?;
    let at = if completion {
        byte
    } else {
        byte.min(source.len().checked_sub(1)?)
    };
    let mut node = tree.root_node().descendant_for_byte_range(at, at)?;
    loop {
        if matches!(
            node.kind(),
            "scoped_call_expression" | "member_call_expression" | "member_access_expression"
        ) && node
            .child_by_field_name("name")
            .is_some_and(|n| n.start_byte() <= at && at <= n.end_byte())
        {
            break;
        }
        node = node.parent()?;
    }
    let name = node.child_by_field_name("name")?;
    let start = name.start_byte();
    let mut end = byte;
    while source
        .get(end..)
        .and_then(|s| s.chars().next())
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        end += source[end..].chars().next()?.len_utf8();
    }
    if !completion {
        end = name.end_byte();
    }
    let prefix = source.get(start..byte)?;
    let (framework_path, framework_source) = locate(&mut budget, &map, MODEL)?;
    let framework = inspect(&budget, &framework_path, &framework_source, MODEL)?;
    if !framework.source.contains("forwardCallTo") || !framework.source.contains("newQuery") {
        return None;
    }
    let mut engine = Engine {
        budget,
        maps: map,
        cache: HashMap::new(),
        framework,
    };
    let receiver = if node.kind() == "scoped_call_expression" {
        let model = names.resolve(text(node.child_by_field_name("scope")?, &parsed))?;
        engine.model(&model)?;
        Receiver {
            model,
            builder: true,
        }
    } else {
        infer(
            &mut engine,
            node.child_by_field_name("object")?,
            &parsed,
            &names,
            0,
        )?
    };
    let mut entries = if receiver.builder {
        engine.methods(&receiver.model)?
    } else {
        let family = engine.model(&receiver.model)?;
        if node.kind() == "member_access_expression" {
            family.into_iter().flat_map(|i| i.fields).collect()
        } else if completion {
            family
                .into_iter()
                .flat_map(|i| i.methods.into_iter().chain(i.fields))
                .collect()
        } else {
            family.into_iter().flat_map(|i| i.methods).collect()
        }
    };
    if node.kind() == "member_access_expression" && receiver.builder {
        return None;
    }
    if node.kind() == "scoped_call_expression" {
        let family = engine.model(&receiver.model)?;
        entries.retain(|entry| {
            !family.iter().any(|i| {
                i.methods
                    .iter()
                    .any(|m| m.name.eq_ignore_ascii_case(&entry.name))
            })
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries.dedup_by(|a, b| a.name == b.name);
    let matching: Vec<_> = entries
        .into_iter()
        .filter(|m| {
            if completion {
                m.name.starts_with(prefix)
            } else {
                m.name == text(name, &parsed)
            }
        })
        .collect();
    engine.budget.check()?;
    let truncated = matching.len() > 128;
    let mut items = Vec::new();
    let mut targets = Vec::new();
    for entry in matching.into_iter().take(128) {
        if completion {
            let position = |b: usize| {
                let s = &source[..b];
                (
                    s.bytes().filter(|b| *b == b'\n').count() as u32,
                    s.rsplit('\n').next().unwrap_or("").encode_utf16().count() as u32,
                )
            };
            let (start_line, start_character) = position(start);
            let (end_line, end_character) = position(end);
            items.push(CompletionItem {
                label: entry.name.clone(),
                detail: Some(format!(
                    "Eloquent · inferred from {}",
                    entry
                        .target
                        .path
                        .strip_prefix(workspace)
                        .unwrap_or(&entry.target.path)
                        .display()
                )),
                kind: Some(entry.kind),
                filter_text: None,
                sort_text: None,
                new_text: entry.name.clone(),
                text_edit: Some(CompletionEdit {
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    new_text: entry.name,
                }),
                insert_edit: None,
                additional_edits: Vec::new(),
                unsupported_reason: None,
            });
        } else {
            targets.push(entry.target);
        }
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
    pub(super) fn write(root: &Path, path: &str, source: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    pub(super) fn fixture() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(
            r,
            "composer.json",
            r#"{"autoload":{"psr-4":{"App\\":"app/"}}}"#,
        );
        write(
            r,
            "vendor/composer/installed.json",
            r#"{"packages":[{"name":"laravel/framework","autoload":{"psr-4":{"Illuminate\\":"src/Illuminate/"}},"install-path":"../laravel/framework"}]}"#,
        );
        write(r, "probe.php", "<?php");
        write(
            r,
            "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php",
            "<?php namespace Illuminate\\Database\\Eloquent; class Model { function __call($name,$args){return $this->forwardCallTo($this->newQuery(),$name,$args);} function hasNamedScope($name){return method_exists($this,'scope'.$name);} function isScopeMethodWithAttribute($name){} function resolveCustomBuilderClass(){} function newEloquentBuilder($q){return new static::$builder($q);} }",
        );
        write(
            r,
            "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Builder.php",
            "<?php namespace Illuminate\\Database\\Eloquent; class Builder { public function where($a,$b){return $this;} /** @return TModel|null */ public function first(){} public function get(){} }",
        );
        write(
            r,
            "vendor/laravel/framework/src/Illuminate/Database/Query/Builder.php",
            "<?php namespace Illuminate\\Database\\Query; class Builder { /** @return $this */ public function orderBy($name){} }",
        );
        write(
            r,
            "app/Models/User.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; use Illuminate\Database\Eloquent\Attributes\Scope; class User extends Model { protected $fillable=['name','email']; protected $casts=['enabled'=>'bool']; protected function casts(): array {return ['birthday'=>'date'];} public function scopeActive($query){return $query->where('active',true);} #[Scope] protected function popular($query){} private function scopeSecret($q){} public function author(){return $this->belongsTo(Author::class);} public function articles(){return $this->hasMany(Author::class);} }"#,
        );
        write(
            r,
            "app/Models/Author.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class Author extends Model { protected $fillable=['display_name']; public function scopeVerified($q){} }"#,
        );
        d
    }
    pub(super) fn ask(
        root: &Path,
        s: &str,
        completion: bool,
        buffers: &[(PathBuf, &str)],
    ) -> Option<ResultSet> {
        let b = s.find('|').unwrap();
        let source = s.replacen('|', "", 1);
        let prefix = &source[..b];
        query(
            root,
            &root.join("probe.php"),
            &source,
            prefix.bytes().filter(|b| *b == b'\n').count() as u32,
            prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
            &AtomicBool::new(false),
            buffers,
            completion,
        )
    }
    pub(super) fn labels(root: &Path, s: &str) -> Vec<String> {
        ask(root, s, true, &[])
            .map(|r| r.items.into_iter().map(|i| i.label).collect())
            .unwrap_or_default()
    }
    #[test]
    fn scopes_are_model_specific_and_chains_respect_terminal_returns() {
        let d = fixture();
        let r = d.path();
        for s in [
            "<?php use App\\Models\\User; User::act|",
            "<?php use App\\Models\\User; User::query()->where('x',1)->act|",
            "<?php use App\\Models\\User; User::active()->orderBy('id')->pop|",
        ] {
            let result = labels(r, s);
            assert!(!result.is_empty(), "{s}");
        }
        for input in [
            "<?php use App\\Models\\User; User::act|OLD();",
            "<?php use App\\Models\\User; User::act|OLD;",
        ] {
            let result = ask(r, input, true, &[]).unwrap();
            assert_eq!(result.items[0].label, "active");
            let edit = result.items[0].text_edit.as_ref().unwrap();
            assert_eq!(edit.end_character - edit.start_character, 6);
        }
        assert_eq!(
            labels(r, "<?php use App\\Models\\User; User::sec|"),
            Vec::<String>::new()
        );
        assert_eq!(
            labels(r, "<?php use App\\Models\\User; User::query()->get()->act|"),
            Vec::<String>::new()
        );
        assert_eq!(
            labels(r, "<?php use App\\Models\\Author; Author::act|"),
            Vec::<String>::new()
        );
        let result = ask(
            r,
            "<?php use App\\Models\\User; User::act|ive();",
            false,
            &[],
        )
        .unwrap();
        let t = &result.targets[0];
        let source = std::fs::read_to_string(&t.path).unwrap();
        let a = crate::utf16_byte_offset(&source, t.start_line, t.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, t.end_line, t.end_character).unwrap();
        assert_eq!(&source[a..b], "scopeActive");
    }
    #[test]
    fn instance_fields_relations_and_related_model_are_source_derived() {
        let d = fixture();
        let r = d.path();
        assert_eq!(
            labels(
                r,
                "<?php use App\\Models\\User; function show(User $user){ $user->em|; }"
            ),
            vec!["email"]
        );
        assert_eq!(
            labels(
                r,
                "<?php use App\\Models\\User; $user=new User; $user->ena|"
            ),
            vec!["enabled"]
        );
        assert_eq!(
            labels(
                r,
                "<?php use App\\Models\\User; $user=new User; $user->bir|"
            ),
            vec!["birthday"]
        );
        assert_eq!(
            labels(
                r,
                "<?php use App\\Models\\User; function show(User $user){ $user->author->dis|; }"
            ),
            vec!["display_name"]
        );
        assert_eq!(
            labels(
                r,
                "<?php use App\\Models\\User; function show(User $user){ $user->author()->ver|; }"
            ),
            vec!["verified"]
        );
        assert!(
            labels(
                r,
                "<?php use App\\Models\\User; function show(User $user){ $user->articles->dis|; }"
            )
            .is_empty()
        );
        assert!(labels(r, "<?php use App\\Models\\User; User::query()->em|").is_empty());
    }
    #[test]
    fn custom_builders_from_literal_property_return_and_attribute() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "app/CustomBuilder.php",
            r#"<?php namespace App; use Illuminate\Database\Eloquent\Builder; class CustomBuilder extends Builder { public function nearby(): static {return $this;} protected function secret(){} }"#,
        );
        for declaration in [
            "protected static $builder=\\App\\CustomBuilder::class;",
            "public function newEloquentBuilder($query): \\App\\CustomBuilder { return new \\App\\CustomBuilder($query); }",
        ] {
            write(
                r,
                "app/Models/User.php",
                &format!(
                    "<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; class User extends Model {{{declaration}}}"
                ),
            );
            assert_eq!(
                labels(r, "<?php use App\\Models\\User; User::nea|"),
                vec!["nearby"],
                "{declaration}"
            );
            assert_eq!(
                labels(r, "<?php use App\\Models\\User; User::nearby()->whe|"),
                vec!["where"]
            );
        }
        write(
            r,
            "app/Models/User.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; use Illuminate\Database\Eloquent\Attributes\UseEloquentBuilder; #[UseEloquentBuilder(\App\CustomBuilder::class)] class User extends Model {}"#,
        );
        assert_eq!(
            labels(r, "<?php use App\\Models\\User; User::nea|"),
            vec!["nearby"]
        );
    }
    #[test]
    fn custom_builder_dispatch_overrides_do_not_inherit_query_forwarding() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "app/Models/User.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class User extends Model { protected static $builder=\App\CustomBuilder::class; }"#,
        );
        for hook in [
            "public function __call($method,$args){return false;}",
            "public function forwardCallTo($object,$method,$args){return false;}",
        ] {
            write(
                r,
                "app/CustomBuilder.php",
                &format!(
                    "<?php namespace App; use Illuminate\\Database\\Eloquent\\Builder; class CustomBuilder extends Builder {{{hook}}}"
                ),
            );
            assert!(
                labels(r, "<?php use App\\Models\\User; User::ord|").is_empty(),
                "{hook}"
            );
        }
    }
    #[test]
    fn dirty_unicode_replacement_negative_contexts_and_cancellation() {
        let d = fixture();
        let r = d.path();
        let p = r.join("app/Models/User.php").canonicalize().unwrap();
        let dirty = r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class User extends Model { public function scopeAtualização($q){} }"#;
        let result = ask(
            r,
            "<?php /* 😀 */ use App\\Models\\User; User::At|junk",
            true,
            &[(p.clone(), dirty)],
        )
        .unwrap();
        assert!(
            result.items.is_empty(),
            "PHP method completion prefix remains case sensitive"
        );
        let result = ask(
            r,
            "<?php /* 😀 */ use App\\Models\\User; User::atu|junk",
            true,
            &[(p, dirty)],
        )
        .unwrap();
        assert_eq!(result.items[0].label, "atualização");
        let edit = result.items[0].text_edit.as_ref().unwrap();
        assert_eq!(edit.end_character - edit.start_character, 7);
        for s in [
            "<?php // User::act|",
            "<?php $s='User::act|",
            "<?php use App\\Models\\User; function show(User $user){$user=null;$user->em|; }",
            "<?php use App\\Models\\User; User::unknown()->act|",
        ] {
            assert!(labels(r, s).is_empty(), "{s}");
        }
        assert!(
            query(
                r,
                &r.join("probe.php"),
                "<?php",
                0,
                5,
                &AtomicBool::new(true),
                &[],
                true
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod real_sources {
    use super::*;
    #[test]
    #[ignore = "requires the explicitly installed MIT Laravel fixture vendor; no PHP execution"]
    fn installed_laravel_sources_support_scopes_chains_fields_and_relationships() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/laravel")
            .canonicalize()
            .unwrap();
        assert!(root.join("vendor/laravel/framework/LICENSE.md").is_file());
        let model = root.join("app/Models/User.php").canonicalize().unwrap();
        let dirty = r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; use Illuminate\Database\Eloquent\Attributes\Scope; class User extends Model { protected $fillable=['name']; public function scopeActive($q){} #[Scope] protected function popular($q){} public function author(){return $this->belongsTo(User::class);} }"#;
        for (input, expected) in [
            ("<?php use App\\Models\\User; User::act", "active"),
            (
                "<?php use App\\Models\\User; User::where('id',1)->pop",
                "popular",
            ),
            (
                "<?php use App\\Models\\User; User::query()->where('id',1)->act",
                "active",
            ),
            (
                "<?php use App\\Models\\User; User::query()->first()->na",
                "name",
            ),
            (
                "<?php use App\\Models\\User; $u=new User; $u->author()->act",
                "active",
            ),
        ] {
            let result = query(
                &root,
                &root.join("editor_probe.php"),
                input,
                0,
                input.encode_utf16().count() as u32,
                &AtomicBool::new(false),
                &[(model.clone(), dirty)],
                true,
            )
            .unwrap_or_else(|| panic!("no source-derived context for {input}"));
            assert!(
                result.items.iter().any(|i| i.label == expected),
                "{input}: {:?}",
                result.items.iter().map(|i| &i.label).collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod conservative_cases {
    use super::tests::{ask, fixture, labels, write};
    #[test]
    fn static_shadowing_terminal_scopes_and_conditional_assignments_are_not_invented() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "app/Models/User.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class User extends Model { protected $fillable=['email']; public function where(){return null;} public function scopeBroken($q){return 42;} public function scopeCollected($q){return $q->get();} public function scopeReassigned($q){$q=false;return $q;} public function scopeActive($q){} }"#,
        );
        for s in [
            "<?php use App\\Models\\User; User::whe|",
            "<?php use App\\Models\\User; User::where()->act|",
            "<?php use App\\Models\\User; User::broken()->act|",
            "<?php use App\\Models\\User; User::collected()->act|",
            "<?php use App\\Models\\User; User::reassigned()->act|",
            "<?php use App\\Models\\User; function f(User $user){$user =& $other;$user->em|;}",
            "<?php use App\\Models\\User; if($yes){$u=new User;} $u->em|",
        ] {
            assert!(labels(r, s).is_empty(), "{s}");
        }
        assert_eq!(
            labels(r, "<?php use App\\Models\\User; User::bro|"),
            vec!["broken"]
        );
    }
    #[test]
    fn real_properties_in_class_trait_or_parent_block_magic_relationships() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "app/Models/RealProperties.php",
            r#"<?php namespace App\Models; trait RealProperties { public $author=false; }"#,
        );
        write(
            r,
            "app/Models/RealParent.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class RealParent extends Model {public $author=false;}"#,
        );
        for (base, body) in [
            (
                "\\Illuminate\\Database\\Eloquent\\Model",
                "public $author=false;",
            ),
            (
                "\\Illuminate\\Database\\Eloquent\\Model",
                "use RealProperties;",
            ),
            ("RealParent", ""),
        ] {
            write(
                r,
                "app/Models/User.php",
                &format!(
                    "<?php namespace App\\Models; class User extends {base} {{{body} public function author(){{return $this->belongsTo(Author::class);}} }}"
                ),
            );
            assert!(
                labels(
                    r,
                    "<?php use App\\Models\\User; $u=new User; $u->author->dis|"
                )
                .is_empty(),
                "{base} {body}"
            );
            assert!(
                ask(
                    r,
                    "<?php use App\\Models\\User; $u=new User; $u->aut|hor;",
                    false,
                    &[]
                )
                .is_some_and(|r| r.targets.is_empty())
            );
            // Explicit method calls retain the relationship query semantics.
            assert_eq!(
                labels(
                    r,
                    "<?php use App\\Models\\User; $u=new User; $u->author()->ver|"
                ),
                vec!["verified"]
            );
        }
    }
    #[test]
    fn inheritance_traits_and_framework_snapshot_preserve_exact_targets() {
        let d = fixture();
        let r = d.path();
        write(
            r,
            "app/Models/Filters.php",
            r#"<?php namespace App\Models; trait Filters { public function scopeArchived($q){} }"#,
        );
        write(
            r,
            "app/Models/BaseUser.php",
            r#"<?php namespace App\Models; use Illuminate\Database\Eloquent\Model; class BaseUser extends Model { public function scopeActive($q){} }"#,
        );
        write(
            r,
            "app/Models/User.php",
            r#"<?php namespace App\Models; class User extends BaseUser {use Filters;}"#,
        );
        assert_eq!(
            labels(r, "<?php use App\\Models\\User; User::active()->arc|"),
            vec!["archived"]
        );
        let result = ask(
            r,
            "<?php use App\\Models\\User; User::arch|ived();",
            false,
            &[],
        )
        .unwrap();
        assert!(result.targets[0].path.ends_with("Filters.php"));
        let model = r
            .join("vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php")
            .canonicalize()
            .unwrap();
        let changed = "<?php namespace Illuminate\\Database\\Eloquent; class Model {}";
        assert!(
            ask(
                r,
                "<?php use App\\Models\\User; User::act|",
                true,
                &[(model, changed)]
            )
            .is_none()
        );
    }
}
