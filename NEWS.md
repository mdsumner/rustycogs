# rustycogs 0.1.0

Initial release. TIFF/COG metadata scanning and tile decoding from R via
Rust, without GDAL or Python.

## Core functions

* `tiff_refs()` scans TIFF/COG files and returns a data.frame of tile
  byte-range references (path, IFD, tile col/row, offset, length, dimensions,
  dtype, compression, CRS EPSG). Accepts S3, GCS, Azure, HTTP/HTTPS, and
  local file paths. Multiple files are scanned sequentially (concurrent
  scanning is a planned enhancement).

* `tiff_tile()` fetches and decodes a single tile from a TIFF/COG, returning
  decoded pixel values as a numeric vector with dimension and dtype metadata.
  Handles deflate and JPEG compression natively via async-tiff.

* `tiff_tiles()` fetches multiple tiles from the same file in a single call,
  opening the TIFF once and reusing the metadata and connection. More
  efficient than calling `tiff_tile()` in a loop.

* `tile_to_array()` converts a tile result to an R matrix (single band) or
  3D array (multi-band).

## Reference store output

* `refs_to_kerchunk()` converts a refs data.frame to Kerchunk V1 JSON,
  suitable for consumption by xarray, zarr-python, or (in future) GDAL's
  Zarr driver. Handles compressor serialization (deflate → zlib, JPEG, etc.)
  and NULL compressor for uncompressed tiles.

* The refs data.frame works directly with `arrow::write_parquet()` for
  large reference sets spanning thousands of files.

## Rust stack

Built on async-tiff 0.2 (Development Seed) and object_store 0.13 (Apache
arrow-rs) via extendr 0.7. Async I/O is handled by tokio, with a runtime
created per call. All cloud credential resolution follows object_store
conventions (environment variables, instance profiles, anonymous access via
`anon = TRUE`).

## Supported formats

Tiled TIFF and COG files. Strip-based IFDs are skipped (no tile offsets).
All standard COG compression methods are supported: deflate, JPEG, LZW,
zstd, uncompressed. Data types: uint8, uint16, uint32, uint64, int8, int16,
int32, int64, float32, float64.

## GeoTIFF metadata

EPSG codes are extracted from GeoTIFF key directory, trying ProjectedCSType
first and falling back to GeographicType. Files without GeoTIFF keys return
NA for crs_epsg.

## Performance

Metadata scanning is network-bound (2–3 HTTP range requests per COG). A
19,000-tile file scans in ~0.4 seconds. Single tile decode runs at ~0.5
seconds including connection setup; batched tiles amortize the setup cost to
~20ms per additional tile. Comparable to or faster than GDAL /vsicurl/ for
equivalent operations.

## Known limitations

* File scanning is sequential across multiple paths. Concurrent scanning via
  tokio `join_all` is the obvious next step.
* Kerchunk JSON output is V1 format. Kerchunk parquet (the VirtualiZarr
  format consumed by xarray) is a planned addition.
* GDAL's Zarr driver cannot yet reliably read the Kerchunk JSON output;
  this depends on GDAL Kerchunk driver maturation.
* No CRS transformation or spatial subsetting — the package returns tile
  indices, not geographic queries. Use grout for tile/extent mapping.
