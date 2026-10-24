# foundry-opencode

macOS Ratatui installer for Foundry OpenCode setup. Requires Homebrew, Git, and SSH access to the configs repository.

```sh
foundry-opencode          # install (same as install)
foundry-opencode install  # guided installation; optional OpenChamber
foundry-opencode update   # update Homebrew packages and configs
foundry-opencode init     # from your stack directory: create .envrc
foundry-opencode --dry-run install  # preview without changes
foundry-opencode init --dry-run     # preview .envrc without entering a token
foundry-opencode version  # print version
foundry-opencode usage    # print examples
```

During development, use `cargo run -- <command>` from this repository.

Release binaries are **not signed or notarized**. On macOS, compare the output of
`shasum -a 256 foundry-opencode-<arch>.tar.gz` with the matching entry in the
release's `SHA256SUMS`. If macOS blocks
the first launch, use **System Settings → Privacy & Security → Open Anyway**
only after confirming you trust the release; do not disable Gatekeeper.

`install` updates Homebrew, installs or upgrades OpenCode and direnv, and optionally
OpenChamber. It verifies the standard OpenCode formula is V2 before removing
confirmed V1 packages/executables or migrating the Homebrew `opencode-v2` formula.
Unrecognized installations stop the process for manual review. A non-Homebrew
`/Applications/OpenChamber.app` is moved to Trash only if OpenChamber was selected;
its application data is untouched. Confirmed global npm-owned V1 packages are
removed with `npm uninstall -g`; the installer never runs `npm install` for these
configs. After confirmation the installer clones the configs, then backs up any
existing `~/.config/opencode` and replaces it
with the new checkout. It then trusts the chosen projects directory in direnv
and creates `.envrc` files for selected stack directories. Broad direnv trust
means **all** `.envrc` files below the projects directory run without individual
approval; only use a directory you trust.
The installer offers to copy itself to `/usr/local/bin/foundry-opencode` after
setup. This may require an administrator password in the restored terminal;
existing binaries are never overwritten. You can decline this step.
Existing `.envrc` files are never overwritten. The installer adds
`eval "$(direnv hook zsh)"` to your active `.zshrc` if needed (respecting
`$ZDOTDIR`), or leaves an existing hook alone. Open a new Zsh shell to use it.

`update` upgrades the installed tools (OpenChamber only when present) and pulls
`main` with `--ff-only`; it refuses to pull over local config changes. `init`
prompts for a Foundry HTTPS base URL. At the token step, select **Open Foundry
token page** with Tab/Arrows and Enter (or click it) to open
`https://<host>/workspace/settings/tokens`,
or press Enter on **Enter an existing token** (the default). Paste the token
into the masked prompt; merely entering a host never opens the browser. It
creates `.envrc` with private permissions. If inside a Git repo, it offers to
ignore `.envrc` and refuses to create an unignored secret. Review the file, and run `direnv allow`
if it isn't under your whitelisted projects directory. Press Esc to cancel.

`--dry-run` works with `init`, `install` (including the no-argument default), and
`update`, before or after the command. It previews paths, commands, and redacted
credentials, then animates the real progress-bar layout with short, illustrative
times for each step. It does not change files, run installation commands,
request a token, or open a browser. The review shows whether `.zshrc` would be
updated or skipped.
The wizard supports mouse clicks as well as keyboard input; use Space to toggle
checkboxes, arrows to navigate stack choices, and Enter to continue. Type a
partial stack name to filter the list; Space immediately toggles the highlighted
match. Back keeps your previous choices. Instructions stay at the
bottom of each screen, with Continue on the right.
On the main projects directory page, existing folder suggestions appear as you
type `~/`, absolute, or relative paths. Use Up/Down to highlight one and Tab
or a mouse click to fill the field; Enter validates the resulting directory.

## License

Copyright (c) Demetry <demetryromanowski@gmail.com>

This project is licensed under the MIT license ([LICENSE] or <http://opensource.org/licenses/MIT>)

[LICENSE]: ./LICENSE
