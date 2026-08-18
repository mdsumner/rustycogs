#' references as tiles
#'
#' Add extent information xmin,xmax,ymin,ymax to refs table
#'
#' @param x file path or url
#'
#' @returns dataframe with xmin,xmax,ymin,ymax add to the tiffs_refs output
#' @export
#' @seealso tiles_to_matrix
#' @examples
refs_tiles <- function(x) {
  refs <- tiff_refs(x)
  info <-  tiff_ifd_info(x)

  # Get IFD-0 geo params
  ifd0 <- subset(info, ifd == 0)
  scale0_x <- ifd0$scale_x
  scale0_y <- ifd0$scale_y  # positive, Y flips below
  origin0_x  <- ifd0$origin_x
  origin0_y  <- ifd0$origin_y

  # Per-tile extents in refs
  refs$pixel_size_x <- scale0_x * ifd0$image_w / refs$image_w
  refs$pixel_size_y <- scale0_y * ifd0$image_h / refs$image_h
  refs$xmin <- origin0_x + refs$tile_col * refs$tile_w * refs$pixel_size_x
  refs$xmax <- origin0_x + (refs$tile_col * refs$tile_w + refs$valid_w) * refs$pixel_size_x
  refs$ymax <-  origin0_y - refs$tile_row * refs$tile_h * refs$pixel_size_y
  refs$ymin <-  origin0_y - (refs$tile_row * refs$tile_h + refs$valid_h) * refs$pixel_size_y
  refs
}

#' Turn already-read data into a full matrix (from tiff_read)
#'
#' @param refs
#'
#' @returns
#' @export
#' @seealso refs_tiles
#' @examples
tiles_to_matrix <- function(refs) {
  # assumes refs is filtered to a single ifd, single path
  # and refs$data is already populated (list of numeric vecs)

  tile_w <- refs$tile_w[1]
  tile_h <- refs$tile_h[1]

  # output matrix dimensions from the grid extent in refs
  col_range <- range(refs$tile_col)
  row_range <- range(refs$tile_row)

  # number of tiles in each direction
  n_col_tiles <- col_range[2] - col_range[1] + 1L
  n_row_tiles <- row_range[2] - row_range[1] + 1L

  # total output pixels — interior tiles are full, edges use valid_w/valid_h
  # sum unique col contributions
  out_w <- sum(tapply(refs$valid_w, refs$tile_col, max))
  out_h <- sum(tapply(refs$valid_h, refs$tile_row, max))

  out <- matrix(NA_real_, nrow = out_h, ncol = out_w)

  # cumulative pixel offsets for each tile_col and tile_row
  col_offset <- c(0L, cumsum(tapply(refs$valid_w, refs$tile_col, max))[-n_col_tiles])
  row_offset <- c(0L, cumsum(tapply(refs$valid_h, refs$tile_row, max))[-n_row_tiles])
  names(col_offset) <- as.character(sort(unique(refs$tile_col)))
  names(row_offset) <- as.character(sort(unique(refs$tile_row)))

  for (i in seq_len(nrow(refs))) {
    tc <- as.character(refs$tile_col[i])
    tr <- as.character(refs$tile_row[i])
    vw <- refs$valid_w[i]
    vh <- refs$valid_h[i]

    m <- matrix(refs$data[[i]], nrow = tile_h, ncol = tile_w, byrow = TRUE)
    m <- m[seq_len(vh), seq_len(vw), drop = FALSE]  # crop to valid

    col_idx <- col_offset[tc] + seq_len(vw)
    row_idx <- row_offset[tr] + seq_len(vh)
    out[row_idx, col_idx] <- m
  }

  out
}
