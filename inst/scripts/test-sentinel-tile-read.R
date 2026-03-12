library(rustycogs)
#url <- "https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/sentinel-2-c1-l2a/55/G/CM/2026/2/S2C_T55GCM_20260227T000650_L2A/B04.tif"
url <- "https://e84-earth-search-sentinel-data.s3.us-west-2.amazonaws.com/sentinel-2-c1-l2a/55/G/DN/2026/2/S2C_T55GDN_20260227T000650_L2A/B04.tif"
ifd <- tiff_ifd_info(url) |> dplyr::filter(ifd == 0)
ifd
source("https://gist.githubusercontent.com/mdsumner/c6ad59afb600b10c5ff602693c40e65e/raw/6a8d582e774d4d87c68ab2c42bb99f0caf312a9c/stretch_funs.R")
refs <- tiff_refs(url) |> dplyr::right_join(ifd |> dplyr::select(ifd), "ifd")
library(terra)
refs <- refs |> dplyr::filter(tile_row >= (max(tile_row) - 2),   tile_col >= (max(tile_col) - 2) )
dim(refs)
library(ximage)
par(mfrow = n2mfrow(nrow(refs)))
for (i in 1:nrow(refs)) {
  x <- rustycogs::tiff_read_tiles(refs[i, ])

  plot(s2_stretch_linear(terra::rast(matrix(x$data[[1]], x$tile_w, byrow = T))))
  title(sprintf("col: %i, row: %i", x$tile_col, x$tile_row), line = 3)
}


