# OpenCode v2 Config

OpenCode configuration for Palantir. Requires OpenCode v2; The configuration uses the native v2 format and must not be loaded by v1.

# Install

## WARNING
The GUI installer backs up and replaces `~/.config/opencode` and managed config files in each selected folder. It does not delete unrelated `.opencode` files or folder contents. OpenChamber is optional; existing sessions are preserved.

## Installation
Download the matching [GUI macOS release](https://github.com/DemiLovato69/Foundry-OpenCode/releases/latest) for Apple Silicon (`aarch64-apple-darwin`) or Intel (`x86_64-apple-darwin`) and open `FoundryOpenCode.app`. Its matching plain CLI is bundled and installed last to `/usr/local/bin/foundry-opencode`. Homebrew, Git and HTTPS access to the public config repository are required.

**macOS security:** The release binaries are not signed or notarized. Check your downloaded GUI archive with `shasum -a 256 foundry-opencode-gui-<arch>.tar.gz` and compare it to the matching line in the release's `SHA256SUMS`. macOS may block the first launch; go to **System Settings → Privacy & Security → Open Anyway** only if you trust the release. Do not disable Gatekeeper or remove the quarantine attribute globally.

The GUI accepts any existing folder, including one outside a Git repository. It collects Foundry stacks, optional OpenChamber, folder assignments, and per-folder RAG choices. It requests administrator authorization before making installation changes, while Homebrew and project setup run as your ordinary account.

The installer verifies that the standard Homebrew `opencode` formula targets V2. The GUI reviews any recognized legacy V1 or `opencode-v2` packages before migrating them as the ordinary user. CLI users must first run `update --dry-run`, then explicitly pass `--migrate-legacy` with `update --yes` if removal is needed. Unrecognized executables are left untouched.

Use the mouse or keyboard in the GUI to assign a stack and optionally RAG to each folder. Those choices are saved for updates. Privilege is used only for the final CLI replacement.

The optional global default also provides a fallback outside selected folders. Each selected folder gets its own `.opencode/opencode.json` referring to its private `.opencode/token` (mode `0600`), plus `.opencode/.gitignore` to ignore generated files even when Git is initialized later. Known old stack credentials can be imported without executing `.envrc`. Legacy parent-stack configs and `.envrc` files are left in place for review: they may still affect unselected repositories. Tracked secrets require Git cleanup and token rotation.

## After Install

The installer provides many commands to make developer experience better.

- Configure one folder (Git is optional)
*Run this command from the selected folder*
`foundry-opencode init`

- Pass the current folder's private token only to an npm subprocess
`foundry-opencode exec --foundry-token -- npm install`

The GUI offers **Open token page** for `https://<host>/workspace/settings/tokens`; opening the browser is optional and never happens just by entering the host.

- Update everything
`foundry-opencode update --yes` (or use the GUI to change assignments)

The GUI reopens saved stacks and folder assignments. A selected folder config is replaced with a backup; unrelated `.opencode` files are preserved. The global default model applies unless a project overrides it.

To keep all saved stacks, folder assignments and RAG choices, check **Accept loaded defaults and update** in the GUI or run `foundry-opencode update -y`. `update -y --dry-run` previews without writing. Missing or invalid saved settings stop the CLI instead of silently changing the selection.

The GUI release bundles a matching CLI. A CLI update downloads and verifies the latest CLI release; both paths replace global configs from `main` and install the CLI only after project setup completes.

Palantir MCP runs `npx` with a command-local `--registry=https://registry.npmjs.org` so project `.npmrc` files can continue to use their Foundry registry. Install/update generate this launcher in the per-project configs and optional global fallback. After deployment, use `/mcps` to reconnect `palantir` and verify a tool call from the affected repo.

If Anthropic fails with `INVALID_ARGUMENT` and `unrecognizedProperty=cache_control`, update the installed config: the compatibility plugin removes unsupported prompt-cache markers from Foundry-bound requests. Use `/reload` if needed, then retry; switching Anthropic models does not address this error.

Tokens remain masked in the GUI; CLI review and progress redact them.

If an older project reports `.opencode/plugins/rag-plugin.js` is not a V2 plugin, run `foundry-opencode repair-plugins` from that project (or `--dry-run` first). It backs up only the exact known V1 wrapper without removing the RAG index; edited plugins are left alone. Reindex manually with `node .opencode/rag-cli.mjs index` where that helper exists. If it is missing, review the project config and run `init-rag` to set up V2 RAG MCP support.
The incompatible `context-mode` plugin is disabled by the global config. A missing `palantir-mcp` executable under npm's cache is a separate npm/MCP installation failure; inspect `opencode mcp list` and repair that package's installation rather than deleting OpenCode sessions or configuration.

- Work in Git worktrees
*Run these from any checkout of the repository*
`foundry-opencode add-worktree [--gen-rag] <git worktree add options>` creates a worktree and links its shared config
`foundry-opencode sync-worktrees [--gen-rag]` moves every worktree to the latest default branch
`foundry-opencode generate-rag [--all]` (re)generates RAG for this checkout, or every checkout

Git worktree commands configure the main checkout and link shared dependencies and scripts. The config, token, and `.gitignore` are copied locally into a worktree so `{file:./token}` resolves there; RAG indexes (`rag_db`), logs and backups remain per-worktree. Ordinary non-Git folders are not processed as worktrees.

`.opencode` and `.openchamber` hold one machine's setup and must never be committed, except `.opencode/commands`, which teams share through the repository. Install/update add `.opencode/*`, `!.opencode/commands/` and `.openchamber` to each selected repository's `.gitignore` (commit that change) and to its local, never-pushed `.git/info/exclude`, which applies in every worktree and branch right away. A rule ignoring `.opencode` as a whole would hide `commands` for good, so such a line is replaced, including the block earlier releases added. Worktrees never link `commands`: each has its own checked-out copy, and edits to it count as uncommitted work for `sync-worktrees`. Other files a repository already commits under `.opencode` or `.openchamber` are left in place and reported with the `git rm -r --cached` command to remove them; uninstall removes the exclude entry but keeps the `.gitignore` lines.

`sync-worktrees` fetches the remote the main checkout's branch tracks (or the only remote, or `origin`), finds its default branch, then for every worktree: moves it to the latest default branch (fast-forwards the checkout on that branch, moves detached ones) and links any missing shared `.opencode` entries. Worktrees on another branch, with uncommitted changes, or with unpushed commits on the default branch are reported as `FAILED` and not moved, but are still linked. With `--gen-rag` it also (re)indexes every checkout.

`add-worktree` runs `git worktree add` with exactly the options you give it (`-b`, `--detach`, paths, and so on), adds the local `.git/info/exclude` entry if it is missing, and links the new worktree's `.opencode`; with `--gen-rag` it also builds the new worktree's RAG index. `generate-rag` (re)indexes only the checkout you are in; `--all` does every checkout of the repository without moving any. `--gen-rag` and `--dry-run` are recognised anywhere among the `add-worktree` options.

Indexing is incremental (output in `.opencode/sync-index.log`). A worktree without an index starts from a copy of the main checkout's index, re-pointed at the worktree, so only files that differ are embedded: seconds instead of minutes. Seeding needs the pinned RAG plugin version and an idle main index; otherwise it falls back to a full build. Every command exits non-zero if anything failed, and `--dry-run` previews without fetching or changing anything.

Install/update can also add the opt-in Git alias `git sync-worktrees` for the same command; `git sync-worktrees --gen-rag` passes the option through. Uninstall removes known worktree links and indexes, and removes the alias if it is still the installer's. Unrecorded token backups remain for manual review.

New RAG setups also skip `.nx`, `.palantir`, `osdk-output` and `generated-imports`, which hold build caches, local ontology state and generated SDKs, and send 4 embedding requests at a time (about 1.7x faster than 2). Update upgrades an earlier installer `rag.json` in place with a backup and reindexes, keeping RAG enabled even if that reindex fails; a hand-edited one is left alone.

- Remove installed tools, configs, credentials and session data permanently
`foundry-opencode uninstall --yes` (review first with `foundry-opencode uninstall --dry-run`)

- Re-run the install process
*This backs up and replaces previous configs after confirmation*
`foundry-opencode install --yes` (or launch the GUI)

- Show the binary version or example commands
`foundry-opencode version` / `foundry-opencode usage`

Preview operations without writing files or opening a browser: `foundry-opencode init --dry-run`, `foundry-opencode update --dry-run`, `foundry-opencode uninstall --dry-run`, or `foundry-opencode --dry-run install`.

Releases are created automatically from successful merges to `main` (including changes to these configs); each release contains macOS Apple Silicon and Intel binaries and checksums.


## Configuration layout

OpenCode loads `~/.config/opencode/opencode.jsonc` as a global base and each
selected folder's `.opencode/opencode.json` as a more specific override. The
folder's private `.opencode/token` (mode `0600`) is referenced by `{file:./token}`;
`.opencode/.gitignore` ignores local setup files even when Git is initialized later.
Legacy central tokens are only migration sources, not active project references.

New model selectors: `palantir-openai/gpt-6-1-sol`, `palantir-google/gemini-4-argon`,
and `palantir-google/gemini-3-8-flash`. Gemini uses Foundry's beta Google proxy;
your enrollment must enable the models. Its streaming requests require `alt=sse`
and Foundry bearer authentication, not a Google API key header.
