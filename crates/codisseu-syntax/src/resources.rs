//! AST-only class/file planning. Filesystem policy and transactional moves belong
//! to the application; this module never reads or modifies a file.
use crate::{
    RenameEdit, RenameSnapshot, ValidationLimits,
    ast::Language,
    incremental::{self, Budget, text_range},
};
use std::{
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicBool,
};
use tree_sitter::{Node, Parser};

#[derive(Clone, Debug)]
pub struct RenameResourceProposal {
    pub source: PathBuf,
    pub target: PathBuf,
    /// Additional edits against the original snapshots, never the post-symbol text.
    pub edits: Vec<(PathBuf, RenameEdit)>,
}
fn normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(component.as_os_str()),
        }
    }
    result
}
fn text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}
fn plain_literal<'a>(node: Node<'_>, source: &'a str) -> Option<(&'a str, std::ops::Range<usize>)> {
    if !matches!(node.kind(), "string" | "encapsed_string") {
        return None;
    }
    let value = text(node, source);
    let quote = value.as_bytes().first()?;
    if !matches!(quote, b'\'' | b'"') || value.as_bytes().last() != Some(quote) || value.len() < 2 {
        return None;
    }
    let inner = &value[1..value.len() - 1];
    if inner.contains(['\\', '\n', '\r', '\0', '$']) {
        return None;
    }
    Some((inner, node.start_byte() + 1..node.end_byte() - 1))
}
fn child_expression(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|n| n.kind() != "comment")
}
fn php_literal<'a>(node: Node<'a>, source: &str) -> Option<(Node<'a>, bool)> {
    if matches!(node.kind(), "string" | "encapsed_string") {
        return Some((node, false));
    }
    if node.kind() == "parenthesized_expression" {
        return php_literal(child_expression(node)?, source);
    }
    if node.kind() == "binary_expression" {
        let left = node.child_by_field_name("left")?;
        let right = node.child_by_field_name("right")?;
        let operator = node.child_by_field_name("operator")?;
        if text(left, source) == "__DIR__"
            && text(operator, source) == "."
            && matches!(right.kind(), "string" | "encapsed_string")
        {
            return Some((right, true));
        }
    }
    None
}
fn module_candidates(path: &Path, typescript: bool) -> Vec<(PathBuf, bool)> {
    let mut candidates = vec![(path.to_path_buf(), false)];
    if typescript {
        if path.extension().is_none() {
            for extension in ["ts", "tsx", "d.ts", "js", "jsx"] {
                candidates.push((path.with_extension(extension), false));
                candidates.push((path.join(format!("index.{extension}")), true));
            }
        } else if path.extension().is_some_and(|e| e == "js" || e == "jsx") {
            for extension in ["ts", "tsx", "d.ts", "js", "jsx"] {
                let candidate = path.with_extension(extension);
                if candidate != path {
                    candidates.push((candidate, false));
                }
            }
        }
    }
    candidates
}
fn rewritten_module(
    raw: &str,
    owner: &Path,
    source: &Path,
    target: &Path,
    typescript: bool,
    dir_relative: bool,
    known: &[RenameSnapshot],
) -> Result<Option<String>, String> {
    let base = owner.parent().ok_or("Arquivo sem diretório pai")?;
    let normalized_source = normalized(source);
    let raw_path = if dir_relative {
        base.join(raw.trim_start_matches('/'))
    } else if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        base.join(raw)
    };
    let path = normalized(&raw_path);
    let candidates = module_candidates(&path, typescript);
    let matches: Vec<_> = candidates
        .iter()
        .filter(|(p, _)| *p == normalized_source || known.iter().any(|s| normalized(&s.path) == *p))
        .collect();
    if !matches.iter().any(|(p, _)| *p == normalized_source) {
        return Ok(None);
    }
    if matches.len() != 1 {
        return Err("Módulo relativo possui resolução ambígua no corpus".into());
    }
    if typescript && !raw.starts_with('.') {
        return Err("Módulo TypeScript não relativo exige resolução de configuração".into());
    }
    if !typescript
        && !dir_relative
        && !Path::new(raw).is_absolute()
        && !raw.starts_with("./")
        && !raw.starts_with("../")
    {
        return Err("Include PHP sem __DIR__ ou prefixo relativo depende de include_path".into());
    }
    let new_stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Novo basename não UTF-8")?;
    let declaration = typescript
        && source
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".d.ts"));
    let new_stem = if declaration && !raw.ends_with(".d.ts") {
        new_stem
            .strip_suffix(".d")
            .ok_or("Rename deve preservar extensão composta .d.ts")?
    } else {
        new_stem
    };
    let new_text = if matches[0].1 {
        format!("{}/{new_stem}", raw.trim_end_matches('/'))
    } else {
        let last = raw.rfind('/').map_or(0, |i| i + 1);
        let suffix = Path::new(&raw[last..])
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| format!(".{s}"))
            .unwrap_or_default();
        format!("{}{new_stem}{suffix}", &raw[..last])
    };
    // Prove that the rewritten module still resolves only to the moved file;
    // a sibling with another JS/TS extension must not capture the new path.
    let new_path = normalized(&if dir_relative {
        base.join(new_text.trim_start_matches('/'))
    } else if Path::new(&new_text).is_absolute() {
        PathBuf::from(&new_text)
    } else {
        base.join(&new_text)
    });
    let normalized_target = normalized(target);
    let candidates_after = module_candidates(&new_path, typescript);
    let resolved_after = candidates_after
        .iter()
        .filter(|(p, _)| {
            *p == normalized_target
                || known
                    .iter()
                    .any(|s| normalized(&s.path) != normalized_source && normalized(&s.path) == *p)
        })
        .collect::<Vec<_>>();
    if resolved_after.len() != 1 || resolved_after[0].0 != normalized_target {
        return Err("Novo módulo relativo possui resolução ambígua no corpus".into());
    }
    Ok(Some(new_text))
}

// Resource planning consumes proposals already proved by a semantic engine.
// Prefix/suffix edits from shorthand/import aliases are valid inputs here; this
// checks only their exact snapshot reconstruction, never approves their binding.
fn proposal_edits(
    snapshot: &RenameSnapshot,
    budget: &mut Budget<'_>,
) -> Result<Vec<incremental::ByteEdit>, String> {
    let mut edits = Vec::with_capacity(snapshot.edits.len());
    for edit in &snapshot.edits {
        budget
            .node()
            .map_err(|_| "Planejamento cancelado ou excedido")?;
        let r = &edit.range;
        let start = incremental::byte_offset(&snapshot.before, r.start_line, r.start_character)
            .ok_or("Início UTF-16 inválido")?;
        let end = incremental::byte_offset(&snapshot.before, r.end_line, r.end_character)
            .ok_or("Fim UTF-16 inválido")?;
        if start >= end || edit.new_text.len() > 4096 {
            return Err("Edição de recurso vazia ou excedida".into());
        }
        edits.push(incremental::ByteEdit {
            before: start..end,
            after: 0..0,
            new_text: edit.new_text.clone(),
        });
    }
    edits.sort_by_key(|e| e.before.start);
    if edits
        .windows(2)
        .any(|p| p[0].before.end > p[1].before.start)
    {
        return Err("Edições sobrepostas no planejamento de recurso".into());
    }
    let mut after = snapshot.before.to_string();
    for edit in edits.iter().rev() {
        budget
            .node()
            .map_err(|_| "Planejamento cancelado ou excedido")?;
        after.replace_range(edit.before.clone(), &edit.new_text);
    }
    if after != snapshot.after.as_ref() {
        return Err("Texto posterior não corresponde às edições de recurso".into());
    }
    Ok(edits)
}

/// Identifies an edited top-level class whose file stem matches its former name,
/// and plans exact AST literal updates to that same-folder file rename.
pub(crate) fn plan(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    plan_with_target(snapshots, new_name, None, false, cancel, limits)
}
/// Same-directory source-file basename. Reject characters requiring escaping in
/// supported module literals and names invalid on the supported desktop paths.
pub fn validate_file_rename_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || name.trim() != name
        || name.ends_with('.')
        || name.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '/' | '\\' | ':' | '*' | '?' | '\"' | '\'' | '`' | '$' | '<' | '>' | '|'
                )
        })
    {
        return Err(
            "Nome de arquivo inválido; use um basename sem separadores ou caracteres escapados",
        );
    }
    Ok(())
}

pub(crate) fn named(
    snapshots: &[RenameSnapshot],
    source: &Path,
    target: &Path,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    if source.parent() != target.parent() || source.extension() != target.extension() {
        return Err("Arquivo relacionado exige mesmo diretório/extensão".into());
    }
    let declaration = |path: &Path| {
        path.file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".d.ts"))
    };
    if declaration(source) != declaration(target) {
        return Err("Rename deve preservar extensão composta .d.ts".into());
    }
    let stem = target
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Nome de arquivo inválido")?;
    plan_with_target(
        snapshots,
        stem,
        Some((source, target)),
        false,
        cancel,
        limits,
    )
}
pub(crate) fn candidate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    plan_with_target(snapshots, new_name, None, true, cancel, limits)
}
fn plan_with_target(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    explicit: Option<(&Path, &Path)>,
    candidate_only: bool,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    if let Some((_, target)) = explicit {
        validate_file_rename_name(
            target
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("Nome de arquivo inválido")?,
        )?;
    } else {
        crate::validate_rename_candidate(new_name)?;
    }
    if snapshots.is_empty()
        || snapshots.len() > 64
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots
            .iter()
            .map(|s| s.before.len() + s.after.len())
            .sum::<usize>()
            > 8 * 1024 * 1024
    {
        return Err("Corpus excede limites de planejamento de arquivo".into());
    }
    let mut budget = Budget::new(cancel, limits);
    let mut trees = Vec::new();
    let mut selected = Vec::new();
    let mut paths = std::collections::HashSet::new();
    for (file_id, snapshot) in snapshots.iter().enumerate() {
        budget
            .node()
            .map_err(|_| "Planejamento cancelado ou excedido")?;
        if !paths.insert(&snapshot.path) {
            return Err("Snapshot duplicado no planejamento de arquivo".into());
        }
        let edits = proposal_edits(snapshot, &mut budget)?;
        let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
            Some("php") => Language::Php,
            Some("ts" | "tsx" | "js" | "jsx") => Language::TypeScript,
            _ => continue,
        };
        let grammar = if snapshot
            .path
            .extension()
            .is_some_and(|e| e == "tsx" || e == "jsx")
        {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        } else {
            language.grammar()
        };
        let mut parser = Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|_| "Gramática incompatível")?;
        let tree =
            incremental::parse(&mut parser, &snapshot.before, None, &budget).ok_or_else(|| {
                format!(
                    "Parser de recursos interrompido: {:?}",
                    budget.check().err()
                )
            })?;
        if tree.root_node().has_error() {
            return Err("Snapshot contém ERROR/MISSING no planejamento de arquivo".into());
        }
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            budget
                .node()
                .map_err(|k| format!("Índice de recursos interrompido: {k:?}"))?;
            if node.kind() == "class_declaration"
                && let Some(name) = node.child_by_field_name("name")
                && edits
                    .iter()
                    .any(|e| e.before == name.byte_range() && e.new_text == new_name)
            {
                let global = node.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "program" | "export_statement" | "namespace_definition"
                    ) || (p.kind() == "compound_statement"
                        && p.parent()
                            .is_some_and(|p| p.kind() == "namespace_definition"))
                });
                if !global {
                    return Err("Classe aninhada não define operação de arquivo".into());
                }
                if snapshot.path.file_stem().and_then(|s| s.to_str())
                    == Some(text(name, &snapshot.before))
                {
                    selected.push(file_id);
                }
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
        trees.push((file_id, language, tree));
    }
    if let Some((source, _)) = explicit {
        selected = vec![
            snapshots
                .iter()
                .position(|s| s.path == source)
                .ok_or("Arquivo relacionado fora do corpus")?,
        ];
    }
    if selected.is_empty() {
        return Ok(None);
    }
    if selected.len() != 1 {
        return Err("Operação exige exatamente uma classe/arquivo correspondente".into());
    }
    let source = snapshots[selected[0]].path.clone();
    let mut target = source.with_file_name(new_name);
    if let Some(extension) = source.extension() {
        target.set_extension(extension);
    }
    if let Some((_, chosen)) = explicit {
        target = chosen.to_path_buf();
    }
    if source == target {
        return Ok(None);
    }
    if candidate_only {
        return Ok(Some(RenameResourceProposal {
            source,
            target,
            edits: vec![],
        }));
    }
    if snapshots.iter().any(|s| s.path == target) {
        return Err("Arquivo de destino já pertence ao corpus".into());
    }
    let source_is_php = source.extension().is_some_and(|s| s == "php");
    let mut edits = Vec::new();
    for (file_id, language, tree) in &trees {
        let snapshot = &snapshots[*file_id];
        if (*language == Language::Php) != source_is_php {
            continue;
        }
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            budget
                .node()
                .map_err(|k| format!("Varredura AST de módulos interrompida: {k:?}"))?;
            let literal = if *language == Language::TypeScript
                && matches!(node.kind(), "import_statement" | "export_statement")
            {
                node.child_by_field_name("source").map(|n| (n, false))
            } else if *language == Language::TypeScript
                && node.kind() == "call_expression"
                && node
                    .child_by_field_name("function")
                    .is_some_and(|n| matches!(text(n, &snapshot.before), "import" | "require"))
            {
                let args = node
                    .child_by_field_name("arguments")
                    .ok_or("Import/require sem argumentos")?;
                let value = child_expression(args).ok_or("Import/require vazio")?;
                Some((value, false))
            } else if *language == Language::Php
                && matches!(
                    node.kind(),
                    "require_expression"
                        | "require_once_expression"
                        | "include_expression"
                        | "include_once_expression"
                )
            {
                let expression = child_expression(node).ok_or("Include/require sem expressão")?;
                Some(
                    php_literal(expression, &snapshot.before).ok_or(
                        "Include/require PHP dinâmico impede provar referências de arquivo",
                    )?,
                )
            } else {
                None
            };
            if let Some((literal, dir_relative)) = literal {
                let (raw, range) = plain_literal(literal, &snapshot.before)
                    .ok_or("Módulo/include com escapes ou expressão dinâmica não comprovado")?;
                if *language == Language::TypeScript && !raw.starts_with('.') {
                    return Err(
                        "Módulo TypeScript não relativo exige resolução de configuração antes de mover arquivos"
                            .into(),
                    );
                }
                if let Some(new_text) = rewritten_module(
                    raw,
                    &snapshot.path,
                    &source,
                    &target,
                    *language == Language::TypeScript,
                    dir_relative,
                    snapshots,
                )? {
                    edits.push((
                        snapshot.path.clone(),
                        RenameEdit {
                            range: text_range(&snapshot.before, range),
                            new_text,
                        },
                    ));
                }
            }
            let mut c = node.walk();
            stack.extend(node.named_children(&mut c));
        }
    }
    if edits.len() > 1024 {
        return Err("Operação de arquivo excede 1024 edições".into());
    }
    edits.sort_by(|a, b| {
        (&a.0, a.1.range.start_line, a.1.range.start_character).cmp(&(
            &b.0,
            b.1.range.start_line,
            b.1.range.start_character,
        ))
    });
    Ok(Some(RenameResourceProposal {
        source,
        target,
        edits,
    }))
}

/// A PHP class/trait/interface/enum whose name implies its filename must use the
/// coordinated class workflow; arbitrary file rename cannot certify PSR-4.
pub fn validate_standalone_file_source(
    path: &Path,
    source: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<(), String> {
    if path.extension().is_none_or(|e| e != "php") {
        return Ok(());
    }
    if source.len() > 1024 * 1024 {
        return Err("Arquivo excede 1 MiB".into());
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("Nome PHP inválido")?;
    let mut budget = Budget::new(cancel, limits);
    let mut parser = Parser::new();
    parser
        .set_language(&Language::Php.grammar())
        .map_err(|e| e.to_string())?;
    let tree = incremental::parse(&mut parser, source, None, &budget)
        .ok_or("Inspeção PHP cancelada ou limitada")?;
    if tree.root_node().has_error() {
        return Err("Arquivo PHP contém sintaxe inválida".into());
    }
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        budget
            .node()
            .map_err(|_| "Inspeção PHP cancelada ou limitada")?;
        if matches!(
            node.kind(),
            "class_declaration"
                | "trait_declaration"
                | "interface_declaration"
                | "enum_declaration"
        ) && node
            .child_by_field_name("name")
            .is_some_and(|n| text(n, source).eq_ignore_ascii_case(stem))
        {
            return Err("Arquivo PHP corresponde a uma declaração nominal; use Rename da classe/trait/interface/enum para preservar autoload".into());
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    Ok(())
}
