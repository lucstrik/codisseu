//! One-line operational feedback shared by every tool, with bounded history.
use super::*;
use std::collections::BTreeMap;
#[derive(Clone, Default)]
struct Messages {
    seen: BTreeMap<String, u64>,
    active: Option<(String, f64)>,
}
fn key() -> egui::Id {
    egui::Id::unique("codisseu.status.messages")
}
/// Presentation only: the original diagnostic remains available in the tooltip.
pub(super) fn friendly(raw: &str) -> String {
    if crate::git_scm::expected_missing_repository(raw) {
        return "Projeto sem repositório Git".into();
    }
    let text = raw.replace(['\n', '\r'], " ");
    let lower = text.to_ascii_lowercase();
    if (lower.starts_with("fatal:") || lower.starts_with("error:"))
        && (lower.contains("permission denied") || lower.contains("access denied"))
    {
        return "Sem permissão para concluir a operação".into();
    }
    if lower.starts_with("fatal:") {
        return "Não foi possível concluir a operação".into();
    }
    text
}
pub(super) fn publish(ctx: &egui::Context, text: impl ToString) {
    let text = text.to_string();
    if text.is_empty() {
        return;
    }
    let frame = ctx.cumulative_frame_nr();
    let now = ctx.input(|i| i.time);
    let changed = ctx.data_mut(|data| {
        let messages = data.get_temp_mut_or_default::<Messages>(key());
        let changed = messages
            .seen
            .get(&text)
            .is_none_or(|last| frame.saturating_sub(*last) > 2);
        if changed {
            messages.active = Some((text.clone(), now));
        }
        messages.seen.insert(text, frame);
        if messages.seen.len() > 128 {
            messages
                .seen
                .retain(|_, last| frame.saturating_sub(*last) <= 2);
        }
        changed
    });
    if changed {
        ctx.request_repaint();
    }
}
pub(super) fn status_message(ui: &egui::Ui, text: impl ToString) -> egui::Response {
    publish(ui.ctx(), text);
    // A response preserves call-site hover/accessibility chaining without placing
    // another copy of the message in a tool, popup or editor overlay.
    ui.interact(
        egui::Rect::NOTHING,
        ui.scope_id().with("status.feedback"),
        egui::Sense::hover(),
    )
}
pub(super) fn current(ctx: &egui::Context) -> Option<String> {
    let now = ctx.input(|i| i.time);
    ctx.data(|data| {
        data.get_temp::<Messages>(key())
            .and_then(|m| m.active)
            .filter(|(_, time)| now - time < 30.)
            .map(|(text, _)| text)
    })
}
impl CodisseuApp {
    pub(super) fn collect_status_messages(&self, ctx: &egui::Context) {
        // A newly completed operation may replace an old, continuously present
        // diagnostic. Re-publishing the same diagnostic does not renew its age.
        publish(ctx, &self.service.status);
        for error in [
            self.service.last_error.as_deref(),
            self.service.tree_error(),
            self.service.db.last_error.as_deref(),
            self.service.remote_db.error.as_deref(),
            self.service.language.last_error.as_deref(),
            self.service.git.error.as_deref(),
            self.service.project_files.error.as_deref(),
            self.service.api.error.as_deref(),
            self.service.debugger.error.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            publish(ctx, error);
        }
    }
    pub(super) fn status_message_text(&self, ctx: &egui::Context) -> String {
        current(ctx)
            .unwrap_or_else(|| self.service.status.clone())
            .replace(['\n', '\r'], " ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expected_absence_is_friendly_but_permission_and_timeout_are_not_absence() {
        assert_eq!(
            friendly("fatal: not a git repository (or any of the parent directories): .git"),
            "Projeto sem repositório Git"
        );
        assert_eq!(
            friendly("fatal: permission denied\nnot a git repository"),
            "Sem permissão para concluir a operação"
        );
        assert_eq!(
            friendly("fatal: timeout\nnot a git repository"),
            "Não foi possível concluir a operação"
        );
        assert_eq!(friendly("Arquivo salvo"), "Arquivo salvo");
        assert_eq!(
            friendly("Erro SQL: coluna ausente"),
            "Erro SQL: coluna ausente"
        );
    }
}
