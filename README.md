
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
