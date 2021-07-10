use std::fmt;
use std::convert::TryInto;

use crate::{Request, http::Method, local::asynchronous};
use crate::http::uri::Origin;

use super::{Client, LocalResponse, super::form::LocalForm};

/// A `blocking` local request as returned by [`Client`](super::Client).
///
/// For details, see [the top-level documentation](../index.html#localrequest).
///
/// ## Example
///
/// The following snippet uses the available builder methods to construct and
/// dispatch a `POST` request to `/` with a JSON body:
///
/// ```rust,no_run
/// use rocket::local::blocking::{Client, LocalRequest};
/// use rocket::http::{ContentType, Cookie};
///
/// let client = Client::tracked(rocket::build()).expect("valid rocket");
/// let req = client.post("/")
///     .header(ContentType::JSON)
///     .remote("127.0.0.1:8000".parse().unwrap())
///     .cookie(Cookie::new("name", "value"))
///     .body(r#"{ "value": 42 }"#);
///
/// let response = req.dispatch();
/// ```
#[derive(Clone)]
pub struct LocalRequest<'c> {
    inner: asynchronous::LocalRequest<'c>,
    client: &'c Client,
}

impl<'c> LocalRequest<'c> {
    #[inline]
    pub(crate) fn new<'u: 'c, U>(client: &'c Client, method: Method, uri: U) -> Self
        where U: TryInto<Origin<'u>> + fmt::Display
    {
        let inner = asynchronous::LocalRequest::new(client.inner(), method, uri);
        Self { inner, client }
    }

    #[inline]
    fn _request(&self) -> &Request<'c> {
        self.inner._request()
    }

    #[inline]
    fn _request_mut(&mut self) -> &mut Request<'c> {
        self.inner._request_mut()
    }

    fn _body_mut(&mut self) -> &mut Vec<u8> {
        self.inner._body_mut()
    }

    fn _dispatch(self) -> LocalResponse<'c> {
        let inner = self.client.block_on(self.inner.dispatch());
        LocalResponse { inner, client: self.client }
    }

    fn _form(mut self, form: LocalForm<'c>) -> LocalRequest<'c> {
        self.inner = self.inner._form(form);
        self
    }

    pub_request_impl!("# use rocket::local::blocking::Client;\n\
        use rocket::local::blocking::LocalRequest;");
}

impl std::fmt::Debug for LocalRequest<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self._request().fmt(f)
    }
}

impl<'c> std::ops::Deref for LocalRequest<'c> {
    type Target = Request<'c>;

    fn deref(&self) -> &Self::Target {
        self.inner()
    }
}

impl<'c> std::ops::DerefMut for LocalRequest<'c> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner_mut()
    }
}
