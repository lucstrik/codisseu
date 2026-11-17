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
fn check(s: &[RenameSnapshot], name: &str) -> RenameValidation {
    validate_property_rename_with_limits(
        s,
        name,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .expect("property proposal")
}
#[test]
fn php_and_ts_properties_preserve_unicode_literals_and_homonyms() {
    for (path, text) in [
        (
            "A.php",
            "<?php /* 😀 */ class A { public string $rain = 'wet'; function read() { return $this->rain; } } class B { public string $rain = 'dry'; } echo (new A())->rain; echo (new B())->rain; // rain\n$s = 'rain';",
        ),
        (
            "A.ts",
            "/* 😀 */ class A { rain: string = 'wet'; read() { return this.rain; } } class B { rain: string = 'dry'; } console.log(new A().rain, new B().rain); // rain\nconst s = 'rain';",
        ),
    ] {
        let offsets = text
            .match_indices("rain")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        let s = selected(
            path,
            text,
            "rain",
            "precipitação",
            &[offsets[0], offsets[1], offsets[3]],
        );
        let r = check(&[s], "precipitação");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_declarations, 1);
        assert_eq!(r.scope, ValidationScope::KnownPropertyBindings);
    }
}
#[test]
fn properties_reject_missing_and_wrong_receiver_references() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public $rain; } class B { public $rain; } echo (new A())->rain; echo (new B())->rain;",
        ),
        (
            "A.ts",
            "class A { rain = 1; } class B { rain = 2; } console.log(new A().rain, new B().rain);",
        ),
    ] {
        let p = text
            .match_indices("rain")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();
        for offsets in [vec![p[0]], vec![p[0], p[2], p[3]]] {
            let r = check(
                &[selected(path, text, "rain", "forecast", &offsets)],
                "forecast",
            );
            assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
            assert!(r.conflicts.iter().any(|c| matches!(
                c.kind,
                ConflictKind::MissingReference | ConflictKind::BindingChanged
            )));
        }
    }
}
#[test]
fn property_collisions_are_case_sensitive_in_php_and_include_ts_methods() {
    for (path, text) in [
        ("A.php", "<?php class A { public $rain; public $forecast; }"),
        ("A.ts", "class A { rain = 1; forecast() {} }"),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::DuplicateMember)
        );
    }
    let r = check(
        &[all(
            "A.php",
            "<?php class A { public $rain; public $Rain; function rain() {} } echo (new A())->rain;",
            "rain",
            "forecast",
        )],
        "forecast",
    );
    assert_ne!(
        r.coverage,
        Coverage::Verified,
        "function edit must not be accepted as property"
    );
    let text =
        "<?php class A { public $rain; public $Rain; function rain() {} } echo (new A())->rain;";
    let p = text
        .match_indices("rain")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(
        check(
            &[selected("A.php", text, "rain", "forecast", &[p[0], p[2]])],
            "forecast"
        )
        .coverage,
        Coverage::Verified
    );
}
#[test]
fn inherited_properties_and_static_fields_use_known_graph() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public string $rain = 'wet'; } class B extends A { function read() { return $this->rain; } } echo (new B())->rain;",
        ),
        (
            "A.ts",
            "class A { rain: string = 'wet'; } class B extends A { read() { return this.rain; } } console.log(new B().rain);",
        ),
        (
            "A.php",
            "<?php class A { public static string $rain = 'wet'; static function read() { return self::$rain; } } class B extends A {} echo B::$rain;",
        ),
        (
            "A.ts",
            "class A { static rain: string = 'wet'; static read() { return this.rain; } } class B extends A {} console.log(B.rain);",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn inherited_collisions_and_redeclarations_are_not_silently_accepted() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public $forecast; } class B extends A { public $rain; }",
        ),
        (
            "A.ts",
            "class A { forecast = 1; } class B extends A { rain = 2; }",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    }
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public $rain; } class B extends A { public $rain; }",
        ),
        (
            "A.ts",
            "class A { rain = 1; } class B extends A { rain = 2; }",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    }
}
#[test]
fn private_fields_work_locally_and_unknown_inheritance_does_not() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { private $rain; function read() { return $this->rain; } }",
        ),
        (
            "A.ts",
            "class A { private rain = 1; read() { return this.rain; } }",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    }
    for (path, text) in [
        ("A.php", "<?php class A extends External { public $rain; }"),
        ("A.ts", "class A extends External { rain = 1; }"),
    ] {
        assert_eq!(
            check(&[all(path, text, "rain", "forecast")], "forecast").coverage,
            Coverage::Unsupported
        );
    }
}
#[test]
fn property_parameters_and_imports_bind_across_files() {
    let php = [
        all(
            "/p/A.php",
            "<?php namespace App; class A { public $rain; }",
            "rain",
            "forecast",
        ),
        all(
            "/p/main.php",
            "<?php use App\\A as Weather; function f(Weather $value) { return $value->rain; } echo f(new Weather());",
            "rain",
            "forecast",
        ),
    ];
    let r = check(&php, "forecast");
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let ts = [
        all(
            "/p/A.ts",
            "export class A { rain = 1; }",
            "rain",
            "forecast",
        ),
        all(
            "/p/main.ts",
            "import { A as Weather } from './A'; function f(value: Weather) { return value.rain; } console.log(f(new Weather()));",
            "rain",
            "forecast",
        ),
    ];
    let r = check(&ts, "forecast");
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
}
#[test]
fn dynamic_promoted_and_structural_properties_are_explicitly_unsupported() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public $rain; function read($name) { return $this->$name; } }",
        ),
        (
            "A.php",
            "<?php class A { public $rain; function __get($name) {} }",
        ),
        (
            "A.php",
            "<?php class A { public $rain; function __get($name) { return 1; } }",
        ),
        (
            "A.php",
            "<?php class A { public $rain; } echo serialize(new A());",
        ),
        (
            "A.ts",
            "class A { rain = 1; } const a = new A(); console.log(a['rain']);",
        ),
        (
            "A.ts",
            "class A { rain = 1; } class B { rain = 2; } function f(value: A) { return value.rain; } console.log(f(new B()));",
        ),
    ] {
        let first = text.find("rain").unwrap();
        let offsets = if path.ends_with("ts") && text.contains("class B") {
            vec![first, text.find("value.rain").unwrap() + 6]
        } else {
            vec![first]
        };
        let r = check(
            &[selected(path, text, "rain", "forecast", &offsets)],
            "forecast",
        );
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
        assert!(!r.issues.is_empty(), "{r:?}");
    }
}
#[test]
fn property_snapshot_identity_limits_cancellation_and_invalid_syntax_are_guarded() {
    let s = all("A.ts", "class A { rain = 1; }", "rain", "forecast");
    let mut changed = s.clone();
    changed.after = Arc::from("class A {} ");
    let r = validate_property_rename_with_limits(
        &[changed],
        "forecast",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    );
    assert!(r.is_none() || r.unwrap().coverage != Coverage::Verified);
    for (cancel, limits) in [
        (true, ValidationLimits::default()),
        (
            false,
            ValidationLimits {
                max_nodes: 1,
                max_duration: Duration::from_secs(1),
            },
        ),
        (
            false,
            ValidationLimits {
                max_nodes: 100,
                max_duration: Duration::ZERO,
            },
        ),
    ] {
        let r = validate_property_rename_with_limits(
            std::slice::from_ref(&s),
            "forecast",
            &AtomicBool::new(cancel),
            limits,
        )
        .unwrap();
        assert_eq!(r.coverage, Coverage::Unsupported);
    }
    let r = check(&[s.clone(), s], "forecast");
    assert_eq!(r.coverage, Coverage::Unsupported);
}
#[test]
fn complete_explicit_property_families_preserve_primitive_contracts() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public string $rain = 'a'; } class B extends A { public string $rain = 'b'; } echo (new A())->rain; echo (new B())->rain;",
        ),
        (
            "A.ts",
            "interface A { rain: string; } class B implements A { rain: string = 'b'; } function f(x: A) { return x.rain; } console.log(f(new B()));",
        ),
        (
            "A.ts",
            "class A { rain: string | number = 'a'; } class B extends A { rain: number | string = 'b'; } console.log(new B().rain);",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
        assert_eq!(r.checked_declarations, 2);
    }
}
#[test]
fn property_families_reject_partial_or_divergent_contracts() {
    let text = "interface A { rain: string; } class B implements A { rain: string = 'b'; }";
    let r = check(
        &[selected(
            "A.ts",
            text,
            "rain",
            "forecast",
            &[text.find("rain").unwrap()],
        )],
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::OverrideChanged)
    );
    for (path, text) in [
        (
            "A.php",
            "<?php class A { public string $rain = 'a'; } class B extends A { public int $rain = 1; }",
        ),
        (
            "A.ts",
            "interface A { readonly rain: string; } class B implements A { rain: string = 'b'; }",
        ),
        (
            "A.ts",
            "interface A { rain?: string; } class B implements A { rain: string = 'b'; }",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    }
}
#[test]
fn promoted_parameters_have_both_local_and_member_bindings() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) { echo $rain; } function read() { return $this->rain; } } echo (new A('wet'))->rain;",
        ),
        (
            "A.ts",
            "class A { constructor(public rain: string) { console.log(rain); } read() { return this.rain; } } console.log(new A('wet').rain);",
        ),
        (
            "A.ts",
            "class A { constructor(readonly rain: string) {} read() { return this.rain; } } console.log(new A('wet').rain);",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn promoted_omissions_captures_nested_scopes_and_named_arguments_are_guarded() {
    for (path, text) in [
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) { echo $rain; } }",
        ),
        (
            "A.ts",
            "class A { constructor(public rain: string) { console.log(rain); } }",
        ),
    ] {
        let r = check(
            &[selected(
                path,
                text,
                "rain",
                "forecast",
                &[text.find("rain").unwrap()],
            )],
            "forecast",
        );
        assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    }
    for (path, text) in [
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) { $forecast = 'x'; echo $rain; } }",
        ),
        (
            "A.ts",
            "class A { constructor(public rain: string) { const forecast = 'x'; console.log(rain); } }",
        ),
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) { $f = function() use ($rain) { return $rain; }; } }",
        ),
        (
            "A.ts",
            "class A { constructor(public rain: string) { const f = () => rain; } }",
        ),
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) {} } new Unknown(rain: 'x');",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
#[test]
fn promoted_php_named_arguments_follow_known_constructor_and_preserve_homonyms() {
    let text = "<?php class A { function __construct(public string $rain) { echo $rain; } } class B extends A {} function other($rain) {} new A(rain: 'x'); new B(rain: 'y'); other(rain: 'z');";
    let offsets = text
        .match_indices("rain")
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    let r = check(
        &[selected(
            "A.php",
            text,
            "rain",
            "forecast",
            &[offsets[0], offsets[1], offsets[3], offsets[4]],
        )],
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = check(
        &[selected(
            "A.php",
            text,
            "rain",
            "forecast",
            &[offsets[0], offsets[1], offsets[3]],
        )],
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
    assert!(
        r.conflicts
            .iter()
            .any(|c| c.kind == ConflictKind::MissingReference)
    );
    let r = check(
        &[selected(
            "A.php",
            text,
            "rain",
            "forecast",
            &[offsets[0], offsets[1], offsets[3], offsets[4], offsets[5]],
        )],
        "forecast",
    );
    assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    let text = "<?php class A { function __construct(public string $rain) {} } class B extends A { function __construct(string $value) { parent::__construct(rain: $value); } }";
    let r = check(&[all("A.php", text, "rain", "forecast")], "forecast");
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let r = check(
        &[selected(
            "A.php",
            text,
            "rain",
            "forecast",
            &[text.find("rain").unwrap()],
        )],
        "forecast",
    );
    assert_eq!(r.coverage, Coverage::Rejected, "{r:?}");
}
#[test]
fn property_multiple_implementations_form_one_family_and_require_concrete_members() {
    let text = "interface A { rain: string; } class B implements A { rain: string = 'b'; } class C implements A { rain: string = 'c'; } console.log(new B().rain,new C().rain);";
    let r = check(&[all("A.ts", text, "rain", "forecast")], "forecast");
    assert_eq!(r.coverage, Coverage::Verified, "{r:?}");
    let text = "interface A { rain: string; } class B implements A {}";
    let r = check(&[all("A.ts", text, "rain", "forecast")], "forecast");
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
    let text = "class A { rain: string = 'a'; } class B { rain: string = 'b'; }";
    let r = check(&[all("A.ts", text, "rain", "forecast")], "forecast");
    assert_eq!(r.coverage, Coverage::Unsupported, "{r:?}");
}
#[test]
fn promoted_shadowed_locals_shorthand_and_local_reflection_cannot_be_certified() {
    for (path, text) in [
        (
            "A.ts",
            "class A { constructor(public rain: string) { { let rain = 1; console.log(rain); } console.log(rain); } }",
        ),
        (
            "A.ts",
            "class A { constructor(public rain: string) { const value = { rain }; console.log(value); } }",
        ),
        (
            "A.php",
            "<?php class A { function __construct(public string $rain) { compact('rain'); } }",
        ),
    ] {
        let r = check(&[all(path, text, "rain", "forecast")], "forecast");
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
        assert!(!r.issues.is_empty(), "{r:?}");
        let r = check(
            &[selected(
                path,
                text,
                "rain",
                "forecast",
                &[text.find("rain").unwrap()],
            )],
            "forecast",
        );
        assert_ne!(r.coverage, Coverage::Verified, "{r:?}");
    }
}
