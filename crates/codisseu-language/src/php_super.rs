//! Resolve only provider-confirmed parent types, then select the nearest declared
//! method per branch. No project scan and no inference of container bindings.
use crate::DefinitionTarget;
use std::{
    collections::{HashSet, VecDeque},
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tree_sitter::{Node, ParseOptions, Parser, Tree};
const LIMIT: usize = 64;
#[derive(Clone)]
pub(crate) struct Query {
    pub path: PathBuf,
    pub text: String,
    pub line: u32,
    pub character: u32,
    depth: usize,
}
pub(crate) struct Search {
    method: Option<String>,
    queue: VecDeque<Query>,
    queued: HashSet<(PathBuf, u32, u32)>,
    visited: HashSet<(PathBuf, usize)>,
    bytes: usize,
    pub step: usize,
    pub targets: Vec<DefinitionTarget>,
}
fn parse(text: &str) -> Result<Tree, String> {
    if text.len() > 1024 * 1024 || text.contains('\0') {
        return Err("Fonte PHP excede o limite de navegação".into());
    }
    if text
        .as_bytes()
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && text.as_bytes().get(i + 1) != Some(&b'\n'))
    {
        return Err(
            "Navegação aos ascendentes requer LF ou CRLF; CR isolado não suportado pelo provider"
                .into(),
        );
    }
    let start = Instant::now();
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .map_err(|e| e.to_string())?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if start.elapsed() > Duration::from_millis(100) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser
        .parse_with_options(
            &mut |offset, _| text.as_bytes().get(offset..).unwrap_or_default(),
            None,
            Some(ParseOptions::new().progress_callback(&mut progress)),
        )
        .ok_or("Análise PHP excedeu o orçamento")?;
    if tree.root_node().has_error() {
        return Err("Corrija a sintaxe PHP antes de navegar aos ascendentes".into());
    }
    Ok(tree)
}
fn is_type(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "trait_declaration" | "enum_declaration"
    )
}
fn position(text: &str, offset: usize) -> Result<(u32, u32), String> {
    let prefix = text.get(..offset).ok_or("Posição PHP inválida")?;
    Ok((
        prefix.bytes().filter(|b| *b == b'\n').count() as u32,
        prefix
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .encode_utf16()
            .count() as u32,
    ))
}
fn target(path: &Path, text: &str, name: Node<'_>) -> Result<DefinitionTarget, String> {
    let (start_line, start_character) = position(text, name.start_byte())?;
    let (end_line, end_character) = position(text, name.end_byte())?;
    Ok(DefinitionTarget {
        path: path.into(),
        start_line,
        start_character,
        end_line,
        end_character,
    })
}
fn private(node: Node<'_>, text: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|n| n.kind() == "visibility_modifier" && text.get(n.byte_range()) == Some("private"))
}
impl Search {
    pub fn start(path: &Path, text: &str, line: u32, character: u32) -> Result<Self, String> {
        let at =
            crate::utf16_byte_offset(text, line, character).ok_or("Posição UTF-16 inválida")?;
        let tree = parse(text)?;
        let mut node = tree
            .root_node()
            .descendant_for_byte_range(at, at)
            .ok_or("Nenhuma declaração PHP nesta posição")?;
        let mut method = None;
        loop {
            if node.kind() == "anonymous_class" {
                return Err("Ascendentes de classe anônima ainda não suportados".into());
            }
            if node.kind() == "function_definition" {
                return Err("Função não possui supermétodo".into());
            }
            if node.kind() == "method_declaration" {
                if private(node, text) {
                    return Err("Método privado não sobrescreve método ancestral".into());
                }
                method = Some(
                    text[node
                        .child_by_field_name("name")
                        .ok_or("Método sem nome")?
                        .byte_range()]
                    .to_string(),
                );
            }
            if is_type(node) {
                break;
            }
            node = node
                .parent()
                .ok_or("Posicione o cursor em uma classe ou método PHP")?;
        }
        let mut search = Self {
            method,
            queue: VecDeque::new(),
            queued: HashSet::new(),
            visited: HashSet::new(),
            bytes: 0,
            step: 0,
            targets: vec![],
        };
        search.visited.insert((path.into(), node.start_byte()));
        search.enqueue_parents(path, text, node, 0)?;
        Ok(search)
    }
    fn enqueue_parents(
        &mut self,
        path: &Path,
        text: &str,
        node: Node<'_>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 16 {
            return Err("Hierarquia PHP excedeu 16 níveis".into());
        }
        let mut cursor = node.walk();
        for clause in node
            .named_children(&mut cursor)
            .filter(|n| matches!(n.kind(), "base_clause" | "class_interface_clause"))
        {
            let mut cursor = clause.walk();
            for parent in clause.named_children(&mut cursor) {
                if !matches!(parent.kind(), "name" | "qualified_name" | "relative_name") {
                    return Err("Supertipo PHP não suportado".into());
                }
                let (line, character) = position(text, parent.start_byte())?;
                if !self.queued.insert((path.into(), line, character)) {
                    continue;
                }
                self.bytes += text.len();
                if self.queued.len() > LIMIT || self.bytes > 16 * 1024 * 1024 {
                    return Err("Hierarquia PHP excedeu o orçamento de navegação".into());
                }
                self.queue.push_back(Query {
                    path: path.into(),
                    text: text.into(),
                    line,
                    character,
                    depth,
                });
            }
        }
        Ok(())
    }
    pub fn next(&mut self) -> Option<Query> {
        let q = self.queue.pop_front();
        if q.is_some() {
            self.step += 1;
        }
        q
    }
    pub fn accept(
        &mut self,
        definition: &DefinitionTarget,
        text: &str,
        depth: usize,
    ) -> Result<(), String> {
        let tree = parse(text)?;
        let start =
            crate::utf16_byte_offset(text, definition.start_line, definition.start_character)
                .ok_or("Supertipo retornou posição inválida")?;
        let end = crate::utf16_byte_offset(text, definition.end_line, definition.end_character)
            .ok_or("Supertipo retornou posição inválida")?;
        if start > end {
            return Err("Intervalo de supertipo inválido".into());
        }
        // Require the range to overlap exactly one type's name. Broad whole-file
        // or same-name method responses cannot manufacture a relation.
        let mut stack = vec![tree.root_node()];
        let mut found = None;
        let mut nodes = 0;
        while let Some(node) = stack.pop() {
            nodes += 1;
            if nodes > 100_000 {
                return Err("Supertipo excedeu orçamento AST".into());
            }
            if is_type(node)
                && let Some(name) = node.child_by_field_name("name")
                && if start == end {
                    (name.start_byte()..name.end_byte()).contains(&start)
                } else {
                    start < name.end_byte() && end > name.start_byte()
                }
            {
                if found.is_some() {
                    return Err("Resposta de supertipo ambígua".into());
                }
                found = Some(node);
            }
            let mut cursor = node.walk();
            stack.extend(node.named_children(&mut cursor));
        }
        let node = found.ok_or("Provider não retornou uma declaração de tipo verificável")?;
        if !self
            .visited
            .insert((definition.path.clone(), node.start_byte()))
        {
            return Ok(());
        }
        if self.visited.len() > LIMIT {
            return Err("Hierarquia PHP excedeu 64 tipos".into());
        }
        if let Some(method) = &self.method {
            let body = node
                .child_by_field_name("body")
                .ok_or("Supertipo sem corpo verificável")?;
            let mut cursor = body.walk();
            let mut matched = None;
            for member in body
                .named_children(&mut cursor)
                .filter(|n| n.kind() == "method_declaration")
            {
                let name = member
                    .child_by_field_name("name")
                    .ok_or("Método sem nome")?;
                if text[name.byte_range()].eq_ignore_ascii_case(method) {
                    if private(member, text) {
                        return Ok(());
                    }
                    if matched.is_some() {
                        return Err("Supertipo declara método duplicado".into());
                    }
                    matched = Some(target(&definition.path, text, name)?);
                }
            }
            if let Some(found) = matched {
                self.targets.push(found);
            } else {
                let mut cursor = body.walk();
                if body
                    .named_children(&mut cursor)
                    .any(|n| n.kind() == "use_declaration")
                {
                    return Err(
                        "Método pode vir de trait; ascendentes não certificados neste ramo".into(),
                    );
                }
                self.enqueue_parents(&definition.path, text, node, depth + 1)?;
            }
        } else {
            self.targets.push(target(
                &definition.path,
                text,
                node.child_by_field_name("name")
                    .ok_or("Supertipo sem nome")?,
            )?);
        }
        if self.targets.len() > LIMIT {
            return Err("Mais de 64 ascendentes".into());
        }
        Ok(())
    }
}
impl Query {
    pub fn depth(&self) -> usize {
        self.depth
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjacent_and_empty_ranges_cannot_manufacture_a_parent() {
        let source = "<?php class Child extends Base {}";
        let parent = "<?php class Base {}";
        for (start, end, accepted) in [
            (11, 12, false),
            (16, 16, false),
            (16, 17, false),
            (12, 12, true),
            (13, 13, true),
            (12, 16, true),
        ] {
            let mut search = Search::start(Path::new("/Child.php"), source, 0, 13).unwrap();
            let range = DefinitionTarget {
                path: "/Base.php".into(),
                start_line: 0,
                start_character: start,
                end_line: 0,
                end_character: end,
            };
            assert_eq!(
                search.accept(&range, parent, 0).is_ok(),
                accepted,
                "{start}..{end}"
            );
            assert_eq!(search.targets.len(), usize::from(accepted));
        }
    }
    fn def(text: &str, name: &str) -> DefinitionTarget {
        let p = text.find(name).unwrap();
        let (a, b) = position(text, p).unwrap();
        DefinitionTarget {
            path: PathBuf::from(format!("/{name}.php")),
            start_line: a,
            start_character: b,
            end_line: a,
            end_character: b + name.len() as u32,
        }
    }
    #[test]
    fn nearest_method_each_branch_and_empty_interface() {
        let text =
            "<?php\nclass Child extends Base implements Port { function deliver(){ return 1; } }";
        let mut s = Search::start(Path::new("/Child.php"), text, 1, 65).unwrap();
        let base = "<?php class Base extends Root { function deliver(){} }";
        let port = "<?php interface Port extends Contract {}";
        let contract = "<?php interface Contract { function deliver(); }";
        assert_eq!(s.next().unwrap().character, 20);
        s.accept(&def(base, "Base"), base, 0).unwrap();
        assert_eq!(s.targets.len(), 1);
        assert_eq!(s.targets[0].path, Path::new("/Base.php"));
        s.next().unwrap();
        s.accept(&def(port, "Port"), port, 0).unwrap();
        assert_eq!(s.next().unwrap().path, Path::new("/Port.php"));
        s.accept(&def(contract, "Contract"), contract, 1).unwrap();
        assert!(s.next().is_none());
        assert_eq!(s.targets.len(), 2);
    }
    #[test]
    fn class_super_and_invalid_responses_private_anonymous_are_distinct() {
        let t = "<?php class Child extends Base { private function x(){} }";
        let mut s = Search::start(Path::new("/Child.php"), t, 0, 15).unwrap();
        assert!(s.method.is_none());
        let b = "<?php class Base {}";
        s.accept(&def(b, "Base"), b, 0).unwrap();
        assert_eq!(s.targets[0].start_character, 12);
        assert!(Search::start(Path::new("/Child.php"), t, 0, 50).is_err());
        let a = "<?php class Outer {function f(){return new class {function g(){}};}}";
        assert!(Search::start(Path::new("/a.php"), a, 0, 59).is_err());
        let invalid = "<?php class X {} class Y {}";
        let broad = DefinitionTarget {
            path: "/invalid.php".into(),
            start_line: 0,
            start_character: 0,
            end_line: 0,
            end_character: 26,
        };
        assert!(s.accept(&broad, invalid, 0).is_err());
    }
    #[test]
    fn trait_inheritance_is_not_silently_skipped_and_cr_only_is_explicit() {
        let source = "<?php class Child extends Base {function run(){}}";
        let mut s = Search::start(Path::new("/Child.php"), source, 0, 42).unwrap();
        let base = "<?php class Base extends Root {use Runner;}";
        assert!(
            s.accept(&def(base, "Base"), base, 0)
                .unwrap_err()
                .contains("trait")
        );
        assert!(
            Search::start(
                Path::new("/cr.php"),
                "<?php\rclass Child extends Base {}",
                0,
                0
            )
            .err()
            .unwrap()
            .contains("CR")
        );
    }
    #[test]
    fn diamond_and_cycle_are_bounded_and_utf16_anchors_preserved() {
        let source = "<?php\r\n/* 😀 */ interface Child extends Left, Right {function run();}";
        let start = source.find("run").unwrap();
        let (line, column) = position(source, start).unwrap();
        let mut s = Search::start(Path::new("/Child.php"), source, line, column).unwrap();
        let left = "<?php interface Left extends Root {}";
        let right = "<?php interface Right extends Root {}";
        let root = "<?php /* 😀 */ interface Root {function run();}";
        s.accept(&def(left, "Left"), left, 0).unwrap();
        s.accept(&def(right, "Right"), right, 0).unwrap();
        s.accept(&def(root, "Root"), root, 1).unwrap();
        s.accept(&def(root, "Root"), root, 1).unwrap();
        assert_eq!(s.targets.len(), 1);
        let t = &s.targets[0];
        assert_eq!(
            crate::utf16_byte_offset(root, t.start_line, t.start_character),
            root.find("run")
        );
        let child_target = def(source, "Child");
        s.accept(&child_target, source, 2).unwrap();
        assert_eq!(s.targets.len(), 1);
    }
}
