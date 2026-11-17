use codisseu_core::{AppService, Command, Coverage};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
fn idle(app: &mut AppService) {
    let end = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll();
        if !app.is_busy() {
            return;
        }
        assert!(
            Instant::now() < end,
            "{} {:?}",
            app.status,
            app.rename.error
        );
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn php_trait_source_and_alias_rename_complete_without_provider_fabrication() {
    for (alias, external) in [(false, false), (true, false), (false, true), (true, true)] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let trait_path = root.join("Rain.php");
        let class_path = root.join("Weather.php");
        let main = root.join("main.php");
        let trait_text = "<?php trait Rain {public function rainfall(): int {return 1;}}\n";
        let class_text = "<?php class Weather {use Rain {Rain::rainfall as drizzle;}}\n";
        let main_text =
            "<?php function sample(Weather $w): int {return $w->rainfall()+$w->drizzle();}\n";
        for (path, text) in [
            (&trait_path, trait_text),
            (&class_path, class_text),
            (&main, main_text),
        ] {
            fs::write(path, text).unwrap();
        }
        let source = if external {
            main.clone()
        } else if alias {
            class_path.clone()
        } else {
            trait_path.clone()
        };
        let old = if alias { "drizzle" } else { "rainfall" };
        let mut app = AppService::new(root.join("session.json"));
        app.dispatch(Command::OpenWorkspace(root.clone())).unwrap();
        idle(&mut app);
        app.dispatch(Command::OpenFile(source.clone())).unwrap();
        idle(&mut app);
        let doc = app.active_document().unwrap();
        let id = doc.id;
        let dirty = format!("{}// dirty 😀\n", doc.text());
        app.dispatch(Command::Edit {
            id,
            expected_version: doc.version,
            text: dirty.clone(),
        })
        .unwrap();
        let doc = app.active_document().unwrap();
        let character_index = doc.text().find(old).unwrap() + 1;
        app.dispatch(Command::PrepareRename {
            id,
            expected_version: doc.version,
            character_index,
        })
        .unwrap();
        idle(&mut app);
        assert!(
            app.rename.is_prepared(),
            "{:?} {}",
            app.rename.error,
            app.status
        );
        app.dispatch(Command::PreviewRename {
            new_name: "forecast".into(),
        })
        .unwrap();
        idle(&mut app);
        assert!(
            app.rename.can_apply(),
            "{:?} {:?}",
            app.rename.error,
            app.rename.validation
        );
        assert_eq!(
            app.rename.validation.as_ref().unwrap().coverage,
            Coverage::Verified
        );
        let expected = app
            .rename
            .files
            .iter()
            .map(|f| (f.path.clone(), f.after.clone()))
            .collect::<Vec<_>>();
        assert!(expected.iter().any(|(p, t)| p == &main
            && t.contains(if alias {
                "$w->rainfall()+$w->forecast()"
            } else {
                "$w->forecast()+$w->drizzle()"
            })));
        assert!(expected.iter().any(|(p, t)| p == &class_path
            && t.contains(if alias {
                "Rain::rainfall as forecast"
            } else {
                "Rain::forecast as drizzle"
            })));
        app.dispatch(Command::ApplyRename).unwrap();
        idle(&mut app);
        assert!(app.rename.error.is_none(), "{:?}", app.rename.error);
        for (path, text) in &expected {
            app.dispatch(Command::OpenFile(path.clone())).unwrap();
            idle(&mut app);
            assert_eq!(app.active_document().unwrap().text(), text.as_ref());
        }
        app.dispatch(Command::Activate { id }).unwrap();
        app.dispatch(Command::Undo { id }).unwrap();
        idle(&mut app);
        assert_eq!(app.active_document().unwrap().text(), dirty);
        app.dispatch(Command::Redo { id }).unwrap();
        idle(&mut app);
        assert!(app.active_document().unwrap().text().contains("forecast"));
        assert_eq!(fs::read_to_string(trait_path).unwrap(), trait_text);
        assert_eq!(fs::read_to_string(class_path).unwrap(), class_text);
        assert_eq!(fs::read_to_string(main).unwrap(), main_text);
    }
}
