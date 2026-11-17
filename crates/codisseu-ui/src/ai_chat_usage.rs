//! A compact, monochrome footer; details disclose what each provider number means.
use super::*;

impl CodisseuApp {
    pub(super) fn ai_usage_footer(&mut self, ui: &mut egui::Ui, busy: bool) {
        let now = ai::usage::now();
        let status = if self.service.ai.is_deleting() {
            "Apagando conversa…".into()
        } else if busy {
            format!(
                "{} · {}s",
                self.service.ai.state.label(),
                self.service.ai.elapsed_seconds
            )
        } else {
            match self.ai_chat.mode {
                Mode::Agent => "Agente · projeto aberto",
                Mode::Consult => "Somente leitura · projeto aberto",
                Mode::FullAccess => "Full access · projeto aberto",
            }
            .into()
        };
        let tokens = self.service.ai.usage.as_ref();
        let remaining = self
            .service
            .ai
            .limits
            .as_ref()
            .and_then(|l| l.remaining(now));
        let caption = usage_caption(tokens.map(|t| t.total.total_tokens), remaining);
        let details = usage_details(&self.service.ai, now);
        ui.horizontal(|ui| {
            let font = egui::TextStyle::Small.resolve(ui.style());
            let size = ui.fonts_mut(|f| {
                f.layout_no_wrap(caption.clone(), font.clone(), color(self.tokens.muted))
                    .size()
            });
            let controls_gap = ui.spacing().item_spacing.x + ui.spacing().button_padding.x * 2.;
            let width = (ui.available_width() - size.x - controls_gap).max(60.);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 20.),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(width, 20.));
                    ui.add(
                        egui::Label::new(
                            RichText::new(&status)
                                .small()
                                .color(color(self.tokens.muted)),
                        )
                        .truncate()
                        .halign(egui::Align::Min),
                    )
                },
            )
            .inner
            .on_hover_text(&status);
            let response = ui
                .add(
                    egui::Button::new(
                        RichText::new(caption)
                            .font(font)
                            .color(color(self.tokens.muted)),
                    )
                    .frame(false),
                )
                .on_hover_text(&details);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::accesskit::Role::Button,
                    true,
                    "Uso de tokens e limites",
                )
            });
            self.ai_open_control(&response, "Uso de tokens e limites");
            egui::Popup::menu(&response).show(|ui| {
                ui.set_max_width(340.);
                ui.label(RichText::new("Uso da conversa").strong());
                ui.add(egui::Label::new(&details).wrap());
                if self.ai_chat.provider == Provider::Codex
                    && ui
                        .add_enabled(!busy, egui::Button::new("Atualizar limites"))
                        .clicked()
                {
                    self.ai_start(Action::Usage);
                    ui.close();
                }
            });
        });
        if self.service.ai.limits.is_some() {
            ui.ctx().request_repaint_after(Duration::from_secs(30));
        }
    }
}
fn compact(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.).replace('.', ",")
    } else if value >= 1000 {
        format!("{:.1}k", value as f64 / 1000.).replace('.', ",")
    } else {
        value.to_string()
    }
}
fn usage_caption(tokens: Option<u64>, remaining: Option<f64>) -> String {
    match (tokens, remaining) {
        (Some(t), Some(p)) => format!("{} tokens · cota {p:.0}% livre", compact(t)),
        (Some(t), None) => format!("{} tokens", compact(t)),
        (None, Some(p)) => format!("Cota: {p:.0}% livre"),
        (None, None) => "Tokens: —".into(),
    }
}
fn usage_details(chat: &ai::ChatService, now: u64) -> String {
    let mut text = String::new();
    if let Some(t) = &chat.usage {
        text.push_str(&format!("Tokens da conversa: {}\nEntrada: {} · em cache: {}\nSaída: {} · raciocínio: {}\nÚltima chamada: {} tokens\n",t.total.total_tokens,t.total.input_tokens,t.total.cached_input_tokens,t.total.output_tokens,t.total.reasoning_output_tokens,t.last.total_tokens));
        if let (Some(capacity), Some(remaining)) = (t.model_context_window, t.context_remaining()) {
            text.push_str(&format!(
                "Contexto: ~{} de {} tokens livres, baseado na última chamada.\n",
                compact(remaining),
                compact(capacity)
            ));
        } else {
            text.push_str("Capacidade de contexto não informada pelo CLI.\n");
        }
    } else {
        text.push_str("Tokens ainda não informados pelo CLI.\n");
    }
    if let Some(limits) = &chat.limits {
        text.push_str("\nCota da conta compartilhada (não é uma quantidade de tokens):\n");
        for b in &limits.buckets {
            text.push_str(&format!("{}\n", b.name));
            for (label, w) in [("principal", &b.primary), ("secundária", &b.secondary)] {
                if let Some(w) = w {
                    text.push_str(&format!(
                        "• {label}: {:.0}% usado / {:.0}% livre",
                        w.used_percent,
                        (100. - w.used_percent).clamp(0., 100.)
                    ));
                    if let Some(mins) = w.duration_mins {
                        text.push_str(&format!(" · janela {}", duration(mins.saturating_mul(60))));
                    }
                    if let Some(reset) = w.resets_at {
                        if reset > now {
                            text.push_str(&format!(" · renova em {}", duration(reset - now)));
                        } else {
                            text.push_str(" · renovação pendente de atualização");
                        }
                    }
                    text.push('\n');
                }
            }
        }
        text.push_str(&format!(
            "Atualizado há {}.\n",
            duration(now.saturating_sub(limits.observed_at))
        ));
        if now.saturating_sub(limits.observed_at) > 300 {
            text.push_str("Dados antigos; atualize os limites.\n");
        }
    } else {
        text.push_str("\nCota da conta: —\n");
    }
    if let Some(error) = &chat.limits_error {
        text.push_str(error);
        text.push('\n');
    }
    text.push_str("Tokens usados, contexto e cota são medidas diferentes. Valores não representam custo em dinheiro.");
    text
}
fn duration(seconds: u64) -> String {
    let mins = seconds / 60;
    if mins >= 1440 {
        format!("{}d {}h", mins / 1440, mins % 1440 / 60)
    } else if mins >= 60 {
        format!("{}h {}min", mins / 60, mins % 60)
    } else if mins > 0 {
        format!("{mins}min")
    } else {
        format!("{seconds}s")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn footer_distinguishes_unavailable_and_zero_and_keeps_precision_in_details() {
        assert_eq!(usage_caption(None, None), "Tokens: —");
        assert_eq!(usage_caption(Some(0), Some(0.)), "0 tokens · cota 0% livre");
        assert_eq!(
            usage_caption(Some(8421), Some(72.)),
            "8,4k tokens · cota 72% livre"
        );
        assert!(
            usage_details(&ai::ChatService::default(), 0).contains("Tokens ainda não informados")
        );
        assert_eq!(duration(18120), "5h 2min");
    }
}
