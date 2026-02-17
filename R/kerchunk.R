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

  # Build zarray list — use explicit null for no compressor so it
  # serializes as "compressor": null rather than being dropped
  zarray_list <- list(
    zarr_format = 2L,
    shape = shape,
    chunks = chunks,
    dtype = r1$dtype,
    fill_value = 0L,
    order = "C"
  )

  # compressor and filters need explicit handling for JSON null
  if (is.null(compressor)) {
    # Write manually to get "compressor": null
    zarray_json <- jsonlite::toJSON(zarray_list, auto_unbox = TRUE)
    # Insert compressor and filters fields
    zarray <- sub("\\}$",
      ',"compressor":null,"filters":null}',
      as.character(zarray_json))
  } else {
    zarray_list$compressor <- compressor
    zarray <- as.character(
      jsonlite::toJSON(c(zarray_list, list(filters = NULL)),
                       auto_unbox = TRUE, null = "null"))
  }

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
  # code is a string from Debug formatting of async-tiff's CompressionMethod enum
  # e.g. "Deflate", "Lzw", "Jpeg", "None", "Zstd"
  switch(tolower(code),
    "none"          = NULL,
    "uncompressed"  = NULL,
    "deflate"       = list(id = "zlib", level = 6L),
    "lzw"           = list(id = "lzw"),
    "jpeg"          = list(id = "jpeg"),
    "zstd"          = list(id = "zstd", level = 3L),
    "webp"          = list(id = "webp"),
    # Default: null and let downstream handle it
    NULL
  )
}
