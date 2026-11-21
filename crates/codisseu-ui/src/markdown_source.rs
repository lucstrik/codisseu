//! Bounded line-level source styling; rendered Markdown uses the document parser worker.
use egui::{FontId, TextFormat, text::LayoutJob};
use pulldown_cmark::{Event, Options, Parser, Tag};
pub(super) fn layout_themed(
    text: &str,
    font: FontId,
    theme: codisseu_core::ThemePreference,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let palette = super::highlight::source_colors_themed(theme);
    if text.len() > 64 * 1024 {
        job.append(
            text,
            0.,
            TextFormat {
                font_id: font,
                color: palette[0],
                ..Default::default()
            },
        );
        return job;
    }
    let mut styles = vec![0u8; text.len()];
    for (event, range) in Parser::new_ext(
        text,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS,
    )
    .into_offset_iter()
    .take(4096)
    {
        let style = match event {
            Event::Start(Tag::Heading { .. }) => 1,
            Event::Start(Tag::Strong) => 2,
            Event::Start(Tag::Emphasis) => 4,
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) => 8,
            Event::Start(Tag::Link { .. }) | Event::Start(Tag::Image { .. }) => 16,
            Event::Html(_) | Event::InlineHtml(_) => 32,
            Event::TaskListMarker(_) => 1,
            _ => 0,
        };
        for slot in &mut styles[range.start.min(text.len())..range.end.min(text.len())] {
            *slot |= style;
        }
    }
    let mut start = 0;
    let mut style = styles.first().copied().unwrap_or(0);
    for (at, _) in text
        .char_indices()
        .skip(1)
        .chain(std::iter::once((text.len(), '\0')))
    {
        let next = styles.get(at).copied().unwrap_or(0);
        if next == style && at != text.len() {
            continue;
        }
        let mut f = font.clone();
        if style & 3 != 0 {
            f.family = egui::FontFamily::Name("codisseu.mono.bold".into());
        } else if style & 4 != 0 {
            f.family = egui::FontFamily::Name("codisseu.mono.italic".into());
        }
        let color = if style & 32 != 0 {
            palette[3]
        } else if style & 16 != 0 {
            palette[4]
        } else if style & 8 != 0 {
            palette[2]
        } else if style & 1 != 0 {
            palette[1]
        } else {
            palette[0]
        };
        job.append(
            &text[start..at],
            0.,
            TextFormat {
                font_id: f,
                color,
                underline: if style & 16 != 0 {
                    egui::Stroke::new(0.5, color)
                } else {
                    egui::Stroke::NONE
                },
                strikethrough: egui::Stroke::NONE,
                ..Default::default()
            },
        );
        start = at;
        style = next;
    }
    job
}
#[cfg(test)]
mod tests {
    #[test]
    fn source_styles_keep_unicode_crlf_and_markup_bytes() {
        let text = "# ação 😀 **strong** and `code` [local](./file.md)\r\n";
        let job = super::layout_themed(
            text,
            egui::FontId::monospace(13.),
            codisseu_core::ThemePreference::Classic,
        );
        assert_eq!(job.text.as_str(), text);
        assert!(job.sections.len() > 3);
    }
}
