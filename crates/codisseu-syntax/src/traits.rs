//! PHP trait adaptation proof over immutable syntax trees and explicit identities.
//! Aliases are independent method symbols; `as` never renames the source method.
use super::*;
use crate::{Coverage, ValidationLimits};
use std::{ops::Range, sync::atomic::AtomicBool};

type SymbolId = usize;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Visibility {
    Public,
    Protected,
    Private,
}
#[derive(Clone)]
struct Symbol {
    owner: ClassId,
    range: Range<usize>,
    name: String,
    after_name: String,
    member: Option<usize>,
    visibility: Visibility,
    static_member: bool,
    abstract_member: bool,
    final_member: bool,
}
#[derive(Clone)]
struct Adaptation {
    owner: ClassId,
    qualifier: Option<ClassId>,
    selector: Range<usize>,
    name: String,
    after_name: String,
    alias: Option<SymbolId>,
    visibility: Option<Visibility>,
    excluded: Vec<ClassId>,
}
#[derive(Clone)]
struct UseGroup {
    traits: Vec<ClassId>,
    adaptations: Vec<usize>,
}
#[derive(Default)]
struct Config {
    own: Vec<SymbolId>,
    groups: Vec<UseGroup>,
    parent: Option<ClassId>,
    interfaces: Vec<ClassId>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct Slot {
    symbol: SymbolId,
    body: SymbolId,
    owner: ClassId,
    visibility: Visibility,
    static_member: bool,
    abstract_member: bool,
}
type Slots = HashMap<String, Slot>;
struct Model {
    slots: HashMap<ClassId, Slots>,
    selectors: HashMap<usize, SymbolId>,
    overrides: HashSet<(SymbolId, SymbolId)>,
}
struct Corpus<'a> {
    files: Vec<File<'a>>,
    symbols: Vec<Symbol>,
    adaptations: Vec<Adaptation>,
    configs: HashMap<ClassId, Config>,
    identities: binding::Identities,
    parents: Parents,
}
fn tick(budget: &mut Budget<'_>) -> Result<(), String> {
    budget
        .node()
        .map_err(|kind| format!("Análise de traits interrompida: {kind:?}"))
}
fn visibility(node: Node<'_>, source: &str) -> Visibility {
    let mut c = node.walk();
    for child in node.named_children(&mut c) {
        if child.kind() == "visibility_modifier" {
            return match text(child, source) {
                "private" => Visibility::Private,
                "protected" => Visibility::Protected,
                _ => Visibility::Public,
            };
        }
    }
    Visibility::Public
}
fn named_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c)
        .filter(|n| n.kind() != "comment")
        .collect()
}
fn changed_name(file: &File<'_>, range: &Range<usize>, name: &str) -> String {
    file.edits.iter().find(|e| e.before == *range).map_or_else(
        || name.to_owned(),
        |e| file.snapshot.after[e.after.clone()].to_owned(),
    )
}
fn ancestor_class(node: Node<'_>, file: usize, files: &[File<'_>]) -> Option<ClassId> {
    let mut current = Some(node);
    while let Some(node) = current {
        if let Some(class) = files[file]
            .before
            .classes
            .iter()
            .position(|c| c.start == node.start_byte())
        {
            return Some(ClassId { file, class });
        }
        current = node.parent();
    }
    None
}
fn php_lookup(
    owner: ClassId,
    name: &str,
    files: &[File<'_>],
    identities: &binding::Identities,
) -> Result<ClassId, String> {
    let file = &files[owner.file];
    let meta = &file.metadata.classes[&file.before.classes[owner.class].start];
    match identities.get(&(Language::Php, php_parent(meta, name)?)) {
        Some(ids) if ids.len() == 1 => Ok(ids[0]),
        _ => Err(format!(
            "Tipo PHP ‘{name}’ ausente/ambíguo no corpus de traits"
        )),
    }
}
impl<'a> Corpus<'a> {
    fn build(files: Vec<File<'a>>, budget: &mut Budget<'_>) -> Result<Self, String> {
        let mut symbols = vec![];
        let mut identities = binding::Identities::new();
        let mut configs = HashMap::new();
        let mut parents = Parents::new();
        for (i, file) in files.iter().enumerate() {
            if file.language != Language::Php {
                continue;
            }
            if file.metadata.dynamic_alias {
                return Err("class_alias dinâmico impede resolver traits".into());
            }
            for (j, class) in file.before.classes.iter().enumerate() {
                tick(budget)?;
                let id = ClassId { file: i, class: j };
                if class.kind == ContainerKind::Unsupported || !class.unsupported.is_empty() {
                    return Err(format!(
                        "Contêiner de trait não verificado: {:?}",
                        class.unsupported
                    ));
                }
                let meta = file
                    .metadata
                    .classes
                    .get(&class.start)
                    .ok_or("Classe sem identidade")?;
                identities
                    .entry((Language::Php, identity(file, meta)))
                    .or_default()
                    .push(id);
                configs.insert(id, Config::default());
            }
        }
        if identities.values().any(|ids| ids.len() != 1) {
            return Err("Identidade PHP duplicada no corpus de traits".into());
        }
        let mut adaptations = vec![];
        for (i, file) in files.iter().enumerate() {
            if file.language != Language::Php {
                continue;
            }
            let source = &file.snapshot.before;
            let mut stack = vec![file.tree.root_node()];
            while let Some(node) = stack.pop() {
                tick(budget)?;
                if class_kind(node.kind(), Language::Php) {
                    let owner = ancestor_class(node, i, &files).ok_or("Classe não indexada")?;
                    let class = &file.before.classes[owner.class];
                    let (namespace, _, problem) = php_context(node, source, budget)
                        .map_err(|_| "Contexto PHP interrompido")?;
                    if let Some(problem) = problem {
                        return Err(problem);
                    }
                    let parent = node.parent().ok_or("Classe sem programa")?;
                    if parent.kind() != "program"
                        && !(parent.kind() == "compound_statement"
                            && parent
                                .parent()
                                .is_some_and(|n| n.kind() == "namespace_definition"))
                    {
                        return Err("Classe/trait condicional ou aninhada não verificada".into());
                    }
                    let meta = &file.metadata.classes[&class.start];
                    if meta.namespace != namespace {
                        return Err("Namespace de trait inconsistente".into());
                    }
                    for relation in &meta.parents {
                        let target = php_lookup(owner, &relation.name, &files, &identities)?;
                        if !valid_edge(
                            class.kind,
                            files[target.file].before.classes[target.class].kind,
                            relation.kind,
                        ) {
                            return Err("Relação de trait aponta para espécie incompatível".into());
                        }
                        let list = parents.entry(owner).or_default();
                        if list.iter().any(|(id, _)| *id == target) {
                            return Err("Relação de trait duplicada".into());
                        }
                        list.push((target, relation.kind));
                        if relation.kind == RelationKind::ExtendsClass {
                            configs.get_mut(&owner).unwrap().parent = Some(target);
                        }
                        if relation.kind == RelationKind::Interface {
                            configs.get_mut(&owner).unwrap().interfaces.push(target);
                        }
                    }
                }
                if node.kind() == "method_declaration" {
                    let owner = ancestor_class(node, i, &files).ok_or("Método sem classe")?;
                    let name = node.child_by_field_name("name").ok_or("Método sem nome")?;
                    let member = file
                        .before
                        .members
                        .iter()
                        .position(|m| m.method && m.range == name.byte_range())
                        .ok_or("Método não indexado")?;
                    let fact = &file.before.members[member];
                    let symbol = symbols.len();
                    symbols.push(Symbol {
                        owner,
                        range: name.byte_range(),
                        name: text(name, source).into(),
                        after_name: changed_name(file, &name.byte_range(), text(name, source)),
                        member: Some(member),
                        visibility: visibility(node, source),
                        static_member: token(node, "static_modifier"),
                        final_member: token(node, "final_modifier"),
                        abstract_member: fact.abstract_member
                            || file.before.classes[owner.class].kind == ContainerKind::Interface,
                    });
                    configs.get_mut(&owner).unwrap().own.push(symbol);
                }
                if node.kind() == "use_declaration" {
                    let owner =
                        ancestor_class(node, i, &files).ok_or("Use de trait fora de classe")?;
                    let mut group = UseGroup {
                        traits: vec![],
                        adaptations: vec![],
                    };
                    for child in named_children(node) {
                        if child.kind() == "use_list" {
                            continue;
                        }
                        if !matches!(child.kind(), "name" | "qualified_name" | "relative_name") {
                            return Err("Use de trait dinâmico não verificado".into());
                        }
                        let target = php_lookup(owner, text(child, source), &files, &identities)?;
                        if files[target.file].before.classes[target.class].kind
                            != ContainerKind::Trait
                        {
                            return Err("Use exige declaração trait".into());
                        }
                        group.traits.push(target);
                    }
                    for list in named_children(node)
                        .into_iter()
                        .filter(|n| n.kind() == "use_list")
                    {
                        for clause in named_children(list) {
                            tick(budget)?;
                            if !matches!(clause.kind(), "use_as_clause" | "use_instead_of_clause") {
                                return Err("Adaptação de trait desconhecida".into());
                            }
                            let children = named_children(clause);
                            let first = *children.first().ok_or("Adaptação sem seletor")?;
                            let (qualifier, selector) = if first.kind()
                                == "class_constant_access_expression"
                            {
                                let parts = named_children(first);
                                if parts.len() != 2
                                    || !matches!(
                                        parts[0].kind(),
                                        "name" | "qualified_name" | "relative_name"
                                    )
                                    || parts[1].kind() != "name"
                                {
                                    return Err("Seletor dinâmico de trait não verificado".into());
                                }
                                (
                                    Some(php_lookup(
                                        owner,
                                        text(parts[0], source),
                                        &files,
                                        &identities,
                                    )?),
                                    parts[1],
                                )
                            } else if first.kind() == "name" {
                                (None, first)
                            } else {
                                return Err("Seletor de trait inválido".into());
                            };
                            if qualifier.is_some_and(|q| !group.traits.contains(&q)) {
                                return Err("Seletor aponta para trait fora do mesmo use".into());
                            }
                            let mut adaptation = Adaptation {
                                owner,
                                qualifier,
                                selector: selector.byte_range(),
                                name: text(selector, source).into(),
                                after_name: changed_name(
                                    file,
                                    &selector.byte_range(),
                                    text(selector, source),
                                ),
                                alias: None,
                                visibility: None,
                                excluded: vec![],
                            };
                            if clause.kind() == "use_instead_of_clause" {
                                if qualifier.is_none() {
                                    return Err("insteadof exige trait qualificada".into());
                                }
                                for excluded in &children[1..] {
                                    if excluded.kind() != "name" {
                                        return Err("Exclusão de trait não nominal".into());
                                    }
                                    let target = php_lookup(
                                        owner,
                                        text(*excluded, source),
                                        &files,
                                        &identities,
                                    )?;
                                    if !group.traits.contains(&target) || Some(target) == qualifier
                                    {
                                        return Err(
                                            "insteadof exclui trait ausente ou selecionada".into(),
                                        );
                                    }
                                    adaptation.excluded.push(target);
                                }
                                if adaptation.excluded.is_empty() {
                                    return Err("insteadof sem exclusão".into());
                                }
                            } else {
                                for part in &children[1..] {
                                    if part.kind() == "visibility_modifier" {
                                        adaptation.visibility = Some(visibility(clause, source));
                                    } else if part.kind() == "name" && adaptation.alias.is_none() {
                                        if text(*part, source).eq_ignore_ascii_case("final") {
                                            return Err("Adaptação as final exige gramática PHP 8.3 específica".into());
                                        }
                                        let symbol = symbols.len();
                                        adaptation.alias = Some(symbol);
                                        symbols.push(Symbol {
                                            owner,
                                            range: part.byte_range(),
                                            name: text(*part, source).into(),
                                            after_name: changed_name(
                                                file,
                                                &part.byte_range(),
                                                text(*part, source),
                                            ),
                                            member: None,
                                            visibility: adaptation
                                                .visibility
                                                .unwrap_or(Visibility::Public),
                                            static_member: false,
                                            abstract_member: false,
                                            final_member: false,
                                        });
                                    } else {
                                        return Err("Adaptação as não reconhecida".into());
                                    }
                                }
                                if adaptation.alias.is_none() && adaptation.visibility.is_none() {
                                    return Err("Alias/visibilidade ausente".into());
                                }
                            }
                            group.adaptations.push(adaptations.len());
                            adaptations.push(adaptation);
                        }
                    }
                    configs.get_mut(&owner).unwrap().groups.push(group);
                }
                let mut c = node.walk();
                stack.extend(node.named_children(&mut c));
            }
        }
        // This model supports only class/interface/trait edges and rejects cycles
        // before composing any effective member table.
        for &id in configs.keys() {
            if ancestry(id, &parents, false, budget)
                .map_err(|_| "Limite de ancestralidade")?
                .contains(&id)
            {
                return Err("Ciclo de traits/herança".into());
            }
        }
        Ok(Self {
            files,
            symbols,
            adaptations,
            configs,
            identities,
            parents,
        })
    }
    fn name(&self, symbol: SymbolId, after: bool) -> String {
        let s = &self.symbols[symbol];
        if after { &s.after_name } else { &s.name }.to_ascii_lowercase()
    }
    fn contract(&self, slot: &Slot) -> Result<crate::contracts::MethodContract, String> {
        let body = &self.symbols[slot.body];
        let member = body.member.ok_or("Corpo de alias não resolvido")?;
        let mut contract = self.files[body.owner.file].before.members[member]
            .contract
            .clone()
            .map_err(str::to_owned)?;
        contract.visibility = match slot.visibility {
            Visibility::Public => "public",
            Visibility::Protected => "protected",
            Visibility::Private => "private",
        }
        .into();
        Ok(contract)
    }
    fn compose(&self, after: bool, budget: &mut Budget<'_>) -> Result<Model, String> {
        let mut model = Model {
            slots: HashMap::new(),
            selectors: HashMap::new(),
            overrides: HashSet::new(),
        };
        let mut pending: Vec<_> = self.configs.keys().copied().collect();
        let mut visiting = HashSet::new();
        while let Some(id) = pending.pop() {
            self.compose_class(id, after, &mut model, &mut visiting, budget)?;
        }
        Ok(model)
    }
    fn compose_class(
        &self,
        id: ClassId,
        after: bool,
        model: &mut Model,
        visiting: &mut HashSet<ClassId>,
        budget: &mut Budget<'_>,
    ) -> Result<(), String> {
        tick(budget)?;
        if model.slots.contains_key(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err("Ciclo de composição".into());
        }
        let config = &self.configs[&id];
        for (parent, _) in self.parents.get(&id).into_iter().flatten() {
            self.compose_class(*parent, after, model, visiting, budget)?;
        }
        let mut inherited = config
            .parent
            .map(|p| model.slots[&p].clone())
            .unwrap_or_default();
        let mut imported = Slots::new();
        for group in &config.groups {
            let mut candidates: HashMap<String, Vec<(ClassId, Slot)>> = HashMap::new();
            for trait_id in &group.traits {
                for (name, slot) in &model.slots[trait_id] {
                    tick(budget)?;
                    let mut slot = slot.clone();
                    slot.owner = id;
                    candidates
                        .entry(name.clone())
                        .or_default()
                        .push((*trait_id, slot));
                }
            }
            let original = candidates.clone();
            for &index in &group.adaptations {
                let adaptation = &self.adaptations[index];
                if adaptation.excluded.is_empty() {
                    continue;
                }
                let name = if after {
                    &adaptation.after_name
                } else {
                    &adaptation.name
                }
                .to_ascii_lowercase();
                let selected = original
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .filter(|(t, _)| Some(*t) == adaptation.qualifier)
                    .collect::<Vec<_>>();
                if selected.len() != 1 {
                    return Err(format!("insteadof: seletor {} não resolve um método", name));
                }
                model.selectors.insert(index, selected[0].1.symbol);
                if let Some(list) = candidates.get_mut(&name) {
                    list.retain(|(t, _)| !adaptation.excluded.contains(t));
                }
            }
            for &index in &group.adaptations {
                let a = &self.adaptations[index];
                if a.excluded.is_empty() {
                    continue;
                }
                let name = if after { &a.after_name } else { &a.name }.to_ascii_lowercase();
                if !candidates
                    .get(&name)
                    .is_some_and(|list| list.iter().any(|(id, _)| Some(*id) == a.qualifier))
                {
                    return Err("insteadof contraditório exclui o próprio vencedor".into());
                }
            }
            let mut selected = Slots::new();
            for (name, list) in &candidates {
                tick(budget)?;
                let mut unique = list.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>();
                unique.sort_by_key(|s| s.symbol);
                unique.dedup_by_key(|s| s.symbol);
                if unique.len() > 1
                    && let Some(&own) = config.own.iter().find(|&&sym| {
                        self.name(sym, after) == *name && !self.symbols[sym].abstract_member
                    })
                {
                    for slot in &unique {
                        model.overrides.insert((own, slot.symbol));
                    }
                    continue;
                }
                let concrete = unique
                    .iter()
                    .filter(|s| !s.abstract_member)
                    .collect::<Vec<_>>();
                let chosen = if unique.len() == 1 {
                    &unique[0]
                } else if concrete.len() == 1
                    && unique.iter().filter(|s| s.abstract_member).all(|s| {
                        self.contract(s).is_ok() && self.contract(s) == self.contract(concrete[0])
                    })
                {
                    for abstract_slot in unique.iter().filter(|s| s.abstract_member) {
                        model
                            .overrides
                            .insert((concrete[0].symbol, abstract_slot.symbol));
                    }
                    concrete[0]
                } else {
                    return Err(format!(
                        "Traits fornecem implementações concorrentes de ‘{name}’ sem insteadof completo"
                    ));
                };
                selected.insert(name.clone(), chosen.clone());
            }
            for &index in &group.adaptations {
                let adaptation = &self.adaptations[index];
                if !adaptation.excluded.is_empty() {
                    continue;
                }
                let name = if after {
                    &adaptation.after_name
                } else {
                    &adaptation.name
                }
                .to_ascii_lowercase();
                let mut slot = if let Some(trait_id) = adaptation.qualifier {
                    let list = original
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .filter(|(t, _)| *t == trait_id)
                        .collect::<Vec<_>>();
                    if list.len() != 1 {
                        return Err(format!(
                            "Alias: seletor ‘{name}’ não resolve método na trait"
                        ));
                    }
                    list[0].1.clone()
                } else {
                    selected
                        .get(&name)
                        .cloned()
                        .ok_or_else(|| format!("Alias não qualificado ‘{name}’ ausente/ambíguo"))?
                };
                model.selectors.insert(index, slot.symbol);
                if let Some(alias) = adaptation.alias {
                    slot.symbol = alias;
                    if let Some(visibility) = adaptation.visibility {
                        slot.visibility = visibility;
                    }
                    let alias_name = self.name(alias, after);
                    if selected.contains_key(&alias_name) {
                        return Err(format!(
                            "Alias de trait ‘{alias_name}’ colide com método importado"
                        ));
                    }
                    selected.insert(alias_name, slot);
                } else {
                    let existing = selected
                        .get_mut(&name)
                        .ok_or("Alteração de visibilidade aponta para método excluído")?;
                    if existing.symbol != slot.symbol {
                        return Err("Visibilidade de trait perdedora não selecionada".into());
                    }
                    existing.visibility = adaptation.visibility.unwrap();
                }
            }
            for (name, slot) in selected {
                if let Some(previous) = imported.insert(name.clone(), slot.clone())
                    && previous.symbol != slot.symbol
                {
                    return Err(format!("Traits em grupos separados colidem em ‘{name}’"));
                }
            }
        }
        for (name, slot) in imported {
            if let Some(previous) = inherited.insert(name, slot.clone())
                && previous.visibility != Visibility::Private
                && previous.symbol != slot.symbol
            {
                if self.symbols[previous.body].final_member {
                    return Err("Trait sobrescreve método final herdado".into());
                }
                model.overrides.insert((slot.symbol, previous.symbol));
            }
        }
        let mut own = HashSet::new();
        for &symbol in &config.own {
            tick(budget)?;
            let s = &self.symbols[symbol];
            let name = self.name(symbol, after);
            if !own.insert(name.clone()) {
                return Err(format!("Classe/trait contém método duplicado ‘{name}’"));
            }
            let slot = Slot {
                symbol,
                body: symbol,
                owner: id,
                visibility: s.visibility,
                static_member: s.static_member,
                abstract_member: s.abstract_member,
            };
            if let Some(previous) = inherited.insert(name, slot.clone())
                && previous.symbol != symbol
                && (previous.visibility != Visibility::Private || previous.owner == id)
            {
                if previous.owner != id && self.symbols[previous.body].final_member {
                    return Err("Método sobrescreve ancestral final".into());
                }
                model.overrides.insert((symbol, previous.symbol));
            }
        }
        // Interfaces impose obligations on the actual selected implementation.
        for interface in &config.interfaces {
            for (name, requirement) in &model.slots[interface] {
                tick(budget)?;
                if let Some(implementation) = inherited.get(name) {
                    if implementation.symbol != requirement.symbol {
                        model
                            .overrides
                            .insert((implementation.symbol, requirement.symbol));
                    }
                } else {
                    inherited.insert(name.clone(), requirement.clone());
                }
            }
        }
        let class = &self.files[id.file].before.classes[id.class];
        if class.kind == ContainerKind::Class
            && !class.abstract_class
            && inherited.values().any(|s| s.abstract_member)
        {
            return Err(
                "Classe concreta não implementa requisito abstrato/interface de trait".into(),
            );
        }
        model.slots.insert(id, inherited);
        visiting.remove(&id);
        Ok(())
    }
}
fn conflict_at(
    corpus: &Corpus<'_>,
    file: usize,
    range: Range<usize>,
    kind: ConflictKind,
    message: impl Into<String>,
    report: &mut RenameValidation,
) {
    if report.conflicts.len() >= 256 {
        return;
    }
    report.conflicts.push(Conflict {
        path: corpus.files[file].snapshot.path.clone(),
        range: text_range(&corpus.files[file].snapshot.before, range),
        message: message.into(),
        related: None,
        kind,
        container_name: None,
    });
}
fn edited(file: &File<'_>, range: &Range<usize>) -> bool {
    file.edits.iter().any(|e| e.before == *range)
}
fn required_edit(
    corpus: &Corpus<'_>,
    file: usize,
    range: Range<usize>,
    required: bool,
    report: &mut RenameValidation,
) {
    match (required, edited(&corpus.files[file], &range)) {
        (true, false) => conflict_at(
            corpus,
            file,
            range,
            ConflictKind::MissingReference,
            "Proposta omite referência/adaptação vinculada ao símbolo de trait",
            report,
        ),
        (false, true) => conflict_at(
            corpus,
            file,
            range,
            ConflictKind::BindingChanged,
            "Proposta altera símbolo independente ou alias que deve permanecer estável",
            report,
        ),
        _ => {}
    }
}
fn enclosing_method(corpus: &Corpus<'_>, file: usize, node: Node<'_>) -> Option<SymbolId> {
    let mut current = Some(node);
    while let Some(node) = current {
        if node.kind() == "method_declaration" {
            let range = node.child_by_field_name("name")?.byte_range();
            return corpus
                .symbols
                .iter()
                .position(|s| s.owner.file == file && s.member.is_some() && s.range == range);
        }
        if matches!(
            node.kind(),
            "anonymous_function" | "arrow_function" | "function_definition"
        ) {
            return None;
        }
        current = node.parent();
    }
    None
}
fn contexts(
    corpus: &Corpus<'_>,
    model: &Model,
    file: usize,
    node: Node<'_>,
    receiver: ClassId,
) -> Result<Vec<ClassId>, String> {
    if corpus.files[receiver.file].before.classes[receiver.class].kind != ContainerKind::Trait {
        return Ok(vec![receiver]);
    }
    let method = enclosing_method(corpus, file, node)
        .ok_or("Receptor de trait fora de método lexical comprovado")?;
    let mut classes = model
        .slots
        .iter()
        .filter(|(id, slots)| {
            corpus.files[id.file].before.classes[id.class].kind == ContainerKind::Class
                && slots.values().any(|s| s.body == method)
        })
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    if classes.is_empty() {
        classes.push(receiver);
    }
    Ok(classes)
}
fn accessible(
    corpus: &Corpus<'_>,
    slot: &Slot,
    context: Option<ClassId>,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    if slot.visibility == Visibility::Public {
        return Ok(true);
    }
    let Some(context) = context else {
        return Ok(false);
    };
    if context == slot.owner {
        return Ok(true);
    }
    if slot.visibility == Visibility::Private {
        return Ok(false);
    }
    Ok(
        ancestry(context, &corpus.parents, true, budget)?.contains(&slot.owner)
            || ancestry(slot.owner, &corpus.parents, true, budget)?.contains(&context),
    )
}
fn check_bindings(
    corpus: &Corpus<'_>,
    before: &Model,
    after: &Model,
    targets: &HashSet<SymbolId>,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let mut recognized = HashSet::<(usize, usize, usize)>::new();
    let mut names = HashSet::new();
    for &target in targets {
        budget.node()?;
        let symbol = &corpus.symbols[target];
        names.insert(symbol.name.to_ascii_lowercase());
        names.insert(symbol.after_name.to_ascii_lowercase());
        recognized.insert((symbol.owner.file, symbol.range.start, symbol.range.end));
    }
    for (index, adaptation) in corpus.adaptations.iter().enumerate() {
        budget.node()?;
        let Some(original) = before.selectors.get(&index) else {
            continue;
        };
        required_edit(
            corpus,
            adaptation.owner.file,
            adaptation.selector.clone(),
            targets.contains(original),
            report,
        );
        recognized.insert((
            adaptation.owner.file,
            adaptation.selector.start,
            adaptation.selector.end,
        ));
        if after.selectors.get(&index) != Some(original) {
            conflict_at(
                corpus,
                adaptation.owner.file,
                adaptation.selector.clone(),
                ConflictKind::BindingChanged,
                "Novo nome altera a origem escolhida por as/insteadof",
                report,
            );
        }
    }
    for (file_index, file) in corpus.files.iter().enumerate() {
        if file.language != Language::Php {
            continue;
        }
        let source = &file.snapshot.before;
        let mut stack = vec![file.tree.root_node()];
        while let Some(node) = stack.pop() {
            budget.node()?;
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
            if let (Some(object), Some(name)) = pair {
                let selected = names.contains(&text(name, source).to_ascii_lowercase())
                    || edited(file, &name.byte_range());
                if name.kind() != "name" {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedEdit,
                        Some(text_range(source, name.byte_range())),
                        "Chamada de método dinâmica impede provar completude do Rename de trait",
                    );
                } else if selected {
                    recognized.insert((file_index, name.start_byte(), name.end_byte()));
                    let enclosing = ancestor_class(node, file_index, &corpus.files);
                    let parent_in_trait = text(object, source) == "parent"
                        && enclosing.is_some_and(|id| {
                            corpus.files[id.file].before.classes[id.class].kind
                                == ContainerKind::Trait
                        });
                    let receiver = if parent_in_trait {
                        Ok((enclosing.unwrap(), true))
                    } else {
                        binding::receiver(
                            file_index,
                            object,
                            &corpus.files,
                            &corpus.identities,
                            &corpus.parents,
                            budget,
                            0,
                        )
                    };
                    match receiver
                        .and_then(|(id, _)| contexts(corpus, before, file_index, node, id))
                    {
                        Err(message) => report.issue(
                            &file.snapshot.path,
                            IssueKind::UnsupportedEdit,
                            Some(text_range(source, name.byte_range())),
                            message,
                        ),
                        Ok(contexts) => {
                            let mut needs = HashSet::new();
                            for mut id in contexts {
                                budget.node()?;
                                let lexical = if enclosing.is_some_and(|c| {
                                    corpus.files[c.file].before.classes[c.class].kind
                                        == ContainerKind::Trait
                                }) {
                                    Some(id)
                                } else {
                                    enclosing
                                };
                                if parent_in_trait {
                                    let Some(parent) = corpus.configs[&id].parent else {
                                        report.issue(
                                            &file.snapshot.path,
                                            IssueKind::UnsupportedEdit,
                                            Some(text_range(source, name.byte_range())),
                                            "parent em trait sem superclass em todo consumidor",
                                        );
                                        continue;
                                    };
                                    id = parent;
                                }
                                let old_name = text(name, source).to_ascii_lowercase();
                                let new_name =
                                    changed_name(file, &name.byte_range(), text(name, source))
                                        .to_ascii_lowercase();
                                let bound =
                                    before.slots.get(&id).and_then(|slots| slots.get(&old_name));
                                let later =
                                    after.slots.get(&id).and_then(|slots| slots.get(&new_name));
                                needs.insert(bound.is_some_and(|s| targets.contains(&s.symbol)));
                                if let Some(bound) = bound {
                                    if enclosing_method(corpus, file_index, node)
                                        .is_some_and(|m| corpus.symbols[m].static_member)
                                        && (text(object, source) == "$this"
                                            || (node.kind() == "scoped_call_expression"
                                                && !bound.static_member))
                                    {
                                        report.issue(
                                            &file.snapshot.path,
                                            IssueKind::UnsupportedEdit,
                                            Some(text_range(source, name.byte_range())),
                                            "Método de instância em contexto lexical estático",
                                        );
                                    }
                                    if !accessible(corpus, bound, lexical, budget)? {
                                        report.issue(
                                            &file.snapshot.path,
                                            IssueKind::UnsupportedEdit,
                                            Some(text_range(source, name.byte_range())),
                                            "Chamada fora da visibilidade efetiva de trait/alias",
                                        );
                                    }
                                    if node.kind() == "scoped_call_expression"
                                        && !bound.static_member
                                        && !matches!(
                                            text(object, source),
                                            "self" | "static" | "parent"
                                        )
                                    {
                                        report.issue(&file.snapshot.path,IssueKind::UnsupportedEdit,Some(text_range(source,name.byte_range())),"Método de instância chamado estaticamente fora de contexto lexical");
                                    }
                                }
                                if bound != later {
                                    conflict_at(
                                        corpus,
                                        file_index,
                                        name.byte_range(),
                                        ConflictKind::BindingChanged,
                                        "Proposta altera o método efetivo/origem/visibilidade de chamada de trait",
                                        report,
                                    );
                                }
                            }
                            if needs.len() > 1 {
                                report.issue(&file.snapshot.path,IssueKind::UnsupportedEdit,Some(text_range(source,name.byte_range())),"Corpo compartilhado da trait vincula este nome a símbolos diferentes entre consumidores");
                            } else {
                                required_edit(
                                    corpus,
                                    file_index,
                                    name.byte_range(),
                                    needs.contains(&true),
                                    report,
                                );
                            }
                        }
                    }
                }
            }
            if matches!(
                node.kind(),
                "function_call_expression" | "array_creation_expression"
            ) {
                let reflective = node.kind() == "array_creation_expression"
                    || node.child_by_field_name("function").is_some_and(|n| {
                        matches!(
                            text(n, source)
                                .trim_start_matches('\\')
                                .to_ascii_lowercase()
                                .as_str(),
                            "call_user_func"
                                | "call_user_func_array"
                                | "forward_static_call"
                                | "forward_static_call_array"
                                | "method_exists"
                                | "is_callable"
                        )
                    });
                if reflective {
                    let mut parts = vec![node];
                    let mut relevant = false;
                    while let Some(part) = parts.pop() {
                        budget.node()?;
                        if matches!(part.kind(), "string" | "encapsed_string")
                            && names.contains(
                                &text(part, source)
                                    .trim_matches(['\'', '"'])
                                    .to_ascii_lowercase(),
                            )
                        {
                            relevant = true;
                            break;
                        }
                        let mut c = part.walk();
                        parts.extend(part.named_children(&mut c));
                    }
                    if relevant {
                        report.issue(&file.snapshot.path,IssueKind::UnsupportedEdit,Some(text_range(source,node.byte_range())),"Callback/reflexão textual relacionado a trait exige resolução adicional");
                    }
                }
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
        for edit in &file.edits {
            budget.node()?;
            if !recognized.contains(&(file_index, edit.before.start, edit.before.end)) {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedEdit,
                    Some(text_range(source, edit.before.clone())),
                    "Edit não é declaração, alias, seletor de trait ou chamada comprovada",
                );
            }
        }
    }
    Ok(())
}
fn proof(
    corpus: &Corpus<'_>,
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let targets = corpus
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| edited(&corpus.files[s.owner.file], &s.range))
        .map(|(i, _)| i)
        .collect::<HashSet<_>>();
    let Some(&first) = targets.iter().next() else {
        return Ok(());
    };
    let origin = &corpus.symbols[first];
    let before = match corpus.compose(false, budget) {
        Ok(model) => model,
        Err(message) => {
            report.issue(
                &corpus.files[origin.owner.file].snapshot.path,
                IssueKind::UnsupportedClass,
                Some(text_range(
                    &corpus.files[origin.owner.file].snapshot.before,
                    origin.range.clone(),
                )),
                message,
            );
            return Ok(());
        }
    };
    let after = match corpus.compose(true, budget) {
        Ok(model) => model,
        Err(message) => {
            conflict_at(
                corpus,
                origin.owner.file,
                origin.range.clone(),
                ConflictKind::DuplicateMember,
                message,
                report,
            );
            return Ok(());
        }
    };
    let mut family = HashSet::from([first]);
    let mut pending = vec![first];
    while let Some(symbol) = pending.pop() {
        for &(a, b) in &before.overrides {
            budget.node()?;
            let next = if a == symbol {
                Some(b)
            } else if b == symbol {
                Some(a)
            } else {
                None
            };
            if let Some(next) = next
                && family.insert(next)
            {
                pending.push(next);
            }
        }
    }
    for &target in &targets {
        budget.node()?;
        let symbol = &corpus.symbols[target];
        if !family.contains(&target) || !symbol.name.eq_ignore_ascii_case(&origin.name) {
            conflict_at(
                corpus,
                symbol.owner.file,
                symbol.range.clone(),
                ConflictKind::BindingChanged,
                "Rename mistura métodos/aliases independentes",
                report,
            );
        }
        if let Some(member) = symbol.member {
            let file = &corpus.files[symbol.owner.file];
            let old = &file.before.members[member];
            if let Some(new) = file.after.as_ref().and_then(|f| f.members.get(member)) {
                if old.class != new.class
                    || old.contract != new.contract
                    || old.abstract_member != new.abstract_member
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::InvalidSnapshot,
                        Some(text_range(&file.snapshot.before, symbol.range.clone())),
                        "Proposta mudou contrato/estrutura além do nome",
                    );
                }
            } else {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::InvalidSnapshot,
                    None,
                    "Método posterior ausente",
                );
            }
        }
    }
    for &(a, b) in before.overrides.symmetric_difference(&after.overrides) {
        budget.node()?;
        let symbol = &corpus.symbols[a];
        conflict_at(
            corpus,
            symbol.owner.file,
            symbol.range.clone(),
            if before.overrides.contains(&(a, b)) {
                ConflictKind::OverrideChanged
            } else {
                ConflictKind::InheritedMember
            },
            "Rename altera relação entre implementação, trait ou contrato; selecione toda a família",
            report,
        );
    }
    for &(a, b) in &before.overrides {
        budget.node()?;
        if !targets.contains(&a) && !targets.contains(&b) {
            continue;
        }
        let matching = before
            .slots
            .values()
            .flat_map(|s| s.values())
            .find(|s| s.symbol == a)
            .zip(
                before
                    .slots
                    .values()
                    .flat_map(|s| s.values())
                    .find(|s| s.symbol == b),
            );
        if let Some((a_slot, b_slot)) = matching {
            if corpus.contract(a_slot).is_err()
                || corpus.contract(a_slot) != corpus.contract(b_slot)
                || a_slot.visibility != b_slot.visibility
                || a_slot.static_member != b_slot.static_member
            {
                let symbol = &corpus.symbols[a];
                report.issue(&corpus.files[symbol.owner.file].snapshot.path,IssueKind::UnsupportedClass,Some(text_range(&corpus.files[symbol.owner.file].snapshot.before,symbol.range.clone())),"Família de traits exige cabeçalhos explícitos e visibilidades efetivas equivalentes");
            } else {
                report.checked_override_pairs += 1;
            }
        } else {
            // A hidden abstract/overridden declaration may not survive in any
            // effective slot; compare its original explicit method header.
            let left = &corpus.symbols[a];
            let right = &corpus.symbols[b];
            let compatible = left.member.zip(right.member).is_some_and(|(lm, rm)| {
                corpus.files[left.owner.file].before.members[lm]
                    .contract
                    .is_ok()
                    && corpus.files[left.owner.file].before.members[lm].contract
                        == corpus.files[right.owner.file].before.members[rm].contract
            });
            if !compatible {
                report.issue(
                    &corpus.files[left.owner.file].snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(
                        &corpus.files[left.owner.file].snapshot.before,
                        left.range.clone(),
                    )),
                    "Contrato de implementação oculta de trait não comprovado",
                );
            } else {
                report.checked_override_pairs += 1;
            }
        }
    }
    check_bindings(corpus, &before, &after, &targets, report, budget)?;
    report.checked_declarations = targets.len();
    Ok(())
}
pub(crate) fn validate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    let mut report = RenameValidation::new();
    report.scope = ValidationScope::KnownReceiverBindings;
    report.note="PHP traits: composição AST antes/depois, precedência classe > trait > pai, as/insteadof, aliases independentes, visibilidade e chamadas nominais em corpus finito. Não certifica consumidores externos, reflexão, seletores dinâmicos ou as final além da gramática disponível.".into();
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
            "Corpus de traits excedido",
        );
        return Some(report);
    }
    if let Err(message) = crate::validate_rename_candidate(new_name) {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, message);
        return Some(report);
    }
    let mut budget = Budget::new(cancel, limits);
    let files = match properties::parse_files(snapshots, new_name, &mut report, &mut budget) {
        Ok(files) => files,
        Err(kind) => {
            report.issue(fallback, kind, None, "Parse de traits interrompido");
            return Some(report);
        }
    };
    let mut has_adaptation = false;
    let mut selected_alias = false;
    for file in &files {
        if file.language != Language::Php {
            continue;
        }
        let mut stack = vec![file.tree.root_node()];
        while let Some(node) = stack.pop() {
            if let Err(kind) = budget.node() {
                report.issue(fallback, kind, None, "Seleção de traits interrompida");
                return Some(report);
            }
            has_adaptation |= node.kind() == "use_list";
            if node.kind() == "use_as_clause" {
                selected_alias |= named_children(node)
                    .iter()
                    .skip(1)
                    .any(|n| n.kind() == "name" && edited(file, &n.byte_range()));
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
    }
    let selected_method = files.iter().any(|f| {
        f.before
            .members
            .iter()
            .any(|m| m.method && edited(f, &m.range))
    });
    if !has_adaptation || (!selected_method && !selected_alias) {
        return if report.issues.is_empty() && report.conflicts.is_empty() {
            None
        } else {
            Some(report)
        };
    }
    if report.issues.is_empty() && report.conflicts.is_empty() {
        match Corpus::build(files, &mut budget) {
            Ok(corpus) => {
                if let Err(kind) = proof(&corpus, &mut report, &mut budget) {
                    report.issue(fallback, kind, None, "Prova de traits interrompida");
                }
            }
            Err(message) => report.issue(fallback, IssueKind::UnsupportedClass, None, message),
        }
    }
    if let Err(kind) = budget.check() {
        report.issue(fallback, kind, None, "Orçamento de traits excedido");
    }
    report.coverage = if !report.conflicts.is_empty() {
        Coverage::Rejected
    } else if !report.issues.is_empty() || report.checked_declarations == 0 {
        Coverage::Unsupported
    } else {
        Coverage::Verified
    };
    Some(report)
}

fn call_symbols(
    corpus: &Corpus<'_>,
    model: &Model,
    file: usize,
    node: Node<'_>,
    budget: &mut Budget<'_>,
) -> Result<Option<Vec<SymbolId>>, String> {
    let (object, name) = match node.kind() {
        "member_call_expression" | "nullsafe_member_call_expression" => (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ),
        "scoped_call_expression" => (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ),
        _ => return Ok(None),
    };
    let (Some(object), Some(name)) = (object, name) else {
        return Ok(None);
    };
    if name.kind() != "name" {
        return Ok(None);
    }
    let source = &corpus.files[file].snapshot.before;
    let enclosing = ancestor_class(node, file, &corpus.files);
    let parent_in_trait = text(object, source) == "parent"
        && enclosing.is_some_and(|id| {
            corpus.files[id.file].before.classes[id.class].kind == ContainerKind::Trait
        });
    let receiver = if parent_in_trait {
        enclosing.unwrap()
    } else {
        binding::receiver(
            file,
            object,
            &corpus.files,
            &corpus.identities,
            &corpus.parents,
            budget,
            0,
        )?
        .0
    };
    let mut result = vec![];
    for mut id in contexts(corpus, model, file, node, receiver)? {
        tick(budget)?;
        if parent_in_trait {
            id = corpus.configs[&id]
                .parent
                .ok_or("parent de trait sem classe ancestral")?;
        }
        let Some(slot) = model
            .slots
            .get(&id)
            .and_then(|slots| slots.get(&text(name, source).to_ascii_lowercase()))
        else {
            return Err("Chamada de trait não resolve método em todo consumidor".into());
        };
        result.push(slot.symbol);
    }
    result.sort_unstable();
    result.dedup();
    Ok(Some(result))
}
fn contains_origin(range: &Range<usize>, start: usize, end: usize) -> bool {
    range.start <= start && end <= range.end && (start < range.end || start == end)
}
/// Exact syntactic preparation for a trait method or adaptation token. Full
/// corpus identity and every call are resolved only by propose + validation.
pub(crate) fn prepare(
    path: &Path,
    source: &str,
    origin: &crate::TextRange,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    if path.extension().and_then(|s| s.to_str()) != Some("php") {
        return Ok(None);
    }
    if source.len() > 1024 * 1024 {
        return Err("Preparação de trait excede 1 MiB".into());
    }
    let start = crate::incremental::byte_offset(source, origin.start_line, origin.start_character)
        .ok_or("Origem UTF-16 inválida")?;
    let end = crate::incremental::byte_offset(source, origin.end_line, origin.end_character)
        .ok_or("Fim UTF-16 inválido")?;
    let mut budget = Budget::new(cancel, limits);
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&Language::Php.grammar())
        .map_err(|_| "Gramática PHP")?;
    let tree = crate::incremental::parse(&mut parser, source, None, &budget)
        .ok_or("Preparação de trait cancelada/excedida")?;
    if tree.root_node().has_error() {
        return Err("Sintaxe inválida na preparação de trait".into());
    }
    let mut stack = vec![tree.root_node()];
    let mut tokens = vec![];
    let mut local_adaptations = false;
    while let Some(node) = stack.pop() {
        tick(&mut budget)?;
        local_adaptations |= node.kind() == "use_list";
        if node.kind() == "use_as_clause" || node.kind() == "use_instead_of_clause" {
            let children = named_children(node);
            if let Some(first) = children.first() {
                let selector = if first.kind() == "name" {
                    Some(*first)
                } else {
                    named_children(*first).last().copied()
                };
                if let Some(selector) = selector
                    && selector.kind() == "name"
                {
                    tokens.push(selector);
                }
            }
            if node.kind() == "use_as_clause" {
                tokens.extend(children.into_iter().skip(1).filter(|n| n.kind() == "name"));
            }
        }
        if node.kind() == "method_declaration" {
            let mut parent = node.parent();
            let mut trait_method = false;
            while let Some(p) = parent {
                if class_kind(p.kind(), Language::Php) {
                    trait_method = p.kind() == "trait_declaration";
                    break;
                }
                parent = p.parent();
            }
            if trait_method && let Some(name) = node.child_by_field_name("name") {
                tokens.push(name);
            }
        }
        let mut c = node.walk();
        stack.extend(node.named_children(&mut c));
    }
    if let Some(token) = tokens
        .iter()
        .find(|n| contains_origin(&n.byte_range(), start, end))
    {
        return Ok(Some(text(*token, source).into()));
    }
    if local_adaptations {
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            tick(&mut budget)?;
            if matches!(
                node.kind(),
                "method_declaration"
                    | "member_call_expression"
                    | "nullsafe_member_call_expression"
                    | "scoped_call_expression"
            ) && let Some(name) = node.child_by_field_name("name")
                && name.kind() == "name"
                && contains_origin(&name.byte_range(), start, end)
            {
                return Ok(Some(text(name, source).into()));
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
    }
    Ok(None)
}
/// AST identity proposal, not LSP output and never an approved transaction.
/// The ordinary validator must prove the exact result before preview/apply.
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
        return Err(
            "Planejamento de trait exige originais imutáveis,64 arquivos/1 MiB/4 MiB".into(),
        );
    }
    let mut report = RenameValidation::new();
    let mut budget = Budget::new(cancel, limits);
    let files = properties::parse_files(snapshots, new_name, &mut report, &mut budget)
        .map_err(|kind| format!("Parse de proposta de trait: {kind:?}"))?;
    if !report.issues.is_empty() || !report.conflicts.is_empty() {
        return Err("Originais inválidos no planejamento de trait".into());
    }
    let corpus = Corpus::build(files, &mut budget)?;
    let model = corpus.compose(false, &mut budget)?;
    let Some(file) = corpus
        .files
        .iter()
        .position(|f| f.snapshot.path == origin_path && f.language == Language::Php)
    else {
        return Ok(None);
    };
    let source = &corpus.files[file].snapshot.before;
    let start = crate::incremental::byte_offset(source, origin.start_line, origin.start_character)
        .ok_or("Origem UTF-16 inválida")?;
    let end = crate::incremental::byte_offset(source, origin.end_line, origin.end_character)
        .ok_or("Fim UTF-16 inválido")?;
    if end < start {
        return Err("Origem invertida".into());
    }
    let mut origins = corpus
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| s.owner.file == file && contains_origin(&s.range, start, end))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    for (i, a) in corpus.adaptations.iter().enumerate() {
        if a.owner.file == file && contains_origin(&a.selector, start, end) {
            origins.extend(model.selectors.get(&i).copied());
        }
    }
    if origins.is_empty() {
        let mut stack = vec![corpus.files[file].tree.root_node()];
        while let Some(node) = stack.pop() {
            tick(&mut budget)?;
            if let Some(name) = node.child_by_field_name("name")
                && contains_origin(&name.byte_range(), start, end)
                && let Some(bound) = call_symbols(&corpus, &model, file, node, &mut budget)?
            {
                origins.extend(bound);
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
    }
    let Some(&first) = origins.first() else {
        return Ok(None);
    };
    let mut targets = HashSet::from([first]);
    let mut pending = vec![first];
    while let Some(symbol) = pending.pop() {
        for &(a, b) in &model.overrides {
            tick(&mut budget)?;
            let next = if a == symbol {
                Some(b)
            } else if b == symbol {
                Some(a)
            } else {
                None
            };
            if let Some(next) = next
                && targets.insert(next)
            {
                pending.push(next);
            }
        }
    }
    if !targets.iter().any(|&i| {
        let s = &corpus.symbols[i];
        s.member.is_none()
            || corpus.files[s.owner.file].before.classes[s.owner.class].kind == ContainerKind::Trait
    }) {
        return Ok(None);
    }
    if origins.iter().any(|s| !targets.contains(s)) {
        return Err("Origem compartilhada resolve símbolos independentes".into());
    }
    let old_names = targets
        .iter()
        .map(|&s| corpus.symbols[s].name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if old_names.len() != 1 {
        return Err("Família de trait com grafias independentes".into());
    }
    let mut ranges: HashMap<usize, Vec<Range<usize>>> = HashMap::new();
    for &target in &targets {
        let s = &corpus.symbols[target];
        ranges
            .entry(s.owner.file)
            .or_default()
            .push(s.range.clone());
    }
    for (i, a) in corpus.adaptations.iter().enumerate() {
        if model.selectors.get(&i).is_some_and(|s| targets.contains(s)) {
            ranges
                .entry(a.owner.file)
                .or_default()
                .push(a.selector.clone());
        }
    }
    for (i, file) in corpus.files.iter().enumerate() {
        if file.language != Language::Php {
            continue;
        }
        let mut stack = vec![file.tree.root_node()];
        while let Some(node) = stack.pop() {
            tick(&mut budget)?;
            if matches!(
                node.kind(),
                "member_call_expression"
                    | "nullsafe_member_call_expression"
                    | "scoped_call_expression"
            ) && let Some(name) = node.child_by_field_name("name")
                && old_names.contains(&text(name, &file.snapshot.before).to_ascii_lowercase())
                && let Some(bound) = call_symbols(&corpus, &model, i, node, &mut budget)?
            {
                let included = bound.iter().any(|s| targets.contains(s));
                if included && bound.iter().any(|s| !targets.contains(s)) {
                    return Err(
                        "Chamada compartilhada da trait não pode ser renomeada univocamente".into(),
                    );
                }
                if included {
                    ranges.entry(i).or_default().push(name.byte_range());
                }
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
    }
    let mut result = vec![];
    let mut total = 0usize;
    for snapshot in snapshots {
        let mut proposed = snapshot.clone();
        if let Some(i) = corpus
            .files
            .iter()
            .position(|f| f.snapshot.path == snapshot.path)
        {
            let ranges = ranges.entry(i).or_default();
            ranges.sort_by_key(|r| (r.start, r.end));
            ranges.dedup();
            total += ranges.len();
            if total > 1024 {
                return Err("Mais de1024 referências de traits".into());
            }
            proposed.edits = ranges
                .iter()
                .map(|r| crate::RenameEdit {
                    range: text_range(&snapshot.before, r.clone()),
                    new_text: new_name.into(),
                })
                .collect();
            let mut after = snapshot.before.to_string();
            for range in ranges.iter().rev() {
                after.replace_range(range.clone(), new_name);
            }
            proposed.after = std::sync::Arc::from(after);
        }
        result.push(proposed);
    }
    budget
        .check()
        .map_err(|kind| format!("Proposta de trait interrompida: {kind:?}"))?;
    Ok(Some(result))
}

/// Resolve the actual symbol over a bounded immutable corpus. The disposable
/// AST proposal is used only to discover its exact original token; it is never
/// returned as provider output or applied. Preview replans the requested name.
pub(crate) fn prepare_corpus(
    snapshots: &[RenameSnapshot],
    path: &Path,
    origin: &crate::TextRange,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    let Some(proposal) = propose(
        snapshots,
        path,
        origin,
        "_codisseu_prepare_trait_probe",
        cancel,
        limits,
    )?
    else {
        return Ok(None);
    };
    let file = proposal
        .iter()
        .find(|s| s.path == path)
        .ok_or("Origem de trait ausente")?;
    let caret =
        crate::incremental::byte_offset(&file.before, origin.start_line, origin.start_character)
            .ok_or("Origem inválida")?;
    for edit in &file.edits {
        let a = crate::incremental::byte_offset(
            &file.before,
            edit.range.start_line,
            edit.range.start_character,
        )
        .ok_or("Edit inválido")?;
        let b = crate::incremental::byte_offset(
            &file.before,
            edit.range.end_line,
            edit.range.end_character,
        )
        .ok_or("Edit inválido")?;
        if a <= caret && caret <= b {
            return Ok(Some(file.before[a..b].into()));
        }
    }
    Err("Símbolo de trait não contém a origem".into())
}
