//! Explicit integration against a real rust-analyzer and rustfmt. No fallback
//! server, generated diagnostics or silent skip when the toolchain is missing.
use codisseu_language::{
    Language, LanguageCommand, LanguageService, ProviderConfig, ProviderState,
    SignatureHelpTrigger, utf16_byte_offset,
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

fn wait(s: &mut LanguageService, check: impl Fn(&LanguageService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        s.poll();
        if check(s) {
            return;
        }
        assert!(
            s.last_error.is_none(),
            "{:?}: {:?}",
            s.last_error,
            s.statuses
        );
        assert!(
            Instant::now() < deadline,
            "Rust timeout: {:?}, diagnostics {:?}, hover {:?}, completion {:?}",
            s.statuses,
            s.diagnostics,
            s.hover_error,
            s.completion_error
        );
        thread::sleep(Duration::from_millis(10));
    }
}
fn at(text: &str, byte: usize) -> (u32, u32) {
    let prefix = &text[..byte];
    (
        prefix.bytes().filter(|b| *b == b'\n').count() as u32,
        prefix.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
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
#[ignore = "real Rust integration; requires rust-analyzer, rust-src and rustfmt"]
fn rust_analyzer_unsaved_completion_navigation_signature_diagnostics_and_formatting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"codisseu_rust_fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::write(
        root.join("build.rs"),
        "fn main() { std::fs::write(\"BUILD_SCRIPT_RAN\", \"unexpected\").unwrap(); }\n",
    )
    .unwrap();
    let path = root.join("src/lib.rs");
    let disk = "// Original disk source remains unchanged.\npub fn original() {}\n";
    fs::write(&path, disk).unwrap();
    let mut s = LanguageService::new();
    let program =
        std::env::var_os("CODISSEU_RUST_ANALYZER").unwrap_or_else(|| "rust-analyzer".into());
    s.dispatch(LanguageCommand::Start {
        workspace: root.clone(),
        configs: vec![ProviderConfig {
            language: Language::Rust,
            program: program.into(),
            args: vec![],
        }],
    })
    .unwrap();
    let bad = "pub fn run() { let _emoji = \"😀\"; let broken = ; }\n";
    sync(&mut s, &path, 1, bad);
    wait(&mut s, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
            && s.diagnostics
                .iter()
                .any(|d| d.path == path && d.severity == 1)
    });
    let d = s
        .diagnostics
        .iter()
        .find(|d| d.path == path && d.severity == 1)
        .unwrap();
    let byte = utf16_byte_offset(bad, d.start_line, d.start_character).unwrap();
    assert!(
        byte >= bad.find("broken").unwrap(),
        "diagnostic follows emoji: {d:?}"
    );
    eprintln!("Rust syntax diagnostic: {d:?}");

    let source = "pub struct Weather { pub degrees: i32 }\nimpl Weather {\n    pub fn forecast(&self, extra: i32) -> i32 { self.degrees + extra }\n}\npub fn run() -> i32 {\n    let weather = Weather { degrees: 21 };\n    let _emoji = \"😀\"; weather.forecast(2)\n}\n";
    sync(&mut s, &path, 2, source);
    wait(&mut s, |s| {
        s.analyzed_versions.get(&path) == Some(&2)
            && s.diagnostics
                .iter()
                .all(|d| d.path != path || d.severity != 1)
    });
    let call = source.rfind("forecast").unwrap();
    let (line, character) = at(source, call + 2);
    // Handshake readiness and project indexing are distinct. A real server
    // may return ContentModified while its Cargo metadata is still loading.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        s.dispatch(LanguageCommand::Hover {
            path: path.clone(),
            version: 2,
            line,
            character,
        })
        .unwrap();
        wait(&mut s, |s| !s.hover_pending());
        if s.hover
            .as_ref()
            .is_some_and(|h| h.text.contains("forecast"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "hover {:?} {:?}",
            s.hover,
            s.hover_error
        );
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("Rust hover: {}", s.hover.as_ref().unwrap().text);
    s.dispatch(LanguageCommand::Definition {
        path: path.clone(),
        version: 2,
        line,
        character,
    })
    .unwrap();
    wait(&mut s, |s| !s.definition_pending());
    let definition = s.definition.as_ref().expect("real definition");
    assert!(
        definition
            .targets
            .iter()
            .any(|t| t.path == path && t.start_line == 2)
    );
    s.dispatch(LanguageCommand::References {
        path: path.clone(),
        version: 2,
        line,
        character,
        include_declaration: false,
    })
    .unwrap();
    wait(&mut s, |s| !s.references_pending());
    assert!(
        s.references
            .as_ref()
            .expect("real references")
            .targets
            .iter()
            .any(|t| t.path == path && t.start_line == line)
    );
    let (sl, sc) = at(source, call + "forecast(".len());
    s.dispatch(LanguageCommand::SignatureHelp {
        path: path.clone(),
        version: 2,
        line: sl,
        character: sc,
        trigger: SignatureHelpTrigger::Invoked,
        is_retrigger: false,
    })
    .unwrap();
    wait(&mut s, |s| !s.signature_help_pending());
    assert!(
        s.signature_help
            .as_ref()
            .expect("real signature")
            .signatures
            .iter()
            .any(|v| v.label.contains("extra"))
    );

    let completion = source.replace("forecast(2)", "fore");
    sync(&mut s, &path, 3, &completion);
    let (line, character) = at(
        &completion,
        completion.rfind("weather.fore").unwrap() + "weather.fore".len(),
    );
    s.dispatch(LanguageCommand::Completion {
        path: path.clone(),
        version: 3,
        line,
        character,
    })
    .unwrap();
    wait(&mut s, |s| !s.completion_pending());
    let result = s.completion.as_ref().expect("real completion");
    eprintln!(
        "Rust completion: {}",
        serde_json::to_string(result).unwrap()
    );
    let member = result
        .items
        .iter()
        .find(|i| i.label.starts_with("forecast"))
        .expect("forecast member");
    assert!(
        member.unsupported_reason.is_none(),
        "{:?}",
        member.unsupported_reason
    );
    assert!(!member.new_text.contains("$0") && !member.new_text.contains("${"));
    assert!(member.text_edit.is_some(), "server editing range retained");

    let unformatted = "pub fn format_me( )->i32{let x=2;x+1}\n";
    sync(&mut s, &path, 4, unformatted);
    s.dispatch(LanguageCommand::Formatting {
        path: path.clone(),
        version: 4,
        range: None,
        tab_size: 4,
        insert_spaces: true,
    })
    .unwrap();
    wait(&mut s, |s| !s.formatting_pending());
    assert!(s.formatting_error.is_none(), "{:?}", s.formatting_error);
    let edits = &s.formatting.as_ref().expect("rustfmt edits").edits;
    assert!(!edits.is_empty());
    let mut formatted: Vec<char> = unformatted.chars().collect();
    for e in edits.iter().rev() {
        formatted.splice(e.range.clone(), e.new_text.chars());
    }
    let formatted: String = formatted.into_iter().collect();
    assert!(
        formatted.contains("pub fn format_me() -> i32 {"),
        "{formatted}"
    );
    let external = "pub fn external() { let _value = std::vec::Vec::<u8>::new(); }\n";
    sync(&mut s, &path, 5, external);
    let (line, character) = at(external, external.find("Vec").unwrap() + 1);
    s.dispatch(LanguageCommand::Definition {
        path: path.clone(),
        version: 5,
        line,
        character,
    })
    .unwrap();
    wait(&mut s, |s| !s.definition_pending());
    assert!(s.definition_error.is_none(), "{:?}", s.definition_error);
    let targets = &s.definition.as_ref().expect("std Vec definition").targets;
    assert!(
        targets.iter().any(|target| !target.path.starts_with(&root)
            && codisseu_language::rust_library_root(&target.path).is_some()),
        "{targets:?}"
    );
    eprintln!("Rust std definition: {targets:?}");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        disk,
        "LSP never saves the buffer"
    );
    assert!(
        !root.join("BUILD_SCRIPT_RAN").exists(),
        "indexing must not run build.rs"
    );
    s.dispatch(LanguageCommand::Stop).unwrap();
    wait(&mut s, |s| {
        !s.is_busy() && s.statuses.iter().all(|p| p.state == ProviderState::Stopped)
    });
}
