use codisseu_syntax::{
    RenameOptions, RenameSearchScope, RenameSnapshot, ValidationLimits, plan_noncode_rename,
};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
fn snap(path: &str, text: &str) -> RenameSnapshot {
    RenameSnapshot {
        path: path.into(),
        before: Arc::from(text),
        after: Arc::from(text),
        edits: vec![],
    }
}
fn options() -> RenameOptions {
    RenameOptions {
        search_comments_strings: true,
        search_text: true,
        scope: RenameSearchScope::Project,
        ..Default::default()
    }
}
fn planned(
    files: &[RenameSnapshot],
    opts: RenameOptions,
) -> Vec<(std::path::PathBuf, codisseu_syntax::RenameEdit)> {
    plan_noncode_rename(
        files,
        Path::new("/w/main.ts"),
        "Weather",
        "Climate",
        opts,
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap()
}
#[test]
fn explicit_optional_occurrences_preserve_code_boundaries_and_escaped_strings() {
    let files = [
        snap(
            "/w/main.ts",
            "class Weather {} // Weather WeatherMore _Weather météoWeather\nconst a='Weather'; const b='Wea\\u0074her'; const c=`${Weather}`;",
        ),
        snap(
            "/w/other.php",
            "<?php class Weather {} // Weather\n$x='Weather'; $y=\"$Weather\";",
        ),
        snap("/w/readme.md", "Weather Weatherish 😀 Weather"),
        snap("/w/skip.json", "{\"Weather\":1}"),
    ];
    let edits = planned(&files, options());
    assert_eq!(edits.iter().filter(|e| e.0 == files[0].path).count(), 2);
    assert_eq!(edits.iter().filter(|e| e.0 == files[1].path).count(), 2);
    assert_eq!(edits.iter().filter(|e| e.0 == files[2].path).count(), 2);
    assert_eq!(edits.len(), 6);
    assert!(planned(&files, RenameOptions::default()).is_empty());
}
#[test]
fn scope_and_categories_are_real_filters() {
    let files = [
        snap("/w/main.ts", "// Weather\nconst a='Weather';"),
        snap("/w/other.ts", "// Weather"),
        snap("/w/readme.md", "Weather"),
    ];
    assert_eq!(
        planned(
            &files,
            RenameOptions {
                scope: RenameSearchScope::CurrentFile,
                ..options()
            }
        )
        .len(),
        2
    );
    assert_eq!(
        planned(
            &files,
            RenameOptions {
                search_comments_strings: false,
                ..options()
            }
        )
        .len(),
        1
    );
    assert_eq!(
        planned(
            &files,
            RenameOptions {
                search_text: false,
                ..options()
            }
        )
        .len(),
        3
    );
}
#[test]
fn utf16_ranges_and_overlap_with_semantic_edits_are_exact() {
    let mut file = snap("/w/main.ts", "/* 😀 Weather */ const a='Weather';");
    let first = planned(&[file.clone()], options());
    assert_eq!(first[0].1.range.start_character, 6);
    file.edits.push(first[0].1.clone());
    let result = planned(&[file], options());
    assert_eq!(result.len(), 1);
    assert!(result[0].1.range.start_character > 6);
}
#[test]
fn malformed_syntax_cancellation_and_limits_do_not_return_partial_proposals() {
    let files = [snap("/w/main.ts", "// Weather\nclass {")];
    assert!(
        plan_noncode_rename(
            &files,
            Path::new("/w/main.ts"),
            "Weather",
            "Climate",
            options(),
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    let files = [snap("/w/main.ts", "// Weather")];
    assert!(
        plan_noncode_rename(
            &files,
            Path::new("/w/main.ts"),
            "Weather",
            "Climate",
            options(),
            &AtomicBool::new(true),
            ValidationLimits::default()
        )
        .is_err()
    );
    assert!(
        plan_noncode_rename(
            &files,
            Path::new("/w/main.ts"),
            "Weather",
            "Climate",
            options(),
            &AtomicBool::new(false),
            ValidationLimits {
                max_nodes: 1,
                ..ValidationLimits::default()
            }
        )
        .is_err()
    );
}

#[test]
fn textual_options_never_rewrite_executable_module_paths() {
    let files = [
        snap(
            "/w/main.ts",
            "import {Weather} from './Weather'; const x=require('./Weather'); const y=import('./Weather'); const z='Weather';",
        ),
        snap(
            "/w/main.php",
            "<?php require __DIR__ . '/Weather.php'; include 'Weather.php'; $text='Weather';",
        ),
    ];
    let edits = planned(&files, options());
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|(_, e)| e.new_text == "Climate"));
}

#[test]
fn file_basename_search_is_separate_from_strict_symbol_identifiers() {
    let files = [
        snap(
            "/w/main.ts",
            "import {weather} from './helpers'; const text='helpers.ts'; // helpers.ts helpers\n",
        ),
        snap("/w/helper.php", "<?php // helpers.ts helpers\n"),
        snap("/w/notes.txt", "helpers.ts helpers helpers.php\n"),
    ];
    let arguments = (
        &files[..],
        Path::new("/w/main.ts"),
        "helpers.ts",
        "weather-utils.ts",
    );
    assert!(
        plan_noncode_rename(
            arguments.0,
            arguments.1,
            arguments.2,
            arguments.3,
            options(),
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    let edits = codisseu_syntax::plan_file_noncode_rename(
        arguments.0,
        arguments.1,
        arguments.2,
        arguments.3,
        options(),
        &AtomicBool::new(false),
        ValidationLimits::default(),
    )
    .unwrap();
    assert_eq!(edits.len(), 4);
    assert!(edits.iter().all(|(_, e)| e.new_text == "weather-utils.ts"));
    assert!(
        codisseu_syntax::plan_file_noncode_rename(
            arguments.0,
            arguments.1,
            arguments.2,
            "escape'quote.ts",
            options(),
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
    assert!(
        codisseu_syntax::plan_file_noncode_rename(
            arguments.0,
            arguments.1,
            arguments.2,
            "weather-utils.php",
            options(),
            &AtomicBool::new(false),
            ValidationLimits::default()
        )
        .is_err()
    );
}
