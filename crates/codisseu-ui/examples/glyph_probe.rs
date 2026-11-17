//! Native experiment: the actual application, public egui font tweaks only.
//! Not a product backend or an acceptance test of the final executable.
use codisseu_core::{AppService, Command, Selection};
use codisseu_ui::CodisseuApp;
use eframe::egui;
use std::path::PathBuf;

struct Probe {
    app: CodisseuApp,
    mode: u8,
    offset: f32,
    configured: bool,
    selected: bool,
}
impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if !self.selected
            && let Some(d) = self.app.service.active_document()
        {
            let id = d.id;
            let len = d.snapshot().len_chars();
            let version = d.version;
            self.app
                .service
                .dispatch(Command::SetSelection {
                    id,
                    expected_version: version,
                    selection: Selection::caret(len),
                })
                .unwrap();
            self.selected = true;
        }
        self.app.ui(ui, frame);
        if !self.configured {
            let mut fonts = ctx.fonts(|f| f.definitions().clone());
            for name in [
                "codisseu.mono",
                "codisseu.mono.bold",
                "codisseu.mono.italic",
            ] {
                let tweak =
                    &mut std::sync::Arc::make_mut(fonts.font_data.get_mut(name).unwrap()).tweak;
                tweak.y_offset = self.offset;
                if self.mode == 8 {
                    tweak.hinting = Some(false);
                } else {
                    tweak.hinting = Some(true);
                    tweak.hinting_target =
                        epaint::text::HintingTarget::Smooth(epaint::text::SmoothHinting {
                            light: self.mode & 1 != 0,
                            symmetric_rendering: self.mode & 2 != 0,
                            preserve_linear_metrics: self.mode & 4 != 0,
                        });
                }
            }
            ctx.set_fonts(fonts);
            ctx.request_repaint();
            self.configured = true;
        }
    }
}
fn main() -> eframe::Result {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        6,
        "glyph_probe WORKSPACE FILE OUTPUT MODE Y_OFFSET"
    );
    let workspace = PathBuf::from(&args[1]);
    let mut service = AppService::new(PathBuf::from(&args[3]).with_extension("session.json"));
    service.dispatch(Command::OpenWorkspace(workspace)).unwrap();
    service
        .dispatch(Command::OpenFile(PathBuf::from(&args[2])))
        .unwrap();
    let mut app = CodisseuApp::new(service);
    app.capture_to(PathBuf::from(&args[3]), true);
    let probe = Probe {
        app,
        mode: args[4].parse().unwrap(),
        offset: args[5].parse().unwrap(),
        configured: false,
        selected: false,
    };
    eframe::run_native(
        "Codisseu glyph laboratory",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([1280., 800.]),
            renderer: eframe::Renderer::Wgpu,
            dithering: false,
            ..Default::default()
        },
        Box::new(move |cc| {
            eprintln!(
                "probe adapter: {:?}",
                cc.wgpu_render_state.as_ref().map(|s| s.adapter.get_info())
            );
            Ok(Box::new(probe))
        }),
    )
}
