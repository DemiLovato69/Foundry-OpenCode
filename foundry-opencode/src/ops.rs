use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{self, Context};
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub const REMOTE: &str = "https://github.com/DemiLovato69/Foundry-OpenCode.git";
const OLD_REMOTE: &str = "https://github.com/DemiLovato69/Foundry-OpenCode-Configs.git";
const LEGACY_REMOTE: &str = "git@github.com:DemiLovato69/Foundry-OpenCode-Configs.git";
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);
pub const BIN_PATH: &str = "/usr/local/bin/foundry-opencode";
pub const ZSH_HOOK: &str = "eval \"$(direnv hook zsh)\"";

pub struct StagedBinary(PathBuf);

impl Drop for StagedBinary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn stage_binary(source: &Path) -> io::Result<StagedBinary> {
    let path = backup_path(&std::env::temp_dir().join("foundry-opencode"));
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let staged = StagedBinary(path);
    io::copy(&mut fs::File::open(source)?, &mut out)?;
    out.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged.0, fs::Permissions::from_mode(0o755))?;
    }
    Ok(staged)
}

fn copy_binary(source: &StagedBinary, destination: &Path) -> io::Result<()> {
    let temporary = backup_path(destination);
    let mut created = false;
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        io::copy(&mut fs::File::open(&source.0)?, &mut file)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
        }
        fs::rename(&temporary, destination)
    })();
    if created {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn install_binary(source: &StagedBinary, destination: &Path) -> eyre::Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(metadata) => eyre::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "{} is not a regular binary; refusing to replace it",
            destination.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = destination
        .parent()
        .ok_or_else(|| eyre::eyre!("Invalid binary destination"))?;
    if let Ok(metadata) = fs::symlink_metadata(parent) {
        eyre::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Refusing to install into a symlinked or non-directory binary folder"
        );
    }
    if parent.is_dir() {
        match copy_binary(source, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {}
            Err(error) => return Err(error.into()),
        }
    }
    // sudo runs only after the TUI restores the terminal, so macOS can prompt normally.
    let script = "set -e; d=$(dirname \"$2\"); if [ -L \"$d\" ]; then exit 1; fi; if [ ! -d \"$d\" ]; then /usr/bin/install -d -m 0755 \"$d\"; fi; if [ -L \"$2\" ] || { [ -e \"$2\" ] && [ ! -f \"$2\" ]; }; then exit 1; fi; t=$(mktemp \"$2.tmp.XXXXXXXX\"); trap 'rm -f \"$t\"' EXIT; /usr/bin/install -m 0755 \"$1\" \"$t\"; /bin/mv -f \"$t\" \"$2\"";
    let status = Command::new("sudo")
        .args(["/bin/sh", "-c", script, "foundry-opencode"])
        .arg(&source.0)
        .arg(destination)
        .status()?;
    eyre::ensure!(
        status.success(),
        "Could not copy the binary to {}; existing configs were installed, but the CLI was not added to PATH",
        destination.display()
    );
    Ok(())
}

pub fn backup_path(path: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    path.with_file_name(format!(
        "{}.backup-{}-{stamp}-{}",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id(),
        NEXT_PATH.fetch_add(1, Ordering::Relaxed)
    ))
}

pub fn valid_url(url: &str) -> bool {
    url.strip_prefix("https://")
        .is_some_and(|host| !host.is_empty() && !host.contains(['/', '?', '#', '@', '\'', '"']))
        && !url.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn write_envrc(path: &Path, base_url: &str, api_key: &str) -> io::Result<()> {
    if !valid_url(base_url) || api_key.is_empty() || api_key.chars().any(char::is_control) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Foundry credentials",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    let result = writeln!(file, "export OPENCODE_API_KEY={}", shell_quote(api_key))
        .and_then(|_| writeln!(file, "export OPENCODE_BASE_URL={}", shell_quote(base_url)))
        .and_then(|_| writeln!(file, "opencode service restart"));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

pub fn add_gitignore(path: &Path) -> io::Result<()> {
    let existing = fs::read_to_string(path).or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    if existing.lines().any(|line| line.trim() == ".envrc") {
        return Ok(());
    }
    let mut file = OpenOptions::new().append(true).create(true).open(path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, ".envrc")
}

pub fn whitelist(path: &Path, directory: &Path) -> eyre::Result<()> {
    let original = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let mut doc = original
        .parse::<DocumentMut>()
        .wrap_err("Invalid direnv.toml; not changing it")?;
    if doc.get("whitelist").is_none() {
        doc["whitelist"] = Item::Table(Table::new());
    }
    eyre::ensure!(
        doc["whitelist"].is_table(),
        "direnv whitelist must be a table; not changing it"
    );
    if doc["whitelist"].get("prefix").is_none() {
        doc["whitelist"]["prefix"] = value(Array::new());
    }
    let entries = doc["whitelist"]["prefix"]
        .as_array_mut()
        .ok_or_else(|| eyre::eyre!("direnv whitelist.prefix must be an array; not changing it"))?;
    // The trailing slash prevents a sibling such as Projects-other matching the trusted prefix.
    let directory = format!("{}/", directory.display().to_string().trim_end_matches('/'));
    if entries
        .iter()
        .any(|entry| entry.as_str() == Some(directory.as_str()))
    {
        return Ok(());
    }
    entries.push(directory.as_str());
    let parent = path
        .parent()
        .ok_or_else(|| eyre::eyre!("Invalid direnv config path"))?;
    fs::create_dir_all(parent)?;
    let temporary = backup_path(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> io::Result<()> {
        file.write_all(doc.to_string().as_bytes())?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(())
}

pub fn needs_zsh_hook(path: &Path) -> eyre::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let original = if metadata.is_some() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    if original.lines().any(|line| {
        let line = line.trim_start();
        let compact = line.split_whitespace().collect::<String>();
        !line.starts_with('#')
            && compact.starts_with("eval")
            && (compact.contains("$(direnvhookzsh)") || compact.contains("`direnvhookzsh`"))
    }) {
        return Ok(false);
    }
    eyre::ensure!(
        !metadata
            .as_ref()
            .is_some_and(|m| m.file_type().is_symlink()),
        "{} is a symlink; add the direnv hook to its target manually",
        path.display()
    );
    eyre::ensure!(
        !metadata.as_ref().is_some_and(|m| m.is_dir()),
        "{} is a directory, not a Zsh profile",
        path.display()
    );
    Ok(true)
}

pub fn add_zsh_hook(path: &Path) -> eyre::Result<bool> {
    if !needs_zsh_hook(path)? {
        return Ok(false);
    }
    let original = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    let existed = path.exists();
    // Avoid replacing a profile that the caller could not edit in place.
    if existed {
        OpenOptions::new().write(true).open(path)?;
    }
    let temporary = backup_path(path);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    let result = (|| -> io::Result<()> {
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions())?;
        }
        file.write_all(original.as_bytes())?;
        if !original.is_empty() && !original.ends_with('\n') {
            file.write_all(b"\n")?;
        }
        writeln!(file, "{ZSH_HOOK}")?;
        file.sync_all()?;
        if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(io::Error::other(
                "Zsh profile became a symlink while updating",
            ));
        }
        match fs::read_to_string(path) {
            Ok(current) if current != original => {
                return Err(io::Error::other("Zsh profile changed while updating"));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !existed => {}
            Err(error) => return Err(error),
            _ => {}
        }
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(true)
}

pub fn replace_config(
    destination: &Path,
    staged: &Path,
    backup: Option<&Path>,
) -> eyre::Result<()> {
    if let Some(backup) = backup {
        fs::rename(destination, backup).wrap_err("Could not back up existing configs")?;
    }
    if let Err(error) = fs::rename(staged, destination) {
        if let Some(backup) = backup {
            fs::rename(backup, destination).wrap_err_with(|| {
                format!("Could not restore config backup at {}", backup.display())
            })?;
        }
        return Err(error).wrap_err("Could not install cloned configs");
    }
    Ok(())
}

pub fn check_checkout(path: &Path) -> eyre::Result<bool> {
    eyre::ensure!(
        path.join(".git").exists(),
        "{} is not a Git checkout; run install instead",
        path.display()
    );
    let run = |args: &[&str]| -> eyre::Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()?;
        eyre::ensure!(output.status.success(), "Could not inspect config checkout");
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    };
    let origin = run(&["remote", "get-url", "origin"])?;
    eyre::ensure!(
        origin == REMOTE || origin == OLD_REMOTE || origin == LEGACY_REMOTE,
        "Config origin does not match {}; not pulling",
        REMOTE
    );
    eyre::ensure!(
        run(&["branch", "--show-current"])? == "main",
        "Config checkout is not on main; not pulling"
    );
    eyre::ensure!(
        run(&["status", "--porcelain"])?.is_empty(),
        "Config checkout has local changes; commit or back them up before updating"
    );
    Ok(origin != REMOTE)
}

pub fn migrate_remote(path: &Path) -> eyre::Result<()> {
    if check_checkout(path)? {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["remote", "set-url", "origin", REMOTE])
            .status()?;
        eyre::ensure!(status.success(), "Could not switch config origin to HTTPS");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox() -> PathBuf {
        let path = backup_path(&std::env::temp_dir().join("foundry-opencode-test"));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn envrc_is_private_and_never_overwritten() {
        let root = sandbox();
        let path = root.join(".envrc");
        write_envrc(&path, "https://example.com", "a'b").unwrap();
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains("export OPENCODE_API_KEY='a'\\''b'\n"));
        assert!(write_envrc(&path, "https://other.com", "other").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(write_envrc(&root.join("bad"), "https://example.com\ncommand", "secret").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whitelist_preserves_other_settings_and_is_idempotent() {
        let root = sandbox();
        let path = root.join("direnv.toml");
        fs::write(
            &path,
            "# keep me\n[global]\nwarn_timeout = \"10s\"\n[whitelist]\nprefix = [\"/existing\"]\n",
        )
        .unwrap();
        whitelist(&path, Path::new("/Users/test/Projects")).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        assert!(once.contains("# keep me"));
        assert!(once.contains("/existing"));
        assert!(once.contains("/Users/test/Projects/"));
        whitelist(&path, Path::new("/Users/test/Projects")).unwrap();
        assert_eq!(once, fs::read_to_string(&path).unwrap());
        fs::write(&path, "[whitelist]\nprefix = 42\n").unwrap();
        assert!(whitelist(&path, Path::new("/Users/test/Projects")).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[whitelist]\nprefix = 42\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zsh_hook_preserves_profile_and_is_idempotent() {
        let root = sandbox();
        let path = root.join(".zshrc");
        fs::write(&path, "# existing settings\nexport EDITOR=vim").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        assert!(needs_zsh_hook(&path).unwrap());
        assert!(add_zsh_hook(&path).unwrap());
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(
            contents,
            format!("# existing settings\nexport EDITOR=vim\n{ZSH_HOOK}\n")
        );
        assert!(!needs_zsh_hook(&path).unwrap());
        assert!(!add_zsh_hook(&path).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zsh_hook_skips_existing_and_rejects_unmodified_symlinks() {
        let root = sandbox();
        let target = root.join("target");
        fs::write(&target, format!("{ZSH_HOOK}\n")).unwrap();
        let link = root.join(".zshrc");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(!add_zsh_hook(&link).unwrap());
            fs::write(&target, "# no active hook\n").unwrap();
            assert!(needs_zsh_hook(&link).is_err());
            assert_eq!(fs::read_to_string(&target).unwrap(), "# no active hook\n");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn zsh_hook_creates_missing_profile_privately() {
        let root = sandbox();
        let path = root.join(".zshrc");
        assert!(add_zsh_hook(&path).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{ZSH_HOOK}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn commented_out_hook_does_not_count_as_configured() {
        let root = sandbox();
        let path = root.join(".zshrc");
        fs::write(&path, format!("# {ZSH_HOOK}\n")).unwrap();
        assert!(needs_zsh_hook(&path).unwrap());
        assert!(add_zsh_hook(&path).unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("# {ZSH_HOOK}\n{ZSH_HOOK}\n")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gitignore_adds_rule_once_without_discarding_contents() {
        let root = sandbox();
        let path = root.join(".gitignore");
        fs::write(&path, "# keep\nother").unwrap();
        add_gitignore(&path).unwrap();
        add_gitignore(&path).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# keep\nother\n.envrc\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_replacement_keeps_a_backup() {
        let root = sandbox();
        let dest = root.join("opencode");
        let staged = root.join("stage");
        let backup = root.join("backup");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("mine"), "keep").unwrap();
        fs::create_dir(&staged).unwrap();
        replace_config(&dest, &staged, Some(&backup)).unwrap();
        assert_eq!(fs::read_to_string(backup.join("mine")).unwrap(), "keep");
        assert!(dest.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_failure_restores_original() {
        let root = sandbox();
        let dest = root.join("opencode");
        let backup = root.join("backup");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("mine"), "keep").unwrap();
        assert!(replace_config(&dest, &root.join("missing"), Some(&backup)).is_err());
        assert_eq!(fs::read_to_string(dest.join("mine")).unwrap(), "keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_rejects_dirty_checkout() {
        let root = sandbox();
        assert!(
            Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["remote", "add", "origin", REMOTE])
                .status()
                .unwrap()
                .success()
        );
        fs::write(root.join("mine"), "local work").unwrap();
        assert!(check_checkout(&root).is_err());
        fs::remove_file(root.join("mine")).unwrap();
        assert!(check_checkout(&root).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_migrates_only_clean_old_origins() {
        let root = sandbox();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q", "-b", "main"]).status.success());
        assert!(
            git(&["remote", "add", "origin", LEGACY_REMOTE])
                .status
                .success()
        );
        let origin = || String::from_utf8(git(&["remote", "get-url", "origin"]).stdout).unwrap();
        assert!(check_checkout(&root).unwrap());
        fs::write(root.join("mine"), "local work").unwrap();
        assert!(migrate_remote(&root).is_err());
        assert_eq!(origin().trim(), LEGACY_REMOTE);
        fs::remove_file(root.join("mine")).unwrap();
        migrate_remote(&root).unwrap();
        assert!(!check_checkout(&root).unwrap());
        migrate_remote(&root).unwrap();
        assert_eq!(origin().trim(), REMOTE);
        assert!(
            git(&["remote", "set-url", "origin", OLD_REMOTE])
                .status
                .success()
        );
        assert!(check_checkout(&root).unwrap());
        migrate_remote(&root).unwrap();
        assert_eq!(origin().trim(), REMOTE);
        assert!(
            git(&[
                "remote",
                "set-url",
                "origin",
                "https://example.com/other.git"
            ])
            .status
            .success()
        );
        assert!(migrate_remote(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn binary_copy_is_executable_and_replaces_existing_file() {
        let root = sandbox();
        let source = root.join("source");
        let destination = root.join("foundry-opencode");
        fs::write(&source, b"binary").unwrap();
        let staged = stage_binary(&source).unwrap();
        install_binary(&staged, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"binary");
        fs::write(&destination, b"outdated").unwrap();
        install_binary(&staged, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"binary");
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(&destination, &link).unwrap();
            assert!(install_binary(&staged, &link).is_err());
            assert_eq!(fs::read(&destination).unwrap(), b"binary");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        let temporary = staged.0.clone();
        drop(staged);
        assert!(!temporary.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
