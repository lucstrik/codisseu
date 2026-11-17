//! Basedpyright owns semantic bindings. This independent Python AST audit only
//! accepts exact identifier replacements with identical concrete tree shape.
//! Existing destination identifiers anywhere in the bounded corpus are rejected
//! conservatively, including NFKC-equivalent names (Python identifier semantics).
use crate::{
    Conflict, ConflictKind, Coverage, IssueKind, RenameSnapshot, RenameValidation, ValidationIssue,
    ValidationLimits, ValidationScope,
    incremental::{self, Budget},
};
use std::{path::Path, sync::atomic::AtomicBool};
use tree_sitter::{Node, Parser, Tree};
use unicode_normalization::UnicodeNormalization;
fn report() -> RenameValidation {
    RenameValidation {
        coverage: Coverage::Unsupported,
        scope: ValidationScope::PythonAnalyzerBindings,
        conflicts: vec![], issues: vec![], checked_declarations: 0,
        checked_override_pairs: 0, related_methods: vec![], checked_files: 0,
        incremental_reparses: 0,
        note: "Bindings fornecidos pelo Basedpyright; tokens e estrutura Python conferidos por AST antes/depois. Nome novo preexistente no corpus é recusado conservadoramente para impedir captura/colisão. Não é typechecking global nem prova de completude do provider, imports dinâmicos, monkey patching ou dependências externas.".into(),
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

fn python(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi" | "pyw")
    )
}
fn identifier(node: Node<'_>) -> bool {
    node.kind() == "identifier"
}
pub fn validate_candidate(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 1024
        || name == "__debug__"
        || name.nfkc().ne(name.chars())
        || name.chars().enumerate().any(|(i, c)| {
            !(c == '_'
                || if i == 0 {
                    unicode_ident::is_xid_start(c)
                } else {
                    unicode_ident::is_xid_continue(c)
                })
        })
    {
        return Err("Nome Python deve ser um identificador Unicode válido em forma NFKC");
    }
    if matches!(
        name,
        "False"
            | "None"
            | "True"
            | "and"
            | "as"
            | "assert"
            | "async"
            | "await"
            | "break"
            | "class"
            | "continue"
            | "def"
            | "del"
            | "elif"
            | "else"
            | "except"
            | "finally"
            | "for"
            | "from"
            | "global"
            | "if"
            | "import"
            | "in"
            | "is"
            | "lambda"
            | "nonlocal"
            | "not"
            | "or"
            | "pass"
            | "raise"
            | "return"
            | "try"
            | "while"
            | "with"
            | "yield"
    ) {
        return Err("Nome é palavra reservada Python");
    }
    let text = format!("{name} = None\n");
    let mut p = Parser::new();
    p.set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|_| "Parser Python indisponível")?;
    let t = p.parse(&text, None).ok_or("Parser Python indisponível")?;
    let n = t
        .root_node()
        .descendant_for_byte_range(0, name.len())
        .ok_or("Identificador inválido")?;
    if t.root_node().has_error() || !identifier(n) || n.byte_range() != (0..name.len()) {
        return Err("Novo nome Python inválido");
    }
    Ok(())
}
fn protected(mut node: Node<'_>) -> bool {
    // An expression inside an f-string interpolation is ordinary code; literal
    // string_content/comment leaves cannot satisfy the exact identifier check.
    loop {
        if matches!(node.kind(), "ERROR" | "comment") {
            return true;
        }
        let Some(p) = node.parent() else { return false };
        node = p;
    }
}
fn declaration(mut node: Node<'_>) -> bool {
    let range = node.byte_range();
    while let Some(p) = node.parent() {
        let field = |f| {
            p.child_by_field_name(f)
                .is_some_and(|n| n.start_byte() <= range.start && n.end_byte() >= range.end)
        };
        if matches!(p.kind(), "function_definition" | "class_definition") && field("name") {
            return true;
        }
        if matches!(
            p.kind(),
            "assignment" | "named_expression" | "for_statement"
        ) && field("left")
        {
            return true;
        }
        if matches!(p.kind(), "parameters" | "lambda_parameters") {
            return true;
        }
        if p.kind() == "aliased_import" && field("alias") {
            return true;
        }
        if matches!(
            p.kind(),
            "module" | "block" | "call" | "attribute" | "expression_statement"
        ) {
            return false;
        }
        node = p;
    }
    false
}
pub(crate) fn is_method(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    for _ in 0..8 {
        let Some(n) = parent else { return false };
        match n.kind() {
            "class_definition" => return true,
            "function_definition" | "module" => return false,
            _ => {}
        }
        parent = n.parent();
    }
    false
}
pub fn classify(
    text: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
) -> Result<crate::RenameSymbolKind, String> {
    use crate::RenameSymbolKind as K;
    if text.len() > 1024 * 1024 {
        return Err("Descrição Python excede1MiB".into());
    }
    let at =
        incremental::byte_offset(text, line, character).ok_or("Posição UTF-16 Python inválida")?;
    let mut budget = Budget::new(cancel, ValidationLimits::default());
    let mut p = Parser::new();
    p.set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| e.to_string())?;
    let t = parse(&mut p, text, None, &budget).ok_or("Descrição Python cancelada ou excedida")?;
    let Some(mut n) = t.root_node().descendant_for_byte_range(at, at) else {
        return Ok(K::Unknown);
    };
    if !identifier(n) {
        return Ok(K::Unknown);
    }
    loop {
        budget
            .node()
            .map_err(|_| "Descrição Python cancelada ou excedida")?;
        match n.kind() {
            "class_definition" => return Ok(K::Class),
            "function_definition" => {
                return Ok(if is_method(n) { K::Method } else { K::Function });
            }
            "parameters" | "lambda_parameters" => return Ok(K::Parameter),
            "attribute" => return Ok(K::Property),
            "assignment" | "module" | "block" => return Ok(K::Variable),
            "call" => return Ok(K::Function),
            _ => {}
        }
        let Some(p) = n.parent() else {
            return Ok(K::Unknown);
        };
        n = p;
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
    if old_name.is_empty() || old_name == new_name {
        issue(
            &mut result,
            origin,
            IssueKind::UnsupportedEdit,
            "Nome Python anterior ausente ou novo nome idêntico",
        );
        return result;
    }
    let mut budget = Budget::new(cancel, ValidationLimits::default());
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .is_err()
    {
        issue(
            &mut result,
            origin,
            IssueKind::Parser,
            "Parser Python indisponível",
        );
        return result;
    }
    let mut changed = 0usize;
    let mut size = 0usize;
    for snapshot in snapshots {
        if !python(&snapshot.path) {
            if !snapshot.edits.is_empty() {
                issue(
                    &mut result,
                    &snapshot.path,
                    IssueKind::UnsupportedLanguage,
                    "Rename Python contém edição fora de fonte .py/.pyi/.pyw",
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
                "Rename Python excede limites de corpus",
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
                    "Fonte Python inválido, cancelado ou excedido antes da refatoração",
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
                || &snapshot.before[edit.before.clone()] != old_name
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
                "Proposta Python deve alterar exatamente identificadores; string/comentário/transformação estrutural não certificado",
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
                    "Validação AST Python cancelada ou excedida",
                );
                invalid = true;
                break;
            }
            if identifier(node)
                && snapshot.before[node.byte_range()]
                    .nfkc()
                    .eq(new_name.chars())
                && !edits.iter().any(|e| e.before == node.byte_range())
                && result.conflicts.len() < 64
            {
                result.conflicts.push(Conflict {
                        path:snapshot.path.clone(),range:incremental::text_range(&snapshot.before,node.byte_range()),
                        kind:ConflictKind::DuplicateBinding,related:None,container_name:None,
                        message:"Nome novo já existe como identificador no corpus Python; colisão/captura não pode ser descartada conservadoramente".into(),
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
                    "Fonte Python após rename inválido, cancelado ou excedido",
                );
                continue;
            }
        };
        if shape(before.root_node(), after.root_node(), &mut budget).is_err() {
            issue(
                &mut result,
                &snapshot.path,
                IssueKind::UnsupportedEdit,
                "Proposta Python muda estrutura AST em vez de apenas nomes",
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
            "Proposta Python não contém declaração e referências alteradas verificáveis",
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
    fn snapshot(source: &str, needle: &str, new: &str, which: usize) -> RenameSnapshot {
        let at = source.match_indices(needle).nth(which).unwrap().0;
        let mut after = source.to_owned();
        after.replace_range(at..at + needle.len(), new);
        RenameSnapshot {
            path: "ação.py".into(),
            before: Arc::from(source),
            after: Arc::from(after),
            edits: vec![crate::RenameEdit {
                range: incremental::text_range(source, at..at + needle.len()),
                new_text: new.into(),
            }],
        }
    }
    #[test]
    fn candidate_accepts_unicode_soft_keywords_and_rejects_keywords_nfkc_aliases() {
        for name in ["ação", "日本語", "match", "case", "type", "_value"] {
            assert!(validate_candidate(name).is_ok(), "{name}");
        }
        for name in [
            "def",
            "True",
            "__debug__",
            "a.b",
            "x y",
            "r#type",
            "Kelvin",
            "ｘ",
        ] {
            assert!(validate_candidate(name).is_err(), "{name}");
        }
    }
    #[test]
    fn exact_identifier_shape_and_capture_validation_preserve_comments_strings_unicode() {
        let cancel = AtomicBool::new(false);
        let s = snapshot(
            "# 😀 rainfall\r\ndef rainfall(first: int):\r\n    return first\r\nlabel = 'rainfall'\r\n",
            "rainfall",
            "precipitação",
            1,
        );
        let r = validate(
            &[s],
            Path::new("ação.py"),
            "rainfall",
            "precipitação",
            &cancel,
        );
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        let s = snapshot(
            "def rainfall():\n    return 'rainfall'\n",
            "rainfall",
            "cloud",
            1,
        );
        assert_ne!(
            validate(&[s], Path::new("ação.py"), "rainfall", "cloud", &cancel).coverage,
            Coverage::Verified
        );
        let s = snapshot(
            "cloud = 0\ndef rainfall():\n    pass\n",
            "rainfall",
            "cloud",
            0,
        );
        assert_eq!(
            validate(&[s], Path::new("ação.py"), "rainfall", "cloud", &cancel).coverage,
            Coverage::Rejected
        );
    }
    #[test]
    fn nfkc_destination_collision_and_cancellation_cannot_be_certified() {
        let cancel = AtomicBool::new(false);
        let s = snapshot("Kelvin = 0\ndef rain():\n    pass\n", "rain", "Kelvin", 0);
        assert_eq!(
            validate(&[s], Path::new("ação.py"), "rain", "Kelvin", &cancel).coverage,
            Coverage::Rejected
        );
        let s = snapshot("def rain():\n    pass\n", "rain", "cloud", 0);
        cancel.store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(
            validate(&[s], Path::new("ação.py"), "rain", "cloud", &cancel).coverage,
            Coverage::Unsupported
        );
    }
}
