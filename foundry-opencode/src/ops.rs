use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use color_eyre::eyre::{self, Context};

pub const REMOTE: &str = "https://github.com/DemiLovato69/Foundry-OpenCode.git";
static NEXT_PATH: AtomicU64 = AtomicU64::new(0);
pub const BIN_PATH: &str = "/usr/local/bin/foundry-opencode";

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

pub fn replace_config(
    destination: &Path,
    staged: &Path,
    backup: Option<&Path>,
) -> eyre::Result<()> {
    #[cfg(target_os = "macos")]
    if let Some(backup) = backup {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> i32;
        }
        let from = CString::new(destination.as_os_str().as_bytes())?;
        let to = CString::new(staged.as_os_str().as_bytes())?;
        // Swap atomically so plugin paths inside the global config never disappear.
        eyre::ensure!(
            unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 0x00000002) } == 0,
            "Could not swap configs: {}",
            io::Error::last_os_error()
        );
        fs::rename(staged, backup).wrap_err_with(|| {
            format!(
                "New config is installed; old config remains at {}",
                staged.display()
            )
        })?;
        return Ok(());
    }
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

pub fn global_credentials(config: &Path) -> eyre::Result<Option<(String, String)>> {
    let token = config.join("token");
    if !token.exists() {
        return Ok(None);
    }
    let meta = fs::symlink_metadata(&token)?;
    eyre::ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "Unsafe global token file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        eyre::ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Global token is not private"
        );
    }
    let doc = crate::stack::read_config(&config.join("opencode.jsonc"))?;
    let url = doc["providers"]["palantir-openai"]["settings"]["baseURL"]
        .as_str()
        .and_then(|value| value.strip_suffix("/language-model-service/api/proxy/openai/v1"))
        .ok_or_else(|| eyre::eyre!("Cannot recover global Foundry host"))?;
    eyre::ensure!(valid_url(url), "Invalid global Foundry host");
    let key = fs::read_to_string(token)?;
    eyre::ensure!(!key.is_empty(), "Empty global Foundry token");
    Ok(Some((url.to_owned(), key)))
}

pub fn prepare_global(config: &Path, url: &str, key: &str) -> eyre::Result<()> {
    eyre::ensure!(
        valid_url(url) && !key.is_empty() && !key.chars().any(char::is_control),
        "Invalid global credentials"
    );
    let path = config.join("opencode.jsonc");
    let mut doc = crate::stack::read_config(&path)?;
    let mut overrides = crate::stack::config(url);
    // The token reference is relative to the global config, not a stack's .opencode directory.
    for provider in ["palantir-openai", "palantir-anthropic"] {
        let existing = doc["providers"][provider]
            .as_object_mut()
            .ok_or_else(|| eyre::eyre!("Missing {provider} provider in bundled config"))?;
        let settings = existing
            .entry("settings")
            .or_insert_with(|| serde_json::json!({}));
        let values = overrides["providers"][provider]["settings"]
            .as_object()
            .unwrap();
        for (name, value) in values {
            settings[name] = value.clone();
        }
        existing.insert(
            "headers".into(),
            overrides["providers"][provider]["headers"].clone(),
        );
    }
    doc["mcp"]["servers"]["palantir"] = overrides["mcp"]["servers"]["palantir"].take();
    let token = config.join("token");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600))?;
    }
    use std::io::Write;
    file.write_all(key.as_bytes())?;
    file.sync_all()?;
    writeln!(
        OpenOptions::new()
            .append(true)
            .open(config.join(".git/info/exclude"))?,
        "token"
    )?;
    fs::write(path, serde_json::to_string_pretty(&doc)?)?;
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
    fn global_fallback_survives_fresh_config_replacement() {
        let root = sandbox();
        let config = root.join("opencode");
        fs::create_dir_all(config.join(".git/info")).unwrap();
        fs::write(config.join(".git/info/exclude"), "keep\n").unwrap();
        fs::write(config.join("opencode.jsonc"), r#"{"providers":{"palantir-openai":{"settings":{"transport":"http"}},"palantir-anthropic":{"settings":{}}},"mcp":{"servers":{}}}"#).unwrap();
        prepare_global(&config, "https://example.com", "private").unwrap();
        assert_eq!(
            global_credentials(&config).unwrap().unwrap(),
            ("https://example.com".into(), "private".into())
        );
        assert!(
            fs::read_to_string(config.join(".git/info/exclude"))
                .unwrap()
                .contains("keep")
        );
        assert_eq!(
            crate::stack::read_config(&config.join("opencode.jsonc")).unwrap()["providers"]["palantir-openai"]
                ["settings"]["transport"],
            "http"
        );
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

    #[cfg(target_os = "macos")]
    #[test]
    fn config_swap_preserves_plugin_path() {
        let root = sandbox();
        let dest = root.join("opencode");
        let stage = root.join("stage");
        fs::create_dir_all(dest.join("plugins/local")).unwrap();
        fs::create_dir_all(stage.join("plugins/local")).unwrap();
        fs::write(dest.join("plugins/local/index.js"), "old").unwrap();
        fs::write(stage.join("plugins/local/index.js"), "new").unwrap();
        let backup = root.join("backup");
        replace_config(&dest, &stage, Some(&backup)).unwrap();
        assert_eq!(
            fs::read_to_string(dest.join("plugins/local/index.js")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(backup.join("plugins/local/index.js")).unwrap(),
            "old"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_does_not_depend_on_old_git_remote_or_local_edits() {
        let root = sandbox();
        let dest = root.join("opencode");
        let stage = root.join("stage");
        let backup = root.join("backup");
        fs::create_dir_all(dest.join(".git")).unwrap();
        fs::write(
            dest.join(".git/config"),
            "[remote \"origin\"]\nurl = unfamiliar\n",
        )
        .unwrap();
        fs::write(dest.join("local-edit"), "keep").unwrap();
        fs::create_dir(&stage).unwrap();
        fs::write(stage.join("new"), "latest").unwrap();
        replace_config(&dest, &stage, Some(&backup)).unwrap();
        assert_eq!(fs::read_to_string(dest.join("new")).unwrap(), "latest");
        assert_eq!(
            fs::read_to_string(backup.join("local-edit")).unwrap(),
            "keep"
        );
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
