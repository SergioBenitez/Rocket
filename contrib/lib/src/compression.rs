//! This module provides brotli and gzip compression for all non-image
//! responses for requests that send Accept-Encoding br and gzip. If
//! accepted, brotli compression is preferred over gzip.
//!
//! To add this feature to your Rocket application, use
//! .attach(rocket_contrib::Compression::fairing())
//! to your Rocket instance. Note that you must add the
//! "compression" feature for brotli and gzip compression to your rocket_contrib
//! dependency in Cargo.toml. Additionally, you can load only brotli compression
//! using "brotli_compression" feature or load only gzip compression using
//! "gzip_compression" in your rocket_contrib dependency in Cargo.toml.
//!
//! In the brotli algorithm, quality is set to 2 in order to have really fast
//! compressions with compression ratio similar to gzip. Also, text and font
//! compression mode is set regarding the Content-Type of the response.
//!
//! In the gzip algorithm, quality is the default (9) in order to have good
//! compression ratio.
//!
//! For brotli compression, the rust-brotli crate is used.
//! For gzip compression, flate2 crate is used.

use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::Header;
use rocket::{Request, Response};
use std::io::Read;

#[cfg(feature = "brotli_compression")]
use brotli;
#[cfg(feature = "brotli_compression")]
use brotli::enc::backward_references::BrotliEncoderMode;

#[cfg(feature = "gzip_compression")]
use flate2;
#[cfg(feature = "gzip_compression")]
use flate2::read::GzEncoder;

pub struct Compression(());

impl Compression {
    /// This function creates a Compression to be used in your Rocket
    /// instance. Add ```.attach(rocket_contrib::Compression::fairing())```
    /// to your Rocket instance to use this fairing.
    ///
    /// # Returns
    ///
    /// A Compression instance.
    pub fn fairing() -> Compression {
        Compression { 0: () }
    }

    fn accepts_encoding(request: &Request, encodings: &[&str]) -> bool {
        request
            .headers()
            .get("Accept-Encoding")
            .flat_map(|e| e.split(","))
            .any(|e| encodings.contains(&e.trim()))
    }

    fn already_encoded(response: &Response) -> bool {
        response
            .headers()
            .get("Content-Encoding")
            .any(|e| e != "identity" && e != "chunked")
    }

    fn set_body_and_header<'r, B: Read + 'r>(
        response: &mut Response<'r>,
        body: B,
        header: &'static str,
    ) {
        response.remove_header("Content-Encoding");
        response.adjoin_header(Header::new("Content-Encoding", header));
        response.set_streamed_body(body);
    }
}

impl Fairing for Compression {
    fn info(&self) -> Info {
        Info {
            name: "Brotli and gzip compressors for responses",
            kind: Kind::Response,
        }
    }

    fn on_response(&self, request: &Request, response: &mut Response) {
        if Compression::already_encoded(response) {
            return;
        }

        let content_type = response.content_type();
        // Images must not be compressed
        if let Some(ref content_type) = content_type {
            if content_type.top() == "image" {
                return;
            }
        }

        // The compression is done if the request supports brotli or gzip and
        // the corresponding feature is enabled
        if cfg!(feature = "brotli_compression")
            && Compression::accepts_encoding(request, &["br", "brotli"])
        {
            if let Some(plain) = response.take_body() {
                let mut params = brotli::enc::BrotliEncoderInitParams();
                params.quality = 2;
                if let Some(ref content_type) = content_type {
                    if content_type.top() == "text" {
                        params.mode = BrotliEncoderMode::BROTLI_MODE_TEXT;
                    } else if content_type.top() == "font" {
                        params.mode = BrotliEncoderMode::BROTLI_MODE_FONT;
                    }
                }
                let mut compressor =
                    brotli::CompressorReader::with_params(plain.into_inner(), 4096, &params);
                Compression::set_body_and_header(response, compressor, "br");
            }
        } else if cfg!(feature = "gzip_compression")
            && Compression::accepts_encoding(request, &["gzip"])
        {
            if let Some(plain) = response.take_body() {
                Compression::set_body_and_header(
                    response,
                    GzEncoder::new(plain.into_inner(), flate2::Compression::default()),
                    "gzip",
                );
            }
        }
    }
}
