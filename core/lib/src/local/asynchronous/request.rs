use std::fmt;
use std::convert::TryInto;

use crate::{Request, Data};
use crate::http::{Status, Method};
use crate::http::uri::Origin;

use super::{Client, LocalResponse, super::form::LocalForm};

/// An `async` local request as returned by [`Client`](super::Client).
///
/// For details, see [the top-level documentation](../index.html#localrequest).
///
/// ## Example
///
/// The following snippet uses the available builder methods to construct and
/// dispatch a `POST` request to `/` with a JSON body:
///
/// ```rust,no_run
/// use rocket::local::asynchronous::{Client, LocalRequest};
/// use rocket::http::{ContentType, Cookie};
///
/// # rocket::async_test(async {
/// let client = Client::tracked(rocket::build()).await.expect("valid rocket");
/// let req = client.post("/")
///     .header(ContentType::JSON)
///     .remote("127.0.0.1:8000".parse().unwrap())
///     .cookie(Cookie::new("name", "value"))
///     .body(r#"{ "value": 42 }"#);
///
/// let response = req.dispatch().await;
/// # });
/// ```
pub struct LocalRequest<'c> {
    pub(in super) client: &'c Client,
    pub(in super) request: Request<'c>,
    data: Vec<u8>,
    // The `Origin` on the right is INVALID! It should _not_ be used!
    uri: Result<Origin<'c>, Origin<'static>>,
}

impl<'c> LocalRequest<'c> {
    pub(crate) fn new<'u: 'c, U>(client: &'c Client, method: Method, uri: U) -> Self
        where U: TryInto<Origin<'u>> + fmt::Display
    {
        // Try to parse `uri` into an `Origin`, storing whether it's good.
        let uri_str = uri.to_string();
        let try_origin = uri.try_into().map_err(|_| Origin::path_only(uri_str));

        // Create a request. We'll handle bad URIs later, in `_dispatch`.
        let origin = try_origin.clone().unwrap_or_else(|bad| bad);
        let mut request = Request::new(client.rocket(), method, origin);

        // Add any cookies we know about.
        if client.tracked {
            client._with_raw_cookies(|jar| {
                for cookie in jar.iter() {
                    request.cookies_mut().add_original(cookie.clone());
                }
            })
        }

        LocalRequest { client, request, uri: try_origin, data: vec![] }
    }

    pub(crate) fn _request(&self) -> &Request<'c> {
        &self.request
    }

    pub(crate) fn _request_mut(&mut self) -> &mut Request<'c> {
        &mut self.request
    }

    pub(crate) fn _body_mut(&mut self) -> &mut Vec<u8> {
        &mut self.data
    }

    // Performs the actual dispatch.
    async fn _dispatch(mut self) -> LocalResponse<'c> {
        // First, revalidate the URI, returning an error response (generated
        // from an error catcher) immediately if it's invalid. If it's valid,
        // then `request` already contains a correct URI.
        let rocket = self.client.rocket();
        if let Err(ref invalid) = self.uri {
            // The user may have changed the URI in the request in which case we
            // _shouldn't_ error. Check that now and error only if not.
            if self.inner().uri() == invalid {
                error!("invalid request URI: {:?}", invalid.path());
                return LocalResponse::new(self.request, move |req| {
                    rocket.handle_error(Status::BadRequest, req)
                }).await
            }
        }

        // Actually dispatch the request.
        let mut data = Data::local(self.data);
        let token = rocket.preprocess_request(&mut self.request, &mut data).await;
        let response = LocalResponse::new(self.request, move |req| {
            rocket.dispatch(token, req, data)
        }).await;

        // If the client is tracking cookies, updates the internal cookie jar
        // with the changes reflected by `response`.
        if self.client.tracked {
            self.client._with_raw_cookies_mut(|jar| {
                let current_time = time::OffsetDateTime::now_utc();
                for cookie in response.cookies().iter() {
                    if let Some(expires) = cookie.expires_datetime() {
                        if expires <= current_time {
                            jar.force_remove(cookie);
                            continue;
                        }
                    }

                    jar.add_original(cookie.clone());
                }
            })
        }

        response
    }

    pub(crate) fn _form(mut self, form: LocalForm<'c>) -> LocalRequest<'c> {
        self.data = form.body_data();
        self.request.replace_header(form.content_type());
        self
    }

    pub_request_impl!("# use rocket::local::asynchronous::Client;\n\
        use rocket::local::asynchronous::LocalRequest;" async await);
}

impl<'c> Clone for LocalRequest<'c> {
    fn clone(&self) -> Self {
        LocalRequest {
            client: self.client,
            request: self.request.clone(),
            data: self.data.clone(),
            uri: self.uri.clone(),
        }
    }
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
