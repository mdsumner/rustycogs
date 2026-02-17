#' Convert tile data to a matrix or array
#'
#' @param tile A tile result from [tiff_tile()].
#' @return A matrix (single band) or 3D array (multi-band).
#' @export
tile_to_array <- function(tile) {
  d <- tile$dim
  if (length(d) == 3 && d[3] > 1) {
    array(tile$data, dim = d)
  } else {
    matrix(tile$data, nrow = d[1], ncol = d[2])
  }
}
