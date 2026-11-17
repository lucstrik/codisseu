use codisseu_syntax::{
    RenameEdit, RenameSnapshot, TextRange, ValidationLimits, plan_class_file_rename,
};
use std::sync::{Arc, atomic::AtomicBool};
fn snapshot(path: &str, before: &str, change: bool) -> RenameSnapshot {
    let mut after = before.to_owned();
    let edits = if change {
        let start = before.find("Weather").unwrap();
        after.replace_range(start..start + 7, "Climate");
        let prefix = &before[..start];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
        let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
        vec![RenameEdit {
            range: TextRange {
                start_line: line,
                start_character: col,
                end_line: line,
                end_character: col + 7,
            },
            new_text: "Climate".into(),
        }]
    } else {
        vec![]
    };
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(before),
        after: Arc::from(after),
        edits,
    }
}
fn plan(
    files: &[RenameSnapshot],
) -> Result<Option<codisseu_syntax::RenameResourceProposal>, String> {
    plan_class_file_rename(
        files,
        "Climate",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
}
#[test]
fn typescript_file_rename_plans_only_ast_module_strings() {
    let files = [
        snapshot("/w/src/Weather.ts", "export class Weather {}", true),
        snapshot(
            "/w/main.ts",
            "import { Weather } from './src/Weather'; const text='./src/Weather'; // ./src/Weather\nexport { Weather } from './src/Weather.js';",
            false,
        ),
        snapshot("/w/other.ts", "import {Other} from './Other';", false),
    ];
    let p = plan(&files).unwrap().unwrap();
    assert_eq!(p.source, std::path::Path::new("/w/src/Weather.ts"));
    assert_eq!(p.target, std::path::Path::new("/w/src/Climate.ts"));
    assert_eq!(p.edits.len(), 2);
    assert_eq!(p.edits[0].1.new_text, "./src/Climate");
    assert_eq!(p.edits[1].1.new_text, "./src/Climate.js");
}
#[test]
fn php_file_rename_plans_dir_literal_relative_and_absolute_requires() {
    let files = [
        snapshot(
            "/w/src/Weather.php",
            "<?php namespace Fixture; class Weather {}",
            true,
        ),
        snapshot(
            "/w/main.php",
            "<?php require __DIR__ . '/src/Weather.php'; include './src/Weather.php'; require_once '/w/src/Weather.php'; $text='src/Weather.php';",
            false,
        ),
    ];
    let p = plan(&files).unwrap().unwrap();
    assert_eq!(p.target, std::path::Path::new("/w/src/Climate.php"));
    assert_eq!(
        p.edits
            .iter()
            .map(|e| e.1.new_text.as_str())
            .collect::<Vec<_>>(),
        [
            "/src/Climate.php",
            "./src/Climate.php",
            "/w/src/Climate.php"
        ]
    );
}
#[test]
fn resources_preserve_unicode_utf16_offsets() {
    let files = [
        snapshot("/w/Weather.ts", "/* 😀 */ export class Weather {}", true),
        snapshot(
            "/w/main.ts",
            "/* 😀 é */ import { Weather } from './Weather';",
            false,
        ),
    ];
    let p = plan(&files).unwrap().unwrap();
    let expected = "/* 😀 é */ import { Weather } from '"
        .encode_utf16()
        .count() as u32;
    assert_eq!(p.edits[0].1.range.start_character, expected);
}
#[test]
fn nonmatching_filename_or_function_has_no_file_operation() {
    for (path, source) in [
        ("/w/container.ts", "export class Weather {}"),
        ("/w/Weather.ts", "export function Weather() {}"),
    ] {
        assert!(plan(&[snapshot(path, source, true)]).unwrap().is_none());
    }
}
#[test]
fn configured_alias_with_different_basename_cannot_survive_unproved_file_move() {
    // The semantic compiler can resolve this exact paths mapping. Resource
    // planning does not consume that configuration and must not guess that a
    // differently spelled module cannot refer to the file being moved.
    let config = snapshot(
        "/w/tsconfig.json",
        r#"{"compilerOptions":{"paths":{"model":["./Weather.ts"]}}}"#,
        false,
    );
    for source in [
        "import { Weather } from 'model';",
        "export { Weather } from 'model';",
        "const module = import('model');",
        "const module = require('model');",
    ] {
        let files = [
            snapshot("/w/Weather.ts", "export class Weather {}", true),
            snapshot("/w/main.ts", source, false),
            config.clone(),
        ];
        let error = plan(&files).unwrap_err();
        assert!(error.contains("não relativo"), "{source}: {error}");
        // The guard belongs to file operations: a semantic rename without a
        // matching class filename must still return no resource proposal.
        let mut symbol_only = files;
        symbol_only[0].path = "/w/container.ts".into();
        assert!(plan(&symbol_only).unwrap().is_none(), "{source}");
    }
}
#[test]
fn dynamic_and_escaped_module_paths_block_resource_plan() {
    for source in [
        "import(path);",
        "require('./Wea\\u0074her');",
        "import { Weather } from '@/Weather';",
    ] {
        assert!(
            plan(&[
                snapshot("/w/Weather.ts", "export class Weather {}", true),
                snapshot("/w/main.ts", source, false)
            ])
            .is_err(),
            "{source}"
        );
    }
    for source in [
        "<?php require $path;",
        "<?php require 'Weather.php';",
        "<?php require dirname(__DIR__) . '/Weather.php';",
    ] {
        assert!(
            plan(&[
                snapshot("/w/Weather.php", "<?php class Weather {}", true),
                snapshot("/w/main.php", source, false)
            ])
            .is_err(),
            "{source}"
        );
    }
}
#[test]
fn dynamic_import_and_require_with_exact_literal_are_planned() {
    let p = plan(&[
        snapshot("/w/Weather.ts", "export class Weather {}", true),
        snapshot(
            "/w/main.ts",
            "import('./Weather'); require('./Weather.js');",
            false,
        ),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(p.edits.len(), 2);
}
#[test]
fn destination_collision_and_ambiguous_modules_block_plan() {
    let files = [
        snapshot("/w/Weather.ts", "export class Weather {}", true),
        snapshot("/w/Climate.ts", "export class Existing {}", false),
    ];
    assert!(plan(&files).is_err());
    let files = [
        snapshot("/w/Weather.ts", "export class Weather {}", true),
        snapshot("/w/Weather/index.ts", "export class Other {}", false),
        snapshot("/w/main.ts", "import {Weather} from './Weather';", false),
    ];
    assert!(plan(&files).is_err());
}
#[test]
fn resource_planner_validates_snapshots_and_budget() {
    let file = snapshot("/w/Weather.ts", "export class Weather {}", true);
    assert!(
        plan_class_file_rename(
            std::slice::from_ref(&file),
            "Climate",
            &AtomicBool::new(true),
            ValidationLimits::default()
        )
        .is_err()
    );
    let mut invalid = file;
    invalid.after = Arc::from("export class Different {}");
    assert!(plan(&[invalid]).is_err());
}
#[test]
fn php_double_quoted_plain_includes_are_updated_without_interpolation() {
    let p = plan(&[
        snapshot("/w/Weather.php", "<?php class Weather {}", true),
        snapshot(
            "/w/main.php",
            "<?php require __DIR__ . \"/Weather.php\";",
            false,
        ),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(p.edits[0].1.new_text, "/Climate.php");
    assert!(
        plan(&[
            snapshot("/w/Weather.php", "<?php class Weather {}", true),
            snapshot(
                "/w/main.php",
                "<?php require \"$folder/Weather.php\";",
                false
            )
        ])
        .is_err()
    );
}

#[test]
fn class_file_moves_update_consumers_across_javascript_and_jsx_extensions() {
    for extension in ["ts", "tsx", "js", "jsx"] {
        let source = format!("/w/Weather.{extension}");
        let p = plan(&[
            snapshot(&source, "export class Weather {}", true),
            snapshot(
                "/w/consumer.js",
                "import {Weather} from './Weather';",
                false,
            ),
            snapshot(
                "/w/view.tsx",
                "import {Weather} from './Weather'; const view=<div/>;",
                false,
            ),
            snapshot(
                "/w/view.jsx",
                "export {Weather} from './Weather.js'; const view=<div/>;",
                false,
            ),
        ])
        .unwrap()
        .unwrap();
        assert_eq!(
            p.target,
            std::path::PathBuf::from(format!("/w/Climate.{extension}"))
        );
        assert_eq!(p.edits.len(), 3, "{extension}");
    }
    assert!(
        plan(&[
            snapshot("/w/Weather.tsx", "export class Weather {}", true),
            snapshot("/w/Weather.js", "export class Other {}", false),
            snapshot("/w/main.jsx", "import {Weather} from './Weather';", false),
        ])
        .is_err()
    );
}

#[test]
fn shorthand_prefix_suffix_proposals_do_not_require_a_resource_operation() {
    let before = "const Weather=1; const object={Weather};";
    let mut s = snapshot("/w/main.js", before, true);
    let start = before.rfind("Weather").unwrap() as u32;
    s.edits.push(RenameEdit {
        range: TextRange {
            start_line: 0,
            start_character: start,
            end_line: 0,
            end_character: start + 7,
        },
        new_text: "Weather: Climate".into(),
    });
    s.after = Arc::from("const Climate=1; const object={Weather: Climate};");
    assert!(plan(std::slice::from_ref(&s)).unwrap().is_none());
    let mut invalid = s.clone();
    invalid.after = Arc::from("unrelated text");
    assert!(plan(&[invalid]).is_err());
    let mut overlap = s.clone();
    overlap.edits.push(overlap.edits[1].clone());
    assert!(plan(&[overlap]).is_err());
    let mut invalid_range = s;
    invalid_range.edits[1].range.end_character = u32::MAX;
    assert!(plan(&[invalid_range]).is_err());
}

#[test]
fn independent_custom_filename_changes_only_module_literals() {
    let files = [
        snapshot("/w/Weather.php", "<?php class Weather {}", true),
        snapshot(
            "/w/main.php",
            "<?php require __DIR__ . '/Weather.php'; $text='Weather.php';",
            false,
        ),
    ];
    let p = codisseu_syntax::plan_named_file_rename(
        &files,
        std::path::Path::new("/w/Weather.php"),
        std::path::Path::new("/w/ChosenRoot.php"),
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.target, std::path::Path::new("/w/ChosenRoot.php"));
    assert_eq!(p.edits.len(), 1);
    assert_eq!(p.edits[0].1.new_text, "/ChosenRoot.php");
    for invalid in ["/other/Chosen.php", "/w/Chosen.ts", "/w/not\"valid.php"] {
        assert!(
            codisseu_syntax::plan_named_file_rename(
                &files,
                std::path::Path::new("/w/Weather.php"),
                std::path::Path::new(invalid),
                &AtomicBool::new(false),
                ValidationLimits::default(),
            )
            .is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn chooser_candidate_is_unapproved_and_occupied_target_still_fails_planning() {
    let files = [
        snapshot("/w/Weather.php", "<?php class Weather {}", true),
        snapshot("/w/Climate.php", "<?php class Existing {}", false),
    ];
    let candidate = codisseu_syntax::class_file_rename_candidate(
        &files,
        "Climate",
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert!(candidate.edits.is_empty());
    assert_eq!(candidate.target, std::path::Path::new("/w/Climate.php"));
    assert!(plan(&files).is_err());
    assert!(
        codisseu_syntax::plan_named_file_rename(
            &files,
            &candidate.source,
            &candidate.target,
            &AtomicBool::new(false),
            ValidationLimits::default(),
        )
        .is_err()
    );
}

#[test]
fn standalone_module_filenames_preserve_hyphens_compound_extensions_and_resolution() {
    let files = [
        snapshot("/w/helpers.ts", "export function weather() {}", false),
        snapshot("/w/main.ts", "import {weather} from './helpers';", false),
    ];
    let p = codisseu_syntax::plan_named_file_rename(
        &files,
        std::path::Path::new("/w/helpers.ts"),
        std::path::Path::new("/w/weather-utils.ts"),
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.edits[0].1.new_text, "./weather-utils");
    let mut ambiguous = files.to_vec();
    ambiguous.push(snapshot(
        "/w/weather-utils.js",
        "export const other=1;",
        false,
    ));
    assert!(
        codisseu_syntax::plan_named_file_rename(
            &ambiguous,
            &p.source,
            &p.target,
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    let declarations = [
        snapshot(
            "/w/helpers.d.ts",
            "export declare function weather(): void;",
            false,
        ),
        snapshot("/w/main.ts", "import {weather} from './helpers';", false),
    ];
    let p = codisseu_syntax::plan_named_file_rename(
        &declarations,
        std::path::Path::new("/w/helpers.d.ts"),
        std::path::Path::new("/w/utilities.d.ts"),
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(p.edits[0].1.new_text, "./utilities");
    assert!(
        codisseu_syntax::plan_named_file_rename(
            &declarations,
            &p.source,
            std::path::Path::new("/w/utilities.ts"),
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
}
#[test]
fn standalone_php_nominal_filename_guard_and_escaped_names_fail_closed() {
    for source in [
        "<?php class Weather {}",
        "<?php interface Weather {}",
        "<?php trait Weather {}",
        "<?php enum Weather {}",
    ] {
        assert!(
            codisseu_syntax::validate_standalone_file_source(
                std::path::Path::new("/w/Weather.php"),
                source,
                &AtomicBool::new(false),
                ValidationLimits::default()
            )
            .is_err()
        );
    }
    assert!(
        codisseu_syntax::validate_standalone_file_source(
            std::path::Path::new("/w/helpers.php"),
            "<?php function weather() {}",
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_ok()
    );
    for name in [
        "../escape.ts",
        "bad'quote.ts",
        "bad\"quote.ts",
        "bad$var.php",
        "",
        "..",
        "file.ts ",
    ] {
        assert!(
            codisseu_syntax::validate_file_rename_name(name).is_err(),
            "{name}"
        );
    }
    for name in [
        "weather-utils.ts",
        "weather.utils.ts",
        "weather utils.ts",
        "clima_á.ts",
    ] {
        assert!(
            codisseu_syntax::validate_file_rename_name(name).is_ok(),
            "{name}"
        );
    }
}
