use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

use foundry_opencode::{cleanup, installer, projects};

use crate::privilege::Privilege;
use crate::state::{Choices, Operation, Outcome, Repository, Request, Stack, Status, WizardState};

enum Event {
    Line(String),
    Finished(Result<Vec<String>, String>),
}

pub struct Backend {
    home: PathBuf,
    saved: Option<projects::Catalog>,
    ids: BTreeMap<u64, String>,
    receiver: Option<Receiver<Event>>,
    cancelled: Arc<AtomicBool>,
    checkpoint: Arc<Mutex<installer::Checkpoint>>,
    inspection: Option<(bool, cleanup::Inventory)>,
    alias_preview: Option<(bool, Option<String>)>,
}

impl Backend {
    pub fn load() -> (Self, Option<Choices>, Option<String>) {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let (catalog, error) = if home.as_os_str().is_empty() {
            (
                None,
                Some("HOME is not set; installation is unavailable".into()),
            )
        } else {
            match projects::Catalog::load(&home) {
                Ok(catalog) => (Some(catalog), None),
                Err(error) => (
                    None,
                    Some(format!("Could not load saved settings: {error}")),
                ),
            }
        };
        let mut ids = BTreeMap::new();
        let saved = catalog
            .as_ref()
            .filter(|catalog| {
                catalog.default.is_some()
                    || !catalog.stacks.is_empty()
                    || !catalog.projects.is_empty()
            })
            .map(|catalog| {
                let mut choices = Choices {
                    operation: Operation::Update,
                    sync_alias: catalog.sync_alias,
                    ..Choices::default()
                };
                if let Some(stack) = &catalog.default {
                    ids.insert(0, stack.id.clone());
                    choices.use_global = true;
                    choices.global.raw_host = stack.url.clone();
                    choices.global.saved_token = catalog
                        .tokens
                        .get(&stack.id)
                        .is_some_and(|token| !token.is_empty());
                }
                for (index, stack) in catalog.stacks.iter().enumerate() {
                    let id = index as u64 + 1;
                    ids.insert(id, stack.id.clone());
                    choices.stacks.push(Stack {
                        id,
                        name: stack.name.clone(),
                        raw_host: stack.url.clone(),
                        token: String::new(),
                        saved_token: catalog
                            .tokens
                            .get(&stack.id)
                            .is_some_and(|token| !token.is_empty()),
                    });
                }
                for project in &catalog.projects {
                    choices.repositories.push(Repository {
                        path: project.path.clone(),
                        stack: ids
                            .iter()
                            .find(|(_, id)| **id == project.stack)
                            .map(|(key, _)| *key),
                        rag: project.rag,
                    });
                }
                choices
            });
        (
            Self {
                home,
                saved: catalog,
                ids,
                receiver: None,
                cancelled: Arc::new(AtomicBool::new(false)),
                checkpoint: Arc::new(Mutex::new(installer::Checkpoint::default())),
                inspection: None,
                alias_preview: None,
            },
            saved,
            error,
        )
    }

    fn catalog(&self, choices: &Choices) -> Result<projects::Catalog, String> {
        let mut catalog = projects::Catalog {
            sync_alias: choices.sync_alias,
            ..projects::Catalog::default()
        };
        let mut ids = BTreeMap::new();
        for stack in choices
            .use_global
            .then_some(&choices.global)
            .into_iter()
            .chain(&choices.stacks)
        {
            let id = self.ids.get(&stack.id).cloned().unwrap_or_else(|| {
                if stack.id == 0 {
                    "default".into()
                } else {
                    format!("stack-{}", stack.id)
                }
            });
            let token = if !stack.token.is_empty() {
                stack.token.clone()
            } else {
                self.saved
                    .as_ref()
                    .and_then(|saved| saved.tokens.get(&id))
                    .cloned()
                    .unwrap_or_default()
            };
            if token.is_empty() {
                return Err(format!("Missing private token for {}", stack.name));
            }
            catalog.tokens.insert(id.clone(), token);
            ids.insert(stack.id, id.clone());
            let entry = projects::Stack {
                id,
                name: stack.name.clone(),
                url: stack.host(),
            };
            if stack.id == 0 {
                catalog.default = Some(entry);
            } else {
                catalog.stacks.push(entry);
            }
        }
        for repo in &choices.repositories {
            catalog.projects.push(projects::Project {
                path: repo.path.clone(),
                stack: ids
                    .get(
                        &repo
                            .stack
                            .ok_or_else(|| format!("Missing stack for {}", repo.path.display()))?,
                    )
                    .cloned()
                    .ok_or_else(|| format!("Unknown stack for {}", repo.path.display()))?,
                rag: repo.rag,
            });
        }
        Ok(catalog)
    }

    fn cli_source() -> Result<PathBuf, String> {
        let current = std::env::current_exe().map_err(|error| error.to_string())?;
        let bundle = current
            .parent()
            .and_then(|parent| parent.parent())
            .map(|parent| parent.join("Resources/foundry-opencode"));
        if let Some(bundle) = bundle
            && bundle.is_file()
        {
            return Ok(bundle);
        }
        #[cfg(debug_assertions)]
        {
            let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../foundry-opencode/target/debug/foundry-opencode");
            if dev.is_file() {
                return Ok(dev);
            }
        }
        Err(
            "The matching CLI binary is not bundled; build both executables before installing"
                .into(),
        )
    }

    pub fn running(&self) -> bool {
        self.receiver.is_some()
    }

    pub fn inspection(&mut self, openchamber: bool) -> Result<&cleanup::Inventory, String> {
        if self
            .inspection
            .as_ref()
            .is_none_or(|(choice, _)| *choice != openchamber)
        {
            self.inspection = Some((
                openchamber,
                cleanup::inspect(&self.home, openchamber).map_err(|error| error.to_string())?,
            ));
        }
        Ok(&self.inspection.as_ref().unwrap().1)
    }

    pub fn refresh_review(&mut self) {
        self.inspection = None;
        self.alias_preview = None;
    }

    pub fn alias_preview(&mut self, install: bool) -> Result<Option<&str>, String> {
        if self
            .alias_preview
            .as_ref()
            .is_none_or(|(choice, _)| *choice != install)
        {
            self.alias_preview = Some((
                install,
                foundry_opencode::worktrees::alias_plan(install, None)
                    .map_err(|error| error.to_string())?,
            ));
        }
        Ok(self.alias_preview.as_ref().unwrap().1.as_deref())
    }

    pub fn ready(&self) -> bool {
        if self.home.as_os_str().is_empty() {
            return false;
        }
        Self::cli_source()
            .ok()
            .and_then(|path| {
                let capability = std::process::Command::new(&path)
                    .arg("capabilities")
                    .output()
                    .ok()?;
                let version = std::process::Command::new(&path)
                    .arg("version")
                    .output()
                    .ok()?;
                Some((capability, version))
            })
            .is_some_and(|(capability, version)| {
                capability.status.success()
                    && capability.stdout == b"foundry-opencode-headless-v1\n"
                    && version.status.success()
                    && String::from_utf8_lossy(&version.stdout).trim()
                        == option_env!("FOUNDRY_OPENCODE_VERSION")
                            .unwrap_or(env!("CARGO_PKG_VERSION"))
            })
    }

    pub fn handle(&mut self, request: Request, state: &mut WizardState) {
        match request {
            Request::Cancel => self.cancelled.store(true, Ordering::Relaxed),
            Request::OpenTokenPage(host) => {
                if !foundry_opencode::ops::valid_url(&host) {
                    state.error = Some(
                        "Enter a valid HTTPS Foundry host before opening its token page".into(),
                    );
                    return;
                }
                if let Err(error) = std::process::Command::new("open")
                    .arg(format!("{host}/workspace/settings/tokens"))
                    .status()
                {
                    state.error = Some(format!("Could not open token page: {error}"));
                }
            }
            Request::Retry => self.start(state.choices.clone(), state, true),
            Request::Start(choices) => self.start(choices, state, false),
        }
    }

    fn start(&mut self, choices: Choices, state: &mut WizardState, retry: bool) {
        if !self.ready() {
            state.apply_status(Status::Finished {
                outcome: Outcome::Failure,
                message: "A matching CLI payload and HOME directory are required".into(),
                repositories: vec![],
                backups: vec![],
            });
            return;
        }
        let reviewed = match self.inspection(choices.install_openchamber) {
            Ok(inventory) => inventory.clone(),
            Err(error) => {
                state.apply_status(Status::Finished {
                    outcome: Outcome::Failure,
                    message: error,
                    repositories: vec![],
                    backups: vec![],
                });
                return;
            }
        };
        if self.running() {
            return;
        }
        if !retry {
            *self.checkpoint.lock().unwrap() = installer::Checkpoint::default();
        }
        let catalog = match self.catalog(&choices) {
            Ok(catalog) => catalog,
            Err(error) => {
                state.apply_status(Status::Finished {
                    outcome: Outcome::Failure,
                    message: error,
                    repositories: vec![],
                    backups: vec![],
                });
                return;
            }
        };
        let cli = match Self::cli_source() {
            Ok(path) => path,
            Err(error) => {
                state.apply_status(Status::Finished {
                    outcome: Outcome::Failure,
                    message: error,
                    repositories: vec![],
                    backups: vec![],
                });
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.cancelled.store(false, Ordering::Relaxed);
        let cancel = self.cancelled.clone();
        let checkpoint = self.checkpoint.clone();
        let home = self.home.clone();
        let openchamber = choices.install_openchamber;
        if !retry {
            state.backup_preview.clear();
        }
        state.apply_status(Status::Running {
            label: "Checking administrator authorization".into(),
            done: 0,
            total: None,
            repositories: vec![],
            logs: vec![],
        });
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<String>, String> {
                let plan = installer::Plan {
                    home,
                    catalog,
                    openchamber,
                    cli,
                    dry_run: false,
                    reviewed,
                };
                installer::preflight(&plan).map_err(|error| error.to_string())?;
                let auth = Privilege::authorize()?;
                installer::apply(
                    &plan,
                    &mut checkpoint.lock().unwrap(),
                    |line| {
                        let _ = tx.send(Event::Line(line));
                    },
                    || cancel.load(Ordering::Relaxed),
                    |cli| {
                        auth.install_cli(cli)
                            .map_err(|error| color_eyre::eyre::eyre!("{error}"))
                    },
                )
                .map_err(|error| error.to_string())
            })();
            let _ = tx.send(Event::Finished(result));
        });
    }

    pub fn poll(&mut self, state: &mut WizardState) {
        let Some(receiver) = &self.receiver else {
            return;
        };
        while let Ok(event) = receiver.try_recv() {
            match event {
                Event::Line(line) => {
                    if let Some(path) = line.strip_prefix("Backup: ") {
                        state.backup_preview.push(PathBuf::from(path));
                    }
                    if let Status::Running { label, logs, .. } = &mut state.status {
                        *label = line.clone();
                        logs.push(line);
                        if logs.len() > 200 {
                            logs.remove(0);
                        }
                    }
                }
                Event::Finished(result) => {
                    let backups = state.backup_preview.clone();
                    let partial = !backups.is_empty();
                    let warning = result.as_ref().is_ok_and(|messages| {
                        messages.iter().any(|message| {
                            message.contains("incomplete") || message.contains("was skipped")
                        })
                    });
                    state.apply_status(Status::Finished {
                        outcome: if result.is_ok() {
                            if warning {
                                Outcome::PartialFailure
                            } else {
                                Outcome::Success
                            }
                        } else if self.cancelled.load(Ordering::Relaxed) {
                            Outcome::Cancelled
                        } else if partial {
                            Outcome::PartialFailure
                        } else {
                            Outcome::Failure
                        },
                        message: match result {
                            Ok(lines) => lines.join("\n"),
                            Err(error) => error,
                        },
                        repositories: vec![],
                        backups,
                    });
                    self.receiver = None;
                    return;
                }
            }
        }
    }
}
