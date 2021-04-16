//! Dynamic template engine support for handlebars and tera.
//!
//! # Overview
//!
//! The general outline for using templates in Rocket is:
//!
//!   0. Enable the `rocket_contrib` feature corresponding to your templating
//!      engine(s) of choice:
//!
//!      ```toml
//!      [dependencies.rocket_contrib]
//!      version = "0.5.0-dev"
//!      default-features = false
//!      features = ["handlebars_templates", "tera_templates"]
//!      ```
//!
//!   1. Write your template files in Handlebars (extension: `.hbs`) or tera
//!      (extensions: `.tera`) in the templates directory (default:
//!      `{rocket_root}/templates`).
//!
//!   2. Attach the template fairing, [`Template::fairing()`]:
//!
//!      ```rust
//!      # extern crate rocket;
//!      # extern crate rocket_contrib;
//!      use rocket_contrib::templates::Template;
//!
//!      fn main() {
//!          rocket::build()
//!              .attach(Template::fairing())
//!              // ...
//!          # ;
//!      }
//!      ```
//!
//!   3. Return a [`Template`] using [`Template::render()`], supplying the name
//!      of the template file minus the last two extensions, from a handler.
//!
//!      ```rust
//!      # #[macro_use] extern crate rocket;
//!      # #[macro_use] extern crate rocket_contrib;
//!      # fn context() {  }
//!      use rocket_contrib::templates::Template;
//!
//!      #[get("/")]
//!      fn index() -> Template {
//!          let context = context();
//!          Template::render("template-name", &context)
//!      }
//!      ```
//!
//! ## Discovery
//!
//! Template names passed in to [`Template::render()`] must correspond to a
//! previously discovered template in the configured template directory. The
//! template directory is configured via the `template_dir` configuration
//! parameter and defaults to `templates/`. The path set in `template_dir` is
//! relative to the Rocket configuration file. See the [configuration
//! chapter](https://rocket.rs/master/guide/configuration/#extras) of the guide
//! for more information on configuration.
//!
//! The corresponding templating engine used for a given template is based on a
//! template's extension. At present, this library supports the following
//! engines and extensions:
//!
//! | Engine       | Version | Extension |
//! |--------------|---------|-----------|
//! | [Tera]       | 1       | `.tera`   |
//! | [Handlebars] | 2       | `.hbs`    |
//!
//! [Tera]: https://docs.rs/crate/tera/1
//! [Handlebars]: https://docs.rs/crate/handlebars/2
//!
//! Any file that ends with one of these extension will be discovered and
//! rendered with the corresponding templating engine. The _name_ of the
//! template will be the path to the template file relative to `template_dir`
//! minus at most two extensions. The following table illustrates this mapping:
//!
//! | path                                          | name                  |
//! |-----------------------------------------------|-----------------------|
//! | {template_dir}/index.html.hbs                 | index                 |
//! | {template_dir}/index.tera                     | index                 |
//! | {template_dir}/index.hbs                      | index                 |
//! | {template_dir}/dir/index.hbs                  | dir/index             |
//! | {template_dir}/dir/index.html.tera            | dir/index             |
//! | {template_dir}/index.template.html.hbs        | index.template        |
//! | {template_dir}/subdir/index.template.html.hbs | subdir/index.template |
//!
//! The recommended naming scheme is to use two extensions: one for the file
//! type, and one for the template extension. This means that template
//! extensions should look like: `.html.hbs`, `.html.tera`, `.xml.hbs`, etc.
//!
//! ## Template Fairing
//!
//! Template discovery is actualized by the template fairing, which itself is
//! created via [`Template::fairing()`], [`Template::custom()`], or
//! [`Template::try_custom()`], the latter two allowing customizations to
//! enabled templating engines. In order for _any_ templates to be rendered, the
//! template fairing _must_ be [attached](rocket::Rocket::attach()) to the
//! running Rocket instance. Failure to do so will result in a run-time error.
//!
//! Templates are rendered with the `render` method. The method takes in the
//! name of a template and a context to render the template with. The context
//! can be any type that implements [`Serialize`] from [`serde`] and would
//! serialize to an `Object` value.
//!
//! In debug mode (without the `--release` flag passed to `rocket`), templates
//! will be automatically reloaded from disk if any changes have been made to
//! the templates directory since the previous request. In release builds,
//! template reloading is disabled to improve performance and cannot be enabled.
//!
//! [`Serialize`]: serde::Serialize

#[cfg(feature = "tera_templates")] pub extern crate tera;
#[cfg(feature = "tera_templates")] mod tera_templates;

#[cfg(feature = "handlebars_templates")] pub extern crate handlebars;
#[cfg(feature = "handlebars_templates")] mod handlebars_templates;

mod engine;
mod fairing;
mod context;
mod metadata;

pub use self::engine::Engines;
pub use self::metadata::Metadata;
pub(crate) use self::context::Context;
pub(crate) use self::fairing::ContextManager;

use self::engine::Engine;
use self::fairing::TemplateFairing;

use serde::Serialize;
use serde_json::{Value, to_value};

use std::borrow::Cow;
use std::path::PathBuf;
use std::error::Error;

use rocket::{Rocket, Orbit};
use rocket::request::Request;
use rocket::fairing::Fairing;
use rocket::response::{self, Content, Responder};
use rocket::http::{ContentType, Status};

const DEFAULT_TEMPLATE_DIR: &str = "templates";

/// Responder that renders a dynamic template.
///
/// # Usage
///
/// To use, add the `handlebars_templates` feature, the `tera_templates`
/// feature, or both, to the `rocket_contrib` dependencies section of your
/// `Cargo.toml`:
///
/// ```toml
/// [dependencies.rocket_contrib]
/// version = "0.5.0-dev"
/// default-features = false
/// features = ["handlebars_templates", "tera_templates"]
/// ```
///
/// Then, ensure that the template [`Fairing`] is attached to your Rocket
/// application:
///
/// ```rust
/// # extern crate rocket;
/// # extern crate rocket_contrib;
/// use rocket_contrib::templates::Template;
///
/// fn main() {
///     rocket::build()
///         .attach(Template::fairing())
///         // ...
///     # ;
/// }
/// ```
///
/// The `Template` type implements Rocket's [`Responder`] trait, so it can be
/// returned from a request handler directly:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[macro_use] extern crate rocket_contrib;
/// # fn context() {  }
/// use rocket_contrib::templates::Template;
///
/// #[get("/")]
/// fn index() -> Template {
///     let context = context();
///     Template::render("index", &context)
/// }
/// ```
///
/// # Helpers, Filters, and Customization
///
/// You may use the [`Template::custom()`] method to construct a fairing with
/// customized templating engines. Among other things, this method allows you to
/// register template helpers and register templates from strings.
#[derive(Debug)]
pub struct Template {
    name: Cow<'static, str>,
    value: Option<Value>
}

#[derive(Debug)]
pub(crate) struct TemplateInfo {
    /// The complete path, including `template_dir`, to this template.
    path: PathBuf,
    /// The extension for the engine of this template.
    extension: String,
    /// The extension before the engine extension in the template, if any.
    data_type: ContentType
}

impl Template {
    /// Returns a fairing that initializes and maintains templating state.
    ///
    /// This fairing, or the one returned by [`Template::custom()`], _must_ be
    /// attached to any `Rocket` instance that wishes to render templates.
    /// Failure to attach this fairing will result in a "Uninitialized template
    /// context: missing fairing." error message when a template is attempted to
    /// be rendered.
    ///
    /// If you wish to customize the internal templating engines, use
    /// [`Template::custom()`] instead.
    ///
    /// # Example
    ///
    /// To attach this fairing, simple call `attach` on the application's
    /// `Rocket` instance with `Template::fairing()`:
    ///
    /// ```rust
    /// extern crate rocket;
    /// extern crate rocket_contrib;
    ///
    /// use rocket_contrib::templates::Template;
    ///
    /// fn main() {
    ///     rocket::build()
    ///         // ...
    ///         .attach(Template::fairing())
    ///         // ...
    ///     # ;
    /// }
    /// ```
    pub fn fairing() -> impl Fairing {
        Template::custom(|_| {})
    }

    /// Returns a fairing that initializes and maintains templating state.
    ///
    /// Unlike [`Template::fairing()`], this method allows you to configure
    /// templating engines via the function `f`. Note that only the enabled
    /// templating engines will be accessible from the `Engines` type.
    ///
    /// This method does not allow the function `f` to fail. If `f` is fallible,
    /// use [`Template::try_custom()`] instead.
    ///
    /// # Example
    ///
    /// ```rust
    /// extern crate rocket;
    /// extern crate rocket_contrib;
    ///
    /// use rocket_contrib::templates::Template;
    ///
    /// fn main() {
    ///     rocket::build()
    ///         // ...
    ///         .attach(Template::custom(|engines| {
    ///             // engines.handlebars.register_helper ...
    ///         }))
    ///         // ...
    ///     # ;
    /// }
    /// ```
    pub fn custom<F: Send + Sync + 'static>(f: F) -> impl Fairing
        where F: Fn(&mut Engines)
    {
        Self::try_custom(move |engines| { f(engines); Ok(()) })
    }

    /// Returns a fairing that initializes and maintains templating state.
    ///
    /// This variant of [`Template::custom()`] allows a fallible `f`. If `f`
    /// returns an error during initialization, it will cancel the launch. If
    /// `f` returns an error during template reloading (in debug mode), then the
    /// newly-reloaded templates are discarded.
    ///
    /// # Example
    ///
    /// ```rust
    /// extern crate rocket;
    /// extern crate rocket_contrib;
    ///
    /// use rocket_contrib::templates::Template;
    ///
    /// fn main() {
    ///     rocket::build()
    ///         // ...
    ///         .attach(Template::try_custom(|engines| {
    ///             // engines.handlebars.register_helper ...
    ///             Ok(())
    ///         }))
    ///         // ...
    ///     # ;
    /// }
    /// ```
    pub fn try_custom<F: Send + Sync + 'static>(f: F) -> impl Fairing
        where F: Fn(&mut Engines) -> Result<(), Box<dyn Error>>
    {
        TemplateFairing { callback: Box::new(f) }
    }

    /// Render the template named `name` with the context `context`. The
    /// `context` can be of any type that implements `Serialize`. This is
    /// typically a `HashMap` or a custom `struct`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::collections::HashMap;
    /// use rocket_contrib::templates::Template;
    ///
    /// // Create a `context`. Here, just an empty `HashMap`.
    /// let mut context = HashMap::new();
    ///
    /// # context.insert("test", "test");
    /// # #[allow(unused_variables)]
    /// let template = Template::render("index", context);
    /// ```
    #[inline]
    pub fn render<S, C>(name: S, context: C) -> Template
        where S: Into<Cow<'static, str>>, C: Serialize
    {
        Template { name: name.into(), value: to_value(context).ok() }
    }

    /// Render the template named `name` with the context `context` into a
    /// `String`. This method should **not** be used in any running Rocket
    /// application. This method should only be used during testing to validate
    /// `Template` responses. For other uses, use [`render()`](#method.render)
    /// instead.
    ///
    /// The `context` can be of any type that implements `Serialize`. This is
    /// typically a `HashMap` or a custom `struct`.
    ///
    /// Returns `Some` if the template could be rendered. Otherwise, returns
    /// `None`. If rendering fails, error output is printed to the console.
    /// `None` is also returned if a `Template` fairing has not been attached.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # extern crate rocket;
    /// # extern crate rocket_contrib;
    /// use std::collections::HashMap;
    ///
    /// use rocket_contrib::templates::Template;
    /// use rocket::local::blocking::Client;
    ///
    /// fn main() {
    ///     let rocket = rocket::build().attach(Template::fairing());
    ///     let client = Client::untracked(rocket).expect("valid rocket");
    ///
    ///     // Create a `context`. Here, just an empty `HashMap`.
    ///     let mut context = HashMap::new();
    ///
    ///     # context.insert("test", "test");
    ///     # #[allow(unused_variables)]
    ///     let template = Template::show(client.rocket(), "index", context);
    /// }
    /// ```
    #[inline]
    pub fn show<S, C>(rocket: &Rocket<Orbit>, name: S, context: C) -> Option<String>
        where S: Into<Cow<'static, str>>, C: Serialize
    {
        let ctxt = rocket.state::<ContextManager>().map(ContextManager::context).or_else(|| {
            warn_span!("missing_fairing", "Uninitialized template context: missing fairing.").in_scope(|| {
                info!("To use templates, you must attach `Template::fairing()`.");
                info!("See the `Template` documentation for more information.");
            });
            None
        })?;

        Template::render(name, context).finalize(&ctxt).ok().map(|v| v.0)
    }

    /// Actually render this template given a template context. This method is
    /// called by the `Template` `Responder` implementation as well as
    /// `Template::show()`.
    #[inline(always)]
    fn finalize(self, ctxt: &Context) -> Result<(String, ContentType), Status> {
        let name = &*self.name;
        let info = ctxt.templates.get(name).ok_or_else(|| {
            let ts: Vec<_> = ctxt.templates.keys().map(|s| s.as_str()).collect();
            error_span!("Template does not exist.", template = %name).in_scope(|| {
                info!("Known templates: {}", ts.join(","));
                info!("Searched in {:?}.", ctxt.root);
            });
            Status::InternalServerError
        })?;

        let value = self.value.ok_or_else(|| {
            error!("The provided template context failed to serialize.");
            Status::InternalServerError
        })?;

        let string = ctxt.engines.render(name, &info, value).ok_or_else(|| {
            error!(template = %name, "Template failed to render.");
            Status::InternalServerError
        })?;

        Ok((string, info.data_type.clone()))
    }
}

/// Returns a response with the Content-Type derived from the template's
/// extension and a fixed-size body containing the rendered template. If
/// rendering fails, an `Err` of `Status::InternalServerError` is returned.
impl<'r> Responder<'r, 'static> for Template {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let (render, content_type) = {
            let ctxt = req.rocket().state::<ContextManager>().ok_or_else(|| {
                error_span!("missing_fairing", "Uninitialized template context: missing fairing.").in_scope(|| {
                    info!("To use templates, you must attach `Template::fairing()`.");
                    info!("See the `Template` documentation for more information.");
                });
                Status::InternalServerError
            })?.context();

            self.finalize(&ctxt)?
        };

        Content(content_type, render).respond_to(req)
    }
}
