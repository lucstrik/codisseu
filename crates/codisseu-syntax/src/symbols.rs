//! Lexical binding verification for exact provider proposals. This is deliberately
//! independent of provider reference discovery and never manufactures an edit.
use crate::{
    Conflict, ConflictKind, ConflictLocation, Coverage, IssueKind, RenameSnapshot,
    RenameValidation, ValidationLimits, ValidationScope,
    ast::Language,
    incremental::{self, Budget, ByteEdit, text_range},
};
use std::{collections::HashMap, ops::Range, sync::atomic::AtomicBool};
use tree_sitter::{Node, Parser};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Declaration,
    Reference,
    OtherSymbol,
}
#[derive(Debug)]
struct Token {
    range: Range<usize>,
    name: String,
    scope: usize,
    role: Role,
    parameter: bool,
    binding: Option<usize>,
}
#[derive(Debug)]
struct Scope {
    parent: Option<usize>,
    function: Option<usize>,
    range: Range<usize>,
}
#[derive(Debug, Default)]
struct Facts {
    scopes: Vec<Scope>,
    tokens: Vec<Token>,
    hazards: Vec<(Range<usize>, &'static str)>,
    errors: Vec<Range<usize>>,
    named_arguments: Vec<Range<usize>>,
    duplicate_declarations: Vec<(usize, usize)>,
}
struct File<'a> {
    snapshot: &'a RenameSnapshot,
    language: Language,
    before: Facts,
    after: Facts,
    edits: Vec<ByteEdit>,
}
fn field_is(parent: Node<'_>, field: &str, node: Node<'_>) -> bool {
    parent
        .child_by_field_name(field)
        .is_some_and(|n| n.id() == node.id())
}
fn function_node(kind: &str, language: Language) -> bool {
    match language {
        Language::Php => matches!(
            kind,
            "function_definition" | "method_declaration" | "anonymous_function" | "arrow_function"
        ),
        Language::TypeScript => matches!(
            kind,
            "function_declaration"
                | "function_expression"
                | "generator_function_declaration"
                | "generator_function"
                | "method_definition"
                | "arrow_function"
        ),
    }
}
fn source<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    &text[node.byte_range()]
}
fn inspect(
    root: Node<'_>,
    text: &str,
    language: Language,
    budget: &mut Budget<'_>,
) -> Result<Facts, IssueKind> {
    let mut facts = Facts::default();
    facts.scopes.push(Scope {
        parent: None,
        function: None,
        range: root.byte_range(),
    });
    let mut stack = vec![(root, 0usize)];
    while let Some((node, inherited_scope)) = stack.pop() {
        budget.node()?;
        let kind = node.kind();
        if (node.is_error() || node.is_missing()) && facts.errors.len() < 32 {
            facts.errors.push(node.byte_range());
        }
        let callable = function_node(kind, language);
        let parent_is_body = node.parent().is_some_and(|p| {
            (function_node(p.kind(), language) || p.kind() == "catch_clause")
                && field_is(p, "body", node)
        });
        let lexical = language == Language::TypeScript
            && (matches!(
                kind,
                "catch_clause" | "for_statement" | "for_in_statement" | "switch_body"
            ) || (kind == "statement_block" && !parent_is_body));
        let scope = if callable || lexical {
            let id = facts.scopes.len();
            facts.scopes.push(Scope {
                parent: Some(inherited_scope),
                function: if callable {
                    Some(id)
                } else {
                    facts.scopes[inherited_scope].function
                },
                range: node.byte_range(),
            });
            id
        } else {
            inherited_scope
        };
        if language == Language::Php {
            if matches!(
                kind,
                "dynamic_variable_name"
                    | "global_declaration"
                    | "function_static_declaration"
                    | "anonymous_function"
                    | "anonymous_function_use_clause"
                    | "arrow_function"
                    | "by_ref"
                    | "reference_assignment_expression"
                    | "reference_modifier"
                    | "attribute_list"
                    | "include_expression"
                    | "include_once_expression"
                    | "require_expression"
                    | "require_once_expression"
                    | "eval_intrinsic"
            ) {
                facts.hazards.push((node.byte_range(), "Variáveis dinâmicas, referências, atributos, global/static, closures e inclusão/eval PHP exigem análise adicional"));
            }
            if kind == "function_call_expression"
                && node.child_by_field_name("function").is_some_and(|f| {
                    !matches!(f.kind(), "name" | "qualified_name" | "relative_name")
                })
            {
                facts.hazards.push((
                    node.byte_range(),
                    "Chamada PHP dinâmica pode observar variáveis por nome",
                ));
            }
            if kind == "argument"
                && let Some(name) = node.child_by_field_name("name")
            {
                facts.named_arguments.push(name.byte_range());
            }
            if kind == "function_call_expression"
                && let Some(function) = node.child_by_field_name("function")
                && matches!(
                    source(function, text)
                        .rsplit('\\')
                        .next()
                        .unwrap_or("")
                        .to_ascii_lowercase()
                        .as_str(),
                    "compact" | "extract" | "get_defined_vars" | "parse_str" | "eval" | "assert"
                )
            {
                facts.hazards.push((
                    node.byte_range(),
                    "Função PHP pode observar ou modificar variáveis por nome",
                ));
            }
            if kind == "variable_name" {
                let mut cursor = node.walk();
                if let Some(name) = node
                    .named_children(&mut cursor)
                    .find(|n| n.kind() == "name")
                {
                    let parameter = node.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            "simple_parameter"
                                | "variadic_parameter"
                                | "property_promotion_parameter"
                        )
                    });
                    if node
                        .parent()
                        .is_some_and(|p| p.kind() == "property_promotion_parameter")
                    {
                        facts.hazards.push((
                            node.byte_range(),
                            "Parâmetro promovido também declara propriedade PHP",
                        ));
                    }
                    let property = node.parent().is_some_and(|p| {
                        p.kind() == "property_element"
                            || (p.kind() == "scoped_property_access_expression"
                                && field_is(p, "name", node))
                    });
                    facts.tokens.push(Token {
                        range: name.byte_range(),
                        name: source(name, text).into(),
                        scope,
                        role: if property {
                            Role::OtherSymbol
                        } else {
                            Role::Reference
                        },
                        parameter,
                        binding: None,
                    });
                }
            } else if matches!(
                kind,
                "function_definition"
                    | "class_declaration"
                    | "interface_declaration"
                    | "trait_declaration"
                    | "enum_declaration"
            ) && let Some(name) = node.child_by_field_name("name")
            {
                facts.tokens.push(Token {
                    range: name.byte_range(),
                    name: source(name, text).into(),
                    scope: inherited_scope,
                    role: Role::OtherSymbol,
                    parameter: false,
                    binding: None,
                });
            }
        } else {
            if matches!(
                kind,
                "variable_declaration"
                    | "object_pattern"
                    | "array_pattern"
                    | "rest_pattern"
                    | "shorthand_property_identifier"
                    | "shorthand_property_identifier_pattern"
                    | "with_statement"
                    | "internal_module"
                    | "type_alias_declaration"
                    | "type_query"
                    | "class_declaration"
                    | "class"
                    | "decorator"
                    | "accessibility_modifier"
                    | "function_signature"
            ) {
                facts.hazards.push((node.byte_range(), "var, destructuring/rest, shorthand, tipos/namespace/classes locais ou decorators exigem análise adicional"));
            }
            if kind == "call_expression"
                && node
                    .child_by_field_name("function")
                    .is_some_and(|f| source(f, text) == "eval")
            {
                facts.hazards.push((
                    node.byte_range(),
                    "eval pode observar variáveis TypeScript por nome",
                ));
            }
            if matches!(kind, "required_parameter" | "optional_parameter")
                && node.child_by_field_name("value").is_some()
            {
                facts.hazards.push((
                    node.byte_range(),
                    "Defaults de parâmetro TypeScript têm ambiente lexical próprio",
                ));
            }
            if kind == "identifier" && source(node, text).contains('\\') {
                facts.hazards.push((
                    node.byte_range(),
                    "Identificadores TypeScript com escape exigem normalização lexical adicional",
                ));
            }
            if kind == "identifier" && source(node, text) == "eval" {
                facts.hazards.push((
                    node.byte_range(),
                    "eval pode observar variáveis TypeScript por nome",
                ));
            }
            if kind == "identifier" {
                let mut token_scope = scope;
                let mut role = Role::Reference;
                let mut parameter = false;
                if let Some(parent) = node.parent() {
                    let pk = parent.kind();
                    if pk == "variable_declarator" && field_is(parent, "name", node) {
                        role = Role::Declaration;
                    } else if (matches!(pk, "required_parameter" | "optional_parameter")
                        && (field_is(parent, "pattern", node) || field_is(parent, "name", node)))
                        || (pk == "arrow_function" && field_is(parent, "parameter", node))
                        || (pk == "catch_clause" && field_is(parent, "parameter", node))
                    {
                        role = Role::Declaration;
                        parameter = true;
                    } else if function_node(pk, language) && field_is(parent, "name", node) {
                        role = Role::OtherSymbol;
                        if matches!(
                            pk,
                            "function_declaration" | "generator_function_declaration"
                        ) {
                            token_scope = facts.scopes[scope].parent.unwrap_or(0);
                        }
                    } else if matches!(
                        pk,
                        "import_specifier"
                            | "import_clause"
                            | "namespace_import"
                            | "import_alias"
                            | "export_specifier"
                            | "function_signature"
                            | "enum_declaration"
                    ) {
                        role = Role::OtherSymbol;
                    }
                }
                facts.tokens.push(Token {
                    range: node.byte_range(),
                    name: source(node, text).into(),
                    scope: token_scope,
                    role,
                    parameter,
                    binding: None,
                });
            } else if matches!(
                kind,
                "class_declaration"
                    | "abstract_class_declaration"
                    | "interface_declaration"
                    | "type_alias_declaration"
                    | "enum_declaration"
            ) && let Some(name) = node.child_by_field_name("name")
                && name.kind() != "identifier"
            {
                facts.tokens.push(Token {
                    range: name.byte_range(),
                    name: source(name, text).into(),
                    scope,
                    role: Role::OtherSymbol,
                    parameter: false,
                    binding: None,
                });
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor).map(|child| (child, scope)));
    }
    facts.tokens.sort_by_key(|t| t.range.start);
    let mut declarations: HashMap<(usize, String), usize> = HashMap::new();
    for index in 0..facts.tokens.len() {
        budget.node()?;
        let token = &facts.tokens[index];
        let declare = if language == Language::Php {
            token.role != Role::OtherSymbol
        } else {
            token.role != Role::Reference
        };
        if !declare {
            continue;
        }
        let key = (token.scope, token.name.clone());
        if let Some(&existing) = declarations.get(&key) {
            if language == Language::TypeScript
                || (token.parameter && facts.tokens[existing].parameter)
            {
                facts.duplicate_declarations.push((existing, index));
            }
        } else {
            declarations.insert(key, index);
        }
    }
    for index in 0..facts.tokens.len() {
        budget.node()?;
        let token = &facts.tokens[index];
        if language == Language::Php && token.role == Role::OtherSymbol {
            continue;
        }
        let mut scope = Some(token.scope);
        let mut binding = None;
        while let Some(id) = scope {
            budget.node()?;
            if let Some(&declaration) = declarations.get(&(id, token.name.clone())) {
                binding = Some(declaration);
                break;
            }
            // PHP ordinary local variables never capture an outer function's locals.
            if language == Language::Php {
                break;
            }
            scope = facts.scopes[id].parent;
        }
        facts.tokens[index].binding = binding;
    }
    Ok(facts)
}
fn issue(
    report: &mut RenameValidation,
    snapshot: &RenameSnapshot,
    kind: IssueKind,
    message: impl Into<String>,
) {
    report.issue(&snapshot.path, kind, None, message);
}
fn conflict(
    report: &mut RenameValidation,
    file: &File<'_>,
    index: usize,
    target: usize,
    kind: ConflictKind,
    message: &str,
) {
    if report.conflicts.len() >= 256 {
        return;
    }
    report.conflicts.push(Conflict {
        path: file.snapshot.path.clone(),
        range: text_range(
            &file.snapshot.before,
            file.before.tokens[index].range.clone(),
        ),
        related: Some(ConflictLocation {
            path: file.snapshot.path.clone(),
            range: text_range(
                &file.snapshot.before,
                file.before.tokens[target].range.clone(),
            ),
        }),
        message: message.into(),
        kind,
        container_name: None,
    });
}
/// `None` means there was no recognized non-method symbol in the proposal and the
/// caller should try its method validator. `Some(Unsupported)` is never permission
/// to apply: it records an exact lexical/snapshot limitation.
pub(crate) fn validate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    let mut report = RenameValidation::new();
    report.scope = ValidationScope::LexicalBindings;
    report.note = "Vínculos lexicais locais PHP/TypeScript nos snapshots fornecidos. Não comprova completude global do provider, módulos, funções/classes, propriedades, reflexão ou chamadas nomeadas fora do corpus.".into();
    let mut budget = Budget::new(cancel, limits);
    let fallback = snapshots
        .first()
        .map_or(std::path::Path::new(""), |s| s.path.as_path());
    if snapshots.is_empty()
        || snapshots.len() > 64
        || snapshots
            .iter()
            .map(|s| s.before.len().saturating_add(s.after.len()))
            .sum::<usize>()
            > 8 * 1024 * 1024
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
    {
        report.issue(fallback, IssueKind::Limit, None, "Validação lexical exige 1..64 snapshots, 1 MiB por texto, 8 MiB por corpus e até 1024 edições");
        return Some(report);
    }
    if let Err(message) = crate::validate_rename_candidate(new_name) {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, message);
        return Some(report);
    }
    let mut files = Vec::new();
    let mut recognized = false;
    let mut paths = std::collections::HashSet::new();
    for snapshot in snapshots {
        if !paths.insert(&snapshot.path) {
            issue(
                &mut report,
                snapshot,
                IssueKind::InvalidSnapshot,
                "Caminho de snapshot duplicado",
            );
            continue;
        }
        if let Err(kind) = budget.check() {
            issue(
                &mut report,
                snapshot,
                kind,
                "Validação lexical cancelada ou orçamento excedido",
            );
            return Some(report);
        }
        let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
            Some("php") => Language::Php,
            Some("ts") => Language::TypeScript,
            _ => {
                if !snapshot.edits.is_empty() {
                    issue(
                        &mut report,
                        snapshot,
                        IssueKind::UnsupportedLanguage,
                        "Validação lexical suporta PHP/TypeScript independentes",
                    );
                }
                continue;
            }
        };
        let edits = match incremental::prepare_edits(snapshot, new_name, &budget) {
            Ok(edits) => edits,
            Err(message) => {
                issue(
                    &mut report,
                    snapshot,
                    budget.check().err().unwrap_or(IssueKind::InvalidSnapshot),
                    message,
                );
                continue;
            }
        };
        let mut parser = Parser::new();
        if parser.set_language(&language.grammar()).is_err() {
            issue(
                &mut report,
                snapshot,
                IssueKind::Parser,
                "Gramática incompatível com runtime",
            );
            continue;
        }
        let Some(mut tree) = incremental::parse(&mut parser, &snapshot.before, None, &budget)
        else {
            issue(
                &mut report,
                snapshot,
                budget.check().err().unwrap_or(IssueKind::Parser),
                "Parser interrompido antes da proposta lexical",
            );
            return Some(report);
        };
        let before = match inspect(tree.root_node(), &snapshot.before, language, &mut budget) {
            Ok(facts) => facts,
            Err(kind) => {
                issue(
                    &mut report,
                    snapshot,
                    kind,
                    "Orçamento excedido durante índice lexical",
                );
                return Some(report);
            }
        };
        recognized |= edits
            .iter()
            .any(|e| before.tokens.iter().any(|t| t.range == e.before));
        report.checked_files += 1;
        if !before.errors.is_empty() {
            issue(
                &mut report,
                snapshot,
                IssueKind::SyntaxBefore,
                "Snapshot anterior contém ERROR/MISSING; vínculos não comprovados",
            );
            continue;
        }
        for edit in edits.iter().rev() {
            tree.edit(&edit.input_edit(&snapshot.before));
        }
        let Some(after_tree) =
            incremental::parse(&mut parser, &snapshot.after, Some(&tree), &budget)
        else {
            issue(
                &mut report,
                snapshot,
                budget.check().err().unwrap_or(IssueKind::Parser),
                "Reparse lexical interrompido",
            );
            return Some(report);
        };
        if !edits.is_empty() {
            report.incremental_reparses += 1;
        }
        let after = match inspect(
            after_tree.root_node(),
            &snapshot.after,
            language,
            &mut budget,
        ) {
            Ok(facts) => facts,
            Err(kind) => {
                issue(
                    &mut report,
                    snapshot,
                    kind,
                    "Orçamento excedido ao inspecionar proposta lexical",
                );
                return Some(report);
            }
        };
        files.push(File {
            snapshot,
            language,
            before,
            after,
            edits,
        });
    }
    if !recognized {
        return None;
    }
    let mut selected = Vec::new();
    for (file_id, file) in files.iter().enumerate() {
        for edit in &file.edits {
            if let Err(kind) = budget.node() {
                report.issue(
                    fallback,
                    kind,
                    None,
                    "Orçamento excedido ao vincular edições",
                );
                return Some(report);
            }
            // Named labels bind to a callee's parameter, not to the caller's
            // local variable scope. The nominal pass below checks them exactly.
            if file.language == Language::Php && file.before.named_arguments.contains(&edit.before)
            {
                continue;
            }
            let Some((index, token)) = file
                .before
                .tokens
                .iter()
                .enumerate()
                .find(|(_, t)| t.range == edit.before)
            else {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Edição não corresponde a identificador AST de variável/parâmetro lexical",
                );
                continue;
            };
            if token.role == Role::OtherSymbol {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Funções, classes, tipos, imports e propriedades exigem resolução própria de símbolos/módulos",
                );
                continue;
            }
            let Some(binding) = token.binding else {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Identificador sem declaração lexical comprovada",
                );
                continue;
            };
            if file.before.scopes[file.before.tokens[binding].scope]
                .function
                .is_none()
            {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Variável global/de módulo exige resolução entre arquivos",
                );
                continue;
            }
            if !selected.contains(&(file_id, binding)) {
                selected.push((file_id, binding));
            }
            if file
                .after
                .tokens
                .get(index)
                .is_none_or(|after| after.range != edit.after || after.name != new_name)
            {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Proposta posterior não preserva um identificador lexical inteiro",
                );
            }
        }
    }
    if selected.len() != 1 {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, "Uma proposta lexical precisa renomear exatamente um vínculo local; homônimos independentes não são relacionados");
    }
    let mut named_anchors = Vec::new();
    for &(file_id, target) in &selected {
        let file = &files[file_id];
        let declaration = &file.before.tokens[target];
        report.checked_declarations += 1;
        let function = file.before.scopes[declaration.scope].function.unwrap_or(0);
        let function_range = &file.before.scopes[function].range;
        for (range, reason) in &file.before.hazards {
            if let Err(kind) = budget.node() {
                report.issue(
                    fallback,
                    kind,
                    None,
                    "Orçamento excedido em limites lexicais",
                );
                return Some(report);
            }
            if range.start >= function_range.start && range.end <= function_range.end {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedEdit,
                    Some(text_range(&file.snapshot.before, range.clone())),
                    *reason,
                );
            }
        }
        let ts_reserved = |name: &str| {
            matches!(
                name,
                "break"
                    | "case"
                    | "catch"
                    | "class"
                    | "const"
                    | "continue"
                    | "debugger"
                    | "default"
                    | "delete"
                    | "do"
                    | "else"
                    | "enum"
                    | "export"
                    | "extends"
                    | "false"
                    | "finally"
                    | "for"
                    | "function"
                    | "if"
                    | "import"
                    | "in"
                    | "instanceof"
                    | "new"
                    | "null"
                    | "return"
                    | "super"
                    | "switch"
                    | "this"
                    | "throw"
                    | "true"
                    | "try"
                    | "typeof"
                    | "var"
                    | "void"
                    | "while"
                    | "with"
                    | "yield"
                    | "let"
                    | "static"
                    | "implements"
                    | "interface"
                    | "package"
                    | "private"
                    | "protected"
                    | "public"
                    | "await"
                    | "eval"
                    | "arguments"
            )
        };
        if file.language == Language::TypeScript
            && (ts_reserved(new_name) || ts_reserved(&declaration.name))
        {
            issue(
                &mut report,
                file.snapshot,
                IssueKind::UnsupportedEdit,
                "Nome de binding TypeScript implícito ou restrito em modo strict",
            );
        }
        if file.language == Language::Php {
            let reserved = |name: &str| {
                matches!(
                    name,
                    "this"
                        | "GLOBALS"
                        | "_SERVER"
                        | "_GET"
                        | "_POST"
                        | "_FILES"
                        | "_COOKIE"
                        | "_SESSION"
                        | "_REQUEST"
                        | "_ENV"
                        | "argc"
                        | "argv"
                )
            };
            if reserved(&declaration.name) || reserved(new_name) || new_name.starts_with('$') {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Variável PHP especial ou edição incluindo $ exige tratamento específico",
                );
            }
            if file
                .before
                .tokens
                .iter()
                .any(|t| t.binding == Some(target) && t.parameter)
            {
                match crate::hierarchy::audit_named_parameter(
                    snapshots,
                    &file.snapshot.path,
                    declaration.range.clone(),
                    new_name,
                    &mut report,
                    &mut budget,
                ) {
                    Ok(anchors) => {
                        named_anchors.extend(anchors);
                        report.note.push_str(" Argumentos nomeados PHP vinculados por declaração AST e receptor nominal; complemento de proposta próprio, independente do Phpactor.");
                    }
                    Err(message) => issue(
                        &mut report,
                        file.snapshot,
                        budget.check().err().unwrap_or(IssueKind::UnsupportedEdit),
                        message,
                    ),
                }
            }
        }
        if !file.after.errors.is_empty() {
            conflict(
                &mut report,
                file,
                target,
                target,
                ConflictKind::InvalidSyntax,
                "A proposta introduz erro sintático",
            );
            continue;
        }
        if file.before.tokens.len() != file.after.tokens.len()
            || file.before.scopes.len() != file.after.scopes.len()
        {
            issue(
                &mut report,
                file.snapshot,
                IssueKind::UnsupportedEdit,
                "A proposta alterou a estrutura lexical AST",
            );
            continue;
        }
        if !file.before.duplicate_declarations.is_empty() {
            issue(
                &mut report,
                file.snapshot,
                IssueKind::UnsupportedEdit,
                "Escopo anterior contém declarações lexicais duplicadas",
            );
            continue;
        }
        for &(a, b) in &file.after.duplicate_declarations {
            if a == target || b == target {
                conflict(
                    &mut report,
                    file,
                    if a == target { b } else { a },
                    target,
                    ConflictKind::DuplicateBinding,
                    "O novo nome já declara outro símbolo no mesmo escopo lexical",
                );
            }
        }
        for (index, (before, after)) in file
            .before
            .tokens
            .iter()
            .zip(&file.after.tokens)
            .enumerate()
        {
            if let Err(kind) = budget.node() {
                report.issue(
                    fallback,
                    kind,
                    None,
                    "Orçamento excedido ao comparar vínculos",
                );
                return Some(report);
            }
            let edited = file.edits.iter().any(|e| e.before == before.range);
            if before.scope != after.scope
                || before.role != after.role
                || before.parameter != after.parameter
            {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "A proposta alterou papel ou escopo de um identificador",
                );
                continue;
            }
            if before.binding == Some(target) && !edited {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Proposta parcial: declaração ou uso do vínculo local não foi renomeado",
                );
            }
            if edited && before.binding != Some(target) {
                issue(
                    &mut report,
                    file.snapshot,
                    IssueKind::UnsupportedEdit,
                    "Proposta inclui identificador de outro vínculo",
                );
            }
            if before.binding != after.binding {
                conflict(
                    &mut report,
                    file,
                    index,
                    target,
                    ConflictKind::BindingChanged,
                    "O novo nome une vínculos distintos ou captura/altera uma referência lexical",
                );
            }
        }
    }
    for file in &files {
        for edit in &file.edits {
            if file.language == Language::Php
                && file.before.named_arguments.contains(&edit.before)
                && !named_anchors
                    .iter()
                    .any(|(path, range)| path == &file.snapshot.path && range == &edit.before)
            {
                report.conflicts.push(Conflict {
                    path: file.snapshot.path.clone(),
                    range: text_range(&file.snapshot.before, edit.before.clone()),
                    related: None,
                    message: "Edição de argumento nomeado não pertence ao parâmetro selecionado"
                        .into(),
                    kind: ConflictKind::BindingChanged,
                    container_name: None,
                });
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
