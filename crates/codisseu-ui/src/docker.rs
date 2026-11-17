//! Native Docker tool window. All process work belongs to the application service.
use super::*;
use codisseu_core::docker::{Container, ContainerAction, DockerCommand};

#[derive(Clone)]
struct RemoveTarget {
    id: String,
    name: String,
    context: String,
    action: ContainerAction,
}

fn removal_action(container: &Container) -> Option<ContainerAction> {
    if ContainerAction::Remove.can_apply(container) {
        Some(ContainerAction::Remove)
    } else {
        ContainerAction::StopAndRemove
            .can_apply(container)
            .then_some(ContainerAction::StopAndRemove)
    }
}

#[derive(Default)]
pub(super) struct DockerUi {
    pub open: bool,
    pub pending_shell: Option<(String, String, String, String)>,
    filter: String,
    focus_next: Option<String>,
    running_only: bool,
    logs: bool,
    confirm_remove: Option<RemoveTarget>,
    settings: bool,
    program: String,
}

// Docker controls keep the neutral monochrome treatment of the tool rail.
fn action_button(ui: &mut egui::Ui, icon: Icon, label: &str, enabled: bool) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| {
        let response = ui.add_sized([26., 26.], egui::Button::new("").frame(false));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::accesskit::Role::Button, ui.is_enabled(), label)
        });
        if response.hovered() || response.is_pointer_button_down_on() {
            ui.painter()
                .rect_filled(response.rect.shrink(2.), 3., Color32::from_white_alpha(22));
        }
        if response.has_focus() {
            ui.painter().rect_stroke(
                response.rect.shrink(2.),
                3.,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
        components::paint_icon_monochrome(
            ui,
            response.rect,
            icon,
            if ui.is_enabled() {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
        response.on_hover_text(label)
    })
    .inner
}

fn state_label(state: &str) -> &str {
    match state {
        "running" => "Em execução",
        "exited" => "Parado",
        "paused" => "Pausado",
        "restarting" => "Reiniciando",
        "created" => "Criado",
        "dead" => "Encerrado",
        "removing" => "Removendo",
        other => other,
    }
}

fn container_row(ui: &mut egui::Ui, container: &Container, selected: bool) -> egui::Response {
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), 44.));
    let response = ui.interact(
        rect,
        egui::Id::unique(("codisseu-docker-row", &container.id)),
        egui::Sense::click(),
    );
    if response.has_focus() {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                response.id,
                egui::EventFilter {
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::accesskit::Role::ListBoxOption,
            ui.is_enabled(),
            selected,
            format!("{} · {}", container.name, state_label(&container.state)),
        )
    });
    let painter = ui.painter().with_clip_rect(rect);
    if selected || response.hovered() {
        painter.rect_filled(
            rect,
            3.,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.weak_bg_fill
            },
        );
    }
    if response.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.),
            3.,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let tint = ui.visuals().weak_text_color();
    let center = egui::pos2(rect.left() + 9., rect.top() + 13.);
    if container.state == "running" {
        painter.circle_filled(center, 3., tint);
    } else {
        painter.circle_stroke(center, 3., egui::Stroke::new(1., tint));
    }
    for (text, y, color) in [
        (
            container.name.clone(),
            rect.top() + 4.,
            ui.visuals().text_color(),
        ),
        (
            format!("{} · {}", state_label(&container.state), container.image),
            rect.top() + 23.,
            tint,
        ),
    ] {
        let galley = painter.layout(
            text,
            FontId::proportional(12.),
            color,
            (rect.width() - 27.).max(1.),
        );
        // Only the first visual line of each label is shown; full names stay in the tooltip.
        painter
            .with_clip_rect(egui::Rect::from_min_size(
                egui::pos2(rect.left() + 21., y),
                egui::vec2((rect.width() - 27.).max(0.), 17.),
            ))
            .galley(egui::pos2(rect.left() + 21., y), galley, color);
    }
    response.on_hover_text(format!(
        "{}\n{}\n{}\n{}",
        container.name, container.image, container.status, container.id
    ))
}

fn metadata(ui: &mut egui::Ui, label: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    ui.label(RichText::new(label).weak().size(11.));
    ui.add(egui::Label::new(value).wrap().selectable(true));
    ui.add_space(7.);
}

impl CodisseuApp {
    fn docker_send(&mut self, command: DockerCommand) {
        if let Err(error) = self.service.dispatch(Command::Docker(command)) {
            self.service.docker.state.error = Some(error);
        }
    }

    pub(super) fn docker_panel(&mut self, ui: &mut egui::Ui) {
        self.tokens.form_style(ui);
        ui.spacing_mut().item_spacing = egui::vec2(4., 4.);
        let busy = self.service.docker.is_foreground_busy();
        let selected = self.service.docker.state.selected.clone();
        self.docker_validate_selection();
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.docker_cancel_remove();
        }
        header(ui, self.tokens.tool_header_height, |ui| {
            ui.add_space(12.);
            ui.label(RichText::new("Docker").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.);
                if action_button(ui, Icon::Remove, "Ocultar Docker", true).clicked() {
                    self.docker_ui.open = false;
                }
                if action_button(ui, Icon::Settings, "Configurar Docker", !busy).clicked() {
                    self.docker_ui.settings = !self.docker_ui.settings;
                    self.docker_ui.program =
                        self.service.docker.program().to_string_lossy().into_owned();
                }
                if action_button(
                    ui,
                    if busy { Icon::Stop } else { Icon::Refresh },
                    if busy {
                        "Cancelar operação Docker"
                    } else {
                        "Atualizar containers"
                    },
                    true,
                )
                .clicked()
                {
                    self.docker_send(if busy {
                        DockerCommand::Cancel
                    } else {
                        DockerCommand::Refresh
                    });
                }
            });
        });
        egui::Frame::new()
            .inner_margin(self.tokens.content_margin())
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if self.docker_ui.settings {
                    ui.label("Executável Docker");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.docker_ui.program)
                            .desired_width(ui.available_width())
                            .frame(
                                egui::Frame::new()
                                    .fill(color(self.tokens.panel))
                                    .stroke(egui::Stroke::new(
                                        1.,
                                        color(self.tokens.control_border),
                                    ))
                                    .corner_radius(self.tokens.control_corner_radius)
                                    .inner_margin(egui::Margin::symmetric(6, 4)),
                            )
                            .id_salt("docker-program")
                            .hint_text("docker"),
                    );
                    field.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::accesskit::Role::TextInput,
                            !busy,
                            "Executável Docker",
                        )
                    });
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(
                                !busy && !self.docker_ui.program.trim().is_empty(),
                                egui::Button::new("Aplicar Docker"),
                            )
                            .clicked()
                        {
                            self.docker_send(DockerCommand::Configure {
                                program: PathBuf::from(self.docker_ui.program.trim()),
                            });
                            self.docker_ui.settings = false;
                        }
                        if ui.button("Cancelar configuração").clicked() {
                            self.docker_ui.settings = false;
                        }
                    });
                    ui.separator();
                }
                let context = self
                    .service
                    .docker
                    .state
                    .context
                    .clone()
                    .unwrap_or_else(|| "Contexto atual".into());
                let contexts = self.service.docker.state.contexts.clone();
                let mut next_context = None;
                ui.add_enabled_ui(!busy, |ui| {
                    egui::ComboBox::from_id_salt("docker-context")
                        .width(ui.available_width())
                        .selected_text(context.clone())
                        .show_ui(ui, |ui| {
                            if contexts.is_empty() {
                                ui.label("Atualize para descobrir contextos");
                            }
                            for c in contexts {
                                if ui
                                    .selectable_label(c.name == context, &c.name)
                                    .on_hover_text(&c.description)
                                    .clicked()
                                {
                                    next_context = Some(c.name);
                                }
                            }
                        })
                        .response
                        .on_hover_text("Contexto Docker");
                });
                if let Some(context) = next_context {
                    self.docker_ui.confirm_remove = None;
                    self.docker_send(DockerCommand::SelectContext(context));
                }
                let filter = ui.add(
                    egui::TextEdit::singleline(&mut self.docker_ui.filter)
                        .desired_width(ui.available_width())
                        .id_salt("docker-filter")
                        .frame(
                            egui::Frame::new()
                                .fill(color(self.tokens.panel))
                                .stroke(egui::Stroke::new(1., color(self.tokens.control_border)))
                                .corner_radius(self.tokens.control_corner_radius)
                                .inner_margin(egui::Margin::symmetric(6, 4)),
                        )
                        .hint_text("Filtrar containers…"),
                );
                filter.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::accesskit::Role::TextInput,
                        true,
                        "Filtrar containers",
                    )
                });
                ui.checkbox(&mut self.docker_ui.running_only, "Somente em execução");
                if !(self.show_activity && self.tool_tab == ToolTab::Docker) {
                    self.docker_feedback(ui);
                }
                if !self.service.docker.state.connected && !busy {
                    ui.add_space(8.);
                    ui.label("Conecte ao Docker para visualizar seus containers.");
                    if ui.button("Conectar ao Docker").clicked() {
                        self.docker_send(DockerCommand::Refresh);
                    }
                }
                let filter = self.docker_ui.filter.to_lowercase();
                let containers: Vec<_> = self
                    .service
                    .docker
                    .state
                    .containers
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        (!self.docker_ui.running_only || c.state == "running")
                            && (filter.is_empty()
                                || c.name.to_lowercase().contains(&filter)
                                || c.image.to_lowercase().contains(&filter)
                                || c.id.contains(&filter))
                    })
                    .map(|(index, _)| index)
                    .collect();
                ui.label(
                    RichText::new(format!("Containers ({})", containers.len()))
                        .weak()
                        .size(11.),
                );
                let list_height = if containers.is_empty() {
                    0.
                } else {
                    ui.available_height().max(44.)
                };
                ui.spacing_mut().item_spacing.y = 0.;
                egui::ScrollArea::vertical()
                    .id_salt("docker-containers")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .codisseu()
                    .show_rows(ui, 44., containers.len(), |ui, range| {
                        ui.spacing_mut().item_spacing.y = 0.;
                        for index in range {
                            let c = self.service.docker.state.containers[containers[index]].clone();
                            let response = container_row(ui, &c, selected.as_ref() == Some(&c.id));
                            if self.docker_ui.focus_next.as_ref() == Some(&c.id) {
                                response.request_focus();
                                self.docker_ui.focus_next = None;
                            }
                            let activate = response.clicked()
                                || (response.has_focus()
                                    && ui.input(|i| {
                                        i.key_pressed(Key::Enter) || i.key_pressed(Key::Space)
                                    }));
                            if activate && !busy {
                                response.request_focus();
                                self.docker_select_container(c.id.clone());
                            }
                            response.context_menu(|ui| {
                                if ui
                                    .add_enabled(
                                        !busy
                                            && self.service.docker.state.connected
                                            && removal_action(&c).is_some(),
                                        egui::Button::new("Excluir container…"),
                                    )
                                    .clicked()
                                {
                                    ui.close();
                                    self.docker_request_remove(&c);
                                }
                            });
                            if response.has_focus() && !busy {
                                if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete)) {
                                    self.docker_request_remove(&c);
                                }
                                let direction = ui.input_mut(|i| {
                                    if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                                        1
                                    } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                                        -1
                                    } else {
                                        0
                                    }
                                });
                                if direction != 0 {
                                    let next = (index as isize + direction)
                                        .clamp(0, containers.len() as isize - 1)
                                        as usize;
                                    let next_id = self.service.docker.state.containers
                                        [containers[next]]
                                        .id
                                        .clone();
                                    self.docker_ui.focus_next = Some(next_id.clone());
                                    ui.scroll_to_rect(
                                        response.rect.translate(egui::vec2(
                                            0.,
                                            (next as isize - index as isize) as f32 * 44.,
                                        )),
                                        None,
                                    );
                                    self.docker_select_container(next_id);
                                }
                            }
                        }
                    });
                ui.spacing_mut().item_spacing.y = 4.;
                if containers.is_empty() && self.service.docker.state.connected {
                    ui.label(
                        RichText::new(if self.service.docker.state.containers.is_empty() {
                            "Nenhum container neste contexto."
                        } else {
                            "Nenhum container corresponde ao filtro."
                        })
                        .weak(),
                    );
                }
            });
    }

    pub(super) fn docker_open_settings(&mut self) {
        self.show_project = true;
        self.api_ui.open = false;
        self.docker_ui.open = true;
        self.git_ui.commit_open = false;
        self.git_ui.source_control = false;
        self.docker_ui.settings = true;
        self.docker_ui.program = self.service.docker.program().to_string_lossy().into_owned();
    }

    fn docker_select_container(&mut self, id: String) -> bool {
        self.docker_ui.confirm_remove = None;
        if self.service.docker.state.selected.as_ref() != Some(&id)
            || self
                .service
                .docker
                .state
                .details
                .as_ref()
                .is_none_or(|details| details.id != id)
        {
            if let Err(error) = self
                .service
                .dispatch(Command::Docker(DockerCommand::Select(id)))
            {
                self.service.docker.state.error = Some(error);
                return false;
            }
            self.docker_ui.logs = false;
        }
        self.show_activity = true;
        self.tool_tab = ToolTab::Docker;
        self.focus_editor = false;
        true
    }

    fn docker_request_remove(&mut self, container: &Container) {
        let Some(action) = removal_action(container) else {
            return;
        };
        let Some(context) = self.service.docker.state.context.clone() else {
            return;
        };
        if self.service.docker.is_foreground_busy()
            || !self.service.docker.state.connected
            || !self.docker_select_container(container.id.clone())
        {
            return;
        }
        self.docker_ui.confirm_remove = Some(RemoveTarget {
            id: container.id.clone(),
            name: container.name.clone(),
            context,
            action,
        });
    }

    fn docker_validate_selection(&mut self) {
        if self
            .docker_ui
            .confirm_remove
            .as_ref()
            .is_some_and(|target| {
                (Some(&target.id) != self.service.docker.state.selected.as_ref()
                    && !self.service.docker.is_foreground_busy())
                    || Some(&target.context) != self.service.docker.state.context.as_ref()
                    || !self.service.docker.state.connected
                    || self
                        .service
                        .docker
                        .state
                        .containers
                        .iter()
                        .find(|c| c.id == target.id)
                        .is_none_or(|c| !target.action.can_apply(c))
            })
        {
            self.docker_ui.confirm_remove = None;
        }
    }

    fn docker_cancel_remove(&mut self) {
        if let Some(target) = self.docker_ui.confirm_remove.take()
            && self.docker_ui.open
            && self.show_project
            && Some(&target.context) == self.service.docker.state.context.as_ref()
            && Some(&target.id) == self.service.docker.state.selected.as_ref()
        {
            self.docker_ui.focus_next = Some(target.id);
        }
    }

    fn docker_feedback(&self, ui: &mut egui::Ui) {
        if let Some(pending) = &self.service.docker.state.pending {
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(RichText::new(pending).weak());
            });
        }
        if let Some(error) = &self.service.docker.state.error {
            crate::status_messages::status_message(ui, error);
        }
        if let Some(notice) = &self.service.docker.state.notice {
            crate::status_messages::status_message(ui, notice);
        }
        if self.service.docker.state.truncated {
            ui.label(RichText::new("Saída limitada pelo Docker.").weak());
        }
    }

    pub(super) fn docker_tool_panel(&mut self, ui: &mut egui::Ui) {
        self.tokens.form_style(ui);
        ui.spacing_mut().item_spacing = egui::vec2(4., 4.);
        self.docker_validate_selection();
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.docker_cancel_remove();
        }
        self.docker_feedback(ui);
        let selected = self.service.docker.state.selected.as_ref();
        let container = self
            .service
            .docker
            .state
            .containers
            .iter()
            .find(|c| Some(&c.id) == selected)
            .cloned();
        if let Some(container) = container {
            self.docker_selected(
                ui,
                &container,
                self.service.docker.is_foreground_busy() || !self.service.docker.state.connected,
            );
        } else {
            ui.label("Selecione um container no painel Docker à esquerda.");
        }
    }

    fn docker_selected(&mut self, ui: &mut egui::Ui, container: &Container, busy: bool) {
        egui::ScrollArea::horizontal()
            .id_salt("docker-actions")
            .codisseu()
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.;
                    for (action, icon, label) in [
                        (ContainerAction::Start, Icon::Play, "Iniciar container"),
                        (ContainerAction::Stop, Icon::Stop, "Parar container"),
                        (
                            ContainerAction::Restart,
                            Icon::Refresh,
                            "Reiniciar container",
                        ),
                        (ContainerAction::Pause, Icon::Pause, "Pausar container"),
                        (ContainerAction::Unpause, Icon::Play, "Continuar container"),
                    ] {
                        if action_button(ui, icon, label, !busy && action.can_apply(container))
                            .clicked()
                        {
                            self.docker_ui.confirm_remove = None;
                            self.docker_send(DockerCommand::Action {
                                id: container.id.clone(),
                                action,
                            });
                        }
                    }
                    let shell = action_button(
                        ui,
                        Icon::Terminal,
                        "Abrir shell do container",
                        !busy && container.state == "running",
                    );
                    egui::Popup::menu(&shell).show(|ui| {
                        for name in ["sh", "bash"] {
                            if ui.button(format!("Abrir {name}")).clicked() {
                                if let Some(context) = self.service.docker.state.context.clone() {
                                    self.docker_ui.pending_shell = Some((
                                        container.id.clone(),
                                        container.name.clone(),
                                        name.into(),
                                        context,
                                    ));
                                }
                                ui.close();
                            }
                        }
                    });
                    if action_button(
                        ui,
                        Icon::Delete,
                        "Excluir container",
                        !busy && removal_action(container).is_some(),
                    )
                    .clicked()
                    {
                        self.docker_request_remove(container);
                    }
                    ui.add_space(8.);
                    ui.spacing_mut().item_spacing.x = 4.;
                    ui.spacing_mut().button_padding = egui::vec2(6., 4.);
                    if ui
                        .selectable_label(!self.docker_ui.logs, "Detalhes")
                        .clicked()
                    {
                        self.docker_ui.logs = false;
                    }
                    if ui.selectable_label(self.docker_ui.logs, "Logs").clicked() && !busy {
                        self.docker_ui.logs = true;
                        self.docker_send(DockerCommand::Logs {
                            id: container.id.clone(),
                        });
                    }
                    if self.docker_ui.logs
                        && action_button(ui, Icon::Refresh, "Atualizar logs", !busy).clicked()
                    {
                        self.docker_send(DockerCommand::Logs {
                            id: container.id.clone(),
                        });
                    }
                });
            });
        if let Some(target) = self.docker_ui.confirm_remove.clone() {
            egui::Frame::new()
                .fill(color(self.tokens.navigation_popup_background))
                .stroke(egui::Stroke::new(1., color(self.tokens.popup_border)))
                .corner_radius(self.tokens.control_corner_radius)
                .inner_margin(self.tokens.content_margin())
                .show(ui, |ui| {
                    let title =
                        RichText::new(format!("Excluir o container “{}”?", target.name)).strong();
                    ui.add(egui::Label::new(title).wrap());
                    let identity =
                        format!("Contexto: {} · ID: {}", target.context, &target.id[..12]);
                    ui.add(egui::Label::new(RichText::new(identity).weak()).wrap());
                    ui.add(
                        egui::Label::new(concat!(
                            "Os arquivos gravados somente no container serão perdidos. ",
                            "A imagem e os volumes serão preservados."
                        ))
                        .wrap(),
                    );
                    if target.action == ContainerAction::StopAndRemove {
                        ui.add(
                            egui::Label::new(concat!(
                                "O container será parado antes da exclusão. ",
                                "Se estiver pausado, será retomado para poder parar."
                            ))
                            .wrap(),
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        let label = if target.action == ContainerAction::StopAndRemove {
                            "Parar e excluir"
                        } else {
                            "Confirmar exclusão"
                        };
                        if ui
                            .add_enabled(
                                !busy
                                    && self.service.docker.state.selected.as_ref()
                                        == Some(&target.id),
                                egui::Button::new(
                                    RichText::new(label).color(color(self.tokens.error)),
                                ),
                            )
                            .clicked()
                            && let Some(target) = self.docker_ui.confirm_remove.take()
                            && Some(&target.context) == self.service.docker.state.context.as_ref()
                            && Some(&target.id) == self.service.docker.state.selected.as_ref()
                        {
                            self.docker_send(DockerCommand::Action {
                                id: target.id,
                                action: target.action,
                            });
                        }
                        if ui.button("Cancelar exclusão").clicked() {
                            self.docker_cancel_remove();
                        }
                    });
                });
        }
        egui::ScrollArea::vertical()
            .id_salt(("docker-details", &container.id, self.docker_ui.logs))
            .auto_shrink([false, false])
            .codisseu()
            .show(ui, |ui| {
                if self.docker_ui.logs {
                    let logs = &self.service.docker.state.logs;
                    if logs.is_empty() {
                        ui.label(
                            RichText::new(if busy {
                                "Carregando logs…"
                            } else {
                                "Nenhum log disponível."
                            })
                            .weak(),
                        );
                    } else {
                        ui.add(
                            egui::Label::new(RichText::new(logs).monospace().size(11.))
                                .wrap()
                                .selectable(true),
                        );
                    }
                } else {
                    metadata(ui, "Nome", &container.name);
                    metadata(ui, "Estado", state_label(&container.state));
                    metadata(ui, "Imagem", &container.image);
                    metadata(ui, "ID", &container.id);
                    metadata(ui, "Portas", &container.ports);
                    if let Some(details) = &self.service.docker.state.details
                        && details.id == container.id
                    {
                        metadata(ui, "Criado em", &details.created);
                        metadata(ui, "Diretório de trabalho", &details.working_dir);
                        metadata(ui, "Usuário", &details.user);
                        metadata(ui, "Reinicialização", &details.restart_policy);
                        metadata(ui, "Volumes", &details.mounts.join("\n"));
                        metadata(ui, "Redes", &details.networks.join("\n"));
                    }
                }
            });
    }
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
