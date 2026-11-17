use codisseu_syntax::{
    Coverage, RenameSnapshot, TextRange, ValidationLimits, plan_class_inheritor_renames,
    propose_global_rename_with_limits, validate_rename,
};
use std::sync::{Arc, atomic::AtomicBool};
fn snapshot(path: &str, text: &str) -> RenameSnapshot {
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(text),
        edits: vec![],
    }
}
fn root(files: &[RenameSnapshot], old: &str, new: &str) -> Vec<RenameSnapshot> {
    let f = &files[0];
    let offset = f.before.find(old).unwrap() as u32;
    propose_global_rename_with_limits(
        files,
        &f.path,
        &TextRange {
            start_line: 0,
            start_character: offset,
            end_line: 0,
            end_character: offset,
        },
        new,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap()
}
fn plan(
    files: &[RenameSnapshot],
    old: &str,
    new: &str,
) -> Vec<codisseu_syntax::ClassInheritorRenameProposal> {
    let primary = root(files, old, new);
    assert_eq!(
        validate_rename(&primary, new, &AtomicBool::new(false)).coverage,
        Coverage::Verified
    );
    plan_class_inheritor_renames(
        &primary,
        new,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
}
#[test]
fn observed_direct_camel_words_and_last_match_preserve_unrelated_and_transitive_names() {
    let text = "<?php class Weather {} class WetWeather extends Weather {} class WeatherChild extends Weather {} class ChildWeatherChild extends WeatherChild {} class AtmosphericWeatherService extends Weather {} class WeatherStation extends Weather {} class WeatherWeather extends Weather {} class Weatherberry extends Weather {} class weatherly extends Weather {} class Custom extends Weather {} class OtherWeather {}";
    let result = plan(&[snapshot("/p/classes.php", text)], "Weather", "Climate");
    let names = result
        .iter()
        .map(|p| (p.old_name.as_str(), p.new_name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec![
            ("WetWeather", "WetClimate"),
            ("WeatherChild", "ClimateChild"),
            ("AtmosphericWeatherService", "AtmosphericClimateService"),
            ("WeatherStation", "ClimateStation"),
            ("WeatherWeather", "WeatherClimate")
        ]
    );
    for item in result {
        assert_eq!(
            validate_rename(&item.snapshots, &item.new_name, &AtomicBool::new(false)).coverage,
            Coverage::Verified
        );
        assert!(item.snapshots[0].after.contains("class Weather {}"));
        assert!(item.snapshots[0].after.contains("class OtherWeather {}"));
    }
}
#[test]
fn nominal_namespace_alias_and_closed_cross_file_calls_follow_child_identity() {
    let files = [
        snapshot("/p/Weather.php", "<?php namespace Domain; class Weather {}"),
        snapshot(
            "/p/WetWeather.php",
            "<?php namespace App; use Domain\\Weather as Sky; class WetWeather extends Sky {}",
        ),
        snapshot(
            "/p/main.php",
            "<?php use App\\WetWeather as Wet; new Wet(); new \\App\\WetWeather(); // WetWeather\n $s='WetWeather';",
        ),
    ];
    let items = plan(&files, "Weather", "Climate");
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.new_name, "WetClimate");
    assert!(
        item.snapshots[2]
            .after
            .contains("use App\\WetClimate as Wet; new Wet(); new \\App\\WetClimate()")
    );
    assert!(item.snapshots[2].after.contains("// WetWeather"));
    assert!(item.snapshots[2].after.contains("'WetWeather'"));
}
#[test]
fn collision_is_kept_for_structured_validator_instead_of_approved_by_planner() {
    let files = [snapshot(
        "/p/a.php",
        "<?php class Weather {} class WetWeather extends Weather {} class WetClimate {} new WetWeather();",
    )];
    let items = plan(&files, "Weather", "Climate");
    assert_eq!(items.len(), 1);
    let report = validate_rename(&items[0].snapshots, "WetClimate", &AtomicBool::new(false));
    assert_eq!(report.coverage, Coverage::Rejected);
    assert!(!report.conflicts.is_empty());
}
#[test]
fn no_child_no_match_and_typescript_are_empty() {
    assert!(
        plan(
            &[snapshot(
                "/p/a.php",
                "<?php class Weather {} class Other extends Weather {}"
            )],
            "Weather",
            "Climate"
        )
        .is_empty()
    );
    assert!(
        plan(
            &[snapshot(
                "/p/a.ts",
                "export class Weather {} export class WetWeather extends Weather {}"
            )],
            "Weather",
            "Climate"
        )
        .is_empty()
    );
}
#[test]
fn invalid_cancelled_limited_and_unresolved_graphs_do_not_produce_guessed_children() {
    let files = [snapshot(
        "/p/a.php",
        "<?php class Weather {} class WetWeather extends Missing {}",
    )];
    let primary = root(&files, "Weather", "Climate");
    assert!(
        plan_class_inheritor_renames(
            &primary,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    let primary = root(
        &[snapshot(
            "/p/a.php",
            "<?php class Weather {} class WetWeather extends Weather {}",
        )],
        "Weather",
        "Climate",
    );
    assert!(
        plan_class_inheritor_renames(
            &primary,
            "Climate",
            &AtomicBool::new(true),
            ValidationLimits::default()
        )
        .is_err()
    );
    assert!(
        plan_class_inheritor_renames(
            &primary,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits {
                max_nodes: 1,
                ..ValidationLimits::default()
            }
        )
        .is_err()
    );
    let mut invalid = primary;
    invalid[0].after = Arc::from("<?php");
    assert!(
        plan_class_inheritor_renames(
            &invalid,
            "Climate",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
}
