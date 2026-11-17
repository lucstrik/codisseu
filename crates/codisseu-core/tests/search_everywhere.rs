//! Real disposable project files exercise the independent search worker.
use codisseu_core::search_everywhere::*;
use codisseu_editor::TextBuffer;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};
fn request(root: &Path, query: &str, category: Category) -> Request {
    Request {
        encoding_profile: None,
        root: Some(root.into()),
        external_roots: vec![],
        buffers: vec![],
        query: query.into(),
        category,
        filter: FileFilter::All,
        include_non_project: false,
        only_open: false,
        refresh: 1,
        preview: false,
        contributors: ALL_CONTRIBUTORS,
    }
}
fn finish(service: &mut SearchEverywhereService) -> Response {
    let deadline = Instant::now() + Duration::from_secs(15);
    while service.pending() {
        service.poll();
        assert!(Instant::now() < deadline, "search did not drain");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.error.is_none(), "{:?}", service.error);
    service.result.clone().expect("current response")
}
#[test]
fn six_categories_use_files_actual_ast_and_text_with_dirty_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir(root.join("src")).unwrap();
    fs::write(root.join("src/Greeter.ts"), "// class FakeGreeter {}\nexport class Greeter { greet() { return 'greet text'; } }\nexport function greetingTitle() {}\n").unwrap();
    fs::write(
        root.join("src/Greeting.php"),
        "<?php class Greeting { public function greet() {} }\n",
    )
    .unwrap();
    let mut service = SearchEverywhereService::new();
    for (category, expected) in [
        (Category::Files, HitKind::File),
        (Category::Classes, HitKind::Class),
        (Category::Symbols, HitKind::Symbol),
        (Category::Text, HitKind::Text),
    ] {
        service.submit(request(root, "greet", category)).unwrap();
        let result = finish(&mut service);
        assert!(
            result.hits.iter().any(|h| h.kind == expected),
            "{category:?}: {:?}",
            result.hits
        );
        assert!(
            !result
                .hits
                .iter()
                .any(|h| h.kind != HitKind::Text && h.title == "FakeGreeter")
        );
    }
    service
        .submit(request(root, "greet", Category::All))
        .unwrap();
    let all = finish(&mut service);
    for expected in [
        HitKind::Class,
        HitKind::File,
        HitKind::Symbol,
        HitKind::Text,
    ] {
        assert!(all.hits.iter().any(|h| h.kind == expected));
    }
    let mut dirty = request(root, "UnsavedGreeting", Category::Classes);
    dirty.buffers.push(BufferSnapshot {
        path: root.join("src/Greeter.ts"),
        version: 7,
        text: TextBuffer::new("export class UnsavedGreeting {}".into())
            .unwrap()
            .snapshot(),
    });
    service.submit(dirty).unwrap();
    let result = finish(&mut service);
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].buffer_version, Some(7));
    assert_eq!(
        fs::read_to_string(root.join("src/Greeter.ts"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "// class FakeGreeter {}"
    );
    service
        .submit(request(root, "greet", Category::Actions))
        .unwrap();
    assert!(
        finish(&mut service).hits.is_empty(),
        "Actions are supplied by the GUI's real action registry"
    );
}
#[test]
fn include_non_project_enables_explicit_roots_and_dependencies_without_scanning_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    let extra = dir.path().join("explicit-library");
    let sibling = dir.path().join("unlisted-private");
    for path in [
        root.join("vendor"),
        root.join("node_modules"),
        extra.clone(),
        sibling.clone(),
    ] {
        fs::create_dir_all(path).unwrap();
    }
    for path in [
        root.join("vendor/Library.ts"),
        root.join("node_modules/Library.ts"),
        extra.join("Library.ts"),
        sibling.join("Library.ts"),
    ] {
        fs::write(path, "export class Library {}\n").unwrap();
    }
    let mut service = SearchEverywhereService::new();
    let mut req = request(&root, "Library", Category::Classes);
    req.external_roots.push(extra.clone());
    service.submit(req.clone()).unwrap();
    assert!(finish(&mut service).hits.is_empty());
    req.include_non_project = true;
    service.submit(req).unwrap();
    let found = finish(&mut service);
    assert_eq!(found.hits.len(), 3);
    assert!(!found.hits.iter().any(|h| h.path.starts_with(&sibling)));
}
#[test]
fn newest_query_cancellation_unicode_and_filters_are_real() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("ação.ts"),
        "const 雨 = 'İ';\nconst texto = 'ALVO 😀';\n",
    )
    .unwrap();
    fs::write(root.join("ação.php"), "<?php function ação() {}\n").unwrap();
    let mut service = SearchEverywhereService::new();
    for query in ["old", "wrong", "ALVO"] {
        service
            .submit(request(root, query, Category::Text))
            .unwrap();
    }
    let found = finish(&mut service);
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].line, 1);
    assert_eq!(found.hits[0].column, 15);
    let mut req = request(root, "ação", Category::Files);
    req.filter = FileFilter::TypeScript;
    service.submit(req).unwrap();
    assert_eq!(finish(&mut service).hits.len(), 1);
    service
        .submit(request(root, "ação", Category::All))
        .unwrap();
    service.cancel();
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.pending() {
        service.poll();
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    assert!(service.result.is_none());
    assert!(service.error.is_none());
    assert!(
        service
            .submit(request(root, &"a".repeat(513), Category::Text))
            .is_err()
    );
}
#[cfg(unix)]
#[test]
fn symlink_escapes_and_binary_files_are_never_followed_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(dir.path().join("secret.ts"), "export class Secret {}\n").unwrap();
    std::os::unix::fs::symlink(dir.path().join("secret.ts"), root.join("escape.ts")).unwrap();
    fs::write(root.join("binary.bin"), b"Secret\0text").unwrap();
    let mut service = SearchEverywhereService::new();
    service
        .submit(request(&root, "Secret", Category::All))
        .unwrap();
    assert!(finish(&mut service).hits.is_empty());
}

#[test]
fn contributors_preview_and_visible_limits_do_not_claim_complete_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join("Greeter.ts"),
        "export class Greeter {\n  greet() {}\n}\n",
    )
    .unwrap();
    let mut service = SearchEverywhereService::new();
    let mut req = request(root, "greet", Category::All);
    req.contributors = CLASSES;
    req.preview = true;
    service.submit(req.clone()).unwrap();
    let result = finish(&mut service);
    assert_eq!(result.hits.len(), 1);
    assert_eq!(result.hits[0].kind, HitKind::Class);
    assert!(
        result.hits[0]
            .preview
            .as_ref()
            .unwrap()
            .text
            .contains("export class Greeter")
    );
    req.contributors = 0;
    service.submit(req).unwrap();
    assert!(finish(&mut service).hits.is_empty());
    for i in 0..240 {
        fs::write(root.join(format!("bounded-{i}.txt")), "small\n").unwrap();
    }
    let mut req = request(root, "bounded", Category::Files);
    req.refresh = 2;
    service.submit(req).unwrap();
    let result = finish(&mut service);
    assert_eq!(result.hits.len(), MAX_RESULTS);
    assert!(result.truncated);
    fs::write(root.join("large.txt"), "x".repeat(1024 * 1024 + 1)).unwrap();
    let mut req = request(root, "absent", Category::Text);
    req.refresh = 3;
    service.submit(req).unwrap();
    assert!(finish(&mut service).truncated);
}

#[test]
fn text_search_decodes_unopened_microservice_files_with_independent_charsets() {
    let dir = tempfile::tempdir().unwrap();
    for (name, charset, bytes) in [
        ("utf8", "utf-8", "<?php // ação".as_bytes()),
        ("latin1", "latin1", b"<?php // a\xe7\xe3o".as_slice()),
        (
            "windows",
            "windows-1252",
            b"<?php // a\xe7\xe3o \x80".as_slice(),
        ),
    ] {
        let root = dir.path().join(name);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("composer.json"), "{}").unwrap();
        fs::write(
            root.join(".editorconfig"),
            format!("root=true\n[*]\ncharset={charset}\n"),
        )
        .unwrap();
        fs::write(root.join("a.php"), bytes).unwrap();
    }
    let mut service = SearchEverywhereService::new();
    service
        .submit(request(dir.path(), "ação", Category::Text))
        .unwrap();
    let result = finish(&mut service);
    assert_eq!(
        result
            .hits
            .iter()
            .filter(|h| h.kind == HitKind::Text)
            .count(),
        3
    );
    assert!(!result.truncated);
}

#[test]
fn selected_preview_preserves_complete_source_and_resolves_project_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join("long.ts");
    let text = format!(
        "// {} 😀\r\n{}// final line without newline",
        "x".repeat(240),
        "export const value = 'ação';\r\n".repeat(45)
    );
    fs::write(&path, &text).unwrap();
    let loaded = load_preview(&path, Some(root), None, &|| false).unwrap();
    assert_eq!(loaded.first_line, 0);
    assert_eq!(loaded.text, text);

    let legacy = root.join("legacy.php");
    fs::write(&legacy, b"<?php // a\xe7\xe3o \x80\r\n").unwrap();
    fs::write(
        root.join(".editorconfig"),
        "root=true\n[*]\ncharset=latin1\n",
    )
    .unwrap();
    let loaded = load_preview(&legacy, Some(root), None, &|| false).unwrap();
    assert_eq!(loaded.text, "<?php // ação \u{80}\r\n");
    let profile = root.join("encodings.json");
    let projects = std::collections::BTreeMap::from([(root.to_path_buf(), "Windows1252")]);
    fs::write(
        &profile,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": 1,
            "projects": projects
        }))
        .unwrap(),
    )
    .unwrap();
    let loaded = load_preview(&legacy, Some(root), Some(&profile), &|| false).unwrap();
    assert_eq!(loaded.text, "<?php // ação €\r\n");
}

#[test]
fn text_queries_preserve_literal_leading_trailing_and_only_whitespace() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("source.ts"), "prefix target suffix\n  indented\n").unwrap();
    let mut service = SearchEverywhereService::new();
    service
        .submit(request(root, " target ", Category::Text))
        .unwrap();
    let found = finish(&mut service);
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].line, 0);
    assert_eq!(found.hits[0].column, 6);
    service.submit(request(root, "  ", Category::Text)).unwrap();
    let found = finish(&mut service);
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].line, 1);
    assert_eq!(found.hits[0].column, 0);
}

#[test]
fn decoded_source_limit_marks_results_partial_instead_of_silently_omitting_legacy_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(
        root.join(".editorconfig"),
        "root=true\n[*]\ncharset=latin1\n",
    )
    .unwrap();
    let path = root.join("legacy.txt");
    fs::write(&path, vec![0xe9; 600_000]).unwrap();
    let mut service = SearchEverywhereService::new();
    service.submit(request(root, "é", Category::Text)).unwrap();
    let found = finish(&mut service);
    assert!(found.hits.is_empty());
    assert!(found.truncated);
    assert!(
        load_preview(&path, Some(root), None, &|| false)
            .unwrap_err()
            .contains("Decoded source exceeds 1 MiB")
    );
}

#[test]
fn symbol_source_extensions_match_the_case_insensitive_file_filter() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("Greeter.TS"), "export class Greeter {}\n").unwrap();
    let mut service = SearchEverywhereService::new();
    let mut query = request(root, "Greeter", Category::Classes);
    query.filter = FileFilter::TypeScript;
    service.submit(query).unwrap();
    let found = finish(&mut service);
    assert_eq!(found.hits.len(), 1);
    assert_eq!(found.hits[0].title, "Greeter");
    assert!(!found.truncated);
}

#[test]
fn selected_preview_rejects_unreadable_binary_oversize_and_cancelled_source() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(load_preview(&root.join("missing.ts"), Some(root), None, &|| false).is_err());
    assert!(load_preview(root, Some(root), None, &|| false).is_err());
    let binary = root.join("binary.ts");
    fs::write(&binary, b"text\0hidden").unwrap();
    assert!(load_preview(&binary, Some(root), None, &|| false).is_err());
    let invalid = root.join("invalid.ts");
    fs::write(&invalid, b"\xff").unwrap();
    assert!(load_preview(&invalid, Some(root), None, &|| false).is_err());
    let large = root.join("large.ts");
    fs::write(&large, "x".repeat(1024 * 1024 + 1)).unwrap();
    assert!(load_preview(&large, Some(root), None, &|| false).is_err());
    let path = root.join("source.ts");
    fs::write(&path, "x".repeat(24 * 1024)).unwrap();
    assert!(
        load_preview(&path, Some(root), None, &|| true)
            .unwrap_err()
            .contains("cancelled")
    );
    let checks = std::cell::Cell::new(0);
    let result = load_preview(&path, Some(root), None, &|| {
        checks.set(checks.get() + 1);
        checks.get() > 2
    });
    assert!(result.unwrap_err().contains("cancelled"));
    #[cfg(unix)]
    {
        let symlink = root.join("symlink.ts");
        std::os::unix::fs::symlink(&path, &symlink).unwrap();
        assert!(load_preview(&symlink, Some(root), None, &|| false).is_err());
    }
}
