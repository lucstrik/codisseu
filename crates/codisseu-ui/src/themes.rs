//! Instance-owned app palette. The photo is a visual cue, not an upstream theme.
use super::*;
use codisseu_core::ThemePreference;

impl Tokens {
    pub(super) fn for_theme(theme: ThemePreference) -> Self {
        let mut value: serde_json::Value =
            serde_json::from_str(include_str!("../../../design/tokens.json"))
                .expect("versioned base tokens");
        if theme == ThemePreference::Midnight {
            let overrides: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(include_str!("../../../design/themes/midnight.json"))
                    .expect("versioned midnight palette");
            let base = value.as_object_mut().expect("token object");
            for (key, color) in overrides {
                assert!(base.contains_key(&key), "unknown theme token: {key}");
                base.insert(key, color);
            }
        }
        let mut tokens: Self = serde_json::from_value(value).expect("complete app palette");
        tokens.theme = theme;
        tokens
    }
}

impl CodisseuApp {
    pub(super) fn sync_theme(&mut self, ui: &mut egui::Ui) {
        let theme = self.service.editor_preferences().theme;
        let changed = theme != self.tokens.theme;
        if changed {
            self.tokens = Tokens::for_theme(theme);
        }
        if changed || !self.styled {
            let ctx = ui.ctx();
            self.tokens.apply(ctx);
            self.styled = true;
            // Root Ui already exists when session restoration completes. Its
            // descendants must inherit this palette during the current pass.
            ui.set_style(ui.ctx().global_style());
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_overlay_changes_color_roles_without_changing_geometry() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../../../design/tokens.json")).unwrap();
        let overlay: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../../design/themes/midnight.json")).unwrap();
        for (key, value) in overlay {
            let original = &base[&key];
            assert!(
                original.is_array(),
                "a theme must not change geometry: {key}"
            );
            assert_eq!(
                original.as_array().unwrap().len(),
                value.as_array().unwrap().len()
            );
            if key != "git_graph" {
                assert!(matches!(value.as_array().unwrap().len(), 3 | 4));
                assert!(
                    value
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|n| n.as_u64().is_some_and(|n| n <= 255))
                );
            }
        }
        let classic = Tokens::for_theme(ThemePreference::Classic);
        let night = Tokens::for_theme(ThemePreference::Midnight);
        assert_ne!(classic.editor, night.editor);
        assert_ne!(classic.panel, night.panel);
        assert_ne!(classic.ai_composer_background, night.ai_composer_background);
        assert_ne!(classic.database_grid_header, night.database_grid_header);
        assert_ne!(classic.git_popup_background, night.git_popup_background);
        assert_eq!(night.editor_git_added, [80, 161, 104]);
        assert_eq!(night.editor_git_modified, [213, 157, 73]);
        assert_eq!(night.editor_git_deleted, [239, 104, 104]);
    }

    #[test]
    fn app_palettes_are_independent_between_contexts() {
        let night_ctx = egui::Context::default();
        let classic_ctx = egui::Context::default();
        let night = Tokens::for_theme(ThemePreference::Midnight);
        let classic = Tokens::for_theme(ThemePreference::Classic);
        night_ctx.set_theme(egui::Theme::Light);
        night.apply(&night_ctx);
        classic.apply(&classic_ctx);
        assert_eq!(
            night_ctx.global_style().visuals.panel_fill,
            color(night.panel)
        );
        assert_eq!(
            classic_ctx.global_style().visuals.panel_fill,
            color(classic.panel)
        );
        assert_eq!(
            night_ctx.global_style().visuals.widgets.hovered.bg_fill,
            color(night.widget_hover)
        );
        assert_eq!(
            night_ctx.global_style().visuals.selection.bg_fill,
            color(night.selection)
        );
        assert_eq!(night_ctx.theme(), egui::Theme::Dark);
        assert_eq!(
            night_ctx.style_of(egui::Theme::Dark).visuals.window_fill,
            color(night.panel)
        );
    }
}
