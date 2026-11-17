//! Exact PHP named-argument bindings for lexical parameter renames.
//! This is a child of hierarchy so receiver resolution is shared, not guessed.
use super::*;
use crate::{ConflictKind, RenameSnapshot};
use std::ops::Range;

/// Completes a provider's lexical PHP parameter proposal with independently
/// resolved named labels. This returns a proposal, never permission to apply.
pub fn complete_php_named_argument_rename(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &std::sync::atomic::AtomicBool,
    limits: crate::ValidationLimits,
) -> Result<Option<Vec<RenameSnapshot>>, String> {
    crate::validate_rename_candidate(new_name)?;
    if snapshots.len() > 64
        || snapshots
            .iter()
            .map(|s| s.before.len() + s.after.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
    {
        return Err("Corpus do complemento de parâmetros excede limites".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut targets = vec![];
    for snapshot in snapshots
        .iter()
        .filter(|s| s.path.extension().is_some_and(|e| e == "php") && !s.edits.is_empty())
    {
        tick(&mut budget)?;
        let edits = crate::incremental::prepare_edits(snapshot, new_name, &budget)?;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&Language::Php.grammar())
            .map_err(|e| e.to_string())?;
        let tree = crate::incremental::parse(&mut parser, &snapshot.before, None, &budget)
            .ok_or("Parser de parâmetro interrompido")?;
        if tree.root_node().has_error() {
            return Err("Snapshot anterior de parâmetro contém erro".into());
        }
        let mut pending = vec![tree.root_node()];
        while let Some(node) = pending.pop() {
            tick(&mut budget)?;
            if node.kind() == "simple_parameter"
                && let Some(variable) = node.child_by_field_name("name")
                && let Some(name) = variable.named_child(0)
                && edits.iter().any(|e| e.before == name.byte_range())
            {
                targets.push((snapshot.path.clone(), name.byte_range()));
            }
            let mut c = node.walk();
            pending.extend(node.named_children(&mut c));
        }
    }
    if targets.is_empty() {
        return Ok(None);
    }
    if targets.len() != 1 {
        return Err("Complemento exige exatamente um parâmetro lexical".into());
    }
    let mut report = RenameValidation::new();
    let anchors = audit(
        snapshots,
        &targets[0].0,
        targets[0].1.clone(),
        new_name,
        &mut report,
        &mut budget,
    )?;
    if !report.issues.is_empty() {
        return Err("Não foi possível provar os argumentos nomeados do parâmetro".into());
    }
    let mut result = snapshots.to_vec();
    for (path, range) in anchors {
        tick(&mut budget)?;
        let snapshot = result
            .iter_mut()
            .find(|s| s.path == path)
            .ok_or("Argumento fora do corpus")?;
        let range = text_range(&snapshot.before, range);
        if !snapshot.edits.iter().any(|e| e.range == range) {
            snapshot.edits.push(crate::RenameEdit {
                range,
                new_text: new_name.into(),
            });
        }
    }
    if result.iter().map(|s| s.edits.len()).sum::<usize>() > 1024 {
        return Err("Complemento excede 1024 edições".into());
    }
    for snapshot in &mut result {
        tick(&mut budget)?;
        snapshot
            .edits
            .sort_by_key(|e| (e.range.start_line, e.range.start_character));
        let mut after = snapshot.before.to_string();
        let mut next = after.len();
        for edit in snapshot.edits.iter().rev() {
            let range = &edit.range;
            let start = crate::incremental::byte_offset(
                &snapshot.before,
                range.start_line,
                range.start_character,
            )
            .ok_or("Início UTF-16 inválido")?;
            let end = crate::incremental::byte_offset(
                &snapshot.before,
                range.end_line,
                range.end_character,
            )
            .ok_or("Fim UTF-16 inválido")?;
            if start >= end || end > next {
                return Err("Edições de parâmetro sobrepostas/inválidas".into());
            }
            after.replace_range(start..end, &edit.new_text);
            next = start;
        }
        snapshot.after = std::sync::Arc::from(after);
    }
    Ok(Some(result))
}

fn tick(budget: &mut Budget<'_>) -> Result<(), String> {
    budget
        .node()
        .map_err(|k| format!("Argumentos nomeados interrompidos: {k:?}"))
}
fn callable_node<'a>(file: &'a File<'_>, range: Range<usize>) -> Result<Node<'a>, String> {
    let mut node = file
        .tree
        .root_node()
        .named_descendant_for_byte_range(range.start, range.end)
        .ok_or("Origem do parâmetro ausente")?;
    loop {
        if matches!(node.kind(), "function_definition" | "method_declaration") {
            return Ok(node);
        }
        node = node.parent().ok_or("Parâmetro sem função/método nomeado")?;
    }
}
fn parameters<'a>(node: Node<'a>, source: &str) -> Result<Vec<(String, Node<'a>, bool)>, String> {
    let params = node
        .child_by_field_name("parameters")
        .ok_or("Função sem parâmetros AST")?;
    let mut cursor = params.walk();
    let mut result = vec![];
    for param in params
        .named_children(&mut cursor)
        .filter(|n| n.kind() != "comment")
    {
        if !matches!(param.kind(), "simple_parameter" | "variadic_parameter") {
            return Err("Parâmetro promovido/desestruturado exige prova própria".into());
        }
        let variable = param
            .child_by_field_name("name")
            .ok_or("Parâmetro sem nome")?;
        let name = variable
            .named_child(0)
            .filter(|n| n.kind() == "name")
            .ok_or("Nome de parâmetro dinâmico")?;
        result.push((
            text(name, source).into(),
            name,
            param.kind() == "variadic_parameter",
        ));
    }
    Ok(result)
}
fn resolve_call<'a>(
    file_index: usize,
    call: Node<'_>,
    files: &'a [File<'_>],
    identities: &binding::Identities,
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<(usize, Node<'a>), String> {
    tick(budget)?;
    if call.kind() == "function_call_expression" {
        let name = call
            .child_by_field_name("function")
            .ok_or("Chamada sem função")?;
        if !matches!(name.kind(), "name" | "qualified_name" | "relative_name") {
            return Err("Argumento nomeado em chamada dinâmica não resolvido".into());
        }
        let (index, range) = crate::globals::function_declaration(files, file_index, name, budget)?;
        return Ok((index, callable_node(&files[index], range)?));
    }
    let (receiver, method) = if call.kind() == "object_creation_expression" {
        (call, "__construct")
    } else {
        let receiver = call
            .child_by_field_name("object")
            .or_else(|| call.child_by_field_name("scope"))
            .ok_or("Chamada sem receptor nominal")?;
        let name = call
            .child_by_field_name("name")
            .filter(|n| n.kind() == "name")
            .ok_or("Método de argumento nomeado é dinâmico")?;
        (receiver, text(name, &files[file_index].snapshot.before))
    };
    let (owner, is_static) =
        binding::receiver(file_index, receiver, files, identities, parents, budget, 0)?;
    let candidates = binding::methods(owner, method, false, is_static, files, parents, budget)
        .map_err(|k| format!("Método interrompido: {k:?}"))?;
    let [(id, member)] = candidates.as_slice() else {
        return Err("Método/constructor ausente ou ambíguo".into());
    };
    Ok((
        id.file,
        callable_node(
            &files[id.file],
            files[id.file].before.members[*member].range.clone(),
        )?,
    ))
}
fn fault(
    report: &mut RenameValidation,
    file: &File<'_>,
    range: Range<usize>,
    kind: ConflictKind,
    message: &str,
) {
    if report.conflicts.len() < 256 {
        report.conflicts.push(Conflict {
            path: file.snapshot.path.clone(),
            range: text_range(&file.snapshot.before, range),
            related: None,
            kind,
            message: message.into(),
            container_name: None,
        });
    }
}
pub(crate) fn audit(
    snapshots: &[RenameSnapshot],
    target_path: &Path,
    target_range: Range<usize>,
    new_name: &str,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<Vec<(PathBuf, Range<usize>)>, String> {
    let mut parsed_report = RenameValidation::new();
    let files = properties::parse_files(snapshots, new_name, &mut parsed_report, budget)
        .map_err(|k| format!("Índice de parâmetros interrompido: {k:?}"))?;
    if !parsed_report.issues.is_empty() || !parsed_report.conflicts.is_empty() {
        report.issues.extend(parsed_report.issues);
        report.conflicts.extend(parsed_report.conflicts);
        return Err("Corpus do parâmetro contém snapshots inválidos".into());
    }
    let target_file = files
        .iter()
        .position(|f| f.snapshot.path == target_path)
        .ok_or("Arquivo do parâmetro ausente")?;
    let declaration = callable_node(&files[target_file], target_range.clone())?;
    let params = parameters(declaration, &files[target_file].snapshot.before)?;
    let parameter = params
        .iter()
        .position(|(_, n, _)| n.byte_range() == target_range)
        .ok_or("Origem não é parâmetro direto")?;
    if params[parameter].2 {
        return Err("Nome de parâmetro variadic não vincula chaves nomeadas arbitrárias".into());
    }
    let old_name = &params[parameter].0;
    let mut identities = binding::Identities::new();
    let mut all = vec![];
    for (index, file) in files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.language == Language::Php)
    {
        for (class_index, class) in file.before.classes.iter().enumerate() {
            tick(budget)?;
            let id = ClassId {
                file: index,
                class: class_index,
            };
            let meta = file
                .metadata
                .classes
                .get(&class.start)
                .ok_or("Classe sem identidade")?;
            if let Some(problem) = &meta.problem {
                return Err(problem.clone());
            }
            if file.metadata.dynamic_alias {
                return Err("class_alias impede vincular parâmetros".into());
            }
            identities
                .entry((file.language, identity(file, meta)))
                .or_default()
                .push(id);
            all.push(id);
        }
    }
    if identities.values().any(|ids| ids.len() != 1) {
        return Err("Identidade de classe ambígua".into());
    }
    let mut parents = Parents::new();
    for &id in &all {
        let file = &files[id.file];
        let class = &file.before.classes[id.class];
        for relation in &file.metadata.classes[&class.start].parents {
            tick(budget)?;
            let parent = resolve_parent(id, relation, &files, &identities, budget)?;
            if !valid_edge(
                class.kind,
                files[parent.file].before.classes[parent.class].kind,
                relation.kind,
            ) {
                return Err("Relação de classes inválida para parâmetros".into());
            }
            parents.entry(id).or_default().push((parent, relation.kind));
        }
    }
    let mut ancestors = Ancestors::new();
    for &id in &all {
        let chain = ancestry(id, &parents, false, budget)
            .map_err(|k| format!("Grafo interrompido: {k:?}"))?;
        if chain.contains(&id) {
            return Err("Ciclo de herança no parâmetro".into());
        }
        ancestors.insert(id, chain);
    }
    if declaration.kind() == "method_declaration" {
        let name = declaration
            .child_by_field_name("name")
            .ok_or("Método sem nome")?;
        let target_class = files[target_file]
            .before
            .members
            .iter()
            .find(|m| m.method && m.range == name.byte_range())
            .map(|m| ClassId {
                file: target_file,
                class: m.class,
            })
            .ok_or("Método sem classe nominal")?;
        for &id in &all {
            if id != target_class
                && related(id, target_class, &ancestors, budget)
                    .map_err(|k| format!("Grafo de parâmetros interrompido: {k:?}"))?
                && files[id.file].before.classes[id.class]
                    .members
                    .iter()
                    .any(|&member| {
                        let m = &files[id.file].before.members[member];
                        m.method
                            && m.name.eq_ignore_ascii_case(text(
                                name,
                                &files[target_file].snapshot.before,
                            ))
                    })
            {
                return Err("Parâmetros de família com overrides exigem proposta conjunta".into());
            }
        }
    }
    let mut anchors = vec![];
    for (file_index, file) in files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.language == Language::Php)
    {
        let mut pending = vec![file.tree.root_node()];
        while let Some(call) = pending.pop() {
            tick(budget)?;
            let mut cursor = call.walk();
            pending.extend(call.named_children(&mut cursor));
            if call.kind() == "name"
                && matches!(
                    text(call, &file.snapshot.before)
                        .to_ascii_lowercase()
                        .as_str(),
                    "reflectionfunction"
                        | "reflectionmethod"
                        | "reflectionclass"
                        | "reflectionparameter"
                        | "reflectionfunctionabstract"
                        | "call_user_func"
                        | "call_user_func_array"
                        | "forward_static_call"
                        | "forward_static_call_array"
                )
            {
                return Err("Reflexão/callbacks, inclusive aliases importados, podem observar nomes de parâmetros".into());
            }
            if !matches!(
                call.kind(),
                "function_call_expression"
                    | "member_call_expression"
                    | "scoped_call_expression"
                    | "object_creation_expression"
            ) {
                continue;
            }
            if call.kind() == "function_call_expression"
                && call.child_by_field_name("function").is_some_and(|n| {
                    !matches!(n.kind(), "name" | "qualified_name" | "relative_name")
                })
                || matches!(
                    call.kind(),
                    "member_call_expression" | "scoped_call_expression"
                ) && call
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() != "name")
            {
                return Err(
                    "Chamada dinâmica pode encaminhar argumentos nomeados sem vínculo comprovado"
                        .into(),
                );
            }
            if call.kind() == "function_call_expression"
                && call.child_by_field_name("function").is_some_and(|n| {
                    matches!(
                        text(n, &file.snapshot.before)
                            .trim_start_matches('\\')
                            .to_ascii_lowercase()
                            .as_str(),
                        "call_user_func"
                            | "call_user_func_array"
                            | "forward_static_call"
                            | "forward_static_call_array"
                    )
                })
            {
                return Err("Callback dinâmico pode observar nomes de parâmetros".into());
            }
            let args = call.child_by_field_name("arguments").or_else(|| {
                let mut c = call.walk();
                call.named_children(&mut c)
                    .find(|n| n.kind() == "arguments")
            });
            let Some(args) = args else { continue };
            let mut c = args.walk();
            let args: Vec<_> = args
                .named_children(&mut c)
                .filter(|n| n.kind() != "comment")
                .collect();
            let mut unpacked = false;
            for arg in &args {
                let mut nodes = vec![*arg];
                while let Some(n) = nodes.pop() {
                    tick(budget)?;
                    if n.kind() == "variadic_unpacking" {
                        unpacked = true;
                    }
                    let mut c = n.walk();
                    nodes.extend(n.named_children(&mut c));
                }
            }
            let relevant = args.iter().any(|a| {
                a.child_by_field_name("name").is_some_and(|n| {
                    text(n, &file.snapshot.before) == old_name
                        || text(n, &file.snapshot.before) == new_name
                })
            });
            if !relevant && !unpacked {
                continue;
            }
            let (resolved_file, resolved) =
                resolve_call(file_index, call, &files, &identities, &parents, budget)?;
            if resolved_file != target_file || resolved.byte_range() != declaration.byte_range() {
                continue;
            }
            if unpacked {
                return Err("Unpacking de argumentos do alvo pode conter chaves nomeadas".into());
            }
            let mut supplied = HashSet::new();
            let mut saw_named = false;
            for (positional, arg) in args.into_iter().enumerate() {
                tick(budget)?;
                let Some(name) = arg.child_by_field_name("name") else {
                    if saw_named {
                        return Err(
                            "Argumento posicional depois de nomeado no snapshot anterior".into(),
                        );
                    }
                    supplied.insert(positional);
                    continue;
                };
                saw_named = true;
                let spelling = text(name, &file.snapshot.before);
                let index = params.iter().position(|(p, _, _)| p == spelling);
                if spelling == new_name && spelling != old_name {
                    fault(
                        report,
                        file,
                        name.byte_range(),
                        ConflictKind::BindingChanged,
                        "Novo nome captura argumento nomeado existente",
                    );
                }
                let Some(index) = index else {
                    if params.last().is_some_and(|p| p.2) {
                        continue;
                    }
                    return Err(
                        "Argumento nomeado anterior não corresponde a parâmetro declarado".into(),
                    );
                };
                if !supplied.insert(index) {
                    return Err(
                        "Argumento repetido/posicional duplicado no snapshot anterior".into(),
                    );
                }
                if index == parameter {
                    anchors.push((file.snapshot.path.clone(), name.byte_range()));
                    if !file
                        .edits
                        .iter()
                        .any(|e| e.before == name.byte_range() && e.new_text == new_name)
                    {
                        fault(
                            report,
                            file,
                            name.byte_range(),
                            ConflictKind::MissingReference,
                            "Proposta omite argumento nomeado do parâmetro selecionado",
                        );
                    }
                }
            }
        }
    }
    Ok(anchors)
}
