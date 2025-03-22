tile_scheme <- function(dsn) {
  ds <- new(gdalraster::GDALRaster, dsn)
  on.exit(ds$close(), add = TRUE)
  ex <- ds$bbox()[c(1, 3, 2, 4)]
  dm <- ds$dim()
  bs <- ds$getBlockSize(1L)
  grout::tile_index(grout::grout(dm[1:2], ex, bs))
}

rusty <- function(file, tile = c(0L, 0L)) {
  tile <- as.integer(rep(tile, length.out = 2L))
  dir <- dirname(file)
  file <- basename(file)
  rusty_rs(file, dir, tile[1L], tile[2L])
}
