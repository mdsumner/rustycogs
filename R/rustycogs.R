#' Extract tile byte-range references from TIFF/COG files
#'
#' Scans one or more TIFF or COG files and returns a data frame with one row
#' per tile per IFD, containing the byte-range metadata needed to construct
#' Kerchunk or Zarr virtual stores.
#'
#' @param paths Character vector of file paths or URLs.
#'   Supported schemes: `s3://`, `gs://`, `az://`, `http://`, `https://`,
#'   or plain local paths.
#' @param region Optional AWS region string (e.g. `"ap-southeast-2"`).
#'   Ignored for non-S3 URLs.
#' @param anon Logical. Use anonymous/unsigned requests. Default `FALSE`.
#' @param concurrency Integer. Maximum number of files scanned concurrently.
#'   Default `16L`.
#' @return A data frame with columns: `path`, `ifd`, `tile_col`, `tile_row`,
#'   `offset`, `length`, `image_w`, `image_h`, `tile_w`, `tile_h`, `dtype`,
#'   `compression`, `bits_per_sample`, `samples_per_pixel`, `crs_epsg`,
#'   `gdal_nodata`, `planar_configuration`, `scale_x`, `scale_y`,
#'   `origin_x`, `origin_y`.
#' @export
#' @seealso [tiff_ifd_info()] for a compact one-row-per-IFD summary,
#'   [tiff_read_tiles()] to fetch pixel data
#' @examples
#' \dontrun{
#' refs <- tiff_refs("s3://my-bucket/data.tif", anon = TRUE)
#' refs <- tiff_refs(c("file1.tif", "file2.tif"), concurrency = 4L)
#' }
tiff_refs <- function(paths, region = NULL, anon = FALSE, concurrency = 16L) {
  paths <- vapply(paths, function(p) if (grepl("://", p, fixed = TRUE)) p else path.expand(p), character(1L))
  .Call(wrap__tiff_refs, paths, region, anon, as.integer(concurrency))
}

#' Summarise IFD-level metadata from TIFF/COG files
#'
#' Returns one row per IFD per file with no tile explosion. Use this to
#' understand file structure — how many IFDs (overviews), tile grid dimensions,
#' dtype, CRS, geotransform — before deciding which tiles to fetch with
#' [tiff_refs()] or [tiff_read_tiles()].
#'
#' @param paths Character vector of file paths or URLs.
#' @param region Optional AWS region string (e.g. `"ap-southeast-2"`).
#' @param anon Logical. Use anonymous/unsigned requests. Default `FALSE`.
#' @param concurrency Integer. Maximum concurrent file scans. Default `16L`.
#' @return A data frame with one row per IFD and columns: `path`, `ifd`,
#'   `is_tiled`, `image_w`, `image_h`, `tile_w`, `tile_h`, `n_tiles_x`,
#'   `n_tiles_y`, `dtype`, `compression`, `bits_per_sample`,
#'   `samples_per_pixel`, `photometric`, `predictor`,
#'   `planar_configuration`, `crs_epsg`, `gdal_nodata`, `scale_x`,
#'   `scale_y`, `origin_x`, `origin_y`.
#' @export
#' @seealso [tiff_refs()] for the full per-tile data frame
#' @examples
#' \dontrun{
#' tiff_ifd_info("scene.tif")
#' tiff_ifd_info("s3://my-bucket/data.tif", anon = TRUE)
#' }
tiff_ifd_info <- function(paths, region = NULL, anon = FALSE, concurrency = 16L) {
  paths <- vapply(paths, function(p) if (grepl("://", p, fixed = TRUE)) p else path.expand(p), character(1L))
  .Call(wrap__tiff_ifd_info, paths, region, anon, as.integer(concurrency))
}

#' Fetch and decode tiles described by a refs data frame
#'
#' Takes a subset of the data frame from [tiff_refs()] and returns it
#' with a `data` list-column added. Each element of `data` is a numeric
#' vector of decoded pixel values in row-major order, suitable for
#' passing to [tile_to_array()].
#'
#' Files are opened once each and tiles fetched in a single vectorized
#' range request per file. Row order of the input is preserved.
#'
#' @param refs A data frame from [tiff_refs()], or a subset of one.
#' @param region Optional AWS region string. Default `NULL`.
#' @param anon Logical. Use anonymous requests. Default `FALSE`.
#' @param concurrency Integer. Max concurrent files. Default `16L`.
#' @return `refs` with a `data` list-column appended.
#' @export
#' @seealso [tiff_refs()], [tile_to_array()]
#' @examples
#' \dontrun{
#' refs <- tiff_refs("scene.tif")
#' refs <- tiff_read_tiles(refs)
#' arrays <- lapply(refs$data, tile_to_array)
#' }
tiff_read_tiles <- function(refs, region = NULL, anon = FALSE, concurrency = 16L) {
  paths <- vapply(refs$path, function(p) if (grepl("://", p, fixed = TRUE)) p else path.expand(p), character(1L))
  refs$data <- .Call(
    wrap__tiff_read_tiles,
    paths,
    refs$ifd,
    refs$tile_col,
    refs$tile_row,
    region,
    anon,
    as.integer(concurrency)
  )
  refs
}

#' Fetch and decode a single tile from a TIFF/COG file
#'
#' @param path File path or URL to the TIFF.
#' @param ifd_index IFD index (0-based). `0` is the full-resolution image.
#' @param col Tile column index (0-based).
#' @param row Tile row index (0-based).
#' @param region Optional AWS region string.
#' @param anon Logical. Use anonymous requests. Default `FALSE`.
#' @return A named list with:
#'   - `data`: numeric vector of decoded pixel values (row-major)
#'   - `dim`: integer vector `c(height, width, bands)`
#'   - `dtype`: character dtype string (e.g. `"<f4"`, `"<u2"`)
#' @export
#' @seealso [tile_to_array()] to reshape the result, [tiff_tiles()] for batches
#' @examples
#' \dontrun{
#' tile <- tiff_tile("scene.tif", col = 0L, row = 0L)
#' m <- tile_to_array(tile)
#' }
tiff_tile <- function(path, ifd_index = 0L, col, row, region = NULL, anon = FALSE) {
  path <- if (grepl("://", path)) path else path.expand(path)
  .Call(wrap__tiff_tile, path, as.integer(ifd_index),
        as.integer(col), as.integer(row), region, anon)
}

#' Fetch and decode multiple tiles from a TIFF/COG file
#'
#' Fetches a batch of tiles from a single file in one vectorized request.
#' More efficient than calling [tiff_tile()] in a loop. For multi-file
#' batches use [tiff_read_tiles()].
#'
#' @param path File path or URL to the TIFF.
#' @param ifd_index IFD index (0-based). Default `0L`.
#' @param cols Integer vector of tile column indices (0-based).
#' @param rows Integer vector of tile row indices (0-based). Must be the same
#'   length as `cols`.
#' @param region Optional AWS region string.
#' @param anon Logical. Use anonymous requests. Default `FALSE`.
#' @return A list of tile results, each with `data`, `dim`, and `dtype`
#'   components as described in [tiff_tile()].
#' @export
#' @seealso [tile_to_array()] to reshape individual results,
#'   [tiff_read_tiles()] for multi-file batches
#' @examples
#' \dontrun{
#' tiles <- tiff_tiles("scene.tif", cols = 0:3, rows = rep(0L, 4))
#' arrays <- lapply(tiles, tile_to_array)
#' }
tiff_tiles <- function(path, ifd_index = 0L, cols, rows, region = NULL, anon = FALSE) {
  path <- if (grepl("://", path)) path else path.expand(path)
  .Call(wrap__tiff_tiles, path, as.integer(ifd_index),
        as.integer(cols), as.integer(rows), region, anon)
}

#' Convert a tile result to a matrix or array
#'
#' Reshapes the flat numeric vector returned by [tiff_tile()], each element
#' of [tiff_tiles()], or each element of the `data` column from
#' [tiff_read_tiles()] into a matrix (single band) or 3D array (multi-band).
#'
#' The matrix is filled row-by-row (`byrow = TRUE`), consistent with the
#' row-major (C) order returned by async-tiff and expected by
#' [graphics::rasterImage()] and `ximage()`. Note that `as.vector(m)` on the
#' result does **not** recover the original input order; use `as.vector(t(m))`
#' for a round-trip.
#'
#' @param tile A tile result list from [tiff_tile()], [tiff_tiles()], or
#'   an element of the `data` column from [tiff_read_tiles()].
#' @return A matrix for single-band tiles, or a 3D array with dimensions
#'   `[height, width, bands]` for multi-band tiles.
#' @export
#' @examples
#' \dontrun{
#' tile <- tiff_tile("scene.tif", col = 0L, row = 0L)
#' m <- tile_to_array(tile)
#' dim(m)  # c(256, 256) for a single-band 256x256 tile
#' ximage::ximage(m)
#' }
tile_to_array <- function(tile) {
  d <- tile$dim
  if (length(d) == 3L && d[3L] > 1L) {
    array(tile$data, dim = d)
  } else {
    matrix(tile$data, nrow = d[1L], ncol = d[2L], byrow = TRUE)
  }
}
