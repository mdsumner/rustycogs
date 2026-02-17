use extendr_api::prelude::*;
use std::sync::Arc;
use tokio::runtime::Runtime;
use futures::stream::{self, StreamExt};

// ── Store construction ──────────────────────────────────────────────
//
// Determine which object_store backend to use from the URL scheme.
// Returns (store, path_in_store) so we can open the TIFF via ObjectReader.
//
// Uses std::result::Result explicitly because extendr's Result<T> is
// aliased to std::result::Result<T, extendr_api::Error> (single generic).

fn make_store_and_path(
    url_str: &str,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<(Arc<dyn object_store::ObjectStore>, object_store::path::Path), String> {
    // Local file path (no scheme or file://)
    if !url_str.contains("://") || url_str.starts_with("file://") {
        let path = if url_str.starts_with("file://") {
            url_str.strip_prefix("file://").unwrap()
        } else {
            url_str
        };
        let parent = std::path::Path::new(path)
            .parent()
            .ok_or_else(|| format!("Cannot determine parent directory for: {}", path))?;
        let filename = std::path::Path::new(path)
            .file_name()
            .ok_or_else(|| format!("Cannot determine filename for: {}", path))?
            .to_str()
            .ok_or_else(|| "Non-UTF8 filename".to_string())?;
        let store = object_store::local::LocalFileSystem::new_with_prefix(parent)
            .map_err(|e| format!("LocalFileSystem error: {}", e))?;
        let obj_path = object_store::path::Path::from(filename);
        return Ok((Arc::new(store), obj_path));
    }

    let parsed = url::Url::parse(url_str).map_err(|e| format!("URL parse error: {}", e))?;

    match parsed.scheme() {
        "s3" | "s3a" => {
            let bucket = parsed.host_str().ok_or_else(|| "Missing S3 bucket in URL".to_string())?;
            let key = parsed.path().trim_start_matches('/');
            let mut builder = object_store::aws::AmazonS3Builder::new()
                .with_bucket_name(bucket);
            if let Some(r) = region {
                builder = builder.with_region(r);
            }
            if anon {
                builder = builder.with_skip_signature(true);
            }
            let store = builder.build().map_err(|e| format!("S3 build error: {}", e))?;
            let obj_path = object_store::path::Path::from(key);
            Ok((Arc::new(store), obj_path))
        }
        "gs" | "gcs" => {
            let bucket = parsed.host_str().ok_or_else(|| "Missing GCS bucket in URL".to_string())?;
            let key = parsed.path().trim_start_matches('/');
            let builder = object_store::gcp::GoogleCloudStorageBuilder::new()
                .with_bucket_name(bucket);
            // GCS anonymous access: object_store handles this via credential
            // chain — if no credentials are configured, it falls through.
            // No explicit with_anonymous() method on GCS builder.
            let store = builder.build().map_err(|e| format!("GCS build error: {}", e))?;
            let obj_path = object_store::path::Path::from(key);
            Ok((Arc::new(store), obj_path))
        }
        "az" | "abfs" | "abfss" => {
            let container = parsed.host_str().ok_or_else(|| "Missing Azure container in URL".to_string())?;
            let key = parsed.path().trim_start_matches('/');
            let builder = object_store::azure::MicrosoftAzureBuilder::new()
                .with_container_name(container);
            let store = builder.build().map_err(|e| format!("Azure build error: {}", e))?;
            let obj_path = object_store::path::Path::from(key);
            Ok((Arc::new(store), obj_path))
        }
        "http" | "https" => {
            let base = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
            let key = parsed.path().trim_start_matches('/');
            let builder = object_store::http::HttpBuilder::new()
                .with_url(&base);
            let store = builder.build().map_err(|e| format!("HTTP build error: {}", e))?;
            let obj_path = object_store::path::Path::from(key);
            Ok((Arc::new(store), obj_path))
        }
        scheme => Err(format!("Unsupported URL scheme: {}", scheme)),
    }
}

// ── IFD scanning ────────────────────────────────────────────────────

/// Columns that we build up across all files and IFDs
struct RefColumns {
    path: Vec<String>,
    ifd: Vec<i32>,
    tile_col: Vec<i32>,
    tile_row: Vec<i32>,
    offset: Vec<f64>,   // f64 because R doesn't have native u64
    length: Vec<f64>,   // same
    image_w: Vec<i32>,
    image_h: Vec<i32>,
    tile_w: Vec<i32>,
    tile_h: Vec<i32>,
    dtype: Vec<String>,
    compression: Vec<String>,    // Debug string of CompressionMethod enum
    bits_per_sample: Vec<i32>,
    samples_per_pixel: Vec<i32>,
    crs_epsg: Vec<i32>,  // i32::MIN signals NA_integer_
}

impl RefColumns {
    fn new() -> Self {
        Self {
            path: Vec::new(),
            ifd: Vec::new(),
            tile_col: Vec::new(),
            tile_row: Vec::new(),
            offset: Vec::new(),
            length: Vec::new(),
            image_w: Vec::new(),
            image_h: Vec::new(),
            tile_w: Vec::new(),
            tile_h: Vec::new(),
            dtype: Vec::new(),
            compression: Vec::new(),
            bits_per_sample: Vec::new(),
            samples_per_pixel: Vec::new(),
            crs_epsg: Vec::new(),
        }
    }

    fn extend(&mut self, other: RefColumns) {
        self.path.extend(other.path);
        self.ifd.extend(other.ifd);
        self.tile_col.extend(other.tile_col);
        self.tile_row.extend(other.tile_row);
        self.offset.extend(other.offset);
        self.length.extend(other.length);
        self.image_w.extend(other.image_w);
        self.image_h.extend(other.image_h);
        self.tile_w.extend(other.tile_w);
        self.tile_h.extend(other.tile_h);
        self.dtype.extend(other.dtype);
        self.compression.extend(other.compression);
        self.bits_per_sample.extend(other.bits_per_sample);
        self.samples_per_pixel.extend(other.samples_per_pixel);
        self.crs_epsg.extend(other.crs_epsg);
    }
}

// Format SampleFormat + bits_per_sample into a NumPy/Zarr dtype string
fn format_dtype(sample_format: &[async_tiff::tags::SampleFormat], bps: &[u16]) -> String {
    use async_tiff::tags::SampleFormat;

    let fmt = sample_format.first().copied()
        .unwrap_or(SampleFormat::Uint);
    let bits = bps.first().copied().unwrap_or(8);
    let bytes = (bits + 7) / 8;

    let prefix = if bytes == 1 { "|" } else { "<" };
    let code = match fmt {
        SampleFormat::Uint => "u",
        SampleFormat::Int => "i",
        // Catch-all covers IEEEFloatingPoint / Float / any other variant
        _ => "f",
    };
    format!("{}{}{}", prefix, code, bytes)
}

async fn scan_one_file(
    url_str: &str,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<RefColumns, String> {
    let (store, obj_path) = make_store_and_path(url_str, region, anon)?;

    let reader = async_tiff::reader::ObjectReader::new(store, obj_path);
    let cache = async_tiff::metadata::cache::ReadaheadMetadataCache::new(reader.clone());
    let mut meta_reader = async_tiff::metadata::TiffMetadataReader::try_open(&cache)
        .await
        .map_err(|e| format!("Failed to open TIFF {}: {}", url_str, e))?;
    let ifds = meta_reader.read_all_ifds(&cache)
        .await
        .map_err(|e| format!("Failed to read IFDs from {}: {}", url_str, e))?;
    let tiff = async_tiff::TIFF::new(ifds, meta_reader.endianness());

    let mut cols = RefColumns::new();

    for (ifd_idx, ifd) in tiff.ifds().iter().enumerate() {
        let img_w = ifd.image_width();
        let img_h = ifd.image_height();

        // tile_width() / tile_height() return Option<u32>.
        // None means strip-based IFD — skip it.
        let t_w = match ifd.tile_width() {
            Some(w) if w > 0 => w,
            _ => continue,
        };
        let t_h = match ifd.tile_height() {
            Some(h) if h > 0 => h,
            _ => continue,
        };

        let tiles_across = (img_w + t_w - 1) / t_w;
        let tiles_down = (img_h + t_h - 1) / t_h;

        // CompressionMethod is not a unit-only enum, use Debug formatting
        let compression_str = format!("{:?}", ifd.compression());

        let sample_fmt = ifd.sample_format();
        let bps = ifd.bits_per_sample();
        let spp = ifd.samples_per_pixel();
        let dtype_str = format_dtype(&sample_fmt, &bps);
        let bps_first = bps.first().copied().unwrap_or(8) as i32;

        // GeoTIFF EPSG — try projected_type first, fall back to geographic_type
        let epsg: i32 = ifd.geo_key_directory()
            .and_then(|geo| geo.projected_type.or(geo.geographic_type))
            .map(|code| code as i32)
            .unwrap_or(i32::MIN);

        // tile_offsets() / tile_byte_counts() return Option<&[u64]>
        let offsets = match ifd.tile_offsets() {
            Some(o) => o,
            None => continue,
        };
        let byte_counts = match ifd.tile_byte_counts() {
            Some(bc) => bc,
            None => continue,
        };

        let n_tiles = (tiles_across * tiles_down) as usize;
        for tile_idx in 0..n_tiles {
            let tc = (tile_idx as u32) % tiles_across;
            let tr = (tile_idx as u32) / tiles_across;

            let off = offsets.get(tile_idx).copied().unwrap_or(0);
            let len = byte_counts.get(tile_idx).copied().unwrap_or(0);

            cols.path.push(url_str.to_string());
            cols.ifd.push(ifd_idx as i32);
            cols.tile_col.push(tc as i32);
            cols.tile_row.push(tr as i32);
            cols.offset.push(off as f64);
            cols.length.push(len as f64);
            cols.image_w.push(img_w as i32);
            cols.image_h.push(img_h as i32);
            cols.tile_w.push(t_w as i32);
            cols.tile_h.push(t_h as i32);
            cols.dtype.push(dtype_str.clone());
            cols.compression.push(compression_str.clone());
            cols.bits_per_sample.push(bps_first);
            cols.samples_per_pixel.push(spp as i32);
            cols.crs_epsg.push(epsg);
        }
    }

    Ok(cols)
}

// ── Exported R function ─────────────────────────────────────────────

/// Extract tile byte-range references from TIFF/COG files.
///
/// @param paths Character vector of file paths or URLs
///   (s3://, gs://, az://, http://, https://, or local paths).
/// @param region Optional AWS region string (e.g. "us-west-2").
/// @param anon Logical, use anonymous/unsigned requests. Default FALSE.
/// @param concurrency Integer, max concurrent file scans. Default 16.
/// @return A data.frame with columns: path, ifd, tile_col, tile_row,
///   offset, length, image_w, image_h, tile_w, tile_h, dtype,
///   compression, bits_per_sample, samples_per_pixel, crs_epsg.
/// @export
#[extendr]
fn tiff_refs(paths: Strings, region: Nullable<String>, anon: bool, concurrency: i32) -> Robj {
    let region_str: Option<String> = match region {
        Nullable::NotNull(r) => Some(r),
        Nullable::Null => None,
    };
    let region_ref = region_str.as_deref();
    let conc = if concurrency < 1 { 1usize } else { concurrency as usize };

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            rprintln!("Error: Failed to create tokio runtime: {}", e);
            return data_frame!(
                path = Vec::<String>::new(),
                ifd = Vec::<i32>::new(),
                tile_col = Vec::<i32>::new(),
                tile_row = Vec::<i32>::new(),
                offset = Vec::<f64>::new(),
                length = Vec::<f64>::new(),
                image_w = Vec::<i32>::new(),
                image_h = Vec::<i32>::new(),
                tile_w = Vec::<i32>::new(),
                tile_h = Vec::<i32>::new(),
                dtype = Vec::<String>::new(),
                compression = Vec::<String>::new(),
                bits_per_sample = Vec::<i32>::new(),
                samples_per_pixel = Vec::<i32>::new(),
                crs_epsg = Vec::<Option<i32>>::new()
            );
        }
    };

    // Collect paths to owned Strings for the async tasks
    let owned_paths: Vec<String> = paths.iter().map(|p| p.as_str().to_string()).collect();

    let (cols, errors) = rt.block_on(async {
        let results: Vec<std::result::Result<RefColumns, String>> = stream::iter(owned_paths)
            .map(|path| {
                // Clone region into each future to avoid lifetime issues
                let region_owned = region_ref.map(|s| s.to_string());
                async move {
                    scan_one_file(&path, region_owned.as_deref(), anon).await
                }
            })
            .buffer_unordered(conc)
            .collect()
            .await;

        let mut merged = RefColumns::new();
        let mut errs = Vec::new();
        for result in results {
            match result {
                Ok(file_cols) => merged.extend(file_cols),
                Err(e) => errs.push(e),
            }
        }
        (merged, errs)
    });

    for e in &errors {
        rprintln!("Warning: {}", e);
    }

    let crs_epsg_r: Vec<Option<i32>> = cols.crs_epsg.iter()
        .map(|&v| if v == i32::MIN { None } else { Some(v) })
        .collect();

    data_frame!(
        path = cols.path,
        ifd = cols.ifd,
        tile_col = cols.tile_col,
        tile_row = cols.tile_row,
        offset = cols.offset,
        length = cols.length,
        image_w = cols.image_w,
        image_h = cols.image_h,
        tile_w = cols.tile_w,
        tile_h = cols.tile_h,
        dtype = cols.dtype,
        compression = cols.compression,
        bits_per_sample = cols.bits_per_sample,
        samples_per_pixel = cols.samples_per_pixel,
        crs_epsg = crs_epsg_r
    )
}

// ── Single tile fetch + decode ──────────────────────────────────────

/// Fetch and decode a single tile from a TIFF/COG file.
///
/// @param path File path or URL to the TIFF.
/// @param ifd_index IFD index (0-based). Default 0 (full resolution).
/// @param col Tile column (0-based).
/// @param row Tile row (0-based).
/// @param region Optional AWS region string.
/// @param anon Logical, use anonymous requests. Default FALSE.
/// @return A named list with components:
///   - `data`: numeric vector of decoded pixel values
///   - `dim`: integer vector c(height, width, bands)
///   - `dtype`: character string (e.g. "<f4", "<u2")
/// @export
#[extendr]
fn tiff_tile(
    path: &str,
    ifd_index: i32,
    col: i32,
    row: i32,
    region: Nullable<String>,
    anon: bool,
) -> Robj {
    let region_str: Option<String> = match region {
        Nullable::NotNull(r) => Some(r),
        Nullable::Null => None,
    };
    let region_ref = region_str.as_deref();

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            rprintln!("Error: Failed to create tokio runtime: {}", e);
            return list!(data = Robj::from(Rfloat::na()), dim = Robj::from(0), dtype = "").into();
        }
    };

    rt.block_on(async {
        match fetch_decode_tile(path, ifd_index as usize, col as usize, row as usize, region_ref, anon).await {
            Ok(tile_result) => tile_result,
            Err(e) => {
                rprintln!("Error: {}", e);
                list!(data = Robj::from(Rfloat::na()), dim = Robj::from(0), dtype = "").into()
            }
        }
    })
}

async fn fetch_decode_tile(
    url_str: &str,
    ifd_index: usize,
    col: usize,
    row: usize,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<Robj, String> {
    let (store, obj_path) = make_store_and_path(url_str, region, anon)?;

    let reader = async_tiff::reader::ObjectReader::new(store, obj_path);
    let cache = async_tiff::metadata::cache::ReadaheadMetadataCache::new(reader.clone());
    let mut meta_reader = async_tiff::metadata::TiffMetadataReader::try_open(&cache)
        .await
        .map_err(|e| format!("Failed to open TIFF: {}", e))?;
    let ifds = meta_reader.read_all_ifds(&cache)
        .await
        .map_err(|e| format!("Failed to read IFDs: {}", e))?;
    let tiff = async_tiff::TIFF::new(ifds, meta_reader.endianness());

    let ifd = tiff.ifds().get(ifd_index)
        .ok_or_else(|| format!("IFD index {} out of range (file has {} IFDs)", ifd_index, tiff.ifds().len()))?;

    // Fetch the compressed tile bytes
    let tile = ifd.fetch_tile(col, row, &reader)
        .await
        .map_err(|e| format!("Failed to fetch tile ({},{}): {}", col, row, e))?;

    // Decode the tile
    let array = tile.decode(&Default::default())
        .map_err(|e| format!("Failed to decode tile: {}", e))?;

    // Decompose via into_inner(): (TypedArray, [usize; 3], Option<DataType>)
    let (typed_data, shape, data_type) = array.into_inner();

    let dtype_str = match data_type {
        Some(async_tiff::DataType::UInt8) => "|u1",
        Some(async_tiff::DataType::UInt16) => "<u2",
        Some(async_tiff::DataType::UInt32) => "<u4",
        Some(async_tiff::DataType::UInt64) => "<u8",
        Some(async_tiff::DataType::Int8) => "|i1",
        Some(async_tiff::DataType::Int16) => "<i2",
        Some(async_tiff::DataType::Int32) => "<i4",
        Some(async_tiff::DataType::Int64) => "<i8",
        Some(async_tiff::DataType::Float32) => "<f4",
        Some(async_tiff::DataType::Float64) => "<f8",
        _ => "|u1",  // fallback: treat unknown/None/Bool as uint8
    };

    // Convert TypedArray to R numeric vector
    let data_vec: Vec<f64> = match typed_data {
        async_tiff::TypedArray::UInt8(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt16(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt32(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt64(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int8(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int16(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int32(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int64(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Float32(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Float64(arr) => arr.iter().copied().collect(),
    };

    let dim: Vec<i32> = shape.iter().map(|&d| d as i32).collect();

    Ok(list!(
        data = data_vec,
        dim = dim,
        dtype = dtype_str
    ).into())
}

/// Fetch and decode multiple tiles as a batch.
///
/// @param path File path or URL to the TIFF.
/// @param ifd_index IFD index (0-based). Default 0 (full resolution).
/// @param cols Integer vector of tile columns (0-based).
/// @param rows Integer vector of tile rows (0-based).
/// @param region Optional AWS region string.
/// @param anon Logical, use anonymous requests. Default FALSE.
/// @return A list of tile results, each with data, dim, and dtype.
/// @export
#[extendr]
fn tiff_tiles(
    path: &str,
    ifd_index: i32,
    cols: &[i32],
    rows: &[i32],
    region: Nullable<String>,
    anon: bool,
) -> Robj {
    let region_str: Option<String> = match region {
        Nullable::NotNull(r) => Some(r),
        Nullable::Null => None,
    };
    let region_ref = region_str.as_deref();

    if cols.len() != rows.len() {
        rprintln!("Error: cols and rows must have the same length");
        return list!().into();
    }

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            rprintln!("Error: Failed to create tokio runtime: {}", e);
            return list!().into();
        }
    };

    rt.block_on(async {
        // Open the TIFF once, fetch multiple tiles
        let result = fetch_decode_tiles_batch(
            path, ifd_index as usize, cols, rows, region_ref, anon
        ).await;

        match result {
            Ok(tiles) => {
                List::from_values(tiles).into_robj()
            }
            Err(e) => {
                rprintln!("Error: {}", e);
                list!().into()
            }
        }
    })
}

async fn fetch_decode_tiles_batch(
    url_str: &str,
    ifd_index: usize,
    cols: &[i32],
    rows: &[i32],
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<Vec<Robj>, String> {
    let (store, obj_path) = make_store_and_path(url_str, region, anon)?;

    let reader = async_tiff::reader::ObjectReader::new(store, obj_path);
    let cache = async_tiff::metadata::cache::ReadaheadMetadataCache::new(reader.clone());
    let mut meta_reader = async_tiff::metadata::TiffMetadataReader::try_open(&cache)
        .await
        .map_err(|e| format!("Failed to open TIFF: {}", e))?;
    let ifds = meta_reader.read_all_ifds(&cache)
        .await
        .map_err(|e| format!("Failed to read IFDs: {}", e))?;
    let tiff = async_tiff::TIFF::new(ifds, meta_reader.endianness());

    let ifd = tiff.ifds().get(ifd_index)
        .ok_or_else(|| format!("IFD index {} out of range", ifd_index))?;

    let mut results: Vec<Robj> = Vec::with_capacity(cols.len());

    // Fetch tiles — these could be made concurrent with join_all later
    for (c, r) in cols.iter().zip(rows.iter()) {
        let tile = ifd.fetch_tile(*c as usize, *r as usize, &reader)
            .await
            .map_err(|e| format!("Failed to fetch tile ({},{}): {}", c, r, e))?;

        let array = tile.decode(&Default::default())
            .map_err(|e| format!("Failed to decode tile ({},{}): {}", c, r, e))?;

        let (typed_data, shape, data_type) = array.into_inner();

        let dtype_str = match data_type {
            Some(async_tiff::DataType::UInt8) => "|u1",
            Some(async_tiff::DataType::UInt16) => "<u2",
            Some(async_tiff::DataType::UInt32) => "<u4",
            Some(async_tiff::DataType::UInt64) => "<u8",
            Some(async_tiff::DataType::Int8) => "|i1",
            Some(async_tiff::DataType::Int16) => "<i2",
            Some(async_tiff::DataType::Int32) => "<i4",
            Some(async_tiff::DataType::Int64) => "<i8",
            Some(async_tiff::DataType::Float32) => "<f4",
            Some(async_tiff::DataType::Float64) => "<f8",
            _ => "|u1",
        };

        let data_vec: Vec<f64> = match typed_data {
            async_tiff::TypedArray::UInt8(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::UInt16(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::UInt32(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::UInt64(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Int8(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Int16(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Int32(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Int64(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Float32(arr) => arr.iter().map(|&v| v as f64).collect(),
            async_tiff::TypedArray::Float64(arr) => arr.iter().copied().collect(),
        };

        let dim: Vec<i32> = shape.iter().map(|&d| d as i32).collect();

        results.push(list!(
            data = data_vec,
            dim = dim,
            dtype = dtype_str
        ).into());
    }

    Ok(results)
}

extendr_module! {
    mod rustycogs;
    fn tiff_refs;
    fn tiff_tile;
    fn tiff_tiles;
}
