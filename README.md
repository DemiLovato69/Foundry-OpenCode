# OpenCode v2 Config

OpenCode configuration for Palantir. Requires OpenCode v2; The configuration uses the native v2 format and must not be loaded by v1.

# Install

## WARNING
The installer replaces `~/.config/opencode` only after confirmation, preserving the old directory as a timestamped backup. It identifies and removes confirmed V1 OpenCode executables/packages and non-Homebrew OpenCode binaries before installing Homebrew OpenCode V2. OpenChamber is optional: a non-Homebrew `/Applications/OpenChamber.app` is moved to Trash only when you choose to install the Homebrew cask. Existing sessions and OpenChamber data are not purged.

## Installation
Download the [latest macOS release](https://github.com/DemiLovato69/Foundry-OpenCode/releases/latest) for Apple Silicon (`aarch64-apple-darwin`) or Intel (`x86_64-apple-darwin`), extract the binary, and run it from the extracted location. The installer offers to copy itself into `/usr/local/bin` for future use. Homebrew, Git, and HTTPS access to this public repository are required; a GitHub SSH key is not. OpenChamber is optional.

**macOS security:** The release binaries are not signed or notarized. Check your downloaded archive with `shasum -a 256 foundry-opencode-<arch>.tar.gz` and compare it to the matching line in the release's `SHA256SUMS`. macOS may block the first launch; after attempting to run the binary, go to **System Settings → Privacy & Security → Open Anyway** only if you trust the release. Do not disable Gatekeeper or remove the quarantine attribute globally.

Run `foundry-opencode` (equivalent to `foundry-opencode install`). In the file explorer, mark stack folders anywhere on disk with Space, open folders with Enter, move up with Left, jump to a path with G, and continue with C. No central projects directory or direnv is required. Existing direnv Zsh hooks, whitelist settings and installation are removed.

Before uninstalling a legacy OpenCode installation, the installer verifies that the standard Homebrew `opencode` formula targets V2. Unrecognized executables are reported but not deleted or allowed to block config setup. A Homebrew-managed `opencode-v2` is migrated to the standard V2 formula after confirmation.

Use the mouse or keyboard in the wizard. Selected stack folders are saved for updates. Installing the CLI to `/usr/local/bin/foundry-opencode` may request an administrator password after closing the TUI.

You may optionally enter a global Foundry host/token as a fallback outside marked stacks. Each marked stack's `.opencode/opencode.json` overrides that fallback. Existing `.envrc` files are migrated without execution; if a file is unrecognized, enter its host/token and choose whether to archive or delete it after the new config is verified. Tracked `.envrc` secrets still require Git cleanup and token rotation.

## After Install

The installer provides many commands to make developer experience better.

- Generate per-stack OpenCode configuration
*Run this command from your stack directory*
`foundry-opencode init`

At the token prompt, select **Open Foundry token page** to open `https://<host>/workspace/settings/tokens`, create your token there, then paste it into the masked prompt. Opening the browser is optional and never happens just by entering the host.

- Update everything
`foundry-opencode update`

- Remove installed tools, configs, credentials and session data permanently
`foundry-opencode uninstall` (review first with `foundry-opencode uninstall --dry-run`)

- Re-run the install process
*This backs up and replaces previous configs after confirmation*
`foundry-opencode install`

- Show the binary version or example commands
`foundry-opencode version` / `foundry-opencode usage`

Preview operations without writing files or opening a browser: `foundry-opencode init --dry-run`, `foundry-opencode update --dry-run`, `foundry-opencode uninstall --dry-run`, or `foundry-opencode --dry-run install`. After review, install/update dry-runs animate illustrative progress.

Releases are created automatically from successful merges to `main` (including changes to these configs); each release contains macOS Apple Silicon and Intel binaries and checksums.


## Configuration layout

OpenCode loads `~/.config/opencode/opencode.jsonc` as a global base and each
stack's `.opencode/opencode.json` as a more specific override. The installer
stores stack credentials in private `.opencode/token` files, not in `.envrc`.
Update clones a fresh global config and backs up the old checkout, even when
its Git remote or working tree has changed. Uninstall removes registered
stack configs, generated RAG files and OpenCode/OpenChamber user data; it does
not delete project directories or discover unregistered stacks automatically.
