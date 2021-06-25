use std::fmt::Debug;
use std::net::{IpAddr, SocketAddr};

use crate::websocket::{WebSocket, WebSocketStatus};
use crate::{Request, Route};
use crate::outcome::{self, IntoOutcome};
use crate::outcome::Outcome::*;

use crate::http::{Status, ContentType, Accept, Method, CookieJar, uri::Origin};

/// Type alias for the `Outcome` of a `FromRequest` conversion.
pub type Outcome<S, E> = outcome::Outcome<S, (Status, E), ()>;

impl<S, E> IntoOutcome<S, (Status, E), ()> for Result<S, E> {
    type Failure = Status;
    type Forward = ();

    #[inline]
    fn into_outcome(self, status: Status) -> Outcome<S, E> {
        match self {
            Ok(val) => Success(val),
            Err(err) => Failure((status, err))
        }
    }

    #[inline]
    fn or_forward(self, _: ()) -> Outcome<S, E> {
        match self {
            Ok(val) => Success(val),
            Err(_) => Forward(())
        }
    }
}

/// Trait implemented by request guards to derive a value from incoming
/// requests.
///
/// # Request Guards
///
/// A request guard is a type that represents an arbitrary validation policy.
/// The validation policy is implemented through `FromRequest`. In other words,
/// every type that implements `FromRequest` is a request guard.
///
/// Request guards appear as inputs to handlers. An arbitrary number of request
/// guards can appear as arguments in a route handler. Rocket will automatically
/// invoke the `FromRequest` implementation for request guards before calling
/// the handler. Rocket only dispatches requests to a handler when all of its
/// guards pass.
///
/// ## Async Trait
///
/// [`FromRequest`] is an _async_ trait. Implementations of `FromRequest` must
/// be decorated with an attribute of `#[rocket::async_trait]`:
///
/// ```rust
/// use rocket::request::{self, Request, FromRequest};
/// # struct MyType;
/// # type MyError = String;
///
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for MyType {
///     type Error = MyError;
///
///     async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
///         /* .. */
///         # unimplemented!()
///     }
/// }
/// ```
///
/// ## Example
///
/// The following dummy handler makes use of three request guards, `A`, `B`, and
/// `C`. An input type can be identified as a request guard if it is not named
/// in the route attribute. This is why, for instance, `param` is not a request
/// guard.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use rocket::http::Method;
/// # type A = Method; type B = Method; type C = Method; type T = ();
/// #[get("/<param>")]
/// fn index(param: isize, a: A, b: B, c: C) -> T { /* ... */ }
/// # fn main() {}
/// ```
///
/// Request guards always fire in left-to-right declaration order. In the
/// example above, the order is `a` followed by `b` followed by `c`. Failure is
/// short-circuiting; if one guard fails, the remaining are not attempted.
///
/// # Outcomes
///
/// The returned [`Outcome`] of a `from_request` call determines how the
/// incoming request will be processed.
///
/// * **Success**(S)
///
///   If the `Outcome` is [`Success`], then the `Success` value will be used as
///   the value for the corresponding parameter.  As long as all other guards
///   succeed, the request will be handled.
///
/// * **Failure**(Status, E)
///
///   If the `Outcome` is [`Failure`], the request will fail with the given
///   status code and error. The designated error [`Catcher`](crate::Catcher) will be
///   used to respond to the request. Note that users can request types of
///   `Result<S, E>` and `Option<S>` to catch `Failure`s and retrieve the error
///   value.
///
/// * **Forward**
///
///   If the `Outcome` is [`Forward`], the request will be forwarded to the next
///   matching request. Note that users can request an `Option<S>` to catch
///   `Forward`s.
///
/// # Provided Implementations
///
/// Rocket implements `FromRequest` for several built-in types. Their behavior
/// is documented here.
///
///   * **Method**
///
///     Extracts the [`Method`] from the incoming request.
///
///     _This implementation always returns successfully._
///
///   * **&Origin**
///
///     Extracts the [`Origin`] URI from the incoming request.
///
///     _This implementation always returns successfully._
///
///   * **&Route**
///
///     Extracts the [`Route`] from the request if one is available. If a route
///     is not available, the request is forwarded.
///
///     For information on when an `&Route` is available, see
///     [`Request::route()`].
///
///   * **&CookieJar**
///
///     Returns a borrow to the [`CookieJar`] in the incoming request. Note that
///     `CookieJar` implements internal mutability, so a handle to a `CookieJar`
///     allows you to get _and_ set cookies in the request.
///
///     _This implementation always returns successfully._
///
///   * **&[`Config`]**
///
///     Extracts the application [`Config`].
///
///     _This implementation always returns successfully._
///
///   * **ContentType**
///
///     Extracts the [`ContentType`] from the incoming request. If the request
///     didn't specify a Content-Type, the request is forwarded.
///
///   * **IpAddr**
///
///     Extracts the client ip address of the incoming request as an [`IpAddr`].
///     If the client's IP address is not known, the request is forwarded.
///
///   * **SocketAddr**
///
///     Extracts the remote address of the incoming request as a [`SocketAddr`].
///     If the remote address is not known, the request is forwarded.
///
///     _This implementation always returns successfully._
///
///   * **Option&lt;T>** _where_ **T: FromRequest**
///
///     The type `T` is derived from the incoming request using `T`'s
///     `FromRequest` implementation. If the derivation is a `Success`, the
///     derived value is returned in `Some`. Otherwise, a `None` is returned.
///
///     _This implementation always returns successfully._
///
///   * **Result&lt;T, T::Error>** _where_ **T: FromRequest**
///
///     The type `T` is derived from the incoming request using `T`'s
///     `FromRequest` implementation. If derivation is a `Success`, the value is
///     returned in `Ok`. If the derivation is a `Failure`, the error value is
///     returned in `Err`. If the derivation is a `Forward`, the request is
///     forwarded.
///
/// [`Config`]: crate::config::Config
///
/// # Example
///
/// Imagine you're running an authenticated API service that requires that some
/// requests be sent along with a valid API key in a header field. You want to
/// ensure that the handlers corresponding to these requests don't get called
/// unless there is an API key in the request and the key is valid. The
/// following example implements this using an `ApiKey` type and a `FromRequest`
/// implementation for that type. The `ApiKey` type is then used in the
/// `sensitive` handler.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// use rocket::http::Status;
/// use rocket::request::{self, Outcome, Request, FromRequest};
///
/// struct ApiKey<'r>(&'r str);
///
/// #[derive(Debug)]
/// enum ApiKeyError {
///     Missing,
///     Invalid,
/// }
///
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for ApiKey<'r> {
///     type Error = ApiKeyError;
///
///     async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
///         /// Returns true if `key` is a valid API key string.
///         fn is_valid(key: &str) -> bool {
///             key == "valid_api_key"
///         }
///
///         match req.headers().get_one("x-api-key") {
///             None => Outcome::Failure((Status::BadRequest, ApiKeyError::Missing)),
///             Some(key) if is_valid(key) => Outcome::Success(ApiKey(key)),
///             Some(_) => Outcome::Failure((Status::BadRequest, ApiKeyError::Invalid)),
///         }
///     }
/// }
///
/// #[get("/sensitive")]
/// fn sensitive(key: ApiKey<'_>) -> &'static str {
///     "Sensitive data."
/// }
/// ```
///
/// # Request-Local State
///
/// Request guards that perform expensive operations, such as those that query a
/// database or an external service, should use the [request-local state] cache
/// to store results if they might be invoked multiple times during the routing
/// of a single request.
///
/// For example, consider a pair of `User` and `Admin` guards and a pair of
/// routes (`admin_dashboard` and `user_dashboard`):
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[cfg(feature = "secrets")] mod wrapper {
/// # use rocket::outcome::{IntoOutcome, try_outcome};
/// # use rocket::request::{self, Outcome, FromRequest, Request};
/// # struct User { id: String, is_admin: bool }
/// # struct Database;
/// # impl Database {
/// #     fn get_user(&self, id: String) -> Result<User, ()> {
/// #         Ok(User { id, is_admin: false })
/// #     }
/// # }
/// # #[rocket::async_trait]
/// # impl<'r> FromRequest<'r> for Database {
/// #     type Error = ();
/// #     async fn from_request(request: &'r Request<'_>) -> Outcome<Database, ()> {
/// #         Outcome::Success(Database)
/// #     }
/// # }
/// #
/// # struct Admin { user: User }
/// #
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for User {
///     type Error = ();
///
///     async fn from_request(request: &'r Request<'_>) -> Outcome<User, ()> {
///         let db = try_outcome!(request.guard::<Database>().await);
///         request.cookies()
///             .get_private("user_id")
///             .and_then(|cookie| cookie.value().parse().ok())
///             .and_then(|id| db.get_user(id).ok())
///             .or_forward(())
///     }
/// }
///
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for Admin {
///     type Error = ();
///
///     async fn from_request(request: &'r Request<'_>) -> Outcome<Admin, ()> {
///         // This will unconditionally query the database!
///         let user = try_outcome!(request.guard::<User>().await);
///         if user.is_admin {
///             Outcome::Success(Admin { user })
///         } else {
///             Outcome::Forward(())
///         }
///     }
/// }
///
/// #[get("/dashboard")]
/// fn admin_dashboard(admin: Admin) { }
///
/// #[get("/dashboard", rank = 2)]
/// fn user_dashboard(user: User) { }
/// # } // end of cfg wrapper
/// ```
///
/// When a non-admin user is logged in, the database will be queried twice: once
/// via the `Admin` guard invoking the `User` guard, and a second time via the
/// `User` guard directly. For cases like these, request-local state should be
/// used, as illustrated below:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[cfg(feature = "secrets")] mod wrapper {
/// # use rocket::outcome::{IntoOutcome, try_outcome};
/// # use rocket::request::{self, Outcome, FromRequest, Request};
/// # struct User { id: String, is_admin: bool }
/// # struct Database;
/// # impl Database {
/// #     fn get_user(&self, id: String) -> Result<User, ()> {
/// #         Ok(User { id, is_admin: false })
/// #     }
/// # }
/// # #[rocket::async_trait]
/// # impl<'r> FromRequest<'r> for Database {
/// #     type Error = ();
/// #     async fn from_request(request: &'r Request<'_>) -> Outcome<Database, ()> {
/// #         Outcome::Success(Database)
/// #     }
/// # }
/// #
/// # struct Admin<'a> { user: &'a User }
/// #
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for &'r User {
///     type Error = std::convert::Infallible;
///
///     async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
///         // This closure will execute at most once per request, regardless of
///         // the number of times the `User` guard is executed.
///         let user_result = request.local_cache_async(async {
///             let db = request.guard::<Database>().await.succeeded()?;
///             request.cookies()
///                 .get_private("user_id")
///                 .and_then(|cookie| cookie.value().parse().ok())
///                 .and_then(|id| db.get_user(id).ok())
///         }).await;
///
///         user_result.as_ref().or_forward(())
///     }
/// }
///
/// #[rocket::async_trait]
/// impl<'r> FromRequest<'r> for Admin<'r> {
///     type Error = std::convert::Infallible;
///
///     async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
///         let user = try_outcome!(request.guard::<&User>().await);
///         if user.is_admin {
///             Outcome::Success(Admin { user })
///         } else {
///             Outcome::Forward(())
///         }
///     }
/// }
/// # } // end of cfg wrapper
/// ```
///
/// Notice that these request guards provide access to *borrowed* data (`&'a
/// User` and `Admin<'a>`) as the data is now owned by the request's cache.
///
/// [request-local state]: https://rocket.rs/v0.5-rc/guide/state/#request-local-state
#[crate::async_trait]
pub trait FromRequest<'r>: Sized {
    /// The associated error to be returned if derivation fails.
    type Error: Debug;

    /// Derives an instance of `Self` from the incoming request metadata.
    ///
    /// If the derivation is successful, an outcome of `Success` is returned. If
    /// the derivation fails in an unrecoverable fashion, `Failure` is returned.
    /// `Forward` is returned to indicate that the request should be forwarded
    /// to other matching routes, if any.
    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error>;
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Method {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Success(request.method())
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r Origin<'r> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Success(request.uri())
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r Route {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.route() {
            Some(route) => Success(route),
            None => Forward(())
        }
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r CookieJar<'r> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Success(request.cookies())
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r Accept {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.accept() {
            Some(accept) => Success(accept),
            None => Forward(())
        }
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for &'r ContentType {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.content_type() {
            Some(content_type) => Success(content_type),
            None => Forward(())
        }
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for IpAddr {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.client_ip() {
            Some(addr) => Success(addr),
            None => Forward(())
        }
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for SocketAddr {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match request.remote() {
            Some(addr) => Success(addr),
            None => Forward(())
        }
    }
}

#[crate::async_trait]
impl<'r, T: FromRequest<'r>> FromRequest<'r> for Result<T, T::Error> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match T::from_request(request).await {
            Success(val) => Success(Ok(val)),
            Failure((_, e)) => Success(Err(e)),
            Forward(_) => Forward(()),
        }
    }
}

#[crate::async_trait]
impl<'r, T: FromRequest<'r>> FromRequest<'r> for Option<T> {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match T::from_request(request).await {
            Success(val) => Success(Some(val)),
            Failure(_) | Forward(_) => Success(None),
        }
    }
}

/// Type alias for the `Outcome` of a `FromRequest` conversion.
pub type WsOutcome<S, E> = outcome::Outcome<S, (WebSocketStatus<'static>, E), ()>;

impl<S, E> From<Outcome<S, E>> for WsOutcome<S, E> {
    fn from(o: Outcome<S, E>) -> Self {
        match o {
            Outcome::Success(s) => Self::Success(s),
            Outcome::Failure((s, e)) => Self::Failure((s.into(), e)),
            Outcome::Forward(()) => Self::Forward(()),
        }
    }
}

/// Similar to [`FromRequest`], but specifically from a websocket connection
#[crate::async_trait]
pub trait FromWebSocket<'r>: Sized {
    /// The associated error to be returned if derivation fails.
    type Error: Debug;

    /// Derives an instance of `Self` from the incoming request metadata.
    ///
    /// If the derivation is successful, an outcome of `Success` is returned. If
    /// the derivation fails in an unrecoverable fashion, `Failure` is returned.
    /// `Forward` is returned to indicate that the request should be forwarded
    /// to other matching routes, if any.
    async fn from_websocket(request: &'r WebSocket<'_>) -> WsOutcome<Self, Self::Error>;
}

#[crate::async_trait]
impl<'r, T: FromRequest<'r>> FromWebSocket<'r> for T {
    /// The associated error to be returned if derivation fails.
    type Error = <T as FromRequest<'r>>::Error;

    /// Derives an instance of `Self` from the incoming request metadata.
    ///
    /// If the derivation is successful, an outcome of `Success` is returned. If
    /// the derivation fails in an unrecoverable fashion, `Failure` is returned.
    /// `Forward` is returned to indicate that the request should be forwarded
    /// to other matching routes, if any.
    async fn from_websocket(request: &'r WebSocket<'_>) -> WsOutcome<Self, Self::Error> {
        <T as FromRequest<'r>>::from_request(request.request()).await.into()
    }
}
