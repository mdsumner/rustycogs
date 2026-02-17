# Design v3: Tile decode and the reference store pivot

Follows design-v2-streamlined.md. Records the addition of tile decoding
and the architectural insight that the refs data.frame is a universal pivot
between scanning and reading.

## What changed from v2

v2 explicitly scoped out pixel decoding. v3 adds it via `tiff_tile()` and
`tiff_tiles()`, using async-tiff's `fetch_tile()` → `Tile::decode()` →
`Array::into_inner()` pipeline. This gives R direct access to decoded pixel
values from cloud COGs without GDAL.

The addition was straightforward because the TIFF open / IFD read machinery
already existed for `tiff_refs()`. Tile decode reuses `make_store_and_path()`
and the same metadata reader pattern, adding only the fetch + decompress +
type-cast step.

## Three read paths

The refs data.frame is the universal pivot. Once you have offset + length +
path for every tile, three read paths are available:

```
tiff_refs() → data.frame(path, offset, length, ...)
                  │
                  ├─ Path A: tiff_tile() / tiff_tiles()
                  │    Rust-native decode. Fastest for R consumption.
                  │    async-tiff handles fetch, decompress, type cast.
                  │    Returns numeric vector + dim + dtype.
                  │
                  ├─ Path B: gdalraster::VSIFile seek/read
                  │    GDAL's virtual filesystem I/O, any /vsi* prefix.
                  │    Fetch raw compressed bytes, decompress in R.
                  │    Works with /vsicurl/, /vsis3/, /vsigs/, /vsiaz/.
                  │    No TIFF parsing needed — just seek(offset), read(length).
                  │
                  └─ Path C: Reference store for external consumers
                       ├─ Kerchunk V1 JSON → xarray, zarr-python
                       ├─ Kerchunk parquet → xarray (VirtualiZarr format)
                       ├─ Arrow/Parquet    → any language, any tool
                       └─ GDAL Zarr driver → when it matures
```

Path A is the fast path for R. Path B is interesting because it decouples
the TIFF scanner (Rust) from the byte reader (GDAL), using only GDAL's I/O
layer without needing GDAL to understand TIFF structure. Path C is the
interop path for handing reference stores to Python/xarray workflows.

## VSIFile pattern

gdalraster's VSIFile class provides direct byte-range access through GDAL's
virtual filesystem. Combined with the refs data.frame:

```r
tile_via_vsi <- function(refs, idx = 1, dsn_prefix = "/vsicurl/") {
  r <- refs[idx, ]
  vsi <- new(gdalraster::VSIFile, paste0(dsn_prefix, r$path))
  vsi$seek(r$offset, gdalraster::SEEK_SET)
  bytes <- vsi$read(r$length)
  vsi$close()
  uncomp <- memDecompress(bytes, "gzip")
  readBin(uncomp, "numeric", n = r$tile_w * r$tile_h, size = r$bits_per_sample / 8)
}
```

This is a GDAL-free TIFF scanner feeding a GDAL-native byte reader. The
refs table provides everything needed: path for the VSI URL, offset and
length for the seek/read, compression for the decompressor, dtype for
readBin sizing. GDAL handles the cloud I/O plumbing (/vsicurl/ caching,
/vsis3/ authentication) without ever opening the file as a raster dataset.

The pattern scales to any /vsi* prefix: /vsicurl/ for HTTP, /vsis3/ for S3,
/vsigs/ for GCS, /vsiaz/ for Azure. Credential handling is GDAL's job.

## Async-tiff API notes (v0.2.0)

Key patterns learned during development:

- `Array::into_inner()` returns `(TypedArray, [usize; 3], Option<DataType>)`.
  There is no `into_typed()` method; the `data` field holding the `TypedArray`
  is accessed via destructuring.
- `data_type()` returns `Option<DataType>`, not `DataType`.
- `shape()` returns `[usize; 3]`, not `&[usize]`.
- `fetch_tile(col, row, reader)` takes `usize` not `u32`.
- `TypedArray` in v0.2.0 has variants: UInt8, UInt16, UInt32, UInt64, Int8,
  Int16, Int32, Int64, Float32, Float64. The `Bool` variant exists on main
  but not in the 0.2.0 release.
- `tile_width()` / `tile_height()` return `Option<u32>`. None means
  strip-based IFD.
- `tile_offsets()` / `tile_byte_counts()` return `Option<&[u64]>`.
- `geo_key_directory()` returns `Option<&GeoKeyDirectory>`. The
  `projected_type` and `geographic_type` fields are `Option<u16>`.
- `CompressionMethod` is not unit-only — use `format!("{:?}", ...)` for
  Debug string ("Deflate", "Jpeg", etc.).
- extendr's `list!()` macro returns `List`, not `Robj`. Append `.into()`
  when the return type is `Robj`.
- extendr's `Result<T>` is `std::result::Result<T, extendr_api::Error>`
  (single generic). Use `std::result::Result<T, String>` for internal
  error handling.

## Kerchunk parquet format (planned)

The VirtualiZarr / kerchunk parquet format that xarray consumes natively
uses one parquet file per Zarr variable with columns:

- `path`: source file URL
- `offset`: byte offset (int64)
- `length`: byte count (int64)

Keyed by chunk index (matching Zarr chunk layout). Our refs data.frame
already contains all these columns. The conversion is a reshape by IFD
with the right partition structure. This would give R the ability to
produce xarray-ready virtual datasets without Python.

## Concurrency (deferred)

Two concurrency improvements are obvious:

1. **Multi-file scanning**: `tiff_refs()` currently scans files sequentially.
   Tokio `join_all` with a semaphore would parallelize across files, reducing
   wall time for large file sets from minutes to seconds.

2. **Multi-tile fetching**: `tiff_tiles()` fetches tiles sequentially within
   a file. `join_all` on the fetch futures would parallelize network I/O,
   though decode is CPU-bound and already fast.

Both are straightforward additions to the existing async architecture.
The sequential versions work correctly and the performance is already
competitive with GDAL.

## Relationship to hypertidy

- **grout**: Tile grid calculations. Given image dimensions and tile size
  from the refs, grout computes which tile col/row covers a geographic
  extent. rustycogs then fetches those tiles. The two packages compose
  naturally.
- **vapour**: GDAL-based reading. rustycogs provides a GDAL-free alternative
  for cloud COGs. The VSIFile pattern shows how they can cooperate: rustycogs
  scans, vapour/gdalraster reads bytes.
- **ximage**: Visualization. `tile_to_array()` output works directly with
  `ximage::ximage()` for quick-look rendering.
- **dsn**: Data source name handling. Could provide URL parsing and
  credential management feeding into rustycogs.

## Performance baseline (February 2026)

Australian Bathymetry 2023 COG (HTTP, 32000×20800, float32 deflate):

```
tiff_refs() scan (19,301 tiles):    0.44s
tiff_tile() single tile:            0.47s
tiff_tiles() 5 tiles batched:       0.56s (0.11s per tile after first)
GDAL /vsicurl/ block read:          0.66s
```

Sentinel-2 B04 (S3, 60 scenes):

```
tiff_refs() sequential:             49s (0.8s per scene, network bound)
```
