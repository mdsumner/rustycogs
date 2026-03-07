# rustycogs (dev)

## New functions

* `tiff_ifd_info()` returns one row per IFD per file — the compact structural
  summary you want before deciding which tiles to fetch. Includes `is_tiled`,
  `n_tiles_x`, `n_tiles_y`, `photometric`, and `predictor` alongside all
  metadata columns also found in `tiff_refs()`.

* `tiff_read_tiles()` takes a `tiff_refs()` data frame (or subset) and returns
  it with a `data` list-column of decoded pixel vectors. Files are opened once
  each and tiles fetched in a single vectorized range request per file via
  `fetch_tiles()`. Row order of the input is preserved across the async
  multi-file fan-out.

## Improvements to `tiff_refs()`

* Six new columns: `gdal_nodata`, `planar_configuration`, `scale_x`,
  `scale_y`, `origin_x`, `origin_y`. The geotransform columns come from
  ModelPixelScale and ModelTiepoint GeoTIFF tags; `scale_y` is the raw
  positive pixel size (negate for north-up convention). `gdal_nodata` is the
  GDAL metadata nodata string, coerced to character.

* Strip-based IFDs now emit a `Warning:` message and are skipped, rather than
  silently producing zero rows.

## Improvements to `refs_to_kerchunk()`

* `fill_value` in the `.zarray` spec now uses `gdal_nodata` from the refs data
  frame when present (e.g. `-32767` for GEBCO), falling back to `0` if absent
  or NA. Previously always hardcoded to `0`.

## R interface

* All exported functions now have default arguments (`region = NULL`,
  `anon = FALSE`, `concurrency = 16L`) — the extendr-generated wrappers are
  no longer the public API.

* `path.expand()` is applied to local paths in all functions. URI schemes
  (`://`) are detected and passed through unchanged.

* `tile_to_array()` now uses `byrow = TRUE` for the single-band matrix case,
  consistent with row-major (C) order from async-tiff and expected by
  `rasterImage()` and `ximage()`. Use `as.vector(t(m))` to recover the
  original flat vector order.

* `tiff_tiles()` now uses `ifd.fetch_tiles()` internally — a single
  vectorized range request to the object store rather than N sequential
  fetches. Significant speedup for large batches over HTTP/S3.

* `refs_to_kerchunk()` has been internalized, it probably doesn't belong here. 

# rustycogs 0.1.0

Initial release. TIFF/COG metadata scanning and tile decoding from R via
Rust, without GDAL or Python.

## Core functions

* `tiff_refs()` scans TIFF/COG files and returns a data.frame of tile
  byte-range references (path, IFD, tile col/row, offset, length, dimensions,
  dtype, compression, CRS EPSG). Accepts S3, GCS, Azure, HTTP/HTTPS, and
  local file paths.

* `tiff_tile()` fetches and decodes a single tile from a TIFF/COG.

* `tiff_tiles()` fetches multiple tiles from the same file in a single call.

* `tile_to_array()` converts a tile result to an R matrix (single band) or
  3D array (multi-band).

* `refs_to_kerchunk()` converts a refs data.frame to Kerchunk V1 JSON.

## Rust stack

Built on async-tiff 0.2 (Development Seed) and object_store 0.13 (Apache
arrow-rs) via extendr 0.7. Async I/O via tokio; all cloud credential
resolution follows object_store conventions.
