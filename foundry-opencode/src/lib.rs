pub mod cleanup;
pub mod installer;
pub mod ops;
pub mod projects;
pub mod rag;
pub mod stack;
pub mod worktrees;

use std::path::{Path, PathBuf};

pub fn load_stacks(home: &Path) -> color_eyre::eyre::Result<Vec<PathBuf>> {
    let path = home.join(".config/foundry-opencode/stacks.json");
    match std::fs::read(&path) {
        Ok(data) => Ok(serde_json::from_slice(&data)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}
