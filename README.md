# OpenCode v2 Config

OpenCode configuration for Palantir. Requires OpenCode v2; The configuration uses the native v2 format and must not be loaded by v1.

# Install

## WARNING
The installer replaces `~/.config/opencode` only after confirmation, preserving the old directory as a timestamped backup. It identifies and removes confirmed V1 OpenCode executables/packages and non-Homebrew OpenCode binaries before installing Homebrew OpenCode V2. OpenChamber is optional: a non-Homebrew `/Applications/OpenChamber.app` is moved to Trash only when you choose to install the Homebrew cask. Existing sessions and OpenChamber data are not purged.

## Installation
Download the [latest macOS release](https://github.com/DemiLovato69/Foundry-OpenCode/releases/latest) for Apple Silicon (`aarch64-apple-darwin`) or Intel (`x86_64-apple-darwin`), extract the binary, and run it from the extracted location. The installer offers to copy itself into `/usr/local/bin` for future use. Homebrew, Git, and HTTPS access to this public repository are required; a GitHub SSH key is not. OpenChamber is optional.

**macOS security:** The release binaries are not signed or notarized. Check your downloaded archive with `shasum -a 256 foundry-opencode-<arch>.tar.gz` and compare it to the matching line in the release's `SHA256SUMS`. macOS may block the first launch; after attempting to run the binary, go to **System Settings → Privacy & Security → Open Anyway** only if you trust the release. Do not disable Gatekeeper or remove the quarantine attribute globally.

Run `foundry-opencode` (equivalent to `foundry-opencode install`). It will walk you through the entire installation setup. The selected projects directory is broadly whitelisted by direnv, so any `.envrc` within it can execute automatically. The installer adds `eval "$(direnv hook zsh)"` to your active `.zshrc` if it is not already present (using `$ZDOTDIR/.zshrc` when configured). Open a new Zsh shell afterward to activate the hook.

Before uninstalling any OpenCode installation, the installer verifies that the standard Homebrew `opencode` formula targets V2. Its review and dry-run list every package and executable it proposes to remove. An unrecognized executable blocks installation rather than risking a wrong deletion. A Homebrew-managed `opencode-v2` is migrated to the standard V2 formula after confirmation.

Use the mouse or keyboard in the wizard: Space toggles checkboxes, arrows navigate stacks, and Enter activates the focused button. Type to filter stack names; Space immediately selects the highlighted match, and choices stay checked when the filter changes. Cancel is on the first page, Back preserves your answers, and Continue is on the right. On first installation the wizard offers to copy itself to `/usr/local/bin/foundry-opencode` so it can be run from anywhere. It will ask before doing so, may request an administrator password after closing the TUI, and will not overwrite an existing binary.

When entering the main projects directory, suggestions show existing folders as you type. Use Up/Down to choose a suggestion and Tab or a mouse click to fill the field; Enter continues after validating the directory. For the recommended layout, choose the parent `~/Projects`, not `~/Projects/Stack1`.

## After Install

The installer provides many commands to make developer experience better.

- Generate .envrc
*Run this command from your stack directory*
`foundry-opencode init`

At the token prompt, select **Open Foundry token page** to open `https://<host>/workspace/settings/tokens`, create your token there, then paste it into the masked prompt. Opening the browser is optional and never happens just by entering the host.

- Update everything
`foundry-opencode update`

- Re-run the install process
*This backs up and replaces previous configs after confirmation*
`foundry-opencode install`

- Show the binary version or example commands
`foundry-opencode version` / `foundry-opencode usage`

Preview any operation without writing files, opening a browser, or requesting a token: `foundry-opencode init --dry-run`, `foundry-opencode update --dry-run`, or `foundry-opencode --dry-run install` (also `foundry-opencode --dry-run` for the default install). After review, dry-run animates a mock progress bar with illustrative times for each planned step; it never performs those steps.

Releases are created automatically from successful merges to `main` (including changes to these configs); each release contains macOS Apple Silicon and Intel binaries and checksums.


# Futher Reading
<details>
<summary>
Expand the section for more information of what happens behind the scenes.
</summary>

## Project Setup

### Directories

Its recommended that projects are stored under a centeralized directory on your system.

For example
`$HOME/Projects/`

For the cleanest results use:

```text
$HOME/Projects/
├── Stack1/
│   ├── .envrc                   # Stack1 host and API key
│   ├── Functions Repo/
│   └── OSDK React App Repo/
├── Stack2/
│   ├── .envrc                   # Stack2 host and API key
│   ├── Functions Repo/
│   ├── Pipeline Repo/
│   └── OSDK React App Repo/
└── ...
```

With that directory topology whenever you switch between stacks your system will always load the most appropriate envirnment variables for that stack.

This is not required but provides the best results.

### Direnv

#### !IMPORTANT!
If you don't follow the recomended topology and put your .envrc in Git tracked repositories, absolutely ensure that you do not commit that file. It contains your secrets.

If you work on multiple stacks direnv is required to swap between workspaces.
*It is recommended to use this direnv setup even if you work on a singlular stack*

direnv allows the hotswapping of environment variables and automatically restarting the opencode service.
*In Opencode-v2 a background service is started which loads the last known environment variables for the host and api key*

Each stack directory will have a `.envrc` file. This file will contain:
```sh
export OPENCODE_API_KEY=<YOUR_FOUNDRY_TOKEN>
export OPENCODE_BASE_URL=https://xyz.palantirfoundry.com
opencode service restart
```

The `.envrc` file can be generated using the installer command.
</details>
