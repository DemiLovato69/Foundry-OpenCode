use std::{fs, process::Command};

#[test]
fn reset_and_uninstall_preview_without_removing_user_data() {
    let home =
        foundry_opencode::ops::backup_path(&std::env::temp_dir().join("foundry-cli-dry-run"));
    let cache = home.join(".cache/opencode");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("keep"), "session data").unwrap();
    for action in ["reset", "uninstall"] {
        let output = Command::new(env!("CARGO_BIN_EXE_foundry-opencode"))
            .env("HOME", &home)
            .args([action, "--dry-run"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Would remove"));
        assert_eq!(
            fs::read_to_string(cache.join("keep")).unwrap(),
            "session data"
        );
    }
    fs::remove_dir_all(home).unwrap();
}
