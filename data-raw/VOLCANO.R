## code to prepare `VOLCANO` dataset goes here

#remotes::install_github("mdsumner/volcano")
library(volcano)
gdalraster::translate(volcano.tif(),
      "inst/extdata/volcano-cog.tif",
    cl_arg = c("-of", "COG",
               "-ot", "UInt8",
               "-r", "BILINEAR",
               "-co", "OVERVIEW_RESAMPLING=AVERAGE",
               "-co", "BLOCKSIZE=128",
               "-co", "COMPRESS=DEFLATE",
               "-outsize", c(61, 87) * 20))

