# Gossia

A spreadsheet like application which handles JSON Schema and XML Schema documents. Each sheet is a tree defined by a schema instead of a 2D grid, and cells can hold Python formulas whose results must match the schema.

**The name:** Gossia uses Python, and its sheets are trees. The Python Tree is *Gossia bidwillii*, an Australian rainforest tree named for its smooth, mottled, python-like bark. 

The full design is in [DESIGN.md](DESIGN.md).

## Requirements

- **Rust 1.97 or newer.** This is a project requirement (RustPython and its `ruff` fork need it). `rust-toolchain.toml` pins the exact toolchain (currently 1.99.1) and `rustup` installs it on first use. If you do not use `rustup`, update with `rustup update stable` or install a compiler that is at least 1.97.
- **Git**, which the helper tools use to fetch pinned dependencies.

## Building

```
cargo xtask test      # run the tests
cargo xtask lint      # formatting and clippy
cargo xtask ci        # everything the blocking CI jobs run
cargo build           # build the application (binary: gossia)
```

`cargo xtask` is the project's helper command (see `tools/`). It lives in its own workspace, so it works on a fresh clone.

### The `vendor/` folder

A few dependencies cannot come from crates.io as published (RustPython and the `ruff` fork it needs). They are fetched at exact commits, listed in `vendor.lock`, into `vendor/`, which is not committed.

```
cargo xtask vendor fetch     # fetch any missing or out-of-date item
cargo xtask vendor status    # show each item and its state
cargo xtask vendor verify    # check every item against vendor.lock
cargo xtask clean            # remove the build output AND vendor/
```

- `vendor/` is kept between builds and is removed only by `cargo xtask clean`.
- `cargo xtask lint`, `test` and `ci` fetch whatever the workspace needs before starting Cargo.
- **The first build takes several minutes**, because it compiles RustPython (about 4.5 minutes in release on the development machine). Later builds reuse it. The `rustpython` item is fetched with a patch (`vendor-patches/rustpython.patch`) that makes its `ruff` dependencies point at `vendor/ruff`; see DESIGN.md, "Patching a vendored item".
- **A plain `cargo build` cannot do that.** Cargo needs every path dependency to exist before it runs any build script, so on a fresh clone, once the workspace depends on `vendor/`, run `cargo xtask vendor fetch` first (once). Cargo's error in that case is `no matching package named ... found`, with the path under `vendor/`.
- Set `GOSSIA_VENDOR_DIR` to use a copy of the folder prepared elsewhere (for example when offline).

## CI

- **On GitHub** (`.github/workflows/ci.yml`): lint and the tests, on GitHub's own Ubuntu runners, for pushes and pull requests.
- **On the owner's Gitea mirror** (`.gitea/workflows/`): everything else, for pushes to main only: the end-to-end and snapshot tests, the web and VS Code tests, the live database tests, and the packages. No self-hosted runner is registered with GitHub, so pull requests never run on the project's machines.

None of it needs Node.js: the VS Code extension's tests are started by `vscode/test/run.py` and the `.vsix` is packed by `packaging/vscode/pack-vsix.py`.

## Support

Questions, problems and ideas: raise an issue at https://github.com/wvdveer/gossia/issues. The privacy policy is in [PRIVACY.md](PRIVACY.md).

## Licence

Apache-2.0. See [LICENSE](LICENSE).

## Trying the window

`cargo run -p gossia-app` opens a sample workbook (an order with formulas, and a price list) so that the grid, the formula bar, undo and the rest can be tried. Opening and saving files comes later in the build.
