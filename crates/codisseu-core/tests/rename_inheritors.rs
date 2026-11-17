use codisseu_core::{AppService, Command, Coverage, RenameInheritorKind, RenameOptions};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            app.status,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn fixture() -> (tempfile::TempDir, AppService) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (name, text) in [
        ("Weather.php", "<?php class Weather {}\n"),
        (
            "WetWeather.php",
            "<?php /* 😀 */ class WetWeather extends Weather {}\n",
        ),
        (
            "WeatherChild.php",
            "<?php class WeatherChild extends Weather {}\n",
        ),
        (
            "ChildWeatherChild.php",
            "<?php class ChildWeatherChild extends WeatherChild {}\n",
        ),
        (
            "main.php",
            "<?php require __DIR__ . '/Weather.php'; require __DIR__ . '/WetWeather.php'; require __DIR__ . '/WeatherChild.php'; new Weather(); new WetWeather(); new WeatherChild(); new ChildWeatherChild(); // Weather\n",
        ),
    ] {
        fs::write(root.join(name), text).unwrap();
    }
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(root.join("Weather.php")))
        .unwrap();
    idle(&mut app);
    let doc = app.active_document().unwrap();
    app.dispatch(Command::Edit {
        id: doc.id,
        expected_version: doc.version,
        text: format!("{}// dirty 😀\n", doc.text()),
    })
    .unwrap();
    (dir, app)
}
fn preview(app: &mut AppService, options: RenameOptions) {
    let doc = app.active_document().unwrap();
    let offset = doc.text().find("Weather").unwrap();
    app.dispatch(Command::PrepareRename {
        id: doc.id,
        expected_version: doc.version,
        character_index: doc.text()[..offset].chars().count(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.is_prepared(), "{:?}", app.rename.error);
    app.dispatch(Command::SetRenameOptions { options }).unwrap();
    app.dispatch(Command::PreviewRename {
        new_name: "Climate".into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
}
fn confirm(app: &mut AppService) {
    app.dispatch(Command::ConfirmRenameInheritors {
        token: app.rename.context_token().unwrap(),
    })
    .unwrap();
    idle(app);
}
fn update(app: &mut AppService, index: usize, selected: bool, name: &str) {
    app.dispatch(Command::UpdateRenameInheritor {
        token: app.rename.context_token().unwrap(),
        index,
        selected,
        new_name: name.into(),
    })
    .unwrap();
}
#[test]
fn default_choices_apply_all_direct_classes_files_and_grouped_undo_redo_without_save() {
    let (dir, mut app) = fixture();
    let original = fs::read(dir.path().join("Weather.php")).unwrap();
    let id = app.active_document().unwrap().id;
    let dirty = app.active_document().unwrap().text().to_string();
    preview(&mut app, RenameOptions::default());
    assert!(app.rename.choosing_inheritors());
    assert!(!app.rename.can_apply());
    assert!(app.dispatch(Command::ApplyRename).is_err());
    assert!(app.rename.choosing_inheritors());
    assert_eq!(app.rename.inheritor_choices().len(), 5);
    assert!(app.rename.inheritor_choices().iter().all(|r| r.selected));
    confirm(&mut app);
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    assert_eq!(app.rename.resource_moves().len(), 3);
    assert!(
        app.rename
            .files
            .iter()
            .all(|f| f.edits.len() == f.presentation.len())
    );
    let root_preview = app
        .rename
        .files
        .iter()
        .find(|f| f.path.ends_with("Weather.php"))
        .unwrap();
    assert!(
        root_preview
            .presentation
            .iter()
            .any(|p| p.kind == codisseu_core::RenameOccurrenceKind::Declaration)
    );

    let main = app
        .rename
        .files
        .iter()
        .find(|f| f.path.ends_with("main.php"))
        .unwrap();
    assert!(main.after.contains("'/Climate.php'"));
    assert!(main.after.contains("new WetClimate()"));
    assert!(main.after.contains("new ClimateChild()"));
    assert!(main.after.contains("new ChildWeatherChild()"));
    assert!(main.after.contains("// Weather"));
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(fs::read(dir.path().join("Climate.php")).unwrap(), original);
    assert!(!dir.path().join("Weather.php").exists());
    assert!(app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert!(dir.path().join("Weather.php").exists());
    assert_eq!(app.active_document().unwrap().text(), dirty);
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    assert!(dir.path().join("Climate.php").exists());
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Climate")
    );
}
#[test]
fn editable_class_selection_is_independent_from_file_rows_and_invalid_drafts_stay_open() {
    let (dir, mut app) = fixture();
    preview(&mut app, RenameOptions::default());
    let rows = app.rename.inheritor_choices().to_vec();
    for (i, row) in rows.iter().enumerate() {
        update(&mut app, i, false, &row.new_name);
    }
    let index = rows
        .iter()
        .position(|r| r.kind == RenameInheritorKind::Class && r.old_name == "WeatherChild")
        .unwrap();
    update(&mut app, index, true, "");
    assert!(!app.rename.can_confirm_inheritors());
    assert!(app.rename.choosing_inheritors());
    update(&mut app, index, true, "ChosenChild");
    confirm(&mut app);
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert!(app.rename.resource_moves().is_empty());
    assert!(
        app.rename
            .files
            .iter()
            .find(|f| f.path.ends_with("WeatherChild.php"))
            .unwrap()
            .after
            .contains("class ChosenChild extends Climate")
    );
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(dir.path().join("Weather.php").exists());
    assert!(dir.path().join("WeatherChild.php").exists());
    assert!(!dir.path().join("Climate.php").exists());
}
#[test]
fn two_selected_classes_cannot_acquire_same_identity_and_choices_can_be_corrected() {
    let (_dir, mut app) = fixture();
    preview(&mut app, RenameOptions::default());
    let rows = app.rename.inheritor_choices().to_vec();
    for (i, row) in rows.iter().enumerate() {
        update(
            &mut app,
            i,
            row.kind == RenameInheritorKind::Class,
            if row.kind == RenameInheritorKind::Class {
                "Twin"
            } else {
                &row.new_name
            },
        );
    }
    confirm(&mut app);
    assert!(!app.rename.can_apply());
    assert_eq!(
        app.rename.validation.as_ref().unwrap().coverage,
        Coverage::Rejected
    );
    assert!(app.rename.is_open());
    assert!(app.active_document().unwrap().is_dirty());
    assert!(
        app.rename
            .validation
            .as_ref()
            .unwrap()
            .conflicts
            .iter()
            .any(|c| c.message.contains("Herdeiro"))
    );
    app.dispatch(Command::EditRenameInheritors {
        token: app.rename.context_token().unwrap(),
    })
    .unwrap();
    let index = rows
        .iter()
        .position(|r| r.kind == RenameInheritorKind::Class)
        .unwrap();
    update(&mut app, index, true, "UniqueChild");
    confirm(&mut app);
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
}
#[test]
fn disabled_option_and_cancel_preserve_all_paths_and_inheritor_names() {
    let (dir, mut app) = fixture();
    preview(
        &mut app,
        RenameOptions {
            rename_inheritors: false,
            ..Default::default()
        },
    );
    assert!(!app.rename.choosing_inheritors());
    assert!(app.rename.can_apply());
    assert!(app.rename.resource_moves().is_empty());
    assert!(
        app.rename
            .files
            .iter()
            .find(|f| f.path.ends_with("WetWeather.php"))
            .unwrap()
            .after
            .contains("class WetWeather extends Climate")
    );
    app.dispatch(Command::CancelRename).unwrap();
    assert!(!app.rename.is_open());
    assert!(dir.path().join("Weather.php").exists());
    assert!(!dir.path().join("Climate.php").exists());
    assert!(
        app.active_document()
            .unwrap()
            .text()
            .contains("class Weather")
    );
}
#[test]
fn file_only_custom_names_and_two_destination_collisions_are_revalidated() {
    let (_dir, mut app) = fixture();
    preview(&mut app, RenameOptions::default());
    let rows = app.rename.inheritor_choices().to_vec();
    for (i, row) in rows.iter().enumerate() {
        update(
            &mut app,
            i,
            row.kind == RenameInheritorKind::File,
            "Shared.php",
        );
    }
    confirm(&mut app);
    assert!(!app.rename.can_apply());
    assert!(app.rename.is_open());
    app.dispatch(Command::EditRenameInheritors {
        token: app.rename.context_token().unwrap(),
    })
    .unwrap();
    for (i, row) in rows.iter().enumerate() {
        update(
            &mut app,
            i,
            row.kind == RenameInheritorKind::File,
            if row.old_name == "Weather.php" {
                "ChosenRoot.php"
            } else {
                &row.old_name
            },
        );
    }
    confirm(&mut app);
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert_eq!(app.rename.resource_moves().len(), 1);
    assert!(
        app.rename
            .files
            .iter()
            .find(|f| f.path.ends_with("main.php"))
            .unwrap()
            .after
            .contains("'/ChosenRoot.php'")
    );
}

#[test]
fn same_file_offsets_unicode_and_two_sequential_names_preserve_mixed_web_context() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let text = "<?php /* 😀 */ class Weather {} class WetWeather extends Weather {} class WeatherChild extends Weather {} new WetWeather(); new WeatherChild();\n";
    fs::write(root.join("classes.php"), text).unwrap();
    fs::write(
        root.join("view.jsx"),
        "export const View = () => <span>Weather</span>;\n",
    )
    .unwrap();
    fs::write(root.join("data.js"), "export const label = 'Weather';\n").unwrap();
    fs::write(
        root.join("other.php"),
        "<?php namespace Other; class Weather {} new Weather();\n",
    )
    .unwrap();
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(root.join("classes.php")))
        .unwrap();
    idle(&mut app);
    preview(&mut app, RenameOptions::default());
    assert_eq!(app.rename.inheritor_choices().len(), 2);
    confirm(&mut app);
    assert!(
        app.rename.can_apply(),
        "{:?} {:?}",
        app.rename.error,
        app.rename.validation
    );
    let after = &app.rename.files[0].after;
    assert!(after.contains("class Climate {} class WetClimate extends Climate {} class ClimateChild extends Climate {} new WetClimate(); new ClimateChild();"));
    assert_eq!(app.rename.files.len(), 1);
}
#[test]
fn no_children_still_offer_root_file_and_external_resource_race_blocks_whole_apply() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(root.join("Weather.php"), "<?php class Weather {}\n").unwrap();
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(root.join("Weather.php")))
        .unwrap();
    idle(&mut app);
    preview(&mut app, RenameOptions::default());
    assert_eq!(app.rename.inheritor_choices().len(), 1);
    assert_eq!(
        app.rename.inheritor_choices()[0].kind,
        RenameInheritorKind::File
    );
    confirm(&mut app);
    assert!(app.rename.can_apply());
    fs::write(root.join("Climate.php"), "external data").unwrap();
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert_eq!(
        fs::read_to_string(root.join("Climate.php")).unwrap(),
        "external data"
    );
    assert_eq!(
        app.active_document().unwrap().text(),
        "<?php class Weather {}\n"
    );
    assert!(root.join("Weather.php").exists());
}

#[test]
fn presentation_labels_never_change_private_proposal_equality() {
    let file = codisseu_core::RenamePreviewFile {
        encoding: codisseu_core::FileEncoding::Utf8,
        path: "/w/Weather.php".into(),
        before: "<?php class Weather {}".into(),
        after: "<?php class Climate {}".into(),
        edits: vec![codisseu_core::RenameTextEdit {
            start_line: 0,
            start_character: 12,
            end_line: 0,
            end_character: 19,
            new_text: "Climate".into(),
        }],
        presentation: vec![],
    };
    let mut labels = file.clone();
    labels
        .presentation
        .push(codisseu_core::RenameOccurrencePresentation {
            kind: codisseu_core::RenameOccurrenceKind::Unknown,
            enclosing: vec![],
            unknown_reason: Some("limited".into()),
        });
    assert_eq!(file, labels);
    labels.edits[0].new_text = "Forged".into();
    assert_ne!(file, labels);
}
