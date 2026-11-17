use codisseu_syntax::{Coverage, RenameEdit, RenameSnapshot, TextRange, validate_rename};
use std::sync::{Arc, atomic::AtomicBool};
fn snapshot(path: &str, before: &str, selected: &[usize]) -> RenameSnapshot {
    let starts: Vec<_> = before.match_indices("city").map(|(p, _)| p).collect();
    let mut edits = vec![];
    let mut after = before.to_owned();
    for &index in selected {
        let start = starts[index];
        let prefix = &before[..start];
        let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        edits.push(RenameEdit {
            range: TextRange {
                start_line: line,
                start_character: col,
                end_line: line,
                end_character: col + 4,
            },
            new_text: "place".into(),
        });
    }
    for &index in selected.iter().rev() {
        let start = starts[index];
        after.replace_range(start..start + 4, "place");
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(before),
        after: Arc::from(after),
        edits,
    }
}
fn check(files: &[RenameSnapshot]) -> codisseu_syntax::RenameValidation {
    validate_rename(files, "place", &AtomicBool::new(false))
}
#[test]
fn php_named_argument_tracks_function_parameter_and_preserves_homonyms() {
    let source = "<?php /* 😀 */ function greet(string $city):string{return $city;} greet(city:'Paris'); function other(string $city):string{return $city;} other(city:'Lima');";
    let report = check(&[snapshot("/w/main.php", source, &[0, 1, 2])]);
    assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
    let report = check(&[snapshot("/w/main.php", source, &[0, 1])]);
    assert_eq!(report.coverage, Coverage::Rejected, "{report:#?}");
    let report = check(&[snapshot("/w/main.php", source, &[0, 1, 2, 5])]);
    assert_eq!(report.coverage, Coverage::Rejected, "{report:#?}");
}
#[test]
fn php_named_arguments_follow_namespace_import_alias_across_files() {
    let a = "<?php namespace One; function greet(string $city):string{return $city;}";
    let b = "<?php namespace Two; use function One\\greet as hello; hello(city:'Paris'); function other(string $city){return $city;} other(city:'Lima');";
    let report = check(&[
        snapshot("/w/a.php", a, &[0, 1]),
        snapshot("/w/b.php", b, &[0]),
    ]);
    assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
}
#[test]
fn php_named_arguments_resolve_instance_static_and_inherited_constructor() {
    for source in [
        "<?php class A {function greet(string $city):string{return $city;}} $a=new A(); $a->greet(city:'Paris');",
        "<?php class A {static function greet(string $city):string{return $city;}} A::greet(city:'Paris');",
        "<?php class A {public string $value; function __construct(string $city){$this->value=$city;}} class B extends A {} new B(city:'Paris');",
    ] {
        let report = check(&[snapshot("/w/a.php", source, &[0, 1, 2])]);
        assert_eq!(report.coverage, Coverage::Verified, "{source}: {report:#?}");
    }
}
#[test]
fn php_named_argument_capture_duplicate_and_wrong_callee_are_blocked() {
    for (source, selected) in [
        (
            "<?php function f($city,$place){return $city;} f(city:1,place:2);",
            vec![0, 1, 2],
        ),
        (
            "<?php function f($city){return $city;} f(city:1,city:2);",
            vec![0, 1, 2, 3],
        ),
        (
            "<?php function f($city){return $city;} f(1,city:2);",
            vec![0, 1, 2],
        ),
        (
            "<?php function f($city){return $city;} function other($city){return $city;} other(city:1);",
            vec![0, 1, 4],
        ),
    ] {
        let report = check(&[snapshot("/w/a.php", source, &selected)]);
        assert_ne!(report.coverage, Coverage::Verified, "{source}: {report:#?}");
    }
}
#[test]
fn php_dynamic_unpacking_callbacks_and_override_parameters_remain_unproved() {
    for source in [
        "<?php function f($city){return $city;} $f='f'; $f(city:1);",
        "<?php function f($city){return $city;} $args=['city'=>1]; f(...$args);",
        "<?php function f($city){return $city;} call_user_func_array('f',['city'=>1]);",
        "<?php function f($city){return $city;} (new ReflectionFunction('f'))->invokeArgs(['city'=>1]);",
        "<?php use function call_user_func_array as dispatch; function f($city){return $city;} dispatch('f',['city'=>1]);",
        "<?php class A{function f($city){return $city;}} class B extends A{function f($other){return $other;}} (new A())->f(city:1);",
    ] {
        let report = check(&[snapshot("/w/a.php", source, &[0, 1])]);
        assert_ne!(report.coverage, Coverage::Verified, "{source}: {report:#?}");
    }
}

#[test]
fn own_named_argument_completion_is_exact_idempotent_and_revalidated() {
    let source = "<?php function greet($city){return $city;} greet(city:'Paris'); function other($city){return $city;} other(city:'Lima');";
    let files = vec![snapshot("/w/a.php", source, &[0, 1])];
    let result = codisseu_syntax::complete_php_named_argument_rename(
        &files,
        "place",
        &AtomicBool::new(false),
        codisseu_syntax::ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(result[0].edits.len(), 3);
    assert_eq!(
        result[0].after.as_ref(),
        "<?php function greet($place){return $place;} greet(place:'Paris'); function other($city){return $city;} other(city:'Lima');"
    );
    assert_eq!(check(&result).coverage, Coverage::Verified);
    let repeated = codisseu_syntax::complete_php_named_argument_rename(
        &result,
        "place",
        &AtomicBool::new(false),
        codisseu_syntax::ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(repeated[0].after, result[0].after);
    assert_eq!(repeated[0].edits.len(), 3);
    assert!(
        codisseu_syntax::complete_php_named_argument_rename(
            &files,
            "place",
            &AtomicBool::new(true),
            codisseu_syntax::ValidationLimits::default()
        )
        .is_err()
    );
    let mut invalid = files;
    invalid[0].after = Arc::from("unrelated bytes");
    assert!(
        codisseu_syntax::complete_php_named_argument_rename(
            &invalid,
            "place",
            &AtomicBool::new(false),
            codisseu_syntax::ValidationLimits::default()
        )
        .is_err()
    );
}
