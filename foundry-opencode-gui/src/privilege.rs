#[cfg(target_os = "macos")]
use std::io::Read;
#[cfg(target_os = "macos")]
use std::path::Path;

#[cfg(target_os = "macos")]
use security_framework::authorization::{Authorization, AuthorizationItemSetBuilder, Flags};

#[cfg(target_os = "macos")]
pub struct Privilege(Authorization);

#[cfg(target_os = "macos")]
impl Privilege {
    pub fn authorize() -> Result<Self, String> {
        let rights = AuthorizationItemSetBuilder::new()
            .add_right("system.privilege.admin")
            .map_err(|error| error.to_string())?
            .build();
        let auth = Authorization::new(
            Some(rights),
            None,
            Flags::EXTEND_RIGHTS | Flags::INTERACTION_ALLOWED | Flags::PREAUTHORIZE,
        )
        .map_err(|error| format!("Administrator authorization was not granted: {error}"))?;
        Ok(Self(auth))
    }

    fn run(&self, executable: &str, args: &[&Path]) -> Result<(), String> {
        let mut pipe = self
            .0
            .execute_with_privileges_piped(executable, args, Flags::DEFAULTS)
            .map_err(|error| format!("Privileged operation could not start: {error}"))?;
        let mut result = Vec::new();
        pipe.read_to_end(&mut result)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn install_cli(&self, source: &Path) -> Result<(), String> {
        replace_cli(
            source,
            Path::new("/usr/local/bin/foundry-opencode"),
            |executable, args| self.run(executable, args),
        )
    }
}

#[cfg(target_os = "macos")]
fn replace_cli(
    source: &Path,
    destination: &Path,
    mut run: impl FnMut(&str, &[&Path]) -> Result<(), String>,
) -> Result<(), String> {
    let parent = destination.parent().unwrap();
    for directory in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match std::fs::symlink_metadata(directory) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                run(
                    "/usr/bin/install",
                    &[
                        Path::new("-d"),
                        Path::new("-m"),
                        Path::new("0755"),
                        directory,
                    ],
                )?;
                let meta =
                    std::fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    return Err("Unsafe CLI installation directory".into());
                }
            }
            _ => return Err("Unsafe CLI installation directory".into()),
        }
    }
    match std::fs::symlink_metadata(destination) {
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
            return Err("Unsafe existing CLI".into());
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error.to_string());
        }
        _ => {}
    }
    let staging = foundry_opencode::ops::backup_path(destination);
    let source = foundry_opencode::ops::stage_binary(source).map_err(|error| error.to_string())?;
    let mode = Path::new("0755");
    run(
        "/usr/bin/install",
        &[Path::new("-m"), mode, source.path(), &staging],
    )?;
    if std::fs::read(&staging).map_err(|error| error.to_string())?
        != std::fs::read(source.path()).map_err(|error| error.to_string())?
    {
        return Err("CLI staging verification failed".into());
    }
    let version = std::process::Command::new(source.path())
        .arg("version")
        .output()
        .map_err(|error| error.to_string())?;
    if !version.status.success() {
        return Err("Invalid CLI payload".into());
    }
    let backup = foundry_opencode::ops::backup_path(destination);
    let old = if destination.exists() {
        Some(std::fs::read(destination).map_err(|error| error.to_string())?)
    } else {
        None
    };
    if let Some(old) = &old {
        run("/bin/cp", &[Path::new("-p"), destination, &backup])?;
        if std::fs::read(&backup).map_err(|error| error.to_string())? != *old {
            return Err("Existing CLI was not backed up; replacement aborted".into());
        }
    }
    let moved = run("/bin/mv", &[Path::new("-f"), &staging, destination]);
    if moved.is_ok()
        && std::process::Command::new(destination)
            .arg("version")
            .output()
            .is_ok_and(|installed| installed.status.success() && installed.stdout == version.stdout)
        && std::fs::read(destination).is_ok_and(|installed| {
            std::fs::read(source.path()).is_ok_and(|source| installed == source)
        })
    {
        return Ok(());
    }
    if let Some(old) = old {
        run("/bin/mv", &[Path::new("-f"), &backup, destination])?;
        if std::fs::read(destination).map_err(|error| error.to_string())? != old {
            return Err(format!(
                "CLI restore could not be verified; inspect {}",
                backup.display()
            ));
        }
    } else if destination.exists() {
        run("/bin/rm", &[destination])?;
        if destination.exists() {
            return Err(format!(
                "CLI restore could not be verified; inspect {}",
                destination.display()
            ));
        }
    }
    Err("CLI replacement failed; previous executable restored if present".into())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn execute(executable: &str, args: &[&Path]) -> Result<(), String> {
        let status = std::process::Command::new(executable)
            .args(args)
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{executable} failed"))
        }
    }

    #[test]
    fn cli_copy_verifies_payload_and_restores_old_binary_after_failure() {
        let folder = foundry_opencode::ops::backup_path(&std::env::temp_dir().join("gui-cli-copy"));
        fs::create_dir(&folder).unwrap();
        let folder = folder.canonicalize().unwrap();
        let old = folder.join("foundry-opencode");
        let new = folder.join("new-cli");
        fs::write(&old, "#!/bin/sh\necho old\n").unwrap();
        fs::write(&new, "#!/bin/sh\necho new\n").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755)).unwrap();
        let mut failed = false;
        let result = replace_cli(&new, &old, |executable, args| {
            execute(executable, args)?;
            if executable == "/bin/mv" && !failed {
                failed = true;
                return Err("simulated interrupted replacement".into());
            }
            Ok(())
        });
        assert!(result.is_err() && failed);
        assert_eq!(
            std::process::Command::new(&old)
                .arg("version")
                .output()
                .unwrap()
                .stdout,
            b"old\n"
        );
        replace_cli(&new, &old, execute).unwrap();
        assert_eq!(
            std::process::Command::new(&old)
                .arg("version")
                .output()
                .unwrap()
                .stdout,
            b"new\n"
        );
        let missing = folder.join("new-bin/foundry-opencode");
        replace_cli(&new, &missing, execute).unwrap();
        assert_eq!(
            std::process::Command::new(&missing)
                .arg("version")
                .output()
                .unwrap()
                .stdout,
            b"new\n"
        );
        let absent = folder.join("new-bin/interrupted-cli");
        let failure = replace_cli(&new, &absent, |executable, args| {
            execute(executable, args)?;
            if executable == "/bin/mv" {
                return Err("interrupted".into());
            }
            Ok(())
        });
        assert!(failure.is_err() && !absent.exists());
        fs::remove_dir_all(folder).unwrap();
    }
}

#[cfg(not(target_os = "macos"))]
pub struct Privilege;
#[cfg(not(target_os = "macos"))]
impl Privilege {
    pub fn authorize() -> Result<Self, String> {
        Err("Installation requires macOS".into())
    }
    pub fn install_cli(&self, _: &std::path::Path) -> Result<(), String> {
        Err("Installation requires macOS".into())
    }
}
