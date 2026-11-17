//! Presentation-only grouping: every row still identifies its exact core edit.
use super::*;

/// The original uses the proportional UI face in usage rows and the editor
/// face in its code viewport. Keep our parser's text/colors, changing only
/// this tree presentation. Complex text retains egui's Unicode shaping.
pub(super) fn snippet(
    ui: &mut egui::Ui,
    cache: &mut crate::signature_paint::SignaturePaint,
    source: &egui::Galley,
    row: egui::Rect,
    indent: f32,
    background: Color32,
    tokens: &Tokens,
) {
    let mut job = (*source.job).clone();
    for section in &mut job.sections {
        section.format.font_id = FontId::proportional(tokens.ui_font_size);
        section.format.line_height = Some(17.);
    }
    let runs: Vec<_> = job
        .sections
        .iter()
        .map(|section| crate::signature_paint::Run {
            text: job.text
                [usize::from(section.byte_range.start)..usize::from(section.byte_range.end)]
                .into(),
            color: section.format.color,
            bold: false,
            clip: false,
            padding: [0., 0.],
        })
        .collect();
    let anchor = row.left_center() + egui::vec2(indent, 5.);
    let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(row));
    clipped.set_clip_rect(row.intersect(ui.clip_rect()));
    let width = cache.measure(&runs, tokens.ui_font_size, ui.ctx().pixels_per_point());
    if width.is_some_and(|width| {
        cache.paint(
            &clipped,
            &runs,
            anchor,
            width + 4.,
            background,
            tokens.ui_font_size,
        )
    }) {
        return;
    }
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    clipped.painter().galley(
        row.left_center() + egui::vec2(indent, -galley.size().y / 2.),
        galley,
        color(tokens.text),
    );
}

pub(super) fn group(
    ui: &mut egui::Ui,
    state: &mut RenameUi,
    tokens: &Tokens,
    key: &str,
    label: &str,
    indent: f32,
    file: Option<&str>,
) -> bool {
    let galley = ui.painter().layout_no_wrap(
        label.into(),
        FontId::proportional(tokens.ui_font_size),
        color(tokens.rename_text),
    );
    let (row, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width().max(indent + galley.size().x + 12.),
            tokens.row_height,
        ),
        egui::Sense::hover(),
    );
    let response = ui.interact(
        row,
        egui::Id::unique(("codisseu.rename.group", key)),
        egui::Sense::click(),
    );
    let mut expanded = !state.collapsed.contains("all") && !state.collapsed.contains(key);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            true,
            format!("{} {label}", if expanded { "Recolher" } else { "Expandir" }),
        )
    });
    if response.clicked()
        || (response.has_focus()
            && ui.input_mut(|i| {
                i.consume_key(Modifiers::NONE, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::Space)
            }))
    {
        state.collapsed.remove("all");
        expanded = !expanded;
        if expanded {
            state.collapsed.remove(key);
        } else {
            state.collapsed.insert(key.into());
        }
    }
    let center = row.left_center()
        + egui::vec2(
            indent - if file == Some("@heading") { 14. } else { 32. },
            0.,
        );
    if expanded {
        name_input::chevron(ui, center, color(tokens.muted));
    } else {
        let p = ui.painter();
        let stroke = egui::Stroke::new(1., color(tokens.muted));
        p.line_segment(
            [center + egui::vec2(-1.5, -3.5), center + egui::vec2(2., 0.)],
            stroke,
        );
        p.line_segment(
            [center + egui::vec2(2., 0.), center + egui::vec2(-1.5, 3.5)],
            stroke,
        );
    }
    let icon = egui::Rect::from_center_size(
        row.left_center() + egui::vec2(indent - 10., 0.),
        egui::vec2(16., 16.),
    );
    if file == Some("@heading") {
        // Headings have a disclosure arrow and no synthetic folder.
    } else if let Some(symbol) = file.and_then(|s| s.strip_prefix('@')) {
        let (letter, tint) = match symbol {
            "Class" => ("C", color(tokens.accent)),
            "Method" => ("m", color(tokens.editor_php_variable)),
            _ => ("ƒ", color(tokens.editor_php_variable)),
        };
        ui.painter()
            .circle_stroke(icon.center(), 6., egui::Stroke::new(1., tint));
        ui.painter().text(
            icon.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            FontId::proportional(10.),
            tint,
        );
    } else if let Some(file) = file {
        crate::components::paint_document_icon(ui, icon, file, color(tokens.accent));
    } else {
        crate::components::paint_icon(ui, icon, Icon::Folder, color(tokens.rename_text));
    }
    source::paint_label(
        &mut state.labels,
        ui,
        label,
        row.left_center() + egui::vec2(indent, 0.),
        row,
        color(tokens.panel),
        tokens,
    );
    expanded
}

pub(super) fn control(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    label: &str,
    kind: usize,
    tokens: &Tokens,
) -> egui::Response {
    let response = ui.put(
        rect,
        egui::Button::new(RichText::new(" ").color(Color32::TRANSPARENT)).frame(false),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, label));
    let c = rect.center();
    let stroke = egui::Stroke::new(1., color(tokens.rename_text));
    let painter = ui.painter();
    if response.hovered() || response.has_focus() {
        painter.rect_filled(rect.shrink(2.), 3., color(tokens.border));
    }
    match kind {
        0 | 1 => {
            let d = if kind == 0 { -1. } else { 1. };
            painter.line_segment([c + egui::vec2(0., -6.), c + egui::vec2(0., 6.)], stroke);
            painter.line_segment(
                [c + egui::vec2(-4., d * 2.), c + egui::vec2(0., d * 6.)],
                stroke,
            );
            painter.line_segment(
                [c + egui::vec2(4., d * 2.), c + egui::vec2(0., d * 6.)],
                stroke,
            );
        }
        2 => {
            painter.add(egui::Shape::closed_line(
                vec![
                    c + egui::vec2(-8., 0.),
                    c + egui::vec2(-4., -4.),
                    c + egui::vec2(4., -4.),
                    c + egui::vec2(8., 0.),
                    c + egui::vec2(4., 4.),
                    c + egui::vec2(-4., 4.),
                ],
                stroke,
            ));
            painter.circle_stroke(c, 2.5, stroke);
        }
        3 | 4 => {
            painter.rect_stroke(
                egui::Rect::from_center_size(c, egui::vec2(12., 12.)),
                1.,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([c - egui::vec2(3., 0.), c + egui::vec2(3., 0.)], stroke);
            if kind == 3 {
                painter.line_segment([c - egui::vec2(0., 3.), c + egui::vec2(0., 3.)], stroke);
            }
        }
        _ => {}
    }
    response.on_hover_text(label)
}

/// A presentation plan over authoritative worker edits. Each edit appears once,
/// even when syntax metadata is absent, and retains its original global index.
pub(super) struct Model {
    pub rows: Vec<Row>,
    pub first_reference: Option<usize>,
}
pub(super) enum Row {
    Group {
        key: String,
        label: String,
        indent: f32,
        icon: Option<String>,
        end: usize,
    },
    Edit {
        index: usize,
        file: usize,
        edit: usize,
        indent: f32,
    },
}
impl Model {
    fn group(&mut self, key: String, label: String, indent: f32, icon: Option<String>) -> usize {
        let index = self.rows.len();
        self.rows.push(Row::Group {
            key,
            label,
            indent,
            icon,
            end: index + 1,
        });
        index
    }
    fn close(&mut self, index: usize) {
        let end = self.rows.len();
        if let Row::Group { end: value, .. } = &mut self.rows[index] {
            *value = end;
        }
    }
    pub fn new(
        files: &[codisseu_core::RenamePreviewFile],
        root: Option<&std::path::Path>,
        kind: codisseu_core::RenameSymbolKind,
        old: &str,
        new: &str,
        file_rename: bool,
    ) -> Self {
        use codisseu_core::RenameOccurrenceKind as K;
        let root = root.unwrap_or(std::path::Path::new(""));
        let rust = !file_rename
            && !files.is_empty()
            && files
                .iter()
                .all(|file| file.path.extension().is_some_and(|ext| ext == "rs"));
        let mut result = Self {
            rows: vec![],
            first_reference: None,
        };
        let mut bases = Vec::with_capacity(files.len());
        let mut next = 0;
        for file in files {
            bases.push(next);
            next += file.edits.len();
        }
        let mut order: Vec<_> = (0..files.len()).collect();
        order.sort_by_key(|i| files[*i].path.to_string_lossy().to_lowercase());
        let category = |file: &codisseu_core::RenamePreviewFile, i: usize| match file
            .presentation
            .get(i)
            .map(|p| p.kind)
            .unwrap_or(K::Unknown)
        {
            K::Declaration => 0,
            K::SemanticReference => 1,
            K::Comment | K::String | K::PlainText => 2,
            K::Unknown => 3,
        };
        let symbol = if file_rename {
            "file"
        } else {
            match kind {
                codisseu_core::RenameSymbolKind::Property => "Field",
                codisseu_core::RenameSymbolKind::Class => "Class",
                codisseu_core::RenameSymbolKind::Function => "Function",
                codisseu_core::RenameSymbolKind::Method => "Method",
                codisseu_core::RenameSymbolKind::Parameter => "Parameter",
                _ => "Symbol",
            }
        };
        for group in 0..4 {
            let count: usize = files
                .iter()
                .map(|f| {
                    (0..f.edits.len())
                        .filter(|i| category(f, *i) == group)
                        .count()
                })
                .sum();
            if count == 0 {
                continue;
            }
            let title = match group {
                0 => format!("{symbol} to be renamed to {new}"),
                1 => format!("References in code to {symbol} {old} ({count} references)"),
                2 => format!("Comments, and Text Occurrences ({count} occurrences)"),
                _ if rust => format!("Edições Rust ({count} ocorrências)"),
                _ => format!("Unclassified edits ({count} edits; syntax unavailable)"),
            };
            let top = result.group(
                format!("category:{group}"),
                title,
                34.,
                Some("@heading".into()),
            );
            let workspace = if group != 0 {
                let name = root.file_name().unwrap_or_default().to_string_lossy();
                Some(result.group(
                    format!("workspace:{group}"),
                    format!("{name}  {count} results"),
                    90.,
                    None,
                ))
            } else {
                None
            };
            let mut folder: Option<(std::path::PathBuf, usize)> = None;
            for file_index in &order {
                let file = &files[*file_index];
                let mut indices: Vec<_> = (0..file.edits.len())
                    .filter(|i| category(file, *i) == group)
                    .collect();
                if indices.is_empty() {
                    continue;
                }
                indices
                    .sort_by_key(|i| (file.edits[*i].start_line, file.edits[*i].start_character));
                let path = file.path.strip_prefix(root).unwrap_or(&file.path);
                if group == 0 {
                    for edit in indices {
                        result.rows.push(Row::Edit {
                            index: bases[*file_index] + edit,
                            file: *file_index,
                            edit,
                            indent: 70.,
                        });
                    }
                    continue;
                }
                let parent = path.parent().unwrap_or(std::path::Path::new(""));
                if folder.as_ref().is_none_or(|(p, _)| p != parent) {
                    if let Some((_, index)) = folder.take() {
                        result.close(index);
                    }
                    let name = if parent.as_os_str().is_empty() {
                        ".".into()
                    } else {
                        parent.display().to_string()
                    };
                    let index = result.group(
                        format!("folder:{group}:{}", parent.display()),
                        name,
                        109.,
                        None,
                    );
                    folder = Some((parent.to_path_buf(), index));
                }
                let file_group = result.group(
                    format!("file:{group}:{}", path.display()),
                    format!("{}  {} results", path.display(), indices.len()),
                    128.,
                    path.file_name().map(|s| s.to_string_lossy().into_owned()),
                );
                let mut contexts: Vec<(String, usize)> = vec![];
                for edit in indices {
                    let enclosing = file
                        .presentation
                        .get(edit)
                        .map(|p| p.enclosing.as_slice())
                        .unwrap_or(&[]);
                    let keys: Vec<_> = enclosing
                        .iter()
                        .map(|e| format!("context:{group}:{file_index}:{:?}:{:?}", e.kind, e.range))
                        .collect();
                    let common = keys
                        .iter()
                        .zip(&contexts)
                        .take_while(|(key, (previous, _))| *key == previous)
                        .count();
                    while contexts.len() > common {
                        let (_, index) = contexts.pop().unwrap();
                        result.close(index);
                    }
                    for (depth, (key, context)) in
                        keys.iter().zip(enclosing).enumerate().skip(common)
                    {
                        let label = context.name.clone();
                        let index = result.group(
                            key.clone(),
                            label,
                            147. + 19. * depth as f32,
                            Some(format!("@{:?}", context.kind)),
                        );
                        contexts.push((key.clone(), index));
                    }
                    let global = bases[*file_index] + edit;
                    if group == 1 && result.first_reference.is_none() {
                        result.first_reference = Some(global);
                    }
                    result.rows.push(Row::Edit {
                        index: global,
                        file: *file_index,
                        edit,
                        indent: 179. + 19. * enclosing.len().saturating_sub(1) as f32,
                    });
                }
                while let Some((_, index)) = contexts.pop() {
                    result.close(index);
                }
                result.close(file_group);
            }
            if let Some((_, index)) = folder {
                result.close(index);
            }
            if let Some(index) = workspace {
                result.close(index);
            }
            result.close(top);
        }
        result
    }
}

pub(super) fn candidate_indices(rename: &codisseu_core::RenameState) -> Vec<usize> {
    if !rename.options().search_dynamic_references {
        return vec![];
    }
    rename
        .validation
        .as_ref()
        .map(|report| {
            report
                .issues
                .iter()
                .enumerate()
                .filter(|(_, issue)| {
                    issue.range.is_some()
                        && matches!(
                            issue.kind,
                            codisseu_core::IssueKind::UnsupportedClass
                                | codisseu_core::IssueKind::UnsupportedEdit
                        )
                })
                .map(|(index, _)| index)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::{
        RenameEnclosingKind as E, RenameEnclosingSymbol, RenameOccurrenceKind as K,
        RenameOccurrencePresentation as P, RenamePreviewFile, RenameTextEdit, TextRange,
    };
    #[test]
    fn presentation_preserves_edits_separates_declarations_and_uses_and_nests_ast_contexts() {
        let range = TextRange {
            start_line: 1,
            start_character: 0,
            end_line: 9,
            end_character: 0,
        };
        let edit = |line| RenameTextEdit {
            start_line: line,
            start_character: 2,
            end_line: line,
            end_character: 5,
            new_text: "new".into(),
        };
        let context = vec![
            RenameEnclosingSymbol {
                kind: E::Class,
                name: "Weather".into(),
                range: range.clone(),
            },
            RenameEnclosingSymbol {
                kind: E::Method,
                name: "lire_Δ".into(),
                range,
            },
        ];
        let present = |kind, enclosing| P {
            kind,
            enclosing,
            unknown_reason: None,
        };
        let files = vec![RenamePreviewFile {
            encoding: codisseu_core::FileEncoding::Utf8,
            path: "/workspace/Weather.ts".into(),
            before: "".into(),
            after: "".into(),
            edits: vec![edit(2), edit(4), edit(8), edit(10), edit(11)],
            presentation: vec![
                present(K::Declaration, vec![]),
                present(K::SemanticReference, context),
                present(K::SemanticReference, vec![]),
                present(K::Comment, vec![]),
            ],
        }];
        let model = Model::new(
            &files,
            Some(std::path::Path::new("/workspace")),
            codisseu_core::RenameSymbolKind::Property,
            "old",
            "new",
            false,
        );
        let mut indexes: Vec<_> = model
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Edit { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        indexes.sort_unstable();
        assert_eq!(indexes, vec![0, 1, 2, 3, 4]);
        assert_eq!(model.first_reference, Some(1));
        let labels: Vec<_> = model
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Group { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"Field to be renamed to new"));
        assert!(labels.iter().any(|l| l.contains("(2 references)")));
        assert!(labels.iter().any(|l| l.contains("(1 occurrences)")));
        assert!(labels.iter().any(|l| l.contains("syntax unavailable")));
        let class = model
            .rows
            .iter()
            .position(|r| matches!(r,Row::Group{label,..}if label=="Weather"))
            .unwrap();
        let method = model
            .rows
            .iter()
            .position(|r| matches!(r,Row::Group{label,..}if label=="lire_Δ"))
            .unwrap();
        assert!(class < method);
        let Row::Group { end, .. } = model.rows[class] else {
            panic!()
        };
        assert!(end > method);
        assert!(
            model.rows[method..end]
                .iter()
                .any(|r| matches!(r, Row::Edit { index: 1, .. }))
        );
        assert!(
            !model.rows[method..end]
                .iter()
                .any(|r| matches!(r, Row::Edit { index: 2, .. }))
        );
    }
}
