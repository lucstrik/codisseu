use super::*;
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, thread, time::Duration};

fn parsed(text: &str) -> parse::Parsed {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    fs::write(&path, text).unwrap();
    parse::parse(text, &path, Some(dir.path()), &AtomicBool::new(false)).unwrap()
}
#[test]
fn commonmark_gfm_retains_nested_structure_unicode_links_and_task_source_offsets() {
    let text = "# Olá 😀\n\n# Olá 😀\n\nA **strong _nested_** and `code` ~~gone~~ [link](#olá-)\n\n1. Parent\n   - Child\n\n- [ ] Incomplete\n- [x] Complete\n\n| A | B |\n|---|---:|\n| α | **β** |\n\n```ts\nconst x = 1;\n```\n\n> Quote\n\n---\n";
    let model = parsed(text);
    let headings: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|b| {
            if let parse::BlockKind::Rich {
                anchor: Some(a), ..
            } = &b.kind
            {
                Some(a.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(headings, ["olá-", "olá--1"]);
    assert!(model.blocks.iter().any(|b| b.indent == 1));
    assert!(model.blocks.iter().any(|b| b.quote == 1));
    let tasks: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|b| {
            if let parse::BlockKind::Rich { task: Some(t), .. } = &b.kind {
                Some(t)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(tasks.len(), 2);
    assert_eq!(text.as_bytes()[tasks[0].byte_offset], b' ');
    assert_eq!(text.as_bytes()[tasks[1].byte_offset], b'x');
    assert!(model.blocks.iter().any(
        |b| matches!(&b.kind,parse::BlockKind::Table{rows,align}if rows.len()==2&&align.len()==2)
    ));
    assert!(model.blocks.iter().any(|b|matches!(&b.kind,parse::BlockKind::Code{language,text,job}if language=="ts"&&text=="const x = 1;\n"&&job.sections.len()>1)));
    assert!(
        model
            .blocks
            .iter()
            .any(|b| matches!(b.kind, parse::BlockKind::Rule))
    );
    let spans: Vec<_> = model
        .blocks
        .iter()
        .flat_map(|b| {
            if let parse::BlockKind::Rich { spans, .. } = &b.kind {
                spans.as_slice()
            } else {
                &[]
            }
        })
        .collect();
    assert!(
        spans
            .iter()
            .any(|s| s.text == "nested" && s.style.strong && s.style.emphasis)
    );
    assert!(spans.iter().any(|s| s.text == "code" && s.style.code));
    assert!(spans.iter().any(|s| s.style.strike));
    assert!(
        spans
            .iter()
            .any(|s| matches!(s.link.as_deref(),Some(parse::LinkTarget::Anchor(a))if a=="olá-"))
    );
}
#[test]
fn source_lines_follow_the_rope_for_crlf_cr_and_unicode_breaks_and_anchors_stay_unique() {
    let text = "# foo\r\n\r\n# foo\r\n\r\n# foo-1\r\n\r\nParagraph\u{2028}Unicode line\r\r# Last\r";
    let model = parsed(text);
    let snapshot = codisseu_core::TextSnapshot::from_str(text);
    let anchors: Vec<_> = model
        .blocks
        .iter()
        .filter_map(|block| {
            if let parse::BlockKind::Rich {
                anchor: Some(anchor),
                ..
            } = &block.kind
            {
                Some(anchor.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(anchors, ["foo", "foo-1", "foo-1-1", "last"]);
    for block in &model.blocks {
        assert_eq!(block.line, snapshot.byte_to_line(block.range.start));
    }
    let lists = parsed(
        "- tight one\n- tight two\n\nParagraph separates lists\n\n- loose one\n\n  continuation\n",
    );
    assert!(lists.blocks.iter().any(|block| matches!(
        &block.kind,
        parse::BlockKind::Rich {
            loose: false,
            marker: Some(_),
            ..
        }
    )));
    assert!(
        lists
            .blocks
            .iter()
            .any(|block| matches!(&block.kind, parse::BlockKind::Rich { loose: true, .. }))
    );
}
#[test]
fn local_links_cannot_escape_workspace_or_run_schemes_and_remote_images_are_never_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    let path = project.join("README.md");
    fs::write(&path, "").unwrap();
    fs::write(project.join("other file.md"), "# Heading").unwrap();
    fs::write(dir.path().join("secret.txt"), "outside").unwrap();
    assert!(
        matches!(parse::resolve_link("other%20file.md#heading",&path,Some(&project)),parse::LinkTarget::Local{anchor:Some(anchor),..}if anchor=="heading")
    );
    for url in [
        "../secret.txt",
        "%2e%2e/secret.txt",
        "javascript:alert(1)",
        "command:workbench.action.terminal.new",
        "file:///etc/passwd",
        "//example.invalid/pixel.png",
        "other%00file.md",
    ] {
        assert!(
            matches!(
                parse::resolve_link(url, &path, Some(&project)),
                parse::LinkTarget::Blocked(_)
            ),
            "{url}"
        );
    }
    let model = parse::parse(
        "![tracking](https://127.0.0.1:9/never.png)\n<script>alert('no execution')</script>",
        &path,
        Some(&project),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(
        model.images["https://127.0.0.1:9/never.png"]
            .as_ref()
            .unwrap_err()
            .contains("never fetches")
    );
    assert!(model.blocks.iter().any(|b|matches!(&b.kind,parse::BlockKind::Rich{spans,..}if spans.iter().any(|s|s.text.contains("<script>")))));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(project.join("other file.md"), project.join("alias.md"))
            .unwrap();
        assert!(matches!(
            parse::resolve_link("alias.md", &path, Some(&project)),
            parse::LinkTarget::Blocked(_)
        ));
    }
}
#[test]
fn parser_cancellation_size_depth_and_local_image_decoding_are_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    fs::write(&path, "").unwrap();
    assert!(
        parse::parse(
            &"x".repeat(parse::MAX_SOURCE + 1),
            &path,
            None,
            &AtomicBool::new(false)
        )
        .err()
        .unwrap()
        .contains("2 MiB")
    );
    assert!(
        parse::parse("# Heading", &path, None, &AtomicBool::new(true))
            .err()
            .unwrap()
            .contains("cancelled")
    );
    assert!(
        parse::parse(
            &("> ".repeat(70) + "nested"),
            &path,
            None,
            &AtomicBool::new(false)
        )
        .is_err()
    );
    image::RgbaImage::from_pixel(3, 2, image::Rgba([12, 34, 56, 255]))
        .save(dir.path().join("local.png"))
        .unwrap();
    let model = parse::parse(
        "![local evidence](local.png)",
        &path,
        Some(dir.path()),
        &AtomicBool::new(false),
    )
    .unwrap();
    let image = model.images["local.png"].as_ref().unwrap();
    assert_eq!(image.size, [3, 2]);
    assert_eq!(image.pixels[0], Color32::from_rgb(12, 34, 56));
}

struct PreviewState {
    markdown: MarkdownUi,
    service: AppService,
    tokens: Tokens,
    actions: Vec<MarkdownAction>,
}
fn preview_fixture(text: &str) -> (tempfile::TempDir, Harness<'static, PreviewState>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    fs::write(&path, text).unwrap();
    fs::write(dir.path().join("other.md"), "# Linked document\n").unwrap();
    image::RgbaImage::from_fn(160, 48, |x, y| {
        image::Rgba([40 + (x / 8) as u8, 80 + (y * 2) as u8, 140, 255])
    })
    .save(dir.path().join("evidence.png"))
    .unwrap();
    let mut service = AppService::new(dir.path().join("session.json"));
    service
        .dispatch(Command::OpenWorkspace(dir.path().into()))
        .unwrap();
    while service.is_busy() {
        service.poll();
        thread::sleep(Duration::from_millis(2));
    }
    service.dispatch(Command::OpenFile(path)).unwrap();
    while service.is_busy() {
        service.poll();
        thread::sleep(Duration::from_millis(2));
    }
    let mut h = Harness::builder()
        .with_size([980., 900.])
        .with_context_setup(|ctx| Tokens::load().apply(ctx))
        .build_ui_state(
            |ui, state: &mut PreviewState| {
                state.service.poll();
                let doc = &state.service.documents[0];
                state.markdown.toolbar(ui, &state.tokens, 1, &doc.path);
                state.actions.extend(state.markdown.preview(
                    ui,
                    &state.tokens,
                    1,
                    doc,
                    state.service.workspace.as_deref(),
                ));
            },
            PreviewState {
                markdown: MarkdownUi::default(),
                service,
                tokens: Tokens::load(),
                actions: Vec::new(),
            },
        );
    settle_preview(&mut h);
    (dir, h)
}
fn settle_preview(h: &mut Harness<'static, PreviewState>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        h.step();
        if h.state().markdown.documents.values().all(|s| !s.waiting) {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(3));
    }
    h.run();
}
#[test]
fn preview_uses_the_dirty_document_and_task_click_produces_versioned_edit_without_saving() {
    let (_dir, mut h) = preview_fixture("# Initial\n\n- [ ] Task\n\n[Heading](#initial)\n");
    let doc = &h.state().service.documents[0];
    let id = doc.id;
    let version = doc.version;
    let path = doc.path.clone();
    h.get_by_label("Markdown task line 3").click();
    h.run();
    let action = h.state_mut().actions.pop().unwrap();
    let MarkdownAction::ToggleTask {
        id: action_id,
        version: action_version,
        byte_offset,
        checked,
    } = action
    else {
        panic!("expected task edit")
    };
    assert_eq!((action_id, action_version, checked), (id, version, true));
    let doc = &h.state().service.documents[0];
    let char_offset = doc.snapshot().byte_to_char(byte_offset);
    h.state_mut()
        .service
        .dispatch(Command::EditDelta {
            id,
            expected_version: version,
            delta: codisseu_core::TextDelta {
                range: char_offset..char_offset + 1,
                insert: "x".into(),
            },
            kind: codisseu_core::EditKind::Atomic,
        })
        .unwrap();
    settle_preview(&mut h);
    assert!(h.state().service.documents[0].is_dirty());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# Initial\n\n- [ ] Task\n\n[Heading](#initial)\n"
    );
    let state = &h.state().markdown.documents[&id];
    assert_eq!(state.stamp.version, version + 1);
    assert!(
        state
            .parsed
            .as_ref()
            .unwrap()
            .blocks
            .iter()
            .any(|b| matches!(&b.kind,parse::BlockKind::Rich{task:Some(t),..}if t.checked))
    );
    h.state_mut()
        .service
        .dispatch(Command::Undo { id })
        .unwrap();
    settle_preview(&mut h);
    assert!(!h.state().service.documents[0].is_dirty());
}

#[test]
fn modes_are_per_view_and_path_bounded_and_never_start_a_worker_for_source_only() {
    let mut markdown = MarkdownUi::default();
    let a = Path::new("/fixture/a.md");
    let b = Path::new("/fixture/b.md");
    markdown.set_mode(1, a, MarkdownMode::Split);
    markdown.set_mode(1, b, MarkdownMode::Preview);
    assert_eq!(markdown.mode(1, a), MarkdownMode::Split);
    assert_eq!(markdown.mode(1, b), MarkdownMode::Preview);
    assert_eq!(markdown.mode(2, a), MarkdownMode::Source);
    markdown.set_mode(1, a, MarkdownMode::Split);
    markdown.set_source_line(1, 45);
    markdown.navigate_anchor(1, "a-heading".into());
    assert_eq!(markdown.views[&(1, a.to_path_buf())].source_line, Some(45));
    for view in 2..70 {
        markdown.set_mode(
            view,
            &PathBuf::from(format!("/fixture/{view}.md")),
            MarkdownMode::Source,
        );
    }
    assert!(markdown.views.len() <= 32);
    assert!(markdown.active_paths.len() <= 32);
    assert!(!markdown.worker.started);
}

#[test]
fn native_links_and_anchor_navigation_are_explicit_and_sync_to_the_source_line() {
    let text = format!(
        "# Start\n\n[Jump](#destination) · [Missing](#unknown) · [Local document](other.md#linked-document) · [Website](https://example.invalid/)\n\n{}\n# Destination\n\nAnchor evidence\n",
        (0..80)
            .map(|i| format!("Paragraph {i} with bounded content.\n\n"))
            .collect::<String>()
    );
    let (dir, mut h) = preview_fixture(&text);
    h.get_by_label("Local document").click();
    h.run();
    assert!(h.state().actions.iter().any(|a|matches!(a,MarkdownAction::OpenLocal{path,anchor:Some(anchor)}if path==&dir.path().join("other.md")&&anchor=="linked-document")));
    h.get_by_label("Website").click();
    h.run();
    assert!(
        h.state().actions.iter().any(
            |a| matches!(a,MarkdownAction::OpenExternal{url}if url=="https://example.invalid/")
        )
    );
    h.get_by_label("Missing").click();
    h.run();
    assert!(h.query_by_label("Heading #unknown was not found").is_none());
    assert_eq!(
        crate::status_messages::current(&h.ctx).as_deref(),
        Some("Heading #unknown was not found")
    );
    h.get_by_label("Jump").click();
    h.run();
    let line = text[..text.find("# Destination").unwrap()]
        .bytes()
        .filter(|b| *b == b'\n')
        .count();
    assert!(
        h.state()
            .actions
            .iter()
            .any(|a| matches!(a,MarkdownAction::ScrollSource{line:actual}if *actual==line))
    );
    assert!(
        h.state().markdown.views.values().any(|view| view
            .render
            .metrics()
            .1
            .is_some_and(|y| y > 1000.))
    );
    assert!(h.query_by_label("Anchor evidence").is_some());
    assert!(
        h.state()
            .markdown
            .views
            .values()
            .all(|view| view.notice.is_none())
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("README.md")).unwrap(),
        text
    );
}

#[test]
fn stale_replies_do_not_replace_current_dirty_preview_and_only_visible_blocks_build_layouts() {
    let text = (0..3000)
        .map(|i| format!("Paragraph {i}: Unicode α😀 remains source data.\n\n"))
        .collect::<String>();
    let (_dir, mut h) = preview_fixture(&text);
    let doc = &h.state().service.documents[0];
    let id = doc.id;
    let version = doc.version;
    assert!(
        h.state()
            .markdown
            .views
            .values()
            .all(|view| view.render.metrics().0 < 70)
    );
    let old_stamp = h.state().markdown.documents[&id].stamp.clone();
    h.state_mut()
        .service
        .dispatch(Command::Edit {
            id,
            expected_version: version,
            text: "# Current dirty heading\n\nStill unsaved.\n".into(),
        })
        .unwrap();
    h.step();
    h.state()
        .markdown
        .worker
        .mailbox
        .0
        .lock()
        .unwrap()
        .completed
        .push_back(Reply {
            stamp: old_stamp,
            result: Ok(parsed("# Obsolete reply")),
        });
    settle_preview(&mut h);
    assert!(h.query_by_label("Current dirty heading").is_some());
    assert!(h.query_by_label("Obsolete reply").is_none());
    let state = &h.state().markdown.documents[&id];
    assert_eq!(state.rendered_version, version + 1);
    assert!(!state.waiting);
    assert!(h.state().service.documents[0].is_dirty());
}

#[test]
fn image_formats_decode_locally_and_a_replaced_link_is_rejected_before_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    fs::write(&path, "").unwrap();
    let rgb = image::RgbImage::from_pixel(4, 3, image::Rgb([20, 80, 140]));
    for extension in ["png", "jpg", "gif", "webp"] {
        rgb.save(dir.path().join(format!("local.{extension}")))
            .unwrap();
    }
    let text = ["png", "jpg", "gif", "webp"]
        .map(|extension| format!("![{extension}](local.{extension})\n\n"))
        .concat();
    let model = parse::parse(&text, &path, Some(dir.path()), &AtomicBool::new(false)).unwrap();
    for extension in ["png", "jpg", "gif", "webp"] {
        assert_eq!(
            model.images[&format!("local.{extension}")]
                .as_ref()
                .unwrap()
                .size,
            [4, 3]
        );
    }
    let local = dir.path().join("local.png");
    assert_eq!(
        validate_local_path(&local, &path, Some(dir.path())).unwrap(),
        local
    );
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.png"), "outside").unwrap();
        fs::remove_file(&local).unwrap();
        std::os::unix::fs::symlink(outside.path().join("outside.png"), &local).unwrap();
        assert!(validate_local_path(&local, &path, Some(dir.path())).is_err());
    }
}

#[test]
fn rich_preview_toolbar_tasks_table_code_and_image_render_in_native_egui() {
    let text = "# Markdown no Codisseu 😀\r\n\r\nFonte e **preview** no mesmo documento, com _ênfase_, `código inline` e ~~riscado~~.\r\n\r\n## Trabalho diário\r\n\r\n- [ ] Revisar a implementação\r\n- [x] Preservar o documento\r\n\r\n1. Item ordenado\r\n   - Item aninhado\r\n\r\n| Recurso | Estado |\r\n|:---|---:|\r\n| Preview nativo | **Ativo** |\r\n| Histórico comum | Preservado |\r\n\r\n```ts\r\nexport const greeting = 'Olá, mundo';\r\n```\r\n\r\n> Citação renderizada sem HTML executável.\r\n\r\n![Imagem local](evidence.png)\r\n\r\n[Documento local](other.md)\r\n";
    let (_dir, mut h) = preview_fixture(text);
    for name in [
        "Markdown Source",
        "Markdown Split",
        "Markdown Preview",
        "Synchronize Markdown scrolling",
        "Zoom out Markdown preview",
        "Zoom in Markdown preview",
        "Copy code",
        "Markdown image Imagem local",
    ] {
        assert!(h.query_by_label(name).is_some(), "{name}");
    }
    h.get_by_label("Markdown Split").click();
    h.run();
    let path = h.state().service.documents[0].path.clone();
    assert_eq!(h.state().markdown.mode(1, &path), MarkdownMode::Split);
    assert!(h.query_by_label("Markdown task line 7").is_some());
    if let Some(path) = std::env::var_os("CODISSEU_MARKDOWN_CAPTURE_DIR") {
        let path = PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        h.render()
            .unwrap()
            .save(path.join("markdown-rich-preview.png"))
            .unwrap();
    }
    assert!(!h.state().service.documents[0].is_dirty());
}
