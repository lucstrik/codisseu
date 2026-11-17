use super::*;
use codisseu_core::ProjectRuntime;

fn file_language(path: &std::path::Path) -> (String, Option<ProjectRuntime>) {
    let key = highlight::syntax_key(path);
    let (name, runtime) = match key.as_str() {
        "php" | "phtml" | "php3" | "php4" | "php5" | "php7" | "phps" => {
            ("PHP", Some(ProjectRuntime::Php))
        }
        "js" | "jsx" | "mjs" | "cjs" => ("JavaScript", Some(ProjectRuntime::Node)),
        "ts" | "tsx" | "mts" | "cts" => ("TypeScript", Some(ProjectRuntime::Node)),
        "py" | "pyw" | "pyi" => ("Python", Some(ProjectRuntime::Python)),
        "rs" => ("Rust", Some(ProjectRuntime::Rust)),
        "html" | "htm" => ("HTML", None),
        "css" => ("CSS", None),
        "scss" => ("SCSS", None),
        "sass" => ("Sass", None),
        "less" => ("Less", None),
        "json" => ("JSON", None),
        "jsonc" => ("JSONC", None),
        "sql" => ("SQL", None),
        "toml" => ("TOML", None),
        "yaml" | "yml" => ("YAML", None),
        "xml" | "svg" => ("XML", None),
        "md" | "markdown" | "mdx" => ("Markdown", None),
        "sh" | "bash" | "zsh" | "fish" => ("Shell", None),
        "rb" => ("Ruby", None),
        "go" => ("Go", None),
        "java" => ("Java", None),
        "kt" | "kts" => ("Kotlin", None),
        "c" | "h" => ("C", None),
        "cpp" | "cc" | "cxx" | "hpp" => ("C++", None),
        "cs" => ("C#", None),
        "vue" => ("Vue", Some(ProjectRuntime::Node)),
        "svelte" => ("Svelte", Some(ProjectRuntime::Node)),
        "swift" => ("Swift", None),
        "ini" | "conf" | "cfg" => ("INI", None),
        "env" => ("ENV", None),
        "dockerfile" => ("Dockerfile", None),
        "txt" | "text" | "" => ("Texto", None),
        _ => return (key.to_uppercase(), None),
    };
    (name.into(), runtime)
}
impl CodisseuApp {
    pub(super) fn language_status_text(&self) -> (String, String) {
        let (language, runtime) = if let Some(doc) = self.service.active_document() {
            file_language(&doc.path)
        } else if let Some(runtime) = self.service.project_versions().hints.first().copied() {
            (runtime.label().into(), Some(runtime))
        } else {
            let hint = self
                .service
                .entries
                .iter()
                .take(128)
                .filter(|e| !e.is_dir)
                .map(|e| file_language(&e.path))
                .find(|(_, runtime)| runtime.is_some());
            hint.unwrap_or_else(|| {
                (
                    if self.service.workspace_path().is_some() {
                        "Projeto"
                    } else {
                        "Sem arquivo"
                    }
                    .into(),
                    None,
                )
            })
        };
        if let Some(runtime) = runtime {
            if let Some(version) = self.service.project_version(runtime) {
                let label = if runtime == ProjectRuntime::Node && language != runtime.label() {
                    format!("{language} · {} {}", runtime.label(), version.value)
                } else {
                    format!("{language} {}", version.value)
                };
                return (label, version.tooltip());
            }
            return (
                language.clone(),
                format!(
                    "{language}\nVersão do projeto indisponível. A linguagem permanece identificada."
                ),
            );
        }
        (
            language.clone(),
            format!("Linguagem/tipo do arquivo: {language}"),
        )
    }
    pub(super) fn language_status(&self, ui: &mut egui::Ui) {
        let (label, tooltip) = self.language_status_text();
        let width = ui.available_width().clamp(0., 200.);
        let text_width = ui.fonts_mut(|f| {
            f.layout_no_wrap(
                label.clone(),
                egui::FontId::proportional(self.tokens.ui_font_size),
                color(self.tokens.text),
            )
            .size()
            .x
        });
        ui.add_sized(
            [width.min(text_width), ui.available_height()],
            egui::Label::new(label)
                .truncate()
                .show_tooltip_when_elided(false),
        )
        .on_hover_text(tooltip);
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

    fn settle(service: &mut AppService) {
        let end = Instant::now() + Duration::from_secs(10);
        while service.is_busy() {
            service.poll();
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        service.poll();
    }
    fn wait(h: &mut Harness<'_, CodisseuApp>, expected: Option<&str>) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            h.run();
            if !h.state().service.project_versions_pending()
                && h.state()
                    .service
                    .project_version(codisseu_core::ProjectRuntime::Php)
                    .map(|v| v.value.as_str())
                    == expected
            {
                break;
            }
            assert!(Instant::now() < end, "PHP status timeout");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn footer_tracks_active_project_with_padding_and_source_tooltip() {
        for scale in [1., 1.25] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            for (folder, value) in [("legacy", "7.4.33"), ("modern", "8.3.12")] {
                let project = root.join(folder);
                fs::create_dir(&project).unwrap();
                fs::write(
                    project.join("composer.json"),
                    format!(r#"{{"config":{{"platform":{{"php":"{value}"}}}}}}"#),
                )
                .unwrap();
                fs::write(project.join("index.php"), "<?php\n// PHP version status\n").unwrap();
            }
            fs::write(root.join("readme.txt"), "workspace\n").unwrap();
            let mut service = AppService::new(root.join("profile/session.json"));
            service
                .dispatch(Command::OpenWorkspace(root.into()))
                .unwrap();
            settle(&mut service);
            service
                .dispatch(Command::OpenFile(root.join("legacy/index.php")))
                .unwrap();
            settle(&mut service);
            let mut h = Harness::builder()
                .with_size([1000., 700.])
                .with_pixels_per_point(scale)
                .with_max_steps(40)
                .build_ui_state(
                    |ui, app: &mut CodisseuApp| app.show(ui),
                    CodisseuApp::new(service),
                );
            wait(&mut h, Some("7.4.33"));
            let version = h.get_by_label("PHP 7.4.33").rect();
            let encoding = h.get_by_label("UTF-8").rect();
            assert!(version.max.x <= encoding.min.x);
            assert!(
                version.min.y >= 700. - h.state().tokens.status_height - 16.
                    && version.max.y <= 696.,
                "{version:?}"
            );
            assert!((version.center().y - encoding.center().y).abs() < 1.);
            assert!(version.min.x >= 12. && version.max.x < 948.);
            h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.);
            h.get_by_label("PHP 7.4.33").hover();
            h.run();
            assert!(h.query_by_label_contains("config.platform.php").is_some());
            if let Some(out) = std::env::var_os("CODISSEU_PHP_STATUS_CAPTURES") {
                let out = PathBuf::from(out);
                fs::create_dir_all(&out).unwrap();
                h.render()
                    .unwrap()
                    .save(out.join(format!("php-status-{scale}.png")))
                    .unwrap();
            }
            h.state_mut()
                .service
                .dispatch(Command::OpenFile(root.join("modern/index.php")))
                .unwrap();
            settle(&mut h.state_mut().service);
            wait(&mut h, Some("8.3.12"));
            assert!(h.query_by_label("PHP 7.4.33").is_none());
            assert!(h.query_by_label("PHP 8.3.12").is_some());
            h.state_mut()
                .service
                .dispatch(Command::OpenFile(root.join("readme.txt")))
                .unwrap();
            settle(&mut h.state_mut().service);
            wait(&mut h, None);
            assert!(h.query_by_label("PHP 8.3.12").is_none());
        }
    }
}

#[cfg(test)]
mod generic_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::{
        fs,
        time::{Duration, Instant},
    };

    fn wait(h: &mut Harness<'_, CodisseuApp>, label: &str) {
        let end = Instant::now() + Duration::from_secs(12);
        loop {
            h.run();
            if !h.state().service.is_busy()
                && !h.state().service.project_versions_pending()
                && h.state().language_status_text().0 == label
            {
                break;
            }
            assert!(
                Instant::now() < end,
                "expected {label}, got {:?}",
                h.state().language_status_text()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn open(h: &mut Harness<'_, CodisseuApp>, path: PathBuf, expected: &str) {
        h.state_mut()
            .service
            .dispatch(Command::OpenFile(path))
            .unwrap();
        wait(h, expected);
        let r = h.get_by_label(expected).rect();
        let e = h.get_by_label("UTF-8").rect();
        assert!(
            r.max.x <= e.min.x && r.min.x >= 12. && r.width() <= 201.,
            "{expected}: {r:?}"
        );
        assert!((r.center().y - e.center().y).abs() < 1.);
    }
    #[test]
    fn mixed_project_selects_its_runtime_and_keeps_language_without_version() {
        for scale in [1., 1.25] {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            for (file, text) in [
                (
                    "composer.json",
                    r#"{"config":{"platform":{"php":"8.2.12"}}}"#,
                ),
                (".nvmrc", "22.12.0\n"),
                (".python-version", "3.12.8\n"),
                ("rust-toolchain.toml", "[toolchain]\nchannel='1.90.0'\n"),
                ("index.php", "<?php\n"),
                ("main.ts", "export {};\n"),
                ("main.js", "let a=1;\n"),
                ("main.py", "pass\n"),
                ("main.rs", "fn main() {}\n"),
                ("page.html", "<h1>Example</h1>\n"),
                ("style.css", "a {}\n"),
                ("data.json", "{}\n"),
                ("query.sql", "SELECT 1;\n"),
                ("readme.md", "# Example\n"),
                ("sample.oddtype", "text\n"),
                ("README", "text\n"),
            ] {
                fs::write(root.join(file), text).unwrap();
            }
            let mut service = AppService::new(root.join("profile/session.json"));
            service
                .dispatch(Command::OpenWorkspace(root.into()))
                .unwrap();
            let mut h = Harness::builder()
                .with_size([1000., 700.])
                .with_pixels_per_point(scale)
                .with_max_steps(40)
                .build_ui_state(
                    |ui, app: &mut CodisseuApp| app.show(ui),
                    CodisseuApp::new(service),
                );
            wait(&mut h, "PHP 8.2.12");
            for (file, label) in [
                ("index.php", "PHP 8.2.12"),
                ("main.ts", "TypeScript · Node.js 22.12.0"),
                ("main.js", "JavaScript · Node.js 22.12.0"),
                ("main.py", "Python 3.12.8"),
                ("main.rs", "Rust 1.90.0"),
                ("page.html", "HTML"),
                ("style.css", "CSS"),
                ("data.json", "JSON"),
                ("query.sql", "SQL"),
                ("readme.md", "Markdown"),
                ("sample.oddtype", "ODDTYPE"),
                ("README", "Texto"),
            ] {
                open(&mut h, root.join(file), label);
                if let Some(out) = std::env::var_os("CODISSEU_LANGUAGE_STATUS_CAPTURES") {
                    let out = PathBuf::from(out);
                    fs::create_dir_all(&out).unwrap();
                    h.render()
                        .unwrap()
                        .save(out.join(format!("{file}-{scale}.png")))
                        .unwrap();
                }
            }
            fs::write(root.join("composer.json"), "{}").unwrap();
            fs::remove_file(root.join(".python-version")).unwrap();
            fs::remove_file(root.join(".nvmrc")).unwrap();
            fs::remove_file(root.join("rust-toolchain.toml")).unwrap();
            open(&mut h, root.join("index.php"), "PHP");
            for (file, label) in [
                ("main.ts", "TypeScript"),
                ("main.js", "JavaScript"),
                ("main.py", "Python"),
                ("main.rs", "Rust"),
            ] {
                open(&mut h, root.join(file), label);
            }
            h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.);
            h.get_by_label("Rust").hover();
            h.run();
            assert!(
                h.query_by_label_contains("Versão do projeto indisponível")
                    .is_some()
            );
        }
    }
    #[test]
    fn empty_workspace_and_no_extension_always_have_a_nonempty_label() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = CodisseuApp::new(AppService::new(dir.path().join("profile/session.json")));
        assert_eq!(app.language_status_text().0, "Sem arquivo");
        app.service
            .dispatch(Command::OpenWorkspace(dir.path().into()))
            .unwrap();
        let mut h = Harness::builder()
            .with_size([800., 600.])
            .build_ui_state(|ui, app: &mut CodisseuApp| app.show(ui), app);
        wait(&mut h, "Projeto");
        assert!(h.query_by_label("Projeto").is_some());
        assert_eq!(file_language(std::path::Path::new(".env.local")).0, "ENV");
        assert_eq!(
            file_language(std::path::Path::new("Dockerfile")).0,
            "Dockerfile"
        );
        assert_eq!(file_language(std::path::Path::new("unknown")).0, "Texto");
    }
}
