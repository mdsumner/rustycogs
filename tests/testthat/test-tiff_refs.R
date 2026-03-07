# ── local / synthetic ─────────────────────────────────────────────────────────

test_that("tiff_refs returns a data.frame with correct schema on empty input", {
  result <- tiff_refs(character(0))
  expect_s3_class(result, "data.frame")
  expect_equal(nrow(result), 0L)
  expected_cols <- c(
    "path", "ifd", "tile_col", "tile_row", "offset", "length",
    "image_w", "image_h", "tile_w", "tile_h", "dtype", "compression",
    "bits_per_sample", "samples_per_pixel", "crs_epsg", "gdal_nodata",
    "planar_configuration", "scale_x", "scale_y", "origin_x", "origin_y"
  )
  expect_true(all(expected_cols %in% names(result)))
})

test_that("tiff_ifd_info returns a data.frame with correct schema on empty input", {
  result <- tiff_ifd_info(character(0))
  expect_s3_class(result, "data.frame")
  expect_equal(nrow(result), 0L)
  expect_true(all(c("ifd", "is_tiled", "n_tiles_x", "n_tiles_y",
                    "photometric", "predictor", "planar_configuration",
                    "gdal_nodata", "scale_x", "scale_y",
                    "origin_x", "origin_y") %in% names(result)))
})

test_that("refs_to_kerchunk produces valid structure, fill_value from gdal_nodata", {
  refs <- data.frame(
    path              = rep("s3://bucket/file.tif", 4),
    ifd               = rep(0L, 4),
    tile_col          = c(0L, 1L, 0L, 1L),
    tile_row          = c(0L, 0L, 1L, 1L),
    offset            = c(1024, 2048, 3072, 4096),
    length            = c(512L, 512L, 512L, 512L),
    image_w           = rep(512L, 4),
    image_h           = rep(512L, 4),
    tile_w            = rep(256L, 4),
    tile_h            = rep(256L, 4),
    dtype             = rep("|u1", 4),
    compression       = rep("Deflate", 4),
    bits_per_sample   = rep(8L, 4),
    samples_per_pixel = rep(1L, 4),
    crs_epsg          = rep(32618L, 4),
    gdal_nodata       = rep(NA_character_, 4),
    stringsAsFactors  = FALSE
  )

  kc <- refs_to_kerchunk(refs)
  expect_equal(kc$version, 1L)
  expect_type(kc$refs, "list")
  expect_true(".zgroup" %in% names(kc$refs))
  expect_true("data/.zarray" %in% names(kc$refs))
  expect_true("data/0.0" %in% names(kc$refs))
  expect_true("data/1.1" %in% names(kc$refs))

  tile_ref <- kc$refs[["data/0.0"]]
  expect_length(tile_ref, 3L)
  expect_equal(tile_ref[[1]], "s3://bucket/file.tif")

  # NA gdal_nodata -> fill_value 0
  zarray <- jsonlite::fromJSON(kc$refs[["data/.zarray"]])
  expect_equal(zarray$fill_value, 0)

  # gdal_nodata present -> fill_value used
  refs$gdal_nodata <- "-32767"
  kc2 <- refs_to_kerchunk(refs)
  zarray2 <- jsonlite::fromJSON(kc2$refs[["data/.zarray"]])
  expect_equal(zarray2$fill_value, -32767)
})

test_that("tile_to_array reshapes correctly", {
  tile <- list(data = seq_len(16L), dim = c(4L, 4L, 1L), dtype = "|u1")
  m <- tile_to_array(tile)
  expect_true(is.matrix(m))
  expect_equal(dim(m), c(4L, 4L))
  # byrow=TRUE: first row is 1:4
  expect_equal(m[1, ], 1:4)

  # multi-band
  tile3 <- list(data = seq_len(24L), dim = c(2L, 4L, 3L), dtype = "|u1")
  a <- tile_to_array(tile3)
  expect_equal(dim(a), c(2L, 4L, 3L))
})

# ── live network tests ────────────────────────────────────────────────────────
# Skipped on CRAN and when offline (LIVE_TESTS env var must be set).

skip_live <- function() {
  skip_on_cran()
  if (!identical(Sys.getenv("LIVE_TESTS"), "true")) {
    skip("set LIVE_TESTS=true to run network tests")
  }
}

test_that("GEBCO: Deflate/Int16/512 — tiff_ifd_info and tiff_refs", {
  skip_live()
  url <- "https://projects.pawsey.org.au/idea-gebco-tif/GEBCO_2024.tif"

  info <- tiff_ifd_info(url)
  expect_s3_class(info, "data.frame")
  expect_equal(info$ifd[1], 0L)
  expect_equal(info$dtype[1], "<i2")
  expect_equal(info$compression[1], "Deflate")
  expect_equal(info$tile_w[1], 512L)
  expect_equal(info$tile_h[1], 512L)
  expect_equal(info$crs_epsg[1], 4326L)
  expect_equal(info$gdal_nodata[1], "-32767")
  expect_equal(info$origin_x[1], -180)
  expect_equal(info$origin_y[1],   90)
  # 9 overview levels
  expect_equal(nrow(info), 9L)

  refs <- tiff_refs(url)
  # IFD 0: 169 x 85 tiles
  ifd0 <- refs[refs$ifd == 0L, ]
  expect_equal(max(ifd0$tile_col) + 1L, info$n_tiles_x[1])
  expect_equal(max(ifd0$tile_row) + 1L, info$n_tiles_y[1])

  # Fetch the single tile from the smallest overview (IFD 8: 337x168)
  smallest <- refs[refs$ifd == max(refs$ifd), ]
  smallest <- tiff_read_tiles(smallest)
  expect_true("data" %in% names(smallest))
  arr <- tile_to_array(smallest$data[[1]])
  expect_true(is.matrix(arr))
  # tile can be partial at image edge but must have pixels
  expect_gt(length(arr), 0L)
  # nodata is -32767, valid bathymetry should include values != -32767
  expect_true(any(arr != -32767L))
})

test_that("Sentinel-2: Deflate+Predictor2/UInt16/1024 — tiff_ifd_info and tile decode", {
  skip_live()
  url <- "https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/sentinel-2-c1-l2a/55/G/DN/2026/2/S2C_T55GDN_20260227T000650_L2A/B04.tif"

  info <- tiff_ifd_info(url, anon = TRUE)
  expect_s3_class(info, "data.frame")
  expect_equal(info$dtype[1], "<u2")
  expect_equal(info$compression[1], "Deflate")
  expect_equal(info$predictor[1], "Horizontal")
  expect_equal(info$tile_w[1], 1024L)
  expect_equal(nrow(info), 5L)   # 4 overviews + full res

  refs <- tiff_refs(url, anon = TRUE)
  ifd0 <- refs[refs$ifd == 0L, ]
  expect_equal(ifd0$compression[1], "Deflate")

  # Decode top-left tile of full resolution
  top_left <- ifd0[ifd0$tile_col == 0L & ifd0$tile_row == 0L, ]
  result <- tiff_read_tiles(top_left, anon = TRUE)
  m <- tile_to_array(result$data[[1]])
  expect_equal(dim(m), c(1024L, 1024L))
  expect_true(is.numeric(m))
})

test_that("GHRSST MUR: Zstd+Predictor2/Int16/512 — tiff_ifd_info and tile decode", {
  skip_live()
  url <- "https://data.source.coop/ausantarctic/ghrsst-mur-v2/2026/03/05/20260305090000-JPL-L4_GHRSST-SSTfnd-MUR-GLOB-v02.0-fv04.1_analysed_sst.tif"

  info <- tiff_ifd_info(url)
  expect_s3_class(info, "data.frame")
  expect_equal(info$dtype[1], "<i2")
  expect_equal(info$compression[1], "ZSTD")
  expect_equal(info$tile_w[1], 512L)
  expect_equal(nrow(info), 6L)   # 5 overviews + full res

  refs <- tiff_refs(url)
  ifd0 <- refs[refs$ifd == 0L, ]
  expect_equal(ifd0$compression[1], "ZSTD")

  # Smallest overview
  smallest <- refs[refs$ifd == max(refs$ifd), ]
  smallest <- tiff_read_tiles(smallest)
  arr <- tile_to_array(smallest$data[[1]])
  expect_true(is.matrix(arr))
  expect_gt(length(arr), 0L)
})
