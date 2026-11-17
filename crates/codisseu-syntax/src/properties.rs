//! Independent property proof over exact LSP edits. Never discovers or creates edits.
use super::*;
use crate::{Coverage, ValidationLimits, ast, incremental};
use std::{ops::Range, sync::atomic::AtomicBool};

fn promoted(node: Node<'_>, language: Language) -> bool {
    (language == Language::Php && node.kind() == "property_promotion_parameter")
        || (language == Language::TypeScript
            && matches!(node.kind(), "required_parameter" | "optional_parameter")
            && (token(node, "accessibility_modifier") || token(node, "readonly")))
}
fn property_name(node: Node<'_>, language: Language) -> Option<Node<'_>> {
    let name = if promoted(node, language) {
        node.child_by_field_name("name")
            .or_else(|| node.child_by_field_name("pattern"))
    } else {
        match language {
            Language::Php if node.kind() == "property_element" => node.child_by_field_name("name"),
            Language::TypeScript
                if matches!(
                    node.kind(),
                    "public_field_definition" | "property_signature"
                ) =>
            {
                node.child_by_field_name("name")
            }
            _ => None,
        }
    }?;
    if language == Language::Php && name.kind() == "variable_name" {
        return name.named_child(0).filter(|n| n.kind() == "name");
    }
    (name.kind() == "property_identifier"
        || (promoted(node, language) && name.kind() == "identifier"))
        .then_some(name)
}
fn augment(
    tree: &Tree,
    source: &str,
    language: Language,
    facts: &mut Facts,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        budget.node()?;
        if (language == Language::Php || promoted(node, language))
            && let Some(name) = property_name(node, language)
        {
            let declaration = if node.kind() == "property_element" {
                node.parent().unwrap()
            } else {
                node
            };
            let mut ancestor = declaration;
            let class = loop {
                if let Some(index) = facts.classes.iter().position(|c| {
                    c.start == ancestor.start_byte() && class_kind(ancestor.kind(), language)
                }) {
                    break Some(index);
                }
                let Some(parent) = ancestor.parent() else {
                    break None;
                };
                ancestor = parent;
            };
            if let Some(class) = class {
                let mut cursor = declaration.walk();
                let private = declaration.named_children(&mut cursor).any(|n| {
                    matches!(n.kind(), "visibility_modifier" | "accessibility_modifier")
                        && text(n, source) == "private"
                });
                facts.classes[class].members.push(facts.members.len());
                facts.members.push(ast::Member {
                    range: name.byte_range(),
                    name: text(name, source).into(),
                    class,
                    static_member: token(declaration, "static_modifier")
                        || token(declaration, "static"),
                    method: false,
                    abstract_member: false,
                    inheritance_uncertain: private,
                    explicit_override: false,
                    contract: Err("Contratos de métodos não se aplicam a propriedades"),
                });
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(())
}
fn declaration<'a>(
    file: &'a File<'_>,
    range: &Range<usize>,
    budget: &mut Budget<'_>,
) -> Result<Option<Node<'a>>, IssueKind> {
    let mut pending = vec![file.tree.root_node()];
    while let Some(node) = pending.pop() {
        budget.node()?;
        if property_name(node, file.language).is_some_and(|n| n.byte_range() == *range) {
            return Ok(Some(node));
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    Ok(None)
}
#[derive(Eq, PartialEq)]
struct PropertyContract {
    ty: crate::contracts::TypeContract,
    visibility: String,
    is_static: bool,
    readonly: bool,
    optional: bool,
}
fn contract(
    file: &File<'_>,
    member: &ast::Member,
    budget: &mut Budget<'_>,
) -> Result<Option<PropertyContract>, IssueKind> {
    let Some(node) = declaration(file, &member.range, budget)? else {
        return Ok(None);
    };
    let node = if node.kind() == "property_element" {
        node.parent().unwrap()
    } else {
        node
    };
    let Some(ty) = node.child_by_field_name("type") else {
        return Ok(None);
    };
    let Some(ty) = crate::contracts::explicit_type(
        ty,
        &file.snapshot.before,
        file.language,
        false,
        budget,
        0,
    )?
    else {
        return Ok(None);
    };
    let mut visibility = "public".to_owned();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        budget.node()?;
        if matches!(
            child.kind(),
            "visibility_modifier" | "accessibility_modifier"
        ) {
            visibility = text(child, &file.snapshot.before).into();
        }
        if matches!(
            child.kind(),
            "attribute_list" | "decorator" | "property_hook_list" | "property_hook"
        ) {
            return Ok(None);
        }
    }
    if !matches!(visibility.as_str(), "public" | "protected") {
        return Ok(None);
    }
    Ok(Some(PropertyContract {
        ty,
        visibility,
        is_static: member.static_member,
        readonly: token(node, "readonly") || token(node, "readonly_modifier"),
        optional: token(node, "?"),
    }))
}
fn fault(
    file: &File<'_>,
    range: Range<usize>,
    kind: ConflictKind,
    message: &str,
    report: &mut RenameValidation,
) {
    if report.conflicts.len() < 256 {
        report.conflicts.push(Conflict {
            path: file.snapshot.path.clone(),
            range: text_range(&file.snapshot.before, range),
            kind,
            message: message.into(),
            related: None,
            container_name: None,
        });
    }
}
fn issue(
    file: &File<'_>,
    range: Range<usize>,
    message: impl Into<String>,
    report: &mut RenameValidation,
) {
    report.issue(
        &file.snapshot.path,
        IssueKind::UnsupportedEdit,
        Some(text_range(&file.snapshot.before, range)),
        message,
    );
}
pub(super) fn parse_files<'a>(
    snapshots: &'a [RenameSnapshot],
    new_name: &str,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<Vec<File<'a>>, IssueKind> {
    let mut files = vec![];
    let mut metadata_bytes = 0usize;
    let mut paths = HashSet::new();
    for snapshot in snapshots {
        budget.node()?;
        if !paths.insert(&snapshot.path) {
            report.issue(
                &snapshot.path,
                IssueKind::InvalidSnapshot,
                None,
                "Caminho de snapshot duplicado",
            );
            continue;
        }
        let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
            Some("php") => Language::Php,
            Some("ts") => Language::TypeScript,
            _ => {
                if !snapshot.edits.is_empty() {
                    report.issue(
                        &snapshot.path,
                        IssueKind::UnsupportedLanguage,
                        None,
                        "Propriedades somente em PHP/TypeScript independente",
                    );
                }
                continue;
            }
        };
        let edits = match incremental::prepare_edits(snapshot, new_name, budget) {
            Ok(edits) => edits,
            Err(message) => {
                report.issue(&snapshot.path, IssueKind::InvalidSnapshot, None, message);
                continue;
            }
        };
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language.grammar())
            .map_err(|_| IssueKind::Parser)?;
        let tree = incremental::parse(&mut parser, &snapshot.before, None, budget)
            .ok_or_else(|| budget.check().err().unwrap_or(IssueKind::Parser))?;
        let mut before = ast::inspect(&tree, &snapshot.before, language, budget)?;
        augment(&tree, &snapshot.before, language, &mut before, budget)?;
        report.checked_files += 1;
        if let Some(error) = before.errors.first() {
            report.issue(
                &snapshot.path,
                IssueKind::SyntaxBefore,
                Some(text_range(&snapshot.before, error.clone())),
                "Snapshot anterior contém ERROR/MISSING",
            );
        }
        let metadata = inspect(&tree, &snapshot.before, language, budget)?;
        metadata_bytes = metadata_bytes.saturating_add(metadata.bytes);
        if metadata_bytes > 8 * 1024 * 1024 {
            return Err(IssueKind::Limit);
        }
        let after = if edits.is_empty() {
            None
        } else {
            let mut changed_tree = tree.clone();
            for edit in edits.iter().rev() {
                changed_tree.edit(&edit.input_edit(&snapshot.before));
            }
            let parsed =
                incremental::parse(&mut parser, &snapshot.after, Some(&changed_tree), budget)
                    .ok_or_else(|| budget.check().err().unwrap_or(IssueKind::Parser))?;
            report.incremental_reparses += 1;
            let mut facts = ast::inspect(&parsed, &snapshot.after, language, budget)?;
            augment(&parsed, &snapshot.after, language, &mut facts, budget)?;
            if !facts.errors.is_empty() {
                report.conflicts.push(Conflict {
                    path: snapshot.path.clone(),
                    range: snapshot.edits[0].range.clone(),
                    message: "Proposta introduz erro sintático".into(),
                    related: None,
                    kind: ConflictKind::InvalidSyntax,
                    container_name: None,
                });
            }
            Some(facts)
        };
        files.push(File {
            tree,
            snapshot,
            language,
            before,
            after,
            metadata,
            edits,
        });
    }
    Ok(files)
}
pub(super) fn validate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    let mut report = RenameValidation::new();
    report.scope = ValidationScope::KnownPropertyBindings;
    report.note = "Propriedades verificadas por declaração AST, receptor nominal conhecido e referências completas nos snapshots. Sem prova de reflexão, nomes dinâmicos, fluxos estruturais arbitrários ou dependências externas ao corpus.".into();
    let fallback = snapshots
        .first()
        .map_or(Path::new(""), |s| s.path.as_path());
    if snapshots.is_empty()
        || snapshots.len() > 64
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots
            .iter()
            .map(|s| s.before.len() + s.after.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
    {
        report.issue(
            fallback,
            IssueKind::Limit,
            None,
            "Limite de corpus de propriedades excedido",
        );
        return Some(report);
    }
    if let Err(message) = crate::validate_rename_candidate(new_name) {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, message);
        return Some(report);
    }
    let mut budget = Budget::new(cancel, limits);
    let files = match parse_files(snapshots, new_name, &mut report, &mut budget) {
        Ok(files) => files,
        Err(kind) => {
            report.issue(
                fallback,
                kind,
                None,
                "Validação de propriedades interrompida",
            );
            return Some(report);
        }
    };
    let recognized = files.iter().any(|f| {
        f.before
            .members
            .iter()
            .any(|m| !m.method && f.edits.iter().any(|e| e.before == m.range))
    });
    if !recognized {
        return if report.issues.is_empty() && report.conflicts.is_empty() {
            None
        } else {
            Some(report)
        };
    }
    if !report.conflicts.is_empty()
        || files.iter().any(|f| {
            f.after.as_ref().is_some_and(|a| {
                a.members.len() != f.before.members.len()
                    || a.classes.len() != f.before.classes.len()
            })
        })
    {
        report.coverage = if report.conflicts.is_empty() {
            Coverage::Unsupported
        } else {
            Coverage::Rejected
        };
        report.issue(
            fallback,
            IssueKind::UnsupportedEdit,
            None,
            "Proposta não preserva a estrutura AST",
        );
        return Some(report);
    }
    if let Err(kind) = check(&files, &mut report, &mut budget) {
        report.issue(fallback, kind, None, "Orçamento de propriedades excedido");
    }
    report.coverage = if !report.conflicts.is_empty() {
        Coverage::Rejected
    } else if !report.issues.is_empty() {
        Coverage::Unsupported
    } else {
        Coverage::Verified
    };
    Some(report)
}
type Property = (ClassId, usize);
fn lookup(
    id: ClassId,
    name: &str,
    after: bool,
    static_access: bool,
    files: &[File<'_>],
    parents: &Parents,
    budget: &mut Budget<'_>,
) -> Result<Vec<Property>, IssueKind> {
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
                let member = &facts.members[m];
                if !member.method && member.static_member == static_access && member.name == name {
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
fn check(
    files: &[File<'_>],
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let mut targets = HashSet::new();
    let mut names: HashMap<Language, HashSet<String>> = HashMap::new();
    let mut identities = HashMap::new();
    let mut all = vec![];
    let mut anchored: Vec<HashSet<Range<usize>>> = files.iter().map(|_| HashSet::new()).collect();
    for (i, f) in files.iter().enumerate() {
        for (j, c) in f.before.classes.iter().enumerate() {
            budget.node()?;
            let id = ClassId { file: i, class: j };
            all.push(id);
            if let Some(meta) = f.metadata.classes.get(&c.start) {
                identities
                    .entry((f.language, identity(f, meta)))
                    .or_insert_with(Vec::new)
                    .push(id);
            }
        }
        for (m, member) in f.before.members.iter().enumerate() {
            budget.node()?;
            if member.method || !changed(f, m) {
                continue;
            }
            let id = ClassId {
                file: i,
                class: member.class,
            };
            targets.insert((id, m));
            anchored[i].insert(member.range.clone());
            report.checked_declarations += 1;
            let after = f.after.as_ref().unwrap();
            let Some(new) = after.members.get(m) else {
                issue(
                    f,
                    member.range.clone(),
                    "Proposta alterou estrutura da classe",
                    report,
                );
                continue;
            };
            let edit = f.edits.iter().find(|e| e.before == member.range).unwrap();
            if new.range != edit.after
                || new.class != member.class
                || new.static_member != member.static_member
                || new.method
                || after.classes.len() != f.before.classes.len()
                || after.members.len() != f.before.members.len()
            {
                issue(
                    f,
                    member.range.clone(),
                    "Declaração posterior não preserva identidade da propriedade",
                    report,
                );
            }
            names
                .entry(f.language)
                .or_default()
                .extend([member.name.clone(), new.name.clone()]);
            if (f.language == Language::TypeScript && new.name == "constructor")
                || (member.static_member
                    && f.language == Language::TypeScript
                    && matches!(
                        new.name.as_str(),
                        "prototype" | "name" | "length" | "caller" | "arguments"
                    ))
            {
                issue(
                    f,
                    member.range.clone(),
                    "Nome reservado de classe/Function",
                    report,
                );
            }
            let class = &f.before.classes[id.class];
            let mut existing = HashSet::new();
            for &index in &class.members {
                budget.node()?;
                let candidate = &f.before.members[index];
                if f.language == Language::Php && candidate.method {
                    continue;
                }
                let category = f.language == Language::TypeScript && candidate.static_member;
                if !existing.insert((category, candidate.name.clone())) {
                    issue(
                        f,
                        candidate.range.clone(),
                        "Classe já contém propriedades/membros duplicados",
                        report,
                    );
                }
            }
            for reason in &class.unsupported {
                if reason != "Parameter properties do constructor não verificadas" {
                    issue(f, member.range.clone(), reason.clone(), report);
                }
            }
            for &other in &class.members {
                budget.node()?;
                if other == m {
                    continue;
                }
                let candidate = &after.members[other];
                let same_namespace = if f.language == Language::Php {
                    !candidate.method
                } else {
                    candidate.static_member == new.static_member
                };
                if same_namespace && candidate.name == new.name {
                    fault(
                        f,
                        f.before.members[other].range.clone(),
                        ConflictKind::DuplicateMember,
                        "Novo nome colide com membro já declarado",
                        report,
                    );
                }
            }
        }
    }
    let active: HashSet<_> = names.keys().copied().collect();
    let mut parents = Parents::new();
    for &id in &all {
        budget.node()?;
        let f = &files[id.file];
        if !active.contains(&f.language) {
            continue;
        }
        let c = &f.before.classes[id.class];
        let Some(meta) = f.metadata.classes.get(&c.start) else {
            continue;
        };
        if let Some(problem) = &meta.problem {
            issue(f, meta.range.clone(), problem.clone(), report);
        }
        if identities
            .get(&(f.language, identity(f, meta)))
            .is_some_and(|ids| ids.len() != 1)
        {
            issue(
                f,
                meta.range.clone(),
                "Identidade de classe ambígua",
                report,
            );
        }
        if f.metadata.dynamic_alias {
            issue(
                f,
                meta.range.clone(),
                "class_alias impede provar propriedades",
                report,
            );
        }
        for relation in &meta.parents {
            match resolve_parent(id, relation, files, &identities, budget) {
                Ok(p)
                    if valid_edge(
                        c.kind,
                        files[p.file].before.classes[p.class].kind,
                        relation.kind,
                    ) =>
                {
                    parents.entry(id).or_default().push((p, relation.kind));
                }
                Ok(_) => issue(
                    f,
                    meta.range.clone(),
                    "Relação incompatível entre classes",
                    report,
                ),
                Err(message) => issue(f, meta.range.clone(), message, report),
            }
        }
    }
    let mut ancestors = Ancestors::new();
    for &id in &all {
        let chain = ancestry(id, &parents, false, budget)?;
        if chain.contains(&id) {
            issue(&files[id.file], 0..0, "Ciclo de herança", report);
        }
        ancestors.insert(id, chain);
    }
    for &(id, m) in &targets {
        let f = &files[id.file];
        let member = &f.before.members[m];
        let after = &f.after.as_ref().unwrap().members[m];
        for &other_id in &all {
            budget.node()?;
            if id == other_id
                || f.language != files[other_id.file].language
                || !related(id, other_id, &ancestors, budget)?
            {
                continue;
            }
            let other_file = &files[other_id.file];
            for reason in &other_file.before.classes[other_id.class].unsupported {
                if reason != "Parameter properties do constructor não verificadas" {
                    issue(other_file, 0..0, reason.clone(), report);
                }
            }
            if member.inheritance_uncertain {
                issue(
                    f,
                    member.range.clone(),
                    "Propriedade private em hierarquia exige seleção lexical específica",
                    report,
                );
            }
            for &n in &other_file.before.classes[other_id.class].members {
                budget.node()?;
                let old = &other_file.before.members[n];
                let new = &other_file
                    .after
                    .as_ref()
                    .unwrap_or(&other_file.before)
                    .members[n];
                if f.language == Language::Php && old.method {
                    continue;
                }
                if f.language == Language::TypeScript && old.static_member != member.static_member {
                    continue;
                }
                if old.name == member.name {
                    if new.name != after.name {
                        fault(
                            other_file,
                            old.range.clone(),
                            ConflictKind::OverrideChanged,
                            "Proposta omite redeclaração da propriedade relacionada",
                            report,
                        );
                    } else if old.method
                        || contract(f, member, budget)?.is_none()
                        || contract(f, member, budget)? != contract(other_file, old, budget)?
                    {
                        issue(
                            f,
                            member.range.clone(),
                            "Redeclarações exigem tipos explícitos equivalentes, visibilidade, static, readonly e optional iguais",
                            report,
                        );
                    }
                } else if new.name == after.name {
                    fault(
                        other_file,
                        old.range.clone(),
                        ConflictKind::InheritedMember,
                        "Novo nome colide com membro de ancestral/descendente",
                        report,
                    );
                }
            }
        }
    }
    // Every edited declaration belongs to one connected nominal family;
    // sibling implementations connect through the edited interface/base.
    if let Some(&(first, _)) = targets.iter().next() {
        let mut reached = HashSet::from([first]);
        loop {
            let mut added = false;
            for &(id, _) in &targets {
                if reached.contains(&id) {
                    continue;
                }
                for &known in &reached.clone() {
                    if related(known, id, &ancestors, budget)? {
                        reached.insert(id);
                        added = true;
                        break;
                    }
                }
            }
            if !added {
                break;
            }
        }
        for &(id, _) in &targets {
            if !reached.contains(&id) {
                issue(
                    &files[id.file],
                    0..0,
                    "Proposta mistura propriedades de famílias independentes",
                    report,
                );
            }
        }
    }
    for &(id, m) in &targets {
        let f = &files[id.file];
        if f.before.classes[id.class].kind != ContainerKind::Interface {
            continue;
        }
        for &candidate in &all {
            if files[candidate.file].before.classes[candidate.class].kind != ContainerKind::Class
                || !ancestors[&candidate].contains(&id)
            {
                continue;
            }
            let mut pending = vec![candidate];
            let mut seen = HashSet::new();
            let mut implementation = false;
            while let Some(current) = pending.pop() {
                budget.node()?;
                if !seen.insert(current) {
                    continue;
                }
                let context = &files[current.file];
                implementation |= context.before.classes[current.class]
                    .members
                    .iter()
                    .any(|&n| {
                        !context.before.members[n].method
                            && context.before.members[n].name == f.before.members[m].name
                            && context.before.members[n].static_member
                                == f.before.members[m].static_member
                    });
                pending.extend(
                    parents
                        .get(&current)
                        .into_iter()
                        .flatten()
                        .filter(|(_, kind)| *kind != RelationKind::Interface)
                        .map(|(p, _)| *p),
                );
            }
            if !implementation {
                issue(
                    &files[candidate.file],
                    0..0,
                    "Classe não fornece implementação concreta da propriedade de interface",
                    report,
                );
            }
        }
    }
    for &(id, m) in &targets {
        check_promoted(
            (id, m),
            files,
            &identities,
            &parents,
            &mut anchored,
            report,
            budget,
        )?;
    }
    for (i, f) in files.iter().enumerate() {
        let Some(names) = names.get(&f.language) else {
            continue;
        };
        let mut pending = vec![f.tree.root_node()];
        while let Some(node) = pending.pop() {
            budget.node()?;
            let pair = match node.kind() {
                "member_expression" => (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("property"),
                ),
                "member_access_expression" | "nullsafe_member_access_expression" => (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("name"),
                ),
                "scoped_property_access_expression" => (
                    node.child_by_field_name("scope"),
                    node.child_by_field_name("name")
                        .and_then(|n| n.named_child(0)),
                ),
                _ => (None, None),
            };
            if let (Some(object), Some(name)) = pair {
                let spelling = text(name, &f.snapshot.before);
                if names.contains(spelling) {
                    let edited = f.edits.iter().any(|e| e.before == name.byte_range());
                    if edited {
                        anchored[i].insert(name.byte_range());
                    }
                    match binding::receiver(i, object, files, &identities, &parents, budget, 0) {
                        Err(message) => issue(f, name.byte_range(), message, report),
                        Ok((class, inferred_static)) => {
                            let static_access = if f.language == Language::Php {
                                node.kind() == "scoped_property_access_expression"
                            } else {
                                inferred_static
                            };
                            let bound = lookup(
                                class,
                                spelling,
                                false,
                                static_access,
                                files,
                                &parents,
                                budget,
                            )?;
                            for &(owner, index) in &bound {
                                let member = &files[owner.file].before.members[index];
                                if member.inheritance_uncertain {
                                    let mut enclosing = Some(node);
                                    let mut in_owner = false;
                                    while let Some(n) = enclosing {
                                        budget.node()?;
                                        if class_kind(n.kind(), f.language) {
                                            in_owner = owner.file == i
                                                && files[owner.file].before.classes[owner.class]
                                                    .start
                                                    == n.start_byte();
                                            break;
                                        }
                                        enclosing = n.parent();
                                    }
                                    if !in_owner {
                                        issue(
                                            f,
                                            name.byte_range(),
                                            "Acesso private fora da classe declaradora não certificado",
                                            report,
                                        );
                                    }
                                }
                            }
                            let changed = bound.iter().any(|p| targets.contains(p));
                            let after_name = f
                                .edits
                                .iter()
                                .find(|e| e.before == name.byte_range())
                                .map_or(spelling, |e| e.new_text.as_str());
                            let after_bound = lookup(
                                class,
                                after_name,
                                true,
                                static_access,
                                files,
                                &parents,
                                budget,
                            )?;
                            if edited && (!changed || bound.iter().any(|p| !targets.contains(p))) {
                                fault(
                                    f,
                                    name.byte_range(),
                                    ConflictKind::BindingChanged,
                                    "Proposta edita propriedade de outro receptor",
                                    report,
                                );
                            } else if changed && !edited {
                                fault(
                                    f,
                                    name.byte_range(),
                                    ConflictKind::MissingReference,
                                    "Proposta omite referência comprovada à propriedade",
                                    report,
                                );
                            } else if bound != after_bound {
                                fault(
                                    f,
                                    name.byte_range(),
                                    ConflictKind::BindingChanged,
                                    "Proposta altera vínculo de referência à propriedade",
                                    report,
                                );
                            }
                        }
                    }
                }
            }
            // Reflection and dynamic lookup can refer to any property without an
            // identifier occurrence; do not pretend the literal list is exhaustive.
            if matches!(
                node.kind(),
                "subscript_expression"
                    | "object_pattern"
                    | "dynamic_variable_name"
                    | "property_hook_list"
                    | "property_hook"
                    | "eval_intrinsic"
            ) || (f.language == Language::Php
                && matches!(
                    node.kind(),
                    "member_access_expression" | "nullsafe_member_access_expression"
                )
                && node
                    .child_by_field_name("name")
                    .is_some_and(|n| n.kind() != "name"))
            {
                issue(
                    f,
                    node.byte_range(),
                    "Acesso computado, destructuring, promoção ou reflexão exige prova adicional de propriedade",
                    report,
                );
            }
            if f.language == Language::Php
                && node.kind() == "method_declaration"
                && node.child_by_field_name("name").is_some_and(|n| {
                    matches!(
                        text(n, &f.snapshot.before).to_ascii_lowercase().as_str(),
                        "__get" | "__set" | "__isset" | "__unset" | "__serialize" | "__unserialize"
                    )
                })
            {
                issue(
                    f,
                    node.byte_range(),
                    "Método mágico observa propriedades por nome",
                    report,
                );
            }
            if matches!(
                node.kind(),
                "function_call_expression"
                    | "call_expression"
                    | "object_creation_expression"
                    | "new_expression"
            ) {
                let callee = node
                    .child_by_field_name("function")
                    .or_else(|| node.child_by_field_name("constructor"))
                    .or_else(|| node.named_child(0));
                if callee.is_some_and(|n| {
                    matches!(
                        text(n, &f.snapshot.before)
                            .trim_start_matches('\\')
                            .to_ascii_lowercase()
                            .as_str(),
                        "eval"
                            | "serialize"
                            | "call_user_func"
                            | "call_user_func_array"
                            | "unserialize"
                            | "get_object_vars"
                            | "property_exists"
                            | "reflectionproperty"
                            | "reflectionclass"
                            | "object.keys"
                            | "object.entries"
                            | "object.values"
                            | "object.assign"
                            | "reflect.get"
                            | "reflect.set"
                            | "json.stringify"
                    )
                }) {
                    issue(
                        f,
                        node.byte_range(),
                        "Reflexão/serialização observa nomes de propriedades",
                        report,
                    );
                }
            }
            let mut cursor = node.walk();
            pending.extend(node.named_children(&mut cursor));
        }
        for edit in &f.edits {
            if !anchored[i].contains(&edit.before) {
                issue(
                    f,
                    edit.before.clone(),
                    "Edição não está ancorada a declaração/acesso da propriedade",
                    report,
                );
            }
        }
    }
    // Reuse the existing TS transfer audit with fields as the selected member
    // category. Its nominal transfer guards must also hold for properties.
    if active.contains(&Language::TypeScript) {
        let mut views = Vec::new();
        for f in files {
            let mut before = f.before.clone();
            let mut after = f.after.clone();
            for member in &mut before.members {
                member.method = !member.method;
            }
            if let Some(after) = &mut after {
                for member in &mut after.members {
                    member.method = !member.method;
                }
            }
            views.push(File {
                tree: f.tree.clone(),
                snapshot: f.snapshot,
                language: f.language,
                before,
                after,
                metadata: inspect(&f.tree, &f.snapshot.before, f.language, budget)?,
                edits: f.edits.clone(),
            });
        }
        binding::check(&views, &parents, &identities, report, budget)?;
        report.scope = ValidationScope::KnownPropertyBindings;
    }
    Ok(())
}

fn check_promoted(
    (id, index): Property,
    files: &[File<'_>],
    identities: &HashMap<(Language, String), Vec<ClassId>>,
    parents: &Parents,
    anchored: &mut [HashSet<Range<usize>>],
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let file = &files[id.file];
    let member = &file.before.members[index];
    let Some(decl) = declaration(file, &member.range, budget)? else {
        return Ok(());
    };
    if !promoted(decl, file.language) {
        return Ok(());
    }
    let mut parent = decl.parent();
    let constructor = loop {
        let Some(node) = parent else {
            issue(
                file,
                member.range.clone(),
                "Promoção fora de constructor",
                report,
            );
            return Ok(());
        };
        if matches!(node.kind(), "method_declaration" | "method_definition") {
            break node;
        }
        parent = node.parent();
    };
    let expected = if file.language == Language::Php {
        "__construct"
    } else {
        "constructor"
    };
    if !constructor
        .child_by_field_name("name")
        .is_some_and(|n| text(n, &file.snapshot.before).eq_ignore_ascii_case(expected))
    {
        issue(
            file,
            member.range.clone(),
            "Promoção somente em constructor",
            report,
        );
        return Ok(());
    }
    let new_name = &file.after.as_ref().unwrap().members[index].name;
    let mut pending = vec![constructor];
    while let Some(node) = pending.pop() {
        budget.node()?;
        if node.id() != constructor.id()
            && matches!(
                node.kind(),
                "function_definition"
                    | "function_declaration"
                    | "function_expression"
                    | "anonymous_function"
                    | "arrow_function"
                    | "generator_function"
                    | "method_definition"
                    | "method_declaration"
                    | "class_declaration"
                    | "class"
            )
        {
            issue(
                file,
                node.byte_range(),
                "Escopo aninhado de parâmetro promovido exige prova lexical própria",
                report,
            );
            continue;
        }
        if matches!(
            node.kind(),
            "by_ref"
                | "reference_modifier"
                | "reference_assignment_expression"
                | "global_declaration"
                | "function_static_declaration"
                | "dynamic_variable_name"
                | "object_pattern"
                | "array_pattern"
                | "rest_pattern"
                | "decorator"
                | "attribute_list"
        ) {
            issue(
                file,
                node.byte_range(),
                "Referências/dinâmica/desestruturação do parâmetro promovido não certificadas",
                report,
            );
        }
        if file.language == Language::TypeScript
            && matches!(
                node.kind(),
                "shorthand_property_identifier" | "shorthand_property_identifier_pattern"
            )
        {
            issue(
                file,
                node.byte_range(),
                "Shorthand do parâmetro promovido exige preservar nome da chave e binding separadamente",
                report,
            );
        }
        if file.language == Language::Php
            && node.kind() == "function_call_expression"
            && node.child_by_field_name("function").is_some_and(|n| {
                !matches!(n.kind(), "name" | "qualified_name" | "relative_name")
                    || matches!(
                        text(n, &file.snapshot.before)
                            .trim_start_matches('\\')
                            .to_ascii_lowercase()
                            .as_str(),
                        "compact" | "extract" | "get_defined_vars" | "parse_str" | "assert"
                    )
            })
        {
            issue(
                file,
                node.byte_range(),
                "Reflexão/dinâmica pode observar o binding do parâmetro promovido",
                report,
            );
        }
        let local = if file.language == Language::Php && node.kind() == "variable_name" {
            if node.parent().is_some_and(|p| {
                p.kind() == "scoped_property_access_expression"
                    && p.child_by_field_name("name")
                        .is_some_and(|n| n.id() == node.id())
            }) {
                None
            } else {
                node.named_child(0)
            }
        } else if file.language == Language::TypeScript && node.kind() == "identifier" {
            Some(node)
        } else {
            None
        };
        if let Some(name) = local {
            let spelling = text(name, &file.snapshot.before);
            if spelling == member.name {
                if file.language == Language::TypeScript
                    && name.byte_range() != member.range
                    && node.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            "variable_declarator"
                                | "catch_clause"
                                | "required_parameter"
                                | "optional_parameter"
                        ) && (p
                            .child_by_field_name("name")
                            .or_else(|| p.child_by_field_name("pattern")))
                        .is_some_and(|n| n.id() == node.id())
                    })
                {
                    issue(
                        file,
                        name.byte_range(),
                        "Shadowing local do parâmetro promovido exige binding lexical separado",
                        report,
                    );
                }
                if file.edits.iter().any(|e| e.before == name.byte_range()) {
                    anchored[id.file].insert(name.byte_range());
                } else {
                    fault(
                        file,
                        name.byte_range(),
                        ConflictKind::MissingReference,
                        "Proposta omite vínculo local do parâmetro promovido",
                        report,
                    );
                }
            } else if spelling == new_name {
                fault(
                    file,
                    name.byte_range(),
                    ConflictKind::DuplicateBinding,
                    "Novo nome captura binding local no constructor",
                    report,
                );
            }
        }
        let mut cursor = node.walk();
        pending.extend(node.named_children(&mut cursor));
    }
    if file.language == Language::Php {
        for (file_index, f) in files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.language == Language::Php)
        {
            let mut pending = vec![f.tree.root_node()];
            while let Some(node) = pending.pop() {
                budget.node()?;
                if node.kind() == "argument"
                    && let Some(name) = node.child_by_field_name("name")
                    && (text(name, &f.snapshot.before) == member.name
                        || text(name, &f.snapshot.before) == new_name)
                {
                    let call = node.parent().and_then(|n| n.parent());
                    if let Some(call) = call.filter(|n| {
                        n.kind() == "object_creation_expression"
                            || (matches!(
                                n.kind(),
                                "member_call_expression" | "scoped_call_expression"
                            ) && n.child_by_field_name("name").is_some_and(|v| {
                                text(v, &f.snapshot.before).eq_ignore_ascii_case("__construct")
                            }))
                    }) {
                        let receiver = if call.kind() == "object_creation_expression" {
                            call
                        } else {
                            call.child_by_field_name("object")
                                .or_else(|| call.child_by_field_name("scope"))
                                .unwrap_or(call)
                        };
                        match binding::receiver(
                            file_index, receiver, files, identities, parents, budget, 0,
                        ) {
                            Ok((mut class, _)) => {
                                let mut seen = HashSet::new();
                                let owner = loop {
                                    budget.node()?;
                                    if !seen.insert(class) {
                                        break None;
                                    }
                                    let context = &files[class.file];
                                    if context.before.classes[class.class].members.iter().any(
                                        |&i| {
                                            context.before.members[i].method
                                                && context.before.members[i]
                                                    .name
                                                    .eq_ignore_ascii_case("__construct")
                                        },
                                    ) {
                                        break Some(class);
                                    }
                                    let p = parents
                                        .get(&class)
                                        .into_iter()
                                        .flatten()
                                        .map(|(p, _)| *p)
                                        .collect::<Vec<_>>();
                                    if p.len() != 1 {
                                        break None;
                                    }
                                    class = p[0];
                                };
                                if owner == Some(id) {
                                    let edited =
                                        f.edits.iter().any(|e| e.before == name.byte_range());
                                    if text(name, &f.snapshot.before) == member.name {
                                        if edited {
                                            anchored[file_index].insert(name.byte_range());
                                        } else {
                                            fault(
                                                f,
                                                name.byte_range(),
                                                ConflictKind::MissingReference,
                                                "Proposta omite argumento nomeado do parâmetro promovido",
                                                report,
                                            );
                                        }
                                    } else {
                                        fault(
                                            f,
                                            name.byte_range(),
                                            ConflictKind::BindingChanged,
                                            "Novo nome captura argumento nomeado existente",
                                            report,
                                        );
                                    }
                                } else if owner.is_none() {
                                    issue(
                                        f,
                                        name.byte_range(),
                                        "Constructor do argumento nomeado não resolvido",
                                        report,
                                    );
                                }
                            }
                            Err(message) => issue(f, name.byte_range(), message, report),
                        }
                    }
                }
                let mut cursor = node.walk();
                pending.extend(node.named_children(&mut cursor));
            }
        }
    }
    Ok(())
}
