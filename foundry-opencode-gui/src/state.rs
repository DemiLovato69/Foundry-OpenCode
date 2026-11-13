use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Install,
    Update,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Options,
    Stacks,
    Repositories,
    Assignments,
    Review,
    Progress,
    Result,
}

impl Step {
    pub fn title(self) -> &'static str {
        match self {
            Self::Welcome => "Welcome",
            Self::Options => "Applications",
            Self::Stacks => "Foundry stacks",
            Self::Repositories => "Repositories",
            Self::Assignments => "Assignments and RAG",
            Self::Review => "Review",
            Self::Progress => "Progress",
            Self::Result => "Result",
        }
    }

    fn next(self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::Options),
            Self::Options => Some(Self::Stacks),
            Self::Stacks => Some(Self::Repositories),
            Self::Repositories => Some(Self::Assignments),
            Self::Assignments => Some(Self::Review),
            _ => None,
        }
    }

    fn previous(self) -> Option<Self> {
        match self {
            Self::Options => Some(Self::Welcome),
            Self::Stacks => Some(Self::Options),
            Self::Repositories => Some(Self::Stacks),
            Self::Assignments => Some(Self::Repositories),
            Self::Review => Some(Self::Assignments),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Stack {
    pub id: u64,
    pub name: String,
    pub token: String,
    pub saved_token: bool,
    pub raw_host: String,
}

impl Stack {
    fn global() -> Self {
        Self {
            id: 0,
            name: "Global default".into(),
            raw_host: String::new(),
            token: String::new(),
            saved_token: false,
        }
    }

    pub fn host(&self) -> String {
        if let Some(clean_host) = strip_host(&self.raw_host) {
            return clean_host.to_string();
        }

        self.raw_host.to_string()
    }
}

#[derive(Clone)]
pub struct Repository {
    pub path: PathBuf,
    pub stack: Option<u64>,
    pub rag: bool,
}

#[derive(Clone)]
pub struct Choices {
    pub operation: Operation,
    pub accept_saved_defaults: bool,
    pub install_openchamber: bool,
    pub sync_alias: bool,
    pub use_global: bool,
    pub global: Stack,
    pub stacks: Vec<Stack>,
    pub repositories: Vec<Repository>,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            operation: Operation::Install,
            accept_saved_defaults: false,
            install_openchamber: false,
            sync_alias: false,
            use_global: false,
            global: Stack::global(),
            stacks: Vec::new(),
            repositories: Vec::new(),
        }
    }
}

pub enum Request {
    Start(Choices),
    Cancel,
    Retry,
    OpenTokenPage(String),
}

pub enum Outcome {
    Success,
    PartialFailure,
    Failure,
    Cancelled,
}

pub enum Status {
    Disconnected,
    Running {
        label: String,
        done: usize,
        total: Option<usize>,
        repositories: Vec<(PathBuf, String)>,
        logs: Vec<String>,
    },
    Finished {
        outcome: Outcome,
        message: String,
        repositories: Vec<(PathBuf, String)>,
        backups: Vec<PathBuf>,
    },
}

pub struct WizardState {
    pub step: Step,
    pub choices: Choices,
    pub saved: Option<Choices>,
    pub status: Status,
    pub request: Option<Request>,
    pub backend_connected: bool,
    pub show_tokens: bool,
    pub path_input: String,
    pub error: Option<String>,
    pub backup_preview: Vec<PathBuf>,
    used_saved_defaults: bool,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            step: Step::Welcome,
            choices: Choices::default(),
            saved: None,
            status: Status::Disconnected,
            request: None,
            backend_connected: false,
            show_tokens: false,
            path_input: String::new(),
            error: None,
            backup_preview: Vec::new(),
            used_saved_defaults: false,
        }
    }
}

impl WizardState {
    pub fn redact(&self, text: &str) -> String {
        self.choices
            .stacks
            .iter()
            .chain(std::iter::once(&self.choices.global))
            .filter(|stack| !stack.token.is_empty())
            .fold(text.to_owned(), |line, stack| {
                line.replace(&stack.token, "[redacted]")
            })
    }

    pub fn apply_status(&mut self, status: Status) {
        self.step = match status {
            Status::Disconnected => Step::Review,
            Status::Running { .. } => Step::Progress,
            Status::Finished { .. } => Step::Result,
        };
        self.status = status;
        self.request = None;
    }

    pub fn stack_name(&self, id: Option<u64>) -> &str {
        self.choices
            .global_stack()
            .into_iter()
            .chain(&self.choices.stacks)
            .find(|stack| Some(stack.id) == id)
            .map_or("Unassigned", |stack| stack.name.as_str())
    }

    pub fn add_stack(&mut self) {
        let mut id = 1;
        while self.choices.stacks.iter().any(|stack| stack.id == id) {
            id += 1;
        }
        self.choices.stacks.push(Stack {
            id,
            name: String::new(),
            raw_host: String::new(),
            token: String::new(),
            saved_token: false,
        });
    }

    pub fn remove_stack(&mut self, id: u64) -> bool {
        if self
            .choices
            .repositories
            .iter()
            .any(|repo| repo.stack == Some(id))
        {
            return false;
        }
        self.choices.stacks.retain(|stack| stack.id != id);
        true
    }

    pub fn add_repository(&mut self, path: PathBuf) -> Result<(), &'static str> {
        if path.as_os_str().is_empty() {
            return Err("Enter a repository folder.");
        }
        let normalized = path.canonicalize().unwrap_or(path);
        if self
            .choices
            .repositories
            .iter()
            .any(|repo| repo.path == normalized)
        {
            return Err("Repository already selected.");
        }
        let stack = self
            .choices
            .global_stack()
            .map(|stack| stack.id)
            .or_else(|| self.choices.stacks.first().map(|stack| stack.id));
        self.choices.repositories.push(Repository {
            path: normalized,
            stack,
            rag: true,
        });
        Ok(())
    }

    pub fn validation_error(&self) -> Option<&'static str> {
        match self.step {
            Step::Welcome
                if self.choices.operation == Operation::Update
                    && self.choices.accept_saved_defaults
                    && self.saved.is_none() =>
            {
                Some("No saved settings have been supplied by the backend.")
            }
            Step::Stacks => {
                let stacks: Vec<_> = self
                    .choices
                    .global_stack()
                    .into_iter()
                    .chain(&self.choices.stacks)
                    .collect();
                if stacks.is_empty() {
                    return Some("Configure at least one Foundry stack.");
                }
                if stacks.iter().any(|stack| !valid_host(&stack.host())) {
                    return Some("Each stack needs an HTTPS host without a path.");
                }
                if stacks
                    .iter()
                    .any(|stack| stack.token.is_empty() && !stack.saved_token)
                {
                    return Some("Each stack needs a token or a saved credential.");
                }
                if self
                    .choices
                    .stacks
                    .iter()
                    .any(|stack| stack.name.trim().is_empty())
                {
                    return Some("Name each stack.");
                }
                for (index, stack) in self.choices.stacks.iter().enumerate() {
                    if self.choices.stacks[..index]
                        .iter()
                        .any(|other| other.name.eq_ignore_ascii_case(&stack.name))
                        || stack.name.eq_ignore_ascii_case("Global default")
                    {
                        return Some("Stack names must be unique.");
                    }
                }
                None
            }
            Step::Repositories if self.choices.repositories.is_empty() => {
                Some("Add at least one repository.")
            }
            Step::Assignments
                if self.choices.repositories.iter().any(|repo| {
                    !self
                        .choices
                        .global_stack()
                        .into_iter()
                        .chain(&self.choices.stacks)
                        .any(|stack| Some(stack.id) == repo.stack)
                }) =>
            {
                Some("Assign every repository to a configured stack.")
            }
            _ => None,
        }
    }

    pub fn advance(&mut self) -> bool {
        if let Some(error) = self.validation_error() {
            self.error = Some(error.into());
            return false;
        }
        self.error = None;
        if self.step == Step::Welcome
            && self.choices.operation == Operation::Update
            && self.choices.accept_saved_defaults
        {
            self.choices = self.saved.clone().expect("validated saved defaults");
            self.choices.operation = Operation::Update;
            self.choices.accept_saved_defaults = true;
            self.used_saved_defaults = true;
            self.step = Step::Review;
            return true;
        }
        if self.step == Step::Welcome
            && self.choices.operation == Operation::Update
            && self.choices.global.raw_host.is_empty()
            && self.choices.stacks.is_empty()
            && self.choices.repositories.is_empty()
            && let Some(saved) = &self.saved
        {
            let openchamber = self.choices.install_openchamber;
            self.choices = saved.clone();
            self.choices.install_openchamber = openchamber;
        }
        if let Some(next) = self.step.next() {
            self.step = next;
            return true;
        }
        false
    }

    pub fn back(&mut self) {
        if self.step == Step::Review && self.used_saved_defaults {
            self.step = Step::Welcome;
            self.used_saved_defaults = false;
        } else if let Some(previous) = self.step.previous() {
            self.step = previous;
        }
        self.error = None;
    }

    pub fn review_lines(&self) -> Vec<String> {
        let mut lines = vec![format!(
            "OpenChamber: {}",
            if self.choices.install_openchamber {
                "Install"
            } else {
                "Skip"
            }
        )];
        for stack in self
            .choices
            .global_stack()
            .into_iter()
            .chain(&self.choices.stacks)
        {
            lines.push(format!(
                "{} — {} (token: {})",
                stack.name,
                stack.host(),
                if !stack.token.is_empty() {
                    "provided"
                } else if stack.saved_token {
                    "saved"
                } else {
                    "missing"
                }
            ));
        }
        for repo in &self.choices.repositories {
            lines.push(format!(
                "{} → {} — RAG: {}",
                repo.path.display(),
                self.stack_name(repo.stack),
                if repo.rag { "on" } else { "off" }
            ));
        }
        lines.into_iter().map(|line| self.redact(&line)).collect()
    }
}

impl Choices {
    pub fn global_stack(&self) -> Option<&Stack> {
        self.use_global.then_some(&self.global)
    }
}

fn strip_host(host: &str) -> Option<&str> {
    host.strip_suffix("/")
}

fn valid_host(host: &str) -> bool {
    host.strip_prefix("https://").is_some_and(|name| {
        !name.is_empty()
            && !name.contains(['/', '?', '#', '@'])
            && !name.chars().any(char::is_whitespace)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_stack(state: &mut WizardState) {
        state.add_stack();
        state.choices.stacks[0].name = "Foundry".into();
        state.choices.stacks[0].raw_host = "https://foundry.example".into();
        state.choices.stacks[0].token = "secret".into();
    }

    #[test]
    fn navigation_preserves_choices_and_does_not_invent_success() {
        let mut state = WizardState::default();
        assert!(state.advance());
        state.choices.install_openchamber = true;
        assert!(state.advance());
        valid_stack(&mut state);
        assert!(state.advance());
        state.add_repository(PathBuf::from("/tmp/repo")).unwrap();
        assert!(state.choices.repositories[0].rag);
        assert!(state.advance());
        state.choices.repositories[0].rag = false;
        assert!(state.advance());
        assert_eq!(state.step, Step::Review);
        state.back();
        assert!(!state.choices.repositories[0].rag);
        assert!(state.choices.install_openchamber);
        assert!(
            state
                .review_lines()
                .iter()
                .all(|line| !line.contains("secret"))
        );
        assert!(matches!(state.status, Status::Disconnected));
        assert_eq!(state.redact("token: secret"), "token: [redacted]");
        assert!(state.advance());
        assert!(!state.advance());
        state.apply_status(Status::Running {
            label: "Installing".into(),
            done: 0,
            total: None,
            repositories: vec![],
            logs: vec![],
        });
        assert_eq!(state.step, Step::Progress);
        state.apply_status(Status::Finished {
            outcome: Outcome::Success,
            message: "Done".into(),
            repositories: vec![],
            backups: vec![],
        });
        assert_eq!(state.step, Step::Result);
    }

    #[test]
    fn saved_defaults_keep_rag_off_and_missing_defaults_block_shortcut() {
        let mut state = WizardState::default();
        state.choices.operation = Operation::Update;
        state.choices.accept_saved_defaults = true;
        assert!(!state.advance());
        let mut saved = Choices {
            operation: Operation::Update,
            use_global: true,
            ..Choices::default()
        };
        saved.global.raw_host = "https://foundry.example".into();
        saved.global.saved_token = true;
        saved.stacks.push(Stack {
            id: 42,
            name: "Saved stack".into(),
            raw_host: "https://foundry.example".into(),
            token: String::new(),
            saved_token: true,
        });
        saved.repositories.push(Repository {
            path: PathBuf::from("/tmp/repo"),
            stack: Some(0),
            rag: false,
        });
        state.saved = Some(saved);
        assert!(state.advance());
        assert_eq!(state.step, Step::Review);
        assert!(!state.choices.repositories[0].rag);
        state.back();
        assert_eq!(state.step, Step::Welcome);
        state.add_stack();
        assert_eq!(state.choices.stacks[1].id, 1);
    }

    #[test]
    fn editing_update_starts_with_saved_assignments() {
        let mut state = WizardState::default();
        state.choices.operation = Operation::Update;
        let mut saved = Choices {
            use_global: true,
            sync_alias: true,
            ..Choices::default()
        };
        saved.global.raw_host = "https://foundry.example".into();
        saved.global.saved_token = true;
        saved.repositories.push(Repository {
            path: PathBuf::from("/tmp/saved"),
            stack: Some(0),
            rag: false,
        });
        state.saved = Some(saved);
        assert!(state.advance());
        assert_eq!(state.step, Step::Options);
        assert_eq!(
            state.choices.repositories[0].path,
            PathBuf::from("/tmp/saved")
        );
        assert!(!state.choices.repositories[0].rag);
        assert!(state.choices.sync_alias);
    }

    #[test]
    fn names_assignments_and_duplicate_repositories_are_checked() {
        let mut state = WizardState {
            step: Step::Stacks,
            ..WizardState::default()
        };
        valid_stack(&mut state);
        state.add_stack();
        state.choices.stacks[1].name = "foundry".into();
        state.choices.stacks[1].raw_host = "https://foundry.example".into();
        state.choices.stacks[1].saved_token = true;
        assert_eq!(
            state.validation_error(),
            Some("Stack names must be unique.")
        );
        state.choices.stacks[1].name = "Other".into();
        assert!(state.validation_error().is_none());
        state.add_repository(PathBuf::from("/tmp/repo")).unwrap();
        assert!(state.add_repository(PathBuf::from("/tmp/repo")).is_err());
        assert!(!state.remove_stack(1));
        state.choices.repositories[0].stack = None;
        state.step = Step::Assignments;
        assert!(state.validation_error().is_some());
    }
}
