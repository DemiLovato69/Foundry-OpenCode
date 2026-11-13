# Foundry OpenCode GUI

Run the UI with `cargo run --manifest-path foundry-opencode-gui/Cargo.toml`.

The wizard has Welcome, Applications, Foundry stacks, Repositories,
Assignments and RAG, Review, Progress, and Result screens. It remembers edits
when going back, uses the native folder picker, masks entered tokens, and never
claims installation succeeded without a result supplied by a backend.

This is UI-only: `WizardApp::state` in `src/app.rs` is the integration point.
Supply `state.saved` for the update-defaults shortcut, set
`state.backend_connected` when an installer is ready, consume `state.request`
after the user acts, and call `state.apply_status(Status::Running { .. })` or
`Status::Finished { .. }` with progress and results. The final Install/Update
button and backend actions remain disabled until connected. The backend must
validate filesystem paths and credentials, avoid secrets in logs, and handle
backup, installation, cancellation, and retries.
