//! Rust-analyzer resolves bindings. This independent check certifies the exact
//! identifier edits, unchanged concrete syntax shape and a conservative no-capture
//! condition over the supplied bounded corpus. It never manufactures references.
use crate::{
    Conflict, ConflictKind, Coverage, IssueKind, RenameSnapshot, RenameValidation, ValidationIssue,
    ValidationLimits, ValidationScope,
    incremental::{self, Budget},
};
use std::{path::Path, sync::atomic::AtomicBool};
use tree_sitter::{Node, Parser, Tree};

fn report() -> RenameValidation {
    RenameValidation {
        coverage: Coverage::Unsupported,
        scope: ValidationScope::RustAnalyzerBindings,
        conflicts: vec![], issues: vec![], checked_declarations: 0,
        checked_override_pairs: 0, related_methods: vec![], checked_files: 0,
        incremental_reparses: 0,
        note: "Bindings fornecidos pelo rust-analyzer; tokens e estrutura Rust conferidos por AST antes/depois. Nome novo preexistente no corpus é recusado conservadoramente para impedir captura/colisão. Não é typechecking global nem prova de completude do provider, macros, cfg ou dependências externas.".into(),
    }
}
fn issue(report: &mut RenameValidation, path: &Path, kind: IssueKind, message: impl Into<String>) {
    if report.issues.len() < 32 {
        report.issues.push(ValidationIssue {
            path: path.into(),
            range: None,
            kind,
            message: message.into(),
        });
    }
}
fn identifier(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "identifier" | "type_identifier" | "field_identifier"
    )
}
fn base(name: &str) -> &str {
    name.strip_prefix("r#").unwrap_or(name)
}

/// Parse the candidate in a binding position; keywords and `_` cannot pretend
/// to be identifiers, while Rust's raw and Unicode identifiers remain intact.
pub fn validate_candidate(name: &str) -> Result<(), &'static str> {
    let value = base(name);
    // The grammar deliberately accepts contextual names in some patterns; the
    // Rust Reference's strict/reserved keyword set closes that lexical gap.
    let keyword = matches!(
        value,
        "as" | "break"
            | "const"
            | "continue"
            | "crate"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "async"
            | "await"
            | "dyn"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "gen"
            | "macro"
            | "override"
            | "priv"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
            | "try"
    );
    if (keyword && !name.starts_with("r#"))
        || (name.starts_with("r#") && matches!(value, "crate" | "self" | "super" | "Self"))
    {
        return Err("Nome reservado Rust; use identificador raw quando permitido");
    }
    if value.is_empty()
        || name.len() > 1024
        || value == "_"
        || value.chars().enumerate().any(|(i, c)| {
            !(c == '_'
                || if i == 0 {
                    unicode_ident::is_xid_start(c)
                } else {
                    unicode_ident::is_xid_continue(c)
                })
        })
    {
        return Err("Novo nome deve ser um identificador Rust Unicode/raw válido");
    }
    let text = format!("fn main() {{ let {name} = 0; }}");
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|_| "Parser Rust indisponível")?;
    let tree = parser
        .parse(&text, None)
        .ok_or("Parser Rust indisponível")?;
    let start = text.find("let ").unwrap() + 4;
    let node = tree
        .root_node()
        .descendant_for_byte_range(start, start + name.len())
        .ok_or("Identificador Rust inválido")?;
    if tree.root_node().has_error()
        || !identifier(node)
        || node.byte_range() != (start..start + name.len())
    {
        return Err(
            "O novo nome é palavra reservada ou identificador Rust inválido; use r# quando apropriado",
        );
    }
    Ok(())
}
/// Presentation only: the language server still owns prepareRename/bindings.
pub fn classify(
    text: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
) -> Result<crate::RenameSymbolKind, String> {
    use crate::RenameSymbolKind as K;
    if text.len() > 1024 * 1024 {
        return Err("Descrição de símbolo Rust excede 1 MiB".into());
    }
    let byte =
        incremental::byte_offset(text, line, character).ok_or("Posição UTF-16 Rust inválida")?;
    let mut budget = Budget::new(cancel, ValidationLimits::default());
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| e.to_string())?;
    let tree = parse(&mut parser, text, None, &budget)
        .ok_or("Descrição de símbolo Rust cancelada ou excedida")?;
    let Some(mut node) = tree.root_node().descendant_for_byte_range(byte, byte) else {
        return Ok(K::Unknown);
    };
    if protected(node) {
        return Ok(K::Unknown);
    }
    loop {
        budget
            .node()
            .map_err(|_| "Descrição de símbolo Rust cancelada ou excedida")?;
        match node.kind() {
            "type_identifier" => return Ok(K::Class),
            "field_identifier" => {
                return Ok(
                    if node.parent().is_some_and(|p| {
                        p.kind() == "field_expression"
                            && p.parent().is_some_and(|g| g.kind() == "call_expression")
                    }) {
                        K::Method
                    } else {
                        K::Property
                    },
                );
            }
            "function_item" | "function_signature_item" => {
                return Ok(
                    if node
                        .parent()
                        .and_then(|p| p.parent())
                        .is_some_and(|p| matches!(p.kind(), "impl_item" | "trait_item"))
                    {
                        K::Method
                    } else {
                        K::Function
                    },
                );
            }
            "parameter" => return Ok(K::Parameter),
            "let_declaration" => return Ok(K::Variable),
            "call_expression" => return Ok(K::Function),
            "source_file" | "block" => return Ok(K::Unknown),
            _ => {}
        }
        let Some(parent) = node.parent() else {
            return Ok(K::Unknown);
        };
        node = parent;
    }
}
fn declaration(mut node: Node<'_>) -> bool {
    let range = node.byte_range();
    while let Some(parent) = node.parent() {
        let field_contains = |field| {
            parent
                .child_by_field_name(field)
                .is_some_and(|n| n.start_byte() <= range.start && n.end_byte() >= range.end)
        };
        if matches!(
            parent.kind(),
            "function_item"
                | "function_signature_item"
                | "struct_item"
                | "enum_item"
                | "trait_item"
                | "type_item"
                | "const_item"
                | "static_item"
                | "enum_variant"
                | "field_declaration"
                | "type_parameter"
        ) && field_contains("name")
        {
            return true;
        }
        if matches!(
            parent.kind(),
            "let_declaration" | "parameter" | "closure_parameters"
        ) && (field_contains("pattern") || parent.kind() == "closure_parameters")
        {
            return true;
        }
        if matches!(
            parent.kind(),
            "block"
                | "source_file"
                | "expression_statement"
                | "call_expression"
                | "field_expression"
                | "scoped_identifier"
                | "scoped_type_identifier"
                | "type_arguments"
        ) {
            return false;
        }
        node = parent;
    }
    false
}
fn protected(mut node: Node<'_>) -> bool {
    loop {
        if matches!(
            node.kind(),
            "token_tree"
                | "token_binding_pattern"
                | "attribute_item"
                | "inner_attribute_item"
                | "macro_definition"
                | "shorthand_field_identifier"
        ) {
            return true;
        }
        let Some(parent) = node.parent() else {
            return false;
        };
        node = parent;
    }
}
fn parse(parser: &mut Parser, text: &str, old: Option<&Tree>, budget: &Budget<'_>) -> Option<Tree> {
    incremental::parse(parser, text, old, budget)
}
fn shape(before: Node<'_>, after: Node<'_>, budget: &mut Budget<'_>) -> Result<(), IssueKind> {
    let mut nodes = vec![(before, after)];
    while let Some((a, b)) = nodes.pop() {
        budget.node()?;
        if a.kind() != b.kind()
            || a.child_count() != b.child_count()
            || a.is_named() != b.is_named()
        {
            return Err(IssueKind::UnsupportedEdit);
        }
        for i in 0..a.child_count() {
            nodes.push((a.child(i).unwrap(), b.child(i).unwrap()));
        }
    }
    Ok(())
}
pub fn validate(
    snapshots: &[RenameSnapshot],
    origin: &Path,
    old_name: &str,
    new_name: &str,
    cancel: &AtomicBool,
) -> RenameValidation {
    let mut result = report();
    if let Err(message) = validate_candidate(new_name) {
        issue(&mut result, origin, IssueKind::UnsupportedEdit, message);
        return result;
    }
    if old_name.is_empty() || base(old_name) == base(new_name) {
        issue(
            &mut result,
            origin,
            IssueKind::UnsupportedEdit,
            "Nome Rust anterior ausente ou novo nome idêntico",
        );
        return result;
    }
    let mut budget = Budget::new(cancel, ValidationLimits::default());
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        issue(
            &mut result,
            origin,
            IssueKind::Parser,
            "Parser Rust indisponível",
        );
        return result;
    }
    let mut changed = 0usize;
    let mut size = 0usize;
    for snapshot in snapshots {
        if snapshot.path.extension().is_none_or(|e| e != "rs") {
            if !snapshot.edits.is_empty() {
                issue(
                    &mut result,
                    &snapshot.path,
                    IssueKind::UnsupportedLanguage,
                    "Rename Rust contém edição fora de fonte .rs",
                );
            }
            continue;
        }
        size += snapshot.before.len() + snapshot.after.len();
        if snapshot.before.len() > 1024 * 1024
            || snapshot.after.len() > 1024 * 1024
            || size > 8 * 1024 * 1024
            || result.checked_files >= 64
        {
            issue(
                &mut result,
                &snapshot.path,
                IssueKind::Limit,
                "Rename Rust excede limites de corpus",
            );
            break;
        }
        let before = match parse(&mut parser, &snapshot.before, None, &budget) {
            Some(tree) if !tree.root_node().has_error() => tree,
            _ => {
                issue(
                    &mut result,
                    &snapshot.path,
                    IssueKind::SyntaxBefore,
                    "Fonte Rust inválido, cancelado ou excedido antes da refatoração",
                );
                continue;
            }
        };
        result.checked_files += 1;
        let edits = match incremental::prepare_edits(snapshot, new_name, &budget) {
            Ok(edits) => edits,
            Err(message) => {
                issue(
                    &mut result,
                    &snapshot.path,
                    IssueKind::UnsupportedEdit,
                    message,
                );
                continue;
            }
        };
        let mut invalid = false;
        for edit in &edits {
            let Some(node) = before
                .root_node()
                .descendant_for_byte_range(edit.before.start, edit.before.end)
            else {
                invalid = true;
                break;
            };
            if !identifier(node)
                || protected(node)
                || node.byte_range() != edit.before
                || base(&snapshot.before[edit.before.clone()]) != base(old_name)
            {
                invalid = true;
                break;
            }
            result.checked_declarations += usize::from(declaration(node));
        }
        if invalid {
            issue(
                &mut result,
                &snapshot.path,
                IssueKind::UnsupportedEdit,
                "Proposta Rust deve alterar exatamente identificadores; string/comentário/macro/shorthand estrutural não certificado",
            );
            continue;
        }
        let mut cursor = before.walk();
        loop {
            let node = cursor.node();
            if let Err(kind) = budget.node() {
                issue(
                    &mut result,
                    &snapshot.path,
                    kind,
                    "Validação AST Rust cancelada ou excedida",
                );
                invalid = true;
                break;
            }
            if (identifier(node) || node.kind() == "primitive_type")
                && base(&snapshot.before[node.byte_range()]) == base(new_name)
                && !edits.iter().any(|e| e.before == node.byte_range())
                && result.conflicts.len() < 64
            {
                result.conflicts.push(Conflict {
                        path:snapshot.path.clone(),range:incremental::text_range(&snapshot.before,node.byte_range()),
                        kind:ConflictKind::DuplicateBinding,related:None,container_name:None,
                        message:"Nome novo já existe como identificador no corpus Rust; colisão/captura não pode ser descartada conservadoramente".into(),
                    });
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    break;
                }
            }
            if cursor.node() == before.root_node() {
                break;
            }
        }
        if invalid {
            continue;
        }
        let mut edited = before.clone();
        for edit in edits.iter().rev() {
            edited.edit(&edit.input_edit(&snapshot.before));
        }
        let after = match parse(&mut parser, &snapshot.after, Some(&edited), &budget) {
            Some(tree) if !tree.root_node().has_error() => tree,
            _ => {
                issue(
                    &mut result,
                    &snapshot.path,
                    IssueKind::UnsupportedEdit,
                    "Fonte Rust após rename inválido, cancelado ou excedido",
                );
                continue;
            }
        };
        if shape(before.root_node(), after.root_node(), &mut budget).is_err() {
            issue(
                &mut result,
                &snapshot.path,
                IssueKind::UnsupportedEdit,
                "Proposta Rust muda estrutura AST em vez de apenas nomes",
            );
            continue;
        }
        changed += edits.len();
        result.incremental_reparses += usize::from(!edits.is_empty());
    }
    if changed == 0 || result.checked_declarations == 0 {
        issue(
            &mut result,
            origin,
            IssueKind::UnsupportedEdit,
            "Proposta Rust não contém declaração e referências alteradas verificáveis",
        );
    }
    result.coverage = if !result.issues.is_empty() {
        Coverage::Unsupported
    } else if !result.conflicts.is_empty() {
        Coverage::Rejected
    } else {
        Coverage::Verified
    };
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn snapshot(text: &str, old: &str, new: &str, which: usize) -> RenameSnapshot {
        let start = text.match_indices(old).nth(which).unwrap().0;
        let mut after = text.to_owned();
        after.replace_range(start..start + old.len(), new);
        RenameSnapshot {
            path: "lib.rs".into(),
            before: Arc::from(text),
            after: Arc::from(after),
            edits: vec![crate::RenameEdit {
                range: incremental::text_range(text, start..start + old.len()),
                new_text: new.into(),
            }],
        }
    }
    #[test]
    fn rust_candidate_parser_accepts_unicode_raw_and_rejects_keywords_and_punctuation() {
        for name in ["ação", "forecast", "r#type", "日本語"] {
            assert!(validate_candidate(name).is_ok(), "{name}");
        }
        for name in ["fn", "Self", "self", "_", "$rain", "r#self", "x y", "a::b"] {
            assert!(validate_candidate(name).is_err(), "{name}");
        }
    }
    #[test]
    fn ast_certifies_identifier_only_and_rejects_string_macro_structure_and_capture() {
        let cancel = AtomicBool::new(false);
        let good = snapshot(
            "// 😀\r\nfn rainfall() {}\r\n",
            "rainfall",
            "precipitação",
            0,
        );
        assert_eq!(
            validate(
                &[good],
                Path::new("lib.rs"),
                "rainfall",
                "precipitação",
                &cancel
            )
            .coverage,
            Coverage::Verified
        );
        let string = snapshot(
            "fn rainfall(){ let _s=\"rainfall\"; }",
            "rainfall",
            "forecast",
            1,
        );
        assert_eq!(
            validate(
                &[string],
                Path::new("lib.rs"),
                "rainfall",
                "forecast",
                &cancel
            )
            .coverage,
            Coverage::Unsupported
        );
        let collision = snapshot(
            "fn rainfall() {} fn forecast() {}",
            "rainfall",
            "forecast",
            0,
        );
        assert_eq!(
            validate(
                &[collision],
                Path::new("lib.rs"),
                "rainfall",
                "forecast",
                &cancel
            )
            .coverage,
            Coverage::Rejected
        );
        let macro_source = snapshot(
            "fn main(){ macro_rules! rainfall { () => {} } }",
            "rainfall",
            "forecast",
            0,
        );
        assert_eq!(
            validate(
                &[macro_source],
                Path::new("lib.rs"),
                "rainfall",
                "forecast",
                &cancel
            )
            .coverage,
            Coverage::Unsupported
        );
    }
    #[test]
    fn forged_after_missing_declaration_invalid_syntax_and_cancel_never_verify() {
        let cancel = AtomicBool::new(false);
        let mut forged = snapshot("pub fn old() {}", "old", "new_name", 0);
        forged.after = Arc::from("pub fn new_name() {} // unrelated mutation");
        assert_eq!(
            validate(&[forged], Path::new("lib.rs"), "old", "new_name", &cancel).coverage,
            Coverage::Unsupported
        );
        let usage_only = snapshot("fn main(){ old(); }", "old", "new_name", 0);
        assert_eq!(
            validate(
                &[usage_only],
                Path::new("lib.rs"),
                "old",
                "new_name",
                &cancel
            )
            .coverage,
            Coverage::Unsupported
        );
        let malformed = snapshot("fn old( {", "old", "new_name", 0);
        assert_eq!(
            validate(
                &[malformed],
                Path::new("lib.rs"),
                "old",
                "new_name",
                &cancel
            )
            .coverage,
            Coverage::Unsupported
        );
        let good = snapshot("pub fn old() {}", "old", "new_name", 0);
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            validate(&[good], Path::new("lib.rs"), "old", "new_name", &cancel).coverage,
            Coverage::Unsupported
        );
    }
}
