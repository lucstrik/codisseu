//! Native egui painting, clipped block layout and explicit user interactions.
use super::{
    parse::{Block, BlockKind, LinkTarget, Parsed, Span},
    *,
};
use std::{collections::HashMap, sync::Arc};

#[derive(Default)]
pub(super) struct RenderState {
    stamp: Option<(DocumentId, u64)>,
    width: u32,
    scale: u32,
    theme: Option<codisseu_core::ThemePreference>,
    heights: Vec<f32>,
    layouts: HashMap<(usize, usize), Arc<egui::Galley>>,
    textures: HashMap<String, egui::TextureHandle>,
    last_offset: Option<f32>,
    table_heights: HashMap<(usize, usize), f32>,
}
impl RenderState {
    #[cfg(test)]
    pub(super) fn metrics(&self) -> (usize, Option<f32>) {
        (self.layouts.len(), self.last_offset)
    }
    pub(super) fn clear_layout(&mut self) {
        self.layouts.clear();
        self.heights.clear();
        self.width = 0;
        self.table_heights.clear();
    }
    pub(super) fn release_resources(&mut self) {
        self.clear_layout();
        self.textures.clear();
    }
}
#[allow(clippy::too_many_arguments)]
pub(super) fn show(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    view: ViewId,
    id: DocumentId,
    version: u64,
    parsed: &Parsed,
    state: &mut RenderState,
    zoom: f32,
    sync: bool,
    target: Option<usize>,
) -> Vec<MarkdownAction> {
    if state.stamp != Some((id, version)) {
        state.stamp = Some((id, version));
        state.clear_layout();
        state.textures.clear();
        state.last_offset = None;
    }
    let width = (ui.available_width() - 64.).clamp(80., 980.);
    let scale = ui.ctx().pixels_per_point().to_bits();
    if state.width != width.to_bits() || state.scale != scale || state.theme != Some(tokens.theme) {
        state.theme = Some(tokens.theme);
        state.width = width.to_bits();
        state.scale = scale;
        state.layouts.clear();
        state.heights = parsed
            .blocks
            .iter()
            .map(|b| estimate(b, width, zoom))
            .collect();
    }
    if state.heights.len() != parsed.blocks.len() {
        state.heights = parsed
            .blocks
            .iter()
            .map(|b| estimate(b, width, zoom))
            .collect();
    }
    let mut actions = Vec::new();
    let mut first_line = None;
    let mut scroll_target_applied = false;
    let output = egui::ScrollArea::vertical()
        .id_salt(("markdown.preview", view, id))
        .auto_shrink([false, false])
        .codisseu()
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.;
            ui.add_space(24.);
            let margin = ((ui.available_width() - width) / 2.).max(12.);
            for (index, block) in parsed.blocks.iter().enumerate() {
                let indent = (block.indent + block.quote) as f32 * 20.;
                let estimated = state.heights[index];
                let rect = egui::Rect::from_min_size(
                    ui.next_widget_position(),
                    egui::vec2(ui.available_width(), estimated),
                );
                let visible = rect.intersects(ui.clip_rect());
                if visible && first_line.is_none() {
                    first_line = Some(block.line);
                }
                if !visible {
                    let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
                    if target == Some(index) {
                        response.scroll_to_me(Some(egui::Align::Min));
                        scroll_target_applied = true;
                    }
                    continue;
                }
                let start = ui.next_widget_position();
                let inner = ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    ui.add_space(margin + indent);
                    let available = (width - indent).max(24.);
                    ui.allocate_ui_with_layout(
                        egui::vec2(available, 0.),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 0.;
                            ui.set_max_width(available);
                            match &block.kind {
                                BlockKind::Rich {
                                    spans,
                                    heading,
                                    anchor,
                                    marker,
                                    task,
                                    loose,
                                } => {
                                    let body = ui.horizontal_top(|ui| {
                                        ui.spacing_mut().item_spacing.x = 6.;
                                        if let Some(task) = task {
                                            let mut checked = task.checked;
                                            let response = ui.add_enabled(
                                                !parsed.read_only,
                                                egui::Checkbox::new(&mut checked, ""),
                                            );
                                            response.widget_info(|| {
                                                egui::WidgetInfo::selected(
                                                    egui::accesskit::Role::CheckBox,
                                                    true,
                                                    checked,
                                                    format!(
                                                        "Markdown task line {}",
                                                        block.line + 1
                                                    ),
                                                )
                                            });
                                            if response.changed() {
                                                actions.push(MarkdownAction::ToggleTask {
                                                    id,
                                                    version,
                                                    byte_offset: task.byte_offset,
                                                    checked,
                                                });
                                            }
                                        } else if let Some(marker) = marker {
                                            ui.add_sized(
                                                [18., 20. * zoom],
                                                egui::Label::new(
                                                    RichText::new(marker)
                                                        .size(14. * zoom)
                                                        .color(color(tokens.text)),
                                                ),
                                            );
                                        }
                                        let available = ui.available_width().min(available);
                                        let galley = cached(ui, state, (index, 0), || {
                                            rich_job(spans, *heading, available, zoom, tokens)
                                        });
                                        let response = ui.add(
                                            egui::Label::new(galley.clone())
                                                .selectable(true)
                                                .sense(egui::Sense::click()),
                                        );
                                        if response.double_clicked() {
                                            actions.push(MarkdownAction::RevealSource {
                                                range: block.range.clone(),
                                                line: block.line,
                                            });
                                        }
                                        links(
                                            ui,
                                            spans,
                                            &galley,
                                            response.rect,
                                            (view, index, 0),
                                            &mut actions,
                                        );
                                        if let Some(anchor) = anchor {
                                            response.on_hover_text(format!(
                                                "#{anchor} · double-click to show source"
                                            ));
                                        }
                                    });
                                    if *heading > 0 && *heading <= 2 {
                                        ui.add_space(5.);
                                        ui.painter().hline(
                                            body.response.rect.x_range(),
                                            ui.next_widget_position().y,
                                            egui::Stroke::new(1., color(tokens.border)),
                                        );
                                    }
                                    ui.add_space(if *heading > 0 {
                                        18. * zoom
                                    } else if (marker.is_some() || task.is_some()) && !loose {
                                        0.
                                    } else {
                                        12. * zoom
                                    });
                                }
                                BlockKind::Code {
                                    language,
                                    text,
                                    job,
                                } => {
                                    egui::Frame::new()
                                        .fill(color(tokens.panel))
                                        .inner_margin(10)
                                        .corner_radius(4)
                                        .show(ui, |ui| {
                                            ui.set_max_width((available - 20.).max(20.));
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(language)
                                                        .small()
                                                        .color(color(tokens.muted)),
                                                );
                                                if ui.small_button("Copy code").clicked() {
                                                    ui.ctx().copy_text(text.clone());
                                                }
                                            });
                                            let galley = cached(ui, state, (index, 0), || {
                                                let mut job = if tokens.theme
                                                    == codisseu_core::ThemePreference::Classic
                                                {
                                                    job.clone()
                                                } else {
                                                    documentation_code_job(
                                                        text,
                                                        language,
                                                        "",
                                                        f32::INFINITY,
                                                        tokens,
                                                    )
                                                };
                                                for section in &mut job.sections {
                                                    section.format.font_id.size = 13. * zoom;
                                                }
                                                job.wrap.max_width = (available - 20.).max(20.);
                                                job
                                            });
                                            let response = ui.add(
                                                egui::Label::new(galley)
                                                    .selectable(true)
                                                    .sense(egui::Sense::click()),
                                            );
                                            if response.double_clicked() {
                                                actions.push(MarkdownAction::RevealSource {
                                                    range: block.range.clone(),
                                                    line: block.line,
                                                });
                                            }
                                        });
                                    ui.add_space(14.);
                                }
                                BlockKind::Table { align, rows } => {
                                    // Fixed column widths keep horizontal density bounded. Long text wraps.
                                    let columns = align.len().max(1);
                                    let cell_width = ((available - 2.) / columns as f32).max(14.);
                                    let line = egui::Stroke::new(1., color(tokens.border));
                                    for (row_index, row) in rows.iter().enumerate() {
                                        let estimated = *state
                                            .table_heights
                                            .get(&(index, row_index))
                                            .unwrap_or(&(21. * zoom + 12.));
                                        let bounds = egui::Rect::from_min_size(
                                            ui.next_widget_position(),
                                            egui::vec2(available, estimated),
                                        );
                                        if !bounds.intersects(ui.clip_rect()) {
                                            ui.allocate_exact_size(
                                                bounds.size(),
                                                egui::Sense::hover(),
                                            );
                                            continue;
                                        }
                                        let cells: Vec<_> = row
                                            .iter()
                                            .enumerate()
                                            .map(|(column, spans)| {
                                                cached(
                                                    ui,
                                                    state,
                                                    (index, 1 + row_index * columns + column),
                                                    || {
                                                        rich_job(
                                                            spans,
                                                            if row_index == 0 { 7 } else { 0 },
                                                            (cell_width - 14.).max(8.),
                                                            zoom,
                                                            tokens,
                                                        )
                                                    },
                                                )
                                            })
                                            .collect();
                                        let height = cells
                                            .iter()
                                            .map(|galley| galley.size().y + 12.)
                                            .fold(21. * zoom + 12., f32::max);
                                        let (row_rect, _) = ui.allocate_exact_size(
                                            egui::vec2(available, height),
                                            egui::Sense::hover(),
                                        );
                                        for (column, (spans, galley)) in
                                            row.iter().zip(cells).enumerate()
                                        {
                                            let cell = egui::Rect::from_min_size(
                                                row_rect.min
                                                    + egui::vec2(column as f32 * cell_width, 0.),
                                                egui::vec2(cell_width, height),
                                            );
                                            let x = match align.get(column) {
                                                Some(pulldown_cmark::Alignment::Right) => {
                                                    cell.right() - 7. - galley.size().x
                                                }
                                                Some(pulldown_cmark::Alignment::Center) => {
                                                    cell.center().x - galley.size().x / 2.
                                                }
                                                _ => cell.left() + 7.,
                                            };
                                            let text = egui::Rect::from_min_size(
                                                egui::pos2(x, cell.top() + 6.),
                                                galley.size(),
                                            );
                                            let response = ui.put(
                                                text,
                                                egui::Label::new(galley.clone()).selectable(true),
                                            );
                                            links(
                                                ui,
                                                spans,
                                                &galley,
                                                response.rect,
                                                (view, index, 1 + row_index * columns + column),
                                                &mut actions,
                                            );
                                            ui.painter().vline(cell.right(), cell.y_range(), line);
                                        }
                                        state.table_heights.insert((index, row_index), height);
                                        ui.painter().hline(
                                            row_rect.x_range(),
                                            row_rect.bottom(),
                                            line,
                                        );
                                        ui.painter().vline(
                                            row_rect.left(),
                                            row_rect.y_range(),
                                            line,
                                        );
                                    }
                                    ui.add_space(16.);
                                }
                                BlockKind::Image {
                                    alt,
                                    destination,
                                    target: _,
                                } => {
                                    if parsed.read_only
                                        && let Some(name) = alt.strip_prefix("codisseu-avatar:")
                                    {
                                        let texture = parsed
                                            .images
                                            .get(destination)
                                            .and_then(|i| i.as_ref().ok())
                                            .filter(|i| crate::jira_images::has_detail(i))
                                            .map(|image| {
                                                state
                                                    .textures
                                                    .entry(destination.clone())
                                                    .or_insert_with(|| {
                                                        ui.ctx().load_texture(
                                                            "jira.author",
                                                            image.as_ref().clone(),
                                                            egui::TextureOptions::LINEAR,
                                                        )
                                                    })
                                                    .id()
                                            });
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.;
                                            crate::jira_images::avatar(
                                                ui, texture, name, 26., tokens,
                                            );
                                            ui.label(egui::RichText::new(name).strong());
                                        });
                                        ui.add_space(8.);
                                        return;
                                    }
                                    match parsed.images.get(destination) {
                                        Some(Ok(image)) => {
                                            let texture = state
                                                .textures
                                                .entry(destination.clone())
                                                .or_insert_with(|| {
                                                    ui.ctx().load_texture(
                                                        format!(
                                                            "markdown.{id}.{version}.{destination}"
                                                        ),
                                                        image.as_ref().clone(),
                                                        egui::TextureOptions::LINEAR,
                                                    )
                                                });
                                            let original = texture.size_vec2();
                                            let size = original * (available / original.x).min(1.);
                                            let response = ui
                                                .add(
                                                    egui::Image::from_texture((
                                                        texture.id(),
                                                        original,
                                                    ))
                                                    .fit_to_exact_size(size)
                                                    .sense(egui::Sense::click()),
                                                )
                                                .on_hover_text(alt);
                                            response.widget_info(|| {
                                                egui::WidgetInfo::labeled(
                                                    egui::accesskit::Role::Image,
                                                    true,
                                                    format!("Markdown image {alt}"),
                                                )
                                            });
                                            if response.double_clicked() {
                                                actions.push(MarkdownAction::RevealSource {
                                                    range: block.range.clone(),
                                                    line: block.line,
                                                });
                                            }
                                        }
                                        other => {
                                            let message = other
                                                .and_then(|image| image.as_ref().err())
                                                .map(String::as_str)
                                                .unwrap_or("Image is outside the preview budget");
                                            ui.label(
                                                RichText::new(format!("[Image: {alt}]\n{message}"))
                                                    .color(color(tokens.muted)),
                                            );
                                        }
                                    }
                                    ui.add_space(14.);
                                }
                                BlockKind::Rule => {
                                    ui.separator();
                                    ui.add_space(16.);
                                }
                            }
                        },
                    );
                });
                let measured = (ui.next_widget_position().y - start.y).max(1.);
                state.heights[index] = measured;
                if block.quote > 0 {
                    ui.painter().vline(
                        start.x + margin + indent - 12.,
                        start.y..=start.y + measured - 8.,
                        egui::Stroke::new(3., color(tokens.border)),
                    );
                }
                if target == Some(index) {
                    inner.response.scroll_to_me(Some(egui::Align::Min));
                    scroll_target_applied = true;
                }
            }
            ui.add_space(32.);
        });
    let moved = state
        .last_offset
        .is_some_and(|old| (old - output.state.clamped_offset().y).abs() > 0.5);
    let user_scroll = ui
        .ctx()
        .input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO || i.pointer.any_down());
    if sync
        && moved
        && user_scroll
        && !scroll_target_applied
        && let Some(line) = first_line
    {
        actions.push(MarkdownAction::ScrollSource { line });
    }
    state.last_offset = Some(output.state.clamped_offset().y);
    actions
}
fn estimate(block: &Block, width: f32, zoom: f32) -> f32 {
    let line = 21. * zoom;
    match &block.kind {
        BlockKind::Rich { spans, heading, .. } => {
            let count = spans.iter().map(|s| s.text.chars().count()).sum::<usize>();
            let per = (width / (7. * zoom)).max(8.);
            ((count as f32 / per).ceil().max(1.) * line * if *heading > 0 { 1.4 } else { 1. }) + 16.
        }
        BlockKind::Code { text, .. } => text.lines().count().max(1) as f32 * 18. * zoom + 54.,
        BlockKind::Table { rows, .. } => rows.len() as f32 * (line + 12.) + 16.,
        BlockKind::Image { alt, .. } if alt.starts_with("codisseu-avatar:") => 38.,
        BlockKind::Image { .. } => 200.,
        BlockKind::Rule => 22.,
    }
}
fn cached(
    ui: &egui::Ui,
    state: &mut RenderState,
    key: (usize, usize),
    build: impl FnOnce() -> egui::text::LayoutJob,
) -> Arc<egui::Galley> {
    if let Some(galley) = state.layouts.get(&key) {
        return galley.clone();
    }
    let galley = ui.fonts_mut(|f| f.layout_job(build()));
    let bytes = |galley: &egui::Galley| {
        galley.num_vertices * std::mem::size_of::<egui::epaint::Vertex>()
            + galley.num_indices * 4
            + galley.job.text.len()
    };
    let cost = bytes(&galley);
    while state.layouts.len() >= 256
        || state.layouts.values().map(|g| bytes(g)).sum::<usize>() + cost > 16 * 1024 * 1024
    {
        let Some(old) = state.layouts.keys().next().copied() else {
            break;
        };
        state.layouts.remove(&old);
    }
    state.layouts.insert(key, galley.clone());
    galley
}
fn rich_job(
    spans: &[Span],
    heading: u8,
    width: f32,
    zoom: f32,
    tokens: &Tokens,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let size = match heading {
        1 => 28.,
        2 => 21.,
        3 => 20.,
        4 => 18.,
        5 => 16.,
        6 => 14.,
        _ => 14.,
    } * zoom;
    for span in spans {
        let foreground = color(if span.link.is_some() {
            tokens.accent
        } else {
            tokens.text
        });
        let mut format = egui::TextFormat {
            font_id: if span.style.code {
                FontId::monospace(size * 0.94)
            } else {
                FontId::proportional(size)
            },
            color: foreground,
            italics: span.style.emphasis,
            line_height: Some(
                size * if (1..=6).contains(&heading) {
                    1.25
                } else {
                    1.6
                },
            ),
            ..Default::default()
        };
        if span.style.strong || heading > 0 {
            format.font_id.family = egui::FontFamily::Name(
                if span.style.code {
                    "codisseu.mono.bold"
                } else {
                    "codisseu.inter.bold"
                }
                .into(),
            );
        }
        if span.style.code {
            format.background = color(tokens.panel);
        }
        if span.style.strike {
            format.strikethrough = egui::Stroke::new(1., foreground);
        }
        if span.link.is_some() {
            format.underline = egui::Stroke::new(0.7, foreground);
        }
        job.append(&span.text, 0., format);
    }
    job
}
fn links(
    ui: &mut egui::Ui,
    spans: &[Span],
    galley: &egui::Galley,
    rect: egui::Rect,
    id: (ViewId, usize, usize),
    actions: &mut Vec<MarkdownAction>,
) {
    let mut start = 0;
    for (span_index, span) in spans.iter().enumerate() {
        let end = start + span.text.chars().count();
        if let Some(target) = span.link.as_deref() {
            let mut row_start = 0;
            for (row_index, row) in galley.rows.iter().enumerate() {
                let row_end = row_start + row.char_count_excluding_newline().0;
                let first = start.max(row_start);
                let last = end.min(row_end);
                if first < last {
                    let a = &row.glyphs[first - row_start];
                    let b = &row.glyphs[last - row_start - 1];
                    let link_rect = egui::Rect::from_min_max(
                        rect.min + row.pos.to_vec2() + egui::vec2(a.pos.x, 0.),
                        rect.min
                            + row.pos.to_vec2()
                            + egui::vec2(b.pos.x + b.advance_width, row.size.y),
                    );
                    let response = ui.interact(
                        link_rect,
                        ui.make_persistent_id(("markdown.link", id, span_index, row_index)),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::Link,
                            !matches!(target, LinkTarget::Blocked(_)),
                            &span.text,
                        )
                    });
                    let hint = match target {
                        LinkTarget::Anchor(a) => format!("#{a}"),
                        LinkTarget::Local { path, .. } => path.display().to_string(),
                        LinkTarget::External(url) => format!("Open in browser: {url}"),
                        LinkTarget::Blocked(reason) => reason.clone(),
                    };
                    if response.clicked() {
                        activate(target, actions);
                    }
                    response
                        .on_hover_text(hint)
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                }
                row_start += row.char_count_including_newline().0;
            }
        }
        start = end;
    }
}
fn activate(target: &LinkTarget, actions: &mut Vec<MarkdownAction>) {
    match target {
        LinkTarget::Anchor(anchor) => actions.push(MarkdownAction::OpenLocal {
            path: PathBuf::new(),
            anchor: Some(anchor.clone()),
        }),
        LinkTarget::Local { path, anchor } => actions.push(MarkdownAction::OpenLocal {
            path: path.clone(),
            anchor: anchor.clone(),
        }),
        LinkTarget::External(url) => {
            actions.push(MarkdownAction::OpenExternal { url: url.clone() })
        }
        LinkTarget::Blocked(_) => {}
    }
}

/// Read-only documentation styling over the shared Markdown model. No link,
/// image, task or source-navigation actions belong to this presentation.
pub(super) fn show_documentation(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    version: u64,
    parsed: &Parsed,
    state: &mut RenderState,
    extension: &str,
) {
    let width = ui.available_width().max(1.);
    let style_key = ui.ctx().pixels_per_point().to_bits()
        ^ tokens.ui_font_size.to_bits().rotate_left(7)
        ^ tokens.editor_font_size.to_bits().rotate_left(13);
    if state.stamp != Some((u64::MAX, version))
        || state.width != width.to_bits()
        || state.scale != style_key
        || state.theme != Some(tokens.theme)
    {
        state.stamp = Some((u64::MAX, version));
        state.clear_layout();
        state.width = width.to_bits();
        state.scale = style_key;
        state.theme = Some(tokens.theme);
        state.heights = parsed
            .blocks
            .iter()
            .map(|b| estimate(b, width, 1.))
            .collect();
    }
    ui.spacing_mut().item_spacing.y = tokens.documentation_block_gap;
    for (index, block) in parsed.blocks.iter().enumerate() {
        let estimated = state.heights[index];
        let rect =
            egui::Rect::from_min_size(ui.next_widget_position(), egui::vec2(width, estimated));
        if !rect.intersects(ui.clip_rect()) {
            ui.allocate_exact_size(rect.size(), egui::Sense::hover());
            continue;
        }
        let top = ui.next_widget_position().y;
        match &block.kind {
            BlockKind::Rich {
                spans,
                heading,
                marker,
                ..
            } => {
                let galley = cached(ui, state, (index, 0), || {
                    documentation_rich_job(
                        spans,
                        *heading,
                        marker.as_deref(),
                        width,
                        tokens,
                        extension,
                    )
                });
                ui.add(egui::Label::new(galley).selectable(true));
            }
            BlockKind::Code { language, text, .. } => {
                egui::Frame::new()
                    .fill(color(tokens.panel))
                    .corner_radius(tokens.control_corner_radius)
                    .inner_margin(tokens.documentation_code_padding)
                    .show(ui, |ui| {
                        let available =
                            (width - 2. * tokens.documentation_code_padding as f32).max(1.);
                        ui.set_width(available);
                        let galley = cached(ui, state, (index, 0), || {
                            documentation_code_job(text, language, extension, available, tokens)
                        });
                        ui.add(egui::Label::new(galley).selectable(true));
                    });
            }
            BlockKind::Table { rows, .. } => {
                let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                egui::Grid::new(("documentation.table", index))
                    .num_columns(columns)
                    .spacing(egui::vec2(8., 6.))
                    .show(ui, |ui| {
                        for (row, cells) in rows.iter().enumerate() {
                            for (column, spans) in cells.iter().enumerate() {
                                let available =
                                    ((width - 8. * (columns - 1) as f32) / columns as f32).max(1.);
                                let galley =
                                    cached(ui, state, (index, 1 + row * columns + column), || {
                                        documentation_rich_job(
                                            spans,
                                            u8::from(row == 0),
                                            None,
                                            available,
                                            tokens,
                                            extension,
                                        )
                                    });
                                ui.add(egui::Label::new(galley).selectable(true));
                            }
                            ui.end_row();
                        }
                    });
            }
            BlockKind::Image { alt, .. } => {
                ui.label(RichText::new(alt).color(color(tokens.muted)));
            }
            BlockKind::Rule => {
                ui.separator();
            }
        }
        state.heights[index] = (ui.next_widget_position().y - top).max(1.);
    }
}
fn documentation_code_job(
    text: &str,
    language: &str,
    fallback: &str,
    width: f32,
    tokens: &Tokens,
) -> egui::text::LayoutJob {
    let language = language.to_ascii_lowercase();
    let extension = match language.as_str() {
        "javascript" => "js",
        "typescript" => "ts",
        "postgresql" | "mysql" => "sql",
        "" => fallback,
        other => other,
    };
    let mut job = crate::highlight::layout_themed(
        text,
        extension,
        FontId::monospace(tokens.editor_font_size),
        tokens.theme,
    );
    job.wrap.max_width = width;
    job.wrap.break_anywhere = false;
    for section in &mut job.sections {
        section.format.line_height = Some(tokens.documentation_line_height);
    }
    job
}
fn documentation_rich_job(
    spans: &[Span],
    heading: u8,
    marker: Option<&str>,
    width: f32,
    tokens: &Tokens,
    extension: &str,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.wrap.break_anywhere = false;
    if let Some(marker) = marker {
        job.append(
            &format!("{marker}  "),
            0.,
            egui::TextFormat {
                font_id: FontId::proportional(tokens.ui_font_size),
                color: color(tokens.muted),
                ..Default::default()
            },
        );
    }
    for span in spans {
        if span.style.code {
            let code = documentation_code_job(&span.text, extension, extension, width, tokens);
            for section in code.sections {
                let mut format = section.format;
                format.background = color(tokens.panel);
                job.append(
                    &span.text[usize::from(section.byte_range.start)
                        ..usize::from(section.byte_range.end)],
                    0.,
                    format,
                );
            }
        } else {
            let mut format = egui::TextFormat {
                font_id: FontId::proportional(tokens.ui_font_size),
                color: color(if heading > 0 || span.link.is_some() {
                    tokens.accent
                } else {
                    tokens.text
                }),
                italics: span.style.emphasis,
                line_height: Some(tokens.documentation_line_height),
                ..Default::default()
            };
            if heading > 0 || span.style.strong {
                format.font_id.family = egui::FontFamily::Name("codisseu.inter.bold".into());
            }
            if span.style.strike {
                format.strikethrough = egui::Stroke::new(1., format.color);
            }
            job.append(&span.text, 0., format);
        }
    }
    job
}

#[cfg(test)]
mod documentation_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn theme_changes_recolor_cached_documentation_without_reparsing_source() {
        let source = "```ts\nconst answer = 42;\n```";
        let parsed = parse::parse_remote(source, &AtomicBool::new(false)).unwrap();
        let ctx = egui::Context::default();
        let classic = Tokens::for_theme(codisseu_core::ThemePreference::Classic);
        let midnight = Tokens::for_theme(codisseu_core::ThemePreference::Midnight);
        classic.apply(&ctx);
        let mut state = RenderState::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            show_documentation(ui, &classic, 1, &parsed, &mut state, "ts");
            let old = state
                .layouts
                .values()
                .find(|g| g.job.text.starts_with("const"))
                .unwrap()
                .clone();
            show_documentation(ui, &midnight, 1, &parsed, &mut state, "ts");
            let new = state
                .layouts
                .values()
                .find(|g| g.job.text.starts_with("const"))
                .unwrap();
            assert!(!Arc::ptr_eq(&old, new));
            assert_ne!(
                old.job.sections[0].format.color,
                new.job.sections[0].format.color
            );
            assert_eq!(old.job.text, new.job.text);
        });
        output.textures_delta.clear();
    }
    #[test]
    fn documentation_renders_markdown_and_exact_editor_code_colors_at_both_scales() {
        let code = "<?php public function aplicar(string $name, int $repeat = 2): void;\n";
        let text = format!(
            "### AplicarConfiguracaoService # aplicar\n\nAplica as configurações aprovadas.\n\n**Return** `void`\n\n```php\n{code}```\n"
        );
        for scale in [1., 1.25] {
            let parsed = parse::parse_remote(&text, &AtomicBool::new(false)).unwrap();
            let tokens = Tokens::load();
            let mut h = Harness::builder()
                .with_context_setup(|ctx| Tokens::load().apply(ctx))
                .with_size([500., 500.])
                .with_pixels_per_point(scale)
                .build_ui_state(
                    move |ui, state: &mut (RenderState, Parsed)| {
                        show_documentation(ui, &tokens, 1, &state.1, &mut state.0, "php");
                    },
                    (RenderState::default(), parsed),
                );
            h.run();
            assert!(
                h.query_by_label("AplicarConfiguracaoService # aplicar")
                    .is_some()
            );
            assert!(
                h.query_by_label("Aplica as configurações aprovadas.")
                    .is_some()
            );
            assert!(h.query_by_label_contains("```php").is_none());
            assert!(h.query_by_label_contains("**Return**").is_none());
            let actual = &h
                .state()
                .0
                .layouts
                .values()
                .find(|g| g.job.text == code)
                .expect("real code widget")
                .job;
            let expected = crate::highlight::layout_themed(
                code,
                "php",
                FontId::monospace(Tokens::load().editor_font_size),
                Tokens::load().theme,
            );
            let colors = |job: &egui::text::LayoutJob| {
                job.sections
                    .iter()
                    .map(|s| {
                        (
                            s.byte_range.clone(),
                            s.format.color,
                            s.format.font_id.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                colors(actual),
                colors(&expected),
                "syntax and font must match the editor, not a separate palette"
            );
            assert!(
                actual
                    .sections
                    .iter()
                    .any(|s| s.format.color != actual.sections[0].format.color)
            );
        }
    }
    #[test]
    fn inline_code_and_language_aliases_keep_source_and_shared_palette() {
        let tokens = Tokens::load();
        let parsed =
            parse::parse_remote("**Parâmetro:** `string $name`", &AtomicBool::new(false)).unwrap();
        let BlockKind::Rich { spans, .. } = &parsed.blocks[0].kind else {
            panic!("rich paragraph")
        };
        let job = documentation_rich_job(spans, 0, None, 300., &tokens, "php");
        assert_eq!(job.text, "Parâmetro: string $name");
        assert!(
            job.sections
                .iter()
                .any(|s| s.format.background == color(tokens.panel))
        );
        for (language, extension, code) in [
            ("TypeScript", "ts", "const answer: number = 42;"),
            (
                "javascript",
                "js",
                "function greet(name) { return 'Olá 😀'; }",
            ),
            ("mysql", "sql", "SELECT * FROM users WHERE id = 2;"),
        ] {
            let actual = documentation_code_job(code, language, "php", 260., &tokens);
            let expected = crate::highlight::layout_themed(
                code,
                extension,
                FontId::monospace(tokens.editor_font_size),
                tokens.theme,
            );
            assert_eq!(actual.text, code);
            assert_eq!(
                actual
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect::<Vec<_>>(),
                expected
                    .sections
                    .iter()
                    .map(|s| s.format.color)
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn documentation_images_and_links_remain_inert() {
        let parsed=parse::parse_remote("[Reference](https://example.invalid)\n\n![local](file:///etc/passwd)\n\n![remote](https://example.invalid/image.png)\n\n<script>alert(1)</script>",&AtomicBool::new(false)).unwrap();
        assert!(parsed.images.values().all(Result::is_err));
        let tokens = Tokens::load();
        let mut h = Harness::builder()
            .with_context_setup(|ctx| Tokens::load().apply(ctx))
            .with_size([500., 500.])
            .build_ui_state(
                move |ui, state: &mut (RenderState, Parsed)| {
                    show_documentation(ui, &tokens, 1, &state.1, &mut state.0, "php")
                },
                (RenderState::default(), parsed),
            );
        h.run();
        assert!(h.query_by_role(egui::accesskit::Role::Link).is_none());
        assert!(h.query_by_role(egui::accesskit::Role::Image).is_none());
        assert!(h.query_by_label_contains("alert(1)").is_some());
    }
}
