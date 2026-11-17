//! One bounded lexical candidate, resolved asynchronously by the PHP provider.
//! This module never parses a project or reads a file on the rendering thread.
use super::*;
use std::ops::Range;

const DELAY: f64 = 0.15;
const MAX_IDENTIFIER: usize = 256;

pub(super) struct Hover {
    version: u64,
    range: Range<usize>,
    since: f64,
    requested: bool,
}

fn identifier(c: char) -> bool {
    c == '_' || c == '$' || c.is_alphanumeric() || ('\u{300}'..='\u{36f}').contains(&c)
}

/// A candidate is never evidence of a semantic link. Whitespace and punctuation
/// do not snap to the preceding identifier, including the right half of a glyph.
fn candidate(snapshot: &TextSnapshot, at: usize) -> Option<Range<usize>> {
    if at >= snapshot.len_chars() || !identifier(snapshot.char(at)) {
        return None;
    }
    let mut start = at;
    let mut end = at + 1;
    while start > 0 && identifier(snapshot.char(start - 1)) {
        start -= 1;
        if end - start > MAX_IDENTIFIER {
            return None;
        }
    }
    while end < snapshot.len_chars() && identifier(snapshot.char(end)) {
        end += 1;
        if end - start > MAX_IDENTIFIER {
            return None;
        }
    }
    let first = snapshot.char(start);
    (first == '_' || first == '$' || first.is_alphabetic()).then_some(start..end)
}

/// Read-only identifier hit-testing shared with documentation. The source map
/// excludes ghost-only rows and uses actual shaped glyphs instead of a width
/// approximation; hovering never changes wrap affinity or the selection.
#[allow(clippy::too_many_arguments)]
pub(super) fn documentation_target(
    ui: &egui::Ui,
    view: &mut EditorView,
    response: &egui::Response,
    snapshot: &TextSnapshot,
    rect: egui::Rect,
    origin: egui::Pos2,
    ext: &str,
    t: &Tokens,
) -> Option<(usize, egui::Pos2)> {
    let point = ui.input(|input| {
        (!input.modifiers.any() && !input.pointer.any_down())
            .then(|| input.pointer.hover_pos())
            .flatten()
    })?;
    if view.composition.is_some()
        || view.drag_anchor.is_some()
        || !response.contains_pointer()
        || !rect.contains(point)
        || ui.ctx().layer_id_at(point) != Some(ui.layer_id())
    {
        return None;
    }
    let row = ((point.y - origin.y) / t.editor_line_height).floor();
    let total = if view.wrap.enabled {
        view.wrap.total(&view.fold)
    } else {
        view.fold.visible_line_count() + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
    };
    if row < 0. || row as usize >= total {
        return None;
    }
    let (line, x) = soft_wrap::pointer_position(view, row as usize, point.x - origin.x)?;
    let start = snapshot.line_to_char(line);
    let end = content_end(snapshot, line);
    let layout = view.cache.get(ui, snapshot, line, ext, t);
    if x < 0. || x >= layout.x(ui, snapshot, t, end - start)? {
        return None;
    }
    let mut column = layout.column(ui, snapshot, t, x)?;
    if layout.x(ui, snapshot, t, column)? > x {
        column = codisseu_core::previous_grapheme(snapshot, start + column) - start;
    }
    let range = candidate(snapshot, start + column)?;
    Some((range.start, point))
}

pub(super) fn clear(service: &mut AppService, view: &mut EditorView) {
    if view.link_hover.take().is_some() {
        let _ = service.dispatch(Command::CancelDefinitionPreview);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    ui: &egui::Ui,
    service: &mut AppService,
    view: &mut EditorView,
    response: &egui::Response,
    snapshot: &TextSnapshot,
    id: DocumentId,
    version: u64,
    rect: egui::Rect,
    origin: egui::Pos2,
    ext: &str,
    t: &Tokens,
    enabled: bool,
) {
    let (modifiers, p, now, down) = ui.input(|i| {
        (
            i.modifiers,
            i.pointer.hover_pos(),
            i.time,
            i.pointer.any_down(),
        )
    });
    let eligible = enabled
        && matches!(ext, "php" | "phtml")
        && (modifiers.ctrl || modifiers.command)
        && !modifiers.alt
        && !modifiers.shift
        && !down
        && view.composition.is_none()
        && view.drag_anchor.is_none()
        && response.contains_pointer()
        && ui.memory(|m| m.top_modal_layer().is_none());
    let range = eligible
        .then_some(p)
        .flatten()
        .filter(|p| rect.contains(*p) && ui.ctx().layer_id_at(*p) == Some(ui.layer_id()))
        .and_then(|p| {
            let row = ((p.y - origin.y) / t.editor_line_height).floor();
            if row < 0.
                || row as usize
                    >= if view.wrap.enabled {
                        view.wrap.total(&view.fold)
                    } else {
                        view.fold.visible_line_count()
                            + view.ghost.as_ref().map_or(0, ghost::Ghost::added_rows)
                    }
            {
                return None;
            }
            let (line, x) = soft_wrap::pointer_position(view, row as usize, p.x - origin.x)?;
            let start = snapshot.line_to_char(line);
            let end = content_end(snapshot, line);
            let layout = view.cache.get(ui, snapshot, line, ext, t);
            if x < 0. || x >= layout.x(ui, snapshot, t, end - start)? {
                return None;
            }
            let mut column = layout.column(ui, snapshot, t, x)?;
            if layout.x(ui, snapshot, t, column)? > x {
                column = codisseu_core::previous_grapheme(snapshot, start + column) - start;
            }
            candidate(snapshot, start + column)
        });
    let Some(range) = range else {
        clear(service, view);
        return;
    };
    if view
        .link_hover
        .as_ref()
        .is_none_or(|h| h.version != version || h.range != range)
    {
        clear(service, view);
        view.link_hover = Some(Hover {
            version,
            range,
            since: now,
            requested: false,
        });
    }
    let hover = view.link_hover.as_mut().unwrap();
    if !hover.requested {
        if now - hover.since >= DELAY {
            hover.requested = true;
            let _ = service.dispatch(Command::PreviewDefinition {
                id,
                expected_version: version,
                character_index: hover.range.start,
            });
        } else {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(
                (DELAY - (now - hover.since)).max(0.001),
            ));
        }
    }
}

pub(super) fn resolved(
    service: &AppService,
    view: &EditorView,
    id: DocumentId,
    version: u64,
) -> Option<Range<usize>> {
    let hover = view.link_hover.as_ref()?;
    let preview = service.navigation.preview.as_ref()?;
    (hover.version == version
        && preview.document_id == id
        && preview.version == version
        && preview.character_index == hover.range.start
        && !preview.targets.is_empty())
    .then(|| hover.range.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn candidates_are_bounded_and_do_not_snap_from_punctuation_or_whitespace() {
        let s = TextSnapshot::from_str("préço($service->handle()); 123");
        assert_eq!(candidate(&s, 2), Some(0..5));
        assert_eq!(candidate(&s, 8), Some(6..14));
        assert_eq!(candidate(&s, 18), Some(16..22));
        for at in [5, 14, 15, 22, 23, 24, 25, 26, 27, 28] {
            assert_eq!(candidate(&s, at), None, "{at}");
        }
        assert!(
            candidate(
                &TextSnapshot::from_str("a".repeat(100_000).as_str()),
                50_000
            )
            .is_none()
        );
    }
}
