# Workflows

| File | Purpose | Triggers |
|---|---|---|
| `ci.yml` | cargo check / test / fmt / clippy on Linux + macOS, plus wasm32 check on Linux | push to main, PR to main |
| `release.yml` | verifies the release gate, runs native render smoke, packages CLI native-render binaries, and publishes a GitHub Release | annotated `v*.*.*` tag push |
| `board-sync.yml` | projects PR/issue lifecycle onto the project board Status field | PR lifecycle, issue assignment |
| `claude.yml` | Claude auto-review and trusted interactive Claude Code Action | PR lifecycle, trusted `@claude` comments |

MSRV pinned in `Cargo.toml` `workspace.package.rust-version`. Update both places
when bumping MSRV.
