//! Real Basedpyright, private interpreter, unsaved Unicode/CRLF snapshots.
use codisseu_language::{
    Language, LanguageCommand as C, LanguageService, ProviderConfig, ProviderState, RenameRange,
    SignatureHelpTrigger,
};
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn wait(s: &mut LanguageService, f: impl Fn(&LanguageService) -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        s.poll();
        if f(s) {
            return;
        }
        assert!(
            Instant::now() < end,
            "timeout {:?} error {:?} completion {:?} hints {:?}",
            s.statuses,
            s.last_error,
            s.completion_error,
            s.inlay_hints_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn position(s: &str, byte: usize) -> (u32, u32) {
    let p = &s[..byte];
    (
        p.bytes().filter(|b| *b == b'\n').count() as u32,
        p.rsplit('\n').next().unwrap().encode_utf16().count() as u32,
    )
}
fn sync(s: &mut LanguageService, path: &Path, version: u64, text: &str) {
    s.dispatch(C::Sync {
        path: path.into(),
        version,
        text: text.into(),
    })
    .unwrap();
    wait(s, |s| !s.is_busy());
}
fn config() -> ProviderConfig {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools");
    ProviderConfig {
        language: Language::Python,
        program: root
            .join("node-v24.22.0-linux-x64/bin/node")
            .canonicalize()
            .unwrap(),
        args: vec![
            root.join("python/node_modules/basedpyright/langserver.index.js")
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            "--stdio".into(),
        ],
    }
}
#[test]
#[ignore = "real Basedpyright1.40.2 and private CPython3.13.18"]
fn python_real_completion_autoimport_hover_signature_navigation_rename_hints_and_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let metrics = root.join("metrics.py");
    fs::write(
        &metrics,
        "def rainfall(first: int, second: int) -> int:\r\n    return first + second\r\n",
    )
    .unwrap();
    fs::write(root.join("models.py"), "class WeatherStation:\n    pass\n").unwrap();
    let path = root.join("ação.py");
    let disk = "# disk untouched\r\n";
    fs::write(&path, disk).unwrap();
    fs::write(
        root.join("pyrightconfig.json"),
        "{\"typeCheckingMode\":\"standard\"}",
    )
    .unwrap();
    let python = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.tools/python/cpython/bin/python3")
        .canonicalize()
        .unwrap();
    let mut s = LanguageService::new();
    s.dispatch(C::PythonEnvironment {
        python: Some(python),
    })
    .unwrap();
    s.dispatch(C::Start {
        workspace: root.clone(),
        configs: vec![config()],
    })
    .unwrap();
    wait(&mut s, |s| {
        s.statuses.iter().any(|p| p.state == ProviderState::Ready)
    });
    assert!(s.rust_capabilities[&Language::Python].inlay_hints);
    let source = "from metrics import rainfall\r\n# rainfall 😀\r\nlabel = 'rainfall'\r\ndef run() -> int:\r\n    emoji = '😀'; result = rainfall(40, 2)\r\n    return result\r\nbad: int = 'wrong'\r\n";
    sync(&mut s, &path, 1, source);
    wait(&mut s, |s| {
        s.analyzed_versions.get(&path) == Some(&1) && s.diagnostics.iter().any(|d| d.severity == 1)
    });
    eprintln!("diagnostics {:?}", s.diagnostics);
    let (line, character) = position(source, source.rfind("rainfall(").unwrap() + 2);
    s.dispatch(C::Hover {
        path: path.clone(),
        version: 1,
        line,
        character,
    })
    .unwrap();
    wait(&mut s, |s| !s.hover_pending());
    assert!(s.hover.as_ref().unwrap().text.contains("rainfall"));
    s.dispatch(C::Definition {
        path: path.clone(),
        version: 1,
        line,
        character,
    })
    .unwrap();
    wait(&mut s, |s| !s.definition_pending());
    assert!(
        s.definition
            .as_ref()
            .expect("real definition")
            .targets
            .iter()
            .any(|t| t.path == metrics)
    );
    s.dispatch(C::References {
        path: path.clone(),
        version: 1,
        line,
        character,
        include_declaration: true,
    })
    .unwrap();
    wait(&mut s, |s| !s.references_pending());
    assert!(
        s.references
            .as_ref()
            .expect("real refs")
            .targets
            .iter()
            .any(|t| t.path == path)
    );
    let (sl, sc) = position(source, source.rfind("rainfall(").unwrap() + 9);
    s.dispatch(C::SignatureHelp {
        path: path.clone(),
        version: 1,
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
            .expect("signature")
            .signatures
            .iter()
            .any(|s| s.label.contains("first"))
    );
    s.dispatch(C::InlayHints {
        path: path.clone(),
        version: 1,
        range: RenameRange {
            start_line: 0,
            start_character: 0,
            end_line: 7,
            end_character: 0,
        },
    })
    .unwrap();
    wait(&mut s, |s| !s.inlay_hints_pending());
    let hints = s.inlay_hints.as_ref().expect("hints");
    eprintln!("hints {hints:?}");
    assert!(hints.items.iter().any(|h| h.kind == Some(1)));
    assert!(hints.items.iter().any(|h| h.label.contains("first")));
    s.dispatch(C::Rename {
        path: path.clone(),
        version: 1,
        line,
        character,
        new_name: "precipitação".into(),
    })
    .unwrap();
    wait(&mut s, |s| !s.rename_pending());
    let rename = s.rename.as_ref().expect("real rename");
    eprintln!("rename {rename:?}");
    assert!(rename.changes.iter().any(|c| c.path == metrics));
    assert!(rename.changes.iter().any(|c| c.path == path));
    let complete = "# 😀\r\nvalue = WeatherSta\r\n";
    sync(&mut s, &path, 2, complete);
    let (line, character) = position(complete, complete.find("WeatherSta").unwrap() + 10);
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        s.dispatch(C::Completion {
            path: path.clone(),
            version: 2,
            line,
            character,
        })
        .unwrap();
        wait(&mut s, |s| !s.completion_pending());
        if let Some(item) = s
            .completion
            .as_ref()
            .and_then(|r| r.items.iter().find(|i| i.label.contains("WeatherStation")))
        {
            eprintln!("autoimport {item:?}");
            assert!(item.unsupported_reason.is_none(), "{item:?}");
            assert!(
                !item.additional_edits.is_empty(),
                "autoimport must include import edits"
            );
            break;
        }
        assert!(Instant::now() < end, "noautoimport {:?}", s.completion);
        thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(fs::read_to_string(path).unwrap(), disk);
    s.dispatch(C::Stop).unwrap();
    wait(&mut s, |s| !s.is_busy());
}
