use codisseu_syntax::{
    RenameEnclosingKind as Context, RenameOccurrenceKind as Kind, TextRange, ValidationLimits,
    rename_occurrence_presentation,
};
use std::{path::Path, sync::atomic::AtomicBool};
fn ranges(source: &str, name: &str) -> Vec<TextRange> {
    source
        .match_indices(name)
        .map(|(byte, _)| {
            let prefix = &source[..byte];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
            let column = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
            TextRange {
                start_line: line,
                start_character: column,
                end_line: line,
                end_character: column + name.encode_utf16().count() as u32,
            }
        })
        .collect()
}
fn labels(
    path: &str,
    source: &str,
    name: &str,
) -> Vec<codisseu_syntax::RenameOccurrencePresentation> {
    rename_occurrence_presentation(
        Path::new(path),
        source,
        &ranges(source, name),
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
}
#[test]
fn php_function_declaration_calls_comments_strings_and_contexts_stay_distinct() {
    let source = "<?php\nfunction rainfall() {}\nclass Weather { function report() { /* 😀 rainfall */ rainfall(); $text='rainfall'; } }\nrainfall();";
    let result = labels("/w/main.php", source, "rainfall");
    assert_eq!(
        result.iter().map(|r| r.kind).collect::<Vec<_>>(),
        [
            Kind::Declaration,
            Kind::Comment,
            Kind::SemanticReference,
            Kind::String,
            Kind::SemanticReference
        ]
    );
    assert_eq!(result[0].enclosing[0].kind, Context::Function);
    assert_eq!(result[0].enclosing[0].name, "rainfall");
    assert_eq!(
        result[2]
            .enclosing
            .iter()
            .map(|c| (c.kind, c.name.as_str()))
            .collect::<Vec<_>>(),
        [(Context::Class, "Weather"), (Context::Method, "report")]
    );
    assert!(result[4].enclosing.is_empty());
    assert_eq!(result[2].enclosing[1].range.start_line, 2);
}
#[test]
fn typescript_class_property_parameter_and_method_declarations_are_labels_only() {
    let source = "export class Weather { rainfall=0; report(rainfall:number) { return rainfall+this.rainfall; } }";
    assert_eq!(
        labels("/w/main.ts", source, "rainfall")
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [
            Kind::Declaration,
            Kind::Declaration,
            Kind::SemanticReference,
            Kind::SemanticReference
        ]
    );
    assert_eq!(
        labels("/w/main.ts", source, "Weather")[0].kind,
        Kind::Declaration
    );
    assert_eq!(
        labels("/w/main.ts", source, "report")[0].kind,
        Kind::Declaration
    );
    let php = "<?php class Weather { public int $rainfall=0; function run(int $rainfall) { return $rainfall+$this->rainfall; } }";
    assert_eq!(
        labels("/w/main.php", php, "rainfall")
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [
            Kind::Declaration,
            Kind::Declaration,
            Kind::SemanticReference,
            Kind::SemanticReference
        ]
    );
}
#[test]
fn module_literals_and_interpolated_symbols_remain_semantic_references() {
    for (path, source, expected) in [
        (
            "/w/main.ts",
            "import { Weather } from './Weather'; const text='Weather';",
            vec![
                Kind::SemanticReference,
                Kind::SemanticReference,
                Kind::String,
            ],
        ),
        (
            "/w/main.php",
            "<?php require __DIR__ . '/Weather.php'; $text='Weather';",
            vec![Kind::SemanticReference, Kind::String],
        ),
        (
            "/w/main.ts",
            "const rainfall=1; const message=`${rainfall}`;",
            vec![Kind::Declaration, Kind::SemanticReference],
        ),
        (
            "/w/main.php",
            "<?php function run($rainfall) { return \"$rainfall\"; }",
            vec![Kind::Declaration, Kind::SemanticReference],
        ),
    ] {
        assert_eq!(
            labels(
                path,
                source,
                if source.contains("rainfall") {
                    "rainfall"
                } else {
                    "Weather"
                }
            )
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
            expected,
            "{source}"
        );
    }
}
#[test]
fn jsx_plain_text_unicode_invalid_ranges_and_parse_failure_keep_every_occurrence() {
    let source = "/* 😀 é */ function rainfall() {} const view=<div>{rainfall()}</div>;";
    assert_eq!(
        labels("/w/view.jsx", source, "rainfall")
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [Kind::Declaration, Kind::SemanticReference]
    );
    assert_eq!(
        labels("/w/readme.md", "😀 rainfall rainfall", "rainfall")
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [Kind::PlainText, Kind::PlainText]
    );
    for (path, text) in [
        ("/w/main.ts", "function rainfall( { rainfall"),
        ("/w/main.sql", "rainfall rainfall"),
    ] {
        let result = labels(path, text, "rainfall");
        assert_eq!(result.len(), 2);
        assert!(
            result
                .iter()
                .all(|r| r.kind == Kind::Unknown && r.unknown_reason.is_some())
        );
    }
    let invalid = TextRange {
        start_line: 0,
        start_character: 1,
        end_line: 0,
        end_character: 2,
    };
    let result = rename_occurrence_presentation(
        Path::new("/w/readme.txt"),
        "😀 rainfall",
        &[invalid],
        &AtomicBool::new(false),
        ValidationLimits::default(),
    );
    assert_eq!(result[0].kind, Kind::Unknown);
}
#[test]
fn cancellation_and_limits_return_aligned_visible_unknown_instead_of_dropping_edits() {
    let source = "function rainfall() {} rainfall();";
    let rs = ranges(source, "rainfall");
    for (cancel, limits) in [
        (true, ValidationLimits::default()),
        (
            false,
            ValidationLimits {
                max_nodes: 0,
                ..ValidationLimits::default()
            },
        ),
    ] {
        let result = rename_occurrence_presentation(
            Path::new("/w/main.ts"),
            source,
            &rs,
            &AtomicBool::new(cancel),
            limits,
        );
        assert_eq!(result.len(), rs.len());
        assert!(result.iter().all(|r| r.kind == Kind::Unknown));
    }
}
