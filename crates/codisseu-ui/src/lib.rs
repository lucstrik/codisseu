//! Native Codisseu shell. Document state and history live in codisseu-core.
use crate::scrollbars::ScrollAreaExt as _;
mod ai_chat;
mod capabilities;
mod components;
mod database;
mod database_grid;
mod database_grid_filters;
#[cfg(test)]
mod database_grid_tests;
mod database_navigation;
mod database_queries;
mod database_selection;
mod database_services;
mod database_transfer;
mod database_transfer_views;
mod encoding;
mod file_history;
mod shell_layout;
use components::{Icon, header, icon_button, icon_toggle_button, tree_row};
use shell_layout::{ShellSide, bounded_panel};
mod breadcrumbs;
mod breakpoint_options;
mod browser;
mod browser_inspector;
#[cfg(test)]
mod browser_inspector_tests;
mod browser_library_ui;
#[cfg(test)]
mod browser_tests;
mod code_actions;
mod completion;
mod completion_auto;
mod copilot;
mod debugger;
mod debugger_config;
#[cfg(test)]
mod debugger_gutter_tests;
#[cfg(test)]
mod debugger_real_tests;
mod documentation;
mod editor;
mod editor_compare;
#[cfg(test)]
mod editor_git_autosave_tests;
mod editor_shortcuts;
mod editor_views;
mod external_changes;
#[cfg(test)]
mod external_changes_tests;
mod jira;
mod jira_images;
mod jira_tabs;
#[cfg(test)]
mod jira_tests;
mod jira_timesheet;
#[cfg(test)]
mod jira_timesheet_tests;
mod js_tests;
mod language_status;
mod markdown;
mod markdown_integration;
mod markdown_source;
mod new_window;
mod php_tests;
mod project_files;
#[cfg(test)]
mod python_tasks_tests;
mod python_tests;
mod python_tools;
#[cfg(test)]
mod python_workflow_tests;
mod query_execution;
#[cfg(test)]
mod query_execution_tests;
mod query_export;
#[cfg(test)]
mod quick_fixes_tests;
mod rust_features;
#[cfg(test)]
mod rust_features_tests;
mod rust_tools;
#[cfg(test)]
mod rust_workflow_tests;
mod sql_completion;
mod sql_productivity;
mod status_messages;
pub use editor::EditorMetrics;
mod api_client;
#[cfg(test)]
mod api_client_tests;
mod diagnostics;
mod docker;
mod docker_integration;
mod drawing_canvas;
mod drawings;
#[cfg(test)]
mod drawings_tests;
mod git;
mod git_actions;
mod git_commit;
mod git_commit_ai;
#[cfg(test)]
mod git_commit_ai_tests;
mod git_conflicts;
mod git_dialogs;
mod git_diff;
mod git_diff_view;
mod git_insights;
mod git_insights_details;
mod git_insights_search;
#[cfg(test)]
mod git_integrity_tests;
mod git_log;
mod git_merge_style;
mod git_merge_view;
mod git_paint;
#[cfg(test)]
mod git_panel_preferences_tests;
#[cfg(test)]
mod git_performance_tests;
mod git_rebase;
mod git_repositories;
mod git_scm;
mod git_scroll;
mod git_shelf;
mod git_sync;
mod highlight;
mod menus;
mod native_folder;
mod navigation;
mod preferences;
mod project_search;
mod project_search_preview;
mod provider_choices;
mod references;
mod rename;
mod scrollbars;
mod search;
mod shell;
#[cfg(test)]
mod shell_polish_tests;
mod shutdown;
pub use diagnostics::set_diagnostic_sink;
mod signature;
mod signature_content;
mod signature_frame;
mod signature_paint;
mod snippets;
mod tab_badges;
mod tab_label;
mod tab_paint;
mod tab_reorder;
mod terminal;
#[cfg(test)]
mod workspace_close_tests;
mod workspace_switch;
mod workspace_trust;
use capabilities::ToolTab;
use codisseu_core::{AppService, Command, DocumentId};
use eframe::egui::{self, Color32, FontId, Key, KeyboardShortcut, Modifiers, RichText};
use serde::Deserialize;
use std::{
    collections::{HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize)]
struct Tokens {
    copilot_ghost_opacity: f32,
    php_test_failed: [u8; 3],
    php_test_passed: [u8; 3],
    php_test_tree_fraction: f32,
    php_test_tree_row_height: f32,
    php_test_indent: f32,
    php_test_toolbar_height: f32,
    ai_content_inset: i8,
    ai_content_bottom_inset: i8,
    ai_composer_background: [u8; 3],
    ai_composer_border: [u8; 3],
    ai_composer_radius: u8,
    ai_history_row_height: f32,
    ai_history_title_size: f32,
    ai_history_meta_size: f32,
    ai_history_popup_radius: u8,
    ai_history_padding: i8,
    git_authorship_gap: f32,
    git_authorship_color: [u8; 4],
    git_blame_max_width: f32,
    git_blame_width_fraction: f32,
    git_push_width: f32,
    git_push_body_height: f32,
    git_sync_footer_height: f32,
    git_pull_width: f32,
    git_rebase_width: f32,
    git_rebase_height: f32,
    git_rebase_row_height: f32,
    git_tree_row_height: f32,
    git_log_toolbar_width: f32,
    git_log_filter_height: f32,
    git_log_row_height: f32,
    git_log_branch_width: f32,
    git_log_details_width: f32,
    git_log_selection: [u8; 3],
    git_log_head: [u8; 3],
    git_changelist_width: f32,
    git_shelve_width: f32,
    git_shelve_height: f32,
    git_focus_selection: [u8; 3],
    git_modified: [u8; 3],
    git_added: [u8; 3],
    git_deleted: [u8; 3],
    git_untracked: [u8; 3],
    git_conflict: [u8; 3],
    git_popup_background: [u8; 3],
    git_reference: [u8; 3],
    git_diff_added: [u8; 3],
    git_diff_deleted: [u8; 3],
    git_graph: Vec<[u8; 3]>,
    database_tab_selection: [u8; 3],
    database_tab_border: [u8; 3],
    database_completion_foreground: [u8; 3],
    database_completion_key: [u8; 3],
    database_dialog_width: f32,
    database_dialog_height: f32,
    database_dialog_tree_width: f32,
    database_toolbar_height: f32,
    database_form_label_width: f32,
    database_options_label_width: f32,
    database_form_trailing_width: f32,
    database_port_width: f32,
    database_save_width: f32,
    database_form_row_gap: f32,
    database_grid_foreground: [u8; 3],
    database_services_selection: [u8; 3],
    database_grid_header_foreground: [u8; 3],
    database_grid_active_row: [u8; 3],
    database_grid_null: [u8; 3],
    database_grid_dirty_selection: [u8; 3],
    database_preview_width: f32,
    database_preview_height: f32,
    database_grid_row_height: f32,
    database_grid_font_size: f32,
    database_row_header_font_size: f32,
    database_grid_header: [u8; 3],
    database_grid_separator: [u8; 3],
    database_grid_selection: [u8; 3],
    database_dirty: [u8; 3],
    database_engine_mysql: [u8; 3],
    database_engine_postgres: [u8; 3],
    database_engine_mongo: [u8; 3],
    database_engine_redis: [u8; 3],
    database_engine_memcached: [u8; 3],
    shell_panel_gap: f32,
    shell_body_insets: [i8; 4],
    panel_content_inset_x: i8,
    panel_content_inset_y: i8,
    dialog_content_inset: i8,
    providers_content_inset: i8,
    shell_panel_radius: u8,
    shell_panel_background: [u8; 3],
    shell_background: [u8; 3],
    shell_corner_radius: u8,
    shell_border: [u8; 3],
    editor: [u8; 3],
    panel: [u8; 3],
    bar: [u8; 3],
    border: [u8; 3],
    text: [u8; 3],
    muted: [u8; 3],
    accent: [u8; 3],
    selection: [u8; 3],
    caret_row: [u8; 3],
    editor_matching_bracket: [u8; 3],
    editor_scrollbar: [u8; 3],
    inactive_tab_underline: [u8; 3],
    editor_selection: [u8; 3],
    editor_selection_highlight: [u8; 4],
    editor_selection_highlight_border: [u8; 4],
    editor_caret: [u8; 3],
    editor_caret_width: f32,
    editor_caret_height: f32,
    caret_line_number: [u8; 3],
    error: [u8; 3],
    control_border: [u8; 3],
    popup_border: [u8; 3],
    navigation_popup_background: [u8; 3],
    navigation_popup_text: [u8; 3],
    navigation_popup_selection: [u8; 3],
    navigation_popup_public_color: [u8; 3],
    navigation_popup_header_height: f32,
    navigation_popup_header_gap: f32,
    navigation_popup_padding: i8,
    navigation_popup_row_height: f32,
    navigation_popup_min_width: f32,
    control_height: f32,
    dialog_button_min_width: f32,
    control_corner_radius: u8,
    menu_row_height: f32,
    ui_font_size: f32,
    editor_font_size: f32,
    editor_glyph_y_offset: f32,
    editor_phpdoc: [u8; 3],
    editor_php_variable: [u8; 3],
    editor_unnecessary_text: [u8; 3],
    tab_text_inset: f32,
    tab_trailing_space: f32,
    tab_spacing: f32,
    tab_close_size: f32,
    tab_close_foreground: [u8; 3],
    tab_close_hover_foreground: [u8; 3],
    tab_close_hover_background: [u8; 3],
    tab_close_padding: [f32; 2],
    tab_close_stroke: f32,
    tab_close_hover_radius: f32,
    tab_underline_radius: [f32; 2],
    tab_underline_inset: f32,
    tab_inactive_opacity: f32,
    tab_unselected_text_blend: f32,
    tab_more_foreground: [u8; 3],
    tab_more_hover_foreground: [u8; 3],
    tab_more_hover_background: [u8; 3],
    tab_more_hover_radius: f32,
    file_type_blue: [u8; 3],
    rail_selection: [u8; 3],
    editor_line_height: f32,
    editor_git_added: [u8; 3],
    editor_git_modified: [u8; 3],
    editor_git_deleted: [u8; 3],
    editor_git_width: f32,
    editor_git_number_gap: f32,
    editor_git_lane_width: f32,
    editor_gutter_width: f32,
    editor_fold_center_offset: f32,
    editor_breakpoint_center_offset: f32,
    editor_gutter_separator: [u8; 3],
    editor_letter_spacing: f32,
    editor_line_number_offset: f32,
    editor_line_number_y: f32,
    editor_line_number_font_size: f32,
    editor_top_padding: f32,
    editor_bottom_padding: f32,
    editor_wrap_right_padding: f32,
    completion_width: f32,
    completion_php_width: f32,
    completion_footer_font_size: f32,
    completion_footer_text: [u8; 3],
    completion_row_height: f32,
    completion_footer_height: f32,
    completion_max_rows: usize,
    completion_padding: i8,
    completion_frame_corner_radius: u8,
    rename_primary: [u8; 3],
    rename_tree_selection: [u8; 3],
    rename_text: [u8; 3],
    rename_name_selection: [u8; 3],
    rename_name_padding: [i8; 2],
    rename_button_fill: [u8; 3],
    rename_button_border: [u8; 3],
    related_dialog_width: f32,
    related_dialog_height: f32,
    related_table_height: f32,
    related_row_height: f32,
    related_header_height: f32,
    rename_conflict_width: f32,
    rename_conflict_height: f32,
    rename_conflict_toolbar_height: f32,
    rename_conflict_tree_height: f32,
    rename_conflict_preview_header_height: f32,
    rename_conflict_footer_height: f32,
    rename_conflict_code_offset: f32,
    rename_conflict_hint_background: [u8; 3],
    rename_conflict_hint_text: [u8; 3],
    rename_conflict_name_background: [u8; 3],
    rename_conflict_hint_min_height: f32,
    rename_conflict_ts_hint_min_height: f32,
    rename_dialog_width: f32,
    rename_input_text: [u8; 3],
    rename_scope_fill: [u8; 3],
    rename_checkbox_border: [u8; 3],
    rename_file_highlight: [u8; 3],
    rename_ts_dialog_width: f32,
    rename_extended_dialog_height: f32,
    rename_dialog_height: f32,
    rename_dialog_margin: i8,
    rename_actionbar_height: f32,
    references_preview_ratio: f32,
    rename_preview_ratio: f32,
    references_splitter_width: f32,
    references_indent: f32,
    references_inactive_selection: [u8; 3],
    status_height: f32,
    breadcrumb_padding: f32,
    breadcrumb_icon_size: f32,
    breadcrumb_max_label_width: f32,
    breadcrumb_gap: f32,
    references_tree_padding: f32,
    completion_icon_color: [u8; 3],
    completion_match_color: [u8; 3],
    completion_anchor_offset: [f32; 2],
    signature_background: [u8; 3],
    signature_foreground: [u8; 3],
    signature_border: [u8; 3],
    signature_active: [u8; 3],
    signature_overload_background: [u8; 3],
    signature_php_padding: f32,
    signature_anchor_gap: f32,
    signature_baseline: f32,
    signature_type: [u8; 3],
    signature_inactive_name: [u8; 3],
    signature_inactive_type: [u8; 3],
    signature_font_size: f32,
    signature_padding: [f32; 2],
    signature_radius: u8,
    signature_row_height: f32,
    documentation_width: f32,
    documentation_padding: i8,
    documentation_code_padding: i8,
    documentation_block_gap: f32,
    documentation_line_height: f32,
    documentation_max_height: f32,
    find_field_min_width: f32,
    find_field_max_width: f32,
    find_padding: [i8; 2],
    row_height: f32,
    sidebar_width: f32,
    shell_bottom_height: f32,
    shell_right_width: f32,
    tool_header_height: f32,
    main_toolbar_height: f32,
    main_button_size: f32,
    rail_width: f32,
    tab_height: f32,
}
fn text_field_focused(ctx: &egui::Context) -> bool {
    ctx.memory(|m| m.focused())
        .is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some())
}
fn consume_exact(ctx: &egui::Context, modifiers: Modifiers, key: Key) -> bool {
    ctx.input_mut(|input|{let mut found=false;input.events.retain(|event|{let hit=matches!(event,egui::Event::Key{key:pressed,pressed:true,modifiers:actual,..}if *pressed==key&&actual.matches_exact(modifiers));found|=hit;!hit});found})
}
fn color(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}
impl Tokens {
    fn content_margin(&self) -> egui::Margin {
        egui::Margin::symmetric(self.panel_content_inset_x, self.panel_content_inset_y)
    }
    fn load() -> Self {
        serde_json::from_str(include_str!("../../../design/tokens.json"))
            .expect("versioned theme tokens")
    }
    fn apply(&self, ctx: &egui::Context) {
        let mut fonts = egui::FontDefinitions::default();
        // Original Codicons glyphs; CC BY 4.0 attribution and source accompany the asset.
        fonts.font_data.insert(
            "codisseu.codicons".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/vscode-codicons/codicon.ttf"
            ))
            .into(),
        );
        fonts.families.insert(
            egui::FontFamily::Name("codisseu.codicons".into()),
            vec!["codisseu.codicons".into()],
        );
        for (name, bytes) in [
            (
                "codisseu.terminal.ui",
                include_bytes!("../../../reference/public/fonts/NotoSans-Regular.ttf").as_slice(),
            ),
            (
                "codisseu.terminal.mono",
                include_bytes!("../../../reference/public/fonts/DejaVuSansMono.ttf").as_slice(),
            ),
        ] {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_static(bytes).into());
            fonts.families.insert(
                egui::FontFamily::Name(name.into()),
                vec![
                    name.into(),
                    "codisseu.symbols".into(),
                    "codisseu.cjk".into(),
                    "codisseu.emoji.color".into(),
                ],
            );
        }
        fonts.font_data.insert(
            "codisseu.inter".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/InterVariable.ttf"
            ))
            .into(),
        );
        let mut bold = egui::FontData::from_static(include_bytes!(
            "../../../reference/public/fonts/InterVariable.ttf"
        ));
        bold.tweak.coords = epaint::text::VariationCoords::new([(b"wght", 700.)]);
        fonts
            .font_data
            .insert("codisseu.inter.bold".into(), bold.into());
        fonts.families.insert(
            egui::FontFamily::Name("codisseu.inter.bold".into()),
            vec!["codisseu.inter.bold".into(), "codisseu.inter".into()],
        );
        let mut semibold = egui::FontData::from_static(include_bytes!(
            "../../../reference/public/fonts/InterVariable.ttf"
        ));
        semibold.tweak.coords = epaint::text::VariationCoords::new([(b"wght", 600.)]);
        fonts
            .font_data
            .insert("codisseu.inter.semibold".into(), semibold.into());
        fonts.families.insert(
            egui::FontFamily::Name("codisseu.inter.semibold".into()),
            vec!["codisseu.inter.semibold".into(), "codisseu.inter".into()],
        );
        fonts.font_data.insert(
            "codisseu.mono".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/JetBrainsMono-Regular.ttf"
            ))
            .into(),
        );
        fonts.font_data.insert(
            "codisseu.mono.bold".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/JetBrainsMono-Bold.ttf"
            ))
            .into(),
        );
        fonts.families.insert(
            egui::FontFamily::Name("codisseu.mono.bold".into()),
            vec!["codisseu.mono.bold".into(), "codisseu.mono".into()],
        );
        fonts.font_data.insert(
            "codisseu.mono.italic".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/JetBrainsMono-Italic.ttf"
            ))
            .into(),
        );
        fonts.families.insert(
            egui::FontFamily::Name("codisseu.mono.italic".into()),
            vec![
                "codisseu.mono.italic".into(),
                "codisseu.emoji.color".into(),
                "codisseu.symbols".into(),
                "codisseu.cjk".into(),
            ],
        );
        fonts.font_data.insert(
            "codisseu.symbols".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/NotoSansSymbols2-Regular.ttf"
            ))
            .into(),
        );
        fonts.font_data.insert(
            "codisseu.emoji.color".into(),
            egui::FontData::from_static(include_bytes!(
                "../../../reference/public/fonts/NotoColorEmoji.ttf"
            ))
            .into(),
        );
        let mut cjk = egui::FontData::from_static(include_bytes!(
            "../../../reference/public/fonts/NotoSansCJK-Regular.ttc"
        ));
        cjk.index = 2; // Unmodified OFL collection, Simplified Chinese face.
        fonts.font_data.insert("codisseu.cjk".into(), cjk.into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .get_mut(&family)
                .unwrap()
                .extend(["codisseu.symbols".into(), "codisseu.cjk".into()]);
        }
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "codisseu.inter".into());
        fonts
            .families
            .get_mut(&egui::FontFamily::Monospace)
            .unwrap()
            .insert(0, "codisseu.mono".into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            // Main text font first, then a complete color emoji face before the
            // generic monochrome fallbacks (including sequence shaping).
            fonts
                .families
                .get_mut(&family)
                .unwrap()
                .insert(1, "codisseu.emoji.color".into());
        }
        fonts
            .families
            .get_mut(&egui::FontFamily::Name("codisseu.inter.bold".into()))
            .unwrap()
            .extend([
                "codisseu.emoji.color".into(),
                "codisseu.symbols".into(),
                "codisseu.cjk".into(),
            ]);
        fonts
            .families
            .get_mut(&egui::FontFamily::Name("codisseu.mono.bold".into()))
            .unwrap()
            .extend([
                "codisseu.emoji.color".into(),
                "codisseu.symbols".into(),
                "codisseu.cjk".into(),
            ]);
        for name in [
            "codisseu.mono",
            "codisseu.mono.bold",
            "codisseu.mono.italic",
        ] {
            std::sync::Arc::make_mut(fonts.font_data.get_mut(name).unwrap())
                .tweak
                .y_offset = self.editor_glyph_y_offset;
        }
        ctx.set_fonts(fonts);
        // Preserve arbitrary Unicode text even when bundled fonts lack a glyph.
        // Tofu is an explicit rendering fallback; it never changes document bytes.
        ctx.set_missing_glyph_policy(egui::epaint::text::MissingGlyphPolicy::Tofu);
        let mut style = egui::Style {
            visuals: egui::Visuals::dark(),
            ..Default::default()
        };
        // Keep the rasterizer's coverage. The dark-mode default boosts edge
        // alpha and made the public Mono/Inter faces heavier than the measured
        // native reference. This also preserves the RGB of color emoji.
        style.visuals.text_options.color_transfer_function = epaint::FontColorTransferFunction::Off;
        style.visuals.panel_fill = color(self.panel);
        style.visuals.window_fill = color(self.panel);
        style.visuals.window_corner_radius = self.shell_corner_radius.into();
        style.visuals.menu_corner_radius = self.shell_corner_radius.into();
        style.visuals.window_stroke = egui::Stroke::new(1., color(self.shell_border));
        style.visuals.extreme_bg_color = color(self.editor);
        style.visuals.faint_bg_color = color(self.bar);
        style.visuals.override_text_color = Some(color(self.text));
        style.visuals.selection.bg_fill = color(self.selection);
        style.visuals.selection.stroke = egui::Stroke::new(1.0, color(self.accent));
        style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, color(self.border));
        style.visuals.widgets.inactive.bg_fill = color(self.bar);
        style.visuals.widgets.hovered.bg_fill = color(self.selection);
        style.spacing.item_spacing = egui::vec2(8.0, 0.0);
        style.spacing.window_margin = egui::Margin::same(self.dialog_content_inset);
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(61, 63, 65);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(75, 76, 79);
        style.visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
        style.spacing.interact_size.y = self.row_height;
        style.text_styles.insert(
            egui::TextStyle::Body,
            FontId::proportional(self.ui_font_size),
        );
        style.text_styles.insert(
            egui::TextStyle::Button,
            FontId::proportional(self.ui_font_size),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            FontId::monospace(self.editor_font_size),
        );
        style.text_styles.insert(
            egui::TextStyle::Small,
            FontId::proportional(self.ui_font_size - 2.),
        );
        scrollbars::configure(ctx, &mut style, color(self.editor_scrollbar));
        ctx.set_global_style(style);
    }
}
#[derive(Clone, Copy)]
enum PathAction {
    Workspace,
    WorkspaceFile,
    File,
    SaveUntitled(DocumentId),
}
#[derive(Clone, Copy)]
enum Confirm {
    Tab {
        view: codisseu_core::ViewId,
        id: DocumentId,
    },
    Exit,
    ExitWithoutSession,
}

enum StartupAction {
    Browser(String),
    Command(Box<Command>),
    ExecuteSql,
    Problems,
}

pub struct CodisseuApp {
    new_window_launcher: Option<Box<dyn FnMut() -> Result<(), String>>>,
    jira_ui: jira::JiraUi,
    api_ui: api_client::ApiUi,
    drawings_ui: drawings::DrawingsUi,
    ai_chat: ai_chat::AiChatUi,
    pub service: AppService,
    tokens: Tokens,
    styled: bool,
    main_menu_open: bool,
    editor_shortcuts: editor_shortcuts::EditorShortcuts,
    editor_compare: editor_compare::CompareUi,
    path_dialog: Option<PathAction>,
    path_input: String,
    path_focus: bool,
    folder_picker: native_folder::FolderPicker,
    workspace_switch: workspace_switch::WorkspaceSwitchUi,
    confirm: Option<Confirm>,
    external_changes: external_changes::ExternalChangesUi,
    expanded: HashSet<PathBuf>,
    project_files_ui: project_files::ProjectFilesUi,
    editor_bridge: editor::EditorBridge,
    breadcrumb_ui: breadcrumbs::BreadcrumbUi,
    php_navigation: editor::NavigationGutter,
    markdown: markdown::MarkdownUi,
    debugger_ui: debugger::DebuggerUi,
    rust_tools: rust_tools::RustToolsUi,
    python_ui: python_tools::PythonUi,
    browser_ui: browser::BrowserUi,
    browser_inspector_ui: browser_inspector::InspectorUi,
    python_tests_ui: python_tests::PythonTestsUi,
    rust_features: rust_features::RustFeatureUi,
    markdown_navigation: Option<(PathBuf, String)>,
    sql_width_cache: Option<capabilities::SqlWidthCache>,
    focus_editor: bool,
    tab_paint: tab_paint::TabPaint,
    tab_drag: Option<tab_reorder::TabDrag>,
    tab_labels: tab_label::TabLabels,
    search_panel: search::SearchPanel,
    project_search: project_search::ProjectSearch,
    terminal: terminal::TerminalPane,
    docker_terminal: terminal::TerminalPane,
    docker_terminal_active: bool,
    docker_ui: docker::DockerUi,
    docker_last_refresh: std::time::Instant,
    documentation: Option<(DocumentId, u64)>,
    documentation_error: Option<String>,
    documentation_hover: documentation::HoverUi,
    caret: Option<(DocumentId, usize)>,
    caret_screen: Option<egui::Pos2>,
    documentation_anchor: egui::Pos2,
    definition_choice: usize,
    completion_popup: Option<completion::Popup>,
    completion_auto: completion_auto::Automatic,
    copilot_ui: copilot::CopilotUi,
    sql_completion: sql_completion::SqlCompletion,
    sql_productivity: sql_productivity::SqlProductivity,
    query_export_ui: query_export::QueryExportUi,
    signature_popup: Option<signature::Popup>,
    signature_paint: signature_paint::SignaturePaint,
    signature_input: Option<(String, DocumentId, u64)>,
    snippet_ui: snippets::SnippetUi,
    document_actions: Vec<(DocumentId, &'static str)>,
    references_ui: references::ReferencesUi,
    rename_ui: rename::RenameUi,
    git_ui: git::GitUi,
    git_repositories: git_repositories::RepositoryUis,
    git_insights: git_insights::InsightsUi,
    show_project: bool,
    show_activity: bool,
    terminal_ui: terminal::TerminalUi,
    exit_when_saved: bool,
    shutdown_started: Option<Instant>,
    exit_without_session: bool,
    session_pending: bool,
    allow_close: bool,
    exit_after_resource: bool,
    capture_path: Option<PathBuf>,
    capture_requested: bool,
    capture_stable_frames: u32,
    capture_result: Option<mpsc::Receiver<Result<(), String>>>,
    capture_done: bool,
    close_after_capture: bool,
    pub cursor: (usize, usize),
    cursor_column_pending: bool,
    show_database: bool,
    database_input: String,
    database_ui: database::DatabaseUi,
    query_library: database_queries::QueryLibrary,
    transfer_ui: database_transfer::TransferUi,
    transfer_views: database_transfer_views::TransferViews,
    grid_selection: database_selection::GridSelection,
    grid_focus: Option<egui::Id>,
    history_ui: file_history::HistoryUi,
    php_tests_ui: php_tests::TestsUi,
    js_tests_ui: js_tests::JsTestsUi,
    provider_dialog: bool,
    preferences_dialog: bool,
    provider_config_input: String,
    tool_tab: ToolTab,
    selection: Option<(DocumentId, std::ops::Range<usize>)>,
    jump_to: Option<(PathBuf, u32, u32)>,
    services_stopped: bool,
    shutdown_finalizing: bool,
    pub wait_for_diagnostics: bool,
    pub wait_for_nonempty_diagnostics: bool,
    startup: VecDeque<StartupAction>,
}
impl CodisseuApp {
    pub fn new(service: AppService) -> Self {
        Self {
            new_window_launcher: None,
            jira_ui: jira::JiraUi::default(),
            api_ui: api_client::ApiUi::default(),
            drawings_ui: drawings::DrawingsUi::default(),
            ai_chat: ai_chat::AiChatUi::default(),
            service,
            tokens: Tokens::load(),
            styled: false,
            main_menu_open: false,
            editor_shortcuts: Default::default(),
            editor_compare: Default::default(),
            path_dialog: None,
            path_input: String::new(),
            path_focus: false,
            folder_picker: native_folder::FolderPicker::default(),
            workspace_switch: workspace_switch::WorkspaceSwitchUi::default(),
            confirm: None,
            external_changes: Default::default(),
            expanded: HashSet::new(),
            project_files_ui: project_files::ProjectFilesUi::default(),
            editor_bridge: editor::EditorBridge::default(),
            breadcrumb_ui: Default::default(),
            php_navigation: Default::default(),
            markdown: markdown::MarkdownUi::default(),
            debugger_ui: debugger::DebuggerUi::default(),
            rust_tools: rust_tools::RustToolsUi::default(),
            python_ui: python_tools::PythonUi::default(),
            browser_ui: Default::default(),
            browser_inspector_ui: Default::default(),
            python_tests_ui: python_tests::PythonTestsUi::default(),
            rust_features: rust_features::RustFeatureUi::default(),
            markdown_navigation: None,
            sql_width_cache: None,
            focus_editor: false,
            tab_paint: tab_paint::TabPaint::default(),
            tab_drag: None,
            tab_labels: tab_label::TabLabels::default(),
            search_panel: search::SearchPanel::default(),
            project_search: project_search::ProjectSearch::default(),
            terminal: terminal::TerminalPane::default(),
            docker_terminal: terminal::TerminalPane::default(),
            docker_terminal_active: false,
            docker_ui: docker::DockerUi::default(),
            docker_last_refresh: std::time::Instant::now(),
            documentation: None,
            documentation_error: None,
            documentation_hover: documentation::HoverUi::default(),
            caret: None,
            caret_screen: None,
            documentation_anchor: egui::Pos2::ZERO,
            definition_choice: 0,
            completion_popup: None,
            completion_auto: completion_auto::Automatic::default(),
            copilot_ui: Default::default(),
            sql_completion: sql_completion::SqlCompletion::default(),
            sql_productivity: sql_productivity::SqlProductivity::default(),
            query_export_ui: query_export::QueryExportUi::default(),
            signature_popup: None,
            signature_paint: signature_paint::SignaturePaint::default(),
            signature_input: None,
            snippet_ui: snippets::SnippetUi::default(),
            document_actions: Vec::new(),
            references_ui: references::ReferencesUi::default(),
            rename_ui: rename::RenameUi::default(),
            git_insights: git_insights::InsightsUi::default(),
            git_ui: git::GitUi {
                labels: tab_label::TabLabels::for_git(),
                branches: true,
                ..Default::default()
            },
            git_repositories: Default::default(),
            show_project: true,
            show_activity: false,
            terminal_ui: Default::default(),
            exit_when_saved: false,
            shutdown_started: None,
            exit_without_session: false,
            session_pending: false,
            allow_close: false,
            exit_after_resource: false,
            capture_path: None,
            capture_requested: false,
            capture_stable_frames: 0,
            capture_result: None,
            capture_done: false,
            close_after_capture: false,
            cursor: (1, 1),
            cursor_column_pending: false,
            show_database: true,
            database_input: String::new(),
            database_ui: database::DatabaseUi::default(),
            query_library: database_queries::QueryLibrary::default(),
            transfer_ui: database_transfer::TransferUi::default(),
            transfer_views: database_transfer_views::TransferViews::default(),
            grid_selection: database_selection::GridSelection::default(),
            grid_focus: None,
            history_ui: Default::default(),
            php_tests_ui: php_tests::TestsUi::default(),
            js_tests_ui: js_tests::JsTestsUi::default(),
            provider_dialog: false,
            preferences_dialog: false,
            provider_config_input: concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../.tools/providers.json"
            )
            .into(),
            tool_tab: ToolTab::Results,
            selection: None,
            jump_to: None,
            services_stopped: false,
            shutdown_finalizing: false,
            wait_for_diagnostics: false,
            wait_for_nonempty_diagnostics: false,
            startup: VecDeque::new(),
        }
    }
    /// Explicit CLI requests use the same commands as the widgets, after workspace loading.
    pub fn configure_startup(
        &mut self,
        database: Option<PathBuf>,
        providers: Option<PathBuf>,
        start_providers: bool,
        execute_sql: bool,
        show_problems: bool,
    ) {
        if let Some(path) = database {
            self.startup
                .push_back(StartupAction::Command(Box::new(Command::Database(
                    codisseu_core::DbCommand::Connect(path),
                ))));
        }
        if let Some(path) = providers {
            self.startup.push_back(StartupAction::Command(Box::new(
                Command::LoadProviderConfig(path),
            )));
        }
        if start_providers {
            self.startup
                .push_back(StartupAction::Command(Box::new(Command::StartProviders)));
        }
        if execute_sql {
            self.startup.push_back(StartupAction::ExecuteSql);
        }
        if show_problems {
            self.startup.push_back(StartupAction::Problems);
        }
    }

    /// Desktop policy is explicit here so widget harnesses and headless core
    /// callers keep their deterministic, process-free default.
    pub fn configure_provider_autostart(
        &mut self,
        profile: PathBuf,
        fallback_config: Option<PathBuf>,
    ) {
        let at = self
            .startup
            .iter()
            .position(|action| {
                matches!(
                    action,
                    StartupAction::Command(command) if matches!(command.as_ref(),Command::StartProviders)
                )
            })
            .unwrap_or(self.startup.len());
        self.startup.insert(
            at,
            StartupAction::Command(Box::new(Command::EnableProviderAutostart {
                profile,
                fallback_config,
            })),
        );
    }

    /// Capture the real native framebuffer after queued document loading settles.
    pub fn capture_to(&mut self, path: PathBuf, close_after: bool) {
        self.capture_path = Some(path);
        self.close_after_capture = close_after;
    }
    fn send(&mut self, command: Command) {
        if let Command::Save { id } = &command {
            self.external_changes.dismissed.remove(id);
        }
        if let Command::OpenWorkspace(path) = command {
            self.request_workspace_switch(path);
            return;
        }
        if matches!(
            &command,
            Command::OpenFile(_)
                | Command::NewUntitled
                | Command::Activate { .. }
                | Command::ActivateTab { .. }
        ) {
            self.browser_ui
                .deactivate(self.service.active_view().unwrap_or(0));
            let view = match &command {
                Command::ActivateTab { view, .. } => *view,
                _ => self.service.active_view().unwrap_or(0),
            };
            self.jira_deactivate_view(view);
        }
        if matches!(
            &command,
            Command::OpenWorkspace(_)
                | Command::OpenFile(_)
                | Command::NewUntitled
                | Command::Activate { .. }
                | Command::ActivateTab { .. }
        ) {
            self.ai_flush_draft();
        }
        if matches!(&command, Command::CancelAll) {
            self.git_cancel_all_repository_uis();
        }
        if let Err(error) = self.service.dispatch(command) {
            self.service.last_error = Some(error);
        }
    }
    fn active_command(&mut self, action: &str) {
        if let Some(id) = self.browser_active() {
            if action == "close" {
                self.browser_close(id);
            }
            return;
        }
        if self
            .api_ui
            .active_in_view(self.service.active_view().unwrap_or(0))
        {
            match action {
                "save" => {
                    if let Err(error) = self.service.api.save() {
                        self.service.api.error = Some(error);
                    }
                }
                "close" => self.api_ui.close_active(),
                _ => {}
            }
            return;
        }
        if let Some(id) = self.service.active {
            match action {
                "save" => {
                    self.save_document_or_choose_path(id);
                    self.focus_editor = true;
                }
                "undo" => {
                    self.send(Command::Undo { id });
                    self.focus_editor = true;
                }
                "redo" => {
                    self.send(Command::Redo { id });
                    self.focus_editor = true;
                }
                "close" => self.request_close(id),
                _ => unreachable!(),
            }
        }
    }
    fn open_path(&mut self, action: PathAction) {
        if matches!(action, PathAction::WorkspaceFile) {
            self.open_native_workspace_file();
            return;
        }
        if matches!(action, PathAction::Workspace) {
            self.open_native_folder();
            return;
        }
        self.path_input = self
            .service
            .workspace
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if matches!(action, PathAction::SaveUntitled(_)) && !self.path_input.is_empty() {
            self.path_input.push(std::path::MAIN_SEPARATOR);
        }
        self.path_dialog = Some(action);
        self.path_focus = true;
    }
    fn request_close(&mut self, id: DocumentId) {
        if let Some(view) = self.service.active_view() {
            self.request_close_tab(view, id);
        }
    }
    fn request_close_tab(&mut self, view: codisseu_core::ViewId, id: DocumentId) {
        if self.service.tab_close_needs_confirmation(view, id) {
            self.confirm = Some(Confirm::Tab { view, id });
        } else {
            self.send(Command::CloseTab {
                view,
                id,
                discard: false,
            });
            // Restore keyboard focus to the currently active editor even when
            // the clicked close button belongs to a passive group.
            self.focus_editor = true;
        }
    }
    fn request_exit(&mut self) {
        if self.exit_when_saved || self.allow_close {
            return;
        }
        self.abandon_native_folder();
        self.workspace_switch = Default::default();
        self.service.jira.timesheet.cancel();
        if self.service.api.can_save()
            && let Err(error) = self.service.api.save()
        {
            self.service.api.error = Some(error);
            return;
        }
        self.ai_flush_draft();
        self.debugger_ui.restart = None;
        self.main_menu_open = false;
        self.sync_git_draft();
        self.git_cancel_all_repository_uis();
        self.service.cancel_shutdown_reads();
        if self.database_exit_guard() {
            return;
        }
        if self.service.rename_resources_busy() {
            self.exit_after_resource = true;
            return;
        }
        if self.service.documents.iter().any(|d| d.is_dirty()) {
            self.confirm = Some(Confirm::Exit);
        } else {
            self.exit_when_saved = true;
        }
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.project_search.open {
            self.project_search_shortcuts(ctx);
            return;
        }
        if self.new_window_shortcut(ctx) {
            return;
        }
        if self.jira_tool_active()
            && !self.jira_ui.settings
            && self.jira_ui.worklog.is_none()
            && self.jira_ui.picker.is_none()
            && !self.jira_ui.hours.date_picker
        {
            if self.browser_shortcuts(ctx) {
                return;
            }
            let view = self.service.active_view().unwrap_or(0);
            if consume_exact(ctx, Modifiers::COMMAND, Key::W) {
                if let Some(kind) = self.jira_ui.tabs.selected(view) {
                    self.jira_close_tab(kind);
                }
            } else if consume_exact(ctx, Modifiers::NONE, Key::Escape) {
                self.jira_deactivate_view(view);
                self.focus_editor = true;
            }
            return;
        }
        if self.jira_ui.hours.date_picker
            || self.browser_ui.library.open
            || self.jira_ui.settings
            || self.jira_ui.worklog.is_some()
            || self.jira_ui.picker.is_some()
        {
            return;
        }
        if let Some(drag) = &self.tab_drag {
            let invalid = !self
                .service
                .view(drag.view)
                .is_some_and(|v| v.tabs().contains(&drag.document));
            let cancel = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                || ctx.input(|i| {
                    !i.focused
                        || i.pointer.hover_pos().is_none()
                        || (!i.pointer.primary_down()
                            && !i.pointer.button_released(egui::PointerButton::Primary))
                });
            if invalid || cancel {
                self.tab_drag = None;
            }
            return;
        }
        // Handle the focused tab before global F10 actions. egui's context
        // menu helper only opens on a secondary pointer click by default.
        let focused_tab = ctx.memory(|m| m.focused()).filter(|focused| {
            self.service.document_views().iter().any(|view| {
                view.tabs()
                    .iter()
                    .any(|id| *focused == egui::Id::unique(("codisseu.tab.activate", view.id, id)))
            })
        });
        if let Some(id) = focused_tab
            && ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::F10))
        {
            ctx.data_mut(|d| d.insert_temp(id.with("keyboard_context"), true));
            egui::Popup::open_id(ctx, id.with("popup"));
            return;
        }
        if self.python_ui.open
            || self.python_tests_ui.settings
            || self.js_tests_ui.settings
            || self.php_tests_ui.settings
            || self.history_ui.is_open()
            || self.rust_tools.open
            || self.service.code_actions.is_open()
        {
            return;
        }
        if self.confirm.is_some()
            || self.external_changes.active.is_some()
            || self.debugger_ui.breakpoint.is_some()
            || self.exit_when_saved
            || self.workspace_switch_pending()
            || self.folder_picker_pending()
            || self.transfer_modal_open()
            || self.sql_productivity_modal_open()
            || self.query_export_modal_open()
            || (self.service.provider_startup().needs_trust && !self.provider_dialog)
            || self.project_files_ui.trash.is_some()
        {
            return;
        }
        if !self.service.language.server_choices.is_empty() {
            return;
        }
        if self.browser_shortcuts(ctx) {
            return;
        }
        if self.php_tests_shortcuts(ctx) {
            return;
        }
        if self.grid_inspection_shortcuts(ctx) {
            return;
        }
        if self.requested_editor_shortcuts(ctx) {
            return;
        }
        if self.terminal_shortcuts(ctx) {
            return;
        }
        if self.project_search_shortcuts(ctx) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::ALT, Key::F12)) {
            self.toggle_terminal();
            return;
        }
        if self.show_activity && self.tool_tab == ToolTab::Terminal && self.terminal.is_focused(ctx)
        {
            return;
        }
        if self.path_dialog.is_some()
            || self.provider_dialog
            || self.ai_chat.settings
            || (self.service.provider_startup().needs_trust && !self.provider_dialog)
            || self.project_files_ui.trash.is_some()
            || self.debugger_ui.dialog
            || self.debugger_ui.breakpoint.is_some()
            || self.git_ui.dialog.is_some()
            || self.git_ui.diff_open
            || self.database_ui.dialog
            || self.database_ui.exit_confirm
            || self.database_ui.preview.is_some()
            || self.database_ui.edit.is_some()
            || self.preferences_dialog
            || self.confirm.is_some()
            || self.exit_when_saved
            || self.workspace_switch_pending()
            || self.folder_picker_pending()
        {
            return;
        }
        if consume_exact(ctx, Modifiers::COMMAND, Key::F9) {
            if self.rust_build_enabled() {
                self.open_cargo(codisseu_core::rust_project::CargoAction::Build);
            }
            return;
        }
        if consume_exact(ctx, Modifiers::SHIFT, Key::F10) {
            if self.python_file().is_some() {
                self.open_python(codisseu_core::python_project::Action::RunFile);
                return;
            }
            self.open_cargo(codisseu_core::rust_project::CargoAction::Run);
            return;
        }
        if self.sql_history_shortcut(ctx) {
            return;
        }
        if self.ai_chat.open
            && ctx.memory(|m| {
                m.focused().is_some_and(|id| {
                    [
                        "codisseu.ai.composer",
                        "codisseu.ai.codex",
                        "codisseu.ai.claude",
                        "codisseu.ai.model",
                    ]
                    .iter()
                    .any(|key| id == egui::Id::unique(*key))
                })
            })
        {
            return;
        }
        if self.api_shortcuts(ctx) {
            return;
        }
        self.debugger_shortcuts(ctx);
        if consume_exact(ctx, Modifiers::ALT, Key::Num5) {
            self.show_activity = !(self.show_activity && self.tool_tab == ToolTab::Debug);
            self.tool_tab = ToolTab::Debug;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL.plus(Modifiers::ALT), Key::S)) {
            self.preferences_dialog = true;
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::CTRL.plus(Modifiers::ALT), Key::Tab)) {
            self.next_editor_group();
            return;
        }
        if self.service.formatting.is_pending() && consume_exact(ctx, Modifiers::NONE, Key::Escape)
        {
            self.send(Command::CancelFormatting);
            return;
        }
        if self.drawings_shortcuts(ctx) {
            return;
        }
        if self.markdown_shortcuts(ctx) {
            return;
        }
        if self.editor_owns_chord(ctx) {
            return;
        }
        if self.git_shortcuts(ctx) {
            return;
        }
        if self.rename_shortcuts(ctx) {
            return;
        }
        if self.copilot_shortcuts(ctx) {
            return;
        }
        if self.project_file_shortcuts(ctx) || self.snippet_shortcuts(ctx) {
            return;
        }
        if self.sql_completion_shortcuts(ctx) {
            return;
        }
        if self.completion_shortcuts(ctx) {
            return;
        }
        if self.signature_shortcuts(ctx) {
            return;
        }
        if self.breadcrumb_shortcuts(ctx) {
            return;
        }
        if self.navigation_shortcuts(ctx) {
            return;
        }
        if self.references_shortcuts(ctx) {
            return;
        }
        if self.documentation.is_some()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.close_documentation();
            return;
        }
        self.search_shortcuts(ctx);
        let search_input = self.search_input_focused(ctx) || self.catalog_filter_focused(ctx);
        let take = |modifiers, key| {
            ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(modifiers, key)))
        };
        if take(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::O) {
            self.open_path(PathAction::Workspace);
        } else if take(Modifiers::COMMAND, Key::O) {
            self.open_path(PathAction::File);
        }
        if !search_input
            && self.editor_focused(ctx)
            && (take(Modifiers::SHIFT, Key::F6) || consume_exact(ctx, Modifiers::NONE, Key::F2))
        {
            self.request_rename_inline();
            return;
        }
        if !search_input
            && self.editor_focused(ctx)
            && (consume_exact(ctx, Modifiers::CTRL, Key::P)
                || consume_exact(ctx, Modifiers::CTRL.plus(Modifiers::SHIFT), Key::Space))
        {
            self.request_signature(codisseu_core::SignatureHelpTrigger::Invoked, false);
        }
        if !search_input && self.editor_focused(ctx) && take(Modifiers::CTRL, Key::Space) {
            self.request_completion();
        }
        if !search_input
            && self.editor_focused(ctx)
            && (take(Modifiers::ALT.plus(Modifiers::SHIFT), Key::Num7)
                || take(Modifiers::ALT, Key::F7)
                || consume_exact(ctx, Modifiers::SHIFT, Key::F12))
        {
            // X11 layouts can emit printable text (e.g. '&') together with
            // Alt+Shift+7. The command owns this keystroke, including its text.
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(_)))
            });
            self.request_references();
        }

        if !search_input
            && self.editor_focused(ctx)
            && consume_exact(ctx, Modifiers::COMMAND, Key::F12)
        {
            self.go_to_implementation();
        }
        if !search_input
            && self.editor_focused(ctx)
            && consume_exact(ctx, Modifiers::COMMAND.plus(Modifiers::ALT), Key::U)
        {
            self.go_to_super_definition();
        }
        if !search_input
            && self.editor_focused(ctx)
            && consume_exact(ctx, Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::B)
        {
            self.go_to_type_definition();
        }
        if !search_input
            && self.editor_focused(ctx)
            && consume_exact(ctx, Modifiers::NONE, Key::F12)
        {
            self.go_to_definition();
        }
        if !search_input && take(Modifiers::COMMAND.plus(Modifiers::ALT), Key::ArrowLeft) {
            self.navigate_history(false);
        }
        if !search_input && take(Modifiers::COMMAND.plus(Modifiers::ALT), Key::ArrowRight) {
            self.navigate_history(true);
        }
        if !search_input && take(Modifiers::COMMAND, Key::Q) {
            self.request_documentation();
        }
        // Preserve physical ordering when several commands arrive in one frame.
        // In particular Undo then Save must not enqueue the pre-Undo snapshot.
        let history_context = self.editor_focused(ctx)
            || (!text_field_focused(ctx)
                && self.service.active_view().is_some_and(|view| {
                    self.service.view_document(view).is_some_and(|d| {
                        markdown::is_markdown(&d.path)
                            && self.markdown.mode(view, &d.path) != markdown::MarkdownMode::Source
                    })
                }));
        let document_actions = ctx.input_mut(|input| {
            let mut actions = Vec::new();
            input.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    return true;
                };
                let action = if !search_input
                    && history_context
                    && *key == Key::Z
                    && modifiers.matches_exact(Modifiers::COMMAND.plus(Modifiers::SHIFT))
                {
                    Some("redo")
                } else if !search_input
                    && history_context
                    && *key == Key::Z
                    && modifiers.matches_exact(Modifiers::COMMAND)
                {
                    Some("undo")
                } else if !search_input
                    && history_context
                    && *key == Key::Y
                    && modifiers.matches_exact(Modifiers::COMMAND)
                {
                    Some("redo")
                } else if *key == Key::S && modifiers.matches_exact(Modifiers::COMMAND) {
                    Some("save")
                } else {
                    None
                };
                if let Some(action) = action {
                    actions.push(action);
                    false
                } else {
                    true
                }
            });
            actions
        });
        for action in document_actions {
            if let Some(id) = self.service.active {
                self.document_actions.push((id, action));
            }
        }
        if take(Modifiers::COMMAND, Key::W) {
            self.active_command("close");
        }
        if !search_input
            && self.editor_focused(ctx)
            && self.is_query_document()
            && consume_exact(ctx, Modifiers::COMMAND, Key::Enter)
        {
            self.execute_sql();
        }
    }
    fn show_bottom_panel(&mut self, ui: &mut egui::Ui) {
        if !self.show_activity {
            return;
        }
        let gap = self.tokens.shell_panel_gap.round().clamp(0., 24.) as i8;
        self.prepare_shell_size(ui, ShellSide::Bottom);
        let panel = egui::Panel::bottom("codisseu.activity")
            .resizable(true)
            .show_separator_line(false)
            .frame(self.shell_panel_frame(egui::Margin {
                top: gap,
                ..Default::default()
            }))
            .size_range(
                if self.tool_tab == ToolTab::Terminal && self.terminal_ui.maximized {
                    ui.available_height()..=ui.available_height()
                } else {
                    160.0..=500.0
                },
            )
            .show(ui, |ui| {
                ui.add_enabled_ui(
                    !self.project_search.open
                        && !self.provider_dialog
                        && !self.debugger_ui.dialog
                        && !self.rust_tools.open
                        && !self.js_tests_ui.settings
                        && !self.python_ui.open
                        && !self.python_tests_ui.settings
                        && !self.php_tests_ui.settings
                        && self.debugger_ui.breakpoint.is_none()
                        && !self.preferences_dialog
                        && self.path_dialog.is_none(),
                    |ui| bounded_panel(ui, |ui| self.tool_panel(ui)),
                );
            });
        self.remember_shell_size(ui, ShellSide::Bottom, panel.response.rect);
        self.finish_shell_panel(
            ui,
            panel.response.rect,
            egui::Margin {
                top: gap,
                ..Default::default()
            },
        );
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if !self.styled {
            self.tokens.apply(&ctx);
            self.styled = true;
        }
        self.terminal.tick(&ctx);
        self.docker_terminal.tick(&ctx);
        let workspace_before_poll = self.service.workspace_path().map(PathBuf::from);
        let rename_before_poll = self.service.rename.is_open();
        let active_before_poll = self.service.active;
        // Root changes can also originate in editor views rather than send().
        // Checkpoint against the backend's old identity before it parks that chat.
        if self.service.ai.workspace() != self.service.ai_workspace() {
            self.ai_flush_draft();
        }
        if self.service.poll() {
            self.capture_stable_frames = 0;
        }
        self.external_changes_tick(&ctx);
        self.poll_close_saves();
        // A ready ACK must be consumed before attempting a selected workspace.
        self.poll_native_folder(&ctx);
        if self.folder_picker_pending()
            || self.workspace_switch_pending()
            || self.exit_when_saved
            || self.confirm.is_some()
            || self.external_changes.active.is_some()
        {
            ui.disable();
        }
        if !self.exit_when_saved
            && !self.workspace_switch_pending()
            && !self.folder_picker_pending()
            && self.confirm.is_none()
            && self.external_changes.active.is_none()
        {
            self.service.tick_auto_save();
            if self.service.tick_git_line_changes() {
                ctx.request_repaint();
            }
        }
        if !self.exit_when_saved {
            self.service.tick_project_versions();
        }
        if self.service.workspace_path().is_some() || self.service.active_document().is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(
                if self.service.project_versions_pending() {
                    50
                } else {
                    3000
                },
            ));
        }
        self.jira_tick(&ctx);
        self.browser_tick(&ctx);
        self.python_tick(&ctx);
        self.rust_features_tick(&ctx);
        self.api_tick(&ctx);
        self.drawings_tick();
        self.ai_tick(&ctx);
        self.poll_git_ui();
        self.git_insights_tick(&ctx);
        self.snippets_tick();
        self.project_files_tick();
        if self.exit_when_saved {
            self.debugger_ui.restart = None;
        }
        self.debugger_tick(&ctx);
        self.sync_git_draft();
        self.sync_database_ui();
        self.sql_productivity_tick();
        self.poll_transfer(&ctx);
        self.sync_grid_selection();
        if workspace_before_poll.as_deref() != self.service.workspace_path() {
            self.debugger_ui = debugger::DebuggerUi::default();
            self.terminal.close();
            self.docker_terminal.close();
            self.docker_terminal_active = false;
            self.ai_workspace_changed();
            if self.tool_tab == ToolTab::Terminal {
                self.show_activity = false;
                self.focus_editor = true;
            }
        }
        self.docker_tick(&ctx);
        self.sync_rename_conflicts();
        if rename_before_poll
            && !self.service.rename.is_open()
            && self.service.rename.error.is_none()
        {
            self.rename_ui = rename::RenameUi::default();
            if self.tool_tab == ToolTab::Rename {
                self.show_activity = false;
            }
            self.focus_editor = true;
        }
        if self.service.active != active_before_poll
            && self.service.active.is_some()
            && !self.project_tree_has_focus(&ctx)
        {
            if self
                .api_ui
                .active_in_view(self.service.active_view().unwrap_or(0))
            {
                self.api_ui.active = None;
            }
            self.browser_ui
                .deactivate(self.service.active_view().unwrap_or(0));
            self.jira_deactivate_view(self.service.active_view().unwrap_or(0));
            // An asynchronous OpenFile may complete after a previous editor consumed
            // the initial focus request. Deliver focus to the actual resulting tab.
            self.focus_editor = true;
        }
        if !self.exit_when_saved
            && !self.workspace_switch_pending()
            && !self.service.is_busy()
            && let Some(action) = self.startup.pop_front()
        {
            if self.service.last_error.is_some()
                || self.service.db.last_error.is_some()
                || self.service.language.last_error.is_some()
            {
                self.startup.clear();
            } else {
                match action {
                    StartupAction::Browser(url) => self.open_browser_url(&url),
                    StartupAction::Command(command) => self.send(*command),
                    StartupAction::ExecuteSql => self.execute_sql(),
                    StartupAction::Problems => {
                        self.tool_tab = ToolTab::Problems;
                        self.show_activity = true;
                    }
                }
                ctx.request_repaint();
            }
        }
        if self.service.is_busy()
            || self.service.git_commit_messages.any_running()
            || self.service.any_project_providers_running()
            || !self.startup.is_empty()
            || self.git_repository_uis_pending()
            || self.service.debugger_has_sessions()
            || self.service.docker.is_busy()
            || self.service.python_tests.is_busy()
            || self.service.python.is_busy()
            || self.service.js_tests.is_busy()
            || self.service.js_tests.watch_pending
            || self.service.php_tests.is_busy()
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.service.workspace.is_some()
            || !self.service.documents.is_empty()
            || self.service.external_files_pending()
        {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.database_input.is_empty()
            && let Some(path) = &self.service.database_path
        {
            self.database_input = path.display().to_string();
        }
        if self.service.completion.selection.is_some() {
            self.completion_popup = None;
        }
        self.copilot_poll(&ctx);
        self.capture_signature_input(&ctx);
        self.activate_clicked_editor(&ctx);
        if !self.exit_when_saved
            && !self.workspace_switch_pending()
            && !self.folder_picker_pending()
        {
            self.shortcuts(&ctx);
        }
        if self.exit_after_resource && !self.service.rename_resources_busy() {
            self.exit_after_resource = false;
            self.request_exit();
        }
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_exit();
        }
        if self.exit_when_saved && !self.service.api.failed_saves.is_empty() {
            self.exit_when_saved = false;
            self.service.api.error = Some(
                "As coleções ainda não foram salvas. Use Tentar salvar novamente antes de sair."
                    .into(),
            );
        }
        self.shutdown_tick(&ctx);
        if self.exit_when_saved {
            ui.disable();
        }
        // Islands-style shell: the spaces between native resizable panels expose
        // the shell background instead of a black separator/frame stroke.
        ui.painter()
            .rect_filled(ui.max_rect(), 0., color(self.tokens.shell_background));
        let gap = self.tokens.shell_panel_gap.round().clamp(0., 24.) as i8;
        self.main_toolbar(ui);
        let status_panel = egui::Panel::bottom("codisseu.status")
            .exact_size(self.tokens.status_height)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(color(self.tokens.bar))
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12 + self.tokens.rail_width.round() as i8,
                        top: 4,
                        bottom: 4,
                    }),
            );
        self.collect_status_messages(&ctx);
        status_panel.show(ui, |ui| self.status_bar(ui));
        self.tool_rails(ui);
        let [left, top, right, bottom] = self.tokens.shell_body_insets;
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left,
                top,
                right,
                bottom,
            })
            .show(ui, |ui| {
                self.show_bottom_panel(ui);
                if self.show_activity
                    && self.tool_tab == ToolTab::Terminal
                    && self.terminal_ui.maximized
                {
                    return;
                }
                self.prepare_shell_size(ui, ShellSide::Right);
                if self.ai_chat.open {
                    let panel = egui::Panel::right("codisseu.database")
                        .resizable(true)
                        .show_separator_line(false)
                        .frame(self.shell_panel_frame(egui::Margin {
                            left: gap,
                            ..Default::default()
                        }))
                        .size_range(220.0..=600.0)
                        .show(ui, |ui| bounded_panel(ui, |ui| self.ai_panel(ui)));
                    self.remember_shell_size(ui, ShellSide::Right, panel.response.rect);
                    self.finish_shell_panel(
                        ui,
                        panel.response.rect,
                        egui::Margin {
                            left: gap,
                            ..Default::default()
                        },
                    );
                } else if self.jira_ui.open && self.service.jira.profile.enabled {
                    let panel = egui::Panel::right("codisseu.database")
                        .resizable(true)
                        .show_separator_line(false)
                        .frame(self.shell_panel_frame(egui::Margin {
                            left: gap,
                            ..Default::default()
                        }))
                        .size_range(260.0..=600.0)
                        .show(ui, |ui| bounded_panel(ui, |ui| self.jira_panel(ui)));
                    self.remember_shell_size(ui, ShellSide::Right, panel.response.rect);
                    self.finish_shell_panel(
                        ui,
                        panel.response.rect,
                        egui::Margin {
                            left: gap,
                            ..Default::default()
                        },
                    );
                } else if self.show_database {
                    let panel = egui::Panel::right("codisseu.database")
                        .resizable(true)
                        .show_separator_line(false)
                        .frame(self.shell_panel_frame(egui::Margin {
                            left: gap,
                            ..Default::default()
                        }))
                        .size_range(220.0..=600.0)
                        .show(ui, |ui| bounded_panel(ui, |ui| self.database_panel(ui)));
                    self.remember_shell_size(ui, ShellSide::Right, panel.response.rect);
                    self.finish_shell_panel(
                        ui,
                        panel.response.rect,
                        egui::Margin {
                            left: gap,
                            ..Default::default()
                        },
                    );
                }
                if self.show_project {
                    self.prepare_shell_size(ui, ShellSide::Left);
                    let panel = egui::Panel::left("codisseu.project")
                        .show_separator_line(false)
                        .frame(self.shell_panel_frame(egui::Margin {
                            right: gap,
                            ..Default::default()
                        }))
                        .size_range(150.0..=500.0)
                        .resizable(true)
                        .show(ui, |ui| {
                            bounded_panel(ui, |ui| {
                                if self.drawings_ui.open {
                                    self.drawings_panel(ui);
                                    return;
                                }
                                if self.api_ui.open {
                                    self.api_panel(ui);
                                    return;
                                }
                                if self.docker_ui.open {
                                    self.docker_panel(ui);
                                    return;
                                }
                                if self.git_ui.commit_open {
                                    self.git_commit_panel(ui);
                                    return;
                                }
                                self.project_panel(ui);
                            })
                        });
                    self.remember_shell_size(ui, ShellSide::Left, panel.response.rect);
                    self.finish_shell_panel(
                        ui,
                        panel.response.rect,
                        egui::Margin {
                            right: gap,
                            ..Default::default()
                        },
                    );
                }
                let panel = egui::CentralPanel::default()
                    .frame(self.shell_panel_frame(egui::Margin::ZERO))
                    .show(ui, |ui| {
                        self.editors(ui);
                    });
                self.finish_shell_panel(ui, panel.response.rect, egui::Margin::ZERO);
            });
        // The editor commits this frame's text input through Edit before Save/Undo
        // use the buffer. Commands within this batch retain their event order.
        for (id, action) in std::mem::take(&mut self.document_actions) {
            if action == "save" {
                self.save_document_or_choose_path(id);
            } else {
                self.send(match action {
                    "save" => unreachable!(),
                    "undo" => Command::Undo { id },
                    "redo" => Command::Redo { id },
                    _ => unreachable!(),
                });
            }
            self.focus_editor = true;
            ctx.request_repaint();
        }
        self.go_to_line_input(&ctx);
        self.editor_comparison_window(&ctx);
        self.terminal_root_picker(&ctx);
        self.update_signature(&ctx);
        self.signature_window(&ctx);
        self.definition_choices(&ctx);
        self.provider_choice_window(&ctx);
        self.documentation_window(&ctx);
        self.completion_window(&ctx);
        if !self.exit_when_saved
            && !self.workspace_switch_pending()
            && !self.folder_picker_pending()
        {
            self.automatic_completion(&ctx);
            self.copilot_tick(&ctx);
        }
        self.copilot_windows(&ctx);
        self.update_sql_completion(&ctx);
        self.sql_completion_window(&ctx);
        self.rename_dialog(&ctx);
        self.rename_conflicts(&ctx);
        self.related_methods_dialog(&ctx);
        self.dialogs(&ctx);
        self.providers_window(&ctx);
        self.api_windows(&ctx);
        self.ai_settings_window(&ctx);
        self.datasource_window(&ctx);
        self.database_edit_windows(&ctx);
        self.file_history_window(&ctx);
        self.php_tests_settings(&ctx);
        self.js_tests_settings(&ctx);
        self.python_tests_settings(&ctx);
        self.transfer_windows(&ctx);
        self.query_library_windows(&ctx);
        self.sql_productivity_window(&ctx);
        self.query_export_window(&ctx);
        self.git_windows(&ctx);
        self.git_insights_windows(&ctx);
        self.workspace_switch_window(&ctx);
        self.shutdown_window(&ctx);
        self.external_changes_window(&ctx);
        self.sync_git_draft();
        // The SCM TextEdit must commit this frame's input before an AI reply
        // compares and replaces its captured draft.
        self.git_commit_ai_tick(&ctx);
        self.copy_result_selection(&ctx);
        self.preferences_window(&ctx);
        self.jira_windows(&ctx);
        self.browser_saved_window(&ctx);
        self.debugger_dialog(&ctx);
        self.cargo_dialog(&ctx);
        self.python_dialog(&ctx);
        self.code_actions_window(&ctx);
        self.breakpoint_dialog(&ctx);
        self.snippets_window(&ctx);
        self.project_file_dialog(&ctx);
        self.project_search_window(&ctx);
        self.capture(&ctx);
    }
    fn shell_panel_frame(&self, margin: egui::Margin) -> egui::Frame {
        egui::Frame::new()
            .fill(color(self.tokens.shell_panel_background))
            .corner_radius(self.tokens.shell_panel_radius)
            .stroke(egui::Stroke::NONE)
            .inner_margin(0.)
            .outer_margin(margin)
    }
    fn finish_shell_panel(&self, ui: &egui::Ui, outer: egui::Rect, margin: egui::Margin) {
        let rect = egui::Rect::from_min_max(
            outer.min + egui::vec2(margin.left as f32, margin.top as f32),
            outer.max - egui::vec2(margin.right as f32, margin.bottom as f32),
        );
        let radius = self.tokens.shell_panel_radius as f32;
        if radius <= 0. {
            return;
        }
        // A clipped outer stroke masks only the four corner wedges. Rectangular
        // child backgrounds (tabs/terminal/scrollbars) cannot square the island,
        // and adjacent widgets/resize interactions are untouched.
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .rect_stroke(
                rect,
                self.tokens.shell_panel_radius,
                egui::Stroke::new(radius * 0.5, color(self.tokens.shell_background)),
                egui::StrokeKind::Outside,
            );
    }
    fn tabs(&mut self, ui: &mut egui::Ui, view_id: codisseu_core::ViewId) {
        let jira_active = self.jira_active_in_view(view_id);
        let mut activate = None;
        let mut reorder = None;
        let mut tab_rects = Vec::new();
        let mut close = None;
        let mut rename_query = None;
        let mut delete_query = None;
        // Preserve normal file-strip scrolling; only reveal a file when returning
        // from a Jira resource, or when its datasource label changes.
        let reveal_jira_file = self.jira_ui.tabs.take_file_reveal(view_id);
        let query_tab = self
            .service
            .view_document(view_id)
            .filter(|doc| {
                !self.api_ui.active_in_view(view_id)
                    && !self.browser_ui.active_in_view(view_id)
                    && !jira_active
                    && self
                        .service
                        .remote_db
                        .console_bindings
                        .contains_key(&doc.path)
            })
            .map(|doc| {
                (
                    doc.id,
                    database_queries::query_label(&self.service.remote_db, &doc.path, true),
                )
            });
        let identity = (query_tab, ui.available_width().round() as u32);
        let reveal_query = ui.ctx().data_mut(|data| {
            let key = egui::Id::unique(("database.query.tab.reveal", view_id));
            let previous = data.get_temp::<(Option<(DocumentId, String)>, u32)>(key);
            let changed = identity.0.is_some() && previous.as_ref() != Some(&identity);
            data.insert_temp(key, identity);
            changed
        });
        let members = self
            .service
            .view(view_id)
            .map(|v| v.tabs().to_vec())
            .unwrap_or_default();
        egui::Frame::new()
            .fill(color(self.tokens.editor))
            .show(ui, |ui| {
                let row = ui.available_rect_before_wrap();
                ui.painter().hline(
                    (row.left()
                        + if self
                            .service
                            .document_views()
                            .first()
                            .is_none_or(|v| v.id == view_id)
                            || self.service.editor_split().horizontal
                        {
                            1.
                        } else {
                            0.
                        })..=row.right(),
                    row.top() + self.tokens.tab_height - 0.5,
                    egui::Stroke::new(1., color(self.tokens.border)),
                );
                ui.allocate_ui_with_layout(
                    egui::vec2((row.width() - 36.).max(0.), self.tokens.tab_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::ScrollArea::horizontal()
                            .id_salt(("codisseu.tabs.scroll", view_id))
                            .codisseu()
                            .wheel_to_horizontal()
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.x = self.tokens.tab_spacing;
                                let mut tools_follow = false;
                                ui.horizontal_top(|ui| {
                                    if self
                                        .service
                                        .document_views()
                                        .first()
                                        .is_some_and(|v| v.id == view_id)
                                        || self.service.editor_split().horizontal
                                    {
                                        ui.add_space(1.);
                                    }
                                    for id in &members {
                                        let Some(doc) =
                                            self.service.documents.iter().find(|d| d.id == *id)
                                        else {
                                            continue;
                                        };
                                        ui.push_id((view_id, doc.id), |ui| {
                                            let name = doc
                                                .path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy();
                                            let display_name = database_queries::query_label(
                                                &self.service.remote_db,
                                                &doc.path,
                                                true,
                                            );
                                            let label = format!(
                                                "{}{}",
                                                display_name,
                                                if doc.is_dirty() { " •" } else { "" }
                                            );
                                            let active = self
                                                .service
                                                .view(view_id)
                                                .is_some_and(|v| v.document == doc.id)
                                                && !self.api_ui.active_in_view(view_id)
                                                && !self.browser_ui.active_in_view(view_id)
                                                && !jira_active;
                                            let font =
                                                FontId::proportional(self.tokens.ui_font_size);
                                            let text_width = ui
                                                .painter()
                                                .layout_no_wrap(
                                                    label.clone(),
                                                    font.clone(),
                                                    color(self.tokens.text),
                                                )
                                                .size()
                                                .x;
                                            let width = (text_width
                                                + self.tokens.tab_text_inset
                                                + self.tokens.tab_trailing_space)
                                                .max(90.0)
                                                .floor();
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(width, self.tokens.tab_height),
                                                egui::Sense::hover(),
                                            );
                                            tab_rects.push((doc.id, rect));
                                            let close_glyph = egui::Rect::from_center_size(
                                                egui::pos2(rect.right() - 16., rect.center().y),
                                                egui::Vec2::splat(self.tokens.tab_close_size),
                                            );
                                            // Inspector: the close component spans the visible
                                            // 41 px tab, although the icon itself is only 16 px.
                                            let close_rect = egui::Rect::from_min_max(
                                                egui::pos2(close_glyph.left(), rect.top() + 1.),
                                                egui::pos2(close_glyph.right(), rect.bottom()),
                                            );
                                            // Disjoint hit targets: closing a passive tab must not
                                            // briefly activate its document or steal the editor input.
                                            let body = egui::Rect::from_min_max(
                                                rect.min + egui::vec2(0., 1.),
                                                egui::pos2(close_rect.left(), rect.bottom()),
                                            );
                                            let response = ui.interact(
                                                body,
                                                egui::Id::unique((
                                                    "codisseu.tab.activate",
                                                    view_id,
                                                    doc.id,
                                                )),
                                                egui::Sense::click_and_drag(),
                                            );
                                            tab_reorder::begin(
                                                &mut self.tab_drag,
                                                view_id,
                                                doc.id,
                                                &label,
                                                &response,
                                            );
                                            response.widget_info(|| {
                                                egui::WidgetInfo::selected(
                                                    egui::accesskit::Role::Button,
                                                    ui.is_enabled(),
                                                    active,
                                                    &label,
                                                )
                                            });
                                            let query = self
                                                .service
                                                .remote_db
                                                .console_bindings
                                                .contains_key(&doc.path);
                                            if response.secondary_clicked() {
                                                ui.ctx().data_mut(|d| {
                                                    d.insert_temp(
                                                        response.id.with("keyboard_context"),
                                                        false,
                                                    )
                                                });
                                            }
                                            let mut context = egui::Popup::context_menu(&response);
                                            if ui
                                                .ctx()
                                                .data(|d| {
                                                    d.get_temp::<bool>(
                                                        response.id.with("keyboard_context"),
                                                    )
                                                })
                                                .unwrap_or(false)
                                            {
                                                context = context.anchor(response.rect);
                                            }
                                            context.show(|ui| {
                                                let index = members
                                                    .iter()
                                                    .position(|id| *id == doc.id)
                                                    .unwrap();
                                                if ui
                                                    .add_enabled(
                                                        index > 0,
                                                        egui::Button::new("Mover aba à esquerda"),
                                                    )
                                                    .clicked()
                                                {
                                                    reorder = Some(Command::ReorderTab {
                                                        view: view_id,
                                                        id: doc.id,
                                                        index: index - 1,
                                                    });
                                                    ui.close();
                                                }
                                                if ui
                                                    .add_enabled(
                                                        index + 1 < members.len(),
                                                        egui::Button::new("Mover aba à direita"),
                                                    )
                                                    .clicked()
                                                {
                                                    reorder = Some(Command::ReorderTab {
                                                        view: view_id,
                                                        id: doc.id,
                                                        index: index + 1,
                                                    });
                                                    ui.close();
                                                }
                                                if query {
                                                    ui.separator();
                                                    if ui.button("Renomear query…").clicked() {
                                                        rename_query = Some(doc.path.clone());
                                                        ui.close();
                                                    }
                                                    if ui.button("Excluir query…").clicked() {
                                                        delete_query = Some(doc.path.clone());
                                                        ui.close();
                                                    }
                                                }
                                            });
                                            let close_response = ui.interact(
                                                close_rect,
                                                egui::Id::unique((
                                                    "codisseu.tab.close",
                                                    view_id,
                                                    doc.id,
                                                )),
                                                egui::Sense::click(),
                                            );
                                            close_response.widget_info(|| {
                                                egui::WidgetInfo::labeled(
                                                    egui::accesskit::Role::Button,
                                                    ui.is_enabled(),
                                                    format!("Fechar {display_name}"),
                                                )
                                            });
                                            let body_hovered = response.hovered();
                                            let body_focused = response.has_focus();
                                            let label_opacity = if active
                                                || body_hovered
                                                || body_focused
                                                || close_response.hovered()
                                                || close_response.has_focus()
                                            {
                                                1.
                                            } else {
                                                self.tokens.tab_inactive_opacity
                                            };
                                            let icon_rect = egui::Rect::from_center_size(
                                                egui::pos2(rect.left() + 20., rect.center().y),
                                                egui::vec2(16., 16.),
                                            );
                                            if let Some(engine) = database_queries::query_engine(
                                                &self.service.remote_db,
                                                &doc.path,
                                            ) {
                                                components::paint_database_engine(
                                                    ui,
                                                    icon_rect,
                                                    Some(engine),
                                                    components::database_engine_tint(
                                                        engine,
                                                        &self.tokens,
                                                    )
                                                    .gamma_multiply(label_opacity),
                                                );
                                            } else if !self.tab_paint.badge(
                                                ui,
                                                icon_rect,
                                                &name,
                                                [
                                                    color(self.tokens.file_type_blue),
                                                    color(self.tokens.editor),
                                                    Color32::WHITE,
                                                ],
                                                label_opacity,
                                            ) {
                                                components::paint_document_icon_with_opacity(
                                                    ui,
                                                    icon_rect,
                                                    &name,
                                                    color(self.tokens.file_type_blue),
                                                    label_opacity,
                                                );
                                            }
                                            let label_anchor = egui::pos2(
                                                rect.left() + self.tokens.tab_text_inset,
                                                rect.center().y,
                                            );
                                            if !self.tab_labels.paint(
                                                ui,
                                                &label,
                                                label_anchor,
                                                close_glyph.left() - label_anchor.x - 2.,
                                                tab_label::LabelStyle {
                                                    foreground: if label_opacity < 1. {
                                                        tab_label::dimmed_color(
                                                            color(self.tokens.editor),
                                                            color(self.tokens.text),
                                                            self.tokens.tab_unselected_text_blend,
                                                        )
                                                    } else {
                                                        color(self.tokens.text)
                                                    },
                                                    background: color(self.tokens.editor),
                                                    opacity: 1.,
                                                    font_size: self.tokens.ui_font_size,
                                                },
                                            ) {
                                                let mut label_painter = ui.painter().clone();
                                                label_painter.multiply_opacity(label_opacity);
                                                label_painter.text(
                                                    label_anchor,
                                                    egui::Align2::LEFT_CENTER,
                                                    label,
                                                    font,
                                                    color(self.tokens.text),
                                                );
                                            }
                                            if response.has_focus() {
                                                ui.painter().rect_stroke(
                                                    body.shrink(2.),
                                                    2.,
                                                    egui::Stroke::new(
                                                        1.,
                                                        color(self.tokens.accent),
                                                    ),
                                                    egui::StrokeKind::Inside,
                                                );
                                            }
                                            if response
                                                .on_hover_text(format!(
                                                    "{display_name}\n{}",
                                                    doc.path.display()
                                                ))
                                                .clicked()
                                            {
                                                activate = Some(doc.id);
                                            }
                                            if active
                                                || body_hovered
                                                || body_focused
                                                || close_response.hovered()
                                                || close_response.has_focus()
                                            {
                                                let hovered = close_response.hovered()
                                                    || close_response.has_focus();
                                                self.tab_paint.close(
                                                    ui,
                                                    close_glyph,
                                                    tab_paint::CloseStyle {
                                                        foreground: color(if hovered {
                                                            self.tokens.tab_close_hover_foreground
                                                        } else {
                                                            self.tokens.tab_close_foreground
                                                        }),
                                                        background: color(self.tokens.editor),
                                                        hover_fill: color(
                                                            self.tokens.tab_close_hover_background,
                                                        ),
                                                        hovered,
                                                        padding: egui::vec2(
                                                            self.tokens.tab_close_padding[0],
                                                            self.tokens.tab_close_padding[1],
                                                        ),
                                                        stroke: self.tokens.tab_close_stroke,
                                                        hover_radius: self
                                                            .tokens
                                                            .tab_close_hover_radius,
                                                    },
                                                );
                                            }
                                            if close_response
                                                .on_hover_text(format!("Fechar {name}"))
                                                .clicked()
                                            {
                                                close = Some(doc.id);
                                            }
                                            if active {
                                                if reveal_query || reveal_jira_file {
                                                    ui.scroll_to_rect_animation(
                                                        rect,
                                                        None,
                                                        egui::style::ScrollAnimation::none(),
                                                    );
                                                }
                                                self.tab_paint.underline(
                                                    ui,
                                                    egui::Rect::from_min_max(
                                                        egui::pos2(rect.left(), rect.bottom() - 4.),
                                                        rect.right_bottom(),
                                                    ),
                                                    [
                                                        color(
                                                            if self.service.active_view()
                                                                == Some(view_id)
                                                            {
                                                                self.tokens.accent
                                                            } else {
                                                                self.tokens.inactive_tab_underline
                                                            },
                                                        ),
                                                        color(self.tokens.editor),
                                                        color(self.tokens.border),
                                                    ],
                                                    self.tokens.tab_underline_radius,
                                                    self.tokens.tab_underline_inset,
                                                );
                                            }
                                        });
                                    }
                                    let documents_end = ui.cursor().left();
                                    self.api_tabs(ui, view_id);
                                    self.browser_tabs(ui, view_id);
                                    self.jira_tabs(ui, view_id);
                                    tools_follow = ui.cursor().left() > documents_end;
                                });
                                if let Some(command) = tab_reorder::finish(
                                    ui,
                                    &mut self.tab_drag,
                                    view_id,
                                    &tab_rects,
                                    tab_reorder::DragStyle {
                                        accent: color(self.tokens.accent),
                                        background: color(self.tokens.editor),
                                        foreground: color(self.tokens.text),
                                        font_size: self.tokens.ui_font_size,
                                    },
                                    tools_follow,
                                ) {
                                    reorder = Some(command);
                                }
                            });
                    },
                );
                self.tab_group_menu(ui, view_id, row);
            });
        if let Some(path) = delete_query {
            self.delete_query_dialog(path);
        }
        if let Some(path) = rename_query {
            self.rename_query_dialog(path);
        }
        if let Some(id) = activate {
            self.browser_ui.deactivate(view_id);
            if self.api_ui.active_in_view(view_id) {
                self.api_ui.active = None;
            }
            self.send(Command::ActivateTab { view: view_id, id });
            self.focus_editor = true;
        }
        if let Some(id) = close {
            self.request_close_tab(view_id, id);
        }
        if let Some(command) = reorder {
            self.send(command);
            ui.ctx().request_repaint();
        }
    }
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.path_dialog
            && !matches!(action, PathAction::Workspace | PathAction::WorkspaceFile)
        {
            let title = match action {
                PathAction::Workspace => "Abrir pasta",
                PathAction::WorkspaceFile => "Abrir workspace",
                PathAction::File => "Abrir arquivo",
                PathAction::SaveUntitled(_) => "Salvar arquivo como",
            };
            let mut submit = false;
            let mut cancel = false;
            egui::Modal::new(egui::Id::unique("codisseu.path.dialog"))
                .frame(self.tokens.dialog_frame(ctx))
                .show(ctx, |ui| {
                    self.tokens.dialog_header(ui, ctx, 550., title);
                    let label = ui.label("Caminho");
                    let response = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.path_input)
                                .id(egui::Id::unique("codisseu.path.input"))
                                .desired_width(f32::INFINITY)
                                .min_size(egui::vec2(49.0, self.tokens.control_height)),
                        )
                        .labelled_by(label.id);
                    if self.path_focus {
                        response.request_focus();
                        self.path_focus = false;
                    }
                    // TextEdit releases focus on Enter; inspect that event only on this field.
                    let enter = response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    ui.add_space(8.0);
                    self.tokens.dialog_footer(ui, |ui| {
                        submit = ui
                            .add_enabled(
                                !self.path_input.trim().is_empty(),
                                self.tokens.dialog_button(
                                    if matches!(action, PathAction::SaveUntitled(_)) {
                                        "Salvar arquivo"
                                    } else {
                                        "Abrir caminho"
                                    },
                                    true,
                                ),
                            )
                            .clicked()
                            || (enter && !self.path_input.trim().is_empty());
                        cancel = ui
                            .add(self.tokens.dialog_button("Cancelar", false))
                            .clicked();
                    });
                });
            if submit {
                let path = PathBuf::from(&self.path_input);
                self.send(match action {
                    PathAction::Workspace | PathAction::WorkspaceFile => {
                        Command::OpenWorkspace(path)
                    }
                    PathAction::File => Command::OpenFile(path),
                    PathAction::SaveUntitled(id) => Command::SaveUntitled { id, path },
                });
                self.path_dialog = None;
                self.focus_editor = true;
            } else if cancel || ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.path_dialog = None;
                if matches!(action, PathAction::SaveUntitled(_)) {
                    self.cancel_close_decision();
                }
                self.focus_editor = true;
                ctx.request_repaint();
            }
        }
        self.unsaved_window(ctx);
    }

    fn capture(&mut self, ctx: &egui::Context) {
        if self.capture_path.is_some()
            && !self.capture_requested
            && !self.workspace_switch_pending()
            && !self.folder_picker_pending()
            && self.startup.is_empty()
            && self.service.db.last_error.is_none()
            && self.service.language.last_error.is_none()
            && !self.service.is_busy()
            && !self.service.project_versions_pending()
            && self.service.last_error.is_none()
            && self.browser_capture_ready()
            && (!self.wait_for_diagnostics
                || self
                    .service
                    .documents
                    .iter()
                    .filter(|d| {
                        matches!(
                            d.path.extension().and_then(|s| s.to_str()),
                            Some("php" | "ts" | "py" | "pyi" | "pyw")
                        )
                    })
                    .all(|d| {
                        self.service.language.analyzed_versions.get(&d.path) == Some(&d.version)
                            && (!self.wait_for_nonempty_diagnostics
                                || self
                                    .service
                                    .language
                                    .diagnostics
                                    .iter()
                                    .any(|diagnostic| diagnostic.path == d.path))
                    }))
        {
            self.capture_stable_frames += 1;
            if self.capture_stable_frames < 3 {
                ctx.request_repaint();
                return;
            }
            let pixels_per_point = ctx.pixels_per_point();
            let frame = ctx.cumulative_pass_nr();
            let observed = serde_json::json!({
                "workspace": self.service.workspace,
                "workspace_file": self.service.workspace_file,
                "workspace_roots": self.service.workspace_roots(),
                "git_selected_root": self.service.git_selected_repository(),
                "git_repositories": self.service.git.repositories.iter().map(|r| &r.root).collect::<Vec<_>>(),
                "git_error": self.service.git.error,
                "project_versions": self.service.project_versions(),
                "language_status": if self.browser_active().is_some() { "Browser".to_owned() } else { self.language_status_text().0 },
                "documents": self.service.documents.iter().map(|d| serde_json::json!({"path":d.path,"project_root":d.project_root(),"encoding":d.encoding().label(),"version":d.version,"dirty":d.is_dirty()})).collect::<Vec<_>>(),
                "editor": self.editor_metrics(),
                "editor_views": self.editor_view_metrics(),
                "active_view": self.service.active_view(),
                "editor_split": self.service.editor_split(),
                "view_state": self.service.document_views().iter().map(|v|serde_json::json!({"view":v.id,"document":v.document,"path":self.service.view_document(v.id).map(|d|&d.path),"tabs":v.tabs().iter().filter_map(|id|self.service.documents.iter().find(|d|d.id==*id).map(|d|&d.path)).collect::<Vec<_>>(),"selection":[v.selection().anchor,v.selection().head],"scroll":v.scroll()})).collect::<Vec<_>>(),
                "sqlite_connected": self.service.db.connected,
                "sqlite_schema": self.service.db.schema.iter().map(|t|t.name.clone()).collect::<Vec<_>>(),
                "sqlite_result_rows": self.service.db.result.as_ref().map(|r|r.rows.len()),
                "providers": self.service.language.statuses,
                "provider_startup": self.service.provider_startup(),
                "diagnostics": self.service.language.diagnostics,
                "browser": self.browser_capture_state(),
                "analyzed_versions": self.service.language.analyzed_versions,
                "unversioned_paths": self.service.language.unversioned_paths,
                "capture_requires_nonempty_diagnostics": self.wait_for_nonempty_diagnostics,
                "analysis_note":"analyzed_versions is client-associated; unversioned_paths are not server-confirmed document versions",
                "panel": match self.tool_tab {ToolTab::Terminal => "terminal", ToolTab::Results => "results", ToolTab::Problems=>"problems",ToolTab::Activity=>"activity",ToolTab::References=>"references",ToolTab::Rename=>"rename",ToolTab::Git=>"git",ToolTab::Debug=>"debug",ToolTab::Docker=>"docker",ToolTab::PhpTests=>"php-tests",ToolTab::JsTests=>"js-tests",ToolTab::PythonTests=>"python-tests"}
            });
            let path = self.capture_path.take().expect("capture configured");
            let (tx, rx) = mpsc::sync_channel(1);
            self.capture_result = Some(rx);
            ctx.request_screenshot(move |image| {
                std::thread::spawn(move || {
                    let result = (|| {
                        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
                        let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        image::save_buffer(&path,&bytes,image.width() as u32,image.height() as u32,image::ColorType::Rgba8).map_err(|e| e.to_string())?;
                        let metadata = serde_json::json!({"source":"eframe native viewport framebuffer readback", "width_px": image.width(), "height_px": image.height(), "pixels_per_point": pixels_per_point, "requested_at_pass": frame, "readiness":"startup complete; I/O idle; no error; three rendered passes; optional real diagnostics gate", "observed": observed});
                        std::fs::write(path.with_extension("json"), serde_json::to_vec_pretty(&metadata).map_err(|e|e.to_string())?).map_err(|e|e.to_string())
                    })();
                    let _ = tx.send(result);
                });
            });
            self.capture_requested = true;
        }
        if let Some(rx) = &self.capture_result {
            if let Ok(result) = rx.try_recv() {
                self.capture_result = None;
                match result {
                    Ok(()) => {
                        self.capture_done = true;
                        if self.close_after_capture {
                            self.request_exit();
                            ctx.request_repaint();
                        }
                    }
                    Err(e) => self.service.last_error = Some(format!("Falha ao capturar: {e}")),
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(20));
            }
        }
    }
}
impl eframe::App for CodisseuApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.parent_native_folder(frame);
        self.show(ui);
    }
}

#[cfg(test)]
mod php_editor_navigation_tests;
