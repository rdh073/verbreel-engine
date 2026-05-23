# Workflows

| File | Purpose | Triggers |
|---|---|---|
| `ci.yml` | cargo check / test / fmt / clippy on Linux + macOS | push to main, PR to main |

MSRV pinned in `Cargo.toml` `workspace.package.rust-version`. Update both places
when bumping MSRV.
