#' Convert TIFF references to Kerchunk V1 JSON
#'
#' Takes the data frame from [tiff_refs()] and reshapes it into the Kerchunk
#' reference specification (version 1) as a list suitable for writing with
#' [jsonlite::write_json()].
#'
#' @param refs Data frame from [tiff_refs()].
#' @param var_name Name for the Zarr variable. Default `"data"`.
#' @return A list in Kerchunk V1 format.
#' @export
#' @examples
#' \dontrun{
#' refs <- tiff_refs("s3://bucket/file.tif", anon = TRUE)
#' kc <- refs_to_kerchunk(refs)
#' jsonlite::write_json(kc, "references.json", auto_unbox = TRUE)
#' }
refs_to_kerchunk <- function(refs, var_name = "data") {

  # .zgroup at root
  zgroup <- '{"zarr_format": 2}'

  # Build .zarray from first row metadata
  r1 <- refs[1, ]
  chunks <- c(as.integer(r1$tile_h), as.integer(r1$tile_w))
  shape  <- c(as.integer(r1$image_h), as.integer(r1$image_w))

  compressor <- tiff_compression_to_zarr(r1$compression)

  zarray <- jsonlite::toJSON(list(
    zarr_format = 2L,
    shape = shape,
    chunks = chunks,
    dtype = r1$dtype,
    compressor = compressor,
    fill_value = 0L,
    order = "C",
    filters = NULL
  ), auto_unbox = TRUE)

  # Build refs: "var/row.col" -> [path, offset, length]
  ref_list <- list(
    ".zgroup" = zgroup
  )
  ref_list[[paste0(var_name, "/.zarray")]] <- as.character(zarray)

  for (i in seq_len(nrow(refs))) {
    key <- sprintf("%s/%d.%d", var_name, refs$tile_row[i], refs$tile_col[i])
    ref_list[[key]] <- list(refs$path[i], refs$offset[i], refs$length[i])
  }

  list(version = 1L, refs = ref_list)
}


#' Map TIFF compression tag to Zarr compressor spec
#'
#' @param code Integer TIFF compression tag value.
#' @return A list describing the Zarr compressor, or NULL for no compression.
#' @keywords internal
tiff_compression_to_zarr <- function(code) {
  # TIFF compression tag values:
  # 1 = None, 5 = LZW, 7 = JPEG, 8 = Deflate/zlib,
  # 32773 = PackBits, 34712 = JPEG2000, 50000 = Zstd
  switch(as.character(code),
    "1"     = NULL,
    "8"     = list(id = "zlib", level = 6L),
    "32946" = list(id = "zlib", level = 6L),  # Deflate (Adobe)
    "5"     = list(id = "lzw"),  # Note: not standard in zarr-python numcodecs
    "7"     = list(id = "jpeg"),
    "50000" = list(id = "zstd", level = 3L),
    # Default: report as raw/null and let downstream handle it
    NULL
  )
}
