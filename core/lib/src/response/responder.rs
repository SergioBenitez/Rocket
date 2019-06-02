use std::fs::File;
use std::io::{self, Cursor, BufReader};
use std::fmt;
use std::str::FromStr;

use http::{Status, ContentType, StatusClass, Method};
use http::hyper::header::{AcceptRanges, Range, RangeUnit};
use response::{self, Response, Body};
use request::Request;

/// Trait implemented by types that generate responses for clients.
///
/// Types that implement this trait can be used as the return type of a handler,
/// as illustrated below with `T`:
///
/// ```rust
/// # #![feature(proc_macro_hygiene, decl_macro)]
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
/// A `Responder` returns an `Ok(Response)` or an `Err(Status)`:
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
///     streamed. Supports range requests. To stream a slice of bytes, use
///     `Stream::from(Cursor::new(data))`.
///
///   * **Vec&lt;u8>**
///
///     Sets the `Content-Type` to `application/octet-stream`. The vector's data
///     is used as the body of the response, which is fixed size and not
///     streamed. Supports range requests. To stream a vector of bytes, use
///     `Stream::from(Cursor::new(vec))`.
///
///   * **File**
///
///     Responds with a streamed body containing the data in the `File`. No
///     `Content-Type` is set. Supports range requests.
///     To automatically have a `Content-Type` set based
///     on the file's extension, use [`NamedFile`](::response::NamedFile).
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
///   * **Result&lt;T, E>** _where_ **E: Debug**
///
///     If the `Result` is `Ok`, the wrapped responder is used to respond to the
///     client. Otherwise, an `Err` with status **500 Internal Server Error** is
///     returned and the error is printed to the console using the `Debug`
///     implementation.
///
///   * **Result&lt;T, E>** _where_ **E: Debug + Responder**
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
/// ## Debug
///
/// A type implementing `Responder` should implement the `Debug` trait when
/// possible. This is because the `Responder` implementation for `Result`
/// requires its `Err` type to implement `Debug`. Therefore, a type implementing
/// `Debug` can more easily be composed.
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
/// # Example
///
/// Say that you have a custom type, `Person`:
///
/// ```rust
///
/// # #[allow(dead_code)]
/// struct Person {
///     name: String,
///     age: u16
/// }
/// ```
///
/// You'd like to use `Person` as a `Responder` so that you can return a
/// `Person` directly from a handler:
///
/// ```rust,ignore
/// #[get("/person/<id>")]
/// fn person(id: usize) -> Option<Person> {
///     Person::from_id(id)
/// }
/// ```
///
/// You want the `Person` responder to set two header fields: `X-Person-Name`
/// and `X-Person-Age` as well as supply a custom representation of the object
/// (`Content-Type: application/x-person`) in the body of the response. The
/// following `Responder` implementation accomplishes this:
///
/// ```rust
/// # #![feature(proc_macro_hygiene, decl_macro)]
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
/// impl<'r> Responder<'r> for Person {
///     fn respond_to(self, _: &Request) -> response::Result<'r> {
///         Response::build()
///             .sized_body(Cursor::new(format!("{}:{}", self.name, self.age)))
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
pub trait Responder<'r> {
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
    fn respond_to(self, request: &Request) -> response::Result<'r>;
}

/// Returns a response with Content-Type `text/plain` and a fixed-size body
/// containing the string `self`. Always returns `Ok`.
impl<'r> Responder<'r> for &'r str {
    fn respond_to(self, _: &Request) -> response::Result<'r> {
        Response::build()
            .header(ContentType::Plain)
            .sized_body(Cursor::new(self))
            .ok()
    }
}

/// Returns a response with Content-Type `text/plain` and a fixed-size body
/// containing the string `self`. Always returns `Ok`.
impl<'r> Responder<'r> for String {
    fn respond_to(self, _: &Request) -> response::Result<'r> {
        Response::build()
            .header(ContentType::Plain)
            .sized_body(Cursor::new(self))
            .ok()
    }
}

/// A special responder type, that may be used by custom [`Responder`][response::Responder]
/// implementation to wrap any type implementing [`io::Seek`](std::io::Seek) and [`io::Read`](std::io::Read)
/// to support [range requests](https://developer.mozilla.org/en-US/docs/Web/HTTP/Range_requests).
///
/// # **Note:**
/// `&[u8]`, `Vec<u8>`, [`File`](std::fs::File) and [`NamedFile`](response::NamedFile)
/// are implemented using `RangeResponder`. There is no need to wrap these types,
/// they do support range requests out of the box.
///
/// # Example
/// ```rust
/// # #![feature(proc_macro_hygiene, decl_macro)]
/// # #[macro_use] extern crate rocket;
/// #
/// # struct CustomIoType;
/// # impl std::io::Read for CustomIoType {
/// #     fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> { Ok(0) }
/// # }
/// # impl std::io::Seek for CustomIoType {
/// #     fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> { Ok(0) }
/// # }
/// #
/// use rocket::request::Request;
/// use rocket::response::{self, Response, Responder, RangeResponder};
/// use rocket::http::ContentType;
///
/// impl<'r> Responder<'r> for CustomIoType {
///     fn respond_to(self, req: &Request) -> response::Result<'r> {
///         Response::build_from(RangeResponder(self).respond_to(req)?)
///             .header(ContentType::Binary)
///             .ok()
///     }
/// }
/// ```
pub struct RangeResponder<B: io::Seek + io::Read>(pub B);

impl<'r, B: io::Seek + io::Read + 'r> Responder<'r> for RangeResponder<B> {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        use http::hyper::header::{ContentRange, ByteRangeSpec, ContentRangeSpec};

        let mut body = self.0;
        //  A server MUST ignore a Range header field received with a request method other than GET.
        if req.method() == Method::Get {
            let range = req.headers().get_one("Range").map(|x| Range::from_str(x));
            match range {
                Some(Ok(Range::Bytes(ranges))) => {
                    if ranges.len() == 1 {
                        let size = body.seek(io::SeekFrom::End(0))
                            .expect("Attempted to retrieve size by seeking, but failed.");

                        let (start, end) = match ranges[0] {
                            ByteRangeSpec::FromTo(start, mut end) => {
                                // make end exclusive
                                end += 1;
                                if end > size {
                                    end = size;
                                }
                                (start, end)
                            },
                            ByteRangeSpec::AllFrom(start) => {
                                (start, size)
                            },
                            ByteRangeSpec::Last(len) => {
                                // we could seek to SeekFrom::End(-len), but if we reach a value < 0, that is an error.
                                // but the RFC reads:
                                //      If the selected representation is shorter than the specified
                                //      suffix-length, the entire representation is used.
                                let start = size.checked_sub(len).unwrap_or(0);
                                (start, size)
                            }
                        };

                        if start > size {
                            return Response::build()
                                .status(Status::RangeNotSatisfiable)
                                .header(AcceptRanges(vec![RangeUnit::Bytes]))
                                .ok()
                        }

                        body.seek(io::SeekFrom::Start(start))
                            .expect("Attempted to seek to the start of the requested range, but failed.");

                        return Response::build()
                            .status(Status::PartialContent)
                            .header(AcceptRanges(vec![RangeUnit::Bytes]))
                            .header(ContentRange(ContentRangeSpec::Bytes {
                                // make end inclusive again
                                range: Some((start, end - 1)),
                                instance_length: Some(size),
                            }))
                            .raw_body(Body::Sized(body, end - start))
                            .ok()
                    }
                    // A server MAY ignore the Range header field.
                },
                // An origin server MUST ignore a Range header field that contains a
                // range unit it does not understand.
                Some(Ok(Range::Unregistered(_, _))) => {},
                Some(Err(_)) => {
                    // Malformed
                    return Response::build()
                        .status(Status::RangeNotSatisfiable)
                        .header(AcceptRanges(vec![RangeUnit::Bytes]))
                        .ok()
                }
                None => {},
            };
        }

        Response::build()
            .header(AcceptRanges(vec![RangeUnit::Bytes]))
            .sized_body(body)
            .ok()
    }
}

/// Returns a response with Content-Type `application/octet-stream` and a
/// fixed-size body containing the data in `self`. Always returns `Ok`.
impl<'r> Responder<'r> for &'r [u8] {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        Response::build_from(RangeResponder(Cursor::new(self)).respond_to(req)?)
            .header(ContentType::Binary)
            .ok()
    }
}

/// Returns a response with Content-Type `application/octet-stream` and a
/// fixed-size body containing the data in `self`. Always returns `Ok`.
impl<'r> Responder<'r> for Vec<u8> {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        Response::build_from(RangeResponder(Cursor::new(self)).respond_to(req)?)
            .header(ContentType::Binary)
            .ok()
    }
}

/// Returns a response with a sized body for the file. Always returns `Ok`.
impl<'r> Responder<'r> for File {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        let (metadata, file) = (self.metadata(), BufReader::new(self));
        match metadata {
            Ok(_) => RangeResponder(file).respond_to(req),
            Err(_) => Response::build().streamed_body(file).ok()
        }
    }
}

/// Returns an empty, default `Response`. Always returns `Ok`.
impl<'r> Responder<'r> for () {
    fn respond_to(self, _: &Request) -> response::Result<'r> {
        Ok(Response::new())
    }
}

/// If `self` is `Some`, responds with the wrapped `Responder`. Otherwise prints
/// a warning message and returns an `Err` of `Status::NotFound`.
impl<'r, R: Responder<'r>> Responder<'r> for Option<R> {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        self.map_or_else(|| {
            warn_!("Response was `None`.");
            Err(Status::NotFound)
        }, |r| r.respond_to(req))
    }
}

/// If `self` is `Ok`, responds with the wrapped `Responder`. Otherwise prints
/// an error message with the `Err` value returns an `Err` of
/// `Status::InternalServerError`.
impl<'r, R: Responder<'r>, E: fmt::Debug> Responder<'r> for Result<R, E> {
    default fn respond_to(self, req: &Request) -> response::Result<'r> {
        self.map(|r| r.respond_to(req)).unwrap_or_else(|e| {
            error_!("Response was a non-`Responder` `Err`: {:?}.", e);
            Err(Status::InternalServerError)
        })
    }
}

/// Responds with the wrapped `Responder` in `self`, whether it is `Ok` or
/// `Err`.
impl<'r, R: Responder<'r>, E: Responder<'r> + fmt::Debug> Responder<'r> for Result<R, E> {
    fn respond_to(self, req: &Request) -> response::Result<'r> {
        match self {
            Ok(responder) => responder.respond_to(req),
            Err(responder) => responder.respond_to(req),
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
impl<'r> Responder<'r> for Status {
    fn respond_to(self, _: &Request) -> response::Result<'r> {
        match self.class() {
            StatusClass::ClientError | StatusClass::ServerError => Err(self),
            StatusClass::Success if self.code < 206 => {
                Response::build().status(self).ok()
            }
            StatusClass::Informational if self.code == 100 => {
                Response::build().status(self).ok()
            }
            _ => {
                error_!("Invalid status used as responder: {}.", self);
                warn_!("Fowarding to 500 (Internal Server Error) catcher.");
                Err(Status::InternalServerError)
            }
        }
    }
}
