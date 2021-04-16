use std::fs::File;
use std::io::Cursor;

use crate::http::{Status, ContentType, StatusClass};
use crate::response::{self, Response};
use crate::request::Request;

/// Trait implemented by types that generate responses for clients.
///
/// Types that implement this trait can be used as the return type of a handler,
/// as illustrated below with `T`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # type T = ();
/// #
/// #[get("/")]
/// fn index() -> T { /* ... */ }
/// ```
///
/// In this example, `T` can be any type, as long as it implements `Responder`.
///
/// # Return Value
///
/// A `Responder` returns a `Future` whose output type is an `Ok(Response)` or
/// an `Err(Status)`:
///
///   * An `Ok` variant means that the `Responder` was successful in generating
///     a `Response`. The `Response` will be written out to the client.
///
///   * An `Err` variant means that the `Responder` could not or did not
///     generate a `Response`. The contained `Status` will be used to find the
///     relevant error catcher which then generates an error response.
///
/// # Provided Implementations
///
/// Rocket implements `Responder` for several standard library types. Their
/// behavior is documented here. Note that the `Result` implementation is
/// overloaded, allowing for two `Responder`s to be used at once, depending on
/// the variant.
///
///   * **&str**
///
///     Sets the `Content-Type` to `text/plain`. The string is used as the body
///     of the response, which is fixed size and not streamed. To stream a raw
///     string, use `Stream::from(Cursor::new(string))`.
///
///   * **String**
///
///     Sets the `Content-Type` to `text/plain`. The string is used as the body
///     of the response, which is fixed size and not streamed. To stream a
///     string, use `Stream::from(Cursor::new(string))`.
///
///   * **&\[u8\]**
///
///     Sets the `Content-Type` to `application/octet-stream`. The slice
///     is used as the body of the response, which is fixed size and not
///     streamed. To stream a slice of bytes, use
///     `Stream::from(Cursor::new(data))`.
///
///   * **Vec&lt;u8>**
///
///     Sets the `Content-Type` to `application/octet-stream`. The vector's data
///     is used as the body of the response, which is fixed size and not
///     streamed. To stream a vector of bytes, use
///     `Stream::from(Cursor::new(vec))`.
///
///   * **File**
///
///     Responds with a streamed body containing the data in the `File`. No
///     `Content-Type` is set. To automatically have a `Content-Type` set based
///     on the file's extension, use [`NamedFile`](crate::response::NamedFile).
///
///   * **()**
///
///     Responds with an empty body. No `Content-Type` is set.
///
///   * **Option&lt;T>**
///
///     If the `Option` is `Some`, the wrapped responder is used to respond to
///     the client. Otherwise, an `Err` with status **404 Not Found** is
///     returned and a warning is printed to the console.
///
///   * **Result&lt;T, E>**
///
///     If the `Result` is `Ok`, the wrapped `Ok` responder is used to respond
///     to the client. If the `Result` is `Err`, the wrapped `Err` responder is
///     used to respond to the client.
///
/// # Implementation Tips
///
/// This section describes a few best practices to take into account when
/// implementing `Responder`.
///
/// ## Joining and Merging
///
/// When chaining/wrapping other `Responder`s, use the
/// [`merge()`](Response::merge()) or [`join()`](Response::join()) methods on
/// the `Response` or `ResponseBuilder` struct. Ensure that you document the
/// merging or joining behavior appropriately.
///
/// ## Inspecting Requests
///
/// A `Responder` has access to the request it is responding to. Even so, you
/// should avoid using the `Request` value as much as possible. This is because
/// using the `Request` object makes your responder _impure_, and so the use of
/// the type as a `Responder` has less intrinsic meaning associated with it. If
/// the `Responder` were pure, however, it would always respond in the same manner,
/// regardless of the incoming request. Thus, knowing the type is sufficient to
/// fully determine its functionality.
///
/// ## Lifetimes
///
/// `Responder` has two lifetimes: `Responder<'r, 'o: 'r>`. The first lifetime,
/// `'r`, refers to the reference to the `&'r Request`, while the second
/// lifetime refers to the returned `Response<'o>`. The bound `'o: 'r` allows
/// `'o` to be any lifetime that lives at least as long as the `Request`. In
/// particular, this includes borrows from the `Request` itself (where `'o` would
/// be `'r` as in `impl<'r> Responder<'r, 'r>`) as well as `'static` data (where
/// `'o` would be `'static` as in `impl<'r> Responder<'r, 'static>`).
///
/// # Example
///
/// Say that you have a custom type, `Person`:
///
/// ```rust
/// struct Person {
///     name: String,
///     age: u16
/// }
/// ```
///
/// You'd like to use `Person` as a `Responder` so that you can return a
/// `Person` directly from a handler:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # type Person = String;
/// #[get("/person/<id>")]
/// fn person(id: usize) -> Option<Person> {
///     # /*
///     Person::from_id(id)
///     # */ None
/// }
/// # fn main() {}
/// ```
///
/// You want the `Person` responder to set two header fields: `X-Person-Name`
/// and `X-Person-Age` as well as supply a custom representation of the object
/// (`Content-Type: application/x-person`) in the body of the response. The
/// following `Responder` implementation accomplishes this:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// # #[derive(Debug)]
/// # struct Person { name: String, age: u16 }
/// #
/// use std::io::Cursor;
///
/// use rocket::request::Request;
/// use rocket::response::{self, Response, Responder};
/// use rocket::http::ContentType;
///
/// impl<'r> Responder<'r, 'static> for Person {
///     fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
///         let person_string = format!("{}:{}", self.name, self.age);
///         Response::build()
///             .sized_body(person_string.len(), Cursor::new(person_string))
///             .raw_header("X-Person-Name", self.name)
///             .raw_header("X-Person-Age", self.age.to_string())
///             .header(ContentType::new("application", "x-person"))
///             .ok()
///     }
/// }
/// #
/// # #[get("/person")]
/// # fn person() -> Person { Person { name: "a".to_string(), age: 20 } }
/// # fn main() {  }
/// ```
pub trait Responder<'r, 'o: 'r> {
    /// Returns `Ok` if a `Response` could be generated successfully. Otherwise,
    /// returns an `Err` with a failing `Status`.
    ///
    /// The `request` parameter is the `Request` that this `Responder` is
    /// responding to.
    ///
    /// When using Rocket's code generation, if an `Ok(Response)` is returned,
    /// the response will be written out to the client. If an `Err(Status)` is
    /// returned, the error catcher for the given status is retrieved and called
    /// to generate a final error response, which is then written out to the
    /// client.
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'o>;
}

/// Returns a response with Content-Type `text/plain` and a fixed-size body
/// containing the string `self`. Always returns `Ok`.
impl<'r, 'o: 'r> Responder<'r, 'o> for &'o str {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .header(ContentType::Plain)
            .sized_body(self.len(), Cursor::new(self))
            .ok()
    }
}

/// Returns a response with Content-Type `text/plain` and a fixed-size body
/// containing the string `self`. Always returns `Ok`.
impl<'r> Responder<'r, 'static> for String {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::Plain)
            .sized_body(self.len(), Cursor::new(self))
            .ok()
    }
}

/// Returns a response with Content-Type `application/octet-stream` and a
/// fixed-size body containing the data in `self`. Always returns `Ok`.
impl<'r, 'o: 'r> Responder<'r, 'o> for &'o [u8] {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'o> {
        Response::build()
            .header(ContentType::Binary)
            .sized_body(self.len(), Cursor::new(self))
            .ok()
    }
}

/// Returns a response with Content-Type `application/octet-stream` and a
/// fixed-size body containing the data in `self`. Always returns `Ok`.
impl<'r> Responder<'r, 'static> for Vec<u8> {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build()
            .header(ContentType::Binary)
            .sized_body(self.len(), Cursor::new(self))
            .ok()
    }
}

/// Returns a response with a sized body for the file. Always returns `Ok`.
impl<'r> Responder<'r, 'static> for File {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        tokio::fs::File::from(self).respond_to(req)
    }
}

/// Returns a response with a sized body for the file. Always returns `Ok`.
impl<'r> Responder<'r, 'static> for tokio::fs::File {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Response::build().sized_body(None, self).ok()
    }
}

/// Returns an empty, default `Response`. Always returns `Ok`.
impl<'r> Responder<'r, 'static> for () {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        Ok(Response::new())
    }
}

/// Responds with the inner `Responder` in `Cow`.
impl<'r, 'o: 'r, R: ?Sized + ToOwned> Responder<'r, 'o> for std::borrow::Cow<'o, R>
    where &'o R: Responder<'r, 'o> + 'o, <R as ToOwned>::Owned: Responder<'r, 'o> + 'r
{
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        match self {
            std::borrow::Cow::Borrowed(b) => b.respond_to(req),
            std::borrow::Cow::Owned(o) => o.respond_to(req),
        }
    }
}

/// If `self` is `Some`, responds with the wrapped `Responder`. Otherwise prints
/// a warning message and returns an `Err` of `Status::NotFound`.
impl<'r, 'o: 'r, R: Responder<'r, 'o>> Responder<'r, 'o> for Option<R> {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        match self {
            Some(r) => r.respond_to(req),
            None => {
                warn!("Response was `None`.");
                Err(Status::NotFound)
            },
        }
    }
}

// Responds with the wrapped `Responder` in `self`, whether it is `Ok` or
/// `Err`.
impl<'r, 'o: 'r, 't: 'o, 'e: 'o, T, E> Responder<'r, 'o> for Result<T, E>
    where T: Responder<'r, 't>, E: Responder<'r, 'e>
{
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        match self {
            Ok(responder) => responder.respond_to(req),
            Err(responder) => responder.respond_to(req),
        }
    }
}

// Responds with the wrapped `Responder` in `self`, whether it is `Left` or
/// `Right`.
impl<'r, 'o: 'r, 't: 'o, 'e: 'o, T, E> Responder<'r, 'o> for crate::Either<T, E>
    where T: Responder<'r, 't>, E: Responder<'r, 'e>
{
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        match self {
            crate::Either::Left(r) => r.respond_to(req),
            crate::Either::Right(r) => r.respond_to(req),
        }
    }
}

/// The response generated by `Status` depends on the status code itself. The
/// table below summarizes the functionality:
///
/// | Status Code Range | Response                              |
/// |-------------------|---------------------------------------|
/// | [400, 599]        | Forwards to catcher for given status. |
/// | 100, [200, 205]   | Empty with status of `self`.          |
/// | All others.       | Invalid. Errors to `500` catcher.     |
///
/// In short, a client or server error status codes will forward to the
/// corresponding error catcher, a successful status code less than `206` or
/// `100` responds with any empty body and the given status code, and all other
/// status code emit an error message and forward to the `500` (internal server
/// error) catcher.
impl<'r> Responder<'r, 'static> for Status {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        match self.class() {
            StatusClass::ClientError | StatusClass::ServerError => Err(self),
            StatusClass::Success if self.code < 206 => {
                Response::build().status(self).ok()
            }
            StatusClass::Informational if self.code == 100 => {
                Response::build().status(self).ok()
            }
            _ => {
                error!("Invalid status used as responder: {}.", self);
                Err(Status::InternalServerError)
            }
        }
    }
}
