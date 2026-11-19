# Maintaining the Foundry OpenCode tap

The source formula template lives in `foundry-opencode.rb.in`. CI renders it with a local source archive and tests it on Apple Silicon and Intel; the release job renders it with a tagged source archive for users. The formula only builds and installs the two Rust binaries. Initial configuration is a separate, interactive `foundry-opencode-gui` run.

## One-time setup

1. Create `DemiLovato69/homebrew-tap` with a default branch and a README linking users to the [installation instructions](https://github.com/DemiLovato69/Foundry-OpenCode#homebrew-macos). Reserve `automation/foundry-opencode` for the release job.
2. Add `HOMEBREW_TAP_TOKEN` as an Actions secret in `Foundry-OpenCode`: use a credential limited to the tap repository with Contents and Pull requests write access.
3. After the first release containing Homebrew support, review and merge the generated PR in the tap. Standalone releases still publish when the token is not configured.

Before announcing the tap, test the GUI from a normal Terminal on a clean Mac and check that a Brew upgrade retains an existing user's saved settings. Separately, `brew uninstall` should leave configuration, tokens, and sessions intact.
