use async_tiff::{decoder::DecoderRegistry, TIFF};
use extendr_api::prelude::*;
//use async_tiff::decoder::DecoderRegistry;
use async_tiff::reader::{AsyncFileReader, ObjectReader};
use tokio::runtime::{Builder, Runtime};

//use std::io::BufReader;
use std::{
    cell::OnceCell,
    sync::{Arc, OnceLock},
};
//use super::*;
use object_store::local::LocalFileSystem;
//use tiff::decoder::{DecodingResult, Limits};

static TOKIO_RUNTIME: OnceLock<Runtime> = OnceLock::new();

// Helper function to get a tokio runtime
fn get_rt() -> &'static Runtime {
    TOKIO_RUNTIME.get_or_init(|| {
        Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// Return string `"Hello world!"` to R.
/// @export
#[extendr]
fn rusty(file: String) -> Raw {
    let dir = "/tiff/";
    let path = object_store::path::Path::parse(file).unwrap();
    let store = Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
    let reader = Arc::new(ObjectReader::new(store, path));
    let rt = get_rt();

    let cog_reader = rt.block_on(async {
        TIFF::try_open(Box::new(reader.as_ref().clone()))
            .await
            .unwrap()
    });
    let ifds = cog_reader.ifds().as_ref();
    let ifd = &ifds[0];
    let decoder_registry = DecoderRegistry::default();
    let tile = rt
        .block_on(ifd.fetch_tile(0, 0, reader.as_ref()))
        .expect("failed to get tile");
    let tile = tile.decode(&decoder_registry).expect("Failed to decode");
    Raw::from_bytes(&tile)
}

// example code from async-tiff
//        let dir = "/tiff/";
//        let path = object_store::path::Path::parse("volcano.tif").unwrap();
//        let store = Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
//        let reader = Arc::new(ObjectReader::new(store, path));
//        let cog_reader = TIFF::try_open(reader.clone()).await.unwrap();
//        let ifd = &cog_reader.ifds.as_ref()[1];
//        let decoder_registry = DecoderRegistry::default();
//        let tile = ifd.fetch_tile(0, 0, reader.as_ref()).await.unwrap();
//        let tile = tile.decode(&decoder_registry).unwrap();
//std::fs::write("img.buf", tile).unwrap();

// Macro to generate exports.
// This ensures exported functions are registered with R.
// See corresponding C code in `entrypoint.c`.
extendr_module! {
    mod rustycogs;
    fn rusty;
}
