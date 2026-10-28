# OpenCode v2 Config

OpenCode configuration for Palantir. Requires OpenCode v2; The configuration uses the native v2 format and must not be loaded by v1.

# Install

## WARNING
The installer replaces `~/.config/opencode` only after confirmation, preserving the old directory as a timestamped backup. It identifies and removes confirmed V1 OpenCode executables/packages and non-Homebrew OpenCode binaries before installing Homebrew OpenCode V2. OpenChamber is optional: a non-Homebrew `/Applications/OpenChamber.app` is moved to Trash only when you choose to install the Homebrew cask. Existing sessions and OpenChamber data are not purged.

## Installation
Download the [latest macOS release](https://github.com/DemiLovato69/Foundry-OpenCode/releases/latest) for Apple Silicon (`aarch64-apple-darwin`) or Intel (`x86_64-apple-darwin`), extract the binary, and run it from the extracted location. The installer offers to copy itself into `/usr/local/bin` for future use. Homebrew, Git, and HTTPS access to this public repository are required; a GitHub SSH key is not. OpenChamber is optional.

**macOS security:** The release binaries are not signed or notarized. Check your downloaded archive with `shasum -a 256 foundry-opencode-<arch>.tar.gz` and compare it to the matching line in the release's `SHA256SUMS`. macOS may block the first launch; after attempting to run the binary, go to **System Settings → Privacy & Security → Open Anyway** only if you trust the release. Do not disable Gatekeeper or remove the quarantine attribute globally.

Run `foundry-opencode` (equivalent to `foundry-opencode install`). The explorer is a single expandable directory tree. Arrows or a single click highlight without opening; Enter expands or collapses, Right expands, Left collapses or selects the parent, Space marks a folder, and C continues. Tab switches between the tree and its search/path field. Press S to filter visible directory names, or `/` to enter an absolute path; `~/`, `./`, and `../` paths also work. Path matches appear in the same tree; Enter opens a complete or arrow-highlighted path. Dot-prefixed and macOS Finder-invisible folders are hidden by default; type their full path to open them. Previously marked folders remain marked even when hidden. Inaccessible directories show an error without losing selections. No central projects directory or direnv is required. Existing direnv Zsh hooks, whitelist settings and installation are removed.

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

The update wizard can optionally change the global Foundry fallback and each selected
stack's host and token independently. Leave a scope unchecked to retain its credentials;
changed stack configs and private tokens are backed up. Child projects inherit their
stack's credentials, but still use the globally configured default model unless the
 stack specifies one. If Foundry rejects a model with `INVALID_ARGUMENT`, select a model
available on that stack or set a stack-local `model` in `.opencode/opencode.json`.

Update downloads the latest published macOS installer binary for your CPU, checks its release SHA-256 and version, and replaces `/usr/local/bin/foundry-opencode` after the wizard closes. Download or verification failures leave the existing binary untouched. The global config is cloned independently from `main`.

Palantir MCP runs `npx` with a command-local `--registry=https://registry.npmjs.org` so project `.npmrc` files can continue to use their Foundry registry. Install/update generate this launcher for new stacks and the optional global fallback; they back up and upgrade only verified older stack launchers. Custom MCP commands and stack tokens are not changed. After deployment, use `/mcps` to reconnect `palantir` and verify a tool call from the affected repo.

Host, token and path fields support mouse cursor positioning, arrows, Home/End, Delete/Backspace, and Ctrl+U or terminal-translated Cmd+Delete to delete to the beginning. Some terminals intercept Cmd shortcuts; use Ctrl+U there. Tokens stay masked.

If an older project reports `.opencode/plugins/rag-plugin.js` is not a V2 plugin, run `foundry-opencode repair-plugins` from that project (or `--dry-run` first). It backs up only the exact known V1 wrapper without removing the RAG index; edited plugins are left alone. Reindex manually with `node .opencode/rag-cli.mjs index` where that helper exists. If it is missing, review the project config and run `init-rag` to set up V2 RAG MCP support.
The incompatible `context-mode` plugin is disabled by the global config. A missing `palantir-mcp` executable under npm's cache is a separate npm/MCP installation failure; inspect `opencode mcp list` and repair that package's installation rather than deleting OpenCode sessions or configuration.

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
