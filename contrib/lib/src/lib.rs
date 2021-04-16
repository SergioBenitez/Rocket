#![doc(html_root_url = "https://api.rocket.rs/master")]
#![doc(html_favicon_url = "https://rocket.rs/images/favicon.ico")]
#![doc(html_logo_url = "https://rocket.rs/images/logo-boxed.png")]

#![warn(rust_2018_idioms)]
#![allow(unused_extern_crates)]

//! This crate contains officially sanctioned contributor libraries that provide
//! functionality commonly used by Rocket applications.
//!
//! These libraries are always kept in-sync with the core Rocket library. They
//! provide common, but not fundamental, abstractions to be used by Rocket
//! applications.
//!
//! Each module in this library is held behind a feature flag, with the most
//! common modules exposed by default. The present feature list is below, with
//! an asterisk next to the features that are enabled by default:
//!
//! * [json*](type@json) - JSON (de)serialization
//! * [serve*](serve) - Static File Serving
//! * [msgpack](msgpack) - MessagePack (de)serialization
//! * [handlebars_templates](templates) - Handlebars Templating
//! * [tera_templates](templates) - Tera Templating
//! * [uuid](uuid) - UUID (de)serialization
//! * [${database}_pool](databases) - Database Configuration and Pooling
//! * [helmet](helmet) - Fairing for Security and Privacy Headers
//!
//! The recommend way to include features from this crate via Rocket in your
//! project is by adding a `[dependencies.rocket_contrib]` section to your
//! `Cargo.toml` file, setting `default-features` to false, and specifying
//! features manually. For example, to use the JSON module, you would add:
//!
//! ```toml
//! [dependencies.rocket_contrib]
//! version = "0.5.0-dev"
//! default-features = false
//! features = ["json"]
//! ```
//!
//! This crate is expected to grow with time, bringing in outside crates to be
//! officially supported by Rocket.

#[allow(unused_imports)] #[macro_use] extern crate tracing;
#[allow(unused_imports)] #[macro_use] extern crate rocket;

#[cfg(feature="json")] #[macro_use] pub mod json;
#[cfg(feature="serve")] pub mod serve;
#[cfg(feature="msgpack")] pub mod msgpack;
#[cfg(feature="templates")] pub mod templates;
#[cfg(feature="uuid")] pub mod uuid;
#[cfg(feature="databases")] pub mod databases;
#[cfg(feature = "helmet")] pub mod helmet;
// TODO.async: Migrate compression, reenable this, tests, and add to docs.
//#[cfg(any(feature="brotli_compression", feature="gzip_compression"))] pub mod compression;

#[cfg(feature="databases")] #[doc(hidden)] pub use rocket_contrib_codegen::*;
