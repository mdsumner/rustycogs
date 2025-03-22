use extendr_api::prelude::*;
use async_tiff::TIFF;
//use async_tiff::decoder::DecoderRegistry;
use async_tiff::reader::ObjectReader;

//use std::io::BufReader;
use std::sync::Arc;
//use super::*;
use object_store::local::LocalFileSystem;
//use tiff::decoder::{DecodingResult, Limits};


/// Return string `"Hello world!"` to R.
/// @export
#[extendr]
fn rusty(file: String) -> &'static str {
        let dir = "/tiff/";
        let path = object_store::path::Path::parse(file).unwrap();
        let store = Arc::new(LocalFileSystem::new_with_prefix(dir).unwrap());
        let reader = Arc::new(ObjectReader::new(store, path));

        // we can't .await.unwrap MDS
        //let cog_reader = TIFF::try_open(reader.clone()).await.unwrap();

    dir
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
