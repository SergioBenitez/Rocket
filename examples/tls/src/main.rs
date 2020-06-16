#![feature(proc_macro_hygiene)]

#[macro_use] extern crate rocket;

#[cfg(test)] mod tests;

#[get("/")]
fn hello() -> &'static str {
    "Hello, world!"
}

#[rocket::launch]
fn rocket() -> rocket::Rocket {
    rocket::ignite().mount("/", routes![hello])
}
