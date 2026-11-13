mod app;
mod backend;
mod privilege;
pub mod state;

use eframe::egui;

fn main() -> eframe::Result {
    // Finder does not inherit the user's shell PATH; include both supported Homebrew prefixes.
    let mut paths = ["/opt/homebrew/bin", "/usr/local/bin"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    paths.extend(
        ["/usr/bin", "/bin"]
            .into_iter()
            .map(std::path::PathBuf::from),
    );
    if let Ok(path) = std::env::join_paths(paths) {
        // Called before eframe starts worker threads.
        unsafe { std::env::set_var("PATH", path) };
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 700.0])
            .with_min_inner_size([680.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Foundry OpenCode Installer",
        options,
        Box::new(|_cc| Ok(Box::<app::WizardApp>::default())),
    )
}
