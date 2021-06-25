use std::io;
use std::sync::Arc;
use std::time::Duration;

use channel::WebSocket;
use rocket_http::hyper::upgrade::OnUpgrade;
use yansi::Paint;
use tokio::sync::oneshot;
use futures::stream::StreamExt;
use futures::future::{self, FutureExt, Future, TryFutureExt, BoxFuture};

use crate::websocket::Extensions;
use crate::websocket::WebSocketEvent;
use crate::websocket::channel;
use crate::websocket::channel::WebSocketChannel;
use crate::websocket::status::StatusError;
use crate::websocket::status::WebSocketStatus;
use crate::{Rocket, Orbit, Request, Response, Data, route};
use crate::form::Form;
use crate::outcome::Outcome;
use crate::error::{Error, ErrorKind};
use crate::ext::{AsyncReadExt, CancellableListener, CancellableIo};

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
            match $name {
                Some(name) => error_!("Handler {} panicked.", Paint::white(name)),
                None => error_!("A handler panicked.")
            };

            info_!("This is an application bug.");
            info_!("A panic in Rust must be treated as an exceptional event.");
            info_!("Panicking is not a suitable error handling mechanism.");
            info_!("Unwinding, the result of a panic, is an expensive operation.");
            info_!("Panics will severely degrade application performance.");
            info_!("Instead of panicking, return `Option` and/or `Result`.");
            info_!("Values of either type can be returned directly from handlers.");
            warn_!("A panic is treated as an internal server error.");
            $e
        }}
    }

    let run = AssertUnwindSafe(run);
    let fut = std::panic::catch_unwind(run)
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
    addr: std::net::SocketAddr,
    mut hyp_req: hyper::Request<hyper::Body>,
) -> Result<hyper::Response<hyper::Body>, io::Error> {
    // This future must return a hyper::Response, but the response body might
    // borrow from the request. Instead, write the body in another future that
    // sends the response metadata (and a body channel) prior.
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let upgrade = crate::websocket::upgrade(&mut hyp_req);
        // Convert a Hyper request into a Rocket request.
        let (h_parts, mut h_body) = hyp_req.into_parts();
        let mut req = match Request::from_hyp(&rocket, &h_parts, addr) {
            Ok(req) => req,
            Err(e) => {
                error!("Bad incoming request: {}", e);
                // TODO: We don't have a request to pass in, so we just
                // fabricate one. This is weird. We should let the user know
                // that we failed to parse a request (by invoking some special
                // handler) instead of doing this.
                let dummy = Request::new(&rocket, Method::Get, Origin::ROOT);
                let r = rocket.handle_error(Status::BadRequest, &dummy).await;
                return rocket.send_response(r, tx).await;
            }
        };

        // Retrieve the data from the hyper body.
        let mut data = Data::from(&mut h_body);

        // Dispatch the request to get a response, then write that response out.
        let token = rocket.preprocess_request(&mut req, &mut data).await;
        if let Some(upgrade) = upgrade {
            // req.clone() is nessecary since the request is borrowed to hande the response. This
            // copy can (and will) outlive the actual request, but will not outlive the websocket
            // connection.
            let req_copy = req.clone();
            let (accept, upgrade) = upgrade.split();
            let (r, ext) = rocket.dispatch_ws(token, &mut req, data, accept).await;
            rocket.send_response(r, tx).await;
            rocket.ws_event_loop(req_copy, upgrade, ext).await;
        } else {
            let r = rocket.dispatch(token, &mut req, data).await;
            rocket.send_response(r, tx).await;
        }
    });

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
            Ok(()) => info_!("{}", Paint::green("Response succeeded.")),
            Err(e) => error_!("Failed to write response: {}.", e),
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

        let body = response.body_mut();
        if let Some(n) = body.size().await {
            hyp_res = hyp_res.header(hyper::header::CONTENT_LENGTH, n);
        }

        let max_chunk_size = body.max_chunk_size();
        let (mut sender, hyp_body) = hyper::Body::channel();
        send_response(hyp_res, hyp_body)?;

        let mut stream = body.into_bytes_stream(max_chunk_size);
        while let Some(next) = stream.next().await {
            sender.send_data(next?).await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        }

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
        data: &mut Data<'_>
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
        data: Data<'r>
    ) -> Response<'r> {
        info!("{}:", request);

        // Remember if the request is `HEAD` for later body stripping.
        let was_head_request = request.method() == Method::Head;

        // Route the request and run the user's handlers.
        let mut response = self.route_and_process(request, data).await;

        // Add a default 'Server' header if it isn't already there.
        // TODO: If removing Hyper, write out `Date` header too.
        if let Some(ident) = request.rocket().config.ident.as_str() {
            if !response.headers().contains("Server") {
                response.set_header(Header::new("Server", ident));
            }
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
        data: Data<'r>
    ) -> Response<'r> {
        let mut response = match self.route(request, data).await {
            Outcome::Success(response) => response,
            Outcome::Forward(data) if request.method() == Method::Head => {
                info_!("Autohandling {} request.", Paint::default("HEAD").bold());

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
        mut data: Data<'r>,
    ) -> route::Outcome<'r> {
        // Go through the list of matching routes until we fail or succeed.
        for route in self.router.route(request) {
            // Retrieve and set the requests parameters.
            info_!("Matched: {}", route);
            request.set_route(route);

            let name = route.name.as_deref();
            let outcome = handle(name, || route.handler.handle(request, data)).await
                .unwrap_or_else(|| Outcome::Failure(Status::InternalServerError));

            // Check if the request processing completed (Some) or if the
            // request needs to be forwarded. If it does, continue the loop
            // (None) to try again.
            info_!("{} {}", Paint::default("Outcome:").bold(), outcome);
            match outcome {
                o@Outcome::Success(_) | o@Outcome::Failure(_) => return o,
                Outcome::Forward(unused_data) => data = unused_data,
            }
        }

        error_!("No matching routes for {}.", request);
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
            warn_!("Responding with registered {} catcher.", catcher);
            let name = catcher.name.as_deref();
            handle(name, || catcher.handler.handle(status, req)).await
                .map(|result| result.map_err(Some))
                .unwrap_or_else(|| Err(None))
        } else {
            let code = Paint::blue(status.code).bold();
            warn_!("No {} catcher registered. Using Rocket default.", code);
            Ok(crate::catcher::default_handler(status, req))
        }
    }

    // Invokes the catcher for `status`. Returns the response on success.
    //
    // On catcher failure, the 500 error catcher is attempted. If _that_ fails,
    // the (infallible) default 500 error cather is used.
    pub(crate) async fn handle_error<'s, 'r: 's>(
        &'s self,
        mut status: Status,
        req: &'r Request<'s>
    ) -> Response<'r> {
        // Dispatch to the `status` catcher.
        if let Ok(r) = self.invoke_catcher(status, req).await {
            return r;
        }

        // If it fails and it's not a 500, try the 500 catcher.
        if status != Status::InternalServerError {
            error_!("Catcher failed. Attemping 500 error catcher.");
            status = Status::InternalServerError;
            if let Ok(r) = self.invoke_catcher(status, req).await {
                return r;
            }
        }

        // If it failed again or if it was already a 500, use Rocket's default.
        error_!("{} catcher failed. Using Rocket default 500.", status.code);
        crate::catcher::default_handler(Status::InternalServerError, req)
    }

    /// Dispatch the Websocket response. This does not invoke ANY user handlers.
    ///
    /// Instead, the join handler is allowed to fail the connection after the first message, with a
    /// Websocket Status Code. This should check the router's websocket connections, to return a
    /// 404 if the endpoint doesn't exist.
    #[inline]
    pub(crate) async fn dispatch_ws<'s, 'r: 's>(
        &'s self,
        _token: RequestToken,
        request: &'r Request<'s>,
        _data: Data<'r>,
        accept: String,
    ) -> (Response<'r>, Extensions) {
        info!("{}:", request);

        // remeber the protocol for later
        let extensions = Extensions::new(request);

        let mut response = if !self.router.route_event(WebSocketEvent::Message)
            .any(|r| r.matches(request))
        {
            // If there is no Message handler for the route
            self.handle_error(Status::NotFound, request).await
        } else {
            use rocket_http::hyper::header::{CONNECTION, UPGRADE};
            let mut response = Response::build();
            response.status(Status::SwitchingProtocols);
            response.header(Header::new(CONNECTION.as_str(), "upgrade"));
            response.header(Header::new(UPGRADE.as_str(), "websocket"));
            response.header(Header::new("Sec-WebSocket-Accept", accept));
            if let Some(ident) = request.rocket().config.ident.as_str() {
                response.header(Header::new("Server", ident));
            }

            extensions.headers(&mut response);

            response.finalize()
        };

        // Run the response fairings.
        self.fairings.handle_response(request, &mut response).await;

        (response, extensions)
    }

    /// Routes a websocket event. This is different from an HTTP route in that the event is passed
    /// seperately, but the reqest still holds all the nessecary information
    // TODO: Simplify the lifetime bounds
    async fn route_event<'s: 'ri, 'r, 'ri>(
        &'s self,
        req: &'r WebSocket<'ri>,
        event: WebSocketEvent,
        mut data: Data<'r>
    ) -> route::WsOutcome<'r> {
        for route in self.router.route_event(event).filter(|r| r.matches(req.request())) {
            info_!("Matched: {:#}", route);
            req.request().set_route(route);

            let name = route.name.as_deref();
            let handler = route.websocket_handler.unwrap_ref();
            let outcome = handle(name, || handler.handle(req, data)).await
                .unwrap_or_else(|| Outcome::Failure(WebSocketStatus::InternalServerError));

            // Check if the request processing completed (Some) or if the
            // request needs to be forwarded. If it does, continue the loop
            // (None) to try again.
            info_!("{} {}", Paint::default("Outcome:").bold(), outcome);
            match outcome {
                o@Outcome::Success(_) | o@Outcome::Failure(_) => return o,
                Outcome::Forward(unused_data) => data = unused_data,
            }
        }
        route::WsOutcome::Forward(data)
    }

    async fn ws_event_loop<'r>(
        &'r self,
        req: Request<'r>,
        upgrade: OnUpgrade,
        extensions: Extensions
    ) {
        if let Ok(upgrade) = upgrade.await {
            let (ch, a, b) = WebSocketChannel::new(upgrade, extensions);
            let req = WebSocket::new(req, ch.subscribe_handle());
            let event_loop = async move {
                // Explicit moves
                let mut ch = ch;
                let mut close_status = Err(StatusError::NoStatus);
                let mut joined = false;
                let broker = self.broker();
                while let Some(message) = ch.next().await {
                    let data = match message.opcode() {
                        websocket_codec::Opcode::Text => Data::from_ws(message, Some(false)),
                        websocket_codec::Opcode::Binary => Data::from_ws(message, Some(true)),
                        websocket_codec::Opcode::Close => {
                            if let Some(status) = message.inner().recv().await {
                                close_status = WebSocketStatus::decode(status);
                            }
                            break;
                        },
                        _ => panic!("An unexpected error occured while\
                                    processing websocket messages. {:?}\
                                    has an invalid opcode", message),
                    };
                    let o = if !joined {
                        let o = self.route_event(&req, WebSocketEvent::Join, data).await;
                        let o = match o {
                            // If the join handlers forwarded, we retry as a message
                            Outcome::Forward(data) => {
                                broker.subscribe(req.topic(), &ch).await;
                                self.route_event(&req, WebSocketEvent::Message, data).await
                            },
                            // If a join handler succeeds, we subscribe the client
                            o@Outcome::Success(_) => {
                                broker.subscribe(req.topic(), &ch).await;
                                o
                            },
                            // If a join handler fails, we do nothing
                            o@Outcome::Failure(_) => {
                                o
                            },
                        };
                        joined = true;
                        o
                    } else {
                        //req.set_topic(Origin::parse("/echo/we").unwrap());
                        self.route_event(&req, WebSocketEvent::Message, data).await
                    };
                    match o {
                        Outcome::Forward(_data) => {
                            break;
                        },
                        Outcome::Failure(status) => {
                            error_!("{}", status);
                            ch.close(status).await;
                            break;
                        },
                        Outcome::Success(_response) => {
                            // We ignore this, since the response should be empty
                        },
                    }
                }
                broker.unsubscribe_all(&ch).await;
                info_!("Websocket closed with status: {:?}", close_status);
                // TODO provide close message
                match self.route_event(&req, WebSocketEvent::Leave, Data::local(vec![])).await {
                    Outcome::Forward(_data) => {
                    },
                    Outcome::Failure(status) => {
                        error_!("{}", status);
                        ch.close(status).await;
                    }
                    Outcome::Success(_response) => {
                        // We ignore this, since the response should be empty
                    }
                }
                // Note: If a close has already been sent, the writer task will just drop this
                ch.close(WebSocketStatus::default_response(close_status)).await;
            };
            // This will poll each future, on the same thread. This should actually be more
            // preformant than spawning tasks for each.
            tokio::join!(a, b, event_loop);
        } else {
            todo!("Handle upgrade error")
        }
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
        where L: Listener + Send, <L as Listener>::Connection: Send + Unpin + 'static
    {
        // Determine keep-alives.
        let http1_keepalive = self.config.keep_alive != 0;
        let http2_keep_alive = match self.config.keep_alive {
            0 => None,
            n => Some(Duration::from_secs(n as u64))
        };

        // Set up cancellable I/O from the given listener. Shutdown occurs when
        // `Shutdown` (`TripWire`) resolves. This can occur directly through a
        // notification or indirectly through an external signal which, when
        // received, results in triggering the notify.
        let shutdown = self.shutdown();
        let sig_stream = self.config.shutdown.signal_stream();
        let force_shutdown = self.config.shutdown.force;
        let grace = self.config.shutdown.grace as u64;
        let mercy = self.config.shutdown.mercy as u64;

        let rocket = Arc::new(self);
        let service_fn = move |conn: &CancellableIo<_, L::Connection>| {
            let rocket = rocket.clone();
            let remote = conn.remote_addr().unwrap_or_else(|| ([0, 0, 0, 0], 0).into());
            async move {
                Ok::<_, std::convert::Infallible>(hyper::service_fn(move |req| {
                    hyper_service_fn(rocket.clone(), remote, req)
                }))
            }
        };

        // NOTE: `hyper` uses `tokio::spawn()` as the default executor.
        let listener = CancellableListener::new(shutdown.clone(), listener, grace, mercy);
        let server = hyper::Server::builder(Incoming::new(listener))
            .http1_keepalive(http1_keepalive)
            .http1_preserve_header_case(true)
            .http2_keep_alive_interval(http2_keep_alive)
            .serve(hyper::make_service_fn(service_fn))
            .with_graceful_shutdown(shutdown.clone())
            .map_err(|e| Error::new(ErrorKind::Runtime(Box::new(e))));

        // Start a task that listens for external signals and notifies shutdown.
        if let Some(mut stream) = sig_stream {
            let shutdown = shutdown.clone();
            tokio::spawn(async move {
                while let Some(sig) = stream.next().await {
                    if shutdown.0.tripped() {
                        warn!("Received {}. Shutdown already in progress.", sig);
                    } else {
                        warn!("Received {}. Requesting shutdown.", sig);
                    }

                    shutdown.0.trip();
                }
            });
        }

        // Wait for a shutdown notification or for the server to somehow fail.
        tokio::pin!(server);
        match future::select(shutdown, server).await {
            future::Either::Left((_, server)) => {
                // If a task has some runaway I/O, like an infinite loop, the
                // runtime will block indefinitely when it is dropped. To
                // subvert, we start a ticking process-exit time bomb here.
                if force_shutdown {
                    use std::thread;

                    // Only a worker thread will have the specified thread name.
                    tokio::task::spawn_blocking(move || {
                        let this = thread::current();
                        let is_rocket_runtime = this.name()
                            .map_or(false, |s| s.starts_with("rocket-worker"));

                        // We only hit our `exit()` if the process doesn't
                        // otherwise exit since this `spawn()` won't block.
                        thread::spawn(move || {
                            thread::sleep(Duration::from_secs(grace + mercy));
                            thread::sleep(Duration::from_millis(500));
                            if is_rocket_runtime {
                                error!("Server failed to shutdown cooperatively. Terminating.");
                                std::process::exit(1);
                            } else {
                                warn!("Server failed to shutdown cooperatively.");
                                warn_!("Server is executing inside of a custom runtime.");
                                info_!("Rocket's runtime is `#[rocket::main]` or `#[launch]`.");
                                warn_!("Refusing to terminate runaway custom runtime.");
                            }
                        });
                    });
                }

                info!("Received shutdown request. Waiting for pending I/O...");
                server.await
            }
            future::Either::Right((result, _)) => result,
        }
    }
}
