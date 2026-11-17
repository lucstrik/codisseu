//! Optional PHP direct-inheritor naming proposals, independently validated.
//! Naming policy is grounded in the isolated PhpStorm observations recorded in
//! docs/m3-rename-inheritors-contract.md; identity/reference edits are our ASTs.
use super::*;
use crate::{TextRange, ValidationLimits};
use std::{sync::atomic::AtomicBool, time::Instant};

#[derive(Clone, Debug)]
pub struct ClassInheritorRenameProposal {
    pub path: PathBuf,
    pub range: TextRange,
    pub old_name: String,
    pub display_name: String,
    pub new_name: String,
    /// Independent class rename over the original corpus. NOT approved: validate
    /// using this item's new_name before combining with the primary proposal.
    pub snapshots: Vec<RenameSnapshot>,
}
fn tick(budget: &mut Budget<'_>) -> Result<(), String> {
    budget
        .node()
        .map_err(|k| format!("Planner de herdeiros interrompido: {k:?}"))
}
/// Deliberately only the observed PascalCase word pattern, not arbitrary prefix
/// replacement. Whole words match case-sensitively and the LAST match wins.
fn words(name: &str) -> Option<Vec<std::ops::Range<usize>>> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    let bytes = name.as_bytes();
    let mut result = vec![];
    let mut start = 0;
    while start < bytes.len() {
        if !bytes[start].is_ascii_uppercase() {
            return None;
        }
        let mut end = start + 1;
        while end < bytes.len() && bytes[end].is_ascii_lowercase() {
            end += 1;
        }
        if end == start + 1 {
            return None;
        }
        result.push(start..end);
        start = end;
    }
    Some(result)
}
fn suggested(candidate: &str, old: &str, new: &str) -> Option<String> {
    let candidate_words = words(candidate)?;
    let old_words = words(old)?;
    words(new)?;
    let last = candidate_words
        .windows(old_words.len())
        .rev()
        .find(|window| {
            window
                .iter()
                .zip(&old_words)
                .all(|(a, b)| candidate[a.clone()] == old[b.clone()])
        })?;
    let range = last.first()?.start..last.last()?.end;
    let mut result = candidate.to_owned();
    result.replace_range(range, new);
    (result != candidate).then_some(result)
}

/// Selects direct nominal PHP subclasses only. Class/file rows and custom names
/// are caller decisions; use the normal class/resource planners for each selected
/// row, keep original snapshots, and replan on Apply. No filesystem I/O occurs.
pub fn plan_class_inheritor_renames(
    snapshots: &[RenameSnapshot],
    new_root: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Vec<ClassInheritorRenameProposal>, String> {
    crate::validate_rename_candidate(new_root)?;
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
        return Err("Corpus de herdeiros excede 64 arquivos/1 MiB/8 MiB/1024 edits".into());
    }
    let started = Instant::now();
    let mut budget = Budget::new(
        cancel,
        ValidationLimits {
            max_nodes: limits.max_nodes / 2,
            ..limits
        },
    );
    let mut report = RenameValidation::new();
    let files = properties::parse_files(snapshots, new_root, &mut report, &mut budget)
        .map_err(|k| format!("Parse de herdeiros interrompido: {k:?}"))?;
    if !report.issues.is_empty() || !report.conflicts.is_empty() {
        return Err("Snapshots primários inválidos no planner de herdeiros".into());
    }
    let mut roots = vec![];
    let mut all = vec![];
    let mut identities = binding::Identities::new();
    for (file_index, file) in files.iter().enumerate() {
        if file.language != Language::Php {
            continue;
        }
        for (class_index, class) in file.before.classes.iter().enumerate() {
            tick(&mut budget)?;
            let id = ClassId {
                file: file_index,
                class: class_index,
            };
            let meta = file
                .metadata
                .classes
                .get(&class.start)
                .ok_or("Classe sem identidade")?;
            identities
                .entry((Language::Php, identity(file, meta)))
                .or_default()
                .push(id);
            all.push(id);
            if class.kind == ContainerKind::Class
                && file.edits.iter().any(|e| e.before == meta.range)
            {
                roots.push(id);
            }
        }
    }
    if roots.is_empty() {
        return Ok(vec![]);
    }
    if roots.len() != 1 {
        return Err("Herdeiros exigem uma única classe PHP primária".into());
    }
    if identities.values().any(|ids| ids.len() != 1) {
        return Err("Identidade PHP ambígua no corpus de herdeiros".into());
    }
    let root = roots[0];
    let root_class = &files[root.file].before.classes[root.class];
    let old_root = root_class
        .name
        .as_deref()
        .ok_or("Classe anônima não tem sugestões")?;
    let mut parents = Parents::new();
    for &id in &all {
        tick(&mut budget)?;
        let file = &files[id.file];
        let class = &file.before.classes[id.class];
        let meta = &file.metadata.classes[&class.start];
        if let Some(problem) = &meta.problem {
            return Err(problem.clone());
        }
        if file.metadata.dynamic_alias {
            return Err("class_alias dinâmico impede resolver herdeiros".into());
        }
        for relation in &meta.parents {
            let parent = resolve_parent(id, relation, &files, &identities, &mut budget)?;
            if !valid_edge(
                class.kind,
                files[parent.file].before.classes[parent.class].kind,
                relation.kind,
            ) {
                return Err("Relação nominal de herdeiro incompatível".into());
            }
            parents.entry(id).or_default().push((parent, relation.kind));
        }
    }
    let mut candidates = vec![];
    for &id in &all {
        tick(&mut budget)?;
        if ancestry(id, &parents, false, &mut budget)
            .map_err(|k| format!("Grafo de herdeiros interrompido: {k:?}"))?
            .contains(&id)
        {
            return Err("Ciclo de classes no corpus de herdeiros".into());
        }
        if !parents
            .get(&id)
            .is_some_and(|p| p.contains(&(root, RelationKind::ExtendsClass)))
        {
            continue;
        }
        let file = &files[id.file];
        let class = &file.before.classes[id.class];
        if class.kind != ContainerKind::Class {
            continue;
        }
        let old_name = class.name.as_deref().ok_or("Herdeiro anônimo")?;
        if let Some(new_name) = suggested(old_name, old_root, new_root) {
            let meta = &file.metadata.classes[&class.start];
            candidates.push((
                file.snapshot.path.clone(),
                text_range(&file.snapshot.before, meta.range.clone()),
                old_name.to_owned(),
                new_name,
                meta.identity.clone(),
            ));
        }
    }
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.start_line.cmp(&b.1.start_line))
            .then(a.1.start_character.cmp(&b.1.start_character))
    });
    if candidates.len() > 32 {
        return Err("Rename excede 32 herdeiros opcionais".into());
    }
    let originals = snapshots
        .iter()
        .map(|s| RenameSnapshot {
            path: s.path.clone(),
            before: s.before.clone(),
            after: s.before.clone(),
            edits: vec![],
        })
        .collect::<Vec<_>>();
    let per_proposal_nodes = (limits.max_nodes - limits.max_nodes / 2) / candidates.len().max(1);
    let mut result = vec![];
    for (path, range, old_name, new_name, display_name) in candidates {
        tick(&mut budget)?;
        let remaining = limits
            .max_duration
            .checked_sub(started.elapsed())
            .ok_or("Tempo de herdeiros excedido")?;
        let proposed = crate::propose_global_rename_with_limits(
            &originals,
            &path,
            &range,
            &new_name,
            cancel,
            ValidationLimits {
                max_nodes: per_proposal_nodes,
                max_duration: remaining,
            },
        )?
        .ok_or("Herdeiro não resolvido pelo planner de classe")?;
        result.push(ClassInheritorRenameProposal {
            path,
            range,
            old_name,
            display_name,
            new_name,
            snapshots: proposed,
        });
    }
    budget
        .check()
        .map_err(|k| format!("Planner de herdeiros interrompido: {k:?}"))?;
    Ok(result)
}
