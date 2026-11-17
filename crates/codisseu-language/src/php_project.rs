//! Shared, bounded static PHP/Composer source access. No autoloader execution.
use serde_json::Value;
use std::{
    collections::HashMap,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser, Tree};
pub(crate) struct Budget<'a> {
    pub(crate) buffers: &'a [(PathBuf, &'a str)],
    pub(crate) root: &'a Path,
    pub(crate) cancel: &'a AtomicBool,
    pub(crate) start: Instant,
    pub(crate) bytes: usize,
    pub(crate) files: usize,
}
impl Budget<'_> {
    pub(crate) fn check(&self) -> Option<()> {
        (!self.cancel.load(Ordering::Acquire) && self.start.elapsed() < Duration::from_millis(250))
            .then_some(())
    }
    pub(crate) fn read(&mut self, path: &Path) -> Option<(PathBuf, String)> {
        self.check()?;
        if self.files >= 64 || self.bytes > 4 * 1024 * 1024 {
            return None;
        }
        let path = path.canonicalize().ok()?;
        if !path.starts_with(self.root) {
            return None;
        }
        if let Some((_, text)) = self.buffers.iter().find(|(p, _)| p == &path) {
            self.bytes += text.len();
            self.files += 1;
            return (self.bytes <= 4 * 1024 * 1024).then(|| (path, (*text).to_owned()));
        }
        // Opening a FIFO can block before File::metadata is available. Reject
        // special files before open as well as checking the opened handle.
        if !std::fs::metadata(&path).ok()?.is_file() {
            return None;
        }
        let mut bytes = Vec::new();
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Preserve nonblocking special-file rejection if a path is changed
            // between the metadata check and open. Flags have no effect on
            // reads from a regular file; no unsafe call is required.
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let file = options.open(&path).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 1024 * 1024 {
            return None;
        }
        if self.bytes + bytes.len() > 4 * 1024 * 1024 {
            return None;
        }
        self.bytes += bytes.len();
        self.files += 1;
        Some((path, String::from_utf8(bytes).ok()?))
    }
    pub(crate) fn tree(&self, text: &str) -> Option<Tree> {
        self.check()?;
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .ok()?;
        let mut callback = |_: &tree_sitter::ParseState| {
            if self.check().is_some() {
                ControlFlow::Continue(())
            } else {
                ControlFlow::Break(())
            }
        };
        parser.parse_with_options(
            &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
            None,
            Some(ParseOptions::new().progress_callback(&mut callback)),
        )
    }
}
pub(crate) fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
pub(crate) fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}
pub(crate) fn name_path(name: &str) -> bool {
    !name.is_empty()
        && name
            .trim_start_matches('\\')
            .split('\\')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
}
#[derive(Default)]
pub(crate) struct Names {
    pub(crate) namespace: String,
    pub(crate) imports: HashMap<String, String>,
}
impl Names {
    pub(crate) fn parse(tree: &Tree, source: &str) -> Option<Self> {
        let namespaces = children(tree.root_node())
            .into_iter()
            .filter(|n| n.kind() == "namespace_definition")
            .collect::<Vec<_>>();
        if namespaces.len() > 1 {
            return None;
        }
        let byte = namespaces
            .first()
            .and_then(|n| n.child_by_field_name("body"))
            .map_or(source.len().saturating_sub(1), |n| n.start_byte() + 1);
        Self::at(tree, source, byte)
    }
    /// Imports are lexical: braced and consecutive namespaces never share aliases.
    pub(crate) fn at(tree: &Tree, source: &str, byte: usize) -> Option<Self> {
        let nodes = children(tree.root_node());
        let mut names = Self::default();
        let mut scope = nodes.clone();
        for (index, node) in nodes.iter().enumerate() {
            if node.kind() != "namespace_definition" {
                continue;
            }
            let body = node.child_by_field_name("body");
            let end = body.map_or_else(
                || {
                    nodes
                        .iter()
                        .skip(index + 1)
                        .find(|n| n.kind() == "namespace_definition")
                        .map_or(source.len(), |n| n.start_byte())
                },
                |n| n.end_byte(),
            );
            if byte >= node.start_byte() && byte < end {
                names.namespace = node
                    .child_by_field_name("name")
                    .map(|n| text(n, source))
                    .unwrap_or("")
                    .into();
                scope = body.map(children).unwrap_or_else(|| {
                    nodes
                        .iter()
                        .skip(index + 1)
                        .take_while(|n| n.kind() != "namespace_definition")
                        .copied()
                        .collect()
                });
                break;
            }
        }
        for node in scope {
            if node.kind() != "namespace_use_declaration"
                || node.child_by_field_name("type").is_some()
            {
                continue;
            }
            let group = node.child_by_field_name("body");
            let base = if group.is_some() {
                let prefix = children(node)
                    .into_iter()
                    .find(|n| n.kind() == "namespace_name")?;
                format!("{}\\", text(prefix, source).trim_matches('\\'))
            } else {
                String::new()
            };
            for clause in children(group.unwrap_or(node)) {
                if clause.kind() != "namespace_use_clause"
                    || clause.child_by_field_name("type").is_some()
                {
                    continue;
                }
                let target_node = children(clause)
                    .into_iter()
                    .find(|n| matches!(n.kind(), "name" | "qualified_name"))?;
                let target = format!(
                    "{base}{}",
                    text(target_node, source).trim_start_matches('\\')
                );
                if !name_path(&target) {
                    return None;
                }
                let alias = clause
                    .child_by_field_name("alias")
                    .map(|n| text(n, source))
                    .unwrap_or_else(|| target.rsplit('\\').next().unwrap());
                if names
                    .imports
                    .insert(alias.to_ascii_lowercase(), target)
                    .is_some()
                {
                    return None;
                }
            }
        }
        Some(names)
    }
    pub(crate) fn resolve(&self, name: &str) -> Option<String> {
        if !name_path(name) || matches!(name, "self" | "static" | "parent") {
            return None;
        }
        if name.starts_with('\\') {
            return Some(name.trim_start_matches('\\').into());
        }
        let (first, tail) = name.split_once('\\').unwrap_or((name, ""));
        if let Some(import) = self.imports.get(&first.to_ascii_lowercase()) {
            return Some(if tail.is_empty() {
                import.clone()
            } else {
                format!("{import}\\{tail}")
            });
        }
        Some(if self.namespace.is_empty() {
            name.into()
        } else {
            format!("{}\\{name}", self.namespace)
        })
    }
}
pub(crate) struct Map {
    pub(crate) prefix: String,
    pub(crate) dir: PathBuf,
}
pub(crate) fn add_maps(maps: &mut Vec<Map>, base: &Path, autoload: &Value) -> Option<()> {
    if let Some(psr) = autoload.get("psr-4").and_then(Value::as_object) {
        for (prefix, value) in psr {
            if !prefix.ends_with('\\') || !name_path(prefix.trim_end_matches('\\')) {
                continue;
            }
            let dirs: Vec<&str> = if let Some(s) = value.as_str() {
                vec![s]
            } else {
                value
                    .as_array()?
                    .iter()
                    .map(Value::as_str)
                    .collect::<Option<_>>()?
            };
            for dir in dirs {
                if maps.len() >= 256 {
                    return None;
                }
                maps.push(Map {
                    prefix: prefix.clone(),
                    dir: base.join(dir),
                });
            }
        }
    }
    Some(())
}
pub(crate) fn locate(
    budget: &mut Budget<'_>,
    maps: &[Map],
    fqcn: &str,
) -> Option<(PathBuf, String)> {
    let mut found = None;
    for map in maps {
        if let Some(tail) = fqcn.strip_prefix(&map.prefix)
            && let Some(file) =
                budget.read(&map.dir.join(format!("{}.php", tail.replace('\\', "/"))))
        {
            if found.as_ref().is_some_and(|(path, _)| path != &file.0) {
                return None;
            }
            found = Some(file);
        }
    }
    found
}

pub(crate) struct ComposerProject {
    pub root: PathBuf,
    pub manifest: Value,
}
impl ComposerProject {
    pub(crate) fn discover(budget: &mut Budget<'_>, path: &Path) -> Option<Self> {
        let mut dir = path.parent()?;
        for _ in 0..32 {
            budget.check()?;
            if !dir.starts_with(budget.root) {
                return None;
            }
            let manifest = dir.join("composer.json");
            if manifest.is_file() {
                let (_, source) = budget.read(&manifest)?;
                let manifest = serde_json::from_str(&source).ok()?;
                return Some(Self {
                    root: dir.into(),
                    manifest,
                });
            }
            if dir == budget.root {
                break;
            }
            dir = dir.parent()?;
        }
        None
    }
    pub(crate) fn is_laravel(&self) -> bool {
        self.manifest
            .get("require")
            .and_then(|r| r.get("laravel/framework"))
            .is_some()
    }
    pub(crate) fn sources(&self, budget: &mut Budget<'_>) -> (Vec<PathBuf>, bool) {
        let mut sources = Vec::new();
        let mut truncated = false;
        for autoload in [
            self.manifest.get("autoload"),
            self.manifest.get("autoload-dev"),
        ]
        .into_iter()
        .flatten()
        {
            source_paths(&mut sources, &self.root, autoload, &mut truncated);
        }
        let installed = self.root.join("vendor/composer/installed.json");
        if installed.is_file() {
            if let Some((_, source)) = budget.read(&installed) {
                if let Ok(installed) = serde_json::from_str::<Value>(&source) {
                    if let Some(packages) =
                        installed.get("packages").unwrap_or(&installed).as_array()
                    {
                        truncated |= packages.len() > 1024;
                        for package in packages.iter().take(1024) {
                            let Some(name) = package["name"].as_str() else {
                                continue;
                            };
                            if name.split('/').count() != 2 || name.contains("..") {
                                continue;
                            }
                            let base = package["install-path"].as_str().map_or_else(
                                || self.root.join("vendor").join(name),
                                |p| self.root.join("vendor/composer").join(p),
                            );
                            if let Some(autoload) = package.get("autoload") {
                                source_paths(&mut sources, &base, autoload, &mut truncated);
                            }
                        }
                    } else {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            } else {
                truncated = true;
            }
        }
        sources.sort();
        sources.dedup();
        (sources, truncated)
    }
}
fn source_paths(sources: &mut Vec<PathBuf>, base: &Path, autoload: &Value, truncated: &mut bool) {
    fn add(sources: &mut Vec<PathBuf>, base: &Path, value: &Value, truncated: &mut bool) {
        if let Some(path) = value.as_str() {
            if sources.len() < 256 {
                sources.push(base.join(path));
            } else {
                *truncated = true;
            }
        }
    }
    for key in ["psr-4", "psr-0"] {
        if let Some(maps) = autoload.get(key).and_then(Value::as_object) {
            *truncated |= maps.len() > 256;
            for value in maps.values().take(256) {
                if let Some(dirs) = value.as_array() {
                    *truncated |= dirs.len() > 256;
                    for dir in dirs.iter().take(256) {
                        add(sources, base, dir, truncated);
                    }
                } else {
                    add(sources, base, value, truncated);
                }
            }
        }
    }
    for key in ["classmap", "files"] {
        // Exclusion globs are left to the semantic provider rather than ignored.
        if key == "classmap" && autoload.get("exclude-from-classmap").is_some() {
            continue;
        }
        if let Some(paths) = autoload.get(key).and_then(Value::as_array) {
            *truncated |= paths.len() > 256;
            for path in paths.iter().take(256) {
                add(sources, base, path, truncated);
            }
        }
    }
}

#[cfg(test)]
mod names_tests {
    use super::*;
    fn parse(source: &str) -> Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        parser.parse(source, None).unwrap()
    }
    #[test]
    fn grouped_class_imports_ignore_functions_and_constants() {
        let source = "<?php namespace App; use Library\\Meta\\{Trace as Mark, Other, function helper, const VALUE}; #[Mark] class C {}";
        let names = Names::parse(&parse(source), source).unwrap();
        assert_eq!(
            names.resolve("Mark").as_deref(),
            Some("Library\\Meta\\Trace")
        );
        assert_eq!(
            names.resolve("Other").as_deref(),
            Some("Library\\Meta\\Other")
        );
        assert_eq!(names.resolve("helper").as_deref(), Some("App\\helper"));
        assert_eq!(names.resolve("VALUE").as_deref(), Some("App\\VALUE"));
    }
    #[test]
    fn braced_and_consecutive_namespaces_keep_aliases_local() {
        for source in [
            "<?php namespace A { use First\\Trace as Mark; class One {} } namespace B { use Second\\Trace as Mark; class Two {} } namespace { class GlobalClass {} }",
            "<?php namespace A; use First\\Trace as Mark; class One {} namespace B; use Second\\Trace as Mark; class Two {}",
        ] {
            let tree = parse(source);
            for (anchor, expected) in [
                ("class One", "First\\Trace"),
                ("class Two", "Second\\Trace"),
            ] {
                let names = Names::at(&tree, source, source.find(anchor).unwrap()).unwrap();
                assert_eq!(names.resolve("Mark").as_deref(), Some(expected));
            }
            assert!(Names::parse(&tree, source).is_none());
            if let Some(byte) = source.find("class GlobalClass") {
                let names = Names::at(&tree, source, byte).unwrap();
                assert_eq!(names.resolve("Mark").as_deref(), Some("Mark"));
            }
        }
    }
    #[test]
    fn duplicate_class_aliases_are_rejected() {
        let source = "<?php use A\\{Trace as M}; use B\\Trace as M; class C {}";
        assert!(Names::parse(&parse(source), source).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn source_budget_rejects_fifo_and_directory_without_opening_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("Source.php");
        assert!(
            std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let cancel = AtomicBool::new(false);
        let mut budget = Budget {
            root: dir.path(),
            buffers: &[],
            cancel: &cancel,
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        assert!(budget.read(&fifo).is_none());
        assert!(budget.read(dir.path()).is_none());
        assert_eq!((budget.bytes, budget.files), (0, 0));
        assert!(budget.check().is_some());
    }
}
