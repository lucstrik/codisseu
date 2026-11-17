//! Presentation only. The editor owns the unchanged marker center and hitbox.
use super::*;
use codisseu_core::debugger::Breakpoint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
    Enabled,
    Pending,
    Verified,
    Rejected,
    Disabled,
    Muted,
}
pub(crate) fn state(
    point: &Breakpoint,
    checked: Option<&Breakpoint>,
    active: bool,
    muted: bool,
) -> State {
    let checked = checked.filter(|p| {
        p.enabled == point.enabled && p.options == point.options && p.revision == point.revision
    });
    if muted {
        State::Muted
    } else if !point.enabled {
        State::Disabled
    } else if !active {
        State::Enabled
    } else if checked.is_some_and(|p| p.verified) {
        State::Verified
    } else if checked.is_some_and(|p| p.message.as_ref().is_some_and(|s| !s.trim().is_empty())) {
        State::Rejected
    } else {
        State::Pending
    }
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Breakpoint enabled",
            Self::Pending => "Breakpoint pending verification",
            Self::Verified => "Breakpoint verified",
            Self::Rejected => "Breakpoint not verified by the adapter",
            Self::Disabled => "Breakpoint disabled",
            Self::Muted => "Breakpoint muted; execution will not stop here",
        }
    }
    pub fn paint(self, painter: &egui::Painter, center: egui::Pos2, tokens: &Tokens) {
        let tint = color(if matches!(self, Self::Disabled | Self::Muted) {
            tokens.muted
        } else {
            tokens.error
        });
        if matches!(self, Self::Enabled | Self::Verified) {
            painter.circle_filled(center, 4.5, tint);
        } else {
            painter.circle_stroke(center, 4.5, egui::Stroke::new(1.5, tint));
        }
        if matches!(self, Self::Muted | Self::Rejected) {
            painter.line_segment(
                [center + egui::vec2(-3., 3.), center + egui::vec2(3., -3.)],
                egui::Stroke::new(1.25, tint),
            );
        }
        if self == Self::Rejected {
            painter.line_segment(
                [center + egui::vec2(-3., -3.), center + egui::vec2(3., 3.)],
                egui::Stroke::new(1.25, tint),
            );
        }
        if self == Self::Verified {
            painter.add(egui::Shape::line(
                vec![
                    center + egui::vec2(-2.3, 0.),
                    center + egui::vec2(-0.6, 1.6),
                    center + egui::vec2(2.5, -2.),
                ],
                egui::Stroke::new(1.1, egui::Color32::WHITE),
            ));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inactive_muted_and_rejected_are_not_reported_as_verified() {
        let mut p = Breakpoint {
            path: "fixture.php".into(),
            line: 3,
            enabled: true,
            verified: true,
            options: Default::default(),
            revision: 0,
            message: None,
        };
        assert_eq!(state(&p, Some(&p), true, true), State::Muted);
        assert_eq!(state(&p, Some(&p), false, false), State::Enabled);
        assert_eq!(state(&p, None, true, false), State::Pending);
        assert_eq!(state(&p, Some(&p), true, false), State::Verified);
        let old = p.clone();
        p.revision += 1;
        assert_eq!(state(&p, Some(&old), true, false), State::Pending);
        p.verified = false;
        p.message = Some("No executable code".into());
        assert_eq!(state(&p, Some(&p), true, false), State::Rejected);
        p.enabled = false;
        assert_eq!(state(&p, Some(&p), true, false), State::Disabled);
    }
}
