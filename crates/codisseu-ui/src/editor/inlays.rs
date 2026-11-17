//! Visual-only insertions into a shaped line. Logical source coordinates stay unchanged.
use super::*;
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Hint {
    pub column: usize,
    pub text: String,
    pub tooltip: Option<String>,
}
#[derive(Clone)]
pub(super) struct Span {
    pub display: Range<usize>,
    pub column: usize,
    pub tooltip: String,
}

pub(super) fn collect(
    service: &AppService,
    id: DocumentId,
    version: u64,
    snapshot: &TextSnapshot,
) -> BTreeMap<usize, Vec<Hint>> {
    let mut result: BTreeMap<usize, Vec<Hint>> = BTreeMap::new();
    if !service.inlay_hints_enabled(id) || service.inlay_hints.anchor() != Some((id, version)) {
        return result;
    }
    for hint in &service.inlay_hints.items {
        let line = hint.line as usize;
        if line >= snapshot.len_lines() {
            continue;
        }
        let row = snapshot.line(line);
        if row.len_chars() > 2048 {
            continue;
        }
        let text = row.to_string();
        let text = text.trim_end_matches(['\r', '\n']);
        let mut units = 0;
        let mut column = None;
        for (index, ch) in text.chars().chain(std::iter::once('\0')).enumerate() {
            if units == hint.character {
                column = Some(index);
                break;
            }
            units += ch.len_utf16() as u32;
            if units > hint.character {
                break;
            }
        }
        let Some(column) = column else {
            continue;
        };
        let label = format!(
            "{}{}{}",
            if hint.padding_left { " " } else { "" },
            hint.label,
            if hint.padding_right { " " } else { "" }
        );
        let values = result.entry(line).or_default();
        if values.len() < 32
            && values.iter().map(|h| h.text.len()).sum::<usize>() + label.len() <= 4096
        {
            values.push(Hint {
                column,
                text: label,
                tooltip: hint.tooltip.clone(),
            });
        }
    }
    for hints in result.values_mut() {
        hints.sort_by_key(|h| h.column);
    }
    result
}

pub(super) fn decorate(
    mut source: egui::text::LayoutJob,
    logical: &[usize],
    hints: &[Hint],
    t: &Tokens,
) -> (egui::text::LayoutJob, Vec<usize>, Vec<Span>) {
    let mut job = egui::text::LayoutJob::default();
    let mut map = logical.to_vec();
    let mut spans = Vec::new();
    let mut previous = 0usize;
    let mut inserted = 0usize;
    let byte_offsets = source
        .text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(source.text.len()))
        .collect::<Vec<_>>();
    let append = |job: &mut egui::text::LayoutJob, a: usize, b: usize| {
        for section in &source.sections {
            let start = a.max(section.byte_range.start.0);
            let end = b.min(section.byte_range.end.0);
            if start < end {
                job.append(&source.text[start..end], 0., section.format.clone());
            }
        }
    };
    for hint in hints {
        if hint.column >= logical.len() {
            continue;
        }
        let offset = logical[hint.column];
        let byte = byte_offsets[offset];
        if byte < previous {
            continue;
        }
        append(&mut job, previous, byte);
        let start = offset + inserted;
        let count = hint.text.chars().count();
        job.append(
            &hint.text,
            0.,
            egui::TextFormat {
                font_id: FontId::monospace((t.editor_font_size - 2.).max(9.)),
                color: color(t.muted),
                background: color(t.panel),
                ..Default::default()
            },
        );
        for at in map.iter_mut().skip(hint.column) {
            *at += count;
        }
        spans.push(Span {
            display: start..start + count,
            column: hint.column,
            tooltip: hint
                .tooltip
                .clone()
                .unwrap_or_else(|| format!("Rust: {}", hint.text.trim())),
        });
        inserted += count;
        previous = byte;
    }
    append(&mut job, previous, source.text.len());
    if spans.is_empty() {
        return (source, logical.to_vec(), spans);
    }
    job.wrap = source.wrap;
    source.sections.clear();
    (job, map, spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_hints_keep_source_coordinates_tabs_unicode_copy_and_cache_stable() {
        let context = egui::Context::default();
        let tokens = Tokens::load();
        tokens.apply(&context);
        let source = "\tlet emoji = \"😀\"; let answer = calculate(40, 2);\r\n";
        let snapshot = TextSnapshot::from_str(source);
        let column = source[..source.find("answer").unwrap() + 6].chars().count();
        let parameter = source[..source.find("40").unwrap()].chars().count();
        let mut cache = layout::LineCache::default();
        let mut output = context.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                cache.begin_frame();
                cache.begin_document(snapshot.clone(), 0, None, "rs");
                cache.sync(0, None, ui, &tokens);
                let original = cache.get(ui, &snapshot, 0, "rs", &tokens);
                let hints = BTreeMap::from([(
                    0,
                    vec![
                        Hint {
                            column,
                            text: ": i32".into(),
                            tooltip: Some("Rust type".into()),
                        },
                        Hint {
                            column: parameter,
                            text: "first: ".into(),
                            tooltip: None,
                        },
                    ],
                )]);
                cache.set_inlays(0, 7, hints.clone());
                let adorned = cache.get(ui, &snapshot, 0, "rs", &tokens);
                assert!(adorned.width() > original.width());
                assert_eq!(
                    adorned.visible_text(ui, &snapshot, &tokens, 0.0..1000.0),
                    original.visible_text(ui, &snapshot, &tokens, 0.0..1000.0)
                );
                for index in 0..=adorned.len() {
                    assert_eq!(
                        adorned.visual_column(ui, &snapshot, &tokens, index),
                        original.visual_column(ui, &snapshot, &tokens, index)
                    );
                    let x = adorned.x(ui, &snapshot, &tokens, index).unwrap();
                    assert_eq!(
                        adorned.column(ui, &snapshot, &tokens, x),
                        Some(index),
                        "source column {index}"
                    );
                }
                let before = original.x(ui, &snapshot, &tokens, column).unwrap();
                let after = adorned.x(ui, &snapshot, &tokens, column).unwrap();
                assert_eq!(
                    adorned.column(ui, &snapshot, &tokens, (before + after) / 2.),
                    Some(column)
                );
                cache.set_inlays(0, 7, hints);
                let same = cache.get(ui, &snapshot, 0, "rs", &tokens);
                assert!(std::rc::Rc::ptr_eq(&same, &adorned));
                assert_eq!(snapshot.to_string(), source);
                cache.sync(1, None, ui, &tokens);
                assert_eq!(
                    cache.inlay_count(0..1),
                    0,
                    "an edit invalidates visual hints before drawing another frame"
                );
            });
        });
        output.textures_delta.clear();
    }
    #[test]
    fn inert_label_insertions_preserve_lexical_sections_and_emoji_boundaries() {
        let tokens = Tokens::load();
        let source = "let message = \"ação 😀\";";
        let job = highlight::layout(source, "rs", FontId::monospace(tokens.editor_font_size));
        let logical = (0..=source.chars().count()).collect::<Vec<_>>();
        let column = source[..source.find("message").unwrap() + 7]
            .chars()
            .count();
        let (job, map, spans) = decorate(
            job,
            &logical,
            &[Hint {
                column,
                text: ": &str".into(),
                tooltip: None,
            }],
            &tokens,
        );
        assert!(job.text.contains("message: &str"));
        assert_eq!(map.len(), logical.len());
        assert_eq!(spans.len(), 1);
        assert_eq!(job.text.chars().count(), source.chars().count() + 6);
        assert!(
            job.sections
                .iter()
                .all(|s| job.text.is_char_boundary(s.byte_range.start.0)
                    && job.text.is_char_boundary(s.byte_range.end.0))
        );
    }
}
