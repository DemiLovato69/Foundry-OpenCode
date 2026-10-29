use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{self, Context};
use serde_json::{Value, json};

const PACKAGE: &str = r#"{"private":true,"type":"module","scripts":{"postinstall":"node patch-opencode-rag.mjs"},"dependencies":{"@opencode-ai/plugin":"1.17.10","opencode-rag-plugin":"1.22.1"}}"#;
const LEGACY_PLUGIN: &str = include_str!("legacy-rag-plugin.txt");
const PATCH: &str = include_str!("rag-patch.mjs");
const CLI: &str = include_str!("rag-cli.mjs");
const IGNORE: &str = "node_modules/\nrag_db/\nrag_db.backup-*/\nrag_db_tmp/\nopencode-rag.log\nruntime-overrides.json\n";

pub fn legacy_plugin(project: &Path) -> eyre::Result<Option<PathBuf>> {
    let open = project.join(".opencode");
    let dir = open.join("plugins");
    let path = dir.join("rag-plugin.js");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    eyre::ensure!(
        fs::symlink_metadata(&open)?.is_dir()
            && !fs::symlink_metadata(&open)?.file_type().is_symlink()
            && fs::symlink_metadata(&dir)?.is_dir()
            && !fs::symlink_metadata(&dir)?.file_type().is_symlink()
            && metadata.is_file()
            && !metadata.file_type().is_symlink(),
        "Unsafe legacy RAG plugin path"
    );
    eyre::ensure!(
        fs::read_to_string(&path)? == LEGACY_PLUGIN,
        "{} is not the known V1 RAG wrapper; leave it untouched and migrate it manually",
        path.display()
    );
    Ok(Some(path))
}

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
        "Initialize a stack with an HTTPS Foundry host first"
    );
    let mut files = files(project, base_url);
    let directory = project.join(".opencode");
    if let Ok(metadata) = fs::symlink_metadata(&directory) {
        eyre::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "{} must be a real directory",
            directory.display()
        );
    }
    for (path, _) in &files {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                eyre::ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "{} must be a regular file",
                    path.display()
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let ignore = &mut files[3];
    if ignore.0.exists() {
        let existing = fs::read_to_string(&ignore.0)?;
        let missing = IGNORE
            .lines()
            .filter(|line| !existing.lines().any(|old| old == *line))
            .collect::<Vec<_>>();
        ignore.1 = format!(
            "{}{}{}",
            existing,
            if !existing.is_empty() && !existing.ends_with('\n') {
                "\n"
            } else {
                ""
            },
            missing
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>()
        );
    }
    for name in ["package-lock.json", "node_modules", "rag_db"] {
        let path = directory.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        eyre::ensure!(
            if name == "package-lock.json" {
                metadata.is_file() && !metadata.file_type().is_symlink()
            } else {
                metadata.is_dir() && !metadata.file_type().is_symlink()
            },
            "{} must be a real file or directory",
            path.display()
        );
    }
    let jsonc = project.join(".opencode/opencode.jsonc");
    let json = project.join(".opencode/opencode.json");
    // Prefer JSONC when both formats exist; retire the other file after activation.
    let config = match fs::symlink_metadata(&jsonc) {
        Ok(_) => jsonc.clone(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json.clone(),
        Err(error) => return Err(error.into()),
    };
    if config == jsonc {
        match fs::symlink_metadata(&json) {
            Ok(metadata) => eyre::ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "{} must be a regular file",
                json.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
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
    let mut doc: Value = original
        .as_deref()
        .and_then(|text| jsonc_parser::parse_to_serde_value(text, &Default::default()).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({"$schema": "https://opencode.ai/config.json"}));
    let mcp = doc
        .as_object_mut()
        .unwrap()
        .entry("mcp")
        .or_insert_with(|| json!({}));
    if !mcp.is_object() {
        *mcp = json!({});
    }
    let servers = mcp
        .as_object_mut()
        .unwrap()
        .entry("servers")
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    let servers = servers.as_object_mut().unwrap();
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
                "Would replace or create RAG files and the MCP server in OpenCode config (backing up replaced files):\n{paths}\nWould run npm install in .opencode and index this project. Existing index is preserved."
            ),
            config,
            original,
            doc,
        ));
    }
    fs::create_dir_all(project.join(".opencode"))?;
    let mut changed = Vec::new();
    let result = (|| -> eyre::Result<()> {
        for (path, text) in &files {
            if fs::read(path).ok().as_deref() == Some(text.as_bytes()) {
                continue;
            }
            let backup = if path.exists() {
                let backup = crate::ops::backup_path(path);
                fs::rename(path, &backup)?;
                Some(backup)
            } else {
                None
            };
            changed.push((path.clone(), backup));
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(text.as_bytes())?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        for (path, backup) in changed.into_iter().rev() {
            let _ = fs::remove_file(&path);
            if let Some(backup) = backup {
                let _ = fs::rename(backup, path);
            }
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
    let text = if config.extension().is_some_and(|ext| ext == "jsonc")
        && original.is_some_and(|text| {
            jsonc_parser::parse_to_serde_value(text, &Default::default()).is_ok_and(
                |value: Value| {
                    value.is_object()
                        && value.get("mcp").is_none_or(Value::is_object)
                        && value["mcp"].get("servers").is_none_or(Value::is_object)
                },
            )
        }) {
        let root = jsonc_parser::cst::CstRootNode::parse(original.unwrap(), &Default::default())?;
        let object = root.object_value_or_set();
        let mcp = if let Some(prop) = object.get("mcp") {
            prop.object_value().unwrap()
        } else {
            object.append("mcp", jsonc_parser::json!({}));
            object.get("mcp").unwrap().object_value().unwrap()
        };
        let servers = if let Some(prop) = mcp.get("servers") {
            prop.object_value().unwrap()
        } else {
            mcp.append("servers", jsonc_parser::json!({}));
            mcp.get("servers").unwrap().object_value().unwrap()
        };
        if let Some(old) = servers.get("rag") {
            old.remove();
        }
        servers.append(
            "rag",
            jsonc_parser::cst::CstInputValue::Object(vec![
                ("type".into(), "local".into()),
                (
                    "command".into(),
                    vec!["node", ".opencode/rag-cli.mjs", "mcp"].into(),
                ),
            ]),
        );
        root.to_string()
    } else {
        format!("{}\n", serde_json::to_string_pretty(doc)?)
    };
    fs::write(&staged, text)?;
    if original.is_some() {
        fs::set_permissions(&staged, fs::metadata(config)?.permissions())?;
    }
    let write = if original.is_some() {
        let backup = crate::ops::backup_path(config);
        if let Err(error) = fs::hard_link(config, &backup) {
            let _ = fs::remove_file(&staged);
            return Err(error.into());
        }
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
    let lock = project.join(".opencode/package-lock.json");
    if lock.is_file() {
        fs::copy(&lock, crate::ops::backup_path(&lock))?;
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
    if config.extension().is_some_and(|ext| ext == "jsonc") {
        let other = project.join(".opencode/opencode.json");
        match fs::symlink_metadata(&other) {
            Ok(metadata) => {
                eyre::ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "Unsafe OpenCode config {}",
                    other.display()
                );
                fs::rename(&other, crate::ops::backup_path(&other))?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(format!("RAG indexed. Project files:\n{paths}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_known_legacy_plugin_can_be_retired() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("legacy-rag-test"));
        fs::create_dir_all(root.join(".opencode/plugins")).unwrap();
        let path = root.join(".opencode/plugins/rag-plugin.js");
        fs::write(&path, LEGACY_PLUGIN).unwrap();
        assert_eq!(legacy_plugin(&root).unwrap(), Some(path.clone()));
        fs::write(&path, "custom plugin").unwrap();
        assert!(legacy_plugin(&root).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "custom plugin");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_is_read_only_with_older_rag_config() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-setup-test"));
        fs::create_dir(&root).unwrap();
        let preview = setup(&root, "https://example.com", true, |_, _| unreachable!()).unwrap();
        assert!(preview.contains(".opencode/rag.json"));
        assert!(!root.join(".opencode").exists());
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join(".opencode/rag.json"), "keep").unwrap();
        assert!(
            setup(&root, "https://example.com", true, |_, _| unreachable!())
                .unwrap()
                .contains("replace or create")
        );
        assert_eq!(
            fs::read_to_string(root.join(".opencode/rag.json")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_preserves_other_mcp_servers_and_replaces_old_rag_entry() {
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
        fs::write(root.join(".opencode/opencode.json"), r#"{"model":"test/model","mcp":{"servers":{"other":{"type":"local","command":["other"]},"rag":{"command":["old"]}}}}"#).unwrap();
        let (_, config, original, doc) = setup_files(&root, "https://example.com", false).unwrap();
        activate_config(&config, original.as_deref(), &doc).unwrap();
        let updated = read_config_for_test(&config);
        assert_eq!(updated["mcp"]["servers"]["other"]["command"][0], "other");
        assert_eq!(updated["mcp"]["servers"]["rag"]["command"][2], "mcp");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn jsonc_config_keeps_comments_when_rag_is_added() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-jsonc-test"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        let path = root.join(".opencode/opencode.jsonc");
        fs::write(
            &path,
            "{\n  // keep this note\n  \"model\": \"test/model\",\n  \"mcp\": {\"servers\": {\"rag\": {\"command\": [\"old\"]}}},\n}\n",
        )
        .unwrap();
        let (_, config, original, doc) = setup_files(&root, "https://example.com", false).unwrap();
        assert_eq!(config, path);
        activate_config(&config, original.as_deref(), &doc).unwrap();
        let result = fs::read_to_string(&path).unwrap();
        assert!(result.contains("// keep this note"));
        assert!(result.contains("rag-cli.mjs"));
        assert!(!result.contains("[\"old\"]"));
        assert!(!root.join(".opencode/opencode.json").exists());
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
        assert!(preview.contains("replace or create"));
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
    fn retry_overwrites_older_artifacts_and_preserves_index() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-conflict-test"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        let directory = root.join(".opencode");
        fs::write(directory.join(".gitignore"), "token\ncustom-secret\n").unwrap();
        fs::create_dir(directory.join("rag_db")).unwrap();
        fs::write(directory.join("rag_db/existing"), "keep").unwrap();
        let lock = directory.join("package-lock.json");
        fs::write(&lock, "not ours").unwrap();
        setup_files(&root, "https://example.com", false).unwrap();
        setup_files(&root, "https://example.com", false).unwrap();
        fs::write(&lock, r#"{"packages":{"":{"dependencies":{"other":"1"}}}}"#).unwrap();
        setup_files(&root, "https://example.com", false).unwrap();
        assert_eq!(
            fs::read_to_string(&lock).unwrap(),
            r#"{"packages":{"":{"dependencies":{"other":"1"}}}}"#
        );
        fs::write(directory.join("rag.json"), "custom").unwrap();
        assert!(setup_files(&root, "https://example.com", true).is_ok());
        assert_eq!(
            fs::read_to_string(directory.join("rag.json")).unwrap(),
            "custom"
        );
        setup_files(&root, "https://example.com", false).unwrap();
        assert_ne!(
            fs::read_to_string(directory.join("rag.json")).unwrap(),
            "custom"
        );
        let ignored = fs::read_to_string(directory.join(".gitignore")).unwrap();
        assert!(ignored.contains("custom-secret\n"));
        assert!(ignored.contains("rag_db/\n"));
        assert!(fs::read_dir(&directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("rag.json.backup-")
        }));
        assert_eq!(
            fs::read_to_string(directory.join("rag_db/existing")).unwrap(),
            "keep"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn both_config_formats_are_backed_up_without_blocking_setup() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-both-configs-test"));
        fs::create_dir_all(root.join(".opencode")).unwrap();
        let directory = root.join(".opencode");
        fs::write(directory.join("opencode.json"), "{\"model\":\"old\"}").unwrap();
        fs::write(directory.join("opencode.jsonc"), "{\"model\":\"new\"}").unwrap();
        setup(&root, "https://example.com", false, |_, _| Ok(())).unwrap();
        assert!(!directory.join("opencode.json").exists());
        assert!(
            fs::read_to_string(directory.join("opencode.jsonc"))
                .unwrap()
                .contains("rag-cli.mjs")
        );
        assert!(fs::read_dir(&directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("opencode.json.backup-")
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_config_is_backed_up_and_replaced() {
        let root = crate::ops::backup_path(&std::env::temp_dir().join("rag-invalid-config-test"));
        fs::create_dir_all(root.join(".opencode")).unwrap();
        let config = root.join(".opencode/opencode.jsonc");
        fs::write(&config, "{invalid").unwrap();
        setup(&root, "https://example.com", false, |_, _| Ok(())).unwrap();
        assert!(fs::read_to_string(&config).unwrap().contains("rag-cli.mjs"));
        assert!(fs::read_dir(root.join(".opencode")).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("opencode.jsonc.backup-")
        }));
        fs::write(&config, "{\"mcp\":null}").unwrap();
        setup(&root, "https://example.com", false, |_, _| Ok(())).unwrap();
        assert!(fs::read_to_string(&config).unwrap().contains("rag-cli.mjs"));
        fs::remove_dir_all(root).unwrap();
    }

    fn read_config_for_test(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }
}
