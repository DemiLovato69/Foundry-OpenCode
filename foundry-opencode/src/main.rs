mod cleanup;
mod ops;
mod rag;
mod stack;
mod ui;

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
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

#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cancelled")
    }
}

impl std::error::Error for Cancelled {}

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

#[derive(Clone, Subcommand)]
enum Mode {
    /// Generate per-stack OpenCode configuration
    Init,
    /// Configure and index RAG in the current project
    InitRag,
    #[command(hide = true)]
    ApplyRag { project: PathBuf },
    /// Retire a verified V1 RAG plugin in the current project
    RepairPlugins,
    /// Update tools, configs and selected RAG projects
    Update,
    /// Remove installed programs, configs, stack credentials and user data
    Uninstall,
    /// Stop OpenCode and OpenChamber and clear the OpenCode cache
    Reset,
    /// Install tools, configs and optionally RAG (on by default for selected projects)
    Install,
    /// Print the installer version
    Version,
    /// Show example commands
    Usage,
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    if let Some(Mode::ApplyRag { project }) = &args.command {
        color_eyre::install()?;
        eyre::ensure!(!args.dry_run, "apply-rag does not run during dry-run");
        let (url, token) = stack::credentials(project)?;
        rag::setup(project, &url, false, |label, mut command| {
            eyre::ensure!(
                command.env("OPENAI_API_KEY", &token).status()?.success(),
                "{label} failed for {}",
                project.display()
            );
            Ok(())
        })?;
        register_rag(project)?;
        return Ok(());
    }
    match args.command {
        Some(Mode::Version) => {
            println!("{}", version());
            return Ok(());
        }
        Some(Mode::Usage) => {
            println!(
                "foundry-opencode           Install (same as install)\nfoundry-opencode install   Install tools; select stacks and optional RAG projects (on by default)\nfoundry-opencode update    Update tools, configs and selected RAG projects\nfoundry-opencode repair-plugins Retire verified V1 RAG plugin in current project\nfoundry-opencode uninstall Remove tools, configs, credentials and user data\nfoundry-opencode reset     Stop OpenCode/OpenChamber and clear OpenCode cache\nfoundry-opencode init      Create per-stack OpenCode config\nfoundry-opencode init-rag  Configure and index RAG in the current project\nfoundry-opencode --dry-run uninstall Preview full removal\nfoundry-opencode --dry-run update  Preview credential, config and RAG changes\nfoundry-opencode version   Print version\nfoundry-opencode usage     Show these examples"
            );
            return Ok(());
        }
        _ => {}
    }
    color_eyre::install()?;
    if !cfg!(target_os = "macos")
        && !args.dry_run
        && !matches!(
            args.command,
            Some(Mode::Init | Mode::InitRag | Mode::RepairPlugins | Mode::Reset)
        )
    {
        eyre::bail!("Installation and updates require macOS and Homebrew");
    }
    let staged = match ratatui::run(|terminal| {
        execute!(std::io::stdout(), EnableMouseCapture)?;
        let result = match args.command.clone().unwrap_or(Mode::Install) {
            Mode::Init => init(terminal, args.dry_run).map(|_| None),
            Mode::InitRag => init_rag(terminal, args.dry_run).map(|_| None),
            Mode::RepairPlugins => repair_plugins(terminal, args.dry_run).map(|_| None),
            Mode::Update => update(terminal, args.dry_run),
            Mode::Uninstall => uninstall(terminal, args.dry_run).map(|_| None),
            Mode::Reset => reset(terminal, args.dry_run).map(|_| None),
            Mode::Install => install(terminal, args.dry_run),
            _ => unreachable!(),
        };
        execute!(std::io::stdout(), DisableMouseCapture)?;
        result
    }) {
        Ok(staged) => staged,
        Err(error) if error.is::<Cancelled>() => {
            println!("Cancelled; no changes were made.");
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if let Some(staged) = staged {
        ops::install_binary(&staged, std::path::Path::new(ops::BIN_PATH))?;
        println!("Installed foundry-opencode to {}", ops::BIN_PATH);
    }
    if matches!(args.command, Some(Mode::Uninstall)) && !args.dry_run {
        let binary = std::path::Path::new(ops::BIN_PATH);
        let mut binaries = vec![binary.to_path_buf()];
        if let Some(parent) = binary.parent()
            && let Ok(entries) = fs::read_dir(parent)
        {
            for entry in entries {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("foundry-opencode.backup-")
                {
                    binaries.push(entry.path());
                }
            }
        }
        for binary in binaries {
            if fs::symlink_metadata(&binary).is_err() {
                continue;
            }
            match remove_safe(&binary) {
                Ok(()) => {}
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied) =>
                {
                    let script =
                        "if [ -L \"$1\" ] || [ ! -f \"$1\" ]; then exit 1; fi; /bin/rm -f \"$1\"";
                    eyre::ensure!(
                        Process::new("sudo")
                            .args(["/bin/sh", "-c", script, "uninstall"])
                            .arg(&binary)
                            .status()?
                            .success(),
                        "Could not remove installer binary"
                    );
                }
                Err(error) => return Err(error),
            }
            println!("Removed {}", binary.display());
        }
    }
    Ok(())
}

fn init_rag(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let cwd = std::env::current_dir()?;
    let (url, token) = stack::credentials(&cwd)?;
    let preview = rag::setup(&cwd, &url, true, |_, _| unreachable!())?;
    if ui::step_review(
        terminal,
        "Review RAG setup",
        &preview,
        dry_run,
        true,
        "Initialize RAG",
    )? != ui::Nav::Next
    {
        return Err(Cancelled.into());
    }
    if dry_run {
        return notice(terminal, "Dry-run complete", &preview);
    }
    let mut progress = Progress {
        done: 0,
        total: 2,
        output: Vec::new(),
    };
    progress.show(terminal, "Preparing RAG files")?;
    let result = rag::setup(&cwd, &url, false, |label, mut command| {
        command.env("OPENAI_API_KEY", &token);
        run(terminal, &mut progress, label, command)
    })?;
    register_rag(&cwd)?;
    notice(terminal, "RAG — complete", &result)
}

fn register_rag(project: &std::path::Path) -> eyre::Result<()> {
    let registry = home()?.join(".config/foundry-opencode/rag.json");
    let mut projects = load_paths(&registry)?;
    let path = project.canonicalize()?;
    if !projects.contains(&path) {
        projects.push(path);
        save_paths(&registry, &projects)?;
    }
    Ok(())
}

fn select_rag(
    terminal: &mut DefaultTerminal,
    projects: &[PathBuf],
    choices: &mut std::collections::BTreeMap<PathBuf, bool>,
) -> eyre::Result<ui::Nav> {
    let mut page = 0;
    while page < projects.len() {
        let project = &projects[page];
        match ui::step_checkbox(
            terminal,
            &format!("RAG — {}", project.display()),
            "Install or update this folder's RAG and index? Existing RAG files are backed up; the index is preserved.",
            "Set up RAG",
            choices.entry(project.clone()).or_insert(true),
            false,
        )? {
            ui::Nav::Next => page += 1,
            ui::Nav::Back if page > 0 => page -= 1,
            ui::Nav::Back => return Ok(ui::Nav::Back),
            ui::Nav::Cancel => return Ok(ui::Nav::Cancel),
            ui::Nav::Open => {}
        }
    }
    Ok(ui::Nav::Next)
}

fn rag_targets(
    projects: &[PathBuf],
    choices: &std::collections::BTreeMap<PathBuf, bool>,
) -> Vec<PathBuf> {
    projects
        .iter()
        .filter(|path| *choices.get(*path).unwrap_or(&true))
        .cloned()
        .collect()
}

fn revisit_rag(
    terminal: &mut DefaultTerminal,
    home: &std::path::Path,
    current: &mut PathBuf,
    projects: &mut Vec<PathBuf>,
    choices: &mut std::collections::BTreeMap<PathBuf, bool>,
) -> eyre::Result<ui::Nav> {
    loop {
        if select_rag(terminal, projects, choices)? == ui::Nav::Next {
            return Ok(ui::Nav::Next);
        }
        if ui::step_project_explorer(terminal, home, current, projects)? != ui::Nav::Next {
            return Ok(ui::Nav::Back);
        }
    }
}

fn validate_rag_projects(
    projects: &[PathBuf],
    stacks: &[PathBuf],
    global: bool,
    config: &std::path::Path,
) -> eyre::Result<()> {
    let user_home = home()?;
    for project in projects {
        eyre::ensure!(
            project.is_dir()
                && project.canonicalize().is_ok_and(|path| path == *project)
                && project != std::path::Path::new("/")
                && project != &user_home
                && !project.starts_with(config)
                && (global
                    || stacks.iter().any(|stack| project.starts_with(stack))
                    || (project.ancestors().any(|path| {
                        path.join(".opencode/token").is_file()
                            && path.join(".opencode/opencode.json").is_file()
                    }) && stack::credentials(project).is_ok())),
            "RAG project {} needs a selected parent stack or Foundry credentials",
            project.display()
        );
    }
    Ok(())
}

fn repair_plugins(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let project = std::env::current_dir()?;
    let Some(path) = rag::legacy_plugin(&project)? else {
        return notice(
            terminal,
            "No known V1 RAG plugin",
            "No installer-known V1 RAG plugin was found in this project. Other plugins were not changed.",
        );
    };
    let backup = ops::backup_path(&project.join(".opencode/rag-plugin.js"));
    let detail = format!(
        "The known V1 RAG watcher at {} fails to load in OpenCode V2. Move it out of the auto-loaded plugins folder to {}. Existing index and RAG MCP configuration will be preserved. Reindex manually with node .opencode/rag-cli.mjs index if that helper exists.",
        path.display(),
        backup.display()
    );
    if ui::step_review(
        terminal,
        "Repair V1 RAG plugin",
        &detail,
        dry_run,
        true,
        "Retire V1 plugin",
    )? != ui::Nav::Next
    {
        return Err(Cancelled.into());
    }
    if dry_run {
        return notice(terminal, "Dry-run complete", &detail);
    }
    eyre::ensure!(
        rag::legacy_plugin(&project)?.as_ref() == Some(&path),
        "RAG plugin changed during review; not moving it"
    );
    fs::rename(&path, &backup)?;
    notice(
        terminal,
        "Plugin repair — complete",
        &format!(
            "V1 RAG plugin backed up at {}. Restart only if OpenCode does not reload automatically. If this project has no RAG MCP helper, run init-rag from the project root to configure it.",
            backup.display()
        ),
    )
}

fn token_page(url: &str) -> String {
    format!("{url}/workspace/settings/tokens")
}

fn preview_stack(path: &std::path::Path, url: &str) -> String {
    format!(
        "Would replace or create {}/.opencode/opencode.json for {url} and private, Git-ignored .opencode/token (mode 0600, token redacted; existing files backed up)",
        path.display(),
    )
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
    if fs::symlink_metadata(cwd.join(".envrc")).is_ok() {
        loop {
            let Some((supplied, archive)) = migration_choice(terminal, &cwd, dry_run)? else {
                return Err(Cancelled.into());
            };
            let preview = stack::migrate(
                &cwd,
                true,
                supplied
                    .as_ref()
                    .map(|(url, key)| (url.as_str(), key.as_str())),
                archive,
            )?;
            match ui::step_review(
                terminal,
                "Migrate stack",
                &preview,
                dry_run,
                supplied.is_none(),
                "Migrate",
            )? {
                ui::Nav::Back => continue,
                ui::Nav::Next => {}
                _ => return Err(Cancelled.into()),
            }
            let result = if dry_run {
                preview
            } else {
                let message = stack::migrate(
                    &cwd,
                    false,
                    supplied
                        .as_ref()
                        .map(|(url, key)| (url.as_str(), key.as_str())),
                    archive,
                )?;
                register_stack(&cwd)?;
                message
            };
            return notice(
                terminal,
                if dry_run {
                    "Dry-run complete"
                } else {
                    "init — complete"
                },
                &result,
            );
        }
    }
    let (mut url, mut key) = stack::credentials(&cwd).unwrap_or_default();
    let mut step = 0usize;
    loop {
        let nav = match step {
            0 => ui::step_text(
                terminal,
                "Foundry host",
                "Enter your stack's HTTPS Foundry host (for example https://your-stack.palantirfoundry.com). Run init from the stack directory whose child projects should inherit this configuration.",
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
                let mut review = preview_stack(&cwd, &url);
                if dry_run {
                    review.push_str("\n\nDry run: no browser or files changed.");
                }
                ui::step_review(
                    terminal,
                    if dry_run {
                        "Review preview"
                    } else {
                        "Review stack config"
                    },
                    &review,
                    dry_run,
                    false,
                    "Create stack config",
                )?
            }
        };
        match nav {
            ui::Nav::Cancel => return Err(Cancelled.into()),
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
        let steps = vec![(
            "replace or create stack config and private token".to_string(),
            350,
        )];
        simulate(terminal, &steps)?;
        return notice(
            terminal,
            "Dry-run complete",
            &format!(
                "{}\n\nSimulation only: no file was created.",
                preview_stack(&cwd, &url),
            ),
        );
    }
    stack::replace(&cwd, &url, &key, false)?;
    register_stack(&cwd)?;
    notice(
        terminal,
        "init — complete",
        "Replaced or created per-stack OpenCode config and private token. OpenCode projects beneath this directory inherit the stack settings.",
    )
}

type MigrationChoice = (Option<(String, String)>, bool);

fn migration_choice(
    terminal: &mut DefaultTerminal,
    path: &std::path::Path,
    dry_run: bool,
) -> eyre::Result<Option<MigrationChoice>> {
    if stack::legacy_credentials(path).is_ok() {
        return Ok(Some((None, false)));
    }
    let mut url = String::new();
    let mut key = String::new();
    let mut archive = true;
    let mut page = 0;
    loop {
        let nav = match page {
            0 => ui::step_text(
                terminal,
                "Unrecognized .envrc — Foundry host",
                &format!(
                    "{} cannot be parsed safely. Enter its HTTPS Foundry host; shell contents will not be executed.",
                    path.display()
                ),
                &mut url,
                false,
                ui::Field::Plain,
            )?,
            1 => ui::step_text(
                terminal,
                "Unrecognized .envrc — token",
                "Enter this stack's token, or open the token page",
                &mut key,
                false,
                ui::Field::Token,
            )?,
            _ => ui::step_checkbox(
                terminal,
                "Retire old .envrc",
                "After the new config is verified, archive the old .envrc (recommended), or uncheck to delete it.",
                "Archive old .envrc",
                &mut archive,
                false,
            )?,
        };
        match nav {
            ui::Nav::Back if page > 0 => page -= 1,
            ui::Nav::Back => return Ok(None),
            ui::Nav::Cancel => return Err(Cancelled.into()),
            ui::Nav::Open => open_token_page(terminal, &url, dry_run)?,
            ui::Nav::Next if page == 0 && !ops::valid_url(&url) => {
                notice(
                    terminal,
                    "Invalid host",
                    "Enter an HTTPS Foundry host without a path.",
                )?;
            }
            ui::Nav::Next if page == 1 && key.is_empty() && !dry_run => {
                notice(terminal, "Token required", "Enter a Foundry token.")?;
            }
            ui::Nav::Next if page == 2 => break,
            ui::Nav::Next => page += 1,
        }
    }
    if dry_run && key.is_empty() {
        key = "dry-run-placeholder".into();
    }
    eyre::ensure!(!key.is_empty(), "Token required to migrate stack");
    Ok(Some((Some((url, key)), archive)))
}

// shortcut: external commands report no percentages, replace the spinner when they expose progress.
struct Progress {
    done: usize,
    total: usize,
    output: Vec<String>,
}

impl Progress {
    fn show(&self, terminal: &mut DefaultTerminal, label: &str) -> eyre::Result<()> {
        terminal.draw(|frame| {
            ui::progress_with_output(frame, label, self.done, self.total, &self.output)
        })?;
        Ok(())
    }

    fn line(&mut self, bytes: &[u8]) -> bool {
        let text = String::from_utf8_lossy(bytes);
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .take(500)
            .collect();
        if !text.trim().is_empty() {
            self.output.push(text);
            if self.output.len() > 200 {
                self.output.remove(0);
            }
            return true;
        }
        false
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

fn busy<T: Send + 'static>(
    terminal: &mut DefaultTerminal,
    label: &str,
    work: impl FnOnce() -> eyre::Result<T> + Send + 'static,
) -> eyre::Result<T> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    let mut tick = 0;
    let spinner = ['|', '/', '-', '\\'];
    loop {
        terminal
            .draw(|frame| ui::progress(frame, &format!("{label} {}", spinner[tick % 4]), 0, 1))?;
        tick += 1;
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eyre::bail!("{label} stopped unexpectedly")
            }
        }
    }
}

fn run(
    terminal: &mut DefaultTerminal,
    progress: &mut Progress,
    label: &str,
    command: Process,
) -> eyre::Result<()> {
    run_limited(terminal, progress, label, command, None)
}

fn stream_output(mut stream: impl Read, tx: mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0; 2048];
    let mut line = Vec::new();
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                for byte in &buffer[..count] {
                    if *byte == b'\r' || *byte == b'\n' {
                        if tx.send(std::mem::take(&mut line)).is_err() {
                            return;
                        }
                    } else if line.len() < 4096 {
                        // shortcut: keep only 4096 bytes of a command line; raise for larger diagnostics.
                        line.push(*byte);
                    }
                }
            }
        }
    }
    if !line.is_empty() {
        let _ = tx.send(line);
    }
}

fn run_limited(
    terminal: &mut DefaultTerminal,
    progress: &mut Progress,
    label: &str,
    mut command: Process,
    timeout: Option<Duration>,
) -> eyre::Result<()> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .wrap_err(format!("{label} failed to start"))?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let streams: [Box<dyn std::io::Read + Send>; 2] = [
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    for stream in streams {
        let tx = tx.clone();
        std::thread::spawn(move || stream_output(stream, tx));
    }
    drop(tx);
    let mut tick = 0;
    let spinner = ['|', '/', '-', '\\'];
    let mut status = None;
    let started = Instant::now();
    let mut last_draw = started - Duration::from_millis(120);
    let mut recent = Vec::new();
    loop {
        if last_draw.elapsed() >= Duration::from_millis(120) {
            progress.show(
                terminal,
                &format!("{label}  {}", spinner[tick % spinner.len()]),
            )?;
            tick += 1;
            last_draw = Instant::now();
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_none() && timeout.is_some_and(|limit| started.elapsed() >= limit) {
            child.kill()?;
            child.wait()?;
            eyre::bail!("{label} timed out");
        }
        match rx.recv_timeout(Duration::from_millis(120)) {
            Ok(line) => {
                if progress.line(&line)
                    && let Some(line) = progress.output.last()
                {
                    recent.push(line.clone());
                    if recent.len() > 8 {
                        recent.remove(0);
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let status = match status {
                    Some(status) => status,
                    None => child.wait()?,
                };
                progress.show(terminal, label)?;
                if !status.success() {
                    eyre::bail!("{label} failed ({status}): {}", recent.join("\n"));
                }
                progress.done += 1;
                progress.show(terminal, label)?;
                return Ok(());
            }
        }
    }
}

fn run_interactive_git(
    terminal: &mut DefaultTerminal,
    progress: &mut Progress,
    label: &str,
    mut command: Process,
    mut retry: Process,
    retry_directory: Option<&std::path::Path>,
) -> eyre::Result<()> {
    command.env("GIT_TERMINAL_PROMPT", "0");
    match run(terminal, progress, label, command) {
        Ok(()) => return Ok(()),
        Err(error)
            if [
                "terminal prompts disabled",
                "Authentication failed",
                "could not read Username",
                "could not read Password",
            ]
            .iter()
            .any(|text| error.to_string().contains(text)) => {}
        Err(error) => return Err(error),
    }
    if let Some(path) = retry_directory {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                fs::remove_dir_all(path)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => eyre::bail!(
                "Cannot retry Git authentication; inspect {}",
                path.display()
            ),
        }
    }
    execute!(std::io::stdout(), DisableMouseCapture)?;
    ratatui::try_restore()?;
    eyre::ensure!(
        !crossterm::terminal::is_raw_mode_enabled()?,
        "Cannot prompt for Git credentials while the terminal is in raw mode"
    );
    println!("{label}.");
    let result = retry.status();
    *terminal = ratatui::try_init()?;
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let status = result.wrap_err(format!("{label} failed to start"))?;
    eyre::ensure!(status.success(), "{label} failed ({status})");
    progress.done += 1;
    progress.show(terminal, label)
}

fn installed(name: &str, cask: bool) -> eyre::Result<bool> {
    let mut command = Process::new("brew");
    command.arg("list");
    if cask {
        command.arg("--cask");
    }
    match command.arg("--versions").arg(name).output() {
        Ok(output) => Ok(output.status.success()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
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

fn chamber_running() -> eyre::Result<bool> {
    let output = Process::new("pgrep").args(["-x", "OpenChamber"]).output()?;
    chamber_status(output.status.code())
}

fn chamber_status(code: Option<i32>) -> eyre::Result<bool> {
    match code {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => eyre::bail!(
            "Could not determine whether OpenChamber is running; skipping its upgrade is safer"
        ),
    }
}

fn chamber_available(terminal: &mut DefaultTerminal) -> bool {
    match chamber_running() {
        Ok(false) => true,
        Ok(true) => {
            let _ = notice(
                terminal,
                "OpenChamber skipped",
                "OpenChamber is running. Quit it and rerun install/update to upgrade it. The remaining setup will continue.",
            );
            false
        }
        Err(error) => {
            let _ = notice(
                terminal,
                "OpenChamber skipped",
                &format!("{error}. The remaining setup will continue."),
            );
            false
        }
    }
}

fn optional_chamber(terminal: &mut DefaultTerminal, progress: &mut Progress) -> bool {
    if chamber_available(terminal) {
        if let Err(error) = brew_package(terminal, progress, "openchamber", true) {
            let _ = notice(
                terminal,
                "OpenChamber upgrade skipped",
                &format!("{error}. Continue setup; retry after quitting OpenChamber."),
            );
            progress.done += 1;
            return false;
        }
    } else {
        progress.done += 1;
        return false;
    }
    true
}

fn restore_chamber(backup: &std::path::Path, app: &std::path::Path) -> eyre::Result<()> {
    eyre::ensure!(
        fs::symlink_metadata(app).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "OpenChamber upgrade failed; previous app is safe at {} but {} is occupied",
        backup.display(),
        app.display()
    );
    fs::rename(backup, app).wrap_err_with(|| {
        format!(
            "OpenChamber upgrade failed; restore the previous app from {}",
            backup.display()
        )
    })?;
    Ok(())
}

fn home() -> eyre::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or_else(|| eyre::eyre!("HOME is not set"))?,
    ))
}

fn stack_registry(home: &std::path::Path) -> PathBuf {
    home.join(".config/foundry-opencode/stacks.json")
}

fn load_stacks(home: &std::path::Path) -> eyre::Result<Vec<PathBuf>> {
    load_paths(&stack_registry(home))
}

fn load_paths(path: &std::path::Path) -> eyre::Result<Vec<PathBuf>> {
    match fs::read(path) {
        Ok(data) => Ok(serde_json::from_slice(&data)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn save_stacks(home: &std::path::Path, paths: &[PathBuf]) -> eyre::Result<()> {
    save_paths(&stack_registry(home), paths)
}

fn register_stack(stack: &std::path::Path) -> eyre::Result<()> {
    let user_home = home()?;
    let mut stacks = load_stacks(&user_home)?;
    let canonical = stack.canonicalize()?;
    if !stacks.contains(&canonical) {
        stacks.push(canonical);
        save_stacks(&user_home, &stacks)?;
    }
    Ok(())
}

fn save_paths(path: &std::path::Path, paths: &[PathBuf]) -> eyre::Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let staged = ops::backup_path(path);
    fs::write(&staged, serde_json::to_vec_pretty(paths)?)?;
    fs::rename(staged, path)?;
    Ok(())
}

fn remove_direnv(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    if dry_run {
        return Ok(());
    }
    let user_home = home()?;
    let zsh = std::env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.clone())
        .join(".zshrc");
    cleanup::clean_direnv_text(&zsh, false)?;
    if zsh != user_home.join(".zshrc") {
        cleanup::clean_direnv_text(&user_home.join(".zshrc"), false)?;
    }
    cleanup::clean_direnv_text(&user_home.join(".config/direnv/direnv.toml"), true)?;
    if installed("direnv", false)? {
        let mut progress = Progress {
            done: 0,
            total: 1,
            output: Vec::new(),
        };
        let mut command = Process::new("brew");
        command.args(["uninstall", "direnv"]);
        run(terminal, &mut progress, "Uninstalling direnv", command)?;
    }
    for path in [
        user_home.join(".config/direnv"),
        user_home.join(".local/share/direnv"),
        user_home.join(".local/state/direnv"),
        user_home.join(".cache/direnv"),
    ] {
        remove_safe(&path)?;
    }
    for path in [
        user_home.join(".local/bin/direnv"),
        user_home.join("bin/direnv"),
    ] {
        remove_safe(&path)?;
    }
    Ok(())
}

fn remove_safe(path: &std::path::Path) -> eyre::Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(meta) => {
            eyre::ensure!(
                !meta.file_type().is_symlink(),
                "Refusing to remove symlink {}",
                path.display()
            );
            if meta.is_dir() {
                fs::remove_dir_all(path)?;
            } else if meta.is_file() {
                fs::remove_file(path)?;
            } else {
                eyre::bail!("Unsupported file type at {}", path.display());
            }
            Ok(())
        }
    }
}

fn uninstall(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let user_home = home()?;
    // shortcut: older RAG projects have no registry; add safe discovery if historical projects must be wiped automatically.
    let mut selected = load_stacks(&user_home)?;
    selected.retain(|path| path.exists());
    let mut projects = load_paths(&user_home.join(".config/foundry-opencode/rag.json"))?;
    projects.retain(|path| path.exists());
    let mut current = user_home.canonicalize()?;
    let (targets, packages, npm_packages) = 'wizard: loop {
        if ui::step_initial_explorer(terminal, &user_home, &mut current, &mut selected)?
            != ui::Nav::Next
        {
            return Err(Cancelled.into());
        }
        let config = user_home.join(".config/opencode");
        let mut targets = vec![
            config.clone(),
            user_home.join(".config/foundry-opencode"),
            user_home.join(".config/direnv"),
            user_home.join(".local/share/direnv"),
            user_home.join(".local/state/direnv"),
            user_home.join(".cache/direnv"),
            user_home.join(".local/bin/direnv"),
            user_home.join("bin/direnv"),
            user_home.join(".cache/opencode"),
            user_home.join(".local/share/opencode"),
            user_home.join(".local/state/opencode"),
            user_home.join("Library/Application Support/opencode"),
            user_home.join("Library/Caches/opencode"),
            user_home.join("Library/Logs/opencode"),
            user_home.join("Library/Application Support/OpenChamber"),
            user_home.join("Library/Application Support/openchamber"),
            user_home.join(".config/openchamber"),
            user_home.join("Library/Caches/OpenChamber"),
            user_home.join("Library/Logs/OpenChamber"),
        ];
        if let Ok(entries) = fs::read_dir(config.parent().unwrap()) {
            for entry in entries {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("opencode.backup-")
                {
                    targets.push(entry.path());
                }
            }
        }
        if let Ok(entries) = fs::read_dir(user_home.join(".Trash")) {
            for entry in entries {
                let entry = entry?;
                if ["opencode.backup-", "OpenChamber.app.backup-"]
                    .iter()
                    .any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix))
                {
                    targets.push(entry.path());
                }
            }
        }
        for stack in &selected {
            eyre::ensure!(
                stack.canonicalize().is_ok_and(|path| path == *stack)
                    && stack != &user_home
                    && stack != std::path::Path::new("/"),
                "Invalid saved stack path {}",
                stack.display()
            );
            eyre::ensure!(
                stack != &config && !stack.starts_with(user_home.join(".config/foundry-opencode")),
                "Managed config directory cannot be a stack"
            );
            let open = stack.join(".opencode");
            eyre::ensure!(
                fs::symlink_metadata(&open)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                    || fs::symlink_metadata(&open)
                        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink()),
                "Unsafe stack config directory {}",
                open.display()
            );
            for name in [
                ".envrc",
                ".opencode/token",
                ".opencode/opencode.json",
                ".opencode/opencode.jsonc",
            ] {
                targets.push(stack.join(name));
            }
            for entry in fs::read_dir(stack)? {
                let entry = entry?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".envrc.backup-")
                {
                    targets.push(entry.path());
                }
            }
            if let Ok(entries) = fs::read_dir(&open) {
                for entry in entries {
                    let entry = entry?;
                    if [
                        "opencode.json.backup-",
                        "opencode.jsonc.backup-",
                        "token.backup-",
                    ]
                    .iter()
                    .any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix))
                    {
                        targets.push(entry.path());
                    }
                }
            }
        }
        for project in &projects {
            eyre::ensure!(
                project.canonicalize().is_ok_and(|path| path == *project),
                "Invalid saved RAG project {}",
                project.display()
            );
            let dir = project.join(".opencode");
            eyre::ensure!(
                fs::symlink_metadata(&dir)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                    || fs::symlink_metadata(&dir)
                        .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink()),
                "Unsafe RAG directory {}",
                dir.display()
            );
            for name in [
                "rag.json",
                "rag-cli.mjs",
                "patch-opencode-rag.mjs",
                "package.json",
                "package-lock.json",
                ".gitignore",
                "node_modules",
                "rag_db",
                "rag_db_tmp",
                "opencode-rag.log",
                "runtime-overrides.json",
            ] {
                targets.push(dir.join(name));
            }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries {
                    let entry = entry?;
                    if [
                        "rag.json.backup-",
                        "rag-cli.mjs.backup-",
                        "patch-opencode-rag.mjs.backup-",
                        "package.json.backup-",
                        "package-lock.json.backup-",
                        ".gitignore.backup-",
                        "opencode.json.backup-",
                        "opencode.jsonc.backup-",
                    ]
                    .iter()
                    .any(|prefix| entry.file_name().to_string_lossy().starts_with(prefix))
                    {
                        targets.push(entry.path());
                    }
                }
            }
        }
        let packages = ["opencode", "opencode-v1", "opencode-v2", "direnv"];
        let npm_packages = cleanup::npm_opencode()?;
        let mut review = format!(
            "PERMANENT FULL WIPE: sessions, credentials, caches and config backups will be deleted.\nRemove CLI: {}\nHomebrew: {:?}; OpenChamber cask (if installed).\nGlobal npm: {:?}.\nRemove direnv hooks from .zshrc (also ZDOTDIR) and generated stack .gitignore entries.\nSelected stacks: {}; RAG projects: {}\nPaths:\n{}\n\nOther files inside project/stack directories are preserved.",
            ops::BIN_PATH,
            packages,
            npm_packages,
            selected.len(),
            projects.len(),
            targets
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
        if dry_run {
            review.push_str("\nDry run: no changes.");
        }
        loop {
            match ui::step_review(
                terminal,
                "Confirm full uninstall",
                &review,
                dry_run,
                false,
                "Wipe everything",
            )? {
                ui::Nav::Back => continue 'wizard,
                ui::Nav::Next => {}
                _ => return Err(Cancelled.into()),
            }
            if dry_run {
                return notice(terminal, "Dry-run complete", &review);
            }
            let mut confirmed = false;
            match ui::step_checkbox(
                terminal,
                "Confirm irreversible removal",
                "Sessions, credentials and backups will be permanently deleted. Confirm you have reviewed the paths and wish to continue.",
                "I understand this cannot be undone",
                &mut confirmed,
                false,
            )? {
                ui::Nav::Back => continue,
                ui::Nav::Next if confirmed => break 'wizard (targets, packages, npm_packages),
                ui::Nav::Next => notice(
                    terminal,
                    "Confirmation required",
                    "Review and check the confirmation box to proceed.",
                )?,
                _ => return Err(Cancelled.into()),
            }
        }
    };
    let mut service = Process::new("opencode");
    service.args(["service", "stop"]);
    let mut progress = Progress {
        done: 0,
        total: 1,
        output: Vec::new(),
    };
    let _ = run_limited(
        terminal,
        &mut progress,
        "Stopping OpenCode service",
        service,
        Some(Duration::from_secs(5)),
    );
    let id = Process::new("id").arg("-u").output()?;
    eyre::ensure!(id.status.success(), "Could not determine current user");
    let uid = String::from_utf8(id.stdout)?.trim().parse::<u32>()?;
    let processes = running_targets(uid)?;
    for pid in &processes {
        signal_target(*pid, uid, "-TERM")?;
    }
    if !processes.is_empty() {
        std::thread::sleep(Duration::from_secs(1));
    }
    for pid in running_targets(uid)?
        .into_iter()
        .filter(|pid| processes.contains(pid))
    {
        signal_target(pid, uid, "-KILL")?;
    }
    eyre::ensure!(
        running_targets(uid)?.is_empty(),
        "OpenCode/OpenChamber processes still running; no configs removed"
    );
    remove_direnv(terminal, false)?;
    for name in packages {
        if installed(name, false)? {
            let mut progress = Progress {
                done: 0,
                total: 1,
                output: Vec::new(),
            };
            let mut command = Process::new("brew");
            command.args(["uninstall", name]);
            run(
                terminal,
                &mut progress,
                &format!("Uninstalling {name}"),
                command,
            )?;
        }
    }
    for name in npm_packages {
        let mut progress = Progress {
            done: 0,
            total: 1,
            output: Vec::new(),
        };
        let mut command = Process::new("npm");
        command.args(["uninstall", "-g", &name]);
        run(
            terminal,
            &mut progress,
            &format!("Uninstalling global npm {name}"),
            command,
        )?;
    }
    if installed("openchamber", true)? {
        let mut progress = Progress {
            done: 0,
            total: 1,
            output: Vec::new(),
        };
        let mut command = Process::new("brew");
        command.args(["uninstall", "--cask", "openchamber"]);
        run(terminal, &mut progress, "Uninstalling OpenChamber", command)?;
    }
    for project in &projects {
        remove_rag_entry(project)?;
    }
    for path in targets {
        remove_safe(&path)?;
    }
    for stack in &selected {
        remove_stack_ignores(stack)?;
    }
    for stack in &selected {
        let dir = stack.join(".opencode");
        if fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none()) {
            fs::remove_dir(dir)?;
        }
    }
    for project in &projects {
        let dir = project.join(".opencode");
        if fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_none()) {
            fs::remove_dir(dir)?;
        }
    }
    notice(
        terminal,
        "Uninstall — complete",
        "Removed registered stack configs, direnv and known OpenCode/OpenChamber user data. Installer binary will be removed when you close this screen.",
    )
}

fn remove_rag_entry(project: &std::path::Path) -> eyre::Result<()> {
    for name in ["opencode.jsonc", "opencode.json"] {
        let path = project.join(".opencode").join(name);
        if !path.exists() {
            continue;
        }
        eyre::ensure!(
            fs::symlink_metadata(&path)?.is_file()
                && !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "Unsafe config {}",
            path.display()
        );
        let root = jsonc_parser::cst::CstRootNode::parse(
            &fs::read_to_string(&path)?,
            &Default::default(),
        )?;
        let object = root.object_value_or_set();
        if let Some(server) = object
            .get("mcp")
            .and_then(|prop| prop.object_value())
            .and_then(|mcp| mcp.get("servers"))
            .and_then(|prop| prop.object_value())
            .and_then(|servers| servers.get("rag"))
        {
            server.remove();
            let text = root.to_string();
            let doc: serde_json::Value =
                jsonc_parser::parse_to_serde_value(&text, &Default::default())?;
            if doc.as_object().is_some_and(|map| {
                map.iter().all(|(key, value)| {
                    key == "$schema"
                        || (key == "mcp"
                            && value["servers"]
                                .as_object()
                                .is_some_and(|servers| servers.is_empty()))
                })
            }) {
                fs::remove_file(path)?;
            } else {
                fs::write(path, text)?;
            }
        }
    }
    Ok(())
}

fn remove_stack_ignores(stack: &std::path::Path) -> eyre::Result<()> {
    let repo = Process::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(stack)
        .output()?;
    if !repo.status.success() {
        return Ok(());
    }
    let root = PathBuf::from(String::from_utf8(repo.stdout)?.trim());
    let ignore = root.join(".gitignore");
    let Ok(original) = fs::read_to_string(&ignore) else {
        return Ok(());
    };
    eyre::ensure!(
        fs::symlink_metadata(&ignore)?.is_file()
            && !fs::symlink_metadata(&ignore)?.file_type().is_symlink(),
        "Unsafe gitignore {}",
        ignore.display()
    );
    let relative = stack
        .canonicalize()?
        .strip_prefix(root.canonicalize()?)?
        .to_string_lossy()
        .to_string();
    let prefix = if relative.is_empty() {
        String::new()
    } else {
        format!("{relative}/")
    };
    let archive = format!("/{prefix}.envrc.backup-");
    let token = format!("/{prefix}.opencode/token");
    let token_backup = format!("/{prefix}.opencode/token.backup-");
    let updated = original
        .lines()
        .filter(|line| {
            *line != token && !line.starts_with(&archive) && !line.starts_with(&token_backup)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let updated = if updated.is_empty() {
        updated
    } else {
        format!("{updated}\n")
    };
    if updated != original {
        fs::write(ignore, updated)?;
    }
    Ok(())
}

fn reset_targets(list: &str, uid: u32) -> Vec<u32> {
    list.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let pid = parts.next()?.parse::<u32>().ok()?;
            let owner = parts.next()?.parse::<u32>().ok()?;
            let name = parts.collect::<Vec<_>>().join(" ");
            let name = name.rsplit('/').next()?;
            (owner == uid
                && pid != std::process::id()
                && (name == "opencode"
                    || name == "OpenCode"
                    || name.starts_with("OpenCode Helper")
                    || name == "OpenChamber"
                    || name == "openchamber"
                    || name.starts_with("OpenChamber Helper")))
            .then_some(pid)
        })
        .collect()
}

fn running_targets(uid: u32) -> eyre::Result<Vec<u32>> {
    let output = Process::new("ps")
        .args(["-axo", "pid=,uid=,comm="])
        .output()?;
    eyre::ensure!(
        output.status.success(),
        "Could not inspect running processes"
    );
    Ok(reset_targets(&String::from_utf8(output.stdout)?, uid))
}

fn signal_target(pid: u32, uid: u32, signal: &str) -> eyre::Result<()> {
    if !running_targets(uid)?.contains(&pid) {
        return Ok(());
    }
    let output = Process::new("kill")
        .args([signal, &pid.to_string()])
        .output()?;
    eyre::ensure!(
        output.status.success() || !running_targets(uid)?.contains(&pid),
        "Could not stop process {pid}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn clear_cache(path: &std::path::Path) -> eyre::Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
        Ok(metadata) => {
            eyre::ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "{} is not a real cache directory; not removing it",
                path.display()
            );
            fs::remove_dir_all(path)?;
            Ok(true)
        }
    }
}

fn reset(terminal: &mut DefaultTerminal, dry_run: bool) -> eyre::Result<()> {
    let cache = home()?.join(".cache/opencode");
    let details = format!(
        "Stop the current user's OpenCode service and OpenCode/OpenChamber processes, then remove {} if it exists. Active sessions will be interrupted. Configs, service registration, and session data are preserved.",
        cache.display()
    );
    if ui::step_review(terminal, "Review reset", &details, dry_run, true, "Reset")? != ui::Nav::Next
    {
        return Err(Cancelled.into());
    }
    if dry_run {
        return notice(terminal, "Dry-run complete", &details);
    }
    let mut progress = Progress {
        done: 0,
        total: 3,
        output: Vec::new(),
    };
    let mut warning = String::new();
    let mut stop = Process::new("opencode");
    stop.args(["service", "stop"]);
    if let Err(error) = run_limited(
        terminal,
        &mut progress,
        "Stopping OpenCode service",
        stop,
        Some(Duration::from_secs(5)),
    ) {
        warning = format!("Service stop did not complete ({error}); stopping processes directly. ");
        progress.line(warning.as_bytes());
        progress.done += 1;
    }
    let id = Process::new("id").arg("-u").output()?;
    eyre::ensure!(id.status.success(), "Could not determine current user");
    let uid = String::from_utf8(id.stdout)?.trim().parse::<u32>()?;
    progress.show(terminal, "Stopping OpenCode and OpenChamber processes")?;
    let targets = running_targets(uid)?;
    for pid in &targets {
        signal_target(*pid, uid, "-TERM")?;
        progress.line(format!("Sent TERM to {pid}").as_bytes());
    }
    if !targets.is_empty() {
        std::thread::sleep(Duration::from_secs(1));
    }
    for pid in running_targets(uid)?
        .into_iter()
        .filter(|pid| targets.contains(pid))
    {
        signal_target(pid, uid, "-KILL")?;
        progress.line(format!("Sent KILL to {pid}").as_bytes());
    }
    eyre::ensure!(
        running_targets(uid)?.is_empty(),
        "OpenCode or OpenChamber is still running; cache was not cleared"
    );
    progress.done += 1;
    progress.show(terminal, "Clearing OpenCode cache")?;
    let removed = clear_cache(&cache)?;
    progress.done += 1;
    progress.show(terminal, "Reset complete")?;
    notice(
        terminal,
        "Reset — complete",
        &format!(
            "{warning}Stopped {} process(es). Cache {}.",
            targets.len(),
            if removed { "removed" } else { "not present" }
        ),
    )
}

#[derive(Clone)]
enum InstallPage {
    Options,
    Backup,
    Stacks,
    Projects,
    Global,
    GlobalHost,
    GlobalToken,
    Host(PathBuf),
    Token(PathBuf),
    Archive(PathBuf),
    Rag(PathBuf),
    Review,
}

struct InstallPlan {
    config: PathBuf,
    backup: PathBuf,
    existing: bool,
    with_chamber: bool,
    stacks: Vec<StackInstall>,
    rag_projects: Vec<PathBuf>,
    global: Option<(String, String)>,
    cleanup: cleanup::Inventory,
}

type StackInstall = (PathBuf, Option<(String, String)>, bool);

fn install_pages(
    existing: bool,
    selected: &[PathBuf],
    projects: &[PathBuf],
    global: bool,
) -> Vec<InstallPage> {
    let mut pages = vec![InstallPage::Options];
    if existing {
        pages.push(InstallPage::Backup);
    }
    pages.extend([
        InstallPage::Stacks,
        InstallPage::Projects,
        InstallPage::Global,
    ]);
    if global {
        pages.extend([InstallPage::GlobalHost, InstallPage::GlobalToken]);
    }
    for path in selected {
        let has_envrc = fs::symlink_metadata(path.join(".envrc")).is_ok();
        if !has_envrc || stack::legacy_credentials(path).is_err() {
            pages.push(InstallPage::Host(path.clone()));
            pages.push(InstallPage::Token(path.clone()));
        }
        if has_envrc && stack::legacy_credentials(path).is_err() {
            pages.push(InstallPage::Archive(path.clone()));
        }
    }
    for path in projects {
        pages.push(InstallPage::Rag(path.clone()));
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
    let existing = fs::symlink_metadata(&config).is_ok();
    let backup = ops::backup_path(&config);
    let mut with_chamber = false;
    let mut backup_ok = false;
    let mut current = user_home.canonicalize()?;
    let mut selected = load_stacks(&user_home)?;
    selected.retain(|path| path.exists());
    let mut projects = Vec::new();
    let mut projects_initialized = false;
    let previous_global = if existing {
        ops::global_credentials(&config).ok().flatten()
    } else {
        None
    };
    let mut global = previous_global.is_some();
    let mut global_credentials = previous_global.unwrap_or_default();
    let mut credentials = std::collections::BTreeMap::<PathBuf, (String, String)>::new();
    let mut archives = std::collections::BTreeMap::<PathBuf, bool>::new();
    let mut rag_choices = std::collections::BTreeMap::<PathBuf, bool>::new();
    let mut page = 0usize;
    let mut preview_summary = String::new();
    let mut cleanup_plan = None;
    loop {
        let pages = install_pages(existing, &selected, &projects, global);
        let current_page = pages[page].clone();
        let nav = match current_page {
            InstallPage::Options => ui::step_checkbox(
                terminal,
                "Optional application",
                "OpenCode is required. OpenChamber is optional and uses the Homebrew cask.",
                "Install OpenChamber",
                &mut with_chamber,
                true,
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
            InstallPage::Stacks => {
                ui::step_explorer(terminal, &user_home, &mut current, &mut selected)?
            }
            InstallPage::Projects => {
                ui::step_project_explorer(terminal, &user_home, &mut current, &mut projects)?
            }
            InstallPage::Global => ui::step_checkbox(
                terminal,
                "Global Foundry fallback",
                "Configure a private default Foundry host/token for projects outside selected stacks? Stack-local configs override it.",
                "Use global fallback",
                &mut global,
                false,
            )?,
            InstallPage::GlobalHost => ui::step_text(
                terminal,
                "Global Foundry host",
                "Enter an HTTPS Foundry host",
                &mut global_credentials.0,
                false,
                ui::Field::Plain,
            )?,
            InstallPage::GlobalToken => ui::step_text(
                terminal,
                "Global Foundry token",
                "Enter a private token for the global fallback",
                &mut global_credentials.1,
                false,
                ui::Field::Token,
            )?,
            InstallPage::Host(name) => {
                let fields = credentials
                    .entry(name.clone())
                    .or_insert_with(|| stack::credentials(&name).unwrap_or_default());
                ui::step_text(
                    terminal,
                    &format!("{} — Foundry host", name.display()),
                    "Enter this stack's Foundry base URL (https://host). The config will be created in the selected stack directory, not its nested repositories.",
                    &mut fields.0,
                    false,
                    ui::Field::Plain,
                )?
            }
            InstallPage::Token(name) => {
                let fields = credentials.entry(name.clone()).or_default();
                ui::step_text(
                    terminal,
                    &format!("{} — API token", name.display()),
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
            InstallPage::Archive(path) => ui::step_checkbox(
                terminal,
                "Unrecognized .envrc",
                &format!(
                    "{} contains shell content the installer will not execute. After verifying the new stack config, archive it (recommended) or delete it? Uncheck to delete.",
                    path.join(".envrc").display()
                ),
                "Archive old .envrc",
                archives.entry(path).or_insert(true),
                false,
            )?,
            InstallPage::Rag(path) => ui::step_checkbox(
                terminal,
                &format!("RAG — {}", path.display()),
                "Install or update this folder's RAG and index? Existing RAG files are backed up; the index is preserved.",
                "Set up RAG",
                rag_choices.entry(path).or_insert(true),
                false,
            )?,
            InstallPage::Review => {
                validate_rag_projects(
                    &rag_targets(&projects, &rag_choices),
                    &selected,
                    global,
                    &config,
                )?;
                let home_for_check = user_home.clone();
                let inventory = busy(terminal, "Inspecting installed tools", move || {
                    cleanup::inspect(&home_for_check, with_chamber)
                })?;
                let mut preview = format!(
                    "Homebrew: update; install/upgrade OpenCode{}.\nClone {} to {}{}.\nConfigure {} selected stacks.\nRemove direnv hook from .zshrc, ~/.config/direnv, ~/.local/share/direnv and its installation.\nDownload latest macOS release, verify SHA-256 and version, install to {}.",
                    if with_chamber { ", OpenChamber" } else { "" },
                    ops::REMOTE,
                    config.display(),
                    if existing {
                        format!(" (backup: {})", backup.display())
                    } else {
                        String::new()
                    },
                    selected.len(),
                    ops::BIN_PATH
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
                        "\nUnrecognized executable (leave untouched): {}",
                        path.display()
                    ));
                }
                cleanup_plan = Some(inventory);
                if global {
                    preview.push_str("\nConfigure private global Foundry fallback.");
                }
                for name in &selected {
                    let path = name.join(".envrc");
                    if stack::outdated_mcp(name).unwrap_or(false) {
                        preview.push_str(&format!("\nBack up and update verified Palantir MCP launcher in {} (npm registry override only)", name.display()));
                    }
                    if fs::symlink_metadata(&path).is_ok() {
                        let pair = credentials
                            .get(name)
                            .map(|(url, key)| (url.as_str(), key.as_str()));
                        let result =
                            stack::migrate(name, true, pair, *archives.get(name).unwrap_or(&false))
                                .wrap_err_with(|| format!("Cannot migrate {}", path.display()))?;
                        preview.push_str(&format!("\n{result}"));
                    } else if let Some((url, _)) = credentials.get(name) {
                        preview.push_str(&format!("\n{}", preview_stack(name, url)));
                    }
                }
                for project in &projects {
                    preview.push_str(&format!(
                        "\nRAG for {}: {}",
                        project.display(),
                        if *rag_choices.get(project).unwrap_or(&true) {
                            "ON (backup, update and index)"
                        } else {
                            "OFF (unchanged)"
                        }
                    ));
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
            ui::Nav::Cancel => return Err(Cancelled.into()),
            ui::Nav::Back => page = page.saturating_sub(1),
            ui::Nav::Open => {
                if let InstallPage::Token(name) = &pages[page] {
                    let url = &credentials[name].0;
                    open_token_page(terminal, url, dry_run)?;
                } else if matches!(&pages[page], InstallPage::GlobalToken) {
                    open_token_page(terminal, &global_credentials.0, dry_run)?;
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
                    InstallPage::Stacks => {
                        let config_path = config.canonicalize().unwrap_or(config.clone());
                        if selected.iter().any(|path| {
                            path.starts_with(&config_path)
                                || !path.is_dir()
                                || path == &user_home
                                || path == std::path::Path::new("/")
                        }) {
                            notice(
                                terminal,
                                "Invalid stack",
                                "Choose existing stack folders outside the config checkout, not your home or filesystem root.",
                            )?;
                            continue;
                        }
                        if !projects_initialized {
                            projects = selected.clone();
                            projects_initialized = true;
                        }
                    }
                    InstallPage::Projects => {
                        let config_path = config.canonicalize().unwrap_or(config.clone());
                        if projects.iter().any(|path| {
                            !path.is_dir()
                                || path.starts_with(&config_path)
                                || path == &user_home
                                || path == std::path::Path::new("/")
                        }) {
                            notice(
                                terminal,
                                "Invalid RAG project",
                                "Choose project folders outside the managed config checkout, not your home or filesystem root.",
                            )?;
                            continue;
                        }
                    }
                    InstallPage::GlobalHost if !ops::valid_url(&global_credentials.0) => {
                        notice(
                            terminal,
                            "Invalid URL",
                            "Enter an HTTPS host without a path.",
                        )?;
                        continue;
                    }
                    InstallPage::GlobalToken if !dry_run && global_credentials.1.is_empty() => {
                        notice(
                            terminal,
                            "Token required",
                            "Enter the global Foundry token.",
                        )?;
                        continue;
                    }
                    InstallPage::GlobalToken if dry_run && global_credentials.1.is_empty() => {
                        global_credentials.1 = "dry-run-placeholder".into();
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
                    InstallPage::Token(name) if dry_run && credentials[name].1.is_empty() => {
                        credentials.get_mut(name).unwrap().1 = "dry-run-placeholder".into();
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
                            "Unrecognized OpenCode executable",
                            "Unknown executables will not be removed. Homebrew installation and config setup will continue; check PATH if an old executable is selected by your shell.",
                        )?;
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
                        steps.push((
                            "clone config repository to ~/.config/opencode".to_string(),
                            500,
                        ));
                        for name in &selected {
                            let action = if fs::symlink_metadata(name.join(".envrc")).is_ok() {
                                "migrate existing .envrc"
                            } else {
                                "replace or create stack config"
                            };
                            steps.push((format!("{action} for {}", name.display()), 170));
                        }
                        for project in rag_targets(&projects, &rag_choices) {
                            steps.push((
                                format!("update and index RAG for {}", project.display()),
                                250,
                            ));
                        }
                        steps.push(("upgrade verified Palantir MCP launchers".to_string(), 220));
                        steps.push(("replace the installer in /usr/local/bin".to_string(), 220));
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
                    for path in selected {
                        let pair = credentials.remove(&path);
                        let archive = *archives.get(&path).unwrap_or(&false);
                        stacks.push((path, pair, archive));
                    }
                    let rag_projects = rag_targets(&projects, &rag_choices);
                    return Ok(Some(InstallPlan {
                        config,
                        backup,
                        existing,
                        with_chamber,
                        stacks,
                        rag_projects,
                        global: global.then_some(global_credentials),
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
        with_chamber,
        stacks,
        rag_projects,
        global,
        cleanup: inventory,
    } = plan;
    let (binary, release_version) = busy(
        terminal,
        "Downloading and verifying latest installer",
        ops::stage_latest_binary,
    )?;
    let release_tag = format!("v{release_version}");
    let mut progress = Progress {
        done: 0,
        total: 4
            + usize::from(with_chamber)
            + stacks.len()
            + rag_projects.len()
            + inventory.brew_v1.len()
            + inventory.npm_v1.len()
            + inventory.files.len()
            + usize::from(inventory.brew_v2)
            + usize::from(inventory.chamber),
        output: Vec::new(),
    };
    brew_update(terminal, &mut progress)?;
    busy(
        terminal,
        "Verifying OpenCode formula",
        cleanup::latest_is_v2,
    )?;
    let home_for_check = home()?;
    eyre::ensure!(
        busy(terminal, "Rechecking installed tools", move || {
            cleanup::inspect(&home_for_check, with_chamber)
        })? == inventory,
        "Install inventory changed after the Homebrew refresh; review the install again before removing anything"
    );
    // Decide before removing anything: a running optional app must never block required setup.
    let plan_with_chamber = with_chamber;
    let with_chamber = with_chamber && chamber_available(terminal);
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
    let version = Process::new("opencode").arg("--version").output();
    if !matches!(version, Ok(ref output) if output.status.success() && cleanup::reports_v2(&String::from_utf8_lossy(&output.stdout)))
    {
        notice(
            terminal,
            "PATH warning",
            "Homebrew installed OpenCode, but another opencode executable is first on PATH. Check your shell PATH after installation.",
        )?;
    }
    if with_chamber {
        let safe_to_move = inventory.chamber && chamber_available(terminal);
        let mut chamber_backup = None;
        if safe_to_move {
            progress.show(terminal, "Moving non-Homebrew OpenChamber.app to Trash")?;
            let backup = cleanup::move_to_trash(
                std::path::Path::new("/Applications/OpenChamber.app"),
                &home()?,
            )?;
            chamber_backup = Some(backup.clone());
            trashed.push(backup);
            progress.done += 1;
        }
        if !inventory.chamber || safe_to_move {
            if !optional_chamber(terminal, &mut progress)
                && let Some(backup) = chamber_backup
            {
                let app = std::path::Path::new("/Applications/OpenChamber.app");
                restore_chamber(&backup, app)?;
                trashed.retain(|path| path != &backup);
            }
        } else {
            progress.done += 2; // skipped the app move and optional cask installation
        }
    } else if inventory.chamber {
        progress.done += 2;
    } else if plan_with_chamber {
        progress.done += 1;
    }
    fs::create_dir_all(config.parent().expect("config has parent"))?;
    let staged = ops::backup_path(&config);
    let make_clone = || {
        let mut clone = Process::new("git");
        clone
            .args([
                "clone",
                "--progress",
                "--depth",
                "1",
                "--branch",
                &release_tag,
                ops::REMOTE,
            ])
            .arg(&staged);
        clone
    };
    if let Err(error) = run_interactive_git(
        terminal,
        &mut progress,
        "Cloning latest configs",
        make_clone(),
        make_clone(),
        Some(&staged),
    ) {
        if staged.exists() {
            fs::remove_dir_all(&staged)?;
        }
        return Err(error);
    }
    if let Some((url, key)) = &global
        && let Err(error) = ops::prepare_global(&staged, url, key)
    {
        fs::remove_dir_all(&staged)?;
        return Err(error);
    }
    ops::replace_config(&config, &staged, existing.then_some(backup.as_path()))?;
    let mut migration = Vec::new();
    let selected: Vec<PathBuf> = stacks.iter().map(|(path, _, _)| path.clone()).collect();
    save_stacks(&home()?, &selected)?;
    for (path, pair, archive) in stacks {
        progress.show(terminal, &format!("Configuring stack {}", path.display()))?;
        if fs::symlink_metadata(path.join(".envrc")).is_ok() {
            migration.push(stack::migrate(
                &path,
                false,
                pair.as_ref().map(|(url, key)| (url.as_str(), key.as_str())),
                archive,
            )?);
        } else if let Some((url, key)) = pair {
            stack::replace(&path, &url, &key, false)?;
        }
        progress.done += 1;
    }
    for path in &selected {
        match stack::outdated_mcp(path) {
            Ok(true) => {
                stack::upgrade_mcp(path)?;
                migration.push(format!(
                    "Updated Palantir MCP registry for {} (old config backed up)",
                    path.display()
                ));
            }
            Ok(false) => {}
            Err(error) => migration.push(format!(
                "MCP launcher in {} was not upgraded: {error}",
                path.display()
            )),
        }
    }
    for path in rag_projects {
        let mut command = Process::new(binary.path());
        command.arg("apply-rag").arg(&path);
        run(
            terminal,
            &mut progress,
            &format!("Setting up RAG for {}", path.display()),
            command,
        )?;
    }
    remove_direnv(terminal, false)?;
    progress.show(terminal, "Installation complete")?;
    notice(
        terminal,
        "Install — complete",
        &format!(
            "Installation finished.{}\nStack configs are inherited by their child projects.{}\nRelease {release_version} will replace /usr/local/bin/foundry-opencode when you leave this screen.{}",
            if existing {
                format!(" Backup: {}", backup.display())
            } else {
                String::new()
            },
            if migration.is_empty() {
                String::new()
            } else {
                format!("\n{}", migration.join("\n"))
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
    Ok(Some(binary))
}

fn update(
    terminal: &mut DefaultTerminal,
    dry_run: bool,
) -> eyre::Result<Option<ops::StagedBinary>> {
    let config = home()?.join(".config/opencode");
    let user_home = home()?;
    let mut current = user_home.canonicalize()?;
    let mut selected = load_stacks(&user_home)?;
    selected.retain(|path| path.exists());
    let mut projects = load_paths(&user_home.join(".config/foundry-opencode/rag.json"))?;
    projects.retain(|path| path.exists());
    let mut projects_initialized = false;
    let mut rag_choices = std::collections::BTreeMap::new();
    let mut saved_global = None;
    let mut saved_stacks = std::collections::BTreeMap::new();
    let (
        migration,
        stack_changes,
        mcp_upgrades,
        global_change,
        global,
        existing,
        backup,
        details,
        rag_projects,
    ) = 'wizard: loop {
        if ui::step_initial_explorer(terminal, &user_home, &mut current, &mut selected)?
            != ui::Nav::Next
        {
            return Err(Cancelled.into());
        }
        if !projects_initialized {
            for path in &selected {
                if !projects.iter().any(|project| project.starts_with(path)) {
                    projects.push(path.clone());
                }
            }
            projects_initialized = true;
        }
        loop {
            if ui::step_project_explorer(terminal, &user_home, &mut current, &mut projects)?
                != ui::Nav::Next
            {
                continue 'wizard;
            }
            if select_rag(terminal, &projects, &mut rag_choices)? == ui::Nav::Next {
                break;
            }
        }
        let config_path = config.canonicalize().unwrap_or(config.clone());
        eyre::ensure!(
            selected.iter().all(|path| path.is_dir()
                && !path.starts_with(&config_path)
                && path != &user_home
                && path != std::path::Path::new("/")),
            "Invalid selected stack directory"
        );
        let mut migration = Vec::new();
        for path in &selected {
            if fs::symlink_metadata(path.join(".envrc")).is_ok() {
                let Some((supplied, archive)) = migration_choice(terminal, path, dry_run)? else {
                    continue 'wizard;
                };
                let preview = stack::migrate(
                    path,
                    true,
                    supplied
                        .as_ref()
                        .map(|(url, key)| (url.as_str(), key.as_str())),
                    archive,
                )?;
                migration.push((path.clone(), supplied, archive, preview));
            }
        }
        let summary = migration
            .iter()
            .map(|(_, _, _, result)| result.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let existing = fs::symlink_metadata(&config).is_ok();
        let backup = ops::backup_path(&config);
        let global = if existing {
            match ops::global_credentials(&config) {
                Ok(value) => value,
                Err(error) => {
                    notice(
                        terminal,
                        "Global fallback not reusable",
                        &format!(
                            "{error}. The old config will be backed up; this fallback will not be copied to the new config."
                        ),
                    )?;
                    None
                }
            }
        } else {
            None
        };
        let scopes = std::iter::once(None)
            .chain(
                selected
                    .iter()
                    .filter(|path| fs::symlink_metadata(path.join(".envrc")).is_err())
                    .cloned()
                    .map(Some),
            )
            .collect::<Vec<_>>();
        let mut offer_page = 0;
        while offer_page < scopes.len() {
            let scope = &scopes[offer_page];
            let previous = match scope {
                Some(path) => saved_stacks
                    .get(path)
                    .cloned()
                    .or_else(|| stack::credentials(path).ok()),
                None => saved_global.clone().or(global.clone()),
            };
            let name = scope.as_ref().map_or_else(
                || "Global Foundry fallback".to_owned(),
                |path| format!("Stack {}", path.display()),
            );
            match offer_credentials(terminal, &name, previous, dry_run)? {
                CredentialChoice::Back if offer_page > 0 => {
                    offer_page -= 1;
                    continue;
                }
                CredentialChoice::Back => {
                    if projects.is_empty() {
                        continue 'wizard;
                    }
                    if revisit_rag(
                        terminal,
                        &user_home,
                        &mut current,
                        &mut projects,
                        &mut rag_choices,
                    )? == ui::Nav::Back
                    {
                        continue 'wizard;
                    }
                    continue;
                }
                CredentialChoice::Keep => {}
                CredentialChoice::Change(pair) => {
                    if let Some(path) = scope {
                        saved_stacks.insert(path.clone(), pair);
                    } else {
                        saved_global = Some(pair);
                    }
                }
            }
            offer_page += 1;
        }
        let global_change = saved_global
            .clone()
            .filter(|pair| global.as_ref() != Some(pair));
        validate_rag_projects(
            &rag_targets(&projects, &rag_choices),
            &selected,
            global_change.is_some() || global.is_some(),
            &config,
        )?;
        let mut stack_changes = Vec::new();
        for path in &selected {
            if fs::symlink_metadata(path.join(".envrc")).is_ok() {
                continue;
            }
            let previous = saved_stacks
                .get(path)
                .cloned()
                .or_else(|| stack::credentials(path).ok());
            let (url, key) = previous.ok_or_else(|| {
                eyre::eyre!(
                    "{} has no reusable credentials; select Change host/token",
                    path.display()
                )
            })?;
            stack::replace(path, &url, &key, true)?;
            stack_changes.push((path.clone(), url, key));
        }
        let mut mcp_upgrades = Vec::new();
        for path in &selected {
            if stack_changes.iter().any(|(changed, _, _)| changed == path) {
                continue; // Credential replacement already writes the new launcher.
            }
            if stack::outdated_mcp(path).unwrap_or(false) {
                mcp_upgrades.push(path.clone());
            }
        }
        let changes = format!(
            "\nGlobal credentials: {}; stack configs replaced or created: {}. Existing stack tokens are reused unless changed. Tokens are redacted.{}",
            if global_change.is_some() {
                "update"
            } else {
                "unchanged"
            },
            if stack_changes.is_empty() {
                "none".to_owned()
            } else {
                stack_changes
                    .iter()
                    .map(|(path, _, _)| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            },
            if selected
                .iter()
                .any(|path| fs::symlink_metadata(path.join(".envrc")).is_ok())
            {
                " Legacy .envrc stacks can be edited after migration by rerunning update."
            } else {
                ""
            }
        );
        let details = loop {
            let rag_summary = projects
                .iter()
                .map(|path| {
                    format!(
                        "\nRAG for {}: {}",
                        path.display(),
                        if *rag_choices.get(path).unwrap_or(&true) {
                            "ON (backup, update and index)"
                        } else {
                            "OFF (unchanged)"
                        }
                    )
                })
                .collect::<String>();
            let details = format!(
                "Update Homebrew packages; clone latest config to {}{}; migrate selected stacks:\n{summary}{changes}{rag_summary}\nBack up existing selected stack configs and tokens before replacement. Use public npm registry for Palantir MCP in newly generated configs. Upgrade remaining managed MCP launchers: {}.\nDownload latest macOS release, verify SHA-256 and version, install to {} after closing the wizard.\nRemove direnv hook from .zshrc, ~/.config/direnv, ~/.local/share/direnv and its installation.{}",
                config.display(),
                if existing {
                    format!(" (backup: {})", backup.display())
                } else {
                    String::new()
                },
                if mcp_upgrades.is_empty() {
                    "none".to_owned()
                } else {
                    mcp_upgrades
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                ops::BIN_PATH,
                if dry_run {
                    "\nDry run: no changes."
                } else {
                    ""
                }
            );
            match ui::step_review(
                terminal,
                "Review update",
                &details,
                dry_run,
                false,
                "Update",
            )? {
                ui::Nav::Back if projects.is_empty() => continue 'wizard,
                ui::Nav::Back
                    if revisit_rag(
                        terminal,
                        &user_home,
                        &mut current,
                        &mut projects,
                        &mut rag_choices,
                    )? == ui::Nav::Back =>
                {
                    continue 'wizard;
                }
                ui::Nav::Back => continue,
                ui::Nav::Next => break details,
                _ => return Err(Cancelled.into()),
            }
        };
        let rag_projects = rag_targets(&projects, &rag_choices);
        break (
            migration,
            stack_changes,
            mcp_upgrades,
            global_change,
            global,
            existing,
            backup,
            details,
            rag_projects,
        );
    };
    if dry_run {
        let mut steps = vec![
            ("refresh Homebrew metadata".to_string(), 300),
            ("install or upgrade OpenCode".to_string(), 400),
            ("upgrade OpenChamber if installed".to_string(), 300),
            ("migrate selected stack .envrc files".to_string(), 300),
            ("upgrade managed Palantir MCP launchers".to_string(), 220),
            (
                "clone latest config and back up existing checkout".to_string(),
                500,
            ),
            ("replace the installer in /usr/local/bin".to_string(), 220),
        ];
        for project in &rag_projects {
            steps.push((
                format!("update and index RAG for {}", project.display()),
                250,
            ));
        }
        simulate(terminal, &steps)?;
        notice(terminal, "Dry-run complete", &details)?;
        return Ok(None);
    }
    let (binary, release_version) = busy(
        terminal,
        "Downloading and verifying latest installer",
        ops::stage_latest_binary,
    )?;
    let release_tag = format!("v{release_version}");
    let chamber = installed("openchamber", true)?;
    let mut progress = Progress {
        done: 0,
        total: 3 + usize::from(chamber) + rag_projects.len(),
        output: Vec::new(),
    };
    brew_update(terminal, &mut progress)?;
    brew_package(terminal, &mut progress, "opencode", false)?;
    if chamber {
        let _ = optional_chamber(terminal, &mut progress);
    }
    let staged = ops::backup_path(&config);
    let make_pull = || {
        let mut git = Process::new("git");
        git.args([
            "clone",
            "--progress",
            "--depth",
            "1",
            "--branch",
            &release_tag,
            ops::REMOTE,
        ])
        .arg(&staged);
        git
    };
    let cloned = run_interactive_git(
        terminal,
        &mut progress,
        "Cloning latest configs",
        make_pull(),
        make_pull(),
        Some(&staged),
    );
    if let Err(error) = cloned {
        if staged.is_dir() {
            fs::remove_dir_all(&staged)?;
        }
        return Err(error);
    }
    if let Some((url, key)) = global_change.or(global)
        && let Err(error) = ops::prepare_global(&staged, &url, &key)
    {
        fs::remove_dir_all(&staged)?;
        return Err(error);
    }
    ops::replace_config(&config, &staged, existing.then_some(backup.as_path()))?;
    save_stacks(&user_home, &selected)?;
    let mut results = Vec::new();
    for (path, supplied, archive, _) in migration {
        results.push(stack::migrate(
            &path,
            false,
            supplied
                .as_ref()
                .map(|(url, key)| (url.as_str(), key.as_str())),
            archive,
        )?);
    }
    for (path, url, key) in stack_changes {
        stack::replace(&path, &url, &key, false)?;
        results.push(format!(
            "Replaced stack config and private credentials for {}",
            path.display()
        ));
    }
    for path in mcp_upgrades {
        results.push(if stack::upgrade_mcp(&path)? {
            format!(
                "Updated Palantir MCP registry for {} (old config backed up)",
                path.display()
            )
        } else {
            format!(
                "Skipped changed Palantir MCP config at {}; review it manually",
                path.display()
            )
        });
    }
    for path in rag_projects {
        let mut command = Process::new(binary.path());
        command.arg("apply-rag").arg(&path);
        run(
            terminal,
            &mut progress,
            &format!("Setting up RAG for {}", path.display()),
            command,
        )?;
    }
    remove_direnv(terminal, dry_run)?;
    notice(
        terminal,
        "Update — complete",
        &format!(
            "Packages and configs are up to date. Release {release_version} will replace the CLI in /usr/local/bin when you leave this screen.{}",
            if results.is_empty() {
                String::new()
            } else {
                format!("\n{}", results.join("\n"))
            }
        ),
    )?;
    Ok(Some(binary))
}

enum CredentialChoice {
    Back,
    Keep,
    Change((String, String)),
}

fn offer_credentials(
    terminal: &mut DefaultTerminal,
    name: &str,
    previous: Option<(String, String)>,
    dry_run: bool,
) -> eyre::Result<CredentialChoice> {
    let mut enabled = false;
    let (mut url, mut key) = previous.unwrap_or_default();
    let mut page = 0;
    loop {
        let nav = match page {
            0 => ui::step_checkbox(
                terminal,
                &format!("Update {name}?"),
                "Leave unchecked to keep the existing host and token. A selected scope can change either or both.",
                "Change host/token",
                &mut enabled,
                false,
            )?,
            1 => ui::step_text(
                terminal,
                &format!("{name} — host"),
                "HTTPS Foundry host",
                &mut url,
                false,
                ui::Field::Plain,
            )?,
            _ => ui::step_text(
                terminal,
                &format!("{name} — token"),
                "Private Foundry token (keep or replace)",
                &mut key,
                false,
                ui::Field::Token,
            )?,
        };
        match nav {
            ui::Nav::Cancel => return Err(Cancelled.into()),
            ui::Nav::Back if page > 0 => page -= 1,
            ui::Nav::Back => return Ok(CredentialChoice::Back),
            ui::Nav::Open => open_token_page(terminal, &url, dry_run)?,
            ui::Nav::Next if page == 0 && !enabled => return Ok(CredentialChoice::Keep),
            ui::Nav::Next if page == 1 && !ops::valid_url(&url) => {
                notice(
                    terminal,
                    "Invalid host",
                    "Enter an HTTPS host without a path or spaces.",
                )?;
            }
            ui::Nav::Next if page == 2 && key.is_empty() && !dry_run => {
                notice(terminal, "Token required", "Enter a Foundry token.")?;
            }
            ui::Nav::Next if page == 2 => {
                if key.is_empty() {
                    key = "dry-run-placeholder".into();
                }
                eyre::ensure!(!key.chars().any(char::is_control), "Invalid Foundry token");
                return Ok(CredentialChoice::Change((url, key)));
            }
            ui::Nav::Next => page += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_is_a_distinct_outcome() {
        let error: eyre::Report = Cancelled.into();
        assert!(error.is::<Cancelled>());
    }

    #[test]
    fn no_arguments_select_install() {
        let args = Args::try_parse_from(["foundry-opencode"]).unwrap();
        assert!(matches!(
            args.command.unwrap_or(Mode::Install),
            Mode::Install
        ));
    }

    #[test]
    fn verified_release_can_run_the_selected_rag_project() {
        let args = Args::try_parse_from(["foundry-opencode", "apply-rag", "/project"]).unwrap();
        assert!(
            matches!(args.command, Some(Mode::ApplyRag { project }) if project == std::path::Path::new("/project"))
        );
    }

    #[test]
    fn dry_run_works_before_and_after_commands() {
        for args in [
            vec!["foundry-opencode", "--dry-run"],
            vec!["foundry-opencode", "--dry-run", "init"],
            vec!["foundry-opencode", "init", "--dry-run"],
            vec!["foundry-opencode", "update", "--dry-run"],
            vec!["foundry-opencode", "init-rag", "--dry-run"],
            vec!["foundry-opencode", "reset", "--dry-run"],
            vec!["foundry-opencode", "uninstall", "--dry-run"],
        ] {
            assert!(Args::try_parse_from(args).unwrap().dry_run);
        }
    }

    #[test]
    fn reset_selects_only_current_users_named_processes_and_cache() {
        let list = " 123 501 /usr/local/bin/opencode\n 124 502 opencode\n 125 501 /Applications/OpenChamber.app/Contents/MacOS/OpenChamber\n 126 501 OpenChamber Helper (Renderer)\n 127 501 unrelated-opencode\n 128 501 OpenCode Helper (GPU)\n";
        assert_eq!(reset_targets(list, 501), [123, 125, 126, 128]);
        let root = ops::backup_path(&std::env::temp_dir().join("reset-test"));
        fs::create_dir(&root).unwrap();
        let cache = root.join("opencode");
        assert!(!clear_cache(&cache).unwrap());
        fs::create_dir(&cache).unwrap();
        fs::write(cache.join("tmp"), "cached").unwrap();
        #[cfg(unix)]
        {
            let link = root.join("linked-cache");
            std::os::unix::fs::symlink(&cache, &link).unwrap();
            assert!(clear_cache(&link).is_err());
            assert!(cache.join("tmp").exists());
        }
        assert!(clear_cache(&cache).unwrap());
        assert!(!cache.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_output_is_bounded_and_strips_controls() {
        let mut progress = Progress {
            done: 0,
            total: 1,
            output: Vec::new(),
        };
        progress.line(b"first\x1b[31m\n");
        assert_eq!(progress.output, ["first[31m"]);
        for _ in 0..220 {
            progress.line(b"next");
        }
        assert_eq!(progress.output.len(), 200);
        let (tx, rx) = mpsc::channel();
        stream_output(&b"git 10%\rgit 20%\nfinished"[..], tx);
        assert_eq!(
            rx.into_iter().collect::<Vec<_>>(),
            [
                b"git 10%".to_vec(),
                b"git 20%".to_vec(),
                b"finished".to_vec()
            ]
        );
    }

    #[test]
    fn unknown_chamber_status_is_not_treated_as_not_running() {
        assert!(chamber_status(Some(0)).unwrap());
        assert!(!chamber_status(Some(1)).unwrap());
        assert!(chamber_status(Some(2)).is_err());
        assert!(chamber_status(None).is_err());
    }

    #[test]
    fn chamber_restore_never_replaces_an_existing_app() {
        let root = ops::backup_path(&std::env::temp_dir().join("chamber-restore-test"));
        fs::create_dir(&root).unwrap();
        let backup = root.join("backup");
        let app = root.join("app");
        fs::write(&backup, "original").unwrap();
        fs::write(&app, "new").unwrap();
        assert!(restore_chamber(&backup, &app).is_err());
        assert_eq!(fs::read_to_string(&backup).unwrap(), "original");
        fs::remove_file(&app).unwrap();
        restore_chamber(&backup, &app).unwrap();
        assert_eq!(fs::read_to_string(&app).unwrap(), "original");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn token_page_is_on_the_validated_host() {
        assert_eq!(
            token_page("https://example.palantirfoundry.com"),
            "https://example.palantirfoundry.com/workspace/settings/tokens"
        );
        assert!(!ops::valid_url("https://example.palantirfoundry.com/other"));
        let preview = preview_stack(
            std::path::Path::new("/stack"),
            "https://example.palantirfoundry.com",
        );
        assert!(preview.contains("token redacted"));
        assert!(preview.contains("https://example.palantirfoundry.com"));
    }

    #[test]
    fn install_pages_prompt_for_unrecognized_envrc() {
        let root = ops::backup_path(&std::env::temp_dir().join("foundry-pages"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("existing")).unwrap();
        fs::create_dir(root.join("existing/.opencode")).unwrap();
        fs::write(root.join("existing/.opencode/opencode.json"), "{}").unwrap();
        fs::write(root.join("existing/.envrc"), "preserve").unwrap();
        let selected = vec![root.join("existing"), root.join("new")];
        let pages = install_pages(true, &selected, &selected, false);
        assert!(matches!(pages.first(), Some(InstallPage::Options)));
        assert!(
            pages
                .iter()
                .any(|page| matches!(page, InstallPage::Host(name) if name == &root.join("new")))
        );
        assert!(
            pages.iter().any(
                |page| matches!(page, InstallPage::Host(name) if name == &root.join("existing"))
            )
        );
        assert!(pages.iter().any(
            |page| matches!(page, InstallPage::Archive(name) if name == &root.join("existing"))
        ));
        assert!(
            pages.iter().any(
                |page| matches!(page, InstallPage::Rag(name) if name == &root.join("existing"))
            )
        );
        assert!(matches!(pages.last(), Some(InstallPage::Review)));
        fs::remove_file(root.join("existing/.envrc")).unwrap();
        let pages = install_pages(
            true,
            &[root.join("existing")],
            &[root.join("existing")],
            false,
        );
        assert!(
            pages.iter().any(
                |page| matches!(page, InstallPage::Host(name) if name == &root.join("existing"))
            )
        );
        let project = root.join("existing/project");
        let pages = install_pages(
            true,
            &[root.join("existing")],
            std::slice::from_ref(&project),
            false,
        );
        assert!(
            pages
                .iter()
                .any(|page| matches!(page, InstallPage::Projects))
        );
        assert!(
            pages
                .iter()
                .any(|page| matches!(page, InstallPage::Rag(name) if name == &project))
        );
        assert!(
            !pages.iter().any(
                |page| matches!(page, InstallPage::Rag(name) if name == &root.join("existing"))
            )
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rag_is_on_by_default_and_opt_out_is_per_folder() {
        let first = PathBuf::from("/first");
        let second = PathBuf::from("/second");
        let projects = vec![first.clone(), second.clone()];
        let mut choices = std::collections::BTreeMap::new();
        assert_eq!(rag_targets(&projects, &choices), projects);
        choices.insert(first, false);
        assert_eq!(rag_targets(&projects, &choices), [second]);
    }

    #[test]
    fn rag_project_needs_credentials_before_install_changes_files() {
        let root = ops::backup_path(&std::env::temp_dir().join("rag-project-choice-test"));
        fs::create_dir_all(root.join("stack/project")).unwrap();
        let root = root.canonicalize().unwrap();
        let project = root.join("stack/project");
        assert!(
            validate_rag_projects(
                std::slice::from_ref(&project),
                &[root.join("stack")],
                false,
                &root.join("config")
            )
            .is_ok()
        );
        assert!(
            validate_rag_projects(
                std::slice::from_ref(&project),
                &[],
                false,
                &root.join("config")
            )
            .is_err()
        );
        assert!(validate_rag_projects(&[project], &[], true, &root.join("stack")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rag_uninstall_preserves_unrelated_jsonc_and_ignores() {
        let root = ops::backup_path(&std::env::temp_dir().join("rag-uninstall-test"));
        fs::create_dir_all(root.join("stack/.opencode")).unwrap();
        fs::write(root.join("stack/.opencode/opencode.jsonc"), "{\n  // keep this comment\n  \"model\": \"other\",\n  \"mcp\": {\"servers\": {\"rag\": {\"type\": \"local\"}}}\n}\n").unwrap();
        remove_rag_entry(&root.join("stack")).unwrap();
        let text = fs::read_to_string(root.join("stack/.opencode/opencode.jsonc")).unwrap();
        assert!(text.contains("keep this comment"));
        assert!(!text.contains("\"rag\""));
        assert!(
            Process::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            root.join(".gitignore"),
            "/stack/.opencode/token\n/stack/.envrc.backup-123\n/keep\n",
        )
        .unwrap();
        remove_stack_ignores(&root.join("stack")).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/keep\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn saved_stack_paths_and_removal_stay_bounded() {
        let root = ops::backup_path(&std::env::temp_dir().join("stack-registry-test"));
        fs::create_dir(&root).unwrap();
        let stack = root.join("stack");
        fs::create_dir(&stack).unwrap();
        save_stacks(&root, std::slice::from_ref(&stack)).unwrap();
        assert_eq!(load_stacks(&root).unwrap(), [stack]);
        let keep = root.join("keep");
        fs::write(&keep, "preserve").unwrap();
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&keep, &link).unwrap();
            assert!(remove_safe(&link).is_err());
            assert!(keep.exists());
        }
        fs::remove_dir_all(root).unwrap();
    }
}
