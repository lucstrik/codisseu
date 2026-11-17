use codisseu_core::{AppService, Command, Language, ProviderConfig, ProviderState, Selection};
use codisseu_ui::CodisseuApp;
use egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::{Harness, kittest::Queryable};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

fn wait(h: &mut Harness<'_, CodisseuApp>, predicate: impl Fn(&CodisseuApp) -> bool) {
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        h.step();
        if predicate(h.state()) {
            h.step();
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            h.state().service.status,
            h.state().service.last_error
        );
        thread::sleep(Duration::from_millis(5));
    }
}
fn key(h: &mut Harness<'_, CodisseuApp>, modifiers: Modifiers, key: Key) {
    h.key_press_modifiers(modifiers, key);
    wait(h, |a| !a.service.is_busy());
    assert!(
        h.state().service.last_error.is_none(),
        "{:?}",
        h.state().service.last_error
    );
}

#[test]
#[ignore = "real managed JavaScript/TypeScript provider; requires CODISSEU_PROVIDER_CONFIG"]
fn real_js_jsx_ts_tsx_shortcut_only_active_whole_dirty_file_undo_redo_and_explicit_save() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("tsconfig.json"),"{\"compilerOptions\":{\"allowJs\":true,\"jsx\":\"preserve\",\"target\":\"ES2022\"},\"include\":[\"*.js\",\"*.jsx\",\"*.ts\",\"*.tsx\"]}").unwrap();
    let other = root.join("other.js");
    fs::write(&other, "export const other={a:1};\n").unwrap();
    let mut service = AppService::new(root.join("profile/session.json"));
    service
        .dispatch(Command::OpenWorkspace(root.clone()))
        .unwrap();
    let mut h = Harness::builder()
        .with_size([1280., 800.])
        .with_pixels_per_point(
            std::env::var("CODISSEU_JS_FORMAT_SCALE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1.),
        )
        .build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
    wait(&mut h, |a| !a.service.is_busy());
    h.state_mut()
        .service
        .dispatch(Command::OpenFile(other.clone()))
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
    let d = h.state().service.active_document().unwrap();
    let other_id = d.id;
    let cmd = Command::Edit {
        id: d.id,
        expected_version: d.version,
        text: "export const other={a:99};\n".into(),
    };
    h.state_mut().service.dispatch(cmd).unwrap();
    let configs: Vec<ProviderConfig> = serde_json::from_slice(
        &fs::read(std::env::var_os("CODISSEU_PROVIDER_CONFIG").unwrap()).unwrap(),
    )
    .unwrap();
    h.state_mut()
        .service
        .dispatch(Command::ConfigureProviders(
            configs
                .into_iter()
                .filter(|c| c.language == Language::TypeScript)
                .collect(),
        ))
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
                .any(|s| s.state == ProviderState::Ready)
    });
    assert!(h.state().service.language.formatting_capabilities[&Language::TypeScript].document);
    for (name, source) in [
        (
            "active.js",
            "// ação 😀\r\nexport function greet(name){\r\nconst data={name,items:[1,2]};\r\nreturn `Hello ${name}`;\r\n}\r\nexport const sum=(a,b)=>{return a+b;};\r\n",
        ),
        (
            "active.jsx",
            "// ação 😀\r\nexport const view=(name)=>{\r\nreturn <section title=\"Hello\"><b>{name}</b></section>;\r\n};\r\nexport const sum=(a,b)=>{return a+b;};\r\n",
        ),
        (
            "typed.ts",
            "// ação 😀\r\ninterface User{name:string;id:number}\r\nexport function greet(user:User):string{\r\nconst items:Array<number>=[1,2];\r\nreturn `Hello ${user.name}`;\r\n}\r\nexport const sum=(a:number,b:number):number=>{return a+b;};\r\n",
        ),
        (
            "view.tsx",
            "// ação 😀\r\ntype Props={name:string};\r\nexport const view=({name}:Props)=>{\r\nreturn <section title=\"Hello\"><b>{name}</b></section>;\r\n};\r\nexport const sum=(a:number,b:number):number=>{return a+b;};\r\n",
        ),
    ] {
        let path = root.join(name);
        fs::write(&path, source).unwrap();
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path.clone()))
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        let dirty = source.replace("Hello", "Dirty");
        let d = h.state().service.active_document().unwrap();
        let id = d.id;
        let cmd = Command::Edit {
            id,
            expected_version: d.version,
            text: dirty.clone(),
        };
        h.state_mut().service.dispatch(cmd).unwrap();
        wait(&mut h, |a| !a.service.is_busy());
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        let d = h.state().service.active_document().unwrap();
        let before = Selection { anchor: 1, head: 0 };
        let cmd = Command::SetSelection {
            id,
            expected_version: d.version,
            selection: before,
        };
        h.state_mut().service.dispatch(cmd).unwrap();
        h.step();
        key(&mut h, Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
        let formatted = h
            .state()
            .service
            .active_document()
            .unwrap()
            .text()
            .to_owned();
        assert_ne!(formatted, dirty);
        assert!(formatted.contains("a + b"), "{formatted}");
        assert!(formatted.contains("Dirty"));
        assert!(formatted.contains("ação 😀"));
        assert!(
            !formatted.replace("\r\n", "").contains('\n'),
            "{formatted:?}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
        assert_eq!(
            h.state()
                .service
                .documents
                .iter()
                .find(|d| d.id == other_id)
                .unwrap()
                .text(),
            "export const other={a:99};\n"
        );
        assert_eq!(
            fs::read_to_string(&other).unwrap(),
            "export const other={a:1};\n"
        );
        if let Some(out) = std::env::var_os("CODISSEU_JS_FORMAT_CAPTURES") {
            fs::create_dir_all(&out).unwrap();
            h.render()
                .unwrap()
                .save(std::path::PathBuf::from(out).join(format!("formatted-{name}.png")))
                .unwrap();
        }
        key(&mut h, Modifiers::COMMAND, Key::Z);
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
        assert_eq!(
            h.state().service.active_document().unwrap().selection(),
            before
        );
        h.state_mut()
            .service
            .dispatch(Command::Save { id })
            .unwrap();
        wait(&mut h, |a| !a.service.is_busy());
        assert_eq!(fs::read_to_string(&path).unwrap(), dirty);
        assert_eq!(h.state().service.active_document().unwrap().text(), dirty);
        key(&mut h, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        assert_eq!(
            h.state().service.active_document().unwrap().text(),
            formatted
        );
    }
    h.state_mut()
        .service
        .dispatch(Command::StopProviders)
        .unwrap();
    wait(&mut h, |a| !a.service.is_busy());
    assert!(
        h.state()
            .service
            .language
            .statuses
            .iter()
            .all(|s| s.state == ProviderState::Stopped)
    );
}

#[test]
fn shortcut_without_provider_preserves_javascript_and_typescript_buffer_and_disk() {
    for extension in ["js", "ts"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(format!("standalone.{extension}"));
        let source = if extension == "ts" {
            "const data:{a:number}={a:1};\n"
        } else {
            "const data={a:1};\n"
        };
        fs::write(&path, source).unwrap();
        let mut service = AppService::new(temp.path().join("session.json"));
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        let mut h = Harness::builder().with_size([1280., 800.]).build_ui_state(
            |ui, a: &mut CodisseuApp| a.show(ui),
            CodisseuApp::new(service),
        );
        wait(&mut h, |a| !a.service.is_busy());
        h.get_by_role(Role::MultilineTextInput).focus();
        h.step();
        h.key_press_modifiers(Modifiers::COMMAND.plus(Modifiers::ALT), Key::L);
        h.step();
        h.step();
        assert!(h.state().service.formatting.error.is_some());
        assert_eq!(h.state().service.active_document().unwrap().text(), source);
        assert_eq!(fs::read_to_string(path).unwrap(), source);
    }
}
