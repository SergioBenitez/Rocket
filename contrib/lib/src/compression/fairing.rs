use rocket::config::{ConfigError, Value};
use rocket::fairing::{Fairing, Info, Kind};
use rocket::http::MediaType;
use rocket::Rocket;
use rocket::{Request, Response};

struct Context {
    exclusions: Vec<MediaType>,
}

impl Default for Context {
    fn default() -> Context {
        Context {
            exclusions: vec![
                MediaType::parse_flexible("application/gzip").unwrap(),
                MediaType::parse_flexible("application/zip").unwrap(),
                MediaType::parse_flexible("image/*").unwrap(),
                MediaType::parse_flexible("video/*").unwrap(),
                MediaType::parse_flexible("application/wasm").unwrap(),
                MediaType::parse_flexible("application/octet-stream").unwrap(),
            ],
        }
    }
}

/// Compresses all responses with Brotli or Gzip compression.
///
/// Compression is done in the same manner as the [`Compress`](super::Compress)
/// responder.
///
/// By default, the fairing does not compress responses with a `Content-Type`
/// matching any of the following:
///
/// - `application/gzip`
/// - `application/zip`
/// - `image/*`
/// - `video/*`
/// - `application/wasm`
/// - `application/octet-stream`
///
/// The excluded types can be changed changing the `compress.exclude` Rocket
/// configuration property in Rocket.toml. The default `Content-Type` exclusions
/// will be ignored if this is set, and must be added back in one by one if
/// desired.
///
/// ```toml
/// [global.compress]
/// exclude = ["video/*", "application/x-xz"]
/// ```
///
/// # Usage
///
/// Attach the compression [fairing](/rocket/fairing/) to your Rocket
/// application:
///
/// ```rust
/// extern crate rocket;
/// extern crate rocket_contrib;
///
/// use rocket_contrib::compression::Compression;
///
/// fn main() {
///     rocket::build()
///         // ...
///         .attach(Compression::fairing())
///         // ...
///     # ;
/// }
/// ```
pub struct Compression(());

impl Compression {
    /// Returns a fairing that compresses outgoing requests.
    ///
    /// ## Example
    /// To attach this fairing, simply call `attach` on the application's
    /// `Rocket` instance with `Compression::fairing()`:
    ///
    /// ```rust
    /// extern crate rocket;
    /// extern crate rocket_contrib;
    ///
    /// use rocket_contrib::compression::Compression;
    ///
    /// fn main() {
    ///     rocket::build()
    ///         // ...
    ///         .attach(Compression::fairing())
    ///         // ...
    ///     # ;
    /// }
    /// ```
    pub fn fairing() -> Compression {
        Compression(())
    }
}

impl Fairing for Compression {
    fn info(&self) -> Info {
        Info {
            name: "Response compression",
            kind: Kind::Attach | Kind::Response,
        }
    }

    fn on_attach(&self, rocket: Rocket) -> Result<Rocket, Rocket> {
        let mut ctxt = Context::default();

        match rocket.config().get_table("compress").and_then(|t| {
            t.get("exclude").ok_or_else(|| ConfigError::Missing(String::from("exclude")))
        }) {
            Ok(excls) => match excls.as_array() {
                Some(excls) => {
                    ctxt.exclusions = excls.iter().flat_map(|ex| {
                        if let Value::String(s) = ex {
                            let mt = MediaType::parse_flexible(s);
                            if mt.is_none() {
                                warn!("Ignoring invalid media type '{:?}'", s);
                            }
                            mt
                        } else {
                            warn!("Ignoring non-string media type '{:?}'", ex);
                            None
                        }
                    }).collect();
                }
                None => {
                    warn!(
                        "Exclusions is not an array; using default compression exclusions '{:?}'",
                        ctxt.exclusions
                    );
                }
            },
            Err(ConfigError::Missing(_)) => { /* ignore missing */ }
            Err(e) => {
                e.pretty_print();
                warn!(
                    "Using default compression exclusions '{:?}'",
                    ctxt.exclusions
                );
            }
        };

        Ok(rocket.manage(ctxt))
    }

    fn on_response<'r>(&self, request: &'r Request<'_>, response: &mut Response<'r>) {
        let context = request
            .guard::<rocket::State<'_, Context>>()
            .expect("Compression Context registered in on_attach");

        super::CompressionUtils::compress_response(request, response, &context.exclusions);
    }
}
