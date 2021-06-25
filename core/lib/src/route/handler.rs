use crate::websocket::{WebSocket, WebSocketStatus};
use crate::{Request, Data};
use crate::response::{Response, Responder};
use crate::http::Status;

/// Type alias for the return type of a [`Route`](crate::Route)'s
/// [`Handler::handle()`].
pub type Outcome<'r> = crate::outcome::Outcome<Response<'r>, Status, Data<'r>>;

/// Type alias for the return type of a _raw_ [`Route`](crate::Route)'s
/// [`Handler`].
pub type BoxFuture<'r, T = Outcome<'r>> = futures::future::BoxFuture<'r, T>;

/// Trait implemented by [`Route`](crate::Route) request handlers.
///
/// In general, you will never need to implement `Handler` manually or be
/// concerned about the `Handler` trait; Rocket's code generation handles
/// everything for you. You only need to learn about this trait if you want to
/// provide an external, library-based mechanism to handle requests where
/// request handling depends on input from the user. In other words, if you want
/// to write a plugin for Rocket that looks mostly like a static route but need
/// user provided state to make a request handling decision, you should consider
/// implementing a custom `Handler`.
///
/// ## Async Trait
///
/// This is an _async_ trait. Implementations must be decorated
/// [`#[rocket::async_trait]`](crate::async_trait).
///
/// # Example
///
/// Say you'd like to write a handler that changes its functionality based on an
/// enum value that the user provides:
///
/// ```rust
/// #[derive(Copy, Clone)]
/// enum Kind {
///     Simple,
///     Intermediate,
///     Complex,
/// }
/// ```
///
/// Such a handler might be written and used as follows:
///
/// ```rust,no_run
/// # #[derive(Copy, Clone)] enum Kind { Simple, Intermediate, Complex, }
/// use rocket::{Request, Data};
/// use rocket::route::{Handler, Route, Outcome};
/// use rocket::http::Method;
///
/// #[derive(Clone)]
/// struct CustomHandler(Kind);
///
/// #[rocket::async_trait]
/// impl Handler for CustomHandler {
///     async fn handle<'r>(&self, req: &'r Request<'_>, data: Data<'r>) -> Outcome<'r> {
///         match self.0 {
///             Kind::Simple => Outcome::from(req, "simple"),
///             Kind::Intermediate => Outcome::from(req, "intermediate"),
///             Kind::Complex => Outcome::from(req, "complex"),
///         }
///     }
/// }
///
/// impl Into<Vec<Route>> for CustomHandler {
///     fn into(self) -> Vec<Route> {
///         vec![Route::new(Method::Get, "/", self)]
///     }
/// }
///
/// #[rocket::launch]
/// fn rocket() -> _ {
///     rocket::build().mount("/", CustomHandler(Kind::Simple))
/// }
/// ```
///
/// Note the following:
///
///   1. `CustomHandler` implements `Clone`. This is required so that
///      `CustomHandler` implements `Cloneable` automatically. The `Cloneable`
///      trait serves no other purpose but to ensure that every `Handler` can be
///      cloned, allowing `Route`s to be cloned.
///   2. `CustomHandler` implements `Into<Vec<Route>>`, allowing an instance to
///      be used directly as the second parameter to `rocket.mount()`.
///   3. Unlike static-function-based handlers, this custom handler can make use
///      of any internal state.
///
/// # Alternatives
///
/// The previous example could have been implemented using a combination of
/// managed state and a static route, as follows:
///
/// ```rust,no_run
/// # #[macro_use] extern crate rocket;
/// #
/// # #[derive(Copy, Clone)]
/// # enum Kind {
/// #     Simple,
/// #     Intermediate,
/// #     Complex,
/// # }
/// #
/// use rocket::State;
///
/// #[get("/")]
/// fn custom_handler(state: &State<Kind>) -> &'static str {
///     match state.inner() {
///         Kind::Simple => "simple",
///         Kind::Intermediate => "intermediate",
///         Kind::Complex => "complex",
///     }
/// }
///
/// #[launch]
/// fn rocket() -> _ {
///     rocket::build()
///         .mount("/", routes![custom_handler])
///         .manage(Kind::Simple)
/// }
/// ```
///
/// Pros:
///
///   * The handler is easier to implement since Rocket's code generation
///     ensures type-safety at all levels.
///
/// Cons:
///
///   * Only one `Kind` can be stored in managed state. As such, only one
///     variant of the custom handler can be used.
///   * The user must remember to manually call `rocket.manage(state)`.
///
/// Use this alternative when a single configuration is desired and your custom
/// handler is private to your application. For all other cases, a custom
/// `Handler` implementation is preferred.
#[crate::async_trait]
pub trait Handler: Cloneable + Send + Sync + 'static {
    /// Called by Rocket when a `Request` with its associated `Data` should be
    /// handled by this handler.
    ///
    /// The variant of `Outcome` returned by the returned `Future` determines
    /// what Rocket does next. If the return value is a `Success(Response)`, the
    /// wrapped `Response` is used to respond to the client. If the return value
    /// is a `Failure(Status)`, the error catcher for `Status` is invoked to
    /// generate a response. Otherwise, if the return value is `Forward(Data)`,
    /// the next matching route is attempted. If there are no other matching
    /// routes, the `404` error catcher is invoked.
    async fn handle<'r>(&self, request: &'r Request<'_>, data: Data<'r>) -> Outcome<'r>;
}

// We write this manually to avoid double-boxing.
impl<F: Clone + Sync + Send + 'static> Handler for F
    where for<'x> F: Fn(&'x Request<'_>, Data<'x>) -> BoxFuture<'x>,
{
    #[inline(always)]
    fn handle<'r, 'life0, 'life1, 'async_trait>(
        &'life0 self,
        req: &'r Request<'life1>,
        data: Data<'r>,
    ) -> BoxFuture<'r>
        where 'r: 'async_trait,
              'life0: 'async_trait,
              'life1: 'async_trait,
              Self: 'async_trait,
    {
        self(req, data)
    }
}

// INTERNAL: A handler to use when one is needed temporarily.
#[doc(hidden)]
pub fn dummy_handler<'r>(r: &'r Request<'_>, _: Data<'r>) -> BoxFuture<'r> {
    Outcome::from(r, ()).pin()
}

/// Type alias for the return type of a [`Route`](crate::Route)'s
/// [`Handler::handle()`].
pub type WsOutcome<'r> = crate::outcome::Outcome<Response<'r>, WebSocketStatus<'static>, Data<'r>>;

/// Type alias for the return type of a _raw_ [`Route`](crate::Route)'s
/// [`Handler`].
pub type BoxWsFuture<'r, T = WsOutcome<'r>> = futures::future::BoxFuture<'r, T>;

/// See [`Handler`]
#[crate::async_trait]
pub trait WebSocketHandler: CloneableWS + Send + Sync + 'static {
    /// Called by Rocket when a `Request` with its associated `Data` should be
    /// handled by this handler.
    ///
    /// The variant of `Outcome` returned by the returned `Future` determines
    /// what Rocket does next. If the return value is a `Success(Response)`, the
    /// wrapped `Response` is used to respond to the client. If the return value
    /// is a `Failure(Status)`, the error catcher for `Status` is invoked to
    /// generate a response. Otherwise, if the return value is `Forward(Data)`,
    /// the next matching route is attempted. If there are no other matching
    /// routes, the `404` error catcher is invoked.
    async fn handle<'r>(&self, request: &'r WebSocket<'_>, data: Data<'r>) -> WsOutcome<'r>;
}

// We write this manually to avoid double-boxing.
impl<F: Clone + Sync + Send + 'static> WebSocketHandler for F
    where for<'x> F: Fn(&'x WebSocket<'_>, Data<'x>) -> BoxWsFuture<'x>,
{
    #[inline(always)]
    fn handle<'r, 'life0, 'life1, 'async_trait>(
        &'life0 self,
        req: &'r WebSocket<'life1>,
        data: Data<'r>,
    ) -> BoxWsFuture<'r>
        where 'r: 'async_trait,
              'life0: 'async_trait,
              'life1: 'async_trait,
              Self: 'async_trait,
    {
        self(req, data)
    }
}

mod private {
    pub trait Sealed {}
    impl<T: super::Handler + Clone> Sealed for T {}
    pub trait SealedWS {}
    impl<T: super::WebSocketHandler + Clone> SealedWS for T {}
}

/// Helper trait to make a [`Route`](crate::Route)'s `Box<dyn Handler>`
/// `Clone`.
///
/// This trait cannot be implemented directly. Instead, implement `Clone` and
/// [`Handler`]; all types that implement `Clone` and `Handler` automatically
/// implement `Cloneable`.
pub trait Cloneable: private::Sealed {
    #[doc(hidden)]
    fn clone_handler(&self) -> Box<dyn Handler>;
}

impl<T: Handler + Clone> Cloneable for T {
    fn clone_handler(&self) -> Box<dyn Handler> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn Handler> {
    fn clone(&self) -> Box<dyn Handler> {
        self.clone_handler()
    }
}

/// Helper trait to make a [`Route`](crate::Route)'s `Box<dyn WebSocketHandler>`
/// `Clone`.
///
/// This trait cannot be implemented directly. Instead, implement `Clone` and
/// [`WebSocketHandler`]; all types that implement `Clone` and `WebSocketHandler`
/// automatically implement `CloneableWs`.
pub trait CloneableWS: private::SealedWS {
    #[doc(hidden)]
    fn clone_handler(&self) -> Box<dyn WebSocketHandler>;
}

impl<T: WebSocketHandler + Clone> CloneableWS for T {
    fn clone_handler(&self) -> Box<dyn WebSocketHandler> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn WebSocketHandler> {
    fn clone(&self) -> Box<dyn WebSocketHandler> {
        self.clone_handler()
    }
}

// FIXME!
impl<'r, 'o: 'r> Outcome<'o> {
    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Failure` is
    /// returned with the status code.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from(req, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn from<R: Responder<'r, 'o>>(req: &'r Request<'_>, responder: R) -> Outcome<'r> {
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(status) => Outcome::Failure(status)
        }
    }

    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Failure` is
    /// returned with the status code.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from(req, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn try_from<R, E>(req: &'r Request<'_>, result: Result<R, E>) -> Outcome<'r>
        where R: Responder<'r, 'o>, E: std::fmt::Debug
    {
        let responder = result.map_err(crate::response::Debug);
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(status) => Outcome::Failure(status)
        }
    }

    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Forward` is
    /// returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, data: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from_or_forward(req, data, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn from_or_forward<R>(req: &'r Request<'_>, data: Data<'r>, responder: R) -> Outcome<'r>
        where R: Responder<'r, 'o>
    {
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(_) => Outcome::Forward(data)
        }
    }

    /// Return an `Outcome` of `Failure` with the status code `code`. This is
    /// equivalent to `Outcome::Failure(code)`.
    ///
    /// This method exists to be used during manual routing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    /// use rocket::http::Status;
    ///
    /// fn bad_req_route<'r>(_: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::failure(Status::BadRequest)
    /// }
    /// ```
    #[inline(always)]
    pub fn failure(code: Status) -> Outcome<'r> {
        Outcome::Failure(code)
    }

    /// Return an `Outcome` of `Forward` with the data `data`. This is
    /// equivalent to `Outcome::Forward(data)`.
    ///
    /// This method exists to be used during manual routing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn always_forward<'r>(_: &'r Request, data: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::forward(data)
    /// }
    /// ```
    #[inline(always)]
    pub fn forward(data: Data<'r>) -> Outcome<'r> {
        Outcome::Forward(data)
    }
}

// FIXME!
impl<'r, 'o: 'r> WsOutcome<'o> {
    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Failure` is
    /// returned with the status code.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from(req, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn from<R: Responder<'r, 'o>>(req: &'r Request<'_>, responder: R) -> Outcome<'r> {
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(status) => Outcome::Failure(status)
        }
    }

    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Failure` is
    /// returned with the status code.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from(req, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn try_from<R, E>(req: &'r Request<'_>, result: Result<R, E>) -> Outcome<'r>
        where R: Responder<'r, 'o>, E: std::fmt::Debug
    {
        let responder = result.map_err(crate::response::Debug);
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(status) => Outcome::Failure(status)
        }
    }

    /// Return the `Outcome` of response to `req` from `responder`.
    ///
    /// If the responder returns `Ok`, an outcome of `Success` is returned with
    /// the response. If the responder returns `Err`, an outcome of `Forward` is
    /// returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn str_responder<'r>(req: &'r Request, data: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::from_or_forward(req, data, "Hello, world!")
    /// }
    /// ```
    #[inline]
    pub fn from_or_forward<R>(req: &'r Request<'_>, data: Data<'r>, responder: R) -> Outcome<'r>
        where R: Responder<'r, 'o>
    {
        match responder.respond_to(req) {
            Ok(response) => Outcome::Success(response),
            Err(_) => Outcome::Forward(data)
        }
    }

    /// Return an `Outcome` of `Failure` with the status code `code`. This is
    /// equivalent to `Outcome::Failure(code)`.
    ///
    /// This method exists to be used during manual routing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    /// use rocket::http::Status;
    ///
    /// fn bad_req_route<'r>(_: &'r Request, _: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::failure(Status::BadRequest)
    /// }
    /// ```
    #[inline(always)]
    pub fn failure(code: Status) -> Outcome<'r> {
        Outcome::Failure(code)
    }

    /// Return an `Outcome` of `Forward` with the data `data`. This is
    /// equivalent to `Outcome::Forward(data)`.
    ///
    /// This method exists to be used during manual routing.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::{Request, Data, route};
    ///
    /// fn always_forward<'r>(_: &'r Request, data: Data<'r>) -> route::Outcome<'r> {
    ///     route::Outcome::forward(data)
    /// }
    /// ```
    #[inline(always)]
    pub fn forward(data: Data<'r>) -> Outcome<'r> {
        Outcome::Forward(data)
    }
}
