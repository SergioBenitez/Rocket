use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::server::accept::Accept;
use tracing::{debug, error};

use tokio::time::Sleep;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};

// TODO.async: 'Listener' and 'Connection' provide common enough functionality
// that they could be introduced in upstream libraries.
/// A 'Listener' yields incoming connections
pub trait Listener {
    /// The connection type returned by this listener.
    type Connection: Connection;

    /// Return the actual address this listener bound to.
    fn local_addr(&self) -> Option<SocketAddr>;

    /// Try to accept an incoming Connection if ready
    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Self::Connection>>;
}

/// A 'Connection' represents an open connection to a client
pub trait Connection: AsyncRead + AsyncWrite {
    /// The remote address, i.e. the client's socket address.
    fn remote_addr(&self) -> Option<SocketAddr>;
}

pin_project_lite::pin_project! {
    /// This is a generic version of hyper's AddrIncoming that is intended to be
    /// usable with listeners other than a plain TCP stream, e.g. TLS and/or Unix
    /// sockets. It does so by bridging the `Listener` trait to what hyper wants (an
    /// Accept). This type is internal to Rocket.
    #[must_use = "streams do nothing unless polled"]
    pub struct Incoming<L> {
        listener: L,
        sleep_on_errors: Option<Duration>,
        #[pin]
        pending_error_delay: Option<Sleep>,
    }
}

impl<L: Listener> Incoming<L> {
    /// Construct an `Incoming` from an existing `Listener`.
    pub fn from_listener(listener: L) -> Self {
        Self {
            listener,
            sleep_on_errors: Some(Duration::from_millis(250)),
            pending_error_delay: None,
        }
    }

    /// Set whether to sleep on accept errors.
    ///
    /// A possible scenario is that the process has hit the max open files
    /// allowed, and so trying to accept a new connection will fail with
    /// `EMFILE`. In some cases, it's preferable to just wait for some time, if
    /// the application will likely close some files (or connections), and try
    /// to accept the connection again. If this option is `true`, the error
    /// will be logged at the `error` level, since it is still a big deal,
    /// and then the listener will sleep for 1 second.
    ///
    /// In other cases, hitting the max open files should be treat similarly
    /// to being out-of-memory, and simply error (and shutdown). Setting
    /// this option to `None` will allow that.
    ///
    /// Default is 1 second.
    pub fn set_sleep_on_errors(&mut self, val: Option<Duration>) {
        self.sleep_on_errors = val;
    }

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<L::Connection>> {
        let mut me = self.project();
        let mut optimistic_retry = true;
        loop {
            // Check if a previous sleep timer is active that was set by IO errors.
            if let Some(delay) = me.pending_error_delay.as_mut().as_pin_mut() {
                if optimistic_retry {
                    error!("optimistically retrying now");
                    optimistic_retry = false;
                } else {
                    error!("retrying in {:?}", me.sleep_on_errors);
                    match delay.poll(cx) {
                        Poll::Ready(()) => {}
                        Poll::Pending => return Poll::Pending,
                    }
                }
            }

            me.pending_error_delay.set(None);

            match me.listener.poll_accept(cx) {
                Poll::Ready(Ok(stream)) => {
                    return Poll::Ready(Ok(stream));
                },
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => {
                    // Connection errors can be ignored directly, continue by
                    // accepting the next request.
                    if is_connection_error(&e) {
                        debug!("accepted connection already errored: {}", e);
                        continue;
                    }

                    if let Some(duration) = me.sleep_on_errors {
                        error!("connection accept error: {}", e);

                        // Sleep for the specified duration
                        me.pending_error_delay.set(Some(tokio::time::sleep(*duration)));
                    } else {
                        return Poll::Ready(Err(e));
                    }
                },
            }
        }
    }
}

impl<L: Listener + Unpin> Accept for Incoming<L> {
    type Conn = L::Connection;
    type Error = io::Error;

    fn poll_accept(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>
    ) -> Poll<Option<io::Result<Self::Conn>>> {
        self.poll_next(cx).map(Some)
    }
}

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a delay before next `accept()` is performed.
/// The delay is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn is_connection_error(e: &io::Error) -> bool {
    match e.kind() {
        io::ErrorKind::ConnectionRefused |
        io::ErrorKind::ConnectionAborted |
        io::ErrorKind::ConnectionReset => true,
        _ => false,
    }
}

impl<L: fmt::Debug> fmt::Debug for Incoming<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Incoming")
            .field("listener", &self.listener)
            .finish()
    }
}

/// Binds a TCP listener to `address` and returns it.
pub async fn bind_tcp(address: SocketAddr) -> io::Result<TcpListener> {
    Ok(TcpListener::bind(address).await?)
}

impl Listener for TcpListener {
    type Connection = TcpStream;

    fn local_addr(&self) -> Option<SocketAddr> {
        self.local_addr().ok()
    }

    fn poll_accept(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<Self::Connection>> {
        (*self).poll_accept(cx).map_ok(|(stream, _addr)| stream)
    }
}

impl Connection for TcpStream {
    fn remote_addr(&self) -> Option<SocketAddr> {
        self.peer_addr().ok()
    }
}
