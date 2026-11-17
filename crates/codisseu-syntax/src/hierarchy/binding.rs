//! Deliberately bounded receiver resolution over the same immutable corpus.
//! No guessed binding: reassignment, unresolved types and structural conversions
//! remain explicit limits. This pass audits actual provider edits; it adds none.
use super::*;
mod receiver_returns;
mod transfers;
pub(super) type Identities = HashMap<(Language, String), Vec<ClassId>>;
type Method = (ClassId, usize);

fn named_type(
    file_index: usize,
    node: Node<'_>,
    name: &str,
    files: &[File<'_>],
    identities: &Identities,
    budget: &mut Budget<'_>,
) -> Result<ClassId, String> {
    let file = &files[file_index];
    let lookup = |key: String| match identities.get(&(file.language, key)) {
        Some(ids) if ids.len() == 1 => Ok(ids[0]),
        _ => Err("Tipo do receptor ausente ou ambíguo no corpus".into()),
    };
    if file.language == Language::Php {
        let (namespace, aliases, problem) = php_context(node, &file.snapshot.before, budget)
            .map_err(|_| "Orçamento excedido ao resolver receptor")?;
        if let Some(problem) = problem {
            return Err(problem);
        }
        let context = ClassName {
            range: node.byte_range(),
            identity: String::new(),
            parents: vec![],
            has_relations: false,
            exported: false,
            problem: None,
            aliases,
            namespace,
        };
        return lookup(php_parent(&context, name)?);
    }
    let local_key = if file.metadata.module {
        format!("{}::{name}", file.snapshot.path.display())
    } else {
        format!("global::{name}")
    };
    let Some(imports) = file.metadata.imports.get(name) else {
        return lookup(local_key);
    };
    if identities.contains_key(&(file.language, local_key))
        || imports.len() != 1
        || !imports[0].valid
    {
        return Err("Import do receptor não é direto, relativo e inequívoco".into());
    }
    let import = &imports[0];
    let path = normalize(
        &file
            .snapshot
            .path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&import.source),
    )
    .ok_or("Caminho de import do receptor inválido")?;
    let candidates = match path.extension().and_then(|s| s.to_str()) {
        Some("ts") => vec![path],
        Some("js") => vec![path.with_extension("ts")],
        None => vec![path.with_extension("ts"), path.join("index.ts")],
        _ => return Err("Extensão de import do receptor não suportada".into()),
    };
    let mut found = vec![];
    for (i, f) in files.iter().enumerate() {
        budget
            .node()
            .map_err(|_| "Orçamento excedido ao resolver import")?;
        if f.language != file.language
            || !candidates
                .iter()
                .any(|p| normalize(&f.snapshot.path).as_ref() == Some(p))
        {
            continue;
        }
        for (j, c) in f.before.classes.iter().enumerate() {
            budget
                .node()
                .map_err(|_| "Orçamento excedido ao resolver export")?;
            if f.metadata
                .classes
                .get(&c.start)
                .is_some_and(|m| m.exported && m.identity == import.imported)
            {
                found.push(ClassId { file: i, class: j });
            }
        }
    }
    if found.len() == 1 {
        Ok(found[0])
    } else {
        Err("Export do receptor ausente/ambíguo; reexports não resolvidos".into())
    }
}
fn function(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "method_declaration"
            | "anonymous_function"
            | "arrow_function"
            | "function_declaration"
            | "function_expression"
            | "method_definition"
            | "generator_function_declaration"
    )
}
fn lexical(kind: &str) -> bool {
    function(kind)
        || matches!(
            kind,
            "program"
                | "statement_block"
                | "compound_statement"
                | "for_statement"
                | "for_in_statement"
                | "catch_clause"
        )
}
fn enclosing_class(node: Node<'_>, file: usize, files: &[File<'_>]) -> Option<ClassId> {
    let mut current = Some(node);
    while let Some(n) = current {
        if class_kind(n.kind(), files[file].language) {
            return files[file]
                .before
                .classes
                .iter()
                .position(|c| c.start == n.start_byte())
                .map(|class| ClassId { file, class });
        }
        if matches!(n.kind(), "object" | "object_pattern") {
            return None;
        }
        // `this` in a nested ordinary function has an independent binding.
        if matches!(
            n.kind(),
            "function_expression" | "function_declaration" | "anonymous_function"
        ) {
            return None;
        }
        current = n.parent();
    }
    None
}
fn constructor(node: Node<'_>) -> Option<Node<'_>> {
    match node.kind() {
        "new_expression" if node.child_by_field_name("type_arguments").is_none() => {
            node.child_by_field_name("constructor")
        }
        "object_creation_expression" => node
            .named_child(0)
            .filter(|n| matches!(n.kind(), "name" | "qualified_name" | "relative_name")),
        _ => None,
    }
}
fn type_node(node: Node<'_>) -> Option<Node<'_>> {
    if matches!(node.kind(), "type_annotation" | "named_type") && node.named_child_count() == 1 {
        return type_node(node.named_child(0)?);
    }
    matches!(
        node.kind(),
        "name" | "qualified_name" | "relative_name" | "type_identifier" | "identifier"
    )
    .then_some(node)
}
// Scan just one lexical environment. Nested closures do not supply declarations
// to their caller. Assignment in a conditional/loop is never a stable inference.
fn candidates<'a>(
    root: Node<'a>,
    name: &str,
    source: &str,
    language: Language,
    budget: &mut Budget<'_>,
) -> Result<(Vec<Node<'a>>, bool), String> {
    let mut declarations = vec![];
    let mut mutated = false;
    let mut pending = vec![(root, false)];
    while let Some((node, conditional)) = pending.pop() {
        budget
            .node()
            .map_err(|_| "Orçamento excedido ao resolver variável")?;
        if node.id() != root.id()
            && (function(node.kind())
                || class_kind(node.kind(), language)
                || (language == Language::TypeScript && lexical(node.kind())))
        {
            continue;
        }
        let declarator = matches!(
            node.kind(),
            "simple_parameter"
                | "required_parameter"
                | "optional_parameter"
                | "variable_declarator"
        );
        let parameter_name = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("pattern"));
        if declarator && parameter_name.is_some_and(|n| text(n, source) == name) {
            declarations.push(node);
            if conditional || node.child_by_field_name("reference_modifier").is_some() {
                mutated = true;
            }
        }
        if matches!(
            node.kind(),
            "assignment_expression"
                | "augmented_assignment_expression"
                | "update_expression"
                | "reference_assignment_expression"
        ) {
            let left = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"));
            if left.is_some_and(|n| text(n, source) == name) {
                if language == Language::Php && node.kind() == "assignment_expression" {
                    declarations.push(node);
                    if conditional {
                        mutated = true;
                    }
                } else {
                    mutated = true;
                }
            }
        }
        // Dynamic variable names, eval/global/reference aliasing destroy the
        // local inference even when their textual spelling differs.
        if matches!(
            node.kind(),
            "dynamic_variable_name"
                | "global_declaration"
                | "by_ref"
                | "reference_assignment_expression"
        ) {
            mutated = true;
        }
        if matches!(node.kind(), "function_call_expression" | "call_expression")
            && node.child_by_field_name("function").is_some_and(|f| {
                text(f, source)
                    .trim_start_matches('\\')
                    .eq_ignore_ascii_case("eval")
            })
        {
            mutated = true;
        }
        let conditional = conditional
            || matches!(
                node.kind(),
                "if_statement"
                    | "for_statement"
                    | "for_in_statement"
                    | "foreach_statement"
                    | "while_statement"
                    | "do_statement"
                    | "try_statement"
                    | "switch_statement"
            );
        let mut c = node.walk();
        pending.extend(node.named_children(&mut c).map(|n| (n, conditional)));
    }
    Ok((declarations, mutated))
}
fn hidden_mutation(
    root: Node<'_>,
    declaration: Node<'_>,
    name: &str,
    source: &str,
    language: Language,
    use_offset: usize,
    budget: &mut Budget<'_>,
) -> Result<bool, String> {
    let mut pending = vec![root];
    while let Some(n) = pending.pop() {
        budget
            .node()
            .map_err(|_| "Orçamento excedido ao conferir atribuições")?;
        if language == Language::Php
            && n.kind() == "function_call_expression"
            && n.start_byte() < use_offset
            && let Some(args) = n.child_by_field_name("arguments")
        {
            let mut c = args.walk();
            for arg in args.named_children(&mut c) {
                budget
                    .node()
                    .map_err(|_| "Orçamento excedido ao conferir argumento PHP")?;
                let mut ac = arg.walk();
                if arg
                    .named_children(&mut ac)
                    .any(|v| v.kind() == "variable_name" && text(v, source) == name)
                {
                    return Ok(true);
                }
            }
        }
        if n.id() != declaration.id()
            && matches!(
                n.kind(),
                "assignment_expression"
                    | "augmented_assignment_expression"
                    | "update_expression"
                    | "reference_assignment_expression"
            )
            && let Some(left) = n
                .child_by_field_name("left")
                .or_else(|| n.child_by_field_name("argument"))
        {
            let mut parts = vec![left];
            while let Some(p) = parts.pop() {
                budget
                    .node()
                    .map_err(|_| "Orçamento excedido ao verificar destino de atribuição")?;
                if text(p, source) == name {
                    return Ok(true);
                }
                let mut c = p.walk();
                parts.extend(p.named_children(&mut c));
            }
        }
        let mut c = n.walk();
        pending.extend(n.named_children(&mut c));
    }
    Ok(false)
}
fn variable_type(
    file: usize,
    node: Node<'_>,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<ClassId, String> {
    let source = &files[file].snapshot.before;
    let language = files[file].language;
    let name = text(node, source);
    let mut current = node;
    while let Some(parent) = current.parent() {
        budget
            .node()
            .map_err(|_| "Orçamento excedido ao localizar escopo")?;
        current = parent;
        let scope = if language == Language::Php {
            function(current.kind()) || current.kind() == "program"
        } else {
            lexical(current.kind())
        };
        if !scope {
            continue;
        }
        let (decls, mutated) = candidates(current, name, source, language, budget)?;
        if mutated {
            return Err(
                "Receptor com atribuição/alias/controle de fluxo exige análise adicional".into(),
            );
        }
        if decls.is_empty() {
            if function(current.kind()) {
                return Err("Receptor capturado ou sem tipo local explícito".into());
            }
            continue;
        }
        if decls.len() != 1 {
            return Err("Receptor possui múltiplas declarações ou atribuições".into());
        }
        let decl = decls[0];
        if hidden_mutation(
            current,
            decl,
            name,
            source,
            language,
            node.start_byte(),
            budget,
        )? {
            return Err("Receptor pode ser reatribuído em bloco ou closure".into());
        }
        let parameter = matches!(
            decl.kind(),
            "simple_parameter" | "required_parameter" | "optional_parameter"
        );
        if !parameter && decl.start_byte() >= node.start_byte() {
            return Err("Receptor usado antes de sua inicialização".into());
        }
        if decl.kind() == "optional_parameter" {
            return Err("Parâmetro receptor opcional exige prova de não nulidade".into());
        }
        let annotation = decl.child_by_field_name("type");
        let value = decl
            .child_by_field_name("value")
            .or_else(|| decl.child_by_field_name("default_value"))
            .or_else(|| decl.child_by_field_name("right"));
        let typed = annotation
            .map(|a| {
                type_node(a)
                    .ok_or_else(|| "Tipo composto/genérico do receptor não suportado".to_owned())
                    .and_then(|a| named_type(file, a, text(a, source), files, identities, budget))
            })
            .transpose()?;
        let initialized = value
            .map(|value| {
                receiver(file, value, files, identities, parents, budget, depth + 1).and_then(
                    |(id, is_static)| {
                        if is_static {
                            Err("Inicializador é classe estática, não instância".into())
                        } else {
                            Ok(id)
                        }
                    },
                )
            })
            .transpose()?;
        if let (Some(a), Some(b)) = (typed, initialized)
            && a != b
        {
            // A structural assignment is not a nominal override. Silently
            // renaming only the interface can introduce TS2741.
            if !ancestry(b, parents, false, budget)
                .map_err(|_| "Orçamento excedido na conversão do receptor")?
                .contains(&a)
            {
                return Err("Conversão entre tipos distintos do receptor exige verificar compatibilidade estrutural".into());
            }
        }
        return typed
            .or(initialized)
            .ok_or_else(|| "Receptor sem tipo nominal ou construção direta conhecida".into());
    }
    Err("Escopo do receptor não resolvido".into())
}
pub(super) fn receiver(
    file: usize,
    node: Node<'_>,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<(ClassId, bool), String> {
    budget
        .node()
        .map_err(|_| "Orçamento excedido ao resolver receptor")?;
    if depth > 16 {
        return Err("Receptor excede profundidade suportada".into());
    }
    let source = &files[file].snapshot.before;
    if node.kind() == "parenthesized_expression" && node.named_child_count() == 1 {
        return receiver(
            file,
            node.named_child(0).unwrap(),
            files,
            identities,
            parents,
            budget,
            depth + 1,
        );
    }
    if (files[file].language == Language::Php
        && matches!(text(node, source), "$this" | "self" | "static" | "parent"))
        || (files[file].language == Language::TypeScript && matches!(node.kind(), "this" | "super"))
    {
        let id = enclosing_class(node, file, files).ok_or("Classe de this/self não resolvida")?;
        let mut ancestor = node;
        let mut static_access = matches!(text(node, source), "self" | "static" | "parent");
        while let Some(p) = ancestor.parent() {
            ancestor = p;
            if matches!(
                p.kind(),
                "method_definition" | "method_declaration" | "public_field_definition"
            ) {
                if files[file].language == Language::TypeScript
                    && let Some(parameters) = p.child_by_field_name("parameters")
                {
                    let mut c = parameters.walk();
                    for parameter in parameters.named_children(&mut c) {
                        budget
                            .node()
                            .map_err(|_| "Orçamento excedido ao verificar this")?;
                        if parameter
                            .child_by_field_name("name")
                            .or_else(|| parameter.child_by_field_name("pattern"))
                            .is_some_and(|n| text(n, source) == "this")
                        {
                            return Err(
                                "Parâmetro explícito this exige resolução adicional do receptor"
                                    .into(),
                            );
                        }
                    }
                }
                static_access = token(p, "static");
                break;
            }
            if class_kind(p.kind(), files[file].language) {
                break;
            }
        }
        if matches!(text(node, source), "parent" | "super") {
            let p = parents
                .get(&id)
                .into_iter()
                .flatten()
                .filter(|(_, k)| *k == RelationKind::ExtendsClass)
                .map(|(p, _)| *p)
                .collect::<Vec<_>>();
            return if p.len() == 1 {
                Ok((p[0], static_access))
            } else {
                Err("Superclass do receptor não resolvida".into())
            };
        }
        return Ok((id, static_access));
    }
    if let Some(n) = constructor(node) {
        return named_type(file, n, text(n, source), files, identities, budget)
            .map(|id| (id, false));
    }
    match node.kind() {
        "variable_name" => variable_type(file, node, files, identities, parents, budget, depth + 1)
            .map(|id| (id, false)),
        "identifier" if files[file].language == Language::TypeScript => {
            // A local declaration shadows a type name. Resolve variables first;
            // class receivers are accepted only when no local binding exists.
            if let Ok(id) = named_type(file, node, text(node, source), files, identities, budget) {
                let mut root = node;
                let mut shadow = false;
                while let Some(p) = root.parent() {
                    root = p;
                    if lexical(root.kind()) {
                        let (decls, mutated) = candidates(
                            root,
                            text(node, source),
                            source,
                            files[file].language,
                            budget,
                        )?;
                        if !decls.is_empty() || mutated {
                            shadow = true;
                            break;
                        }
                    }
                    if function(root.kind()) {
                        break;
                    }
                }
                if !shadow {
                    return Ok((id, true));
                }
            }
            variable_type(file, node, files, identities, parents, budget, depth + 1)
                .map(|id| (id, false))
        }
        "name" | "qualified_name" | "relative_name" => {
            named_type(file, node, text(node, source), files, identities, budget)
                .map(|id| (id, true))
        }
        _ => receiver_returns::value(file, node, files, identities, parents, budget, depth + 1)
            .map(|id| (id, false)),
    }
}
pub(super) fn methods(
    id: ClassId,
    name: &str,
    after: bool,
    static_access: bool,
    files: &[File<'_>],
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<Vec<Method>, IssueKind> {
    let mut pending = vec![id];
    let mut visited = HashSet::new();
    while !pending.is_empty() {
        let mut found = vec![];
        let mut next = vec![];
        for current in pending {
            budget.node()?;
            if !visited.insert(current) {
                continue;
            }
            let f = &files[current.file];
            let facts = if after {
                f.after.as_ref().unwrap_or(&f.before)
            } else {
                &f.before
            };
            for &m in &facts.classes[current.class].members {
                budget.node()?;
                if facts.members[m].method
                    && (f.language == Language::Php
                        || facts.members[m].static_member == static_access)
                    && f.language.key(&facts.members[m].name) == f.language.key(name)
                {
                    found.push((current, m));
                }
            }
            next.extend(parents.get(&current).into_iter().flatten().map(|(p, _)| *p));
        }
        if !found.is_empty() {
            found.sort_by_key(|(id, m)| (id.file, id.class, *m));
            found.dedup();
            return Ok(found);
        }
        pending = next;
    }
    Ok(vec![])
}
fn problem(
    file: &File<'_>,
    range: std::ops::Range<usize>,
    kind: ConflictKind,
    message: &str,
    report: &mut RenameValidation,
) {
    if report.conflicts.len() >= 256 {
        return;
    }
    report.conflicts.push(Conflict {
        path: file.snapshot.path.clone(),
        range: text_range(&file.snapshot.before, range),
        kind,
        message: message.into(),
        related: None,
        container_name: None,
    });
}
pub(super) fn check(
    files: &[File<'_>],
    parents: &Parents,
    identities: &Identities,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let mut targets = HashSet::new();
    let mut names = HashMap::<Language, HashSet<String>>::new();
    for (i, f) in files.iter().enumerate() {
        for (m, member) in f.before.members.iter().enumerate() {
            budget.node()?;
            if member.method && changed(f, m) {
                targets.insert((
                    ClassId {
                        file: i,
                        class: member.class,
                    },
                    m,
                ));
                names
                    .entry(f.language)
                    .or_default()
                    .insert(f.language.key(&member.name));
                if let Some(after) = &f.after {
                    names
                        .entry(f.language)
                        .or_default()
                        .insert(f.language.key(&after.members[m].name));
                }
            }
        }
    }
    if targets.is_empty() {
        return Ok(());
    }
    transfers::check(files, parents, identities, &targets, report, budget)?;
    for (i, file) in files.iter().enumerate() {
        let Some(names) = names.get(&file.language) else {
            continue;
        };
        let source = &file.snapshot.before;
        let mut pending = vec![file.tree.root_node()];
        while let Some(node) = pending.pop() {
            budget.node()?;
            // Bracket lookup and destructuring are references too. Until their
            // complete provider edits and binding are supported, never approve
            // a proposal which can silently strand these occurrences.
            if file.language == Language::TypeScript
                && matches!(node.kind(), "subscript_expression" | "object_pattern")
            {
                let mut parts = vec![node];
                let mut relevant = false;
                while let Some(p) = parts.pop() {
                    budget.node()?;
                    let value = text(p, source).trim_matches(['\'', '"']);
                    if names.contains(&file.language.key(value)) {
                        relevant = true;
                        break;
                    }
                    let mut c = p.walk();
                    parts.extend(p.named_children(&mut c));
                }
                if relevant {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        Some(text_range(source, node.byte_range())),
                        "Acesso computado/desestruturação ao método exige resolução adicional",
                    );
                }
            }
            let pair = match node.kind() {
                "member_expression" => (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ),
                "member_call_expression" | "nullsafe_member_call_expression" => (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("name"),
                ),
                "scoped_call_expression" => (
                    node.child_by_field_name("scope"),
                    node.child_by_field_name("name"),
                ),
                _ => (None, None),
            };
            if let (Some(object), Some(name)) = pair
                && names.contains(&file.language.key(text(name, source)))
            {
                let edited = file.edits.iter().any(|e| e.before == name.byte_range());
                match receiver(i, object, files, identities, parents, budget, 0) {
                    Err(message) => report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        Some(text_range(source, name.byte_range())),
                        message,
                    ),
                    Ok((id, static_access)) => {
                        let bound = methods(
                            id,
                            text(name, source),
                            false,
                            static_access,
                            files,
                            parents,
                            budget,
                        )?;
                        let changed = bound.iter().any(|m| targets.contains(m));
                        if edited && (!changed || bound.iter().any(|m| !targets.contains(m))) {
                            problem(
                                file,
                                name.byte_range(),
                                ConflictKind::BindingChanged,
                                "Proposta inclui referência vinculada a outro método ou receptor sem método comprovado",
                                report,
                            );
                        } else if changed && !edited {
                            problem(
                                file,
                                name.byte_range(),
                                ConflictKind::MissingReference,
                                "Proposta omite referência vinculada ao método renomeado",
                                report,
                            );
                        } else if !edited {
                            let after = methods(
                                id,
                                text(name, source),
                                true,
                                static_access,
                                files,
                                parents,
                                budget,
                            )?;
                            if bound != after {
                                problem(
                                    file,
                                    name.byte_range(),
                                    ConflictKind::BindingChanged,
                                    "Novo nome altera a vinculação de uma referência existente",
                                    report,
                                );
                            }
                        }
                    }
                }
            }
            let mut c = node.walk();
            pending.extend(node.named_children(&mut c));
        }
    }
    report.scope = ValidationScope::KnownReceiverBindings;
    report.note="Verificação local de declarações, contratos nominais e referências com receptor resolvido nos snapshots. Resolve this/self, construção direta e parâmetros de tipo nominal, imports diretos e herança conhecida. Receptores dinâmicos, reatribuições, capturas, conversões estruturais e tipos compostos sem resolução bloqueiam. Não certifica reflexão, código gerado nem dependências fora do corpus.".into();
    Ok(())
}
