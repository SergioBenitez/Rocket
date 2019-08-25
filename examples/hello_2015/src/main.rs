#![feature(proc_macro_hygiene)]

#[macro_use] extern crate rocket;

#[cfg(test)] mod tests;

#[get("/")]
fn hello() -> &'static str {
    "Hello, Rust 2015!"
}

fn main() {
    let _ = rocket::ignite().mount("/", routes![hello]).launch();
}
