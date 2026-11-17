//! Conservative, source-derived Eloquent builder navigation. No application boot,
//! autoloader execution, database access, generated stubs or hardcoded method list.
use crate::php_project::{Budget, Map, Names, add_maps, children, locate, text};
use crate::{CompletionItem, DefinitionTarget};
use serde_json::Value;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Instant,
};
use tree_sitter::Node;
pub(crate) mod components;
pub(crate) mod eloquent_static;
pub(crate) mod requests;
pub(crate) mod resources;

const MODEL: &str = "Illuminate\\Database\\Eloquent\\Model";
const BUILDER: &str = "Illuminate\\Database\\Eloquent\\Builder";
const QUERY: &str = "Illuminate\\Database\\Query\\Builder";
const HOOKS: &[&str] = &[
    "__call",
    "__callStatic",
    "query",
    "newQuery",
    "newModelQuery",
    "newQueryWithoutScopes",
    "newEloquentBuilder",
    "newBaseQueryBuilder",
    "resolveCustomBuilderClass",
    "getConnection",
    "resolveConnection",
];
struct QueryContext {
    model: String,
    method: String,
}
fn query_context(
    budget: &Budget<'_>,
    source: &str,
    byte: usize,
    completion: bool,
) -> Option<QueryContext> {
    // Make the unfinished method call parseable without changing the user's text.
    let parsed = if completion {
        format!("{}();", source.get(..byte)?)
    } else {
        source.into()
    };
    let tree = budget.tree(&parsed)?;
    let names = Names::parse(&tree, &parsed)?;
    let at = if completion {
        byte.checked_sub(1)?
    } else {
        byte
    };
    let mut node = tree.root_node().descendant_for_byte_range(at, at)?;
    loop {
        if matches!(
            node.kind(),
            "scoped_call_expression" | "member_call_expression"
        ) {
            let name = node.child_by_field_name("name")?;
            if name.start_byte() <= at && at <= name.end_byte() && !name.has_error() {
                let method = text(name, &parsed).to_string();
                let scope = if node.kind() == "scoped_call_expression" {
                    node.child_by_field_name("scope")?
                } else {
                    let object = node.child_by_field_name("object")?;
                    if object.kind() != "scoped_call_expression"
                        || text(object.child_by_field_name("name")?, &parsed) != "query"
                    {
                        return None;
                    }
                    let arguments = object.child_by_field_name("arguments")?;
                    if !children(arguments).is_empty() {
                        return None;
                    }
                    object.child_by_field_name("scope")?
                };
                if !matches!(scope.kind(), "name" | "qualified_name") {
                    return None;
                }
                return Some(QueryContext {
                    model: names.resolve(text(scope, &parsed))?,
                    method,
                });
            }
        }
        node = node.parent()?;
    }
}
fn maps(budget: &mut Budget<'_>) -> Option<Vec<Map>> {
    let (_, manifest) = budget.read(&budget.root.join("composer.json"))?;
    let manifest: Value = serde_json::from_str(&manifest).ok()?;
    let mut maps = Vec::new();
    add_maps(&mut maps, budget.root, manifest.get("autoload")?)?;
    if let Some(autoload) = manifest.get("autoload-dev") {
        add_maps(&mut maps, budget.root, autoload)?;
    }
    let (_, installed) = budget.read(&budget.root.join("vendor/composer/installed.json"))?;
    let installed: Value = serde_json::from_str(&installed).ok()?;
    let packages = installed.get("packages").unwrap_or(&installed).as_array()?;
    if !packages
        .iter()
        .any(|p| p["name"] == "laravel/framework" || p["name"] == "illuminate/database")
    {
        return None;
    }
    if packages.len() > 1024 {
        return None;
    }
    for package in packages {
        let Some(name) = package["name"].as_str() else {
            continue;
        };
        if name.split('/').count() != 2 || name.contains("..") {
            continue;
        }
        let base = if let Some(path) = package["install-path"].as_str() {
            budget.root.join("vendor/composer").join(path)
        } else {
            budget.root.join("vendor").join(name)
        };
        if let Some(autoload) = package.get("autoload") {
            add_maps(&mut maps, &base, autoload)?;
        }
    }
    maps.sort_by_key(|m| std::cmp::Reverse(m.prefix.len()));
    Some(maps)
}
struct Class {
    methods: Vec<Method>,
    parent: Option<String>,
    traits: Vec<String>,
    unsafe_builder: bool,
}
#[derive(Clone)]
struct Method {
    name: String,
    public: bool,
    target: DefinitionTarget,
    calls: HashSet<String>,
}
fn methods(node: Node<'_>, source: &str, path: &Path) -> Vec<Method> {
    let Some(body) = node.child_by_field_name("body") else {
        return Vec::new();
    };
    children(body)
        .into_iter()
        .filter(|n| n.kind() == "method_declaration")
        .filter_map(|m| {
            let name = m.child_by_field_name("name")?;
            let mods = children(m);
            let public = !mods
                .iter()
                .any(|n| n.kind() == "visibility_modifier" && text(*n, source) != "public");
            let pos = |node: Node<'_>, end: bool| {
                let byte = if end {
                    node.end_byte()
                } else {
                    node.start_byte()
                };
                let row = if end {
                    node.end_position().row
                } else {
                    node.start_position().row
                };
                let start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
                (
                    row as u32,
                    source[start..byte].encode_utf16().count() as u32,
                )
            };
            let (start_line, start_character) = pos(name, false);
            let (end_line, end_character) = pos(name, true);
            let mut calls = HashSet::new();
            if matches!(text(name, source), "__call" | "query") {
                let mut stack = vec![m];
                while let Some(n) = stack.pop() {
                    if matches!(
                        n.kind(),
                        "member_call_expression" | "scoped_call_expression"
                    ) && let Some(name) = n.child_by_field_name("name")
                    {
                        calls.insert(text(name, source).into());
                    }
                    stack.extend(children(n));
                }
            }
            Some(Method {
                name: text(name, source).into(),
                public,
                target: DefinitionTarget {
                    path: path.into(),
                    start_line,
                    start_character,
                    end_line,
                    end_character,
                },
                calls,
            })
        })
        .collect()
}
fn class(budget: &Budget<'_>, path: &Path, source: &str, fqcn: &str) -> Option<Class> {
    let tree = budget.tree(source)?;
    let names = Names::parse(&tree, source)?;
    let node = children(tree.root_node()).into_iter().find(|n| {
        matches!(n.kind(), "class_declaration" | "trait_declaration")
            && n.child_by_field_name("name")
                .is_some_and(|v| names.resolve(text(v, source)).as_deref() == Some(fqcn))
    })?;
    if node.has_error() {
        return None;
    }
    let body = node.child_by_field_name("body")?;
    let mut parent = None;
    if let Some(base) = children(node)
        .into_iter()
        .find(|n| n.kind() == "base_clause")
    {
        let bases = children(base);
        if bases.len() != 1 {
            return None;
        }
        parent = Some(names.resolve(text(bases[0], source))?);
    }
    let mut traits = Vec::new();
    let mut unsafe_builder = children(node).iter().any(|n| n.kind() == "attribute_list");
    for child in children(body) {
        if child.kind() == "use_declaration" {
            for t in children(child) {
                if !matches!(t.kind(), "name" | "qualified_name") {
                    // Only direct builder methods are offered; trait adaptations
                    // in framework builder files are outside this candidate set.
                    if matches!(fqcn, MODEL | BUILDER | QUERY) {
                        continue;
                    }
                    return None;
                }
                traits.push(names.resolve(text(t, source))?);
            }
        }
        if child.kind() == "property_declaration"
            && children(child).iter().any(|p| {
                p.kind() == "property_element"
                    && p.child_by_field_name("name")
                        .is_some_and(|n| text(n, source) == "$builder")
            })
        {
            unsafe_builder = true;
        }
    }
    Some(Class {
        methods: methods(node, source, path),
        parent,
        traits,
        unsafe_builder,
    })
}
fn verified_model(
    budget: &mut Budget<'_>,
    maps: &[Map],
    model: &str,
    _method: &str,
) -> Option<HashSet<String>> {
    let mut blocked = HashSet::new();
    let mut queue = vec![(model.to_string(), false)];
    let mut seen = HashSet::new();
    let mut reached_model = false;
    while let Some((name, is_trait)) = queue.pop() {
        budget.check()?;
        if !seen.insert(name.clone()) {
            continue;
        }
        if seen.len() > 32 {
            return None;
        }
        if name == MODEL {
            reached_model = true;
            continue;
        }
        let (path, source) = locate(budget, maps, &name)?;
        let c = class(budget, &path, &source, &name)?;
        if c.unsafe_builder
            || c.methods
                .iter()
                .any(|m| HOOKS.iter().any(|hook| m.name.eq_ignore_ascii_case(hook)))
        {
            return None;
        }
        for m in &c.methods {
            blocked.insert(m.name.to_ascii_lowercase());
            if let Some(scope) = m.name.to_ascii_lowercase().strip_prefix("scope") {
                blocked.insert(scope.into());
            }
        }
        if !is_trait {
            let parent = c.parent?;
            queue.push((parent, false));
        }
        queue.extend(c.traits.into_iter().map(|t| (t, true)));
    }
    reached_model.then_some(blocked)
}
fn resolve(
    root: &Path,
    source: &str,
    line: u32,
    character: u32,
    completion: bool,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
) -> Option<(QueryContext, Vec<Method>)> {
    if source.len() > 1024 * 1024 {
        return None;
    }
    let mut budget = Budget {
        root,
        cancel,
        buffers,
        start: Instant::now(),
        bytes: 0,
        files: 0,
    };
    let byte = crate::utf16_byte_offset(source, line, character)?;
    let query = query_context(&budget, source, byte, completion)?;
    let maps = maps(&mut budget)?;
    let blocked = verified_model(&mut budget, &maps, &query.model, &query.method)?;
    let mut result = Vec::new();
    for fqcn in [MODEL, BUILDER, QUERY] {
        let (path, source) = locate(&mut budget, &maps, fqcn)?;
        let c = class(&budget, &path, &source, fqcn)?;
        if fqcn == MODEL {
            // Verify the standard dynamic Model→EloquentBuilder pipeline from
            // declarations in this installation, rather than assuming by name.
            if !c.methods.iter().any(|m| {
                m.name == "__call"
                    && m.calls.contains("forwardCallTo")
                    && m.calls.contains("newQuery")
            }) || !c
                .methods
                .iter()
                .any(|m| m.name == "query" && m.calls.contains("newQuery"))
            {
                return None;
            }
        } else {
            if fqcn == BUILDER
                && !c
                    .methods
                    .iter()
                    .any(|m| m.name == "__call" && m.calls.contains("forwardCallTo"))
            {
                return None;
            }
            for method in c.methods {
                if method.public
                    && !blocked.contains(&method.name.to_ascii_lowercase())
                    && !method.name.starts_with('_')
                    && !result.iter().any(|m: &Method| m.name == method.name)
                {
                    result.push(method);
                }
            }
        }
    }
    budget.check()?;
    Some((query, result))
}
pub(crate) fn completions(
    root: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
) -> Vec<CompletionItem> {
    let Some((query, methods)) = resolve(root, source, line, character, true, cancel, buffers)
    else {
        return Vec::new();
    };
    methods
        .into_iter()
        .filter(|m| m.name.starts_with(&query.method))
        .take(128)
        .map(|m| CompletionItem {
            label: m.name.clone(),
            detail: Some(format!(
                "Eloquent builder · inferred from {}",
                m.target
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&m.target.path)
                    .display()
            )),
            kind: Some(2),
            filter_text: None,
            sort_text: None,
            new_text: m.name,
            text_edit: None,
            insert_edit: None,
            additional_edits: Vec::new(),
            unsupported_reason: None,
        })
        .collect()
}
pub(crate) fn definitions(
    root: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
) -> Vec<DefinitionTarget> {
    let Some((query, methods)) = resolve(root, source, line, character, false, cancel, buffers)
    else {
        return Vec::new();
    };
    methods
        .into_iter()
        .filter(|m| m.name == query.method)
        .take(1)
        .map(|m| m.target)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn query_receiver_is_ast_resolved_and_strings_comments_are_not_queries() {
        let cancel = AtomicBool::new(false);
        let root = Path::new("/");
        let budget = Budget {
            root,
            cancel: &cancel,
            buffers: &[],
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        for tail in ["User::whe", "User::query()->whe"] {
            let text = format!("<?php use App\\Models\\User; {tail}");
            let q = query_context(&budget, &text, text.len(), true).expect(tail);
            assert_eq!(q.model, "App\\Models\\User");
            assert_eq!(q.method, "whe");
        }
        for text in [
            "<?php // User::whe",
            "<?php $x = 'User::whe",
            "<?php $user->whe",
            "<?php User::custom()->whe",
            "<?php self::whe",
        ] {
            assert!(
                query_context(&budget, text, text.len(), true).is_none(),
                "{text}"
            );
        }
    }
    #[test]
    fn cancelled_and_outside_roots_produce_no_invented_results() {
        let cancel = AtomicBool::new(true);
        assert!(completions(Path::new("/"), "<?php User::whe", 0, 15, &cancel, &[]).is_empty());
        let dir = tempfile::tempdir().unwrap();
        let mut budget = Budget {
            root: dir.path(),
            cancel: &cancel,
            buffers: &[],
            start: Instant::now(),
            bytes: 0,
            files: 0,
        };
        assert!(budget.read(Path::new("/etc/passwd")).is_none());
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (path, text) in [
            (
                "composer.json",
                r#"{"autoload":{"psr-4":{"App\\":"app/"}}}"#,
            ),
            (
                "vendor/composer/installed.json",
                r#"{"packages":[{"name":"laravel/framework","autoload":{"psr-4":{"Illuminate\\":"src/Illuminate/"}},"install-path":"../laravel/framework"}]}"#,
            ),
            (
                "app/Models/User.php",
                "<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; class User extends Model {}",
            ),
            (
                "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Model.php",
                "<?php namespace Illuminate\\Database\\Eloquent; class Model { public static function query() { return (new static)->newQuery(); } public function __call($method,$args) { return $this->forwardCallTo($this->newQuery(),$method,$args); } }",
            ),
            (
                "vendor/laravel/framework/src/Illuminate/Database/Eloquent/Builder.php",
                "<?php namespace Illuminate\\Database\\Eloquent; class Builder { public function where() {} public function __call($method,$args) { return $this->forwardCallTo($this->query,$method,$args); } protected function hidden() {} }",
            ),
            (
                "vendor/laravel/framework/src/Illuminate/Database/Query/Builder.php",
                "<?php namespace Illuminate\\Database\\Query; class Builder { public function whereDate() {} public function brandNewDeclaredMethod() {} }",
            ),
        ] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }
    fn query(root: &Path, tail: &str, buffers: &[(PathBuf, &str)]) -> Vec<CompletionItem> {
        let source = format!("<?php use App\\Models\\User as Person; {tail}");
        completions(
            root,
            &source,
            0,
            source.encode_utf16().count() as u32,
            &AtomicBool::new(false),
            buffers,
        )
    }
    #[test]
    fn declared_builder_methods_are_discovered_not_hardcoded() {
        let fixture = fixture();
        let root = fixture.path();
        let items = query(root, "Person::whe", &[]);
        assert!(items.iter().any(|i| i.label == "where"));
        assert!(items.iter().any(|i| i.label == "whereDate"));
        assert!(
            items
                .iter()
                .all(|i| i.detail.as_ref().unwrap().contains("inferred"))
        );
        assert_eq!(
            query(root, "Person::brandNew", &[])[0].label,
            "brandNewDeclaredMethod"
        );
        assert!(query(root, "Person::hidden", &[]).is_empty());
        let source = "<?php use App\\Models\\User; User::where('x',1);";
        let targets = definitions(
            root,
            source,
            0,
            source.find("where").unwrap() as u32 + 1,
            &AtomicBool::new(false),
            &[],
        );
        assert_eq!(targets.len(), 1);
        let target = std::fs::read_to_string(&targets[0].path).unwrap();
        let a =
            crate::utf16_byte_offset(&target, targets[0].start_line, targets[0].start_character)
                .unwrap();
        let b = crate::utf16_byte_offset(&target, targets[0].end_line, targets[0].end_character)
            .unwrap();
        assert_eq!(&target[a..b], "where");
    }
    #[test]
    fn custom_builder_hooks_attributes_missing_traits_and_unrelated_classes_are_rejected() {
        let fixture = fixture();
        let root = fixture.path();
        for body in [
            "protected static $builder = Custom::class;",
            "public function newEloquentBuilder($q) { return new Custom($q); }",
            "public static function query() { return null; }",
            "public function __call($name,$args) { return null; }",
            "use MissingTrait;",
        ] {
            std::fs::write(root.join("app/Models/User.php"),format!("<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; class User extends Model {{ {body} }}")).unwrap();
            assert!(query(root, "Person::whe", &[]).is_empty(), "{body}");
        }
        for source in [
            "<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; #[CustomBuilder] class User extends Model {}",
            "<?php namespace App\\Models; class User {}",
        ] {
            std::fs::write(root.join("app/Models/User.php"), source).unwrap();
            assert!(query(root, "Person::whe", &[]).is_empty());
        }
    }
    #[test]
    fn local_scopes_and_dirty_custom_builders_do_not_invent_targets() {
        let fixture = fixture();
        let root = fixture.path();
        let path = root.join("app/Models/User.php").canonicalize().unwrap();
        let source = "<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; class User extends Model { public function scopeWhere($q) {} }";
        let items = query(root, "Person::whe", &[(path.clone(), source)]);
        assert!(!items.iter().any(|i| i.label == "where"));
        let dirty = "<?php namespace App\\Models; use Illuminate\\Database\\Eloquent\\Model; class User extends Model { public function newQuery() {return null;} }";
        assert!(query(root, "Person::whe", &[(path, dirty)]).is_empty());
        assert!(
            query(root, "Person::whe", &[])
                .iter()
                .any(|i| i.label == "where"),
            "disk not changed by the overlay"
        );
    }
    #[cfg(unix)]
    #[test]
    fn external_symlink_is_never_read_even_with_valid_composer_metadata() {
        let fixture = fixture();
        let outside = tempfile::tempdir().unwrap();
        let model = fixture.path().join("app/Models/User.php");
        std::fs::copy(&model, outside.path().join("User.php")).unwrap();
        std::fs::remove_file(&model).unwrap();
        std::os::unix::fs::symlink(outside.path().join("User.php"), &model).unwrap();
        assert!(query(fixture.path(), "Person::whe", &[]).is_empty());
    }
}
