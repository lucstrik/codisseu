//! Licensed public IDE icons plus original tool fallbacks; provenance in reference/public/ide-icons/.
use eframe::egui::{self, Color32, Pos2, Rect, Response, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub(super) enum Icon {
    Clock,
    Drawing,
    MergeAcceptLeft,
    MergeAcceptRight,
    MergeIgnore,
    MergeNonconflictingLeft,
    MergeNonconflictingAll,
    MergeNonconflictingRight,
    MergeSimple,
    Jira,
    Api,
    Docker,
    GitInspect,
    GitCompare,
    GitHistory,
    GitCopy,
    GitVersions,
    GitReveal,
    Ai,
    Brain,
    AiSend,
    Delete,
    Shield,
    MoreDots,
    Pause,
    Debug,
    Build,
    Browser,
    StepOver,
    StepInto,
    StepOut,
    MuteBreakpoints,
    Up,
    Down,
    Branch,
    Tag,
    Star,
    Filter,
    ExpandAll,
    CollapseAll,
    Locate,
    Changes,
    Stash,
    Menu,
    Search,
    Terminal,
    Code,
    Folder,
    NewFile,
    OpenFile,
    NewFolder,
    File,
    Save,
    Play,
    Settings,
    Close,
    Refresh,
    Database,
    Problems,
    Disconnect,
    Schema,
    Table,
    Column,
    PrimaryKey,
    Copy,
    Add,
    Remove,
    Stop,
    Submit,
    SqlPreview,
    Revert,
    Commit,
    Rollback,
    Export,
    Import,
    Compare,
    View,
    Previous,
    FirstPage,
    Next,
}

/// Shared filename-based artwork in the project tree, tabs, Git and search results.
#[path = "document_icons.rs"]
mod document_icons;

pub(super) fn paint_document_icon(ui: &Ui, rect: Rect, name: &str, tint: Color32) {
    paint_document_icon_with_opacity(ui, rect, name, tint, 1.);
}
pub(super) fn paint_document_icon_with_opacity(
    ui: &Ui,
    rect: Rect,
    name: &str,
    tint: Color32,
    opacity: f32,
) {
    if codisseu_core::drawings::is_drawing(std::path::Path::new(name)) {
        paint_icon(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(16.)),
            Icon::Drawing,
            tint.gamma_multiply(opacity),
        );
        return;
    }
    document_icons::paint(ui, rect, document_icons::file_asset(name), 16., opacity);
}

pub(super) fn paint_markdown_icon(ui: &Ui, rect: Rect, icon: u8) {
    let name = match icon {
        0 => "markdown-source",
        1 => "markdown-split",
        2 => "markdown-preview",
        3 => "markdown-sync",
        4 => "remove",
        _ => "add",
    };
    document_icons::paint(ui, rect, name, 16., 1.);
}

fn public_icon(icon: Icon) -> Option<&'static str> {
    Some(match icon {
        Icon::Drawing => "excalidraw-symbol",
        Icon::MergeAcceptLeft => "merge-accept-left",
        Icon::MergeAcceptRight => "merge-accept-right",
        Icon::MergeIgnore => "merge-ignore",
        Icon::MergeNonconflictingLeft => "merge-nonconflicting-left",
        Icon::MergeNonconflictingAll => "merge-nonconflicting-all",
        Icon::MergeNonconflictingRight => "merge-nonconflicting-right",
        Icon::MergeSimple => "merge-simple",
        Icon::Docker => "docker-tool-window",
        Icon::GitInspect => "gitlens-git-commit",
        Icon::GitCompare => "gitlens-git-compare",
        Icon::GitHistory => "gitlens-history",
        Icon::GitCopy => "gitlens-copy",
        Icon::GitVersions => "gitlens-versions",
        Icon::GitReveal => "gitlens-search",
        Icon::Ai => "ai-symbol",
        Icon::Brain => "ai-brain",
        Icon::Shield => "ai-shield",
        Icon::AiSend => "ai-send",
        Icon::Delete => "ai-delete",
        Icon::MoreDots => "more",
        Icon::Pause => "pause",
        Icon::Debug => "debug",
        Icon::Build => "build",
        Icon::StepOver => "step-over",
        Icon::StepInto => "step-into",
        Icon::StepOut => "step-out",
        Icon::MuteBreakpoints => "mute-breakpoints",
        Icon::Up => "up",
        Icon::Down => "down",
        Icon::Branch => "branch",
        Icon::Tag => "tag",
        Icon::Star => "star",
        Icon::Filter => "filter",
        Icon::ExpandAll => "expand-all",
        Icon::CollapseAll => "collapse-all",
        Icon::Locate => "locate",
        Icon::Changes => "changes",
        Icon::Stash => "stash",
        Icon::Menu => "menu",
        Icon::Search => "search",
        Icon::Terminal => "terminal",
        Icon::Code => "code",
        Icon::Folder => "folder",
        Icon::NewFile => "new-file",
        Icon::OpenFile => "open-file",
        Icon::NewFolder => "new-folder",
        Icon::File => "file-unknown",
        Icon::Save => "save",
        Icon::Play => "play",
        Icon::Settings => "settings",
        Icon::Close => "close",
        Icon::Refresh => "refresh",
        Icon::Problems => "problems",
        Icon::Schema => "schema",
        Icon::Table => "table",
        Icon::Column => "column",
        Icon::Copy => "copy",
        Icon::Add => "add",
        Icon::Remove => "remove",
        Icon::Stop => "stop",
        Icon::Submit => "submit",
        Icon::SqlPreview => "sql-preview",
        Icon::Revert => "revert",
        Icon::Commit => "commit",
        Icon::Rollback => "rollback",
        Icon::Export => "export",
        Icon::Import => "import",
        Icon::Compare => "compare",
        Icon::View => "view",
        Icon::Previous => "previous",
        Icon::Next => "next",
        // Original request/response artwork and database-specific fallbacks.
        Icon::Clock
        | Icon::Jira
        | Icon::Api
        | Icon::Browser
        | Icon::Database
        | Icon::Disconnect
        | Icon::PrimaryKey
        | Icon::FirstPage => {
            return None;
        }
    })
}

/// Tool-window variant: licensed geometry, current monochrome UI foreground.
pub(super) fn paint_icon_monochrome(ui: &Ui, rect: Rect, icon: Icon, tint: Color32) {
    if let Some(asset) = public_icon(icon) {
        let side = if rect.width().min(rect.height()) >= 34. {
            20.
        } else {
            16.
        };
        document_icons::paint_monochrome(ui, rect, asset, side, tint);
    } else {
        paint_icon(ui, rect, icon, tint);
    }
}

pub(super) fn paint_icon(ui: &Ui, rect: Rect, icon: Icon, tint: Color32) {
    if matches!(icon, Icon::Drawing) {
        // Exact MIT-licensed Excalidraw LogoIcon silhouette; use the same
        // monochrome foreground, disabled opacity and size cache as other tools.
        let side = if rect.width().min(rect.height()) >= 34. {
            20.
        } else {
            16.
        };
        document_icons::paint_monochrome(ui, rect, "excalidraw-symbol", side, tint);
        return;
    }
    if let Some(asset) = public_icon(icon) {
        let side = if matches!(icon, Icon::Brain | Icon::Shield | Icon::AiSend)
            || rect.width().min(rect.height()) >= 34.
        {
            20.
        } else {
            16.
        };
        document_icons::paint(
            ui,
            rect,
            asset,
            side,
            if ui.is_enabled() { 1. } else { 0.4 },
        );
        return;
    }
    let side =
        rect.width()
            .min(rect.height())
            .min(if matches!(icon, Icon::Browser | Icon::PrimaryKey) {
                16.0
            } else {
                20.0
            });
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y) * (side / 20.0);
    let stroke = Stroke::new(
        if matches!(icon, Icon::Browser | Icon::PrimaryKey) {
            1.2
        } else {
            1.4
        },
        tint,
    );
    let line = |points: &[(f32, f32)]| {
        ui.painter().add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ));
    };
    match icon {
        Icon::Drawing => {}
        Icon::Clock => {
            // Original monochrome clock, following the shared icon box/stroke.
            ui.painter().circle_stroke(p(10., 10.), side * 0.36, stroke);
            line(&[(10., 5.5), (10., 10.), (13., 12.)]);
        }
        Icon::Jira => {
            // Original task glyph; no Atlassian trademark asset.
            line(&[(8., 4.), (18., 4.)]);
            line(&[(8., 10.), (18., 10.)]);
            line(&[(8., 16.), (18., 16.)]);
            line(&[(2., 4.), (3.5, 5.5), (6., 2.5)]);
            line(&[(2., 10.), (3.5, 11.5), (6., 8.5)]);
            line(&[(2., 16.), (3.5, 17.5), (6., 14.5)]);
        }
        Icon::Browser => {
            // Original globe artwork, using the existing monochrome icon stroke.
            ui.painter().circle_stroke(p(10., 10.), side * 0.36, stroke);
            line(&[(10., 2.8), (7., 6.), (6., 10.), (7., 14.), (10., 17.2)]);
            line(&[(10., 2.8), (13., 6.), (14., 10.), (13., 14.), (10., 17.2)]);
            line(&[(3., 10.), (17., 10.)]);
        }
        Icon::Api => {
            // Original request/response icon in the shared 20-unit rail box.
            line(&[(2., 6.), (18., 6.)]);
            line(&[(14., 2.), (18., 6.), (14., 10.)]);
            line(&[(18., 14.), (2., 14.)]);
            line(&[(6., 10.), (2., 14.), (6., 18.)]);
        }
        Icon::Build
        | Icon::MergeAcceptLeft
        | Icon::MergeAcceptRight
        | Icon::MergeIgnore
        | Icon::MergeNonconflictingLeft
        | Icon::MergeNonconflictingAll
        | Icon::MergeNonconflictingRight
        | Icon::MergeSimple => {}
        Icon::Docker
        | Icon::GitInspect
        | Icon::GitCompare
        | Icon::GitHistory
        | Icon::GitCopy
        | Icon::GitVersions
        | Icon::GitReveal => {} // Public SVG path above.
        Icon::Ai | Icon::Brain | Icon::AiSend | Icon::Delete => {
            // Original abstract chat/agent symbol, no lettering or star.
            line(&[
                (3., 3.),
                (16., 3.),
                (18., 5.),
                (18., 13.),
                (16., 15.),
                (7., 15.),
                (3., 18.),
                (3., 15.),
                (2., 13.),
                (2., 5.),
                (3., 3.),
            ]);
            line(&[(6., 7.), (9., 9.5), (6., 12.)]);
            line(&[(11., 12.), (15., 12.)]);
        }
        Icon::Shield => {
            line(&[
                (10., 2.),
                (17., 5.),
                (16., 12.),
                (13., 16.),
                (10., 18.),
                (7., 16.),
                (4., 12.),
                (3., 5.),
                (10., 2.),
            ]);
            line(&[(10., 6.), (10., 11.)]);
            ui.painter().circle_filled(p(10., 14.), 0.8, tint);
        }
        Icon::NewFile | Icon::NewFolder => {
            if matches!(icon, Icon::NewFolder) {
                line(&[
                    (2., 16.),
                    (2., 5.),
                    (8., 5.),
                    (10., 7.),
                    (16., 7.),
                    (16., 10.),
                ]);
            } else {
                line(&[(3., 17.), (3., 3.), (10., 3.), (14., 7.), (14., 10.)]);
                line(&[(10., 3.), (10., 7.), (14., 7.)]);
            }
            line(&[(15., 11.), (15., 19.)]);
            line(&[(11., 15.), (19., 15.)]);
        }
        Icon::Debug => {
            ui.painter().rect_stroke(
                Rect::from_min_max(p(6., 6.), p(14., 16.)),
                4.,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(8., 5.), (7., 2.)]);
            line(&[(12., 5.), (13., 2.)]);
            line(&[(6., 8.), (2., 6.)]);
            line(&[(14., 8.), (18., 6.)]);
            line(&[(6., 11.), (2., 11.)]);
            line(&[(14., 11.), (18., 11.)]);
            line(&[(6., 14.), (3., 17.)]);
            line(&[(14., 14.), (17., 17.)]);
        }
        Icon::StepOver => {
            line(&[
                (3., 10.),
                (3., 6.),
                (6., 3.),
                (12., 3.),
                (16., 7.),
                (16., 11.),
            ]);
            line(&[(12., 8.), (16., 12.), (19., 8.)]);
            ui.painter().circle_filled(p(5., 15.), side * 0.1, tint);
        }
        Icon::StepInto => {
            line(&[(10., 2.), (10., 13.)]);
            line(&[(5., 9.), (10., 14.), (15., 9.)]);
            line(&[(5., 18.), (15., 18.)]);
        }
        Icon::StepOut => {
            line(&[(10., 16.), (10., 4.)]);
            line(&[(5., 8.), (10., 3.), (15., 8.)]);
            line(&[(4., 18.), (16., 18.)]);
        }
        Icon::MuteBreakpoints => {
            ui.painter().circle_stroke(p(10., 10.), side * 0.31, stroke);
            line(&[(3., 17.), (17., 3.)]);
        }
        Icon::Filter => {
            line(&[
                (3., 3.),
                (17., 3.),
                (11.5, 9.5),
                (11.5, 15.5),
                (8.5, 18.),
                (8.5, 9.5),
                (3., 3.),
            ]);
        }
        Icon::Star => {
            let points = (0..10)
                .map(|i| {
                    let angle =
                        std::f32::consts::TAU * i as f32 / 10. - std::f32::consts::FRAC_PI_2;
                    let radius = if i % 2 == 0 { 8. } else { 3.6 };
                    p(10. + radius * angle.cos(), 10. + radius * angle.sin())
                })
                .collect::<Vec<_>>();
            ui.painter().add(egui::Shape::closed_line(points, stroke));
        }
        Icon::Tag => {
            line(&[
                (3., 3.),
                (10., 3.),
                (18., 11.),
                (11., 18.),
                (3., 10.),
                (3., 3.),
            ]);
            ui.painter()
                .circle_stroke(p(7., 7.), side * 0.075, Stroke::new(1., tint));
        }
        Icon::ExpandAll | Icon::CollapseAll => {
            let expand = matches!(icon, Icon::ExpandAll);
            for (y, dir) in [
                (6., if expand { -1. } else { 1. }),
                (14., if expand { 1. } else { -1. }),
            ] {
                line(&[(6., y - 2. * dir), (10., y + 2. * dir), (14., y - 2. * dir)]);
            }
        }
        Icon::Locate => {
            ui.painter()
                .circle_stroke(p(10., 10.), side * 0.29, Stroke::new(1., tint));
            for points in [
                [(10., 1.), (10., 6.)],
                [(10., 14.), (10., 19.)],
                [(1., 10.), (6., 10.)],
                [(14., 10.), (19., 10.)],
            ] {
                line(&points);
            }
        }
        Icon::Pause => {
            line(&[(7., 4.), (7., 16.)]);
            line(&[(13., 4.), (13., 16.)]);
        }
        Icon::Up => {
            line(&[(5., 12.), (10., 7.), (15., 12.)]);
        }
        Icon::Down => {
            line(&[(5., 8.), (10., 13.), (15., 8.)]);
        }
        Icon::MoreDots => {
            for y in [5., 10., 15.] {
                ui.painter().circle_filled(p(10., y), side * 0.065, tint);
            }
        }
        Icon::Branch => {
            line(&[(5., 5.), (5., 15.)]);
            line(&[(5., 11.), (14., 7.), (14., 4.)]);
            for (x, y) in [(5., 3.), (5., 17.), (14., 3.)] {
                ui.painter().circle_stroke(p(x, y), side * 0.105, stroke);
            }
        }
        Icon::Changes => {
            line(&[(2., 10.), (6., 10.)]);
            line(&[(14., 10.), (18., 10.)]);
            ui.painter().circle_stroke(p(10., 10.), side * 0.2, stroke);
        }
        Icon::Stash => {
            line(&[(3., 11.), (3., 17.), (17., 17.), (17., 11.)]);
            line(&[(6., 8.), (10., 12.), (14., 8.)]);
            line(&[(10., 3.), (10., 12.)]);
            line(&[(2., 11.), (6., 11.)]);
            line(&[(14., 11.), (18., 11.)]);
        }
        Icon::Export | Icon::Import | Icon::Compare | Icon::View => {
            paint_database_icon(ui, rect, icon, tint)
        }
        Icon::Copy => {
            line(&[(3., 13.), (3., 3.), (13., 3.)]);
            line(&[(7., 7.), (17., 7.), (17., 17.), (7., 17.), (7., 7.)]);
        }
        Icon::Add => {
            line(&[(10., 4.), (10., 16.)]);
            line(&[(4., 10.), (16., 10.)]);
        }
        Icon::Remove => line(&[(4., 10.), (16., 10.)]),
        Icon::Stop => {
            ui.painter()
                .rect_filled(egui::Rect::from_min_max(p(5., 5.), p(15., 15.)), 1., tint);
        }
        Icon::Submit => {
            line(&[(10., 16.), (10., 4.)]);
            line(&[(5., 9.), (10., 4.), (15., 9.)]);
            line(&[(4., 17.), (16., 17.)]);
        }
        Icon::SqlPreview => {
            line(&[(3., 2.), (12., 2.), (16., 6.), (16., 9.)]);
            line(&[(12., 2.), (12., 6.), (16., 6.)]);
            line(&[(3., 2.), (3., 18.), (8., 18.)]);
            line(&[(6., 6.), (9., 6.)]);
            line(&[(6., 9.), (11., 9.)]);
            ui.painter().circle_stroke(p(13., 13.), side * 0.17, stroke);
            line(&[(15.5, 15.5), (18., 18.)]);
        }
        Icon::Revert => {
            line(&[(8., 4.), (3., 8.), (8., 12.)]);
            line(&[(3., 8.), (12., 8.), (16., 11.), (16., 15.), (12., 18.)]);
        }
        Icon::Commit => line(&[(3., 10.), (8., 15.), (17., 5.)]),
        Icon::Rollback => {
            line(&[(8., 3.), (3., 7.), (8., 11.)]);
            line(&[
                (3., 7.),
                (12., 7.),
                (17., 11.),
                (17., 15.),
                (13., 18.),
                (7., 18.),
            ]);
            line(&[(11., 12.), (14., 15.)]);
            line(&[(14., 12.), (11., 15.)]);
        }
        Icon::Previous => line(&[(13., 4.), (7., 10.), (13., 16.)]),
        Icon::FirstPage => {
            line(&[(6., 4.), (6., 16.)]);
            line(&[(14., 4.), (8., 10.), (14., 16.)]);
        }
        Icon::Next => line(&[(7., 4.), (13., 10.), (7., 16.)]),
        Icon::Folder => line(&[
            (2., 16.),
            (2., 4.),
            (8., 4.),
            (10., 6.),
            (18., 6.),
            (18., 16.),
            (2., 16.),
        ]),
        Icon::OpenFile => {} // Licensed Codicon go-to-file above.
        Icon::File => {
            line(&[
                (4., 2.),
                (12., 2.),
                (16., 6.),
                (16., 18.),
                (4., 18.),
                (4., 2.),
            ]);
            line(&[(12., 2.), (12., 6.), (16., 6.)]);
        }
        Icon::Save => {
            line(&[
                (3., 3.),
                (15., 3.),
                (17., 5.),
                (17., 17.),
                (3., 17.),
                (3., 3.),
            ]);
            line(&[(6., 3.), (6., 8.), (13., 8.), (13., 3.)]);
            line(&[(6., 17.), (6., 12.), (14., 12.), (14., 17.)]);
        }
        Icon::Play => {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![p(6., 3.), p(17., 10.), p(6., 17.)],
                tint,
                Stroke::NONE,
            ));
        }
        Icon::Settings => {
            let mut outline = Vec::new();
            for tooth in 0..8 {
                for (phase, radius) in [(0., 6.2), (0.16, 8.), (0.52, 8.), (0.68, 6.2)] {
                    let angle = (tooth as f32 + phase) * std::f32::consts::TAU / 8.;
                    outline.push(p(10. + radius * angle.cos(), 10. + radius * angle.sin()));
                }
            }
            outline.push(outline[0]);
            ui.painter()
                .add(egui::Shape::line(outline, Stroke::new(1.1, tint)));
            ui.painter()
                .circle_stroke(p(10., 10.), side * 0.15, Stroke::new(1.1, tint));
        }
        Icon::Menu => {
            for y in [5., 10., 15.] {
                line(&[(3., y), (17., y)]);
            }
        }
        Icon::Search => {
            ui.painter().circle_stroke(p(8.5, 8.5), side * 0.29, stroke);
            line(&[(13., 13.), (18., 18.)]);
        }
        Icon::Terminal => {
            line(&[(2., 3.), (18., 3.), (18., 17.), (2., 17.), (2., 3.)]);
            line(&[(5., 7.), (8., 10.), (5., 13.)]);
            line(&[(10., 13.), (14., 13.)]);
        }
        Icon::Code => {
            line(&[(6., 5.), (2., 10.), (6., 15.)]);
            line(&[(14., 5.), (18., 10.), (14., 15.)]);
            line(&[(12., 3.), (8., 17.)]);
        }
        Icon::Close => {
            line(&[(5., 5.), (15., 15.)]);
            line(&[(15., 5.), (5., 15.)]);
        }
        Icon::Refresh => {
            line(&[
                (16., 7.),
                (13., 3.),
                (7., 3.),
                (3., 7.),
                (3., 13.),
                (7., 17.),
                (13., 17.),
                (17., 13.),
            ]);
            line(&[(16., 2.), (16., 7.), (11., 7.)]);
        }
        Icon::Database => {
            let ellipse = |cy| {
                let pts: Vec<Pos2> = (0..=24)
                    .map(|i| {
                        let a = i as f32 / 24. * std::f32::consts::TAU;
                        p(10. + 7. * a.cos(), cy + 2.5 * a.sin())
                    })
                    .collect();
                ui.painter().add(egui::Shape::line(pts, stroke));
            };
            ellipse(5.);
            line(&[(3., 5.), (3., 15.)]);
            line(&[(17., 5.), (17., 15.)]);
            ellipse(15.);
        }
        Icon::Problems => {
            ui.painter().circle_stroke(p(10., 10.), 7., stroke);
            line(&[(10., 5.), (10., 11.)]);
            ui.painter().circle_filled(p(10., 14.), 0.9, tint);
        }
        Icon::Disconnect => {
            line(&[
                (5., 3.),
                (5., 7.),
                (3., 7.),
                (3., 11.),
                (7., 14.),
                (11., 11.),
                (11., 7.),
                (9., 7.),
                (9., 3.),
            ]);
            line(&[(7., 14.), (7., 18.)]);
            line(&[(14., 5.), (18., 9.)]);
            line(&[(18., 5.), (14., 9.)]);
        }
        Icon::Schema => {
            line(&[(3., 3.), (17., 3.), (17., 8.), (3., 8.), (3., 3.)]);
            line(&[(10., 8.), (10., 12.)]);
            line(&[(4., 17.), (4., 12.), (16., 12.), (16., 17.)]);
            line(&[(2., 17.), (6., 17.)]);
            line(&[(14., 17.), (18., 17.)]);
        }
        Icon::Table => {
            line(&[(3., 3.), (17., 3.), (17., 17.), (3., 17.), (3., 3.)]);
            line(&[(3., 8.), (17., 8.)]);
            line(&[(3., 12.), (17., 12.)]);
            line(&[(8., 8.), (8., 17.)]);
        }
        Icon::Column => {
            line(&[(6., 3.), (14., 3.), (14., 17.), (6., 17.), (6., 3.)]);
            line(&[(6., 8.), (14., 8.)]);
            line(&[(6., 12.), (14., 12.)]);
        }
        Icon::PrimaryKey => {
            ui.painter().circle_stroke(p(6., 7.), side * 0.2, stroke);
            line(&[(9., 10.), (16., 17.), (18., 15.)]);
            line(&[(13., 14.), (15., 12.)]);
        }
    }
}

pub(super) fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    size: f32,
) -> Response {
    let response = ui.add_sized([size, size], egui::Button::new("").frame(false));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
    });
    let bg = if response.is_pointer_button_down_on() {
        Color32::from_white_alpha(38)
    } else if response.hovered() {
        Color32::from_white_alpha(22)
    } else if selected && size >= 40. {
        static RAIL_SELECTION: std::sync::OnceLock<Color32> = std::sync::OnceLock::new();
        *RAIL_SELECTION.get_or_init(|| super::color(super::Tokens::load().rail_selection))
    } else if selected {
        ui.visuals().selection.stroke.color
    } else {
        Color32::TRANSPARENT
    };
    if matches!(icon, Icon::AiSend) {
        ui.painter().circle_filled(
            response.rect.center(),
            size / 2.,
            if ui.is_enabled() {
                Color32::from_gray(55)
            } else {
                Color32::from_gray(39)
            },
        );
    } else {
        ui.painter().rect_filled(
            response.rect.shrink(if size >= 40. {
                5.
            } else if size == 26. {
                2.
            } else {
                3.
            }),
            6.0,
            bg,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.shrink(2.),
            6.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let tint = if !ui.is_enabled() {
        ui.visuals().weak_text_color()
    } else if matches!(icon, Icon::Play | Icon::Commit) {
        Color32::from_rgb(95, 173, 101)
    } else {
        ui.visuals().text_color()
    };
    paint_icon(ui, response.rect, icon, tint);
    response.on_hover_text(label)
}

pub(super) fn icon_toggle_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    size: f32,
) -> Response {
    let response = icon_button(ui, icon, label, selected, size);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::accesskit::Role::Button,
            ui.is_enabled(),
            selected,
            label,
        )
    });
    response
}

pub(super) fn header<R>(
    ui: &mut Ui,
    height: f32,
    content: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    let out = ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(height);
            content(ui)
        },
    );
    ui.painter().hline(
        out.response.rect.x_range(),
        out.response.rect.bottom(),
        ui.visuals().widgets.noninteractive.bg_stroke,
    );
    out
}

pub(super) fn tree_row(
    ui: &mut Ui,
    label: &str,
    name: &str,
    depth: usize,
    folder: Option<bool>,
    selected: bool,
    selection_active: bool,
) -> Response {
    tree_row_impl(
        ui,
        None,
        label,
        TreeObjectRow {
            name,
            detail: "",
            description: "",
            depth,
            expanded: folder,
            selected,
            icon: if folder.is_some() {
                Icon::Folder
            } else {
                Icon::File
            },
        },
        None,
        selection_active,
    )
}

pub(super) struct TreeObjectRow<'a> {
    pub name: &'a str,
    pub detail: &'a str,
    pub description: &'a str,
    pub depth: usize,
    pub expanded: Option<bool>,
    pub selected: bool,
    pub icon: Icon,
}

/// Database-only icon palette/geometry with the common accessible tree behavior.
pub(super) fn database_object_row(
    ui: &mut Ui,
    id: egui::Id,
    row: TreeObjectRow<'_>,
    tint: Color32,
    key_tint: Color32,
) -> Response {
    tree_row_impl(
        ui,
        Some(id),
        row.name,
        row,
        Some((tint, key_tint, None)),
        false,
    )
}

/// Query rows share the catalog geometry and use their owning source's motif.
pub(super) fn database_query_row(
    ui: &mut Ui,
    id: egui::Id,
    row: TreeObjectRow<'_>,
    engine: Option<codisseu_core::remote::Engine>,
    tokens: &super::Tokens,
) -> Response {
    let tint = engine.map_or_else(
        || super::color(tokens.muted),
        |engine| database_engine_tint(engine, tokens),
    );
    tree_row_impl(
        ui,
        Some(id),
        row.name,
        row,
        Some((tint, super::color(tokens.muted), engine)),
        false,
    )
}
pub(super) fn database_engine_tint(
    engine: codisseu_core::remote::Engine,
    tokens: &super::Tokens,
) -> Color32 {
    use codisseu_core::remote::Engine;
    super::color(match engine {
        Engine::Mysql | Engine::AuroraMysql => tokens.database_engine_mysql,
        Engine::Postgres | Engine::AuroraPostgres => tokens.database_engine_postgres,
        Engine::Mongo => tokens.database_engine_mongo,
        Engine::Redis => tokens.database_engine_redis,
        Engine::Memcached => tokens.database_engine_memcached,
    })
}

fn tree_row_impl(
    ui: &mut Ui,
    stable_id: Option<egui::Id>,
    label: &str,
    row: TreeObjectRow<'_>,
    database_tints: Option<(Color32, Color32, Option<codisseu_core::remote::Engine>)>,
    selection_active: bool,
) -> Response {
    let font = egui::FontId::proportional(13.);
    let text =
        ui.painter()
            .layout_no_wrap(row.name.into(), font.clone(), ui.visuals().text_color());
    let detail =
        ui.painter()
            .layout_no_wrap(row.detail.into(), font, ui.visuals().weak_text_color());
    let detail_width = if row.detail.is_empty() {
        0.
    } else {
        8. + detail.size().x
    };
    let inset = if stable_id.is_none() { 24. } else { 12. };
    let required_width = inset + row.depth as f32 * 16. + 34. + text.size().x + detail_width + 12.;
    let size = egui::vec2(ui.available_width().max(required_width).max(100.), 24.);
    let response = if let Some(id) = stable_id {
        let (_, rect) = ui.allocate_space(size);
        ui.interact(rect, id, egui::Sense::click())
    } else {
        ui.add_sized(size, egui::Button::new("").frame(false))
    };
    if stable_id.is_some() && response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        if response.gained_focus() {
            response.scroll_to_me(None);
            ui.ctx().request_repaint();
        }
    }
    response.widget_info(|| {
        if stable_id.is_some() {
            egui::WidgetInfo::labeled(egui::accesskit::Role::TreeItem, ui.is_enabled(), label)
        } else {
            egui::WidgetInfo::selected(
                egui::accesskit::Role::Button,
                ui.is_enabled(),
                row.selected,
                label,
            )
        }
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_selected(row.selected);
        if let Some(expanded) = row.expanded {
            node.set_expanded(expanded);
        }
        if stable_id.is_some() {
            node.set_level(row.depth + 1);
            node.set_description(row.description);
        }
    });
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    if row.selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            response
                .rect
                .shrink2(egui::vec2(if stable_id.is_none() { 12. } else { 4. }, 0.)),
            4.,
            if row.selected && stable_id.is_none() && !selection_active && !response.has_focus() {
                static INACTIVE: std::sync::OnceLock<Color32> = std::sync::OnceLock::new();
                *INACTIVE.get_or_init(|| {
                    super::color(super::Tokens::load().references_inactive_selection)
                })
            } else if row.selected {
                ui.visuals().selection.bg_fill
            } else {
                Color32::from_white_alpha(22)
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response
                .rect
                .shrink2(egui::vec2(if stable_id.is_none() { 12. } else { 4. }, 0.)),
            4.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let left = response.rect.left() + inset + row.depth as f32 * 16.;
    let center = response.rect.center().y;
    if let Some(expanded) = row.expanded {
        let points = if expanded {
            vec![
                egui::pos2(left, center - 2.),
                egui::pos2(left + 4., center + 2.),
                egui::pos2(left + 8., center - 2.),
            ]
        } else {
            vec![
                egui::pos2(left + 2., center - 4.),
                egui::pos2(left + 6., center),
                egui::pos2(left + 2., center + 4.),
            ]
        };
        ui.painter().add(egui::Shape::line(
            points,
            Stroke::new(1.2, ui.visuals().weak_text_color()),
        ));
    }
    let icon_rect = Rect::from_center_size(egui::pos2(left + 23., center), Vec2::splat(16.));
    if let Some((tint, key_tint, engine)) = database_tints {
        if let Some(engine) = engine {
            paint_database_engine(ui, icon_rect, Some(engine), tint);
        } else if matches!(row.icon, Icon::Database) && row.depth == 0 {
            // SQLite is the only database root rendered by this object-row adapter.
            paint_database_engine(ui, icon_rect, None, tint);
        } else if matches!(row.icon, Icon::Column | Icon::PrimaryKey) {
            paint_database_column(
                ui,
                icon_rect,
                matches!(row.icon, Icon::PrimaryKey),
                tint,
                key_tint,
            );
        } else {
            paint_database_icon(ui, icon_rect, row.icon, tint);
        }
    } else if stable_id.is_none() && matches!(row.icon, Icon::File) {
        static FILE_BLUE: std::sync::OnceLock<Color32> = std::sync::OnceLock::new();
        paint_document_icon(
            ui,
            icon_rect,
            row.name,
            *FILE_BLUE.get_or_init(|| super::color(super::Tokens::load().file_type_blue)),
        );
    } else {
        paint_icon(
            ui,
            icon_rect,
            row.icon,
            if matches!(row.icon, Icon::Folder | Icon::Schema | Icon::Column) {
                Color32::from_rgb(157, 160, 168)
            } else {
                Color32::from_rgb(108, 149, 235)
            },
        );
    }
    let text_pos = egui::pos2(left + 34., center - text.size().y / 2.);
    let detail_pos = egui::pos2(
        text_pos.x + text.size().x + 8.,
        center - detail.size().y / 2.,
    );
    ui.painter()
        .galley(text_pos, text, ui.visuals().text_color());
    if !row.detail.is_empty() {
        ui.painter()
            .galley(detail_pos, detail, ui.visuals().weak_text_color());
    }
    response
}

/// Database-only glyphs: 16 logical pixels and a thin 1.15px stroke. General IDE
/// icons retain their existing geometry in `paint_icon` above.
pub(super) fn paint_database_icon(ui: &Ui, rect: Rect, icon: Icon, tint: Color32) {
    if let Some(asset) = public_icon(icon) {
        document_icons::paint(ui, rect, asset, 16., if ui.is_enabled() { 1. } else { 0.4 });
        return;
    }
    let side = rect.width().min(rect.height()).min(16.);
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y) * (side / 16.);
    let stroke = Stroke::new(1.15, tint);
    let line = |points: &[(f32, f32)]| {
        ui.painter().add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ))
    };
    let box_line = |x1, y1, x2, y2| {
        ui.painter().rect_stroke(
            Rect::from_min_max(p(x1, y1), p(x2, y2)),
            1.,
            stroke,
            egui::StrokeKind::Inside,
        )
    };
    match icon {
        Icon::Export | Icon::Import => {
            line(&[(2., 10.), (2., 14.), (14., 14.), (14., 10.)]);
            line(&[(8., 2.), (8., 11.)]);
            if matches!(icon, Icon::Export) {
                line(&[(4., 7.), (8., 11.), (12., 7.)]);
            } else {
                line(&[(4., 6.), (8., 2.), (12., 6.)]);
            }
        }
        Icon::Compare => {
            box_line(1., 3., 6., 13.);
            box_line(10., 3., 15., 13.);
            line(&[(6., 6.), (10., 6.)]);
            line(&[(6., 10.), (10., 10.)]);
        }
        Icon::View => {
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(1., 8.), p(5., 0.), p(11., 0.), p(15., 8.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(1., 8.), p(5., 16.), p(11., 16.), p(15., 8.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            ui.painter().circle_stroke(p(8., 8.), 2.5, stroke);
        }
        Icon::Copy => {
            line(&[(2., 11.), (2., 2.), (11., 2.)]);
            box_line(5., 5., 14., 14.);
        }
        Icon::Add => {
            line(&[(8., 3.), (8., 13.)]);
            line(&[(3., 8.), (13., 8.)]);
        }
        Icon::Remove => {
            line(&[(3., 8.), (13., 8.)]);
        }
        Icon::Close => {
            line(&[(4., 4.), (12., 12.)]);
            line(&[(12., 4.), (4., 12.)]);
        }
        Icon::Play => {
            ui.painter().add(egui::Shape::convex_polygon(
                vec![p(4., 2.5), p(13., 8.), p(4., 13.5)],
                tint,
                Stroke::NONE,
            ));
        }
        Icon::Stop => {
            ui.painter()
                .rect_filled(Rect::from_min_max(p(4., 4.), p(12., 12.)), 1., tint);
        }
        Icon::Submit => {
            line(&[(8., 13.), (8., 3.)]);
            line(&[(4.5, 6.5), (8., 3.), (11.5, 6.5)]);
            line(&[(3., 14.), (13., 14.)]);
        }
        Icon::SqlPreview => {
            // Eye + small upload arrow, observed in Preview Pending Changes.
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(1., 7.), p(4., 1.5), p(10., 1.5), p(13., 7.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(1., 7.), p(4., 12.5), p(10., 12.5), p(13., 7.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            ui.painter()
                .circle_stroke(p(7., 7.), 2. * side / 16., stroke);
            let arrow = Stroke::new(
                1.15,
                if ui.is_enabled() {
                    Color32::from_rgb(95, 173, 101)
                } else {
                    tint
                },
            );
            ui.painter().line_segment([p(12., 15.), p(12., 9.)], arrow);
            ui.painter().add(egui::Shape::line(
                vec![p(9.5, 11.5), p(12., 9.), p(14.5, 11.5)],
                arrow,
            ));
        }
        Icon::Revert => {
            line(&[(5.5, 3.), (2., 6.5), (5.5, 10.)]);
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(2.5, 6.5), p(16., 2.), p(16., 15.), p(6., 13.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
        }
        Icon::Commit => {
            line(&[(2.5, 8.), (6.5, 12.), (13.5, 4.)]);
        }
        Icon::Rollback => {
            line(&[(5., 2.5), (2., 5.5), (5., 8.5)]);
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(2.5, 5.5), p(16., 1.), p(16., 15.), p(6., 13.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
        }
        Icon::Previous => {
            line(&[(10., 3.), (5., 8.), (10., 13.)]);
        }
        Icon::FirstPage => {
            line(&[(3., 3.), (3., 13.)]);
            line(&[(12., 3.), (7., 8.), (12., 13.)]);
        }
        Icon::Next => {
            line(&[(6., 3.), (11., 8.), (6., 13.)]);
        }
        Icon::Schema => {
            box_line(1.5, 1.5, 5.5, 5.5);
            box_line(10.5, 1.5, 14.5, 5.5);
            box_line(1.5, 10.5, 5.5, 14.5);
            box_line(10.5, 10.5, 14.5, 14.5);
            line(&[(5.5, 3.5), (10.5, 3.5)]);
            line(&[(3.5, 5.5), (3.5, 10.5)]);
            line(&[(12.5, 5.5), (12.5, 10.5)]);
            line(&[(5.5, 12.5), (10.5, 12.5)]);
        }
        Icon::Table => {
            box_line(1.5, 2., 14.5, 14.);
            line(&[(2., 6.), (14., 6.)]);
            line(&[(2., 10.), (14., 10.)]);
            line(&[(6., 2.5), (6., 13.5)]);
            line(&[(10., 6.), (10., 13.5)]);
        }
        Icon::Column => {
            box_line(2., 2., 14., 14.);
            ui.painter().rect_filled(
                Rect::from_min_max(p(3., 3.), p(6., 13.)),
                0.,
                tint.gamma_multiply(0.24),
            );
            line(&[(6.5, 2.5), (6.5, 13.5)]);
        }
        Icon::PrimaryKey => {
            ui.painter()
                .circle_stroke(p(5., 5.), 3. * side / 16., stroke);
            line(&[(7.2, 7.2), (13.5, 13.5), (15., 12.)]);
            line(&[(11., 11.), (12.5, 9.5)]);
        }
        Icon::Refresh => {
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(13., 5.), p(6., -2.), p(-1., 5.), p(4., 11.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            ui.painter()
                .add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [p(3., 11.), p(10., 18.), p(17., 11.), p(12., 5.)],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            line(&[(9., 5.), (13., 5.), (13., 1.)]);
            line(&[(7., 11.), (3., 11.), (3., 15.)]);
        }
        Icon::Code => {
            box_line(1., 2., 15., 14.);
            line(&[(4., 6.), (6.5, 8.), (4., 10.)]);
            line(&[(8., 10.), (11., 10.)]);
        }
        _ => paint_icon(ui, rect, icon, tint),
    }
}

/// A database action preserves its accessible label even though its presentation
/// is icon-only. The 26px pitch and 22px hover tile follow the measured toolbar.
pub(super) fn database_icon_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        // The datasource form has 28px controls. Toolbar tiles keep their own
        // measured 26px pitch without changing the parent form or IDE buttons.
        ui.spacing_mut().interact_size.y = 26.;
        ui.spacing_mut().button_padding.y = 0.;
        let response = ui.add_sized([26., 26.], egui::Button::new("").frame(false));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
        });
        let background = if response.is_pointer_button_down_on() {
            Color32::from_white_alpha(35)
        } else if response.hovered() {
            Color32::from_white_alpha(22)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(response.rect.shrink(2.), 3., background);
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.shrink(2.),
                3.,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
        let tint = if !ui.is_enabled() {
            ui.visuals().weak_text_color()
        } else if matches!(icon, Icon::Play | Icon::Submit | Icon::Commit) {
            Color32::from_rgb(95, 173, 101)
        } else if matches!(icon, Icon::Stop | Icon::Rollback) {
            super::color(super::Tokens::load().error)
        } else {
            ui.visuals().text_color()
        };
        paint_database_icon(ui, response.rect, icon, tint);
        response.on_hover_text(label)
    })
    .inner
}

/// Independent procedural motifs, not extracted vendor logos. `None` is SQLite.
/// Colors come from the caller's engine tokens; shapes also distinguish engines.
pub(super) fn paint_database_engine(
    ui: &Ui,
    rect: Rect,
    engine: Option<codisseu_core::remote::Engine>,
    tint: Color32,
) {
    use codisseu_core::remote::Engine;
    let side = rect.width().min(rect.height()).min(16.);
    let rect = Rect::from_center_size(rect.center(), Vec2::splat(side));
    let p = |x: f32, y: f32| rect.min + egui::vec2(x, y) * (side / 16.);
    let stroke = Stroke::new(1.1, tint);
    let line = |points: &[(f32, f32)]| {
        ui.painter().add(egui::Shape::line(
            points.iter().map(|&(x, y)| p(x, y)).collect(),
            stroke,
        ))
    };
    let curve = |points: [(f32, f32); 4]| {
        ui.painter()
            .add(egui::epaint::CubicBezierShape::from_points_stroke(
                points.map(|(x, y)| p(x, y)),
                false,
                Color32::TRANSPARENT,
                stroke,
            ))
    };
    match engine {
        Some(Engine::Mysql | Engine::AuroraMysql) => {
            // A small swimming-animal outline with dorsal fin and separate tail.
            curve([(1.5, 10.5), (4., 2.), (10., 3.), (14.5, 8.5)]);
            curve([(14.5, 8.5), (12., 8.), (11., 9.), (9., 11.)]);
            curve([(9., 11.), (8., 11.5), (8., 8.), (6., 8.5)]);
            line(&[(6., 8.5), (4.5, 12.5), (3.5, 10.), (1.5, 10.5)]);
            line(&[(7., 4.5), (9., 2.), (9., 5.)]);
            line(&[(13.5, 8.5), (15., 10.5)]);
            ui.painter()
                .circle_filled(p(11.5, 6.7), 0.65 * side / 16., tint);
        }
        Some(Engine::Postgres | Engine::AuroraPostgres) => {
            // Rounded head, two ears and a hanging trunk; no logo contour reused.
            curve([(4., 11.5), (-0.5, 6.), (1.5, 1.5), (6., 3.)]);
            curve([(10., 3.), (14.5, 1.5), (17., 6.), (12., 11.5)]);
            curve([(6., 3.), (7., 1.5), (9., 1.5), (10., 3.)]);
            curve([(5., 6.), (4., 9.), (7., 11.), (6.5, 14.)]);
            curve([(10.5, 6.), (11., 10.), (11., 15.5), (8., 14.)]);
            line(&[(8., 14.), (8., 9.)]);
            ui.painter()
                .circle_filled(p(5.7, 5.4), 0.65 * side / 16., tint);
            ui.painter()
                .circle_filled(p(10.2, 5.4), 0.65 * side / 16., tint);
        }
        Some(Engine::Mongo) => {
            let mut leaf = Vec::new();
            for i in 0..=12 {
                let t = i as f32 / 12.;
                leaf.push(p(8. + 4.2 * (std::f32::consts::PI * t).sin(), 1. + 12. * t));
            }
            for i in (0..=12).rev() {
                let t = i as f32 / 12.;
                leaf.push(p(8. - 3.7 * (std::f32::consts::PI * t).sin(), 1. + 12. * t));
            }
            ui.painter().add(egui::Shape::convex_polygon(
                leaf,
                tint.gamma_multiply(0.3),
                stroke,
            ));
            line(&[(8., 3.), (8., 15.)]);
            line(&[(8., 9.), (10.5, 6.5)]);
        }
        Some(Engine::Redis) => {
            for y in [11., 7., 3.] {
                let polygon = vec![p(1., y + 1.), p(8., y - 1.), p(15., y + 1.), p(8., y + 4.)];
                ui.painter().add(egui::Shape::convex_polygon(
                    polygon,
                    tint.gamma_multiply(0.32),
                    stroke,
                ));
            }
            ui.painter()
                .circle_filled(p(5.3, 3.8), 0.7 * side / 16., tint);
            line(&[(9., 3.), (12., 4.)]);
        }
        Some(Engine::Memcached) => {
            ui.painter().rect_stroke(
                Rect::from_min_max(p(2.5, 3.), p(13.5, 13.)),
                2.,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(&[(5., 10.5), (5., 5.5), (8., 8.5), (11., 5.5), (11., 10.5)]);
            for n in [5., 8., 11.] {
                line(&[(n, 1.5), (n, 3.)]);
                line(&[(n, 13.), (n, 14.5)]);
            }
        }
        None => {
            ui.painter().rect_filled(
                Rect::from_min_max(p(2., 2.), p(10., 14.)),
                1.,
                tint.gamma_multiply(0.25),
            );
            line(&[(2., 14.), (2., 2.), (10., 2.), (10., 4.)]);
            curve([(4.5, 12.5), (6., 4.), (12., 0.5), (14., 1.5)]);
            curve([(14., 1.5), (13., 9.), (9., 10.), (6., 11.)]);
            line(&[(4.5, 14.), (10.5, 5.)]);
        }
    }
    if matches!(engine, Some(Engine::AuroraMysql | Engine::AuroraPostgres)) {
        ui.painter()
            .circle_filled(p(12.7, 12.7), 2.6 * side / 16., ui.visuals().panel_fill);
        line(&[(12.7, 10.7), (12.7, 14.7)]);
        line(&[(10.7, 12.7), (14.7, 12.7)]);
    }
}

/// Column motif with the small PK badge used by SQL completion and explorer rows.
/// Both colors belong to the caller's theme, not the language/LSP icon palette.
pub(super) fn paint_database_column(
    ui: &Ui,
    rect: Rect,
    primary_key: bool,
    tint: Color32,
    key_tint: Color32,
) {
    paint_database_icon(ui, rect, Icon::Column, tint);
    if primary_key {
        let side = rect.width().min(rect.height()).min(16.);
        let square = Rect::from_center_size(rect.center(), Vec2::splat(side));
        let p = |x: f32, y: f32| square.min + egui::vec2(x, y) * (side / 16.);
        let stroke = Stroke::new(1.1, key_tint);
        ui.painter()
            .circle_stroke(p(10., 10.), 1.7 * side / 16., stroke);
        ui.painter()
            .line_segment([p(11.2, 11.2), p(14.5, 14.5)], stroke);
        ui.painter()
            .line_segment([p(13., 13.), p(14., 12.)], stroke);
    }
}

#[cfg(test)]
mod database_icon_tests {
    use super::*;
    use egui_kittest::{
        Harness,
        kittest::{NodeT, Queryable},
    };
    #[test]
    fn database_action_tiles_are_26px_named_and_respect_disabled_state() {
        let mut h = Harness::builder().with_size([300., 80.]).build_ui_state(
            |ui, count: &mut usize| {
                super::super::Tokens::load().form_style(ui);
                ui.horizontal(|ui| {
                    if database_icon_button(ui, Icon::Play, "Execute", true).clicked() {
                        *count += 1;
                    }
                    database_icon_button(ui, Icon::Stop, "Stop", false);
                });
            },
            0usize,
        );
        h.run();
        let run = h.get_by_role_and_label(egui::accesskit::Role::Button, "Execute");
        assert_eq!(run.rect().size(), Vec2::splat(26.));
        assert!(h.get_by_label("Stop").accesskit_node().is_disabled());
        run.click();
        h.run();
        assert_eq!(*h.state(), 1);
    }
    #[test]
    #[ignore = "explicit wgpu widget render for icon visual review"]
    fn database_glyph_sheet_renders_with_distinct_engine_motifs() {
        use codisseu_core::remote::Engine;
        let tokens = super::super::Tokens::load();
        let mut h = Harness::builder().with_size([1000., 280.]).build_ui(|ui| {
            ui.visuals_mut().override_text_color = Some(super::super::color(tokens.text));
            ui.label("Codisseu native database vector glyphs / 16px");
            ui.horizontal(|ui| {
                for engine in Engine::ALL {
                    ui.vertical(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::hover());
                        let tint = match engine {
                            Engine::Mysql | Engine::AuroraMysql => tokens.database_engine_mysql,
                            Engine::Postgres | Engine::AuroraPostgres => {
                                tokens.database_engine_postgres
                            }
                            Engine::Mongo => tokens.database_engine_mongo,
                            Engine::Redis => tokens.database_engine_redis,
                            Engine::Memcached => tokens.database_engine_memcached,
                        };
                        paint_database_engine(ui, rect, Some(engine), super::super::color(tint));
                        ui.label(engine.label());
                    });
                }
                ui.vertical(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.), egui::Sense::hover());
                    paint_database_engine(ui, rect, None, Color32::from_rgb(102, 179, 222));
                    ui.label("SQLite");
                });
            });
            ui.add_space(12.);
            ui.horizontal(|ui| {
                for (icon, label) in [
                    (Icon::Play, "Execute"),
                    (Icon::Stop, "Stop"),
                    (Icon::Submit, "Submit"),
                    (Icon::SqlPreview, "Preview"),
                    (Icon::Revert, "Revert"),
                    (Icon::Commit, "Commit"),
                    (Icon::Rollback, "Rollback"),
                    (Icon::Refresh, "Refresh"),
                    (Icon::Schema, "Schema"),
                    (Icon::Table, "Table"),
                    (Icon::Column, "Column"),
                    (Icon::PrimaryKey, "Key"),
                ] {
                    ui.vertical(|ui| {
                        database_icon_button(ui, icon, label, true);
                        ui.label(label);
                        database_icon_button(ui, icon, &format!("disabled {label}"), false);
                    });
                }
            });
            ui.horizontal(|ui| {
                for pk in [false, true] {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(26.), egui::Sense::hover());
                    paint_database_column(
                        ui,
                        rect,
                        pk,
                        ui.visuals().text_color(),
                        Color32::from_rgb(224, 172, 69),
                    );
                    ui.label(if pk { "column + PK" } else { "column" });
                }
            });
        });
        h.run();
        let image = h.render().expect("native wgpu widget render");
        let path = std::env::var("CODISSEU_DB_ICON_RENDER").expect("explicit local artifact path");
        image.save(path).expect("save icon visual review artifact");
    }
}
