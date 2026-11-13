# Foundry OpenCode GUI

Run the UI with `cargo run --manifest-path foundry-opencode-gui/Cargo.toml`.

The wizard has Welcome, Applications, Foundry stacks, Repositories,
Assignments and RAG, Review, Progress, and Result screens. It remembers edits
when going back, uses the native folder picker, masks entered tokens, and never
claims installation succeeded without a result supplied by a backend.

Build the matching CLI first with `cargo build --manifest-path foundry-opencode/Cargo.toml`
when developing locally. The GUI enables Install/Update only when it finds a
matching headless CLI. Before changing files, it verifies the settings and
requests macOS administrator authorization once for the final CLI replacement;
Homebrew, Git, npm, and project files still run as the current user. The GUI
streams progress from the shared `foundry-opencode` library, preserves backup
paths, and installs the CLI last. Release bundles put the CLI in the app's
`Contents/Resources` directory.

Apps launched from Finder include the standard Apple Silicon and Intel Homebrew
paths even without a shell profile. The review lists recognized legacy V1
packages and unmanaged executables; only reviewed items are migrated, and a
changed inventory stops installation before removal. The GUI release is
unsigned and not notarized; check the release checksum before opening it.
