# CodexPilot

CodexPilot is a compact desktop controller for Codex provider switching.

It focuses on:

- switching Codex between official and third-party providers
- preserving the same `~/.codex` history/session surface
- managing Codex accounts, managed homes, and the live system account
- running a local proxy that maps Codex Responses traffic to supported provider APIs

## Development

```bash
pnpm install
pnpm dev
```

The renderer dev server runs on `http://127.0.0.1:1420`.

## Build

macOS:

```bash
pnpm build
```

Windows packages are built by the GitHub Actions workflow in
`.github/workflows/windows-build.yml`. Open the workflow in GitHub and run it
manually, then download the generated artifact.

## Verification

```bash
pnpm build:renderer
cargo test --manifest-path src-tauri/Cargo.toml
```
