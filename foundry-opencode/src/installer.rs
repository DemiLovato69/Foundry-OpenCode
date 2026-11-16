use std::collections::BTreeSet;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use color_eyre::eyre::{self, Context};

use crate::{cleanup, ops, projects, rag, worktrees};

pub struct Plan {
    pub home: PathBuf,
    pub catalog: projects::Catalog,
    pub openchamber: bool,
    pub cli: PathBuf,
    pub dry_run: bool,
    pub reviewed: cleanup::Inventory,
}

#[derive(Default)]
pub struct Checkpoint {
    pub packages: bool,
    pub chamber: bool,
    pub global: bool,
    pub configured: BTreeSet<PathBuf>,
    pub projects: BTreeSet<PathBuf>,
}

fn command(
    command: &mut Command,
    label: &str,
    report: &mut impl FnMut(String),
    cancelled: &impl Fn() -> bool,
) -> eyre::Result<()> {
    report(label.into());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .wrap_err_with(|| format!("Could not start {label}"))?;
    let (tx, rx) = mpsc::channel();
    for stream in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    drop(tx);
    let status = loop {
        for line in rx.try_iter() {
            report(line);
        }
        // Homebrew can be left inconsistent if killed mid-upgrade; finish that command first.
        if cancelled()
            && ![
                "Refreshing Homebrew",
                "Updating opencode",
                "Updating openchamber",
            ]
            .contains(&label)
        {
            #[cfg(unix)]
            if child.try_wait()?.is_none() {
                // Stop npm's descendants as well as its parent before reporting cancellation.
                unsafe { libc::killpg(child.id() as i32, libc::SIGKILL) };
            }
            #[cfg(not(unix))]
            if child.try_wait()?.is_none() {
                child.kill()?;
            }
            child.wait()?;
            eyre::bail!("{label} cancelled");
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for line in rx.try_iter() {
        report(line);
    }
    eyre::ensure!(status.success(), "{label} failed ({status})");
    eyre::ensure!(
        !cancelled(),
        "{label} completed; installation cancelled before the next step"
    );
    Ok(())
}

fn installed(name: &str, cask: bool) -> eyre::Result<bool> {
    let mut cmd = Command::new("brew");
    cmd.arg("list");
    if cask {
        cmd.arg("--cask");
    }
    Ok(cmd.arg("--versions").arg(name).output()?.status.success())
}

fn brew(
    name: &str,
    cask: bool,
    report: &mut impl FnMut(String),
    cancelled: &impl Fn() -> bool,
) -> eyre::Result<()> {
    let exists = installed(name, cask)?;
    let mut cmd = Command::new("brew");
    cmd.arg(if exists { "upgrade" } else { "install" });
    if cask {
        cmd.arg("--cask");
    }
    command(
        cmd.arg(name),
        &format!("Updating {name}"),
        report,
        cancelled,
    )
}

pub fn preflight(plan: &Plan) -> eyre::Result<()> {
    eyre::ensure!(cfg!(target_os = "macos"), "Installation requires macOS");
    eyre::ensure!(
        unsafe { libc::geteuid() } != 0,
        "Do not run Homebrew or the installer as root"
    );
    plan.catalog.validate(&plan.home)?;
    for project in &plan.catalog.projects {
        projects::preflight(&project.path)?;
    }
    if plan.catalog.projects.iter().any(|project| project.rag) {
        for tool in ["node", "npm"] {
            eyre::ensure!(
                Command::new(tool)
                    .arg("--version")
                    .output()?
                    .status
                    .success(),
                "{tool} is required for RAG setup"
            );
        }
    }
    eyre::ensure!(
        Command::new("brew")
            .arg("--version")
            .output()?
            .status
            .success(),
        "Homebrew is required"
    );
    cleanup::latest_is_v2()?;
    eyre::ensure!(
        cleanup::inspect(&plan.home, plan.openchamber)?.within(&plan.reviewed),
        "Installed packages changed since review; review the migration again before retrying"
    );
    worktrees::alias_plan(plan.catalog.sync_alias, None)?;
    let output = Command::new(&plan.cli).arg("capabilities").output()?;
    eyre::ensure!(
        output.status.success() && output.stdout == b"foundry-opencode-headless-v1\n",
        "Matching headless CLI payload is required before installation"
    );
    Ok(())
}

pub fn apply(
    plan: &Plan,
    checkpoint: &mut Checkpoint,
    mut emit: impl FnMut(String),
    cancelled: impl Fn() -> bool,
    install_cli: impl FnOnce(&Path) -> eyre::Result<()>,
) -> eyre::Result<Vec<String>> {
    preflight(plan)?;
    let mut report = |mut line: String| {
        for token in plan
            .catalog
            .tokens
            .values()
            .filter(|token| !token.is_empty())
        {
            line = line.replace(token, "[redacted]");
        }
        emit(line);
    };
    let mut results = Vec::new();
    report("Validated saved configuration and CLI payload".into());
    for path in &plan.reviewed.unknown {
        report(format!(
            "Leaving unrecognized executable untouched: {}",
            path.display()
        ));
    }
    if plan.dry_run {
        for name in &plan.reviewed.brew_v1 {
            report(format!("Would uninstall legacy Homebrew {name}"));
        }
        if plan.reviewed.brew_v2 {
            report("Would uninstall legacy Homebrew opencode-v2".into());
        }
        for name in &plan.reviewed.npm_v1 {
            report(format!("Would uninstall legacy global npm {name}"));
        }
        for path in &plan.reviewed.files {
            report(format!("Would move {} to Trash", path.display()));
        }
        if plan.reviewed.chamber {
            report("Would move unmanaged OpenChamber.app to Trash".into());
        }
        for project in &plan.catalog.projects {
            report(format!(
                "Would configure {} (RAG: {})",
                project.path.display(),
                project.rag
            ));
        }
        report("Would install the CLI last".into());
        return Ok(results);
    }
    eyre::ensure!(!cancelled(), "Installation cancelled before changes began");
    if !checkpoint.packages || !installed("opencode", false)? {
        command(
            Command::new("brew").arg("update"),
            "Refreshing Homebrew",
            &mut report,
            &cancelled,
        )?;
        cleanup::latest_is_v2()?;
        let legacy = cleanup::inspect(&plan.home, plan.openchamber)?;
        eyre::ensure!(
            legacy.within(&plan.reviewed),
            "Installed packages changed after Homebrew refresh; review the migration again before removing anything"
        );
        for name in legacy
            .brew_v1
            .iter()
            .map(String::as_str)
            .chain(legacy.brew_v2.then_some("opencode-v2"))
        {
            command(
                Command::new("brew").args(["uninstall", name]),
                &format!("Uninstalling legacy Homebrew {name}"),
                &mut report,
                &cancelled,
            )?;
        }
        for name in &legacy.npm_v1 {
            command(
                Command::new("npm").args(["uninstall", "-g", name]),
                &format!("Uninstalling legacy global npm {name}"),
                &mut report,
                &cancelled,
            )?;
        }
        for path in &legacy.files {
            eyre::ensure!(
                fs::symlink_metadata(path)?.is_file()
                    && !fs::symlink_metadata(path)?.file_type().is_symlink(),
                "Legacy executable changed since review: {}",
                path.display()
            );
            let moved = cleanup::move_to_trash(path, &plan.home)?;
            report(format!("Backup: {}", moved.display()));
            results.push(format!("Moved legacy executable to {}", moved.display()));
        }
        if legacy.chamber {
            let moved =
                cleanup::move_to_trash(Path::new("/Applications/OpenChamber.app"), &plan.home)?;
            report(format!("Backup: {}", moved.display()));
            results.push(format!(
                "Moved unmanaged OpenChamber to {}",
                moved.display()
            ));
        }
        brew("opencode", false, &mut report, &cancelled)?;
        checkpoint.packages = true;
    }
    if plan.openchamber && !checkpoint.chamber {
        if Command::new("pgrep")
            .args(["-x", "OpenChamber"])
            .output()?
            .status
            .success()
        {
            results.push("OpenChamber is running; its optional upgrade was skipped".into());
        } else if let Err(error) = brew("openchamber", true, &mut report, &cancelled) {
            results.push(format!("OpenChamber upgrade incomplete: {error}"));
        } else {
            checkpoint.chamber = true;
        }
    }
    eyre::ensure!(
        !cancelled(),
        "Installation cancelled before config replacement"
    );
    let config = plan.home.join(".config/opencode");
    fs::create_dir_all(config.parent().unwrap())?;
    if !checkpoint.global || !config.is_dir() {
        let staged = ops::backup_path(&config);
        if let Err(error) = command(
            Command::new("git")
                .args(["clone", "--depth", "1", "--branch", "main", ops::REMOTE])
                .arg(&staged),
            "Cloning global OpenCode configuration",
            &mut report,
            &cancelled,
        ) {
            if staged.is_dir() {
                fs::remove_dir_all(&staged)?;
            }
            return Err(error);
        }
        if let Some(global) = &plan.catalog.default {
            ops::prepare_global(&staged, &global.url, &plan.catalog.tokens[&global.id])?;
        }
        let backup = fs::symlink_metadata(&config)
            .is_ok()
            .then(|| ops::backup_path(&config));
        ops::replace_config(&config, &staged, backup.as_deref())?;
        if let Some(path) = backup {
            results.push(format!("Global config backup: {}", path.display()));
            report(format!("Backup: {}", path.display()));
        }
        checkpoint.global = true;
    }
    for project in &plan.catalog.projects {
        eyre::ensure!(
            !cancelled(),
            "Installation cancelled before {}",
            project.path.display()
        );
        let stack = plan
            .catalog
            .choices()
            .into_iter()
            .find(|item| item.id == project.stack)
            .ok_or_else(|| eyre::eyre!("Unknown stack"))?;
        let secret = &plan.catalog.tokens[&stack.id];
        let expected = projects::project_config(&plan.home, stack);
        let valid_config = checkpoint.configured.contains(&project.path)
            && crate::stack::read_config(&project.path.join(".opencode/opencode.json")).is_ok_and(
                |doc| {
                    doc["providers"]["palantir-openai"]["settings"]
                        == expected["providers"]["palantir-openai"]["settings"]
                        && doc["mcp"]["servers"]["rag"].is_object()
                            == (project.rag && rag::ready(&project.path, &stack.url))
                },
            )
            && projects::private_token(&project.path.join(".opencode/token"))
                .is_ok_and(|token| token == *secret);
        if checkpoint.projects.contains(&project.path)
            && valid_config
            && (!project.rag || rag::ready(&project.path, &stack.url))
        {
            report(format!("Already configured {}", project.path.display()));
            continue;
        }
        if !valid_config {
            report(format!("Configuring {}", project.path.display()));
            for backup in projects::write_project(&plan.home, project, stack, secret)? {
                results.push(format!("Backup: {}", backup.display()));
                report(format!("Backup: {}", backup.display()));
            }
            checkpoint.configured.insert(project.path.clone());
        }
        if project.rag && !rag::ready(&project.path, &stack.url) {
            rag::setup(&project.path, &stack.url, false, |label, mut cmd| {
                cmd.env("OPENAI_API_KEY", secret);
                command(&mut cmd, label, &mut report, &cancelled)
            })?;
        }
        if projects::is_repo(&project.path).unwrap_or(false) {
            for message in worktrees::share(&project.path)? {
                results.push(message);
            }
        }
        checkpoint.projects.insert(project.path.clone());
    }
    plan.catalog.save(&plan.home)?;
    if let Some(alias) = worktrees::set_alias(plan.catalog.sync_alias, None)? {
        results.push(alias);
    }
    eyre::ensure!(
        !cancelled(),
        "Installation cancelled before CLI replacement"
    );
    report("Installing CLI as final step".into());
    install_cli(&plan.cli)?;
    results.push("CLI installed as final step".into());
    Ok(results)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn cancelling_command_stops_child_process_group() {
        let folder = ops::backup_path(&std::env::temp_dir().join("installer-cancel"));
        fs::create_dir(&folder).unwrap();
        let marker = folder.join("child-finished");
        let start = std::time::Instant::now();
        let result = command(
            Command::new("sh")
                .arg("-c")
                .arg("(sleep 1; touch child-finished) & sleep 5")
                .current_dir(&folder),
            "test process",
            &mut |_| {},
            &|| start.elapsed() > Duration::from_millis(150),
        );
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(1100));
        assert!(!marker.exists());
        fs::remove_dir_all(folder).unwrap();
    }
}
