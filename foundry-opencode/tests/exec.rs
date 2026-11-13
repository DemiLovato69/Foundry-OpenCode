use std::{fs, process::Command};

#[test]
fn npm_child_uses_the_nearest_private_token_and_never_inherited_value() {
    let root = foundry_opencode::ops::backup_path(&std::env::temp_dir().join("foundry-token-exec"));
    fs::create_dir_all(root.join(".opencode")).unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(
        root.join(".opencode/opencode.json"),
        r#"{"providers":{"palantir-openai":{"settings":{"apiKey":"{file:./token}"}}}}"#,
    )
    .unwrap();
    let token = root.join(".opencode/token");
    fs::write(&token, "private").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let binary = env!("CARGO_BIN_EXE_foundry-opencode");
    assert!(
        Command::new(binary)
            .current_dir(root.join("nested"))
            .env("FOUNDRY_TOKEN", "stale")
            .args([
                "exec",
                "--foundry-token",
                "--",
                "sh",
                "-c",
                "test \"$FOUNDRY_TOKEN\" = private"
            ])
            .status()
            .unwrap()
            .success()
    );
    fs::remove_file(&token).unwrap();
    let output = Command::new(binary)
        .current_dir(root.join("nested"))
        .env("FOUNDRY_TOKEN", "stale")
        .args(["exec", "--foundry-token", "--", "sh", "-c", "touch marker"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("private"));
    assert!(!root.join("nested/marker").exists());
    fs::remove_dir_all(root).unwrap();
}
