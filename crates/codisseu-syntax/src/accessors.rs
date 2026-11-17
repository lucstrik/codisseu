//! PHP accessor proposals are independent method renames over original snapshots.
use super::*;
use crate::{RenameEdit, ValidationLimits};
use std::{
    ops::Range,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Debug)]
pub struct AccessorRenameProposal {
    pub old_name: String,
    pub new_name: String,
    pub snapshots: Vec<RenameSnapshot>,
}

fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
fn tick(budget: &mut Budget<'_>) -> Result<(), String> {
    budget
        .node()
        .map_err(|kind| format!("Planner de accessors interrompido: {kind:?}"))
}

/// Propose get/set/is/has methods associated by property owner and nominal family.
/// Every returned item MUST be validated independently with its own new_name.
/// Inputs contain only the original property's edits; outputs use the same originals.
pub fn plan_property_accessor_renames(
    snapshots: &[RenameSnapshot],
    new_property: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Vec<AccessorRenameProposal>, String> {
    crate::validate_rename_candidate(new_property)?;
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
        return Err("Limite do corpus de accessors excedido".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut report = RenameValidation::new();
    let files = properties::parse_files(snapshots, new_property, &mut report, &mut budget)
        .map_err(|kind| format!("Análise dos accessors interrompida: {kind:?}"))?;
    if !report.issues.is_empty() || !report.conflicts.is_empty() {
        return Err("Snapshots da propriedade inválidos para planejar accessors".into());
    }
    let mut owners = HashSet::new();
    let mut old_property = None;
    for (i, file) in files.iter().enumerate() {
        for member in &file.before.members {
            tick(&mut budget)?;
            if file.language != Language::Php
                || member.method
                || !file.edits.iter().any(|e| e.before == member.range)
            {
                continue;
            }
            if old_property.as_ref().is_some_and(|old| old != &member.name) {
                return Err("Proposta altera propriedades de nomes diferentes".into());
            }
            old_property = Some(member.name.clone());
            owners.insert(ClassId {
                file: i,
                class: member.class,
            });
        }
    }
    let Some(old_property) = old_property else {
        return Ok(vec![]);
    };
    let mut identities = binding::Identities::new();
    let mut all = vec![];
    for (i, file) in files.iter().enumerate() {
        if file.language != Language::Php {
            continue;
        }
        for (j, class) in file.before.classes.iter().enumerate() {
            tick(&mut budget)?;
            let id = ClassId { file: i, class: j };
            let meta = file
                .metadata
                .classes
                .get(&class.start)
                .ok_or("Classe sem identidade nominal")?;
            if let Some(problem) = &meta.problem {
                return Err(problem.clone());
            }
            if file.metadata.dynamic_alias {
                return Err("class_alias impede associação de accessors".into());
            }
            identities
                .entry((file.language, identity(file, meta)))
                .or_default()
                .push(id);
            all.push(id);
        }
    }
    if identities.values().any(|ids| ids.len() != 1) {
        return Err("Classe ambígua no corpus de accessors".into());
    }
    let mut parents = Parents::new();
    for &id in &all {
        tick(&mut budget)?;
        let file = &files[id.file];
        let class = &file.before.classes[id.class];
        let meta = &file.metadata.classes[&class.start];
        for relation in &meta.parents {
            tick(&mut budget)?;
            let parent = resolve_parent(id, relation, &files, &identities, &mut budget)?;
            if !valid_edge(
                class.kind,
                files[parent.file].before.classes[parent.class].kind,
                relation.kind,
            ) {
                return Err("Relação de classes incompatível para accessors".into());
            }
            parents.entry(id).or_default().push((parent, relation.kind));
        }
    }
    let mut ancestors = Ancestors::new();
    for &id in &all {
        let chain = ancestry(id, &parents, false, &mut budget)
            .map_err(|k| format!("Grafo de accessors interrompido: {k:?}"))?;
        if chain.contains(&id) {
            return Err("Ciclo de herança nos accessors".into());
        }
        ancestors.insert(id, chain);
    }
    let mut proposals = vec![];
    for prefix in ["get", "set", "is", "has"] {
        tick(&mut budget)?;
        let old_name = format!("{prefix}{}", capitalized(&old_property));
        let new_name = format!("{prefix}{}", capitalized(new_property));
        if old_name == new_name {
            continue;
        }
        let mut targets = HashSet::new();
        // Seed only methods declared on actual property owners. An inherited
        // same-named method can read a distinct private ancestor property.
        for &owner in &owners {
            for &id in &all {
                tick(&mut budget)?;
                if id != owner {
                    continue;
                }
                let file = &files[id.file];
                for &m in &file.before.classes[id.class].members {
                    tick(&mut budget)?;
                    let member = &file.before.members[m];
                    if member.method && member.name.eq_ignore_ascii_case(&old_name) {
                        targets.insert((id, m));
                    }
                }
            }
        }
        if targets.is_empty() {
            continue;
        }
        // Expand the proven nominal method family; ordinary method validation
        // subsequently checks contracts, private/trait behavior and collisions.
        loop {
            let old_len = targets.len();
            let known: Vec<_> = targets.iter().copied().collect();
            for &id in &all {
                for &(target, _) in &known {
                    tick(&mut budget)?;
                    if id != target
                        && !related(id, target, &ancestors, &mut budget)
                            .map_err(|k| format!("Família excede orçamento: {k:?}"))?
                    {
                        continue;
                    }
                    for &m in &files[id.file].before.classes[id.class].members {
                        tick(&mut budget)?;
                        let member = &files[id.file].before.members[m];
                        if member.method && member.name.eq_ignore_ascii_case(&old_name) {
                            targets.insert((id, m));
                        }
                    }
                }
            }
            if old_len == targets.len() {
                break;
            }
        }
        let mut ranges: Vec<Vec<Range<usize>>> = files.iter().map(|_| vec![]).collect();
        for &(id, m) in &targets {
            ranges[id.file].push(files[id.file].before.members[m].range.clone());
        }
        for (i, file) in files.iter().enumerate() {
            if file.language != Language::Php {
                continue;
            }
            let mut pending = vec![file.tree.root_node()];
            while let Some(node) = pending.pop() {
                tick(&mut budget)?;
                let pair = match node.kind() {
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
                    && name.kind() == "name"
                    && text(name, &file.snapshot.before).eq_ignore_ascii_case(&old_name)
                {
                    let (id, static_access) = binding::receiver(
                        i,
                        object,
                        &files,
                        &identities,
                        &parents,
                        &mut budget,
                        0,
                    )?;
                    let bound = binding::methods(
                        id,
                        &old_name,
                        false,
                        static_access,
                        &files,
                        &parents,
                        &mut budget,
                    )
                    .map_err(|k| format!("Referências de accessor interrompidas: {k:?}"))?;
                    if bound.iter().any(|m| targets.contains(m)) {
                        if bound.iter().any(|m| !targets.contains(m)) {
                            return Err("Referência de accessor ambígua".into());
                        }
                        ranges[i].push(name.byte_range());
                    }
                }
                let mut cursor = node.walk();
                pending.extend(node.named_children(&mut cursor));
            }
        }
        if ranges.iter().map(Vec::len).sum::<usize>() > 1024 {
            return Err("Limite de edições de accessor excedido".into());
        }
        let mut proposed: Vec<_> = snapshots
            .iter()
            .map(|s| RenameSnapshot {
                path: s.path.clone(),
                before: s.before.clone(),
                after: s.before.clone(),
                edits: vec![],
            })
            .collect();
        for (i, file) in files.iter().enumerate() {
            tick(&mut budget)?;
            ranges[i].sort_by_key(|r| r.start);
            ranges[i].dedup();
            let snapshot = proposed
                .iter_mut()
                .find(|s| s.path == file.snapshot.path)
                .ok_or("Snapshot de accessor ausente")?;
            let mut after = snapshot.before.to_string();
            for range in ranges[i].iter().rev() {
                after.replace_range(range.clone(), &new_name);
            }
            snapshot.edits = ranges[i]
                .iter()
                .map(|range| RenameEdit {
                    range: text_range(&snapshot.before, range.clone()),
                    new_text: new_name.clone(),
                })
                .collect();
            snapshot.after = Arc::from(after);
        }
        proposals.push(AccessorRenameProposal {
            old_name,
            new_name,
            snapshots: proposed,
        });
    }
    Ok(proposals)
}
