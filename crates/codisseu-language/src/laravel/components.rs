//! Source-derived Blade/Livewire/Inertia assistance. Never boots a project.
use super::resources::{ResultSet, argument, position, quoted_node, target, walk};
use crate::php_project::{Budget, ComposerProject, Names, children, text};
use crate::{CompletionEdit, CompletionItem, DefinitionTarget};
use serde_json::Value;
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Blade,
    Livewire,
    Page,
    Prop,
    Slot,
    Property,
    Method,
}
struct Context {
    kind: Kind,
    range: Range<usize>,
    owner: Option<String>,
}
#[derive(Clone)]
struct Entry {
    name: String,
    target: DefinitionTarget,
}
#[derive(Clone)]
struct Component {
    name: String,
    class: Option<String>,
    target: DefinitionTarget,
    view: Option<PathBuf>,
    props: Vec<Entry>,
    methods: Vec<Entry>,
    slots: Vec<Entry>,
}
struct Index<'a> {
    budget: Budget<'a>,
    visited: usize,
    truncated: bool,
}
impl Index<'_> {
    fn read(&mut self, path: &Path) -> Option<(PathBuf, String)> {
        if !safe_path(self.budget.root, path) {
            return None;
        }
        let result = self.budget.read(path);
        if result.is_none() && path.is_file() {
            self.truncated = true;
        }
        result
    }
    fn files(&mut self, dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
        if depth > 16 || self.visited >= 512 || self.budget.check().is_none() {
            self.truncated = true;
            return;
        }
        if !safe_path(self.budget.root, dir) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            if self.visited >= 512 || self.budget.check().is_none() {
                self.truncated = true;
                break;
            }
            self.visited += 1;
            let Ok(entry) = entry else {
                self.truncated = true;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                self.truncated = true;
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                self.files(&entry.path(), depth + 1, files);
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
        files.sort();
    }
    fn package(&mut self, project: &ComposerProject, name: &str) -> bool {
        if ["require", "require-dev"]
            .into_iter()
            .any(|section| project.manifest[section].get(name).is_some())
        {
            return true;
        }
        let path = project.root.join("vendor/composer/installed.json");
        if !path.is_file() {
            return false;
        }
        let Some((_, source)) = self.read(&path) else {
            return false;
        };
        let Ok(data) = serde_json::from_str::<Value>(&source) else {
            return false;
        };
        data.get("packages")
            .unwrap_or(&data)
            .as_array()
            .is_some_and(|packages| {
                packages
                    .iter()
                    .take(1024)
                    .any(|package| package["name"].as_str() == Some(name))
            })
    }
}
fn safe_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return false;
        }
        current.push(part);
        if current
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            return false;
        }
    }
    true
}
fn identifier(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':')
}
fn word(source: &str, byte: usize, predicate: impl Fn(char) -> bool) -> Range<usize> {
    let start = source[..byte]
        .char_indices()
        .rev()
        .take_while(|(_, c)| predicate(*c))
        .last()
        .map_or(byte, |(i, _)| i);
    let end = byte
        + source[byte..]
            .chars()
            .take_while(|c| predicate(*c))
            .map(char::len_utf8)
            .sum::<usize>();
    start..end
}
// A small HTML lexer; quoted attributes, comments, raw script/style and Blade
// comments are skipped instead of matching a tag-looking string inside them.
#[derive(Clone)]
struct Attribute {
    name: Range<usize>,
    value: Option<Range<usize>>,
}
struct Tag {
    name: Range<usize>,
    attrs: Vec<Attribute>,
    range: Range<usize>,
    close: bool,
    empty: bool,
}
fn tags(source: &str) -> Vec<Tag> {
    let bytes = source.as_bytes();
    let mut tags = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if tags.len() >= 4096 {
            break;
        }
        let mut skip = None;
        for (open, close) in [
            ("{{", "}}"),
            ("{!!", "!!}"),
            ("<?", "?>"),
            ("@php", "@endphp"),
        ] {
            if source[at..].starts_with(open) {
                skip = Some(
                    source[at..]
                        .find(close)
                        .map_or(bytes.len() - at, |v| v + close.len()),
                );
                break;
            }
        }
        if let Some(count) = skip {
            at += count;
            continue;
        }
        if source[at..].starts_with("{{--") {
            at += source[at..]
                .find("--}}")
                .map_or(bytes.len() - at, |v| v + 4);
            continue;
        }
        if source[at..].starts_with("<!--") {
            at += source[at..].find("-->").map_or(bytes.len() - at, |v| v + 3);
            continue;
        }
        if bytes[at] != b'<' {
            at += source[at..].chars().next().unwrap().len_utf8();
            continue;
        }
        let start = at;
        at += 1;
        let close = bytes.get(at) == Some(&b'/');
        if close {
            at += 1;
        }
        let name_start = at;
        while at < bytes.len()
            && bytes[at] != b'/'
            && identifier(source[at..].chars().next().unwrap())
        {
            at += source[at..].chars().next().unwrap().len_utf8();
        }
        if at == name_start {
            continue;
        }
        let name = name_start..at;
        let mut attrs = Vec::new();
        loop {
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            if at == bytes.len() || bytes[at] == b'>' || bytes[at] == b'/' {
                break;
            }
            let a = at;
            while at < bytes.len()
                && !bytes[at].is_ascii_whitespace()
                && !matches!(bytes[at], b'=' | b'>' | b'/')
            {
                at += source[at..].chars().next().unwrap().len_utf8();
            }
            if at == a {
                break;
            }
            let attr_name = a..at;
            while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                at += 1;
            }
            let mut value = None;
            if bytes.get(at) == Some(&b'=') {
                at += 1;
                while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
                    at += 1;
                }
                if bytes.get(at).is_some_and(|b| matches!(b, b'\'' | b'"')) {
                    let quote = bytes[at];
                    at += 1;
                    let v = at;
                    while at < bytes.len() && bytes[at] != quote {
                        at += 1;
                    }
                    value = Some(v..at);
                    if at < bytes.len() {
                        at += 1;
                    }
                } else {
                    let v = at;
                    while at < bytes.len() && !bytes[at].is_ascii_whitespace() && bytes[at] != b'>'
                    {
                        at += 1;
                    }
                    value = Some(v..at);
                }
            }
            attrs.push(Attribute {
                name: attr_name,
                value,
            });
        }
        let empty = bytes.get(at) == Some(&b'/');
        if empty {
            at += 1;
        }
        if bytes.get(at) == Some(&b'>') {
            at += 1;
        }
        let raw = &source[name.clone()];
        if !close && matches!(raw, "script" | "style") {
            at += source[at..]
                .find(&format!("</{raw}"))
                .unwrap_or(bytes.len() - at);
        }
        tags.push(Tag {
            name,
            attrs,
            range: start..at,
            close,
            empty,
        });
    }
    tags
}
fn blade_context(source: &str, byte: usize) -> Option<Context> {
    let tags = tags(source);
    let mut owners = Vec::<String>::new();
    for tag in &tags {
        let name = &source[tag.name.clone()];
        if tag.name.start <= byte && byte <= tag.name.end {
            if name.starts_with("x-slot:") {
                return Some(Context {
                    kind: Kind::Slot,
                    range: tag.name.start + 7..tag.name.end,
                    owner: owners.last().cloned(),
                });
            }
            for (prefix, kind) in [("x-", Kind::Blade), ("livewire:", Kind::Livewire)] {
                if name.starts_with(prefix) {
                    return Some(Context {
                        kind,
                        range: tag.name.start + prefix.len()..tag.name.end,
                        owner: None,
                    });
                }
            }
        }
        for attr in &tag.attrs {
            if attr.name.start <= byte
                && byte <= attr.name.end
                && name.starts_with("x-")
                && !tag.close
            {
                let start =
                    attr.name.start + usize::from(source[attr.name.clone()].starts_with(':'));
                return Some(Context {
                    kind: Kind::Prop,
                    range: start..attr.name.end,
                    owner: Some(name[2..].into()),
                });
            }
            if let Some(value) = &attr.value
                && value.start <= byte
                && byte <= value.end
            {
                let attr_name = &source[attr.name.clone()];
                if name == "x-slot" && attr_name == "name" {
                    return Some(Context {
                        kind: Kind::Slot,
                        range: value.clone(),
                        owner: owners.last().cloned(),
                    });
                }
                let property = attr_name == "wire:model" || attr_name.starts_with("wire:model.");
                let method = ["wire:click", "wire:submit", "wire:change", "wire:init"]
                    .into_iter()
                    .any(|n| {
                        attr_name == n
                            || attr_name
                                .strip_prefix(n)
                                .is_some_and(|t| t.starts_with('.'))
                    });
                if property || method {
                    let range = word(source, byte, |c| c.is_alphanumeric() || c == '_');
                    if range.start != value.start {
                        return None;
                    }
                    return Some(Context {
                        kind: if property {
                            Kind::Property
                        } else {
                            Kind::Method
                        },
                        range,
                        owner: None,
                    });
                }
            }
        }
        if tag.range.start > byte {
            break;
        }
        if name.starts_with("x-") && !name.starts_with("x-slot") && tag.range.end <= byte {
            if tag.close {
                owners.pop();
            } else if !tag.empty {
                owners.push(name[2..].into());
            }
        }
    }
    None
}
fn kebab(value: &str) -> String {
    let mut result = String::new();
    let chars: Vec<_> = value.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        if c.is_uppercase()
            && i > 0
            && (chars[i - 1].is_lowercase()
                || chars[i - 1].is_ascii_digit()
                || chars.get(i + 1).is_some_and(|n| n.is_lowercase()))
        {
            result.push('-');
        }
        result.extend(c.to_lowercase());
    }
    result
}
fn conventional(path: &Path, base: &Path, suffix: &str, class: bool) -> Option<String> {
    let relative = path
        .strip_prefix(base)
        .ok()?
        .to_str()?
        .strip_suffix(suffix)?;
    Some(
        relative
            .split('/')
            .map(|s| if class { kebab(s) } else { s.into() })
            .collect::<Vec<_>>()
            .join("."),
    )
}
fn public(node: tree_sitter::Node<'_>, source: &str) -> bool {
    !children(node)
        .iter()
        .any(|n| n.kind() == "visibility_modifier" && text(*n, source) != "public")
        && !children(node).iter().any(|n| n.kind() == "static_modifier")
}
fn php_class(
    index: &Index<'_>,
    path: &Path,
    source: &str,
    base: &str,
    name: String,
) -> Option<Component> {
    let tree = index.budget.tree(source)?;
    let names = Names::parse(&tree, source)?;
    let nodes: Vec<_> = children(tree.root_node())
        .into_iter()
        .filter(|n| n.kind() == "class_declaration")
        .collect();
    if nodes.len() != 1 {
        return None;
    }
    let class = nodes[0];
    if class.has_error() {
        return None;
    }
    let extends = children(class)
        .into_iter()
        .find(|n| n.kind() == "base_clause")?;
    let parents = children(extends);
    if parents.len() != 1 || names.resolve(text(parents[0], source)).as_deref() != Some(base) {
        return None;
    }
    let class_name = class.child_by_field_name("name")?;
    let body = class.child_by_field_name("body")?;
    // Traits may alter render resolution/members; decline instead of guessing.
    if children(body).iter().any(|n| n.kind() == "use_declaration") {
        return None;
    }
    let mut component = Component {
        name,
        class: names.resolve(text(class_name, source)),
        target: target(path, source, class_name.byte_range()),
        view: None,
        props: Vec::new(),
        methods: Vec::new(),
        slots: Vec::new(),
    };
    for member in children(body) {
        if !public(member, source) {
            continue;
        }
        if member.kind() == "property_declaration" {
            for element in children(member)
                .into_iter()
                .filter(|n| n.kind() == "property_element")
            {
                if let Some(variable) = element.child_by_field_name("name") {
                    component.props.push(Entry {
                        name: text(variable, source).trim_start_matches('$').into(),
                        target: target(path, source, variable.byte_range()),
                    });
                }
            }
        }
        if member.kind() != "method_declaration" {
            continue;
        }
        let Some(method) = member.child_by_field_name("name") else {
            continue;
        };
        let method_name = text(method, source);
        if !method_name.starts_with('_')
            && !matches!(
                method_name,
                "render" | "mount" | "boot" | "hydrate" | "dehydrate"
            )
        {
            component.methods.push(Entry {
                name: method_name.into(),
                target: target(path, source, method.byte_range()),
            });
        }
        if method_name == "__construct"
            && let Some(params) = member.child_by_field_name("parameters")
        {
            for param in children(params) {
                if !children(param)
                    .iter()
                    .any(|n| n.kind() == "visibility_modifier" && text(*n, source) == "public")
                {
                    continue;
                }
                if let Some(variable) = param.child_by_field_name("name") {
                    component.props.push(Entry {
                        name: kebab(text(variable, source).trim_start_matches('$')),
                        target: target(path, source, variable.byte_range()),
                    });
                }
            }
        }
        if method_name == "render" {
            let mut views = Vec::new();
            walk(member, &mut |node| {
                if node.kind() != "return_statement" {
                    return;
                }
                let values = children(node);
                if values.len() != 1 || values[0].kind() != "function_call_expression" {
                    return;
                }
                let call = values[0];
                if call
                    .child_by_field_name("function")
                    .is_some_and(|n| matches!(text(n, source), "view" | "\\view"))
                    && let Some((view, _)) = argument(call, 0).and_then(|n| quoted_node(n, source))
                {
                    views.push(view);
                }
            });
            if views.len() == 1 && resource_name(&views[0]) {
                component.view = Some(PathBuf::from(
                    views.remove(0).replace('.', "/") + ".blade.php",
                ));
            } else {
                return None;
            }
        }
    }
    Some(component)
}
fn resource_name(name: &str) -> bool {
    !name.is_empty()
        && name.split(['.', '/']).all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
}
fn view_data(index: &Index<'_>, path: &Path, source: &str) -> (Vec<Entry>, Vec<Entry>) {
    // Project @props(...) into PHP while retaining the original byte positions.
    let mut props = Vec::new();
    let mut slots = Vec::new();
    let visible = masked(source);
    for (start, _) in visible.match_indices("@props") {
        let open = start + 6;
        let tail = visible[open..].trim_start();
        if !tail.starts_with('(') {
            continue;
        }
        let offset = open + visible[open..].len() - tail.len() + 1;
        let parse = format!("<?php __props({}", &visible[offset..]);
        let Some(tree) = index.budget.tree(&parse) else {
            continue;
        };
        walk(tree.root_node(), &mut |node| {
            if node.kind() != "function_call_expression"
                || node
                    .child_by_field_name("function")
                    .is_none_or(|n| text(n, &parse) != "__props")
            {
                return;
            }
            let Some(array) = argument(node, 0) else {
                return;
            };
            if array.kind() != "array_creation_expression" {
                return;
            }
            for element in children(array) {
                let parts = children(element);
                if let Some((name, range)) = parts.first().and_then(|n| quoted_node(*n, &parse)) {
                    if !name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                    {
                        continue;
                    }
                    let start = range.start + offset - 14;
                    let end = range.end + offset - 14;
                    if end <= source.len() {
                        props.push(Entry {
                            name,
                            target: target(path, source, start..end),
                        });
                    }
                }
            }
        });
    }
    // Slot names are the root variables of Blade echo expressions; excluding
    // declared props and built-ins avoids suggesting every template variable.
    for (start, _) in visible.match_indices("{{") {
        let Some(end) = visible[start + 2..].find("}}") else {
            continue;
        };
        let expression = &visible[start + 2..start + 2 + end];
        let trim = expression.trim_start();
        if !trim.starts_with('$') {
            continue;
        }
        let name: String = trim[1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty()
            || matches!(
                name.as_str(),
                "slot" | "attributes" | "component" | "loop" | "errors"
            )
            || props.iter().any(|p| p.name == name)
        {
            continue;
        }
        let at = start + 2 + expression.len() - trim.len() + 1;
        slots.push(Entry {
            name: name.clone(),
            target: target(path, source, at..at + name.len()),
        });
    }
    (props, slots)
}
fn masked(source: &str) -> String {
    let mut bytes = source.as_bytes().to_vec();
    for (open, close) in [
        ("{{--", "--}}"),
        ("<!--", "-->"),
        ("<script", "</script>"),
        ("<style", "</style>"),
    ] {
        let mut at = 0;
        while let Some(start) = source[at..].find(open) {
            let start = at + start;
            let end = start
                + source[start..]
                    .find(close)
                    .map_or(source.len() - start, |n| n + close.len());
            for b in &mut bytes[start..end] {
                if !matches!(*b, b'\n' | b'\r') {
                    *b = b' ';
                }
            }
            at = end;
        }
    }
    String::from_utf8(bytes).unwrap()
}
fn php_context(index: &Index<'_>, source: &str, byte: usize) -> Option<Context> {
    let tree = index.budget.tree(source)?;
    let names = Names::parse(&tree, source)?;
    let mut context = None;
    walk(tree.root_node(), &mut |node| {
        let valid = if node.kind() == "function_call_expression" {
            node.child_by_field_name("function")
                .is_some_and(|n| matches!(text(n, source), "inertia" | "\\inertia"))
        } else if node.kind() == "scoped_call_expression" {
            node.child_by_field_name("scope")
                .and_then(|n| names.resolve(text(n, source)))
                .as_deref()
                == Some("Inertia\\Inertia")
                && node
                    .child_by_field_name("name")
                    .is_some_and(|n| text(n, source) == "render")
        } else {
            false
        };
        if valid
            && let Some((_, range)) = argument(node, 0).and_then(|n| quoted_node(n, source))
            && range.start <= byte
            && byte <= range.end
        {
            context = Some(Context {
                kind: Kind::Page,
                range,
                owner: None,
            });
        }
    });
    context
}
fn incomplete_php_context(index: &Index<'_>, source: &str, byte: usize) -> Option<Context> {
    if let Some(context) = php_context(index, source, byte) {
        return Some(context);
    }
    // A completion request commonly arrives before the closing quote/parenthesis.
    // Repair only the suffix, then clip its literal range to the actual document.
    for quote in ['\'', '"'] {
        let parsed = format!("{source}{quote});");
        if let Some(mut context) = php_context(index, &parsed, byte) {
            context.range.end = context.range.end.min(source.len());
            return Some(context);
        }
    }
    None
}
fn directive_context(index: &Index<'_>, source: &str, byte: usize) -> Option<Context> {
    let visible = masked(source);
    for (at, _) in visible.match_indices("@livewire") {
        let tail = &visible[at + 9..];
        if !tail.trim_start().starts_with('(') {
            continue;
        }
        let open = at + 9 + tail.find('(')?;
        let parsed = format!("<?php __livewire{};", &visible[open..]);
        let tree = index.budget.tree(&parsed)?;
        let mut found = None;
        walk(tree.root_node(), &mut |node| {
            if node.kind() != "function_call_expression"
                || node
                    .child_by_field_name("function")
                    .is_none_or(|n| text(n, &parsed) != "__livewire")
            {
                return;
            }
            if let Some((_, range)) = argument(node, 0).and_then(|n| quoted_node(n, &parsed)) {
                let range = range.start + open - 16..range.end + open - 16;
                if range.start <= byte && byte <= range.end {
                    found = Some(Context {
                        kind: Kind::Livewire,
                        range,
                        owner: None,
                    });
                }
            }
        });
        if found.is_some() {
            return found;
        }
    }
    None
}
fn components(index: &mut Index<'_>, project: &Path, livewire: bool) -> Vec<Component> {
    let mut result = Vec::new();
    let views = project.join("resources/views");
    if !livewire {
        let base = views.join("components");
        let mut files = Vec::new();
        index.files(&base, 0, &mut files);
        for path in files {
            let Some(name) = conventional(&path, &base, ".blade.php", false) else {
                continue;
            };
            let Some((path, source)) = index.read(&path) else {
                continue;
            };
            let (props, slots) = view_data(index, &path, &source);
            result.push(Component {
                name,
                class: None,
                target: target(&path, &source, 0..0),
                view: Some(path),
                props,
                methods: Vec::new(),
                slots,
            });
        }
    }
    let roots: &[&str] = if livewire {
        &["app/Livewire", "app/Http/Livewire"]
    } else {
        &["app/View/Components"]
    };
    for directory in roots {
        let base = project.join(directory);
        let mut files = Vec::new();
        index.files(&base, 0, &mut files);
        for path in files {
            let Some(name) = conventional(&path, &base, ".php", true) else {
                continue;
            };
            let Some((path, source)) = index.read(&path) else {
                continue;
            };
            let Some(mut component) = php_class(
                index,
                &path,
                &source,
                if livewire {
                    "Livewire\\Component"
                } else {
                    "Illuminate\\View\\Component"
                },
                name,
            ) else {
                continue;
            };
            // Conventional class namespaces are verified, rather than interpreting
            // arbitrary classes in the same directory as framework components.
            let namespace = if livewire && *directory == "app/Http/Livewire" {
                "App\\Http\\Livewire\\"
            } else if livewire {
                "App\\Livewire\\"
            } else {
                "App\\View\\Components\\"
            };
            if component
                .class
                .as_ref()
                .is_none_or(|n| !n.starts_with(namespace))
            {
                continue;
            }
            let view = component
                .view
                .take()
                .map(|v| views.join(v))
                .unwrap_or_else(|| {
                    views.join(format!(
                        "{}/{}.blade.php",
                        if livewire { "livewire" } else { "components" },
                        component.name.replace('.', "/")
                    ))
                });
            if let Some((view, source)) = index.read(&view) {
                let (props, slots) = view_data(index, &view, &source);
                component.props.extend(props);
                component.slots = slots;
                component.view = Some(view);
            }
            // A class component has precedence over its implementation view.
            if !livewire {
                result.retain(|c| c.name != component.name || c.class.is_some());
            }
            result.push(component);
        }
    }
    result
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn query(
    workspace: &Path,
    path: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
    completion: bool,
) -> Option<ResultSet> {
    if source.len() > 1024 * 1024 {
        return None;
    }
    let byte = crate::utf16_byte_offset(source, line, character)?;
    let mut index = Index {
        budget: Budget {
            root: workspace,
            buffers,
            cancel,
            start: Instant::now(),
            bytes: 0,
            files: 0,
        },
        visited: 0,
        truncated: false,
    };
    index.budget.check()?;
    let blade = path.to_str()?.ends_with(".blade.php");
    let context = if blade {
        blade_context(source, byte).or_else(|| directive_context(&index, source, byte))
    } else {
        incomplete_php_context(&index, source, byte)
    }?;
    if context.range.start > byte || context.range.end > source.len() {
        return None;
    }
    let project = ComposerProject::discover(&mut index.budget, path)?;
    if !project.is_laravel() {
        return None;
    }
    let mut entries = Vec::new();
    match context.kind {
        Kind::Page => {
            if !index.package(&project, "inertiajs/inertia-laravel") {
                return None;
            }
            for directory in ["resources/js/Pages", "resources/js/pages"] {
                let base = project.root.join(directory);
                let mut files = Vec::new();
                index.files(&base, 0, &mut files);
                for path in files {
                    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                        continue;
                    };
                    if !matches!(ext, "vue" | "js" | "jsx" | "ts" | "tsx" | "svelte") {
                        continue;
                    }
                    let Some(name) = path
                        .strip_prefix(&base)
                        .ok()
                        .and_then(|p| p.to_str())
                        .and_then(|p| p.strip_suffix(&format!(".{ext}")))
                    else {
                        continue;
                    };
                    if name.contains(['\'', '"', '$', '\\', '\n', '\r']) {
                        continue;
                    }
                    if let Ok(path) = path.canonicalize() {
                        entries.push(Entry {
                            name: name.into(),
                            target: target(&path, "", 0..0),
                        });
                    }
                }
            }
        }
        Kind::Blade | Kind::Prop | Kind::Slot => {
            for component in components(&mut index, &project.root, false) {
                if context.kind == Kind::Blade {
                    entries.push(Entry {
                        name: component.name,
                        target: component.target,
                    });
                } else if context.owner.as_deref() == Some(&component.name) {
                    entries.extend(if context.kind == Kind::Prop {
                        component.props
                    } else {
                        component.slots
                    });
                }
            }
        }
        Kind::Livewire | Kind::Property | Kind::Method => {
            if !index.package(&project, "livewire/livewire") {
                return None;
            }
            let current = path.canonicalize().ok()?;
            for component in components(&mut index, &project.root, true) {
                if context.kind == Kind::Livewire {
                    entries.push(Entry {
                        name: component.name,
                        target: component.target,
                    });
                } else if component.view.as_ref() == Some(&current) {
                    entries.extend(if context.kind == Kind::Property {
                        component.props
                    } else {
                        component.methods
                    });
                }
            }
        }
    }
    index.truncated |= index.budget.check().is_none();
    entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.target.path.cmp(&b.target.path)));
    let full = &source[context.range.clone()];
    let prefix = &source[context.range.start..byte];
    let mut targets: Vec<_> = entries
        .iter()
        .filter(|e| e.name == full)
        .map(|e| e.target.clone())
        .collect();
    targets.dedup();
    if targets.len() > 64 {
        index.truncated = true;
    }
    if index.truncated {
        targets.clear();
    }
    let mut items = Vec::new();
    if completion {
        let (start_line, start_character) = position(source, context.range.start);
        let (end_line, end_character) = position(source, context.range.end);
        for entry in entries.into_iter().filter(|e| e.name.starts_with(prefix)) {
            if items.iter().any(|i: &CompletionItem| i.label == entry.name) {
                continue;
            }
            if items.len() == 128 {
                index.truncated = true;
                break;
            }
            items.push(CompletionItem {
                label: entry.name.clone(),
                detail: Some(format!(
                    "Laravel {:?} · {} · {}",
                    context.kind,
                    if context.kind == Kind::Slot {
                        "inferred slot candidate from rendered variable"
                    } else {
                        "static source convention"
                    },
                    entry
                        .target
                        .path
                        .strip_prefix(workspace)
                        .unwrap_or(&entry.target.path)
                        .display()
                )),
                kind: Some(
                    if matches!(context.kind, Kind::Property | Kind::Prop | Kind::Slot) {
                        10
                    } else if context.kind == Kind::Method {
                        2
                    } else {
                        7
                    },
                ),
                filter_text: None,
                sort_text: None,
                new_text: entry.name.clone(),
                text_edit: Some(CompletionEdit {
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    new_text: entry.name,
                }),
                insert_edit: None,
                additional_edits: Vec::new(),
                unsupported_reason: None,
            });
        }
    }
    Some(ResultSet {
        items,
        targets,
        truncated: index.truncated,
    })
}

#[cfg(test)]
#[path = "components_tests.rs"]
mod tests;
