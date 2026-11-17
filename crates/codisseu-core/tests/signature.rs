//! Real providers, shared application commands, unsaved Unicode snapshots.
use codisseu_core::{AppService, Command, ProviderState, SignatureHelpTrigger};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(app: &mut AppService, check: impl Fn(&AppService) -> bool) {
    let end = Instant::now() + Duration::from_secs(45);
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
            Instant::now() < end,
            "{} / {:?}",
            app.status,
            app.language.statuses
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn query(app: &mut AppService, caret: usize) {
    let d = app.active_document().unwrap();
    app.dispatch(Command::SignatureHelp {
        id: d.id,
        expected_version: d.version,
        character_index: caret,
        trigger: SignatureHelpTrigger::Invoked,
        is_retrigger: false,
    })
    .unwrap();
    assert!(app.signature_help.is_pending());
    wait(app, |a| !a.signature_help.is_pending());
    assert!(
        app.signature_help.error.is_none(),
        "{:?}",
        app.signature_help.error
    );
}
#[test]
#[ignore = "real PHP/TypeScript signature help; requires CODISSEU_PROVIDER_CONFIG"]
fn real_php_ts_signatures_follow_unsaved_unicode_parameters_overloads_and_revisions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let php = "<?php\n/** Forecast documentation. */\nfunction forecast(string $city, int $days = 1): string { return $city; }\necho '🌧 e\u{301}'; forecast('Paris', 2);\n";
    let ts = "export {};\n/** Forecast documentation. */\nfunction forecast(city: string, days: number): string;\nfunction forecast(city: number, days: boolean): number;\nfunction forecast(city: string | number, days: number | boolean): string | number { return city; }\nconsole.log('🌧 e\u{301}'); forecast('Paris', 2);\n";
    let mut app = AppService::new(root.join("session.json"));
    app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
    wait(&mut app, |a| !a.is_busy());
    for (name, text, saved) in [
        ("signature.php", php, "<?php // saved\n"),
        ("signature.ts", ts, "export {}; // saved\n"),
    ] {
        fs::write(root.join(name), saved).unwrap();
        app.dispatch(Command::OpenFile(root.join(name))).unwrap();
        wait(&mut app, |a| !a.is_busy());
        let d = app.active_document().unwrap();
        app.dispatch(Command::Edit {
            id: d.id,
            expected_version: d.version,
            text: text.into(),
        })
        .unwrap();
    }
    app.dispatch(Command::LoadProviderConfig(
        std::env::var_os("CODISSEU_PROVIDER_CONFIG")
            .expect("explicit provider configuration")
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
    for (name, text, saved) in [
        ("signature.ts", ts, "export {}; // saved\n"),
        ("signature.php", php, "<?php // saved\n"),
    ] {
        let id = app
            .documents
            .iter()
            .find(|d| d.path == root.join(name))
            .unwrap()
            .id;
        app.dispatch(Command::Activate { id }).unwrap();
        for (suffix, expected_parameter) in [("'Paris'", 0), ("2);", 1)] {
            let byte = text.rfind(suffix).unwrap();
            query(&mut app, text[..byte].chars().count());
            let r = app.signature_help.result.as_ref().unwrap();
            eprintln!(
                "{name}/{expected_parameter}: {}",
                serde_json::to_string(r).unwrap()
            );
            assert_eq!(r.version, 2);
            assert!(
                r.signatures
                    .iter()
                    .any(|s| s.label.contains("city") && s.label.contains("days"))
            );
            assert_eq!(r.active_parameter, Some(expected_parameter));
            if name.ends_with("ts") {
                assert_eq!(r.signatures.len(), 2);
            }
            assert_eq!(app.active_document().unwrap().text(), text);
        }
        let updated = text.replace("days", "nights");
        app.dispatch(Command::Edit {
            id,
            expected_version: 2,
            text: updated.clone(),
        })
        .unwrap();
        assert!(!app.signature_help.is_open());
        assert!(app.signature_help.result.is_none());
        let caret = updated[..updated.rfind("2);").unwrap()].chars().count();
        query(&mut app, caret);
        let r = app.signature_help.result.as_ref().unwrap();
        assert_eq!(r.version, 3);
        assert!(r.signatures.iter().any(|s| s.label.contains("nights")));
        assert!(!r.signatures.iter().any(|s| s.label.contains("days")));
        app.dispatch(Command::Undo { id }).unwrap();
        assert!(!app.signature_help.is_open());
        assert_eq!(app.active_document().unwrap().text(), text);
        query(&mut app, 0);
        assert!(
            app.signature_help
                .result
                .as_ref()
                .unwrap()
                .signatures
                .is_empty()
        );
        app.dispatch(Command::CancelSignatureHelp).unwrap();
        assert!(!app.signature_help.is_open());
        assert_eq!(fs::read_to_string(root.join(name)).unwrap(), saved);
    }
    app.dispatch(Command::StopProviders).unwrap();
    wait(&mut app, |a| !a.is_busy());
    assert!(!app.signature_help.is_open());
}
