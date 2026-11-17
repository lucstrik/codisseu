//! Thin, non-interactive gutter marks; breakpoint and fold hit targets are intact.
use super::super::{Tokens, color};
use codisseu_core::git::line_changes::{Change, Kind};
pub(super) fn paint(
    painter: &egui::Painter,
    changes: &[Change],
    line: usize,
    top: egui::Pos2,
    height: f32,
    tokens: &Tokens,
) {
    // Hunks are ordered. Avoid scanning a whole-file diff for each visible line.
    let first = changes.partition_point(|c| {
        if c.lines.is_empty() {
            c.lines.start.saturating_sub(1) < line
        } else {
            c.lines.end <= line
        }
    });
    for change in &changes[first..] {
        let anchor = change.lines.start.saturating_sub(1);
        if change.lines.start > line && (!change.lines.is_empty() || anchor > line) {
            break;
        }
        if change.lines.contains(&line) {
            painter.rect_filled(
                egui::Rect::from_min_size(top, egui::vec2(tokens.editor_git_width, height)),
                0.,
                color(match change.kind {
                    Kind::Added => tokens.editor_git_added,
                    Kind::Modified => tokens.editor_git_modified,
                    Kind::Deleted => tokens.editor_git_deleted,
                }),
            );
        } else if change.lines.is_empty() && anchor == line {
            let y = if change.lines.start == 0 {
                top.y + 2.
            } else {
                top.y + height
            };
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(top.x - 2., y - 3.),
                    egui::pos2(top.x + tokens.editor_git_width + 2., y),
                    egui::pos2(top.x - 2., y + 3.),
                ],
                color(tokens.editor_git_deleted),
                egui::Stroke::NONE,
            ));
        }
    }
}
