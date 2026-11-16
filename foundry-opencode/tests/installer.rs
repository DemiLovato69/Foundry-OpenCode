#![cfg(target_os = "macos")]

use foundry_opencode::{cleanup, installer, ops, projects};
use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn reviewed_legacy_migration_and_project_writes_finish_before_cli_copy() {
    let home = ops::backup_path(&std::env::temp_dir().join("installer-sandbox"));
    let bin = home.join("bin");
    let state = home.join("state");
    let folder = home.join("ordinary-folder");
    for path in [&bin, &state, &folder, &home.join(".config/opencode")] {
        fs::create_dir_all(path).unwrap();
    }
    fs::write(state.join("v1"), "").unwrap();
    fs::write(state.join("npm1"), "").unwrap();
    fs::write(home.join(".config/opencode/old-config"), "keep as backup").unwrap();
    executable(&bin.join("opencode"), "#!/bin/sh\necho 1.0.0\n");
    executable(
        &bin.join("brew"),
        r#"#!/bin/sh
case "$*" in
  --version) echo Homebrew ;;
  --prefix) echo /no-brew-prefix ;;
  info*) echo '{"formulae":[{"versions":{"stable":"2.1.0"}}]}' ;;
  list*opencode-v1) test -e "$FAKE_STATE/v1" && echo 'opencode-v1 1.0.0' ;;
  list*opencode-v2) exit 1 ;;
  list*opencode) test -e "$FAKE_STATE/v2" && echo 'opencode 2.1.0' ;;
  uninstall*opencode-v1) rm "$FAKE_STATE/v1" ;;
  install*opencode|upgrade*opencode) touch "$FAKE_STATE/v2" ;;
  update) : ;;
  *) exit 1 ;;
esac
"#,
    );
    executable(
        &bin.join("npm"),
        r#"#!/bin/sh
case "$*" in
  'root -g') echo /no-npm-root ;;
  'ls -g --depth=0 --json') if test -e "$FAKE_STATE/npm1"; then echo '{"dependencies":{"opencode-ai":{"version":"1.0.0"}}}'; else echo '{"dependencies":{}}'; fi ;;
  'uninstall -g opencode-ai') rm "$FAKE_STATE/npm1" ;;
  *) exit 1 ;;
esac
"#,
    );
    executable(
        &bin.join("git"),
        r#"#!/bin/sh
if test "$1" = clone; then
  for path do :; done
  mkdir -p "$path/.git/info"
else
  exit 1
fi
"#,
    );
    let old_path = std::env::var_os("PATH").unwrap();
    unsafe {
        std::env::set_var(
            "PATH",
            std::env::join_paths([
                bin.as_os_str(),
                Path::new("/usr/bin").as_os_str(),
                Path::new("/bin").as_os_str(),
            ])
            .unwrap(),
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("FAKE_STATE", &state);
    }
    let mut catalog = projects::Catalog::default();
    catalog.stacks.push(projects::Stack {
        id: "test".into(),
        name: "Test".into(),
        url: "https://example.com".into(),
    });
    catalog.tokens = BTreeMap::from([("test".into(), "synthetic-token".into())]);
    catalog.projects.push(projects::Project {
        path: folder.clone(),
        stack: "test".into(),
        rag: false,
    });
    projects::write_project(
        &home,
        &catalog.projects[0],
        &catalog.stacks[0],
        "synthetic-token",
    )
    .unwrap();
    catalog.save(&home).unwrap();
    let cli = env!("CARGO_BIN_EXE_foundry-opencode");
    let preview = Command::new(cli)
        .args(["update", "--dry-run"])
        .output()
        .unwrap();
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stderr)
    );
    assert!(
        String::from_utf8_lossy(&preview.stdout)
            .contains("Would uninstall legacy Homebrew opencode-v1")
    );
    assert!(
        !Command::new(cli)
            .args(["update", "--yes"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(state.join("v1").exists() && state.join("npm1").exists());
    let reviewed = cleanup::inspect(&home, false).unwrap();
    assert!(reviewed.migration_needed());
    assert_eq!(reviewed.brew_v1, ["opencode-v1"]);
    assert_eq!(reviewed.npm_v1, ["opencode-ai"]);
    assert!(reviewed.files.contains(&bin.join("opencode")));
    let plan = installer::Plan {
        home: home.clone(),
        catalog,
        cli: env!("CARGO_BIN_EXE_foundry-opencode").into(),
        openchamber: false,
        dry_run: false,
        reviewed,
    };
    let mut checkpoint = installer::Checkpoint::default();
    let mut copied = false;
    let first = installer::apply(
        &plan,
        &mut checkpoint,
        |_| {},
        || false,
        |_| {
            assert!(!state.join("v1").exists() && !state.join("npm1").exists());
            assert!(!bin.join("opencode").exists());
            assert!(
                ops::global_credentials(&home.join(".config/opencode"))
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                projects::private_token(&folder.join(".opencode/token")).unwrap(),
                "synthetic-token"
            );
            assert!(
                fs::read_to_string(folder.join(".opencode/.gitignore"))
                    .unwrap()
                    .starts_with("*\n")
            );
            copied = true;
            Err(color_eyre::eyre::eyre!("synthetic final copy failure"))
        },
    );
    assert!(first.is_err() && copied);
    let backups = fs::read_dir(home.join(".config"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("opencode.backup-")
        })
        .count();
    let result = installer::apply(&plan, &mut checkpoint, |_| {}, || false, |_| Ok(())).unwrap();
    assert!(
        copied
            && result
                .iter()
                .any(|line| line == "CLI installed as final step")
    );
    assert!(fs::read_dir(home.join(".config")).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("opencode.backup-")
    }));
    assert_eq!(
        fs::read_dir(home.join(".config"))
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("opencode.backup-"))
            .count(),
        backups
    );
    assert!(fs::read_dir(home.join(".Trash")).unwrap().next().is_some());
    unsafe {
        std::env::set_var("PATH", old_path);
        std::env::remove_var("FAKE_STATE");
    }
    fs::remove_dir_all(home).unwrap();
}
