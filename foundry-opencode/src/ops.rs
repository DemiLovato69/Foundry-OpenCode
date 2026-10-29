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
const RELEASES: &str = "https://api.github.com/repos/DemiLovato69/Foundry-OpenCode/releases/latest";

pub struct StagedBinary(PathBuf);

impl Drop for StagedBinary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

impl StagedBinary {
    pub fn path(&self) -> &Path {
        &self.0
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

fn release_asset() -> eyre::Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("aarch64-apple-darwin"),
        "x86_64" => Ok("x86_64-apple-darwin"),
        arch => eyre::bail!("No macOS release binary for {arch}"),
    }
}

fn curl(url: &str, destination: Option<&Path>) -> eyre::Result<Vec<u8>> {
    let mut command = Command::new("curl");
    command.args([
        "--fail",
        "--location",
        "--silent",
        "--show-error",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--retry",
        "2",
        "--max-time",
        "120",
    ]);
    if let Some(path) = destination {
        command.arg("--output").arg(path);
    }
    let output = command.arg(url).output()?;
    eyre::ensure!(
        output.status.success(),
        "Could not download {url}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output.stdout)
}

fn release_version(value: &str) -> eyre::Result<&str> {
    let version = value
        .strip_prefix('v')
        .ok_or_else(|| eyre::eyre!("Unexpected release tag"))?;
    eyre::ensure!(
        version.split('.').count() == 3
            && version
                .split('.')
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        "Unexpected release tag"
    );
    Ok(version)
}

fn checksum(contents: &str, asset: &str) -> eyre::Result<String> {
    let matches: Vec<_> = contents
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once("  ")?;
            (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .then_some(hash.to_ascii_lowercase())
        })
        .collect();
    eyre::ensure!(
        matches.len() == 1,
        "Release checksum is missing or ambiguous for {asset}"
    );
    Ok(matches[0].clone())
}

struct ReleaseDirectory(PathBuf);

impl Drop for ReleaseDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn stage_latest_binary() -> eyre::Result<(StagedBinary, String)> {
    eyre::ensure!(
        cfg!(target_os = "macos"),
        "Release binaries are for macOS only"
    );
    let release: serde_json::Value = serde_json::from_slice(&curl(RELEASES, None)?)?;
    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("Latest release has no tag"))?;
    let version = release_version(tag)?;
    let asset = format!("foundry-opencode-{}.tar.gz", release_asset()?);
    let base = format!("https://github.com/DemiLovato69/Foundry-OpenCode/releases/download/{tag}");
    let directory = ReleaseDirectory(backup_path(
        &std::env::temp_dir().join("foundry-opencode-release"),
    ));
    fs::create_dir(&directory.0)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700))?;
    }
    let archive = directory.0.join(&asset);
    curl(&format!("{base}/{asset}"), Some(&archive))?;
    let sums = String::from_utf8(curl(&format!("{base}/SHA256SUMS"), None)?)?;
    let expected = checksum(&sums, &asset)?;
    let hash = Command::new("shasum")
        .args(["-a", "256"])
        .arg(&archive)
        .output()?;
    eyre::ensure!(
        hash.status.success()
            && String::from_utf8_lossy(&hash.stdout)
                .split_whitespace()
                .next()
                == Some(expected.as_str()),
        "Release archive checksum mismatch; installed binary was not changed"
    );
    // Stream only the named member; never extract archive paths into the filesystem.
    let extracted = directory.0.join("foundry-opencode");
    let output = Command::new("tar")
        .arg("-xOf")
        .arg(&archive)
        .arg("foundry-opencode")
        .output()?;
    eyre::ensure!(
        output.status.success() && !output.stdout.is_empty(),
        "Invalid release archive"
    );
    fs::write(&extracted, output.stdout)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&extracted, fs::Permissions::from_mode(0o700))?;
    }
    let archs = Command::new("/usr/bin/lipo")
        .arg("-archs")
        .arg(&extracted)
        .output()?;
    let arch = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        "x86_64"
    };
    eyre::ensure!(
        archs.status.success()
            && String::from_utf8_lossy(&archs.stdout)
                .split_whitespace()
                .any(|a| a == arch),
        "Release binary has the wrong architecture"
    );
    let actual = Command::new(&extracted).arg("version").output()?;
    eyre::ensure!(
        actual.status.success() && String::from_utf8_lossy(&actual.stdout).trim() == version,
        "Release binary version does not match its tag"
    );
    Ok((stage_binary(&extracted)?, version.to_owned()))
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
        if destination.exists() {
            fs::hard_link(destination, backup_path(destination))?;
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
    let script = "set -e; d=$(dirname \"$2\"); if [ -L \"$d\" ]; then exit 1; fi; if [ ! -d \"$d\" ]; then /usr/bin/install -d -m 0755 \"$d\"; fi; if [ -L \"$2\" ] || { [ -e \"$2\" ] && [ ! -f \"$2\" ]; }; then exit 1; fi; t=$(mktemp \"$2.tmp.XXXXXXXX\"); trap 'rm -f \"$t\"' EXIT; /usr/bin/install -m 0755 \"$1\" \"$t\"; if [ -f \"$2\" ]; then b=$(mktemp \"$2.backup.XXXXXXXX\"); if ! /bin/cp -p \"$2\" \"$b\"; then /bin/rm -f \"$b\"; exit 1; fi; fi; /bin/mv -f \"$t\" \"$2\"";
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
    fn release_metadata_is_strict() {
        assert_eq!(release_version("v0.1.42").unwrap(), "0.1.42");
        for tag in ["main", "v0.1.1/../../x", "v0.1.1-rc1"] {
            assert!(release_version(tag).is_err());
        }
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(
                &format!("{hash}  foundry-opencode-aarch64-apple-darwin.tar.gz\n"),
                "foundry-opencode-aarch64-apple-darwin.tar.gz"
            )
            .unwrap(),
            hash
        );
        assert!(
            checksum(
                &format!("{hash}  other.tar.gz\n"),
                "foundry-opencode-aarch64-apple-darwin.tar.gz"
            )
            .is_err()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires GitHub network access; stages but does not install the release"]
    fn published_release_can_be_staged_safely() {
        let (_binary, version) = stage_latest_binary().unwrap();
        assert_eq!(release_version(&format!("v{version}")).unwrap(), version);
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
        let global = crate::stack::read_config(&config.join("opencode.jsonc")).unwrap();
        assert_eq!(
            global["mcp"]["servers"]["palantir"]["command"],
            serde_json::json!([
                "npx",
                "-y",
                "--registry=https://registry.npmjs.org",
                "palantir-mcp",
                "--foundry-api-url",
                "https://example.com"
            ])
        );
        assert_eq!(
            global["mcp"]["servers"]["palantir"]["environment"]["FOUNDRY_TOKEN"],
            "{file:./token}"
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

    #[cfg(target_os = "macos")]
    #[test]
    fn plugin_remains_readable_during_config_swap() {
        let root = sandbox();
        let dest = root.join("opencode");
        let plugin = dest.join("plugins/local/index.js");
        let stage = root.join("stage");
        fs::create_dir_all(plugin.parent().unwrap()).unwrap();
        fs::create_dir_all(stage.join("plugins/local")).unwrap();
        fs::write(&plugin, "old").unwrap();
        fs::write(stage.join("plugins/local/index.js"), "new").unwrap();
        let reader = std::thread::spawn(move || {
            for _ in 0..1000 {
                assert!(matches!(
                    fs::read_to_string(&plugin).as_deref(),
                    Ok("old" | "new")
                ));
            }
        });
        replace_config(&dest, &stage, Some(&root.join("backup"))).unwrap();
        reader.join().unwrap();
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
        assert!(fs::read_dir(&root).unwrap().any(|entry| {
            let entry = entry.unwrap();
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("foundry-opencode.backup-")
                && fs::read(entry.path()).unwrap() == b"outdated"
        }));
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
