## blocks must be 16x16 at minimum so
dm <- c(16, 16)
v <- seq_len(prod(dm))
m <- matrix(v, dm[2], dm[1], byrow = TRUE)

nc <- cbind(m, m)
library(terra)
r <- rast(rbind(nc, nc)[seq(1, 2 * dm[1] - 1), seq(1, 2 * dm[2] - 1)])
plot(r)
filename <- "deflate-16x16-crop-no-predictor.tif"

writeRaster(r, filename,
            gdal = c("BLOCKXSIZE=16", "BLOCKYSIZE=16", "TILED=YES", "COMPRESS=DEFLATE", "PREDICTOR=2"),
            filetype = "GTiff", overwrite = TRUE)

filename2 <- "deflate-16x16-crop-predictor=2.tif"
writeRaster(r, filename2,
            gdal = c("BLOCKXSIZE=16", "BLOCKYSIZE=16", "TILED=YES", "COMPRESS=DEFLATE", "PREDICTOR=2"),
            filetype = "GTiff", overwrite = TRUE)


f <- normalizePath(filename)
ifd <- rustycogs::tiff_ifd_info(f)
refs <- rustycogs::tiff_refs(f)

par(mfrow = n2mfrow(nrow(refs)))
l <- vector("list", nrow(refs))
for (i in seq_along(l)) {
l[[i]] <- rustycogs::tiff_read_tiles(refs[i, ], region = "", anon = TRUE, concurrency = 1L)$data[[1]]

ximage::ximage(matrix(l[[i]], dm[2], byrow = TRUE), zlim = c(0, 1024))
}





