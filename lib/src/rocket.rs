use std::collections::HashMap;
use std::str::from_utf8_unchecked;
use std::cmp::min;
use std::net::SocketAddr;
use std::io::{self, Write};

use term_painter::Color::*;
use term_painter::ToStyle;

use state::Container;

use {logger, handler};
use ext::ReadExt;
use config::{self, Config};
use request::{Request, FormItems};
use data::Data;
use response::{Body, Response};
use router::{Router, Route};
use catcher::{self, Catcher};
use outcome::Outcome;
use error::{Error, LaunchError};

use http::{Method, Status, Header, Session};
use http::hyper::{self, header};
use http::uri::URI;

/// The main `Rocket` type: used to mount routes and catchers and launch the
/// application.
pub struct Rocket {
    config: &'static Config,
    router: Router,
    default_catchers: HashMap<u16, Catcher>,
    catchers: HashMap<u16, Catcher>,
    state: Container
}

#[doc(hidden)]
impl hyper::Handler for Rocket {
    // This function tries to hide all of the Hyper-ness from Rocket. It
    // essentially converts Hyper types into Rocket types, then calls the
    // `dispatch` function, which knows nothing about Hyper. Because responding
    // depends on the `HyperResponse` type, this function does the actual
    // response processing.
    fn handle<'h, 'k>(&self,
                      hyp_req: hyper::Request<'h, 'k>,
                      res: hyper::FreshResponse<'h>) {
        // Get all of the information from Hyper.
        let (h_addr, h_method, h_headers, h_uri, _, h_body) = hyp_req.deconstruct();

        // Convert the Hyper request into a Rocket request.
        let mut req = match Request::from_hyp(h_method, h_headers, h_uri, h_addr) {
            Ok(req) => req,
            Err(e) => {
                error!("Bad incoming request: {}", e);
                let dummy = Request::new(Method::Get, URI::new("<unknown>"));
                let r = self.handle_error(Status::InternalServerError, &dummy);
                return self.issue_response(r, res);
            }
        };

        // Retrieve the data from the hyper body.
        let data = match Data::from_hyp(h_body) {
            Ok(data) => data,
            Err(reason) => {
                error_!("Bad data in request: {}", reason);
                let r = self.handle_error(Status::InternalServerError, &req);
                return self.issue_response(r, res);
            }
        };

        // Dispatch the request to get a response, then write that response out.
        let response = self.dispatch(&mut req, data);
        self.issue_response(response, res)
    }
}

impl Rocket {
    #[inline]
    fn issue_response(&self, mut response: Response, hyp_res: hyper::FreshResponse) {
        // Add the 'rocket' server header, and write out the response.
        // TODO: If removing Hyper, write out `Date` header too.
        response.set_header(Header::new("Server", "Rocket"));

        match self.write_response(response, hyp_res) {
            Ok(_) => info_!("{}", Green.paint("Response succeeded.")),
            Err(e) => error_!("Failed to write response: {:?}.", e)
        }
    }

    fn write_response(&self, mut response: Response,
                      mut hyp_res: hyper::FreshResponse) -> io::Result<()>
    {
        *hyp_res.status_mut() = hyper::StatusCode::from_u16(response.status().code);

        for header in response.headers() {
            // FIXME: Using hyper here requires two allocations.
            let name = header.name.into_string();
            let value = Vec::from(header.value.as_bytes());
            hyp_res.headers_mut().append_raw(name, value);
        }

        if response.body().is_none() {
            hyp_res.headers_mut().set(header::ContentLength(0));
            return hyp_res.start()?.end();
        }

        match response.body() {
            None => {
                hyp_res.headers_mut().set(header::ContentLength(0));
                hyp_res.start()?.end()
            }
            Some(Body::Sized(mut body, size)) => {
                hyp_res.headers_mut().set(header::ContentLength(size));
                let mut stream = hyp_res.start()?;
                io::copy(body, &mut stream)?;
                stream.end()
            }
            Some(Body::Chunked(mut body, chunk_size)) => {
                // This _might_ happen on a 32-bit machine!
                if chunk_size > (usize::max_value() as u64) {
                    let msg = "chunk size exceeds limits of usize type";
                    return Err(io::Error::new(io::ErrorKind::Other, msg));
                }

                // The buffer stores the current chunk being written out.
                let mut buffer = vec![0; chunk_size as usize];
                let mut stream = hyp_res.start()?;
                loop {
                    match body.read_max(&mut buffer)? {
                        0 => break,
                        n => stream.write_all(&buffer[..n])?,
                    }
                }

                stream.end()
            }
        }
    }

    /// Preprocess the request for Rocket things. Currently, this means:
    ///
    ///   * Rewriting the method in the request if _method form field exists.
    ///   * Rewriting the remote IP if the 'X-Real-IP' header is set.
    ///
    /// Keep this in-sync with derive_form when preprocessing form fields.
    fn preprocess_request(&self, req: &mut Request, data: &Data) {
        // Rewrite the remote IP address. The request must already have an
        // address associated with it to do this since we need to know the port.
        if let Some(current) = req.remote() {
            let ip = req.headers()
                .get_one("X-Real-IP")
                .and_then(|ip_str| ip_str.parse().map_err(|_| {
                    warn_!("The 'X-Real-IP' header is malformed: {}", ip_str)
                }).ok());

            if let Some(ip) = ip {
                req.set_remote(SocketAddr::new(ip, current.port()));
            }
        }

        // Check if this is a form and if the form contains the special _method
        // field which we use to reinterpret the request's method.
        let data_len = data.peek().len();
        let (min_len, max_len) = ("_method=get".len(), "_method=delete".len());
        let is_form = req.content_type().map_or(false, |ct| ct.is_form());
        if is_form && req.method() == Method::Post && data_len >= min_len {
            let form = unsafe {
                from_utf8_unchecked(&data.peek()[..min(data_len, max_len)])
            };

            if let Some((key, value)) = FormItems::from(form).next() {
                if key == "_method" {
                    if let Ok(method) = value.parse() {
                        req.set_method(method);
                    }
                }
            }
        }
    }

    #[inline]
    pub(crate) fn dispatch<'s, 'r>(&'s self, request: &'r mut Request<'s>, data: Data)
            -> Response<'r> {
        info!("{}:", request);

        // Inform the request about all of the precomputed state.
        request.set_preset_state(&self.config.session_key(), &self.state);

        // Do a bit of preprocessing before routing.
        self.preprocess_request(request, &data);

        // Route the request to get a response.
        match self.route(request, data) {
            Outcome::Success(mut response) => {
                // A user's route responded! Set the regular cookies.
                for cookie in request.cookies().delta() {
                    response.adjoin_header(cookie);
                }

                // And now the session cookies.
                for cookie in request.session().delta() {
                    response.adjoin_header(Session::header_for(cookie));
                }

                response
            }
            Outcome::Forward(data) => {
                // There was no matching route.
                // Rust thinks `request` is still borrowed here, but it's
                // obviously not (data has nothing to do with it), so we
                // convince it to give us another mutable reference.
                // FIXME: Pay the cost to copy Request into UnsafeCell? Pay the
                // cost to use RefCell? Move the call to `issue_response` here
                // to move Request and move directly into an UnsafeCell?
                let request: &'r mut Request = unsafe {
                    &mut *(request as *const Request as *mut Request)
                };

                if request.method() == Method::Head {
                    info_!("Autohandling {} request.", White.paint("HEAD"));
                    request.set_method(Method::Get);
                    let mut response = self.dispatch(request, data);
                    response.strip_body();
                    response
                } else {
                    self.handle_error(Status::NotFound, request)
                }
            }
            Outcome::Failure(status) => self.handle_error(status, request),
        }
    }

    /// Tries to find a `Responder` for a given `request`. It does this by
    /// routing the request and calling the handler for each matching route
    /// until one of the handlers returns success or failure, or there are no
    /// additional routes to try (forward). The corresponding outcome for each
    /// condition is returned.
    #[inline]
    pub(crate) fn route<'r>(&self, request: &'r Request, mut data: Data)
            -> handler::Outcome<'r> {
        // Go through the list of matching routes until we fail or succeed.
        let matches = self.router.route(request);
        for route in matches {
            // Retrieve and set the requests parameters.
            info_!("Matched: {}", route);
            // FIXME: Users should not be able to use this.
            request.set_params(route);

            // Dispatch the request to the handler.
            let outcome = (route.handler)(request, data);

            // Check if the request processing completed or if the request needs
            // to be forwarded. If it does, continue the loop to try again.
            info_!("{} {}", White.paint("Outcome:"), outcome);
            match outcome {
                o@Outcome::Success(_) | o @Outcome::Failure(_) => return o,
                Outcome::Forward(unused_data) => data = unused_data,
            };
        }

        error_!("No matching routes for {}.", request);
        Outcome::Forward(data)
    }

    // TODO: DOC.
    fn handle_error<'r>(&self, status: Status, req: &'r Request) -> Response<'r> {
        warn_!("Responding with {} catcher.", Red.paint(&status));

        // Try to get the active catcher but fallback to user's 500 catcher.
        let catcher = self.catchers.get(&status.code).unwrap_or_else(|| {
            error_!("No catcher found for {}. Using 500 catcher.", status);
            self.catchers.get(&500).expect("500 catcher.")
        });

        // Dispatch to the user's catcher. If it fails, use the default 500.
        let error = Error::NoRoute;
        catcher.handle(error, req).unwrap_or_else(|err_status| {
            error_!("Catcher failed with status: {}!", err_status);
            warn_!("Using default 500 error catcher.");
            let default = self.default_catchers.get(&500).expect("Default 500");
            default.handle(error, req).expect("Default 500 response.")
        })
    }

    /// Create a new `Rocket` application using the configuration information in
    /// `Rocket.toml`. If the file does not exist or if there is an I/O error
    /// reading the file, the defaults are used. See the
    /// [config](/rocket/config/index.html) documentation for more information
    /// on defaults.
    ///
    /// This method is typically called through the `rocket::ignite` alias.
    ///
    /// # Panics
    ///
    /// If there is an error parsing the `Rocket.toml` file, this functions
    /// prints a nice error message and then exits the process.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # {
    /// rocket::ignite()
    /// # };
    /// ```
    pub fn ignite() -> Rocket {
        // Note: init() will exit the process under config errors.
        let (config, initted) = config::init();
        Rocket::configured(config, initted)
    }

    /// Creates a new `Rocket` application using the supplied custom
    /// configuration information. The `Rocket.toml` file, if present, is
    /// ignored. Any environment variables setting config parameters are
    /// ignored. If `log` is `true`, logging is enabled.
    ///
    /// This method is typically called through the `rocket::custom` alias.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rocket::config::{Config, Environment};
    /// # use rocket::config::ConfigError;
    ///
    /// # #[allow(dead_code)]
    /// # fn try_config() -> Result<(), ConfigError> {
    /// let config = Config::build(Environment::Staging)
    ///     .address("1.2.3.4")
    ///     .port(9234)
    ///     .finalize()?;
    ///
    /// # #[allow(unused_variables)]
    /// let app = rocket::custom(config, false);
    /// # Ok(())
    /// # }
    /// ```
    pub fn custom(config: Config, log: bool) -> Rocket {
        let (config, initted) = config::custom_init(config);
        Rocket::configured(config, log && initted)
    }

    fn configured(config: &'static Config, log: bool) -> Rocket {
        if log {
            logger::init(config.log_level);
        }

        info!("🔧  Configured for {}.", config.environment);
        info_!("address: {}", White.paint(&config.address));
        info_!("port: {}", White.paint(&config.port));
        info_!("log: {}", White.paint(config.log_level));
        info_!("workers: {}", White.paint(config.workers));
        info_!("session key: {}", White.paint(config.session_key.kind()));

        for (name, value) in config.extras() {
            info_!("{} {}: {}", Yellow.paint("[extra]"), name, White.paint(value));
        }

        Rocket {
            config: config,
            router: Router::new(),
            default_catchers: catcher::defaults::get(),
            catchers: catcher::defaults::get(),
            state: Container::new()
        }
    }

    /// Mounts all of the routes in the supplied vector at the given `base`
    /// path. Mounting a route with path `path` at path `base` makes the route
    /// available at `base/path`.
    ///
    /// # Examples
    ///
    /// Use the `routes!` macro to mount routes created using the code
    /// generation facilities. Requests to the `/hello/world` URI will be
    /// dispatched to the `hi` route.
    ///
    /// # Panics
    ///
    /// The `base` mount point must be a static path. That is, the mount point
    /// must _not_ contain dynamic path parameters: `<param>`.
    ///
    /// ```rust
    /// # #![feature(plugin)]
    /// # #![plugin(rocket_codegen)]
    /// # extern crate rocket;
    /// #
    /// #[get("/world")]
    /// fn hi() -> &'static str {
    ///     "Hello!"
    /// }
    ///
    /// fn main() {
    /// # if false { // We don't actually want to launch the server in an example.
    ///     rocket::ignite().mount("/hello", routes![hi])
    /// #       .launch();
    /// # }
    /// }
    /// ```
    ///
    /// Manually create a route named `hi` at path `"/world"` mounted at base
    /// `"/hello"`. Requests to the `/hello/world` URI will be dispatched to the
    /// `hi` route.
    ///
    /// ```rust
    /// use rocket::{Request, Route, Data};
    /// use rocket::handler::Outcome;
    /// use rocket::http::Method::*;
    ///
    /// fn hi(_: &Request, _: Data) -> Outcome<'static> {
    ///     Outcome::of("Hello!")
    /// }
    ///
    /// # if false { // We don't actually want to launch the server in an example.
    /// rocket::ignite().mount("/hello", vec![Route::new(Get, "/world", hi)])
    /// #     .launch();
    /// # }
    /// ```
    pub fn mount(mut self, base: &str, routes: Vec<Route>) -> Self {
        info!("🛰  {} '{}':", Magenta.paint("Mounting"), base);

        if base.contains('<') {
            error_!("Bad mount point: '{}'.", base);
            error_!("Mount points must be static paths!");
            panic!("Bad mount point.")
        }

        for mut route in routes {
            let path = format!("{}/{}", base, route.path);
            route.set_path(path);

            info_!("{}", route);
            self.router.add(route);
        }

        self
    }

    /// Registers all of the catchers in the supplied vector.
    ///
    /// # Examples
    ///
    /// ```rust
    /// #![feature(plugin)]
    /// #![plugin(rocket_codegen)]
    ///
    /// extern crate rocket;
    ///
    /// use rocket::Request;
    ///
    /// #[error(500)]
    /// fn internal_error() -> &'static str {
    ///     "Whoops! Looks like we messed up."
    /// }
    ///
    /// #[error(400)]
    /// fn not_found(req: &Request) -> String {
    ///     format!("I couldn't find '{}'. Try something else?", req.uri())
    /// }
    ///
    /// fn main() {
    /// # if false { // We don't actually want to launch the server in an example.
    ///     rocket::ignite().catch(errors![internal_error, not_found])
    /// #       .launch();
    /// # }
    /// }
    /// ```
    pub fn catch(mut self, catchers: Vec<Catcher>) -> Self {
        info!("👾  {}:", Magenta.paint("Catchers"));
        for c in catchers {
            if self.catchers.get(&c.code).map_or(false, |e| !e.is_default()) {
                let msg = "(warning: duplicate catcher!)";
                info_!("{} {}", c, Yellow.paint(msg));
            } else {
                info_!("{}", c);
            }

            self.catchers.insert(c.code, c);
        }

        self
    }

    /// Add `state` to the state managed by this instance of Rocket.
    ///
    /// This method can be called any number of times as long as each call
    /// refers to a different `T`.
    ///
    /// Managed state can be retrieved by any request handler via the
    /// [State](/rocket/struct.State.html) request guard. In particular, if a
    /// value of type `T` is managed by Rocket, adding `State<T>` to the list of
    /// arguments in a request handler instructs Rocket to retrieve the managed
    /// value.
    ///
    /// # Panics
    ///
    /// Panics if state of type `T` is already being managed.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #![feature(plugin)]
    /// # #![plugin(rocket_codegen)]
    /// # extern crate rocket;
    /// use rocket::State;
    ///
    /// struct MyValue(usize);
    ///
    /// #[get("/")]
    /// fn index(state: State<MyValue>) -> String {
    ///     format!("The stateful value is: {}", state.0)
    /// }
    ///
    /// fn main() {
    /// # if false { // We don't actually want to launch the server in an example.
    ///     rocket::ignite()
    ///         .mount("/", routes![index])
    ///         .manage(MyValue(10))
    ///         .launch();
    /// # }
    /// }
    /// ```
    pub fn manage<T: Send + Sync + 'static>(self, state: T) -> Self {
        if !self.state.set::<T>(state) {
            error!("State for this type is already being managed!");
            panic!("Aborting due to duplicately managed state.");
        }

        self
    }

    /// Starts the application server and begins listening for and dispatching
    /// requests to mounted routes and catchers. Unless there is an error, this
    /// function does not return and blocks until program termination.
    ///
    /// # Error
    ///
    /// If there is a problem starting the application, a
    /// [LaunchError](/rocket/struct.LaunchError.html) is returned. Note
    /// that a value of type `LaunchError` panics if dropped without first being
    /// inspected. See the [LaunchError
    /// documentation](/rocket/struct.LaunchError.html) for more
    /// information.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # if false {
    /// rocket::ignite().launch();
    /// # }
    /// ```
    pub fn launch(self) -> LaunchError {
        if self.router.has_collisions() {
            warn!("Route collisions detected!");
        }

        let full_addr = format!("{}:{}", self.config.address, self.config.port);
        let server = match hyper::Server::http(full_addr.as_str()) {
            Ok(hyper_server) => hyper_server,
            Err(e) => return LaunchError::from(e)
        };

        info!("🚀  {} {}{}",
              White.paint("Rocket has launched from"),
              White.bold().paint("http://"),
              White.bold().paint(&full_addr));

        let threads = self.config.workers as usize;
        if let Err(e) = server.handle_threads(self, threads) {
            return LaunchError::from(e);
        }

        unreachable!("the call to `handle_threads` should block on success")
    }
}
