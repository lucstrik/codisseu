use codisseu_syntax::{RenameSymbolKind as K, classify_rename_symbol};
use std::{path::Path, sync::atomic::AtomicBool};
#[test]
fn dialog_descriptions_follow_ast_symbol_kind_at_utf16_caret() {
    for (path, source, needle, expected) in [
        (
            "a.php",
            "<?php /* 😀 */ class Weather { public int $rainfall=1; function measure(string $city): string {return $city;} }",
            "rainfall",
            K::Property,
        ),
        (
            "a.php",
            "<?php /* 😀 */ class Weather {}",
            "Weather",
            K::Class,
        ),
        (
            "a.php",
            "<?php function rainfall(): int {return 1;}",
            "rainfall",
            K::Function,
        ),
        (
            "a.php",
            "<?php class W { function rainfall(): int {return 1;} }",
            "rainfall",
            K::Method,
        ),
        (
            "a.php",
            "<?php function measure(string $city):string{return $city;}",
            "city",
            K::Parameter,
        ),
        (
            "a.ts",
            "/* 😀 */ class Weather { rainfall: number=1; }",
            "rainfall",
            K::Property,
        ),
        ("a.ts", "export class Weather {}", "Weather", K::Class),
        (
            "a.ts",
            "function rainfall(): number {return 1;}",
            "rainfall",
            K::Function,
        ),
        (
            "a.ts",
            "class W { rainfall(): number {return 1;} }",
            "rainfall",
            K::Method,
        ),
        (
            "a.ts",
            "function measure(city: string):string{return city;}",
            "city",
            K::Parameter,
        ),
        (
            "a.ts",
            "class W { constructor(public rainfall: number){} }",
            "rainfall",
            K::Property,
        ),
        ("a.ts", "const rainfall=1;", "rainfall", K::Variable),
    ] {
        let offset = source.find(needle).unwrap() + 2;
        let col = source[..offset].encode_utf16().count() as u32;
        assert_eq!(
            classify_rename_symbol(Path::new(path), source, 0, col, &AtomicBool::new(false))
                .unwrap(),
            expected,
            "{source}"
        );
    }
}
#[test]
fn description_cancellation_and_invalid_utf16_do_not_guess() {
    assert!(
        classify_rename_symbol(
            Path::new("a.ts"),
            "class W {}",
            0,
            6,
            &AtomicBool::new(true)
        )
        .is_err()
    );
    assert!(
        classify_rename_symbol(
            Path::new("a.ts"),
            "/*😀*/ class W {}",
            0,
            3,
            &AtomicBool::new(false)
        )
        .is_err()
    );
}

#[test]
fn javascript_and_jsx_use_their_source_grammar_for_symbol_descriptions() {
    for extension in ["js", "jsx", "tsx"] {
        for (source, expected) in [
            ("export class Weather {}", K::Class),
            ("export function Weather(){return 1;}", K::Function),
            ("class W { Weather(){return 1;} }", K::Method),
            ("class W { Weather=1; }", K::Property),
        ] {
            let column = source.find("Weather").unwrap() as u32 + 2;
            assert_eq!(
                classify_rename_symbol(
                    Path::new(&format!("source.{extension}")),
                    source,
                    0,
                    column,
                    &AtomicBool::new(false)
                )
                .unwrap(),
                expected
            );
        }
    }
    let source = "const view=<div/>; class W { Weather=1; }";
    for extension in ["jsx", "tsx"] {
        assert_eq!(
            classify_rename_symbol(
                Path::new(&format!("source.{extension}")),
                source,
                0,
                source.find("Weather").unwrap() as u32,
                &AtomicBool::new(false)
            )
            .unwrap(),
            K::Property
        );
    }
}
