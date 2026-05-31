# Release

This file records the v1.0.0 release policy. The tag/release action still needs
an explicit human checkpoint before it runs.

## Version Policy

- One workspace version is authoritative: `workspace.package.version`.
- Every package inherits that version with `version.workspace = true`.
- v1.0.0 is `1.0.0` across the workspace.
- No crate is published to crates.io for v1.0.0. All packages inherit
  `publish.workspace = true`, and the workspace sets `publish = false`.

The crates stay path-internal because the workspace still exposes internal
kernel/runtime boundaries and path dependencies. The v1.0.0 distribution is the
GitHub Release CLI binary asset set.

## Release Assets

The release workflow builds `verbreel-cli` as the `verbreel` binary with
`--features native-render`. Assets are tarballs named:

```text
verbreel-v1.0.0-<os>-<arch>.tar.gz
verbreel-v1.0.0-<os>-<arch>.tar.gz.sha256
```

The native-render binary links against system FFmpeg through rsmpeg. Operators
need compatible FFmpeg runtime libraries on the target host.

## Pre-Tag Verification

Run these from a clean checkout of `main` before creating the annotated tag:

```bash
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p verbreel-conformance --release
cargo run -p examples --features native-render
```

The native render smoke requires FFmpeg development libraries and a software
Vulkan path such as lavapipe on Linux.

## Tag And Publish

Only after the explicit release checkpoint:

```bash
git tag -a v1.0.0 -m "Verbreel Engine v1.0.0"
git push origin v1.0.0
```

The tag-triggered workflow verifies that the pushed tag is annotated, reruns the
release gates, builds CLI assets, and publishes the GitHub Release with generated
notes.
