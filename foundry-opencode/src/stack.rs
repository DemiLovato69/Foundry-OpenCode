use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{self, Context};
use serde_json::{Value, json};

use crate::ops;

pub const TOKEN: &str = ".opencode/token";
const CONFIG: &str = ".opencode/opencode.json";

pub fn config(url: &str) -> Value {
    let token = "{file:./token}";
    json!({
        "$schema": "https://opencode.ai/config.json",
        "providers": {
            "palantir-openai": {
                "settings": {"baseURL": format!("{url}/language-model-service/api/proxy/openai/v1"), "apiKey": token},
                "headers": {"Authorization": format!("Bearer {token}")}
            },
            "palantir-anthropic": {
                "settings": {"baseURL": format!("{url}/api/v2/llm/proxy/anthropic/v1"), "apiKey": token},
                "headers": {"Authorization": format!("Bearer {token}")}
            }
        },
        "mcp": {"servers": {"palantir": {
            "type": "local",
            "command": ["npx", "-y", "palantir-mcp", "--foundry-api-url", url],
            "environment": {"FOUNDRY_TOKEN": token}
        }}}
    })
}

fn metadata(path: &Path) -> eyre::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            eyre::ensure!(
                !meta.file_type().is_symlink(),
                "{} is a symlink; not changing it",
                path.display()
            );
            Ok(Some(meta))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn read_config(path: &Path) -> eyre::Result<Value> {
    eyre::ensure!(
        metadata(path)?.is_some_and(|m| m.is_file()),
        "{} is not a regular config file",
        path.display()
    );
    jsonc_parser::parse_to_serde_value(&fs::read_to_string(path)?, &Default::default())
        .wrap_err_with(|| format!("Invalid OpenCode config at {}", path.display()))
}

fn ignored_token(stack: &Path) -> eyre::Result<Option<PathBuf>> {
    let repo = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(stack)
        .output()?;
    if !repo.status.success() {
        return Ok(None);
    }
    let root = PathBuf::from(String::from_utf8(repo.stdout)?.trim());
    let token = stack.join(TOKEN);
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(&token)
        .current_dir(stack)
        .output()?;
    eyre::ensure!(
        !tracked.status.success(),
        "{} is tracked by Git; untrack it before continuing",
        token.display()
    );
    let ignored = Command::new("git")
        .args(["check-ignore", "--quiet", "--no-index", "--"])
        .arg(&token)
        .current_dir(stack)
        .status()?;
    Ok((!ignored.success()).then(|| root.join(".gitignore")))
}

fn add_ignore(path: &Path, stack: &Path) -> eyre::Result<()> {
    let root = path.parent().unwrap();
    let relative = stack
        .canonicalize()?
        .join(TOKEN)
        .strip_prefix(root.canonicalize()?)?
        .to_string_lossy()
        .to_string();
    eyre::ensure!(
        metadata(path)?.is_none_or(|m| m.is_file()),
        "{} is not a regular file",
        path.display()
    );
    let existing = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.into()),
    };
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if !existing.is_empty() && !existing.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file, "/{relative}")?;
    Ok(())
}

pub fn inspect(stack: &Path, url: &str, key: &str) -> eyre::Result<bool> {
    eyre::ensure!(
        ops::valid_url(url) && !key.is_empty() && !key.chars().any(char::is_control),
        "Invalid Foundry host or token"
    );
    let directory = stack.join(".opencode");
    eyre::ensure!(
        metadata(&directory)?.is_none_or(|m| m.is_dir()),
        "{} is not a directory",
        directory.display()
    );
    eyre::ensure!(
        !stack.join(".opencode/opencode.jsonc").exists(),
        "{} already exists; review it manually",
        stack.join(".opencode/opencode.jsonc").display()
    );
    let config_path = stack.join(CONFIG);
    let token_path = stack.join(TOKEN);
    let config_exists = metadata(&config_path)?.is_some();
    let token_exists = metadata(&token_path)?.is_some();
    eyre::ensure!(
        config_exists == token_exists,
        "Partial stack configuration in {}; review it manually",
        stack.display()
    );
    if config_exists {
        let current = read_config(&config_path)?;
        let expected = config(url);
        for provider in ["palantir-openai", "palantir-anthropic"] {
            eyre::ensure!(
                current["providers"][provider] == expected["providers"][provider],
                "{} has different provider settings; not overwriting it",
                config_path.display()
            );
        }
        eyre::ensure!(
            current["mcp"]["servers"]["palantir"] == expected["mcp"]["servers"]["palantir"],
            "{} has different Foundry MCP settings; not overwriting it",
            config_path.display()
        );
        let token_meta = metadata(&token_path)?;
        eyre::ensure!(
            token_meta.as_ref().is_some_and(|m| m.is_file()),
            "{} is not a regular file",
            token_path.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            eyre::ensure!(
                token_meta.unwrap().permissions().mode() & 0o077 == 0,
                "{} is not private (mode 0600 required)",
                token_path.display()
            );
        }
        eyre::ensure!(
            ignored_token(stack)?.is_none(),
            "{} is not Git-ignored",
            token_path.display()
        );
        eyre::ensure!(
            fs::read_to_string(&token_path)? == key,
            "{} has a different token; not overwriting it",
            token_path.display()
        );
    }
    Ok(config_exists)
}

pub fn create(stack: &Path, url: &str, key: &str) -> eyre::Result<()> {
    if inspect(stack, url, key)? {
        return Ok(());
    }
    let ignore = ignored_token(stack)?;
    fs::create_dir_all(stack.join(".opencode"))?;
    if let Some(path) = ignore {
        add_ignore(&path, stack)?;
    }
    eyre::ensure!(
        ignored_token(stack)?.is_none(),
        "Could not Git-ignore {}",
        stack.join(TOKEN).display()
    );
    let token_path = stack.join(TOKEN);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&token_path)?;
    if let Err(e) = file.write_all(key.as_bytes()).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&token_path);
        return Err(e.into());
    }
    let result = (|| -> eyre::Result<()> {
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stack.join(CONFIG))?;
        writeln!(out, "{}", serde_json::to_string_pretty(&config(url))?)?;
        out.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(stack.join(CONFIG));
        let _ = fs::remove_file(token_path);
    }
    result
}

fn legacy_value(value: &str) -> Option<String> {
    if value.starts_with('\'') && value.ends_with('\'') {
        let unescaped = value[1..value.len() - 1].replace("'\\''", "'");
        (format!("'{}'", unescaped.replace('\'', "'\\''")) == value).then_some(unescaped)
    } else if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._:/".contains(&b))
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn legacy(stack: &Path) -> eyre::Result<Option<(String, String, String)>> {
    let path = stack.join(".envrc");
    let Some(meta) = metadata(&path)? else {
        return Ok(None);
    };
    eyre::ensure!(meta.is_file(), "{} is not a regular file", path.display());
    let mut url = None;
    let mut key = None;
    let mut restart = false;
    let original = fs::read_to_string(&path)?;
    for line in original.lines().map(str::trim) {
        if let Some(value) = line.strip_prefix("export OPENCODE_BASE_URL=") {
            if url
                .replace(
                    legacy_value(value)
                        .ok_or_else(|| eyre::eyre!("Unsupported host in {}", path.display()))?,
                )
                .is_some()
            {
                eyre::bail!("Duplicate host in {}", path.display())
            }
        } else if let Some(value) = line.strip_prefix("export OPENCODE_API_KEY=") {
            if key
                .replace(
                    legacy_value(value)
                        .ok_or_else(|| eyre::eyre!("Unsupported token in {}", path.display()))?,
                )
                .is_some()
            {
                eyre::bail!("Duplicate token in {}", path.display())
            }
        } else if line == "opencode service restart" && !restart {
            restart = true;
        } else {
            eyre::bail!(
                "{} contains custom shell commands; not migrating it",
                path.display()
            );
        }
    }
    let (Some(url), Some(key)) = (url, key) else {
        eyre::bail!("{} is missing Foundry credentials", path.display())
    };
    eyre::ensure!(
        ops::valid_url(&url) && !key.is_empty() && !key.chars().any(char::is_control),
        "Invalid credentials in {}",
        path.display()
    );
    Ok(Some((url, key, original)))
}

pub fn migrate(stack: &Path, dry_run: bool) -> eyre::Result<String> {
    let Some((url, key, original)) = legacy(stack)? else {
        return Ok(format!("No legacy .envrc in {}", stack.display()));
    };
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--", ".envrc"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(stack)
        .output()?;
    eyre::ensure!(
        !tracked.status.success(),
        "{} is tracked by Git; untrack it and rotate its token before migrating",
        stack.join(".envrc").display()
    );
    inspect(stack, &url, &key)?;
    if !dry_run {
        create(stack, &url, &key)?;
        // Keep the original intact until the new credential and config are verified.
        inspect(stack, &url, &key)?;
        eyre::ensure!(
            original == fs::read_to_string(stack.join(".envrc"))?,
            "{} changed during migration; not removing it",
            stack.join(".envrc").display()
        );
        fs::remove_file(stack.join(".envrc"))?;
    }
    Ok(format!(
        "{} {} to .opencode config",
        if dry_run { "Would migrate" } else { "Migrated" },
        stack.display()
    ))
}

pub fn credentials(project: &Path) -> eyre::Result<(String, String)> {
    for directory in project.ancestors() {
        let config_path = directory.join(CONFIG);
        if !config_path.exists() {
            continue;
        }
        let doc = read_config(&config_path)?;
        if doc["providers"]["palantir-openai"]["settings"]["baseURL"].is_null() {
            continue;
        }
        let base = doc["providers"]["palantir-openai"]["settings"]["baseURL"]
            .as_str()
            .and_then(|url| url.strip_suffix("/language-model-service/api/proxy/openai/v1"))
            .ok_or_else(|| eyre::eyre!("Missing stack host in {}", config_path.display()))?;
        eyre::ensure!(
            ops::valid_url(base),
            "Invalid stack host in {}",
            config_path.display()
        );
        eyre::ensure!(
            doc["providers"]["palantir-openai"]["settings"]["apiKey"] == "{file:./token}",
            "Unsupported stack token reference in {}",
            config_path.display()
        );
        let token_path = directory.join(TOKEN);
        eyre::ensure!(
            metadata(&token_path)?.is_some_and(|m| m.is_file()),
            "Missing private stack token at {}",
            token_path.display()
        );
        let key = fs::read_to_string(&token_path)?;
        eyre::ensure!(
            !key.is_empty() && !key.chars().any(char::is_control),
            "Invalid stack token"
        );
        return Ok((base.to_owned(), key));
    }
    eyre::bail!("Run foundry-opencode init from the stack directory first")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_safe_and_repeatable() {
        let root = ops::backup_path(&std::env::temp_dir().join("stack-migrate-test"));
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".envrc"), "export OPENCODE_API_KEY='a'\\''b'\nexport OPENCODE_BASE_URL='https://example.com'\nopencode service restart\n").unwrap();
        assert!(migrate(&root, true).unwrap().starts_with("Would migrate"));
        assert!(!root.join(TOKEN).exists());
        migrate(&root, false).unwrap();
        assert!(!root.join(".envrc").exists());
        assert_eq!(
            credentials(&root).unwrap(),
            ("https://example.com".into(), "a'b".into())
        );
        let doc = read_config(&root.join(CONFIG)).unwrap();
        assert_eq!(
            doc["providers"]["palantir-openai"]["settings"]["apiKey"],
            "{file:./token}"
        );
        assert_eq!(
            doc["providers"]["palantir-anthropic"]["headers"]["Authorization"],
            "Bearer {file:./token}"
        );
        assert_eq!(
            doc["mcp"]["servers"]["palantir"]["environment"]["FOUNDRY_TOKEN"],
            "{file:./token}"
        );
        assert_eq!(doc["mcp"]["servers"]["palantir"]["command"][0], "npx");
        assert!(migrate(&root, false).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(root.join(TOKEN)).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_shell_and_conflicting_config_are_preserved() {
        let root = ops::backup_path(&std::env::temp_dir().join("stack-migrate-test"));
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".envrc"), "export OPENCODE_API_KEY='key'\nexport OPENCODE_BASE_URL='https://example.com'\necho custom\n").unwrap();
        assert!(migrate(&root, false).is_err());
        assert!(!root.join(TOKEN).exists());
        fs::write(
            root.join(".envrc"),
            "export OPENCODE_API_KEY='key'\nexport OPENCODE_BASE_URL='https://example.com'\n",
        )
        .unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join(CONFIG), "{\"model\":\"other\"}").unwrap();
        assert!(migrate(&root, false).is_err());
        assert!(root.join(".envrc").exists());
        assert_eq!(
            fs::read_to_string(root.join(CONFIG)).unwrap(),
            "{\"model\":\"other\"}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_ignores_private_token_and_child_projects_inherit_credentials() {
        let root = ops::backup_path(&std::env::temp_dir().join("stack-git-test"));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        create(&root, "https://example.com", "private").unwrap();
        assert!(
            Command::new("git")
                .args(["check-ignore", "--quiet", "--", ".opencode/token"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        let project = root.join("repo");
        fs::create_dir(&project).unwrap();
        fs::create_dir(project.join(".opencode")).unwrap();
        fs::write(project.join(CONFIG), r#"{"mcp":{"servers":{"rag":{"type":"local","command":["node",".opencode/rag-cli.mjs","mcp"]}}}}"#).unwrap();
        assert_eq!(credentials(&project).unwrap().1, "private");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_preserves_other_existing_stack_settings() {
        let root = ops::backup_path(&std::env::temp_dir().join("stack-rag-test"));
        fs::create_dir(&root).unwrap();
        create(&root, "https://example.com", "private").unwrap();
        let mut doc = read_config(&root.join(CONFIG)).unwrap();
        doc["mcp"]["servers"]["rag"] = json!({"type":"local","command":["node","rag-cli.mjs"]});
        fs::write(root.join(CONFIG), serde_json::to_string(&doc).unwrap()).unwrap();
        fs::write(root.join(".envrc"), "export OPENCODE_API_KEY='private'\nexport OPENCODE_BASE_URL='https://example.com'\nopencode service restart\n").unwrap();
        migrate(&root, false).unwrap();
        assert_eq!(
            read_config(&root.join(CONFIG)).unwrap()["mcp"]["servers"]["rag"],
            doc["mcp"]["servers"]["rag"]
        );
        assert!(!root.join(".envrc").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
