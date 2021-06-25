use std::fmt;
use std::convert::From;
use std::borrow::Cow;

use yansi::Paint;

use crate::http::{uri, Method, MediaType};
use crate::route::{Handler, RouteUri, BoxFuture};
use crate::sentinel::Sentry;
use crate::websocket::WebSocket;

use super::BoxWsFuture;
use super::WebSocketHandler;

/// A request handling route.
///
/// A route consists of exactly the information in its fields. While a `Route`
/// can be instantiated directly, doing so should be a rare or nonexistent
/// event. Instead, a Rocket application should use Rocket's
/// [`#[route]`](macro@crate::route) series of attributes to generate a `Route`.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use std::path::PathBuf;
/// #[get("/route/<path..>?query", rank = 2, format = "json")]
/// fn route_name(path: PathBuf) { /* handler procedure */ }
///
/// use rocket::http::{Method, MediaType};
///
/// let route = routes![route_name].remove(0);
/// assert_eq!(route.name.unwrap(), "route_name");
/// assert_eq!(route.method, Method::Get);
/// assert_eq!(route.uri, "/route/<path..>?query");
/// assert_eq!(route.rank, 2);
/// assert_eq!(route.format.unwrap(), MediaType::JSON);
/// ```
///
/// Note that the `rank` and `format` attribute parameters are optional. See
/// [`#[route]`](macro@crate::route) for details on macro usage. Note also that
/// a route's mounted _base_ becomes part of its URI; see [`RouteUri`] for
/// details.
///
/// # Routing
///
/// A request _matches_ a route _iff_:
///
///   * The route's method matches that of the incoming request.
///   * The route's format (if any) matches that of the incoming request.
///     - If route specifies a format, it only matches requests for that format.
///     - If route doesn't specify a format, it matches requests for any format.
///     - A route's `format` matches against the `Accept` header in the request
///       when the route's method [`supports_payload()`] and `Content-Type`
///       header otherwise.
///     - Non-specific `Accept` header components (`*`) match anything.
///   * All static components in the route's path match the corresponding
///     components in the same position in the incoming request.
///   * All static components in the route's query string are also in the
///     request query string, though in any position. If there is no query
///     in the route, requests with and without queries match.
///
/// Rocket routes requests to matching routes.
///
/// [`supports_payload()`]: Method::supports_payload()
///
/// # Collisions
///
/// Two routes are said to _collide_ if there exists a request that matches both
/// routes. Colliding routes present a routing ambiguity and are thus disallowed
/// by Rocket. Because routes can be constructed dynamically, collision checking
/// is done at [`ignite`](crate::Rocket::ignite()) time, after it becomes
/// statically impossible to add any more routes to an instance of `Rocket`.
///
/// Note that because query parsing is always lenient -- extra and missing query
/// parameters are allowed -- queries do not directly impact whether two routes
/// collide.
///
/// ## Resolving Collisions
///
/// Collisions are resolved through _ranking_. Routes with lower ranks have
/// higher precedence during routing than routes with higher ranks. Thus, routes
/// are attempted in ascending rank order. If a higher precedence route returns
/// an `Outcome` of `Forward`, the next highest precedence route is attempted,
/// and so on, until a route returns `Success` or `Failure`, or there are no
/// more routes to try. When all routes have been attempted, Rocket issues a
/// `404` error, handled by the appropriate [`Catcher`](crate::Catcher).
///
/// ## Default Ranking
///
/// Most collisions are automatically resolved by Rocket's _default rank_. The
/// default rank prefers static components over dynamic components in both paths
/// and queries: the _more_ static a route's path and query are, the lower its
/// rank and thus the higher its precedence.
///
/// There are three "colors" to paths and queries:
///   1. `static` - all components are static
///   2. `partial` - at least one, but not all, components are dynamic
///   3. `wild` - all components are dynamic
///
/// Static paths carry more weight than static queries. The same is true for
/// partial and wild paths. This results in the following default ranking
/// table:
///
/// | path    | query   | rank |
/// |---------|---------|------|
/// | static  | static  | -12  |
/// | static  | partial | -11  |
/// | static  | wild    | -10  |
/// | static  | none    | -9   |
/// | partial | static  | -8   |
/// | partial | partial | -7   |
/// | partial | wild    | -6   |
/// | partial | none    | -5   |
/// | wild    | static  | -4   |
/// | wild    | partial | -3   |
/// | wild    | wild    | -2   |
/// | wild    | none    | -1   |
///
/// Recall that _lower_ ranks have _higher_ precedence.
///
/// ### Example
///
/// ```rust
/// use rocket::Route;
/// use rocket::http::Method;
///
/// macro_rules! assert_rank {
///     ($($uri:expr => $rank:expr,)*) => {$(
///         let route = Route::new(Method::Get, $uri, rocket::route::dummy_handler);
///         assert_eq!(route.rank, $rank);
///     )*}
/// }
///
/// assert_rank! {
///     "/?foo" => -12,                 // static path, static query
///     "/foo/bar?a=b&bob" => -12,      // static path, static query
///     "/?a=b&bob" => -12,             // static path, static query
///
///     "/?a&<zoo..>" => -11,           // static path, partial query
///     "/foo?a&<zoo..>" => -11,        // static path, partial query
///     "/?a&<zoo>" => -11,             // static path, partial query
///
///     "/?<zoo..>" => -10,             // static path, wild query
///     "/foo?<zoo..>" => -10,          // static path, wild query
///     "/foo?<a>&<b>" => -10,          // static path, wild query
///
///     "/" => -9,                      // static path, no query
///     "/foo/bar" => -9,               // static path, no query
///
///     "/a/<b>?foo" => -8,             // partial path, static query
///     "/a/<b..>?foo" => -8,           // partial path, static query
///     "/<a>/b?foo" => -8,             // partial path, static query
///
///     "/a/<b>?<b>&c" => -7,           // partial path, partial query
///     "/a/<b..>?a&<c..>" => -7,       // partial path, partial query
///
///     "/a/<b>?<c..>" => -6,           // partial path, wild query
///     "/a/<b..>?<c>&<d>" => -6,       // partial path, wild query
///     "/a/<b..>?<c>" => -6,           // partial path, wild query
///
///     "/a/<b>" => -5,                 // partial path, no query
///     "/<a>/b" => -5,                 // partial path, no query
///     "/a/<b..>" => -5,               // partial path, no query
///
///     "/<b>/<c>?foo&bar" => -4,       // wild path, static query
///     "/<a>/<b..>?foo" => -4,         // wild path, static query
///     "/<b..>?cat" => -4,             // wild path, static query
///
///     "/<b>/<c>?<foo>&bar" => -3,     // wild path, partial query
///     "/<a>/<b..>?a&<b..>" => -3,     // wild path, partial query
///     "/<b..>?cat&<dog>" => -3,       // wild path, partial query
///
///     "/<b>/<c>?<foo>" => -2,         // wild path, wild query
///     "/<a>/<b..>?<b..>" => -2,       // wild path, wild query
///     "/<b..>?<c>&<dog>" => -2,       // wild path, wild query
///
///     "/<b>/<c>" => -1,               // wild path, no query
///     "/<a>/<b..>" => -1,             // wild path, no query
///     "/<b..>" => -1,                 // wild path, no query
/// }
/// ```
#[derive(Clone)]
pub struct Route {
    /// The name of this route, if one was given.
    pub name: Option<Cow<'static, str>>,
    /// The method this route matches against.
    pub method: Method,
    /// The function that should be called when the route matches.
    pub handler: Box<dyn Handler>,
    /// The function that should be called when a websocket event matches
    pub websocket_handler: WebSocketEvent<Box<dyn WebSocketHandler>>,
    /// The route URI.
    pub uri: RouteUri<'static>,
    /// The rank of this route. Lower ranks have higher priorities.
    pub rank: isize,
    /// The media type this route matches against, if any.
    pub format: Option<MediaType>,
    /// The discovered sentinels.
    pub(crate) sentinels: Vec<Sentry>,
}

impl Route {
    /// Creates a new route with the given method, path, and handler with a base
    /// of `/` and a computed [default rank](#default-ranking).
    ///
    /// # Panics
    ///
    /// Panics if `path` is not a valid Rocket route URI.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Route;
    /// use rocket::http::Method;
    /// # use rocket::route::dummy_handler as handler;
    ///
    /// // this is a rank 1 route matching requests to `GET /`
    /// let index = Route::new(Method::Get, "/", handler);
    /// assert_eq!(index.rank, -9);
    /// assert_eq!(index.method, Method::Get);
    /// assert_eq!(index.uri, "/");
    /// ```
    pub fn new<H: Handler>(method: Method, uri: &str, handler: H) -> Route {
        Route::ranked(None, method, uri, handler)
    }

    /// Creates a new route with the given rank, method, path, and handler with
    /// a base of `/`. If `rank` is `None`, the computed [default
    /// rank](#default-ranking) is used.
    ///
    /// # Panics
    ///
    /// Panics if `path` is not a valid Rocket route URI.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Route;
    /// use rocket::http::Method;
    /// # use rocket::route::dummy_handler as handler;
    ///
    /// let foo = Route::ranked(1, Method::Post, "/foo?bar", handler);
    /// assert_eq!(foo.rank, 1);
    /// assert_eq!(foo.method, Method::Post);
    /// assert_eq!(foo.uri, "/foo?bar");
    ///
    /// let foo = Route::ranked(None, Method::Post, "/foo?bar", handler);
    /// assert_eq!(foo.rank, -12);
    /// assert_eq!(foo.method, Method::Post);
    /// assert_eq!(foo.uri, "/foo?bar");
    /// ```
    pub fn ranked<H, R>(rank: R, method: Method, uri: &str, handler: H) -> Route
        where H: Handler + 'static, R: Into<Option<isize>>,
    {
        let uri = RouteUri::new("/", uri);
        let rank = rank.into().unwrap_or_else(|| uri.default_rank());
        Route {
            name: None,
            format: None,
            sentinels: Vec::new(),
            handler: Box::new(handler),
            websocket_handler: WebSocketEvent::None,
            rank, uri, method,
        }
    }

    /// Maps the `base` of this route using `mapper`, returning a new `Route`
    /// with the returned base.
    ///
    /// `mapper` is called with the current base. The returned `String` is used
    /// as the new base if it is a valid URI. If the returned base URI contains
    /// a query, it is ignored. Returns an error if the base produced by
    /// `mapper` is not a valid origin URI.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Route;
    /// use rocket::http::{Method, uri::Origin};
    /// # use rocket::route::dummy_handler as handler;
    ///
    /// let index = Route::new(Method::Get, "/foo/bar", handler);
    /// assert_eq!(index.uri.base(), "/");
    /// assert_eq!(index.uri.unmounted_origin.path(), "/foo/bar");
    /// assert_eq!(index.uri.path(), "/foo/bar");
    ///
    /// let index = index.map_base(|base| format!("{}{}", "/boo", base)).unwrap();
    /// assert_eq!(index.uri.base(), "/boo");
    /// assert_eq!(index.uri.unmounted_origin.path(), "/foo/bar");
    /// assert_eq!(index.uri.path(), "/boo/foo/bar");
    /// ```
    pub fn map_base<'a, F>(mut self, mapper: F) -> Result<Self, uri::Error<'static>>
        where F: FnOnce(uri::Origin<'a>) -> String
    {
        let base = mapper(self.uri.base);
        self.uri = RouteUri::try_new(&base, &self.uri.unmounted_origin.to_string())?;
        Ok(self)
    }

    /// Gets the HTTP rank of this route. This is the same as `self.rank`, unless this is
    /// a WebSocket event handler. In that case, this is `isize::max_value()`
    #[inline]
    pub fn http_rank(&self) -> isize {
        if self.websocket_handler.is_none() {
            self.rank
        } else {
            isize::max_value()
        }
    }

    /// Gets the WebSocket rank of this route. This is the same as `self.rank`, unless this
    /// is an HTTP route. In that case, this is `isize::max_value()`
    #[inline]
    pub fn websocket_rank(&self) -> isize {
        if self.websocket_handler.is_some() {
            self.rank
        } else {
            isize::max_value()
        }
    }
}

impl fmt::Display for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref n) = self.name {
            write!(f, "{}{}{} ", Paint::cyan("("), Paint::white(n), Paint::cyan(")"))?;
        }

        if f.alternate() {
            write!(f, "{} ", Paint::green(&self.websocket_handler))?;
        } else {
            write!(f, "{} ", Paint::green(&self.method))?;
        }

        if self.uri.base() != "/" {
            write!(f, "{}", Paint::blue(self.uri.base()).underline())?;
        }

        write!(f, "{}", Paint::blue(&self.uri.unmounted_origin))?;

        if self.rank > 1 {
            write!(f, " [{}]", Paint::default(&self.rank).bold())?;
        }

        if let Some(ref format) = self.format {
            write!(f, " {}", Paint::yellow(format))?;
        }

        Ok(())
    }
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("name", &self.name)
            .field("method", &self.method)
            .field("uri", &self.uri)
            .field("rank", &self.rank)
            .field("format", &self.format)
            .finish()
    }
}

/// Information generated by the `route` attribute during codegen.
#[doc(hidden)]
pub struct StaticInfo {
    /// The route's name, i.e, the name of the function.
    pub name: &'static str,
    /// The route's method.
    pub method: Method,
    /// The route's URi, without the base mount point.
    pub uri: &'static str,
    /// The route's format, if any.
    pub format: Option<MediaType>,
    /// The route's handler, i.e, the annotated function.
    pub handler: for<'r> fn(&'r crate::Request<'_>, crate::Data<'r>) -> BoxFuture<'r>,
    /// The route's websocket handler, i.e, the annotated function.
    pub websocket_handler:
        WebSocketEvent<for<'r> fn(&'r WebSocket<'_>, crate::Data<'r>) -> BoxWsFuture<'r>>,
    /// The route's rank, if any.
    pub rank: Option<isize>,
    /// Route-derived sentinels, if any.
    /// This isn't `&'static [SentryInfo]` because `type_name()` isn't `const`.
    pub sentinels: Vec<Sentry>,
}

#[doc(hidden)]
impl From<StaticInfo> for Route {
    fn from(info: StaticInfo) -> Route {
        // This should never panic since `info.path` is statically checked.
        let uri = RouteUri::new("/", info.uri);

        Route {
            name: Some(info.name.into()),
            method: info.method,
            handler: Box::new(info.handler),
            websocket_handler: info.websocket_handler.map(|h| Box::new(h) as Box<_>),
            rank: info.rank.unwrap_or_else(|| uri.default_rank()),
            format: info.format,
            sentinels: info.sentinels.into_iter().collect(),
            uri,
        }
    }
}

/// Type to represent a Websocket Event.
#[derive(Clone, Debug)]
pub enum WebSocketEvent<T> {
    /// A Join event, triggered when a client joins
    Join(T),
    /// A Message event, triggered for every message the client sends
    Message(T),
    /// A Leave event, triggered when a client disconnects
    Leave(T),
    /// None, for routes which don't handle Websocket Events
    None,
}

impl<T> WebSocketEvent<T> {
    /// Maps `Self<T>` to `Self<E>`, using the provided function, while preserving the event type.
    pub(crate) fn map<E>(self, f: impl Fn(T) -> E) -> WebSocketEvent<E> {
        match self {
            Self::Join(t) => WebSocketEvent::Join(f(t)),
            Self::Message(t) => WebSocketEvent::Message(f(t)),
            Self::Leave(t) => WebSocketEvent::Leave(f(t)),
            Self::None => WebSocketEvent::None,
        }
    }

    /// Returns true if this event is None
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Returns true if this event is not None
    pub fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Checks if self and other collide. This more or less just checks if they are the same
    /// variant.
    pub fn collides(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Join(_), Self::Join(_)) => true,
            (Self::Message(_), Self::Message(_)) => true,
            (Self::Leave(_), Self::Leave(_)) => true,
            (Self::None, Self::None) => true,
            _ => false,
        }
    }

    /// Unwraps a reference to the inner T.
    ///
    /// # Panics
    ///
    /// Panics if `self` is `WebSocketEvent::None`, since it has no inner T.
    pub(crate) fn unwrap_ref(&self) -> &T {
        match self {
            Self::None => panic!("WebSocketEvent::None is not a valid websocket event"),
            Self::Join(t) | Self::Message(t) | Self::Leave(t) => t,
        }
    }
}

impl<T> fmt::Display for WebSocketEvent<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "NONE"),
            Self::Join(_) => write!(f, "JOIN"),
            Self::Message(_) => write!(f, "MESSAGE"),
            Self::Leave(_) => write!(f, "LEAVE"),
        }
    }
}
