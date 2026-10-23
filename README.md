# cliotree

An Excel-like application which handles JSON Schema and XML Schema documents. Each sheet is a tree defined by a schema instead of a 2D grid, and cells can hold Python formulas whose results must match the schema.

The full design is in [DESIGN.md](DESIGN.md).

## Requirements

- **Rust 1.97 or newer.** This is a project requirement (RustPython and its `ruff` fork need it). `rust-toolchain.toml` pins the exact toolchain (currently 1.99.1) and `rustup` installs it on first use. If you do not use `rustup`, update with `rustup update stable` or install a compiler that is at least 1.97.
- **Git**, which the helper tools use to fetch pinned dependencies.

## Building

```
cargo xtask test      # run the tests
cargo xtask lint      # formatting and clippy
cargo xtask ci        # everything the blocking CI jobs run
cargo build           # build the application (binary: cliotree)
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
- **A plain `cargo build` cannot do that.** Cargo needs every path dependency to exist before it runs any build script, so on a fresh clone, once the workspace depends on `vendor/`, run `cargo xtask vendor fetch` first (once). Cargo's error in that case is `no matching package named ... found`, with the path under `vendor/`.
- Set `CLIOTREE_VENDOR_DIR` to use a copy of the folder prepared elsewhere (for example when offline).

## Licence

Apache-2.0. See [LICENSE](LICENSE).
