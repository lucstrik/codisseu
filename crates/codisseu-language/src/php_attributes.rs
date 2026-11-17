//! Completion of declared PHP attributes missing from the semantic provider.
use crate::{
    CompletionEdit, CompletionItem,
    php_project::{Budget, ComposerProject, Names, children, text},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn completions(
    root: &Path,
    path: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
) -> Option<(Vec<CompletionItem>, bool)> {
    if source.len() > 1024 * 1024 {
        return None;
    }
    let byte = crate::utf16_byte_offset(source, line, character)?;
    let attribute_start = source[..byte].rfind("#[")?;
    if byte - attribute_start > 8192 {
        return None;
    }
    let mut budget = Budget {
        root,
        buffers,
        cancel,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    let parsed = format!(
        "{}] class __CodisseuAttributeProbe {{}}",
        source.get(..byte)?
    );
    let tree = budget.tree(&parsed)?;
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(byte.saturating_sub(1), byte.saturating_sub(1))?;
    while node.kind() != "attribute" {
        node = node.parent()?;
    }
    let name = children(node)
        .into_iter()
        .find(|n| matches!(n.kind(), "name" | "qualified_name"))?;
    if name.end_byte() != byte {
        return None;
    }
    let start = name.start_byte();
    let prefix = &source[start..byte];
    let scope_tree = budget.tree(source)?;
    let names = Names::at(&scope_tree, source, byte.saturating_sub(1))?;
    let project = ComposerProject::discover(&mut budget, path)?;
    let mut files = Vec::new();
    let mut visited = 0;
    let (sources, mut truncated) = project.sources(&mut budget);
    for dir in sources {
        files_in(&budget, &dir, 0, &mut visited, &mut truncated, &mut files);
    }
    files.sort();
    files.dedup();
    let leaf = prefix.rsplit('\\').next()?.to_lowercase();
    files.sort_by_key(|p| {
        !p.file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.to_lowercase().starts_with(&leaf))
    });
    let suffix: String = source[byte..]
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '\\'))
        .collect();
    let end_character = character + suffix.encode_utf16().count() as u32;
    let start_character = character.checked_sub(prefix.encode_utf16().count() as u32)?;
    let mut result = Vec::new();
    for path in files {
        if budget.check().is_none() {
            truncated = true;
            break;
        }
        // PSR-4/PSR-0 candidates are still checked against their actual AST;
        // classmap/files may declare a class unrelated to the file's basename.
        let Some((path, source)) = budget.read(&path) else {
            truncated = true;
            break;
        };
        let Some(tree) = budget.tree(&source) else {
            truncated = true;
            break;
        };
        let mut classes = children(tree.root_node());
        for node in children(tree.root_node()) {
            if node.kind() == "namespace_definition"
                && let Some(body) = node.child_by_field_name("body")
            {
                classes.extend(children(body));
            }
        }
        for class in classes
            .into_iter()
            .filter(|n| n.kind() == "class_declaration" && !n.has_error())
        {
            let Some(declarations) = Names::at(&tree, &source, class.start_byte()) else {
                continue;
            };
            let Some(name_node) = class.child_by_field_name("name") else {
                continue;
            };
            let short = text(name_node, &source);
            let Some(fqcn) = declarations.resolve(short) else {
                continue;
            };
            let attribute = class
                .child_by_field_name("attributes")
                .is_some_and(|attributes| {
                    let mut stack = vec![attributes];
                    while let Some(n) = stack.pop() {
                        if n.kind() == "attribute"
                            && children(n)
                                .first()
                                .and_then(|n| declarations.resolve(text(*n, &source)))
                                .as_deref()
                                == Some("Attribute")
                        {
                            return true;
                        }
                        stack.extend(children(n));
                    }
                    false
                });
            if !attribute {
                continue;
            }
            let alias = names
                .imports
                .iter()
                .find(|(_, target)| target.eq_ignore_ascii_case(&fqcn))
                .map(|(a, _)| a.as_str());
            let (label, new_text) = if prefix.contains('\\') {
                let Some(resolved) = names.resolve(prefix) else {
                    continue;
                };
                if !fqcn.to_lowercase().starts_with(&resolved.to_lowercase()) {
                    continue;
                }
                let suffix = &fqcn[resolved.len()..];
                (format!("{prefix}{suffix}"), format!("{prefix}{suffix}"))
            } else if short.to_lowercase().starts_with(&prefix.to_lowercase()) {
                (short.into(), format!("\\{fqcn}"))
            } else if let Some(alias) =
                alias.filter(|a| a.starts_with(&prefix.to_ascii_lowercase()))
            {
                (alias.into(), format!("\\{fqcn}"))
            } else {
                continue;
            };
            if result
                .iter()
                .any(|i: &CompletionItem| i.new_text == new_text)
            {
                continue;
            }
            if result.len() >= 128 {
                truncated = true;
                break;
            }
            result.push(CompletionItem {
                label: label.clone(),
                detail: Some(format!(
                    "PHP attribute · {fqcn} · {}",
                    path.strip_prefix(root).unwrap_or(&path).display()
                )),
                kind: Some(7),
                filter_text: Some(label),
                sort_text: None,
                new_text: new_text.clone(),
                text_edit: Some(CompletionEdit {
                    start_line: line,
                    start_character,
                    end_line: line,
                    end_character,
                    new_text,
                }),
                insert_edit: None,
                additional_edits: Vec::new(),
                unsupported_reason: None,
            });
        }
    }
    result.sort_by(|a, b| a.label.cmp(&b.label));
    Some((result, truncated))
}
fn files_in(
    budget: &Budget<'_>,
    path: &Path,
    depth: usize,
    visited: &mut usize,
    truncated: &mut bool,
    result: &mut Vec<PathBuf>,
) {
    if depth > 16 || *visited >= 4096 || budget.check().is_none() {
        *truncated = true;
        return;
    }
    let Ok(meta) = path.symlink_metadata() else {
        return;
    };
    if meta.file_type().is_symlink() {
        return;
    }
    let Ok(canonical) = path.canonicalize() else {
        return;
    };
    if !canonical.starts_with(budget.root) {
        return;
    }
    if meta.is_file() {
        if path.extension().is_some_and(|e| e == "php") {
            result.push(canonical);
        }
        return;
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries {
        *visited += 1;
        if *visited > 4096 || budget.check().is_none() {
            *truncated = true;
            break;
        }
        if let Ok(entry) = entry {
            files_in(budget, &entry.path(), depth + 1, visited, truncated, result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn put(root: &Path, name: &str, source: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, source).unwrap();
        path
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            "composer.json",
            r#"{"autoload":{"psr-4":{"Fixture\\":"src/"},"classmap":["legacy/"],"files":["helpers.php"]},"autoload-dev":{"psr-0":{"":"tests/"}}}"#,
        );
        put(
            dir.path(),
            "src/TraceSpan.php",
            "<?php namespace Fixture; #[\\Attribute] class TraceSpan {}",
        );
        put(
            dir.path(),
            "src/NotAttribute.php",
            "<?php namespace Fixture; class NotAttribute {}",
        );
        put(
            dir.path(),
            "legacy/DifferentFilename.php",
            "<?php namespace Legacy; use Attribute as Meta; #[Meta] class LegacyAttribute {}",
        );
        put(
            dir.path(),
            "helpers.php",
            "<?php #[\\Attribute] class HelperAttribute {}",
        );
        put(
            dir.path(),
            "tests/TestAttribute.php",
            "<?php namespace Dev; #[\\Attribute] class TestAttribute {}",
        );
        dir
    }
    fn run(
        root: &Path,
        marked: &str,
        buffers: &[(PathBuf, &str)],
    ) -> Option<(Vec<CompletionItem>, bool)> {
        let byte = marked.find('|').unwrap();
        let source = marked.replace('|', "");
        let before = &source[..byte];
        completions(
            root,
            &root.join("main.php"),
            &source,
            before.bytes().filter(|b| *b == b'\n').count() as u32,
            before.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
            &AtomicBool::new(false),
            buffers,
        )
    }
    #[test]
    fn attributes_are_actual_php_classes_across_composer_autoload_kinds() {
        let dir = fixture();
        for (prefix, expected) in [
            ("Trac", "\\Fixture\\TraceSpan"),
            ("Legacy", "\\Legacy\\LegacyAttribute"),
            ("Helper", "\\HelperAttribute"),
            ("Test", "\\Dev\\TestAttribute"),
        ] {
            let (items, truncated) = run(
                dir.path(),
                &format!("<?php #[{prefix}|] class Example {{}}"),
                &[],
            )
            .unwrap();
            assert!(!truncated);
            assert!(items.iter().any(|i| i.new_text == expected), "{prefix}");
        }
        assert!(
            run(dir.path(), "<?php #[Not|] class Example {}", &[])
                .unwrap()
                .0
                .is_empty()
        );
    }
    #[test]
    fn attribute_declarations_and_uses_resolve_each_namespace_scope() {
        let dir = fixture();
        put(
            dir.path(),
            "legacy/Many.php",
            "<?php namespace First { use Attribute as Meta; #[Meta] class BracedTrace {} } namespace Second { use Attribute as Flag; #[Flag] class OtherTrace {} } namespace Third { class BracedTrace {} }",
        );
        for (source, expected) in [
            (
                "<?php namespace Client { use First\\{BracedTrace as Mark}; #[Mar|] class C {} }",
                "\\First\\BracedTrace",
            ),
            (
                "<?php namespace A { use Second\\OtherTrace as Mark; class C {} } namespace B { use First\\BracedTrace as Mark; #[Mar|] class D {} }",
                "\\First\\BracedTrace",
            ),
            (
                "<?php namespace A; use First\\BracedTrace as Mark; class C {} namespace B; use Second\\OtherTrace as Mark; #[Mar|] class D {}",
                "\\Second\\OtherTrace",
            ),
        ] {
            let (items, truncated) = run(dir.path(), source, &[]).unwrap();
            assert!(!truncated);
            assert_eq!(items.len(), 1, "{source}: {items:?}");
            assert_eq!(items[0].new_text, expected);
        }
    }
    #[test]
    fn qualified_attributes_aliases_unicode_crlf_and_suffix_ranges_are_preserved() {
        let dir = fixture();
        for source in [
            "<?php #[\\Fixture\\Trac|OLD] class E {}",
            "<?php use Fixture as F; #[F\\Trac|OLD] class E {}",
            "<?php\r\n// 🌧\r\n#[Trac|OLD] class E {}",
        ] {
            let (items, _) = run(dir.path(), source, &[]).unwrap();
            assert_eq!(items.len(), 1, "{source}");
            let source = source.replace('|', "");
            let item = &items[0];
            let edit = item.text_edit.as_ref().unwrap();
            let a =
                crate::utf16_byte_offset(&source, edit.start_line, edit.start_character).unwrap();
            let b = crate::utf16_byte_offset(&source, edit.end_line, edit.end_character).unwrap();
            assert!(source[a..b].ends_with("TracOLD"));
            let result = format!("{}{}{}", &source[..a], edit.new_text, &source[b..]);
            assert!(result.ends_with("] class E {}"));
            assert!(!result.contains("OLD"));
        }
    }
    #[test]
    fn comments_strings_unrelated_new_and_invalid_positions_never_get_attribute_candidates() {
        let dir = fixture();
        for code in [
            "<?php // #[Trac|",
            "<?php $s = '#[Trac|';",
            "<?php new Trac|;",
            "<?php #[Marker('Trac|')] class E {}",
        ] {
            assert!(run(dir.path(), code, &[]).is_none(), "{code}");
        }
    }
    #[test]
    fn dirty_declarations_and_composer_are_used_without_writes() {
        let dir = fixture();
        let root = dir.path();
        let path = root.join("src/TraceSpan.php").canonicalize().unwrap();
        let dirty = "<?php namespace Fixture; class TraceSpan {}";
        assert!(
            run(root, "<?php #[Trac|] class E {}", &[(path.clone(), dirty)])
                .unwrap()
                .0
                .is_empty()
        );
        assert!(fs::read_to_string(&path).unwrap().contains("Attribute"));
        let manifest = root.join("composer.json").canonicalize().unwrap();
        assert!(
            run(root, "<?php #[Trac|] class E {}", &[(manifest, "{}")])
                .unwrap()
                .0
                .is_empty()
        );
    }
    #[test]
    fn installed_package_metadata_is_data_and_can_supply_attributes() {
        let dir = fixture();
        let root = dir.path();
        put(
            root,
            "vendor/composer/installed.json",
            r#"{"packages":[{"name":"test/attributes","install-path":"../test/attributes","autoload":{"psr-4":{"Package\\":"src/"}}}]}"#,
        );
        put(
            root,
            "vendor/test/attributes/src/PackageAttribute.php",
            "<?php namespace Package; #[\\Attribute] class PackageAttribute {}",
        );
        assert_eq!(
            run(root, "<?php #[Package|] class E {}", &[]).unwrap().0[0].new_text,
            "\\Package\\PackageAttribute"
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_escapes_and_cancelled_queries_are_rejected() {
        let dir = fixture();
        let other = tempfile::tempdir().unwrap();
        let external = put(
            other.path(),
            "Leak.php",
            "<?php #[\\Attribute] class Leak {}",
        );
        std::os::unix::fs::symlink(&external, dir.path().join("src/Leak.php")).unwrap();
        assert!(
            run(dir.path(), "<?php #[Leak|] class E {}", &[])
                .unwrap()
                .0
                .is_empty()
        );
        assert!(
            completions(
                dir.path(),
                &dir.path().join("main.php"),
                "<?php #[Trac] class E {}",
                0,
                12,
                &AtomicBool::new(true),
                &[]
            )
            .is_none()
        );
    }
}
