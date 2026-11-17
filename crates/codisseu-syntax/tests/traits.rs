use codisseu_syntax::{
    Coverage, RenameEdit, RenameSnapshot, TextRange, ValidationLimits, validate_rename,
    validate_trait_rename_with_limits,
};
use std::sync::{Arc, atomic::AtomicBool};
fn file(path: &str, marked: &str, new: &str) -> RenameSnapshot {
    let mut before = String::new();
    let mut ranges = vec![];
    let mut remaining = marked;
    while let Some(start) = remaining.find("[[") {
        before.push_str(&remaining[..start]);
        let rest = &remaining[start + 2..];
        let end = rest.find("]]").unwrap();
        let offset = before.len();
        before.push_str(&rest[..end]);
        ranges.push(offset..before.len());
        remaining = &rest[end + 2..];
    }
    before.push_str(remaining);
    let edits = ranges
        .iter()
        .map(|r| {
            let prefix = &before[..r.start];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
            let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
            RenameEdit {
                range: TextRange {
                    start_line: line,
                    start_character: col,
                    end_line: line,
                    end_character: col + before[r.clone()].encode_utf16().count() as u32,
                },
                new_text: new.into(),
            }
        })
        .collect();
    let mut after = before.clone();
    for range in ranges.iter().rev() {
        after.replace_range(range.clone(), new);
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(before),
        after: Arc::from(after),
        edits,
    }
}
fn checked(marked: &str, new: &str) -> codisseu_syntax::RenameValidation {
    validate_rename(
        &[file("/p/a.php", marked, new)],
        new,
        &AtomicBool::new(false),
    )
}
fn verified(marked: &str, new: &str) {
    let report = checked(marked, new);
    assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
}
fn denied(marked: &str, new: &str) {
    let report = checked(marked, new);
    assert_ne!(report.coverage, Coverage::Verified, "{report:#?}");
}
#[test]
fn original_trait_rename_preserves_independent_alias_and_homonyms() {
    verified(
        "<?php trait Rain { public function [[rainfall]](): int { return 1; } } class Weather { use Rain { Rain::[[rainfall]] as drizzle; } } class Other { public function rainfall(): int {return 2;} } (new Weather())->[[rainfall]](); (new Weather())->drizzle(); (new Other())->rainfall(); // rainfall 😀\n $s='rainfall';",
        "forecast",
    );
}
#[test]
fn alias_can_be_renamed_independently_from_source() {
    verified(
        "<?php trait Rain { public function rainfall(): int { return 1; } } class Weather { use Rain { Rain::rainfall as [[drizzle]]; } } (new Weather())->rainfall(); (new Weather())->[[drizzle]]();",
        "forecast",
    );
}
#[test]
fn insteadof_selects_winner_and_retains_loser_alias() {
    verified(
        "<?php trait A {public function [[rainfall]](): int {return 1;}} trait B {public function rainfall(): int {return 2;}} class Weather {use A,B {A::[[rainfall]] insteadof B; B::rainfall as drizzle;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle();",
        "forecast",
    );
    verified(
        "<?php trait A {public function rainfall(): int {return 1;}} trait B {public function [[rainfall]](): int {return 2;}} class Weather {use A,B {A::rainfall insteadof B; B::[[rainfall]] as drizzle;}} (new Weather())->rainfall(); (new Weather())->drizzle();",
        "forecast",
    );
}
#[test]
fn private_protected_and_visibility_only_adaptations_have_effective_access() {
    verified(
        "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {rainfall as private [[drizzle]];} public function sample(): int {return $this->[[drizzle]]();}} (new Weather())->sample();",
        "forecast",
    );
    verified(
        "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {rainfall as protected [[drizzle]];}} class Child extends Weather {public function sample(): int {return $this->[[drizzle]]();}} (new Child())->sample();",
        "forecast",
    );
    verified(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {[[rainfall]] as protected;} public function sample(): int {return $this->[[rainfall]]();}}",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {rainfall as private [[drizzle]];}} (new Weather())->[[drizzle]]();",
        "forecast",
    );
}
#[test]
fn nested_traits_and_source_aliases_follow_independent_identities() {
    verified(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} trait Wet {use Rain {Rain::[[rainfall]] as drizzle;}} class Weather {use Wet {Wet::drizzle as shower;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle(); (new Weather())->shower();",
        "forecast",
    );
    verified(
        "<?php trait Rain {public function rainfall(): int {return 1;}} trait Wet {use Rain {Rain::rainfall as [[drizzle]];}} class Weather {use Wet {Wet::[[drizzle]] as shower;}} (new Weather())->rainfall(); (new Weather())->[[drizzle]](); (new Weather())->shower();",
        "forecast",
    );
}
#[test]
fn namespace_imports_and_typed_receivers_work_across_files() {
    let files = [
        file(
            "/p/t.php",
            "<?php namespace Domain; trait Rain {public function [[rainfall]](): int {return 1;}}",
            "forecast",
        ),
        file(
            "/p/c.php",
            "<?php namespace App; use Domain\\Rain as R; class Weather {use R {R::[[rainfall]] as drizzle;}} function sample(Weather $w): int {return $w->[[rainfall]]()+$w->drizzle();}",
            "forecast",
        ),
    ];
    let report = validate_rename(&files, "forecast", &AtomicBool::new(false));
    assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
}
#[test]
fn omissions_wrong_aliases_and_collisions_are_rejected() {
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::rainfall as drizzle;}} (new Weather())->[[rainfall]]();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} (new Weather())->rainfall();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as [[drizzle]];}} (new Weather())->[[rainfall]]();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as forecast;}} (new Weather())->[[rainfall]]();",
        "forecast",
    );
    denied(
        "<?php trait A {public function [[rainfall]](): int {return 1;}} trait B {public function rainfall(): int {return 2;}} class Weather {use A,B {A::[[rainfall]] as drizzle;}}",
        "forecast",
    );
}
#[test]
fn class_override_family_must_remain_complete_and_compatible() {
    let full = "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;} public function [[rainfall]](): int {return 2;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle();";
    verified(full, "forecast");
    denied(
        &full.replace(
            "public function [[rainfall]](): int {return 2;}",
            "public function rainfall(): int {return 2;}",
        ),
        "forecast",
    );
    denied(
        &full.replace(
            "public function [[rainfall]](): int {return 2;}",
            "public function [[rainfall]](): string {return '2';}",
        ),
        "forecast",
    );
}
#[test]
fn shared_this_binding_dynamic_calls_unknown_receivers_and_reflection_are_guarded() {
    verified(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;} public function sample(): int {return $this->[[rainfall]]();}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} (new Weather())->sample();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;} public function sample(): int {return $this->[[rainfall]]();}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} class Other {use Rain; public function rainfall(): int {return 2;}}",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} $unknown->[[rainfall]]();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} (new Weather())->$method();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} call_user_func([new Weather(),'rainfall']);",
        "forecast",
    );
}
#[test]
fn static_alias_calls_and_inherited_aliases_are_complete() {
    verified(
        "<?php trait Rain {public static function rainfall(): int {return 1;}} class Weather {use Rain {Rain::rainfall as [[drizzle]];}} class Child extends Weather {} Weather::[[drizzle]](); Child::[[drizzle]]();",
        "forecast",
    );
    denied(
        "<?php trait Rain {public static function rainfall(): int {return 1;}} class Weather {use Rain {Rain::rainfall as [[drizzle]];}} class Child extends Weather {} Weather::[[drizzle]](); Child::drizzle();",
        "forecast",
    );
}
#[test]
fn limits_cancel_invalid_snapshots_and_missing_traits_never_verify() {
    let snapshot = file(
        "/p/a.php",
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}}",
        "forecast",
    );
    let report = validate_trait_rename_with_limits(
        std::slice::from_ref(&snapshot),
        "forecast",
        &AtomicBool::new(true),
        ValidationLimits::default(),
    )
    .unwrap();
    assert_ne!(report.coverage, Coverage::Verified);
    let report = validate_trait_rename_with_limits(
        std::slice::from_ref(&snapshot),
        "forecast",
        &AtomicBool::new(false),
        ValidationLimits {
            max_nodes: 1,
            ..ValidationLimits::default()
        },
    )
    .unwrap();
    assert_ne!(report.coverage, Coverage::Verified);
    let mut invalid = snapshot;
    invalid.after = Arc::from("<?php");
    assert_ne!(
        validate_rename(&[invalid], "forecast", &AtomicBool::new(false)).coverage,
        Coverage::Verified
    );
    denied(
        "<?php class Weather {use Missing {Missing::rainfall as [[drizzle]];}} (new Weather())->[[drizzle]]();",
        "forecast",
    );
}

#[test]
fn semantic_trait_proposer_discovers_exact_alias_selector_call_and_override_edits() {
    let cases = [
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle();",
        "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {Rain::rainfall as [[drizzle]];}} (new Weather())->rainfall(); (new Weather())->[[drizzle]]();",
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;} public function [[rainfall]](): int {return 2;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle();",
    ];
    for marked in cases {
        let expected = file("/p/a.php", marked, "forecast");
        let original = RenameSnapshot {
            after: expected.before.clone(),
            edits: vec![],
            ..expected.clone()
        };
        for edit in &expected.edits {
            let proposal = codisseu_syntax::propose_trait_rename_with_limits(
                std::slice::from_ref(&original),
                &original.path,
                &edit.range,
                "forecast",
                &AtomicBool::new(false),
                ValidationLimits::default(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(proposal[0].after, expected.after);
            assert_eq!(
                proposal[0]
                    .edits
                    .iter()
                    .map(|e| (&e.range, &e.new_text))
                    .collect::<Vec<_>>(),
                expected
                    .edits
                    .iter()
                    .map(|e| (&e.range, &e.new_text))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                validate_rename(&proposal, "forecast", &AtomicBool::new(false)).coverage,
                Coverage::Verified
            );
        }
    }
}
#[test]
fn preparation_identifies_trait_methods_aliases_and_selectors_without_approving_them() {
    let expected = file(
        "/p/a.php",
        "<?php trait Rain {function [[rainfall]](): int {return 1;}} class Weather {use Rain {Rain::[[rainfall]] as [[drizzle]];}}",
        "forecast",
    );
    for edit in expected.edits {
        assert!(
            codisseu_syntax::prepare_trait_rename_with_limits(
                &expected.path,
                &expected.before,
                edit.range.start_line,
                edit.range.start_character,
                &AtomicBool::new(false),
                ValidationLimits::default()
            )
            .unwrap()
            .is_some()
        );
    }
    assert!(
        codisseu_syntax::prepare_trait_rename_with_limits(
            &expected.path,
            &expected.before,
            0,
            0,
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn concrete_class_method_resolves_competing_trait_implementations() {
    verified(
        "<?php trait A {public function [[rainfall]](): int {return 1;}} trait B {public function [[rainfall]](): int {return 2;}} class Weather {use A,B {A::[[rainfall]] as drizzle;} public function [[rainfall]](): int {return 3;}} (new Weather())->[[rainfall]](); (new Weather())->drizzle();",
        "forecast",
    );
}
#[test]
fn final_static_and_contradictory_adaptations_are_guarded() {
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;}} class ParentWeather {final public function rainfall(): int {return 2;}} class Weather extends ParentWeather {use Rain {Rain::[[rainfall]] as drizzle;}}",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;} public static function sample(): int {return self::[[rainfall]]();}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}}",
        "forecast",
    );
    denied(
        "<?php trait Rain {public function [[rainfall]](): int {return 1;} public static function sample(): int {return $this->[[rainfall]]();}} class Weather {use Rain {Rain::[[rainfall]] as drizzle;}}",
        "forecast",
    );
    denied(
        "<?php trait A {public function [[rainfall]](): int {return 1;}} trait B {public function rainfall(): int {return 2;}} class Weather {use A,B {A::[[rainfall]] insteadof B; B::rainfall insteadof A;}}",
        "forecast",
    );
}
#[test]
fn abstract_trait_requirement_and_effective_alias_visibility_are_preserved() {
    verified(
        "<?php trait Rain {abstract public function [[rainfall]](): int;} class Weather {use Rain {Rain::[[rainfall]] as drizzle;} public function [[rainfall]](): int {return 1;} public function drizzle(): int {return 2;}} (new Weather())->[[rainfall]]();",
        "forecast",
    );
    verified(
        "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {Rain::rainfall as protected [[drizzle]];}} class Child extends Weather {protected function [[drizzle]](): int {return 2;} public function sample(): int {return $this->[[drizzle]]();}}",
        "forecast",
    );
}

#[test]
fn corpus_preparation_resolves_external_alias_but_leaves_ordinary_methods_to_provider() {
    let marked = "<?php trait Rain {public function rainfall(): int {return 1;}} class Weather {use Rain {Rain::rainfall as drizzle;} public function ordinary(): int {return 2;}} function sample(Weather $w): int {return $w->[[drizzle]]();}";
    let expected = file("/p/a.php", marked, "forecast");
    let original = RenameSnapshot {
        after: expected.before.clone(),
        edits: vec![],
        ..expected.clone()
    };
    let origin = &expected.edits[0].range;
    assert_eq!(
        codisseu_syntax::prepare_trait_rename_in_corpus_with_limits(
            std::slice::from_ref(&original),
            &original.path,
            origin,
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .unwrap()
        .as_deref(),
        Some("drizzle")
    );
    let offset = original.before.find("ordinary").unwrap() as u32;
    assert!(
        codisseu_syntax::prepare_trait_rename_in_corpus_with_limits(
            std::slice::from_ref(&original),
            &original.path,
            &TextRange {
                start_line: 0,
                start_character: offset,
                end_line: 0,
                end_character: offset
            },
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .unwrap()
        .is_none()
    );
}
