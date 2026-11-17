//! Acceptance against the real analyzer, with no mock or synthetic hint fallback.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState, RenameRange,
    utf16_byte_offset,
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(s.last_error.is_none(), "{:?}", s.last_error);
        assert!(
            Instant::now() < end,
            "Rust timeout: {:?} actions {:?} hints {:?}",
            s.statuses,
            s.code_actions_error,
            s.inlay_hints_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn position(text: &str, byte: usize) -> (u32, u32) {
    let p = &text[..byte];
    (
        p.bytes().filter(|b| *b == b'\n').count() as u32,
        p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}
fn sync(s: &mut LanguageService, path: &Path, version: u64, text: &str) {
    s.dispatch(LanguageCommand::Sync {
        path: path.into(),
        version,
        text: text.into(),
    })
    .unwrap();
    wait(s, |s| !s.is_busy());
}
#[test]
#[ignore = "real rust-analyzer/rust-src integration for quick fixes and type/parameter inlay hints"]
fn real_import_quick_fix_resolves_and_real_type_parameter_hints_follow_unsaved_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname=\"codisseu_rust_features\"\nversion=\"0.1.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main(){std::fs::write(\"BUILD_SCRIPT_RAN\",\"unexpected\").unwrap();}",
    )
    .unwrap();
    let path = root.join("src/lib.rs");
    let disk = "// source on disk is preserved\npub fn original() {}\n";
    fs::write(&path, disk).unwrap();
    let mut s = LanguageService::new();
    s.dispatch(LanguageCommand::Start {
        workspace: root.clone(),
        configs: vec![ProviderConfig {
            language: Language::Rust,
            program: std::env::var_os("CODISSEU_RUST_ANALYZER")
                .unwrap_or_else(|| "rust-analyzer".into())
                .into(),
            args: vec![],
        }],
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    assert!(s.rust_capabilities[&Language::Rust].code_actions);
    assert!(s.rust_capabilities[&Language::Rust].inlay_hints);
    let text = "// ação 😀\r\npub fn combine(first: i32, second: i32) -> i32 { first + second }\r\npub fn run() -> i32 {\r\n    let mut values = HashMap::new();\r\n    values.insert(\"answer\", 42);\r\n    let _emoji = \"😀\"; let result = combine(40, 2);\r\n    result + values[\"answer\"]\r\n}\r\n";
    sync(&mut s, &path, 1, text);
    let (line, character) = position(text, text.find("HashMap").unwrap() + 2);
    let range = RenameRange {
        start_line: line,
        start_character: character,
        end_line: line,
        end_character: character,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let token = loop {
        s.dispatch(LanguageCommand::CodeActions {
            path: path.clone(),
            version: 1,
            range: range.clone(),
        })
        .unwrap();
        wait(&mut s, |s| !s.code_actions_pending());
        if let Some(action) = s.code_actions.as_ref().and_then(|r| {
            r.items
                .iter()
                .find(|i| i.title.contains("HashMap") && i.disabled_reason.is_none())
        }) {
            eprintln!("Real import action: {}", action.title);
            break action.token;
        }
        assert!(
            Instant::now() < deadline,
            "No HashMap action: {:?} {:?}",
            s.code_actions,
            s.code_actions_error
        );
        thread::sleep(Duration::from_millis(100));
    };
    s.dispatch(LanguageCommand::ResolveCodeAction { token })
        .unwrap();
    wait(&mut s, |s| !s.code_actions_pending());
    assert!(s.code_actions_error.is_none(), "{:?}", s.code_actions_error);
    let edit = s.code_action_edit.as_ref().unwrap();
    assert_eq!(edit.changes.len(), 1);
    assert_eq!(edit.changes[0].path, path);
    let mut updated = text.to_owned();
    let mut changes = edit.changes[0]
        .edits
        .iter()
        .map(|e| {
            let start = utf16_byte_offset(text, e.start_line, e.start_character).unwrap();
            let end = utf16_byte_offset(text, e.end_line, e.end_character).unwrap();
            (start, end, e.new_text.clone())
        })
        .collect::<Vec<_>>();
    changes.sort_by_key(|e| e.0);
    for (start, end, text) in changes.into_iter().rev() {
        updated.replace_range(start..end, &text);
    }
    assert!(
        updated.contains("use std::collections::HashMap;"),
        "{updated}"
    );
    assert!(updated.contains("ação 😀"));
    assert_eq!(fs::read_to_string(&path).unwrap(), disk);
    sync(&mut s, &path, 2, &updated);
    let lines = updated.bytes().filter(|b| *b == b'\n').count() as u32;
    s.dispatch(LanguageCommand::InlayHints {
        path: path.clone(),
        version: 2,
        range: RenameRange {
            start_line: 0,
            start_character: 0,
            end_line: lines,
            end_character: 0,
        },
    })
    .unwrap();
    wait(&mut s, |s| !s.inlay_hints_pending());
    assert!(s.inlay_hints_error.is_none(), "{:?}", s.inlay_hints_error);
    let result = s.inlay_hints.as_ref().unwrap();
    eprintln!("Real inlay hints: {result:?}");
    assert_eq!(result.version, 2);
    assert!(
        result
            .items
            .iter()
            .any(|h| h.kind == Some(1) && h.label.contains("i32")),
        "{:?}",
        result.items
    );
    assert!(
        result
            .items
            .iter()
            .any(|h| h.kind == Some(2) && h.label.contains("first")),
        "{:?}",
        result.items
    );
    assert!(
        result
            .items
            .iter()
            .any(|h| h.kind == Some(2) && h.label.contains("second")),
        "{:?}",
        result.items
    );
    let typed = result
        .items
        .iter()
        .find(|h| h.kind == Some(1) && h.label == ": i32")
        .unwrap();
    let offset = utf16_byte_offset(&updated, typed.line, typed.character).unwrap();
    assert!(
        updated[..offset].ends_with("result"),
        "type hint must follow identifier after emoji"
    );
    let parameter = result
        .items
        .iter()
        .find(|h| h.kind == Some(2) && h.label == "first:")
        .unwrap();
    let offset = utf16_byte_offset(&updated, parameter.line, parameter.character).unwrap();
    assert!(
        updated[offset..].starts_with("40"),
        "parameter hint keeps UTF-16 position after emoji"
    );
    assert!(!root.join("BUILD_SCRIPT_RAN").exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), disk);
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
}
