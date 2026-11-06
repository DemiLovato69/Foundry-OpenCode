#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![expect(rustdoc::missing_crate_level_docs)] // it's an example

use std::process::exit;

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Foundry Opencode Installer",
        options,
        Box::new(|cc| {
            // This gives us image support:
            egui_extras::install_image_loaders(&cc.egui_ctx);

            Ok(Box::<FoundryOpencodeInstaller>::default())
        }),
    )
}

struct InstallationSettings {
    install_openchamber: bool,
}

impl Default for InstallationSettings {
    fn default() -> Self {
        Self {
            install_openchamber: false,
        }
    }
}

struct FoundryOpencodeInstaller {
    installation_settings: InstallationSettings,
    page: u32,
    page_title: String,
    input_text: String,
}

impl Default for FoundryOpencodeInstaller {
    fn default() -> Self {
        Self {
            installation_settings: InstallationSettings::default(),
            page: 0,
            page_title: "Foundry Opencode Installer".into(),
            input_text: String::new(),
        }
    }
}

trait Pages {
    fn page_0(&mut self, ui: &mut egui::Ui);
    fn page_1(&mut self, ui: &mut egui::Ui);
}

impl Pages for FoundryOpencodeInstaller {
    fn page_0(&mut self, ui: &mut egui::Ui) {
        self.page_title = "Foundry Opencode Installer - Page 0".into();
        ui.label("Do you want to install OpenChamber?");
        ui.with_layout(
            egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
            |ui| {
                ui.checkbox(
                    &mut self.installation_settings.install_openchamber,
                    "Install OpenChamber",
                );
            },
        );
    }

    fn page_1(&mut self, ui: &mut egui::Ui) {
        self.page_title = "Foundry Opencode Installer - Page 1".into();
        ui.label("What is the airspeed velocity of an unladen swallow?");

        // 1. Set the exact bounding width/height you want your input field to occupy.
        // We give it a comfortable width (e.g., 300.0) and a single-line height (e.g., 24.0).
        let input_size = egui::vec2(300.0, 24.0);

        // 2. Bound the available UI space so 'centered_and_justified' doesn't blow up full-screen.
        ui.allocate_ui(input_size, |ui| {
            ui.with_layout(
                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                |ui| {
                    ui.text_edit_singleline(&mut self.input_text);
                },
            );
        });

        // 3. This safely consumes whatever is left at the bottom of the page
        ui.allocate_space(egui::vec2(0.0, ui.available_height()));

        // self.page_title = "Foundry Opencode Installer - Page 1".into();
        // ui.label("What is the airspeed velocity of an unladen swallow?");
        // ui.with_layout(
        //     egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        //     |ui| {
        //         ui.text_edit_singleline(&mut self.input_text);
        //     },
        // );
        // ui.allocate_space(egui::vec2(0.0, ui.available_height()));
    }
}

impl eframe::App for FoundryOpencodeInstaller {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::bottom("bottom")
            .resizable(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut back_button_text = "Back";
                    if self.page == 0 {
                        back_button_text = "Exit"
                    }
                    if ui.button(back_button_text).clicked() {
                        if self.page > 0 {
                            self.page -= 1;
                        } else {
                            std::thread::spawn(|| {
                                let confirmed = rfd::MessageDialog::new()
                                    .set_title("Are you sure you want to exit?")
                                    .set_description("All unsaved changes will be lost.")
                                    .set_buttons(rfd::MessageButtons::YesNo)
                                    .set_level(rfd::MessageLevel::Warning)
                                    .show();

                                if confirmed == rfd::MessageDialogResult::Yes {
                                    // Proceed with deletion logic
                                    exit(0);
                                }
                            });
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Next").clicked() {
                            // self.age += 1;
                            self.page += 1;
                        }
                    });
                });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            // Double scale
            ui.ctx().set_pixels_per_point(3.0);

            ui.heading(&self.page_title);

            match self.page {
                0 => self.page_0(ui),
                1 => self.page_1(ui),
                _ => {
                    std::thread::spawn(|| {
                        let confirmed = rfd::MessageDialog::new()
                            .set_title("Success")
                            .set_description("Operation completed successfully.")
                            .set_buttons(rfd::MessageButtons::Ok)
                            .set_level(rfd::MessageLevel::Warning)
                            .show();

                        if confirmed == rfd::MessageDialogResult::Ok {
                            exit(0);
                        }
                    });
                }
            }
        });
    }
}
