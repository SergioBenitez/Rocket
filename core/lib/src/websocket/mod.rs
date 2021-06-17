//! WebSocket types
//!
//! # TODO
//!
//! - [ ] Write more documentation
//! - [ ] Finalize Data type
//! - [ ] Organize websocket types

use rocket_http::Header;
use rocket_http::Status;
use rocket_http::hyper::upgrade::OnUpgrade;
use websocket_codec::ClientRequest;

pub(crate) mod channel;
pub(crate) mod message;
pub(crate) mod status;

pub use channel::{WebSocket, Channel};
pub use status::WebSocketStatus;

use crate::Request;
use crate::http::hyper;

/// Soft maximum for chunks reasons
pub const MAX_BUFFER_SIZE: usize = 1024;

/// Identifier of WebSocketEvent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebSocketEvent {
    /// A Join event, triggered when a client connects
    Join,
    /// A Message event, triggered when a client sends a message
    Message,
    /// A Leave event, triggered when a client disconnects
    Leave,
}

impl WebSocketEvent {
    pub(crate) fn from_handler<T>(h: &crate::route::WebSocketEvent<T>) -> Option<Self> {
        match h {
            crate::route::WebSocketEvent::Join(_) => Some(Self::Join),
            crate::route::WebSocketEvent::Message(_) => Some(Self::Message),
            crate::route::WebSocketEvent::Leave(_) => Some(Self::Leave),
            crate::route::WebSocketEvent::None => None,
        }
    }
}

/// Create the upgrade object associated with this request IF the request should be upgraded
pub(crate) fn upgrade(req: &mut hyper::Request<hyper::Body>) -> Option<WebsocketUpgrade> {
    if req.method() == hyper::Method::GET {
        ClientRequest::parse(|n|
            req.headers().get(n).map(|s| s.to_str().unwrap_or(""))
        ).ok().map(|accept| WebsocketUpgrade {
            accept: accept.ws_accept(), on_upgrade: hyper::upgrade::on(req)
        })
    } else {
        None
    }
}

/// The extensions and protocol for a websocket connection
pub(crate) struct Extensions {
    protocol: Protocol,
    extensions: Vec<Extension>,
}

impl Extensions {
    /// Select a protocol and extensions for the connection from a request
    pub fn new(req: &Request<'_>) -> Self {
        Self {
            protocol: Protocol::new(req),
            extensions: vec![],
        }
    }

    /// Gets the list of headers to describe the extensions and protocol
    pub fn headers(self) -> impl Iterator<Item = Header<'static>> {
        self.protocol.get_name().into_iter().map(|s| Header::new("Sec-WebSocket-Protocol", s))
            .chain(self.extensions.into_iter().map(|e| e.header()))
    }

    /// Failed to parse extensions or protocol
    pub fn is_err(&self) -> Option<Status> {
        self.protocol.is_err()
    }
}

/// An individual WebSocket Extension
pub(crate) enum Extension {
}

impl Extension {
    /// Gets the header valus to enable this extension
    fn header(self) -> Header<'static> {
        match self {
        }
    }
}

/// A WebSocket Protocol. This lists every websocket protocol known to Rocket
#[allow(unused)]
pub(crate) enum Protocol {
    Multiplex,
    Naked,
    Invalid,
}

impl Protocol {
    pub fn new(_req: &Request<'_>) -> Self {
        Self::Naked
    }

    /// Gets a status code if the Protocol requested was invalid
    pub fn is_err(&self) -> Option<Status> {
        match self {
            Self::Naked => None,
            _ => Some(Status::ImATeapot),
        }
    }

    /// Gets the name to set for the WebSocket Protocol header
    pub fn get_name(&self) -> Option<&'static str> {
        match self {
            _ => None,
        }
    }
}

/// Everything needed to desribe a websocket Upgrade
/// TODO: Maybe don't use this? I think the only thing I do is split it up right away
pub(crate) struct WebsocketUpgrade {
    accept: String,
    on_upgrade: OnUpgrade,
}

impl WebsocketUpgrade {
    /// ?
    pub fn split(self) -> (String, OnUpgrade) {
        (self.accept, self.on_upgrade)
    }
}
