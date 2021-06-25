#![recursion_limit="128"]

#![doc(html_root_url = "https://api.rocket.rs/v0.5-rc")]
#![doc(html_favicon_url = "https://rocket.rs/images/favicon.ico")]
#![doc(html_logo_url = "https://rocket.rs/images/logo-boxed.png")]

#![warn(rust_2018_idioms, missing_docs)]

//! # Rocket - Code Generation
//!
//! This crate implements the code generation portions of Rocket. This includes
//! custom derives, custom attributes, and procedural macros. The documentation
//! here is purely technical. The code generation facilities are documented
//! thoroughly in the [Rocket programming guide](https://rocket.rs/v0.5-rc/guide).
//!
//! # Usage
//!
//! You **_should not_** directly depend on this library. To use the macros,
//! attributes, and derives in this crate, it suffices to depend on `rocket` in
//! `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! rocket = "0.5.0-rc.1"
//! ```
//!
//! And to import all macros, attributes, and derives via `#[macro_use]` in the
//! crate root:
//!
//! ```rust
//! #[macro_use] extern crate rocket;
//! # #[get("/")] fn hello() { }
//! # fn main() { rocket::build().mount("/", routes![hello]); }
//! ```
//!
//! Or, alternatively, selectively import from the top-level scope:
//!
//! ```rust
//! # extern crate rocket;
//!
//! use rocket::{get, routes};
//! # #[get("/")] fn hello() { }
//! # fn main() { rocket::build().mount("/", routes![hello]); }
//! ```
//!
//! # Debugging Codegen
//!
//! When the `ROCKET_CODEGEN_DEBUG` environment variable is set, this crate
//! logs, at compile-time and to the console, the items it generates. For
//! example, you might run the following to build a Rocket application with
//! codegen debug logging enabled:
//!
//! ```sh
//! ROCKET_CODEGEN_DEBUG=1 cargo build
//! ```

#[macro_use] extern crate quote;

use rocket_http as http;

#[macro_use]
mod exports;
mod proc_macro_ext;
mod derive;
mod attribute;
mod bang;
mod http_codegen;
mod syn_ext;
mod name;

use crate::{http::Method, http_codegen::WebSocketEvent};
use proc_macro::TokenStream;

static URI_MACRO_PREFIX: &str = "rocket_uri_macro_";
static ROCKET_IDENT_PREFIX: &str = "__rocket_";

macro_rules! emit {
    ($tokens:expr) => ({
        use devise::ext::SpanDiagnosticExt;

        let mut tokens = $tokens;
        if std::env::var_os("ROCKET_CODEGEN_DEBUG").is_some() {
            let debug_tokens = proc_macro2::Span::call_site()
                .note("emitting Rocket code generation debug output")
                .note(tokens.to_string())
                .emit_as_item_tokens();

            tokens.extend(debug_tokens);
        }

        tokens.into()
    })
}

macro_rules! route_attribute {
    ($name:ident => $method:expr $(, $t1:ty, $t2:ty)?) => (
        /// Attribute to generate a [`Route`] and associated metadata.
        ///
        /// This and all other route attributes can only be applied to free
        /// functions:
        ///
        /// ```rust
        /// # #[macro_use] extern crate rocket;
        /// #
        /// #[get("/")]
        /// fn index() -> &'static str {
        ///     "Hello, world!"
        /// }
        /// ```
        ///
        /// There are 7 method-specific route attributes:
        ///
        ///   * [`get`] - `GET` specific route
        ///   * [`put`] - `PUT` specific route
        ///   * [`post`] - `POST` specific route
        ///   * [`delete`] - `DELETE` specific route
        ///   * [`head`] - `HEAD` specific route
        ///   * [`options`] - `OPTIONS` specific route
        ///   * [`patch`] - `PATCH` specific route
        ///
        /// Additionally, [`route`] allows the method and uri to be explicitly
        /// specified:
        ///
        /// ```rust
        /// # #[macro_use] extern crate rocket;
        /// #
        /// #[route(GET, uri = "/")]
        /// fn index() -> &'static str {
        ///     "Hello, world!"
        /// }
        /// ```
        ///
        /// [`get`]: attr.get.html
        /// [`put`]: attr.put.html
        /// [`post`]: attr.post.html
        /// [`delete`]: attr.delete.html
        /// [`head`]: attr.head.html
        /// [`options`]: attr.options.html
        /// [`patch`]: attr.patch.html
        /// [`route`]: attr.route.html
        ///
        /// # Grammar
        ///
        /// The grammar for all method-specific route attributes is defined as:
        ///
        /// ```text
        /// route := '"' uri ('?' query)? '"' (',' parameter)*
        ///
        /// uri := ('/' segment)*
        ///
        /// query := segment ('&' segment)*
        ///
        /// segment := URI_SEG
        ///          | SINGLE_PARAM
        ///          | TRAILING_PARAM
        ///
        /// parameter := 'rank' '=' INTEGER
        ///            | 'format' '=' '"' MEDIA_TYPE '"'
        ///            | 'data' '=' '"' SINGLE_PARAM '"'
        ///
        /// SINGLE_PARAM := '<' IDENT '>'
        /// TRAILING_PARAM := '<' IDENT '..>'
        ///
        /// URI_SEG := valid, non-percent-encoded HTTP URI segment
        /// MEDIA_TYPE := valid HTTP media type or known shorthand
        ///
        /// INTEGER := unsigned integer, as defined by Rust
        /// IDENT := valid identifier, as defined by Rust
        /// ```
        ///
        /// The generic route attribute is defined as:
        ///
        /// ```text
        /// generic-route := METHOD ',' 'uri' '=' route
        /// ```
        ///
        /// # Typing Requirements
        ///
        /// Every identifier, except for `_`, that appears in a dynamic
        /// parameter (`SINGLE_PARAM` or `TRAILING_PARAM`) must appear as an
        /// argument to the function. For example, the following route requires
        /// the decorated function to have the arguments `foo`, `baz`, `msg`,
        /// `rest`, and `form`:
        ///
        /// ```rust
        /// # #[macro_use] extern crate rocket;
        /// # use rocket::form::Form;
        /// # use std::path::PathBuf;
        /// # #[derive(FromForm)] struct F { a: usize }
        /// #[get("/<foo>/bar/<baz..>?<msg>&closed&<rest..>", data = "<form>")]
        /// # fn f(foo: usize, baz: PathBuf, msg: String, rest: F, form: Form<F>) {  }
        /// ```
        ///
        /// The type of each function argument corresponding to a dynamic
        /// parameter is required to implement one of Rocket's guard traits. The
        /// exact trait that is required to be implemented depends on the kind
        /// of dynamic parameter (`SINGLE` or `TRAILING`) and where in the route
        /// attribute the parameter appears. The table below summarizes trait
        /// requirements:
        ///
        /// | position | kind        | trait             |
        /// |----------|-------------|-------------------|
        /// | path     | `<ident>`   | [`FromParam`]     |
        /// | path     | `<ident..>` | [`FromSegments`]  |
        /// | query    | `<ident>`   | [`FromForm`]      |
        /// | query    | `<ident..>` | [`FromForm`]      |
        /// | data     | `<ident>`   | [`FromData`]      |
        ///
        /// The type of each function argument that _does not_ have a
        /// corresponding dynamic parameter is required to implement the
        /// [`FromRequest`] trait.
        ///
        /// A route argument declared a `_` must _not_ appear in the function
        /// argument list and has no typing requirements.
        ///
        /// The return type of the decorated function must implement the
        /// [`Responder`] trait.
        ///
        /// [`FromParam`]: ../rocket/request/trait.FromParam.html
        /// [`FromSegments`]: ../rocket/request/trait.FromSegments.html
        /// [`FromFormField`]: ../rocket/request/trait.FromFormField.html
        /// [`FromForm`]: ../rocket/form/trait.FromForm.html
        /// [`FromData`]: ../rocket/data/trait.FromData.html
        /// [`FromRequest`]: ../rocket/request/trait.FromRequest.html
        /// [`Route`]: ../rocket/struct.Route.html
        /// [`Responder`]: ../rocket/response/trait.Responder.html
        ///
        /// # Semantics
        ///
        /// The attribute generates three items:
        ///
        ///   1. A route [`Handler`].
        ///
        ///      The generated handler validates and generates all arguments for
        ///      the generated function according to the trait that their type
        ///      must implement. The order in which arguments are processed is:
        ///
        ///         1. Request guards from left to right.
        ///
        ///            If a request guard fails, the request is forwarded if the
        ///            [`Outcome`] is `Forward` or failed if the [`Outcome`] is
        ///            `Failure`. See [`FromRequest` Outcomes] for further
        ///            detail.
        ///
        ///         2. Path and query guards in an unspecified order. If a path
        ///            or query guard fails, the request is forwarded.
        ///
        ///         3. Data guard, if any.
        ///
        ///            If a data guard fails, the request is forwarded if the
        ///            [`Outcome`] is `Forward` or failed if the [`Outcome`] is
        ///            `Failure`. See [`FromData`] for further detail.
        ///
        ///      If all validation succeeds, the decorated function is called.
        ///      The returned value is used to generate a [`Response`] via the
        ///      type's [`Responder`] implementation.
        ///
        ///   2. A static structure used by [`routes!`] to generate a [`Route`].
        ///
        ///      The static structure (and resulting [`Route`]) is populated
        ///      with the name (the function's name), path, query, rank, and
        ///      format from the route attribute. The handler is set to the
        ///      generated handler.
        ///
        ///   3. A macro used by [`uri!`] to type-check and generate an
        ///      [`Origin`].
        ///
        /// [`Handler`]: ../rocket/route/trait.Handler.html
        /// [`routes!`]: macro.routes.html
        /// [`uri!`]: macro.uri.html
        /// [`Origin`]: ../rocket/http/uri/struct.Origin.html
        /// [`Outcome`]: ../rocket/outcome/enum.Outcome.html
        /// [`Response`]: ../rocket/struct.Response.html
        /// [`FromRequest` Outcomes]: ../rocket/request/trait.FromRequest.html#outcomes
        #[proc_macro_attribute]
        pub fn $name(args: TokenStream, input: TokenStream) -> TokenStream {
            emit!(attribute::route::route_attribute$(::<$t1, $t2>)?($method, args, input))
        }
    )
}

route_attribute!(route => None, Option<WebSocketEvent>, WebSocketEvent);
route_attribute!(get => Method::Get);
route_attribute!(put => Method::Put);
route_attribute!(post => Method::Post);
route_attribute!(delete => Method::Delete);
route_attribute!(head => Method::Head);
route_attribute!(patch => Method::Patch);
route_attribute!(options => Method::Options);

route_attribute!(join => WebSocketEvent::WebSocketJoin);
route_attribute!(message => WebSocketEvent::WebSocketMessage);
route_attribute!(leave => WebSocketEvent::WebSocketLeave);

/// Attribute to generate a [`Catcher`] and associated metadata.
///
/// This attribute can only be applied to free functions:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// use rocket::Request;
/// use rocket::http::Status;
///
/// #[catch(404)]
/// fn not_found(req: &Request) -> String {
///     format!("Sorry, {} does not exist.", req.uri())
/// }
///
/// #[catch(default)]
/// fn default(status: Status, req: &Request) -> String {
///     format!("{} ({})", status, req.uri())
/// }
/// ```
///
/// # Grammar
///
/// The grammar for the `#[catch]` attributes is defined as:
///
/// ```text
/// catch := STATUS | 'default'
///
/// STATUS := valid HTTP status code (integer in [200, 599])
/// ```
///
/// # Typing Requirements
///
/// The decorated function may take zero, one, or two arguments. It's type
/// signature must be one of the following, where `R:`[`Responder`]:
///
///   * `fn() -> R`
///   * `fn(`[`&Request`]`) -> R`
///   * `fn(`[`Status`]`, `[`&Request`]`) -> R`
///
/// # Semantics
///
/// The attribute generates two items:
///
///   1. An error [`Handler`].
///
///      The generated handler calls the decorated function, passing in the
///      [`Status`] and [`&Request`] values if requested. The returned value is
///      used to generate a [`Response`] via the type's [`Responder`]
///      implementation.
///
///   2. A static structure used by [`catchers!`] to generate a [`Catcher`].
///
///      The static structure (and resulting [`Catcher`]) is populated with the
///      name (the function's name) and status code from the route attribute or
///      `None` if `default`. The handler is set to the generated handler.
///
/// [`&Request`]: ../rocket/struct.Request.html
/// [`Status`]: ../rocket/http/struct.Status.html
/// [`Handler`]: ../rocket/catcher/trait.Handler.html
/// [`catchers!`]: macro.catchers.html
/// [`Catcher`]: ../rocket/struct.Catcher.html
/// [`Response`]: ../rocket/struct.Response.html
/// [`Responder`]: ../rocket/response/trait.Responder.html
#[proc_macro_attribute]
pub fn catch(args: TokenStream, input: TokenStream) -> TokenStream {
    emit!(attribute::catch::catch_attribute(args, input))
}

/// Retrofits supports for `async fn` in unit tests.
///
/// Simply decorate a test `async fn` with `#[async_test]` instead of `#[test]`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[cfg(test)]
/// mod tests {
///     #[async_test]
///     async fn test() {
///         /* .. */
///     }
/// }
/// ```
///
/// The attribute rewrites the function to execute inside of a Rocket-compatible
/// async runtime.
#[proc_macro_attribute]
pub fn async_test(args: TokenStream, input: TokenStream) -> TokenStream {
    emit!(attribute::entry::async_test_attribute(args, input))
}

/// Retrofits `async fn` support in `main` functions.
///
/// A `main` `async fn` function decorated with `#[rocket::main]` is transformed
/// into a regular `main` function that internally initializes a Rocket-specific
/// tokio runtime and runs the attributed `async fn` inside of it:
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
/// It should be used only when inspection of an ignited instance of `Rocket` is
/// required, or when the return value of `launch()` is to be inspected:
///
/// ```rust,no_run
/// #[rocket::main]
/// async fn main() -> Result<(), rocket::Error> {
///     let rocket = rocket::build().ignite().await?;
///     println!("Hello, Rocket: {:?}", rocket);
///
///     let result = rocket.launch().await;
///     println!("The server shutdown: {:?}", result);
///
///     result
/// }
/// ```
///
/// For all other cases, use [`#[launch]`](launch) instead.
///
/// The function attributed with `#[rocket::main]` _must_ be `async` and _must_
/// be called `main`. Violation of either results in a compile-time error.
#[proc_macro_attribute]
pub fn main(args: TokenStream, input: TokenStream) -> TokenStream {
    emit!(attribute::entry::main_attribute(args, input))
}

/// Generates a `main` function that launches a returned `Rocket<Build>`.
///
/// When applied to a function that returns a `Rocket<Build>` instance,
/// `#[launch]` automatically initializes an `async` runtime and
/// launches the function's returned instance:
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
/// This generates code equivalent to the following:
///
/// ```rust,no_run
/// # use rocket::{Rocket, Build};
/// # fn rocket() -> Rocket<Build> {
/// #     rocket::build()
/// # }
/// #
/// #[rocket::main]
/// async fn main() {
///     // Recall that an uninspected `Error` will cause a pretty-printed panic,
///     // so rest assured failures do not go undetected when using `#[launch]`.
///     let _ = rocket().launch().await;
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
///
/// The attributed function may be `async`:
///
/// ```rust,no_run
/// # use rocket::launch;
/// # async fn some_async_work() {}
/// #[launch]
/// async fn rocket() -> _ {
///     some_async_work().await;
///     rocket::build()
/// }
/// ```
#[proc_macro_attribute]
pub fn launch(args: TokenStream, input: TokenStream) -> TokenStream {
    emit!(attribute::entry::launch_attribute(args, input))
}

/// Derive for the [`FromFormField`] trait.
///
/// The [`FromFormField`] derive can be applied to enums with nullary
/// (zero-length) fields:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// #[derive(FromFormField)]
/// enum MyValue {
///     First,
///     Second,
///     Third,
/// }
/// ```
///
/// The derive generates an implementation of the [`FromFormField`] trait for
/// the decorated `enum`. The implementation returns successfully when the form
/// value matches, case insensitively, the stringified version of a variant's
/// name, returning an instance of said variant. If there is no match, an error
/// recording all of the available options is returned.
///
/// As an example, for the `enum` above, the form values `"first"`, `"FIRST"`,
/// `"fiRSt"`, and so on would parse as `MyValue::First`, while `"second"` and
/// `"third"` (in any casing) would parse as `MyValue::Second` and
/// `MyValue::Third`, respectively.
///
/// The `field` field attribute can be used to change the string value that is
/// compared against for a given variant:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// #[derive(FromFormField)]
/// enum MyValue {
///     First,
///     Second,
///     #[field(value = "fourth")]
///     #[field(value = "fifth")]
///     Third,
/// }
/// ```
///
/// When more than one `value` is specified, matching _any_ value will result in
/// parsing the decorated variant. Declaring any two values that are
/// case-insensitively equal to any other value or variant name is a
/// compile-time error.
///
/// The `#[field]` attribute's grammar is:
///
/// ```text
/// field := 'value' '=' STRING_LIT
///
/// STRING_LIT := any valid string literal, as defined by Rust
/// ```
///
/// The attribute accepts a single string parameter of name `value`
/// corresponding to the string to use to match against for the decorated
/// variant. In the example above, the the strings `"fourth"`, `"FOUrth"`,
/// `"fiFTH"` and so on would parse as `MyValue::Third`.
///
/// [`FromFormField`]: ../rocket/form/trait.FromFormField.html
#[proc_macro_derive(FromFormField, attributes(field))]
pub fn derive_from_form_field(input: TokenStream) -> TokenStream {
    emit!(derive::from_form_field::derive_from_form_field(input))
}

/// Derive for the [`FromForm`] trait.
///
/// The [`FromForm`] derive can be applied to structures with named fields:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[derive(FromForm)]
/// struct MyStruct<'r> {
///     field: usize,
///     #[field(name = "renamed_field")]
///     #[field(name = uncased("RenamedField"))]
///     other: &'r str,
///     #[field(validate = range(1..), default = 3)]
///     r#type: usize,
///     #[field(default = None)]
///     is_nice: bool,
/// }
/// ```
///
/// Each field's type is required to implement [`FromForm`].
///
/// The derive generates an implementation of the [`FromForm`] trait. The
/// implementation parses a form whose field names match the field names of the
/// structure on which the derive was applied. Each field's value is parsed with
/// the [`FromForm`] implementation of the field's type. The `FromForm`
/// implementation succeeds only when all fields parse successfully or return a
/// default. Errors are collected into a [`form::Errors`] and returned if
/// non-empty after parsing all fields.
///
/// The derive accepts one field attribute: `field`, with the following syntax:
///
/// ```text
/// field := name? default? validate*
///
/// name := 'name' '=' name_val ','?
/// name_val :=  '"' FIELD_NAME '"'
///          | 'uncased(' '"' FIELD_NAME '"' ')
///
/// default := 'default' '=' EXPR ','?
///          | 'default_with' '=' EXPR ','?
///
/// validate := 'validate' '=' EXPR ','?
///
/// FIELD_NAME := valid field name, according to the HTML5 spec
/// EXPR := valid expression, as defined by Rust
/// ```
///
/// The attribute can be applied any number of times on a field as long as at
/// most _one_ of `default` or `default_with` is present per field:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[derive(FromForm)]
/// struct MyStruct {
///     #[field(name = uncased("number"))]
///     #[field(default = 42)]
///     field: usize,
///     #[field(name = "renamed_field")]
///     #[field(name = uncased("anotherName"))]
///     #[field(validate = eq("banana"))]
///     #[field(validate = neq("orange"))]
///     other: String
/// }
/// ```
///
/// **`name`**
///
/// A `name` attribute changes the name to match against when parsing the form
/// field. The value is either an exact string to match against (`"foo"`), or
/// `uncased("foo")`, which causes the match to be case-insensitive but
/// case-preserving. When more than one `name` attribute is applied, the field
/// will match against _any_ of the names.
///
/// **`validate = expr`**
///
/// The validation `expr` is run if the field type parses successfully. The
/// expression must return a value of type `Result<(), form::Errors>`. On `Err`,
/// the errors are added to the thus-far collected errors. If more than one
/// `validate` attribute is applied, _all_ validations are run.
///
/// **`default = expr`**
///
/// If `expr` is not literally `None`, the parameter sets the default value of
/// the field to be `expr.into()`. If `expr` _is_ `None`, the parameter _unsets_
/// the default value of the field, if any. The expression is only evaluated if
/// the attributed field is missing in the incoming form.
///
/// Except when `expr` is `None`, `expr` must be of type `T: Into<F>` where `F`
/// is the field's type.
///
/// **`default_with = expr`**
///
/// The parameter sets the default value of the field to be exactly `expr` which
/// must be of type `Option<F>` where `F` is the field's type. If the expression
/// evaluates to `None`, there is no default. Otherwise the value wrapped in
/// `Some` is used. The expression is only evaluated if the attributed field is
/// missing in the incoming form.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// use std::num::NonZeroUsize;
///
/// #[derive(FromForm)]
/// struct MyForm {
///     // `NonZeroUsize::new()` return an `Option<NonZeroUsize>`.
///     #[field(default_with = NonZeroUsize::new(42))]
///     num: NonZeroUsize,
/// }
/// ```
///
/// [`FromForm`]: rocket::form::FromForm
/// [`form::Errors`]: rocket::form::Errors
#[proc_macro_derive(FromForm, attributes(field))]
pub fn derive_from_form(input: TokenStream) -> TokenStream {
    emit!(derive::from_form::derive_from_form(input))
}

/// Derive for the [`Responder`] trait.
///
/// The [`Responder`] derive can be applied to enums and structs with named
/// fields. When applied to enums, variants must have at least one field. When
/// applied to structs, the struct must have at least one field.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use std::fs::File;
/// # use rocket::http::ContentType;
/// # type OtherResponder = MyResponderA;
/// #
/// #[derive(Responder)]
/// enum MyResponderA {
///     A(String),
///     B(File, ContentType),
/// }
///
/// #[derive(Responder)]
/// struct MyResponderB {
///     inner: OtherResponder,
///     header: ContentType,
/// }
/// ```
///
/// # Semantics
///
/// The derive generates an implementation of the [`Responder`] trait for the
/// decorated enum or structure. The derive uses the _first_ field of a variant
/// or structure to generate a [`Response`]. As such, the type of the first
/// field must implement [`Responder`]. The remaining fields of a variant or
/// structure are set as headers in the produced [`Response`] using
/// [`Response::set_header()`]. As such, every other field (unless explicitly
/// ignored, explained next) must implement `Into<Header>`.
///
/// Except for the first field, fields decorated with `#[response(ignore)]` are
/// ignored by the derive:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use std::fs::File;
/// # use rocket::http::ContentType;
/// # use rocket::fs::NamedFile;
/// # type Other = usize;
/// #
/// #[derive(Responder)]
/// enum MyResponder {
///     A(String),
///     B(File, ContentType, #[response(ignore)] Other),
/// }
///
/// #[derive(Responder)]
/// struct MyOtherResponder {
///     inner: NamedFile,
///     header: ContentType,
///     #[response(ignore)]
///     other: Other,
/// }
/// ```
///
/// Decorating the first field with `#[response(ignore)]` has no effect.
///
/// # Field Attribute
///
/// Additionally, the `response` attribute can be used on named structures and
/// enum variants to override the status and/or content-type of the [`Response`]
/// produced by the generated implementation. The `response` attribute used in
/// these positions has the following grammar:
///
/// ```text
/// response := parameter (',' parameter)?
///
/// parameter := 'status' '=' STATUS
///            | 'content_type' '=' CONTENT_TYPE
///
/// STATUS := unsigned integer >= 100 and < 600
/// CONTENT_TYPE := string literal, as defined by Rust, identifying a valid
///                 Content-Type, as defined by Rocket
/// ```
///
/// It can be used as follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use rocket::http::ContentType;
/// # use rocket::fs::NamedFile;
/// # type Other = usize;
/// # type InnerResponder = String;
/// #
/// #[derive(Responder)]
/// enum Error {
///     #[response(status = 500, content_type = "json")]
///     A(String),
///     #[response(status = 404)]
///     B(NamedFile, ContentType),
/// }
///
/// #[derive(Responder)]
/// #[response(status = 400)]
/// struct MyResponder {
///     inner: InnerResponder,
///     header: ContentType,
///     #[response(ignore)]
///     other: Other,
/// }
/// ```
///
/// The attribute accepts two key/value pairs: `status` and `content_type`. The
/// value of `status` must be an unsigned integer representing a valid status
/// code. The [`Response`] produced from the generated implementation will have
/// its status overridden to this value.
///
/// The value of `content_type` must be a valid media-type in `top/sub` form or
/// `shorthand` form. Examples include:
///
///   * `"text/html"`
///   * `"application/x-custom"`
///   * `"html"`
///   * `"json"`
///   * `"plain"`
///   * `"binary"`
///
/// See [`ContentType::parse_flexible()`] for a full list of available
/// shorthands. The [`Response`] produced from the generated implementation will
/// have its content-type overridden to this value.
///
/// [`Responder`]: ../rocket/response/trait.Responder.html
/// [`Response`]: ../rocket/struct.Response.html
/// [`Response::set_header()`]: ../rocket/response/struct.Response.html#method.set_header
/// [`ContentType::parse_flexible()`]: ../rocket/http/struct.ContentType.html#method.parse_flexible
///
/// # Generics
///
/// The derive accepts any number of type generics and at most one lifetime
/// generic. If a type generic is present, the generated implementation will
/// require a bound of `Responder<'r, 'o>` for each generic unless a
/// `#[response(bound = ...)]` attribute as used:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// use rocket::serde::Serialize;
/// use rocket::serde::json::Json;
/// use rocket::http::ContentType;
/// use rocket::response::Responder;
///
/// // The bound `T: Responder` will be added to the generated implementation.
/// #[derive(Responder)]
/// #[response(status = 404, content_type = "html")]
/// struct NotFoundHtml<T>(T);
///
/// // The bound `T: Serialize` will be added to the generated implementation.
/// // This would fail to compile otherwise.
/// #[derive(Responder)]
/// #[response(bound = "T: Serialize", status = 404)]
/// struct NotFoundJson<T>(Json<T>);
///
/// // The bounds `T: Serialize, E: Responder` will be added to the generated
/// // implementation. This would fail to compile otherwise.
/// #[derive(Responder)]
/// #[response(bound = "T: Serialize, E: Responder<'r, 'o>")]
/// enum MyResult<T, E> {
///     Ok(Json<T>),
///     #[response(status = 404)]
///     Err(E, ContentType)
/// }
/// ```
///
/// If a lifetime generic is present, it will be replace with `'o` in the
/// generated implementation `impl Responder<'r, 'o>`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// // Generates `impl<'r, 'o> Responder<'r, 'o> for NotFoundHtmlString<'o>`.
/// #[derive(Responder)]
/// #[response(status = 404, content_type = "html")]
/// struct NotFoundHtmlString<'a>(&'a str);
/// ```
///
/// Both type generics and lifetime generic may be used:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # use rocket::response::Responder;
/// #[derive(Responder)]
/// struct SomeResult<'o, T>(Result<T, &'o str>);
/// ```
#[proc_macro_derive(Responder, attributes(response))]
pub fn derive_responder(input: TokenStream) -> TokenStream {
    emit!(derive::responder::derive_responder(input))
}

/// Derive for the [`UriDisplay<Query>`] trait.
///
/// The [`UriDisplay<Query>`] derive can be applied to enums and structs. When
/// applied to an enum, the enum must have at least one variant. When applied to
/// a struct, the struct must have at least one field.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[derive(UriDisplayQuery)]
/// enum Kind {
///     A(String),
///     B(usize),
/// }
///
/// #[derive(UriDisplayQuery)]
/// struct MyStruct {
///     name: String,
///     id: usize,
///     kind: Kind,
/// }
/// ```
///
/// Each field's type is required to implement [`UriDisplay<Query>`].
///
/// The derive generates an implementation of the [`UriDisplay<Query>`] trait.
/// The implementation calls [`Formatter::write_named_value()`] for every named
/// field, using the field's name (unless overridden, explained next) as the
/// `name` parameter, and [`Formatter::write_value()`] for every unnamed field
/// in the order the fields are declared.
///
/// The derive accepts one field attribute: `field`, with the following syntax:
///
/// ```text
/// field := 'name' '=' '"' FIELD_NAME '"'
///        | 'value' '=' '"' FIELD_VALUE '"'
///
/// FIELD_NAME := valid HTTP field name
/// FIELD_VALUE := valid HTTP field value
/// ```
///
/// When applied to a struct, the attribute can only contain `name` and looks
/// as follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[derive(UriDisplayQuery)]
/// # struct Kind(String);
/// #[derive(UriDisplayQuery)]
/// struct MyStruct {
///     name: String,
///     id: usize,
///     #[field(name = "type")]
///     #[field(name = "kind")]
///     kind: Kind,
/// }
/// ```
///
/// The field attribute directs that a different field name be used when calling
/// [`Formatter::write_named_value()`] for the given field. The value of the
/// `name` attribute is used instead of the structure's actual field name. If
/// more than one `field` attribute is applied to a field, the _first_ name is
/// used. In the example above, the field `MyStruct::kind` is rendered with a
/// name of `type`.
///
/// The attribute can slso be applied to variants of C-like enums; it may only
/// contain `value` and looks as follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[derive(UriDisplayQuery)]
/// enum Kind {
///     File,
///     #[field(value = "str")]
///     #[field(value = "string")]
///     String,
///     Other
/// }
/// ```
///
/// The field attribute directs that a different value be used when calling
/// [`Formatter::write_named_value()`] for the given variant. The value of the
/// `value` attribute is used instead of the variant's actual name. If more than
/// one `field` attribute is applied to a variant, the _first_ value is used. In
/// the example above, the variant `Kind::String` will render with a value of
/// `str`.
///
/// [`UriDisplay<Query>`]: ../rocket/http/uri/fmt/trait.UriDisplay.html
/// [`Formatter::write_named_value()`]: ../rocket/http/uri/fmt/struct.Formatter.html#method.write_named_value
/// [`Formatter::write_value()`]: ../rocket/http/uri/fmt/struct.Formatter.html#method.write_value
#[proc_macro_derive(UriDisplayQuery, attributes(field))]
pub fn derive_uri_display_query(input: TokenStream) -> TokenStream {
    emit!(derive::uri_display::derive_uri_display_query(input))
}

/// Derive for the [`UriDisplay<Path>`] trait.
///
/// The [`UriDisplay<Path>`] derive can only be applied to tuple structs with
/// one field.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[derive(UriDisplayPath)]
/// struct Name(String);
///
/// #[derive(UriDisplayPath)]
/// struct Age(usize);
/// ```
///
/// The field's type is required to implement [`UriDisplay<Path>`].
///
/// The derive generates an implementation of the [`UriDisplay<Path>`] trait.
/// The implementation calls [`Formatter::write_value()`] for the field.
///
/// [`UriDisplay<Path>`]: ../rocket/http/uri/fmt/trait.UriDisplay.html
/// [`Formatter::write_value()`]: ../rocket/http/uri/fmt/struct.Formatter.html#method.write_value
#[proc_macro_derive(UriDisplayPath)]
pub fn derive_uri_display_path(input: TokenStream) -> TokenStream {
    emit!(derive::uri_display::derive_uri_display_path(input))
}

/// Generates a [`Vec`] of [`Route`]s from a set of route paths.
///
/// The `routes!` macro expands a list of route paths into a [`Vec`] of their
/// corresponding [`Route`] structures. For example, given the following routes:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// #[get("/")]
/// fn index() { /* .. */ }
///
/// mod person {
///     #[post("/hi/<person>")]
///     pub fn hello(person: String) { /* .. */ }
/// }
/// ```
///
/// The `routes!` macro can be used as:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// # use rocket::http::Method;
/// #
/// # #[get("/")] fn index() { /* .. */ }
/// # mod person {
/// #   #[post("/hi/<person>")] pub fn hello(person: String) { /* .. */ }
/// # }
/// let my_routes = routes![index, person::hello];
/// assert_eq!(my_routes.len(), 2);
///
/// let index_route = &my_routes[0];
/// assert_eq!(index_route.method, Method::Get);
/// assert_eq!(index_route.name.as_ref().unwrap(), "index");
/// assert_eq!(index_route.uri.path(), "/");
///
/// let hello_route = &my_routes[1];
/// assert_eq!(hello_route.method, Method::Post);
/// assert_eq!(hello_route.name.as_ref().unwrap(), "hello");
/// assert_eq!(hello_route.uri.path(), "/hi/<person>");
/// ```
///
/// The grammar for `routes!` is defined as:
///
/// ```text
/// routes := PATH (',' PATH)*
///
/// PATH := a path, as defined by Rust
/// ```
///
/// [`Route`]: ../rocket/struct.Route.html
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    emit!(bang::routes_macro(input))
}

/// Generates a [`Vec`] of [`Catcher`]s from a set of catcher paths.
///
/// The `catchers!` macro expands a list of catcher paths into a [`Vec`] of
/// their corresponding [`Catcher`] structures. For example, given the following
/// catchers:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// #[catch(404)]
/// fn not_found() { /* .. */ }
///
/// mod inner {
///     #[catch(400)]
///     pub fn unauthorized() { /* .. */ }
/// }
///
/// #[catch(default)]
/// fn default_catcher() { /* .. */ }
/// ```
///
/// The `catchers!` macro can be used as:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #
/// # #[catch(404)] fn not_found() { /* .. */ }
/// # #[catch(default)] fn default_catcher() { /* .. */ }
/// # mod inner {
/// #     #[catch(400)] pub fn unauthorized() { /* .. */ }
/// # }
/// let my_catchers = catchers![not_found, inner::unauthorized, default_catcher];
/// assert_eq!(my_catchers.len(), 3);
///
/// let not_found = &my_catchers[0];
/// assert_eq!(not_found.code, Some(404));
///
/// let unauthorized = &my_catchers[1];
/// assert_eq!(unauthorized.code, Some(400));
///
/// let default = &my_catchers[2];
/// assert_eq!(default.code, None);
/// ```
///
/// The grammar for `catchers!` is defined as:
///
/// ```text
/// catchers := PATH (',' PATH)*
///
/// PATH := a path, as defined by Rust
/// ```
///
/// [`Catcher`]: ../rocket/struct.Catcher.html
#[proc_macro]
pub fn catchers(input: TokenStream) -> TokenStream {
    emit!(bang::catchers_macro(input))
}

/// Type-safe, encoding-safe route and non-route URI generation.
///
/// The `uri!` macro creates type-safe, URL-safe URIs given a route and concrete
/// parameters for its URI or a URI string literal.
///
/// # String Literal Parsing
///
/// Given a string literal as input, `uri!` parses the string using
/// [`Uri::parse_any()`] and emits a `'static`, `const` value whose type is one
/// of [`Asterisk`], [`Origin`], [`Authority`], [`Absolute`], or [`Reference`],
/// reflecting the parsed value. If the type allows normalization, the value is
/// normalized before being emitted. Parse errors are caught and emitted at
/// compile-time.
///
/// The grammar for this variant of `uri!` is:
///
/// ```text
/// uri := STRING
///
/// STRING := an uncooked string literal, as defined by Rust (example: `"/hi"`)
/// ```
///
/// `STRING` is expected to be an undecoded URI of any variant.
///
/// ## Examples
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// use rocket::http::uri::Absolute;
///
/// // Values returned from `uri!` are `const` and `'static`.
/// const ROOT_CONST: Absolute<'static> = uri!("https://rocket.rs");
/// static ROOT_STATIC: Absolute<'static> = uri!("https://rocket.rs?root");
///
/// // Any variant can be parsed, but beware of ambiguities.
/// let asterisk = uri!("*");
/// let origin = uri!("/foo/bar/baz");
/// let authority = uri!("rocket.rs:443");
/// let absolute = uri!("https://rocket.rs:443");
/// let reference = uri!("foo?bar#baz");
///
/// # use rocket::http::uri::{Asterisk, Origin, Authority, Reference};
/// # // Ensure we get the types we expect.
/// # let asterisk: Asterisk = asterisk;
/// # let origin: Origin<'static> = origin;
/// # let authority: Authority<'static> = authority;
/// # let absolute: Absolute<'static> = absolute;
/// # let reference: Reference<'static> = reference;
/// ```
///
/// # Type-Safe Route URIs
///
/// A URI to a route name `foo` is generated using `uri!(foo(v1, v2, v3))` or
/// `uri!(foo(a = v1, b = v2, c = v3))`, where `v1`, `v2`, `v3` are the values
/// to fill in for route parameters named `a`, `b`, and `c`. If the named
/// parameter sytnax is used (`a = v1`, etc.), parameters can appear in any
/// order.
///
/// More concretely, for the route `person` defined below:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[get("/person/<name>?<age>")]
/// fn person(name: &str, age: Option<u8>) { }
/// ```
///
/// ...a URI can be created as follows:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[get("/person/<name>?<age>")]
/// # fn person(name: &str, age: Option<u8>) { }
/// // with unnamed parameters, in route path declaration order
/// let mike = uri!(person("Mike Smith", Some(28)));
/// assert_eq!(mike.to_string(), "/person/Mike%20Smith?age=28");
///
/// // with named parameters, order irrelevant
/// let mike = uri!(person(name = "Mike", age = Some(28)));
/// let mike = uri!(person(age = Some(28), name = "Mike"));
/// assert_eq!(mike.to_string(), "/person/Mike?age=28");
///
/// // with unnamed values, explicitly `None`.
/// let mike = uri!(person("Mike", None::<u8>));
/// assert_eq!(mike.to_string(), "/person/Mike");
///
/// // with named values, explicitly `None`
/// let option: Option<u8> = None;
/// let mike = uri!(person(name = "Mike", age = None::<u8>));
/// assert_eq!(mike.to_string(), "/person/Mike");
/// ```
///
/// For optional query parameters, those of type `Option` or `Result`, a `_` can
/// be used in-place of `None` or `Err`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// # #[get("/person/<name>?<age>")]
/// # fn person(name: &str, age: Option<u8>) { }
/// // with named values ignored
/// let mike = uri!(person(name = "Mike", age = _));
/// assert_eq!(mike.to_string(), "/person/Mike");
///
/// // with named values ignored
/// let mike = uri!(person(age = _, name = "Mike"));
/// assert_eq!(mike.to_string(), "/person/Mike");
///
/// // with unnamed values ignored
/// let mike = uri!(person("Mike", _));
/// assert_eq!(mike.to_string(), "/person/Mike");
/// ```
///
/// It is a type error to attempt to ignore query parameters that are neither
/// `Option` or `Result`. Path parameters can never be ignored. A path parameter
/// of type `Option<T>` or `Result<T, E>` must be filled by a value that can
/// target a type of `T`:
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[get("/person/<name>")]
/// fn maybe(name: Option<&str>) { }
///
/// let bob1 = uri!(maybe(name = "Bob"));
/// let bob2 = uri!(maybe("Bob Smith"));
/// assert_eq!(bob1.to_string(), "/person/Bob");
/// assert_eq!(bob2.to_string(), "/person/Bob%20Smith");
///
/// #[get("/person/<age>")]
/// fn ok(age: Result<u8, &str>) { }
///
/// let kid1 = uri!(ok(age = 10));
/// let kid2 = uri!(ok(12));
/// assert_eq!(kid1.to_string(), "/person/10");
/// assert_eq!(kid2.to_string(), "/person/12");
/// ```
///
/// Values for ignored route segments can be of any type as long as the type
/// implements [`UriDisplay`] for the appropriate URI part. If a route URI
/// contains ignored segments, the route URI invocation cannot use named
/// arguments.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[get("/ignore/<_>/<other>")]
/// fn ignore(other: &str) { }
///
/// let bob = uri!(ignore("Bob Hope", "hello"));
/// let life = uri!(ignore(42, "cat&dog"));
/// assert_eq!(bob.to_string(), "/ignore/Bob%20Hope/hello");
/// assert_eq!(life.to_string(), "/ignore/42/cat%26dog");
/// ```
///
/// ## Prefixes and Suffixes
///
/// A route URI can be be optionally prefixed and/or suffixed by a URI generated
/// from a string literal or an arbitrary expression. This takes the form
/// `uri!(prefix, foo(v1, v2, v3), suffix)`, where both `prefix` and `suffix`
/// are optional, and either `prefix` or `suffix` may be `_` to specify the
/// value as empty.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// #[get("/person/<name>?<age>")]
/// fn person(name: &str, age: Option<u8>) { }
///
/// // with a specific mount-point of `/api`.
/// let bob = uri!("/api", person("Bob", Some(28)));
/// assert_eq!(bob.to_string(), "/api/person/Bob?age=28");
///
/// // with an absolute URI as a prefix
/// let bob = uri!("https://rocket.rs", person("Bob", Some(28)));
/// assert_eq!(bob.to_string(), "https://rocket.rs/person/Bob?age=28");
///
/// // with another absolute URI as a prefix
/// let bob = uri!("https://rocket.rs/foo", person("Bob", Some(28)));
/// assert_eq!(bob.to_string(), "https://rocket.rs/foo/person/Bob?age=28");
///
/// // with an expression as a prefix
/// let host = uri!("http://bob.me");
/// let bob = uri!(host, person("Bob", Some(28)));
/// assert_eq!(bob.to_string(), "http://bob.me/person/Bob?age=28");
///
/// // with a suffix but no prefix
/// let bob = uri!(_, person("Bob", Some(28)), "#baz");
/// assert_eq!(bob.to_string(), "/person/Bob?age=28#baz");
///
/// // with both a prefix and suffix
/// let bob = uri!("https://rocket.rs/", person("Bob", Some(28)), "#woo");
/// assert_eq!(bob.to_string(), "https://rocket.rs/person/Bob?age=28#woo");
///
/// // with an expression suffix. if the route URI already has a query, the
/// // query part is ignored. otherwise it is added.
/// let suffix = uri!("?woo#bam");
/// let bob = uri!(_, person("Bob", Some(28)), suffix.clone());
/// assert_eq!(bob.to_string(), "/person/Bob?age=28#bam");
///
/// let bob = uri!(_, person("Bob", None::<u8>), suffix.clone());
/// assert_eq!(bob.to_string(), "/person/Bob?woo#bam");
/// ```
///
/// ## Grammar
///
/// The grammar for this variant of the `uri!` macro is:
///
/// ```text
/// uri := (prefix ',')? route
///      | prefix ',' route ',' suffix
///
/// prefix := STRING | expr                     ; `Origin` or `Absolute`
/// suffix := STRING | expr                     ; `Reference` or `Absolute`
///
/// route := PATH '(' (named | unnamed) ')'
///
/// named := IDENT = expr (',' named)? ','?
/// unnamed := expr (',' unnamed)? ','?
///
/// expr := EXPR | '_'
///
/// EXPR := a valid Rust expression (examples: `foo()`, `12`, `"hey"`)
/// IDENT := a valid Rust identifier (examples: `name`, `age`)
/// STRING := an uncooked string literal, as defined by Rust (example: `"hi"`)
/// PATH := a path, as defined by Rust (examples: `route`, `my_mod::route`)
/// ```
///
/// ## Dynamic Semantics
///
/// The returned value is that of the prefix (minus any query part) concatenated
/// with the route URI concatenated with the query (if the route has no query
/// part) and fragment parts of the suffix. The route URI is generated by
/// interpolating the declared route URI with the URL-safe version of the route
/// values in `uri!()`. The generated URI is guaranteed to be URI-safe.
///
/// Each route value is rendered in its appropriate place in the URI using the
/// [`UriDisplay`] implementation for the value's type. The `UriDisplay`
/// implementation ensures that the rendered value is URL-safe.
///
/// A `uri!()` invocation allocated at-most once.
///
/// ## Static Semantics
///
/// The `uri!` macro returns one of [`Origin`], [`Absolute`], or [`Reference`],
/// depending on the types of the prefix and suffix, if any. The table below
/// specifies all combinations:
///
/// | Prefix     | Suffix      | Output      |
/// |------------|-------------|-------------|
/// | None       | None        | `Origin`    |
/// | None       | `Absolute`  | `Origin`    |
/// | None       | `Reference` | `Reference` |
/// | `Origin`   | None        | `Origin`    |
/// | `Origin`   | `Absolute`  | `Origin`    |
/// | `Origin`   | `Reference` | `Reference` |
/// | `Absolute` | None        | `Absolute`  |
/// | `Absolute` | `Absolute`  | `Absolute`  |
/// | `Absolute` | `Reference` | `Reference` |
///
/// A `uri!` invocation only typechecks if the type of every route URI value in
/// the invocation matches the type declared for the parameter in the given
/// route, after conversion with [`FromUriParam`], or if a value is ignored
/// using `_` and the corresponding route type implements [`Ignorable`].
///
/// ### Conversion
///
/// The [`FromUriParam`] trait is used to typecheck and perform a conversion for
/// each value passed to `uri!`. If a `FromUriParam<P, S> for T` implementation
/// exists for a type `T` for part URI part `P`, then a value of type `S` can be
/// used in `uri!` macro for a route URI parameter declared with a type of `T`
/// in part `P`. For example, the following implementation, provided by Rocket,
/// allows an `&str` to be used in a `uri!` invocation for route URI parameters
/// declared as `String`:
///
/// ```rust,ignore
/// impl<P: Part, 'a> FromUriParam<P, &'a str> for String { .. }
/// ```
///
/// ### Ignorables
///
/// Query parameters can be ignored using `_` in place of an expression. The
/// corresponding type in the route URI must implement [`Ignorable`]. Ignored
/// parameters are not interpolated into the resulting `Origin`. Path parameters
/// are not ignorable.
///
/// [`Uri`]: ../rocket/http/uri/enum.Uri.html
/// [`Origin`]: ../rocket/http/uri/struct.Origin.html
/// [`Asterisk`]: ../rocket/http/uri/struct.Asterisk.html
/// [`Authority`]: ../rocket/http/uri/struct.Authority.html
/// [`Absolute`]: ../rocket/http/uri/struct.Absolute.html
/// [`Reference`]: ../rocket/http/uri/struct.Reference.html
/// [`FromUriParam`]: ../rocket/http/uri/fmt/trait.FromUriParam.html
/// [`UriDisplay`]: ../rocket/http/uri/fmt/trait.UriDisplay.html
/// [`Ignorable`]: ../rocket/http/uri/fmt/trait.Ignorable.html
#[proc_macro]
pub fn uri(input: TokenStream) -> TokenStream {
    emit!(bang::uri_macro(input))
}

/// Internal macro: `rocket_internal_uri!`.
#[proc_macro]
#[doc(hidden)]
pub fn rocket_internal_uri(input: TokenStream) -> TokenStream {
    emit!(bang::uri_internal_macro(input))
}

/// Internal macro: `__typed_stream!`.
#[proc_macro]
#[doc(hidden)]
pub fn __typed_stream(input: TokenStream) -> TokenStream {
    emit!(bang::typed_stream(input))
}

/// Private Rocket internal macro: `internal_guide_tests!`.
#[proc_macro]
#[doc(hidden)]
pub fn internal_guide_tests(input: TokenStream) -> TokenStream {
    emit!(bang::guide_tests_internal(input))
}

/// Private Rocket internal macro: `export!`.
#[proc_macro]
#[doc(hidden)]
pub fn export(input: TokenStream) -> TokenStream {
    emit!(bang::export_internal(input))
}
