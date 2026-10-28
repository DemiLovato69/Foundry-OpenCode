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

#[derive(Subcommand)]
enum Mode {
    /// Generate per-stack OpenCode configuration
    Init,
    /// Configure and index RAG in the current project
    InitRag,
    /// Update Homebrew packages and fast-forward the config checkout
    Update,
    /// Stop OpenCode and OpenChamber and clear the OpenCode cache
    Reset,
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
                "foundry-opencode           Install (same as install)\nfoundry-opencode install   Re-run installation; back up existing configs\nfoundry-opencode update    Update installed tools and configs; migrate stacks\nfoundry-opencode reset     Stop OpenCode/OpenChamber and clear OpenCode cache\nfoundry-opencode init      Create per-stack OpenCode config\nfoundry-opencode init-rag  Configure and index RAG in the current project\nfoundry-opencode --dry-run init-rag  Preview RAG setup\nfoundry-opencode --dry-run install  Preview without changes\nfoundry-opencode version   Print version\nfoundry-opencode usage     Show these examples"
            );
            return Ok(());
        }
        _ => {}
    }
    color_eyre::install()?;
    if !cfg!(target_os = "macos")
        && !args.dry_run
        && !matches!(args.command, Some(Mode::Init | Mode::InitRag | Mode::Reset))
    {
        eyre::bail!("Installation and updates require macOS and Homebrew");
    }
    let staged = match ratatui::run(|terminal| {
        execute!(std::io::stdout(), EnableMouseCapture)?;
        let result = match args.command.unwrap_or(Mode::Install) {
            Mode::Init => init(terminal, args.dry_run).map(|_| None),
            Mode::InitRag => init_rag(terminal, args.dry_run).map(|_| None),
            Mode::Update => update(terminal, args.dry_run),
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
        false,
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
    notice(terminal, "RAG — complete", &result)
}

fn token_page(url: &str) -> String {
    format!("{url}/workspace/settings/tokens")
}

fn preview_stack(path: &std::path::Path, url: &str) -> String {
    format!(
        "Would create {}/.opencode/opencode.json for {url} and private, Git-ignored .opencode/token (mode 0600, token redacted)",
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
        let preview = stack::migrate(&cwd, true)?;
        if ui::step_review(
            terminal,
            "Migrate stack",
            &preview,
            dry_run,
            false,
            "Migrate",
        )? != ui::Nav::Next
        {
            return Err(Cancelled.into());
        }
        return notice(
            terminal,
            if dry_run {
                "Dry-run complete"
            } else {
                "init — complete"
            },
            &stack::migrate(&cwd, dry_run)?,
        );
    }
    let config = cwd.join(".opencode/opencode.json");
    if fs::symlink_metadata(&config).is_ok()
        || fs::symlink_metadata(cwd.join(".opencode/opencode.jsonc")).is_ok()
    {
        if dry_run {
            return notice(
                terminal,
                "init — dry run",
                &format!(
                    "{} already exists; would not overwrite it.",
                    config.display()
                ),
            );
        }
        eyre::bail!(
            "{} already has OpenCode configuration; it was not changed",
            cwd.display()
        );
    }
    let mut url = String::new();
    let mut key = String::new();
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
        let steps = vec![("create per-stack config and private token".to_string(), 350)];
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
    stack::create(&cwd, &url, &key)?;
    notice(
        terminal,
        "init — complete",
        "Created per-stack OpenCode config and private token. OpenCode projects beneath this directory inherit the stack settings.",
    )
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
    Projects,
    Stacks,
    Host(String),
    Token(String),
    Review,
}

struct InstallPlan {
    config: PathBuf,
    backup: PathBuf,
    existing: bool,
    with_chamber: bool,
    stacks: Vec<(PathBuf, Option<(String, String)>)>,
    cleanup: cleanup::Inventory,
}

fn install_pages(
    existing: bool,
    projects: &std::path::Path,
    selected: &[String],
) -> Vec<InstallPage> {
    let mut pages = vec![InstallPage::Options];
    if existing {
        pages.push(InstallPage::Backup);
    }
    pages.extend([InstallPage::Projects, InstallPage::Stacks]);
    for name in selected {
        if fs::symlink_metadata(projects.join(name).join(".envrc")).is_err()
            && fs::symlink_metadata(projects.join(name).join(".opencode/opencode.json")).is_err()
            && fs::symlink_metadata(projects.join(name).join(".opencode/opencode.jsonc")).is_err()
        {
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
    let existing = fs::symlink_metadata(&config).is_ok();
    let backup = ops::backup_path(&config);
    let mut with_chamber = false;
    let mut backup_ok = false;
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
        let pages = install_pages(existing, &projects, &selected);
        let current = pages[page].clone();
        let nav = match current {
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
            InstallPage::Projects => ui::step_text(
                terminal,
                "Main projects directory",
                "Choose the EXISTING parent directory that contains one folder per Foundry stack.\n\nExample:\n~/Projects/\n  Stack1/  ← .opencode config and private token live here\n    Functions Repo/\n  Stack2/\n    OSDK React App Repo/\n\nRepositories inside stacks inherit their stack's OpenCode configuration. Enter an absolute path or ~/Projects:",
                &mut projects_input,
                false,
                ui::Field::Directory(user_home.as_path()),
            )?,
            InstallPage::Stacks => ui::step_stacks(terminal, &names, &mut selected, &mut query)?,
            InstallPage::Host(name) => {
                let fields = credentials.entry(name.clone()).or_default();
                ui::step_text(
                    terminal,
                    &format!("{name} — Foundry host"),
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
                let home_for_check = user_home.clone();
                let inventory = busy(terminal, "Inspecting installed tools", move || {
                    cleanup::inspect(&home_for_check, with_chamber)
                })?;
                let mut preview = format!(
                    "Homebrew: update; install/upgrade OpenCode{}.\nClone {} to {}{}.\nCreate or migrate selected stacks below {}.\nInstaller copy: {}.",
                    if with_chamber { ", OpenChamber" } else { "" },
                    ops::REMOTE,
                    config.display(),
                    if existing {
                        format!(" (backup: {})", backup.display())
                    } else {
                        String::new()
                    },
                    projects.display(),
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
                        "\nBLOCKED: unidentified installation at {}",
                        path.display()
                    ));
                }
                cleanup_plan = Some(inventory);
                for name in &selected {
                    let path = projects.join(name).join(".envrc");
                    if fs::symlink_metadata(&path).is_ok() {
                        match stack::migrate(&projects.join(name), true) {
                            Ok(result) => preview.push_str(&format!("\n{result}")),
                            Err(error) => {
                                preview.push_str(&format!("\nSkip {}: {error}", path.display()))
                            }
                        }
                    } else if projects.join(name).join(".opencode/opencode.json").exists()
                        || projects
                            .join(name)
                            .join(".opencode/opencode.jsonc")
                            .exists()
                    {
                        preview.push_str(&format!("\nSkip existing stack config for {name}"));
                    } else if let Some((url, _)) = credentials.get(name) {
                        preview
                            .push_str(&format!("\n{}", preview_stack(&projects.join(name), url)));
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
            ui::Nav::Cancel => return Err(Cancelled.into()),
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
                        steps.push((
                            "clone config repository to ~/.config/opencode".to_string(),
                            500,
                        ));
                        for name in &selected {
                            let action = if fs::symlink_metadata(projects.join(name).join(".envrc"))
                                .is_ok()
                            {
                                "migrate existing .envrc if supported"
                            } else {
                                "create or reuse stack config"
                            };
                            steps.push((format!("{action} for {name}"), 170));
                        }
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
                    for name in selected {
                        let path = projects.join(&name);
                        let pair = credentials.remove(&name);
                        stacks.push((path, pair));
                    }
                    return Ok(Some(InstallPlan {
                        config,
                        backup,
                        existing,
                        with_chamber,
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
        with_chamber,
        stacks,
        cleanup: inventory,
    } = plan;
    let mut progress = Progress {
        done: 0,
        total: 4
            + usize::from(with_chamber)
            + stacks.len()
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
    let version = Process::new("opencode").arg("--version").output()?;
    eyre::ensure!(
        version.status.success() && cleanup::reports_v2(&String::from_utf8_lossy(&version.stdout)),
        "Installed opencode does not resolve to V2; check PATH before continuing"
    );
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
                "main",
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
    // Stage before moving ~/.config/opencode; the running binary may be inside it.
    let binary = ops::stage_binary(&std::env::current_exe()?)?;
    ops::replace_config(&config, &staged, existing.then_some(backup.as_path()))?;
    let mut migration = Vec::new();
    for (path, pair) in stacks {
        progress.show(terminal, &format!("Configuring stack {}", path.display()))?;
        if fs::symlink_metadata(path.join(".envrc")).is_ok() {
            migration.push(match stack::migrate(&path, false) {
                Ok(result) => result,
                Err(error) => format!("Skipped {}: {error}", path.display()),
            });
        } else if let Some((url, key)) = pair {
            stack::create(&path, &url, &key)?;
        }
        progress.done += 1;
    }
    progress.show(terminal, "Installation complete")?;
    notice(
        terminal,
        "Install — complete",
        &format!(
            "Installation finished.{}\nStack configs are inherited by their child projects.{}{}{}",
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
            "\nThe CLI will replace /usr/local/bin/foundry-opencode when you leave this screen.",
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
    let mut input = String::new();
    let projects = loop {
        match ui::step_text(
            terminal,
            "Projects directory",
            "Choose the existing parent of your stack folders. Update will migrate recognizable .envrc files in its immediate child directories; custom files stay unchanged.",
            &mut input,
            false,
            ui::Field::Directory(home()?.as_path()),
        )? {
            ui::Nav::Cancel | ui::Nav::Back => return Err(Cancelled.into()),
            ui::Nav::Next => {
                let path = if let Some(rest) = input.strip_prefix("~/") {
                    home()?.join(rest)
                } else {
                    PathBuf::from(&input)
                };
                if let Ok(path) = path.canonicalize()
                    && path.is_dir()
                    && !path.starts_with(config.canonicalize().unwrap_or(config.clone()))
                {
                    break path;
                }
                notice(
                    terminal,
                    "Invalid directory",
                    "Select an existing projects parent outside the OpenCode config checkout.",
                )?;
            }
            ui::Nav::Open => {}
        }
    };
    let mut migration = Vec::new();
    for entry in fs::read_dir(&projects)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && fs::symlink_metadata(entry.path().join(".envrc")).is_ok()
        {
            migration.push((entry.path(), stack::migrate(&entry.path(), true)));
        }
    }
    let summary = migration
        .iter()
        .map(|(path, result)| match result {
            Ok(message) => message.clone(),
            Err(error) => format!("Skip {}: {error}", path.display()),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut can_pull = true;
    let details = if dry_run {
        let checkout = config.clone();
        let state = match busy(terminal, "Checking config checkout", move || {
            ops::check_checkout(&checkout)
        }) {
            Ok(legacy) => format!(
                "Checkout is clean and on main. {}",
                if legacy {
                    "Would switch the config origin from SSH to HTTPS."
                } else {
                    "Config origin already uses HTTPS."
                }
            ),
            Err(error) => {
                can_pull = false;
                format!("Config pull would be blocked: {error}")
            }
        };
        format!(
            "Would run brew update; install/upgrade opencode; upgrade OpenChamber if installed; replace {}.\n{} in {}.\n{state}\nMigrate stacks under {}:\n{summary}\n\nNo modifying commands ran or files changed.",
            ops::BIN_PATH,
            if can_pull {
                "Would run git pull --ff-only origin main"
            } else {
                "Would skip git pull"
            },
            config.display(),
            projects.display()
        )
    } else {
        let checkout = config.clone();
        let legacy = busy(terminal, "Checking config checkout", move || {
            ops::check_checkout(&checkout)
        })?;
        format!(
            "Upgrade OpenCode and OpenChamber if installed, fast-forward the config checkout in {} from main, and replace {}?{}\nMigrate stacks under {}:\n{summary}",
            config.display(),
            ops::BIN_PATH,
            if legacy {
                " The config origin will switch from SSH to HTTPS."
            } else {
                ""
            },
            projects.display()
        )
    };
    if ui::step_review(terminal, "Review update", &details, dry_run, true, "Update")?
        != ui::Nav::Next
    {
        return Err(Cancelled.into());
    }
    if dry_run {
        simulate(
            terminal,
            &[
                ("refresh Homebrew metadata".to_string(), 300),
                ("install or upgrade OpenCode".to_string(), 400),
                ("upgrade OpenChamber if installed".to_string(), 300),
                ("migrate recognizable stack .envrc files".to_string(), 300),
                (
                    if can_pull {
                        "fast-forward the config checkout from main"
                    } else {
                        "skip config pull (checkout blocked)"
                    }
                    .to_string(),
                    if can_pull { 500 } else { 180 },
                ),
                ("replace the installer in /usr/local/bin".to_string(), 220),
            ],
        )?;
        notice(terminal, "Dry-run complete", &details)?;
        return Ok(None);
    }
    let home_for_check = home()?;
    let inventory = busy(terminal, "Inspecting installed tools", move || {
        cleanup::inspect(&home_for_check, false)
    })?;
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
        total: 3 + usize::from(chamber),
        output: Vec::new(),
    };
    brew_update(terminal, &mut progress)?;
    busy(
        terminal,
        "Verifying OpenCode formula",
        cleanup::latest_is_v2,
    )?;
    brew_package(terminal, &mut progress, "opencode", false)?;
    if chamber {
        let _ = optional_chamber(terminal, &mut progress);
    }
    let binary = ops::stage_binary(&std::env::current_exe()?)?;
    let checkout = config.clone();
    busy(terminal, "Checking config remote", move || {
        ops::migrate_remote(&checkout)
    })?;
    let make_pull = || {
        let mut git = Process::new("git");
        git.arg("-C")
            .arg(&config)
            .args(["pull", "--progress", "--ff-only", "origin", "main"]);
        git
    };
    run_interactive_git(
        terminal,
        &mut progress,
        "Updating configs",
        make_pull(),
        make_pull(),
        None,
    )?;
    let mut results = Vec::new();
    for (path, _) in migration {
        results.push(match stack::migrate(&path, false) {
            Ok(message) => message,
            Err(error) => format!("Skipped {}: {error}", path.display()),
        });
    }
    notice(
        terminal,
        "Update — complete",
        &format!(
            "Packages and configs are up to date. The CLI will be replaced in /usr/local/bin when you leave this screen.{}",
            if results.is_empty() {
                String::new()
            } else {
                format!("\n{}", results.join("\n"))
            }
        ),
    )?;
    Ok(Some(binary))
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
    fn dry_run_works_before_and_after_commands() {
        for args in [
            vec!["foundry-opencode", "--dry-run"],
            vec!["foundry-opencode", "--dry-run", "init"],
            vec!["foundry-opencode", "init", "--dry-run"],
            vec!["foundry-opencode", "update", "--dry-run"],
            vec!["foundry-opencode", "init-rag", "--dry-run"],
            vec!["foundry-opencode", "reset", "--dry-run"],
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
    fn install_pages_skip_existing_envrc_but_keep_selected_stacks() {
        let root = ops::backup_path(&std::env::temp_dir().join("foundry-pages"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("existing")).unwrap();
        fs::write(root.join("existing/.envrc"), "preserve").unwrap();
        let selected = vec!["existing".into(), "new".into()];
        let pages = install_pages(true, &root, &selected);
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
