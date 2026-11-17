//! Repository-bound literal commit search and comparison, independent of Git mutations.
use super::{
    Tokens, color,
    components::{self, Icon},
    git_insights_details::{Action, DetailsUi},
};
use crate::scrollbars::ScrollAreaExt as _;
use codisseu_core::git::search::{Request, SearchPage, Worker};
use std::{
    path::{Component, Path, PathBuf},
    time::Duration,
};

const PAGE_SIZE: usize = 50;

#[derive(Default)]
pub(super) struct SearchUi {
    open: bool,
    workspace: PathBuf,
    root: PathBuf,
    query: String,
    author: String,
    path: String,
    reference: String,
    base: String,
    head: String,
    compare_path: String,
    compare_open: bool,
    worker: Option<Worker>,
    pending: Option<u64>,
    page: Option<SearchPage>,
    changed: bool,
    error: Option<String>,
}
impl SearchUi {
    pub(super) fn open(&mut self, workspace: PathBuf, root: PathBuf, path: Option<PathBuf>) {
        self.cancel();
        self.workspace = workspace;
        self.root = root;
        self.query.clear();
        self.author.clear();
        self.reference.clear();
        self.base = "HEAD~1".into();
        self.head = "HEAD".into();
        self.path = path
            .and_then(|path| {
                if path.is_absolute() {
                    path.strip_prefix(&self.root).ok().map(Path::to_path_buf)
                } else {
                    Some(path)
                }
            })
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.compare_path = self.path.clone();
        self.compare_open = false;
        self.open = true;
        self.search(0);
    }
    pub(super) fn cancel(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
        self.pending = None;
        self.page = None;
        self.error = None;
        self.open = false;
    }
    pub(super) fn is_busy(&self) -> bool {
        self.worker.as_ref().is_some_and(Worker::is_busy)
    }
    pub(super) fn foreground_busy(&self, root: &Path) -> bool {
        self.root == root && self.open && self.is_busy()
    }
    fn scope(&self) -> Result<(), String> {
        if !self.root.is_absolute()
            || !self.workspace.is_absolute()
            || !(self.root.starts_with(&self.workspace) || self.workspace.starts_with(&self.root))
        {
            return Err("O repositório precisa pertencer ao projeto aberto.".into());
        }
        Ok(())
    }
    fn request(&self, skip: usize, stable: bool) -> Result<Request, String> {
        self.scope()?;
        let reference = if skip > 0 || stable {
            self.page
                .as_ref()
                .and_then(|page| page.resolved_reference.clone())
        } else {
            None
        }
        .or_else(|| nonempty(&self.reference));
        Ok(Request {
            workspace: self.workspace.clone(),
            root: self.root.clone(),
            query: self.query.clone(),
            author: self.author.clone(),
            path: relative(&self.path)?,
            reference,
            skip,
            limit: PAGE_SIZE,
        })
    }
    fn search(&mut self, skip: usize) {
        self.search_page(skip, false);
    }
    fn search_page(&mut self, skip: usize, stable: bool) {
        let request = match self.request(skip, stable) {
            Ok(request) => request,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let worker = self.worker.get_or_insert_with(Worker::new);
        match worker.request(request) {
            Ok(id) => {
                self.pending = Some(id);
                self.changed = false;
                self.error = None;
            }
            Err(error) => {
                self.error = Some(error.to_string());
                self.pending = None;
            }
        }
    }
    fn comparison(&self) -> Result<Action, String> {
        self.scope()?;
        if self.head.trim().is_empty() {
            return Err("Informe a revisão de destino da comparação.".into());
        }
        if [self.base.as_str(), self.head.as_str()]
            .iter()
            .any(|value| value.contains('\0') || value.len() > 1024)
        {
            return Err("Revisões devem ter até 1024 bytes e não podem conter NUL.".into());
        }
        Ok(Action::OpenDiff {
            root: self.root.clone(),
            base: nonempty(&self.base),
            head: self.head.trim().into(),
            path: relative(&self.compare_path)?,
        })
    }
    fn poll(&mut self) {
        if let Some(worker) = &self.worker {
            while let Some(reply) = worker.poll() {
                if Some(reply.id) != self.pending
                    || reply.request.workspace != self.workspace
                    || reply.request.root != self.root
                {
                    continue;
                }
                self.pending = None;
                match reply.result {
                    Ok(page) => {
                        self.root = page.root.clone();
                        self.page = Some(page);
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
        }
    }
    pub(super) fn show(
        &mut self,
        ctx: &egui::Context,
        tokens: &Tokens,
        details: &mut DetailsUi,
    ) -> Vec<Action> {
        self.poll();
        if self.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(25));
        }
        let mut actions = Vec::new();
        if !self.open {
            return actions;
        }
        let mut open = true;
        let mut search = None;
        let mut changed = false;
        let mut inspect = None;
        egui::Window::new("Buscar commits e comparar revisões").id(egui::Id::unique("git.insights.search"))
            .open(&mut open).default_width(740.).default_height((ctx.content_rect().height() - 100.).clamp(180., 620.))
            .max_width((ctx.content_rect().width() - 40.).max(240.)).max_height((ctx.content_rect().height() - 40.).max(180.))
            .constrain_to(ctx.content_rect().shrink(8.)).show(ctx, |ui| {
                tokens.form_style(ui);
                ui.set_max_width((ctx.content_rect().width() - 64.).max(240.));
                egui::ScrollArea::vertical().id_salt("git.insights.search.outer")
                    .max_height((ctx.content_rect().height() - 140.).max(180.)).codisseu().show(ui, |ui| {
                let field_width = (ui.available_width() - 112.).clamp(100., 520.);
                ui.label(egui::RichText::new(self.root.display().to_string()).monospace().color(color(tokens.muted)));
                let mut enter = false;
                egui::Grid::new("git.insights.search.filters").num_columns(2).spacing([12., 7.]).show(ui, |ui| {
                    for (label, value, hint) in [
                        ("Mensagem", &mut self.query, "Texto literal na mensagem do commit"),
                        ("Autor", &mut self.author, "Nome ou e-mail literal"),
                        ("Revisão", &mut self.reference, "HEAD, branch, tag ou SHA · vazio = HEAD"),
                        ("Caminho", &mut self.path, "Arquivo ou pasta relativo ao repositório · opcional"),
                    ] {
                        ui.label(label);
                        let field = ui.add(egui::TextEdit::singleline(value).char_limit(4096).hint_text(hint).desired_width(field_width));
                        field.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, format!("Buscar commits · {label}")));
                        changed |= field.changed();
                        enter |= field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        ui.end_row();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Buscar commits").clicked() || enter { search = Some((0, false)); }
                    if components::database_icon_button(ui, Icon::Refresh, "Atualizar busca de commits", !self.is_busy()).clicked() { search = Some((0, false)); }
                    if self.is_busy() && ui.small_button("Cancelar busca").clicked() {
                        if let Some(worker) = &self.worker { worker.cancel(); }
                        self.pending = None;
                    }
                });
                ui.label(egui::RichText::new("Busca literal · maiúsculas/minúsculas ASCII equivalentes · mensagem e autor combinados · 50 commits por página").small().color(color(tokens.muted)));
                self.changed |= changed;
                if changed {
                    if let Some(worker) = &self.worker { worker.cancel(); }
                    self.pending = None;
                }
                if self.changed { ui.label(egui::RichText::new("Filtros alterados · clique em Buscar commits para atualizar os resultados.").color(color(tokens.muted))); }
                if let Some(error) = &self.error { crate::status_messages::status_message(ui, error); }
                ui.separator();
                let usable = self.pending.is_none() && !self.changed;
                if let Some(page) = &self.page {
                    ui.add_enabled_ui(usable, |ui| {
                        ui.horizontal(|ui| {
                            if ui.add_enabled(page.skip > 0, egui::Button::new("Anterior")).clicked() { search = Some((page.skip.saturating_sub(PAGE_SIZE), true)); }
                            ui.label(format!("Página {} · {} commits", page.skip / PAGE_SIZE + 1, page.commits.len()));
                            if ui.add_enabled(page.has_more, egui::Button::new("Próxima")).clicked() { search = Some((page.skip + PAGE_SIZE, true)); }
                        });
                        egui::ScrollArea::vertical().id_salt("git.insights.search.results").max_height(300.).codisseu().show(ui, |ui| {
                            for commit in &page.commits {
                                ui.push_id(&commit.oid, |ui| {
                                    ui.group(|ui| {
                                        ui.horizontal(|ui| {
                                            if components::database_icon_button(ui, Icon::GitInspect, "Inspecionar commit encontrado", true).clicked() { inspect = Some(commit.oid.clone()); }
                                            if ui.selectable_label(false, short(&commit.subject, 400)).clicked() { inspect = Some(commit.oid.clone()); }
                                            if components::database_icon_button(ui, Icon::GitCopy, "Copiar SHA do commit encontrado", true).clicked() { ctx.copy_text(commit.oid.clone()); }
                                        });
                                        ui.label(egui::RichText::new(format!("{} · {} · {}", short(&commit.author, 100), super::git_log::git_date(commit.timestamp), short(&commit.oid, 8))).small().color(color(tokens.muted)));
                                    });
                                });
                            }
                            if page.commits.is_empty() { ui.label("Nenhum commit corresponde aos filtros nesta revisão."); }
                        });
                    });
                    ui.label(egui::RichText::new("Paginação preserva a revisão consultada. Atualizar acompanha novas alterações da branch.").small().color(color(tokens.muted)));
                } else if self.pending.is_some() { ui.label("Consultando histórico Git…"); }
                ui.separator();
                let comparison = egui::CollapsingHeader::new("Comparar duas revisões").open(Some(self.compare_open)).show(ui, |ui| {
                    egui::Grid::new("git.insights.compare.fields").num_columns(2).spacing([12., 7.]).show(ui, |ui| {
                        for (label, value, hint) in [
                            ("Base", &mut self.base, "HEAD~1 · vazio = árvore vazia"),
                            ("Destino", &mut self.head, "HEAD, branch, tag ou SHA"),
                            ("Caminho", &mut self.compare_path, "Relativo ao repositório · vazio = todos os arquivos"),
                        ] {
                            ui.label(label);
                            ui.add(egui::TextEdit::singleline(value).char_limit(4096).hint_text(hint).desired_width(field_width)).widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::TextInput, true, format!("Comparar revisões · {label}")));
                            ui.end_row();
                        }
                    });
                    ui.label(egui::RichText::new("Base vazia compara o destino com uma árvore vazia. As revisões são validadas pelo Git ao abrir o diff.").small().color(color(tokens.muted)));
                    if ui.button("Comparar revisões").clicked() {
                        match self.comparison() { Ok(action) => { self.error = None; actions.push(action); }, Err(error) => self.error = Some(error) }
                    }
                });
                if comparison.header_response.clicked() { self.compare_open = !self.compare_open; }
                });
            });
        if !open {
            self.cancel();
        }
        if let Some((skip, stable)) = search {
            self.search_page(skip, stable);
        }
        if let Some(oid) = inspect {
            // Global search opens the complete commit, without inventing a file context.
            details.open_commit(self.root.clone(), oid, PathBuf::new(), 1);
            ctx.request_repaint();
        }
        actions
    }
}
impl Drop for SearchUi {
    fn drop(&mut self) {
        if let Some(worker) = &self.worker {
            worker.cancel();
        }
    }
}
fn nonempty(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.trim().to_owned())
}
fn relative(value: &str) -> Result<Option<PathBuf>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    let path = PathBuf::from(value);
    if value.contains('\0')
        || value.len() > 4096
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(
            "Use um caminho relativo ao repositório, sem '..' e com até 4096 bytes.".into(),
        );
    }
    Ok(Some(path))
}
fn short(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::time::Instant;
    fn scroll_to_comparison(harness: &mut Harness<'_, (SearchUi, DetailsUi, Tokens, Vec<Action>)>) {
        let window = harness
            .get_by_label("Buscar commits e comparar revisões")
            .rect();
        harness.hover_at(window.right_bottom() - egui::vec2(20., 30.));
        harness.event(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -2000.),
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        let button = harness.get_by_label("Comparar revisões").rect();
        assert!(
            window.contains_rect(button),
            "window={window:?}, button={button:?}"
        );
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    #[test]
    fn fifty_results_and_expanded_comparison_remain_reachable_at_750_by_600() {
        let mut search = SearchUi::default();
        search.open = true;
        search.workspace = "/workspace".into();
        search.root = "/workspace/repo".into();
        search.base = "HEAD~1".into();
        search.head = "HEAD".into();
        search.compare_open = true;
        search.page = Some(SearchPage {
            root: search.root.clone(),
            skip: 0,
            has_more: true,
            resolved_reference: Some("a".repeat(40)),
            commits: (0..50)
                .map(|index| codisseu_core::git::CommitEntry {
                    oid: format!("{index:040x}"),
                    parents: vec![],
                    author: "Fixture".into(),
                    email: "fixture@example.invalid".into(),
                    timestamp: 0,
                    subject: format!("Commit de teste {index}"),
                    refs: vec![],
                    lane: 0,
                    edges: vec![],
                })
                .collect(),
        });
        let mut harness = Harness::builder()
            .with_max_steps(12)
            .with_size([750., 600.])
            .build_ui_state(
                |ui, state: &mut (SearchUi, DetailsUi, Tokens, Vec<Action>)| {
                    state.2.apply(ui.ctx());
                    state
                        .3
                        .extend(state.0.show(ui.ctx(), &state.2, &mut state.1));
                },
                (search, DetailsUi::default(), Tokens::load(), Vec::new()),
            );
        harness.run();
        let field = harness.get_by_label("Buscar commits · Mensagem").rect();
        assert!(field.right() <= 742., "{field:?}");
        let window = harness
            .get_by_label("Buscar commits e comparar revisões")
            .rect();
        assert!(
            window.bottom() <= 600. && window.right() <= 750.,
            "{window:?}"
        );
        harness.hover_at(window.right_bottom() - egui::vec2(20., 30.));
        harness.event(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -2000.),
            modifiers: egui::Modifiers::NONE,
        });
        harness.run();
        let button = harness.get_by_label("Comparar revisões").rect();
        assert!(button.top() >= 0. && button.bottom() <= 600., "{button:?}");
        harness.get_by_label("Comparar revisões").click();
        harness.run();
        assert!(
            harness
                .state()
                .3
                .iter()
                .any(|action| matches!(action, Action::OpenDiff { .. }))
        );
    }
    #[test]
    fn literal_search_and_comparison_are_repository_bound_and_driven_by_widgets() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        git(&root, &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Search Fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            git(&root, &["config", key, value]);
        }
        for (text, message) in [
            ("first\n", "Fix literal A.* one"),
            ("second\n", "Other message"),
            ("third\n", "Fix literal A.* two"),
        ] {
            std::fs::write(root.join("a.php"), text).unwrap();
            git(&root, &["add", "a.php"]);
            git(&root, &["commit", "-m", message]);
        }
        let head = git(&root, &["rev-parse", "HEAD"]);
        let mut search = SearchUi::default();
        search.open(root.clone(), root.clone(), None);
        search.query = "A.*".into();
        search.author = "Search Fixture".into();
        search.search(0);
        let started = Instant::now();
        while search.pending.is_some() || search.is_busy() {
            search.poll();
            std::thread::sleep(Duration::from_millis(5));
            assert!(started.elapsed() < Duration::from_secs(10));
        }
        assert!(search.error.is_none(), "{:?}", search.error);
        assert_eq!(search.page.as_ref().unwrap().commits.len(), 2);
        let mut harness = Harness::builder()
            .with_max_steps(12)
            .with_size([1150., 950.])
            .build_ui_state(
                |ui, state: &mut (SearchUi, DetailsUi, Tokens, Vec<Action>)| {
                    state.2.apply(ui.ctx());
                    state
                        .3
                        .extend(state.0.show(ui.ctx(), &state.2, &mut state.1));
                },
                (search, DetailsUi::default(), Tokens::load(), Vec::new()),
            );
        harness.run();
        assert!(harness.query_by_label("Other message").is_none());
        harness.get_by_label("Fix literal A.* two").click();
        harness.run();
        assert!(harness.state().1.is_busy());
        harness.get_by_label("Comparar duas revisões").click();
        harness.run();
        scroll_to_comparison(&mut harness);
        harness.get_by_label("Comparar revisões").click();
        harness.run();
        assert!(harness.state().3.iter().any(|action| matches!(action, Action::OpenDiff { root: bound, base: Some(base), head: target, path: None } if *bound == root && base == "HEAD~1" && target == "HEAD")));
        harness.state_mut().0.base.clear();
        scroll_to_comparison(&mut harness);
        harness.get_by_label("Comparar revisões").click();
        harness.run();
        assert!(
            harness
                .state()
                .3
                .iter()
                .any(|action| matches!(action, Action::OpenDiff { base: None, .. }))
        );
        harness.state_mut().0.compare_path = "../escape.php".into();
        let before = harness.state().3.len();
        scroll_to_comparison(&mut harness);
        harness.get_by_label("Comparar revisões").click();
        harness.run();
        assert_eq!(harness.state().3.len(), before);
        assert!(
            harness
                .state()
                .0
                .error
                .as_ref()
                .unwrap()
                .contains("relativo")
        );
        harness.state_mut().0.cancel();
        assert!(!harness.state().0.open);
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), head);
        assert!(git(&root, &["status", "--porcelain"]).is_empty());
    }
    #[test]
    fn comparison_rejects_empty_target_or_unbound_root_and_pagination_keeps_resolved_tip() {
        let mut ui = SearchUi::default();
        ui.workspace = "/workspace".into();
        ui.root = "/workspace/repo".into();
        ui.head = "HEAD".into();
        assert!(matches!(
            ui.comparison(),
            Ok(Action::OpenDiff { base: None, .. })
        ));
        ui.head.clear();
        assert!(ui.comparison().is_err());
        ui.head = "HEAD".into();
        ui.root = "/outside".into();
        assert!(ui.comparison().is_err());
        ui.root = "/workspace/repo".into();
        ui.page = Some(SearchPage {
            root: ui.root.clone(),
            commits: Vec::new(),
            has_more: true,
            skip: 50,
            resolved_reference: Some("a".repeat(40)),
        });
        ui.reference = "main".into();
        assert_eq!(
            ui.request(100, true).unwrap().reference,
            Some("a".repeat(40))
        );
        assert_eq!(ui.request(0, true).unwrap().reference, Some("a".repeat(40)));
        assert_eq!(ui.request(0, false).unwrap().reference, Some("main".into()));
        assert!(relative("../escape").is_err());
        assert!(relative("/absolute").is_err());
        assert!(relative("src/file á.php").unwrap().is_some());
        assert_eq!(short("á🌧xyz", 2), "á🌧…");
    }
}
