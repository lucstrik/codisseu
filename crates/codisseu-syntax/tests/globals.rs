use codisseu_syntax::{
    ConflictKind, Coverage, IssueKind, RenameEdit, RenameSnapshot, RenameValidation, TextRange,
    ValidationLimits, validate_global_rename_with_limits,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn snapshot(path: &str, marked: &str, new: &str) -> RenameSnapshot {
    let mut before = String::new();
    let mut ranges = Vec::new();
    let mut rest = marked;
    while let Some(start) = rest.find('~') {
        before.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find('~').unwrap();
        let a = before.len();
        before.push_str(&rest[..end]);
        ranges.push(a..before.len());
        rest = &rest[end + 1..];
    }
    before.push_str(rest);
    let position = |byte: usize| {
        let prefix = &before[..byte];
        (
            prefix.bytes().filter(|b| *b == b'\n').count() as u32,
            prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
        )
    };
    let edits = ranges
        .iter()
        .map(|r| {
            let (a, b) = position(r.start);
            let (c, d) = position(r.end);
            RenameEdit {
                range: TextRange {
                    start_line: a,
                    start_character: b,
                    end_line: c,
                    end_character: d,
                },
                new_text: new.into(),
            }
        })
        .collect();
    let mut after = before.clone();
    for range in ranges.into_iter().rev() {
        after.replace_range(range, new);
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(before),
        after: Arc::from(after),
        edits,
    }
}
fn validate(files: &[(&str, &str)], new: &str) -> RenameValidation {
    let snapshots: Vec<_> = files.iter().map(|(p, s)| snapshot(p, s, new)).collect();
    validate_global_rename_with_limits(
        &snapshots,
        new,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .expect("global dispatcher")
}
fn verified(files: &[(&str, &str)], new: &str) {
    let r = validate(files, new);
    assert_eq!(r.coverage, Coverage::Verified, "{r:#?}");
    assert_eq!(r.checked_declarations, 1);
}
#[test]
fn php_class_namespace_import_alias_and_qualified_names() {
    verified(
        &[
            (
                "/w/src/Weather.php",
                "<?php namespace Fixture; class ~Weather~ {}",
            ),
            (
                "/w/main.php",
                "<?php use Fixture\\~Weather~; new ~Weather~(); function run(~Weather~ $weather): ~Weather~ { return new ~Weather~(); } ",
            ),
            (
                "/w/alias.php",
                "<?php use Fixture\\~Weather~ as Sky; new Sky(); new \\Fixture\\~Weather~(); // Weather\n$x='Weather';",
            ),
            (
                "/w/other.php",
                "<?php namespace Other; class Weather {} new Weather();",
            ),
        ],
        "Climate",
    );
}
#[test]
fn php_function_namespace_import_alias_and_recursive_calls() {
    verified(
        &[
            (
                "/w/src/functions.php",
                "<?php namespace Fixture; function ~forecast~(): string { if (false) { return ~forecast~(); } return 'forecast'; }",
            ),
            (
                "/w/main.php",
                "<?php use function Fixture\\~forecast~; echo ~forecast~();",
            ),
            (
                "/w/alias.php",
                "<?php use function Fixture\\~forecast~ as rain; echo rain(); echo \\Fixture\\~forecast~();",
            ),
            (
                "/w/other.php",
                "<?php namespace Other; function forecast() {} forecast();",
            ),
        ],
        "predict",
    );
}
#[test]
fn ts_class_export_import_alias_types_extends_and_construct() {
    verified(
        &[
            ("/w/src/weather.ts", "export class ~Weather~ {}"),
            (
                "/w/main.ts",
                "import { ~Weather~ } from './src/weather'; const w: ~Weather~ = new ~Weather~(); class Child extends ~Weather~ {}",
            ),
            (
                "/w/alias.ts",
                "import { ~Weather~ as Sky } from './src/weather'; const w: Sky = new Sky(); // Weather\nconst text='Weather';",
            ),
            (
                "/w/other.ts",
                "export class Weather {} const w = new Weather();",
            ),
        ],
        "Climate",
    );
}
#[test]
fn ts_function_export_import_alias_and_escaping_reference() {
    verified(
        &[
            (
                "/w/src/weather.ts",
                "export function ~forecast~(): string { return 'forecast'; } const callback = ~forecast~;",
            ),
            (
                "/w/main.ts",
                "import { ~forecast~ } from './src/weather'; ~forecast~();",
            ),
            (
                "/w/alias.ts",
                "import { ~forecast~ as rain } from './src/weather'; rain();",
            ),
        ],
        "predict",
    );
}
#[test]
fn ts_shadow_parameters_and_blocks_keep_other_bindings() {
    verified(
        &[(
            "/w/main.ts",
            "function ~forecast~() {} ~forecast~(); function inner(forecast: () => void) { forecast(); } { const forecast = () => {}; forecast(); } function closure(){ ~forecast~(); }",
        )],
        "predict",
    );
}
#[test]
fn php_class_names_are_case_insensitive() {
    verified(
        &[(
            "/w/a.php",
            "<?php class ~Weather~ {} new ~weather~(); new ~WEATHER~();",
        )],
        "Climate",
    );
}
#[test]
fn php_function_names_are_case_insensitive() {
    verified(
        &[("/w/a.php", "<?php function ~forecast~() {} ~FORECAST~();")],
        "predict",
    );
}
#[test]
fn unicode_utf16_crlf_and_no_final_newline() {
    verified(
        &[(
            "/w/a.ts",
            "/* 😀 é */ class ~Weather~ {}\r\nconst w: ~Weather~ = new ~Weather~();",
        )],
        "Climática",
    );
}
#[test]
fn declaration_only_is_valid() {
    for (p, s) in [
        ("/w/a.php", "<?php class ~Weather~ {}"),
        ("/w/a.ts", "export class ~Weather~ {}"),
    ] {
        verified(&[(p, s)], "Climate");
    }
}
#[test]
fn complete_corpus_rejects_missing_reference_or_import() {
    for src in [
        "import { Weather } from './a'; new Weather();",
        "import { ~Weather~ } from './a'; new Weather();",
    ] {
        let r = validate(
            &[("/w/a.ts", "export class ~Weather~ {}"), ("/w/b.ts", src)],
            "Climate",
        );
        assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::MissingReference)
        );
    }
}
#[test]
fn same_namespace_collisions_are_rejected() {
    for (p, s) in [
        (
            "/w/a.php",
            "<?php class ~Weather~ {} class CLIMATE {} new ~Weather~();",
        ),
        (
            "/w/a.ts",
            "class ~Weather~ {} class Climate {} new ~Weather~();",
        ),
    ] {
        let r = validate(&[(p, s)], "Climate");
        assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::DuplicateBinding)
        );
    }
}
#[test]
fn new_name_cannot_capture_preexisting_reference() {
    let r = validate(
        &[(
            "/w/a.ts",
            "function ~forecast~() {} function f() { predict(); ~forecast~(); }",
        )],
        "predict",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::BindingChanged)
    );
}
#[test]
fn new_name_cannot_be_captured_by_local_variable() {
    let r = validate(
        &[(
            "/w/a.ts",
            "function ~forecast~() {} function f(predict:()=>void) { ~forecast~(); }",
        )],
        "predict",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
}
#[test]
fn unrelated_homonym_included_by_provider_is_rejected() {
    let r = validate(
        &[
            ("/w/a.ts", "export class ~Weather~ {}"),
            ("/w/b.ts", "export class ~Weather~ {}"),
        ],
        "Climate",
    );
    assert_ne!(r.coverage, Coverage::Verified);
}
#[test]
fn explicit_alias_cannot_be_renamed_with_exported_symbol() {
    let r = validate(
        &[
            ("/w/a.ts", "export class ~Weather~ {}"),
            (
                "/w/b.ts",
                "import { ~Weather~ as Sky } from './a'; new ~Sky~();",
            ),
        ],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
}
#[test]
fn missing_module_or_ambiguous_resolution_never_verified() {
    for module in ["package", "./missing", "./a"] {
        let mut files = vec![("/w/a.ts", "export class ~Weather~ {}"), ("/w/b.ts", "")];
        let body = format!("import {{ ~Weather~ }} from '{module}'; new ~Weather~();");
        files[1].1 = &body;
        if module == "./a" {
            files.push(("/w/a/index.ts", "export class Weather {}"));
        }
        let r = validate(&files, "Climate");
        assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}
#[test]
fn reflection_reexports_and_shorthand_are_not_certified() {
    for (p, s) in [
        (
            "/w/a.php",
            "<?php class ~Weather~ {} class_exists('Weather');",
        ),
        (
            "/w/a.ts",
            "export class ~Weather~ {} export { Weather as Sky };",
        ),
        (
            "/w/a.ts",
            "function ~forecast~() {} const object={~forecast~};",
        ),
    ] {
        let r = validate(&[(p, s)], "Climate");
        assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}
#[test]
fn invalid_snapshot_syntax_limits_and_cancel_are_explicit() {
    let snap = snapshot("/w/a.ts", "class ~Weather~ {}", "Climate");
    let mut invalid = snap.clone();
    invalid.after = Arc::from("class Different {}");
    let r = validate_global_rename_with_limits(
        &[invalid],
        "Climate",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap();
    assert_ne!(r.coverage, Coverage::Verified);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::InvalidSnapshot)
    );
    for (cancel, limits, kind) in [
        (true, ValidationLimits::default(), IssueKind::Cancelled),
        (
            false,
            ValidationLimits {
                max_nodes: 0,
                max_duration: Duration::from_secs(1),
            },
            IssueKind::Limit,
        ),
    ] {
        let r = validate_global_rename_with_limits(
            std::slice::from_ref(&snap),
            "Climate",
            &AtomicBool::new(cancel),
            limits,
        )
        .unwrap();
        assert!(r.issues.iter().any(|i| i.kind == kind), "{r:#?}");
    }
    let r = validate(&[("/w/a.ts", "class ~Weather~ { broken( }")], "Climate");
    assert_ne!(r.coverage, Coverage::Verified);
}
#[test]
fn dispatch_leaves_methods_and_locals_to_their_validators() {
    for s in [
        "class A { ~forecast~(){} }",
        "function f(~weather~: number) { return ~weather~; }",
    ] {
        let r = validate_global_rename_with_limits(
            &[snapshot("/w/a.ts", s, "changed")],
            "changed",
            &AtomicBool::new(false),
            ValidationLimits::default(),
        );
        assert!(r.is_none(), "{r:#?}");
    }
}

#[test]
fn declaration_only_collisions_with_import_local_and_other_symbol_kind_are_rejected() {
    for source in [
        "class ~Weather~ {} const Climate = 1;",
        "class ~Weather~ {} function Climate() {}",
        "class ~Weather~ {} import { Existing as Climate } from './b';",
    ] {
        let r = validate(
            &[("/w/a.ts", source), ("/w/b.ts", "export class Existing {}")],
            "Climate",
        );
        assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    }
}
#[test]
fn typescript_scripts_share_globals_but_modules_do_not() {
    verified(
        &[
            ("/w/a.ts", "class ~Weather~ {}"),
            ("/w/b.ts", "new ~Weather~();"),
        ],
        "Climate",
    );
    let r = validate(
        &[
            ("/w/a.ts", "class ~Weather~ {}"),
            ("/w/b.ts", "class Climate {}"),
        ],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    verified(
        &[
            ("/w/a.ts", "export class ~Weather~ {}"),
            ("/w/b.ts", "export class Climate {}"),
        ],
        "Climate",
    );
}
#[test]
fn php_static_class_constants_instanceof_and_attributes_are_bound() {
    verified(
        &[(
            "/w/a.php",
            "<?php class ~Weather~ {} $a = ~Weather~::class; $b = new ~Weather~(); $c = $b instanceof ~Weather~; #[~Weather~] class Other {}",
        )],
        "Climate",
    );
    let r = validate(
        &[("/w/a.php", "<?php class ~Weather~ {} $a = Weather::class;")],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
}
#[test]
fn type_positions_are_not_shadowed_by_value_parameters() {
    verified(
        &[(
            "/w/a.ts",
            "class ~Weather~ {} function f(Weather: number, other: ~Weather~): ~Weather~ { return other; }",
        )],
        "Climate",
    );
}
#[test]
fn var_hoisting_and_nested_php_globals_are_explicitly_unsupported() {
    for (p, s) in [
        (
            "/w/a.ts",
            "function ~forecast~() {} function f(){ if (true) {var forecast = 1;} forecast; }",
        ),
        (
            "/w/a.php",
            "<?php function ~forecast~() {} if (true) {function other() {}}",
        ),
    ] {
        let r = validate(&[(p, s)], "predict");
        assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}

fn caret(text: &str, name: &str) -> TextRange {
    let byte = text.find(name).unwrap();
    let p = &text[..byte];
    let line = p.bytes().filter(|b| *b == b'\n').count() as u32;
    let character = p.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
    TextRange {
        start_line: line,
        start_character: character,
        end_line: line,
        end_character: character,
    }
}
#[test]
fn independent_semantic_provider_proposes_exact_ast_bindings_preserving_aliases_and_text() {
    let snapshots = vec![
        snapshot(
            "/w/a.php",
            "<?php namespace Fixture; class Weather {}",
            "Climate",
        ),
        snapshot(
            "/w/b.php",
            "<?php use Fixture\\Weather; new Weather(); $s='Weather'; // Weather",
            "Climate",
        ),
        snapshot(
            "/w/c.php",
            "<?php use Fixture\\Weather as Sky; new Sky();",
            "Climate",
        ),
        snapshot(
            "/w/d.php",
            "<?php namespace Other; class Weather {} new Weather();",
            "Climate",
        ),
    ];
    let origin = caret(&snapshots[1].before, "Weather");
    let proposed = codisseu_syntax::propose_global_rename_with_limits(
        &snapshots,
        &snapshots[1].path,
        &origin,
        "Climate",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(proposed.iter().map(|s| s.edits.len()).sum::<usize>(), 4);
    assert_eq!(
        &*proposed[1].after,
        "<?php use Fixture\\Climate; new Climate(); $s='Weather'; // Weather"
    );
    assert_eq!(
        &*proposed[2].after,
        "<?php use Fixture\\Climate as Sky; new Sky();"
    );
    assert_eq!(proposed[3].before, proposed[3].after);
    assert_eq!(
        validate_global_rename_with_limits(
            &proposed,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .unwrap()
        .coverage,
        Coverage::Verified
    );
}
#[test]
fn semantic_provider_keeps_collisions_in_proposal_for_normal_rejected_preview() {
    let snapshots = vec![snapshot(
        "/w/a.php",
        "<?php function forecast() {} function predict() {} forecast();",
        "predict",
    )];
    let origin = caret(&snapshots[0].before, "forecast");
    let proposed = codisseu_syntax::propose_global_rename_with_limits(
        &snapshots,
        &snapshots[0].path,
        &origin,
        "predict",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(proposed[0].edits.len(), 2);
    assert_eq!(
        validate_global_rename_with_limits(
            &proposed,
            "predict",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .unwrap()
        .coverage,
        Coverage::Rejected
    );
}
#[test]
fn semantic_prepare_obeys_ast_utf16_and_does_not_claim_method_or_variable() {
    for (text, name, wanted) in [
        (
            "<?php /* 😀 */ class Weather {}",
            "Weather",
            Some("Weather"),
        ),
        ("<?php function forecast() {}", "forecast", Some("forecast")),
        ("<?php class A { function forecast() {} }", "forecast", None),
        ("<?php function f($weather) {}", "weather", None),
    ] {
        let p = caret(text, name);
        let result = codisseu_syntax::prepare_global_rename_with_limits(
            std::path::Path::new("/w/a.php"),
            text,
            p.start_line,
            p.start_character,
            &AtomicBool::new(false),
            ValidationLimits::default(),
        )
        .unwrap();
        assert_eq!(result.as_deref(), wanted);
    }
}
#[test]
fn semantic_provider_rejects_mutated_inputs_cancel_and_invalid_origin() {
    let snapshots = vec![snapshot("/w/a.php", "<?php class Weather {}", "Climate")];
    let origin = caret(&snapshots[0].before, "Weather");
    assert!(
        codisseu_syntax::propose_global_rename_with_limits(
            &snapshots,
            &snapshots[0].path,
            &origin,
            "Climate",
            &AtomicBool::new(true),
            ValidationLimits::default()
        )
        .is_err()
    );
    let altered = vec![snapshot("/w/a.php", "<?php class ~Weather~ {}", "Climate")];
    assert!(
        codisseu_syntax::propose_global_rename_with_limits(
            &altered,
            &altered[0].path,
            &origin,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    let mut invalid = origin;
    invalid.start_character = 999;
    invalid.end_character = 999;
    assert!(
        codisseu_syntax::propose_global_rename_with_limits(
            &snapshots,
            &snapshots[0].path,
            &invalid,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
}
#[test]
fn explicit_import_aliases_are_independent_rename_bindings() {
    verified(
        &[
            ("/w/a.ts", "export class Weather {}"),
            (
                "/w/b.ts",
                "import { Weather as ~Sky~ } from './a'; const w: ~Sky~ = new ~Sky~();",
            ),
        ],
        "Climate",
    );
    verified(
        &[
            (
                "/w/a.php",
                "<?php namespace Fixture; function forecast() {}",
            ),
            (
                "/w/b.php",
                "<?php use function Fixture\\forecast as ~rain~; ~rain~();",
            ),
        ],
        "predict",
    );
    verified(
        &[
            ("/w/a.php", "<?php namespace Fixture; class Weather {}"),
            (
                "/w/b.php",
                "<?php use Fixture\\Weather as ~Sky~; new ~Sky~();",
            ),
        ],
        "Climate",
    );
}
#[test]
fn alias_rename_collisions_and_missing_uses_are_rejected() {
    for s in [
        "import { Weather as ~Sky~ } from './a'; const Climate=1; new ~Sky~();",
        "import { Weather as ~Sky~ } from './a'; new Sky();",
    ] {
        let r = validate(
            &[("/w/a.ts", "export class Weather {}"), ("/w/b.ts", s)],
            "Climate",
        );
        assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    }
}
#[test]
fn known_php_runtime_collisions_are_blocked_only_in_global_namespace() {
    for (source, new) in [
        ("<?php class ~Weather~ {}", "stdClass"),
        ("<?php function ~forecast~() {}", "strlen"),
    ] {
        let r = validate(&[("/w/a.php", source)], new);
        assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    }
    verified(
        &[(
            "/w/a.php",
            "<?php namespace Fixture; function ~forecast~() {} ~forecast~();",
        )],
        "strlen",
    );
}
#[test]
fn named_typescript_reexports_preserve_identity_across_multiple_modules() {
    verified(
        &[
            ("/w/a.ts", "export class ~Weather~ {}"),
            ("/w/b.ts", "export { ~Weather~ } from './a';"),
            ("/w/c.ts", "export { ~Weather~ } from './b';"),
            (
                "/w/main.ts",
                "import { ~Weather~ } from './c'; new ~Weather~();",
            ),
        ],
        "Climate",
    );
}
#[test]
fn reexport_alias_preserves_exported_name_when_original_changes() {
    verified(
        &[
            ("/w/a.ts", "export function ~forecast~() {}"),
            ("/w/b.ts", "export { ~forecast~ as predict } from './a';"),
            ("/w/main.ts", "import { predict } from './b'; predict();"),
        ],
        "rainfall",
    );
}
#[test]
fn reexport_alias_can_be_renamed_independently_with_consumers() {
    verified(
        &[
            ("/w/a.ts", "export class Weather {}"),
            ("/w/b.ts", "export { Weather as ~Sky~ } from './a';"),
            ("/w/main.ts", "import { ~Sky~ } from './b'; new ~Sky~();"),
        ],
        "Climate",
    );
}
#[test]
fn reexport_missing_consumer_cycle_and_duplicate_exports_are_not_certified() {
    let r = validate(
        &[
            ("/w/a.ts", "export class ~Weather~ {}"),
            ("/w/b.ts", "export { ~Weather~ } from './a';"),
            (
                "/w/main.ts",
                "import { Weather } from './b'; new Weather();",
            ),
        ],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    let r = validate(
        &[
            ("/w/a.ts", "export { ~Weather~ } from './b';"),
            ("/w/b.ts", "export { Weather } from './a';"),
        ],
        "Climate",
    );
    assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    let r = validate(
        &[
            ("/w/a.ts", "export class Weather {} export class Other {}"),
            (
                "/w/b.ts",
                "export { Weather as ~Sky~, Other as Climate } from './a';",
            ),
        ],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
}
#[test]
fn global_object_function_references_are_complete_and_module_distinct() {
    verified(
        &[(
            "/w/a.ts",
            "function ~forecast~() {} globalThis.~forecast~(); window.~forecast~();",
        )],
        "predict",
    );
    let r = validate(
        &[("/w/a.ts", "function ~forecast~() {} globalThis.forecast();")],
        "predict",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    let r = validate(
        &[(
            "/w/a.ts",
            "export function ~forecast~() {} globalThis.~forecast~();",
        )],
        "predict",
    );
    assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
}
#[test]
fn computed_global_access_and_destructuring_do_not_receive_false_proof() {
    for source in [
        "function ~forecast~() {} globalThis['forecast']();",
        "function ~forecast~() {} function f(){const {forecast}=obj; ~forecast~();}",
    ] {
        let r = validate(&[("/w/a.ts", source)], "predict");
        assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}
#[test]
fn php_reflection_string_callbacks_and_dynamic_construction_are_blocked() {
    for source in [
        "<?php class ~Weather~ {} new ReflectionClass('Weather');",
        "<?php class ~Weather~ {} new ('Weather')();",
        "<?php function ~forecast~() {} array_map('forecast', []);",
    ] {
        let r = validate(&[("/w/a.php", source)], "Climate");
        assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}
#[test]
fn restricted_typescript_binding_names_are_not_certified() {
    for new in ["eval", "arguments", "await", "yield", "any"] {
        let r = validate(&[("/w/a.ts", "export class ~Weather~ {}")], new);
        assert_ne!(r.coverage, Coverage::Verified, "{new}: {r:#?}");
    }
}
#[test]
fn unknown_namespace_alias_prefix_is_not_silently_left_stale() {
    let r = validate(
        &[(
            "/w/a.php",
            "<?php use Outside\\Package as ~Api~; new Api\\Weather();",
        )],
        "Changed",
    );
    assert_ne!(r.coverage, Coverage::Verified, "{r:#?}");
}
#[test]
fn grouped_php_class_and_function_imports_keep_alias_bindings() {
    verified(
        &[
            (
                "/w/a.php",
                "<?php namespace Fixture; class ~Weather~ {} class Other {}",
            ),
            (
                "/w/b.php",
                "<?php use Fixture\\{~Weather~, Other as Sky}; new ~Weather~(); new Sky();",
            ),
        ],
        "Climate",
    );
    verified(
        &[
            (
                "/w/a.php",
                "<?php namespace Fixture; function ~forecast~() {} function other() {}",
            ),
            (
                "/w/b.php",
                "<?php use function Fixture\\{~forecast~, other as rain}; ~forecast~(); rain();",
            ),
        ],
        "predict",
    );
}
#[test]
fn local_export_clauses_and_import_then_export_preserve_identity() {
    verified(
        &[
            ("/w/a.ts", "class ~Weather~ {} export { ~Weather~ };"),
            (
                "/w/b.ts",
                "import { ~Weather~ } from './a'; export { ~Weather~ };",
            ),
            (
                "/w/main.ts",
                "import { ~Weather~ } from './b'; new ~Weather~();",
            ),
        ],
        "Climate",
    );
}
#[test]
fn local_export_alias_rename_and_original_rename_are_independent() {
    verified(
        &[
            ("/w/a.ts", "class ~Weather~ {} export { ~Weather~ as Sky };"),
            ("/w/main.ts", "import { Sky } from './a'; new Sky();"),
        ],
        "Climate",
    );
    verified(
        &[
            ("/w/a.ts", "class Weather {} export { Weather as ~Sky~ };"),
            ("/w/main.ts", "import { ~Sky~ } from './a'; new ~Sky~();"),
        ],
        "Climate",
    );
}
#[test]
fn exports_of_import_aliases_follow_the_alias_identity() {
    verified(
        &[
            ("/w/a.ts", "export class Weather {}"),
            (
                "/w/b.ts",
                "import { Weather as ~Sky~ } from './a'; export { ~Sky~ };",
            ),
            ("/w/main.ts", "import { ~Sky~ } from './b'; new ~Sky~();"),
        ],
        "Climate",
    );
}
#[test]
fn malicious_extra_edit_in_tokenless_file_reports_syntax_without_foreign_offsets() {
    let r = validate(
        &[
            (
                "/w/a.php",
                "<?php /* Long unrelated prefix with emoji 😀 */ class ~Weather~ {}",
            ),
            ("/w/b.php", "<?php ~/*x*/~"),
        ],
        "Climate",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:#?}");
    assert!(r.conflicts.iter().any(
        |c| c.path == std::path::Path::new("/w/b.php") && c.kind == ConflictKind::InvalidSyntax
    ));
}

#[test]
fn php_namespace_prefix_keeps_alias_when_class_or_function_changes() {
    verified(
        &[
            (
                "/w/model.php",
                "<?php namespace Domain\\Sub; class ~Weather~ {}",
            ),
            (
                "/w/main.php",
                "<?php use Domain\\Sub as Api; function accept(Api\\~Weather~ $x): Api\\~Weather~ { return $x; } new Api\\~Weather~(); Api\\~Weather~::class;",
            ),
            (
                "/w/other.php",
                "<?php namespace Other; class Weather {} new Weather();",
            ),
        ],
        "Climate",
    );
    verified(
        &[
            (
                "/w/functions.php",
                "<?php namespace Domain\\Sub; function ~forecast~() {}",
            ),
            (
                "/w/main.php",
                "<?php use Domain\\Sub as Api; Api\\~forecast~();",
            ),
            (
                "/w/other.php",
                "<?php namespace Other; function forecast() {} forecast();",
            ),
        ],
        "predict",
    );
}

#[test]
fn php_namespace_alias_prefix_changes_independently_across_known_suffixes() {
    verified(
        &[
            (
                "/w/model.php",
                "<?php namespace Domain\\Sub; class Weather {} function forecast() {}",
            ),
            (
                "/w/deep.php",
                "<?php namespace Domain\\Sub\\Deep; class Weather {}",
            ),
            (
                "/w/main.php",
                "<?php /* 😀 */ use Domain\\Sub as ~Api~; function accept(~Api~\\Weather $x): ~Api~\\Weather { return $x; } new ~Api~\\Weather(); ~Api~\\forecast(); new ~Api~\\Deep\\Weather(); ~Api~\\Weather::class;",
            ),
            (
                "/w/other.php",
                "<?php use Domain\\Sub as Api; new Api\\Weather();",
            ),
        ],
        "Model",
    );
}

#[test]
fn php_namespace_prefix_omissions_wrong_suffixes_and_capture_are_rejected() {
    for main in [
        "<?php use Domain\\Sub as ~Api~; new Api\\Weather();",
        "<?php use Domain\\Sub as ~Api~; new ~Api~\\~Weather~();",
        "<?php use Domain\\Sub as ~Api~; use Other\\Sub as Model; new ~Api~\\Weather();",
        "<?php use Domain\\Sub as ~Api~; new ~Api~\\Weather(); new Model\\Weather();",
    ] {
        let r = validate(
            &[
                (
                    "/w/model.php",
                    "<?php namespace Domain\\Sub; class Weather {}",
                ),
                ("/w/other.php", "<?php namespace Model; class Weather {}"),
                ("/w/main.php", main),
            ],
            "Model",
        );
        assert_ne!(r.coverage, Coverage::Verified, "{main}: {r:#?}");
    }
    let missing = validate(
        &[
            (
                "/w/model.php",
                "<?php namespace Domain\\Sub; class ~Weather~ {}",
            ),
            (
                "/w/main.php",
                "<?php use Domain\\Sub as Api; new Api\\Weather();",
            ),
        ],
        "Climate",
    );
    assert_eq!(missing.coverage, Coverage::Rejected, "{missing:#?}");
}

#[test]
fn php_namespace_prefix_does_not_claim_unknown_external_or_constant_targets() {
    for tail in [
        "new ~Api~\\Outside();",
        "echo Api\\VALUE;",
        "new Api\\$kind();",
    ] {
        let main = format!("<?php use Domain\\Sub as ~Api~; new ~Api~\\Weather(); {tail}");
        let r = validate(
            &[
                (
                    "/w/model.php",
                    "<?php namespace Domain\\Sub; class Weather {}",
                ),
                ("/w/main.php", &main),
            ],
            "Model",
        );
        assert_ne!(r.coverage, Coverage::Verified, "{tail}: {r:#?}");
    }
}

#[test]
fn php_namespace_prefix_semantic_proposal_uses_exact_component_origins() {
    let files = vec![
        snapshot(
            "/w/model.php",
            "<?php namespace Domain\\Sub; class Weather {} function forecast() {}",
            "unused",
        ),
        snapshot(
            "/w/main.php",
            "<?php /* 😀 */ use Domain\\Sub as Api; new Api\\Weather(); Api\\forecast();",
            "unused",
        ),
    ];
    for (needle, new, expected_count) in [
        ("Api\\Weather", "Model", 3),
        ("Weather()", "Climate", 2),
        ("forecast()", "predict", 2),
    ] {
        let byte = files[1].before.find(needle).unwrap();
        let column = files[1].before[..byte].encode_utf16().count() as u32;
        let range = TextRange {
            start_line: 0,
            start_character: column,
            end_line: 0,
            end_character: column,
        };
        let proposed = codisseu_syntax::propose_global_rename_with_limits(
            &files,
            &files[1].path,
            &range,
            new,
            &AtomicBool::new(false),
            ValidationLimits::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            proposed.iter().map(|s| s.edits.len()).sum::<usize>(),
            expected_count
        );
        let report = validate_global_rename_with_limits(
            &proposed,
            new,
            &AtomicBool::new(false),
            ValidationLimits::default(),
        )
        .unwrap();
        assert_eq!(report.coverage, Coverage::Verified, "{needle}: {report:#?}");
        assert_eq!(
            codisseu_syntax::prepare_global_rename_with_limits(
                &files[1].path,
                &files[1].before,
                0,
                column,
                &AtomicBool::new(false),
                ValidationLimits::default()
            )
            .unwrap()
            .as_deref(),
            Some(if new == "Model" {
                "Api"
            } else if new == "Climate" {
                "Weather"
            } else {
                "forecast"
            })
        );
    }
}
