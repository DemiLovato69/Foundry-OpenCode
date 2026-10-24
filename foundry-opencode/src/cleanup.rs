use color_eyre::eyre::{self, Context};
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Default, PartialEq, Eq)]
pub struct Inventory {
    pub brew_v1: Vec<String>,
    pub brew_v2: bool,
    pub npm_v1: Vec<String>,
    pub files: Vec<PathBuf>,
    pub chamber: bool,
    pub unknown: Vec<PathBuf>,
}

fn output(command: &mut Command) -> eyre::Result<String> {
    let result = command.output()?;
    eyre::ensure!(
        result.status.success(),
        "Could not inspect installed packages: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(String::from_utf8(result.stdout)?)
}

fn brew_version(name: &str) -> eyre::Result<Option<String>> {
    let result = Command::new("brew")
        .args(["list", "--formula", "--versions", name])
        .output()?;
    if !result.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8(result.stdout)?
        .split_whitespace()
        .nth(1)
        .map(str::to_string))
}

pub fn latest_is_v2() -> eyre::Result<String> {
    let json = output(Command::new("brew").args(["info", "--json=v2", "--formula", "opencode"]))?;
    parse_target_version(&json)
}

fn parse_target_version(json: &str) -> eyre::Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(json).wrap_err("Invalid Homebrew formula metadata")?;
    let version = value["formulae"][0]["versions"]["stable"]
        .as_str()
        .ok_or_else(|| eyre::eyre!("Homebrew did not provide a stable OpenCode version"))?;
    eyre::ensure!(
        version.starts_with("2."),
        "Standard Homebrew opencode targets {version}, not V2; no installation was removed"
    );
    Ok(version.to_string())
}

fn major(version: &str) -> Option<u64> {
    version
        .split_whitespace()
        .find_map(|word| word.trim_start_matches('v').split('.').next()?.parse().ok())
}

pub fn reports_v2(version: &str) -> bool {
    major(version) == Some(2)
}

fn brew_owned(target: &Path, prefix: &Path) -> bool {
    target.starts_with(prefix.join("Cellar")) || target.starts_with(prefix.join("Caskroom"))
}

fn known_unmanaged_path(path: &Path, home: &Path) -> bool {
    [
        home.join(".opencode/bin/opencode"),
        home.join(".local/bin/opencode"),
        home.join("bin/opencode"),
        PathBuf::from("/usr/local/bin/opencode"),
    ]
    .iter()
    .any(|known| path == known)
}

fn npm_packages() -> eyre::Result<(Option<PathBuf>, Vec<String>)> {
    let root = match Command::new("npm").args(["root", "-g"]).output() {
        Ok(result) if result.status.success() => {
            Some(PathBuf::from(String::from_utf8(result.stdout)?.trim()))
        }
        _ => return Ok((None, Vec::new())),
    };
    let json = output(Command::new("npm").args(["ls", "-g", "--depth=0", "--json"]))?;
    let value: serde_json::Value = serde_json::from_str(&json)?;
    let packages = ["opencode", "opencode-ai"]
        .into_iter()
        .filter(|name| {
            value["dependencies"][name]["version"]
                .as_str()
                .is_some_and(|version| major(version) == Some(1))
        })
        .map(str::to_string)
        .collect();
    Ok((root, packages))
}

pub fn inspect(home: &Path, with_chamber: bool) -> eyre::Result<Inventory> {
    latest_is_v2()?;
    let mut inventory = Inventory::default();
    for name in ["opencode-v1", "opencode-v2"] {
        if let Some(version) = brew_version(name)? {
            if name == "opencode-v2" {
                eyre::ensure!(
                    major(&version) == Some(2),
                    "Homebrew opencode-v2 reported unexpected version {version}"
                );
                inventory.brew_v2 = true;
            } else if major(&version) == Some(1) {
                inventory.brew_v1.push(name.to_string());
            }
        }
    }
    if let Some(version) = brew_version("opencode")?
        && major(&version) == Some(1)
    {
        inventory.brew_v1.push("opencode".into());
    }
    let brew_prefix = PathBuf::from(
        output(Command::new("brew").arg("--prefix"))?
            .trim()
            .to_string(),
    );
    let (npm_root, npm_v1) = npm_packages()?;
    inventory.npm_v1 = npm_v1;
    let mut paths = HashSet::new();
    paths.extend(
        env::split_paths(&env::var_os("PATH").unwrap_or_default()).map(|dir| dir.join("opencode")),
    );
    paths.extend([
        home.join(".opencode/bin/opencode"),
        home.join(".local/bin/opencode"),
        home.join("bin/opencode"),
    ]);
    for path in paths {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            continue;
        }
        let Ok(target) = path.canonicalize() else {
            inventory.unknown.push(path);
            continue;
        };
        if brew_owned(&target, &brew_prefix) {
            continue;
        }
        if let Some(root) = &npm_root
            && target.starts_with(root)
        {
            if inventory
                .npm_v1
                .iter()
                .any(|package| target.starts_with(root.join(package)))
            {
                continue;
            }
            inventory.unknown.push(path);
            continue;
        }
        if target
            .components()
            .any(|component| component.as_os_str() == "node_modules")
        {
            inventory.unknown.push(path);
            continue;
        }
        if metadata.file_type().is_symlink() || !known_unmanaged_path(&path, home) {
            inventory.unknown.push(path);
            continue;
        }
        let result = Command::new(&path).arg("--version").output();
        if matches!(result, Ok(ref result) if result.status.success()
            && matches!(major(&String::from_utf8_lossy(&result.stdout)), Some(1 | 2)))
        {
            inventory.files.push(path);
        } else {
            inventory.unknown.push(path);
        }
    }
    inventory.files.sort();
    inventory.unknown.sort();
    inventory.brew_v1.sort();
    if with_chamber {
        let cask = Command::new("brew")
            .args(["list", "--cask", "--versions", "openchamber"])
            .output()?;
        if !cask.status.success() {
            let path = PathBuf::from("/Applications/OpenChamber.app");
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                    inventory.chamber = true
                }
                Ok(_) => inventory.unknown.push(path),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(inventory)
}

pub fn move_to_trash(path: &Path, home: &Path) -> eyre::Result<PathBuf> {
    let trash = home.join(".Trash");
    if !trash.exists() {
        fs::create_dir(&trash)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&trash, fs::Permissions::from_mode(0o700))?;
        }
    }
    eyre::ensure!(
        fs::symlink_metadata(&trash)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink()),
        "Trash directory {} does not exist; remove {} manually",
        trash.display(),
        path.display()
    );
    let name = path
        .file_name()
        .ok_or_else(|| eyre::eyre!("Invalid installation path"))?;
    let destination = crate::ops::backup_path(&trash.join(name));
    fs::rename(path, &destination).wrap_err_with(|| {
        format!(
            "Could not move {} to Trash; remove it manually before installing",
            path.display()
        )
    })?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_parser_recognizes_major_versions() {
        assert_eq!(major("opencode v1.9.0\n"), Some(1));
        assert_eq!(major("2.1.3\n"), Some(2));
        assert_eq!(major("unknown"), None);
        assert!(reports_v2("opencode v2.1.3"));
        assert_eq!(
            parse_target_version(r#"{"formulae":[{"versions":{"stable":"2.1.3"}}]}"#).unwrap(),
            "2.1.3"
        );
        assert!(parse_target_version(r#"{"formulae":[{"versions":{"stable":"1.9.0"}}]}"#).is_err());
        assert!(brew_owned(
            Path::new("/opt/homebrew/Cellar/opencode-v2/2.1.3/bin/opencode"),
            Path::new("/opt/homebrew")
        ));
        assert!(!known_unmanaged_path(
            Path::new("/opt/homebrew/bin/opencode"),
            Path::new("/Users/test")
        ));
        assert!(known_unmanaged_path(
            Path::new("/Users/test/.local/bin/opencode"),
            Path::new("/Users/test")
        ));
    }

    #[test]
    fn cleanup_moves_only_the_requested_executable_to_trash() {
        let home = crate::ops::backup_path(&std::env::temp_dir().join("foundry-cleanup"));
        fs::create_dir(&home).unwrap();
        let binary = home.join("opencode");
        fs::write(&binary, "v1").unwrap();
        let data = home.join("sessions");
        fs::write(&data, "preserve").unwrap();
        let moved = move_to_trash(&binary, &home).unwrap();
        assert!(!binary.exists());
        assert_eq!(fs::read_to_string(moved).unwrap(), "v1");
        assert_eq!(fs::read_to_string(data).unwrap(), "preserve");
        fs::remove_dir_all(home).unwrap();
    }
}
