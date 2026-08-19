# Investigation: tile decode fails for sub-byte BitsPerSample (e.g. 2-bit)

## Symptom

Discovered while testing the http query-string fix against the AAD EDS test
URL:

```
https://data.aad.gov.au/eds/api/dataset/9ab5c3a3-7753-4f0e-bac1-bfce742d1722/object/download?prefix=rock_union1.tif
```

`tiff_ifd_info()` reads this file's metadata fine:

```
bits_per_sample = 2, samples_per_pixel = 1, compression = None,
photometric = BlackIsZero, predictor = NA
```

But `tiff_tile()` / `tiff_tiles()` / `tiff_read_tiles()` fail on every tile:

```
Error: Failed to decode tile: General error: Internal error: incorrect shape
or data length passed to Array::try_new. Got data length 4096, expected 16384
```

4096 vs 16384 is exactly the 4x ratio you'd expect from 2-bit-packed samples
(4 samples per byte) being treated as 1 byte per sample: 128×128 tile,
2 bits/sample ⇒ 128*128*2/8 = 4096 bytes on the wire, but the code expects
128*128*1 = 16384 one-byte samples.

## Root cause (in async-tiff, not rustycogs)

Traced through the pinned commit
(`/perm_storage/home/mdsumner/asyct/async-tiff`, `ebb6664`):

1. `DataType::from_tags()` (`src/data_type.rs`) only maps
   `(SampleFormat::Uint, bits)` to a concrete type for
   `bits ∈ {1 (→ Bool), 8, 16, 32, 64}`. Any other bit depth — including the
   TIFF-spec-legal 2 and 4 bit depths used by low-bit-depth
   grayscale/palette/classified images — falls through to `None`.

2. `Tile::decode()` (`src/tile.rs`) calls `UncompressedDecoder::decode_tile()`
   for `Compression::None`, which just returns the raw bytes unchanged — no
   bit-unpacking happens anywhere in the compression/predictor pipeline for
   these bit depths.

3. `Array::try_new()` → `TypedArray::try_new()` (`src/array.rs`) treats
   `data_type == None` the same as `UInt8`:
   ```rust
   None | Some(DataType::UInt8) => Ok(TypedArray::UInt8(data)),
   ```
   i.e. it silently assumes 1 byte per sample rather than raising a clear
   "unsupported bit depth" error at this point.

4. `Array::try_new()` then compares the resulting element count against
   `shape[0] * shape[1] * shape[2]` (computed from image dimensions and
   samples-per-pixel, independent of bit depth) and only *then* fails, with
   a message that reads like an internal invariant violation rather than an
   actionable "this bit depth isn't supported yet".

Note that 1-bit data *is* handled correctly — `Array::try_new` has a
dedicated `Bool` branch that calls `expand_bitmask()` to unpack a bitmask
into `Vec<bool>`. There is no equivalent unpacking path for 2-bit or 4-bit
samples; only the 1-bit case was special-cased.

## Scope of the gap

- Affects any TIFF/COG with `BitsPerSample` of 2 or 4 (per-sample), which
  turns up in indexed/paletted or classified-category rasters (this AAD file
  is exactly that: a rock-unit classification raster, `BlackIsZero`,
  1 sample/pixel, 2 bits/sample, uncompressed).
- Only affects **tile decoding** (`tiff_tile`, `tiff_tiles`,
  `tiff_read_tiles`). Metadata-only functions (`tiff_ifd_info`, `tiff_refs`)
  are unaffected since they never call `.decode()`.
- Independent of, and unrelated to, the http query-string URL fix — this
  file happens to be reachable only via a query-string URL, which is how it
  surfaced during testing, but the same failure would occur reading this
  file from S3, GCS, or local disk.

## Why rustycogs can't easily patch around this

`Tile`'s fields (`compressed_bytes`, `bits_per_sample`, `endianness`, etc. in
`src/tile.rs`) are `pub(crate)` to async-tiff, not public. rustycogs only
gets a `Tile` back from `ifd.fetch_tile()`/`fetch_tiles()` and can only call
`.decode()` on it — there's no way to intercept the raw decompressed bytes
from outside the crate to unpack them ourselves. A real fix has to happen
upstream in async-tiff:

- Add 2-bit/4-bit (and ideally the general N-bit-per-sample case, matching
  what the TIFF spec allows) to `DataType`, or a separate packed-integer
  representation, plus unpacking logic analogous to `expand_bitmask()` but
  parameterized on bit width — most naturally added as a new `TypedArray`
  path exercised for the `None` bits_per_sample case in `Array::try_new`.
  `predictor.rs` would need updating too if such files ever combine sub-byte
  depths with a predictor (uncommon in practice, but the `match bits_per_sample`
  arms there would need to stay consistent).

## Recommended near-term action for rustycogs

Until that upstream support exists, the cheapest improvement is a clearer,
earlier error rather than trying to work around the missing decode support:
check `bits_per_sample` before calling `tile.decode()` in `fetch_decode_tile`,
`fetch_decode_tiles_batch`, and `fetch_decode_group` in `lib.rs`, and if it's
not one of `{1, 8, 16, 32, 64}`, return a message like:

```
"Unsupported bits_per_sample={} for tile decoding (async-tiff only supports
1/8/16/32/64-bit samples); this file needs an async-tiff upstream fix"
```

This doesn't add support for 2-bit tiles, but turns a confusing
"Internal error: incorrect shape..." message into an actionable one, and
would apply uniformly regardless of which decoder path async-tiff uses
internally. Not yet implemented — flagging for a decision on whether it's
worth doing now versus waiting to see whether an upstream async-tiff fix
lands first.
