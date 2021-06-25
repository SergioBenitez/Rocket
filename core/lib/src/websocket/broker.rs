//! Phoenix-like channels for Rocket websockets.
//!
//! The implementation is somewhat complex, but quite flexible. A `Channel` object is created to
//! share messages using a subscription based model. A client can subscribe to a specific
//! descriptor in a channel, using the methods provided by the `Channel` object. See the
//! documentation for the `ChannelDescriptor` trait for more information on the matching process.
//!
//! Typically, a Channel will be created and added to the state that Rocket manages. This is
//! nessecary since Rocket needs to know what type you would like to use as the `ChannelDescriptor`,
//! and it also allows mutiple channels, depending on the descriptor type.

use rocket_http::{ext::IntoOwned, uri::Origin};
use tokio::sync::mpsc;

use crate::{Request, request::{FromRequest, Outcome}};

use super::{Protocol, channel::WebSocketChannel, message::{IntoMessage, WebSocketMessage}};

/// Internal enum for sharing messages between clients
enum BrokerMessage {
    /// Registers a websocket to recieve messages from a room
    ///
    /// Note: this should only be sent once per websocket connection
    Register(Origin<'static>, mpsc::Sender<WebSocketMessage>, Protocol),

    /// Removes a previously registered listener
    ///
    /// Note, this will remove all matching listeners, since there is no Eq bounds
    Unregister(Origin<'static>, mpsc::Sender<WebSocketMessage>),

    /// Removes all previously registered listeners for this client
    UnregisterAll(mpsc::Sender<WebSocketMessage>),

    /// Sends a message that should be forwarded to every socket listening
    Forward(Origin<'static>, WebSocketMessage),
}

/// A channel for sharing messages between multiple clients, and the central server.
///
/// This should typically be created, and added to Rocket's managed state, where it
/// can be accessed via the state request guard. `Channel` also implements clone, and
/// acts as a handle to the internal channels, which allows messages to be generated
/// and sent outside of Rocket request handlers.
///
/// See the examples for how to use Channel.
/// TODO: Create examples
#[derive(Clone)]
pub struct Broker {
    channels: mpsc::UnboundedSender<BrokerMessage>,
}

impl Broker {
    /// Creates a new channel, and starts the nessecary tasks in the background. The task will
    /// automatically end as soon as every handle on this channel has been dropped.
    pub(crate) fn new() -> Self {
        let (sender, reciever) = mpsc::unbounded_channel();
        tokio::spawn(Self::channel_task(reciever));
        Self {
            channels: sender,
        }
    }

    /// Sends a message to all clients subscribed to the channel using descriptor `id`
    pub async fn broadcast_to(&self, id: &Origin<'_>, message: impl IntoMessage) {
        let (tx, rx) = mpsc::channel(1);
        if let Ok(()) = self.channels.send(BrokerMessage::Forward(
            id.clone().into_owned(),
            WebSocketMessage::new(message.is_binary(), rx)
        )) {
            message.into_message(tx).await;
        }
    }

    /// Subscribes the client to this channel using the descriptor `id`
    pub(crate) async fn subscribe(
        &self,
        id: &Origin<'_>,
        channel: &WebSocketChannel,
    ) {
        let _ = self.channels.send(BrokerMessage::Register(
            id.clone().into_owned(),
            channel.subscribe_handle(),
            channel.extensions().protocol()
        ));
    }

    /// Unsubscribes the client from this channel using the descriptor `id`
    ///
    /// # Note
    /// This will unsubscribe this client from EVERY descriptor that matches `id`
    #[allow(unused)]
    pub(crate) async fn unsubscribe(&self, id: &Origin<'_>, channel: &WebSocketChannel) {
        let _ = self.channels.send(
            BrokerMessage::Unregister(id.clone().into_owned(), channel.subscribe_handle())
        );
    }

    /// Unsubscribes the client from any messages on this channel
    ///
    /// The client is automatically unsubscribed if they are disconnected, so this does not need
    /// to be called when the client is disconnecting
    pub(crate) async fn unsubscribe_all(&self, channel: &WebSocketChannel) {
        let _ = self.channels.send(BrokerMessage::UnregisterAll(channel.subscribe_handle()));
    }

    /// Channel task for tracking subscribtions and forwarding messages
    async fn channel_task(mut rx: mpsc::UnboundedReceiver<BrokerMessage>) {
        let mut subs = ChannelMap::new(100);
        while let Some(wsm) = rx.recv().await {
            match wsm {
                BrokerMessage::Register(room, tx, protocol) => subs.insert(tx, room, protocol),
                BrokerMessage::Forward(room, message) => subs.send(room, message).await,
                BrokerMessage::Unregister(room, tx) => subs.remove_value(tx, room),
                BrokerMessage::UnregisterAll(tx) => subs.remove_key(tx),
            }
            // TODO make this happen less often
            subs.cleanup();
        }
    }
}

impl std::fmt::Debug for Broker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Broker {{}}")
    }
}

#[crate::async_trait]
impl<'r> FromRequest<'r> for Broker {
    type Error = std::convert::Infallible;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        Outcome::Success(request.rocket().broker())
    }
}

/// Convient struct for holding channel subscribtions
struct ChannelMap(Vec<(mpsc::Sender<WebSocketMessage>, Vec<Origin<'static>>, Protocol)>);

impl ChannelMap {
    /// Create map with capactity
    fn new(capacity: usize) -> Self {
        Self(Vec::with_capacity(capacity))
    }

    /// Add `descriptor` to the list of subscriptions for `tx`
    fn insert(
        &mut self,
        tx: mpsc::Sender<WebSocketMessage>,
        descriptor: Origin<'static>,
        protocol: Protocol,
    ) {
        for (t, v, _) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.push(descriptor);
                return;
            }
        }
        self.0.push((tx, vec![descriptor], protocol));
    }

    /// Remove every descriptor `tx` is subscribed to
    fn remove_key(&mut self, tx: mpsc::Sender<WebSocketMessage>) {
        self.0.retain(|(t, _, _)| !t.same_channel(&tx));
    }

    /// Remove every descriptor that `descriptor` matches and `tx` is subscribed to
    fn remove_value(&mut self, tx: mpsc::Sender<WebSocketMessage>, descriptor: Origin<'static>) {
        for (t, v, _) in self.0.iter_mut() {
            if t.same_channel(&tx) {
                v.retain(|d| d != &descriptor);
                return;
            }
        }
    }

    /// Forward a message to every client that is subscribed to a descriptor that matches
    /// `descriptor`
    async fn send(&mut self, descriptor: Origin<'static>, message: WebSocketMessage) {
        let mut chs = vec![];
        let (header, _, mut data) = message.into_parts();
        for (t, v, p) in self.0.iter() {
            if v.iter().any(|r| r == &descriptor) {
                // message.clone() should be very cheap, since it uses `Bytes` internally to store
                // the raw data
                let (data_tx, data_rx) = mpsc::channel(2);
                if let Ok(()) = t.send(WebSocketMessage::from_parts(
                    header.clone(),
                    p.with_topic(&descriptor),
                    data_rx
                )).await {
                    chs.push(data_tx);
                }
            }
        }

        tokio::spawn(async move {
            while let Some(next) = data.recv().await {
                for ch in chs.iter() {
                    // TODO prevent a slow client from blocking others
                    let _e = ch.send(next.clone()).await;
                }
            }
        });
    }

    fn cleanup(&mut self) {
        self.0.retain(|(t, _, _)| !t.is_closed());
    }
}
