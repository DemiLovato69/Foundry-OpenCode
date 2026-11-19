mod app;
mod backend;
mod privilege;
pub mod state;

use eframe::egui;

fn main() -> eframe::Result {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--version")
    {
        println!(
            "{}",
            option_env!("FOUNDRY_OPENCODE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
        );
        return Ok(());
    }
    // Finder does not inherit the user's shell PATH; keep the terminal's selection for Brew installs.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"]
        .into_iter()
        .map(std::path::PathBuf::from)
        .collect::<Vec<_>>();
    let inherited = std::env::split_paths(&path).filter(|entry| !entry.as_os_str().is_empty());
    let priority = if foundry_opencode::ops::HOMEBREW {
        0
    } else {
        2
    };
    paths.splice(priority..priority, inherited);
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
