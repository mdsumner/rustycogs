# Rust Setup for rustycogs Development

A practical guide for getting Rust working with R package development,
written for macOS and Linux.

## 1. Install Rust via rustup

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Accept the defaults (stable toolchain, modify PATH). Then restart your
terminal or run:

```bash
source "$HOME/.cargo/env"
```

Verify:

```bash
rustc --version   # e.g. rustc 1.82.0
cargo --version   # e.g. cargo 1.82.0
```

## 2. Check from R

```r
install.packages("rextendr")
rextendr::rust_sitrep()
```

This should show green ticks for `rustup`, `cargo`, and your host target.
On macOS you want `aarch64-apple-darwin` or `x86_64-apple-darwin`.
On Linux you want `x86_64-unknown-linux-gnu`.

If it reports a missing target, add it:

```bash
rustup target add x86_64-unknown-linux-gnu
```

## 3. Building rustycogs

```r
# Clone or download the repo, then from the package root:
devtools::load_all(".")

# Or install:
devtools::install(".")
```

The first build will take a few minutes as it compiles tokio, object_store,
async-tiff and all their dependencies. Subsequent builds are incremental
and much faster.

If the build fails, common issues:

**"linker cc not found"** — Install system C compiler. On Ubuntu:
`sudo apt install build-essential`. On macOS: `xcode-select --install`.

**OpenSSL errors** — object_store's HTTP backend needs OpenSSL headers.
On Ubuntu: `sudo apt install libssl-dev pkg-config`.
On macOS: usually provided by Homebrew `brew install openssl@3`, then
set `OPENSSL_DIR=$(brew --prefix openssl@3)`.

**Cargo network errors** — The first build fetches crates from crates.io.
You need internet access. Behind a proxy, set `HTTPS_PROXY` in your
environment.

## 4. Development workflow

The typical cycle:

1. Edit Rust code in `src/rust/src/lib.rs`
2. Run `rextendr::document()` — this compiles the Rust, generates R wrappers,
   and updates roxygen docs
3. Run `devtools::load_all()` to load the updated package
4. Test

`rextendr::document()` replaces both `cargo build` and `roxygen2::roxygenise()`.
You rarely need to run cargo directly.

## 5. Adding Rust dependencies

From R:

```r
rextendr::use_crate("serde", features = "derive")
```

Or manually edit `src/rust/Cargo.toml` and add to `[dependencies]`.

## 6. Updating Rust

```bash
rustup update
```

This updates the stable toolchain. Generally safe and recommended periodically.

## 7. CRAN preparation (later)

When ready for CRAN:

```r
rextendr::vendor_pkgs()
```

This creates `src/rust/vendor.tar.xz` containing all Rust dependencies,
so CRAN can build without network access. The tarball will be 10-20MB
for this package due to tokio + object_store.

## 8. Cargo basics (reference)

You don't need these for normal development (rextendr handles it), but
useful for debugging:

```bash
# Build the Rust crate directly
cd src/rust
cargo build --release

# Check for errors without full build
cargo check

# Run Rust tests (if any)
cargo test

# Format code
cargo fmt

# Lint
cargo clippy
```

## 9. Troubleshooting

**"can't find crate for `std`"** — Wrong or missing target. Run
`rustup target list --installed` and check against your platform.

**Slow builds** — First build is always slow (5-10 min). Set
`CARGO_BUILD_JOBS=4` (or however many cores) to parallelise. Incremental
builds after the first are typically seconds.

**rextendr::document() fails** — Try `rextendr::clean()` to wipe build
artifacts, then rebuild. Nuclear option: delete `src/rust/target/` entirely.

**Version conflicts** — If async-tiff and object_store versions drift,
`cargo update` in `src/rust/` will resolve to latest compatible versions.
Check `Cargo.lock` (which should be gitignored for library crates but
can be useful for debugging).
