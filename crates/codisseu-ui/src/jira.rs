//! Native optional Jira tool window and central reader;
//! Code buffers stay intact.
use super::*;
use codisseu_core::jira::{
    self as jira, Connection, Filters, Picker, Secret, Status, View, WorklogDraft,
};
use std::time::Instant;
// Owned layout values, reviewed against the user screenshot; logical points.
const TASK_ROW_HEIGHT: f32 = 78.;
const SEARCH_LABEL_GAP: f32 = 6.;
const FILTER_HEIGHT: f32 = 28.;
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Description,
    People,
    Comments,
    Hours,
}
pub(super) struct PickerUi {
    kind: Picker,
    text: String,
    deadline: Option<Instant>,
}
pub(super) struct LogUi {
    draft: WorklogDraft,
    review: bool,
    sent: bool,
    choose_issue: bool,
}
impl LogUi {
    pub(super) fn for_calendar(started: String) -> Self {
        Self {
            draft: WorklogDraft {
                key: String::new(),
                minutes: 30,
                started,
                comment: String::new(),
            },
            review: false,
            sent: false,
            choose_issue: true,
        }
    }
}
#[derive(Default)]
pub(super) struct JiraUi {
    pub open: bool,
    pub tabs: crate::jira_tabs::Tabs,
    pub hours: crate::jira_timesheet::HoursUi,
    created_month_input: String,
    created_month_editing: bool,
    pub settings: bool,
    pub picker: Option<PickerUi>,
    pub worklog: Option<LogUi>,
    connection: Connection,
    token: Secret,
    remember: bool,
    expanded: bool,
    section: Section,
    markdown: markdown::RemoteMarkdown,
    images: crate::jira_images::Images,
    stamp: Option<(u64, Section)>,
    view_name: String,
    back: Vec<String>,
    pub(super) notice: Option<String>,
}
impl CodisseuApp {
    pub(super) fn jira_result(&mut self, r: Result<(), String>) {
        if let Err(e) = r {
            self.jira_ui.notice = Some(e)
        }
    }
    pub(super) fn toggle_jira(&mut self) {
        self.jira_ui.open = !self.jira_ui.open;
        if self.jira_ui.open {
            self.ai_chat.open = false;
            self.show_database = false
        }
    }
    pub(super) fn open_jira_settings(&mut self) {
        self.jira_ui.settings = true;
        self.jira_ui.connection = self.service.jira.profile.connection.clone();
        self.jira_ui.remember =
            self.service.jira.profile.auto_connect && !self.jira_ui.connection.site.is_empty();
        if self.jira_ui.connection.api == self.jira_ui.connection.site {
            self.jira_ui.connection.api.clear();
        }
        self.jira_ui.token = Secret::default();
        self.jira_ui.notice = None
    }
    pub(super) fn jira_tick(&mut self, ctx: &egui::Context) {
        if self.exit_when_saved {
            return;
        }
        self.jira_tabs_tick();
        self.jira_hours_tick();
        self.jira_ui.images.tick(ctx, &mut self.service.jira.media);
        if !self.service.jira.connected {
            self.jira_ui.images.clear();
            self.jira_ui.markdown.clear();
            self.jira_ui.stamp = None;
            self.jira_ui.tabs.close(crate::jira_tabs::Kind::Issue);
        }
        if !self.service.jira.profile.enabled {
            self.jira_ui.open = false;
            self.jira_ui.tabs = crate::jira_tabs::Tabs::default();
            self.jira_ui.markdown.clear();
            self.jira_ui.back.clear();
            self.jira_ui.worklog = None;
            self.jira_ui.picker = None;
        }
        if let Some(p) = &mut self.jira_ui.picker
            && p.deadline
                .is_some_and(|t| t.elapsed() >= Duration::from_millis(350))
        {
            p.deadline = None;
            self.service.jira.suggest(p.kind, p.text.clone())
        }
        if self
            .jira_ui
            .picker
            .as_ref()
            .is_some_and(|p| p.deadline.is_some())
        {
            ctx.request_repaint_after(Duration::from_millis(50))
        }
        if self.jira_ui.worklog.as_ref().is_some_and(|w| w.sent) && !self.service.jira.main_busy() {
            if self.service.jira.error.is_none() {
                self.jira_ui.worklog = None
            } else if let Some(w) = &mut self.jira_ui.worklog {
                w.sent = false;
                w.review = false;
            }
        }
    }
    pub(super) fn jira_open_issue(&mut self, key: &str, back: bool) {
        if back
            && let Some(k) = &self.service.jira.selected
            && k != key
        {
            if self.jira_ui.back.len() >= 24 {
                self.jira_ui.back.remove(0);
            }
            self.jira_ui.back.push(k.clone())
        }
        let r = self.service.jira.select(key);
        self.jira_result(r);
        self.jira_open_tab(crate::jira_tabs::Kind::Issue);
        self.jira_ui.section = Section::Description;
        self.jira_ui.stamp = None;
    }
    fn jira_picker(&mut self, p: Picker) {
        self.jira_ui.picker = Some(PickerUi {
            kind: p,
            text: String::new(),
            deadline: None,
        });
        self.service.jira.suggest(p, String::new());
        self.jira_ui.notice = None
    }
    pub(super) fn jira_panel(&mut self, ui: &mut egui::Ui) {
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(self.tokens.panel_content_inset_x as f32);
            ui.strong("Jira");
            if self.service.jira.request_pending() {
                ui.add(egui::Spinner::new().size(14.));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Close, "Fechar painel Jira", false, 26.).clicked() {
                    self.jira_ui.open = false
                }
                if icon_button(ui, Icon::Settings, "Configurar Jira", false, 26.).clicked() {
                    self.open_jira_settings()
                }
                if icon_button(
                    ui,
                    Icon::Clock,
                    "Abrir Minhas horas",
                    self.jira_ui.tabs.is_active(crate::jira_tabs::Kind::Hours),
                    26.,
                )
                .clicked()
                {
                    self.open_jira_hours();
                }
                ui.add_enabled_ui(
                    self.service.jira.connected && !self.service.jira.request_pending(),
                    |ui| {
                        if icon_button(ui, Icon::Refresh, "Atualizar tarefas Jira", false, 26.)
                            .clicked()
                        {
                            self.service.jira.refresh()
                        }
                    },
                );
            });
        });
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(
                self.tokens.panel_content_inset_x,
                10,
            ))
            .show(ui, |ui| self.jira_panel_body(ui));
    }
    fn jira_panel_body(&mut self, ui: &mut egui::Ui) {
        if self.service.jira.restoring() {
            ui.add_space(20.);
            ui.strong("Reconectando ao Jira…");
            ui.add_space(8.);
            ui.label("Usando o token salvo no cofre do sistema.");
            jira_messages(
                ui,
                &self.service.jira.error,
                &self.service.jira.notice,
                &self.jira_ui.notice,
                &self.tokens,
            );
            return;
        }
        if !self.service.jira.connected {
            ui.add_space(20.);
            ui.label(
                RichText::new("Seu trabalho, perto do código.")
                    .size(19.)
                    .strong(),
            );
            ui.add_space(10.);
            ui.label(
                "Consulte tarefas, acompanhe histórias e registre horas sem sair do Codisseu.",
            );
            ui.add_space(14.);
            if ui
                .add(self.tokens.dialog_button("Conectar ao Jira…", true))
                .clicked()
            {
                self.open_jira_settings()
            }
            jira_messages(
                ui,
                &self.service.jira.error,
                &self.service.jira.notice,
                &self.jira_ui.notice,
                &self.tokens,
            );
            return;
        }
        if let Some(id) = &self.service.jira.identity {
            ui.label(
                RichText::new(&id.name)
                    .size(12.)
                    .color(color(self.tokens.muted)),
            );
        }
        ui.add_space(12.);
        let mut f = self.service.jira.profile.filters.clone();
        let before = f.clone();
        let search_label = ui.label(
            RichText::new("Buscar tarefas")
                .small()
                .color(color(self.tokens.muted)),
        );
        ui.add_space(SEARCH_LABEL_GAP);
        egui::Frame::new()
            .fill(color(self.tokens.editor))
            .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
            .corner_radius(self.tokens.control_corner_radius)
            .inner_margin(egui::Margin::symmetric(8, 2))
            .show(ui, |ui| {
                ui.spacing_mut().interact_size.y = 24.;
                ui.add(
                    egui::TextEdit::singleline(&mut f.text)
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text("Título, conteúdo ou chave…")
                        .desired_width(ui.available_width())
                        .char_limit(512),
                )
                .labelled_by(search_label.id);
            });
        ui.add_space(6.);
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), FILTER_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_height(FILTER_HEIGHT);
                ui.spacing_mut().item_spacing.x = 6.;
                ui.spacing_mut().interact_size.y = FILTER_HEIGHT;
                egui::ComboBox::from_id_salt("jira.status")
                    .width((ui.available_width() - 68.).max(80.))
                    .selected_text(f.status.label())
                    .show_ui(ui, |ui| {
                        for s in [Status::Open, Status::Progress, Status::Done, Status::All] {
                            ui.selectable_value(&mut f.status, s, s.label());
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::ComboBox,
                            true,
                            "Status Jira",
                        )
                    });
                if icon_button(ui, Icon::Filter, "Filtros Jira", self.jira_ui.expanded, 28.)
                    .clicked()
                {
                    self.jira_ui.expanded = !self.jira_ui.expanded
                }
                if icon_button(ui, Icon::Revert, "Redefinir filtros Jira", false, 28.).clicked() {
                    f = Filters::default();
                    self.jira_ui.view_name.clear()
                }
            },
        );
        ui.add_space(6.);
        if ui.checkbox(&mut f.mine, "Somente minhas tarefas").changed() && f.mine {
            f.assignee = None;
        }
        if let Some(month) = f.created_month.clone() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Criados em {month}")).small());
                if icon_button(ui, Icon::Close, "Limpar mês de criação", false, 24.).clicked() {
                    f.created_month = None;
                    self.jira_ui.created_month_editing = false;
                }
            });
        }
        if self.jira_ui.expanded {
            ui.add_space(10.);
            egui::ScrollArea::vertical()
                .id_salt("jira.filters")
                .max_height((ui.available_height() - 170.).clamp(100., 280.))
                .codisseu()
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8., 8.);
                    ui.spacing_mut().interact_size.y = FILTER_HEIGHT;
                    ui.spacing_mut().button_padding = egui::vec2(10., 5.);
                    let label = ui.label("Criados no mês · aaaa-mm");
                    ui.add_space(6.);
                    if !self.jira_ui.created_month_editing {
                        self.jira_ui.created_month_input =
                            f.created_month.clone().unwrap_or_default();
                    }
                    let input = ui
                        .add(
                            egui::TextEdit::singleline(&mut self.jira_ui.created_month_input)
                                .hint_text("2026-11")
                                .char_limit(7)
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id);
                    self.jira_ui.created_month_editing = input.has_focus();
                    if input.changed() {
                        let month = self.jira_ui.created_month_input.trim();
                        if month.is_empty() {
                            f.created_month = None;
                        } else if jira::period::month_bounds(month).is_ok() {
                            f.created_month = Some(month.into());
                        }
                    }
                    if ui.button("Criados para mim neste mês").clicked() {
                        f.mine = true;
                        f.assignee = None;
                        f.status = Status::All;
                        f.created_month = Some(jira::period::today().chars().take(7).collect());
                        self.jira_ui.created_month_editing = false;
                    }
                    ui.add_space(10.);
                    if let Some(identity) = &self.service.jira.identity
                        && identity.team_fields.len() > 1
                    {
                        let before = f.team_field.clone();
                        egui::ComboBox::from_id_salt("jira.team_field")
                            .selected_text(
                                f.team_field
                                    .as_ref()
                                    .map(|f| f.name.as_str())
                                    .unwrap_or("Campo de equipe: Team"),
                            )
                            .show_ui(ui, |ui| {
                                for field in &identity.team_fields {
                                    ui.selectable_value(
                                        &mut f.team_field,
                                        Some(field.clone()),
                                        &field.name,
                                    );
                                }
                            })
                            .response
                            .widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::accesskit::Role::ComboBox,
                                    true,
                                    "Campo de equipe Jira",
                                )
                            });
                        if f.team_field != before {
                            f.team = None;
                        }
                    }
                    for p in [
                        Picker::Project,
                        Picker::Kind,
                        Picker::Parent,
                        Picker::User,
                        Picker::Team,
                    ] {
                        let c = match p {
                            Picker::Project => &mut f.project,
                            Picker::Kind => &mut f.kind,
                            Picker::Parent => &mut f.parent,
                            Picker::User => &mut f.assignee,
                            Picker::Team => &mut f.team,
                        };
                        ui.horizontal(|ui| {
                            let enabled = p != Picker::Team
                                || self
                                    .service
                                    .jira
                                    .identity
                                    .as_ref()
                                    .is_some_and(|i| i.team_field.is_some());
                            ui.add_enabled_ui(enabled, |ui| {
                                let label = c
                                    .as_ref()
                                    .map(|c| format!("{}: {}", p.label(), c.name))
                                    .unwrap_or_else(|| format!("{}: todos", p.label()));
                                if ui
                                    .add_sized(
                                        [ui.available_width() - 32., FILTER_HEIGHT],
                                        egui::Button::new((label, egui::Atom::grow())).truncate(),
                                    )
                                    .clicked()
                                {
                                    self.jira_picker(p)
                                }
                                if c.is_some()
                                    && icon_button(
                                        ui,
                                        Icon::Close,
                                        &format!("Limpar {}", p.label()),
                                        false,
                                        24.,
                                    )
                                    .clicked()
                                {
                                    *c = None
                                }
                            });
                        });
                    }
                    if self
                        .service
                        .jira
                        .identity
                        .as_ref()
                        .is_none_or(|i| i.team_field.is_none())
                    {
                        ui.label(
                            RichText::new("Equipe: campo Team indisponível nesta conta.")
                                .small()
                                .color(color(self.tokens.muted)),
                        );
                    }
                    ui.add_space(4.);
                    ui.label(
                        RichText::new("Filtros salvos automaticamente")
                            .small()
                            .color(color(self.tokens.muted)),
                    );
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.jira_ui.view_name)
                                .hint_text("Nome da vista")
                                .char_limit(80)
                                .desired_width((ui.available_width() - 34.).max(80.)),
                        );
                        if ui
                            .add_enabled(
                                !self.jira_ui.view_name.trim().is_empty()
                                    && self.service.jira.profile.views.len() < 20,
                                egui::Button::new("+"),
                            )
                            .on_hover_text("Salvar vista com estes filtros")
                            .clicked()
                        {
                            let mut profile = self.service.jira.profile.clone();
                            let name = self.jira_ui.view_name.trim().to_string();
                            profile.views.retain(|v| v.name != name);
                            profile.views.push(View {
                                name,
                                filters: f.clone(),
                            });
                            let r = self.service.jira.set_profile(profile);
                            self.jira_result(r);
                            self.jira_ui.view_name.clear();
                        }
                    });
                });
        }
        if !self.service.jira.profile.views.is_empty() {
            ui.horizontal(|ui| {
                let views = self.service.jira.profile.views.clone();
                egui::ComboBox::from_id_salt("jira.saved")
                    .selected_text("Vistas salvas")
                    .width(ui.available_width() - 8.)
                    .show_ui(ui, |ui| {
                        for (i, v) in views.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.selectable_label(false, &v.name).clicked() {
                                    f = v.filters.clone();
                                    ui.close()
                                }
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Excluir vista")
                                    .clicked()
                                {
                                    let mut p = self.service.jira.profile.clone();
                                    p.views.remove(i);
                                    let r = self.service.jira.set_profile(p);
                                    self.jira_result(r);
                                    ui.close()
                                }
                            });
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::ComboBox,
                            true,
                            "Vistas salvas Jira",
                        )
                    });
            });
        }
        if f != before {
            let r = self.service.jira.set_filters(f);
            self.jira_result(r)
        }
        ui.add_space(5.);
        jira_messages(
            ui,
            &self.service.jira.error,
            &self.service.jira.notice,
            &self.jira_ui.notice,
            &self.tokens,
        );
        let count = self.service.jira.page.issues.len();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} tarefas nesta página", count))
                    .small()
                    .color(color(self.tokens.muted)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new("Atualizadas primeiro")
                        .small()
                        .color(color(self.tokens.muted)),
                );
            });
        });
        ui.separator();
        let height = (ui.available_height() - 42.).max(40.);
        if count == 0 {
            egui::ScrollArea::vertical()
                .max_height(height)
                .codisseu()
                .show(ui, |ui| {
                    ui.add_space(16.);
                    ui.strong(if self.service.jira.main_busy() {
                        "Buscando suas tarefas…"
                    } else {
                        "Nenhuma tarefa com estes filtros"
                    });
                    ui.add_space(6.);
                    ui.label("Altere o status ou redefina os filtros para ampliar a busca.");
                });
            ui.allocate_space(egui::vec2(0., (height - 95.).max(0.)));
        } else {
            let mut open = None;
            egui::ScrollArea::vertical()
                .id_salt("jira.issues")
                .max_height(height)
                .auto_shrink([false, false])
                .codisseu()
                .show_rows(ui, TASK_ROW_HEIGHT, count, |ui, range| {
                    for i in range {
                        let issue = &self.service.jira.page.issues[i];
                        let r = task_row(
                            ui,
                            issue,
                            self.service.jira.selected.as_deref() == Some(&issue.key),
                            &mut self.jira_ui.images,
                            &mut self.service.jira.media,
                            &self.tokens,
                        );
                        if r.clicked() {
                            open = Some(issue.key.clone())
                        }
                        r.on_hover_text(format!(
                            "{}\n{} · {}\n{}",
                            issue.title, issue.kind, issue.project, issue.assignee.name
                        ));
                    }
                });
            if let Some(k) = open {
                self.jira_open_issue(&k, false)
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.service.jira.main_busy() && self.service.jira.page_index > 0,
                    egui::Button::new("‹"),
                )
                .on_hover_text("Página anterior Jira")
                .clicked()
            {
                self.service.jira.previous_page()
            }
            ui.label(format!("Página {}", self.service.jira.page_index + 1));
            if ui
                .add_enabled(
                    !self.service.jira.main_busy()
                        && self.service.jira.page.next.is_some()
                        && self.service.jira.page_index < 63,
                    egui::Button::new("›"),
                )
                .on_hover_text("Próxima página Jira")
                .clicked()
            {
                self.service.jira.next_page()
            }
            ui.label(
                RichText::new("50 por página")
                    .small()
                    .color(color(self.tokens.muted)),
            );
        });
    }
    pub(super) fn jira_reader(&mut self, ui: &mut egui::Ui) {
        let Some(d) = self.service.jira.detail.as_ref() else {
            egui::Frame::new().inner_margin(24).show(ui, |ui| {
                ui.heading("Carregando tarefa…");
                if self.service.jira.request_pending() {
                    ui.spinner();
                }
                jira_messages(
                    ui,
                    &self.service.jira.error,
                    &self.service.jira.notice,
                    &self.jira_ui.notice,
                    &self.tokens,
                );
                if ui.button("Tentar carregar novamente").clicked()
                    && let Some(k) = self.service.jira.selected.clone()
                {
                    let r = self.service.jira.select(&k);
                    self.jira_result(r)
                }
            });
            return;
        };
        let issue = d.issue.clone();
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(24, 14))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if !self.jira_ui.back.is_empty()
                        && ui.small_button("‹ Voltar").clicked()
                        && let Some(k) = self.jira_ui.back.pop()
                    {
                        self.jira_open_issue(&k, false);
                        return;
                    }
                    ui.label(
                        RichText::new(format!(
                            "{}  /  {}  /  {}",
                            issue.project, issue.kind, issue.key
                        ))
                        .small()
                        .color(color(self.tokens.muted)),
                    );
                    ui.hyperlink_to(
                        "Abrir no Jira ↗",
                        format!(
                            "{}/browse/{}",
                            self.service.jira.profile.connection.site, issue.key
                        ),
                    );
                });
                ui.add_space(10.);
                ui.label(RichText::new(&issue.title).size(23.).strong());
                ui.add_space(10.);
                ui.horizontal_wrapped(|ui| {
                    egui::Frame::new()
                        .fill(color(self.tokens.rail_selection))
                        .corner_radius(4)
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.label(RichText::new(&issue.status).color(color(self.tokens.accent)));
                        });
                    self.jira_ui.images.avatar(
                        ui,
                        &mut self.service.jira.media,
                        &issue.assignee_avatar,
                        &issue.assignee.name,
                        24.,
                        &self.tokens,
                    );
                    ui.label(format!(
                        "Responsável: {}",
                        if issue.assignee.name.is_empty() {
                            "Não atribuído"
                        } else {
                            &issue.assignee.name
                        }
                    ));
                    if !issue.priority.is_empty() {
                        ui.label(format!("Prioridade: {}", issue.priority));
                    }
                });
                ui.add_space(12.);
                ui.horizontal_wrapped(|ui| {
                    for (s, label) in [
                        (Section::Description, "Descrição"),
                        (Section::People, "Pessoas e vínculos"),
                        (Section::Comments, "Comentários"),
                        (Section::Hours, "Horas"),
                    ] {
                        if ui
                            .selectable_label(self.jira_ui.section == s, label)
                            .clicked()
                        {
                            self.jira_ui.section = s;
                            match s {
                                Section::Comments => self.service.jira.comments(0),
                                Section::Hours => self.service.jira.worklogs(0),
                                _ => (),
                            }
                        }
                    }
                    if self.service.jira.request_pending() {
                        ui.add(egui::Spinner::new().size(14.));
                    }
                });
                ui.separator();
                jira_messages(
                    ui,
                    &self.service.jira.error,
                    &self.service.jira.notice,
                    &self.jira_ui.notice,
                    &self.tokens,
                );
            });
        if self.jira_ui.section == Section::People {
            let Some(d) = self.service.jira.detail.as_ref() else {
                return;
            };
            let relations = d.issue.relations.clone();
            let mut target = None;
            egui::ScrollArea::vertical().id_salt("jira.people").codisseu().show(ui,|ui|{
egui::Frame::new().inner_margin(24).show(ui,|ui|{
                ui.heading("Pessoas envolvidas");
ui.add_space(12.);
ui.horizontal(|ui|{
self.jira_ui.images.avatar(ui,&mut self.service.jira.media,&issue.assignee_avatar,&issue.assignee.name,28.,&self.tokens);
ui.label(format!("Responsável   {}",issue.assignee.name));
}
);
ui.horizontal(|ui|{
self.jira_ui.images.avatar(ui,&mut self.service.jira.media,&issue.reporter_avatar,&issue.reporter.name,28.,&self.tokens);
ui.label(format!("Criado por      {}",issue.reporter.name));
}
);
                if !issue.team.is_empty(){
ui.label(format!("Equipe           {}",issue.team));
}
                ui.add_space(22.);
ui.heading("História, épico e relações");
ui.add_space(8.);
                if relations.is_empty(){
ui.label("Nenhum vínculo disponibilizado pelo Jira para este card.");
}
                for rel in relations{
ui.add_space(8.);
ui.label(RichText::new(&rel.label).small().color(color(self.tokens.muted)));
if ui.add(egui::Button::new(format!("{}  {}  →",rel.key,rel.title)).wrap()).clicked(){
target=Some(rel.key)
}
}
                ui.add_space(16.);
ui.label(RichText::new("Participantes adicionais aparecem como autores dos comentários e apontamentos.").small().color(color(self.tokens.muted)));

}
);
}
);
            if let Some(k) = target {
                self.jira_open_issue(&k, true)
            }
            return;
        }
        let Some(d) = self.service.jira.detail.as_ref() else {
            return;
        };
        if self.jira_ui.section == Section::Hours {
            let total = d.worklog_total;
            let offset = d.worklog_offset;
            let len = d.worklogs.len();
            let account = self
                .service
                .jira
                .identity
                .as_ref()
                .map(|i| i.account.as_str());
            let seconds: i64 = d
                .worklogs
                .iter()
                .filter(|w| Some(w.account.as_str()) == account)
                .map(|w| w.seconds)
                .sum();
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(24, 8))
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "Suas horas nesta página: {}",
                            jira::period::duration(seconds)
                        ));
                        if ui
                            .add_enabled(
                                !self.service.jira.main_busy(),
                                self.tokens.dialog_button("Registrar horas…", true),
                            )
                            .clicked()
                        {
                            self.jira_ui.worklog = Some(LogUi {
                                draft: WorklogDraft {
                                    key: issue.key.clone(),
                                    minutes: 30,
                                    started: jira::now(),
                                    comment: String::new(),
                                },
                                review: false,
                                sent: false,
                                choose_issue: false,
                            });
                            self.jira_ui.notice = None
                        }
                    });
                    jira_pager(ui, offset, len, total, |o| self.service.jira.worklogs(o));
                });
        } else if self.jira_ui.section == Section::Comments {
            let total = d.comment_total;
            let offset = d.comment_offset;
            let len = d.comments.len();
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(24, 8))
                .show(ui, |ui| {
                    jira_pager(ui, offset, len, total, |o| self.service.jira.comments(o));
                });
        }
        let stamp = (self.service.jira.revision, self.jira_ui.section);
        if self.jira_ui.stamp != Some(stamp) {
            self.jira_ui.stamp = Some(stamp);
            let d = self.service.jira.detail.as_ref().unwrap();
            let source = match self.jira_ui.section {
                Section::Description => {
                    let mut body = d.issue.description.clone();
                    if !d.issue.attachments.is_empty() {
                        body.push_str("\n\n## Anexos\n\n");
                    }
                    for a in &d.issue.attachments {
                        if a.mime.starts_with("image/") {
                            let url = if a.thumbnail.is_empty() {
                                &a.url
                            } else {
                                &a.thumbnail
                            };
                            body.push_str(&format!(
                                "![{}](<{}>)\n\n",
                                plain_markdown(&a.name),
                                url
                            ));
                        } else {
                            body.push_str(&format!(
                                "[{}](<{}>)\n\n",
                                plain_markdown(&a.name),
                                a.url
                            ));
                        }
                    }
                    body
                }
                Section::Comments => d
                    .comments
                    .iter()
                    .map(|c| {
                        format!(
                            "{}\n{}\n\n{}\n\n---\n",
                            author_header(&c.author, &c.avatar),
                            plain_markdown(&c.created),
                            c.body
                        )
                    })
                    .collect(),
                Section::Hours => d
                    .worklogs
                    .iter()
                    .filter(|w| {
                        self.service
                            .jira
                            .identity
                            .as_ref()
                            .is_some_and(|id| id.account == w.account)
                    })
                    .map(|w| {
                        format!(
                            "{}\n**{}h{}min**\n{}\n\n{}\n\n---\n",
                            author_header(&w.author, &w.avatar),
                            w.seconds / 3600,
                            (w.seconds % 3600) / 60,
                            plain_markdown(&w.started),
                            w.body
                        )
                    })
                    .collect(),
                _ => String::new(),
            };
            self.jira_ui.markdown.set(source);
        }
        self.jira_ui
            .images
            .content(&mut self.jira_ui.markdown, &mut self.service.jira.media);
        self.jira_ui.markdown.show(ui, &self.tokens);
        if self.service.jira.media.pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
    pub(super) fn jira_windows(&mut self, ctx: &egui::Context) {
        self.jira_settings(ctx);
        self.jira_picker_window(ctx);
        self.jira_log_window(ctx);
        self.jira_hours_date_window(ctx)
    }
    fn jira_settings(&mut self, ctx: &egui::Context) {
        if !self.jira_ui.settings {
            return;
        }
        let mut close = false;
        let modal=egui::Modal::new(egui::Id::unique("jira.settings")).show(ctx,|ui|{
            self.tokens.form_style(ui);
ui.set_width(520.);
ui.heading("Conectar ao Jira");
ui.label("Tarefas, contexto e horas junto do seu código.");
ui.separator();
            egui::ScrollArea::vertical().id_salt("jira.settings.body").max_height((ctx.content_rect().height()-190.).clamp(160.,460.)).codisseu().show(ui,|ui|{
                field(ui,"Endereço do Jira",&mut self.jira_ui.connection.site,"https://suaempresa.atlassian.net",false);
                field(ui,"Email Atlassian",&mut self.jira_ui.connection.email,"voce@empresa.com.br",false);
                field(ui,"Token de API",self.jira_ui.token.edit(),"Cole o token ou deixe vazio para usar o cofre",true);
                ui.hyperlink_to("Criar ou gerenciar token Atlassian ↗","https://id.atlassian.com/manage-profile/security/api-tokens");
                ui.add_space(8.);
ui.checkbox(&mut self.jira_ui.remember,"Guardar token no cofre do sistema");
                ui.label(RichText::new("Um token novo sem esta opção vale somente nesta sessão.").small().color(color(self.tokens.muted)));
                ui.label(RichText::new("Com o token salvo, o Jira conecta automaticamente ao abrir a IDE.").small().color(color(self.tokens.muted)));
                ui.add_space(10.);
ui.collapsing("Token com escopos · configuração avançada",|ui|{
field(ui,"URL da API",&mut self.jira_ui.connection.api,"https://api.atlassian.com/ex/jira/CLOUD_ID",false);
ui.label("Tokens com escopos usam o gateway Atlassian. Sem escopos, deixe a URL da API igual ao endereço do Jira.");
}
);
                ui.add_space(12.);
                if self.service.jira.connected{
ui.label(RichText::new("Conectado").color(color(self.tokens.accent)));
}
                jira_messages(ui,&self.service.jira.error,&self.service.jira.notice,&self.jira_ui.notice,&self.tokens);
                if self.service.jira.request_pending(){
ui.horizontal(|ui|{
ui.spinner();
ui.label("Validando conexão / cofre…");
}
);
}

}
);
ui.separator();
            ui.horizontal(|ui|{
                if ui.add_enabled(!self.service.jira.main_busy(),egui::Button::new("Esquecer token")).clicked(){
self.service.jira.forget();
self.jira_ui.token=Secret::default()
}
                if self.service.jira.connected && ui.button("Desconectar").clicked(){
self.service.jira.disconnect()
}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    if ui.add(self.tokens.dialog_button("Concluído",false)).clicked(){
close=true
}
                    if ui.add_enabled(!self.service.jira.main_busy(),self.tokens.dialog_button("Conectar",true)).clicked(){
let t=if self.jira_ui.token.expose().is_empty(){
None
}
else{
Some(std::mem::take(&mut self.jira_ui.token))
}
;
let r=self.service.jira.connect(self.jira_ui.connection.clone(),t,self.jira_ui.remember);
self.jira_result(r);
}

}
);

}
);

}
);
        if close || modal.should_close() {
            self.jira_ui.settings = false;
            self.jira_ui.token = Secret::default();
            if self.service.jira.connected {
                self.jira_ui.open = true;
                self.ai_chat.open = false;
                self.show_database = false
            }
        }
    }
    fn jira_picker_window(&mut self, ctx: &egui::Context) {
        let Some(mut p) = self.jira_ui.picker.take() else {
            return;
        };
        let mut close = false;
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::unique("jira.picker")).show(ctx, |ui| {
            self.tokens.form_style(ui);
            ui.set_width(440.);
            ui.heading(format!("Filtrar por {}", p.kind.label().to_lowercase()));
            if ui
                .add(
                    egui::TextEdit::singleline(&mut p.text)
                        .hint_text("Buscar por nome…")
                        .desired_width(f32::INFINITY)
                        .char_limit(256),
                )
                .changed()
            {
                p.deadline = Some(Instant::now());
                self.service.jira.suggestions.clear()
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("jira.picker.list")
                .max_height((ctx.content_rect().height() - 250.).clamp(120., 320.))
                .codisseu()
                .show(ui, |ui| {
                    if self.service.jira.main_busy() {
                        ui.spinner();
                    }
                    for c in &self.service.jira.suggestions {
                        if ui
                            .add(egui::Button::new(&c.name).wrap().frame(false))
                            .clicked()
                        {
                            choice = Some(c.clone());
                        }
                    }
                    if !self.service.jira.main_busy() && self.service.jira.suggestions.is_empty() {
                        ui.label("Nenhuma opção disponível. Digite para pesquisar.");
                    }
                    jira_messages(
                        ui,
                        &self.service.jira.error,
                        &None,
                        &self.jira_ui.notice,
                        &self.tokens,
                    );
                });
            ui.separator();
            if ui.button("Cancelar").clicked() {
                close = true
            }
        });
        if let Some(c) = choice {
            let mut f = self.service.jira.profile.filters.clone();
            match p.kind {
                Picker::User => {
                    f.mine = false;
                    f.assignee = Some(c)
                }
                Picker::Project => f.project = Some(c),
                Picker::Kind => f.kind = Some(c),
                Picker::Parent => f.parent = Some(c),
                Picker::Team => f.team = Some(c),
            }
            let r = self.service.jira.set_filters(f);
            self.jira_result(r)
        } else if !close && !modal.should_close() {
            self.jira_ui.picker = Some(p)
        }
    }
    fn jira_log_window(&mut self, ctx: &egui::Context) {
        let Some(mut w) = self.jira_ui.worklog.take() else {
            return;
        };
        let mut close = false;
        let modal=egui::Modal::new(egui::Id::unique("jira.worklog")).show(ctx,|ui|{
            self.tokens.form_style(ui);
ui.set_width(500.);
ui.heading(if w.draft.key.is_empty() { "Registrar horas".into() } else { format!("Registrar horas · {}",w.draft.key) });
ui.separator();
            egui::ScrollArea::vertical().id_salt("jira.log.body").max_height((ctx.content_rect().height()-210.).clamp(150.,400.)).codisseu().show(ui,|ui|{
                ui.add_enabled_ui(!w.sent&&!w.review,|ui|{
                    if w.choose_issue {
                        field(ui,"Chave do card",&mut w.draft.key,"PROJ-123",false);
                        ui.label(RichText::new("Use a chave de qualquer card em que você possa registrar horas.").small().color(color(self.tokens.muted)));
                        if !self.service.jira.page.issues.is_empty() {
                            egui::ComboBox::from_id_salt("jira.log.task").selected_text("Selecionar das tarefas carregadas").width(ui.available_width()).show_ui(ui, |ui| {
                                for issue in &self.service.jira.page.issues {
                                    ui.selectable_value(&mut w.draft.key, issue.key.clone(), format!("{} · {}",issue.key,issue.title));
                                }
                            });
                        }
                    }
                    ui.horizontal(|ui|{
let duration_label=ui.label("Duração em minutos");
ui.add(egui::DragValue::new(&mut w.draft.minutes).range(1..=1440)).labelled_by(duration_label.id);
}
);
                    field(ui,"Início com fuso",&mut w.draft.started,"2026-11-03T09:00:00-03:00",false);
                    let description_label=ui.label("Descrição do trabalho");
ui.add(egui::TextEdit::multiline(&mut w.draft.comment).desired_rows(4).desired_width(f32::INFINITY).char_limit(16384)).labelled_by(description_label.id);

}
);
                if w.review{
ui.add_space(12.);
ui.strong(format!("{} · {}h{}min",w.draft.key,w.draft.minutes/60,w.draft.minutes%60));
ui.label(format!("Início: {}",w.draft.started));
ui.label("Será criado um apontamento no Jira com a sua conta. A estimativa restante será mantida.");
if w.choose_issue {
    if self.service.jira.main_busy() { ui.label("Conferindo o card no Jira…"); }
    else if let Some(detail) = &self.service.jira.detail { ui.label(&detail.issue.title); }
}
}
                jira_messages(ui,&self.service.jira.error,&self.service.jira.notice,&self.jira_ui.notice,&self.tokens);
                if w.sent{
ui.spinner();
ui.label("Enviando uma única vez. Aguarde a confirmação.");
}

}
);
ui.separator();
ui.horizontal(|ui|{
                if ui.button(if w.review{
"Voltar"
}
else{
"Cancelar"
}
).clicked(){
if w.review{
w.review=false
}
else{
close=true
}
}
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{
                    if !w.review {
if ui.add(self.tokens.dialog_button("Revisar apontamento",true)).clicked(){
w.draft.key = w.draft.key.trim().to_ascii_uppercase();
let mut r=w.draft.validate();
if r.is_ok() && w.choose_issue { r=self.service.jira.select(&w.draft.key); }
if r.is_ok(){
w.review=true;
self.jira_ui.notice=None
}
self.jira_result(r)
}
}
                    else if ui.add_enabled(!w.sent&&!self.service.jira.main_busy() && (!w.choose_issue || self.service.jira.detail.as_ref().is_some_and(|d| d.issue.key == w.draft.key)),self.tokens.dialog_button("Confirmar e registrar",true)).clicked(){
let r=self.service.jira.post_worklog(w.draft.clone());
if r.is_ok(){
w.sent=true
}
self.jira_result(r)
}

}
);

}
);

}
);
        if !close && (!modal.should_close() || w.sent) {
            self.jira_ui.worklog = Some(w)
        }
    }
}
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str, password: bool) {
    ui.add_space(8.);
    let l = ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .password(password)
            .desired_width(f32::INFINITY)
            .char_limit(if password { 16384 } else { 1024 }),
    )
    .labelled_by(l.id);
}
fn jira_messages(
    ui: &mut egui::Ui,
    error: &Option<String>,
    notice: &Option<String>,
    local: &Option<String>,
    tokens: &Tokens,
) {
    for (msg, err) in [(error, true), (notice, false), (local, true)] {
        if let Some(s) = msg {
            ui.add_space(4.);
            ui.label(RichText::new(s).small().color(color(if err {
                tokens.error
            } else {
                tokens.muted
            })));
        }
    }
}
fn jira_pager(
    ui: &mut egui::Ui,
    offset: usize,
    len: usize,
    total: usize,
    mut select: impl FnMut(usize),
) {
    ui.horizontal(|ui| {
        ui.label(format!(
            "{}–{} de {}",
            if len > 0 { offset + 1 } else { 0 },
            offset + len,
            total
        ));
        if ui
            .add_enabled(offset > 0, egui::Button::new("‹ Anteriores"))
            .clicked()
        {
            select(offset.saturating_sub(20))
        }
        if ui
            .add_enabled(
                offset + len < total && len > 0,
                egui::Button::new("Próximos ›"),
            )
            .clicked()
        {
            select(offset + len)
        }
    });
}
fn plain_markdown(s: &str) -> String {
    s.replace(['\r', '\n'], " ")
        .replace('*', "\\*")
        .replace('#', "\\#")
        .replace('[', "\\[")
        .replace('`', "\\`")
}
fn author_header(name: &str, avatar: &str) -> String {
    if avatar.is_empty() {
        format!("### {}", plain_markdown(name))
    } else {
        format!("![codisseu-avatar:{}](<{}>)", plain_markdown(name), avatar)
    }
}

fn task_row(
    ui: &mut egui::Ui,
    issue: &jira::Issue,
    selected: bool,
    images: &mut crate::jira_images::Images,
    media: &mut jira::media::Media,
    tokens: &Tokens,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), TASK_ROW_HEIGHT),
        egui::Sense::click(),
    );
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            4,
            color(if selected {
                tokens.rail_selection
            } else {
                tokens.editor
            }),
        );
    }
    let width = (rect.width() - 48.).max(40.);
    let mut key = egui::text::LayoutJob::simple(
        format!("{} · {}", issue.key, issue.status),
        FontId::proportional(11.),
        color(tokens.accent),
        width,
    );
    key.wrap.max_rows = 1;
    key.wrap.break_anywhere = true;
    let key = ui.fonts_mut(|f| f.layout_job(key));
    let mut title = egui::text::LayoutJob::simple(
        issue.title.clone(),
        FontId::proportional(13.),
        color(tokens.text),
        width,
    );
    title.wrap.max_rows = 2;
    title.wrap.break_anywhere = false;
    let title = ui.fonts_mut(|f| f.layout_job(title));
    let text = rect.min + egui::vec2(40., 9.);
    let title_pos = text + egui::vec2(0., key.size().y + 3.);
    let avatar_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 20., title_pos.y + title.size().y / 2.),
        egui::vec2(24., 24.),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(avatar_rect), |ui| {
        images.avatar(
            ui,
            media,
            &issue.assignee_avatar,
            &issue.assignee.name,
            24.,
            tokens,
        );
    });
    ui.put(
        egui::Rect::from_min_size(text, key.size()),
        egui::Label::new(key)
            .selectable(false)
            .sense(egui::Sense::hover()),
    );
    ui.put(
        egui::Rect::from_min_size(title_pos, title.size()),
        egui::Label::new(title)
            .selectable(false)
            .sense(egui::Sense::hover()),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::accesskit::Role::Button,
            true,
            format!("Abrir tarefa {}", issue.key),
        )
    });
    ui.advance_cursor_after_rect(rect);
    response
}
