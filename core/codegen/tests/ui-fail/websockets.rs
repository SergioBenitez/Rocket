#[macro_use] extern crate rocket;

#[join("/echo")]
fn join() -> &'static str {
    ""
}

#[message("/echo")]
fn message() -> &'static str {
    ""
}

#[leave("/echo")]
fn leave() -> &'static str {
    ""
}

fn main() { }
