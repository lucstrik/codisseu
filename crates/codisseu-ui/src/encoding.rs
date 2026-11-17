use super::*;
use codisseu_core::FileEncoding;

impl CodisseuApp {
    pub(super) fn encoding_status(&mut self, ui: &mut egui::Ui) {
        let encoding = self
            .service
            .active_document()
            .map_or(self.service.project_encoding(), |d| d.encoding());
        let root = self.service.encoding_project_root().map(PathBuf::from);
        ui.add_enabled_ui(root.is_some() && !self.service.encoding_pending(), |ui| {
            let response = ui.add(egui::Button::new(encoding.label()).frame(false));
            egui::Popup::menu(&response).show(|ui| {
                let mut folders: Vec<_> = self.service.project_roots().cloned().collect();
        folders.extend(
            self.service
                .entries
                .iter()
                .filter(|e| {
                    e.depth == 0
                        && e.is_dir
                        && e.path.file_name().is_some_and(|n| {
                            !matches!(
                                n.to_str(),
                                Some(".git" | "vendor" | "node_modules" | "target")
                            )
                        })
                })
                .map(|e| e.path.clone()),
        );
        folders.sort();
        folders.dedup();
        folders.truncate(512);
                ui.set_min_width(280.);
                ui.label("Codificação do projeto");
                if let Some(root) = &root {
                    ui.add(egui::Label::new(root.display().to_string()).wrap());
                }
                ui.separator();
                ui.label("Recarrega somente os arquivos deste projeto.");
                let current = self.service.project_encoding();
                for choice in FileEncoding::ALL {
                    if ui.selectable_label(choice == current, choice.label()).clicked() {
                        self.send(Command::SetProjectEncoding(choice));
                        ui.close();
                    }
                }
                if folders.len() > 1 {
                    ui.separator();
                    ui.menu_button("Codificação de outra pasta…", |ui| {
                        egui::ScrollArea::vertical().max_height(360.).codisseu().show(ui, |ui| {
                            for folder in folders {
                                let name = self.service.workspace.as_ref().and_then(|w| folder.strip_prefix(w).ok()).filter(|p| !p.as_os_str().is_empty()).map_or_else(|| folder.file_name().unwrap_or_default().to_string_lossy().into_owned(), |p| p.display().to_string());
                                ui.menu_button(name, |ui| {
                                    ui.add(egui::Label::new(folder.display().to_string()).wrap());
                                    ui.separator();
                                    for choice in FileEncoding::ALL {
                                        if ui.button(choice.label()).clicked() {
                                            self.send(Command::SetFolderEncoding { root: folder.clone(), encoding: choice });
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        });
                    });
                }
                ui.separator();
                ui.label("A escolha será lembrada para esta pasta.");
            });
            response.on_hover_text("Escolher a codificação deste projeto. Os outros projetos mantêm suas configurações.");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{
        fs,
        time::{Duration, Instant},
    };
    fn settle(app: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(10);
        while app.is_busy() {
            app.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        app.poll();
    }
    #[test]
    fn status_menu_changes_project_encoding_and_reloads_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("accent.php");
        fs::write(&path, "<?php // ação\r\n").unwrap();
        let mut service = AppService::new(dir.path().join("profile/session.json"));
        service
            .dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        settle(&mut service);
        service.dispatch(Command::OpenFile(path.clone())).unwrap();
        settle(&mut service);
        let original = fs::read(&path).unwrap();
        let mut h = Harness::builder().with_size([640., 160.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                app.service.poll();
                app.encoding_status(ui);
            },
            CodisseuApp::new(service),
        );
        h.run();
        h.get_by_label("UTF-8").click();
        h.run();
        h.get_by_label("ISO-8859-1").click();
        h.run();
        settle(&mut h.state_mut().service);
        h.run();
        assert_eq!(h.state().service.project_encoding(), FileEncoding::Latin1);
        assert_eq!(
            h.state().service.active_document().unwrap().encoding(),
            FileEncoding::Latin1
        );
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(!h.state().service.active_document().unwrap().is_dirty());
        h.get_by_label("ISO-8859-1").click();
        h.run();
        h.get_by_label("Windows-1252").click();
        h.run();
        settle(&mut h.state_mut().service);
        h.run();
        assert_eq!(
            h.state().service.project_encoding(),
            FileEncoding::Windows1252
        );
    }
    #[test]
    fn footer_encoding_menu_names_and_reloads_only_active_microservice() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("workspace");
        let first = root.join("utf8");
        let second = root.join("legacy");
        for project in [&first, &second] {
            fs::create_dir_all(project).unwrap();
            fs::write(project.join("composer.json"), "{}").unwrap();
        }
        fs::write(first.join("a.php"), "<?php // ação 😀").unwrap();
        fs::write(second.join("b.php"), b"<?php // \xe9").unwrap();
        fs::write(
            second.join(".editorconfig"),
            "root=true\n[*]\ncharset=latin1\n",
        )
        .unwrap();
        let mut service = AppService::new(dir.path().join("profile/session.json"));
        service.dispatch(Command::OpenWorkspace(root)).unwrap();
        settle(&mut service);
        service
            .dispatch(Command::OpenFile(first.join("a.php")))
            .unwrap();
        settle(&mut service);
        let id = service.documents[0].id;
        let expected_version = service.documents[0].version;
        service
            .dispatch(Command::Edit {
                id,
                expected_version,
                text: "<?php // alteração pendente 😀".into(),
            })
            .unwrap();
        service
            .dispatch(Command::OpenFile(second.join("b.php")))
            .unwrap();
        settle(&mut service);
        let version = service.documents[0].version;
        let mut h = Harness::builder().with_size([800., 240.]).build_ui_state(
            |ui, app: &mut CodisseuApp| {
                app.service.poll();
                app.encoding_status(ui);
            },
            CodisseuApp::new(service),
        );
        h.run();
        h.get_by_label("ISO-8859-1").click();
        h.run();
        assert!(h.query_by_label(&second.display().to_string()).is_some());
        h.get_by_label("Windows-1252").click();
        h.run();
        settle(&mut h.state_mut().service);
        h.run();
        let service = &h.state().service;
        assert_eq!(service.documents[0].version, version);
        assert!(service.documents[0].is_dirty());
        assert_eq!(service.documents[0].encoding(), FileEncoding::Utf8);
        assert_eq!(service.documents[1].encoding(), FileEncoding::Windows1252);
        assert_eq!(fs::read(second.join("b.php")).unwrap(), b"<?php // \xe9");
    }
}
