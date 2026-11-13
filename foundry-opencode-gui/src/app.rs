use std::path::PathBuf;

use eframe::egui;

use crate::state::{Operation, Outcome, Request, Stack, Status, Step, WizardState};

#[derive(Default)]
pub struct WizardApp {
    pub state: WizardState,
}

impl WizardApp {
    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.heading("Set up Foundry OpenCode");
        ui.label("Choose what to configure. The GUI does not run installer commands yet.");
        ui.radio_value(
            &mut self.state.choices.operation,
            Operation::Install,
            "Install",
        );
        ui.radio_value(
            &mut self.state.choices.operation,
            Operation::Update,
            "Update",
        );
        if self.state.choices.operation == Operation::Update {
            ui.add_enabled(
                self.state.saved.is_some(),
                egui::Checkbox::new(
                    &mut self.state.choices.accept_saved_defaults,
                    "Accept loaded defaults and update",
                ),
            );
            if self.state.saved.is_none() {
                ui.label(
                    "Saved settings are not supplied until the installer backend is connected.",
                );
            }
        }
    }

    fn options(&mut self, ui: &mut egui::Ui) {
        ui.heading("Applications");
        ui.label("OpenCode is required.");
        ui.checkbox(
            &mut self.state.choices.install_openchamber,
            "Install OpenChamber (optional)",
        );
    }

    fn stack_fields(ui: &mut egui::Ui, stack: &mut Stack, show_tokens: bool) {
        ui.label("Name");
        ui.text_edit_singleline(&mut stack.name);
        ui.label("Foundry HTTPS host");
        ui.add(
            egui::TextEdit::singleline(&mut stack.raw_host)
                .hint_text("https://your-stack.palantirfoundry.com")
                .desired_width(f32::INFINITY),
        );
        ui.label("Private token");
        ui.add(
            egui::TextEdit::singleline(&mut stack.token)
                .password(!show_tokens)
                .desired_width(f32::INFINITY),
        );
        if stack.saved_token && stack.token.is_empty() {
            ui.label("Saved credential available");
        }
    }

    fn stacks(&mut self, ui: &mut egui::Ui) {
        ui.heading("Foundry stacks");
        ui.label("Add a named stack or configure the optional global fallback.");
        ui.checkbox(&mut self.state.show_tokens, "Show entered tokens");
        let assigned_global = self
            .state
            .choices
            .repositories
            .iter()
            .any(|repo| repo.stack == Some(0));
        ui.add_enabled(
            !assigned_global || !self.state.choices.use_global,
            egui::Checkbox::new(&mut self.state.choices.use_global, "Use global fallback"),
        );
        if self.state.choices.use_global {
            ui.group(|ui| {
                ui.label("Global default");
                ui.label("Foundry HTTPS host");
                ui.text_edit_singleline(&mut self.state.choices.global.raw_host);
                ui.label("Private token");
                ui.add(
                    egui::TextEdit::singleline(&mut self.state.choices.global.token)
                        .password(!self.state.show_tokens)
                        .desired_width(f32::INFINITY),
                );
                if self.state.choices.global.saved_token
                    && self.state.choices.global.token.is_empty()
                {
                    ui.label("Saved credential available");
                }
                if ui
                    .add_enabled(
                        self.state.backend_connected,
                        egui::Button::new("Open token page"),
                    )
                    .clicked()
                {
                    self.state.request = Some(Request::OpenTokenPage(
                        self.state.choices.global.host().to_string().clone(),
                    ));
                }
            });
        }

        let mut remove = None;
        for stack in &mut self.state.choices.stacks {
            ui.push_id(stack.id, |ui| {
                ui.group(|ui| {
                    Self::stack_fields(ui, stack, self.state.show_tokens);
                    let assigned = self
                        .state
                        .choices
                        .repositories
                        .iter()
                        .any(|repo| repo.stack == Some(stack.id));
                    if ui
                        .add_enabled(!assigned, egui::Button::new("Remove stack"))
                        .clicked()
                    {
                        remove = Some(stack.id);
                    }
                    if ui
                        .add_enabled(
                            self.state.backend_connected,
                            egui::Button::new("Open token page"),
                        )
                        .clicked()
                    {
                        self.state.request = Some(Request::OpenTokenPage(stack.host().to_string().clone()));
                    }
                    if assigned {
                        ui.label("Reassign repositories before removing this stack.");
                    }
                });
            });
        }
        if let Some(id) = remove {
            self.state.remove_stack(id);
        }
        if ui.button("Add stack").clicked() {
            self.state.add_stack();
        }
    }

    fn repositories(&mut self, ui: &mut egui::Ui) {
        ui.heading("Repositories");
        ui.label(
            "Choose project roots. Your backend will check them before changing files.",
        );
        ui.horizontal(|ui| {
            if ui.button("Choose folder…").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                self.add_repository(path);
            }
            ui.add(
                egui::TextEdit::singleline(&mut self.state.path_input)
                    .hint_text("/path/to/repository")
                    .desired_width(420.0),
            );
            if ui.button("Add path").clicked() {
                self.add_repository(PathBuf::from(self.state.path_input.trim()));
            }
        });
        let mut remove = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, repo) in self.state.choices.repositories.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        remove = Some(index);
                    }
                    ui.monospace(repo.path.display().to_string());
                });
            }
        });
        if let Some(index) = remove {
            self.state.choices.repositories.remove(index);
        }
    }

    fn add_repository(&mut self, path: PathBuf) {
        match self.state.add_repository(path) {
            Ok(()) => {
                self.state.path_input.clear();
                self.state.error = None;
            }
            Err(message) => self.state.error = Some(message.into()),
        }
    }

    fn assignments(&mut self, ui: &mut egui::Ui) {
        ui.heading("Assignments and RAG");
        let mut choices = Vec::new();
        if self.state.choices.use_global {
            choices.push((0, self.state.choices.global.name.clone()));
        }
        choices.extend(
            self.state
                .choices
                .stacks
                .iter()
                .map(|stack| (stack.id, stack.name.clone())),
        );
        egui::ScrollArea::vertical().show(ui, |ui| {
            for repo in &mut self.state.choices.repositories {
                ui.push_id(&repo.path, |ui| {
                    ui.group(|ui| {
                        ui.monospace(repo.path.display().to_string());
                        egui::ComboBox::from_id_salt("stack")
                            .selected_text(
                                choices
                                    .iter()
                                    .find(|(id, _)| repo.stack == Some(*id))
                                    .map_or("Select a stack", |(_, name)| name),
                            )
                            .show_ui(ui, |ui| {
                                for (id, name) in &choices {
                                    ui.selectable_value(&mut repo.stack, Some(*id), name);
                                }
                            });
                        ui.checkbox(&mut repo.rag, "Initialize RAG");
                    });
                });
            }
        });
    }

    fn review(&mut self, ui: &mut egui::Ui) {
        ui.heading("Review your choices");
        ui.label("Tokens are hidden. The installer backend must verify paths, credentials and backups before running.");
        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in self.state.review_lines() {
                ui.label(line);
            }
            if !self.state.backup_preview.is_empty() {
                ui.separator();
                ui.label("Backups proposed by the backend:");
                for path in &self.state.backup_preview {
                    ui.monospace(self.state.redact(&path.display().to_string()));
                }
            }
        });
        if !self.state.backend_connected {
            ui.label("Installer backend not connected. Install/Update is disabled.");
        }
    }

    fn progress(&mut self, ui: &mut egui::Ui) {
        if let Status::Running {
            label,
            done,
            total,
            repositories,
            logs,
        } = &self.state.status
        {
            ui.heading(self.state.redact(label));
            if let Some(total) = total {
                ui.add(
                    egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                        .text(format!("{done}/{total}")),
                );
            } else {
                ui.spinner();
            }
            for (path, status) in repositories {
                ui.label(self.state.redact(&format!("{}: {status}", path.display())));
            }
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in logs {
                        ui.monospace(self.state.redact(line));
                    }
                });
        } else {
            ui.label("Waiting for installer progress…");
        }
    }

    fn result(&mut self, ui: &mut egui::Ui) {
        if let Status::Finished {
            outcome,
            message,
            repositories,
            backups,
        } = &self.state.status
        {
            ui.heading(match outcome {
                Outcome::Success => "Installation complete",
                Outcome::PartialFailure => "Partially complete",
                Outcome::Failure => "Installation failed",
                Outcome::Cancelled => "Cancelled",
            });
            ui.label(self.state.redact(message));
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (path, status) in repositories {
                    ui.label(self.state.redact(&format!("{}: {status}", path.display())));
                }
                if !backups.is_empty() {
                    ui.label("Backups:");
                }
                for path in backups {
                    ui.monospace(self.state.redact(&path.display().to_string()));
                }
            });
        } else {
            ui.label("No result has been supplied by the installer backend.");
        }
    }

    fn footer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            match self.state.step {
                Step::Welcome => {
                    if ui.button("Cancel").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Step::Progress => {
                    if ui
                        .add_enabled(
                            self.state.backend_connected && self.state.request.is_none(),
                            egui::Button::new("Cancel operation"),
                        )
                        .clicked()
                    {
                        self.state.request = Some(Request::Cancel);
                    }
                }
                Step::Result => {
                    if ui.button("Back to settings").clicked() {
                        self.state.step = Step::Review;
                    }
                    if ui
                        .add_enabled(
                            self.state.backend_connected
                                && matches!(
                                    self.state.status,
                                    Status::Finished {
                                        outcome: Outcome::Failure | Outcome::PartialFailure,
                                        ..
                                    }
                                ),
                            egui::Button::new("Retry"),
                        )
                        .clicked()
                    {
                        self.state.request = Some(Request::Retry);
                    }
                }
                _ => {
                    if ui.button("Back").clicked() {
                        self.state.back();
                    }
                }
            }
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match self.state.step {
                    Step::Welcome
                    | Step::Options
                    | Step::Stacks
                    | Step::Repositories
                    | Step::Assignments => {
                        let valid = self.state.validation_error().is_none();
                        if ui
                            .add_enabled(valid, egui::Button::new("Continue"))
                            .clicked()
                        {
                            self.state.advance();
                        }
                    }
                    Step::Review => {
                        let label = if self.state.choices.operation == Operation::Install {
                            "Install"
                        } else {
                            "Update"
                        };
                        if ui
                            .add_enabled(
                                self.state.backend_connected && self.state.request.is_none(),
                                egui::Button::new(label),
                            )
                            .clicked()
                        {
                            self.state.request = Some(Request::Start(self.state.choices.clone()));
                            self.state.step = Step::Progress;
                        }
                    }
                    Step::Result => {
                        if ui.button("Close").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    Step::Progress => {}
                },
            );
        });
    }
}

impl eframe::App for WizardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if ui.ctx().input(|input| input.key_pressed(egui::Key::Escape))
            && self.state.step != Step::Welcome
        {
            self.state.back();
        }
        egui::Panel::bottom("wizard_footer").show(ui, |ui| self.footer(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(self.state.step.title());
            ui.separator();
            if let Some(error) = &self.state.error {
                ui.colored_label(egui::Color32::DARK_RED, self.state.redact(error));
            }
            if let Some(error) = self.state.validation_error() {
                ui.colored_label(egui::Color32::DARK_RED, error);
            }
            match self.state.step {
                Step::Welcome => self.welcome(ui),
                Step::Options => self.options(ui),
                Step::Stacks => {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| self.stacks(ui))
                        .inner
                }
                Step::Repositories => self.repositories(ui),
                Step::Assignments => self.assignments(ui),
                Step::Review => self.review(ui),
                Step::Progress => self.progress(ui),
                Step::Result => self.result(ui),
            }
        });
    }
}
