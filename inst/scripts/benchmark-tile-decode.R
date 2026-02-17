#!/usr/bin/env Rscript
## benchmark-tile-decode.R
##
## Compare three paths for reading a single tile from a cloud COG:
##
##   A) rustycogs::tiff_tile()  – async-tiff direct decode
##   B) GDAL /vsicurl/          – standard GDAL remote read
##   C) GDAL via Kerchunk JSON  – scan once, read via GDAL Kerchunk driver
##
## Requirements:
##   - rustycogs installed
##   - terra (or vapour) for GDAL reads
##   - arrow for parquet (optional)

library(rustycogs)

## --- Test files ---------------------------------------------------------

## Australian Bathymetry 2023 (HTTP, float32, deflate, EPSG:4326)
url_bathy <- "https://files.ausseabed.gov.au/survey_data/AU_BA_2023_cog.tif"

## Sentinel-2 B04 (S3, uint16, deflate, UTM)
url_s2 <- "s3://sentinel-cogs/sentinel-s2-l2a-cogs/12/S/YJ/2023/8/S2B_12SYJ_20230801_0_L2A/B04.tif"


cat("=== rustycogs tile decode benchmark ===\n\n")

## --- Path A: rustycogs direct decode ------------------------------------

cat("--- A) rustycogs::tiff_tile() ---\n")

# First call includes TIFF open + metadata read
t_a1 <- system.time({
  tile <- tiff_tile(url_bathy, ifd_index = 0L, col = 0L, row = 0L)
})
cat(sprintf("  Bathy tile (0,0): %.3fs elapsed, dim=%s, dtype=%s\n",
            t_a1["elapsed"], paste(tile$dim, collapse = "x"), tile$dtype))
cat(sprintf("  range: [%.1f, %.1f]\n", min(tile$data, na.rm = TRUE),
            max(tile$data, na.rm = TRUE)))

# Multi-tile fetch
t_a2 <- system.time({
  tiles <- tiff_tiles(url_bathy, ifd_index = 0L,
                      cols = c(0L, 1L, 2L, 3L, 4L),
                      rows = c(0L, 0L, 0L, 0L, 0L))
})
cat(sprintf("  Bathy 5 tiles (row 0, cols 0-4): %.3fs elapsed\n",
            t_a2["elapsed"]))

# Overview tile
t_a3 <- system.time({
  tile_ov <- tiff_tile(url_bathy, ifd_index = 3L, col = 0L, row = 0L)
})
cat(sprintf("  Bathy overview IFD3 tile (0,0): %.3fs, dim=%s\n",
            t_a3["elapsed"], paste(tile_ov$dim, collapse = "x")))

## --- Path B: GDAL /vsicurl/ --------------------------------------------

cat("\n--- B) GDAL /vsicurl/ ---\n")

if (requireNamespace("terra", quietly = TRUE)) {
  vsicurl_url <- paste0("/vsicurl/", url_bathy)

  # Read same tile region: tile (0,0) at 512x512
  t_b1 <- system.time({
    r <- terra::rast(vsicurl_url)
    v <- terra::values(terra::crop(r, terra::ext(0, 512 * terra::res(r)[1],
                                                  terra::ymax(r) - 512 * terra::res(r)[2],
                                                  terra::ymax(r))))
  })
  cat(sprintf("  GDAL /vsicurl/ tile-equivalent: %.3fs elapsed, %d values\n",
              t_b1["elapsed"], length(v)))

  # For a fairer comparison: use terra::readValues with window
  t_b2 <- system.time({
    r2 <- terra::rast(vsicurl_url)
    block <- terra::values(r2, row = 1, nrows = 512, col = 1, ncols = 512)
  })
  cat(sprintf("  GDAL /vsicurl/ block read (512x512): %.3fs elapsed\n",
              t_b2["elapsed"]))
} else {
  cat("  (terra not available, skipping GDAL comparison)\n")
}

## --- Path C: Kerchunk JSON → GDAL --------------------------------------

cat("\n--- C) Kerchunk JSON → GDAL ---\n")

# Step 1: Scan references
t_c1 <- system.time({
  refs <- tiff_refs(url_bathy)
})
cat(sprintf("  Scan refs: %.3fs (%d tiles)\n", t_c1["elapsed"], nrow(refs)))

# Step 2: Write Kerchunk JSON
kj <- refs_to_kerchunk(refs)
kj_file <- tempfile(fileext = ".json")
writeLines(kj, kj_file)
cat(sprintf("  Kerchunk JSON: %d bytes → %s\n", nchar(kj), kj_file))

# Step 3: Try reading via GDAL Kerchunk driver
if (requireNamespace("terra", quietly = TRUE)) {
  # GDAL >= 3.8 has Kerchunk driver support
  # The driver name is "Zarr" with Kerchunk JSON as input
  t_c3 <- tryCatch({
    system.time({
      ## GDAL reads Kerchunk via ZARR driver:
      ## gdal_translate "ZARR:path/to/kerchunk.json" out.tif
      ## In R via terra, this may need explicit driver specification
      r3 <- terra::rast(paste0("ZARR:", kj_file))
      v3 <- terra::values(r3, row = 1, nrows = 512, col = 1, ncols = 512)
    })
  }, error = function(e) {
    cat(sprintf("  GDAL Kerchunk read failed: %s\n", e$message))
    cat("  (requires GDAL >= 3.8 with Zarr/Kerchunk driver)\n")
    NULL
  })
  if (!is.null(t_c3)) {
    cat(sprintf("  GDAL via Kerchunk: %.3fs elapsed\n", t_c3["elapsed"]))
  }
} else {
  cat("  (terra not available, skipping GDAL Kerchunk test)\n")
}

## --- Summary ------------------------------------------------------------

cat("\n=== Summary ===\n")
cat(sprintf("  rustycogs single tile:     %.3fs\n", t_a1["elapsed"]))
cat(sprintf("  rustycogs 5 tiles (batch): %.3fs (%.3fs per tile)\n",
            t_a2["elapsed"], t_a2["elapsed"] / 5))
if (exists("t_b2")) {
  cat(sprintf("  GDAL /vsicurl/ block read: %.3fs\n", t_b2["elapsed"]))
}
if (exists("t_c3") && !is.null(t_c3)) {
  cat(sprintf("  GDAL via Kerchunk JSON:    %.3fs\n", t_c3["elapsed"]))
}
cat("\nDone.\n")
