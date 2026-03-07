use extendr_api::prelude::*;
use std::sync::Arc;
use tokio::runtime::Runtime;
use futures::stream::{self, StreamExt};

// ── Store construction ──────────────────────────────────────────────

fn make_store_and_path(
    url_str: &str,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<(Arc<dyn object_store::ObjectStore>, object_store::path::Path), String> {
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

/// Columns built up across all files and IFDs.
/// New vs v1: gdal_nodata, planar_configuration, scale_x, scale_y,
/// origin_x, origin_y (flat geotransform from GeoTIFF tags).
struct RefColumns {
    path: Vec<String>,
    ifd: Vec<i32>,
    tile_col: Vec<i32>,
    tile_row: Vec<i32>,
    offset: Vec<f64>,
    length: Vec<f64>,
    image_w: Vec<i32>,
    image_h: Vec<i32>,
    tile_w: Vec<i32>,
    tile_h: Vec<i32>,
    dtype: Vec<String>,
    compression: Vec<String>,
    bits_per_sample: Vec<i32>,
    samples_per_pixel: Vec<i32>,
    crs_epsg: Vec<i32>,
    gdal_nodata: Vec<Option<String>>,
    planar_configuration: Vec<String>,
    scale_x: Vec<Option<f64>>,
    scale_y: Vec<Option<f64>>,
    origin_x: Vec<Option<f64>>,
    origin_y: Vec<Option<f64>>,
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
            gdal_nodata: Vec::new(),
            planar_configuration: Vec::new(),
            scale_x: Vec::new(),
            scale_y: Vec::new(),
            origin_x: Vec::new(),
            origin_y: Vec::new(),
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
        self.gdal_nodata.extend(other.gdal_nodata);
        self.planar_configuration.extend(other.planar_configuration);
        self.scale_x.extend(other.scale_x);
        self.scale_y.extend(other.scale_y);
        self.origin_x.extend(other.origin_x);
        self.origin_y.extend(other.origin_y);
    }
}

fn format_dtype(sample_format: &[async_tiff::tags::SampleFormat], bps: &[u16]) -> String {
    use async_tiff::tags::SampleFormat;
    let fmt = sample_format.first().copied().unwrap_or(SampleFormat::Uint);
    let bits = bps.first().copied().unwrap_or(8);
    let bytes = (bits + 7) / 8;
    let prefix = if bytes == 1 { "|" } else { "<" };
    let code = match fmt {
        SampleFormat::Uint => "u",
        SampleFormat::Int => "i",
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

        // Strip-based IFDs: warn and skip
        let t_w = match ifd.tile_width() {
            Some(w) if w > 0 => w,
            _ => {
                rprintln!(
                    "Warning: IFD {} in {} is strip-based (not tiled), skipping",
                    ifd_idx, url_str
                );
                continue;
            }
        };
        let t_h = match ifd.tile_height() {
            Some(h) if h > 0 => h,
            _ => continue,
        };

        let tiles_across = (img_w + t_w - 1) / t_w;
        let tiles_down   = (img_h + t_h - 1) / t_h;

        let compression_str   = format!("{:?}", ifd.compression());
        let planar_str        = format!("{:?}", ifd.planar_configuration());
        let sample_fmt        = ifd.sample_format();
        let bps               = ifd.bits_per_sample();
        let spp               = ifd.samples_per_pixel();
        let dtype_str         = format_dtype(&sample_fmt, &bps);
        let bps_first         = bps.first().copied().unwrap_or(8) as i32;

        let epsg: i32 = ifd.geo_key_directory()
            .and_then(|geo| geo.projected_type.or(geo.geographic_type))
            .map(|code| code as i32)
            .unwrap_or(i32::MIN);

        let nodata: Option<String> = ifd.gdal_nodata().map(|s| s.to_string());

        // Geotransform from GeoTIFF tags.
        // ModelPixelScale: [scale_x, scale_y, scale_z]
        // ModelTiepoint:   [i, j, k, x, y, z, ...]  — first tiepoint used
        let scale_x: Option<f64> = ifd.model_pixel_scale().and_then(|s| s.first().copied());
        let scale_y: Option<f64> = ifd.model_pixel_scale().and_then(|s| s.get(1).copied());
        let origin_x: Option<f64> = ifd.model_tiepoint().and_then(|t| t.get(3).copied());
        let origin_y: Option<f64> = ifd.model_tiepoint().and_then(|t| t.get(4).copied());

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
            cols.gdal_nodata.push(nodata.clone());
            cols.planar_configuration.push(planar_str.clone());
            cols.scale_x.push(scale_x);
            cols.scale_y.push(scale_y);
            cols.origin_x.push(origin_x);
            cols.origin_y.push(origin_y);
        }
    }

    Ok(cols)
}

// ── Exported R functions ────────────────────────────────────────────

/// Extract tile byte-range references from TIFF/COG files.
///
/// @param paths Character vector of file paths or URLs
///   (s3://, gs://, az://, http://, https://, or local paths).
/// @param region Optional AWS region string (e.g. "us-west-2").
/// @param anon Logical, use anonymous/unsigned requests.
/// @param concurrency Integer, max concurrent file scans.
/// @return A data.frame with columns path, ifd, tile_col, tile_row,
///   offset, length, image_w, image_h, tile_w, tile_h, dtype,
///   compression, bits_per_sample, samples_per_pixel, crs_epsg,
///   gdal_nodata, planar_configuration, scale_x, scale_y,
///   origin_x, origin_y.
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
            return empty_refs_df();
        }
    };

    let owned_paths: Vec<String> = paths.iter().map(|p| p.as_str().to_string()).collect();

    let (cols, errors) = rt.block_on(async {
        let results: Vec<std::result::Result<RefColumns, String>> = stream::iter(owned_paths)
            .map(|path| {
                let region_owned = region_ref.map(|s| s.to_string());
                async move { scan_one_file(&path, region_owned.as_deref(), anon).await }
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
        path                = cols.path,
        ifd                 = cols.ifd,
        tile_col            = cols.tile_col,
        tile_row            = cols.tile_row,
        offset              = cols.offset,
        length              = cols.length,
        image_w             = cols.image_w,
        image_h             = cols.image_h,
        tile_w              = cols.tile_w,
        tile_h              = cols.tile_h,
        dtype               = cols.dtype,
        compression         = cols.compression,
        bits_per_sample     = cols.bits_per_sample,
        samples_per_pixel   = cols.samples_per_pixel,
        crs_epsg            = crs_epsg_r,
        gdal_nodata         = cols.gdal_nodata,
        planar_configuration = cols.planar_configuration,
        scale_x             = cols.scale_x,
        scale_y             = cols.scale_y,
        origin_x            = cols.origin_x,
        origin_y            = cols.origin_y
    )
}

fn empty_refs_df() -> Robj {
    data_frame!(
        path                = Vec::<String>::new(),
        ifd                 = Vec::<i32>::new(),
        tile_col            = Vec::<i32>::new(),
        tile_row            = Vec::<i32>::new(),
        offset              = Vec::<f64>::new(),
        length              = Vec::<f64>::new(),
        image_w             = Vec::<i32>::new(),
        image_h             = Vec::<i32>::new(),
        tile_w              = Vec::<i32>::new(),
        tile_h              = Vec::<i32>::new(),
        dtype               = Vec::<String>::new(),
        compression         = Vec::<String>::new(),
        bits_per_sample     = Vec::<i32>::new(),
        samples_per_pixel   = Vec::<i32>::new(),
        crs_epsg            = Vec::<Option<i32>>::new(),
        gdal_nodata         = Vec::<Option<String>>::new(),
        planar_configuration = Vec::<String>::new(),
        scale_x             = Vec::<Option<f64>>::new(),
        scale_y             = Vec::<Option<f64>>::new(),
        origin_x            = Vec::<Option<f64>>::new(),
        origin_y            = Vec::<Option<f64>>::new()
    )
}

// ── IFD-level summary (one row per IFD, no tile explosion) ──────────

async fn ifd_info_one_file(
    url_str: &str,
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<IfdInfoColumns, String> {
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

    let mut cols = IfdInfoColumns::new();

    for (ifd_idx, ifd) in tiff.ifds().iter().enumerate() {
        let is_tiled = ifd.tile_width().map(|w| w > 0).unwrap_or(false);
        let (n_tiles_x, n_tiles_y, t_w, t_h) = match ifd.tile_count() {
            Some((nx, ny)) => (
                nx as i32,
                ny as i32,
                ifd.tile_width().unwrap_or(0) as i32,
                ifd.tile_height().unwrap_or(0) as i32,
            ),
            None => (0, 0, 0, 0),
        };

        let epsg: i32 = ifd.geo_key_directory()
            .and_then(|geo| geo.projected_type.or(geo.geographic_type))
            .map(|code| code as i32)
            .unwrap_or(i32::MIN);

        let sample_fmt = ifd.sample_format();
        let bps = ifd.bits_per_sample();
        let dtype_str = format_dtype(&sample_fmt, &bps);
        let bps_first = bps.first().copied().unwrap_or(8) as i32;

        let scale_x: Option<f64> = ifd.model_pixel_scale().and_then(|s| s.first().copied());
        let scale_y: Option<f64> = ifd.model_pixel_scale().and_then(|s| s.get(1).copied());
        let origin_x: Option<f64> = ifd.model_tiepoint().and_then(|t| t.get(3).copied());
        let origin_y: Option<f64> = ifd.model_tiepoint().and_then(|t| t.get(4).copied());

        cols.path.push(url_str.to_string());
        cols.ifd.push(ifd_idx as i32);
        cols.is_tiled.push(is_tiled);
        cols.image_w.push(ifd.image_width() as i32);
        cols.image_h.push(ifd.image_height() as i32);
        cols.tile_w.push(t_w);
        cols.tile_h.push(t_h);
        cols.n_tiles_x.push(n_tiles_x);
        cols.n_tiles_y.push(n_tiles_y);
        cols.dtype.push(dtype_str);
        cols.compression.push(format!("{:?}", ifd.compression()));
        cols.bits_per_sample.push(bps_first);
        cols.samples_per_pixel.push(ifd.samples_per_pixel() as i32);
        cols.photometric.push(format!("{:?}", ifd.photometric_interpretation()));
        cols.predictor.push(ifd.predictor().map(|p| format!("{:?}", p)));
        cols.planar_configuration.push(format!("{:?}", ifd.planar_configuration()));
        cols.crs_epsg.push(epsg);
        cols.gdal_nodata.push(ifd.gdal_nodata().map(|s| s.to_string()));
        cols.scale_x.push(scale_x);
        cols.scale_y.push(scale_y);
        cols.origin_x.push(origin_x);
        cols.origin_y.push(origin_y);
    }

    Ok(cols)
}

struct IfdInfoColumns {
    path: Vec<String>,
    ifd: Vec<i32>,
    is_tiled: Vec<bool>,
    image_w: Vec<i32>,
    image_h: Vec<i32>,
    tile_w: Vec<i32>,
    tile_h: Vec<i32>,
    n_tiles_x: Vec<i32>,
    n_tiles_y: Vec<i32>,
    dtype: Vec<String>,
    compression: Vec<String>,
    bits_per_sample: Vec<i32>,
    samples_per_pixel: Vec<i32>,
    photometric: Vec<String>,
    predictor: Vec<Option<String>>,
    planar_configuration: Vec<String>,
    crs_epsg: Vec<i32>,
    gdal_nodata: Vec<Option<String>>,
    scale_x: Vec<Option<f64>>,
    scale_y: Vec<Option<f64>>,
    origin_x: Vec<Option<f64>>,
    origin_y: Vec<Option<f64>>,
}

impl IfdInfoColumns {
    fn new() -> Self {
        Self {
            path: Vec::new(),
            ifd: Vec::new(),
            is_tiled: Vec::new(),
            image_w: Vec::new(),
            image_h: Vec::new(),
            tile_w: Vec::new(),
            tile_h: Vec::new(),
            n_tiles_x: Vec::new(),
            n_tiles_y: Vec::new(),
            dtype: Vec::new(),
            compression: Vec::new(),
            bits_per_sample: Vec::new(),
            samples_per_pixel: Vec::new(),
            photometric: Vec::new(),
            predictor: Vec::new(),
            planar_configuration: Vec::new(),
            crs_epsg: Vec::new(),
            gdal_nodata: Vec::new(),
            scale_x: Vec::new(),
            scale_y: Vec::new(),
            origin_x: Vec::new(),
            origin_y: Vec::new(),
        }
    }

    fn extend(&mut self, other: IfdInfoColumns) {
        self.path.extend(other.path);
        self.ifd.extend(other.ifd);
        self.is_tiled.extend(other.is_tiled);
        self.image_w.extend(other.image_w);
        self.image_h.extend(other.image_h);
        self.tile_w.extend(other.tile_w);
        self.tile_h.extend(other.tile_h);
        self.n_tiles_x.extend(other.n_tiles_x);
        self.n_tiles_y.extend(other.n_tiles_y);
        self.dtype.extend(other.dtype);
        self.compression.extend(other.compression);
        self.bits_per_sample.extend(other.bits_per_sample);
        self.samples_per_pixel.extend(other.samples_per_pixel);
        self.photometric.extend(other.photometric);
        self.predictor.extend(other.predictor);
        self.planar_configuration.extend(other.planar_configuration);
        self.crs_epsg.extend(other.crs_epsg);
        self.gdal_nodata.extend(other.gdal_nodata);
        self.scale_x.extend(other.scale_x);
        self.scale_y.extend(other.scale_y);
        self.origin_x.extend(other.origin_x);
        self.origin_y.extend(other.origin_y);
    }
}

/// Summarise IFD-level metadata from TIFF/COG files.
///
/// Returns one row per IFD per file — no tile explosion. Useful for
/// understanding file structure before deciding which tiles to fetch.
///
/// @param paths Character vector of file paths or URLs.
/// @param region Optional AWS region string.
/// @param anon Logical, use anonymous requests.
/// @param concurrency Integer, max concurrent file scans.
/// @return A data.frame with one row per IFD containing image dimensions,
///   tile grid size, dtype, compression, photometric interpretation,
///   predictor, planar configuration, CRS, nodata, and geotransform.
#[extendr]
fn tiff_ifd_info(paths: Strings, region: Nullable<String>, anon: bool, concurrency: i32) -> Robj {
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
            return data_frame!().into();
        }
    };

    let owned_paths: Vec<String> = paths.iter().map(|p| p.as_str().to_string()).collect();

    let (cols, errors) = rt.block_on(async {
        let results: Vec<std::result::Result<IfdInfoColumns, String>> = stream::iter(owned_paths)
            .map(|path| {
                let region_owned = region_ref.map(|s| s.to_string());
                async move { ifd_info_one_file(&path, region_owned.as_deref(), anon).await }
            })
            .buffer_unordered(conc)
            .collect()
            .await;

        let mut merged = IfdInfoColumns::new();
        let mut errs = Vec::new();
        for result in results {
            match result {
                Ok(c) => merged.extend(c),
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
        path                 = cols.path,
        ifd                  = cols.ifd,
        is_tiled             = cols.is_tiled,
        image_w              = cols.image_w,
        image_h              = cols.image_h,
        tile_w               = cols.tile_w,
        tile_h               = cols.tile_h,
        n_tiles_x            = cols.n_tiles_x,
        n_tiles_y            = cols.n_tiles_y,
        dtype                = cols.dtype,
        compression          = cols.compression,
        bits_per_sample      = cols.bits_per_sample,
        samples_per_pixel    = cols.samples_per_pixel,
        photometric          = cols.photometric,
        predictor            = cols.predictor,
        planar_configuration = cols.planar_configuration,
        crs_epsg             = crs_epsg_r,
        gdal_nodata          = cols.gdal_nodata,
        scale_x              = cols.scale_x,
        scale_y              = cols.scale_y,
        origin_x             = cols.origin_x,
        origin_y             = cols.origin_y
    )
}

// ── Single tile fetch + decode ──────────────────────────────────────

/// Fetch and decode a single tile from a TIFF/COG file.
///
/// @param path File path or URL to the TIFF.
/// @param ifd_index IFD index (0-based).
/// @param col Tile column (0-based).
/// @param row Tile row (0-based).
/// @param region Optional AWS region string.
/// @param anon Logical, use anonymous requests.
/// @return A named list with data, dim, dtype.
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
            Ok(r) => r,
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

    let tile = ifd.fetch_tile(col, row, &reader)
        .await
        .map_err(|e| format!("Failed to fetch tile ({},{}): {}", col, row, e))?;

    let array = tile.decode(&Default::default())
        .map_err(|e| format!("Failed to decode tile: {}", e))?;

    Ok(typed_array_to_robj(array))
}

// ── Batch tile fetch + decode ───────────────────────────────────────

/// Fetch and decode multiple tiles from a TIFF/COG file.
///
/// Uses ifd.fetch_tiles() for a single vectorized range request.
///
/// @param path File path or URL to the TIFF.
/// @param ifd_index IFD index (0-based).
/// @param cols Integer vector of tile columns (0-based).
/// @param rows Integer vector of tile rows (0-based).
/// @param region Optional AWS region string.
/// @param anon Logical, use anonymous requests.
/// @return A list of tile result lists, each with data, dim, dtype.
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
        match fetch_decode_tiles_batch(path, ifd_index as usize, cols, rows, region_ref, anon).await {
            Ok(tiles) => List::from_values(tiles).into_robj(),
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

    // Use fetch_tiles() — one vectorized range request to the object
    // store rather than N sequential ones. async-tiff 0.2.0 takes
    // separate x and y slices.
    let xs: Vec<usize> = cols.iter().map(|&c| c as usize).collect();
    let ys: Vec<usize> = rows.iter().map(|&r| r as usize).collect();

    let tiles = ifd.fetch_tiles(&xs, &ys, &reader)
        .await
        .map_err(|e| format!("Failed to fetch tiles: {}", e))?;

    tiles.into_iter()
        .map(|tile| {
            let array = tile.decode(&Default::default())
                .map_err(|e| format!("Failed to decode tile: {}", e))?;
            Ok(typed_array_to_robj(array))
        })
        .collect()
}

// ── Shared decode helper ────────────────────────────────────────────

fn typed_array_to_robj(array: async_tiff::Array) -> Robj {
    let (typed_data, shape, data_type) = array.into_inner();

    let dtype_str = match data_type {
        Some(async_tiff::DataType::UInt8)   => "|u1",
        Some(async_tiff::DataType::UInt16)  => "<u2",
        Some(async_tiff::DataType::UInt32)  => "<u4",
        Some(async_tiff::DataType::UInt64)  => "<u8",
        Some(async_tiff::DataType::Int8)    => "|i1",
        Some(async_tiff::DataType::Int16)   => "<i2",
        Some(async_tiff::DataType::Int32)   => "<i4",
        Some(async_tiff::DataType::Int64)   => "<i8",
        Some(async_tiff::DataType::Float32) => "<f4",
        Some(async_tiff::DataType::Float64) => "<f8",
        _ => "|u1",
    };

    let data_vec: Vec<f64> = match typed_data {
        async_tiff::TypedArray::UInt8(arr)   => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt16(arr)  => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt32(arr)  => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::UInt64(arr)  => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int8(arr)    => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int16(arr)   => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int32(arr)   => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Int64(arr)   => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Float32(arr) => arr.iter().map(|&v| v as f64).collect(),
        async_tiff::TypedArray::Float64(arr) => arr.iter().copied().collect(),
    };

    let dim: Vec<i32> = shape.iter().map(|&d| d as i32).collect();

    list!(data = data_vec, dim = dim, dtype = dtype_str).into()
}

// ── Multi-file tile fetch ───────────────────────────────────────────

/// Internal entry point for tiff_read_tiles().
///
/// The public R API is the [tiff_read_tiles()] wrapper in `R/rustycogs.R`
/// which accepts a refs data frame and returns it with a `data` list-column.
/// This function takes the raw column vectors directly.
///
/// @keywords internal
#[extendr]
fn tiff_read_tiles(
    paths: Strings,
    ifd_indices: &[i32],
    cols: &[i32],
    rows: &[i32],
    region: Nullable<String>,
    anon: bool,
    concurrency: i32,
) -> Robj {
    let n = paths.len();
    if ifd_indices.len() != n || cols.len() != n || rows.len() != n {
        rprintln!("Error: paths, ifd_indices, cols, rows must all have the same length");
        return list!().into();
    }

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
            return list!().into();
        }
    };

    // Group rows by (path, ifd_index), preserving original row indices
    // so we can restore order after async fan-out.
    // key: (path, ifd_index)  value: Vec<(original_row_idx, col, row)>
    let mut groups: std::collections::HashMap<(String, usize), Vec<(usize, usize, usize)>> =
        std::collections::HashMap::new();

    for i in 0..n {
        let path = paths[i].as_str().to_string();
        let ifd = ifd_indices[i] as usize;
        let c = cols[i] as usize;
        let r = rows[i] as usize;
        groups.entry((path, ifd)).or_default().push((i, c, r));
    }

    // Each task returns Vec<(original_row_idx, Robj)>
    let tasks: Vec<_> = groups.into_iter().map(|((path, ifd_index), tile_specs)| {
        let region_owned = region_ref.map(|s| s.to_string());
        async move {
            fetch_decode_group(&path, ifd_index, &tile_specs, region_owned.as_deref(), anon).await
        }
    }).collect();

    let results = rt.block_on(async {
        stream::iter(tasks)
            .buffer_unordered(conc)
            .collect::<Vec<_>>()
            .await
    });

    // Collect into a flat vec sized to n, filling by original row index
    let mut output: Vec<Option<Robj>> = (0..n).map(|_| None).collect();
    for result in results {
        match result {
            Ok(pairs) => {
                for (orig_idx, robj) in pairs {
                    output[orig_idx] = Some(robj);
                }
            }
            Err(e) => { rprintln!("Warning: {}", e); },
        }
    }

    // Replace any failed tiles with NA
    let final_list: Vec<Robj> = output.into_iter()
        .map(|opt| opt.unwrap_or_else(|| Robj::from(Rfloat::na())))
        .collect();

    List::from_values(final_list).into_robj()
}

async fn fetch_decode_group(
    url_str: &str,
    ifd_index: usize,
    tile_specs: &[(usize, usize, usize)],  // (orig_idx, col, row)
    region: Option<&str>,
    anon: bool,
) -> std::result::Result<Vec<(usize, Robj)>, String> {
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

    let ifd = tiff.ifds().get(ifd_index)
        .ok_or_else(|| format!("IFD index {} out of range in {}", ifd_index, url_str))?;

    let xs: Vec<usize> = tile_specs.iter().map(|&(_, c, _)| c).collect();
    let ys: Vec<usize> = tile_specs.iter().map(|&(_, _, r)| r).collect();

    let tiles = ifd.fetch_tiles(&xs, &ys, &reader)
        .await
        .map_err(|e| format!("Failed to fetch tiles from {}: {}", url_str, e))?;

    let mut out = Vec::with_capacity(tile_specs.len());
    for (tile, &(orig_idx, _, _)) in tiles.into_iter().zip(tile_specs.iter()) {
        let array = tile.decode(&Default::default())
            .map_err(|e| format!("Failed to decode tile in {}: {}", url_str, e))?;
        out.push((orig_idx, typed_array_to_robj(array)));
    }

    Ok(out)
}

extendr_module! {
    mod rustycogs;
    fn tiff_refs;
    fn tiff_ifd_info;
    fn tiff_read_tiles;
    fn tiff_tile;
    fn tiff_tiles;
}
