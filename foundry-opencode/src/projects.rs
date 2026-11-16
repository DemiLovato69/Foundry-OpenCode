use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use color_eyre::eyre::{self, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ops, rag, stack};

#[derive(Clone, Serialize, Deserialize)]
pub struct Stack {
    pub id: String,
    pub name: String,
    pub url: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    pub path: PathBuf,
    pub stack: String,
    pub rag: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Catalog {
    pub default: Option<Stack>,
    pub stacks: Vec<Stack>,
    pub projects: Vec<Project>,
    /// Install the `git sync-worktrees` alias (opt-in).
    #[serde(default)]
    pub sync_alias: bool,
    #[serde(skip)]
    pub tokens: BTreeMap<String, String>,
}

fn location(home: &Path) -> PathBuf {
    home.join(".config/foundry-opencode/catalog.json")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

pub fn token_file(home: &Path, id: &str) -> PathBuf {
    home.join(".config/foundry-opencode/tokens").join(id)
}

pub fn private_token(path: &Path) -> eyre::Result<String> {
    let parent = fs::symlink_metadata(path.parent().unwrap())?;
    eyre::ensure!(
        parent.is_dir() && !parent.file_type().is_symlink(),
        "Unsafe token directory {}",
        path.parent().unwrap().display()
    );
    let meta = fs::symlink_metadata(path)?;
    eyre::ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "Unsafe token file {}",
        path.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        eyre::ensure!(
            meta.permissions().mode() & 0o077 == 0,
            "Token is not private: {}",
            path.display()
        );
    }
    let token = fs::read_to_string(path)?;
    eyre::ensure!(
        !token.is_empty() && !token.chars().any(char::is_control),
        "Invalid token at {}",
        path.display()
    );
    Ok(token)
}

impl Catalog {
    pub fn saved(home: &Path) -> eyre::Result<Self> {
        if fs::symlink_metadata(location(home))
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
        {
            eyre::bail!("No saved project catalog; run update without --yes to configure it");
        }
        let catalog = Self::load(home)?;
        eyre::ensure!(
            catalog.default.is_some() || !catalog.stacks.is_empty() || !catalog.projects.is_empty(),
            "Saved project catalog is empty; run update without --yes to configure it"
        );
        catalog.validate(home)?;
        let mut paths = std::collections::BTreeSet::new();
        for project in &catalog.projects {
            eyre::ensure!(
                paths.insert(&project.path),
                "Duplicate saved repository {}",
                project.path.display()
            );
            preflight(&project.path).wrap_err_with(|| {
                format!(
                    "Saved repository {} needs review in the update wizard",
                    project.path.display()
                )
            })?;
        }
        Ok(catalog)
    }

    pub fn validate(&self, home: &Path) -> eyre::Result<()> {
        let path = location(home);
        match fs::symlink_metadata(&path) {
            Ok(meta) => eyre::ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unsafe project catalog"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for stack in self.choices() {
            eyre::ensure!(
                valid_id(&stack.id)
                    && ids.insert(&stack.id)
                    && ops::valid_url(&stack.url)
                    && names.insert(stack.name.to_lowercase())
                    && !stack.name.trim().is_empty()
                    && !stack.name.chars().any(char::is_control),
                "Invalid stack catalog entry"
            );
            let token = self
                .tokens
                .get(&stack.id)
                .ok_or_else(|| eyre::eyre!("Missing token for {}", stack.name))?;
            eyre::ensure!(
                !token.is_empty() && !token.chars().any(char::is_control),
                "Invalid token for {}",
                stack.name
            );
        }
        eyre::ensure!(
            self.projects
                .iter()
                .all(|project| ids.contains(&project.stack)),
            "Repository has an unknown stack"
        );
        for directory in [
            location(home).parent().unwrap().to_path_buf(),
            token_file(home, "default").parent().unwrap().to_path_buf(),
        ] {
            match fs::symlink_metadata(&directory) {
                Ok(meta) => eyre::ensure!(
                    meta.is_dir() && !meta.file_type().is_symlink(),
                    "Unsafe catalog directory {}",
                    directory.display()
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn inventory(home: &Path) -> eyre::Result<Self> {
        let path = location(home);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                eyre::ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "Unsafe project catalog"
                );
                Ok(serde_json::from_slice(&fs::read(&path)?)?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn load(home: &Path) -> eyre::Result<Self> {
        let path = location(home);
        let mut catalog: Self = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                eyre::ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "Unsafe catalog"
                );
                serde_json::from_slice(&fs::read(&path)?)?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut catalog = Self::default();
                if let Some((url, token)) = ops::global_credentials(&home.join(".config/opencode"))
                    .ok()
                    .flatten()
                {
                    catalog.default = Some(Stack {
                        id: "default".into(),
                        name: "Global default".into(),
                        url,
                    });
                    catalog.tokens.insert("default".into(), token);
                }
                for path in crate::load_stacks(home)? {
                    let old_config = path.join(".opencode/opencode.json");
                    let credentials = if old_config.exists() {
                        match stack::read_config(&old_config) {
                            Ok(doc)
                                if doc["providers"]["palantir-openai"]["settings"]["baseURL"]
                                    .as_str()
                                    .is_some() =>
                            {
                                stack::credentials(&path)
                            }
                            _ => Err(eyre::eyre!("Not a generated stack config")),
                        }
                    } else {
                        stack::legacy_credentials(&path).and_then(|pair| {
                            pair.ok_or_else(|| eyre::eyre!("No legacy credentials"))
                        })
                    };
                    if let Ok((url, token)) = credentials {
                        let id = format!("legacy-{}", catalog.stacks.len());
                        catalog.stacks.push(Stack {
                            id: id.clone(),
                            name: path.display().to_string(),
                            url,
                        });
                        catalog.tokens.insert(id, token);
                    }
                }
                return Ok(catalog);
            }
            Err(error) => return Err(error.into()),
        };
        for item in catalog.default.iter().chain(&catalog.stacks) {
            eyre::ensure!(
                valid_id(&item.id)
                    && ops::valid_url(&item.url)
                    && !catalog.tokens.contains_key(&item.id),
                "Invalid saved stack {}",
                item.name
            );
            let legacy = token_file(home, &item.id);
            let project = catalog
                .projects
                .iter()
                .find(|project| project.stack == item.id);
            let source = if let Some(project) = project {
                let local = project.path.join(".opencode/token");
                if fs::symlink_metadata(&local).is_ok() {
                    local
                } else {
                    let config = stack::read_config(&project.path.join(".opencode/opencode.json"));
                    if config.is_ok_and(|doc| {
                        doc["providers"]["palantir-openai"]["settings"]["apiKey"]
                            == format!("{{file:{}}}", legacy.display())
                    }) {
                        legacy.clone()
                    } else {
                        local
                    }
                }
            } else {
                legacy
            };
            let token = match private_token(&source) {
                Ok(token) => token,
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
                {
                    String::new()
                }
                Err(error) => return Err(error),
            };
            catalog.tokens.insert(item.id.clone(), token);
        }
        Ok(catalog)
    }

    pub fn choices(&self) -> Vec<&Stack> {
        self.default.iter().chain(&self.stacks).collect()
    }

    pub fn save(&self, home: &Path) -> eyre::Result<()> {
        self.validate(home)?;
        let path = location(home);
        fs::create_dir_all(path.parent().unwrap())?;
        eyre::ensure!(
            fs::symlink_metadata(path.parent().unwrap())?.is_dir()
                && !fs::symlink_metadata(path.parent().unwrap())?
                    .file_type()
                    .is_symlink(),
            "Unsafe catalog directory"
        );
        let folder = token_file(home, "default").parent().unwrap().to_path_buf();
        fs::create_dir_all(&folder)?;
        eyre::ensure!(
            fs::symlink_metadata(&folder)?.is_dir()
                && !fs::symlink_metadata(&folder)?.file_type().is_symlink(),
            "Unsafe token directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
        }
        for item in self.choices() {
            if self.projects.iter().any(|project| project.stack == item.id) {
                continue;
            }
            eyre::ensure!(
                valid_id(&item.id) && ops::valid_url(&item.url),
                "Invalid stack {}",
                item.name
            );
            let token = self
                .tokens
                .get(&item.id)
                .ok_or_else(|| eyre::eyre!("Missing token for {}", item.name))?;
            eyre::ensure!(
                !token.is_empty() && !token.chars().any(char::is_control),
                "Invalid token for {}",
                item.name
            );
            let target = token_file(home, &item.id);
            if fs::symlink_metadata(&target).is_ok() && private_token(&target)? == *token {
                continue;
            }
            let staged = ops::backup_path(&target);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&staged)?;
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            let backup = if fs::symlink_metadata(&target).is_ok() {
                eyre::ensure!(
                    fs::symlink_metadata(&target)?.is_file()
                        && !fs::symlink_metadata(&target)?.file_type().is_symlink(),
                    "Unsafe existing token"
                );
                let backup = ops::backup_path(&target);
                fs::rename(&target, &backup)?;
                Some(backup)
            } else {
                None
            };
            if let Err(error) = fs::rename(&staged, &target) {
                if let Some(backup) = backup {
                    fs::rename(backup, &target)?;
                }
                let _ = fs::remove_file(staged);
                return Err(error.into());
            }
        }
        let staged = ops::backup_path(&path);
        fs::write(&staged, serde_json::to_vec_pretty(self)?)?;
        fs::rename(staged, path)?;
        Ok(())
    }
}

pub fn is_repo(path: &Path) -> eyre::Result<bool> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()?;
    Ok(output.status.success()
        && Path::new(String::from_utf8(output.stdout)?.trim()).canonicalize()?
            == path.canonicalize()?)
}

pub fn project_config(_home: &Path, stack: &Stack) -> Value {
    let mut config = stack::config(&stack.url);
    let token = "{file:./token}";
    for provider in ["palantir-openai", "palantir-anthropic", "palantir-google"] {
        config["providers"][provider]["settings"]["apiKey"] = token.into();
        config["providers"][provider]["headers"]["Authorization"] =
            format!("Bearer {token}").into();
    }
    config["mcp"]["servers"]["palantir"]["environment"]["FOUNDRY_TOKEN"] = token.into();
    config
}

pub fn config_paths(repo: &Path) -> Vec<PathBuf> {
    [
        "opencode.json",
        "opencode.jsonc",
        ".opencode/opencode.json",
        ".opencode/opencode.jsonc",
    ]
    .iter()
    .map(|name| repo.join(name))
    .collect()
}

pub fn preflight(repo: &Path) -> eyre::Result<()> {
    let folder = fs::symlink_metadata(repo)?;
    eyre::ensure!(
        folder.is_dir() && !folder.file_type().is_symlink(),
        "{} is not a real directory",
        repo.display()
    );
    if is_repo(repo).unwrap_or(false)
        && let Some(main) = crate::worktrees::main_of(repo)?
    {
        eyre::bail!(
            "{} is a Git worktree of {}; select {} instead. Its worktrees share that configuration (foundry-opencode sync-worktrees)",
            repo.display(),
            main.display(),
            main.display()
        );
    }
    let open = repo.join(".opencode");
    match fs::symlink_metadata(&open) {
        Ok(meta) => eyre::ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Unsafe config directory {}",
            open.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for path in config_paths(repo) {
        match fs::symlink_metadata(&path) {
            Ok(meta) => eyre::ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unsafe config {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    for path in [open.join("token"), open.join(".gitignore")] {
        match fs::symlink_metadata(&path) {
            Ok(meta) => eyre::ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Unsafe local file {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if open.join("token").exists() {
        private_token(&open.join("token"))?;
    }
    if is_repo(repo).unwrap_or(false) {
        let tracked = Command::new("git")
            .args(["ls-files", "--error-unmatch", "--", ".opencode/token"])
            .current_dir(repo)
            .output()?;
        eyre::ensure!(
            !tracked.status.success(),
            "{} is tracked by Git; untrack and rotate it before continuing",
            open.join("token").display()
        );
    }
    Ok(())
}

pub fn write_project(
    home: &Path,
    project: &Project,
    stack: &Stack,
    secret: &str,
) -> eyre::Result<Vec<PathBuf>> {
    preflight(&project.path)?;
    eyre::ensure!(
        !secret.is_empty() && !secret.chars().any(char::is_control),
        "Invalid Foundry token"
    );
    let open = project.path.join(".opencode");
    let paths = config_paths(&project.path);
    fs::create_dir_all(&open)?;
    let local_token = open.join("token");
    let tracked = Command::new("git")
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(&local_token)
        .current_dir(&project.path)
        .output();
    eyre::ensure!(
        !tracked.is_ok_and(|output| output.status.success()),
        "{} is tracked by Git; untrack and rotate it before continuing",
        local_token.display()
    );
    let ignore = open.join(".gitignore");
    match fs::symlink_metadata(&ignore) {
        Ok(meta) => eyre::ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "Unsafe ignore file {}",
            ignore.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let existing_ignore = match fs::read_to_string(&ignore) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    const RULES: &str = "*\n!commands/\n!commands/**\n";
    if !existing_ignore.ends_with(RULES) {
        let staged_ignore = ops::backup_path(&ignore);
        fs::write(
            &staged_ignore,
            format!(
                "{existing_ignore}{}{RULES}",
                if existing_ignore.is_empty() || existing_ignore.ends_with('\n') {
                    ""
                } else {
                    "\n"
                }
            ),
        )?;
        fs::rename(staged_ignore, &ignore)?;
    }
    let target = project.path.join(".opencode/opencode.json");
    let staged = ops::backup_path(&target);
    let mut config = project_config(home, stack);
    if project.rag && rag::ready(&project.path, &stack.url) {
        config["mcp"]["servers"]["rag"] = serde_json::json!({
            "type": "local", "command": ["node", ".opencode/rag-cli.mjs", "mcp"],
            "environment": {"OPENAI_API_KEY": config["providers"]["palantir-openai"]["settings"]["apiKey"]}
        });
    }
    fs::write(&staged, serde_json::to_vec_pretty(&config)?)?;
    let mut backups = Vec::<(PathBuf, PathBuf)>::new();
    let result = (|| -> eyre::Result<()> {
        let token_matches = fs::symlink_metadata(&local_token)
            .map(|meta| {
                eyre::ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "Unsafe local token"
                );
                Ok(private_token(&local_token)? == secret)
            })
            .unwrap_or(Ok(false))?;
        if !token_matches {
            let staged_token = ops::backup_path(&local_token);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&staged_token)?;
            file.write_all(secret.as_bytes())?;
            file.sync_all()?;
            if local_token.exists() {
                let backup = ops::backup_path(&local_token);
                fs::hard_link(&local_token, &backup)?;
                backups.push((local_token.clone(), backup));
            }
            fs::rename(&staged_token, &local_token)?;
        }
        for path in paths
            .iter()
            .filter(|path| path != &&target && path.exists())
        {
            let backup = ops::backup_path(path);
            fs::rename(path, &backup)?;
            backups.push((path.clone(), backup));
        }
        if target.exists() {
            let backup = ops::backup_path(&target);
            fs::hard_link(&target, &backup)?;
            backups.push((target.clone(), backup));
        }
        fs::rename(&staged, &target).wrap_err("Could not install project config")?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(staged);
        for (original, backup) in backups.iter().rev() {
            if original == &local_token || !original.exists() {
                fs::rename(backup, original).wrap_err_with(|| {
                    format!(
                        "Could not restore {} from {}",
                        original.display(),
                        backup.display()
                    )
                })?;
            }
        }
        return Err(error);
    }
    Ok(backups.into_iter().map(|(_, backup)| backup).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_config_uses_local_token_and_public_registry() {
        let home = Path::new("/Users/example");
        let stack = Stack {
            id: "default".into(),
            name: "Global default".into(),
            url: "https://foundry.example".into(),
        };
        let doc = project_config(home, &stack);
        assert_eq!(
            doc["providers"]["palantir-openai"]["settings"]["apiKey"],
            "{file:./token}"
        );
        assert_eq!(
            doc["mcp"]["servers"]["palantir"]["command"][2],
            "--registry=https://registry.npmjs.org"
        );
        assert_eq!(
            doc["mcp"]["servers"]["palantir"]["environment"]["FOUNDRY_TOKEN"],
            doc["providers"]["palantir-openai"]["settings"]["apiKey"]
        );
        assert_eq!(
            doc["providers"]["palantir-google"]["settings"]["apiKey"],
            doc["providers"]["palantir-openai"]["settings"]["apiKey"]
        );
        assert_eq!(
            doc["providers"]["palantir-google"]["headers"]["Authorization"],
            "Bearer {file:./token}"
        );
    }

    #[test]
    fn ordinary_folder_gets_ignored_private_local_token() {
        let root = ops::backup_path(&std::env::temp_dir().join("folder-config-test"));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join(".opencode/.gitignore"), "!token\n").unwrap();
        let stack = Stack {
            id: "default".into(),
            name: "Global default".into(),
            url: "https://example.com".into(),
        };
        let project = Project {
            path: root.canonicalize().unwrap(),
            stack: "default".into(),
            rag: false,
        };
        write_project(&root, &project, &stack, "private").unwrap();
        let config = read_config(&root.join(".opencode/opencode.json"));
        assert_eq!(
            config["providers"]["palantir-openai"]["settings"]["apiKey"],
            "{file:./token}"
        );
        assert_eq!(
            private_token(&root.join(".opencode/token")).unwrap(),
            "private"
        );
        assert!(
            fs::read_to_string(root.join(".opencode/.gitignore"))
                .unwrap()
                .ends_with("*\n!commands/\n!commands/**\n")
        );
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["check-ignore", "--quiet", "--", ".opencode/token"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(stack::credentials(&root).unwrap().1, "private");
        let moved = ops::backup_path(&root);
        fs::rename(&root, &moved).unwrap();
        assert_eq!(stack::credentials(&moved).unwrap().1, "private");
        fs::remove_dir_all(moved).unwrap();
    }

    #[test]
    fn replacing_repo_configs_backs_up_all_formats_without_removing_other_files() {
        let root = ops::backup_path(&std::env::temp_dir().join("project-config-test"));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join(".opencode")).unwrap();
        fs::write(root.join("opencode.jsonc"), "{\"model\":\"mine\"}").unwrap();
        fs::write(root.join(".opencode/opencode.json"), "{\"model\":\"old\"}").unwrap();
        fs::write(
            root.join(".opencode/opencode.jsonc"),
            "{\"model\":\"other\"}",
        )
        .unwrap();
        fs::write(root.join(".opencode/plugin.js"), "preserve").unwrap();
        let stack = Stack {
            id: "default".into(),
            name: "Global default".into(),
            url: "https://example.com".into(),
        };
        let project = Project {
            path: root.canonicalize().unwrap(),
            stack: "default".into(),
            rag: false,
        };
        let backups = write_project(&root, &project, &stack, "private").unwrap();
        assert_eq!(backups.len(), 3);
        assert_eq!(
            fs::read_to_string(root.join(".opencode/plugin.js")).unwrap(),
            "preserve"
        );
        assert!(!root.join("opencode.jsonc").exists());
        assert!(!root.join(".opencode/opencode.jsonc").exists());
        assert_eq!(
            read_config(&root.join(".opencode/opencode.json"))["providers"]["palantir-openai"]["settings"]
                ["baseURL"],
            "https://example.com/language-model-service/api/proxy/openai/v1"
        );
        assert!(backups.iter().all(|path| path.exists()));
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_symlinked_project_config_before_writing() {
        let root = ops::backup_path(&std::env::temp_dir().join("project-symlink-test"));
        fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(root.join("other")).unwrap();
        std::os::unix::fs::symlink(root.join("other"), root.join(".opencode")).unwrap();
        assert!(preflight(&root).is_err());
        assert!(!root.join("other/opencode.json").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_linked_or_public_local_tokens() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = ops::backup_path(&std::env::temp_dir().join("project-token-preflight"));
        fs::create_dir_all(root.join(".opencode")).unwrap();
        fs::write(root.join("elsewhere"), "secret").unwrap();
        symlink(root.join("elsewhere"), root.join(".opencode/token")).unwrap();
        assert!(preflight(&root).is_err());
        fs::remove_file(root.join(".opencode/token")).unwrap();
        fs::write(root.join(".opencode/token"), "secret").unwrap();
        fs::set_permissions(
            root.join(".opencode/token"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(preflight(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn worktrees_point_to_their_main_checkout_instead_of_becoming_projects() {
        let root = ops::backup_path(&std::env::temp_dir().join("project-worktree-test"))
            .with_extension("d");
        let main = root.join("main");
        fs::create_dir_all(&main).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
                    .args(["-c", "commit.gpgsign=false"])
                    .args(args)
                    .current_dir(&main)
                    .status()
                    .unwrap()
                    .success()
            )
        };
        git(&["init", "-q"]);
        git(&["commit", "-q", "--allow-empty", "-m", "start"]);
        git(&["worktree", "add", "-q", "--detach", "../linked"]);
        assert!(preflight(&main).is_ok());
        let error = preflight(&root.join("linked")).unwrap_err().to_string();
        assert!(error.contains("is a Git worktree of"), "{error}");
        assert!(
            error.contains(&main.canonicalize().unwrap().display().to_string()),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn read_config(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn catalog_keeps_tokens_out_of_registry_and_loads_private_files() {
        let home = ops::backup_path(&std::env::temp_dir().join("project-catalog-test"));
        fs::create_dir(&home).unwrap();
        let mut catalog = Catalog {
            default: Some(Stack {
                id: "default".into(),
                name: "Global default".into(),
                url: "https://example.com".into(),
            }),
            ..Catalog::default()
        };
        catalog
            .tokens
            .insert("default".into(), "private-key".into());
        catalog.save(&home).unwrap();
        assert!(
            !fs::read_to_string(location(&home))
                .unwrap()
                .contains("private-key")
        );
        assert_eq!(
            Catalog::load(&home).unwrap().tokens["default"],
            "private-key"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(token_file(&home, "default"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn saved_defaults_keep_assignments_and_refuse_invalid_repositories_or_tokens() {
        let home = ops::backup_path(&std::env::temp_dir().join("saved-update-test"));
        fs::create_dir_all(home.join("repo")).unwrap();
        let home = home.canonicalize().unwrap();
        assert!(Catalog::saved(&home).is_err());
        let repo = home.join("repo");
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&repo)
                .status()
                .unwrap()
                .success()
        );
        let mut catalog = Catalog {
            default: Some(Stack {
                id: "default".into(),
                name: "Global default".into(),
                url: "https://example.com".into(),
            }),
            projects: vec![Project {
                path: repo.clone(),
                stack: "default".into(),
                rag: false,
            }],
            ..Catalog::default()
        };
        catalog.tokens.insert("default".into(), "private".into());
        catalog.save(&home).unwrap();
        write_project(
            &home,
            &catalog.projects[0],
            catalog.default.as_ref().unwrap(),
            "private",
        )
        .unwrap();
        let before = fs::read(location(&home)).unwrap();
        let loaded = Catalog::saved(&home).unwrap();
        assert_eq!(loaded.tokens["default"], "private");
        assert_eq!(loaded.projects[0].path, repo);
        assert!(!loaded.projects[0].rag);
        assert_eq!(fs::read(location(&home)).unwrap(), before);
        assert!(repo.join(".opencode/token").exists());

        catalog.projects.push(catalog.projects[0].clone());
        catalog.save(&home).unwrap();
        assert!(Catalog::saved(&home).is_err());
        catalog.projects.pop();
        catalog.save(&home).unwrap();

        fs::remove_file(repo.join(".opencode/token")).unwrap();
        let central = token_file(&home, "default");
        fs::write(&central, "stale-legacy-token").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&central, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(Catalog::saved(&home).is_err());
        write_project(
            &home,
            &catalog.projects[0],
            catalog.default.as_ref().unwrap(),
            "private",
        )
        .unwrap();
        catalog.projects[0].path = home.join("missing-repository");
        catalog.save(&home).unwrap();
        assert!(Catalog::saved(&home).is_err());
        catalog.projects[0].path = repo;
        catalog.projects[0].rag = true;
        catalog.save(&home).unwrap();
        assert!(Catalog::saved(&home).unwrap().projects[0].rag);
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn catalog_refuses_duplicate_ids_and_unknown_assignments() {
        let home = ops::backup_path(&std::env::temp_dir().join("project-catalog-validate"));
        fs::create_dir(&home).unwrap();
        let mut catalog = Catalog {
            default: Some(Stack {
                id: "default".into(),
                name: "Global default".into(),
                url: "https://example.com".into(),
            }),
            ..Catalog::default()
        };
        catalog.tokens.insert("default".into(), "private".into());
        catalog.stacks.push(Stack {
            id: "default".into(),
            name: "Other".into(),
            url: "https://other.example.com".into(),
        });
        assert!(catalog.validate(&home).is_err());
        catalog.stacks.clear();
        catalog.projects.push(Project {
            path: home.join("repo"),
            stack: "absent".into(),
            rag: false,
        });
        assert!(catalog.validate(&home).is_err());
        assert!(!location(&home).exists());
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn old_stack_registry_imports_credentials_without_rewriting_legacy_files() {
        let home = ops::backup_path(&std::env::temp_dir().join("project-legacy-catalog"));
        let stack = home.join("old-stack");
        fs::create_dir_all(&stack).unwrap();
        let envrc = stack.join(".envrc");
        let legacy = "export OPENCODE_BASE_URL='https://example.com'\nexport OPENCODE_API_KEY='legacy-token'\n";
        fs::write(&envrc, legacy).unwrap();
        let registry = home.join(".config/foundry-opencode/stacks.json");
        fs::create_dir_all(registry.parent().unwrap()).unwrap();
        fs::write(&registry, serde_json::to_vec(&vec![stack]).unwrap()).unwrap();
        let catalog = Catalog::load(&home).unwrap();
        assert_eq!(catalog.stacks.len(), 1);
        assert_eq!(catalog.stacks[0].url, "https://example.com");
        assert_eq!(catalog.tokens[&catalog.stacks[0].id], "legacy-token");
        assert_eq!(fs::read_to_string(envrc).unwrap(), legacy);
        assert!(!location(&home).exists());
        fs::remove_dir_all(home).unwrap();
    }
}
