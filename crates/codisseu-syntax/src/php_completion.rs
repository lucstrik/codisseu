//! A bounded syntactic guard for host-added call parentheses, run on a worker.
use crate::{
    ValidationLimits,
    ast::Language,
    incremental::{self, Budget},
};
use std::{sync::atomic::AtomicBool, time::Duration};

/// Confirms that the completed name is the callable part of an actual PHP call.
/// It does not resolve the name or infer arguments; those come from the provider.
pub fn is_call_name(text: &str, end_scalar: usize, name_chars: usize) -> bool {
    if text.len() > 1024 * 1024 || end_scalar < name_chars {
        return false;
    }
    let byte = |scalar| {
        text.char_indices()
            .nth(scalar)
            .map_or(text.len(), |(at, _)| at)
    };
    let start = byte(end_scalar - name_chars);
    let end = byte(end_scalar);
    let cancel = AtomicBool::new(false);
    let budget = Budget::new(
        &cancel,
        ValidationLimits {
            max_duration: Duration::from_millis(150),
            ..Default::default()
        },
    );
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&Language::Php.grammar()).is_err() {
        return false;
    }
    let Some(tree) = incremental::parse(&mut parser, text, None, &budget) else {
        return false;
    };
    let Some(mut node) = tree.root_node().descendant_for_byte_range(start, end) else {
        return false;
    };
    let mut call = false;
    for _ in 0..256 {
        if matches!(
            node.kind(),
            "comment" | "string" | "encapsed_string" | "heredoc" | "nowdoc"
        ) {
            return false;
        }
        let field = match node.kind() {
            "member_call_expression"
            | "nullsafe_member_call_expression"
            | "scoped_call_expression" => Some("name"),
            "function_call_expression" => Some("function"),
            _ => None,
        };
        call |= field
            .and_then(|field| node.child_by_field_name(field))
            .is_some_and(|name| name.start_byte() == start && name.end_byte() == end);
        let Some(parent) = node.parent() else {
            return call;
        };
        node = parent;
    }
    false
}
