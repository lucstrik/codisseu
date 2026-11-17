//! Jira resources share the native file tab strip, without synthetic documents.
use super::*;
use codisseu_core::ViewId;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Hours,
    Issue,
}
#[derive(Default)]
pub(super) struct Tabs {
    file_reveals: BTreeSet<ViewId>,
    hours: Option<ViewId>,
    issue: Option<ViewId>,
    active: BTreeMap<ViewId, Kind>,
    revealed: BTreeMap<ViewId, (Kind, String, u32)>,
}
impl Tabs {
    pub fn take_file_reveal(&mut self, view: ViewId) -> bool {
        self.file_reveals.remove(&view)
    }
    pub fn owner(&self, kind: Kind) -> Option<ViewId> {
        match kind {
            Kind::Hours => self.hours,
            Kind::Issue => self.issue,
        }
    }
    pub fn contains(&self, kind: Kind) -> bool {
        self.owner(kind).is_some()
    }
    pub fn has(&self, view: ViewId) -> bool {
        [self.hours, self.issue].contains(&Some(view))
    }
    pub fn selected(&self, view: ViewId) -> Option<Kind> {
        self.active
            .get(&view)
            .copied()
            .filter(|k| self.owner(*k) == Some(view))
    }
    pub fn is_active(&self, kind: Kind) -> bool {
        self.owner(kind)
            .is_some_and(|v| self.selected(v) == Some(kind))
    }
    pub fn deactivate(&mut self, view: ViewId) {
        self.active.remove(&view);
        self.revealed.remove(&view);
    }
    pub fn open(&mut self, kind: Kind, view: ViewId) {
        match kind {
            Kind::Hours => self.hours = Some(view),
            Kind::Issue => self.issue = Some(view),
        }
        self.active.insert(view, kind);
        self.revealed.remove(&view);
    }
    pub fn close(&mut self, kind: Kind) {
        if let Some(view) = self.owner(kind)
            && self.selected(view) == Some(kind)
        {
            self.deactivate(view)
        }
        match kind {
            Kind::Hours => self.hours = None,
            Kind::Issue => self.issue = None,
        }
    }
    fn repair(&mut self, views: &[ViewId]) {
        self.file_reveals
            .retain(|id| views.contains(id) || (*id == 0 && views.is_empty()));
        let first = views.first().copied().unwrap_or(0);
        let selected = self.active.clone();
        for kind in [Kind::Hours, Kind::Issue] {
            if let Some(owner) = self.owner(kind)
                && !views.contains(&owner)
                && owner != first
            {
                self.deactivate(owner);
                match kind {
                    Kind::Hours => self.hours = Some(first),
                    Kind::Issue => self.issue = Some(first),
                }
            }
        }
        for (old, kind) in selected {
            if !views.contains(&old) && self.owner(kind) == Some(first) {
                self.active.entry(first).or_insert(kind);
            }
        }
    }
}
impl CodisseuApp {
    pub(super) fn jira_tool_active(&self) -> bool {
        self.jira_active_in_view(self.service.active_view().unwrap_or(0))
    }
    pub(super) fn jira_active_in_view(&self, view: ViewId) -> bool {
        self.service.jira.profile.enabled && self.jira_ui.tabs.selected(view).is_some()
    }
    pub(super) fn jira_deactivate_view(&mut self, view: ViewId) {
        if self.jira_ui.tabs.selected(view).is_some() {
            self.jira_ui.tabs.file_reveals.insert(view);
        }
        self.jira_ui.tabs.deactivate(view);
    }
    pub(super) fn jira_open_tab(&mut self, kind: Kind) {
        let view = self
            .jira_ui
            .tabs
            .owner(kind)
            .unwrap_or_else(|| self.service.active_view().unwrap_or(0));
        if self.service.view(view).is_some() {
            self.send(Command::ActivateView { view });
        }
        self.browser_ui.deactivate(view);
        if self.api_ui.active_in_view(view) {
            self.api_ui.active = None;
        }
        self.jira_ui.tabs.open(kind, view);
        self.focus_editor = false;
    }
    pub(super) fn jira_close_tab(&mut self, kind: Kind) {
        if let Some(view) = self.jira_ui.tabs.owner(kind)
            && self.jira_ui.tabs.selected(view) == Some(kind)
        {
            self.jira_ui.tabs.file_reveals.insert(view);
        }
        self.jira_ui.tabs.close(kind);
        if kind == Kind::Hours {
            self.jira_ui.hours.date_picker = false;
            self.service.jira.timesheet.cancel();
        }
        self.focus_editor = true;
    }
    pub(super) fn jira_tabs_tick(&mut self) {
        let views: Vec<_> = self.service.document_views().iter().map(|v| v.id).collect();
        self.jira_ui.tabs.repair(&views);
    }
    pub(super) fn jira_resource_content(&mut self, ui: &mut egui::Ui, view: ViewId) -> bool {
        if !self.service.jira.profile.enabled {
            return false;
        }
        match self.jira_ui.tabs.selected(view) {
            Some(Kind::Hours) => self.jira_hours_page(ui),
            Some(Kind::Issue) => self.jira_reader(ui),
            None => return false,
        }
        true
    }
    pub(super) fn jira_tabs(&mut self, ui: &mut egui::Ui, view: ViewId) {
        if !self.service.jira.profile.enabled {
            return;
        }
        let mut close = None;
        for kind in [Kind::Hours, Kind::Issue] {
            if self.jira_ui.tabs.owner(kind) != Some(view) {
                continue;
            }
            let name = match kind {
                Kind::Hours => "Minhas horas".into(),
                Kind::Issue => format!(
                    "Jira · {}",
                    self.service.jira.selected.as_deref().unwrap_or("Tarefa")
                ),
            };
            let selected = self.jira_ui.tabs.selected(view) == Some(kind);
            let font = FontId::proportional(self.tokens.ui_font_size);
            let width = (ui
                .painter()
                .layout_no_wrap(name.clone(), font.clone(), color(self.tokens.text))
                .size()
                .x
                + self.tokens.tab_text_inset
                + self.tokens.tab_trailing_space)
                .max(90.)
                .floor();
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(width, self.tokens.tab_height),
                egui::Sense::hover(),
            );
            let signature = (kind, name.clone(), ui.clip_rect().width().round() as u32);
            if selected && self.jira_ui.tabs.revealed.get(&view) != Some(&signature) {
                ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
                self.jira_ui.tabs.revealed.insert(view, signature);
            }
            let glyph = egui::Rect::from_center_size(
                egui::pos2(rect.right() - 16., rect.center().y),
                egui::Vec2::splat(self.tokens.tab_close_size),
            );
            let close_rect = egui::Rect::from_min_max(
                egui::pos2(glyph.left(), rect.top() + 1.),
                egui::pos2(glyph.right(), rect.bottom()),
            );
            let body = egui::Rect::from_min_max(
                rect.min + egui::vec2(0., 1.),
                egui::pos2(close_rect.left(), rect.bottom()),
            );
            let response = ui.interact(
                body,
                egui::Id::unique(("jira.tab", view, kind as u8)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    selected,
                    &name,
                )
            });
            if response.clicked() {
                self.jira_open_tab(kind);
            }
            let opacity = if selected || response.hovered() || response.has_focus() {
                1.
            } else {
                self.tokens.tab_inactive_opacity
            };
            let foreground = if opacity < 1. {
                tab_label::dimmed_color(
                    color(self.tokens.editor),
                    color(self.tokens.text),
                    self.tokens.tab_unselected_text_blend,
                )
            } else {
                color(self.tokens.text)
            };
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(rect.left() + 20., rect.center().y),
                egui::Vec2::splat(16.),
            );
            components::paint_icon_monochrome(
                ui,
                icon_rect,
                if kind == Kind::Hours {
                    Icon::Clock
                } else {
                    Icon::Jira
                },
                color(self.tokens.text).gamma_multiply(opacity),
            );
            let anchor = egui::pos2(rect.left() + self.tokens.tab_text_inset, rect.center().y);
            if !self.tab_labels.paint(
                ui,
                &name,
                anchor,
                glyph.left() - anchor.x - 2.,
                tab_label::LabelStyle {
                    foreground,
                    background: color(self.tokens.editor),
                    opacity: 1.,
                    font_size: self.tokens.ui_font_size,
                },
            ) {
                ui.painter().with_clip_rect(body).text(
                    anchor,
                    egui::Align2::LEFT_CENTER,
                    &name,
                    font,
                    color(self.tokens.text),
                );
            }
            if response.has_focus() {
                ui.painter().rect_stroke(
                    body.shrink(2.),
                    2.,
                    egui::Stroke::new(1., color(self.tokens.accent)),
                    egui::StrokeKind::Inside,
                );
            }
            let x = ui.interact(
                close_rect,
                egui::Id::unique(("jira.tab.close", view, kind as u8)),
                egui::Sense::click(),
            );
            x.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    ui.is_enabled(),
                    if kind == Kind::Hours {
                        "Fechar aba Minhas horas"
                    } else {
                        "Fechar leitor Jira"
                    },
                )
            });
            if selected
                || response.hovered()
                || response.has_focus()
                || x.hovered()
                || x.has_focus()
            {
                let hovered = x.hovered() || x.has_focus();
                self.tab_paint.close(
                    ui,
                    glyph,
                    tab_paint::CloseStyle {
                        foreground: color(if hovered {
                            self.tokens.tab_close_hover_foreground
                        } else {
                            self.tokens.tab_close_foreground
                        }),
                        background: color(self.tokens.editor),
                        hover_fill: color(self.tokens.tab_close_hover_background),
                        hovered,
                        padding: egui::vec2(
                            self.tokens.tab_close_padding[0],
                            self.tokens.tab_close_padding[1],
                        ),
                        stroke: self.tokens.tab_close_stroke,
                        hover_radius: self.tokens.tab_close_hover_radius,
                    },
                );
            }
            if x.clicked() || response.clicked_by(egui::PointerButton::Middle) {
                close = Some(kind);
            }
            if selected {
                self.tab_paint.underline(
                    ui,
                    egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 4.), rect.max),
                    [
                        color(if self.service.active_view().is_none_or(|v| v == view) {
                            self.tokens.accent
                        } else {
                            self.tokens.inactive_tab_underline
                        }),
                        color(self.tokens.editor),
                        color(self.tokens.border),
                    ],
                    self.tokens.tab_underline_radius,
                    self.tokens.tab_underline_inset,
                );
            }
            response.on_hover_text(name);
        }
        if let Some(kind) = close {
            self.jira_close_tab(kind);
        }
    }
}
