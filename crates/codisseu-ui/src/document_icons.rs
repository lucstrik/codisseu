//! Licensed public IDE icons and original fallbacks. See reference/public/ide-icons/sources.json.
use eframe::egui::{self, Color32, Rect, TextureHandle, Ui, Vec2};
use std::collections::{HashMap, VecDeque};

macro_rules! asset {
    ($name:literal) => {
        (
            concat!("codisseu.ide-icon.", $name),
            include_bytes!(concat!("../assets/ide-icons/", $name, ".png")).as_slice(),
        )
    };
}
fn asset(name: &str) -> (&'static str, &'static [u8]) {
    match name {
        "merge-accept-left" => asset!("merge-accept-left"),
        "merge-accept-right" => asset!("merge-accept-right"),
        "merge-ignore" => asset!("merge-ignore"),
        "merge-nonconflicting-left" => asset!("merge-nonconflicting-left"),
        "merge-nonconflicting-all" => asset!("merge-nonconflicting-all"),
        "merge-nonconflicting-right" => asset!("merge-nonconflicting-right"),
        "merge-simple" => asset!("merge-simple"),
        "docker-tool-window" => (
            "codisseu.docker-tool-window",
            include_bytes!("../assets/docker/docker-tool-window.png").as_slice(),
        ),
        "gitlens-git-commit" => asset!("gitlens-git-commit"),
        "gitlens-git-compare" => asset!("gitlens-git-compare"),
        "gitlens-history" => asset!("gitlens-history"),
        "gitlens-copy" => asset!("gitlens-copy"),
        "gitlens-versions" => asset!("gitlens-versions"),
        "gitlens-search" => asset!("gitlens-search"),

        "ai-brain" => asset!("ai-brain"),
        "ai-symbol" => asset!("ai-symbol"),
        "ai-shield" => asset!("ai-shield"),
        "ai-send" => asset!("ai-send"),
        "ai-delete" => asset!("ai-delete"),
        "add" => asset!("add"),
        "branch" => asset!("branch"),
        "changes" => asset!("changes"),
        "close" => asset!("close"),
        "code" => asset!("code"),
        "collapse-all" => asset!("collapse-all"),
        "column" => asset!("column"),
        "commit" => asset!("commit"),
        "compare" => asset!("compare"),
        "copy" => asset!("copy"),
        "debug" => asset!("debug"),
        "build" => asset!("build"),
        "down" => asset!("down"),
        "expand-all" => asset!("expand-all"),
        "export" => asset!("export"),
        "file-archive" => asset!("file-archive"),
        "file-c" => asset!("file-c"),
        "file-config" => asset!("file-config"),
        "file-cpp" => asset!("file-cpp"),
        "file-css" => asset!("file-css"),
        "file-docker" => asset!("file-docker"),
        "file-elixir" => asset!("file-elixir"),
        "file-graphql" => asset!("file-graphql"),
        "file-html" => asset!("file-html"),
        "file-image" => asset!("file-image"),
        "file-java" => asset!("file-java"),
        "file-javaScript" => asset!("file-javaScript"),
        "file-json" => asset!("file-json"),
        "file-jsx" => asset!("file-jsx"),
        "file-markdown" => asset!("file-markdown"),
        "file-php" => asset!("file-php"),
        "file-python" => asset!("file-python"),
        "file-rust" => asset!("file-rust"),
        "file-shell" => asset!("file-shell"),
        "file-sql" => asset!("file-sql"),
        "file-text" => asset!("file-text"),
        "file-toml" => asset!("file-toml"),
        "file-tsx" => asset!("file-tsx"),
        "file-typeScript" => asset!("file-typeScript"),
        "file-unknown" => asset!("file-unknown"),
        "file-vue" => asset!("file-vue"),
        "file-xml" => asset!("file-xml"),
        "file-yaml" => asset!("file-yaml"),
        "filter" => asset!("filter"),
        "folder" => asset!("folder"),
        "import" => asset!("import"),
        "locate" => asset!("locate"),
        "markdown-preview" => asset!("markdown-preview"),
        "markdown-source" => asset!("markdown-source"),
        "markdown-split" => asset!("markdown-split"),
        "markdown-sync" => asset!("markdown-sync"),
        "menu" => asset!("menu"),
        "more" => asset!("more"),
        "mute-breakpoints" => asset!("mute-breakpoints"),
        "new-file" => asset!("new-file"),
        "open-file" => asset!("open-file"),
        "new-folder" => asset!("new-folder"),
        "next" => asset!("next"),
        "pause" => asset!("pause"),
        "play" => asset!("play"),
        "previous" => asset!("previous"),
        "problems" => asset!("problems"),
        "refresh" => asset!("refresh"),
        "remove" => asset!("remove"),
        "revert" => asset!("revert"),
        "rollback" => asset!("rollback"),
        "save" => asset!("save"),
        "schema" => asset!("schema"),
        "search" => asset!("search"),
        "settings" => asset!("settings"),
        "sql-preview" => asset!("sql-preview"),
        "star" => asset!("star"),
        "stash" => asset!("stash"),
        "step-into" => asset!("step-into"),
        "step-out" => asset!("step-out"),
        "step-over" => asset!("step-over"),
        "stop" => asset!("stop"),
        "submit" => asset!("submit"),
        "table" => asset!("table"),
        "tag" => asset!("tag"),
        "terminal" => asset!("terminal"),
        "up" => asset!("up"),
        "view" => asset!("view"),
        _ => asset!("file-unknown"),
    }
}

// An asset may appear at 16px in the toolbar and 20px in a rail in the same
// frame. Keep both sizes; replacing one with the other redecodes/resizes PNGs
// and uploads textures on every scroll frame. 256 * 64² * RGBA <= 4 MiB.
type IconKey = (&'static str, bool, u32);
const MAX_ICON_VARIANTS: usize = 256;
#[derive(Clone, Default)]
struct IconCache {
    textures: HashMap<IconKey, TextureHandle>,
    order: VecDeque<IconKey>,
}
impl IconCache {
    fn insert(&mut self, key: IconKey, texture: TextureHandle) {
        if !self.textures.contains_key(&key) {
            if self.textures.len() >= MAX_ICON_VARIANTS
                && let Some(old) = self.order.pop_front()
            {
                self.textures.remove(&old);
            }
            self.order.push_back(key);
        }
        self.textures.insert(key, texture);
    }
}

pub(super) fn paint(ui: &Ui, rect: Rect, name: &str, side: f32, opacity: f32) {
    paint_variant(ui, rect, name, side, opacity, None);
}
pub(super) fn paint_monochrome(ui: &Ui, rect: Rect, name: &str, side: f32, tint: Color32) {
    paint_variant(
        ui,
        rect,
        name,
        side,
        if ui.is_enabled() { 1. } else { 0.4 },
        Some(tint),
    );
}
fn paint_variant(ui: &Ui, rect: Rect, name: &str, side: f32, opacity: f32, tint: Option<Color32>) {
    let (key, bytes) = asset(name);
    let side = side.min(rect.width()).min(rect.height());
    let physical = (side * ui.ctx().pixels_per_point()).round().clamp(8., 64.) as u32;
    let variant = (key, tint.is_some(), physical);
    let cache_id = egui::Id::unique("codisseu.ide-icon.cache");
    let cached = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<IconCache>(cache_id)
            .textures
            .get(&variant)
            .cloned()
    });
    let texture = cached.unwrap_or_else(|| {
        // All bytes are compiled-in, audited PNGs; no filesystem/network request occurs here.
        let mut decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .expect("bundled IDE icon is a valid PNG")
            .to_rgba8();
        if tint.is_some() {
            // Use only the licensed artwork's alpha coverage. The UI supplies
            // its normal gray; source colors remain untouched in other tools.
            for pixel in decoded.pixels_mut() {
                pixel.0[..3].fill(255);
            }
        }
        let image = image::imageops::resize(
            &decoded,
            physical,
            physical,
            image::imageops::FilterType::Lanczos3,
        );
        let handle = ui.ctx().load_texture(
            format!("{key}.{}", tint.is_some()),
            egui::ColorImage::from_rgba_unmultiplied(
                [physical as usize, physical as usize],
                image.as_raw(),
            ),
            egui::TextureOptions::LINEAR,
        );
        ui.ctx().data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<IconCache>(cache_id);
            cache.insert(variant, handle.clone());
        });
        handle
    });
    ui.painter().image(
        texture.id(),
        Rect::from_center_size(rect.center(), Vec2::splat(side)),
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1., 1.)),
        tint.unwrap_or(Color32::WHITE).gamma_multiply(opacity),
    );
}

pub(super) fn file_asset(name: &str) -> &'static str {
    let name = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);
    if name == "Cargo.lock" {
        return "file-toml";
    }
    if name == "yarn.lock" || name == "pnpm-lock.yaml" {
        return "file-yaml";
    }
    if name == "composer.lock" {
        return "file-json";
    }
    if name.eq_ignore_ascii_case("Dockerfile") {
        return "file-docker";
    }
    if name == ".gitignore"
        || name == ".gitattributes"
        || name == ".editorconfig"
        || name.starts_with(".env")
    {
        return "file-config";
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension)
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" | "mdown" | "mkd" | "mdx" => "file-markdown",
        "json" | "jsonc" | "json5" => "file-json",
        "py" | "pyi" | "pyw" => "file-python",
        "ex" | "exs" | "eex" | "heex" => "file-elixir",
        "java" | "class" => "file-java",
        "rs" => "file-rust",
        "tsx" => "file-tsx",
        "jsx" => "file-jsx",
        "vue" => "file-vue",
        "ts" | "mts" | "cts" => "file-typeScript",
        "js" | "mjs" | "cjs" => "file-javaScript",
        "php" | "phtml" | "php3" | "php4" | "php5" | "phps" => "file-php",
        "htm" | "html" | "xhtml" => "file-html",
        "css" | "scss" | "sass" | "less" => "file-css",
        "sql" | "sqlite" | "sqlite3" | "db" => "file-sql",
        "txt" | "log" | "csv" => "file-text",
        "xml" | "xsd" | "svg" => "file-xml",
        "yaml" | "yml" => "file-yaml",
        "toml" => "file-toml",
        "sh" | "bash" | "zsh" | "fish" => "file-shell",
        "ini" | "conf" | "config" | "properties" => "file-config",
        "zip" | "gz" | "tar" | "bz2" | "xz" | "7z" | "rar" | "jar" => "file-archive",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" => "file-image",
        "c" | "h" => "file-c",
        "cpp" | "cc" | "cxx" | "hpp" => "file-cpp",
        "graphql" | "gql" => "file-graphql",
        _ => "file-unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternating_icon_sizes_reuse_textures_and_bound_variants() {
        let ctx = egui::Context::default();
        let mut first = None;
        for _ in 0..4 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let rect = Rect::from_min_size(egui::pos2(0., 0.), Vec2::splat(24.));
                for side in [16., 20., 16., 20.] {
                    paint(ui, rect, "settings", side, 1.);
                }
            });
            output.textures_delta.clear();
            let ids = ctx.data_mut(|d| {
                let cache = d.get_temp_mut_or_default::<IconCache>(egui::Id::unique(
                    "codisseu.ide-icon.cache",
                ));
                assert_eq!(cache.textures.len(), 2);
                [16, 20].map(|size| cache.textures[&(asset("settings").0, false, size)].id())
            });
            if let Some(first) = first {
                assert_eq!(ids, first);
            } else {
                first = Some(ids);
            }
        }
        let mut cache = IconCache::default();
        for i in 0..300 {
            cache.insert(
                ("test", false, i),
                ctx.load_texture(
                    "test",
                    egui::ColorImage::new([1, 1], vec![Color32::WHITE]),
                    egui::TextureOptions::LINEAR,
                ),
            );
        }
        assert_eq!(cache.textures.len(), MAX_ICON_VARIANTS);
        assert_eq!(cache.order.len(), MAX_ICON_VARIANTS);
        assert!(!cache.textures.contains_key(&("test", false, 0)));
    }
    #[test]
    fn common_file_types_have_distinct_and_valid_icons() {
        let names = [
            "readme.md",
            "package.json",
            "app.py",
            "app.ex",
            "App.java",
            "main.rs",
            "App.tsx",
            "App.vue",
            "App.jsx",
            "app.php",
            "app.js",
            "app.ts",
        ];
        let mut seen = std::collections::HashSet::new();
        for name in names {
            let key = file_asset(name);
            assert!(seen.insert(key), "distinct file type: {name}");
            let (_, bytes) = asset(key);
            let image = image::load_from_memory(bytes).unwrap();
            assert_eq!((image.width(), image.height()), (64, 64));
        }
        assert_eq!(file_asset("README.MARKDOWN"), "file-markdown");
        assert_eq!(file_asset("/folder.with.dots/noextension"), "file-unknown");
        assert_eq!(file_asset("composer.lock"), "file-json");
    }
    #[test]
    #[ignore = "explicit wgpu icon atlas for visual review"]
    fn ide_icon_atlas_renders() {
        let files = [
            "README.md",
            "package.json",
            "app.py",
            "app.ex",
            "App.java",
            "main.rs",
            "App.tsx",
            "App.vue",
            "App.jsx",
            "app.php",
            "app.js",
            "app.ts",
            "index.html",
            "app.css",
            "query.sql",
            "notes.txt",
        ];
        let mut h = egui_kittest::Harness::builder()
            .with_size([1040., 320.])
            .build_ui(|ui| {
                ui.set_min_size(egui::vec2(1020., 300.));
                ui.visuals_mut().override_text_color = Some(Color32::from_gray(210));
                ui.label(
                    "Codisseu · file types · licensed public assets and independent fallbacks",
                );
                egui::Grid::new("icon atlas files")
                    .num_columns(8)
                    .spacing([18., 8.])
                    .show(ui, |ui| {
                        for (i, file) in files.iter().enumerate() {
                            ui.vertical(|ui| {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(28.), egui::Sense::hover());
                                paint(ui, rect, file_asset(file), 16., 1.);
                                ui.add(
                                    egui::Label::new(*file).wrap_mode(egui::TextWrapMode::Extend),
                                );
                            });
                            if i % 8 == 7 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(16.);
                ui.horizontal(|ui| {
                    for name in [
                        "terminal",
                        "problems",
                        "changes",
                        "branch",
                        "debug",
                        "settings",
                        "play",
                        "stop",
                        "add",
                        "folder",
                        "search",
                        "markdown-source",
                        "markdown-split",
                        "markdown-preview",
                        "markdown-sync",
                    ] {
                        ui.vertical(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(28.), egui::Sense::hover());
                            paint(ui, rect, name, 16., 1.);
                            ui.small(name.strip_prefix("markdown-").unwrap_or(name));
                        });
                    }
                });
            });
        h.run();
        let image = h.render().expect("wgpu atlas render");
        image
            .save(std::env::var("CODISSEU_ICON_ATLAS").expect("explicit output path"))
            .unwrap();
    }
}
