use std::io;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures::stream::StreamExt;
use futures::future::{self, FutureExt, Future, TryFutureExt, BoxFuture};
use tokio::sync::oneshot;
use tracing::instrument::{Instrument, Instrumented};
use yansi::Paint;

use crate::{Rocket, Orbit, Request, Data, route};
use crate::form::Form;
use crate::response::{Response, Body};
use crate::outcome::Outcome;
use crate::error::{Error, ErrorKind};
use crate::ext::AsyncReadExt;

use crate::http::{Method, Status, Header, hyper};
use crate::http::private::{Listener, Connection, Incoming};
use crate::http::uri::Origin;
use crate::http::private::bind_tcp;

// A token returned to force the execution of one method before another.
pub(crate) struct RequestToken;

async fn handle<Fut, T, F>(name: Option<&str>, run: F) -> Option<T>
    where F: FnOnce() -> Fut, Fut: Future<Output = T>,
{
    use std::panic::AssertUnwindSafe;

    macro_rules! panic_info {
        ($name:expr, $e:expr) => {{
            let error_span = match $name {
                Some(name) => error_span!("handler_panic", "Handler {} panicked.", Paint::white(name)),
                None => error_span!("handler_panic", "A handler panicked.")
            };

            error_span.in_scope(|| {
                info!("This is an application bug.");
                info!("A panic in Rust must be treated as an exceptional event.");
                info!("Panicking is not a suitable error handling mechanism.");
                info!("Unwinding, the result of a panic, is an expensive operation.");
                info!("Panics will severely degrade application performance.");
                info!("Instead of panicking, return `Option` and/or `Result`.");
                info!("Values of either type can be returned directly from handlers.");
                warn!("A panic is treated as an internal server error.");
                $e
            });
        }}
    }

    let run = AssertUnwindSafe(run);
    let fut = std::panic::catch_unwind(move || run())
        .map_err(|e| panic_info!(name, e))
        .ok()?;

    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .map_err(|e| panic_info!(name, e))
        .ok()
}

// This function tries to hide all of the Hyper-ness from Rocket. It essentially
// converts Hyper types into Rocket types, then calls the `dispatch` function,
// which knows nothing about Hyper. Because responding depends on the
// `HyperResponse` type, this function does the actual response processing.
async fn hyper_service_fn(
    rocket: Arc<Rocket<Orbit>>,
    h_addr: std::net::SocketAddr,
    hyp_req: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, io::Error> {
    // This future must return a hyper::Response, but the response body might
    // borrow from the request. Instead, write the body in another future that
    // sends the response metadata (and a body channel) prior.
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        // Get all of the information from Hyper.
        let (h_parts, h_body) = hyp_req.into_parts();

        // Convert the Hyper request into a Rocket request.
        let req_res = Request::from_hyp(
            &rocket, h_parts.method, h_parts.headers, &h_parts.uri, h_addr
        );

        let mut req = match req_res {
            Ok(req) => req,
            Err(e) => {
                error!(error = %e, "Bad incoming request");
                // TODO: We don't have a request to pass in, so we just
                // fabricate one. This is weird. We should let the user know
                // that we failed to parse a request (by invoking some special
                // handler) instead of doing this.
                let dummy = Request::new(&rocket, Method::Get, Origin::dummy());
                let r = rocket.handle_error(Status::BadRequest, &dummy).await;
                return rocket.send_response(r, tx).await;
            }
        };

        let span = info_span!("request", "{}", req);
        async {
            // Retrieve the data from the hyper body.
            let mut data = Data::from(h_body);

            // Dispatch the request to get a response, then write that response out.
            let token = rocket.preprocess_request(&mut req, &mut data).await;
            let r = rocket.dispatch(token, &mut req, data).await;
            rocket.send_response(r, tx).await;
        }
            .instrument(span)
            .await

    }.in_current_span());

    // Receive the response written to `tx` by the task above.
    rx.await.map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}

impl Rocket<Orbit> {
    /// Wrapper around `make_response` to log a success or failure.
    #[inline]
    async fn send_response(
        &self,
        response: Response<'_>,
        tx: oneshot::Sender<hyper::Response<hyper::Body>>,
    ) {
        match self.make_response(response, tx).await {
            Ok(()) => info!("{}", Paint::green("Response succeeded.")),
            Err(error) => error!(%error, "Failed to write response"),
        }
    }

    /// Attempts to create a hyper response from `response` and send it to `tx`.
    #[inline]
    async fn make_response(
        &self,
        mut response: Response<'_>,
        tx: oneshot::Sender<hyper::Response<hyper::Body>>,
    ) -> io::Result<()> {
        let mut hyp_res = hyper::Response::builder()
            .status(response.status().code);

        for header in response.headers().iter() {
            let name = header.name.as_str();
            let value = header.value.as_bytes();
            hyp_res = hyp_res.header(name, value);
        }

        let send_response = move |res: hyper::ResponseBuilder, body| -> io::Result<()> {
            let response = res.body(body)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

            tx.send(response).map_err(|_| {
                let msg = "client disconnected before the response was started";
                io::Error::new(io::ErrorKind::BrokenPipe, msg)
            })
        };

        match response.body_mut() {
            None => {
                hyp_res = hyp_res.header(hyper::header::CONTENT_LENGTH, 0);
                send_response(hyp_res, hyper::Body::empty())?;
            }
            Some(body) => {
                if let Some(s) = body.size().await {
                    hyp_res = hyp_res.header(hyper::header::CONTENT_LENGTH, s);
                }

                let chunk_size = match *body {
                    Body::Chunked(_, chunk_size) => chunk_size as usize,
                    Body::Sized(_, _) => crate::response::DEFAULT_CHUNK_SIZE,
                };

                let (mut sender, hyp_body) = hyper::Body::channel();
                send_response(hyp_res, hyp_body)?;

                let mut stream = body.as_reader().into_bytes_stream(chunk_size);
                while let Some(next) = stream.next().await {
                    sender.send_data(next?).await
                        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                }
            }
        };

        Ok(())
    }

    /// Preprocess the request for Rocket things. Currently, this means:
    ///
    ///   * Rewriting the method in the request if _method form field exists.
    ///   * Run the request fairings.
    ///
    /// Keep this in-sync with derive_form when preprocessing form fields.
    pub(crate) async fn preprocess_request(
        &self,
        req: &mut Request<'_>,
        data: &mut Data
    ) -> RequestToken {
        // Check if this is a form and if the form contains the special _method
        // field which we use to reinterpret the request's method.
        let (min_len, max_len) = ("_method=get".len(), "_method=delete".len());
        let peek_buffer = data.peek(max_len).await;
        let is_form = req.content_type().map_or(false, |ct| ct.is_form());

        if is_form && req.method() == Method::Post && peek_buffer.len() >= min_len {
            let method = std::str::from_utf8(peek_buffer).ok()
                .and_then(|raw_form| Form::values(raw_form).next())
                .filter(|field| field.name == "_method")
                .and_then(|field| field.value.parse().ok());

            if let Some(method) = method {
                req._set_method(method);
            }
        }

        // Run request fairings.
        self.fairings.handle_request(req, data).await;

        RequestToken
    }

    #[inline]
    pub(crate) async fn dispatch<'s, 'r: 's>(
        &'s self,
        _token: RequestToken,
        request: &'r Request<'s>,
        data: Data
    ) -> Response<'r> {
        // Remember if the request is `HEAD` for later body stripping.
        let was_head_request = request.method() == Method::Head;

        // Route the request and run the user's handlers.
        let mut response = self.route_and_process(request, data).await;

        // Add a default 'Server' header if it isn't already there.
        // TODO: If removing Hyper, write out `Date` header too.
        if !response.headers().contains("Server") {
            response.set_header(Header::new("Server", "Rocket"));
        }

        // Run the response fairings.
        self.fairings.handle_response(request, &mut response).await;

        // Strip the body if this is a `HEAD` request.
        if was_head_request {
            response.strip_body();
        }

        response
    }

    async fn route_and_process<'s, 'r: 's>(
        &'s self,
        request: &'r Request<'s>,
        data: Data
    ) -> Response<'r> {
        let mut response = match self.route(request, data).await {
            Outcome::Success(response) => response,
            Outcome::Forward(data) if request.method() == Method::Head => {
                info!("Autohandling {} request.", Paint::default("HEAD").bold());

                // Dispatch the request again with Method `GET`.
                request._set_method(Method::Get);
                match self.route(request, data).await {
                    Outcome::Success(response) => response,
                    Outcome::Failure(status) => self.handle_error(status, request).await,
                    Outcome::Forward(_) => self.handle_error(Status::NotFound, request).await,
                }
            }
            Outcome::Forward(_) => self.handle_error(Status::NotFound, request).await,
            Outcome::Failure(status) => self.handle_error(status, request).await,
        };

        // Set the cookies. Note that error responses will only include cookies
        // set by the error handler. See `handle_error` for more.
        let delta_jar = request.cookies().take_delta_jar();
        for cookie in delta_jar.delta() {
            response.adjoin_header(cookie);
        }

        response
    }

    /// Tries to find a `Responder` for a given `request`. It does this by
    /// routing the request and calling the handler for each matching route
    /// until one of the handlers returns success or failure, or there are no
    /// additional routes to try (forward). The corresponding outcome for each
    /// condition is returned.
    #[inline]
    async fn route<'s, 'r: 's>(
        &'s self,
        request: &'r Request<'s>,
        mut data: Data,
    ) -> route::Outcome<'r> {
        // Go through the list of matching routes until we fail or succeed.
        for route in self.router.route(request) {
            // Retrieve and set the requests parameters.
            info!(matched = %route);
            request.set_route(route);

            let name = route.name.as_deref();
            let outcome = handle(name, || route.handler.handle(request, data)).await
                .unwrap_or_else(|| Outcome::Failure(Status::InternalServerError));

            // Check if the request processing completed (Some) or if the
            // request needs to be forwarded. If it does, continue the loop
            // (None) to try again.
            info!(%outcome);
            match outcome {
                o@Outcome::Success(_) | o@Outcome::Failure(_) => return o,
                Outcome::Forward(unused_data) => data = unused_data,
            }
        }

        error!("No matching routes for {}.", request);
        Outcome::Forward(data)
    }

    /// Invokes the handler with `req` for catcher with status `status`.
    ///
    /// In order of preference, invoked handler is:
    ///   * the user's registered handler for `status`
    ///   * the user's registered `default` handler
    ///   * Rocket's default handler for `status`
    ///
    /// Return `Ok(result)` if the handler succeeded. Returns `Ok(Some(Status))`
    /// if the handler ran to completion but failed. Returns `Ok(None)` if the
    /// handler panicked while executing.
    async fn invoke_catcher<'s, 'r: 's>(
        &'s self,
        status: Status,
        req: &'r Request<'s>
    ) -> Result<Response<'r>, Option<Status>> {
        // For now, we reset the delta state to prevent any modifications
        // from earlier, unsuccessful paths from being reflected in error
        // response. We may wish to relax this in the future.
        req.cookies().reset_delta();

        if let Some(catcher) = self.router.catch(status, req) {
            warn!("Responding with registered {} catcher.", catcher);
            let name = catcher.name.as_deref();
            handle(name, || catcher.handler.handle(status, req)).await
                .map(|result| result.map_err(Some))
                .unwrap_or_else(|| Err(None))
        } else {
            let code = Paint::blue(status.code).bold();
            warn!("No {} catcher registered. Using Rocket default.", code);
            Ok(crate::catcher::default_handler(status, req))
        }
    }

    // Invokes the catcher for `status`. Returns the response on success.
    //
    // On catcher failure, the 500 error catcher is attempted. If _that_ fails,
    // the (infallible) default 500 error cather is used.
    pub(crate) fn handle_error<'s, 'r: 's>(
        &'s self,
        mut status: Status,
        req: &'r Request<'s>
    ) -> impl Future<Output = Response<'r>> + Send + 's {
        async move {
            // Dispatch to the `status` catcher.
            if let Ok(r) = self.invoke_catcher(status, req).await {
                return r;
            }

            // If it fails and it's not a 500, try the 500 catcher.
            if status != Status::InternalServerError {
                error!("Catcher failed. Attemping 500 error catcher.");
                status = Status::InternalServerError;
                if let Ok(r) = self.invoke_catcher(status, req).await {
                    return r;
                }
            }

            // If it failed again or if it was already a 500, use Rocket's default.
            error!("{} catcher failed. Using Rocket default 500.", status.code);
            crate::catcher::default_handler(Status::InternalServerError, req)
        }.instrument(warn_span!("handle_error", "Responding with {} catcher.", Paint::red(&status)))
    }

    pub(crate) async fn default_tcp_http_server<C>(mut self, ready: C) -> Result<(), Error>
        where C: for<'a> Fn(&'a Self) -> BoxFuture<'a, ()>
    {
        use std::net::ToSocketAddrs;

        // Determine the address we're going to serve on.
        let addr = format!("{}:{}", self.config.address, self.config.port);
        let mut addr = addr.to_socket_addrs()
            .map(|mut addrs| addrs.next().expect(">= 1 socket addr"))
            .map_err(|e| Error::new(ErrorKind::Io(e)))?;

        #[cfg(feature = "tls")]
        if let Some(ref config) = self.config.tls {
            use crate::http::private::tls::bind_tls;

            let (certs, key) = config.to_readers().map_err(ErrorKind::Io)?;
            let l = bind_tls(addr, certs, key).await.map_err(ErrorKind::Bind)?;
            addr = l.local_addr().unwrap_or(addr);
            self.config.address = addr.ip();
            self.config.port = addr.port();
            ready(&mut self).await;
            return self.http_server(l).await;
        }

        let l = bind_tcp(addr).await.map_err(ErrorKind::Bind)?;
        addr = l.local_addr().unwrap_or(addr);
        self.config.address = addr.ip();
        self.config.port = addr.port();
        ready(&mut self).await;
        self.http_server(l).await
    }

    // TODO.async: Solidify the Listener APIs and make this function public
    pub(crate) async fn http_server<L>(self, listener: L) -> Result<(), Error>
        where L: Listener + Send + Unpin + 'static,
              <L as Listener>::Connection: Send + Unpin + 'static
    {
        // Determine keep-alives.
        let http1_keepalive = self.config.keep_alive != 0;
        let http2_keep_alive = match self.config.keep_alive {
            0 => None,
            n => Some(std::time::Duration::from_secs(n as u64))
        };

        // Get the shutdown handle (to initiate) and signal (when initiated).
        let shutdown_handle = self.shutdown.clone();
        let shutdown_signal = match self.config.ctrlc {
            true => tokio::signal::ctrl_c().boxed(),
            false => future::pending().boxed(),
        };

        let rocket = Arc::new(self);
        let service = hyper::make_service_fn(move |conn: &<L as Listener>::Connection| {
            let rocket = rocket.clone();
            let remote = conn.remote_addr().unwrap_or_else(|| ([0, 0, 0, 0], 0).into());
            async move {
                let service = hyper::service_fn(move |req| {
                    hyper_service_fn(rocket.clone(), remote, req)
                });
                Ok::<_, std::convert::Infallible>(service)
            }
        });

        #[derive(Clone)]
        struct InCurrentSpanExecutor;

        impl<Fut> hyper::Executor<Fut> for InCurrentSpanExecutor
            where Fut: Future + Send + 'static, Fut::Output: Send
        {
            fn execute(&self, fut: Fut) {
                tokio::spawn(fut.in_current_span());
            }
        }

        let shutdown_receiver = shutdown_handle.clone();
        let server = hyper::Server::builder(Incoming::from_listener(listener))
            .executor(InCurrentSpanExecutor)
            .http1_keepalive(http1_keepalive)
            .http2_keep_alive_interval(http2_keep_alive)
            .serve(service)
            .with_graceful_shutdown(async move { shutdown_receiver.notified().await; })
            .map_err(|e| Error::new(ErrorKind::Runtime(Box::new(e))));

        tokio::pin!(server);

        let selecter = future::select(shutdown_signal, server);
        match selecter.await {
            future::Either::Left((Ok(()), server)) => {
                // Ctrl-was pressed. Signal shutdown, wait for the server.
                shutdown_handle.notify_one();
                server.await
            }
            future::Either::Left((Err(error), server)) => {
                // Error setting up ctrl-c signal. Let the user know.
                warn!(%error, "Failed to enable `ctrl-c` graceful signal shutdown.");
                server.await
            }
            // Server shut down before Ctrl-C; return the result.
            future::Either::Right((result, _)) => result,
        }
    }
}

struct InstrumentedService<S> {
    span: tracing::Span,
    service: S
}

impl<S: hyper::Service<R>, R> hyper::Service<R> for InstrumentedService<S> {
    type Response = S::Response;
    type Future = Instrumented<S::Future>;
    type Error = S::Error;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let _e = self.span.enter();
        self.service.poll_ready(cx)
    }

    fn call(&mut self, request: R) -> Self::Future {
        self.service.call(request).instrument(self.span.clone())
    }
}
