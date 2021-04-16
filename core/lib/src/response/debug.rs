use crate::request::Request;
use crate::response::{self, Responder};
use crate::http::Status;

use yansi::Paint;

/// Debug prints the internal value before forwarding to the 500 error catcher.
///
/// This value exists primarily to allow handler return types that would not
/// otherwise implement [`Responder`]. It is typically used in conjunction with
/// `Result<T, E>` where `E` implements `Debug` but not `Responder`.
///
/// Note that because of it's common use as an error value, `std::io::Error`
/// _does_ implement `Responder`. As a result, a `std::io::Result<T>` can be
/// returned directly without the need for `Debug`:
///
/// ```rust
/// use std::io;
///
/// # use rocket::get;
/// use rocket::response::NamedFile;
///
/// #[get("/")]
/// async fn index() -> io::Result<NamedFile> {
///     NamedFile::open("index.html").await
/// }
/// ```
///
/// # Example
///
/// Because of the generic `From<E>` implementation for `Debug<E>`, conversions
/// from `Result<T, E>` to `Result<T, Debug<E>>` through `?` occur
/// automatically:
///
/// ```rust
/// use std::string::FromUtf8Error;
///
/// # use rocket::get;
/// use rocket::response::Debug;
///
/// #[get("/")]
/// fn rand_str() -> Result<String, Debug<FromUtf8Error>> {
///     # /*
///     let bytes: Vec<u8> = random_bytes();
///     # */
///     # let bytes: Vec<u8> = vec![];
///     Ok(String::from_utf8(bytes)?)
/// }
/// ```
///
/// It is also possible to map the error directly to `Debug` via
/// [`Result::map_err()`]:
///
/// ```rust
/// use std::string::FromUtf8Error;
///
/// # use rocket::get;
/// use rocket::response::Debug;
///
/// #[get("/")]
/// fn rand_str() -> Result<String, Debug<FromUtf8Error>> {
///     # /*
///     let bytes: Vec<u8> = random_bytes();
///     # */
///     # let bytes: Vec<u8> = vec![];
///     String::from_utf8(bytes).map_err(Debug)
/// }
/// ```
#[derive(Debug)]
pub struct Debug<E>(pub E);

impl<E> From<E> for Debug<E> {
    #[inline(always)]
    fn from(e: E) -> Self {
        Debug(e)
    }
}

impl<'r, E: std::fmt::Debug> Responder<'r, 'static> for Debug<E> {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        warn!("Debug: {:?}", Paint::default(&self.0));
        warn!("Debug always responds with {}.", Status::InternalServerError);
        Err(Status::InternalServerError)
    }
}

/// Prints a warning with the error and forwards to the `500` error catcher.
impl<'r> Responder<'r, 'static> for std::io::Error {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        warn!("I/O Error: {:?}", yansi::Paint::default(&self));
        Err(Status::InternalServerError)
    }
}
