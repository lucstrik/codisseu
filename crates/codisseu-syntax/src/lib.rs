//! Class-member and conservatively resolved hierarchy checks over exact snapshots.
//!
//! `Verified` is local to the reported scope/corpus. The application entry point
//! audits supported lexical/receiver bindings. The method-only entry point is a
//! nominal component check. Neither proves compiler module resolution, general
//! type compatibility, runtime loading, corpus completeness or equivalence.
mod ast;
pub mod breadcrumbs;
mod contracts;
mod globals;
mod hierarchy;
pub mod navigation_symbols;
pub mod phpdoc;
pub mod python_rename;
pub mod rust_rename;
pub mod search_symbols;
pub use hierarchy::{
    AccessorRenameProposal, ClassInheritorRenameProposal, complete_php_named_argument_rename,
    plan_class_inheritor_renames, plan_property_accessor_renames,
};
mod incremental;
mod noncode;
pub use noncode::{
    RenameOptions, RenameSearchScope, plan_file_noncode_rename, plan_noncode_rename,
};
mod resources;
pub use resources::{
    RenameResourceProposal, validate_file_rename_name, validate_standalone_file_source,
};
mod rename_presentation;
pub use rename_presentation::{
    RenameEnclosingKind, RenameEnclosingSymbol, RenameOccurrenceKind, RenameOccurrencePresentation,
    rename_occurrence_presentation,
};
pub mod php_completion;
mod symbol_kind;
mod symbols;
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
pub use symbol_kind::{RenameSymbolKind, classify_rename_symbol};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TextRange {
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
}
#[derive(Clone, Debug)]
pub struct RenameEdit {
    pub range: TextRange,
    pub new_text: String,
}
#[derive(Clone, Debug)]
pub struct RenameSnapshot {
    pub path: PathBuf,
    pub before: Arc<str>,
    pub after: Arc<str>,
    pub edits: Vec<RenameEdit>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Coverage {
    Verified,
    Rejected,
    Unsupported,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ValidationScope {
    SameClassMemberNamespace,
    KnownHierarchyMemberNamespaces,
    LexicalBindings,
    KnownReceiverBindings,
    KnownGlobalBindings,
    KnownPropertyBindings,
    CompilerBindings,
    RustAnalyzerBindings,
    PythonAnalyzerBindings,
    ResourceReferences,
    ResourceOperationOnly,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConflictLocation {
    pub path: PathBuf,
    pub range: TextRange,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ConflictKind {
    DuplicateMember,
    InvalidSyntax,
    InheritedMember,
    OverrideChanged,
    DuplicateBinding,
    BindingChanged,
    MissingReference,
}
#[derive(Clone, Debug, Serialize)]
pub struct Conflict {
    pub path: PathBuf,
    pub range: TextRange,
    pub message: String,
    pub related: Option<ConflictLocation>,
    pub kind: ConflictKind,
    pub container_name: Option<String>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum IssueKind {
    UnsupportedLanguage,
    UnsupportedEdit,
    UnsupportedClass,
    SyntaxBefore,
    InvalidSnapshot,
    Limit,
    Cancelled,
    Parser,
}
#[derive(Clone, Debug, Serialize)]
pub struct ValidationIssue {
    pub path: PathBuf,
    pub range: Option<TextRange>,
    pub message: String,
    pub kind: IssueKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RelatedMethod {
    pub path: PathBuf,
    pub range: TextRange,
    pub container_name: String,
    pub name: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RenameValidation {
    pub coverage: Coverage,
    pub scope: ValidationScope,
    pub conflicts: Vec<Conflict>,
    pub issues: Vec<ValidationIssue>,
    pub checked_declarations: usize,
    /// Unique nominally related member pairs with equal supported explicit headers.
    pub checked_override_pairs: usize,
    pub related_methods: Vec<RelatedMethod>,
    pub checked_files: usize,
    pub incremental_reparses: usize,
    pub note: String,
}
#[derive(Clone, Copy, Debug)]
pub struct ValidationLimits {
    pub max_nodes: usize,
    pub max_duration: Duration,
}
impl Default for ValidationLimits {
    fn default() -> Self {
        Self {
            max_nodes: 250_000,
            max_duration: Duration::from_secs(2),
        }
    }
}
impl RenameValidation {
    fn new() -> Self {
        Self{coverage:Coverage::Unsupported,scope:ValidationScope::SameClassMemberNamespace,conflicts:vec![],issues:vec![],checked_declarations:0,checked_override_pairs:0,related_methods:vec![],checked_files:0,incremental_reparses:0,note:"Verificação local de membros da classe. Não verifica vinculação de referências, tipos, contratos, herança/traits ou descendentes fora dos snapshots fornecidos, nem ausência global de conflitos.".into()}
    }
    fn issue(
        &mut self,
        path: &std::path::Path,
        kind: IssueKind,
        range: Option<TextRange>,
        message: impl Into<String>,
    ) {
        if self.issues.len() < 256 {
            self.issues.push(ValidationIssue {
                path: path.into(),
                range,
                kind,
                message: message.into(),
            });
        }
    }
}
/// Bounds candidates to Unicode XID identifiers plus `_` and `$` (a conservative
/// subset of supported PHP/TypeScript identifiers). It never normalizes spelling.
/// This preliminary filter only checks lexical characters. The PHP/TypeScript parser
/// must still prove that each whole edit is precisely one method identifier.
pub fn validate_rename_candidate(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 1024 {
        return Err("Novo nome deve conter entre 1 e 1024 bytes");
    }
    for (index, ch) in name.chars().enumerate() {
        let valid = matches!(ch, '_' | '$')
            || if index == 0 {
                unicode_ident::is_xid_start(ch)
            } else {
                unicode_ident::is_xid_continue(ch)
            };
        if !valid || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}') {
            return Err(
                "Novo nome deve ser um identificador Unicode sem espaços, escapes ou pontuação",
            );
        }
    }
    Ok(())
}
/// Does not read files, modify snapshots, run tools, or apply edits. Call on a worker.
pub fn validate_method_rename(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
) -> RenameValidation {
    validate_method_rename_with_limits(snapshots, new_name, cancel, ValidationLimits::default())
}
pub fn validate_method_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> RenameValidation {
    validate_methods(snapshots, new_name, cancel, limits, false)
}
/// Full application check: lexical symbols or method namespaces plus bounded
/// receiver binding. Unknown receivers never receive `Verified` coverage.
pub fn validate_rename(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
) -> RenameValidation {
    validate_rename_with_limits(snapshots, new_name, cancel, ValidationLimits::default())
}
pub fn validate_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> RenameValidation {
    let started = std::time::Instant::now();
    // All dispatch passes share one overall time/node allowance.
    let pass = ValidationLimits {
        max_nodes: limits.max_nodes / 5,
        max_duration: limits.max_duration,
    };
    for validate in [
        validate_global_rename_with_limits,
        validate_property_rename_with_limits,
        validate_symbol_rename_with_limits,
        validate_trait_rename_with_limits,
    ] {
        if let Some(report) = validate(
            snapshots,
            new_name,
            cancel,
            ValidationLimits {
                max_duration: limits.max_duration.saturating_sub(started.elapsed()),
                ..pass
            },
        ) {
            return report;
        }
    }
    validate_methods(
        snapshots,
        new_name,
        cancel,
        ValidationLimits {
            max_nodes: limits.max_nodes - 4 * (limits.max_nodes / 5),
            max_duration: limits.max_duration.saturating_sub(started.elapsed()),
        },
        true,
    )
}
pub fn prepare_trait_rename_with_limits(
    path: &std::path::Path,
    text: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    hierarchy::prepare_traits(
        path,
        text,
        &TextRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character,
        },
        cancel,
        limits,
    )
}
pub fn propose_trait_rename_with_limits(
    snapshots: &[RenameSnapshot],
    origin_path: &std::path::Path,
    origin: &TextRange,
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<Vec<RenameSnapshot>>, String> {
    hierarchy::propose_traits(snapshots, origin_path, origin, new_name, cancel, limits)
}
pub fn validate_trait_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    hierarchy::validate_traits(snapshots, new_name, cancel, limits)
}
pub fn validate_global_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    globals::validate(snapshots, new_name, cancel, limits)
}
/// Discover a syntactic global symbol in one immutable buffer. The complete
/// binding/collision proof is deferred to the bounded semantic preview worker.
pub fn prepare_global_rename_with_limits(
    path: &std::path::Path,
    text: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    let text: Arc<str> = Arc::from(text);
    globals::prepare(
        &RenameSnapshot {
            path: path.into(),
            before: text.clone(),
            after: text,
            edits: vec![],
        },
        &TextRange {
            start_line: line,
            start_character: character,
            end_line: line,
            end_character: character,
        },
        cancel,
        limits,
    )
}
/// Independent semantic provider: edits come from resolved AST token identities,
/// never from textual substitution. Callers must validate and revalidate before Apply.
pub fn propose_global_rename_with_limits(
    snapshots: &[RenameSnapshot],
    origin_path: &std::path::Path,
    origin: &TextRange,
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<Vec<RenameSnapshot>>, String> {
    globals::propose(snapshots, origin_path, origin, new_name, cancel, limits)
}
pub fn validate_property_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    hierarchy::validate_properties(snapshots, new_name, cancel, limits)
}
pub fn validate_symbol_rename(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
) -> Option<RenameValidation> {
    validate_symbol_rename_with_limits(snapshots, new_name, cancel, ValidationLimits::default())
}
pub fn validate_symbol_rename_with_limits(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Option<RenameValidation> {
    symbols::validate(snapshots, new_name, cancel, limits)
}
fn validate_methods(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
    binding: bool,
) -> RenameValidation {
    let mut report = RenameValidation::new();
    let mut budget = incremental::Budget::new(cancel, limits);
    let fallback = snapshots
        .first()
        .map_or(std::path::Path::new(""), |s| s.path.as_path());
    let mut evidence = ast::Evidence::default();
    let mut corpus = Vec::new();
    let mut metadata_bytes = 0usize;
    if snapshots.is_empty() || snapshots.len() > 64 {
        report.issue(
            fallback,
            IssueKind::Limit,
            None,
            "Validação exige 1..64 snapshots",
        );
        return report;
    }
    let bytes: usize = snapshots
        .iter()
        .map(|s| s.before.len() + s.after.len())
        .sum();
    if bytes > 8 * 1024 * 1024
        || snapshots
            .iter()
            .any(|s| s.before.len() > 1024 * 1024 || s.after.len() > 1024 * 1024)
        || snapshots.iter().map(|s| s.edits.len()).sum::<usize>() > 1024
    {
        report.issue(
            fallback,
            IssueKind::Limit,
            None,
            "Limite de 1 MiB por texto, 8 MiB por conjunto e 1024 edições",
        );
        return report;
    }
    if let Err(message) = validate_rename_candidate(new_name) {
        report.issue(fallback, IssueKind::UnsupportedEdit, None, message);
        return report;
    }
    let mut paths = std::collections::HashSet::new();
    for snapshot in snapshots {
        if !paths.insert(&snapshot.path) {
            report.issue(
                &snapshot.path,
                IssueKind::InvalidSnapshot,
                None,
                "Caminho de snapshot duplicado",
            );
            continue;
        }
        if let Err(kind) = budget.check() {
            report.issue(
                &snapshot.path,
                kind,
                None,
                "Validação cancelada ou orçamento excedido",
            );
            break;
        }
        let language = match snapshot.path.extension().and_then(|s| s.to_str()) {
            Some("php") => ast::Language::Php,
            Some("ts") => ast::Language::TypeScript,
            _ => {
                if !snapshot.edits.is_empty() {
                    report.issue(
                        &snapshot.path,
                        IssueKind::UnsupportedLanguage,
                        None,
                        "Somente PHP e TypeScript independentes nesta fatia",
                    );
                }
                continue;
            }
        };
        let edits = match incremental::prepare_edits(snapshot, new_name, &budget) {
            Ok(edits) => edits,
            Err(message) => {
                report.issue(
                    &snapshot.path,
                    budget.check().err().unwrap_or(IssueKind::InvalidSnapshot),
                    None,
                    message,
                );
                continue;
            }
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&language.grammar()).is_err() {
            report.issue(
                &snapshot.path,
                IssueKind::Parser,
                None,
                "Gramática incompatível com o runtime",
            );
            continue;
        }
        let Some(mut tree) = incremental::parse(&mut parser, &snapshot.before, None, &budget)
        else {
            let kind = budget.check().err().unwrap_or(IssueKind::Parser);
            report.issue(
                &snapshot.path,
                kind,
                None,
                "Parser interrompido antes de verificar o snapshot",
            );
            continue;
        };
        let before = match ast::inspect(&tree, &snapshot.before, language, &mut budget) {
            Ok(f) => f,
            Err(kind) => {
                report.issue(
                    &snapshot.path,
                    kind,
                    None,
                    "Limite de nós/tempo ao inspecionar AST",
                );
                continue;
            }
        };
        report.checked_files += 1;
        if let Some(error) = before.errors.first() {
            report.issue(
                &snapshot.path,
                IssueKind::SyntaxBefore,
                Some(incremental::text_range(&snapshot.before, error.clone())),
                "Snapshot anterior contém ERROR/MISSING; validação sem garantia",
            );
            continue;
        }
        let metadata = match hierarchy::inspect(&tree, &snapshot.before, language, &mut budget) {
            Ok(metadata) => metadata,
            Err(kind) => {
                report.issue(
                    &snapshot.path,
                    kind,
                    None,
                    "Orçamento excedido ao indexar classes",
                );
                continue;
            }
        };
        metadata_bytes = metadata_bytes.saturating_add(metadata.bytes);
        if metadata_bytes > 8 * 1024 * 1024 {
            report.issue(
                &snapshot.path,
                IssueKind::Limit,
                None,
                "Índice nominal excede 8 MiB de strings normalizadas",
            );
            break;
        }
        if edits.is_empty() {
            corpus.push(hierarchy::File {
                tree,
                snapshot,
                language,
                before,
                after: None,
                metadata,
                edits,
            });
            continue;
        }
        let original_tree = tree.clone();
        for edit in edits.iter().rev() {
            tree.edit(&edit.input_edit(&snapshot.before));
        }
        let Some(after_tree) =
            incremental::parse(&mut parser, &snapshot.after, Some(&tree), &budget)
        else {
            let kind = budget.check().err().unwrap_or(IssueKind::Parser);
            report.issue(
                &snapshot.path,
                kind,
                None,
                "Reparse incremental interrompido",
            );
            continue;
        };
        report.incremental_reparses += 1;
        let after = match ast::inspect(&after_tree, &snapshot.after, language, &mut budget) {
            Ok(f) => f,
            Err(kind) => {
                report.issue(
                    &snapshot.path,
                    kind,
                    None,
                    "Limite de nós/tempo ao inspecionar proposta",
                );
                continue;
            }
        };
        if !after.errors.is_empty() {
            report.conflicts.push(Conflict {
                path: snapshot.path.clone(),
                range: snapshot.edits[0].range.clone(),
                message: "A proposta introduz erro sintático; nenhuma alteração foi aplicada"
                    .into(),
                related: None,
                kind: ConflictKind::InvalidSyntax,
                container_name: None,
            });
            continue;
        }
        if let Err(kind) = ast::check_changes(
            snapshot,
            &edits,
            (&before, &after),
            language,
            &mut report,
            &mut evidence,
            &mut budget,
        ) {
            report.issue(
                &snapshot.path,
                kind,
                None,
                "Orçamento excedido durante verificação de membros",
            );
            break;
        }
        corpus.push(hierarchy::File {
            tree: original_tree,
            snapshot,
            language,
            before,
            after: Some(after),
            metadata,
            edits,
        });
    }
    if let Err(kind) = hierarchy::check(&corpus, &mut report, &mut budget, binding) {
        report.issue(
            fallback,
            kind,
            None,
            "Orçamento excedido ao verificar hierarquia",
        );
    }
    if let Err(kind) = evidence.finish(&mut report, &mut budget) {
        report.issue(
            fallback,
            kind,
            None,
            "Validação cancelada ou orçamento excedido ao conferir usos",
        );
    }
    if report.checked_declarations == 0 {
        report.issue(
            fallback,
            IssueKind::UnsupportedEdit,
            None,
            "Nenhuma edição de declaração de método suportada foi comprovada",
        );
    }
    report.coverage = if !report.conflicts.is_empty() {
        Coverage::Rejected
    } else if !report.issues.is_empty() {
        Coverage::Unsupported
    } else {
        Coverage::Verified
    };
    report
}

/// Plan a same-folder class/file rename and exact AST module/include edits.
/// The application must validate symbol edits and perform filesystem checks.
pub fn plan_class_file_rename(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    resources::plan(snapshots, new_name, cancel, limits)
}

/// Resolves PHP trait/alias call origins against immutable project snapshots.
pub fn prepare_trait_rename_in_corpus_with_limits(
    snapshots: &[RenameSnapshot],
    path: &std::path::Path,
    origin: &TextRange,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<String>, String> {
    hierarchy::prepare_traits_corpus(snapshots, path, origin, cancel, limits)
}

/// AST module edits for an explicitly selected same-folder file row. No I/O.
pub fn plan_named_file_rename(
    snapshots: &[RenameSnapshot],
    source: &std::path::Path,
    target: &std::path::Path,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    resources::named(snapshots, source, target, cancel, limits)
}

/// Discover the optional matching class file row without approving its move.
pub fn class_file_rename_candidate(
    snapshots: &[RenameSnapshot],
    new_name: &str,
    cancel: &AtomicBool,
    limits: ValidationLimits,
) -> Result<Option<RenameResourceProposal>, String> {
    resources::candidate(snapshots, new_name, cancel, limits)
}
