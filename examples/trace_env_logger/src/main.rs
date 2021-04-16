#[macro_use] extern crate rocket;
use rocket::trace::info;

#[get("/hello/<name>/<age>")]
fn hello(name: &str, age: u8) -> String {
    info!(?name, age, "saying hello to");
    format!("Hello, {} year old named {}!", age, name)
}

#[get("/hello/<name>")]
fn hi(name: &str) -> &str {
    info!(?name, "saying hi to");
    name
}

#[launch]
fn rocket() -> _ {
    env_logger::init();
    rocket::build().mount("/", routes![hello, hi])
}
