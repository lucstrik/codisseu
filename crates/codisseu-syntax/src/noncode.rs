//! Explicit, optional textual occurrences. These edits never count as semantic
//! evidence and are kept separate from the symbol proposal by the application.
use crate::{
    RenameEdit, RenameSnapshot, ValidationLimits,
    ast::Language,
    incremental::{self, Budget},
};
use std::{path::Path, sync::atomic::AtomicBool};
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RenameSearchScope {
    #[default]
    Project,
    CurrentFile,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenameOptions {
    /// File-origin Rename only: false deliberately leaves semantic module references unchanged.
    pub search_file_references: bool,
    /// Excluding JavaScript proposals never excludes their snapshots from proof.
    pub search_javascript: bool,
    /// Show uncertain references found by mandatory semantic auditing for review.
    pub search_dynamic_references: bool,
    pub search_comments_strings: bool,
    pub rename_accessors: bool,
    pub rename_inheritors: bool,
    pub search_text: bool,
    /// Scope of optional occurrences; semantic references remain mandatory.
    pub scope: RenameSearchScope,
}
impl Default for RenameOptions {
    fn default() -> Self {
        Self {
            search_file_references: true,
            search_javascript: true,
            search_dynamic_references: false,
            search_comments_strings: false,
            search_text: false,
            scope: RenameSearchScope::Project,
            rename_accessors: true,
            rename_inheritors: true,
        }
    }
}
fn module_literal(node: tree_sitter::Node<'_>, source: &str) -> bool {
    let mut ancestor = node.parent();
    while let Some(owner) = ancestor {
        if matches!(owner.kind(), "import_statement" | "export_statement")
            && owner.child_by_field_name("source").is_some_and(|n| {
                n.start_byte() <= node.start_byte() && n.end_byte() >= node.end_byte()
            })
        {
            return true;
        }
        if matches!(
            owner.kind(),
            "include_expression"
                | "include_once_expression"
                | "require_expression"
                | "require_once_expression"
        ) {
            return true;
        }
        if owner.kind() == "call_expression"
            && owner
                .child_by_field_name("function")
                .is_some_and(|n| matches!(&source[n.byte_range()], "require" | "import"))
        {
            return true;
        }
        if matches!(
            owner.kind(),
            "program" | "expression_statement" | "lexical_declaration" | "return_statement"
        ) {
            break;
        }
        ancestor = owner.parent();
    }
    false
}
fn word(ch: char) -> bool {
    ch == '$' || ch == '_' || unicode_ident::is_xid_continue(ch)
}
fn occurrences(
    source: &str,
    range: std::ops::Range<usize>,
    old: &str,
    new: &str,
    out: &mut Vec<RenameEdit>,
    budget: &mut Budget<'_>,
) -> Result<(), String> {
    for (relative, _) in source[range.clone()].match_indices(old) {
        budget
            .node()
            .map_err(|_| "Busca textual cancelada ou excedida")?;
        let start = range.start + relative;
        let end = start + old.len();
        if source[..start].chars().next_back().is_some_and(word)
            || source[end..].chars().next().is_some_and(word)
        {
            continue;
        }
        if out.len() >= 1024 {
            return Err("Mais de 1024 ocorrências textuais no arquivo".into());
        }
        out.push(RenameEdit {
            range: incremental::text_range(source, start..end),
            new_text: new.into(),
        });
    }
    Ok(())
}
/// Finds whole identifier spellings in AST comments/plain string literals, and
/// optionally UTF-8 .txt/.md snapshots. Does not search executable identifiers,
/// interpolations, escaped strings, or guessed dynamic references.
pub fn plan_noncode_rename(
    snapshots: &[RenameSnapshot],
    origin: &Path,
    old: &str,
    new: &str,
    options: RenameOptions,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Vec<(std::path::PathBuf, RenameEdit)>, String> {
    crate::validate_rename_candidate(old).map_err(str::to_owned)?;
    crate::validate_rename_candidate(new).map_err(str::to_owned)?;
    plan(snapshots, origin, old, new, options, cancel, limits)
}

/// File-origin textual search uses complete safe basenames, independently from
/// the strict identifier contract of symbol search. AST exclusions are shared.
pub fn plan_file_noncode_rename(
    snapshots: &[RenameSnapshot],
    origin: &Path,
    old: &str,
    new: &str,
    options: RenameOptions,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Vec<(std::path::PathBuf, RenameEdit)>, String> {
    crate::validate_file_rename_name(old).map_err(str::to_owned)?;
    crate::validate_file_rename_name(new).map_err(str::to_owned)?;
    if Path::new(old).extension() != Path::new(new).extension() {
        return Err("Busca de arquivo conserva extensão".into());
    }
    plan(snapshots, origin, old, new, options, cancel, limits)
}

fn plan(
    snapshots: &[RenameSnapshot],
    origin: &Path,
    old: &str,
    new: &str,
    options: RenameOptions,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Vec<(std::path::PathBuf, RenameEdit)>, String> {
    if !options.search_comments_strings && !options.search_text {
        return Ok(vec![]);
    }
    if snapshots.len() > 128
        || snapshots.iter().map(|s| s.before.len()).sum::<usize>() > 8 * 1024 * 1024
    {
        return Err("Busca textual excede 128 arquivos/8 MiB".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut result = Vec::new();
    for snapshot in snapshots {
        budget
            .node()
            .map_err(|_| "Busca textual cancelada ou excedida")?;
        if options.scope == RenameSearchScope::CurrentFile && snapshot.path != origin {
            continue;
        }
        if snapshot.before.len() > 1024 * 1024 {
            return Err("Busca textual excede 1 MiB por arquivo".into());
        }
        let source = snapshot.before.as_ref();
        let ext = snapshot
            .path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let mut edits = Vec::new();
        if options.search_text && matches!(ext, "txt" | "md") {
            occurrences(source, 0..source.len(), old, new, &mut edits, &mut budget)?;
        } else if options.search_comments_strings
            && matches!(ext, "php" | "ts" | "tsx" | "js" | "jsx")
        {
            let language = if ext == "php" {
                Language::Php
            } else {
                Language::TypeScript
            };
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&if matches!(ext, "tsx" | "jsx") {
                    tree_sitter_typescript::LANGUAGE_TSX.into()
                } else {
                    language.grammar()
                })
                .map_err(|e| e.to_string())?;
            let tree = incremental::parse(&mut parser, source, None, &budget)
                .ok_or("Busca textual cancelada ou excedida")?;
            if tree.root_node().has_error() {
                return Err(
                    "Não é possível delimitar comentários/strings em sintaxe inválida".into(),
                );
            }
            let mut stack = vec![tree.root_node()];
            while let Some(node) = stack.pop() {
                budget
                    .node()
                    .map_err(|_| "Busca textual cancelada ou excedida")?;
                let raw = &source[node.byte_range()];
                if node.kind() == "comment" {
                    occurrences(source, node.byte_range(), old, new, &mut edits, &mut budget)?;
                } else if node.kind() == "string" {
                    if raw.starts_with(['\'', '"'])
                        && raw.len() >= 2
                        && !raw.contains('\\')
                        && !module_literal(node, source)
                    {
                        occurrences(
                            source,
                            node.start_byte() + 1..node.end_byte() - 1,
                            old,
                            new,
                            &mut edits,
                            &mut budget,
                        )?;
                    }
                } else {
                    let mut cursor = node.walk();
                    stack.extend(node.named_children(&mut cursor));
                }
            }
        }
        edits.sort_by_key(|e| (e.range.start_line, e.range.start_character));
        for edit in edits {
            let overlaps = snapshot.edits.iter().any(|existing| {
                let a = &edit.range;
                let b = &existing.range;
                (a.start_line, a.start_character) < (b.end_line, b.end_character)
                    && (b.start_line, b.start_character) < (a.end_line, a.end_character)
            });
            if !overlaps {
                result.push((snapshot.path.clone(), edit));
            }
        }
    }
    Ok(result)
}
