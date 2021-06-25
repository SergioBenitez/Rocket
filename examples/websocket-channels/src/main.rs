#[macro_use] extern crate rocket;

use rocket::websocket::Channel;
use rocket::response::content::Html;
use rocket::{State, Data};

#[message("/listen", data = "<data>")]
async fn listen(data: Data<'_>, ws: Channel<'_>) {
    ws.broadcast(data).await;
}

#[get("/")]
fn index() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
    <head>
        <title>WebSocket Channel Server</title>
    </head>
    <body>
        <h1>Channel Server</h1>
        <p>Messages sent from here will be sent to every connected client</p>
        <p id="status">Connecting...</p>
        <input type="text" id="text" />
        <button type="button" id="send">Send</button>
        <div id="lines"></div>
        <script type="text/javascript">
            const lines = document.getElementById('lines');
            const text = document.getElementById('text');
            const status = document.getElementById('status');
            const ws = new WebSocket('ws://' + location.host + '/listen');
            ws.onopen = function(e) {
                status.innerText = 'Connected :)';
                // Send a message to make sure we recieve broadcasts
                ws.send("Hello World!");
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

#[launch]
fn rocket() -> _ {
    rocket::build().mount("/", routes![listen, index])
}
