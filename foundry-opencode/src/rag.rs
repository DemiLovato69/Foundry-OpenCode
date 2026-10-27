use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{self, Context};
use serde_json::{Value, json};

const PACKAGE: &str = r#"{"private":true,"type":"module","scripts":{"postinstall":"node patch-opencode-rag.mjs"},"dependencies":{"@opencode-ai/plugin":"1.17.10","opencode-rag-plugin":"1.22.1"}}"#;
const PATCH: &str = include_str!("rag-patch.mjs");
const CLI: &str = include_str!("rag-cli.mjs");
const IGNORE: &str = "node_modules/\nrag_db/\nrag_db.backup-*/\nrag_db_tmp/\nopencode-rag.log\nruntime-overrides.json\n";

fn files(project: &Path, base_url: &str) -> Vec<(std::path::PathBuf, String)> {
    vec![
        (
            project.join(".opencode/package.json"),
            format!("{PACKAGE}\n"),
        ),
        (
            project.join(".opencode/patch-opencode-rag.mjs"),
            PATCH.to_owned(),
        ),
        (project.join(".opencode/rag-cli.mjs"), CLI.to_owned()),
        (project.join(".opencode/.gitignore"), IGNORE.to_owned()),
        (
            project.join(".opencode/rag.json"),
            format!(
                "{}\n",
                json!({
                    "embedding": {"provider": "openai", "baseUrl": format!("{base_url}/api/v2/llm/proxy/openai/v1"), "model": "ri.language-model-service..language-model.text-embedding-3-large_azure", "timeoutMs": 60000, "vectorDimension": 3072},
                    "vectorStore": {"path": "./.opencode/rag_db"},
                    "description": {"enabled": false},
                    "imageDescription": {"enabled": false},
                    "indexing": {"excludeDirs": ["node_modules", ".git", ".opencode", "build", "dist", ".venv", "__pycache__"], "excludeFiles": ["**/*.png", "**/*.jpg", "**/*.jpeg", "**/*.pdf", "package-lock.json", "pnpm-lock.yaml"], "embedBatchSize": 32, "embedConcurrency": 2}
                })
            ),
        ),
    ]
}

fn setup_files(
    project: &Path,
    base_url: &str,
    dry_run: bool,
) -> eyre::Result<(String, PathBuf, Option<String>, Value)> {
    eyre::ensure!(
        crate::ops::valid_url(base_url),
        "Set OPENCODE_BASE_URL to an HTTPS Foundry host"
    );
    let files = files(project, base_url);
    let directory = project.join(".opencode");
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        eyre::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "{} must be a real directory",
            directory.display()
        );
    }
    let mut reused = 0;
    let mut package_owned = false;
    let mut rag_owned = false;
    let mut missing = Vec::new();
    for (index, (path, expected)) in files.iter().enumerate() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                eyre::ensure!(
                    metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && fs::read_to_string(path)? == *expected,
                    "{} differs from the installer RAG file; not changing it",
                    path.display()
                );
                reused += 1;
                if index == 0 {
                    package_owned = true;
                }
                if index == 4 {
                    rag_owned = true;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing.push(path),
            Err(error) => return Err(error.into()),
        }
    }
    for name in ["package-lock.json", "node_modules", "rag_db"] {
        let path = directory.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        eyre::ensure!(
            package_owned
                && (name != "rag_db" || rag_owned)
                && if name == "package-lock.json" {
                    metadata.is_file() && !metadata.file_type().is_symlink()
                } else {
                    metadata.is_dir() && !metadata.file_type().is_symlink()
                },
            "{} already exists and is not verified installer RAG data; not changing it",
            path.display()
        );
        if name == "package-lock.json" {
            let lock: Value = serde_json::from_slice(&fs::read(&path)?)
                .wrap_err_with(|| format!("Invalid {}; not changing it", path.display()))?;
            let package: Value = serde_json::from_str(PACKAGE)?;
            eyre::ensure!(
                lock["packages"][""]["dependencies"] == package["dependencies"],
                "{} does not match the installer dependencies; not changing it",
                path.display()
            );
        }
    }
    let jsonc = project.join(".opencode/opencode.jsonc");
    eyre::ensure!(
        !jsonc.exists(),
        "{} exists; configure its mcp.servers.rag entry manually rather than overriding it",
        jsonc.display()
    );
    let config = project.join(".opencode/opencode.json");
    if let Ok(metadata) = fs::symlink_metadata(&config) {
        eyre::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "{} must be a regular file",
            config.display()
        );
    }
    let original = match fs::read_to_string(&config) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    let mut doc: Value = match &original {
        Some(text) => {
            serde_json::from_str(text).wrap_err("Invalid OpenCode JSON; not changing it")?
        }
        None => json!({"$schema": "https://opencode.ai/config.json"}),
    };
    eyre::ensure!(doc.is_object(), "OpenCode config must be a JSON object");
    let mcp = doc
        .as_object_mut()
        .unwrap()
        .entry("mcp")
        .or_insert_with(|| json!({}));
    eyre::ensure!(mcp.is_object(), "OpenCode mcp config must be an object");
    let servers = mcp
        .as_object_mut()
        .unwrap()
        .entry("servers")
        .or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| eyre::eyre!("OpenCode mcp.servers must be an object"))?;
    eyre::ensure!(
        !servers.contains_key("rag"),
        "mcp.servers.rag already exists; not replacing it"
    );
    servers.insert(
        "rag".into(),
        json!({"type": "local", "command": ["node", ".opencode/rag-cli.mjs", "mcp"]}),
    );
    let paths = files
        .iter()
        .map(|(p, _)| p.display().to_string())
        .chain(std::iter::once(config.display().to_string()))
        .collect::<Vec<_>>()
        .join("\n");
    if dry_run {
        return Ok((
            format!(
                "Would {} RAG files and add the MCP server to OpenCode config:\n{paths}\nWould run npm install in .opencode and index this project.",
                if reused > 0 {
                    "reuse matching or create missing"
                } else {
                    "create"
                }
            ),
            config,
            original,
            doc,
        ));
    }
    fs::create_dir_all(project.join(".opencode"))?;
    let mut created = Vec::new();
    let result = (|| -> eyre::Result<()> {
        for (path, text) in &files {
            if !missing.contains(&path) {
                continue;
            }
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            created.push(path.clone());
            file.write_all(text.as_bytes())?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for path in created {
            let _ = fs::remove_file(path);
        }
        return Err(e);
    }
    Ok((paths, config, original, doc))
}

fn activate_config(config: &Path, original: Option<&str>, doc: &Value) -> eyre::Result<()> {
    // Keep the MCP server disabled until installation and indexing have succeeded.
    eyre::ensure!(
        fs::read_to_string(config).ok().as_deref() == original,
        "{} changed during setup",
        config.display()
    );
    let staged = crate::ops::backup_path(config);
    fs::write(&staged, format!("{}\n", serde_json::to_string_pretty(doc)?))?;
    let write = if original.is_some() {
        fs::rename(&staged, config)
    } else {
        fs::hard_link(&staged, config)
    };
    let _ = fs::remove_file(staged);
    write?;
    Ok(())
}

pub fn setup(
    project: &Path,
    base_url: &str,
    dry_run: bool,
    mut run: impl FnMut(&str, Command) -> eyre::Result<()>,
) -> eyre::Result<String> {
    let (paths, config, original, doc) = setup_files(project, base_url, dry_run)?;
    if dry_run {
        return Ok(paths);
    }
    let mut npm = Command::new("npm");
    npm.arg("install").current_dir(project.join(".opencode"));
    run("Installing RAG dependencies (npm install)", npm)
        .wrap_err("RAG setup files remain; npm is required")?;
    let mut node = Command::new("node");
    node.args([".opencode/rag-cli.mjs", "index"])
        .current_dir(project);
    run("Indexing project (node .opencode/rag-cli.mjs index)", node)
        .wrap_err("RAG setup files remain; retry with node .opencode/rag-cli.mjs index")?;
    activate_config(&config, original.as_deref(), &doc)?;
    Ok(format!("RAG indexed. Project files:\n{paths}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_is_read_only_and_preserves_conflicts() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-setup-test"));
        fs::create_dir(&root).unwrap();
        let preview = setup(&root, "https://example.com", true, |_, _| unreachable!()).unwrap();
        assert!(preview.contains(".opencode/rag.json"));
        assert!(!root.join(".opencode").exists());
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join(".opencode/rag.json"), "keep").unwrap();
        assert!(setup(&root, "https://example.com", true, |_, _| unreachable!()).is_err());
        assert_eq!(
            fs::read_to_string(root.join(".opencode/rag.json")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_preserves_other_mcp_servers_and_refuses_reruns() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-setup-test"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join(".opencode/opencode.json"), r#"{"model":"test/model","mcp":{"servers":{"other":{"type":"local","command":["other"]}}}}"#).unwrap();
        let (_, config, original, doc) = setup_files(&root, "https://example.com", false).unwrap();
        assert!(serde_json::from_str::<Value>(&fs::read_to_string(&config).unwrap()).unwrap()["mcp"]["servers"].get("rag").is_none());
        activate_config(&config, original.as_deref(), &doc).unwrap();
        let doc: Value = serde_json::from_str(
            &fs::read_to_string(root.join(".opencode/opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(doc["model"], "test/model");
        assert_eq!(doc["mcp"]["servers"]["other"]["command"][0], "other");
        assert_eq!(doc["mcp"]["servers"]["rag"]["command"][2], "mcp");
        assert!(setup_files(&root, "https://example.com", false).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retries_verified_partial_setup_without_discarding_index() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-retry-test"));
        fs::create_dir(&root).unwrap();
        setup_files(&root, "https://example.com", false).unwrap();
        let directory = root.join(".opencode");
        fs::create_dir(directory.join("node_modules")).unwrap();
        fs::create_dir(directory.join("rag_db")).unwrap();
        fs::write(directory.join("rag_db/existing"), "keep").unwrap();
        let lock = json!({"packages": {"": {"dependencies": serde_json::from_str::<Value>(PACKAGE).unwrap()["dependencies"]}}});
        fs::write(directory.join("package-lock.json"), lock.to_string()).unwrap();
        fs::remove_file(directory.join("rag-cli.mjs")).unwrap();
        let preview = setup(&root, "https://example.com", true, |_, _| unreachable!()).unwrap();
        assert!(preview.contains("reuse matching"));
        assert!(!directory.join("rag-cli.mjs").exists());
        assert!(!directory.join("opencode.json").exists());
        let mut commands = Vec::new();
        assert!(
            setup(&root, "https://example.com", false, |label, _| {
                commands.push(label.to_string());
                if commands.len() == 2 {
                    eyre::bail!("index unavailable")
                }
                Ok(())
            })
            .is_err()
        );
        assert!(!directory.join("opencode.json").exists());
        assert_eq!(
            fs::read_to_string(directory.join("rag-cli.mjs")).unwrap(),
            CLI
        );
        assert_eq!(
            fs::read_to_string(directory.join("rag_db/existing")).unwrap(),
            "keep"
        );
        commands.clear();
        setup(&root, "https://example.com", false, |label, _| {
            commands.push(label.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            fs::read_to_string(directory.join("rag_db/existing")).unwrap(),
            "keep"
        );
        assert!(
            fs::read_to_string(directory.join("opencode.json"))
                .unwrap()
                .contains("rag-cli.mjs")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn retry_rejects_unrelated_or_modified_artifacts() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-conflict-test"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        let directory = root.join(".opencode");
        let lock = directory.join("package-lock.json");
        fs::write(&lock, "not ours").unwrap();
        assert!(setup_files(&root, "https://example.com", false).is_err());
        assert!(!directory.join("package.json").exists());
        fs::remove_file(&lock).unwrap();
        setup_files(&root, "https://example.com", false).unwrap();
        fs::write(&lock, r#"{"packages":{"":{"dependencies":{"other":"1"}}}}"#).unwrap();
        assert!(setup_files(&root, "https://example.com", false).is_err());
        assert_eq!(
            fs::read_to_string(&lock).unwrap(),
            r#"{"packages":{"":{"dependencies":{"other":"1"}}}}"#
        );
        fs::remove_file(&lock).unwrap();
        fs::write(directory.join("rag.json"), "custom").unwrap();
        assert!(setup_files(&root, "https://example.com", true).is_err());
        assert_eq!(
            fs::read_to_string(directory.join("rag.json")).unwrap(),
            "custom"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
