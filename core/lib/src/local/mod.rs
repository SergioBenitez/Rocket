//! Structures for local dispatching of requests, primarily for testing.
//!
//! This module allows for simple request dispatching against a local,
//! non-networked instance of Rocket. The primary use of this module is to unit
//! and integration test Rocket applications by crafting requests, dispatching
//! them, and verifying the response.
//!
//! # Async. vs. Blocking
//!
//! This module contains two variants, in its two submodules, of the same local
//! API: [`asynchronous`], and [`blocking`]. As their names imply, the
//! `asynchronous` API is `async`, returning a `Future` for operations that
//! would otherwise block, while `blocking` blocks for the same operations.
//!
//! Unless your request handling requires concurrency to make progress, or
//! you're making use of a `Client` in an environment that necessitates or would
//! benefit from concurrency, you should use the `blocking` set of APIs due
//! their ease-of-use. If your request handling _does_ require concurrency to
//! make progress, for instance by having one handler `await` a response
//! generated from a request to another handler, use the `asynchronous` set of
//! APIs.
//!
//! Both APIs include a `Client` structure that is used to create `LocalRequest`
//! structures that can be dispatched against a given [`Rocket`](crate::Rocket)
//! instance to yield a `LocalResponse` structure. The APIs are identical except
//! in that the `asynchronous` APIs return `Future`s for otherwise blocking
//! operations.
//!
//! # Unit/Integration Testing
//!
//! This module is primarily intended to be used to test a Rocket application by
//! constructing requests via `Client`, dispatching them, and validating the
//! resulting response. As a complete example, consider the following "Hello,
//! world!" application, with testing.
//!
//! ```rust
//! #[macro_use] extern crate rocket;
//!
//! #[get("/")]
//! fn hello() -> &'static str {
//!     "Hello, world!"
//! }
//!
//! #[launch]
//! fn rocket() -> _ {
//!     rocket::build().mount("/", routes![hello])
//! }
//!
//! #[cfg(test)]
//! mod test {
//!     // Using the preferred `blocking` API.
//!     #[test]
//!     fn test_hello_world_blocking() {
//!         use rocket::local::blocking::Client;
//!
//!         // Construct a client to use for dispatching requests.
//!         let client = Client::tracked(super::rocket())
//!             .expect("valid `Rocket`");
//!
//!         // Dispatch a request to 'GET /' and validate the response.
//!         let response = client.get("/").dispatch();
//!         assert_eq!(response.into_string().unwrap(), "Hello, world!");
//!     }
//!
//!     // Using the `asynchronous` API.
//!     #[rocket::async_test]
//!     async fn test_hello_world_async() {
//!         use rocket::local::asynchronous::Client;
//!
//!         // Construct a client to use for dispatching requests.
//!         let client = Client::tracked(super::rocket()).await
//!             .expect("valid `Rocket`");
//!
//!         // Dispatch a request to 'GET /' and validate the response.
//!         let response = client.get("/").dispatch().await;
//!         assert_eq!(response.into_string().await.unwrap(), "Hello, world!");
//!     }
//! }
//! ```
//!
//! For more details on testing, see the [testing guide].
//!
//! [testing guide]: https://rocket.rs/v0.5-rc/guide/testing/
//! [`Client`]: crate::local::asynchronous::Client
//!
//! # `Client`
//!
//! A `Client`, either [`blocking::Client`] or [`asynchronous::Client`],
//! referred to as simply [`Client`] and [`async` `Client`], respectively,
//! constructs requests for local dispatching.
//!
//! **Usage**
//!
//! A `Client` is constructed via the [`tracked()`] ([`async` `tracked()`]) or
//! [`untracked()`] ([`async` `untracked()`]) methods from an already
//! constructed `Rocket` instance. Once a value of `Client` has been
//! constructed, [`get()`], [`put()`], [`post()`], and so on ([`async` `get()`],
//! [`async` `put()`], [`async` `post()`]) can be called to create a
//! [`LocalRequest`] ([`async` `LocalRequest`]) for dispatching.
//!
//! **Cookie Tracking**
//!
//! A `Client` constructed using `tracked()` propagates cookie changes made by
//! responses to previously dispatched requests. In other words, if a previously
//! dispatched request resulted in a response that adds a cookie, any future
//! requests will contain that cookie. Similarly, cookies removed by a response
//! won't be propagated further.
//!
//! This is typically the desired mode of operation for a `Client` as it removes
//! the burden of manually tracking cookies. Under some circumstances, however,
//! disabling this tracking may be desired. In these cases, use the
//! `untracked()` constructor to create a `Client` that _will not_ track
//! cookies.
//!
//! [`Client`]: blocking::Client
//! [`async` `Client`]: asynchronous::Client
//! [`LocalRequest`]: blocking::LocalRequest
//! [`async` `LocalRequest`]: asynchronous::LocalRequest
//!
//! [`tracked()`]: blocking::Client::tracked()
//! [`untracked()`]: blocking::Client::untracked()
//! [`async` `tracked()`]: asynchronous::Client::tracked()
//! [`async` `untracked()`]: asynchronous::Client::untracked()
//!
//! [`get()`]: blocking::Client::get()
//! [`put()`]: blocking::Client::put()
//! [`post()`]: blocking::Client::post()
//! [`async` `get()`]: asynchronous::Client::get()
//! [`async` `put()`]: asynchronous::Client::put()
//! [`async` `post()`]: asynchronous::Client::post()
//!
//! **Example**
//!
//! For a usage example, see [`Client`] or [`async` `Client`].
//!
//! # `LocalRequest`
//!
//! A [`LocalRequest`] ([`async` `LocalRequest`]) is constructed via a `Client`.
//! Once obtained, headers, cookies, including private cookies, the remote IP
//! address, and the request body can all be set via methods on the
//! `LocalRequest` structure.
//!
//! **Dispatching**
//!
//! A `LocalRequest` is dispatched by calling [`dispatch()`] ([`async`
//! `dispatch()`]). The `LocalRequest` is consumed and a [`LocalResponse`]
//! ([`async` `LocalResponse`]) is returned.
//!
//! Note that `LocalRequest` implements [`Clone`]. As such, if the same request
//! needs to be dispatched multiple times, the request can first be cloned and
//! then dispatched: `request.clone().dispatch()`.
//!
//! **Example**
//!
//! For a usage example, see [`LocalRequest`] or [`async` `LocalRequest`].
//!
//! [`LocalResponse`]: blocking::LocalResponse
//! [`async` `LocalResponse`]: asynchronous::LocalResponse
//! [`dispatch()`]: blocking::LocalRequest::dispatch()
//! [`async` `dispatch()`]: asynchronous::LocalRequest::dispatch()
//!
//! # `LocalResponse`
//!
//! The result of `dispatch()`ing a `LocalRequest` is a [`LocalResponse`]
//! ([`async` `LocalResponse`]). A `LocalResponse` can be queried for response
//! metadata, including the HTTP status, headers, and cookies, via its getter
//! methods. Additionally, the body of the response can be read into a string
//! ([`into_string()`] or [`async` `into_string()`]) or a vector
//! ([`into_bytes()`] or [`async` `into_bytes()`]).
//!
//! The response body must also be read directly using standard I/O mechanisms:
//! the `blocking` `LocalResponse` implements `Read` while the `async`
//! `LocalResponse` implements `AsyncRead`.
//!
//! For a usage example, see [`LocalResponse`] or [`async` `LocalResponse`].
//!
//! [`into_string()`]: blocking::LocalResponse::into_string()
//! [`async` `into_string()`]: asynchronous::LocalResponse::into_string()
//! [`into_bytes()`]: blocking::LocalResponse::into_bytes()
//! [`async` `into_bytes()`]: asynchronous::LocalResponse::into_bytes()

#[macro_use] mod client;
#[macro_use] mod request;
#[macro_use] mod response;

pub mod asynchronous;
pub mod blocking;
pub mod form;
