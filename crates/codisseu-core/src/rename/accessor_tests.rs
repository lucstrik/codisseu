use super::*;
use std::sync::atomic::AtomicBool;
fn proposal(accessors: bool, collision: bool) -> (tempfile::TempDir, Work, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Weather.php");
    let mut text="<?php\nclass Weather { public int $weather=0; public function getWeather(): int { return $this->weather; } public function setWeather(int $value): void { $this->weather=$value; } }\nfunction sample(Weather $w): int { $w->setWeather(2); return $w->getWeather(); }\n".to_owned();
    if collision {
        text = text.replace(
            "public int $weather=0;",
            "public int $weather=0; public function getForecast(): int {return 3;}",
        );
    }
    fs::write(&path, &text).unwrap();
    let edits = text
        .match_indices("weather")
        .map(|(start, _)| {
            let prefix = &text[..start];
            let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32;
            let col = prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32;
            RenameTextEdit {
                start_line: line,
                start_character: col,
                end_line: line,
                end_character: col + 7,
                new_text: "forecast".into(),
            }
        })
        .collect();
    let work = Work::RenamePreview {
        token: 1,
        workspace: dir.path().into(),
        changes: vec![codisseu_language::RenameFileEdits {
            path: path.clone(),
            version: Some(1),
            edits,
        }],
        new_name: "forecast".into(),
        buffers: vec![(path.clone(), 1, Arc::from(text))],
        context_limitation: None,
        local_origin: None,
        search: rename::RenameSearch {
            options: RenameOptions {
                rename_accessors: accessors,
                ..RenameOptions::default()
            },
            origin: path.clone(),
            old: "weather".into(),
            text_snapshots: vec![],
            ..rename::RenameSearch::default()
        },
        cancel: Arc::new(AtomicBool::new(false)),
    };
    (dir, work, path)
}
#[test]
fn property_accessors_are_independently_validated_combined_and_revalidated_on_apply() {
    for enabled in [true, false] {
        let (dir, work, path) = proposal(enabled, false);
        let generation = AtomicU64::new(1);
        let token = Cancellation {
            encoding_profile: None,
            workspace: None,
            generation: &generation,
            expected: 1,
        };
        let Completed::Rename { prepared, .. } = execute(work, &token).unwrap() else {
            panic!()
        };
        assert_eq!(
            prepared.validation.coverage,
            Coverage::Verified,
            "{:?}",
            prepared.validation
        );
        assert!(prepared.files[0].after.contains("$forecast"));
        assert_eq!(prepared.files[0].after.contains("getForecast"), enabled);
        assert_eq!(prepared.files[0].after.contains("setForecast"), enabled);
        // Primary snapshots retain the property-only proof; method edits live in
        // the combined private proposal and are recomputed independently.
        assert!(prepared.snapshots[0].after.contains("getWeather"));
        let verify = Work::RenameVerify {
            token: 1,
            workspace: dir.path().into(),
            files: prepared.files,
            resources: prepared.resources,
            search: prepared.search,
            open: vec![path.clone()],
            snapshots: prepared.snapshots,
            corpus: prepared.corpus,
            new_name: "forecast".into(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        let Completed::Rename {
            prepared, apply, ..
        } = execute(verify, &token).unwrap()
        else {
            panic!()
        };
        assert!(apply);
        assert_eq!(prepared.files[0].after.contains("getForecast"), enabled);
        assert!(fs::read_to_string(path).unwrap().contains("$weather"));
    }
}
#[test]
fn accessor_collision_blocks_property_batch_and_disabling_option_is_effective() {
    let (_dir, work, path) = proposal(true, true);
    let generation = AtomicU64::new(1);
    let token = Cancellation {
        encoding_profile: None,
        workspace: None,
        generation: &generation,
        expected: 1,
    };
    let Completed::Rename { prepared, .. } = execute(work, &token).unwrap() else {
        panic!()
    };
    assert_eq!(prepared.validation.coverage, Coverage::Rejected);
    assert!(
        prepared
            .validation
            .conflicts
            .iter()
            .any(|c| c.message.contains("Accessor") && c.path == path)
    );
    assert!(!prepared.files.is_empty());
    assert!(fs::read_to_string(path).unwrap().contains("$weather"));
    let (_dir, work, _) = proposal(false, true);
    let Completed::Rename { prepared, .. } = execute(work, &token).unwrap() else {
        panic!()
    };
    assert_eq!(prepared.validation.coverage, Coverage::Verified);
    assert!(prepared.files[0].after.contains("function getWeather()"));
}
