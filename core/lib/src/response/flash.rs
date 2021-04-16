use time::Duration;
use serde::ser::{Serialize, Serializer, SerializeStruct};

use crate::outcome::IntoOutcome;
use crate::response::{self, Responder};
use crate::request::{self, Request, FromRequest};
use crate::http::{Status, Cookie, CookieJar};
use std::sync::atomic::{AtomicBool, Ordering};

// The name of the actual flash cookie.
const FLASH_COOKIE_NAME: &str = "_flash";

// Character to use as a delimiter after the cookie's name's length.
const FLASH_COOKIE_DELIM: char = ':';

/// Sets a "flash" cookie that will be removed when it is accessed. The
/// analogous request type is [`FlashMessage`].
///
/// This type makes it easy to send messages across requests. It is typically
/// used for "status" messages after redirects. For instance, if a user attempts
/// to visit a page he/she does not have access to, you may want to redirect the
/// user to a safe place and show a message indicating what happened on the
/// redirected page. The message should only persist for a single request. This
/// can be accomplished with this type.
///
/// # Usage
///
/// Each `Flash` message consists of a `kind` and `message`. A generic
/// constructor ([new](#method.new)) can be used to construct a message of any
/// kind, while the [warning](#method.warning), [success](#method.success), and
/// [error](#method.error) constructors create messages with the corresponding
/// kinds.
///
/// Messages can be retrieved on the request side via the [`FlashMessage`] type
/// and the [kind](#method.kind) and [message](#method.message) methods.
///
/// # Response
///
/// The `Responder` implementation for `Flash` sets the message cookie and then
/// uses the passed in responder `res` to complete the response. In other words,
/// it simply sets a cookie and delegates the rest of the response handling to
/// the wrapped responder.
///
/// # Example
///
/// The following routes illustrate the use of a `Flash` message on both the
/// request and response sides.
///
/// ```rust
/// # #[macro_use] extern crate rocket;
/// use rocket::response::{Flash, Redirect};
/// use rocket::request::FlashMessage;
///
/// #[post("/login/<name>")]
/// fn login(name: &str) -> Result<&'static str, Flash<Redirect>> {
///     if name == "special_user" {
///         Ok("Hello, special user!")
///     } else {
///         Err(Flash::error(Redirect::to(uri!(index)), "Invalid username."))
///     }
/// }
///
/// #[get("/")]
/// fn index(flash: Option<FlashMessage<'_>>) -> String {
///     flash.map(|flash| format!("{}: {}", flash.kind(), flash.message()))
///          .unwrap_or_else(|| "Welcome!".to_string())
/// }
/// ```
///
/// On the response side (in `login`), a `Flash` error message is set if some
/// fictional authentication failed, and the user is redirected to `"/"`. On the
/// request side (in `index`), the handler emits the flash message if there is
/// one and otherwise emits a standard welcome message. Note that if the user
/// were to refresh the index page after viewing a flash message, the user would
/// receive the standard welcome message.
#[derive(Debug)]
pub struct Flash<R> {
    kind: String,
    message: String,
    consumed: AtomicBool,
    inner: R,
}

/// Type alias to retrieve [`Flash`] messages from a request.
///
/// # Flash Cookie
///
/// A `FlashMessage` holds the parsed contents of the flash cookie. As long as
/// there is a flash cookie present (set by the `Flash` `Responder`), a
/// `FlashMessage` request guard will succeed.
///
/// The flash cookie is cleared if either the [`kind()`] or [`message()`] method is
/// called. If neither method is called, the flash cookie is not cleared.
///
/// [`kind()`]: Flash::kind()
/// [`message()`]: Flash::message()
pub type FlashMessage<'a> = crate::response::Flash<&'a CookieJar<'a>>;

impl<R> Flash<R> {
    /// Constructs a new `Flash` message with the given `kind`, `message`, and
    /// underlying `responder`.
    ///
    /// # Examples
    ///
    /// Construct a "suggestion" message with contents "Try this out!" that
    /// redirects to "/".
    ///
    /// ```rust
    /// use rocket::response::{Redirect, Flash};
    ///
    /// # #[allow(unused_variables)]
    /// let message = Flash::new(Redirect::to("/"), "suggestion", "Try this out!");
    /// ```
    pub fn new<K: Into<String>, M: Into<String>>(res: R, kind: K, message: M) -> Flash<R> {
        Flash {
            kind: kind.into(),
            message: message.into(),
            consumed: AtomicBool::default(),
            inner: res,
        }
    }

    /// Constructs a "success" `Flash` message with the given `responder` and
    /// `message`.
    ///
    /// # Examples
    ///
    /// Construct a "success" message with contents "It worked!" that redirects
    /// to "/".
    ///
    /// ```rust
    /// use rocket::response::{Redirect, Flash};
    ///
    /// # #[allow(unused_variables)]
    /// let message = Flash::success(Redirect::to("/"), "It worked!");
    /// ```
    pub fn success<S: Into<String>>(responder: R, message: S) -> Flash<R> {
        Flash::new(responder, "success", message.into())
    }

    /// Constructs a "warning" `Flash` message with the given `responder` and
    /// `message`.
    ///
    /// # Examples
    ///
    /// Construct a "warning" message with contents "Watch out!" that redirects
    /// to "/".
    ///
    /// ```rust
    /// use rocket::response::{Redirect, Flash};
    ///
    /// # #[allow(unused_variables)]
    /// let message = Flash::warning(Redirect::to("/"), "Watch out!");
    /// ```
    pub fn warning<S: Into<String>>(responder: R, message: S) -> Flash<R> {
        Flash::new(responder, "warning", message.into())
    }

    /// Constructs an "error" `Flash` message with the given `responder` and
    /// `message`.
    ///
    /// # Examples
    ///
    /// Construct an "error" message with contents "Whoops!" that redirects
    /// to "/".
    ///
    /// ```rust
    /// use rocket::response::{Redirect, Flash};
    ///
    /// # #[allow(unused_variables)]
    /// let message = Flash::error(Redirect::to("/"), "Whoops!");
    /// ```
    pub fn error<S: Into<String>>(responder: R, message: S) -> Flash<R> {
        Flash::new(responder, "error", message.into())
    }

    fn cookie(&self) -> Cookie<'static> {
        let content = format!("{}{}{}{}",
            self.kind.len(), FLASH_COOKIE_DELIM, self.kind, self.message);

        Cookie::build(FLASH_COOKIE_NAME, content)
            .max_age(Duration::minutes(5))
            .finish()
    }
}

/// Sets the message cookie and then uses the wrapped responder to complete the
/// response. In other words, simply sets a cookie and delegates the rest of the
/// response handling to the wrapped responder. As a result, the `Outcome` of
/// the response is the `Outcome` of the wrapped `Responder`.
impl<'r, 'o: 'r, R: Responder<'r, 'o>> Responder<'r, 'o> for Flash<R> {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'o> {
        req.cookies().add(self.cookie());
        self.inner.respond_to(req)
    }
}

impl<'r> FlashMessage<'r> {
    /// Constructs a new message with the given name and message for the given
    /// request.
    fn named<S: Into<String>>(kind: S, message: S, req: &'r Request<'_>) -> Self {
        Flash {
            kind: kind.into(),
            message: message.into(),
            consumed: AtomicBool::new(false),
            inner: req.cookies(),
        }
    }

    // Clears the request cookie if it hasn't already been cleared.
    fn clear_cookie_if_needed(&self) {
        // Remove the cookie if it hasn't already been removed.
        if !self.consumed.swap(true, Ordering::Relaxed) {
            self.inner.remove(Cookie::named(FLASH_COOKIE_NAME));
        }
    }

    /// Returns a tuple of `(kind, message)`, consuming `self`.
    pub fn into_inner(self) -> (String, String) {
        self.clear_cookie_if_needed();
        (self.kind, self.message)
    }

    /// Returns the `kind` of this message.
    pub fn kind(&self) -> &str {
        self.clear_cookie_if_needed();
        &self.kind
    }

    /// Returns the `message` contents of this message.
    pub fn message(&self) -> &str {
        self.clear_cookie_if_needed();
        &self.message
    }
}

/// Retrieves a flash message from a flash cookie. If there is no flash cookie,
/// or if the flash cookie is malformed, an empty `Err` is returned.
///
/// The suggested use is through an `Option` and the `FlashMessage` type alias
/// in `request`: `Option<FlashMessage>`.
#[crate::async_trait]
impl<'r> FromRequest<'r> for FlashMessage<'r> {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        trace!("Flash: attempting to retrieve message.");
        req.cookies().get(FLASH_COOKIE_NAME).ok_or(()).and_then(|cookie| {
            trace!("Flash: retrieving message: {:?}", cookie);

            // Parse the flash message.
            let content = cookie.value();
            let (len_str, kv) = match content.find(FLASH_COOKIE_DELIM) {
                Some(i) => (&content[..i], &content[(i + 1)..]),
                None => return Err(()),
            };

            match len_str.parse::<usize>() {
                Ok(i) if i <= kv.len() => Ok(Flash::named(&kv[..i], &kv[i..], req)),
                _ => Err(())
            }
        }).into_outcome(Status::BadRequest)
    }
}

impl Serialize for FlashMessage<'_> {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        let mut flash = ser.serialize_struct("Flash", 2)?;
        flash.serialize_field("kind", self.kind())?;
        flash.serialize_field("message", self.message())?;
        flash.end()
    }
}
