# Contributing to Konnect

Thanks for your interest! Bug reports, feature requests, and pull requests are welcome.

## Before you start

- Check [ROADMAP.md](ROADMAP.md) — your idea may already be planned (or intentionally
  out of scope).
- For anything non-trivial, open an issue first so we can agree on the approach before
  you invest time.

## Development setup

```bash
# protoc is required for protobuf code generation (kicad-ipc crate)
# Windows: choco install protoc   /   macOS: brew install protobuf   /   Linux: apt install protobuf-compiler

cargo check --workspace
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo build --locked --release -p konnect

# The viewer is outside the workspace and must be checked separately
cargo test --locked --manifest-path crates/schematic-viewer/Cargo.toml
cargo build --locked --release --manifest-path crates/schematic-viewer/Cargo.toml
```

See [DEV.md](DEV.md) for the architecture guide, tool conventions, and how to add a
new tool.

## Pull request checklist

- `cargo test --locked --workspace --all-targets` passes
- `cargo clippy --locked --workspace --all-targets -- -D warnings` is clean
- `cargo fmt --all -- --check` passes
- If the change affects the viewer: its standalone tests and release build pass
- If the change affects Linux discovery, IPC, packaging, or system dependencies:
  the Linux parity and real-KiCAD E2E workflows pass
- If you added or removed tools: update `tool_count` in `router/registry.rs`,
  regenerate the matching section of `tool-directory.md`, and run the registry /
  documentation invariant tests

## Contributor License Agreement

Konnect is dual-licensed: AGPL-3.0 for the community, with commercial licenses
available for organizations that can't comply with the AGPL (see
[COMMERCIAL.md](COMMERCIAL.md)). To make that possible, the project must be able to
relicense contributed code.

By submitting a contribution, you agree that:

1. You have the right to submit the work under the project's licenses.
2. You grant the project maintainer a perpetual, worldwide, non-exclusive,
   royalty-free, irrevocable license to use, reproduce, modify, distribute, and
   sublicense your contribution — including under licenses other than the AGPL.
3. Your contribution remains available to the community under the AGPL-3.0.

If you can't agree to these terms, please open an issue describing the change
instead of a pull request — reimplementations from descriptions are fine.
