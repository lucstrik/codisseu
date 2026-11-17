//! Explicit real Phpactor UI corpus; only disposable sources and provider processes.
use super::*;
use codisseu_core::{Language, ProviderConfig, ProviderState};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, path::Path, thread, time::Instant};
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &to.join(e.file_name()));
        } else {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}
fn wait(h: &mut Harness<CodisseuApp>, check: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        h.step();
        assert!(
            h.state().service.last_error.is_none(),
            "{:?}",
            h.state().service.last_error
        );
        if check(h.state()) {
            h.step();
            return;
        }
        assert!(Instant::now() < end, "{}", h.state().service.status);
        thread::sleep(Duration::from_millis(5));
    }
}
fn open(h: &mut Harness<CodisseuApp>, path: &Path) {
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.into()))
        .unwrap();
    wait(h, |a| !a.service.is_busy() && !a.php_navigation.pending());
}
fn capture(h: &mut Harness<CodisseuApp>, name: &str) {
    if let Some(out) = std::env::var_os("CODISSEU_PHP_EDITOR_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(PathBuf::from(out).join(format!("{name}.png")))
            .unwrap();
    }
}
#[test]
#[ignore = "Explicit real PHP gutter/navigation UI; CODISSEU_PROVIDER_CONFIG required"]
fn gutter_usages_transitive_base_implementations_and_idle_parse_budget() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/php-navigation"),
        &root,
    );
    fs::write(
        root.join("src/ParcelGateway.php"),
        "<?php\nnamespace CodisseuNavigation;\ninterface ParcelGateway extends BaseGateway {}\n",
    )
    .unwrap();
    fs::write(root.join("src/BaseGateway.php"),"<?php\nnamespace CodisseuNavigation;\ninterface BaseGateway {\n    public function deliver(): string;\n}\n").unwrap();
    let child_text = "<?php\r\nnamespace CodisseuNavigation;\r\nclass ChildCourier extends CourierService {\r\n    public function deliver(): string {\r\n        /* ação 😀 */ return 'child';\r\n    }\r\n}\r\n";
    let child_path = root.join("src/ChildCourier.php");
    fs::write(&child_path, child_text).unwrap();
    fs::write(root.join("src/OtherGateway.php"), "<?php\nnamespace CodisseuNavigation;\ninterface OtherGateway { public function deliver(): string; }\n").unwrap();
    fs::write(root.join("src/MultiService.php"), "<?php\nnamespace CodisseuNavigation;\nclass MultiService implements ParcelGateway, OtherGateway {\n    public function deliver(): string { return 'multi'; }\n}\n").unwrap();
    let originals = fs::read(root.join("src/CourierService.php")).unwrap();
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .current_dir(&root)
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut service = AppService::new(temp.path().join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_PHP_EDITOR_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .with_max_steps(100)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| !a.service.is_busy());
    open(&mut h, &root.join("main.php"));
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(
            configs
                .into_iter()
                .filter(|p| p.language == Language::Php)
                .collect(),
        ))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut h, |a| {
        a.service
            .language
            .statuses
            .iter()
            .any(|s| s.state == ProviderState::Ready)
            && !a.service.is_busy()
    });
    // Ctrl+Alt+U from inside the body must visit the immediate override, not
    // skip it for the top-level interface. Keep an unsaved edit throughout.
    open(&mut h, &child_path);
    let dirty_text = child_text.replace("'child'", "'dirty child'");
    let child_id = h.state().service.active.unwrap();
    let version = h.state().service.active_document().unwrap().version;
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id: child_id,
            expected_version: version,
            text: dirty_text.clone(),
        })
        .unwrap();
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.php_navigation.pending()
    });
    let index = dirty_text[..dirty_text.find("return").unwrap()]
        .chars()
        .count();
    let version = h.state().service.active_document().unwrap().version;
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id: child_id,
            expected_version: version,
            selection: codisseu_core::Selection::caret(index),
        })
        .unwrap();
    h.state_mut().focus_editor = true;
    h.step();
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::U);
    wait(&mut h, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path.ends_with("CourierService.php"))
            && !a.service.is_busy()
    });
    capture(&mut h, "super-immediate-override");
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::ArrowLeft);
    wait(&mut h, |a| {
        a.service.active == Some(child_id) && !a.service.is_busy()
    });
    assert_eq!(
        h.state().service.active_document().unwrap().text(),
        dirty_text
    );
    assert!(h.state().service.active_document().unwrap().is_dirty());
    assert_eq!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .selection()
            .head,
        index
    );
    assert_eq!(fs::read_to_string(&child_path).unwrap(), child_text);

    // A factory's type is a separate intent from its method declaration.
    open(&mut h, &root.join("main.php"));
    let d = h.state().service.active_document().unwrap();
    let index = d.text()[..d.text().find("create()").unwrap()]
        .chars()
        .count();
    let (id, expected_version) = (d.id, d.version);
    h.state_mut()
        .service
        .dispatch(Command::SetSelection {
            id,
            expected_version,
            selection: codisseu_core::Selection::caret(index),
        })
        .unwrap();
    h.state_mut().focus_editor = true;
    h.step();
    let sidebars = (h.state().show_project, h.state().show_database);
    h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::B);
    wait(&mut h, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path.ends_with("ParcelGateway.php"))
            && !a.service.is_busy()
    });
    assert_eq!((h.state().show_project, h.state().show_database), sidebars);
    capture(&mut h, "factory-return-type");
    open(&mut h, &root.join("src/CourierService.php"));
    wait(&mut h, |a| {
        a.editor_metrics().navigation_symbols >= 2 && !a.editor_metrics().navigation_pending
    });
    capture(&mut h, "gutter-method-base");
    let stats = h.state().php_navigation.stats();
    for _ in 0..100 {
        h.step();
    }
    assert_eq!(h.state().php_navigation.stats(), stats);
    h.get_by_label("Ir ao super de deliver").click();
    wait(&mut h, |a| {
        a.service
            .active_document()
            .is_some_and(|d| d.path.ends_with("BaseGateway.php"))
            && !a.service.is_busy()
    });
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .text()
            .contains("function deliver")
    );
    capture(&mut h, "transitive-base");
    wait(&mut h, |a| !a.php_navigation.pending());
    h.get_by_label("Buscar implementações de deliver").click();
    wait(&mut h, |a| a.service.navigation.choices.len() >= 2);
    capture(&mut h, "implementation-choices");
    h.key_press(Key::Escape);
    h.run();
    open(&mut h, &root.join("src/CourierService.php"));
    wait(&mut h, |a| !a.php_navigation.pending());
    h.get_by_label("Ir ao super de deliver").click_secondary();
    h.run();
    h.get_by_label("Localizar usos de deliver").click();
    wait(&mut h, |a| !a.service.references.is_pending());
    assert!(
        !h.state().service.references.items.is_empty(),
        "{:?}",
        h.state().service.references.error
    );
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .path
            .ends_with("CourierService.php")
    );
    capture(&mut h, "method-usages");
    assert_eq!(
        fs::read(root.join("src/CourierService.php")).unwrap(),
        originals
    );
    // Multiple contracts must remain explicit choices in both the method and
    // class gutter. Escape changes neither the current file nor its content.
    open(&mut h, &root.join("src/MultiService.php"));
    wait(&mut h, |a| !a.php_navigation.pending());
    h.get_by_label("Ir ao super de deliver").click();
    wait(&mut h, |a| a.service.navigation.choices.len() == 2);
    assert!(h.query_by_label("Escolher supermétodo/supertipo").is_some());
    let mut destinations: Vec<_> = h
        .state()
        .service
        .navigation
        .choices
        .iter()
        .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    destinations.sort();
    assert_eq!(destinations, ["BaseGateway.php", "OtherGateway.php"]);
    capture(&mut h, "multiple-super-methods");
    h.key_press(Key::Escape);
    h.run();
    assert!(
        h.state()
            .service
            .active_document()
            .unwrap()
            .path
            .ends_with("MultiService.php")
    );
    h.get_by_label("Ir ao super de MultiService").click();
    wait(&mut h, |a| a.service.navigation.choices.len() == 2);
    let mut destinations: Vec<_> = h
        .state()
        .service
        .navigation
        .choices
        .iter()
        .map(|t| t.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    destinations.sort();
    assert_eq!(destinations, ["OtherGateway.php", "ParcelGateway.php"]);
    capture(&mut h, "multiple-direct-supertypes");
    h.key_press(Key::ArrowDown);
    h.key_press(Key::Enter);
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.service.navigation.is_pending()
    });
    assert!(
        !h.state()
            .service
            .active_document()
            .unwrap()
            .path
            .ends_with("MultiService.php")
    );
    assert_eq!(fs::read_to_string(&child_path).unwrap(), child_text);
    assert!(!root.join("HOOK_EXECUTED").exists());
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
}

#[test]
fn large_php_idle_frames_do_not_reschedule_declaration_analysis() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.php");
    let mut text = String::from("<?php\nclass LargeService {\n");
    for n in 0..3000 {
        text.push_str(&format!("    public function method{n}(): int {{\n        // ação 😀\n        return {n};\n    }}\n\n"));
    }
    text.push_str("}\n");
    fs::write(&path, &text).unwrap();
    let mut service = AppService::new(temp.path().join("session.json"));
    service.dispatch(Command::OpenFile(path.clone())).unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 900.])
        .with_max_steps(100)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| {
        !a.service.is_busy() && !a.php_navigation.pending()
    });
    let stats = h.state().php_navigation.stats();
    assert!(
        h.state().php_navigation.error.is_none(),
        "{:?}",
        h.state().php_navigation.error
    );
    assert_eq!(stats.parses, 1);
    let mut samples = Vec::new();
    for _ in 0..200 {
        h.step();
        samples.push(h.state().editor_metrics().frame_micros);
    }
    assert_eq!(h.state().php_navigation.stats(), stats);
    assert!(h.state().service.language.statuses.is_empty());
    samples.sort_unstable();
    let metrics = serde_json::json!({"scenario":"own PHP 15004 lines;200 stable frames;no language provider","bytes":text.len(),"lines":h.state().service.active_document().unwrap().line_count(),"frames":samples.len(),"declaration_parses":stats.parses,"additional_idle_parses":0,"editor_frame_micros_median":samples[100],"editor_frame_micros_p95":samples[190],"rendered_lines":h.state().editor_metrics().visible_end-h.state().editor_metrics().visible_start,"partial":h.state().editor_metrics().navigation_partial});
    if let Some(out) = std::env::var_os("CODISSEU_PHP_EDITOR_CAPTURES") {
        fs::create_dir_all(&out).unwrap();
        fs::write(
            PathBuf::from(out).join("large-file-metrics.json"),
            serde_json::to_vec_pretty(&metrics).unwrap(),
        )
        .unwrap();
    }
    eprintln!("{metrics}");
    assert_eq!(fs::read_to_string(path).unwrap(), text);
}

#[test]
fn compact_navigation_gutter_preserves_breakpoint_and_fold_targets_at_both_scales() {
    for scale in [1., 1.25] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("compact.php");
        fs::write(&path, "<?php\nclass ChildService extends BaseService\n{\n    public function deliver(): int {\n        return 1;\n    }\n}\n").unwrap();
        let mut service = AppService::new(temp.path().join("session.json"));
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        let mut h = Harness::builder()
            .with_size([1280., 800.])
            .with_pixels_per_point(scale)
            .with_max_steps(100)
            .build_ui_state(
                |ui, a: &mut CodisseuApp| a.show(ui),
                CodisseuApp::new(service),
            );
        wait(&mut h, |a| {
            !a.service.is_busy()
                && !a.php_navigation.pending()
                && a.editor_metrics().folding_regions >= 2
        });
        let marker = h.get_by_label("Ir ao super de deliver").rect();
        let breakpoint_name = format!("Add breakpoint: {}:4", path.display());
        let breakpoint = h.get_by_label(&breakpoint_name).rect();
        let fold = h.get_by_label("Fold lines 4–6").rect();
        assert_eq!(marker.width(), 14.);
        assert!(
            marker.right() + 1. <= breakpoint.left(),
            "{scale}: {marker:?} vs {breakpoint:?}"
        );
        assert!(
            breakpoint.right() < fold.left(),
            "{scale}: {breakpoint:?} vs {fold:?}"
        );
        h.get_by_label(&breakpoint_name).click();
        h.run();
        assert!(
            h.query_by_label(&format!("Remove breakpoint: {}:4", path.display()))
                .is_some()
        );
        h.get_by_label("Fold lines 4–6").click();
        h.run();
        assert!(h.query_by_label("Unfold lines 4–6").is_some());
        capture(&mut h, &format!("compact-breakpoint-fold-{scale}"));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "<?php\nclass ChildService extends BaseService\n{\n    public function deliver(): int {\n        return 1;\n    }\n}\n"
        );
    }
}
