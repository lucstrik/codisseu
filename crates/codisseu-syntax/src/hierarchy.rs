//! A bounded class-name graph, deliberately distinct from receiver/type resolution.
#[path = "class_inheritors.rs"]
mod class_inheritors;
pub use class_inheritors::{ClassInheritorRenameProposal, plan_class_inheritor_renames};
mod binding;
#[path = "symbols/named_arguments.rs"]
mod named_arguments;
pub(crate) use named_arguments::audit as audit_named_parameter;
pub use named_arguments::complete_php_named_argument_rename;
#[path = "traits.rs"]
mod traits;
pub(crate) use traits::{
    prepare as prepare_traits, prepare_corpus as prepare_traits_corpus, propose as propose_traits,
    validate as validate_traits,
};
#[path = "accessors.rs"]
mod accessors;
pub use accessors::{AccessorRenameProposal, plan_property_accessor_renames};
#[path = "properties.rs"]
mod properties;

pub(crate) fn validate_properties(
    snapshots: &[crate::RenameSnapshot],
    new_name: &str,
    cancel: &std::sync::atomic::AtomicBool,
    limits: crate::ValidationLimits,
) -> Option<crate::RenameValidation> {
    properties::validate(snapshots, new_name, cancel, limits)
}
use crate::ast::{ContainerKind, Facts, Language};
use crate::incremental::{Budget, ByteEdit, text_range};
use crate::{
    Conflict, ConflictKind, ConflictLocation, IssueKind, RenameSnapshot, RenameValidation,
    ValidationScope,
};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use tree_sitter::{Node, Tree};

#[derive(Default)]
pub(crate) struct Metadata {
    classes: HashMap<usize, ClassName>,
    imports: HashMap<String, Vec<Import>>,
    module: bool,
    dynamic_alias: bool,
    pub bytes: usize,
}
type PhpContext = (String, HashMap<String, Vec<String>>, Option<String>);

struct ClassName {
    range: std::ops::Range<usize>,
    identity: String,
    parents: Vec<ParentName>,
    has_relations: bool,
    exported: bool,
    problem: Option<String>,
    aliases: HashMap<String, Vec<String>>,
    namespace: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RelationKind {
    ExtendsClass,
    Interface,
    Trait,
}
struct ParentName {
    name: String,
    kind: RelationKind,
}
#[derive(Clone)]
struct Import {
    source: String,
    imported: String,
    valid: bool,
    type_only: bool,
}
pub(crate) struct File<'a> {
    pub tree: Tree,
    pub snapshot: &'a RenameSnapshot,
    pub language: Language,
    pub before: Facts,
    pub after: Option<Facts>,
    pub metadata: Metadata,
    pub edits: Vec<ByteEdit>,
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn token(node: Node<'_>, kind: &str) -> bool {
    let mut c = node.walk();
    node.children(&mut c).any(|n| n.kind() == kind)
}
fn class_kind(kind: &str, lang: Language) -> bool {
    match lang {
        Language::Php => matches!(
            kind,
            "class_declaration"
                | "anonymous_class"
                | "trait_declaration"
                | "interface_declaration"
                | "enum_declaration"
        ),
        Language::TypeScript => matches!(
            kind,
            "class_declaration" | "class" | "abstract_class_declaration" | "interface_declaration"
        ),
    }
}
fn name_path(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '\\' || !c.is_ascii())
}
fn join(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.into()
    } else {
        format!("{namespace}\\{name}")
    }
}
fn php_context(
    node: Node<'_>,
    source: &str,
    budget: &mut Budget<'_>,
) -> Result<PhpContext, IssueKind> {
    let mut root = node;
    let mut block = None;
    while let Some(parent) = root.parent() {
        budget.node()?;
        if parent.kind() == "namespace_definition" && parent.child_by_field_name("body").is_some() {
            block = Some(parent);
            break;
        }
        root = parent;
    }
    let (container, mut namespace) = if let Some(block) = block {
        (
            block.child_by_field_name("body").unwrap(),
            block
                .child_by_field_name("name")
                .map_or("", |n| text(n, source))
                .to_owned(),
        )
    } else {
        (root, String::new())
    };
    let mut begin = 0;
    let mut end = usize::MAX;
    if block.is_none() {
        let mut c = container.walk();
        for child in container.named_children(&mut c) {
            budget.node()?;
            if child.kind() == "namespace_definition" {
                if child.start_byte() < node.start_byte() {
                    namespace = child
                        .child_by_field_name("name")
                        .map_or("", |n| text(n, source))
                        .into();
                    begin = child.end_byte();
                } else {
                    end = child.start_byte();
                    break;
                }
            }
        }
    }
    let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
    let mut problem = None;
    let mut c = container.walk();
    for child in container.named_children(&mut c) {
        budget.node()?;
        if child.start_byte() < begin
            || child.start_byte() >= end
            || child.kind() != "namespace_use_declaration"
        {
            continue;
        }
        if child.child_by_field_name("type").is_some() {
            continue;
        }
        if child.child_by_field_name("body").is_some() {
            problem = Some("Imports PHP agrupados exigem resolução adicional".into());
            continue;
        }
        let mut cc = child.walk();
        for clause in child.named_children(&mut cc) {
            budget.node()?;
            if clause.kind() != "namespace_use_clause"
                || clause.child_by_field_name("type").is_some()
            {
                continue;
            }
            let mut nc = clause.walk();
            let target = clause
                .named_children(&mut nc)
                .find(|n| matches!(n.kind(), "name" | "qualified_name"));
            if let Some(target) = target {
                let target = text(target, source).trim_start_matches('\\');
                if !name_path(target) {
                    problem = Some("Import PHP não simples".into());
                    continue;
                }
                let alias = clause
                    .child_by_field_name("alias")
                    .map(|n| text(n, source))
                    .unwrap_or_else(|| target.rsplit('\\').next().unwrap());
                aliases
                    .entry(alias.to_ascii_lowercase())
                    .or_default()
                    .push(target.into());
            }
        }
    }
    Ok((namespace, aliases, problem))
}
// Every named relation is retained, including unresolved ones: dropping an edge
// could conceal a descendant or an inherited collision from the proof.
fn add_parents(
    class: &mut ClassName,
    clause: Node<'_>,
    source: &str,
    lang: Language,
    kind: RelationKind,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    class.has_relations = true;
    let mut cursor = clause.walk();
    let mut count = 0;
    for child in clause.named_children(&mut cursor) {
        budget.node()?;
        if child.kind() == "comment" {
            continue;
        }
        let simple = match lang {
            Language::Php => {
                matches!(child.kind(), "name" | "qualified_name" | "relative_name")
                    && name_path(text(child, source))
            }
            Language::TypeScript => matches!(child.kind(), "identifier" | "type_identifier"),
        };
        if !simple {
            class.problem = Some("Relação genérica/dinâmica, adaptação de trait ou nome não simples exige análise adicional".into());
            continue;
        }
        count += 1;
        class.parents.push(ParentName {
            name: text(child, source).into(),
            kind,
        });
    }
    if count == 0 || (kind == RelationKind::ExtendsClass && count != 1) {
        class.problem = Some("Relação sem ancestral simples ou múltiplas superclasses".into());
    }
    Ok(())
}
fn metadata_class(
    node: Node<'_>,
    source: &str,
    lang: Language,
    budget: &mut Budget<'_>,
) -> Result<ClassName, IssueKind> {
    let name = node
        .child_by_field_name("name")
        .map_or("", |n| text(n, source));
    let mut class = ClassName {
        range: node
            .child_by_field_name("name")
            .map_or(node.byte_range(), |n| n.byte_range()),
        identity: name.into(),
        parents: vec![],
        has_relations: false,
        exported: false,
        problem: None,
        aliases: HashMap::new(),
        namespace: String::new(),
    };
    let parent = node.parent();
    match lang {
        Language::Php => {
            let (namespace, aliases, problem) = php_context(node, source, budget)?;
            class.identity = join(&namespace, name);
            class.namespace = namespace;
            class.aliases = aliases;
            if !matches!(
                parent.map(|n| n.kind()),
                Some("program" | "compound_statement")
            ) || (parent.is_some_and(|n| n.kind() == "compound_statement")
                && !parent
                    .and_then(|n| n.parent())
                    .is_some_and(|n| n.kind() == "namespace_definition"))
            {
                class.problem = Some("Classe PHP condicional/aninhada não resolvida".into());
            }
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                budget.node()?;
                let kind = match child.kind() {
                    "base_clause" if node.kind() == "interface_declaration" => {
                        RelationKind::Interface
                    }
                    "base_clause" => RelationKind::ExtendsClass,
                    "class_interface_clause" => RelationKind::Interface,
                    _ => continue,
                };
                add_parents(&mut class, child, source, lang, kind, budget)?;
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut c = body.walk();
                for child in body.named_children(&mut c) {
                    budget.node()?;
                    if child.kind() == "use_declaration" {
                        add_parents(&mut class, child, source, lang, RelationKind::Trait, budget)?;
                    }
                }
            }
            if class.has_relations && problem.is_some() {
                class.problem = problem;
            }
        }
        Language::TypeScript => {
            let container = if parent.is_some_and(|n| n.kind() == "export_statement") {
                class.exported = parent.is_some_and(|n| !token(n, "default"));
                parent.and_then(|n| n.parent())
            } else {
                parent
            };
            if !container.is_some_and(|n| n.kind() == "program") {
                class.problem = Some("Classe TypeScript aninhada não resolvida".into());
            }
            if node.child_by_field_name("type_parameters").is_some() {
                class.problem = Some("Classe genérica exige verificação de tipos".into());
            }
            let mut c = node.walk();
            for child in node.named_children(&mut c) {
                budget.node()?;
                if child.kind() == "extends_type_clause" {
                    add_parents(
                        &mut class,
                        child,
                        source,
                        lang,
                        RelationKind::Interface,
                        budget,
                    )?;
                }
                if child.kind() == "class_heritage" {
                    let mut cc = child.walk();
                    for clause in child.named_children(&mut cc) {
                        budget.node()?;
                        let kind = match clause.kind() {
                            "extends_clause" => RelationKind::ExtendsClass,
                            "implements_clause" => RelationKind::Interface,
                            _ => {
                                class.problem = Some("Relação TypeScript não resolvida".into());
                                continue;
                            }
                        };
                        add_parents(&mut class, clause, source, lang, kind, budget)?;
                    }
                }
            }
        }
    }
    Ok(class)
}
pub(crate) fn inspect(
    tree: &Tree,
    source: &str,
    language: Language,
    budget: &mut Budget<'_>,
) -> Result<Metadata, IssueKind> {
    let mut metadata = Metadata::default();
    if language == Language::TypeScript {
        let root = tree.root_node();
        let mut c = root.walk();
        for child in root.named_children(&mut c) {
            budget.node()?;
            if matches!(child.kind(), "import_statement" | "export_statement") {
                metadata.module = true;
            }
            if child.kind() != "import_statement" {
                continue;
            }
            let source_node = child.child_by_field_name("source");
            let raw = source_node.map_or("", |n| text(n, source));
            let source_name = raw.get(1..raw.len().saturating_sub(1)).unwrap_or("");
            let valid_source = (source_name.starts_with("./") || source_name.starts_with("../"))
                && !source_name.contains(['\\', '\0'])
                && !raw.contains('`');
            let mut cursor = child.walk();
            loop {
                budget.node()?;
                let n = cursor.node();
                if n.kind() == "import_specifier"
                    && let Some(name) = n.child_by_field_name("name")
                {
                    let alias = n.child_by_field_name("alias").unwrap_or(name);
                    metadata
                        .imports
                        .entry(text(alias, source).into())
                        .or_default()
                        .push(Import {
                            source: source_name.into(),
                            imported: text(name, source).into(),
                            valid: valid_source
                                && name.kind() == "identifier"
                                && !token(child, "import_attribute"),
                            type_only: token(n, "type") || token(child, "type"),
                        });
                }
                if n.kind() == "identifier"
                    && n.parent().is_some_and(|p| {
                        matches!(
                            p.kind(),
                            "import_clause" | "namespace_import" | "import_require_clause"
                        )
                    })
                {
                    metadata
                        .imports
                        .entry(text(n, source).into())
                        .or_default()
                        .push(Import {
                            source: source_name.into(),
                            imported: String::new(),
                            valid: false,
                            type_only: false,
                        });
                }
                if cursor.goto_first_child() {
                    continue;
                }
                loop {
                    if cursor.goto_next_sibling() {
                        break;
                    }
                    if !cursor.goto_parent() {
                        break;
                    }
                }
                if cursor.node() == child {
                    break;
                }
            }
        }
    }
    metadata.bytes = metadata
        .imports
        .iter()
        .map(|(alias, imports)| {
            alias.len()
                + imports
                    .iter()
                    .map(|i| i.source.len() + i.imported.len())
                    .sum::<usize>()
        })
        .sum();
    let mut cursor = tree.walk();
    loop {
        budget.node()?;
        let n = cursor.node();
        if n.is_named() && class_kind(n.kind(), language) {
            let class = metadata_class(n, source, language, budget)?;
            metadata.bytes += class.identity.len()
                + class.namespace.len()
                + class.parents.iter().map(|p| p.name.len()).sum::<usize>()
                + class.problem.as_ref().map_or(0, String::len)
                + class
                    .aliases
                    .iter()
                    .map(|(a, v)| a.len() + v.iter().map(String::len).sum::<usize>())
                    .sum::<usize>();
            if metadata.bytes > 8 * 1024 * 1024 {
                return Err(IssueKind::Limit);
            }
            metadata.classes.insert(n.start_byte(), class);
        }
        if language == Language::Php
            && n.kind() == "function_call_expression"
            && n.child_by_field_name("function").is_some_and(|f| {
                text(f, source)
                    .trim_start_matches('\\')
                    .eq_ignore_ascii_case("class_alias")
            })
        {
            metadata.dynamic_alias = true;
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(metadata);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct ClassId {
    file: usize,
    class: usize,
}
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => result.push(component.as_os_str()),
        }
    }
    Some(result)
}
fn identity(file: &File<'_>, class: &ClassName) -> String {
    if file.language == Language::Php {
        class.identity.to_ascii_lowercase()
    } else if file.metadata.module {
        format!("{}::{}", file.snapshot.path.display(), class.identity)
    } else {
        format!("global::{}", class.identity)
    }
}
fn php_parent(class: &ClassName, parent: &str) -> Result<String, String> {
    if parent.starts_with('\\') {
        return Ok(parent.trim_start_matches('\\').to_ascii_lowercase());
    }
    if parent.to_ascii_lowercase().starts_with("namespace\\") {
        return Ok(join(&class.namespace, &parent[10..]).to_ascii_lowercase());
    }
    let (first, rest) = parent.split_once('\\').unwrap_or((parent, ""));
    if let Some(imports) = class.aliases.get(&first.to_ascii_lowercase()) {
        if imports.len() != 1 {
            return Err("Alias PHP ambíguo".into());
        }
        return Ok(if rest.is_empty() {
            imports[0].clone()
        } else {
            join(&imports[0], rest)
        }
        .to_ascii_lowercase());
    }
    Ok(join(&class.namespace, parent).to_ascii_lowercase())
}
pub(crate) fn php_type_identity(
    node: Node<'_>,
    source: &str,
    budget: &mut Budget<'_>,
) -> Result<Option<String>, IssueKind> {
    let (namespace, aliases, problem) = php_context(node, source, budget)?;
    let spelling = text(node, source);
    if problem.is_some()
        || !name_path(spelling)
        || matches!(
            spelling.to_ascii_lowercase().as_str(),
            "self" | "static" | "parent"
        )
    {
        return Ok(None);
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
    Ok(php_parent(&context, spelling).ok())
}
fn resolve_parent(
    id: ClassId,
    relation: &ParentName,
    files: &[File<'_>],
    by_identity: &HashMap<(Language, String), Vec<ClassId>>,
    budget: &mut Budget<'_>,
) -> Result<ClassId, String> {
    let file = &files[id.file];
    let class = &file.metadata.classes[&file.before.classes[id.class].start];
    let parent = &relation.name;
    if let Some(problem) = &class.problem {
        return Err(problem.clone());
    }
    let lookup = |key: String| -> Result<ClassId, String> {
        match by_identity.get(&(file.language, key)) {
            Some(ids) if ids.len() == 1 => Ok(ids[0]),
            Some(_) => Err("Identidade de classe ambígua no corpus".into()),
            None => Err("Classe ancestral ausente do corpus".into()),
        }
    };
    if file.language == Language::Php {
        return lookup(php_parent(class, parent)?);
    }
    let local_key = if file.metadata.module {
        format!("{}::{parent}", file.snapshot.path.display())
    } else {
        format!("global::{parent}")
    };
    let local = by_identity.get(&(Language::TypeScript, local_key.clone()));
    let imports = file.metadata.imports.get(parent);
    if local.is_some() && imports.is_some() {
        return Err("Extends combina classe local e import ambíguo".into());
    }
    let Some(imports) = imports else {
        return lookup(local_key);
    };
    if imports.len() != 1 || !imports[0].valid {
        return Err("Import de ancestral ambíguo/não relativo/não suportado".into());
    }
    let import = &imports[0];
    if import.type_only && relation.kind != RelationKind::Interface {
        return Err("Import type não fornece superclass de runtime".into());
    }
    let path = normalize(
        &file
            .snapshot
            .path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&import.source),
    )
    .ok_or("Caminho relativo de ancestral inválido")?;
    let candidates = match path.extension().and_then(|s| s.to_str()) {
        Some("ts") => vec![path],
        Some("js") => vec![path.with_extension("ts")],
        None => vec![path.with_extension("ts"), path.join("index.ts")],
        _ => return Err("Extensão de módulo ancestral não suportada".into()),
    };
    let mut result = vec![];
    for (i, other) in files.iter().enumerate() {
        budget
            .node()
            .map_err(|_| "Orçamento excedido na resolução de import".to_owned())?;
        if other.language != Language::TypeScript
            || !candidates
                .iter()
                .any(|p| normalize(&other.snapshot.path).as_ref() == Some(p))
        {
            continue;
        }
        for (j, c) in other.before.classes.iter().enumerate() {
            budget
                .node()
                .map_err(|_| "Orçamento excedido na resolução de export".to_owned())?;
            if let Some(meta) = other.metadata.classes.get(&c.start)
                && meta.exported
                && meta.identity == import.imported
            {
                result.push(ClassId { file: i, class: j });
            }
        }
    }
    if result.len() == 1 {
        Ok(result[0])
    } else {
        Err("Export direto do ancestral ausente/ambíguo; reexports/default não resolvidos".into())
    }
}
type Parents = HashMap<ClassId, Vec<(ClassId, RelationKind)>>;
type Ancestors = HashMap<ClassId, HashSet<ClassId>>;

fn ancestry(
    start: ClassId,
    parents: &Parents,
    class_only: bool,
    budget: &mut Budget<'_>,
) -> Result<HashSet<ClassId>, IssueKind> {
    let mut result = HashSet::new();
    let mut pending = vec![start];
    let mut visited = HashSet::new();
    while let Some(current) = pending.pop() {
        budget.node()?;
        if !visited.insert(current) {
            continue;
        }
        if let Some(edges) = parents.get(&current) {
            for &(parent, kind) in edges {
                budget.node()?;
                if class_only && kind != RelationKind::ExtendsClass {
                    continue;
                }
                result.insert(parent);
                pending.push(parent);
            }
        }
    }
    Ok(result)
}

// Members of otherwise unrelated contracts can share one implementation: the
// implementing class (or derived interface) creates a common descendant. This
// is deliberately not a relation between sibling classes sharing an ancestor.
fn related(
    a: ClassId,
    b: ClassId,
    ancestors: &Ancestors,
    budget: &mut Budget<'_>,
) -> Result<bool, IssueKind> {
    if ancestors.get(&a).is_some_and(|v| v.contains(&b))
        || ancestors.get(&b).is_some_and(|v| v.contains(&a))
    {
        return Ok(true);
    }
    for set in ancestors.values() {
        budget.node()?;
        if set.contains(&a) && set.contains(&b) {
            return Ok(true);
        }
    }
    Ok(false)
}
fn valid_edge(source: ContainerKind, target: ContainerKind, relation: RelationKind) -> bool {
    match relation {
        RelationKind::ExtendsClass => {
            source == ContainerKind::Class && target == ContainerKind::Class
        }
        RelationKind::Interface => {
            matches!(source, ContainerKind::Class | ContainerKind::Interface)
                && target == ContainerKind::Interface
        }
        RelationKind::Trait => {
            matches!(source, ContainerKind::Class | ContainerKind::Trait)
                && target == ContainerKind::Trait
        }
    }
}
fn changed(file: &File<'_>, member: usize) -> bool {
    file.edits
        .iter()
        .any(|e| e.before == file.before.members[member].range)
}
fn conflict(
    files: &[File<'_>],
    source: (ClassId, usize),
    other: (ClassId, usize),
    kind: ConflictKind,
    report: &mut RenameValidation,
) {
    if report.conflicts.len() >= 256 {
        return;
    }
    let (id, index) = source;
    let (other_id, other_index) = other;
    let file = &files[id.file];
    let other_file = &files[other_id.file];
    let member = &file.before.members[index];
    let existing = &other_file.before.members[other_index];
    let class = &other_file.before.classes[other_id.class];
    report.conflicts.push(Conflict {
        path: other_file.snapshot.path.clone(),
        range: text_range(&other_file.snapshot.before, existing.range.clone()),
        message: match kind {
            ConflictKind::OverrideChanged => format!(
                "Renomeação parcial altera a relação de override com ‘{}’ na classe ‘{}’",
                existing.name,
                class.name.as_deref().unwrap_or("?")
            ),
            _ => format!(
                "Novo nome cria colisão/override herdado com ‘{}’ na classe ‘{}’",
                existing.name,
                class.name.as_deref().unwrap_or("?")
            ),
        },
        related: Some(ConflictLocation {
            path: file.snapshot.path.clone(),
            range: text_range(&file.snapshot.before, member.range.clone()),
        }),
        kind,
        container_name: class.name.clone(),
    });
}
pub(crate) fn check(
    files: &[File<'_>],
    report: &mut RenameValidation,
    budget: &mut Budget<'_>,
    binding: bool,
) -> Result<(), IssueKind> {
    let mut active = HashSet::new();
    let mut targets = vec![];
    let mut identities: HashMap<(Language, String), Vec<ClassId>> = HashMap::new();
    let mut all = vec![];
    for (i, file) in files.iter().enumerate() {
        for (j, class) in file.before.classes.iter().enumerate() {
            budget.node()?;
            let id = ClassId { file: i, class: j };
            all.push(id);
            if let Some(meta) = file.metadata.classes.get(&class.start)
                && !meta.identity.is_empty()
            {
                identities
                    .entry((file.language, identity(file, meta)))
                    .or_default()
                    .push(id);
            }
            for &m in &class.members {
                budget.node()?;
                if file.before.members[m].method && changed(file, m) {
                    active.insert(file.language);
                    targets.push((id, m));
                }
            }
        }
    }
    for file in files {
        budget.node()?;
        if active.contains(&file.language) && file.metadata.dynamic_alias {
            report.issue(
                &file.snapshot.path,
                IssueKind::UnsupportedClass,
                None,
                "class_alias dinâmico impede resolver a hierarquia PHP do corpus",
            );
        }
    }
    let mut parents: Parents = HashMap::new();
    for &id in &all {
        budget.node()?;
        let file = &files[id.file];
        if !active.contains(&file.language) {
            continue;
        }
        let class = &file.before.classes[id.class];
        let Some(meta) = file.metadata.classes.get(&class.start) else {
            continue;
        };
        if meta.has_relations {
            report.scope = ValidationScope::KnownHierarchyMemberNamespaces;
        }
        if identities
            .get(&(file.language, identity(file, meta)))
            .is_some_and(|ids| ids.len() > 1)
        {
            report.issue(
                &file.snapshot.path,
                IssueKind::UnsupportedClass,
                Some(text_range(&file.snapshot.before, meta.range.clone())),
                "Identidade de classe/interface/trait duplicada ou ambígua no corpus",
            );
        }
        if let Some(problem) = &meta.problem
            && meta.has_relations
        {
            report.issue(
                &file.snapshot.path,
                IssueKind::UnsupportedClass,
                Some(text_range(&file.snapshot.before, meta.range.clone())),
                problem.clone(),
            );
        }
        let mut seen = HashSet::new();
        for relation in &meta.parents {
            budget.node()?;
            match resolve_parent(id, relation, files, &identities, budget) {
                Ok(parent)
                    if valid_edge(
                        class.kind,
                        files[parent.file].before.classes[parent.class].kind,
                        relation.kind,
                    ) =>
                {
                    if !seen.insert(parent) {
                        report.issue(
                            &file.snapshot.path,
                            IssueKind::UnsupportedClass,
                            Some(text_range(&file.snapshot.before, meta.range.clone())),
                            "Relação nominal duplicada no mesmo tipo",
                        );
                    }
                    parents.entry(id).or_default().push((parent, relation.kind));
                }
                Ok(_) => report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&file.snapshot.before, meta.range.clone())),
                    "Extends/implements/use aponta para espécie de declaração incompatível",
                ),
                Err(message) => report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&file.snapshot.before, meta.range.clone())),
                    message,
                ),
            }
        }
    }
    let mut ancestors = HashMap::new();
    for &id in &all {
        budget.node()?;
        let set = ancestry(id, &parents, false, budget)?;
        if set.contains(&id) {
            report.issue(
                &files[id.file].snapshot.path,
                IssueKind::UnsupportedClass,
                files[id.file]
                    .metadata
                    .classes
                    .get(&files[id.file].before.classes[id.class].start)
                    .map(|m| text_range(&files[id.file].snapshot.before, m.range.clone())),
                "Ciclo de herança/contratos/traits no corpus",
            );
        }
        ancestors.insert(id, set);
    }
    if report.scope == ValidationScope::KnownHierarchyMemberNamespaces {
        report.note="Verificação local: grafo nominal de classes/interfaces/traits resolvido no corpus de snapshots fornecido. Não certifica vinculação de referências/receptores, tipos de corpos ou descendentes fora dos snapshots, nem ausência global de conflitos. Resolução nominal do corpus não certifica autoload PHP nem resolução do compilador TypeScript/tsconfig. Herança não resolvida de qualquer classe do mesmo idioma bloqueia por possível descendência oculta; Relações completas exigem cabeçalhos explícitos equivalentes; não há inferência de variância ou tipos de corpos.".into();
    }
    let mut verified_pairs = HashSet::new();
    let mut verified_methods = HashSet::new();
    for (id, m) in targets {
        budget.node()?;
        let file = &files[id.file];
        let member = &file.before.members[m];
        let Some(after) = &file.after else {
            continue;
        };
        let Some(new_member) = after.members.get(m) else {
            continue;
        };
        if let Some(meta) = file
            .metadata
            .classes
            .get(&file.before.classes[id.class].start)
            && let Some(problem) = &meta.problem
        {
            report.issue(
                &file.snapshot.path,
                IssueKind::UnsupportedClass,
                Some(text_range(&file.snapshot.before, member.range.clone())),
                problem.clone(),
            );
        }
        if member.explicit_override {
            let mut found = false;
            for parent in ancestry(id, &parents, true, budget)? {
                let parent_file = &files[parent.file];
                for &n in &parent_file.before.classes[parent.class].members {
                    budget.node()?;
                    let other = &parent_file.before.members[n];
                    found |= other.method
                        && other.static_member == member.static_member
                        && file.language.key(&other.name) == file.language.key(&member.name);
                }
            }
            if !found {
                report.issue(
                    &file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&file.snapshot.before, member.range.clone())),
                    "Override explícito sem método ancestral de classe conhecido",
                );
            }
        }
        if file.before.classes[id.class].kind == ContainerKind::Interface || member.abstract_member
        {
            // A known concrete implementor must have a declaration, inherited
            // class method or concrete trait method. Merely spelling implements
            // does not prove that the interface contract was satisfied before.
            for &implementor in &all {
                budget.node()?;
                let container = &files[implementor.file].before.classes[implementor.class];
                let inherited = &ancestors[&implementor];
                if container.kind != ContainerKind::Class
                    || container.abstract_class
                    || !inherited.contains(&id)
                {
                    continue;
                }
                let mut implementations = Vec::new();
                for provider in std::iter::once(&implementor).chain(inherited.iter()) {
                    budget.node()?;
                    let provider_file = &files[provider.file];
                    let provider_class = &provider_file.before.classes[provider.class];
                    if !matches!(
                        provider_class.kind,
                        ContainerKind::Class | ContainerKind::Trait
                    ) {
                        continue;
                    }
                    for &n in &provider_class.members {
                        budget.node()?;
                        let candidate = &provider_file.before.members[n];
                        if candidate.method
                            && candidate.static_member == member.static_member
                            && file.language.key(&candidate.name) == file.language.key(&member.name)
                        {
                            implementations.push((*provider, candidate.abstract_member));
                        }
                    }
                }
                let mut implementation = false;
                for (provider, abstract_member) in &implementations {
                    budget.node()?;
                    if *abstract_member {
                        continue;
                    }
                    let mut shadowed = false;
                    for (closer, _) in &implementations {
                        budget.node()?;
                        if closer != provider && ancestors[closer].contains(provider) {
                            shadowed = true;
                            break;
                        }
                    }
                    if !shadowed {
                        implementation = true;
                        break;
                    }
                }
                if !implementation {
                    report.issue(
                        &files[implementor.file].snapshot.path,
                        IssueKind::UnsupportedClass,
                        None,
                        "Implementador concreto conhecido não fornece o método do contrato",
                    );
                }
            }
        }
        for &other_id in &all {
            budget.node()?;
            if id == other_id
                || file.language != files[other_id.file].language
                || !related(id, other_id, &ancestors, budget)?
            {
                continue;
            }
            let other_file = &files[other_id.file];
            let other_class = &other_file.before.classes[other_id.class];
            if let Some(meta) = other_file.metadata.classes.get(&other_class.start)
                && let Some(problem) = &meta.problem
            {
                report.issue(
                    &other_file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    None,
                    problem.clone(),
                );
            }
            for reason in &other_class.unsupported {
                report.issue(
                    &other_file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    None,
                    format!("Classe relacionada não verificada: {reason}"),
                );
            }
            if other_class
                .name
                .as_ref()
                .is_some_and(|name| other_file.before.augmentations.contains(name))
            {
                report.issue(
                    &other_file.snapshot.path,
                    IssueKind::UnsupportedClass,
                    None,
                    "Declaration merging em classe relacionada não verificado",
                );
            }
            let mut names = HashSet::new();
            let mut duplicate = false;
            for &n in &other_class.members {
                budget.node()?;
                let other = &other_file.before.members[n];
                if !names.insert((other.static_member, file.language.key(&other.name))) {
                    duplicate = true;
                    report.issue(
                        &other_file.snapshot.path,
                        IssueKind::UnsupportedClass,
                        Some(text_range(&other_file.snapshot.before, other.range.clone())),
                        "Classe relacionada já contém membros duplicados antes da proposta",
                    );
                }
            }
            if duplicate
                || !other_class.unsupported.is_empty()
                || other_file
                    .metadata
                    .classes
                    .get(&other_class.start)
                    .is_some_and(|m| m.problem.is_some())
            {
                continue;
            }
            for &n in &other_class.members {
                budget.node()?;
                let other = &other_file.before.members[n];
                let after_other = other_file
                    .after
                    .as_ref()
                    .unwrap_or(&other_file.before)
                    .members
                    .get(n)
                    .unwrap_or(other);
                if member.static_member != other.static_member {
                    continue;
                }
                let old_equal = file.language.key(&member.name) == file.language.key(&other.name);
                let new_equal =
                    file.language.key(&new_member.name) == file.language.key(&after_other.name);
                if old_equal
                    && file.before.classes[id.class].kind == ContainerKind::Trait
                    && other_class.kind == ContainerKind::Trait
                    && !ancestors[&id].contains(&other_id)
                    && !ancestors[&other_id].contains(&id)
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedClass,
                        Some(text_range(&file.snapshot.before, member.range.clone())),
                        "Composição de traits fornece implementações concorrentes do mesmo método",
                    );
                    continue;
                }
                if (old_equal || new_equal)
                    && (member.inheritance_uncertain || other.inheritance_uncertain)
                {
                    report.issue(
                        &file.snapshot.path,
                        IssueKind::UnsupportedClass,
                        Some(text_range(&file.snapshot.before, member.range.clone())),
                        "Relação com método private/final exige validação específica de herança",
                    );
                    continue;
                }
                if old_equal {
                    if !new_equal {
                        conflict(
                            files,
                            (id, m),
                            (other_id, n),
                            ConflictKind::OverrideChanged,
                            report,
                        );
                    } else {
                        let result = match (&member.contract, &other.contract) {
                            (Ok(a), Ok(b))
                                if new_member.contract.as_ref() == Ok(a)
                                    && after_other.contract.as_ref() == Ok(b)
                                    && other.method =>
                            {
                                let compatible = if file.language == Language::Php {
                                    let mut subtype =
                                        |sub: &str, sup: &str| -> Result<bool, IssueKind> {
                                            let find = |name: &str| {
                                                identities
                                                    .get(&(Language::Php, name.to_owned()))
                                                    .filter(|ids| ids.len() == 1)
                                                    .map(|ids| ids[0])
                                                    .filter(|id| {
                                                        matches!(
                                                            files[id.file].before.classes[id.class]
                                                                .kind,
                                                            ContainerKind::Class
                                                                | ContainerKind::Interface
                                                        )
                                                    })
                                            };
                                            Ok(match (find(sub), find(sup)) {
                                                (Some(sub), Some(sup)) => {
                                                    sub == sup || ancestors[&sub].contains(&sup)
                                                }
                                                _ => false,
                                            })
                                        };
                                    if ancestors[&id].contains(&other_id) {
                                        crate::contracts::compatible_override(
                                            a,
                                            b,
                                            &mut subtype,
                                            budget,
                                        )?
                                    } else if ancestors[&other_id].contains(&id) {
                                        crate::contracts::compatible_override(
                                            b,
                                            a,
                                            &mut subtype,
                                            budget,
                                        )?
                                    } else {
                                        a == b
                                            && crate::contracts::compatible_override(
                                                a,
                                                b,
                                                &mut subtype,
                                                budget,
                                            )?
                                    }
                                } else {
                                    a == b
                                };
                                if compatible {
                                    Ok(())
                                } else {
                                    Err(
                                        "Assinaturas não são equivalentes nem variância nominal PHP comprovada",
                                    )
                                }
                            }
                            (Err(reason), _) | (_, Err(reason)) => Err(*reason),
                            _ => Err(
                                "Assinatura/visibilidade/static diverge entre overrides; igualdade de contratos obrigatória",
                            ),
                        };
                        match result {
                            Ok(()) => {
                                let mut pair = [(id.file, m), (other_id.file, n)];
                                pair.sort_unstable();
                                if verified_pairs.insert(pair) {
                                    report.checked_override_pairs += 1;
                                    for (class_id, member_index) in [(id, m), (other_id, n)] {
                                        if changed(&files[class_id.file], member_index)
                                            && verified_methods
                                                .insert((class_id.file, member_index))
                                        {
                                            let f = &files[class_id.file];
                                            let method = &f.before.members[member_index];
                                            report.related_methods.push(crate::RelatedMethod {
                                                path: f.snapshot.path.clone(),
                                                range: text_range(
                                                    &f.snapshot.before,
                                                    method.range.clone(),
                                                ),
                                                container_name: f.before.classes[class_id.class]
                                                    .name
                                                    .clone()
                                                    .unwrap_or_default(),
                                                name: method.name.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                            Err(reason) => report.issue(
                                &file.snapshot.path,
                                IssueKind::UnsupportedClass,
                                Some(text_range(&file.snapshot.before, member.range.clone())),
                                reason,
                            ),
                        }
                    }
                } else if new_equal {
                    conflict(
                        files,
                        (id, m),
                        (other_id, n),
                        ConflictKind::InheritedMember,
                        report,
                    );
                }
            }
        }
    }
    report.related_methods.sort_by(|a, b| {
        (&a.path, a.range.start_line, a.range.start_character).cmp(&(
            &b.path,
            b.range.start_line,
            b.range.start_character,
        ))
    });
    // A case-only rename may leave an equivalent ancestor unchanged: never list
    // a declaration that isn't in the provider proposal as a selectable edit.
    if report.related_methods.len() < 2 {
        report.related_methods.clear();
    }
    if binding {
        binding::check(files, &parents, &identities, report, budget)?;
    }
    Ok(())
}
