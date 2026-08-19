"""Read http(s) TIFF URLs with query strings using the official async-tiff
Python package, working around the object_store `Path`/`HttpStore`
query-string bug documented in fix-http-query-string-urls.md.

`TIFF.open(path, store=...)` accepts either a `pyo3_object_store`-backed
store (obstore, which goes through object_store's `Path` model and drops
query strings) OR any Python object implementing the obspec
`GetRangeAsync`/`GetRangesAsync` protocol (get_range_async /
get_ranges_async). `HttpRangeReader` below is a minimal obspec-compatible
backend that issues plain HTTP Range requests against a single, fixed URL,
following redirects on every request -- this is the Python-side analogue of
Rust's `async_tiff::reader::ReqwestReader`, and sidesteps object_store
entirely rather than working around it.

Usage:

    import asyncio
    from async_tiff import TIFF
    from python_http_range_reader import HttpRangeReader

    async def main():
        url = (
            "https://data.aad.gov.au/eds/api/dataset/"
            "9ab5c3a3-7753-4f0e-bac1-bfce742d1722/object/download"
            "?prefix=rock_union1.tif"
        )
        reader = HttpRangeReader(url)
        tiff = await TIFF.open("", store=reader)  # `path` is unused/ignored
        print(tiff.ifds[0].image_width, tiff.ifds[0].image_height)

    asyncio.run(main())

Tested against async-tiff 0.7.2 / obstore 0.10.1. Metadata reads
(`TIFF.open`, `.ifds`) and tile *fetching* (`fetch_tile`/`fetch_tiles`) both
work through this reader. Tile *decoding* for this particular test file
still fails -- but that's the separate, unrelated 2-bit-per-sample decode
gap documented in investigate-2bit-tile-decode.md, not a fetch problem.
"""

from __future__ import annotations

import httpx


class HttpRangeReader:
    """obspec-compatible GetRangeAsync/GetRangesAsync backend.

    Reads byte ranges from a single, fixed URL via plain HTTP Range
    requests, following redirects on every request. `path` arguments are
    accepted (required by the obspec protocol / async-tiff's Rust glue) but
    ignored, since this backend always targets the one URL it was
    constructed with.
    """

    def __init__(self, url: str, client: httpx.AsyncClient | None = None):
        self.url = url
        self._client = client or httpx.AsyncClient(follow_redirects=True, timeout=30)

    async def get_range_async(self, path: str, start: int, end: int) -> bytes:
        resp = await self._client.get(
            self.url, headers={"Range": f"bytes={start}-{end - 1}"}
        )
        resp.raise_for_status()
        return resp.content

    async def get_ranges_async(
        self, path: str, starts: list[int], ends: list[int]
    ) -> list[bytes]:
        return [
            await self.get_range_async(path, s, e) for s, e in zip(starts, ends)
        ]
