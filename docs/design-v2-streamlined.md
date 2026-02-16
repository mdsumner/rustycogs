# rustycogs: TIFF virtualization from R via Rust

## Goal

Given a vector of TIFF/COG URLs (or local paths), return a data frame of byte-range chunk references. No pixel data moves. Output goes to arrow for Parquet or jsonlite for Kerchunk JSON — both already efficient in R.

## Rust dependencies

- **`async-tiff`** (Development Seed) — async IFD parser, built on `object_store`
- **`object_store`** (Apache) — comes in transitively, handles S3/GCS/Azure/HTTP/local
- **`tokio`** — async runtime, also transitive

One dependency tree, no version conflicts.

## R interface

```r
refs <- tiff_refs("s3://bucket/file.tif")

refs <- tiff_refs(paths, region = "us-west-2", anon = TRUE)

refs <- tiff_refs(local_files)
```

Returns a data frame:

```
path | ifd | tile_col | tile_row | offset | length | image_w | image_h | tile_w | tile_h | dtype | compression | bits_per_sample | crs_epsg
```

That's the package.

## Rust internals

```rust
use async_tiff::TIFF;
use extendr_api::prelude::*;
use tokio::runtime::Runtime;

#[extendr]
fn tiff_refs(paths: &[Rstr], region: Option<&str>, anon: Option<bool>) -> Robj {
    let rt = Runtime::new().unwrap();
    rt.block_on(async {
        // Build store from URL scheme + credentials
        // For each path: TIFF::open, walk IFDs, collect references
        // Fan out concurrently with join_all / semaphore
        // Pack columns into R data frame
    })
}
```

The tokio runtime is created per call. This is fine — runtime construction is ~microseconds, the actual cost is network I/O. If profiling later shows otherwise, add a persistent handle.

Credential resolution follows `object_store` conventions: environment variables (AWS_ACCESS_KEY_ID etc.), instance profiles, anonymous. The `region` and `anon` arguments cover the common cases. More can be added as needed without breaking the interface.

## Concurrency

Rust-side only. A single `tiff_refs()` call fans out across all paths using tokio tasks, bounded by a semaphore to limit concurrent connections. R stays single-threaded.

For 1000 files, R sees one blocking call that returns one data frame. The Rust side is reading IFDs from many files concurrently.

## Output assembly (pure R)

```r
refs <- tiff_refs(paths, region = "us-west-2", anon = TRUE)

# Parquet reference store
arrow::write_parquet(refs, "references.parquet")

# Kerchunk V1 JSON
refs_to_kerchunk <- function(refs, ...) {
  # Reshape data frame into {"version":1, "refs": {"var/0.0": [url, offset, length], ...}}
  # Pure R, jsonlite::write_json
}
```

The TIFF-to-Zarr metadata mapping (compression codec, dtype, chunk shape) is derived from the IFD fields already in the data frame. This logic is small and belongs in R.

## Binding

extendr via rextendr. External pointer not needed in v1 — just a function call that blocks on async and returns a data frame.

## Distribution

r-universe first. CRAN when stable. Vendored tarball will be chunky (async-tiff + object_store + tokio) but within precedent for Rust R packages.

## Out of scope

- Pixel decoding
- Store handle management
- Bucket listing
- Zarr reading
- CRS anything
