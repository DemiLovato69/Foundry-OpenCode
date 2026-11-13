mod app;
pub mod state;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 700.0])
            .with_min_inner_size([680.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Foundry OpenCode Installer",
        options,
        Box::new(|_cc| {
            // cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::<app::WizardApp>::default())
        }),
    )
}
