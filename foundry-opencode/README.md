# foundry-opencode

macOS Ratatui installer for Foundry OpenCode setup. Requires Homebrew, Git, and HTTPS access to the public configs repository; no GitHub SSH key is needed.

```sh
foundry-opencode          # install (same as install)
foundry-opencode install  # guided installation; optional OpenChamber
foundry-opencode update   # replace backed-up configs, update tools and migrate stacks
foundry-opencode uninstall # remove tools, configs and user data (irreversible)
foundry-opencode reset    # stop OpenCode/OpenChamber and clear OpenCode cache
foundry-opencode init     # from your stack directory: create per-stack config
foundry-opencode init-rag # from a project directory: install and index RAG
foundry-opencode init-rag --dry-run # preview RAG files and commands
foundry-opencode --dry-run install  # preview without changes
foundry-opencode init --dry-run     # preview config without entering a token
foundry-opencode version  # print version
foundry-opencode usage    # print examples
```

During development, use `cargo run -- <command>` from this repository.

Release binaries are **not signed or notarized**. On macOS, compare the output of
`shasum -a 256 foundry-opencode-<arch>.tar.gz` with the matching entry in the
release's `SHA256SUMS`. If macOS blocks
the first launch, use **System Settings → Privacy & Security → Open Anyway**
only after confirming you trust the release; do not disable Gatekeeper.

`install` updates Homebrew, installs or upgrades OpenCode, and optionally
OpenChamber. It verifies the standard OpenCode formula is V2 before removing
confirmed V1 packages/executables or migrating the Homebrew `opencode-v2` formula.
Unrecognized installations are reported, not deleted or used to block config setup. A non-Homebrew
`/Applications/OpenChamber.app` is moved to Trash only if OpenChamber was selected;
its application data is untouched. Confirmed global npm-owned V1 packages are
removed with `npm uninstall -g`; the installer never runs `npm install` for these
configs. After confirmation the installer clones the configs, then backs up any
existing `~/.config/opencode` and replaces it
with the new checkout. Choose any number of stack folders in the file explorer;
the selection is saved in `~/.config/foundry-opencode/stacks.json` for future updates.
Use Space to mark/unmark, Enter to open a folder, Left to go up, G to jump to
another directory, and C to continue. It creates `.opencode/opencode.json` and a private
`.opencode/token` in selected stack directories. Child projects inherit their
stack's providers and Foundry MCP connection. The token file is mode `0600`
and Git-ignored if the stack is in a repository. Optionally configure a private
global Foundry fallback for projects outside marked stacks; stack-local settings
override it. Existing direnv Zsh hooks, whitelist config and installations are removed.
The installer replaces `/usr/local/bin/foundry-opencode` after a successful
install or update, even if an older copy exists. This may require an administrator
password in the restored terminal. Installation commands show live output in the
installer's Terminal pane; Git credential prompts use the native terminal.
Install, update and `init` migrate `.envrc` in selected stack folders without
executing shell content. Unrecognized files prompt for credentials and a choice
to archive (private, Git-ignored) or delete them after verifying the new config.
Conflicting stack configs are backed up and replaced; unrelated JSON settings are
retained when possible. Tracked `.envrc` secrets still require Git cleanup and token rotation.

`update` upgrades the installed tools (OpenChamber only when present), clones
fresh configs from `main`, and backs up and replaces `~/.config/opencode` regardless
of its previous Git remote or local edits. It preserves the optional global
fallback. `init`
prompts for a Foundry HTTPS base URL. At the token step, select **Open Foundry
token page** with Tab/Arrows and Enter (or click it) to open
`https://<host>/workspace/settings/tokens`,
or press Enter on **Enter an existing token** (the default). Paste the token
into the masked prompt; merely entering a host never opens the browser. It
creates `.opencode/opencode.json` and a private `.opencode/token`. If inside a
Git repo, it Git-ignores the token and refuses a tracked or conflicting secret.
Press Esc to cancel.

`reset` asks for confirmation, stops the current user's OpenCode service and
OpenCode/OpenChamber processes, then deletes `~/.cache/opencode` if present.
It interrupts active sessions but preserves configs and the session database.
`reset --dry-run` previews the operation without stopping anything.

`uninstall` lets you mark additional legacy stack folders, then reviews the
registered stack folders and RAG projects before a
permanent wipe. It removes OpenCode/OpenChamber Homebrew packages, direnv,
the installer binary, registered stack credentials/configs, generated RAG files,
the global config and its backups, session data and caches. Project folders and
unrelated files inside them are preserved. Use `uninstall --dry-run` to review
paths without changing anything. Unregistered projects and programs installed
by other package managers cannot be discovered reliably; register them first
or remove them manually.

If OpenChamber is running, `install` and `update` skip its optional installation
or upgrade and continue with the required setup. Quit OpenChamber and rerun the
command to upgrade it.

`init-rag` runs separately from installation, from the root of **one project**.
Run `init` first from its stack directory, or configure the optional global
fallback, so its private token and host can be found without direnv.
It requires Node.js and npm. It creates `.opencode/rag.json`, pinned npm
dependencies, compatibility patches, a CLI helper, local ignores, and a
project-local V2 MCP server entry in `.opencode/opencode.json` (or an existing
`.opencode/opencode.jsonc`), preserving other settings and JSONC comments.
It refuses conflicting RAG files or MCP entries rather
than overwriting them. If an earlier attempt stopped after creating the
RAG files, rerun `init-rag` from the same project and Foundry host: it verifies
those files and the npm lockfile before resuming, without deleting an existing
index. Unrelated or edited files still require manual review. It installs the
dependencies and indexes the project with Foundry's `text-embedding-3-large`
endpoint; API credentials come from the stack's private token file.
Reindex after edits with
`node .opencode/rag-cli.mjs index` from the project root. The underlying RAG
package's automatic watcher is a V1 plugin and cannot run in OpenCode V2;
indexing is manual after the initial setup. The RAG MCP process reads the
same private stack token when it starts.

`--dry-run` works with `init`, `init-rag`, `install` (including the no-argument default),
`update`, `reset`, and `uninstall`, before or after the command. It previews paths, commands, and redacted
credentials; installation and update also animate the progress-bar layout with short,
illustrative times for each step. It does not change files, run installation commands,
request a token, or open a browser. The review lists stack migrations and skips.
The wizard supports mouse clicks as well as keyboard input. The file explorer
selects multiple stack directories anywhere accessible; no common parent is
required. Other text fields still offer existing directory suggestions.

## License

Copyright (c) Demetry <demetryromanowski@gmail.com>

This project is licensed under the MIT license ([LICENSE] or <http://opensource.org/licenses/MIT>)

[LICENSE]: ./LICENSE
