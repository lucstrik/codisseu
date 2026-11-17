//! Static Laravel resource names from project declarations. Never boots PHP.
use super::{Budget, Names, children, text};
use crate::{CompletionEdit, CompletionItem, DefinitionTarget};
use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};
use tree_sitter::Node;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    View,
    Config,
    Translation,
    Route,
    Asset,
    Controller,
}
struct Context {
    kind: Kind,
    range: Range<usize>,
    prefix: String,
    name: String,
    controller: Option<String>,
}
struct Entry {
    name: String,
    target: DefinitionTarget,
}
pub(crate) struct ResultSet {
    pub items: Vec<CompletionItem>,
    pub targets: Vec<DefinitionTarget>,
    pub truncated: bool,
}
pub(crate) fn position(source: &str, byte: usize) -> (u32, u32) {
    let prefix = &source[..byte];
    (
        prefix.bytes().filter(|b| *b == b'\n').count() as u32,
        prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count() as u32,
    )
}
pub(crate) fn target(path: &Path, source: &str, range: Range<usize>) -> DefinitionTarget {
    let (start_line, start_character) = position(source, range.start);
    let (end_line, end_character) = position(source, range.end);
    DefinitionTarget {
        path: path.into(),
        start_line,
        start_character,
        end_line,
        end_character,
    }
}
// Returns literal content and the position after its closing delimiter. Escaped
// names/interpolation deliberately require a richer PHP value evaluator.
pub(crate) fn literal(source: &str, start: usize) -> Option<(Range<usize>, usize)> {
    let quote = *source.as_bytes().get(start)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    let mut end = start + 1;
    let mut plain = true;
    while let Some(&byte) = source.as_bytes().get(end) {
        if byte == quote {
            return plain.then_some((start + 1..end, end + 1));
        }
        if byte == b'\\' {
            plain = false;
            end += 1;
        }
        if byte == b'$' && quote == b'"' {
            plain = false;
        }
        end += 1;
    }
    plain.then_some((start + 1..source.len(), source.len()))
}
pub(crate) fn quoted_node(node: Node<'_>, source: &str) -> Option<(String, Range<usize>)> {
    if !matches!(node.kind(), "string" | "encapsed_string") {
        return None;
    }
    let (range, end) = literal(source, node.start_byte())?;
    if end != node.end_byte() || end == range.end {
        return None;
    }
    Some((source[range.clone()].into(), range))
}
pub(crate) fn walk<'a>(node: Node<'a>, callback: &mut impl FnMut(Node<'a>)) {
    // Parsing limits input size; traversal also avoids recursion on hostile ASTs.
    let mut cursor = node.walk();
    loop {
        callback(cursor.node());
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}
pub(crate) fn argument(node: Node<'_>, index: usize) -> Option<Node<'_>> {
    let args = node.child_by_field_name("arguments")?;
    let arg = children(args)
        .into_iter()
        .filter(|n| n.kind() == "argument")
        .nth(index)?;
    // Named arguments are supported only where the parameter name is known.
    if arg.child_by_field_name("name").is_some() {
        return None;
    }
    children(arg).into_iter().next()
}
fn facade(node: Node<'_>, source: &str, names: &Names, class: &str) -> bool {
    node.child_by_field_name("scope")
        .and_then(|n| names.resolve(text(n, source)))
        .is_some_and(|n| n == format!("Illuminate\\Support\\Facades\\{class}"))
}
fn call_kind(node: Node<'_>, source: &str, names: &Names, blade: bool) -> Option<Kind> {
    if node.kind() == "function_call_expression" {
        let name = text(node.child_by_field_name("function")?, source);
        // Unqualified helpers can be shadowed by namespaced user functions. This
        // adapter describes conventional Laravel helper resolution, not runtime.
        return match name.trim_start_matches('\\') {
            "view" => Some(Kind::View),
            "config" => Some(Kind::Config),
            "__" | "trans" | "trans_choice" => Some(Kind::Translation),
            "route" | "to_route" => Some(Kind::Route),
            "asset" | "secure_asset" => Some(Kind::Asset),
            "_xtends" | "_nclude" | "_ncludeIf" | "_ncludeOnce" | "_omponent" | "_ach" if blade => {
                Some(Kind::View)
            }
            "_ang" | "_hoice" if blade => Some(Kind::Translation),
            _ => None,
        };
    }
    if node.kind() == "scoped_call_expression" {
        let method = text(node.child_by_field_name("name")?, source);
        return if facade(node, source, names, "View") && matches!(method, "make" | "exists") {
            Some(Kind::View)
        } else if facade(node, source, names, "Config") && matches!(method, "get" | "has" | "set") {
            Some(Kind::Config)
        } else if facade(node, source, names, "Lang") && matches!(method, "get" | "has" | "choice")
        {
            Some(Kind::Translation)
        } else if facade(node, source, names, "Route") && method == "has" {
            Some(Kind::Route)
        } else {
            None
        };
    }
    None
}

fn controller_at(node: Node<'_>, source: &str, names: &Names, at: usize) -> Option<String> {
    if node.kind() != "scoped_call_expression" || !facade(node, source, names, "Route") {
        return None;
    }
    let method = text(node.child_by_field_name("name")?, source);
    let action = argument(node, if method == "match" { 2 } else { 1 })?;
    if !matches!(
        method,
        "get" | "post" | "put" | "patch" | "delete" | "options" | "any" | "match"
    ) || action.kind() != "array_creation_expression"
    {
        return None;
    }
    let elements = children(action);
    if elements.len() != 2 {
        return None;
    }
    let first = children(elements[0]);
    let second = children(elements[1]);
    if first.len() != 1
        || second.len() != 1
        || second[0].start_byte() != at
        || quoted_node(second[0], source).is_none()
    {
        return None;
    }
    if first[0].kind() != "class_constant_access_expression" {
        return None;
    }
    let class = children(first[0]);
    if class.len() != 2 || text(class[1], source) != "class" {
        return None;
    }
    names.resolve(text(class[0], source))
}

fn parsed_context(
    budget: &Budget<'_>,
    source: &str,
    at: usize,
    blade: bool,
) -> Option<(Kind, Option<String>)> {
    let tree = budget.tree(source)?;
    let names = Names::parse(&tree, source)?;
    let mut found = None;
    walk(tree.root_node(), &mut |node| {
        if found.is_some() {
            return;
        }
        if let Some(controller) = controller_at(node, source, &names, at) {
            found = Some((Kind::Controller, Some(controller)));
            return;
        }
        let Some(kind) = call_kind(node, source, &names, blade) else {
            return;
        };
        let Some(arg) = argument(node, 0) else {
            return;
        };
        if arg.start_byte() == at && quoted_node(arg, source).is_some() {
            found = Some((kind, None));
        }
    });
    found
}

fn group_name(mut call: Node<'_>, source: &str, names: &Names) -> Option<String> {
    let mut name = None;
    loop {
        let scoped = call.kind() == "scoped_call_expression";
        if !scoped && call.kind() != "member_call_expression" {
            return None;
        }
        let method = text(call.child_by_field_name("name")?, source);
        match method {
            "name" if name.is_none() => {
                name = Some(quoted_node(argument(call, 0)?, source)?.0);
            }
            "group" if scoped => {
                let array = argument(call, 0)?;
                if array.kind() != "array_creation_expression" {
                    return None;
                }
                for element in children(array) {
                    let parts = children(element);
                    if parts.len() != 2 {
                        return None;
                    }
                    let (key, _) = quoted_node(parts[0], source)?;
                    if key == "as" {
                        name = Some(quoted_node(parts[1], source)?.0);
                    }
                }
            }
            "group"
            | "name"
            | "prefix"
            | "middleware"
            | "withoutMiddleware"
            | "domain"
            | "controller"
            | "namespace"
            | "scopeBindings"
            | "withoutScopedBindings"
            | "where"
            | "whereNumber"
            | "whereAlpha"
            | "whereAlphaNumeric"
            | "whereUuid"
            | "whereUlid"
            | "whereIn" => {}
            _ => return None,
        }
        if scoped {
            return facade(call, source, names, "Route").then(|| name.unwrap_or_default());
        }
        call = call.child_by_field_name("object")?;
    }
}

fn group_prefix(node: Node<'_>, source: &str, names: &Names) -> Option<String> {
    let mut prefixes = Vec::new();
    let mut parent = node.parent();
    while let Some(p) = parent {
        if matches!(p.kind(), "anonymous_function" | "arrow_function") {
            let argument = p.parent()?;
            if argument.kind() != "argument" {
                return None;
            }
            let call = argument.parent()?.parent()?;
            if call
                .child_by_field_name("name")
                .is_none_or(|n| text(n, source) != "group")
            {
                return None;
            }
            prefixes.push(group_name(call, source, names)?);
        }
        parent = p.parent();
    }
    prefixes.reverse();
    Some(prefixes.concat())
}
// Conventional resource registrar names documented for Laravel. Modifiers that
// can change registration without a static value deliberately reject this set.
fn resource_routes(
    node: Node<'_>,
    source: &str,
    names: &Names,
) -> Option<Vec<(String, Range<usize>)>> {
    if node.kind() != "scoped_call_expression" || !facade(node, source, names, "Route") {
        return None;
    }
    let method = text(node.child_by_field_name("name")?, source);
    if !matches!(method, "resource" | "apiResource") || argument(node, 2).is_some() {
        return None;
    }
    let (resource, range) = quoted_node(argument(node, 0)?, source)?;
    if resource.is_empty()
        || resource.split('.').any(|s| {
            s.is_empty()
                || !s
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        })
    {
        return None;
    }
    let mut actions = vec![
        "index", "create", "store", "show", "edit", "update", "destroy",
    ];
    if method == "apiResource" {
        actions.retain(|a| !matches!(*a, "create" | "edit"));
    }
    let mut renamed = std::collections::HashMap::new();
    let mut base = resource;
    let mut call = node;
    while let Some(parent) = call.parent().filter(|p| {
        p.kind() == "member_call_expression" && p.child_by_field_name("object") == Some(call)
    }) {
        let modifier = text(parent.child_by_field_name("name")?, source);
        match modifier {
            "only" | "except" => {
                let arg = argument(parent, 0)?;
                let values = if arg.kind() == "array_creation_expression" {
                    children(arg)
                        .into_iter()
                        .map(|n| {
                            let parts = children(n);
                            (n.kind() == "array_element_initializer" && parts.len() == 1)
                                .then(|| quoted_node(parts[0], source).map(|p| p.0))
                                .flatten()
                        })
                        .collect::<Option<Vec<_>>>()?
                } else {
                    let args = parent.child_by_field_name("arguments")?;
                    children(args)
                        .into_iter()
                        .enumerate()
                        .map(|(i, _)| quoted_node(argument(parent, i)?, source).map(|p| p.0))
                        .collect::<Option<Vec<_>>>()?
                };
                actions.retain(|a| values.iter().any(|v| v == a) == (modifier == "only"));
            }
            "names" => {
                let arg = argument(parent, 0)?;
                if let Some((name, _)) = quoted_node(arg, source) {
                    base = name;
                } else if arg.kind() == "array_creation_expression" {
                    for entry in children(arg) {
                        let parts = children(entry);
                        if parts.len() != 2 {
                            return None;
                        }
                        renamed.insert(
                            quoted_node(parts[0], source)?.0,
                            quoted_node(parts[1], source)?.0,
                        );
                    }
                } else {
                    return None;
                }
            }
            "name" => {
                renamed.insert(
                    quoted_node(argument(parent, 0)?, source)?.0,
                    quoted_node(argument(parent, 1)?, source)?.0,
                );
            }
            "middleware" | "withoutMiddleware" | "parameters" | "parameter" | "scoped"
            | "missing" | "withTrashed" => {}
            _ => return None,
        }
        call = parent;
    }
    let prefix = group_prefix(node, source, names)?;
    Some(
        actions
            .into_iter()
            .map(|action| {
                let name = renamed
                    .get(action)
                    .cloned()
                    .unwrap_or_else(|| format!("{base}.{action}"));
                (format!("{prefix}{name}"), range.clone())
            })
            .collect(),
    )
}
// Skip comments and quoted strings while finding a delimiter in a PHP fragment.
fn expression_end(source: &str, mut at: usize, terminator: &str) -> usize {
    while at < source.len() {
        if source[at..].starts_with(terminator) {
            return at;
        }
        let byte = source.as_bytes()[at];
        if matches!(byte, b'\'' | b'"' | b'`') {
            let quote = byte;
            at += 1;
            while at < source.len() {
                if source.as_bytes()[at] == b'\\' {
                    at = (at + 2).min(source.len());
                    continue;
                }
                if source.as_bytes()[at] == quote {
                    at += 1;
                    break;
                }
                at += 1;
            }
        } else if source[at..].starts_with("/*") {
            at += source[at + 2..]
                .find("*/")
                .map_or(source.len() - at, |n| n + 4);
        } else if source[at..].starts_with("//") || byte == b'#' {
            at += source[at..].find('\n').unwrap_or(source.len() - at);
        } else {
            at += source[at..].chars().next().unwrap().len_utf8();
        }
    }
    source.len()
}
fn balanced_end(source: &str, open: usize) -> usize {
    let mut depth = 1;
    let mut at = open + 1;
    while at < source.len() {
        let rest = &source[at..];
        if rest.starts_with("/*") {
            at += rest.find("*/").map_or(rest.len(), |n| n + 2);
        } else if rest.starts_with("//") || rest.starts_with('#') {
            at += rest.find('\n').unwrap_or(rest.len());
        } else if matches!(source.as_bytes()[at], b'\'' | b'"' | b'`') {
            let quote = source.as_bytes()[at];
            at += 1;
            while at < source.len() {
                if source.as_bytes()[at] == b'\\' {
                    at = (at + 2).min(source.len());
                    continue;
                }
                if source.as_bytes()[at] == quote {
                    at += 1;
                    break;
                }
                at += 1;
            }
        } else {
            match source.as_bytes()[at] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return at + 1;
                    }
                }
                _ => {}
            }
            at += rest.chars().next().unwrap().len_utf8();
        }
    }
    source.len()
}
pub(crate) fn fragment(source: &str, byte: usize, blade: bool) -> Option<(String, usize)> {
    if !blade {
        return Some((source.into(), 0));
    }
    let mut at = 0;
    while at < source.len() {
        let tail = &source[at..];
        if let Some(close) = if tail.starts_with("@verbatim") {
            Some("@endverbatim")
        } else if tail.starts_with("{{--") {
            Some("--}}")
        } else if tail.starts_with("<!--") {
            Some("-->")
        } else {
            None
        } {
            at += tail.find(close).map_or(tail.len(), |n| n + close.len());
            continue;
        }
        let region = if tail.starts_with("{!!") {
            Some((at + 3, "!!}"))
        } else if tail.starts_with("{{") {
            Some((at + 2, "}}"))
        } else if tail.starts_with("<?php") {
            Some((at + 5, "?>"))
        } else if tail.starts_with("@php") && !tail[4..].starts_with('(') {
            Some((at + 4, "@endphp"))
        } else {
            None
        };
        if let Some((start, close)) = region {
            let end = expression_end(source, start, close);
            if start <= byte && byte <= end {
                return Some((source[start..end].into(), start));
            }
            at = (end + close.len()).min(source.len());
            continue;
        }
        if tail.starts_with('@') && !tail.starts_with("@@") {
            let length = tail[1..]
                .find(|c: char| !c.is_ascii_alphabetic())
                .map_or(tail.len(), |n| n + 1);
            let name = &tail[1..length];
            let open = at + length;
            let open = open + source[open..].len() - source[open..].trim_start().len();
            if source.as_bytes().get(open) == Some(&b'(') {
                // A complete PHP call can contain nested parentheses. Closing
                // string recognition below is independent of the closing paren.
                let end = balanced_end(source, open);
                if open < byte && byte <= end {
                    let mut part = source[at + 1..end].to_string();
                    if !name.is_empty() {
                        part.replace_range(..1, "_");
                    }
                    return Some((part, at + 1));
                }
                at = end;
                continue;
            }
        }
        if tail.starts_with("@@") {
            at += 2;
        } else {
            at += tail.chars().next()?.len_utf8();
        }
    }
    None
}

fn directive_range(source: &str, byte: usize) -> Option<Range<usize>> {
    // A directive name is a lexical token outside PHP/echo/directive arguments.
    if fragment(source, byte, true).is_some() {
        return None;
    }
    let start = source[..byte].rfind('@')?;
    if source[start + 1..byte]
        .chars()
        .any(|c| !c.is_ascii_alphabetic())
        || source[..start].ends_with('@')
    {
        return None;
    }
    let mut at = 0;
    while at < start {
        let tail = &source[at..];
        let close = if tail.starts_with("{{--") {
            Some("--}}")
        } else if tail.starts_with("<!--") {
            Some("-->")
        } else if tail.starts_with("@verbatim") {
            Some("@endverbatim")
        } else {
            None
        };
        if let Some(close) = close {
            let end = at + tail.find(close).map_or(tail.len(), |n| n + close.len());
            if start < end {
                return None;
            }
            at = end;
        } else {
            at += tail.chars().next()?.len_utf8();
        }
    }
    let end = byte
        + source[byte..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
    Some(start..end)
}
fn context(budget: &Budget<'_>, source: &str, byte: usize, blade: bool) -> Option<Context> {
    let (part, offset) = fragment(source, byte, blade)?;
    let header = if blade { "<?php " } else { "" };
    let input = format!("{header}{part}");
    let caret = byte.checked_sub(offset)? + header.len();
    // Scan PHP lexical strings, excluding comments and inline HTML. The parser
    // below determines whether a literal is a first argument of a supported API.
    let mut at = if blade { header.len() } else { 0 };
    let mut php = blade;
    while at < input.len() {
        let rest = &input[at..];
        if !php {
            let begin = rest.find("<?")?;
            at += begin + 2;
            if input[at..].starts_with("php") {
                at += 3;
            } else if input[at..].starts_with('=') {
                at += 1;
            }
            php = true;
            continue;
        }
        if rest.starts_with("?>") {
            php = false;
            at += 2;
            continue;
        }
        if rest.starts_with("/*") {
            at += rest.find("*/").map_or(rest.len(), |n| n + 2);
            continue;
        }
        if rest.starts_with("//") || (rest.starts_with('#') && !rest.starts_with("#[")) {
            at += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("<<<") {
            return None;
        }
        if matches!(input.as_bytes()[at], b'\'' | b'"') {
            let quote = input.as_bytes()[at];
            let mut end = at + 1;
            while end < input.len() {
                if input.as_bytes()[end] == b'\\' {
                    end = (end + 2).min(input.len());
                    continue;
                }
                if input.as_bytes()[end] == quote {
                    break;
                }
                end += 1;
            }
            if at < caret && caret <= end {
                let (range, _) = literal(&input, at)?;
                if range.end != end {
                    return None;
                }
                let mut found = if end < input.len() {
                    parsed_context(budget, &input, at, blade)
                } else {
                    None
                };
                for suffix in [");", "]);"] {
                    if found.is_some() {
                        break;
                    }
                    let parse = format!("{}{}{suffix}", &input[..end], quote as char);
                    found = parsed_context(budget, &parse, at, blade);
                }
                let (kind, controller) = found?;
                let start = range.start + offset - header.len();
                let end = range.end + offset - header.len();
                if source[start..end].contains(['\n', '\r', '\0']) {
                    return None;
                }
                return Some(Context {
                    kind,
                    range: start..end,
                    prefix: source[start..byte].into(),
                    name: source[start..end].into(),
                    controller,
                });
            }
            at = (end + 1).min(input.len());
            continue;
        }
        if input.as_bytes()[at] == b'`' {
            return None;
        }
        at += rest.chars().next()?.len_utf8();
    }
    None
}
struct Inventory<'a> {
    budget: Budget<'a>,
    entries: Vec<Entry>,
    visited: usize,
    truncated: bool,
}
impl Inventory<'_> {
    fn add(&mut self, name: String, target: DefinitionTarget) {
        if self.entries.len() >= 8192 || self.budget.check().is_none() {
            self.truncated = true;
            return;
        }
        if !name.is_empty()
            && name.len() <= 1024
            && !name.contains(['\0', '\n', '\r', '\'', '"', '\\', '$'])
        {
            self.entries.push(Entry { name, target });
        }
    }
    fn files(&mut self, dir: &Path, depth: usize, output: &mut Vec<PathBuf>) {
        if depth > 16 || self.budget.check().is_none() {
            self.truncated = true;
            return;
        }
        let Ok(meta) = dir.symlink_metadata() else {
            return;
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return;
        }
        let Ok(canonical) = dir.canonicalize() else {
            return;
        };
        if !canonical.starts_with(self.budget.root) {
            return;
        }
        let Ok(files) = std::fs::read_dir(dir) else {
            return;
        };
        // Bound enumeration before sorting, including directories and non-files.
        let mut paths = Vec::new();
        for file in files {
            self.visited += 1;
            if self.visited > 4096 || self.budget.check().is_none() {
                self.truncated = true;
                break;
            }
            let Ok(file) = file else {
                continue;
            };
            let Ok(ty) = file.file_type() else {
                continue;
            };
            if !ty.is_symlink() {
                paths.push((file.path(), ty.is_dir()));
            }
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, dir) in paths {
            if dir {
                self.files(&path, depth + 1, output);
            } else {
                output.push(path);
            }
        }
    }
    fn read(&mut self, path: &Path) -> Option<(PathBuf, String)> {
        let result = self.budget.read(path);
        if result.is_none() {
            self.truncated = true;
        }
        result
    }
    fn arrays(&mut self, node: Node<'_>, path: &Path, source: &str, prefix: &str, depth: usize) {
        if depth > 16 || self.budget.check().is_none() {
            self.truncated = true;
            return;
        }
        for element in children(node) {
            if self.budget.check().is_none() {
                self.truncated = true;
                return;
            }
            if element.kind() != "array_element_initializer" {
                continue;
            }
            let parts = children(element);
            if parts.len() != 2 || !text(element, source).contains("=>") {
                continue;
            }
            let Some((key, range)) = quoted_node(parts[0], source) else {
                continue;
            };
            let name = format!("{prefix}.{key}");
            self.add(name.clone(), target(path, source, range));
            if parts[1].kind() == "array_creation_expression" {
                self.arrays(parts[1], path, source, &name, depth + 1);
            }
        }
    }
    fn array_file(&mut self, path: &Path, prefix: &str) {
        let Some((path, source)) = self.read(path) else {
            return;
        };
        let Some(tree) = self.budget.tree(&source) else {
            self.truncated = true;
            return;
        };
        if tree.root_node().has_error() {
            return;
        }
        // Only an unconditional top-level returned literal is knowable statically.
        for node in children(tree.root_node()) {
            if node.kind() != "return_statement" {
                continue;
            }
            if let Some(array) = children(node)
                .into_iter()
                .find(|n| n.kind() == "array_creation_expression")
            {
                self.add(
                    prefix.into(),
                    target(&path, &source, array.start_byte()..array.start_byte()),
                );
                self.arrays(array, &path, &source, prefix, 0);
            }
        }
    }
    fn json_translations(&mut self, path: &Path) {
        let Some((path, source)) = self.read(path) else {
            return;
        };
        let Ok(serde_json::Value::Object(values)) = serde_json::from_str(&source) else {
            return;
        };
        let mut at = source.find('{').unwrap() + 1;
        loop {
            if self.budget.check().is_none() {
                self.truncated = true;
                return;
            }
            at += source[at..].len() - source[at..].trim_start().len();
            if source.as_bytes().get(at) != Some(&b'"') {
                break;
            }
            let start = at + 1;
            let mut end = start;
            while end < source.len() {
                if source.as_bytes()[end] == b'\\' {
                    end += 2;
                    continue;
                }
                if source.as_bytes()[end] == b'"' {
                    break;
                }
                end += 1;
            }
            let Ok(name) = serde_json::from_str::<String>(&source[at..end + 1]) else {
                break;
            };
            at = end + 1;
            at += source[at..].len() - source[at..].trim_start().len();
            if source.as_bytes().get(at) != Some(&b':') {
                break;
            }
            at += 1;
            let mut value =
                serde_json::Deserializer::from_str(&source[at..]).into_iter::<serde_json::Value>();
            if value.next().is_none_or(|v| v.is_err()) {
                break;
            }
            at += value.byte_offset();
            if values.get(&name).is_some_and(|v| v.is_string()) {
                self.add(name, target(&path, &source, start..end));
            }
            at += source[at..].len() - source[at..].trim_start().len();
            if source.as_bytes().get(at) != Some(&b',') {
                break;
            }
            at += 1;
        }
    }
    fn routes(&mut self, path: &Path) {
        let Some((path, source)) = self.read(path) else {
            return;
        };
        let Some(tree) = self.budget.tree(&source) else {
            self.truncated = true;
            return;
        };
        let Some(names) = Names::parse(&tree, &source) else {
            return;
        };
        let mut candidates = Vec::new();
        walk(tree.root_node(), &mut |node| {
            if let Some(resources) = resource_routes(node, &source, &names) {
                candidates.extend(resources);
            }
            if node.kind() != "member_call_expression"
                || node
                    .child_by_field_name("name")
                    .is_none_or(|n| text(n, &source) != "name")
            {
                return;
            }
            let Some(arg) = argument(node, 0) else {
                return;
            };
            let Some((name, range)) = quoted_node(arg, &source) else {
                return;
            };
            let mut object = node.child_by_field_name("object");
            while let Some(o) = object {
                if o.kind() == "scoped_call_expression" {
                    let method = o.child_by_field_name("name").map(|n| text(n, &source));
                    if facade(o, &source, &names, "Route")
                        && matches!(
                            method,
                            Some(
                                "get"
                                    | "post"
                                    | "put"
                                    | "patch"
                                    | "delete"
                                    | "options"
                                    | "any"
                                    | "match"
                                    | "view"
                                    | "redirect"
                                    | "permanentRedirect"
                            )
                        )
                        && let Some(prefix) = group_prefix(node, &source, &names)
                    {
                        candidates.push((format!("{prefix}{name}"), range));
                    }
                    break;
                }
                if o.kind() != "member_call_expression" {
                    break;
                }
                object = o.child_by_field_name("object");
            }
        });
        for (name, range) in candidates {
            self.add(name, target(&path, &source, range));
        }
    }
}
fn root(budget: &mut Budget<'_>, path: &Path) -> Option<PathBuf> {
    let project = crate::php_project::ComposerProject::discover(budget, path)?;
    project.is_laravel().then_some(project.root)
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
    let mut budget = Budget {
        root: workspace,
        buffers,
        cancel,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    let byte = crate::utf16_byte_offset(source, line, character)?;
    let blade = path.to_str()?.ends_with(".blade.php");
    let directive = blade.then(|| directive_range(source, byte)).flatten();
    let context = context(&budget, source, byte, blade);
    if directive.is_none() && context.is_none() {
        return None;
    }
    let project = root(&mut budget, path)?;
    if let Some(range) = directive {
        let (start_line, start_character) = position(source, range.start);
        let (end_line, end_character) = position(source, range.end);
        let items = crate::snippets::candidates(path, &source[range.start..byte])
            .map(|s| CompletionItem {
                label: s.trigger.into(),
                detail: Some(format!("Blade · {} · Tab: live template", s.description)),
                kind: Some(14),
                filter_text: None,
                sort_text: None,
                new_text: s.trigger.into(),
                text_edit: Some(CompletionEdit {
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                    new_text: s.trigger.into(),
                }),
                insert_edit: None,
                additional_edits: Vec::new(),
                unsupported_reason: None,
            })
            .collect();
        return Some(ResultSet {
            items,
            targets: Vec::new(),
            truncated: false,
        });
    }
    let context = context?;
    let mut inventory = Inventory {
        budget,
        entries: Vec::new(),
        visited: 0,
        truncated: false,
    };
    let mut files = Vec::new();
    match context.kind {
        Kind::View | Kind::Asset => {
            let base = project.join(if context.kind == Kind::View {
                "resources/views"
            } else {
                "public"
            });
            inventory.files(&base, 0, &mut files);
            for path in files {
                let Some(relative) = path.strip_prefix(&base).ok().and_then(Path::to_str) else {
                    continue;
                };
                let name = if context.kind == Kind::View {
                    let Some(stem) = relative
                        .strip_suffix(".blade.php")
                        .or_else(|| relative.strip_suffix(".php"))
                    else {
                        continue;
                    };
                    if stem.split('/').any(|p| p.contains('.')) {
                        continue;
                    }
                    stem.replace('/', ".")
                } else {
                    relative.into()
                };
                // No need to read an asset (possibly binary/private) for linking.
                if let Ok(path) = path.canonicalize()
                    && path.starts_with(workspace)
                {
                    inventory.add(name, target(&path, "", 0..0));
                }
            }
        }
        Kind::Config => {
            let base = project.join("config");
            inventory.files(&base, 0, &mut files);
            for path in files {
                if path.parent() != Some(base.as_path())
                    || path.extension().is_none_or(|x| x != "php")
                {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    inventory.array_file(&path, stem);
                }
            }
        }
        Kind::Translation => {
            for base in [project.join("lang"), project.join("resources/lang")] {
                files.clear();
                inventory.files(&base, 0, &mut files);
                for path in &files {
                    if path.extension().is_some_and(|x| x == "json")
                        && path.parent() == Some(base.as_path())
                    {
                        inventory.json_translations(path);
                        continue;
                    }
                    if path.extension().is_none_or(|x| x != "php") {
                        continue;
                    }
                    let Ok(relative) = path.strip_prefix(&base) else {
                        continue;
                    };
                    if relative.components().count() != 2 {
                        continue;
                    }
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        inventory.array_file(path, stem);
                    }
                }
            }
        }
        Kind::Route => {
            inventory.files(&project.join("routes"), 0, &mut files);
            for path in files {
                if path.extension().is_some_and(|x| x == "php") {
                    inventory.routes(&path);
                }
            }
        }
        Kind::Controller => {
            let (_, manifest) = inventory.read(&project.join("composer.json"))?;
            let manifest: serde_json::Value = serde_json::from_str(&manifest).ok()?;
            let mut maps = Vec::new();
            super::add_maps(&mut maps, &project, manifest.get("autoload")?)?;
            let mut class = context.controller?;
            let mut seen = std::collections::HashSet::new();
            let mut methods = std::collections::HashSet::new();
            for _ in 0..32 {
                if !seen.insert(class.clone()) {
                    break;
                }
                let Some((path, source)) = super::locate(&mut inventory.budget, &maps, &class)
                else {
                    break;
                };
                let Some(declaration) = super::class(&inventory.budget, &path, &source, &class)
                else {
                    break;
                };
                // Trait adaptations/ambiguous inherited declarations require the
                // semantic provider; direct class methods always win in PHP.
                for method in declaration.methods {
                    if !methods.insert(method.name.to_ascii_lowercase()) {
                        continue;
                    }
                    if method.public && !method.name.starts_with('_') {
                        inventory.add(method.name, method.target);
                    }
                }
                if !declaration.traits.is_empty() {
                    break;
                }
                let Some(parent) = declaration.parent else {
                    break;
                };
                class = parent;
            }
        }
    }
    if inventory.budget.check().is_none() {
        inventory.truncated = true;
    }
    inventory
        .entries
        .sort_by(|a, b| a.name.cmp(&b.name).then(a.target.path.cmp(&b.target.path)));
    let targets = inventory
        .entries
        .iter()
        .filter(|e| e.name == context.name)
        .take(64)
        .map(|e| e.target.clone())
        .collect();
    if inventory
        .entries
        .iter()
        .filter(|e| e.name == context.name)
        .count()
        > 64
    {
        inventory.truncated = true;
    }
    let mut items = Vec::new();
    if completion {
        let (start_line, start_character) = position(source, context.range.start);
        let (end_line, end_character) = position(source, context.range.end);
        for entry in inventory
            .entries
            .into_iter()
            .filter(|e| e.name.starts_with(&context.prefix))
        {
            if items.iter().any(|i: &CompletionItem| i.label == entry.name) {
                continue;
            }
            if items.len() == 128 {
                inventory.truncated = true;
                break;
            }
            items.push(CompletionItem {
                label: entry.name.clone(),
                detail: Some(format!(
                    "Laravel {:?} · {}",
                    context.kind,
                    entry
                        .target
                        .path
                        .strip_prefix(workspace)
                        .unwrap_or(&entry.target.path)
                        .display()
                )),
                kind: Some(
                    if context.kind == Kind::View || context.kind == Kind::Asset {
                        17
                    } else {
                        12
                    },
                ),
                filter_text: Some(entry.name.clone()),
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
        truncated: inventory.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn put(root: &Path, name: &str, value: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, value).unwrap();
        path
    }
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(
            root,
            "composer.json",
            r#"{"require":{"laravel/framework":"^12.0"}}"#,
        );
        put(root, "app/Example.php", "<?php");
        put(
            root,
            "resources/views/admin/users.blade.php",
            "<h1>🌧 Users</h1>",
        );
        put(
            root,
            "resources/views/layouts/app.blade.php",
            "<main>@yield('content')</main>",
        );
        put(
            root,
            "config/app.php",
            "<?php\r\n// 🌧\r\nreturn ['name' => 'App', 'services' => ['mail' => ['host' => env('MAIL_HOST')]], 'ignored' => function () { return ['secret' => 1]; }];",
        );
        put(
            root,
            "lang/en/messages.php",
            "<?php return ['welcome' => 'Welcome', 'users' => ['name' => 'Name']];",
        );
        put(
            root,
            "resources/lang/pt_BR/messages.php",
            "<?php return ['welcome' => 'Bem-vindo'];",
        );
        put(
            root,
            "routes/web.php",
            "<?php\nuse Illuminate\\Support\\Facades\\Route as R;\n// 🌧\nR::get('/users', fn () => 42)->middleware('auth')->name('admin.users');\nR::name('prefix.')->group(function () { R::get('/bad', fn()=>1)->name('hidden'); });",
        );
        put(root, "public/css/app.css", "body {}");
        dir
    }
    fn run(
        root: &Path,
        path: &str,
        marked: &str,
        buffers: &[(PathBuf, &str)],
        completion: bool,
    ) -> Option<ResultSet> {
        let byte = marked.find('|').unwrap();
        let source = marked.replacen('|', "", 1);
        let (line, ch) = position(&source, byte);
        query(
            root,
            &root.join(path),
            &source,
            line,
            ch,
            &AtomicBool::new(false),
            buffers,
            completion,
        )
    }
    fn names(result: ResultSet) -> Vec<String> {
        result.items.into_iter().map(|i| i.label).collect()
    }
    #[test]
    fn actual_views_configs_translations_routes_and_assets_are_completed() {
        let dir = fixture();
        let root = dir.path();
        for (code, expected) in [
            ("<?php view('admin.|');", "admin.users"),
            ("<?php config('app.services.ma|');", "app.services.mail"),
            ("<?php __('messages.w|');", "messages.welcome"),
            ("<?php trans_choice('messages.u|', 2);", "messages.users"),
            ("<?php route('admin.u|');", "admin.users"),
            ("<?php asset('css/|');", "css/app.css"),
        ] {
            let result = run(root, "app/Example.php", code, &[], true).unwrap();
            assert!(!result.truncated, "{code}");
            assert!(names(result).iter().any(|n| n == expected), "{code}");
        }
    }
    #[test]
    fn resource_routes_expand_static_actions_filters_names_and_group_prefix() {
        let dir = fixture();
        put(
            dir.path(),
            "routes/resources.php",
            "<?php use Illuminate\\Support\\Facades\\Route as R; R::resource('photos', C::class)->only(['index','show'])->names(['show'=>'photos.inspect']); R::apiResource('teams.members', C::class)->except('destroy'); R::name('admin.')->group(function() { R::resource('posts', C::class)->only('index')->name('index','post-list'); }); R::resource('unknown', C::class)->only($dynamic); R::resource('shallow', C::class)->shallow();",
        );
        let result = run(
            dir.path(),
            "app/Example.php",
            "<?php route('|');",
            &[],
            true,
        )
        .unwrap();
        assert!(!result.truncated);
        let labels = names(result);
        for expected in [
            "photos.index",
            "photos.inspect",
            "teams.members.index",
            "teams.members.show",
            "admin.post-list",
        ] {
            assert!(
                labels.iter().any(|n| n == expected),
                "{expected}: {labels:?}"
            );
        }
        for absent in [
            "photos.create",
            "photos.show",
            "teams.members.create",
            "teams.members.destroy",
            "unknown.index",
            "shallow.index",
        ] {
            assert!(!labels.iter().any(|n| n == absent), "{absent}: {labels:?}");
        }
        let definition = run(
            dir.path(),
            "app/Example.php",
            "<?php route('photos.in|dex');",
            &[],
            false,
        )
        .unwrap();
        assert_eq!(definition.targets.len(), 1);
        let target = &definition.targets[0];
        let source = fs::read_to_string(&target.path).unwrap();
        let a =
            crate::utf16_byte_offset(&source, target.start_line, target.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, target.end_line, target.end_character).unwrap();
        assert_eq!(&source[a..b], "photos");
    }
    #[test]
    fn partial_unclosed_literals_and_empty_prefix_are_supported() {
        let dir = fixture();
        for code in ["<?php view('admin.|", "<?php view('|');"] {
            assert!(
                names(run(dir.path(), "app/Example.php", code, &[], true).unwrap())
                    .contains(&"admin.users".into())
            );
        }
    }
    #[test]
    fn aliases_and_fully_qualified_facades_are_resolved_without_guessing_short_names() {
        let dir = fixture();
        for code in [
            "<?php use Illuminate\\Support\\Facades\\View as V; V::make('admin.|');",
            "<?php \\Illuminate\\Support\\Facades\\View::exists('admin.|');",
        ] {
            assert_eq!(
                names(run(dir.path(), "app/Example.php", code, &[], true).unwrap()),
                ["admin.users"]
            );
        }
        for code in [
            "<?php View::make('admin.|');",
            "<?php use Other\\View; View::make('admin.|');",
        ] {
            assert!(run(dir.path(), "app/Example.php", code, &[], true).is_none());
        }
    }
    #[test]
    fn blade_directives_echo_php_and_comments_have_distinct_contexts() {
        let dir = fixture();
        for code in [
            "@extends('layouts.|')",
            "@include('admin.|')",
            "@includeIf('admin.|')",
            "<p>{{ view('admin.|') }}</p>",
            "{!! view('admin.|') !!}",
            "@php $x = view('admin.|'); @endphp",
            "<?php view('admin.|'); ?>",
            "@if (true)\n@include('admin.|')\n@endif",
        ] {
            assert!(
                !run(
                    dir.path(),
                    "resources/views/test.blade.php",
                    code,
                    &[],
                    true
                )
                .unwrap()
                .items
                .is_empty(),
                "{code}"
            );
        }
        for code in [
            "{{-- @include('admin.|') --}}",
            "<!-- {{ view('admin.|') }} -->",
            "<p>view('admin.|')</p>",
            "@@include('admin.|')",
        ] {
            assert!(
                run(
                    dir.path(),
                    "resources/views/test.blade.php",
                    code,
                    &[],
                    true
                )
                .is_none(),
                "{code}"
            );
        }
    }
    #[test]
    fn strings_comments_interpolation_second_arguments_and_unrelated_calls_are_rejected() {
        let dir = fixture();
        for code in [
            "<?php // view('admin.|')",
            "<?php /* view('admin.|') */",
            "<?php $s = \"view('admin.|')\";",
            "<?php view($name, 'admin.|');",
            "<?php $x->view('admin.|');",
            "<?php other('admin.|');",
            "<?php view(\"$prefix.admin.|\");",
            "<?php view('admin.\\u|');",
            "<?php view(name: 'admin.|');",
        ] {
            assert!(
                run(dir.path(), "app/Example.php", code, &[], true).is_none(),
                "{code}"
            );
        }
    }
    #[test]
    fn replacement_covers_entire_literal_with_suffix_unicode_and_crlf() {
        let dir = fixture();
        let code = "<?php\r\n// 🌧\r\necho '🌧'; view('admin.u|nfinished');";
        let item = run(dir.path(), "app/Example.php", code, &[], true)
            .unwrap()
            .items
            .remove(0);
        let edit = item.text_edit.unwrap();
        let source = code.replace('|', "");
        let a = crate::utf16_byte_offset(&source, edit.start_line, edit.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, edit.end_line, edit.end_character).unwrap();
        assert_eq!(&source[a..b], "admin.unfinished");
        assert_eq!(
            format!("{}{}{}", &source[..a], edit.new_text, &source[b..]),
            source.replace("admin.unfinished", "admin.users")
        );
    }
    #[test]
    fn definition_points_to_exact_keys_routes_and_all_locales() {
        let dir = fixture();
        let root = dir.path();
        for (code, needle, expected_count) in [
            ("<?php config('app.services.mail.h|ost');", "host", 1),
            ("<?php route('admin.us|ers');", "admin.users", 1),
            ("<?php __('messages.wel|come');", "welcome", 2),
        ] {
            let result = run(root, "app/Example.php", code, &[], false).unwrap();
            assert_eq!(result.targets.len(), expected_count, "{code}");
            for t in result.targets {
                let source = fs::read_to_string(t.path).unwrap();
                let a = crate::utf16_byte_offset(&source, t.start_line, t.start_character).unwrap();
                let b = crate::utf16_byte_offset(&source, t.end_line, t.end_character).unwrap();
                assert_eq!(&source[a..b], needle);
            }
        }
        assert!(
            run(
                root,
                "app/Example.php",
                "<?php route('hid|den');",
                &[],
                false
            )
            .unwrap()
            .targets
            .is_empty()
        );
    }
    #[test]
    fn unsaved_config_route_and_composer_snapshots_override_disk() {
        let dir = fixture();
        let root = dir.path();
        let config = root.join("config/app.php").canonicalize().unwrap();
        assert_eq!(
            names(
                run(
                    root,
                    "app/Example.php",
                    "<?php config('app.|');",
                    &[(config, "<?php return ['dirty' => 1];")],
                    true
                )
                .unwrap()
            ),
            ["app.dirty"]
        );
        let routes = root.join("routes/web.php").canonicalize().unwrap();
        assert_eq!(names(run(root,"app/Example.php","<?php route('|');",&[(routes,"<?php use Illuminate\\Support\\Facades\\Route; Route::get('/',fn()=>1)->name('unsaved');")],true).unwrap()),["unsaved"]);
        let composer = root.join("composer.json").canonicalize().unwrap();
        assert!(
            run(
                root,
                "app/Example.php",
                "<?php view('|');",
                &[(composer, "{}")],
                true
            )
            .is_none()
        );
        assert!(!root.join("HOOK_EXECUTED").exists());
    }
    #[test]
    fn nearest_composer_root_keeps_microservices_separate() {
        let dir = fixture();
        let root = dir.path();
        put(
            root,
            "nested/service/composer.json",
            r#"{"require":{"laravel/framework":"^6.2"}}"#,
        );
        put(
            root,
            "nested/service/config/app.php",
            "<?php return ['service' => 1];",
        );
        assert_eq!(
            names(
                run(
                    root,
                    "nested/service/app/Example.php",
                    "<?php config('app.|');",
                    &[],
                    true
                )
                .unwrap()
            ),
            ["app.service"]
        );
        put(root, "nested/service/composer.json", "{}");
        assert!(
            run(
                root,
                "nested/service/app/Example.php",
                "<?php config('app.|');",
                &[],
                true
            )
            .is_none()
        );
    }
    #[test]
    fn cancel_and_inventory_bounds_are_visible_without_unbounded_walk() {
        let dir = fixture();
        let root = dir.path();
        let result = query(
            root,
            &root.join("app/Example.php"),
            "<?php view('a');",
            0,
            13,
            &AtomicBool::new(true),
            &[],
            true,
        );
        assert!(result.is_none());
        for n in 0..140 {
            put(
                root,
                &format!("resources/views/generated/{n:03}.blade.php"),
                "",
            );
        }
        let result = run(
            root,
            "app/Example.php",
            "<?php view('generated.|');",
            &[],
            true,
        )
        .unwrap();
        assert_eq!(result.items.len(), 128);
        assert!(result.truncated);
    }

    #[test]
    fn controller_action_strings_use_declared_class_public_methods_and_dirty_buffers() {
        let dir = fixture();
        let root = dir.path();
        put(
            root,
            "composer.json",
            r#"{"require":{"laravel/framework":"^12.0"},"autoload":{"psr-4":{"App\\":"app/"}}}"#,
        );
        let controller=put(root,"app/Http/Controllers/ExampleController.php","<?php namespace App\\Http\\Controllers; class ExampleController { public function index() {} protected function hidden() {} }").canonicalize().unwrap();
        let head = "<?php use Illuminate\\Support\\Facades\\Route as R; use App\\Http\\Controllers\\ExampleController as C; ";
        for tail in [
            "R::get('/', [C::class, 'ind|']);",
            "R::get('/', [C::class, 'ind|",
            "R::match(['get'], '/', [C::class, 'ind|']);",
        ] {
            let code = format!("{head}{tail}");
            let result = run(root, "routes/web.php", &code, &[], true).unwrap();
            assert_eq!(names(result), ["index"], "{code}");
        }
        let code = format!("{head}R::get('/', [C::class, 'in|dex']);");
        let result = run(root, "routes/web.php", &code, &[], false).unwrap();
        assert_eq!(result.targets.len(), 1);
        assert_eq!(result.targets[0].path, controller);
        let dirty = "<?php namespace App\\Http\\Controllers; class ExampleController { public function updated() {} }";
        let code = format!("{head}R::get('/', [C::class, '|']);");
        assert_eq!(
            names(
                run(
                    root,
                    "routes/web.php",
                    &code,
                    &[(controller.clone(), dirty)],
                    true
                )
                .unwrap()
            ),
            ["updated"]
        );
        assert!(fs::read_to_string(controller).unwrap().contains("index"));
        for tail in [
            "R::get('/', [$controller, 'ind|']);",
            "Other::get('/', [C::class, 'ind|']);",
            "$list = [C::class, 'ind|'];",
        ] {
            assert!(
                run(root, "routes/web.php", &format!("{head}{tail}"), &[], true).is_none(),
                "{tail}"
            );
        }
    }
    #[test]
    fn grouped_routes_resolve_nested_literal_prefixes_but_reject_dynamic_names() {
        let dir = fixture();
        put(
            dir.path(),
            "routes/groups.php",
            "<?php use Illuminate\\Support\\Facades\\Route; Route::name('admin.')->middleware('auth')->group(function () { Route::group(['as' => 'users.'], function () { Route::get('/', fn()=>1)->name('index'); }); }); Route::name($dynamic)->group(function () { Route::get('/', fn()=>1)->name('unresolved'); });",
        );
        assert_eq!(
            names(
                run(
                    dir.path(),
                    "app/Example.php",
                    "<?php route('admin.users.|');",
                    &[],
                    true
                )
                .unwrap()
            ),
            ["admin.users.index"]
        );
        assert!(
            run(
                dir.path(),
                "app/Example.php",
                "<?php route('unre|solved');",
                &[],
                false
            )
            .unwrap()
            .targets
            .is_empty()
        );
    }
    #[test]
    fn json_translation_keys_are_completed_and_navigate_to_source_key() {
        let dir = fixture();
        put(
            dir.path(),
            "lang/pt_BR.json",
            r#"{"Hello world": "Olá mundo", "Weather 🌧": "Previsão", "unsupported": {"child": 1}}"#,
        );
        assert_eq!(
            names(
                run(
                    dir.path(),
                    "app/Example.php",
                    "<?php __('Hello |');",
                    &[],
                    true
                )
                .unwrap()
            ),
            ["Hello world"]
        );
        let result = run(
            dir.path(),
            "resources/views/test.blade.php",
            "{{ __('Weather |🌧') }}",
            &[],
            false,
        )
        .unwrap();
        assert_eq!(result.targets.len(), 1);
        let t = &result.targets[0];
        let source = fs::read_to_string(&t.path).unwrap();
        let a = crate::utf16_byte_offset(&source, t.start_line, t.start_character).unwrap();
        let b = crate::utf16_byte_offset(&source, t.end_line, t.end_character).unwrap();
        assert_eq!(&source[a..b], "Weather 🌧");
        assert!(
            names(
                run(
                    dir.path(),
                    "app/Example.php",
                    "<?php __('unsupp|');",
                    &[],
                    true
                )
                .unwrap()
            )
            .is_empty()
        );
    }
    #[test]
    fn blade_directives_complete_names_and_multiline_view_arguments() {
        let dir = fixture();
        for code in ["@fore|", "@fore|ach($items as $item)"] {
            let result = run(
                dir.path(),
                "resources/views/test.blade.php",
                code,
                &[],
                true,
            )
            .unwrap();
            assert!(names(result).contains(&"@foreach".into()), "{code}");
        }
        for code in [
            "@include(\n    'admin.|'\n)",
            "@if (true)\n{{ view('admin.|') }}\n@endif",
        ] {
            assert_eq!(
                names(
                    run(
                        dir.path(),
                        "resources/views/test.blade.php",
                        code,
                        &[],
                        true
                    )
                    .unwrap()
                ),
                ["admin.users"]
            );
        }
        for code in [
            "{{-- @fore| --}}",
            "<!-- @fore| -->",
            "@verbatim @include('admin.|') @endverbatim",
            "{{ '@fore|' }}",
        ] {
            assert!(
                run(
                    dir.path(),
                    "resources/views/test.blade.php",
                    code,
                    &[],
                    true
                )
                .is_none(),
                "{code}"
            );
        }
    }

    #[test]
    #[ignore = "explicitly authorized real projects, static reads only; CODISSEU_LARAVEL_READONLY_ROOT"]
    fn real_microservices_are_read_only_and_names_stay_inside_each_project() {
        let root = PathBuf::from(
            std::env::var_os("CODISSEU_LARAVEL_READONLY_ROOT").expect("explicit root required"),
        )
        .canonicalize()
        .unwrap();
        let mut projects = 0;
        let mut config_projects = 0;
        let mut completions = 0;
        let mut versions = std::collections::BTreeSet::new();
        for version in fs::read_dir(&root).unwrap().filter_map(Result::ok) {
            if !version.file_type().unwrap().is_dir() {
                continue;
            }
            for project in fs::read_dir(version.path()).unwrap().filter_map(Result::ok) {
                if !project.file_type().unwrap().is_dir() {
                    continue;
                }
                let path = project.path();
                let manifest = path.join("composer.json");
                if !manifest.is_file() {
                    continue;
                }
                let before = fs::read(&manifest).unwrap();
                let data: serde_json::Value = serde_json::from_slice(&before).unwrap();
                let Some(version) = data["require"]["laravel/framework"].as_str() else {
                    continue;
                };
                versions.insert(version.to_string());
                projects += 1;
                let result = run(
                    &root,
                    path.join("app/CodisseuReadonlyProbe.php").to_str().unwrap(),
                    "<?php config('|');",
                    &[],
                    true,
                )
                .expect("Laravel detected from nearest project");
                if !result.items.is_empty() {
                    config_projects += 1;
                }
                completions += result.items.len();
                for item in result.items {
                    let relative = path.strip_prefix(&root).unwrap();
                    assert!(
                        item.detail
                            .unwrap()
                            .contains(&relative.to_string_lossy().to_string())
                    );
                }
                assert_eq!(fs::read(&manifest).unwrap(), before);
            }
        }
        assert!(projects > 0);
        assert!(config_projects > 0);
        eprintln!(
            "readonly projects={projects}, config_projects={config_projects}, suggestions={completions}, Laravel requirements={versions:?}; no scripts/providers/network/writes invoked"
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_resources_cannot_escape_workspace() {
        use std::os::unix::fs::symlink;
        let dir = fixture();
        let other = tempfile::tempdir().unwrap();
        put(other.path(), "secret.blade.php", "secret");
        symlink(other.path(), dir.path().join("resources/views/external")).unwrap();
        symlink(
            other.path().join("secret.blade.php"),
            dir.path().join("resources/views/leak.blade.php"),
        )
        .unwrap();
        assert_eq!(
            names(run(dir.path(), "app/Example.php", "<?php view('|');", &[], true).unwrap()),
            ["admin.users", "layouts.app"]
        );
    }
}
