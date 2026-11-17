//! TypeScript object transfers are part of a rename proof: a parameter's
//! annotation does not establish the runtime class of its incoming argument.
//! Only direct, unambiguous declared functions and nominal assignments pass.
use super::*;

#[derive(Clone, Copy, Default)]
struct Destination {
    id: Option<ClassId>,
    relevant: bool,
}
struct Context<'a, 'f> {
    files: &'a [File<'f>],
    parents: &'a Parents,
    identities: &'a Identities,
    names: &'a HashSet<String>,
    targets: &'a HashSet<Method>,
}
struct Callable<'a> {
    file: usize,
    node: Node<'a>,
    name: Option<Node<'a>>,
    parameters: Vec<Destination>,
    result: Option<Destination>,
    relevant: bool,
    direct_method: bool,
}
fn issue(file: &File<'_>, node: Node<'_>, message: &str, report: &mut RenameValidation) {
    report.issue(
        &file.snapshot.path,
        IssueKind::UnsupportedEdit,
        Some(text_range(&file.snapshot.before, node.byte_range())),
        message,
    );
}
fn related(
    id: ClassId,
    names: &HashSet<String>,
    files: &[File<'_>],
    parents: &Parents,
    identities: &Identities,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    let mut pending = vec![id];
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        budget.node()?;
        if !visited.insert(id) {
            continue;
        }
        pending.extend(receiver_returns::member_types(
            id, files, identities, budget,
        )?);
        pending.extend(parents.get(&id).into_iter().flatten().map(|(p, _)| *p));
        for name in names {
            budget.node()?;
            for static_access in [false, true] {
                budget.node()?;
                if !methods(id, name, false, static_access, files, parents, budget)?.is_empty() {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
fn changed_source(
    id: ClassId,
    names: &HashSet<String>,
    files: &[File<'_>],
    parents: &Parents,
    identities: &Identities,
    targets: &HashSet<Method>,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    let mut pending = vec![id];
    let mut visited = HashSet::new();
    while let Some(id) = pending.pop() {
        budget.node()?;
        if !visited.insert(id) {
            continue;
        }
        pending.extend(receiver_returns::member_types(
            id, files, identities, budget,
        )?);
        pending.extend(parents.get(&id).into_iter().flatten().map(|(p, _)| *p));
        for name in names {
            budget.node()?;
            for static_access in [false, true] {
                budget.node()?;
                for bound in methods(id, name, false, static_access, files, parents, budget)? {
                    budget.node()?;
                    if targets.contains(&bound) {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}
fn destination(
    file: usize,
    annotation: Node<'_>,
    files: &[File<'_>],
    parents: &Parents,
    identities: &Identities,
    names: &HashSet<String>,
    budget: &mut Budget<'_>,
) -> Result<Destination, IssueKind> {
    budget.node()?;
    let source = &files[file].snapshot.before;
    if let Some(name) = type_node(annotation) {
        let id = named_type(file, name, text(name, source), files, identities, budget).ok();
        budget.check()?;
        return Ok(Destination {
            id,
            relevant: if let Some(id) = id {
                related(id, names, files, parents, identities, budget)?
            } else {
                false
            },
        });
    }
    let mut result = Destination::default();
    let mut pending = vec![annotation];
    while let Some(node) = pending.pop() {
        budget.node()?;
        if matches!(
            node.kind(),
            "type_identifier" | "identifier" | "nested_type_identifier"
        ) {
            if let Ok(id) = named_type(file, node, text(node, source), files, identities, budget) {
                result.relevant |= related(id, names, files, parents, identities, budget)?;
            }
            budget.check()?;
        }
        if node.kind() == "property_identifier" && names.contains(text(node, source)) {
            result.relevant = true;
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(result)
}
fn top_level(mut node: Node<'_>) -> bool {
    if let Some(parent) = node.parent()
        && parent.kind() == "export_statement"
    {
        if token(parent, "default") {
            return false;
        }
        node = parent;
    }
    node.parent().is_some_and(|p| p.kind() == "program")
}
fn function_key(file: usize, name: &str, files: &[File<'_>]) -> String {
    if files[file].metadata.module {
        format!("{file}::{name}")
    } else {
        format!("global::{name}")
    }
}
fn shadowed(
    node: Node<'_>,
    file: usize,
    name: &str,
    files: &[File<'_>],
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        budget.node()?;
        current = parent;
        if lexical(current.kind()) {
            match candidates(
                current,
                name,
                &files[file].snapshot.before,
                Language::TypeScript,
                budget,
            ) {
                Ok((declarations, mutated)) if declarations.is_empty() && !mutated => {}
                _ => {
                    budget.check()?;
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}
// Unlike receiver inference, an annotation remains an assignment constraint
// after mutation. Never infer an exact receiver class from this helper.
fn assignment_destination(
    file: usize,
    left: Node<'_>,
    context: &Context<'_, '_>,
    budget: &mut Budget<'_>,
) -> Result<Destination, IssueKind> {
    let Context {
        files,
        parents,
        identities,
        names,
        ..
    } = context;
    if left.kind() != "identifier" {
        return Ok(Destination::default());
    }
    let source = &files[file].snapshot.before;
    let mut current = left;
    while let Some(parent) = current.parent() {
        budget.node()?;
        current = parent;
        if !lexical(current.kind()) {
            continue;
        }
        let Ok((declarations, _)) = candidates(
            current,
            text(left, source),
            source,
            Language::TypeScript,
            budget,
        ) else {
            budget.check()?;
            return Ok(Destination::default());
        };
        if declarations.is_empty() {
            continue;
        }
        let mut result = Destination::default();
        for declaration in &declarations {
            budget.node()?;
            let info = if let Some(annotation) = declaration.child_by_field_name("type") {
                destination(file, annotation, files, parents, identities, names, budget)?
            } else if let Some(value) = declaration.child_by_field_name("value") {
                if let Ok((id, false)) =
                    receiver(file, value, files, identities, parents, budget, 0)
                {
                    Destination {
                        id: Some(id),
                        relevant: related(id, names, files, parents, identities, budget)?,
                    }
                } else {
                    budget.check()?;
                    Destination::default()
                }
            } else {
                Destination::default()
            };
            if declarations.len() == 1 {
                return Ok(info);
            }
            result.relevant |= info.relevant;
        }
        return Ok(result);
    }
    Ok(Destination::default())
}
fn conversion(
    file: usize,
    value: Option<Node<'_>>,
    destination: Destination,
    site: Node<'_>,
    context: &Context<'_, '_>,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let Context {
        files,
        parents,
        identities,
        names,
        targets,
    } = context;
    budget.node()?;
    let actual = value.map(|node| receiver(file, node, files, identities, parents, budget, 0));
    budget.check()?;
    let source_changed = if let Some(Ok((id, _))) = actual.as_ref() {
        changed_source(*id, names, files, parents, identities, targets, budget)?
    } else {
        false
    };
    if !destination.relevant && !source_changed {
        return Ok(());
    }
    match (destination.id, actual) {
        (Some(expected), Some(Ok((actual, false)))) if expected == actual => {}
        (Some(expected), Some(Ok((actual, false)))) => {
            if !ancestry(actual, parents, false, budget)?.contains(&expected) {
                issue(
                    &files[file],
                    site,
                    "Transferência estrutural TypeScript entre tipos sem relação nominal comprovada exige verificar argumentos/retornos/inicializadores",
                    report,
                );
            }
        }
        _ => issue(
            &files[file],
            site,
            "Origem ou destino de transferência TypeScript do tipo afetado não resolvido; argumentos/retornos/inicializadores não podem receber binding presumido",
            report,
        ),
    }
    Ok(())
}
fn import_source_matches(file: &File<'_>, statement: Node<'_>, target: &File<'_>) -> bool {
    let Some(source) = statement.child_by_field_name("source") else {
        return false;
    };
    let source = text(source, &file.snapshot.before).trim_matches(['\'', '"']);
    if !source.starts_with('.') {
        return true;
    } // tsconfig/path aliases are not resolved here.
    let Some(path) = normalize(
        &file
            .snapshot
            .path
            .parent()
            .unwrap_or(Path::new(""))
            .join(source),
    ) else {
        return true;
    };
    let Some(target) = normalize(&target.snapshot.path) else {
        return true;
    };
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") => path == target,
        Some("js") => path.with_extension("ts") == target,
        None => path.with_extension("ts") == target || path.join("index.ts") == target,
        _ => false,
    }
}

pub(super) fn check(
    files: &[File<'_>],
    parents: &Parents,
    identities: &Identities,
    targets: &HashSet<Method>,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let mut names = HashSet::new();
    for &(id, member) in targets {
        budget.node()?;
        if files[id.file].language == Language::TypeScript {
            names.insert(files[id.file].before.members[member].name.clone());
            if let Some(after) = &files[id.file].after {
                names.insert(after.members[member].name.clone());
            }
        }
    }
    if names.is_empty() {
        return Ok(());
    }
    let mut callables = Vec::new();
    let mut lookup = HashMap::<String, Vec<usize>>::new();
    let mut callable_index = HashMap::new();
    // Build declarations first so declaration order never substitutes for binding.
    for (file_index, file) in files.iter().enumerate() {
        if file.language != Language::TypeScript {
            continue;
        }
        let source = &file.snapshot.before;
        let mut pending = vec![file.tree.root_node()];
        while let Some(node) = pending.pop() {
            budget.node()?;
            if matches!(node.kind(), "type_alias_declaration" | "type_parameters") {
                let annotation = node.child_by_field_name("value").unwrap_or(node);
                if destination(
                    file_index, annotation, files, parents, identities, &names, budget,
                )?
                .relevant
                {
                    issue(
                        file,
                        node,
                        "Alias/generic TypeScript ligado ao método afetado exige resolução estrutural adicional",
                        report,
                    );
                }
            }
            if function(node.kind())
                || matches!(
                    node.kind(),
                    "function_signature"
                        | "method_signature"
                        | "abstract_method_signature"
                        | "constructor_type"
                )
            {
                let mut parameters = Vec::new();
                let mut uncertain_parameters = false;
                if let Some(list) = node.child_by_field_name("parameters") {
                    let mut cursor = list.walk();
                    for parameter in list.named_children(&mut cursor) {
                        budget.node()?;
                        let info = match parameter.child_by_field_name("type") {
                            Some(annotation) => destination(
                                file_index, annotation, files, parents, identities, &names, budget,
                            )?,
                            None => Destination::default(),
                        };
                        let name = parameter
                            .child_by_field_name("pattern")
                            .or_else(|| parameter.child_by_field_name("name"));
                        uncertain_parameters |= parameter.kind() != "required_parameter"
                            || parameter.child_by_field_name("value").is_some()
                            || name.is_none_or(|n| n.kind() != "identifier")
                            || parameter.child_by_field_name("decorator").is_some();
                        parameters.push(info);
                    }
                } else if node.kind() == "arrow_function" {
                    parameters.push(Destination::default());
                }
                let result = match node.child_by_field_name("return_type") {
                    Some(annotation) => Some(destination(
                        file_index, annotation, files, parents, identities, &names, budget,
                    )?),
                    None => None,
                };
                let relevant =
                    parameters.iter().any(|p| p.relevant) || result.is_some_and(|r| r.relevant);
                let name = node.child_by_field_name("name");
                let direct = node.kind() == "function_declaration"
                    && top_level(node)
                    && node.child_by_field_name("body").is_some()
                    && name.is_some_and(|n| n.kind() == "identifier")
                    && node.child_by_field_name("type_parameters").is_none()
                    && !uncertain_parameters;
                let direct_method = node.kind() == "method_definition"
                    && node.child_by_field_name("body").is_some()
                    && node.child_by_field_name("type_parameters").is_none()
                    && name.is_some_and(|n| {
                        n.kind() == "property_identifier" && text(n, source) != "constructor"
                    })
                    && !uncertain_parameters
                    && !token(node, "async")
                    && !token(node, "get")
                    && !token(node, "set")
                    && !token(node, "*")
                    && !token(node, "?")
                    && enclosing_class(node, file_index, files).is_some();
                let index = callables.len();
                callable_index.insert((file_index, node.id()), index);
                if direct && let Some(name) = name {
                    lookup
                        .entry(function_key(file_index, text(name, source), files))
                        .or_default()
                        .push(index);
                } else if relevant && !direct_method {
                    issue(
                        file,
                        node,
                        "Assinatura TS afetada de método/closure/overload/default/rest ou função não direta exige resolução adicional dos valores transferidos",
                        report,
                    );
                }
                callables.push(Callable {
                    file: file_index,
                    node,
                    name,
                    parameters,
                    result,
                    relevant,
                    direct_method,
                });
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
    }
    for declarations in lookup.values() {
        budget.node()?;
        if declarations.len() > 1 {
            for &index in declarations {
                budget.node()?;
                let callable = &callables[index];
                if callable.relevant {
                    issue(
                        &files[callable.file],
                        callable.node,
                        "Nome de função TS afetada é ambíguo/duplicado no escopo nominal",
                        report,
                    );
                }
            }
        }
    }
    let context = Context {
        files,
        parents,
        identities,
        names: &names,
        targets,
    };
    let mut method_transfer_names = HashSet::new();
    for callable in &callables {
        budget.node()?;
        if callable.relevant
            && callable.direct_method
            && let Some(name) = callable.name
        {
            method_transfer_names
                .insert(text(name, &files[callable.file].snapshot.before).to_owned());
        }
    }
    for (file_index, file) in files.iter().enumerate() {
        if file.language != Language::TypeScript {
            continue;
        }
        let source = &file.snapshot.before;
        let mut pending = vec![file.tree.root_node()];
        while let Some(node) = pending.pop() {
            budget.node()?;
            if matches!(node.kind(), "import_statement" | "export_statement")
                && node.child_by_field_name("source").is_some()
            {
                for callable in &callables {
                    budget.node()?;
                    if callable.parameters.iter().any(|p| p.relevant)
                        && callable.file != file_index
                        && import_source_matches(file, node, &files[callable.file])
                    {
                        issue(
                            file,
                            node,
                            "Import/reexport de função TS com transferência afetada exige resolver aliases e chamadas entre módulos",
                            report,
                        );
                        break;
                    }
                }
            }
            if node.kind() == "identifier"
                && let Some(indices) =
                    lookup.get(&function_key(file_index, text(node, source), files))
            {
                for &index in indices {
                    budget.node()?;
                    let callable = &callables[index];
                    if !callable.relevant || callable.name.is_some_and(|n| n.id() == node.id()) {
                        continue;
                    }
                    let direct_call = node.parent().is_some_and(|p| {
                        p.kind() == "call_expression"
                            && p.child_by_field_name("function")
                                .is_some_and(|n| n.id() == node.id())
                    });
                    if !direct_call
                        || shadowed(node, file_index, text(node, source), files, budget)?
                    {
                        issue(
                            file,
                            node,
                            "Função TS com tipo afetado escapa como valor, tem alias ou é sombreada; transferências não comprovadas",
                            report,
                        );
                    }
                }
            }
            if node.kind() == "call_expression" {
                let function = node.child_by_field_name("function");
                let mut resolved = if let Some(name) = function.filter(|n| n.kind() == "identifier")
                {
                    let key = function_key(file_index, text(name, source), files);
                    if let Some(indices) = lookup.get(&key).filter(|v| v.len() == 1) {
                        if !shadowed(name, file_index, text(name, source), files, budget)? {
                            Some(&callables[indices[0]])
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };
                if resolved.is_none()
                    && let Ok((declaration_file, declaration)) = receiver_returns::callable(
                        file_index, node, files, identities, parents, budget, 0,
                    )
                    && let Some(&index) = callable_index.get(&(declaration_file, declaration.id()))
                {
                    let callable = &callables[index];
                    if callable.direct_method
                        || (callable.node.kind() == "function_declaration"
                            && top_level(callable.node))
                    {
                        resolved = Some(callable);
                    }
                }
                budget.check()?;
                if let Some(arguments) = node.child_by_field_name("arguments") {
                    let mut cursor = arguments.walk();
                    let mut args = Vec::new();
                    for argument in arguments.named_children(&mut cursor) {
                        budget.node()?;
                        if argument.kind() != "comment" {
                            args.push(argument);
                        }
                    }
                    let count = args.len().max(resolved.map_or(0, |f| f.parameters.len()));
                    for index in 0..count {
                        budget.node()?;
                        let destination = resolved
                            .and_then(|f| f.parameters.get(index))
                            .copied()
                            .unwrap_or_default();
                        let value = args.get(index).copied();
                        conversion(
                            file_index,
                            value,
                            destination,
                            value.unwrap_or(node),
                            &context,
                            report,
                            budget,
                        )?;
                    }
                }
            }
            // A method carrying an affected object can escape as a callback.
            // Its parameters would then bypass the direct-call transfer checks.
            if node.kind() == "member_expression"
                && let Some(name) = node.child_by_field_name("property")
                && method_transfer_names.contains(text(name, source))
            {
                let direct_call = node.parent().is_some_and(|p| {
                    p.kind() == "call_expression"
                        && p.child_by_field_name("function")
                            .is_some_and(|f| f.id() == node.id())
                });
                if !direct_call {
                    issue(
                        file,
                        node,
                        "Método TS com transferência afetada escapa como callback; chamadas indiretas não comprovadas",
                        report,
                    );
                }
            }
            if node.kind() == "subscript_expression" && !method_transfer_names.is_empty() {
                issue(
                    file,
                    node,
                    "Acesso computado pode ocultar método com transferência afetada",
                    report,
                );
            }
            if node.kind() == "assignment_expression"
                && let Some(left) = node.child_by_field_name("left")
            {
                let info = assignment_destination(file_index, left, &context, budget)?;
                conversion(
                    file_index,
                    node.child_by_field_name("right"),
                    info,
                    node,
                    &context,
                    report,
                    budget,
                )?;
            }
            if matches!(
                node.kind(),
                "variable_declarator" | "public_field_definition"
            ) && let Some(annotation) = node.child_by_field_name("type")
            {
                let info = destination(
                    file_index, annotation, files, parents, identities, &names, budget,
                )?;
                conversion(
                    file_index,
                    node.child_by_field_name("value"),
                    info,
                    node,
                    &context,
                    report,
                    budget,
                )?;
            }
            if node.kind() == "return_statement" {
                let mut parent = node.parent();
                while let Some(callable_node) = parent {
                    budget.node()?;
                    if function(callable_node.kind()) {
                        if let Some(&index) = callable_index.get(&(file_index, callable_node.id()))
                        {
                            let callable = &callables[index];
                            let mut cursor = node.walk();
                            let value = node
                                .named_children(&mut cursor)
                                .find(|n| n.kind() != "comment");
                            if let Some(result) = callable.result {
                                conversion(
                                    file_index, value, result, node, &context, report, budget,
                                )?;
                            }
                        }
                        break;
                    }
                    parent = callable_node.parent();
                }
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
    }
    Ok(())
}
