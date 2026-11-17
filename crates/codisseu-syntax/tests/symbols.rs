use codisseu_syntax::*;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
fn range(text: &str, start: usize, end: usize) -> TextRange {
    let pos = |n| {
        let p = &text[..n];
        (
            p.bytes().filter(|b| *b == b'\n').count() as u32,
            p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
        )
    };
    let (a, b) = pos(start);
    let (c, d) = pos(end);
    TextRange {
        start_line: a,
        start_character: b,
        end_line: c,
        end_character: d,
    }
}
fn selected(path: &str, text: &str, old: &str, new: &str, offsets: &[usize]) -> RenameSnapshot {
    let edits = offsets
        .iter()
        .map(|&i| RenameEdit {
            range: range(text, i, i + old.len()),
            new_text: new.into(),
        })
        .collect();
    let mut after = text.to_owned();
    for &i in offsets.iter().rev() {
        after.replace_range(i..i + old.len(), new);
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(after),
        edits,
    }
}
fn all(path: &str, text: &str, old: &str, new: &str) -> RenameSnapshot {
    selected(
        path,
        text,
        old,
        new,
        &text.match_indices(old).map(|(i, _)| i).collect::<Vec<_>>(),
    )
}
fn check(s: RenameSnapshot, name: &str) -> RenameValidation {
    validate_symbol_rename(&[s], name, &AtomicBool::new(false)).expect("recognized lexical symbol")
}
#[test]
fn locals_and_parameters_preserve_unicode_utf16_crlf_comments_strings_and_homonyms() {
    for (path, text) in [
        (
            "A.ts",
            "/* 😀 */ function f(rain: number) {\r\nconst 雨 = rain; return 雨 + rain; }\r\nfunction g(rain: number) { return rain; }\r\n// rain\r\nconst comment = 'rain';",
        ),
        (
            "A.php",
            "<?php /* 😀 */ function f($rain) {\r\n$雨 = $rain; return $雨 + $rain; }\r\nfunction g($rain) { return $rain; }\r\n// rain\r\n$comment = 'rain';",
        ),
    ] {
        let end = text.find("function g").unwrap();
        let offsets = text[..end]
            .match_indices("rain")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let s = selected(path, text, "rain", "precipitação", &offsets);
        let after = s.after.clone();
        let r = check(s, "precipitação");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.scope, ValidationScope::LexicalBindings);
        assert_eq!(r.checked_declarations, 1);
        assert!(after.contains("// rain"));
        assert!(after.contains("'rain'"));
        assert!(after.contains(if path.ends_with("php") {
            "function g($rain)"
        } else {
            "function g(rain: number)"
        }));
    }
}
#[test]
fn ts_lexical_nested_block_shadowing_and_captured_uses_keep_separate_bindings() {
    let text = "function f(rain: number) { const cb = () => rain; { let rain = 2; consume(rain); } return cb() + rain; }";
    let offsets = text
        .match_indices("rain")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let r = check(
        selected(
            "A.ts",
            text,
            "rain",
            "forecast",
            &[offsets[0], offsets[1], offsets[4]],
        ),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn local_variables_and_arrow_parameters_are_supported() {
    for (path, text) in [
        (
            "A.ts",
            "function f() { let rain = 1; rain += 2; return rain; }",
        ),
        ("A.ts", "const f = (rain: number) => rain + rain;"),
        ("A.ts", "const f = rain => rain + rain;"),
        (
            "A.php",
            "<?php function f() { $rain = 1; $rain += 2; return $rain; }",
        ),
    ] {
        let r = check(all(path, text, "rain", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Verified, "{path} {text}: {r:?}");
    }
}
#[test]
fn local_collisions_reject_and_bindings_report_locations() {
    for (path, text) in [
        (
            "A.ts",
            "function f(rain: number, forecast: number) { return rain + forecast; }",
        ),
        (
            "A.php",
            "<?php function f($rain, $forecast) { return $rain + $forecast; }",
        ),
    ] {
        let r = check(all(path, text, "rain", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
        assert!(r.conflicts.iter().any(|c| matches!(
            c.kind,
            ConflictKind::BindingChanged | ConflictKind::DuplicateBinding
        )));
        assert!(r.conflicts.iter().all(|c| c.related.is_some()));
    }
}
#[test]
fn ts_captures_in_both_directions_reject_without_textual_substitution() {
    for text in [
        "function f() { let rain = 1; { let forecast = 2; consume(rain); } return rain; }",
        "function f() { let rain = 1; consume(forecast); return rain; }",
        "function f() { let rain = 1; const g = (forecast: number) => rain + forecast; return rain; }",
    ] {
        let r = check(all("A.ts", text, "rain", "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Rejected, "{text} {r:?}");
    }
}
#[test]
fn parameter_and_function_body_lexical_variable_cannot_merge() {
    let text = "function f(rain: number) { let forecast = 1; return rain + forecast; }";
    let r = check(all("A.ts", text, "rain", "forecast"), "forecast");
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
}
#[test]
fn incomplete_proposals_and_unrelated_same_named_bindings_are_unsupported() {
    let text = "function f(rain: number) { return rain; }";
    let r = check(
        selected(
            "A.ts",
            text,
            "rain",
            "forecast",
            &[text.find("rain").unwrap()],
        ),
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::BindingChanged)
    );
    assert!(r.issues.iter().any(|i| i.message.contains("parcial")));
    for (path, text) in [
        (
            "A.ts",
            "function f(rain: number) { return rain; } function g(rain: number) { return rain; }",
        ),
        (
            "A.php",
            "<?php function f($rain) { return $rain; } function g($rain) { return $rain; }",
        ),
    ] {
        assert_ne!(
            check(all(path, text, "rain", "forecast"), "forecast").coverage,
            Coverage::Verified
        );
    }
}
#[test]
fn globals_functions_classes_and_imports_never_claim_lexical_support() {
    for (path, text, old) in [
        ("A.ts", "let rain = 1; consume(rain);", "rain"),
        ("A.php", "<?php $rain = 1; echo $rain;", "rain"),
        ("A.ts", "function rain() { return 1; } rain();", "rain"),
        (
            "A.php",
            "<?php function rain() { return 1; } rain();",
            "rain",
        ),
        ("A.ts", "class Rain {} const x = new Rain();", "Rain"),
        ("A.php", "<?php class Rain {} $x = new Rain();", "Rain"),
        ("A.ts", "import { rain } from './other'; rain();", "rain"),
    ] {
        let r = check(all(path, text, old, "forecast"), "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{path} {text}: {r:?}");
    }
}
#[test]
fn php_dynamic_capturing_reflection_references_and_named_arguments_are_unsupported() {
    for text in [
        "<?php function f($rain) { global $other; return $rain; }",
        "<?php function f($rain) { $name = 'rain'; return $$name + $rain; }",
        "<?php function f($rain) { return compact('rain') + $rain; }",
        "<?php function f($rain) { return \\compact('rain') + $rain; }",
        "<?php function f($rain) { return (function() use ($rain) { return $rain; })(); }",
        "<?php function f($rain) { return fn() => $rain; }",
        "<?php function f(&$rain) { return $rain; }",
        "<?php function f($rain) { $other =& $rain; return $rain; }",
        "<?php function f($rain) { include 'test.php'; return $rain; }",
        "<?php function f($rain) { return $rain; } f(rain: 1);",
        "<?php function f(#[Attr] $rain) { return $rain; }",
        "<?php function f($rain, $rain) { return $rain; }",
    ] {
        let offsets = text
            .match_indices("$rain")
            .map(|(i, _)| i + 1)
            .collect::<Vec<_>>();
        let r = check(
            selected("A.php", text, "rain", "forecast", &offsets),
            "forecast",
        );
        assert_ne!(r.coverage, Coverage::Verified, "{text}: {r:?}");
    }
}
#[test]
fn ts_unproven_var_destructuring_shorthand_default_and_eval_are_unsupported() {
    for text in [
        "function f(rain: number) { var other = 1; return rain; }",
        "function f(rain: number) { const {other} = x; return rain; }",
        "function f(rain: number) { return { rain }; }",
        "function f(rain = globalValue) { return rain; }",
        "function f(rain: number) { eval('rain'); return rain; }",
        "function f(rain: number) { (eval)('rain'); return rain; }",
        "function f(rain: number) { const g = () => eval('rain'); return rain; }",
        r"function f(rain: number) { let \u0061 = 1; return rain + a; }",
        "function f(rain: number, x: typeof globalValue) { return rain; }",
    ] {
        let offsets = text
            .match_indices("rain")
            .filter_map(|(i, _)| {
                (text.as_bytes().get(i.wrapping_sub(1)) != Some(&b'\'')).then_some(i)
            })
            .collect::<Vec<_>>();
        assert_ne!(
            check(
                selected("A.ts", text, "rain", "forecast", &offsets),
                "forecast"
            )
            .coverage,
            Coverage::Verified,
            "{text}"
        );
    }
}
#[test]
fn php_special_variables_and_promoted_properties_stay_unsupported() {
    for (text, old, new) in [
        (
            "<?php class A { function f() { return $this; } }",
            "this",
            "forecast",
        ),
        ("<?php function f($rain) { return $rain; }", "rain", "this"),
        (
            "<?php class A { function __construct(public int $rain) { echo $rain; } }",
            "rain",
            "forecast",
        ),
    ] {
        assert_ne!(
            check(all("A.php", text, old, new), new).coverage,
            Coverage::Verified,
            "{text}"
        );
    }
}
#[test]
fn methods_are_delegated_and_mixed_symbol_proposals_are_unsupported() {
    for (path, text) in [
        ("A.ts", "class A { rain() {} } const a = new A(); a.rain();"),
        (
            "A.php",
            "<?php class A { function rain() {} } $a = new A(); $a->rain();",
        ),
    ] {
        assert!(
            validate_symbol_rename(
                &[all(path, text, "rain", "forecast")],
                "forecast",
                &AtomicBool::new(false)
            )
            .is_none()
        );
    }
    let text = "class A { rain() { const rain = 1; return rain; } }";
    assert_eq!(
        check(all("A.ts", text, "rain", "forecast"), "forecast").coverage,
        Coverage::Unsupported
    );
}
#[test]
fn invalid_snapshots_syntax_reserved_words_and_cancel_limits_never_verify() {
    let text = "function f(rain: number) { return rain; }";
    let valid = all("A.ts", text, "rain", "forecast");
    let mut invalid = valid.clone();
    invalid.after = Arc::from("different");
    let r = validate_symbol_rename(&[invalid], "forecast", &AtomicBool::new(false));
    assert!(r.is_none_or(|r| r.coverage != Coverage::Verified));
    let mut invalid = valid.clone();
    invalid.edits[0].range.start_character = 1;
    let r = validate_symbol_rename(&[invalid], "forecast", &AtomicBool::new(false));
    assert!(r.is_none_or(|r| r.coverage != Coverage::Verified));
    for new in [
        "class",
        "public",
        "private",
        "interface",
        "eval",
        "arguments",
    ] {
        assert_ne!(
            check(all("A.ts", text, "rain", new), new).coverage,
            Coverage::Verified,
            "reserved binding {new}"
        );
    }
    let c = AtomicBool::new(true);
    let r = validate_symbol_rename(std::slice::from_ref(&valid), "forecast", &c).unwrap();
    assert!(r.issues.iter().any(|i| i.kind == IssueKind::Cancelled));
    let r = validate_symbol_rename_with_limits(
        std::slice::from_ref(&valid),
        "forecast",
        &AtomicBool::new(false),
        ValidationLimits {
            max_nodes: 1,
            max_duration: Duration::from_secs(1),
        },
    )
    .unwrap();
    assert!(r.issues.iter().any(|i| i.kind == IssueKind::Limit));
    let r = validate_symbol_rename(&[valid.clone(), valid], "forecast", &AtomicBool::new(false))
        .unwrap();
    assert_ne!(r.coverage, Coverage::Verified);
    assert!(
        r.issues
            .iter()
            .any(|i| i.kind == IssueKind::InvalidSnapshot)
    );
}
