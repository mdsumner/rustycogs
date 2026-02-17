use extendr_api::prelude::*;
use std::sync::Arc;
use tokio::runtime::Runtime;

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
    cols: &mut RefColumns,
) -> std::result::Result<(), String> {
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

    Ok(())
}

// ── Exported R function ─────────────────────────────────────────────

/// Extract tile byte-range references from TIFF/COG files.
///
/// @param paths Character vector of file paths or URLs
///   (s3://, gs://, az://, http://, https://, or local paths).
/// @param region Optional AWS region string (e.g. "us-west-2").
/// @param anon Logical, use anonymous/unsigned requests. Default FALSE.
/// @return A data.frame with columns: path, ifd, tile_col, tile_row,
///   offset, length, image_w, image_h, tile_w, tile_h, dtype,
///   compression, bits_per_sample, samples_per_pixel, crs_epsg.
/// @export
#[extendr]
fn tiff_refs(paths: Strings, region: Nullable<String>, anon: bool) -> Robj {
    let region_str: Option<String> = match region {
        Nullable::NotNull(r) => Some(r),
        Nullable::Null => None,
    };
    let region_ref = region_str.as_deref();

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

    let mut cols = RefColumns::new();

    let errors: Vec<String> = rt.block_on(async {
        let mut errs = Vec::new();
        for path in paths.iter() {
            if let Err(e) = scan_one_file(path.as_str(), region_ref, anon, &mut cols).await {
                errs.push(e);
            }
        }
        errs
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

extendr_module! {
    mod rustycogs;
    fn tiff_refs;
}
