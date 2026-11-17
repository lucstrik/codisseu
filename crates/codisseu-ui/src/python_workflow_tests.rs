//! Owned Python projects, actual Basedpyright/Ruff, shared widgets and WGPU.
//! No user workspace, project execution, package install or synthetic semantic data.
use super::*;
use codisseu_core::{Language, ProviderConfig, ProviderState, Selection};
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{fs, path::Path, thread, time::Instant};
const SOURCE: &str = "# ação 😀\r\nimport sys\r\nimport os\r\n\r\ndef combine(first:int,second:int)->int:\r\n  return first+second\r\n\r\ndef run()->int:\r\n  emoji='😀';result=combine(40,2)\r\n  return result\r\n";
fn wait(h: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < until,
            "status={} error={:?} actions={:?} format={:?} rename={:?} hints={:?}",
            h.state().service.status,
            h.state().service.last_error,
            h.state().service.code_actions.error,
            h.state().service.formatting.error,
            h.state().service.rename.error,
            h.state().service.inlay_hints.error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn idle(h: &mut Harness<'_, CodisseuApp>) {
    wait(h, |a| !a.service.is_busy());
}
fn key(h: &mut Harness<'_, CodisseuApp>, m: Modifiers, k: Key) {
    h.key_press_modifiers(m, k);
    h.step();
    h.step();
}
fn source(h: &Harness<'_, CodisseuApp>) -> String {
    h.state().service.active_document().unwrap().text().into()
}
fn select(h: &mut Harness<'_, CodisseuApp>, selection: Selection) {
    let d = h.state().service.active_document().unwrap();
    let cmd = Command::SetSelection {
        id: d.id,
        expected_version: d.version,
        selection,
    };
    h.state_mut().service.dispatch(cmd).unwrap();
    h.get_by_role(Role::MultilineTextInput).focus();
    h.step();
    h.step();
}
fn at(h: &mut Harness<'_, CodisseuApp>, needle: &str, offset: usize) {
    let text = source(h);
    let caret = text[..text.find(needle).unwrap()].chars().count() + offset;
    select(h, Selection::caret(caret));
}
fn open(h: &mut Harness<'_, CodisseuApp>, path: &Path) {
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(path.into()))
        .unwrap();
    idle(h);
}
fn capture(h: &mut Harness<'_, CodisseuApp>, scale: f32, name: &str) {
    if std::env::var_os("CODISSEU_PYTHON_CAPTURES").is_none() {
        return;
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/python-integration-20261102/ui")
        .join(if scale == 1. { "100" } else { "125" });
    fs::create_dir_all(&dir).unwrap();
    h.render()
        .unwrap()
        .save(dir.join(format!("{name}.png")))
        .unwrap();
}
fn setup(scale: f32) -> (tempfile::TempDir, Harness<'static, CodisseuApp>, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Python ação");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("main.py"), SOURCE).unwrap();
    fs::write(root.join("models.py"), "class WeatherStation:\n    pass\n").unwrap();
    fs::write(
        root.join("consumer.py"),
        "from main import combine\r\nanswer = combine(20, 22)\r\n",
    )
    .unwrap();
    fs::write(root.join("pyproject.toml"),"[tool.ruff]\nline-length = 88\n[tool.ruff.lint]\nselect = [\"F401\", \"I\"]\n[tool.basedpyright]\ntypeCheckingMode = \"standard\"\n").unwrap();
    let root = root.canonicalize().unwrap();
    let mut service = AppService::new(dir.path().join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(scale)
        .with_max_steps(180)
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    idle(&mut h);
    open(&mut h, &root.join("main.py"));
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.tools");
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(vec![ProviderConfig {
            language: Language::Python,
            program: tools
                .join("node-v24.22.0-linux-x64/bin/node")
                .canonicalize()
                .unwrap(),
            args: vec![
                tools
                    .join("python/node_modules/basedpyright/langserver.index.js")
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into(),
                "--stdio".into(),
            ],
        }]))
        .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::StartProviders)
        .unwrap();
    wait(&mut h, |a| {
        !a.service.is_busy()
            && a.service
                .language
                .statuses
                .iter()
                .any(|p| p.language == Language::Python && p.state == ProviderState::Ready)
    });
    (dir, h, root)
}
fn stop(h: &mut Harness<'_, CodisseuApp>) {
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    idle(h);
    assert!(!h.state().terminal.has_sessions());
}
#[test]
#[ignore = "real private Python/Basedpyright/Ruff and WGPU, 100%/125%"]
fn python_hints_copy_pointer_ruff_format_undo_save_and_quickfix_first_click() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(scale);
        let path = root.join("main.py");
        wait(&mut h, |a| {
            a.editor_metrics().inlay_hints > 0
                && a.service
                    .inlay_hints
                    .items
                    .iter()
                    .any(|i| i.kind == Some(1))
                && a.service
                    .inlay_hints
                    .items
                    .iter()
                    .any(|i| i.kind == Some(2))
        });
        assert_eq!(source(&h), SOURCE);
        capture(&mut h, scale, "python-real-hints");
        let caret = SOURCE[..SOURCE.find("combine(40,2)").unwrap() + "combine(40,".len()]
            .chars()
            .count();
        select(&mut h, Selection::caret(caret));
        let point = h.state().caret_screen.unwrap()
            - egui::vec2(0., h.state().tokens.editor_line_height / 2.);
        select(&mut h, Selection::caret(caret - 1));
        h.hover_at(point);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        h.step();
        h.step();
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            Selection::caret(caret)
        );
        select(
            &mut h,
            Selection {
                anchor: 0,
                head: SOURCE.chars().count(),
            },
        );
        h.event(egui::Event::Copy);
        h.step();
        assert!(
            h.output()
                .platform_output
                .commands
                .iter()
                .any(|c| matches!(c,egui::OutputCommand::CopyText(t) if t==SOURCE))
        );
        assert_eq!(source(&h), SOURCE);
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        select(&mut h, Selection::caret(caret));
        capture(&mut h, scale, "python-hints-source-caret");
        key(&mut h, Modifiers::CTRL.plus(Modifiers::ALT), Key::L);
        wait(&mut h, |a| !a.service.formatting.is_pending());
        let formatted = source(&h);
        assert_ne!(formatted, SOURCE);
        assert!(formatted.contains("def combine(first: int, second: int) -> int:"));
        assert!(formatted.contains("# ação 😀\r\n"));
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        capture(&mut h, scale, "python-ruff-formatted");
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(source(&h), SOURCE);
        key(&mut h, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        assert_eq!(source(&h), formatted);
        key(&mut h, Modifiers::COMMAND, Key::S);
        idle(&mut h);
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        at(&mut h, "import os", 8);
        key(&mut h, Modifiers::ALT, Key::Enter);
        wait(&mut h, |a| {
            a.service.code_actions.has_preview() && !a.service.code_actions.is_pending()
        });
        assert!(
            h.state()
                .service
                .code_actions
                .items
                .iter()
                .any(|i| i.title.contains("Ruff"))
        );
        assert_eq!(source(&h), formatted);
        assert!(
            h.state()
                .service
                .code_actions
                .files
                .iter()
                .any(|f| f.before.contains("import os") && !f.after.contains("import os"))
        );
        assert!(h.query_by_label("Prévia da correção").is_some());
        let button = h.get_by_label("Aplicar correção").rect();
        h.get_by_label("Aplicar correção").hover();
        h.step();
        assert_eq!(
            button,
            h.get_by_label("Aplicar correção").rect(),
            "quickfix footer moved under pointer"
        );
        capture(&mut h, scale, "python-ruff-fix-preview");
        h.get_by_label("Aplicar correção").click();
        wait(&mut h, |a| !a.service.code_actions.is_open());
        assert!(!source(&h).contains("import os"));
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        capture(&mut h, scale, "python-ruff-fix-applied");
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(source(&h), formatted);
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        at(&mut h, "import os", 8);
        key(&mut h, Modifiers::ALT, Key::Enter);
        wait(&mut h, |a| {
            a.service.code_actions.has_preview() && !a.service.code_actions.is_pending()
        });
        assert!(
            h.state()
                .service
                .code_actions
                .items
                .iter()
                .any(|i| i.title == "Ruff · Organizar imports")
        );
        key(&mut h, Modifiers::NONE, Key::ArrowDown);
        wait(&mut h, |a| {
            a.service.code_actions.has_preview()
                && !a.service.code_actions.is_pending()
                && a.service.code_actions.title.contains("Organizar imports")
        });
        let sorted = &h.state().service.code_actions.files[0].after;
        assert!(sorted.find("import os").unwrap() < sorted.find("import sys").unwrap());
        assert_eq!(source(&h), formatted);
        let button = h.get_by_label("Aplicar correção").rect();
        h.get_by_label("Aplicar correção").hover();
        h.step();
        assert_eq!(button, h.get_by_label("Aplicar correção").rect());
        capture(&mut h, scale, "python-organize-imports-preview");
        h.get_by_label("Aplicar correção").click();
        wait(&mut h, |a| !a.service.code_actions.is_open());
        let sorted = source(&h);
        assert!(sorted.find("import os").unwrap() < sorted.find("import sys").unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), formatted);
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(source(&h), formatted);
        stop(&mut h);
    }
}
#[test]
#[ignore = "real private Python/Basedpyright and WGPU, 100%/125%"]
fn python_completion_autoimport_hover_rename_widgets_and_original_snippets() {
    for scale in [1., 1.25] {
        let (_dir, mut h, root) = setup(scale);
        let path = root.join("main.py");
        at(&mut h, "combine(40,2)", 2);
        key(&mut h, Modifiers::COMMAND, Key::Q);
        wait(&mut h, |a| {
            a.service
                .language
                .hover
                .as_ref()
                .is_some_and(|r| r.text.contains("combine"))
        });
        assert!(h.query_by_label("Documentação do símbolo").is_some());
        capture(&mut h, scale, "python-hover");
        key(&mut h, Modifiers::NONE, Key::Escape);
        at(&mut h, "combine(first", 2);
        key(&mut h, Modifiers::SHIFT, Key::F6);
        wait(&mut h, |a| {
            a.service.rename.prepared || a.service.rename.error.is_some()
        });
        assert!(
            h.state().service.rename.prepared,
            "{:?}",
            h.state().service.rename.error
        );
        for _ in 0..3 {
            h.step();
        }
        key(&mut h, Modifiers::SHIFT, Key::F6);
        h.get_by_label("Renomear símbolo e seus usos para:").focus();
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.event(egui::Event::Text("precipitação".into()));
        h.step();
        h.step();
        h.get_by_label("Preview").click();
        wait(&mut h, |a| {
            !a.service.rename.is_pending() && a.service.rename.validation.is_some()
        });
        assert!(
            h.state().service.rename.can_apply(),
            "{:?} {:?}",
            h.state().service.rename.error,
            h.state().service.rename.validation
        );
        assert_eq!(
            h.state().service.rename.validation.as_ref().unwrap().scope,
            codisseu_core::ValidationScope::PythonAnalyzerBindings
        );
        capture(&mut h, scale, "python-rename-preview");
        h.get_by_label("Refatorar").click();
        idle(&mut h);
        assert!(source(&h).contains("def precipitação("));
        assert_eq!(fs::read_to_string(&path).unwrap(), SOURCE);
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(source(&h), SOURCE);
        let completion = root.join("completion.py");
        let before = "# 😀\r\nstation = WeatherSta\r\n";
        fs::write(&completion, before).unwrap();
        open(&mut h, &completion);
        at(&mut h, "WeatherSta", 10);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            key(&mut h, Modifiers::CTRL, Key::Space);
            wait(&mut h, |a| !a.service.completion.is_pending());
            if h.state()
                .service
                .completion
                .items
                .iter()
                .any(|i| i.label == "WeatherStation")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{:?}",
                h.state().service.completion.error
            );
            thread::sleep(Duration::from_millis(100));
        }
        capture(&mut h, scale, "python-completion-autoimport");
        h.get_by_label("WeatherStation").click();
        idle(&mut h);
        let completed = source(&h);
        assert!(completed.contains("from models import WeatherStation"));
        assert!(completed.contains("station = WeatherStation"));
        assert!(completed.contains("# 😀\r\n"));
        assert_eq!(fs::read_to_string(&completion).unwrap(), before);
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(source(&h), before);
        let snippet = root.join("snippet.py");
        let before = "# snippet 😀\r\ndef";
        fs::write(&snippet, before).unwrap();
        open(&mut h, &snippet);
        select(&mut h, Selection::caret(before.chars().count()));
        key(&mut h, Modifiers::NONE, Key::Tab);
        assert!(source(&h).contains("def name():\r\n    "));
        h.event(egui::Event::Text("forecast".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Tab);
        h.event(egui::Event::Text("value: int".into()));
        h.step();
        key(&mut h, Modifiers::NONE, Key::Tab);
        h.event(egui::Event::Text("return value".into()));
        h.step();
        h.step();
        assert!(source(&h).contains("def forecast(value: int):\r\n    return value"));
        assert_eq!(fs::read_to_string(&snippet).unwrap(), before);
        capture(&mut h, scale, "python-original-snippet");
        stop(&mut h);
    }
}
