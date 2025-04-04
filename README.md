
<!-- README.md is generated from README.Rmd. Please edit that file -->

# rustycogs

<!-- badges: start -->
<!-- badges: end -->

The goal of rustycogs is … another attempt learning to wrap Rust.

Very very WIP. Using grout and gdalraster for now to get the metadata so
we can sanely plot the tile bytes.

Obviously need some checks on the actual block size …

``` r
## gdal_translate /vsicurl/https://projects.pawsey.org.au/idea-gebco-tif/GEBCO_2024.tif?ovr=0 -outsize 4320 2160 -co TILED=YES gebco_4320.tif

dsn <- "/tiff/gebco_4320.tif"
devtools::load_all()
tiles <- tile_scheme(dsn)

plot(range(c(tiles$xmin, tiles$xmax)), range(c(tiles$ymin, tiles$ymax)))


for (i in seq_along(tiles$tile)) {
tx <- tiles[i, ]
size <- unlist(tx[c("ncol", "nrow")]) |>  as.integer()
tile_idx <- unlist(tx[c("tile_col", "tile_row")]) |> as.integer()
bytes <- rusty(dsn, tile = tile_idx - 1)  ## 0-based internally, 1-based in grout

ximage::ximage(matrix(readBin(bytes, "integer", size = 2L, n = prod(size)), size[2L], byrow = TRUE), 
               unlist(tx[c("xmin", "xmax", "ymin", "ymax")]), col = hcl.colors(24), add = TRUE)

}
```

<figure>
<img src="Rplot.png" title="Title" alt="alt text" />
<figcaption aria-hidden="true">alt text</figcaption>
</figure>

Is it really worth it? We automatically get type-conversion via GDAL …
though I guess we have spinup overhead for every read in the Rust, which
is probably the differnce

``` r
dsn <- "/tiff/gebco_4320.tif"
devtools::load_all()
tiles <- tile_scheme(dsn)

plot(range(c(tiles$xmin, tiles$xmax)), range(c(tiles$ymin, tiles$ymax)))

system.time({
for (i in seq_along(tiles$tile)) {
  tx <- tiles[i, ]
  size <- unlist(tx[c("ncol", "nrow")]) |>  as.integer()
  tile_idx <- unlist(tx[c("tile_col", "tile_row")]) |> as.integer()
  bytes <- rusty(dsn, tile = tile_idx - 1)  ## 0-based internally, 1-based in grout
}
})
# user  system elapsed 
# 0.593   0.684   1.368

system.time({
  ds <- new(gdalraster::GDALRaster, dsn)
  for (i in seq_along(tiles$tile)) {
    tx <- tiles[i, ]
    size <- unlist(tx[c("ncol", "nrow")]) |>  as.integer()
    tile_offset <- unlist(tx[c("offset_x", "offset_y")]) |> as.integer()
    values <- ds$read(band = 1L, xoff = tile_offset[1L], yoff = tile_offset[2L], 
                      xsize = size[1L], ysize = size[2L], 
                      out_xsize = size[1L], out_ysize = size[2L])
                      plot(values); scan("", 1)
    
  }
})

#user  system elapsed 
#0.056   0.000   0.056 
```

Do it with spinup for every read, so yes we can compete with GDAL if we
keep the reader open in Rust ;). Not sure I get the compulsion though,
because we can parallelize the read with GDAL too and not have to
rewrite everything.

``` r
system.time({
  for (i in seq_along(tiles$tile)) {
    tx <- tiles[i, ]
    size <- unlist(tx[c("ncol", "nrow")]) |>  as.integer()
    tile_offset <- unlist(tx[c("offset_x", "offset_y")]) |> as.integer()
    values <- vapour::vapour_read_raster(dsn, window = c(tile_offset[1L], tile_offset[2L], size))
    

  }
})
user  system elapsed 
1.010   0.020   1.029
```
