# Workflows

| File | Purpose | Triggers |
|---|---|---|
| `ci.yml` | cargo check / test / fmt / clippy on Linux + macOS, plus wasm32 check on Linux | push to main, PR to main |

MSRV pinned in `Cargo.toml` `workspace.package.rust-version`. Update both places
when bumping MSRV.
