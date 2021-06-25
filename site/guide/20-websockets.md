# WebSockets

Traditional HTTP requests must always be initiated by the client, but there are
several reasons why a server might need to send events to the client. There are
several solutions, such as polling the server at some interval, Server Sent
Events (SSE), or WebSockets.

Polling is generally frowned upon, since it uses quite a bit of bandwidth and
makes the server process extra requests even when nothing is ready to be sent.

Rocket provides an API for supporting both SSE and WebSockets. See [SSE] for
more information on how to use SSE with Rocket.

! NOTE: This page assumes a general familiarity with Rocket & WebSockets.

## WebSocket Events

The WebSocket protocol is fairly invloved, but Rocket handles much of the complexity
internally. The following is a complete echo server:

```rust
#[macro_use] extern crate rocket;

use rocket::websocket::Channel;
use rocket::response::content::Html;
use rocket::Data;

#[message("/echo", data = "<data>")]
async fn echo(data: Data<'_>, ch: Channel<'_>) {
    ch.send(data).await;
}

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![echo])
}
```

There are only a couple of differences to point out here. The use of a `message`
attribute, which isn't an HTTP Method like GET, etc. `message` is a WebSocket event;
there are two more that will be discussed later. The `message` event is run on each
incomming message. Unlike HTTP Methods, message cannot return a response.

Conceptually, `Channel` is just a Request Guard that provides a handle to send messages
to the client. There is a slight difference that will be discussed later.

`Data` is handled exactly the same in WebSocket and HTTP routes.

### A Client implementation

There are several options to create a client to connect to and test the echo server.
There are several WebSocket libraries in a variety of languages, such as the
[tungstenite](https://crates.io/crates/tungstenite) crate for Rust. Below is a simple
example using the JavaScript WebSocket API, which is provided by the browser. This
creates an index page which uses JavaScript to open a websocket connection, and send
messages from the browser.

```rust
#[get("/")]
fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>WebSocket Echo Server</title>
    </head>
    <body>
        <h1>Echo Server</h1>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript">
            const lines = document.getElementById('lines');
            const text = document.getElementById('text');
            const status = document.getElementById('status');
            const ws = new WebSocket('ws://' + location.host + '/echo');
            ws.onopen = function(e) {
                status.innerText = 'Connected :)';
            };
            ws.onclose = function(e) {
                status.innerText = 'Disconnected :(';
                lines.innerHTML = '';
            };
            ws.onmessage = function(msg, e) {
                const line = document.createElement('p');
                line.innerText = msg.data;
                lines.prepend(line);
            };
            send.onclick = function(e) {
                ws.send(text.value);
                text.value = '';
            };
        </script>
    </body>
</html>"#)
}
```

Don't forget to mount this route as well!

## Join Events

A Join event (the `join` attribute) is run on the first message the client sends.
Since the Join handler is optional, there is no guarentee that a join handler has
run before a message handler is run.

! WARNING: Authentication

  Any and All authentication should be enforced via Request Guards on ALL of the
  relevant Event handlers.

### Leave Events

A Leave event is run after the client disconnects.

## Channel

The `Channel` is actually a Channel Guard, which means it can only be used on
WebSocket Event handlers. All Request Guards are automatically Channel Guards,
but the reverse is not always true.

`Channel` has a lifetime arguement, which should almost always just be elided (`'_`).

## Broadcasts

Rocket provides a `Broker` implementation, which handles broadcasting messages to
multiple clients. `Channel` holds a reference to Rocket's broker, and has a broadcast
method which broadcasts a message to every client connected to the same topic (the
URI of the initial request). For example, if a client connects to
`ws:://rocket.rs/updates/listen`, the topic would be `/updates/listen`.
