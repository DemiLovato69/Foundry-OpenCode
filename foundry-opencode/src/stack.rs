use std::fs;
use std::io;
use std::path::{Path, PathBuf};

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
            },
            "palantir-google": {
                "settings": {"baseURL": format!("{url}/api/v2/llm/proxy/google/v1"), "apiKey": token, "transport": "http", "encoding": "none"},
                "headers": {"Authorization": format!("Bearer {token}")}
            }
        },
        "mcp": {"servers": {"palantir": {
            "type": "local",
            "command": ["npx", "-y", "--registry=https://registry.npmjs.org", "palantir-mcp", "--foundry-api-url", url],
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

pub fn legacy_credentials(stack: &Path) -> eyre::Result<Option<(String, String)>> {
    Ok(legacy(stack)?.map(|(url, key, _)| (url, key)))
}

fn token_path(reference: &str, home: &Path, directory: &Path) -> eyre::Result<PathBuf> {
    if reference == "{file:./token}" {
        return Ok(directory.join(TOKEN));
    }
    let name = reference
        .strip_prefix("{file:")
        .and_then(|part| part.strip_suffix('}'))
        .ok_or_else(|| eyre::eyre!("Unsupported token reference"))?;
    let token = PathBuf::from(name);
    eyre::ensure!(
        token.parent() == crate::projects::token_file(home, "default").parent(),
        "Unsupported token location"
    );
    Ok(token)
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
        let reference = doc["providers"]["palantir-openai"]["settings"]["apiKey"]
            .as_str()
            .ok_or_else(|| eyre::eyre!("Missing token reference in {}", config_path.display()))?;
        let user_home = std::env::var_os("HOME").ok_or_else(|| eyre::eyre!("HOME is not set"))?;
        let token_path = token_path(reference, Path::new(&user_home), directory)?;
        eyre::ensure!(
            metadata(&token_path)?.is_some_and(|m| m.is_file()),
            "Missing private stack token at {}",
            token_path.display()
        );
        let key = crate::projects::private_token(&token_path)?;
        eyre::ensure!(
            !key.is_empty() && !key.chars().any(char::is_control),
            "Invalid stack token"
        );
        return Ok((base.to_owned(), key));
    }
    if let Some(home) = std::env::var_os("HOME")
        && let Some(global) =
            ops::global_credentials(&PathBuf::from(home).join(".config/opencode"))?
    {
        return Ok(global);
    }
    eyre::bail!("Run foundry-opencode init from the stack directory or configure a global fallback")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_config_keeps_public_mcp_registry() {
        let doc = config("https://example.com");
        assert_eq!(
            doc["mcp"]["servers"]["palantir"]["command"][2],
            "--registry=https://registry.npmjs.org"
        );
        assert_eq!(
            doc["mcp"]["servers"]["palantir"]["environment"]["FOUNDRY_TOKEN"],
            "{file:./token}"
        );
        assert_eq!(
            doc["providers"]["palantir-google"]["settings"]["baseURL"],
            "https://example.com/api/v2/llm/proxy/google/v1"
        );
        assert_eq!(
            doc["providers"]["palantir-google"]["headers"]["Authorization"],
            "Bearer {file:./token}"
        );
    }
    #[test]
    fn legacy_credentials_are_parsed_without_running_shell_commands() {
        let root = ops::backup_path(&std::env::temp_dir().join("legacy-read-test"));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join(".envrc"),
            "export OPENCODE_BASE_URL='https://example.com'\nexport OPENCODE_API_KEY='private'\n",
        )
        .unwrap();
        assert_eq!(
            legacy_credentials(&root).unwrap().unwrap(),
            ("https://example.com".into(), "private".into())
        );
        fs::write(root.join(".envrc"), "echo unsafe\n").unwrap();
        assert!(legacy_credentials(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn central_token_references_are_limited_to_the_private_catalog_directory() {
        let home = Path::new("/Users/example");
        let repo = home.join("repo");
        assert_eq!(
            token_path("{file:./token}", home, &repo).unwrap(),
            repo.join(".opencode/token")
        );
        let central = crate::projects::token_file(home, "default");
        assert_eq!(
            token_path(&format!("{{file:{}}}", central.display()), home, &repo).unwrap(),
            central
        );
        assert!(token_path("{file:/etc/passwd}", home, &repo).is_err());
        assert!(token_path("{env:API_KEY}", home, &repo).is_err());
    }
}
