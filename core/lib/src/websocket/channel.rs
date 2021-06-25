use std::convert::TryFrom;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;
use futures::Future;
use rocket_http::uri::Origin;
use rocket_http::hyper::upgrade::Upgraded;
use tokio::io::{AsyncWrite, ReadHalf, WriteHalf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::select;
use tokio::time::timeout;
use tokio::sync::mpsc;
use tokio_util::codec::{Decoder, Encoder};
use websocket_codec::Opcode;
use websocket_codec::protocol::FrameHeader;
use websocket_codec::protocol::FrameHeaderCodec;

use crate::Request;
use crate::form::ValueField;
use crate::request::{FromWebSocket, WsOutcome};

use super::Extensions;
use super::broker::Broker;
use super::message::{IntoMessage, WebSocketMessage, to_message};
use super::status::WebSocketStatus;
use validation::Utf8Validator;

/// The smallest amount of space to allocate at once This is also used as a soft maximum in some
/// places, specifically when allocating space for chunks.
const BUFFER_ALLOCAION_MIN: usize = 1024 * 4;

/// An internal representation of a WebSocket connection
pub(crate) struct WebSocketChannel {
    inner: mpsc::Receiver<WebSocketMessage>,
    sender: mpsc::Sender<WebSocketMessage>,
    extensions: Extensions,
}

impl WebSocketChannel {
    /// Creates a WebSocketChannel, using the provided Upgraded Object.
    ///
    /// This also returns two futures, for reading and writing. They MUST be polled to advance the
    /// channel. For performance reasons, they shouldn't be sent to seperate tasks, typically they
    /// should just be joined with `tokio::join!`
    pub fn new(
        upgrade: Upgraded,
        extensions: Extensions
    ) -> (Self, impl Future<Output = ()>, impl Future<Output = ()>) {
        let (broker_tx, broker_rx) = mpsc::channel(50);
        let (message_tx, message_rx) = mpsc::channel(1);
        let (a, b) = Self::message_handler(
            upgrade,
            broker_tx.clone(),
            broker_rx,
            message_tx,
            extensions.clone(),
        );
        (Self {
                inner: message_rx,
                sender: broker_tx,
                extensions,
        }, a, b)
    }

    /// Gets the handle to subscribe this channel to a descriptor
    pub fn subscribe_handle(&self) -> mpsc::Sender<WebSocketMessage> {
        self.sender.clone()
    }

    /// Sends a close event
    pub async fn close(&self, status: WebSocketStatus<'_>) {
        let _e = self.sender.send(WebSocketMessage::close(status)).await;
    }

    /// Sends a message
    #[allow(unused)]
    pub async fn send(&self, message: impl IntoMessage) {
        to_message(message, &self.sender).await;
    }

    /// Get the next message from this client.
    ///
    /// This method also forwards messages sent from any channels the client is subscribed to
    pub async fn next(&mut self) -> Option<WebSocketMessage> {
        self.inner.recv().await
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    // TODO maybe avoid repeated if blocks?
    fn protocol_error(header: &FrameHeader, remaining: usize, extensions: &Extensions) -> bool {
        if (header.opcode() == u8::from(Opcode::Close) ||
            header.opcode() == u8::from(Opcode::Ping) ||
            header.opcode() == u8::from(Opcode::Pong)) &&
            (remaining > 125 || !header.fin())
        {
            true
        } else if header.rsv() & !extensions.allowed_rsv_bits() != 0u8 {
            true
        } else if Opcode::try_from(header.opcode()).is_none() && header.opcode() != 0x0 {
            true
        } else {
            false
        }
    }

    async fn send_close(
        broker_tx: &mpsc::Sender<WebSocketMessage>,
        status: WebSocketStatus<'_>
    ) {
        let (tx, rx) = mpsc::channel(1);
        let header = FrameHeader::new(
            true,
            0x0,
            Opcode::Close.into(),
            None,
            0usize.into(),
        );
        let _e = broker_tx.send(WebSocketMessage::from_parts(header, None, rx)).await;
        let _e = tx.send(status.encode()).await;
    }

    async fn reader(
        recv_close: Arc<AtomicBool>,
        mut read: ReadHalf<Upgraded>,
        broker_tx: mpsc::Sender<WebSocketMessage>,
        ping_tx: mpsc::Sender<Bytes>,
        message_tx: mpsc::Sender<WebSocketMessage>,
        extensions: Extensions,
    ) {
        let mut codec = websocket_codec::protocol::FrameHeaderCodec;
        let mut read_buf = BytesMut::with_capacity(BUFFER_ALLOCAION_MIN);
        let (mut data_tx, data_rx) = mpsc::channel(3);
        let mut data_rx = Some(data_rx);
        let mut validator = Utf8Validator::new();
        let mut utf8 = false;
        loop {
            let h = match codec.decode(&mut read_buf) {
                Ok(Some(h)) => h,
                Ok(None) => {
                    read_buf.reserve(BUFFER_ALLOCAION_MIN);
                    match read.read_buf(&mut read_buf).await {
                        Ok(0) => break,//warn_!("Potential EOF from client"),
                        Ok(_n) => (),
                        Err(e) =>
                            error_!(
                                "Io error occured during WebSocket messages: {:?}",
                                e
                            ),
                    }
                    continue;
                },
                Err(e) => {
                    error_!("WebSocket client broke protocol: {:?}", e);
                    Self::send_close(&broker_tx, WebSocketStatus::ProtocolError).await;
                    break;
                },
            };
            // A frame has been decoded
            // Save some attributes
            let fin = h.fin();
            if h.opcode() == u8::from(Opcode::Close) {
                recv_close.store(true, atomic::Ordering::Release);
            }
            let mut mask: Option<u32> = h.mask().map(|m| m.into());
            let mut remaining = if let Ok(r) = usize::try_from(h.data_len()) {
                r
            }else {
                // Send a protocol error if the datalength isn't valid
                Self::send_close(&broker_tx, WebSocketStatus::ProtocolError).await;
                break;
            };
            // Checks for some other protocol errors
            if Self::protocol_error(&h, remaining, &extensions) {
                warn_!("Remote Protocol Error");
                Self::send_close(&broker_tx, WebSocketStatus::ProtocolError).await;
                break;
            }
            if h.opcode() == u8::from(Opcode::Ping) {
                // Read data, and send on ping_tx
                // Note that remaining MUST be less than 125, otherwise it's a protocol
                // error - checked by Self::protocol_error
                // Avoid overflow
                if read_buf.capacity() < remaining {
                    read_buf.reserve(BUFFER_ALLOCAION_MIN.max(remaining - read_buf.capacity()));
                }
                while remaining > read_buf.len() {
                    let _e = read.read_buf(&mut read_buf).await;
                }
                let mut chunk = read_buf.split_to(remaining);
                if let Some(mask) = &mut mask {
                    for b in chunk.iter_mut() {
                        *b ^= *mask as u8;
                        *mask = mask.rotate_right(8);
                    }
                }
                let _e = ping_tx.send(chunk.freeze()).await;
                continue;
            } else if h.opcode() == u8::from(Opcode::Pong) {
                if remaining > read_buf.capacity() {
                    read_buf.reserve(BUFFER_ALLOCAION_MIN.max(remaining - read_buf.capacity()));
                }
                while remaining > read_buf.len() {
                    let _e = read.read_buf(&mut read_buf).await;
                }
                // discard the pong message
                let _ = read_buf.split_to(remaining);
                continue;
            }
            // If there is a frame to continue, it must be continued
            if let Some(data_rx) = data_rx.take() {
                if h.opcode() == 0x0 {
                    error_!("Unexpected continue frame");
                    Self::send_close(&broker_tx, WebSocketStatus::ProtocolError).await;
                    break;
                } else {
                    utf8 = h.opcode() == u8::from(Opcode::Text);
                    let message = WebSocketMessage::from_parts(h, None, data_rx);
                    let _e = message_tx.send(message).await;
                }
            } else if h.opcode() == u8::from(Opcode::Close) {
                // This will disrupt handling of later messages, but since this was a close
                // frame, we drop later messages anyway.
                let (tx, rx) = mpsc::channel(3);
                data_tx = tx;
                let message = WebSocketMessage::from_parts(h, None, rx);
                let _e = message_tx.send(message).await;
                utf8 = false;
            } else if h.opcode() != 0x0 {
                error_!("Expected continue frame");
                Self::send_close(&broker_tx, WebSocketStatus::ProtocolError).await;
                break;
            }
            // Read and forward data - note that this will read (and discard) any data that
            // the server decided to ignore
            while remaining > 0 {
                let mut chunk = read_buf.split_to(read_buf.len().min(remaining));
                if let Some(mask) = &mut mask {
                    for b in chunk.iter_mut() {
                        *b ^= *mask as u8;
                        *mask = mask.rotate_right(8);
                    }
                }
                if utf8 && !validator.validate(&chunk) {
                    error_!("WebSocket client sent Invalid UTF-8");
                    Self::send_close(&broker_tx, WebSocketStatus::InvalidDataType).await;
                    break;
                }
                remaining -= chunk.len();
                let _e = data_tx.send(chunk.into()).await;
                if read_buf.len() < remaining {
                    if read_buf.capacity() < BUFFER_ALLOCAION_MIN {
                        read_buf.reserve(BUFFER_ALLOCAION_MIN);
                    }
                    // this will execute multiple reads until the buffer has enough data in it.
                    // the target length is the smallest of the capacity (how much the buffer can
                    // hold), remaining (how much more the client has to write), and
                    // BUFFER_ALLOCAION_MIN (provides a static upper bound). Note that there is no
                    // guarentee that the buffer contains exactly target_len bytes, but rather it
                    // contains at least target_len bytes.
                    let target_len = read_buf.capacity().min(remaining).min(BUFFER_ALLOCAION_MIN);
                    while read_buf.len() < target_len {
                        // read_buf typically only executes a single read syscall, so we put in a
                        // loop to make sure we aren't reading really small chunks.
                        let _e = read.read_buf(&mut read_buf).await;
                    }
                }
            }
            if validator.error() {
                // Break above only breaks out of the inner loop
                break;
            }
            if fin {
                if utf8 && !validator.fin() {
                    error_!("WebSocket client sent Incomplete UTF-8");
                    Self::send_close(&broker_tx, WebSocketStatus::InvalidDataType).await;
                    // Note that combining any two valid UTF-8 sequences must still be a
                    // valid UTF-8 sequence. Therefore, the validator doesn't need to be
                    // recreated. This is easy to see when looking at the implementation,
                    // since the validator just holds the number of continue sequences seen
                    break;
                }
                // If this was the final frame, prepare for the next message
                let (tx, rx) = mpsc::channel(3);
                // explicitly drop data_tx
                //let _e = data_tx.send(Bytes::new()).await;
                //drop(data_tx);
                data_tx = tx;
                data_rx = Some(rx);
            }
            // If this was a close frame, stop recieving messages
            if recv_close.load(atomic::Ordering::Acquire) {
                break;
            }
        }
        // If we broke out at any point, we should set the flag to prevent messages in
        // flight from sending invalid frames
        recv_close.store(true, atomic::Ordering::Release);
        // This return to try and prevent dropping early
        //read
    }

    async fn writer(
        recv_close: Arc<AtomicBool>,
        mut write: WriteHalf<Upgraded>,
        mut broker_rx: mpsc::Receiver<WebSocketMessage>,
        mut ping_rx: mpsc::Receiver<Bytes>,
        _extensions: Extensions,
    ) {
        let mut codec = websocket_codec::protocol::FrameHeaderCodec;
        let mut write_buf = BytesMut::with_capacity(BUFFER_ALLOCAION_MIN);
        let mut close_sent = false;
        while let Some(message) = Self::await_or_ping(
            &mut broker_rx,
            &mut ping_rx,
            &mut write,
            &mut write_buf,
            &mut codec).await
        {
            let (header, mut topic, mut rx) = message.into_parts();
            if header.opcode() == u8::from(Opcode::Close) {
                close_sent = true;
            }
            let mut running_header = Some(header);
            while let Some(chunk) = Self::await_or_ping(
                &mut rx,
                &mut ping_rx,
                &mut write,
                &mut write_buf,
                &mut codec).await
            {
                let mut parts = vec![chunk];
                let mut fin = false;
                // Buffer additional chunks if they are ready
                loop {
                    // if the duration is zero, the rx will be polled at least once.
                    // However, because rx has backpressure, we want to suspend the current
                    // taks and allow new, ready chunks to be added.
                    // However, the backpressure can be taken advantage of to make 5.19 &
                    // 5.20 pass
                    match timeout(Duration::from_millis(0), rx.recv()).await {
                        Ok(Some(chunk)) => parts.push(chunk),
                        Ok(None) => {
                            fin = true;
                            break;
                        },
                        Err(_e) => break,
                    }
                }
                if let Some(header) = running_header.take() {
                    // Prevent message from finishing if the closing handshake has already
                    // started
                    if recv_close.load(atomic::Ordering::Acquire)
                        && header.opcode() != u8::from(Opcode::Close)
                    {
                        break;
                    }
                    if header.mask().is_some() {
                        error_!("WebSocket messages should not be sent with masks");
                    }
                    let topic_buf = topic.take().map(|topic| topic.to_string());
                    let int_header = FrameHeader::new(
                            fin,
                            header.rsv(),
                            header.opcode(),
                            None,
                            // .chain is used to add the length of the topic (if any)
                            parts.iter()
                                .map(|s| s.len())
                                .chain(topic_buf.iter().map(|b| b.len() + 2))
                                .sum::<usize>().into(),
                        );
                    let _e = codec.encode(int_header, &mut write_buf);
                    let _e = write.write_all_buf(&mut write_buf).await;

                    // Write out the topic if we have one. This can only be present once
                    // (since it is replaced with None by take), so it will only be sent
                    // once at the beginning of a message. Naked messages will send topic
                    // as None, and therefore nothing will be appended
                    if let Some(_topic) = topic_buf {
                        //let _e = write.write_all(topic.as_bytes()).await;
                        //let _e = write.write_all(MULTIPLEX_CONTROL_CHAR).await;
                    }

                    for mut chunk in parts {
                        let _e = write.write_all_buf(&mut chunk).await;
                    }
                    if !fin {
                        running_header = Some(FrameHeader::new(
                                false,
                                header.rsv(),
                                0x0,// Continue frame
                                header.mask(),
                                0usize.into(), // We're going to ignore this anyway
                            ));
                    }else {
                        break;
                    }
                }else {
                    unreachable!()
                }
            }
            // If no data was sent, we send the header with an empty body anyway
            if let Some(header) = running_header.take() {
                if header.mask().is_some() {
                    error_!("WebSocket messages should not be sent with masks");
                }
                // Only write the fin frame if a close hasn't been received
                // if opcode = close => always write
                // if opcode != close, recv_close = false => write
                // if opcode != close, recv_close = true => ignore
                if header.opcode() == u8::from(Opcode::Close)
                    || !recv_close.load(atomic::Ordering::Acquire)
                {
                    let int_header = FrameHeader::new(
                            true,
                            header.rsv(),
                            header.opcode(),
                            None,
                            0usize.into(),
                        );
                    let _e = codec.encode(int_header, &mut write_buf);
                    let _e = write.write_all_buf(&mut write_buf).await;
                }
            }
            if close_sent {
                break;
            }
        }
        if !close_sent {
            warn_!("Writer task did not send close");
        }
        let _e = write.shutdown().await;
        //write
    }

    #[inline]
    fn message_handler(
        upgrade: Upgraded,
        broker_tx: mpsc::Sender<WebSocketMessage>,
        broker_rx: mpsc::Receiver<WebSocketMessage>,
        message_tx: mpsc::Sender<WebSocketMessage>,
        extensions: Extensions,
    ) -> (impl Future<Output = ()>, impl Future<Output = ()>) {
        // Split creates a mutex to syncronize access to the underlying io stream
        // This should be okay, since the lock shouldn't be in high contention, although it
        // make cause issues with trying to send and recieve messages at the same time.
        let (read, write) = tokio::io::split(upgrade);
        // The reader & writer only hold the lock while polling - so there should be NO
        // contention at all. This is important, since I believe the mutex uses a spin-lock (at
        // least in the version I looked at). Since the writer and reader are executed
        // concurrently, but not in parallel (by the join! macro), they should never be polled
        // at the same time.

        let recv_close = Arc::new(AtomicBool::new(false));
        let (ping_tx, ping_rx) = mpsc::channel(1);

        let reader = Self::reader(
            recv_close.clone(),
            read,
            broker_tx,
            ping_tx,
            message_tx,
            extensions.clone()
        );
        let writer = Self::writer(
            recv_close,
            write,
            broker_rx,
            ping_rx,
            extensions
        );
        (reader, writer)
    }

    /// This waits for a message on `f`, but will forward pings to the client if any are sent. This
    /// allows pings & pongs to go between other messages
    #[inline]
    async fn await_or_ping<T, W: AsyncWrite + Unpin>(
        f: &mut mpsc::Receiver<T>,
        ping_rx: &mut mpsc::Receiver<Bytes>,
        writer: &mut W,
        write_buf: &mut BytesMut,
        codec: &mut FrameHeaderCodec,
    ) -> Option<T> {
        loop {
            select! {
                o = f.recv() => break o,
                Some(p) = ping_rx.recv() => {
                    let pong_header = FrameHeader::new(
                        true,
                        0x0,
                        Opcode::Pong.into(),
                        None,
                        p.len().into()
                    );
                    let _e = codec.encode(pong_header, write_buf);
                    let _e = writer.write_all_buf(write_buf).await;
                    let _e = writer.write_all(&p).await;
                },
            }
        }
    }
}

mod validation {
    /// A simple and effienct UTF-8 validator. The validator can validate any UTF-8 text, and
    /// (unlike the standard lib) doesn't need to validate the entire string at once. This is
    /// also able to show errors on the first byte that makes the string invalid.
    pub enum Utf8Validator {
        Start,
        Continue,
        ContinueTwo,
        ContinueThree,
        E0,
        ED,
        F0,
        F4,
        Error,
    }

    impl Utf8Validator {
        /// Create a new validator, in the initial state
        pub fn new() -> Self {
            Self::Start
        }

        fn set_error(&mut self) -> bool {
            *self = Self::Error;
            false
        }

        /// Validates the bytes provided. `bytes` does not need to be a complete message, and the
        /// validator will track the current bytes as needed.
        ///
        /// Returns false if bytes doesn't not contain a valid chunk of UTF-8, or the validator has
        /// previously encountered an error.
        pub fn validate(&mut self, bytes: &[u8]) -> bool {
            for b in bytes.iter() {
                match self {
                    Self::Start => match b {
                        0x00..=0x7F => (),
                        0xC2..=0xDF => *self = Self::Continue,
                        0xE1..=0xEC => *self = Self::ContinueTwo,
                        0xEE..=0xEF => *self = Self::ContinueTwo,
                        0xF1..=0xF3 => *self = Self::ContinueThree,
                        0xE0 => *self = Self::E0,
                        0xED => *self = Self::ED,
                        0xF0 => *self = Self::F0,
                        0xF4 => *self = Self::F4,
                        _ => return self.set_error(),
                    }
                    Self::Continue => match b {
                        0x80..=0xBF => *self = Self::Start,
                        _ => return self.set_error(),
                    }
                    Self::ContinueTwo => match b {
                        0x80..=0xBF => *self = Self::Continue,
                        _ => return self.set_error(),
                    }
                    Self::ContinueThree => match b {
                        0x80..=0xBF => *self = Self::ContinueTwo,
                        _ => return self.set_error(),
                    }
                    Self::E0 => match b {
                        0xA0..=0xBF => *self = Self::Continue,
                        _ => return self.set_error(),
                    },
                    Self::ED => match b {
                        0x80..=0x9F => *self = Self::Continue,
                        _ => return self.set_error(),
                    },
                    Self::F0 => match b {
                        0x90..=0xBF => *self = Self::ContinueTwo,
                        _ => return self.set_error(),
                    },
                    Self::F4 => match b {
                        0x80..=0x8F => *self = Self::ContinueTwo,
                        _ => return self.set_error(),
                    },
                    Self::Error => return false,
                }
            }
            true
        }

        /// Returns true if the last call to validate ended on a UTF-8 character boundary.
        pub fn fin(&self) -> bool {
            matches!(self, Self::Start)
        }

        /// Returns true if this validator has previously encountered invalid UTF-8
        pub fn error(&self) -> bool {
            matches!(self, Self::Error)
        }
    }
}

/// Represents an in-progress WebSocket Connection
pub struct WebSocket<'r> {
    request: Request<'r>,
    sender: mpsc::Sender<WebSocketMessage>,
}

impl<'r> WebSocket<'r> {
    pub(crate) fn new(request: Request<'r>, sender: mpsc::Sender<WebSocketMessage>) -> Self {
        Self { request, sender }
    }

    /// Gets the inner request object
    pub fn request(&self) -> &Request<'r> {
        &self.request
    }

    /// Gets the topic URI associated with this channel
    pub fn topic(&self) -> &Origin<'r> {
        self.request.uri()
    }

    /// Sets the topic URI for this WebSocket
    #[allow(unused)]
    pub(crate) fn set_topic(&mut self, topic: Origin<'r>) {
        self.request.set_uri(topic);
    }

    pub(crate) fn broker(&self) -> &Broker {
        &self.request.state.rocket.broker
    }
}

#[doc(hidden)]
impl<'r> WebSocket<'r> {
    // Retrieves the pre-parsed query items. Used by matching and codegen.
    #[inline]
    pub fn query_fields(&self) -> impl Iterator<Item = ValueField<'_>> {
        self.request.query_fields()
    }
}

/// A WebSocket Channel. This can be used to message the client that sent a message (namely, the
/// one you are responding to), as well as broadcast messages to other clients. Note that every
/// client will recieve a broadcast, including the client that triggered the broadcast.
///
/// The lifetime parameter should typically be <'_>. This is nessecary since the object contains
/// references (which it might be possible to remove in later versions), and recent versions of
/// Rust recommend not allowing implicit elided lifetimes.
///
/// This is only valid as a Guard on WebSocket handlers (`message`, `join`, and `leave`).
pub struct Channel<'r> {
    sender: mpsc::Sender<WebSocketMessage>,
    broker: &'r Broker,
    topic: &'r Origin<'r>,
}

impl<'r> Channel<'r> {
    /// Sends a message to the client
    pub async fn send(&self, message: impl IntoMessage) {
        to_message(message, &self.sender).await;
    }

    /// Broadcasts a message to every client connected to the same topic
    ///
    /// This is equavelent to `channel.broadcast_to(channel.topic(), message)`
    pub async fn broadcast(&self, message: impl IntoMessage) {
        self.broker.broadcast_to(self.topic, message).await;
    }

    /// Gets the topic URI associated with this channel.
    ///
    /// If you would like to send the topic to
    /// another task, you will need to make it owned, which will allocate a clone of it.
    pub fn topic(&self) -> &Origin<'r> {
        self.topic
    }

    /// Broadcasts a message to every client connected to `topic`
    ///
    /// When constructing topics, it is recommended to use the `uri!` macro, since it does type
    /// checking and verification of the url.
    pub async fn broadcast_to(&self, topic: &Origin<'_>, message: impl IntoMessage) {
        self.broker.broadcast_to(topic, message).await;
    }
}

#[crate::async_trait]
impl<'r> FromWebSocket<'r> for Channel<'r> {
    type Error = std::convert::Infallible;

    async fn from_websocket(request: &'r WebSocket<'_>) -> WsOutcome<Self, Self::Error> {
        WsOutcome::Success(Self {
            sender: request.sender.clone(),
            broker: request.broker(),
            topic: request.request.uri(),
        })
    }
}
