use codisseu_syntax::{
    ConflictKind, Coverage, RenameEdit, RenameSnapshot, RenameValidation, TextRange,
    validate_method_rename,
};
use std::sync::{Arc, atomic::AtomicBool};

// Explicit fixtures contain only method-name occurrences; this helper builds the
// provider proposal. Production validation neither searches nor generates edits.
fn proposal(path: &str, before: &str) -> RenameSnapshot {
    let mut after = before.to_owned();
    let positions = before
        .match_indices("rainfall")
        .map(|(p, _)| p)
        .collect::<Vec<_>>();
    let edits = positions
        .iter()
        .map(|&p| {
            let prefix = &before[..p];
            let line = prefix.bytes().filter(|&b| b == b'\n').count() as u32;
            let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
            RenameEdit {
                range: TextRange {
                    start_line: line,
                    start_character: column,
                    end_line: line,
                    end_character: column + 8,
                },
                new_text: "precipitation".into(),
            }
        })
        .collect();
    for &p in positions.iter().rev() {
        after.replace_range(p..p + 8, "precipitation");
    }
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(before),
        after: Arc::from(after),
        edits,
    }
}
fn context(path: &str, text: &str) -> RenameSnapshot {
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(text),
        edits: vec![],
    }
}
fn verify(files: &[RenameSnapshot]) -> RenameValidation {
    validate_method_rename(files, "precipitation", &AtomicBool::new(false))
}
fn expect(files: &[RenameSnapshot], expected: Coverage) -> RenameValidation {
    let report = verify(files);
    assert_eq!(report.coverage, expected, "{report:#?}");
    report
}
#[test]
fn named_interface_contracts_resolve_across_alias_imports_and_report_related_declarations() {
    for files in [
        vec![
            proposal(
                "/p/Port.php",
                "<?php namespace Contract; interface WeatherPort { public function rainfall(string $city): string; }",
            ),
            proposal(
                "/p/Weather.php",
                "<?php namespace App; use Contract\\WeatherPort as Port; class Weather implements Port { public function rainfall(string $city): string { return $city; } }",
            ),
            proposal(
                "/p/main.php",
                "<?php /* 🌧 é */ $weather->rainfall('Paris');",
            ),
        ],
        vec![
            proposal(
                "/p/port.ts",
                "export interface WeatherPort { rainfall(city: string): string; }",
            ),
            proposal(
                "/p/weather.ts",
                "import type { WeatherPort as Port } from './port'; export class Weather implements Port { rainfall(city: string): string { return city; } }",
            ),
            proposal("/p/main.ts", "/* 🌧 é */ weather.rainfall('Paris');"),
        ],
    ] {
        let r = expect(&files, Coverage::Verified);
        assert_eq!(r.checked_declarations, 2);
        assert_eq!(r.checked_override_pairs, 1);
        assert_eq!(r.related_methods.len(), 2);
        assert!(
            r.related_methods
                .iter()
                .any(|m| m.container_name == "WeatherPort")
        );
    }
}
#[test]
fn interfaces_with_multiple_parents_and_diamonds_are_fully_connected_without_cycle_false_positive()
{
    for (path, text) in [
        (
            "a.php",
            "<?php interface Root { function rainfall(): string; } interface Left extends Root { function rainfall(): string; } interface Right extends Root { function rainfall(): string; } interface Both extends Left, Right {} class Impl implements Both { function rainfall(): string { return ''; } }",
        ),
        (
            "a.ts",
            "interface Root { rainfall(): string; } interface Left extends Root { rainfall(): string; } interface Right extends Root { rainfall(): string; } interface Both extends Left, Right {} class Impl implements Both { rainfall(): string { return ''; } }",
        ),
    ] {
        let r = expect(&[proposal(path, text)], Coverage::Verified);
        assert_eq!(r.checked_override_pairs, 6);
        assert_eq!(r.related_methods.len(), 4);
    }
}
#[test]
fn inherited_implementation_and_multiple_interfaces_require_the_entire_proposal() {
    for (path, interface, base, derived) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(): string; }",
            "<?php class Base { function rainfall(): string { return ''; } }",
            "<?php class Derived extends Base implements Port {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; }",
            "class Base { rainfall(): string { return ''; } }",
            "class Derived extends Base implements Port {}",
        ),
    ] {
        let ext = path.rsplit('.').next().unwrap();
        let files = [
            proposal(&format!("port.{ext}"), interface),
            proposal(&format!("base.{ext}"), base),
            context(&format!("derived.{ext}"), derived),
        ];
        assert_eq!(expect(&files, Coverage::Verified).checked_override_pairs, 1);
        let missing = [
            context(&format!("port.{ext}"), interface),
            files[1].clone(),
            files[2].clone(),
        ];
        let r = expect(&missing, Coverage::Rejected);
        assert!(
            r.conflicts
                .iter()
                .any(|c| c.kind == ConflictKind::OverrideChanged)
        );
    }
}
#[test]
fn simple_php_trait_use_and_transitive_use_preserve_nominal_member_namespaces() {
    let r = expect(
        &[
            proposal(
                "/p/Rain.php",
                "<?php namespace Contract; trait Rain { function rainfall(): string { return 'rain'; } }",
            ),
            context(
                "/p/Wrapper.php",
                "<?php namespace Contract; trait Wrapper { use Rain; }",
            ),
            context(
                "/p/Weather.php",
                "<?php namespace App; use Contract\\Wrapper as WeatherTrait; class Weather { use WeatherTrait; }",
            ),
            proposal("/p/main.php", "<?php $weather->rainfall();"),
        ],
        Coverage::Verified,
    );
    assert_eq!(r.checked_declarations, 1);
    assert_eq!(r.checked_override_pairs, 0);
    assert!(r.related_methods.is_empty());
    let r = expect(
        &[
            proposal(
                "trait.php",
                "<?php trait Rain { function rainfall(): string { return ''; } }",
            ),
            proposal(
                "class.php",
                "<?php class Weather { use Rain; function rainfall(): string { return ''; } }",
            ),
        ],
        Coverage::Verified,
    );
    assert_eq!(r.checked_override_pairs, 1);
}
#[test]
fn traits_and_interfaces_detect_new_inherited_or_coinherited_name_collisions() {
    for (path, text) in [
        (
            "a.php",
            "<?php trait Rain { function rainfall(): string { return ''; } } trait Other { function precipitation(): string { return ''; } } class Weather { use Rain, Other; }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(): string; } interface Other { function precipitation(): string; } interface Both extends Port, Other {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } interface Other { precipitation(): string; } interface Both extends Port, Other {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; precipitation: string; }",
        ),
    ] {
        let r = expect(&[proposal(path, text)], Coverage::Rejected);
        assert!(r.conflicts.iter().any(|c| matches!(
            c.kind,
            ConflictKind::InheritedMember | ConflictKind::DuplicateMember
        )));
    }
}
#[test]
fn unresolved_ambiguous_cyclic_wrong_kind_and_adapted_relations_fail_closed() {
    for (path, text) in [
        (
            "a.php",
            "<?php trait Rain { function rainfall(): string { return ''; } } class Weather { use Rain { rainfall as alias; } }",
        ),
        (
            "a.php",
            "<?php interface Port extends Other { function rainfall(): string; } interface Other extends Port {}",
        ),
        (
            "a.php",
            "<?php trait Rain { use Other; function rainfall(): string { return ''; } } trait Other { use Rain; }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(): string; } class Weather extends Port {}",
        ),
        (
            "a.php",
            "<?php trait Rain { function rainfall(): string { return ''; } } class Weather implements Rain {}",
        ),
        (
            "a.php",
            "<?php class Rain { function rainfall(): string { return ''; } } class Weather { use Rain; }",
        ),
        (
            "a.ts",
            "interface Port extends Other { rainfall(): string; } interface Other extends Port {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Weather extends Port {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } interface Port { other(): string; }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Weather implements Missing {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Weather extends factory() {}",
        ),
        (
            "a.ts",
            "interface Port<T> { rainfall(): string; } class Weather implements Port<string> { rainfall(): string { return ''; } }",
        ),
    ] {
        expect(&[proposal(path, text)], Coverage::Unsupported);
    }
}
#[test]
fn interface_contract_differences_optional_members_and_overloads_are_not_certified() {
    for (path, text) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(): string { return ''; } }",
        ),
        ("a.ts", "interface Port { public rainfall(): string; }"),
        ("a.ts", "interface Port { override rainfall(): string; }"),
        (
            "a.php",
            "<?php interface Port { function rainfall(): string; } class Weather implements Port { function rainfall(): int { return 1; } }",
        ),
        (
            "a.php",
            "<?php interface Port { protected function rainfall(): string; }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Weather implements Port { rainfall(): number { return 1; } }",
        ),
        ("a.ts", "interface Port { rainfall?(): string; }"),
        (
            "a.ts",
            "interface Port { rainfall(city: string): string; rainfall(city: number): string; }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; [key: string]: unknown; }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Weather implements Port { override rainfall(): string { return ''; } }",
        ),
    ] {
        expect(&[proposal(path, text)], Coverage::Unsupported);
    }
}
#[test]
fn missing_concrete_implementation_and_competing_trait_bodies_are_unsupported() {
    for (path, text) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(): string; } class Missing implements Port {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Missing implements Port {}",
        ),
        (
            "a.ts",
            "interface Port { rainfall(): string; } class Missing implements Port { static rainfall(): string { return ''; } }",
        ),
        (
            "a.php",
            "<?php trait A { function rainfall(): string { return ''; } } trait B { function rainfall(): string { return ''; } } class C { use A, B; }",
        ),
    ] {
        expect(&[proposal(path, text)], Coverage::Unsupported);
    }
}
#[test]
fn explicit_builtin_union_nullable_and_array_contracts_are_compared_structurally() {
    for (path, text) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(?string $city): string|int; } class Weather implements Port { function rainfall(string|null $city): int|string { return ''; } }",
        ),
        (
            "a.php",
            "<?php trait Rain { function rainfall(array $values): ?string { return null; } } class Weather { use Rain; function rainfall(array $values): string|null { return null; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(values: (string | number)[]): string | null; } class Weather implements Port { rainfall(values: (number | string)[]): null | string { return null; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(value: string | undefined): string[][]; } class Weather implements Port { rainfall(value: undefined | string): string[][] { return []; } }",
        ),
    ] {
        assert_eq!(
            expect(&[proposal(path, text)], Coverage::Verified).checked_override_pairs,
            1
        );
    }
}
#[test]
fn composite_contracts_do_not_infer_named_alias_structural_or_invalid_types() {
    for (path, text) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(): ?void; }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(): string|string; }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(): string&int; }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(): self; }",
        ),
        (
            "a.ts",
            "type Value = string; interface Port { rainfall(): Value; }",
        ),
        ("a.ts", "interface Port { rainfall(): Array<string>; }"),
        ("a.ts", "interface Port { rainfall(): { value: string }; }"),
        ("a.ts", "interface Port { rainfall(): string & number; }"),
        (
            "a.ts",
            "interface Port { rainfall(): string | number; } class Weather implements Port { rainfall(): string | boolean { return ''; } }",
        ),
    ] {
        assert_ne!(
            verify(&[proposal(path, text)]).coverage,
            Coverage::Verified,
            "{text}"
        );
    }
}
fn named_proposal(path: &str, source: &str, new_name: &str) -> RenameSnapshot {
    let mut snapshot = proposal(path, source);
    snapshot.after = Arc::from(snapshot.after.replace("precipitation", new_name));
    for edit in &mut snapshot.edits {
        edit.new_text = new_name.into();
    }
    snapshot
}
#[test]
fn unicode_identifier_edits_preserve_exact_utf16_ranges_and_detect_unicode_collisions() {
    for (path, source) in [
        (
            "a.php",
            "<?php /* 🌧 é */ interface Port { function rainfall(): string; } class Weather implements Port { function rainfall(): string { return ''; } } $a->rainfall();",
        ),
        (
            "a.ts",
            "/* 🌧 é */ interface Port { rainfall(): string; } class Weather implements Port { rainfall(): string { return ''; } } weather.rainfall();",
        ),
    ] {
        for name in ["precipitação", "prévision", "雨量", "chuva\u{301}"] {
            let snapshot = named_proposal(path, source, name);
            let report = validate_method_rename(&[snapshot], name, &AtomicBool::new(false));
            assert_eq!(
                report.coverage,
                Coverage::Verified,
                "{path}, {name}: {report:#?}"
            );
            assert_eq!(report.checked_override_pairs, 1);
        }
    }
    for (path, source) in [
        (
            "a.php",
            "<?php class Weather { function rainfall(): string { return ''; } function previsão(): string { return ''; } }",
        ),
        (
            "a.ts",
            "class Weather { rainfall(): string { return ''; } previsão(): string { return ''; } }",
        ),
    ] {
        let snapshot = named_proposal(path, source, "previsão");
        let report = validate_method_rename(&[snapshot], "previsão", &AtomicBool::new(false));
        assert_eq!(report.coverage, Coverage::Rejected, "{report:#?}");
    }
}
#[test]
fn candidate_precheck_and_grammar_never_accept_identifier_injection_or_escaped_aliases() {
    for name in [
        "",
        "0bad",
        "a b",
        "a\nb",
        "a.b",
        "a\\u0062",
        "f(){}evil",
        "a\u{202e}b",
        "a\u{00a0}b",
    ] {
        assert!(
            codisseu_syntax::validate_rename_candidate(name).is_err(),
            "{name:?}"
        );
    }
    assert!(codisseu_syntax::validate_rename_candidate(&"é".repeat(513)).is_err());
    // PHP rejects `$rain` only during grammar validation. Emoji and an initial
    // combining mark must fail the lexical filter as Tree-sitter is permissive.
    for name in ["🌧", "\u{301}bad", "$rain"] {
        let path = if name == "$rain" { "a.php" } else { "a.ts" };
        let text = if path.ends_with("php") {
            "<?php class A { function rainfall() {} }"
        } else {
            "class A { rainfall() {} }"
        };
        let report = validate_method_rename(
            &[named_proposal(path, text, name)],
            name,
            &AtomicBool::new(false),
        );
        assert_ne!(report.coverage, Coverage::Verified, "{name}: {report:?}");
    }
    expect(
        &[proposal(
            "a.ts",
            "class A { rainfall(): string { return ''; } \\u0070recipitation(): string { return ''; } }",
        )],
        Coverage::Unsupported,
    );
}

#[test]
fn optional_rest_and_literal_default_contracts_preserve_exact_callable_shape() {
    for (path, source) in [
        (
            "a.php",
            "<?php class Base { function rainfall(string $city = 'Paris'): string { return $city; } } class Weather extends Base { function rainfall(string $city = 'Paris'): string { return $city; } }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(string ...$cities): string; } class Weather implements Port { function rainfall(string ...$cities): string { return ''; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(city?: string): string; } class Weather implements Port { rainfall(city?: string): string { return city || ''; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(...cities: string[]): string; } class Weather implements Port { rainfall(...cities: string[]): string { return ''; } }",
        ),
        (
            "a.ts",
            "class Base { rainfall(city: string = 'Paris'): string { return city; } } class Weather extends Base { rainfall(city: string = 'Paris'): string { return city; } }",
        ),
    ] {
        let files = [proposal(path, source)];
        assert_eq!(expect(&files, Coverage::Verified).checked_override_pairs, 1);
        let report =
            codisseu_syntax::validate_rename(&files, "precipitation", &AtomicBool::new(false));
        assert_eq!(report.coverage, Coverage::Verified, "{report:#?}");
    }
}

#[test]
fn optional_rest_defaults_do_not_hide_incompatible_or_dynamic_contracts() {
    for (path, source) in [
        (
            "a.php",
            "<?php interface Port { function rainfall(string $city): string; } class Weather implements Port { function rainfall(string ...$city): string { return ''; } }",
        ),
        (
            "a.php",
            "<?php class Base { function rainfall(string $city = 'Paris'): string { return $city; } } class Weather extends Base { function rainfall(string $city = 'London'): string { return $city; } }",
        ),
        (
            "a.php",
            "<?php interface Port { function rainfall(string &$city): string; }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(city?: string): string; } class Weather implements Port { rainfall(city: string): string { return city; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(...city: string): string; }",
        ),
        (
            "a.ts",
            "class Base { rainfall(city: string = getCity()): string { return city; } } class Weather extends Base { rainfall(city: string = getCity()): string { return city; } }",
        ),
        (
            "a.ts",
            "interface Port { rainfall(city: string = 'Paris'): string; }",
        ),
    ] {
        assert_ne!(
            verify(&[proposal(path, source)]).coverage,
            Coverage::Verified,
            "{source}"
        );
    }
}
#[test]
fn abstract_contracts_include_concrete_implementations_and_typed_calls() {
    for (path, source) in [
        (
            "/p/a.php",
            "<?php abstract class Port { abstract public function rainfall(string $city): string; } class Weather extends Port { public function rainfall(string $city): string { return $city; } } function usePort(Port $w): string { return $w->rainfall('Paris'); }",
        ),
        (
            "/p/a.ts",
            "abstract class Port { abstract rainfall(city: string): string; } class Weather extends Port { rainfall(city: string): string { return city; } } function usePort(w: Port): string { return w.rainfall('Paris'); }",
        ),
    ] {
        let files = [proposal(path, source)];
        let r = expect(&files, Coverage::Verified);
        assert_eq!(r.related_methods.len(), 2);
        let full =
            codisseu_syntax::validate_rename(&files, "precipitation", &AtomicBool::new(false));
        assert_eq!(full.coverage, Coverage::Verified, "{full:#?}");
    }
}
#[test]
fn abstract_missing_concrete_implementation_or_partial_family_never_applies() {
    for (path, source) in [
        (
            "/p/a.php",
            "<?php abstract class Port { abstract public function rainfall(string $city): string; } class Weather extends Port {}",
        ),
        (
            "/p/a.ts",
            "abstract class Port { abstract rainfall(city: string): string; } class Weather extends Port {}",
        ),
        (
            "/p/a.ts",
            "class Base { rainfall(city: string): string { return city; } } abstract class Port extends Base { abstract rainfall(city: string): string; } class Weather extends Port {}",
        ),
        (
            "/p/a.php",
            "<?php class Port { abstract public function rainfall(string $city): string; }",
        ),
    ] {
        assert_ne!(
            verify(&[proposal(path, source)]).coverage,
            Coverage::Verified,
            "{source}"
        );
    }
    let files = [
        proposal(
            "/p/base.ts",
            "export abstract class Port { abstract rainfall(city: string): string; }",
        ),
        context(
            "/p/impl.ts",
            "import {Port} from './base'; export class Weather extends Port { rainfall(city: string): string {return city;} }",
        ),
    ];
    assert_ne!(verify(&files).coverage, Coverage::Verified);
}
#[test]
fn local_module_type_aliases_and_generic_method_headers_preserve_equivalent_contracts() {
    for source in [
        "export {}; type Value = string; type Values = Value[]; interface Port { rainfall(city: Value): Values; } class Weather implements Port { rainfall(city: string): string[] {return [city];} }",
        "interface Port { rainfall<T>(city: T): T; } class Weather implements Port { rainfall<U>(city: U): U {return city;} }",
        "interface Port { rainfall<T extends string = string>(city: T): T[]; } class Weather implements Port { rainfall<U extends string = string>(city: U): U[] {return [city];} }",
    ] {
        let files = [proposal("/p/a.ts", source)];
        assert_eq!(expect(&files, Coverage::Verified).checked_override_pairs, 1);
        let r = codisseu_syntax::validate_rename(&files, "precipitation", &AtomicBool::new(false));
        assert_eq!(r.coverage, Coverage::Verified, "{r:#?}");
    }
}
#[test]
fn generic_mismatch_recursive_aliases_and_unresolved_imports_remain_blocked() {
    for source in [
        "interface Port { rainfall<T extends string>(city: T): T; } class Weather implements Port { rainfall<T extends number>(city: T): T {return city;} }",
        "interface Port { rainfall<T>(city: T): T; } class Weather implements Port { rainfall<T,U>(city: T): U {throw 0;} }",
        "export {}; type A=B; type B=A; interface Port {rainfall(): A;}",
        "export {}; type Value=string; type Value=number; interface Port {rainfall(): Value;}",
        "import {Value} from './missing'; interface Port {rainfall(): Value;}",
        "interface Port {rainfall<T extends T>(city:T):T;}",
        "interface Port {rainfall<const T>(city:T):T;}",
    ] {
        assert_ne!(
            verify(&[proposal("/p/a.ts", source)]).coverage,
            Coverage::Verified,
            "{source}"
        );
    }
}
#[test]
fn php_nominal_covariance_and_contravariance_follow_known_inheritance_direction() {
    for source in [
        "<?php class Animal {} class Dog extends Animal {} abstract class Port { abstract function rainfall(Dog $animal): Animal; } class Weather extends Port { function rainfall(Animal $animal): Dog { return new Dog(); } }",
        "<?php namespace Zoo; class Animal {} class Dog extends Animal {} interface Port { function rainfall(Dog $animal): ?Animal; } class Weather implements Port { function rainfall(?Animal $animal): Dog { return new Dog(); } }",
    ] {
        let files = [proposal("/p/a.php", source)];
        assert_eq!(expect(&files, Coverage::Verified).checked_override_pairs, 1);
        let r = codisseu_syntax::validate_rename(&files, "precipitation", &AtomicBool::new(false));
        assert_eq!(r.coverage, Coverage::Verified, "{r:#?}");
    }
    let files = [
        context(
            "/p/types.php",
            "<?php namespace Zoo; class Animal {} class Dog extends Animal {}",
        ),
        proposal(
            "/p/port.php",
            "<?php namespace Contract; use Zoo\\Dog; use Zoo\\Animal as ResultType; interface Port { function rainfall(Dog $animal): ResultType; }",
        ),
        proposal(
            "/p/weather.php",
            "<?php namespace App; use Contract\\Port; use Zoo\\Animal; use Zoo\\Dog as Puppy; class Weather implements Port { function rainfall(Animal $animal): Puppy { return new Puppy(); } }",
        ),
    ];
    assert_eq!(expect(&files, Coverage::Verified).checked_override_pairs, 1);
}
#[test]
fn php_nominal_variance_never_reverses_subtyping_or_guesses_missing_types() {
    for source in [
        "<?php class Animal {} class Dog extends Animal {} interface Port { function rainfall(Animal $animal): Animal; } class Weather implements Port { function rainfall(Dog $animal): Animal { return $animal; } }",
        "<?php class Animal {} class Dog extends Animal {} interface Port { function rainfall(Dog $animal): Dog; } class Weather implements Port { function rainfall(Dog $animal): Animal { return $animal; } }",
        "<?php interface Port { function rainfall(Absent $animal): Absent; } class Weather implements Port { function rainfall(Absent $animal): Absent { return $animal; } }",
        "<?php trait Value {} interface Port { function rainfall(Value $animal): Value; } class Weather implements Port { function rainfall(Value $animal): Value { return $animal; } }",
    ] {
        assert_ne!(
            verify(&[proposal("/p/a.php", source)]).coverage,
            Coverage::Verified,
            "{source}"
        );
    }
}
