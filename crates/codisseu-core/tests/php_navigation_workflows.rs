//! Real provider regression for PHPDoc-only factories and chained navigation.
//! All code belongs to the temporary corpus; factory bodies are never executed.
use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        assert!(
            app.last_error.is_none(),
            "{:?} / {}",
            app.last_error,
            app.status
        );
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < until,
            "{} / {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}

fn open(app: &mut AppService, path: &Path) {
    app.dispatch(Command::OpenFile(path.into())).unwrap();
    wait(app, |a| !a.is_busy());
}

fn cursor(app: &AppService, needle: &str) -> (u64, u64, usize) {
    let doc = app.active_document().unwrap();
    (
        doc.id,
        doc.version,
        doc.text()[..doc.text().rfind(needle).unwrap()]
            .chars()
            .count()
            + 1,
    )
}

#[test]
#[ignore = "real PHPDoc factory navigation; requires CODISSEU_PROVIDER_CONFIG and Composer"]
fn phpdoc_only_factory_property_and_history_preserve_dirty_documents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    let root = root.canonicalize().unwrap();
    let sources = [
        (
            "composer.json",
            r#"{"name":"codisseu/navigation-workflows","autoload":{"psr-4":{"NavigationWorkflows\\":"src/"}},"scripts":{"post-autoload-dump":"@php -r \"file_put_contents('HOOK_EXECUTED','forbidden');\""}}"#,
        ),
        (
            "src/Port.php",
            "<?php\nnamespace NavigationWorkflows;\ninterface Port { public function deliver(): string; }\n",
        ),
        (
            "src/Courier.php",
            "<?php\nnamespace NavigationWorkflows;\nclass Courier implements Port { public function deliver(): string { return 'courier'; } }\n",
        ),
        (
            "src/Local.php",
            "<?php\nnamespace NavigationWorkflows;\nclass Local implements Port { public function deliver(): string { return 'local'; } }\n",
        ),
        (
            "src/Unrelated.php",
            "<?php\nnamespace NavigationWorkflows;\nclass Unrelated { public function deliver(): string { return 'unrelated'; } }\n",
        ),
        // No concrete return expression: resolving callers depends on @return.
        (
            "src/Factory.php",
            "<?php\nnamespace NavigationWorkflows;\nclass Factory {\n    /** @return Port */\n    public static function create() { throw new \\RuntimeException('never execute'); }\n}\n",
        ),
        (
            "src/Consumer.php",
            "<?php\nnamespace NavigationWorkflows;\nclass Consumer {\n    /** @var Port */\n    private $port;\n    public function run(): string { return $this->port->deliver(); }\n}\n",
        ),
        (
            "main.php",
            "<?php\nuse NavigationWorkflows\\Factory;\nuse NavigationWorkflows\\Unrelated;\necho 'ação 😀'; $parcel = Factory::create();\n$parcel->deliver();\n(new Unrelated())->deliver();\n",
        ),
    ];
    for (path, source) in sources {
        fs::write(root.join(path), source).unwrap();
    }
    let output = std::process::Command::new("composer")
        .args([
            "dump-autoload",
            "--no-scripts",
            "--no-plugins",
            "--no-interaction",
        ])
        .env("COMPOSER_DISABLE_NETWORK", "1")
        .current_dir(&root)
        .output()
        .expect("Composer required");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").expect("provider config required"))
            .unwrap(),
    )
    .unwrap();
    let mut app = AppService::new(dir.path().join("profile/session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    open(&mut app, &root.join("main.php"));
    app.dispatch(Command::ConfigureProviders(
        configs
            .into_iter()
            .filter(|c| c.language == Language::Php)
            .collect(),
    ))
    .unwrap();
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        !a.is_busy()
            && a.language
                .statuses
                .iter()
                .any(|s| s.state == ProviderState::Ready)
    });

    // Keep a dirty contract open: navigation must use its current coordinates.
    open(&mut app, &root.join("src/Port.php"));
    let doc = app.active_document().unwrap();
    let port_id = doc.id;
    let dirty_port = format!("\n{}", doc.text());
    app.dispatch(Command::Edit {
        id: port_id,
        expected_version: doc.version,
        text: dirty_port.clone(),
    })
    .unwrap();

    for (source, needle, destination) in [
        ("main.php", "create();", "src/Factory.php"),
        ("main.php", "deliver();\n(new", "src/Port.php"),
        ("src/Consumer.php", "deliver();", "src/Port.php"),
        ("main.php", "deliver();", "src/Unrelated.php"),
    ] {
        open(&mut app, &root.join(source));
        let (id, expected_version, at) = cursor(&app, needle);
        app.dispatch(Command::Definition {
            id,
            expected_version,
            character_index: at,
        })
        .unwrap();
        wait(&mut app, |a| !a.is_busy());
        assert_eq!(
            app.active_document().unwrap().path,
            root.join(destination),
            "{source} {needle}"
        );
        let target = app.active_document().unwrap();
        let target_id = target.id;
        let target_version = target.version;
        let target_at = app.navigation.selection.as_ref().unwrap().range.start;
        app.dispatch(Command::NavigateBack {
            id: target_id,
            expected_version: target_version,
            character_index: target_at,
        })
        .unwrap();
        assert_eq!(app.active, Some(id));
        assert_eq!(app.navigation.selection.as_ref().unwrap().range, at..at);
        app.dispatch(Command::NavigateForward {
            id,
            expected_version,
            character_index: at,
        })
        .unwrap();
        assert_eq!(app.active, Some(target_id));
        assert_eq!(
            app.navigation.selection.as_ref().unwrap().range,
            target_at..target_at
        );
        eprintln!("{source}: {needle} -> {destination}; Back/Forward exact");
    }

    open(&mut app, &root.join("main.php"));
    let (id, expected_version, at) = cursor(&app, "deliver();\n(new");
    app.dispatch(Command::Implementation {
        id,
        expected_version,
        character_index: at,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    let mut targets: Vec<_> = app
        .navigation
        .choices
        .iter()
        .map(|t| t.path.clone())
        .collect();
    targets.sort();
    assert_eq!(
        targets,
        [root.join("src/Courier.php"), root.join("src/Local.php")]
    );
    assert_eq!(app.active, Some(id));
    app.dispatch(Command::CancelDefinition).unwrap();
    open(&mut app, &root.join("src/Port.php"));
    let (id, expected_version, at) = cursor(&app, "deliver():");
    app.dispatch(Command::References {
        id,
        expected_version,
        character_index: at,
    })
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(app.references.error.is_none(), "{:?}", app.references.error);
    let usages: Vec<_> = app
        .references
        .items
        .iter()
        .map(|r| (r.target.path.clone(), r.target.start_line))
        .collect();
    assert!(usages.contains(&(root.join("main.php"), 4)), "{usages:?}");
    assert!(
        usages.contains(&(root.join("src/Consumer.php"), 5)),
        "{usages:?}"
    );
    assert!(
        !usages.contains(&(root.join("main.php"), 5)),
        "unrelated same-name call: {usages:?}"
    );
    assert!(
        !usages
            .iter()
            .any(|(path, _)| path.ends_with("Unrelated.php"))
    );
    let port = app.documents.iter().find(|d| d.id == port_id).unwrap();
    assert!(port.is_dirty());
    assert_eq!(port.text(), dirty_port);
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for (path, source) in sources {
        assert_eq!(fs::read_to_string(root.join(path)).unwrap(), source);
    }
    assert!(!root.join("HOOK_EXECUTED").exists());
}
