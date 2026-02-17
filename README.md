# rustycogs

Extract byte-range chunk references and decode tiles from cloud-hosted TIFF
and COG files, entirely from R, without Python or GDAL.

Uses Rust crates [async-tiff](https://github.com/developmentseed/async-tiff)
and [object_store](https://docs.rs/object_store/) (Apache arrow-rs) for async
I/O across S3, GCS, Azure, HTTP, and local storage.

Two modes:

- **Scan** (`tiff_refs`): extract tile byte-range references for Kerchunk/Zarr virtual stores
- **Decode** (`tiff_tile`, `tiff_tiles`): fetch and decompress pixel data directly

## Installation

Requires a Rust toolchain. See `docs/rust-setup.md` for guidance.

```r
remotes::install_github("hypertidy/rustycogs")
```

## Usage

### Scan tile references

```r
library(rustycogs)

# Scan a cloud-hosted COG — returns byte offsets for every tile in every IFD
refs <- tiff_refs("s3://bucket/path/file.tif", region = "us-west-2", anon = TRUE)

# Write to Parquet (for large reference sets)
arrow::write_parquet(refs, "references.parquet")

# Or build Kerchunk V1 JSON for GDAL's Zarr driver
kc <- refs_to_kerchunk(refs)
writeLines(kc, "references.json")
```

### Decode tiles

```r
# Fetch and decode a single tile (returns raw pixel values)
tile <- tiff_tile("https://example.com/cog.tif", ifd_index = 0, col = 0, row = 0)
m <- tile_to_array(tile)
image(m)

# Batch fetch: opens the file once, fetches multiple tiles
tiles <- tiff_tiles("https://example.com/cog.tif",
                    cols = c(0, 1, 2, 3, 4), rows = c(0, 0, 0, 0, 0))
```

### GDAL Kerchunk round-trip

The Kerchunk JSON output is compatible with GDAL >= 3.8's Zarr driver:

```r
# 1. Scan (fast — metadata only)
refs <- tiff_refs("https://example.com/big.tif")

# 2. Write reference store
writeLines(refs_to_kerchunk(refs), "refs.json")

# 3. Read via GDAL (uses byte-range fetches guided by the reference store)
r <- terra::rast("ZARR:refs.json")
```

## What comes back

### tiff_refs

```
path | ifd | tile_col | tile_row | offset | length | image_w | image_h |
tile_w | tile_h | dtype | compression | bits_per_sample | samples_per_pixel |
crs_epsg
```

Each row is one tile in one IFD of one file.

### tiff_tile / tiff_tiles

A list with:
- `data`: numeric vector of decoded pixel values
- `dim`: integer vector `c(height, width)` or `c(height, width, bands)`
- `dtype`: numpy-style type string (`"<f4"`, `"<u2"`, etc.)

## Related

- [vapour](https://github.com/hypertidy/vapour) — GDAL-based raster/vector reading
- [grout](https://github.com/hypertidy/grout) — tile scheme calculations
- [gdalraster](https://github.com/USDAForestService/gdalraster) — GDAL bindings for R
- [async-tiff](https://github.com/developmentseed/async-tiff) — the Rust crate powering this
- [virtual-tiff](https://github.com/virtual-zarr/virtual-tiff) — Python equivalent using async-tiff
