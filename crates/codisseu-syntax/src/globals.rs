//! Bounded global symbol identity checks. Proposals are never synthesized here.
use crate::{
    Conflict, ConflictKind, ConflictLocation, Coverage, IssueKind, RenameSnapshot,
    RenameValidation, ValidationLimits, ValidationScope,
    ast::Language,
    incremental::{self, Budget, ByteEdit, text_range},
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
};
use tree_sitter::{Node, Parser};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Class,
    Function,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Role {
    Declaration(Kind),
    Import(usize),
    Alias(usize),
    ExportAlias,
    Reference(Option<Kind>),
    GlobalReference,
    NamespacePrefix(Option<Kind>),
    Local,
}
#[derive(Clone, Debug)]
struct Token {
    range: Range<usize>,
    name: String,
    qualified: String,
    scope: usize,
    role: Role,
    namespace: String,
    unsafe_use: bool,
}
#[derive(Clone, Debug)]
struct Import {
    name: String,
    local: String,
    module: String,
    kind: Option<Kind>,
    explicit_alias: bool,
    reexport: bool,
    namespace: String,
}
#[derive(Default)]
struct Facts {
    tokens: Vec<Token>,
    imports: Vec<Import>,
    parents: Vec<Option<usize>>,
    hazards: Vec<String>,
    errors: bool,
    exported: HashSet<usize>,
    module: bool,
}
struct File<'a> {
    snapshot: &'a RenameSnapshot,
    language: Language,
    before: Facts,
    after: Facts,
    edits: Vec<ByteEdit>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Binding {
    Global(usize, usize),
    Local(usize),
    Unknown,
    Ambiguous,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Resolved {
    binding: Binding,
    changes_name: bool,
}
fn field(parent: Node<'_>, name: &str, node: Node<'_>) -> bool {
    parent
        .child_by_field_name(name)
        .is_some_and(|n| n.id() == node.id())
}
fn source<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}
fn is_callable(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "method_declaration"
            | "anonymous_function"
            | "arrow_function"
            | "function_declaration"
            | "function_expression"
            | "generator_function_declaration"
            | "generator_function"
            | "method_definition"
    )
}
fn terminal(node: Node<'_>) -> Node<'_> {
    let mut c = node.walk();
    node.named_children(&mut c)
        .filter(|n| matches!(n.kind(), "name" | "identifier" | "type_identifier"))
        .last()
        .unwrap_or(node)
}
fn global_parent(node: Node<'_>) -> bool {
    node.parent().is_some_and(|p| {
        matches!(
            p.kind(),
            "program" | "export_statement" | "namespace_definition"
        ) || (p.kind() == "compound_statement"
            && p.parent()
                .is_some_and(|p| p.kind() == "namespace_definition"))
    })
}
fn inspect(
    root: Node<'_>,
    text: &str,
    language: Language,
    budget: &mut Budget<'_>,
) -> Result<Facts, IssueKind> {
    let mut facts = Facts::default();
    facts.parents.push(None);
    let mut stack = vec![(root, 0usize, String::new())];
    let mut flat_namespace = String::new();
    let mut top = root.walk();
    for n in root.named_children(&mut top) {
        if n.kind() == "namespace_definition"
            && n.child_by_field_name("body").is_none()
            && let Some(name) = n.child_by_field_name("name")
        {
            if !flat_namespace.is_empty() {
                facts
                    .hazards
                    .push("Múltiplos namespaces PHP sem bloco exigem mapeamento adicional".into());
            }
            flat_namespace = source(name, text).into();
        }
    }
    while let Some((node, inherited, mut namespace)) = stack.pop() {
        budget.node()?;
        let kind = node.kind();
        facts.module |= matches!(kind, "import_statement" | "export_statement");
        facts.errors |= node.is_error() || node.is_missing();
        if namespace.is_empty() {
            namespace = flat_namespace.clone();
        }
        if kind == "namespace_definition" {
            namespace = node
                .child_by_field_name("name")
                .map_or(String::new(), |n| source(n, text).into());
        }
        if matches!(
            kind,
            "namespace_name"
                | "comment"
                | "string"
                | "string_content"
                | "encapsed_string"
                | "heredoc"
                | "nowdoc"
                | "php_tag"
        ) {
            continue;
        }
        if kind == "namespace_use_declaration" && language == Language::Php {
            if source(node, text).trim_start().starts_with("use const ") {
                facts
                    .hazards
                    .push("Import de constante PHP exige namespace distinto".into());
                continue;
            }
            let mut c = node.walk();
            let children = node.named_children(&mut c).collect::<Vec<_>>();
            let prefix = children
                .iter()
                .find(|n| n.kind() == "namespace_name")
                .map(|n| source(*n, text))
                .unwrap_or("");
            let mut clauses = Vec::new();
            for child in children {
                budget.node()?;
                if child.kind() == "namespace_use_clause" {
                    clauses.push((child, ""));
                } else if child.kind() == "namespace_use_group" {
                    let mut gc = child.walk();
                    for clause in child.named_children(&mut gc) {
                        budget.node()?;
                        if clause.kind() == "namespace_use_clause" {
                            clauses.push((clause, prefix));
                        } else {
                            facts
                                .hazards
                                .push("Cláusula agrupada PHP não resolvida".into());
                        }
                    }
                }
            }
            for (clause, prefix) in clauses {
                let mut cc = clause.walk();
                let Some(name) = clause
                    .named_children(&mut cc)
                    .find(|n| matches!(n.kind(), "qualified_name" | "name"))
                else {
                    facts.hazards.push("Import PHP sem nome direto".into());
                    continue;
                };
                let leaf = terminal(name);
                let alias = clause.child_by_field_name("alias");
                let function = source(node, text).trim_start().starts_with("use function ")
                    || source(clause, text).trim_start().starts_with("function ");
                if source(clause, text).trim_start().starts_with("const ") {
                    facts
                        .hazards
                        .push("Import de constante PHP exige namespace distinto".into());
                    continue;
                }
                let imported = if prefix.is_empty() {
                    source(name, text).trim_start_matches('\\').to_owned()
                } else {
                    format!("{prefix}\\{}", source(name, text).trim_start_matches('\\'))
                };
                let imp = Import {
                    name: imported,
                    local: alias.map_or(source(leaf, text), |n| source(n, text)).into(),
                    module: String::new(),
                    kind: Some(if function {
                        Kind::Function
                    } else {
                        Kind::Class
                    }),
                    explicit_alias: alias.is_some(),
                    reexport: false,
                    namespace: namespace.clone(),
                };
                let id = facts.imports.len();
                facts.imports.push(imp);
                facts.tokens.push(Token {
                    range: leaf.byte_range(),
                    name: source(leaf, text).into(),
                    qualified: source(name, text).into(),
                    scope: 0,
                    role: Role::Import(id),
                    namespace: namespace.clone(),
                    unsafe_use: false,
                });
                if let Some(alias) = alias {
                    facts.tokens.push(Token {
                        range: alias.byte_range(),
                        name: source(alias, text).into(),
                        qualified: source(alias, text).into(),
                        scope: 0,
                        role: Role::Alias(id),
                        namespace: namespace.clone(),
                        unsafe_use: false,
                    });
                }
            }
            continue;
        }
        if kind == "import_statement" && language == Language::TypeScript {
            let module = node
                .child_by_field_name("source")
                .map(|n| source(n, text).trim_matches(['\'', '"']).to_owned())
                .unwrap_or_default();
            let mut descendants = vec![node];
            let mut found = false;
            while let Some(n) = descendants.pop() {
                budget.node()?;
                if n.kind() == "import_specifier" {
                    found = true;
                    let Some(name) = n.child_by_field_name("name") else {
                        continue;
                    };
                    let alias = n.child_by_field_name("alias");
                    let id = facts.imports.len();
                    facts.imports.push(Import {
                        name: source(name, text).into(),
                        local: alias.map_or(source(name, text), |n| source(n, text)).into(),
                        module: module.clone(),
                        kind: None,
                        explicit_alias: alias.is_some(),
                        reexport: false,
                        namespace: String::new(),
                    });
                    facts.tokens.push(Token {
                        range: name.byte_range(),
                        name: source(name, text).into(),
                        qualified: source(name, text).into(),
                        scope: 0,
                        role: Role::Import(id),
                        namespace: String::new(),
                        unsafe_use: false,
                    });
                    if let Some(alias) = alias {
                        facts.tokens.push(Token {
                            range: alias.byte_range(),
                            name: source(alias, text).into(),
                            qualified: source(alias, text).into(),
                            scope: 0,
                            role: Role::Alias(id),
                            namespace: String::new(),
                            unsafe_use: false,
                        });
                    }
                } else {
                    let mut c = n.walk();
                    descendants.extend(
                        n.named_children(&mut c)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev(),
                    );
                }
            }
            if !found {
                facts
                    .hazards
                    .push("Import default/namespace/lateral TypeScript não resolvido".into());
            }
            continue;
        }
        if kind == "export_statement"
            && source(node, text)
                .trim_start()
                .starts_with("export default")
        {
            facts
                .hazards
                .push("Export default exige contrato de nome local e módulo".into());
        }
        if kind == "export_statement" && node.child_by_field_name("declaration").is_none() {
            if let Some(module_node) = node.child_by_field_name("source") {
                let module = source(module_node, text)
                    .trim_matches(['\'', '"'])
                    .to_owned();
                let mut descendants = vec![node];
                let mut found = false;
                while let Some(n) = descendants.pop() {
                    budget.node()?;
                    if n.kind() == "export_specifier" {
                        found = true;
                        let Some(name) = n.child_by_field_name("name") else {
                            facts.hazards.push("Reexport sem identificador".into());
                            continue;
                        };
                        let alias = n.child_by_field_name("alias");
                        let id = facts.imports.len();
                        facts.imports.push(Import {
                            name: source(name, text).into(),
                            local: alias.map_or(source(name, text), |n| source(n, text)).into(),
                            module: module.clone(),
                            kind: None,
                            explicit_alias: alias.is_some(),
                            reexport: true,
                            namespace: String::new(),
                        });
                        let token_id = facts.tokens.len();
                        facts.tokens.push(Token {
                            range: name.byte_range(),
                            name: source(name, text).into(),
                            qualified: source(name, text).into(),
                            scope: 0,
                            role: Role::Import(id),
                            namespace: String::new(),
                            unsafe_use: false,
                        });
                        if let Some(alias) = alias {
                            facts.exported.insert(facts.tokens.len());
                            facts.tokens.push(Token {
                                range: alias.byte_range(),
                                name: source(alias, text).into(),
                                qualified: source(alias, text).into(),
                                scope: 0,
                                role: Role::Alias(id),
                                namespace: String::new(),
                                unsafe_use: false,
                            });
                        } else {
                            facts.exported.insert(token_id);
                        }
                    } else {
                        let mut c = n.walk();
                        descendants.extend(
                            n.named_children(&mut c)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev(),
                        );
                    }
                }
                if !found {
                    facts
                        .hazards
                        .push("Export estrela ou namespace exige resolução adicional".into());
                }
                continue;
            }
            let mut descendants = vec![node];
            let mut found = false;
            while let Some(n) = descendants.pop() {
                budget.node()?;
                if n.kind() == "export_specifier" {
                    found = true;
                    let Some(name) = n.child_by_field_name("name") else {
                        facts.hazards.push("Export local sem identificador".into());
                        continue;
                    };
                    let token = facts.tokens.len();
                    facts.tokens.push(Token {
                        range: name.byte_range(),
                        name: source(name, text).into(),
                        qualified: source(name, text).into(),
                        scope: 0,
                        role: Role::Reference(None),
                        namespace: String::new(),
                        unsafe_use: false,
                    });
                    if let Some(alias) = n.child_by_field_name("alias") {
                        facts.exported.insert(facts.tokens.len());
                        facts.tokens.push(Token {
                            range: alias.byte_range(),
                            name: source(alias, text).into(),
                            qualified: source(alias, text).into(),
                            scope: 0,
                            role: Role::ExportAlias,
                            namespace: String::new(),
                            unsafe_use: false,
                        });
                    } else {
                        facts.exported.insert(token);
                    }
                } else {
                    let mut c = n.walk();
                    descendants.extend(
                        n.named_children(&mut c)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev(),
                    );
                }
            }
            if !found {
                facts
                    .hazards
                    .push("Export local sem cláusula nomeada exige resolução adicional".into());
            }
            continue;
        }
        if matches!(
            kind,
            "object_pattern"
                | "array_pattern"
                | "rest_pattern"
                | "variable_declaration"
                | "internal_module"
                | "ambient_declaration"
                | "import_alias"
                | "namespace_use_group"
                | "eval_intrinsic"
                | "dynamic_variable_name"
        ) {
            facts.hazards.push(format!(
                "Construção dinâmica ou namespace não resolvido: {kind}"
            ));
        }
        if language == Language::Php
            && kind == "function_call_expression"
            && let Some(n) = node.child_by_field_name("function")
            && (!matches!(n.kind(), "name" | "qualified_name" | "relative_name")
                || matches!(
                    source(n, text).to_ascii_lowercase().as_str(),
                    "call_user_func"
                        | "call_user_func_array"
                        | "class_alias"
                        | "function_exists"
                        | "class_exists"
                        | "method_exists"
                        | "is_a"
                        | "is_subclass_of"
                        | "get_class"
                        | "constant"
                        | "defined"
                        | "define"
                        | "array_map"
                        | "array_filter"
                        | "usort"
                        | "uasort"
                        | "uksort"
                        | "array_walk"
                        | "array_walk_recursive"
                        | "preg_replace_callback"
                        | "register_shutdown_function"
                        | "set_error_handler"
                        | "set_exception_handler"
                        | "spl_autoload_register"
                        | "forward_static_call"
                        | "forward_static_call_array"
                        | "is_callable"
                        | "unserialize"
                ))
        {
            facts
                .hazards
                .push("Reflexão ou chamada PHP dinâmica impede provar símbolos globais".into());
        }
        if language == Language::Php && kind == "object_creation_expression" {
            let mut cursor = node.walk();
            if let Some(class) = node
                .named_children(&mut cursor)
                .find(|n| n.kind() != "comment")
                && (!matches!(
                    class.kind(),
                    "name" | "qualified_name" | "relative_name" | "anonymous_class"
                ) || source(class, text)
                    .trim_start_matches('\\')
                    .rsplit('\\')
                    .next()
                    .is_some_and(|n| n.to_ascii_lowercase().starts_with("reflection")))
            {
                facts
                    .hazards
                    .push("Construção dinâmica ou reflexão PHP não resolvida".into());
            }
        }
        if language == Language::TypeScript
            && kind == "call_expression"
            && node
                .child_by_field_name("function")
                .is_some_and(|n| source(n, text) == "eval" || source(n, text) == "Function")
        {
            facts
                .hazards
                .push("Avaliação dinâmica TypeScript não resolvida".into());
        }
        let declaration_kind = match kind {
            "class_declaration" => Some(Kind::Class),
            "function_definition" | "function_declaration" | "generator_function_declaration" => {
                Some(Kind::Function)
            }
            _ => None,
        };
        let declaration = node
            .child_by_field_name("name")
            .filter(|_| declaration_kind.is_some());
        if let (Some(k), Some(name)) = (declaration_kind, declaration) {
            let global = global_parent(node);
            if language == Language::Php && !global {
                facts
                    .hazards
                    .push("Declaração PHP condicional ou aninhada exige prova de carga".into());
            }
            let id = facts.tokens.len();
            facts.tokens.push(Token {
                range: name.byte_range(),
                name: source(name, text).into(),
                qualified: source(name, text).into(),
                scope: inherited,
                role: if global {
                    Role::Declaration(k)
                } else {
                    Role::Local
                },
                namespace: namespace.clone(),
                unsafe_use: false,
            });
            if global
                && node
                    .parent()
                    .is_some_and(|p| p.kind() == "export_statement")
            {
                facts.exported.insert(id);
            }
        }
        let scope = if is_callable(kind)
            || matches!(
                kind,
                "statement_block"
                    | "class_body"
                    | "declaration_list"
                    | "catch_clause"
                    | "for_statement"
                    | "for_in_statement"
            ) {
            let id = facts.parents.len();
            facts.parents.push(Some(inherited));
            id
        } else {
            inherited
        };
        if language == Language::Php && matches!(kind, "name" | "qualified_name" | "relative_name")
        {
            if let Some(p) = node.parent() {
                let pk = p.kind();
                let expected = if pk == "function_call_expression" && field(p, "function", node) {
                    Some(Kind::Function)
                } else if matches!(
                    pk,
                    "object_creation_expression"
                        | "named_type"
                        | "base_clause"
                        | "class_interface_clause"
                        | "instanceof_expression"
                        | "attribute"
                ) || (matches!(
                    pk,
                    "scoped_call_expression"
                        | "class_constant_access_expression"
                        | "scoped_property_access_expression"
                ) && field(p, "scope", node))
                    || (pk == "class_constant_access_expression"
                        && p.named_child(0).is_some_and(|n| n.id() == node.id()))
                    || (pk == "binary_expression"
                        && field(p, "right", node)
                        && p.child_by_field_name("operator")
                            .is_some_and(|n| source(n, text) == "instanceof"))
                {
                    Some(Kind::Class)
                } else {
                    None
                };
                // Qualified names contain separate AST identifiers for the
                // import alias prefix and the terminal class/function. Keep
                // both bindings: renaming one must preserve the other.
                if kind == "qualified_name" && !source(node, text).starts_with('\\') {
                    let mut cursor = node.walk();
                    let prefix = node
                        .named_children(&mut cursor)
                        .find(|n| n.kind() == "namespace_name")
                        .and_then(|n| n.named_child(0));
                    if let Some(prefix) = prefix.filter(|n| n.kind() == "name") {
                        facts.tokens.push(Token {
                            range: prefix.byte_range(),
                            name: source(prefix, text).into(),
                            qualified: source(node, text).into(),
                            scope,
                            role: Role::NamespacePrefix(expected),
                            namespace: namespace.clone(),
                            unsafe_use: false,
                        });
                    }
                }
                if let Some(k) = expected {
                    let leaf = terminal(node);
                    facts.tokens.push(Token {
                        range: leaf.byte_range(),
                        name: source(leaf, text).into(),
                        qualified: source(node, text).into(),
                        scope,
                        role: Role::Reference(Some(k)),
                        namespace: namespace.clone(),
                        unsafe_use: false,
                    });
                }
            }
            if matches!(kind, "qualified_name" | "relative_name") {
                continue;
            }
        }
        if language == Language::TypeScript
            && kind == "property_identifier"
            && let Some(parent) = node.parent()
            && parent.kind() == "member_expression"
            && parent
                .child_by_field_name("object")
                .is_some_and(|n| matches!(source(n, text), "globalThis" | "window"))
        {
            facts.tokens.push(Token {
                range: node.byte_range(),
                name: source(node, text).into(),
                qualified: source(node, text).into(),
                scope: 0,
                role: Role::GlobalReference,
                namespace: String::new(),
                unsafe_use: false,
            });
        }
        if language == Language::TypeScript
            && kind == "subscript_expression"
            && node
                .child_by_field_name("object")
                .is_some_and(|n| matches!(source(n, text), "globalThis" | "window"))
        {
            facts
                .hazards
                .push("Acesso computado ao objeto global exige prova adicional".into());
        }
        if language == Language::TypeScript
            && matches!(
                kind,
                "identifier" | "type_identifier" | "shorthand_property_identifier"
            )
            && let Some(p) = node.parent()
        {
            let pk = p.kind();
            let is_decl_name = declaration.is_some_and(|n| n.id() == node.id())
                || (matches!(
                    pk,
                    "class_declaration" | "function_declaration" | "generator_function_declaration"
                ) && field(p, "name", node));
            let property = (pk == "member_expression" && field(p, "property", node))
                || matches!(
                    pk,
                    "property_signature"
                        | "method_signature"
                        | "method_definition"
                        | "public_field_definition"
                        | "pair"
                        | "labeled_statement"
                        | "break_statement"
                        | "continue_statement"
                ) && field(p, "name", node);
            if source(node, text).contains('\\') {
                facts
                    .hazards
                    .push("Identificador TypeScript escapado não resolvido".into());
            }
            let local = (pk == "variable_declarator" && field(p, "name", node))
                || (matches!(pk, "required_parameter" | "optional_parameter")
                    && field(p, "pattern", node))
                || (pk == "catch_clause" && field(p, "parameter", node))
                || (pk == "arrow_function" && field(p, "parameter", node));
            if !is_decl_name && !property {
                let unsupported = matches!(
                    pk,
                    "type_alias_declaration"
                        | "type_parameter"
                        | "interface_declaration"
                        | "enum_declaration"
                        | "object_pattern"
                        | "array_pattern"
                        | "rest_pattern"
                        | "pair_pattern"
                ) || kind == "shorthand_property_identifier";
                let expected = if kind == "type_identifier" {
                    Some(Kind::Class)
                } else {
                    None
                };
                facts.tokens.push(Token {
                    range: node.byte_range(),
                    name: source(node, text).into(),
                    qualified: source(node, text).into(),
                    scope,
                    role: if local {
                        Role::Local
                    } else {
                        Role::Reference(expected)
                    },
                    namespace: String::new(),
                    unsafe_use: unsupported,
                });
            }
        }
        let mut c = node.walk();
        for child in node
            .named_children(&mut c)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if declaration.is_some_and(|n| n.id() == child.id()) {
                continue;
            }
            stack.push((child, scope, namespace.clone()));
        }
    }
    Ok(facts)
}
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
fn module_target(files: &[File<'_>], file: usize, module: &str) -> Option<usize> {
    if !module.starts_with('.') || module.contains('\\') {
        return None;
    }
    let path = normalized(&files[file].snapshot.path.parent()?.join(module));
    let mut candidates = vec![path.clone()];
    if path.extension().is_none() {
        candidates.push(path.with_extension("ts"));
        candidates.push(path.join("index.ts"));
    } else if path.extension().is_some_and(|e| e == "js") {
        candidates.push(path.with_extension("ts"));
    }
    let matches: Vec<_> = files
        .iter()
        .enumerate()
        .filter(|(_, f)| candidates.contains(&normalized(&f.snapshot.path)))
        .map(|(i, _)| i)
        .collect();
    if matches.len() == 1 {
        Some(matches[0])
    } else {
        None
    }
}
fn facts<'a>(file: &'a File<'_>, after: bool) -> &'a Facts {
    if after { &file.after } else { &file.before }
}
fn global_key(language: Language, namespace: &str, name: &str) -> String {
    language.key(&if namespace.is_empty() {
        name.into()
    } else {
        format!("{namespace}\\{name}")
    })
}
fn ts_key(file: usize, facts: &Facts, name: &str) -> String {
    if facts.module {
        format!("{file}:{name}")
    } else {
        format!("script:{name}")
    }
}
fn export_binding(
    files: &[File<'_>],
    after: bool,
    file: usize,
    name: &str,
    budget: &mut Budget<'_>,
    visiting: &mut HashSet<(usize, String)>,
) -> Result<Binding, IssueKind> {
    budget.node()?;
    if !visiting.insert((file, name.into())) {
        return Ok(Binding::Unknown);
    }
    let f = facts(&files[file], after);
    let mut candidates = Vec::new();
    for &id in &f.exported {
        budget.node()?;
        if f.tokens[id].name == name {
            candidates.push(id);
        }
    }
    let binding = if candidates.len() > 1 {
        Binding::Ambiguous
    } else if let Some(&token) = candidates.first() {
        match f.tokens[token].role {
            Role::Declaration(_) | Role::Alias(_) | Role::ExportAlias => {
                Binding::Global(file, token)
            }
            Role::Import(import) => {
                let import = &f.imports[import];
                if let Some(target) = module_target(files, file, &import.module) {
                    export_binding(files, after, target, &import.name, budget, visiting)?
                } else {
                    Binding::Unknown
                }
            }
            Role::Reference(_) => {
                let mut local = Vec::new();
                for (id, t) in f.tokens.iter().enumerate() {
                    budget.node()?;
                    if matches!(t.role, Role::Declaration(_)) && t.name == name && t.scope == 0 {
                        local.push(Binding::Global(file, id));
                    }
                }
                for (id, import) in f.imports.iter().enumerate() {
                    budget.node()?;
                    if import.reexport || import.local != name {
                        continue;
                    }
                    if import.explicit_alias {
                        if let Some((token, _)) = f
                            .tokens
                            .iter()
                            .enumerate()
                            .find(|(_, t)| t.role == Role::Alias(id))
                        {
                            local.push(Binding::Global(file, token));
                        }
                    } else if let Some(target) = module_target(files, file, &import.module) {
                        local.push(export_binding(
                            files,
                            after,
                            target,
                            &import.name,
                            budget,
                            visiting,
                        )?);
                    }
                }
                if local.len() == 1 {
                    local[0]
                } else if local.is_empty() {
                    Binding::Unknown
                } else {
                    Binding::Ambiguous
                }
            }
            _ => Binding::Unknown,
        }
    } else {
        Binding::Unknown
    };
    visiting.remove(&(file, name.into()));
    Ok(binding)
}
fn resolve(
    files: &[File<'_>],
    after: bool,
    budget: &mut Budget<'_>,
) -> Result<Vec<Vec<Resolved>>, IssueKind> {
    let mut globals: HashMap<(Language, String, Kind), Vec<(usize, usize)>> = HashMap::new();
    for (fi, file) in files.iter().enumerate() {
        for (ti, t) in facts(file, after).tokens.iter().enumerate() {
            budget.node()?;
            if let Role::Declaration(kind) = t.role {
                let key = if file.language == Language::Php {
                    global_key(file.language, &t.namespace, &t.name)
                } else {
                    ts_key(fi, facts(file, after), &t.name)
                };
                globals
                    .entry((file.language, key, kind))
                    .or_default()
                    .push((fi, ti));
            }
        }
    }
    let lookup = |language: Language, key: String, kind: Option<Kind>| -> Binding {
        let mut matches = Vec::new();
        for k in [Kind::Class, Kind::Function] {
            if kind.is_none_or(|wanted| wanted == k)
                && let Some(ids) = globals.get(&(language, key.clone(), k))
            {
                matches.extend(ids);
            }
        }
        match matches.as_slice() {
            [] => Binding::Unknown,
            [(f, t)] => Binding::Global(*f, *t),
            _ => Binding::Ambiguous,
        }
    };
    let mut result = Vec::new();
    for (fi, file) in files.iter().enumerate() {
        let f = facts(file, after);
        let mut resolved = Vec::new();
        let mut locals: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
        for (id, token) in f.tokens.iter().enumerate() {
            budget.node()?;
            if token.role == Role::Local {
                locals
                    .entry((token.scope, &token.name))
                    .or_default()
                    .push(id);
            }
        }
        let mut import_names: HashMap<(String, Option<Kind>, String), Vec<usize>> = HashMap::new();
        for (id, import) in f.imports.iter().enumerate() {
            budget.node()?;
            if import.reexport {
                continue;
            }
            import_names
                .entry((
                    import.namespace.clone(),
                    import.kind,
                    file.language.key(&import.local),
                ))
                .or_default()
                .push(id);
        }

        let aliases: HashMap<usize, usize> = f
            .tokens
            .iter()
            .enumerate()
            .filter_map(|(id, t)| {
                if let Role::Alias(import) = t.role {
                    Some((import, id))
                } else {
                    None
                }
            })
            .collect();
        let mut imports = Vec::with_capacity(f.imports.len());
        for import in &f.imports {
            budget.node()?;
            let binding = if file.language == Language::Php {
                lookup(file.language, file.language.key(&import.name), import.kind)
            } else if let Some(target) = module_target(files, fi, &import.module) {
                export_binding(
                    files,
                    after,
                    target,
                    &import.name,
                    budget,
                    &mut HashSet::new(),
                )?
            } else {
                Binding::Unknown
            };
            imports.push(binding);
        }
        for (ti, t) in f.tokens.iter().enumerate() {
            budget.node()?;
            let changes_name = true;
            let binding = match t.role {
                Role::Declaration(k) => lookup(
                    file.language,
                    if file.language == Language::Php {
                        global_key(file.language, &t.namespace, &t.name)
                    } else {
                        ts_key(fi, facts(file, after), &t.name)
                    },
                    if file.language == Language::Php {
                        Some(k)
                    } else {
                        None
                    },
                ),
                Role::Import(i) => imports[i],
                Role::Alias(_) | Role::ExportAlias => Binding::Global(fi, ti),
                Role::Local => Binding::Local(ti),
                Role::NamespacePrefix(None) => Binding::Unknown,
                Role::NamespacePrefix(Some(expected)) => {
                    let ids = import_names.get(&(
                        t.namespace.clone(),
                        Some(Kind::Class),
                        file.language.key(&t.name),
                    ));
                    match ids.map(Vec::as_slice).unwrap_or_default() {
                        [id] if f.imports[*id].explicit_alias => {
                            let suffix = t.qualified.split_once('\\').map(|(_, rest)| rest);
                            let terminal_binding = suffix.map_or(Binding::Unknown, |suffix| {
                                lookup(
                                    file.language,
                                    file.language
                                        .key(&format!("{}\\{}", f.imports[*id].name, suffix)),
                                    Some(expected),
                                )
                            });
                            if matches!(terminal_binding, Binding::Global(_, _)) {
                                aliases
                                    .get(id)
                                    .map_or(Binding::Unknown, |&alias| Binding::Global(fi, alias))
                            } else {
                                terminal_binding
                            }
                        }
                        [] | [_] => Binding::Unknown,
                        _ => Binding::Ambiguous,
                    }
                }
                Role::GlobalReference => lookup(
                    Language::TypeScript,
                    format!("script:{}", t.name),
                    Some(Kind::Function),
                ),
                Role::Reference(expected) => {
                    let mut local = None;
                    let mut scope = Some(t.scope);
                    while let Some(s) = scope {
                        budget.node()?;
                        if expected != Some(Kind::Class)
                            && let Some(found) = locals.get(&(s, t.name.as_str()))
                        {
                            local = Some(if found.len() == 1 {
                                Binding::Local(found[0])
                            } else {
                                Binding::Ambiguous
                            });
                            break;
                        }
                        scope = f.parents[s];
                    }
                    if let Some(local) = local {
                        local
                    } else if file.language == Language::TypeScript {
                        let ids = import_names.get(&(String::new(), None, t.name.clone()));
                        let imps: Vec<_> = ids
                            .into_iter()
                            .flatten()
                            .map(|&id| (id, &f.imports[id]))
                            .collect();
                        if imps.len() > 1 {
                            Binding::Ambiguous
                        } else if let Some((id, imp)) = imps.first() {
                            if imp.explicit_alias {
                                aliases
                                    .get(id)
                                    .map_or(Binding::Unknown, |&alias| Binding::Global(fi, alias))
                            } else {
                                imports[*id]
                            }
                        } else {
                            lookup(
                                file.language,
                                ts_key(fi, facts(file, after), &t.name),
                                expected,
                            )
                        }
                    } else {
                        let qualified = t.qualified.trim_start_matches('\\');
                        let mut parts = qualified.split('\\');
                        let first = parts.next().unwrap_or("");
                        let rest = parts.collect::<Vec<_>>();
                        let absolute = t.qualified.starts_with('\\');
                        let relative = t.qualified.starts_with("namespace\\");
                        let ids = import_names.get(&(
                            t.namespace.clone(),
                            if rest.is_empty() {
                                expected
                            } else {
                                Some(Kind::Class)
                            },
                            file.language.key(first),
                        ));
                        let imps: Vec<_> = ids
                            .into_iter()
                            .flatten()
                            .map(|&id| (id, &f.imports[id]))
                            .collect();
                        if !absolute && !relative && imps.len() > 1 {
                            Binding::Ambiguous
                        } else if !absolute
                            && !relative
                            && rest.is_empty()
                            && let Some((id, imp)) = imps.first()
                        {
                            if imp.explicit_alias {
                                aliases
                                    .get(id)
                                    .map_or(Binding::Unknown, |&alias| Binding::Global(fi, alias))
                            } else {
                                imports[*id]
                            }
                        } else {
                            let key = if absolute {
                                qualified.to_owned()
                            } else if relative {
                                global_key(
                                    file.language,
                                    &t.namespace,
                                    qualified.trim_start_matches("namespace\\"),
                                )
                            } else if !imps.is_empty() {
                                format!("{}\\{}", imps[0].1.name, rest.join("\\"))
                            } else {
                                global_key(file.language, &t.namespace, qualified)
                            };
                            let b = lookup(file.language, file.language.key(&key), expected);
                            if b == Binding::Unknown
                                && expected == Some(Kind::Function)
                                && !absolute
                                && !relative
                                && rest.is_empty()
                            {
                                lookup(file.language, file.language.key(qualified), expected)
                            } else {
                                b
                            }
                        }
                    }
                }
            };
            resolved.push(Resolved {
                binding,
                changes_name,
            });
        }
        result.push(resolved);
    }
    Ok(result)
}

/// Resolve a callable reference through the same lexical/import graph as Rename.
/// Alias identity is followed only to a concrete function declaration. Existing
/// parsed trees are reused and every index operation shares the caller's budget.
pub(crate) fn function_declaration(
    source_files: &[crate::hierarchy::File<'_>],
    file: usize,
    name: Node<'_>,
    budget: &mut Budget<'_>,
) -> Result<(usize, Range<usize>), String> {
    let mut files = Vec::new();
    for f in source_files {
        let before = inspect(f.tree.root_node(), &f.snapshot.before, f.language, budget)
            .map_err(|k| format!("Índice de funções interrompido: {k:?}"))?;
        if before.errors || !before.hazards.is_empty() {
            return Err(
                "Corpus contém sintaxe/fluxos globais não resolvidos para retorno de função".into(),
            );
        }
        files.push(File {
            snapshot: f.snapshot,
            language: f.language,
            before,
            after: Facts::default(),
            edits: vec![],
        });
    }
    let bindings = resolve(&files, false, budget)
        .map_err(|k| format!("Binding de função interrompido: {k:?}"))?;
    let range = terminal(name).byte_range();
    let token = files[file]
        .before
        .tokens
        .iter()
        .position(|t| t.range == range)
        .ok_or("Referência de função sem token nominal")?;
    let mut binding = bindings[file][token].binding;
    let mut visited = HashSet::new();
    loop {
        budget
            .node()
            .map_err(|k| format!("Alias de função interrompido: {k:?}"))?;
        let Binding::Global(f, t) = binding else {
            return Err("Função sombreada, ausente ou ambígua".into());
        };
        if !visited.insert((f, t)) {
            return Err("Ciclo de aliases de função".into());
        }
        match files[f].before.tokens[t].role {
            Role::Declaration(Kind::Function) => {
                return Ok((f, files[f].before.tokens[t].range.clone()));
            }
            Role::Alias(import) => {
                let original = files[f]
                    .before
                    .tokens
                    .iter()
                    .position(|t| t.role == Role::Import(import))
                    .ok_or("Alias sem import original de função")?;
                binding = bindings[f][original].binding;
            }
            _ => {
                return Err(
                    "Retorno exige declaração concreta de função; alias de export não resolvido"
                        .into(),
                );
            }
        }
    }
}
// Conservative known runtime symbols, observed with PHP 8.4.27. This is not a
// complete extension/runtime inventory; external declarations remain outside scope.
fn php_builtin(kind: Kind, name: &str) -> bool {
    match kind {
        Kind::Class => matches!(
            name.to_ascii_lowercase().as_str(),
            "stdclass"
                | "exception"
                | "error"
                | "closure"
                | "generator"
                | "datetime"
                | "datetimeimmutable"
                | "throwable"
                | "traversable"
                | "arrayaccess"
                | "iterator"
                | "countable"
                | "stringable"
                | "weakreference"
                | "weakmap"
                | "fiber"
        ),
        Kind::Function => matches!(
            name.to_ascii_lowercase().as_str(),
            "strlen"
                | "count"
                | "sprintf"
                | "printf"
                | "print_r"
                | "var_dump"
                | "var_export"
                | "json_encode"
                | "json_decode"
                | "array_map"
                | "array_filter"
                | "array_merge"
                | "array_keys"
                | "array_values"
                | "in_array"
                | "is_array"
                | "is_string"
                | "is_int"
                | "abs"
                | "round"
                | "min"
                | "max"
                | "date"
                | "time"
                | "trim"
                | "strtolower"
                | "strtoupper"
                | "substr"
                | "str_replace"
                | "file_get_contents"
                | "file_put_contents"
                | "fopen"
                | "fclose"
                | "fwrite"
                | "fread"
                | "define"
                | "defined"
                | "constant"
                | "function_exists"
                | "class_exists"
                | "get_class"
                | "is_a"
                | "is_subclass_of"
                | "call_user_func"
                | "call_user_func_array"
                | "error_reporting"
                | "set_error_handler"
                | "trigger_error"
                | "header"
                | "setcookie"
                | "session_start"
                | "password_hash"
                | "password_verify"
                | "preg_match"
                | "preg_replace"
                | "extract"
                | "compact"
                | "get_defined_vars"
        ),
    }
}
type SymbolId = (usize, usize);
type SymbolCollisions = HashSet<(SymbolId, SymbolId)>;
type BindingNamespace = (Language, String, Option<Kind>);
fn binding_collisions(
    files: &[File<'_>],
    after: bool,
    budget: &mut Budget<'_>,
) -> Result<SymbolCollisions, IssueKind> {
    let mut names: HashMap<BindingNamespace, Vec<SymbolId>> = HashMap::new();
    for (fi, file) in files.iter().enumerate() {
        let f = facts(file, after);
        for (ti, token) in f.tokens.iter().enumerate() {
            budget.node()?;
            let (name, kind) = match token.role {
                Role::Declaration(kind) => (token.name.as_str(), Some(kind)),
                Role::Import(id) if !f.imports[id].reexport => {
                    (f.imports[id].local.as_str(), f.imports[id].kind)
                }
                Role::Local if file.language == Language::TypeScript && token.scope == 0 => {
                    (token.name.as_str(), None)
                }
                _ => continue,
            };
            let key = if file.language == Language::Php {
                format!("{fi}:{}", global_key(file.language, &token.namespace, name))
            } else {
                ts_key(fi, f, name)
            };
            names
                .entry((
                    file.language,
                    key,
                    if file.language == Language::Php {
                        kind
                    } else {
                        None
                    },
                ))
                .or_default()
                .push((fi, ti));
        }
    }
    for (fi, file) in files.iter().enumerate() {
        let f = facts(file, after);
        for &ti in &f.exported {
            budget.node()?;
            names
                .entry((
                    file.language,
                    format!("export:{fi}:{}", f.tokens[ti].name),
                    None,
                ))
                .or_default()
                .push((fi, ti));
        }
    }
    let mut pairs = HashSet::new();
    for ids in names.values() {
        for (i, &a) in ids.iter().enumerate() {
            for &b in &ids[i + 1..] {
                budget.node()?;
                pairs.insert((a, b));
            }
        }
    }
    Ok(pairs)
}
fn add_conflict(
    report: &mut RenameValidation,
    file: &File<'_>,
    token: &Token,
    target: (&File<'_>, &Token),
    kind: ConflictKind,
    message: &str,
) {
    if report.conflicts.len() < 256 {
        report.conflicts.push(Conflict {
            path: file.snapshot.path.clone(),
            range: text_range(&file.snapshot.before, token.range.clone()),
            message: message.into(),
            related: Some(ConflictLocation {
                path: target.0.snapshot.path.clone(),
                range: text_range(&target.0.snapshot.before, target.1.range.clone()),
            }),
            kind,
            container_name: None,
        });
    }
}
/// `None` delegates proposals without an edited global declaration/import to another validator.
pub(crate) fn validate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    let mut report = RenameValidation::new();
    report.scope = ValidationScope::KnownGlobalBindings;
    report.note="Classes/funções e imports diretos resolvidos nos snapshots PHP/TypeScript fornecidos. Não certifica configuração completa do compilador, autoload dinâmico, reflexão ou arquivos externos ao corpus.".into();
    let fallback = snapshots
        .first()
        .map_or(Path::new(""), |s| s.path.as_path());
    let mut budget = Budget::new(cancel, limits);
    if snapshots.is_empty()
        || snapshots.len() > 64
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots
            .iter()
            .map(|s| s.before.len().saturating_add(s.after.len()))
            .sum::<usize>()
            > 8 * 1024 * 1024
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
    {
        report.issue(
            fallback,
            IssueKind::Limit,
            None,
            "Limite de snapshots globais excedido",
        );
        return Some(report);
    }
    if let Err(message) = crate::validate_rename_candidate(new_name) {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, message);
        return Some(report);
    }
    let mut files = Vec::new();
    let mut recognized = false;
    let mut paths = HashSet::new();
    for snapshot in snapshots {
        if !paths.insert(&snapshot.path) {
            report.issue(
                &snapshot.path,
                IssueKind::InvalidSnapshot,
                None,
                "Caminho duplicado",
            );
            continue;
        }
        let language = match snapshot.path.extension().and_then(|e| e.to_str()) {
            Some("php") => Language::Php,
            Some("ts") => Language::TypeScript,
            _ => {
                if !snapshot.edits.is_empty() {
                    report.issue(
                        &snapshot.path,
                        IssueKind::UnsupportedLanguage,
                        None,
                        "Símbolos globais exigem PHP ou TypeScript independente",
                    );
                }
                continue;
            }
        };
        let edits = match incremental::prepare_edits(snapshot, new_name, &budget) {
            Ok(e) => e,
            Err(e) => {
                report.issue(
                    &snapshot.path,
                    budget.check().err().unwrap_or(IssueKind::InvalidSnapshot),
                    None,
                    e,
                );
                continue;
            }
        };
        let mut parser = Parser::new();
        if parser.set_language(&language.grammar()).is_err() {
            report.issue(
                &snapshot.path,
                IssueKind::Parser,
                None,
                "Gramática incompatível",
            );
            return Some(report);
        }
        let Some(mut tree) = incremental::parse(&mut parser, &snapshot.before, None, &budget)
        else {
            report.issue(
                &snapshot.path,
                budget.check().err().unwrap_or(IssueKind::Parser),
                None,
                "Parser global interrompido",
            );
            return Some(report);
        };
        let before = match inspect(tree.root_node(), &snapshot.before, language, &mut budget) {
            Ok(f) => f,
            Err(k) => {
                report.issue(&snapshot.path, k, None, "Índice global interrompido");
                return Some(report);
            }
        };
        recognized |= edits.iter().any(|e| {
            before.tokens.iter().any(|t| {
                t.range == e.before
                    && matches!(
                        t.role,
                        Role::Declaration(_) | Role::Import(_) | Role::Alias(_) | Role::ExportAlias
                    )
            })
        });
        report.checked_files += 1;
        if before.errors {
            report.issue(
                &snapshot.path,
                IssueKind::SyntaxBefore,
                None,
                "Snapshot anterior contém ERROR/MISSING",
            );
        }
        for edit in edits.iter().rev() {
            tree.edit(&edit.input_edit(&snapshot.before));
        }
        let Some(after_tree) =
            incremental::parse(&mut parser, &snapshot.after, Some(&tree), &budget)
        else {
            report.issue(
                &snapshot.path,
                budget.check().err().unwrap_or(IssueKind::Parser),
                None,
                "Reparse global interrompido",
            );
            return Some(report);
        };
        let after = match inspect(
            after_tree.root_node(),
            &snapshot.after,
            language,
            &mut budget,
        ) {
            Ok(f) => f,
            Err(k) => {
                report.issue(&snapshot.path, k, None, "Índice posterior interrompido");
                return Some(report);
            }
        };
        if !edits.is_empty() {
            report.incremental_reparses += 1;
        }
        files.push(File {
            snapshot,
            language,
            before,
            after,
            edits,
        });
    }
    if !recognized {
        return if report.issues.is_empty() {
            None
        } else {
            Some(report)
        };
    }
    let before = match resolve(&files, false, &mut budget) {
        Ok(r) => r,
        Err(k) => {
            report.issue(fallback, k, None, "Resolução global interrompida");
            return Some(report);
        }
    };
    let after = match resolve(&files, true, &mut budget) {
        Ok(r) => r,
        Err(k) => {
            report.issue(fallback, k, None, "Resolução posterior interrompida");
            return Some(report);
        }
    };
    let previous_collisions = match binding_collisions(&files, false, &mut budget) {
        Ok(c) => c,
        Err(k) => {
            report.issue(fallback, k, None, "Orçamento excedido em colisões");
            return Some(report);
        }
    };
    let collisions = match binding_collisions(&files, true, &mut budget) {
        Ok(c) => c,
        Err(k) => {
            report.issue(fallback, k, None, "Orçamento excedido em colisões");
            return Some(report);
        }
    };
    for &((fi, ti), (df, dt)) in collisions.difference(&previous_collisions) {
        if let (Some(token), Some(target)) = (
            files[fi].before.tokens.get(ti),
            files[df].before.tokens.get(dt),
        ) {
            add_conflict(
                &mut report,
                &files[fi],
                token,
                (&files[df], target),
                ConflictKind::DuplicateBinding,
                "O novo nome colide com declaração ou import existente",
            );
        }
    }
    let mut selected = HashSet::new();
    for (fi, file) in files.iter().enumerate() {
        for e in &file.edits {
            if let Some((ti, _)) = file
                .before
                .tokens
                .iter()
                .enumerate()
                .find(|(_, t)| t.range == e.before)
            {
                if let Binding::Global(df, dt) = before[fi][ti].binding {
                    selected.insert((df, dt));
                } else {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        Some(text_range(&file.snapshot.before, e.before.clone())),
                        "Edição não vinculada a declaração global única",
                    );
                }
            } else {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedEdit,
                    Some(text_range(&file.snapshot.before, e.before.clone())),
                    "Edição não corresponde a identificador global AST inteiro",
                );
            }
        }
    }
    if selected.len() != 1 {
        report.issue(
            fallback,
            IssueKind::UnsupportedEdit,
            None,
            "Rename global exige exatamente uma declaração independente",
        );
    }
    if let Some(&(df, dt)) = selected.iter().next() {
        let target = &files[df].before.tokens[dt];
        report.checked_declarations = selected.len();
        if files[df].language == Language::TypeScript
            && (matches!(new_name, "eval" | "arguments" | "await" | "yield")
                || matches!(target.role, Role::Declaration(Kind::Class))
                    && matches!(
                        new_name,
                        "any"
                            | "unknown"
                            | "never"
                            | "number"
                            | "string"
                            | "boolean"
                            | "bigint"
                            | "symbol"
                            | "object"
                            | "undefined"
                            | "null"
                            | "void"
                    ))
        {
            report.issue(
                &files[df].snapshot.path,
                IssueKind::UnsupportedEdit,
                Some(text_range(&files[df].snapshot.before, target.range.clone())),
                "Novo nome é restrito para binding TypeScript/strict",
            );
        }

        if files[df].language == Language::Php
            && target.namespace.is_empty()
            && let Role::Declaration(kind) = target.role
            && php_builtin(kind, new_name)
        {
            add_conflict(
                &mut report,
                &files[df],
                target,
                (&files[df], target),
                ConflictKind::DuplicateBinding,
                "O novo nome colide com símbolo interno PHP conhecido no namespace global",
            );
        }

        for (fi, file) in files.iter().enumerate() {
            if file.language != files[df].language {
                continue;
            }
            for h in &file.before.hazards {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedEdit,
                    None,
                    h.clone(),
                );
            }
            if file.after.errors {
                if report.conflicts.len() < 256 {
                    report.conflicts.push(Conflict {
                        path: file.snapshot.path.clone(),
                        range: file
                            .snapshot
                            .edits
                            .first()
                            .map(|e| e.range.clone())
                            .unwrap_or_else(|| text_range(&file.snapshot.before, 0..0)),
                        message: "Proposta introduz erro de sintaxe".into(),
                        related: Some(ConflictLocation {
                            path: files[df].snapshot.path.clone(),
                            range: text_range(&files[df].snapshot.before, target.range.clone()),
                        }),
                        kind: ConflictKind::InvalidSyntax,
                        container_name: None,
                    });
                }
                continue;
            }
            if file.before.tokens.len() != file.after.tokens.len()
                || file.before.parents != file.after.parents
                || file.before.imports.len() != file.after.imports.len()
            {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedEdit,
                    None,
                    "Proposta altera estrutura do índice global",
                );
                continue;
            }
            for (ti, (old, new)) in file
                .before
                .tokens
                .iter()
                .zip(&file.after.tokens)
                .enumerate()
            {
                if let Err(k) = budget.node() {
                    report.issue(fallback, k, None, "Comparação global interrompida");
                    return Some(report);
                }
                let edited = file.edits.iter().find(|e| e.before == old.range);
                let selected_binding = before[fi][ti].binding == Binding::Global(df, dt);
                let expected = selected_binding && before[fi][ti].changes_name;
                if old.role != new.role || old.scope != new.scope || old.namespace != new.namespace
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        None,
                        "Papel ou escopo de identificador mudou",
                    );
                }
                if selected_binding && old.unsafe_use {
                    report.issue(&file.snapshot.path,IssueKind::UnsupportedEdit,Some(text_range(&file.snapshot.before,old.range.clone())),"Uso estrutural, shorthand ou declaração não suportada exige prova adicional");
                }
                if expected && edited.is_none() {
                    add_conflict(
                        &mut report,
                        file,
                        old,
                        (&files[df], target),
                        ConflictKind::MissingReference,
                        "Proposta omite declaração, import ou referência comprovada",
                    );
                }
                if edited.is_some() && !expected {
                    add_conflict(
                        &mut report,
                        file,
                        old,
                        (&files[df], target),
                        ConflictKind::BindingChanged,
                        "Proposta altera outro símbolo ou alias explícito independente",
                    );
                }
                if let Some(e) = edited
                    && (new.range != e.after || new.name != new_name)
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        None,
                        "Edição posterior não preserva identificador inteiro",
                    );
                }
                if before[fi][ti].binding != after[fi][ti].binding {
                    add_conflict(
                        &mut report,
                        file,
                        old,
                        (&files[df], target),
                        if after[fi][ti].binding == Binding::Ambiguous {
                            ConflictKind::DuplicateBinding
                        } else {
                            ConflictKind::BindingChanged
                        },
                        "O novo nome colide, captura ou altera um vínculo global",
                    );
                }
                if before[fi][ti].binding == Binding::Unknown
                    && matches!(
                        old.role,
                        Role::Reference(_) | Role::GlobalReference | Role::NamespacePrefix(_)
                    )
                    && (file.language.key(&old.name) == file.language.key(&target.name)
                        || file.language.key(&old.name) == file.language.key(new_name))
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        Some(text_range(&file.snapshot.before, old.range.clone())),
                        "Homônimo sem resolução global comprovada no corpus",
                    );
                }
            }
        }
    }
    report.coverage = if !report.conflicts.is_empty() {
        Coverage::Rejected
    } else if report.issues.is_empty() && selected.len() == 1 {
        Coverage::Verified
    } else {
        Coverage::Unsupported
    };
    Some(report)
}

/// Syntactic preparation for the independent PHP/TS global-symbol provider.
/// The returned placeholder is not a semantic approval; `propose` resolves the
/// complete bounded corpus and the normal validator must approve the preview.
pub(crate) fn prepare(
    snapshot: &RenameSnapshot,
    origin: &crate::TextRange,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    if snapshot.before.len() > 1024 * 1024 {
        return Err("Arquivo excede 1 MiB para preparação global".into());
    }
    let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
        Some("php") => Language::Php,
        Some("ts") => Language::TypeScript,
        _ => return Ok(None),
    };
    let start =
        incremental::byte_offset(&snapshot.before, origin.start_line, origin.start_character)
            .ok_or("Posição UTF-16 inválida na preparação global")?;
    let end = incremental::byte_offset(&snapshot.before, origin.end_line, origin.end_character)
        .ok_or("Posição final UTF-16 inválida na preparação global")?;
    if end < start {
        return Err("Intervalo invertido na preparação global".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut parser = Parser::new();
    parser
        .set_language(&language.grammar())
        .map_err(|_| "Gramática incompatível")?;
    let tree = incremental::parse(&mut parser, &snapshot.before, None, &budget)
        .ok_or_else(|| format!("Preparação global interrompida: {:?}", budget.check().err()))?;
    let facts = inspect(tree.root_node(), &snapshot.before, language, &mut budget)
        .map_err(|k| format!("Preparação global interrompida: {k:?}"))?;
    if facts.errors {
        return Err("Snapshot contém ERROR/MISSING; preparação global indisponível".into());
    }
    Ok(facts
        .tokens
        .iter()
        .find(|t| {
            t.range.start <= start
                && end <= t.range.end
                && (start < t.range.end || start == end)
                && !matches!(t.role, Role::Local)
        })
        .map(|t| t.name.clone()))
}

/// Builds an independent semantic proposal from AST identities, never by text
/// matching. The result is NOT approved: the application must call the ordinary
/// validator and preserve the same exact corpus on preview/apply.
pub(crate) fn propose(
    snapshots: &[RenameSnapshot],
    origin_path: &Path,
    origin: &crate::TextRange,
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<Vec<RenameSnapshot>>, String> {
    crate::validate_rename_candidate(new_name)?;
    if snapshots.is_empty()
        || snapshots.len() > 64
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.before != s.after || !s.edits.is_empty())
        || snapshots.iter().map(|s| s.before.len()).sum::<usize>() > 4 * 1024 * 1024
    {
        return Err("Planejamento global exige até 64 snapshots exatos, sem edições, 1 MiB por arquivo e 4 MiB por corpus".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut files = Vec::new();
    let mut paths = HashSet::new();
    for snapshot in snapshots {
        budget
            .check()
            .map_err(|k| format!("Planejamento global interrompido: {k:?}"))?;
        if !paths.insert(&snapshot.path) {
            return Err("Caminho duplicado no planejamento global".into());
        }
        let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
            Some("php") => Language::Php,
            Some("ts") => Language::TypeScript,
            _ => continue,
        };
        let mut parser = Parser::new();
        parser
            .set_language(&language.grammar())
            .map_err(|_| "Gramática incompatível")?;
        let tree = incremental::parse(&mut parser, &snapshot.before, None, &budget)
            .ok_or_else(|| format!("Parser global interrompido: {:?}", budget.check().err()))?;
        let before = inspect(tree.root_node(), &snapshot.before, language, &mut budget)
            .map_err(|k| format!("Índice global interrompido: {k:?}"))?;
        if before.errors {
            return Err(format!(
                "Snapshot {} contém ERROR/MISSING",
                snapshot.path.display()
            ));
        }
        files.push(File {
            snapshot,
            language,
            before,
            after: Facts::default(),
            edits: Vec::new(),
        });
    }
    let Some(fi) = files.iter().position(|f| f.snapshot.path == origin_path) else {
        return Ok(None);
    };
    let source = &files[fi].snapshot.before;
    let start = incremental::byte_offset(source, origin.start_line, origin.start_character)
        .ok_or("Origem UTF-16 inválida")?;
    let end = incremental::byte_offset(source, origin.end_line, origin.end_character)
        .ok_or("Fim da origem UTF-16 inválido")?;
    if end < start {
        return Err("Intervalo de origem invertido".into());
    }
    let Some(ti) = files[fi].before.tokens.iter().position(|t| {
        t.range.start <= start && end <= t.range.end && (start < t.range.end || start == end)
    }) else {
        return Ok(None);
    };
    let bindings = resolve(&files, false, &mut budget)
        .map_err(|k| format!("Resolução global interrompida: {k:?}"))?;
    let Resolved {
        binding: Binding::Global(df, dt),
        changes_name: true,
    } = bindings[fi][ti]
    else {
        return Ok(None);
    };
    if !matches!(
        files[df].before.tokens[dt].role,
        Role::Declaration(_) | Role::Alias(_) | Role::ExportAlias
    ) {
        return Ok(None);
    }
    let mut result = Vec::with_capacity(files.len());
    let mut count = 0usize;
    for (file_id, file) in files.iter().enumerate() {
        let mut ranges = Vec::new();
        for (token_id, token) in file.before.tokens.iter().enumerate() {
            budget
                .node()
                .map_err(|k| format!("Geração AST interrompida: {k:?}"))?;
            if bindings[file_id][token_id]
                == (Resolved {
                    binding: Binding::Global(df, dt),
                    changes_name: true,
                })
            {
                ranges.push(token.range.clone());
            }
        }
        ranges.sort_by_key(|r| r.start);
        ranges.dedup();
        count += ranges.len();
        if count > 1024 {
            return Err("Proposta global excede 1024 edições".into());
        }
        if ranges.windows(2).any(|p| p[0].end > p[1].start) {
            return Err("Índice global produziu intervalos sobrepostos".into());
        }
        let edits = ranges
            .iter()
            .map(|r| crate::RenameEdit {
                range: text_range(&file.snapshot.before, r.clone()),
                new_text: new_name.into(),
            })
            .collect();
        let mut after = file.snapshot.before.to_string();
        for range in ranges.iter().rev() {
            budget
                .check()
                .map_err(|k| format!("Montagem global interrompida: {k:?}"))?;
            after.replace_range(range.clone(), new_name);
        }
        result.push(RenameSnapshot {
            path: file.snapshot.path.clone(),
            before: file.snapshot.before.clone(),
            after: after.into(),
            edits,
        });
    }
    Ok(Some(result))
}
