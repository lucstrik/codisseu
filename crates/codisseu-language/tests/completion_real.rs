//! Explicit integration of plain-text member completions over unsaved buffers.
use codisseu_language::{
    CompletionItem, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
    utf16_byte_offset,
};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert!(
            Instant::now() < deadline,
            "{:?}, completion {:?}",
            s.statuses,
            s.completion_error
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn apply(text: &str, caret: usize, item: &CompletionItem) -> String {
    assert!(
        item.unsupported_reason.is_none(),
        "{:?}",
        item.unsupported_reason
    );
    let range = if let Some(e) = &item.text_edit {
        utf16_byte_offset(text, e.start_line, e.start_character).unwrap()
            ..utf16_byte_offset(text, e.end_line, e.end_character).unwrap()
    } else {
        let start = text[..caret]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
            .map_or(0, |b| b + text[b..].chars().next().unwrap().len_utf8());
        start..caret
    };
    let mut result = text.to_string();
    result.replace_range(range, &item.new_text);
    result
}
#[test]
#[ignore = "real PHP/TypeScript completion; requires CODISSEU_PROVIDER_CONFIG"]
fn real_php_and_typescript_member_completion_tracks_unsaved_unicode_revisions() {
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(
            std::env::var_os("CODISSEU_PROVIDER_CONFIG")
                .expect("CODISSEU_PROVIDER_CONFIG required"),
        )
        .unwrap(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let php = root.join("completion.php");
    let ts = root.join("completion.ts");
    fs::write(&php, "<?php // saved marker\n").unwrap();
    fs::write(&ts, "export {}; // saved marker\n").unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{"compilerOptions":{"strict":true,"target":"ES2022"},"include":["*.ts"]}"#,
    )
    .unwrap();
    let php_text = "<?php\nclass Weather { public function rainfall(): string { return 'rain'; } public function raincoat(): string { return 'coat'; } public function forecast(): string { return 'sun'; } }\n$weather = new Weather();\necho '🌧 e\u{301}'; $weather->rain";
    let ts_text = "class Weather { rainfall(): string { return 'rain'; } raincoat(): string { return 'coat'; } forecast(): string { return 'sun'; } }\nconst weather = new Weather();\nconsole.log('🌧 e\u{301}'); weather.rain";
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root,
        configs,
    })
    .unwrap();
    for (path, text) in [(&ts, ts_text), (&php, php_text)] {
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 2,
            text: text.into(),
        })
        .unwrap();
    }
    wait(&mut s, |s| {
        s.statuses.len() == 2
            && s.statuses.iter().all(|p| p.state == ProviderState::Ready)
            && !s.is_busy()
    });
    for (path, text) in [(&ts, ts_text), (&php, php_text)] {
        let line = text.lines().count() as u32 - 1;
        let column = text.lines().last().unwrap().encode_utf16().count() as u32;
        s.dispatch(LanguageCommand::Completion {
            path: path.clone(),
            version: 2,
            line,
            character: column,
        })
        .unwrap();
        wait(&mut s, |s| !s.completion_pending());
        assert!(s.completion_error.is_none(), "{:?}", s.completion_error);
        let result = s.completion.as_ref().unwrap();
        assert_eq!(result.version, 2);
        assert_eq!(result.path, *path);
        eprintln!(
            "{path:?}: {}",
            serde_json::to_string_pretty(result).unwrap()
        );
        let item = result
            .items
            .iter()
            .find(|i| i.label.starts_with("rainfall"))
            .expect("real rainfall member");
        assert!(
            item.unsupported_reason.is_none(),
            "{:?}",
            item.unsupported_reason
        );
        assert!(item.additional_edits.is_empty());
        let completed = apply(text, text.len(), item);
        assert!(completed.contains("rainfall"));
        assert!(!completed.contains("rainrainfall"));
        let updated = text
            .replace("rainfall", "stormfall")
            .replace("->rain", "->storm")
            .replace(".rain", ".storm");
        s.dispatch(LanguageCommand::Sync {
            path: path.clone(),
            version: 3,
            text: updated.clone(),
        })
        .unwrap();
        assert!(s.completion.is_none());
        let line = updated.lines().count() as u32 - 1;
        let column = updated.lines().last().unwrap().encode_utf16().count() as u32;
        s.dispatch(LanguageCommand::Completion {
            path: path.clone(),
            version: 3,
            line,
            character: column,
        })
        .unwrap();
        wait(&mut s, |s| !s.completion_pending());
        assert!(s.completion_error.is_none(), "{:?}", s.completion_error);
        let result = s.completion.as_ref().unwrap();
        assert_eq!(result.version, 3);
        eprintln!(
            "updated {path:?}: {}",
            serde_json::to_string_pretty(result).unwrap()
        );
        assert!(
            result
                .items
                .iter()
                .any(|i| i.label.starts_with("stormfall") && i.unsupported_reason.is_none())
        );
    }
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
    assert_eq!(fs::read_to_string(php).unwrap(), "<?php // saved marker\n");
    assert_eq!(
        fs::read_to_string(ts).unwrap(),
        "export {}; // saved marker\n"
    );
}
