use codisseu_core::{AppService, Command, ValidationScope};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let until = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.is_busy() {
            break;
        }
        assert!(
            Instant::now() < until,
            "{} {:?}",
            app.status,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
fn app_at(root: &Path, path: &str) -> AppService {
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.into())).unwrap();
    idle(&mut app);
    app.dispatch(Command::OpenFile(root.join(path))).unwrap();
    idle(&mut app);
    app
}
fn prepare(app: &mut AppService, new_name: &str) {
    let d = app.active_document().unwrap();
    app.dispatch(Command::PrepareFileRename {
        id: d.id,
        expected_version: d.version,
    })
    .unwrap();
    assert!(app.rename.is_file_rename());
    assert!(app.rename.is_prepared());
    app.dispatch(Command::PreviewRename {
        new_name: new_name.into(),
    })
    .unwrap();
    idle(app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
}
fn dirty(app: &mut AppService) -> u64 {
    let d = app.active_document().unwrap();
    let id = d.id;
    app.dispatch(Command::Edit {
        id,
        expected_version: d.version,
        text: format!("{}// unsaved 😀 é\n", d.text()),
    })
    .unwrap();
    id
}
#[test]
fn standalone_ts_module_preserves_dirty_source_closed_consumers_cancel_and_group_history() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = "export function readWeather() { return 1; }\n";
    let main = "import {readWeather} from './helpers'; readWeather(); const text='helpers';\n";
    let closed = "export {readWeather} from './helpers.js';\n";
    fs::write(root.join("helpers.ts"), source).unwrap();
    fs::write(root.join("main.ts"), main).unwrap();
    fs::write(root.join("closed.ts"), closed).unwrap();
    let mut app = app_at(root, "helpers.ts");
    let id = dirty(&mut app);
    let unsaved = app.active_document().unwrap().text().to_owned();
    prepare(&mut app, "weather-utils.ts");
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert_eq!(
        app.rename.validation.as_ref().unwrap().scope,
        ValidationScope::ResourceReferences
    );
    assert_eq!(app.rename.resource_moves().len(), 1);
    assert_eq!(app.rename.files.len(), 2);
    app.dispatch(Command::CancelRename).unwrap();
    assert_eq!(fs::read_to_string(root.join("main.ts")).unwrap(), main);
    assert_eq!(app.active_document().unwrap().text(), unsaved);
    prepare(&mut app, "weather-utils.ts");
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
    let doc = app.documents.iter().find(|d| d.id == id).unwrap();
    assert_eq!(doc.path, root.join("weather-utils.ts"));
    assert_eq!(doc.text(), unsaved);
    assert!(doc.is_dirty());
    assert_eq!(
        fs::read_to_string(root.join("weather-utils.ts")).unwrap(),
        source
    );
    assert_eq!(fs::read_to_string(root.join("closed.ts")).unwrap(), closed);
    assert!(
        app.documents
            .iter()
            .find(|d| d.path == root.join("closed.ts"))
            .unwrap()
            .text()
            .contains("'./weather-utils.js'")
    );
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert!(root.join("helpers.ts").exists());
    assert_eq!(
        app.documents.iter().find(|d| d.id == id).unwrap().text(),
        unsaved
    );
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    let ids = app.documents.iter().map(|d| d.id).collect::<Vec<_>>();
    for id in ids {
        app.dispatch(Command::Save { id }).unwrap();
        idle(&mut app);
    }
    assert_eq!(
        fs::read_to_string(root.join("weather-utils.ts")).unwrap(),
        unsaved
    );
    assert_eq!(
        fs::read_to_string(root.join("main.ts")).unwrap(),
        main.replace("from './helpers'", "from './weather-utils'")
    );
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert_eq!(
        fs::read_to_string(root.join("helpers.ts")).unwrap(),
        unsaved
    );
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    assert!(root.join("weather-utils.ts").exists());
    assert!(
        !app.documents
            .iter()
            .find(|d| d.id == id)
            .unwrap()
            .is_dirty()
    );
}
#[test]
fn resource_without_occurrences_has_path_only_history_without_consuming_prior_buffer_undo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let source = "<?php function readWeather() { return 1; }\n";
    fs::write(root.join("helpers.php"), source).unwrap();
    let mut app = app_at(root, "helpers.php");
    let id = dirty(&mut app);
    let unsaved = app.active_document().unwrap().text().to_owned();
    prepare(&mut app, "utilities.php");
    assert!(app.rename.files.is_empty());
    assert!(app.rename.can_apply());
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(root.join("utilities.php").exists());
    assert!(app.can_undo_document(id));
    app.dispatch(Command::Save { id }).unwrap();
    idle(&mut app);
    assert!(!app.active_document().unwrap().is_dirty());
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert!(root.join("helpers.php").exists());
    assert_eq!(app.active_document().unwrap().text(), unsaved);
    assert!(!app.active_document().unwrap().is_dirty());
    assert!(app.can_redo_document(id));
    assert!(!app.active_document().unwrap().can_redo());
    app.dispatch(Command::Redo { id }).unwrap();
    idle(&mut app);
    assert!(root.join("utilities.php").exists());
    assert_eq!(app.active_document().unwrap().text(), unsaved);
}
#[test]
fn php_include_updates_only_ast_paths_with_independent_function_spelling() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("helpers.php"),
        "<?php function helperGreeting() { return 7; }\n",
    )
    .unwrap();
    fs::write(
        root.join("main.php"),
        "<?php require __DIR__ . '/helpers.php'; helperGreeting(); $text='helpers.php';\n",
    )
    .unwrap();
    let mut app = app_at(root, "helpers.php");
    prepare(&mut app, "utilities.php");
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert_eq!(app.rename.files.len(), 1);
    assert_eq!(
        app.rename.files[0].after.as_ref(),
        "<?php require __DIR__ . '/utilities.php'; helperGreeting(); $text='utilities.php';\n"
    );
}
#[test]
fn destination_and_context_races_do_not_apply_even_with_forged_public_report() {
    for race in ["destination", "context", "new_context"] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let source = "export function weather() {}\n";
        let main = "import {weather} from './helpers'; weather();\n";
        fs::write(root.join("helpers.ts"), source).unwrap();
        fs::write(root.join("main.ts"), main).unwrap();
        let mut app = app_at(root, "helpers.ts");
        let id = dirty(&mut app);
        let before = app.active_document().unwrap().text().to_owned();
        prepare(&mut app, "utilities.ts");
        assert!(app.rename.can_apply());
        match race {
            "destination" => fs::write(root.join("utilities.ts"), "external").unwrap(),
            "context" => fs::write(root.join("main.ts"), "export const external=1;").unwrap(),
            _ => fs::write(root.join("new.ts"), "export const added=1;").unwrap(),
        }
        app.rename.validation.as_mut().unwrap().coverage = codisseu_core::Coverage::Verified;
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(!app.rename.can_apply());
        assert!(root.join("helpers.ts").exists());
        assert_eq!(
            app.documents.iter().find(|d| d.id == id).unwrap().text(),
            before
        );
        assert_eq!(
            app.documents.iter().find(|d| d.id == id).unwrap().path,
            root.join("helpers.ts")
        );
        assert_eq!(fs::read_to_string(root.join("helpers.ts")).unwrap(), source);
    }
}
#[test]
fn unsupported_modules_target_ambiguity_and_existing_destination_remain_blocked_previews() {
    for (main, extra) in [
        ("import {weather} from '@helpers';", None),
        ("import(path);", None),
        (
            "import {weather} from './helpers';",
            Some(("utilities.js", "export const other=1;")),
        ),
        (
            "import {weather} from './helpers';",
            Some(("utilities.ts", "export const other=1;")),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("helpers.ts"), "export function weather() {}\n").unwrap();
        fs::write(root.join("main.ts"), main).unwrap();
        if let Some((p, t)) = extra {
            fs::write(root.join(p), t).unwrap();
        }
        let mut app = app_at(root, "helpers.ts");
        prepare(&mut app, "utilities.ts");
        assert!(!app.rename.can_apply(), "{main} {extra:?}");
        assert!(app.rename.semantic_blocked());
        assert!(app.rename.is_file_rename());
        assert!(root.join("helpers.ts").exists());
    }
}
#[cfg(unix)]
#[test]
fn symlink_destination_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("helpers.php"), "<?php function hello() {}\n").unwrap();
    fs::write(root.join("outside.txt"), "external").unwrap();
    std::os::unix::fs::symlink(root.join("outside.txt"), root.join("utilities.php")).unwrap();
    let mut app = app_at(root, "helpers.php");
    prepare(&mut app, "utilities.php");
    assert!(!app.rename.can_apply());
    assert_eq!(
        fs::read_to_string(root.join("outside.txt")).unwrap(),
        "external"
    );
}

#[test]
fn file_search_options_use_full_basename_and_refs_off_is_explicit_resource_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("helpers.ts"),
        "export function weather() {} // helpers.ts helpers helpers.php\n",
    )
    .unwrap();
    fs::write(root.join("main.ts"),"import {weather} from './helpers'; // helpers.ts helpers\nconst text='helpers.ts'; weather();\n").unwrap();
    fs::write(root.join("notes.txt"), "helpers.ts helpers helpers.php\n").unwrap();
    let mut app = app_at(root, "helpers.ts");
    prepare(&mut app, "utilities.ts");
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    let main = app
        .rename
        .files
        .iter()
        .find(|f| f.path.ends_with("main.ts"))
        .unwrap();
    assert!(main.after.contains("from './utilities'"));
    assert!(main.after.contains("// utilities.ts helpers"));
    assert_eq!(
        app.rename
            .files
            .iter()
            .find(|f| f.path.ends_with("notes.txt"))
            .unwrap()
            .after
            .as_ref(),
        "utilities.ts helpers helpers.php\n"
    );
    let mut options = app.rename.options();
    options.search_file_references = false;
    options.search_comments_strings = false;
    options.search_text = false;
    app.dispatch(Command::SetRenameOptions { options }).unwrap();
    app.dispatch(Command::PreviewRename {
        new_name: "utilities.ts".into(),
    })
    .unwrap();
    idle(&mut app);
    assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
    assert_eq!(
        app.rename.validation.as_ref().unwrap().scope,
        ValidationScope::ResourceOperationOnly
    );
    assert!(app.rename.files.is_empty());
    assert!(
        app.rename
            .validation
            .as_ref()
            .unwrap()
            .note
            .contains("NÃO serão atualizados")
    );
    let id = app.active_document().unwrap().id;
    app.dispatch(Command::ApplyRename).unwrap();
    idle(&mut app);
    assert!(
        fs::read_to_string(root.join("main.ts"))
            .unwrap()
            .contains("from './helpers'")
    );
    assert!(root.join("utilities.ts").exists());
    app.dispatch(Command::Undo { id }).unwrap();
    idle(&mut app);
    assert!(root.join("helpers.ts").exists());
}
#[test]
fn php_file_search_never_includes_plain_text_and_current_file_scope_limits_optional_edits_only() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("helpers.php"),
        "<?php function weather() {} // helpers.php helpers\n",
    )
    .unwrap();
    fs::write(
        root.join("main.php"),
        "<?php require __DIR__ . '/helpers.php'; // helpers.php\n",
    )
    .unwrap();
    fs::write(root.join("notes.txt"), "helpers.php\n").unwrap();
    let mut app = app_at(root, "helpers.php");
    prepare(&mut app, "utilities.php");
    assert!(!app.rename.options().search_text);
    assert!(
        !app.rename
            .files
            .iter()
            .any(|f| f.path.ends_with("notes.txt"))
    );
    let mut options = app.rename.options();
    options.scope = codisseu_core::RenameSearchScope::CurrentFile;
    app.dispatch(Command::SetRenameOptions { options }).unwrap();
    app.dispatch(Command::PreviewRename {
        new_name: "utilities.php".into(),
    })
    .unwrap();
    idle(&mut app);
    assert!(app.rename.can_apply());
    let main = app
        .rename
        .files
        .iter()
        .find(|f| f.path.ends_with("main.php"))
        .unwrap();
    assert!(main.after.contains("'/utilities.php'"));
    assert!(main.after.contains("// helpers.php"));
    assert!(
        app.rename
            .files
            .iter()
            .find(|f| f.path.ends_with("helpers.php"))
            .unwrap()
            .after
            .contains("// utilities.php helpers")
    );
}
#[test]
fn file_rename_blocks_nominal_php_autoload_and_explicit_config_file_references() {
    for (source, text, config, configuration, target) in [
        (
            "Weather.php",
            "<?php class Weather {}",
            "composer.json",
            "{}",
            "Climate.php",
        ),
        (
            "helpers.php",
            "<?php function weather() {}",
            "composer.json",
            r#"{"autoload":{"files":["helpers.php"]}}"#,
            "utilities.php",
        ),
        (
            "helpers.ts",
            "export function weather() {}",
            "tsconfig.json",
            r#"{"files":["helpers.ts"]}"#,
            "utilities.ts",
        ),
        (
            "helpers.ts",
            "export function weather() {}",
            "tsconfig.json",
            r#"{"include":["helpers.ts"]}"#,
            "utilities.ts",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(source), text).unwrap();
        fs::write(root.join(config), configuration).unwrap();
        let mut app = app_at(root, source);
        prepare(&mut app, target);
        assert!(!app.rename.can_apply(), "{config} {configuration}");
        assert!(app.rename.semantic_blocked());
        assert_eq!(fs::read_to_string(root.join(source)).unwrap(), text);
    }
}
#[test]
fn configuration_creation_or_optional_text_race_after_preview_blocks_the_move() {
    for text_race in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("helpers.ts"), "export function weather() {}\n").unwrap();
        fs::write(root.join("notes.txt"), "helpers.ts\n").unwrap();
        let mut app = app_at(root, "helpers.ts");
        prepare(&mut app, "utilities.ts");
        assert!(app.rename.can_apply(), "{:?}", app.rename.validation);
        if text_race {
            fs::write(root.join("notes.txt"), "external content").unwrap();
        } else {
            fs::write(root.join("tsconfig.json"), r#"{"files":["helpers.ts"]}"#).unwrap();
        }
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(root.join("helpers.ts").exists());
        assert!(!root.join("utilities.ts").exists());
        assert!(!app.rename.can_apply());
    }
}
