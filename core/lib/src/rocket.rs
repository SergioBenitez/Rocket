use std::fmt;
use std::ops::{Deref, DerefMut};
use std::convert::TryInto;
use std::sync::Arc;

use figment::{Figment, Provider};
use either::Either;
use yansi::Paint;
use tokio::sync::Notify;

use crate::{Route, Catcher, Config, Shutdown};
use crate::router::Router;
use crate::fairing::{Fairing, Fairings};
use crate::phase::{Phase, Build, Building, Ignite, Igniting, Orbit, Orbiting};
use crate::phase::{Stateful, StateRef, State};
use crate::http::uri::{self, Origin};
use crate::http::ext::IntoOwned;
use crate::error::{Error, ErrorKind};
use crate::trace::PaintExt;

/// The application server itself.
///
/// # Phases
///
/// An instance of `Rocket` represents a web server and its state. It progresses
/// through three statically-enforced phases into orbit: build, ignite, orbit.
///
/// ## Build
///
/// All application and server configuration occurs during the [`Build`] phase.
/// This includes setting configuration options, mounting/registering
/// routes/catchers, managing state, and attaching fairings. This is the _only_
/// phase in which an instance can be modified. To finalize changes, an instance
/// is ignited via [`Rocket::ignite()`], progressing it into the _ignite_ phase,
/// or directly launched into orbit with [`Rocket::launch()`] which progress the
/// instance through ignite into orbit.
///
/// ## Ignite
///
/// An instance in the [`Ignite`] phase is in its final configuration, available
/// via [`Rocket::config()`]. Barring user-supplied iterior mutation,
/// application state is guaranteed to remain unchanged beyond this point. An
/// instance in the ignite phase can be launched into orbit to serve requests
/// via [`Rocket::launch()`].
///
/// ## Orbit
///
/// An instance in the [`Orbit`] phase represents a _running_ application,
/// actively serving requests.
///
/// # Launching
///
/// ## Manual Launching
///
/// To launch an instance of `Rocket`, it _must_ progress through all three
/// phases. To progress into the ignite or launch phases, a tokio `async`
/// runtime is required. The [`#[main]`](crate::main) attribute initializes a
/// Rocket-specific tokio runtime and runs attributed async code inside of it:
///
/// ```rust,no_run
/// #[rocket::main]
/// async fn main() -> Result<(), rocket::Error> {
///     rocket::build()
///         .ignite().await?
///         .launch().await
/// }
/// ```
///
/// Note that [`Rocket::launch()`] automatically progresses an instance of
/// `Rocket` from any phase into orbit:
///
/// ```rust,no_run
/// #[rocket::main]
/// async fn main() -> Result<(), rocket::Error> {
///     rocket::build().launch().await
/// }
/// ```
///
/// ## Automatic Launching
///
/// Manually progressing an instance of Rocket though its phases is only
/// necessary when either an instance's finalized state is to be inspected (in
/// the _ignite_ phase) or the instance is expected to deorbit due to
/// [`Rocket::shutdown()`]. In the more common case when neither is required,
/// the [`#[launch]`](crate::launch) attribute can be used. When applied to a
/// function that returns a `Rocket<Build>`, it automatically initializes an
/// `async` runtime and launches the function's returned instance:
///
/// ```rust,no_run
/// # use rocket::launch;
/// use rocket::{Rocket, Build};
///
/// #[launch]
/// fn rocket() -> Rocket<Build> {
///     rocket::build()
/// }
/// ```
///
/// To avoid needing to import _any_ items in the common case, the `launch`
/// attribute will infer a return type written as `_` as `Rocket<Build>`:
///
/// ```rust,no_run
/// # use rocket::launch;
/// #[launch]
/// fn rocket() -> _ {
///     rocket::build()
/// }
/// ```
pub struct Rocket<P: Phase>(pub(crate) P::State);

impl Rocket<Build> {
    /// Create a new `Rocket` application using the default configuration
    /// provider, [`Config::figment()`].
    ///
    /// This method is typically called through the
    /// [`rocket::build()`](crate::build) alias.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rocket::launch;
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build()
    /// }
    /// ```
    #[inline(always)]
    pub fn build() -> Self {
        Rocket::custom(Config::figment())
    }

    /// Creates a new `Rocket` application using the supplied configuration
    /// provider.
    ///
    /// This method is typically called through the
    /// [`rocket::custom()`](crate::custom()) alias.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::launch;
    /// use rocket::figment::{Figment, providers::{Toml, Env, Format}};
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     let figment = Figment::from(rocket::Config::default())
    ///         .merge(Toml::file("MyApp.toml").nested())
    ///         .merge(Env::prefixed("MY_APP_").global());
    ///
    ///     rocket::custom(figment)
    /// }
    /// ```
    pub fn custom<T: Provider>(provider: T) -> Self {
        Rocket(Building {
            figment: Figment::from(provider),
            ..Default::default()
        })
    }

    /// Sets the configuration provider in `self` to `provider`.
    ///
    /// A [`Figment`] generated from the current `provider` can _always_ be
    /// retrieved via [`Rocket::figment()`]. However, because the provider can
    /// be changed at any point prior to ignition, a [`Config`] can only be
    /// retrieved in the ignite or orbit phases, or by manually extracing one
    /// from a particular figment.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::Config;
    /// # use std::net::Ipv4Addr;
    /// # use std::path::{Path, PathBuf};
    /// # type Result = std::result::Result<(), rocket::Error>;
    ///
    /// let config = Config {
    ///     port: 7777,
    ///     address: Ipv4Addr::new(18, 127, 0, 1).into(),
    ///     temp_dir: PathBuf::from("/tmp/config-example"),
    ///     ..Config::debug_default()
    /// };
    ///
    /// # let _: Result = rocket::async_test(async move {
    /// let rocket = rocket::custom(&config).ignite().await?;
    /// assert_eq!(rocket.config().port, 7777);
    /// assert_eq!(rocket.config().address, Ipv4Addr::new(18, 127, 0, 1));
    /// assert_eq!(rocket.config().temp_dir, Path::new("/tmp/config-example"));
    ///
    /// // Create a new figment which modifies _some_ keys the existing figment:
    /// let figment = rocket.figment().clone()
    ///     .merge((Config::PORT, 8888))
    ///     .merge((Config::ADDRESS, "171.64.200.10"));
    ///
    /// let rocket = rocket::custom(&config)
    ///     .configure(figment)
    ///     .ignite().await?;
    ///
    /// assert_eq!(rocket.config().port, 8888);
    /// assert_eq!(rocket.config().address, Ipv4Addr::new(171, 64, 200, 10));
    /// assert_eq!(rocket.config().temp_dir, Path::new("/tmp/config-example"));
    /// # Ok(())
    /// # });
    /// ```
    pub fn configure<T: Provider>(mut self, provider: T) -> Self {
        self.figment = Figment::from(provider);
        self
    }

    fn load<'a, B, T, F, M>(mut self, kind: &str, base: B, items: Vec<T>, m: M, f: F) -> Self
        where B: TryInto<Origin<'a>> + Clone + fmt::Display,
              B::Error: fmt::Display,
              M: Fn(&Origin<'a>, T) -> Result<T, uri::Error<'static>>,
              F: Fn(&mut Self, T),
              T: Clone + fmt::Display,
    {
        let mut base = base.clone().try_into()
            .map(|origin| origin.into_owned())
            .unwrap_or_else(|error| {
                let span = error_span!("invalid_base", base = %Paint::white(&base), "invalid {} base", kind);
                let _e = span.enter();
                error!(%error);
                panic!("aborting due to {} base error", kind);
            });

        if base.query().is_some() {
            warn!("query in {} base '{}' is ignored", kind, Paint::white(&base));
            base.clear_query();
        }

        for unmounted_item in items {
            let item = m(&base, unmounted_item.clone())
                .unwrap_or_else(|error| {
                    let span = error_span!("malformed_item", %unmounted_item, "malformed URI in {}", kind);
                    let _e = span.enter();
                    error!(%error);
                    panic!("aborting due to invalid {} URI", kind);
                });

            f(&mut self, item)
        }

        self
    }

    /// Mounts all of the routes in the supplied vector at the given `base`
    /// path. Mounting a route with path `path` at path `base` makes the route
    /// available at `base/path`.
    ///
    /// # Panics
    ///
    /// Panics if either:
    ///   * the `base` mount point is not a valid static path: a valid origin
    ///     URI without dynamic parameters.
    ///
    ///   * any route's URI is not a valid origin URI.
    ///
    ///     **Note:** _This kind of panic is guaranteed not to occur if the routes
    ///     were generated using Rocket's code generation._
    ///
    /// # Examples
    ///
    /// Use the `routes!` macro to mount routes created using the code
    /// generation facilities. Requests to the `/hello/world` URI will be
    /// dispatched to the `hi` route.
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// #
    /// #[get("/world")]
    /// fn hi() -> &'static str {
    ///     "Hello!"
    /// }
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build().mount("/hello", routes![hi])
    /// }
    /// ```
    ///
    /// Manually create a route named `hi` at path `"/world"` mounted at base
    /// `"/hello"`. Requests to the `/hello/world` URI will be dispatched to the
    /// `hi` route.
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// use rocket::{Request, Route, Data, route};
    /// use rocket::http::Method;
    ///
    /// fn hi<'r>(req: &'r Request, _: Data) -> route::BoxFuture<'r> {
    ///     route::Outcome::from(req, "Hello!").pin()
    /// }
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     let hi_route = Route::new(Method::Get, "/world", hi);
    ///     rocket::build().mount("/hello", vec![hi_route])
    /// }
    /// ```
    pub fn mount<'a, B, R>(self, base: B, routes: R) -> Self
        where B: TryInto<Origin<'a>> + Clone + fmt::Display,
              B::Error: fmt::Display,
              R: Into<Vec<Route>>
    {
        self.load("route", base, routes.into(),
            |base, route| route.map_base(|old| format!("{}{}", base, old)),
            |r, route| r.0.routes.push(route))
    }

    /// Registers all of the catchers in the supplied vector, scoped to `base`.
    ///
    /// # Panics
    ///
    /// Panics if `base` is not a valid static path: a valid origin URI without
    /// dynamic parameters.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// use rocket::Request;
    ///
    /// #[catch(500)]
    /// fn internal_error() -> &'static str {
    ///     "Whoops! Looks like we messed up."
    /// }
    ///
    /// #[catch(400)]
    /// fn not_found(req: &Request) -> String {
    ///     format!("I couldn't find '{}'. Try something else?", req.uri())
    /// }
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build().register("/", catchers![internal_error, not_found])
    /// }
    /// ```
    pub fn register<'a, B, C>(self, base: B, catchers: C) -> Self
        where B: TryInto<Origin<'a>> + Clone + fmt::Display,
              B::Error: fmt::Display,
              C: Into<Vec<Catcher>>
    {
        self.load("catcher", base, catchers.into(),
            |base, catcher| catcher.map_base(|old| format!("{}{}", base, old)),
            |r, catcher| r.0.catchers.push(catcher))
    }

    /// Add `state` to the state managed by this instance of Rocket.
    ///
    /// This method can be called any number of times as long as each call
    /// refers to a different `T`.
    ///
    /// Managed state can be retrieved by any request handler via the
    /// [`State`](crate::State) request guard. In particular, if a value of type `T`
    /// is managed by Rocket, adding `State<T>` to the list of arguments in a
    /// request handler instructs Rocket to retrieve the managed value.
    ///
    /// # Panics
    ///
    /// Panics if state of type `T` is already being managed.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// use rocket::State;
    ///
    /// struct MyInt(isize);
    /// struct MyString(String);
    ///
    /// #[get("/int")]
    /// fn int(state: State<'_, MyInt>) -> String {
    ///     format!("The stateful int is: {}", state.0)
    /// }
    ///
    /// #[get("/string")]
    /// fn string<'r>(state: State<'r, MyString>) -> &'r str {
    ///     &state.inner().0
    /// }
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build()
    ///         .manage(MyInt(10))
    ///         .manage(MyString("Hello, managed state!".to_string()))
    ///         .mount("/", routes![int, string])
    /// }
    /// ```
    pub fn manage<T>(self, state: T) -> Self
        where T: Send + Sync + 'static
    {
        let type_name = std::any::type_name::<T>();
        if !self.state.set(state) {
            error!("state for type '{}' is already being managed", type_name);
            panic!("aborting due to duplicately managed state");
        }

        self
    }

    /// Attaches a fairing to this instance of Rocket. No fairings are eagerly
    /// excuted; fairings are executed at their appropriate time.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// use rocket::Rocket;
    /// use rocket::fairing::AdHoc;
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build()
    ///         .attach(AdHoc::on_liftoff("Liftoff Message", |_| Box::pin(async {
    ///             println!("We have liftoff!");
    ///         })))
    /// }
    /// ```
    pub fn attach<F: Fairing>(mut self, fairing: F) -> Self {
        self.fairings.add(Box::new(fairing));
        self
    }

    /// Returns a `Future` that transitions this instance of `Rocket` into the
    /// _ignite_ phase.
    ///
    /// When `await`ed, the future runs all _ignite_ fairings in serial,
    /// [attach](Rocket::attach()) order, and verifies that `self` represents a
    /// valid instance of `Rocket` ready for launch. This means that:
    ///
    ///   * All ignite fairings succeeded.
    ///   * A valid [`Config`] was extracted from [`Rocket::figment()`].
    ///   * If `secrets` are enabled, the extracted `Config` contains a safe
    ///     secret key.
    ///   * There are no [`Route#collisions`] or [`Catcher#collisions`]
    ///     collisions.
    ///
    /// If any of these conditions fail to be met, a respective [`Error`] is
    /// returned.
    ///
    /// [configured]: Rocket::figment()
    ///
    /// # Example
    ///
    /// ```rust
    /// use rocket::fairing::AdHoc;
    ///
    /// #[rocket::main]
    /// async fn main() -> Result<(), rocket::Error> {
    ///     let rocket = rocket::build()
    ///         # .configure(rocket::Config::debug_default())
    ///         .attach(AdHoc::on_ignite("Manage State", |rocket| async move {
    ///             rocket.manage(String::from("managed string"))
    ///         }));
    ///
    ///     // No fairings are run until ignition occurs.
    ///     assert!(rocket.state::<String>().is_none());
    ///
    ///     let rocket = rocket.ignite().await?;
    ///     assert_eq!(rocket.state::<String>().unwrap(), "managed string");
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn ignite(mut self) -> Result<Rocket<Ignite>, Error> {
        // We initialize the logger here so that logging from fairings are
        // visible but change the max-log-level when we have a final config.
        crate::trace::try_init(&Config::debug_default());
        self = Fairings::handle_ignite(self).await;
        self.fairings.audit().map_err(|f| ErrorKind::FailedFairings(f.to_vec()))?;

        // Extract the configuration; initialize the logger.
        #[allow(unused_mut)]
        let mut config = self.figment.extract::<Config>().map_err(ErrorKind::Config)?;
        crate::trace::try_init(&config);

        // Check for safely configured secrets.
        #[cfg(feature = "secrets")]
        if !config.secret_key.is_provided() {
            let profile = self.figment.profile();
            if profile != Config::DEBUG_PROFILE {
                return Err(Error::new(ErrorKind::InsecureSecretKey(profile.clone())));
            }

            if config.secret_key.is_zero() {
                config.secret_key = crate::config::SecretKey::generate()
                    .unwrap_or(crate::config::SecretKey::zero());
            }
        };

        // Initialize the router; check for collisions.
        let mut router = Router::new();
        self.routes.clone().into_iter().for_each(|r| router.add_route(r));
        self.catchers.clone().into_iter().for_each(|c| router.add_catcher(c));
        router.finalize().map_err(ErrorKind::Collisions)?;

        // Finally, freeze managed state.
        self.state.freeze();

        // Log everything we know: config, routes, catchers, fairings.
        // TODO: Store/print managed state type names?
        config.pretty_print(self.figment());
        log_items("🛰  ", "Routes", self.routes(), |r| &r.uri.base, |r| &r.uri);
        log_items("👾 ", "Catchers", self.catchers(), |c| &c.base, |c| &c.base);
        self.fairings.pretty_print();

        // Ignite the rocket.
        Ok(Rocket(Igniting {
            router, config,
            shutdown: Arc::new(Notify::new()),
            figment: self.0.figment,
            fairings: self.0.fairings,
            state: self.0.state,
        }))
    }
}

fn log_items<T, I, B, O>(e: &str, t: &str, items: I, base: B, origin: O)
    where T: fmt::Display + Copy, I: Iterator<Item = T>,
          B: Fn(&T) -> &Origin<'_>, O: Fn(&T) -> &Origin<'_>
{
    let mut items: Vec<_> = items.collect();
    if !items.is_empty() {
        info!(target: "rocket::support", "{}{}:", Paint::emoji(e), Paint::magenta(t));
    }

    items.sort_by_key(|i| origin(i).path().as_str().chars().count());
    items.sort_by_key(|i| origin(i).path_segments().len());
    items.sort_by_key(|i| base(i).path().as_str().chars().count());
    items.sort_by_key(|i| base(i).path_segments().len());
    items.iter().for_each(|i| info!(target: "rocket::support", "{}", i));
}

impl Rocket<Ignite> {
    /// Returns the finalized, active configuration. This is guaranteed to
    /// remain stable through ignition and into orbit.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// #[rocket::main]
    /// async fn main() -> Result<(), rocket::Error> {
    ///     let rocket = rocket::build().ignite().await?;
    ///     let config = rocket.config();
    ///     Ok(())
    /// }
    /// ```
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a handle which can be used to notify this instance of Rocket to
    /// stop serving connections, resolving the future returned by
    /// [`Rocket::launch()`]. If [`Shutdown::notify()`] is called _before_ the
    /// instance is launched, it will be immediately shutdown after liftoff.
    ///
    /// # Caveats
    ///
    /// Due to [bugs](https://github.com/hyperium/hyper/issues/1885) in Rocket's
    /// upstream HTTP library, graceful shutdown currently works by stopping new
    /// connections from arriving without stopping in-process connections from
    /// sending or receiving. As a result, shutdown will stall if a response is
    /// infinite or if a client stalls a connection.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use std::time::Duration;
    /// use rocket::tokio::{self, time};
    ///
    /// #[rocket::main]
    /// async fn main() -> Result<(), rocket::Error> {
    ///     let rocket = rocket::build().ignite().await?;
    ///
    ///     let shutdown = rocket.shutdown();
    ///     tokio::spawn(async move {
    ///         time::sleep(time::Duration::from_secs(5)).await;
    ///         shutdown.notify();
    ///     });
    ///
    ///     // The `launch()` future resolves after ~5 seconds.
    ///     let result = rocket.launch().await;
    ///     assert!(result.is_ok());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn shutdown(&self) -> Shutdown {
        Shutdown(self.shutdown.clone())
    }

    fn into_orbit(self) -> Rocket<Orbit> {
        Rocket(Orbiting {
            router: self.0.router,
            fairings: self.0.fairings,
            figment: self.0.figment,
            config: self.0.config,
            state: self.0.state,
            shutdown: self.0.shutdown,
        })
    }

    async fn _local_launch(self) -> Rocket<Orbit> {
        let rocket = self.into_orbit();
        rocket.fairings.handle_liftoff(&rocket).await;
        info!(target: "rocket::support", "{}{}", Paint::emoji("🚀 "),
            Paint::default("Rocket has launched into local orbit").bold());

        rocket
    }

    async fn _launch(self) -> Result<(), Error> {
        self.into_orbit().default_tcp_http_server(|rkt| Box::pin(async move {
            rkt.fairings.handle_liftoff(&rkt).await;

            let proto = rkt.config.tls_enabled().then(|| "https").unwrap_or("http");
            let addr = format!("{}://{}:{}", proto, rkt.config.address, rkt.config.port);
            info!(target: "rocket::support", "{}{} {}",
                Paint::emoji("🚀 "),
                Paint::default("Rocket has launched from").bold(),
                Paint::default(&addr).bold().underline());
        })).await
    }
}

impl Rocket<Orbit> {
    /// Returns the finalized, active configuration. This is guaranteed to
    /// remain stable after [`Rocket::ignite()`], through ignition and into
    /// orbit.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// use rocket::fairing::AdHoc;
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build()
    ///         .attach(AdHoc::on_liftoff("Config", |rocket| Box::pin(async move {
    ///             println!("Rocket launch config: {:?}", rocket.config());
    ///         })))
    /// }
    /// ```
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a handle which can be used to notify this instance of Rocket to
    /// stop serving connections, resolving the future returned by
    /// [`Rocket::launch()`].
    ///
    /// # Caveats
    ///
    /// Due to [bugs](https://github.com/hyperium/hyper/issues/1885) in Rocket's
    /// upstream HTTP library, graceful shutdown currently works by stopping new
    /// connections from arriving without stopping in-process connections from
    /// sending or receiving. As a result, shutdown will stall if a response is
    /// infinite or if a client stalls a connection.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # #[macro_use] extern crate rocket;
    /// use rocket::tokio::{self, time};
    /// use rocket::fairing::AdHoc;
    ///
    /// #[launch]
    /// fn rocket() -> _ {
    ///     rocket::build()
    ///         .attach(AdHoc::on_liftoff("Shutdown", |rocket| Box::pin(async move {
    ///             let shutdown = rocket.shutdown();
    ///             tokio::spawn(async move {
    ///                 time::sleep(time::Duration::from_secs(5)).await;
    ///                 shutdown.notify();
    ///             });
    ///         })))
    /// }
    /// ```
    pub fn shutdown(&self) -> Shutdown {
        Shutdown(self.shutdown.clone())
    }
}

impl<P: Phase> Rocket<P> {
    /// Returns an iterator over all of the routes mounted on this instance of
    /// Rocket. The order is unspecified.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::*;
    /// use rocket::Rocket;
    /// use rocket::fairing::AdHoc;
    ///
    /// #[get("/hello")]
    /// fn hello() -> &'static str {
    ///     "Hello, world!"
    /// }
    ///
    /// let rocket = rocket::build()
    ///     .mount("/", routes![hello])
    ///     .mount("/hi", routes![hello]);
    ///
    /// assert_eq!(rocket.routes().count(), 2);
    /// assert!(rocket.routes().any(|r| r.uri == "/hello"));
    /// assert!(rocket.routes().any(|r| r.uri == "/hi/hello"));
    /// ```
    pub fn routes(&self) -> impl Iterator<Item = &Route> {
        match self.0.as_state_ref() {
            StateRef::Build(p) => Either::Left(p.routes.iter()),
            StateRef::Ignite(p) => Either::Right(p.router.routes()),
            StateRef::Orbit(p) => Either::Right(p.router.routes()),
        }
    }

    /// Returns an iterator over all of the catchers registered on this instance
    /// of Rocket. The order is unspecified.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use rocket::*;
    /// use rocket::Rocket;
    /// use rocket::fairing::AdHoc;
    ///
    /// #[catch(404)] fn not_found() -> &'static str { "Nothing here, sorry!" }
    /// #[catch(500)] fn just_500() -> &'static str { "Whoops!?" }
    /// #[catch(default)] fn some_default() -> &'static str { "Everything else." }
    ///
    /// let rocket = rocket::build()
    ///     .register("/foo", catchers![not_found])
    ///     .register("/", catchers![just_500, some_default]);
    ///
    /// assert_eq!(rocket.catchers().count(), 3);
    /// assert!(rocket.catchers().any(|c| c.code == Some(404) && c.base == "/foo"));
    /// assert!(rocket.catchers().any(|c| c.code == Some(500) && c.base == "/"));
    /// assert!(rocket.catchers().any(|c| c.code == None && c.base == "/"));
    /// ```
    pub fn catchers(&self) -> impl Iterator<Item = &Catcher> {
        match self.0.as_state_ref() {
            StateRef::Build(p) => Either::Left(p.catchers.iter()),
            StateRef::Ignite(p) => Either::Right(p.router.catchers()),
            StateRef::Orbit(p) => Either::Right(p.router.catchers()),
        }
    }

    /// Returns `Some` of the managed state value for the type `T` if it is
    /// being managed by `self`. Otherwise, returns `None`.
    ///
    /// # Example
    ///
    /// ```rust
    /// #[derive(PartialEq, Debug)]
    /// struct MyState(&'static str);
    ///
    /// let rocket = rocket::build().manage(MyState("hello!"));
    /// assert_eq!(rocket.state::<MyState>().unwrap(), &MyState("hello!"));
    /// ```
    pub fn state<T: Send + Sync + 'static>(&self) -> Option<&T> {
        match self.0.as_state_ref() {
            StateRef::Build(p) => p.state.try_get(),
            StateRef::Ignite(p) => p.state.try_get(),
            StateRef::Orbit(p) => p.state.try_get(),
        }
    }

    /// Returns the figment derived from the configuration provider set for
    /// `self`. To extract a typed config, prefer to use
    /// [`AdHoc::config()`](crate::fairing::AdHoc::config()).
    ///
    /// # Example
    ///
    /// ```rust
    /// let rocket = rocket::build();
    /// let figment = rocket.figment();
    /// ```
    pub fn figment(&self) -> &Figment {
        match self.0.as_state_ref() {
            StateRef::Build(p) => &p.figment,
            StateRef::Ignite(p) => &p.figment,
            StateRef::Orbit(p) => &p.figment,
        }
    }

    pub(crate) async fn local_launch(self) -> Result<Rocket<Orbit>, Error> {
        let rocket = match self.0.into_state() {
            State::Build(s) => Rocket::from(s).ignite().await?._local_launch().await,
            State::Ignite(s) => Rocket::from(s)._local_launch().await,
            State::Orbit(s) => Rocket::from(s)
        };

        Ok(rocket)
    }

    /// Returns a `Future` that transitions this instance of `Rocket` from any
    /// phase into the _orbit_ phase. When `await`ed, the future drives the
    /// server forward, listening for and dispatching requests to mounted routes
    /// and catchers.
    ///
    /// In addition to all of the processes that occur during
    /// [ignition](Rocket::ignite()), a successful launch results in _liftoff_
    /// fairings being executed _after_ binding to any respective network
    /// interfaces but before serving the first request. Liftoff fairings are
    /// run concurrently; resolution of all fairings is `await`ed before
    /// resuming request serving.
    ///
    /// The `Future` resolves as an `Err` if any of the following occur:
    ///
    ///   * there is an error igniting; see [`Rocket::ignite()`].
    ///   * there is an I/O error starting the server.
    ///   * an unrecoverable, system-level error occurs while running.
    ///
    /// The `Future` resolves as an `Ok` if any of the following occur:
    ///
    ///   * the server is shutdown via [`Shutdown::notify()`].
    ///   * if the `ctrlc` config option is `true`, when `Ctrl+C` is pressed.
    ///
    /// The `Future` does not resolve otherwise.
    ///
    /// # Error
    ///
    /// If there is a problem starting the application, an [`Error`] is
    /// returned. Note that a value of type `Error` panics if dropped without
    /// first being inspected. See the [`Error`] documentation for more
    /// information.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// #[rocket::main]
    /// async fn main() {
    ///     let result = rocket::build().launch().await;
    ///
    ///     // this is reachable only after `Shutdown::notify()` or `Ctrl+C`.
    ///     println!("Rocket: deorbit.");
    /// }
    /// ```
    pub async fn launch(self) -> Result<(), Error> {
        match self.0.into_state() {
            State::Build(s) => Rocket::from(s).ignite().await?._launch().await,
            State::Ignite(s) => Rocket::from(s)._launch().await,
            State::Orbit(_) => Ok(())
        }
    }
}

#[doc(hidden)]
impl<P: Phase> Deref for Rocket<P> {
    type Target = P::State;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[doc(hidden)]
impl<P: Phase> DerefMut for Rocket<P> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<P: Phase> fmt::Debug for Rocket<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
