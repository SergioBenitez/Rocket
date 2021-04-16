//! Automatic MessagePack (de)serialization support.

//!
//! See the [`MsgPack`](crate::msgpack::MsgPack) type for further details.
//!
//! # Enabling
//!
//! This module is only available when the `msgpack` feature is enabled. Enable
//! it in `Cargo.toml` as follows:
//!
//! ```toml
//! [dependencies.rocket_contrib]
//! version = "0.5.0-dev"
//! default-features = false
//! features = ["msgpack"]
//! ```

use std::io;
use std::ops::{Deref, DerefMut};

use rocket::request::{Request, local_cache};
use rocket::data::{ByteUnit, Data, FromData, Outcome};
use rocket::response::{self, Responder, content};
use rocket::http::Status;
use rocket::form::prelude as form;

use serde::Serialize;
use serde::de::{Deserialize, DeserializeOwned};

pub use rmp_serde::decode::Error;

/// The `MsgPack` data guard and responder: easily consume and respond with
/// MessagePack.
///
/// ## Receiving MessagePack
///
/// `MsgPack` is both a data guard and a form guard.
///
/// ### Data Guard
///
/// To parse request body data as MessagePack , add a `data` route argument with
/// a target type of `MsgPack<T>`, where `T` is some type you'd like to parse
/// from JSON. `T` must implement [`serde::Deserialize`].
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type User = usize;
/// use rocket_contrib::msgpack::MsgPack;
///
/// #[post("/users", format = "msgpack", data = "<user>")]
/// fn new_user(user: MsgPack<User>) {
///     /* ... */
/// }
/// ```
///
/// You don't _need_ to use `format = "msgpack"`, but it _may_ be what you want.
/// Using `format = msgpack` means that any request that doesn't specify
/// "application/msgpack" as its first `Content-Type:` header parameter will not
/// be routed to this handler.
///
/// ### Form Guard
///
/// `MsgPack<T>`, as a form guard, accepts value and data fields and parses the
/// data as a `T`. Simple use `MsgPack<T>`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type Metadata = usize;
/// use rocket::form::{Form, FromForm};
/// use rocket_contrib::msgpack::MsgPack;
///
/// #[derive(FromForm)]
/// struct User<'r> {
///     name: &'r str,
///     metadata: MsgPack<Metadata>
/// }
///
/// #[post("/users", data = "<form>")]
/// fn new_user(form: Form<User<'_>>) {
///     /* ... */
/// }
/// ```
///
/// ## Sending MessagePack
///
/// If you're responding with MessagePack data, return a `MsgPack<T>` type,
/// where `T` implements [`Serialize`] from [`serde`]. The content type of the
/// response is set to `application/msgpack` automatically.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type User = usize;
/// use rocket_contrib::msgpack::MsgPack;
///
/// #[get("/users/<id>")]
/// fn user(id: usize) -> MsgPack<User> {
///     let user_from_id = User::from(id);
///     /* ... */
///     MsgPack(user_from_id)
/// }
/// ```
///
/// ## Incoming Data Limits
///
/// The default size limit for incoming MessagePack data is 1MiB. Setting a
/// limit protects your application from denial of service (DOS) attacks and
/// from resource exhaustion through high memory consumption. The limit can be
/// increased by setting the `limits.msgpack` configuration parameter. For
/// instance, to increase the MessagePack limit to 5MiB for all environments,
/// you may add the following to your `Rocket.toml`:
///
/// ```toml
/// [global.limits]
/// msgpack = 5242880
/// ```
#[derive(Debug)]
pub struct MsgPack<T>(pub T);

impl<T> MsgPack<T> {
    /// Consumes the `MsgPack` wrapper and returns the wrapped item.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket_contrib::msgpack::MsgPack;
    /// let string = "Hello".to_string();
    /// let my_msgpack = MsgPack(string);
    /// assert_eq!(my_msgpack.into_inner(), "Hello".to_string());
    /// ```
    #[inline(always)]
    pub fn into_inner(self) -> T {
        self.0
    }
}

const DEFAULT_LIMIT: ByteUnit = ByteUnit::Mebibyte(1);

impl<'r, T: Deserialize<'r>> MsgPack<T> {
    fn from_bytes(buf: &'r [u8]) -> Result<Self, Error> {
        rmp_serde::from_slice(buf).map(MsgPack)
    }

    async fn from_data(req: &'r Request<'_>, data: Data) -> Result<Self, Error> {
        let size_limit = req.limits().get("msgpack").unwrap_or(DEFAULT_LIMIT);
        let bytes = match data.open(size_limit).into_bytes().await {
            Ok(buf) if buf.is_complete() => buf.into_inner(),
            Ok(_) => {
                let eof = io::ErrorKind::UnexpectedEof;
                return Err(Error::InvalidDataRead(io::Error::new(eof, "data limit exceeded")));
            },
            Err(e) => return Err(Error::InvalidDataRead(e)),
        };

        Self::from_bytes(local_cache!(req, bytes))
    }
}
#[rocket::async_trait]
impl<'r, T: Deserialize<'r>> FromData<'r> for MsgPack<T> {
    type Error = Error;

    async fn from_data(req: &'r Request<'_>, data: Data) -> Outcome<Self, Self::Error> {
        match Self::from_data(req, data).await {
            Ok(value) => Outcome::Success(value),
            Err(Error::InvalidDataRead(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Outcome::Failure((Status::PayloadTooLarge, Error::InvalidDataRead(e)))
            },
            | Err(e@Error::TypeMismatch(_))
            | Err(e@Error::OutOfRange)
            | Err(e@Error::LengthMismatch(_))
            => {
                Outcome::Failure((Status::UnprocessableEntity, e))
            },
            Err(e) => Outcome::Failure((Status::BadRequest, e)),
        }
    }
}

/// Serializes the wrapped value into MessagePack. Returns a response with
/// Content-Type `MsgPack` and a fixed-size body with the serialization. If
/// serialization fails, an `Err` of `Status::InternalServerError` is returned.
impl<'r, T: Serialize> Responder<'r, 'static> for MsgPack<T> {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let buf = rmp_serde::to_vec(&self.0)
            .map_err(|e| {
                error!("MsgPack failed to serialize: {:?}", e);
                Status::InternalServerError
            })?;

        content::MsgPack(buf).respond_to(req)
    }
}

#[rocket::async_trait]
impl<'v, T: DeserializeOwned + Send> form::FromFormField<'v> for MsgPack<T> {
    async fn from_data(f: form::DataField<'v, '_>) -> Result<Self, form::Errors<'v>> {
        Self::from_data(f.request, f.data).await.map_err(|e| {
            match e {
                Error::InvalidMarkerRead(e) | Error::InvalidDataRead(e) => e.into(),
                Error::Utf8Error(e) => e.into(),
                _ => form::Error::custom(e).into(),
            }
        })
    }
}

impl<T> From<T> for MsgPack<T> {
    fn from(value: T) -> Self {
        MsgPack(value)
    }
}

impl<T> Deref for MsgPack<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for MsgPack<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}
