mod cleanup;
mod ops;
mod ui;

use std::fs;
use std::path::PathBuf;
use std::process::Command as Process;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use color_eyre::eyre::{self, Context};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use ratatui::DefaultTerminal;
use ui::notice;

fn version() -> &'static str {
    option_env!("FOUNDRY_OPENCODE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

#[derive(Parser)]
#[command(version = version(), about = "Foundry OpenCode installer")]
struct Args {
    /// Preview actions without opening a browser or changing anything
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Option<Mode>,
}

#[derive(Subcommand)]
enum Mode {
    /// Generate .envrc in the current stack directory
    Init,
    /// Update Homebrew packages and fast-forward the config checkout
    Update,
    /// Install tools and set up Foundry OpenCode (the default)
    Install,
    /// Print the installer version
    Version,
    /// Show example commands
    Usage,
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    match args.command {
        Some(Mode::Version) => {
            println!("{}", version());
            return Ok(());
        }
        Some(Mode::Usage) => {
            println!(
                "foundry-opencode           Install (same as install)\nfoundry-opencode install   Re-run installation; back up existing configs\nfoundry-opencode update    Update installed tools and configs\nfoundry-opencode init      Create .envrc in the current stack directory\nfoundry-opencode --dry-run install  Preview without changes\nfoundry-opencode version   Print version\nfoundry-opencode usage     Show these examples"
            );
            return Ok(());
        }
        _ => {}
    }
    color_eyre::install()?;
    if !cfg!(target_os = "macos") && !args.dry_run && !matches!(args.command, Some(Mode::Init)) {
        eyre::bail!("Installation and updates require macOS and Homebrew");
    }
    let staged = ratatui::run(|terminal| {
        execute!(std::io::stdout(), EnableMouseCapture)?;
        let result = match args.command.unwrap_or(Mode::Install) {
            Mode::Init => init(terminal, args.dry_run).map(|_| None),
            Mode::Update => update(terminal, args.dry_run).map(|_| None),
            Mode::Install => install(terminal, args.dry_run),
            _ => unreachable!(),
        };
        execute!(std::io::stdout(), DisableMouseCapture)?;
        result
    })?;
    if let Some(staged) = staged {
        ops::install_binary(&staged, std::path::Path::new(ops::BIN_PATH))?;
        println!("Installed foundry-opencode to {}", ops::BIN_PATH);
    }
    Ok(())
}

fn token_page(url: &str) -> String {
    format!("{url}/workspace/settings/tokens")
}

fn preview_envrc(path: &std::path::Path, url: &str) -> String {
    format!(
        "Would create {} (mode 0600):\nexport OPENCODE_API_KEY='[REDACTED]'\nexport OPENCODE_BASE_URL='{url}'\nopencode service restart",
        path.display()
    )
}

fn git_protection(cwd: &std::path::Path) -> eyre::Result<Option<PathBuf>> {
    let repo = Process::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(cwd)
        .output();
    let git_root = repo
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()));
    if let Some(root) = git_root {
        let tracked = Process::new("git")
            .args(["ls-files", "--error-unmatch", "--", ".envrc"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .current_dir(cwd)
            .output()?;
        eyre::ensure!(
            !tracked.status.success(),
            ".envrc is already tracked by Git; untrack it before creating a secret file"
        );
        let ignored = Process::new("git")
            .args(["check-ignore", "--quiet", "--no-index", "--", ".envrc"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .current_dir(cwd)
            .status()?
            .success();
        if !ignored {
            let path = root.join(".gitignore");
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn open_token_page(terminal: &mut DefaultTerminal, url: &str, dry_run: bool) -> eyre::Result<()> {
    let page = token_page(url);
    if dry_run {
        return notice(
            terminal,
            "Browser preview",
            &format!("Would open {page}; no browser opened."),
        );
    }
    eyre::ensure!(cfg!(target_os = "macos"), "Browser flow requires macOS");
    eyre::ensure!(
        Process::new("open").arg(&page).status()?.success(),
        "Could not open Foundry token page"
    );
    Ok(())
}

fn init(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let cwd = std::env::current_dir()?;
    let envrc = cwd.join(".envrc");
    if fs::symlink_metadata(&envrc).is_ok() {
        if dry_run {
            return notice(
                terminal,
                "init — dry run",
                &format!(
                    "{} already exists; would not overwrite it.",
                    envrc.display()
                ),
            );
        }
        eyre::bail!("{} already exists; it was not changed", envrc.display());
    }
    let ignore = git_protection(&cwd)?;
    let mut url = String::new();
    let mut key = String::new();
    let mut step = 0usize;
    loop {
        let nav = match step {
            0 => ui::step_text(
                terminal,
                "Foundry host",
                "Enter your stack's HTTPS Foundry host (for example https://your-stack.palantirfoundry.com). Run init from the stack directory where .envrc belongs.",
                &mut url,
                true,
                ui::Field::Plain,
            )?,
            1 => ui::step_text(
                terminal,
                "Foundry token",
                &format!(
                    "Paste a token for {url}. Open the token page only if you choose to create one.{}",
                    if dry_run {
                        " A dry run does not require a token."
                    } else {
                        ""
                    }
                ),
                &mut key,
                false,
                ui::Field::Token,
            )?,
            _ => {
                let mut review = preview_envrc(&envrc, &url);
                if let Some(ignore) = &ignore {
                    review.push_str(&format!(
                        "\nAdd .envrc to {} to protect your token from Git.",
                        ignore.display()
                    ));
                }
                if dry_run {
                    review.push_str("\n\nDry run: no browser or files changed.");
                }
                ui::step_review(
                    terminal,
                    if dry_run {
                        "Review preview"
                    } else {
                        "Review .envrc"
                    },
                    &review,
                    dry_run,
                    false,
                    "Create .envrc",
                )?
            }
        };
        match nav {
            ui::Nav::Cancel => eyre::bail!("Cancelled; no changes were made"),
            ui::Nav::Back => step = step.saturating_sub(1),
            ui::Nav::Open => open_token_page(terminal, &url, dry_run)?,
            ui::Nav::Next => {
                if step == 0 && !ops::valid_url(&url) {
                    notice(
                        terminal,
                        "Invalid host",
                        "Enter an HTTPS Foundry host without a path or spaces.",
                    )?;
                    continue;
                }
                if step == 1 && !dry_run && key.is_empty() {
                    notice(
                        terminal,
                        "Token required",
                        "Paste your token or go back to correct the host.",
                    )?;
                    continue;
                }
                if step == 2 {
                    break;
                }
                step += 1;
            }
        }
    }
    if dry_run {
        let mut steps = Vec::new();
        if ignore.is_some() {
            steps.push(("add .envrc to .gitignore".to_string(), 220));
        }
        steps.push(("create .envrc in the stack directory".to_string(), 350));
        simulate(terminal, &steps)?;
        return notice(
            terminal,
            "Dry-run complete",
            &format!(
                "{}{}\n\nSimulation only: no file was created.",
                preview_envrc(&envrc, &url),
                ignore
                    .as_ref()
                    .map(|p| format!("\nWould add .envrc to {}", p.display()))
                    .unwrap_or_default()
            ),
        );
    }
    if let Some(path) = ignore {
        ops::add_gitignore(&path)?;
    }
    ops::write_envrc(&envrc, &url, &key)?;
    notice(
        terminal,
        "init — complete",
        "Created .envrc. Review it, then run direnv allow unless its projects directory is whitelisted.",
    )
}

// shortcut: external commands report no percentages, replace the spinner when they expose progress.
struct Progress {
    done: usize,
    total: usize,
}

impl Progress {
    fn show(&self, terminal: &mut DefaultTerminal, label: &str) -> eyre::Result<()> {
        terminal.draw(|frame| ui::progress(frame, label, self.done, self.total))?;
        Ok(())
    }
}

fn simulate(terminal: &mut DefaultTerminal, steps: &[(String, u64)]) -> eyre::Result<()> {
    // shortcut: mock times are illustrative, use measured timings only if real task telemetry is added.
    for (index, (label, millis)) in steps.iter().enumerate() {
        let start = Instant::now();
        loop {
            let fraction = (start.elapsed().as_secs_f64() * 1000.0 / *millis as f64).min(1.0);
            terminal.draw(|frame| {
                ui::mock_progress(
                    frame,
                    &format!("Would {label} (mock {millis} ms)"),
                    index,
                    steps.len(),
                    fraction,
                )
            })?;
            if fraction >= 1.0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    terminal.draw(|frame| {
        ui::mock_progress(frame, "Simulation complete", steps.len(), steps.len(), 0.0)
    })?;
    Ok(())
}

fn run(
    terminal: &mut DefaultTerminal,
    progress: &mut Progress,
    label: &str,
    mut command: Process,
) -> eyre::Result<()> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(command.output());
    });
    let mut tick = 0;
    let spinner = ['|', '/', '-', '\\'];
    loop {
        progress.show(
            terminal,
            &format!("{label}  {}", spinner[tick % spinner.len()]),
        )?;
        tick += 1;
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(Ok(output)) if output.status.success() => {
                progress.done += 1;
                progress.show(terminal, label)?;
                return Ok(());
            }
            Ok(Ok(output)) => eyre::bail!(
                "{label} failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Ok(Err(error)) => return Err(error).wrap_err(format!("{label} failed to start")),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eyre::bail!("{label} stopped unexpectedly")
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn installed(name: &str, cask: bool) -> eyre::Result<bool> {
    let mut command = Process::new("brew");
    command.arg("list");
    if cask {
        command.arg("--cask");
    }
    Ok(command
        .arg("--versions")
        .arg(name)
        .output()?
        .status
        .success())
}

fn brew_package(
    terminal: &mut DefaultTerminal,
    progress: &mut Progress,
    name: &str,
    cask: bool,
) -> eyre::Result<()> {
    let exists = installed(name, cask)?;
    let mut command = Process::new("brew");
    command.arg(if exists { "upgrade" } else { "install" });
    if cask {
        command.arg("--cask");
    }
    command.arg(name);
    run(
        terminal,
        progress,
        &format!("{} {name}", if exists { "Upgrading" } else { "Installing" }),
        command,
    )
}

fn brew_update(terminal: &mut DefaultTerminal, progress: &mut Progress) -> eyre::Result<()> {
    eyre::ensure!(
        Process::new("brew").arg("--version").output().is_ok(),
        "Homebrew is required: https://brew.sh"
    );
    run(terminal, progress, "Refreshing Homebrew", {
        let mut cmd = Process::new("brew");
        cmd.arg("update");
        cmd
    })
}

fn home() -> eyre::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or_else(|| eyre::eyre!("HOME is not set"))?,
    ))
}

fn direnv_config() -> eyre::Result<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(home()?.join(".config"));
    Ok(root.join("direnv/direnv.toml"))
}

fn zshrc_path(home: &std::path::Path) -> eyre::Result<PathBuf> {
    let directory = match std::env::var_os("ZDOTDIR") {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            eyre::ensure!(
                path.is_absolute(),
                "ZDOTDIR must be an absolute path to update its .zshrc safely"
            );
            path
        }
        _ => home.to_path_buf(),
    };
    eyre::ensure!(
        directory.is_dir(),
        "Zsh configuration directory {} does not exist",
        directory.display()
    );
    Ok(directory.join(".zshrc"))
}

#[derive(Clone)]
enum InstallPage {
    Options,
    Cli,
    Backup,
    Projects,
    Trust,
    Stacks,
    Host(String),
    Token(String),
    Review,
}

struct InstallPlan {
    config: PathBuf,
    backup: PathBuf,
    existing: bool,
    projects: PathBuf,
    zshrc: PathBuf,
    with_chamber: bool,
    install_cli: bool,
    new_binary: bool,
    stacks: Vec<(PathBuf, String, String, Option<PathBuf>)>,
    cleanup: cleanup::Inventory,
}

fn install_pages(
    existing: bool,
    new_binary: bool,
    projects: &std::path::Path,
    selected: &[String],
) -> Vec<InstallPage> {
    let mut pages = vec![InstallPage::Options];
    if new_binary {
        pages.push(InstallPage::Cli);
    }
    if existing {
        pages.push(InstallPage::Backup);
    }
    pages.extend([
        InstallPage::Projects,
        InstallPage::Trust,
        InstallPage::Stacks,
    ]);
    for name in selected {
        if fs::symlink_metadata(projects.join(name).join(".envrc")).is_err() {
            pages.push(InstallPage::Host(name.clone()));
            pages.push(InstallPage::Token(name.clone()));
        }
    }
    pages.push(InstallPage::Review);
    pages
}

fn collect_install_plan(
    terminal: &mut DefaultTerminal,
    dry_run: bool,
) -> eyre::Result<Option<InstallPlan>> {
    let user_home = home()?;
    let config = user_home.join(".config/opencode");
    let zshrc = zshrc_path(&user_home)?;
    let hook_needed = ops::needs_zsh_hook(&zshrc)?;
    let existing = fs::symlink_metadata(&config).is_ok();
    let backup = ops::backup_path(&config);
    let new_binary = ops::bin_is_missing(std::path::Path::new(ops::BIN_PATH))?;
    let mut with_chamber = false;
    let mut install_cli = new_binary;
    let mut backup_ok = false;
    let mut trust = false;
    let mut projects_input = String::new();
    let mut projects = PathBuf::new();
    let mut names = Vec::new();
    let mut selected = Vec::new();
    let mut query = String::new();
    let mut credentials = std::collections::BTreeMap::<String, (String, String)>::new();
    let mut page = 0usize;
    let mut preview_summary = String::new();
    let mut cleanup_plan = None;
    loop {
        let pages = install_pages(existing, new_binary, &projects, &selected);
        let current = pages[page].clone();
        let nav = match current {
            InstallPage::Options => ui::step_checkbox(
                terminal,
                "Optional application",
                "OpenCode and direnv are required. OpenChamber is optional and uses the Homebrew cask.",
                "Install OpenChamber",
                &mut with_chamber,
                true,
            )?,
            InstallPage::Cli => ui::step_checkbox(
                terminal,
                "Make installer available everywhere",
                &format!(
                    "After setup succeeds, copy this binary to {}. Administrator permission may be required after this wizard closes. An existing binary will never be overwritten.",
                    ops::BIN_PATH
                ),
                "Copy installer to /usr/local/bin",
                &mut install_cli,
                false,
            )?,
            InstallPage::Backup => ui::step_checkbox(
                terminal,
                "Existing configuration",
                &format!(
                    "{} already exists. We will back it up to {} before replacing it. Your old files remain in that backup.",
                    config.display(),
                    backup.display()
                ),
                "I understand",
                &mut backup_ok,
                false,
            )?,
            InstallPage::Projects => ui::step_text(
                terminal,
                "Main projects directory",
                "Choose the EXISTING parent directory that contains one folder per Foundry stack.\n\nExample:\n~/Projects/\n  Stack1/  ← .envrc will be created here\n    Functions Repo/\n  Stack2/  ← another .envrc\n    OSDK React App Repo/\n\nThis parent directory will be broadly trusted by direnv. Keeping each .envrc at a stack root lets you switch stacks automatically. Repositories inside stacks need no .envrc. Enter an absolute path or ~/Projects:",
                &mut projects_input,
                false,
                ui::Field::Directory(user_home.as_path()),
            )?,
            InstallPage::Trust => ui::step_checkbox(
                terminal,
                "Trust projects directory",
                &format!(
                    "Direnv will trust ALL .envrc files below {} without individual approval. Anyone able to write in that folder, including a Git checkout, could run code automatically.",
                    projects.display()
                ),
                "I understand and trust this entire directory",
                &mut trust,
                false,
            )?,
            InstallPage::Stacks => ui::step_stacks(terminal, &names, &mut selected, &mut query)?,
            InstallPage::Host(name) => {
                let fields = credentials.entry(name.clone()).or_default();
                ui::step_text(
                    terminal,
                    &format!("{name} — Foundry host"),
                    "Enter this stack's Foundry base URL (https://host). The .envrc will be created in the selected stack directory, not its nested repositories.",
                    &mut fields.0,
                    false,
                    ui::Field::Plain,
                )?
            }
            InstallPage::Token(name) => {
                let fields = credentials.entry(name.clone()).or_default();
                ui::step_text(
                    terminal,
                    &format!("{name} — API token"),
                    &format!(
                        "Paste a token for {}. Only activate Open token page if you want to create one in your browser. The token remains hidden.{}",
                        fields.0,
                        if dry_run {
                            " No token is requested in dry-run mode."
                        } else {
                            ""
                        }
                    ),
                    &mut fields.1,
                    false,
                    ui::Field::Token,
                )?
            }
            InstallPage::Review => {
                let inventory = cleanup::inspect(&user_home, with_chamber)?;
                let mut preview = format!(
                    "Homebrew: update; install/upgrade OpenCode, direnv{}.\nClone {} to {}{}.\nTrust all .envrc files below {}.\nZsh hook in {}: {}.\nInstaller copy: {}.",
                    if with_chamber { ", OpenChamber" } else { "" },
                    ops::REMOTE,
                    config.display(),
                    if existing {
                        format!(" (backup: {})", backup.display())
                    } else {
                        String::new()
                    },
                    projects.display(),
                    zshrc.display(),
                    if hook_needed {
                        format!("add {}", ops::ZSH_HOOK)
                    } else {
                        "already configured (skip)".into()
                    },
                    if install_cli && new_binary {
                        ops::BIN_PATH
                    } else {
                        "skip"
                    }
                );
                if inventory.brew_v2 {
                    preview.push_str(
                        "\nMigrate Homebrew opencode-v2 to standard opencode (V2 verified).",
                    );
                }
                for name in &inventory.brew_v1 {
                    preview.push_str(&format!("\nUninstall V1 Homebrew package: {name}"));
                }
                for name in &inventory.npm_v1 {
                    preview.push_str(&format!("\nUninstall global V1 npm package: {name}"));
                }
                for path in &inventory.files {
                    preview.push_str(&format!(
                        "\nMove non-Homebrew OpenCode binary to Trash: {}",
                        path.display()
                    ));
                }
                if inventory.chamber {
                    preview.push_str("\nMove non-Homebrew /Applications/OpenChamber.app to Trash (app data is preserved).");
                }
                for path in &inventory.unknown {
                    preview.push_str(&format!(
                        "\nBLOCKED: unidentified installation at {}",
                        path.display()
                    ));
                }
                cleanup_plan = Some(inventory);
                for name in &selected {
                    let path = projects.join(name).join(".envrc");
                    if fs::symlink_metadata(&path).is_ok() {
                        preview.push_str(&format!("\nSkip existing {}", path.display()));
                    } else if let Some((url, _)) = credentials.get(name) {
                        preview.push_str(&format!("\n{}", preview_envrc(&path, url)));
                        if let Some(ignore) = git_protection(&projects.join(name))? {
                            preview.push_str(&format!(
                                "\nAdd .envrc to {} to protect your token from Git.",
                                ignore.display()
                            ));
                        }
                    }
                }
                if dry_run {
                    preview.push_str("\n\nDry run: read-only inventory only; no install/removal commands, browser or file changes.");
                    preview_summary = preview.clone();
                }
                ui::step_review(
                    terminal,
                    if dry_run {
                        "Review preview"
                    } else {
                        "Review installation"
                    },
                    &preview,
                    dry_run,
                    false,
                    "Install",
                )?
            }
        };
        match nav {
            ui::Nav::Cancel => eyre::bail!("Cancelled; no changes were made"),
            ui::Nav::Back => page = page.saturating_sub(1),
            ui::Nav::Open => {
                if let InstallPage::Token(name) = &pages[page] {
                    let url = &credentials[name].0;
                    open_token_page(terminal, url, dry_run)?;
                }
            }
            ui::Nav::Next => {
                match &pages[page] {
                    InstallPage::Backup if !backup_ok => {
                        notice(
                            terminal,
                            "Confirmation required",
                            "Please agree to the backup and replacement, or go back to cancel.",
                        )?;
                        continue;
                    }
                    InstallPage::Trust if !trust => {
                        notice(
                            terminal,
                            "Confirmation required",
                            "Please acknowledge the broad direnv trust before continuing.",
                        )?;
                        continue;
                    }
                    InstallPage::Projects => {
                        let expanded = if let Some(rest) = projects_input.strip_prefix("~/") {
                            user_home.join(rest)
                        } else {
                            PathBuf::from(&projects_input)
                        };
                        let Ok(path) = expanded.canonicalize() else {
                            notice(
                                terminal,
                                "Invalid directory",
                                "Choose an existing parent directory, for example ~/Projects.",
                            )?;
                            continue;
                        };
                        if !path.is_dir()
                            || path.starts_with(config.canonicalize().unwrap_or(config.clone()))
                        {
                            notice(
                                terminal,
                                "Invalid directory",
                                "Select an existing directory outside the config directory being replaced.",
                            )?;
                            continue;
                        }
                        if path != projects {
                            projects = path;
                            names = fs::read_dir(&projects)?
                                .filter_map(Result::ok)
                                .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
                                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                                .collect();
                            names.sort();
                            selected.clear();
                            query.clear();
                            credentials.clear();
                            trust = false;
                        }
                    }
                    InstallPage::Host(name) => {
                        if !ops::valid_url(&credentials[name].0) {
                            notice(
                                terminal,
                                "Invalid URL",
                                "Enter the HTTPS Foundry host without a path or spaces.",
                            )?;
                            continue;
                        }
                    }
                    InstallPage::Token(name) if !dry_run && credentials[name].1.is_empty() => {
                        notice(
                            terminal,
                            "Token required",
                            "Paste a Foundry API token, or go back to change the host.",
                        )?;
                        continue;
                    }
                    _ => {}
                }
                if page + 1 == pages.len() {
                    let inventory = cleanup_plan
                        .take()
                        .ok_or_else(|| eyre::eyre!("Missing install inventory"))?;
                    if !inventory.unknown.is_empty() && !dry_run {
                        notice(
                            terminal,
                            "Cleanup blocked",
                            "An installation could not be identified safely. Remove it manually or change PATH before installing.",
                        )?;
                        continue;
                    }
                    if dry_run {
                        let mut steps = vec![("refresh Homebrew metadata".to_string(), 300)];
                        for name in &inventory.brew_v1 {
                            steps.push((format!("uninstall V1 Homebrew {name}"), 220));
                        }
                        for name in &inventory.npm_v1 {
                            steps.push((format!("npm uninstall -g {name}"), 240));
                        }
                        for path in &inventory.files {
                            steps.push((format!("move {} to Trash", path.display()), 180));
                        }
                        if inventory.brew_v2 {
                            steps.push(("uninstall Homebrew opencode-v2".to_string(), 280));
                        }
                        steps.push(("install or upgrade OpenCode V2".to_string(), 400));
                        if with_chamber {
                            if inventory.chamber {
                                steps.push((
                                    "move non-Homebrew OpenChamber.app to Trash".to_string(),
                                    200,
                                ));
                            }
                            steps
                                .push(("install or upgrade optional OpenChamber".to_string(), 350));
                        }
                        steps.push(("install or upgrade direnv".to_string(), 300));
                        steps.push((
                            "clone config repository to ~/.config/opencode".to_string(),
                            500,
                        ));
                        steps.push(("whitelist the projects directory".to_string(), 200));
                        steps.push((
                            if hook_needed {
                                "add direnv hook to .zshrc"
                            } else {
                                "skip existing direnv shell hook"
                            }
                            .to_string(),
                            if hook_needed { 200 } else { 100 },
                        ));
                        for name in &selected {
                            let action = if fs::symlink_metadata(projects.join(name).join(".envrc"))
                                .is_ok()
                            {
                                "skip existing .envrc"
                            } else {
                                "create .envrc"
                            };
                            steps.push((format!("{action} for {name}"), 170));
                        }
                        if install_cli && new_binary {
                            steps.push(("copy the installer to /usr/local/bin".to_string(), 220));
                        }
                        if !inventory.unknown.is_empty() {
                            steps.push((
                                "skip unresolved OpenCode installations (manual removal required)"
                                    .to_string(),
                                150,
                            ));
                        }
                        simulate(terminal, &steps)?;
                        notice(terminal, "Dry-run complete", &preview_summary)?;
                        return Ok(None);
                    }
                    let mut stacks = Vec::new();
                    for name in selected {
                        let path = projects.join(&name);
                        if fs::symlink_metadata(path.join(".envrc")).is_ok() {
                            continue;
                        }
                        let ignore = git_protection(&path)?;
                        let (url, key) = credentials
                            .remove(&name)
                            .ok_or_else(|| eyre::eyre!("Missing stack credentials"))?;
                        stacks.push((path, url, key, ignore));
                    }
                    return Ok(Some(InstallPlan {
                        config,
                        backup,
                        existing,
                        projects,
                        zshrc,
                        with_chamber,
                        install_cli: install_cli && new_binary,
                        new_binary,
                        stacks,
                        cleanup: inventory,
                    }));
                }
                page += 1;
            }
        }
    }
}

fn install(
    terminal: &mut DefaultTerminal,
    dry_run: bool,
) -> eyre::Result<Option<ops::StagedBinary>> {
    let Some(plan) = collect_install_plan(terminal, dry_run)? else {
        return Ok(None);
    };
    let InstallPlan {
        config,
        backup,
        existing,
        projects,
        zshrc,
        with_chamber,
        install_cli,
        new_binary,
        stacks,
        cleanup: inventory,
    } = plan;
    let mut progress = Progress {
        done: 0,
        total: 6
            + usize::from(with_chamber)
            + stacks.len()
            + inventory.brew_v1.len()
            + inventory.npm_v1.len()
            + inventory.files.len()
            + usize::from(inventory.brew_v2)
            + usize::from(inventory.chamber),
    };
    brew_update(terminal, &mut progress)?;
    cleanup::latest_is_v2()?;
    eyre::ensure!(
        cleanup::inspect(&home()?, with_chamber)? == inventory,
        "Install inventory changed after the Homebrew refresh; review the install again before removing anything"
    );
    if inventory.chamber {
        let running = Process::new("pgrep").args(["-x", "OpenChamber"]).output()?;
        eyre::ensure!(
            running.status.code() == Some(1),
            "Quit OpenChamber or verify its process status before replacing its non-Homebrew app bundle"
        );
    }
    let mut trashed = Vec::new();
    for name in &inventory.brew_v1 {
        let mut command = Process::new("brew");
        command.args(["uninstall", name]);
        run(
            terminal,
            &mut progress,
            &format!("Removing V1 Homebrew package {name}"),
            command,
        )?;
    }
    for name in &inventory.npm_v1 {
        let mut command = Process::new("npm");
        command.args(["uninstall", "-g", name]);
        run(
            terminal,
            &mut progress,
            &format!("Removing V1 npm package {name}"),
            command,
        )?;
    }
    for path in &inventory.files {
        progress.show(terminal, &format!("Moving {} to Trash", path.display()))?;
        trashed.push(cleanup::move_to_trash(path, &home()?)?);
        progress.done += 1;
    }
    if inventory.brew_v2 {
        let mut command = Process::new("brew");
        command.args(["uninstall", "opencode-v2"]);
        run(
            terminal,
            &mut progress,
            "Migrating Homebrew opencode-v2",
            command,
        )?;
    }
    if let Err(error) = brew_package(terminal, &mut progress, "opencode", false) {
        if inventory.brew_v2 {
            let mut restore = Process::new("brew");
            restore.args(["install", "opencode-v2"]);
            if let Err(restore_error) = run(
                terminal,
                &mut progress,
                "Restoring previous Homebrew OpenCode",
                restore,
            ) {
                eyre::bail!("{error}; restore of opencode-v2 also failed: {restore_error}");
            }
        }
        return Err(error);
    }
    let version = Process::new("opencode").arg("--version").output()?;
    eyre::ensure!(
        version.status.success() && cleanup::reports_v2(&String::from_utf8_lossy(&version.stdout)),
        "Installed opencode does not resolve to V2; check PATH before continuing"
    );
    if with_chamber {
        if inventory.chamber {
            progress.show(terminal, "Moving non-Homebrew OpenChamber.app to Trash")?;
            trashed.push(cleanup::move_to_trash(
                std::path::Path::new("/Applications/OpenChamber.app"),
                &home()?,
            )?);
            progress.done += 1;
        }
        brew_package(terminal, &mut progress, "openchamber", true)?;
    }
    brew_package(terminal, &mut progress, "direnv", false)?;
    fs::create_dir_all(config.parent().expect("config has parent"))?;
    let staged = ops::backup_path(&config);
    let mut clone = Process::new("git");
    clone
        .args(["clone", "--depth", "1", "--branch", "main", ops::REMOTE])
        .arg(&staged);
    if let Err(error) = run(terminal, &mut progress, "Cloning latest configs", clone) {
        if staged.exists() {
            fs::remove_dir_all(&staged)?;
        }
        return Err(error);
    }
    // Stage before moving ~/.config/opencode; the running binary may be inside it.
    let binary = if install_cli {
        Some(ops::stage_binary(&std::env::current_exe()?)?)
    } else {
        None
    };
    ops::replace_config(&config, &staged, existing.then_some(backup.as_path()))?;
    progress.show(terminal, "Updating direnv whitelist...")?;
    ops::whitelist(&direnv_config()?, &projects)?;
    progress.done += 1;
    progress.show(terminal, "Configuring direnv Zsh hook...")?;
    let hook_added = ops::add_zsh_hook(&zshrc)?;
    progress.done += 1;
    for (path, url, key, ignore) in stacks {
        progress.show(
            terminal,
            &format!("Creating {}", path.join(".envrc").display()),
        )?;
        if let Some(ignore) = ignore {
            ops::add_gitignore(&ignore)?;
        }
        ops::write_envrc(&path.join(".envrc"), &url, &key)?;
        progress.done += 1;
    }
    progress.show(terminal, "Installation complete")?;
    notice(
        terminal,
        "Install — complete",
        &format!(
            "Installation finished.{}\nDirenv hook {} in {}. Open a new Zsh shell to activate it.{}{}",
            if existing {
                format!(" Backup: {}", backup.display())
            } else {
                String::new()
            },
            if hook_added {
                "added"
            } else {
                "already configured"
            },
            zshrc.display(),
            if binary.is_some() {
                "\nThe CLI will be copied to /usr/local/bin when you leave this screen."
            } else if !new_binary {
                "\n/usr/local/bin/foundry-opencode already exists and was not changed."
            } else {
                "\nCLI copy was skipped."
            },
            if trashed.is_empty() {
                String::new()
            } else {
                format!(
                    "\nMoved older applications to Trash:\n{}",
                    trashed
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            },
        ),
    )?;
    Ok(binary)
}

fn update(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let config = home()?.join(".config/opencode");
    let mut can_pull = true;
    let details = if dry_run {
        let state = match ops::check_checkout(&config) {
            Ok(()) => "Checkout is clean and on main.".to_string(),
            Err(error) => {
                can_pull = false;
                format!("Config pull would be blocked: {error}")
            }
        };
        format!(
            "Would run brew update; install/upgrade opencode and direnv; upgrade OpenChamber if installed.\n{} in {}.\n{state}\n\nNo modifying commands ran or files changed.",
            if can_pull {
                "Would run git pull --ff-only origin main"
            } else {
                "Would skip git pull"
            },
            config.display()
        )
    } else {
        ops::check_checkout(&config)?;
        format!(
            "Upgrade OpenCode and direnv, upgrade OpenChamber if installed, and fast-forward the config checkout in {} from main?",
            config.display()
        )
    };
    eyre::ensure!(
        ui::step_review(terminal, "Review update", &details, dry_run, true, "Update")?
            == ui::Nav::Next,
        "Cancelled"
    );
    if dry_run {
        simulate(
            terminal,
            &[
                ("refresh Homebrew metadata".to_string(), 300),
                ("install or upgrade OpenCode".to_string(), 400),
                ("upgrade OpenChamber if installed".to_string(), 300),
                ("install or upgrade direnv".to_string(), 300),
                (
                    if can_pull {
                        "fast-forward the config checkout from main"
                    } else {
                        "skip config pull (checkout blocked)"
                    }
                    .to_string(),
                    if can_pull { 500 } else { 180 },
                ),
            ],
        )?;
        return notice(terminal, "Dry-run complete", &details);
    }
    let inventory = cleanup::inspect(&home()?, false)?;
    eyre::ensure!(
        !inventory.brew_v2
            && inventory.brew_v1.is_empty()
            && inventory.npm_v1.is_empty()
            && inventory.files.is_empty()
            && inventory.unknown.is_empty(),
        "Older or non-Homebrew OpenCode installation detected; run install to review migration before updating"
    );
    let chamber = installed("openchamber", true)?;
    let mut progress = Progress {
        done: 0,
        total: 4 + usize::from(chamber),
    };
    brew_update(terminal, &mut progress)?;
    cleanup::latest_is_v2()?;
    brew_package(terminal, &mut progress, "opencode", false)?;
    if chamber {
        brew_package(terminal, &mut progress, "openchamber", true)?;
    }
    brew_package(terminal, &mut progress, "direnv", false)?;
    let mut git = Process::new("git");
    git.arg("-C")
        .arg(&config)
        .args(["pull", "--ff-only", "origin", "main"]);
    run(terminal, &mut progress, "Updating configs", git)?;
    notice(
        terminal,
        "Update — complete",
        "Packages and configs are up to date.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_select_install() {
        let args = Args::try_parse_from(["foundry-opencode"]).unwrap();
        assert!(matches!(
            args.command.unwrap_or(Mode::Install),
            Mode::Install
        ));
    }

    #[test]
    fn dry_run_works_before_and_after_commands() {
        for args in [
            vec!["foundry-opencode", "--dry-run"],
            vec!["foundry-opencode", "--dry-run", "init"],
            vec!["foundry-opencode", "init", "--dry-run"],
            vec!["foundry-opencode", "update", "--dry-run"],
        ] {
            assert!(Args::try_parse_from(args).unwrap().dry_run);
        }
    }

    #[test]
    fn token_page_is_on_the_validated_host() {
        assert_eq!(
            token_page("https://example.palantirfoundry.com"),
            "https://example.palantirfoundry.com/workspace/settings/tokens"
        );
        assert!(!ops::valid_url("https://example.palantirfoundry.com/other"));
        let preview = preview_envrc(
            std::path::Path::new("/stack/.envrc"),
            "https://example.palantirfoundry.com",
        );
        assert!(preview.contains("export OPENCODE_API_KEY='[REDACTED]'"));
        assert!(preview.contains("export OPENCODE_BASE_URL='https://example.palantirfoundry.com'"));
    }

    #[test]
    fn install_pages_skip_existing_envrc_but_keep_selected_stacks() {
        let root = ops::backup_path(&std::env::temp_dir().join("foundry-pages"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("existing")).unwrap();
        fs::write(root.join("existing/.envrc"), "preserve").unwrap();
        let selected = vec!["existing".into(), "new".into()];
        let pages = install_pages(true, true, &root, &selected);
        assert!(matches!(pages.first(), Some(InstallPage::Options)));
        assert!(
            pages
                .iter()
                .any(|page| matches!(page, InstallPage::Host(name) if name == "new"))
        );
        assert!(
            !pages
                .iter()
                .any(|page| matches!(page, InstallPage::Host(name) if name == "existing"))
        );
        assert!(matches!(pages.last(), Some(InstallPage::Review)));
        fs::remove_dir_all(root).unwrap();
    }
}
