//! Bounded PHP declaration anchors. Syntax identifies an action's location;
//! semantic providers, not these anchors, resolve references or implementations.
use std::{
    ops::{ControlFlow, Range},
    sync::Arc,
    time::{Duration, Instant},
};
use tree_sitter::{InputEdit, Node, ParseOptions, Parser, Point, Tree};
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_SYMBOLS: usize = 2048;
const MAX_NODES: usize = 100_000;
const DEADLINE: Duration = Duration::from_millis(150);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationKind {
    Class,
    Interface,
    Trait,
    Function,
    Method,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationParent {
    pub name: String,
    pub line: usize,
    pub scalar_range: Range<usize>,
    pub byte_range: Range<usize>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationAnchor {
    pub name: String,
    pub kind: NavigationKind,
    /// Zero-based logical source line (LF/CRLF/CR).
    pub line: usize,
    pub scalar_range: Range<usize>,
    pub byte_range: Range<usize>,
    pub container_name: Option<String>,
    /// A search affordance, not a claim that an implementation exists.
    pub can_find_implementations: bool,
    pub has_parent_relations: bool,
    /// Exact extends/implements type tokens, not resolved super-method claims.
    pub parent_types: Vec<NavigationParent>,
}
#[derive(Clone, Debug, Default)]
pub struct NavigationInspection {
    pub anchors: Vec<NavigationAnchor>,
    pub truncated: bool,
    pub incremental: bool,
}
#[derive(Clone, Debug)]
pub struct NavigationEdit {
    pub old: Range<usize>,
    pub new: Range<usize>,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NavigationStats {
    pub parses: u64,
    pub incremental_parses: u64,
}
#[derive(Default)]
pub struct PhpNavigationCache {
    parser: Option<Parser>,
    tree: Option<Tree>,
    source: String,
    stats: NavigationStats,
}
impl PhpNavigationCache {
    pub fn stats(&self) -> NavigationStats {
        self.stats
    }
    pub fn clear(&mut self) {
        self.tree = None;
        self.source.clear();
    }
    pub fn inspect(
        &mut self,
        source: &str,
        edit: Option<NavigationEdit>,
        cancel: &dyn Fn() -> bool,
    ) -> Result<NavigationInspection, String> {
        if source.len() > MAX_BYTES {
            self.clear();
            return Ok(NavigationInspection {
                truncated: true,
                ..Default::default()
            });
        }
        if cancel() {
            return Err("Análise de declarações cancelada".into());
        }
        let started = Instant::now();
        let interrupted = || cancel() || started.elapsed() >= DEADLINE;
        if self.parser.is_none() {
            let mut parser = Parser::new();
            parser
                .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
                .map_err(|e| e.to_string())?;
            self.parser = Some(parser);
        }
        let old = self.tree.as_ref().and_then(|tree| {
            let edit = edit.as_ref()?;
            if edit.old.start != edit.new.start
                || edit.old.start > edit.old.end
                || edit.new.start > edit.new.end
                || !self.source.is_char_boundary(edit.old.start)
                || !self.source.is_char_boundary(edit.old.end)
                || !source.is_char_boundary(edit.new.start)
                || !source.is_char_boundary(edit.new.end)
                || self.source.get(..edit.old.start) != source.get(..edit.new.start)
                || self.source.get(edit.old.end..) != source.get(edit.new.end..)
            {
                return None;
            }
            let mut tree = tree.clone();
            tree.edit(&InputEdit {
                start_byte: edit.old.start,
                old_end_byte: edit.old.end,
                new_end_byte: edit.new.end,
                start_position: point(&self.source, edit.old.start),
                old_end_position: point(&self.source, edit.old.end),
                new_end_position: point(source, edit.new.end),
            });
            Some(tree)
        });
        let parser = self.parser.as_mut().unwrap();
        let mut progress = |_: &tree_sitter::ParseState| {
            if interrupted() {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        };
        self.stats.parses += 1;
        let tree = parser.parse_with_options(
            &mut |offset, _| source.as_bytes().get(offset..).unwrap_or_default(),
            old.as_ref(),
            Some(ParseOptions::new().progress_callback(&mut progress)),
        );
        let Some(tree) = tree else {
            parser.reset();
            return Err(if cancel() {
                "Análise de declarações cancelada"
            } else {
                "Análise de declarações excedeu150ms"
            }
            .into());
        };
        if interrupted() {
            parser.reset();
            return Err("Análise de declarações cancelada ou excedeu150ms".into());
        }
        let incremental = old.is_some();
        if incremental {
            self.stats.incremental_parses += 1;
        }
        let mut result = anchors(&tree, source, &interrupted)?;
        result.incremental = incremental;
        self.tree = Some(tree);
        self.source.clear();
        self.source.push_str(source);
        Ok(result)
    }
}
fn point(source: &str, byte: usize) -> Point {
    let prefix = &source[..byte];
    Point::new(
        prefix.bytes().filter(|&b| b == b'\n').count(),
        prefix.rsplit('\n').next().unwrap_or("").len(),
    )
}
#[derive(Clone, Default)]
struct Container {
    name: Option<Arc<str>>,
    relations: bool,
    parents: Arc<[NavigationParent]>,
}
fn token(node: Node<'_>, kind: &str, source: &str, value: Option<&str>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|n| n.kind() == kind && value.is_none_or(|v| source.get(n.byte_range()) == Some(v)))
}
fn anchors(
    tree: &Tree,
    source: &str,
    interrupted: &dyn Fn() -> bool,
) -> Result<NavigationInspection, String> {
    let mut result = NavigationInspection::default();
    let mut stack = vec![(tree.root_node(), Container::default())];
    let mut visited = 0;
    while let Some((node, mut container)) = stack.pop() {
        visited += 1;
        if interrupted() {
            return Err("Análise de declarações cancelada ou excedeu150ms".into());
        }
        if visited > MAX_NODES {
            result.truncated = true;
            break;
        }
        // An unnamed class is a new lexical container; never attach its methods
        // to an enclosing factory/service class. No guessed anonymous supertype.
        if node.kind() == "anonymous_class" {
            container = Container::default();
        }
        let kind = match node.kind() {
            "class_declaration" | "enum_declaration" => Some(NavigationKind::Class),
            "interface_declaration" => Some(NavigationKind::Interface),
            "trait_declaration" => Some(NavigationKind::Trait),
            "function_definition" => Some(NavigationKind::Function),
            "method_declaration" => Some(NavigationKind::Method),
            _ => None,
        };
        if let Some(kind) = kind {
            let name = node.child_by_field_name("name");
            let relations = token(node, "base_clause", source, None)
                || token(node, "class_interface_clause", source, None);
            let mut parents = Vec::new();
            if relations {
                let mut cursor = node.walk();
                for clause in node
                    .named_children(&mut cursor)
                    .filter(|n| matches!(n.kind(), "base_clause" | "class_interface_clause"))
                {
                    let mut cursor = clause.walk();
                    for parent in clause.named_children(&mut cursor) {
                        let name = source.get(parent.byte_range()).unwrap_or("");
                        if parents.len() < 16
                            && !parent.is_missing()
                            && !parent.has_error()
                            && name.len() <= 256
                            && name
                                .chars()
                                .all(|c| c.is_alphanumeric() || c == '_' || c == '\\')
                        {
                            parents.push(NavigationParent {
                                name: name.into(),
                                line: 0,
                                scalar_range: 0..0,
                                byte_range: parent.byte_range(),
                            });
                        }
                    }
                }
            }
            if let Some(name) = name.filter(|n| !n.is_missing() && !n.has_error()) {
                let text = source.get(name.byte_range()).unwrap_or("");
                if !text.is_empty() && text.len() <= 256 {
                    if result.anchors.len() >= MAX_SYMBOLS {
                        result.truncated = true;
                        break;
                    }
                    let can_find_implementations = match kind {
                        NavigationKind::Interface => true,
                        NavigationKind::Class => !token(node, "final_modifier", source, None),
                        NavigationKind::Method => {
                            !token(node, "visibility_modifier", source, Some("private"))
                                && !token(node, "final_modifier", source, None)
                        }
                        _ => false,
                    };
                    result.anchors.push(NavigationAnchor {
                        name: text.into(),
                        kind,
                        line: 0,
                        scalar_range: 0..0,
                        byte_range: name.byte_range(),
                        container_name: container.name.as_deref().map(str::to_owned),
                        can_find_implementations,
                        parent_types: if kind == NavigationKind::Method {
                            container.parents.to_vec()
                        } else {
                            parents.clone()
                        },
                        has_parent_relations: if kind == NavigationKind::Method {
                            container.relations
                        } else {
                            relations
                        },
                    });
                }
                if matches!(
                    kind,
                    NavigationKind::Class | NavigationKind::Interface | NavigationKind::Trait
                ) {
                    container = Container {
                        name: Some(text.into()),
                        relations,
                        parents: parents.into(),
                    };
                }
            }
        }
        let mut cursor = node.walk();
        let children_start = stack.len();
        for child in node.named_children(&mut cursor) {
            if stack.len() >= MAX_NODES {
                result.truncated = true;
                break;
            }
            stack.push((child, container.clone()));
        }
        // A bounded partial result starts at the top of the file, where the
        // initial viewport is, rather than traversing declarations backwards.
        stack[children_start..].reverse();
    }
    result.anchors.sort_by_key(|a| a.byte_range.start);
    // Convert all anchor boundaries together: linear in source length, avoiding
    // one prefix scan per declaration in large generated classes.
    let mut boundaries = Vec::new();
    for (i, anchor) in result.anchors.iter().enumerate() {
        boundaries.extend([
            (anchor.byte_range.start, i, None, false),
            (anchor.byte_range.end, i, None, true),
        ]);
        for (p, parent) in anchor.parent_types.iter().enumerate() {
            boundaries.extend([
                (parent.byte_range.start, i, Some(p), false),
                (parent.byte_range.end, i, Some(p), true),
            ]);
        }
    }
    boundaries.sort_unstable();
    let mut next = 0;
    let mut line = 0;
    let mut cr = false;
    for (scalar, (byte, ch)) in source
        .char_indices()
        .chain(std::iter::once((source.len(), '\0')))
        .enumerate()
    {
        if next == boundaries.len() {
            break;
        }
        if scalar % 256 == 0 && interrupted() {
            return Err("Análise de declarações cancelada ou excedeu150ms".into());
        }
        while next < boundaries.len() && boundaries[next].0 == byte {
            let (_, i, parent, end) = boundaries[next];
            let (range, target_line) = if let Some(parent) = parent {
                let p = &mut result.anchors[i].parent_types[parent];
                (&mut p.scalar_range, &mut p.line)
            } else {
                let a = &mut result.anchors[i];
                (&mut a.scalar_range, &mut a.line)
            };
            if end {
                range.end = scalar;
            } else {
                range.start = scalar;
                *target_line = line;
            }
            next += 1;
        }
        if ch == '\r' {
            line += 1;
            cr = true;
        } else if ch == '\n' {
            if !cr {
                line += 1;
            }
            cr = false;
        } else {
            cr = false;
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchors_preserve_emoji_crlf_private_and_interface_context() {
        let text = "<?php /* 😀 */\r\ninterface Port { function send(); }\r\nclass Service implements Port { public function send(){} private function secret(){} }\rfunction helper(){}";
        let mut cache = PhpNavigationCache::default();
        let result = cache.inspect(text, None, &|| false).unwrap();
        assert_eq!(result.anchors.len(), 6);
        let method = result
            .anchors
            .iter()
            .find(|a| {
                a.kind == NavigationKind::Method
                    && a.container_name.as_deref() == Some("Service")
                    && a.name == "send"
            })
            .unwrap();
        assert_eq!(method.line, 2);
        assert_eq!(
            text.chars()
                .skip(method.scalar_range.start)
                .take(method.scalar_range.len())
                .collect::<String>(),
            "send"
        );
        assert!(method.has_parent_relations);
        assert_eq!(method.parent_types[0].name, "Port");
        assert_eq!(&text[method.parent_types[0].byte_range.clone()], "Port");
        assert_eq!(
            text.chars()
                .skip(method.parent_types[0].scalar_range.start)
                .take(method.parent_types[0].scalar_range.len())
                .collect::<String>(),
            "Port"
        );
        assert!(method.can_find_implementations);
        assert!(
            !result
                .anchors
                .iter()
                .find(|a| a.name == "secret")
                .unwrap()
                .can_find_implementations
        );
        assert_eq!(
            result
                .anchors
                .iter()
                .find(|a| a.name == "helper")
                .unwrap()
                .line,
            3
        );
    }
    #[test]
    fn verified_edit_reuses_tree_and_unrelated_or_skipped_changes_cold_parse() {
        let mut cache = PhpNavigationCache::default();
        let before = "<?php /* 😀 */ function before() {}";
        cache.inspect(before, None, &|| false).unwrap();
        let start = before.find("before").unwrap();
        let after = before.replace("before", "after");
        let result = cache
            .inspect(
                &after,
                Some(NavigationEdit {
                    old: start..start + 6,
                    new: start..start + 5,
                }),
                &|| false,
            )
            .unwrap();
        assert!(result.incremental);
        assert_eq!(result.anchors[0].name, "after");
        assert_eq!(cache.stats().incremental_parses, 1);
        let result = cache
            .inspect(
                "<?php function different() {}",
                Some(NavigationEdit {
                    old: 0..1,
                    new: 0..1,
                }),
                &|| false,
            )
            .unwrap();
        assert!(!result.incremental);
        assert_eq!(result.anchors[0].name, "different");
    }
    #[test]
    fn cancellation_and_caps_do_not_publish_old_coordinates() {
        let mut cache = PhpNavigationCache::default();
        assert!(
            cache
                .inspect("<?php function x(){}", None, &|| true)
                .is_err()
        );
        assert_eq!(cache.stats().parses, 0);
        assert!(
            cache
                .inspect(&" ".repeat(MAX_BYTES + 1), None, &|| false)
                .unwrap()
                .truncated
        );
        let source = format!(
            "<?php {}",
            (0..MAX_SYMBOLS + 1)
                .map(|i| format!("function f{i}(){{}}\n"))
                .collect::<String>()
        );
        let r = cache.inspect(&source, None, &|| false).unwrap();
        assert!(r.truncated);
        assert_eq!(r.anchors.len(), MAX_SYMBOLS);
    }
    #[test]
    fn strings_comments_and_anonymous_factory_classes_do_not_inherit_outer_metadata() {
        let source = "<?php class Outer extends Base { function factory(){ /* function fake(){} */ $x='function nope(){}';return new class {function inner(){}};}} function ação(){}";
        let result = PhpNavigationCache::default()
            .inspect(source, None, &|| false)
            .unwrap();
        assert!(
            !result
                .anchors
                .iter()
                .any(|a| matches!(a.name.as_str(), "fake" | "nope"))
        );
        let inner = result.anchors.iter().find(|a| a.name == "inner").unwrap();
        assert!(inner.container_name.is_none());
        assert!(!inner.has_parent_relations);
        assert!(inner.parent_types.is_empty());
        let unicode = result.anchors.iter().find(|a| a.name == "ação").unwrap();
        assert_eq!(
            source
                .chars()
                .skip(unicode.scalar_range.start)
                .take(unicode.scalar_range.len())
                .collect::<String>(),
            "ação"
        );
    }
}
