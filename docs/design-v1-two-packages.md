# Design v1: Two-Package Approach with Binding Framework Comparison

This document records the original design exploration before simplifying to
the single-package approach in design-v2-streamlined.md. Preserved as a
record of the evaluation process and considered alternatives.

## Original Goal

Two R packages wrapping Rust crates for TIFF virtualization:

- **robstore** — wraps Apache `object_store` crate for S3/GCS/Azure/HTTP/local
  storage operations (list, head, get_range)
- **rustycogs** — wraps `async-tiff` (Development Seed) for parsing TIFF/COG
  IFD structures and extracting tile byte-range references

The split mirrored the Rust crate boundaries. robstore would provide a
reusable store handle; rustycogs would accept that handle for I/O.

## Decision: Simplified to Single Package

The two-package split was abandoned because:

1. robstore's standalone value was marginal — listing buckets and reading
   ranges is already available via aws.s3, gdalraster, arrow's S3 filesystem
2. Cross-package external pointer ABI coupling is fragile and complicates
   CRAN submission
3. async-tiff already depends on object_store transitively — no version
   conflict risk within a single crate
4. The exploration use case (scan IFDs, get references) doesn't need a
   persistent store handle — constructing one per call is microseconds

robstore remains a valid future package if standalone cloud store operations
from R prove independently useful. The API design work transfers directly.

## Binding Framework Comparison

### extendr (RECOMMENDED)

- Most mature: JOSS paper 2024, rextendr on CRAN, 30+ CRAN packages
  (arcgis suite, gifski, heck, prqlr, string2path)
- `#[extendr]` on structs gives R6-like external pointers for keeping
  object_store client / tokio runtime alive across calls
- Rich type conversions: `Vec<f64>` ↔ R numeric, `&[u8]` ↔ raw vectors
- rextendr handles vendoring, Makevars, CRAN submission workflow
- Active development, Discord community, multiple maintainers
- Concern: Implicit conversions can be opaque for complex cases

### savvy (Yutani)

- Explicit over ergonomic: work with `IntegerSexp`/`OwnedRealSexp`/`RawSexp` directly
- Lighter weight, faster compiles, owned/read-only SEXP distinction maps to
  Rust ownership
- savvy-cli handles R wrapper/C glue generation
- Used on CRAN (string2path moved from extendr to savvy)
- Concern: Smaller community (essentially one maintainer), more verbose for
  simple cases, less tooling than rextendr

### roxido (Dahl, BYU)

- Minimal overhead, lowest call overhead per benchmarks
- Transparent/extensible: wrapper code in `src/rustlib/roxido/` directory
- Pc structure for protection management is elegant
- Concern: Smallest ecosystem, less documentation for complex use cases
  (async, external pointers)

### Decision

extendr via rextendr, because:
- Async Rust (tokio) support is the primary challenge; extendr's struct-based
  external pointers are the most proven path for keeping runtime handles alive
- CRAN vendoring workflow via rextendr is essential for distribution
- Largest community for troubleshooting novel patterns
- savvy is the credible alternative if extendr proves problematic

## Async Architecture

Both object_store and async-tiff are heavily tokio-based. The key challenge is
managing the async runtime when crossing to R (single-threaded).

**Solution**: Create a tokio Runtime in Rust, run all async operations via
`rt.block_on()`, which blocks the R thread until completion. Rust-side
concurrency (reading IFDs from many files in parallel) uses tokio tasks
internally via `futures::future::join_all`, bounded by semaphore for
connection limits.

This means R stays single-threaded, but a single `tiff_refs()` call fans out
across all paths concurrently on the Rust side.

## Original Two-Package API Sketch

### robstore

```r
store <- rs_store("s3://bucket", region = "us-west-2", anon = TRUE)
rs_list(store, prefix = "path/to/")
rs_head(store, "path/to/file.tif")
rs_get_range(store, "path/to/file.tif", offset = 0, length = 1024)
```

### rustycogs (with robstore dependency)

```r
store <- rs_store("s3://bucket", region = "us-west-2", anon = TRUE)
refs <- tiff_refs(store, paths = c("file1.tif", "file2.tif"))
```

## Relationship to Hypertidy Packages

- **vapour**: GDAL-based raster/vector reading via C++. rustycogs provides
  GDAL-free path for Zarr/cloud-native formats. Complementary.
- **grout**: Tile scheme calculations. rustycogs delegates grid algebra to
  R/grout, handles byte-level I/O.
- **gdalraster**: Benchmark baseline. extendr struct pattern solves handle
  spinup overhead observed in gdalraster benchmarks.
- **dsn**: Data source name handling. Could provide URL/path parsing feeding
  into rustycogs.

## Key Insight

async-tiff from Development Seed is the exact crate needed — same engine
powering the Python virtual-tiff project. Provides async IFD walking over
object_store with tile request merging and concurrency. R output side
(arrow for Parquet, jsonlite for JSON) is already efficient and doesn't
need Rust reimplementation.
