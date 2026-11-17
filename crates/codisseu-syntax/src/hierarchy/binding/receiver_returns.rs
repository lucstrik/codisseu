//! Nominal receiver sources backed by explicit declarations, never spelling alone.
use super::*;

fn tick(budget: &mut Budget<'_>) -> Result<(), String> {
    budget
        .node()
        .map_err(|k| format!("Resolução de retorno/propriedade interrompida: {k:?}"))
}
fn declaration<'a>(file: &'a File<'_>, range: &std::ops::Range<usize>) -> Result<Node<'a>, String> {
    let mut node = file
        .tree
        .root_node()
        .named_descendant_for_byte_range(range.start, range.end)
        .ok_or("Declaração nominal ausente da AST")?;
    loop {
        if function(node.kind())
            || matches!(
                node.kind(),
                "method_signature" | "abstract_method_signature"
            )
        {
            if node
                .child_by_field_name("name")
                .is_some_and(|n| n.byte_range() == *range)
            {
                return Ok(node);
            }
            return Err("Nome nominal não é declaração da função esperada".into());
        }
        node = node.parent().ok_or("Declaração da função não encontrada")?;
    }
}

pub(super) fn callable<'a>(
    file: usize,
    node: Node<'_>,
    files: &'a [File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<(usize, Node<'a>), String> {
    tick(budget)?;
    if depth > 16 {
        return Err("Cadeia de chamadas excede profundidade suportada".into());
    }
    let (object, name) = match node.kind() {
        "member_call_expression" => (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ),
        "scoped_call_expression" => (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ),
        "call_expression" => {
            if node.child_by_field_name("type_arguments").is_some() {
                return Err("Chamada genérica não resolvida".into());
            }
            let callee = node
                .child_by_field_name("function")
                .ok_or("Chamada sem função")?;
            if callee.kind() == "member_expression" {
                if token(callee, "optional_chain") || token(node, "optional_chain") {
                    return Err("Chamada opcional exige prova de não nulidade".into());
                }
                (
                    callee.child_by_field_name("object"),
                    callee.child_by_field_name("property"),
                )
            } else {
                (None, Some(callee))
            }
        }
        "function_call_expression" => (None, node.child_by_field_name("function")),
        _ => return Err("Expressão não é chamada nominal direta".into()),
    };
    let name = name.ok_or("Chamada sem nome explícito")?;
    let (decl_file, decl) = if let Some(object) = object {
        if !matches!(name.kind(), "name" | "property_identifier") {
            return Err("Método dinâmico não resolvido".into());
        }
        let (owner, is_static) =
            receiver(file, object, files, identities, parents, budget, depth + 1)?;
        let candidates = methods(
            owner,
            text(name, &files[file].snapshot.before),
            false,
            is_static,
            files,
            parents,
            budget,
        )
        .map_err(|k| format!("Método de retorno interrompido: {k:?}"))?;
        let [(id, member)] = candidates.as_slice() else {
            return Err("Método de retorno ausente/ambíguo".into());
        };
        (*id, *member)
    } else {
        if !matches!(
            name.kind(),
            "identifier" | "name" | "qualified_name" | "relative_name"
        ) {
            return Err("Função dinâmica/closure não resolvida".into());
        }
        let (f, range) = crate::globals::function_declaration(files, file, name, budget)?;
        let declaration = declaration(&files[f], &range)?;
        stable_callable(
            f,
            declaration,
            text(name, &files[file].snapshot.before),
            files,
            budget,
        )?;
        return checked_callable(f, declaration);
    };
    let declaration = declaration(
        &files[decl_file.file],
        &files[decl_file.file].before.members[decl].range,
    )?;
    stable_callable(
        decl_file.file,
        declaration,
        text(name, &files[file].snapshot.before),
        files,
        budget,
    )?;
    checked_callable(decl_file.file, declaration)
}
fn stable_callable(
    file: usize,
    declaration: Node<'_>,
    used_name: &str,
    files: &[File<'_>],
    budget: &mut Budget<'_>,
) -> Result<(), String> {
    let name = declaration
        .child_by_field_name("name")
        .ok_or("Função sem nome declarado")?;
    let declared_name = text(name, &files[file].snapshot.before);
    for source in files {
        if source.language != files[file].language {
            continue;
        }
        let mut pending = vec![source.tree.root_node()];
        while let Some(node) = pending.pop() {
            tick(budget)?;
            if matches!(
                node.kind(),
                "assignment_expression"
                    | "augmented_assignment_expression"
                    | "update_expression"
                    | "reference_assignment_expression"
            ) && let Some(left) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"))
            {
                if left.kind() == "subscript_expression"
                    || (left.kind() == "member_access_expression"
                        && left
                            .child_by_field_name("name")
                            .is_some_and(|n| n.kind() != "name"))
                {
                    return Err("Atribuição dinâmica pode substituir função/método receptor".into());
                }
                let mut parts = vec![left];
                while let Some(part) = parts.pop() {
                    tick(budget)?;
                    let value = text(part, &source.snapshot.before);
                    if value == declared_name || value == used_name {
                        return Err("Função/método receptor pode ser reatribuído".into());
                    }
                    let mut cursor = part.walk();
                    parts.extend(part.named_children(&mut cursor));
                }
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
    }
    Ok(())
}
fn checked_callable(file: usize, node: Node<'_>) -> Result<(usize, Node<'_>), String> {
    if node.child_by_field_name("body").is_none()
        || node.child_by_field_name("type_parameters").is_some()
        || token(node, "async")
        || token(node, "*")
        || token(node, "?")
    {
        return Err("Retorno exige corpo concreto sem async/generic/overload/opcional".into());
    }
    Ok((file, node))
}
fn explicit_type(
    file: usize,
    declaration: Node<'_>,
    annotation: Node<'_>,
    files: &[File<'_>],
    identities: &Identities,
    budget: &mut Budget<'_>,
) -> Result<ClassId, String> {
    let name = type_node(annotation)
        .ok_or("Retorno/propriedade composto, anulável ou genérico não resolvido")?;
    let source = &files[file].snapshot.before;
    let mut ancestor = Some(declaration);
    while let Some(node) = ancestor {
        tick(budget)?;
        if node.child_by_field_name("type_parameters").is_some() {
            return Err(
                "Tipo nominal sob parâmetros genéricos exige substituição explícita".into(),
            );
        }
        ancestor = node.parent();
    }
    if files[file].language == Language::Php && text(name, source).eq_ignore_ascii_case("self") {
        return enclosing_class(declaration, file, files).ok_or("self sem classe declarada".into());
    }
    named_type(file, name, text(name, source), files, identities, budget)
}

/// Nominal types appearing in a class's public shape can carry an affected
/// member indirectly (Box.item: Port, Factory.make(): Port). Transfer proofs
/// must follow those dependencies instead of considering only direct methods.
pub(super) fn member_types(
    id: ClassId,
    files: &[File<'_>],
    identities: &Identities,
    budget: &mut Budget<'_>,
) -> Result<Vec<ClassId>, IssueKind> {
    let file = &files[id.file];
    let start = file.before.classes[id.class].start;
    let mut pending = vec![file.tree.root_node()];
    let mut result = HashSet::new();
    while let Some(node) = pending.pop() {
        budget.node()?;
        if node.start_byte() == start && class_kind(node.kind(), file.language) {
            let Some(body) = node.child_by_field_name("body") else {
                return Ok(vec![]);
            };
            let mut cursor = body.walk();
            for member in body.named_children(&mut cursor) {
                budget.node()?;
                let mut annotations = Vec::new();
                if let Some(ty) = member.child_by_field_name("type") {
                    annotations.push(ty);
                }
                if let Some(ty) = member.child_by_field_name("return_type") {
                    annotations.push(ty);
                }
                if let Some(parameters) = member.child_by_field_name("parameters") {
                    let mut pc = parameters.walk();
                    for p in parameters.named_children(&mut pc) {
                        budget.node()?;
                        if let Some(ty) = p.child_by_field_name("type") {
                            annotations.push(ty);
                        }
                    }
                }
                while let Some(ty) = annotations.pop() {
                    budget.node()?;
                    if ty.kind() == "type_identifier" {
                        if let Ok(target) = named_type(
                            id.file,
                            ty,
                            text(ty, &file.snapshot.before),
                            files,
                            identities,
                            budget,
                        ) {
                            result.insert(target);
                        }
                        budget.check()?;
                    }
                    let mut tc = ty.walk();
                    annotations.extend(ty.named_children(&mut tc));
                }
            }
            break;
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(result.into_iter().collect())
}

#[derive(Clone, Copy)]
struct Property<'a> {
    owner: ClassId,
    declaration: Node<'a>,
    annotation: Node<'a>,
    value: Option<Node<'a>>,
}
fn property<'a>(
    owner: ClassId,
    name: &str,
    is_static: bool,
    files: &'a [File<'_>],
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<Property<'a>, String> {
    let mut layer = vec![owner];
    let mut visited = HashSet::new();
    while !layer.is_empty() {
        let mut found = vec![];
        let mut next = vec![];
        for id in layer {
            tick(budget)?;
            if !visited.insert(id) {
                continue;
            }
            let file = &files[id.file];
            let source = &file.snapshot.before;
            let start = file.before.classes[id.class].start;
            let mut pending = vec![file.tree.root_node()];
            while let Some(node) = pending.pop() {
                tick(budget)?;
                let php = file.language == Language::Php;
                let promoted = if php {
                    node.kind() == "property_promotion_parameter"
                } else {
                    matches!(node.kind(), "required_parameter" | "optional_parameter")
                        && (token(node, "accessibility_modifier") || token(node, "readonly"))
                };
                let property = if php {
                    node.kind() == "property_element"
                } else {
                    node.kind() == "public_field_definition"
                };
                if property || promoted {
                    let field = node
                        .child_by_field_name("name")
                        .or_else(|| node.child_by_field_name("pattern"));
                    if field.is_some_and(|n| text(n, source).trim_start_matches('$') == name)
                        && enclosing_class(node, id.file, files).is_some_and(|c| c == id)
                    {
                        let decl = if php && property {
                            node.parent().unwrap()
                        } else {
                            node
                        };
                        let static_decl = token(decl, "static_modifier") || token(decl, "static");
                        if static_decl == is_static {
                            if token(decl, "?") || node.kind() == "optional_parameter" {
                                return Err(
                                    "Propriedade opcional exige prova de não nulidade".into()
                                );
                            }
                            let annotation = decl
                                .child_by_field_name("type")
                                .ok_or("Propriedade receptora sem tipo explícito")?;
                            found.push(Property {
                                owner: id,
                                declaration: decl,
                                annotation,
                                value: node
                                    .child_by_field_name("value")
                                    .or_else(|| node.child_by_field_name("default_value")),
                            });
                        }
                    }
                }
                if class_kind(node.kind(), file.language) && node.start_byte() != start {
                    continue;
                }
                let mut cursor = node.walk();
                pending.extend(node.named_children(&mut cursor));
            }
            next.extend(parents.get(&id).into_iter().flatten().map(|(p, _)| *p));
        }
        if found.len() == 1 {
            return Ok(found[0]);
        }
        if !found.is_empty() {
            return Err("Propriedade receptora ambígua".into());
        }
        layer = next;
    }
    Err("Propriedade receptora não declarada no corpus".into())
}
fn property_pair(node: Node<'_>) -> Option<(Node<'_>, Node<'_>, bool)> {
    match node.kind() {
        "member_expression" | "member_access_expression" => Some((
            node.child_by_field_name("object")?,
            node.child_by_field_name("property")
                .or_else(|| node.child_by_field_name("name"))?,
            false,
        )),
        "scoped_property_access_expression" => Some((
            node.child_by_field_name("scope")?,
            node.child_by_field_name("name")?,
            true,
        )),
        _ => None,
    }
}

fn nominal_subtype(
    actual: ClassId,
    expected: ClassId,
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<bool, String> {
    Ok(actual == expected
        || ancestry(actual, parents, false, budget)
            .map_err(|k| format!("Retorno herdado interrompido: {k:?}"))?
            .contains(&expected))
}
fn descendant_returns(
    owner: ClassId,
    name: &str,
    expected: ClassId,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<(), String> {
    for (file, source) in files.iter().enumerate() {
        if source.language != files[owner.file].language {
            continue;
        }
        for class in 0..source.before.classes.len() {
            tick(budget)?;
            let id = ClassId { file, class };
            if id == owner || !nominal_subtype(id, owner, parents, budget)? {
                continue;
            }
            for is_static in [false, true] {
                let found = methods(id, name, false, is_static, files, parents, budget)
                    .map_err(|k| format!("Override do retorno interrompido: {k:?}"))?;
                if found.len() > 1 {
                    return Err("Override de retorno ambíguo".into());
                }
                for (id, m) in found {
                    tick(budget)?;
                    let decl =
                        declaration(&files[id.file], &files[id.file].before.members[m].range)?;
                    let annotation = decl
                        .child_by_field_name("return_type")
                        .ok_or("Override sem retorno explícito")?;
                    let actual =
                        explicit_type(id.file, decl, annotation, files, identities, budget)?;
                    if !nominal_subtype(actual, expected, parents, budget)? {
                        return Err("Override de retorno usa tipo estrutural sem subtipagem nominal comprovada".into());
                    }
                }
            }
        }
    }
    Ok(())
}
fn descendant_properties(
    receiver: (ClassId, bool),
    name: &str,
    expected: ClassId,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<(), String> {
    let (owner, is_static) = receiver;
    for (file, source) in files.iter().enumerate() {
        if source.language != files[owner.file].language {
            continue;
        }
        for class in 0..source.before.classes.len() {
            tick(budget)?;
            let id = ClassId { file, class };
            if id == owner || !nominal_subtype(id, owner, parents, budget)? {
                continue;
            }
            let member = property(id, name, is_static, files, parents, budget)?;
            let actual = explicit_type(
                member.owner.file,
                member.declaration,
                member.annotation,
                files,
                identities,
                budget,
            )?;
            if !nominal_subtype(actual, expected, parents, budget)? {
                return Err(
                    "Propriedade herdada possui tipo estrutural sem subtipagem nominal comprovada"
                        .into(),
                );
            }
        }
    }
    Ok(())
}
fn stable_property(
    selected: Property<'_>,
    name: &str,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<(), String> {
    for (file, source) in files.iter().enumerate() {
        if source.language != files[selected.owner.file].language {
            continue;
        }
        let mut pending = vec![source.tree.root_node()];
        while let Some(node) = pending.pop() {
            tick(budget)?;
            if matches!(
                node.kind(),
                "assignment_expression"
                    | "augmented_assignment_expression"
                    | "update_expression"
                    | "reference_assignment_expression"
            ) && let Some(left) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"))
                && (left.kind() == "subscript_expression"
                    || (left.kind() == "member_access_expression"
                        && left
                            .child_by_field_name("name")
                            .is_some_and(|n| n.kind() != "name")))
            {
                return Err("Propriedade receptora pode sofrer atribuição dinâmica".into());
            }
            if node.kind() == "unset_statement"
                || (node.kind() == "unary_expression" && token(node, "delete"))
            {
                return Err("Remoção de propriedade exige análise de fluxo adicional".into());
            }
            if matches!(
                node.kind(),
                "assignment_expression"
                    | "augmented_assignment_expression"
                    | "update_expression"
                    | "reference_assignment_expression"
            ) && let Some(left) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("argument"))
                && let Some((object, field, _)) = property_pair(left)
                && text(field, &source.snapshot.before).trim_start_matches('$') == name
            {
                let (id, _) =
                    receiver(file, object, files, identities, parents, budget, depth + 1)?;
                if id == selected.owner
                    || ancestry(id, parents, false, budget)
                        .map_err(|k| format!("Atribuição de propriedade interrompida: {k:?}"))?
                        .contains(&selected.owner)
                {
                    return Err(
                        "Propriedade receptora possui atribuição/mutação e exige análise de fluxo"
                            .into(),
                    );
                }
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
    }
    Ok(())
}

pub(super) fn value(
    file: usize,
    node: Node<'_>,
    files: &[File<'_>],
    identities: &Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
    depth: usize,
) -> Result<ClassId, String> {
    tick(budget)?;
    if depth > 16 {
        return Err("Cadeia de retornos/propriedades excede profundidade suportada".into());
    }
    if let Some((object, name, static_property)) = property_pair(node) {
        if token(node, "optional_chain")
            || !matches!(
                name.kind(),
                "name" | "property_identifier" | "variable_name"
            )
            || (!static_property && name.kind() == "variable_name")
        {
            return Err("Propriedade dinâmica/opcional não resolvida".into());
        }
        let (owner, is_static) =
            receiver(file, object, files, identities, parents, budget, depth + 1)?;
        let property = property(
            owner,
            text(name, &files[file].snapshot.before).trim_start_matches('$'),
            static_property || is_static,
            files,
            parents,
            budget,
        )?;
        let ty = explicit_type(
            property.owner.file,
            property.declaration,
            property.annotation,
            files,
            identities,
            budget,
        )?;
        descendant_properties(
            (property.owner, static_property || is_static),
            text(name, &files[file].snapshot.before).trim_start_matches('$'),
            ty,
            files,
            identities,
            parents,
            budget,
        )?;
        if let Some(value) = property.value {
            let (actual, static_value) = receiver(
                property.owner.file,
                value,
                files,
                identities,
                parents,
                budget,
                depth + 1,
            )?;
            if static_value
                || (actual != ty
                    && !ancestry(actual, parents, false, budget)
                        .map_err(|k| format!("Inicializador interrompido: {k:?}"))?
                        .contains(&ty))
            {
                return Err(
                    "Inicializador de propriedade não possui relação nominal comprovada".into(),
                );
            }
        }
        stable_property(
            property,
            text(name, &files[file].snapshot.before).trim_start_matches('$'),
            files,
            identities,
            parents,
            budget,
            depth + 1,
        )?;
        return Ok(ty);
    }
    let (decl_file, decl) = callable(file, node, files, identities, parents, budget, depth + 1)?;
    let annotation = decl
        .child_by_field_name("return_type")
        .ok_or("Função/método receptor sem retorno explícito")?;
    let ty = explicit_type(decl_file, decl, annotation, files, identities, budget)?;
    if matches!(decl.kind(), "method_definition" | "method_declaration") {
        let owner =
            enclosing_class(decl, decl_file, files).ok_or("Retorno de método sem classe")?;
        let name = decl.child_by_field_name("name").ok_or("Método sem nome")?;
        descendant_returns(
            owner,
            text(name, &files[decl_file].snapshot.before),
            ty,
            files,
            identities,
            parents,
            budget,
        )?;
    }
    Ok(ty)
}
