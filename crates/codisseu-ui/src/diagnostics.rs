//! Metadata-only lifecycle diagnostics. The desktop sink must be nonblocking.
use std::sync::Arc;

type Sink = Arc<dyn Fn(&str) + Send + Sync>;

pub fn set_diagnostic_sink(ctx: &egui::Context, sink: impl Fn(&str) + Send + Sync + 'static) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::unique("codisseu.diagnostic_sink"),
            Arc::new(sink) as Sink,
        )
    });
}
pub(super) fn record(ctx: &egui::Context, message: &str) {
    let sink = ctx.data(|data| data.get_temp::<Sink>(egui::Id::unique("codisseu.diagnostic_sink")));
    if let Some(sink) = sink {
        sink(message);
    }
    eprintln!("Codisseu lifecycle: {message}");
}
