use std::collections::HashMap;
use std::str::from_utf8_unchecked;
use std::cmp::min;
use std::io::{self, Write};

use term_painter::Color::*;
use term_painter::ToStyle;

use {logger, handler};
use ext::ReadExt;
use config::{self, Config};
use request::{Request, FormItems};
use data::Data;
use response::{Body, Response};
use router::{Router, Route};
use catcher::{self, Catcher};
use outcome::Outcome;
use error::Error;

use http::{Method, Status};
use http::hyper::{self, header};
use http::uri::URI;

/// The main `Rocket` type: used to mount routes and catchers and launch the
/// application.
pub struct Rocket {
    address: String,
    port: usize,
    router: Router,
    default_catchers: HashMap<u16, Catcher>,
    catchers: HashMap<u16, Catcher>,
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
        let (_, h_method, h_headers, h_uri, _, h_body) = hyp_req.deconstruct();

        // Convert the Hyper request into a Rocket request.
        let mut request = match Request::from_hyp(h_method, h_headers, h_uri) {
            Ok(request) => request,
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
                let r = self.handle_error(Status::InternalServerError, &request);
                return self.issue_response(r, res);
            }
        };

        // Dispatch the request to get a response, then write that response out.
        let response = self.dispatch(&mut request, data);
        self.issue_response(response, res)
    }
}

impl Rocket {
    #[inline]
    fn issue_response(&self, mut response: Response, hyp_res: hyper::FreshResponse) {
        // Add the 'rocket' server header, and write out the response.
        // TODO: If removing Hyper, write out `Date` header too.
        response.set_header(header::Server("rocket".to_string()));

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
            let name = header.name.into_owned();
            let value = vec![header.value.into_owned().into()];
            hyp_res.headers_mut().set_raw(name, value);
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

    /// Preprocess the request for Rocket-specific things. At this time, we're
    /// only checking for _method in forms. Keep this in-sync with derive_form
    /// when preprocessing form fields.
    fn preprocess_request(&self, req: &mut Request, data: &Data) {
        // Check if this is a form and if the form contains the special _method
        // field which we use to reinterpret the request's method.
        let data_len = data.peek().len();
        let (min_len, max_len) = ("_method=get".len(), "_method=delete".len());
        if req.method() == Method::Post && req.content_type().is_form() && data_len >= min_len {
            let form = unsafe {
                from_utf8_unchecked(&data.peek()[..min(data_len, max_len)])
            };

            let mut form_items = FormItems(form);
            if let Some(("_method", value)) = form_items.next() {
                if let Ok(method) = value.parse() {
                    req.set_method(method);
                }
            }
        }
    }

    #[doc(hidden)]
    #[inline(always)]
    pub fn dispatch<'r>(&self, request: &'r mut Request, data: Data) -> Response<'r> {
        // Do a bit of preprocessing before routing.
        self.preprocess_request(request, &data);

        // Route the request to get a response.
        match self.route(request, data) {
            Outcome::Success(mut response) => {
                let cookie_delta = request.cookies().delta();
                if !cookie_delta.is_empty() {
                    response.adjoin_header(header::SetCookie(cookie_delta));
                }

                response
            }
            Outcome::Forward(data) => {
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
    /// until one of the handlers returns success or failure. If a handler
    /// returns a failure, or there are no matching handlers willing to accept
    /// the request, this function returns an `Err` with the status code.
    #[doc(hidden)]
    #[inline(always)]
    pub fn route<'r>(&self, request: &'r Request, mut data: Data)
            -> handler::Outcome<'r> {
        // Go through the list of matching routes until we fail or succeed.
        info!("{}:", request);
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
    #[doc(hidden)]
    pub fn handle_error<'r>(&self, status: Status, req: &'r Request) -> Response<'r> {
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
        if initted {
            logger::init(config.log_level);
        }

        Rocket::custom(config)
    }

    /// Creates a new `Rocket` application using the supplied custom
    /// configuration information. Ignores the `Rocket.toml` file. Does not
    /// enable logging. To enable logging, use the hidden
    /// `logger::init(LoggingLevel)` method.
    ///
    /// This method is typically called through the `rocket::custom` alias.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rocket::config::{Config, Environment};
    /// # use rocket::config::ConfigError;
    ///
    /// # fn try_config() -> Result<(), ConfigError> {
    /// let config = Config::default_for(Environment::active()?, "/custom")?
    ///     .address("1.2.3.4".into())
    ///     .port(9234);
    ///
    /// let app = rocket::custom(&config);
    /// # Ok(())
    /// # }
    /// ```
    pub fn custom(config: &Config) -> Rocket {
        info!("🔧  Configured for {}.", config.env);
        info_!("listening: {}:{}",
               White.paint(&config.address),
               White.paint(&config.port));
        info_!("logging: {:?}", White.paint(config.log_level));

        let session_key = config.take_session_key();
        if session_key.is_some() {
            info_!("session key: {}", White.paint("present"));
            warn_!("Signing and encryption of cookies is currently disabled.");
            warn_!("See https://github.com/SergioBenitez/Rocket/issues/20 for info.");
        }

        for (name, value) in config.extras() {
            info_!("{} {}: {}", Yellow.paint("[extra]"), name, White.paint(value));
        }

        Rocket {
            address: config.address.clone(),
            port: config.port,
            router: Router::new(),
            default_catchers: catcher::defaults::get(),
            catchers: catcher::defaults::get(),
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
    /// #       .launch()
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
    /// #     .launch()
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
    /// #       .launch()
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

    /// Starts the application server and begins listening for and dispatching
    /// requests to mounted routes and catchers.
    ///
    /// # Panics
    ///
    /// If the server could not be started, this method prints the reason and
    /// then exits the process.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # if false {
    /// rocket::ignite().launch()
    /// # }
    /// ```
    pub fn launch(self) {
        if self.router.has_collisions() {
            warn!("Route collisions detected!");
        }

        let full_addr = format!("{}:{}", self.address, self.port);
        let server = match hyper::Server::http(full_addr.as_str()) {
            Ok(hyper_server) => hyper_server,
            Err(e) => {
                error!("Failed to start server.");
                panic!("{}", e);
            }
        };

        info!("🚀  {} {}{}...",
              White.paint("Rocket has launched from"),
              White.bold().paint("http://"),
              White.bold().paint(&full_addr));

        server.handle(self).unwrap();
    }
}
