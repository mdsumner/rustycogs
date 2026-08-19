# Fix: http(s) URLs with query strings fail (use ReqwestReader, not object_store HttpStore)

**Status: implemented.** `object_store` was kept at 0.13 (the bump to 0.14
discussed below was deferred as a separate change, pending a check of
`aws-lc-sys` portability on CRAN's Windows/macOS builders). Note that adding
`reqwest` with the `rustls` feature pulls in `aws-lc-rs`/`aws-lc-sys` anyway
(reqwest 0.13 changed its default rustls crypto provider from `ring` to
`aws-lc-rs`), so this dependency was not actually avoided by deferring the
`object_store` bump — see NEWS.md for the summary of the change actually
shipped.

## Problem

Reading a TIFF via an http(s) URL that carries a query string fails, e.g. the AAD
EDS endpoint:

```
https://data.aad.gov.au/eds/api/dataset/9ab5c3a3-7753-4f0e-bac1-bfce742d1722/object/download?prefix=rock_union1.tif
```

GDAL reads this fine (VSICURL follows the 302 to a signed
`transfer.data.aad.gov.au` S3 URL and caches the effective URL), but rustycogs
fails at `TiffMetadataReader::try_open`.

## Root cause

Not an async-tiff version issue. The Cargo.lock pin (`ebb6664`, git main) is only
a couple dozen chore commits behind HEAD, and everything needed has been in
async-tiff since early on.

The bug is in `make_store_and_path` in `src/rust/src/lib.rs`, in the
`"http" | "https"` branch:

```rust
let base = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
let key = parsed.path().trim_start_matches('/');
let obj_path = object_store::path::Path::from(key);
```

- `parsed.path()` discards the query string, so the server receives
  `.../object/download` with no `?prefix=rock_union1.tif` (the port is also
  dropped from `base`).
- This is not fixable within object_store: its `Path` model cannot express query
  strings at all (a `?` would be percent-encoded). `HttpStore` is the wrong
  vehicle for this class of URL.

Verified with a local mock reproducing the AAD shape (query-string URL, HEAD
rejected, 302 redirect to a signed URL on another host that honors Range):

```
== rustycogs current approach (object_store HttpStore) ==
   object_store path sent to server: Path { raw: "eds/api/dataset/xyz/object/download" }   <- query gone
   FAILED: try_open: Generic HTTP error ...
== ReqwestReader with full URL ==
   OK: 1 IFDs
```

## Why ReqwestReader works

`async_tiff::reader::ReqwestReader` (default `reqwest` feature, `Clone`) takes a
full `reqwest::Url`:

- the query string survives intact
- reqwest's default policy follows redirects (up to 10), including the
  cross-host 302 to the signed URL
- the `Range` header is preserved across the redirect (reqwest only strips
  Authorization/Cookie-class headers on host change); the signed S3 URL answers
  206
- metadata reading (`try_open` + `read_all_ifds`) issues only ranged GETs from
  offset 0: no HEAD, no file-size probe. This matters because the AAD signed
  URLs are GET-only (GDAL's debug shows it retrying with GET after HEAD fails).

Mock server log for the metadata scan, showing Range preserved through the
redirect on every request:

```
[primary] GET /eds/api/.../object/download?prefix=rock_union1.tif Range=bytes=0-32767
[signed]  GET /aadc-datasets/AAS_4568/rock_union1.tif Range=bytes=0-32767   -> 206
[primary] GET ...?prefix=rock_union1.tif Range=bytes=32768-98303
[signed]  GET ... Range=bytes=32768-98303   -> 206
```

## Change

### 1. `src/rust/Cargo.toml`

Add reqwest with a rustls backend. async-tiff declares reqwest with
`default-features = false`, so without a TLS feature somewhere in the graph the
client fails on https at runtime. Feature unification means enabling it here
also gives async-tiff's reqwest instance TLS. rustls avoids system-openssl
linkage in the R package build.

```toml
reqwest = { version = "0.13", default-features = false, features = ["rustls"] }
```

Note: reqwest 0.13 renamed this feature from `rustls-tls` (used in 0.11/0.12) to
plain `rustls`; `rustls-tls` does not exist in 0.13 and will fail to build.

Also bump object_store to match async-tiff git main, which has moved to 0.14.
The current lock is consistent, but the next `cargo update` of the git pin will
otherwise produce the "two different versions of crate `object_store`" E0308.

```toml
object_store = { version = "0.14", features = ["aws", "gcp", "azure", "http"] }
```

### 2. `src/rust/src/lib.rs`

Add a `make_reader` that routes http(s) to `ReqwestReader` and everything else
through the existing `make_store_and_path` / `ObjectReader` path.
`AsyncFileReader` is implemented for `Arc<dyn AsyncFileReader>`, so the cache
plumbing takes it directly.

```rust
fn make_reader(
    url_str: &str,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<Arc<dyn async_tiff::reader::AsyncFileReader>, String> {
    if url_str.starts_with("http://") || url_str.starts_with("https://") {
        // Full URL straight to reqwest: query strings survive, redirects
        // (including signed-URL redirects) are followed, Range is preserved.
        let url = reqwest::Url::parse(url_str)
            .map_err(|e| format!("URL parse error: {}", e))?;
        let client = reqwest::Client::new();
        return Ok(Arc::new(async_tiff::reader::ReqwestReader::new(client, url)));
    }
    let (store, obj_path) = make_store_and_path(url_str, region, anon)?;
    Ok(Arc::new(async_tiff::reader::ObjectReader::new(store, obj_path)))
}
```

In `scan_one_file`, replace:

```rust
let (store, obj_path) = make_store_and_path(url_str, region, anon)?;
let reader = async_tiff::reader::ObjectReader::new(store, obj_path);
let cache = async_tiff::metadata::cache::ReadaheadMetadataCache::new(reader.clone());
```

with:

```rust
let reader = make_reader(url_str, region, anon)?;
let cache = async_tiff::metadata::cache::ReadaheadMetadataCache::new(reader.clone());
```

The `"http" | "https"` arm of `make_store_and_path` becomes dead code and can be
removed (it was also dropping the port from `base`).

If any tile-fetch path constructs its own `ObjectReader` from
`make_store_and_path`, switch it to `make_reader` the same way.

## Performance note

Unlike GDAL (which caches the effective URL for the signature lifetime,
`X-Amz-Expires=86400` here), `ReqwestReader` hits the primary URL on every range
request, so each fetch pays the redirect round trip. That is fine for an
IFD/metadata scan (a handful of requests). For tile-heavy reads, resolve the
redirect once and build the reader on the signed URL:

```rust
// Resolve the redirect once, then read ranges directly from the signed URL.
let resp = client
    .get(url.clone())
    .header("Range", "bytes=0-0")
    .send()
    .await
    .map_err(|e| format!("{}", e))?;
let effective = resp.url().clone();
let reader = async_tiff::reader::ReqwestReader::new(client, effective);
```

Caveat: the signed URL expires (24 hours for this endpoint), so a long-running
scan should retain the primary URL for refresh.

## Test URL

```
https://data.aad.gov.au/eds/api/dataset/9ab5c3a3-7753-4f0e-bac1-bfce742d1722/object/download?prefix=rock_union1.tif
```

Expected: metadata and all IFD tables read via ranged GETs through the redirect;
the `.tif.vat.dbf` 404 seen in GDAL debug is unrelated sidecar probing.

## Corroboration: same bug in the official async-tiff Python package

The official `async-tiff` Python package (built on `obstore`, which wraps the
same `object_store` Rust crate) fails on this URL identically when using
`obstore.store.HTTPStore` + `TIFF.open`, confirming the bug lives in
`object_store`'s `Path`/`HttpStore` model, not in anything rustycogs-specific:

```python
from obstore.store import HTTPStore
from async_tiff import TIFF

store = HTTPStore.from_url("https://data.aad.gov.au")
path = "eds/api/dataset/9ab5c3a3-7753-4f0e-bac1-bfce742d1722/object/download?prefix=rock_union1.tif"
tiff = await TIFF.open(path, store=store)
# FileNotFoundError: ... GET .../object/download%3Fprefix=rock_union1.tif ... 404 Not Found
```

The `?` gets percent-encoded as a literal path character (`%3F`) rather than
treated as a query string, so the server 404s. Resolving the redirect
manually first doesn't help either — the signed S3 URL it redirects to also
carries a query string (the AWS SigV4 signature parameters), so it hits the
same limitation.

`async-tiff`'s Python bindings, however, also accept any object implementing
the `obspec` `GetRangeAsync`/`GetRangesAsync` protocol as the `store`
argument (`TIFF.open(path, store=...)`, see
`python/src/reader.rs`/`python/src/tiff.rs` in the async-tiff source) —
this is the same escape hatch rustycogs uses via `ReqwestReader` on the Rust
side. A minimal such backend that issues plain HTTP Range requests (via
`httpx`, following redirects on every request) against the full URL works:
see `python_http_range_reader.py` alongside this doc. Tested working for
`TIFF.open` and `fetch_tile`/`fetch_tiles` (tile *decoding* for this
particular file separately fails on the unrelated 2-bit-per-sample gap
documented in `investigate-2bit-tile-decode.md`).
