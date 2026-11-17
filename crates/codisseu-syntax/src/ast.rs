use crate::incremental::{Budget, ByteEdit, text_range};
use crate::{
    Conflict, ConflictKind, ConflictLocation, IssueKind, RenameSnapshot, RenameValidation,
};
use std::ops::Range;
use tree_sitter::{Node, Tree};
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum Language {
    Php,
    TypeScript,
}
impl Language {
    pub(crate) fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        }
    }
    pub(crate) fn key(self, name: &str) -> String {
        if self == Self::Php {
            name.to_ascii_lowercase()
        } else {
            name.to_owned()
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct Member {
    pub(crate) range: Range<usize>,
    pub(crate) name: String,
    pub(crate) class: usize,
    pub(crate) static_member: bool,
    pub(crate) method: bool,
    pub(crate) abstract_member: bool,
    pub(crate) inheritance_uncertain: bool,
    pub(crate) explicit_override: bool,
    pub(crate) contract: Result<crate::contracts::MethodContract, &'static str>,
}
#[derive(Clone, Debug)]
pub(crate) struct Class {
    pub(crate) start: usize,
    pub(crate) name: Option<String>,
    pub(crate) members: Vec<usize>,
    pub(crate) unsupported: Vec<String>,
    pub(crate) kind: ContainerKind,
    pub(crate) abstract_class: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContainerKind {
    Class,
    Interface,
    Trait,
    Unsupported,
}
impl ContainerKind {
    pub(crate) fn from_node(kind: &str) -> Self {
        match kind {
            "class_declaration" | "abstract_class_declaration" => Self::Class,
            "interface_declaration" => Self::Interface,
            "trait_declaration" => Self::Trait,
            _ => Self::Unsupported,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    pub(crate) classes: Vec<Class>,
    pub(crate) members: Vec<Member>,
    uses: Vec<Range<usize>>,
    pub errors: Vec<Range<usize>>,
    pub(crate) augmentations: Vec<String>,
}
#[derive(Default)]
pub(crate) struct Evidence {
    declarations: Vec<(Language, String)>,
    uses: Vec<(Language, String, std::path::PathBuf, crate::TextRange)>,
}
impl Evidence {
    pub(crate) fn finish(
        &self,
        report: &mut RenameValidation,
        budget: &mut Budget<'_>,
    ) -> Result<(), IssueKind> {
        let mut declarations = std::collections::HashSet::new();
        for (lang, name) in &self.declarations {
            budget.node()?;
            declarations.insert((*lang, lang.key(name)));
        }
        for (lang, name, path, range) in &self.uses {
            budget.node()?;
            if !declarations.contains(&(*lang, lang.key(name))) {
                report.issue(
                    path,
                    IssueKind::UnsupportedEdit,
                    Some(range.clone()),
                    "Uso editado não corresponde ao nome de uma declaração de método comprovada",
                );
            }
        }
        Ok(())
    }
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn has_token(node: Node<'_>, kind: &str) -> bool {
    let mut c = node.walk();
    node.children(&mut c).any(|n| n.kind() == kind)
}
fn class_facts(
    node: Node<'_>,
    source: &str,
    language: Language,
    facts: &mut Facts,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let id = facts.classes.len();
    let name = node
        .child_by_field_name("name")
        .map(|n| text(n, source).to_owned());
    let mut class = Class {
        start: node.start_byte(),
        name,
        members: vec![],
        unsupported: vec![],
        kind: ContainerKind::from_node(node.kind()),
        abstract_class: node.kind() == "abstract_class_declaration"
            || has_token(node, "abstract_modifier"),
    };
    if class.kind == ContainerKind::Unsupported || class.name.is_none() {
        class.unsupported.push(
            "Somente classe, interface ou trait nomeados; enums/anônimas não verificados".into(),
        );
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        budget.node()?;
        if matches!(child.kind(), "decorator" | "attribute_list") {
            class
                .unsupported
                .push("Decorator/atributo de classe não verificado".into());
        }
    }
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for member in body.named_children(&mut cursor) {
            budget.node()?;
            if matches!(
                member.kind(),
                "decorator" | "index_signature" | "call_signature" | "construct_signature"
            ) || (member.kind() == "method_signature" && class.kind != ContainerKind::Interface)
            {
                class.unsupported.push(
                    "Assinaturas/overloads de classe e index/call/construct signatures não verificados"
                        .into(),
                );
            }
            if member.child_by_field_name("decorator").is_some()
                || member.child_by_field_name("attributes").is_some()
            {
                class
                    .unsupported
                    .push("Atributos/decorators de membro não verificados nesta fatia".into());
            }
            let method = match language {
                Language::Php => member.kind() == "method_declaration",
                Language::TypeScript => {
                    member.kind() == "method_definition"
                        || member.kind() == "abstract_method_signature"
                        || (class.kind == ContainerKind::Interface
                            && member.kind() == "method_signature")
                }
            };
            let field = language == Language::TypeScript
                && matches!(
                    member.kind(),
                    "public_field_definition" | "property_signature"
                );
            if !method && !field {
                continue;
            }
            let Some(name_node) = member.child_by_field_name("name") else {
                class.unsupported.push("Membro sem nome simples".into());
                continue;
            };
            let simple = match language {
                Language::Php => name_node.kind() == "name",
                Language::TypeScript => name_node.kind() == "property_identifier",
            };
            if !simple {
                class
                    .unsupported
                    .push("Membro literal, privado ou computed exige validação adicional".into());
                continue;
            }
            let name = text(name_node, source).to_owned();
            if language == Language::TypeScript && name.contains('\\') {
                class.unsupported.push(
                    "Identificador de membro com escape exige normalização lexical adicional"
                        .into(),
                );
            }
            let abstract_member = member.kind() == "abstract_method_signature"
                || has_token(member, "abstract_modifier");
            if abstract_member
                && ((!class.abstract_class && class.kind != ContainerKind::Trait)
                    || member.child_by_field_name("body").is_some())
            {
                class
                    .unsupported
                    .push("Método abstrato exige classe abstrata e ausência de corpo".into());
            }
            if method
                && !abstract_member
                && member.child_by_field_name("body").is_none()
                && class.kind != ContainerKind::Interface
            {
                class
                    .unsupported
                    .push("Método abstrato/sem corpo não verificado".into());
            }
            if method
                && class.kind == ContainerKind::Interface
                && member.child_by_field_name("body").is_some()
            {
                class
                    .unsupported
                    .push("Interface não pode fornecer corpo de método".into());
            }
            if language == Language::TypeScript
                && (has_token(member, "get") || has_token(member, "set"))
            {
                class
                    .unsupported
                    .push("Accessors não verificados nesta fatia".into());
            }
            if language == Language::TypeScript && name == "constructor" {
                let mut stack = member
                    .child_by_field_name("parameters")
                    .into_iter()
                    .collect::<Vec<_>>();
                while let Some(n) = stack.pop() {
                    budget.node()?;
                    if n.kind() == "accessibility_modifier" || has_token(n, "readonly") {
                        class
                            .unsupported
                            .push("Parameter properties do constructor não verificadas".into());
                    }
                    let mut c = n.walk();
                    stack.extend(n.named_children(&mut c));
                }
            }
            class.members.push(facts.members.len());
            facts.members.push(Member {
                range: name_node.byte_range(),
                name,
                class: id,
                static_member: language == Language::TypeScript && has_token(member, "static"),
                method,
                abstract_member,
                inheritance_uncertain: {
                    let mut c = member.walk();
                    member.named_children(&mut c).any(|n| {
                        matches!(
                            n.kind(),
                            "visibility_modifier" | "accessibility_modifier" | "final_modifier"
                        ) && matches!(text(n, source), "private" | "final")
                    })
                },
                explicit_override: has_token(member, "override_modifier"),
                contract: crate::contracts::inspect(member, source, language, budget)?,
            });
        }
    }
    facts.classes.push(class);
    Ok(())
}
pub(crate) fn inspect(
    tree: &Tree,
    source: &str,
    language: Language,
    budget: &mut Budget<'_>,
) -> Result<Facts, IssueKind> {
    let mut facts = Facts::default();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        budget.node()?;
        if language == Language::TypeScript
            && node.kind() == "internal_module"
            && let Some(name) = node.child_by_field_name("name")
        {
            facts.augmentations.push(text(name, source).into());
        }

        if (node.is_error() || node.is_missing()) && facts.errors.len() < 32 {
            facts.errors.push(node.byte_range());
        }
        if node.is_named()
            && match language {
                Language::Php => matches!(
                    node.kind(),
                    "class_declaration"
                        | "anonymous_class"
                        | "trait_declaration"
                        | "interface_declaration"
                        | "enum_declaration"
                ),
                Language::TypeScript => matches!(
                    node.kind(),
                    "class_declaration"
                        | "class"
                        | "abstract_class_declaration"
                        | "interface_declaration"
                ),
            }
        {
            class_facts(node, source, language, &mut facts, budget)?;
        }
        let field = match language {
            Language::Php
                if matches!(
                    node.kind(),
                    "member_call_expression"
                        | "nullsafe_member_call_expression"
                        | "scoped_call_expression"
                ) =>
            {
                Some(("name", "name"))
            }
            Language::TypeScript if node.kind() == "member_expression" => {
                Some(("property", "property_identifier"))
            }
            _ => None,
        };
        if let Some((field, kind)) = field
            && let Some(name) = node.child_by_field_name(field)
            && name.kind() == kind
        {
            facts.uses.push(name.byte_range());
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return Ok(facts);
            }
        }
    }
}
pub(crate) fn check_changes(
    snapshot: &RenameSnapshot,
    edits: &[ByteEdit],
    trees: (&Facts, &Facts),
    language: Language,
    report: &mut RenameValidation,
    evidence: &mut Evidence,
    budget: &mut Budget<'_>,
) -> Result<(), IssueKind> {
    let (before, after) = trees;
    if before.classes.len() != after.classes.len() || before.members.len() != after.members.len() {
        report.issue(
            &snapshot.path,
            IssueKind::UnsupportedEdit,
            None,
            "A proposta alterou a estrutura dos membros/classes",
        );
        return Ok(());
    }
    for edit in edits {
        budget.node()?;
        let Some(after_name) = snapshot.after.get(edit.after.clone()) else {
            report.issue(
                &snapshot.path,
                IssueKind::InvalidSnapshot,
                None,
                "Range posterior inválido",
            );
            continue;
        };
        if let Some((index, member)) = before
            .members
            .iter()
            .enumerate()
            .find(|(_, m)| m.range == edit.before && m.method)
        {
            let Some(new_member) = after.members.get(index) else {
                continue;
            };
            let class = &before.classes[member.class];
            if (matches!(class.kind, ContainerKind::Interface | ContainerKind::Trait)
                || member.abstract_member)
                && let Err(reason) = member.contract
            {
                report.issue(
                    &snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&snapshot.before, member.range.clone())),
                    reason,
                );
            }
            if class
                .name
                .as_ref()
                .is_some_and(|name| before.augmentations.contains(name))
            {
                report.issue(
                    &snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&snapshot.before, member.range.clone())),
                    "Declaration merging de interface/namespace exige análise adicional",
                );
            }

            if new_member.range != edit.after
                || new_member.name != after_name
                || new_member.class != member.class
                || new_member.static_member != member.static_member
            {
                report.issue(
                    &snapshot.path,
                    IssueKind::UnsupportedEdit,
                    Some(text_range(&snapshot.before, edit.before.clone())),
                    "Declaração posterior não corresponde ao método original",
                );
                continue;
            }
            let mut before_names = std::collections::HashSet::new();
            for &member_index in &class.members {
                budget.node()?;
                let candidate = &before.members[member_index];
                if !before_names.insert((candidate.static_member, language.key(&candidate.name))) {
                    report.issue(
                        &snapshot.path,
                        IssueKind::UnsupportedClass,
                        Some(text_range(&snapshot.before, candidate.range.clone())),
                        "Classe já continha nomes de membros duplicados antes da proposta",
                    );
                }
            }
            if language == Language::TypeScript
                && member.static_member
                && matches!(
                    after_name,
                    "prototype" | "name" | "length" | "caller" | "arguments"
                )
            {
                report.issue(&snapshot.path,IssueKind::UnsupportedClass,Some(text_range(&snapshot.before,member.range.clone())),"Nome estático implícito de Function exige checagem de tipos/tsconfig fora desta fatia");
            }
            report.checked_declarations += 1;
            evidence.declarations.push((language, member.name.clone()));
            if (language == Language::Php && member.name.starts_with("__"))
                || (language == Language::TypeScript
                    && (member.name == "constructor" || after_name == "constructor"))
                || (language == Language::Php && after_name.starts_with("__"))
            {
                report.issue(
                    &snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&snapshot.before, member.range.clone())),
                    "Constructors e métodos mágicos não verificados nesta fatia",
                );
            }
            for reason in &class.unsupported {
                report.issue(
                    &snapshot.path,
                    IssueKind::UnsupportedClass,
                    Some(text_range(&snapshot.before, member.range.clone())),
                    reason.clone(),
                );
            }
            for &other_index in &class.members {
                budget.node()?;
                if other_index == index {
                    continue;
                }
                let other = &before.members[other_index];
                let other_after = &after.members[other_index];
                if other_after.class != member.class
                    || other_after.static_member != new_member.static_member
                {
                    continue;
                }
                if language.key(&new_member.name) == language.key(&other_after.name) {
                    if language.key(&member.name) == language.key(&other.name) {
                        report.issue(
                            &snapshot.path,
                            IssueKind::UnsupportedClass,
                            Some(text_range(&snapshot.before, other.range.clone())),
                            "Classe já continha nomes de membro duplicados antes da proposta",
                        );
                        continue;
                    }
                    if report.conflicts.len() < 256 {
                        report.conflicts.push(Conflict {
                            path: snapshot.path.clone(),
                            range: text_range(&snapshot.before, other.range.clone()),
                            message: format!(
                                "Método ‘{}’ já está definido na classe ‘{}’",
                                new_member.name,
                                class.name.as_deref().unwrap_or("(anônima)")
                            ),
                            related: Some(ConflictLocation {
                                path: snapshot.path.clone(),
                                range: text_range(&snapshot.before, member.range.clone()),
                            }),
                            kind: ConflictKind::DuplicateMember,
                            container_name: class.name.clone(),
                        });
                    }
                }
            }
        } else if before.uses.contains(&edit.before) && after.uses.contains(&edit.after) {
            evidence.uses.push((
                language,
                snapshot.before[edit.before.clone()].into(),
                snapshot.path.clone(),
                text_range(&snapshot.before, edit.before.clone()),
            ));
        } else {
            report.issue(&snapshot.path,IssueKind::UnsupportedEdit,Some(text_range(&snapshot.before,edit.before.clone())),"Edição não está ancorada ao nome AST de método declarado ou acesso de membro suportado");
        }
    }
    Ok(())
}
