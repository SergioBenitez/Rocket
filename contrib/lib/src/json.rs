//! Automatic JSON (de)serialization support.
//!
//! See the [`Json`](crate::json::Json) type for further details.
//!
//! # Enabling
//!
//! This module is only available when the `json` feature is enabled. Enable it
//! in `Cargo.toml` as follows:
//!
//! ```toml
//! [dependencies.rocket_contrib]
//! version = "0.5.0-dev"
//! default-features = false
//! features = ["json"]
//! ```

use std::io;
use std::ops::{Deref, DerefMut};
use std::iter::FromIterator;

use rocket::request::{Request, local_cache};
use rocket::data::{ByteUnit, Data, FromData, Outcome};
use rocket::response::{self, Responder, content};
use rocket::http::Status;
use rocket::form::prelude as form;

use serde::{Serialize, Serializer};
use serde::de::{Deserialize, DeserializeOwned, Deserializer};

#[doc(hidden)]
pub use serde_json::{json_internal, json_internal_vec};

/// The JSON data guard: easily consume and respond with JSON.
///
/// ## Receiving JSON
///
/// `Json` is both a data guard and a form guard.
///
/// ### Data Guard
///
/// To parse request body data as JSON , add a `data` route argument with a
/// target type of `Json<T>`, where `T` is some type you'd like to parse from
/// JSON. `T` must implement [`serde::Deserialize`].
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type User = usize;
/// use rocket_contrib::json::Json;
///
/// #[post("/user", format = "json", data = "<user>")]
/// fn new_user(user: Json<User>) {
///     /* ... */
/// }
/// ```
///
/// You don't _need_ to use `format = "json"`, but it _may_ be what you want.
/// Using `format = json` means that any request that doesn't specify
/// "application/json" as its `Content-Type` header value will not be routed to
/// the handler.
///
/// ### Form Guard
///
/// `Json<T>`, as a form guard, accepts value and data fields and parses the
/// data as a `T`. Simple use `Json<T>`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type Metadata = usize;
/// use rocket::form::{Form, FromForm};
/// use rocket_contrib::json::Json;
///
/// #[derive(FromForm)]
/// struct User<'r> {
///     name: &'r str,
///     metadata: Json<Metadata>
/// }
///
/// #[post("/user", data = "<form>")]
/// fn new_user(form: Form<User<'_>>) {
///     /* ... */
/// }
/// ```
///
/// ## Sending JSON
///
/// If you're responding with JSON data, return a `Json<T>` type, where `T`
/// implements [`Serialize`] from [`serde`]. The content type of the response is
/// set to `application/json` automatically.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # extern crate rocket_contrib;
/// # type User = usize;
/// use rocket_contrib::json::Json;
///
/// #[get("/users/<id>")]
/// fn user(id: usize) -> Json<User> {
///     let user_from_id = User::from(id);
///     /* ... */
///     Json(user_from_id)
/// }
/// ```
///
/// ## Incoming Data Limits
///
/// The default size limit for incoming JSON data is 1MiB. Setting a limit
/// protects your application from denial of service (DoS) attacks and from
/// resource exhaustion through high memory consumption. The limit can be
/// increased by setting the `limits.json` configuration parameter. For
/// instance, to increase the JSON limit to 5MiB for all environments, you may
/// add the following to your `Rocket.toml`:
///
/// ```toml
/// [global.limits]
/// json = 5242880
/// ```
#[derive(Debug)]
pub struct Json<T>(pub T);

/// An error returned by the [`Json`] data guard when incoming data fails to
/// serialize as JSON.
#[derive(Debug)]
pub enum JsonError<'a> {
    /// An I/O error occurred while reading the incoming request data.
    Io(io::Error),

    /// The client's data was received successfully but failed to parse as valid
    /// JSON or as the requested type. The `&str` value in `.0` is the raw data
    /// received from the user, while the `Error` in `.1` is the deserialization
    /// error from `serde`.
    Parse(&'a str, serde_json::error::Error),
}

const DEFAULT_LIMIT: ByteUnit = ByteUnit::Mebibyte(1);

impl<T> Json<T> {
    /// Consumes the JSON wrapper and returns the wrapped item.
    ///
    /// # Example
    /// ```rust
    /// # use rocket_contrib::json::Json;
    /// let string = "Hello".to_string();
    /// let my_json = Json(string);
    /// assert_eq!(my_json.into_inner(), "Hello".to_string());
    /// ```
    #[inline(always)]
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<'r, T: Deserialize<'r>> Json<T> {
    fn from_str(s: &'r str) -> Result<Self, JsonError<'r>> {
        serde_json::from_str(s).map(Json).map_err(|e| JsonError::Parse(s, e))
    }

    async fn from_data(req: &'r Request<'_>, data: Data) -> Result<Self, JsonError<'r>> {
        let size_limit = req.limits().get("json").unwrap_or(DEFAULT_LIMIT);
        let string = match data.open(size_limit).into_string().await {
            Ok(s) if s.is_complete() => s.into_inner(),
            Ok(_) => {
                let eof = io::ErrorKind::UnexpectedEof;
                return Err(JsonError::Io(io::Error::new(eof, "data limit exceeded")));
            },
            Err(e) => return Err(JsonError::Io(e)),
        };

        Self::from_str(local_cache!(req, string))
    }
}

#[rocket::async_trait]
impl<'r, T: Deserialize<'r>> FromData<'r> for Json<T> {
    type Error = JsonError<'r>;

    async fn from_data(req: &'r Request<'_>, data: Data) -> Outcome<Self, Self::Error> {
        match Self::from_data(req, data).await {
            Ok(value) => Outcome::Success(value),
            Err(JsonError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Outcome::Failure((Status::PayloadTooLarge, JsonError::Io(e)))
            },
            Err(JsonError::Parse(s, e)) if e.classify() == serde_json::error::Category::Data => {
                Outcome::Failure((Status::UnprocessableEntity, JsonError::Parse(s, e)))
            },
            Err(e) => Outcome::Failure((Status::BadRequest, e)),

        }
    }
}

/// Serializes the wrapped value into JSON. Returns a response with Content-Type
/// JSON and a fixed-size body with the serialized value. If serialization
/// fails, an `Err` of `Status::InternalServerError` is returned.
impl<'r, T: Serialize> Responder<'r, 'static> for Json<T> {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let string = serde_json::to_string(&self.0)
            .map_err(|e| {
                error!("JSON failed to serialize: {:?}", e);
                Status::InternalServerError
            })?;

        content::Json(string).respond_to(req)
    }
}

impl<T> From<T> for Json<T> {
    fn from(value: T) -> Self {
        Json(value)
    }
}

impl<T> Deref for Json<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl From<JsonError<'_>> for form::Error<'_> {
    fn from(e: JsonError<'_>) -> Self {
        match e {
            JsonError::Io(e) => e.into(),
            JsonError::Parse(_, e) => form::Error::custom(e)
        }
    }
}

#[rocket::async_trait]
impl<'v, T: DeserializeOwned + Send> form::FromFormField<'v> for Json<T> {
    fn from_value(field: form::ValueField<'v>) -> Result<Self, form::Errors<'v>> {
        Ok(Self::from_str(field.value)?)
    }

    async fn from_data(f: form::DataField<'v, '_>) -> Result<Self, form::Errors<'v>> {
        Ok(Self::from_data(f.request, f.data).await?)
    }
}

/// An arbitrary JSON value.
///
/// This structure wraps `serde`'s [`Value`] type. Importantly, unlike `Value`,
/// this type implements [`Responder`], allowing a value of this type to be
/// returned directly from a handler.
///
/// [`Value`]: serde_json::value
/// [`Responder`]: rocket::response::Responder
///
/// # `Responder`
///
/// The `Responder` implementation for `JsonValue` serializes the represented
/// value into a JSON string and sets the string as the body of a fixed-sized
/// response with a `Content-Type` of `application/json`.
///
/// # Usage
///
/// A value of this type is constructed via the [`json!`](json) macro. The macro
/// and this type are typically used to construct JSON values in an ad-hoc
/// fashion during request handling. This looks something like:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[macro_use] extern crate rocket_contrib;
/// use rocket_contrib::json::JsonValue;
///
/// #[get("/json")]
/// fn get_json() -> JsonValue {
///     json!({
///         "id": 83,
///         "values": [1, 2, 3, 4]
///     })
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsonValue(pub serde_json::Value);

impl Serialize for JsonValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde_json::Value::deserialize(deserializer).map(JsonValue)
    }
}

impl JsonValue {
    #[inline(always)]
    fn into_inner(self) -> serde_json::Value {
        self.0
    }
}

impl Deref for JsonValue {
    type Target = serde_json::Value;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for JsonValue {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Into<serde_json::Value> for JsonValue {
    #[inline(always)]
    fn into(self) -> serde_json::Value {
        self.into_inner()
    }
}

impl From<serde_json::Value> for JsonValue {
    #[inline(always)]
    fn from(value: serde_json::Value) -> JsonValue {
        JsonValue(value)
    }
}

impl<T> FromIterator<T> for JsonValue where serde_json::Value: FromIterator<T> {
    fn from_iter<I: IntoIterator<Item=T>>(iter: I) -> Self {
        JsonValue(serde_json::Value::from_iter(iter))
    }
}

/// Serializes the value into JSON. Returns a response with Content-Type JSON
/// and a fixed-size body with the serialized value.
impl<'r> Responder<'r, 'static> for JsonValue {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        content::Json(self.0.to_string()).respond_to(req)
    }
}

/// A macro to create ad-hoc JSON serializable values using JSON syntax.
///
/// # Usage
///
/// To import the macro, add the `#[macro_use]` attribute to the `extern crate
/// rocket_contrib` invocation:
///
/// ```rust
/// #[macro_use] extern crate rocket_contrib;
/// ```
///
/// The return type of a `json!` invocation is
/// [`JsonValue`](crate::json::JsonValue). A value created with this macro can
/// be returned from a handler as follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[macro_use] extern crate rocket_contrib;
/// use rocket_contrib::json::JsonValue;
///
/// #[get("/json")]
/// fn get_json() -> JsonValue {
///     json!({
///         "key": "value",
///         "array": [1, 2, 3, 4]
///     })
/// }
/// ```
///
/// The [`Responder`](rocket::response::Responder) implementation for
/// `JsonValue` serializes the value into a JSON string and sets it as the body
/// of the response with a `Content-Type` of `application/json`.
///
/// # Examples
///
/// Create a simple JSON object with two keys: `"username"` and `"id"`:
///
/// ```rust
/// # #![allow(unused_variables)]
/// # #[macro_use] extern crate rocket_contrib;
/// # fn main() {
/// let value = json!({
///     "username": "mjordan",
///     "id": 23
/// });
/// # }
/// ```
///
/// Create a more complex object with a nested object and array:
///
/// ```rust
/// # #![allow(unused_variables)]
/// # #[macro_use] extern crate rocket_contrib;
/// # fn main() {
/// let value = json!({
///     "code": 200,
///     "success": true,
///     "payload": {
///         "features": ["serde", "json"],
///         "ids": [12, 121],
///     },
/// });
/// # }
/// ```
///
/// Variables or expressions can be interpolated into the JSON literal. Any type
/// interpolated into an array element or object value must implement serde's
/// `Serialize` trait, while any type interpolated into a object key must
/// implement `Into<String>`.
///
/// ```rust
/// # #![allow(unused_variables)]
/// # #[macro_use] extern crate rocket_contrib;
/// # fn main() {
/// let code = 200;
/// let features = vec!["serde", "json"];
///
/// let value = json!({
///    "code": code,
///    "success": code == 200,
///    "payload": {
///        features[0]: features[1]
///    }
/// });
/// # }
/// ```
///
/// Trailing commas are allowed inside both arrays and objects.
///
/// ```rust
/// # #![allow(unused_variables)]
/// # #[macro_use] extern crate rocket_contrib;
/// # fn main() {
/// let value = json!([
///     "notice",
///     "the",
///     "trailing",
///     "comma -->",
/// ]);
/// # }
/// ```
#[macro_export]
macro_rules! json {
    ($($json:tt)+) => {
        $crate::json::JsonValue($crate::json::json_internal!($($json)+))
    };
}

#[doc(inline)]
pub use json;
