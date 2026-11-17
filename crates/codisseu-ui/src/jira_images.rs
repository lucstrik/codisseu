//! Bounded GPU/image cache shared by Jira people and rich content.
use super::*;
use std::{collections::HashMap, sync::Arc};
struct Entry {
    image: Option<Arc<egui::ColorImage>>,
    texture: Option<egui::TextureHandle>,
    error: Option<String>,
    used: u64,
}
#[derive(Default)]
pub(super) struct Images {
    cache: HashMap<String, Entry>,
    clock: u64,
}
impl Images {
    pub(super) fn clear(&mut self) {
        self.cache.clear()
    }
    pub(super) fn tick(
        &mut self,
        ctx: &egui::Context,
        media: &mut codisseu_core::jira::media::Media,
    ) {
        self.clock = self.clock.wrapping_add(1);
        while let Some((url, result)) = media.completed.pop_front() {
            let (image, texture, error) = match result {
                Ok(r) => {
                    let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied(
                        [r.width, r.height],
                        &r.rgba,
                    ));
                    let texture = ctx.load_texture(
                        "jira.image",
                        image.as_ref().clone(),
                        egui::TextureOptions::LINEAR,
                    );
                    (Some(image), Some(texture), None)
                }
                Err(e) => (None, None, Some(e)),
            };
            self.cache.insert(
                url,
                Entry {
                    image,
                    texture,
                    error,
                    used: self.clock,
                },
            );
        }
        while self.cache.len() > 32
            || self
                .cache
                .values()
                .filter_map(|e| e.image.as_ref())
                .map(|i| i.pixels.len() * 4)
                .sum::<usize>()
                > 24 * 1024 * 1024
        {
            if let Some(k) = self
                .cache
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            {
                self.cache.remove(&k);
            } else {
                break;
            }
        }
        if media.pending() {
            ctx.request_repaint_after(Duration::from_millis(50))
        }
    }
    fn texture(
        &mut self,
        media: &mut codisseu_core::jira::media::Media,
        url: &str,
    ) -> Option<egui::TextureId> {
        if url.is_empty() {
            return None;
        }
        if let Some(e) = self.cache.get_mut(url) {
            e.used = self.clock;
            return e.texture.as_ref().map(egui::TextureHandle::id);
        }
        if media.request(url) {
            self.cache.insert(
                url.into(),
                Entry {
                    image: None,
                    texture: None,
                    error: None,
                    used: self.clock,
                },
            );
        }
        None
    }
    pub(super) fn avatar(
        &mut self,
        ui: &mut egui::Ui,
        media: &mut codisseu_core::jira::media::Media,
        url: &str,
        name: &str,
        size: f32,
        tokens: &Tokens,
    ) {
        let texture = self.texture(media, url);
        let texture = texture.filter(|_| {
            self.cache
                .get(url)
                .and_then(|e| e.image.as_ref())
                .is_some_and(|i| has_detail(i))
        });
        avatar(ui, texture, name, size, tokens);
        if media.pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
    pub(super) fn content(
        &mut self,
        markdown: &mut markdown::RemoteMarkdown,
        media: &mut codisseu_core::jira::media::Media,
    ) {
        for url in markdown.image_urls() {
            self.texture(media, &url);
            if let Some(e) = self.cache.get(&url) {
                if let Some(image) = &e.image {
                    markdown.install_image(&url, image.clone())
                } else if let Some(error) = &e.error {
                    markdown.image_error(&url, error.clone())
                }
            }
        }
    }
}
pub(super) fn avatar(
    ui: &mut egui::Ui,
    texture: Option<egui::TextureId>,
    name: &str,
    size: f32,
    tokens: &Tokens,
) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if let Some(texture) = texture {
        let mut mesh = egui::Mesh::with_texture(texture);
        let n = 48;
        mesh.vertices.push(egui::epaint::Vertex {
            pos: rect.center(),
            uv: egui::pos2(0.5, 0.5),
            color: egui::Color32::WHITE,
        });
        for i in 0..=n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let d = egui::vec2(a.cos(), a.sin());
            mesh.vertices.push(egui::epaint::Vertex {
                pos: rect.center() + d * size / 2.,
                uv: egui::pos2(0.5 + d.x * 0.5, 0.5 + d.y * 0.5),
                color: egui::Color32::WHITE,
            });
            if i > 0 {
                mesh.indices.extend_from_slice(&[0, i, i + 1]);
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    } else {
        ui.painter()
            .circle_filled(rect.center(), size / 2., color(tokens.rail_selection));
        let initials = initials(name);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initials.to_uppercase(),
            FontId::proportional(size * 0.40),
            color(tokens.text),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Image,
            true,
            format!("Avatar de {name}"),
        )
    });
    response.on_hover_text(name);
}
pub(super) fn has_detail(image: &egui::ColorImage) -> bool {
    let Some(first) = image.pixels.first() else {
        return false;
    };
    image
        .pixels
        .iter()
        .step_by((image.pixels.len() / 256).max(1))
        .any(|p| {
            p.r().abs_diff(first.r()) > 6
                || p.g().abs_diff(first.g()) > 6
                || p.b().abs_diff(first.b()) > 6
                || p.a().abs_diff(first.a()) > 6
        })
}
fn initials(name: &str) -> String {
    let words: Vec<_> = name
        .split_whitespace()
        .filter_map(|s| s.chars().find(|c| c.is_alphanumeric()))
        .collect();
    let mut text = String::new();
    if let Some(c) = words.first() {
        text.push(*c)
    }
    if words.len() > 1 {
        text.push(*words.last().unwrap())
    }
    if text.is_empty() {
        "?".into()
    } else {
        text.to_uppercase().chars().take(2).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_blank_photos_use_readable_name_initials() {
        assert_eq!(initials("Lucas Torres Sao Felicio"), "LF");
        assert_eq!(initials("  Maria  Silva "), "MS");
        assert_eq!(initials("Lucas"), "L");
        assert_eq!(initials(""), "?");
        let mut image = egui::ColorImage::filled([48, 48], egui::Color32::from_rgb(80, 130, 210));
        assert!(!has_detail(&image));
        for y in 12..36 {
            for x in 12..36 {
                image.pixels[y * 48 + x] = egui::Color32::WHITE
            }
        }
        assert!(has_detail(&image));
    }
}
