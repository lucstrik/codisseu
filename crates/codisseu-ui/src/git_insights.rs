//! Native line authorship. Reads are bounded and independent of SCM mutations.
use super::git_insights_details::{Action, DetailsUi};
use super::*;
use codisseu_core::git::{
    GitCommand,
    autoria::{self, Line, Snapshot},
};
use std::{path::Path, sync::Arc, time::Instant};

#[derive(Clone, PartialEq, Eq)]
struct Identity {
    document: DocumentId,
    workspace: PathBuf,
    path: PathBuf,
    version: u64,
}
struct Accepted {
    identity: Identity,
    snapshot: Snapshot,
}
struct CurrentLine {
    identity: Identity,
    line: Line,
}
pub(super) struct InsightsUi {
    worker: autoria::Worker,
    identity: Option<Identity>,
    workspace: Option<PathBuf>,
    contents: Option<Arc<str>>,
    accepted: Option<Accepted>,
    current: Option<CurrentLine>,
    requested: Option<(u64, usize, usize)>,
    changed: Instant,
    refreshed: Instant,
    pub error: Option<String>,
    pub details: DetailsUi,
    search: super::git_insights_search::SearchUi,
    pending: Option<(PathBuf, Action)>,
    history_candidate: Option<(DocumentId, u64, usize, usize)>,
    history_requested: Option<(DocumentId, u64, usize, usize)>,
    history_changed: Instant,
}
impl Default for InsightsUi {
    fn default() -> Self {
        Self {
            worker: autoria::Worker::new(),
            identity: None,
            workspace: None,
            contents: None,
            accepted: None,
            current: None,
            requested: None,
            changed: Instant::now(),
            refreshed: Instant::now(),
            error: None,
            details: DetailsUi::default(),
            search: super::git_insights_search::SearchUi::default(),
            pending: None,
            history_candidate: None,
            history_requested: None,
            history_changed: Instant::now(),
        }
    }
}
impl InsightsUi {
    pub(super) fn foreground_busy(&self, root: &Path) -> bool {
        self.details.foreground_busy(root) || self.search.foreground_busy(root)
    }
    pub(super) fn is_busy(&self) -> bool {
        self.worker.is_busy() || self.details.is_busy() || self.search.is_busy()
    }
    pub fn line(&self, path: &Path, version: u64, line: usize) -> Option<&Line> {
        self.accepted
            .as_ref()
            .filter(|a| a.identity.path == path && a.identity.version == version)
            .and_then(|a| a.snapshot.lines.iter().find(|l| l.line == line))
            .or_else(|| {
                self.current
                    .as_ref()
                    .filter(|c| {
                        c.identity.path == path
                            && c.identity.version == version
                            && c.line.line == line
                    })
                    .map(|c| &c.line)
            })
    }

    fn root(&self) -> Option<PathBuf> {
        self.accepted
            .as_ref()
            .map(|a| a.snapshot.repository.root.clone())
    }
    pub fn gutter_width(&self, path: &Path, _version: u64, enabled: bool) -> f32 {
        if enabled
            && self
                .accepted
                .as_ref()
                .is_some_and(|a| a.identity.path == path)
        {
            460.
        } else {
            0.
        }
    }
}
impl CodisseuApp {
    pub(super) fn git_insights_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            self.git_insights.worker.cancel();
            self.git_insights.details.cancel();
            self.git_insights.search.cancel();
            return;
        }
        let workspace_identity = self.service.workspace_path().map(PathBuf::from);
        if self.git_insights.workspace != workspace_identity {
            self.git_insights.workspace = workspace_identity;
            self.git_insights.details.cancel();
            self.git_insights.search.cancel();
            self.git_insights.pending = None;
            self.git_insights.history_candidate = None;
            self.git_insights.history_requested = None;
        }
        let enabled = self.service.editor_preferences();
        let identity = self.service.active_document().and_then(|d| {
            self.service.workspace_root_for(&d.path).map(|w| Identity {
                document: d.id,
                workspace: w.to_path_buf(),
                path: d.path.clone(),
                version: d.version,
            })
        });
        if identity != self.git_insights.identity {
            self.git_insights.worker.cancel();
            self.git_insights.identity = identity.clone();
            let same_document = self.git_insights.accepted.as_ref().is_some_and(|accepted| {
                identity.as_ref().is_some_and(|i| {
                    i.document == accepted.identity.document
                        && i.workspace == accepted.identity.workspace
                        && i.path == accepted.identity.path
                })
            });
            if !same_document {
                self.git_insights.accepted = None;
                self.git_insights.current = None;
            }
            self.git_insights.contents = None;
            self.git_insights.requested = None;
            self.git_insights.changed = Instant::now();
            self.git_insights.error = None;
        }
        while let Some(reply) = self.git_insights.worker.poll() {
            if let Some(identity) = identity.as_ref()
                && identity.path == reply.path
                && identity.version == reply.document_version
                && self.git_insights.requested.is_some_and(|r| r.0 == reply.id)
            {
                match reply.result {
                    Ok(snapshot) => {
                        if self.git_insights.accepted.as_ref().is_some_and(|old| {
                            old.snapshot.head != snapshot.head
                                || old.snapshot.repository != snapshot.repository
                        }) {
                            self.git_insights.current = None;
                        }
                        let caret = self.service.active_document().and_then(|doc| {
                            self.service
                                .active_view()
                                .and_then(|v| self.service.view(v))
                                .map(|v| {
                                    doc.snapshot().char_to_line(
                                        v.selection().head.min(doc.snapshot().len_chars()),
                                    ) + 1
                                })
                        });
                        if let Some(line) =
                            snapshot.lines.iter().find(|line| Some(line.line) == caret)
                        {
                            self.git_insights.current = Some(CurrentLine {
                                identity: identity.clone(),
                                line: metadata(line),
                            });
                        }
                        self.git_insights.accepted = Some(Accepted {
                            identity: identity.clone(),
                            snapshot,
                        });
                        self.git_insights.error = None;
                    }
                    Err(e) => {
                        self.git_insights.accepted = None;
                        self.git_insights.current = None;
                        self.git_insights.error = Some(e.to_string());
                    }
                }
            }
        }
        self.git_insights_follow_history(ctx);
        if !(enabled.git_current_line || enabled.git_file_blame) {
            self.git_insights.worker.cancel();
            return;
        }
        let Some(identity) = identity else {
            return;
        };
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let snapshot = doc.snapshot();
        let caret = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
            .map_or(0, |v| {
                snapshot.char_to_line(v.selection().head.min(snapshot.len_chars()))
            });
        if let Some(accepted) = self
            .git_insights
            .accepted
            .as_ref()
            .filter(|a| a.identity == identity)
            && let Some(line) = accepted.snapshot.lines.iter().find(|l| l.line == caret + 1)
            && !self
                .git_insights
                .current
                .as_ref()
                .is_some_and(|c| c.identity == identity && c.line.line == caret + 1)
        {
            self.git_insights.current = Some(CurrentLine {
                identity: identity.clone(),
                line: metadata(line),
            });
        }
        let metrics = self.editor_metrics();
        let need_current = enabled.git_current_line
            && !self
                .git_insights
                .current
                .as_ref()
                .is_some_and(|c| c.identity == identity && c.line.line == caret + 1);
        let use_viewport = enabled.git_file_blame
            && metrics.document == Some(doc.id)
            && !(need_current && (caret < metrics.visible_start || caret >= metrics.visible_end));
        let first = if use_viewport {
            metrics.visible_start
        } else {
            caret
        };
        let last = if use_viewport {
            metrics.visible_end.max(first + 1)
        } else {
            caret + 1
        };
        // Cover nearby cursor moves without starting a process per keystroke.
        let start = first.saturating_sub(40) + 1;
        let lines = (last.saturating_sub(start - 1) + 40)
            .min(500)
            .min(snapshot.len_lines().saturating_sub(start - 1))
            .max(1);
        let covered = self
            .git_insights
            .requested
            .is_some_and(|(_, s, n)| first + 1 >= s && last < s + n);
        let expired = self.git_insights.refreshed.elapsed() >= Duration::from_secs(3);
        if self.git_insights.changed.elapsed() < Duration::from_millis(250) {
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if !covered || (expired && !self.git_insights.worker.is_busy()) {
            // text() materializes only once per document version, never per frame.
            if self.git_insights.contents.is_none() {
                if snapshot.len_bytes() > autoria::MAX_CONTENTS {
                    self.git_insights.error =
                        Some("Autoria indisponível: buffer excede4MiB".into());
                    return;
                }
                self.git_insights.contents = Some(Arc::from(doc.text()));
            }
            let request = autoria::Request {
                workspace: identity.workspace,
                path: identity.path,
                document_version: identity.version,
                contents: self.git_insights.contents.as_ref().unwrap().clone(),
                start,
                lines,
            };
            match self.git_insights.worker.request(request) {
                Ok(id) => self.git_insights.requested = Some((id, start, lines)),
                Err(e) => {
                    self.git_insights.error = Some(e.to_string());
                    self.git_insights.requested = Some((0, start, lines));
                }
            }
            self.git_insights.refreshed = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_millis(
            if self.git_insights.worker.is_busy() {
                30
            } else {
                500
            },
        ));
    }
    pub(super) fn git_insights_menu(&mut self, ui: &mut egui::Ui) {
        let prefs = self.service.editor_preferences();
        let mut inline = prefs.git_current_line;
        let mut hover = prefs.git_hover;
        let mut gutter = prefs.git_file_blame;
        let a = ui.checkbox(&mut inline, "Autoria da linha atual").changed();
        let b = ui
            .checkbox(&mut gutter, "Anotações de autoria do arquivo")
            .changed();
        let c = ui
            .checkbox(&mut hover, "Detalhes ao passar o mouse")
            .changed();
        if a || b || c {
            self.send(Command::SetGitInsights {
                inline,
                hover,
                gutter,
            });
        }
        ui.separator();
        if ui.button("Buscar commits e comparar revisões").clicked() {
            let root = self
                .git_insights
                .root()
                .or_else(|| self.service.git.repo.as_ref().map(|r| r.root.clone()));
            let workspace = root
                .as_ref()
                .and_then(|r| self.service.git_workspace_for(r))
                .map(PathBuf::from);
            if let (Some(workspace), Some(root)) = (workspace, root) {
                self.git_insights.search.open(workspace, root, None);
            } else {
                self.git_insights.error =
                    Some("Abra um projeto com repositório Git para buscar commits.".into());
            }
            ui.close();
        }
        if ui.button("Inspecionar commit da linha").clicked() {
            self.git_insights_open_current(false);
            ui.close();
        }
        if ui.button("Histórico da linha / seleção").clicked() {
            self.git_insights_open_history();
            ui.close();
        }
        if ui.button("Histórico do arquivo").clicked() {
            self.git_insights_open_current(true);
            ui.close();
        }
        if ui.button("Atualizar autoria").clicked() {
            self.git_insights.refreshed = Instant::now() - Duration::from_secs(4);
            ui.close();
        }
        if let Some(error) = &self.git_insights.error {
            ui.separator();
            crate::status_messages::status_message(ui, error);
        }
    }
    pub(super) fn git_insights_status(&mut self, ui: &mut egui::Ui) {
        if !self.service.editor_preferences().git_current_line {
            return;
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let line = self
            .git_insights
            .line(&doc.path, doc.version, self.cursor.0)
            .map(metadata);
        let Some(line) = line else {
            return;
        };
        let text = if line.uncommitted {
            "Alterações não confirmadas".into()
        } else {
            format!("{}, {}", line.author, relative_time(line.timestamp))
        };
        let response = ui.add_sized(
            [
                ui.available_width().clamp(0., 260.),
                ui.text_style_height(&egui::TextStyle::Body),
            ],
            egui::Label::new(RichText::new(text).color(color(self.tokens.muted)))
                .truncate()
                .sense(egui::Sense::click()),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                "Autoria Git da linha atual",
            )
        });
        if response.clicked() {
            self.git_insights_open_current(false);
        }
        response.on_hover_text(annotation(&line));
    }
    fn git_insights_open_current(&mut self, history: bool) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let snapshot = doc.snapshot();
        let caret = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
            .map_or(0, |v| {
                snapshot.char_to_line(v.selection().head.min(snapshot.len_chars()))
            });
        let line = self
            .git_insights
            .line(&doc.path, doc.version, caret + 1)
            .map(metadata);
        let Some(root) = self.git_insights.root() else {
            return;
        };
        if history {
            if let Ok(path) = doc.path.strip_prefix(&root) {
                self.git_insights_action(Action::OpenFileHistory {
                    root: root.clone(),
                    path: path.to_path_buf(),
                });
            }
        } else if let Some(line) = line.filter(|l| !l.uncommitted) {
            self.git_insights
                .details
                .open_commit(root, line.oid, line.path, line.original_line);
        }
    }
    fn git_insights_follow_history(&mut self, ctx: &egui::Context) {
        if !self.git_insights.details.follows_cursor() {
            return;
        }
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let Some(view) = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
        else {
            return;
        };
        let snapshot = doc.snapshot();
        let r = view.selection().range();
        let stamp = (
            doc.id,
            doc.version,
            snapshot.char_to_line(r.start.min(snapshot.len_chars())) + 1,
            snapshot.char_to_line(
                if r.end > r.start { r.end - 1 } else { r.end }.min(snapshot.len_chars()),
            ) + 1,
        );
        if self.git_insights.history_candidate != Some(stamp) {
            self.git_insights.history_candidate = Some(stamp);
            self.git_insights.history_changed = Instant::now();
        }
        if self.git_insights.history_requested != Some(stamp) {
            if self.git_insights.history_changed.elapsed() >= Duration::from_millis(300) {
                self.git_insights_request_history(false);
            } else {
                ctx.request_repaint_after(Duration::from_millis(60));
            }
        }
    }
    pub(super) fn git_insights_open_history(&mut self) {
        self.git_insights_request_history(true);
    }
    fn git_insights_request_history(&mut self, foreground: bool) {
        let Some(doc) = self.service.active_document() else {
            return;
        };
        let Some(workspace) = self
            .service
            .workspace_root_for(&doc.path)
            .map(PathBuf::from)
        else {
            return;
        };
        let snapshot = doc.snapshot();
        let Some(view) = self
            .service
            .active_view()
            .and_then(|v| self.service.view(v))
        else {
            return;
        };
        let r = view.selection().range();
        let start = snapshot.char_to_line(r.start.min(snapshot.len_chars())) + 1;
        let end = snapshot.char_to_line(
            if r.end > r.start { r.end - 1 } else { r.end }.min(snapshot.len_chars()),
        ) + 1;
        if snapshot.len_bytes() <= autoria::MAX_CONTENTS {
            let stamp = (doc.id, doc.version, start, end);
            self.git_insights.history_requested = Some(stamp);
            self.git_insights.history_candidate = Some(stamp);
            self.git_insights.history_changed = Instant::now();
            let cache_matches = self.git_insights.identity.as_ref().is_some_and(|i| {
                i.document == doc.id && i.version == doc.version && i.path == doc.path
            });
            let contents = self
                .git_insights
                .contents
                .as_ref()
                .filter(|_| cache_matches)
                .cloned()
                .unwrap_or_else(|| Arc::from(doc.text()));
            if cache_matches {
                self.git_insights.contents = Some(contents.clone());
            }
            self.git_insights.details.open_history(
                workspace,
                doc.path.clone(),
                doc.version,
                contents,
                start,
                end,
            );
            if !foreground {
                self.git_insights
                    .details
                    .silence_automatic_history_progress();
            }
        }
    }
    pub(super) fn git_insights_hover(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &Path,
        version: u64,
        line_number: usize,
    ) {
        let Some(line) = self
            .git_insights
            .line(path, version, line_number)
            .map(metadata)
        else {
            return;
        };
        let Some(root) = self.git_insights.root() else {
            return;
        };
        let hit = ui.interact(
            rect,
            egui::Id::unique(("git.insight", path, line_number, rect.left().to_bits())),
            egui::Sense::click(),
        );
        hit.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::accesskit::Role::Button,
                true,
                format!("Autoria da linha {line_number}: {}", annotation(&line)),
            )
        });
        if hit.clicked() && !line.uncommitted {
            self.git_insights.details.open_commit(
                root.clone(),
                line.oid.clone(),
                line.path.clone(),
                line.original_line,
            );
        }
        if !self.service.editor_preferences().git_hover {
            return;
        }
        if hit.hovered() && !line.uncommitted {
            self.git_insights.details.prefetch_commit(
                root.clone(),
                line.oid.clone(),
                line.path.clone(),
                line.original_line,
            );
        }
        let previous = self
            .git_insights
            .details
            .previous_diff(&root, &line.oid, &line.path);
        let mut action = None;
        let mut history = false;
        let mut inspect = false;
        hit.on_hover_ui(|ui| {
            ui.set_max_width(458.);
            ui.add_space(4.);
            if line.uncommitted {
                ui.strong("Alterações não confirmadas");
                ui.label("Esta linha contém alterações locais ainda sem commit.");
            } else {
                ui.horizontal(|ui| {
                    ui.strong(&line.author);
                    ui.label(
                        RichText::new(format!("<{}>", line.email))
                            .small()
                            .color(color(self.tokens.muted)),
                    );
                });
                ui.label(
                    RichText::new(format!(
                        "{} · {} {:02}:{:02} UTC",
                        relative_time(line.timestamp),
                        super::git_log::git_date(line.timestamp),
                        line.timestamp.rem_euclid(86400) / 3600,
                        line.timestamp.rem_euclid(3600) / 60
                    ))
                    .small()
                    .color(color(self.tokens.muted)),
                );
                ui.add_space(6.);
                ui.label(&line.summary);
                ui.add_space(4.);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .button(&line.oid[..line.oid.len().min(8)])
                        .on_hover_text("Copiar SHA")
                        .clicked()
                    {
                        ui.ctx().copy_text(line.oid.clone());
                    }
                    if icon_button(ui, Icon::GitInspect, "Inspecionar commit", false, 26.).clicked()
                    {
                        inspect = true;
                    }
                    if ui
                        .add_enabled_ui(previous.is_some(), |ui| {
                            icon_button(
                                ui,
                                Icon::GitCompare,
                                "Comparar com revisão anterior",
                                false,
                                26.,
                            )
                        })
                        .inner
                        .clicked()
                    {
                        action = previous.clone();
                    }
                    if icon_button(
                        ui,
                        Icon::GitReveal,
                        "Mostrar commit no histórico",
                        false,
                        26.,
                    )
                    .clicked()
                    {
                        action = Some(Action::RevealCommit {
                            root: root.clone(),
                            oid: line.oid.clone(),
                        });
                    }
                    if icon_button(ui, Icon::GitHistory, "Histórico da linha", false, 26.).clicked()
                    {
                        history = true;
                    }
                    if icon_button(ui, Icon::GitCopy, "Copiar SHA", false, 26.).clicked() {
                        ui.ctx().copy_text(line.oid.clone());
                    }
                    if icon_button(
                        ui,
                        Icon::GitVersions,
                        "Abrir arquivo nesta revisão",
                        false,
                        26.,
                    )
                    .clicked()
                    {
                        action = Some(Action::OpenHistoricalFile {
                            root: root.clone(),
                            oid: line.oid.clone(),
                            path: line.path.clone(),
                            line: line.original_line,
                        });
                    }
                });
                ui.add_space(4.);
                if let Some(patch) = self.git_insights.details.hover_patch(
                    &root,
                    &line.oid,
                    &line.path,
                    line.original_line,
                ) {
                    egui::Frame::new()
                        .fill(color(self.tokens.editor))
                        .inner_margin(8)
                        .show(ui, |ui| {
                            for text in patch.lines().take(8) {
                                let c = if text.starts_with('+') {
                                    self.tokens.git_added
                                } else if text.starts_with('-') {
                                    self.tokens.git_deleted
                                } else {
                                    self.tokens.muted
                                };
                                ui.label(
                                    RichText::new(text.chars().take(100).collect::<String>())
                                        .monospace()
                                        .small()
                                        .color(color(c)),
                                );
                            }
                        });
                }
            }
            ui.add_space(4.);
        });
        if inspect {
            self.git_insights
                .details
                .open_commit(root, line.oid, line.path, line.original_line);
        }
        if history {
            let current = self
                .service
                .active_document()
                .filter(|d| d.path == path && d.version == version)
                .map(|d| (d.id, d.snapshot()));
            if let Some((id, snapshot)) = current {
                let caret = self
                    .service
                    .active_view()
                    .and_then(|v| self.service.view(v))
                    .map(|v| v.selection().head);
                if line_number > 0 && line_number <= snapshot.len_lines() {
                    if caret.is_some_and(|head| {
                        snapshot.char_to_line(head.min(snapshot.len_chars())) + 1 != line_number
                    }) {
                        self.send(Command::SetSelection {
                            id,
                            expected_version: version,
                            selection: codisseu_core::Selection::caret(
                                snapshot.line_to_char(line_number - 1),
                            ),
                        });
                    }
                    self.git_insights_open_history();
                }
            }
        }
        if let Some(action) = action {
            self.git_insights_action(action);
        }
    }
    pub(super) fn git_insights_windows(&mut self, ctx: &egui::Context) {
        let mut actions = self.git_insights.details.show(ctx, &self.tokens);
        actions.extend(self.git_insights.search.show(
            ctx,
            &self.tokens,
            &mut self.git_insights.details,
        ));
        for action in actions {
            self.git_insights_action(action);
        }
        if let Some((root, _)) = &self.git_insights.pending
            && self
                .service
                .git
                .repo
                .as_ref()
                .is_some_and(|r| &r.root == root)
            && !self.service.git.is_busy()
        {
            let (_, action) = self.git_insights.pending.take().unwrap();
            self.git_insights_action(action);
        }
    }
    fn git_insights_action(&mut self, action: Action) {
        let root = match &action {
            Action::OpenDiff { root, .. }
            | Action::OpenFileHistory { root, .. }
            | Action::RevealCommit { root, .. }
            | Action::OpenHistoricalFile { root, .. } => root.clone(),
        };
        if !self
            .service
            .git
            .repo
            .as_ref()
            .is_some_and(|r| r.root == root)
        {
            match self
                .service
                .dispatch(Command::SelectGitRepository(root.clone()))
            {
                Ok(()) => self.git_insights.pending = Some((root, action)),
                Err(e) => self.git_insights.error = Some(e),
            }
            return;
        }
        match action {
            Action::OpenDiff {
                base, head, path, ..
            } => self.git_open_revision_diff(base, head, path),
            Action::OpenFileHistory { path, .. } => self.git_open_file_history(path),
            Action::RevealCommit { oid, .. } => {
                self.git_ui.log_branch = oid.clone();
                self.git_ui.filter.clear();
                self.git_open_log();
                self.git_ui.commit = Some(oid.clone());
                self.git_send(GitCommand::CommitDetails { commit: oid });
            }
            Action::OpenHistoricalFile {
                oid, path, line, ..
            } => {
                self.git_ui.notice =
                    format!("Revisão {} · linha {}", &oid[..oid.len().min(8)], line);
                self.git_ui.commit = Some(oid.clone());
                self.git_ui.diff_range = None;
                self.git_ui.selected_hunks.clear();
                self.git_ui.file_version_open = true;
                self.git_ui.diff_open = true;
                self.git_ui.file = Some(path.clone());
                self.git_send(GitCommand::FileAt { commit: oid, path });
            }
        }
    }
}
/// Presentation copies only metadata; a long source line never gets cloned per frame.
fn metadata(line: &Line) -> Line {
    Line {
        line: line.line,
        original_line: line.original_line,
        oid: line.oid.clone(),
        author: truncate(&line.author, 80),
        email: truncate(&line.email, 200),
        timestamp: line.timestamp,
        timezone: line.timezone.clone(),
        summary: truncate(&line.summary, 280),
        path: line.path.clone(),
        previous_oid: line.previous_oid.clone(),
        previous_path: line.previous_path.clone(),
        uncommitted: line.uncommitted,
        text: Vec::new(),
    }
}
pub(super) fn annotation(line: &Line) -> String {
    if line.uncommitted {
        "Você, agora • Alterações não confirmadas".into()
    } else {
        format!(
            "{}, {} • {}",
            truncate(&line.author, 80),
            relative_time(line.timestamp),
            truncate(&line.summary, 50)
        )
    }
}
pub(super) fn truncate(text: &str, n: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
pub(super) fn relative_time(timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let age = now.saturating_sub(timestamp);
    if age < 0 {
        return "no futuro".into();
    }
    let (n, label) = if age < 60 {
        return "agora".into();
    } else if age < 3600 {
        (age / 60, "min")
    } else if age < 86400 {
        (age / 3600, "h")
    } else if age < 2592000 {
        (age / 86400, "dias")
    } else if age < 31536000 {
        (age / 2592000, "meses")
    } else {
        (age / 31536000, "anos")
    };
    let label = if n == 1 {
        match label {
            "dias" => "dia",
            "meses" => "mês",
            "anos" => "ano",
            other => other,
        }
    } else {
        label
    };
    format!("há {n} {label}")
}

/// GitLens-style age rail: recent work warm, old work cool; local work neutral.
pub(super) fn heat(timestamp: i64, uncommitted: bool) -> Color32 {
    if uncommitted {
        return Color32::GRAY;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let age = now.saturating_sub(timestamp).max(0) as f32 / 86400.;
    let t = (age / 90.).clamp(0., 1.);
    Color32::from_rgb(
        (220. - 110. * t) as u8,
        (125. - 15. * t) as u8,
        (92. + 100. * t) as u8,
    )
}

#[cfg(test)]
#[path = "git_insights_tests.rs"]
mod tests;
