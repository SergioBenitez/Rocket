use rocket::{Request, State};
use rocket::http::Status;
use rocket::request::{self, FromRequest};

use crate::templates::ContextManager;

/// Request guard for dynamically querying template metadata.
///
/// # Usage
///
/// The `Metadata` type implements Rocket's [`FromRequest`] trait, so it can be
/// used as a request guard in any request handler.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[macro_use] extern crate rocket_contrib;
/// use rocket_contrib::templates::{Template, Metadata};
///
/// #[get("/")]
/// fn homepage(metadata: Metadata) -> Template {
///     # use std::collections::HashMap;
///     # let context: HashMap<String, String> = HashMap::new();
///     // Conditionally render a template if it's available.
///     if metadata.contains_template("some-template") {
///         Template::render("some-template", &context)
///     } else {
///         Template::render("fallback", &context)
///     }
/// }
///
///
/// fn main() {
///     rocket::build()
///         .attach(Template::fairing())
///         // ...
///     # ;
/// }
/// ```
pub struct Metadata<'a>(&'a ContextManager);

impl Metadata<'_> {
    /// Returns `true` if the template with the given `name` is currently
    /// loaded.  Otherwise, returns `false`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// # extern crate rocket_contrib;
    /// #
    /// use rocket_contrib::templates::Metadata;
    ///
    /// #[get("/")]
    /// fn handler(metadata: Metadata) {
    ///     // Returns `true` if the template with name `"name"` was loaded.
    ///     let loaded = metadata.contains_template("name");
    /// }
    /// ```
    pub fn contains_template(&self, name: &str) -> bool {
        self.0.context().templates.contains_key(name)
    }

    /// Returns `true` if template reloading is enabled.
    ///
    /// # Example
    ///
    /// ```rust
    /// # #[macro_use] extern crate rocket;
    /// # extern crate rocket_contrib;
    /// #
    /// use rocket_contrib::templates::Metadata;
    ///
    /// #[get("/")]
    /// fn handler(metadata: Metadata) {
    ///     // Returns `true` if template reloading is enabled.
    ///     let reloading = metadata.reloading();
    /// }
    /// ```
    pub fn reloading(&self) -> bool {
        self.0.is_reloading()
    }
}

/// Retrieves the template metadata. If a template fairing hasn't been attached,
/// an error is printed and an empty `Err` with status `InternalServerError`
/// (`500`) is returned.
#[rocket::async_trait]
impl<'r> FromRequest<'r> for Metadata<'r> {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> request::Outcome<Self, ()> {
        request.guard::<State<'_, ContextManager>>().await
            .succeeded()
            .and_then(|cm| Some(request::Outcome::Success(Metadata(cm.inner()))))
            .unwrap_or_else(|| {
                error_span!("missing_fairing", "Uninitialized template context: missing fairing.").in_scope(|| {
                    info!("To use templates, you must attach `Template::fairing()`.");
                    info!("See the `Template` documentation for more information.");
                });
                request::Outcome::Failure((Status::InternalServerError, ()))
            })
    }
}
