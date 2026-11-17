//! Explicit real-provider formatting. Synthetic files only; no on-disk writes by formatting.
use codisseu_core::{AppService, Command, ProviderState};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(45);
    loop {
        app.poll();
        if check(app) {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?} {:?}",
            app.status,
            app.last_error,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "explicit PHP/TypeScript formatting; requires CODISSEU_PROVIDER_CONFIG"]
fn configured_real_providers_negotiate_format_unsaved_unicode_and_keep_single_undo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("composer.json"),
        "{\"name\":\"storm/formatting-fixture\"}",
    )
    .unwrap();
    fs::write(
        root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true,\"target\":\"ES2022\"},\"include\":[\"*.ts\"]}",
    )
    .unwrap();
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for (name, disk) in [
        ("format.ts", "export {};\r\n"),
        ("format.php", "<?php // saved\r\n"),
    ] {
        fs::write(root.join(name), disk).unwrap();
        app.dispatch(Command::OpenFile(root.join(name))).unwrap();
        wait(&mut app, |a| !a.is_busy());
    }
    app.dispatch(Command::LoadProviderConfig(
        std::env::var_os("CODISSEU_PROVIDER_CONFIG")
            .expect("explicit provider config")
            .into(),
    ))
    .unwrap();
    wait(&mut app, |a| !a.is_busy());
    app.dispatch(Command::StartProviders).unwrap();
    wait(&mut app, |a| {
        a.language.statuses.len() == 2
            && a.language
                .statuses
                .iter()
                .all(|s| s.state == ProviderState::Ready)
            && !a.is_busy()
    });
    for (name, dirty) in [
        (
            "format.ts",
            "// 🌧 e\u{301}\r\nexport function add(a:number,b:number){return a+b;}\r\n",
        ),
        (
            "format.php",
            "<?php\r\n// 🌧 e\u{301}\r\nfunction add(int $a,int $b):int{return $a+$b;}\r\n",
        ),
    ] {
        let id = app
            .documents
            .iter()
            .find(|d| d.path == root.join(name))
            .unwrap()
            .id;
        app.dispatch(Command::Activate { id }).unwrap();
        let version = app.active_document().unwrap().version;
        let disk = fs::read_to_string(root.join(name)).unwrap();
        app.dispatch(Command::Edit {
            id,
            expected_version: version,
            text: dirty.into(),
        })
        .unwrap();
        let version = app.active_document().unwrap().version;
        let language = codisseu_language::Language::for_path(&root.join(name)).unwrap();
        let capability = app.language.formatting_capabilities[&language].clone();
        eprintln!(
            "{name}: document={} range={}",
            capability.document, capability.range
        );
        let started = app.dispatch(Command::FormatDocument {
            id,
            expected_version: version,
            range: None,
        });
        if !capability.document {
            assert!(started.is_err());
            assert_eq!(app.active_document().unwrap().text(), dirty);
            eprintln!("{name}: explicit unsupported-capability feedback observed");
            continue;
        }
        started.unwrap();
        wait(&mut app, |a| !a.formatting.is_pending());
        assert!(
            app.formatting.error.is_none(),
            "{name}: {:?}",
            app.formatting.error
        );
        let after = app.active_document().unwrap().text().to_string();
        assert_ne!(
            after, dirty,
            "{name}: real formatter must return effective edits"
        );
        assert!(after.contains("🌧 e\u{301}"));
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), disk);
        app.dispatch(Command::Undo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::Redo { id }).unwrap();
        assert_eq!(app.active_document().unwrap().text(), after);
        if capability.range {
            app.dispatch(Command::Undo { id }).unwrap();
            let version = app.active_document().unwrap().version;
            let start = dirty[..dirty.find("function").unwrap()].chars().count();
            let end = dirty.trim_end_matches(['\r', '\n']).chars().count();
            app.dispatch(Command::FormatDocument {
                id,
                expected_version: version,
                range: Some(start..end),
            })
            .unwrap();
            wait(&mut app, |a| !a.formatting.is_pending());
            assert!(
                app.formatting.error.is_none(),
                "{name} range: {:?}",
                app.formatting.error
            );
            assert_eq!(fs::read_to_string(root.join(name)).unwrap(), disk);
            assert_ne!(
                app.active_document().unwrap().text(),
                dirty,
                "{name}: range formatter must return effective edits"
            );
            eprintln!("{name}: real range formatter applied effective edits");
        }
    }
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(
        app.language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Stopped)
    );
}
