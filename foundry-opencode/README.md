# foundry-opencode

macOS Ratatui installer for Foundry OpenCode setup. Requires Homebrew, Git, and HTTPS access to the public configs repository; no GitHub SSH key is needed.

```sh
foundry-opencode          # install (same as install)
foundry-opencode install  # guided installation; optional OpenChamber
foundry-opencode update   # update Homebrew packages and configs
foundry-opencode reset    # stop OpenCode/OpenChamber and clear OpenCode cache
foundry-opencode init     # from your stack directory: create .envrc
foundry-opencode init-rag # from a project directory: install and index RAG
foundry-opencode init-rag --dry-run # preview RAG files and commands
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
The installer replaces `/usr/local/bin/foundry-opencode` after a successful
install or update, even if an older copy exists. This may require an administrator
password in the restored terminal. Installation commands show live output in the
installer's Terminal pane; Git credential prompts use the native terminal.
Existing `.envrc` files are never overwritten. The installer adds
`eval "$(direnv hook zsh)"` to your active `.zshrc` if needed (respecting
`$ZDOTDIR`), or leaves an existing hook alone. Open a new Zsh shell to use it.

`update` upgrades the installed tools (OpenChamber only when present) and pulls
`main` with `--ff-only`; it refuses to pull over local config changes. Existing
checkouts using the old repository URL (SSH or HTTPS) are switched to the new
HTTPS URL before pulling. `init`
prompts for a Foundry HTTPS base URL. At the token step, select **Open Foundry
token page** with Tab/Arrows and Enter (or click it) to open
`https://<host>/workspace/settings/tokens`,
or press Enter on **Enter an existing token** (the default). Paste the token
into the masked prompt; merely entering a host never opens the browser. It
creates `.envrc` with private permissions. If inside a Git repo, it offers to
ignore `.envrc` and refuses to create an unignored secret. Review the file, and run `direnv allow`
if it isn't under your whitelisted projects directory. Press Esc to cancel.

`reset` asks for confirmation, stops the current user's OpenCode service and
OpenCode/OpenChamber processes, then deletes `~/.cache/opencode` if present.
It interrupts active sessions but preserves configs and the session database.
`reset --dry-run` previews the operation without stopping anything.

If OpenChamber is running, `install` and `update` skip its optional installation
or upgrade and continue with the required setup. Quit OpenChamber and rerun the
command to upgrade it.

`init-rag` runs separately from installation, from the root of **one project**.
Load that project's stack `.envrc` first so `OPENCODE_BASE_URL` and
`OPENCODE_API_KEY` are available (or supply `FOUNDRY_TOKEN` for the token).
It requires Node.js and npm. It creates `.opencode/rag.json`, pinned npm
dependencies, compatibility patches, a CLI helper, local ignores, and a
project-local V2 MCP server entry in `.opencode/opencode.json`, preserving
other JSON settings. It refuses existing RAG files or a conflicting MCP entry
instead of overwriting them. It installs the dependencies and indexes the
project with Foundry's `text-embedding-3-large` endpoint; API credentials are
read from the environment, not written to disk. Reindex after edits with
`node .opencode/rag-cli.mjs index` from the project root. The underlying RAG
package's automatic watcher is a V1 plugin and cannot run in OpenCode V2;
indexing is manual after the initial setup. The OpenCode background service
also needs access to the same credentials to connect the MCP search server.

`--dry-run` works with `init`, `init-rag`, `install` (including the no-argument default), and
`update` and `reset`, before or after the command. It previews paths, commands, and redacted
credentials; installation and update also animate the progress-bar layout with short,
illustrative times for each step. It does not change files, run installation commands,
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
