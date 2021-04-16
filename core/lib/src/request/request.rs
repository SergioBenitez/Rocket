use std::{ops::RangeFrom, sync::Arc};
use std::net::{IpAddr, SocketAddr};
use std::future::Future;
use std::fmt;
use std::str;

use yansi::Paint;
use state::{Container, Storage};
use futures::future::BoxFuture;
use atomic::{Atomic, Ordering};

// use crate::request::{FromParam, FromSegments, FromRequest, Outcome};
use crate::request::{FromParam, FromSegments, FromRequest, Outcome};
use crate::form::{self, ValueField, FromForm};

use crate::{Rocket, Route, Orbit};
use crate::http::{hyper, uri::{Origin, Segments}, uncased::UncasedStr};
use crate::http::{Method, Header, HeaderMap};
use crate::http::{ContentType, Accept, MediaType, CookieJar, Cookie};
use crate::data::Limits;

/// The type of an incoming web request.
///
/// This should be used sparingly in Rocket applications. In particular, it
/// should likely only be used when writing [`FromRequest`] implementations. It
/// contains all of the information for a given web request except for the body
/// data. This includes the HTTP method, URI, cookies, headers, and more.
pub struct Request<'r> {
    method: Atomic<Method>,
    uri: Origin<'r>,
    headers: HeaderMap<'r>,
    remote: Option<SocketAddr>,
    pub(crate) state: RequestState<'r>,
}

pub(crate) struct RequestState<'r> {
    pub rocket: &'r Rocket<Orbit>,
    pub route: Atomic<Option<&'r Route>>,
    pub cookies: CookieJar<'r>,
    pub accept: Storage<Option<Accept>>,
    pub content_type: Storage<Option<ContentType>>,
    pub cache: Arc<Container![Send + Sync]>,
}

impl Request<'_> {
    pub(crate) fn clone(&self) -> Self {
        Request {
            method: Atomic::new(self.method()),
            uri: self.uri.clone(),
            headers: self.headers.clone(),
            remote: self.remote.clone(),
            state: self.state.clone(),
        }
    }
}

impl RequestState<'_> {
    fn clone(&self) -> Self {
        RequestState {
            rocket: self.rocket,
            route: Atomic::new(self.route.load(Ordering::Acquire)),
            cookies: self.cookies.clone(),
            accept: self.accept.clone(),
            content_type: self.content_type.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<'r> Request<'r> {
    /// Create a new `Request` with the given `method` and `uri`.
    #[inline(always)]
    pub(crate) fn new<'s: 'r>(
        rocket: &'r Rocket<Orbit>,
        method: Method,
        uri: Origin<'s>
    ) -> Request<'r> {
        Request {
            uri,
            method: Atomic::new(method),
            headers: HeaderMap::new(),
            remote: None,
            state: RequestState {
                rocket,
                route: Atomic::new(None),
                cookies: CookieJar::new(rocket.config()),
                accept: Storage::new(),
                content_type: Storage::new(),
                cache: Arc::new(<Container![Send + Sync]>::new()),
            }
        }
    }

    /// Retrieve the method from `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::Method;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// # let post = |uri| c.post(uri);
    /// assert_eq!(get("/").method(), Method::Get);
    /// assert_eq!(post("/").method(), Method::Post);
    /// ```
    #[inline(always)]
    pub fn method(&self) -> Method {
        self.method.load(Ordering::Acquire)
    }

    /// Set the method of `self` to `method`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::Method;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// assert_eq!(request.method(), Method::Get);
    ///
    /// request.set_method(Method::Post);
    /// assert_eq!(request.method(), Method::Post);
    /// ```
    #[inline(always)]
    pub fn set_method(&mut self, method: Method) {
        self._set_method(method);
    }

    /// Borrow the [`Origin`] URI from `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert_eq!(get("/hello/rocketeer").uri().path(), "/hello/rocketeer");
    /// assert_eq!(get("/hello").uri().query(), None);
    /// ```
    #[inline(always)]
    pub fn uri(&self) -> &Origin<'r> {
        &self.uri
    }

    /// Set the URI in `self` to `uri`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::uri::Origin;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// let uri = Origin::parse("/hello/Sergio?type=greeting").unwrap();
    /// request.set_uri(uri);
    /// assert_eq!(request.uri().path(), "/hello/Sergio");
    /// assert_eq!(request.uri().query().unwrap(), "type=greeting");
    ///
    /// let new_uri = request.uri().map_path(|p| format!("/foo{}", p)).unwrap();
    /// request.set_uri(new_uri);
    /// assert_eq!(request.uri().path(), "/foo/hello/Sergio");
    /// assert_eq!(request.uri().query().unwrap(), "type=greeting");
    /// ```
    #[inline(always)]
    pub fn set_uri(&mut self, uri: Origin<'r>) {
        self.uri = uri;
    }

    /// Returns the raw address of the remote connection that initiated this
    /// request if the address is known. If the address is not known, `None` is
    /// returned.
    ///
    /// Because it is common for proxies to forward connections for clients, the
    /// remote address may contain information about the proxy instead of the
    /// client. For this reason, proxies typically set the "X-Real-IP" header
    /// with the client's true IP. To extract this IP from the request, use the
    /// [`real_ip()`] or [`client_ip()`] methods.
    ///
    /// [`real_ip()`]: #method.real_ip
    /// [`client_ip()`]: #method.client_ip
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::net::{SocketAddrV4, Ipv4Addr};
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// assert_eq!(request.remote(), None);
    ///
    /// let localhost = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8000).into();
    /// request.set_remote(localhost);
    /// assert_eq!(request.remote(), Some(localhost));
    /// ```
    #[inline(always)]
    pub fn remote(&self) -> Option<SocketAddr> {
        self.remote
    }

    /// Sets the remote address of `self` to `address`.
    ///
    /// # Example
    ///
    /// Set the remote address to be 127.0.0.1:8000:
    ///
    /// ```rust
    /// use std::net::{SocketAddrV4, Ipv4Addr};
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// assert_eq!(request.remote(), None);
    ///
    /// let localhost = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8000).into();
    /// request.set_remote(localhost);
    /// assert_eq!(request.remote(), Some(localhost));
    /// ```
    #[inline(always)]
    pub fn set_remote(&mut self, address: SocketAddr) {
        self.remote = Some(address);
    }

    /// Returns the IP address in the "X-Real-IP" header of the request if such
    /// a header exists and contains a valid IP address.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::net::Ipv4Addr;
    /// use rocket::http::Header;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let req = c.get("/");
    /// assert_eq!(req.real_ip(), None);
    ///
    /// let req = req.header(Header::new("X-Real-IP", "127.0.0.1"));
    /// assert_eq!(req.real_ip(), Some(Ipv4Addr::LOCALHOST.into()));
    /// ```
    pub fn real_ip(&self) -> Option<IpAddr> {
        self.headers()
            .get_one("X-Real-IP")
            .and_then(|ip| {
                ip.parse()
                    .map_err(|_| warn!("'X-Real-IP' header is malformed: {}", ip))
                    .ok()
            })
    }

    /// Attempts to return the client's IP address by first inspecting the
    /// "X-Real-IP" header and then using the remote connection's IP address.
    ///
    /// If the "X-Real-IP" header exists and contains a valid IP address, that
    /// address is returned. Otherwise, if the address of the remote connection
    /// is known, that address is returned. Otherwise, `None` is returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::http::Header;
    /// # use std::net::{SocketAddr, IpAddr, Ipv4Addr};
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// // starting without an "X-Real-IP" header or remote addresss
    /// assert!(request.client_ip().is_none());
    ///
    /// // add a remote address; this is done by Rocket automatically
    /// request.set_remote("127.0.0.1:8000".parse().unwrap());
    /// assert_eq!(request.client_ip(), Some("127.0.0.1".parse().unwrap()));
    ///
    /// // now with an X-Real-IP header
    /// request.add_header(Header::new("X-Real-IP", "8.8.8.8"));
    /// assert_eq!(request.client_ip(), Some("8.8.8.8".parse().unwrap()));
    /// ```
    #[inline]
    pub fn client_ip(&self) -> Option<IpAddr> {
        self.real_ip().or_else(|| self.remote().map(|r| r.ip()))
    }

    /// Returns a wrapped borrow to the cookies in `self`.
    ///
    /// [`CookieJar`] implements internal mutability, so this method allows you
    /// to get _and_ add/remove cookies in `self`.
    ///
    /// # Example
    ///
    /// Add a new cookie to a request's cookies:
    ///
    /// ```rust
    /// use rocket::http::Cookie;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// # let req = request.inner();
    /// req.cookies().add(Cookie::new("key", "val"));
    /// req.cookies().add(Cookie::new("ans", format!("life: {}", 38 + 4)));
    ///
    /// assert_eq!(req.cookies().get_pending("key").unwrap().value(), "val");
    /// assert_eq!(req.cookies().get_pending("ans").unwrap().value(), "life: 42");
    /// ```
    #[inline(always)]
    pub fn cookies(&self) -> &CookieJar<'r> {
        &self.state.cookies
    }

    /// Returns a [`HeaderMap`] of all of the headers in `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::{Accept, ContentType};
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert!(get("/").headers().is_empty());
    ///
    /// let req = get("/").header(Accept::HTML).header(ContentType::HTML);
    /// assert_eq!(req.headers().len(), 2);
    /// ```
    #[inline(always)]
    pub fn headers(&self) -> &HeaderMap<'r> {
        &self.headers
    }

    /// Add `header` to `self`'s headers. The type of `header` can be any type
    /// that implements the `Into<Header>` trait. This includes common types
    /// such as [`ContentType`] and [`Accept`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::ContentType;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// assert!(request.headers().is_empty());
    ///
    /// request.add_header(ContentType::HTML);
    /// assert!(request.headers().contains("Content-Type"));
    /// assert_eq!(request.headers().len(), 1);
    /// ```
    #[inline]
    pub fn add_header<'h: 'r, H: Into<Header<'h>>>(&mut self, header: H) {
        let header = header.into();
        self.bust_header_cache(header.name(), false);
        self.headers.add(header);
    }

    /// Replaces the value of the header with name `header.name` with
    /// `header.value`. If no such header exists, `header` is added as a header
    /// to `self`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::ContentType;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let mut req = c.get("/");
    /// # let request = req.inner_mut();
    ///
    /// assert!(request.headers().is_empty());
    ///
    /// request.add_header(ContentType::Any);
    /// assert_eq!(request.headers().get_one("Content-Type"), Some("*/*"));
    /// assert_eq!(request.content_type(), Some(&ContentType::Any));
    ///
    /// request.replace_header(ContentType::PNG);
    /// assert_eq!(request.headers().get_one("Content-Type"), Some("image/png"));
    /// assert_eq!(request.content_type(), Some(&ContentType::PNG));
    /// ```
    #[inline]
    pub fn replace_header<'h: 'r, H: Into<Header<'h>>>(&mut self, header: H) {
        let header = header.into();
        self.bust_header_cache(header.name(), true);
        self.headers.replace(header);
    }

    /// Returns the Content-Type header of `self`. If the header is not present,
    /// returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::ContentType;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert_eq!(get("/").content_type(), None);
    ///
    /// let req = get("/").header(ContentType::JSON);
    /// assert_eq!(req.content_type(), Some(&ContentType::JSON));
    /// ```
    #[inline]
    pub fn content_type(&self) -> Option<&ContentType> {
        self.state.content_type.get_or_set(|| {
            self.headers().get_one("Content-Type").and_then(|v| v.parse().ok())
        }).as_ref()
    }

    /// Returns the Accept header of `self`. If the header is not present,
    /// returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::Accept;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert_eq!(get("/").accept(), None);
    /// assert_eq!(get("/").header(Accept::JSON).accept(), Some(&Accept::JSON));
    /// ```
    #[inline]
    pub fn accept(&self) -> Option<&Accept> {
        self.state.accept.get_or_set(|| {
            self.headers().get_one("Accept").and_then(|v| v.parse().ok())
        }).as_ref()
    }

    /// Returns the media type "format" of the request.
    ///
    /// The "format" of a request is either the Content-Type, if the request
    /// methods indicates support for a payload, or the preferred media type in
    /// the Accept header otherwise. If the method indicates no payload and no
    /// Accept header is specified, a media type of `Any` is returned.
    ///
    /// The media type returned from this method is used to match against the
    /// `format` route attribute.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::http::{Accept, ContentType, MediaType};
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// # let post = |uri| c.post(uri);
    ///
    /// // Non-payload-bearing: format is accept header.
    /// let req = get("/").header(Accept::HTML);
    /// assert_eq!(req.format(), Some(&MediaType::HTML));
    ///
    /// let req = get("/").header(ContentType::JSON).header(Accept::HTML);
    /// assert_eq!(req.format(), Some(&MediaType::HTML));
    ///
    /// // Payload: format is content-type header.
    /// let req = post("/").header(ContentType::HTML);
    /// assert_eq!(req.format(), Some(&MediaType::HTML));
    ///
    /// let req = post("/").header(ContentType::JSON).header(Accept::HTML);
    /// assert_eq!(req.format(), Some(&MediaType::JSON));
    ///
    /// // Non-payload-bearing method and no accept header: `Any`.
    /// assert_eq!(get("/").format(), Some(&MediaType::Any));
    /// ```
    pub fn format(&self) -> Option<&MediaType> {
        static ANY: MediaType = MediaType::Any;
        if self.method().supports_payload() {
            self.content_type().map(|ct| ct.media_type())
        } else {
            // FIXME: Should we be using `accept_first` or `preferred`? Or
            // should we be checking neither and instead pass things through
            // where the client accepts the thing at all?
            self.accept()
                .map(|accept| accept.preferred().media_type())
                .or(Some(&ANY))
        }
    }

    /// Returns the [`Rocket`] instance that is handling this request.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// # type Pool = usize;
    /// // Retrieve the application config via `Rocket::config()`.
    /// let config = request.rocket().config();
    ///
    /// // Retrieve managed state via `Rocket::state()`.
    /// let state = request.rocket().state::<Pool>();
    ///
    /// // Get a list of all of the registered routes and catchers.
    /// let routes = request.rocket().routes();
    /// let catchers = request.rocket().catchers();
    /// ```
    #[inline(always)]
    pub fn rocket(&self) -> &'r Rocket<Orbit> {
        &self.state.rocket
    }

    /// Returns the configured application data limits.
    ///
    /// This is convenience function equivalent to:
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// &request.rocket().config().limits
    /// # ;
    /// ```
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::data::ToByteUnit;
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    ///
    /// // This is the default `form` limit.
    /// assert_eq!(request.limits().get("form"), Some(32.kibibytes()));
    ///
    /// // Retrieve the limit for files with extension `.pdf`; etails to 1MiB.
    /// assert_eq!(request.limits().get("file/pdf"), Some(1.mebibytes()));
    /// ```
    #[inline(always)]
    pub fn limits(&self) -> &'r Limits {
        &self.rocket().config().limits
    }

    /// Get the presently matched route, if any.
    ///
    /// This method returns `Some` any time a handler or its guards are being
    /// invoked. This method returns `None` _before_ routing has commenced; this
    /// includes during request fairing callbacks.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// let route = request.route();
    /// ```
    #[inline(always)]
    pub fn route(&self) -> Option<&'r Route> {
        self.state.route.load(Ordering::Acquire)
    }

    /// Invokes the request guard implementation for `T`, returning its outcome.
    ///
    /// # Example
    ///
    /// Assuming a `User` request guard exists, invoke it:
    ///
    /// ```rust
    /// # type User = rocket::http::Method;
    /// # rocket::async_test(async move {
    /// # let c = rocket::local::asynchronous::Client::debug_with(vec![]).await.unwrap();
    /// # let request = c.get("/");
    /// let outcome = request.guard::<User>().await;
    /// # })
    /// ```
    #[inline(always)]
    pub fn guard<'z, 'a, T>(&'a self) -> BoxFuture<'z, Outcome<T, T::Error>>
        where T: FromRequest<'a> + 'z, 'a: 'z, 'r: 'z
    {
        T::from_request(self)
    }

    /// Retrieves the cached value for type `T` from the request-local cached
    /// state of `self`. If no such value has previously been cached for this
    /// request, `f` is called to produce the value which is subsequently
    /// returned.
    ///
    /// Different values of the same type _cannot_ be cached without using a
    /// proxy, wrapper type. To avoid the need to write these manually, or for
    /// libraries wishing to store values of public types, use the
    /// [`local_cache!`] macro to generate a locally anonymous wrapper type,
    /// store, and retrieve the wrapped value from request-local cache.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let request = c.get("/");
    /// // The first store into local cache for a given type wins.
    /// let value = request.local_cache(|| "hello");
    /// assert_eq!(*request.local_cache(|| "hello"), "hello");
    ///
    /// // The following return the cached, previously stored value for the type.
    /// assert_eq!(*request.local_cache(|| "goodbye"), "hello");
    /// ```
    #[inline]
    pub fn local_cache<T, F>(&self, f: F) -> &T
        where F: FnOnce() -> T,
              T: Send + Sync + 'static
    {
        self.state.cache.try_get()
            .unwrap_or_else(|| {
                self.state.cache.set(f());
                self.state.cache.get()
            })
    }

    /// Retrieves the cached value for type `T` from the request-local cached
    /// state of `self`. If no such value has previously been cached for this
    /// request, `fut` is `await`ed to produce the value which is subsequently
    /// returned.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::Request;
    /// # type User = ();
    /// async fn current_user<'r>(request: &Request<'r>) -> User {
    ///     // validate request for a given user, load from database, etc
    /// }
    ///
    /// # rocket::async_test(async move {
    /// # let c = rocket::local::asynchronous::Client::debug_with(vec![]).await.unwrap();
    /// # let request = c.get("/");
    /// let current_user = request.local_cache_async(async {
    ///     current_user(&request).await
    /// }).await;
    /// # })
    #[inline]
    pub async fn local_cache_async<'a, T, F>(&'a self, fut: F) -> &'a T
        where F: Future<Output = T>,
              T: Send + Sync + 'static
    {
        match self.state.cache.try_get() {
            Some(s) => s,
            None => {
                self.state.cache.set(fut.await);
                self.state.cache.get()
            }
        }
    }

    /// Retrieves and parses into `T` the 0-indexed no`n`th non-empty segment
    /// from the _routed_ request, that is, the `n`th segment _after_ the mount
    /// point. If the request has not been routed, then this is simply the `n`th
    /// non-empty request URI segment.
    ///
    /// Returns `None` if `n` is greater than the number of non-empty segments.
    /// Returns `Some(Err(T::Error))` if the parameter type `T` failed to be
    /// parsed from the `n`th dynamic parameter.
    ///
    /// This method exists only to be used by manual routing. To retrieve
    /// parameters from a request, use Rocket's code generation facilities.
    ///
    /// # Example
    ///
    /// ```rust
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert_eq!(get("/a/b/c").param(0), Some(Ok("a")));
    /// assert_eq!(get("/a/b/c").param(1), Some(Ok("b")));
    /// assert_eq!(get("/a/b/c").param(2), Some(Ok("c")));
    /// assert_eq!(get("/a/b/c").param::<&str>(3), None);
    ///
    /// assert_eq!(get("/1/b/3").param(0), Some(Ok(1)));
    /// assert!(get("/1/b/3").param::<usize>(1).unwrap().is_err());
    /// assert_eq!(get("/1/b/3").param(2), Some(Ok(3)));
    ///
    /// assert_eq!(get("/").param::<&str>(0), None);
    /// ```
    #[inline]
    pub fn param<'a, T>(&'a self, n: usize) -> Option<Result<T, T::Error>>
        where T: FromParam<'a>
    {
        self.routed_segment(n).map(T::from_param)
    }

    /// Retrieves and parses into `T` all of the path segments in the request
    /// URI beginning and including the 0-indexed `n`th non-empty segment
    /// _after_ the mount point.,that is, the `n`th segment _after_ the mount
    /// point. If the request has not been routed, then this is simply the `n`th
    /// non-empty request URI segment.
    ///
    /// `T` must implement [`FromSegments`], which is used to parse the
    /// segments. If there are no non-empty segments, the `Segments` iterator
    /// will be empty.
    ///
    /// This method exists only to be used by manual routing. To retrieve
    /// segments from a request, use Rocket's code generation facilities.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::path::PathBuf;
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// assert_eq!(get("/").segments(0..), Ok(PathBuf::new()));
    /// assert_eq!(get("/").segments(2..), Ok(PathBuf::new()));
    ///
    /// // Empty segments are skipped.
    /// assert_eq!(get("///").segments(2..), Ok(PathBuf::new()));
    /// assert_eq!(get("/a/b/c").segments(0..), Ok(PathBuf::from("a/b/c")));
    /// assert_eq!(get("/a/b/c").segments(1..), Ok(PathBuf::from("b/c")));
    /// assert_eq!(get("/a/b/c").segments(2..), Ok(PathBuf::from("c")));
    /// assert_eq!(get("/a/b/c").segments(3..), Ok(PathBuf::new()));
    /// assert_eq!(get("/a/b/c").segments(4..), Ok(PathBuf::new()));
    /// ```
    #[inline]
    pub fn segments<'a, T>(&'a self, n: RangeFrom<usize>) -> Result<T, T::Error>
        where T: FromSegments<'a>
    {
        T::from_segments(self.routed_segments(n))
    }

    /// Retrieves and parses into `T` the query value with field name `name`.
    /// `T` must implement [`FromForm`], which is used to parse the query's
    /// value. Key matching is performed case-sensitively.
    ///
    /// # Warning
    ///
    /// This method exists _only_ to be used by manual routing and should
    /// _never_ be used in a regular Rocket application. It is much more
    /// expensive to use this method than to retrieve query parameters via
    /// Rocket's codegen. To retrieve query values from a request, _always_
    /// prefer to use Rocket's code generation facilities.
    ///
    /// # Error
    ///
    /// If a query segment with name `name` isn't present, returns `None`. If
    /// parsing the value fails, returns `Some(Err(_))`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::form::FromForm;
    ///
    /// #[derive(Debug, PartialEq, FromForm)]
    /// struct Dog<'r> {
    ///     name: &'r str,
    ///     age: usize
    /// }
    ///
    /// # let c = rocket::local::blocking::Client::debug_with(vec![]).unwrap();
    /// # let get = |uri| c.get(uri);
    /// let req = get("/?a=apple&z=zebra&a=aardvark");
    /// assert_eq!(req.query_value::<&str>("a").unwrap(), Ok("apple"));
    /// assert_eq!(req.query_value::<&str>("z").unwrap(), Ok("zebra"));
    /// assert_eq!(req.query_value::<&str>("b"), None);
    ///
    /// let a_seq = req.query_value::<Vec<&str>>("a");
    /// assert_eq!(a_seq.unwrap().unwrap(), ["apple", "aardvark"]);
    ///
    /// let req = get("/?dog.name=Max+Fido&dog.age=3");
    /// let dog = req.query_value::<Dog>("dog");
    /// assert_eq!(dog.unwrap().unwrap(), Dog { name: "Max Fido", age: 3 });
    /// ```
    #[inline]
    pub fn query_value<'a, T>(&'a self, name: &str) -> Option<form::Result<'a, T>>
        where T: FromForm<'a>
    {
        if self.query_fields().find(|f| f.name == name).is_none() {
            return None;
        }

        let mut ctxt = T::init(form::Options::Lenient);

        self.query_fields()
            .filter(|f| f.name == name)
            .for_each(|f| T::push_value(&mut ctxt, f.shift()));

        Some(T::finalize(ctxt))
    }
}

// All of these methods only exist for internal, including codegen, purposes.
// They _are not_ part of the stable API. Please, don't use these.
#[doc(hidden)]
impl<'r> Request<'r> {
    /// Resets the cached value (if any) for the header with name `name`.
    fn bust_header_cache(&mut self, name: &UncasedStr, replace: bool) {
        if name == "Content-Type" {
            if self.content_type().is_none() || replace {
                self.state.content_type = Storage::new();
            }
        } else if name == "Accept" {
            if self.accept().is_none() || replace {
                self.state.accept = Storage::new();
            }
        }
    }

    /// Get the `n`th path segment, 0-indexed, after the mount point for the
    /// currently matched route, as a string, if it exists. Used by codegen.
    #[inline]
    pub fn routed_segment(&self, n: usize) -> Option<&str> {
        self.routed_segments(0..).get(n)
    }

    /// Get the segments beginning at the `n`th, 0-indexed, after the mount
    /// point for the currently matched route, if they exist. Used by codegen.
    #[inline]
    pub fn routed_segments(&self, n: RangeFrom<usize>) -> Segments<'_> {
        let mount_segments = self.route()
            .map(|r| r.uri.metadata.base_segs.len())
            .unwrap_or(0);

        self.uri().path_segments().skip(mount_segments + n.start)
    }

    // Retrieves the pre-parsed query items. Used by matching and codegen.
    #[inline]
    pub fn query_fields(&self) -> impl Iterator<Item = ValueField<'_>> {
        self.uri().query_segments().map(ValueField::from)
    }

    /// Set `self`'s parameters given that the route used to reach this request
    /// was `route`. Use during routing when attempting a given route.
    #[inline(always)]
    pub(crate) fn set_route(&self, route: &'r Route) {
        self.state.route.store(Some(route), Ordering::Release)
    }

    /// Set the method of `self`, even when `self` is a shared reference. Used
    /// during routing to override methods for re-routing.
    #[inline(always)]
    pub(crate) fn _set_method(&self, method: Method) {
        self.method.store(method, Ordering::Release)
    }

    pub(crate) fn cookies_mut(&mut self) -> &mut CookieJar<'r> {
        &mut self.state.cookies
    }

    /// Convert from Hyper types into a Rocket Request.
    pub(crate) fn from_hyp(
        rocket: &'r Rocket<Orbit>,
        h_method: hyper::Method,
        h_headers: hyper::HeaderMap<hyper::HeaderValue>,
        h_uri: &'r hyper::Uri,
        h_addr: SocketAddr,
    ) -> Result<Request<'r>, Error<'r>> {
        // Get a copy of the URI (only supports path-and-query) for later use.
        let uri = match (h_uri.scheme(), h_uri.authority(), h_uri.path_and_query()) {
            (None, None, Some(path_query)) => path_query.as_str(),
            _ => return Err(Error::InvalidUri(h_uri)),
        };

        // Ensure that the method is known. TODO: Allow made-up methods?
        let method = match Method::from_hyp(&h_method) {
            Some(method) => method,
            None => return Err(Error::BadMethod(h_method))
        };

        // We need to re-parse the URI since we don't trust Hyper... :(
        let uri = Origin::parse(uri)?;

        // Construct the request object.
        let mut request = Request::new(rocket, method, uri);
        request.set_remote(h_addr);

        // Set the request cookies, if they exist.
        for header in h_headers.get_all("Cookie") {
            let raw_str = match std::str::from_utf8(header.as_bytes()) {
                Ok(string) => string,
                Err(_) => continue
            };

            for cookie_str in raw_str.split(';').map(|s| s.trim()) {
                if let Ok(cookie) = Cookie::parse_encoded(cookie_str) {
                    request.state.cookies.add_original(cookie.into_owned());
                }
            }
        }

        // Set the rest of the headers.
        // This is rather unfortunate and slow.
        for (name, value) in h_headers.iter() {
            // FIXME: This is not totally correct since values needn't be UTF8.
            let value_str = String::from_utf8_lossy(value.as_bytes()).into_owned();
            let header = Header::new(name.to_string(), value_str);
            request.add_header(header);
        }

        Ok(request)
    }
}

#[derive(Debug)]
pub(crate) enum Error<'r> {
    InvalidUri(&'r hyper::Uri),
    UriParse(crate::http::uri::Error<'r>),
    BadMethod(hyper::Method),
}

impl fmt::Display for Error<'_> {
    /// Pretty prints a Request. This is primarily used by Rocket's logging
    /// infrastructure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidUri(u) => write!(f, "invalid origin URI: {}", u),
            Error::UriParse(u) => write!(f, "URI `{}` failed to parse as origin", u),
            Error::BadMethod(m) => write!(f, "invalid or unrecognized method: {}", m),
        }
    }
}

impl<'r> From<crate::http::uri::Error<'r>> for Error<'r> {
    fn from(uri_parse: crate::http::uri::Error<'r>) -> Self {
        Error::UriParse(uri_parse)
    }
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Request")
            .field("method", &self.method)
            .field("uri", &self.uri)
            .field("headers", &self.headers())
            .field("remote", &self.remote())
            .field("cookies", &self.cookies())
            .finish()
    }
}

impl fmt::Display for Request<'_> {
    /// Pretty prints a Request. This is primarily used by Rocket's logging
    /// infrastructure.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", Paint::green(self.method()), Paint::blue(&self.uri))?;

        // Print the requests media type when the route specifies a format.
        if let Some(media_type) = self.format() {
            if !media_type.is_any() {
                write!(f, " {}", Paint::yellow(media_type))?;
            }
        }

        Ok(())
    }
}
