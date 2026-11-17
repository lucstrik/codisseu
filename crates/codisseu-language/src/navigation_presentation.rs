//! Optional PHP labels for the chooser. Never used to resolve or authorize a target.
use crate::{
    DefinitionLabel, DefinitionPresentation, DefinitionResult,
    php_project::{Budget, Names, text as node_text},
};
use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

struct Label {
    name: String,
    owner: Option<String>,
    namespace: String,
    kind: char,
    public: bool,
}
fn label(budget: &Budget<'_>, text: &str, line: u32, character: u32) -> Option<Label> {
    let byte = crate::utf16_byte_offset(text, line, character)?;
    let tree = budget.tree(text)?;
    let mut node = tree.root_node().descendant_for_byte_range(byte, byte)?;
    let namespace = Names::at(&tree, text, byte)?.namespace;
    let mut name = None;
    let mut owner = None;
    let mut kind = '?';
    let mut public = false;
    for _ in 0..64 {
        budget.check()?;
        match node.kind() {
            "member_call_expression" | "scoped_call_expression" | "function_call_expression" => {
                let called = node
                    .child_by_field_name("name")
                    .or_else(|| node.child_by_field_name("function"))?;
                let name = node_text(called, text).to_owned();
                return (name.len() <= 256).then_some(Label {
                    name,
                    owner: None,
                    namespace,
                    kind: '?',
                    public: false,
                });
            }
            "method_declaration" | "function_definition" => {
                if name.is_none() {
                    public = !crate::php_project::children(node).iter().any(|n| {
                        n.kind() == "visibility_modifier"
                            && matches!(node_text(*n, text), "private" | "protected")
                    });
                    name = Some(node_text(node.child_by_field_name("name")?, text).to_owned());
                    kind = if node.kind() == "method_declaration" {
                        'm'
                    } else {
                        'f'
                    };
                }
            }
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                let class = node_text(node.child_by_field_name("name")?, text).to_owned();
                if name.is_some() {
                    owner = Some(class);
                } else {
                    name = Some(class);
                    kind = if node.kind() == "interface_declaration" {
                        'i'
                    } else {
                        'c'
                    };
                }
                break;
            }
            _ => {}
        }
        let Some(parent) = node.parent() else {
            break;
        };
        node = parent;
    }
    let name = name.or_else(|| {
        let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
        matches!(node.kind(), "name" | "qualified_name" | "variable_name")
            .then(|| node_text(node, text).to_owned())
    })?;
    if name.len() > 256 || owner.as_ref().is_some_and(|n| n.len() > 256) || namespace.len() > 512 {
        return None;
    }
    Some(Label {
        name,
        owner,
        namespace,
        kind,
        public,
    })
}

pub(crate) fn describe(
    root: &Path,
    source: &str,
    result: &DefinitionResult,
    buffers: &[(std::path::PathBuf, &str)],
    stop: &AtomicBool,
) -> Option<DefinitionPresentation> {
    if result.targets.len() < 2
        || crate::Language::for_path(&result.path) != Some(crate::Language::Php)
    {
        return None;
    }
    let mut budget = Budget {
        buffers,
        root,
        cancel: stop,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    let source = label(&budget, source, result.line, result.character);
    let source_symbol = source
        .map(|s| {
            if let Some(owner) = s.owner {
                format!("{owner} \\{}.{}", s.namespace, s.name)
            } else {
                s.name
            }
        })
        .unwrap_or_default();
    let targets = result
        .targets
        .iter()
        .map(|target| {
            let (_, text) = budget.read(&target.path)?;
            let label = label(&budget, &text, target.start_line, target.start_character)?;
            Some(DefinitionLabel {
                public: label.public,
                name: label.owner.unwrap_or(label.name),
                namespace: if label.namespace.is_empty() {
                    String::new()
                } else {
                    format!("\\{}", label.namespace)
                },
                kind: label.kind,
            })
        })
        .collect();
    Some(DefinitionPresentation {
        source_symbol,
        targets,
    })
}

pub(crate) fn sort(result: &mut DefinitionResult) {
    if result.targets.len() < 2 {
        return;
    }
    let mut indices = (0..result.targets.len()).collect::<Vec<_>>();
    indices.sort_by_cached_key(|&i| {
        let target = &result.targets[i];
        let label = result
            .presentation
            .as_ref()
            .and_then(|p| p.targets.get(i))
            .and_then(Option::as_ref);
        (
            label.map(|d| d.name.to_lowercase()).unwrap_or_else(|| {
                target
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
            }),
            target.path.clone(),
            target.start_line,
            target.start_character,
        )
    });
    result.targets = indices.iter().map(|&i| result.targets[i].clone()).collect();
    if let Some(p) = &mut result.presentation {
        p.targets = indices
            .iter()
            .map(|&i| p.targets.get(i).cloned().flatten())
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_use_the_ast_and_utf16_without_changing_provider_targets() {
        let stop = AtomicBool::new(false);
        let dir = tempfile::tempdir().unwrap();
        let budget = Budget {
            buffers: &[],
            root: dir.path(),
            cancel: &stop,
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        let source =
            "<?php namespace A { interface Contract { public function deliver(): string; } }";
        let at = source.find("public").unwrap() as u32;
        let found = label(&budget, source, 0, at).unwrap();
        assert_eq!(found.name, "deliver");
        assert_eq!(found.owner.as_deref(), Some("Contract"));
        assert_eq!(found.namespace, "A");
        assert_eq!(found.kind, 'm');
        let source = "<?php namespace B; echo '😀'; class Service {}";
        let at = source[..source.find("class").unwrap()]
            .encode_utf16()
            .count() as u32;
        let found = label(&budget, source, 0, at).unwrap();
        assert_eq!(found.name, "Service");
        assert_eq!(found.namespace, "B");
        assert!(label(&budget, source, 0, u32::MAX).is_none());
    }
}
