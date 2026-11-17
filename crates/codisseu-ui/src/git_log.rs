use super::git::GitActionKind;
use super::*;
use codisseu_core::git::{BranchInfo, CommitEntry, GitCommand, LocalAction, TagInfo};
use std::collections::BTreeMap;

/// Tree grouping is presentation only: each leaf keeps the exact ref identity.
#[derive(Default)]
struct RefTree {
    children: BTreeMap<String, RefTree>,
    leaves: Vec<RefLeaf>,
}
#[derive(Clone)]
enum RefLeaf {
    Branch(BranchInfo),
    Tag(TagInfo),
}
impl RefLeaf {
    fn name(&self) -> &str {
        match self {
            Self::Branch(b) => &b.name,
            Self::Tag(t) => &t.name,
        }
    }
}
impl RefTree {
    fn insert(&mut self, leaf: RefLeaf) {
        let components = leaf
            .name()
            .split('/')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut node = self;
        for component in components.iter().take(components.len().saturating_sub(1)) {
            node = node.children.entry(component.clone()).or_default();
        }
        node.leaves.push(leaf);
    }
}

/// These caches own at most the bounded service page/ref lists, never every scroll frame.
pub(super) struct RefRowsCache {
    key: CacheKey,
    rows: Vec<RefRow>,
}
enum RefRow {
    Head,
    Group {
        path: String,
        name: String,
        depth: usize,
        open: bool,
    },
    Leaf(RefLeaf, usize),
}
pub(super) struct CommitRowsCache {
    key: CacheKey,
    indices: Vec<usize>,
    dates: Vec<String>,
    max_lane: usize,
}
#[derive(PartialEq)]
struct CacheKey {
    generation: u64,
    revision: u64,
    filter: String,
    counts: [usize; 3],
    minute: u64,
}
impl CodisseuApp {
    fn git_cache_key(&self, filter: &str, commits: bool) -> CacheKey {
        CacheKey {
            generation: self.service.git.repo_generation,
            revision: self.service.git.revision,
            filter: filter.into(),
            counts: if commits {
                self.service
                    .git
                    .history
                    .as_ref()
                    .map_or([0, 0, 0], |h| [1, h.skip, h.commits.len()])
            } else {
                [
                    self.service.git.branches.len(),
                    self.service.git.tags.len(),
                    self.service.git.branch_preferences.favorites.len(),
                ]
            },
            minute: if commits {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() / 60)
            } else {
                0
            },
        }
    }
    fn git_ref_rows(&mut self) -> std::sync::Arc<RefRowsCache> {
        let key = self.git_cache_key(&self.git_ui.branch_filter, false);
        if let Some(cache) = &self.git_ui.refs_cache
            && cache.key == key
        {
            return cache.clone();
        }
        let query = key.filter.to_lowercase();
        let mut local = RefTree::default();
        let mut remote = RefTree::default();
        let mut tags = RefTree::default();
        for branch in &self.service.git.branches {
            if query.is_empty() || branch.name.to_lowercase().contains(&query) {
                if branch.remote {
                    remote.insert(RefLeaf::Branch(branch.clone()));
                } else {
                    local.insert(RefLeaf::Branch(branch.clone()));
                }
            }
        }
        for tag in &self.service.git.tags {
            if query.is_empty() || tag.name.to_lowercase().contains(&query) {
                tags.insert(RefLeaf::Tag(tag.clone()));
            }
        }
        let mut rows = Vec::new();
        if query.is_empty() {
            rows.push(RefRow::Head);
        }
        for (name, tree) in [("Local", local), ("Remote", remote), ("Tags", tags)] {
            flatten_refs(
                &self.git_ui.groups,
                &self.service.git.branch_preferences.favorites,
                &mut rows,
                name,
                &tree,
                0,
                !query.is_empty(),
            );
        }
        let cache = std::sync::Arc::new(RefRowsCache { key, rows });
        self.git_ui.refs_cache = Some(cache.clone());
        cache
    }
    fn git_commit_rows(&mut self) -> std::sync::Arc<CommitRowsCache> {
        let key = self.git_cache_key(&self.git_ui.filter, true);
        if let Some(cache) = &self.git_ui.commits_cache
            && cache.key == key
        {
            return cache.clone();
        }
        let query = key.filter.to_lowercase();
        let mut indices = Vec::new();
        let mut dates = Vec::new();
        let mut max_lane = 0;
        if let Some(page) = &self.service.git.history {
            for (index, c) in page.commits.iter().enumerate() {
                max_lane = max_lane.max(c.lane);
                if query.is_empty()
                    || c.subject.to_lowercase().contains(&query)
                    || c.author.to_lowercase().contains(&query)
                    || c.oid.starts_with(&query)
                {
                    indices.push(index);
                    dates.push(git_log_date(c.timestamp));
                }
            }
        }
        let cache = std::sync::Arc::new(CommitRowsCache {
            key,
            indices,
            dates,
            max_lane,
        });
        self.git_ui.commits_cache = Some(cache.clone());
        cache
    }
}
fn flatten_refs(
    groups: &std::collections::HashMap<String, bool>,
    favorites: &[String],
    rows: &mut Vec<RefRow>,
    path: &str,
    tree: &RefTree,
    depth: usize,
    searching: bool,
) {
    let open = searching || *groups.get(&format!("log.tree.{path}")).unwrap_or(&true);
    rows.push(RefRow::Group {
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or(path).into(),
        depth,
        open,
    });
    if !open {
        return;
    }
    let favorite = |leaf: &RefLeaf| matches!(leaf,RefLeaf::Branch(b) if favorites.contains(&branch_reference(b)));
    for leaf in tree.leaves.iter().filter(|leaf| favorite(leaf)) {
        rows.push(RefRow::Leaf(leaf.clone(), depth));
    }
    for (name, child) in &tree.children {
        flatten_refs(
            groups,
            favorites,
            rows,
            &format!("{path}/{name}"),
            child,
            depth + 1,
            searching,
        );
    }
    for leaf in tree.leaves.iter().filter(|leaf| !favorite(leaf)) {
        rows.push(RefRow::Leaf(leaf.clone(), depth));
    }
}

impl CodisseuApp {
    pub(super) fn git_log_panel(&mut self, ui: &mut egui::Ui) {
        if self.git_ui.tab == 1 {
            egui::ScrollArea::both()
                .id_salt("git.console")
                .codisseu()
                .show(ui, |ui| {
                    ui.monospace(&self.git_ui.output);
                });
            return;
        }
        self.git_operation_banner(ui);
        // Progress/errors live in the fixed tool header, so rows never jump.
        let height = ui.available_height().max(60.);
        let width = ui.available_width();
        // Reference geometry is measured at 1400px. Narrow windows retain all
        // controls without imposing the reference's fixed desktop width.
        let rail = self.tokens.git_log_toolbar_width;
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(rail, height),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(rail);
                        self.git_log_rail(ui);
                    },
                );
                log_divider(ui, height, color(self.tokens.border));
                // The rail can toggle visibility above. Measure from the same
                // state used below, including the frame that receives the click.
                let branch_width = if self.git_ui.branches {
                    self.tokens
                        .git_log_branch_width
                        .min((width * 0.29).max(170.))
                } else {
                    0.
                };
                let detail_width = self
                    .tokens
                    .git_log_details_width
                    .min((width - rail - branch_width) * 0.42);
                let table_width = (width - rail - branch_width - detail_width - 3.).max(120.);
                if self.git_ui.branches {
                    ui.allocate_ui_with_layout(
                        egui::vec2(branch_width, height),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_min_width(branch_width);
                            ui.push_id("git.log.branch.column", |ui| {
                                self.git_log_branches(ui, branch_width, height)
                            });
                        },
                    );
                    log_divider(ui, height, color(self.tokens.border));
                }
                ui.allocate_ui_with_layout(
                    egui::vec2(table_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(table_width);
                        ui.push_id("git.log.history.column", |ui| {
                            self.git_log_filters(ui, table_width);
                            ui.add_enabled_ui(!self.git_history_loading(), |ui| {
                                self.git_log_commits(
                                    ui,
                                    table_width,
                                    (height - self.tokens.git_log_filter_height).max(20.),
                                )
                            });
                        });
                    },
                );
                log_divider(ui, height, color(self.tokens.border));
                ui.allocate_ui_with_layout(
                    egui::vec2(detail_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(detail_width);
                        ui.push_id("git.log.detail.column", |ui| {
                            ui.add_enabled_ui(!self.git_history_loading(), |ui| {
                                self.git_log_details(ui, height)
                            });
                        });
                    },
                );
            });
        });
    }

    fn git_log_rail(&mut self, ui: &mut egui::Ui) {
        ui.add_space(5.);
        if components::database_icon_button(
            ui,
            if self.git_ui.branches {
                Icon::Previous
            } else {
                Icon::Next
            },
            "Show or hide Git branches",
            true,
        )
        .clicked()
        {
            self.git_ui.branches = !self.git_ui.branches;
        }
        ui.add_space(7.);
        if components::database_icon_button(ui, Icon::Add, "New Branch…", true).clicked() {
            self.git_action(GitActionKind::NewBranch, self.git_ui.log_branch.clone());
        }
        let selected = self
            .service
            .git
            .branches
            .iter()
            .find(|b| branch_is_selected(&self.git_ui.log_branch, b))
            .cloned();
        let can_delete = selected.as_ref().is_some_and(|b| !b.remote && !b.current);
        if components::database_icon_button(ui, Icon::Remove, "Delete selected branch…", can_delete)
            .clicked()
            && let Some(selected) = selected
        {
            self.git_action(GitActionKind::DeleteBranch, selected.name);
        }
        if components::database_icon_button(ui, Icon::Import, "Fetch…", true).clicked() {
            self.git_action(GitActionKind::Fetch, String::new());
        }
        ui.add_space(5.);
        if components::database_icon_button(ui, Icon::ExpandAll, "Expand all Git branches", true)
            .clicked()
        {
            self.git_ui
                .groups
                .retain(|key, _| !key.starts_with("log.tree."));
            self.git_ui.refs_cache = None;
        }
        if components::database_icon_button(
            ui,
            Icon::CollapseAll,
            "Collapse all Git branches",
            true,
        )
        .clicked()
        {
            self.git_ui.refs_cache = None;
            for group in ["Local", "Remote", "Tags"] {
                self.git_ui
                    .groups
                    .insert(format!("log.tree.{group}"), false);
            }
        }
    }

    fn git_log_branches(&mut self, ui: &mut egui::Ui, width: f32, height: f32) {
        ui.allocate_ui_with_layout(
            egui::vec2(width, self.tokens.git_log_filter_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(3.);
                ui.add_sized(
                    [(width - 7.).max(0.), 30.],
                    egui::TextEdit::singleline(&mut self.git_ui.branch_filter)
                        .id(ui.make_persistent_id("git.log.branch.search"))
                        .hint_text("Branch or tag")
                        .margin(egui::vec2(9., 5.)),
                )
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Search branches or tags",
                    )
                });
            },
        );
        let rows = self.git_ref_rows();
        egui::ScrollArea::both()
            .id_salt("git.log.branches")
            .auto_shrink([false, false])
            .max_height((height - self.tokens.git_log_filter_height).max(20.))
            .codisseu()
            .show_rows(
                ui,
                self.tokens.git_tree_row_height,
                rows.rows.len(),
                |ui, range| {
                    ui.set_min_width(width);
                    for row in &rows.rows[range] {
                        match row {
                            RefRow::Head => {
                                let head = log_tree_row(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.labels,
                                    "HEAD (Current Branch)",
                                    "HEAD (Current Branch)",
                                    0,
                                    None,
                                    Some(Icon::Branch),
                                    self.git_ui.log_branch == "HEAD",
                                    width,
                                );
                                if head.clicked()
                                    && (self.git_ui.log_branch != "HEAD"
                                        || (self.service.git.history.is_none()
                                            && !self.git_history_loading()))
                                {
                                    self.git_ui.log_branch = "HEAD".into();
                                    self.git_ui.page = 0;
                                    self.git_request_history(true);
                                }
                            }
                            RefRow::Group {
                                path,
                                name,
                                depth,
                                open,
                            } => {
                                let response = log_tree_row(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.labels,
                                    name,
                                    path,
                                    *depth,
                                    Some(*open),
                                    if *depth == 0 {
                                        None
                                    } else {
                                        Some(Icon::Folder)
                                    },
                                    false,
                                    width,
                                );
                                if response.clicked() {
                                    self.git_ui.groups.insert(format!("log.tree.{path}"), !open);
                                    self.git_ui.refs_cache = None;
                                }
                            }
                            RefRow::Leaf(leaf, depth) => {
                                self.git_log_ref_leaf(ui, leaf, *depth, width)
                            }
                        }
                    }
                },
            );
    }

    fn git_log_ref_leaf(&mut self, ui: &mut egui::Ui, leaf: &RefLeaf, depth: usize, width: f32) {
        let name = leaf.name().rsplit('/').next().unwrap_or(leaf.name());
        let accessible = match leaf {
            RefLeaf::Branch(branch) => format!(
                "{} branch {}",
                if branch.remote { "Remote" } else { "Local" },
                branch.name
            ),
            RefLeaf::Tag(tag) => format!("Tag {}", tag.name),
        };
        let response = log_tree_row(
            ui,
            &self.tokens,
            &mut self.git_ui.labels,
            name,
            &accessible,
            depth + 1,
            None,
            Some(
                if matches!(leaf, RefLeaf::Branch(b) if self.service.git.branch_preferences.favorites.contains(&branch_reference(b)))
                {
                    Icon::Star
                } else if matches!(leaf, RefLeaf::Tag(_)) {
                    Icon::Tag
                } else {
                    Icon::Branch
                },
            ),
            matches!(leaf, RefLeaf::Branch(b) if branch_is_selected(&self.git_ui.log_branch, b)),
            width,
        );
        match leaf {
            RefLeaf::Branch(b) => {
                if b.current {
                    let center = egui::pos2(
                        response.rect.left() + 20. + (depth + 1) as f32 * 18.,
                        response.rect.center().y,
                    );
                    ui.painter()
                        .circle_filled(center, 2., color(self.tokens.git_added));
                }
                if response.clicked()
                    && (!branch_is_selected(&self.git_ui.log_branch, b)
                        || (self.service.git.history.is_none() && !self.git_history_loading()))
                {
                    self.git_ui.log_branch = branch_reference(b);
                    self.git_ui.page = 0;
                    self.git_request_history(true);
                }
                if response.double_clicked() && !b.current && !b.remote {
                    self.git_prepare(LocalAction::CheckoutBranch {
                        name: b.name.clone(),
                    });
                }
                response
                    .context_menu(|ui| self.git_branch_context(ui, &b.name, b.current, b.remote));
            }
            RefLeaf::Tag(tag) => {
                if response.clicked() {
                    self.git_ui.commit = Some(tag.target.clone());
                    self.git_send(GitCommand::CommitDetails {
                        commit: tag.target.clone(),
                    });
                }
                response.context_menu(|ui| {
                    if ui.button("Delete Tag…").clicked() {
                        self.git_action(GitActionKind::DeleteTag, tag.name.clone());
                        ui.close();
                    }
                });
            }
        }
    }

    fn git_log_filters(&mut self, ui: &mut egui::Ui, width: f32) {
        let height = self.tokens.git_log_filter_height;
        ui.allocate_ui_with_layout(
            egui::vec2(width, height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                ui.add_space(3.);
                let combo_width = (width * 0.19).clamp(70., 135.);
                let text_width = (width - combo_width - 4. * 26. - 31.)
                    .max(0.)
                    .min((width * 0.39).min(220.));
                ui.add_sized(
                    [text_width, 30.],
                    egui::TextEdit::singleline(&mut self.git_ui.filter)
                        .id(ui.make_persistent_id("git.log.text.search"))
                        .hint_text("Text or hash")
                        .margin(egui::vec2(8., 5.)),
                )
                .on_hover_text("Search text, revision or author in the current page")
                .widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Text/hash/author in this page",
                    )
                });
                let before = self.git_ui.log_branch.clone();
                ui.allocate_ui_with_layout(
                    egui::vec2(combo_width, 30.),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_width(combo_width);
                        egui::ComboBox::from_id_salt("git.log.branch.filter")
                            .width(combo_width)
                            .truncate()
                            .selected_text(if self.git_ui.log_branch.is_empty() {
                                "Branch"
                            } else {
                                display_reference(&self.git_ui.log_branch)
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.git_ui.log_branch,
                                    String::new(),
                                    "All branches",
                                );
                                for b in &self.service.git.branches {
                                    ui.selectable_value(
                                        &mut self.git_ui.log_branch,
                                        branch_reference(b),
                                        &b.name,
                                    );
                                }
                            })
                            .response
                            .on_hover_text(if self.git_ui.log_branch.is_empty() {
                                "All branches".to_owned()
                            } else {
                                display_reference(&self.git_ui.log_branch).to_owned()
                            })
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::ComboBox,
                                    true,
                                    "Filter history by branch",
                                )
                            });
                    },
                );
                if before != self.git_ui.log_branch {
                    self.git_ui.page = 0;
                    self.git_request_history(true);
                }
                if components::database_icon_button(
                    ui,
                    Icon::Previous,
                    "Previous log page",
                    self.git_ui.page > 0,
                )
                .clicked()
                {
                    self.git_ui.page -= 1;
                    self.git_request_history(false);
                }
                if components::database_icon_button(
                    ui,
                    Icon::Next,
                    "Next log page",
                    self.service
                        .git
                        .history
                        .as_ref()
                        .is_some_and(|h| h.has_more),
                )
                .clicked()
                {
                    self.git_ui.page += 1;
                    self.git_request_history(false);
                }
                if components::database_icon_button(
                    ui,
                    Icon::Refresh,
                    "Refresh Git Log",
                    !self.service.git.is_busy(),
                )
                .clicked()
                {
                    self.git_send_foreground(GitCommand::Refresh);
                    self.git_request_history(true);
                    self.git_send(GitCommand::Branches);
                    self.git_send(GitCommand::Tags);
                    self.git_send(GitCommand::BranchPreferences);
                }
                let more =
                    components::database_icon_button(ui, Icon::MoreDots, "Git Log actions", true);
                egui::Popup::menu(&more).show(|ui| {
                    for (kind, label) in [
                        (GitActionKind::Fetch, "Fetch…"),
                        (GitActionKind::Pull, "Pull…"),
                        (GitActionKind::Push, "Push…"),
                        (GitActionKind::Stash, "Stash Changes…"),
                        (GitActionKind::Stashes, "Unstash Changes…"),
                    ] {
                        if ui.button(label).clicked() {
                            self.git_action(kind, String::new());
                            ui.close();
                        }
                    }
                });
            },
        );
        ui.painter().hline(
            ui.min_rect().x_range(),
            ui.cursor().top(),
            egui::Stroke::new(1., color(self.tokens.border)),
        );
    }

    fn git_log_commits(&mut self, ui: &mut egui::Ui, width: f32, height: f32) {
        let rows = self.git_commit_rows();
        let row_height = self.tokens.git_log_row_height;
        let graph_width = ((rows.max_lane.min(8) + 1) as f32 * 12. + 8.)
            .max(64.)
            .min(width * 0.25);
        let viewport = egui::ScrollArea::vertical()
            .id_salt("git.log.rows")
            .auto_shrink([false, false])
            .max_height(height)
            .codisseu()
            .show_rows(ui, row_height, rows.indices.len(), |ui, range| {
                for n in range {
                    // Copy only a visible row so menu actions may mutate application state.
                    let Some(c) = self
                        .service
                        .git
                        .history
                        .as_ref()
                        .and_then(|h| h.commits.get(rows.indices[n]))
                        .cloned()
                    else {
                        continue;
                    };
                    let (_, rect) = ui.allocate_space(egui::vec2(width, row_height));
                    let response = ui.interact(
                        rect,
                        ui.make_persistent_id(("git.log.commit", &c.oid)),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::accesskit::Role::Button,
                            true,
                            self.git_ui.commit.as_ref() == Some(&c.oid),
                            format!("Commit {} {}", &c.oid[..c.oid.len().min(8)], c.subject),
                        )
                    });
                    ui.ctx().accesskit_node_builder(response.id, |node| {
                        node.set_selected(self.git_ui.commit.as_ref() == Some(&c.oid))
                    });
                    let background =
                        if self.git_ui.commit.as_ref() == Some(&c.oid) {
                            color(self.tokens.git_log_selection)
                        } else if c.refs.iter().any(|reference| {
                            reference == "HEAD" || reference.starts_with("HEAD -> ")
                        }) {
                            color(self.tokens.git_log_head)
                        } else if response.hovered() {
                            color(self.tokens.border)
                        } else {
                            color(self.tokens.panel)
                        };
                    ui.painter().rect_filled(rect, 0., background);
                    paint_log_graph(ui, &self.tokens, rect, &c, graph_width);
                    let text_left = rect.left() + graph_width;
                    let date_width = (width * 0.17).clamp(65., 100.);
                    let author_width = (width * 0.19).clamp(75., 130.);
                    let date_x = rect.right() - date_width;
                    let author_x = date_x - author_width;
                    let mut reference_right = author_x - 8.;
                    for reference in c.refs.iter().take(3).rev() {
                        let name = reference
                            .strip_prefix("HEAD -> ")
                            .unwrap_or(reference)
                            .strip_prefix("tag: ")
                            .unwrap_or(reference.strip_prefix("HEAD -> ").unwrap_or(reference));
                        let galley = ui.fonts_mut(|f| {
                            f.layout_no_wrap(
                                name.to_owned(),
                                FontId::proportional(13.),
                                color(self.tokens.muted),
                            )
                        });
                        let badge_width = galley.size().x + 18.;
                        if reference_right - badge_width < text_left + 75. {
                            break;
                        }
                        let badge = egui::Rect::from_min_size(
                            egui::pos2(reference_right - badge_width, rect.top() + 3.),
                            egui::vec2(badge_width, row_height - 6.),
                        );
                        components::paint_icon(
                            ui,
                            egui::Rect::from_center_size(
                                egui::pos2(badge.left() + 7., badge.center().y),
                                egui::Vec2::splat(12.),
                            ),
                            if reference.starts_with("tag: ") {
                                Icon::Tag
                            } else {
                                Icon::Branch
                            },
                            color(self.tokens.git_added),
                        );
                        log_text(
                            ui,
                            &mut self.git_ui.labels,
                            egui::Rect::from_min_max(badge.min + egui::vec2(16., 0.), badge.max),
                            name,
                            color(self.tokens.muted),
                            background,
                        );
                        reference_right = badge.left() - 6.;
                    }
                    log_text(
                        ui,
                        &mut self.git_ui.labels,
                        egui::Rect::from_min_max(
                            egui::pos2(text_left, rect.top()),
                            egui::pos2(reference_right, rect.bottom()),
                        ),
                        &c.subject,
                        color(self.tokens.text),
                        background,
                    );
                    log_text(
                        ui,
                        &mut self.git_ui.labels,
                        egui::Rect::from_min_max(
                            egui::pos2(author_x, rect.top()),
                            egui::pos2(date_x - 8., rect.bottom()),
                        ),
                        &c.author,
                        color(self.tokens.text),
                        background,
                    );
                    log_text(
                        ui,
                        &mut self.git_ui.labels,
                        egui::Rect::from_min_max(
                            egui::pos2(date_x, rect.top()),
                            rect.max - egui::vec2(8., 0.),
                        ),
                        &rows.dates[n],
                        color(self.tokens.text),
                        background,
                    );
                    if response.clicked() {
                        self.git_ui.commit = Some(c.oid.clone());
                        self.git_send(GitCommand::CommitDetails {
                            commit: c.oid.clone(),
                        });
                    }
                    if response.double_clicked() {
                        self.git_open_revision_diff(
                            c.parents.first().cloned(),
                            c.oid.clone(),
                            None,
                        );
                    }
                    response.context_menu(|ui| {
                        if ui.button("Copy Revision Number").clicked() {
                            ui.ctx().copy_text(c.oid.clone());
                            ui.close();
                        }
                        if ui.button("Show Diff").clicked() {
                            self.git_open_revision_diff(
                                c.parents.first().cloned(),
                                c.oid.clone(),
                                None,
                            );
                            ui.close();
                        }
                        if ui
                            .button("Interactively Rebase Commits after This…")
                            .clicked()
                        {
                            self.git_open_interactive_rebase(c.oid.clone());
                            ui.close();
                        }
                        ui.separator();
                        for (kind, label) in [
                            (GitActionKind::CherryPick, "Cherry-Pick…"),
                            (GitActionKind::Revert, "Revert Commit…"),
                            (GitActionKind::Reset, "Reset Current Branch to Here…"),
                            (GitActionKind::NewBranch, "New Branch…"),
                            (GitActionKind::NewTag, "New Tag…"),
                        ] {
                            if ui.button(label).clicked() {
                                self.git_action(kind, c.oid.clone());
                                ui.close();
                            }
                        }
                    });
                }
            });
        if rows.indices.is_empty() {
            let rect = egui::Rect::from_min_size(
                viewport.inner_rect.min + egui::vec2(8., 4.),
                egui::vec2((viewport.inner_rect.width() - 16.).max(0.), 24.),
            );
            ui.interact(
                rect,
                egui::Id::unique("git.log.empty.status"),
                egui::Sense::hover(),
            )
            .widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Label,
                    true,
                    if self.git_history_loading() {
                        "Loading Git history…"
                    } else {
                        "No commits matching the current filter"
                    },
                )
            });
            log_text(
                ui,
                &mut self.git_ui.labels,
                rect,
                "No commits matching the current filter",
                color(self.tokens.muted),
                color(self.tokens.panel),
            );
        }
    }

    fn git_log_details(&mut self, ui: &mut egui::Ui, height: f32) {
        let filter_height = self.tokens.git_log_filter_height;
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), filter_height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(5.);
                let details = self.service.git.details.clone();
                if components::database_icon_button(
                    ui,
                    Icon::Compare,
                    "Show selected commit diff",
                    details.is_some(),
                )
                .clicked()
                    && let Some(details) = details
                {
                    self.git_open_revision_diff(
                        details.parents.first().cloned(),
                        details.commit,
                        None,
                    );
                }
                if components::database_icon_button(
                    ui,
                    Icon::Copy,
                    "Copy selected revision",
                    self.git_ui.commit.is_some(),
                )
                .clicked()
                    && let Some(oid) = &self.git_ui.commit
                {
                    ui.ctx().copy_text(oid.clone());
                }
            },
        );
        if let Some(details) = self.service.git.details.clone() {
            let files_height = ((height - filter_height) * 0.58).max(35.);
            egui::ScrollArea::both()
                .id_salt("git.log.files")
                .max_height(files_height)
                .auto_shrink([false, false])
                .codisseu()
                .show(ui, |ui| {
                    let count = format!(
                        "{} {}",
                        details.changes.len(),
                        if details.changes.len() == 1 {
                            "file"
                        } else {
                            "files"
                        }
                    );
                    let common_parent = details
                        .changes
                        .first()
                        .and_then(|change| change.path.parent())
                        .filter(|parent| !parent.as_os_str().is_empty())
                        .filter(|parent| {
                            details
                                .changes
                                .iter()
                                .all(|change| change.path.parent() == Some(*parent))
                        });
                    let header = common_parent
                        .map(|parent| format!("{}  {count}", parent.display()))
                        .unwrap_or(count);
                    log_tree_row(
                        ui,
                        &self.tokens,
                        &mut self.git_ui.labels,
                        &header,
                        &header,
                        0,
                        Some(true),
                        Some(Icon::Folder),
                        false,
                        ui.available_width(),
                    );
                    for change in &details.changes {
                        let name = change
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let response = log_tree_row(
                            ui,
                            &self.tokens,
                            &mut self.git_ui.labels,
                            &name,
                            &change.path.to_string_lossy(),
                            1,
                            None,
                            Some(Icon::File),
                            self.git_ui.file.as_ref() == Some(&change.path),
                            ui.available_width(),
                        );
                        let tint = if change.status == 'A' {
                            color(self.tokens.git_added)
                        } else if change.status == 'D' {
                            color(self.tokens.git_deleted)
                        } else {
                            color(self.tokens.git_modified)
                        };
                        // File identity/extension controls the badge. Status colors only
                        // the label, as in the reference's changed-files list.
                        let label = egui::Rect::from_min_max(
                            response.rect.min + egui::vec2(64., 0.),
                            response.rect.max,
                        );
                        ui.painter().rect_filled(
                            label,
                            0.,
                            if self.git_ui.file.as_ref() == Some(&change.path) {
                                color(self.tokens.git_log_selection)
                            } else if response.hovered() {
                                color(self.tokens.border)
                            } else {
                                color(self.tokens.panel)
                            },
                        );
                        log_text(
                            ui,
                            &mut self.git_ui.labels,
                            label,
                            &name,
                            tint,
                            if self.git_ui.file.as_ref() == Some(&change.path) {
                                color(self.tokens.git_log_selection)
                            } else if response.hovered() {
                                color(self.tokens.border)
                            } else {
                                color(self.tokens.panel)
                            },
                        );
                        if response.clicked() {
                            self.git_ui.file = Some(change.path.clone());
                        }
                        if response.double_clicked() {
                            self.git_open_revision_diff(
                                details.parents.first().cloned(),
                                details.commit.clone(),
                                Some(change.path.clone()),
                            );
                        }
                        response.context_menu(|ui| {
                            if ui.button("Show Diff").clicked() {
                                self.git_open_revision_diff(
                                    details.parents.first().cloned(),
                                    details.commit.clone(),
                                    Some(change.path.clone()),
                                );
                                ui.close();
                            }
                            if ui.button("Show File at Revision").clicked() {
                                self.git_ui.file_version_open = true;
                                self.git_ui.file = Some(change.path.clone());
                                self.git_ui.commit = Some(details.commit.clone());
                                self.git_send(GitCommand::FileAt {
                                    commit: details.commit.clone(),
                                    path: change.path.clone(),
                                });
                                self.git_ui.diff_open = true;
                                ui.close();
                            }
                        });
                    }
                });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("git.log.commit.details")
                .max_height((height - files_height - filter_height - 5.).max(25.))
                .codisseu()
                .show(ui, |ui| {
                    egui::Frame::NONE
                        .inner_margin(self.tokens.content_margin())
                        .show(ui, |ui| {
                            log_wrapped(
                                ui,
                                &self.tokens,
                                &mut self.git_ui.labels,
                                &details.message,
                                color(self.tokens.text),
                            );
                            ui.add_space(12.);
                            if let Some(commit) =
                                self.service.git.history.as_ref().and_then(|h| {
                                    h.commits.iter().find(|c| c.oid == details.commit)
                                })
                            {
                                log_wrapped(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.labels,
                                    &format!(
                                        "{} {}",
                                        &commit.oid[..commit.oid.len().min(8)],
                                        commit.author
                                    ),
                                    color(self.tokens.text),
                                );
                                log_wrapped(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.labels,
                                    &format!("<{}>", commit.email),
                                    color(self.tokens.git_modified),
                                );
                                log_wrapped(
                                    ui,
                                    &self.tokens,
                                    &mut self.git_ui.labels,
                                    &git_date(commit.timestamp),
                                    color(self.tokens.text),
                                );
                            }
                            for parent in &details.parents {
                                if ui
                                    .link(format!("Parent {}", &parent[..parent.len().min(8)]))
                                    .clicked()
                                {
                                    self.git_ui.commit = Some(parent.clone());
                                    self.git_send(GitCommand::CommitDetails {
                                        commit: parent.clone(),
                                    });
                                }
                            }
                        });
                });
        } else {
            ui.add_space(8.);
            ui.horizontal(|ui| {
                ui.add_space(8.);
                ui.add(
                    egui::Label::new(
                        RichText::new("Select a commit to view its details")
                            .color(color(self.tokens.muted)),
                    )
                    .truncate(),
                );
            });
        }
    }
    pub(super) fn git_branch_context(
        &mut self,
        ui: &mut egui::Ui,
        name: &str,
        current: bool,
        remote: bool,
    ) {
        let reference = format!("refs/{}/{}", if remote { "remotes" } else { "heads" }, name);
        let favorite = self
            .service
            .git
            .branch_preferences
            .favorites
            .contains(&reference);
        if ui
            .button(if favorite {
                "Remove from Favorites"
            } else {
                "Add to Favorites"
            })
            .clicked()
        {
            self.git_send(GitCommand::SetBranchFavorite {
                reference,
                favorite: !favorite,
            });
            ui.close();
        }
        ui.separator();
        if !current
            && ui
                .button(if remote {
                    "Checkout Revision (detached)…"
                } else {
                    "Checkout"
                })
                .clicked()
        {
            self.git_prepare(if remote {
                LocalAction::CheckoutRevision {
                    revision: format!("refs/remotes/{name}"),
                }
            } else {
                LocalAction::CheckoutBranch { name: name.into() }
            });
            ui.close();
        }
        if ui.button("New Branch from Here…").clicked() {
            self.git_action(GitActionKind::NewBranch, name.into());
            ui.close();
        }
        if !current {
            for (kind, label) in [
                (GitActionKind::Merge, "Merge into Current…"),
                (GitActionKind::Rebase, "Rebase Current onto…"),
            ] {
                if ui.button(label).clicked() {
                    self.git_action(kind, name.into());
                    ui.close();
                }
            }
        }
        if !remote {
            if ui.button("Rename…").clicked() {
                self.git_action(GitActionKind::RenameBranch, name.into());
                ui.close();
            }
            if !current && ui.button("Delete…").clicked() {
                self.git_action(GitActionKind::DeleteBranch, name.into());
                ui.close();
            }
        }
    }
}
// Stable civil date conversion, UTC; avoids a locale process on the GUI thread.
pub(super) fn git_date(seconds: i64) -> String {
    let z = seconds.div_euclid(86400) + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn branch_reference(branch: &BranchInfo) -> String {
    format!(
        "refs/{}/{}",
        if branch.remote { "remotes" } else { "heads" },
        branch.name
    )
}
fn branch_is_selected(reference: &str, branch: &BranchInfo) -> bool {
    reference == branch.name
        || reference.strip_prefix(if branch.remote {
            "refs/remotes/"
        } else {
            "refs/heads/"
        }) == Some(branch.name.as_str())
}
fn display_reference(reference: &str) -> &str {
    reference
        .strip_prefix("refs/heads/")
        .or_else(|| reference.strip_prefix("refs/remotes/"))
        .or_else(|| reference.strip_prefix("refs/tags/"))
        .unwrap_or(reference)
}

fn git_log_date(seconds: i64) -> String {
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_secs()).ok());
        if let Some(now) = now
            && let Some(date) = local_date(seconds)
            && let Some(today) = local_date(now)
        {
            if (date.tm_year, date.tm_yday) == (today.tm_year, today.tm_yday) {
                return format!("Today {:02}:{:02}", date.tm_hour, date.tm_min);
            }
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            if let Some(month) = usize::try_from(date.tm_mon)
                .ok()
                .and_then(|month| MONTHS.get(month))
            {
                return format!("{} {month} {}", date.tm_mday, date.tm_year + 1900);
            }
        }
    }
    git_date(seconds)
}
#[cfg(unix)]
fn local_date(seconds: i64) -> Option<nix::libc::tm> {
    // time_t is narrower than i64 on some supported Unix targets.
    #[allow(clippy::useless_conversion)]
    let epoch = nix::libc::time_t::try_from(seconds).ok()?;
    let mut result = std::mem::MaybeUninit::<nix::libc::tm>::uninit();
    // localtime_r writes one tm into this exclusive buffer on success; it never
    // exposes the C library's shared calendar buffer to concurrent GUI workers.
    let pointer = unsafe { nix::libc::localtime_r(&epoch, result.as_mut_ptr()) };
    if pointer.is_null() {
        None
    } else {
        Some(unsafe { result.assume_init() })
    }
}

fn log_divider(ui: &mut egui::Ui, height: f32, tint: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1., height), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0., tint);
}
fn log_text(
    ui: &mut egui::Ui,
    labels: &mut super::tab_label::TabLabels,
    rect: egui::Rect,
    text: &str,
    tint: Color32,
    background: Color32,
) {
    if rect.width() > 0. {
        super::git_paint::text(ui, labels, rect, text, tint, background);
    }
}
fn log_wrapped(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    labels: &mut super::tab_label::TabLabels,
    text: &str,
    tint: Color32,
) {
    let width = (ui.available_width() - 20.).max(10.);
    let galley =
        ui.fonts_mut(|fonts| fonts.layout(text.to_owned(), FontId::proportional(13.), tint, width));
    for row in &galley.rows {
        let text = row.text();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.), egui::Sense::hover());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Label, true, &text));
        let rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(10., 0.),
            rect.max - egui::vec2(10., 0.),
        );
        log_text(ui, labels, rect, &text, tint, color(tokens.panel));
    }
}
#[allow(clippy::too_many_arguments)]
fn log_tree_row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    labels: &mut super::tab_label::TabLabels,
    label: &str,
    accessible: &str,
    depth: usize,
    expanded: Option<bool>,
    icon: Option<Icon>,
    selected: bool,
    width: f32,
) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(width, tokens.git_tree_row_height));
    let response = ui.interact(
        rect,
        ui.make_persistent_id(("git.log.ref", accessible)),
        egui::Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::accesskit::Role::TreeItem, true, selected, accessible)
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_selected(selected);
        if let Some(open) = expanded {
            node.set_expanded(open);
        }
    });
    if selected || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            0.,
            if selected {
                color(tokens.git_log_selection)
            } else {
                color(tokens.border)
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(1.),
            0.,
            egui::Stroke::new(1., color(tokens.selection)),
            egui::StrokeKind::Inside,
        );
    }
    let x = rect.left() + 12. + depth as f32 * 18.;
    let y = rect.center().y;
    if let Some(open) = expanded {
        let points = if open {
            vec![
                egui::pos2(x, y - 2.),
                egui::pos2(x + 4., y + 2.),
                egui::pos2(x + 8., y - 2.),
            ]
        } else {
            vec![
                egui::pos2(x + 2., y - 4.),
                egui::pos2(x + 6., y),
                egui::pos2(x + 2., y + 4.),
            ]
        };
        ui.painter().add(egui::Shape::line(
            points,
            egui::Stroke::new(1., color(tokens.muted)),
        ));
    }
    let icon_rect = egui::Rect::from_center_size(egui::pos2(x + 23., y), egui::Vec2::splat(16.));
    if let Some(icon) = icon {
        if matches!(icon, Icon::File) {
            components::paint_document_icon(ui, icon_rect, label, color(tokens.git_modified));
        } else {
            components::paint_icon(ui, icon_rect, icon, color(tokens.text));
        }
    }
    let inset = if icon.is_some() { x + 34. } else { x + 14. };
    log_text(
        ui,
        labels,
        egui::Rect::from_min_max(egui::pos2(inset, rect.top()), rect.max),
        label,
        color(tokens.text),
        if selected {
            color(tokens.git_log_selection)
        } else if response.hovered() || response.has_focus() {
            color(tokens.border)
        } else {
            color(tokens.panel)
        },
    );
    response.on_hover_text(accessible)
}
fn paint_log_graph(ui: &egui::Ui, tokens: &Tokens, rect: egui::Rect, c: &CommitEntry, width: f32) {
    if tokens.git_graph.is_empty() {
        return;
    }
    let painter = ui
        .painter()
        .with_clip_rect(rect.intersect(egui::Rect::from_min_size(
            rect.min,
            egui::vec2(width, rect.height()),
        )));
    let x = |lane: usize| rect.left() + 12. + lane.min(8) as f32 * 12.;
    let y = rect.center().y;
    for &(from, to) in &c.edges {
        let tint = color(tokens.git_graph[from % tokens.git_graph.len()]);
        let start = if from == c.lane {
            egui::pos2(x(from), y)
        } else {
            egui::pos2(x(from), rect.top())
        };
        let end = egui::pos2(x(to), rect.bottom());
        if from == to {
            painter.line_segment([start, end], egui::Stroke::new(1.5, tint));
        } else {
            painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                [start, egui::pos2(start.x, y), egui::pos2(end.x, y), end],
                false,
                Color32::TRANSPARENT,
                egui::Stroke::new(1.5, tint),
            ));
        }
    }
    let tint = color(tokens.git_graph[c.lane % tokens.git_graph.len()]);
    painter.line_segment(
        [egui::pos2(x(c.lane), rect.top()), egui::pos2(x(c.lane), y)],
        egui::Stroke::new(1.5, tint),
    );
    if c.refs
        .iter()
        .any(|reference| reference == "HEAD" || reference.starts_with("HEAD -> "))
    {
        painter.circle_stroke(egui::pos2(x(c.lane), y), 4., egui::Stroke::new(1.8, tint));
    } else {
        painter.circle_filled(egui::pos2(x(c.lane), y), 3.5, tint);
    }
}

#[cfg(test)]
#[path = "git_log_tests.rs"]
mod tests;
