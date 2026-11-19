# Foundry OpenCode GUI

Run `foundry-opencode-gui` from Terminal after [installing with Homebrew](../README.md#homebrew-macos).
For local development, use `cargo run --manifest-path foundry-opencode-gui/Cargo.toml`.

The wizard has Welcome, Applications, Foundry stacks, Repositories,
Assignments and RAG, Review, Progress, and Result screens. It remembers edits
when going back, uses the native folder picker, masks entered tokens, and never
claims installation succeeded without a result supplied by a backend.

Build the matching CLI first with `cargo build --manifest-path foundry-opencode/Cargo.toml`
when developing locally. The GUI enables Install/Update only when it finds a
matching headless CLI. For Homebrew installs, the GUI uses the CLI beside it
in the same Homebrew version directory; it does not replace the executable or
request administrator authorization. Standalone releases bundle the CLI in
`Contents/Resources` and request authorization for the final CLI replacement
only. Homebrew, Git, npm, and project setup run as the current user. The GUI
streams progress from the shared `foundry-opencode` library and shows backup
paths.

Apps launched from Finder include the standard Apple Silicon and Intel Homebrew
paths even without a shell profile. The review lists recognized legacy V1
packages and unmanaged executables; only reviewed items are migrated, and a
changed inventory stops installation before removal. The standalone GUI release is
unsigned and not notarized; check the release checksum before opening it.
