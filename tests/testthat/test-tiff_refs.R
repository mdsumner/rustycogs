test_that("tiff_refs returns a data.frame", {
  # Test with empty input
  result <- tiff_refs(character(0))
  expect_s3_class(result, "data.frame")
  expect_equal(nrow(result), 0)
  expect_true("path" %in% names(result))
  expect_true("offset" %in% names(result))
  expect_true("length" %in% names(result))
})

test_that("refs_to_kerchunk produces valid structure", {
  # Minimal synthetic refs data.frame
  refs <- data.frame(
    path = rep("s3://bucket/file.tif", 4),
    ifd = rep(0L, 4),
    tile_col = c(0L, 1L, 0L, 1L),
    tile_row = c(0L, 0L, 1L, 1L),
    offset = c(1024, 2048, 3072, 4096),
    length = c(512, 512, 512, 512),
    image_w = rep(512L, 4),
    image_h = rep(512L, 4),
    tile_w = rep(256L, 4),
    tile_h = rep(256L, 4),
    dtype = rep("|u1", 4),
    compression = rep("Deflate", 4),
    bits_per_sample = rep(8L, 4),
    samples_per_pixel = rep(1L, 4),
    crs_epsg = rep(32618L, 4),
    stringsAsFactors = FALSE
  )

  kc <- refs_to_kerchunk(refs)
  expect_equal(kc$version, 1L)
  expect_type(kc$refs, "list")
  expect_true(".zgroup" %in% names(kc$refs))
  expect_true("data/.zarray" %in% names(kc$refs))
  expect_true("data/0.0" %in% names(kc$refs))
  expect_true("data/1.1" %in% names(kc$refs))

  # Each tile ref should be [path, offset, length]
  tile_ref <- kc$refs[["data/0.0"]]
  expect_length(tile_ref, 3)
  expect_equal(tile_ref[[1]], "s3://bucket/file.tif")
})
