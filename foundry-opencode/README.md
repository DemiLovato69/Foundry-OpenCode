# foundry-opencode CLI

The guided installer is `foundry-opencode-gui`. `foundry-opencode` is a plain,
non-interactive command-line utility: it does not open a terminal wizard.

```sh
foundry-opencode --help
foundry-opencode version
foundry-opencode update --dry-run
foundry-opencode update --yes
foundry-opencode update --yes --migrate-legacy  # only after reviewing update --dry-run
foundry-opencode init --stack default   # configure this folder (Git optional)
foundry-opencode init-rag
foundry-opencode exec --foundry-token -- npm install
foundry-opencode sync-worktrees --gen-rag
foundry-opencode add-worktree --gen-rag -- -b topic ../topic
foundry-opencode generate-rag --all
foundry-opencode repair-plugins --dry-run
foundry-opencode reset --dry-run
foundry-opencode uninstall --dry-run
```

Each configured folder uses a private `.opencode/token` and local
`.opencode/.gitignore`. A `FOUNDRY_TOKEN` passed through `exec --foundry-token`
is scoped only to that child process; an inherited value is replaced. The
command refuses a missing token or a config that still references a legacy
central token. It cannot change its parent shell's environment.

Install/update accepts saved catalog assignments and replaces existing global
and project config files with backups. Use the GUI to edit stacks, tokens and
RAG choices. `--dry-run` does not run package installs or write config files.
Destructive CLI commands require `--yes` after reviewing their dry-run output.
Uninstall removes only worktree token backups recorded by this release;
unrecorded recovery tokens are left in place and reported for manual review.

macOS GUI releases bundle a matching CLI. GUI authorization is requested before
the installation pipeline starts, and only the final CLI replacement uses
privileges. Do not run Homebrew or the GUI as root. Release binaries are not
signed or notarized unless signing credentials are configured in CI.
