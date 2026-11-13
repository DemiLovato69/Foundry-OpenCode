use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{CommandFactory, Parser, Subcommand};
use color_eyre::eyre;
use foundry_opencode::{installer, ops, projects, rag, stack, worktrees};

#[derive(Parser)]
#[command(
    about = "Foundry OpenCode command-line utility; use foundry-opencode-gui for guided setup"
)]
struct Args {
    #[arg(long, global = true)]
    dry_run: bool,
    #[command(subcommand)]
    command: Option<Mode>,
}

#[derive(Subcommand)]
enum Mode {
    Capabilities,
    Version,
    Usage,
    Install {
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        migrate_legacy: bool,
    },
    Update {
        #[arg(short, long)]
        yes: bool,
        #[arg(long)]
        migrate_legacy: bool,
    },
    Init {
        #[arg(long)]
        stack: Option<String>,
    },
    InitRag,
    RepairPlugins {
        #[arg(long)]
        yes: bool,
    },
    Reset {
        #[arg(long)]
        yes: bool,
    },
    Uninstall {
        #[arg(long)]
        yes: bool,
    },
    SyncWorktrees {
        #[arg(long)]
        gen_rag: bool,
    },
    AddWorktree {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        options: Vec<String>,
    },
    GenerateRag {
        #[arg(long)]
        all: bool,
    },
    Exec {
        #[arg(long)]
        foundry_token: bool,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<std::ffi::OsString>,
    },
}

fn home() -> eyre::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("HOME").ok_or_else(|| eyre::eyre!("HOME is not set"))?,
    ))
}

fn exec(foundry_token: bool, command: &[std::ffi::OsString]) -> eyre::Result<()> {
    let mut child = Command::new(&command[0]);
    child.args(&command[1..]);
    if foundry_token {
        let cwd = std::env::current_dir()?;
        let folder = cwd
            .ancestors()
            .find(|path| path.join(".opencode/opencode.json").exists())
            .ok_or_else(|| eyre::eyre!("No configured Foundry folder for {}", cwd.display()))?;
        let config = stack::read_config(&folder.join(".opencode/opencode.json"))?;
        eyre::ensure!(
            config["providers"]["palantir-openai"]["settings"]["apiKey"] == "{file:./token}",
            "{} needs a local token; migrate this folder",
            folder.display()
        );
        child.env(
            "FOUNDRY_TOKEN",
            projects::private_token(&folder.join(".opencode/token"))?,
        );
    }
    let status = child.status()?;
    std::process::exit(status.code().unwrap_or(1));
}

fn configure_current(stack_id: Option<String>, dry_run: bool) -> eyre::Result<()> {
    let home = home()?;
    let mut catalog = projects::Catalog::load(&home)?;
    let path = std::env::current_dir()?.canonicalize()?;
    projects::preflight(&path)?;
    let id = stack_id
        .or_else(|| {
            catalog
                .projects
                .iter()
                .find(|project| project.path == path)
                .map(|project| project.stack.clone())
        })
        .or_else(|| (catalog.choices().len() == 1).then(|| catalog.choices()[0].id.clone()))
        .ok_or_else(|| {
            eyre::eyre!("Choose a stack with --stack <id> or configure this folder in the GUI")
        })?;
    let chosen = catalog
        .choices()
        .into_iter()
        .find(|stack| stack.id == id)
        .ok_or_else(|| eyre::eyre!("Unknown stack {id}"))?;
    let secret = catalog
        .tokens
        .get(&id)
        .ok_or_else(|| eyre::eyre!("Missing private token for stack {id}"))?;
    let project = projects::Project {
        path: path.clone(),
        stack: id.clone(),
        rag: false,
    };
    println!(
        "{} {} for {}",
        if dry_run {
            "Would configure"
        } else {
            "Configuring"
        },
        path.display(),
        chosen.name
    );
    if !dry_run {
        projects::write_project(&home, &project, chosen, secret)?;
        catalog.projects.retain(|item| item.path != path);
        catalog.projects.push(project);
        catalog.save(&home)?;
    }
    Ok(())
}

fn init_rag(dry_run: bool) -> eyre::Result<()> {
    let project = std::env::current_dir()?;
    let (host, token) = stack::credentials(&project)?;
    let message = rag::setup(&project, &host, dry_run, |label, mut command| {
        command.env("OPENAI_API_KEY", &token);
        eyre::ensure!(command.status()?.success(), "{label} failed");
        Ok(())
    })?;
    println!("{message}");
    Ok(())
}

fn repair(dry_run: bool, yes: bool) -> eyre::Result<()> {
    let project = std::env::current_dir()?;
    if let Some(plugin) = rag::legacy_plugin(&project)? {
        let backup = ops::backup_path(&project.join(".opencode/rag-plugin.js"));
        println!(
            "{} {} to {}",
            if dry_run { "Would move" } else { "Move" },
            plugin.display(),
            backup.display()
        );
        if !dry_run {
            eyre::ensure!(yes, "Pass --yes after reviewing the path");
            std::fs::rename(plugin, backup)?;
        }
    } else {
        println!("No known V1 RAG wrapper found");
    }
    Ok(())
}

fn remove_safe(path: &Path) -> eyre::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
        Ok(meta) if meta.file_type().is_symlink() => {
            eyre::bail!("Refusing to remove symlink {}", path.display())
        }
        Ok(meta) if meta.is_dir() => Ok(std::fs::remove_dir_all(path)?),
        Ok(meta) if meta.is_file() => Ok(std::fs::remove_file(path)?),
        Ok(_) => eyre::bail!("Unsafe path {}", path.display()),
    }
}

fn quiet_opencode() -> eyre::Result<()> {
    let uid = unsafe { libc::geteuid() }.to_string();
    for name in ["opencode", "OpenChamber"] {
        eyre::ensure!(
            !Command::new("pgrep")
                .args(["-U", &uid, "-x", name])
                .status()?
                .success(),
            "{name} is still running; user data was not removed"
        );
    }
    Ok(())
}

fn stop_processes() -> eyre::Result<()> {
    let uid = unsafe { libc::geteuid() }.to_string();
    for signal in ["-TERM", "-KILL"] {
        for name in ["opencode", "OpenChamber"] {
            let status = Command::new("pkill")
                .args([signal, "-U", &uid, "-x", name])
                .status()?;
            eyre::ensure!(
                status.success() || status.code() == Some(1),
                "Could not stop {name}"
            );
        }
        if signal == "-TERM" {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    quiet_opencode()
}

fn reset(dry_run: bool, yes: bool) -> eyre::Result<()> {
    let cache = home()?.join(".cache/opencode");
    println!(
        "{} OpenCode and OpenChamber processes",
        if dry_run { "Would stop" } else { "Stop" }
    );
    println!(
        "{} {} (sessions and configuration are preserved)",
        if dry_run { "Would remove" } else { "Remove" },
        cache.display()
    );
    if dry_run {
        return Ok(());
    }
    eyre::ensure!(yes, "Pass --yes to confirm cache removal");
    let _ = Command::new("opencode").args(["service", "stop"]).status();
    stop_processes()?;
    remove_safe(&cache)
}

fn warn_token_backups(dir: &Path) -> eyre::Result<()> {
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("token.backup-"))
            {
                eprintln!(
                    "Private recovery token remains at {}; inspect and remove it manually if no longer needed",
                    path.display()
                );
            }
        }
    }
    Ok(())
}

fn uninstall(dry_run: bool, yes: bool) -> eyre::Result<()> {
    let home = home()?;
    let catalog = projects::Catalog::inventory(&home)?;
    let npm_packages = foundry_opencode::cleanup::npm_opencode()?;
    let targets = [
        home.join(".config/opencode"),
        home.join(".config/foundry-opencode"),
        home.join(".cache/opencode"),
        home.join(".local/share/opencode"),
        home.join(".local/state/opencode"),
    ];
    for project in &catalog.projects {
        println!(
            "{} managed config and token in {}",
            if dry_run { "Would remove" } else { "Remove" },
            project.path.display()
        );
    }
    for path in &targets {
        println!(
            "{} {}",
            if dry_run { "Would remove" } else { "Remove" },
            path.display()
        );
    }
    println!(
        "{} Homebrew opencode/opencode-v1/opencode-v2/OpenChamber, global npm {:?}, and {}",
        if dry_run { "Would remove" } else { "Remove" },
        npm_packages,
        ops::BIN_PATH
    );
    if dry_run {
        return Ok(());
    }
    eyre::ensure!(yes, "Pass --yes after reviewing with uninstall --dry-run");
    for project in &catalog.projects {
        projects::preflight(&project.path)?;
    }
    for path in &targets {
        match std::fs::symlink_metadata(path) {
            Ok(meta) => eyre::ensure!(
                !meta.file_type().is_symlink(),
                "Unsafe uninstall path {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    match std::fs::symlink_metadata(ops::BIN_PATH) {
        Ok(meta) => eyre::ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Unsafe CLI binary"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let _ = Command::new("opencode").args(["service", "stop"]).status();
    stop_processes()?;
    for (name, cask) in [
        ("opencode", false),
        ("opencode-v1", false),
        ("opencode-v2", false),
        ("openchamber", true),
    ] {
        let mut list = Command::new("brew");
        list.arg("list");
        if cask {
            list.arg("--cask");
        }
        if list.args(["--versions", name]).status()?.success() {
            let mut command = Command::new("brew");
            command.arg("uninstall");
            if cask {
                command.arg("--cask");
            }
            eyre::ensure!(
                command.arg(name).status()?.success(),
                "Could not uninstall {name}"
            );
        }
    }
    for name in npm_packages {
        eyre::ensure!(
            Command::new("npm")
                .args(["uninstall", "-g", &name])
                .status()?
                .success(),
            "Could not uninstall global npm {name}"
        );
    }
    for project in &catalog.projects {
        if projects::is_repo(&project.path).unwrap_or(false) {
            for worktree in worktrees::linked(&project.path) {
                worktrees::unlink(&project.path, &worktree)?;
                remove_safe(&worktree.join(".opencode/token"))?;
                remove_safe(&worktree.join(".opencode/opencode.json"))?;
                warn_token_backups(&worktree.join(".opencode"))?;
            }
            worktrees::unexclude(&project.path)?;
        }
        let mut names = vec!["token", "opencode.json"];
        if project.rag {
            names.extend([
                "rag-cli.mjs",
                "rag.json",
                "patch-opencode-rag.mjs",
                "package.json",
                "package-lock.json",
                "rag_db",
                "node_modules",
            ]);
        }
        for name in names {
            remove_safe(&project.path.join(".opencode").join(name))?;
        }
        let ignore = project.path.join(".opencode/.gitignore");
        if std::fs::read_to_string(&ignore)
            .is_ok_and(|text| text == "*\n!commands/\n!commands/**\n")
        {
            remove_safe(&ignore)?;
        }
        warn_token_backups(&project.path.join(".opencode"))?;
    }
    worktrees::set_alias(false, None)?;
    for path in targets {
        remove_safe(&path)?;
    }
    let binary = Path::new(ops::BIN_PATH);
    if binary.exists() {
        eyre::ensure!(
            std::fs::symlink_metadata(binary)?.is_file()
                && !std::fs::symlink_metadata(binary)?.file_type().is_symlink(),
            "Unsafe CLI binary"
        );
        if let Err(error) = std::fs::remove_file(binary) {
            eyre::ensure!(
                error.kind() == std::io::ErrorKind::PermissionDenied
                    && Command::new("sudo")
                        .arg("/bin/rm")
                        .arg(binary)
                        .status()?
                        .success(),
                "Could not remove CLI: {error}"
            );
        }
    }
    Ok(())
}

fn update(dry_run: bool, yes: bool, migrate_legacy: bool) -> eyre::Result<()> {
    let home = home()?;
    let catalog = projects::Catalog::saved(&home)?;
    eyre::ensure!(
        yes || dry_run,
        "Pass --yes to accept saved settings or use the GUI to edit them"
    );
    let reviewed = foundry_opencode::cleanup::inspect(&home, false)?;
    eyre::ensure!(
        dry_run || !reviewed.migration_needed() || migrate_legacy,
        "Legacy packages need review: run update --dry-run, then update --yes --migrate-legacy"
    );
    let staged = if dry_run {
        None
    } else {
        Some(ops::stage_latest_binary()?.0)
    };
    let cli = staged.as_ref().map_or(std::env::current_exe()?, |binary| {
        binary.path().to_path_buf()
    });
    let plan = installer::Plan {
        home,
        catalog,
        openchamber: false,
        cli,
        dry_run,
        reviewed,
    };
    let mut checkpoint = installer::Checkpoint::default();
    for line in installer::apply(
        &plan,
        &mut checkpoint,
        |line| println!("{line}"),
        || false,
        |path| {
            let staged = ops::stage_binary(path)?;
            ops::install_binary(&staged, Path::new(ops::BIN_PATH))
        },
    )? {
        println!("{line}");
    }
    Ok(())
}

fn main() -> eyre::Result<()> {
    let args = Args::parse();
    color_eyre::install()?;
    match args.command {
        Some(Mode::Capabilities) => println!("foundry-opencode-headless-v1"),
        Some(Mode::Version) => println!(
            "{}",
            option_env!("FOUNDRY_OPENCODE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
        ),
        Some(Mode::Usage) | None => Args::command().print_help()?,
        Some(Mode::Exec {
            foundry_token,
            command,
        }) => {
            eyre::ensure!(!args.dry_run, "exec cannot run during a dry run");
            exec(foundry_token, &command)?;
        }
        Some(
            Mode::Install {
                yes,
                migrate_legacy,
            }
            | Mode::Update {
                yes,
                migrate_legacy,
            },
        ) => update(args.dry_run, yes, migrate_legacy)?,
        Some(Mode::Init { stack }) => configure_current(stack, args.dry_run)?,
        Some(Mode::InitRag) => init_rag(args.dry_run)?,
        Some(Mode::RepairPlugins { yes }) => repair(args.dry_run, yes)?,
        Some(Mode::SyncWorktrees { gen_rag }) => eyre::ensure!(
            worktrees::sync(&std::env::current_dir()?, args.dry_run, gen_rag)?,
            "Worktree sync incomplete"
        ),
        Some(Mode::AddWorktree { options }) => {
            let gen_rag = options.iter().any(|option| option == "--gen-rag");
            let dry_run = args.dry_run || options.iter().any(|option| option == "--dry-run");
            let git_options = options
                .into_iter()
                .filter(|option| option != "--gen-rag" && option != "--dry-run")
                .collect::<Vec<_>>();
            eyre::ensure!(
                worktrees::add(&std::env::current_dir()?, &git_options, gen_rag, dry_run)?,
                "Worktree creation incomplete"
            );
        }
        Some(Mode::GenerateRag { all }) => eyre::ensure!(
            worktrees::generate(&std::env::current_dir()?, all, args.dry_run)?,
            "RAG generation incomplete"
        ),
        Some(Mode::Reset { yes }) => reset(args.dry_run, yes)?,
        Some(Mode::Uninstall { yes }) => uninstall(args.dry_run, yes)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_cli_parses_without_a_terminal() {
        assert!(matches!(
            Args::try_parse_from(["foundry-opencode", "update", "-y"])
                .unwrap()
                .command,
            Some(Mode::Update { yes: true, .. })
        ));
        assert!(matches!(
            Args::try_parse_from([
                "foundry-opencode",
                "exec",
                "--foundry-token",
                "--",
                "npm",
                "install"
            ])
            .unwrap()
            .command,
            Some(Mode::Exec {
                foundry_token: true,
                ..
            })
        ));
    }
}
