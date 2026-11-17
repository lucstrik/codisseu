//! Personal time overview: one period, a calendar, and the selected day's records.
use super::*;
use codisseu_core::jira::{
    self as jira,
    period::{self, Mode, Period},
    timesheet::{Snapshot, State},
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct HoursUi {
    pub selected: String,
    pub date_picker: bool,
    date_input: String,
    date_error: Option<String>,
    scroll_day: bool,
    cache: Option<(Arc<Snapshot>, Period, Arc<Snapshot>)>,
}
impl CodisseuApp {
    pub(super) fn open_jira_hours(&mut self) {
        self.jira_open_tab(crate::jira_tabs::Kind::Hours);
        let period = self.service.jira.profile.timesheet.clone();
        if self.jira_ui.hours.selected.is_empty() {
            self.jira_ui.hours.selected = period.anchor.clone();
        }
        self.ensure_jira_hours(false);
    }
    fn ensure_jira_hours(&mut self, force: bool) {
        let p = self.service.jira.profile.timesheet.clone();
        let covered = self.service.jira.timesheet.view.as_ref().is_some_and(|v| {
            if matches!(v.state, State::Error) {
                return false;
            }
            v.period
                .instants()
                .ok()
                .zip(p.instants().ok())
                .is_some_and(|(source, selected)| source == selected)
        });
        if self.service.jira.connected
            && (force || self.service.jira.timesheet.stale || !covered)
            && let Err(error) = self.service.jira.timesheet.start(p)
        {
            self.jira_ui.hours.date_error = Some(error);
        }
    }
    pub(super) fn jira_hours_tick(&mut self) {
        let connected = self.service.jira.connected;
        if !connected {
            self.jira_ui.hours.cache = None;
        }
        if !self.service.jira.profile.enabled {
            self.jira_ui.hours = HoursUi::default();
        }
        if self.jira_ui.tabs.is_active(crate::jira_tabs::Kind::Hours)
            && connected
            && !self.service.jira.timesheet.busy
            && self.service.jira.timesheet.stale
        {
            self.ensure_jira_hours(self.service.jira.timesheet.stale);
        }
    }
    fn choose_jira_period(&mut self, p: Period) {
        if let Err(e) = p.validate() {
            self.jira_ui.hours.date_error = Some(e);
            return;
        }
        let mut profile = self.service.jira.profile.clone();
        profile.timesheet = p.clone();
        if let Err(error) = self.service.jira.set_profile(profile) {
            self.jira_ui.hours.date_error = Some(error);
            return;
        }
        self.jira_ui.hours.date_error = None;
        self.jira_ui.hours.selected = p.anchor.clone();
        self.jira_ui.hours.cache = None;
        self.ensure_jira_hours(false);
    }
    pub(super) fn jira_created_for_month(&mut self, month: String) {
        let filters = jira::Filters {
            mine: true,
            status: jira::Status::All,
            created_month: Some(month),
            ..Default::default()
        };
        let r = self.service.jira.set_filters(filters);
        self.jira_result(r);
        self.jira_ui.open = true;
        self.ai_chat.open = false;
        self.show_database = false;
    }
    fn jira_start_hours_worklog(&mut self) {
        let date = if period::parse_date(&self.jira_ui.hours.selected).is_ok() {
            self.jira_ui.hours.selected.clone()
        } else {
            self.service.jira.profile.timesheet.anchor.clone()
        };
        let started = match period::worklog_start(&date) {
            Ok(value) => value,
            Err(error) => {
                self.jira_ui.hours.date_error = Some(error);
                return;
            }
        };
        self.jira_ui.notice = None;
        self.jira_ui.worklog = Some(crate::jira::LogUi::for_calendar(started));
    }
    fn display_jira_hours(&mut self) -> Option<Arc<Snapshot>> {
        let source = self.service.jira.timesheet.view.as_ref()?;
        let p = &self.service.jira.profile.timesheet;
        let ((from, to), (start, end)) = (source.period.instants().ok()?, p.instants().ok()?);
        if from > start || to < end {
            return None;
        }
        if self
            .jira_ui
            .hours
            .cache
            .as_ref()
            .is_none_or(|(old, period, _)| !Arc::ptr_eq(old, source) || period != p)
        {
            let (a, b) = p.dates().ok()?;
            let a = a.to_string();
            let b = b.to_string();
            let entries = source
                .entries
                .iter()
                .filter(|e| e.date >= a && e.date < b)
                .cloned()
                .collect();
            let snapshot = Snapshot::from_entries(
                p.clone(),
                entries,
                source.state.clone(),
                source.inspected,
                source.requests,
                source.message.clone(),
            );
            self.jira_ui.hours.cache = Some((source.clone(), p.clone(), Arc::new(snapshot)));
        }
        self.jira_ui.hours.cache.as_ref().map(|(_, _, v)| v.clone())
    }
    pub(super) fn jira_hours_page(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new().inner_margin(24).show(ui, |ui| {
            let Some((period, snapshot, complete)) = self.jira_hours_controls(ui) else {
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("jira.hours.page")
                .auto_shrink([false, false])
                .codisseu()
                .show(ui, |ui| {
                    self.jira_hours_details(ui, &period, snapshot.as_deref(), complete)
                });
        });
    }
    fn jira_hours_controls(
        &mut self,
        ui: &mut egui::Ui,
    ) -> Option<(Period, Option<Arc<Snapshot>>, bool)> {
        let mut p = self.service.jira.profile.timesheet.clone();
        ui.label(RichText::new("Minhas horas").size(24.).strong());
        ui.add_space(6.);
        if let Some(id) = &self.service.jira.identity {
            ui.label(RichText::new(format!("{} · Horário local",id.name)).color(color(self.tokens.muted))).on_hover_text("Somente apontamentos desta conta. Agrupados pelo dia de início no fuso deste computador.");
        }
        ui.add_space(18.);
        if !self.service.jira.connected {
            ui.strong("Conecte sua conta para consultar seus apontamentos.");
            ui.add_space(10.);
            if ui.button("Configurar conexão Jira…").clicked() {
                self.open_jira_settings();
            }
            return None;
        }
        let mut changed = false;
        let mut refresh_current = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8., 8.);
            for mode in [Mode::Month, Mode::Week, Mode::Day] {
                if ui.selectable_label(p.mode == mode, mode.label()).clicked() {
                    if mode != Mode::Month
                        && period::parse_date(&self.jira_ui.hours.selected).is_ok()
                    {
                        p.anchor = self.jira_ui.hours.selected.clone();
                    }
                    p.mode = mode;
                    changed = true;
                }
            }
            if ui.button("Hoje").clicked() {
                p.anchor = period::today();
                changed = true;
                refresh_current = true;
            }
            if ui.button("Mês atual").clicked() {
                p = Period::default();
                changed = true;
                refresh_current = true;
            }
            if ui.button("Escolher data…").clicked() {
                self.jira_ui.hours.date_picker = true;
                self.jira_ui.hours.date_input = period::parse_date(&p.anchor)
                    .map(|d| d.format("%d/%m/%Y").to_string())
                    .unwrap_or_default();
                self.jira_ui.hours.date_error = None;
            }
        });
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.;
            if icon_button(ui, Icon::Previous, "Período anterior de horas", false, 28.).clicked() {
                match p.shifted(-1) {
                    Ok(n) => {
                        p = n;
                        changed = true
                    }
                    Err(e) => self.jira_ui.hours.date_error = Some(e),
                }
            }
            ui.add(
                egui::Label::new(RichText::new(p.label()).strong())
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            if icon_button(ui, Icon::Next, "Próximo período de horas", false, 28.).clicked() {
                match p.shifted(1) {
                    Ok(n) => {
                        p = n;
                        changed = true
                    }
                    Err(e) => self.jira_ui.hours.date_error = Some(e),
                }
            }
        });
        if changed {
            self.choose_jira_period(p.clone());
            // Today/current month also refreshes a cached empty or failed range.
            if refresh_current && !self.service.jira.timesheet.busy {
                self.ensure_jira_hours(true);
            }
        }
        ui.add_space(14.);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8., 8.);
            ui.spacing_mut().button_padding = egui::vec2(12., 6.);
            if ui
                .add(self.tokens.dialog_button("Registrar horas…", true))
                .clicked()
            {
                self.jira_start_hours_worklog();
            }
            if ui
                .add_enabled(
                    !self.service.jira.timesheet.busy,
                    egui::Button::new("Atualizar horas"),
                )
                .clicked()
            {
                self.ensure_jira_hours(true);
            }
            if self.service.jira.timesheet.busy && ui.button("Interromper consulta").clicked() {
                self.service.jira.timesheet.cancel();
            }
            if self.service.jira.timesheet.resumable()
                && ui
                    .button(if self.service.jira.timesheet.error.is_some() {
                        "Tentar novamente"
                    } else {
                        "Continuar carregamento"
                    })
                    .clicked()
            {
                let r = self.service.jira.timesheet.continue_scan();
                self.jira_result(r);
            }
            if ui
                .button("Localizar cards para apontar")
                .on_hover_text(
                    "Filtrar cards criados neste mês e atribuídos a você, incluindo concluídos",
                )
                .clicked()
            {
                self.jira_created_for_month(p.month());
            }
        });
        ui.add_space(16.);
        let snapshot = self.display_jira_hours();
        let complete = snapshot
            .as_ref()
            .is_some_and(|s| s.state == State::Complete);
        let busy = self.service.jira.timesheet.busy;
        if busy || !complete {
            ui.horizontal_wrapped(|ui| {
                if busy {
                    ui.add(egui::Spinner::new().size(14.));
                }
                ui.label(
                    RichText::new(if busy {
                        "Consultando seus apontamentos…"
                    } else {
                        "Consulta ainda não concluída"
                    })
                    .color(color(self.tokens.muted)),
                );
                if let Some(s) = &snapshot {
                    ui.label(
                        RichText::new(format!("{} cards conferidos", s.inspected))
                            .small()
                            .color(color(self.tokens.muted)),
                    );
                }
            });
            ui.add_space(10.);
        }
        if let Some(message) = snapshot.as_ref().and_then(|s| s.message.as_deref()) {
            ui.label(RichText::new(message).color(color(self.tokens.error)));
            ui.add_space(10.);
        }
        if complete && snapshot.as_ref().is_some_and(|s| s.entries.is_empty()) {
            ui.label(
                RichText::new("Nenhum apontamento encontrado neste período.")
                    .color(color(self.tokens.muted)),
            );
            ui.add_space(8.);
        }
        if let Some(error) = &self.jira_ui.hours.date_error {
            ui.label(RichText::new(error).color(color(self.tokens.error)));
        }
        let has_data = snapshot.as_ref().is_some_and(|s| !s.entries.is_empty());
        let total = snapshot.as_ref().map_or(0, |s| s.seconds);
        let days = snapshot.as_ref().map_or(0, |s| s.days.len());
        let cards = snapshot.as_ref().map_or(0, |s| s.cards);
        let values = [
            (
                if complete {
                    "Horas no período"
                } else {
                    "Horas encontradas"
                },
                if complete || has_data {
                    period::duration(total)
                } else {
                    "…".into()
                },
            ),
            (
                "Dias com apontamentos",
                if complete || has_data {
                    days.to_string()
                } else {
                    "…".into()
                },
            ),
            (
                "Cards com apontamentos",
                if complete || has_data {
                    cards.to_string()
                } else {
                    "…".into()
                },
            ),
        ];
        ui.columns(3, |columns| {
            for (column, (label, value)) in columns.iter_mut().zip(values) {
                egui::Frame::new()
                    .fill(color(self.tokens.editor))
                    .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
                    .corner_radius(6)
                    .inner_margin(14)
                    .show(column, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(
                            egui::Label::new(
                                RichText::new(label).small().color(color(self.tokens.muted)),
                            )
                            .truncate(),
                        )
                        .on_hover_text(label);
                        ui.add_space(8.);
                        ui.label(RichText::new(value).size(23.).strong());
                    });
            }
        });
        ui.add_space(18.);
        Some((p, snapshot, complete))
    }
    fn jira_hours_details(
        &mut self,
        ui: &mut egui::Ui,
        p: &Period,
        snapshot: Option<&Snapshot>,
        complete: bool,
    ) {
        ui.add_space(20.);
        if p.mode != Mode::Day {
            self.jira_hours_calendar(ui, p, snapshot, complete);
            ui.add_space(20.);
        }
        let date = if p.mode == Mode::Day {
            p.anchor.clone()
        } else {
            self.jira_ui.hours.selected.clone()
        };
        let title = period::parse_date(&date)
            .map(|d| d.format("%d/%m/%Y").to_string())
            .unwrap_or_else(|_| p.anchor.clone());
        ui.separator();
        ui.add_space(14.);
        let heading = ui.label(
            RichText::new(format!("Apontamentos de {title}"))
                .size(18.)
                .strong(),
        );
        if self.jira_ui.hours.scroll_day {
            heading.scroll_to_me(Some(egui::Align::TOP));
            self.jira_ui.hours.scroll_day = false;
        }
        ui.add_space(8.);
        if let Some(s) = &snapshot {
            if let Some(day) = s.days.get(&date) {
                ui.label(
                    RichText::new(format!(
                        "{} · {} cards · {} apontamentos",
                        period::duration(day.seconds),
                        day.cards,
                        day.records
                    ))
                    .color(color(self.tokens.muted)),
                );
                ui.add_space(12.);
            }
            let entries: Vec<_> = s.entries.iter().filter(|e| e.date == date).collect();
            if entries.is_empty() {
                ui.label(if complete {
                    "Nenhum apontamento neste dia."
                } else {
                    "A conferência ainda não encontrou apontamentos neste dia."
                });
            }
            let mut target = None;
            egui::ScrollArea::vertical()
                .id_salt(("jira.hours.rows", &date))
                .max_height(420.)
                .auto_shrink([false, true])
                .codisseu()
                .show_rows(ui, 78., entries.len(), |ui, range| {
                    for i in range {
                        let e = entries[i];
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .fill(color(self.tokens.editor))
                            .corner_radius(4)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&e.time).color(color(self.tokens.muted)),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.strong(period::duration(e.seconds));
                                        },
                                    );
                                });
                                if ui
                                    .add(
                                        egui::Button::new(format!("{} · {}", e.key, e.title))
                                            .frame(false)
                                            .truncate(),
                                    )
                                    .clicked()
                                {
                                    target = Some(e.key.clone());
                                }
                                if !e.note.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&e.note)
                                                .small()
                                                .color(color(self.tokens.muted)),
                                        )
                                        .truncate(),
                                    );
                                }
                            });
                    }
                });
            if let Some(key) = target {
                self.jira_open_issue(&key, true);
            }
        }
        ui.add_space(18.);
    }
    fn jira_hours_calendar(
        &mut self,
        ui: &mut egui::Ui,
        p: &Period,
        snapshot: Option<&Snapshot>,
        complete: bool,
    ) {
        let Ok(days) = p.calendar() else { return };
        let gap = 6.;
        let cell_width = ((ui.available_width() - gap * 6.) / 7.).max(36.);
        let height = if p.mode == Mode::Week { 80. } else { 60. };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for name in ["Seg", "Ter", "Qua", "Qui", "Sex", "Sáb", "Dom"] {
                ui.add_sized(
                    [cell_width, 20.],
                    egui::Label::new(RichText::new(name).small().color(color(self.tokens.muted))),
                );
            }
        });
        for week in days.chunks(7) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for d in week {
                    let day = snapshot.and_then(|s| s.days.get(&d.date));
                    let time = day
                        .map(|d| period::duration(d.seconds))
                        .unwrap_or_else(|| if complete { "—".into() } else { "…".into() });
                    let n = day.map_or(0, |d| d.cards);
                    let selected = self.jira_ui.hours.selected == d.date;
                    let mut button = egui::Button::new("").selected(selected);
                    if day.is_some() && !selected {
                        button = button.fill(color(self.tokens.accent).gamma_multiply(0.16));
                    }
                    let r = ui
                        .add_enabled_ui(d.in_period, |ui| {
                            ui.add_sized([cell_width, height], button)
                        })
                        .inner;
                    let text_color = if d.in_period {
                        color(self.tokens.text)
                    } else {
                        color(self.tokens.muted)
                    };
                    let left = r.rect.left() + 8.;
                    let top = r.rect.top() + 7.;
                    ui.painter().text(
                        egui::pos2(left, top),
                        egui::Align2::LEFT_TOP,
                        d.number.to_string(),
                        egui::FontId::proportional(12.),
                        text_color,
                    );
                    if d.in_period {
                        let compact = if cell_width < 68. {
                            day.map(|d| {
                                format!("{}:{:02}", d.seconds / 3600, (d.seconds % 3600) / 60)
                            })
                            .unwrap_or(time.clone())
                        } else {
                            time.clone()
                        };
                        ui.painter().text(
                            egui::pos2(left, top + 17.),
                            egui::Align2::LEFT_TOP,
                            compact,
                            egui::FontId::proportional(12.),
                            text_color,
                        );
                        if n > 0 {
                            let label = if cell_width < 60. {
                                n.to_string()
                            } else {
                                format!("{n} cards")
                            };
                            ui.painter().text(
                                egui::pos2(left, top + 33.),
                                egui::Align2::LEFT_TOP,
                                label,
                                egui::FontId::proportional(10.),
                                text_color,
                            );
                        }
                    }
                    let r = r.on_hover_text(format!("{} · {time} · {n} cards", d.date));
                    r.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::accesskit::Role::Button,
                            d.in_period,
                            self.jira_ui.hours.selected == d.date,
                            format!("Horas em {}", d.date),
                        )
                    });
                    if r.clicked() {
                        self.jira_ui.hours.selected = d.date.clone();
                        self.jira_ui.hours.scroll_day = true;
                    }
                }
            });
            ui.add_space(gap);
        }
    }
    pub(super) fn jira_hours_date_window(&mut self, ctx: &egui::Context) {
        if !self.jira_ui.hours.date_picker {
            return;
        }
        let mut close = false;
        let mut apply = false;
        let modal = egui::Modal::new(egui::Id::unique("jira.hours.date")).show(ctx, |ui| {
            self.tokens.form_style(ui);
            ui.set_width(340.);
            ui.heading("Escolher uma data");
            ui.add_space(10.);
            let label = ui.label("Data · dd/mm/aaaa");
            ui.add_space(6.);
            let input = ui
                .add(
                    egui::TextEdit::singleline(&mut self.jira_ui.hours.date_input)
                        .desired_width(f32::INFINITY)
                        .hint_text("03/11/2026")
                        .char_limit(10),
                )
                .labelled_by(label.id);
            if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
            if let Some(e) = &self.jira_ui.hours.date_error {
                ui.label(RichText::new(e).color(color(self.tokens.error)));
            }
            ui.add_space(12.);
            ui.horizontal(|ui| {
                if ui.button("Cancelar").clicked() {
                    close = true
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    apply |= ui
                        .add(self.tokens.dialog_button("Ir para data", true))
                        .clicked();
                });
            });
        });
        if apply {
            match period::typed_date(&self.jira_ui.hours.date_input) {
                Ok(anchor) => {
                    let p = Period {
                        mode: self.service.jira.profile.timesheet.mode,
                        anchor,
                    };
                    self.choose_jira_period(p);
                    close = true;
                    self.jira_ui.hours.date_error = None
                }
                Err(e) => self.jira_ui.hours.date_error = Some(e),
            }
        }
        if close || modal.should_close() {
            self.jira_ui.hours.date_picker = false;
        }
    }
}
