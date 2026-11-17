//! Exact method identifiers inside a provider-confirmed declaration, never a
//! project scan or a syntactic guess about runtime bindings.
use crate::DefinitionTarget;
use std::ops::ControlFlow;
use std::time::{Duration, Instant};
use tree_sitter::{ParseOptions, Parser};
const LIMIT: usize = 1024 * 1024;
pub(crate) fn read_base(path: &std::path::Path, root: &std::path::Path) -> Option<String> {
    use std::io::Read;
    if !path.starts_with(root) || path.canonicalize().ok()?.as_path() != path {
        return None;
    }
    let before = std::fs::symlink_metadata(path).ok()?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > LIMIT as u64 {
        return None;
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    let file = options.open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64).read_to_end(&mut bytes).ok()?;
    let after = std::fs::symlink_metadata(path).ok()?;
    if bytes.len() > LIMIT
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || path.canonicalize().ok()?.as_path() != path
    {
        return None;
    }
    String::from_utf8(bytes).ok()
}
fn tree(source: &str) -> Option<tree_sitter::Tree> {
    if source.len() > LIMIT || source.contains('\0') {
        return None;
    }
    let until = Instant::now() + Duration::from_millis(100);
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .ok()?;
    let mut progress = |_: &tree_sitter::ParseState| {
        if Instant::now() >= until {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let tree = parser.parse_with_options(
        &mut |offset, _| source.as_bytes().get(offset..).unwrap_or_default(),
        None,
        Some(ParseOptions::new().progress_callback(&mut progress)),
    )?;
    (!tree.root_node().has_error()).then_some(tree)
}
pub(crate) fn method_at(source: &str, line: u32, character: u32) -> Option<String> {
    let at = crate::utf16_byte_offset(source, line, character)?;
    let tree = tree(source)?;
    let mut node = tree.root_node().descendant_for_byte_range(at, at)?;
    while node.kind() != "method_declaration" {
        node = node.parent()?;
    }
    let name = node.child_by_field_name("name")?;
    if !(name.start_byte()..name.end_byte()).contains(&at) {
        return None;
    }
    let value = source.get(name.byte_range())?;
    (value.len() <= 256).then(|| value.to_owned())
}
fn position(source: &str, offset: usize) -> Option<(u32, u32)> {
    let prefix = source.get(..offset)?;
    let line = prefix.bytes().filter(|b| *b == b'\n').count();
    let column = prefix.rsplit('\n').next()?.encode_utf16().count();
    Some((line.try_into().ok()?, column.try_into().ok()?))
}
pub(crate) fn target_method(
    source: &str,
    target: &DefinitionTarget,
    method: &str,
) -> Option<(u32, u32)> {
    let tree = tree(source)?;
    let start = crate::utf16_byte_offset(source, target.start_line, target.start_character)?;
    let end = crate::utf16_byte_offset(source, target.end_line, target.end_character)?;
    if start > end {
        return None;
    }
    let mut stack = vec![tree.root_node()];
    let mut remaining = 200_000usize;
    let mut found = None;
    while let Some(node) = stack.pop() {
        if remaining == 0 {
            return None;
        }
        remaining -= 1;
        if node.end_byte() < start || node.start_byte() > end {
            continue;
        }
        if node.kind() == "method_declaration" {
            let name = node.child_by_field_name("name")?;
            if name.start_byte() >= start
                && name.end_byte() <= end
                && source.get(name.byte_range())?.eq_ignore_ascii_case(method)
            {
                if found.is_some() {
                    return None;
                }
                found = position(source, name.start_byte());
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    found
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_method_declarations_and_provider_range_are_accepted() {
        let source = "<?php\r\n/* 😀 */ interface Contract { public function passes(): bool; }\r\nclass Impl implements Contract { public function passes(): bool { return true; } }\r\n";
        let byte = source.find("passes").unwrap();
        let at = position(source, byte + 1).unwrap();
        assert_eq!(method_at(source, at.0, at.1).as_deref(), Some("passes"));
        assert!(method_at(source, 1, 3).is_none());
        let end = position(source, source.find(" }\r\n").unwrap()).unwrap();
        let target = DefinitionTarget {
            path: "/unused".into(),
            start_line: 1,
            start_character: 0,
            end_line: end.0,
            end_character: end.1,
        };
        assert_eq!(
            target_method(source, &target, "passes"),
            position(source, byte)
        );
        assert!(target_method(source, &target, "other").is_none());
        let mut broad = target.clone();
        broad.end_line = 2;
        broad.end_character = 80;
        assert!(target_method(source, &broad, "passes").is_none());
    }
    #[test]
    fn bounded_base_read_rejects_escape_special_oversized_and_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let inside = root.join("Base.php");
        std::fs::write(
            &inside,
            "<?php interface Base {}
",
        )
        .unwrap();
        assert!(read_base(&inside, &root).is_some());
        let outside = dir.path().join("outside.php");
        std::fs::write(&outside, "<?php").unwrap();
        assert!(read_base(&outside, &root).is_none());
        std::fs::write(&inside, [0xff]).unwrap();
        assert!(read_base(&inside, &root).is_none());
        std::fs::write(&inside, vec![b'x'; LIMIT + 1]).unwrap();
        assert!(read_base(&inside, &root).is_none());
        assert!(read_base(&root, &root).is_none());
        #[cfg(unix)]
        {
            let link = root.join("escape.php");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(read_base(&link, &root).is_none());
        }
    }
}
